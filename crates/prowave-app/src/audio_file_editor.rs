//! Audio File Editor (Logic: Audio File Editor): edits that change the sound of a file itself, for
//! every region that plays it: normalize, gain, fades, silence, reverse and trim.
//!
//! Nothing here ever writes to the file being edited. A copy of it is made first and checked
//! (the backup), the edit is written to a new file, and the project's regions then point at the
//! new file. So a user's original on disk stays as it was, Undo only has to point the regions
//! back, and a failure at any step leaves the project and every file untouched. The copies stay
//! in the config folder until the user deletes them; the project file holds only what it plays.
use crate::model::{AudioRegion, Project};
use crate::UiState;
use prowave_engine::AudioFile;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, TryRecvError};

/// Longest file it will load into memory: about 2.7 GB of stereo at 48 kHz.
const MAX_SECONDS: f64 = 2.0 * 3600.0;
/// Below this the file counts as not having been read to its end.
const MIN_READ_SHARE: f64 = 0.99;

/// What an edit does, over the selection or the whole file.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    /// Brings the loudest sample to this level, in dBFS.
    Normalize { target_db: f32 },
    Gain { db: f32 },
    FadeIn,
    FadeOut,
    Silence,
    Reverse,
    /// Keeps the selection and removes the rest.
    Trim,
}

impl Op {
    /// Whether it moves audio in time, which take folders cannot follow.
    fn moves_audio(self) -> bool {
        matches!(self, Op::Reverse | Op::Trim)
    }
}

/// Carries out `op` on interleaved stereo. `from` and `to` are frames; `to` is past the last one.
/// Says why when there is nothing it can do, and then leaves the audio as it was.
pub fn apply(stereo: &mut Vec<f32>, op: Op, from: usize, to: usize) -> Result<(), String> {
    let frames = stereo.len() / 2;
    let (from, to) = (from.min(frames), to.min(frames));
    if from >= to {
        return Err("Nothing is selected in the file".to_string());
    }
    let part = from * 2..to * 2;
    match op {
        Op::Normalize { target_db } => {
            let peak = stereo[part.clone()]
                .iter()
                .fold(0.0f32, |m, s| m.max(s.abs()));
            if peak <= 1e-9 {
                return Err("Normalize: this part of the file is silent".to_string());
            }
            let gain = 10f32.powf(target_db / 20.0) / peak;
            stereo[part].iter_mut().for_each(|s| *s *= gain);
        }
        Op::Gain { db } => {
            let gain = 10f32.powf(db / 20.0);
            stereo[part].iter_mut().for_each(|s| *s *= gain);
        }
        Op::FadeIn | Op::FadeOut => {
            let span = (to - from).max(2) as f32 - 1.0;
            for (k, frame) in stereo[part].chunks_exact_mut(2).enumerate() {
                let t = k as f32 / span;
                let gain = if op == Op::FadeIn { t } else { 1.0 - t };
                frame[0] *= gain;
                frame[1] *= gain;
            }
        }
        Op::Silence => stereo[part].fill(0.0),
        Op::Reverse => {
            let chunk = &mut stereo[part];
            chunk.reverse();
            // Reversing the samples swapped left and right as well: put them back.
            chunk.chunks_exact_mut(2).for_each(|f| f.swap(0, 1));
        }
        Op::Trim => {
            stereo.truncate(to * 2);
            stereo.drain(..from * 2);
        }
    }
    Ok(())
}

/// What an edit left behind.
#[derive(Clone, Debug)]
pub struct Done {
    pub file: AudioFile,
    pub backup: PathBuf,
    /// Loudest sample of the result: above 1 it is over full scale (kept, as the file is float).
    pub peak: f32,
}

/// Where edited files and backups are kept.
fn store_dir() -> Option<PathBuf> {
    if cfg!(test) {
        return Some(std::env::temp_dir().join("prowave-test-audio-edits"));
    }
    Some(crate::recent_files::config_dir()?.join("audio-edits"))
}

