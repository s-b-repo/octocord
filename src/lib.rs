pub mod audio;
pub mod config;
pub mod gui;
pub mod screen;
pub mod video;
pub mod webcam;
pub mod runtime;

// Re-export main types
pub use gui::DiscordRecorderApp;
pub use config::Config;

// Global Tokio runtime for async tasks
use once_cell::sync::Lazy;
use tokio::runtime::{Builder, Handle, Runtime};

static RUNTIME: Lazy<Runtime> = Lazy::new(|| {
    Builder::new_multi_thread()
        .enable_all()
        .thread_name("octocord-rt")
        .build()
        .expect("Failed to build Tokio runtime")
});

pub fn runtime_handle() -> Handle {
    RUNTIME.handle().clone()
}