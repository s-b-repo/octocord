use anyhow::{anyhow, Result};
use ffmpeg_next::{
    codec::{self, encoder},
    format::{self, output},
        frame,
        software::scaling::{self, flag::Flags as ScaleFlags},
        util::{
            format::pixel,
                rational::Rational,
        },
        Dictionary,
        Packet,
};
use image::{DynamicImage, GenericImageView};
use log::{error, info};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, Once};
use std::thread;
use std::time::{Duration, Instant};

use crate::config::VideoQuality;

static FFMPEG_INIT: Once = Once::new();

pub struct VideoEncoder {
    output_path: String,
    width: u32,
    height: u32,
    frame_rate: u32,
    bitrate: u32,
    encoder: Option<Arc<Mutex<VideoEncoderContext>>>,
    frame_queue: Arc<Mutex<VecDeque<DynamicImage>>>,
    is_encoding: Arc<Mutex<bool>>,
    encoding_thread: Option<thread::JoinHandle<()>>,
}

struct VideoEncoderContext {
    encoder: encoder::Video,
    output_context: format::context::Output,
    video_stream_index: usize,
    scaler: scaling::Context,
    time_base: Rational,
    next_pts: i64,
}

impl VideoEncoder {
    pub fn new(output_path: &str, quality: VideoQuality) -> Result<Self> {
        let (width, height) = (1920, 1080);
        let frame_rate = 30;
        let bitrate = match quality {
            VideoQuality::Low => 1_000_000,
            VideoQuality::Medium => 2_500_000,
            VideoQuality::High => 5_000_000,
            VideoQuality::Ultra => 10_000_000,
        };

        Ok(Self {
            output_path: output_path.to_string(),
           width,
           height,
           frame_rate,
           bitrate,
           encoder: None,
           frame_queue: Arc::new(Mutex::new(VecDeque::new())),
           is_encoding: Arc::new(Mutex::new(false)),
           encoding_thread: None,
        })
    }

    pub fn start(&mut self) -> Result<()> {
        // Initialize FFmpeg only once per process
        let mut init_err: Option<anyhow::Error> = None;
        FFMPEG_INIT.call_once(|| {
            if let Err(e) = ffmpeg_next::init() {
                init_err = Some(anyhow!("Failed to initialize FFmpeg: {}", e));
            }
        });
        if let Some(e) = init_err {
            return Err(e);
        }

        // Create output context
        let mut output_context = output(&self.output_path)?;

        // Create video stream
        let video_stream = output_context.add_stream(codec::Id::H264)?;
        let video_stream_index = video_stream.index();

        // Find H.264 encoder
        let codec = encoder::find(codec::Id::H264).ok_or_else(|| anyhow!("H.264 encoder not found"))?;

        // Get codec context from stream and configure it
        let mut context = video_stream.codec();

        context.set_width(self.width as i32);
        context.set_height(self.height as i32);
        let time_base = Rational::new(1, self.frame_rate as i32);
        context.set_time_base(time_base);
        context.set_frame_rate(Some(Rational::new(self.frame_rate as i32, 1)));
        context.set_bit_rate(self.bitrate as i64);
        context.set_format(pixel::Pixel::YUV420P);

        // Open encoder
        let mut encoder = context.encoder().video()?;
        let dict = Dictionary::new(); // Could add options like "preset", "medium"
        let encoder = encoder.open_with(dict)?;

        // Set stream parameters
        video_stream.set_parameters(&encoder);

        // Prepare scaler from RGB24 -> YUV420P
        let scaler = scaling::Context::get(
            pixel::Pixel::RGB24,
            self.width,
            self.height,
            pixel::Pixel::YUV420P,
            self.width,
            self.height,
            ScaleFlags::BILINEAR,
        )?;

        // Write header
        output_context.write_header()?;

        let ctx = Arc::new(Mutex::new(VideoEncoderContext {
            encoder,
            output_context,
            video_stream_index,
            scaler,
            time_base,
            next_pts: 0,
        }));

        self.encoder = Some(ctx.clone());

        // Start encoding thread
        let frame_queue = Arc::clone(&self.frame_queue);
        let is_encoding = Arc::clone(&self.is_encoding);

        *is_encoding.lock().unwrap() = true;

        self.encoding_thread = Some(thread::spawn(move || {
            if let Err(e) = encoding_loop(ctx, frame_queue, is_encoding) {
                error!("Encoding error: {}", e);
            }
        }));

        info!("Video encoding started");
        Ok(())
    }

