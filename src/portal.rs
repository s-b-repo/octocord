//! xdg-desktop-portal ScreenCast client.
//!
//! This is the only way to capture a Wayland desktop: the compositor never exposes
//! the screen contents to X11, and this ffmpeg build has no `pipewiregrab` filter, so
//! the frames have to be pulled from PipeWire by the application itself.
//!
//! The handshake is the one described by the portal specification:
//!
//! ```text
//! CreateSession      -> session_handle
//! SelectSources      -> which outputs, cursor mode, persistence
//! Start              -> the compositor's picker, then the PipeWire node ids
//! OpenPipeWireRemote -> a file descriptor for the PipeWire connection
//! ```
//!
//! Every step except the last answers asynchronously through a `Response` signal on a
//! per-request object path, so each call registers a match on that path before issuing
//! the method call.

use anyhow::{anyhow, bail, Context, Result};
use dbus::arg::{PropMap, RefArg, TypeMismatchError, Variant};
use dbus::blocking::Connection;
use dbus::message::MatchRule;
use dbus::Path;
use log::{debug, info, warn};
use std::os::fd::{FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const PORTAL_DEST: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SCREENCAST_IFACE: &str = "org.freedesktop.portal.ScreenCast";
const REQUEST_IFACE: &str = "org.freedesktop.portal.Request";

/// Source types bitmask (portal spec): 1 = monitor, 2 = window, 4 = virtual.
const SOURCE_TYPE_MONITOR: u32 = 1;
/// Cursor modes: 1 = hidden, 2 = embedded in the frames, 4 = sent as metadata.
const CURSOR_MODE_HIDDEN: u32 = 1;
const CURSOR_MODE_EMBEDDED: u32 = 2;
/// Persist modes: 0 = never, 1 = until the app stops, 2 = until revoked.
const PERSIST_MODE_PERSISTENT: u32 = 2;

/// One capture stream offered by the compositor.
#[derive(Debug, Clone, Copy)]
pub struct ScreenCastStream {
    pub node_id: u32,
    /// Size hint from the portal. The authoritative size is the one PipeWire
    /// negotiates, so this is only used for logging and UI.
    pub size: Option<(u32, u32)>,
}

/// A live ScreenCast session.
///
/// The session belongs to the D-Bus connection that created it: dropping the
/// connection tells the portal to tear the session down and the PipeWire node
/// disappears. The connection is therefore kept inside this struct for as long as
/// the capture runs.
pub struct ScreenCastSession {
    _connection: Connection,
    pub streams: Vec<ScreenCastStream>,
    pub restore_token: Option<String>,
    pub fd: OwnedFd,
}

impl ScreenCastSession {
    pub fn primary_stream(&self) -> Result<ScreenCastStream> {
        self.streams
            .first()
            .copied()
            .ok_or_else(|| anyhow!("The portal returned no capture streams"))
    }
}

/// Whether a ScreenCast portal implementation is reachable on the session bus.
///
/// Cached: this is consulted on every UI frame, and each probe is a D-Bus round trip.
pub fn is_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| match probe() {
        Ok(version) => {
            info!("ScreenCast portal available (version {})", version);
            true
        }
        Err(e) => {
            debug!("ScreenCast portal unavailable: {}", e);
            false
        }
    })
}

fn probe() -> Result<u32> {
    let conn = Connection::new_session().context("Failed to connect to the session bus")?;
    let proxy = conn.with_proxy(PORTAL_DEST, PORTAL_PATH, Duration::from_secs(2));
    let (version,): (Variant<Box<dyn RefArg>>,) = proxy
        .method_call(
            "org.freedesktop.DBus.Properties",
            "Get",
            (SCREENCAST_IFACE, "version"),
        )
        .context("ScreenCast interface not present")?;
    version
        .0
        .as_u64()
        .map(|v| v as u32)
        .ok_or_else(|| anyhow!("Portal returned a malformed version property"))
}

/// Options for starting a capture session.
pub struct ScreenCastOptions {
    pub capture_cursor: bool,
    /// Token from a previous session; lets the portal skip the picker dialog.
    pub restore_token: Option<String>,
}

