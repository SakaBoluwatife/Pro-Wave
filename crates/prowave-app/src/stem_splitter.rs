//! Mix > Stem Splitter: splits the selected audio region into drums, bass, vocals and the rest, each
//! on a new track in the same place, with the original muted. The work is done by a helper program
//! (`stems\demucs.exe`, from demucs-rs) next to Pro Wave. It is its own process, so its size stays
//! out of this program and a failure in it costs only the split.
use crate::model::Project;
use crate::UiState;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, TryRecvError};

/// The stems the bundled model makes, by the names of their files.
const STEMS: [&str; 4] = ["drums", "bass", "vocals", "other"];
/// The model file, which the helper looks for in its cache.
const MODEL: &str = "htdemucs.safetensors";

/// The region being split.
#[derive(Clone, Debug)]
pub struct Source {
    track: usize,
    region: usize,
    path: PathBuf,
    name: String,
    start_bar: f32,
    len_bars: f32,
    source_start: f32,
    source_len: f32,
}

#[derive(Default)]
pub struct StemSplitter {
    /// What the window is showing: it is open while this is set.
    source: Option<Source>,
    job: Option<Receiver<Result<PathBuf, String>>>,
    error: Option<String>,
}

/// Opens the window on a region.
pub fn open(p: &Project, s: &mut UiState, track: usize, region: usize) {
    let Some(r) = p.tracks.get(track).and_then(|t| t.regions.get(region)) else {
        return;
    };
    let Some(audio) = r.audio.as_ref() else {
        return;
    };
    s.stems.error = None;
    s.stems.source = Some(Source {
        track,
        region,
        path: audio.file.path.clone(),
        name: r.name.clone(),
        start_bar: r.start_bar,
        len_bars: r.len_bars,
        source_start: r.params.source_start,
        source_len: r.params.source_len,
    });
}

/// Where the helper and the model are: `stems` beside the program, or `PROWAVE_STEMS_DIR`.
fn helper_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("PROWAVE_STEMS_DIR") {
        return Some(dir.into());
    }
    Some(std::env::current_exe().ok()?.parent()?.join("stems"))
}

/// Puts the bundled model where the helper looks for it, unless it is there already.
fn place_model(dir: &Path) -> Result<(), String> {
    let Some(cache) = std::env::var_os("LOCALAPPDATA") else {
        return Err("LOCALAPPDATA is not set".to_string());
    };
    let cache = PathBuf::from(cache).join("demucs-rs");
    let target = cache.join(MODEL);
    if target.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
    // By way of a temporary name: a copy cut short must not pass for the model.
    let partial = cache.join(format!("{MODEL}.part"));
    std::fs::copy(dir.join(MODEL), &partial).map_err(|e| format!("the bundled model: {e}"))?;
    std::fs::rename(&partial, &target).map_err(|e| e.to_string())
}

/// Runs the helper on `input`, writing the stems to `out`. On its own thread: it takes minutes.
fn split(dir: &Path, input: &Path, out: &Path) -> Result<(), String> {
    place_model(dir)?;
    let mut command = std::process::Command::new(dir.join("demucs.exe"));
    command
        .arg("-m")
        .arg("htdemucs")
        .arg("-o")
        .arg(out)
        .arg(input);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let done = command
        .output()
        .map_err(|e| format!("cannot start the splitter: {e}"))?;
    if done.status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&done.stderr);
    let last = text.lines().rev().find(|l| !l.trim().is_empty());
    Err(last.unwrap_or("the splitter stopped").to_string())
}

/// A folder of its own for one split, among the session's recordings so that saving keeps them.
fn new_output_dir(name: &str) -> PathBuf {
    let safe: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    crate::project_io::recordings_dir().join(format!("stems-{safe}-{stamp}"))
}

/// Puts the stems in `dir` on new tracks, playing what the source region plays, and mutes it.
fn place_stems(
    p: &mut Project,
    ctx: &egui::Context,
    source: &Source,
    dir: &Path,
) -> Result<(), String> {
    for stem in STEMS {
        let path = dir.join(format!("{stem}.wav"));
        crate::import::import_audio_at(p, &path, ctx, f64::from(source.start_bar), None)
            .map_err(|e| format!("{stem}: {e}"))?;
        if let Some(track) = p.tracks.last_mut() {
            let mut title = stem.to_string();
            title[..1].make_ascii_uppercase();
            track.name = format!("{} {title}", source.name);
            if let Some(region) = track.regions.last_mut() {
                region.name.clone_from(&track.name);
                region.len_bars = source.len_bars;
                region.params.source_start = source.source_start;
                region.params.source_len = source.source_len;
            }
        }
    }
    let original = p
        .tracks
        .get_mut(source.track)
        .and_then(|t| t.regions.get_mut(source.region))
        .filter(|r| r.audio.as_ref().is_some_and(|a| a.file.path == source.path));
    if let Some(region) = original {
        region.params.mute = true;
    }
    p.song_rev += 1;
    Ok(())
}

