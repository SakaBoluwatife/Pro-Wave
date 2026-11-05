//! Take folders, as in Logic: recording with the Cycle on makes a new take on every pass, and
//! the passes are kept together in one region. A comp says which take plays in each stretch
//! (the last take everywhere to begin with); the notes of the comp are copied into the
//! region's own `notes`, so the engine, the Piano Roll and the Event list need no special case.

use crate::model::{CompSegment, Control, Note, Take, TakeFolder};

const EPS: f32 = 1e-4;

/// A pass shorter than this at the end of a recording is the moment Stop was pressed.
const SHORTEST_PASS: f64 = 0.25;
/// The joins of a comp are smoothed over this many beats, to not click.
const JOIN_FADE: f64 = 0.01;

/// Where each pass of an audio recording lies in its file, as [`Take::audio_offset`] says. It began
/// `lead` seconds into a cycle that lasts `pass` seconds, and the file is `file` seconds long.
pub fn audio_take_offsets(file: f64, lead: f64, pass: f64) -> Vec<f32> {
    let mut out = vec![-lead as f32];
    if pass <= 0.0 {
        return out;
    }
    let mut at = pass - lead;
    while file - at >= SHORTEST_PASS {
        out.push(at as f32);
        at += pass;
    }
    out
}

/// What the engine streams for a comp of audio takes: where each stretch starts, in beats, and how
/// it reads the file. `base` holds the region's own gain.
pub fn audio_comp_clips(
    folder: &TakeFolder,
    map: &prowave_engine::TempoMap,
    origin: f64,
    base: prowave_engine::ClipEdit,
    file_seconds: f64,
) -> Vec<(f64, prowave_engine::ClipEdit)> {
    let start_secs = map.seconds_at(origin);
    let last = folder.comp.len().saturating_sub(1);
    folder
        .comp
        .iter()
        .enumerate()
        .filter_map(|(i, seg)| {
            let offset = f64::from(folder.takes.get(seg.take)?.audio_offset?);
            let from = map.seconds_at(origin + f64::from(seg.start)) - start_secs;
            let to = map.seconds_at(origin + f64::from(seg.end)) - start_secs;
            // The recording may start after this stretch does, and end before it does.
            let source = (offset + from).max(0.0);
            let from = source - offset;
            let end = (offset + to).min(file_seconds);
            if end - source <= 0.0 {
                return None;
            }
            let mut edit = base;
            edit.source_start_secs = source;
            edit.length_secs = end - source;
            edit.fade_in_beats = if i == 0 {
                base.fade_in_beats
            } else {
                JOIN_FADE
            };
            edit.fade_out_beats = if i == last {
                base.fade_out_beats
            } else {
                JOIN_FADE
            };
            Some((map.beat_at(start_secs + from), edit))
        })
        .collect()
}

impl TakeFolder {
    /// Whether its takes are passes of an audio recording.
    pub fn is_audio(&self) -> bool {
        self.takes.iter().any(|t| t.audio_offset.is_some())
    }

    /// A folder of `takes` over `len` beats whose comp plays the last take throughout.
    pub fn new(takes: Vec<Take>, len: f32) -> Self {
        let last = takes.len().saturating_sub(1);
        Self {
            comp: vec![CompSegment {
                start: 0.0,
                end: len,
                take: last,
            }],
            takes,
            len,
            open: false,
        }
    }

    /// The notes and controllers the comp plays: each take contributes what starts inside
    /// the stretches that use it.
    pub fn flatten(&self) -> (Vec<Note>, Vec<Control>) {
        let mut notes = Vec::new();
        let mut controls = Vec::new();
        for seg in &self.comp {
            let Some(take) = self.takes.get(seg.take) else {
                continue;
            };
            let inside = |start: f32| start >= seg.start - EPS && start < seg.end - EPS;
            notes.extend(take.notes.iter().filter(|n| inside(n.start)).copied());
            controls.extend(take.controls.iter().filter(|c| inside(c.start)).copied());
        }
        notes.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
        controls.sort_by(|a, b| a.start.total_cmp(&b.start));
        (notes, controls)
    }

    /// Copies the comp into a region's own notes and controllers.
    pub fn apply(&self, notes: &mut Vec<Note>, controls: &mut Vec<Control>) {
        (*notes, *controls) = self.flatten();
    }

