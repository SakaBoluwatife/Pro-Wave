//! Patterns of the Channel Rack (FL Studio's patterns), as opposed to the step regions in
//! `pattern.rs`. A pattern is a set of note lanes, one per channel; a channel is an instrument
//! track. Placing a pattern on the Tracks area makes a region on each track it uses, linked to it.
//!
//! The lanes are the source of truth. The rack changes a lane and calls [`push`], which rewrites
//! the placed regions. An edit made in the Piano Roll changes a region, and [`sync`] notices and
//! takes it back into the lane, so every copy of the clip follows. A region longer than its
//! pattern loops it; only its first pass can be edited this way. Notes past the end of a pattern
//! stay in the lane, unheard, so making a pattern shorter and longer again loses nothing.
use crate::model::{Note, Project, TrackKind};
use egui::Color32;

/// The Channel Rack's step: a sixteenth note, in beats.
pub const STEP_BEATS: f32 = 0.25;
/// How close a note must start to a step to be that step.
const ON_STEP: f32 = 0.002;
const DEFAULT_STEPS: usize = 16;
const DEFAULT_STEP_PITCH: u8 = 60;

#[derive(Clone, Debug, PartialEq)]
pub struct Lane {
    pub channel: u64,
    /// The pitch the step buttons play.
    pub step_pitch: u8,
    pub notes: Vec<Note>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SongPattern {
    pub id: u64,
    pub name: String,
    pub color: Color32,
    pub steps: usize,
    pub lanes: Vec<Lane>,
}

impl SongPattern {
    pub fn len_beats(&self) -> f32 {
        self.steps as f32 * STEP_BEATS
    }

    pub fn lane(&self, channel: u64) -> Option<&Lane> {
        self.lanes.iter().find(|l| l.channel == channel)
    }

    pub fn lane_mut(&mut self, channel: u64) -> &mut Lane {
        if let Some(i) = self.lanes.iter().position(|l| l.channel == channel) {
            return &mut self.lanes[i];
        }
        self.lanes.push(Lane {
            channel,
            step_pitch: DEFAULT_STEP_PITCH,
            notes: Vec::new(),
        });
        let last = self.lanes.len() - 1;
        &mut self.lanes[last]
    }
}

impl Lane {
    pub fn step_is_on(&self, step: usize) -> bool {
        let at = step as f32 * STEP_BEATS;
        self.notes
            .iter()
            .any(|n| n.pitch == self.step_pitch && (n.start - at).abs() <= ON_STEP)
    }

    /// Switches a step on or off. Only notes on that step at the step pitch are touched: anything
    /// else in the lane was placed some other way and stays.
    pub fn toggle_step(&mut self, step: usize) {
        let at = step as f32 * STEP_BEATS;
        if self.step_is_on(step) {
            let pitch = self.step_pitch;
            self.notes
                .retain(|n| n.pitch != pitch || (n.start - at).abs() > ON_STEP);
        } else {
            self.notes.push(Note {
                start: at,
                len: STEP_BEATS,
                pitch: self.step_pitch,
                vel: crate::pattern::DEFAULT_VELOCITY,
                muted: false,
                color: 0,
                slide: false,
            });
        }
    }

