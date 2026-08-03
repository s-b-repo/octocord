use anyhow::Result;

use crate::config::{
    AudioQuality, AudioSource, Config, EncoderPreference, OutputResolution, VideoQuality,
};
use crate::video::{RecorderOptions, VideoEncoder};
use std::env;

fn test_options() -> RecorderOptions {
    RecorderOptions::new(env::temp_dir().join("discord_recorder_tests"))
}

#[test]
fn config_default_creates_output_dir() -> Result<()> {
    let config = Config::default();
    assert!(!config.output_directory.is_empty());
    Ok(())
}

#[test]
fn video_encoder_new_validates_inputs() -> Result<()> {
    let encoder = VideoEncoder::new(test_options());
    assert!(encoder.is_ok());
    Ok(())
}

#[test]
fn video_encoder_rejects_empty_capture_set() {
    let mut options = test_options();
    options.include_audio = false;
    options.include_video = false;
    options.include_webcam = false;

    assert!(VideoEncoder::new(options).is_err());
}

#[test]
fn every_video_quality_maps_to_a_bitrate_and_frame_rate() {
    let mut config = Config::default();
    for quality in VideoQuality::ALL {
        config.video_quality = quality;
        assert!(config.get_video_bitrate() > 0, "{:?} has no bitrate", quality);
        assert!(
            matches!(config.get_frame_rate(), 30 | 60),
            "{:?} has an unexpected frame rate",
            quality
        );
    }
}

#[test]
fn every_audio_quality_maps_to_a_sample_rate_and_bitrate() {
    let mut config = Config::default();
    for quality in AudioQuality::ALL {
        config.audio_quality = quality;
        assert!(
            config.get_audio_sample_rate() >= 22_050,
            "{:?} has an implausible sample rate",
            quality
        );
        assert!(config.get_audio_bitrate() > 0, "{:?} has no bitrate", quality);
    }
}

#[test]
fn resolution_presets_have_even_targets() {
    for preset in OutputResolution::PRESETS {
        if let Some((width, height)) = preset.target() {
            assert_eq!(width % 2, 0, "{:?} width is odd", preset);
            assert_eq!(height % 2, 0, "{:?} height is odd", preset);
        }
    }
    assert!(OutputResolution::Native.target().is_none());
}

#[test]
fn custom_resolution_is_clamped_to_a_usable_size() {
    let tiny = OutputResolution::Custom {
        width: 0,
        height: 0,
    };
    let (width, height) = tiny.target().expect("custom resolutions have a target");
    assert!(width >= 2 && height >= 2);
}

#[test]
fn config_round_trips_through_json() -> Result<()> {
    let mut config = Config::default();
    config.output_resolution = OutputResolution::Custom {
        width: 1234,
        height: 718,
    };
    config.encoder = EncoderPreference::Hardware;
    config.audio_source = AudioSource::Both;
    config.screencast_restore_token = Some("token".to_string());

    let json = serde_json::to_string(&config)?;
    let parsed: Config = serde_json::from_str(&json)?;

    assert_eq!(parsed.output_resolution, config.output_resolution);
    assert_eq!(parsed.encoder, config.encoder);
    assert_eq!(parsed.audio_source, config.audio_source);
    assert_eq!(parsed.screencast_restore_token, config.screencast_restore_token);
    Ok(())
}

/// Configuration files written before these fields existed must keep loading.
#[test]
fn config_accepts_files_without_the_new_fields() -> Result<()> {
    let legacy = r#"{
        "output_directory": "/tmp/recordings",
        "video_quality": "High",
        "audio_quality": "Medium",
        "default_screen": null,
        "default_audio_device": null,
        "default_webcam": null,
        "record_audio": true,
        "record_video": true,
        "record_webcam": false,
        "discord_theme": "Dark"
    }"#;

    let config: Config = serde_json::from_str(legacy)?;
    assert_eq!(config.output_resolution, OutputResolution::Native);
    assert_eq!(config.encoder, EncoderPreference::Auto);
    assert_eq!(config.audio_source, AudioSource::System);
    assert!(config.capture_cursor);
    Ok(())
}
