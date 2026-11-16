//! The clip launcher (F10): a grid of cells, a column per track and a row per scene, as Ableton's
//! Session view and Logic's Live Loops. A cell holds a region (MIDI, pattern or audio) to loop.
//! Click a cell and it starts on the next bar, looping, in place of the track's arrangement; click
//! a scene's button and every cell of that row starts together. The square under a column stops
//! that track, and Back to Arrangement stops them all.
//!
//! Fill a cell from the Tracks area: select a region, then right-click an empty cell of its track
//! and choose Put the Selected Region Here.

use crate::model::{Project, Region, TrackKind};
use crate::UiState;
use egui::{vec2, Color32, RichText};

/// Scenes a new grid has.
const SCENES: usize = 8;
/// Repeats laid out when a cell starts; more are laid out as the song nears their end.
const LOOPS: usize = 32;

/// The launcher window, while it is open.
#[derive(Default)]
pub struct LauncherView {
    pub open: bool,
}

fn takes_cells(kind: TrackKind) -> bool {
    matches!(kind, TrackKind::Instrument | TrackKind::Audio | TrackKind::ExternalMidi | TrackKind::Drummer)
}

/// The bar a cell started now begins on: the next bar while playing, this one while stopped.
fn launch_bar(p: &Project) -> f64 {
    let bar = p.beats_to_bar(p.position_beats.max(0.0));
    if p.playing {
        bar.floor() + 1.0
    } else {
        bar.floor()
    }
}

/// Lays out `cell` from bar `from` (0-based) as repeats.
fn loops_of(cell: &Region, from: f64) -> Vec<Region> {
    let len = f64::from(cell.len_bars.max(0.25));
    (0..LOOPS)
        .map(|k| {
            let mut r = cell.clone();
            r.start_bar = (from + k as f64 * len) as f32 + 1.0;
            r.params.mute = false;
            r
        })
        .collect()
}

/// Starts the cell of `scene` on track `t`. False when the cell is empty.
pub fn launch(p: &mut Project, t: usize, scene: usize) -> bool {
    let from = launch_bar(p);
    let Some(cell) = p.tracks.get(t).and_then(|tr| tr.cells.get(scene)).cloned().flatten() else {
        return false;
    };
    let track = &mut p.tracks[t];
    track.launch_loops = loops_of(&cell, from);
    track.launched = Some(scene);
    p.song_rev += 1;
    true
}

/// Stops what is launched on track `t`: its arrangement plays again.
pub fn stop(p: &mut Project, t: usize) {
    if let Some(track) = p.tracks.get_mut(t) {
        if track.launched.take().is_some() {
            track.launch_loops.clear();
            p.song_rev += 1;
        }
    }
}

/// Once a frame: a launched cell close to the end of its repeats gets more.
pub fn keep_going(p: &mut Project) {
    let now_bar = p.beats_to_bar(p.position_beats);
    let mut changed = false;
    for t in &mut p.tracks {
        let (Some(scene), Some(last)) = (t.launched, t.launch_loops.last()) else {
            continue;
        };
        if now_bar + 4.0 < f64::from(last.start_bar) {
            continue;
        }
        if let Some(Some(cell)) = t.cells.get(scene) {
            let first = f64::from(t.launch_loops[0].start_bar) - 1.0;
            let len = f64::from(cell.len_bars.max(0.25));
            // Carries on from a whole number of repeats before now, so the loop keeps its place.
            let done = ((now_bar - first) / len).floor().max(0.0);
            t.launch_loops = loops_of(cell, first + done * len);
            changed = true;
        }
    }
    if changed {
        p.song_rev += 1;
    }
}

