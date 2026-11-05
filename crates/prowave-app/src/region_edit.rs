//! Editing regions on the Tracks area: cut, copy, paste, split, join and trim. Each function does
//! one edit on the project and says why when it cannot; the windows and menus only call them.
//!
//! Audio regions are windows on their file: `source_start` is where in the file the region
//! begins and `source_len` how much of it plays (see `RegionParams`). Nothing here changes a file.
use crate::model::{Control, Note, Project, Region, TrackKind};
use crate::tracks_area::TracksView;

/// Closer than this to a region's edge, in beats, is at its edge: nothing to cut there.
const EDGE_BEATS: f64 = 1.0 / 256.0;
/// Two audio regions join when the second carries on the first within this, in seconds.
const JOIN_SECONDS: f32 = 0.005;

fn region_at(p: &Project, t: usize, r: usize) -> Result<&Region, String> {
    p.tracks
        .get(t)
        .and_then(|track| track.regions.get(r))
        .ok_or_else(|| "That region is gone".to_string())
}

fn seconds_between(p: &Project, from: f64, to: f64) -> f32 {
    let map = p.tempo_map();
    (map.seconds_at(to) - map.seconds_at(from)) as f32
}

/// How much of its file an audio region plays, from its `source_start`, in seconds.
fn played_seconds(region: &Region) -> f32 {
    let Some(a) = region.audio.as_ref() else {
        return 0.0;
    };
    let rest = (a.file.duration_secs() as f32 - region.params.source_start).max(0.0);
    if region.params.source_len > 0.0 {
        region.params.source_len.min(rest)
    } else {
        rest
    }
}

/// Cuts region `r` of track `t` in two at `beat`. Returns the number of the second part.
pub fn split(p: &mut Project, t: usize, r: usize, beat: f64) -> Result<usize, String> {
    let region = region_at(p, t, r)?;
    if region.folder.is_some() {
        return Err("Pick a take first: a take folder cannot be split".to_string());
    }
    let start = p.region_start_beats(region);
    let end = p.region_end_beats(region);
    if beat <= start + EDGE_BEATS || beat >= end - EDGE_BEATS {
        return Err("The cut has to be inside the region".to_string());
    }
    let cut_seconds = seconds_between(p, start, beat);
    let cut_bar = p.beats_to_bar(beat) as f32;
    let at = (beat - start) as f32;
    let played = played_seconds(region);
    if region.audio.is_some() && cut_seconds >= played {
        return Err("There is no audio left to cut there".to_string());
    }

    // A part of a pattern's clip is not the pattern any more.
    crate::song_pattern::make_unique(p, t, r);
    let region = &mut p.tracks[t].regions[r];
    let mut right = region.clone();
    let whole_len = region.len_bars;
    region.len_bars = cut_bar - (region.start_bar - 1.0);
    right.start_bar = cut_bar + 1.0;
    right.len_bars = whole_len - region.len_bars;

    // A generated region's notes are made for its whole length, so the parts become plain.
    region.generated = None;
    right.generated = None;
    // A note under the cut sounds in both parts.
    let notes = std::mem::take(&mut region.notes);
    right.notes.clear();
    for n in notes {
        if n.start < at {
            region.notes.push(Note {
                len: n.len.min(at - n.start),
                ..n
            });
        }
        if n.start + n.len > at {
            let from = n.start.max(at);
            right.notes.push(Note {
                start: from - at,
                len: n.start + n.len - from,
                ..n
            });
        }
    }
    let controls = std::mem::take(&mut region.controls);
    let (before, after): (Vec<Control>, Vec<Control>) =
        controls.into_iter().partition(|c| c.start < at);
    region.controls = before;
    right.controls = after
        .into_iter()
        .map(|c| Control {
            start: c.start - at,
            ..c
        })
        .collect();

    right.auto = crate::automation::cut_lanes(&mut region.auto, f64::from(at));
    if region.audio.is_some() {
        let first = region.params.source_start;
        let rest = played - cut_seconds;
        if region.params.reverse {
            // Played backwards, what sounds first is the end of the file.
            region.params.source_start = first + rest;
            right.params.source_start = first;
        } else {
            right.params.source_start = first + cut_seconds;
        }
        region.params.source_len = cut_seconds;
        right.params.source_len = rest;
    }
    // Their fades stay at the outer ends.
    region.params.fade_out = 0.0;
    right.params.fade_in = 0.0;
    p.tracks[t].regions.push(right);
    p.song_rev += 1;
    Ok(p.tracks[t].regions.len() - 1)
}

/// The region that begins where region `r` of track `t` ends, or the nearest one after it.
fn next_region(p: &Project, t: usize, r: usize) -> Option<usize> {
    let track = p.tracks.get(t)?;
    let start = track.regions.get(r)?.start_bar;
    track
        .regions
        .iter()
        .enumerate()
        .filter(|(i, other)| *i != r && other.start_bar >= start)
        .min_by(|a, b| a.1.start_bar.total_cmp(&b.1.start_bar))
        .map(|(i, _)| i)
}

