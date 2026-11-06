//! Audio recording, as in Logic: while Record and Play are both on, every armed audio track
//! (R button) records its input into a new region; with none armed, the selected audio track
//! does. The region is red and grows with the song position, and takes the track's colour and
//! the waveform when the recording stops.
//!
//! The input is open only while a track wants it, so the microphone is not held for nothing.
//! The recordings are files in this session's media folder; saving the project copies them
//! into it.

use crate::i18n::{tr, trf};
use crate::model::{Project, Region, RegionParams, TrackKind};
use prowave_engine::{input_devices, AudioFile, Capture, InputInfo, InputSource};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How much of a level meter's value is left at the next frame.
const METER_FALL: f32 = 0.85;
/// Less free disk than this: recording does not start, and one in progress stops and is kept.
const STOP_FREE_BYTES: u64 = 200 * 1024 * 1024;
/// Less room than this is pointed out when recording starts.
const WARN_FREE_SECONDS: u64 = 10 * 60;
/// Recording that waits for sound starts this far before it was noticed, to keep the attack.
const LEAD_SECONDS: f64 = 0.1;
/// A playhead moving further than this in one frame was moved by hand.
const JUMP_BEATS: f64 = 4.0;

/// The free space where recordings go, looked up off the UI thread: asking can take a moment.
struct DiskWatch {
    free: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
}

impl DiskWatch {
    fn start(dir: PathBuf) -> Self {
        let free = Arc::new(AtomicU64::new(u64::MAX));
        let stop = Arc::new(AtomicBool::new(false));
        let (out, end) = (free.clone(), stop.clone());
        std::thread::spawn(move || {
            while !end.load(Ordering::Relaxed) {
                if let Some(bytes) = free_bytes(&dir) {
                    out.store(bytes, Ordering::Relaxed);
                }
                for _ in 0..50 {
                    if end.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        });
        Self { free, stop }
    }

    /// `None` until it is known.
    fn free(&self) -> Option<u64> {
        Some(self.free.load(Ordering::Relaxed)).filter(|b| *b != u64::MAX)
    }
}

impl Drop for DiskWatch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn free_bytes(dir: &Path) -> Option<u64> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    disks
        .list()
        .iter()
        .filter(|d| dir.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| d.available_space())
}

/// The stretch of playback the input last heard, for Capture Recording.
struct Played {
    began: Instant,
    from_beat: f64,
    last_beat: f64,
    ended: Option<Instant>,
}

/// The audio input device of this computer that Pro Wave records from (Logic: Settings >
/// Audio > Input Device). It belongs to the computer, not to a project. `default()` never
/// touches the disk (used by tests); [`InputChoice::load`] is the real, remembered one.
#[derive(Default)]
pub struct InputChoice {
    /// The list as of the last [`InputChoice::refresh`].
    pub devices: Vec<InputInfo>,
    /// The chosen device's id; `None` is the system's default input.
    pub device: Option<String>,
    changed: bool,
    file: Option<PathBuf>,
}

impl InputChoice {
    pub fn load() -> Self {
        let file = crate::recent_files::config_dir().map(|d| d.join("audio-input"));
        let device = file
            .as_deref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .map(|text| text.trim().to_string())
            .filter(|id| !id.is_empty());
        Self {
            device,
            file,
            ..Self::default()
        }
    }

    /// Asks the system for the inputs (this can take a moment, so not on every frame).
    pub fn refresh(&mut self) {
        self.devices = input_devices();
    }

    /// Records from `device` from now on, and remembers it. Best effort: a choice that cannot
    /// be saved is not worth interrupting the user for.
    pub fn pick(&mut self, device: Option<String>) {
        if device == self.device {
            return;
        }
        self.device = device;
        self.changed = true;
        let Some(file) = &self.file else { return };
        let result = file
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(file, self.device.as_deref().unwrap_or_default()));
        if let Err(e) = result {
            tracing::warn!("could not save the audio input: {e}");
        }
    }

    /// How many inputs the chosen device has, for the track's Input menu.
    pub fn channels(&self) -> u16 {
        let chosen = self.devices.iter().find(|d| match &self.device {
            Some(id) => d.id == *id,
            None => d.is_default,
        });
        chosen
            .or(self.devices.first())
            .map_or(2, |d| d.channels)
            .min(prowave_engine::MAX_INPUT_CHANNELS as u16)
    }
}

/// The name of an input in the channel strip's Input slot.
pub fn input_label(source: InputSource) -> String {
    match source {
        InputSource::Mono(c) => trf("Input {}", &[&(c + 1)]),
        InputSource::Stereo(c) => trf("Input {}", &[&format!("{}-{}", c + 1, c + 2)]),
    }
}

