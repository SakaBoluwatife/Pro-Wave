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

fn ctrl_shift() -> Modifiers {
    Modifiers::COMMAND | Modifiers::SHIFT
}

fn alt_shift() -> Modifiers {
    Modifiers::ALT | Modifiers::SHIFT
}

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
    /// Calls back the screenset on this number.
    RecallScreenset(u8),
    /// Saves the panels as they are in the screenset on this number.
    SaveScreenset(u8),
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
fn alt() -> Modifiers {
    Modifiers::ALT
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
    cmd(Key::Tab, none, Action::Menu("Go to Next Transient")),
    cmd(Key::Tab, shift, Action::Menu("Go to Previous Transient")),
    cmd(Key::Period, ctrl, Action::Menu("Discard Recording")),
    // Tracks.
    cmd(Key::A, none, Action::Menu("Show Track Automation")),
    cmd(Key::M, none, Action::MuteTrack),
    cmd(Key::S, none, Action::SoloTrack),
    cmd(Key::ArrowUp, none, Action::PreviousTrack),
    cmd(Key::ArrowDown, none, Action::NextTrack),
    cmd(Key::A, ctrl_alt, Action::NewAudioTrack),
    cmd(Key::S, ctrl_alt, Action::NewInstrumentTrack),
    cmd(Key::N, ctrl, Action::Menu("New Tracks…")),
    cmd(Key::U, ctrl_alt, Action::Menu("New Aux Track")),
    cmd(Key::M, ctrl_alt, Action::Menu("Mono Sum Output")),
    cmd(Key::I, ctrl_alt, Action::Menu("Dim Output")),
    cmd(Key::X, ctrl_alt, Action::Menu("New External MIDI Track")),
    cmd(Key::D, ctrl, Action::Menu("New Track with Same Settings")),
    cmd(Key::D, ctrl_alt, Action::Menu("Duplicate Track")),
    cmd(Key::Backspace, ctrl, Action::Menu("Delete Track")),
    // Regions.
    cmd(Key::G, ctrl_alt, Action::Menu("Group Regions")),
    cmd(Key::H, ctrl_alt, Action::Menu("Ungroup Regions")),
    cmd(Key::L, none, Action::Menu("Loop Region…")),
    cmd(Key::F, ctrl, Action::Menu("Show Flex Editor")),
    cmd(Key::ArrowLeft, alt, Action::Menu("Nudge Left")),
    cmd(Key::ArrowRight, alt, Action::Menu("Nudge Right")),
    cmd(Key::X, ctrl, Action::Edit(EditCmd::Cut)),
    cmd(Key::C, ctrl, Action::Edit(EditCmd::Copy)),
    cmd(Key::V, ctrl, Action::Edit(EditCmd::Paste)),
    cmd(Key::T, ctrl, Action::Edit(EditCmd::Split)),
    cmd(Key::J, ctrl, Action::Edit(EditCmd::Join)),
    cmd(Key::R, ctrl, Action::Repeat),
    cmd(Key::A, ctrl, Action::SelectAll),
    cmd(Key::M, ctrl, Action::MuteRegions),
    cmd(Key::B, ctrl, Action::Menu("Project or Section…")),
    cmd(Key::Comma, ctrl, Action::Menu("Settings…")),
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
    cmd(Key::B, none, Action::Menu("Show Smart Controls")),
    cmd(Key::G, none, Action::Menu("Show Global Tracks")),
    cmd(Key::O, none, Action::Menu("Show Loop Browser")),
    cmd(Key::K, shift, Action::Menu("Count-in")),
    cmd(Key::C, alt, Action::Menu("Assign Track Color…")),
    cmd(Key::T, alt, Action::Menu("Configure Track Header…")),
    cmd(Key::K, alt, Action::Menu("Logic-style Key Commands…")),
    cmd(Key::Z, ctrl_alt, Action::Menu("Undo History…")),
    cmd(Key::D, ctrl_shift, Action::Menu("Create Track Stack…")),
    cmd(Key::H, ctrl_shift, Action::Menu("Create Track Stack…")),
    cmd(Key::G, ctrl_shift, Action::Menu("Create Summing Stack")),
    cmd(Key::D, alt_shift, Action::Menu("Deselect All")),
    cmd(Key::W, ctrl, Action::Menu("Close")),
    cmd(Key::W, ctrl_alt, Action::Menu("Close Project")),
    cmd(Key::Num2, ctrl, Action::Menu("Mixer")),
    cmd(Key::Num4, ctrl, Action::Menu("Piano Roll")),
    // Screensets.
    cmd(Key::Num1, none, Action::RecallScreenset(1)),
    cmd(Key::Num2, none, Action::RecallScreenset(2)),
    cmd(Key::Num3, none, Action::RecallScreenset(3)),
    cmd(Key::Num4, none, Action::RecallScreenset(4)),
    cmd(Key::Num5, none, Action::RecallScreenset(5)),
    cmd(Key::Num6, none, Action::RecallScreenset(6)),
    cmd(Key::Num7, none, Action::RecallScreenset(7)),
    cmd(Key::Num8, none, Action::RecallScreenset(8)),
    cmd(Key::Num9, none, Action::RecallScreenset(9)),
    cmd(Key::Num1, alt, Action::SaveScreenset(1)),
    cmd(Key::Num2, alt, Action::SaveScreenset(2)),
    cmd(Key::Num3, alt, Action::SaveScreenset(3)),
    cmd(Key::Num4, alt, Action::SaveScreenset(4)),
    cmd(Key::Num5, alt, Action::SaveScreenset(5)),
    cmd(Key::Num6, alt, Action::SaveScreenset(6)),
    cmd(Key::Num7, alt, Action::SaveScreenset(7)),
    cmd(Key::Num8, alt, Action::SaveScreenset(8)),
    cmd(Key::Num9, alt, Action::SaveScreenset(9)),
];

