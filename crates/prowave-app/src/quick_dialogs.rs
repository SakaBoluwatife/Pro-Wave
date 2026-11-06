//! Small windows the menus open: Go to Position, Rename Track, Assign Track Color, Undo History.
use crate::model::Project;
use crate::UiState;

#[derive(Default)]
pub struct QuickDialogs {
    /// Navigate > Go to Position: what has been typed.
    pub go_to: Option<String>,
    /// Track > Rename Track: the track and what has been typed.
    pub rename: Option<(usize, String)>,
    /// Track > Assign Track Color: the track.
    pub color: Option<usize>,
    /// The regions whose colour window is open.
    pub region_color: Vec<(usize, usize)>,
    /// The big input level window.
    pub rec_meter: bool,
    pub undo_history: bool,
    pub routing: bool,
    pub project_settings: bool,
    /// The Text tool: the region and what has been typed.
    pub rename_region: Option<(usize, usize, String)>,
    /// Track > Configure Track Header.
    pub track_header: bool,
    /// Track > Assign Track Icon: the track.
    pub icon: Option<usize>,
    /// File > Import > Other Projects: the project opened, and which of its tracks to bring.
    pub import_project: Option<(Box<Project>, Vec<bool>)>,
    /// File > Clean Up: the files it found to delete.
    pub clean_up: Option<Vec<std::path::PathBuf>>,
    /// Help > About Pro Wave.
    pub about: bool,
    /// View > Customize Toolbar.
    pub customize_toolbar: bool,
    /// Window > Plugin Manager.
    pub plugin_manager: bool,
    /// A track's right-click menu > Track Alternatives > Rename: the track and what has been typed.
    pub rename_alternative: Option<(usize, String)>,
    /// Mix > Edit Groups.
    pub groups: bool,
    /// Window > Screensets.
    pub screensets: bool,
    /// File > Project Management > Rename Project: what has been typed.
    pub rename_project: Option<String>,
    /// Asked for from the Undo History: this many steps back (`App` carries it out).
    pub undo_steps: usize,
    /// Asked for from the Undo History: this many undone steps forward.
    pub redo_steps: usize,
    focus: bool,
}

/// A position typed as bar, beat, division and tick, of which only the bar is needed: `9`, `9 3`,
/// `9.3.2.120`. Counted from 1, as the display does. In beats from the start of the song.
pub fn parse_position(p: &Project, text: &str) -> Option<f64> {
    let parts: Vec<f64> = text
        .split(|c: char| c == '.' || c == ':' || c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<f64>().ok().filter(|v| v.is_finite()))
        .collect::<Option<_>>()?;
    let bar = *parts.first()?;
    if bar < 1.0 {
        return None;
    }
    let start = p.bar_to_beats(bar.floor() - 1.0);
    let (_, denominator) = p.signature_of_bar((bar.floor() - 1.0) as u32);
    let unit = 4.0 / f64::from(denominator.max(1));
    let part = |i: usize| parts.get(i).copied().unwrap_or(1.0).max(1.0) - 1.0;
    Some(start + part(1) * unit + part(2) * unit / 4.0 + part(3) * unit / 4.0 / 240.0)
}

fn answer(ui: &mut egui::Ui, focus: &mut bool, edit: &egui::Response) -> (bool, bool) {
    if std::mem::take(focus) {
        edit.request_focus();
    }
    let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let mut ok = enter;
    let mut close = ui.input(|i| i.key_pressed(egui::Key::Escape));
    ui.horizontal(|ui| {
        ok |= ui.button("OK").clicked();
        close |= ui.button("Cancel").clicked();
    });
    (ok, close)
}

impl QuickDialogs {
    pub fn open_rename_region(&mut self, t: usize, r: usize, name: &str) {
        self.rename_region = Some((t, r, name.to_string()));
        self.focus = true;
    }

    pub fn open_go_to(&mut self) {
        self.go_to = Some(String::new());
        self.focus = true;
    }