/// Joins region `r` of track `t` with the next region on the track. MIDI regions always can; two
/// audio regions only when the second carries on the first in the same file, since anything else
/// needs a new file (Bounce). Returns the number of the joined region.
pub fn join_with_next(p: &mut Project, t: usize, r: usize) -> Result<usize, String> {
    let next = next_region(p, t, r).ok_or_else(|| "There is no region after it".to_string())?;
    crate::song_pattern::make_unique(p, t, r);
    crate::song_pattern::make_unique(p, t, next);
    let (first, second) = (region_at(p, t, r)?, region_at(p, t, next)?);
    if first.folder.is_some() || second.folder.is_some() {
        return Err("Pick a take first: a take folder cannot be joined".to_string());
    }
    match (&first.audio, &second.audio) {
        (Some(a), Some(b)) => {
            let same_file = a.file.path == b.file.path;
            let carries_on = !first.params.reverse
                && !second.params.reverse
                && (first.params.source_start + played_seconds(first) - second.params.source_start)
                    .abs()
                    < JOIN_SECONDS;
            let touching = (first.start_bar + first.len_bars - second.start_bar).abs() < 0.01;
            if !(same_file && carries_on && touching) {
                return Err(
                    "These audio regions are not one stretch of a file: joining them needs Bounce"
                        .to_string(),
                );
            }
        }
        (None, None) => {}
        _ => return Err("An audio region and a MIDI region cannot be joined".to_string()),
    }
    let shift = (p.region_start_beats(second) - p.region_start_beats(first)) as f32;
    let end_bar = (first.start_bar + first.len_bars).max(second.start_bar + second.len_bars);
    let played = played_seconds(first) + played_seconds(second);
    let second = second.clone();

    let region = &mut p.tracks[t].regions[r];
    region.len_bars = end_bar - region.start_bar;
    region.generated = None;
    region.notes.extend(second.notes.iter().map(|n| Note {
        start: n.start + shift,
        ..*n
    }));
    region
        .controls
        .extend(second.controls.iter().map(|c| Control {
            start: c.start + shift,
            ..*c
        }));
    crate::automation::join_lanes(&mut region.auto, &second.auto, f64::from(shift));
    region.params.fade_out = second.params.fade_out;
    if region.audio.is_some() {
        region.params.source_len = played;
    }
    p.tracks[t].regions.remove(next);
    p.song_rev += 1;
    Ok(if next < r { r - 1 } else { r })
}

/// Moves the end of a region to `beat`. An audio region cannot run past the end of its file.
pub fn trim_end(p: &mut Project, t: usize, r: usize, beat: f64) -> Result<(), String> {
    let region = region_at(p, t, r)?;
    let start = p.region_start_beats(region);
    let beat = beat.max(start + EDGE_BEATS * 4.0);
    let mut seconds = seconds_between(p, start, beat);
    let mut beat = beat;
    if let Some(a) = region.audio.as_ref() {
        let room = (a.file.duration_secs() as f32 - region.params.source_start).max(0.0);
        if seconds > room {
            seconds = room;
            let map = p.tempo_map();
            beat = map.beat_at(map.seconds_at(start) + f64::from(room));
        }
    }
    let bar = p.beats_to_bar(beat) as f32;
    let region = &mut p.tracks[t].regions[r];
    region.len_bars = (bar - (region.start_bar - 1.0)).max(1.0 / 64.0);
    if region.audio.is_some() {
        region.params.source_len = seconds;
    }
    Ok(())
}

/// What a trim of a region's start works from, so a drag can go back the way it came.
#[derive(Clone)]
pub struct TrimStart {
    beat: f64,
    end: f64,
    source_start: f32,
    played: f32,
    notes: Vec<Note>,
    controls: Vec<Control>,
    auto: Vec<crate::model::RegionLane>,
}

impl TrimStart {
    pub fn of(p: &Project, t: usize, r: usize) -> Result<Self, String> {
        let region = region_at(p, t, r)?;
        Ok(Self {
            beat: p.region_start_beats(region),
            end: p.region_end_beats(region),
            source_start: region.params.source_start,
            played: played_seconds(region),
            notes: region.notes.clone(),
            controls: region.controls.clone(),
            auto: region.auto.clone(),
        })
    }
}