/// The user's own keys: a command's name and what it is on now, kept in the config folder. A
/// command not listed has its key from [`COMMANDS`].
#[derive(Default, Clone, Debug, PartialEq)]
pub struct KeyMap {
    own: Vec<(String, Key, Modifiers)>,
    loaded: bool,
    /// The Settings window waits for a key for this command.
    pub capturing: Option<usize>,
}

const KEYS_FILE: &str = "key-commands";

/// What a command is called, in the list and in the file.
pub fn name_of(action: Action) -> String {
    label_of(action)
        .map(str::to_string)
        .unwrap_or_else(|| format!("{action:?}"))
}

fn mods_text(m: Modifiers) -> String {
    format!(
        "{}{}{}",
        if m.command || m.ctrl { "c" } else { "" },
        if m.alt { "a" } else { "" },
        if m.shift { "s" } else { "" }
    )
}

fn mods_from(text: &str) -> Modifiers {
    let mut m = Modifiers::NONE;
    if text.contains('c') {
        m |= Modifiers::COMMAND;
    }
    if text.contains('a') {
        m |= Modifiers::ALT;
    }
    if text.contains('s') {
        m |= Modifiers::SHIFT;
    }
    m
}

/// How a key reads: Ctrl+Shift+T.
pub fn key_text(key: Key, m: Modifiers) -> String {
    let mut out = String::new();
    if m.command || m.ctrl {
        out.push_str("Ctrl+");
    }
    if m.alt {
        out.push_str("Alt+");
    }
    if m.shift {
        out.push_str("Shift+");
    }
    out.push_str(key.name());
    out
}

impl KeyMap {
    fn to_text(&self) -> String {
        self.own
            .iter()
            .map(|(name, key, m)| format!("{name}\t{}\t{}\n", key.name(), mods_text(*m)))
            .collect()
    }

    fn from_text(text: &str) -> Vec<(String, Key, Modifiers)> {
        text.lines()
            .filter_map(|line| {
                let mut parts = line.split('\t');
                let (name, key) = (parts.next()?, Key::from_name(parts.next()?)?);
                Some((name.to_string(), key, mods_from(parts.next().unwrap_or(""))))
            })
            .collect()
    }

    fn make_ready(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        if cfg!(test) {
            return;
        }
        let text = crate::recent_files::config_dir()
            .and_then(|d| std::fs::read_to_string(d.join(KEYS_FILE)).ok());
        if let Some(text) = text {
            self.own = Self::from_text(&text);
        }
    }

