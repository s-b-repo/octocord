use anyhow::Result;
use log::{error, info};
use std::sync::{Arc, Mutex};

mod audio;
mod video;
mod screen;
mod webcam;
mod gui;
mod config;
mod runtime;

use gui::DiscordRecorderApp;
use eframe::egui;

fn main() -> Result<()> {
    // Initialize logging (default to info if RUST_LOG is not set)
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    info!("Starting Discord Recorder");

    // Create application state
    let app_state = Arc::new(Mutex::new(gui::AppState::new()));

    // Configure eframe
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([800.0, 600.0])
            .with_title("Discord Recorder"),
        ..Default::default()
    };

    // Run the application
    if let Err(err) = eframe::run_native(
        "Discord Recorder",
        native_options,
        Box::new(|cc| Ok(Box::new(DiscordRecorderApp::new(cc, app_state)))),
    ) {
        error!("Failed to launch application: {}", err);
    }

    Ok(())
}