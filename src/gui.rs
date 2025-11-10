use anyhow::Result;
use egui::{
    CentralPanel, TopBottomPanel, RichText, Color32, ColorImage, TextureHandle, TextureOptions,
    Stroke, ProgressBar, DragValue, Slider, KeyboardShortcut, Modifiers, Key
};
use egui::vec2;
use image::DynamicImage;
use log::{info, error};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::{
    audio::{self, AudioProcessor, AudioRecorder},
    config::{Config, VideoQuality, AudioQuality},
    screen::{self, ScreenCapture},
    video::{RecorderOptions, VideoEncoder},
    webcam::{self, WebcamCapture},
};
#[cfg(feature = "webcam")]
use crate::webcam::WebcamOverlay;

// Remove these duplicate re-exports
// pub use crate::config::VideoQuality;
// pub use crate::config::AudioQuality;

#[derive(Clone)]
pub struct HotkeyConfig {
    pub start_stop: KeyboardShortcut,
    pub pause_resume: KeyboardShortcut,
    pub toggle_webcam: KeyboardShortcut,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        let mut ctrl = Modifiers::default();
        ctrl.ctrl = true;
        Self {
            start_stop: KeyboardShortcut::new(ctrl, Key::R),
            pause_resume: KeyboardShortcut::new(ctrl, Key::P),
            toggle_webcam: KeyboardShortcut::new(ctrl, Key::W),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HotkeyAction {
    StartStop,
    PauseResume,
    ToggleWebcam,
}

pub struct AppState {
    pub is_recording: bool,
    pub is_paused: bool,
    pub record_audio: bool,
    pub record_video: bool,
    pub record_webcam: bool,
    pub separate_outputs: bool,
    pub selected_screen: Option<usize>,
    pub selected_audio_device: Option<String>,
    pub selected_webcam: Option<String>,
    pub output_path: String,
    pub video_quality: VideoQuality,
    pub audio_quality: AudioQuality,
    pub config: Config,
    pub show_settings: bool,
    pub audio_gain_db: f32,
    pub overlay_position: (u32, u32),
    pub overlay_size: (u32, u32),
    pub overlay_opacity: f32,
    pub hotkeys: HotkeyConfig,
    pub use_pipewire_on_wayland: bool,
    pub enable_preview_overlay: bool,
    pub screen_zoom: f32,
    pub webcam_zoom: f32,
}

impl AppState {
    pub fn new() -> Self {
        let config = Config::load().unwrap_or_default();
        let enable_preview_overlay = config.enable_preview_overlay;
        Self {
            is_recording: false,
            is_paused: false,
            record_audio: config.record_audio,
            record_video: config.record_video,
            record_webcam: config.record_webcam,
            separate_outputs: config.separate_outputs,
            selected_screen: config.default_screen,
            selected_audio_device: config.default_audio_device.clone(),
            selected_webcam: config.default_webcam.clone(),
            output_path: config.get_output_directory().to_string(),
            video_quality: config.video_quality,
            audio_quality: config.audio_quality,
            show_settings: false,
            audio_gain_db: 0.0,
            overlay_position: (40, 40),
            overlay_size: (320, 180),
            overlay_opacity: 0.9,
            hotkeys: HotkeyConfig::default(),
            use_pipewire_on_wayland: false,
            enable_preview_overlay,
            screen_zoom: 1.0,
            webcam_zoom: 1.0,
            config,
        }
    }
}

#[derive(Default)]
struct HotkeyTriggers {
    toggle_record: bool,
    toggle_pause: bool,
    toggle_webcam: bool,
}

fn format_shortcut(shortcut: &KeyboardShortcut) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if shortcut.modifiers.ctrl { parts.push("Ctrl"); }
    if shortcut.modifiers.shift { parts.push("Shift"); }
    if shortcut.modifiers.alt { parts.push("Alt"); }
    if shortcut.modifiers.command { parts.push("Cmd"); }
    parts.push(shortcut.logical_key.name());
    parts.join(" + ")
}

fn capture_shortcut(ctx: &egui::Context) -> Option<KeyboardShortcut> {
    let mut captured = None;
    ctx.input(|input| {
        let modifiers = input.modifiers;
        for event in &input.events {
            if let egui::Event::Key { key, pressed, repeat, .. } = event {
                if *pressed && !repeat {
                    captured = Some(KeyboardShortcut::new(modifiers, *key));
                }
            }
        }
    });
    captured
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
    screen_preview_texture: Option<TextureHandle>,
    webcam_preview_texture: Option<TextureHandle>,
    audio_level: f32,
    awaiting_hotkey: Option<HotkeyAction>,
    active_screen_index: Option<usize>,
    active_webcam_name: Option<String>,
    dragging_overlay: bool,
    active_resize: Option<ResizeHandle>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ResizeHandle {
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
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
            screen_preview_texture: None,
            webcam_preview_texture: None,
            audio_level: 0.0,
            awaiting_hotkey: None,
            active_screen_index: None,
            active_webcam_name: None,
            dragging_overlay: false,
            active_resize: None,
        };

        // Initialize available devices
        if let Err(e) = app.refresh_devices() {
            error!("Failed to refresh devices: {}", e);
        }

        app.initialize_previews();

        app
    }

