//! Take folders, as in Logic: recording with the Cycle on makes a new take on every pass, and
//! the passes are kept together in one region. A comp says which take plays in each stretch
//! (the last take everywhere to begin with); the notes of the comp are copied into the
//! region's own `notes`, so the engine, the Piano Roll and the Event list need no special case.

use crate::model::{CompSegment, Control, Note, Project, Region, Take, TakeFile, TakeFolder};
use ovrtone_engine::AudioFile;

const EPS: f32 = 1e-4;
/// Recordings that touch by less than this, in beats, are not over one another.
const EPS_BEATS: f64 = 1e-3;

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
pub fn audio_comp_clips<'a>(
    folder: &'a TakeFolder,
    map: &ovrtone_engine::TempoMap,
    origin: f64,
    base: ovrtone_engine::ClipEdit,
    main: &'a AudioFile,
) -> Vec<(f64, ovrtone_engine::ClipEdit, &'a AudioFile)> {
    let start_secs = map.seconds_at(origin);
    let last = folder.comp.len().saturating_sub(1);
    folder
        .comp
        .iter()
        .enumerate()
        .filter_map(|(i, seg)| {
            let take = folder.takes.get(seg.take)?;
            let offset = f64::from(take.audio_offset?);
            let file = take.file.as_ref().map_or(main, |f| &f.0.file);
            let (first, last_second) = take.window.map_or((0.0, file.duration_secs()), |w| {
                (f64::from(w.0), f64::from(w.1).min(file.duration_secs()))
            });
            let from = map.seconds_at(origin + f64::from(seg.start)) - start_secs;
            let to = map.seconds_at(origin + f64::from(seg.end)) - start_secs;
            // The recording may start after this stretch does, and end before it does.
            let source = (offset + from).max(first);
            let from = source - offset;
            let end = (offset + to).min(last_second);
            if end - source <= 0.0 {
                return None;
            }
            let mut edit = base;
            edit.gain_db = base.gain_db + take.gain_db;
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
            Some((map.beat_at(start_secs + from), edit, file))
        })
        .collect()
}

/// Whether a region can go in a take folder with what was recorded over it. One with edits a take
/// cannot hold (Flex, reverse, automation of its own, a clip of a pattern) stays as it is, and so
/// does a muted one, which the user has already set aside.
fn foldable(r: &Region, audio: bool) -> bool {
    r.audio.is_some() == audio
        && r.params.pattern == 0
        && r.generated.is_none()
        && !r.params.mute
        && !r.params.reverse
        && r.auto.is_empty()
        && r.audio
            .as_ref()
            .is_none_or(|a| a.flex.is_none() && a.follow_bpm.is_none())
}

/// The takes of `region` when its start is `beats` beats and `seconds` seconds into a folder.
fn takes_of(region: &Region, beats: f64, seconds: f64) -> Vec<Take> {
    let shift_notes = |notes: &[Note]| -> Vec<Note> {
        notes
            .iter()
            .map(|n| Note {
                start: n.start + beats as f32,
                ..*n
            })
            .collect()
    };
    let shift_controls = |controls: &[Control]| -> Vec<Control> {
        controls
            .iter()
            .map(|c| Control {
                start: c.start + beats as f32,
                ..*c
            })
            .collect()
    };
    let file = region.audio.clone().map(TakeFile);
    if let Some(folder) = &region.folder {
        return folder
            .takes
            .iter()
            .map(|t| Take {
                notes: shift_notes(&t.notes),
                controls: shift_controls(&t.controls),
                audio_offset: t.audio_offset.map(|o| o - seconds as f32),
                file: t.file.clone().or_else(|| file.clone()),
                window: t.window,
                gain_db: t.gain_db + region.params.gain_db,
            })
            .collect();
    }
    let source = region.params.source_start;
    vec![Take {
        notes: shift_notes(&region.notes),
        controls: shift_controls(&region.controls),
        audio_offset: region.audio.as_ref().map(|_| source - seconds as f32),
        window: region
            .audio
            .as_ref()
            .map(|_| (source, source + crate::region_edit::played_seconds(region))),
        file,
        gain_db: region.params.gain_db,
    }]
}