/// `name`, safe as a file name on every system.
fn safe_name(name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let clean = clean.trim().trim_matches('.');
    if clean.is_empty() {
        "audio".to_string()
    } else {
        clean.to_string()
    }
}

/// A file made here, under a name nothing has yet: reserves it, so another copy of the program
/// cannot get the same one. Numbered from 1.
fn reserve(store: &Path, stem: &str, ext: &str) -> Result<PathBuf, String> {
    for n in 1..100_000u32 {
        let path = store.join(format!("{stem} edit {n}.{ext}"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("cannot write in {}: {e}", store.display())),
        }
    }
    Err(format!("too many edits in {}", store.display()))
}

/// The name of a file without its " edit 3" from earlier edits.
fn base_stem(path: &Path) -> String {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("audio");
    let base = match stem.rsplit_once(" edit ") {
        Some((base, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => base,
        _ => stem,
    };
    safe_name(base)
}

/// Makes sure a copy of `path` is kept before it is edited, and returns where. A file made here
/// by an earlier edit is its own copy, as nothing ever writes to it again. The copy is checked
/// against the file's length: an error here stops the edit before anything has changed.
fn keep_backup(path: &Path, store: &Path) -> Result<PathBuf, String> {
    if path.starts_with(store) {
        return Ok(path.to_path_buf());
    }
    let dir = store.join("backups");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("cannot make a backup in {}: {e}", dir.display()))?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("audio");
    let target = reserve(&dir, &base_stem(path), &safe_name(ext))?;
    let fail = |e: String| {
        let _ = std::fs::remove_file(&target);
        format!("cannot back up {}: {e}", path.display())
    };
    let copied = std::fs::copy(path, &target).map_err(|e| fail(e.to_string()))?;
    let len = std::fs::metadata(path)
        .map_err(|e| fail(e.to_string()))?
        .len();
    if copied != len {
        return Err(fail(format!("copied {copied} of {len} bytes")));
    }
    Ok(target)
}

/// The edit itself: backup, read, change, write. Runs off the UI thread. `range` is in seconds;
/// none means the whole file.
pub fn run_edit(
    file: &AudioFile,
    op: Op,
    range: Option<(f64, f64)>,
    store: &Path,
) -> Result<Done, String> {
    if file.channels > 2 {
        return Err(format!(
            "It has {} channels, and only mono and stereo files can be edited here",
            file.channels
        ));
    }
    let seconds = file.duration_secs();
    if seconds > MAX_SECONDS {
        return Err("The file is too long to edit here (the limit is two hours)".to_string());
    }
    std::fs::create_dir_all(store)
        .map_err(|e| format!("cannot write in {}: {e}", store.display()))?;
    // Before anything else. Nothing has been changed if this fails.
    let backup = keep_backup(&file.path, store)?;
    let mut stereo = file
        .read_stereo_for(seconds + 1.0)
        .map_err(|e| format!("cannot read {}: {e}", file.path.display()))?;
    let frames = stereo.len() / 2;
    if frames == 0 || (frames as f64) < file.frames as f64 * MIN_READ_SHARE {
        return Err(format!(
            "could only read {frames} of the {} frames in {}, so it was left alone",
            file.frames,
            file.path.display()
        ));
    }
    let rate = f64::from(file.sample_rate.max(1));
    let (from, to) = match range {
        Some((a, b)) => ((a * rate) as usize, (b * rate).ceil() as usize),
        None => (0, frames),
    };
    apply(&mut stereo, op, from, to)?;
    let peak = stereo.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let path = reserve(store, &base_stem(&file.path), "wav")?;
    let written = prowave_engine::write_wav(&path, &stereo, file.sample_rate)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
        .and_then(|()| {
            AudioFile::open(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))
        });
    match written {
        Ok(file) => Ok(Done { file, backup, peak }),
        Err(e) => {
            let _ = std::fs::remove_file(&path);
            Err(e)
        }
    }
}

