//! MIDI learn: hardware controls on faders, pans and plugin parameters. Arm a control, move a knob on
//! the device, and from then on that knob moves it. Assignments are kept in the project, by what they
//! move, so they follow tracks as they are moved around. Controllers (CC messages) only: a note or a
//! pitch wheel is not learned.
use crate::midi_in::Control;
use crate::model::{AutoTarget, LearnTarget, MidiMapping, Project};

/// What is armed, and whether the list of assignments is showing.
#[derive(Default)]
pub struct MidiLearn {
    /// The next controller moved goes on this.
    pub armed: Option<LearnTarget>,
    pub window: bool,
}

/// What `target` is called, for the list. `None` when what it moved is gone.
pub fn target_name(p: &Project, target: LearnTarget) -> Option<String> {
    let track = |t: usize| p.tracks.get(t).map(|x| x.name.as_str());
    Some(match target {
        LearnTarget::Volume(t) => format!("{} level", track(t)?),
        LearnTarget::Pan(t) => format!("{} pan", track(t)?),
        LearnTarget::MasterVolume => "Stereo Out level".to_string(),
        LearnTarget::Plugin { uid, param } => {
            let plugin = p
                .tracks
                .iter()
                .flat_map(|t| t.hosted())
                .chain(&p.master_plugins)
                .find(|i| i.uid == uid)?;
            let name = plugin
                .params
                .iter()
                .find(|x| x.id == param)
                .map_or_else(|| format!("parameter {param}"), |x| x.name.clone());
            format!("{} {name}", plugin.name)
        }
    })
}

/// A controller's value as a fader position, in dB on the fader's own scale.
fn level_of(value: u8) -> f32 {
    crate::mixer::frac_to_db(f32::from(value.min(127)) / 127.0).clamp(-60.0, 6.0)
}

/// A controller's value as a pan position. 64 is the centre, as it is on hardware.
fn pan_of(value: u8) -> f32 {
    let value = f32::from(value.min(127)) - 64.0;
    if value < 0.0 {
        value / 64.0
    } else {
        value / 63.0
    }
}

/// Moves a track's level or pan as its control would be moved, unless its automation is playing it.
fn move_strip(p: &mut Project, t: usize, target: AutoTarget, value: f32) {
    let (beat, playing) = (p.position_beats, p.playing);
    let Some(track) = p.tracks.get_mut(t) else {
        return;
    };
    if !crate::automation::editable(track, target) {
        return;
    }
    crate::automation::moved(track, target, value);
    // A knob has no moment of letting go: each message is a whole gesture.
    crate::automation::released(track, target, beat, playing);
}

/// Takes what came from the MIDI devices: learns the first controller moved while something is armed,
/// and moves what the others are assigned to. Plugin moves go in `plugin_moves` for `App`. What was
/// learned, in words for the status line.
pub fn apply(
    p: &mut Project,
    learn: &mut MidiLearn,
    controls: &[Control],
    plugin_moves: &mut Vec<(u64, u32, f64)>,
) -> Option<String> {
    let mut said = None;
    for control in controls {
        if let Some(target) = learn.armed.take() {
            p.midi_map.retain(|m| {
                m.target != target
                    && (m.channel, m.controller) != (control.channel, control.controller)
            });
            p.midi_map.push(MidiMapping {
                channel: control.channel,
                controller: control.controller,
                target,
            });
            let name = target_name(p, target).unwrap_or_default();
            said = Some(format!(
                "Controller {} on channel {} now moves {name}",
                control.controller,
                control.channel + 1
            ));
            continue;
        }
        let assigned = p
            .midi_map
            .iter()
            .find(|m| (m.channel, m.controller) == (control.channel, control.controller))
            .map(|m| m.target);
        match assigned {
            Some(LearnTarget::Volume(t)) => {
                let Some(track) = p.tracks.get(t) else {
                    continue;
                };
                let to = level_of(control.value);
                // Its mix group moves with it, by the same amount, as with the fader.
                let delta = to - track.volume_db;
                move_strip(p, t, AutoTarget::Volume, to);
                for j in p.group_mates(t, |g| g.volume) {
                    let mate = p.tracks[j].volume_db + delta;
                    move_strip(p, j, AutoTarget::Volume, mate);
                }
            }
            Some(LearnTarget::Pan(t)) => {
                move_strip(p, t, AutoTarget::Pan, pan_of(control.value));
            }
            Some(LearnTarget::MasterVolume) => {
                p.master_vol = 10f32.powf(level_of(control.value) / 20.0).min(2.0);
            }
            Some(LearnTarget::Plugin { uid, param }) => {
                let value = f64::from(control.value.min(127)) / 127.0;
                plugin_moves.push((uid, param, value));
                let (beat, playing) = (p.position_beats, p.playing);
                let home = p
                    .tracks
                    .iter_mut()
                    .find(|t| t.hosted().any(|i| i.uid == uid));
                if let Some(track) = home {
                    let target = AutoTarget::Plugin { uid, param };
                    crate::automation::plugin_moved(track, target, value as f32, beat, playing);
                }
            }
            None => {}
        }
    }
    said
}

/// The window while something is armed, and the list of assignments.
pub fn show(ctx: &egui::Context, p: &mut Project, s: &mut crate::UiState) {
    // What the strips' menus asked for, in the Mixer or the Inspector.
    if let Some(target) = s.mixer.learn.take() {
        s.midi_learn.armed = Some(target);
    }
    s.midi_learn.window |= std::mem::take(&mut s.mixer.learn_window);
    let learn = &mut s.midi_learn;
    if let Some(target) = learn.armed {
        let name = target_name(p, target).unwrap_or_default();
        let mut cancel = false;
        egui::Window::new("MIDI Learn")
            .id(egui::Id::unique("midi_learn_armed"))
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!("Move a control on your MIDI device for {name}."));
                cancel = ui.button("Cancel").clicked();
            });
        if cancel {
            learn.armed = None;
        }
        // The device gives no other reason to draw.
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
    if !learn.window {
        return;
    }
    let mut open = true;
    let mut remove = None;
    let mut arm_master = false;
    egui::Window::new("MIDI Assignments")
        .id(egui::Id::unique("midi_assignments"))
        .open(&mut open)
        .default_width(340.0)
        .show(ctx, |ui| {
            if p.midi_map.is_empty() {
                ui.label(
                    "Nothing is assigned. Right-click a channel strip, or use Learn under a \
                     Smart Controls knob.",
                );
            }
            egui::Grid::new(egui::Id::unique("midi_assignments_grid"))
                .num_columns(3)
                .striped(true)
                .show(ui, |ui| {
                    for (k, m) in p.midi_map.iter().enumerate() {
                        ui.label(format!("Ch {} CC {}", m.channel + 1, m.controller));
                        match target_name(p, m.target) {
                            Some(name) => ui.label(name),
                            None => ui.weak("(no longer in the project)"),
                        };
                        if ui.small_button("Remove").clicked() {
                            remove = Some(k);
                        }
                        ui.end_row();
                    }
                });
            ui.separator();
            arm_master = ui.button("Learn Stereo Out level").clicked();
        });
    if let Some(k) = remove {
        p.midi_map.remove(k);
    }
    if arm_master {
        learn.armed = Some(LearnTarget::MasterVolume);
    }
    learn.window = open;
}
