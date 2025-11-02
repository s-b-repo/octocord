use anyhow::Result;
use ffmpeg_next::{
    codec::context::Context,
    encoder::video::Video as VideoEncoder,
    format::{input, output, Pixel},
    frame::Video as VideoFrame,
    media::Type,
    Rational,
};
use image::DynamicImage;
use log::{info, error};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::config::VideoQuality;

pub struct VideoEncoder {
    output_path: String,
    width: u32,
    height: u32,
    frame_rate: u32,
    bitrate: u32,
    encoder: Option<VideoEncoderContext>,
    frame_queue: Arc<Mutex<Vec<DynamicImage>>>,
    is_encoding: Arc<Mutex<bool>>,
    encoding_thread: Option<thread::JoinHandle<()>>,
}

struct VideoEncoderContext {
    encoder: VideoEncoder,
    output_context: ffmpeg_next::format::context::Output,
    video_stream_index: usize,
}

impl VideoEncoder {
    pub fn new(output_path: &str, quality: VideoQuality) -> Result<Self> {
        let (width, height) = (1920, 1080); // Default resolution
        let frame_rate = 30;
        let bitrate = match quality {
            VideoQuality::Low => 1000_000,
            VideoQuality::Medium => 2500_000,
            VideoQuality::High => 5000_000,
            VideoQuality::Ultra => 10000_000,
        };

        Ok(Self {
            output_path: output_path.to_string(),
            width,
            height,
            frame_rate,
            bitrate,
            encoder: None,
            frame_queue: Arc::new(Mutex::new(Vec::new())),
            is_encoding: Arc::new(Mutex::new(false)),
            encoding_thread: None,
        })
    }

    pub fn start(&mut self) -> Result<()> {
        // Initialize FFmpeg
        ffmpeg_next::init()?;

        // Create output context
        let mut output_context = output(&self.output_path)?;
        
        // Create video stream
        let video_stream = output_context.add_stream(ffmpeg_next::codec::Id::H264)?;
        let video_stream_index = video_stream.index();

        // Configure encoder
        let mut encoder_context = Context::new();
        encoder_context.set_codec(ffmpeg_next::codec::Id::H264);
        encoder_context.set_width(self.width);
        encoder_context.set_height(self.height);
        encoder_context.set_time_base(Rational::new(1, self.frame_rate as i32));
        encoder_context.set_frame_rate(Some(Rational::new(self.frame_rate as i32, 1)));
        encoder_context.set_bit_rate(self.bitrate as i64);
        encoder_context.set_pix_fmt(Pixel::YUV420P);

        // Open encoder
        let encoder = encoder_context.encoder().video()?;
        encoder.open()?;
        
        video_stream.set_parameters(&encoder);

        // Write header
        output_context.write_header()?;

        self.encoder = Some(VideoEncoderContext {
            encoder,
            output_context,
            video_stream_index,
        });

        // Start encoding thread
        let frame_queue = Arc::clone(&self.frame_queue);
        let is_encoding = Arc::clone(&self.is_encoding);
        let encoder_context = self.encoder.take().unwrap();

        *is_encoding.lock().unwrap() = true;

        self.encoding_thread = Some(thread::spawn(move || {
            if let Err(e) = encoding_loop(encoder_context, frame_queue, is_encoding) {
                error!("Encoding error: {}", e);
            }
        }));

        info!("Video encoding started");
        Ok(())
    }

    pub fn add_frame(&mut self, frame: DynamicImage) -> Result<()> {
        let mut queue = self.frame_queue.lock().unwrap();
        queue.push(frame);
        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        *self.is_encoding.lock().unwrap() = false;
        
        if let Some(thread) = self.encoding_thread.take() {
            thread.join().ok();
        }

        if let Some(mut encoder_context) = self.encoder.take() {
            // Write trailer
            encoder_context.output_context.write_trailer()?;
        }

        info!("Video encoding stopped");
        Ok(())
    }

    pub fn is_running(&self) -> bool {
        *self.is_encoding.lock().unwrap()
    }

    pub fn set_resolution(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }

