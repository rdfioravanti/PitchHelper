use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rodio::buffer::SamplesBuffer;
use rodio::source::SineWave;
use rodio::{Decoder, OutputStream, Sink, Source};

const PRACTICE_SAMPLE_RATE: u32 = 44_100;
const NOTE_EDGE_FADE: Duration = Duration::from_millis(8);
const STOP_FADE_DURATION: Duration = Duration::from_millis(24);

pub struct SongPlayer {
    output: Option<OutputStream>,
    sink: Option<Sink>,
    volume: f32,
    seek_position: Duration,
}

impl SongPlayer {
    pub fn new() -> Self {
        Self {
            output: None,
            sink: None,
            volume: 0.5,
            seek_position: Duration::ZERO,
        }
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
        if let Some(sink) = &self.sink {
            sink.set_volume(self.volume);
        }
    }

    pub fn toggle(&mut self, path: &Path) -> Result<(), String> {
        if let Some(sink) = &self.sink {
            if sink.is_paused() {
                sink.play();
                return Ok(());
            }
            if !sink.empty() {
                sink.pause();
                return Ok(());
            }
        }

        let start_position = if self.sink.as_ref().is_some_and(Sink::empty) {
            Duration::ZERO
        } else {
            self.position()
        };
        self.clear_output();
        let file =
            File::open(path).map_err(|error| format!("Could not open audio file: {error}"))?;
        let source = Decoder::new(BufReader::new(file))
            .map_err(|error| format!("Could not decode audio file: {error}"))?;
        let (output, handle) = OutputStream::try_default()
            .map_err(|error| format!("Could not open audio output: {error}"))?;
        let sink = Sink::try_new(&handle)
            .map_err(|error| format!("Could not create audio player: {error}"))?;
        sink.set_volume(self.volume);
        sink.append(source);
        if !start_position.is_zero() {
            sink.try_seek(start_position)
                .map_err(|error| format!("Could not seek in audio track: {error}"))?;
        }
        sink.play();
        self.output = Some(output);
        self.sink = Some(sink);
        self.seek_position = start_position;
        Ok(())
    }

    pub fn stop(&mut self) {
        self.clear_output();
        self.seek_position = Duration::ZERO;
    }

    fn clear_output(&mut self) {
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }
        self.output = None;
    }

    pub fn is_playing(&self) -> bool {
        self.sink
            .as_ref()
            .map(|sink| !sink.is_paused() && !sink.empty())
            .unwrap_or(false)
    }

    pub fn is_paused(&self) -> bool {
        self.sink
            .as_ref()
            .map(|sink| sink.is_paused() && !sink.empty())
            .unwrap_or(false)
    }

    pub fn is_finished(&self) -> bool {
        self.sink.as_ref().is_some_and(Sink::empty)
    }

    pub fn seek(&mut self, position: Duration) -> Result<(), String> {
        if let Some(sink) = &self.sink {
            if sink.empty() {
                self.clear_output();
            } else {
                sink.try_seek(position)
                    .map_err(|error| format!("Could not seek in audio track: {error}"))?;
            }
        }
        self.seek_position = position;
        Ok(())
    }

    pub fn position(&self) -> Duration {
        self.sink
            .as_ref()
            .map(Sink::get_pos)
            .unwrap_or(self.seek_position)
    }
}

pub struct TonePlayer {
    output: Option<OutputStream>,
    sink: Option<Sink>,
    stop_signal: Option<Arc<AtomicBool>>,
}

impl TonePlayer {
    pub fn new() -> Self {
        Self {
            output: None,
            sink: None,
            stop_signal: None,
        }
    }

    pub fn play_drone(&mut self, midi: i32, volume: f32) -> Result<(), String> {
        self.start_output(volume)?;
        let source = SineWave::new(midi_frequency(midi)).repeat_infinite();
        let (source, stop_signal) = FadeOut::new(source, STOP_FADE_DURATION);
        self.append_source(source)?;
        self.stop_signal = Some(stop_signal);
        Ok(())
    }

