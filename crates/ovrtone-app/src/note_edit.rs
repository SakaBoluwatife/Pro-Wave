//! Pure note-editing operations for the Piano Roll (no UI): quantize, scale quantize,
//! marquee selection and hit testing. Kept separate so they can be unit tested.
use crate::model::{Control, Note};

/// Scale modes for Scale Quantize: name and semitone offsets from the root.
pub const MODES: [(&str, &[u8]); 7] = [
    ("Major", &[0, 2, 4, 5, 7, 9, 11]),
    ("Minor", &[0, 2, 3, 5, 7, 8, 10]),
    ("Dorian", &[0, 2, 3, 5, 7, 9, 10]),
    ("Mixolydian", &[0, 2, 4, 5, 7, 9, 10]),
    ("Major Pentatonic", &[0, 2, 4, 7, 9]),
    ("Minor Pentatonic", &[0, 3, 5, 7, 10]),
    ("Chromatic", &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]),
];

pub const ROOTS: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Time Quantize grid choices: label and length in beats.
pub const GRIDS: [(&str, f32); 8] = [
    ("1/4 Note", 1.0),
    ("1/8 Note", 0.5),
    ("1/8 Triplet", 1.0 / 3.0),
    ("1/16 Note", 0.25),
    ("1/16 Triplet", 1.0 / 6.0),
    ("1/32 Note", 0.125),
    ("1/32 Triplet", 1.0 / 12.0),
    ("1/64 Note", 0.0625),
];

/// Moves the start of every selected note towards the grid.
/// `strength` 0..=1 (1 = exactly on the grid). `swing` 0..=1 delays every second grid step by
/// up to a third of the step (Ovrtone's mapping of Logic's Swing percentage).
pub fn quantize(notes: &mut [Note], sel: &[usize], grid: f32, strength: f32, swing: f32) {
    if grid <= 0.0 {
        return;
    }
    let strength = strength.clamp(0.0, 1.0);
    let delay = swing.clamp(0.0, 1.0) * grid / 3.0;
    for &i in sel {
        let Some(n) = notes.get_mut(i) else { continue };
        let step = (n.start / grid).round();
        let mut target = step * grid;
        if (step as i64).rem_euclid(2) == 1 {
            target += delay;
        }
        n.start = (n.start + (target - n.start) * strength).max(0.0);
    }
}

/// Nearest pitch inside the scale (ties resolve downwards).
pub fn scale_snap(pitch: u8, root: u8, offsets: &[u8]) -> u8 {
    if offsets.is_empty() {
        return pitch;
    }
    let in_scale = |p: i32| {
        let pc = (p - root as i32).rem_euclid(12) as u8;
        offsets.contains(&pc)
    };
    for d in 0..12i32 {
        for p in [pitch as i32 - d, pitch as i32 + d] {
            if (0..=127).contains(&p) && in_scale(p) {
                return p as u8;
            }
        }
    }
    pitch
}

pub fn scale_quantize(notes: &mut [Note], sel: &[usize], root: u8, offsets: &[u8]) {
    for &i in sel {
        if let Some(n) = notes.get_mut(i) {
            n.pitch = scale_snap(n.pitch, root, offsets);
        }
    }
}

/// Indices of notes that intersect the beat/pitch rectangle.
pub fn marquee(notes: &[Note], beat0: f32, beat1: f32, pitch0: u8, pitch1: u8) -> Vec<usize> {
    let (b0, b1) = (beat0.min(beat1), beat0.max(beat1));
    let (p0, p1) = (pitch0.min(pitch1), pitch0.max(pitch1));
    notes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.pitch >= p0 && n.pitch <= p1 && n.start < b1 && n.start + n.len > b0)
        .map(|(i, _)| i)
        .collect()
}

/// Topmost note under (beat, pitch). The bool is true when the point is on the note's right
/// edge (within `edge` beats), which starts a resize instead of a move.
pub fn hit_note(notes: &[Note], beat: f32, pitch: u8, edge: f32) -> Option<(usize, bool)> {
    notes
        .iter()
        .enumerate()
        .rev()
        .find(|(_, n)| n.pitch == pitch && beat >= n.start && beat <= n.start + n.len)
        .map(|(i, n)| (i, beat >= n.start + n.len - edge.min(n.len * 0.5)))
}

