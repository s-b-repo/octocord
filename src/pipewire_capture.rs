//! PipeWire video capture for the frames offered by the ScreenCast portal.
//!
//! The portal hands over a file descriptor and a node id; this module attaches a
//! PipeWire stream to that node and copies every frame into a tightly packed buffer
//! that both the encoder (through ffmpeg's stdin) and the GUI preview can read.
//!
//! DMA-BUF is deliberately not advertised: without a modifier property in the format
//! the compositor falls back to shared memory, which `StreamFlags::MAP_BUFFERS` maps
//! for us, so no GL/EGL import path is needed.

use anyhow::{anyhow, bail, Context, Result};
use log::{debug, error, info, warn};
use pipewire as pw;
use pw::spa;
use spa::param::format::{FormatProperties, MediaSubtype, MediaType};
use spa::param::video::VideoFormat;
use spa::param::ParamType;
use spa::pod::{ChoiceValue, Object, Pod, Property, PropertyFlags, Value};
use spa::utils::{Choice, ChoiceEnum, ChoiceFlags, Fraction, Id, Rectangle, SpaTypes};
use std::io::Cursor;
use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::thread;
use std::time::{Duration, Instant};

/// One captured frame, rows tightly packed (no padding).
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub pixel_format: PixelFormat,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Bgrx,
    Bgra,
    Rgbx,
    Rgba,
}

impl PixelFormat {
    /// Name of the equivalent ffmpeg pixel format.
    pub fn ffmpeg_name(self) -> &'static str {
        match self {
            PixelFormat::Bgrx => "bgr0",
            PixelFormat::Bgra => "bgra",
            PixelFormat::Rgbx => "rgb0",
            PixelFormat::Rgba => "rgba",
        }
    }

    fn from_spa(format: VideoFormat) -> Option<Self> {
        match format {
            VideoFormat::BGRx => Some(PixelFormat::Bgrx),
            VideoFormat::BGRA => Some(PixelFormat::Bgra),
            VideoFormat::RGBx => Some(PixelFormat::Rgbx),
            VideoFormat::RGBA => Some(PixelFormat::Rgba),
            _ => None,
        }
    }
}

/// Format negotiated with the compositor.
#[derive(Debug, Clone, Copy)]
pub struct NegotiatedFormat {
    pub width: u32,
    pub height: u32,
    pub pixel_format: PixelFormat,
    pub framerate: Option<u32>,
}

#[derive(Default)]
struct Shared {
    format: Mutex<Option<NegotiatedFormat>>,
    latest: Mutex<Option<Arc<Frame>>>,
    error: Mutex<Option<String>>,
    frames: AtomicU64,
}

pub struct PipeWireCapture {
    shared: Arc<Shared>,
    // Interior mutability so a capture shared between the preview and the encoder
    // can still be shut down through an `Arc`.
    stop: Mutex<Option<pw::channel::Sender<()>>>,
    thread: Mutex<Option<thread::JoinHandle<()>>>,
}

/// A portal session together with the PipeWire stream reading from it.
///
/// Preview and recording share one of these: opening a second session would mean a
/// second picker dialog and a second copy of every frame.
pub struct ScreenSource {
    /// Keeps the portal session (and therefore the PipeWire node) alive.
    pub session: crate::portal::ScreenCastSession,
    pub capture: Arc<PipeWireCapture>,
    pub format: NegotiatedFormat,
}

impl ScreenSource {
    pub fn start(restore_token: Option<String>, capture_cursor: bool) -> Result<Self> {
        let options = crate::portal::ScreenCastOptions {
            capture_cursor,
            restore_token,
        };
        let session = crate::portal::start_screencast(&options)?;
        let stream = session.primary_stream()?;
        let capture = Arc::new(PipeWireCapture::start(
            session.fd.try_clone().context("Failed to duplicate the PipeWire descriptor")?,
            stream.node_id,
        )?);
        let format = capture.wait_for_format(Duration::from_secs(10))?;
        Ok(Self {
            session,
            capture,
            format,
        })
    }

    pub fn restore_token(&self) -> Option<&str> {
        self.session.restore_token.as_deref()
    }
}

