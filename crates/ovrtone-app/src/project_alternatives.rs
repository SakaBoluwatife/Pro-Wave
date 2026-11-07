//! Project Alternatives (Logic: File > Alternatives): named versions of one project, kept in its
//! file. New keeps the project as it stands under its name and carries on under a new one;
//! switching keeps the one on show and brings the other back, with its plugins' settings.
//!
//! The one on show lives in `Project` as usual. The others are held as what a save would write, so
//! bringing one back is opening it, and it takes the same path as opening a project: plugins are
//! loaded again from their settings, and Undo starts afresh, as it does for an opened project.
use crate::file_commands::FileTask;
use crate::model::{AltSnapshot, Alternative, Project};
use crate::{project_io, App, UiState};
use std::sync::Arc;

/// Most versions one project keeps (`ovrtone_project` reads no more than this).
pub const MAX: usize = ovrtone_project::MAX_ALTERNATIVES;

/// A name not taken, for a new one: Alternative 2, Alternative 3...
pub fn next_name(p: &Project) -> String {
    let start = p.alternatives.len().max(1) + 1;
    (start..)
        .map(|n| format!("Alternative {n}"))
        .find(|name| !p.alternatives.iter().any(|a| a.name == *name))
        .unwrap_or_default()
}

/// Whether a version can have this name: something, and not one another version has. `own` is the
/// one being renamed.
pub fn name_ok(p: &Project, name: &str, own: Option<usize>) -> bool {
    let name = name.trim();
    !name.is_empty()
        && !p
            .alternatives
            .iter()
            .enumerate()
            .any(|(i, a)| Some(i) != own && a.name == name)
}

/// Keeps the project as it stands under the name of the version on show.
fn keep_current(p: &mut Project) {
    if p.alternatives.is_empty() {
        p.alternatives.push(Alternative {
            name: "Alternative 1".to_string(),
            saved: None,
        });
        p.active_alternative = 0;
    }
    let snapshot = Arc::new(project_io::snapshot(p));
    if let Some(a) = p.alternatives.get_mut(p.active_alternative) {
        a.saved = Some(snapshot);
    }
}

/// Starts a new version from the project as it stands, which stays on show under the new name.
/// The running plugins' settings must have been collected first.
pub fn create(p: &mut Project, name: &str) -> bool {
    let name = name.trim();
    if !name_ok(p, name, None) || p.alternatives.len() >= MAX {
        return false;
    }
    keep_current(p);
    p.alternatives.push(Alternative {
        name: name.to_string(),
        saved: None,
    });
    p.active_alternative = p.alternatives.len() - 1;
    true
}

/// Renames a version.
pub fn rename(p: &mut Project, i: usize, name: &str) -> bool {
    let name = name.trim();
    if !name_ok(p, name, Some(i)) {
        return false;
    }
    match p.alternatives.get_mut(i) {
        Some(a) => {
            a.name = name.to_string();
            true
        }
        None => false,
    }
}

/// Takes out a version that is not on show. The last one left then goes as well, as a project with
/// one version has no versions to name.
pub fn remove_other(p: &mut Project, i: usize) -> bool {
    if i >= p.alternatives.len() || i == p.active_alternative {
        return false;
    }
    p.alternatives.remove(i);
    if i < p.active_alternative {
        p.active_alternative -= 1;
    }
    true
}

/// The project a kept version opens as, under the file's name, with the list in it. The version
/// on show is kept first, so nothing is lost. `None` when `i` cannot be brought back.
pub fn bring_back(p: &mut Project, i: usize, ctx: &egui::Context) -> Option<(Project, Vec<String>)> {
    if i == p.active_alternative {
        return None;
    }
    let snapshot: Arc<AltSnapshot> = p.alternatives.get(i)?.saved.clone()?;
    keep_current(p);
    let (mut next, warnings) = project_io::open_snapshot(&snapshot, ctx);
    next.name = p.name.clone();
    next.alternatives = p.alternatives.clone();
    next.alternatives[i].saved = None;
    next.active_alternative = i;
    next.song_rev = p.song_rev.wrapping_add(1);
    Some((next, warnings))
}