/// Moves the start of a region to `beat`, its end staying where it is. What it plays stays in
/// place: an audio region starts later in its file, notes before the new start are cut off.
pub fn trim_start(p: &mut Project, t: usize, r: usize, from: &TrimStart, beat: f64) {
    crate::song_pattern::make_unique(p, t, r);
    let mut beat = beat.clamp(0.0, from.end - EDGE_BEATS * 4.0);
    let is_audio = region_at(p, t, r).is_ok_and(|x| x.audio.is_some());
    if is_audio {
        // It cannot start before the beginning of the file.
        let map = p.tempo_map();
        let earliest = map
            .beat_at(map.seconds_at(from.beat) - f64::from(from.source_start))
            .max(0.0);
        beat = beat.max(earliest);
    }
    let moved = (beat - from.beat) as f32;
    let seconds = if beat >= from.beat {
        seconds_between(p, from.beat, beat)
    } else {
        -seconds_between(p, beat, from.beat)
    };
    let start_bar = p.beats_to_bar(beat) as f32 + 1.0;
    let end_bar = p.beats_to_bar(from.end) as f32 + 1.0;
    let Some(region) = p.tracks.get_mut(t).and_then(|x| x.regions.get_mut(r)) else {
        return;
    };
    region.start_bar = start_bar;
    region.len_bars = (end_bar - start_bar).max(1.0 / 64.0);
    region.notes = from
        .notes
        .iter()
        .filter(|n| n.start + n.len > moved)
        .map(|n| {
            let start = n.start.max(moved);
            Note {
                start: start - moved,
                len: n.start + n.len - start,
                ..*n
            }
        })
        .collect();
    region.controls = from
        .controls
        .iter()
        .filter(|c| c.start >= moved)
        .map(|c| Control {
            start: c.start - moved,
            ..*c
        })
        .collect();
    region.auto = crate::automation::trimmed_lanes(&from.auto, f64::from(moved));
    if is_audio {
        if !region.params.reverse {
            region.params.source_start = (from.source_start + seconds).max(0.0);
        }
        region.params.source_len = (from.played - seconds).max(0.0);
    }
}

/// Keeps copies of the selected regions for Paste.
pub fn copy(p: &Project, v: &mut TracksView) -> Result<(), String> {
    let selected = v.all_selected();
    if selected.is_empty() {
        return Err("Select a region first".to_string());
    }
    let mut clip = Vec::with_capacity(selected.len());
    for (t, r) in selected {
        clip.push((t, region_at(p, t, r)?.clone()));
    }
    v.clipboard = clip;
    Ok(())
}

/// Takes the stretch `from..to` (beats) off a track: regions across its edges are split there and
/// what is inside goes. A region that cannot be split (a take folder) stays, and that is said.
fn clear_range(p: &mut Project, t: usize, from: f64, to: f64) -> Result<(), String> {
    const NEAR: f64 = 1e-4;
    let span = |p: &Project, r: &Region| {
        (
            p.bar_to_beats(f64::from(r.start_bar) - 1.0),
            p.region_end_beats(r),
        )
    };
    for edge in [from, to] {
        let across = p.tracks[t].regions.iter().position(|r| {
            let (a, b) = span(p, r);
            a + NEAR < edge && edge < b - NEAR
        });
        if let Some(r) = across {
            split(p, t, r, edge)?;
        }
    }
    let inside: Vec<bool> = p.tracks[t]
        .regions
        .iter()
        .map(|r| {
            let (a, b) = span(p, r);
            a >= from - NEAR && b <= to + NEAR
        })
        .collect();
    let mut flags = inside.into_iter();
    p.tracks[t]
        .regions
        .retain(|_| !flags.next().unwrap_or(false));
    Ok(())
}

/// Edit > Paste Replace: pastes at the playhead and takes away what was under the pasted regions.
pub fn paste_replace(p: &mut Project, v: &mut TracksView) -> Result<(), String> {
    paste(p, v)?;
    let mut placed = v.all_selected();
    // The last index first, so the ones still to come stay where they are.
    placed.sort_unstable_by(|a, b| b.cmp(a));
    let taken: Vec<(usize, Region)> = placed
        .into_iter()
        .map(|(t, r)| (t, p.tracks[t].regions.remove(r)))
        .collect();
    let mut cleared = Ok(());
    for (t, region) in &taken {
        let from = p.bar_to_beats(f64::from(region.start_bar) - 1.0);
        let to = p.region_end_beats(region);
        if let Err(e) = clear_range(p, *t, from, to) {
            cleared = Err(e);
        }
    }
    let mut again = Vec::with_capacity(taken.len());
    for (t, region) in taken {
        p.tracks[t].regions.push(region);
        again.push((t, p.tracks[t].regions.len() - 1));
    }
    v.select_many(again);
    p.song_rev += 1;
    cleared
}

/// Moves the starts of the notes of the selected MIDI regions to the grid (in beats). Pattern and
/// Drummer regions make their notes from settings and take folders from their takes, so they stay
/// as they are. How many regions changed.
pub fn quantize_selected(p: &mut Project, v: &TracksView, grid: f32) -> usize {
    let mut done = 0;
    for (t, r) in v.all_selected() {
        let Some(region) = p.tracks.get_mut(t).and_then(|x| x.regions.get_mut(r)) else {
            continue;
        };
        if region.audio.is_some() || region.generated.is_some() || region.folder.is_some() {
            continue;
        }
        let all: Vec<usize> = (0..region.notes.len()).collect();
        crate::note_edit::quantize(&mut region.notes, &all, grid, 1.0, 0.0);
        done += 1;
    }
    if done > 0 {
        p.song_rev += 1;
    }
    done
}

