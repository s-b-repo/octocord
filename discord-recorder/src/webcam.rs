use anyhow::Result;
use log::{info, error};
use nokhwa::{Camera, CameraFormat, FrameFormat, Resolution};
use nokhwa::utils::{CameraIndex, RequestedFormat, RequestedFormatType, RgbFormat};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use image::DynamicImage;

pub struct WebcamCapture {
    camera: Option<Camera>,
    camera_index: CameraIndex,
    format: CameraFormat,
    is_capturing: Arc<Mutex<bool>>,
    frame_sender: crossbeam::Sender<DynamicImage>,
    frame_receiver: crossbeam::Receiver<DynamicImage>,
    capture_thread: Option<thread::JoinHandle<()>>,
}

impl WebcamCapture {
    pub fn new(camera_name: &str) -> Result<Self> {
        let camera_index = find_camera_index(camera_name)?;
        let format = CameraFormat::new(
            Resolution::new(640, 480),
            FrameFormat::MJPEG,
            30,
        );
        
        let (sender, receiver) = crossbeam::unbounded();
        
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
        // Initialize camera
        let requested_format = RequestedFormat::new::<RgbFormat>(RequestedFormatType::Closest(self.format));
        let mut camera = Camera::new(self.camera_index.clone(), requested_format)?;
        camera.open_stream()?;
        
        self.camera = Some(camera);
        
        // Start capture thread
        let is_capturing = Arc::clone(&self.is_capturing);
        let sender = self.frame_sender.clone();
        let mut camera = self.camera.take().unwrap();

        *is_capturing.lock().unwrap() = true;

        self.capture_thread = Some(thread::spawn(move || {
            info!("Webcam capture thread started");
            
            while *is_capturing.lock().unwrap() {
                match camera.frame() {
                    Ok(frame) => {
                        let image = DynamicImage::ImageRgb8(frame);
                        if let Err(e) = sender.send(image) {
                            error!("Failed to send webcam frame: {}", e);
                            break;
                        }
                    }
                    Err(e) => {
                        error!("Failed to capture webcam frame: {}", e);
                        thread::sleep(Duration::from_millis(100));
                    }
                }
                
                thread::sleep(Duration::from_millis(33)); // ~30 FPS
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
        
        if let Some(mut camera) = self.camera.take() {
            camera.stop_stream()?;
        }
        
        info!("Webcam capture stopped");
        Ok(())
    }

    pub fn get_latest_frame(&self) -> Option<DynamicImage> {
        self.frame_receiver.try_recv().ok()
    }

    pub fn is_running(&self) -> bool {
        *self.is_capturing.lock().unwrap()
    }

    pub fn set_resolution(&mut self, width: u32, height: u32) -> Result<()> {
        self.format = CameraFormat::new(
            Resolution::new(width, height),
            FrameFormat::MJPEG,
            30,
        );
        Ok(())
    }

    pub fn get_resolution(&self) -> (u32, u32) {
        (self.format.width(), self.format.height())
    }

    pub fn get_available_resolutions(&self) -> Result<Vec<(u32, u32)>> {
        if let Some(camera) = &self.camera {
            let formats = camera.compatible_camera_formats()?;
            let mut resolutions = Vec::new();
            
            for format in formats {
                resolutions.push((format.width(), format.height()));
            }
            
            // Remove duplicates and sort
            resolutions.sort();
            resolutions.dedup();
            
            Ok(resolutions)
        } else {
            Ok(vec![
                (640, 480),
                (1280, 720),
                (1920, 1080),
            ])
        }
    }
}

fn find_camera_index(camera_name: &str) -> Result<CameraIndex> {
    let cameras = nokhwa::query(nokhwa::utils::ApiBackend::Auto)?;
    
    for (i, camera_info) in cameras.iter().enumerate() {
        if camera_info.human_name().contains(camera_name) {
            return Ok(CameraIndex::Index(i as u32));
        }
    }
    
    // Fall back to first available camera
    if !cameras.is_empty() {
        Ok(CameraIndex::Index(0))
    } else {
        Err(anyhow::anyhow!("No cameras found"))
    }
}

pub fn get_available_webcams() -> Result<Vec<String>> {
    let cameras = nokhwa::query(nokhwa::utils::ApiBackend::Auto)?;
    
    let mut webcam_names = Vec::new();
    
    for camera_info in cameras {
        webcam_names.push(camera_info.human_name());
    }
    
    // Add default if no cameras found
    if webcam_names.is_empty() {
        webcam_names.push("Default Webcam".to_string());
    }
    
    Ok(webcam_names)
}

// Webcam overlay utilities
pub struct WebcamOverlay {
    position: (u32, u32),
    size: (u32, u32),
    opacity: f32,
    border_color: [u8; 4],
    border_width: u32,
}

impl WebcamOverlay {
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            position: (x, y),
            size: (width, height),
            opacity: 1.0,
            border_color: [88, 101, 242, 255], // Discord blurple
            border_width: 2,
        }
    }

    pub fn overlay_onto(&self, webcam_frame: &DynamicImage, screen_frame: &mut DynamicImage) {
        let (webcam_width, webcam_height) = webcam_frame.dimensions();
        let (overlay_width, overlay_height) = self.size;
        
        // Resize webcam frame if needed
        let resized_webcam = if webcam_width != overlay_width || webcam_height != overlay_height {
            webcam_frame.resize_exact(overlay_width, overlay_height, image::imageops::FilterType::Lanczos3)
        } else {
            webcam_frame.clone()
        };

        // Overlay webcam onto screen frame
        let (x, y) = self.position;
        
        for (dy, row) in resized_webcam.pixels().chunks(overlay_width as usize).enumerate() {
            for (dx, pixel) in row.iter().enumerate() {
                let screen_x = x + dx as u32;
                let screen_y = y + dy as u32;
                
                if screen_x < screen_frame.width() && screen_y < screen_frame.height() {
                    let screen_pixel = screen_frame.get_pixel_mut(screen_x, screen_y);
                    
                    // Simple alpha blending
                    let alpha = pixel.0[3] as f32 / 255.0 * self.opacity;
                    let inv_alpha = 1.0 - alpha;
                    
                    screen_pixel.0[0] = (pixel.0[0] as f32 * alpha + screen_pixel.0[0] as f32 * inv_alpha) as u8;
                    screen_pixel.0[1] = (pixel.0[1] as f32 * alpha + screen_pixel.0[1] as f32 * inv_alpha) as u8;
                    screen_pixel.0[2] = (pixel.0[2] as f32 * alpha + screen_pixel.0[2] as f32 * inv_alpha) as u8;
                }
            }
        }

        // Draw border
        self.draw_border(screen_frame);
    }

    fn draw_border(&self, frame: &mut DynamicImage) {
        let (x, y) = self.position;
        let (width, height) = self.size;
        let color = self.border_color;
        
        // Top border
        for dx in 0..width {
            for dy in 0..self.border_width {
                if x + dx < frame.width() && y + dy < frame.height() {
                    frame.put_pixel(x + dx, y + dy, image::Rgba(color));
                }
            }
        }
        
        // Bottom border
        for dx in 0..width {
            for dy in 0..self.border_width {
                let border_y = y + height - dy - 1;
                if x + dx < frame.width() && border_y < frame.height() {
                    frame.put_pixel(x + dx, border_y, image::Rgba(color));
                }
            }
        }
        
        // Left border
        for dy in 0..height {
            for dx in 0..self.border_width {
                if x + dx < frame.width() && y + dy < frame.height() {
                    frame.put_pixel(x + dx, y + dy, image::Rgba(color));
                }
            }
        }
        
        // Right border
        for dy in 0..height {
            for dx in 0..self.border_width {
                let border_x = x + width - dx - 1;
                if border_x < frame.width() && y + dy < frame.height() {
                    frame.put_pixel(border_x, y + dy, image::Rgba(color));
                }
            }
        }
    }

    // Setters
    pub fn set_position(&mut self, x: u32, y: u32) {
        self.position = (x, y);
    }

    pub fn set_size(&mut self, width: u32, height: u32) {
        self.size = (width, height);
    }

    pub fn set_opacity(&mut self, opacity: f32) {
        self.opacity = opacity.clamp(0.0, 1.0);
    }

    pub fn set_border_color(&mut self, r: u8, g: u8, b: u8, a: u8) {
        self.border_color = [r, g, b, a];
    }

    // Getters
    pub fn get_position(&self) -> (u32, u32) {
        self.position
    }

    pub fn get_size(&self) -> (u32, u32) {
        self.size
    }
}

// Fallback implementation for when nokhwa is not available
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

    pub fn is_running(&self) -> bool {
        *self.is_capturing.lock().unwrap()
    }
}

#[cfg(not(feature = "webcam"))]
pub fn get_available_webcams() -> Result<Vec<String>> {
    Ok(vec!["Default Webcam".to_string()])
}