/// Each note lasts until the next later start among the targets, the last ones until `end` (FL
/// Studio's Quick Legato). Algorithm from Lemon Studio (GPL-3.0-or-later).
pub fn legato(notes: &mut [Note], targets: &[usize], end: f32) {
    let mut starts: Vec<f32> = targets
        .iter()
        .filter_map(|&i| notes.get(i).map(|n| n.start))
        .collect();
    starts.sort_by(f32::total_cmp);
    starts.dedup();
    for &i in targets {
        if let Some(n) = notes.get_mut(i) {
            let next = starts.iter().copied().find(|&t| t > n.start).unwrap_or(end);
            if next > n.start {
                n.len = next - n.start;
            }
        }
    }
}

/// Removes the notes at `indices`, whatever their order.
pub fn remove_notes(notes: &mut Vec<Note>, indices: &[usize]) {
    let mut sorted = indices.to_vec();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    sorted.dedup();
    for i in sorted {
        if i < notes.len() {
            notes.remove(i);
        }
    }
}

/// Cuts each target into `size`-long pieces counted from its own start, the last piece keeping
/// what is left (Quick Chop). Returns where the pieces are. Algorithm from Lemon Studio.
pub fn chop(notes: &mut Vec<Note>, targets: &[usize], size: f32) -> Vec<usize> {
    let size = size.max(1.0 / 64.0);
    let mut pieces = Vec::new();
    for &i in targets {
        let Some(n) = notes.get(i).copied() else {
            continue;
        };
        let end = n.start + n.len;
        let mut at = n.start;
        while at + size < end - 1e-4 {
            pieces.push(Note {
                start: at,
                len: size,
                ..n
            });
            at += size;
        }
        pieces.push(Note {
            start: at,
            len: end - at,
            ..n
        });
    }
    remove_notes(notes, targets);
    let first = notes.len();
    notes.extend(pieces);
    (first..notes.len()).collect()
}

