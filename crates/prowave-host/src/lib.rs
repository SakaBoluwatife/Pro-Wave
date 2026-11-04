//! Third-party plugins for Pro Wave: finding them on disk and loading them for the engine.
//!
//! This crate runs on the UI thread, apart from the [`PluginProcessor`] each load returns, which
//! goes to the engine in a `PluginSlot`. CLAP and VST3 look the same from outside: a
//! [`LoadedPlugin`] on the UI thread and a [`PluginProcessor`] on the audio thread. Plugins run in
//! this process for now, so a plugin that crashes takes the program with it; these traits are where
//! a bridge to another process would go.
mod clap;
#[cfg(windows)]
mod editor_window;
mod sampler;
#[cfg(windows)]
mod vst3_plugin;

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
}

/// Both halves of a plugin that was just loaded.
pub type Loaded = (Box<dyn LoadedPlugin>, Box<dyn PluginProcessor>);

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

/// Loads plugin `id` of a file of any format, for the engine's sample rate.
pub fn load_plugin(path: &Path, id: &str, sample_rate: u32) -> Result<Loaded, HostError> {
    match PluginFormat::of(path) {
        PluginFormat::Sampler => load_sampler(path, sample_rate),
        PluginFormat::Clap => {
            let (loaded, audio) = load_clap(path, id, sample_rate)?;
            Ok((Box::new(loaded), audio))
        }
        #[cfg(windows)]
        PluginFormat::Vst3 => load_vst3(path, id, sample_rate),
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
            let (loaded, mut audio) = match load_plugin(file, &info.id, 48_000) {
                Ok(both) => both,
                Err(e) => {
                    println!("{}: {e}", info.name);
                    continue;
                }
            };
            let params = loaded.params();
            println!("{}: {} parameters", info.name, params.len());
            let input = vec![0.1f32; PLUGIN_BLOCK];
            let (mut l, mut r) = (vec![0.0f32; PLUGIN_BLOCK], vec![0.0f32; PLUGIN_BLOCK]);
            for block in 0..200 {
                if let Some(param) = params.first() {
                    audio.set_param(param.id, f64::from(block % 2));
                }
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
