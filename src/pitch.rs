const NOTE_NAMES: [&str; 12] = [
    "C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B",
];
const MIN_FREQUENCY: f32 = 65.0;
const MAX_FREQUENCY: f32 = 1000.0;

#[derive(Clone, Copy, Debug)]
pub struct PitchReading {
    pub frequency: f32,
    pub midi: f32,
}

pub fn detect_pitch(samples: &[f32], sample_rate: u32) -> Option<PitchReading> {
    if samples.len() < 2048 || sample_rate == 0 {
        return None;
    }

    let samples = &samples[samples.len().saturating_sub(4096)..];
    let mean = samples.iter().sum::<f32>() / samples.len() as f32;
    let signal: Vec<f32> = samples.iter().map(|sample| sample - mean).collect();
    let rms =
        (signal.iter().map(|sample| sample * sample).sum::<f32>() / signal.len() as f32).sqrt();
    if rms < 0.008 {
        return None;
    }

    let min_lag = (sample_rate as f32 / MAX_FREQUENCY).floor() as usize;
    let max_lag = ((sample_rate as f32 / MIN_FREQUENCY).ceil() as usize).min(signal.len() / 2);
    let mut correlations = vec![0.0; max_lag + 1];
    for lag in min_lag..=max_lag {
        let length = signal.len() - lag;
        let mut cross = 0.0;
        let mut left_energy = 0.0;
        let mut right_energy = 0.0;
        for index in 0..length {
            let left = signal[index];
            let right = signal[index + lag];
            cross += left * right;
            left_energy += left * left;
            right_energy += right * right;
        }
        let energy = (left_energy * right_energy).sqrt();
        correlations[lag] = if energy > 0.0 { cross / energy } else { 0.0 };
    }

    let mut best_lag = 0;
    for lag in min_lag..=max_lag {
        let correlation = correlations[lag];
        if lag > min_lag
            && correlation > 0.75
            && correlation >= correlations[lag.saturating_sub(1)]
            && correlation >= correlations[(lag + 1).min(max_lag)]
        {
            best_lag = lag;
            break;
        }
    }
    if best_lag == 0 {
        return None;
    }

    let before = correlations[best_lag - 1];
    let center = correlations[best_lag];
    let after = correlations[(best_lag + 1).min(max_lag)];
    let denominator = before - 2.0 * center + after;
    let adjustment = if denominator.abs() > f32::EPSILON {
        (0.5 * (before - after) / denominator).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    let frequency = sample_rate as f32 / (best_lag as f32 + adjustment);
    let midi = 69.0 + 12.0 * (frequency / 440.0).log2();
    Some(PitchReading { frequency, midi })
}

pub fn midi_to_note(midi: f32) -> (i32, String, f32) {
    let nearest_midi = midi.round() as i32;
    let pitch_class = nearest_midi.rem_euclid(12) as usize;
    let octave = nearest_midi.div_euclid(12) - 1;
    let cents = (midi - nearest_midi as f32) * 100.0;
    (
        nearest_midi,
        format!("{}{}", NOTE_NAMES[pitch_class], octave),
        cents,
    )
}

pub fn enharmonic_name(midi: f32) -> Option<String> {
    let nearest_midi = midi.round() as i32;
    let pitch_class = nearest_midi.rem_euclid(12) as usize;
    let flat = match pitch_class {
        1 => "D♭",
        3 => "E♭",
        6 => "G♭",
        8 => "A♭",
        10 => "B♭",
        _ => return None,
    };
    Some(format!("{flat}{}", nearest_midi.div_euclid(12) - 1))
}

#[cfg(test)]
mod tests {
    use super::{detect_pitch, enharmonic_name, midi_to_note};

    #[test]
    fn identifies_a_sine_wave_and_tuning_offset() {
        let sample_rate = 44_100;
        let frequency = 440.0 * 2.0_f32.powf(0.18 / 12.0);
        let samples: Vec<f32> = (0..4096)
            .map(|index| {
                (std::f32::consts::TAU * frequency * index as f32 / sample_rate as f32).sin()
            })
            .collect();

        let reading = detect_pitch(&samples, sample_rate).expect("sine wave should be detected");
        let (_, note, cents) = midi_to_note(reading.midi);
        assert_eq!(note, "A4");
        assert!(
            (cents - 18.0).abs() < 4.0,
            "expected about 18 cents, got {cents}"
        );
    }

    #[test]
    fn detects_low_singer_range_without_subharmonic_octave_errors() {
        let sample_rate = 44_100;
        let frequency = 82.41;
        let samples: Vec<f32> = (0..4096)
            .map(|index| {
                (std::f32::consts::TAU * frequency * index as f32 / sample_rate as f32).sin()
            })
            .collect();

        let reading = detect_pitch(&samples, sample_rate).expect("low note should be detected");
        let (_, note, _) = midi_to_note(reading.midi);
        assert_eq!(note, "E2");
    }

    #[test]
    fn ignores_silence_and_short_buffers() {
        assert!(detect_pitch(&[0.0; 4096], 44_100).is_none());
        assert!(detect_pitch(&[0.1; 100], 44_100).is_none());
    }

    #[test]
    fn names_notes_using_sharps_and_flats_from_pitch_direction() {
        let (_, note, cents) = midi_to_note(60.0 - 0.2);
        assert_eq!(note, "C4");
        assert!(cents < 0.0);

        let (_, note, cents) = midi_to_note(61.0 + 0.2);
        assert_eq!(note, "C♯4");
        assert!(cents > 0.0);
        assert_eq!(enharmonic_name(61.0), Some("D♭4".to_owned()));
    }
}
