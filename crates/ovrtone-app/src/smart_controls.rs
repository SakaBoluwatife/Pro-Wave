//! View > Show Smart Controls: eight knobs on the selected track's plugin, each on a parameter the
//! user picks, so the sound can be shaped without opening the plugin's window. A knob moves the plugin
//! the way automation does, and is recorded the same way when the strip is writing.
use crate::model::{AutoTarget, LearnTarget, Project};
use crate::UiState;
use egui::{vec2, Sense};
use std::collections::HashMap;

const KNOBS: usize = crate::model::SMART_KNOBS;
/// Drag this far to go from one end of a knob to the other.
const KNOB_TRAVEL: f32 = 150.0;

#[derive(Default)]
pub struct SmartControls {
    pub open: bool,
    /// The plugin shown, per track. Which parameter each knob is on goes with the plugin, in the
    /// project.
    plugin_of: HashMap<usize, u64>,
    /// Knob moves waiting for `App`: plugin, parameter, position.
    pub moves: Vec<(u64, u32, f64)>,
    /// Parameters whose position to read, and what was read (`App` does both).
    pub wanted: Vec<(u64, u32)>,
    pub values: HashMap<(u64, u32), f64>,
}

/// The knobs of a plugin as the user left them, or on its first parameters.
fn knobs_of(
    chosen: Option<[Option<u32>; KNOBS]>,
    params: &[ovrtone_host::PluginParam],
) -> [Option<u32>; KNOBS] {
    chosen.unwrap_or_else(|| {
        let mut first = [None; KNOBS];
        for (knob, param) in first.iter_mut().zip(params) {
            *knob = Some(param.id);
        }
        first
    })
}

/// The parameters a plugin's knobs were on, kept when a knob's parameter is changed.
fn keep_knobs(p: &mut Project, track: usize, uid: u64, knobs: [Option<u32>; KNOBS]) {
    let plugin = p
        .tracks
        .get_mut(track)
        .and_then(|t| t.hosted_mut().find(|i| i.uid == uid));
    if let Some(plugin) = plugin {
        plugin.smart_knobs = Some(knobs);
    }
}

/// Where a knob stands for a parameter at `value`, given what part of it the knob covers.
fn knob_of(value: f64, (lo, hi): (f32, f32)) -> f64 {
    let span = f64::from(hi - lo);
    if span.abs() < 1e-6 {
        return 0.0;
    }
    ((value - f64::from(lo)) / span).clamp(0.0, 1.0)
}

/// The parameter's value with the knob at `knob`.
fn value_of(knob: f64, (lo, hi): (f32, f32)) -> f64 {
    (f64::from(lo) + knob * f64::from(hi - lo)).clamp(0.0, 1.0)
}

