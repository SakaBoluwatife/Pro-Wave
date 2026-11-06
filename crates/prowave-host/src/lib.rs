//! Third-party plugins for Pro Wave: finding them on disk and loading them for the engine.
//!
//! This crate runs on the UI thread, apart from the [`PluginProcessor`] each load returns, which
//! goes to the engine in a `PluginSlot`. CLAP and VST3 look the same from outside: a
//! [`LoadedPlugin`] on the UI thread and a [`PluginProcessor`] on the audio thread. Plugins run in
//! this process unless crash isolation is on, in which case [`bridge`] puts them in another one
//! behind these same traits.
#[cfg(windows)]
mod ara;
#[cfg(windows)]
mod bridge;
mod clap;
#[cfg(windows)]
mod editor_window;
mod sampler;
#[cfg(windows)]
mod vst3_plugin;

#[cfg(windows)]
pub use crate::bridge::{
    host_request, isolation_on, load_isolated, load_plugin_safely, run_plugin_host, set_host_exe,
    set_isolation,
};
pub use crate::clap::{clap_folders, find_clap_files, load_clap, scan_clap_file, LoadedClap};
pub use crate::sampler::load_sampler;
#[cfg(windows)]
pub use crate::vst3_plugin::{find_vst3_files, load_vst3, scan_vst3_file, vst3_folders};
#[doc(no_inline)]
pub use prowave_engine::PluginProcessor;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginFormat {
    Clap,
    Vst3,
    /// An audio file, played by the built-in sampler.
    Sampler,
}

impl PluginFormat {
    /// The format of a plugin file, by its name.
    pub fn of(path: &Path) -> Self {
        let is = |ext: &str| {
            path.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case(ext))
        };
        if is("vst3") {
            Self::Vst3
        } else if is("clap") {
            Self::Clap
        } else {
            Self::Sampler
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Clap => "CLAP",
            Self::Vst3 => "VST3",
            Self::Sampler => "Sampler",
        }
    }
}

/// One plugin a file offers (a file can hold several).
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

/// The UI-thread half of a loaded plugin, of any format. It must outlive the audio half that came
/// with it and stay on the UI thread. Call [`LoadedPlugin::tick`] about once a frame.
/// A button in the bar at the top of a plugin's window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderClick {
    Bypass,
    Presets,
    Compare,
    CopyCompare,
}

/// What the bar at the top of a plugin's window shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeaderState {
    pub bypass: bool,
    /// A/B compare is on its B side.
    pub on_b: bool,
    /// The preset last loaded or saved, if any.
    pub preset: Option<String>,
}

/// A parameter a plugin lets the host move.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginParam {
    pub id: u32,
    pub name: String,
}

pub trait LoadedPlugin {
    /// The parameters that can be automated.
    fn params(&self) -> Vec<PluginParam> {
        Vec::new()
    }

    /// What the user moved in the plugin's window since last asked: parameter and position 0..=1.
    fn take_edits(&mut self) -> Vec<(u32, f64)> {
        Vec::new()
    }

    /// Whether a control in the plugin's window is being held now, for plugins that say so.
    fn editing(&self) -> bool {
        false
    }

    /// Whether a control in the plugin's window was let go since the last call.
    fn take_edit_ended(&mut self) -> bool {
        false
    }

    /// Where a parameter is now, as a position 0..=1.
    fn param_value(&mut self, _id: u32) -> Option<f64> {
        None
    }

    /// Shows a parameter's position in the plugin's window, as an automation lane moves it. Formats
    /// whose window follows the audio half by itself need nothing here.
    fn show_param(&mut self, _id: u32, _value: f64) {}

    /// Whether the plugin offers a side-chain input.
    fn has_side_input(&self) -> bool {
        false
    }

    /// Whether the plugin must be loaded again for its side-chain input to be on or off. A VST3
    /// plugin's buses can only be switched while it is not running.
    fn side_needs_reload(&self, _on: bool) -> bool {
        false
    }

    /// What the bar at the top of the editor window shows, while it is open.
    fn show_header(&mut self, _state: &HeaderState) {}

