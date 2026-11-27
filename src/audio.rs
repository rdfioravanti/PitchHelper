use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::pitch::{self, PitchReading};

const BUFFER_CAPACITY: usize = 8192;
const STOPPED: u8 = 0;
const STARTING: u8 = 1;
const RUNNING: u8 = 2;
const FAILED: u8 = 3;

#[derive(Clone, Copy, Default)]
pub struct AudioAnalysis {
    pub pitch: Option<PitchReading>,
    pub rms: f32,
    pub peak: f32,
}

pub fn measure_level(samples: &[f32]) -> (f32, f32) {
    let samples = &samples[samples.len().saturating_sub(4096)..];
    if samples.is_empty() {
        return (0.0, 0.0);
    }
    let rms =
        (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt();
    let peak = samples
        .iter()
        .map(|sample| sample.abs())
        .fold(0.0_f32, f32::max);
    (rms, peak)
}

pub struct InputDevice {
    pub name: String,
    pub is_default: bool,
    device: Arc<cpal::Device>,
}

pub struct Microphone {
    worker: Option<JoinHandle<()>>,
    stop_signal: Arc<AtomicBool>,
    status: Arc<AtomicU8>,
    samples: Arc<Mutex<VecDeque<f32>>>,
    analysis: Arc<Mutex<AudioAnalysis>>,
    sensitivity_threshold: Arc<AtomicU32>,
    last_error: Arc<Mutex<Option<String>>>,
}

impl Microphone {
    pub fn new() -> Self {
        Self {
            worker: None,
            stop_signal: Arc::new(AtomicBool::new(true)),
            status: Arc::new(AtomicU8::new(STOPPED)),
            samples: Arc::new(Mutex::new(VecDeque::with_capacity(BUFFER_CAPACITY))),
            analysis: Arc::new(Mutex::new(AudioAnalysis::default())),
            sensitivity_threshold: Arc::new(AtomicU32::new(
                pitch::sensitivity_threshold(80.0).to_bits(),
            )),
            last_error: Arc::new(Mutex::new(None)),
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
                device: Arc::new(device),
            });
        }
        Ok(inputs)
    }

    pub fn is_running(&self) -> bool {
        self.status.load(Ordering::Acquire) == RUNNING
    }

    pub fn is_starting(&self) -> bool {
        self.status.load(Ordering::Acquire) == STARTING
    }

    pub fn analysis(&self) -> Result<AudioAnalysis, String> {
        self.analysis
            .lock()
            .map(|analysis| *analysis)
            .map_err(|_| "The microphone analysis buffer is unavailable.".to_owned())
    }

    pub fn set_sensitivity(&self, sensitivity: f32) {
        self.sensitivity_threshold.store(
            pitch::sensitivity_threshold(sensitivity).to_bits(),
            Ordering::Relaxed,
        );
    }

    pub fn take_error(&self) -> Option<String> {
        self.last_error.lock().ok()?.take()
    }

    pub fn start(&mut self, input: &InputDevice) -> Result<(), String> {
        if self.is_running() || self.is_starting() {
            return Ok(());
        }
        self.stop();
        self.samples
            .lock()
            .map_err(|_| "The audio sample buffer is unavailable.".to_owned())?
            .clear();
        *self
            .analysis
            .lock()
            .map_err(|_| "The microphone analysis buffer is unavailable.".to_owned())? =
            AudioAnalysis::default();
        if let Ok(mut error) = self.last_error.lock() {
            *error = None;
        }

        let device = Arc::clone(&input.device);
        let samples = Arc::clone(&self.samples);
        let analysis = Arc::clone(&self.analysis);
        let threshold = Arc::clone(&self.sensitivity_threshold);
        let stop_signal = Arc::new(AtomicBool::new(false));
        let stop_worker = Arc::clone(&stop_signal);
        let status = Arc::new(AtomicU8::new(STARTING));
        let worker_status = Arc::clone(&status);
        let last_error = Arc::new(Mutex::new(None));
        let worker_error = Arc::clone(&last_error);

        let worker = thread::Builder::new()
            .name("pitchhelper-microphone".to_owned())
            .spawn(move || {
                let (stream, sample_rate) = match make_input_stream(&device, &samples) {
                    Ok(stream) => stream,
                    Err(error) => {
                        set_worker_error(&worker_error, error);
                        worker_status.store(FAILED, Ordering::Release);
                        return;
                    }
                };
                if let Err(error) = stream.play() {
                    set_worker_error(
                        &worker_error,
                        format!("Could not start the microphone: {error}"),
                    );
                    worker_status.store(FAILED, Ordering::Release);
                    return;
                }

                worker_status.store(RUNNING, Ordering::Release);
                while !stop_worker.load(Ordering::Acquire) {
                    let samples = match samples.lock() {
                        Ok(samples) => samples.iter().copied().collect::<Vec<_>>(),
                        Err(_) => {
                            set_worker_error(
                                &worker_error,
                                "The microphone sample buffer is unavailable.".to_owned(),
                            );
                            worker_status.store(FAILED, Ordering::Release);
                            break;
                        }
                    };
                    let (rms, peak) = measure_level(&samples);
                    let min_rms = f32::from_bits(threshold.load(Ordering::Relaxed));
                    let reading = pitch::detect_pitch(&samples, sample_rate, min_rms);
                    match analysis.lock() {
                        Ok(mut current) => {
                            *current = AudioAnalysis {
                                pitch: reading,
                                rms,
                                peak,
                            }
                        }
                        Err(_) => {
                            set_worker_error(
                                &worker_error,
                                "The microphone analysis buffer is unavailable.".to_owned(),
                            );
                            worker_status.store(FAILED, Ordering::Release);
                            break;
                        }
                    }
                    if !stop_worker.load(Ordering::Acquire) {
                        thread::park_timeout(Duration::from_millis(35));
                    }
                }
                drop(stream);
                worker_status.store(STOPPED, Ordering::Release);
            })
            .map_err(|error| format!("Could not start microphone capture: {error}"))?;

        self.stop_signal = stop_signal;
        self.status = status;
        self.last_error = last_error;
        self.worker = Some(worker);
        Ok(())
    }

    pub fn stop(&mut self) {
        self.stop_signal.store(true, Ordering::Release);
        self.status.store(STOPPED, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            if worker.join().is_err() {
                eprintln!("Microphone worker exited unexpectedly.");
            }
        }
        if let Ok(mut analysis) = self.analysis.lock() {
            *analysis = AudioAnalysis::default();
        }
    }
}

