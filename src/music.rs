use std::fs::File;
use std::io::ErrorKind;
use std::path::Path;
use std::time::Duration;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

const WAVEFORM_BINS: usize = 1600;

pub struct TrackAnalysis {
    pub bpm: Option<f32>,
    pub duration: Duration,
    pub waveform: Vec<f32>,
}

pub fn analyze_track(path: &Path) -> Result<TrackAnalysis, String> {
    let file = File::open(path).map_err(|error| format!("Could not open audio file: {error}"))?;
    let source = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());
    let probed = symphonia::default::get_probe()
        .format(
            &Hint::new(),
            source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|error| format!("Could not read audio file: {error}"))?;
    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| "The audio file does not contain a playable track.".to_owned())?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|error| format!("Could not decode audio file: {error}"))?;

    let mut energy_levels = Vec::new();
    let mut sample_rate = None;
    let mut frames_per_window = None;
    let mut window_energy = 0.0_f64;
    let mut window_frames = 0_usize;
    let mut total_frames = 0_u64;
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(error)) if error.kind() == ErrorKind::UnexpectedEof => {
                break;
            }
            Err(error) => return Err(format!("Could not read audio data: {error}")),
        };
        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(error) => return Err(format!("Could not decode audio data: {error}")),
        };
        let channels = decoded.spec().channels.count();
        if channels == 0 {
            continue;
        }
        let packet_sample_rate = decoded.spec().rate;
        if packet_sample_rate == 0 {
            return Err("The audio file has an invalid sample rate.".to_owned());
        }
        sample_rate = Some(packet_sample_rate);
        let frames_per_window =
            *frames_per_window.get_or_insert((packet_sample_rate / 100).max(1) as usize);
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
        buffer.copy_interleaved_ref(decoded);
        for frame in buffer.samples().chunks(channels) {
            let mono = frame.iter().copied().sum::<f32>() / channels as f32;
            window_energy += f64::from(mono * mono);
            window_frames += 1;
            total_frames += 1;
            if window_frames == frames_per_window {
                energy_levels.push((window_energy / window_frames as f64).sqrt() as f32);
                window_energy = 0.0;
                window_frames = 0;
            }
        }
    }

    let sample_rate = sample_rate
        .ok_or_else(|| "The audio file did not contain decodable samples.".to_owned())?;
    if window_frames > 0 {
        energy_levels.push((window_energy / window_frames as f64).sqrt() as f32);
    }
    let duration = Duration::from_secs_f64(total_frames as f64 / sample_rate as f64);
    Ok(TrackAnalysis {
        bpm: detect_bpm(&energy_levels),
        duration,
        waveform: waveform_envelope(&energy_levels, WAVEFORM_BINS),
    })
}

fn waveform_envelope(samples: &[f32], bin_limit: usize) -> Vec<f32> {
    if samples.is_empty() || bin_limit == 0 {
        return Vec::new();
    }

    let bin_count = bin_limit.min(samples.len());
    let mut envelope = Vec::with_capacity(bin_count);
    for bin in 0..bin_count {
        let start = bin * samples.len() / bin_count;
        let end = (bin + 1) * samples.len() / bin_count;
        let rms = (samples[start..end]
            .iter()
            .map(|sample| sample * sample)
            .sum::<f32>()
            / (end - start) as f32)
            .sqrt();
        envelope.push(rms);
    }
    let peak = envelope.iter().copied().fold(0.0_f32, f32::max);
    if peak > 0.0 {
        for value in &mut envelope {
            *value /= peak;
        }
    }
    envelope
}

fn detect_bpm(energy: &[f32]) -> Option<f32> {
    if energy.len() < 200 {
        return None;
    }

    let mut onset = Vec::with_capacity(energy.len());
    let mut previous = energy[0];
    for level in energy.iter().copied() {
        onset.push((level - previous).max(0.0));
        previous = level;
    }
    let mean = onset.iter().sum::<f32>() / onset.len() as f32;
    for value in &mut onset {
        *value -= mean;
    }

    let min_lag = 25;
    let max_lag = 150.min(onset.len() / 2);
    let mut correlations = vec![0.0; max_lag + 1];
    for lag in min_lag..=max_lag {
        let length = onset.len() - lag;
        let cross = onset[..length]
            .iter()
            .zip(&onset[lag..])
            .map(|(left, right)| left * right)
            .sum::<f32>();
        let left_energy = onset[..length]
            .iter()
            .map(|value| value * value)
            .sum::<f32>();
        let right_energy = onset[lag..].iter().map(|value| value * value).sum::<f32>();
        let denominator = (left_energy * right_energy).sqrt();
        if denominator > 0.0 {
            correlations[lag] = cross / denominator;
        }
    }

    let mut best_lag = 0;
    let mut best_score = 0.0;
    for lag in min_lag..=max_lag {
        let mut score = correlations[lag];
        if lag * 2 <= max_lag {
            score += correlations[lag * 2] * 0.5;
        }
        if lag * 3 <= max_lag {
            score += correlations[lag * 3] * 0.25;
        }
        if score > best_score {
            best_score = score;
            best_lag = lag;
        }
    }
    if best_lag == 0 || best_score < 0.12 {
        return None;
    }

    let before = correlations[best_lag.saturating_sub(1)];
    let center = correlations[best_lag];
    let after = correlations[(best_lag + 1).min(max_lag)];
    let denominator = before - 2.0 * center + after;
    let adjustment = if denominator.abs() > f32::EPSILON {
        (0.5 * (before - after) / denominator).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    Some((60.0 * 100.0 / (best_lag as f32 + adjustment)).clamp(40.0, 240.0))
}

#[cfg(test)]
mod tests {
    use super::{detect_bpm, waveform_envelope};

    #[test]
    fn estimates_tempo_from_regular_transients() {
        let mut energy = vec![0.0; 1200];
        for beat in (0..energy.len()).step_by(50) {
            energy[beat] = 0.9;
        }

        let bpm = detect_bpm(&energy).expect("regular beat should be detected");
        assert!((bpm - 120.0).abs() < 3.0, "expected 120 BPM, got {bpm}");
    }

    #[test]
    fn waveform_envelope_is_normalized_and_limited_to_requested_bins() {
        let envelope = waveform_envelope(&[1.0, 1.0, 0.0, 0.0, -0.5, -0.5, 0.0, 0.0], 4);
        assert_eq!(envelope, vec![1.0, 0.0, 0.5, 0.0]);
        assert!(
            waveform_envelope(&[0.0; 20], 8)
                .iter()
                .all(|value| *value == 0.0)
        );
    }
}
