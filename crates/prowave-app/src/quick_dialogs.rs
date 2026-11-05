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
    pub undo_history: bool,
    pub routing: bool,
    pub project_settings: bool,
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

const TRACK_COLORS: [egui::Color32; 12] = [
    egui::Color32::from_rgb(0xd9, 0x4f, 0x4f),
    egui::Color32::from_rgb(0xe0, 0x82, 0x3a),
    egui::Color32::from_rgb(0xd9, 0xb4, 0x3a),
    egui::Color32::from_rgb(0x8f, 0xb8, 0x3f),
    egui::Color32::from_rgb(0x3f, 0xa8, 0x5a),
    egui::Color32::from_rgb(0x2f, 0xa8, 0x9a),
    egui::Color32::from_rgb(0x3f, 0x9f, 0xd0),
    egui::Color32::from_rgb(0x3f, 0x5f, 0xc0),
    egui::Color32::from_rgb(0x7a, 0x4f, 0xc0),
    egui::Color32::from_rgb(0xb0, 0x4f, 0xb8),
    egui::Color32::from_rgb(0xd0, 0x5a, 0x8f),
    egui::Color32::from_rgb(0x8a, 0x8a, 0x8a),
];

fn color_window(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    let Some(track) = s.dialogs.color else {
        return;
    };
    let Some(t) = p.tracks.get_mut(track) else {
        s.dialogs.color = None;
        return;
    };
    let mut open = true;
    egui::Window::new(format!("Track Color: {}", t.name))
        .id(egui::Id::unique("track_color"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(190.0);
                for color in TRACK_COLORS {
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());
                    ui.painter().rect_filled(rect, 3.0, color);
                    if t.color == color {
                        ui.painter().rect_stroke(
                            rect,
                            3.0,
                            egui::Stroke::new(2.0, egui::Color32::WHITE),
                            egui::StrokeKind::Inside,
                        );
                    }
                    if resp.clicked() {
                        t.color = color;
                    }
                }
            });
            ui.color_edit_button_srgba(&mut t.color);
        });
    if !open {
        s.dialogs.color = None;
    }
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
            ui.checkbox(&mut p.low_latency, "Low Latency mode");
            ui.separator();
            ui.heading("Synchronization");
            crate::sync_menu::items(ui, p);
        });
    s.dialogs.project_settings = open;
}

/// The windows.
pub fn show(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    go_to_window(ctx, p, s);
    rename_window(ctx, p, s);
    color_window(ctx, p, s);
    history_window(ctx, s);
    project_settings_window(ctx, p, s);
    crate::routing_matrix::show(ctx, p, s);
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
