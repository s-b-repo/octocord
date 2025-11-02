use anyhow::Result;
use egui::{CentralPanel, SidePanel, TopBottomPanel, Ui, RichText, Color32, Stroke, Rounding};
use std::sync::{Arc, Mutex};
use log::{info, error};

use crate::{
    audio::AudioRecorder,
    video::VideoEncoder,
    screen::ScreenCapture,
    webcam::WebcamCapture,
    config::Config,
};

pub struct AppState {
    pub is_recording: bool,
    pub record_audio: bool,
    pub record_video: bool,
    pub record_webcam: bool,
    pub selected_screen: Option<usize>,
    pub selected_audio_device: Option<String>,
    pub selected_webcam: Option<String>,
    pub output_path: String,
    pub video_quality: VideoQuality,
    pub audio_quality: AudioQuality,
    pub config: Config,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum VideoQuality {
    Low,
    Medium,
    High,
    Ultra,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AudioQuality {
    Low,
    Medium,
    High,
    Lossless,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            is_recording: false,
            record_audio: true,
            record_video: true,
            record_webcam: false,
            selected_screen: None,
            selected_audio_device: None,
            selected_webcam: None,
            output_path: dirs::home_dir()
                .unwrap_or_default()
                .join("Videos")
                .join("discord-recordings")
                .to_string_lossy()
                .to_string(),
            video_quality: VideoQuality::High,
            audio_quality: AudioQuality::High,
            config: Config::load().unwrap_or_default(),
        }
    }
}

pub struct DiscordRecorderApp {
    state: Arc<Mutex<AppState>>,
    audio_recorder: Option<AudioRecorder>,
    video_encoder: Option<VideoEncoder>,
    screen_capture: Option<ScreenCapture>,
    webcam_capture: Option<WebcamCapture>,
    available_screens: Vec<String>,
    available_audio_devices: Vec<String>,
    available_webcams: Vec<String>,
}

impl DiscordRecorderApp {
    pub fn new(_cc: &eframe::CreationContext<'_>, state: Arc<Mutex<AppState>>) -> Self {
        let mut app = Self {
            state,
            audio_recorder: None,
            video_encoder: None,
            screen_capture: None,
            webcam_capture: None,
            available_screens: Vec::new(),
            available_audio_devices: Vec::new(),
            available_webcams: Vec::new(),
        };

        // Initialize available devices
        if let Err(e) = app.refresh_devices() {
            error!("Failed to refresh devices: {}", e);
        }

        app
    }

    fn refresh_devices(&mut self) -> Result<()> {
        // Get available screens
        self.available_screens = crate::screen::get_available_screens()?;
        
        // Get available audio devices
        self.available_audio_devices = crate::audio::get_available_devices()?;
        
        // Get available webcams
        self.available_webcams = crate::webcam::get_available_webcams()?;
        
        Ok(())
    }

    fn start_recording(&mut self) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        
        if state.is_recording {
            return Ok(());
        }

        info!("Starting recording");
        state.is_recording = true;

        // Initialize components based on settings
        if state.record_audio {
            if let Some(device) = &state.selected_audio_device {
                self.audio_recorder = Some(AudioRecorder::new(device)?);
                self.audio_recorder.as_mut().unwrap().start()?;
            }
        }

        if state.record_video {
            let screen_idx = state.selected_screen.unwrap_or(0);
            self.screen_capture = Some(ScreenCapture::new(screen_idx)?);
            
            let output_file = format!("{}/recording_{}.mp4", 
                state.output_path, 
                chrono::Local::now().format("%Y%m%d_%H%M%S")
            );
            
            self.video_encoder = Some(VideoEncoder::new(&output_file, state.video_quality)?);
            self.screen_capture.as_mut().unwrap().start()?;
            self.video_encoder.as_mut().unwrap().start()?;
        }

        if state.record_webcam {
            if let Some(webcam) = &state.selected_webcam {
                self.webcam_capture = Some(WebcamCapture::new(webcam)?);
                self.webcam_capture.as_mut().unwrap().start()?;
            }
        }

