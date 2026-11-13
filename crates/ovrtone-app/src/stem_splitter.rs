//! Mix > Stem Splitter: splits the selected audio region into drums, bass, vocals and the rest, each
//! on a new track in the same place, with the original muted. The work is done by a copy of this
//! program started for it (see `stem_worker.rs`), with the model and ONNX Runtime from the `stems`
//! folder next to Ovrtone. It is its own process, so a failure in it costs only the split.
use crate::icons::{self, Icon};
use crate::model::{Project, Track, TrackKind};
use crate::UiState;
use egui::{pos2, vec2, Color32, Sense};
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
    progress: Arc<Mutex<Progress>>,
    stop: Arc<AtomicBool>,
}

/// How far the helper is: it works through a song in overlapping parts and reports each.
#[derive(Clone, Copy, Default)]
struct Progress {
    done: usize,
    total: usize,
    /// When the work on the parts began, as near as is known, and when the last part was done.
    began: Option<Instant>,
    last: Option<Instant>,
}

impl Progress {
    /// The time one part takes, from those done so far.
    fn per_part(&self) -> Option<Duration> {
        let (began, last) = (self.began?, self.last?);
        (self.done > 0).then(|| last.duration_since(began) / self.done as u32)
    }

    /// How much of the split is done, 0..=1. Between reports it moves on by how long a part takes,
    /// and waits short of the next report if that is late.
    fn fraction(&self, now: Instant) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        let into_part = match (self.per_part(), self.last) {
            (Some(per), Some(last)) if !per.is_zero() => {
                (now.duration_since(last).as_secs_f32() / per.as_secs_f32()).min(0.95)
            }
            _ => 0.0,
        };
        ((self.done as f32 + into_part) / self.total as f32).min(1.0)
    }

    /// The time left, once a part has been timed.
    fn left(&self, now: Instant) -> Option<Duration> {
        let per = self.per_part()?;
        let parts = self.total as f32 * (1.0 - self.fraction(now));
        Some(per.mul_f32(parts.max(0.0)))
    }

    /// A report such as `Separated chunk 3/40`.
    fn read(&mut self, message: &str, now: Instant) {
        let parts = message
            .strip_prefix("Separated chunk ")
            .and_then(|m| m.split_once('/'));
        if let Some((done, total)) = parts {
            if let (Ok(done), Ok(total)) = (done.trim().parse(), total.trim().parse()) {
                self.done = done;
                self.total = total;
                self.last = Some(now);
            }
        }
    }
}

/// The words the helper prints, and what to tell the user.
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

/// A row of the window: its name, the worker's stem, its picture and colour.
struct Row {
    name: &'static str,
    stem: &'static str,
    icon: Icon,
    color: Color32,
}

const fn row(name: &'static str, stem: &'static str, icon: Icon, color: Color32) -> Row {
    Row {
        name,
        stem,
        icon,
        color,
    }
}

const DRUMS: usize = 1;
const GUITAR: usize = 3;
const PIANO: usize = 4;
const MAIN: [Row; 6] = [
    row("Vocals", "vocals", Icon::Mic, Color32::from_rgb(0x1f, 0xb8, 0xa6)),
    row("Drums", "drums", Icon::Drum, Color32::from_rgb(0x1f, 0xa8, 0xc4)),
    row("Bass", "bass", Icon::Wave, Color32::from_rgb(0x25, 0x95, 0xd8)),
    row("Guitar", "guitar", Icon::Pick, Color32::from_rgb(0x2f, 0x80, 0xe0)),
    row("Piano", "piano", Icon::Piano, Color32::from_rgb(0x46, 0x6c, 0xe6)),
    row("Other", "other", Icon::Other, Color32::from_rgb(0x62, 0x58, 0xe0)),
];
/// The pieces of the kit, as the model gives them. It has no hi-hat of its own.
const KIT: [(&str, &str); 4] = [
    ("Kick", "kick"),
    ("Snare", "snare"),
    ("Cymbals and hi-hats", "cymbals"),
    ("Toms", "toms"),
];

/// What is asked of a split.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Choice {
    main: [bool; 6],
    /// The drums come as the pieces of the kit.
    kit_on: bool,
    kit: [bool; 4],
    /// All that is left out goes in one more file.
    submix: bool,
}

