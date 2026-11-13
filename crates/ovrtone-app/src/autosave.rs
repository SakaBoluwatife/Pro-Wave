//! Crash recovery. While a project has unsaved changes, a copy goes to the config folder every two
//! minutes, on a thread of its own. It never touches the project's own file. A clean exit or a save
//! removes the copy, so one found at startup means a program that did not get to close, and the
//! user is offered what it was working on.
//!
//! The copies are named by process, so a second running Ovrtone does not take the first one's copy
//! for a crash.
use crate::App;
use ovrtone_project::ProjectDoc;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

const EVERY: Duration = Duration::from_secs(120);
const PREFIX: &str = "recovery-";
const EXTENSION: &str = ovrtone_project::EXTENSION;
/// Beside the copy: which file it was a copy of, and the project's name.
const NOTE: &str = "txt";

/// A copy left by a program that did not close.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recovery {
    pub file: PathBuf,
    /// The project's own file, if it had one.
    pub original: Option<PathBuf>,
    pub name: String,
}

pub struct Autosave {
    next: Instant,
    /// What the copy on disk holds, so an unchanged project is not written again.
    written: Option<ProjectDoc>,
    job: Option<Receiver<Result<(), String>>>,
    /// Waiting for the user's answer.
    pub found: Vec<Recovery>,
}

impl Default for Autosave {
    fn default() -> Self {
        Self {
            next: Instant::now() + EVERY,
            written: None,
            job: None,
            found: Vec::new(),
        }
    }
}

fn folder() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    Some(crate::recent_files::config_dir()?.join("autosave"))
}

fn copy_in(dir: &Path, pid: u32) -> PathBuf {
    dir.join(format!("{PREFIX}{pid}.{EXTENSION}"))
}

fn write_note(file: &Path, original: Option<&Path>, name: &str) -> std::io::Result<()> {
    let original = original.map_or(String::new(), |p| p.display().to_string());
    std::fs::write(file.with_extension(NOTE), format!("{original}\n{name}\n"))
}

fn remove(file: &Path) {
    let _ = std::fs::remove_file(file);
    let _ = std::fs::remove_file(file.with_extension(NOTE));
}

/// The copies in `dir` whose program is gone, newest first.
fn found_in(dir: &Path, alive: &dyn Fn(u32) -> bool) -> Vec<Recovery> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<(std::time::SystemTime, Recovery)> = Vec::new();
    for entry in entries.flatten() {
        let file = entry.path();
        // Copies left by a version from before the rename have the old extension.
        if !ovrtone_project::is_project_file(&file) {
            continue;
        }
        let pid = file
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.strip_prefix(PREFIX))
            .and_then(|s| s.parse::<u32>().ok());
        let Some(pid) = pid else { continue };
        if alive(pid) {
            continue;
        }
        let note = std::fs::read_to_string(file.with_extension(NOTE)).unwrap_or_default();
        let mut lines = note.lines();
        let original = lines.next().filter(|l| !l.is_empty()).map(PathBuf::from);
        let name = lines.next().unwrap_or("Untitled").to_string();
        let when = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        out.push((
            when,
            Recovery {
                file,
                original,
                name,
            },
        ));
    }
    out.sort_by_key(|x| std::cmp::Reverse(x.0));
    out.into_iter().map(|(_, r)| r).collect()
}

fn process_alive(pid: u32) -> bool {
    use sysinfo::{Pid, ProcessesToUpdate, System};
    let pid = Pid::from_u32(pid);
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
    system.process(pid).is_some()
}

impl Autosave {
    /// Looks for what a crash left behind. Called once at startup.
    pub fn look_for_recovery(&mut self) {
        if let Some(dir) = folder() {
            self.found = found_in(&dir, &process_alive);
        }
    }

    /// Removes this program's copy: the project is saved, or the program closes.
    pub fn forget(&mut self) {
        self.written = None;
        if let Some(dir) = folder() {
            remove(&copy_in(&dir, std::process::id()));
        }
    }
}