/// Why `op` cannot be done to this file as the project stands, if it cannot. A Flex edit is made
/// from the file, and a take folder's takes are places in it, so neither can have it changed
/// under them.
pub fn refusal(p: &Project, path: &Path, op: Op) -> Option<String> {
    for t in &p.tracks {
        for r in &t.regions {
            let Some(a) = &r.audio else { continue };
            if a.flex.as_ref().is_some_and(|f| f.original.path == path) {
                return Some(format!(
                    "\"{}\" on {} has a Flex edit made from this file: turn Flex off first",
                    r.name, t.name
                ));
            }
            if op.moves_audio() && r.folder.is_some() && a.file.path == path {
                return Some(format!(
                    "\"{}\" on {} is a take folder recorded in this file: flatten it first",
                    r.name, t.name
                ));
            }
        }
    }
    None
}

/// Moves a region's window in its file so that it plays the same audio at the same song time
/// after the part of the file before `kept.0` and from `kept.1` on was removed.
fn follow_trim(p: &Project, start_bar: &mut f32, len_bars: &mut f32, r: &mut (f32, f32), kept: (f64, f64)) {
    let (source_start, source_len) = (f64::from(r.0), f64::from(r.1));
    let plays_to = if r.1 > 0.0 {
        source_start + source_len
    } else {
        f64::INFINITY
    };
    let (first, last) = (source_start.max(kept.0), plays_to.min(kept.1));
    if last <= first {
        // Nothing it played is left: it plays from the start of what is.
        r.0 = 0.0;
        return;
    }
    // Its first audio is removed: the region starts later, so the rest stays where it was.
    let late = first - source_start;
    if late > 0.0 {
        let map = p.tempo_map();
        let begin = p.bar_to_beats(f64::from(*start_bar) - 1.0);
        let end = p.bar_to_beats(f64::from(*start_bar) - 1.0 + f64::from(*len_bars));
        let moved = map.advance(begin, late).min(end);
        let bar = p.beats_to_bar(moved) + 1.0;
        *len_bars = ((p.beats_to_bar(end) + 1.0 - bar) as f32).max(1.0 / 64.0);
        *start_bar = bar as f32;
    }
    r.0 = (first - kept.0) as f32;
    if r.1 > 0.0 {
        r.1 = (last - first) as f32;
    }
}

/// Puts the result of an edit of `old` in place of it, for every region and in the project's list
/// of files. `kept` is what a Trim kept, in seconds; `reversed` that the file now plays backwards.
/// Returns how many regions changed.
pub fn repoint(
    p: &mut Project,
    old: &AudioFile,
    new: &AudioRegion,
    kept: Option<(f64, f64)>,
    reversed: bool,
) -> usize {
    let duration = old.duration_secs();
    let mut changed = 0;
    for ti in 0..p.tracks.len() {
        for ri in 0..p.tracks[ti].regions.len() {
            let r = &p.tracks[ti].regions[ri];
            let Some(a) = r.audio.as_ref().filter(|a| a.file.path == old.path) else {
                continue;
            };
            let follow_bpm = a.follow_bpm;
            let (mut start, mut len) = (r.start_bar, r.len_bars);
            let mut window = (r.params.source_start, r.params.source_len);
            if let Some(kept) = kept {
                follow_trim(p, &mut start, &mut len, &mut window, kept);
            }
            if reversed {
                // The same stretch of audio is now at the other end of the file.
                let plays = if window.1 > 0.0 {
                    f64::from(window.1)
                } else {
                    duration - f64::from(window.0)
                };
                window.0 = (duration - f64::from(window.0) - plays).max(0.0) as f32;
            }
            let r = &mut p.tracks[ti].regions[ri];
            (r.start_bar, r.len_bars) = (start, len);
            (r.params.source_start, r.params.source_len) = window;
            r.audio = Some(AudioRegion {
                follow_bpm,
                ..new.clone()
            });
            changed += 1;
        }
    }
    let listed = p.audio_pool.iter().position(|f| f.path == old.path);
    match listed {
        Some(i) => p.audio_pool[i] = new.file.clone(),
        None => p.audio_pool.push(new.file.clone()),
    }
    p.song_rev += 1;
    changed
}

