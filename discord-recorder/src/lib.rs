pub mod audio;
pub mod config;
pub mod gui;
pub mod screen;
pub mod video;
pub mod webcam;

// Re-export main types
pub use gui::DiscordRecorderApp;
pub use config::Config;