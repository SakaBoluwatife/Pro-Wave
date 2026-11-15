//! Import audio files as new audio tracks (Logic Pro: File > Import > Audio File, or drag-and-drop).
use crate::model::{AudioRegion, Project, Region, RegionParams, Track, TrackKind, TrackParams};
use ovrtone_engine::{AudioError, AudioFile};
use std::path::Path;
use std::sync::{Arc, OnceLock};

/// Points in a file's overview, for when it is seen whole.
const PEAK_BINS: usize = 4096;
/// Frames per point in the fine level, for when it is zoomed in. Long files have many.
const FINE_HOP: u64 = 64;
/// Saved levels kept before the oldest go.
const MAX_SAVED_PEAKS: usize = 500;

/// Where the fine level of `file` is saved. The file's size and date are in the name, so a file
/// that changes is read again.
fn saved_peaks_path(file: &AudioFile) -> Option<std::path::PathBuf> {
    use std::hash::{Hash, Hasher};
    let meta = std::fs::metadata(&file.path).ok()?;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    file.path.hash(&mut hash);
    meta.len().hash(&mut hash);
    meta.modified().ok()?.hash(&mut hash);
    file.frames.hash(&mut hash);
    FINE_HOP.hash(&mut hash);
    let dir = crate::recent_files::config_dir()?.join("peak-cache");
    Some(dir.join(format!("{:016x}.peaks", hash.finish())))
}

fn read_saved_peaks(path: &Path) -> Option<Vec<(f32, f32)>> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() % 8 != 0 || bytes.is_empty() {
        return None;
    }
    let number = |b: &[u8]| f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    Some(
        bytes
            .chunks_exact(8)
            .map(|p| (number(&p[..4]), number(&p[4..])))
            .collect(),
    )
}

/// By way of a temporary name, so that a write cut short never passes for the peaks.
fn save_peaks(path: &Path, peaks: &[(f32, f32)]) -> std::io::Result<()> {
    let dir = path.parent().ok_or(std::io::ErrorKind::NotFound)?;
    std::fs::create_dir_all(dir)?;
    let mut bytes = Vec::with_capacity(peaks.len() * 8);
    for (lo, hi) in peaks {
        bytes.extend_from_slice(&lo.to_le_bytes());
        bytes.extend_from_slice(&hi.to_le_bytes());
    }
    let partial = path.with_extension("part");
    std::fs::write(&partial, bytes)?;
    std::fs::rename(&partial, path)?;
    prune_saved_peaks(dir);
    Ok(())
}

/// Leaves the newest [`MAX_SAVED_PEAKS`] files.
fn prune_saved_peaks(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<_> = entries
        .flatten()
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    if files.len() <= MAX_SAVED_PEAKS {
        return;
    }
    files.sort();
    let extra = files.len() - MAX_SAVED_PEAKS;
    for (_, path) in files.into_iter().take(extra) {
        let _ = std::fs::remove_file(path);
    }
}

/// The whole of `fine` in at most `bins` points, each the widest of those it covers.
fn overview_of(fine: &[(f32, f32)], bins: usize) -> Vec<(f32, f32)> {
    if fine.len() <= bins {
        return fine.to_vec();
    }
    (0..bins)
        .map(|b| {
            let from = b * fine.len() / bins;
            let to = (b + 1) * fine.len() / bins;
            fine[from..to]
                .iter()
                .fold((0.0f32, 0.0f32), |(lo, hi), p| (lo.min(p.0), hi.max(p.1)))
        })
        .collect()
}

/// The fine level of `file`: from where it was saved, else read from the file and saved.
fn fine_peaks(file: &AudioFile) -> Result<Vec<(f32, f32)>, AudioError> {
    let saved = if cfg!(test) {
        None
    } else {
        saved_peaks_path(file)
    };
    if let Some(peaks) = saved.as_deref().and_then(read_saved_peaks) {
        return Ok(peaks);
    }
    let bins = file.frames.div_ceil(FINE_HOP).max(1) as usize;
    let peaks = file.peaks(bins)?;
    if let Some(path) = saved {
        if let Err(e) = save_peaks(&path, &peaks) {
            tracing::warn!(
                "could not save the waveform of {}: {e}",
                file.path.display()
            );
        }
    }
    Ok(peaks)
}