    pub fn open_rename(&mut self, track: usize, name: &str) {
        self.rename = Some((track, name.to_string()));
        self.focus = true;
    }
}

fn go_to_window(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    let d = &mut s.dialogs;
    let Some(text) = d.go_to.as_mut() else {
        return;
    };
    let mut result = (false, false);
    let mut focus = std::mem::take(&mut d.focus);
    egui::Modal::new(egui::Id::unique("go_to_position")).show(ctx, |ui| {
        ui.set_width(260.0);
        ui.label("Go to bar, beat, division, tick:");
        let edit = ui.add(egui::TextEdit::singleline(text).hint_text("9 3"));
        result = answer(ui, &mut focus, &edit);
    });
    let (ok, close) = result;
    if ok {
        match parse_position(p, text) {
            Some(beat) => {
                p.position_beats = beat;
                d.go_to = None;
            }
            None => s.status = Some("Type a bar number, then beat, division and tick".to_string()),
        }
    } else if close {
        d.go_to = None;
    }
}

fn rename_window(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    let d = &mut s.dialogs;
    let Some((track, text)) = d.rename.as_mut() else {
        return;
    };
    let mut result = (false, false);
    let mut focus = std::mem::take(&mut d.focus);
    egui::Modal::new(egui::Id::unique("rename_track")).show(ctx, |ui| {
        ui.set_width(280.0);
        ui.label("Track name:");
        let edit = ui.add(egui::TextEdit::singleline(text));
        result = answer(ui, &mut focus, &edit);
    });
    let (ok, close) = result;
    if ok {
        let name = text.trim();
        if let (false, Some(t)) = (name.is_empty(), p.tracks.get_mut(*track)) {
            t.name = name.to_string();
        }
        d.rename = None;
    } else if close {
        d.rename = None;
    }
}

/// The colours to choose from, and the one clicked.
fn palette(ui: &mut egui::Ui, current: Option<egui::Color32>) -> Option<egui::Color32> {
    let mut chosen = None;
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(12.0 * 30.0);
        for color in crate::model::TRACK_COLORS {
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());
            ui.painter().rect_filled(rect, 3.0, color);
            if current == Some(color) {
                ui.painter().rect_stroke(
                    rect,
                    3.0,
                    egui::Stroke::new(2.0, egui::Color32::WHITE),
                    egui::StrokeKind::Inside,
                );
            }
            if resp.clicked() {
                chosen = Some(color);
            }
        }
    });
    chosen
}

fn color_window(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    let Some(track) = s.dialogs.color else {
        return;
    };
    let Some(t) = p.tracks.get(track) else {
        s.dialogs.color = None;
        return;
    };
    // With several tracks picked, they change together if this one is among them.
    let tracks = if s.tracks.picked_tracks.contains(&track) {
        s.tracks.picked_tracks.clone()
    } else {
        vec![track]
    };
    let title = if tracks.len() > 1 {
        format!("Track Color: {} tracks", tracks.len())
    } else {
        format!("Track Color: {}", t.name)
    };
    let before = t.color;
    let mut color = before;
    let mut open = true;
    egui::Window::new(title)
        .id(egui::Id::unique("track_color"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            if let Some(chosen) = palette(ui, Some(color)) {
                color = chosen;
            }
            ui.color_edit_button_srgba(&mut color);
        });
    if color != before {
        for t in tracks {
            if let Some(t) = p.tracks.get_mut(t) {
                t.color = color;
            }
        }
    }
    if !open {
        s.dialogs.color = None;
    }
}