/// Run the full portal handshake and return a session ready to be handed to PipeWire.
///
/// `Start` may block on a user-facing picker dialog, hence the long timeout.
pub fn start_screencast(options: &ScreenCastOptions) -> Result<ScreenCastSession> {
    let conn = Connection::new_session().context("Failed to connect to the session bus")?;
    let unique = conn.unique_name().to_string();

    // --- CreateSession -----------------------------------------------------
    let session_token = next_token("session");
    let mut create_options = PropMap::new();
    insert_str(&mut create_options, "handle_token", &next_token("create"));
    insert_str(&mut create_options, "session_handle_token", &session_token);

    let results = call_with_response(
        &conn,
        &unique,
        "CreateSession",
        create_options,
        |proxy, opts| proxy.method_call(SCREENCAST_IFACE, "CreateSession", (opts,)),
        Duration::from_secs(20),
    )?;

    let session_handle = results
        .get("session_handle")
        .and_then(|v| v.0.as_str().map(str::to_owned))
        .ok_or_else(|| anyhow!("Portal did not return a session handle"))?;
    debug!("ScreenCast session: {}", session_handle);
    let session_path = Path::new(session_handle.clone())
        .map_err(|_| anyhow!("Portal returned an invalid session path: {}", session_handle))?
        .into_static();

    // --- SelectSources -----------------------------------------------------
    let mut select_options = PropMap::new();
    insert_str(&mut select_options, "handle_token", &next_token("select"));
    select_options.insert("types".into(), Variant(Box::new(SOURCE_TYPE_MONITOR)));
    select_options.insert("multiple".into(), Variant(Box::new(false)));
    select_options.insert(
        "cursor_mode".into(),
        Variant(Box::new(if options.capture_cursor {
            CURSOR_MODE_EMBEDDED
        } else {
            CURSOR_MODE_HIDDEN
        })),
    );
    select_options.insert(
        "persist_mode".into(),
        Variant(Box::new(PERSIST_MODE_PERSISTENT)),
    );
    if let Some(token) = &options.restore_token {
        insert_str(&mut select_options, "restore_token", token);
    }

    let select_path = session_path.clone();
    call_with_response(
        &conn,
        &unique,
        "SelectSources",
        select_options,
        move |proxy, opts| {
            proxy.method_call(SCREENCAST_IFACE, "SelectSources", (select_path.clone(), opts))
        },
        Duration::from_secs(30),
    )?;

    // --- Start -------------------------------------------------------------
    let mut start_options = PropMap::new();
    insert_str(&mut start_options, "handle_token", &next_token("start"));

    let start_path = session_path.clone();
    let results = call_with_response(
        &conn,
        &unique,
        "Start",
        start_options,
        move |proxy, opts| {
            proxy.method_call(
                SCREENCAST_IFACE,
                "Start",
                (start_path.clone(), "".to_string(), opts),
            )
        },
        // The compositor shows its screen picker here, so allow for a human.
        Duration::from_secs(300),
    )?;

    let streams = parse_streams(&results);
    if streams.is_empty() {
        bail!("The portal session started but offered no streams");
    }
    let restore_token = results
        .get("restore_token")
        .and_then(|v| v.0.as_str().map(str::to_owned));

    // --- OpenPipeWireRemote ------------------------------------------------
    let proxy = conn.with_proxy(PORTAL_DEST, PORTAL_PATH, Duration::from_secs(20));
    let (fd,): (dbus::arg::OwnedFd,) = proxy
        .method_call(
            SCREENCAST_IFACE,
            "OpenPipeWireRemote",
            (session_path, PropMap::new()),
        )
        .context("OpenPipeWireRemote failed")?;
    // `into_fd` hands over ownership without closing the descriptor.
    let fd = unsafe { OwnedFd::from_raw_fd(fd.into_fd()) };

    info!(
        "ScreenCast started: {} stream(s), restore token {}",
        streams.len(),
        if restore_token.is_some() { "stored" } else { "not offered" }
    );

    Ok(ScreenCastSession {
        _connection: conn,
        streams,
        restore_token,
        fd,
    })
}

fn parse_streams(results: &PropMap) -> Vec<ScreenCastStream> {
    let Some(streams) = results.get("streams") else {
        return Vec::new();
    };
    let Some(entries) = streams.0.as_iter() else {
        return Vec::new();
    };

    let mut parsed = Vec::new();
    for entry in entries {
        // Each entry is a `(ua{sv})` struct: node id followed by its properties.
        let Some(mut fields) = entry.as_iter() else { continue };
        let Some(node_id) = fields.next().and_then(|f| f.as_u64()) else { continue };
        let size = fields.next().and_then(parse_size);
        parsed.push(ScreenCastStream {
            node_id: node_id as u32,
            size,
        });
    }
    parsed
}