/// Makes one take folder of region `r` of track `t` and the regions it was recorded over, with the
/// new one playing where it was recorded and the others where they were. Returns its number.
pub fn fold_over(p: &mut Project, t: usize, r: usize) -> usize {
    let new = &p.tracks[t].regions[r];
    let (from, to) = (p.region_start_beats(new), p.region_end_beats(new));
    let audio = new.audio.is_some();
    let mut under: Vec<usize> = p.tracks[t]
        .regions
        .iter()
        .enumerate()
        .filter(|(i, x)| {
            *i != r
                && foldable(x, audio)
                && p.region_start_beats(x) < to - EPS_BEATS
                && p.region_end_beats(x) > from + EPS_BEATS
        })
        .map(|(i, _)| i)
        .collect();
    if under.is_empty() {
        return r;
    }
    let regions = &p.tracks[t].regions;
    under.sort_by(|a, b| regions[*a].start_bar.total_cmp(&regions[*b].start_bar));
    let start = under
        .iter()
        .map(|i| p.region_start_beats(&regions[*i]))
        .fold(from, f64::min);
    let end = under
        .iter()
        .map(|i| p.region_end_beats(&regions[*i]))
        .fold(to, f64::max);
    let map = p.tempo_map();
    let start_secs = map.seconds_at(start);
    let mut takes = Vec::new();
    // Each with where its own takes begin among them, and where it lies in the folder.
    let mut placed = Vec::with_capacity(under.len() + 1);
    for i in under.iter().copied().chain([r]) {
        let region = &regions[i];
        let beats = p.region_start_beats(region) - start;
        let seconds = map.seconds_at(p.region_start_beats(region)) - start_secs;
        placed.push((i, takes.len(), beats));
        takes.extend(takes_of(region, beats, seconds));
    }
    let mut folder = TakeFolder::new(takes, (end - start) as f32);
    for (i, first, beats) in placed {
        let region = &regions[i];
        match &region.folder {
            Some(own) => {
                for seg in &own.comp {
                    folder.swipe(
                        first + seg.take,
                        seg.start + beats as f32,
                        seg.end + beats as f32,
                    );
                }
            }
            None => {
                let len = p.region_len_beats(region) as f32;
                folder.swipe(first, beats as f32, beats as f32 + len);
            }
        }
    }
    let (first_fade, last_fade) = (
        regions[under[0]].params.fade_in,
        regions[under[under.len() - 1]].params.fade_out,
    );
    let mut merged = regions[r].clone();
    merged.start_bar = p.beats_to_bar(start) as f32 + 1.0;
    merged.len_bars = (p.beats_to_bar(end) - p.beats_to_bar(start)) as f32;
    merged.params.gain_db = 0.0;
    merged.params.source_start = 0.0;
    merged.params.source_len = 0.0;
    merged.params.fade_in = if start < from {
        first_fade
    } else {
        merged.params.fade_in
    };
    merged.params.fade_out = if end > to {
        last_fade
    } else {
        merged.params.fade_out
    };
    merged.params.crossfade = false;
    folder.apply(&mut merged.notes, &mut merged.controls);
    merged.folder = Some(folder);
    let mut gone = under;
    gone.push(r);
    gone.sort_unstable_by(|a, b| b.cmp(a));
    for i in gone {
        p.tracks[t].regions.remove(i);
    }
    p.tracks[t].regions.push(merged);
    p.song_rev += 1;
    p.tracks[t].regions.len() - 1
}

/// What a finished recording does to what lies under it: replace mode takes that away, else with
/// `fold` it becomes a take folder with it. Returns the region's number afterwards.
pub fn settle_recording(p: &mut Project, t: usize, r: usize, fold: bool) -> usize {
    if p.rec_replace {
        let (r, done) = crate::region_edit::replace_under(p, t, r);
        if let Err(e) = done {
            tracing::warn!("replace mode: {e}");
        }
        r
    } else if fold {
        fold_over(p, t, r)
    } else {
        r
    }
}

