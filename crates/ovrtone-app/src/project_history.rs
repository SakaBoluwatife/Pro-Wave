//! Project history (File > Project History…): the backups kept beside a project each time it is
//! saved, newest first, any of which can be opened in its place. And when a project will not open
//! (a damaged or half-copied file), its newest backup is offered instead.

use std::path::{Path, PathBuf};

use crate::App;

/// A backup: its file, when it was made (seconds since 1970) and its size.
pub struct Backup {
    pub path: PathBuf,
    pub made: u64,
    pub bytes: u64,
}

/// The backups of the project at `path`, newest first.
pub fn backups_of(path: &Path) -> Vec<Backup> {
    let (Some(dir), Some(stem)) = (path.parent(), path.file_stem().and_then(|s| s.to_str())) else {
        return Vec::new();
    };
    let prefix = format!("{stem} backup ");
    let mut found: Vec<Backup> = std::fs::read_dir(dir.join("Backups"))
        .map(|it| {
            it.flatten()
                .filter_map(|e| {
                    let path = e.path();
                    let name = path.file_stem()?.to_str()?.to_string();
                    let made = name.strip_prefix(&prefix)?.parse().ok()?;
                    let bytes = e.metadata().map_or(0, |m| m.len());
                    Some(Backup { path, made, bytes })
                })
                .collect()
        })
        .unwrap_or_default();
    found.sort_by(|a, b| b.made.cmp(&a.made));
    found
}

/// A time as the user's clock would show it, roughly: date and time in UTC.
fn when(secs: u64) -> String {
    let days = secs / 86_400;
    let rest = secs % 86_400;
    // Civil date from days since 1970 (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y}-{m:02}-{d:02} {:02}:{:02} UTC", rest / 3600, rest % 3600 / 60)
}

impl App {
    /// The Project History window and the damaged-file offer. Call once per frame.
    pub fn show_history(&mut self, ctx: &egui::Context) {
        self.show_damaged(ctx);
        if !self.ui.history_open {
            return;
        }
        let mut open = true;
        let mut restore: Option<PathBuf> = None;
        let path = self.path.clone();
        egui::Window::new("Project History")
            .id(egui::Id::unique("project_history"))
            .open(&mut open)
            .default_width(420.0)
            .show(ctx, |ui| {
                let Some(path) = path.as_ref() else {
                    ui.label("The project has not been saved yet: it has no history.");
                    return;
                };
                let backups = backups_of(path);
                ui.label(format!(
                    "Each save keeps the version before it in the Backups folder beside the project ({} kept; \
                     Settings > General).",
                    self.ui.backups_kept
                ));
                ui.separator();
                if backups.is_empty() {
                    ui.label("No backups yet: they are made from the second save on.");
                    return;
                }
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    egui::Grid::new(egui::Id::unique("history_grid"))
                        .num_columns(3)
                        .striped(true)
                        .show(ui, |ui| {
                            for b in &backups {
                                ui.label(when(b.made));
                                ui.label(format!("{:.1} MB", b.bytes as f64 / 1_048_576.0));
                                if ui
                                    .button("Open This Version")
                                    .on_hover_text(
                                        "Opens it in place of what is open now. Save to keep it; the version you \
                                         replace is kept as a backup when you do.",
                                    )
                                    .clicked()
                                {
                                    restore = Some(b.path.clone());
                                }
                                ui.end_row();
                            }
                        });
                });
            });
        self.ui.history_open = open;
        if let (Some(backup), Some(original)) = (restore, self.path.clone()) {
            self.restore_backup(&backup, &original, ctx);
            self.ui.history_open = false;
        }
    }

    /// Opens `backup` as the project at `original`, unsaved, so a save puts it back in place.
    fn restore_backup(&mut self, backup: &Path, original: &Path, ctx: &egui::Context) {
        match crate::project_io::open_project(backup, ctx) {
            Ok((p, _)) => {
                self.replace_recovered(p, Some(original.to_path_buf()));
                self.ui.opening_plugins = Some(0);
                self.ui.status = Some("An earlier version is open: save to keep it".to_string());
            }
            Err(e) => self.ui.status = Some(format!("cannot open that backup: {e}")),
        }
    }

    /// A project that would not open: says why, and offers its newest backup.
    fn show_damaged(&mut self, ctx: &egui::Context) {
        let Some((path, backup, why)) = self.ui.damaged.clone() else {
            return;
        };
        let mut answer = None;
        egui::Window::new("This project could not be opened")
            .id(egui::Id::unique("damaged_project"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(format!("{}\n\n{why}", path.display()));
                ui.add_space(4.0);
                let made = backups_of(&path)
                    .into_iter()
                    .find(|b| b.path == backup)
                    .map_or_else(String::new, |b| when(b.made));
                ui.label(format!(
                    "The file may be damaged or only partly copied. Its newest backup, from {made}, can be opened \
                     instead (save to put it back in place)."
                ));
                ui.horizontal(|ui| {
                    if ui.button("Open the Backup").clicked() {
                        answer = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        answer = Some(false);
                    }
                });
            });
        match answer {
            Some(true) => {
                self.ui.damaged = None;
                self.restore_backup(&backup, &path, ctx);
            }
            Some(false) => self.ui.damaged = None,
            None => {}
        }
    }
}
