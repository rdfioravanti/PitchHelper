use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

const BUFFER_CAPACITY: usize = 8192;

pub struct InputDevice {
    pub name: String,
    pub is_default: bool,
    device: cpal::Device,
}

pub struct Microphone {
    stream: Option<cpal::Stream>,
    samples: Arc<Mutex<VecDeque<f32>>>,
    sample_rate: u32,
}

impl Microphone {
    pub fn new() -> Self {
        Self {
            stream: None,
            samples: Arc::new(Mutex::new(VecDeque::with_capacity(BUFFER_CAPACITY))),
            sample_rate: 0,
        }
    }

    pub fn available_devices() -> Result<Vec<InputDevice>, String> {
        let host = cpal::default_host();
        let default_name = host
            .default_input_device()
            .and_then(|device| device.name().ok());
        let devices = host
            .input_devices()
            .map_err(|error| format!("Could not list microphone devices: {error}"))?;

        let mut inputs = Vec::new();
        for device in devices {
            let name = device
                .name()
                .unwrap_or_else(|_| "Unnamed microphone".to_owned());
            let is_default = default_name.as_deref() == Some(name.as_str());
            inputs.push(InputDevice {
                name,
                is_default,
                device,
            });
        }
        Ok(inputs)
    }

    pub fn is_running(&self) -> bool {
        self.stream.is_some()
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn samples(&self) -> Option<Vec<f32>> {
        let samples = self.samples.lock().ok()?;
        Some(samples.iter().copied().collect())
    }

    pub fn start(&mut self, input: &InputDevice) -> Result<(), String> {
        let supported_config = input
            .device
            .default_input_config()
            .map_err(|error| format!("Could not read microphone settings: {error}"))?;
        let config: cpal::StreamConfig = supported_config.clone().into();
        let channels = config.channels as usize;
        self.sample_rate = config.sample_rate.0;
        self.samples
            .lock()
            .map_err(|_| "The audio sample buffer is unavailable.".to_owned())?
            .clear();

        let stream = match supported_config.sample_format() {
            cpal::SampleFormat::F32 => {
                build_input_stream::<f32>(&input.device, &config, channels, &self.samples, |v| v)
            }
            cpal::SampleFormat::F64 => {
                build_input_stream::<f64>(&input.device, &config, channels, &self.samples, |v| {
                    v as f32
                })
            }
            cpal::SampleFormat::I8 => {
                build_input_stream::<i8>(&input.device, &config, channels, &self.samples, |v| {
                    v as f32 / i8::MAX as f32
                })
            }
            cpal::SampleFormat::I16 => {
                build_input_stream::<i16>(&input.device, &config, channels, &self.samples, |v| {
                    v as f32 / i16::MAX as f32
                })
            }
            cpal::SampleFormat::I32 => {
                build_input_stream::<i32>(&input.device, &config, channels, &self.samples, |v| {
                    v as f32 / i32::MAX as f32
                })
            }
            cpal::SampleFormat::I64 => {
                build_input_stream::<i64>(&input.device, &config, channels, &self.samples, |v| {
                    v as f32 / i64::MAX as f32
                })
            }
            cpal::SampleFormat::U8 => {
                build_input_stream::<u8>(&input.device, &config, channels, &self.samples, |v| {
                    (v as f32 / u8::MAX as f32) * 2.0 - 1.0
                })
            }
            cpal::SampleFormat::U16 => {
                build_input_stream::<u16>(&input.device, &config, channels, &self.samples, |v| {
                    (v as f32 / u16::MAX as f32) * 2.0 - 1.0
                })
            }
            cpal::SampleFormat::U32 => {
                build_input_stream::<u32>(&input.device, &config, channels, &self.samples, |v| {
                    (v as f64 / u32::MAX as f64 * 2.0 - 1.0) as f32
                })
            }
            cpal::SampleFormat::U64 => {
                build_input_stream::<u64>(&input.device, &config, channels, &self.samples, |v| {
                    (v as f64 / u64::MAX as f64 * 2.0 - 1.0) as f32
                })
            }
            format => {
                return Err(format!(
                    "Microphone sample format {format:?} is not supported."
                ));
            }
        }
        .map_err(|error| format!("Could not open the microphone: {error}"))?;

        stream
            .play()
            .map_err(|error| format!("Could not start the microphone: {error}"))?;
        self.stream = Some(stream);
        Ok(())
    }

    pub fn stop(&mut self) {
        self.stream = None;
    }
}

fn build_input_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    buffer: &Arc<Mutex<VecDeque<f32>>>,
    convert: fn(T) -> f32,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample + Copy + 'static,
{
    let buffer = Arc::clone(buffer);
    device.build_input_stream(
        config,
        move |data: &[T], _| {
            let mono: Vec<f32> = data
                .chunks(channels)
                .map(|frame| {
                    frame.iter().map(|sample| convert(*sample)).sum::<f32>() / channels as f32
                })
                .collect();
            if let Ok(mut samples) = buffer.lock() {
                samples.extend(mono);
                let overflow = samples.len().saturating_sub(BUFFER_CAPACITY);
                if overflow > 0 {
                    samples.drain(..overflow);
                }
            }
        },
        |error| eprintln!("Microphone stream error: {error}"),
        None,
    )
}

pub struct Metronome {
    stream: Option<cpal::Stream>,
    settings: Arc<MetronomeSettings>,
}

struct MetronomeSettings {
    enabled: AtomicBool,
    bpm: AtomicU32,
    volume: AtomicU32,
}

impl Metronome {
    pub fn new(bpm: f32, volume: f32) -> Self {
        Self {
            stream: None,
            settings: Arc::new(MetronomeSettings {
                enabled: AtomicBool::new(false),
                bpm: AtomicU32::new(bpm.to_bits()),
                volume: AtomicU32::new(volume.to_bits()),
            }),
        }
    }