    fn refresh_devices(&mut self) -> Result<()> {
        self.available_screens = screen::get_available_screens()?;

        self.available_audio_devices = audio::get_available_devices()?;

        self.available_webcams = webcam::get_available_webcams()?;

        Ok(())
    }

    fn initialize_previews(&mut self) {
        let (record_video, record_webcam, screen_index, webcam_name) = {
            let state = self.state.lock().unwrap();
            (
                state.record_video,
                state.record_webcam,
                state.selected_screen.unwrap_or(0),
                state.selected_webcam.clone().unwrap_or_else(|| "Default Webcam".to_string()),
            )
        };

        if record_video && self.screen_capture.is_none() {
            if let Ok(mut capture) = ScreenCapture::new(screen_index) {
                if let Err(e) = capture.start() {
                    error!("Failed to start screen preview: {}", e);
                } else {
                    self.screen_capture = Some(capture);
                    self.active_screen_index = Some(screen_index);
                }
            }
        }

        if record_webcam && self.webcam_capture.is_none() {
            if let Ok(mut capture) = WebcamCapture::new(&webcam_name) {
                if let Err(e) = capture.start() {
                    error!("Failed to start webcam preview: {}", e);
                } else {
                    self.webcam_capture = Some(capture);
                    self.active_webcam_name = Some(webcam_name);
                }
            }
        }
    }

    fn ensure_capture_state(&mut self) {
        let (record_video, screen_index, record_webcam, webcam_name) = {
            let state = self.state.lock().unwrap();
            (
                state.record_video,
                state.selected_screen.unwrap_or(0),
                state.record_webcam,
                state
                    .selected_webcam
                    .clone()
                    .unwrap_or_else(|| "Default Webcam".to_string()),
            )
        };

        if record_video {
            if self.active_screen_index != Some(screen_index) {
                if let Some(capture) = self.screen_capture.as_mut() {
                    let _ = capture.stop();
                }
                self.screen_capture = None;
                match ScreenCapture::new(screen_index) {
                    Ok(mut capture) => {
                        if let Err(e) = capture.start() {
                            error!("Failed to start screen capture: {}", e);
                        } else {
                            self.screen_capture = Some(capture);
                            self.active_screen_index = Some(screen_index);
                        }
                    }
                    Err(e) => error!("Failed to create screen capture: {}", e),
                }
            }
        } else if self.screen_capture.is_some() {
            if let Some(capture) = self.screen_capture.as_mut() {
                let _ = capture.stop();
            }
            self.screen_capture = None;
            self.active_screen_index = None;
        }

        if record_webcam {
            if self.active_webcam_name.as_deref() != Some(webcam_name.as_str()) {
                if let Some(capture) = self.webcam_capture.as_mut() {
                    let _ = capture.stop();
                }
                self.webcam_capture = None;
                match WebcamCapture::new(&webcam_name) {
                    Ok(mut capture) => {
                        if let Err(e) = capture.start() {
                            error!("Failed to start webcam capture: {}", e);
                        } else {
                            self.webcam_capture = Some(capture);
                            self.active_webcam_name = Some(webcam_name);
                        }
                    }
                    Err(e) => error!("Failed to create webcam capture: {}", e),
                }
            }
        } else if self.webcam_capture.is_some() {
            if let Some(capture) = self.webcam_capture.as_mut() {
                let _ = capture.stop();
            }
            self.webcam_capture = None;
            self.active_webcam_name = None;
        }
    }

    fn set_hotkey(&mut self, action: HotkeyAction, shortcut: KeyboardShortcut) {
        let mut state = self.state.lock().unwrap();
        match action {
            HotkeyAction::StartStop => state.hotkeys.start_stop = shortcut,
            HotkeyAction::PauseResume => state.hotkeys.pause_resume = shortcut,
            HotkeyAction::ToggleWebcam => state.hotkeys.toggle_webcam = shortcut,
        }
    }