/// Pull the `size` property, a `(ii)` struct, out of a stream's property map.
fn parse_size(props: &dyn RefArg) -> Option<(u32, u32)> {
    let mut iter = props.as_iter()?;
    while let Some(key) = iter.next() {
        let value = iter.next()?;
        if key.as_str() != Some("size") {
            continue;
        }
        // Variants need one more unwrap before the struct fields are reachable.
        let inner = value.as_iter().and_then(|mut it| it.next().map(|v| v.box_clone()));
        let target = inner.as_deref().unwrap_or(value);
        let mut dims = target.as_iter()?;
        let width = dims.next()?.as_i64()?;
        let height = dims.next()?.as_i64()?;
        if width > 0 && height > 0 {
            return Some((width as u32, height as u32));
        }
    }
    None
}

/// The portal's asynchronous reply.
struct PortalResponse {
    status: u32,
    results: PropMap,
}

impl dbus::arg::ReadAll for PortalResponse {
    fn read(i: &mut dbus::arg::Iter) -> Result<Self, TypeMismatchError> {
        Ok(PortalResponse {
            status: i.read()?,
            results: i.read()?,
        })
    }
}

/// Issue a portal method call and block until its `Response` signal arrives.
fn call_with_response<F>(
    conn: &Connection,
    unique_name: &str,
    method: &str,
    options: PropMap,
    call: F,
    timeout: Duration,
) -> Result<PropMap>
where
    F: FnOnce(&dbus::blocking::Proxy<'_, &Connection>, PropMap) -> Result<(Path<'static>,), dbus::Error>,
{
    let token = options
        .get("handle_token")
        .and_then(|v| v.0.as_str().map(str::to_owned))
        .ok_or_else(|| anyhow!("internal: {} called without a handle token", method))?;

    // Register the match *before* the call: the portal may answer immediately.
    let expected = request_path(unique_name, &token);
    let rule = MatchRule::new_signal(REQUEST_IFACE, "Response").with_path(expected.clone());
    let slot: Arc<Mutex<Option<PortalResponse>>> = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&slot);
    let token_handle = conn
        .add_match(rule, move |response: PortalResponse, _, _| {
            if let Ok(mut slot) = sink.lock() {
                *slot = Some(response);
            }
            true
        })
        .with_context(|| format!("Failed to watch for the {} response", method))?;

    let outcome = (|| -> Result<PropMap> {
        let proxy = conn.with_proxy(PORTAL_DEST, PORTAL_PATH, Duration::from_secs(30));
        let (request_path,) = call(&proxy, options)
            .with_context(|| format!("Portal call {} failed", method))?;
        if request_path != expected {
            // Not fatal on its own, but the response would never reach our match rule.
            warn!(
                "Portal {} returned request path {} instead of {}",
                method, request_path, expected
            );
        }

        let deadline = Instant::now() + timeout;
        loop {
            if let Some(response) = slot.lock().unwrap().take() {
                if response.status != 0 {
                    bail!(
                        "{} was cancelled or denied (portal status {})",
                        method,
                        response.status
                    );
                }
                return Ok(response.results);
            }
            if Instant::now() >= deadline {
                bail!("Timed out waiting for the portal to answer {}", method);
            }
            conn.process(Duration::from_millis(100))
                .with_context(|| format!("D-Bus error while waiting for {}", method))?;
        }
    })();

    let _ = conn.remove_match(token_handle);
    outcome
}

/// Object path the portal will emit the `Response` signal on.
fn request_path(unique_name: &str, token: &str) -> Path<'static> {
    let sender = unique_name.trim_start_matches(':').replace('.', "_");
    Path::new(format!(
        "/org/freedesktop/portal/desktop/request/{}/{}",
        sender, token
    ))
    .expect("request paths are always valid")
    .into_static()
}

fn next_token(prefix: &str) -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!(
        "octocord_{}_{}_{}",
        prefix,
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

fn insert_str(map: &mut PropMap, key: &str, value: &str) {
    map.insert(key.to_string(), Variant(Box::new(value.to_string())));
}