impl Drop for Microphone {
    fn drop(&mut self) {
        self.stop();
    }
}

fn set_worker_error(error_slot: &Mutex<Option<String>>, message: String) {
    match error_slot.lock() {
        Ok(mut error) => *error = Some(message),
        Err(_) => eprintln!("Microphone worker could not report an error: {message}"),
    }
}

fn make_input_stream(
    device: &cpal::Device,
    samples: &Arc<Mutex<VecDeque<f32>>>,
) -> Result<(cpal::Stream, u32), String> {
    let supported_config = device
        .default_input_config()
        .map_err(|error| format!("Could not read microphone settings: {error}"))?;
    let config: cpal::StreamConfig = supported_config.clone().into();
    let channels = config.channels as usize;
    let sample_rate = config.sample_rate.0;
    let stream = match supported_config.sample_format() {
        cpal::SampleFormat::F32 => {
            build_input_stream::<f32>(device, &config, channels, samples, |v| v)
        }
        cpal::SampleFormat::F64 => {
            build_input_stream::<f64>(device, &config, channels, samples, |v| v as f32)
        }
        cpal::SampleFormat::I8 => {
            build_input_stream::<i8>(device, &config, channels, samples, |v| {
                v as f32 / i8::MAX as f32
            })
        }
        cpal::SampleFormat::I16 => {
            build_input_stream::<i16>(device, &config, channels, samples, |v| {
                v as f32 / i16::MAX as f32
            })
        }
        cpal::SampleFormat::I32 => {
            build_input_stream::<i32>(device, &config, channels, samples, |v| {
                v as f32 / i32::MAX as f32
            })
        }
        cpal::SampleFormat::I64 => {
            build_input_stream::<i64>(device, &config, channels, samples, |v| {
                v as f32 / i64::MAX as f32
            })
        }
        cpal::SampleFormat::U8 => {
            build_input_stream::<u8>(device, &config, channels, samples, |v| {
                (v as f32 / u8::MAX as f32) * 2.0 - 1.0
            })
        }
        cpal::SampleFormat::U16 => {
            build_input_stream::<u16>(device, &config, channels, samples, |v| {
                (v as f32 / u16::MAX as f32) * 2.0 - 1.0
            })
        }
        cpal::SampleFormat::U32 => {
            build_input_stream::<u32>(device, &config, channels, samples, |v| {
                (v as f64 / u32::MAX as f64 * 2.0 - 1.0) as f32
            })
        }
        cpal::SampleFormat::U64 => {
            build_input_stream::<u64>(device, &config, channels, samples, |v| {
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
    Ok((stream, sample_rate))
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
    running: AtomicBool,
    reset_generation: AtomicU32,
    position: AtomicU32,
    bpm: AtomicU32,
    volume: AtomicU32,
}

impl Metronome {
    pub fn new(bpm: f32, volume: f32) -> Self {
        Self {
            stream: None,
            settings: Arc::new(MetronomeSettings {
                enabled: AtomicBool::new(false),
                running: AtomicBool::new(false),
                reset_generation: AtomicU32::new(0),
                position: AtomicU32::new(0.0_f32.to_bits()),
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

    pub fn set_running(&self, running: bool) {
        self.settings.running.store(running, Ordering::Relaxed);
    }

    pub fn reset_phase(&self) {
        self.set_position(0.0);
    }

    pub fn set_position(&self, beat_position: f64) {
        self.settings
            .position
            .store((beat_position.max(0.0) as f32).to_bits(), Ordering::Relaxed);
        self.settings
            .reset_generation
            .fetch_add(1, Ordering::Relaxed);
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
    let initial_position = f32::from_bits(settings.position.load(Ordering::Relaxed)).max(0.0);
    let mut beat_phase = initial_position.fract();
    let mut click_phase = 0.0_f32;
    let mut click_remaining = 0_usize;
    let mut click_length = (sample_rate as f32 * 0.035) as usize;
    let mut click_frequency = 1600.0_f32;
    let mut beat_index = initial_position.floor() as u64 + 1;
    let mut reset_generation = settings.reset_generation.load(Ordering::Relaxed);
    device.build_output_stream(
        config,
        move |data: &mut [T], _| {
            let generation = settings.reset_generation.load(Ordering::Relaxed);
            let reset = generation != reset_generation;
            if reset {
                reset_generation = generation;
                let position = f32::from_bits(settings.position.load(Ordering::Relaxed)).max(0.0);
                beat_phase = position.fract();
                click_phase = 0.0;
                click_remaining = 0;
                beat_index = position.floor() as u64 + 1;
            }
            let enabled = settings.enabled.load(Ordering::Relaxed)
                && settings.running.load(Ordering::Relaxed);
            let bpm = f32::from_bits(settings.bpm.load(Ordering::Relaxed)).clamp(40.0, 240.0);
            let volume = f32::from_bits(settings.volume.load(Ordering::Relaxed)).clamp(0.0, 1.0);
            if enabled && reset && beat_phase <= f32::EPSILON {
                click_length = (sample_rate as f32 * 0.035) as usize;
                click_remaining = click_length;
                click_phase = 0.0;
                let current_beat = beat_index.saturating_sub(1);
                click_frequency = if is_measure_start_beat(current_beat) {
                    1600.0
                } else {
                    1100.0
                };
            }
            for frame in data.chunks_mut(channels) {
                let mut value = 0.0;
                if enabled {
                    beat_phase += bpm / (60.0 * sample_rate as f32);
                    if beat_phase >= 1.0 {
                        beat_phase -= 1.0;
                        click_length = (sample_rate as f32 * 0.035) as usize;
                        click_remaining = click_length;
                        click_phase = 0.0;
                        click_frequency = if is_measure_start_beat(beat_index) {
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
                } else {
                    click_remaining = 0;
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

fn is_measure_start_beat(beat_position: u64) -> bool {
    beat_position.is_multiple_of(4)
}

#[cfg(test)]
mod tests {
    use super::{is_measure_start_beat, measure_level};

    #[test]
    fn measures_rms_and_peak_for_live_input_meter() {
        let (rms, peak) = measure_level(&[0.5, -0.5, 0.0, 0.0]);
        assert!((rms - 0.35355).abs() < 0.0001);
        assert_eq!(peak, 0.5);
        assert_eq!(measure_level(&[]), (0.0, 0.0));
    }

    #[test]
    fn metronome_accents_the_first_quarter_note_of_each_measure() {
        assert!(is_measure_start_beat(0));
        assert!(!is_measure_start_beat(1));
        assert!(!is_measure_start_beat(2));
        assert!(!is_measure_start_beat(3));
        assert!(is_measure_start_beat(4));
        assert!(is_measure_start_beat(8));
    }
}