pub fn is_audio_path(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "wav"
                | "wave"
                | "aif"
                | "aiff"
                | "aifc"
                | "flac"
                | "mp3"
                | "caf"
                | "ogg"
                | "oga"
                | "m4a"
                | "mp4"
        )
    })
}

/// Wraps an audio file as a region source and computes its waveform overview on a
/// background thread (the region shows a flat line until it is ready).
pub fn audio_region(file: AudioFile, ctx: &egui::Context) -> AudioRegion {
    let peaks = Arc::new(OnceLock::new());
    let fine_level = Arc::new(OnceLock::new());
    {
        let (file, peaks, fine, ctx) =
            (file.clone(), peaks.clone(), fine_level.clone(), ctx.clone());
        std::thread::spawn(move || match fine_peaks(&file) {
            Ok(v) => {
                let _ = peaks.set(overview_of(&v, PEAK_BINS));
                let _ = fine.set(v);
                ctx.request_repaint();
            }
            Err(e) => tracing::warn!("waveform overview failed for {}: {e}", file.path.display()),
        });
    }
    AudioRegion {
        file,
        peaks,
        fine_peaks: fine_level,
        peak_top: Default::default(),
        flex: None,
        follow_bpm: None,
    }
}

/// Adds a new audio track with one region starting at the bar under the playhead.
pub fn import_audio(p: &mut Project, path: &Path, ctx: &egui::Context) -> Result<(), AudioError> {
    let start_bar = p.beats_to_bar(p.position_beats).floor() + 1.0;
    import_audio_at(p, path, ctx, start_bar, None)
}

/// A region of a whole audio file, starting at bar 1, that goes on no track.
pub fn frozen_region(p: &Project, path: &Path, ctx: &egui::Context) -> Result<Region, AudioError> {
    let file = AudioFile::open(path)?;
    let end = p.tempo_map().advance(0.0, file.duration_secs());
    Ok(Region {
        name: "Frozen".to_string(),
        start_bar: 1.0,
        len_bars: p.beats_to_bar(end) as f32,
        notes: vec![],
        controls: Vec::new(),
        folder: None,
        auto: Vec::new(),
        generated: None,
        gain_line: Vec::new(),
        audio: Some(audio_region(file, ctx)),
        params: RegionParams::default(),
    })
}

