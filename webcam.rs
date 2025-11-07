use anyhow::Result;
use std::sync::{Arc, Mutex};

#[cfg(feature = "webcam")]
use log::{info, error};
#[cfg(feature = "webcam")]
use nokhwa::Camera;
#[cfg(feature = "webcam")]
use nokhwa::pixel_format::RgbFormat;
#[cfg(feature = "webcam")]
use nokhwa::utils::{CameraFormat, FrameFormat, Resolution};
#[cfg(feature = "webcam")]
use nokhwa::utils::{CameraIndex, RequestedFormat, RequestedFormatType};
#[cfg(feature = "webcam")]
use std::thread;
#[cfg(feature = "webcam")]
use std::time::Duration;
#[cfg(feature = "webcam")]
use image::{DynamicImage, GenericImageView, GenericImage};
#[cfg(not(feature = "webcam"))]
use image::DynamicImage;
#[cfg(feature = "webcam")]
use crossbeam::channel::{Sender, Receiver, bounded};

#[cfg(feature = "webcam")]
pub struct WebcamCapture {
    camera: Option<Camera>,
    camera_index: CameraIndex,
    format: CameraFormat,
    is_capturing: Arc<Mutex<bool>>,
    frame_sender: Sender<DynamicImage>,
    frame_receiver: Receiver<DynamicImage>,
    capture_thread: Option<thread::JoinHandle<()>>,
}

#[cfg(feature = "webcam")]
impl WebcamCapture {
    pub fn new(camera_name: &str) -> Result<Self> {
        let camera_index = find_camera_index(camera_name)?;
        let format = CameraFormat::new(Resolution::new(640, 480), FrameFormat::MJPEG, 30);

        let (sender, receiver) = bounded(2);

        Ok(Self {
            camera: None,
            camera_index,
            format,
            is_capturing: Arc::new(Mutex::new(false)),
            frame_sender: sender,
            frame_receiver: receiver,
            capture_thread: None,
        })
    }

    pub fn start(&mut self) -> Result<()> {
        let requested_format = RequestedFormat::new::<RgbFormat>(RequestedFormatType::Closest(self.format));
        let mut camera = Camera::new(self.camera_index.clone(), requested_format)?;
        camera.open_stream()?;

        self.camera = Some(camera);

        let is_capturing = Arc::clone(&self.is_capturing);
        let sender = self.frame_sender.clone();
        let mut camera = self.camera.take().unwrap();

        *is_capturing.lock().unwrap() = true;

        self.capture_thread = Some(thread::spawn(move || {
            info!("Webcam capture thread started");

            while *is_capturing.lock().unwrap() {
                match camera.frame() {
                    Ok(frame) => match frame.decode_image::<RgbFormat>() {
                        Ok(image_buffer) => {
                            let image = DynamicImage::ImageRgb8(image_buffer);
                            if let Err(e) = sender.try_send(image) {
                                log::debug!("Dropping webcam frame ({}): channel full", e);
                                // Drop if channel is full
                            }
                        }
                        Err(e) => {
                            error!("Failed to decode webcam frame: {}", e);
                        }
                    },
                    Err(e) => {
                        error!("Failed to capture webcam frame: {}", e);
                        thread::sleep(Duration::from_millis(100));
                    }
                }

                thread::sleep(Duration::from_millis(33));
            }

            info!("Webcam capture thread stopped");
        }));

        info!("Webcam capture started");
        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        *self.is_capturing.lock().unwrap() = false;

        if let Some(thread) = self.capture_thread.take() {
            thread.join().ok();
        }

        info!("Webcam capture stopped");
        Ok(())
    }

    pub fn get_latest_frame(&self) -> Option<DynamicImage> {
        self.frame_receiver.try_recv().ok()
    }
}

#[cfg(feature = "webcam")]
fn find_camera_index(camera_name: &str) -> Result<CameraIndex> {
    let cameras = nokhwa::query(nokhwa::utils::ApiBackend::Auto)?;

    for (i, camera_info) in cameras.iter().enumerate() {
        if camera_info.human_name().contains(camera_name) {
            return Ok(CameraIndex::Index(i as u32));
        }
    }

    if !cameras.is_empty() {
        Ok(CameraIndex::Index(0))
    } else {
        Err(anyhow::anyhow!("No cameras found"))
    }
}