    fn handle_hotkeys(&mut self, ctx: &egui::Context) -> HotkeyTriggers {
        let mut triggers = HotkeyTriggers::default();

        if let Some(action) = self.awaiting_hotkey {
            if let Some(shortcut) = capture_shortcut(ctx) {
                self.set_hotkey(action, shortcut);
                self.awaiting_hotkey = None;
            } else if ctx.input(|i| i.key_pressed(Key::Escape)) {
                self.awaiting_hotkey = None;
            }
            return triggers;
        }

        let hotkeys = {
            self.state.lock().unwrap().hotkeys.clone()
        };
 
        if ctx.input_mut(|i| i.consume_shortcut(&hotkeys.start_stop)) {
            triggers.toggle_record = true;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&hotkeys.pause_resume)) {
            triggers.toggle_pause = true;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&hotkeys.toggle_webcam)) {
            triggers.toggle_webcam = true;
        }

        triggers
    }

    fn toggle_recording(&mut self) {
        let is_recording = { self.state.lock().unwrap().is_recording };
        if is_recording {
            if let Err(e) = self.stop_recording() {
                error!("Failed to stop recording: {}", e);
            }
        } else if let Err(e) = self.start_recording() {
            error!("Failed to start recording: {}", e);
        }
    }

    fn toggle_pause(&mut self) {
        let should_toggle = {
            let state = self.state.lock().unwrap();
            state.is_recording
        };

        if !should_toggle {
            return;
        }

        if let Some(encoder) = self.video_encoder.as_mut() {
            if let Err(e) = encoder.toggle_pause() {
                error!("Failed to toggle pause: {}", e);
                return;
            }
        }

        let mut state = self.state.lock().unwrap();
        state.is_paused = !state.is_paused;
    }

    fn toggle_webcam_capture(&mut self) {
        let mut state = self.state.lock().unwrap();
        state.record_webcam = !state.record_webcam;
        let desired = state.record_webcam;
        let webcam_name = state
            .selected_webcam
            .clone()
            .unwrap_or_else(|| "Default Webcam".to_string());
        drop(state);

        if desired {
            if let Some(capture) = self.webcam_capture.as_mut() {
                let _ = capture.stop();
            }
            self.webcam_capture = None;
            match WebcamCapture::new(&webcam_name) {
                Ok(mut capture) => {
                    if let Err(e) = capture.start() {
                        error!("Failed to start webcam capture: {}", e);
                    } else {
                        self.webcam_capture = Some(capture);
                        self.active_webcam_name = Some(webcam_name);
                    }
                }
                Err(e) => error!("Failed to create webcam capture: {}", e),
            }
        } else {
            if let Some(capture) = self.webcam_capture.as_mut() {
                let _ = capture.stop();
            }
            self.webcam_capture = None;
            self.active_webcam_name = None;
        }
    }

    fn draw_settings_contents(&mut self, ui: &mut egui::Ui) -> bool {
        let mut state = self.state.lock().unwrap();
        let mut refresh_requested = false;

        ui.heading("Capture Options");
        ui.checkbox(&mut state.record_audio, "Record system audio");
        ui.checkbox(&mut state.record_video, "Record screen");
        ui.checkbox(&mut state.record_webcam, "Enable webcam overlay");
        ui.checkbox(&mut state.separate_outputs, "Save audio and video separately");

        ui.separator();
        ui.heading("Audio");
        ui.label("Input Gain (dB)");
        ui.add(Slider::new(&mut state.audio_gain_db, -30.0..=12.0).suffix(" dB"));

        if state.record_video {
            ui.separator();
            ui.label("Screen");
            let selected_text = state
                .selected_screen
                .and_then(|idx| self.available_screens.get(idx).cloned())
                .unwrap_or_else(|| "Select screen".to_string());
            egui::ComboBox::from_id_salt("settings_screen")
                .selected_text(selected_text)
                .show_ui(ui, |ui| {
                    for (idx, screen) in self.available_screens.iter().enumerate() {
                        ui.selectable_value(&mut state.selected_screen, Some(idx), screen);
                    }
                });
        }

        if state.record_audio {
            ui.separator();
            ui.label("Audio Device");
            let selected_text = state
                .selected_audio_device
                .clone()
                .unwrap_or_else(|| "default".to_string());
            egui::ComboBox::from_id_salt("settings_audio_device")
                .selected_text(selected_text)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut state.selected_audio_device, None, "default");
                    for device in &self.available_audio_devices {
                        ui.selectable_value(&mut state.selected_audio_device, Some(device.clone()), device);
                    }
                });
        }

        if state.record_webcam {
            ui.separator();
            ui.label("Webcam Device");
            let selected_text = state
                .selected_webcam
                .clone()
                .unwrap_or_else(|| "Select webcam".to_string());
            egui::ComboBox::from_id_salt("settings_webcam")
                .selected_text(selected_text)
                .show_ui(ui, |ui| {
                    for webcam in &self.available_webcams {
                        ui.selectable_value(&mut state.selected_webcam, Some(webcam.clone()), webcam);
                    }
                });
        }

        ui.separator();
        ui.heading("Webcam Overlay");
        let mut overlay_x = state.overlay_position.0 as i32;
        let mut overlay_y = state.overlay_position.1 as i32;
        let mut overlay_width = state.overlay_size.0 as i32;
        let mut overlay_height = state.overlay_size.1 as i32;

        ui.horizontal(|ui| {
            ui.label("X");
            if ui.add(DragValue::new(&mut overlay_x).range(0..=8000)).changed() {
                state.overlay_position.0 = overlay_x.max(0) as u32;
            }
            ui.label("Y");
            if ui.add(DragValue::new(&mut overlay_y).range(0..=8000)).changed() {
                state.overlay_position.1 = overlay_y.max(0) as u32;
            }
        });

        ui.horizontal(|ui| {
            ui.label("Width");
            if ui.add(DragValue::new(&mut overlay_width).range(1..=8000)).changed() {
                state.overlay_size.0 = overlay_width.max(1) as u32;
            }
            ui.label("Height");
            if ui.add(DragValue::new(&mut overlay_height).range(1..=8000)).changed() {
                state.overlay_size.1 = overlay_height.max(1) as u32;
            }
        });

        ui.horizontal(|ui| {
            ui.label("Opacity");
            ui.add(Slider::new(&mut state.overlay_opacity, 0.0..=1.0));
        });

        ui.separator();
        ui.heading("Hotkeys");

        ui.separator();
        ui.heading("Wayland");
        let ffmpeg_has_pipewire = crate::video::ffmpeg_supports_pipewire("ffmpeg");
        ui.add_enabled(
            ffmpeg_has_pipewire,
            egui::Checkbox::without_text(&mut state.use_pipewire_on_wayland)
        ).on_hover_text(
            if ffmpeg_has_pipewire { "Use Wayland PipeWire capture (experimental)" } else { "ffmpeg pipewire input not available" }
        );
        state.config.enable_preview_overlay = state.enable_preview_overlay;
 
        let default_hotkeys = HotkeyConfig::default();

        ui.horizontal(|ui| {
            ui.label("Start/Stop Recording");
            let button_label = if self.awaiting_hotkey == Some(HotkeyAction::StartStop) {
                "Press keys...".to_string()
            } else {
                format_shortcut(&state.hotkeys.start_stop)
            };
            if ui.button(button_label).clicked() {
                self.awaiting_hotkey = Some(HotkeyAction::StartStop);
            }
            if ui.small_button("Reset").clicked() {
                state.hotkeys.start_stop = default_hotkeys.start_stop;
            }
        });

        ui.horizontal(|ui| {
            ui.label("Pause/Resume");
            let button_label = if self.awaiting_hotkey == Some(HotkeyAction::PauseResume) {
                "Press keys...".to_string()
            } else {
                format_shortcut(&state.hotkeys.pause_resume)
            };
            if ui.button(button_label).clicked() {
                self.awaiting_hotkey = Some(HotkeyAction::PauseResume);
            }
            if ui.small_button("Reset").clicked() {
                state.hotkeys.pause_resume = default_hotkeys.pause_resume;
            }
        });

        ui.horizontal(|ui| {
            ui.label("Toggle Webcam");
            let button_label = if self.awaiting_hotkey == Some(HotkeyAction::ToggleWebcam) {
                "Press keys...".to_string()
            } else {
                format_shortcut(&state.hotkeys.toggle_webcam)
            };
            if ui.button(button_label).clicked() {
                self.awaiting_hotkey = Some(HotkeyAction::ToggleWebcam);
            }
            if ui.small_button("Reset").clicked() {
                state.hotkeys.toggle_webcam = default_hotkeys.toggle_webcam;
            }
        });
 
        ui.separator();
        if ui.button("Refresh device list").clicked() {
            refresh_requested = true;
        }

        refresh_requested
    }

    fn start_recording(&mut self) -> Result<()> {
        let (
            output_path,
            include_audio,
            include_video,
            include_webcam,
            separate_outputs,
            selected_screen,
            audio_device_opt,
            webcam_device_opt,
            audio_gain_db,
            use_pipewire_on_wayland,
            enable_preview_overlay,
            video_quality,
            audio_quality,
            mut config_snapshot,
        ) = {
            let state = self.state.lock().unwrap();
            if state.is_recording {
                return Ok(());
            }

            (
                state.output_path.clone(),
                state.record_audio,
                state.record_video,
                state.record_webcam,
                state.separate_outputs,
                state.selected_screen,
                state.selected_audio_device.clone(),
                state.selected_webcam.clone(),
                state.audio_gain_db,
                state.use_pipewire_on_wayland,
                state.enable_preview_overlay,
                state.video_quality,
                state.audio_quality,
                state.config.clone(),
            )
        };

        info!("Starting recording");

        // Ensure output directory exists
        std::fs::create_dir_all(&output_path)?;

        config_snapshot.set_output_directory(output_path.clone());
        config_snapshot.record_audio = include_audio;
        config_snapshot.record_video = include_video;
        config_snapshot.record_webcam = include_webcam;
        config_snapshot.separate_outputs = separate_outputs;
        config_snapshot.use_pipewire_on_wayland = use_pipewire_on_wayland;
        config_snapshot.enable_preview_overlay = enable_preview_overlay;
        config_snapshot.video_quality = video_quality;
        config_snapshot.audio_quality = audio_quality;
        config_snapshot.default_screen = selected_screen;
        config_snapshot.default_audio_device = audio_device_opt.clone();
        config_snapshot.default_webcam = webcam_device_opt.clone();

        config_snapshot.save()?;

        // Reflect Wayland PipeWire preference via environment for the encoder
        if use_pipewire_on_wayland {
            std::env::set_var("OCTOCORD_USE_PIPEWIRE", "1");
        } else {
            std::env::remove_var("OCTOCORD_USE_PIPEWIRE");
        }

        let options = RecorderOptions {
            output_directory: PathBuf::from(config_snapshot.get_output_directory()),
            video_quality: config_snapshot.video_quality,
            video_bitrate_kbps: config_snapshot.get_video_bitrate(),
            audio_bitrate_kbps: config_snapshot.get_audio_bitrate(),
            audio_sample_rate: config_snapshot.get_audio_sample_rate(),
            frame_rate: 60,
            include_audio,
            include_video,
            include_webcam,
            separate_outputs,
            selected_screen,
            audio_device: audio_device_opt.clone(),
            webcam_device: webcam_device_opt.clone(),
            ffmpeg_path: "ffmpeg".to_string(),
            audio_gain_db,
        };

        if include_video {
            let screen_index = selected_screen.unwrap_or(0);
            if self.screen_capture.is_none() {
                match ScreenCapture::new(screen_index) {
                    Ok(mut capture) => {
                        if let Err(e) = capture.start() {
                            error!("Failed to start screen capture (screen_index={}): {}", screen_index, e);
                        } else {
                            self.screen_capture = Some(capture);
                        }
                    }
                    Err(e) => error!("Failed to create screen capture (screen_index={}): {}", screen_index, e),
                }
            }
        }

        // To avoid v4l2 device busy when ffmpeg opens the webcam, stop preview capture first.
        if include_webcam {
            if let Some(capture) = self.webcam_capture.as_mut() {
                let _ = capture.stop();
            }
            self.webcam_capture = None;
        }

        self.video_encoder = Some(VideoEncoder::new(options)?);
        if let Some(encoder) = self.video_encoder.as_mut() {
            if let Err(e) = encoder.start() {
                error!("Failed to start encoder: {}", e);
                self.video_encoder = None;
                return Err(e);
            }
        }

        if include_audio {
            let device_name = audio_device_opt
                .as_deref()
                .filter(|name| !name.is_empty())
                .unwrap_or("default");

            match AudioRecorder::new(device_name) {
                Ok(mut recorder) => {
                    if let Err(err) = recorder.start() {
                        error!("Failed to start audio monitor: {}", err);
                    } else {
                        self.audio_recorder = Some(recorder);
                    }
                }
                Err(err) => {
                    error!("Failed to initialize audio monitor: {}", err);
                }
            }
        }

        {
            let mut state = self.state.lock().unwrap();
            state.is_recording = true;
            state.is_paused = false;
            state.config = config_snapshot;
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
        state.is_paused = false;

        if let Some(encoder) = &mut self.video_encoder {
            encoder.stop()?;
        }

        if let Some(recorder) = &mut self.audio_recorder {
            let _ = recorder.stop();
        }
        self.audio_recorder = None;

        if let Some(capture) = &mut self.screen_capture {
            let _ = capture.stop();
        }
        self.screen_capture = None;

        if let Some(capture) = &mut self.webcam_capture {
            let _ = capture.stop();
        }
        self.webcam_capture = None;
        self.video_encoder = None;

        // Restore previews to prior state
        {
            let state = self.state.lock().unwrap();
            if state.record_video && self.screen_capture.is_none() {
                if let Some(idx) = state.selected_screen {
                    if let Ok(mut cap) = ScreenCapture::new(idx) { let _ = cap.start(); self.screen_capture = Some(cap); }
                }
            }
            if state.record_webcam && self.webcam_capture.is_none() {
                let name = state.selected_webcam.clone().unwrap_or_else(|| "Default Webcam".to_string());
                if let Ok(mut cap) = WebcamCapture::new(&name) { let _ = cap.start(); self.webcam_capture = Some(cap); }
            }
        }

        Ok(())
    }
}

