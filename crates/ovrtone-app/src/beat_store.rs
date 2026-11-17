//! File > Export > For Beat Store: what a beat store sells, in one go. An untagged WAV and an MP3 of the
//! whole beat and a ZIP of its stems, named after the beat with its tempo and key, in a folder of its
//! own under Music\Ovrtone Beat Store. When they are made, the folder opens and so does the store's
//! site (Trakroom), to drag them in. Nothing is sent anywhere by Ovrtone itself.

use crate::bounce::{self, After, BitDepth, FileFormat, Render};
use crate::model::Project;
use crate::UiState;
use std::path::{Path, PathBuf};

/// The store the list starts with.
const FIRST_STORE: (&str, &str) = ("Trakroom", "https://trakroom.com");
/// The user's stores, one a line: name, address and whether it opens, split by tabs.
const STORES_FILE: &str = "beat-stores";

/// A store a seller uploads beats to.
#[derive(Clone, Debug, PartialEq)]
pub struct Store {
    pub name: String,
    pub url: String,
    /// Opened when an export is made.
    pub open: bool,
}

/// The export window: the stores and a new one being typed in.
#[derive(Default)]
pub struct Dialog {
    stores: Vec<Store>,
    new_name: String,
    new_url: String,
}

fn stores_path() -> Option<PathBuf> {
    crate::recent_files::config_dir().map(|d| d.join(STORES_FILE))
}

/// The saved stores, or Trakroom alone the first time.
pub fn load_stores() -> Vec<Store> {
    let text = stores_path().and_then(|f| std::fs::read_to_string(f).ok());
    let Some(text) = text else {
        return vec![Store {
            name: FIRST_STORE.0.to_string(),
            url: FIRST_STORE.1.to_string(),
            open: true,
        }];
    };
    parse_stores(&text)
}

fn parse_stores(text: &str) -> Vec<Store> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let name = parts.next()?.trim().to_string();
            let url = parts.next()?.trim().to_string();
            let open = parts.next().is_none_or(|o| o.trim() != "0");
            (!name.is_empty() && is_web_address(&url)).then_some(Store { name, url, open })
        })
        .collect()
}

fn save_stores(stores: &[Store]) {
    let Some(file) = stores_path() else { return };
    let text: String = stores
        .iter()
        .map(|s| format!("{}\t{}\t{}\n", s.name, s.url, u8::from(s.open)))
        .collect();
    let saved = file
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&file, text));
    if let Err(e) = saved {
        tracing::warn!("could not save the beat stores: {e}");
    }
}

/// Only web addresses are opened: anything else could start a program.
fn is_web_address(url: &str) -> bool {
    let lower = url.to_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://"))
        && !url.chars().any(|c| c.is_whitespace() || c == '"')
}

/// A typed address with `https://` added when it has none.
fn web_address(typed: &str) -> String {
    let typed = typed.trim();
    let lower = typed.to_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") {
        typed.to_string()
    } else {
        format!("https://{typed}")
    }
}

/// File > Export > For Beat Store: opens the window that asks which stores to open.
pub fn ask(s: &mut UiState) {
    s.beat_store_dialog = Some(Dialog {
        stores: load_stores(),
        ..Dialog::default()
    });
}

/// The export window, while it is open. Once a frame.
pub fn dialog(ctx: &egui::Context, p: &Project, s: &mut UiState) {
    let Some(d) = s.beat_store_dialog.as_mut() else { return };
    let (mut export, mut close, mut changed) = (false, false, false);
    let mut remove = None;
    egui::Window::new("Export for Beat Store")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.label("A WAV, an MP3 and the stems in a ZIP. Then these stores open, to upload them:");
            ui.add_space(4.0);
            for (i, store) in d.stores.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    changed |= ui.checkbox(&mut store.open, &store.name).changed();
                    ui.weak(&store.url);
                    if ui.small_button("Remove").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if d.stores.is_empty() {
                ui.weak("No stores yet: only the folder opens.");
            }
            ui.separator();
            ui.label("Add a store");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut d.new_name).hint_text("Name").desired_width(110.0));
                ui.add(
                    egui::TextEdit::singleline(&mut d.new_url)
                        .hint_text("beatstars.com")
                        .desired_width(180.0),
                );
                let url = web_address(&d.new_url);
                let ok = !d.new_name.trim().is_empty() && !d.new_url.trim().is_empty() && is_web_address(&url);
                if ui.add_enabled(ok, egui::Button::new("Add")).clicked() {
                    d.stores.push(Store {
                        name: d.new_name.trim().to_string(),
                        url,
                        open: true,
                    });
                    d.new_name.clear();
                    d.new_url.clear();
                    changed = true;
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                export = ui.button("Export").clicked();
                close = ui.button("Cancel").clicked();
            });
        });
    if let Some(i) = remove {
        d.stores.remove(i);
        changed = true;
    }
    if changed {
        save_stores(&d.stores);
    }
    if export {
        s.beat_store_dialog = None;
        start(p, s);
    } else if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        s.beat_store_dialog = None;
    }
}

