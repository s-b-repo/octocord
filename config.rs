use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub output_directory: String,
    pub video_quality: VideoQuality,
    pub audio_quality: AudioQuality,
    pub default_screen: Option<usize>,
    pub default_audio_device: Option<String>,
    pub default_webcam: Option<String>,
    pub record_audio: bool,
    pub record_video: bool,
    pub record_webcam: bool,
    pub discord_theme: DiscordTheme,
    #[serde(default)]
    pub separate_outputs: bool,
    #[serde(default)]
    pub use_pipewire_on_wayland: bool,
    #[serde(default)]
    pub enable_preview_overlay: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum VideoQuality {
    Low,
    Medium,
    High,
    Ultra,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AudioQuality {
    Low,
    Medium,
    High,
    Lossless,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum DiscordTheme {
    Dark,
    Light,
    AMOLED,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            output_directory: dirs::home_dir()
                .unwrap_or_default()
                .join("Videos")
                .join("discord-recordings")
                .to_string_lossy()
                .to_string(),
            video_quality: VideoQuality::High,
            audio_quality: AudioQuality::High,
            default_screen: None,
            default_audio_device: None,
            default_webcam: None,
            record_audio: true,
            record_video: true,
            record_webcam: false,
            discord_theme: DiscordTheme::Dark,
            separate_outputs: false,
            use_pipewire_on_wayland: false,
            enable_preview_overlay: false,
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let config_path = Self::get_config_path()?;
        
        if config_path.exists() {
            let content = std::fs::read_to_string(config_path)?;
            let config: Config = serde_json::from_str(&content)?;
            Ok(config)
        } else {
            let config = Config::default();
            config.save()?;
            Ok(config)
        }
    }

    pub fn save(&self) -> Result<()> {
        let config_path = Self::get_config_path()?;
        
        // Create directory if it doesn't exist
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(config_path, content)?;
        
        Ok(())
    }

    fn get_config_path() -> Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .unwrap_or_default()
            .join("discord-recorder");
        
        Ok(config_dir.join("config.json"))
    }

    pub fn get_output_directory(&self) -> &str {
        &self.output_directory
    }

    pub fn set_output_directory(&mut self, path: String) {
        self.output_directory = path;
    }

    pub fn get_video_bitrate(&self) -> u32 {
        match self.video_quality {
            VideoQuality::Low => 1000,
            VideoQuality::Medium => 2500,
            VideoQuality::High => 5000,
            VideoQuality::Ultra => 10000,
        }
    }

    pub fn get_audio_sample_rate(&self) -> u32 {
        match self.audio_quality {
            AudioQuality::Low => 22050,
            AudioQuality::Medium => 44100,
            AudioQuality::High => 48000,
            AudioQuality::Lossless => 96000,
        }
    }

    pub fn get_audio_bitrate(&self) -> u32 {
        match self.audio_quality {
            AudioQuality::Low => 64,
            AudioQuality::Medium => 128,
            AudioQuality::High => 256,
            AudioQuality::Lossless => 320,
        }
    }
}