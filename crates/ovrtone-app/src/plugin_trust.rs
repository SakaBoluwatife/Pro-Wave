//! Which plugin files a project may load: only those the user's own scan found. A project names its
//! plugins by file path, so a project from someone else could otherwise make Ovrtone load any file,
//! even one packed in the project's own media. A plugin that is not in the list is shown as missing
//! and keeps its settings, for when it is installed. The sampler's "plugins" are sample files, not
//! code, and need no check.

use ovrtone_host::{PluginFormat, PluginInfo};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

/// Why a plugin of a project was not loaded.
pub const NOT_SCANNED: &str =
    "Not in your plugins: install it, then press Rescan on the Plugins tab.";
/// Why a plugin file inside a project's media was not loaded.
pub const IN_MEDIA: &str = "This plugin file came inside the project, so it is not loaded.";

/// Every plugin the scan found, by id and file. `None` until the first scan result (or cache).
static KNOWN: RwLock<Option<Vec<(String, PathBuf)>>> = RwLock::new(None);
/// Goes up each time the list changes, so that plugins found missing are tried again.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Takes in what the scan found: all of it, both formats of a plugin installed as both.
pub fn set_known(all: &[PluginInfo]) {
    let list = all.iter().map(|i| (i.id.clone(), i.path.clone())).collect();
    if let Ok(mut known) = KNOWN.write() {
        *known = Some(list);
    }
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// Changes each time the list of scanned plugins changes.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

/// Paths compared as Windows does: case and slash direction do not matter.
fn same_file(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        let key = |p: &Path| p.to_string_lossy().replace('/', "\\").to_lowercase();
        key(a) == key(b)
    } else {
        a == b
    }
}

fn inside(path: &Path, folder: &Path) -> bool {
    if cfg!(windows) {
        let key = |p: &Path| p.to_string_lossy().replace('/', "\\").to_lowercase();
        let (p, f) = (key(path), key(folder));
        p.starts_with(&f)
    } else {
        path.starts_with(folder)
    }
}

/// The file to load for plugin `id`, stored with `path`. Matched by id first: the stored file if
/// the scan found the plugin there, else where the scan found it on this computer (installed
/// elsewhere). Then by path, for a file the scan found under another id. Anything else is refused.
pub fn allowed(path: &Path, id: &str) -> Result<PathBuf, &'static str> {
    let format = PluginFormat::of(path);
    if format == PluginFormat::Sampler {
        return Ok(path.to_path_buf());
    }
    if inside(path, &crate::project_io::media_root()) {
        return Err(IN_MEDIA);
    }
    let Ok(known) = KNOWN.read() else {
        return Err(NOT_SCANNED);
    };
    // No scan has run in this program (measuring, tests): nothing to go by.
    let Some(list) = known.as_ref() else {
        return Ok(path.to_path_buf());
    };
    allowed_in(list, path, id, format)
}

fn allowed_in(
    list: &[(String, PathBuf)],
    path: &Path,
    id: &str,
    format: PluginFormat,
) -> Result<PathBuf, &'static str> {
    let by_id: Vec<&PathBuf> = list
        .iter()
        .filter(|(i, p)| i == id && PluginFormat::of(p) == format)
        .map(|(_, p)| p)
        .collect();
    if let Some(same) = by_id.iter().find(|p| same_file(p, path)) {
        return Ok((*same).clone());
    }
    if let Some(elsewhere) = by_id.first() {
        return Ok((*elsewhere).clone());
    }
    if list.iter().any(|(_, p)| same_file(p, path)) {
        return Ok(path.to_path_buf());
    }
    Err(NOT_SCANNED)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn only_scanned_files_load() {
        let list = vec![
            ("com.a.comp".to_string(), PathBuf::from("C:/Plugins/A.clap")),
            ("com.b.eq".to_string(), PathBuf::from("D:/Other/B.clap")),
        ];
        let clap = PluginFormat::Clap;
        let a = Path::new("C:/Plugins/A.clap");
        assert_eq!(allowed_in(&list, a, "com.a.comp", clap).unwrap(), a);
        // Installed somewhere else on this computer: the scanned file is used.
        let moved = allowed_in(&list, Path::new("E:/Old/B.clap"), "com.b.eq", clap);
        assert_eq!(moved.unwrap(), PathBuf::from("D:/Other/B.clap"));
        // A file nobody scanned, under an id that is not installed.
        let bad = allowed_in(&list, Path::new("C:/Temp/evil.clap"), "com.x", clap);
        assert_eq!(bad, Err(NOT_SCANNED));
    }
}
