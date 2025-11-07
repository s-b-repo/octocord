use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{available_hosts, Device, Host, HostId, SampleFormat, SupportedStreamConfig};
use log::{info, error};
use num_traits::ToPrimitive; // <-- added
use std::env;
use std::sync::{Arc, Mutex};

fn pick_best_input_host() -> Host {
    // Honor explicit override for common Linux hosts we can reliably map
    if let Ok(force) = env::var("OCTOCORD_AUDIO_HOST") {
        if force.eq_ignore_ascii_case("alsa") {
            if let Ok(h) = cpal::host_from_id(HostId::Alsa) {
                return h;
            }
        }
    }

    // Prefer JACK then ALSA if available and have at least one input device
    let mut preferred: Vec<HostId> = Vec::new();
    let avail = available_hosts();
    if avail.contains(&HostId::Alsa) {
        preferred.push(HostId::Alsa);
    }
    // Add the rest to try something that works
    for id in avail {
        if !preferred.contains(&id) {
            preferred.push(id);
        }
    }

    for id in preferred {
        if let Ok(h) = cpal::host_from_id(id) {
            if h.input_devices().map(|mut it| it.next().is_some()).unwrap_or(false) {
                info!("Audio host selected: {:?}", id);
                return h;
            }
        }
    }

    cpal::default_host()
}

pub struct AudioRecorder {
    device: Device,
    stream: Option<cpal::Stream>,
    is_recording: Arc<Mutex<bool>>,
    audio_data: Arc<Mutex<Vec<f32>>>,
    sample_rate: u32,
    channels: u16,
}

impl AudioRecorder {
    pub fn new(device_name: &str) -> Result<Self> {
        let host = pick_best_input_host();
        let device = find_audio_device(&host, device_name)?;
        if let Ok(name) = device.name() {
            info!("Audio input device: {}", name);
        }

        // Keep the default input config for metadata, but convert to SupportedStreamConfig when building stream
        let default_conf = device.default_input_config()?;
        let sample_rate = default_conf.sample_rate().0;
        let channels = default_conf.channels() as u16;

        Ok(Self {
            device,
            stream: None,
            is_recording: Arc::new(Mutex::new(false)),
            audio_data: Arc::new(Mutex::new(Vec::new())),
            sample_rate,
            channels,
        })
    }

    pub fn start(&mut self) -> Result<()> {
        let default_config = self.device.default_input_config()?;
        info!("Audio input config: {:?}", default_config);

        // Convert DefaultInputConfig -> SupportedStreamConfig for building the stream
        let supported_config: SupportedStreamConfig = default_config.clone().into();

        let is_recording = Arc::clone(&self.is_recording);
        let audio_data = Arc::clone(&self.audio_data);
        *is_recording.lock().unwrap() = true;

        let stream = match supported_config.sample_format() {
            SampleFormat::F32 => self.build_stream::<f32>(&supported_config, audio_data, is_recording)?,
            SampleFormat::I16 => self.build_stream::<i16>(&supported_config, audio_data, is_recording)?,
            SampleFormat::U16 => self.build_stream::<u16>(&supported_config, audio_data, is_recording)?,
            fmt => return Err(anyhow::anyhow!("Unsupported sample format: {:?}", fmt)),
        };

        // start capture
        stream.play()?;
        self.stream = Some(stream);

        info!("Audio recording started");
        Ok(())
    }

