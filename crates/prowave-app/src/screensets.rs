//! Screensets (Logic: Screensets): panel layouts kept with the project and called back on the
//! number keys. 1 to 9 recall a layout and Alt+1 to Alt+9 save one (nothing else uses those keys,
//! and `key_commands` keeps them in its table, where they can be moved and are checked for
//! clashes). While the computer keyboard plays notes, plain numbers belong to it.
//!
//! A layout is which panels are showing and how far the Tracks area is zoomed. The height of the
//! bottom panel and the width of the side panels are not part of it.
use crate::file_browser::Tab;
use crate::left_panel::LeftTab;
use crate::model::Project;
use crate::{Bottom, UiState};
use prowave_project::ScreensetDoc;

/// The highest slot, as far as there are number keys for.
pub const SLOTS: u8 = 9;

fn tab_name(tab: Option<Tab>) -> &'static str {
    match tab {
        None => "",
        Some(Tab::Loops) => "loops",
        Some(Tab::Files) => "files",
    }
}

fn tab_from(name: &str) -> Option<Tab> {
    match name {
        "loops" => Some(Tab::Loops),
        "files" => Some(Tab::Files),
        _ => None,
    }
}

/// The layout on the screen now.
pub fn capture(s: &UiState, slot: u8, name: String) -> ScreensetDoc {
    ScreensetDoc {
        slot,
        name,
        show_mixer: s.show_mixer,
        show_piano: s.bottom == Bottom::PianoRoll,
        score_tab: s.score_tab,
        show_library: s.show_library,
        left_tab: match s.left_tab {
            LeftTab::Library => 0,
            LeftTab::Browser => 1,
            LeftTab::Plugins => 2,
        },
        show_inspector: s.show_inspector,
        show_lists: s.show_lists,
        show_quick_help: s.show_quick_help,
        show_toolbar: s.show_toolbar,
        show_notes: s.show_notes,
        browser: tab_name(s.browser.open).to_string(),
        show_rack: s.rack.open,
        show_smart: s.smart.open,
        show_mastering: s.mastering.open,
        show_tempo: s.tracks.show_tempo,
        show_markers: s.tracks.show_markers,
        show_signatures: s.tracks.show_signatures,
        bar_w: s.tracks.bar_w,
    }
}

/// Puts a layout on the screen. Nothing in the project changes.
pub fn recall(s: &mut UiState, set: &ScreensetDoc) {
    s.show_mixer = set.show_mixer;
    s.bottom = if set.show_piano {
        Bottom::PianoRoll
    } else {
        Bottom::Mixer
    };
    s.score_tab = set.score_tab;
    s.show_library = set.show_library;
    s.left_tab = match set.left_tab {
        1 => LeftTab::Browser,
        2 => LeftTab::Plugins,
        _ => LeftTab::Library,
    };
    s.show_inspector = set.show_inspector;
    s.show_lists = set.show_lists;
    s.show_quick_help = set.show_quick_help;
    s.show_toolbar = set.show_toolbar;
    s.show_notes = set.show_notes;
    s.browser.open = tab_from(&set.browser);
    s.rack.open = set.show_rack;
    s.smart.open = set.show_smart;
    s.mastering.open = set.show_mastering;
    s.tracks.show_tempo = set.show_tempo;
    s.tracks.show_markers = set.show_markers;
    s.tracks.show_signatures = set.show_signatures;
    if set.bar_w.is_finite() && set.bar_w > 0.0 {
        s.tracks.bar_w = set.bar_w.clamp(6.0, 600.0);
    }
}

/// The screensets a file holds, kept only where a number key can reach them, one in each slot.
pub fn from_doc(sets: &[ScreensetDoc]) -> Vec<ScreensetDoc> {
    let mut out: Vec<ScreensetDoc> = Vec::new();
    for set in sets.iter().filter(|x| (1..=SLOTS).contains(&x.slot)) {
        if !out.iter().any(|x| x.slot == set.slot) {
            let mut set = set.clone();
            set.name = set.name.chars().take(64).collect();
            out.push(set);
        }
    }
    out.sort_by_key(|x| x.slot);
    out
}

