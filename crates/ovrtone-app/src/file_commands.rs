//! File > New… / Open… / Open Recent / Save / Save As… / Save A Copy As… / Revert to Saved:
//! native dialogs (rfd), key commands and the "save changes?" and "revert?" prompts.
use crate::i18n::{tr, trf};
use crate::model::Project;
use crate::start_screen::Template;
use crate::{project_io, App};
use egui::{Key, KeyboardShortcut, Modifiers};
use ovrtone_project::ProjectDoc;
use rfd::FileDialog;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FileCmd {
    New,
    /// Closes the project and leaves a new, empty one.
    Close,
    Open,
    Save,
    SaveAs,
    /// Writes the project to another file, which does not become the current one.
    SaveCopy,
    /// Reloads the project from its file, after asking.
    Revert,
    /// The Project History window: the backups, to open one.
    History,
    /// Opens `UiState::recent_pick` (File > Open Recent).
    OpenRecent,
    /// Starts a new project from a template (Project Chooser).
    NewFrom(Template),
    /// Starts a new project from the template file in `UiState::recent_pick`.
    NewFromFile,
}

impl FileCmd {
    /// Save As before Save: egui ignores an extra Shift when matching Cmd+S.
    const ALL: [FileCmd; 4] = [FileCmd::SaveAs, FileCmd::Save, FileCmd::Open, FileCmd::New];

    /// Logic's key commands: Cmd+N, Cmd+O, Cmd+S, Shift+Cmd+S (Ctrl on Windows/Linux). The
    /// other commands are menu-only.
    pub fn shortcut(self) -> Option<KeyboardShortcut> {
        let (modifiers, key) = match self {
            FileCmd::New => (Modifiers::COMMAND | Modifiers::SHIFT, Key::N),
            FileCmd::Open => (Modifiers::COMMAND, Key::O),
            FileCmd::Save => (Modifiers::COMMAND, Key::S),
            FileCmd::SaveAs => (Modifiers::COMMAND | Modifiers::SHIFT, Key::S),
            FileCmd::Close
            | FileCmd::SaveCopy
            | FileCmd::Revert
            | FileCmd::History
            | FileCmd::OpenRecent
            | FileCmd::NewFromFile
            | FileCmd::NewFrom(_) => return None,
        };
        Some(KeyboardShortcut::new(modifiers, key))
    }

    /// The File menu entry that runs this command.
    pub fn from_menu(label: &str) -> Option<Self> {
        match label {
            "New…" => Some(FileCmd::New),
            "Close Project" | "Close" => Some(FileCmd::Close),
            "Open…" => Some(FileCmd::Open),
            "Save" => Some(FileCmd::Save),
            "Save As…" => Some(FileCmd::SaveAs),
            "Save A Copy As…" => Some(FileCmd::SaveCopy),
            "Revert to Saved" => Some(FileCmd::Revert),
            "Project History…" => Some(FileCmd::History),
            _ => None,
        }
    }
}

/// Work on the project's file that needs `App` but is not a File command with a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileTask {
    ImportAudio,
    ImportProject,
    Consolidate,
    Reveal,
    SaveTemplate,
    Rename(String),
    /// Track `n` with its effects, plugins (their settings too), sends and mix, no regions.
    SaveTrackTemplate(usize),
    /// The effects and plugins of track `n`.
    SaveChainPreset(usize),
    /// Every track's set-up, no regions: a starting mixer for other projects.
    SaveMixerTemplate,
    /// A new track (or tracks) from a template file.
    LoadTracks(PathBuf),
    /// The effects and plugins of a chain preset onto track `n`, in place of its own.
    LoadChain(PathBuf, usize),
}

/// Where track templates, effect chains and mixer templates are kept.
pub fn template_dir(kind: TemplateKind) -> Option<PathBuf> {
    let name = match kind {
        TemplateKind::Track => "track-templates",
        TemplateKind::Chain => "effect-chains",
        TemplateKind::Mixer => "mixer-templates",
    };
    crate::recent_files::config_dir().map(|d| d.join(name))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateKind {
    Track,
    Chain,
    Mixer,
}

/// The templates of one kind: their names and files, by name.
pub fn templates(kind: TemplateKind) -> Vec<(String, PathBuf)> {
    let Some(dir) = template_dir(kind) else {
        return Vec::new();
    };
    let mut out: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
        .map(|it| {
            it.flatten()
                .map(|e| e.path())
                .filter(|p| ovrtone_project::is_project_file(p))
                .filter_map(|p| Some((p.file_stem()?.to_string_lossy().into_owned(), p)))
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|(n, _)| n.to_lowercase());
    out
}

/// Copies the project file at `path` (if there is one) into `Backups` beside it, named with the
/// time, and lets the oldest go past `kept`. A failure only goes to the log: the save goes
/// ahead.
fn keep_backup(path: &Path, kept: usize) {
    if !path.is_file() || kept == 0 {
        return;
    }
    let (Some(dir), Some(stem)) = (path.parent(), path.file_stem().and_then(|s| s.to_str())) else {
        return;
    };
    let backups = dir.join("Backups");
    if let Err(e) = std::fs::create_dir_all(&backups) {
        tracing::warn!("cannot make {}: {e}", backups.display());
        return;
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or(ovrtone_project::EXTENSION);
    let copy = backups.join(format!("{stem} backup {stamp}.{ext}"));
    if let Err(e) = std::fs::copy(path, &copy) {
        tracing::warn!("cannot back up {}: {e}", path.display());
        return;
    }
    let prefix = format!("{stem} backup ");
    let mut older: Vec<PathBuf> = std::fs::read_dir(&backups)
        .map(|it| {
            it.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with(&prefix))
                })
                .collect()
        })
        .unwrap_or_default();
    // The names end in the time, so the oldest sort first.
    older.sort();
    while older.len() > kept {
        let gone = older.remove(0);
        let _ = std::fs::remove_file(gone);
    }
}