/// Makes a new track of the same kind holding one take, or the comp with `None`, as ordinary
/// regions in the same place. The folder is left as it was. Returns the new track's number.
pub fn copy_to_new_track(
    p: &mut Project,
    t: usize,
    r: usize,
    take: Option<usize>,
) -> Option<usize> {
    let source = p.tracks.get(t)?;
    let (kind, title) = (source.kind, source.name.clone());
    let mut region = source.regions.get(r)?.clone();
    let folder = region.folder.as_mut()?;
    folder.end_audition();
    folder.saved.clear();
    let suffix = match take {
        Some(take) => {
            let len = folder.len;
            folder.swipe(take, 0.0, len);
            format!("Take {}", take + 1)
        }
        None => "Comp".to_string(),
    };
    let folder_now = region.folder.clone()?;
    folder_now.apply(&mut region.notes, &mut region.controls);
    let new = p.add_track(kind);
    p.tracks[new].name = format!("{title} {suffix}");
    p.tracks[new].regions = vec![region];
    flatten(p, new, 0);
    Some(new)
}

/// An ordinary region again, or for a comp of audio one region per stretch. Returns their numbers.
pub fn flatten(p: &mut Project, t: usize, r: usize) -> Vec<usize> {
    if let Some(region) = p.tracks.get_mut(t).and_then(|x| x.regions.get_mut(r)) {
        if let Some(mut folder) = region.folder.take() {
            folder.end_audition();
            folder.apply(&mut region.notes, &mut region.controls);
            region.folder = Some(folder);
        }
    }
    let Some(region) = p.tracks.get(t).and_then(|x| x.regions.get(r)) else {
        return Vec::new();
    };
    let (Some(folder), Some(main)) = (region.folder.as_ref(), region.audio.as_ref()) else {
        // MIDI: the region's notes already are the comp.
        if let Some(region) = p.tracks[t].regions.get_mut(r) {
            region.folder = None;
        }
        p.song_rev += 1;
        return vec![r];
    };
    if !folder.is_audio() {
        p.tracks[t].regions[r].folder = None;
        p.song_rev += 1;
        return vec![r];
    }
    let map = p.tempo_map();
    let origin = p.region_start_beats(region);
    let edit = region.params.clip_edit();
    let mut made = Vec::new();
    for (at, clip, file) in audio_comp_clips(folder, &map, origin, edit, &main.file) {
        let source = folder
            .takes
            .iter()
            .filter_map(|take| take.file.as_ref())
            .find(|f| f.0.file.path == file.path)
            .map_or_else(|| main.clone(), |f| f.0.clone());
        let end = map.beat_at(map.seconds_at(at) + clip.length_secs);
        let mut part = region.clone();
        part.folder = None;
        part.audio = Some(source);
        part.notes.clear();
        part.controls.clear();
        part.start_bar = p.beats_to_bar(at) as f32 + 1.0;
        part.len_bars = (p.beats_to_bar(end) - p.beats_to_bar(at)) as f32;
        part.params.source_start = clip.source_start_secs as f32;
        part.params.source_len = clip.length_secs as f32;
        part.params.gain_db = clip.gain_db;
        part.params.fade_in = clip.fade_in_beats as f32;
        part.params.fade_out = clip.fade_out_beats as f32;
        part.params.crossfade = false;
        made.push(part);
    }
    let track = &mut p.tracks[t];
    track.regions.remove(r);
    let first = track.regions.len();
    track.regions.extend(made);
    p.song_rev += 1;
    (first..track.regions.len()).collect()
}

/// The take that plays at the playhead in the folder of region `r`, else the last one.
pub fn take_at_playhead(p: &Project, t: usize, r: usize) -> Option<usize> {
    let region = p.tracks.get(t)?.regions.get(r)?;
    let folder = region.folder.as_ref()?;
    let at = (p.position_beats - p.region_start_beats(region)) as f32;
    folder
        .comp
        .iter()
        .find(|s| at >= s.start && at < s.end)
        .map(|s| s.take)
        .or_else(|| folder.takes.len().checked_sub(1))
}

