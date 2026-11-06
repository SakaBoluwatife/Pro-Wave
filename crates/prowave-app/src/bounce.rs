//! Bounce and export: the project played through the real engine, faster than real time, into a
//! file. A second engine does the work with its own instances of the plugins, set up the way the
//! running ones are, so a file sounds like what is heard: effects, automation, delay compensation.
//!
//! The render runs on a thread of its own. The plugins' UI-thread halves stay here, on the thread
//! they must stay on, until it is done. Delay compensation makes the whole mix start late, so that
//! much is left off the start of the file, and the end waits for tails to ring out.
use crate::engine_sync::EngineSync;
use crate::model::{Project, TrackKind};
use prowave_engine::{Command, Engine, EngineCore};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Frames rendered at a time.
const RENDER_BLOCK: usize = 512;
/// Used when there is no sound device to take the rate from.
const DEFAULT_RATE: u32 = 48_000;
/// A tail longer than this is cut.
const MAX_TAIL_SECONDS: f64 = 30.0;
/// A tail has ended when it stays under this level for this long.
const TAIL_SILENCE: f32 = 0.0001;
const TAIL_SILENT_SECONDS: f64 = 0.25;
/// How long a block may wait for the disk before it goes on without it.
const DISK_PATIENCE: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BitDepth {
    Int16,
    #[default]
    Int24,
    Float32,
}

impl BitDepth {
    pub const ALL: [BitDepth; 3] = [BitDepth::Int16, BitDepth::Int24, BitDepth::Float32];

