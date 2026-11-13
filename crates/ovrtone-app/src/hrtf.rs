//! Measured heads for the headphone render: a SOFA file (AES69) of head-related impulse responses,
//! which the user brings (Ovrtone ships no data set). It is read with `sofar` (pure Rust, MIT or
//! Apache-2.0), resampled to the engine's rate, and turned into one pair of responses for each virtual
//! speaker of the layout. The choice belongs to this computer, so it is kept in the settings folder.

use std::path::PathBuf;
use std::sync::Mutex;

use ovrtone_engine::{HrtfSet, SpeakerLayout};
#[cfg(feature = "sofa")]
use ovrtone_engine::{HrtfSpeaker, MAX_HRIR, SPATIAL_CHANNELS};

const FILE: &str = "hrtf-file";

/// The chosen file (none: the modelled head) and a count that goes up when it changes.
static CHOICE: Mutex<Option<(Option<PathBuf>, u64)>> = Mutex::new(None);

/// The file read last, at the rate it was read for.
#[cfg(feature = "sofa")]
type Loaded = (PathBuf, u32, std::sync::Arc<sofar::reader::Sofar>);
#[cfg(feature = "sofa")]
static LOADED: Mutex<Option<Loaded>> = Mutex::new(None);

fn config() -> Option<PathBuf> {
    crate::recent_files::config_dir()
        .filter(|_| !cfg!(test))
        .map(|d| d.join(FILE))
}

/// The SOFA file chosen, and its change count.
pub fn current() -> (Option<PathBuf>, u64) {
    let Ok(mut held) = CHOICE.lock() else {
        return (None, 0);
    };
    held.get_or_insert_with(|| {
        let path = config()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .map(|t| PathBuf::from(t.trim()))
            .filter(|p| p.is_file());
        (path, 1)
    })
    .clone()
}

/// Chooses a SOFA file (none: back to the modelled head), and remembers it.
pub fn choose(path: Option<PathBuf>) {
    if let Ok(mut held) = CHOICE.lock() {
        let generation = held.as_ref().map_or(1, |(_, g)| g + 1);
        *held = Some((path.clone(), generation));
    }
    if let Some(f) = config() {
        let result = match &path {
            Some(p) => {
                if let Some(dir) = f.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                std::fs::write(&f, p.to_string_lossy().as_bytes())
            }
            None => std::fs::remove_file(&f).or(Ok(())),
        };
        if let Err(e) = result {
            tracing::warn!("cannot keep the HRTF choice: {e}");
        }
    }
}

/// Reads `path` for `rate`, or gives the copy read last.
#[cfg(feature = "sofa")]
fn load(path: &PathBuf, rate: u32) -> Result<std::sync::Arc<sofar::reader::Sofar>, String> {
    if let Ok(held) = LOADED.lock() {
        if let Some((p, r, sofa)) = held.as_ref() {
            if p == path && *r == rate {
                return Ok(sofa.clone());
            }
        }
    }
    let sofa = sofar::reader::OpenOptions::new()
        .sample_rate(rate as f32)
        .open(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let sofa = std::sync::Arc::new(sofa);
    if let Ok(mut held) = LOADED.lock() {
        *held = Some((path.clone(), rate, sofa.clone()));
    }
    Ok(sofa)
}

/// Without the SOFA reader in the build: a chosen file cannot be used.
#[cfg(not(feature = "sofa"))]
pub fn set_for(_layout: SpeakerLayout, _rate: u32) -> Result<Option<HrtfSet>, String> {
    match current().0 {
        Some(_) => Err("This build cannot read SOFA files yet: the modelled head is used.".to_string()),
        None => Ok(None),
    }
}

/// The responses of the chosen head for each virtual speaker of `layout` at `rate`. None with no
/// file chosen; Err when the file cannot be read.
#[cfg(feature = "sofa")]
pub fn set_for(layout: SpeakerLayout, rate: u32) -> Result<Option<HrtfSet>, String> {
    let (Some(path), _) = current() else {
        return Ok(None);
    };
    let sofa = load(&path, rate)?;
    let mut filter = sofar::reader::Filter::new(sofa.filter_len());
    let mut speakers = Vec::new();
    for channel in 0..SPATIAL_CHANNELS {
        let Some((az, el)) = layout.place(channel) else {
            continue;
        };
        // SOFA: x ahead, y to the left, z up, in metres; the bus's azimuth is to the right.
        let (a, e) = (az.to_radians(), el.to_radians());
        let distance = 1.2;
        let (x, y, z) = (e.cos() * a.cos() * distance, -e.cos() * a.sin() * distance, e.sin() * distance);
        sofa.filter(x, y, z, &mut filter);
        let frames = |seconds: f32| (seconds.max(0.0) * rate as f32).round() as usize;
        let take = |ir: &[f32]| ir[..ir.len().min(MAX_HRIR)].to_vec();
        speakers.push(HrtfSpeaker {
            channel,
            azimuth: az,
            left: take(&filter.left),
            right: take(&filter.right),
            delay_left: frames(filter.ldelay),
            delay_right: frames(filter.rdelay),
        });
    }
    Ok(Some(HrtfSet { speakers }))
}

/// The file's name, for showing.
pub fn name() -> Option<String> {
    current()
        .0
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
}
