use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Host, SampleFormat, SampleRate, StreamConfig, SupportedStreamConfig};
use log::{info, error};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

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
        let host = cpal::default_host();
        let device = find_audio_device(&host, device_name)?;
        
        let config = device.default_input_config()?;
        let sample_rate = config.sample_rate().0;
        let channels = config.channels() as u16;

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
        let config = self.device.default_input_config()?;
        
        let is_recording = Arc::clone(&self.is_recording);
        let audio_data = Arc::clone(&self.audio_data);
        
        *is_recording.lock().unwrap() = true;

        let stream = match config.sample_format() {
            SampleFormat::F32 => self.build_stream::<f32>(&config)?,
            SampleFormat::I16 => self.build_stream::<i16>(&config)?,
            SampleFormat::U16 => self.build_stream::<u16>(&config)?,
            _ => return Err(anyhow::anyhow!("Unsupported sample format")),
        };

        stream.play()?;
        self.stream = Some(stream);
        
        info!("Audio recording started");
        Ok(())
    }

    fn build_stream<T>(&self, config: &SupportedStreamConfig) -> Result<cpal::Stream>
    where
        T: cpal::Sample + cpal::SizedSample + Into<f32>,
    {
        let is_recording = Arc::clone(&self.is_recording);
        let audio_data = Arc::clone(&self.audio_data);
        
        let err_fn = move |err| {
            error!("Audio stream error: {}", err);
        };

        let stream = self.device.build_input_stream(
            &config.clone().into(),
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                if *is_recording.lock().unwrap() {
                    let mut audio_buffer = audio_data.lock().unwrap();
                    for sample in data {
                        audio_buffer.push(sample.to_owned().into());
                    }
                }
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
    let devices = host.input_devices()?;
    
    // First try exact match
    for device in devices {
        if let Ok(name) = device.name() {
            if name == device_name {
                return Ok(device);
            }
        }
    }
    
    // Then try contains match
    for device in devices {
        if let Ok(name) = device.name() {
            if name.contains(device_name) {
                return Ok(device);
            }
        }
    }
    
    // Fall back to default device
    host.default_input_device()
        .ok_or_else(|| anyhow::anyhow!("No input device available"))
}

pub fn get_available_devices() -> Result<Vec<String>> {
    let host = cpal::default_host();
    let devices = host.input_devices()?;
    
    let mut device_names = Vec::new();
    
    for device in devices {
        if let Ok(name) = device.name() {
            device_names.push(name);
        }
    }
    
    // Add default device if no devices found
    if device_names.is_empty() {
        device_names.push("Default Device".to_string());
    }
    
    Ok(device_names)
}

// Audio processing utilities
pub struct AudioProcessor {
    sample_rate: u32,
    channels: u16,
}

impl AudioProcessor {
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        Self { sample_rate, channels }
    }

    pub fn normalize_audio(&self, data: &[f32]) -> Vec<i16> {
        data.iter()
            .map(|&sample| {
                let normalized = (sample * i16::MAX as f32).clamp(i16::MIN as f32, i16::MAX as f32);
                normalized as i16
            })
            .collect()
    }

    pub fn apply_gain(&self, data: &mut Vec<f32>, gain: f32) {
        for sample in data {
            *sample = (*sample * gain).clamp(-1.0, 1.0);
        }
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

// Audio format conversion
pub fn convert_to_wav(data: &[f32], sample_rate: u32, channels: u16) -> Vec<u8> {
    use hound::{WavSpec, WavWriter};
    use std::io::Cursor;

    let spec = WavSpec {
        channels: channels as u16,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = WavWriter::new(&mut cursor, spec).unwrap();
        for &sample in data {
            writer.write_sample((sample * i16::MAX as f32) as i16).unwrap();
        }
        writer.finalize().unwrap();
    }

    cursor.into_inner()
}

// Add hound dependency for WAV support
// In Cargo.toml: hound = "3.5"