fn region_color_window(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    if s.dialogs.region_color.is_empty() {
        return;
    }
    let first = s.dialogs.region_color.iter().find_map(|&(t, r)| {
        let track = p.tracks.get(t)?;
        let region = track.regions.get(r)?;
        Some(region.params.color.unwrap_or(track.color))
    });
    let Some(mut color) = first else {
        s.dialogs.region_color.clear();
        return;
    };
    let (mut open, mut own, mut changed) = (true, true, false);
    egui::Window::new("Region Color")
        .id(egui::Id::unique("region_color"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            if let Some(chosen) = palette(ui, Some(color)) {
                color = chosen;
                changed = true;
            }
            ui.horizontal(|ui| {
                changed |= ui.color_edit_button_srgba(&mut color).changed();
                if ui.button("Use Track Color").clicked() {
                    own = false;
                    changed = true;
                }
            });
        });
    if changed {
        for &(t, r) in &s.dialogs.region_color {
            if let Some(region) = p.tracks.get_mut(t).and_then(|t| t.regions.get_mut(r)) {
                region.params.color = own.then_some(color);
            }
        }
    }
    if !open {
        s.dialogs.region_color.clear();
    }
}

/// One level as a wide bar, from -60 to 0 dB, with marks each 12 dB.
fn big_meter(ui: &mut egui::Ui, level: f32) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().max(240.0), 22.0),
        egui::Sense::hover(),
    );
    let g = ui.painter();
    g.rect_filled(rect, 2.0, egui::Color32::from_gray(0x18));
    let db = 20.0 * level.max(1e-6).log10();
    let x_of = |db: f32| rect.left() + rect.width() * ((db + 60.0) / 60.0).clamp(0.0, 1.0);
    let fill = if db >= -0.1 {
        egui::Color32::from_rgb(0xe0, 0x40, 0x40)
    } else if db >= -6.0 {
        egui::Color32::from_rgb(0xe0, 0xb0, 0x40)
    } else {
        egui::Color32::from_rgb(0x50, 0xc0, 0x60)
    };
    let bar = egui::Rect::from_min_max(rect.min, egui::pos2(x_of(db), rect.bottom()));
    g.rect_filled(bar, 2.0, fill);
    for mark in [-48.0, -36.0, -24.0, -12.0] {
        let x = x_of(mark);
        g.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(1.0, egui::Color32::from_gray(0x60)),
        );
    }
    g.text(
        rect.right_center() - egui::vec2(6.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        if db <= -60.0 {
            "-inf".to_string()
        } else {
            format!("{db:.1} dB")
        },
        egui::FontId::proportional(13.0),
        egui::Color32::WHITE,
    );
}

fn rec_meter_window(ctx: &egui::Context, p: &Project, s: &mut UiState) {
    if !s.dialogs.rec_meter {
        return;
    }
    let rows = s.audio_rec.input_levels(p);
    let mut open = true;
    egui::Window::new("Recording Meter")
        .id(egui::Id::unique("recording_meter"))
        .open(&mut open)
        .collapsible(false)
        .default_width(360.0)
        .show(ctx, |ui| {
            if rows.is_empty() {
                ui.label("Arm an audio track to see its input here.");
            }
            for (name, left, right) in rows {
                ui.label(egui::RichText::new(name).size(16.0));
                big_meter(ui, left);
                if let Some(right) = right {
                    big_meter(ui, right);
                }
            }
        });
    s.dialogs.rec_meter = open;
}

fn history_window(ctx: &egui::Context, s: &mut UiState) {
    if !s.dialogs.undo_history {
        return;
    }
    let mut open = true;
    let (undo, redo) = (s.undo_labels.clone(), s.redo_labels.clone());
    egui::Window::new("Undo History")
        .id(egui::Id::unique("undo_history"))
        .open(&mut open)
        .default_width(240.0)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .max_height(360.0)
                .show(ui, |ui| {
                    ui.label("Click a step to go back to just after it.");
                    for (i, label) in undo.iter().enumerate() {
                        if ui
                            .selectable_label(i + 1 == undo.len(), label.as_str())
                            .clicked()
                        {
                            s.dialogs.undo_steps = undo.len() - 1 - i;
                        }
                    }
                    for (i, label) in redo.iter().rev().enumerate() {
                        let text = egui::RichText::new(label.as_str()).weak();
                        if ui.selectable_label(false, text).clicked() {
                            s.dialogs.redo_steps = i + 1;
                        }
                    }
                    if undo.is_empty() && redo.is_empty() {
                        ui.label("Nothing yet.");
                    }
                });
        });
    if !open {
        s.dialogs.undo_history = false;
    }
}