/// The work in progress.
struct Job {
    rx: Receiver<Result<Done, String>>,
    old: AudioFile,
    kept: Option<(f64, f64)>,
    reversed: bool,
}

/// The editor window.
pub struct FileEditor {
    /// The versions of the file seen since the window opened; the one the project has now is the
    /// one shown, so Undo and Redo take the window along.
    versions: Vec<AudioRegion>,
    /// The selection, in seconds, and where its drag began.
    selection: Option<(f64, f64)>,
    anchor: Option<f64>,
    gain_db: f32,
    target_db: f32,
    job: Option<Job>,
    message: Option<String>,
}

impl Default for FileEditor {
    fn default() -> Self {
        Self {
            versions: Vec::new(),
            selection: None,
            anchor: None,
            gain_db: 0.0,
            target_db: 0.0,
            job: None,
            message: None,
        }
    }
}

impl FileEditor {
    /// Opens the window on a file of the project.
    pub fn open(&mut self, file: AudioRegion) {
        self.versions = vec![file];
        self.selection = None;
        self.anchor = None;
        self.message = None;
    }

    pub fn is_open(&self) -> bool {
        !self.versions.is_empty()
    }

    /// The version the project has now.
    fn current(&self, p: &Project) -> Option<&AudioRegion> {
        self.versions
            .iter()
            .rev()
            .find(|v| p.audio_pool.iter().any(|f| f.path == v.file.path))
    }
}

/// Brings in a finished edit. Call once per frame.
pub fn poll(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    let Some(job) = &s.file_edit.job else {
        return;
    };
    let result = match job.rx.try_recv() {
        Err(TryRecvError::Empty) => {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        Err(TryRecvError::Disconnected) => Err("The edit stopped before it finished".to_string()),
        Ok(result) => result,
    };
    let Some(job) = s.file_edit.job.take() else {
        return;
    };
    match result {
        Ok(done) => {
            let new = crate::import::audio_region(done.file.clone(), ctx);
            repoint(p, &job.old, &new, job.kept, job.reversed);
            s.file_edit.versions.push(new);
            s.file_edit.selection = None;
            s.file_edit.message = Some(if done.peak > 1.0 {
                format!(
                    "Done. It now peaks {:.1} dB over full scale: lower it before it reaches an output.",
                    20.0 * done.peak.log10()
                )
            } else {
                format!("Done. The earlier version is kept at {}", done.backup.display())
            });
        }
        Err(e) => s.file_edit.message = Some(e),
    }
}

/// Starts an edit of the file on show, off the UI thread.
fn start(p: &Project, v: &mut FileEditor, op: Op) {
    let Some(file) = v.current(p).map(|c| c.file.clone()) else {
        return;
    };
    if let Some(why) = refusal(p, &file.path, op) {
        v.message = Some(why);
        return;
    }
    let Some(store) = store_dir() else {
        v.message = Some("There is no folder to keep the backup in".to_string());
        return;
    };
    let range = v.selection;
    let kept = (op == Op::Trim).then_some(range).flatten();
    let reversed = op == Op::Reverse && range.is_none();
    let (tx, rx) = channel();
    let old = file.clone();
    let spawned = std::thread::Builder::new()
        .name("prowave-file-edit".into())
        .spawn(move || {
            let _ = tx.send(run_edit(&file, op, range, &store));
        });
    match spawned {
        Ok(_) => {
            v.message = Some("Working…".to_string());
            v.job = Some(Job {
                rx,
                old,
                kept,
                reversed,
            });
        }
        Err(e) => v.message = Some(format!("cannot start the edit: {e}")),
    }
}

/// The waveform, in which a drag makes the selection.
fn waveform(ui: &mut egui::Ui, v: &mut FileEditor, file: &AudioRegion) {
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 170.0), egui::Sense::drag());
    let g = ui.painter_at(rect);
    g.rect_filled(rect, 2.0, egui::Color32::from_gray(0x1a));
    let seconds = file.file.duration_secs().max(1e-6);
    let x_of = |t: f64| rect.left() + (t / seconds) as f32 * rect.width();
    let time_at = |x: f32| (f64::from((x - rect.left()) / rect.width()) * seconds).clamp(0.0, seconds);
    let (mid, half) = (rect.center().y, rect.height() * 0.48);
    if let Some(peaks) = file.peaks.get() {
        let columns = rect.width().max(1.0) as usize;
        for c in 0..columns {
            let at = |k: usize| k * peaks.len() / columns;
            let bins = &peaks[at(c)..at(c + 1).max(at(c) + 1).min(peaks.len())];
            let lo = bins.iter().fold(0.0f32, |m, b| m.min(b.0)).max(-1.0);
            let hi = bins.iter().fold(0.0f32, |m, b| m.max(b.1)).min(1.0);
            let x = rect.left() + c as f32 + 0.5;
            g.line_segment(
                [egui::pos2(x, mid - hi * half), egui::pos2(x, mid - lo * half)],
                egui::Stroke::new(1.0, egui::Color32::from_rgb(0x7a, 0xb8, 0xe8)),
            );
        }
    } else {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }
    if resp.drag_started() {
        v.anchor = resp.interact_pointer_pos().map(|pt| time_at(pt.x));
        v.selection = None;
    }
    if let (Some(anchor), Some(pt)) = (v.anchor, resp.interact_pointer_pos()) {
        let now = time_at(pt.x);
        if (now - anchor).abs() > seconds / 4000.0 {
            v.selection = Some((anchor.min(now), anchor.max(now)));
        }
    }
    if resp.drag_stopped() {
        v.anchor = None;
    }
    if let Some((a, b)) = v.selection {
        let band = egui::Rect::from_min_max(
            egui::pos2(x_of(a), rect.top()),
            egui::pos2(x_of(b), rect.bottom()),
        );
        g.rect_filled(band, 0.0, egui::Color32::from_white_alpha(40));
    }
}

