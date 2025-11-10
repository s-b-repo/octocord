use anyhow::{anyhow, Context, Result};
use chrono::Local;
use log::{error, warn, info};
use screenshots::Screen;
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::config::VideoQuality;
use crate::runtime::runtime_handle;
use once_cell::sync::OnceCell;

#[derive(Debug, Clone)]
pub struct RecorderOptions {
    pub output_directory: PathBuf,
    pub video_quality: VideoQuality,
    pub video_bitrate_kbps: u32,
    pub audio_bitrate_kbps: u32,
    pub audio_sample_rate: u32,
    pub frame_rate: u32,
    pub include_audio: bool,
    pub include_video: bool,
    pub include_webcam: bool,
    pub separate_outputs: bool,
    pub selected_screen: Option<usize>,
    pub audio_device: Option<String>,
    pub webcam_device: Option<String>,
    pub ffmpeg_path: String,
    pub audio_gain_db: f32,
}

#[derive(Debug, Clone)]
pub struct RecordingOutputs {
    pub combined: Option<PathBuf>,
    pub video_only: Option<PathBuf>,
    pub audio_only: Option<PathBuf>,
}

pub struct VideoEncoder {
    options: RecorderOptions,
    process: Option<Child>,
    outputs: Option<RecordingOutputs>,
    stdout_task: Option<tokio::task::JoinHandle<()>>,
    stderr_task: Option<tokio::task::JoinHandle<()>>,
}

impl VideoEncoder {
    pub fn new(options: RecorderOptions) -> Result<Self> {
        if !options.include_audio && !options.include_video && !options.include_webcam {
            return Err(anyhow!("At least one of audio, video, or webcam capture must be enabled"));
        }

        fs::create_dir_all(&options.output_directory)
            .with_context(|| format!("Failed to create output directory: {}", options.output_directory.display()))?;

        Ok(Self {
            options,
            process: None,
            outputs: None,
            stdout_task: None,
            stderr_task: None,
        })
    }

    pub fn start(&mut self) -> Result<()> {
        if self.process.is_some() {
            return Ok(());
        }

        ensure_ffmpeg_available(&self.options.ffmpeg_path)?;

        info!("Recorder options: {:?}", self.options);
        let (mut child, outputs) = build_ffmpeg(&self.options).with_context(|| "Failed to start ffmpeg with computed inputs/outputs")?;

        info!(
            "ffmpeg started. Outputs: {:?}",
            (
                outputs
                    .combined
                    .as_ref()
                    .map(|p| p.display().to_string()),
                outputs
                    .video_only
                    .as_ref()
                    .map(|p| p.display().to_string()),
                outputs
                    .audio_only
                    .as_ref()
                    .map(|p| p.display().to_string())
            )
        );

        // Drain stdout/stderr in background to avoid pipe blockage
        if let Some(stdout) = child.stdout.take() {
            let task = runtime_handle().spawn_blocking(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines().flatten() {
                    info!("ffmpeg: {}", line);
                }
            });
            self.stdout_task = Some(task);
        }
        if let Some(stderr) = child.stderr.take() {
            let task = runtime_handle().spawn_blocking(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().flatten() {
                    error!("ffmpeg: {}", line);
                }
            });
            self.stderr_task = Some(task);
        }

        self.outputs = Some(outputs);
        self.process = Some(child);
        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        if let Some(mut child) = self.process.take() {
            if let Some(stdin) = child.stdin.as_mut() {
                let _ = stdin.write_all(b"q\n");
            }

            let timeout = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(status) = child.try_wait()? {
                    info!("ffmpeg exited with status {}", status);
                    break;
                }

                if Instant::now() > timeout {
                    info!("ffmpeg did not exit gracefully, sending kill signal");
                    child.kill()?;
                    child.wait()?;
                    break;
                }

                thread::sleep(Duration::from_millis(100));
            }
        }

        // Detach log tasks to avoid blocking UI on stop
        if let Some(h) = self.stdout_task.take() { h.abort(); }
        if let Some(h) = self.stderr_task.take() { h.abort(); }

        Ok(())
    }

    pub fn toggle_pause(&mut self) -> Result<()> {
        if let Some(child) = self.process.as_mut() {
            if let Some(stdin) = child.stdin.as_mut() {
                if stdin.write_all(b"p\n").is_err() {
                    warn!("Pause toggle ignored: ffmpeg stdin not writable (process likely exited)");
                    return Ok(());
                }
                let _ = stdin.flush();
            }
        }
        Ok(())
    }
}
static PIPEWIRE_SUPPORTED: OnceCell<bool> = OnceCell::new();
static PULSE_SUPPORTED: OnceCell<bool> = OnceCell::new();

