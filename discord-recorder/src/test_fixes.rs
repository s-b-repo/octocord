use anyhow::Result;

// Test basic compilation and imports
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_imports() -> Result<()> {
        // Test that all modules can be imported
        use crate::audio::AudioRecorder;
        use crate::video::VideoEncoder;
        use crate::screen::ScreenCapture;
        use crate::webcam::WebcamCapture;
        use crate::config::Config;
        
        Ok(())
    }
    
    #[test]
    fn test_config_creation() -> Result<()> {
        let config = Config::default();
        assert!(!config.output_directory.is_empty());
        Ok(())
    }
    
    #[test]
    fn test_video_quality_settings() -> Result<()> {
        use crate::config::VideoQuality;
        
        let low_res = crate::video::get_video_encoder_settings(VideoQuality::Low);
        assert_eq!(low_res, (1280, 720));
        
        Ok(())
    }
}