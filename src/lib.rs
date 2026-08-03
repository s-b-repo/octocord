pub mod audio;
pub mod config;
pub mod gui;
pub mod pipewire_capture;
pub mod portal;
pub mod screen;
pub mod video;
pub mod webcam;

// Re-export main types
pub use config::Config;
pub use gui::{AppState, DiscordRecorderApp};

#[cfg(test)]
mod test_fixes;