impl App {
    /// Puts a kept version on show.
    fn switch_alternative(&mut self, i: usize, ctx: &egui::Context) {
        if self.ui.rec.active() || self.ui.audio_rec.active() {
            self.ui.status = Some("Stop recording before switching alternatives".to_string());
            return;
        }
        if self.ui.bounce.busy() {
            self.ui.status = Some("Wait for the bounce before switching alternatives".to_string());
            return;
        }
        if let Some(sync) = self.sync.as_mut() {
            sync.collect_plugin_states(&mut self.project);
        }
        let Some((next, warnings)) = bring_back(&mut self.project, i, ctx) else {
            return;
        };
        let name = next.alternatives[i].name.clone();
        self.project = next;
        self.ui.reset_views();
        // Undo steps belong to the version they were made in.
        self.history.reset(&self.project);
        self.ui.status = Some(if warnings.is_empty() {
            format!("Alternative: {name}")
        } else {
            format!("Alternative: {name}. Missing media: {}", warnings.join(", "))
        });
        ctx.request_repaint();
    }

    /// Deletes a version, moving to its neighbour first when it is the one on show.
    fn delete_alternative(&mut self, i: usize, ctx: &egui::Context) {
        if i == self.project.active_alternative {
            let neighbour = if i + 1 < self.project.alternatives.len() {
                i + 1
            } else {
                i.wrapping_sub(1)
            };
            self.switch_alternative(neighbour, ctx);
            if self.project.active_alternative == i {
                return;
            }
        }
        remove_other(&mut self.project, i);
        // One version left has nothing to be told apart from.
        if self.project.alternatives.len() == 1 {
            self.project.alternatives.clear();
            self.project.active_alternative = 0;
        }
    }

    /// Carries out what the Alternatives windows asked for.
    pub fn run_alternative_task(&mut self, task: FileTask, ctx: &egui::Context) {
        match task {
            FileTask::NewAlternative(name) => {
                if let Some(sync) = self.sync.as_mut() {
                    sync.collect_plugin_states(&mut self.project);
                }
                if !create(&mut self.project, &name) {
                    self.ui.status = Some(format!("cannot make the alternative \"{name}\""));
                }
            }
            FileTask::SwitchAlternative(i) => self.switch_alternative(i, ctx),
            FileTask::DeleteAlternative(i) => self.delete_alternative(i, ctx),
            _ => {}
        }
    }
}

/// What the small windows of this feature are waiting for.
#[derive(Default)]
pub struct AltDialogs {
    /// File > Project Alternatives: the list.
    pub list: bool,
    /// The name being typed, and the version it renames, or none for a new one.
    pub naming: Option<(Option<usize>, String)>,
    /// The version waiting for a yes before it is deleted.
    pub deleting: Option<usize>,
}

/// The list, with what can be done to it.
fn list_window(ctx: &egui::Context, p: &Project, s: &mut UiState) {
    if !s.alt_dialogs.list {
        return;
    }
    let mut open = true;
    let (mut pick, mut rename, mut delete, mut new) = (None, None, None, false);
    egui::Window::new("Project Alternatives")
        .id(egui::Id::unique("project_alternatives"))
        .open(&mut open)
        .resizable(false)
        .show(ctx, |ui| {
            if p.alternatives.is_empty() {
                ui.label("This project has one version. New Alternative keeps it as it stands.");
            }
            egui::Grid::new(egui::Id::unique("alternatives_grid")).show(ui, |ui| {
                for (i, a) in p.alternatives.iter().enumerate() {
                    let on = i == p.active_alternative;
                    if ui.selectable_label(on, a.name.as_str()).clicked() && !on {
                        pick = Some(i);
                    }
                    if ui.small_button("Rename").clicked() {
                        rename = Some(i);
                    }
                    if ui
                        .add_enabled(p.alternatives.len() > 1, egui::Button::new("Delete").small())
                        .clicked()
                    {
                        delete = Some(i);
                    }
                    ui.end_row();
                }
            });
            ui.separator();
            new = ui
                .add_enabled(p.alternatives.len() < MAX, egui::Button::new("New Alternative"))
                .clicked();
            ui.small("All of them are saved in the project file. Click a name to switch to it.");
        });
    if let Some(i) = pick {
        s.file_task = Some(FileTask::SwitchAlternative(i));
    }
    if let Some(i) = rename {
        s.alt_dialogs.naming = Some((Some(i), p.alternatives[i].name.clone()));
    }
    s.alt_dialogs.deleting = delete.or(s.alt_dialogs.deleting);
    if new {
        s.alt_dialogs.naming = Some((None, next_name(p)));
    }
    s.alt_dialogs.list = open;
}