/// Record > Delete Take: takes `take` out of the folder of region `r`. With one take left the folder
/// has done its work and becomes an ordinary region. Returns the regions it leaves.
pub fn delete_take(p: &mut Project, t: usize, r: usize, take: usize) -> Vec<usize> {
    let Some(region) = p.tracks.get_mut(t).and_then(|x| x.regions.get_mut(r)) else {
        return Vec::new();
    };
    let Some(folder) = region.folder.as_mut() else {
        return Vec::new();
    };
    folder.remove_take(take);
    if !folder.is_audio() {
        let (notes, controls) = folder.flatten();
        region.notes = notes;
        region.controls = controls;
    }
    let left = folder.takes.len();
    p.song_rev += 1;
    if left <= 1 {
        flatten(p, t, r)
    } else {
        vec![r]
    }
}

/// What Record Repeat and New Take asked for, run at the start of the next frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecCmd {
    /// Throw the recording in progress away and record again from where it began.
    Repeat,
    /// Throw the recording in progress away, stop, and go back to where it began.
    Discard,
    /// Keep the recording in progress as a take and record another over the same stretch. Stopped:
    /// record a new take over the selected region.
    NewTake,
}

/// Going back to record again: the beat, and whether the playhead has been sent there yet.
pub type Restart = (f64, bool);

