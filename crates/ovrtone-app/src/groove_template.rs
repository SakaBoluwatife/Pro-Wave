//! Groove templates (Logic: Make Groove Template): the timing of a region kept as a quantize grid,
//! with the project. Quantize then moves notes to the places that region played instead of to
//! a straight grid. A MIDI region gives its notes' starts; an audio one the hits found in it,
//! read in the background.
//!
//! A template repeats every `length` beats, counted from the start of a bar, so it lands on the
//! same beats whichever bar a note is in.
use crate::model::{Note, Project, Region};
use crate::UiState;
use ovrtone_project::GrooveDoc;
use std::sync::mpsc::{channel, Receiver, TryRecvError};

/// Places closer than this are one place.
const SAME_PLACE: f64 = 1.0 / 128.0;
/// Most places a template keeps, and most templates a file can give a project.
const MAX_HITS: usize = 4096;
pub const MAX_TEMPLATES: usize = 128;

/// The template of `hits` (beats from the start of the template's first bar), repeating every
/// `length` beats. `None` when there is nothing to make one from.
pub fn from_hits(name: &str, hits: &[f64], length: f64) -> Option<GrooveDoc> {
    if !length.is_finite() || length <= 0.0 {
        return None;
    }
    let mut places: Vec<f64> = hits
        .iter()
        .filter(|h| h.is_finite())
        .map(|h| h.rem_euclid(length))
        .collect();
    places.sort_by(f64::total_cmp);
    places.dedup_by(|b, a| *b - *a < SAME_PLACE);
    places.truncate(MAX_HITS);
    if places.is_empty() {
        return None;
    }
    Some(GrooveDoc {
        name: name.to_string(),
        length: length as f32,
        hits: places.into_iter().map(|h| h as f32).collect(),
    })
}

/// A template from a file, if what it holds can be used: finite places inside its length, in
/// order, and not too many.
pub fn clean(doc: &GrooveDoc) -> Option<GrooveDoc> {
    let hits: Vec<f64> = doc.hits.iter().map(|h| f64::from(*h)).collect();
    let name: String = doc.name.chars().take(64).collect();
    from_hits(&name, &hits, f64::from(doc.length))
}

/// The templates a file holds.
pub fn from_docs(docs: &[GrooveDoc]) -> Vec<GrooveDoc> {
    docs.iter().filter_map(clean).take(MAX_TEMPLATES).collect()
}

/// The place of `groove` nearest to `beat`, which counts from the start of a bar.
pub fn nearest(groove: &GrooveDoc, beat: f32) -> f32 {
    let length = groove.length.max(1e-3);
    let cycle = (beat / length).floor();
    let mut best = beat;
    let mut best_distance = f32::INFINITY;
    // One round before and after, as the nearest place may be across the repeat.
    for round in [cycle - 1.0, cycle, cycle + 1.0] {
        for hit in &groove.hits {
            let place = round * length + hit;
            let distance = (place - beat).abs();
            if distance < best_distance {
                best_distance = distance;
                best = place;
            }
        }
    }
    best
}

/// Moves each of `targets` towards its nearest place in `groove`, its length staying. `strength`
/// is 0..=1, and `offset` is how far the region starts after the start of its bar.
pub fn quantize(
    notes: &mut [Note],
    targets: &[usize],
    groove: &GrooveDoc,
    offset: f32,
    strength: f32,
) {
    let strength = strength.clamp(0.0, 1.0);
    for &i in targets {
        if let Some(n) = notes.get_mut(i) {
            let target = nearest(groove, n.start + offset) - offset;
            n.start = (n.start + (target - n.start) * strength).max(0.0);
        }
    }
}

/// How far a region starting at `origin` (beats) is after the start of its bar.
pub fn bar_offset(p: &Project, origin: f64) -> f64 {
    origin - p.bar_to_beats(p.beats_to_bar(origin).floor())
}

/// How long a template of this region repeats: the whole bars it covers.
fn whole_bars(p: &Project, r: &Region) -> f64 {
    let (from, to) = (p.region_start_beats(r), p.region_end_beats(r));
    let first = p.beats_to_bar(from).floor();
    let last = p.beats_to_bar(to - 1e-6).floor() + 1.0;
    p.bar_to_beats(last) - p.bar_to_beats(first)
}

/// A name no template has, starting from `name`.
fn free_name(p: &Project, name: &str) -> String {
    let taken = |x: &str| p.grooves.iter().any(|g| g.name == x);
    if !taken(name) {
        return name.to_string();
    }
    (2..)
        .map(|n| format!("{name} {n}"))
        .find(|x| !taken(x))
        .unwrap_or_default()
}

/// Reading an audio region's hits to make a template of them.
#[derive(Default)]
pub struct MakeState {
    job: Option<Receiver<Result<GrooveDoc, String>>>,
}

fn add(p: &mut Project, mut groove: GrooveDoc) -> usize {
    groove.name = free_name(p, &groove.name);
    p.grooves.push(groove);
    p.grooves.len() - 1
}

