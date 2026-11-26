mod audio;
mod music;
mod pitch;
mod playback;
mod practice;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use audio::{InputDevice, Metronome, Microphone};
use eframe::egui::{self, Align, Color32, FontId, Layout, RichText, Stroke, Vec2};
use pitch::{PitchReading, midi_to_note};
use playback::{SongPlayer, TonePlayer};
use practice::{RangeTracker, Scale, pitch_metrics};

const BAR_BEATS: f32 = 4.0;
const VISIBLE_BARS: f32 = 4.0;
const CENTS_TOLERANCE: f32 = 15.0;
const LOOP_HANDLE_HIT_RADIUS: f32 = 12.0;
const ECHO_COUNTDOWN_SECONDS: u64 = 3;
const COPYRIGHT: &str = "© 2026 Richard Fioravanti";
const KEY_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

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

#[derive(Clone, Copy, PartialEq, Eq)]
enum WarmupPattern {
    FiveTone,
    Octave,
    Arpeggio,
    Siren,
}

impl WarmupPattern {
    fn label(self) -> &'static str {
        match self {
            Self::FiveTone => "5-Tone Scale",
            Self::Octave => "Octaves",
            Self::Arpeggio => "Arpeggio",
            Self::Siren => "Sirens",
        }
    }

    fn intervals(self) -> &'static [i32] {
        match self {
            Self::FiveTone => &[0, 2, 4, 5, 7, 5, 4, 2, 0],
            Self::Octave => &[0, 12, 0, 12, 0],
            Self::Arpeggio => &[0, 4, 7, 12, 7, 4, 0],
            Self::Siren => &[0, 7, 12, 19, 12, 7, 0],
        }
    }
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
    Responding,
}

struct EchoSession {
    notes: Vec<i32>,
    phase: EchoPhase,
    note_duration: Duration,
    started_at: Instant,
    responses: Vec<Vec<f32>>,
}

struct RepaintTicker {
    running: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl RepaintTicker {
    fn new(ctx: egui::Context) -> std::io::Result<Self> {
        let running = Arc::new(AtomicBool::new(true));
        let worker_running = Arc::clone(&running);
        let worker = thread::Builder::new()
            .name("pitchhelper-repaint-ticker".to_owned())
            .spawn(move || {
                while worker_running.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(33));
                    if worker_running.load(Ordering::Relaxed) {
                        ctx.request_repaint();
                    }
                }
            })?;
        Ok(Self {
            running,
            worker: Some(worker),
        })
    }
}