impl PipeWireCapture {
    /// Attach to `node_id` over the portal's PipeWire connection.
    pub fn start(fd: OwnedFd, node_id: u32) -> Result<Self> {
        let shared = Arc::new(Shared::default());
        let (stop_tx, stop_rx) = pw::channel::channel::<()>();

        let thread_shared = Arc::clone(&shared);
        let thread = thread::Builder::new()
            .name("pipewire-capture".into())
            .spawn(move || {
                if let Err(e) = run_stream(fd, node_id, Arc::clone(&thread_shared), stop_rx) {
                    error!("PipeWire capture failed: {:#}", e);
                    *thread_shared.error.lock().unwrap() = Some(format!("{:#}", e));
                }
            })
            .context("Failed to spawn the PipeWire capture thread")?;

        Ok(Self {
            shared,
            stop: Mutex::new(Some(stop_tx)),
            thread: Mutex::new(Some(thread)),
        })
    }

    /// Block until the compositor has negotiated a format (or the stream failed).
    pub fn wait_for_format(&self, timeout: Duration) -> Result<NegotiatedFormat> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(format) = *self.shared.format.lock().unwrap() {
                return Ok(format);
            }
            if let Some(err) = self.shared.error.lock().unwrap().clone() {
                bail!("{}", err);
            }
            if Instant::now() >= deadline {
                bail!("PipeWire did not negotiate a video format in time");
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn latest_frame(&self) -> Option<Arc<Frame>> {
        self.shared.latest.lock().unwrap().clone()
    }

    /// Block until at least one frame has been delivered.
    ///
    /// A compositor only pushes a frame when something on screen changes, so a
    /// recording started before the first buffer arrives would begin with whatever
    /// the stream's memory happened to hold.
    pub fn wait_for_frame(&self, timeout: Duration) -> Result<Arc<Frame>> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(frame) = self.latest_frame() {
                return Ok(frame);
            }
            if let Some(err) = self.error() {
                bail!("{}", err);
            }
            if Instant::now() >= deadline {
                bail!("The compositor did not deliver a frame in time");
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn frames_captured(&self) -> u64 {
        self.shared.frames.load(Ordering::Relaxed)
    }

    pub fn error(&self) -> Option<String> {
        self.shared.error.lock().unwrap().clone()
    }

    pub fn stop(&self) {
        if let Some(stop) = self.stop.lock().unwrap().take() {
            // Failure here just means the loop already exited.
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.lock().unwrap().take() {
            let _ = thread.join();
        }
    }

    /// Negotiated format, once the compositor has answered.
    pub fn format(&self) -> Option<NegotiatedFormat> {
        *self.shared.format.lock().unwrap()
    }
}

impl Drop for PipeWireCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

struct UserData {
    shared: Arc<Shared>,
    format: Option<NegotiatedFormat>,
}

fn run_stream(
    fd: OwnedFd,
    node_id: u32,
    shared: Arc<Shared>,
    stop_rx: pw::channel::Receiver<()>,
) -> Result<()> {
    static INIT: Once = Once::new();
    INIT.call_once(|| pw::init());

    let mainloop = pw::main_loop::MainLoop::new(None).context("Failed to create a PipeWire loop")?;
    let context =
        pw::context::Context::new(&mainloop).context("Failed to create a PipeWire context")?;
    let core = context
        .connect_fd(fd, None)
        .context("Failed to connect to PipeWire through the portal descriptor")?;

    let props = pw::properties::properties! {
        *pw::keys::MEDIA_TYPE => "Video",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => "Screen",
    };

    let stream = pw::stream::Stream::new(&core, "octocord-screencast", props)
        .context("Failed to create the PipeWire stream")?;

    let user_data = UserData {
        shared: Arc::clone(&shared),
        format: None,
    };

    let _listener = stream
        .add_local_listener_with_user_data(user_data)
        .state_changed(|_, user_data, old, new| {
            debug!("PipeWire stream state: {:?} -> {:?}", old, new);
            if let pw::stream::StreamState::Error(err) = &new {
                *user_data.shared.error.lock().unwrap() = Some(err.clone());
            }
        })
        .param_changed(|_, user_data, id, param| {
            if id != ParamType::Format.as_raw() {
                return;
            }
            let Some(param) = param else { return };
            match parse_format(param) {
                Ok(format) => {
                    info!(
                        "Screen capture negotiated: {}x{} {:?} @ {:?} fps",
                        format.width, format.height, format.pixel_format, format.framerate
                    );
                    user_data.format = Some(format);
                    *user_data.shared.format.lock().unwrap() = Some(format);
                }
                Err(e) => warn!("Ignoring unusable PipeWire format: {}", e),
            }
        })
        .process(|stream, user_data| {
            let Some(format) = user_data.format else { return };
            let Some(mut buffer) = stream.dequeue_buffer() else {
                debug!("PipeWire: out of buffers");
                return;
            };

            let datas = buffer.datas_mut();
            let Some(data) = datas.first_mut() else { return };

            let chunk_size = data.chunk().size() as usize;
            let chunk_stride = data.chunk().stride() as usize;
            let chunk_offset = data.chunk().offset() as usize;
            if chunk_size == 0 {
                // Compositors send empty chunks when nothing changed.
                return;
            }

            let Some(mapped) = data.data() else {
                // Only happens if the buffer is a DMA-BUF, which we never advertise.
                *user_data.shared.error.lock().unwrap() =
                    Some("PipeWire delivered an unmapped buffer".to_string());
                return;
            };

            let row_bytes = format.width as usize * 4;
            let stride = if chunk_stride > 0 { chunk_stride } else { row_bytes };
            let mut packed = vec![0u8; row_bytes * format.height as usize];

            for row in 0..format.height as usize {
                let start = chunk_offset + row * stride;
                let end = start + row_bytes;
                if end > mapped.len() {
                    break;
                }
                packed[row * row_bytes..(row + 1) * row_bytes].copy_from_slice(&mapped[start..end]);
            }

            let frame = Arc::new(Frame {
                width: format.width,
                height: format.height,
                pixel_format: format.pixel_format,
                data: packed,
            });
            *user_data.shared.latest.lock().unwrap() = Some(frame);
            user_data.shared.frames.fetch_add(1, Ordering::Relaxed);
        })
        .register()
        .context("Failed to register the PipeWire stream listener")?;

    let values = format_params().context("Failed to build the PipeWire format parameters")?;
    let pod = Pod::from_bytes(&values).ok_or_else(|| anyhow!("Malformed format parameters"))?;
    let mut params = [pod];

    stream
        .connect(
            spa::utils::Direction::Input,
            Some(node_id),
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )
        .context("Failed to connect the PipeWire stream")?;

    let quit_loop = mainloop.clone();
    let _stop = stop_rx.attach(mainloop.loop_(), move |()| quit_loop.quit());

    mainloop.run();
    let _ = stream.disconnect();
    Ok(())
}

/// `SPA_PARAM_EnumFormat` describing everything we can consume: packed 32-bit RGB
/// in any size and at any frame rate. No modifier property, so no DMA-BUF.
fn format_params() -> Result<Vec<u8>> {
    let object = Object {
        type_: SpaTypes::ObjectParamFormat.as_raw(),
        id: ParamType::EnumFormat.as_raw(),
        properties: vec![
            Property {
                key: FormatProperties::MediaType.as_raw(),
                flags: PropertyFlags::empty(),
                value: Value::Id(Id(MediaType::Video.as_raw())),
            },
            Property {
                key: FormatProperties::MediaSubtype.as_raw(),
                flags: PropertyFlags::empty(),
                value: Value::Id(Id(MediaSubtype::Raw.as_raw())),
            },
            Property {
                key: FormatProperties::VideoFormat.as_raw(),
                flags: PropertyFlags::empty(),
                value: Value::Choice(ChoiceValue::Id(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Enum {
                        default: Id(VideoFormat::BGRx.as_raw()),
                        alternatives: vec![
                            Id(VideoFormat::BGRx.as_raw()),
                            Id(VideoFormat::BGRA.as_raw()),
                            Id(VideoFormat::RGBx.as_raw()),
                            Id(VideoFormat::RGBA.as_raw()),
                        ],
                    },
                ))),
            },
            Property {
                key: FormatProperties::VideoSize.as_raw(),
                flags: PropertyFlags::empty(),
                value: Value::Choice(ChoiceValue::Rectangle(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Range {
                        default: Rectangle {
                            width: 1920,
                            height: 1080,
                        },
                        min: Rectangle {
                            width: 1,
                            height: 1,
                        },
                        max: Rectangle {
                            width: 16384,
                            height: 16384,
                        },
                    },
                ))),
            },
            Property {
                key: FormatProperties::VideoFramerate.as_raw(),
                flags: PropertyFlags::empty(),
                value: Value::Choice(ChoiceValue::Fraction(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Range {
                        default: Fraction { num: 60, denom: 1 },
                        min: Fraction { num: 0, denom: 1 },
                        max: Fraction {
                            num: 1000,
                            denom: 1,
                        },
                    },
                ))),
            },
        ],
    };