    fn save(&self) {
        if cfg!(test) {
            return;
        }
        if let Some(dir) = crate::recent_files::config_dir() {
            let saved = std::fs::create_dir_all(&dir)
                .and_then(|()| std::fs::write(dir.join(KEYS_FILE), self.to_text()));
            if let Err(e) = saved {
                tracing::warn!("cannot save the key commands: {e}");
            }
        }
    }

    /// The key a command is on now.
    pub fn key_of(&self, command: &Command) -> (Key, Modifiers) {
        let name = name_of(command.action);
        self.own
            .iter()
            .find(|(n, _, _)| *n == name)
            .map_or((command.key, (command.modifiers)()), |(_, k, m)| (*k, *m))
    }

    /// The command already on this key, other than `except`.
    pub fn taken_by(&self, key: Key, m: Modifiers, except: usize) -> Option<&'static Command> {
        COMMANDS
            .iter()
            .enumerate()
            .find(|(i, c)| *i != except && self.key_of(c) == (key, m))
            .map(|(_, c)| c)
    }

    /// Puts command number `index` on this key. Saved at once.
    pub fn set(&mut self, index: usize, key: Key, m: Modifiers) {
        let Some(command) = COMMANDS.get(index) else {
            return;
        };
        let name = name_of(command.action);
        self.own.retain(|(n, _, _)| *n != name);
        self.own.push((name, key, m));
        self.save();
    }

    /// Back to the keys Pro Wave comes with.
    pub fn reset(&mut self) {
        self.own.clear();
        self.save();
    }
}

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
    if ctx.egui_wants_keyboard_input() || s.bounce.busy() {
        return;
    }
    s.keys.make_ready();
    if s.keys.capturing.is_some() {
        return;
    }
    for command in COMMANDS {
        let screenset = matches!(
            command.action,
            Action::RecallScreenset(_) | Action::SaveScreenset(_)
        );
        // The piano roll's keys win over it, but it has no use for the numbers, and recording
        // has to start from there too.
        let recording = matches!(
            command.action,
            Action::Menu("Record" | "Capture Recording" | "Discard Recording")
        );
        if s.piano_focus && !screenset && !recording {
            continue;
        }
        // While the computer keyboard plays notes, plain numbers are its own.
        if s.typing.on && matches!(command.action, Action::RecallScreenset(_)) {
            continue;
        }
        let (key, mods) = s.keys.key_of(command);
        let pressed =
            ctx.input_mut(|i| i.modifiers.matches_exact(mods) && i.consume_key(mods, key));
        if pressed {
            perform(command.action, p, s);
        }
    }
}

/// Turns Cycle on or off, over what is selected in the Tracks area.
pub fn toggle_cycle(p: &mut Project, view: &crate::tracks_area::TracksView) {
    let selection = crate::bounce::selection_range(p, &view.all_selected());
    p.toggle_cycle(selection);
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
        Action::Cycle => toggle_cycle(p, &s.tracks),
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
        Action::RecallScreenset(slot) => crate::screensets::go_to(p, s, slot),
        Action::SaveScreenset(slot) => crate::screensets::save(p, s, slot),
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
    fn your_own_keys_replace_the_table_and_are_kept_as_text() {
        let ctx = egui::Context::default();
        let mut p = Project::demo();
        let mut s = UiState::new();
        let cycle = COMMANDS
            .iter()
            .position(|c| c.action == Action::Cycle)
            .unwrap();
        s.keys.set(cycle, Key::F9, Modifiers::SHIFT);
        press(&ctx, Key::C, NONE, &mut p, &mut s);
        assert!(!p.cycle, "it left its old key");
        press(&ctx, Key::F9, Modifiers::SHIFT, &mut p, &mut s);
        assert!(p.cycle, "and is on the new one");

        assert_eq!(KeyMap::from_text(&s.keys.to_text()), s.keys.own);
        assert_eq!(key_text(Key::F9, Modifiers::SHIFT), "Shift+F9");
        // Someone else's key is named before it is taken.
        let metronome = COMMANDS
            .iter()
            .position(|c| c.action == Action::Metronome)
            .unwrap();
        assert!(s
            .keys
            .taken_by(Key::F9, Modifiers::SHIFT, metronome)
            .is_some());
        assert!(s.keys.taken_by(Key::F9, Modifiers::SHIFT, cycle).is_none());
        s.keys.reset();
        assert_eq!(s.keys.key_of(&COMMANDS[cycle]).0, Key::C);
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
