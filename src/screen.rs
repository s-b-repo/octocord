use anyhow::Result;
use image::{DynamicImage, ImageBuffer};
use log::{info, error};
use screenshots::Screen;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use crossbeam::channel::{Sender, Receiver, bounded};
use std::panic::{catch_unwind, AssertUnwindSafe};

pub struct ScreenCapture {
    screen_index: usize,
    is_capturing: Arc<Mutex<bool>>,
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
            is_capturing: Arc::new(Mutex::new(false)),
            frame_sender: sender,
            frame_receiver: receiver,
            capture_thread: None,
            capture_rate: Duration::from_millis(33),
        })
    }

    pub fn start(&mut self) -> Result<()> {
        let is_capturing = Arc::clone(&self.is_capturing);
        let sender = self.frame_sender.clone();
        let screen_index = self.screen_index;
        let capture_rate = self.capture_rate;

        *is_capturing.lock().unwrap() = true;

        self.capture_thread = Some(thread::spawn(move || {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                info!("Screen capture thread started for screen {}", screen_index);

                let screens = match Screen::all() {
                    Ok(screens) => screens,
                    Err(e) => {
                        error!("Failed to get screens: {}", e);
                        return;
                    }
                };

                if screen_index >= screens.len() {
                    error!("Invalid screen index: {}", screen_index);
                    return;
                }

                let screen = &screens[screen_index];
                let mut last_capture = Instant::now();

                while *is_capturing.lock().unwrap() {
                    let now = Instant::now();

                    if now.duration_since(last_capture) >= capture_rate {
                        match screen.capture() {
                            Ok(image) => {
                                let (width, height) = image.dimensions();
                                let raw_pixels = image.as_raw();
                                if let Some(rgba_image) = ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(
                                    width,
                                    height,
                                    raw_pixels.to_vec(),
                                ) {
                                let dynamic_image = DynamicImage::ImageRgba8(rgba_image);
                                if let Err(e) = sender.try_send(dynamic_image) {
                                    log::debug!("Dropping screen frame ({}): channel full", e);
                                    // Drop frame if channel is full
                                    }
                                } else {
                                    error!("Failed to create image buffer");
                                }
                            }
                            Err(e) => {
                                error!("Failed to capture screen: {}", e);
                                thread::sleep(Duration::from_millis(100));
                            }
                        }
                        last_capture = now;
                    }

                    thread::sleep(Duration::from_millis(1));
                }

                info!("Screen capture thread stopped");
            }));
        }));

        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        *self.is_capturing.lock().unwrap() = false;

        if let Some(thread) = self.capture_thread.take() {
            thread.join().ok();
        }

        info!("Screen recording stopped");
        Ok(())
    }

    pub fn get_latest_frame(&self) -> Option<DynamicImage> {
        self.frame_receiver.try_recv().ok()
    }
}

#[cfg(feature = "screenshots")]
pub fn get_available_screens() -> Result<Vec<String>> {
    let screens = Screen::all()?;

    let mut screen_names = Vec::new();
    for (i, screen) in screens.iter().enumerate() {
        let dimensions = match screen.capture() {
            Ok(img) => {
                let (w, h) = img.dimensions();
                format!("{}x{}", w, h)
            }
            Err(_) => "unknown".to_string(),
        };
        screen_names.push(format!("Screen {} ({})", i, dimensions));
    }

    if screen_names.is_empty() {
        screen_names.push("Primary Screen".to_string());
    }

    Ok(screen_names)
}

#[cfg(not(feature = "screenshots"))]
pub fn get_available_screens() -> Result<Vec<String>> {
    Ok(vec!["Primary Screen (1920x1080)".to_string()])
}