    pub fn play_notes(
        &mut self,
        notes: &[i32],
        note_duration: Duration,
        volume: f32,
    ) -> Result<(), String> {
        if notes.is_empty() {
            return Err("The practice routine has no notes to play.".to_owned());
        }
        if note_duration.is_zero() {
            return Err("Practice notes must have a positive duration.".to_owned());
        }
        self.start_output(volume)?;
        let samples = SamplesBuffer::new(
            1,
            PRACTICE_SAMPLE_RATE,
            practice_samples(notes, note_duration, PRACTICE_SAMPLE_RATE),
        );
        let (source, stop_signal) = FadeOut::new(samples, STOP_FADE_DURATION);
        self.append_source(source)?;
        self.stop_signal = Some(stop_signal);
        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(stop_signal) = &self.stop_signal {
            stop_signal.store(true, Ordering::Relaxed);
        }
    }

    pub fn is_playing(&self) -> bool {
        self.sink.as_ref().is_some_and(|sink| !sink.empty())
    }

    fn start_output(&mut self, volume: f32) -> Result<(), String> {
        if let Some(stop_signal) = &self.stop_signal {
            stop_signal.store(true, Ordering::Relaxed);
        }
        let volume = volume.clamp(0.0, 1.0);
        if let Some(sink) = &self.sink {
            sink.set_volume(volume);
            return Ok(());
        }
        let (output, handle) = OutputStream::try_default()
            .map_err(|error| format!("Could not open audio output: {error}"))?;
        let sink = Sink::try_new(&handle)
            .map_err(|error| format!("Could not create practice audio player: {error}"))?;
        sink.set_volume(volume);
        sink.play();
        self.output = Some(output);
        self.sink = Some(sink);
        Ok(())
    }

    fn append_source<S>(&self, source: S) -> Result<(), String>
    where
        S: Source<Item = f32> + Send + 'static,
    {
        let Some(sink) = &self.sink else {
            return Err("Could not create practice audio player.".to_owned());
        };
        sink.append(source);
        Ok(())
    }
}

struct FadeOut<S> {
    source: S,
    stop_signal: Arc<AtomicBool>,
    fade_frames: usize,
    fade_remaining: Option<usize>,
}

impl<S> FadeOut<S>
where
    S: Source<Item = f32>,
{
    fn new(source: S, duration: Duration) -> (Self, Arc<AtomicBool>) {
        let stop_signal = Arc::new(AtomicBool::new(false));
        let fade_frames = (duration.as_secs_f64() * source.sample_rate() as f64).round() as usize;
        (
            Self {
                source,
                stop_signal: Arc::clone(&stop_signal),
                fade_frames: fade_frames.max(1),
                fade_remaining: None,
            },
            stop_signal,
        )
    }
}

impl<S> Iterator for FadeOut<S>
where
    S: Source<Item = f32>,
{
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        if self.fade_remaining == Some(0) {
            return None;
        }
        let sample = self.source.next()?;
        if self.stop_signal.load(Ordering::Relaxed) {
            let remaining = *self.fade_remaining.get_or_insert(self.fade_frames);
            let gain = (remaining.saturating_sub(1) as f32
                / self.fade_frames.saturating_sub(1).max(1) as f32)
                .clamp(0.0, 1.0);
            self.fade_remaining = Some(remaining - 1);
            Some(sample * gain)
        } else {
            Some(sample)
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, None)
    }
}

impl<S> Source for FadeOut<S>
where
    S: Source<Item = f32>,
{
    fn current_frame_len(&self) -> Option<usize> {
        self.source.current_frame_len()
    }

    fn channels(&self) -> u16 {
        self.source.channels()
    }

    fn sample_rate(&self) -> u32 {
        self.source.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.source.total_duration()
    }
}