/// Asks for a name.
fn naming_window(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    let Some((target, mut text)) = s.alt_dialogs.naming.take() else {
        return;
    };
    let (mut ok, mut cancel) = (false, false);
    let acceptable = name_ok(p, &text, target);
    egui::Modal::new(egui::Id::unique("alternative_name")).show(ctx, |ui| {
        ui.set_width(300.0);
        ui.label(if target.is_some() {
            "Rename this alternative:"
        } else {
            "Name of the new alternative:"
        });
        let edit = ui.text_edit_singleline(&mut text);
        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if !acceptable {
            ui.small("It needs a name no other alternative has.");
        }
        cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
        ui.horizontal(|ui| {
            ok = ui.add_enabled(acceptable, egui::Button::new("OK")).clicked()
                || (enter && acceptable);
            cancel |= ui.button("Cancel").clicked();
        });
    });
    if ok {
        match target {
            Some(i) => {
                rename(p, i, &text);
            }
            None => s.file_task = Some(FileTask::NewAlternative(text)),
        }
    } else if !cancel {
        s.alt_dialogs.naming = Some((target, text));
    }
}

/// Asks before deleting a version, which Undo cannot bring back.
fn deleting_window(ctx: &egui::Context, p: &Project, s: &mut UiState) {
    let Some(i) = s.alt_dialogs.deleting else {
        return;
    };
    let Some(a) = p.alternatives.get(i) else {
        s.alt_dialogs.deleting = None;
        return;
    };
    let (mut yes, mut no) = (false, false);
    egui::Modal::new(egui::Id::unique("alternative_delete")).show(ctx, |ui| {
        ui.set_width(340.0);
        ui.label(format!("Delete the alternative \"{}\"?", a.name));
        ui.label("Everything made in it goes, and Undo cannot bring it back.");
        ui.horizontal(|ui| {
            no = ui.button("Cancel").clicked();
            yes = ui.button("Delete").clicked();
        });
    });
    if yes {
        s.file_task = Some(FileTask::DeleteAlternative(i));
    }
    if yes || no {
        s.alt_dialogs.deleting = None;
    }
}