/// Whether `name` can be a file name on every system.
pub fn valid_project_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|'])
}

/// The file command whose key command was pressed this frame, if any (consumes the keys).
pub fn pressed(ctx: &egui::Context) -> Option<FileCmd> {
    FileCmd::ALL.into_iter().find(|c| {
        c.shortcut()
            .is_some_and(|s| ctx.input_mut(|i| i.consume_shortcut(&s)))
    })
}

/// What gets saved, minus state that changes on its own (playhead, selected track), so that
/// playing or clicking around does not count as an unsaved change.
pub fn content(p: &Project) -> ProjectDoc {
    let mut doc = project_io::to_doc(p).0;
    doc.position_beats = 0.0;
    doc.selected_track = 0;
    doc
}

/// Shows the file in the system's file manager.
pub(crate) fn reveal_in_file_manager(path: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("explorer")
            .raw_arg(format!("/select,\"{}\"", path.display()))
            .spawn()
            .map(|_| ())
    }
    #[cfg(not(windows))]
    {
        let program = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        let folder = path.parent().unwrap_or(path);
        std::process::Command::new(program)
            .arg(folder)
            .spawn()
            .map(|_| ())
    }
}

fn dialog() -> FileDialog {
    FileDialog::new()
        .add_filter(tr("Ovrtone Project"), &ovrtone_project::OPEN_EXTENSIONS)
        .add_filter("Other DAW Sessions (DAWproject, REAPER, Ableton Live, Ardour, AAF)", &["dawproject", "rpp", "als", "ardour", "aaf"])
        .add_filter("Logic Pro project (pick a file inside the .logicx folder)", &["plist", "logicx"])
        .add_filter("Pro Tools Session Info as Text", &["txt"])
}

/// The answer to the "save changes?" prompt.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Answer {
    Save,
    DontSave,
    Cancel,
}

