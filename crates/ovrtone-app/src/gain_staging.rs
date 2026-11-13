//! Auto Gain Staging: measures the audio of the session, works out how far each track or clip is
//! from a level the user sets, shows what it would change, and applies it only when told to, as a
//! track's input trim (before its effects, the fader untouched) or as clip gain. Nothing is
//! written to the audio files, faders, pans, automation, plugins and routing are left alone, and
//! Undo takes it back.
//!
//! The measuring reads files on a thread of its own, never on the audio thread.
use crate::model::{Project, TrackKind, MAX_REGION_GAIN, MIN_REGION_GAIN};
use crate::UiState;
use egui::RichText;
use ovrtone_engine::AudioFile;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;

/// The furthest a track's input trim goes, either way, in dB.
pub const TRIM_RANGE: f32 = 36.0;
/// Below this peak a clip counts as silence and is left alone.
const SILENCE: f32 = 0.001;
/// The peak a boost never passes, in dB.
const CEILING_DB: f32 = -0.1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Measure {
    Peak,
    Rms,
    /// Integrated loudness (BS.1770, gated), as the Mastering window measures it.
    Lufs,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scope {
    Track,
    Clip,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Method {
    InputTrim,
    ClipGain,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Settings {
    pub target_db: f32,
    pub measure: Measure,
    pub scope: Scope,
    pub method: Method,
    /// The most it turns anything up or down by, in dB.
    pub max_boost: f32,
    pub max_cut: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            target_db: -12.0,
            measure: Measure::Peak,
            scope: Scope::Track,
            method: Method::InputTrim,
            max_boost: 24.0,
            max_cut: 36.0,
        }
    }
}

/// Starting points; every number stays editable.
const PRESETS: [(&str, Measure, f32); 6] = [
    ("Peak -6 dB", Measure::Peak, -6.0),
    ("Peak -12 dB", Measure::Peak, -12.0),
    ("RMS -18 dB", Measure::Rms, -18.0),
    ("RMS -20 dB", Measure::Rms, -20.0),
    ("-18 LUFS", Measure::Lufs, -18.0),
    ("-23 LUFS", Measure::Lufs, -23.0),
];

/// The level a row has in measure `m`.
fn level_in(m: Measure, peak: f32, rms: f32, lufs: f32) -> f32 {
    match m {
        Measure::Peak => peak,
        Measure::Rms => rms,
        Measure::Lufs => lufs,
    }
}

fn settings_file() -> Option<PathBuf> {
    crate::recent_files::config_dir().map(|d| d.join("gain-staging"))
}

impl Settings {
    fn to_text(self) -> String {
        format!(
            "{} {} {} {} {} {}",
            self.target_db,
            match self.measure {
                Measure::Peak => "peak",
                Measure::Rms => "rms",
                Measure::Lufs => "lufs",
            },
            if self.scope == Scope::Track { "track" } else { "clip" },
            if self.method == Method::InputTrim { "trim" } else { "clipgain" },
            self.max_boost,
            self.max_cut
        )
    }

    fn from_text(text: &str) -> Option<Self> {
        let mut parts = text.split_whitespace();
        let target_db: f32 = parts.next()?.parse().ok()?;
        let measure = match parts.next()? {
            "rms" => Measure::Rms,
            "lufs" => Measure::Lufs,
            _ => Measure::Peak,
        };
        let scope = match parts.next()? {
            "clip" => Scope::Clip,
            _ => Scope::Track,
        };
        let method = match parts.next()? {
            "clipgain" => Method::ClipGain,
            _ => Method::InputTrim,
        };
        let max_boost: f32 = parts.next()?.parse().ok()?;
        let max_cut: f32 = parts.next()?.parse().ok()?;
        let ok = |v: f32| v.is_finite();
        (ok(target_db) && ok(max_boost) && ok(max_cut)).then_some(Self {
            target_db: target_db.clamp(-60.0, 0.0),
            measure,
            scope,
            method,
            max_boost: max_boost.clamp(0.0, 48.0),
            max_cut: max_cut.clamp(0.0, 60.0),
        })
    }

    /// The user's saved default, or the built-in one.
    fn load() -> Self {
        settings_file()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|t| Self::from_text(&t))
            .unwrap_or_default()
    }

    fn save(self) -> Result<(), String> {
        let file = settings_file().ok_or("no settings folder")?;
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(file, self.to_text()).map_err(|e| e.to_string())
    }
}