/// The window.
pub fn show(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    let learn = &mut s.midi_learn;
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
            let plugins: Vec<(u64, String, std::sync::Arc<Vec<ovrtone_host::PluginParam>>)> = track
                .hosted()
                .map(|i| (i.uid, i.name.clone(), i.params.clone()))
                .collect();
            let stored = |uid: u64| track.hosted().find(|i| i.uid == uid)?.smart_knobs;
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
            let stored = stored(chosen);
            let was = knobs_of(stored, params);
            let mut knobs = was;
            let was_ranges = track
                .hosted()
                .find(|i| i.uid == chosen)
                .and_then(|i| i.smart_ranges)
                .unwrap_or([(0.0, 1.0); KNOBS]);
            let mut ranges = was_ranges;
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
                            let shown = knob_of(value.unwrap_or(0.0), ranges[k]) as f32;
                            crate::widgets::draw_knob(
                                ui.painter(),
                                rect.center(),
                                13.0,
                                shown * 2.0 - 1.0,
                            );
                            if let (Some(id), true) = (id, resp.dragged()) {
                                let knob = (f64::from(shown)
                                    - f64::from(resp.drag_delta().y / KNOB_TRAVEL))
                                .clamp(0.0, 1.0);
                                let moved = value_of(knob, ranges[k]);
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
                            // The next control moved on a MIDI device goes on this parameter.
                            if let Some(id) = id {
                                let target = LearnTarget::Plugin {
                                    uid: chosen,
                                    param: id,
                                };
                                let armed = learn.armed == Some(target);
                                if ui.selectable_label(armed, "Learn").clicked() {
                                    learn.armed = (!armed).then_some(target);
                                }
                            }
                            // What part of the parameter the knob's travel covers.
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                let (lo, hi) = &mut ranges[k];
                                for end in [lo, hi] {
                                    let mut percent = *end * 100.0;
                                    let drag = egui::DragValue::new(&mut percent)
                                        .range(0.0..=100.0)
                                        .speed(0.5)
                                        .max_decimals(0)
                                        .suffix("%");
                                    if ui
                                        .add_sized([46.0, 16.0], drag)
                                        .on_hover_text(
                                            "Low and high end of the knob. High below low turns \
                                             it around.",
                                        )
                                        .changed()
                                    {
                                        *end = percent / 100.0;
                                    }
                                }
                            });
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
            // Only a choice of the user's goes in the project.
            if knobs != was {
                keep_knobs(p, t, chosen, knobs);
            }
            if ranges != was_ranges {
                let plugin = p
                    .tracks
                    .get_mut(t)
                    .and_then(|t| t.hosted_mut().find(|i| i.uid == chosen));
                if let Some(plugin) = plugin {
                    plugin.smart_ranges = Some(ranges);
                }
            }
        });
    s.smart.open = open;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_knob_covers_the_part_of_its_parameter_it_was_given() {
        let part = (0.25, 0.75);
        assert_eq!(value_of(0.0, part), 0.25);
        assert_eq!(value_of(1.0, part), 0.75);
        assert_eq!(knob_of(0.5, part), 0.5);
        assert_eq!(
            knob_of(0.9, part),
            1.0,
            "past its end, the knob rests there"
        );
        let turned = (1.0, 0.0);
        assert_eq!(value_of(0.25, turned), 0.75);
        assert_eq!(knob_of(0.75, turned), 0.25);
        assert_eq!(knob_of(0.5, (0.5, 0.5)), 0.0, "no travel at all");
    }

    #[test]
    fn knobs_start_on_the_first_parameters_and_stay_where_the_user_put_them() {
        let param = |id: u32| ovrtone_host::PluginParam {
            id,
            name: format!("P{id}"),
        };
        let params: Vec<_> = (10..13).map(param).collect();
        let first = knobs_of(None, &params);
        assert_eq!(first[0], Some(10));
        assert_eq!(first[2], Some(12));
        assert_eq!(first[3], None, "fewer parameters than knobs");
        assert_eq!(knobs_of(Some([Some(12); KNOBS]), &params)[0], Some(12));
        assert_eq!(knobs_of(Some([None; KNOBS]), &params)[0], None, "all off");
    }

    #[test]
    fn a_choice_goes_with_the_plugin_it_was_made_for() {
        let info = ovrtone_host::PluginInfo {
            id: "c".into(),
            name: "Comp".into(),
            vendor: String::new(),
            version: String::new(),
            format: ovrtone_host::PluginFormat::Clap,
            path: "c.clap".into(),
            is_instrument: false,
        };
        let mut p = Project::empty();
        let t = p.add_track(crate::model::TrackKind::Audio);
        p.tracks[t]
            .plugins
            .push(crate::model::PluginInsert::new(&info));
        p.tracks[t]
            .plugins
            .push(crate::model::PluginInsert::new(&info));
        let (one, two) = (p.tracks[t].plugins[0].uid, p.tracks[t].plugins[1].uid);
        keep_knobs(&mut p, t, two, [Some(5); KNOBS]);
        keep_knobs(&mut p, t, 999, [Some(6); KNOBS]);
        assert_eq!(p.tracks[t].plugins[0].uid, one);
        assert_eq!(p.tracks[t].plugins[0].smart_knobs, None);
        assert_eq!(p.tracks[t].plugins[1].smart_knobs, Some([Some(5); KNOBS]));
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