/// Puts an audio file at `start_bar` (counted from 1): on track `onto` when that is an audio
/// track, else on a new audio track. Used by drag-and-drop from the Browser.
pub fn import_audio_at(
    p: &mut Project,
    path: &Path,
    ctx: &egui::Context,
    start_bar: f64,
    onto: Option<usize>,
) -> Result<(), AudioError> {
    let file = AudioFile::open(path)?;
    let start_bar = start_bar.max(1.0);
    // The region's length follows the Tempo track from where it starts.
    let map = p.tempo_map();
    let start = p.bar_to_beats(start_bar - 1.0);
    let end = map.advance(start, file.duration_secs());
    let len_bars = p.beats_to_bar(end) - (start_bar - 1.0);
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Audio")
        .to_string();
    let region = Region {
        name: name.clone(),
        start_bar: start_bar as f32,
        len_bars: len_bars as f32,
        notes: vec![],
        controls: Vec::new(),
        folder: None,
        auto: Vec::new(),
        generated: None,
        gain_line: Vec::new(),
        audio: Some(audio_region(file, ctx)),
        params: RegionParams::default(),
    };
    if let Some(t) = onto.filter(|&t| p.tracks.get(t).is_some_and(|t| t.kind == TrackKind::Audio)) {
        p.tracks[t].regions.push(region);
        p.selected_track = t;
        p.song_rev += 1;
        return Ok(());
    }

    p.tracks.push(Track {
        name,
        kind: TrackKind::Audio,
        color: p.next_track_color(),
        mute: false,
        solo: false,
        rec: false,
        monitor: false,
        hardware_out: 0,
        aux_bus: 0,
        object: None,
        bed_pan: false,
        trim_db: 0.0,
        slide_range: crate::model::DEFAULT_SLIDE_RANGE,
        input: ovrtone_engine::InputSource::Mono(0),
        output: 0,
        volume_db: 0.0,
        pan: 0.0,
        inserts: vec![],
        plugins: Vec::new(),
        instrument_plugin: None,
        regions: vec![region],
        params: TrackParams::default(),
        category: None,
        icon: None,
        frozen: None,
        sends: Vec::new(),
        auto: crate::model::Automation::default(),
        group: None,
        height: 1.0,
        indent: 0,
        folded: false,
        vca: None,
        on_timeline: false,
        off: false,
        locked: false,
        reference: false,
        midi_port: None,
        midi_channel: 1,
        midi_in_port: None,
        midi_in_channel: 0,
        midi_fx: Vec::new(),
        show_auto: false,
        alternatives: Vec::new(),
        active_alt: 0,
    });
    if let Some(mut track) = p.tracks.pop() {
        p.spatial_default(&mut track);
        p.tracks.push(track);
    }
    p.selected_track = p.tracks.len() - 1;
    p.song_rev += 1;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn an_overview_keeps_the_widest_of_what_it_covers_and_saved_peaks_read_back() {
        let fine: Vec<(f32, f32)> = (0..1000)
            .map(|i| (-(i as f32) / 1000.0, i as f32 / 1000.0))
            .collect();
        let small = overview_of(&fine, 10);
        assert_eq!(small.len(), 10);
        assert_eq!(small[9].1, 0.999);
        assert_eq!(
            overview_of(&fine[..5], 10).len(),
            5,
            "short files stay as they are"
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.peaks");
        save_peaks(&path, &fine).unwrap();
        assert_eq!(read_saved_peaks(&path).unwrap(), fine);
        std::fs::write(&path, [1u8, 2, 3]).unwrap();
        assert!(
            read_saved_peaks(&path).is_none(),
            "a broken file is read again"
        );
    }

    #[test]
    fn a_dropped_sample_lands_on_the_audio_track_under_it_or_on_a_new_one() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("kick.wav");
        ovrtone_engine::write_wav(&wav, &vec![0.1; 48_000 * 2], 48_000).unwrap();
        let ctx = egui::Context::default();
        let mut p = Project::demo();
        let other = p
            .tracks
            .iter()
            .position(|t| t.kind != TrackKind::Audio)
            .unwrap();

        // Nothing under the pointer: a new track, at the bar it was dropped on.
        let n = p.tracks.len();
        import_audio_at(&mut p, &wav, &ctx, 5.0, None).unwrap();
        assert_eq!(p.tracks.len(), n + 1);
        assert_eq!(p.tracks[n].regions[0].start_bar, 5.0);

        // An audio track: on that track, which gets selected.
        import_audio_at(&mut p, &wav, &ctx, 9.0, Some(n)).unwrap();
        assert_eq!(p.tracks.len(), n + 1);
        assert_eq!(p.tracks[n].regions.len(), 2);
        assert_eq!(p.tracks[n].regions[1].start_bar, 9.0);
        assert_eq!(p.selected_track, n);

        // Any other kind of track, or a row that is gone: a new track.
        let before = p.tracks[other].regions.len();
        import_audio_at(&mut p, &wav, &ctx, 1.0, Some(other)).unwrap();
        assert_eq!(p.tracks.len(), n + 2);
        assert_eq!(p.tracks[other].regions.len(), before);
        import_audio_at(&mut p, &wav, &ctx, 1.0, Some(999)).unwrap();
        assert_eq!(p.tracks.len(), n + 3);

        // Bars count from 1, so a drop before the start is held at the start.
        import_audio_at(&mut p, &wav, &ctx, -3.0, None).unwrap();
        assert_eq!(p.tracks.last().unwrap().regions[0].start_bar, 1.0);
    }
}