    /// Build the input stream.
    ///
    /// NOTE: we require `num_traits::ToPrimitive` so we can reliably convert arbitrary numeric sample
    /// types to `f32`. We also require `cpal::Sample` because that's the type CPAL will provide.
    ///
    /// If you'd rather use `exr::block::samples::IntoNativeSample` (from the EXR crate), you could
    /// swap the `ToPrimitive` bound for that trait and call its conversion method instead.
    fn build_stream<T>(
        &self,
        config: &SupportedStreamConfig,
        audio_data: Arc<Mutex<Vec<f32>>>,
        is_recording: Arc<Mutex<bool>>,
    ) -> Result<cpal::Stream>
    where
        T: cpal::Sample + ToPrimitive + Copy + 'static,
    {
        let err_fn = move |err| {
            error!("Audio stream error (build_stream): {}", err);
        };

        // Convert SupportedStreamConfig -> StreamConfig when calling build_input_stream
        let stream_config = config.clone().into();

        let device = self.device.clone();
        let stream = device.build_input_stream(
            &stream_config,
            move |data: &[T], _info: &cpal::InputCallbackInfo| {
                // Fast path check: if not recording, return early
                if !*is_recording.lock().unwrap() {
                    return;
                }

                // Collect into a local buffer first to avoid locking per-sample
                let mut local: Vec<f32> = Vec::with_capacity(data.len());
                for &sample in data {
                    // Try num_traits -> f32 first (returns Option<f32>), otherwise fall back to cpal's to_f32
                    let f: f32 = num_traits::ToPrimitive::to_f32(&sample)
                        .unwrap_or_else(|| cpal::Sample::to_f32(sample));
                    local.push(f);
                }

                // Lock once and extend shared buffer
                let mut audio_buffer = audio_data.lock().unwrap();
                audio_buffer.extend_from_slice(&local);
            },
            err_fn,
            None,
        )?;

        Ok(stream)
    }

    pub fn stop(&mut self) -> Result<()> {
        *self.is_recording.lock().unwrap() = false;

        if let Some(stream) = &self.stream {
            stream.pause()?;
        }

        info!("Audio recording stopped");
        Ok(())
    }

    pub fn get_audio_data(&self) -> Vec<f32> {
        let mut data = self.audio_data.lock().unwrap();
        std::mem::take(&mut *data)
    }

    pub fn get_sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn get_channels(&self) -> u16 {
        self.channels
    }

    pub fn is_recording(&self) -> bool {
        *self.is_recording.lock().unwrap()
    }
}

fn find_audio_device(host: &Host, device_name: &str) -> Result<Device> {
    let devices: Vec<_> = host.input_devices()?.collect();

    for device in &devices {
        if let Ok(name) = device.name() {
            if name == device_name {
                return Ok(device.clone());
            }
        }
    }
    for device in &devices {
        if let Ok(name) = device.name() {
            if name.to_lowercase().contains(&device_name.to_lowercase()) {
                return Ok(device.clone());
            }
        }
    }
    host.default_input_device()
        .ok_or_else(|| anyhow::anyhow!("No input device available on {:?}", host.id()))
}

pub fn get_available_devices() -> Result<Vec<String>> {
    // Try preferred hosts first, then others
    let mut try_ids: Vec<HostId> = Vec::new();
    let avail = available_hosts();
    if avail.contains(&HostId::Alsa) {
        try_ids.push(HostId::Alsa);
    }
    for id in avail {
        if !try_ids.contains(&id) {
            try_ids.push(id);
        }
    }

    for id in try_ids {
        if let Ok(h) = cpal::host_from_id(id) {
            if let Ok(mut it) = h.input_devices() {
                let mut device_names = Vec::new();
                for d in &mut it {
                    if let Ok(name) = d.name() {
                        device_names.push(name);
                    }
                }
                if !device_names.is_empty() {
                    return Ok(device_names);
                }
            }
        }
    }

    // Fallback to default host
    let host = cpal::default_host();
    let devices = host.input_devices()?;

    let mut device_names = Vec::new();
    for device in devices {
        if let Ok(name) = device.name() {
            device_names.push(name);
        }
    }
    if device_names.is_empty() {
        device_names.push("default".to_string());
    }
    Ok(device_names)
}

pub struct AudioProcessor {
    channels: u16,
}

impl AudioProcessor {
    pub fn new(_sample_rate: u32, channels: u16) -> Self {
        Self { channels }
    }

    pub fn mix_to_mono(&self, data: &[f32]) -> Vec<f32> {
        if self.channels == 1 {
            return data.to_vec();
        }
        data.chunks(self.channels as usize)
            .map(|frame| frame.iter().sum::<f32>() / self.channels as f32)
            .collect()
    }
}