/// One clip to measure, with what the thread needs of it.
struct Job {
    track: usize,
    region: usize,
    track_name: String,
    name: String,
    file: AudioFile,
    /// The part of the file the clip plays, in seconds (length 0: to the end).
    start: f32,
    len: f32,
}

/// What a clip measured, of its file alone (no gain applied).
struct Clip {
    track: usize,
    region: usize,
    track_name: String,
    name: String,
    /// The loudest sample, linear.
    peak: f32,
    /// The mean square of the stretches with sound in them, and how many there were.
    power: f64,
    blocks: usize,
    /// Integrated loudness in LUFS, and the power it stands for (to add clips of a track up).
    lufs: f64,
    clipped: bool,
    error: Option<String>,
}

/// Peak, the mean square of the 0.4 s stretches above -60 dB, their number, and whether it clips
/// (three frames in a row at full scale). `samples` is interleaved stereo.
fn measure(samples: &[f32], rate: u32) -> (f32, f64, usize, bool) {
    let block = ((rate as usize * 4 / 10).max(1)) * 2;
    let gate = 1e-6f64;
    let mut peak = 0f32;
    let mut run = 0u32;
    let mut clipped = false;
    let (mut sum, mut count) = (0f64, 0usize);
    for chunk in samples.chunks(block) {
        let mut p = 0f64;
        for v in chunk {
            let a = v.abs();
            peak = peak.max(a);
            if a >= 0.999 {
                run += 1;
                if run >= 6 {
                    clipped = true;
                }
            } else {
                run = 0;
            }
            p += f64::from(*v) * f64::from(*v);
        }
        let mean = p / chunk.len().max(1) as f64;
        if mean > gate {
            sum += mean;
            count += 1;
        }
    }
    let power = if count > 0 { sum / count as f64 } else { 0.0 };
    (peak, power, count, clipped)
}

fn db(linear: f32) -> f32 {
    20.0 * linear.max(1e-9).log10()
}

fn power_db(power: f64) -> f32 {
    (10.0 * power.max(1e-18).log10()) as f32
}

/// A row of the table: a track or a clip, where it is and what would change.
struct Row {
    key: (usize, Option<usize>),
    name: String,
    peak_db: f32,
    rms_db: f32,
    lufs_db: f32,
    change: f32,
    silent: bool,
    warning: String,
}

#[derive(Default)]
pub struct GainStagingView {
    pub open: bool,
    settings: Option<Settings>,
    clips: Vec<Clip>,
    /// Rows the user left out.
    excluded: HashSet<(usize, Option<usize>)>,
    job: Option<Receiver<Clip>>,
    total: usize,
    /// What this window applied, to take back with Reset: where, by how much, and how.
    applied: Vec<(usize, Option<usize>, f32, Method)>,
    note: String,
}

impl GainStagingView {
    fn settings(&mut self) -> Settings {
        *self.settings.get_or_insert_with(Settings::load)
    }