    pub fn name(self) -> &'static str {
        match self {
            BitDepth::Int16 => "16-bit",
            BitDepth::Int24 => "24-bit",
            BitDepth::Float32 => "32-bit float",
        }
    }

    fn bytes(self) -> u16 {
        match self {
            BitDepth::Int16 => 2,
            BitDepth::Int24 => 3,
            BitDepth::Float32 => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FileFormat {
    #[default]
    Wav,
    Flac,
    Mp3,
}

impl FileFormat {
    pub const ALL: [FileFormat; 3] = [FileFormat::Wav, FileFormat::Flac, FileFormat::Mp3];

    pub fn name(self) -> &'static str {
        match self {
            FileFormat::Wav => "WAV",
            FileFormat::Flac => "FLAC",
            FileFormat::Mp3 => "MP3",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            FileFormat::Wav => "wav",
            FileFormat::Flac => "flac",
            FileFormat::Mp3 => "mp3",
        }
    }

    /// The depth a file in this format has, when `asked` is what was asked for: FLAC has no float,
    /// and MP3 takes float in and has no depth of its own.
    fn depth(self, asked: BitDepth) -> BitDepth {
        match (self, asked) {
            (FileFormat::Flac, BitDepth::Float32) => BitDepth::Int24,
            (FileFormat::Mp3, _) => BitDepth::Float32,
            _ => asked,
        }
    }
}

/// Writes a WAV file as it is rendered, next to its final name until it is finished, so nothing
/// ever sees half a file.
struct WavWriter {
    file: std::io::BufWriter<std::fs::File>,
    part: PathBuf,
    path: PathBuf,
    depth: BitDepth,
    bytes: u64,
    /// For the dither of 16-bit files.
    noise: u32,
}

impl WavWriter {
    fn create(path: &Path, depth: BitDepth, rate: u32) -> std::io::Result<Self> {
        let part = path.with_extension("wav.part");
        let mut file = std::io::BufWriter::new(std::fs::File::create(&part)?);
        let bytes_per_frame = u32::from(depth.bytes()) * 2;
        let format: u16 = if depth == BitDepth::Float32 { 3 } else { 1 };
        file.write_all(b"RIFF\0\0\0\0WAVEfmt ")?;
        file.write_all(&16u32.to_le_bytes())?;
        file.write_all(&format.to_le_bytes())?;
        file.write_all(&2u16.to_le_bytes())?;
        file.write_all(&rate.to_le_bytes())?;
        file.write_all(&(rate * bytes_per_frame).to_le_bytes())?;
        file.write_all(&(bytes_per_frame as u16).to_le_bytes())?;
        file.write_all(&(depth.bytes() * 8).to_le_bytes())?;
        file.write_all(b"data\0\0\0\0")?;
        Ok(Self {
            file,
            part,
            path: path.to_path_buf(),
            depth,
            bytes: 0,
            noise: 0x9E37_79B9,
        })
    }

    /// A random number, uniform in -0.5..0.5.
    fn random(&mut self) -> f32 {
        self.noise ^= self.noise << 13;
        self.noise ^= self.noise >> 17;
        self.noise ^= self.noise << 5;
        self.noise as f32 / u32::MAX as f32 - 0.5
    }

    fn write(&mut self, samples: &[f32]) -> std::io::Result<()> {
        for &s in samples {
            let s = if s.is_finite() { s } else { 0.0 };
            match self.depth {
                BitDepth::Float32 => self.file.write_all(&s.to_le_bytes())?,
                BitDepth::Int16 => {
                    // Triangular dither, as losing the bits of a master should have.
                    let dither = self.random() + self.random();
                    let v = (s * 32767.0 + dither).round().clamp(-32768.0, 32767.0) as i16;
                    self.file.write_all(&v.to_le_bytes())?;
                }
                BitDepth::Int24 => {
                    let v = (f64::from(s) * 8_388_607.0)
                        .round()
                        .clamp(-8_388_608.0, 8_388_607.0) as i32;
                    self.file.write_all(&v.to_le_bytes()[..3])?;
                }
            }
        }
        self.bytes += samples.len() as u64 * u64::from(self.depth.bytes());
        if self.bytes > u64::from(u32::MAX) - 64 {
            return Err(std::io::Error::other("too long for a WAV file"));
        }
        Ok(())
    }

    fn finish(mut self) -> std::io::Result<PathBuf> {
        let data = self.bytes as u32;
        self.file.seek(SeekFrom::Start(4))?;
        self.file.write_all(&(36 + data).to_le_bytes())?;
        self.file.seek(SeekFrom::Start(40))?;
        self.file.write_all(&data.to_le_bytes())?;
        self.file.flush()?;
        drop(self.file);
        std::fs::rename(&self.part, &self.path)?;
        Ok(self.path)
    }

    fn abandon(self) {
        let part = self.part.clone();
        drop(self);
        let _ = std::fs::remove_file(part);
    }
}

/// What to do with the file once it is made.
#[derive(Clone, Debug, PartialEq)]
pub enum After {
    /// Only tell the user.
    Report,
    /// A new audio track holding it, with the regions it was made from muted.
    NewTrack {
        name: String,
        bar: f64,
        sources: Vec<(usize, usize)>,
    },
    /// It takes the place of these regions on track `track`.
    Replace {
        track: usize,
        bar: f64,
        sources: Vec<(usize, usize)>,
    },
    /// Track `track` is frozen with it.
    Freeze { track: usize },
    /// It goes on a new track below `track`, with the same fader, and `track` is switched off.
    Commit { track: usize, name: String },
    /// It takes the place of the stretch `from_beat..to_beat` of region `region` on track `track`,
    /// made by the effects named here. The file the region played stays as it was.
    Process {
        track: usize,
        region: usize,
        from_beat: f64,
        to_beat: f64,
        /// The hosted plugins that are heard, by uid, and the built-in effects, by their place.
        plugins: Vec<u64>,
        inserts: Vec<usize>,
    },
}

/// One file to make.
#[derive(Clone, Debug, PartialEq)]
pub struct Render {
    pub path: PathBuf,
    pub format: FileFormat,
    pub from_beat: f64,
    pub to_beat: f64,
    pub depth: BitDepth,
    /// The file's sample rate; 0 for the engine's.
    pub out_rate: u32,
    /// Only this track is heard.
    pub solo: Option<usize>,
    pub after: After,
}

/// The last beat anything plays on.
pub fn project_end(p: &Project) -> f64 {
    p.tracks
        .iter()
        .flat_map(|t| &t.regions)
        .filter(|r| !r.params.mute)
        .map(|r| p.region_end_beats(r))
        .fold(0.0, f64::max)
}

/// The project as a render should play it. What it plays stays, what is about playing live goes.
fn render_project(p: &Project, render: &Render) -> Project {
    let mut copy = p.clone();
    copy.playing = false;
    copy.recording = false;
    copy.counting_in = false;
    copy.cycle = false;
    copy.metronome = false;
    copy.count_in = false;
    copy.position_beats = render.from_beat;
    for (i, t) in copy.tracks.iter_mut().enumerate() {
        t.rec = false;
        t.monitor = false;
        if let Some(only) = render.solo {
            t.solo = i == only;
            if i == only {
                t.mute = false;
            }
        }
    }
    if let After::NewTrack { sources, .. } | After::Replace { sources, .. } = &render.after {
        // Bouncing in place takes only what was selected, even if that was muted, and it will go
        // through the output again when played.
        copy.master_vol = 1.0;
        for (t, track) in copy.tracks.iter_mut().enumerate() {
            for (r, region) in track.regions.iter_mut().enumerate() {
                if render.solo == Some(t) {
                    region.params.mute = !sources.contains(&(t, r));
                }
            }
        }
    }
    if let After::Replace { track, .. } = render.after {
        // The file stays on this track, so its effects and fader would apply a second time.
        if let Some(t) = copy.tracks.get_mut(track) {
            t.inserts.clear();
            t.plugins.clear();
            t.sends.clear();
            t.volume_db = 0.0;
            t.pan = 0.0;
            t.output = 0;
            t.vca = None;
            t.auto = crate::model::Automation::default();
        }
    }
    if let After::Freeze { track } | After::Commit { track, .. } = render.after {
        // What the strip makes before its fader: that, and what comes after it, stay live.
        copy.master_vol = 1.0;
        copy.master_plugins.clear();
        if let Some(t) = copy.tracks.get_mut(track) {
            t.sends.clear();
            t.volume_db = 0.0;
            t.pan = 0.0;
            t.output = 0;
            t.vca = None;
            t.auto.volume.clear();
            t.auto.pan.clear();
        }
    }
    if let After::Process {
        track,
        region,
        plugins,
        inserts,
        ..
    } = &render.after
    {
        // The region alone, through the chosen effects and nothing else. Effects, fader and output
        // stay on the track and go on applying to what is played, so they are not printed.
        copy.master_vol = 1.0;
        copy.master_plugins.clear();
        if let Some(t) = copy.tracks.get_mut(*track) {
            for (r, x) in t.regions.iter_mut().enumerate() {
                x.params.mute = r != *region;
            }
            t.plugins.retain(|x| plugins.contains(&x.uid));
            let mut place = 0;
            t.inserts.retain(|_| {
                place += 1;
                inserts.contains(&(place - 1))
            });
            t.sends.clear();
            t.volume_db = 0.0;
            t.pan = 0.0;
            t.output = 0;
            t.vca = None;
            t.auto.volume.clear();
            t.auto.pan.clear();
        }
    }
    // A new song for the new engine, whatever revision the project was at.
    copy.song_rev = copy.song_rev.wrapping_add(1);
    copy
}

struct Finished {
    core: EngineCore,
    result: Result<PathBuf, String>,
}

/// How long the file comes out, in frames, for telling how far along it is.
fn frames_between(p: &Project, from: f64, to: f64, rate: u32) -> u64 {
    let map = p.tempo_map();
    ((map.seconds_at(to) - map.seconds_at(from)).max(0.0) * f64::from(rate)) as u64
}

/// The thread's work. Runs up to the end, then on until the tail has died away.
#[allow(clippy::too_many_arguments)]
fn run_render(
    core: &mut EngineCore,
    writer: &mut WavWriter,
    frames: u64,
    skip: u64,
    rate: u32,
    progress: &AtomicU32,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let mut buf = vec![0.0f32; RENDER_BLOCK * 2];
    let mut done = 0u64;
    let total = frames + skip;
    let max_tail = (MAX_TAIL_SECONDS * f64::from(rate)) as u64;
    let quiet_enough = (TAIL_SILENT_SECONDS * f64::from(rate)) as u64;
    let mut quiet = 0u64;
    // Quiet at the end stays out of the file, so with no tail the file is as long as the range.
    let mut held: Vec<f32> = Vec::new();
    if frames == 0 {
        core.end_of_range();
    }
    while done < total + max_tail {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".to_string());
        }
        // The blocks end exactly where the range does and where the delayed sound of it does.
        let block = RENDER_BLOCK as u64;
        let n = if done < frames {
            block.min(frames - done)
        } else if done < total {
            block.min(total - done)
        } else {
            block
        };
        let waited = Instant::now();
        while !core.clips_ready(n as usize) {
            if waited.elapsed() > DISK_PATIENCE {
                return Err("An audio file could not be read in time".to_string());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let out = &mut buf[..n as usize * 2];
        core.process(out);
        if done < frames && done + n >= frames {
            core.end_of_range();
        }
        // What the delay compensation put in front stays out of the file.
        let from = skip.saturating_sub(done).min(n) as usize;
        if done >= total {
            if out.iter().all(|s| s.abs() < TAIL_SILENCE) {
                quiet += n;
                if quiet >= quiet_enough {
                    break;
                }
                held.extend_from_slice(out);
            } else {
                quiet = 0;
                writer.write(&held).map_err(|e| e.to_string())?;
                held.clear();
                writer.write(out).map_err(|e| e.to_string())?;
            }
        } else {
            writer.write(&out[from * 2..]).map_err(|e| e.to_string())?;
        }
        done += n;
        let permille = (done.min(total) * 1000 / total.max(1)) as u32;
        progress.store(permille, Ordering::Relaxed);
    }
    Ok(())
}

/// Makes the file asked for from the rendered WAV.
fn encode(
    wav: &Path,
    target: &Path,
    format: FileFormat,
    depth: BitDepth,
    rate: u32,
    out_rate: u32,
) -> Result<PathBuf, String> {
    let made = convert(wav, target, format, depth, rate, out_rate);
    let _ = std::fs::remove_file(wav);
    made.map(|()| target.to_path_buf())
}

/// Frames read at a time from the render.
const CONVERT_FRAMES: usize = 65_536;

/// Writes the float render `wav` at another rate and depth, as `out`. Only a change of rate needs
/// all of it in memory at once.
fn write_converted(
    wav: &Path,
    out: &Path,
    depth: BitDepth,
    rate: u32,
    out_rate: u32,
) -> Result<(), String> {
    use std::io::Read;
    let fail = |e: std::io::Error| e.to_string();
    let mut writer = WavWriter::create(out, depth, out_rate).map_err(fail)?;
    let mut input = std::io::BufReader::new(std::fs::File::open(wav).map_err(fail)?);
    input.seek(SeekFrom::Start(44)).map_err(fail)?;
    let floats = |bytes: &[u8]| -> Vec<f32> {
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect()
    };
    let written = if rate == out_rate {
        let mut raw = vec![0u8; CONVERT_FRAMES * 8];
        loop {
            let got = crate::audio_encode::read_all(&mut input, &mut raw).map_err(fail)?;
            if got == 0 {
                break Ok(());
            }
            let chunk = floats(&raw[..got]);
            if let Err(e) = writer.write(&chunk) {
                break Err(e.to_string());
            }
        }
    } else {
        let mut raw = Vec::new();
        input.read_to_end(&mut raw).map_err(fail)?;
        let all = floats(&raw);
        drop(raw);
        let converted = crate::audio_encode::resample(&all, rate, out_rate);
        writer.write(&converted).map_err(fail)
    };
    match written {
        Ok(()) => writer.finish().map(|_| ()).map_err(fail),
        Err(e) => {
            writer.abandon();
            Err(e)
        }
    }
}

fn convert(
    wav: &Path,
    target: &Path,
    format: FileFormat,
    depth: BitDepth,
    rate: u32,
    out_rate: u32,
) -> Result<(), String> {
    if format == FileFormat::Wav {
        return write_converted(wav, target, depth, rate, out_rate);
    }
    let stage = target.with_extension("stage.wav");
    write_converted(wav, &stage, depth, rate, out_rate)?;
    let bits = usize::from(depth.bytes()) * 8;
    let made = match format {
        FileFormat::Flac => crate::audio_encode::flac(&stage, target, bits, out_rate),
        _ => crate::audio_encode::mp3(&stage, target, out_rate),
    };
    let _ = std::fs::remove_file(&stage);
    made
}

/// A render under way.
pub struct Job {
    render: Render,
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
    finished: Receiver<Finished>,
    /// What went wrong setting it up, for when it is done: a plugin missing from the sound.
    warning: Option<String>,
    /// Keeps the plugins' UI-thread halves and the disk thread going. Goes after the engine core.
    sync: EngineSync,
}

impl Job {
    /// Sets up a second engine for the render, on this thread, and starts it on its own.
    pub fn start(p: &Project, render: Render, rate: u32) -> Result<Self, String> {
        let mut copy = render_project(p, &render);
        let rate = if rate == 0 { DEFAULT_RATE } else { rate };
        let (engine, mut core) = Engine::offline(rate);
        let mut sync = EngineSync::new(engine, &copy);
        // The plugins load one a call, and each call's commands need the core to take them.
        let mut settled = 0;
        for _ in 0..4096 {
            sync.sync(&mut copy);
            core.process(&mut []);
            settled = if sync.needs_retry() { 0 } else { settled + 1 };
            if settled >= 3 {
                break;
            }
        }
        let warning = sync.take_plugin_message();
        let skip = sync.total_latency(&copy) as u64;
        let frames = frames_between(&copy, render.from_beat, render.to_beat, rate);
        let format = render.format;
        let depth = format.depth(render.depth);
        let out_rate = if render.out_rate == 0 {
            rate
        } else {
            render.out_rate
        };
        // The render is kept in floating point with all its headroom. The file's rate, depth and
        // format are made from that afterwards, once.
        let wav = render.path.with_extension("tmp.wav");
        let target = render.path.clone();
        let mut writer = WavWriter::create(&wav, BitDepth::Float32, rate)
            .map_err(|e| format!("cannot write {}: {e}", render.path.display()))?;
        core.keep_headroom();
        // Chained plugins need rounds, for the delay to be what the compensation expects.
        if prowave_engine::plugins_chained() {
            let capacity = (prowave_engine::MAX_INSERTS + 1) * prowave_engine::MAX_TRACKS
                + prowave_engine::MAX_INSERTS;
            core.use_plugin_round(prowave_engine::Engine::plugin_round(capacity));
        }
        let start = render.from_beat;
        let send = |sync: &mut EngineSync, cmd| sync.engine.send(cmd);
        if !send(&mut sync, Command::SetPosition { beat: start }) {
            return Err("The bounce could not be started".to_string());
        }
        core.process(&mut []);
        // The disk catches up with the new position before the first frame is asked for.
        let waited = Instant::now();
        while !core.clips_ready(RENDER_BLOCK) && waited.elapsed() < DISK_PATIENCE {
            std::thread::sleep(Duration::from_millis(1));
        }
        if !send(&mut sync, Command::Play) {
            return Err("The bounce could not be started".to_string());
        }
        core.process(&mut []);

        let progress = Arc::new(AtomicU32::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, finished) = channel();
        let (for_thread, stop) = (progress.clone(), cancel.clone());
        std::thread::Builder::new()
            .name("prowave-bounce".into())
            .spawn(move || {
                let ran = run_render(
                    &mut core,
                    &mut writer,
                    frames,
                    skip,
                    rate,
                    &for_thread,
                    &stop,
                );
                let result = match ran {
                    Ok(()) => writer
                        .finish()
                        .map_err(|e| e.to_string())
                        .and_then(|wav| encode(&wav, &target, format, depth, rate, out_rate)),
                    Err(e) => {
                        writer.abandon();
                        Err(e)
                    }
                };
                let _ = tx.send(Finished { core, result });
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            render,
            progress,
            cancel,
            finished,
            warning,
            sync,
        })
    }

    pub fn fraction(&self) -> f32 {
        self.progress.load(Ordering::Relaxed) as f32 / 1000.0
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn file_name(&self) -> String {
        self.render
            .path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
    }

    /// Looks after the plugins while it runs. When it is over, gives back the file made.
    pub fn poll(&mut self) -> Option<(Render, Result<PathBuf, String>)> {
        self.sync.tick_plugins();
        let result = match self.finished.try_recv() {
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err("The bounce stopped unexpectedly".to_string()),
            Ok(Finished { core, result }) => {
                // The audio halves of the plugins go first: their UI-thread halves wait for that.
                drop(core);
                result
            }
        };
        Some((self.render.clone(), result))
    }
}

impl Drop for Job {
    /// The thread holds the audio halves of the plugins, which go before the UI-thread halves here.
    fn drop(&mut self) {
        self.cancel();
        let _ = self.finished.recv_timeout(Duration::from_secs(10));
    }
}

/// Puts a finished file where it was meant to go.
pub fn place(
    p: &mut Project,
    render: &Render,
    file: &Path,
    ctx: &egui::Context,
) -> Result<(), String> {
    let import = |p: &mut Project, bar: f64, onto: Option<usize>| {
        crate::import::import_audio_at(p, file, ctx, bar, onto)
            .map_err(|e| format!("cannot import {}: {e}", file.display()))
    };
    match &render.after {
        After::Report => Ok(()),
        After::Freeze { track } => {
            let region = crate::import::frozen_region(p, file, ctx)
                .map_err(|e| format!("cannot read {}: {e}", file.display()))?;
            if let Some(t) = p.tracks.get_mut(*track) {
                t.frozen = Some(Box::new(region));
                p.song_rev += 1;
            }
            Ok(())
        }
        After::Commit { track, name } => {
            import(p, 1.0, None)?;
            let last = p.tracks.len() - 1;
            if let Some(old) = p.tracks.get(*track).cloned() {
                let new = &mut p.tracks[last];
                new.name = format!("{name} Committed");
                new.volume_db = old.volume_db;
                new.pan = old.pan;
                new.output = old.output;
                new.sends = old.sends.clone();
                new.color = old.color;
                p.tracks[*track].off = true;
                p.move_tracks(&[last], *track + 1);
                p.selected_track = *track + 1;
            }
            p.song_rev += 1;
            Ok(())
        }
        After::NewTrack { name, bar, sources } => {
            import(p, *bar, None)?;
            if let Some(t) = p.tracks.last_mut() {
                t.name = name.clone();
            }
            for (t, r) in sources {
                if let Some(region) = p.tracks.get_mut(*t).and_then(|x| x.regions.get_mut(*r)) {
                    region.params.mute = true;
                }
            }
            p.song_rev += 1;
            Ok(())
        }
        After::Replace {
            track,
            bar,
            sources,
        } => {
            let mut gone = sources.clone();
            gone.sort_unstable_by(|a, b| b.cmp(a));
            for (t, r) in gone {
                if let Some(x) = p.tracks.get_mut(t) {
                    if r < x.regions.len() {
                        x.regions.remove(r);
                    }
                }
            }
            import(p, *bar, Some(*track))
        }
        After::Process {
            track,
            region,
            from_beat,
            to_beat,
            ..
        } => {
            let made = crate::import::frozen_region(p, file, ctx)
                .map_err(|e| format!("cannot read {}: {e}", file.display()))?;
            put_processed(p, *track, *region, (*from_beat, *to_beat), made)
        }
    }
}

/// Cuts the stretch `range` out of region `r` of track `t` and has it play from the start of the
/// region `made` holds. The parts around it go on playing the old file, untouched. What the region
/// did to its sound (gain, fades, reverse) is in the new file now, so the stretch does none of it.
fn put_processed(
    p: &mut Project,
    t: usize,
    r: usize,
    (from, to): (f64, f64),
    made: crate::model::Region,
) -> Result<(), String> {
    let region = p
        .tracks
        .get(t)
        .and_then(|x| x.regions.get(r))
        .ok_or("The region is gone")?;
    let (start, end) = (p.region_start_beats(region), p.region_end_beats(region));
    let edge = crate::region_edit::EDGE_BEATS;
    let middle = if from > start + edge {
        crate::region_edit::split(p, t, r, from)?
    } else {
        r
    };
    if to < end - edge {
        crate::region_edit::split(p, t, middle, to)?;
    }
    let map = p.tempo_map();
    let seconds = map.seconds_at(to) - map.seconds_at(from);
    let target = &mut p.tracks[t].regions[middle];
    target.name = format!("{} Processed", target.name);
    target.audio = made.audio;
    target.params = crate::model::RegionParams {
        source_start: 0.0,
        source_len: seconds as f32,
        ..Default::default()
    };
    p.song_rev += 1;
    Ok(())
}

/// The files of `files` that hold a freeze and that no track in `p` is playing.
fn unused_among(p: &Project, files: Vec<PathBuf>) -> Vec<PathBuf> {
    let in_use: Vec<&Path> = p
        .tracks
        .iter()
        .filter_map(|t| t.frozen.as_ref()?.audio.as_ref())
        .map(|a| a.file.path.as_path())
        .collect();
    files
        .into_iter()
        .filter(|f| {
            let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name.contains(" freeze ") && name.ends_with(".wav") && !in_use.contains(&f.as_path())
        })
        .collect()
}

/// File > Clean Up: the freeze files left behind. They can be made again by freezing again.
pub fn unused_freeze_files(p: &Project) -> Vec<PathBuf> {
    if cfg!(test) {
        return Vec::new();
    }
    let files = std::fs::read_dir(bounce_dir())
        .map(|dir| dir.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    unused_among(p, files)
}

/// Whether a track can be frozen: it has something to make sound with, and regions to play.
pub fn can_freeze(t: &crate::model::Track) -> bool {
    matches!(
        t.kind,
        TrackKind::Audio | TrackKind::Instrument | TrackKind::Drummer
    ) && !t.regions.is_empty()
}

/// Commit: prints track `t` through its plugins and effects to a new audio track below it, and
/// switches `t` off. Nothing is lost: switch `t` on again to go back.
pub fn commit_track(p: &mut Project, s: &mut crate::UiState, t: usize) {
    let Some(track) = p.tracks.get(t) else {
        return;
    };
    if !can_freeze(track) {
        s.status = Some("This track has nothing to commit".to_string());
        return;
    }
    let name = track.name.clone();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let dir = bounce_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        s.status = Some(e.to_string());
        return;
    }
    s.bounce.enqueue(vec![Render {
        path: dir.join(format!("{} commit {stamp}.wav", file_safe(&name))),
        format: FileFormat::Wav,
        out_rate: 0,
        from_beat: 0.0,
        to_beat: project_end(p),
        depth: BitDepth::Float32,
        solo: Some(t),
        after: After::Commit { track: t, name },
    }]);
}

/// Freezes track `t`, or thaws it. A frozen track plays a file and runs none of its plugins or
/// effects, which frees what they cost; the fader, pan and sends still work.
pub fn toggle_freeze(p: &mut Project, s: &mut crate::UiState, t: usize) {
    let Some(track) = p.tracks.get_mut(t) else {
        return;
    };
    if track.frozen.take().is_some() {
        p.song_rev += 1;
        return;
    }
    if !can_freeze(track) {
        s.status = Some("This track has nothing to freeze".to_string());
        return;
    }
    let name = track.name.clone();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let dir = bounce_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        s.status = Some(e.to_string());
        return;
    }
    s.bounce.enqueue(vec![Render {
        path: dir.join(format!("{} freeze {stamp}.wav", file_safe(&name))),
        format: FileFormat::Wav,
        out_rate: 0,
        from_beat: 0.0,
        to_beat: project_end(p),
        depth: BitDepth::Float32,
        solo: Some(t),
        after: After::Freeze { track: t },
    }]);
}

/// The renders for each track on its own, into `folder`, named after the project and the track.
pub fn stems(
    p: &Project,
    folder: &Path,
    format: FileFormat,
    depth: BitDepth,
    range: (f64, f64),
) -> Vec<Render> {
    // An Aux alone has nothing to play: what it adds is in the stems of the tracks sending to it.
    let plays = |kind: TrackKind| {
        matches!(
            kind,
            TrackKind::Audio | TrackKind::Instrument | TrackKind::Drummer
        )
    };
    p.tracks
        .iter()
        .enumerate()
        .filter(|(i, t)| plays(t.kind) && !p.is_muted(*i) && !t.regions.is_empty())
        .map(|(i, t)| Render {
            out_rate: 0,
            path: folder.join(format!(
                "{} {:02} {}.{}",
                file_safe(&p.name),
                i + 1,
                file_safe(&t.name),
                format.extension()
            )),
            format,
            from_beat: range.0,
            to_beat: range.1,
            depth,
            solo: Some(i),
            after: After::Report,
        })
        .collect()
}

/// A name without what Windows does not allow in a file name.
pub fn file_safe(name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let clean = clean.trim().trim_end_matches('.');
    if clean.is_empty() {
        "Untitled".to_string()
    } else {
        clean.to_string()
    }
}

/// The time the selected regions take up: where they start and end, in beats.
pub fn selection_range(p: &Project, regions: &[(usize, usize)]) -> Option<(f64, f64)> {
    let mut range: Option<(f64, f64)> = None;
    for (t, r) in regions {
        let region = p.tracks.get(*t)?.regions.get(*r)?;
        let (a, b) = (p.region_start_beats(region), p.region_end_beats(region));
        range = Some(range.map_or((a, b), |(x, y)| (x.min(a), y.max(b))));
    }
    range
}

/// What the dialog is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// The whole mix into one file.
    Mix,
    /// The selected track into a file.
    Track,
    /// Every track into a file of its own.
    Stems,
}

/// What part of the project a bounce covers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RangeChoice {
    #[default]
    Project,
    Cycle,
    Selection,
}