impl Default for Choice {
    fn default() -> Self {
        Self {
            main: [true; 6],
            kit_on: false,
            kit: [true; 4],
            submix: false,
        }
    }
}

/// The ready-made choices: a name, and what it picks.
const PRESETS: [(&str, Choice); 5] = [
    ("None", Choice { main: [true; 6], kit_on: false, kit: [true; 4], submix: false }),
    ("Vocals and the rest", Choice { main: [true, false, false, false, false, false], kit_on: false, kit: [true; 4], submix: true }),
    ("Four stems", Choice { main: [true, true, true, false, false, true], kit_on: false, kit: [true; 4], submix: false }),
    ("Six stems", Choice { main: [true; 6], kit_on: false, kit: [true; 4], submix: false }),
    ("Every kit piece", Choice { main: [true; 6], kit_on: true, kit: [true; 4], submix: false }),
];

impl Choice {
    /// The stems of this choice in the order they go on tracks: the worker's name, the track's, and
    /// whether it is a piece of the kit.
    fn stems(&self) -> Vec<(&'static str, &'static str, bool)> {
        let mut stems = Vec::new();
        for (k, row) in MAIN.iter().enumerate() {
            if k == DRUMS && self.kit_on {
                for (piece, on) in KIT.iter().zip(self.kit) {
                    if on {
                        stems.push((piece.1, piece.0, true));
                    }
                }
            } else if self.main[k] {
                stems.push((row.stem, row.name, false));
            }
        }
        if self.submix {
            stems.push((crate::stem_worker::SUBMIX, "Submix", false));
        }
        stems
    }

    /// Whether the stems hold all of the song, so that the original can go quiet. Other holds the
    /// guitar and piano when neither is asked for.
    fn covers_all(&self) -> bool {
        let drums = if self.kit_on {
            self.kit == [true; 4]
        } else {
            self.main[DRUMS]
        };
        self.submix
            || (self.main[0]
                && drums
                && self.main[2]
                && self.main[5]
                && self.main[GUITAR] == self.main[PIANO])
    }
}

/// What a split that was stopped on purpose reports. Not shown as an error.
const STOPPED: &str = "stopped";

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

impl StemSplitter {
    /// While a split is running nothing plays: it has the use of every core, and sound would break
    /// up. Call before the transport is told what the project wants.
    pub fn hold_playback(&self, p: &mut Project) {
        if self.job.is_some() {
            p.playing = false;
            p.recording = false;
        }
    }
}

pub struct StemSplitter {
    chosen: Choice,
    /// The ready-made choice in force, 0 for none.
    preset: usize,
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
            chosen: Choice::default(),
            preset: 0,
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

/// Where the helper and the model are: `stems` beside the program, or `OVRTONE_STEMS_DIR`.
fn helper_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("OVRTONE_STEMS_DIR") {
        return Some(dir.into());
    }
    Some(std::env::current_exe().ok()?.parent()?.join("stems"))
}