    pub fn add_frame(&mut self, frame: DynamicImage) -> Result<()> {
        let mut queue = self.frame_queue.lock().unwrap();
        queue.push_back(frame);
        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        // Signal the encoding loop to stop
        {
            let mut run = self.is_encoding.lock().unwrap();
            *run = false;
        }

        // Wait for the thread to finish
        if let Some(thread) = self.encoding_thread.take() {
            let _ = thread.join();
        }

        // Write trailer once encoding is flushed
        if let Some(encoder_context) = self.encoder.take() {
            let mut enc = encoder_context.lock().unwrap();
            enc.output_context.write_trailer()?;
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
    encoder_context: Arc<Mutex<VideoEncoderContext>>,
    frame_queue: Arc<Mutex<VecDeque<DynamicImage>>>,
    is_encoding: Arc<Mutex<bool>>,
) -> Result<(), anyhow::Error> {
    // Frame rate derived from time_base denominator
    let frame_rate = {
        let enc = encoder_context.lock().unwrap();
        enc.time_base.denominator()
    } as u32;

    let frame_duration = Duration::from_secs_f64(1.0 / frame_rate as f64);
    let mut last_frame_time = Instant::now();
    let mut frame_count = 0;

    while *is_encoding.lock().unwrap() {
        let mut queue = frame_queue.lock().unwrap();

        if let Some(frame) = queue.pop_front() {
            drop(queue);

            // Convert DynamicImage -> RGB24 frame
            let rgb_frame = convert_to_rgb_frame(frame)?;

            // Scale to YUV420P and set PTS
            let mut enc_ctx = encoder_context.lock().unwrap();

            let mut yuv_frame = frame::Video::new(pixel::Pixel::YUV420P, rgb_frame.width(), rgb_frame.height());
            enc_ctx.scaler.run(&rgb_frame, &mut yuv_frame)?;

            yuv_frame.set_pts(Some(enc_ctx.next_pts));
            enc_ctx.next_pts += 1;

            // Send frame to encoder
            enc_ctx.encoder.send_frame(&yuv_frame)?;

            // Receive and write packets
            let mut encoded = Packet::empty();
            while enc_ctx.encoder.receive_packet(&mut encoded).is_ok() {
                encoded.set_stream(enc_ctx.video_stream_index);
                let stream = enc_ctx.output_context.stream(enc_ctx.video_stream_index).unwrap();
                encoded.rescale_ts(enc_ctx.time_base, stream.time_base());
                enc_ctx.output_context.write_interleaved(&encoded).unwrap();
            }
            drop(enc_ctx);

            frame_count += 1;

            // Maintain frame rate pacing
            let elapsed = last_frame_time.elapsed();
            if elapsed < frame_duration {
                thread::sleep(frame_duration - elapsed);
            }
            last_frame_time = Instant::now();
        } else {
            drop(queue);
            thread::sleep(Duration::from_millis(1));
        }
    }

    // Flush encoder
    let mut enc_ctx = encoder_context.lock().unwrap();
    enc_ctx.encoder.send_eof()?;
    let mut encoded = Packet::empty();
    while enc_ctx.encoder.receive_packet(&mut encoded).is_ok() {
        encoded.set_stream(enc_ctx.video_stream_index);
        let stream = enc_ctx.output_context.stream(enc_ctx.video_stream_index).unwrap();
        encoded.rescale_ts(enc_ctx.time_base, stream.time_base());
        enc_ctx.output_context.write_interleaved(&encoded).unwrap();
    }
    drop(enc_ctx);

    info!("Encoded {} frames", frame_count);
    Ok(())
}

fn convert_to_rgb_frame(image: DynamicImage) -> Result<frame::Video> {
    let rgb_image = image.to_rgb8();
    let (width, height) = rgb_image.dimensions();

    let mut frame = frame::Video::new(pixel::Pixel::RGB24, width, height);

    let stride = (width as usize) * 3;
    let data = frame.data_mut(0);
    for (y, line) in rgb_image.as_raw().chunks_exact(stride).enumerate() {
        let offset = y * stride;
        data[offset..offset + stride].copy_from_slice(line);
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
            return Err(anyhow!("No frames to encode"));
        }

        info!("Encoding {} frames to {}", self.frames.len(), self.output_path);

        // Save frames as individual images (fallback behavior kept)
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
        VideoQuality::Low => (1280, 720),     // 720p
        VideoQuality::Medium => (1920, 1080), // 1080p
        VideoQuality::High => (2560, 1440),   // 1440p
        VideoQuality::Ultra => (3840, 2160),  // 4K
    }
}

pub fn get_video_bitrate(quality: VideoQuality) -> u32 {
    match quality {
        VideoQuality::Low => 1_000_000,    // 1 Mbps
        VideoQuality::Medium => 2_500_000, // 2.5 Mbps
        VideoQuality::High => 5_000_000,   // 5 Mbps
        VideoQuality::Ultra => 10_000_000, // 10 Mbps
    }
}
