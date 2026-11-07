//! The user's own plugin folders, and what keeps a plugin that crashes the program on load from
//! doing it every time a project opens.
//!
//! A plugin runs inside this process, so its crash ends the program. Before one loads, its path is
//! written down, and rubbed out when it has. A path still there at the next load means the program
//! never came back from that plugin: it is not loaded again until the user presses Rescan.
use std::path::{Path, PathBuf};

const FOLDERS_FILE: &str = "plugin-folders";
const LOADING_FILE: &str = "plugin-loading";
const BLOCKED_FILE: &str = "plugin-blocked";

fn config() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    crate::recent_files::config_dir()
}

fn read_lines(dir: &Path, name: &str) -> Vec<PathBuf> {
    std::fs::read_to_string(dir.join(name))
        .map(|text| text.lines().map(PathBuf::from).collect())
        .unwrap_or_default()
}

fn write_lines(dir: &Path, name: &str, lines: &[PathBuf]) {
    let text: Vec<String> = lines.iter().map(|l| l.display().to_string()).collect();
    let saved =
        std::fs::create_dir_all(dir).and_then(|()| std::fs::write(dir.join(name), text.join("\n")));
    if let Err(e) = saved {
        tracing::warn!("could not save {name}: {e}");
    }
}

fn add_line(dir: &Path, name: &str, line: &Path) {
    let mut lines = read_lines(dir, name);
    if !lines.iter().any(|l| l == line) {
        lines.push(line.to_path_buf());
        write_lines(dir, name, &lines);
    }
}

/// What is left from a load that never came back goes on the blocked list.
fn settle_in(dir: &Path) {
    if let Some(crashed) = read_lines(dir, LOADING_FILE).first() {
        add_line(dir, BLOCKED_FILE, crashed);
    }
    let _ = std::fs::remove_file(dir.join(LOADING_FILE));
}

fn is_blocked_in(dir: &Path, path: &Path) -> bool {
    settle_in(dir);
    read_lines(dir, BLOCKED_FILE).iter().any(|l| l == path)
}

fn before_load_in(dir: &Path, path: &Path) {
    write_lines(dir, LOADING_FILE, &[path.to_path_buf()]);
}

fn after_load_in(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(LOADING_FILE));
}

/// Whether this plugin ended the program the last time it was loaded.
pub fn is_blocked(path: &Path) -> bool {
    config().is_some_and(|dir| is_blocked_in(&dir, path))
}

/// Say this before loading a plugin, and [`after_load`] once it is done with.
pub fn before_load(path: &Path) {
    if let Some(dir) = config() {
        before_load_in(&dir, path);
    }
}

pub fn after_load() {
    if let Some(dir) = config() {
        after_load_in(&dir);
    }
}

/// Gives every blocked plugin another chance.
pub fn clear_blocked() {
    if let Some(dir) = config() {
        let _ = std::fs::remove_file(dir.join(BLOCKED_FILE));
    }
}

/// The folders the user added to look for plugins in.
pub fn plugin_folders() -> Vec<PathBuf> {
    config().map_or_else(Vec::new, |dir| read_lines(&dir, FOLDERS_FILE))
}

pub fn add_folder(folder: &Path) {
    if let Some(dir) = config() {
        add_line(&dir, FOLDERS_FILE, folder);
    }
}

pub fn remove_folder(folder: &Path) {
    if let Some(dir) = config() {
        let mut lines = read_lines(&dir, FOLDERS_FILE);
        lines.retain(|l| l != folder);
        write_lines(&dir, FOLDERS_FILE, &lines);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_plugin_that_never_came_back_is_blocked_until_a_rescan() {
        let dir = tempfile::tempdir().unwrap();
        let (good, bad) = (Path::new("good.vst3"), Path::new("bad.vst3"));
        before_load_in(dir.path(), good);
        after_load_in(dir.path());
        assert!(!is_blocked_in(dir.path(), good), "it came back");

        // The program dies here: there is no `after_load`.
        before_load_in(dir.path(), bad);
        assert!(is_blocked_in(dir.path(), bad));
        assert!(!is_blocked_in(dir.path(), good));
        assert!(is_blocked_in(dir.path(), bad), "and it stays so");

        std::fs::remove_file(dir.path().join(BLOCKED_FILE)).unwrap();
        assert!(!is_blocked_in(dir.path(), bad));
    }

    #[test]
    fn lines_keep_a_folder_once() {
        let dir = tempfile::tempdir().unwrap();
        add_line(dir.path(), FOLDERS_FILE, Path::new("D:\\Plugins"));
        add_line(dir.path(), FOLDERS_FILE, Path::new("D:\\Plugins"));
        add_line(dir.path(), FOLDERS_FILE, Path::new("E:\\More Plugins"));
        assert_eq!(read_lines(dir.path(), FOLDERS_FILE).len(), 2);
    }
}
