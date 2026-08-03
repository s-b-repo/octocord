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
    /// Kept for compatibility with older configuration files. Wayland capture now
    /// always goes through the portal when one is present, because x11grab cannot
    /// see a Wayland desktop at all.
    #[serde(default = "default_true")]
    pub use_pipewire_on_wayland: bool,
    #[serde(default)]
    pub enable_preview_overlay: bool,
    /// Output geometry. Independent of the quality tier so any monitor size,
    /// preset or hand-typed resolution can be recorded.
    #[serde(default)]
    pub output_resolution: OutputResolution,
    #[serde(default)]
    pub encoder: EncoderPreference,
    #[serde(default)]
    pub audio_source: AudioSource,
    #[serde(default = "default_true")]
    pub capture_cursor: bool,
    /// Portal token that lets a later session skip the screen picker dialog.
    #[serde(default)]
    pub screencast_restore_token: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Resolution of the encoded file. `Native` keeps whatever the capture produced.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum OutputResolution {
    #[default]
    Native,
    P720,
    P1080,
    P1440,
    P2160,
    Custom {
        width: u32,
        height: u32,
    },
}

impl OutputResolution {
    pub const PRESETS: [OutputResolution; 5] = [
        OutputResolution::Native,
        OutputResolution::P720,
        OutputResolution::P1080,
        OutputResolution::P1440,
        OutputResolution::P2160,
    ];

    /// Target box in pixels, or `None` to keep the source size.
    pub fn target(self) -> Option<(u32, u32)> {
        match self {
            OutputResolution::Native => None,
            OutputResolution::P720 => Some((1280, 720)),
            OutputResolution::P1080 => Some((1920, 1080)),
            OutputResolution::P1440 => Some((2560, 1440)),
            OutputResolution::P2160 => Some((3840, 2160)),
            OutputResolution::Custom { width, height } => Some((width.max(2), height.max(2))),
        }
    }

    pub fn label(self) -> String {
        match self {
            OutputResolution::Native => "Native (source resolution)".to_string(),
            OutputResolution::P720 => "1280 x 720".to_string(),
            OutputResolution::P1080 => "1920 x 1080".to_string(),
            OutputResolution::P1440 => "2560 x 1440".to_string(),
            OutputResolution::P2160 => "3840 x 2160".to_string(),
            OutputResolution::Custom { width, height } => format!("Custom ({} x {})", width, height),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum EncoderPreference {
    /// Hardware when the probe succeeds, software otherwise.
    #[default]
    Auto,
    Hardware,
    Software,
}

impl EncoderPreference {
    pub const ALL: [EncoderPreference; 3] = [
        EncoderPreference::Auto,
        EncoderPreference::Hardware,
        EncoderPreference::Software,
    ];

    pub fn label(self) -> &'static str {
        match self {
            EncoderPreference::Auto => "Auto (hardware when available)",
            EncoderPreference::Hardware => "Hardware (VAAPI)",
            EncoderPreference::Software => "Software (libx264)",
        }
    }
}

/// Which audio the recorder captures. "System" is the monitor of the default output,
/// which is what desktop audio actually lives on — the default *source* is the mic.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum AudioSource {
    #[default]
    System,
    Microphone,
    Both,
}

impl AudioSource {
    pub const ALL: [AudioSource; 3] = [
        AudioSource::System,
        AudioSource::Microphone,
        AudioSource::Both,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AudioSource::System => "System audio (speaker monitor)",
            AudioSource::Microphone => "Microphone",
            AudioSource::Both => "System audio + microphone",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum VideoQuality {
    Low,
    Medium,
    High,
    Ultra,
}

impl VideoQuality {
    pub const ALL: [VideoQuality; 4] = [
        VideoQuality::Low,
        VideoQuality::Medium,
        VideoQuality::High,
        VideoQuality::Ultra,
    ];
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AudioQuality {
    Low,
    Medium,
    High,
    Lossless,
}

impl AudioQuality {
    pub const ALL: [AudioQuality; 4] = [
        AudioQuality::Low,
        AudioQuality::Medium,
        AudioQuality::High,
        AudioQuality::Lossless,
    ];
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DiscordTheme {
    Dark,
    Light,
    AMOLED,
}

impl DiscordTheme {
    pub const ALL: [DiscordTheme; 3] = [DiscordTheme::Dark, DiscordTheme::Light, DiscordTheme::AMOLED];

    pub fn label(self) -> &'static str {
        match self {
            DiscordTheme::Dark => "Dark",
            DiscordTheme::Light => "Light",
            DiscordTheme::AMOLED => "AMOLED",
        }
    }
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
            use_pipewire_on_wayland: true,
            enable_preview_overlay: false,
            output_resolution: OutputResolution::Native,
            encoder: EncoderPreference::Auto,
            audio_source: AudioSource::System,
            capture_cursor: true,
            screencast_restore_token: None,
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

    /// Capture frame rate implied by the selected video quality.
    pub fn get_frame_rate(&self) -> u32 {
        match self.video_quality {
            VideoQuality::Low | VideoQuality::Medium => 30,
            VideoQuality::High | VideoQuality::Ultra => 60,
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