/// Where regions dragged up or down were dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropRow {
    Track(usize),
    /// Under the last track.
    Below,
}

/// Whether `t` can hold `region`: audio on an audio track, MIDI on one that plays MIDI.
fn track_holds(p: &Project, t: usize, region: &Region) -> bool {
    p.tracks.get(t).is_some_and(|track| {
        track.kind.has_regions() && (track.kind == TrackKind::Audio) == region.audio.is_some()
    })
}

/// Moves `regions` to the track they were dropped on, keeping their distances between tracks. The
/// one grabbed is `lead`. Dropped under the last track, or on a track that cannot hold it, it gets
/// a new track of its own kind. A clip of the Channel Rack belongs to its channel and stays. They
/// end up selected. Nothing moves during a recording, which is still writing into its region.
pub fn move_to_track(
    p: &mut Project,
    v: &mut TracksView,
    regions: &[(usize, usize)],
    lead: (usize, usize),
    drop: DropRow,
) {
    let Some(grabbed) = p.tracks.get(lead.0).and_then(|t| t.regions.get(lead.1)) else {
        return;
    };
    let is_audio = grabbed.audio.is_some();
    if p.recording {
        return;
    }
    let target = match drop {
        DropRow::Track(t) if t == lead.0 => return,
        DropRow::Track(t) if track_holds(p, t, grabbed) => t,
        _ => p.add_track(if is_audio {
            TrackKind::Audio
        } else {
            TrackKind::Instrument
        }),
    };
    let new_track = target >= p.tracks.len().saturating_sub(1)
        && p.tracks[target].regions.is_empty()
        && !matches!(drop, DropRow::Track(t) if t == target);
    // From the back of each track, so the ones still to go keep their numbers.
    let mut going = regions.to_vec();
    going.sort_by(|a, b| b.cmp(a));
    let mut placed = Vec::new();
    for (t, r) in going {
        let Some(region) = p.tracks.get(t).and_then(|x| x.regions.get(r)) else {
            continue;
        };
        let dest = if new_track {
            target
        } else {
            (t + target).saturating_sub(lead.0)
        };
        if dest == t || region.params.pattern != 0 || !track_holds(p, dest, region) {
            continue;
        }
        let region = p.tracks[t].regions.remove(r);
        p.tracks[dest].regions.push(region);
        placed.push((dest, p.tracks[dest].regions.len() - 1));
    }
    if placed.is_empty() {
        return;
    }
    p.selected_track = target;
    v.select_many(placed);
    p.song_rev += 1;
}

/// Moves the selected regions together so the earliest starts at the playhead.
pub fn move_selected_to_playhead(p: &mut Project, v: &TracksView) {
    let selected = v.all_selected();
    let earliest = selected
        .iter()
        .filter_map(|(t, r)| p.tracks.get(*t)?.regions.get(*r))
        .map(|r| r.start_bar)
        .fold(f32::INFINITY, f32::min);
    if !earliest.is_finite() {
        return;
    }
    let shift = p.beats_to_bar(p.position_beats) as f32 + 1.0 - earliest;
    for (t, r) in selected {
        if let Some(region) = p.tracks.get_mut(t).and_then(|x| x.regions.get_mut(r)) {
            region.start_bar += shift;
        }
    }
    p.song_rev += 1;
}

/// Puts copies of what was copied at the playhead, keeping their distances in time and between
/// tracks: starting on the selected track if it can hold them, else on the tracks they came from.
/// They end up selected. Returns where the first one went.
pub fn paste(p: &mut Project, v: &mut TracksView) -> Result<(usize, usize), String> {
    if v.clipboard.is_empty() {
        return Err("There is nothing to paste".to_string());
    }
    let clip = v.clipboard.clone();
    let earliest = clip
        .iter()
        .map(|(_, r)| r.start_bar)
        .fold(f32::INFINITY, f32::min);
    let first_track = clip.iter().map(|(t, _)| *t).min().unwrap_or(0);
    let shift = p.beats_to_bar(p.position_beats) as f32 + 1.0 - earliest;
    let holds = |p: &Project, t: usize, region: &Region| {
        p.tracks.get(t).is_some_and(|track| {
            track.kind.has_regions() && (track.kind == TrackKind::Audio) == region.audio.is_some()
        })
    };
    let base = p.selected_track;
    let mut placed = Vec::with_capacity(clip.len());
    for (from, region) in clip {
        let there = base + (from - first_track);
        let target = [there, from]
            .into_iter()
            .find(|t| holds(p, *t, &region))
            .ok_or_else(|| "No track here can hold that region".to_string())?;
        let start_bar = region.start_bar + shift;
        p.tracks[target].regions.push(Region {
            start_bar,
            ..region
        });
        placed.push((target, p.tracks[target].regions.len() - 1));
    }
    p.song_rev += 1;
    let first = placed[0];
    p.selected_track = first.0;
    v.select_many(placed);
    Ok(first)
}

