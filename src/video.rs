use anyhow::{anyhow, Context, Result};
use chrono::Local;
use log::{error, info, warn};
use once_cell::sync::OnceCell;
use screenshots::Screen;
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::config::{AudioSource, EncoderPreference, OutputResolution, VideoQuality};
use crate::pipewire_capture::PipeWireCapture;

/// Render node used for VAAPI. Every Intel/AMD system exposes the first one here.
const VAAPI_DEVICE: &str = "/dev/dri/renderD128";

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
    pub audio_source: AudioSource,
    pub audio_device: Option<String>,
    pub webcam_device: Option<String>,
    pub ffmpeg_path: String,
    pub audio_gain_db: f32,
    pub output_resolution: OutputResolution,
    pub encoder: EncoderPreference,
}

impl RecorderOptions {
    /// Options that only need an output directory; everything else is a sane default.
    /// Mostly useful in tests.
    pub fn new(output_directory: PathBuf) -> Self {
        Self {
            output_directory,
            video_quality: VideoQuality::High,
            video_bitrate_kbps: 5_000,
            audio_bitrate_kbps: 256,
            audio_sample_rate: 48_000,
            frame_rate: 60,
            include_audio: false,
            include_video: true,
            include_webcam: false,
            separate_outputs: false,
            selected_screen: None,
            audio_source: AudioSource::System,
            audio_device: None,
            webcam_device: None,
            ffmpeg_path: "ffmpeg".to_string(),
            audio_gain_db: 0.0,
            output_resolution: OutputResolution::Native,
            encoder: EncoderPreference::Auto,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RecordingOutputs {
    pub combined: Option<PathBuf>,
    pub video_only: Option<PathBuf>,
    pub audio_only: Option<PathBuf>,
}

/// Where the video frames come from.
enum VideoSource {
    /// Frames pulled from the compositor and piped into ffmpeg's stdin.
    PipeWire {
        width: u32,
        height: u32,
        pixel_format: &'static str,
    },
    /// Classic X11 root-window grab.
    X11Grab {
        display_input: String,
        video_size: String,
    },
    None,
}

/// Copies frames into ffmpeg's stdin at a fixed cadence.
struct FrameFeeder {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl FrameFeeder {
    fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

pub struct VideoEncoder {
    options: RecorderOptions,
    screen: Option<Arc<PipeWireCapture>>,
    process: Option<Child>,
    outputs: Option<RecordingOutputs>,
    stdout_task: Option<thread::JoinHandle<()>>,
    stderr_task: Option<thread::JoinHandle<()>>,
    feeder: Option<FrameFeeder>,
    hardware_used: bool,
}

impl VideoEncoder {
    pub fn new(options: RecorderOptions) -> Result<Self> {
        if !options.include_audio && !options.include_video && !options.include_webcam {
            return Err(anyhow!(
                "At least one of audio, video, or webcam capture must be enabled"
            ));
        }

        fs::create_dir_all(&options.output_directory).with_context(|| {
            format!(
                "Failed to create output directory: {}",
                options.output_directory.display()
            )
        })?;

        Ok(Self {
            options,
            screen: None,
            process: None,
            outputs: None,
            stdout_task: None,
            stderr_task: None,
            feeder: None,
            hardware_used: false,
        })
    }

    /// Use an already running compositor capture as the video source.
    pub fn set_screen_source(&mut self, capture: Arc<PipeWireCapture>) {
        self.screen = Some(capture);
    }

    pub fn outputs(&self) -> Option<&RecordingOutputs> {
        self.outputs.as_ref()
    }

    pub fn hardware_encoding(&self) -> bool {
        self.hardware_used
    }

    pub fn start(&mut self) -> Result<()> {
        if self.process.is_some() {
            return Ok(());
        }

        ensure_ffmpeg_available(&self.options.ffmpeg_path)?;

        let source = self.resolve_video_source()?;
        let use_hardware = should_use_hardware(&self.options);
        self.hardware_used = use_hardware;

        info!(
            "Recording {} -> {} ({}, {} fps)",
            describe_source(&source),
            self.options.output_resolution.label(),
            if use_hardware { "VAAPI" } else { "libx264" },
            self.options.frame_rate
        );

        let (mut child, outputs) = build_ffmpeg(&self.options, &source, use_hardware)
            .context("Failed to start ffmpeg with the computed inputs and outputs")?;

        info!(
            "ffmpeg started. Outputs: {:?}",
            (
                outputs.combined.as_ref().map(|p| p.display().to_string()),
                outputs.video_only.as_ref().map(|p| p.display().to_string()),
                outputs.audio_only.as_ref().map(|p| p.display().to_string())
            )
        );

        // Feed compositor frames into ffmpeg's stdin before draining the logs, so no
        // frames are lost while the pipes are being set up.
        if let VideoSource::PipeWire { width, height, .. } = source {
            let capture = self
                .screen
                .clone()
                .ok_or_else(|| anyhow!("internal: PipeWire source without a capture"))?;
            let stdin = child
                .stdin
                .take()
                .ok_or_else(|| anyhow!("ffmpeg did not provide a stdin pipe"))?;
            self.feeder = Some(spawn_frame_feeder(
                capture,
                stdin,
                width,
                height,
                self.options.frame_rate,
            ));
        }

        // Drain stdout/stderr in background to avoid pipe blockage.
        // Both threads end on their own once ffmpeg exits and the pipes hit EOF.
        if let Some(stdout) = child.stdout.take() {
            let task = thread::Builder::new()
                .name("ffmpeg-stdout".into())
                .spawn(move || {
                    let reader = BufReader::new(stdout);
                    for line in reader.lines().map_while(|line| line.ok()) {
                        info!("ffmpeg: {}", line);
                    }
                })?;
            self.stdout_task = Some(task);
        }
        if let Some(stderr) = child.stderr.take() {
            let task = thread::Builder::new()
                .name("ffmpeg-stderr".into())
                .spawn(move || {
                    let reader = BufReader::new(stderr);
                    for line in reader.lines().map_while(|line| line.ok()) {
                        // ffmpeg writes progress stats and warnings to stderr, so
                        // this is not necessarily an error.
                        info!("ffmpeg: {}", line);
                    }
                })?;
            self.stderr_task = Some(task);
        }

        self.outputs = Some(outputs);
        self.process = Some(child);
        Ok(())
    }

    fn resolve_video_source(&self) -> Result<VideoSource> {
        if !self.options.include_video {
            return Ok(VideoSource::None);
        }

        if let Some(capture) = &self.screen {
            let format = capture.format().ok_or_else(|| {
                anyhow!("The compositor capture has not negotiated a format yet")
            })?;
            // Make sure real pixels are already flowing: ffmpeg is told the exact
            // geometry up front, and starting the pipe before the first buffer
            // arrives is what produces a recording that opens on a blank screen.
            let frame = capture
                .wait_for_frame(Duration::from_secs(5))
                .context("No frames arrived from the compositor")?;
            if frame.width != format.width || frame.height != format.height {
                return Err(anyhow!(
                    "Capture geometry changed while starting ({}x{} vs {}x{})",
                    frame.width,
                    frame.height,
                    format.width,
                    format.height
                ));
            }
            return Ok(VideoSource::PipeWire {
                width: format.width,
                height: format.height,
                pixel_format: format.pixel_format.ffmpeg_name(),
            });
        }

        let screen_input = determine_screen_input(self.options.selected_screen)?;
        Ok(VideoSource::X11Grab {
            display_input: screen_input.display_input,
            video_size: screen_input.video_size,
        })
    }

    pub fn stop(&mut self) -> Result<()> {
        // Stop feeding first: ffmpeg sees EOF on stdin and finalises the file.
        if let Some(feeder) = self.feeder.as_mut() {
            feeder.stop();
        }
        self.feeder = None;

        if let Some(mut child) = self.process.take() {
            if let Some(stdin) = child.stdin.as_mut() {
                let _ = stdin.write_all(b"q\n");
            }
            // Closing stdin is what tells a pipe-fed ffmpeg that the stream ended.
            drop(child.stdin.take());

            let timeout = Instant::now() + Duration::from_secs(10);
            loop {
                if let Some(status) = child.try_wait()? {
                    info!("ffmpeg exited with status {}", status);
                    break;
                }

                if Instant::now() > timeout {
                    warn!("ffmpeg did not exit gracefully, sending kill signal");
                    child.kill()?;
                    child.wait()?;
                    break;
                }

                thread::sleep(Duration::from_millis(100));
            }
        }

        // The pipes are closed now that ffmpeg is gone, so both drains return promptly.
        if let Some(h) = self.stdout_task.take() {
            let _ = h.join();
        }
        if let Some(h) = self.stderr_task.take() {
            let _ = h.join();
        }

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

impl Drop for VideoEncoder {
    fn drop(&mut self) {
        if let Err(err) = self.stop() {
            error!("Failed to stop ffmpeg process: {}", err);
        }
    }
}

/// Push the newest captured frame into ffmpeg at a fixed rate.
///
/// A compositor only sends a frame when something changed, so the timeline is kept
/// steady here by repeating the last frame. That keeps rawvideo timestamps honest and
/// avoids the drift a variable-rate pipe would cause against the live audio input.
fn spawn_frame_feeder(
    capture: Arc<PipeWireCapture>,
    mut stdin: std::process::ChildStdin,
    width: u32,
    height: u32,
    frame_rate: u32,
) -> FrameFeeder {
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let interval = Duration::from_secs_f64(1.0 / frame_rate.max(1) as f64);

    let handle = thread::Builder::new()
        .name("ffmpeg-feeder".into())
        .spawn(move || {
            let mut next = Instant::now();
            let mut mismatch_logged = false;
            let mut written: u64 = 0;

            while !thread_stop.load(Ordering::Relaxed) {
                if let Some(frame) = capture.latest_frame() {
                    if frame.width != width || frame.height != height {
                        if !mismatch_logged {
                            warn!(
                                "Capture resized to {}x{} mid-recording; dropping frames that no longer fit {}x{}",
                                frame.width, frame.height, width, height
                            );
                            mismatch_logged = true;
                        }
                    } else if let Err(e) = stdin.write_all(&frame.data) {
                        // Broken pipe simply means ffmpeg is gone.
                        info!("Stopped feeding frames: {}", e);
                        break;
                    } else {
                        written += 1;
                    }
                }

                next += interval;
                let now = Instant::now();
                if next > now {
                    thread::sleep(next - now);
                } else {
                    // Fell behind (slow disk, slow encoder): resync instead of spinning.
                    next = now;
                }
            }

            let _ = stdin.flush();
            info!("Frame feeder finished after {} frames", written);
        })
        .ok();

    FrameFeeder { stop, handle }
}

static PULSE_SUPPORTED: OnceCell<bool> = OnceCell::new();
static VAAPI_SUPPORTED: OnceCell<bool> = OnceCell::new();

pub fn ffmpeg_supports_pulse(ffmpeg_path: &str) -> bool {
    *PULSE_SUPPORTED.get_or_init(|| {
        let res = Command::new(ffmpeg_path)
            .args(["-v", "error", "-f", "pulse", "-sources", "true", "-i", "dummy"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        info!("ffmpeg pulse support: {}", res);
        res
    })
}

/// Whether this machine can actually encode H.264 on the GPU.
///
/// The probe runs a real one-frame encode: the presence of the encoder and of a render
/// node says nothing about whether the driver will accept the pipeline.
pub fn hardware_encoding_available(ffmpeg_path: &str) -> bool {
    *VAAPI_SUPPORTED.get_or_init(|| {
        if !std::path::Path::new(VAAPI_DEVICE).exists() {
            info!("VAAPI unavailable: {} is missing", VAAPI_DEVICE);
            return false;
        }
        let res = Command::new(ffmpeg_path)
            .args([
                "-v", "error",
                "-vaapi_device", VAAPI_DEVICE,
                "-f", "lavfi",
                "-i", "testsrc=size=64x64:rate=10:duration=0.2",
                "-vf", "format=nv12,hwupload",
                "-c:v", "h264_vaapi",
                "-f", "null", "-",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        info!("VAAPI hardware encoding available: {}", res);
        res
    })
}

fn should_use_hardware(options: &RecorderOptions) -> bool {
    match options.encoder {
        EncoderPreference::Software => false,
        EncoderPreference::Hardware | EncoderPreference::Auto => {
            let available = hardware_encoding_available(&options.ffmpeg_path);
            if !available && options.encoder == EncoderPreference::Hardware {
                warn!("Hardware encoding was requested but is unavailable; using libx264");
            }
            available
        }
    }
}

fn describe_source(source: &VideoSource) -> String {
    match source {
        VideoSource::PipeWire { width, height, pixel_format } => {
            format!("compositor {}x{} {}", width, height, pixel_format)
        }
        VideoSource::X11Grab { video_size, display_input } => {
            format!("x11grab {} at {}", video_size, display_input)
        }
        VideoSource::None => "audio only".to_string(),
    }
}

/// Verify the ffmpeg binary can be launched.
///
/// Worth calling before anything user-visible happens: without ffmpeg the recording
/// cannot start at all, and asking the compositor to share a screen first would put a
/// permission dialog in front of an error that was already knowable.
pub fn ensure_ffmpeg_available(path: &str) -> Result<()> {
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
        .args(["-v", "error", "-f", "v4l2", "-list_formats", "all", "-i", device_path])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// One `-f ... -i ...` audio input.
struct AudioInput {
    format: &'static str,
    device: String,
}

fn audio_inputs(options: &RecorderOptions) -> Vec<AudioInput> {
    let explicit = options
        .audio_device
        .as_deref()
        .filter(|d| !d.is_empty() && !d.eq_ignore_ascii_case("default"))
        .map(str::to_string);

    if !ffmpeg_supports_pulse(&options.ffmpeg_path) {
        // ALSA cannot address a monitor source, so this is microphone only.
        if matches!(options.audio_source, AudioSource::System | AudioSource::Both) {
            warn!("System audio needs the pulse backend; falling back to the ALSA capture device");
        }
        return vec![AudioInput {
            format: "alsa",
            device: explicit.unwrap_or_else(|| "default".to_string()),
        }];
    }

    // Desktop audio lives on the monitor of the default sink; the default *source*
    // is the microphone.
    let system = AudioInput {
        format: "pulse",
        device: "@DEFAULT_MONITOR@".to_string(),
    };
    let microphone = AudioInput {
        format: "pulse",
        device: explicit.unwrap_or_else(|| "default".to_string()),
    };

    match options.audio_source {
        AudioSource::System => vec![system],
        AudioSource::Microphone => vec![microphone],
        AudioSource::Both => vec![system, microphone],
    }
}

/// Scale step that honours the requested resolution without ever upscaling, and
/// always lands on even dimensions (yuv420p and every hardware encoder need that).
fn scale_step(resolution: OutputResolution) -> String {
    match resolution.target() {
        None => "scale=trunc(iw/2)*2:trunc(ih/2)*2".to_string(),
        Some((width, height)) => format!(
            "scale=w='min(iw,{})':h='min(ih,{})':force_original_aspect_ratio=decrease:force_divisible_by=2",
            width, height
        ),
    }
}

fn build_ffmpeg(
    options: &RecorderOptions,
    source: &VideoSource,
    use_hardware: bool,
) -> Result<(Child, RecordingOutputs)> {
    let mut cmd = Command::new(&options.ffmpeg_path);
    cmd.args(["-y", "-hide_banner", "-loglevel", "warning", "-stats"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if use_hardware {
        cmd.args(["-vaapi_device", VAAPI_DEVICE]);
    }

    // --- inputs, in the order ffmpeg will number them ----------------------
    let mut next_index = 0usize;

    let video_index = match source {
        VideoSource::PipeWire {
            width,
            height,
            pixel_format,
        } => {
            cmd.args(["-f", "rawvideo"])
                .args(["-pixel_format", pixel_format])
                .args(["-video_size", &format!("{}x{}", width, height)])
                .args(["-framerate", &options.frame_rate.to_string()])
                .args(["-i", "pipe:0"]);
            let index = next_index;
            next_index += 1;
            Some(index)
        }
        VideoSource::X11Grab {
            display_input,
            video_size,
        } => {
            cmd.args(["-thread_queue_size", "2048"])
                .args(["-f", "x11grab"])
                .args(["-framerate", &options.frame_rate.to_string()])
                .args(["-probesize", "50M"])
                .args(["-fflags", "+nobuffer"])
                .args(["-video_size", video_size])
                .args(["-i", display_input]);
            let index = next_index;
            next_index += 1;
            Some(index)
        }
        VideoSource::None => None,
    };

    let mut audio_indices: Vec<usize> = Vec::new();
    if options.include_audio {
        for input in audio_inputs(options) {
            cmd.args(["-thread_queue_size", "2048"])
                .args(["-f", input.format])
                .args(["-ac", "2"])
                .args(["-ar", &options.audio_sample_rate.to_string()])
                .args(["-i", &input.device]);
            info!(
                "Audio input {}: {}:{} @ {} Hz",
                next_index, input.format, input.device, options.audio_sample_rate
            );
            audio_indices.push(next_index);
            next_index += 1;
        }
    }

    let mut webcam_index = None;
    if options.include_webcam {
        match resolve_webcam(options) {
            Some(device) => {
                info!("Webcam input {}: v4l2 {}", next_index, device);
                cmd.args(["-thread_queue_size", "512"])
                    .args(["-f", "v4l2"])
                    .args(["-framerate", "30"])
                    .args(["-i", &device]);
                webcam_index = Some(next_index);
            }
            None => warn!("Webcam device not accessible; continuing without webcam"),
        }
    }

    // --- filter graph ------------------------------------------------------
    let mut chains: Vec<String> = Vec::new();

    let mut video_label = video_index.map(|i| format!("{}:v", i));
    if let Some(cam) = webcam_index {
        match video_label.take() {
            Some(base) => {
                chains.push(format!("[{}:v]scale=640:-2[cam]", cam));
                chains.push(format!("[{}][cam]overlay=W-w-40:H-h-40[ovl]", base));
                video_label = Some("ovl".to_string());
            }
            // Webcam-only recording: the camera is the video stream.
            None => video_label = Some(format!("{}:v", cam)),
        }
    }

    if let Some(base) = video_label.take() {
        let pixel_step = if use_hardware {
            "format=nv12,hwupload"
        } else {
            "format=yuv420p"
        };
        chains.push(format!(
            "[{}]{},{}[vout]",
            base,
            scale_step(options.output_resolution),
            pixel_step
        ));
        video_label = Some("vout".to_string());
    }

    let any_video = video_label.is_some();
    if !any_video && !options.include_audio {
        return Err(anyhow!(
            "Video capture was requested but no video stream could be configured"
        ));
    }

    let audio_label = if audio_indices.is_empty() {
        None
    } else {
        let mut steps: Vec<String> = Vec::new();
        let gain = 10f32.powf(options.audio_gain_db / 20.0);
        if (gain - 1.0).abs() > f32::EPSILON {
            steps.push(format!("volume={:.3}", gain));
        }
        if any_video {
            // Keeps live audio aligned with the paced video timeline.
            steps.push("aresample=async=1:first_pts=0".to_string());
        }
        if steps.is_empty() {
            steps.push("anull".to_string());
        }

        if audio_indices.len() > 1 {
            let sources: String = audio_indices
                .iter()
                .map(|i| format!("[{}:a]", i))
                .collect::<Vec<_>>()
                .join("");
            chains.push(format!(
                "{}amix=inputs={}:duration=longest:dropout_transition=0,{}[aout]",
                sources,
                audio_indices.len(),
                steps.join(",")
            ));
        } else {
            chains.push(format!(
                "[{}:a]{}[aout]",
                audio_indices[0],
                steps.join(",")
            ));
        }
        Some("aout".to_string())
    };

    if !chains.is_empty() {
        cmd.args(["-filter_complex", &chains.join(";")]);
    }

    // --- outputs -----------------------------------------------------------
    let outputs = prepare_output_paths(options, any_video)?;

    if options.separate_outputs && audio_label.is_some() && any_video {
        let video_output = outputs
            .video_only
            .as_ref()
            .ok_or_else(|| anyhow!("Expected a video-only output path"))?;
        cmd.args(["-map", "[vout]"]);
        apply_video_codec(&mut cmd, options, use_hardware);
        cmd.arg("-shortest").arg(video_output);

        let audio_output = outputs
            .audio_only
            .as_ref()
            .ok_or_else(|| anyhow!("Expected an audio-only output path"))?;
        cmd.args(["-map", "[aout]"])
            .args(["-c:a", "flac"])
            .args(["-ar", &options.audio_sample_rate.to_string()])
            .arg(audio_output);
    } else {
        let combined_output = outputs
            .combined
            .as_ref()
            .ok_or_else(|| anyhow!("Expected a combined output path"))?;

        if any_video {
            cmd.args(["-map", "[vout]"]);
            apply_video_codec(&mut cmd, options, use_hardware);
        }
        if audio_label.is_some() {
            cmd.args(["-map", "[aout]"]);
            if any_video {
                cmd.args(["-c:a", "aac"])
                    .args(["-b:a", &format!("{}k", options.audio_bitrate_kbps)]);
            } else {
                cmd.args(["-c:a", "flac"]);
            }
            cmd.args(["-ar", &options.audio_sample_rate.to_string()]);
        }
        if any_video && audio_label.is_some() {
            cmd.arg("-shortest");
        }
        cmd.arg(combined_output);
    }

    let child = cmd.spawn().context("Failed to spawn ffmpeg process")?;
    Ok((child, outputs))
}

fn apply_video_codec(cmd: &mut Command, options: &RecorderOptions, use_hardware: bool) {
    let bitrate = format!("{}k", options.video_bitrate_kbps);
    if use_hardware {
        cmd.args(["-c:v", "h264_vaapi"])
            .args(["-b:v", &bitrate])
            .args(["-maxrate", &bitrate])
            .args(["-g", &(options.frame_rate * 2).to_string()]);
    } else {
        cmd.args(["-c:v", "libx264"])
            .args(["-preset", preset_for_quality(options.video_quality)])
            .args(["-crf", &crf_for_quality(options.video_quality).to_string()])
            .args(["-maxrate", &bitrate])
            .args(["-bufsize", &format!("{}k", options.video_bitrate_kbps * 2)])
            .args(["-g", &(options.frame_rate * 2).to_string()]);
    }
    cmd.args(["-r", &options.frame_rate.to_string()]);
}

fn resolve_webcam(options: &RecorderOptions) -> Option<String> {
    let requested = options
        .webcam_device
        .clone()
        .unwrap_or_else(|| "/dev/video0".to_string());

    let candidate = if requested.starts_with("/dev/video") && std::path::Path::new(&requested).exists()
    {
        Some(requested)
    } else {
        (0..10)
            .map(|i| format!("/dev/video{}", i))
            .find(|p| std::path::Path::new(p).exists())
    };

    candidate.filter(|path| ffmpeg_v4l2_accessible(&options.ffmpeg_path, path))
}

struct ScreenCaptureInput {
    display_input: String,
    video_size: String,
}

fn determine_screen_input(screen_index: Option<usize>) -> Result<ScreenCaptureInput> {
    let display = env::var("DISPLAY").unwrap_or_else(|_| ":0.0".to_string());
    let screens = catch_unwind(AssertUnwindSafe(Screen::all))
        .map_err(|_| anyhow!("Screen enumeration backend crashed (is DISPLAY reachable?)"))?
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

    // display-info reports logical coordinates (divided by the Xft scale factor),
    // while x11grab wants device pixels — so scale them back up.
    let (x, y, width, height) = physical_geometry(&screen.display_info);

    Ok(ScreenCaptureInput {
        display_input: format!("{}+{},{}", display, x, y),
        video_size: format!("{}x{}", width, height),
    })
}

/// Device-pixel geometry (x, y, width, height) of a display.
pub fn physical_geometry(info: &screenshots::display_info::DisplayInfo) -> (i32, i32, u32, u32) {
    let scale = if info.scale_factor > 0.0 {
        info.scale_factor
    } else {
        1.0
    };
    (
        (info.x as f32 * scale).round() as i32,
        (info.y as f32 * scale).round() as i32,
        (info.width as f32 * scale).round() as u32,
        (info.height as f32 * scale).round() as u32,
    )
}

fn prepare_output_paths(options: &RecorderOptions, any_video: bool) -> Result<RecordingOutputs> {
    let timestamp = Local::now().format("%Y%m%d_%H%M%S");
    let base_name = format!("recording_{}", timestamp);
    let split = options.separate_outputs && options.include_audio && any_video;

    let combined = if split {
        None
    } else {
        let ext = if any_video { "mkv" } else { "flac" };
        Some(options.output_directory.join(format!("{}.{}", base_name, ext)))
    };

    let video_only = split.then(|| {
        options
            .output_directory
            .join(format!("{}.video.mkv", base_name))
    });

    let audio_only = split.then(|| {
        options
            .output_directory
            .join(format!("{}.audio.flac", base_name))
    });

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

fn preset_for_quality(quality: VideoQuality) -> &'static str {
    match quality {
        VideoQuality::Low | VideoQuality::Medium => "veryfast",
        VideoQuality::High => "fast",
        VideoQuality::Ultra => "medium",
    }
}