    /// Starts measuring the audio tracks (only `only` if given) on a thread of its own.
    fn scan(&mut self, p: &Project, only: Option<&[usize]>, ctx: &egui::Context) {
        let mut jobs = Vec::new();
        for (ti, t) in p.tracks.iter().enumerate() {
            if t.kind != TrackKind::Audio || t.reference || only.is_some_and(|o| !o.contains(&ti)) {
                continue;
            }
            for (ri, r) in t.regions.iter().enumerate() {
                let Some(audio) = r.audio.as_ref() else {
                    continue;
                };
                if r.params.mute {
                    continue;
                }
                jobs.push(Job {
                    track: ti,
                    region: ri,
                    track_name: t.name.clone(),
                    name: r.name.clone(),
                    file: audio.file.clone(),
                    start: r.params.source_start,
                    len: r.params.source_len,
                });
            }
        }
        self.clips.clear();
        self.excluded.clear();
        self.total = jobs.len();
        self.note = if jobs.is_empty() {
            "No audio clips to measure.".to_string()
        } else {
            String::new()
        };
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            // Clips of one recording follow one another: the last file read is kept.
            let mut last: Option<(PathBuf, Arc<Vec<f32>>)> = None;
            for job in jobs {
                let data = match &last {
                    Some((path, data)) if *path == job.file.path => Ok(data.clone()),
                    _ => job.file.read_stereo().map(Arc::new).map_err(|e| e.to_string()),
                };
                let clip = match data {
                    Ok(data) => {
                        let rate = f64::from(job.file.sample_rate.max(1));
                        let from = ((f64::from(job.start.max(0.0)) * rate) as usize * 2).min(data.len());
                        let to = if job.len > 0.0 {
                            (from + (f64::from(job.len) * rate) as usize * 2).min(data.len())
                        } else {
                            data.len()
                        };
                        let (peak, power, blocks, clipped) =
                            measure(&data[from..to], job.file.sample_rate);
                        let mut loud = crate::mastering::Loudness::new(job.file.sample_rate.max(1));
                        for frame in data[from..to].chunks_exact(2) {
                            loud.add([frame[0], frame[1]]);
                        }
                        let lufs = loud.integrated();
                        last = Some((job.file.path.clone(), data));
                        Clip {
                            track: job.track,
                            region: job.region,
                            track_name: job.track_name,
                            name: job.name,
                            peak,
                            power,
                            blocks,
                            lufs,
                            clipped,
                            error: None,
                        }
                    }
                    Err(e) => Clip {
                        track: job.track,
                        region: job.region,
                        track_name: job.track_name,
                        name: job.name,
                        peak: 0.0,
                        power: 0.0,
                        blocks: 0,
                        lufs: -120.0,
                        clipped: false,
                        error: Some(e),
                    },
                };
                if tx.send(clip).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
        });
        self.job = Some(rx);
    }

    /// Takes in what the measuring thread has finished.
    fn drain(&mut self) {
        let Some(rx) = self.job.as_ref() else {
            return;
        };
        loop {
            match rx.try_recv() {
                Ok(clip) => self.clips.push(clip),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.job = None;
                    break;
                }
            }
        }
    }

    /// The table for these settings, with the project's gains as they are now.
    fn rows(&self, p: &Project, s: Settings) -> Vec<Row> {
        let clip_gain = |c: &Clip| {
            p.tracks
                .get(c.track)
                .and_then(|t| t.regions.get(c.region))
                .map_or(0.0, |r| r.params.gain_db)
        };
        let trim = |t: usize| p.tracks.get(t).map_or(0.0, |t| t.trim_db);
        let mut rows = Vec::new();
        let mut finish = |key: (usize, Option<usize>), name: String, peak_db: f32, rms_db: f32, lufs_db: f32, silent: bool, clipped: bool, error: Option<&String>| {
            let current = level_in(s.measure, peak_db, rms_db, lufs_db);
            let mut warning = String::new();
            let mut change = 0.0;
            if let Some(e) = error {
                warning = format!("Could not be read: {e}");
            } else if silent {
                warning = "Silent: left alone".to_string();
            } else {
                let wanted = s.target_db - current;
                change = wanted.clamp(-s.max_cut, s.max_boost);
                if change != wanted {
                    warning = format!("Held to {change:+.1} dB by the limit");
                }
                // A boost never takes the loudest point over full scale.
                let room = CEILING_DB - peak_db;
                if change > room {
                    change = room.max(change.min(0.0));
                    warning = "Held back: its peaks leave no more room".to_string();
                }
                if clipped {
                    warning = "Clipped in the recording: turning it down cannot repair that".to_string();
                }
            }
            rows.push(Row {
                key,
                name,
                peak_db,
                rms_db,
                lufs_db,
                change,
                silent: silent || error.is_some(),
                warning,
            });
        };
        match s.scope {
            Scope::Clip => {
                for c in &self.clips {
                    let g = clip_gain(c) + trim(c.track);
                    finish(
                        (c.track, Some(c.region)),
                        format!("{} · {}", c.track_name, c.name),
                        db(c.peak) + g,
                        power_db(c.power) + g,
                        c.lufs as f32 + g,
                        c.peak < SILENCE,
                        c.clipped,
                        c.error.as_ref(),
                    );
                }
            }
            Scope::Track => {
                let mut tracks: Vec<usize> = self.clips.iter().map(|c| c.track).collect();
                tracks.sort_unstable();
                tracks.dedup();
                for t in tracks {
                    let of_track = || self.clips.iter().filter(move |c| c.track == t);
                    // The loudest clip's peak, and the clips' power by how long each sounds: their
                    // levels against one another stay as they are.
                    let peak_db = of_track()
                        .filter(|c| c.peak >= SILENCE)
                        .map(|c| db(c.peak) + clip_gain(c))
                        .fold(f32::MIN, f32::max);
                    let (mut sum, mut weight) = (0f64, 0usize);
                    for c in of_track() {
                        let g = f64::from(clip_gain(c));
                        sum += c.power * 10f64.powf(g / 10.0) * c.blocks as f64;
                        weight += c.blocks;
                    }
                    // Loudness added up as power, by how long each clip sounds.
                    let (mut lsum, mut lweight) = (0f64, 0usize);
                    for c in of_track().filter(|c| c.lufs > -70.0) {
                        let g = f64::from(clip_gain(c));
                        lsum += 10f64.powf((c.lufs + g) / 10.0) * c.blocks.max(1) as f64;
                        lweight += c.blocks.max(1);
                    }
                    let lufs_db = if lweight > 0 {
                        (10.0 * (lsum / lweight as f64).log10()) as f32
                    } else {
                        -120.0
                    };
                    let silent = peak_db == f32::MIN;
                    let rms_db = if weight > 0 {
                        power_db(sum / weight as f64)
                    } else {
                        -180.0
                    };
                    let name = of_track().next().map_or_else(String::new, |c| c.track_name.clone());
                    let error = of_track().find_map(|c| c.error.as_ref());
                    finish(
                        (t, None),
                        name,
                        if silent { -180.0 } else { peak_db + trim(t) },
                        rms_db + trim(t),
                        lufs_db + trim(t),
                        silent,
                        of_track().any(|c| c.clipped),
                        // One unreadable clip does not stop the rest of its track.
                        error.filter(|_| silent),
                    );
                }
            }
        }
        rows
    }

    /// Applies the rows that are not left out (`only`: just that one).
    fn apply(&mut self, p: &mut Project, s: Settings, only: Option<(usize, Option<usize>)>) {
        let rows = self.rows(p, s);
        let mut done = 0;
        for row in rows {
            if row.silent
                || row.change.abs() < 0.05
                || self.excluded.contains(&row.key)
                || only.is_some_and(|k| k != row.key)
            {
                continue;
            }
            let (t, region) = row.key;
            let Some(track) = p.tracks.get_mut(t) else {
                continue;
            };
            let method = match (region, s.method) {
                (Some(_), _) => Method::ClipGain,
                (None, m) => m,
            };
            change_gain(track, region, row.change, method);
            self.applied.push((t, region, row.change, method));
            done += 1;
        }
        if done > 0 {
            p.song_rev += 1;
        }
        self.note = format!("Applied to {done}. Edit > Undo takes it back.");
    }

    /// Takes back everything this window applied.
    fn reset(&mut self, p: &mut Project) {
        let n = self.applied.len();
        for (t, region, change, method) in self.applied.drain(..).rev() {
            if let Some(track) = p.tracks.get_mut(t) {
                change_gain(track, region, -change, method);
            }
        }
        if n > 0 {
            p.song_rev += 1;
        }
        self.note = format!("Took back {n} changes.");
    }
}