        Ok(())
    }

    fn stop_recording(&mut self) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        
        if !state.is_recording {
            return Ok(());
        }

        info!("Stopping recording");
        state.is_recording = false;

        // Stop all recording components
        if let Some(recorder) = &mut self.audio_recorder {
            recorder.stop()?;
        }
        
        if let Some(capture) = &mut self.screen_capture {
            capture.stop()?;
        }
        
        if let Some(encoder) = &mut self.video_encoder {
            encoder.stop()?;
        }
        
        if let Some(webcam) = &mut self.webcam_capture {
            webcam.stop()?;
        }

        // Clean up
        self.audio_recorder = None;
        self.screen_capture = None;
        self.video_encoder = None;
        self.webcam_capture = None;

        Ok(())
    }
}

impl eframe::App for DiscordRecorderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Apply Discord-like styling
        ctx.set_style(discord_style());

        // Top panel with recording controls
        TopBottomPanel::top("controls_panel").show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                let state = self.state.lock().unwrap();
                
                // Recording button
                let button_text = if state.is_recording {
                    "⏹ Stop Recording"
                } else {
                    "⏺ Start Recording"
                };
                
                let button_color = if state.is_recording {
                    Color32::from_rgb(240, 71, 71) // Discord red
                } else {
                    Color32::from_rgb(35, 165, 90) // Discord green
                };

                let button = ui.add_sized(
                    [120.0, 40.0],
                    egui::Button::new(RichText::new(button_text).size(16.0))
                        .fill(button_color)
                        .rounding(Rounding::same(8.0))
                );

                if button.clicked() {
                    drop(state);
                    if let Err(e) = if self.state.lock().unwrap().is_recording {
                        self.stop_recording()
                    } else {
                        self.start_recording()
                    } {
                        error!("Recording operation failed: {}", e);
                    }
                }

                // Recording status indicator
                ui.horizontal(|ui| {
                    let (status_text, status_color) = if state.is_recording {
                        ("● REC", Color32::from_rgb(240, 71, 71))
                    } else {
                        ("● IDLE", Color32::from_rgb(116, 127, 141))
                    };
                    
                    ui.colored_label(status_color, RichText::new(status_text).size(14.0));
                });
            });
        });

        // Side panel with settings
        SidePanel::left("settings_panel")
            .resizable(true)
            .default_width(300.0)
            .show(ctx, |ui| {
                let mut state = self.state.lock().unwrap();
                
                ui.heading("Recording Settings");
                ui.separator();

                // Output path
                ui.horizontal(|ui| {
                    ui.label("Output Path:");
                    if ui.text_edit_singleline(&mut state.output_path).changed() {
                        // Save config when path changes
                        if let Err(e) = state.config.save() {
                            error!("Failed to save config: {}", e);
                        }
                    }
                });

                ui.separator();

                // Recording options
                ui.checkbox(&mut state.record_video, "Record Screen");
                ui.checkbox(&mut state.record_audio, "Record Audio");
                ui.checkbox(&mut state.record_webcam, "Record Webcam");

                ui.separator();

                // Screen selection
                if state.record_video && !self.available_screens.is_empty() {
                    ui.label("Screen:");
                    egui::ComboBox::from_label("")
                        .selected_text(
                            state.selected_screen
                                .and_then(|idx| self.available_screens.get(idx))
                                .unwrap_or(&"Select Screen".to_string())
                        )
                        .show_ui(ui, |ui| {
                            for (idx, screen) in self.available_screens.iter().enumerate() {
                                ui.selectable_value(
                                    &mut state.selected_screen,
                                    Some(idx),
                                    screen
                                );
                            }
                        });
                }

                // Audio device selection
                if state.record_audio && !self.available_audio_devices.is_empty() {
                    ui.label("Audio Device:");
                    egui::ComboBox::from_label("")
                        .selected_text(
                            state.selected_audio_device
                                .as_ref()
                                .unwrap_or(&"Select Device".to_string())
                        )
                        .show_ui(ui, |ui| {
                            for device in &self.available_audio_devices {
                                ui.selectable_value(
                                    &mut state.selected_audio_device,
                                    Some(device.clone()),
                                    device
                                );
                            }
                        });
                }

                // Webcam selection
                if state.record_webcam && !self.available_webcams.is_empty() {
                    ui.label("Webcam:");
                    egui::ComboBox::from_label("")
                        .selected_text(
                            state.selected_webcam
                                .as_ref()
                                .unwrap_or(&"Select Webcam".to_string())
                        )
                        .show_ui(ui, |ui| {
                            for webcam in &self.available_webcams {
                                ui.selectable_value(
                                    &mut state.selected_webcam,
                                    Some(webcam.clone()),
                                    webcam
                                );
                            }
                        });
                }

                ui.separator();

                // Quality settings
                ui.label("Video Quality:");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut state.video_quality, VideoQuality::Low, "Low");
                    ui.radio_value(&mut state.video_quality, VideoQuality::Medium, "Medium");
                    ui.radio_value(&mut state.video_quality, VideoQuality::High, "High");
                    ui.radio_value(&mut state.video_quality, VideoQuality::Ultra, "Ultra");
                });

                ui.label("Audio Quality:");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut state.audio_quality, AudioQuality::Low, "Low");
                    ui.radio_value(&mut state.audio_quality, AudioQuality::Medium, "Medium");
                    ui.radio_value(&mut state.audio_quality, AudioQuality::High, "High");
                    ui.radio_value(&mut state.audio_quality, AudioQuality::Lossless, "Lossless");
                });

                ui.separator();

                // Refresh devices button
                if ui.button("Refresh Devices").clicked() {
                    drop(state);
                    if let Err(e) = self.refresh_devices() {
                        error!("Failed to refresh devices: {}", e);
                    }
                }
            });

        // Central panel with preview
        CentralPanel::default().show(ctx, |ui| {
            let state = self.state.lock().unwrap();
            
            ui.heading("Recording Preview");
            
            // Show preview area
            let available_size = ui.available_size();
            let preview_rect = egui::Rect::from_min_size(
                ui.cursor().min,
                egui::Vec2::new(available_size.x, available_size.y - 50.0)
            );
            
            // Draw preview background
            ui.painter().rect(
                preview_rect,
                Rounding::same(8.0),
                Color32::from_rgb(30, 31, 34), // Discord dark background
                Stroke::new(1.0, Color32::from_rgb(64, 68, 75))
            );
            
            // Show webcam preview if enabled
            if state.record_webcam && self.webcam_capture.is_some() {
                // Draw webcam overlay in corner
                let webcam_rect = egui::Rect::from_min_size(
                    preview_rect.right_top() - egui::Vec2::new(320.0, 180.0),
                    egui::Vec2::new(320.0, 180.0)
                );
                
                ui.painter().rect(
                    webcam_rect,
                    Rounding::same(8.0),
                    Color32::from_rgb(24, 25, 28),
                    Stroke::new(2.0, Color32::from_rgb(88, 101, 242))
                );
                
                ui.painter().text(
                    webcam_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Webcam Preview",
                    egui::FontId::monospace(14.0),
                    Color32::from_rgb(185, 187, 190)
                );
            }
            
            // Show recording info
            ui.allocate_ui_at_rect(preview_rect, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(20.0);
                    ui.label(RichText::new("Screen Preview Area").size(16.0));
                    
                    if state.is_recording {
                        ui.label(RichText::new("🔴 Recording in progress...").color(Color32::RED));
                    } else {
                        ui.label(RichText::new("⏸ Ready to record").color(Color32::GREEN));
                    }
                });
            });
        });
    }
}

fn discord_style() -> egui::Style {
    let mut style = egui::Style::default();
    
    // Discord color palette
    style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(88, 101, 242); // Discord blurple
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(71, 82, 196); // Darker blurple
    style.visuals.widgets.active.bg_fill = Color32::from_rgb(58, 67, 159); // Even darker
    
    // Background colors
    style.visuals.panel_fill = Color32::from_rgb(54, 57, 63); // Discord background
    style.visuals.extreme_bg_color = Color32::from_rgb(47, 49, 54); // Discord sidebar
    style.visuals.code_bg_color = Color32::from_rgb(40, 42, 46); // Discord channel area
    
    // Text colors
    style.visuals.widgets.inactive.fg_stroke.color = Color32::from_rgb(255, 255, 255);
    style.visuals.text_color = Color32::from_rgb(185, 187, 190); // Discord text
    
    // Window styling
    style.visuals.window_fill = Color32::from_rgb(54, 57, 63);
    style.visuals.window_stroke = Stroke::new(1.0, Color32::from_rgb(32, 34, 37));
    
    // Spacing
    style.spacing.item_spacing = egui::Vec2::new(8.0, 8.0);
    style.spacing.window_margin = egui::Margin::same(8.0);
    
    style