impl eframe::App for DiscordRecorderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.set_style(discord_style());

        let hotkey_triggers = self.handle_hotkeys(ctx);
        self.ensure_capture_state();

        let audio_gain_db = { self.state.lock().unwrap().audio_gain_db };
        let audio_gain_linear = 10f32.powf(audio_gain_db / 20.0);

        let mut toggle_record_click = false;
        let mut toggle_pause_click = false;
        let mut toggle_webcam_click = false;

        TopBottomPanel::top("controls_panel").show(ctx, |ui| {
            let mut state = self.state.lock().unwrap();

            ui.horizontal_centered(|ui| {
                let record_label = if state.is_recording {
                    "⏹ Stop Recording"
                } else {
                    "⏺ Start Recording"
                };
                let record_color = if state.is_recording {
                    Color32::from_rgb(240, 71, 71)
                } else {
                    Color32::from_rgb(35, 165, 90)
                };
                if ui
                    .add_sized(
                        [150.0, 44.0],
                        egui::Button::new(RichText::new(record_label).size(16.0))
                            .fill(record_color)
                            .corner_radius(10),
                    )
                    .clicked()
                {
                    toggle_record_click = true;
                }

                let pause_enabled = state.is_recording;
                let pause_label = if state.is_paused { "▶ Resume" } else { "⏸ Pause" };
                let pause_button = egui::Button::new(pause_label).min_size(vec2(120.0, 40.0));
                if ui.add_enabled(pause_enabled, pause_button).clicked() && pause_enabled
                {
                    toggle_pause_click = true;
                }

                let webcam_label = if state.record_webcam {
                    "📷 Webcam On"
                } else {
                    "📷 Webcam Off"
                };
                if ui
                    .add_sized([140.0, 40.0], egui::Button::new(webcam_label))
                    .clicked()
                {
                    toggle_webcam_click = true;
                }

                if ui
                    .add_sized([120.0, 40.0], egui::Button::new("⚙ Settings"))
                    .clicked()
                {
                    state.show_settings = true;
                }

                let status_text = if state.is_recording {
                    if state.is_paused {
                        "⏸ Paused"
                    } else {
                        "● REC"
                    }
                } else {
                    "● IDLE"
                };
                let status_color = if state.is_recording {
                    if state.is_paused {
                        Color32::from_rgb(255, 180, 0)
                    } else {
                        Color32::from_rgb(240, 71, 71)
                    }
                } else {
                    Color32::from_rgb(116, 127, 141)
                };
                ui.colored_label(status_color, RichText::new(status_text).size(14.0));
            });
        });

        let screen_frame_opt = self
            .screen_capture
            .as_ref()
            .and_then(|capture| capture.get_latest_frame());
        let webcam_frame_opt = self
            .webcam_capture
            .as_ref()
            .and_then(|capture| capture.get_latest_frame());

        // Update textures; optionally composite webcam over screen for preview when enabled
        let enable_overlay = { self.state.lock().unwrap().enable_preview_overlay };

        #[cfg(feature = "webcam")]
        {
            if enable_overlay {
                if let (Some(screen_frame), Some(webcam_frame)) = (screen_frame_opt.as_ref(), webcam_frame_opt.as_ref()) {
                    let (pos, size, opacity) = {
                        let st = self.state.lock().unwrap();
                        (st.overlay_position, st.overlay_size, st.overlay_opacity)
                    };
                    let mut composed = screen_frame.clone();
                    let mut overlay = WebcamOverlay::new(pos.0, pos.1, size.0, size.1);
                    overlay.set_opacity(opacity);
                    overlay.overlay_onto(webcam_frame, &mut composed);
                    update_texture(ctx, &mut self.screen_preview_texture, &composed, "screen_preview");
                } else if let Some(screen_frame) = screen_frame_opt.as_ref() {
                    update_texture(ctx, &mut self.screen_preview_texture, screen_frame, "screen_preview");
                }
            } else if let Some(screen_frame) = screen_frame_opt.as_ref() {
                update_texture(ctx, &mut self.screen_preview_texture, screen_frame, "screen_preview");
            }
        }

        #[cfg(not(feature = "webcam"))]
        {
            if let Some(screen_frame) = screen_frame_opt.as_ref() {
                let _ = enable_overlay; // keep var used when feature off
                update_texture(ctx, &mut self.screen_preview_texture, screen_frame, "screen_preview");
            }
        }

        if let Some(webcam_frame) = webcam_frame_opt.as_ref() {
            update_texture(ctx, &mut self.webcam_preview_texture, webcam_frame, "webcam_preview");
        }

        if let Some(recorder) = self.audio_recorder.as_ref() {
            let data = recorder.get_audio_data();
            if recorder.is_recording() && !data.is_empty() {
                let processor = AudioProcessor::new(recorder.get_sample_rate(), recorder.get_channels());
                let mono = processor.mix_to_mono(&data);
                let peak = mono
                    .iter()
                    .copied()
                    .map(f32::abs)
                    .fold(0.0, f32::max)
                    .clamp(0.0, 1.0)
                    * audio_gain_linear;
                self.audio_level = self.audio_level * 0.8 + peak.clamp(0.0, 1.0) * 0.2;
            } else {
                self.audio_level *= 0.95;
            }
        }

        let mut refresh_requested = false;
        let show_settings = { self.state.lock().unwrap().show_settings };
        if show_settings {
            let mut open_flag = show_settings;
            egui::Window::new("Settings")
                .open(&mut open_flag)
                .resizable(true)
                .show(ctx, |ui| {
                    if let Some(action) = self.awaiting_hotkey {
                        let action_name = match action {
                            HotkeyAction::StartStop => "Start/Stop Recording",
                            HotkeyAction::PauseResume => "Pause/Resume",
                            HotkeyAction::ToggleWebcam => "Toggle Webcam",
                        };
                        ui.colored_label(Color32::from_rgb(255, 180, 0), format!(
                            "Waiting for new shortcut for {action_name}. Press desired keys or Esc to cancel."
                        ));
                        ui.separator();
                    }

                    refresh_requested |= self.draw_settings_contents(ui);
                });

            {
                let mut state = self.state.lock().unwrap();
                state.show_settings = open_flag;
            }
        }

        if refresh_requested {
            if let Err(e) = self.refresh_devices() {
                error!("Failed to refresh devices: {}", e);
            } else {
                self.ensure_capture_state();
            }
        }

        CentralPanel::default().show(ctx, |ui| {
            ui.heading("Preview");
            ui.separator();

            if let Some(texture) = &self.screen_preview_texture {
                let size = texture.size();
                let tex_w = size[0] as f32;
                let tex_h = size[1] as f32;
                let avail = ui.available_size();
                let max_w = avail.x.max(100.0);
                let max_h = (avail.y * 0.6).max(100.0);
                let zoom = { self.state.lock().unwrap().screen_zoom }.clamp(0.25, 4.0);
                let fit_scale = (max_w / tex_w).min(max_h / tex_h).min(1.0);
                let scale = (fit_scale * zoom).max(0.1);
                let disp = vec2(tex_w * scale, tex_h * scale);
                let response = ui.image((texture.id(), disp));

                // Draw draggable/resizable overlay guides when enabled
                let enable_overlay = { self.state.lock().unwrap().enable_preview_overlay };
                if enable_overlay {
                    let rect = response.rect;
                    self.handle_overlay_interactions(ui, rect, scale, tex_w, tex_h);
                }
            } else {
                ui.label("No screen preview available");
            }

            ui.separator();

            if let Some(texture) = &self.webcam_preview_texture {
                let size = texture.size();
                let tex_w = size[0] as f32;
                let tex_h = size[1] as f32;
                let avail = ui.available_size();
                let max_w = avail.x.max(100.0);
                let max_h = (avail.y * 0.3).max(80.0);
                let zoom = { self.state.lock().unwrap().webcam_zoom }.clamp(0.25, 4.0);
                let fit_scale = (max_w / tex_w).min(max_h / tex_h).min(1.0);
                let scale = (fit_scale * zoom).max(0.1);
                let disp = vec2(tex_w * scale, tex_h * scale);
                ui.image((texture.id(), disp));
            } else {
                ui.label("No webcam preview available");
            }

            ui.separator();
            ui.label("Audio Level");
            ui.add(ProgressBar::new(self.audio_level.clamp(0.0, 1.0)).desired_width(200.0));

            // Zoom controls
            let mut screen_zoom = { self.state.lock().unwrap().screen_zoom };
            let mut webcam_zoom = { self.state.lock().unwrap().webcam_zoom };
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Screen zoom");
                if ui.add(Slider::new(&mut screen_zoom, 0.25..=4.0)).changed() {
                    self.state.lock().unwrap().screen_zoom = screen_zoom;
                }
                ui.label("Webcam zoom");
                if ui.add(Slider::new(&mut webcam_zoom, 0.25..=4.0)).changed() {
                    self.state.lock().unwrap().webcam_zoom = webcam_zoom;
                }
            });
        });

        if toggle_record_click || hotkey_triggers.toggle_record {
            self.toggle_recording();
        }
        if toggle_pause_click || hotkey_triggers.toggle_pause {
            self.toggle_pause();
        }
        if toggle_webcam_click || hotkey_triggers.toggle_webcam {
            self.toggle_webcam_capture();
        }
    }
}

