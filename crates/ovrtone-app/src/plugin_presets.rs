//! Plugin presets: a plugin's whole state in a file of its own, in the config folder under the
//! plugin's id. Any plugin that can save its state can have them, whatever its format.
use std::path::{Path, PathBuf};

const EXTENSION: &str = "pwpreset";

/// What a plugin slot's menu asked for; `App` carries it out, as the plugin lives in the engine sync.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PresetCmd {
    Save {
        uid: u64,
        file: PathBuf,
    },
    Load {
        uid: u64,
        file: PathBuf,
    },
    /// Switches the plugin to its other set of settings (A/B compare).
    Compare {
        uid: u64,
    },
    /// Copies the settings it has now over the other set.
    CopyCompare {
        uid: u64,
    },
}

/// What a channel strip's Setting menu asked for that needs the running plugins.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StripCmd {
    Copy { track: usize },
    Save { track: usize, file: PathBuf },
}

const STRIP_EXTENSION: &str = "pwstrip";

/// Where channel strip settings are kept.
fn strip_folder() -> Option<PathBuf> {
    Some(crate::recent_files::config_dir()?.join("channel strips"))
}

fn list_strips_in(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<(String, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some(STRIP_EXTENSION))
        .filter_map(|p| Some((p.file_stem()?.to_str()?.to_string(), p)))
        .collect();
    out.sort();
    out
}

/// The saved channel strip settings: name and file, by name.
pub fn list_strips() -> Vec<(String, PathBuf)> {
    if cfg!(test) {
        return Vec::new();
    }
    strip_folder().map_or_else(Vec::new, |dir| list_strips_in(&dir))
}

/// Asks where to save a channel strip setting. `None` if the user changed their mind.
pub fn ask_where_strip(name: &str) -> Option<PathBuf> {
    let dir = strip_folder()?;
    std::fs::create_dir_all(&dir).ok()?;
    let mut file = rfd::FileDialog::new()
        .set_directory(&dir)
        .add_filter("Ovrtone channel strip", &[STRIP_EXTENSION])
        .set_file_name(format!("{name}.{STRIP_EXTENSION}"))
        .save_file()?;
    file.set_extension(STRIP_EXTENSION);
    Some(file)
}

/// A name that is safe as a folder name, from a plugin's id.
pub fn folder_name(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Where the presets of plugin `id` are kept.
pub fn folder_of(id: &str) -> Option<PathBuf> {
    Some(
        crate::recent_files::config_dir()?
            .join("presets")
            .join(folder_name(id)),
    )
}

fn list_in(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<(String, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some(EXTENSION))
        .filter_map(|p| Some((p.file_stem()?.to_str()?.to_string(), p)))
        .collect();
    out.sort();
    out
}

/// The presets of plugin `id`: name and file, by name.
pub fn list(id: &str) -> Vec<(String, PathBuf)> {
    if cfg!(test) {
        return Vec::new();
    }
    folder_of(id).map_or_else(Vec::new, |dir| list_in(&dir))
}

/// Every saved preset in the user's presets folder, as (`plugin_folder_name`, `preset_name`,
/// `file`). The plugin folder name is a safe form of the plugin id (what [`folder_of`] uses).
pub fn list_all() -> Vec<(String, String, PathBuf)> {
    let Some(root) = crate::recent_files::config_dir().map(|d| d.join("presets")) else {
        return Vec::new();
    };
    let mut out: Vec<(String, String, PathBuf)> = Vec::new();
    let Ok(dirs) = std::fs::read_dir(&root) else {
        return out;
    };
    for dir_entry in dirs.flatten() {
        let folder = dir_entry.path();
        if !folder.is_dir() {
            continue;
        }
        let folder_name = folder
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
            .unwrap_or_default();
        for (preset, path) in list_in(&folder) {
            out.push((folder_name.clone(), preset, path));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    out
}

/// Asks where to save a preset of plugin `id`. `None` if the user changed their mind.
pub fn ask_where(id: &str, name: &str) -> Option<PathBuf> {
    let dir = folder_of(id)?;
    std::fs::create_dir_all(&dir).ok()?;
    let mut file = rfd::FileDialog::new()
        .set_directory(&dir)
        .add_filter("Ovrtone preset", &[EXTENSION])
        .set_file_name(format!("{name}.{EXTENSION}"))
        .save_file()?;
    file.set_extension(EXTENSION);
    Some(file)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_listed_by_name_and_ids_make_safe_folders() {
        assert_eq!(folder_name("com.fab/filter:Pro C"), "com.fab_filter_Pro_C");
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Warm.pwpreset"), b"a").unwrap();
        std::fs::write(dir.path().join("Air.pwpreset"), b"b").unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"c").unwrap();
        let names: Vec<String> = list_in(dir.path()).into_iter().map(|p| p.0).collect();
        assert_eq!(names, ["Air", "Warm"]);
        assert!(list_in(&dir.path().join("gone")).is_empty());
    }
}