/// Makes a template of a region: at once for MIDI, in the background for audio, when it arrives
/// from [`poll`]. Says what happened in the status line.
pub fn make(p: &mut Project, s: &mut UiState, t: usize, r: usize) {
    let Some(region) = p.tracks.get(t).and_then(|x| x.regions.get(r)) else {
        return;
    };
    if p.grooves.len() >= MAX_TEMPLATES {
        s.status = Some("This project has as many groove templates as it can keep".to_string());
        return;
    }
    let origin = p.region_start_beats(region);
    let offset = bar_offset(p, origin);
    let length = whole_bars(p, region);
    let name = region.name.clone();
    let Some(audio) = &region.audio else {
        let hits: Vec<f64> = region
            .notes
            .iter()
            .filter(|n| !n.muted)
            .map(|n| f64::from(n.start) + offset)
            .collect();
        s.status = Some(match from_hits(&name, &hits, length) {
            Some(groove) => {
                let count = groove.hits.len();
                let at = add(p, groove);
                format!("Groove template \"{}\": {count} places", p.grooves[at].name)
            }
            None => "That region has no notes to make a groove from".to_string(),
        });
        return;
    };
    if s.groove_make.job.is_some() {
        s.status = Some("Still reading the last region for its groove".to_string());
        return;
    }
    let file = audio
        .flex
        .as_ref()
        .map_or(&audio.file, |f| &f.original)
        .clone();
    let map = p.tempo_map();
    let start_seconds = map.seconds_at(origin);
    let end_beats = p.region_end_beats(region);
    let source_start = f64::from(region.params.source_start);
    let bar_start = origin - offset;
    let (tx, rx) = channel();
    let spawned = std::thread::Builder::new()
        .name("ovrtone-groove-template".into())
        .spawn(move || {
            let made = file.analyze().map_err(|e| e.to_string()).and_then(|found| {
                let hits: Vec<f64> = found
                    .onsets
                    .iter()
                    .filter(|o| **o >= source_start)
                    .map(|o| map.beat_at(start_seconds + (o - source_start)))
                    .filter(|b| *b < end_beats)
                    .map(|b| b - bar_start)
                    .collect();
                from_hits(&name, &hits, length)
                    .ok_or_else(|| "No hits were found in that region".to_string())
            });
            let _ = tx.send(made);
        });
    match spawned {
        Ok(_) => {
            s.groove_make.job = Some(rx);
            s.status = Some("Reading the region for its groove…".to_string());
        }
        Err(e) => s.status = Some(e.to_string()),
    }
}