/// The window, and the split's end when it comes.
pub fn show(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    let Some(source) = s.stems.source.clone() else {
        return;
    };
    if let Some(job) = &s.stems.job {
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
        match job.try_recv() {
            Ok(Ok(dir)) => {
                s.stems.job = None;
                match place_stems(p, ctx, &source, &dir) {
                    Ok(()) => {
                        s.status = Some(format!("{} split into stems", source.name));
                        s.stems.source = None;
                        return;
                    }
                    Err(e) => s.stems.error = Some(e),
                }
            }
            Ok(Err(e)) => {
                s.stems.job = None;
                s.stems.error = Some(e);
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                s.stems.job = None;
                s.stems.error = Some("the splitter stopped".to_string());
            }
        }
    }
    let dir = helper_dir();
    let ready = dir
        .as_ref()
        .is_some_and(|d| d.join("demucs.exe").exists() && d.join(MODEL).exists());
    let (busy, mut start) = (s.stems.job.is_some(), false);
    let mut open = true;
    egui::Window::new("Stem Splitter")
        .id(egui::Id::unique("stem_splitter"))
        .open(&mut open)
        .resizable(false)
        .show(ctx, |ui| {
            ui.label(format!(
                "Split \"{}\" into drums, bass, vocals and other.",
                source.name
            ));
            ui.label("Each goes on a new track. The original region is muted.");
            if !ready {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "The splitter is not installed: stems\\demucs.exe and its model are missing.",
                );
            }
            if let Some(e) = &s.stems.error {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            ui.add_space(4.0);
            if busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Splitting. This can take a few minutes.");
                });
            } else if ui.add_enabled(ready, egui::Button::new("Split")).clicked() {
                start = true;
            }
        });
    if let (true, Some(dir)) = (start, dir) {
        let (tx, rx) = channel();
        let (input, out) = (source.path.clone(), new_output_dir(&source.name));
        std::thread::spawn(move || {
            let _ = tx.send(split(&dir, &input, &out).map(|()| out));
        });
        s.stems.error = None;
        s.stems.job = Some(rx);
    }
    // Closing while it works lets it finish unseen: nothing would take its result.
    if !open {
        s.stems.source = None;
        s.stems.job = None;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::model::TrackKind;

    fn wav(path: &Path) {
        let mut bytes = Vec::new();
        let data = 2000u32;
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&44_100u32.to_le_bytes());
        bytes.extend_from_slice(&88_200u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data.to_le_bytes());
        bytes.resize(44 + data as usize, 0);
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn stems_land_on_new_tracks_in_place_and_the_original_is_muted() {
        let dir = tempfile::tempdir().unwrap();
        for stem in STEMS {
            wav(&dir.path().join(format!("{stem}.wav")));
        }
        let song = dir.path().join("song.wav");
        wav(&song);
        let ctx = egui::Context::default();
        let mut p = Project::empty();
        let t = p.add_track(TrackKind::Audio);
        crate::import::import_audio_at(&mut p, &song, &ctx, 3.0, Some(t)).unwrap();
        let tracks = p.tracks.len();
        let mut s = UiState::new();
        open(&p, &mut s, t, 0);
        let source = s.stems.source.clone().unwrap();
        place_stems(&mut p, &ctx, &source, dir.path()).unwrap();
        assert_eq!(p.tracks.len(), tracks + 4);
        assert!(p.tracks[t].regions[0].params.mute);
        let drums = &p.tracks[tracks];
        assert_eq!(drums.name, "song Drums");
        assert_eq!(drums.kind, TrackKind::Audio);
        assert_eq!(drums.regions[0].start_bar, 3.0);
        assert!(!drums.regions[0].params.mute);
    }

    #[test]
    fn a_missing_stem_is_an_error_and_the_original_keeps_playing() {
        let dir = tempfile::tempdir().unwrap();
        let song = dir.path().join("song.wav");
        wav(&song);
        let ctx = egui::Context::default();
        let mut p = Project::empty();
        let t = p.add_track(TrackKind::Audio);
        crate::import::import_audio_at(&mut p, &song, &ctx, 1.0, Some(t)).unwrap();
        let mut s = UiState::new();
        open(&p, &mut s, t, 0);
        let source = s.stems.source.clone().unwrap();
        assert!(place_stems(&mut p, &ctx, &source, dir.path()).is_err());
        assert!(!p.tracks[t].regions[0].params.mute);
    }
}
