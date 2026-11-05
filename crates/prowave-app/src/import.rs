//! Import audio files as new audio tracks (Logic Pro: File > Import > Audio File, or drag-and-drop).
use crate::model::{AudioRegion, Project, Region, RegionParams, Track, TrackKind, TrackParams};
use prowave_engine::{AudioError, AudioFile};
use std::path::Path;
use std::sync::{Arc, OnceLock};

const PEAK_BINS: usize = 4096;

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
    {
        let (file, peaks, ctx) = (file.clone(), peaks.clone(), ctx.clone());
        std::thread::spawn(move || match file.peaks(PEAK_BINS) {
            Ok(v) => {
                let _ = peaks.set(v);
                ctx.request_repaint();
            }
            Err(e) => tracing::warn!("waveform overview failed for {}: {e}", file.path.display()),
        });
    }
    AudioRegion {
        file,
        peaks,
        flex: None,
        follow_bpm: None,
    }
}

/// Adds a new audio track with one region starting at the bar under the playhead.
pub fn import_audio(p: &mut Project, path: &Path, ctx: &egui::Context) -> Result<(), AudioError> {
    let start_bar = p.beats_to_bar(p.position_beats).floor() + 1.0;
    import_audio_at(p, path, ctx, start_bar, None)
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
        generated: None,
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
        color: egui::Color32::from_rgb(0x3f, 0x5f, 0xc0),
        mute: false,
        solo: false,
        rec: false,
        monitor: false,
        input: prowave_engine::InputSource::Mono(0),
        output: 0,
        volume_db: 0.0,
        pan: 0.0,
        inserts: vec![],
        plugins: Vec::new(),
        instrument_plugin: None,
        instrument: None,
        regions: vec![region],
        params: TrackParams::default(),
        category: None,
        icon: None,
        sends: Vec::new(),
        auto: crate::model::Automation::default(),
        group: None,
        height: 1.0,
        indent: 0,
        folded: false,
        vca: None,
        midi_port: None,
        midi_channel: 1,
        midi_fx: Vec::new(),
    });
    p.selected_track = p.tracks.len() - 1;
    p.song_rev += 1;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_dropped_sample_lands_on_the_audio_track_under_it_or_on_a_new_one() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("kick.wav");
        prowave_engine::write_wav(&wav, &vec![0.1; 48_000 * 2], 48_000).unwrap();
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
