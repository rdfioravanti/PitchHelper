use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

const BUFFER_CAPACITY: usize = 8192;

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

    pub fn start(&mut self) -> Result<(), String> {
        let host = cpal::default_host();
        let device = host.default_input_device().ok_or_else(|| {
            "No microphone was found. Connect an input device and try again.".to_owned()
        })?;
        let supported_config = device
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
                build_stream::<f32>(&device, &config, channels, &self.samples, |v| v)
            }
            cpal::SampleFormat::I16 => {
                build_stream::<i16>(&device, &config, channels, &self.samples, |v| {
                    v as f32 / i16::MAX as f32
                })
            }
            cpal::SampleFormat::U16 => {
                build_stream::<u16>(&device, &config, channels, &self.samples, |v| {
                    (v as f32 / u16::MAX as f32) * 2.0 - 1.0
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

fn build_stream<T>(
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
