use anyhow::{anyhow, Result};
use crossbeam::channel::{bounded, Receiver, Sender};
use image::{DynamicImage, ImageBuffer};
use log::{error, info};
use screenshots::Screen;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::video::physical_geometry;

/// True when the session is a Wayland session.
///
/// It matters because the `screenshots` backend can only reach a Wayland desktop
/// through the xdg-desktop-portal Screenshot API, which round-trips a PNG through
/// DBus (and usually a permission prompt) for *every single frame*. That is fine for
/// a one-off screenshot and completely unusable as a 30 fps preview source, so the
/// live preview is disabled there instead of hanging the UI.
pub fn is_wayland_session() -> bool {
    std::env::var("WAYLAND_DISPLAY").is_ok()
        || std::env::var("XDG_SESSION_TYPE")
            .map(|t| t.eq_ignore_ascii_case("wayland"))
            .unwrap_or(false)
}

pub struct ScreenCapture {
    screen_index: usize,
    is_capturing: Arc<AtomicBool>,
    frame_sender: Sender<DynamicImage>,
    frame_receiver: Receiver<DynamicImage>,
    capture_thread: Option<thread::JoinHandle<()>>,
    capture_rate: Duration,
}

impl ScreenCapture {
    pub fn new(screen_index: usize) -> Result<Self> {
        let (sender, receiver) = bounded(2);

        Ok(Self {
            screen_index,
            is_capturing: Arc::new(AtomicBool::new(false)),
            frame_sender: sender,
            frame_receiver: receiver,
            capture_thread: None,
            capture_rate: Duration::from_millis(33),
        })
    }

    pub fn start(&mut self) -> Result<()> {
        if self.capture_thread.is_some() {
            return Ok(());
        }

        if is_wayland_session() {
            return Err(anyhow!(
                "Live screen preview is not available in a Wayland session"
            ));
        }

        let is_capturing = Arc::clone(&self.is_capturing);
        let sender = self.frame_sender.clone();
        let screen_index = self.screen_index;
        let capture_rate = self.capture_rate;

        is_capturing.store(true, Ordering::SeqCst);

        self.capture_thread = Some(thread::Builder::new()
            .name("screen-preview".into())
            .spawn(move || {
                let _ = catch_unwind(AssertUnwindSafe(|| {
                    info!("Screen capture thread started for screen {}", screen_index);

                    let screens = match Screen::all() {
                        Ok(screens) => screens,
                        Err(e) => {
                            error!("Failed to get screens: {}", e);
                            return;
                        }
                    };

                    let Some(screen) = screens.get(screen_index) else {
                        error!("Invalid screen index: {}", screen_index);
                        return;
                    };

                    while is_capturing.load(Ordering::Relaxed) {
                        let started = Instant::now();

                        match screen.capture() {
                            Ok(image) => {
                                let (width, height) = image.dimensions();
                                match ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(
                                    width,
                                    height,
                                    image.into_raw(),
                                ) {
                                    Some(rgba_image) => {
                                        let frame = DynamicImage::ImageRgba8(rgba_image);
                                        if let Err(e) = sender.try_send(frame) {
                                            // Preview frames are disposable: drop when the UI is behind.
                                            log::trace!("Dropping screen frame ({})", e);
                                        }
                                    }
                                    None => error!("Failed to create image buffer"),
                                }
                            }
                            Err(e) => {
                                error!("Failed to capture screen: {}", e);
                                thread::sleep(Duration::from_millis(500));
                            }
                        }

                        // Pace the loop by how long the capture actually took, so a slow
                        // backend degrades the frame rate instead of pegging a core.
                        if let Some(remaining) = capture_rate.checked_sub(started.elapsed()) {
                            thread::sleep(remaining);
                        }
                    }

                    info!("Screen capture thread stopped");
                }));
            })?);

        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        self.is_capturing.store(false, Ordering::SeqCst);

        if let Some(thread) = self.capture_thread.take() {
            thread.join().ok();
        }

        info!("Screen preview stopped");
        Ok(())
    }

    pub fn get_latest_frame(&self) -> Option<DynamicImage> {
        // Keep only the newest queued frame so the preview never lags behind.
        let mut latest = None;
        while let Ok(frame) = self.frame_receiver.try_recv() {
            latest = Some(frame);
        }
        latest
    }
}

impl Drop for ScreenCapture {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

/// Human readable list of the connected displays.
///
/// Geometry comes from the display metadata rather than from a test capture: a capture
/// costs a full screenshot per screen, which is slow on X11 and portal-prompting on Wayland.
pub fn get_available_screens() -> Result<Vec<String>> {
    let screens = catch_unwind(AssertUnwindSafe(Screen::all))
        .map_err(|_| anyhow!("Screen enumeration backend crashed"))?
        .unwrap_or_default();

    let mut screen_names: Vec<String> = screens
        .iter()
        .enumerate()
        .map(|(i, screen)| {
            let (x, y, width, height) = physical_geometry(&screen.display_info);
            let primary = if screen.display_info.is_primary { " *" } else { "" };
            format!("Screen {} ({}x{} @ {},{}){}", i, width, height, x, y, primary)
        })
        .collect();

    if screen_names.is_empty() {
        screen_names.push("Primary Screen".to_string());
    }

    Ok(screen_names)
}