/// Joins targets of the same pitch that touch or overlap into one, with the first one's
/// velocity (Glue). Returns where the results are. Algorithm from Lemon Studio.
pub fn glue(notes: &mut Vec<Note>, targets: &[usize]) -> Vec<usize> {
    let mut chosen: Vec<Note> = targets
        .iter()
        .filter_map(|&i| notes.get(i).copied())
        .collect();
    chosen.sort_by(|a, b| {
        (a.pitch, a.start)
            .partial_cmp(&(b.pitch, b.start))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut joined: Vec<Note> = Vec::new();
    for n in chosen {
        match joined.last_mut() {
            Some(last) if last.pitch == n.pitch && n.start <= last.start + last.len + 1e-4 => {
                last.len = (last.start + last.len).max(n.start + n.len) - last.start;
            }
            _ => joined.push(n),
        }
    }
    remove_notes(notes, targets);
    let first = notes.len();
    notes.extend(joined);
    (first..notes.len()).collect()
}

/// Copies the targets right after their own end (Ctrl+B). Returns where the copies are.
pub fn duplicate(notes: &mut Vec<Note>, targets: &[usize]) -> Vec<usize> {
    let chosen: Vec<Note> = targets
        .iter()
        .filter_map(|&i| notes.get(i).copied())
        .collect();
    let start = chosen.iter().map(|n| n.start).fold(f32::INFINITY, f32::min);
    let end = chosen
        .iter()
        .map(|n| n.start + n.len)
        .fold(f32::NEG_INFINITY, f32::max);
    if chosen.is_empty() {
        return Vec::new();
    }
    paste(notes, &chosen, end.max(start))
}

/// Puts copies of `clip` in so that its earliest note starts at `at`. Returns where they are.
pub fn paste(notes: &mut Vec<Note>, clip: &[Note], at: f32) -> Vec<usize> {
    let start = clip.iter().map(|n| n.start).fold(f32::INFINITY, f32::min);
    let first = notes.len();
    for n in clip {
        notes.push(Note {
            start: (n.start - start + at).max(0.0),
            ..*n
        });
    }
    (first..notes.len()).collect()
}

/// Cuts every note a line at `beat` crosses in two (the Slice tool). Returns how many it cut.
/// Algorithm from Lemon Studio.
pub fn slice_at(notes: &mut Vec<Note>, beat: f32) -> usize {
    let mut cut = 0;
    for i in 0..notes.len() {
        let n = notes[i];
        if n.start < beat - 1e-4 && beat + 1e-4 < n.start + n.len {
            notes[i].len = beat - n.start;
            notes.push(Note {
                start: beat,
                len: n.start + n.len - beat,
                ..n
            });
            cut += 1;
        }
    }
    cut
}

/// The Paint tool: notes of `template`'s length and velocity at `pitch`, from `start` to wherever
/// `pointer` is, one after the other, leaving out the places a note already starts. Returns
/// how many it added. Algorithm from Lemon Studio.
pub fn paint(notes: &mut Vec<Note>, template: Note, pitch: u8, start: f32, pointer: f32) -> usize {
    let len = template.len.max(1.0 / 64.0);
    let k = ((pointer - start) / len).floor() as i32;
    let mut added = 0;
    for j in k.min(0)..=k.max(0) {
        let at = start + j as f32 * len;
        if at < 0.0 {
            continue;
        }
        let taken = notes
            .iter()
            .any(|n| n.pitch == pitch && (n.start - at).abs() < 1e-3);
        if !taken {
            notes.push(Note {
                start: at,
                pitch,
                muted: false,
                ..template
            });
            added += 1;
        }
    }
    added
}

/// Moves the start of each target to the nearest step, its length staying (Shift+Q).
pub fn quantize_starts(notes: &mut [Note], targets: &[usize], step: f32) {
    let step = step.max(1.0 / 64.0);
    for &i in targets {
        if let Some(n) = notes.get_mut(i) {
            n.start = ((n.start / step).round() * step).max(0.0);
        }
    }
}

/// The notes not in `selection` (Shift+I).
pub fn inverted(count: usize, selection: &[usize]) -> Vec<usize> {
    (0..count).filter(|i| !selection.contains(i)).collect()
}

/// The notes that start inside a time selection (FL Studio's time selection takes these).
pub fn in_time(notes: &[Note], from: f32, to: f32) -> Vec<usize> {
    notes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.start >= from - 1e-4 && n.start < to - 1e-4)
        .map(|(i, _)| i)
        .collect()
}

/// Takes the time from `from` to `to` out of a region (Ctrl+Delete): what is inside goes, a note
/// across an edge keeps the part outside, and everything later moves up.
pub fn delete_time(notes: &mut Vec<Note>, controls: &mut Vec<Control>, from: f32, to: f32) {
    let span = to - from;
    if span <= 0.0 {
        return;
    }
    notes.retain_mut(|n| {
        let end = n.start + n.len;
        if n.start >= to {
            n.start -= span;
        } else if n.start >= from {
            // It starts inside: only what sticks out after the selection is left.
            if end <= to + 1e-4 {
                return false;
            }
            n.start = from;
            n.len = end - to;
        } else if end > to {
            n.len -= span;
        } else if end > from {
            n.len = from - n.start;
        }
        true
    });
    controls.retain_mut(|c| {
        if c.start >= to {
            c.start -= span;
            true
        } else {
            c.start < from
        }
    });
}

/// Opens up empty time from `from` to `to` (Ctrl+Insert): what starts there or later moves back.
pub fn insert_time(notes: &mut [Note], controls: &mut [Control], from: f32, to: f32) {
    let span = to - from;
    if span <= 0.0 {
        return;
    }
    for n in notes.iter_mut().filter(|n| n.start >= from - 1e-4) {
        n.start += span;
    }
    for c in controls.iter_mut().filter(|c| c.start >= from - 1e-4) {
        c.start += span;
    }
}

/// A stroke in a controller lane from `a` to `b` (beat, value): one event every `step`, in a
/// straight line, in place of what that controller had under the stroke. Kept sorted by time.
pub fn draw_controls(controls: &mut Vec<Control>, num: u8, a: (f32, u8), b: (f32, u8), step: f32) {
    let step = step.max(1.0 / 64.0);
    let ((x0, v0), (x1, v1)) = if a.0 <= b.0 { (a, b) } else { (b, a) };
    let first = (x0.max(0.0) / step).floor() as i64;
    let last = (x1.max(0.0) / step).floor() as i64;
    let (lo, hi) = (first as f32 * step, last as f32 * step);
    controls.retain(|c| c.num != num || c.start < lo - 1e-4 || c.start > hi + 1e-4);
    for k in first..=last {
        let at = k as f32 * step;
        let t = if x1 > x0 {
            ((at - x0) / (x1 - x0)).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let val = f32::from(v0) + (f32::from(v1) - f32::from(v0)) * t;
        controls.push(Control {
            start: at,
            num,
            val: val.round().clamp(0.0, 127.0) as u8,
        });
    }
    controls.sort_by(|p, q| p.start.total_cmp(&q.start));
}

/// Removes what a controller has between two beats, whichever way the stroke went.
pub fn erase_controls(controls: &mut Vec<Control>, num: u8, a: f32, b: f32) {
    let (lo, hi) = (a.min(b), a.max(b));
    controls.retain(|c| c.num != num || c.start < lo || c.start > hi);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(start: f32, pitch: u8) -> Note {
        Note {
            start,
            len: 0.5,
            pitch,
            vel: 90,
            muted: false,
            color: 0,
            slide: false,
        }
    }

    #[test]
    fn quantize_full_strength_snaps_to_grid() {
        let mut notes = vec![note(0.9, 60), note(1.3, 62)];
        quantize(&mut notes, &[0, 1], 0.5, 1.0, 0.0);
        assert!((notes[0].start - 1.0).abs() < 1e-6);
        assert!((notes[1].start - 1.5).abs() < 1e-6);
    }

    #[test]
    fn quantize_half_strength_moves_halfway_and_only_selected() {
        let mut notes = vec![note(0.9, 60), note(0.9, 62)];
        quantize(&mut notes, &[0], 1.0, 0.5, 0.0);
        assert!((notes[0].start - 0.95).abs() < 1e-6);
        assert!((notes[1].start - 0.9).abs() < 1e-6);
    }

    #[test]
    fn swing_delays_odd_steps_only() {
        let mut notes = vec![note(0.0, 60), note(0.5, 60), note(1.0, 60), note(1.5, 60)];
        quantize(&mut notes, &[0, 1, 2, 3], 0.5, 1.0, 1.0);
        let d = 0.5 / 3.0;
        let want = [0.0, 0.5 + d, 1.0, 1.5 + d];
        for (n, w) in notes.iter().zip(want) {
            assert!((n.start - w).abs() < 1e-5, "{} vs {w}", n.start);
        }
    }

    #[test]
    fn scale_snap_c_major() {
        let major = MODES[0].1;
        assert_eq!(scale_snap(61, 0, major), 60); // C# -> C (tie goes down)
        assert_eq!(scale_snap(63, 0, major), 62); // D# -> D
        assert_eq!(scale_snap(64, 0, major), 64); // E stays
        assert_eq!(scale_snap(70, 0, major), 69); // A# -> A
                                                  // Other roots: D major contains F# but not F.
        assert_eq!(scale_snap(65, 2, major), 64); // F -> E
        assert_eq!(scale_snap(66, 2, major), 66);
    }

    #[test]
    fn scale_quantize_changes_only_selected() {
        let mut notes = vec![note(0.0, 61), note(1.0, 61)];
        scale_quantize(&mut notes, &[1], 0, MODES[0].1);
        assert_eq!((notes[0].pitch, notes[1].pitch), (61, 60));
    }

    #[test]
    fn marquee_picks_intersecting_notes() {
        let notes = vec![note(0.0, 60), note(2.0, 64), note(2.0, 72)];
        assert_eq!(marquee(&notes, 1.9, 3.0, 62, 66), vec![1]);
        assert_eq!(marquee(&notes, 3.0, 1.9, 66, 62), vec![1]);
        assert!(marquee(&notes, 5.0, 6.0, 0, 127).is_empty());
    }

    #[test]
    fn hit_note_detects_body_and_right_edge() {
        let notes = vec![Note {
            start: 1.0,
            len: 2.0,
            pitch: 60,
            vel: 100,
            muted: false,
            color: 0,
            slide: false,
        }];
        assert_eq!(hit_note(&notes, 1.5, 60, 0.2), Some((0, false)));
        assert_eq!(hit_note(&notes, 2.9, 60, 0.2), Some((0, true)));
        assert_eq!(hit_note(&notes, 1.5, 61, 0.2), None);
        assert_eq!(hit_note(&notes, 3.5, 60, 0.2), None);
    }

    fn long(start: f32, len: f32, pitch: u8) -> Note {
        Note {
            start,
            len,
            pitch,
            vel: 100,
            muted: false,
            color: 0,
            slide: false,
        }
    }

    #[test]
    fn legato_runs_each_note_up_to_the_next_start() {
        let mut notes = vec![
            long(0.0, 0.25, 60),
            long(1.0, 0.25, 62),
            long(1.0, 0.25, 64),
            long(3.0, 0.25, 65),
        ];
        legato(&mut notes, &[0, 1, 2, 3], 8.0);
        assert_eq!(notes[0].len, 1.0);
        assert_eq!(
            (notes[1].len, notes[2].len),
            (2.0, 2.0),
            "a chord stays one"
        );
        assert_eq!(notes[3].len, 5.0, "the last one runs to the end");
    }

    #[test]
    fn chop_cuts_into_pieces_and_keeps_what_is_left() {
        let mut notes = vec![long(1.0, 1.25, 60), long(9.0, 1.0, 72)];
        let pieces = chop(&mut notes, &[0], 0.5);
        assert_eq!(notes.len(), 4);
        let got: Vec<(f32, f32)> = pieces
            .iter()
            .map(|&i| (notes[i].start, notes[i].len))
            .collect();
        assert_eq!(got, [(1.0, 0.5), (1.5, 0.5), (2.0, 0.25)]);
        assert!(notes.iter().any(|n| n.pitch == 72), "others are left alone");
    }

    #[test]
    fn glue_joins_touching_notes_of_one_pitch_only() {
        let mut notes = vec![
            long(0.0, 1.0, 60),
            long(1.0, 1.0, 60),
            long(3.0, 1.0, 60),
            long(0.0, 2.0, 64),
        ];
        let joined = glue(&mut notes, &[0, 1, 2, 3]);
        assert_eq!(joined.len(), 3);
        assert!(notes
            .iter()
            .any(|n| n.pitch == 60 && n.start == 0.0 && n.len == 2.0));
        assert!(notes.iter().any(|n| n.pitch == 60 && n.start == 3.0));
    }

    #[test]
    fn duplicate_goes_right_after_the_selection_and_keeps_its_shape() {
        let mut notes = vec![long(1.0, 1.0, 60), long(1.5, 1.5, 64)];
        let copies = duplicate(&mut notes, &[0, 1]);
        assert_eq!(copies, [2, 3]);
        assert_eq!((notes[2].start, notes[3].start), (3.0, 3.5));
        assert!(duplicate(&mut notes, &[]).is_empty());
    }

    #[test]
    fn slice_cuts_what_the_line_crosses_and_nothing_else() {
        let mut notes = vec![long(0.0, 2.0, 60), long(1.0, 1.0, 62), long(0.0, 1.0, 64)];
        assert_eq!(
            slice_at(&mut notes, 1.0),
            1,
            "a note that starts or ends there is not cut"
        );
        assert_eq!(notes.len(), 4);
        assert_eq!(notes[0].len, 1.0);
        assert_eq!(
            (notes[3].start, notes[3].len, notes[3].pitch),
            (1.0, 1.0, 60)
        );
    }

    #[test]
    fn paint_lays_notes_both_ways_and_not_twice() {
        let template = long(0.0, 0.5, 0);
        let mut notes = vec![long(2.5, 0.5, 60)];
        assert_eq!(
            paint(&mut notes, template, 60, 2.0, 3.2),
            2,
            "2.0 and 3.0; 2.5 is taken"
        );
        assert_eq!(
            paint(&mut notes, template, 60, 2.0, 3.2),
            0,
            "already there"
        );
        assert_eq!(
            paint(&mut notes, template, 60, 2.0, 1.2),
            2,
            "backwards: 1.0 and 1.5"
        );
        assert_eq!(
            paint(&mut notes, template, 60, 0.0, -5.0),
            1,
            "only the one at 0"
        );
    }

    #[test]
    fn quantizing_starts_leaves_lengths_and_inverting_takes_the_rest() {
        let mut notes = vec![long(1.1, 0.33, 60)];
        quantize_starts(&mut notes, &[0], 0.25);
        assert_eq!((notes[0].start, notes[0].len), (1.0, 0.33));
        assert_eq!(inverted(4, &[1, 2]), [0, 3]);
    }

    fn cc(start: f32, val: u8) -> Control {
        Control { start, num: 1, val }
    }

    #[test]
    fn a_time_selection_takes_the_notes_that_start_in_it() {
        let notes = vec![long(0.0, 4.0, 60), long(1.0, 1.0, 62), long(2.0, 1.0, 64)];
        assert_eq!(in_time(&notes, 1.0, 2.0), [1], "the end is not in it");
    }

    #[test]
    fn deleting_time_closes_the_gap_and_keeps_what_sticks_out() {
        let mut notes = vec![
            long(0.0, 1.0, 60), // before: stays
            long(1.0, 2.0, 62), // crosses the start: cut at it
            long(2.5, 0.5, 64), // inside: goes
            long(3.5, 2.0, 65), // crosses the end: the part after it stays
            long(6.0, 1.0, 67), // after: moves up
            long(0.5, 6.0, 69), // over the whole of it: shorter
        ];
        let mut controls = vec![cc(1.0, 1), cc(2.5, 2), cc(4.0, 3)];
        delete_time(&mut notes, &mut controls, 2.0, 4.0);
        let got: Vec<(u8, f32, f32)> = notes.iter().map(|n| (n.pitch, n.start, n.len)).collect();
        assert_eq!(
            got,
            [
                (60, 0.0, 1.0),
                (62, 1.0, 1.0),
                (65, 2.0, 1.5),
                (67, 4.0, 1.0),
                (69, 0.5, 4.0),
            ]
        );
        let times: Vec<f32> = controls.iter().map(|c| c.start).collect();
        assert_eq!(times, [1.0, 2.0]);
    }

    #[test]
    fn inserting_time_moves_what_comes_later_only() {
        let mut notes = vec![long(0.0, 4.0, 60), long(2.0, 1.0, 62)];
        let mut controls = vec![cc(1.0, 1), cc(2.0, 2)];
        insert_time(&mut notes, &mut controls, 2.0, 3.0);
        assert_eq!((notes[0].start, notes[1].start), (0.0, 3.0));
        assert_eq!((controls[0].start, controls[1].start), (1.0, 3.0));
    }

    #[test]
    fn a_controller_stroke_replaces_what_it_crosses_and_stays_in_order() {
        let mut controls = vec![
            cc(0.0, 10),
            cc(0.3, 11),
            cc(3.0, 12),
            Control {
                start: 0.5,
                num: 7,
                val: 99,
            },
        ];
        draw_controls(&mut controls, 1, (0.0, 0), (1.0, 100), 0.25);
        let one: Vec<(f32, u8)> = controls
            .iter()
            .filter(|c| c.num == 1)
            .map(|c| (c.start, c.val))
            .collect();
        assert_eq!(
            one,
            [
                (0.0, 0),
                (0.25, 25),
                (0.5, 50),
                (0.75, 75),
                (1.0, 100),
                (3.0, 12)
            ]
        );
        assert!(controls.windows(2).all(|w| w[0].start <= w[1].start));
        assert!(
            controls.iter().any(|c| c.num == 7),
            "another controller stays"
        );
        // Backwards is the same stroke, and it never goes before the region.
        let mut back = Vec::new();
        draw_controls(&mut back, 1, (0.5, 127), (-3.0, 127), 0.25);
        assert_eq!(back.len(), 3);
        assert_eq!(back[0].start, 0.0);

        erase_controls(&mut controls, 1, 1.0, 0.25);
        let left: Vec<f32> = controls
            .iter()
            .filter(|c| c.num == 1)
            .map(|c| c.start)
            .collect();
        assert_eq!(left, [0.0, 3.0]);
    }
}
