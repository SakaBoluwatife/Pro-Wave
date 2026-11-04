//! Key commands, all in one table: Logic Pro's default keys as they map onto Windows, Cmd becoming
//! Ctrl and Option becoming Alt. The menus show them from here too.
//!
//! With the pointer over the piano roll its FL Studio keys win, so most of these wait until the
//! pointer is elsewhere. Text boxes keep their letters, and while the computer keyboard plays notes
//! (Ctrl+K) its note keys never get this far.
use crate::model::{Project, TrackKind};
use crate::tracks_area::{run_edit, EditCmd};
use crate::UiState;
use egui::{Key, Modifiers};

const NONE: Modifiers = Modifiers::NONE;
const CTRL: Modifiers = Modifiers::COMMAND;
const SHIFT: Modifiers = Modifiers::SHIFT;

fn ctrl_alt() -> Modifiers {
    Modifiers::COMMAND | Modifiers::ALT
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// A menu entry, by its label. It does nothing while the menu has it greyed.
    Menu(&'static str),
    Edit(EditCmd),
    Repeat,
    SelectAll,
    Cycle,
    Metronome,
    MuteTrack,
    SoloTrack,
    MuteRegions,
    PreviousTrack,
    NextTrack,
    BarBack,
    BarForward,
    NewAudioTrack,
    NewInstrumentTrack,
}

#[derive(Clone, Copy, Debug)]
pub struct Command {
    pub key: Key,
    pub modifiers: fn() -> Modifiers,
    pub action: Action,
}

const fn cmd(key: Key, modifiers: fn() -> Modifiers, action: Action) -> Command {
    Command {
        key,
        modifiers,
        action,
    }
}

fn none() -> Modifiers {
    NONE
}
fn ctrl() -> Modifiers {
    CTRL
}
fn shift() -> Modifiers {
    SHIFT
}

/// The whole table. Space (play) and the Undo keys are handled where they always were.
pub const COMMANDS: &[Command] = &[
    // Transport.
    cmd(Key::R, none, Action::Menu("Record")),
    cmd(Key::R, shift, Action::Menu("Capture Recording")),
    cmd(Key::Enter, none, Action::Menu("Go to Beginning")),
    cmd(Key::C, none, Action::Cycle),
    cmd(Key::K, none, Action::Metronome),
    cmd(Key::Comma, none, Action::BarBack),
    cmd(Key::Period, none, Action::BarForward),
    // Tracks.
    cmd(Key::M, none, Action::MuteTrack),
    cmd(Key::S, none, Action::SoloTrack),
    cmd(Key::ArrowUp, none, Action::PreviousTrack),
    cmd(Key::ArrowDown, none, Action::NextTrack),
    cmd(Key::A, ctrl_alt, Action::NewAudioTrack),
    cmd(Key::S, ctrl_alt, Action::NewInstrumentTrack),
    cmd(Key::Backspace, ctrl, Action::Menu("Delete Track")),
    // Regions.
    cmd(Key::X, ctrl, Action::Edit(EditCmd::Cut)),
    cmd(Key::C, ctrl, Action::Edit(EditCmd::Copy)),
    cmd(Key::V, ctrl, Action::Edit(EditCmd::Paste)),
    cmd(Key::T, ctrl, Action::Edit(EditCmd::Split)),
    cmd(Key::J, ctrl, Action::Edit(EditCmd::Join)),
    cmd(Key::R, ctrl, Action::Repeat),
    cmd(Key::A, ctrl, Action::SelectAll),
    cmd(Key::M, ctrl, Action::MuteRegions),
    // Views.
    cmd(Key::X, none, Action::Menu("Show Mixer")),
    cmd(Key::Y, none, Action::Menu("Show Library")),
    cmd(Key::I, none, Action::Menu("Show Inspector")),
    cmd(Key::P, none, Action::Menu("Show Piano Roll")),
    cmd(Key::E, none, Action::Menu("Show Piano Roll")),
    cmd(Key::D, none, Action::Menu("Show List Editors")),
    cmd(Key::F, none, Action::Menu("Show Media Browser")),
    cmd(Key::Z, none, Action::Menu("Zoom to Fit")),
    cmd(Key::ArrowRight, ctrl, Action::Menu("Zoom In")),
    cmd(Key::ArrowLeft, ctrl, Action::Menu("Zoom Out")),
];

/// The menu label an action has, if it has one.
fn label_of(action: Action) -> Option<&'static str> {
    Some(match action {
        Action::Menu(label) => label,
        Action::Edit(EditCmd::Cut) => "Cut",
        Action::Edit(EditCmd::Copy) => "Copy",
        Action::Edit(EditCmd::Paste) => "Paste",
        Action::Edit(EditCmd::Split) => "Split at Playhead",
        Action::Edit(EditCmd::Join) => "Join",
        Action::SelectAll => "Select All",
        _ => return None,
    })
}