/// What [`AudioRecorder::update`] did, for the app to react to.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Nothing,
    /// Recording ended: the last region that was kept, if any, and a warning, if any.
    Finished(Option<(usize, usize)>, Option<String>),
    /// The input cannot be used or the recording cannot start.
    Failed(String),
}

/// One track's recording in progress.
struct Take {
    track: usize,
    region: usize,
    id: u32,
    start: f64,
    path: PathBuf,
    /// Recording inside the Cycle: where it runs, as each pass becomes a take.
    cycle: Option<(f64, f64)>,
    live: prowave_engine::LivePeaks,
    /// The waveform so far, and its loudest sample.
    peaks: Vec<(f32, f32)>,
    top: f32,
}

/// The real waveform of a recording in progress, to draw in its region.
pub struct LiveWave<'a> {
    pub peaks: &'a [(f32, f32)],
    /// Where it starts in the region, and how long it is, in beats.
    pub from_beats: f64,
    pub len_beats: f64,
    /// The same fit to the region's height as the finished clip gets.
    pub fit: f32,
}

/// The recording in progress, and the open input.
#[derive(Default)]
pub struct AudioRecorder {
    capture: Option<Capture>,
    /// Opening the input failed: not tried again until something changes.
    failed: bool,
    takes: Vec<Take>,
    /// Numbers the files of this session.
    count: u32,
    /// Level of each input channel, for the meters.
    levels: Vec<f32>,
    fresh: Vec<f32>,
    /// The delay of the output, as of the last update.
    latency_us: u32,
    played: Option<Played>,
    disk: Option<DiskWatch>,
}

/// The audio tracks that record: the armed ones, or else the selected one.
pub fn targets(p: &Project) -> Vec<usize> {
    let armed: Vec<usize> = p
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.kind == TrackKind::Audio && t.rec)
        .map(|(i, _)| i)
        .collect();
    if !armed.is_empty() {
        return armed;
    }
    let selected = p.tracks.get(p.selected_track);
    if selected.is_some_and(|t| t.kind == TrackKind::Audio) {
        vec![p.selected_track]
    } else {
        Vec::new()
    }
}

/// Whether Record means audio now, so that the MIDI recorder leaves it alone.
pub fn takes_over(p: &Project) -> bool {
    p.recording && !targets(p).is_empty()
}

/// Whether a track needs the input open now.
fn needs_input(p: &Project) -> bool {
    takes_over(p)
        || p.tracks
            .iter()
            .any(|t| t.kind == TrackKind::Audio && (t.rec || t.monitor))
}

/// The file for a recording on track `name`.
fn file_name(name: &str, count: u32) -> String {
    let clean: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        .take(40)
        .collect();
    let clean = clean.trim();
    let clean = if clean.is_empty() { "Audio" } else { clean };
    format!("{clean}-{count}.wav")
}

/// Whether region `r` of track `t` is one being recorded (its name starts with `*`, as for
/// MIDI recording). If not, the track or the region was deleted or moved.
fn is_open(p: &Project, t: usize, r: usize) -> bool {
    p.tracks
        .get(t)
        .and_then(|t| t.regions.get(r))
        .is_some_and(|region| region.name.starts_with('*'))
}

/// Turns the red region into the finished audio region: its length is the file's. Recorded over
/// another audio region it makes a take folder with it, or in replace mode takes its place. Returns
/// the region's number afterwards.
fn apply_take(
    p: &mut Project,
    t: usize,
    r: usize,
    start: f64,
    cycle: Option<(f64, f64)>,
    file: AudioFile,
    ctx: &egui::Context,
) -> usize {
    let map = p.tempo_map();
    let name = p.tracks[t].name.clone();
    // Round the Cycle more than once: a take folder, with a take for each pass.
    let passes = cycle.map(|(from, to)| {
        let lead = map.seconds_at(start) - map.seconds_at(from);
        let pass = map.seconds_at(to) - map.seconds_at(from);
        (
            from,
            to,
            crate::takes::audio_take_offsets(file.duration_secs(), lead, pass),
        )
    });
    if let Some((from, to, offsets)) = passes.filter(|(_, _, o)| o.len() > 1) {
        let takes = offsets
            .into_iter()
            .map(|offset| crate::model::Take {
                audio_offset: Some(offset),
                ..crate::model::Take::default()
            })
            .collect();
        let start_bar = p.beats_to_bar(from) as f32 + 1.0;
        let len_bars = (p.beats_to_bar(to) - p.beats_to_bar(from)) as f32;
        let region = &mut p.tracks[t].regions[r];
        region.name = name;
        region.start_bar = start_bar;
        region.len_bars = len_bars;
        region.audio = Some(crate::import::audio_region(file, ctx));
        region.folder = Some(crate::model::TakeFolder::new(takes, (to - from) as f32));
    } else {
        let end = map.advance(start, file.duration_secs());
        let start_bar = p.beats_to_bar(start) as f32 + 1.0;
        let len_bars = (p.beats_to_bar(end) - p.beats_to_bar(start)) as f32;
        let region = &mut p.tracks[t].regions[r];
        region.name = name;
        region.start_bar = start_bar;
        region.len_bars = len_bars;
        region.audio = Some(crate::import::audio_region(file, ctx));
    }
    p.song_rev += 1;
    crate::takes::settle_recording(p, t, r, true)
}

