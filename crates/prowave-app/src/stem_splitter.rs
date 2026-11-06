//! Mix > Stem Splitter: splits the selected audio region into drums, bass, vocals and the rest, each
//! on a new track in the same place, with the original muted. The work is done by a helper program
//! (`stems\demucs.exe`, from demucs-rs) next to Pro Wave. It is its own process, so its size stays
//! out of this program and a failure in it costs only the split.
use crate::model::Project;
use crate::UiState;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// What the helper says it is doing, and the word to stop it.
#[derive(Clone, Default)]
struct JobControl {
    stage: Arc<Mutex<String>>,
    stop: Arc<AtomicBool>,
}

/// The words the helper prints, and what to tell the user. It prints no percentage.
fn stage_of(line: &str) -> Option<&'static str> {
    let line = line.trim_start();
    if line.starts_with("Reading") {
        Some("Reading the file")
    } else if line.starts_with("Loading") {
        Some("Loading the model")
    } else if line.starts_with("Separating") {
        Some("Separating the stems")
    } else if line.starts_with("Wrote") {
        Some("Writing the stems")
    } else {
        None
    }
}

/// What can be asked for: its name in the window, and the helper's stems with the track each makes.
const CHOICES: [(&str, &[(&str, &str)]); 3] = [
    ("Vocals", &[("vocals", "Vocals")]),
    ("Drums", &[("drums", "Drums")]),
    ("Instruments", &[("bass", "Bass"), ("other", "Instruments")]),
];

/// The stems of what is chosen: the helper's name, and the track's.
fn stems_of(chosen: [bool; 3]) -> Vec<(&'static str, &'static str)> {
    CHOICES
        .iter()
        .zip(chosen)
        .filter(|(_, on)| *on)
        .flat_map(|(choice, _)| choice.1.iter().copied())
        .collect()
}
/// What a split that was stopped on purpose reports. Not shown as an error.
const STOPPED: &str = "stopped";
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

pub struct StemSplitter {
    /// What to split out: vocals, drums, instruments.
    chosen: [bool; 3],
    /// What the window is showing: it is open while this is set.
    source: Option<Source>,
    job: Option<Receiver<Result<PathBuf, String>>>,
    control: JobControl,
    began: Option<Instant>,
    thread: Option<std::thread::JoinHandle<()>>,
    error: Option<String>,
}

impl Default for StemSplitter {
    fn default() -> Self {
        Self {
            chosen: [true; 3],
            source: None,
            job: None,
            control: JobControl::default(),
            began: None,
            thread: None,
            error: None,
        }
    }
}

impl Drop for StemSplitter {
    /// The helper does not outlive the program: it would go on using every core.
    fn drop(&mut self) {
        self.control.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
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
fn split(
    dir: &Path,
    input: &Path,
    out: &Path,
    stems: &[&str],
    control: &JobControl,
) -> Result<(), String> {
    place_model(dir)?;
    let mut command = std::process::Command::new(dir.join("demucs.exe"));
    command
        .arg("-m")
        .arg("htdemucs")
        .arg("-s")
        .arg(stems.join(","))
        .arg("-o")
        .arg(out)
        .arg(input);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start the splitter: {e}"))?;
    let last_line = Arc::new(Mutex::new(String::new()));
    if let Some(stderr) = child.stderr.take() {
        let (stage, last) = (control.stage.clone(), last_line.clone());
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if let (Some(now), Ok(mut stage)) = (stage_of(&line), stage.lock()) {
                    *stage = now.to_string();
                }
                if !line.trim().is_empty() {
                    if let Ok(mut last) = last.lock() {
                        *last = line;
                    }
                }
            }
        });
    }
    loop {
        if control.stop.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_dir_all(out);
            return Err(STOPPED.to_string());
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(_)) => {
                let last = last_line.lock().map(|l| l.clone()).unwrap_or_default();
                return Err(if last.is_empty() {
                    "the splitter stopped".to_string()
                } else {
                    last
                });
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(e.to_string()),
        }
    }
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
    chosen: [bool; 3],
) -> Result<(), String> {
    for (stem, title) in stems_of(chosen) {
        let path = dir.join(format!("{stem}.wav"));
        crate::import::import_audio_at(p, &path, ctx, f64::from(source.start_bar), None)
            .map_err(|e| format!("{stem}: {e}"))?;
        if let Some(track) = p.tracks.last_mut() {
            track.name = format!("{} {title}", source.name);
            if let Some(region) = track.regions.last_mut() {
                region.name.clone_from(&track.name);
                region.len_bars = source.len_bars;
                region.params.source_start = source.source_start;
                region.params.source_len = source.source_len;
            }
        }
    }
    // With only part split out, the original goes on playing: nothing else has the rest.
    if chosen != [true; 3] {
        p.song_rev += 1;
        return Ok(());
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
                match place_stems(p, ctx, &source, &dir, s.stems.chosen) {
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
                s.stems.error = (e != STOPPED).then_some(e);
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
            ui.add_space(2.0);
            ui.label("What to split out:");
            ui.add_enabled_ui(!busy, |ui| {
                ui.horizontal(|ui| {
                    for (k, (name, _)) in CHOICES.iter().enumerate() {
                        ui.checkbox(&mut s.stems.chosen[k], *name);
                    }
                });
            });
            ui.label("Each goes on a new track. With all three, the original region is muted.");
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
                let stage = s
                    .stems
                    .control
                    .stage
                    .lock()
                    .map(|t| t.clone())
                    .unwrap_or_default();
                let elapsed = s.stems.began.map_or(0, |b| b.elapsed().as_secs());
                ui.horizontal(|ui| {
                    ui.spinner();
                    let shown = if stage.is_empty() {
                        "Starting"
                    } else {
                        stage.as_str()
                    };
                    ui.label(format!("{shown}: {}:{:02}", elapsed / 60, elapsed % 60));
                });
                ui.label("On the CPU this is slow, often much longer than the song.");
                if ui.button("Stop").clicked() {
                    s.stems.control.stop.store(true, Ordering::Relaxed);
                }
            } else if ui
                .add_enabled(
                    ready && s.stems.chosen.contains(&true),
                    egui::Button::new("Split"),
                )
                .clicked()
            {
                start = true;
            }
        });
    if let (true, Some(dir)) = (start, dir) {
        let (tx, rx) = channel();
        let (input, out) = (source.path.clone(), new_output_dir(&source.name));
        let stems = stems_of(s.stems.chosen);
        let control = JobControl::default();
        s.stems.control = control.clone();
        s.stems.began = Some(Instant::now());
        s.stems.thread = Some(std::thread::spawn(move || {
            let names: Vec<&str> = stems.iter().map(|(stem, _)| *stem).collect();
            let _ = tx.send(split(&dir, &input, &out, &names, &control).map(|()| out));
        }));
        s.stems.error = None;
        s.stems.job = Some(rx);
    }
    // Closing while it works stops it: nothing would take its result, and it holds the CPU.
    if !open {
        s.stems.control.stop.store(true, Ordering::Relaxed);
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
        for (stem, _) in stems_of([true; 3]) {
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
        place_stems(&mut p, &ctx, &source, dir.path(), [true; 3]).unwrap();
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
        assert!(place_stems(&mut p, &ctx, &source, dir.path(), [true; 3]).is_err());
        assert!(!p.tracks[t].regions[0].params.mute);
    }
}