/// The window. Call once per frame.
pub fn show(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    if !s.file_edit.is_open() {
        return;
    }
    // Everything it showed has gone (a project was closed, or Undo took it out).
    let Some(file) = s.file_edit.current(p).cloned() else {
        s.file_edit.versions.clear();
        return;
    };
    let path = file.file.path.clone();
    let busy = s.file_edit.job.is_some();
    let uses = p
        .tracks
        .iter()
        .flat_map(|t| &t.regions)
        .filter_map(|r| r.audio.as_ref())
        .filter(|a| a.file.path == path)
        .count();
    let (mut open, can_undo) = (true, s.can_undo);
    let (mut undo, mut preview, mut reveal, mut op) = (false, false, false, None);
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    egui::Window::new(format!("Audio File Editor: {name}"))
        .id(egui::Id::unique("audio_file_editor"))
        .open(&mut open)
        .default_size([760.0, 340.0])
        .show(ctx, |ui| {
            let v = &mut s.file_edit;
            let f = &file.file;
            ui.label(format!(
                "{} Hz, {}, {:.3} seconds. {uses} region{} play it.",
                f.sample_rate,
                if f.channels == 1 { "mono" } else { "stereo" },
                f.duration_secs(),
                if uses == 1 { "" } else { "s" },
            ));
            ui.small(
                "These edits change the sound for every region that plays this file. \
                 The file you imported stays as it is on disk: a backup is made, and the \
                 result is a new file (32-bit float WAV, stereo). Ctrl+Z undoes an edit.",
            );
            waveform(ui, v, &file);
            match v.selection {
                Some((a, b)) => ui.label(format!(
                    "Selection: {a:.3} to {b:.3} seconds ({:.3}). Edits work on this.",
                    b - a
                )),
                None => ui.label("Nothing selected: edits work on the whole file. Drag to select."),
            };
            ui.add_enabled_ui(!busy, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut v.target_db)
                            .range(-60.0..=0.0)
                            .speed(0.1)
                            .suffix(" dBFS"),
                    );
                    if ui.button("Normalize").clicked() {
                        op = Some(Op::Normalize {
                            target_db: v.target_db,
                        });
                    }
                    ui.separator();
                    ui.add(
                        egui::DragValue::new(&mut v.gain_db)
                            .range(-60.0..=24.0)
                            .speed(0.1)
                            .suffix(" dB"),
                    );
                    if ui.button("Change Gain").clicked() {
                        op = Some(Op::Gain { db: v.gain_db });
                    }
                    ui.separator();
                    for (text, choice) in [
                        ("Fade In", Op::FadeIn),
                        ("Fade Out", Op::FadeOut),
                        ("Silence", Op::Silence),
                        ("Reverse", Op::Reverse),
                    ] {
                        if ui.button(text).clicked() {
                            op = Some(choice);
                        }
                    }
                    if ui
                        .add_enabled(v.selection.is_some(), egui::Button::new("Trim to Selection"))
                        .on_disabled_hover_text("Drag over the part to keep first")
                        .clicked()
                    {
                        op = Some(Op::Trim);
                    }
                });
            });
            ui.horizontal(|ui| {
                undo = ui
                    .add_enabled(can_undo && !busy, egui::Button::new("Undo"))
                    .clicked();
                preview = ui.button("Preview").clicked();
                reveal = ui.button("Show Backups Folder").clicked();
            });
            if let Some(message) = &v.message {
                ui.label(message.as_str());
            }
        });
    if let Some(op) = op {
        start(p, &mut s.file_edit, op);
    }
    if undo {
        s.undo_cmd = Some(crate::undo::UndoCmd::Undo);
    }
    if preview {
        s.samples.toggle_preview(path);
    }
    if reveal {
        if let Some(dir) = store_dir() {
            let _ = std::fs::create_dir_all(&dir);
            if let Err(e) = crate::file_commands::reveal_in_file_manager(&dir) {
                s.file_edit.message = Some(e.to_string());
            }
        }
    }
    if !open {
        s.file_edit.versions.clear();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::model::TrackKind;

    fn ramp() -> Vec<f32> {
        vec![0.1, -0.1, 0.2, -0.2, 0.3, -0.3, 0.4, -0.4]
    }

    fn close(a: &[f32], b: &[f32]) {
        assert_eq!(a.len(), b.len(), "{a:?} vs {b:?}");
        for (x, y) in a.iter().zip(b) {
            assert!((x - y).abs() < 1e-4, "{a:?} vs {b:?}");
        }
    }

    #[test]
    fn each_edit_does_what_it_says_only_where_it_is_asked_to() {
        let mut a = ramp();
        apply(&mut a, Op::Normalize { target_db: 0.0 }, 0, 4).unwrap();
        close(&a, &[0.25, -0.25, 0.5, -0.5, 0.75, -0.75, 1.0, -1.0]);

        let mut a = ramp();
        apply(&mut a, Op::Normalize { target_db: 0.0 }, 0, 1).unwrap();
        close(&a, &[1.0, -1.0, 0.2, -0.2, 0.3, -0.3, 0.4, -0.4]);

        let mut a = ramp();
        apply(&mut a, Op::Gain { db: 6.0206 }, 2, 4).unwrap();
        close(&a, &[0.1, -0.1, 0.2, -0.2, 0.6, -0.6, 0.8, -0.8]);

        let mut a = ramp();
        apply(&mut a, Op::Silence, 1, 3).unwrap();
        close(&a, &[0.1, -0.1, 0.0, 0.0, 0.0, 0.0, 0.4, -0.4]);

        let mut a = ramp();
        apply(&mut a, Op::Reverse, 0, 4).unwrap();
        close(&a, &[0.4, -0.4, 0.3, -0.3, 0.2, -0.2, 0.1, -0.1]);

        let mut a = ramp();
        apply(&mut a, Op::Trim, 1, 3).unwrap();
        close(&a, &[0.2, -0.2, 0.3, -0.3]);

        let mut a = vec![1.0; 8];
        apply(&mut a, Op::FadeIn, 0, 4).unwrap();
        assert_eq!(a[0], 0.0);
        assert_eq!(a[6], 1.0);
        let mut a = vec![1.0; 8];
        apply(&mut a, Op::FadeOut, 0, 4).unwrap();
        assert_eq!(a[0], 1.0);
        assert_eq!(a[6], 0.0);
    }

    #[test]
    fn what_cannot_be_done_says_so_and_changes_nothing() {
        let mut a = vec![0.0; 8];
        assert!(apply(&mut a, Op::Normalize { target_db: 0.0 }, 0, 4).is_err());
        let mut a = ramp();
        assert!(apply(&mut a, Op::Trim, 3, 3).is_err());
        assert!(apply(&mut a, Op::Silence, 9, 12).is_err());
        close(&a, &ramp());
    }

    /// A one-second stereo file with a known sound.
    fn source(dir: &Path, name: &str) -> AudioFile {
        let path = dir.join(name);
        let stereo: Vec<f32> = (0..48_000).flat_map(|i| [i as f32 / 96_000.0, 0.1]).collect();
        prowave_engine::write_wav(&path, &stereo, 48_000).unwrap();
        AudioFile::open(&path).unwrap()
    }

    #[test]
    fn the_original_is_never_written_to_and_a_checked_copy_comes_first() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("store");
        let file = source(dir.path(), "kick.wav");
        let before = std::fs::read(&file.path).unwrap();

        let done = run_edit(&file, Op::Silence, None, &store).unwrap();
        assert_eq!(std::fs::read(&file.path).unwrap(), before, "untouched");
        assert_eq!(std::fs::read(&done.backup).unwrap(), before, "the backup");
        assert!(done.backup.starts_with(store.join("backups")));
        assert_ne!(done.file.path, file.path);
        let sound = done.file.read_stereo().unwrap();
        assert!(sound.iter().all(|s| *s == 0.0));
        assert_eq!(sound.len(), 96_000);
        assert_eq!(done.file.sample_rate, 48_000);

        // A version made here is not copied again: nothing ever writes to it.
        let again = run_edit(&done.file, Op::Reverse, None, &store).unwrap();
        assert_eq!(again.backup, done.file.path);
        assert_ne!(again.file.path, done.file.path);
        assert!(
            again.file.path.to_string_lossy().contains("kick edit 2"),
            "{}",
            again.file.path.display()
        );
    }

    #[test]
    fn no_backup_means_no_edit() {
        let dir = tempfile::tempdir().unwrap();
        let file = source(dir.path(), "kick.wav");
        let before = std::fs::read(&file.path).unwrap();
        // The store's place is taken by a file, so nothing can be written there.
        let blocked = dir.path().join("store");
        std::fs::write(&blocked, b"not a folder").unwrap();
        assert!(run_edit(&file, Op::Silence, None, &blocked).is_err());
        assert_eq!(std::fs::read(&file.path).unwrap(), before);
    }

    #[test]
    fn a_mono_file_comes_out_stereo_with_the_same_length() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("mono.wav");
        crate::project_io::tests::write_wav(&wav);
        let file = AudioFile::open(&wav).unwrap();
        assert_eq!(file.channels, 1);
        let done = run_edit(&file, Op::Gain { db: -6.0 }, None, &dir.path().join("s")).unwrap();
        assert_eq!(done.file.channels, 2);
        assert_eq!(done.file.frames, file.frames);
    }

    /// A project with `file` in a region at bar 3 of an audio track.
    fn project_with(file: &AudioFile, ctx: &egui::Context) -> (Project, usize) {
        let mut p = Project::empty();
        let t = p.add_track(TrackKind::Audio);
        crate::import::import_audio_at(&mut p, &file.path, ctx, 3.0, Some(t)).unwrap();
        crate::project_audio::sync_pool(&mut p);
        (p, t)
    }

    #[test]
    fn undo_and_redo_of_an_edit_put_the_regions_back_and_forward() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let file = source(dir.path(), "kick.wav");
        let (mut p, t) = project_with(&file, &ctx);
        let mut history = crate::undo::History::new(&p);
        let done = run_edit(&file, Op::Reverse, None, &dir.path().join("s")).unwrap();
        let new = crate::import::audio_region(done.file.clone(), &ctx);
        assert_eq!(repoint(&mut p, &file, &new, None, true), 1);
        let path_of = |p: &Project| p.tracks[t].regions[0].audio.as_ref().unwrap().file.path.clone();
        assert_eq!(path_of(&p), done.file.path);
        assert!(history.record(&p), "an edit is a step of its own");
        assert!(history.undo(&mut p));
        assert_eq!(path_of(&p), file.path);
        assert!(p.audio_pool.iter().any(|f| f.path == file.path));
        assert!(history.redo(&mut p));
        assert_eq!(path_of(&p), done.file.path);
    }

    #[test]
    fn a_trim_leaves_the_rest_of_the_audio_where_it_was_in_the_song() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let file = source(dir.path(), "kick.wav");
        let (mut p, t) = project_with(&file, &ctx);
        p.tempo = 120.0;
        let before = p.region_start_beats(&p.tracks[t].regions[0]);
        let done = run_edit(&file, Op::Trim, Some((0.5, 0.75)), &dir.path().join("s")).unwrap();
        assert!((done.file.duration_secs() - 0.25).abs() < 0.001);
        let new = crate::import::audio_region(done.file.clone(), &ctx);
        repoint(&mut p, &file, &new, Some((0.5, 0.75)), false);
        let r = &p.tracks[t].regions[0];
        // Half a second at 120 BPM is one beat.
        assert!((p.region_start_beats(r) - (before + 1.0)).abs() < 0.01);
        assert_eq!(r.params.source_start, 0.0);
    }

    #[test]
    fn a_reversed_window_plays_the_same_stretch_from_the_other_end() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let file = source(dir.path(), "kick.wav");
        let (mut p, t) = project_with(&file, &ctx);
        p.tracks[t].regions[0].params.source_start = 0.25;
        p.tracks[t].regions[0].params.source_len = 0.25;
        let new = crate::import::audio_region(file.clone(), &ctx);
        repoint(&mut p, &file, &new, None, true);
        let params = &p.tracks[t].regions[0].params;
        assert!((params.source_start - 0.5).abs() < 1e-4);
        assert!((params.source_len - 0.25).abs() < 1e-4);
    }

    #[test]
    fn flex_edits_and_take_folders_stop_what_would_break_them() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let file = source(dir.path(), "kick.wav");
        let (mut p, t) = project_with(&file, &ctx);
        assert_eq!(refusal(&p, &file.path, Op::Trim), None);
        p.tracks[t].regions[0].folder = Some(crate::model::TakeFolder {
            takes: Vec::new(),
            comp: Vec::new(),
            len: 1.0,
            open: false,
        });
        assert!(refusal(&p, &file.path, Op::Trim).is_some());
        assert!(refusal(&p, &file.path, Op::Silence).is_none(), "time stays");
        p.tracks[t].regions[0].folder = None;
        p.tracks[t].regions[0].audio.as_mut().unwrap().flex = Some(
            crate::flex_edit::FlexEdit::new(file.clone(), prowave_engine::FlexMode::default()),
        );
        assert!(refusal(&p, &file.path, Op::Silence).is_some());
    }

    #[test]
    fn names_stay_short_of_trouble() {
        assert_eq!(base_stem(Path::new("a/kick edit 12.wav")), "kick");
        assert_eq!(base_stem(Path::new("a/my edit one.wav")), "my edit one");
        assert_eq!(safe_name("a:b?"), "a_b_");
        assert_eq!(safe_name("..."), "audio");
    }
}
