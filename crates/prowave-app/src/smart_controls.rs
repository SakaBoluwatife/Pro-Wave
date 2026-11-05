//! View > Show Smart Controls: eight knobs on the selected track's plugin, each on a parameter the
//! user picks, so the sound can be shaped without opening the plugin's window. A knob moves the plugin
//! the way automation does, and is recorded the same way when the strip is writing.
use crate::model::{AutoTarget, Project};
use crate::UiState;
use egui::{vec2, Sense};
use std::collections::HashMap;

const KNOBS: usize = 8;
/// Drag this far to go from one end of a knob to the other.
const KNOB_TRAVEL: f32 = 150.0;

#[derive(Default)]
pub struct SmartControls {
    pub open: bool,
    /// The plugin shown, per track, and which parameter each knob is on, per plugin.
    plugin_of: HashMap<usize, u64>,
    knobs: HashMap<u64, [Option<u32>; KNOBS]>,
    /// Knob moves waiting for `App`: plugin, parameter, position.
    pub moves: Vec<(u64, u32, f64)>,
    /// Parameters whose position to read, and what was read (`App` does both).
    pub wanted: Vec<(u64, u32)>,
    pub values: HashMap<(u64, u32), f64>,
}

/// The knobs of a plugin as the user left them, or on its first parameters.
fn knobs_of(
    knobs: &mut HashMap<u64, [Option<u32>; KNOBS]>,
    uid: u64,
    params: &[prowave_host::PluginParam],
) -> [Option<u32>; KNOBS] {
    *knobs.entry(uid).or_insert_with(|| {
        let mut first = [None; KNOBS];
        for (knob, param) in first.iter_mut().zip(params) {
            *knob = Some(param.id);
        }
        first
    })
}

/// The window.
pub fn show(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    let v = &mut s.smart;
    v.wanted.clear();
    if !v.open {
        return;
    }
    let mut open = true;
    let t = p.selected_track;
    egui::Window::new("Smart Controls")
        .id(egui::Id::unique("smart_controls"))
        .open(&mut open)
        .default_width(460.0)
        .show(ctx, |ui| {
            let Some(track) = p.tracks.get(t) else {
                ui.label("Select a track.");
                return;
            };
            let plugins: Vec<(u64, String, std::sync::Arc<Vec<prowave_host::PluginParam>>)> = track
                .hosted()
                .map(|i| (i.uid, i.name.clone(), i.params.clone()))
                .collect();
            if plugins.is_empty() {
                ui.label(format!("{} has no plugins.", track.name));
                return;
            }
            let chosen = v
                .plugin_of
                .get(&t)
                .copied()
                .filter(|uid| plugins.iter().any(|x| x.0 == *uid))
                .unwrap_or(plugins[0].0);
            let mut pick = chosen;
            let Some(plugin) = plugins.iter().find(|x| x.0 == chosen) else {
                return;
            };
            egui::ComboBox::from_id_salt(egui::Id::unique("smart_plugin"))
                .selected_text(plugin.1.as_str())
                .width(240.0)
                .show_ui(ui, |ui| {
                    for (uid, name, _) in &plugins {
                        ui.selectable_value(&mut pick, *uid, name.as_str());
                    }
                });
            v.plugin_of.insert(t, pick);
            let params = &plugin.2;
            if params.is_empty() {
                ui.label("This plugin offers no parameters, or has not loaded yet.");
                return;
            }
            let mut knobs = knobs_of(&mut v.knobs, chosen, params);
            ui.add_space(6.0);
            let (beat, playing) = (p.position_beats, p.playing);
            egui::Grid::new(egui::Id::unique("smart_knobs"))
                .num_columns(4)
                .show(ui, |ui| {
                    for (k, slot) in knobs.iter_mut().enumerate() {
                        ui.vertical(|ui| {
                            ui.set_width(100.0);
                            let (rect, resp) =
                                ui.allocate_exact_size(vec2(100.0, 40.0), Sense::drag());
                            let id = *slot;
                            let value = id.and_then(|id| v.values.get(&(chosen, id)).copied());
                            let shown = value.unwrap_or(0.0) as f32;
                            crate::widgets::draw_knob(
                                ui.painter(),
                                rect.center(),
                                13.0,
                                shown * 2.0 - 1.0,
                            );
                            if let (Some(id), true) = (id, resp.dragged()) {
                                let moved = (f64::from(shown)
                                    - f64::from(resp.drag_delta().y / KNOB_TRAVEL))
                                .clamp(0.0, 1.0);
                                v.values.insert((chosen, id), moved);
                                v.moves.push((chosen, id, moved));
                                if let Some(track) = p.tracks.get_mut(t) {
                                    let target = AutoTarget::Plugin {
                                        uid: chosen,
                                        param: id,
                                    };
                                    crate::automation::plugin_moved(
                                        track,
                                        target,
                                        moved as f32,
                                        beat,
                                        playing,
                                    );
                                }
                            }
                            if let Some(id) = id {
                                v.wanted.push((chosen, id));
                            }
                            let name = id
                                .and_then(|id| params.iter().find(|x| x.id == id))
                                .map_or("Off", |x| x.name.as_str());
                            egui::ComboBox::from_id_salt(egui::Id::unique(("smart_knob", k)))
                                .selected_text(egui::RichText::new(name).small())
                                .width(94.0)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(slot, None, "Off");
                                    for param in params.iter() {
                                        ui.selectable_value(
                                            slot,
                                            Some(param.id),
                                            param.name.as_str(),
                                        );
                                    }
                                });
                        });
                        if k % 4 == 3 {
                            ui.end_row();
                        }
                    }
                });
            v.knobs.insert(chosen, knobs);
        });
    s.smart.open = open;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knobs_start_on_the_first_parameters_and_stay_where_the_user_put_them() {
        let param = |id: u32| prowave_host::PluginParam {
            id,
            name: format!("P{id}"),
        };
        let mut knobs = HashMap::new();
        let params: Vec<_> = (10..13).map(param).collect();
        let first = knobs_of(&mut knobs, 1, &params);
        assert_eq!(first[0], Some(10));
        assert_eq!(first[2], Some(12));
        assert_eq!(first[3], None, "fewer parameters than knobs");
        knobs.insert(1, [Some(12); KNOBS]);
        assert_eq!(knobs_of(&mut knobs, 1, &params)[0], Some(12));
    }

    #[test]
    fn it_draws_with_nothing_to_show() {
        let mut p = Project::empty();
        let mut s = UiState::new();
        s.smart.open = true;
        let mut h = crate::ui_test::Harness::new(800.0, 600.0);
        h.settle(|ui| show(ui.ctx(), &mut p, &mut s));
        assert!(s.smart.open);
    }
}