fn project_settings_window(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    if !s.dialogs.project_settings {
        return;
    }
    let mut open = true;
    egui::Window::new("Project Settings")
        .id(egui::Id::unique("project_settings"))
        .open(&mut open)
        .default_width(320.0)
        .show(ctx, |ui| {
            ui.heading("General");
            let beat = p.position_beats;
            let mut tempo = p.tempo_at(beat);
            if ui
                .add(
                    egui::DragValue::new(&mut tempo)
                        .range(5.0..=990.0)
                        .speed(0.1)
                        .suffix(" BPM"),
                )
                .changed()
            {
                p.set_tempo_at(beat, tempo);
            }
            ui.separator();
            ui.heading("Metronome and recording");
            ui.checkbox(&mut p.metronome, "Metronome");
            ui.checkbox(&mut p.count_in, "Count-in before recording");
            ui.checkbox(
                &mut p.rec_replace,
                "Replace mode: a new recording takes away what it covers",
            );
            ui.checkbox(&mut p.auto_monitor, "Automatic input monitoring")
                .on_hover_text(
                    "Armed audio tracks are heard while stopped and while recording, and not when \
                     playing back. Use headphones with a microphone: speakers can feed back.",
                );
            ui.checkbox(&mut p.wait_for_sound, "Start recording when sound arrives")
                .on_hover_text(
                    "With Record and Play on, the take starts when the input passes this level.",
                );
            if p.wait_for_sound {
                ui.add(
                    egui::Slider::new(&mut p.trigger_db, -60.0..=-6.0)
                        .suffix(" dB")
                        .text("Sound level"),
                );
            }
            ui.checkbox(&mut p.low_latency, "Low Latency mode");
            ui.checkbox(&mut p.keep_plugins_running, "Keep idle plugins running")
                .on_hover_text(
                    "Plugins with nothing to process stop using CPU after a few seconds. \
                     Turn this on if one misbehaves, such as one that makes sound by itself.",
                );
            ui.separator();
            ui.heading("Synchronization");
            crate::sync_menu::items(ui, p);
        });
    s.dialogs.project_settings = open;
}

/// The windows.
pub fn show(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    groups_window(ctx, p, s);
    rename_alternative_window(ctx, p, s);
    about_window(ctx, s);
    customize_toolbar_window(ctx, s);
    crate::screensets::window(ctx, p, s);
    import_project_window(ctx, p, s);
    clean_up_window(ctx, s);
    plugin_manager_window(ctx, s);
    rename_region_window(ctx, p, s);
    track_header_window(ctx, s);
    icon_window(ctx, p, s);
    rename_project_window(ctx, s);
    go_to_window(ctx, p, s);
    rename_window(ctx, p, s);
    color_window(ctx, p, s);
    region_color_window(ctx, p, s);
    rec_meter_window(ctx, p, s);
    history_window(ctx, s);
    project_settings_window(ctx, p, s);
    crate::routing_matrix::show(ctx, p, s);
}

/// The Text tool: a new name for a region.
fn rename_region_window(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    let d = &mut s.dialogs;
    let Some((t, r, text)) = d.rename_region.as_mut() else {
        return;
    };
    let mut result = (false, false);
    let mut focus = std::mem::take(&mut d.focus);
    egui::Modal::new(egui::Id::unique("rename_region")).show(ctx, |ui| {
        ui.set_width(280.0);
        ui.label("Region name:");
        let edit = ui.add(egui::TextEdit::singleline(text));
        result = answer(ui, &mut focus, &edit);
    });
    let (ok, close) = result;
    if ok {
        let name = text.trim();
        let region = p.tracks.get_mut(*t).and_then(|x| x.regions.get_mut(*r));
        if let (false, Some(region)) = (name.is_empty(), region) {
            region.name = name.to_string();
        }
    }
    if ok || close {
        d.rename_region = None;
    }
}