fn discord_style() -> egui::Style {
    let mut style = egui::Style::default();

    // Discord color palette
    style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(88, 101, 242);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(71, 82, 196);
    style.visuals.widgets.active.bg_fill = Color32::from_rgb(58, 67, 159);

    // Background colors
    style.visuals.panel_fill = Color32::from_rgb(54, 57, 63);
    style.visuals.extreme_bg_color = Color32::from_rgb(47, 49, 54);
    style.visuals.code_bg_color = Color32::from_rgb(40, 42, 46);

    // Text colors
    style.visuals.widgets.inactive.fg_stroke.color = Color32::from_rgb(255, 255, 255);
    // Fixed: Added .widgets. to the path
    style.visuals.widgets.noninteractive.fg_stroke.color = Color32::from_rgb(185, 187, 190);

    // Window styling
    style.visuals.window_fill = Color32::from_rgb(54, 57, 63);
    style.visuals.window_stroke = Stroke::new(1.0, Color32::from_rgb(32, 34, 37));

    // Spacing
    style.spacing.item_spacing = egui::Vec2::new(8.0, 8.0);
    style.spacing.window_margin = egui::Margin::same(8); // Changed: 8.0 -> 8

    style
}

fn update_texture(
    ctx: &egui::Context,
    texture: &mut Option<TextureHandle>,
    image: &DynamicImage,
    name: &str,
) {
    let rgba = image.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    let color_image = ColorImage::from_rgba_unmultiplied(size, &rgba);

    if let Some(handle) = texture {
        handle.set(color_image, TextureOptions::LINEAR);
    } else {
        *texture = Some(ctx.load_texture(name.to_string(), color_image, TextureOptions::LINEAR));
    }
}