impl Drop for RepaintTicker {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            eprintln!("UI repaint ticker exited unexpectedly.");
        }
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PitchHelper")
            .with_inner_size([1120.0, 760.0])
            .with_min_inner_size([760.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "PitchHelper",
        options,
        Box::new(|cc| Ok(Box::new(PitchHelperApp::new(cc)?))),
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ThemeChoice {
    System,
    Light,
    Dark,
}

struct PitchHelperApp {
    _repaint_ticker: RepaintTicker,
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
    key_root: usize,
    scale: Scale,
    reference_enabled: bool,
    reference_kind: ReferenceKind,
    warmup_pattern: WarmupPattern,
    root_octave: i32,
    warmup: Option<WarmupSession>,
    echo: Option<EchoSession>,
    echo_score: Option<f32>,
    echo_notes: Option<Vec<i32>>,
    theme: ThemeChoice,
    applied_theme: ThemeChoice,
    applied_dark: bool,
    last_theme_check: Instant,
    last_device_refresh: Instant,
    reading: Option<PitchReading>,
    input_level_rms: f32,
    input_peak: f32,
    sensitivity: f32,
    history: VecDeque<(f32, f32)>,
    recent_pitch: VecDeque<(Instant, f32)>,
    range_tracker: RangeTracker,
    range_dirty: bool,
    last_range_save: Instant,
    graph_min_limit: f32,
    graph_max_limit: f32,
    zoom_scale: f32,
    graph_center: f32,
    highlight_opacity: f32,
    beat_position: f64,
    last_song_position: f64,
    measure_phase: f32,
    last_cycle: u64,
    last_frame: Instant,
    error: Option<String>,
    about_open: bool,
}

impl PitchHelperApp {
    fn new(cc: &eframe::CreationContext<'_>) -> std::io::Result<Self> {
        let repaint_ticker = RepaintTicker::new(cc.egui_ctx.clone())?;
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
        Ok(Self {
            _repaint_ticker: repaint_ticker,
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
            key_root: 0,
            scale: Scale::Major,
            reference_enabled: false,
            reference_kind: ReferenceKind::Root,
            warmup_pattern: WarmupPattern::FiveTone,
            root_octave: 4,
            warmup: None,
            echo: None,
            echo_score: None,
            echo_notes: None,
            theme,
            applied_theme: theme,
            applied_dark,
            last_theme_check: Instant::now(),
            last_device_refresh: Instant::now(),
            reading: None,
            input_level_rms: 0.0,
            input_peak: 0.0,
            sensitivity: 80.0,
            history: VecDeque::new(),
            recent_pitch: VecDeque::new(),
            range_tracker,
            range_dirty: false,
            last_range_save: Instant::now(),
            graph_min_limit: 24.0,
            graph_max_limit: 96.0,
            zoom_scale: 1.0,
            graph_center: 60.0,
            highlight_opacity: 0.04,
            beat_position: 0.0,
            last_song_position: 0.0,
            measure_phase: 0.0,
            last_cycle: 0,
            last_frame: Instant::now(),
            error: initial_error,
            about_open: false,
        })
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
                EchoPhase::Countdown | EchoPhase::Prompting => None,
                EchoPhase::Responding => Some(practice_phase(
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
                session.notes.len(),
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
        if pitch_metrics(&self.recent_pitch.iter().copied().collect::<Vec<_>>(), now)
            .is_some_and(|metrics| self.range_tracker.record(reading.midi, metrics.steady))
        {
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
        let root = root_midi(self.key_root, self.root_octave);
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
        self.echo_notes = None;
        self.history.clear();
        let tonic = root_midi(self.key_root, self.root_octave);
        let notes = warmup_notes(self.warmup_pattern, tonic);
        let note_duration = Duration::from_millis(450);
        if let Err(message) = self.tone_player.play_notes(&notes, note_duration, 0.25) {
            self.warmup = None;
            self.error = Some(message);
            return;
        }
        self.warmup = Some(WarmupSession {
            pattern: self.warmup_pattern,
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
        let tonic = root_midi(self.key_root, self.root_octave);
        let intervals = self.scale.intervals();
        let notes = vec![tonic, tonic + intervals[1], tonic + intervals[2]];
        let note_duration = Duration::from_millis(450);
        self.echo_notes = Some(notes.clone());
        self.echo = Some(EchoSession {
            responses: vec![Vec::new(); notes.len()],
            notes,
            phase: EchoPhase::Countdown,
            note_duration,
            started_at: Instant::now(),
        });
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
                    if let Err(message) =
                        self.tone_player
                            .play_notes(&session.notes, session.note_duration, 0.25)
                    {
                        self.echo = None;
                        self.echo_notes = None;
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
                    session.phase = EchoPhase::Responding;
                    session.started_at = now;
                    self.history.clear();
                }
                return;
            }
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
            self.echo = None;
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
        let target_center = if let Some(target_center) = self.practice_target_center() {
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
        let notes = if let Some(session) = &self.warmup {
            Some(session.notes.as_slice())
        } else if let Some(session) = &self.echo {
            Some(session.notes.as_slice())
        } else if self.echo_score.is_some() {
            self.echo_notes.as_deref()
        } else {
            None
        }?;
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

    fn draw_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("PitchHelper").size(21.0).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new("LIVE PITCH COACH")
                        .size(10.0)
                        .strong()
                        .color(Color32::from_rgb(125, 145, 161)),
                );
            });
        });
    }

    fn draw_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("TEMPO")
                    .size(10.0)
                    .strong()
                    .color(Color32::from_rgb(125, 145, 161)),
            );
            ui.add_space(6.0);
            let response = ui.add(
                egui::Slider::new(&mut self.bpm, 40.0..=240.0)
                    .suffix(" BPM")
                    .show_value(false),
            );
            if response.changed() && self.detected_bpm.is_some() {
                self.bpm_overridden = true;
            }
            ui.label(
                RichText::new(format!("{:.0}", self.bpm))
                    .monospace()
                    .strong(),
            );
            if let Some(detected_bpm) = self.detected_bpm {
                ui.label(
                    RichText::new(if self.bpm_overridden {
                        format!("file: {detected_bpm:.0} BPM · adjusted")
                    } else {
                        format!("file: {detected_bpm:.0} BPM")
                    })
                    .size(10.0)
                    .color(Color32::from_rgb(125, 145, 161)),
                );
            }
            ui.add_space(12.0);
            ui.label(
                RichText::new("ZOOM")
                    .size(10.0)
                    .strong()
                    .color(Color32::from_rgb(125, 145, 161)),
            );
            ui.add_sized(
                [130.0, 20.0],
                egui::Slider::new(&mut self.zoom_scale, 0.5..=2.0)
                    .step_by(0.1)
                    .show_value(false),
            )
            .on_hover_text(
                "0.5x shows the full selected note range. 1x is the default view; higher values zoom in.",
            );
            ui.label(zoom_label(self.zoom_scale));

            ui.label(
                RichText::new("LOW")
                    .size(10.0)
                    .strong()
                    .color(Color32::from_rgb(125, 145, 161)),
            );
            let mut lowest = self.graph_min_limit;
            egui::ComboBox::from_id_salt("graph_low_note")
                .selected_text(midi_to_note(lowest).1)
                .width(64.0)
                .show_ui(ui, |ui| {
                    for midi in 12..=60 {
                        ui.selectable_value(
                            &mut lowest,
                            midi as f32,
                            midi_to_note(midi as f32).1,
                        );
                    }
                });
            self.graph_min_limit = lowest;

            ui.label(
                RichText::new("HIGH")
                    .size(10.0)
                    .strong()
                    .color(Color32::from_rgb(125, 145, 161)),
            );
            let mut highest = self.graph_max_limit;
            egui::ComboBox::from_id_salt("graph_high_note")
                .selected_text(midi_to_note(highest).1)
                .width(64.0)
                .show_ui(ui, |ui| {
                    for midi in 84..=120 {
                        ui.selectable_value(
                            &mut highest,
                            midi as f32,
                            midi_to_note(midi as f32).1,
                        );
                    }
                });
            self.graph_max_limit = highest;
        });
        ui.add_space(8.0);
        let mut refresh_inputs = false;
        let panel_width = ui.available_width();
        ui.group(|ui| {
            ui.set_width((panel_width - 16.0).max(0.0));
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("MICROPHONE")
                        .size(10.0)
                        .strong()
                        .color(Color32::from_rgb(125, 145, 161)),
                );
                ui.horizontal(|ui| {
                    self.draw_input_meter(ui);
                });
                ui.horizontal_wrapped(|ui| {
                    self.draw_listening_button(ui);
                    let selected_name = self
                        .input_devices
                        .get(self.selected_input)
                        .map(|device| {
                            if device.is_default {
                                format!("{} (default)", device.name)
                            } else {
                                device.name.clone()
                            }
                        })
                        .unwrap_or_else(|| "No microphones found".to_owned());
                    let mut selected = self.selected_input;
                    egui::ComboBox::from_id_salt("microphone_input")
                        .selected_text(selected_name)
                        .width(180.0)
                        .show_ui(ui, |ui| {
                            for (index, device) in self.input_devices.iter().enumerate() {
                                let label = if device.is_default {
                                    format!("{} (default)", device.name)
                                } else {
                                    device.name.clone()
                                };
                                ui.selectable_value(&mut selected, index, label);
                            }
                        });
                    if selected != self.selected_input {
                        self.selected_input = selected;
                        if self.microphone.is_running() || self.microphone.is_starting() {
                            self.microphone.stop();
                            self.start_listening();
                        }
                    }
                    if ui.button("Refresh Microphones").clicked() {
                        refresh_inputs = true;
                    }
                    if self.microphone.is_starting() {
                        ui.label(
                            RichText::new("STARTING")
                                .size(10.0)
                                .color(Color32::from_rgb(175, 125, 45)),
                        );
                    } else if self.microphone.is_running() {
                        ui.label(
                            RichText::new("● LIVE")
                                .size(11.0)
                                .strong()
                                .color(Color32::from_rgb(55, 180, 125)),
                        );
                    } else {
                        ui.label(
                            RichText::new("READY")
                                .size(10.0)
                                .color(Color32::from_rgb(125, 145, 161)),
                        );
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("PITCH SENSITIVITY")
                            .size(10.0)
                            .strong()
                            .color(Color32::from_rgb(125, 145, 161)),
                    );
                    ui.add_sized(
                        [210.0, 20.0],
                        egui::Slider::new(&mut self.sensitivity, 1.0..=100.0).show_value(false),
                    )
                    .on_hover_text(
                        "Higher sensitivity lowers the detection threshold for quieter singing.",
                    );
                    ui.label(format!("{:.0}%", self.sensitivity));
                });
            });
        });
        if refresh_inputs {
            self.refresh_input_devices();
            self.last_device_refresh = Instant::now();
        }
    }

    fn draw_listening_button(&mut self, ui: &mut egui::Ui) {
        let listening = self.microphone.is_running();
        let starting = self.microphone.is_starting();
        let available = listening || starting || !self.input_devices.is_empty();
        let sense = if available && !starting {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        };
        let (rect, response) = ui.allocate_exact_size(Vec2::new(178.0, 44.0), sense);
        let color = if !available {
            Color32::from_rgb(105, 105, 105)
        } else if starting {
            Color32::from_rgb(175, 125, 45)
        } else if listening {
            Color32::from_rgb(190, 64, 68)
        } else {
            Color32::from_rgb(42, 145, 91)
        };
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 7.0, color);

        let icon_center = egui::pos2(rect.left() + 24.0, rect.center().y);
        if listening {
            painter.rect_filled(
                egui::Rect::from_center_size(icon_center, Vec2::splat(13.0)),
                1.0,
                Color32::WHITE,
            );
        } else if !starting {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(icon_center.x - 6.0, icon_center.y - 9.0),
                    egui::pos2(icon_center.x - 6.0, icon_center.y + 9.0),
                    egui::pos2(icon_center.x + 10.0, icon_center.y),
                ],
                Color32::WHITE,
                Stroke::NONE,
            ));
        } else {
            painter.circle_filled(icon_center, 5.0, Color32::WHITE);
        }
        painter.text(
            egui::pos2(rect.center().x + 15.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            if starting {
                "Starting..."
            } else if listening {
                "Stop Listening"
            } else {
                "Start Listening"
            },
            FontId::proportional(14.0),
            Color32::WHITE,
        );

        if response.clicked() && !starting {
            self.error = None;
            if listening {
                self.microphone.stop();
                self.reading = None;
                self.history.clear();
                self.recent_pitch.clear();
                self.input_level_rms = 0.0;
                self.input_peak = 0.0;
            } else {
                self.start_listening();
            }
        }
        response.on_hover_text(if starting {
            "Opening microphone"
        } else if !available {
            "No microphone devices are available"
        } else if listening {
            "Stop microphone input"
        } else {
            "Start live pitch tracking"
        });
    }

    fn draw_input_meter(&self, ui: &mut egui::Ui) {
        let rms_db = 20.0 * self.input_level_rms.max(0.00001).log10();
        let level = ((rms_db + 60.0) / 60.0).clamp(0.0, 1.0);
        let color = if self.input_peak >= 0.95 {
            Color32::from_rgb(237, 86, 91)
        } else if self.input_peak >= 0.65 {
            Color32::from_rgb(238, 183, 63)
        } else {
            Color32::from_rgb(54, 190, 132)
        };
        let (rect, _) = ui.allocate_exact_size(Vec2::new(104.0, 14.0), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 3.0, Color32::from_rgb(44, 54, 65));
        if level > 0.0 {
            let filled = egui::Rect::from_min_max(
                rect.min,
                egui::pos2(rect.left() + rect.width() * level, rect.bottom()),
            );
            painter.rect_filled(filled, 3.0, color);
        }
        ui.label(
            RichText::new(if !self.microphone.is_running() {
                "OFF".to_owned()
            } else if self.input_peak >= 0.98 {
                "CLIP".to_owned()
            } else {
                format!("{rms_db:.0} dB")
            })
            .size(9.0)
            .strong()
            .color(if self.microphone.is_running() {
                color
            } else {
                Color32::from_rgb(125, 145, 161)
            }),
        );
    }

    fn draw_track_controls(&mut self, ui: &mut egui::Ui) {
        let panel_width = ui.available_width();
        ui.group(|ui| {
            ui.set_width((panel_width - 16.0).max(0.0));
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("BACKING TRACK")
                        .size(10.0)
                        .strong()
                        .color(Color32::from_rgb(125, 145, 161)),
                );
                ui.horizontal_wrapped(|ui| {
                    if let Some(status) = &self.audio_load_status {
                        match status {
                            AudioLoadStatus::ChoosingFile => {
                                ui.label(
                                    RichText::new("Choose an audio file…")
                                        .size(12.0)
                                        .color(Color32::from_rgb(125, 145, 161)),
                                );
                            }
                            AudioLoadStatus::Analyzing(name) => {
                                ui.spinner();
                                ui.label(
                                    RichText::new(format!("Analyzing {name}…"))
                                        .size(12.0)
                                        .color(Color32::from_rgb(125, 145, 161)),
                                );
                            }
                        }
                    } else if let Some(path) = self.audio_file.clone() {
                        ui.label(
                            RichText::new(self.track_name.as_deref().unwrap_or("Audio track"))
                                .strong()
                                .size(13.0),
                        );
                        let playback_position = self.player.position().min(self.track_duration);
                        ui.label(
                            RichText::new(format!(
                                "{} / {}",
                                format_track_time(playback_position),
                                format_track_time(self.track_duration)
                            ))
                            .monospace()
                            .size(11.0)
                            .color(Color32::from_rgb(125, 145, 161)),
                        );
                        let play_label = if self.player.is_playing() {
                            "Pause"
                        } else if self.player.is_paused() {
                            "Resume"
                        } else {
                            "Play"
                        };
                        if ui.button(play_label).clicked() {
                            self.error = None;
                            let was_playing = self.player.is_playing();
                            let was_paused = self.player.is_paused();
                            let was_finished = self.player.is_finished();
                            if let Err(message) = self.player.toggle(&path) {
                                self.error = Some(message);
                            } else {
                                if !was_playing && !was_paused && was_finished {
                                    self.reset_timeline();
                                    self.metronome.reset_phase();
                                }
                                self.metronome.set_running(self.player.is_playing());
                            }
                        }
                        if ui.button("Stop").clicked() {
                            self.player.stop();
                            self.reset_timeline();
                            self.metronome.reset_phase();
                            self.metronome.set_running(false);
                            self.last_scrub_position = None;
                        }
                        if ui.button("Clear Track").clicked() {
                            self.player.stop();
                            self.audio_file = None;
                            self.track_name = None;
                            self.track_duration = Duration::ZERO;
                            self.track_waveform.clear();
                            self.loop_start = Duration::ZERO;
                            self.loop_end = Duration::ZERO;
                            self.dragging_loop_handle = None;
                            self.last_scrub_position = None;
                            self.detected_bpm = None;
                            self.bpm_overridden = false;
                            self.reset_timeline();
                            self.metronome.reset_phase();
                            self.metronome.set_running(true);
                        }
                        ui.label(
                            RichText::new("VOLUME")
                                .size(10.0)
                                .strong()
                                .color(Color32::from_rgb(125, 145, 161)),
                        );
                        let volume_slider = ui.add_sized(
                            [110.0, 20.0],
                            egui::Slider::new(&mut self.track_volume, 0.0..=1.0).show_value(false),
                        );
                        if volume_slider.changed() {
                            self.player.set_volume(self.track_volume);
                        }
                        ui.label(format!("{:.0}%", self.track_volume * 100.0));
                        if let Some(detected_bpm) = self.detected_bpm {
                            ui.label(
                                RichText::new(format!("Auto-detected {detected_bpm:.0} BPM"))
                                    .size(11.0)
                                    .color(Color32::from_rgb(125, 145, 161)),
                            );
                        } else {
                            ui.label(
                                RichText::new("No steady BPM detected · Set tempo above")
                                    .size(11.0)
                                    .color(Color32::from_rgb(125, 145, 161)),
                            );
                        }
                    } else {
                        ui.label(
                            RichText::new("No backing track loaded · Open audio from File")
                                .size(11.0)
                                .color(Color32::from_rgb(125, 145, 161)),
                        );
                    }
                });
            });
        });
    }

    fn draw_practice_controls(&mut self, ui: &mut egui::Ui) {
        let mut key_changed = false;
        let mut scale_changed = false;
        let mut root_octave_changed = false;
        let mut warmup_pattern_changed = false;
        let mut reference_changed = false;
        let mut reference_toggled = false;
        let mut metronome_changed = false;
        let mut stop_practice = false;
        let mut start_warmup = false;
        let mut start_echo = false;
        ui.add_space(8.0);
        let panel_width = ui.available_width();
        ui.group(|ui| {
            ui.set_width((panel_width - 16.0).max(0.0));
            ui.columns(2, |columns| {
                let column_width = columns[0].available_width();
                columns[0].group(|ui| {
                    ui.set_width((column_width - 16.0).max(0.0));
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("METRONOME").small().strong());
                        if ui
                            .checkbox(&mut self.metronome_enabled, "Enabled")
                            .changed()
                        {
                            metronome_changed = true;
                        }
                        ui.add_enabled(
                            self.metronome_enabled,
                            egui::Slider::new(&mut self.metronome_volume, 0.0..=1.0)
                                .show_value(false)
                                .text("Volume"),
                        );
                        ui.label(format!("{:.0}%", self.metronome_volume * 100.0));
                    });
                });
                let column_width = columns[1].available_width();
                columns[1].group(|ui| {
                    ui.set_width((column_width - 16.0).max(0.0));
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("KEY & SCALE").small().strong());
                        egui::ComboBox::from_id_salt("song_key")
                            .selected_text(KEY_NAMES[self.key_root])
                            .width(54.0)
                            .show_ui(ui, |ui| {
                                for (index, key) in KEY_NAMES.iter().enumerate() {
                                    key_changed |= ui
                                        .selectable_value(&mut self.key_root, index, *key)
                                        .changed();
                                }
                            });
                        let previous_octave = self.root_octave;
                        egui::ComboBox::from_id_salt("key_root_octave")
                            .selected_text(format!(
                                "{}{}",
                                KEY_NAMES[self.key_root], self.root_octave
                            ))
                            .width(58.0)
                            .show_ui(ui, |ui| {
                                for octave in 1..=6 {
                                    ui.selectable_value(
                                        &mut self.root_octave,
                                        octave,
                                        format!("{}{octave}", KEY_NAMES[self.key_root]),
                                    );
                                }
                            })
                            .response
                            .on_hover_text(
                                "Shared root octave for reference tones and practice routines.",
                            );
                        root_octave_changed = previous_octave != self.root_octave;
                        egui::ComboBox::from_id_salt("song_scale")
                            .selected_text(self.scale.label())
                            .width(124.0)
                            .show_ui(ui, |ui| {
                                for scale in [
                                    Scale::Major,
                                    Scale::Minor,
                                    Scale::MajorPentatonic,
                                    Scale::MinorPentatonic,
                                ] {
                                    scale_changed |= ui
                                        .selectable_value(&mut self.scale, scale, scale.label())
                                        .changed();
                                }
                            });
                    });
                });
            });

            ui.add_space(6.0);
            ui.columns(3, |columns| {
                let column_width = columns[0].available_width();
                columns[0].group(|ui| {
                    ui.set_width((column_width - 16.0).max(0.0));
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("REFERENCE TONE").small().strong());
                        if ui.checkbox(&mut self.reference_enabled, "Play").changed() {
                            reference_toggled = true;
                        }
                        egui::ComboBox::from_id_salt("reference_kind")
                            .selected_text(match self.reference_kind {
                                ReferenceKind::Root => "Root",
                                ReferenceKind::Fifth => "Fifth",
                            })
                            .width(100.0)
                            .show_ui(ui, |ui| {
                                for (kind, label) in [
                                    (ReferenceKind::Root, "Root"),
                                    (ReferenceKind::Fifth, "Fifth"),
                                ] {
                                    reference_changed |= ui
                                        .selectable_value(&mut self.reference_kind, kind, label)
                                        .changed();
                                }
                            });
                    });
                });
                let column_width = columns[1].available_width();
                columns[1].group(|ui| {
                    ui.set_width((column_width - 16.0).max(0.0));
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("WARM-UP").small().strong())
                            .on_hover_text(
                                "Warm-ups start from the shared Key & Scale root and octave.",
                            );
                        ui.label(
                            RichText::new(format!(
                                "{}{}",
                                KEY_NAMES[self.key_root], self.root_octave
                            ))
                            .size(11.0)
                            .color(Color32::from_rgb(125, 145, 161)),
                        );
                        egui::ComboBox::from_id_salt("warmup_pattern")
                            .selected_text(self.warmup_pattern.label())
                            .width(112.0)
                            .show_ui(ui, |ui| {
                                for pattern in [
                                    WarmupPattern::FiveTone,
                                    WarmupPattern::Octave,
                                    WarmupPattern::Arpeggio,
                                    WarmupPattern::Siren,
                                ] {
                                    warmup_pattern_changed |= ui
                                        .selectable_value(
                                            &mut self.warmup_pattern,
                                            pattern,
                                            pattern.label(),
                                        )
                                        .changed();
                                }
                            });
                        if self.warmup.is_some() {
                            if ui.button("Stop Warm-Up").clicked() {
                                stop_practice = true;
                            }
                        } else if ui.button("Start Warm-Up").clicked() {
                            start_warmup = true;
                        }
                    });
                });
                let column_width = columns[2].available_width();
                columns[2].group(|ui| {
                    ui.set_width((column_width - 16.0).max(0.0));
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("PITCH ECHO").small().strong());
                        if self.echo.is_some() {
                            if ui.button("Stop Echo").clicked() {
                                stop_practice = true;
                            }
                        } else if ui.button("Play Phrase").clicked() {
                            start_echo = true;
                        }
                        if let Some(session) = &self.echo {
                            let status = match session.phase {
                                EchoPhase::Countdown => echo_countdown_value(
                                    Instant::now().duration_since(session.started_at),
                                )
                                .map(|number| format!("Get ready: {number}"))
                                .unwrap_or_else(|| "Starting phrase…".to_owned()),
                                EchoPhase::Prompting => "Listen, then sing it back".to_owned(),
                                EchoPhase::Responding => "Sing the 3-note phrase".to_owned(),
                            };
                            ui.label(status);
                        } else if let Some(score) = self.echo_score {
                            ui.label(format!("Match: {score:.0}%"));
                        }
                    });
                });
            });
        });

        let root_changed = key_changed || root_octave_changed;
        let warmup_config_changed = root_changed || scale_changed || warmup_pattern_changed;
        if warmup_config_changed && self.warmup.is_some() {
            self.stop_warmup();
        }
        if (root_changed || scale_changed) && self.echo.is_some() {
            self.stop_echo();
        }
        if root_changed && self.reference_enabled {
            self.start_reference_tone();
        }
        if metronome_changed
            && let Err(message) = self.metronome.set_enabled(self.metronome_enabled)
        {
            self.metronome_enabled = false;
            self.error = Some(message);
        }
        if reference_toggled || reference_changed {
            if self.reference_enabled {
                self.stop_warmup();
                self.stop_echo();
                self.start_reference_tone();
            } else {
                self.tone_player.stop();
            }
        }
        if stop_practice {
            self.stop_warmup();
            self.stop_echo();
        }
        if start_warmup {
            self.start_warmup();
        } else if start_echo {
            self.start_echo();
        }
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
            self.echo_notes = None;
            self.tone_player.stop();
            self.history.clear();
        }
    }

    fn draw_track_waveform(&mut self, ui: &mut egui::Ui, dark: bool) {
        if self.audio_file.is_none()
            || self.track_waveform.is_empty()
            || self.track_duration.is_zero()
        {
            return;
        }

        ui.add_space(12.0);
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), 78.0),
            egui::Sense::click_and_drag(),
        );
        let painter = ui.painter_at(rect);
        let background = if dark {
            Color32::from_rgb(20, 28, 37)
        } else {
            Color32::from_rgb(247, 249, 251)
        };
        let wave_color = if dark {
            Color32::from_rgb(255, 190, 75)
        } else {
            Color32::from_rgb(158, 87, 14)
        };
        let progress_fill = Color32::from_rgba_unmultiplied(242, 166, 45, 28);
        let handle_color = if dark {
            Color32::WHITE
        } else {
            Color32::from_rgb(48, 48, 48)
        };
        let outside_loop = if dark {
            Color32::from_rgba_unmultiplied(0, 0, 0, 85)
        } else {
            Color32::from_rgba_unmultiplied(0, 0, 0, 35)
        };
        painter.rect_filled(rect, 7.0, background);

        let waveform_rect = rect.shrink2(Vec2::new(8.0, 6.0));
        let current_position = self.player.position().min(self.track_duration);
        let progress = (current_position.as_secs_f64() / self.track_duration.as_secs_f64())
            .clamp(0.0, 1.0) as f32;
        let playhead_x = waveform_rect.left() + waveform_rect.width() * progress;
        let loop_start_fraction = (self.loop_start.as_secs_f64()
            / self.track_duration.as_secs_f64())
        .clamp(0.0, 1.0) as f32;
        let loop_end_fraction = (self.loop_end.as_secs_f64() / self.track_duration.as_secs_f64())
            .clamp(0.0, 1.0) as f32;
        let loop_start_x = waveform_rect.left() + waveform_rect.width() * loop_start_fraction;
        let loop_end_x = waveform_rect.left() + waveform_rect.width() * loop_end_fraction;
        if loop_start_x > waveform_rect.left() {
            painter.rect_filled(
                egui::Rect::from_min_max(
                    waveform_rect.left_top(),
                    egui::pos2(loop_start_x, waveform_rect.bottom()),
                ),
                0.0,
                outside_loop,
            );
        }
        if loop_end_x < waveform_rect.right() {
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(loop_end_x, waveform_rect.top()),
                    waveform_rect.right_bottom(),
                ),
                0.0,
                outside_loop,
            );
        }
        painter.rect_filled(
            egui::Rect::from_min_max(
                waveform_rect.left_top(),
                egui::pos2(playhead_x, waveform_rect.bottom()),
            ),
            0.0,
            progress_fill,
        );

        let center_y = waveform_rect.center().y;
        let half_height = waveform_rect.height() * 0.44;
        let bin_width = waveform_rect.width() / self.track_waveform.len() as f32;
        painter.line_segment(
            [
                egui::pos2(waveform_rect.left(), center_y),
                egui::pos2(waveform_rect.right(), center_y),
            ],
            Stroke::new(0.5_f32, wave_color),
        );
        for (index, amplitude) in self.track_waveform.iter().copied().enumerate() {
            let x = waveform_rect.left() + (index as f32 + 0.5) * bin_width;
            let extent = amplitude * half_height;
            painter.line_segment(
                [
                    egui::pos2(x, center_y - extent),
                    egui::pos2(x, center_y + extent),
                ],
                Stroke::new(bin_width.clamp(0.6_f32, 1.5_f32), wave_color),
            );
        }
        painter.line_segment(
            [
                egui::pos2(loop_start_x, waveform_rect.top()),
                egui::pos2(loop_start_x, waveform_rect.bottom()),
            ],
            Stroke::new(2.0_f32, handle_color),
        );
        painter.line_segment(
            [
                egui::pos2(loop_end_x, waveform_rect.top()),
                egui::pos2(loop_end_x, waveform_rect.bottom()),
            ],
            Stroke::new(2.0_f32, handle_color),
        );
        for (x, label, is_start) in [(loop_start_x, "A", true), (loop_end_x, "B", false)] {
            let label_x = if is_start { x + 7.0 } else { x - 7.0 };
            let align = if is_start {
                egui::Align2::LEFT_TOP
            } else {
                egui::Align2::RIGHT_TOP
            };
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(x - 5.0, waveform_rect.top()),
                    egui::pos2(x + 5.0, waveform_rect.top()),
                    egui::pos2(x, waveform_rect.top() + 8.0),
                ],
                handle_color,
                Stroke::NONE,
            ));
            painter.text(
                egui::pos2(label_x, waveform_rect.top() + 1.0),
                align,
                label,
                FontId::monospace(9.0),
                handle_color,
            );
        }
        painter.line_segment(
            [
                egui::pos2(playhead_x, waveform_rect.top()),
                egui::pos2(playhead_x, waveform_rect.bottom()),
            ],
            Stroke::new(2.0_f32, wave_color),
        );

        let hover_pointer = response.hover_pos().filter(|pointer| {
            self.dragging_loop_handle.is_none()
                && should_show_scrub_preview(pointer.x, loop_start_x, loop_end_x)
        });
        if let Some(pointer) = hover_pointer {
            let x = pointer.x.clamp(waveform_rect.left(), waveform_rect.right());
            let fraction = (x - waveform_rect.left()) / waveform_rect.width();
            let hover_time =
                Duration::from_secs_f64(self.track_duration.as_secs_f64() * fraction as f64);
            let cursor_color = if dark {
                Color32::from_rgba_unmultiplied(255, 255, 255, 105)
            } else {
                Color32::from_rgba_unmultiplied(25, 25, 25, 105)
            };
            painter.line_segment(
                [
                    egui::pos2(x, waveform_rect.top()),
                    egui::pos2(x, waveform_rect.bottom()),
                ],
                Stroke::new(1.0_f32, cursor_color),
            );
            let label_width = 56.0;
            let label_rect = egui::Rect::from_center_size(
                egui::pos2(
                    x.clamp(
                        waveform_rect.left() + label_width * 0.5,
                        waveform_rect.right() - label_width * 0.5,
                    ),
                    waveform_rect.top() + 11.0,
                ),
                Vec2::new(label_width, 18.0),
            );
            painter.rect_filled(
                label_rect,
                3.0,
                if dark {
                    Color32::from_rgba_unmultiplied(0, 0, 0, 190)
                } else {
                    Color32::from_rgba_unmultiplied(255, 255, 255, 220)
                },
            );
            painter.text(
                label_rect.center(),
                egui::Align2::CENTER_CENTER,
                format_track_time(hover_time),
                FontId::monospace(10.0),
                handle_color,
            );
        }

        if response.drag_started() {
            self.last_scrub_position = None;
            self.dragging_loop_handle = response.interact_pointer_pos().and_then(|pointer| {
                let start_distance = (pointer.x - loop_start_x).abs();
                let end_distance = (pointer.x - loop_end_x).abs();
                if start_distance <= LOOP_HANDLE_HIT_RADIUS && start_distance <= end_distance {
                    Some(LoopHandle::Start)
                } else if end_distance <= LOOP_HANDLE_HIT_RADIUS {
                    Some(LoopHandle::End)
                } else {
                    None
                }
            });
        }
        if (response.clicked()
            || response.drag_started()
            || response.dragged()
            || response.drag_stopped())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let fraction =
                ((pointer.x - waveform_rect.left()) / waveform_rect.width()).clamp(0.0, 1.0);
            let target =
                Duration::from_secs_f64(self.track_duration.as_secs_f64() * fraction as f64);
            if let Some(handle) = self.dragging_loop_handle {
                (self.loop_start, self.loop_end) =
                    move_loop_handle(self.loop_start, self.loop_end, target, handle);
            } else {
                let near_handle = (pointer.x - loop_start_x).abs() <= LOOP_HANDLE_HIT_RADIUS
                    || (pointer.x - loop_end_x).abs() <= LOOP_HANDLE_HIT_RADIUS;
                let should_seek = !near_handle
                    && (response.clicked()
                        || response.drag_started()
                        || self
                            .last_scrub_position
                            .map(|last| (target.as_secs_f64() - last.as_secs_f64()).abs() >= 0.02)
                            .unwrap_or(true));
                if should_seek && self.seek_to(target) {
                    self.last_scrub_position = Some(target);
                }
            }
        }
        if response.drag_stopped() {
            self.dragging_loop_handle = None;
            self.last_scrub_position = None;
        } else if response.clicked() {
            self.last_scrub_position = None;
        }
        response.context_menu(|ui| {
            if ui.button("Reset Start Loop").clicked() {
                self.loop_start = Duration::ZERO;
                ui.close_menu();
            }
            if ui.button("Reset End Loop").clicked() {
                self.loop_end = self.track_duration;
                ui.close_menu();
            }
            if ui.button("Reset Both Loops").clicked() {
                self.loop_start = Duration::ZERO;
                self.loop_end = self.track_duration;
                ui.close_menu();
            }
        });
        response.on_hover_text("Click or drag to seek; drag A/B handles to set the loop");
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
        self.echo_score = None;
        self.echo_notes = None;
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

    fn draw_menu_bar(&mut self, ctx: &egui::Context) -> bool {
        let mut open_audio = false;
        egui::TopBottomPanel::top("application_menu").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui
                        .add_enabled(self.audio_load.is_none(), egui::Button::new("Open Audio…"))
                        .clicked()
                    {
                        open_audio = true;
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Settings", |ui| {
                    ui.menu_button("Appearance", |ui| {
                        ui.selectable_value(&mut self.theme, ThemeChoice::System, "System");
                        ui.selectable_value(&mut self.theme, ThemeChoice::Light, "Light");
                        ui.selectable_value(&mut self.theme, ThemeChoice::Dark, "Dark");
                    });
                    ui.separator();
                    ui.label("Key / Scale Highlight Opacity");
                    ui.add(
                        egui::Slider::new(&mut self.highlight_opacity, 0.0..=0.5).show_value(false),
                    );
                    ui.label(format!("{:.0}%", self.highlight_opacity * 100.0));
                });
                ui.menu_button("Help", |ui| {
                    if ui.button("About PitchHelper…").clicked() {
                        self.about_open = true;
                    }
                });
            });
        });
        open_audio
    }

    fn draw_about(&mut self, ctx: &egui::Context) {
        egui::Window::new("About PitchHelper")
            .resizable(false)
            .open(&mut self.about_open)
            .default_width(380.0)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("PitchHelper").size(22.0).strong());
                    ui.label(
                        RichText::new(format!("version {}", env!("CARGO_PKG_VERSION")))
                            .size(11.0)
                            .color(Color32::from_rgb(125, 145, 161)),
                    );
                });
                ui.add_space(10.0);
                ui.label("A live pitch coach for singers.");
                ui.add_space(8.0);

                ui.strong("What it does");
                ui.label("•  Tracks your live pitch — closest note and cents of tuning.");
                ui.label("•  Plots your pitch over a moving four-bar 4/4 timeline.");
                ui.label("•  Plays a backing track (WAV/MP3/FLAC/OGG) with an auto-detected tempo, a metronome, and a seekable waveform.");
                ui.label("•  Practice tools: key/scale highlighting, a reference tone, warm-ups, and a pitch-echo game.");
                ui.label("•  Tracks your vocal range and tessitura over time.");
                ui.add_space(8.0);

                ui.strong("Reading the display");
                ui.label("•  Green = in tune (within 15 cents)   ·   Red = flat   ·   Blue = sharp");
                ui.add_space(8.0);

                ui.strong("Tips");
                ui.label("•  Use the Pitch Sensitivity slider if the app can't hear quieter singing.");
                ui.label("•  Choose a Key and Scale to highlight in-scale notes; chord tones are emphasized.");
                ui.label("•  Use Zoom, LOW and HIGH to shape the graph.");
                ui.label("•  Right-click the waveform to reset the A/B loop handles.");
                ui.add_space(10.0);

                ui.weak(COPYRIGHT);
            });
    }

    fn draw_pitch_readout(&self, ui: &mut egui::Ui) {
        let (note, cents, color) = match self.reading {
            Some(reading) => {
                let (_, note, cents) = midi_to_note(reading.midi);
                (note, cents, pitch_color(cents))
            }
            None => ("--".to_owned(), 0.0, Color32::from_rgb(125, 145, 161)),
        };
        ui.horizontal_wrapped(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("CLOSEST NOTE")
                        .size(10.0)
                        .strong()
                        .color(Color32::from_rgb(125, 145, 161)),
                );
                ui.label(RichText::new(note).size(48.0).strong().color(color));
                if let Some(reading) = self.reading {
                    if let Some(alternate) = pitch::enharmonic_name(reading.midi) {
                        ui.label(
                            RichText::new(format!("also {alternate}"))
                                .size(11.0)
                                .color(Color32::from_rgb(125, 145, 161)),
                        );
                    }
                    ui.label(
                        RichText::new(format!("{:.1} Hz", reading.frequency))
                            .size(11.0)
                            .color(Color32::from_rgb(125, 145, 161)),
                    );
                }
            });
            ui.add_space(24.0);
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("PITCH DEVIATION")
                        .size(10.0)
                        .strong()
                        .color(Color32::from_rgb(125, 145, 161)),
                );
                let label = if self.reading.is_none() {
                    "Waiting for audio".to_owned()
                } else if cents.abs() <= CENTS_TOLERANCE {
                    "In tune".to_owned()
                } else {
                    format!("{:+.0} cents", cents)
                };
                ui.label(RichText::new(label).size(19.0).strong().color(color));
            });
            ui.add_space(20.0);
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("VIBRATO / STABILITY")
                        .size(10.0)
                        .strong()
                        .color(Color32::from_rgb(125, 145, 161)),
                );
                let metrics = pitch_metrics(
                    &self.recent_pitch.iter().copied().collect::<Vec<_>>(),
                    Instant::now(),
                );
                let label = match metrics {
                    Some(metrics) => match metrics.vibrato_hz {
                        Some(rate) if (3.0..=9.0).contains(&rate) => {
                            format!(
                                "Vibrato {rate:.1} Hz · {:.0} cents",
                                metrics.width_semitones * 100.0
                            )
                        }
                        _ if metrics.steady => "Steady pitch".to_owned(),
                        _ => format!("Pitch width {:.0} cents", metrics.width_semitones * 100.0),
                    },
                    None => "Hold a note".to_owned(),
                };
                ui.label(RichText::new(label).size(15.0).strong());
                let range_text = match (self.range_tracker.low, self.range_tracker.high) {
                    (Some(low), Some(high)) => {
                        let (tessitura_low, tessitura_high) =
                            self.range_tracker.tessitura().unwrap_or((low, high));
                        format!(
                            "Range {}–{} · Tessitura {}–{}",
                            midi_to_note(low as f32).1,
                            midi_to_note(high as f32).1,
                            midi_to_note(tessitura_low as f32).1,
                            midi_to_note(tessitura_high as f32).1
                        )
                    }
                    _ => "Range tracker: sing steady notes".to_owned(),
                };
                ui.label(
                    RichText::new(range_text)
                        .size(10.0)
                        .color(Color32::from_rgb(125, 145, 161)),
                );
            });
        });
    }

    fn draw_graph(&self, ui: &mut egui::Ui, dark: bool) {
        let available_width = ui.available_width();
        let available_height = ui.available_height().max(120.0);
        let (rect, _) = ui.allocate_exact_size(
            Vec2::new(available_width, available_height),
            egui::Sense::hover(),
        );
        let painter = ui.painter_at(rect);
        let background = if dark {
            Color32::from_rgb(20, 28, 37)
        } else {
            Color32::from_rgb(247, 249, 251)
        };
        let grid = if dark {
            Color32::from_rgb(41, 52, 64)
        } else {
            Color32::from_rgb(222, 229, 235)
        };
        let muted = if dark {
            Color32::from_rgb(130, 147, 162)
        } else {
            Color32::from_rgb(105, 121, 135)
        };
        painter.rect_filled(rect, 9.0, background);

        let plot = egui::Rect::from_min_max(
            rect.min + Vec2::new(55.0, 15.0),
            rect.max - Vec2::new(14.0, 28.0),
        );
        let (low_midi, high_midi) = self.visible_graph_range();
        if let Some((tessitura_low, tessitura_high)) = self.range_tracker.tessitura()
            && tessitura_high as f32 >= low_midi
            && tessitura_low as f32 <= high_midi
        {
            let visible_low = (tessitura_low as f32).max(low_midi);
            let visible_high = (tessitura_high as f32).min(high_midi);
            let top =
                plot.bottom() - (visible_high - low_midi) / (high_midi - low_midi) * plot.height();
            let bottom =
                plot.bottom() - (visible_low - low_midi) / (high_midi - low_midi) * plot.height();
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(plot.left(), top),
                    egui::pos2(plot.right(), bottom),
                ),
                0.0,
                Color32::from_rgba_unmultiplied(120, 115, 195, if dark { 6 } else { 7 }),
            );
        }
        let row_height = plot.height() / (high_midi - low_midi);
        for midi in low_midi.floor() as i32..=high_midi.ceil() as i32 {
            if !(low_midi..=high_midi).contains(&(midi as f32)) {
                continue;
            }
            let interval = (midi - self.key_root as i32).rem_euclid(12);
            if self.scale.intervals().contains(&interval) {
                let top = plot.bottom() - (midi as f32 + 0.5 - low_midi) * row_height;
                let bottom = plot.bottom() - (midi as f32 - 0.5 - low_midi) * row_height;
                let chord_tone = self.scale.chord_intervals().contains(&interval);
                let opacity = if chord_tone {
                    self.highlight_opacity * 1.3
                } else {
                    self.highlight_opacity
                }
                .clamp(0.0, 0.9);
                let alpha = (opacity * 255.0).round() as u8;
                painter.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(plot.left(), top.max(plot.top())),
                        egui::pos2(plot.right(), bottom.min(plot.bottom())),
                    ),
                    0.0,
                    Color32::from_rgba_unmultiplied(110, 145, 128, alpha),
                );
            }
        }
        painter.text(
            egui::pos2(plot.right(), rect.top() + 7.0),
            egui::Align2::RIGHT_CENTER,
            format!(
                "{} {} · brighter rows are chord tones",
                KEY_NAMES[self.key_root],
                self.scale.label()
            ),
            FontId::proportional(9.0),
            muted,
        );
        let label_start = low_midi.ceil() as i32;
        let label_end = high_midi.floor() as i32;
        for midi in (label_start..=label_end).step_by(3) {
            let y =
                plot.bottom() - (midi as f32 - low_midi) / (high_midi - low_midi) * plot.height();
            painter.line_segment(
                [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
                Stroke::new(if midi % 12 == 0 { 1.2_f32 } else { 0.7_f32 }, grid),
            );
            let (_, name, _) = midi_to_note(midi as f32);
            painter.text(
                egui::pos2(rect.left() + 10.0, y),
                egui::Align2::LEFT_CENTER,
                name,
                FontId::monospace(10.0),
                muted,
            );
        }

        for bar in 0..=VISIBLE_BARS as usize {
            let x = plot.left() + bar as f32 / VISIBLE_BARS * plot.width();
            painter.line_segment(
                [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
                Stroke::new(1.0_f32, grid),
            );
            if bar < VISIBLE_BARS as usize {
                painter.text(
                    egui::pos2(x + 7.0, plot.bottom() + 15.0),
                    egui::Align2::LEFT_CENTER,
                    format!("BAR {}", bar + 1),
                    FontId::proportional(9.0),
                    muted,
                );
            }
        }
        for beat in 0..(VISIBLE_BARS as usize * BAR_BEATS as usize) {
            if beat % BAR_BEATS as usize == 0 {
                continue;
            }
            let x = plot.left() + beat as f32 / (VISIBLE_BARS * BAR_BEATS) * plot.width();
            painter.line_segment(
                [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
                Stroke::new(0.55_f32, grid),
            );
        }

        let points: Vec<_> = self
            .history
            .iter()
            .filter_map(|(phase, midi)| {
                if !(low_midi..=high_midi).contains(midi) {
                    return None;
                }
                Some(egui::pos2(
                    plot.left() + phase * plot.width(),
                    plot.bottom() - (midi - low_midi) / (high_midi - low_midi) * plot.height(),
                ))
            })
            .collect();
        for pair in points.windows(2) {
            let average_midi = (pair[0].y + pair[1].y) * 0.5;
            let midi_at_y =
                high_midi - (average_midi - plot.top()) / plot.height() * (high_midi - low_midi);
            let cents = (midi_at_y - midi_at_y.round()) * 100.0;
            painter.line_segment([pair[0], pair[1]], Stroke::new(2.4_f32, pitch_color(cents)));
        }

        let now = Instant::now();
        if let Some(session) = &self.warmup {
            draw_target_notes(
                &painter,
                plot,
                low_midi,
                high_midi,
                &session.notes,
                Color32::from_rgb(248, 191, 76),
            );
        } else if let Some(session) = &self.echo {
            draw_target_notes(
                &painter,
                plot,
                low_midi,
                high_midi,
                &session.notes,
                Color32::from_rgb(105, 208, 190),
            );
        } else if self.echo_score.is_some()
            && let Some(notes) = &self.echo_notes
        {
            draw_target_notes(
                &painter,
                plot,
                low_midi,
                high_midi,
                notes,
                Color32::from_rgb(105, 208, 190),
            );
        } else if self.reference_enabled {
            let note = [self.reference_midi()];
            draw_target_notes(
                &painter,
                plot,
                low_midi,
                high_midi,
                &note,
                Color32::from_rgb(248, 191, 76),
            );
        }

        if let Some(session) = &self.echo
            && session.phase == EchoPhase::Countdown
            && let Some(number) = echo_countdown_value(now.duration_since(session.started_at))
        {
            painter.circle_filled(
                plot.center(),
                48.0,
                Color32::from_rgba_unmultiplied(15, 20, 25, 205),
            );
            painter.text(
                plot.center(),
                egui::Align2::CENTER_CENTER,
                number.to_string(),
                FontId::proportional(76.0),
                Color32::from_rgb(248, 191, 76),
            );
        }

        let phase = if self.echo_score.is_some() {
            1.0
        } else if let Some(session) = &self.echo {
            match session.phase {
                EchoPhase::Countdown | EchoPhase::Prompting => 0.0,
                EchoPhase::Responding => practice_phase(
                    now,
                    session.started_at,
                    session.note_duration,
                    session.notes.len(),
                ),
            }
        } else if let Some(session) = &self.warmup {
            practice_phase(
                now,
                session.started_at,
                session.note_duration,
                session.notes.len(),
            )
        } else {
            self.measure_phase
        };
        let playhead_x = plot.left() + phase * plot.width();
        painter.line_segment(
            [
                egui::pos2(playhead_x, plot.top()),
                egui::pos2(playhead_x, plot.bottom()),
            ],
            Stroke::new(1.5_f32, Color32::from_rgb(80, 156, 255)),
        );
        painter.circle_filled(
            egui::pos2(playhead_x, plot.top() + 4.0),
            3.0,
            Color32::from_rgb(80, 156, 255),
        );
        if self.history.is_empty() && self.warmup.is_none() && self.echo.is_none() {
            painter.text(
                plot.center(),
                egui::Align2::CENTER_CENTER,
                "Start listening to see your pitch",
                FontId::proportional(14.0),
                muted,
            );
        }
    }
}

impl eframe::App for PitchHelperApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = Instant::now();
        let delta = now.duration_since(self.last_frame).as_secs_f64();
        self.last_frame = now;
        self.update_audio_load();
        self.update_track_loop();
        if self.audio_file.is_some() {
            let song_position = self.player.position().as_secs_f64();
            self.beat_position = advance_song_beat_position(
                self.beat_position,
                self.last_song_position,
                song_position,
                self.bpm,
            );
            self.last_song_position = song_position;
        } else {
            self.beat_position = advance_beat_position(self.beat_position, delta, self.bpm);
        }
        let cycle = (self.beat_position / beats_per_cycle()).floor() as u64;
        if cycle != self.last_cycle {
            if self.warmup.is_none() && self.echo.is_none() && self.echo_score.is_none() {
                self.history.clear();
            }
            self.last_cycle = cycle;
        }
        self.measure_phase = measure_phase(self.beat_position);
        if self.last_device_refresh.elapsed() >= Duration::from_secs(2) {
            self.refresh_input_devices();
            self.last_device_refresh = now;
        }
        self.update_audio();
        self.update_practice(now);
        self.update_graph_view();
        self.save_range_if_needed(now);
        if let Some(error) = self.microphone.take_error() {
            self.error = Some(error);
            self.reading = None;
            self.history.clear();
            self.recent_pitch.clear();
        }
        self.metronome.set_bpm(self.bpm);
        self.metronome.set_volume(self.metronome_volume);
        self.metronome
            .set_running(self.audio_file.is_none() || self.player.is_playing());

        if self.theme != self.applied_theme
            || self.last_theme_check.elapsed() >= Duration::from_secs(1)
        {
            let dark = theme_is_dark(self.theme);
            if dark != self.applied_dark {
                set_theme(ctx, dark);
                self.applied_dark = dark;
            }
            self.applied_theme = self.theme;
            self.last_theme_check = now;
        }

        let open_audio = self.draw_menu_bar(ctx);
        if open_audio {
            self.choose_audio_file();
        }

        let (footer_background, footer_divider, footer_text) = if self.applied_dark {
            (
                Color32::from_rgb(20, 20, 20),
                Color32::from_rgb(60, 60, 60),
                Color32::from_rgb(180, 180, 180),
            )
        } else {
            (
                Color32::from_rgb(236, 236, 236),
                Color32::from_rgb(205, 205, 205),
                Color32::from_rgb(105, 105, 105),
            )
        };
        egui::TopBottomPanel::bottom("status_footer")
            .frame(egui::Frame::new().fill(footer_background))
            .show(ctx, |ui| {
                ui.set_min_height(32.0);
                let footer_rect = ui.max_rect();
                ui.painter().line_segment(
                    [
                        egui::pos2(footer_rect.left(), footer_rect.top()),
                        egui::pos2(footer_rect.right(), footer_rect.top()),
                    ],
                    Stroke::new(1.0_f32, footer_divider),
                );
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new("4/4  ·  Live pitch  ·  Note colors: flat / in tune / sharp")
                            .size(10.0)
                            .color(footer_text),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(12.0);
                        ui.label(RichText::new(COPYRIGHT).size(10.0).color(footer_text));
                    });
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(8.0);
            self.draw_header(ui);
            ui.add_space(22.0);
            self.draw_controls(ui);
            self.draw_practice_controls(ui);
            if let Some(error) = &self.error {
                ui.add_space(8.0);
                ui.colored_label(Color32::from_rgb(220, 75, 75), error);
            }
            ui.add_space(12.0);
            ui.columns(2, |columns| {
                self.draw_pitch_readout(&mut columns[0]);
                self.draw_track_controls(&mut columns[1]);
            });
            ui.add_space(15.0);
            self.draw_track_waveform(ui, self.applied_dark);
            self.draw_graph(ui, self.applied_dark);
        });

        self.draw_about(ctx);
        ctx.request_repaint_after(Duration::from_millis(33));
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