    /// A button pressed in that bar since last asked.
    fn take_header_click(&mut self) -> Option<HeaderClick> {
        None
    }

    /// Shows a menu under the bar's preset button and waits for the choice, by its place in `items`.
    /// An empty item is a separator.
    fn header_menu(&mut self, _items: &[String]) -> Option<usize> {
        None
    }

    /// The delay the plugin reported when it was activated, in samples.
    fn latency_samples(&self) -> usize;
    /// Whether the audio half is gone. Keep this half until it is.
    fn released(&self) -> bool;
    /// Runs what the plugin asked for on this thread, and looks after its editor.
    fn tick(&mut self);
    fn editor_is_open(&self) -> bool;
    /// Where the editor window was when it closed, once, to open there next time.
    fn take_closed_position(&mut self) -> Option<(i32, i32)>;
    /// Opens the plugin's own interface in a window above `owner`, or brings it to the front.
    fn open_editor(
        &mut self,
        title: &str,
        owner: Option<isize>,
        position: Option<(i32, i32)>,
    ) -> Result<(), HostError>;
    fn close_editor(&mut self);
    /// The plugin's whole state, to give back with `load_state`; `None` if it keeps none.
    fn save_state(&mut self) -> Result<Option<Vec<u8>>, HostError>;
    /// Puts back a state from `save_state`, perhaps of an earlier run of the program.
    fn load_state(&mut self, bytes: &[u8]) -> Result<(), HostError>;
    /// Whether the plugin said its state changed, since last asked.
    fn take_dirty(&mut self) -> bool;

    /// Whether the plugin's process has ended, in the case that it has one of its own.
    fn crashed(&self) -> bool {
        false
    }

    /// Something that went wrong with no call waiting to hear it, once.
    fn take_failure(&mut self) -> Option<String> {
        None
    }

    /// Whether this plugin plays the audio regions of its track itself (ARA).
    fn is_ara(&self) -> bool {
        false
    }

    /// Tells a plugin that [`Self::is_ara`] which regions its track has. False if it could not be
    /// done just now: try again soon. The sound of this plugin stops for a moment if regions were
    /// added or taken away.
    fn ara_sync(&mut self, _regions: &[AraRegion]) -> Result<bool, HostError> {
        Ok(true)
    }

    /// The process it runs in, if that is not this one.
    fn process_id(&self) -> Option<u32> {
        None
    }
}

/// An audio region on the track of a plugin that plays the track's audio itself (ARA). All times
/// are in seconds.
#[derive(Clone, Debug)]
pub struct AraRegion {
    pub file: prowave_engine::AudioFile,
    pub name: String,
    /// Where it starts in the project.
    pub start: f64,
    /// Where in the file it starts.
    pub offset: f64,
    pub duration: f64,
}

/// Both halves of a plugin that was just loaded.
pub type Loaded = (Box<dyn LoadedPlugin>, Box<dyn PluginProcessor>);

#[cfg(not(windows))]
pub fn load_plugin_safely(
    path: &Path,
    id: &str,
    sample_rate: u32,
    side: bool,
) -> Result<Loaded, HostError> {
    load_plugin(path, id, sample_rate, side)
}

#[cfg(not(windows))]
pub fn isolation_on() -> bool {
    false
}

/// Where plugins of every format are installed, plus the folders `extra`, and the files there.
/// Nothing is loaded.
pub fn find_plugin_files(extra: &[PathBuf]) -> Vec<PathBuf> {
    let with_extra = |mut dirs: Vec<PathBuf>| {
        dirs.extend_from_slice(extra);
        dirs
    };
    let mut files = find_clap_files(&with_extra(clap_folders()));
    #[cfg(windows)]
    files.extend(find_vst3_files(&with_extra(vst3_folders())));
    files
}

/// Loads a plugin file of any format and reads what it offers. This runs the file's code, so a
/// plugin that crashes here crashes the caller: scan in a process of its own.
pub fn scan_plugin_file(path: &Path) -> Result<Vec<PluginInfo>, HostError> {
    match PluginFormat::of(path) {
        PluginFormat::Sampler => Err(HostError(format!("{} is not a plugin", path.display()))),
        PluginFormat::Clap => scan_clap_file(path),
        #[cfg(windows)]
        PluginFormat::Vst3 => scan_vst3_file(path),
        #[cfg(not(windows))]
        PluginFormat::Vst3 => Err(HostError("VST3 needs Windows for now".to_string())),
    }
}