pub fn window(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F10)) {
        s.launcher.open = !s.launcher.open;
    }
    keep_going(p);
    if !s.launcher.open {
        return;
    }
    let mut open = true;
    let selected = s.tracks.selected;
    egui::Window::new("Clip Launcher")
        .open(&mut open)
        .default_size(vec2(760.0, 420.0))
        .resizable(true)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Back to Arrangement").clicked() {
                    for t in 0..p.tracks.len() {
                        stop(p, t);
                    }
                }
                ui.weak("Cells start on the next bar. Right-click a cell to fill or clear it.");
            });
            ui.separator();
            let columns: Vec<usize> = (0..p.tracks.len()).filter(|t| takes_cells(p.tracks[*t].kind)).collect();
            let scenes = p
                .tracks
                .iter()
                .map(|t| t.cells.len())
                .max()
                .unwrap_or(0)
                .max(SCENES);
            egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                egui::Grid::new(egui::Id::unique("launcher_grid"))
                    .spacing(vec2(4.0, 4.0))
                    .show(ui, |ui| {
                        // The track names over their columns.
                        ui.label("");
                        for &t in &columns {
                            let name: String = p.tracks[t].name.chars().take(14).collect();
                            ui.add_sized([96.0, 18.0], egui::Label::new(RichText::new(name).strong()).truncate());
                        }
                        ui.end_row();
                        for scene in 0..scenes {
                            if ui
                                .add_sized([70.0, 30.0], egui::Button::new(format!("Scene {} \u{25b6}", scene + 1)))
                                .on_hover_text("Start every cell of this row on the next bar")
                                .clicked()
                            {
                                for &t in &columns {
                                    if !launch(p, t, scene) {
                                        stop(p, t);
                                    }
                                }
                            }
                            for &t in &columns {
                                cell(ui, p, t, scene, selected);
                            }
                            ui.end_row();
                        }
                        // A stop square under each column.
                        ui.label("");
                        for &t in &columns {
                            let playing = p.tracks[t].launched.is_some();
                            let stop_button = egui::Button::new("\u{25a0}").fill(if playing {
                                crate::theme::gray(0x50)
                            } else {
                                crate::theme::gray(0x2a)
                            });
                            if ui.add_sized([96.0, 22.0], stop_button).on_hover_text("Stop this track's cell").clicked() {
                                stop(p, t);
                            }
                        }
                        ui.end_row();
                    });
            });
        });
    if !open {
        s.launcher.open = false;
    }
}

fn cell(ui: &mut egui::Ui, p: &mut Project, t: usize, scene: usize, selected: Option<(usize, usize)>) {
    let track = &p.tracks[t];
    let content = track.cells.get(scene).cloned().flatten();
    let playing = track.launched == Some(scene);
    let color = track.color;
    let label = content.as_ref().map_or(String::new(), |r| {
        let name: String = r.name.chars().take(12).collect();
        format!("{} {name}", if playing { "\u{25b6}" } else { "\u{25b7}" })
    });
    let fill = match (&content, playing) {
        (None, _) => crate::theme::gray(0x24),
        (Some(_), true) => color,
        (Some(_), false) => color.gamma_multiply(0.45),
    };
    let text = if playing { Color32::BLACK } else { crate::theme::text() };
    let resp = ui.add_sized(
        [96.0, 30.0],
        egui::Button::new(RichText::new(label).color(text)).fill(fill).corner_radius(4.0),
    );
    if resp.clicked() && content.is_some() {
        if playing {
            stop(p, t);
        } else {
            launch(p, t, scene);
        }
    }
    resp.context_menu(|ui| {
        let from_selection = selected.filter(|(st, _)| *st == t);
        if ui
            .add_enabled(from_selection.is_some(), egui::Button::new("Put the Selected Region Here"))
            .on_disabled_hover_text("Select a region of this track in the Tracks area first")
            .clicked()
        {
            if let Some((_, r)) = from_selection {
                if let Some(region) = p.tracks[t].regions.get(r).cloned() {
                    let cells = &mut p.tracks[t].cells;
                    if cells.len() <= scene {
                        cells.resize(scene + 1, None);
                    }
                    cells[scene] = Some(region);
                }
            }
            ui.close();
        }
        if ui.add_enabled(content.is_some(), egui::Button::new("Clear Cell")).clicked() {
            if p.tracks[t].launched == Some(scene) {
                stop(p, t);
            }
            if let Some(c) = p.tracks[t].cells.get_mut(scene) {
                *c = None;
            }
            ui.close();
        }
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_launched_cell_loops_from_the_next_bar_and_stopping_gives_the_arrangement_back() {
        let mut p = Project::empty();
        let r = p.add_midi_region(0, 0.0).unwrap();
        let mut cell = p.tracks[0].regions[r].clone();
        cell.len_bars = 2.0;
        p.tracks[0].cells = vec![None, Some(cell)];
        p.playing = true;
        p.position_beats = p.bar_to_beats(4.5);
        assert!(launch(&mut p, 0, 1));
        let loops = &p.tracks[0].launch_loops;
        assert_eq!(loops[0].start_bar, 6.0, "the bar after the one playing, from 1");
        assert_eq!(loops[1].start_bar, 8.0, "two bars a repeat");
        assert!(std::ptr::eq(p.tracks[0].sounding_regions(), p.tracks[0].launch_loops.as_slice()));
        stop(&mut p, 0);
        assert!(p.tracks[0].launched.is_none());
        assert!(!launch(&mut p, 0, 0), "an empty cell does nothing");
    }
}
