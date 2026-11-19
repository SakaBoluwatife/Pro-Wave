//! Where a project keeps what it makes, as Logic keeps a project's files together:
//!
//! ```text
//! Documents\Ovrtone\<Project name>\<Project name>.ovrtone
//! Documents\Ovrtone\<Project name>\Audio Files     recordings, captured audio
//! Documents\Ovrtone\<Project name>\Bounces         File > Export / Bounce, Bounce in Place, freezes
//! Documents\Ovrtone\<Project name>\Stems           All Tracks as Audio Files, Stem Splitter
//! Documents\Ovrtone\<Project name>\Beat Store      File > Export > For Beat Store
//! ```
//!
//! A project that was saved anywhere uses the folder of its file. One that was never saved uses
//! `Documents\Ovrtone\<name>` (made when first needed). The app tells this module which project
//! is open once a frame ([`set`]), so the places that write files only ask for a folder.
//! Caches (peaks, stretch, flex, autosave, settings) are not kept here.
use std::path::{Path, PathBuf};
use std::sync::Mutex;

struct Current {
    /// The `.ovrtone` file, when the project has been saved or was opened.
    file: Option<PathBuf>,
    name: String,
}

static CURRENT: Mutex<Current> = Mutex::new(Current {
    file: None,
    name: String::new(),
});

/// Tells it which project is open: its file (if it has one) and its name.
pub fn set(file: Option<&Path>, name: &str) {
    if let Ok(mut c) = CURRENT.lock() {
        if c.file.as_deref() != file || c.name != name {
            c.file = file.map(Path::to_path_buf);
            c.name = name.to_string();
        }
    }
}

/// The user's Documents folder.
pub fn documents_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(|home| PathBuf::from(home).join("Documents"))
        .unwrap_or_else(std::env::temp_dir)
}

/// A name that can be a folder's and a file's: no characters Windows refuses; "Untitled" if
/// nothing is left.
pub fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_end_matches('.').trim().to_string();
    if cleaned.is_empty() {
        "Untitled".to_string()
    } else {
        cleaned
    }
}

/// `Documents\Ovrtone\<name>`: the folder of a project that has no file yet. Not made.
fn default_dir(name: &str) -> PathBuf {
    if cfg!(test) {
        return std::env::temp_dir()
            .join("ovrtone-test-projects")
            .join(safe_name(name));
    }
    documents_dir().join("Ovrtone").join(safe_name(name))
}

/// The file a never-saved project proposes: `Documents\Ovrtone\<name>\<name>.ovrtone`. The
/// folder is made.
pub fn proposed_file(name: &str) -> PathBuf {
    let dir = default_dir(name);
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!(
        "{}.{}",
        safe_name(name),
        ovrtone_project::EXTENSION
    ))
}

/// The folder the project is kept in: that of its file, else the one proposed for its name. Not
/// made (see [`subdir`]).
pub fn project_dir() -> PathBuf {
    let Ok(c) = CURRENT.lock() else {
        return default_dir("Untitled");
    };
    match c.file.as_deref().and_then(Path::parent) {
        Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
        _ => default_dir(&c.name),
    }
}

/// A folder of the project (`"Audio Files"`, `"Bounces"`, `"Stems"`, `"Beat Store"`), made.
pub fn subdir(name: &str) -> PathBuf {
    let dir = project_dir().join(name);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_project_uses_its_own_folder_and_a_new_one_the_documents_folder() {
        set(Some(Path::new("D:\\Songs\\Night\\Night.ovrtone")), "Night");
        assert_eq!(project_dir(), PathBuf::from("D:\\Songs\\Night"));
        set(None, "A: B/C");
        assert!(project_dir().ends_with("A_ B_C"));
        set(None, "  ");
        assert!(project_dir().ends_with("Untitled"));
        assert_eq!(safe_name("x?"), "x_");
    }
}
