use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    Major,
    Minor,
    MajorPentatonic,
    MinorPentatonic,
}

impl Scale {
    pub fn label(self) -> &'static str {
        match self {
            Self::Major => "Major",
            Self::Minor => "Minor",
            Self::MajorPentatonic => "Major Pentatonic",
            Self::MinorPentatonic => "Minor Pentatonic",
        }
    }

    pub fn intervals(self) -> &'static [i32] {
        match self {
            Self::Major => &[0, 2, 4, 5, 7, 9, 11],
            Self::Minor => &[0, 2, 3, 5, 7, 8, 10],
            Self::MajorPentatonic => &[0, 2, 4, 7, 9],
            Self::MinorPentatonic => &[0, 3, 5, 7, 10],
        }
    }

    pub fn chord_intervals(self) -> &'static [i32] {
        match self {
            Self::Major | Self::MajorPentatonic => &[0, 4, 7],
            Self::Minor | Self::MinorPentatonic => &[0, 3, 7],
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WarmupPattern {
    FiveTone,
    Octave,
    Arpeggio,
    Siren,
}

impl WarmupPattern {
    pub fn label(self) -> &'static str {
        match self {
            Self::FiveTone => "5-Tone Scale",
            Self::Octave => "Octaves",
            Self::Arpeggio => "Arpeggio",
            Self::Siren => "Sirens",
        }
    }

    pub fn intervals(self) -> &'static [i32] {
        match self {
            Self::FiveTone => &[0, 2, 4, 5, 7, 5, 4, 2, 0],
            Self::Octave => &[0, 12, 0, 12, 0],
            Self::Arpeggio => &[0, 4, 7, 12, 7, 4, 0],
            Self::Siren => &[0, 7, 12, 19, 12, 7, 0],
        }
    }
}

#[derive(Clone, Debug)]
pub struct PitchMetrics {
    pub width_semitones: f32,
    pub vibrato_hz: Option<f32>,
    pub steady: bool,
    pub sample_span: Duration,
}

pub fn pitch_metrics(samples: &[(Instant, f32)], now: Instant) -> Option<PitchMetrics> {
    let cutoff = now.checked_sub(Duration::from_millis(900))?;
    let pitches: Vec<f32> = samples
        .iter()
        .filter_map(|(time, midi)| (*time >= cutoff).then_some(*midi))
        .collect();
    if pitches.len() < 5 {
        return None;
    }

    let mean = pitches.iter().sum::<f32>() / pitches.len() as f32;
    let mut centered = pitches.clone();
    centered.sort_by(f32::total_cmp);
    let low = centered[(centered.len() as f32 * 0.1) as usize];
    let high = centered[((centered.len() as f32 * 0.9) as usize).min(centered.len() - 1)];
    let width_semitones = high - low;

    let mut crossings = 0;
    let mut previous_sign = 0_i8;
    for pitch in pitches {
        let sign = if pitch > mean + 0.025 {
            1
        } else if pitch < mean - 0.025 {
            -1
        } else {
            0
        };
        if sign != 0 && previous_sign != 0 && sign != previous_sign {
            crossings += 1;
        }
        if sign != 0 {
            previous_sign = sign;
        }
    }
    let mut sample_times = samples
        .iter()
        .filter(|(time, _)| *time >= cutoff)
        .map(|(time, _)| *time);
    let sample_span = sample_times
        .next()
        .zip(sample_times.next_back())
        .map(|(start, end)| end.duration_since(start))
        .unwrap_or_default();
    let span = sample_span.as_secs_f32();
    let vibrato_hz = (crossings >= 4 && span > 0.2).then_some(crossings as f32 / (2.0 * span));
    Some(PitchMetrics {
        width_semitones,
        vibrato_hz,
        steady: width_semitones <= 0.25,
        sample_span,
    })
}

#[derive(Clone, Debug)]
pub struct RangeTracker {
    pub low: Option<i32>,
    pub high: Option<i32>,
    pub stable_counts: Vec<u32>,
}

impl Default for RangeTracker {
    fn default() -> Self {
        Self {
            low: None,
            high: None,
            stable_counts: vec![0; 109],
        }
    }
}

impl RangeTracker {
    pub fn record(&mut self, midi: f32, steady: bool) -> bool {
        let note = midi.round() as i32;
        if !(12..=120).contains(&note) {
            return false;
        }
        self.low = Some(self.low.map_or(note, |low| low.min(note)));
        self.high = Some(self.high.map_or(note, |high| high.max(note)));
        if steady {
            self.stable_counts[(note - 12) as usize] =
                self.stable_counts[(note - 12) as usize].saturating_add(1);
        }
        true
    }

    pub fn tessitura(&self) -> Option<(i32, i32)> {
        let total: u64 = self.stable_counts.iter().map(|count| *count as u64).sum();
        if total == 0 {
            return None;
        }
        let lower_rank = (total as f64 * 0.1).floor() as u64;
        let upper_rank = (total as f64 * 0.9).ceil() as u64;
        let mut cumulative = 0_u64;
        let mut low = None;
        let mut high = None;
        for (index, count) in self.stable_counts.iter().enumerate() {
            cumulative += *count as u64;
            if low.is_none() && cumulative > lower_rank {
                low = Some(index as i32 + 12);
            }
            if cumulative >= upper_rank {
                high = Some(index as i32 + 12);
                break;
            }
        }
        low.zip(high)
    }