fn warmup_notes(pattern: WarmupPattern, tonic: i32) -> Vec<i32> {
    pattern
        .intervals()
        .iter()
        .map(|interval| tonic + interval)
        .collect()
}

fn root_midi(key_root: usize, octave: i32) -> i32 {
    (octave.clamp(1, 6) + 1) * 12 + key_root as i32
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

fn practice_phase(
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

fn pitch_color(cents: f32) -> Color32 {
    if cents.abs() <= CENTS_TOLERANCE {
        Color32::from_rgb(54, 190, 132)
    } else if cents < 0.0 {
        Color32::from_rgb(237, 86, 91)
    } else {
        Color32::from_rgb(77, 151, 255)
    }
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
        LoopHandle, WarmupPattern, advance_beat_position, advance_song_beat_position,
        centered_graph_range, clamp_graph_center, echo_countdown_value, follow_graph_center,
        format_track_time, graph_range_for_zoom, measure_phase, move_loop_handle,
        next_warmup_tonic, practice_phase, root_midi, should_show_scrub_preview, warmup_notes,
        zoom_label,
    };
    use std::time::Duration;

    #[test]
    fn default_zoom_is_one_and_half_zoom_shows_the_full_selected_range() {
        assert_eq!(graph_range_for_zoom(24.0, 96.0, 0.5), (24.0, 96.0));
        assert_eq!(graph_range_for_zoom(24.0, 96.0, 1.0), (42.0, 78.0));
        assert_eq!(graph_range_for_zoom(24.0, 96.0, 2.0), (51.0, 69.0));
        assert_eq!(zoom_label(1.0), "1x");
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
    fn practice_progress_maps_to_the_full_graph_width() {
        let started_at = std::time::Instant::now();
        let note_duration = Duration::from_millis(450);
        assert_eq!(
            practice_phase(started_at, started_at, note_duration, 3),
            0.0
        );
        assert!(
            (practice_phase(
                started_at + Duration::from_millis(675),
                started_at,
                note_duration,
                3
            ) - 0.5)
                .abs()
                < 0.001
        );
        assert_eq!(
            practice_phase(
                started_at + Duration::from_millis(2000),
                started_at,
                note_duration,
                3
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
    fn shared_root_octave_maps_key_to_expected_midi_note() {
        assert_eq!(root_midi(0, 3), 48);
        assert_eq!(root_midi(0, 4), 60);
        assert_eq!(root_midi(2, 4), 62);
        assert_eq!(root_midi(9, 3), 57);
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
