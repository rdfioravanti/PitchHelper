use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::time::Duration;

use rodio::source::SineWave;
use rodio::{Decoder, OutputStream, Sink, Source};

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
}

impl TonePlayer {
    pub fn new() -> Self {
        Self {
            output: None,
            sink: None,
        }
    }

    pub fn play_drone(&mut self, midi: i32, volume: f32) -> Result<(), String> {
        self.start_output(volume)?;
        let Some(sink) = self.sink.as_ref() else {
            return Err("Could not create practice audio player.".to_owned());
        };
        sink.append(SineWave::new(midi_frequency(midi)).repeat_infinite());
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
        self.start_output(volume)?;
        let Some(sink) = self.sink.as_ref() else {
            return Err("Could not create practice audio player.".to_owned());
        };
        for note in notes {
            sink.append(SineWave::new(midi_frequency(*note)).take_duration(note_duration));
        }
        Ok(())
    }

    pub fn stop(&mut self) {
        self.clear_output();
    }

    pub fn is_playing(&self) -> bool {
        self.sink.as_ref().is_some_and(|sink| !sink.empty())
    }

    fn start_output(&mut self, volume: f32) -> Result<(), String> {
        self.clear_output();
        let (output, handle) = OutputStream::try_default()
            .map_err(|error| format!("Could not open audio output: {error}"))?;
        let sink = Sink::try_new(&handle)
            .map_err(|error| format!("Could not create practice audio player: {error}"))?;
        sink.set_volume(volume.clamp(0.0, 1.0));
        sink.play();
        self.output = Some(output);
        self.sink = Some(sink);
        Ok(())
    }

    fn clear_output(&mut self) {
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }
        self.output = None;
    }
}

fn midi_frequency(midi: i32) -> f32 {
    440.0 * 2.0_f32.powf((midi as f32 - 69.0) / 12.0)
}
