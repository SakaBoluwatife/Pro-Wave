//! The small panes behind control bar buttons: the Toolbar, the Note Pad and Quick Help.
use crate::i18n::tr;
use crate::model::Project;

/// Width of the Note Pad.
pub const NOTES_WIDTH: f32 = 260.0;
/// Height of the Toolbar row.
pub const TOOLBAR_HEIGHT: f32 = 30.0;

/// The longest note a project keeps, in characters.
pub const MAX_NOTES: usize = 100_000;

/// What the Toolbar offers: menu entries, so each is greyed out when the menu's is.
const TOOLBAR: &[&str] = &[
    "Undo",
    "Redo",
    "New",
    "Create Marker",
    "Capture Recording",
    "Zoom In",
    "Zoom Out",
    "Zoom to Fit",
];

/// What View > Customize Toolbar offers, in the order they show: menu entries.
pub const TOOLBAR_CHOICES: &[&str] = &[
    "Undo",
    "Redo",
    "Cut",
    "Copy",
    "Paste",
    "Split at Playhead",
    "Join",
    "Bounce In Place",
    "New",
    "New Aux Track",
    "Create Marker",
    "Capture Recording",
    "Show Automation",
    "Show Smart Controls",
    "Show Channel Rack",
    "Mastering",
    "Zoom In",
    "Zoom Out",
    "Zoom to Fit",
];

/// Puts an entry on the Toolbar or takes it off. It keeps the order of [`TOOLBAR_CHOICES`].
pub fn set_toolbar_item(items: &mut Vec<&'static str>, label: &'static str, on: bool) {
    items.retain(|x| *x != label);
    if on {
        items.push(label);
    }
    items.sort_by_key(|x| TOOLBAR_CHOICES.iter().position(|c| c == x));
}

/// The Toolbar from what was saved. An entry that is no longer offered is left out.
fn toolbar_from_text(text: &str) -> Vec<&'static str> {
    TOOLBAR_CHOICES
        .iter()
        .copied()
        .filter(|choice| text.lines().any(|line| line.trim() == *choice))
        .collect()
}

fn toolbar_file() -> Option<std::path::PathBuf> {
    Some(crate::recent_files::config_dir()?.join("toolbar.txt"))
}

/// The Toolbar as the user left it, or the usual one.
pub fn load_toolbar() -> Vec<&'static str> {
    let saved = if cfg!(test) {
        None
    } else {
        toolbar_file().and_then(|file| std::fs::read_to_string(file).ok())
    };
    match saved {
        Some(text) => toolbar_from_text(&text),
        None => TOOLBAR.to_vec(),
    }
}

/// Keeps the Toolbar for next time. A failure only means the choice lasts for this run.
pub fn save_toolbar(items: &[&str]) {
    if cfg!(test) {
        return;
    }
    if let Some(file) = toolbar_file() {
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(file, items.join("\n"));
    }
}

/// The Toolbar under the control bar (Logic: View > Show Toolbar).
pub fn toolbar(ui: &mut egui::Ui, p: &mut Project, s: &mut crate::UiState) {
    ui.horizontal_centered(|ui| {
        ui.add_space(8.0);
        for label in s.toolbar.clone() {
            let on = crate::menus::enabled(label, p, s);
            if ui.add_enabled(on, egui::Button::new(tr(label))).clicked() {
                crate::menus::apply(label, p, s);
            }
        }
    });
}

/// The Note Pad: free text saved with the project (Logic: Project Notes).
pub fn note_pad(ui: &mut egui::Ui, p: &mut Project) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.strong(tr("Notes"));
    });
    ui.add_space(4.0);
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.add_sized(
            ui.available_size(),
            egui::TextEdit::multiline(&mut p.notes).char_limit(MAX_NOTES),
        );
    });
}

/// How to do the main things, in a window of its own.
pub fn quick_help(ctx: &egui::Context, s: &mut crate::UiState) {
    egui::Window::new(tr("Quick Help"))
        .open(&mut s.show_quick_help)
        .resizable(false)
        .default_width(320.0)
        .show(ctx, |ui| {
            for line in [
                tr("Space plays and stops. Drag a file onto the window to add it."),
                tr("To record audio, pick the input in the track's Input slot, press R, then Record and Play."),
                tr("To record MIDI, press R on an instrument track, then Record and Play."),
                tr("The I button lets you hear an audio track's input. Use headphones to avoid feedback."),
                tr("With Count-in on, recording starts after one bar of clicks."),
                tr("Capture Recording turns what you just played on a MIDI keyboard into a region."),
            ] {
                ui.label(line);
                ui.add_space(4.0);
            }
        });
}

#[cfg(test)]
mod toolbar_tests {
    use super::*;

    #[test]
    fn the_usual_toolbar_only_has_what_can_be_chosen() {
        assert!(TOOLBAR.iter().all(|x| TOOLBAR_CHOICES.contains(x)));
    }

    #[test]
    fn every_choice_is_a_menu_entry() {
        let all = crate::menus::all_entries();
        for choice in TOOLBAR_CHOICES {
            assert!(all.contains(choice), "{choice}");
        }
    }

    #[test]
    fn entries_keep_their_order_and_survive_a_save() {
        let mut items = vec!["Zoom In"];
        set_toolbar_item(&mut items, "Undo", true);
        assert_eq!(items, ["Undo", "Zoom In"]);
        set_toolbar_item(&mut items, "Zoom In", false);
        assert_eq!(items, ["Undo"]);
        assert_eq!(toolbar_from_text("Undo\nNot a thing\nCut"), ["Undo", "Cut"]);
    }
}
