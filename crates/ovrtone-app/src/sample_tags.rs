//! What the Browser knows of a sample beyond its name: the tags the user gave it, and the tempo and
//! key its file name tells (packs name loops like "Afro chills_ F#_ 101.wav"). Tags are kept in the
//! settings folder (`sample-tags`), one sample a line, and search finds them.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

fn file() -> Option<PathBuf> {
    crate::recent_files::config_dir().map(|d| d.join("sample-tags"))
}

fn store() -> &'static Mutex<HashMap<PathBuf, Vec<String>>> {
    static STORE: OnceLock<Mutex<HashMap<PathBuf, Vec<String>>>> = OnceLock::new();
    STORE.get_or_init(|| {
        let text = file()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .unwrap_or_default();
        let map = text
            .lines()
            .filter_map(|line| {
                let (path, tags) = line.split_once('\t')?;
                let tags = parse(tags);
                (!tags.is_empty()).then(|| (PathBuf::from(path), tags))
            })
            .collect();
        Mutex::new(map)
    })
}

/// Tags typed as a list: split at commas, trimmed, empty ones dropped.
pub fn parse(text: &str) -> Vec<String> {
    text.split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

pub fn tags_of(path: &Path) -> Vec<String> {
    store()
        .lock()
        .ok()
        .and_then(|m| m.get(path).cloned())
        .unwrap_or_default()
}

/// Sets a sample's tags (none: forgets it) and keeps the list.
pub fn set_tags(path: &Path, tags: Vec<String>) {
    let Ok(mut m) = store().lock() else {
        return;
    };
    if tags.is_empty() {
        m.remove(path);
    } else {
        m.insert(path.to_path_buf(), tags);
    }
    let mut lines: Vec<String> = m
        .iter()
        .map(|(p, t)| format!("{}\t{}", p.display(), t.join(", ")))
        .collect();
    drop(m);
    lines.sort();
    let Some(f) = file() else {
        return;
    };
    let saved = f
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&f, lines.join("\n")));
    if let Err(e) = saved {
        tracing::warn!("could not save the sample tags: {e}");
    }
}

/// The tempo and key a file name gives, if any: a number from 60 to 200 on its own (or with
/// "bpm"), and a note name with an optional sharp or flat and minor or major (F#, Abm, C#min, Amaj).
pub fn name_info(name: &str) -> (Option<u32>, Option<String>) {
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    let words: Vec<&str> = stem
        .split(|c: char| !(c.is_alphanumeric() || c == '#'))
        .filter(|w| !w.is_empty())
        .collect();
    let mut bpm = None;
    let mut key = None;
    for (i, w) in words.iter().enumerate() {
        let lower = w.to_lowercase();
        let number = lower.trim_end_matches("bpm");
        if bpm.is_none() {
            if let Ok(n) = number.parse::<u32>() {
                let said_bpm = lower.ends_with("bpm")
                    || words.get(i + 1).is_some_and(|n| n.eq_ignore_ascii_case("bpm"));
                if (60..=200).contains(&n) && (said_bpm || number.len() == lower.len()) {
                    bpm = Some(n);
                    continue;
                }
            }
        }
        if key.is_none() {
            if let Some(k) = key_of(w) {
                key = Some(k);
            }
        }
    }
    (bpm, key)
}

/// A key written as a word: `F#`, `Abm`, `Dbmin`, `Cmaj`, `Gminor`. A single capital letter alone
/// (`A`, `E`) counts too, as packs often write the key that way.
fn key_of(word: &str) -> Option<String> {
    let mut chars = word.chars();
    let root = chars.next()?;
    if !('A'..='G').contains(&root) {
        return None;
    }
    let rest: String = chars.collect();
    let (accidental, rest) = if let Some(r) = rest.strip_prefix('#') {
        ("#", r)
    } else if let Some(r) = rest.strip_prefix('b') {
        ("b", r)
    } else {
        ("", rest.as_str())
    };
    let quality = match rest.to_lowercase().as_str() {
        "" | "maj" | "major" => "",
        "m" | "min" | "minor" => "m",
        _ => return None,
    };
    Some(format!("{root}{accidental}{quality}"))
}
