use super::{
    AudioLoadStatus, BAR_BEATS, CENTS_TOLERANCE, COPYRIGHT, EchoPhase, LOOP_HANDLE_HIT_RADIUS,
    LoopHandle, PitchHelperApp, ReferenceKind, Scale, ThemeChoice, VISIBLE_BARS, WarmupPattern,
    advance_beat_position, advance_song_beat_position, beats_per_cycle, close_menu_if_pointer_left,
    countdown_value, draw_practice_target_notes, draw_target_notes, format_track_time,
    graph_high_note_choices, graph_low_note_choices, measure_phase, midi_to_note, move_loop_handle,
    pitch, pitch_color, pitch_metrics, practice_phase, root_note_choices, set_theme,
    should_show_scrub_preview, theme_is_dark, zoom_label,
};
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Color32, FontId, Layout, RichText, Stroke, Vec2};

const APP_TITLE_SIZE: f32 = 21.0;
const DIALOG_BODY_SIZE: f32 = 14.0;
const SECTION_LABEL_SIZE: f32 = 10.0;

impl PitchHelperApp {
    fn draw_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("PitchHelper").size(APP_TITLE_SIZE).strong());
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
        let previous_graph_min = self.graph_min_limit;
        let previous_graph_max = self.graph_max_limit;
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
                "Zoom is relative to the selected LOW/HIGH range. 0.5x shows all notes; 1x shows half the range; 2x shows a quarter.",
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
                    for midi in graph_low_note_choices(self.graph_max_limit) {
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
                    for midi in graph_high_note_choices(self.graph_min_limit) {
                        ui.selectable_value(
                            &mut highest,
                            midi as f32,
                            midi_to_note(midi as f32).1,
                        );
                    }
                });
            self.graph_max_limit = highest;
        });
        if self.graph_min_limit != previous_graph_min || self.graph_max_limit != previous_graph_max
        {
            self.center_on_root = true;
        }
        let clamped_root = self.root_note.clamp(
            self.graph_min_limit.ceil() as i32,
            self.graph_max_limit.floor() as i32,
        );
        if clamped_root != self.root_note {
            self.root_note = clamped_root;
            self.center_on_root = true;
            if self.warmup.is_some() {
                self.stop_warmup();
            }
            if self.echo.is_some() {
                self.stop_echo();
            }
            if self.reference_enabled {
                self.start_reference_tone();
            }
        }
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
                    let mut monitor_enabled = self.microphone.is_monitoring();
                    let monitor_response = ui.add_enabled(
                        self.microphone.is_running(),
                        egui::Checkbox::new(&mut monitor_enabled, "Monitor"),
                    );
                    if monitor_response.changed()
                        && let Err(message) = self.microphone.set_monitoring(monitor_enabled)
                    {
                        self.error = Some(message);
                    }
                    monitor_response.on_hover_text(
                        "Play microphone audio through the default output. Use headphones to prevent feedback.",
                    );
                });
            });
        });
        if refresh_inputs {
            self.refresh_input_devices();
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
                                    RichText::new("Choose an audio file...")
                                        .size(12.0)
                                        .color(Color32::from_rgb(125, 145, 161)),
                                );
                            }
                            AudioLoadStatus::Analyzing(name) => {
                                ui.spinner();
                                ui.label(
                                    RichText::new(format!("Analyzing {name}..."))
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
        let mut root_changed = false;
        let mut recenter_root = false;
        let mut exercise_pattern_changed = false;
        let mut reference_changed = false;
        let mut reference_toggled = false;
        let mut metronome_changed = false;
        let mut stop_practice = false;
        let mut start_warmup = false;
        let mut start_echo = false;
        let mut next_echo = false;
        ui.add_space(8.0);
        let panel_width = ui.available_width();
        ui.group(|ui| {
            ui.set_width((panel_width - 16.0).max(0.0));
            ui.columns(3, |columns| {
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
                        ui.add_enabled_ui(self.metronome_enabled, |ui| {
                            ui.add_sized(
                                [52.0, 20.0],
                                egui::Slider::new(&mut self.metronome_volume, 0.0..=1.0)
                                    .show_value(false),
                            )
                            .on_hover_text("Metronome volume");
                        });
                        ui.label(format!("{:.0}%", self.metronome_volume * 100.0));
                    });
                });
                let column_width = columns[1].available_width();
                columns[1].group(|ui| {
                    ui.set_width((column_width - 16.0).max(0.0));
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("KEY & SCALE").small().strong());
                        let previous_root = self.root_note;
                        egui::ComboBox::from_id_salt("song_key")
                            .selected_text(midi_to_note(self.root_note as f32).1)
                            .width(56.0)
                            .show_ui(ui, |ui| {
                                for midi in
                                    root_note_choices(self.graph_min_limit, self.graph_max_limit)
                                {
                                    let response = ui.selectable_value(
                                        &mut self.root_note,
                                        midi,
                                        midi_to_note(midi as f32).1,
                                    );
                                    root_changed |= response.changed();
                                    recenter_root |= response.clicked();
                                }
                            });
                        root_changed |= previous_root != self.root_note;
                        egui::ComboBox::from_id_salt("song_scale")
                            .selected_text(self.scale.label())
                            .width(112.0)
                            .show_ui(ui, |ui| {
                                for scale in [
                                    Scale::Major,
                                    Scale::Minor,
                                    Scale::MajorPentatonic,
                                    Scale::MinorPentatonic,
                                ] {
                                    ui.selectable_value(&mut self.scale, scale, scale.label());
                                }
                            });
                    });
                });
                let column_width = columns[2].available_width();
                columns[2].group(|ui| {
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
                            .width(84.0)
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
            });

            ui.add_space(6.0);
            ui.columns(3, |columns| {
                let column_width = columns[0].available_width();
                columns[0].group(|ui| {
                    ui.set_width((column_width - 16.0).max(0.0));
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("EXERCISE PATTERN").small().strong())
                            .on_hover_text("Shared by Warm-Up and Pitch Echo.");
                        egui::ComboBox::from_id_salt("exercise_pattern")
                            .selected_text(self.exercise_pattern.label())
                            .width(108.0)
                            .show_ui(ui, |ui| {
                                for pattern in [
                                    WarmupPattern::FiveTone,
                                    WarmupPattern::Octave,
                                    WarmupPattern::Arpeggio,
                                    WarmupPattern::Siren,
                                ] {
                                    exercise_pattern_changed |= ui
                                        .selectable_value(
                                            &mut self.exercise_pattern,
                                            pattern,
                                            pattern.label(),
                                        )
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
                            .on_hover_text("Warm-ups start from the shared Key & Scale note.");
                        ui.label(
                            RichText::new(midi_to_note(self.root_note as f32).1)
                                .size(11.0)
                                .color(Color32::from_rgb(125, 145, 161)),
                        );
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
                        if let Some(session) = &self.echo {
                            if session.phase == EchoPhase::Score {
                                if ui.button("Next Sequence").clicked() {
                                    next_echo = true;
                                }
                                if ui.button("Stop Echo").clicked() {
                                    stop_practice = true;
                                }
                                if let Some(score) = self.echo_score {
                                    ui.label(format!("Match: {score:.0}%"));
                                }
                            } else {
                                if ui.button("Stop Echo").clicked() {
                                    stop_practice = true;
                                }
                                let status = match session.phase {
                                    EchoPhase::Countdown => "Get ready to listen",
                                    EchoPhase::Prompting => "Listen to the phrase",
                                    EchoPhase::Pause => "Get ready to sing",
                                    EchoPhase::Responding => "Sing the phrase back",
                                    EchoPhase::Score => "Sequence scored",
                                };
                                ui.label(status);
                            }
                        } else if ui.button("Start Echo").clicked() {
                            start_echo = true;
                        }
                    });
                });
            });
        });

        if root_changed || recenter_root {
            self.center_on_root = true;
        }
        let practice_config_changed = root_changed || exercise_pattern_changed;
        if practice_config_changed && self.warmup.is_some() {
            self.stop_warmup();
        }
        if practice_config_changed && self.echo.is_some() {
            self.stop_echo();
        }
        if root_changed && self.reference_enabled {
            self.start_reference_tone();
        }
        if metronome_changed {
            if let Err(message) = self.metronome.set_enabled(self.metronome_enabled) {
                self.metronome_enabled = false;
                self.error = Some(message);
            } else if self.metronome_enabled {
                self.metronome.set_position(self.beat_position);
            }
        }
        if reference_toggled {
            if self.reference_enabled {
                self.stop_warmup();
                self.stop_echo();
                self.start_reference_tone();
            } else {
                self.tone_player.stop();
            }
        } else if reference_changed && self.reference_enabled {
            self.stop_warmup();
            self.stop_echo();
            self.start_reference_tone();
        }
        if stop_practice {
            self.stop_warmup();
            self.stop_echo();
        }
        if start_warmup {
            self.start_warmup();
        } else if start_echo {
            self.start_echo();
        } else if next_echo {
            self.start_next_echo_sequence();
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

    fn draw_menu_bar(&mut self, ctx: &egui::Context) -> bool {
        let mut open_audio = false;
        egui::TopBottomPanel::top("application_menu").show(ctx, |ui| {
            let menu_bar_rect = ui.max_rect();
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui
                        .add_enabled(self.audio_load.is_none(), egui::Button::new("Open Audio"))
                        .clicked()
                    {
                        open_audio = true;
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        ui.close_menu();
                    }
                    close_menu_if_pointer_left(ui, menu_bar_rect);
                });
                ui.menu_button("Settings", |ui| {
                    ui.menu_button("Appearance", |ui| {
                        ui.selectable_value(&mut self.theme, ThemeChoice::System, "System");
                        ui.selectable_value(&mut self.theme, ThemeChoice::Light, "Light");
                        ui.selectable_value(&mut self.theme, ThemeChoice::Dark, "Dark");
                    });
                    ui.menu_button("Range Sensitivity", |ui| {
                        ui.label("Higher values accept quieter notes.");
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                [140.0, 20.0],
                                egui::Slider::new(&mut self.range_sensitivity, 0.0..=100.0)
                                    .step_by(1.0)
                                    .show_value(false),
                            )
                            .on_hover_text(
                                "Lower values require a louder signal. Pitches must also be sustained for at least 400 ms.",
                            );
                            ui.label(format!("{:.0}%", self.range_sensitivity));
                        });
                    });
                    ui.menu_button("Highlight Opacity", |ui| {
                        ui.label("Key / scale highlight strength.");
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                [140.0, 20.0],
                                egui::Slider::new(&mut self.highlight_opacity, 0.0..=0.5)
                                    .show_value(false),
                            )
                            .on_hover_text(
                                "Adjusts the opacity of key and scale highlights on the graph.",
                            );
                            ui.label(format!("{:.0}%", self.highlight_opacity * 100.0));
                        });
                    });
                    ui.separator();
                    if ui.button("Clear Range & Tessitura History").clicked() {
                        self.clear_range_confirmation = true;
                        ui.close_menu();
                    }
                    close_menu_if_pointer_left(ui, menu_bar_rect);
                });
                ui.menu_button("Help", |ui| {
                    if ui.button("About PitchHelper").clicked() {
                        self.about_open = true;
                        ui.close_menu();
                    }
                    close_menu_if_pointer_left(ui, menu_bar_rect);
                });
            });
        });
        open_audio
    }

    fn draw_about(&mut self, ctx: &egui::Context) {
        let screen_rect = ctx.screen_rect();
        let max_content_height = (screen_rect.height() - 200.0).clamp(100.0, 560.0);
        let about_size = Vec2::new(
            screen_rect.width().min(430.0),
            self.about_content_height.min(max_content_height),
        );
        let content_height = egui::Window::new("About PitchHelper")
            .collapsible(false)
            .movable(false)
            .resizable(false)
            .frame(egui::Frame::window(&ctx.style()))
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .constrain_to(screen_rect.shrink(48.0))
            .open(&mut self.about_open)
            .fixed_size(about_size)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("about_content")
                    .auto_shrink([false, false])
                    .max_height(max_content_height)
                    .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                let heading_color = Color32::from_rgb(125, 145, 161);
                let title_color = ui.visuals().text_color();
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new("PitchHelper")
                            .size(APP_TITLE_SIZE)
                            .strong()
                            .color(title_color),
                    );
                    ui.label(
                        RichText::new(format!("version {}", env!("CARGO_PKG_VERSION")))
                            .size(12.0)
                            .weak(),
                    );
                    ui.label(RichText::new("Live pitch coach for singers").size(DIALOG_BODY_SIZE));
                });
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);

                ui.label(
                    RichText::new("GETTING STARTED")
                        .size(SECTION_LABEL_SIZE)
                        .strong()
                        .color(heading_color),
                );
                ui.label(
                    RichText::new("Choose a microphone, then select Start Listening. The device list refreshes every two seconds.")
                        .size(DIALOG_BODY_SIZE),
                );
                ui.label(
                    RichText::new("Raise Pitch Sensitivity if quieter singing is not detected.")
                        .size(DIALOG_BODY_SIZE),
                );
                ui.label(
                    RichText::new(
                        "When listening, enable Monitor to hear the microphone through your output; use headphones to prevent feedback.",
                    )
                    .size(DIALOG_BODY_SIZE),
                );
                ui.add_space(6.0);

                ui.label(
                    RichText::new("READING THE DISPLAY")
                        .size(SECTION_LABEL_SIZE)
                        .strong()
                        .color(heading_color),
                );
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("IN TUNE")
                            .size(12.0)
                            .strong()
                            .color(Color32::from_rgb(54, 190, 132)),
                    );
                    ui.label(RichText::new("within 15 cents").size(12.0));
                    ui.label(
                        RichText::new("FLAT")
                            .size(12.0)
                            .strong()
                            .color(Color32::from_rgb(220, 75, 75)),
                    );
                    ui.label(
                        RichText::new("SHARP")
                            .size(12.0)
                            .strong()
                            .color(Color32::from_rgb(80, 156, 255)),
                    );
                });
                ui.label(
                    RichText::new("Outside exercises, the playhead cycles through four 4/4 bars. Tempo controls its pace; LOW, HIGH, and Zoom adjust the visible pitch range.")
                        .size(DIALOG_BODY_SIZE),
                );
                ui.add_space(6.0);

                ui.label(
                    RichText::new("PLAYBACK AND PRACTICE")
                        .size(SECTION_LABEL_SIZE)
                        .strong()
                        .color(heading_color),
                );
                ui.label(
                    RichText::new("File > Open Audio loads WAV, MP3, FLAC, or OGG. Tempo detection is a best effort; adjust it manually if needed.")
                        .size(DIALOG_BODY_SIZE),
                );
                ui.label(
                    RichText::new("The metronome follows the graph or playing track; its higher-pitched click marks each measure's first beat.")
                        .size(DIALOG_BODY_SIZE),
                );
                ui.label(
                    RichText::new("Key / Scale highlights notes. The shared root sets the reference tone and exercises; Warm-Up counts down once, and Warm-Up and Pitch Echo share a pattern.")
                        .size(DIALOG_BODY_SIZE),
                );
                ui.label(
                    RichText::new("Echo plays a phrase, waits three seconds, then lets you sing it back and shows your match score.")
                        .size(DIALOG_BODY_SIZE),
                );
                ui.label(
                    RichText::new("Drag the A/B waveform handles to loop a section; right-click for reset options.")
                        .size(DIALOG_BODY_SIZE),
                );
                ui.add_space(6.0);

                ui.label(
                    RichText::new("RANGE AND SETTINGS")
                        .size(SECTION_LABEL_SIZE)
                        .strong()
                        .color(heading_color),
                );
                ui.label(
                    RichText::new("Session and saved global range appear beside the pitch readout. The blue band estimates the central 80% of stable pitches.")
                        .size(DIALOG_BODY_SIZE),
                );
                ui.label(
                    RichText::new("Settings > Range Sensitivity filters quiet or brief pitch detections. Lower values require a louder signal; a pitch must last at least 400 ms.")
                        .size(DIALOG_BODY_SIZE),
                );
                ui.label(
                    RichText::new("Appearance follows your system by default. Settings can override it, adjust scale-highlight opacity, or clear saved history.")
                        .size(DIALOG_BODY_SIZE),
                );
                ui.add_space(6.0);
                ui.label(RichText::new(COPYRIGHT).size(11.0).weak());
                    })
                    .content_size
                    .y
            });
        if let Some(content_height) = content_height.and_then(|response| response.inner) {
            self.about_content_height = content_height.clamp(100.0, 560.0);
        }
    }

    fn draw_range_clear_confirmation(&mut self, ctx: &egui::Context) {
        let mut confirmed = false;
        let mut cancelled = false;
        egui::Window::new("Clear Range & Tessitura History?")
            .collapsible(false)
            .movable(false)
            .resizable(false)
            .frame(egui::Frame::window(&ctx.style()))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .open(&mut self.clear_range_confirmation)
            .default_width(360.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(
                        "This permanently removes the saved global range and tessitura history, and clears the current session values.",
                    )
                    .size(DIALOG_BODY_SIZE),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    confirmed = ui.button("Clear History").clicked();
                    cancelled = ui.button("Cancel").clicked();
                });
            });
        if cancelled {
            self.clear_range_confirmation = false;
        }
        if confirmed {
            self.clear_range_history();
        }
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
                let metrics = pitch_metrics(&self.recent_pitch, Instant::now());
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
                ui.label(
                    RichText::new("RANGE & TESSITURA")
                        .size(10.0)
                        .strong()
                        .color(Color32::from_rgb(125, 145, 161)),
                );
                let session_text = match (
                    self.session_range_tracker.low,
                    self.session_range_tracker.high,
                ) {
                    (Some(low), Some(high)) => format!(
                        "{}–{}",
                        midi_to_note(low as f32).1,
                        midi_to_note(high as f32).1
                    ),
                    _ => "Sing a note to start tracking".to_owned(),
                };
                ui.label(
                    RichText::new(format!("Session: {session_text}"))
                        .size(10.0)
                        .color(Color32::from_rgb(125, 145, 161)),
                );
                let global_values = match (
                    self.range_tracker.low,
                    self.range_tracker.high,
                    self.range_tracker.tessitura(),
                ) {
                    (Some(low), Some(high), Some((tessitura_low, tessitura_high))) => format!(
                        "Range {}–{} · Tessitura {}–{} (blue band)",
                        midi_to_note(low as f32).1,
                        midi_to_note(high as f32).1,
                        midi_to_note(tessitura_low as f32).1,
                        midi_to_note(tessitura_high as f32).1
                    ),
                    (Some(low), Some(high), None) => format!(
                        "Range {}–{} · Tessitura not available yet",
                        midi_to_note(low as f32).1,
                        midi_to_note(high as f32).1
                    ),
                    (None, None, Some((tessitura_low, tessitura_high))) => format!(
                        "Range not available · Tessitura {}–{} (blue band)",
                        midi_to_note(tessitura_low as f32).1,
                        midi_to_note(tessitura_high as f32).1
                    ),
                    _ => "No saved range or tessitura".to_owned(),
                };
                ui.label(
                    RichText::new(format!("Global: {global_values}"))
                        .size(10.0)
                        .color(Color32::from_rgb(125, 145, 161)),
                )
                .on_hover_text(
                    "The blue band shows an across-session tessitura estimate: the central 80% of steady pitch samples. It is recalculated from all saved samples, so it can move lower or higher; sustained steady notes in a new range may be needed to shift it. It indicates stability, not subjective comfort. Global history is stored in your user data folder.",
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
            Color32::from_rgb(62, 75, 90)
        } else {
            Color32::from_rgb(205, 216, 226)
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
            let tessitura_rect = egui::Rect::from_min_max(
                egui::pos2(plot.left(), top),
                egui::pos2(plot.right(), bottom),
            );
            painter.rect_filled(
                tessitura_rect,
                0.0,
                Color32::from_rgba_unmultiplied(80, 156, 255, if dark { 28 } else { 38 }),
            );
            painter.line_segment(
                [
                    egui::pos2(tessitura_rect.left(), tessitura_rect.top()),
                    egui::pos2(tessitura_rect.right(), tessitura_rect.top()),
                ],
                Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(80, 156, 255, 150)),
            );
            painter.line_segment(
                [
                    egui::pos2(tessitura_rect.left(), tessitura_rect.bottom()),
                    egui::pos2(tessitura_rect.right(), tessitura_rect.bottom()),
                ],
                Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(80, 156, 255, 150)),
            );
        }
        let row_height = plot.height() / (high_midi - low_midi);
        for midi in low_midi.floor() as i32..=high_midi.ceil() as i32 {
            if !(low_midi..=high_midi).contains(&(midi as f32)) {
                continue;
            }
            let interval = (midi - self.root_note.rem_euclid(12)).rem_euclid(12);
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

        let label_start = low_midi.ceil() as i32;
        let label_end = high_midi.floor() as i32;
        let label_stride = (13.0 / row_height).ceil().max(1.0) as i32;
        for midi in label_start..=label_end {
            let y =
                plot.bottom() - (midi as f32 - low_midi) / (high_midi - low_midi) * plot.height();
            painter.line_segment(
                [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
                Stroke::new(
                    if midi.rem_euclid(12) == 0 {
                        1.6_f32
                    } else {
                        1.0_f32
                    },
                    grid,
                ),
            );
            if (midi - label_start) % label_stride == 0 {
                let (_, name, _) = midi_to_note(midi as f32);
                painter.text(
                    egui::pos2(rect.left() + 10.0, y),
                    egui::Align2::LEFT_CENTER,
                    name,
                    FontId::monospace(10.0),
                    muted,
                );
            }
        }
        painter.text(
            egui::pos2(plot.right(), rect.top() + 7.0),
            egui::Align2::RIGHT_CENTER,
            format!(
                "{} {} · tinted rows: scale · brighter: chord tones",
                midi_to_note(self.root_note as f32).1,
                self.scale.label()
            ),
            FontId::proportional(9.0),
            muted,
        );

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

        let mut previous_point: Option<egui::Pos2> = None;
        for (phase, midi) in &self.history {
            if !(low_midi..=high_midi).contains(midi) {
                previous_point = None;
                continue;
            }
            let point = egui::pos2(
                plot.left() + phase * plot.width(),
                plot.bottom() - (midi - low_midi) / (high_midi - low_midi) * plot.height(),
            );
            if let Some(previous_point) = previous_point {
                let average_y = (previous_point.y + point.y) * 0.5;
                let midi_at_y =
                    high_midi - (average_y - plot.top()) / plot.height() * (high_midi - low_midi);
                let cents = (midi_at_y - midi_at_y.round()) * 100.0;
                painter.line_segment(
                    [previous_point, point],
                    Stroke::new(2.4_f32, pitch_color(cents)),
                );
            }
            previous_point = Some(point);
        }

        let now = Instant::now();
        if let Some(session) = &self.warmup {
            draw_practice_target_notes(
                &painter,
                plot,
                low_midi,
                high_midi,
                &session.notes,
                session.note_duration,
                Color32::from_rgb(248, 191, 76),
            );
        } else if let Some(session) = &self.echo {
            draw_practice_target_notes(
                &painter,
                plot,
                low_midi,
                high_midi,
                &session.notes,
                session.note_duration,
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

        let countdown = self
            .echo
            .as_ref()
            .filter(|session| matches!(session.phase, EchoPhase::Countdown | EchoPhase::Pause))
            .map(|session| now.duration_since(session.started_at))
            .or_else(|| {
                self.warmup
                    .as_ref()
                    .and_then(|session| session.countdown_started_at)
                    .map(|started_at| now.duration_since(started_at))
            });
        if let Some(number) = countdown.and_then(countdown_value) {
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

        let phase = if let Some(session) = &self.echo {
            match session.phase {
                EchoPhase::Countdown | EchoPhase::Pause => 0.0,
                EchoPhase::Prompting | EchoPhase::Responding | EchoPhase::Score => {
                    practice_phase(now, session.started_at, session.note_duration)
                }
            }
        } else if let Some(session) = &self.warmup {
            if session.countdown_started_at.is_some() {
                0.0
            } else {
                practice_phase(now, session.started_at, session.note_duration)
            }
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
        self.update_input_devices(now);
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
        self.draw_range_clear_confirmation(ctx);
        ctx.request_repaint_after(Duration::from_millis(33));
    }
}
