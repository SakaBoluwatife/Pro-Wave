//! Strip Silence: cuts the silent passages out of a selected audio region, leaving a region for each
//! stretch that sounds, where it was. The file is not touched, and nothing is moved. The levels of
//! the file are read on a thread of their own; after that the settings can be tried at once.
use crate::model::{Project, Region};
use crate::UiState;
use egui::{pos2, vec2, Color32, Rect, Sense, Stroke};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};

/// How long a step of the file's level is, in seconds.
const STEP: f64 = 0.002;

/// What counts as silence, and what is kept around what sounds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Quieter than this is silence, in dB.
    pub threshold_db: f32,
    /// Silence shorter than this stays: a breath within a phrase. Milliseconds.
    pub min_silence_ms: f32,
    /// Kept before and after what sounds, so its start and its tail are not cut. Milliseconds.
    pub pre_ms: f32,
    pub post_ms: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            threshold_db: -40.0,
            min_silence_ms: 250.0,
            pre_ms: 10.0,
            post_ms: 80.0,
        }
    }
}

/// The stretches of the file that stay, as (start, end) in seconds of the file, from the file's
/// level in steps of `step` seconds. Only what lies between `window.0` and `window.1` counts.
pub fn kept_stretches(
    levels: &[f32],
    step: f64,
    s: &Settings,
    window: (f64, f64),
) -> Vec<(f64, f64)> {
    let threshold = 10f32.powf(s.threshold_db / 20.0);
    let (pre, post) = (f64::from(s.pre_ms) / 1000.0, f64::from(s.post_ms) / 1000.0);
    let min_gap = f64::from(s.min_silence_ms) / 1000.0;
    // What sounds.
    let mut sounding: Vec<(f64, f64)> = Vec::new();
    let mut open: Option<usize> = None;
    for (i, level) in levels.iter().enumerate() {
        match (open, *level >= threshold) {
            (None, true) => open = Some(i),
            (Some(at), false) => {
                sounding.push((at as f64 * step, i as f64 * step));
                open = None;
            }
            _ => {}
        }
    }
    if let Some(at) = open {
        sounding.push((at as f64 * step, levels.len() as f64 * step));
    }
    // With what is kept around it, and the gaps too short to count filled in.
    let mut kept: Vec<(f64, f64)> = Vec::new();
    for (a, b) in sounding {
        let (a, b) = ((a - pre).max(window.0), (b + post).min(window.1));
        if b <= a {
            continue;
        }
        match kept.last_mut() {
            Some(last) if a - last.1 < min_gap => last.1 = b.max(last.1),
            _ => kept.push((a, b)),
        }
    }
    kept
}

/// What of its file a region plays, in seconds.
fn played_window(region: &Region) -> (f64, f64) {
    let first = f64::from(region.params.source_start);
    (
        first,
        first + f64::from(crate::region_edit::played_seconds(region)),
    )
}

/// Whether Strip Silence can work on a region.
fn can_strip(region: &Region) -> Result<(), String> {
    let Some(audio) = region.audio.as_ref() else {
        return Err("Strip Silence is for audio regions".to_string());
    };
    if region.folder.is_some() {
        return Err("Pick a take first: a take folder cannot be cut".to_string());
    }
    if audio.flex.is_some() {
        return Err("Remove the Flex edits first".to_string());
    }
    Ok(())
}

