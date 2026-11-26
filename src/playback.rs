use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use rodio::{Decoder, OutputStream, Sink};

pub struct SongPlayer {
    output: Option<OutputStream>,
    sink: Option<Sink>,
}

impl SongPlayer {
    pub fn new() -> Self {
        Self {
            output: None,
            sink: None,
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

        self.stop();
        let file =
            File::open(path).map_err(|error| format!("Could not open audio file: {error}"))?;
        let source = Decoder::new(BufReader::new(file))
            .map_err(|error| format!("Could not decode audio file: {error}"))?;
        let (output, handle) = OutputStream::try_default()
            .map_err(|error| format!("Could not open audio output: {error}"))?;
        let sink = Sink::try_new(&handle)
            .map_err(|error| format!("Could not create audio player: {error}"))?;
        sink.append(source);
        sink.play();
        self.output = Some(output);
        self.sink = Some(sink);
        Ok(())
    }

    pub fn stop(&mut self) {
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
}