    pub fn get_resolution(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

fn encoding_loop(
    mut encoder_context: VideoEncoderContext,
    frame_queue: Arc<Mutex<Vec<DynamicImage>>>,
    is_encoding: Arc<Mutex<bool>>,
) -> Result<()> {
    let frame_duration = Duration::from_secs_f64(1.0 / 30.0);
    let mut last_frame_time = std::time::Instant::now();
    let mut frame_count = 0;

    while *is_encoding.lock().unwrap() {
        let mut queue = frame_queue.lock().unwrap();
        
        if !queue.is_empty() {
            let frame = queue.remove(0);
            drop(queue);

            // Convert DynamicImage to VideoFrame
            let video_frame = convert_to_video_frame(frame)?;
            
            // Encode frame
            encoder_context.encoder.send_frame(&video_frame)?;
            
            // Receive and write packets
            let mut encoded = ffmpeg_next::Packet::empty();
            while encoder_context.encoder.receive_packet(&mut encoded).is_ok() {
                encoded.set_stream(encoder_context.video_stream_index);
                encoded.write_interleaved(&mut encoder_context.output_context)?;
            }
            
            frame_count += 1;
            
            // Maintain frame rate
            let elapsed = last_frame_time.elapsed();
            if elapsed < frame_duration {
                thread::sleep(frame_duration - elapsed);
            }
            last_frame_time = std::time::Instant::now();
        } else {
            drop(queue);
            thread::sleep(Duration::from_millis(1));
        }
    }

    // Flush encoder
    encoder_context.encoder.send_eof()?;
    
    let mut encoded = ffmpeg_next::Packet::empty();
    while encoder_context.encoder.receive_packet(&mut encoded).is_ok() {
        encoded.set_stream(encoder_context.video_stream_index);
        encoded.write_interleaved(&mut encoder_context.output_context)?;
    }

    info!("Encoded {} frames", frame_count);
    Ok(())
}

fn convert_to_video_frame(image: DynamicImage) -> Result<VideoFrame> {
    let rgb_image = image.to_rgb8();
    let (width, height) = rgb_image.dimensions();
    
    let mut frame = VideoFrame::new(Pixel::RGB24, width, height);
    
    // Copy pixel data
    for (y, line) in rgb_image.chunks_exact(width as usize * 3).enumerate() {
        let frame_line = frame.data_mut(0).chunks_exact_mut(width as usize * 3).nth(y).unwrap();
        frame_line.copy_from_slice(line);
    }
    
    Ok(frame)
}

// Alternative video encoder using simpler approach
pub struct SimpleVideoEncoder {
    output_path: String,
    frames: Vec<DynamicImage>,
    frame_rate: u32,
}

impl SimpleVideoEncoder {
    pub fn new(output_path: &str) -> Self {
        Self {
            output_path: output_path.to_string(),
            frames: Vec::new(),
            frame_rate: 30,
        }
    }

    pub fn add_frame(&mut self, frame: DynamicImage) {
        self.frames.push(frame);
    }

    pub fn encode(&mut self) -> Result<()> {
        if self.frames.is_empty() {
            return Err(anyhow::anyhow!("No frames to encode"));
        }

        // Use image crate to save as animated GIF or WebP as fallback
        let first_frame = &self.frames[0];
        let (width, height) = first_frame.dimensions();
        
        info!("Encoding {} frames to {}", self.frames.len(), self.output_path);
        
        // For now, save frames as individual images
        for (i, frame) in self.frames.iter().enumerate() {
            let frame_path = format!("{}_frame_{:04}.png", self.output_path, i);
            frame.save(&frame_path)?;
        }
        
        Ok(())
    }

    pub fn clear(&mut self) {
        self.frames.clear();
    }
}

// Video quality settings
pub fn get_video_encoder_settings(quality: VideoQuality) -> (u32, u32) {
    match quality {
        VideoQuality::Low => (1280, 720),      // 720p
        VideoQuality::Medium => (1920, 1080),   // 1080p
        VideoQuality::High => (2560, 1440),     // 1440p
        VideoQuality::Ultra => (3840, 2160),    // 4K
    }
}

pub fn get_video_bitrate(quality: VideoQuality) -> u32 {
    match quality {
        VideoQuality::Low => 1000_000,      // 1 Mbps
        VideoQuality::Medium => 2500_000,   // 2.5 Mbps
        VideoQuality::High => 5000_000,     // 5 Mbps
        VideoQuality::Ultra => 10000_000,   // 10 Mbps
    }
}