/// Sample rates offered, 0 being the engine's own.
const OUT_RATES: [(u32, &str); 5] = [
    (0, "Same as now"),
    (44_100, "44.1 kHz"),
    (48_000, "48 kHz"),
    (88_200, "88.2 kHz"),
    (96_000, "96 kHz"),
];

/// File > Bounce > Process Selection: what is about to be printed.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcessDialog {
    pub track: usize,
    pub region: usize,
    /// The stretch to print, in bars counted from 1, inside the region.
    pub from_bar: f64,
    pub to_bar: f64,
    /// The region's own bounds, in the same units.
    pub bounds: (f64, f64),
    /// The track's hosted plugins (uid, name) and built-in effects (place, name), and whether each
    /// goes in the print.
    pub plugins: Vec<(u64, String, bool)>,
    pub inserts: Vec<(usize, String, bool)>,
}

/// The one region Process Selection works on, or why there is none.
pub fn process_target(p: &Project, selected: &[(usize, usize)]) -> Result<(usize, usize), String> {
    let &[(t, r)] = selected else {
        return Err("Select one audio region first".to_string());
    };
    let track = p.tracks.get(t).ok_or("Select one audio region first")?;
    let region = track
        .regions
        .get(r)
        .ok_or("Select one audio region first")?;
    if region.audio.is_none() {
        return Err("Process Selection works on audio regions".to_string());
    }
    if region.folder.is_some() {
        return Err("Pick a take first".to_string());
    }
    if track.frozen.is_some() {
        return Err(format!("{} is frozen", track.name));
    }
    Ok((t, r))
}

