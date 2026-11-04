//! Third-party plugins for Pro Wave: finding them on disk and loading them for the engine.
//!
//! This crate runs on the UI thread, apart from the [`PluginProcessor`] each load returns, which
//! goes to the engine in a `PluginSlot`. CLAP comes first; VST3 follows behind the same trait.
//! Plugins run in this process for now, so a plugin that crashes takes the program with it; the
//! trait is where a bridge to another process would go.
mod clap;
#[cfg(windows)]
mod editor_window;

pub use crate::clap::{clap_folders, find_clap_files, load_clap, scan_clap_file, LoadedClap};
#[doc(no_inline)]
pub use prowave_engine::PluginProcessor;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginFormat {
    Clap,
}

/// One plugin a file offers (a CLAP file can hold several).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginInfo {
    /// The plugin's own stable identifier, which a project stores.
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub format: PluginFormat,
    pub path: PathBuf,
    pub is_instrument: bool,
}

/// Why a plugin could not be scanned or loaded, in words for the status line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostError(pub String);

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for HostError {}