#[cfg(feature = "webcam")]
pub fn get_available_webcams() -> Result<Vec<String>> {
    let cameras = nokhwa::query(nokhwa::utils::ApiBackend::Auto)?;

    let mut webcam_names = Vec::new();

    for camera_info in cameras {
        webcam_names.push(camera_info.human_name());
    }

    if webcam_names.is_empty() {
        webcam_names.push("Default Webcam".to_string());
    }

    Ok(webcam_names)
}

#[cfg(feature = "webcam")]
#[allow(dead_code)]
pub struct WebcamOverlay {
    position: (u32, u32),
    size: (u32, u32),
    opacity: f32,
    border_color: [u8; 4],
    border_width: u32,
}

#[cfg(feature = "webcam")]
#[allow(dead_code)]
impl WebcamOverlay {
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            position: (x, y),
            size: (width, height),
            opacity: 1.0,
            border_color: [88, 101, 242, 255],
            border_width: 2,
        }
    }

    pub fn overlay_onto(&self, webcam_frame: &DynamicImage, screen_frame: &mut DynamicImage) {
        let (webcam_width, webcam_height) = webcam_frame.dimensions();
        let (overlay_width, overlay_height) = self.size;
        let (x, y) = self.position;

        let resized_webcam = if webcam_width != overlay_width || webcam_height != overlay_height {
            webcam_frame.resize_exact(overlay_width, overlay_height, image::imageops::FilterType::Lanczos3)
        } else {
            webcam_frame.clone()
        };

        let rgba_image = resized_webcam.to_rgba8();

        for (dx, dy, pixel) in rgba_image.enumerate_pixels() {
            let screen_x = x + dx;
            let screen_y = y + dy;

            if screen_x < screen_frame.width() && screen_y < screen_frame.height() {
                let mut screen_pixel = screen_frame.get_pixel(screen_x, screen_y);

                let alpha = pixel.0[3] as f32 / 255.0 * self.opacity;
                let inv_alpha = 1.0 - alpha;

                screen_pixel.0[0] = (pixel.0[0] as f32 * alpha + screen_pixel.0[0] as f32 * inv_alpha) as u8;
                screen_pixel.0[1] = (pixel.0[1] as f32 * alpha + screen_pixel.0[1] as f32 * inv_alpha) as u8;
                screen_pixel.0[2] = (pixel.0[2] as f32 * alpha + screen_pixel.0[2] as f32 * inv_alpha) as u8;

                screen_frame.put_pixel(screen_x, screen_y, screen_pixel);
            }
        }

        self.draw_border(screen_frame);
    }

    fn draw_border(&self, frame: &mut DynamicImage) {
        let (x, y) = self.position;
        let (width, height) = self.size;
        let color = self.border_color;

        for dx in 0..width {
            for dy in 0..self.border_width {
                if x + dx < frame.width() && y + dy < frame.height() {
                    frame.put_pixel(x + dx, y + dy, image::Rgba(color));
                }
            }
        }

        for dx in 0..width {
            for dy in 0..self.border_width {
                let border_y = y + height - dy - 1;
                if x + dx < frame.width() && border_y < frame.height() {
                    frame.put_pixel(x + dx, border_y, image::Rgba(color));
                }
            }
        }

        for dy in 0..height {
            for dx in 0..self.border_width {
                if x + dx < frame.width() && y + dy < frame.height() {
                    frame.put_pixel(x + dx, y + dy, image::Rgba(color));
                }
            }
        }

        for dy in 0..height {
            for dx in 0..self.border_width {
                let border_x = x + width - dx - 1;
                if border_x < frame.width() && y + dy < frame.height() {
                    frame.put_pixel(border_x, y + dy, image::Rgba(color));
                }
            }
        }
    }

    pub fn set_opacity(&mut self, opacity: f32) {
        self.opacity = opacity.clamp(0.0, 1.0);
    }
}

#[cfg(not(feature = "webcam"))]
pub struct WebcamCapture {
    is_capturing: Arc<Mutex<bool>>,
}

#[cfg(not(feature = "webcam"))]
impl WebcamCapture {
    pub fn new(_camera_name: &str) -> Result<Self> {
        Ok(Self {
            is_capturing: Arc::new(Mutex::new(false)),
        })
    }

    pub fn start(&mut self) -> Result<()> {
        *self.is_capturing.lock().unwrap() = true;
        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        *self.is_capturing.lock().unwrap() = false;
        Ok(())
    }

    pub fn get_latest_frame(&self) -> Option<DynamicImage> {
        None
    }
}

#[cfg(not(feature = "webcam"))]
pub fn get_available_webcams() -> Result<Vec<String>> {
    Ok(vec!["Default Webcam".to_string()])
}