/// Moves a track's input trim, one clip's gain, or (a track by clip gain) all its clips' gains.
fn change_gain(track: &mut crate::model::Track, region: Option<usize>, by: f32, method: Method) {
    let clip = |r: &mut crate::model::Region| {
        if r.audio.is_some() {
            r.params.gain_db = (r.params.gain_db + by).clamp(MIN_REGION_GAIN, MAX_REGION_GAIN);
        }
    };
    match (region, method) {
        (Some(ri), _) => {
            if let Some(r) = track.regions.get_mut(ri) {
                clip(r);
            }
        }
        (None, Method::InputTrim) => {
            track.trim_db = (track.trim_db + by).clamp(-TRIM_RANGE, TRIM_RANGE);
        }
        (None, Method::ClipGain) => track.regions.iter_mut().for_each(clip),
    }
}

fn frame() -> egui::Frame {
    egui::Frame::new()
        .fill(crate::theme::panel_bg())
        .stroke(egui::Stroke::new(1.0, crate::theme::border()))
        .corner_radius(10)
        .inner_margin(egui::Margin::same(12))
        .shadow(egui::epaint::Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: egui::Color32::from_black_alpha(110),
        })
}

/// The Auto Gain Staging window.
pub fn show(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    // The tracks picked with Shift or Ctrl, or else the selected one.
    let chosen: Vec<usize> = if s.tracks.picked_tracks.contains(&p.selected_track) {
        s.tracks.picked_tracks.clone()
    } else {
        vec![p.selected_track]
    };
    let v = &mut s.gain_staging;
    if !v.open {
        return;
    }
    v.drain();
    let mut set = v.settings();
    let mut open = true;
    egui::Window::new("Auto Gain Staging")
        .id(egui::Id::unique("gain_staging"))
        .frame(frame())
        .open(&mut open)
        .default_width(760.0)
        .default_height(460.0)
        .show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Target");
                ui.add(
                    egui::DragValue::new(&mut set.target_db)
                        .speed(0.1)
                        .range(-60.0..=0.0)
                        .suffix(if set.measure == Measure::Lufs { " LUFS" } else { " dB" }),
                );
                ui.selectable_value(&mut set.measure, Measure::Peak, "Peak");
                ui.selectable_value(&mut set.measure, Measure::Rms, "RMS");
                ui.selectable_value(&mut set.measure, Measure::Lufs, "LUFS")
                    .on_hover_text("Integrated loudness, as streaming services measure it");
                egui::ComboBox::from_id_salt("gain_presets")
                    .selected_text("Presets")
                    .show_ui(ui, |ui| {
                        for (name, measure, target) in PRESETS {
                            if ui.button(name).clicked() {
                                set.measure = measure;
                                set.target_db = target;
                                ui.close();
                            }
                        }
                    });
                ui.separator();
                ui.label("Measure");
                ui.selectable_value(&mut set.scope, Scope::Track, "Per Track")
                    .on_hover_text("One change for the track: its clips keep their levels against one another");
                ui.selectable_value(&mut set.scope, Scope::Clip, "Per Clip")
                    .on_hover_text("Each clip on its own, by clip gain");
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Apply as");
                ui.add_enabled_ui(set.scope == Scope::Track, |ui| {
                    ui.selectable_value(&mut set.method, Method::InputTrim, "Track Input Trim")
                        .on_hover_text("Before the track's effects. The fader stays where it is");
                    ui.selectable_value(&mut set.method, Method::ClipGain, "Clip Gain")
                        .on_hover_text("The same change on every clip of the track");
                });
                ui.separator();
                ui.label("Most boost");
                ui.add(egui::DragValue::new(&mut set.max_boost).speed(0.5).range(0.0..=48.0).suffix(" dB"));
                ui.label("Most cut");
                ui.add(egui::DragValue::new(&mut set.max_cut).speed(0.5).range(0.0..=60.0).suffix(" dB"));
                if ui.button("Save as Default").clicked() {
                    v.note = match set.save() {
                        Ok(()) => "Saved as the default.".to_string(),
                        Err(e) => format!("Could not save: {e}"),
                    };
                }
            });
            ui.add_space(4.0);
            let busy = v.job.is_some();
            let mut act: Option<u8> = None;
            ui.horizontal_wrapped(|ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    if ui.button("Scan Session").clicked() {
                        act = Some(0);
                    }
                    if ui
                        .button(if chosen.len() > 1 { "Scan Selected Tracks" } else { "Scan Selected Track" })
                        .clicked()
                    {
                        act = Some(1);
                    }
                });
                ui.add_enabled_ui(!busy && !v.clips.is_empty(), |ui| {
                    if ui
                        .button(RichText::new("Apply All").strong())
                        .on_hover_text("Every row that is ticked")
                        .clicked()
                    {
                        act = Some(2);
                    }
                });
                ui.add_enabled_ui(!v.applied.is_empty(), |ui| {
                    if ui
                        .button("Reset Gain Staging")
                        .on_hover_text("Takes back what this window applied")
                        .clicked()
                    {
                        act = Some(3);
                    }
                });
                if busy {
                    ui.spinner();
                    ui.label(format!("Measuring {} of {}…", v.clips.len(), v.total));
                } else if !v.note.is_empty() {
                    ui.label(RichText::new(v.note.as_str()).color(crate::theme::text_dim()));
                }
            });
            ui.separator();
            let rows = v.rows(p, set);
            let mut apply_one = None;
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                egui::Grid::new(egui::Id::unique("gain_rows"))
                    .num_columns(9)
                    .striped(true)
                    .spacing([14.0, 5.0])
                    .show(ui, |ui| {
                        for head in ["", "Track or clip", "Peak", "RMS", "LUFS", "Target", "Change", "After", ""] {
                            ui.label(RichText::new(head).small().color(crate::theme::text_dim()));
                        }
                        ui.end_row();
                        for row in &rows {
                            let mut on = !v.excluded.contains(&row.key);
                            if ui.checkbox(&mut on, "").on_hover_text("Untick to leave it out").changed() {
                                if on {
                                    v.excluded.remove(&row.key);
                                } else {
                                    v.excluded.insert(row.key);
                                }
                            }
                            ui.label(row.name.as_str());
                            if row.silent {
                                ui.label("-");
                                ui.label("-");
                                ui.label("-");
                            } else {
                                ui.label(format!("{:.1} dB", row.peak_db));
                                ui.label(format!("{:.1} dB", row.rms_db));
                                if row.lufs_db > -70.0 {
                                    ui.label(format!("{:.1}", row.lufs_db));
                                } else {
                                    ui.label("-");
                                }
                            }
                            ui.label(format!("{:.1}", set.target_db));
                            let current = level_in(set.measure, row.peak_db, row.rms_db, row.lufs_db);
                            if row.silent {
                                ui.label("-");
                                ui.label("-");
                            } else if row.change.abs() < 0.05 {
                                ui.label(RichText::new("at target").color(crate::theme::GOOD));
                                ui.label(format!("{current:.1} dB"));
                            } else {
                                ui.label(RichText::new(format!("{:+.1} dB", row.change)).strong());
                                ui.label(format!("{:.1} dB", current + row.change));
                            }
                            ui.horizontal(|ui| {
                                if !row.silent
                                    && row.change.abs() >= 0.05
                                    && ui.small_button("Apply").clicked()
                                {
                                    apply_one = Some(row.key);
                                }
                                if !row.warning.is_empty() {
                                    ui.label(
                                        RichText::new(row.warning.as_str())
                                            .small()
                                            .color(crate::theme::SOLO),
                                    );
                                }
                            });
                            ui.end_row();
                        }
                    });
                if rows.is_empty() && !busy {
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new(
                            "Scan the session to measure its audio tracks. Nothing changes until you apply.",
                        )
                        .color(crate::theme::text_dim()),
                    );
                }
            });
            match act {
                Some(0) => v.scan(p, None, ctx),
                Some(1) => v.scan(p, Some(&chosen), ctx),
                Some(2) => v.apply(p, set, None),
                Some(3) => v.reset(p),
                _ => {}
            }
            if let Some(key) = apply_one {
                v.apply(p, set, Some(key));
            }
        });
    v.settings = Some(set);
    if !open {
        v.open = false;
    }
}