/// Replaces region `r` of track `t` with a region for each of `kept` (seconds of its file). Each
/// sounds where it did before. The first keeps the region's fade in and the last its fade out.
/// Returns their numbers.
pub fn strip(
    p: &mut Project,
    t: usize,
    r: usize,
    kept: &[(f64, f64)],
) -> Result<Vec<usize>, String> {
    let region = p
        .tracks
        .get(t)
        .and_then(|x| x.regions.get(r))
        .ok_or_else(|| "That region is gone".to_string())?;
    can_strip(region)?;
    if kept.is_empty() {
        return Err("Everything in this region is under the threshold".to_string());
    }
    let map = p.tempo_map();
    let origin = p.region_start_beats(region);
    let start = map.seconds_at(origin);
    let (first, last) = played_window(region);
    let reverse = region.params.reverse;
    let mut parts: Vec<(f64, Region)> = Vec::with_capacity(kept.len());
    for &(a, b) in kept {
        let (a, b) = (a.max(first), b.min(last));
        if b <= a {
            continue;
        }
        let into = if reverse { last - b } else { a - first };
        let from = map.beat_at(start + into);
        let to = map.beat_at(start + into + (b - a));
        let mut part = region.clone();
        part.start_bar = p.beats_to_bar(from) as f32 + 1.0;
        part.len_bars = (p.beats_to_bar(to) - p.beats_to_bar(from)) as f32;
        part.params.source_start = a as f32;
        part.params.source_len = (b - a) as f32;
        part.params.fade_in = 0.0;
        part.params.fade_out = 0.0;
        part.params.crossfade = false;
        part.auto = crate::automation::trimmed_lanes(&region.auto, from - origin);
        parts.push((from, part));
    }
    parts.sort_by(|x, y| x.0.total_cmp(&y.0));
    let (fade_in, fade_out) = (region.params.fade_in, region.params.fade_out);
    if let Some(part) = parts.first_mut() {
        part.1.params.fade_in = fade_in;
    }
    if let Some(part) = parts.last_mut() {
        part.1.params.fade_out = fade_out;
    }
    if parts.is_empty() {
        return Err("Everything in this region is under the threshold".to_string());
    }
    let track = &mut p.tracks[t];
    track.regions.remove(r);
    let at = track.regions.len();
    track.regions.extend(parts.into_iter().map(|x| x.1));
    p.song_rev += 1;
    Ok((at..p.tracks[t].regions.len()).collect())
}

/// The window, while it is open.
pub struct Open {
    track: usize,
    region: usize,
    path: PathBuf,
    /// The levels, once read.
    levels: Option<Vec<f32>>,
    job: Option<Receiver<Result<Vec<f32>, String>>>,
    settings: Settings,
}

/// Strip Silence's state.
#[derive(Default)]
pub struct StripSilence {
    pub open: Option<Open>,
}

/// Opens the window for region `r` of track `t` and starts reading the file's levels.
pub fn open(p: &Project, s: &mut UiState, t: usize, r: usize) {
    let Some(region) = p.tracks.get(t).and_then(|x| x.regions.get(r)) else {
        return;
    };
    if let Err(e) = can_strip(region) {
        s.status = Some(e);
        return;
    }
    let Some(audio) = region.audio.as_ref() else {
        return;
    };
    let file = audio.file.clone();
    let path = file.path.clone();
    let (tx, job) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("ovrtone-strip-silence".into())
        .spawn(move || {
            let frames = (STEP * f64::from(file.sample_rate)).max(1.0) as usize;
            let _ = tx.send(file.levels(frames).map_err(|e| e.to_string()));
        });
    if let Err(e) = spawned {
        s.status = Some(format!("cannot read the region's file: {e}"));
        return;
    }
    s.strip_silence.open = Some(Open {
        track: t,
        region: r,
        path,
        levels: None,
        job: Some(job),
        settings: Settings::default(),
    });
}

/// The picture of what stays: the levels of the region, with what goes dimmed.
fn preview(ui: &mut egui::Ui, levels: &[f32], kept: &[(f64, f64)], window: (f64, f64)) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 70.0), Sense::hover());
    let g = ui.painter_at(rect);
    g.rect_filled(rect, 3.0, Color32::from_black_alpha(90));
    let span = (window.1 - window.0).max(1e-6);
    let x_of = |sec: f64| rect.left() + ((sec - window.0) / span) as f32 * rect.width();
    for (a, b) in kept {
        g.rect_filled(
            Rect::from_min_max(pos2(x_of(*a), rect.top()), pos2(x_of(*b), rect.bottom())),
            0.0,
            crate::theme::accent().gamma_multiply(0.45),
        );
    }
    let mid = rect.center().y;
    let top = levels.iter().copied().fold(1e-3, f32::max);
    let columns = rect.width().max(1.0) as usize;
    for c in 0..columns {
        let from = window.0 + span * c as f64 / columns as f64;
        let to = window.0 + span * (c + 1) as f64 / columns as f64;
        let bins = (from / STEP) as usize..((to / STEP).ceil() as usize).min(levels.len());
        let level = levels
            .get(bins)
            .map_or(0.0, |x| x.iter().copied().fold(0.0, f32::max));
        let h = level / top * rect.height() * 0.48;
        let x = rect.left() + c as f32;
        g.line_segment(
            [pos2(x, mid - h), pos2(x, mid + h)],
            Stroke::new(1.0, crate::theme::gray(0xc8)),
        );
    }
}