impl AudioRecorder {
    pub fn active(&self) -> bool {
        !self.takes.is_empty()
    }

    /// Where the song was when the recording in progress began.
    pub fn began_at(&self) -> Option<f64> {
        self.takes.first().map(|t| t.start)
    }

    /// Record Repeat: the recordings in progress are thrown away, regions and files.
    pub fn discard(&mut self, p: &mut Project) {
        for take in std::mem::take(&mut self.takes) {
            if let Some(capture) = self.capture.as_mut() {
                let _ = capture.stop(take.id);
            }
            if is_open(p, take.track, take.region) {
                p.tracks[take.track].regions.remove(take.region);
                p.song_rev += 1;
            }
            let _ = std::fs::remove_file(&take.path);
        }
    }

    /// The regions being recorded now (drawn red).
    pub fn regions(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.takes.iter().map(|t| (t.track, t.region))
    }

    /// Takes what the recorder has added to each waveform.
    fn read_live(&mut self) {
        for take in &mut self.takes {
            let Ok(live) = take.live.lock() else {
                continue;
            };
            let new = live.get(take.peaks.len()..).unwrap_or_default();
            for point in new {
                take.top = take.top.max(point.1).max(-point.0);
            }
            take.peaks.extend_from_slice(new);
        }
    }

    /// The waveform of region `r` of track `t`, if it is being recorded. Round the Cycle, the
    /// pass in progress.
    pub fn live_wave(&self, p: &Project, t: usize, r: usize) -> Option<LiveWave<'_>> {
        let take = self.takes.iter().find(|k| k.track == t && k.region == r)?;
        let rate = f64::from(self.capture.as_ref()?.sample_rate());
        let point = f64::from(prowave_engine::LIVE_HOP) / rate;
        let map = p.tempo_map();
        let begun = map.seconds_at(take.start);
        let (shown_from, first, offset) = match take.cycle {
            Some((from, to)) => {
                let from_at = map.seconds_at(from);
                let pass = (map.seconds_at(to) - from_at).max(1e-3);
                let lead = begun - from_at;
                let so_far = lead + take.peaks.len() as f64 * point;
                let pass_begun = (so_far / pass).floor() * pass;
                let first = ((pass_begun - lead).max(0.0) / point) as usize;
                (
                    from,
                    first.min(take.peaks.len()),
                    (lead - pass_begun).max(0.0),
                )
            }
            None => (take.start, 0, 0.0),
        };
        let peaks = &take.peaks[first..];
        let at = map.seconds_at(shown_from) + offset;
        let start = map.beat_at(at);
        let end = map.beat_at(at + peaks.len() as f64 * point);
        Some(LiveWave {
            peaks,
            from_beats: start - shown_from,
            len_beats: end - start,
            fit: if take.top > 0.0 {
                (0.95 / take.top).clamp(1.0, crate::model::MAX_WAVEFORM_FIT)
            } else {
                1.0
            },
        })
    }

    /// What the Recording Meter shows: each recording track's name and input levels. The second
    /// level is for the right side of a stereo input.
    pub fn input_levels(&self, p: &Project) -> Vec<(String, f32, Option<f32>)> {
        if self.capture.is_none() {
            return Vec::new();
        }
        let at = |c: u16| self.levels.get(usize::from(c)).copied().unwrap_or(0.0);
        targets(p)
            .into_iter()
            .map(|t| {
                let track = &p.tracks[t];
                match track.input {
                    InputSource::Mono(c) => (track.name.clone(), at(c), None),
                    InputSource::Stereo(c) => (track.name.clone(), at(c), Some(at(c + 1))),
                }
            })
            .collect()
    }

    /// Whether an input a track would record from is louder than the project's trigger level.
    fn sound_arrived(&self, p: &Project) -> bool {
        let level = 10f32.powf(p.trigger_db / 20.0);
        self.input_levels(p)
            .iter()
            .any(|(_, l, r)| *l >= level || r.is_some_and(|r| r >= level))
    }

    /// Keeps track of the playback that Capture Recording would take.
    fn note_playing(&mut self, p: &Project) {
        if self.capture.is_none() || p.recording {
            self.played = None;
            return;
        }
        let now = Instant::now();
        if !p.playing || p.counting_in {
            if let Some(played) = &mut self.played {
                played.ended.get_or_insert(now);
            }
            return;
        }
        let at = p.position_beats;
        match &mut self.played {
            Some(played)
                if played.ended.is_none()
                    && at >= played.last_beat - 1e-6
                    && at - played.last_beat < JUMP_BEATS =>
            {
                played.last_beat = at;
            }
            _ => {
                self.played = Some(Played {
                    began: now,
                    from_beat: at,
                    last_beat: at,
                    ended: None,
                });
            }
        }
    }

    /// Capture Recording: makes regions of what the input heard during the last playback, on
    /// the tracks that would record, in their places in the song. Returns the last of them.
    pub fn capture_played(
        &mut self,
        p: &mut Project,
        ctx: &egui::Context,
    ) -> Option<(usize, usize)> {
        let played = self.played.take()?;
        let dir = crate::project_io::recordings_dir();
        std::fs::create_dir_all(&dir).ok()?;
        let now = Instant::now();
        let ago_start = (now - played.began).as_secs_f64();
        let ago_end = played.ended.map_or(0.0, |end| (now - end).as_secs_f64());
        let latency_us = self.latency_us;
        let mut kept = None;
        for t in targets(p) {
            let capture = self.capture.as_mut()?;
            let rate = f64::from(capture.sample_rate());
            self.count += 1;
            let (name, input) = (p.tracks[t].name.clone(), p.tracks[t].input);
            let path = dir.join(file_name(&name, self.count));
            let grabbed = capture.grab(&path, input, (ago_start, ago_end), latency_us);
            let file = match &grabbed {
                Ok((done, _)) if done.frames > 0 => AudioFile::open(&done.path).ok(),
                _ => None,
            };
            let (Ok((_, old)), Some(file)) = (grabbed, file) else {
                let _ = std::fs::remove_file(&path);
                continue;
            };
            let map = p.tempo_map();
            let start = map.beat_at(map.seconds_at(played.from_beat) + old as f64 / rate);
            let start_bar = p.beats_to_bar(start) as f32 + 1.0;
            let track = &mut p.tracks[t];
            track.regions.push(Region {
                name: format!("*{name}"),
                start_bar,
                len_bars: 0.01,
                notes: Vec::new(),
                controls: Vec::new(),
                folder: None,
                auto: Vec::new(),
                generated: None,
                audio: None,
                params: RegionParams::default(),
            });
            let region = track.regions.len() - 1;
            kept = Some((t, apply_take(p, t, region, start, None, file, ctx)));
        }
        kept
    }

    /// While a track is armed or recording its meter shows its input, as in Logic.
    pub fn show_levels(&self, p: &Project, meters: &mut [(f32, f32)]) {
        if self.capture.is_none() {
            return;
        }
        let at = |c: u16| self.levels.get(usize::from(c)).copied().unwrap_or(0.0);
        for (i, t) in p.tracks.iter().enumerate() {
            let listens = t.rec || t.monitor || self.takes.iter().any(|k| k.track == i);
            if t.kind != TrackKind::Audio || !listens {
                continue;
            }
            if let Some(meter) = meters.get_mut(i) {
                *meter = match t.input {
                    InputSource::Mono(c) => (at(c), at(c)),
                    InputSource::Stereo(c) => (at(c), at(c + 1)),
                };
            }
        }
    }

    /// The live input of a newly opened device, for the engine's monitoring.
    pub fn take_feed(&mut self) -> Option<Box<prowave_engine::InputFeed>> {
        self.capture.as_mut()?.take_monitor_feed()
    }

    /// Whether the input is open (the meters then need redrawing).
    pub fn listening(&self) -> bool {
        self.capture.is_some()
    }

    /// Call once per frame. `latency_us` is the delay of the output.
    pub fn update(
        &mut self,
        choice: &mut InputChoice,
        p: &mut Project,
        latency_us: u32,
        ctx: &egui::Context,
    ) -> Outcome {
        self.latency_us = latency_us;
        let changed = std::mem::take(&mut choice.changed);
        let wanted = needs_input(p);
        if self.takes.is_empty() && (changed || !wanted) {
            self.capture = None;
            self.failed = false;
            self.levels.clear();
        }
        let mut error = None;
        if wanted && self.capture.is_none() && !self.failed {
            match Capture::open(choice.device.as_deref()) {
                Ok(capture) => {
                    tracing::info!(
                        "audio input {} ({} channels at {} Hz)",
                        capture.device_name(),
                        capture.channels(),
                        capture.sample_rate()
                    );
                    self.capture = Some(capture);
                }
                Err(e) => {
                    self.failed = true;
                    error = Some(trf("Cannot open the audio input: {}", &[&e]));
                }
            }
        }
        if self.capture.is_none() {
            self.disk = None;
        } else if self.disk.is_none() {
            self.disk = Some(DiskWatch::start(crate::project_io::recordings_dir()));
        }
        self.read_levels();
        self.read_live();
        self.note_playing(p);
        let record = p.recording && p.playing && !p.counting_in && p.punched_in();
        let waiting = p.wait_for_sound && !self.sound_arrived(p);
        if self.takes.is_empty() {
            if self.failed && takes_over(p) {
                p.recording = false;
            } else if record && !waiting && !targets(p).is_empty() {
                let outcome = self.start(p, latency_us);
                if outcome != Outcome::Nothing {
                    return outcome;
                }
            }
        } else if record {
            let free = self.disk.as_ref().and_then(DiskWatch::free);
            if free.is_some_and(|bytes| bytes < STOP_FREE_BYTES) {
                let warning = "Recording stopped: the disk is nearly full".to_string();
                return match self.finish(p, ctx) {
                    Outcome::Finished(kept, _) => Outcome::Finished(kept, Some(warning)),
                    other => other,
                };
            }
            self.grow(p);
        } else {
            return self.finish(p, ctx);
        }
        error.map_or(Outcome::Nothing, Outcome::Failed)
    }

    fn read_levels(&mut self) {
        let Some(capture) = &self.capture else { return };
        capture.take_peaks(&mut self.fresh);
        self.levels.resize(self.fresh.len(), 0.0);
        for (level, peak) in self.levels.iter_mut().zip(&self.fresh) {
            *level = peak.max(*level * METER_FALL);
        }
    }

    fn start(&mut self, p: &mut Project, latency_us: u32) -> Outcome {
        let Some(capture) = self.capture.as_mut() else {
            p.recording = false;
            return Outcome::Failed(tr("Record: there is no audio input").into());
        };
        let dir = crate::project_io::recordings_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            p.recording = false;
            return Outcome::Failed(trf("Cannot record: {}", &[&e]));
        }
        let free = self.disk.as_ref().and_then(DiskWatch::free);
        if free.is_some_and(|bytes| bytes < STOP_FREE_BYTES) {
            p.recording = false;
            return Outcome::Failed("Cannot record: the disk is nearly full".to_string());
        }
        // 32-bit stereo at most, for every track that will record.
        let per_second = u64::from(capture.sample_rate()) * 8 * (targets(p).len() as u64).max(1);
        let low_space = free
            .map(|bytes| bytes / per_second.max(1))
            .filter(|seconds| *seconds < WARN_FREE_SECONDS)
            .map(|seconds| {
                format!(
                    "Disk space for only about {} minutes of recording",
                    seconds / 60 + 1
                )
            });
        let start = p.position_beats.max(0.0);
        // With the Cycle on, recording from inside it goes round: each pass is a take.
        let cycle = (p.cycle
            && p.cycle_end > p.cycle_start
            && (p.cycle_start..p.cycle_end).contains(&start))
        .then_some((p.cycle_start, p.cycle_end));
        // Waiting for sound: the take starts just before the sound was noticed.
        let (start, lead) = if p.wait_for_sound && cycle.is_none() {
            let map = p.tempo_map();
            let at = map.seconds_at(start);
            let lead = LEAD_SECONDS.min(at.max(0.0));
            (map.beat_at(at - lead), lead)
        } else {
            (start, 0.0)
        };
        let lead_us = (lead * 1e6) as u32;
        let shown_from = cycle.map_or(start, |(from, _)| from);
        let (start_bar, mut error) = (p.beats_to_bar(shown_from) as f32 + 1.0, low_space);
        for t in targets(p) {
            self.count += 1;
            let cycle_bars =
                cycle.map(|(from, to)| (p.beats_to_bar(to) - p.beats_to_bar(from)) as f32);
            let track = &mut p.tracks[t];
            let path = dir.join(file_name(&track.name, self.count));
            match capture.start_early(&path, track.input, latency_us, lead_us) {
                Ok((id, live)) => {
                    track.regions.push(Region {
                        name: format!("*{}", track.name),
                        start_bar,
                        len_bars: cycle_bars.unwrap_or(0.01),
                        notes: Vec::new(),
                        controls: Vec::new(),
                        folder: None,
                        auto: Vec::new(),
                        generated: None,
                        audio: None,
                        params: RegionParams::default(),
                    });
                    self.takes.push(Take {
                        track: t,
                        region: track.regions.len() - 1,
                        id,
                        start,
                        path,
                        cycle,
                        live,
                        peaks: Vec::new(),
                        top: 0.0,
                    });
                }
                Err(e) => error = Some(trf("Cannot record: {}", &[&e])),
            }
        }
        if self.takes.is_empty() {
            p.recording = false;
        }
        error.map_or(Outcome::Nothing, Outcome::Failed)
    }

    fn grow(&mut self, p: &mut Project) {
        let takes = std::mem::take(&mut self.takes);
        for take in takes {
            if is_open(p, take.track, take.region) {
                if take.cycle.is_some() {
                    // It stays as long as the Cycle: the passes pile up, they do not spread out.
                    self.takes.push(take);
                    continue;
                }
                let now = p.position_beats.max(take.start);
                let len = (p.beats_to_bar(now) - p.beats_to_bar(take.start)) as f32;
                let region = &mut p.tracks[take.track].regions[take.region];
                region.len_bars = region.len_bars.max(len);
                self.takes.push(take);
            } else if let Some(capture) = self.capture.as_mut() {
                // Its region is gone: nothing would play this file.
                let _ = capture.stop(take.id);
                let _ = std::fs::remove_file(&take.path);
            }
        }
    }

    fn finish(&mut self, p: &mut Project, ctx: &egui::Context) -> Outcome {
        p.recording = false;
        let (mut kept, mut warning) = (None, None);
        for take in std::mem::take(&mut self.takes) {
            let Some(capture) = self.capture.as_mut() else {
                break;
            };
            let done = capture.stop(take.id);
            let open = is_open(p, take.track, take.region);
            let file = match &done {
                Ok(d) if d.frames > 0 => AudioFile::open(&d.path).map_err(|e| e.to_string()),
                Ok(_) => Err(tr("no audio arrived from the input").to_string()),
                Err(e) => Err(e.to_string()),
            };
            match (open, file) {
                (true, Ok(file)) => {
                    let region = apply_take(
                        p,
                        take.track,
                        take.region,
                        take.start,
                        take.cycle,
                        file,
                        ctx,
                    );
                    kept = Some((take.track, region));
                    if let Ok(d) = &done {
                        if d.lost {
                            warning = Some(tr("Part of the recording was lost").to_string());
                        } else if let Some(text) = one_sided_warning(d.channels, d.side_peaks) {
                            warning = Some(text);
                        }
                    }
                }
                (open, file) => {
                    if open {
                        p.tracks[take.track].regions.remove(take.region);
                    }
                    if let Err(e) = file {
                        warning = Some(trf("Cannot record: {}", &[&e]));
                    }
                    let _ = std::fs::remove_file(&take.path);
                }
            }
        }
        Outcome::Finished(kept, warning)
    }
}

