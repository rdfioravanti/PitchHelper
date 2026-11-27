mod audio;
mod music;
mod pitch;
mod playback;
mod practice;
mod ui;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use audio::{InputDevice, Metronome, Microphone};
use eframe::egui::{self, Color32, Stroke};
use pitch::{PitchReading, midi_to_note};
use playback::{SongPlayer, TonePlayer};
use practice::{RangeTracker, Scale, WarmupPattern, pitch_metrics};

const BAR_BEATS: f32 = 4.0;
const VISIBLE_BARS: f32 = 4.0;
const CENTS_TOLERANCE: f32 = 15.0;
const LOOP_HANDLE_HIT_RADIUS: f32 = 12.0;
const ECHO_COUNTDOWN_SECONDS: u64 = 3;
const DEFAULT_RANGE_SENSITIVITY: f32 = 35.0;
const RANGE_TRACKING_MIN_RMS: f32 = 0.002;
const RANGE_TRACKING_MAX_RMS: f32 = 0.04;
const RANGE_TRACKING_MIN_DURATION: Duration = Duration::from_millis(400);
const COPYRIGHT: &str = "© 2026 Richard Fioravanti";

enum AudioLoadEvent {
    Analyzing(String),
    Complete(PathBuf, Result<music::TrackAnalysis, String>),
    Cancelled,
}

enum AudioLoadStatus {
    ChoosingFile,
    Analyzing(String),
}

