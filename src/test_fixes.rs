use anyhow::Result;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, VideoQuality};
    use crate::video::{RecorderOptions, VideoEncoder};
    use std::env;
    use std::path::PathBuf;

    #[test]
    fn config_default_creates_output_dir() -> Result<()> {
        let config = Config::default();
        assert!(!config.output_directory.is_empty());
        Ok(())
    }

    #[test]
    fn video_encoder_new_validates_inputs() -> Result<()> {
        let temp_dir = env::temp_dir().join("discord_recorder_tests");
        let options = RecorderOptions {
            output_directory: temp_dir,
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
            audio_device: None,
            webcam_device: None,
            ffmpeg_path: "ffmpeg".to_string(),
            audio_gain_db: 0.0,
        };

        let encoder = VideoEncoder::new(options);
        assert!(encoder.is_ok());
        Ok(())
    }
}