    pub fn set_bpm(&self, bpm: f32) {
        self.settings.bpm.store(bpm.to_bits(), Ordering::Relaxed);
    }

    pub fn set_volume(&self, volume: f32) {
        self.settings
            .volume
            .store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn set_enabled(&mut self, enabled: bool) -> Result<(), String> {
        if enabled && self.stream.is_none() {
            self.open_stream()?;
        }
        self.settings.enabled.store(enabled, Ordering::Relaxed);
        if !enabled {
            self.stream = None;
        }
        Ok(())
    }

    fn open_stream(&mut self) -> Result<(), String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "No output device is available for the metronome.".to_owned())?;
        let supported_config = device
            .default_output_config()
            .map_err(|error| format!("Could not read output settings: {error}"))?;
        let config: cpal::StreamConfig = supported_config.clone().into();
        let channels = config.channels as usize;
        let sample_rate = config.sample_rate.0;
        let stream = match supported_config.sample_format() {
            cpal::SampleFormat::F32 => build_metronome_stream::<f32>(
                &device,
                &config,
                channels,
                sample_rate,
                Arc::clone(&self.settings),
                |sample| sample,
            ),
            cpal::SampleFormat::F64 => build_metronome_stream::<f64>(
                &device,
                &config,
                channels,
                sample_rate,
                Arc::clone(&self.settings),
                |sample| sample as f64,
            ),
            cpal::SampleFormat::I8 => build_metronome_stream::<i8>(
                &device,
                &config,
                channels,
                sample_rate,
                Arc::clone(&self.settings),
                |sample| (sample.clamp(-1.0, 1.0) * i8::MAX as f32) as i8,
            ),
            cpal::SampleFormat::I16 => build_metronome_stream::<i16>(
                &device,
                &config,
                channels,
                sample_rate,
                Arc::clone(&self.settings),
                |sample| (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16,
            ),
            cpal::SampleFormat::I32 => build_metronome_stream::<i32>(
                &device,
                &config,
                channels,
                sample_rate,
                Arc::clone(&self.settings),
                |sample| (sample.clamp(-1.0, 1.0) * i32::MAX as f32) as i32,
            ),
            cpal::SampleFormat::I64 => build_metronome_stream::<i64>(
                &device,
                &config,
                channels,
                sample_rate,
                Arc::clone(&self.settings),
                |sample| (sample.clamp(-1.0, 1.0) as f64 * i64::MAX as f64) as i64,
            ),
            cpal::SampleFormat::U8 => build_metronome_stream::<u8>(
                &device,
                &config,
                channels,
                sample_rate,
                Arc::clone(&self.settings),
                |sample| ((sample.clamp(-1.0, 1.0) + 1.0) * 0.5 * u8::MAX as f32) as u8,
            ),
            cpal::SampleFormat::U16 => build_metronome_stream::<u16>(
                &device,
                &config,
                channels,
                sample_rate,
                Arc::clone(&self.settings),
                |sample| ((sample.clamp(-1.0, 1.0) + 1.0) * 0.5 * u16::MAX as f32) as u16,
            ),
            cpal::SampleFormat::U32 => build_metronome_stream::<u32>(
                &device,
                &config,
                channels,
                sample_rate,
                Arc::clone(&self.settings),
                |sample| ((sample.clamp(-1.0, 1.0) as f64 + 1.0) * 0.5 * u32::MAX as f64) as u32,
            ),
            cpal::SampleFormat::U64 => build_metronome_stream::<u64>(
                &device,
                &config,
                channels,
                sample_rate,
                Arc::clone(&self.settings),
                |sample| ((sample.clamp(-1.0, 1.0) as f64 + 1.0) * 0.5 * u64::MAX as f64) as u64,
            ),
            format => {
                return Err(format!("Output sample format {format:?} is not supported."));
            }
        }
        .map_err(|error| format!("Could not start metronome output: {error}"))?;
        stream
            .play()
            .map_err(|error| format!("Could not play metronome: {error}"))?;
        self.stream = Some(stream);
        Ok(())
    }
}

fn build_metronome_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    sample_rate: u32,
    settings: Arc<MetronomeSettings>,
    convert: fn(f32) -> T,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample + Copy + 'static,
{
    let mut beat_phase = 0.0_f32;
    let mut click_phase = 0.0_f32;
    let mut click_remaining = 0_usize;
    let mut click_length = (sample_rate as f32 * 0.035) as usize;
    let mut click_frequency = 1600.0_f32;
    let mut beat_index = 0_u64;
    device.build_output_stream(
        config,
        move |data: &mut [T], _| {
            let enabled = settings.enabled.load(Ordering::Relaxed);
            let bpm = f32::from_bits(settings.bpm.load(Ordering::Relaxed)).clamp(40.0, 240.0);
            let volume = f32::from_bits(settings.volume.load(Ordering::Relaxed)).clamp(0.0, 1.0);
            for frame in data.chunks_mut(channels) {
                let mut value = 0.0;
                if enabled {
                    beat_phase += bpm / (60.0 * sample_rate as f32);
                    if beat_phase >= 1.0 {
                        beat_phase -= 1.0;
                        click_length = (sample_rate as f32 * 0.035) as usize;
                        click_remaining = click_length;
                        click_phase = 0.0;
                        click_frequency = if beat_index.is_multiple_of(4) {
                            1600.0
                        } else {
                            1100.0
                        };
                        beat_index += 1;
                    }
                    if click_remaining > 0 && click_length > 0 {
                        let envelope = click_remaining as f32 / click_length as f32;
                        value = (click_phase * std::f32::consts::TAU).sin() * envelope * volume;
                        click_phase += click_frequency / sample_rate as f32;
                        click_remaining -= 1;
                    }
                }
                for sample in frame {
                    *sample = convert(value);
                }
            }
        },
        |error| eprintln!("Metronome output error: {error}"),
        None,
    )
}