/// File > Import > Other Projects: which tracks to bring.
fn import_project_window(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    let Some((from, picks)) = s.dialogs.import_project.as_mut() else {
        return;
    };
    let (mut import, mut open) = (false, true);
    egui::Window::new(format!("Import from {}", from.name))
        .id(egui::Id::unique("import_project"))
        .open(&mut open)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .max_height(320.0)
                .show(ui, |ui| {
                    for (t, on) in from.tracks.iter().zip(picks.iter_mut()) {
                        let movable = !matches!(
                            t.kind,
                            crate::model::TrackKind::Folder | crate::model::TrackKind::Vca
                        );
                        ui.add_enabled(movable, egui::Checkbox::new(on, t.name.as_str()));
                    }
                });
            ui.label("They come to Stereo Out, with no sends or groups, at the bars they were on.");
            import = ui.button("Import").clicked();
        });
    let mut message = None;
    if import {
        let n = p.import_tracks(from, picks);
        message = Some(format!("Imported {n} tracks"));
    }
    if import || !open {
        s.dialogs.import_project = None;
    }
    if message.is_some() {
        s.status = message;
    }
}

/// File > Clean Up: says what it found and deletes it when asked.
fn clean_up_window(ctx: &egui::Context, s: &mut UiState) {
    let Some(files) = s.dialogs.clean_up.as_ref() else {
        return;
    };
    let bytes: u64 = files
        .iter()
        .filter_map(|f| std::fs::metadata(f).ok())
        .map(|m| m.len())
        .sum();
    let (mut delete, mut open) = (false, true);
    egui::Window::new("Clean Up")
        .id(egui::Id::unique("clean_up"))
        .open(&mut open)
        .resizable(false)
        .show(ctx, |ui| {
            if files.is_empty() {
                ui.label("Nothing to clean up.");
                return;
            }
            ui.label(format!(
                "{} freeze files no track here is playing, {} MB.",
                files.len(),
                bytes / 1_000_000
            ));
            ui.label("A project open in another copy of Pro Wave could be using one.");
            delete = ui.button("Delete Them").clicked();
        });
    if delete {
        let failed = files
            .iter()
            .filter(|f| std::fs::remove_file(f).is_err())
            .count();
        s.status = Some(if failed == 0 {
            "Cleaned up".to_string()
        } else {
            format!("{failed} files could not be deleted")
        });
    }
    if delete || !open {
        s.dialogs.clean_up = None;
    }
}

/// Help > About Pro Wave.
fn about_window(ctx: &egui::Context, s: &mut UiState) {
    if !s.dialogs.about {
        return;
    }
    let mut open = true;
    egui::Window::new("About Pro Wave")
        .id(egui::Id::unique("about_pro_wave"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.heading("Pro Wave");
            ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
            ui.label("Free software under the GNU General Public License, version 3 or later.");
            ui.hyperlink("https://github.com/SakaBoluwatife/Pro-Wave");
            ui.label("Built on Pro Wave by fabox-os.");
        });
    s.dialogs.about = open;
}

/// View > Customize Toolbar: which entries the Toolbar has.
fn customize_toolbar_window(ctx: &egui::Context, s: &mut UiState) {
    if !s.dialogs.customize_toolbar {
        return;
    }
    let mut open = true;
    let mut changed = false;
    egui::Window::new("Customize Toolbar")
        .id(egui::Id::unique("customize_toolbar"))
        .open(&mut open)
        .resizable(false)
        .show(ctx, |ui| {
            for label in crate::side_panes::TOOLBAR_CHOICES {
                let mut on = s.toolbar.contains(label);
                if ui.checkbox(&mut on, crate::i18n::tr(label)).changed() {
                    crate::side_panes::set_toolbar_item(&mut s.toolbar, label, on);
                    changed = true;
                }
            }
        });
    if changed {
        crate::side_panes::save_toolbar(&s.toolbar);
    }
    s.dialogs.customize_toolbar = open;
}