impl App {
    /// Closing Ovrtone always asks first, whatever closed it (the window's close button, Alt+F4,
    /// Ctrl+Q): with unsaved changes Save, Don't Save or Cancel, else Quit or Cancel. Once a frame.
    pub(crate) fn quit_guard(&mut self, ctx: &egui::Context) {
        if self.quitting {
            return;
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.quit_asked = true;
        }
        if !self.quit_asked {
            return;
        }
        #[derive(Clone, Copy)]
        enum Choice {
            Save,
            Quit,
            Cancel,
        }
        let unsaved = content(&self.project) != self.saved;
        let mut choice = None;
        egui::Modal::new(egui::Id::unique("quit_ovrtone")).show(ctx, |ui| {
            ui.set_width(320.0);
            ui.heading("Quit Ovrtone?");
            ui.add_space(4.0);
            if unsaved {
                ui.label(format!("\"{}\" has changes that are not saved.", self.project.name));
            } else {
                ui.label("Everything is saved.");
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if unsaved {
                    if ui.button("Save and Quit").clicked() {
                        choice = Some(Choice::Save);
                    }
                    if ui.button("Don't Save").clicked() {
                        choice = Some(Choice::Quit);
                    }
                } else if ui.button("Quit").clicked() {
                    choice = Some(Choice::Quit);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(Choice::Cancel);
                }
            });
        });
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            choice = Some(Choice::Cancel);
        }
        match choice {
            Some(Choice::Save) => {
                if self.save() {
                    self.quitting = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                self.quit_asked = false;
            }
            Some(Choice::Quit) => {
                self.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Some(Choice::Cancel) => self.quit_asked = false,
            None => {}
        }
    }

    /// Runs a File menu command. New / Open / Open Recent first ask what to do with unsaved
    /// changes, and Revert asks before throwing them away.
    pub fn run_file_cmd(&mut self, cmd: FileCmd, ctx: &egui::Context) {
        let discards = matches!(
            cmd,
            FileCmd::New
                | FileCmd::Close
                | FileCmd::Open
                | FileCmd::OpenRecent
                | FileCmd::NewFromFile
                | FileCmd::NewFrom(_)
        );
        if cmd == FileCmd::Revert {
            self.request_revert();
        } else if discards && self.has_unsaved_changes() {
            self.ui.ask_save = Some(cmd);
        } else {
            self.do_file_cmd(cmd, ctx);
        }
        ctx.request_repaint();
    }

    fn do_file_cmd(&mut self, cmd: FileCmd, ctx: &egui::Context) {
        match cmd {
            FileCmd::New | FileCmd::Close => self.replace(Project::empty(), None),
            FileCmd::NewFrom(t) => {
                let mut p = t.project();
                if let Some(details) = self.ui.new_details.take() {
                    details.apply(&mut p);
                }
                self.replace(p, None);
            }
            FileCmd::NewFromFile => {
                if let Some(path) = self.ui.recent_pick.take() {
                    match project_io::open_project(&path, ctx) {
                        Ok((mut p, _)) => {
                            p.name = "Untitled".to_string();
                            if let Some(details) = self.ui.new_details.take() {
                                details.apply(&mut p);
                            }
                            self.replace(p, None);
                        }
                        Err(e) => {
                            self.ui.status = Some(format!("cannot open {}: {e}", path.display()));
                        }
                    }
                }
            }
            FileCmd::Open => {
                if let Some(path) = dialog().set_title(tr("Open Project")).pick_file() {
                    self.open(&path, ctx);
                }
            }
            FileCmd::Save => {
                self.save();
            }
            FileCmd::SaveAs => {
                self.save_as();
            }
            FileCmd::SaveCopy => {
                self.save_copy();
            }
            FileCmd::OpenRecent => {
                if let Some(path) = self.ui.recent_pick.take() {
                    self.open(&path, ctx);
                }
            }
            FileCmd::Revert => self.revert(ctx),
            FileCmd::History => self.ui.history_open = true,
        }
    }

    /// Offered after saving to an old `.prowave` file. The save has already happened, so that
    /// answering late, or not at all, loses nothing.
    pub fn show_new_extension_offer(&mut self, ctx: &egui::Context) {
        if !self.ui.offer_new_extension {
            return;
        }
        let (old, new) = (
            ovrtone_project::LEGACY_EXTENSION,
            ovrtone_project::EXTENSION,
        );
        let (mut move_on, mut keep) = (false, false);
        let modal = egui::Modal::new(egui::Id::unique(("file", "new_extension"))).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading("Ovrtone");
            ui.add_space(6.0);
            ui.label(format!(
                "This project was saved as a .{old} file. Save it as .{new} from now on? The .{old} file stays where it is."
            ));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                keep = ui.button(format!("Keep .{old}")).clicked();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    move_on = ui.button(format!("Save as .{new}")).clicked();
                });
            });
        });
        if keep || modal.should_close() {
            self.ui.offer_new_extension = false;
            self.ui.keep_old_extension.clone_from(&self.path);
        } else if move_on {
            self.ui.offer_new_extension = false;
            self.save_as();
        }
    }

    /// The "Do you want to save the changes…?" prompt. Drawn by the app itself (not a system
    /// message box), so it looks the same everywhere and needs no helper program on Linux.
    pub fn show_save_prompt(&mut self, ctx: &egui::Context) {
        let Some(cmd) = self.ui.ask_save else {
            return;
        };
        let mut answer = None;
        let modal = egui::Modal::new(egui::Id::unique(("file", "save_prompt"))).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading("Ovrtone");
            ui.add_space(6.0);
            ui.label(trf(
                "Do you want to save the changes made to the project \"{}\"?",
                &[&self.project.name],
            ));
            ui.label(tr("Your changes will be lost if you don't save them."));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button(tr("Don't Save")).clicked() {
                    answer = Some(Answer::DontSave);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(tr("Save")).clicked() {
                        answer = Some(Answer::Save);
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        answer = Some(Answer::Cancel);
                    }
                });
            });
        });
        if modal.should_close() && answer.is_none() {
            answer = Some(Answer::Cancel);
        }
        let Some(answer) = answer else {
            return;
        };
        self.ui.ask_save = None;
        let proceed = match answer {
            Answer::Save => self.save(),
            Answer::DontSave => true,
            Answer::Cancel => false,
        };
        if proceed {
            self.do_file_cmd(cmd, ctx);
        } else {
            self.ui.recent_pick = None;
        }
        ctx.request_repaint();
    }

    /// The "revert to the last saved version?" prompt, drawn like the save prompt.
    pub fn show_revert_prompt(&mut self, ctx: &egui::Context) {
        if !self.ui.ask_revert {
            return;
        }
        let mut answer = None;
        let modal = egui::Modal::new(egui::Id::unique(("file", "revert_prompt"))).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading("Ovrtone");
            ui.add_space(6.0);
            ui.label(trf(
                "Do you want to revert the project \"{}\" to the last saved version?",
                &[&self.project.name],
            ));
            ui.label(tr("All changes made since it was saved will be lost."));
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(tr("Revert")).clicked() {
                    answer = Some(true);
                }
                if ui.button(tr("Cancel")).clicked() {
                    answer = Some(false);
                }
            });
        });
        if modal.should_close() && answer.is_none() {
            answer = Some(false);
        }
        let Some(revert) = answer else {
            return;
        };
        self.ui.ask_revert = false;
        if revert {
            self.revert(ctx);
        }
        ctx.request_repaint();
    }

    /// Carries out a [`FileTask`].
    pub fn run_file_task(&mut self, task: FileTask, ctx: &egui::Context) {
        match task {
            FileTask::ImportAudio => {
                let files = FileDialog::new()
                    .add_filter("Audio", &["wav", "aif", "aiff", "flac", "mp3", "ogg"])
                    .pick_files()
                    .unwrap_or_default();
                for file in files {
                    if let Err(e) = crate::import::import_audio(&mut self.project, &file, ctx) {
                        self.ui.status = Some(format!("{}: {e}", file.display()));
                    }
                }
            }
            FileTask::ImportProject => {
                let Some(file) = dialog().set_title("Import from Project").pick_file() else {
                    return;
                };
                match project_io::open_project(&file, ctx) {
                    Ok((project, _)) => {
                        let picks = vec![true; project.tracks.len()];
                        self.ui.dialogs.import_project = Some((Box::new(project), picks));
                    }
                    Err(e) => {
                        self.ui.status = Some(format!("cannot open {}: {e}", file.display()));
                    }
                }
            }
            // A saved project holds its own copy of every audio file it uses.
            FileTask::Consolidate => {
                if self.save() {
                    let n = project_io::doc_for_saving(&self.project).1.len();
                    self.ui.status =
                        Some(format!("{n} audio files are now inside the project file"));
                }
            }
            FileTask::Reveal => match &self.path {
                Some(path) => {
                    if let Err(e) = reveal_in_file_manager(path) {
                        self.ui.status = Some(e.to_string());
                    }
                }
                None => self.ui.status = Some("The project has not been saved yet".to_string()),
            },
            FileTask::Rename(name) => self.rename_project(name.trim()),
            FileTask::SaveTemplate => self.save_template(),
            FileTask::SaveTrackTemplate(t) => self.save_tracks_as(TemplateKind::Track, Some(t)),
            FileTask::SaveChainPreset(t) => self.save_tracks_as(TemplateKind::Chain, Some(t)),
            FileTask::SaveMixerTemplate => self.save_tracks_as(TemplateKind::Mixer, None),
            FileTask::LoadTracks(file) => match project_io::open_project(&file, ctx) {
                Ok((from, _)) => {
                    let picks = vec![true; from.tracks.len()];
                    let n = self.project.import_tracks(&from, &picks);
                    self.ui.status = Some(format!("{n} tracks added from the template"));
                }
                Err(e) => self.ui.status = Some(format!("cannot open {}: {e}", file.display())),
            },
            FileTask::LoadChain(file, t) => match project_io::open_project(&file, ctx) {
                Ok((from, _)) => {
                    let Some(chain) = from.tracks.first() else {
                        return;
                    };
                    let Some(track) = self.project.tracks.get_mut(t) else {
                        return;
                    };
                    track.inserts = chain.inserts.clone();
                    track.plugins = chain.plugins.clone();
                    for fx in track.inserts.iter_mut() {
                        fx.sidechain = None;
                    }
                    for plugin in track.plugins.iter_mut() {
                        plugin.sidechain = None;
                    }
                    self.project.song_rev += 1;
                    self.ui.status = Some("Effect chain loaded".to_string());
                }
                Err(e) => self.ui.status = Some(format!("cannot open {}: {e}", file.display())),
            },
        }
    }

    /// Saves track `only` (or every track) as a template of `kind`: their set-up with the plugins'
    /// settings, and no regions, in the templates folder under a name the user gives.
    fn save_tracks_as(&mut self, kind: TemplateKind, only: Option<usize>) {
        let Some(dir) = template_dir(kind) else {
            return;
        };
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.ui.status = Some(e.to_string());
            return;
        }
        self.collect_plugin_states();
        let mut copy = self.project.clone();
        if let Some(t) = only {
            let Some(track) = copy.tracks.get(t).cloned() else {
                return;
            };
            copy.tracks = vec![track];
        }
        for track in copy.tracks.iter_mut() {
            track.regions.clear();
            track.auto = Default::default();
            if kind != TemplateKind::Mixer {
                track.output = 0;
                track.sends.clear();
                track.group = None;
                track.vca = None;
            }
        }
        copy.mixer_snapshots.clear();
        let title = match kind {
            TemplateKind::Track => "Save Track as Template",
            TemplateKind::Chain => "Save Effect Chain",
            TemplateKind::Mixer => "Save Mixer as Template",
        };
        let name = match only {
            Some(t) => self.project.tracks.get(t).map_or_else(String::new, |x| x.name.clone()),
            None => self.project.name.clone(),
        };
        let file = dialog()
            .set_title(title)
            .set_directory(&dir)
            .set_file_name(format!("{name}.{}", ovrtone_project::EXTENSION))
            .save_file();
        let Some(file) = file else {
            return;
        };
        self.ui.status = Some(match project_io::save_project(&copy, &file) {
            Ok(saved) => format!("Saved {}", saved.display()),
            Err(e) => format!("cannot save {}: {e}", file.display()),
        });
    }

    /// File > Save as Template: the project as it is, for New from Template. It stays the project
    /// being worked on, under its own file.
    fn save_template(&mut self) {
        let Some(dir) = crate::start_screen::template_folder() else {
            return;
        };
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.ui.status = Some(e.to_string());
            return;
        }
        let file = dialog()
            .set_title("Save as Template")
            .set_directory(&dir)
            .set_file_name(format!(
                "{}.{}",
                self.project.name,
                ovrtone_project::EXTENSION
            ))
            .save_file();
        let Some(file) = file else {
            return;
        };
        self.collect_plugin_states();
        match project_io::save_project(&self.project, &file) {
            Ok(saved) => self.ui.thumbnail_wanted = Some((saved, 3)),
            Err(e) => self.ui.status = Some(format!("cannot save {}: {e}", file.display())),
        }
    }

    /// Gives the project a new name, and its file too when it has one, in the same folder.
    fn rename_project(&mut self, name: &str) {
        if !valid_project_name(name) {
            self.ui.status = Some(format!("\"{name}\" cannot be a project name"));
            return;
        }
        if let Some(old) = self.path.clone() {
            // Only the name changes: an old file keeps its extension until the user moves it.
            let extension = old
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or(ovrtone_project::EXTENSION);
            let new = old.with_file_name(format!("{name}.{extension}"));
            if new != old {
                if new.exists() {
                    self.ui.status = Some(format!("{} already exists", new.display()));
                    return;
                }
                if let Err(e) = std::fs::rename(&old, &new) {
                    self.ui.status = Some(format!("cannot rename: {e}"));
                    return;
                }
                self.ui.recent.remove(&old);
                self.ui.recent.add(&new);
                self.path = Some(new);
            }
        }
        // The name inside the file changes with the next save, so this counts as a change.
        self.project.name = name.to_string();
    }

    /// Revert needs a saved file and something to throw away; otherwise it says why not.
    fn request_revert(&mut self) {
        if self.path.is_none() {
            self.ui.status =
                Some(tr("nothing to revert to: the project has not been saved yet").into());
        } else if !self.has_unsaved_changes() {
            self.ui.status = Some(tr("no unsaved changes to revert").into());
        } else {
            self.ui.ask_revert = true;
        }
    }

    /// Reloads the project from its file, dropping unsaved changes (no prompt).
    fn revert(&mut self, ctx: &egui::Context) {
        if let Some(path) = self.path.clone() {
            self.open(&path, ctx);
        }
    }

    /// Opens `path`, replacing the current project (no prompt; callers ask first).
    pub fn open(&mut self, path: &Path, ctx: &egui::Context) {
        // Another DAW's session: read into a new, unsaved project, with what was left behind.
        if crate::session_import::can_open(path) {
            match crate::session_import::open(path, ctx) {
                Ok((p, log)) => {
                    self.replace(p, None);
                    self.ui.opening_plugins = Some(0);
                    let name = path
                        .file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    self.ui.import_log = Some((name, log));
                }
                Err(e) => {
                    self.ui.status = Some(format!("cannot open {}: {e}", path.display()));
                }
            }
            return;
        }
        match project_io::open_project(path, ctx) {
            Ok((p, warnings)) => {
                self.replace(p, Some(path.to_path_buf()));
                self.ui.opening_plugins = Some(0);
                self.ui.recent.add(path);
                self.ui.thumbnail_wanted = Some((path.to_path_buf(), 30));
                if !warnings.is_empty() {
                    self.ui.status = Some(trf("missing media: {}", &[&warnings.join(", ")]));
                }
            }
            Err(e) => {
                // A damaged or half-copied file: its newest backup is offered instead.
                if let Some(newest) = crate::project_history::backups_of(path).into_iter().next() {
                    self.ui.damaged = Some((path.to_path_buf(), newest.path, e.to_string()));
                    return;
                }
                self.ui.recent.remove(path);
                self.ui.status = Some(trf("cannot open {}: {}", &[&path.display(), &e]));
            }
        }
    }

    /// Save to the current file, or ask for one. Returns false if cancelled or failed.
    fn save(&mut self) -> bool {
        match self.path.clone() {
            Some(path) => self.save_to(&path),
            None => self.save_as(),
        }
    }

    fn save_as(&mut self) -> bool {
        // A saved project starts in its own folder; a new one in Documents\Ovrtone\<name>.
        let (dir, name) = match &self.path {
            Some(path) => (
                path.parent().map(Path::to_path_buf),
                format!("{}.{}", self.project.name, ovrtone_project::EXTENSION),
            ),
            None => {
                let proposed = crate::project_folder::proposed_file(&self.project.name);
                let name = proposed
                    .file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                (proposed.parent().map(Path::to_path_buf), name)
            }
        };
        let mut save = dialog().set_title(tr("Save As")).set_file_name(name);
        if let Some(dir) = dir.filter(|d| d.is_dir()) {
            save = save.set_directory(dir);
        }
        match save.save_file()
        {
            Some(path) => self.save_to(&path),
            None => false,
        }
    }

    /// Save A Copy As…: writes the project elsewhere and keeps working in the current file.
    fn save_copy(&mut self) -> bool {
        let name = format!("{} copy.{}", self.project.name, ovrtone_project::EXTENSION);
        match dialog()
            .set_title(tr("Save A Copy As"))
            .set_file_name(name)
            .save_file()
        {
            Some(path) => self.save_copy_to(&path),
            None => false,
        }
    }

    /// Writes the project to `path` without making it the current file: the title, the unsaved
    /// state and Open Recent stay as they were.
    pub fn save_copy_to(&mut self, path: &Path) -> bool {
        self.collect_plugin_states();
        match project_io::save_project(&self.project, path) {
            Ok(path) => {
                self.ui.status = Some(trf("saved a copy: {}", &[&path.display()]));
                true
            }
            Err(e) => {
                self.ui.status = Some(trf("cannot save {}: {}", &[&path.display(), &e]));
                false
            }
        }
    }

    pub fn save_to(&mut self, path: &Path) -> bool {
        self.collect_plugin_states();
        // The file as it was before this save goes into a Backups folder beside it.
        keep_backup(path, self.ui.backups_kept);
        match project_io::save_project(&self.project, path) {
            Ok(path) => {
                if let Some(sync) = self.sync.as_mut() {
                    sync.clear_plugins_touched();
                }
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    self.project.name = stem.to_string();
                }
                self.ui.recent.add(&path);
                self.ui.thumbnail_wanted = Some((path.clone(), 3));
                self.ui.offer_new_extension = ovrtone_project::is_legacy_file(&path)
                    && self.ui.keep_old_extension.as_ref() != Some(&path);
                self.path = Some(path);
                self.saved = content(&self.project);
                self.ui.status = None;
                self.autosave.forget();
                true
            }
            Err(e) => {
                self.ui.status = Some(trf("cannot save {}: {}", &[&path.display(), &e]));
                false
            }
        }
    }

    /// Whether the project differs from the file. Plugins keep their settings to themselves, so
    /// for those it goes by whether they may have changed.
    fn has_unsaved_changes(&self) -> bool {
        content(&self.project) != self.saved
            || self.sync.as_ref().is_some_and(|s| s.plugins_touched())
    }

    /// Gets the running plugins' settings into the project, ready to save.
    fn collect_plugin_states(&mut self) {
        if let Some(sync) = self.sync.as_mut() {
            sync.collect_plugin_states(&mut self.project);
        }
    }

    /// Swaps in a project recovered after a crash: it stays unsaved until the user saves it.
    pub fn replace_recovered(&mut self, p: Project, path: Option<PathBuf>) {
        self.replace(p, path);
        self.saved = content(&Project::empty());
    }

    /// Swaps in a whole new project and resets editor state that indexes into the old one.
    fn replace(&mut self, mut p: Project, path: Option<PathBuf>) {
        if let Some(sync) = self.sync.as_mut() {
            sync.clear_plugins_touched();
        }
        // A new song revision makes `EngineSync` rebuild and swap the engine's song.
        p.song_rev = self.project.song_rev.wrapping_add(1);
        self.project = p;
        self.path = path;
        self.saved = content(&self.project);
        self.ui.reset_views();
        // Undo stays within one project.
        self.history.reset(&self.project);
        // A project is open now, so the Project Chooser has done its job.
        self.ui.chooser = None;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn project_names_must_be_able_to_be_file_names() {
        assert!(valid_project_name("Song 2"));
        for bad in ["", "  ", ".", "..", "a/b", "a\\b", "a:b", "what?", "a|b"] {
            assert!(!valid_project_name(bad), "{bad}");
        }
    }

    fn app() -> App {
        let project = Project::demo();
        App {
            autosave: crate::autosave::Autosave::default(),
            saved: content(&project),
            history: crate::undo::History::new(&project),
            project,
            ui: crate::UiState::new(),
            sync: None,
            path: None,
            native_menu: None,
            quit_asked: false,
            quitting: false,
            bench: None,
            shots: None,
            frames_since_record: 0,
            midi: None,
            midi_out: None,
            surface: None,
            surface_tried: None,
            mtc_in: None,
            chase: crate::mtc_in::Chase::default(),
            updater: crate::updater::Updater::new(),
        }
    }

    #[test]
    fn playhead_and_selection_are_not_unsaved_changes() {
        let mut p = Project::demo();
        let before = content(&p);
        p.position_beats = 12.0;
        p.selected_track = 2;
        assert_eq!(content(&p), before);
        p.tempo += 1.0;
        assert_ne!(content(&p), before);
    }

    #[test]
    fn menu_entries_and_key_commands() {
        assert_eq!(FileCmd::from_menu("Save As…"), Some(FileCmd::SaveAs));
        assert_eq!(FileCmd::from_menu("Bounce"), None);
        assert_eq!(FileCmd::from_menu("Close Project"), Some(FileCmd::Close));
        let save_as = FileCmd::SaveAs.shortcut().unwrap();
        assert!(save_as.modifiers.shift && save_as.logical_key == Key::S);
        assert!(!FileCmd::Save.shortcut().unwrap().modifiers.shift);
        assert_eq!(
            FileCmd::from_menu("Save A Copy As…"),
            Some(FileCmd::SaveCopy)
        );
        assert_eq!(FileCmd::from_menu("Revert to Saved"), Some(FileCmd::Revert));
        assert!(FileCmd::SaveCopy.shortcut().is_none() && FileCmd::OpenRecent.shortcut().is_none());
    }

    /// What the program wrote before it was renamed: a zip holding `project.json`, by hand so that
    /// it does not follow later changes to the saving code.
    fn old_project(file: &Path) {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::fs::File::create(file).unwrap());
        zip.start_file(
            ovrtone_project::PROJECT_JSON,
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(
            br#"{"format_version":2,"name":"Old Song","tempo":97.0,"tracks":[{"name":"Lead"}]}"#,
        )
        .unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn an_old_project_opens_and_saves_where_it_was_until_moved() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let file = dir.path().join("Old Song.prowave");
        old_project(&file);
        let mut a = app();
        a.open(&file, &ctx);
        assert_eq!(a.ui.status, None);
        assert_eq!(a.project.tempo, 97.0);
        assert_eq!(a.project.tracks[0].name, "Lead");
        assert_eq!(a.path.as_deref(), Some(file.as_path()));
        assert!(!a.ui.offer_new_extension, "opening does not ask");
        // Saving keeps to the file it came from, then asks.
        assert!(a.save_to(&file));
        assert_eq!(a.path.as_deref(), Some(file.as_path()));
        assert!(a.ui.offer_new_extension);
        // Asked once: no is remembered for this file.
        a.ui.offer_new_extension = false;
        a.ui.keep_old_extension = Some(file.clone());
        assert!(a.save_to(&file));
        assert!(!a.ui.offer_new_extension);
        // Moving on leaves the old file, and the new one does not ask.
        let new = dir.path().join("Old Song.ovrtone");
        assert!(a.save_to(&new));
        assert!(file.exists() && new.exists());
        assert!(!a.ui.offer_new_extension);
    }

    #[test]
    fn a_new_project_is_saved_with_the_new_extension() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = app();
        assert!(a.save_to(&dir.path().join("Fresh")));
        assert_eq!(
            a.path.as_deref(),
            Some(dir.path().join("Fresh.ovrtone").as_path())
        );
        assert!(!a.ui.offer_new_extension);
    }

    #[test]
    fn save_then_open_tracks_path_name_and_clean_state() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut a = app();
        a.project.tempo = 97.0;
        assert_ne!(content(&a.project), a.saved);

        assert!(a.save_to(&dir.path().join("Night Drive")));
        let file = dir.path().join("Night Drive.ovrtone");
        assert_eq!(a.path.as_deref(), Some(file.as_path()));
        assert_eq!(a.project.name, "Night Drive");
        assert_eq!(content(&a.project), a.saved);

        let rev = a.project.song_rev;
        a.project.tempo = 120.0;
        a.open(&file, &ctx);
        assert_eq!(a.project.tempo, 97.0);
        assert_ne!(a.project.song_rev, rev, "engine must rebuild the song");
        assert_eq!(content(&a.project), a.saved);
    }

    #[test]
    fn new_with_unsaved_changes_asks_first() {
        let ctx = egui::Context::default();

        let mut clean = app();
        let rev = clean.project.song_rev;
        clean.run_file_cmd(FileCmd::New, &ctx);
        assert!(clean.ui.ask_save.is_none(), "nothing to lose, no prompt");
        assert_ne!(clean.project.song_rev, rev, "New replaced the project");

        let mut dirty = app();
        dirty.project.tempo += 1.0;
        let tracks = dirty.project.tracks.len();
        dirty.run_file_cmd(FileCmd::New, &ctx);
        assert_eq!(dirty.ui.ask_save, Some(FileCmd::New));
        assert_eq!(
            dirty.project.tracks.len(),
            tracks,
            "not replaced until answered"
        );

        // Drawing the prompt (nobody clicks) keeps it open.
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            dirty.show_save_prompt(ui.ctx())
        });
        out.textures_delta.clear();
        assert_eq!(dirty.ui.ask_save, Some(FileCmd::New));
    }

    #[test]
    fn save_commands_never_prompt() {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let mut a = app();
        a.project.tempo += 1.0;
        a.save_to(&dir.path().join("x"));
        a.project.tempo += 1.0;
        a.run_file_cmd(FileCmd::Save, &ctx);
        assert!(a.ui.ask_save.is_none());
        assert_eq!(content(&a.project), a.saved);
    }

    #[test]
    fn open_failure_keeps_the_project_and_reports() {
        let mut a = app();
        let tracks = a.project.tracks.len();
        a.open(
            Path::new("does-not-exist.ovrtone"),
            &egui::Context::default(),
        );
        assert_eq!(a.project.tracks.len(), tracks);
        assert!(a.ui.status.is_some());
    }

    #[test]
    fn saving_and_opening_feed_open_recent_and_a_broken_file_is_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut a = app();
        assert!(a.save_to(&dir.path().join("One")));
        assert!(a.save_to(&dir.path().join("Two")));
        let (one, two) = (
            dir.path().join("One.ovrtone"),
            dir.path().join("Two.ovrtone"),
        );
        assert_eq!(a.ui.recent.paths(), [two.clone(), one.clone()]);

        a.open(&one, &ctx);
        assert_eq!(a.ui.recent.paths()[0], one);

        let bad = dir.path().join("Bad.ovrtone");
        std::fs::write(&bad, b"not a zip").unwrap();
        a.ui.recent.add(&bad);
        a.open(&bad, &ctx);
        assert!(!a.ui.recent.paths().contains(&bad));
        assert!(a.ui.status.as_deref().unwrap().starts_with("cannot open"));
    }

    #[test]
    fn open_recent_asks_before_discarding_changes() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut a = app();
        assert!(a.save_to(&dir.path().join("Saved")));
        let file = dir.path().join("Saved.ovrtone");

        a.project.tempo = 61.0;
        a.ui.recent_pick = Some(file.clone());
        a.run_file_cmd(FileCmd::OpenRecent, &ctx);
        assert_eq!(a.ui.ask_save, Some(FileCmd::OpenRecent));
        assert_eq!(a.project.tempo, 61.0, "not replaced until answered");

        // "Don't Save": the pending project opens and the pick is used up.
        a.ui.ask_save = None;
        a.do_file_cmd(FileCmd::OpenRecent, &ctx);
        assert_ne!(a.project.tempo, 61.0);
        assert!(a.ui.recent_pick.is_none());
    }

    #[test]
    fn revert_needs_a_saved_file_and_a_confirmed_change() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut a = app();
        a.run_file_cmd(FileCmd::Revert, &ctx);
        assert!(!a.ui.ask_revert);
        assert!(a.ui.status.as_deref().unwrap().contains("not been saved"));

        assert!(a.save_to(&dir.path().join("Song")));
        a.run_file_cmd(FileCmd::Revert, &ctx);
        assert!(!a.ui.ask_revert);
        assert!(a
            .ui
            .status
            .as_deref()
            .unwrap()
            .contains("no unsaved changes"));

        let saved_tempo = a.project.tempo;
        a.project.tempo += 10.0;
        a.run_file_cmd(FileCmd::Revert, &ctx);
        assert!(a.ui.ask_revert, "asks before throwing changes away");
        assert_eq!(a.project.tempo, saved_tempo + 10.0);

        a.revert(&ctx);
        assert_eq!(a.project.tempo, saved_tempo);
        assert_eq!(content(&a.project), a.saved);
    }

    #[test]
    fn save_a_copy_leaves_the_current_project_alone() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = app();
        assert!(a.save_to(&dir.path().join("Original")));
        let (path, name) = (a.path.clone(), a.project.name.clone());
        let recent = a.ui.recent.paths().to_vec();

        a.project.tempo += 5.0;
        assert!(a.save_copy_to(&dir.path().join("Backup")));
        let copy = dir.path().join("Backup.ovrtone");
        assert!(copy.is_file());
        assert_eq!((a.path.clone(), a.project.name.clone()), (path, name));
        assert_ne!(content(&a.project), a.saved, "still has unsaved changes");
        assert_eq!(a.ui.recent.paths(), recent.as_slice());

        // The copy holds the current state, tempo included.
        let (opened, _) = project_io::open_project(&copy, &egui::Context::default()).unwrap();
        assert_eq!(opened.tempo, a.project.tempo);
        assert_eq!(opened.name, "Backup");
    }

    #[test]
    fn an_edited_time_signature_is_saved_and_restored() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut a = app();
        assert!(a
            .project
            .step_signature(crate::model::SigPart::Numerator, 2));
        assert!(a
            .project
            .step_signature(crate::model::SigPart::Denominator, 1));
        assert_eq!(a.project.sig, (6, 8));
        assert_ne!(
            content(&a.project),
            a.saved,
            "a new signature is an unsaved change"
        );

        assert!(a.save_to(&dir.path().join("Waltz")));
        a.project.sig = (4, 4);
        a.open(&dir.path().join("Waltz.ovrtone"), &ctx);
        assert_eq!(a.project.sig, (6, 8));
        assert_eq!(a.project.bar_len(0), 3.0);
    }
}