impl App {
    /// Makes a copy if one is due. Called every frame; costs nothing until then.
    pub fn autosave_tick(&mut self) {
        if let Some(job) = &self.autosave.job {
            match job.try_recv() {
                Ok(Err(e)) => {
                    tracing::warn!("autosave failed: {e}");
                    self.autosave.job = None;
                }
                Ok(Ok(())) => self.autosave.job = None,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.autosave.job = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
            }
        }
        if Instant::now() < self.autosave.next {
            return;
        }
        // As often as Settings > General says; never when it is off.
        let minutes = self.ui.autosave_minutes;
        self.autosave.next = Instant::now()
            + if minutes == 0 { EVERY } else { Duration::from_secs(u64::from(minutes) * 60) };
        if minutes == 0 {
            return;
        }
        let Some(dir) = folder() else { return };
        if self.ui.rec.active() || self.ui.audio_rec.active() {
            return;
        }
        let doc = crate::file_commands::content(&self.project);
        let touched = self.sync.as_ref().is_some_and(|s| s.plugins_touched());
        if doc == self.saved && !touched {
            self.autosave.forget();
            return;
        }
        if self.autosave.written.as_ref() == Some(&doc) && !touched {
            return;
        }
        if let Some(sync) = self.sync.as_mut() {
            sync.collect_plugin_states(&mut self.project);
        }
        let (full, media) = crate::project_io::doc_for_saving(&self.project);
        self.autosave.written = Some(doc);
        let file = copy_in(&dir, std::process::id());
        let (original, name) = (self.path.clone(), self.project.name.clone());
        let (tx, rx) = channel();
        self.autosave.job = Some(rx);
        let spawned = std::thread::Builder::new()
            .name("ovrtone-autosave".into())
            .spawn(move || {
                let done = std::fs::create_dir_all(&dir)
                    .map_err(|e| e.to_string())
                    .and_then(|()| {
                        ovrtone_project::save(&file, &full, &media).map_err(|e| e.to_string())
                    })
                    .and_then(|()| {
                        write_note(&file, original.as_deref(), &name).map_err(|e| e.to_string())
                    });
                let _ = tx.send(done);
            });
        if let Err(e) = spawned {
            tracing::warn!("autosave could not start: {e}");
            self.autosave.job = None;
        }
    }

    /// Asks what to do with what a crash left behind.
    pub fn show_recovery(&mut self, ctx: &egui::Context) {
        let Some(found) = self.autosave.found.first().cloned() else {
            return;
        };
        let (mut recover, mut discard) = (false, false);
        egui::Modal::new(egui::Id::unique("recovery_prompt")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.heading("Ovrtone did not close properly");
            ui.add_space(6.0);
            ui.label(format!(
                "There are unsaved changes to \"{}\". Do you want them back?",
                found.name
            ));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                recover = ui.button("Recover").clicked();
                discard = ui.button("Discard").clicked();
            });
        });
        if discard {
            remove(&found.file);
            self.autosave.found.remove(0);
        } else if recover {
            match crate::project_io::open_project(&found.file, ctx) {
                Ok((mut project, warnings)) => {
                    project.name = found.name.clone();
                    self.replace_recovered(project, found.original.clone());
                    self.ui.status = Some(if warnings.is_empty() {
                        format!("Recovered \"{}\": save it to keep it", found.name)
                    } else {
                        warnings.join("; ")
                    });
                    remove(&found.file);
                    self.autosave.found.clear();
                }
                Err(e) => {
                    self.ui.status = Some(format!("Could not recover it: {e}"));
                    self.autosave.found.remove(0);
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_is_found_only_when_its_program_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let file = copy_in(dir.path(), 4242);
        std::fs::write(&file, b"x").unwrap();
        write_note(&file, Some(Path::new("D:\\Songs\\a.ovrtone")), "My Song").unwrap();

        assert!(found_in(dir.path(), &|_| true).is_empty(), "still running");
        let found = found_in(dir.path(), &|_| false);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "My Song");
        assert_eq!(
            found[0].original.as_deref(),
            Some(Path::new("D:\\Songs\\a.ovrtone"))
        );

        remove(&file);
        assert!(found_in(dir.path(), &|_| false).is_empty());
        assert!(!file.with_extension(NOTE).exists());
    }

    #[test]
    fn a_project_that_never_had_a_file_comes_back_without_one() {
        let dir = tempfile::tempdir().unwrap();
        let file = copy_in(dir.path(), 7);
        std::fs::write(&file, b"x").unwrap();
        write_note(&file, None, "Untitled").unwrap();
        assert_eq!(found_in(dir.path(), &|_| false)[0].original, None);
        // Other files in the folder are not copies.
        std::fs::write(dir.path().join("song.ovrtone"), b"x").unwrap();
        assert_eq!(found_in(dir.path(), &|_| false).len(), 1);
    }

    #[test]
    fn a_copy_left_with_the_old_extension_is_still_found() {
        let dir = tempfile::tempdir().unwrap();
        let file = copy_in(dir.path(), 7).with_extension(ovrtone_project::LEGACY_EXTENSION);
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(found_in(dir.path(), &|_| false).len(), 1);
    }

    #[test]
    fn a_copy_written_for_recovery_opens_with_the_work_in_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = crate::model::Project::empty();
        p.tempo = 97.0;
        p.tracks[0].name = "Unsaved Work".to_string();
        let (doc, media) = crate::project_io::doc_for_saving(&p);
        let file = copy_in(dir.path(), 9);
        ovrtone_project::save(&file, &doc, &media).unwrap();
        let ctx = egui::Context::default();
        let (back, _) = crate::project_io::open_project(&file, &ctx).unwrap();
        assert_eq!(back.tempo, 97.0);
        assert_eq!(back.tracks[0].name, "Unsaved Work");
    }
}