#[derive(Clone, Copy)]
enum LoopHandle {
    Start,
    End,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReferenceKind {
    Root,
    Fifth,
}

struct WarmupSession {
    pattern: WarmupPattern,
    tonic: i32,
    low_tonic: i32,
    high_tonic: i32,
    direction: i32,
    note_duration: Duration,
    started_at: Instant,
    notes: Vec<i32>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EchoPhase {
    Countdown,
    Prompting,
    Pause,
    Responding,
    Score,
}

struct EchoSession {
    pattern: WarmupPattern,
    tonic: i32,
    low_tonic: i32,
    high_tonic: i32,
    direction: i32,
    notes: Vec<i32>,
    phase: EchoPhase,
    note_duration: Duration,
    started_at: Instant,
    responses: Vec<Vec<f32>>,
}

fn main() -> eframe::Result<()> {
    let app_icon =
        eframe::icon_data::from_png_bytes(include_bytes!("../assets/pitchhelper-icon.png"))
            .expect("embedded application icon should be a valid PNG");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PitchHelper")
            .with_icon(Arc::new(app_icon))
            .with_inner_size([1120.0, 760.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "PitchHelper",
        options,
        Box::new(|cc| Ok(Box::new(PitchHelperApp::new(cc)))),
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ThemeChoice {
    System,
    Light,
    Dark,
}

struct PitchHelperApp {
    microphone: Microphone,
    input_devices: Vec<InputDevice>,
    selected_input: usize,
    metronome: Metronome,
    tone_player: TonePlayer,
    metronome_enabled: bool,
    metronome_volume: f32,
    track_volume: f32,
    player: SongPlayer,
    audio_file: Option<PathBuf>,
    audio_load: Option<Receiver<AudioLoadEvent>>,
    audio_load_status: Option<AudioLoadStatus>,
    track_name: Option<String>,
    track_duration: Duration,
    track_waveform: Vec<f32>,
    loop_start: Duration,
    loop_end: Duration,
    dragging_loop_handle: Option<LoopHandle>,
    last_scrub_position: Option<Duration>,
    detected_bpm: Option<f32>,
    bpm_overridden: bool,
    bpm: f32,
    root_note: i32,
    scale: Scale,
    reference_enabled: bool,
    reference_kind: ReferenceKind,
    exercise_pattern: WarmupPattern,
    warmup: Option<WarmupSession>,
    echo: Option<EchoSession>,
    echo_score: Option<f32>,
    theme: ThemeChoice,
    applied_theme: ThemeChoice,
    applied_dark: bool,
    last_theme_check: Instant,
    last_device_refresh: Instant,
    reading: Option<PitchReading>,
    input_level_rms: f32,
    input_peak: f32,
    sensitivity: f32,
    range_sensitivity: f32,
    history: VecDeque<(f32, f32)>,
    recent_pitch: VecDeque<(Instant, f32)>,
    range_tracker: RangeTracker,
    session_range_tracker: RangeTracker,
    range_dirty: bool,
    last_range_save: Instant,
    graph_min_limit: f32,
    graph_max_limit: f32,
    zoom_scale: f32,
    graph_center: f32,
    center_on_root: bool,
    highlight_opacity: f32,
    beat_position: f64,
    last_song_position: f64,
    measure_phase: f32,
    last_cycle: u64,
    last_frame: Instant,
    error: Option<String>,
    about_open: bool,
    clear_range_confirmation: bool,
}

impl PitchHelperApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let theme = ThemeChoice::System;
        let applied_dark = theme_is_dark(theme);
        set_theme(&cc.egui_ctx, applied_dark);
        let (input_devices, mut initial_error) = match Microphone::available_devices() {
            Ok(devices) => (devices, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        let range_tracker = match RangeTracker::load() {
            Ok(tracker) => tracker,
            Err(error) => {
                initial_error = Some(match initial_error {
                    Some(existing) => format!("{existing} {error}"),
                    None => error,
                });
                RangeTracker::default()
            }
        };
        let selected_input = input_devices
            .iter()
            .position(|device| device.is_default)
            .unwrap_or(0);
        Self {
            microphone: Microphone::new(),
            input_devices,
            selected_input,
            metronome: Metronome::new(100.0, 0.35),
            tone_player: TonePlayer::new(),
            metronome_enabled: false,
            metronome_volume: 0.35,
            track_volume: 0.5,
            player: SongPlayer::new(),
            audio_file: None,
            audio_load: None,
            audio_load_status: None,
            track_name: None,
            track_duration: Duration::ZERO,
            track_waveform: Vec::new(),
            loop_start: Duration::ZERO,
            loop_end: Duration::ZERO,
            dragging_loop_handle: None,
            last_scrub_position: None,
            detected_bpm: None,
            bpm_overridden: false,
            bpm: 100.0,
            root_note: 60,
            scale: Scale::Major,
            reference_enabled: false,
            reference_kind: ReferenceKind::Root,
            exercise_pattern: WarmupPattern::FiveTone,
            warmup: None,
            echo: None,
            echo_score: None,
            theme,
            applied_theme: theme,
            applied_dark,
            last_theme_check: Instant::now(),
            last_device_refresh: Instant::now(),
            reading: None,
            input_level_rms: 0.0,
            input_peak: 0.0,
            sensitivity: 80.0,
            range_sensitivity: DEFAULT_RANGE_SENSITIVITY,
            history: VecDeque::new(),
            recent_pitch: VecDeque::new(),
            range_tracker,
            session_range_tracker: RangeTracker::default(),
            range_dirty: false,
            last_range_save: Instant::now(),
            graph_min_limit: 24.0,
            graph_max_limit: 96.0,
            zoom_scale: 1.0,
            graph_center: 60.0,
            center_on_root: true,
            highlight_opacity: 0.04,
            beat_position: 0.0,
            last_song_position: 0.0,
            measure_phase: 0.0,
            last_cycle: 0,
            last_frame: Instant::now(),
            error: initial_error,
            about_open: false,
            clear_range_confirmation: false,
        }
    }

    fn update_audio(&mut self) {
        self.microphone.set_sensitivity(self.sensitivity);
        if !self.microphone.is_running() {
            self.input_level_rms = 0.0;
            self.input_peak = 0.0;
            return;
        }
        let analysis = match self.microphone.analysis() {
            Ok(analysis) => analysis,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        self.input_level_rms = analysis.rms;
        self.input_peak = analysis.peak;
        let Some(reading) = analysis.pitch else {
            self.reading = None;
            return;
        };

        self.reading = Some(reading);
        let now = Instant::now();
        let history_phase = if self.echo_score.is_some() {
            None
        } else if let Some(session) = &self.echo {
            match session.phase {
                EchoPhase::Countdown
                | EchoPhase::Prompting
                | EchoPhase::Pause
                | EchoPhase::Score => None,
                EchoPhase::Responding => Some(echo_sequence_phase(
                    now,
                    session.started_at,
                    session.note_duration,
                    session.notes.len(),
                )),
            }
        } else if let Some(session) = &self.warmup {
            Some(practice_phase(
                now,
                session.started_at,
                session.note_duration,
            ))
        } else {
            Some(self.measure_phase)
        };
        if let Some(phase) = history_phase {
            self.history.push_back((phase, reading.midi));
            while self.history.len() > 900 {
                self.history.pop_front();
            }
        }
        self.recent_pitch.push_back((now, reading.midi));
        while self.recent_pitch.front().is_some_and(|(sample_time, _)| {
            now.duration_since(*sample_time) > Duration::from_secs(1)
        }) {
            self.recent_pitch.pop_front();
        }
        if let Some(metrics) =
            pitch_metrics(&self.recent_pitch.iter().copied().collect::<Vec<_>>(), now)
            && range_sample_is_eligible(analysis.rms, metrics.sample_span, self.range_sensitivity)
            && self.range_tracker.record(reading.midi, metrics.steady)
        {
            self.session_range_tracker
                .record(reading.midi, metrics.steady);
            self.range_dirty = true;
        }
    }

    fn start_reference_tone(&mut self) {
        let midi = self.reference_midi();
        if let Err(message) = self.tone_player.play_drone(midi, 0.2) {
            self.reference_enabled = false;
            self.error = Some(message);
        }
    }

    fn reference_midi(&self) -> i32 {
        let root = self.root_note;
        match self.reference_kind {
            ReferenceKind::Root => root,
            ReferenceKind::Fifth => root + 7,
        }
    }

    fn start_warmup(&mut self) {
        self.tone_player.stop();
        self.reference_enabled = false;
        self.echo = None;
        self.echo_score = None;
        self.history.clear();
        let tonic = self.root_note;
        let notes = warmup_notes(self.exercise_pattern, tonic);
        let note_duration = exercise_note_duration(self.bpm);
        if let Err(message) = self.tone_player.play_notes(&notes, note_duration, 0.25) {
            self.warmup = None;
            self.error = Some(message);
            return;
        }
        self.warmup = Some(WarmupSession {
            pattern: self.exercise_pattern,
            tonic,
            low_tonic: tonic,
            high_tonic: tonic + 12,
            direction: 1,
            note_duration,
            started_at: Instant::now(),
            notes,
        });
    }

    fn start_echo(&mut self) {
        self.tone_player.stop();
        self.reference_enabled = false;
        self.warmup = None;
        self.echo_score = None;
        self.history.clear();
        let tonic = self.root_note;
        let notes = warmup_notes(self.exercise_pattern, tonic);
        let note_duration = exercise_note_duration(self.bpm);
        self.echo = Some(EchoSession {
            pattern: self.exercise_pattern,
            tonic,
            low_tonic: tonic,
            high_tonic: tonic + 12,
            direction: 1,
            responses: vec![Vec::new(); notes.len()],
            notes,
            phase: EchoPhase::Countdown,
            note_duration,
            started_at: Instant::now(),
        });
    }

    fn start_next_echo_sequence(&mut self) {
        let Some(session) = self.echo.as_ref() else {
            return;
        };
        let (tonic, direction) = next_warmup_tonic(
            session.tonic,
            session.direction,
            session.low_tonic,
            session.high_tonic,
        );
        let pattern = session.pattern;
        let note_duration = exercise_note_duration(self.bpm);
        let notes = warmup_notes(pattern, tonic);

        if let Some(session) = self.echo.as_mut() {
            session.tonic = tonic;
            session.direction = direction;
            session.notes = notes.clone();
            session.responses = vec![Vec::new(); notes.len()];
            session.phase = EchoPhase::Countdown;
            session.note_duration = note_duration;
            session.started_at = Instant::now();
        }
        self.echo_score = None;
        self.history.clear();
    }

    fn update_practice(&mut self, now: Instant) {
        if let Some(session) = self.warmup.as_mut()
            && !self.tone_player.is_playing()
        {
            (session.tonic, session.direction) = next_warmup_tonic(
                session.tonic,
                session.direction,
                session.low_tonic,
                session.high_tonic,
            );
            session.notes = warmup_notes(session.pattern, session.tonic);
            session.note_duration = exercise_note_duration(self.bpm);
            if let Err(message) =
                self.tone_player
                    .play_notes(&session.notes, session.note_duration, 0.25)
            {
                self.warmup = None;
                self.error = Some(message);
            } else {
                session.started_at = now;
                self.history.clear();
            }
        }

        let Some(session) = self.echo.as_mut() else {
            return;
        };
        match session.phase {
            EchoPhase::Countdown => {
                if now.duration_since(session.started_at)
                    >= Duration::from_secs(ECHO_COUNTDOWN_SECONDS)
                {
                    session.note_duration = exercise_note_duration(self.bpm);
                    if let Err(message) =
                        self.tone_player
                            .play_notes(&session.notes, session.note_duration, 0.25)
                    {
                        self.echo = None;
                        self.error = Some(message);
                    } else {
                        session.phase = EchoPhase::Prompting;
                        session.started_at = now;
                    }
                }
                return;
            }
            EchoPhase::Prompting => {
                if !self.tone_player.is_playing() {
                    session.phase = EchoPhase::Pause;
                    session.started_at = now;
                    self.history.clear();
                }
                return;
            }
            EchoPhase::Pause => {
                if now.duration_since(session.started_at)
                    >= Duration::from_secs(ECHO_COUNTDOWN_SECONDS)
                {
                    session.phase = EchoPhase::Responding;
                    session.started_at = now;
                    self.history.clear();
                }
                return;
            }
            EchoPhase::Score => return,
            EchoPhase::Responding => {}
        }
        let elapsed = now.duration_since(session.started_at);
        let slot = (elapsed.as_secs_f32() / session.note_duration.as_secs_f32()) as usize;
        if slot < session.notes.len()
            && let Some(reading) = self.reading
        {
            session.responses[slot].push(reading.midi);
        }
        let total_duration = session.note_duration * session.notes.len() as u32;
        if elapsed >= total_duration {
            let mut error_sum = 0.0;
            for (note, responses) in session.notes.iter().zip(&session.responses) {
                if !responses.is_empty() {
                    let mean = responses.iter().sum::<f32>() / responses.len() as f32;
                    error_sum += (mean - *note as f32).abs();
                } else {
                    error_sum += 2.0;
                }
            }
            self.echo_score =
                Some((100.0 - error_sum / session.notes.len() as f32 * 50.0).clamp(0.0, 100.0));
            session.phase = EchoPhase::Score;
        }
    }

    fn save_range_if_needed(&mut self, now: Instant) {
        if self.range_dirty && now.duration_since(self.last_range_save) >= Duration::from_secs(10) {
            match self.range_tracker.save() {
                Ok(()) => {
                    self.range_dirty = false;
                    self.last_range_save = now;
                }
                Err(error) => {
                    self.error = Some(format!("Could not save vocal range: {error}"));
                    self.last_range_save = now;
                }
            }
        }
    }

    fn clear_range_history(&mut self) {
        match RangeTracker::clear_saved() {
            Ok(()) => {
                self.range_tracker = RangeTracker::default();
                self.session_range_tracker = RangeTracker::default();
                self.range_dirty = false;
                self.last_range_save = Instant::now();
                self.clear_range_confirmation = false;
            }
            Err(error) => {
                self.error = Some(format!("Could not clear range history: {error}"));
            }
        }
    }

    fn visible_graph_range(&self) -> (f32, f32) {
        centered_graph_range(
            self.graph_min_limit,
            self.graph_max_limit,
            self.zoom_scale,
            self.graph_center,
        )
    }

    fn update_graph_view(&mut self) {
        let (_, _, span) =
            graph_view_limits(self.graph_min_limit, self.graph_max_limit, self.zoom_scale);
        if span <= 0.0 {
            return;
        }
        let target_center = if self.center_on_root {
            self.center_on_root = false;
            let target = match self.practice_target_center() {
                Some(target) => target,
                None => self.root_note as f32,
            };
            clamp_graph_center(self.graph_min_limit, self.graph_max_limit, span, target)
        } else if let Some(target_center) = self.practice_target_center() {
            clamp_graph_center(
                self.graph_min_limit,
                self.graph_max_limit,
                span,
                target_center,
            )
        } else if let Some(reading) = self.reading {
            follow_graph_center(
                self.graph_center,
                reading.midi,
                self.graph_min_limit,
                self.graph_max_limit,
                span,
            )
        } else {
            self.graph_center
        };
        self.graph_center = clamp_graph_center(
            self.graph_min_limit,
            self.graph_max_limit,
            span,
            target_center,
        );
    }

    fn practice_target_center(&self) -> Option<f32> {
        if self.reference_enabled {
            return Some(self.reference_midi() as f32);
        }
        let notes = self
            .warmup
            .as_ref()
            .map(|session| session.notes.as_slice())
            .or_else(|| self.echo.as_ref().map(|session| session.notes.as_slice()))?;
        let low = notes.iter().min()?;
        let high = notes.iter().max()?;
        Some((*low as f32 + *high as f32) * 0.5)
    }

    fn reset_timeline(&mut self) {
        self.beat_position = 0.0;
        self.last_song_position = 0.0;
        self.measure_phase = 0.0;
        self.last_cycle = 0;
        self.history.clear();
    }

    fn stop_warmup(&mut self) {
        if self.warmup.take().is_some() {
            self.tone_player.stop();
            self.history.clear();
        }
    }

    fn stop_echo(&mut self) {
        if self.echo.take().is_some() {
            self.echo_score = None;
            self.tone_player.stop();
            self.history.clear();
        }
    }

    fn seek_to(&mut self, position: Duration) -> bool {
        let position = position.min(self.track_duration);
        if let Err(message) = self.player.seek(position) {
            self.error = Some(message);
            return false;
        }

        let song_position = position.as_secs_f64();
        let beat_position = song_position * self.bpm as f64 / 60.0;
        let cycle = (beat_position / beats_per_cycle()).floor() as u64;
        if cycle != self.last_cycle {
            self.history.clear();
        }
        self.beat_position = beat_position;
        self.last_song_position = song_position;
        self.measure_phase = measure_phase(beat_position);
        self.last_cycle = cycle;
        self.metronome.set_position(beat_position);
        self.error = None;
        true
    }

    fn update_track_loop(&mut self) {
        let finished = self.player.is_finished();
        if self.loop_start >= self.loop_end
            || (!self.player.is_playing() && !finished)
            || (!finished && self.player.position() < self.loop_end)
        {
            return;
        }

        let loop_start = self.loop_start;
        if !self.seek_to(loop_start) || !finished {
            return;
        }
        if let Some(path) = self.audio_file.clone() {
            if let Err(message) = self.player.toggle(&path) {
                self.error = Some(message);
            } else {
                self.metronome.set_running(true);
            }
        }
    }

    fn start_listening(&mut self) {
        self.error = None;
        self.reading = None;
        self.history.clear();
        self.recent_pitch.clear();
        self.input_level_rms = 0.0;
        self.input_peak = 0.0;
        let Some(device) = self.input_devices.get(self.selected_input) else {
            self.error = Some(
                "No microphone is available. Connect an input device and refresh the list."
                    .to_owned(),
            );
            return;
        };
        if let Err(message) = self.microphone.start(device) {
            self.error = Some(message);
        }
    }

    fn refresh_input_devices(&mut self) {
        let selected_device = self
            .input_devices
            .get(self.selected_input)
            .map(|device| (self.selected_input, device.name.clone(), device.is_default));
        match Microphone::available_devices() {
            Ok(devices) => {
                let selected_is_present = selected_device
                    .as_ref()
                    .map(|(_, name, _)| devices.iter().any(|device| device.name == *name))
                    .unwrap_or(false);
                self.selected_input = selected_device
                    .as_ref()
                    .and_then(|(old_index, name, is_default)| {
                        if devices
                            .get(*old_index)
                            .map(|device| device.name == *name)
                            .unwrap_or(false)
                        {
                            return Some(*old_index);
                        }
                        devices
                            .iter()
                            .position(|device| {
                                device.name == *name && device.is_default == *is_default
                            })
                            .or_else(|| devices.iter().position(|device| device.name == *name))
                    })
                    .or_else(|| devices.iter().position(|device| device.is_default))
                    .unwrap_or(0);
                self.input_devices = devices;
                if (self.microphone.is_running() || self.microphone.is_starting())
                    && !selected_is_present
                {
                    self.microphone.stop();
                    self.reading = None;
                    self.recent_pitch.clear();
                    self.input_level_rms = 0.0;
                    self.input_peak = 0.0;
                    self.error = Some(
                        "The selected microphone was disconnected. Select an input and start listening again."
                            .to_owned(),
                    );
                }
            }
            Err(message) => self.error = Some(message),
        }
    }

    fn choose_audio_file(&mut self) {
        if self.audio_load.is_some() {
            return;
        }
        self.error = None;
        let (sender, receiver) = mpsc::channel();
        match thread::Builder::new()
            .name("pitchhelper-audio-dialog".to_owned())
            .spawn(move || {
                let Some(path) = rfd::FileDialog::new()
                    .add_filter("Audio files", &["wav", "mp3", "flac", "ogg"])
                    .pick_file()
                else {
                    let _ = sender.send(AudioLoadEvent::Cancelled);
                    return;
                };
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Audio track".to_owned());
                if sender.send(AudioLoadEvent::Analyzing(name)).is_err() {
                    return;
                }
                let result = music::analyze_track(&path);
                let _ = sender.send(AudioLoadEvent::Complete(path, result));
            }) {
            Ok(_) => {
                self.audio_load = Some(receiver);
                self.audio_load_status = Some(AudioLoadStatus::ChoosingFile);
            }
            Err(error) => self.error = Some(format!("Could not start audio analysis: {error}")),
        }
    }

    fn update_audio_load(&mut self) {
        let event = match self.audio_load.as_ref() {
            Some(receiver) => receiver.try_recv(),
            None => return,
        };
        match event {
            Ok(AudioLoadEvent::Analyzing(name)) => {
                self.audio_load_status = Some(AudioLoadStatus::Analyzing(name));
            }
            Ok(AudioLoadEvent::Complete(path, result)) => {
                self.audio_load = None;
                self.audio_load_status = None;
                match result {
                    Ok(analysis) => {
                        self.player.stop();
                        self.reset_timeline();
                        self.metronome.reset_phase();
                        self.track_name = path
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned());
                        self.track_duration = analysis.duration;
                        self.track_waveform = analysis.waveform;
                        self.loop_start = Duration::ZERO;
                        self.loop_end = analysis.duration;
                        self.dragging_loop_handle = None;
                        self.last_scrub_position = None;
                        self.audio_file = Some(path);
                        let detected_bpm = analysis.bpm;
                        self.detected_bpm = detected_bpm;
                        self.bpm_overridden = false;
                        if let Some(detected_bpm) = detected_bpm {
                            self.bpm = detected_bpm;
                        } else {
                            self.error = Some(
                                "Track loaded, but no steady BPM was detected. Set the tempo manually."
                                    .to_owned(),
                            );
                        }
                    }
                    Err(message) => self.error = Some(message),
                }
            }
            Ok(AudioLoadEvent::Cancelled) => {
                self.audio_load = None;
                self.audio_load_status = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.audio_load = None;
                self.audio_load_status = None;
                self.error = Some("Audio loading stopped before it could finish.".to_owned());
            }
        }
    }
}

impl Drop for PitchHelperApp {
    fn drop(&mut self) {
        if self.range_dirty
            && let Err(error) = self.range_tracker.save()
        {
            eprintln!("Could not save vocal range: {error}");
        }
    }
}

fn close_menu_if_pointer_left(ui: &mut egui::Ui, menu_bar_rect: egui::Rect) {
    let menu_rect = ui.min_rect().expand(10.0);
    let pointer_inside_menu = ui
        .input(|input| input.pointer.hover_pos())
        .is_some_and(|position| {
            menu_rect.contains(position)
                || menu_bar_rect.contains(position)
                || ui
                    .ctx()
                    .layer_id_at(position)
                    .is_some_and(|layer_id| layer_id.order == egui::Order::Foreground)
        });
    if !pointer_inside_menu {
        ui.close_menu();
    }
}

fn warmup_notes(pattern: WarmupPattern, tonic: i32) -> Vec<i32> {
    pattern
        .intervals()
        .iter()
        .map(|interval| tonic + interval)
        .collect()
}

fn exercise_note_duration(bpm: f32) -> Duration {
    Duration::from_secs_f64(60.0 / bpm.clamp(40.0, 240.0) as f64)
}

fn root_note_choices(low: f32, high: f32) -> std::ops::RangeInclusive<i32> {
    low.ceil() as i32..=high.floor() as i32
}

fn graph_low_note_choices(high: f32) -> std::ops::RangeInclusive<i32> {
    12..=(high.floor() as i32 - 1)
}

fn graph_high_note_choices(low: f32) -> std::ops::RangeInclusive<i32> {
    (low.ceil() as i32 + 1)..=120
}

fn next_warmup_tonic(tonic: i32, direction: i32, low: i32, high: i32) -> (i32, i32) {
    let direction = if direction > 0 && tonic >= high {
        -1
    } else if direction < 0 && tonic <= low {
        1
    } else {
        direction
    };
    (tonic + direction, direction)
}

fn practice_phase(now: Instant, started_at: Instant, note_duration: Duration) -> f32 {
    let total_seconds = note_duration.as_secs_f32() * VISIBLE_BARS * BAR_BEATS;
    if total_seconds <= 0.0 {
        return 0.0;
    }
    (now.duration_since(started_at).as_secs_f32() / total_seconds).clamp(0.0, 1.0)
}

fn echo_sequence_phase(
    now: Instant,
    started_at: Instant,
    note_duration: Duration,
    note_count: usize,
) -> f32 {
    let total_seconds = note_duration.as_secs_f32() * note_count as f32;
    if total_seconds <= 0.0 {
        return 0.0;
    }
    (now.duration_since(started_at).as_secs_f32() / total_seconds).clamp(0.0, 1.0)
}

fn echo_countdown_value(elapsed: Duration) -> Option<u8> {
    let elapsed_seconds = elapsed.as_secs();
    (elapsed_seconds < ECHO_COUNTDOWN_SECONDS)
        .then_some((ECHO_COUNTDOWN_SECONDS - elapsed_seconds) as u8)
}

fn draw_target_notes(
    painter: &egui::Painter,
    plot: egui::Rect,
    low_midi: f32,
    high_midi: f32,
    notes: &[i32],
    color: Color32,
) {
    for (index, note) in notes.iter().enumerate() {
        if !(*note as f32 >= low_midi && *note as f32 <= high_midi) {
            continue;
        }
        let left = plot.left() + index as f32 / notes.len() as f32 * plot.width();
        let right = plot.left() + (index + 1) as f32 / notes.len() as f32 * plot.width();
        let y = plot.bottom() - (*note as f32 - low_midi) / (high_midi - low_midi) * plot.height();
        painter.line_segment(
            [egui::pos2(left, y), egui::pos2(right, y)],
            Stroke::new(2.0_f32, color),
        );
    }
}

fn draw_practice_target_notes(
    painter: &egui::Painter,
    plot: egui::Rect,
    low_midi: f32,
    high_midi: f32,
    notes: &[i32],
    note_duration: Duration,
    color: Color32,
) {
    let measure_duration = note_duration.as_secs_f32() * VISIBLE_BARS * BAR_BEATS;
    if notes.is_empty() || measure_duration <= 0.0 {
        return;
    }
    let note_width = plot.width() * note_duration.as_secs_f32() / measure_duration;
    for (index, note) in notes.iter().enumerate() {
        if !(*note as f32 >= low_midi && *note as f32 <= high_midi) {
            continue;
        }
        let left = plot.left() + index as f32 * note_width;
        if left >= plot.right() {
            break;
        }
        let right = (left + note_width).min(plot.right());
        let y = plot.bottom() - (*note as f32 - low_midi) / (high_midi - low_midi) * plot.height();
        painter.line_segment(
            [egui::pos2(left, y), egui::pos2(right, y)],
            Stroke::new(2.0_f32, color),
        );
    }
}

fn pitch_color(cents: f32) -> Color32 {
    if cents.abs() <= CENTS_TOLERANCE {
        Color32::from_rgb(54, 190, 132)
    } else if cents < 0.0 {
        Color32::from_rgb(237, 86, 91)
    } else {
        Color32::from_rgb(77, 151, 255)
    }
}

fn range_sample_is_eligible(rms: f32, sample_span: Duration, sensitivity: f32) -> bool {
    let sensitivity = (sensitivity / 100.0).clamp(0.0, 1.0);
    let minimum_rms = RANGE_TRACKING_MIN_RMS
        + (RANGE_TRACKING_MAX_RMS - RANGE_TRACKING_MIN_RMS) * (1.0 - sensitivity).powi(2);
    rms.is_finite() && rms >= minimum_rms && sample_span >= RANGE_TRACKING_MIN_DURATION
}

fn theme_is_dark(theme: ThemeChoice) -> bool {
    match theme {
        ThemeChoice::System => dark_light::detect() == dark_light::Mode::Dark,
        ThemeChoice::Light => false,
        ThemeChoice::Dark => true,
    }
}

fn set_theme(ctx: &egui::Context, dark: bool) {
    ctx.set_visuals(if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    });
}

fn beats_per_cycle() -> f64 {
    BAR_BEATS as f64 * VISIBLE_BARS as f64
}

fn advance_beat_position(position: f64, elapsed_seconds: f64, bpm: f32) -> f64 {
    position + elapsed_seconds * bpm as f64 / 60.0
}

fn advance_song_beat_position(
    beat_position: f64,
    previous_song_position: f64,
    song_position: f64,
    bpm: f32,
) -> f64 {
    let elapsed = (song_position - previous_song_position).max(0.0);
    advance_beat_position(beat_position, elapsed, bpm)
}

fn measure_phase(beat_position: f64) -> f32 {
    (beat_position.rem_euclid(beats_per_cycle()) / beats_per_cycle()) as f32
}

fn format_track_time(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let seconds = total_seconds % 60;
    let total_minutes = total_seconds / 60;
    if total_minutes >= 60 {
        format!(
            "{}:{:02}:{:02}",
            total_minutes / 60,
            total_minutes % 60,
            seconds
        )
    } else {
        format!("{:02}:{:02}", total_minutes, seconds)
    }
}

fn move_loop_handle(
    start: Duration,
    end: Duration,
    target: Duration,
    handle: LoopHandle,
) -> (Duration, Duration) {
    match handle {
        LoopHandle::Start => (target.min(end), end),
        LoopHandle::End => (start, target.max(start)),
    }
}

fn should_show_scrub_preview(cursor_x: f32, start_x: f32, end_x: f32) -> bool {
    (cursor_x - start_x).abs() > LOOP_HANDLE_HIT_RADIUS
        && (cursor_x - end_x).abs() > LOOP_HANDLE_HIT_RADIUS
}

fn graph_range_for_zoom(min_note: f32, max_note: f32, zoom_scale: f32) -> (f32, f32) {
    let zoom_scale = zoom_scale.clamp(0.5, 2.0) * 2.0;
    let center = (min_note + max_note) * 0.5;
    let span = (max_note - min_note) / zoom_scale;
    (center - span * 0.5, center + span * 0.5)
}

fn graph_view_limits(min_note: f32, max_note: f32, zoom_scale: f32) -> (f32, f32, f32) {
    let (visible_low, visible_high) = graph_range_for_zoom(min_note, max_note, zoom_scale);
    let span = visible_high - visible_low;
    (min_note + span * 0.5, max_note - span * 0.5, span)
}

fn clamp_graph_center(min_note: f32, max_note: f32, span: f32, center: f32) -> f32 {
    center.clamp(min_note + span * 0.5, max_note - span * 0.5)
}

fn follow_graph_center(
    current_center: f32,
    midi: f32,
    min_note: f32,
    max_note: f32,
    span: f32,
) -> f32 {
    let current_center = clamp_graph_center(min_note, max_note, span, current_center);
    let low = current_center - span * 0.5;
    let high = current_center + span * 0.5;
    let target_center = if midi < low + span * 0.25 || midi > high - span * 0.25 {
        current_center + (midi - current_center) * 0.18
    } else {
        current_center
    };
    clamp_graph_center(min_note, max_note, span, target_center)
}

fn centered_graph_range(min_note: f32, max_note: f32, zoom_scale: f32, center: f32) -> (f32, f32) {
    let (min_center, max_center, span) = graph_view_limits(min_note, max_note, zoom_scale);
    let center = center.clamp(min_center, max_center);
    (center - span * 0.5, center + span * 0.5)
}

fn zoom_label(zoom_scale: f32) -> String {
    if (zoom_scale - zoom_scale.round()).abs() < 0.001 {
        format!("{:.0}x", zoom_scale)
    } else {
        format!("{zoom_scale:.1}x")
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_RANGE_SENSITIVITY, LoopHandle, WarmupPattern, advance_beat_position,
        advance_song_beat_position, centered_graph_range, clamp_graph_center, echo_countdown_value,
        echo_sequence_phase, exercise_note_duration, follow_graph_center, format_track_time,
        graph_high_note_choices, graph_low_note_choices, graph_range_for_zoom, measure_phase,
        move_loop_handle, next_warmup_tonic, practice_phase, range_sample_is_eligible,
        root_note_choices, should_show_scrub_preview, warmup_notes, zoom_label,
    };
    use std::time::Duration;

    #[test]
    fn range_tracking_filters_quiet_and_transient_pitch_samples() {
        let sustained = Duration::from_millis(450);
        assert!(!range_sample_is_eligible(
            0.01,
            sustained,
            DEFAULT_RANGE_SENSITIVITY
        ));
        assert!(!range_sample_is_eligible(
            0.03,
            Duration::from_millis(350),
            DEFAULT_RANGE_SENSITIVITY
        ));
        assert!(range_sample_is_eligible(
            0.03,
            sustained,
            DEFAULT_RANGE_SENSITIVITY
        ));
        assert!(range_sample_is_eligible(0.003, sustained, 100.0));
        assert!(!range_sample_is_eligible(0.003, sustained, 0.0));
    }

    #[test]
    fn default_zoom_is_one_and_half_zoom_shows_the_full_selected_range() {
        assert_eq!(graph_range_for_zoom(24.0, 96.0, 0.5), (24.0, 96.0));
        assert_eq!(graph_range_for_zoom(24.0, 96.0, 1.0), (42.0, 78.0));
        assert_eq!(graph_range_for_zoom(24.0, 96.0, 2.0), (51.0, 69.0));
        assert_eq!(zoom_label(1.0), "1x");
    }

    #[test]
    fn graph_bounds_always_allow_a_nonzero_pitch_range() {
        let low_choices = graph_low_note_choices(96.0).collect::<Vec<_>>();
        let high_choices = graph_high_note_choices(24.0).collect::<Vec<_>>();
        assert_eq!(low_choices.first(), Some(&12));
        assert_eq!(low_choices.last(), Some(&95));
        assert_eq!(high_choices.first(), Some(&25));
        assert_eq!(high_choices.last(), Some(&120));
        assert_eq!(graph_low_note_choices(25.0).last(), Some(24));
        assert_eq!(graph_high_note_choices(95.0).next(), Some(96));
        assert!(graph_low_note_choices(12.0).is_empty());
        assert!(graph_high_note_choices(120.0).is_empty());
    }

    #[test]
    fn zoomed_graph_centers_practice_targets_and_follows_live_pitch_within_limits() {
        assert_eq!(centered_graph_range(24.0, 96.0, 2.0, 51.5), (42.5, 60.5));
        assert_eq!(clamp_graph_center(24.0, 96.0, 18.0, 10.0), 33.0);
        assert!((follow_graph_center(60.0, 80.0, 24.0, 96.0, 18.0) - 63.6).abs() < 0.001);
    }

    #[test]
    fn tempo_changes_speed_measure_motion_without_changing_its_current_phase() {
        let current_position = 7.25;
        let phase_before_change = measure_phase(current_position);
        assert_eq!(
            measure_phase(advance_beat_position(current_position, 0.0, 180.0)),
            phase_before_change
        );

        let one_second_at_60_bpm = advance_beat_position(current_position, 1.0, 60.0);
        let one_second_at_120_bpm = advance_beat_position(current_position, 1.0, 120.0);
        assert_eq!(
            one_second_at_120_bpm - current_position,
            2.0 * (one_second_at_60_bpm - current_position)
        );
    }

    #[test]
    fn song_clock_pauses_with_playback_and_tempo_changes_do_not_jump_its_position() {
        let current_position = 6.5;
        assert_eq!(
            advance_song_beat_position(current_position, 12.0, 12.0, 120.0),
            current_position
        );
        assert_eq!(
            advance_song_beat_position(current_position, 12.0, 12.0, 180.0),
            current_position
        );
        assert_eq!(
            advance_song_beat_position(current_position, 12.0, 12.5, 120.0),
            7.5
        );
    }

    #[test]
    fn track_time_format_handles_minutes_and_hours() {
        assert_eq!(format_track_time(Duration::from_secs(125)), "02:05");
        assert_eq!(format_track_time(Duration::from_secs(3661)), "1:01:01");
    }

    #[test]
    fn practice_progress_tracks_beats_across_the_four_bar_graph() {
        let started_at = std::time::Instant::now();
        let note_duration = Duration::from_millis(450);
        assert_eq!(practice_phase(started_at, started_at, note_duration), 0.0);
        assert!(
            (practice_phase(
                started_at + Duration::from_millis(675),
                started_at,
                note_duration,
            ) - 0.09375)
                .abs()
                < 0.001
        );
        assert_eq!(
            practice_phase(
                started_at + Duration::from_millis(7200),
                started_at,
                note_duration,
            ),
            1.0
        );
    }

    #[test]
    fn echo_progress_spans_only_the_sequence_duration_and_holds_at_the_end() {
        let started_at = std::time::Instant::now();
        let note_duration = Duration::from_millis(500);
        assert_eq!(
            echo_sequence_phase(started_at, started_at, note_duration, 4),
            0.0
        );
        assert_eq!(
            echo_sequence_phase(
                started_at + Duration::from_secs(1),
                started_at,
                note_duration,
                4,
            ),
            0.5
        );
        assert_eq!(
            echo_sequence_phase(
                started_at + Duration::from_secs(3),
                started_at,
                note_duration,
                4,
            ),
            1.0
        );
    }

    #[test]
    fn pitch_echo_countdown_shows_three_two_one_then_finishes() {
        assert_eq!(echo_countdown_value(Duration::ZERO), Some(3));
        assert_eq!(echo_countdown_value(Duration::from_millis(1_500)), Some(2));
        assert_eq!(echo_countdown_value(Duration::from_millis(2_500)), Some(1));
        assert_eq!(echo_countdown_value(Duration::from_secs(3)), None);
    }

    #[test]
    fn exercise_note_duration_follows_selected_tempo() {
        assert_eq!(exercise_note_duration(60.0), Duration::from_secs(1));
        assert_eq!(exercise_note_duration(120.0), Duration::from_millis(500));
        assert_eq!(exercise_note_duration(240.0), Duration::from_millis(250));
    }

    #[test]
    fn warmup_notes_transpose_the_pattern_by_semitones() {
        let c_pattern = warmup_notes(WarmupPattern::FiveTone, 60);
        let c_sharp_pattern = warmup_notes(WarmupPattern::FiveTone, 61);
        assert_eq!(c_pattern.len(), 9);
        assert!(
            c_pattern
                .iter()
                .zip(c_sharp_pattern)
                .all(|(c_note, c_sharp_note)| c_sharp_note == c_note + 1)
        );
    }

    #[test]
    fn key_root_choices_include_each_note_inside_graph_limits() {
        let choices = root_note_choices(24.0, 96.0).collect::<Vec<_>>();
        assert_eq!(choices.first(), Some(&24));
        assert_eq!(choices.last(), Some(&96));
        assert_eq!(choices.len(), 73);
        assert_eq!(
            root_note_choices(60.2, 63.8).collect::<Vec<_>>(),
            vec![61, 62, 63]
        );
    }

    #[test]
    fn centering_on_the_selected_root_clamps_to_the_display_limits() {
        // Zoom 1.0 spans MIDI 42..=78 (span 36), so a valid center is 42..=78.
        // A normal root stays where it is asked.
        assert_eq!(clamp_graph_center(24.0, 96.0, 36.0, 60.0), 60.0);
        // The lowest selectable root (C1) is pushed up as far as the limits allow.
        assert_eq!(clamp_graph_center(24.0, 96.0, 36.0, 24.0), 42.0);
        // The highest selectable root (C7) is pushed down as far as the limits allow.
        assert_eq!(clamp_graph_center(24.0, 96.0, 36.0, 96.0), 78.0);
        // A tighter zoom (span 18) narrows the valid center range to 33..=87.
        assert_eq!(clamp_graph_center(24.0, 96.0, 18.0, 24.0), 33.0);
    }

    #[test]
    fn warmup_transposition_reverses_at_octave_boundaries_without_repeating_notes() {
        assert_eq!(next_warmup_tonic(59, 1, 48, 60), (60, 1));
        assert_eq!(next_warmup_tonic(60, 1, 48, 60), (59, -1));
        assert_eq!(next_warmup_tonic(49, -1, 48, 60), (48, -1));
        assert_eq!(next_warmup_tonic(48, -1, 48, 60), (49, 1));
    }

    #[test]
    fn loop_handles_cannot_cross() {
        let start = Duration::from_secs(2);
        let end = Duration::from_secs(8);
        assert_eq!(
            move_loop_handle(start, end, Duration::from_secs(10), LoopHandle::Start),
            (end, end)
        );
        assert_eq!(
            move_loop_handle(start, end, Duration::ZERO, LoopHandle::End),
            (start, start)
        );
    }

    #[test]
    fn scrub_preview_is_hidden_near_loop_handles() {
        assert!(!should_show_scrub_preview(108.0, 100.0, 300.0));
        assert!(!should_show_scrub_preview(288.0, 100.0, 300.0));
        assert!(should_show_scrub_preview(200.0, 100.0, 300.0));
    }
}