/// Runs a copy of `exe` on `input`, writing the stems to `out`. On its own thread: it takes minutes.
fn split(
    exe: &Path,
    dir: &Path,
    input: &Path,
    out: &Path,
    stems: &[&str],
    control: &JobControl,
) -> Result<(), String> {
    let mut command = std::process::Command::new(exe);
    command.args(crate::stem_worker::arguments(dir, input, out, stems));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start the splitter: {e}"))?;
    // While this is open the copy goes on. If this program goes, so does the copy.
    let _keep_going = child.stdin.take();
    let reports = child.stdout.take().map(|stdout| {
        let control = control.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                read_report(&line, &control);
            }
        })
    });
    let errors = child.stderr.take().map(|stderr| {
        std::thread::spawn(move || {
            let mut last = String::new();
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if !line.trim().is_empty() {
                    last = line;
                }
            }
            last
        })
    });
    loop {
        if control.stop.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_dir_all(out);
            return Err(STOPPED.to_string());
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if let Some(reports) = reports {
                    let _ = reports.join();
                }
                if status.success() {
                    return Ok(());
                }
                let last = errors.and_then(|e| e.join().ok()).unwrap_or_default();
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

/// Takes in a line from the copy doing the work.
fn read_report(line: &str, control: &JobControl) {
    if let (Some(now), Ok(mut stage)) = (stage_of(line), control.stage.lock()) {
        *stage = now.to_string();
    }
    if let Ok(mut progress) = control.progress.lock() {
        let now = Instant::now();
        // The time for a part is timed from here: loading the model is not part of it.
        if line.starts_with("Separating") {
            progress.began = Some(now);
        }
        progress.read(line, now);
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

/// Puts the stems in `dir` on new tracks, playing what the source region plays, and mutes it if they
/// hold all of it. The pieces of the kit go in a stack of their own.
fn place_stems(
    p: &mut Project,
    ctx: &egui::Context,
    source: &Source,
    dir: &Path,
    chosen: &Choice,
) -> Result<(), String> {
    let mut stack = false;
    for (stem, title, of_kit) in chosen.stems() {
        if of_kit && !stack {
            stack = true;
            let mut folder = Track::new(format!("{} Drums", source.name), TrackKind::Folder);
            folder.color = p.next_track_color();
            p.tracks.push(folder);
        }
        let path = dir.join(format!("{stem}.wav"));
        crate::import::import_audio_at(p, &path, ctx, f64::from(source.start_bar), None)
            .map_err(|e| format!("{stem}: {e}"))?;
        if let Some(track) = p.tracks.last_mut() {
            track.name = format!("{} {title}", source.name);
            track.indent = u8::from(of_kit);
            if let Some(region) = track.regions.last_mut() {
                region.name.clone_from(&track.name);
                region.len_bars = source.len_bars;
                region.params.source_start = source.source_start;
                region.params.source_len = source.source_len;
            }
        }
    }
    // With only part split out, the original goes on playing: nothing else has the rest.
    if !chosen.covers_all() {
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

/// A switch that slides.
fn switch(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let (r, mut resp) = ui.allocate_exact_size(vec2(30.0, 16.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let g = ui.painter();
    let track = if *on {
        crate::theme::accent()
    } else {
        crate::theme::gray(0x50)
    };
    g.rect_filled(r, 8.0, track);
    let x = if *on { r.right() - 8.0 } else { r.left() + 8.0 };
    g.circle_filled(pos2(x, r.center().y), 6.0, Color32::WHITE);
    resp
}

/// The left of a row: its picture on a coloured tile, and its name. Reserves the row.
fn row_start(ui: &mut egui::Ui, name: &str, tile: Option<(Icon, Color32)>, indent: f32) {
    if let Some((icon, color)) = tile {
        let (r, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
        ui.painter().rect_filled(r, 6.0, color);
        icons::paint(ui.painter(), r.center(), 17.0, icon, Color32::WHITE);
    } else {
        ui.add_space(indent);
    }
    ui.add_space(6.0);
    ui.label(name);
}

/// The rows, in a panel of their own. True if the user changed any.
fn rows(ui: &mut egui::Ui, c: &mut Choice, six_there: bool, kit_there: bool) -> bool {
    let mut changed = false;
    egui::Frame::new()
        .fill(crate::theme::gray(0x22))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            for (k, row) in MAIN.iter().enumerate() {
                let there = six_there || (k != GUITAR && k != PIANO);
                ui.add_enabled_ui(there, |ui| {
                    ui.horizontal(|ui| {
                        ui.set_min_height(28.0);
                        let name = if k == DRUMS && c.kit_on {
                            let n = c.kit.iter().filter(|on| **on).count();
                            format!("{} ({n})", row.name)
                        } else {
                            row.name.to_string()
                        };
                        row_start(ui, &name, Some((row.icon, row.color)), 0.0);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if k == DRUMS && kit_there {
                                if !c.kit_on {
                                    changed |= ui.checkbox(&mut c.main[k], "").changed();
                                }
                                changed |= switch(ui, &mut c.kit_on)
                                    .on_hover_text("Split the drums again, into the pieces of the kit")
                                    .changed();
                                ui.label("Split the kit");
                            } else {
                                changed |= ui.checkbox(&mut c.main[k], "").changed();
                            }
                        });
                    });
                    if !there {
                        c.main[k] = false;
                    }
                });
                if k == DRUMS && c.kit_on {
                    for (piece, on) in KIT.iter().zip(&mut c.kit) {
                        ui.horizontal(|ui| {
                            ui.set_min_height(22.0);
                            row_start(ui, piece.0, None, 34.0);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| changed |= ui.checkbox(on, "").changed(),
                            );
                        });
                    }
                }
            }
        });
    ui.add_space(4.0);
    egui::Frame::new()
        .fill(crate::theme::gray(0x22))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_min_height(28.0);
                row_start(ui, "Submix of the rest", Some((Icon::Stack, crate::theme::gray(0x60))), 0.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    changed |= ui
                        .checkbox(&mut c.submix, "")
                        .on_hover_text("One more track, with everything not ticked above")
                        .changed();
                });
            });
        });
    changed
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
                match place_stems(p, ctx, &source, &dir, &s.stems.chosen) {
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
    let has = |name: &str| dir.as_ref().is_some_and(|d| d.join(name).exists());
    let ready = has(crate::stem_worker::RUNTIME) && has(crate::stem_worker::MODEL);
    let (six_there, kit_there) = (
        has(crate::stem_worker::MODEL_SIX),
        has(crate::stem_worker::MODEL_KIT),
    );
    if !kit_there {
        s.stems.chosen.kit_on = false;
    }
    let (busy, mut start) = (s.stems.job.is_some(), false);
    let mut open = true;
    let mut cancel = false;
    egui::Window::new("Stem Splitter")
        .id(egui::Id::unique("stem_splitter"))
        .open(&mut open)
        .resizable(false)
        .default_width(380.0)
        .show(ctx, |ui| {
            ui.set_width(380.0);
            ui.add_enabled_ui(!busy, |ui| {
                egui::Frame::new()
                    .fill(crate::theme::gray(0x22))
                    .corner_radius(8.0)
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Preset");
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let now = s.stems.preset.min(PRESETS.len() - 1);
                                egui::ComboBox::from_id_salt(egui::Id::unique("stem_preset"))
                                    .selected_text(PRESETS[now].0)
                                    .show_ui(ui, |ui| {
                                        for (k, preset) in PRESETS.iter().enumerate() {
                                            if ui.selectable_label(now == k, preset.0).clicked() {
                                                s.stems.preset = k;
                                                s.stems.chosen = preset.1;
                                            }
                                        }
                                    });
                            });
                        });
                    });
                ui.add_space(4.0);
                if rows(ui, &mut s.stems.chosen, six_there, kit_there) {
                    s.stems.preset = 0;
                }
            });
            ui.add_space(4.0);
            if !ready {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "The splitter is not installed: the stems folder is missing ONNX Runtime or the model.",
                );
            }
            if let Some(e) = &s.stems.error {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            if busy {
                let stage = s
                    .stems
                    .control
                    .stage
                    .lock()
                    .map(|t| t.clone())
                    .unwrap_or_default();
                let elapsed = s.stems.began.map_or(0, |b| b.elapsed().as_secs());
                let now = Instant::now();
                let progress = s
                    .stems
                    .control
                    .progress
                    .lock()
                    .map(|p| *p)
                    .unwrap_or_default();
                let fraction = progress.fraction(now);
                let bar = egui::ProgressBar::new(fraction)
                    .desired_width(380.0)
                    .animate(true)
                    .text(format!("{:.0}%", fraction * 100.0));
                ui.add(bar);
                let shown = if progress.total > 0 {
                    format!("Part {} of {}", progress.done, progress.total)
                } else if stage.is_empty() {
                    "Starting".to_string()
                } else {
                    stage
                };
                let left = progress.left(now).map_or(String::new(), |left| {
                    let minutes = left.as_secs().div_ceil(60);
                    format!(", about {minutes} min left")
                });
                ui.label(format!(
                    "{shown}: {}:{:02}{left}",
                    elapsed / 60,
                    elapsed % 60
                ));
                ui.label("Playback is paused until the split is done.");
            }
            ui.add_space(2.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let can = ready && !busy && !s.stems.chosen.stems().is_empty();
                let apply = egui::Button::new(
                    egui::RichText::new("Split").color(crate::theme::on_accent()),
                )
                .fill(crate::theme::accent());
                if ui.add_enabled(can, apply).clicked() {
                    start = true;
                }
                let word = if busy { "Stop" } else { "Cancel" };
                if ui.button(word).clicked() {
                    if busy {
                        s.stems.control.stop.store(true, Ordering::Relaxed);
                    } else {
                        cancel = true;
                    }
                }
            });
        });
    if cancel {
        open = false;
    }
    if let (true, Some(dir)) = (start, dir) {
        let (tx, rx) = channel();
        let (input, out) = (source.path.clone(), new_output_dir(&source.name));
        let stems = s.stems.chosen.stems();
        let control = JobControl::default();
        s.stems.control = control.clone();
        s.stems.began = Some(Instant::now());
        s.stems.thread = Some(std::thread::spawn(move || {
            let names: Vec<&str> = stems.iter().map(|(stem, ..)| *stem).collect();
            let result = std::env::current_exe()
                .map_err(|e| e.to_string())
                .and_then(|exe| split(&exe, &dir, &input, &out, &names, &control));
            let _ = tx.send(result.map(|()| out));
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
    fn progress_goes_by_the_parts_reported() {
        let began = Instant::now();
        let mut progress = Progress {
            began: Some(began),
            ..Progress::default()
        };
        assert_eq!(progress.fraction(began), 0.0);
        let at = began + Duration::from_secs(10);
        progress.read("Separated chunk 1/4", at);
        assert!((progress.fraction(at) - 0.25).abs() < 1e-4);
        assert_eq!(progress.left(at), Some(Duration::from_secs(30)));
        // Halfway through the next part, and waiting short of the report if it is late.
        assert!((progress.fraction(at + Duration::from_secs(5)) - 0.375).abs() < 1e-4);
        assert!(progress.fraction(at + Duration::from_secs(60)) < 0.5);
        progress.read("Loading model...", at);
        assert_eq!(progress.done, 1);
    }

    /// A real split, by `OVRTONE_TEST_EXE` with `OVRTONE_STEMS_DIR` on `OVRTONE_TEST_STEMS_WAV`.
    #[test]
    #[ignore = "takes minutes and needs the helper"]
    fn the_helper_splits_and_reports_its_progress() {
        let exe = PathBuf::from(std::env::var("OVRTONE_TEST_EXE").unwrap());
        let dir = helper_dir().unwrap();
        let input = PathBuf::from(std::env::var("OVRTONE_TEST_STEMS_WAV").unwrap());
        let out = tempfile::tempdir().unwrap();
        let control = JobControl::default();
        split(&exe, &dir, &input, out.path(), &["vocals"], &control).unwrap();
        assert!(out.path().join("vocals.wav").exists());
        assert!(!out.path().join("drums.wav").exists());
        let progress = *control.progress.lock().unwrap();
        assert!(progress.total > 0 && progress.done == progress.total);
    }

    #[test]
    fn stems_land_on_new_tracks_in_place_and_the_original_is_muted() {
        let dir = tempfile::tempdir().unwrap();
        for (stem, ..) in Choice::default().stems() {
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
        place_stems(&mut p, &ctx, &source, dir.path(), &Choice::default()).unwrap();
        assert_eq!(p.tracks.len(), tracks + 6);
        assert!(p.tracks[t].regions[0].params.mute);
        let vocals = &p.tracks[tracks];
        assert_eq!(vocals.name, "song Vocals");
        assert_eq!(vocals.kind, TrackKind::Audio);
        assert_eq!(vocals.regions[0].start_bar, 3.0);
        assert!(!vocals.regions[0].params.mute);
    }

    #[test]
    fn the_kit_pieces_stand_in_for_the_drums_and_the_original_goes_quiet_only_when_all_is_there() {
        let mut c = Choice {
            kit_on: true,
            ..Choice::default()
        };
        let names: Vec<&str> = c.stems().iter().map(|s| s.0).collect();
        assert_eq!(
            names,
            ["vocals", "kick", "snare", "cymbals", "toms", "bass", "guitar", "piano", "other"]
        );
        assert!(c.covers_all());
        c.kit[0] = false;
        assert!(!c.covers_all(), "no kick anywhere");
        c.submix = true;
        assert!(c.covers_all(), "the kick is in the submix");
        // Without guitar and piano, the four stem model keeps them in Other.
        let four = PRESETS[2].1;
        assert!(four.covers_all());
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
        assert!(place_stems(&mut p, &ctx, &source, dir.path(), &Choice::default()).is_err());
        assert!(!p.tracks[t].regions[0].params.mute);
    }
}
