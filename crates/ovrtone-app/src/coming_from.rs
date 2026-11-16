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
        if let Err(e) = saved {
            tracing::warn!("could not save the choice of program: {e}");
        }
    }
}

/// What was chosen before, if anything.
pub fn chosen() -> Option<String> {
    file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
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