/// [`load_plugin_safely`] for a plugin on an audio track: one that offers ARA is then bound to the
/// track's audio. Not for plugins in a process of their own, which run as any other.
pub fn load_plugin_for_audio(
    path: &Path,
    id: &str,
    sample_rate: u32,
    side: bool,
) -> Result<Loaded, HostError> {
    #[cfg(windows)]
    if !isolation_on() && PluginFormat::of(path) == PluginFormat::Vst3 {
        return crate::vst3_plugin::load_vst3_with(path, id, sample_rate, side, true);
    }
    load_plugin_safely(path, id, sample_rate, side)
}

/// Loads plugin `id` of a file of any format, for the engine's sample rate. `side` says a source for
/// its side-chain input is chosen, if it has one.
pub fn load_plugin(
    path: &Path,
    id: &str,
    sample_rate: u32,
    side: bool,
) -> Result<Loaded, HostError> {
    match PluginFormat::of(path) {
        PluginFormat::Sampler => load_sampler(path, sample_rate),
        PluginFormat::Clap => {
            let (loaded, audio) = load_clap(path, id, sample_rate)?;
            Ok((Box::new(loaded), audio))
        }
        #[cfg(windows)]
        PluginFormat::Vst3 => load_vst3(path, id, sample_rate, side),
        #[cfg(not(windows))]
        PluginFormat::Vst3 => Err(HostError("VST3 needs Windows for now".to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "loads plugins installed on this computer"]
    fn installed_plugins_list_their_parameters_and_take_moves() {
        use prowave_engine::{PluginTransport, PLUGIN_BLOCK};
        let files = find_plugin_files(&[]);
        let wanted = ["De-Esser", "Lounge Lizard"];
        let mut tried = 0;
        let vst3 = files
            .iter()
            .filter(|f| wanted.iter().any(|w| f.display().to_string().contains(w)));
        let clap = files
            .iter()
            .filter(|f| PluginFormat::of(f) == PluginFormat::Clap)
            .take(2);
        for file in vst3.chain(clap) {
            let Ok(found) = scan_plugin_file(file) else {
                continue;
            };
            let Some(info) = found.first() else {
                continue;
            };
            let (loaded, mut audio) = match load_plugin(file, &info.id, 48_000, true) {
                Ok(both) => both,
                Err(e) => {
                    println!("{}: {e}", info.name);
                    continue;
                }
            };
            let params = loaded.params();
            println!(
                "{}: {} parameters, side input {}",
                info.name,
                params.len(),
                loaded.has_side_input()
            );
            let key = vec![0.5f32; PLUGIN_BLOCK];
            let input = vec![0.1f32; PLUGIN_BLOCK];
            let (mut l, mut r) = (vec![0.0f32; PLUGIN_BLOCK], vec![0.0f32; PLUGIN_BLOCK]);
            for block in 0..200 {
                if let Some(param) = params.first() {
                    audio.set_param(param.id, f64::from(block % 2));
                }
                audio.set_side_input([&key, &key]);
                audio.process(
                    [&input, &input],
                    [&mut l, &mut r],
                    &[],
                    &PluginTransport::default(),
                );
                assert!(l.iter().chain(&r).all(|s| s.is_finite()), "{}", info.name);
            }
            drop(audio);
            drop(loaded);
            tried += 1;
        }
        assert!(tried > 0);
    }

    #[test]
    fn the_format_goes_by_the_file_name() {
        assert_eq!(PluginFormat::of(Path::new("a.clap")), PluginFormat::Clap);
        assert_eq!(PluginFormat::of(Path::new("B.VST3")), PluginFormat::Vst3);
        assert_eq!(PluginFormat::Vst3.name(), "VST3");
        assert_eq!(
            PluginFormat::of(Path::new("kick.wav")),
            PluginFormat::Sampler
        );
    }
}