/// An export under way: what to do when its files are made.
pub struct Pending {
    folder: PathBuf,
    stems: PathBuf,
    base: String,
    info: String,
}

/// Starts the export of `p`.
pub fn start(p: &Project, s: &mut UiState) {
    if s.bounce.busy() || s.beat_store.is_some() {
        s.status = Some("Wait for the export under way to finish".to_string());
        return;
    }
    let end = bounce::project_end(p);
    if end <= 0.0 {
        s.status = Some("Nothing to export yet".to_string());
        return;
    }
    let key = crate::score::key_label(p.key);
    let bpm = p.tempo.round() as i32;
    let base = bounce::file_safe(&format!("{} ({bpm} BPM, {key})", p.name));
    let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from) else {
        s.status = Some("No user folder to export to".to_string());
        return;
    };
    let folder = home.join("Music").join("Ovrtone Beat Store").join(&base);
    let stems = folder.join("stems (being made)");
    if let Err(e) = std::fs::create_dir_all(&stems) {
        s.status = Some(format!("Cannot make the folder: {e}"));
        return;
    }
    let whole = |format: FileFormat| Render {
        path: folder.join(format!("{base}.{}", format.extension())),
        format,
        from_beat: 0.0,
        to_beat: end,
        depth: BitDepth::Int24,
        out_rate: 0,
        solo: None,
        after: After::Report,
        mono: false,
        normalize: None,
    };
    let mut renders = vec![whole(FileFormat::Wav), whole(FileFormat::Mp3)];
    renders.extend(bounce::stems(p, &stems, FileFormat::Wav, BitDepth::Int24, (0.0, end)));
    let seconds = p.tempo_map().seconds_at(end);
    let mut info = format!(
        "Title: {}\r\nBPM: {bpm}\r\nKey: {key}\r\nLength: {}:{:02}\r\n",
        p.name,
        (seconds / 60.0) as u32,
        (seconds % 60.0) as u32,
    );
    if !p.engineer.trim().is_empty() {
        info.push_str(&format!("Mixed by: {}\r\n", p.engineer.trim()));
    }
    s.bounce.enqueue(renders);
    s.beat_store = Some(Pending {
        folder,
        stems,
        base,
        info,
    });
    s.status = Some("Exporting for your beat store: WAV, MP3 and stems".to_string());
}

/// Finishes an export once its files are made: the stems into a ZIP, a note of the beat's tempo and
/// key, and the folder and the store opened. Once a frame.
pub fn poll(s: &mut UiState) {
    if s.beat_store.is_none() || s.bounce.busy() {
        return;
    }
    let Some(job) = s.beat_store.take() else { return };
    let stores: Vec<Store> = load_stores().into_iter().filter(|s| s.open).collect();
    let names: Vec<&str> = stores.iter().map(|s| s.name.as_str()).collect();
    let upload = if names.is_empty() {
        String::new()
    } else {
        format!(" Drag them into {}.", names.join(", "))
    };
    let zip = job.folder.join(format!("{} Stems.zip", job.base));
    let zipped = zip_folder(&job.stems, &zip);
    let _ = std::fs::remove_dir_all(&job.stems);
    let _ = std::fs::write(job.folder.join("Beat info.txt"), &job.info);
    s.status = Some(match zipped {
        Ok(0) => "Exported the WAV and MP3 (no tracks for stems)".to_string(),
        Ok(n) => format!("Exported for your beat store: WAV, MP3 and {n} stems.{upload}"),
        Err(e) => format!("Exported the WAV and MP3; the stems ZIP failed: {e}"),
    });
    let _ = std::process::Command::new("explorer").arg(&job.folder).spawn();
    for store in stores.iter().filter(|s| is_web_address(&s.url)) {
        let _ = std::process::Command::new("explorer").arg(&store.url).spawn();
    }
}

/// Every file in `folder` into the ZIP `out`. Returns how many.
fn zip_folder(folder: &Path, out: &Path) -> Result<usize, String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(folder)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    if files.is_empty() {
        return Ok(0);
    }
    let file = std::fs::File::create(out).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true);
    for path in &files {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        zip.start_file(name, options).map_err(|e| e.to_string())?;
        let mut input = std::fs::File::open(path).map_err(|e| e.to_string())?;
        std::io::copy(&mut input, &mut zip).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(files.len())
}
