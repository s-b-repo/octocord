//! Headless end-to-end recording, using the same code path as the GUI.
//!
//! ```text
//! cargo run --example record_probe -- <seconds> <resolution> <quality> <audio> <encoder>
//! ```
//!
//! resolution: native | 720 | 1080 | 1440 | 2160 | WxH
//! quality:    low | medium | high | ultra
//! audio:      none | system | mic | both
//! encoder:    auto | hw | sw
//!
//! Set OCTOCORD_TOKEN to a portal restore token to skip the picker dialog.

use anyhow::{bail, Result};
use discord_recorder::config::{
    AudioSource, Config, EncoderPreference, OutputResolution, VideoQuality,
};
use discord_recorder::pipewire_capture::ScreenSource;
use discord_recorder::portal;
use discord_recorder::screen;
use discord_recorder::video::{RecorderOptions, VideoEncoder};
use std::sync::Arc;
use std::time::Duration;

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let mut args = std::env::args().skip(1);

    let seconds: u64 = args.next().unwrap_or_else(|| "5".into()).parse()?;
    let resolution = parse_resolution(&args.next().unwrap_or_else(|| "native".into()))?;
    let quality = parse_quality(&args.next().unwrap_or_else(|| "high".into()))?;
    let audio = parse_audio(&args.next().unwrap_or_else(|| "none".into()))?;
    let encoder = parse_encoder(&args.next().unwrap_or_else(|| "auto".into()))?;
    let audio_quality = parse_audio_quality(&args.next().unwrap_or_else(|| "high".into()))?;

    let mut config = Config::default();
    config.audio_quality = audio_quality;
    config.video_quality = quality;
    config.output_resolution = resolution;
    config.encoder = encoder;
    config.audio_source = audio.unwrap_or(AudioSource::System);

    let output_directory = std::env::temp_dir().join("octocord-probe");
    let mut options = RecorderOptions::new(output_directory);
    options.video_quality = quality;
    options.video_bitrate_kbps = config.get_video_bitrate();
    options.audio_bitrate_kbps = config.get_audio_bitrate();
    options.audio_sample_rate = config.get_audio_sample_rate();
    options.frame_rate = config.get_frame_rate();
    options.output_resolution = resolution;
    options.encoder = encoder;
    options.include_audio = audio.is_some();
    options.audio_source = config.audio_source;
    options.include_webcam = std::env::var("OCTOCORD_WEBCAM").is_ok();

    let mut encoder = VideoEncoder::new(options)?;

    // Hold the session for the whole recording: dropping it closes the PipeWire node.
    let _source = if screen::is_wayland_session() && portal::is_available() {
        let source = ScreenSource::start(std::env::var("OCTOCORD_TOKEN").ok(), true)?;
        println!(
            "capturing {}x{} from the compositor",
            source.format.width, source.format.height
        );
        if let Some(token) = source.restore_token() {
            println!("restore token: {}", token);
        }
        encoder.set_screen_source(Arc::clone(&source.capture));
        Some(source)
    } else {
        println!("capturing with x11grab");
        None
    };

    encoder.start()?;
    println!(
        "recording {} s ({} encoding)",
        seconds,
        if encoder.hardware_encoding() { "hardware" } else { "software" }
    );
    std::thread::sleep(Duration::from_secs(seconds));

    let output = encoder
        .outputs()
        .and_then(|o| o.combined.clone().or_else(|| o.video_only.clone()))
        .ok_or_else(|| anyhow::anyhow!("no output path was produced"))?;
    encoder.stop()?;

    let size = std::fs::metadata(&output)?.len();
    println!("wrote {} ({} bytes)", output.display(), size);
    if size == 0 {
        bail!("the recording is empty");
    }
    Ok(())
}

fn parse_resolution(value: &str) -> Result<OutputResolution> {
    Ok(match value {
        "native" => OutputResolution::Native,
        "720" => OutputResolution::P720,
        "1080" => OutputResolution::P1080,
        "1440" => OutputResolution::P1440,
        "2160" => OutputResolution::P2160,
        custom => {
            let (width, height) = custom
                .split_once('x')
                .ok_or_else(|| anyhow::anyhow!("expected WxH, got {}", custom))?;
            OutputResolution::Custom {
                width: width.parse()?,
                height: height.parse()?,
            }
        }
    })
}

fn parse_quality(value: &str) -> Result<VideoQuality> {
    Ok(match value {
        "low" => VideoQuality::Low,
        "medium" => VideoQuality::Medium,
        "high" => VideoQuality::High,
        "ultra" => VideoQuality::Ultra,
        other => bail!("unknown quality {}", other),
    })
}

fn parse_audio_quality(value: &str) -> Result<discord_recorder::config::AudioQuality> {
    use discord_recorder::config::AudioQuality;
    Ok(match value {
        "low" => AudioQuality::Low,
        "medium" => AudioQuality::Medium,
        "high" => AudioQuality::High,
        "lossless" => AudioQuality::Lossless,
        other => bail!("unknown audio quality {}", other),
    })
}

fn parse_audio(value: &str) -> Result<Option<AudioSource>> {
    Ok(match value {
        "none" => None,
        "system" => Some(AudioSource::System),
        "mic" => Some(AudioSource::Microphone),
        "both" => Some(AudioSource::Both),
        other => bail!("unknown audio mode {}", other),
    })
}

fn parse_encoder(value: &str) -> Result<EncoderPreference> {
    Ok(match value {
        "auto" => EncoderPreference::Auto,
        "hw" => EncoderPreference::Hardware,
        "sw" => EncoderPreference::Software,
        other => bail!("unknown encoder {}", other),
    })
}