/// The windows.
pub fn show(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    list_window(ctx, p, s);
    naming_window(ctx, p, s);
    deleting_window(ctx, p, s);
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::model::{Note, TrackKind};

    fn note_count(p: &Project) -> usize {
        p.tracks.iter().flat_map(|t| &t.regions).map(|r| r.notes.len()).sum()
    }

    /// A project with one MIDI region of `n` notes.
    fn with_notes(p: &mut Project, n: usize) {
        p.tracks.truncate(1);
        p.tracks[0].kind = TrackKind::Instrument;
        p.tracks[0].regions.clear();
        p.add_midi_region(0, 1.0);
        p.tracks[0].regions[0].notes = vec![
            Note {
                start: 0.0,
                len: 1.0,
                pitch: 60,
                vel: 90,
                muted: false,
            };
            n
        ];
    }

    #[test]
    fn switching_keeps_what_was_on_show_and_brings_the_other_back() {
        let ctx = egui::Context::default();
        let mut p = Project::empty();
        with_notes(&mut p, 2);
        assert!(create(&mut p, "Loud"));
        assert_eq!(p.alternatives[0].name, "Alternative 1");
        assert_eq!((p.active_alternative, note_count(&p)), (1, 2));

        // The new one changes; going back finds the first as it was.
        with_notes(&mut p, 5);
        let (mut back, warnings) = bring_back(&mut p, 0, &ctx).unwrap();
        assert!(warnings.is_empty());
        assert_eq!((back.active_alternative, note_count(&back)), (0, 2));
        assert_eq!(back.name, p.name);

        // And the other way, nothing having been lost.
        let (forth, _) = bring_back(&mut back, 1, &ctx).unwrap();
        assert_eq!((forth.active_alternative, note_count(&forth)), (1, 5));
        assert!(bring_back(&mut back, 0, &ctx).is_none(), "it is on show");
        assert!(bring_back(&mut back, 9, &ctx).is_none());
    }

    #[test]
    fn names_must_be_there_and_differ() {
        let mut p = Project::empty();
        assert!(create(&mut p, "B"));
        assert!(!create(&mut p, "B") && !create(&mut p, "  "));
        assert_eq!(p.alternatives.len(), 2);
        assert_eq!(next_name(&p), "Alternative 3");
        assert!(rename(&mut p, 1, "C"));
        assert!(rename(&mut p, 1, "C"), "its own name is fine");
        assert!(!rename(&mut p, 1, "Alternative 1"));
        assert!(!rename(&mut p, 7, "D"));
        assert_eq!(p.alternatives[1].name, "C");
    }

    #[test]
    fn taking_one_out_keeps_the_one_on_show_right() {
        let mut p = Project::empty();
        assert!(create(&mut p, "B") && create(&mut p, "C"));
        assert_eq!(p.active_alternative, 2);
        assert!(!remove_other(&mut p, 2), "not the one on show");
        assert!(remove_other(&mut p, 0));
        assert_eq!(p.active_alternative, 1);
        assert_eq!(p.alternatives[p.active_alternative].name, "C");
    }

    #[test]
    fn every_version_goes_in_the_file_with_its_audio() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let (a, b) = (dir.path().join("a.wav"), dir.path().join("b.wav"));
        project_io::tests::write_wav(&a);
        project_io::tests::write_wav(&b);

        let mut p = Project::empty();
        with_notes(&mut p, 2);
        crate::import::import_audio(&mut p, &a, &ctx).unwrap();
        assert!(create(&mut p, "With B"));
        crate::import::import_audio(&mut p, &b, &ctx).unwrap();

        let file = dir.path().join("song.ovrtone");
        project_io::save_project(&p, &file).unwrap();
        // The audio can go from the disk now: the file has it.
        std::fs::remove_file(&a).unwrap();
        std::fs::remove_file(&b).unwrap();

        let (mut opened, warnings) = project_io::open_project(&file, &ctx).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(opened.active_alternative, 1);
        assert_eq!(opened.alternatives[0].name, "Alternative 1");
        let audio = |p: &Project| {
            p.tracks
                .iter()
                .flat_map(|t| &t.regions)
                .filter(|r| r.audio.is_some())
                .count()
        };
        assert_eq!(audio(&opened), 2);
        let (first, warnings) = bring_back(&mut opened, 0, &ctx).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!((audio(&first), note_count(&first)), (1, 2));

        // Saving what was opened, with the other one on show, loses neither.
        let again = dir.path().join("again.ovrtone");
        project_io::save_project(&first, &again).unwrap();
        let (mut twice, warnings) = project_io::open_project(&again, &ctx).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(audio(&twice), 1);
        let (second, warnings) = bring_back(&mut twice, 1, &ctx).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(audio(&second), 2);
    }

    #[test]
    fn a_kept_version_keeps_its_plugin_settings() {
        let ctx = egui::Context::default();
        let mut p = Project::empty();
        p.tracks[0].plugins.push(crate::model::PluginInsert {
            row: 0,
            uid: 77,
            id: "x.y".into(),
            name: "Y".into(),
            vendor: String::new(),
            path: "y.clap".into(),
            bypass: false,
            failed: None,
            state: Some(Arc::new(vec![1, 2, 3])),
            params: Arc::default(),
        });
        assert!(create(&mut p, "B"));
        p.tracks[0].plugins[0].state = Some(Arc::new(vec![9]));
        let (back, _) = bring_back(&mut p, 0, &ctx).unwrap();
        let plugin = &back.tracks[0].plugins[0];
        assert_eq!(plugin.state.as_deref(), Some(&vec![1, 2, 3]));
        assert_ne!(plugin.uid, 77, "a new instance, loaded from those settings");
    }

    #[test]
    fn making_or_naming_one_is_an_unsaved_change_and_can_be_undone() {
        let mut p = Project::empty();
        let mut h = crate::undo::History::new(&p);
        let before = crate::file_commands::content(&p);
        assert!(create(&mut p, "B"));
        assert_ne!(crate::file_commands::content(&p), before);
        assert!(h.record(&p));
        assert!(h.undo(&mut p));
        assert!(p.alternatives.is_empty());
        assert_eq!(crate::file_commands::content(&p), before);
    }

    #[test]
    fn an_old_file_has_no_alternatives() {
        let doc: ovrtone_project::ProjectDoc = serde_json::from_str("{}").unwrap();
        let (p, _) = project_io::from_doc(&doc, std::path::Path::new("."), &egui::Context::default());
        assert!(p.alternatives.is_empty());
    }
}