    let bytes = spa::pod::serialize::PodSerializer::serialize(
        Cursor::new(Vec::new()),
        &Value::Object(object),
    )
    .map_err(|e| anyhow!("Failed to serialize format parameters: {:?}", e))?
    .0
    .into_inner();

    Ok(bytes)
}

/// A property in a negotiated format is normally a plain value, but compositors are
/// allowed to answer with a single-option choice — take that choice's default.
fn choice_default<T: Copy + spa::pod::CanonicalFixedSizedPod>(choice: &ChoiceEnum<T>) -> T {
    match choice {
        ChoiceEnum::None(value) => *value,
        ChoiceEnum::Range { default, .. } => *default,
        ChoiceEnum::Step { default, .. } => *default,
        ChoiceEnum::Enum { default, .. } => *default,
        ChoiceEnum::Flags { default, .. } => *default,
    }
}

fn fixed_rectangle(value: &Value) -> Option<Rectangle> {
    match value {
        Value::Rectangle(rect) => Some(*rect),
        Value::Choice(ChoiceValue::Rectangle(Choice(_, choice))) => Some(choice_default(choice)),
        _ => None,
    }
}

fn fixed_id(value: &Value) -> Option<u32> {
    match value {
        Value::Id(id) => Some(id.0),
        Value::Choice(ChoiceValue::Id(Choice(_, choice))) => Some(choice_default(choice).0),
        _ => None,
    }
}

