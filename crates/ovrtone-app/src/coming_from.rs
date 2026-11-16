//! "I'm coming from": on the first launch, and from Settings, the user names the program they know.
//! It sets the key commands and the starting layout to that program's habits (not its look, which
//! stays Ovrtone's). Kept in `coming-from` in the config folder.

use crate::key_commands::KeyPreset;
use crate::left_panel::LeftTab;
use crate::{Bottom, UiState};

const FILE: &str = "coming-from";

/// The programs offered, and the set of keys each brings.
const FROM: [(&str, KeyPreset); 5] = [
    ("Logic Pro", KeyPreset::Logic),
    ("FL Studio", KeyPreset::FlStudio),
    ("Ableton Live", KeyPreset::Ableton),
    ("Pro Tools", KeyPreset::ProTools),
    ("None of these", KeyPreset::Ovrtone),
];

/// The window, while it is open.
#[derive(Default)]
pub struct ComingFrom {
    pub open: bool,
    /// The launch checked for an earlier choice.
    checked: bool,
}

fn file() -> Option<std::path::PathBuf> {
    crate::recent_files::config_dir().map(|d| d.join(FILE))
}

/// The window comes up on the first launch, when nothing was chosen yet.
pub fn on_launch(s: &mut UiState) {
    if s.coming_from.checked || cfg!(test) {
        return;
    }
    s.coming_from.checked = true;
    if file().is_some_and(|f| !f.exists()) {
        s.coming_from.open = true;
    }
}

/// The layout a program's users expect to start with.
fn apply(s: &mut UiState, name: &str, keys: KeyPreset) {
    s.keys.choose_preset(keys);
    // Middle C as that program names it: C5 in FL Studio, C3 in Logic, Ableton and Pro Tools.
    crate::settings::set_octave_naming(if name == "FL Studio" { 0 } else { -2 });
    match name {
        // Channel Rack beside the Playlist, the Browser on the left, the mixer away.
        "FL Studio" => {
            s.rack.open = true;
            s.show_library = true;
            s.left_tab = LeftTab::Browser;
            s.show_inspector = false;
            s.show_mixer = false;
        }
        // The Browser on the left, the editor of what is selected at the bottom.
        "Ableton Live" => {
            s.rack.open = false;
            s.show_library = true;
            s.left_tab = LeftTab::Browser;
            s.show_inspector = false;
            s.show_mixer = true;
            s.bottom = Bottom::PianoRoll;
        }
        // The Edit window with the mix underneath.
        "Pro Tools" => {
            s.rack.open = false;
            s.show_library = false;
            s.show_inspector = false;
            s.show_mixer = true;
            s.bottom = Bottom::Mixer;
        }
        // Logic's: Library and Inspector at the left.
        _ => {
            s.rack.open = false;
            s.show_library = true;
            s.show_inspector = true;
        }
    }
    if let Some(f) = file() {
        let saved = f
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&f, name));
        if let Ok(mut cached) = CHOSEN.lock() {
            *cached = Some(Some(name.to_string()).filter(|n| *n != "None of these"));
        }
        if let Err(e) = saved {
            tracing::warn!("could not save the choice of program: {e}");
        }
    }
}

/// What a few of Ovrtone's things are called in the program the user came from, for the tooltips:
/// (the start of Ovrtone's tooltip, that program, what it calls it).
const WORDS: &[(&str, &str, &str)] = &[
    ("Cycle", "FL Studio", "the Playlist's loop (time selection)"),
    ("Cycle", "Ableton Live", "the Loop switch and loop brace"),
    ("Cycle", "Pro Tools", "Loop Playback"),
    ("Count-in", "FL Studio", "the countdown before recording"),
    ("Count-in", "Pro Tools", "Countoff"),
    ("Add an effect", "FL Studio", "a Mixer insert slot"),
    ("Add an effect", "Ableton Live", "the Device chain"),
    ("Add an effect", "Pro Tools", "an Insert"),
    ("Add a send", "FL Studio", "routing to a Mixer track"),
    ("Add a send", "Ableton Live", "a send to a Return track"),
    ("Add a send", "Pro Tools", "a Send to an Aux Input"),
    ("Media Browser", "FL Studio", "the Browser"),
    ("Media Browser", "Ableton Live", "the Browser"),
    ("Media Browser", "Pro Tools", "the Workspace"),
];

/// `text` as Ovrtone says it, and, when the user came from another program, what that program
/// calls it ("Cycle (C)" and "Loop Playback in Pro Tools").
pub fn in_their_words(text: &str) -> String {
    let Some(from) = chosen() else {
        return text.to_string();
    };
    WORDS
        .iter()
        .find(|(ours, program, _)| text.starts_with(ours) && *program == from)
        .map_or_else(
            || text.to_string(),
            |(_, program, theirs)| format!("{text}\n{theirs} in {program}"),
        )
}

/// What was chosen before, if anything.
/// Read once, then kept: tooltips ask every frame.
static CHOSEN: std::sync::Mutex<Option<Option<String>>> = std::sync::Mutex::new(None);

pub fn chosen() -> Option<String> {
    let Ok(mut cached) = CHOSEN.lock() else {
        return None;
    };
    cached
        .get_or_insert_with(|| {
            file()
                .and_then(|f| std::fs::read_to_string(f).ok())
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
        })
        .clone()
}

pub fn window(ctx: &egui::Context, s: &mut UiState) {
    if !s.coming_from.open {
        return;
    }
    let mut picked = None;
    let mut close = false;
    egui::Window::new("Welcome to Ovrtone")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.label("Which program are you coming from?");
            ui.weak("Ovrtone takes its key commands and starting layout. Change it any time in Settings > General.");
            ui.add_space(6.0);
            for (name, keys) in FROM {
                if ui.add_sized([220.0, 26.0], egui::Button::new(name)).clicked() {
                    picked = Some((name, keys));
                }
            }
            ui.add_space(4.0);
            close = ui.small_button("Not now").clicked();
        });
    if let Some((name, keys)) = picked {
        apply(s, name, keys);
        s.coming_from.open = false;
        s.status = Some(format!("Set up for {name}: its key commands and layout"));
    } else if close {
        s.coming_from.open = false;
        // Asked once: "Not now" counts as an answer, so the window does not come back.
        if let Some(f) = file() {
            let _ = f.parent().map_or(Ok(()), std::fs::create_dir_all);
            let _ = std::fs::write(f, "");
        }
    }
}