    /// The notes the step buttons cannot show: another pitch, or between steps.
    pub fn off_grid(&self) -> impl Iterator<Item = &Note> {
        self.notes.iter().filter(|n| {
            let nearest = (n.start / STEP_BEATS).round() * STEP_BEATS;
            n.pitch != self.step_pitch || (n.start - nearest).abs() > ON_STEP
        })
    }
}

/// An id not used by a pattern or channel yet.
fn fresh_id(p: &Project) -> u64 {
    let patterns = p.patterns.iter().map(|x| x.id);
    let channels = p.tracks.iter().map(|t| t.params.channel);
    patterns.chain(channels).max().unwrap_or(0) + 1
}

/// The channel number of track `t`, given one the first time it is asked for. Tracks that never
/// join a pattern have none, so a project is not changed by being opened.
pub fn channel_of(p: &mut Project, t: usize) -> u64 {
    let id = fresh_id(p);
    let Some(track) = p.tracks.get_mut(t) else {
        return 0;
    };
    if track.params.channel == 0 {
        track.params.channel = id;
    }
    track.params.channel
}

/// The tracks that are channels.
pub fn channels(p: &Project) -> Vec<usize> {
    (0..p.tracks.len())
        .filter(|&t| p.tracks[t].kind == TrackKind::Instrument)
        .collect()
}

const COLORS: [Color32; 6] = [
    Color32::from_rgb(0x4f, 0x9d, 0x69),
    Color32::from_rgb(0x4f, 0x7d, 0xc9),
    Color32::from_rgb(0xc9, 0x7a, 0x3f),
    Color32::from_rgb(0xa8, 0x5c, 0xc4),
    Color32::from_rgb(0xc2, 0x4f, 0x5f),
    Color32::from_rgb(0x3f, 0xa8, 0xa8),
];

/// A new empty pattern. Returns its id.
pub fn new_pattern(p: &mut Project) -> u64 {
    let id = fresh_id(p);
    let n = p.patterns.len();
    p.patterns.push(SongPattern {
        id,
        name: format!("Pattern {}", n + 1),
        color: COLORS[n % COLORS.len()],
        steps: DEFAULT_STEPS,
        lanes: Vec::new(),
    });
    id
}

/// A copy of pattern `id`, on its own from now on. Returns its id.
pub fn clone_pattern(p: &mut Project, id: u64) -> Option<u64> {
    let mut copy = p.patterns.iter().find(|x| x.id == id)?.clone();
    copy.id = fresh_id(p);
    copy.name = format!("{} copy", copy.name);
    let new = copy.id;
    p.patterns.push(copy);
    Some(new)
}

/// Removes a pattern. Its placed regions keep their notes as ordinary regions.
pub fn delete_pattern(p: &mut Project, id: u64) {
    p.patterns.retain(|x| x.id != id);
    for region in p.tracks.iter_mut().flat_map(|t| &mut t.regions) {
        if region.params.pattern == id {
            region.params.pattern = 0;
            region.params.pattern_color = None;
        }
    }
}

/// A region goes its own way, keeping its notes.
pub fn make_unique(p: &mut Project, t: usize, r: usize) {
    if let Some(region) = p.tracks.get_mut(t).and_then(|x| x.regions.get_mut(r)) {
        region.params.pattern = 0;
        region.params.pattern_color = None;
    }
}

/// The region gets a copy of its pattern for itself, which can then be placed again.
pub fn clone_for_region(p: &mut Project, t: usize, r: usize) -> Option<u64> {
    let id = p.tracks.get(t)?.regions.get(r)?.params.pattern;
    let new = clone_pattern(p, id)?;
    p.tracks[t].regions[r].params.pattern = new;
    push(p, new);
    Some(new)
}

/// The notes a region of `region_len` beats plays from a lane: the pattern, looped and cut off.
fn looped(notes: &[Note], pattern_len: f32, region_len: f32) -> Vec<Note> {
    let mut out = Vec::new();
    if pattern_len <= 0.0 {
        return out;
    }
    let mut offset = 0.0;
    while offset < region_len {
        for n in notes.iter().filter(|n| n.start < pattern_len) {
            let start = n.start + offset;
            if start < region_len {
                out.push(Note {
                    start,
                    len: n.len.min(region_len - start),
                    ..*n
                });
            }
        }
        offset += pattern_len;
    }
    out
}

fn sorted(mut notes: Vec<Note>) -> Vec<Note> {
    notes.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
    notes
}

/// Rewrites every region placed from pattern `id`.
pub fn push(p: &mut Project, id: u64) {
    let Some(pattern) = p.patterns.iter().find(|x| x.id == id).cloned() else {
        return;
    };
    for t in 0..p.tracks.len() {
        let channel = p.tracks[t].params.channel;
        for r in 0..p.tracks[t].regions.len() {
            if p.tracks[t].regions[r].params.pattern != id {
                continue;
            }
            let len = p.region_len_beats(&p.tracks[t].regions[r]) as f32;
            let lane = pattern.lane(channel).map_or(&[][..], |l| &l.notes[..]);
            let expected = sorted(looped(lane, pattern.len_beats(), len));
            let region = &mut p.tracks[t].regions[r];
            region.params.pattern_len = len;
            region.params.pattern_color = Some(pattern.color);
            if sorted(region.notes.clone()) != expected {
                region.notes = expected;
                p.song_rev += 1;
            }
        }
    }
}

/// Takes an edit of a region back into its lane: what was changed within the part of the pattern
/// the region shows. Notes the region does not reach are kept.
fn take_back(lane: &mut Lane, region_notes: &[Note], pattern_len: f32, region_len: f32) {
    let reach = pattern_len.min(region_len);
    lane.notes.retain(|n| n.start >= reach);
    lane.notes
        .extend(region_notes.iter().filter(|n| n.start < reach).copied());
}

/// Keeps regions and lanes the same. Run when no edit is under way: a drag ends before this.
pub fn sync(p: &mut Project) {
    if p.patterns.is_empty() && !p.tracks.iter().any(has_links) {
        return;
    }
    // A copied track has its original's channel number. The copy is a channel of its own, and
    // what it copied stays as notes.
    let mut seen = Vec::new();
    for t in 0..p.tracks.len() {
        let channel = p.tracks[t].params.channel;
        if channel != 0 && seen.contains(&channel) {
            p.tracks[t].params.channel = 0;
            for region in &mut p.tracks[t].regions {
                region.params.pattern = 0;
                region.params.pattern_color = None;
            }
        } else {
            seen.push(channel);
        }
    }
    let mut changed = Vec::new();
    // One region of a lane taken back; its copies are then behind, not edited.
    let mut taken = Vec::new();
    for t in 0..p.tracks.len() {
        let channel = p.tracks[t].params.channel;
        for r in 0..p.tracks[t].regions.len() {
            let id = p.tracks[t].regions[r].params.pattern;
            if id == 0 {
                continue;
            }
            let Some(at) = p.patterns.iter().position(|x| x.id == id) else {
                make_unique(p, t, r);
                continue;
            };
            let len = p.region_len_beats(&p.tracks[t].regions[r]) as f32;
            let region = &p.tracks[t].regions[r];
            // Made longer or shorter, or just opened: it only needs making again.
            if (region.params.pattern_len - len).abs() > 1e-4 {
                if !changed.contains(&id) {
                    changed.push(id);
                }
                continue;
            }
            if taken.contains(&(id, channel)) {
                continue;
            }
            let pattern = &p.patterns[at];
            let pattern_len = pattern.len_beats();
            let lane = pattern.lane(channel).map_or(&[][..], |l| &l.notes[..]);
            let expected = sorted(looped(lane, pattern_len, len));
            if sorted(region.notes.clone()) != expected {
                let notes = region.notes.clone();
                take_back(p.patterns[at].lane_mut(channel), &notes, pattern_len, len);
                taken.push((id, channel));
                if !changed.contains(&id) {
                    changed.push(id);
                }
            }
        }
    }
    for id in changed {
        push(p, id);
    }
}

fn has_links(t: &crate::model::Track) -> bool {
    t.regions.iter().any(|r| r.params.pattern != 0)
}

/// Puts pattern `id` on track `t` at `bar` (counted from 1). Returns the region.
pub fn place_on(p: &mut Project, id: u64, t: usize, bar: f32) -> Option<usize> {
    let pattern = p.patterns.iter().find(|x| x.id == id)?;
    let (name, len_beats) = (pattern.name.clone(), f64::from(pattern.len_beats()));
    let channel = channel_of(p, t);
    let bar_len = p.bar_len_at(p.bar_to_beats(f64::from(bar) - 1.0)).max(1e-3);
    let r = p.add_midi_region(t, bar)?;
    let region = &mut p.tracks[t].regions[r];
    region.name = name;
    region.len_bars = (len_beats / bar_len) as f32;
    region.params.pattern = id;
    if let Some(pattern) = p.patterns.iter_mut().find(|x| x.id == id) {
        pattern.lane_mut(channel);
    }
    push(p, id);
    Some(r)
}

/// Places pattern `id` at `bar`: a region on every channel it has notes for. Returns how many.
pub fn place(p: &mut Project, id: u64, bar: f32) -> usize {
    let Some(pattern) = p.patterns.iter().find(|x| x.id == id) else {
        return 0;
    };
    let used: Vec<u64> = pattern
        .lanes
        .iter()
        .filter(|l| !l.notes.is_empty())
        .map(|l| l.channel)
        .collect();
    let tracks: Vec<usize> = (0..p.tracks.len())
        .filter(|&t| used.contains(&p.tracks[t].params.channel))
        .collect();
    tracks
        .into_iter()
        .filter_map(|t| place_on(p, id, t, bar))
        .count()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// A project at 4/4 with two instrument tracks and a pattern with a step on each.
    fn project() -> (Project, u64, u64, u64) {
        let mut p = Project::empty();
        p.tracks.clear();
        let (a, b) = (
            p.add_track(TrackKind::Instrument),
            p.add_track(TrackKind::Instrument),
        );
        let id = new_pattern(&mut p);
        let (ca, cb) = (channel_of(&mut p, a), channel_of(&mut p, b));
        assert_ne!(ca, cb);
        let pattern = p.patterns.iter_mut().find(|x| x.id == id).unwrap();
        pattern.lane_mut(ca).toggle_step(0);
        pattern.lane_mut(cb).toggle_step(4);
        (p, id, ca, cb)
    }

    fn pattern(p: &mut Project, id: u64) -> &mut SongPattern {
        p.patterns.iter_mut().find(|x| x.id == id).unwrap()
    }

    #[test]
    fn steps_switch_notes_and_leave_other_notes_alone() {
        let mut lane = Lane {
            channel: 1,
            step_pitch: 60,
            notes: vec![Note {
                start: 0.1,
                len: 0.5,
                pitch: 64,
                vel: 90,
                muted: false,
                color: 0,
                slide: false,
            }],
        };
        lane.toggle_step(2);
        assert!(lane.step_is_on(2));
        assert_eq!(lane.notes.len(), 2);
        assert_eq!(lane.off_grid().count(), 1, "the note the grid cannot show");
        lane.toggle_step(2);
        assert!(!lane.step_is_on(2));
        assert_eq!(lane.notes.len(), 1, "it is still there");
        // Another step pitch does not touch them either.
        lane.toggle_step(0);
        lane.step_pitch = 72;
        assert!(!lane.step_is_on(0));
        lane.toggle_step(0);
        assert_eq!(lane.notes.len(), 3);
    }

    #[test]
    fn placing_makes_a_region_on_each_channel_used() {
        let (mut p, id, ..) = project();
        assert_eq!(place(&mut p, id, 3.0), 2);
        for t in 0..2 {
            let region = &p.tracks[t].regions[0];
            assert_eq!(region.params.pattern, id);
            assert_eq!(region.start_bar, 3.0);
            assert_eq!(region.len_bars, 1.0, "16 steps are a bar of 4/4");
            assert_eq!(region.notes.len(), 1);
        }
        assert_eq!(p.tracks[1].regions[0].notes[0].start, 1.0);
    }

    #[test]
    fn editing_the_lane_updates_every_placed_clip() {
        let (mut p, id, ca, _) = project();
        place(&mut p, id, 1.0);
        place(&mut p, id, 5.0);
        let rev = p.song_rev;
        pattern(&mut p, id).lane_mut(ca).toggle_step(8);
        push(&mut p, id);
        assert!(p.song_rev > rev);
        assert_eq!(p.tracks[0].regions[0].notes.len(), 2);
        assert_eq!(p.tracks[0].regions[1].notes.len(), 2);
    }

    #[test]
    fn a_piano_roll_edit_reaches_the_lane_and_the_other_clips() {
        let (mut p, id, ca, _) = project();
        place(&mut p, id, 1.0);
        place(&mut p, id, 5.0);
        p.tracks[0].regions[0].notes.push(Note {
            start: 2.5,
            len: 0.5,
            pitch: 67,
            vel: 80,
            muted: false,
            color: 0,
            slide: false,
        });
        sync(&mut p);
        assert_eq!(pattern(&mut p, id).lane(ca).unwrap().notes.len(), 2);
        assert_eq!(p.tracks[0].regions[1].notes.len(), 2);
        // Settled: nothing more happens.
        let rev = p.song_rev;
        sync(&mut p);
        assert_eq!(p.song_rev, rev);
    }

    #[test]
    fn a_longer_region_loops_and_resizing_is_not_an_edit() {
        let (mut p, id, ca, _) = project();
        place(&mut p, id, 1.0);
        p.tracks[0].regions[0].len_bars = 2.0;
        sync(&mut p);
        assert_eq!(p.tracks[0].regions[0].notes.len(), 2, "played twice");
        assert_eq!(p.tracks[0].regions[0].notes[1].start, 4.0);
        // Shorter than a step's place: the note it no longer reaches stays in the lane.
        pattern(&mut p, id).lane_mut(ca).toggle_step(12);
        push(&mut p, id);
        p.tracks[0].regions[0].len_bars = 0.5;
        sync(&mut p);
        assert_eq!(p.tracks[0].regions[0].notes.len(), 1);
        assert_eq!(pattern(&mut p, id).lane(ca).unwrap().notes.len(), 2);
    }

    #[test]
    fn a_shorter_pattern_keeps_the_notes_past_its_end() {
        let (mut p, id, ca, _) = project();
        pattern(&mut p, id).lane_mut(ca).toggle_step(12);
        place(&mut p, id, 1.0);
        pattern(&mut p, id).steps = 8;
        push(&mut p, id);
        assert!(
            p.tracks[0].regions[0].notes.iter().all(|n| n.start != 3.0),
            "the note past its end is not played"
        );
        assert_eq!(pattern(&mut p, id).lane(ca).unwrap().notes.len(), 2);
        pattern(&mut p, id).steps = 16;
        push(&mut p, id);
        assert_eq!(p.tracks[0].regions[0].notes.len(), 2, "back again");
    }

    #[test]
    fn make_unique_and_clone_detach_a_clip() {
        let (mut p, id, ..) = project();
        place(&mut p, id, 1.0);
        place(&mut p, id, 5.0);
        make_unique(&mut p, 0, 0);
        assert_eq!(p.tracks[0].regions[0].params.pattern, 0);
        assert_eq!(p.tracks[0].regions[0].notes.len(), 1, "it keeps its notes");
        let new = clone_for_region(&mut p, 0, 1).unwrap();
        assert_ne!(new, id);
        assert_eq!(p.tracks[0].regions[1].params.pattern, new);
        assert_eq!(p.patterns.len(), 2);
    }

    #[test]
    fn deleting_a_pattern_leaves_notes_behind_and_a_copied_track_goes_its_own_way() {
        let (mut p, id, ..) = project();
        place(&mut p, id, 1.0);
        let copy = p.tracks[0].clone();
        p.tracks.push(copy);
        sync(&mut p);
        assert_eq!(p.tracks[2].params.channel, 0);
        assert_eq!(p.tracks[2].regions[0].params.pattern, 0);
        assert_eq!(p.tracks[2].regions[0].notes.len(), 1);
        delete_pattern(&mut p, id);
        assert_eq!(p.tracks[0].regions[0].params.pattern, 0);
        assert_eq!(p.tracks[0].regions[0].notes.len(), 1);
    }
}
