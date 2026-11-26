mod audio;
mod pitch;

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use audio::Microphone;
use eframe::egui::{self, Align, Color32, FontId, Layout, RichText, Stroke, Vec2};
use pitch::{PitchReading, midi_to_note};

const BAR_BEATS: f32 = 4.0;
const VISIBLE_BARS: f32 = 4.0;
const CENTS_TOLERANCE: f32 = 15.0;

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
    bpm: f32,
    last_bpm: f32,
    theme: ThemeChoice,
    applied_theme: ThemeChoice,
    applied_dark: bool,
    last_theme_check: Instant,
    reading: Option<PitchReading>,
    history: VecDeque<(f32, f32)>,
    elapsed: f32,
    last_cycle: u64,
    last_frame: Instant,
    last_analysis: Instant,
    error: Option<String>,
}

impl PitchHelperApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let theme = ThemeChoice::System;
        let applied_dark = theme_is_dark(theme);
        set_theme(&cc.egui_ctx, applied_dark);
        Self {
            microphone: Microphone::new(),
            bpm: 100.0,
            last_bpm: 100.0,
            theme,
            applied_theme: theme,
            applied_dark,
            last_theme_check: Instant::now(),
            reading: None,
            history: VecDeque::new(),
            elapsed: 0.0,
            last_cycle: 0,
            last_frame: Instant::now(),
            last_analysis: Instant::now(),
            error: None,
        }
    }

    fn update_audio(&mut self) {
        if !self.microphone.is_running() || self.last_analysis.elapsed() < Duration::from_millis(35)
        {
            return;
        }
        self.last_analysis = Instant::now();

        let Some(samples) = self.microphone.samples() else {
            return;
        };
        let Some(reading) = pitch::detect_pitch(&samples, self.microphone.sample_rate()) else {
            self.reading = None;
            return;
        };

        self.reading = Some(reading);
        let bar_duration = BAR_BEATS * 60.0 / self.bpm;
        let cycle_duration = bar_duration * VISIBLE_BARS;
        if self.bpm != self.last_bpm {
            self.history.clear();
            self.last_bpm = self.bpm;
        }
        let cycle = (self.elapsed / cycle_duration) as u64;
        if cycle != self.last_cycle {
            self.history.clear();
            self.last_cycle = cycle;
        }
        let phase = (self.elapsed % cycle_duration) / cycle_duration;
        self.history.push_back((phase, reading.midi));
        while self.history.len() > 900 {
            self.history.pop_front();
        }
    }

    fn draw_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("PITCH")
                    .size(12.0)
                    .strong()
                    .color(Color32::from_rgb(125, 145, 161)),
            );
            ui.add_space(4.0);
            ui.label(RichText::new("helper").size(17.0).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                egui::ComboBox::from_id_salt("theme_choice")
                    .selected_text(match self.theme {
                        ThemeChoice::System => "System",
                        ThemeChoice::Light => "Light",
                        ThemeChoice::Dark => "Dark",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.theme, ThemeChoice::System, "System");
                        ui.selectable_value(&mut self.theme, ThemeChoice::Light, "Light");
                        ui.selectable_value(&mut self.theme, ThemeChoice::Dark, "Dark");
                    });
                ui.label(
                    RichText::new("APPEARANCE")
                        .size(10.0)
                        .strong()
                        .color(Color32::from_rgb(125, 145, 161)),
                );
            });
        });
    }

    fn draw_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("TEMPO")
                    .size(10.0)
                    .strong()
                    .color(Color32::from_rgb(125, 145, 161)),
            );
            ui.add_space(6.0);
            ui.add(
                egui::Slider::new(&mut self.bpm, 40.0..=240.0)
                    .suffix(" BPM")
                    .show_value(false),
            );
            ui.label(
                RichText::new(format!("{:.0}", self.bpm))
                    .monospace()
                    .strong(),
            );
            ui.add_space(18.0);
            let button_text = if self.microphone.is_running() {
                "Stop listening"
            } else {
                "Start listening"
            };
            if ui.button(button_text).clicked() {
                self.error = None;
                if self.microphone.is_running() {
                    self.microphone.stop();
                    self.reading = None;
                    self.history.clear();
                } else if let Err(message) = self.microphone.start() {
                    self.error = Some(message);
                }
            }
            if self.microphone.is_running() {
                ui.label(
                    RichText::new("● LIVE")
                        .size(11.0)
                        .strong()
                        .color(Color32::from_rgb(55, 180, 125)),
                );
            } else {
                ui.label(
                    RichText::new("MICROPHONE OFF")
                        .size(10.0)
                        .color(Color32::from_rgb(125, 145, 161)),
                );
            }
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
        ui.horizontal(|ui| {
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
        });
    }

    fn draw_graph(&self, ui: &mut egui::Ui, dark: bool) {
        let available_width = ui.available_width();
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(available_width, 340.0), egui::Sense::hover());
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
        let low_midi = 36.0;
        let high_midi = 84.0;
        for midi in (36..=84).step_by(3) {
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

        let bar_duration = BAR_BEATS * 60.0 / self.bpm;
        let cycle_duration = bar_duration * VISIBLE_BARS;
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

        let phase = (self.elapsed % cycle_duration) / cycle_duration;
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
        if self.history.is_empty() {
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
        self.elapsed += now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.update_audio();

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

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(8.0);
            self.draw_header(ui);
            ui.add_space(22.0);
            self.draw_controls(ui);
            if let Some(error) = &self.error {
                ui.add_space(8.0);
                ui.colored_label(Color32::from_rgb(220, 75, 75), error);
            }
            ui.add_space(20.0);
            self.draw_pitch_readout(ui);
            ui.add_space(15.0);
            self.draw_graph(ui, self.applied_dark);
            ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
                ui.add_space(8.0);
                ui.label(
                    RichText::new("4/4  ·  Live pitch  ·  Note colors: flat / in tune / sharp")
                        .size(10.0)
                        .color(Color32::from_rgb(125, 145, 161)),
                );
            });
        });
        ctx.request_repaint_after(Duration::from_millis(33));
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