/// The dialog for a region: the Cycle's stretch of it when the Cycle is on and crosses it, else all
/// of it, through every effect on the track that is not bypassed.
fn process_dialog(p: &Project, t: usize, r: usize) -> ProcessDialog {
    let track = &p.tracks[t];
    let region = &track.regions[r];
    let (start, end) = (p.region_start_beats(region), p.region_end_beats(region));
    let bar = |beat: f64| p.beats_to_bar(beat) + 1.0;
    let (from, to) = if p.cycle && p.cycle_start < end && p.cycle_end > start {
        (p.cycle_start.max(start), p.cycle_end.min(end))
    } else {
        (start, end)
    };
    ProcessDialog {
        track: t,
        region: r,
        from_bar: bar(from),
        to_bar: bar(to),
        bounds: (bar(start), bar(end)),
        plugins: track
            .plugins
            .iter()
            .filter(|x| x.failed.is_none())
            .map(|x| (x.uid, x.name.clone(), !x.bypass))
            .collect(),
        inserts: track
            .inserts
            .iter()
            .enumerate()
            .map(|(k, x)| (k, x.name().to_string(), !x.bypass))
            .collect(),
    }
}

/// Opens the dialog for what is selected, or says why it cannot.
pub fn open_process(p: &Project, s: &mut crate::UiState) {
    match process_target(p, &s.tracks.all_selected()) {
        Ok((t, r)) => s.bounce.process = Some(process_dialog(p, t, r)),
        Err(e) => s.status = Some(e),
    }
}