pub fn ffmpeg_supports_pipewire(ffmpeg_path: &str) -> bool {
    *PIPEWIRE_SUPPORTED.get_or_init(|| {
        let res = Command::new(ffmpeg_path)
            .arg("-v").arg("error")
            .arg("-f").arg("pipewire")
            .arg("-list_devices").arg("true")
            .arg("-i").arg("dummy")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        info!("ffmpeg pipewire support: {}", res);
        res
    })
}

pub fn ffmpeg_supports_pulse(ffmpeg_path: &str) -> bool {
    *PULSE_SUPPORTED.get_or_init(|| {
        let res = Command::new(ffmpeg_path)
            .arg("-v").arg("error")
            .arg("-f").arg("pulse")
            .arg("-sources").arg("true")
            .arg("-i").arg("dummy")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        info!("ffmpeg pulse support: {}", res);
        res
    })
}

impl Drop for VideoEncoder {
    fn drop(&mut self) {
        if let Err(err) = self.stop() {
            error!("Failed to stop ffmpeg process: {}", err);
        }
    }
}

fn ensure_ffmpeg_available(path: &str) -> Result<()> {
    Command::new(path)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("Failed to launch ffmpeg binary at '{}'", path))?
        .success()
        .then_some(())
        .ok_or_else(|| anyhow!("ffmpeg binary '{}' returned non-zero status", path))
}