    /// Logic's swipe on a take: the stretch `a..b` (beats) now plays `take`. The rest of the
    /// comp keeps what it had, and neighbouring stretches of the same take join up.
    pub fn swipe(&mut self, take: usize, a: f32, b: f32) {
        if take >= self.takes.len() {
            return;
        }
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let (a, b) = (a.clamp(0.0, self.len), b.clamp(0.0, self.len));
        if b - a < EPS {
            return;
        }
        let mut out: Vec<CompSegment> = Vec::new();
        for seg in &self.comp {
            if seg.start < a - EPS {
                out.push(CompSegment {
                    end: seg.end.min(a),
                    ..*seg
                });
            }
            if seg.end > b + EPS {
                out.push(CompSegment {
                    start: seg.start.max(b),
                    ..*seg
                });
            }
        }
        out.push(CompSegment {
            start: a,
            end: b,
            take,
        });
        out.sort_by(|x, y| x.start.total_cmp(&y.start));
        let mut merged: Vec<CompSegment> = Vec::new();
        for seg in out {
            match merged.last_mut() {
                Some(prev) if prev.take == seg.take && (seg.start - prev.end).abs() < EPS => {
                    prev.end = seg.end;
                }
                _ => merged.push(seg),
            }
        }
        self.comp = merged;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_passes_of_a_recording_are_found_in_its_file() {
        // Started half a second into a 2 s cycle, and went round twice more and a little.
        assert_eq!(audio_take_offsets(6.0, 0.5, 2.0), [-0.5, 1.5, 3.5, 5.5]);
        // Stopped just after coming round: that is no pass.
        assert_eq!(
            audio_take_offsets(3.6, 0.5, 2.0),
            [-0.5, 1.5, 3.5][..2].to_vec()
        );
        // Stopped during the first.
        assert_eq!(audio_take_offsets(1.0, 0.0, 2.0), [0.0]);
    }

    #[test]
    fn a_comp_reads_each_stretch_from_its_pass_and_stays_inside_the_file() {
        let map = {
            let mut p = crate::model::Project::empty();
            p.tempo = 120.0;
            p.tempo_map()
        };
        let take = |offset: f32| Take {
            audio_offset: Some(offset),
            ..Take::default()
        };
        // Two beats are a second. The first pass began half a second late.
        let mut folder = TakeFolder::new(vec![take(-0.5), take(1.5), take(3.5)], 4.0);
        folder.swipe(0, 0.0, 2.0);
        folder.swipe(1, 2.0, 3.0);
        assert_eq!(folder.comp.len(), 3);
        let base = prowave_engine::ClipEdit::default();
        let clips = audio_comp_clips(&folder, &map, 8.0, base, 5.25);
        // Take 1's first half: nothing was recorded for the first half second of it.
        assert_eq!(clips[0].0, 9.0);
        assert_eq!(clips[0].1.source_start_secs, 0.0);
        assert!((clips[0].1.length_secs - 0.5).abs() < 1e-9);
        // Take 2 from beat 2 of the region, one second in.
        assert_eq!(clips[1].0, 10.0);
        assert!((clips[1].1.source_start_secs - 2.5).abs() < 1e-6);
        assert!((clips[1].1.length_secs - 0.5).abs() < 1e-9);
        // The last take from beat 3, cut where the file ends.
        assert_eq!(clips[2].0, 11.0);
        assert!((clips[2].1.source_start_secs - 5.0).abs() < 1e-6);
        assert!((clips[2].1.length_secs - 0.25).abs() < 1e-6);
    }

    fn note(start: f32) -> Note {
        Note {
            start,
            len: 0.5,
            pitch: 60,
            vel: 100,
            muted: false,
        }
    }

    fn folder() -> TakeFolder {
        // Take 0 plays at 0, 2, 5; take 1 at 1, 3, 6. Eight beats long.
        let take = |starts: &[f32]| Take {
            audio_offset: None,
            notes: starts.iter().map(|&s| note(s)).collect(),
            controls: Vec::new(),
        };
        TakeFolder::new(vec![take(&[0.0, 2.0, 5.0]), take(&[1.0, 3.0, 6.0])], 8.0)
    }

    fn starts(f: &TakeFolder) -> Vec<f32> {
        f.flatten().0.iter().map(|n| n.start).collect()
    }

    #[test]
    fn the_last_take_plays_everywhere_to_begin_with() {
        let f = folder();
        assert_eq!(f.comp.len(), 1);
        assert_eq!(starts(&f), vec![1.0, 3.0, 6.0]);
    }

    #[test]
    fn a_swipe_takes_over_only_its_stretch() {
        let mut f = folder();
        f.swipe(0, 1.5, 5.5);
        assert_eq!(
            f.comp
                .iter()
                .map(|s| (s.start, s.end, s.take))
                .collect::<Vec<_>>(),
            vec![(0.0, 1.5, 1), (1.5, 5.5, 0), (5.5, 8.0, 1)]
        );
        assert_eq!(starts(&f), vec![1.0, 2.0, 5.0, 6.0]);
    }

    #[test]
    fn swiping_backwards_and_past_the_ends_is_fine() {
        let mut f = folder();
        f.swipe(0, 20.0, -3.0);
        assert_eq!(f.comp.len(), 1);
        assert_eq!(f.comp[0].take, 0);
        assert_eq!(starts(&f), vec![0.0, 2.0, 5.0]);
    }

    #[test]
    fn neighbouring_stretches_of_one_take_join_up() {
        let mut f = folder();
        f.swipe(0, 2.0, 4.0);
        f.swipe(0, 4.0, 6.0);
        assert_eq!(f.comp.len(), 3);
        assert_eq!((f.comp[1].start, f.comp[1].end), (2.0, 6.0));
        // Swiping the same take back over everything leaves one stretch.
        f.swipe(1, 0.0, 8.0);
        assert_eq!(f.comp.len(), 1);
    }

    #[test]
    fn a_tiny_or_unknown_swipe_changes_nothing() {
        let mut f = folder();
        let before = f.clone();
        f.swipe(0, 3.0, 3.0);
        f.swipe(9, 0.0, 4.0);
        assert_eq!(f, before);
    }

    #[test]
    fn controllers_follow_the_comp_too() {
        let mut f = folder();
        f.takes[0].controls.push(Control {
            start: 0.0,
            num: 64,
            val: 127,
        });
        f.takes[1].controls.push(Control {
            start: 0.0,
            num: 64,
            val: 0,
        });
        assert_eq!(f.flatten().1[0].val, 0);
        f.swipe(0, 0.0, 4.0);
        assert_eq!(f.flatten().1[0].val, 127);
    }
}