/// The window.
pub fn show(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    let Some(open) = s.strip_silence.open.as_mut() else {
        return;
    };
    // The region can have gone or moved while the window was up.
    let region = p
        .tracks
        .get(open.track)
        .and_then(|x| x.regions.get(open.region))
        .filter(|x| x.audio.as_ref().is_some_and(|a| a.file.path == open.path));
    let Some(region) = region else {
        s.strip_silence.open = None;
        return;
    };
    let window = played_window(region);
    let mut failed = None;
    if let Some(job) = &open.job {
        match job.try_recv() {
            Ok(Ok(levels)) => {
                open.levels = Some(levels);
                open.job = None;
            }
            Ok(Err(e)) => failed = Some(format!("Strip Silence: {e}")),
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
            Err(TryRecvError::Disconnected) => {
                failed = Some("Strip Silence: reading the file stopped".to_string());
            }
        }
    }
    if failed.is_some() {
        s.status = failed;
        s.strip_silence.open = None;
        return;
    }
    let mut window_open = true;
    let (mut run, mut cancel) = (false, false);
    let kept = open
        .levels
        .as_deref()
        .map(|levels| kept_stretches(levels, STEP, &open.settings, window));
    egui::Window::new("Strip Silence")
        .id(egui::Id::unique("strip_silence"))
        .open(&mut window_open)
        .default_width(420.0)
        .show(ctx, |ui| {
            let d = &mut open.settings;
            egui::Grid::new(egui::Id::unique("strip_silence_grid")).show(ui, |ui| {
                ui.label("Threshold");
                ui.add(egui::Slider::new(&mut d.threshold_db, -80.0..=-6.0).suffix(" dB"));
                ui.end_row();
                ui.label("Shortest silence");
                ui.add(egui::Slider::new(&mut d.min_silence_ms, 10.0..=4000.0).suffix(" ms"));
                ui.end_row();
                ui.label("Keep before");
                ui.add(egui::Slider::new(&mut d.pre_ms, 0.0..=500.0).suffix(" ms"));
                ui.end_row();
                ui.label("Keep after");
                ui.add(egui::Slider::new(&mut d.post_ms, 0.0..=1000.0).suffix(" ms"));
                ui.end_row();
            });
            match (&open.levels, &kept) {
                (Some(levels), Some(kept)) => {
                    preview(ui, levels, kept, window);
                    ui.label(match kept.len() {
                        0 => "Everything is under the threshold".to_string(),
                        1 => "1 region".to_string(),
                        n => format!("{n} regions"),
                    });
                }
                _ => {
                    ui.spinner();
                    ui.label("Reading the region's file…");
                }
            }
            ui.horizontal(|ui| {
                let ready = kept.as_ref().is_some_and(|k| !k.is_empty());
                if ui.add_enabled(ready, egui::Button::new("Strip")).clicked() {
                    run = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
    if run {
        let (t, r) = (open.track, open.region);
        match strip(p, t, r, &kept.unwrap_or_default()) {
            Ok(made) => {
                s.tracks
                    .select_many(made.into_iter().map(|r| (t, r)).collect());
            }
            Err(e) => s.status = Some(e),
        }
    }
    if run || cancel || !window_open {
        s.strip_silence.open = None;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Levels at 10 ms a step: silence, then sound, each as long as a number says.
    fn levels(stretches: &[(usize, f32)]) -> Vec<f32> {
        stretches
            .iter()
            .flat_map(|(n, level)| std::iter::repeat_n(*level, *n))
            .collect()
    }

    const NONE: Settings = Settings {
        threshold_db: -40.0,
        min_silence_ms: 0.0,
        pre_ms: 0.0,
        post_ms: 0.0,
    };

    fn close(got: &[(f64, f64)], want: &[(f64, f64)]) {
        assert_eq!(got.len(), want.len(), "{got:?}");
        for (g, w) in got.iter().zip(want) {
            assert!(
                (g.0 - w.0).abs() < 1e-9 && (g.1 - w.1).abs() < 1e-9,
                "{got:?}"
            );
        }
    }

    #[test]
    fn what_sounds_is_found_and_the_threshold_decides_what_that_is() {
        // 1 s silence, 1 s loud, 1 s quiet, 1 s loud.
        let l = levels(&[(100, 0.0), (100, 0.5), (100, 0.005), (100, 0.5)]);
        close(
            &kept_stretches(&l, 0.01, &NONE, (0.0, 4.0)),
            &[(1.0, 2.0), (3.0, 4.0)],
        );
        // A lower threshold hears the quiet part.
        let low = Settings {
            threshold_db: -60.0,
            ..NONE
        };
        close(&kept_stretches(&l, 0.01, &low, (0.0, 4.0)), &[(1.0, 4.0)]);
        // All silence, and a very high threshold.
        assert!(kept_stretches(&[0.0; 50], 0.01, &NONE, (0.0, 1.0)).is_empty());
        let high = Settings {
            threshold_db: -3.0,
            ..NONE
        };
        assert!(kept_stretches(&l, 0.01, &high, (0.0, 4.0)).is_empty());
    }

    #[test]
    fn short_silences_stay_and_what_is_kept_around_sound_is_kept() {
        let l = levels(&[(100, 0.0), (50, 0.5), (20, 0.0), (50, 0.5), (100, 0.0)]);
        // 200 ms of silence is shorter than 250 ms.
        let phrase = Settings {
            min_silence_ms: 250.0,
            ..NONE
        };
        close(
            &kept_stretches(&l, 0.01, &phrase, (0.0, 3.2)),
            &[(1.0, 2.2)],
        );
        // Longer than 100 ms: two regions, with what is kept around each.
        let cut = Settings {
            min_silence_ms: 100.0,
            pre_ms: 20.0,
            post_ms: 30.0,
            ..NONE
        };
        close(
            &kept_stretches(&l, 0.01, &cut, (0.0, 3.2)),
            &[(0.98, 1.53), (1.68, 2.23)],
        );
        // What is kept around sound can make a gap too short to count.
        let wide = Settings {
            pre_ms: 90.0,
            post_ms: 90.0,
            ..cut
        };
        close(
            &kept_stretches(&l, 0.01, &wide, (0.0, 3.2)),
            &[(0.91, 2.29)],
        );
    }

    #[test]
    fn only_what_the_region_plays_counts() {
        let l = levels(&[(100, 0.5), (100, 0.0), (100, 0.5)]);
        close(
            &kept_stretches(&l, 0.01, &NONE, (0.5, 2.5)),
            &[(0.5, 1.0), (2.0, 2.5)],
        );
        assert!(kept_stretches(&l, 0.01, &NONE, (1.2, 1.8)).is_empty());
    }

    /// An audio track with a region of a file of `seconds` at bar 1, at 120 BPM.
    fn project(seconds: f64) -> (Project, usize, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("take.wav");
        ovrtone_engine::write_wav(&wav, &vec![0.1; (seconds * 48_000.0) as usize * 2], 48_000)
            .unwrap();
        let mut p = Project::empty();
        p.tempo = 120.0;
        let ctx = egui::Context::default();
        crate::import::import_audio_at(&mut p, &wav, &ctx, 1.0, None).unwrap();
        let t = p.tracks.len() - 1;
        (p, t, dir)
    }

    #[test]
    fn each_stretch_becomes_a_region_that_sounds_where_it_did() {
        let (mut p, t, _dir) = project(8.0);
        {
            let r = &mut p.tracks[t].regions[0];
            r.params.fade_in = 0.5;
            r.params.fade_out = 0.25;
            r.params.gain_db = -3.0;
        }
        let rev = p.song_rev;
        let made = strip(&mut p, t, 0, &[(1.0, 2.0), (4.0, 6.0)]).unwrap();
        assert!(p.song_rev > rev);
        assert_eq!(made.len(), 2);
        assert_eq!(p.tracks[t].regions.len(), 2);
        let (a, b) = (&p.tracks[t].regions[made[0]], &p.tracks[t].regions[made[1]]);
        // A second is two beats: half a bar of 4/4.
        assert_eq!((a.start_bar, a.len_bars), (1.5, 0.5));
        assert_eq!((a.params.source_start, a.params.source_len), (1.0, 1.0));
        assert_eq!((b.start_bar, b.len_bars), (3.0, 1.0));
        assert_eq!((b.params.source_start, b.params.source_len), (4.0, 2.0));
        assert_eq!(a.params.gain_db, -3.0, "they keep the region's settings");
        assert_eq!((a.params.fade_in, a.params.fade_out), (0.5, 0.0));
        assert_eq!((b.params.fade_in, b.params.fade_out), (0.0, 0.25));
    }

    #[test]
    fn a_reversed_region_keeps_its_stretches_in_the_order_they_sound() {
        let (mut p, t, _dir) = project(8.0);
        p.tracks[t].regions[0].params.reverse = true;
        let made = strip(&mut p, t, 0, &[(1.0, 2.0), (4.0, 6.0)]).unwrap();
        // Backwards, the file's seconds 4 to 6 sound from 2 to 4 seconds in, and 1 to 2 from 6 to 7.
        let (a, b) = (&p.tracks[t].regions[made[0]], &p.tracks[t].regions[made[1]]);
        assert_eq!((a.start_bar, a.params.source_start), (2.0, 4.0));
        assert_eq!((b.start_bar, b.params.source_start), (4.0, 1.0));
    }

    #[test]
    fn a_trimmed_region_keeps_within_what_it_played_and_its_automation_moves_with_it() {
        let (mut p, t, _dir) = project(8.0);
        {
            let r = &mut p.tracks[t].regions[0];
            r.params.source_start = 2.0;
            r.params.source_len = 4.0;
            r.len_bars = 2.0;
            r.start_bar = 5.0;
            r.auto.push(crate::model::RegionLane {
                target: crate::model::AutoTarget::Volume,
                points: vec![
                    crate::model::AutoPoint::new(0.0, -10.0),
                    crate::model::AutoPoint::new(8.0, -20.0),
                ],
            });
        }
        let made = strip(&mut p, t, 0, &[(0.0, 3.0), (5.0, 8.0)]).unwrap();
        let (a, b) = (&p.tracks[t].regions[made[0]], &p.tracks[t].regions[made[1]]);
        assert_eq!((a.params.source_start, a.params.source_len), (2.0, 1.0));
        assert_eq!((a.start_bar, a.len_bars), (5.0, 0.5));
        assert_eq!((b.params.source_start, b.params.source_len), (5.0, 1.0));
        assert_eq!((b.start_bar, b.len_bars), (6.5, 0.5));
        let at_start = ovrtone_engine::lane_value(&b.auto[0].points, 0.0).unwrap();
        assert!((at_start + 17.5).abs() < 0.01, "{at_start}");
    }

    #[test]
    fn what_cannot_be_cut_says_so_and_nothing_is_touched() {
        let (mut p, t, _dir) = project(2.0);
        assert!(strip(&mut p, t, 0, &[]).is_err());
        assert!(strip(&mut p, t, 0, &[(5.0, 6.0)]).is_err(), "outside it");
        assert!(strip(&mut p, t, 9, &[(0.0, 1.0)]).is_err());
        assert_eq!(p.tracks[t].regions.len(), 1);
        let keys = p.add_track(crate::model::TrackKind::Instrument);
        p.add_midi_region(keys, 1.0);
        assert!(strip(&mut p, keys, 0, &[(0.0, 1.0)]).is_err(), "MIDI");
    }

    #[test]
    fn the_window_reads_the_file_shows_the_result_and_strips() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("gaps.wav");
        // Half a second of sound, a second of silence, half a second of sound.
        let mut stereo = vec![0.5; 48_000];
        stereo.extend(vec![0.0; 96_000]);
        stereo.extend(vec![0.5; 48_000]);
        ovrtone_engine::write_wav(&wav, &stereo, 48_000).unwrap();
        let mut p = Project::empty();
        p.tempo = 120.0;
        let ctx = egui::Context::default();
        crate::import::import_audio_at(&mut p, &wav, &ctx, 1.0, None).unwrap();
        let t = p.tracks.len() - 1;
        let mut s = UiState::new();
        open(&p, &mut s, t, 0);
        let mut h = crate::ui_test::Harness::new(900.0, 600.0);
        let mut ready = false;
        for _ in 0..2000 {
            h.frame(vec![], |ui| show(ui.ctx(), &mut p, &mut s));
            let open = s.strip_silence.open.as_ref().unwrap();
            if open.levels.is_some() {
                ready = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(ready, "the levels arrive");
        let open = s.strip_silence.open.as_ref().unwrap();
        let kept = kept_stretches(
            open.levels.as_deref().unwrap(),
            STEP,
            &open.settings,
            (0.0, 2.0),
        );
        assert_eq!(kept.len(), 2);
        assert!(strip(&mut p, t, 0, &kept).is_ok());
        assert_eq!(p.tracks[t].regions.len(), 2);
        // The region went: the window closes by itself.
        p.tracks[t].regions.clear();
        h.frame(vec![], |ui| show(ui.ctx(), &mut p, &mut s));
        assert!(s.strip_silence.open.is_none());
    }
}