fn fixed_fraction(value: &Value) -> Option<Fraction> {
    match value {
        Value::Fraction(fraction) => Some(*fraction),
        Value::Choice(ChoiceValue::Fraction(Choice(_, choice))) => Some(choice_default(choice)),
        _ => None,
    }
}

/// Read the concrete size / pixel format out of the format the compositor picked.
fn parse_format(param: &Pod) -> Result<NegotiatedFormat> {
    let (_, value) = spa::pod::deserialize::PodDeserializer::deserialize_any_from(param.as_bytes())
        .map_err(|e| anyhow!("Could not parse the format pod: {:?}", e))?;

    let Value::Object(object) = value else {
        bail!("Format parameter was not an object");
    };

    let mut size: Option<Rectangle> = None;
    let mut pixel_format: Option<PixelFormat> = None;
    let mut framerate: Option<Fraction> = None;

    for property in &object.properties {
        let key = property.key;
        if key == FormatProperties::VideoSize.as_raw() {
            size = fixed_rectangle(&property.value);
        } else if key == FormatProperties::VideoFormat.as_raw() {
            pixel_format = fixed_id(&property.value).and_then(|id| PixelFormat::from_spa(VideoFormat(id)));
        } else if key == FormatProperties::VideoFramerate.as_raw() {
            framerate = fixed_fraction(&property.value);
        }
    }

    let size = size.ok_or_else(|| anyhow!("Negotiated format has no size"))?;
    let pixel_format =
        pixel_format.ok_or_else(|| anyhow!("Negotiated format uses an unsupported pixel layout"))?;
    if size.width == 0 || size.height == 0 {
        bail!("Negotiated format has an empty size");
    }

    Ok(NegotiatedFormat {
        width: size.width,
        height: size.height,
        pixel_format,
        framerate: framerate
            .filter(|f| f.denom > 0 && f.num > 0)
            .map(|f| (f.num / f.denom).max(1)),
    })
}