/// The key shown beside a menu entry.
pub fn shortcut_of(label: &str) -> Option<egui::KeyboardShortcut> {
    COMMANDS
        .iter()
        .find(|c| label_of(c.action) == Some(label))
        .map(|c| egui::KeyboardShortcut::new((c.modifiers)(), c.key))
}

/// Runs what this frame's keys ask for.
pub fn run(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    if ctx.egui_wants_keyboard_input() || s.piano_focus {
        return;
    }
    for command in COMMANDS {
        let mods = (command.modifiers)();
        let pressed =
            ctx.input_mut(|i| i.modifiers.matches_exact(mods) && i.consume_key(mods, command.key));
        if pressed {
            perform(command.action, p, s);
        }
    }
}

/// Moves the playhead to the bar `by` bars from the one it is in.
fn move_by_bars(p: &mut Project, by: f64) {
    let bar = p.beats_to_bar(p.position_beats);
    let target = if by < 0.0 && bar > bar.floor() {
        bar.floor()
    } else {
        (bar.floor() + by).max(0.0)
    };
    p.position_beats = p.bar_to_beats(target);
}

pub fn perform(action: Action, p: &mut Project, s: &mut UiState) {
    let selected = p.selected_track;
    match action {
        Action::Menu(label) => {
            if crate::menus::enabled(label, p, s) {
                crate::menus::apply(label, p, s);
            }
        }
        Action::Edit(cmd) => run_edit(p, &mut s.tracks, &mut s.status, cmd),
        Action::Repeat => crate::tracks_area::repeat_selected(p, &mut s.tracks),
        Action::SelectAll => s.tracks.select_all(p),
        Action::Cycle => p.cycle = !p.cycle,
        Action::Metronome => p.metronome = !p.metronome,
        Action::MuteTrack => {
            if let Some(t) = p.tracks.get_mut(selected) {
                t.mute = !t.mute;
            }
        }
        Action::SoloTrack => {
            if let Some(t) = p.tracks.get_mut(selected) {
                t.solo = !t.solo;
            }
        }
        Action::MuteRegions => {
            for (t, r) in s.tracks.all_selected() {
                if let Some(region) = p.tracks.get_mut(t).and_then(|x| x.regions.get_mut(r)) {
                    region.params.mute = !region.params.mute;
                }
            }
            p.song_rev += 1;
        }
        Action::PreviousTrack => {
            p.selected_track = selected.saturating_sub(1);
            s.tracks.track_focus = true;
        }
        Action::NextTrack => {
            p.selected_track = (selected + 1).min(p.tracks.len().saturating_sub(1));
            s.tracks.track_focus = true;
        }
        Action::BarBack => move_by_bars(p, -1.0),
        Action::BarForward => move_by_bars(p, 1.0),
        Action::NewAudioTrack => {
            p.add_track(TrackKind::Audio);
        }
        Action::NewInstrumentTrack => {
            p.add_track(TrackKind::Instrument);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use egui::{Event, RawInput};

    fn press(ctx: &egui::Context, key: Key, mods: Modifiers, p: &mut Project, s: &mut UiState) {
        let events = vec![
            Event::ModifiersChanged(mods),
            Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: mods,
            },
        ];
        let mut out = ctx.run_ui(
            RawInput {
                events,
                ..RawInput::default()
            },
            |ui| run(ui.ctx(), p, s),
        );
        out.textures_delta.clear();
        let up = vec![
            Event::Key {
                key,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: mods,
            },
            Event::ModifiersChanged(Modifiers::NONE),
        ];
        let mut out = ctx.run_ui(
            RawInput {
                events: up,
                ..RawInput::default()
            },
            |_| {},
        );
        out.textures_delta.clear();
    }

    #[test]
    fn no_two_commands_share_keys() {
        let all: Vec<String> = COMMANDS
            .iter()
            .map(|c| format!("{:?} {:?}", (c.modifiers)(), c.key))
            .collect();
        for (i, one) in all.iter().enumerate() {
            assert!(!all[i + 1..].contains(one), "{one} twice");
        }
    }

    #[test]
    fn every_menu_label_in_the_table_is_a_real_menu_entry() {
        let entries = crate::menus::all_entries();
        for c in COMMANDS {
            if let Some(label) = label_of(c.action) {
                assert!(entries.contains(&label), "{label}");
            }
        }
    }

    #[test]
    fn logics_keys_do_what_they_do_there() {
        let ctx = egui::Context::default();
        let mut p = Project::demo();
        let mut s = UiState::new();
        p.selected_track = 0;

        press(&ctx, Key::C, NONE, &mut p, &mut s);
        assert!(p.cycle);
        press(&ctx, Key::K, NONE, &mut p, &mut s);
        assert!(p.metronome);
        press(&ctx, Key::M, NONE, &mut p, &mut s);
        assert!(p.tracks[0].mute);
        press(&ctx, Key::S, NONE, &mut p, &mut s);
        assert!(p.tracks[0].solo);
        press(&ctx, Key::ArrowDown, NONE, &mut p, &mut s);
        assert_eq!(p.selected_track, 1);
        press(&ctx, Key::ArrowUp, NONE, &mut p, &mut s);
        assert_eq!(p.selected_track, 0);

        // Ctrl+C copies, plain C toggles the cycle: the modifiers count exactly.
        s.tracks.selected = Some((0, 0));
        let cycle = p.cycle;
        press(&ctx, Key::C, CTRL, &mut p, &mut s);
        assert_eq!(p.cycle, cycle);
        assert_eq!(s.tracks.clipboard.len(), 1);

        // Bars.
        p.position_beats = 0.0;
        press(&ctx, Key::Period, NONE, &mut p, &mut s);
        assert_eq!(p.position_beats, 4.0);
        press(&ctx, Key::Comma, NONE, &mut p, &mut s);
        assert_eq!(p.position_beats, 0.0);
        press(&ctx, Key::Comma, NONE, &mut p, &mut s);
        assert_eq!(p.position_beats, 0.0, "not before the start");

        // New tracks.
        let n = p.tracks.len();
        press(&ctx, Key::A, ctrl_alt(), &mut p, &mut s);
        assert_eq!(p.tracks.len(), n + 1);
        assert_eq!(p.tracks[n].kind, TrackKind::Audio);
    }

    #[test]
    fn over_the_piano_roll_they_wait_and_the_menus_show_them() {
        let ctx = egui::Context::default();
        let mut p = Project::demo();
        let mut s = UiState::new();
        s.piano_focus = true;
        press(&ctx, Key::C, NONE, &mut p, &mut s);
        assert!(!p.cycle, "C is the Slice tool there");
        assert_eq!(
            shortcut_of("Split at Playhead"),
            Some(egui::KeyboardShortcut::new(CTRL, Key::T))
        );
        assert_eq!(shortcut_of("Bounce"), None);
    }
}