fn ffmpeg_v4l2_accessible(ffmpeg_path: &str, device_path: &str) -> bool {
    Command::new(ffmpeg_path)
        .arg("-v").arg("error")
        .arg("-f").arg("v4l2")
        .arg("-list_formats").arg("all")
        .arg("-i").arg(device_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn build_ffmpeg(options: &RecorderOptions) -> Result<(Child, RecordingOutputs)> {
    let mut cmd = Command::new(&options.ffmpeg_path);
    cmd.arg("-y")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("warning")
        .arg("-stats")
        .arg("-threads").arg("0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut video_map: Option<String> = None;
    let mut audio_map: Option<String> = None;
    let mut filter_complex: Option<String> = None;
    let even_scale_filter = "trunc(iw/2)*2:trunc(ih/2)*2";
    let mut needs_even_scale = false;
    let effective_include_video = options.include_video;
    let mut effective_include_webcam = options.include_webcam;

    if effective_include_video {
        let wayland = std::env::var("WAYLAND_DISPLAY").is_ok();
        // Derive preference from environment to avoid struct field coupling
        let prefer_pipewire = std::env::var("OCTOCORD_USE_PIPEWIRE")
            .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE"))
            .unwrap_or(false);
        let have_display = std::env::var("DISPLAY").is_ok();

        if wayland && prefer_pipewire && !have_display {
            // Experimental PipeWire screen capture (requires xdg-desktop-portal + ffmpeg pipewire)
            cmd.arg("-thread_queue_size").arg("2048")
                .arg("-f").arg("pipewire")
                .arg("-i").arg("0");
            info!("Video input: pipewire (Wayland)");
            video_map = Some("0:v".to_string());
            needs_even_scale = true;
        } else {
            let screen_input = determine_screen_input(options.selected_screen)?;
            let video_size_str = screen_input.video_size.clone();
            cmd.arg("-thread_queue_size").arg("2048")
                .arg("-f").arg("x11grab")
                .arg("-framerate").arg(options.frame_rate.to_string())
                .arg("-probesize").arg("50M")
                .arg("-fflags").arg("+nobuffer")
                .arg("-use_wallclock_as_timestamps").arg("1")
                .arg("-video_size").arg(video_size_str.clone())
                .arg("-i").arg(screen_input.display_input);
            info!("Video input: x11grab {}", video_size_str);
            video_map = Some("0:v".to_string());
            needs_even_scale = true;
        }
    }

    if options.include_audio {
        // Choose ffmpeg audio backend
        // Default to pulse when available (common with PipeWire), else ALSA.
        let ff_backend = std::env::var("OCTOCORD_AUDIO_BACKEND").ok().unwrap_or_else(|| {
            if ffmpeg_supports_pulse(&options.ffmpeg_path) { "pulse".to_string() } else { "alsa".to_string() }
        });
        let ff_format = if ff_backend.eq_ignore_ascii_case("pulse") { "pulse" } else { "alsa" };
        // Use provided device when compatible, else logical default to avoid busy ALSA hw nodes
        let ff_device = match (ff_format, options.audio_device.clone()) {
            ("pulse", Some(dev)) if dev != "default" => dev,
            _ => "default".to_string(),
        };

        cmd.arg("-thread_queue_size").arg("2048")
            .arg("-f").arg(ff_format)
            .arg("-ac").arg("2")
            .arg("-ar").arg(options.audio_sample_rate.to_string())
            .arg("-i").arg(ff_device);
        let audio_index = if options.include_video { 1 } else { 0 };
        audio_map = Some(format!("{}:a", audio_index));
        info!("Audio input: {}:{} @ {} Hz", ff_format, "default", options.audio_sample_rate);
    }

    // Optionally include webcam only if a valid v4l2 path is resolved
    if effective_include_webcam {
        let requested = options
            .webcam_device
            .clone()
            .unwrap_or_else(|| "/dev/video0".to_string());

        let resolved = if requested.starts_with("/dev/video") && std::path::Path::new(&requested).exists() {
            Some(requested)
        } else {
            // Try to discover a usable v4l2 device
            (0..10)
                .map(|i| format!("/dev/video{}", i))
                .find(|p| std::path::Path::new(p).exists())
        };

        let resolved = resolved.and_then(|s| {
            if ffmpeg_v4l2_accessible(&options.ffmpeg_path, &s) { Some(s) } else { None }
        });

        if effective_include_webcam {
            match resolved {
                Some(webcam_source) => {
                    cmd.arg("-thread_queue_size").arg("512")
                        .arg("-f").arg("v4l2")
                        .arg("-framerate").arg("30")
                        .arg("-i").arg(webcam_source);
                }
                None => {
                    effective_include_webcam = false;
                    info!("Webcam device not accessible; continuing without webcam");
                }
            }
            let webcam_index = if options.include_video { 1 } else { 0 }
                + if options.include_audio { 1 } else { 0 };

            if options.include_video {
                filter_complex = Some(format!(
                    "[{webcam}:v]scale=640:-1[cam_scaled];[0:v][cam_scaled]overlay=W-w-40:H-h-40[overlayed];[overlayed]scale={filter}[vout]",
                    webcam = webcam_index,
                    filter = even_scale_filter
                ));
                video_map = Some("[vout]".to_string());
                needs_even_scale = false;
            } else {
                video_map = Some(format!("{}:v", webcam_index));
            }
        } else {
            info!("Webcam device not found; continuing without webcam overlay");
        }
    }

    if filter_complex.is_none() && needs_even_scale {
        cmd.arg("-vf").arg(format!("scale={}", even_scale_filter));
    }

    if let Some(filter) = filter_complex {
        cmd.arg("-filter_complex").arg(filter);
    }

    if !effective_include_video && effective_include_webcam {
        if video_map.is_none() {
            video_map = Some("0:v".to_string());
        }
    }

    if effective_include_video || effective_include_webcam {
        if video_map.is_none() {
            // If no video streams available, downgrade to audio-only if audio is enabled
            if !options.include_audio {
                return Err(anyhow!("Video/Webcam output requested but no video stream was configured"));
            }
        }
    }

    if options.include_audio {
        let volume_scale = 10f32.powf(options.audio_gain_db / 20.0);
        if (volume_scale - 1.0).abs() > f32::EPSILON {
            cmd.arg("-filter:a").arg(format!("volume={:.3}", volume_scale));
        }
    }

    cmd.arg("-shortest");
    
    // Compute outputs based on effective stream availability
    let outputs = prepare_output_paths_effective(options, effective_include_video || effective_include_webcam)?;

    if options.separate_outputs && options.include_audio && (effective_include_video || effective_include_webcam) {
        let video_stream = video_map
            .clone()
            .ok_or_else(|| anyhow!("Video output requested but no video stream available"))?;
        let video_output = outputs
            .video_only
            .as_ref()
            .ok_or_else(|| anyhow!("Expected video-only output path"))?;

        cmd.arg("-map").arg(video_stream)
            .arg("-c:v").arg("libx264")
            .arg("-preset").arg(preset_for_quality(options.video_quality))
            .arg("-crf").arg(crf_for_quality(options.video_quality).to_string())
            .arg("-pix_fmt").arg("yuv420p")
            .arg("-b:v").arg(format!("{}k", options.video_bitrate_kbps))
            .arg(video_output);

        let audio_stream = audio_map
            .clone()
            .ok_or_else(|| anyhow!("Audio output requested but no audio stream available"))?;
        let audio_output = outputs
            .audio_only
            .as_ref()
            .ok_or_else(|| anyhow!("Expected audio-only output path"))?;

        // Standalone audio file uses FLAC codec to match .flac container
        cmd.arg("-map").arg(audio_stream)
            .arg("-c:a").arg("flac")
            .arg("-ar").arg(options.audio_sample_rate.to_string())
            .arg(audio_output);
    } else {
        let combined_output = outputs
            .combined
            .as_ref()
            .ok_or_else(|| anyhow!("Expected combined output path"))?;

        if let Some(video_stream) = video_map.clone() {
            cmd.arg("-map").arg(video_stream)
                .arg("-c:v").arg("libx264")
                .arg("-preset").arg(preset_for_quality(options.video_quality))
                .arg("-crf").arg(crf_for_quality(options.video_quality).to_string())
                .arg("-pix_fmt").arg("yuv420p")
                .arg("-b:v").arg(format!("{}k", options.video_bitrate_kbps));
        }

        if let Some(audio_stream) = audio_map {
            // If combined has no video (audio-only flac), use FLAC codec. Otherwise AAC for MKV with video
            let use_flac = video_map.is_none();
            if use_flac {
                cmd.arg("-map").arg(audio_stream)
                    .arg("-c:a").arg("flac")
                    .arg("-ar").arg(options.audio_sample_rate.to_string());
            } else {
                cmd.arg("-map").arg(audio_stream)
                    .arg("-c:a").arg("aac")
                    .arg("-b:a").arg(format!("{}k", options.audio_bitrate_kbps))
                    .arg("-ar").arg(options.audio_sample_rate.to_string());
            }
        }

        cmd.arg(combined_output);
    }

    let child = cmd.spawn().context("Failed to spawn ffmpeg process")?;
    Ok((child, outputs))
}

struct ScreenCaptureInput {
    display_input: String,
    video_size: String,
}

fn determine_screen_input(screen_index: Option<usize>) -> Result<ScreenCaptureInput> {
    let display = env::var("DISPLAY").unwrap_or_else(|_| ":0.0".to_string());
    let screens = catch_unwind(AssertUnwindSafe(Screen::all))
        .map_err(|_| anyhow!("Screen capture backend crashed (missing Wayland screencopy support?)"))?
        .context("Failed to enumerate screens")?;

    let screen = if let Some(index) = screen_index {
        screens
            .get(index)
            .ok_or_else(|| anyhow!("Invalid screen index {}", index))?
    } else {
        screens
            .first()
            .ok_or_else(|| anyhow!("No screens detected"))?
    };

    let image = catch_unwind(AssertUnwindSafe(|| screen.capture()))
        .map_err(|_| anyhow!("Screen capture unsupported by compositor (missing ZwlrScreencopy?)"))?
        .context("Failed to capture screen to determine resolution")?;
    let (width, height) = image.dimensions();
    Ok(ScreenCaptureInput {
        display_input: format!("{}+0,0", display),
        video_size: format!("{}x{}", width, height),
    })
}

fn prepare_output_paths_effective(options: &RecorderOptions, any_video: bool) -> Result<RecordingOutputs> {
    let timestamp = Local::now().format("%Y%m%d_%H%M%S");
    let base_name = format!("recording_{}", timestamp);

    let combined = if options.separate_outputs && options.include_audio && any_video {
        None
    } else {
        let ext = if any_video { "mkv" } else { "flac" };
        Some(options.output_directory.join(format!("{}.{}", base_name, ext)))
    };

    let video_only = if options.separate_outputs && any_video && options.include_audio {
        Some(options.output_directory.join(format!("{}.video.mkv", base_name)))
    } else if !options.include_audio && any_video {
        Some(options.output_directory.join(format!("{}.mkv", base_name)))
    } else {
        None
    };

    let audio_only = if options.include_audio {
        if options.separate_outputs && any_video {
            Some(options.output_directory.join(format!("{}.audio.flac", base_name)))
        } else if !any_video {
            Some(options.output_directory.join(format!("{}.flac", base_name)))
        } else {
            None
        }
    } else {
        None
    };

    Ok(RecordingOutputs {
        combined,
        video_only,
        audio_only,
    })
}

fn crf_for_quality(quality: VideoQuality) -> u8 {
    match quality {
        VideoQuality::Low => 28,
        VideoQuality::Medium => 23,
        VideoQuality::High => 20,
        VideoQuality::Ultra => 18,
    }
}

fn preset_for_quality(_quality: VideoQuality) -> &'static str {
    "veryfast"
}