fn midi_frequency(midi: i32) -> f32 {
    440.0 * 2.0_f32.powf((midi as f32 - 69.0) / 12.0)
}

fn practice_samples(notes: &[i32], note_duration: Duration, sample_rate: u32) -> Vec<f32> {
    let frames_per_note = (note_duration.as_secs_f64() * sample_rate as f64).round() as usize;
    let fade_frames = ((NOTE_EDGE_FADE.as_secs_f64() * sample_rate as f64).round() as usize)
        .min(frames_per_note / 2);
    let mut samples = Vec::with_capacity(frames_per_note.saturating_mul(notes.len()));
    let mut phase = 0.0_f64;
    let tau = std::f64::consts::TAU;

    for note in notes {
        let phase_step = tau * midi_frequency(*note) as f64 / sample_rate as f64;
        for frame in 0..frames_per_note {
            let attack = if frame < fade_frames {
                0.5 - 0.5 * (std::f32::consts::PI * frame as f32 / fade_frames as f32).cos()
            } else {
                1.0
            };
            let frames_to_end = frames_per_note - 1 - frame;
            let release = if frames_to_end < fade_frames {
                0.5 - 0.5 * (std::f32::consts::PI * frames_to_end as f32 / fade_frames as f32).cos()
            } else {
                1.0
            };
            samples.push(phase.sin() as f32 * attack.min(release));
            phase = (phase + phase_step).rem_euclid(tau);
        }
    }

    samples
}

#[cfg(test)]
mod tests {
    use super::{FadeOut, NOTE_EDGE_FADE, PRACTICE_SAMPLE_RATE, practice_samples};
    use rodio::buffer::SamplesBuffer;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    #[test]
    fn practice_notes_fade_to_silence_at_each_note_boundary() {
        let note_duration = Duration::from_millis(450);
        let frames_per_note =
            (note_duration.as_secs_f64() * PRACTICE_SAMPLE_RATE as f64).round() as usize;
        let samples = practice_samples(&[60, 67], note_duration, PRACTICE_SAMPLE_RATE);

        assert_eq!(samples.len(), frames_per_note * 2);
        assert!(samples[0].abs() < 0.001);
        assert!(samples[samples.len() - 1].abs() < 0.001);
        assert!(samples[frames_per_note - 1].abs() < 0.001);
        assert!(samples[frames_per_note].abs() < 0.001);
        assert!(samples[frames_per_note - 2].abs() < 0.01);
        assert!(samples[frames_per_note + 1].abs() < 0.01);
        assert!(samples.iter().any(|sample| sample.abs() > 0.5));
    }

    #[test]
    fn practice_tone_waveform_stays_smooth_throughout_long_notes() {
        let note_duration = Duration::from_secs(2);
        let frames_per_note =
            (note_duration.as_secs_f64() * PRACTICE_SAMPLE_RATE as f64).round() as usize;
        let samples = practice_samples(&[69], note_duration, PRACTICE_SAMPLE_RATE);
        let fade_frames =
            (NOTE_EDGE_FADE.as_secs_f64() * PRACTICE_SAMPLE_RATE as f64).round() as usize;
        let largest_step = samples[fade_frames..frames_per_note - fade_frames]
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .fold(0.0_f32, f32::max);

        assert!(largest_step < 0.1, "largest sample step was {largest_step}");
    }

    #[test]
    fn stopping_practice_audio_fades_to_silence() {
        let source = SamplesBuffer::new(1, 1_000, vec![0.5; 100]);
        let (source, stop_signal) = FadeOut::new(source, Duration::from_millis(10));
        stop_signal.store(true, Ordering::Relaxed);

        let faded = source.collect::<Vec<_>>();
        assert_eq!(faded.len(), 10);
        assert_eq!(faded.first(), Some(&0.5));
        assert_eq!(faded.last(), Some(&0.0));
        assert!(faded.windows(2).all(|pair| pair[0] >= pair[1]));
    }
}