impl DiscordRecorderApp {
    fn handle_overlay_interactions(&mut self, ui: &mut egui::Ui, image_rect: egui::Rect, scale: f32, tex_w: f32, tex_h: f32) {
        let (pos, size, opacity) = {
            let st = self.state.lock().unwrap();
            (st.overlay_position, st.overlay_size, st.overlay_opacity)
        };

        let top_left = image_rect.min + egui::vec2(pos.0 as f32 * scale, pos.1 as f32 * scale);
        let overlay_size = egui::vec2(size.0 as f32 * scale, size.1 as f32 * scale);
        let overlay_rect = egui::Rect::from_min_size(top_left, overlay_size);

        // Draw border for visual feedback
        let stroke = Stroke::new(2.0, Color32::from_rgb(88, 101, 242));
        let tl = overlay_rect.left_top();
        let tr = overlay_rect.right_top();
        let bl = overlay_rect.left_bottom();
        let br = overlay_rect.right_bottom();
        ui.painter().line_segment([tl, tr], stroke);
        ui.painter().line_segment([tr, br], stroke);
        ui.painter().line_segment([br, bl], stroke);
        ui.painter().line_segment([bl, tl], stroke);

        // Draw resize handles
        let handle_s = 10.0;
        let corners = [
            (ResizeHandle::NW, overlay_rect.left_top()),
            (ResizeHandle::NE, overlay_rect.right_top()),
            (ResizeHandle::SW, overlay_rect.left_bottom()),
            (ResizeHandle::SE, overlay_rect.right_bottom()),
        ];
        for (handle, center) in corners {
            let hr = egui::Rect::from_center_size(center, egui::vec2(handle_s, handle_s));
            ui.painter().rect_filled(hr, 2.0, Color32::from_rgb(71, 82, 196));
            let resp = ui.interact(hr, ui.make_persistent_id(format!("overlay_handle_{:?}", handle)), egui::Sense::click_and_drag());
            if resp.drag_started() {
                self.active_resize = Some(handle);
            }
        }

        // Edge handles
        let edges = [
            (ResizeHandle::N, egui::pos2(overlay_rect.center().x, overlay_rect.top())),
            (ResizeHandle::S, egui::pos2(overlay_rect.center().x, overlay_rect.bottom())),
            (ResizeHandle::W, egui::pos2(overlay_rect.left(), overlay_rect.center().y)),
            (ResizeHandle::E, egui::pos2(overlay_rect.right(), overlay_rect.center().y)),
        ];
        for (handle, center) in edges {
            let size_vec = if matches!(handle, ResizeHandle::N | ResizeHandle::S) { egui::vec2(handle_s, handle_s * 0.6) } else { egui::vec2(handle_s * 0.6, handle_s) };
            let hr = egui::Rect::from_center_size(center, size_vec);
            ui.painter().rect_filled(hr, 1.0, Color32::from_rgb(71, 82, 196));
            let resp = ui.interact(hr, ui.make_persistent_id(format!("overlay_edge_{:?}", handle)), egui::Sense::click_and_drag());
            if resp.drag_started() {
                self.active_resize = Some(handle);
            }
        }

        // Drag body
        let body_resp = ui.interact(overlay_rect, ui.make_persistent_id("overlay_body"), egui::Sense::click_and_drag());
        if body_resp.drag_started() {
            self.dragging_overlay = true;
            self.active_resize = None;
        }

        let mut st = self.state.lock().unwrap();
        let min_w = 32.0f32.max(4.0);
        let min_h = 32.0f32.max(4.0);
        let _img_w = tex_w * scale;
        let _img_h = tex_h * scale;

        // Apply dragging
        if self.dragging_overlay && body_resp.dragged() {
            let delta = body_resp.drag_delta();
            let mut new_x = st.overlay_position.0 as f32 + (delta.x / scale);
            let mut new_y = st.overlay_position.1 as f32 + (delta.y / scale);
            new_x = new_x.clamp(0.0, (tex_w - st.overlay_size.0 as f32).max(0.0));
            new_y = new_y.clamp(0.0, (tex_h - st.overlay_size.1 as f32).max(0.0));
            st.overlay_position.0 = new_x.round() as u32;
            st.overlay_position.1 = new_y.round() as u32;
            ui.ctx().request_repaint();
        }

        if ui.input(|i| !i.pointer.button_down(egui::PointerButton::Primary)) {
            self.dragging_overlay = false;
        }

        // Apply resizing
        if let Some(handle) = self.active_resize {
            let pointer_pos = ui.input(|i| i.pointer.interact_pos());
            if let Some(pp) = pointer_pos {
                // convert to image space
                let img_origin = image_rect.min;
                let px = ((pp.x - img_origin.x) / scale).clamp(0.0, tex_w);
                let py = ((pp.y - img_origin.y) / scale).clamp(0.0, tex_h);

                let mut x = st.overlay_position.0 as f32;
                let mut y = st.overlay_position.1 as f32;
                let mut w = st.overlay_size.0 as f32;
                let mut h = st.overlay_size.1 as f32;

                match handle {
                    ResizeHandle::NW => { w += x - px; h += y - py; x = px; y = py; }
                    ResizeHandle::NE => { w = (px - x).max(min_w / scale); h += y - py; y = py; }
                    ResizeHandle::SW => { w += x - px; x = px; h = (py - y).max(min_h / scale); }
                    ResizeHandle::SE => { w = (px - x).max(min_w / scale); h = (py - y).max(min_h / scale); }
                    ResizeHandle::N => { h += y - py; y = py; }
                    ResizeHandle::S => { h = (py - y).max(min_h / scale); }
                    ResizeHandle::W => { w += x - px; x = px; }
                    ResizeHandle::E => { w = (px - x).max(min_w / scale); }
                }

                // Clamp within texture bounds
                if x < 0.0 { w += x; x = 0.0; }
                if y < 0.0 { h += y; y = 0.0; }
                if x + w > tex_w { w = tex_w - x; }
                if y + h > tex_h { h = tex_h - y; }

                w = w.max(min_w / scale);
                h = h.max(min_h / scale);

                st.overlay_position = (x.round() as u32, y.round() as u32);
                st.overlay_size = (w.round() as u32, h.round() as u32);
                ui.ctx().request_repaint();
            }

            // Release on mouse up
            if ui.input(|i| !i.pointer.button_down(egui::PointerButton::Primary)) {
                self.active_resize = None;
            }
        }

        let _ = opacity; // kept in case we later reflect in guides
    }
}