/// Window > Plugin Manager: every plugin found and every file that failed, with where they are.
fn plugin_manager_window(ctx: &egui::Context, s: &mut UiState) {
    if !s.dialogs.plugin_manager {
        return;
    }
    let list = &mut s.plugins;
    list.clap.ensure_started(ctx);
    list.clap.drain();
    let mut open = true;
    egui::Window::new("Plugin Manager")
        .id(egui::Id::unique("plugin_manager"))
        .open(&mut open)
        .default_size([720.0, 460.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(format!(
                    "{} plugins, {} failed",
                    list.clap.plugins.len(),
                    list.clap.failed.len()
                ));
                if ui
                    .button("Rescan")
                    .on_hover_text("Look for new plugins and try the ones that failed again")
                    .clicked()
                {
                    crate::plugin_guard::clear_blocked();
                    list.clap.start(true, ctx);
                }
                if let Some((done, of)) = list.clap.progress {
                    ui.label(format!("Scanning {done} of {of}..."));
                }
            });
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Grid::new(egui::Id::unique("plugin_manager_grid"))
                    .striped(true)
                    .show(ui, |ui| {
                        for head in ["Name", "Maker", "Format", "Kind", "File"] {
                            ui.strong(head);
                        }
                        ui.end_row();
                        for info in &list.clap.plugins {
                            ui.label(&info.name);
                            ui.label(&info.vendor);
                            ui.label(info.format.name());
                            ui.label(if info.is_instrument {
                                "Instrument"
                            } else {
                                "Effect"
                            });
                            ui.label(info.path.display().to_string());
                            ui.end_row();
                        }
                    });
                if !list.clap.failed.is_empty() {
                    ui.separator();
                    ui.strong("Failed");
                    for (path, why) in &list.clap.failed {
                        ui.label(format!("{}: {why}", path.display()));
                    }
                }
            });
        });
    s.dialogs.plugin_manager = open;
}

/// Track > Configure Track Header: what the track headers show.
fn track_header_window(ctx: &egui::Context, s: &mut UiState) {
    if !s.dialogs.track_header {
        return;
    }
    let mut open = true;
    let o = &mut s.tracks.header_opts;
    egui::Window::new("Configure Track Header")
        .id(egui::Id::unique("track_header"))
        .open(&mut open)
        .resizable(false)
        .show(ctx, |ui| {
            ui.checkbox(&mut o.number, "Track number");
            ui.checkbox(&mut o.icon, "Icon");
            ui.checkbox(&mut o.buttons, "Mute, Solo, Record and Input buttons");
            ui.checkbox(&mut o.level, "Volume and pan");
        });
    s.dialogs.track_header = open;
}

/// Track > Assign Track Icon.
fn icon_window(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    let Some(track) = s.dialogs.icon else {
        return;
    };
    let Some(t) = p.tracks.get_mut(track) else {
        s.dialogs.icon = None;
        return;
    };
    let mut open = true;
    egui::Window::new(format!("Track Icon: {}", t.name))
        .id(egui::Id::unique("track_icon"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(210.0);
                for (kind, _) in crate::track_icons::CHOICES {
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::click());
                    crate::track_icons::draw_icon(ui.painter(), rect.shrink(2.0), kind);
                    if t.icon == Some(kind) {
                        ui.painter().rect_stroke(
                            rect,
                            3.0,
                            egui::Stroke::new(2.0, egui::Color32::WHITE),
                            egui::StrokeKind::Inside,
                        );
                    }
                    if resp.clicked() {
                        t.icon = Some(kind);
                    }
                }
            });
            if ui.button("Automatic").clicked() {
                t.icon = None;
            }
        });
    if !open {
        s.dialogs.icon = None;
    }
}

