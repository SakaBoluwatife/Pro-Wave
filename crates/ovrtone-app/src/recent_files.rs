//! File > Open Recent: the projects last opened or saved, newest first. Kept in a plain text
//! file (one path per line) in the user's config folder, so it survives restarts.
use std::path::{Path, PathBuf};

/// How many projects the menu remembers.
const MAX: usize = 10;

/// `<config folder>/ovrtone/recent.txt`.
fn default_file() -> Option<PathBuf> {
    Some(config_dir()?.join("recent.txt"))
}

/// `<config folder>/ovrtone`: %APPDATA% on Windows, ~/Library/Application Support on macOS,
/// $XDG_CONFIG_HOME or ~/.config elsewhere. `None` when no home folder is known.
pub(crate) fn config_dir() -> Option<PathBuf> {
    Some(config_base()?.join(CONFIG_FOLDER))
}

/// The settings folder's name.
const CONFIG_FOLDER: &str = "ovrtone";
/// Its name before the program was renamed. Never deleted: the old version may still be in use.
const OLD_CONFIG_FOLDER: &str = "prowave";
/// Rebuilt when needed and large (hundreds of MB), so not worth a second copy. Projects that point
/// into the old folder still find them there.
const NOT_MIGRATED: [&str; 2] = ["peak-cache", "flex-cache"];

/// The first time this version runs, brings over the settings of the old one, once.
pub(crate) fn migrate_old_settings() {
    let Some(base) = config_base() else {
        return;
    };
    if let Err(e) = migrate_in(&base) {
        tracing::warn!("could not bring over the old settings: {e}");
    }
}

/// Copies the old settings folder under `base` to the new one, if there is an old one and no new
/// one. It appears under its final name only when complete, so a copy cut short starts over next
/// time. True if it copied.
fn migrate_in(base: &Path) -> std::io::Result<bool> {
    let (old, new) = (base.join(OLD_CONFIG_FOLDER), base.join(CONFIG_FOLDER));
    if new.exists() || !old.is_dir() {
        return Ok(false);
    }
    let part = base.join(format!("{CONFIG_FOLDER}.migrating"));
    if part.exists() {
        std::fs::remove_dir_all(&part)?;
    }
    copy_settings(&old, &part, true)?;
    std::fs::rename(&part, &new)?;
    Ok(true)
}

fn copy_settings(from: &Path, to: &Path, top: bool) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let name = entry.file_name();
        let target = to.join(&name);
        if kind.is_dir() {
            if !(top && NOT_MIGRATED.iter().any(|skip| name == *skip)) {
                copy_settings(&entry.path(), &target, false)?;
            }
        } else if kind.is_file() {
            // One unreadable file, say held by the old version, should not cost all the others.
            if let Err(e) = std::fs::copy(entry.path(), &target) {
                tracing::warn!("could not bring over {}: {e}", entry.path().display());
            }
        }
    }
    Ok(())
}

/// The folder holding every program's settings.
fn config_base() -> Option<PathBuf> {
    let var = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let base = if cfg!(windows) {
        var("APPDATA")?
    } else if cfg!(target_os = "macos") {
        var("HOME")?.join("Library").join("Application Support")
    } else {
        var("XDG_CONFIG_HOME").or_else(|| var("HOME").map(|h| h.join(".config")))?
    };
    Some(base)
}

/// The recent-projects list. `Recent::default()` is an in-memory list that never touches disk
/// (used by tests); [`Recent::load`] is the real, persistent one.
#[derive(Default)]
pub struct Recent {
    paths: Vec<PathBuf>,
    file: Option<PathBuf>,
}

impl Recent {
    /// Reads the saved list, leaving out projects that no longer exist.
    pub fn load() -> Self {
        Self::from_file(default_file())
    }

