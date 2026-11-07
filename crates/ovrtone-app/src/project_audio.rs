//! Project Audio Browser (Logic: Project Audio): every audio file in the project, with the ones no
//! region plays marked, and a way to put one on a track.
//!
//! A file stays on the list after its last region is deleted, and is saved in the project with
//! it, until the user removes it from the list. Removing it never deletes anything on disk.
use crate::model::{AudioRegion, Project, TrackKind};
use crate::UiState;
use ovrtone_engine::AudioFile;
use std::path::Path;

/// The file a region's audio comes from: the recording itself when it has a Flex edit, whose
/// sound is made from it and saved as the edit.
pub fn source_of(a: &AudioRegion) -> &AudioFile {
    a.flex.as_ref().map_or(&a.file, |f| &f.original)
}

/// Adds the files regions play that are not on the list yet.
pub fn sync_pool(p: &mut Project) {
    let Project {
        tracks, audio_pool, ..
    } = p;
    for a in tracks
        .iter()
        .flat_map(|t| &t.regions)
        .filter_map(|r| r.audio.as_ref())
    {
        let file = source_of(a);
        if !audio_pool.iter().any(|f| f.path == file.path) {
            audio_pool.push(file.clone());
        }
    }
}

/// How many regions play `path`.
pub fn uses_of(p: &Project, path: &Path) -> usize {
    p.tracks
        .iter()
        .flat_map(|t| &t.regions)
        .filter_map(|r| r.audio.as_ref())
        .filter(|a| source_of(a).path == path)
        .count()
}

/// The files no region plays.
pub fn unused(p: &Project) -> Vec<&AudioFile> {
    p.audio_pool
        .iter()
        .filter(|f| uses_of(p, &f.path) == 0)
        .collect()
}

/// Puts `file` on the selected track when that is an audio track, else on a new one, at the bar
/// the playhead is in.
pub fn add_to_track(p: &mut Project, path: &Path, ctx: &egui::Context) -> Result<(), String> {
    let bar = p.beats_to_bar(p.position_beats).floor() + 1.0;
    let onto = Some(p.selected_track).filter(|t| {
        p.tracks
            .get(*t)
            .is_some_and(|track| track.kind == TrackKind::Audio)
    });
    crate::import::import_audio_at(p, path, ctx, bar, onto).map_err(|e| e.to_string())
}

/// The region to open in the Audio File Editor for a file on the list: one that plays it, or else
/// the file on its own.
fn editable(p: &Project, file: &AudioFile, ctx: &egui::Context) -> AudioRegion {
    p.tracks
        .iter()
        .flat_map(|t| &t.regions)
        .filter_map(|r| r.audio.as_ref())
        .find(|a| a.file.path == file.path)
        .cloned()
        .unwrap_or_else(|| crate::import::audio_region(file.clone(), ctx))
}

/// What a row asks for.
enum Asked {
    Add(usize),
    Edit(usize),
    Remove(usize),
}