/// The render a dialog asks for, into `path`. `None` if the stretch is empty.
fn process_render(p: &Project, d: &ProcessDialog, path: PathBuf) -> Option<Render> {
    let (from_beat, to_beat) = (
        p.bar_to_beats(d.from_bar - 1.0),
        p.bar_to_beats(d.to_bar - 1.0),
    );
    (to_beat > from_beat + crate::region_edit::EDGE_BEATS).then(|| Render {
        path,
        format: FileFormat::Wav,
        out_rate: 0,
        from_beat,
        to_beat,
        depth: BitDepth::Float32,
        solo: Some(d.track),
        after: After::Process {
            track: d.track,
            region: d.region,
            from_beat,
            to_beat,
            plugins: d.plugins.iter().filter(|x| x.2).map(|x| x.0).collect(),
            inserts: d.inserts.iter().filter(|x| x.2).map(|x| x.0).collect(),
        },
    })
}

/// The Process Selection dialog.
fn show_process(ctx: &egui::Context, p: &Project, s: &mut crate::UiState) {
    let Some(d) = s.bounce.process.as_mut() else {
        return;
    };
    let mut open = true;
    let mut go = false;
    egui::Window::new("Process Selection")
        .id(egui::Id::unique("process_selection"))
        .open(&mut open)
        .collapsible(false)
        .default_width(340.0)
        .show(ctx, |ui| {
            ui.label(
                "Prints this stretch through the effects ticked here to a new file. The old file \
                 stays as it was, and Undo brings the region back.",
            );
            ui.add_space(4.0);
            let (low, high) = d.bounds;
            egui::Grid::new(egui::Id::unique("process_range"))
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label("From bar");
                    ui.add(
                        egui::DragValue::new(&mut d.from_bar)
                            .speed(0.01)
                            .range(low..=high)
                            .fixed_decimals(3),
                    );
                    ui.end_row();
                    ui.label("To bar");
                    ui.add(
                        egui::DragValue::new(&mut d.to_bar)
                            .speed(0.01)
                            .range(low..=high)
                            .fixed_decimals(3),
                    );
                    ui.end_row();
                });
            ui.add_space(4.0);
            for (_, name, on) in &mut d.inserts {
                ui.checkbox(on, name.as_str());
            }
            for (_, name, on) in &mut d.plugins {
                ui.checkbox(on, name.as_str());
            }
            let chosen = d.plugins.iter().any(|x| x.2) || d.inserts.iter().any(|x| x.2);
            if d.plugins.is_empty() && d.inserts.is_empty() {
                ui.label("This track has no effects. Put the ones to print on it first.");
            }
            ui.add_space(6.0);
            let ready = chosen && d.to_bar > d.from_bar;
            go = ui
                .add_enabled(ready, egui::Button::new("Process"))
                .clicked();
        });
    if go {
        let name = p.tracks.get(d.track).map_or("", |t| t.name.as_str());
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |x| x.as_millis());
        let dir = bounce_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            s.status = Some(e.to_string());
            return;
        }
        let path = dir.join(format!("{} processed {stamp}.wav", file_safe(name)));
        let render = process_render(p, d, path);
        s.bounce.process = None;
        if let Some(render) = render {
            s.bounce.enqueue(vec![render]);
            s.tracks.select_many(Vec::new());
        }
        return;
    }
    if !open {
        s.bounce.process = None;
    }
}

/// The window that asks how, and shows how far along it is.
#[derive(Default)]
pub struct BounceState {
    /// Process Selection, while its dialog is open.
    pub process: Option<ProcessDialog>,
    pub dialog: Option<Purpose>,
    pub format: FileFormat,
    pub depth: BitDepth,
    pub range: RangeChoice,
    /// The file's sample rate; 0 for the engine's.
    pub out_rate: u32,
    /// Renders asked for; `App` starts them, one at a time.
    pub queue: Vec<Render>,
    pub job: Option<Job>,
    /// How many of a batch have been made.
    pub done: usize,
    pub total: usize,
}

impl BounceState {
    pub fn busy(&self) -> bool {
        self.job.is_some() || !self.queue.is_empty()
    }

    /// Asks for `renders`, in order.
    pub fn enqueue(&mut self, renders: Vec<Render>) {
        self.total = renders.len();
        self.done = 0;
        self.queue = renders;
        self.queue.reverse();
    }

    /// Runs what is next. `collect` gets the running plugins' settings into the project first, so
    /// the render starts from what is heard now.
    pub fn poll(&mut self, p: &mut Project, rate: u32, ctx: &egui::Context) -> Option<String> {
        let mut message = None;
        if let Some(job) = self.job.as_mut() {
            ctx.request_repaint_after(Duration::from_millis(60));
            if let Some((render, result)) = job.poll() {
                let warning = job.warning.take();
                self.job = None;
                self.done += 1;
                match result {
                    Ok(file) => {
                        message = Some(match place(p, &render, &file, ctx) {
                            Ok(()) => match warning {
                                Some(w) => format!("Bounced {}, but: {w}", file.display()),
                                None => format!("Bounced {}", file.display()),
                            },
                            Err(e) => e,
                        });
                    }
                    Err(e) => {
                        self.queue.clear();
                        message = Some(e);
                    }
                }
            }
            return message;
        }
        if let Some(next) = self.queue.pop() {
            match Job::start(p, next, rate) {
                Ok(job) => self.job = Some(job),
                Err(e) => {
                    self.queue.clear();
                    message = Some(e);
                }
            }
            ctx.request_repaint();
        }
        message
    }
}

/// The path with `extension` on the end, keeping a name with dots in it whole.
fn with_extension_kept(path: PathBuf, extension: &str) -> PathBuf {
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(extension))
    {
        return path;
    }
    let mut name = path.into_os_string();
    name.push(".");
    name.push(extension);
    PathBuf::from(name)
}

/// Where a bounce placed in the project is kept. Not the sound cache, which is cleaned out.
fn bounce_dir() -> PathBuf {
    crate::recent_files::config_dir()
        .map(|d| d.join("bounces"))
        .unwrap_or_else(|| std::env::temp_dir().join("prowave-bounces"))
}

/// Asks where, then what `dialog` said to make.
fn ask_and_enqueue(p: &Project, s: &mut crate::UiState, purpose: Purpose) {
    let b = &mut s.bounce;
    let selected = selection_range(p, &s.tracks.all_selected());
    let range = match b.range {
        RangeChoice::Cycle if p.cycle_end > p.cycle_start => (p.cycle_start, p.cycle_end),
        RangeChoice::Selection => selected.unwrap_or((0.0, 0.0)),
        _ => (0.0, project_end(p)),
    };
    if range.1 <= range.0 {
        s.status = Some("There is nothing to bounce".to_string());
        return;
    }
    let renders = match purpose {
        Purpose::Mix | Purpose::Track => {
            let track = p.selected_track;
            let (name, solo) = match purpose {
                Purpose::Track => (
                    p.tracks.get(track).map_or("", |t| t.name.as_str()),
                    Some(track),
                ),
                _ => (p.name.as_str(), None),
            };
            let picked = rfd::FileDialog::new()
                .add_filter(b.format.name(), &[b.format.extension()])
                .set_file_name(format!("{}.{}", file_safe(name), b.format.extension()))
                .save_file();
            let Some(path) = picked else { return };
            vec![Render {
                path: with_extension_kept(path, b.format.extension()),
                format: b.format,
                out_rate: b.out_rate,
                from_beat: range.0,
                to_beat: range.1,
                depth: b.depth,
                solo,
                after: After::Report,
            }]
        }
        Purpose::Stems => {
            let Some(folder) = rfd::FileDialog::new().pick_folder() else {
                return;
            };
            let mut all = stems(p, &folder, b.format, b.depth, range);
            for render in &mut all {
                render.out_rate = b.out_rate;
            }
            all
        }
    };
    b.enqueue(renders);
    b.dialog = None;
}

/// Bounces the selected regions: onto the track they are on when they are audio and `replace` is
/// set, else onto a new audio track with the originals muted.
pub fn bounce_selection(p: &Project, s: &mut crate::UiState, replace: bool) {
    let regions = s.tracks.all_selected();
    let Some(range) = selection_range(p, &regions) else {
        s.status = Some("Select a region first".to_string());
        return;
    };
    let track = regions[0].0;
    if regions.iter().any(|(t, _)| *t != track) {
        s.status = Some("Bounce regions of one track at a time".to_string());
        return;
    }
    let bar = p.beats_to_bar(range.0).floor();
    let start_bar = p.beats_to_bar(range.0) + 1.0;
    let name = p.tracks[track].name.clone();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let dir = bounce_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        s.status = Some(e.to_string());
        return;
    }
    let path = dir.join(format!("{} bounce {stamp}.wav", file_safe(&name)));
    let on_audio = p.tracks[track].kind == TrackKind::Audio;
    let after = if replace && on_audio {
        After::Replace {
            track,
            bar: start_bar,
            sources: regions.clone(),
        }
    } else {
        After::NewTrack {
            name: format!("{name} Bounce"),
            bar: bar + 1.0,
            sources: regions.clone(),
        }
    };
    s.bounce.enqueue(vec![Render {
        path,
        format: FileFormat::Wav,
        out_rate: 0,
        from_beat: range.0,
        to_beat: range.1,
        depth: BitDepth::Float32,
        solo: Some(track),
        after,
    }]);
    s.tracks.select_many(Vec::new());
}