/// Saves the layout on the screen in `slot`, keeping the name it has there.
pub fn save(p: &mut Project, s: &mut UiState, slot: u8) {
    if !(1..=SLOTS).contains(&slot) {
        return;
    }
    let at = p.screensets.iter().position(|x| x.slot == slot);
    let name = at.map_or_else(String::new, |i| p.screensets[i].name.clone());
    let set = capture(s, slot, name);
    match at {
        Some(i) => p.screensets[i] = set,
        None => {
            p.screensets.push(set);
            p.screensets.sort_by_key(|x| x.slot);
        }
    }
    s.screenset_on = Some(slot);
    s.status = Some(format!("Screenset {slot} saved"));
}

/// Calls back the layout in `slot`, or says there is none.
pub fn go_to(p: &Project, s: &mut UiState, slot: u8) {
    match p.screensets.iter().find(|x| x.slot == slot) {
        Some(set) => {
            recall(s, set);
            s.screenset_on = Some(slot);
        }
        None => s.status = Some(format!("Screenset {slot} is empty: Alt+{slot} saves it")),
    }
}

/// Window > Screensets: the slots, with their names, and buttons for what the keys do.
pub fn window(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    if !s.dialogs.screensets {
        return;
    }
    let mut open = true;
    let (mut save_slot, mut recall_slot, mut clear_slot) = (None, None, None);
    let on = s.screenset_on;
    egui::Window::new("Screensets")
        .id(egui::Id::unique("screensets"))
        .open(&mut open)
        .resizable(false)
        .show(ctx, |ui| {
            ui.label("1 to 9 call a layout back. Alt+1 to Alt+9 save the one on the screen.");
            ui.small("It keeps which panels show and the zoom of the tracks, with the project.");
            egui::Grid::new(egui::Id::unique("screensets_grid")).show(ui, |ui| {
                for slot in 1..=SLOTS {
                    let at = p.screensets.iter().position(|x| x.slot == slot);
                    let number = format!("{slot}");
                    if on == Some(slot) {
                        ui.strong(number);
                    } else {
                        ui.label(number);
                    }
                    match at {
                        Some(i) => {
                            ui.add(
                                egui::TextEdit::singleline(&mut p.screensets[i].name)
                                    .hint_text("Name")
                                    .desired_width(140.0),
                            );
                        }
                        None => {
                            ui.weak("Empty");
                        }
                    }
                    if ui
                        .add_enabled(at.is_some(), egui::Button::new("Recall"))
                        .clicked()
                    {
                        recall_slot = Some(slot);
                    }
                    if ui.button("Save").clicked() {
                        save_slot = Some(slot);
                    }
                    if ui
                        .add_enabled(at.is_some(), egui::Button::new("Clear"))
                        .clicked()
                    {
                        clear_slot = Some(slot);
                    }
                    ui.end_row();
                }
            });
        });
    if let Some(slot) = save_slot {
        save(p, s, slot);
    }
    if let Some(slot) = recall_slot {
        go_to(p, s, slot);
    }
    if let Some(slot) = clear_slot {
        p.screensets.retain(|x| x.slot != slot);
        if s.screenset_on == Some(slot) {
            s.screenset_on = None;
        }
    }
    s.dialogs.screensets = open;
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::key_commands;
    use egui::{Event, Key, Modifiers, RawInput};

    fn press(ctx: &egui::Context, key: Key, mods: Modifiers, p: &mut Project, s: &mut UiState) {
        let event = Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: mods,
        };
        let events = vec![Event::ModifiersChanged(mods), event];
        let input = RawInput {
            events,
            ..RawInput::default()
        };
        let mut out = ctx.run_ui(input, |ui| key_commands::run(ui.ctx(), p, s));
        out.textures_delta.clear();
        let up = Event::Key {
            key,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: mods,
        };
        let input = RawInput {
            events: vec![up, Event::ModifiersChanged(Modifiers::NONE)],
            ..RawInput::default()
        };
        let mut out = ctx.run_ui(input, |_| {});
        out.textures_delta.clear();
    }

    #[test]
    fn a_layout_comes_back_as_it_was_and_changes_nothing_in_the_song() {
        let (mut p, mut s) = (Project::demo(), UiState::new());
        s.show_library = true;
        s.left_tab = LeftTab::Plugins;
        s.bottom = Bottom::PianoRoll;
        s.browser.open = Some(Tab::Loops);
        s.tracks.bar_w = 120.0;
        s.rack.open = true;
        save(&mut p, &mut s, 3);
        assert_eq!(p.screensets.len(), 1);

        s.show_library = false;
        s.left_tab = LeftTab::Library;
        s.bottom = Bottom::Mixer;
        s.browser.open = None;
        s.tracks.bar_w = 40.0;
        s.rack.open = false;
        let song = crate::file_commands::content(&p);
        go_to(&p, &mut s, 3);
        assert!(s.show_library && s.left_tab == LeftTab::Plugins);
        assert!(s.bottom == Bottom::PianoRoll && s.rack.open);
        assert_eq!(s.browser.open, Some(Tab::Loops));
        assert_eq!(s.tracks.bar_w, 120.0);
        assert_eq!(crate::file_commands::content(&p), song);
    }

    #[test]
    fn an_empty_slot_says_so_and_changes_no_panel() {
        let (mut p, mut s) = (Project::demo(), UiState::new());
        save(&mut p, &mut s, 1);
        s.show_library = true;
        go_to(&p, &mut s, 2);
        assert!(s.show_library);
        assert!(s.status.as_deref().unwrap().contains("empty"));
    }

    #[test]
    fn the_number_keys_save_and_recall_and_clash_with_nothing() {
        let ctx = egui::Context::default();
        let (mut p, mut s) = (Project::demo(), UiState::new());
        s.show_library = true;
        press(&ctx, Key::Num4, Modifiers::ALT, &mut p, &mut s);
        assert_eq!(p.screensets[0].slot, 4);
        s.show_library = false;
        press(&ctx, Key::Num4, Modifiers::NONE, &mut p, &mut s);
        assert!(s.show_library, "called back");
        // Over the piano roll, which has no use for them, they work the same.
        s.show_library = false;
        s.piano_focus = true;
        press(&ctx, Key::Num4, Modifiers::NONE, &mut p, &mut s);
        assert!(s.show_library);
        // While the keyboard plays notes, plain numbers are its own.
        s.show_library = false;
        s.typing.on = true;
        press(&ctx, Key::Num1, Modifiers::NONE, &mut p, &mut s);
        press(&ctx, Key::Num4, Modifiers::NONE, &mut p, &mut s);
        assert!(!s.show_library);
    }

    #[test]
    fn they_are_saved_with_the_project_and_a_bad_file_is_repaired() {
        let (mut p, mut s) = (Project::demo(), UiState::new());
        save(&mut p, &mut s, 2);
        p.screensets[0].name = "Mix".into();
        s.show_library = !s.show_library;
        save(&mut p, &mut s, 2);
        assert_eq!(p.screensets[0].name, "Mix", "saving again keeps the name");
        let mut doc = crate::project_io::to_doc(&p).0;
        let (back, _) =
            crate::project_io::from_doc(&doc, std::path::Path::new("."), &egui::Context::default());
        assert_eq!(back.screensets, p.screensets);

        doc.screensets[0].slot = 200;
        let second = ScreensetDoc {
            slot: 5,
            ..ScreensetDoc::default()
        };
        doc.screensets.push(second.clone());
        doc.screensets.push(second);
        let (back, _) =
            crate::project_io::from_doc(&doc, std::path::Path::new("."), &egui::Context::default());
        assert_eq!(
            back.screensets.len(),
            1,
            "no key for slot 200, and one in 5"
        );

        // A zoom from a hand-edited file stays in range.
        let set = ScreensetDoc {
            bar_w: f32::NAN,
            ..ScreensetDoc::default()
        };
        let before = s.tracks.bar_w;
        recall(&mut s, &set);
        assert_eq!(s.tracks.bar_w, before);
    }
}