/// What to say about a stereo recording with sound on one side only: a single microphone or
/// instrument on a stereo input plays from one speaker. A mono input plays from both.
fn one_sided_warning(channels: u16, (left, right): (f32, f32)) -> Option<String> {
    const SILENT: f32 = 0.001;
    if channels != 2 || left.max(right) < SILENT * 10.0 {
        return None;
    }
    let side = if right < SILENT {
        ("left", "right")
    } else if left < SILENT {
        ("right", "left")
    } else {
        return None;
    };
    Some(format!(
        "This recording has sound on the {} side only: nothing came in on the {} input. \
         For one microphone or instrument, set the strip's Input slot to a single input.",
        side.0, side.1
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_stereo_recording_with_one_empty_side_is_pointed_out() {
        assert!(one_sided_warning(2, (0.5, 0.0))
            .unwrap()
            .contains("left side"));
        assert!(one_sided_warning(2, (0.0, 0.5))
            .unwrap()
            .contains("right side"));
        assert_eq!(one_sided_warning(2, (0.5, 0.4)), None, "both sides");
        assert_eq!(one_sided_warning(1, (0.5, 0.0)), None, "mono plays on both");
        assert_eq!(one_sided_warning(2, (0.0, 0.0)), None, "silence");
    }
    use crate::model::Track;

    fn project(kinds: &[TrackKind]) -> Project {
        let mut p = Project::empty();
        p.tracks = kinds.iter().map(|k| Track::new("T", *k)).collect();
        p.selected_track = 0;
        p
    }

    #[test]
    fn armed_audio_tracks_record_and_else_the_selected_one() {
        let mut p = project(&[TrackKind::Audio, TrackKind::Instrument, TrackKind::Audio]);
        assert_eq!(targets(&p), vec![0], "nothing armed: the selected track");
        p.tracks[2].rec = true;
        p.tracks[1].rec = true;
        assert_eq!(targets(&p), vec![2], "armed tracks, audio only");
        p.tracks[0].rec = true;
        assert_eq!(targets(&p), vec![0, 2], "all at once");
    }

    #[test]
    fn midi_keeps_record_unless_an_audio_track_is_the_target() {
        let mut p = project(&[TrackKind::Instrument, TrackKind::Audio]);
        p.recording = true;
        assert!(!takes_over(&p), "an instrument is selected");
        p.selected_track = 1;
        assert!(takes_over(&p));
        p.recording = false;
        assert!(!takes_over(&p));
    }

    #[test]
    fn the_input_is_held_only_while_a_track_wants_it() {
        let mut p = project(&[TrackKind::Audio, TrackKind::Instrument]);
        assert!(!needs_input(&p));
        p.tracks[1].rec = true;
        assert!(!needs_input(&p), "an instrument needs no audio input");
        p.tracks[0].rec = true;
        assert!(needs_input(&p), "armed");
        p.tracks[0].rec = false;
        p.recording = true;
        assert!(needs_input(&p), "about to record the selected track");
        p.recording = false;
        p.tracks[0].monitor = true;
        assert!(needs_input(&p), "listening to the input");
    }

    #[test]
    fn a_track_name_makes_a_safe_file_name() {
        assert_eq!(file_name("Lead Vox", 3), "Lead Vox-3.wav");
        assert_eq!(file_name("../../etc/passwd", 1), "etcpasswd-1.wav");
        assert_eq!(file_name("  ", 2), "Audio-2.wav");
        assert!(file_name(&"x".repeat(500), 1).len() < 60);
    }

    #[test]
    fn input_names() {
        assert_eq!(input_label(InputSource::Mono(0)), "Input 1");
        assert_eq!(input_label(InputSource::Stereo(2)), "Input 3-4");
    }

    #[test]
    fn the_channels_of_the_chosen_device_or_of_the_default() {
        let info = |id: &str, channels, is_default| InputInfo {
            id: id.into(),
            name: id.into(),
            channels,
            is_default,
        };
        let mut c = InputChoice {
            devices: vec![info("a", 2, true), info("b", 8, false)],
            ..InputChoice::default()
        };
        assert_eq!(c.channels(), 2);
        c.pick(Some("b".into()));
        assert_eq!(c.channels(), 8);
        c.pick(Some("gone".into()));
        assert_eq!(c.channels(), 2, "a device that left");
        assert_eq!(
            InputChoice::default().channels(),
            2,
            "before the first look"
        );
    }

    #[test]
    fn a_new_device_reopens_the_input_only_when_it_changed() {
        let mut c = InputChoice::default();
        c.pick(None);
        assert!(!c.changed, "the same choice");
        c.pick(Some("b".into()));
        assert!(c.changed);
    }

    #[test]
    fn a_finished_take_becomes_an_audio_region_of_the_files_length() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("take.wav");
        // 2 seconds at 120 BPM are 4 quarter notes: one bar of 4/4.
        prowave_engine::write_wav(&path, &vec![0.1; 48_000 * 2 * 2], 48_000).unwrap();
        let mut p = project(&[TrackKind::Audio]);
        p.tracks[0].name = "Vox".into();
        p.tempo = 120.0;
        p.tracks[0].regions.push(Region {
            name: "*Vox".into(),
            start_bar: 2.0,
            len_bars: 0.01,
            notes: Vec::new(),
            controls: Vec::new(),
            folder: None,
            auto: Vec::new(),
            generated: None,
            audio: None,
            params: RegionParams::default(),
        });
        let rev = p.song_rev;
        assert!(is_open(&p, 0, 0));
        let file = AudioFile::open(&path).unwrap();
        apply_take(&mut p, 0, 0, 4.0, None, file, &egui::Context::default());
        let region = &p.tracks[0].regions[0];
        assert_eq!(region.name, "Vox");
        assert!((region.len_bars - 1.0).abs() < 1e-3, "{}", region.len_bars);
        assert!(region.audio.is_some());
        assert!(!is_open(&p, 0, 0), "no longer being recorded");
        assert_eq!(p.song_rev, rev + 1, "the engine hears about it");
    }

    /// An audio region of the file at `path` on track 0, as a recording would leave it.
    fn recorded(p: &mut Project, path: &std::path::Path, start_bar: f32) -> usize {
        p.tracks[0].regions.push(Region {
            name: "*Vox".into(),
            start_bar,
            len_bars: 0.01,
            notes: Vec::new(),
            controls: Vec::new(),
            folder: None,
            auto: Vec::new(),
            generated: None,
            audio: None,
            params: RegionParams::default(),
        });
        let r = p.tracks[0].regions.len() - 1;
        let file = AudioFile::open(path).unwrap();
        let start = p.bar_to_beats(f64::from(start_bar) - 1.0);
        apply_take(p, 0, r, start, None, file, &egui::Context::default())
    }

    #[test]
    fn recording_over_an_audio_region_makes_takes_and_replace_mode_takes_its_place() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.wav"), dir.path().join("b.wav"));
        // At 120 BPM two seconds are a bar of 4/4.
        prowave_engine::write_wav(&a, &vec![0.1; 48_000 * 2 * 8], 48_000).unwrap();
        prowave_engine::write_wav(&b, &vec![0.2; 48_000 * 2 * 2], 48_000).unwrap();
        let mut p = project(&[TrackKind::Audio]);
        p.tempo = 120.0;
        p.tracks[0].name = "Vox".into();
        recorded(&mut p, &a, 1.0);
        assert_eq!(p.tracks[0].regions[0].len_bars, 4.0);
        p.tracks[0].regions[0].params.gain_db = -6.0;
        let r = recorded(&mut p, &b, 2.0);
        assert_eq!(p.tracks[0].regions.len(), 1, "they are one folder now");
        let region = &p.tracks[0].regions[r];
        assert_eq!((region.start_bar, region.len_bars), (1.0, 4.0));
        let folder = region.folder.as_ref().unwrap();
        assert_eq!(folder.takes.len(), 2);
        assert_eq!(folder.takes[0].gain_db, -6.0, "the old take keeps its gain");
        assert_eq!(
            folder.comp.iter().map(|s| s.take).collect::<Vec<_>>(),
            [0, 1, 0],
            "the new take plays where it was recorded"
        );
        // Over the folder: one more take.
        let r = recorded(&mut p, &b, 3.0);
        let folder = p.tracks[0].regions[r].folder.as_ref().unwrap();
        assert_eq!(folder.takes.len(), 3);

        let mut p = project(&[TrackKind::Audio]);
        p.tempo = 120.0;
        p.tracks[0].name = "Vox".into();
        recorded(&mut p, &a, 1.0);
        p.rec_replace = true;
        recorded(&mut p, &b, 2.0);
        let mut bars: Vec<(f32, f32)> = p.tracks[0]
            .regions
            .iter()
            .map(|r| (r.start_bar, r.len_bars))
            .collect();
        bars.sort_by(|x, y| x.0.total_cmp(&y.0));
        assert_eq!(bars, [(1.0, 1.0), (2.0, 1.0), (3.0, 2.0)]);
        assert!(p.tracks[0].regions.iter().all(|r| r.folder.is_none()));
    }

    #[test]
    fn a_region_that_was_deleted_is_not_written_to() {
        let p = project(&[TrackKind::Audio]);
        assert!(!is_open(&p, 0, 0));
        assert!(!is_open(&p, 5, 0));
    }

    #[test]
    fn armed_tracks_meter_their_input() {
        let mut p = project(&[TrackKind::Audio, TrackKind::Audio]);
        p.tracks[0].rec = true;
        p.tracks[0].input = InputSource::Stereo(0);
        let rec = AudioRecorder::default();
        let mut meters = vec![(0.5, 0.5), (0.5, 0.5)];
        rec.show_levels(&p, &mut meters);
        assert_eq!(meters, vec![(0.5, 0.5), (0.5, 0.5)], "the input is closed");
    }

    /// The whole path on real hardware: arm, record, stop, a region with the audio. Needs an
    /// input device, so it runs only on request:
    /// `cargo test -p prowave-app records_from_the_real_input -- --ignored --nocapture`
    #[test]
    #[ignore = "needs an audio input device"]
    fn records_from_the_real_input() {
        let ctx = egui::Context::default();
        let mut p = project(&[TrackKind::Audio]);
        p.tracks[0].name = "Vox".into();
        p.tracks[0].rec = true;
        let (mut rec, mut choice) = (AudioRecorder::default(), InputChoice::default());
        assert_eq!(rec.update(&mut choice, &mut p, 0, &ctx), Outcome::Nothing);
        assert!(rec.listening(), "arming opens the input");

        p.position_beats = 4.0;
        p.recording = true;
        p.playing = true;
        for _ in 0..50 {
            assert_eq!(rec.update(&mut choice, &mut p, 0, &ctx), Outcome::Nothing);
            std::thread::sleep(std::time::Duration::from_millis(20));
            p.position_beats += 0.04;
        }
        assert_eq!(rec.regions().collect::<Vec<_>>(), vec![(0, 0)]);
        assert!(
            p.tracks[0].regions[0].len_bars > 0.2,
            "the red region grows"
        );

        p.playing = false;
        let outcome = rec.update(&mut choice, &mut p, 0, &ctx);
        assert_eq!(outcome, Outcome::Finished(Some((0, 0)), None));
        assert!(!p.recording && !rec.active());
        let region = &p.tracks[0].regions[0];
        let file = &region.audio.as_ref().unwrap().file;
        println!("recorded {} frames at {} Hz", file.frames, file.sample_rate);
        assert!(file.duration_secs() > 0.8, "{}", file.duration_secs());
        assert_eq!(region.start_bar, 2.0);
        assert_eq!(region.name, "Vox");

        p.tracks[0].rec = false;
        rec.update(&mut choice, &mut p, 0, &ctx);
        assert!(!rec.listening(), "the input closes when nothing wants it");
    }
}