/// Mix > Edit Groups: each group's name, what it links, how many tracks it has.
fn groups_window(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    if !s.dialogs.groups {
        return;
    }
    let mut open = s.dialogs.groups;
    let mut remove = None;
    egui::Window::new("Edit Groups")
        .id(egui::Id::unique("edit_groups"))
        .open(&mut open)
        .show(ctx, |ui| {
            egui::Grid::new(egui::Id::unique("groups_grid")).show(ui, |ui| {
                for head in ["Name", "Volume", "Pan", "Mute", "Solo", "Tracks", ""] {
                    ui.label(head);
                }
                ui.end_row();
                for g in 0..p.groups.len() {
                    let members = p.tracks.iter().filter(|t| t.group == Some(g)).count();
                    let group = &mut p.groups[g];
                    ui.add(egui::TextEdit::singleline(&mut group.name).desired_width(140.0));
                    ui.checkbox(&mut group.volume, "");
                    ui.checkbox(&mut group.pan, "");
                    ui.checkbox(&mut group.mute, "");
                    ui.checkbox(&mut group.solo, "");
                    ui.label(members.to_string());
                    if ui.button("Delete").clicked() {
                        remove = Some(g);
                    }
                    ui.end_row();
                }
            });
        });
    if let Some(g) = remove {
        p.delete_group(g);
    }
    s.dialogs.groups = open;
}

/// Renames the version a track is playing.
fn rename_alternative_window(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    let Some((track, mut text)) = s.dialogs.rename_alternative.take() else {
        return;
    };
    let (mut ok, mut keep) = (false, true);
    egui::Window::new("Rename Alternative")
        .id(egui::Id::unique("rename_alternative"))
        .resizable(false)
        .show(ctx, |ui| {
            let edit = ui.text_edit_singleline(&mut text);
            let mut focus = false;
            let (done, close) = answer(ui, &mut focus, &edit);
            ok = done;
            keep = !done && !close;
        });
    if ok {
        let alt = p
            .tracks
            .get_mut(track)
            .and_then(|t| t.alternatives.get_mut(t.active_alt));
        if let Some(alt) = alt {
            alt.name = text.trim().chars().take(32).collect();
        }
    }
    if keep {
        s.dialogs.rename_alternative = Some((track, text));
    }
}

/// File > Project Management > Rename Project.
fn rename_project_window(ctx: &egui::Context, s: &mut UiState) {
    let Some(mut text) = s.dialogs.rename_project.take() else {
        return;
    };
    let (mut done, mut keep) = (None, true);
    egui::Window::new("Rename Project")
        .id(egui::Id::unique("rename_project"))
        .resizable(false)
        .show(ctx, |ui| {
            let edit = ui.text_edit_singleline(&mut text);
            let mut focus = false;
            let (ok, close) = answer(ui, &mut focus, &edit);
            if ok {
                done = Some(text.clone());
            }
            keep = !ok && !close;
        });
    if let Some(name) = done {
        s.file_task = Some(crate::file_commands::FileTask::Rename(name));
    }
    if keep {
        s.dialogs.rename_project = Some(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_position_is_typed_as_the_display_shows_it() {
        let p = Project::empty();
        assert_eq!(parse_position(&p, "1"), Some(0.0));
        assert_eq!(parse_position(&p, "3"), Some(8.0));
        assert_eq!(parse_position(&p, "3 2"), Some(9.0));
        assert_eq!(parse_position(&p, "3.2.3"), Some(9.5));
        assert_eq!(parse_position(&p, "3 2 3 121"), Some(9.5 + 0.125));
        assert_eq!(parse_position(&p, "0"), None);
        assert_eq!(parse_position(&p, ""), None);
        assert_eq!(parse_position(&p, "soon"), None);
    }
}