/// Whether Split at Playhead would do something.
pub fn can_split_at_playhead(p: &Project, v: &TracksView) -> bool {
    regions_to_split(p, v).next().is_some()
}

/// The selected region if the playhead is inside it, else whatever is under the playhead on the
/// selected track.
fn regions_to_split<'a>(
    p: &'a Project,
    v: &TracksView,
) -> impl Iterator<Item = (usize, usize)> + 'a {
    let beat = p.position_beats;
    let inside = move |t: usize, r: usize| {
        region_at(p, t, r).is_ok_and(|region| {
            region.folder.is_none()
                && beat > p.region_start_beats(region) + EDGE_BEATS
                && beat < p.region_end_beats(region) - EDGE_BEATS
        })
    };
    let picked: Vec<(usize, usize)> = v
        .all_selected()
        .into_iter()
        .filter(|(t, r)| inside(*t, *r))
        .collect();
    let track = p.selected_track;
    let count = p.tracks.get(track).map_or(0, |x| x.regions.len());
    let under: Vec<(usize, usize)> = if picked.is_empty() {
        (0..count)
            .filter(|r| inside(track, *r))
            .map(|r| (track, r))
            .collect()
    } else {
        picked
    };
    under.into_iter()
}

/// Splits at the playhead. Afterwards the second part is selected, ready to move or delete.
pub fn split_at_playhead(p: &mut Project, v: &mut TracksView) -> Result<(), String> {
    let targets: Vec<(usize, usize)> = regions_to_split(p, v).collect();
    if targets.is_empty() {
        return Err("Put the playhead inside a region to split it".to_string());
    }
    let beat = p.position_beats;
    let mut rights = Vec::with_capacity(targets.len());
    for (t, r) in targets {
        rights.push((t, split(p, t, r, beat)?));
    }
    v.select_many(rights);
    Ok(())
}

/// Whether there is a region after the selected one to join it with.
pub fn can_join(p: &Project, v: &TracksView) -> bool {
    v.selected
        .is_some_and(|(t, r)| next_region(p, t, r).is_some())
}