/// Carries out Record Repeat and New Take. Call once per frame, before the engine is told about the
/// frame: playback stops, the recorders finish in that frame, the playhead goes back, and recording
/// starts again as it would from stopped, with its count-in.
pub fn record_commands(p: &mut Project, s: &mut crate::UiState) {
    let began = s.rec.began_at().or(s.audio_rec.began_at());
    let was_going_back = s.rec_restart.is_some();
    match s.rec_cmd.take() {
        Some(RecCmd::Repeat) => match began {
            Some(beat) => {
                crate::recorder::discard(&mut s.rec, p);
                s.audio_rec.discard(p);
                p.playing = false;
                s.rec_restart = Some((beat, false));
            }
            None => {
                p.recording = true;
                p.playing = true;
            }
        },
        Some(RecCmd::Discard) => {
            if let Some(beat) = began {
                crate::recorder::discard(&mut s.rec, p);
                s.audio_rec.discard(p);
                p.playing = false;
                p.recording = false;
                p.position_beats = beat;
            }
        }
        Some(RecCmd::NewTake) => {
            s.rec.as_take = true;
            if let Some(beat) = began {
                // The recorders see playback stop and finish; the next take follows.
                p.playing = false;
                s.rec_restart = Some((beat, false));
            } else if let Some((t, r)) = s.tracks.selected {
                let start = p
                    .tracks
                    .get(t)
                    .and_then(|x| x.regions.get(r))
                    .map(|region| p.region_start_beats(region));
                if let Some(start) = start {
                    p.selected_track = t;
                    p.playing = false;
                    s.rec_restart = Some((start, false));
                }
            }
        }
        None => {}
    }
    if s.rec_restart.is_some() && !was_going_back {
        // This frame the recorders finish, where the song is now.
        return;
    }
    match s.rec_restart {
        Some((beat, false)) => {
            p.position_beats = beat;
            s.rec_restart = Some((beat, true));
        }
        Some((_, true)) => {
            s.rec_restart = None;
            p.recording = true;
            p.playing = true;
        }
        None => {
            if !p.recording && !s.rec.active() {
                s.rec.as_take = false;
            }
        }
    }
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
            saved: Vec::new(),
            audition: None,
        }
    }

    /// The comp, whatever is being listened to just now.
    pub fn real_comp(&self) -> &[CompSegment] {
        self.audition.as_ref().map_or(&self.comp, |(_, comp)| comp)
    }

    /// Goes back to the comp, if a take was being listened to.
    pub fn end_audition(&mut self) {
        if let Some((_, comp)) = self.audition.take() {
            self.comp = comp;
        }
    }

    /// Plays `take` throughout to hear it alone; the same take again goes back to the comp.
    pub fn toggle_audition(&mut self, take: usize) {
        if take >= self.takes.len() {
            return;
        }
        let same = self.audition.as_ref().is_some_and(|(t, _)| *t == take);
        self.end_audition();
        if !same {
            let comp = std::mem::replace(
                &mut self.comp,
                vec![CompSegment {
                    start: 0.0,
                    end: self.len,
                    take,
                }],
            );
            self.audition = Some((take, comp));
        }
    }

    /// Keeps the comp as it is under a new name.
    pub fn save_comp(&mut self) {
        let mut n = self.saved.len() + 1;
        while self.saved.iter().any(|c| c.name == format!("Comp {n}")) {
            n += 1;
        }
        self.saved.push(crate::model::SavedComp {
            name: format!("Comp {n}"),
            comp: self.real_comp().to_vec(),
        });
    }

    /// Plays the comp kept as number `index`.
    pub fn use_comp(&mut self, index: usize) {
        self.end_audition();
        if let Some(saved) = self.saved.get(index) {
            self.comp = saved.comp.clone();
        }
    }

    /// Moves the join between stretch `index` and the next to `beat`, each keeping some length.
    pub fn move_join(&mut self, index: usize, beat: f32) {
        const SHORTEST: f32 = 1.0 / 64.0;
        if index + 1 >= self.comp.len() {
            return;
        }
        let (low, high) = (
            self.comp[index].start + SHORTEST,
            self.comp[index + 1].end - SHORTEST,
        );
        if low <= high {
            let at = beat.clamp(low, high);
            self.comp[index].end = at;
            self.comp[index + 1].start = at;
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

    /// Takes a take out: where it played, the latest of the others plays.
    pub fn remove_take(&mut self, take: usize) {
        if take >= self.takes.len() || self.takes.len() < 2 {
            return;
        }
        self.end_audition();
        let instead = if take == self.takes.len() - 1 {
            take - 1
        } else {
            self.takes.len() - 1
        };
        let without = |comp: &mut Vec<CompSegment>| {
            for seg in comp.iter_mut() {
                if seg.take == take {
                    seg.take = instead;
                }
                if seg.take > take {
                    seg.take -= 1;
                }
            }
            let mut merged: Vec<CompSegment> = Vec::new();
            for seg in std::mem::take(comp) {
                match merged.last_mut() {
                    Some(prev) if prev.take == seg.take => prev.end = seg.end,
                    _ => merged.push(seg),
                }
            }
            *comp = merged;
        };
        for saved in &mut self.saved {
            without(&mut saved.comp);
        }
        self.takes.remove(take);
        let mut merged = std::mem::take(&mut self.comp);
        without(&mut merged);
        self.comp = merged;
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
        let base = ovrtone_engine::ClipEdit::default();
        let main = audio_file("main.wav", 5.25);
        let clips = audio_comp_clips(&folder, &map, 8.0, base, &main);
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

    /// A file that is never opened, of `seconds` at 48 kHz.
    fn audio_file(name: &str, seconds: f64) -> AudioFile {
        AudioFile {
            path: name.into(),
            sample_rate: 48_000,
            frames: (seconds * 48_000.0) as u64,
            channels: 2,
        }
    }

    #[test]
    fn a_take_from_another_recording_reads_its_own_file_within_its_window_at_its_gain() {
        let map = {
            let mut p = crate::model::Project::empty();
            p.tempo = 120.0;
            p.tempo_map()
        };
        let own = crate::model::AudioRegion {
            file: audio_file("own.wav", 10.0),
            peaks: std::sync::Arc::default(),
            fine_peaks: std::sync::Arc::default(),
            peak_top: std::sync::Arc::default(),
            flex: None,
            follow_bpm: None,
        };
        let folder = TakeFolder::new(
            vec![
                Take {
                    audio_offset: Some(0.0),
                    ..Take::default()
                },
                Take {
                    audio_offset: Some(2.0),
                    file: Some(TakeFile(own)),
                    window: Some((3.0, 4.0)),
                    gain_db: -6.0,
                    ..Take::default()
                },
            ],
            8.0,
        );
        let main = audio_file("main.wav", 4.0);
        let clips = audio_comp_clips(
            &folder,
            &map,
            0.0,
            ovrtone_engine::ClipEdit {
                gain_db: 1.0,
                ..Default::default()
            },
            &main,
        );
        assert_eq!(clips.len(), 1);
        let (at, edit, file) = clips[0];
        assert_eq!(file.path, std::path::PathBuf::from("own.wav"));
        assert_eq!(edit.gain_db, -5.0);
        // Its window begins a second in: two beats after the folder does.
        assert_eq!(at, 2.0);
        assert_eq!((edit.source_start_secs, edit.length_secs), (3.0, 1.0));
    }

    #[test]
    fn deleting_a_take_gives_its_stretches_to_the_latest_and_renumbers_the_rest() {
        let take = |pitch: u8| Take {
            notes: vec![Note { pitch, ..note(0.0) }],
            ..Take::default()
        };
        let mut f = TakeFolder::new(vec![take(1), take(2), take(3)], 8.0);
        f.swipe(0, 0.0, 2.0);
        f.swipe(1, 2.0, 4.0);
        // Plays 0, 1, 2. Without take 1 its stretch goes to the last, which joins up.
        f.remove_take(1);
        assert_eq!(f.takes.len(), 2);
        assert_eq!(
            f.comp
                .iter()
                .map(|s| (s.start, s.end, s.take))
                .collect::<Vec<_>>(),
            [(0.0, 2.0, 0), (2.0, 8.0, 1)]
        );
        // Without the last, the one before it takes over.
        f.remove_take(1);
        assert_eq!(f.takes[0].notes[0].pitch, 1);
        assert_eq!(f.comp.len(), 1);
        assert_eq!(f.comp[0].take, 0);
        // The last take cannot go: Delete Take makes the region ordinary before that.
        f.remove_take(0);
        assert_eq!(f.takes.len(), 1);
    }

    #[test]
    fn delete_take_down_to_one_makes_an_ordinary_region_that_plays_that_take() {
        let mut p = crate::model::Project::empty();
        p.tempo = 120.0;
        let main = crate::model::AudioRegion {
            file: audio_file("main.wav", 4.0),
            peaks: std::sync::Arc::default(),
            fine_peaks: std::sync::Arc::default(),
            peak_top: std::sync::Arc::default(),
            flex: None,
            follow_bpm: None,
        };
        let t = p.add_track(crate::model::TrackKind::Instrument);
        let r = p.add_midi_region(t, 2.0).unwrap_or(0);
        let region = &mut p.tracks[t].regions[r];
        region.len_bars = 1.0;
        region.audio = Some(main);
        // Two passes in the file, a bar each; the second starts two seconds in.
        region.folder = Some(TakeFolder::new(
            vec![
                Take {
                    audio_offset: Some(0.0),
                    ..Take::default()
                },
                Take {
                    audio_offset: Some(2.0),
                    ..Take::default()
                },
            ],
            4.0,
        ));
        assert_eq!(take_at_playhead(&p, t, r), Some(1));
        let left = delete_take(&mut p, t, r, 0);
        assert_eq!(left.len(), 1);
        let region = &p.tracks[t].regions[left[0]];
        assert!(region.folder.is_none());
        assert_eq!(region.start_bar, 2.0);
        assert_eq!(region.len_bars, 1.0);
        assert_eq!(region.params.source_start, 2.0);
        assert_eq!(region.params.source_len, 2.0);
    }

    #[test]
    fn record_repeat_stops_goes_back_and_records_again() {
        let mut p = crate::model::Project::empty();
        let mut s = crate::UiState::new();
        // Stopped: it just starts.
        s.rec_cmd = Some(RecCmd::Repeat);
        record_commands(&mut p, &mut s);
        assert!(p.recording && p.playing);
        // While recording: stop and throw away, back to where it began, then on again.
        p.position_beats = 8.0;
        assert!(matches!(
            crate::recorder::update(&mut s.rec, &mut p, &[]),
            crate::recorder::Outcome::Started(..)
        ));
        p.position_beats = 12.0;
        s.rec_cmd = Some(RecCmd::Repeat);
        record_commands(&mut p, &mut s);
        assert!(!p.playing, "it stops");
        assert!(!s.rec.active());
        assert!(p.tracks[0].regions.is_empty(), "what was recorded is gone");
        record_commands(&mut p, &mut s);
        assert_eq!(p.position_beats, 8.0, "back where it began");
        assert!(!p.playing, "the engine moves the playhead first");
        record_commands(&mut p, &mut s);
        assert!(p.recording && p.playing);
        assert_eq!(s.rec_restart, None);
    }

    #[test]
    fn new_take_keeps_what_was_recorded_and_goes_again_over_the_same_stretch() {
        let mut p = crate::model::Project::empty();
        let mut s = crate::UiState::new();
        p.recording = true;
        p.playing = true;
        p.position_beats = 4.0;
        crate::recorder::update(&mut s.rec, &mut p, &[]);
        p.position_beats = 5.0;
        let played = [crate::recorder::LiveNote::On { pitch: 60, vel: 90 }];
        crate::recorder::update(&mut s.rec, &mut p, &played);
        s.rec_cmd = Some(RecCmd::NewTake);
        record_commands(&mut p, &mut s);
        assert!(!p.playing);
        assert!(s.rec.as_take);
        // The recorder sees playback stop, and keeps the take.
        crate::recorder::update(&mut s.rec, &mut p, &[]);
        assert_eq!(p.tracks[0].regions.len(), 1);
        record_commands(&mut p, &mut s);
        record_commands(&mut p, &mut s);
        assert_eq!(p.position_beats, 4.0);
        assert!(p.recording && p.playing);
        assert!(s.rec.as_take, "the next one folds with it");
        // Stopped with a region selected: from its start.
        let mut p = crate::model::Project::empty();
        let mut s = crate::UiState::new();
        let r = p.add_midi_region(0, 3.0).unwrap_or(0);
        s.tracks.select_only((0, r));
        s.rec_cmd = Some(RecCmd::NewTake);
        for _ in 0..3 {
            record_commands(&mut p, &mut s);
        }
        assert_eq!(p.position_beats, p.bar_to_beats(2.0));
        assert!(p.recording && p.playing);
    }

    fn note(start: f32) -> Note {
        Note {
            start,
            len: 0.5,
            pitch: 60,
            vel: 100,
            muted: false,
            color: 0,
            slide: false,
        }
    }

    fn folder() -> TakeFolder {
        // Take 0 plays at 0, 2, 5; take 1 at 1, 3, 6. Eight beats long.
        let take = |starts: &[f32]| Take {
            notes: starts.iter().map(|&s| note(s)).collect(),
            ..Take::default()
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
    fn a_take_can_be_heard_alone_and_the_comp_comes_back() {
        let mut f = folder();
        f.swipe(0, 1.0, 2.0);
        let comp = f.comp.clone();
        f.toggle_audition(1);
        assert_eq!(f.comp.len(), 1);
        assert_eq!(f.comp[0].take, 1);
        assert_eq!(
            f.real_comp(),
            comp.as_slice(),
            "it is the comp that is saved"
        );
        // Another take goes on to that one, and the same one goes back.
        f.toggle_audition(0);
        assert_eq!(f.comp[0].take, 0);
        assert_eq!(f.real_comp(), comp.as_slice());
        f.toggle_audition(0);
        assert_eq!(f.comp, comp);
        assert!(f.audition.is_none());
    }

    #[test]
    fn comps_are_kept_and_gone_back_to() {
        let mut f = folder();
        f.swipe(0, 1.0, 2.0);
        f.save_comp();
        let kept = f.comp.clone();
        f.swipe(1, 0.0, f.len);
        assert_ne!(f.comp, kept);
        f.save_comp();
        assert_eq!(f.saved[0].name, "Comp 1");
        assert_eq!(f.saved[1].name, "Comp 2");
        f.use_comp(0);
        assert_eq!(f.comp, kept);
        // Deleting a take keeps the kept comps right.
        let last = f.takes.len() - 1;
        f.remove_take(last);
        assert!(f.saved[0].comp.iter().all(|s| s.take < f.takes.len()));
    }

    #[test]
    fn a_join_moves_and_each_side_keeps_some_length() {
        let mut f = folder();
        f.swipe(0, 1.0, 2.0);
        f.move_join(0, 1.5);
        assert_eq!((f.comp[0].end, f.comp[1].start), (1.5, 1.5));
        f.move_join(0, -5.0);
        assert!(f.comp[0].end > f.comp[0].start);
        f.move_join(1, 1000.0);
        assert!(f.comp[1].end < f.comp[2].end);
        assert_eq!(f.comp[1].end, f.comp[2].start);
        f.move_join(9, 1.0);
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