/// The pane.
pub fn show(ui: &mut egui::Ui, p: &mut Project, s: &mut UiState) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.strong("Project Audio");
    });
    let to = if p
        .tracks
        .get(p.selected_track)
        .is_some_and(|t| t.kind == TrackKind::Audio)
    {
        "the selected track"
    } else {
        "a new track"
    };
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.small(format!("Add puts a file on {to}, at the playhead."));
    });
    ui.separator();
    if p.audio_pool.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label("No audio files in this project yet.");
        });
        return;
    }
    let mut asked = None;
    egui::ScrollArea::vertical().show(ui, |ui| {
        for (i, f) in p.audio_pool.iter().enumerate() {
            let uses = uses_of(p, &f.path);
            let missing = !f.path.exists();
            let name = f
                .path
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.label(egui::RichText::new(name).strong())
                    .on_hover_text(f.path.display().to_string());
            });
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.small(format!(
                    "{:.1} s, {} Hz, {}",
                    f.duration_secs(),
                    f.sample_rate,
                    if f.channels == 1 { "mono" } else { "stereo" },
                ));
                let warn = egui::Color32::from_rgb(0xff, 0xb0, 0x40);
                if missing {
                    ui.colored_label(warn, "Missing");
                } else if uses == 0 {
                    ui.colored_label(warn, "Unused");
                } else {
                    ui.small(format!("{uses} region{}", if uses == 1 { "" } else { "s" }));
                }
            });
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                if ui.add_enabled(!missing, egui::Button::new("Add")).clicked() {
                    asked = Some(Asked::Add(i));
                }
                if ui
                    .add_enabled(!missing, egui::Button::new("Edit"))
                    .on_hover_text("Open it in the Audio File Editor")
                    .clicked()
                {
                    asked = Some(Asked::Edit(i));
                }
                // Only what nothing plays, and the file stays on the disk.
                if uses == 0
                    && ui
                        .button("Remove")
                        .on_hover_text("Take it off this list and out of the project. The file on disk stays.")
                        .clicked()
                {
                    asked = Some(Asked::Remove(i));
                }
            });
            ui.separator();
        }
    });
    let ctx = ui.ctx().clone();
    match asked {
        Some(Asked::Add(i)) => {
            let path = p.audio_pool[i].path.clone();
            if let Err(e) = add_to_track(p, &path, &ctx) {
                s.status = Some(format!("cannot add {}: {e}", path.display()));
            }
        }
        Some(Asked::Edit(i)) => {
            let file = editable(p, &p.audio_pool[i], &ctx);
            s.file_edit.open(file);
        }
        Some(Asked::Remove(i)) => {
            p.audio_pool.remove(i);
        }
        None => {}
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn wav(dir: &Path, name: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        crate::project_io::tests::write_wav(&path);
        path
    }

    #[test]
    fn a_file_stays_listed_and_marked_unused_after_its_last_region_goes() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut p = Project::empty();
        let (a, b) = (wav(dir.path(), "a.wav"), wav(dir.path(), "b.wav"));
        crate::import::import_audio(&mut p, &a, &ctx).unwrap();
        crate::import::import_audio(&mut p, &b, &ctx).unwrap();
        sync_pool(&mut p);
        sync_pool(&mut p);
        assert_eq!(p.audio_pool.len(), 2, "each file once");
        assert!(unused(&p).is_empty());
        assert_eq!(uses_of(&p, &a), 1);

        let t = p
            .tracks
            .iter()
            .position(|t| t.regions.iter().any(|r| r.name == "a"))
            .unwrap();
        p.tracks[t].regions.clear();
        sync_pool(&mut p);
        assert_eq!(p.audio_pool.len(), 2);
        let gone: Vec<&Path> = unused(&p).iter().map(|f| f.path.as_path()).collect();
        assert_eq!(gone, [a.as_path()]);
    }

    #[test]
    fn adding_a_file_goes_on_the_selected_audio_track_or_a_new_one() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut p = Project::empty();
        let a = wav(dir.path(), "a.wav");
        let tracks = p.tracks.len();
        p.selected_track = 0;
        add_to_track(&mut p, &a, &ctx).unwrap();
        assert_eq!(p.tracks.len(), tracks + 1, "an instrument track holds no audio");
        let audio = p.selected_track;
        p.position_beats = 8.0;
        add_to_track(&mut p, &a, &ctx).unwrap();
        assert_eq!(p.tracks.len(), tracks + 1);
        assert_eq!(p.tracks[audio].regions.len(), 2);
        assert_eq!(p.tracks[audio].regions[1].start_bar, 3.0);
    }

    #[test]
    fn an_unused_file_is_saved_with_the_project_and_comes_back_unused() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut p = Project::empty();
        let (a, b) = (wav(dir.path(), "a.wav"), wav(dir.path(), "b.wav"));
        crate::import::import_audio(&mut p, &a, &ctx).unwrap();
        crate::import::import_audio(&mut p, &b, &ctx).unwrap();
        sync_pool(&mut p);
        let t = p.tracks.iter().position(|t| t.name == "a").unwrap();
        p.tracks[t].regions.clear();

        let file = dir.path().join("song.ovrtone");
        crate::project_io::save_project(&p, &file).unwrap();
        let (back, warnings) = crate::project_io::open_project(&file, &ctx).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(back.audio_pool.len(), 2);
        assert_eq!(unused(&back).len(), 1);
        assert!(unused(&back)[0].path.to_string_lossy().contains("a.wav"));
    }

    #[test]
    fn an_unused_file_that_is_gone_from_the_disk_does_not_stop_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut p = Project::empty();
        let a = wav(dir.path(), "a.wav");
        crate::import::import_audio(&mut p, &a, &ctx).unwrap();
        sync_pool(&mut p);
        let t = p.tracks.iter().position(|t| t.name == "a").unwrap();
        p.tracks[t].regions.clear();
        std::fs::remove_file(&a).unwrap();
        let file = dir.path().join("song.ovrtone");
        crate::project_io::save_project(&p, &file).unwrap();
        let (back, _) = crate::project_io::open_project(&file, &ctx).unwrap();
        assert!(back.audio_pool.is_empty());
    }

    #[test]
    fn unused_files_do_not_count_as_a_change_until_they_are_unused() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut p = Project::empty();
        let a = wav(dir.path(), "a.wav");
        crate::import::import_audio(&mut p, &a, &ctx).unwrap();
        let before = crate::file_commands::content(&p);
        sync_pool(&mut p);
        assert_eq!(
            crate::file_commands::content(&p),
            before,
            "listing a file the regions play is no edit"
        );
    }
}