/// The window. While something renders it shows how far along it is, with nothing else clickable.
pub fn show(ctx: &egui::Context, p: &Project, s: &mut crate::UiState) {
    if s.bounce.busy() {
        let (fraction, name) = s
            .bounce
            .job
            .as_ref()
            .map_or((0.0, String::new()), |j| (j.fraction(), j.file_name()));
        let (done, total) = (s.bounce.done, s.bounce.total);
        let mut cancel = false;
        egui::Modal::new(egui::Id::unique("bounce_progress")).show(ctx, |ui| {
            ui.set_min_width(360.0);
            ui.label(if total > 1 {
                format!("Bouncing {} of {total}", done + 1)
            } else {
                "Bouncing".to_string()
            });
            ui.label(egui::RichText::new(name).small());
            ui.add(egui::ProgressBar::new(fraction).show_percentage());
            cancel = ui.button("Cancel").clicked();
        });
        if cancel {
            s.bounce.queue.clear();
            if let Some(job) = &s.bounce.job {
                job.cancel();
            }
        }
        return;
    }
    show_process(ctx, p, s);
    let Some(purpose) = s.bounce.dialog else {
        return;
    };
    let mut go = false;
    let mut close = false;
    let title = match purpose {
        Purpose::Mix => "Bounce Project",
        Purpose::Track => "Export Track",
        Purpose::Stems => "Bounce All Tracks",
    };
    let has_cycle = p.cycle_end > p.cycle_start;
    let has_selection = s.tracks.selected.is_some();
    egui::Modal::new(egui::Id::unique("bounce_dialog")).show(ctx, |ui| {
        ui.set_min_width(320.0);
        ui.heading(title);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("Format");
            for format in FileFormat::ALL {
                ui.selectable_value(&mut s.bounce.format, format, format.name());
            }
        });
        ui.horizontal(|ui| {
            let format = s.bounce.format;
            if format == FileFormat::Mp3 {
                ui.label("Quality");
                ui.label("320 kbps");
                return;
            }
            s.bounce.depth = format.depth(s.bounce.depth);
            ui.label("Depth");
            for depth in BitDepth::ALL {
                if format.depth(depth) == depth {
                    ui.selectable_value(&mut s.bounce.depth, depth, depth.name());
                }
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Range");
            ui.selectable_value(&mut s.bounce.range, RangeChoice::Project, "Whole project");
            ui.add_enabled_ui(has_cycle, |ui| {
                ui.selectable_value(&mut s.bounce.range, RangeChoice::Cycle, "Cycle range");
            });
            ui.add_enabled_ui(has_selection, |ui| {
                ui.selectable_value(
                    &mut s.bounce.range,
                    RangeChoice::Selection,
                    "Selected regions",
                );
            });
        });
        ui.horizontal(|ui| {
            ui.label("Sample rate");
            for (rate, name) in OUT_RATES {
                ui.selectable_value(&mut s.bounce.out_rate, rate, name);
            }
        });
        if s.bounce.depth == BitDepth::Int16 && s.bounce.format != FileFormat::Mp3 {
            ui.label(egui::RichText::new("16-bit files get triangular dither.").small());
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            go = ui.button("Bounce…").clicked();
            close = ui.button("Cancel").clicked();
        });
    });
    if close {
        s.bounce.dialog = None;
    } else if go {
        ask_and_enqueue(p, s, purpose);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::model::Note;
    use prowave_engine::AudioFile;

    #[test]
    fn clean_up_takes_only_freeze_files_nothing_plays() {
        let p = Project::empty();
        let found = unused_among(
            &p,
            vec![
                PathBuf::from("Bass freeze 123.wav"),
                PathBuf::from("Bass bounce 123.wav"),
                PathBuf::from("Bass freeze 123.txt"),
            ],
        );
        assert_eq!(found, [PathBuf::from("Bass freeze 123.wav")]);
    }

    const RATE: u32 = 48_000;

    /// Runs a render to its end and gives back the left channel of the file.
    fn render_and_read(p: &Project, render: Render) -> Vec<f32> {
        let path = render.path.clone();
        let mut job = Job::start(p, render, RATE).unwrap();
        let waited = Instant::now();
        loop {
            if let Some((_, result)) = job.poll() {
                result.unwrap();
                break;
            }
            assert!(waited.elapsed() < Duration::from_secs(60), "it finishes");
            std::thread::sleep(Duration::from_millis(2));
        }
        let file = AudioFile::open(&path).unwrap();
        assert!(!path.with_extension("wav.part").exists(), "no part left");
        file.read_stereo()
            .unwrap()
            .chunks(2)
            .map(|f| f[0])
            .collect()
    }

    fn mix_to(path: PathBuf, to_beat: f64) -> Render {
        Render {
            format: FileFormat::Wav,
            out_rate: 0,
            path,
            from_beat: 0.0,
            to_beat,
            depth: BitDepth::Float32,
            solo: None,
            after: After::Report,
        }
    }

    /// A project at 120 BPM, a beat lasting half a second, with `seconds` of a level of 0.5 in a
    /// clip starting at bar `bar`.
    fn audio_project(seconds: f64, bar: f64) -> (Project, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("clip.wav");
        let frames = (seconds * f64::from(RATE)) as usize;
        prowave_engine::write_wav(&wav, &vec![0.5; frames * 2], RATE).unwrap();
        let mut p = Project::empty();
        p.tempo = 120.0;
        p.tracks.clear();
        let ctx = egui::Context::default();
        crate::import::import_audio_at(&mut p, &wav, &ctx, bar, None).unwrap();
        (p, dir)
    }

    fn first_sound(samples: &[f32]) -> usize {
        samples.iter().position(|s| s.abs() > 0.01).unwrap()
    }

    #[test]
    fn an_audio_clip_comes_out_in_its_place_at_its_length() {
        // One second of audio from bar 2, which starts two seconds in.
        let (p, dir) = audio_project(1.0, 2.0);
        let end = project_end(&p);
        let out = render_and_read(&p, mix_to(dir.path().join("mix.wav"), end));
        let rate = RATE as usize;
        let start = first_sound(&out);
        assert!(
            start.abs_diff(2 * rate) < 200,
            "starts two seconds in: {start}"
        );
        let sounding = out.iter().filter(|s| s.abs() > 0.01).count();
        assert!(sounding.abs_diff(rate) < 400, "a second long: {sounding}");
        assert!(out.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn a_trimmed_or_split_clip_plays_as_it_does_live() {
        let (mut p, dir) = audio_project(4.0, 1.0);
        let t = p.tracks.len() - 1;
        // Bar 1 is two seconds. Cut there, and take the first part away.
        let right = crate::region_edit::split(&mut p, t, 0, 4.0).unwrap();
        p.tracks[t].regions[0].params.mute = true;
        assert_eq!(p.tracks[t].regions[right].params.source_start, 2.0);
        let end = project_end(&p);
        let out = render_and_read(&p, mix_to(dir.path().join("mix.wav"), end));
        let rate = RATE as usize;
        let start = first_sound(&out);
        assert!(start.abs_diff(2 * rate) < 200, "{start}");
        let sounding = out.iter().filter(|s| s.abs() > 0.01).count();
        assert!(sounding.abs_diff(2 * rate) < 400, "{sounding}");
    }

    #[test]
    fn delay_compensation_does_not_move_the_mix() {
        // A limiter looks ahead, so everything else is delayed to match; the file starts in time.
        let (mut plain, dir) = audio_project(1.0, 2.0);
        let end = project_end(&plain);
        let before = first_sound(&render_and_read(
            &plain,
            mix_to(dir.path().join("plain.wav"), end),
        ));
        let t = plain.tracks.len() - 1;
        plain.tracks[t].inserts = vec![crate::model::Insert::new(prowave_engine::FxKind::Limiter)];
        let after = first_sound(&render_and_read(
            &plain,
            mix_to(dir.path().join("limited.wav"), end),
        ));
        assert!(after.abs_diff(before) < 200, "{before} and {after}");
    }

    #[test]
    fn notes_play_through_the_sampler_and_mute_and_solo_count() {
        let (mut p, dir) = audio_project(0.5, 3.0);
        let hit = dir.path().join("hit.wav");
        prowave_engine::write_wav(&hit, &vec![0.5; 4800 * 2], RATE).unwrap();
        let keys = p.add_track(TrackKind::Instrument);
        let info = crate::sample_place::sampler_info(&hit);
        crate::plugin_rack::add_plugin(&mut p, keys, &info).unwrap();
        let r = p.add_midi_region(keys, 1.0).unwrap();
        p.tracks[keys].regions[r].notes = vec![Note {
            start: 1.0,
            len: 0.5,
            pitch: 60,
            vel: 127,
            muted: false,
        }];
        let end = project_end(&p);
        // Soloed: only the note, half a second in, and not the clip.
        let solo = Render {
            solo: Some(keys),
            ..mix_to(dir.path().join("keys.wav"), end)
        };
        let out = render_and_read(&p, solo);
        let start = first_sound(&out);
        let block = prowave_engine::PLUGIN_BLOCK;
        assert!(start.abs_diff(RATE as usize / 2) < 200 + block, "{start}");
        let sounding = out.iter().filter(|s| s.abs() > 0.01).count();
        assert!(sounding < 6000, "only the hit: {sounding}");
    }

    #[test]
    fn a_tail_rings_out_past_the_end_and_silence_does_not_add_length() {
        // Nothing after the end: the file stops about where the range does.
        let (mut p, dir) = audio_project(1.0, 1.0);
        let end = project_end(&p);
        let out = render_and_read(&p, mix_to(dir.path().join("dry.wav"), end));
        let dry = out.len();
        assert!(dry < RATE as usize * 3, "{dry}");
        // With a long delay on it, it carries on.
        let t = p.tracks.len() - 1;
        let mut delay = crate::model::Insert::new(prowave_engine::FxKind::Delay);
        delay.params[0] = 500.0;
        delay.params[1] = 0.5;
        delay.params[4] = 1.0;
        p.tracks[t].inserts = vec![delay];
        let out = render_and_read(&p, mix_to(dir.path().join("wet.wav"), end));
        assert!(
            out.len() > dry + RATE as usize / 4,
            "{} and {dry}",
            out.len()
        );
    }

    #[test]
    fn every_depth_writes_a_file_that_reads_back() {
        let (p, dir) = audio_project(0.5, 1.0);
        let end = project_end(&p);
        for depth in BitDepth::ALL {
            let render = Render {
                depth,
                ..mix_to(dir.path().join(format!("{}.wav", depth.bytes())), end)
            };
            let out = render_and_read(&p, render);
            // The clip's 0.5 through the output fader's 0.8.
            let level = out[RATE as usize / 4];
            assert!((level - 0.4).abs() < 0.001, "{}: {level}", depth.name());
        }
    }

    #[test]
    fn in_place_takes_only_the_selection_and_a_replacement_is_dry() {
        let (mut p, dir) = audio_project(1.0, 1.0);
        let t = p.tracks.len() - 1;
        let second = crate::region_edit::split(&mut p, t, 0, 1.0).unwrap();
        p.tracks[t].volume_db = -12.0;
        p.tracks[t].inserts = vec![crate::model::Insert::new(prowave_engine::FxKind::Limiter)];
        let end = project_end(&p);
        let level_and_length = |after: After, name: &str| {
            let render = Render {
                solo: Some(t),
                after,
                ..mix_to(dir.path().join(name), end)
            };
            let out = render_and_read(&p, render);
            let sounding = out.iter().filter(|s| s.abs() > 0.01).count();
            (out[RATE as usize * 3 / 4], sounding)
        };
        // Half a second is the first region, the rest of the second is the second.
        let (replaced, sounding) = level_and_length(
            After::Replace {
                track: t,
                bar: 1.0,
                sources: vec![(t, second)],
            },
            "dry.wav",
        );
        assert!(
            (replaced - 0.5).abs() < 0.001,
            "no fader, no output: {replaced}"
        );
        assert!(sounding.abs_diff(RATE as usize / 2) < 400, "{sounding}");
        // On a track of its own the fader is printed: the new track will not add it.
        let (new_track, _) = level_and_length(
            After::NewTrack {
                name: "B".to_string(),
                bar: 1.0,
                sources: vec![(t, second)],
            },
            "printed.wav",
        );
        assert!((new_track - 0.1256).abs() < 0.002, "{new_track}");
    }

    #[test]
    fn a_cycle_range_is_what_comes_out() {
        let (p, dir) = audio_project(4.0, 1.0);
        let out = render_and_read(
            &p,
            Render {
                from_beat: 2.0,
                to_beat: 4.0,
                ..mix_to(dir.path().join("cycle.wav"), 0.0)
            },
        );
        assert_eq!(out.len(), RATE as usize, "exactly the range: a second");
        assert!(
            out[100].abs() > 0.1,
            "it starts from the range, not from silence"
        );
    }

    #[test]
    fn a_name_with_dots_keeps_them() {
        assert_eq!(
            with_extension_kept("a.b".into(), "wav"),
            PathBuf::from("a.b.wav")
        );
        assert_eq!(
            with_extension_kept("a.WAV".into(), "wav"),
            PathBuf::from("a.WAV")
        );
    }

    #[test]
    fn flac_is_the_same_as_wav_and_mp3_is_close() {
        // Noise, as music does not compress to nothing. The FLAC reader needs a few frames.
        let dir = tempfile::tempdir().unwrap();
        let clip = dir.path().join("noise.wav");
        let mut x = 0x1234_5678u32;
        let noise: Vec<f32> = (0..RATE * 4 * 2)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x as f32 / u32::MAX as f32 - 0.5) * 0.5
            })
            .collect();
        prowave_engine::write_wav(&clip, &noise, RATE).unwrap();
        let mut p = Project::empty();
        p.tempo = 120.0;
        p.tracks.clear();
        let ctx = egui::Context::default();
        crate::import::import_audio_at(&mut p, &clip, &ctx, 1.0, None).unwrap();
        let end = project_end(&p);
        let make = |format: FileFormat, depth: BitDepth, name: &str| {
            let path = dir.path().join(name);
            let out = render_and_read(
                &p,
                Render {
                    format,
                    depth,
                    ..mix_to(path.clone(), end)
                },
            );
            assert!(path.exists());
            assert!(!path.with_extension("tmp.wav").exists(), "no WAV left");
            out
        };
        let rms = |s: &[f32]| (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt();
        let wav = make(FileFormat::Wav, BitDepth::Int24, "a.wav");
        let flac = make(FileFormat::Flac, BitDepth::Int24, "a.flac");
        assert_eq!(flac.len(), wav.len());
        let worst = wav
            .iter()
            .zip(&flac)
            .fold(0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(worst < 1e-6, "FLAC loses nothing: {worst}");
        let flac16 = make(FileFormat::Flac, BitDepth::Int16, "b.flac");
        assert_eq!(flac16.len(), wav.len());
        // An MP3 file is padded to its frames and starts late by the encoder's delay.
        let mp3 = make(FileFormat::Mp3, BitDepth::Int24, "a.mp3");
        assert!(mp3.len().abs_diff(wav.len()) <= 4000, "{}", mp3.len());
        let (a, b) = (rms(&wav), rms(&mp3));
        assert!((a - b).abs() / a < 0.1, "{a} and {b}");
    }

    #[test]
    fn float_keeps_what_is_over_full_scale_and_fixed_point_stops_there() {
        let (mut p, dir) = audio_project(1.0, 1.0);
        let t = p.tracks.len() - 1;
        // 0.5 in the clip, +6 dB of region gain and +6 dB on the fader: about 2.0.
        p.tracks[t].regions[0].params.gain_db = 6.0;
        p.tracks[t].volume_db = 6.0;
        p.master_vol = 1.0;
        let end = project_end(&p);
        let level = |depth, name: &str| {
            let out = render_and_read(
                &p,
                Render {
                    depth,
                    ..mix_to(dir.path().join(name), end)
                },
            );
            out[RATE as usize / 2]
        };
        assert!((level(BitDepth::Float32, "float.wav") - 1.99).abs() < 0.02);
        assert!(level(BitDepth::Int24, "fixed.wav") <= 1.0);
    }

    #[test]
    fn a_file_can_be_made_at_another_sample_rate() {
        let (p, dir) = audio_project(1.0, 1.0);
        let end = project_end(&p);
        let path = dir.path().join("cd.wav");
        let out = render_and_read(
            &p,
            Render {
                depth: BitDepth::Int16,
                out_rate: 44_100,
                ..mix_to(path.clone(), end)
            },
        );
        let file = AudioFile::open(&path).unwrap();
        assert_eq!(file.sample_rate, 44_100);
        assert!(out.len().abs_diff(44_100) < 8, "{}", out.len());
        assert!((out[22_050] - 0.4).abs() < 0.01, "{}", out[22_050]);
        assert!(!path.with_extension("tmp.wav").exists());
    }

    #[test]
    fn a_comp_of_audio_takes_plays_each_stretch_from_its_pass() {
        // Three passes of a second each in one file, each at its own level.
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("passes.wav");
        let levels = [0.1f32, 0.2, 0.3];
        let samples: Vec<f32> = levels
            .iter()
            .flat_map(|l| std::iter::repeat_n(*l, RATE as usize * 2))
            .collect();
        prowave_engine::write_wav(&wav, &samples, RATE).unwrap();
        let mut p = Project::empty();
        p.tempo = 120.0;
        p.master_vol = 1.0;
        p.tracks.clear();
        let ctx = egui::Context::default();
        crate::import::import_audio_at(&mut p, &wav, &ctx, 1.0, None).unwrap();
        // A second is two beats: half a bar, the length of each pass.
        let take = |offset: f32| crate::model::Take {
            audio_offset: Some(offset),
            ..crate::model::Take::default()
        };
        let region = &mut p.tracks[0].regions[0];
        region.len_bars = 0.5;
        let mut folder = crate::model::TakeFolder::new(vec![take(0.0), take(1.0), take(2.0)], 2.0);
        folder.swipe(1, 1.0, 2.0);
        region.folder = Some(folder);
        let end = project_end(&p);
        let out = render_and_read(&p, mix_to(dir.path().join("comp.wav"), end));
        let rate = RATE as usize;
        assert!(
            (out[rate / 4] - 0.3).abs() < 0.01,
            "the last take: {}",
            out[rate / 4]
        );
        assert!(
            (out[rate * 3 / 4] - 0.2).abs() < 0.01,
            "the swipe: {}",
            out[rate * 3 / 4]
        );
        assert!(
            out.len().abs_diff(rate) < 200,
            "as long as the cycle: {}",
            out.len()
        );

        // And it comes back from a file the same.
        let path = crate::project_io::save_project(&p, &dir.path().join("song")).unwrap();
        let (back, _) = crate::project_io::open_project(&path, &ctx).unwrap();
        assert_eq!(
            back.tracks[0].regions[0].folder,
            p.tracks[0].regions[0].folder
        );
    }

    /// The routing as it sounds, not as it looks: a track sent to a bus is heard through the Aux of
    /// that bus at its fader, a track with a bus as its output only there, and an empty bus not at all.
    #[test]
    fn what_the_routing_says_is_what_comes_out() {
        let (mut p, dir) = audio_project(1.0, 1.0);
        p.master_vol = 1.0;
        let aux = p.add_track(TrackKind::Aux);
        let bus = p.aux_bus_of(aux);
        assert_eq!(bus, 1);
        let level = |p: &Project, name: &str| {
            let end = project_end(p);
            let out = render_and_read(p, mix_to(dir.path().join(name), end));
            out[RATE as usize / 2]
        };
        let direct = level(&p, "direct.wav");
        assert!((direct - 0.5).abs() < 0.01, "to Stereo Out: {direct}");

        // Through the bus alone, with the Aux at -6 dB.
        p.tracks[0].output = bus;
        p.tracks[aux].volume_db = -6.0;
        let through = level(&p, "through.wav");
        assert!((through - 0.25).abs() < 0.01, "through the Aux: {through}");

        // The Aux muted: nothing else carries it.
        p.tracks[aux].mute = true;
        let silent = level(&p, "silent.wav");
        assert!(silent.abs() < 0.001, "{silent}");
        p.tracks[aux].mute = false;

        // To a bus nobody listens to: gone.
        p.tracks[0].output = 5;
        let lost = level(&p, "lost.wav");
        assert!(lost.abs() < 0.001, "{lost}");

        // A send adds to the direct sound.
        p.tracks[0].output = 0;
        assert!(crate::routing_matrix::toggle_send(&mut p, 0, bus));
        let send = prowave_engine::send_gain(p.tracks[0].sends[0].level);
        let both = level(&p, "send.wav");
        let expected = 0.5 + 0.5 * send * 0.501;
        assert!((both - expected).abs() < 0.02, "{both} and {expected}");
    }

    #[test]
    fn a_format_only_has_the_depths_it_can_store() {
        assert_eq!(FileFormat::Flac.depth(BitDepth::Float32), BitDepth::Int24);
        assert_eq!(FileFormat::Flac.depth(BitDepth::Int16), BitDepth::Int16);
        assert_eq!(FileFormat::Mp3.depth(BitDepth::Int16), BitDepth::Float32);
        assert_eq!(FileFormat::Wav.depth(BitDepth::Float32), BitDepth::Float32);
    }

    #[test]
    fn a_cancelled_bounce_leaves_nothing_behind() {
        let (p, dir) = audio_project(1.0, 1.0);
        let path = dir.path().join("gone.wav");
        let mut job = Job::start(&p, mix_to(path.clone(), 100_000.0), RATE).unwrap();
        job.cancel();
        let waited = Instant::now();
        let result = loop {
            if let Some((_, result)) = job.poll() {
                break result;
            }
            assert!(waited.elapsed() < Duration::from_secs(30));
            std::thread::sleep(Duration::from_millis(2));
        };
        assert!(result.is_err());
        assert!(!path.exists());
        assert!(!path.with_extension("wav.part").exists());
    }

    #[test]
    fn stems_are_for_the_tracks_that_play_and_names_are_safe() {
        let (mut p, _dir) = audio_project(1.0, 1.0);
        p.name = "My: Song?".to_string();
        let silent = p.add_track(TrackKind::Instrument);
        let muted = p.add_track(TrackKind::Instrument);
        p.add_midi_region(muted, 1.0).unwrap();
        p.tracks[muted].mute = true;
        let renders = stems(
            &p,
            Path::new("out"),
            FileFormat::Flac,
            BitDepth::Int24,
            (0.0, 4.0),
        );
        assert_eq!(
            renders.len(),
            1,
            "not the empty one {silent} or the muted one"
        );
        assert_eq!(renders[0].solo, Some(0));
        assert!(renders[0].path.to_string_lossy().contains("My_ Song_"));
        assert_eq!(renders[0].path.extension().unwrap(), "flac");
        assert_eq!(file_safe("..."), "Untitled");
    }

    #[test]
    fn bouncing_in_place_makes_a_track_or_takes_the_place_of_the_regions() {
        let (mut p, dir) = audio_project(1.0, 1.0);
        let ctx = egui::Context::default();
        let file = dir.path().join("clip.wav");
        let t = p.tracks.len() - 1;
        let render = Render {
            after: After::NewTrack {
                name: "Keys Bounce".to_string(),
                bar: 1.0,
                sources: vec![(t, 0)],
            },
            ..mix_to(dir.path().join("x.wav"), 4.0)
        };
        place(&mut p, &render, &file, &ctx).unwrap();
        assert_eq!(p.tracks.last().unwrap().name, "Keys Bounce");
        assert!(p.tracks[t].regions[0].params.mute, "the original is muted");

        let render = Render {
            after: After::Replace {
                track: t,
                bar: 2.0,
                sources: vec![(t, 0)],
            },
            ..mix_to(dir.path().join("x.wav"), 4.0)
        };
        place(&mut p, &render, &file, &ctx).unwrap();
        assert_eq!(p.tracks[t].regions.len(), 1);
        assert_eq!(p.tracks[t].regions[0].start_bar, 2.0);
    }
}