    pub fn load() -> Result<Self, String> {
        let path = range_file_path().ok_or_else(|| {
            "Could not locate a user data directory for range history.".to_owned()
        })?;
        let contents = match fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(format!("Could not read vocal range history: {error}")),
        };
        let mut lines = contents.lines();
        let Some((low, high)) = lines.next().and_then(|line| {
            let mut values = line.split(',');
            Some((values.next()?.parse().ok()?, values.next()?.parse().ok()?))
        }) else {
            return Err("Vocal range history has an invalid format.".to_owned());
        };
        if (low != 0 && !(12..=120).contains(&low)) || (high != 0 && !(12..=120).contains(&high)) {
            return Err("Vocal range history contains an invalid note range.".to_owned());
        }
        if (low == 0) != (high == 0) || (low != 0 && low > high) {
            return Err("Vocal range history contains an inconsistent note range.".to_owned());
        }
        let mut tracker = Self {
            low: (low != 0).then_some(low),
            high: (high != 0).then_some(high),
            ..Self::default()
        };
        let counts = lines
            .next()
            .ok_or_else(|| "Vocal range history is missing pitch counts.".to_owned())?;
        let parsed_counts = counts
            .split(',')
            .map(str::parse::<u32>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "Vocal range history contains invalid pitch counts.".to_owned())?;
        if parsed_counts.len() != tracker.stable_counts.len() || lines.next().is_some() {
            return Err("Vocal range history has an invalid number of pitch counts.".to_owned());
        }
        tracker.stable_counts = parsed_counts;
        Ok(tracker)
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = range_file_path()
            .ok_or_else(|| std::io::Error::other("Could not locate a user data directory."))?;
        let Some(parent) = path.parent() else {
            return Err(std::io::Error::other(
                "The range history path has no parent directory.",
            ));
        };
        fs::create_dir_all(parent)?;
        let low = self.low.unwrap_or(0);
        let high = self.high.unwrap_or(0);
        let counts = self
            .stable_counts
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let temporary = path.with_extension("tmp");
        fs::write(&temporary, format!("{low},{high}\n{counts}\n"))?;
        if let Err(error) = fs::rename(&temporary, path) {
            let _ = fs::remove_file(temporary);
            return Err(error);
        }
        Ok(())
    }

    pub fn clear_saved() -> std::io::Result<()> {
        let path = range_file_path()
            .ok_or_else(|| std::io::Error::other("Could not locate a user data directory."))?;
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

fn range_file_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let root = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(not(target_os = "windows"))]
    let root = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("share"))
        });
    root.map(|root| root.join("PitchHelper").join("range.dat"))
}

#[cfg(test)]
mod tests {
    use super::{RangeTracker, Scale, pitch_metrics};
    use std::time::{Duration, Instant};

    #[test]
    fn scales_and_chord_tones_have_expected_intervals() {
        assert_eq!(Scale::Major.intervals(), &[0, 2, 4, 5, 7, 9, 11]);
        assert_eq!(Scale::Minor.chord_intervals(), &[0, 3, 7]);
        assert!(!Scale::MajorPentatonic.intervals().contains(&5));
    }

    #[test]
    fn range_tracks_detected_notes_but_tessitura_uses_only_steady_notes() {
        let mut tracker = RangeTracker::default();
        assert!(tracker.record(40.0, false));
        assert_eq!((tracker.low, tracker.high), (Some(40), Some(40)));
        assert_eq!(tracker.tessitura(), None);
        tracker.record(48.0, true);
        tracker.record(60.0, true);
        tracker.record(60.0, true);
        tracker.record(72.0, true);
        assert_eq!((tracker.low, tracker.high), (Some(40), Some(72)));
        assert_eq!(tracker.tessitura(), Some((48, 72)));
    }

    #[test]
    fn tessitura_estimate_can_shift_lower_with_new_steady_samples() {
        let mut tracker = RangeTracker::default();
        for _ in 0..10 {
            tracker.record(72.0, true);
        }
        assert_eq!(tracker.tessitura(), Some((72, 72)));

        for _ in 0..20 {
            tracker.record(48.0, true);
        }
        assert_eq!(tracker.tessitura(), Some((48, 72)));
    }

    #[test]
    fn pitch_metrics_distinguish_steady_tone_from_vibrato() {
        let now = Instant::now();
        let steady_samples: Vec<_> = (0..30)
            .map(|index| {
                let elapsed = Duration::from_millis(index * 30);
                (now - Duration::from_millis(900) + elapsed, 60.0)
            })
            .collect();
        let steady = pitch_metrics(&steady_samples, now).expect("steady samples are sufficient");
        assert!(steady.steady);
        assert!(steady.vibrato_hz.is_none());

        let vibrato_samples: Vec<_> = (0..30)
            .map(|index| {
                let elapsed = index as f32 * 0.03;
                let time = now - Duration::from_millis(900) + Duration::from_secs_f32(elapsed);
                let pitch = 60.0 + 0.25 * (std::f32::consts::TAU * 5.0 * elapsed).sin();
                (time, pitch)
            })
            .collect();
        let vibrato = pitch_metrics(&vibrato_samples, now).expect("vibrato samples are sufficient");
        assert!(!vibrato.steady);
        assert!(
            vibrato
                .vibrato_hz
                .is_some_and(|rate| (3.0..=9.0).contains(&rate))
        );
    }
}
