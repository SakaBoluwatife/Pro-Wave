//! Favourite plugins, and the kind of effect a plugin is, for sorting and finding them.
//!
//! The favourites are kept in the settings folder (`plugin-favourites`, one plugin id a line), for
//! every project. The kind is worked out from the plugin's name, as the plugin files do not say it
//! in a form all formats share.
use ovrtone_host::PluginInfo;
use std::collections::BTreeSet;
use std::sync::{Mutex, OnceLock};

fn file() -> Option<std::path::PathBuf> {
    crate::recent_files::config_dir().map(|d| d.join("plugin-favourites"))
}

fn set() -> &'static Mutex<BTreeSet<String>> {
    static SET: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();
    SET.get_or_init(|| {
        let text = file()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .unwrap_or_default();
        Mutex::new(
            text.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect(),
        )
    })
}

pub fn is_favourite(id: &str) -> bool {
    set().lock().is_ok_and(|s| s.contains(id))
}

/// Stars or unstars a plugin, and keeps the list.
pub fn toggle(id: &str) {
    let Ok(mut s) = set().lock() else {
        return;
    };
    if !s.remove(id) {
        s.insert(id.to_string());
    }
    let text: String = s.iter().map(|id| format!("{id}\n")).collect();
    drop(s);
    if let Some(f) = file() {
        let saved = f
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&f, text));
        if let Err(e) = saved {
            tracing::warn!("could not save the favourite plugins: {e}");
        }
    }
}

/// The kinds a plugin can be filed under, in the order the lists show them.
pub const KINDS: [&str; 13] = [
    "Instrument",
    "EQ",
    "Dynamics",
    "Saturation",
    "Reverb",
    "Delay",
    "Modulation",
    "Pitch",
    "Filter",
    "Stereo",
    "Mastering",
    "Utility",
    "Other",
];

/// The kind of plugin `info` is, from its name (and whether it plays notes).
pub fn kind_of(info: &PluginInfo) -> &'static str {
    if info.is_instrument {
        return "Instrument";
    }
    let name = info.name.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| name.contains(w));
    if has(&["eq", "equal", "pultec", "pult", "tilt", "filter bank"]) && !has(&["sequenc"]) {
        "EQ"
    } else if has(&["comp", "limit", "gate", "expand", "de-ess", "deess", "dyn", "transient", "clip", "1176", "la-2", "la2", "ssl", "bus", "glue", "squash", "leveler", "maxim", "l1", "l2"]) {
        "Dynamics"
    } else if has(&["satur", "tape", "tube", "drive", "dist", "crush", "fuzz", "warm", "decap", "heat", "amp", "lo-fi", "lofi"]) {
        "Saturation"
    } else if has(&["reverb", "verb", "room", "hall", "plate", "space", "convol", "chamber"]) {
        "Reverb"
    } else if has(&["delay", "echo", "dly"]) {
        "Delay"
    } else if has(&["chorus", "flang", "phase", "trem", "vibrato", "rotary", "ensemble", "modul", "pan"]) {
        "Modulation"
    } else if has(&["pitch", "tune", "melodyne", "harmon", "shift", "vocod"]) {
        "Pitch"
    } else if has(&["filter", "wah", "cutoff"]) {
        "Filter"
    } else if has(&["stereo", "width", "imager", "spread", "wider", "mid/side", "ms "]) {
        "Stereo"
    } else if has(&["master", "ozone", "loud", "meter", "lufs", "analy"]) {
        "Mastering"
    } else if has(&["utility", "gain", "trim", "scope", "tuner", "test"]) {
        "Utility"
    } else {
        "Other"
    }
}