/// Joins the selected regions, track by track; a single region joins with the one after it.
pub fn join_selected(p: &mut Project, v: &mut TracksView) -> Result<(), String> {
    let selected = v.all_selected();
    if selected.is_empty() {
        return Err("Select a region first".to_string());
    }
    let mut tracks: Vec<usize> = selected.iter().map(|(t, _)| *t).collect();
    tracks.sort_unstable();
    tracks.dedup();
    let mut joined = Vec::new();
    for t in tracks {
        let mine: Vec<usize> = selected
            .iter()
            .filter(|(track, _)| *track == t)
            .map(|(_, r)| *r)
            .collect();
        let earliest = mine.iter().copied().min_by(|a, b| {
            let bar = |r: &usize| p.tracks[t].regions.get(*r).map_or(0.0, |x| x.start_bar);
            bar(a).total_cmp(&bar(b))
        });
        let Some(mut at) = earliest else { continue };
        // Those selected on the track, or one join when only one is.
        for _ in 0..mine.len().saturating_sub(1).max(1) {
            at = join_with_next(p, t, at)?;
        }
        joined.push((t, at));
    }
    v.select_many(joined);
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::model::TrackKind;
    use std::path::PathBuf;

    fn note(start: f32, len: f32, pitch: u8) -> Note {
        Note {
            start,
            len,
            pitch,
            vel: 100,
            muted: false,
        }
    }

    #[test]
    fn regions_move_between_tracks_and_a_new_track_is_made_when_needed() {
        let (mut p, t) = midi_project();
        let other = p.add_track(TrackKind::Instrument);
        let audio = p.add_track(TrackKind::Audio);
        let mut v = TracksView::default();
        move_to_track(&mut p, &mut v, &[(t, 0)], (t, 0), DropRow::Track(other));
        assert!(p.tracks[t].regions.is_empty());
        assert_eq!(
            p.tracks[other].regions.len(),
            1,
            "onto a track that can hold it"
        );
        assert_eq!(v.selected, Some((other, 0)));
        let tracks = p.tracks.len();
        move_to_track(
            &mut p,
            &mut v,
            &[(other, 0)],
            (other, 0),
            DropRow::Track(audio),
        );
        assert_eq!(
            p.tracks.len(),
            tracks + 1,
            "MIDI cannot go on an audio track"
        );
        assert_eq!(p.tracks[tracks].kind, TrackKind::Instrument);
        assert_eq!(p.tracks[tracks].regions.len(), 1);
        move_to_track(&mut p, &mut v, &[(tracks, 0)], (tracks, 0), DropRow::Below);
        assert_eq!(p.tracks.len(), tracks + 2, "under the last track");
        assert_eq!(p.tracks[tracks + 1].regions.len(), 1);
        p.recording = true;
        move_to_track(
            &mut p,
            &mut v,
            &[(tracks + 1, 0)],
            (tracks + 1, 0),
            DropRow::Below,
        );
        assert_eq!(p.tracks.len(), tracks + 2, "not while recording");
    }

    /// One instrument track with a region of four bars at bar 1, at 120 BPM in 4/4.
    fn midi_project() -> (Project, usize) {
        let mut p = Project::empty();
        p.tempo = 120.0;
        let t = p.add_track(TrackKind::Instrument);
        p.add_midi_region(t, 1.0).unwrap();
        (p, t)
    }

    /// A second region of `seconds` of audio on a new audio track, at bar 1.
    fn audio_project(seconds: f64) -> (Project, usize, tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("take.wav");
        let frames = (seconds * 48_000.0) as usize;
        prowave_engine::write_wav(&wav, &vec![0.1; frames * 2], 48_000).unwrap();
        let mut p = Project::empty();
        p.tempo = 120.0;
        let ctx = egui::Context::default();
        crate::import::import_audio_at(&mut p, &wav, &ctx, 1.0, None).unwrap();
        let t = p.tracks.len() - 1;
        (p, t, dir, wav)
    }

    #[test]
    fn a_split_shares_the_notes_and_a_note_under_the_cut_sounds_in_both() {
        let (mut p, t) = midi_project();
        p.tracks[t].regions[0].notes =
            vec![note(1.0, 1.0, 60), note(7.0, 2.0, 62), note(9.0, 1.0, 64)];
        p.tracks[t].regions[0].controls = vec![
            Control {
                start: 1.0,
                num: 64,
                val: 127,
            },
            Control {
                start: 9.0,
                num: 64,
                val: 0,
            },
        ];
        p.tracks[t].regions[0].params.fade_in = 1.0;
        p.tracks[t].regions[0].params.fade_out = 2.0;
        let rev = p.song_rev;
        let right = split(&mut p, t, 0, 8.0).unwrap();
        assert!(p.song_rev > rev);
        let (a, b) = (&p.tracks[t].regions[0], &p.tracks[t].regions[right]);
        assert_eq!((a.start_bar, a.len_bars), (1.0, 2.0));
        assert_eq!((b.start_bar, b.len_bars), (3.0, 2.0));
        assert_eq!(a.notes, [note(1.0, 1.0, 60), note(7.0, 1.0, 62)]);
        assert_eq!(b.notes, [note(0.0, 1.0, 62), note(1.0, 1.0, 64)]);
        assert_eq!((a.controls.len(), b.controls.len()), (1, 1));
        assert_eq!(b.controls[0].start, 1.0);
        assert_eq!((a.params.fade_in, a.params.fade_out), (1.0, 0.0));
        assert_eq!((b.params.fade_in, b.params.fade_out), (0.0, 2.0));
        // Nothing to cut at or past the edges.
        assert!(split(&mut p, t, 0, 0.0).is_err());
        assert!(split(&mut p, t, 0, 8.0).is_err());
    }

    #[test]
    fn a_split_audio_region_plays_on_from_where_the_first_part_stopped() {
        // Eight seconds is four bars at 120 BPM; two bars in is four seconds in.
        let (mut p, t, _dir, _wav) = audio_project(8.0);
        let right = split(&mut p, t, 0, 8.0).unwrap();
        let (a, b) = (&p.tracks[t].regions[0], &p.tracks[t].regions[right]);
        assert_eq!((a.params.source_start, a.params.source_len), (0.0, 4.0));
        assert_eq!((b.params.source_start, b.params.source_len), (4.0, 4.0));
        assert_eq!(b.start_bar, 3.0);

        // And again in the second part: its own window is cut.
        let last = split(&mut p, t, right, 12.0).unwrap();
        let (b, c) = (&p.tracks[t].regions[right], &p.tracks[t].regions[last]);
        assert_eq!((b.params.source_start, b.params.source_len), (4.0, 2.0));
        assert_eq!((c.params.source_start, c.params.source_len), (6.0, 2.0));
    }

    #[test]
    fn a_split_reversed_region_sounds_the_same_in_order() {
        let (mut p, t, _dir, _wav) = audio_project(8.0);
        p.tracks[t].regions[0].params.reverse = true;
        let right = split(&mut p, t, 0, 2.0).unwrap();
        let (a, b) = (&p.tracks[t].regions[0], &p.tracks[t].regions[right]);
        // A second in, backwards: the first part plays the last second of the file.
        assert_eq!((a.params.source_start, a.params.source_len), (7.0, 1.0));
        assert_eq!((b.params.source_start, b.params.source_len), (0.0, 7.0));
    }

    #[test]
    fn splitting_a_generated_region_keeps_its_notes_as_plain_ones() {
        let (mut p, t) = midi_project();
        let r = p.add_generated_region(t, false, 1.0).unwrap();
        p.refresh_generated();
        let right = split(&mut p, t, r, 8.0).unwrap();
        assert!(p.tracks[t].regions[r].generated.is_none());
        assert!(p.tracks[t].regions[right].generated.is_none());
    }

    #[test]
    fn paste_replace_takes_away_what_was_under_the_paste() {
        let mut p = Project::empty();
        let t = p.add_track(TrackKind::Instrument);
        let long = p.add_midi_region(t, 1.0).unwrap_or(0);
        p.tracks[t].regions[long].len_bars = 4.0;
        p.tracks[t].regions[long].notes = vec![note(0.0, 1.0, 60), note(9.0, 1.0, 62)];
        let short = p.add_midi_region(t, 9.0).unwrap_or(1);
        p.tracks[t].regions[short].len_bars = 1.0;
        let mut v = TracksView::default();
        v.select_only((t, short));
        copy(&p, &mut v).unwrap_or_default();
        p.selected_track = t;
        p.position_beats = p.bar_to_beats(2.0);
        assert!(paste_replace(&mut p, &mut v).is_ok());
        let mut bars: Vec<(i32, i32)> = p.tracks[t]
            .regions
            .iter()
            .map(|r| (r.start_bar as i32, (r.start_bar + r.len_bars) as i32))
            .collect();
        bars.sort_unstable();
        assert_eq!(bars, [(1, 3), (3, 4), (4, 5), (9, 10)]);
        assert_eq!(v.all_selected().len(), 1, "what was pasted stays selected");
    }

    #[test]
    fn functions_quantize_and_move_what_is_selected() {
        let mut p = Project::empty();
        let t = p.add_track(TrackKind::Instrument);
        let r = p.add_midi_region(t, 3.0).unwrap_or(0);
        p.tracks[t].regions[r].notes = vec![note(0.3, 0.5, 60)];
        let mut v = TracksView::default();
        assert_eq!(quantize_selected(&mut p, &v, 0.25), 0, "nothing selected");
        v.select_only((t, r));
        assert_eq!(quantize_selected(&mut p, &v, 0.25), 1);
        assert!((p.tracks[t].regions[r].notes[0].start - 0.25).abs() < 1e-6);
        p.position_beats = 0.0;
        move_selected_to_playhead(&mut p, &v);
        assert!((p.tracks[t].regions[r].start_bar - 1.0).abs() < 1e-6);
    }

    #[test]
    fn midi_regions_join_with_their_notes_in_place() {
        let (mut p, t) = midi_project();
        p.tracks[t].regions[0].len_bars = 1.0;
        p.tracks[t].regions[0].notes = vec![note(1.0, 1.0, 60)];
        let second = p.add_midi_region(t, 3.0).unwrap();
        p.tracks[t].regions[second].len_bars = 1.0;
        p.tracks[t].regions[second].notes = vec![note(1.0, 1.0, 62)];
        let joined = join_with_next(&mut p, t, 0).unwrap();
        let region = &p.tracks[t].regions[joined];
        assert_eq!(p.tracks[t].regions.len(), 1);
        assert_eq!((region.start_bar, region.len_bars), (1.0, 3.0));
        assert_eq!(region.notes, [note(1.0, 1.0, 60), note(9.0, 1.0, 62)]);
        assert!(join_with_next(&mut p, t, 0).is_err(), "nothing after it");
    }

    #[test]
    fn a_split_joins_back_and_other_audio_does_not() {
        let (mut p, t, _dir, _wav) = audio_project(8.0);
        let right = split(&mut p, t, 0, 8.0).unwrap();
        let joined = join_with_next(&mut p, t, 0).unwrap();
        assert_eq!(p.tracks[t].regions.len(), 1);
        let region = &p.tracks[t].regions[joined];
        assert_eq!(
            (region.params.source_start, region.params.source_len),
            (0.0, 8.0)
        );
        assert!((region.len_bars - 4.0).abs() < 0.01);

        // Parts moved apart, or out of order, need Bounce.
        let right2 = split(&mut p, t, 0, 8.0).unwrap();
        assert_eq!(right, right2);
        p.tracks[t].regions[right2].start_bar = 4.0;
        assert!(join_with_next(&mut p, t, 0).unwrap_err().contains("Bounce"));
        assert_eq!(p.tracks[t].regions.len(), 2);
    }

    #[test]
    fn trimming_the_end_cuts_what_plays_and_stops_at_the_end_of_the_file() {
        let (mut p, t, _dir, _wav) = audio_project(8.0);
        trim_end(&mut p, t, 0, 4.0).unwrap();
        let region = &p.tracks[t].regions[0];
        assert_eq!((region.len_bars, region.params.source_len), (1.0, 2.0));
        trim_end(&mut p, t, 0, 400.0).unwrap();
        let region = &p.tracks[t].regions[0];
        assert!((region.len_bars - 4.0).abs() < 0.01, "{}", region.len_bars);
        assert_eq!(region.params.source_len, 8.0);

        let (mut p, t) = midi_project();
        trim_end(&mut p, t, 0, 400.0).unwrap();
        assert_eq!(
            p.tracks[t].regions[0].len_bars, 100.0,
            "MIDI has no such limit"
        );
    }

    #[test]
    fn trimming_the_start_leaves_the_sound_where_it_was() {
        let (mut p, t, _dir, _wav) = audio_project(8.0);
        let from = TrimStart::of(&p, t, 0).unwrap();
        trim_start(&mut p, t, 0, &from, 4.0);
        let region = &p.tracks[t].regions[0];
        assert_eq!((region.start_bar, region.len_bars), (2.0, 3.0));
        assert_eq!(
            (region.params.source_start, region.params.source_len),
            (2.0, 6.0)
        );
        // Back again within the same drag, and not before the file begins.
        trim_start(&mut p, t, 0, &from, 0.0);
        let region = &p.tracks[t].regions[0];
        assert_eq!((region.start_bar, region.params.source_start), (1.0, 0.0));

        let (mut p, t) = midi_project();
        p.tracks[t].regions[0].notes = vec![note(1.0, 2.0, 60), note(5.0, 1.0, 62)];
        let from = TrimStart::of(&p, t, 0).unwrap();
        trim_start(&mut p, t, 0, &from, 2.0);
        assert_eq!(
            p.tracks[t].regions[0].notes,
            [note(0.0, 1.0, 60), note(3.0, 1.0, 62)]
        );
        trim_start(&mut p, t, 0, &from, 0.0);
        assert_eq!(
            p.tracks[t].regions[0].notes[0],
            note(1.0, 2.0, 60),
            "and back"
        );
    }

    #[test]
    fn copy_and_paste_goes_to_the_playhead_on_a_track_that_can_hold_it() {
        let (mut p, keys) = midi_project();
        let audio = p.add_track(TrackKind::Audio);
        let mut v = TracksView::default();
        assert!(paste(&mut p, &mut v).is_err(), "nothing copied");
        assert!(copy(&p, &mut v).is_err(), "nothing selected");
        v.selected = Some((keys, 0));
        copy(&p, &mut v).unwrap();
        p.position_beats = 8.0;
        // The selected track holds audio, so it goes back where it came from.
        p.selected_track = audio;
        let (t, r) = paste(&mut p, &mut v).unwrap();
        assert_eq!(t, keys);
        assert_eq!(p.tracks[keys].regions[r].start_bar, 3.0);
        assert_eq!(v.all_selected(), [(keys, r)]);
    }

    #[test]
    fn split_at_playhead_takes_the_selected_region_or_what_is_under_it() {
        let (mut p, t) = midi_project();
        let mut v = TracksView::default();
        p.selected_track = t;
        p.position_beats = 100.0;
        assert!(!can_split_at_playhead(&p, &v));
        assert!(split_at_playhead(&mut p, &mut v).is_err());
        p.position_beats = 8.0;
        assert!(can_split_at_playhead(&p, &v));
        split_at_playhead(&mut p, &mut v).unwrap();
        assert_eq!(p.tracks[t].regions.len(), 2);
        assert_eq!(v.selected, Some((t, 1)), "the second part, ready to move");
    }

    #[test]
    fn a_group_copies_splits_and_pastes_together() {
        let (mut p, a) = midi_project();
        let b = p.add_track(TrackKind::Instrument);
        p.add_midi_region(b, 2.0).unwrap();
        let mut v = TracksView::default();
        v.select_many(vec![(a, 0), (b, 0)]);

        // Split at the playhead, in both.
        p.position_beats = 8.0;
        split_at_playhead(&mut p, &mut v).unwrap();
        assert_eq!(
            (p.tracks[a].regions.len(), p.tracks[b].regions.len()),
            (2, 2)
        );
        assert_eq!(v.all_selected().len(), 2, "the second parts");

        // Copy the first parts; paste keeps their distances.
        v.select_many(vec![(a, 0), (b, 0)]);
        copy(&p, &mut v).unwrap();
        p.position_beats = 40.0;
        p.selected_track = a;
        paste(&mut p, &mut v).unwrap();
        assert_eq!(p.tracks[a].regions.last().unwrap().start_bar, 11.0);
        assert_eq!(
            p.tracks[b].regions.last().unwrap().start_bar,
            12.0,
            "a bar later"
        );
        assert_eq!(v.all_selected().len(), 2, "the copies");
    }

    #[test]
    fn several_selected_regions_join_into_one() {
        let (mut p, t) = midi_project();
        p.tracks[t].regions[0].len_bars = 1.0;
        for bar in [2.0, 3.0] {
            let r = p.add_midi_region(t, bar).unwrap();
            p.tracks[t].regions[r].len_bars = 1.0;
        }
        let mut v = TracksView::default();
        v.select_many(vec![(t, 2), (t, 0), (t, 1)]);
        join_selected(&mut p, &mut v).unwrap();
        assert_eq!(p.tracks[t].regions.len(), 1);
        assert_eq!(p.tracks[t].regions[0].len_bars, 3.0);
        assert_eq!(v.all_selected(), [(t, 0)]);
    }
}