    fn from_file(file: Option<PathBuf>) -> Self {
        let text = file
            .as_deref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .unwrap_or_default();
        let paths = text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(PathBuf::from)
            .filter(|p| p.is_file())
            .take(MAX)
            .collect();
        Self { paths, file }
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Moves `path` to the top (adding it if new) and saves.
    pub fn add(&mut self, path: &Path) {
        self.paths.retain(|p| p != path);
        self.paths.insert(0, path.to_path_buf());
        self.paths.truncate(MAX);
        self.save();
    }

    /// Drops `path`, e.g. after it turned out to be unreadable.
    pub fn remove(&mut self, path: &Path) {
        self.paths.retain(|p| p != path);
        self.save();
    }

    pub fn clear(&mut self) {
        self.paths.clear();
        self.save();
    }

    /// Best effort: a list that cannot be saved is not worth interrupting the user for.
    fn save(&self) {
        let Some(file) = &self.file else { return };
        let text: String = self
            .paths
            .iter()
            .filter_map(|p| p.to_str())
            .map(|p| format!("{p}\n"))
            .collect();
        let result = file
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(file, text));
        if let Err(e) = result {
            tracing::warn!("could not save the recent projects list: {e}");
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn touch(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, b"x").unwrap();
        p
    }

    fn old_settings(base: &Path) -> PathBuf {
        let old = base.join(OLD_CONFIG_FOLDER);
        std::fs::create_dir_all(old.join("autosave")).unwrap();
        std::fs::create_dir_all(old.join("peak-cache")).unwrap();
        std::fs::create_dir_all(old.join("flex-cache")).unwrap();
        std::fs::write(old.join("browser-roots"), "D:\\Samples").unwrap();
        std::fs::write(old.join("plugin-cache"), "version\t2").unwrap();
        std::fs::write(old.join("autosave").join("copy.prowave"), "work").unwrap();
        std::fs::write(old.join("peak-cache").join("a"), "big").unwrap();
        std::fs::write(old.join("flex-cache").join("b"), "big").unwrap();
        old
    }

    #[test]
    fn old_settings_are_copied_once_and_the_old_folder_stays() {
        let dir = tempfile::tempdir().unwrap();
        let old = old_settings(dir.path());
        assert!(migrate_in(dir.path()).unwrap());
        let new = dir.path().join(CONFIG_FOLDER);
        let read = |p: PathBuf| std::fs::read_to_string(p).unwrap();
        assert_eq!(read(new.join("browser-roots")), "D:\\Samples");
        assert_eq!(read(new.join("plugin-cache")), "version\t2");
        assert_eq!(read(new.join("autosave").join("copy.prowave")), "work");
        assert!(!new.join("peak-cache").exists() && !new.join("flex-cache").exists());
        assert!(!dir.path().join("ovrtone.migrating").exists());
        // The old folder is all still there.
        assert_eq!(read(old.join("browser-roots")), "D:\\Samples");
        assert_eq!(read(old.join("peak-cache").join("a")), "big");
        // Once only: what changes later on either side is left alone.
        std::fs::write(old.join("browser-roots"), "changed in the old version").unwrap();
        std::fs::write(new.join("plugin-cache"), "changed in the new version").unwrap();
        assert!(!migrate_in(dir.path()).unwrap());
        assert_eq!(read(new.join("browser-roots")), "D:\\Samples");
        assert_eq!(read(new.join("plugin-cache")), "changed in the new version");
    }

    #[test]
    fn nothing_is_migrated_without_an_old_folder_or_over_a_new_one() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!migrate_in(dir.path()).unwrap());
        assert!(!dir.path().join(CONFIG_FOLDER).exists());
        old_settings(dir.path());
        std::fs::create_dir_all(dir.path().join(CONFIG_FOLDER)).unwrap();
        assert!(!migrate_in(dir.path()).unwrap());
        assert!(!dir
            .path()
            .join(CONFIG_FOLDER)
            .join("browser-roots")
            .exists());
    }

    #[test]
    fn a_copy_cut_short_starts_over() {
        let dir = tempfile::tempdir().unwrap();
        old_settings(dir.path());
        let part = dir.path().join("ovrtone.migrating");
        std::fs::create_dir_all(&part).unwrap();
        std::fs::write(part.join("half"), "x").unwrap();
        assert!(migrate_in(dir.path()).unwrap());
        let new = dir.path().join(CONFIG_FOLDER);
        assert!(new.join("browser-roots").exists() && !new.join("half").exists());
    }

    #[test]
    fn newest_first_without_duplicates_and_capped() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = Recent::default();
        let files: Vec<PathBuf> = (0..12)
            .map(|i| touch(dir.path(), &format!("p{i}.ovrtone")))
            .collect();
        for f in &files {
            r.add(f);
        }
        assert_eq!(r.paths().len(), MAX);
        assert_eq!(r.paths()[0], files[11]);
        r.add(&files[5]);
        assert_eq!(r.paths()[0], files[5]);
        assert_eq!(r.paths().iter().filter(|p| **p == files[5]).count(), 1);
        r.remove(&files[5]);
        assert!(!r.paths().contains(&files[5]));
        r.clear();
        assert!(r.paths().is_empty());
    }

    #[test]
    fn survives_a_restart_and_forgets_deleted_projects() {
        let dir = tempfile::tempdir().unwrap();
        let list = dir.path().join("config").join("ovrtone").join("recent.txt");
        let (a, b) = (
            touch(dir.path(), "a.ovrtone"),
            touch(dir.path(), "b.ovrtone"),
        );
        let mut r = Recent::from_file(Some(list.clone()));
        r.add(&a);
        r.add(&b);

        let reloaded = Recent::from_file(Some(list.clone()));
        assert_eq!(reloaded.paths(), [b.clone(), a.clone()]);

        std::fs::remove_file(&b).unwrap();
        assert_eq!(Recent::from_file(Some(list)).paths(), [a]);
    }

    #[test]
    fn a_missing_or_unwritable_list_is_just_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Recent::from_file(Some(dir.path().join("nope.txt")))
            .paths()
            .is_empty());
        // The "file" is really a folder, so saving fails; that must not panic.
        let mut r = Recent::from_file(Some(dir.path().to_path_buf()));
        r.add(&touch(dir.path(), "a.ovrtone"));
        assert_eq!(r.paths().len(), 1);
    }
}
