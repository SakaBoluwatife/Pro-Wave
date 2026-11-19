//! Tempo stretches rendered in the background, as Logic renders its Flex edits: a region that
//! follows the tempo is stretched once to a file while the project keeps one steady tempo, and
//! then plays that file straight, with nothing stretched live. Live stretching (`disk.rs`) stays
//! for tempo changes and while a render is still being made.
//!
//! The files are kept in a folder of their own in the system's temporary folder, named after the
//! file, the tempo it was made at and the project tempo, so going back to a tempo finds its file
//! again.
use crate::model::Project;
use ovrtone_engine::AudioFile;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How often the regions are looked over for stretches to make.
const SCAN_EVERY: Duration = Duration::from_millis(500);
/// Renders at a time: each reads a whole file and keeps a core busy.
const MAX_JOBS: usize = 1;

#[derive(Default)]
struct Cache {
    ready: HashMap<u64, AudioFile>,
    /// Keys being rendered, or that failed (not tried again this session).
    tried: HashSet<u64>,
    jobs: Vec<(u64, Receiver<Option<AudioFile>>)>,
    last_scan: Option<Instant>,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

fn key(path: &Path, made_at: f64, tempo: f64) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    made_at.to_bits().hash(&mut h);
    tempo.to_bits().hash(&mut h);
    h.finish()
}

fn folder() -> PathBuf {
    std::env::temp_dir().join("ovrtone-stretch")
}

fn file_for(k: u64) -> PathBuf {
    folder().join(format!("{k:016x}.wav"))
}

/// The stretched copy of `path` (made at `made_at` BPM) for the project tempo `tempo`, if it is
/// ready.
pub fn stretched(path: &Path, made_at: f64, tempo: f64) -> Option<AudioFile> {
    let k = key(path, made_at, tempo);
    CACHE.lock().ok()?.as_ref()?.ready.get(&k).cloned()
}

/// Whether region stretches can be rendered: one steady tempo in the whole project.
pub fn steady(p: &Project) -> bool {
    p.tempo_points().is_empty()
}

/// Collects finished renders and starts new ones. Once a frame, on the UI thread.
pub fn poll(p: &mut Project) {
    let Ok(mut guard) = CACHE.lock() else {
        return;
    };
    let cache = guard.get_or_insert_with(Cache::default);
    // Finished renders: the engine is told to play the new files.
    let mut done = false;
    cache.jobs.retain(|(k, rx)| match rx.try_recv() {
        Ok(Some(file)) => {
            cache.ready.insert(*k, file);
            done = true;
            false
        }
        Ok(None) | Err(TryRecvError::Disconnected) => false,
        Err(TryRecvError::Empty) => true,
    });
    if done {
        p.song_rev += 1;
    }
    if cache.last_scan.is_some_and(|at| at.elapsed() < SCAN_EVERY) {
        return;
    }
    cache.last_scan = Some(Instant::now());
    if !steady(p) {
        return;
    }
    let tempo = f64::from(p.tempo);
    let mut found = false;
    for region in p.tracks.iter().flat_map(|t| t.regions.iter()) {
        let Some(a) = region.audio.as_ref() else {
            continue;
        };
        let Some(made_at) = a.follow_bpm else {
            continue;
        };
        // Resampled regions change pitch with the tempo: nothing to render for them.
        if a.flex.as_ref().is_some_and(|f| f.mode == ovrtone_engine::FlexMode::Speed) {
            continue;
        }
        if (made_at - tempo).abs() < 1e-6 {
            continue;
        }
        let k = key(&a.file.path, made_at, tempo);
        if cache.ready.contains_key(&k) || cache.tried.contains(&k) {
            continue;
        }
        // Made in an earlier session.
        let target = file_for(k);
        if target.exists() {
            if let Ok(file) = AudioFile::open(&target) {
                cache.ready.insert(k, file);
                found = true;
                continue;
            }
        }
        if cache.jobs.len() >= MAX_JOBS {
            continue;
        }
        cache.tried.insert(k);
        let (tx, rx) = channel();
        let source = a.file.clone();
        let spawned = std::thread::Builder::new()
            .name("ovrtone-stretch".into())
            .spawn(move || {
                let _ = tx.send(render(&source, made_at, tempo, &target));
            });
        if spawned.is_ok() {
            cache.jobs.push((k, rx));
        }
    }
    if found {
        p.song_rev += 1;
    }
}

/// Stretches `source`, made at `made_at` BPM, to play at `tempo` with its pitch kept, into `target`.
fn render(source: &AudioFile, made_at: f64, tempo: f64, target: &Path) -> Option<AudioFile> {
    let stereo = source.read_stereo().ok()?;
    let n = stereo.len() / 2;
    if n == 0 || made_at <= 0.0 || tempo <= 0.0 {
        return None;
    }
    let frames = ((n as f64) * made_at / tempo).round() as usize;
    let left: Vec<f32> = stereo.iter().step_by(2).copied().collect();
    let right: Vec<f32> = stereo.iter().skip(1).step_by(2).copied().collect();
    drop(stereo);
    let (l, r) = ovrtone_engine::stretch_whole(&left, &right, source.sample_rate, frames);
    drop((left, right));
    let mut out = Vec::with_capacity(frames * 2);
    for (a, b) in l.iter().zip(r.iter()) {
        out.push(*a);
        out.push(*b);
    }
    std::fs::create_dir_all(folder()).ok()?;
    // Written next to its name and renamed, so a half-written file is never taken for a whole one.
    let part = target.with_extension("part");
    ovrtone_engine::write_wav(&part, &out, source.sample_rate).ok()?;
    std::fs::rename(&part, target).ok()?;
    AudioFile::open(target).ok()
}