/// Brings in a template read from audio. Call once per frame.
pub fn poll(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    let Some(rx) = &s.groove_make.job else {
        return;
    };
    let result = match rx.try_recv() {
        Err(TryRecvError::Empty) => {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        Err(TryRecvError::Disconnected) => Err("Reading the region stopped".to_string()),
        Ok(result) => result,
    };
    s.groove_make.job = None;
    s.status = Some(match result {
        Ok(groove) => {
            let count = groove.hits.len();
            let at = add(p, groove);
            format!("Groove template \"{}\": {count} places", p.grooves[at].name)
        }
        Err(e) => format!("Groove template: {e}"),
    });
}

/// Quantizes the notes of the selected regions to template `index` (the Tracks area's menu).
pub fn quantize_selected(p: &mut Project, s: &UiState, index: usize) -> usize {
    let Some(groove) = p.grooves.get(index).cloned() else {
        return 0;
    };
    let mut done = 0;
    for (t, r) in s.tracks.all_selected() {
        let Some(region) = p.tracks.get(t).and_then(|x| x.regions.get(r)) else {
            continue;
        };
        if region.audio.is_some() || region.generated.is_some() || region.folder.is_some() {
            continue;
        }
        let offset = bar_offset(p, p.region_start_beats(region)) as f32;
        let region = &mut p.tracks[t].regions[r];
        let all: Vec<usize> = (0..region.notes.len()).collect();
        quantize(&mut region.notes, &all, &groove, offset, 1.0);
        done += 1;
    }
    if done > 0 {
        p.song_rev += 1;
    }
    done
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn note(start: f32) -> Note {
        Note {
            start,
            len: 0.25,
            pitch: 60,
            vel: 100,
            muted: false,
            color: 0,
            slide: false,
        }
    }

    #[test]
    fn places_fold_over_the_length_and_repeat() {
        // A swung bar, played twice with a hair of difference.
        let hits = [0.0, 0.58, 1.0, 1.58, 4.0, 4.581, 5.0, 5.58];
        let g = from_hits("Swing", &hits, 4.0).unwrap();
        assert_eq!(g.hits.len(), 4, "{:?}", g.hits);
        assert!(from_hits("x", &[], 4.0).is_none());
        assert!(from_hits("x", &[1.0], 0.0).is_none());

        assert!((nearest(&g, 0.5) - 0.58).abs() < 1e-3);
        assert!((nearest(&g, 8.5) - 8.58).abs() < 1e-3, "in a later bar");
        // Near the end of the bar the nearest place is the next bar's first.
        assert!((nearest(&g, 3.9) - 4.0).abs() < 1e-3);
    }

    #[test]
    fn quantize_moves_notes_to_the_template_by_the_strength_asked() {
        let g = from_hits("Swing", &[0.0, 0.58, 1.0, 1.58], 2.0).unwrap();
        let mut notes = vec![note(0.5), note(1.5), note(4.5), note(0.5)];
        quantize(&mut notes, &[0, 1, 2], &g, 0.0, 1.0);
        assert!((notes[0].start - 0.58).abs() < 1e-3);
        assert!((notes[1].start - 1.58).abs() < 1e-3);
        assert!((notes[2].start - 4.58).abs() < 1e-3);
        assert_eq!(notes[3].start, 0.5, "only those asked for");
        assert_eq!(notes[0].len, 0.25, "lengths stay");

        let mut half = vec![note(0.5)];
        quantize(&mut half, &[0], &g, 0.0, 0.5);
        assert!((half[0].start - 0.54).abs() < 1e-3);

        // A region starting a beat into its bar lands on the same beats of the song.
        let mut late = vec![note(0.5)];
        quantize(&mut late, &[0], &g, 1.0, 1.0);
        assert!((late[0].start + 1.0 - 1.58).abs() < 1e-3);
    }

    #[test]
    fn a_midi_region_makes_one_and_it_is_saved_with_the_project() {
        let mut p = Project::empty();
        let mut s = UiState::new();
        p.tracks[0].regions.clear();
        let r = p.add_midi_region(0, 2.0).unwrap();
        p.tracks[0].regions[r].len_bars = 2.0;
        p.tracks[0].regions[r].name = "Beat".into();
        p.tracks[0].regions[r].notes = vec![note(0.1), note(1.1)];
        p.tracks[0].regions[r].notes[1].muted = false;
        make(&mut p, &mut s, 0, r);
        assert_eq!(p.grooves.len(), 1);
        assert_eq!(p.grooves[0].name, "Beat");
        assert_eq!(p.grooves[0].length, 8.0, "two bars of four");
        assert_eq!(p.grooves[0].hits.len(), 2);

        make(&mut p, &mut s, 0, r);
        assert_eq!(p.grooves[1].name, "Beat 2");

        let doc = crate::project_io::to_doc(&p).0;
        let (back, _) =
            crate::project_io::from_doc(&doc, std::path::Path::new("."), &egui::Context::default());
        assert_eq!(back.grooves, p.grooves);
    }

    #[test]
    fn a_region_with_no_notes_says_so() {
        let mut p = Project::empty();
        let mut s = UiState::new();
        p.tracks[0].regions.clear();
        let r = p.add_midi_region(0, 1.0).unwrap();
        make(&mut p, &mut s, 0, r);
        assert!(p.grooves.is_empty());
        assert!(s.status.as_deref().unwrap().contains("no notes"));
    }

    #[test]
    fn an_audio_region_gives_its_hits_in_the_background() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let wav = dir.path().join("hits.wav");
        // Clicks every half second at 120 BPM: every beat.
        let mut seed = 0x1234_5678u32;
        let mut noise = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
        };
        let mut stereo: Vec<f32> = (0..48_000 * 2 * 8).map(|_| noise() * 0.002).collect();
        for k in 0..16 {
            let at = k * 24_000 * 2;
            for i in 0..4800 {
                stereo[at + i] += noise() * 0.8 * (-(i as f32) / 800.0).exp();
            }
        }
        ovrtone_engine::write_wav(&wav, &stereo, 48_000).unwrap();
        let mut p = Project::empty();
        crate::import::import_audio_at(&mut p, &wav, &ctx, 1.0, None).unwrap();
        let t = p.tracks.len() - 1;
        let mut s = UiState::new();
        make(&mut p, &mut s, t, 0);
        let waited = std::time::Instant::now();
        while s.groove_make.job.is_some() {
            assert!(waited.elapsed().as_secs() < 30, "it finishes");
            poll(&ctx, &mut p, &mut s);
            std::thread::yield_now();
        }
        assert_eq!(p.grooves.len(), 1, "{:?}", s.status);
        assert!(p.grooves[0].hits.len() >= 2, "{:?}", p.grooves[0].hits);
        // Each hit is on a beat, near enough.
        for h in &p.grooves[0].hits {
            let off = (h - h.round()).abs();
            assert!(off < 0.08, "{:?}", p.grooves[0].hits);
        }
    }

    #[test]
    fn what_a_file_holds_is_made_safe() {
        let bad = GrooveDoc {
            name: "x".into(),
            length: f32::NAN,
            hits: vec![1.0],
        };
        let wild = GrooveDoc {
            name: "y".into(),
            length: 4.0,
            hits: vec![f32::INFINITY, 9.0, 1.0],
        };
        let got = from_docs(&[bad, wild]);
        assert_eq!(got.len(), 1);
        assert_eq!(
            got[0].hits,
            [1.0],
            "9 folds onto 1, and the two are one place"
        );
    }
}
