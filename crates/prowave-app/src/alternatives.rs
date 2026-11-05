//! Track alternatives: other versions of a track's regions, one playing at a time, switched from
//! the track's header and saved in the project. The regions of the version playing are in
//! `Track::regions`, so everything else in the program works on them as it always did; the others
//! wait in `Track::alternatives`.
use crate::model::{Project, Track, TrackAlternative};

/// An entry of a track's Alternatives menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AltCmd {
    Switch(usize),
    /// A new version with nothing on it.
    New,
    /// A new version with a copy of what is on the track now.
    Duplicate,
    /// Deletes the version playing, and its regions.
    Delete,
}

/// The first of A, B, C... not taken, then numbers.
fn next_name(t: &Track) -> String {
    let free = |name: &String| !t.alternatives.iter().any(|a| a.name == *name);
    ('A'..='Z')
        .map(String::from)
        .find(&free)
        .or_else(|| (1..).map(|n| format!("V{n}")).find(&free))
        .unwrap_or_default()
}

/// Makes the list if it was only ever one version.
fn make_list(t: &mut Track) {
    if t.alternatives.is_empty() {
        t.alternatives.push(TrackAlternative {
            name: "A".to_string(),
            regions: Vec::new(),
        });
        t.active_alt = 0;
    }
}

/// The letter of the version playing.
#[cfg(test)]
pub fn button_label(t: &Track) -> String {
    t.alternatives
        .get(t.active_alt)
        .and_then(|a| a.name.chars().next())
        .map_or_else(|| "A".to_string(), String::from)
}

/// Carries out `cmd` on track `t`. Not during a recording, which is writing into a region of the
/// version playing, and not on a frozen track, whose file was made from that version.
pub fn run(p: &mut Project, t: usize, cmd: AltCmd) -> Result<(), String> {
    if p.recording {
        return Err("Track alternatives cannot change during a recording".to_string());
    }
    let track = p
        .tracks
        .get_mut(t)
        .ok_or_else(|| "That track is gone".to_string())?;
    if !track.kind.has_regions() {
        return Err("This kind of track has no regions".to_string());
    }
    if track.frozen.is_some() {
        return Err("Unfreeze the track to change its alternative".to_string());
    }
    make_list(track);
    match cmd {
        AltCmd::Switch(i) => {
            if i == track.active_alt || i >= track.alternatives.len() {
                return Ok(());
            }
            let now = std::mem::take(&mut track.regions);
            track.alternatives[track.active_alt].regions = now;
            track.regions = std::mem::take(&mut track.alternatives[i].regions);
            track.active_alt = i;
        }
        AltCmd::New | AltCmd::Duplicate => {
            let name = next_name(track);
            let regions = if cmd == AltCmd::New {
                Vec::new()
            } else {
                track.regions.clone()
            };
            let now = std::mem::replace(&mut track.regions, regions);
            track.alternatives[track.active_alt].regions = now;
            track.alternatives.push(TrackAlternative {
                name,
                regions: Vec::new(),
            });
            track.active_alt = track.alternatives.len() - 1;
        }
        AltCmd::Delete => {
            if track.alternatives.len() < 2 {
                return Err("A track keeps at least one version".to_string());
            }
            track.alternatives.remove(track.active_alt);
            let next = track.active_alt.min(track.alternatives.len() - 1);
            track.regions = std::mem::take(&mut track.alternatives[next].regions);
            track.active_alt = next;
        }
    }
    p.song_rev += 1;
    // Clips of a pattern that changed while their version was not playing.
    crate::song_pattern::sync(p);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TrackKind;

    fn project() -> (Project, usize) {
        let mut p = Project::empty();
        let t = p.add_track(TrackKind::Instrument);
        p.add_midi_region(t, 1.0);
        p.add_midi_region(t, 5.0);
        (p, t)
    }

    fn bars(p: &Project, t: usize) -> Vec<f32> {
        p.tracks[t].regions.iter().map(|r| r.start_bar).collect()
    }

    #[test]
    fn a_new_version_starts_empty_and_switching_brings_the_regions_back() {
        let (mut p, t) = project();
        let rev = p.song_rev;
        run(&mut p, t, AltCmd::New).unwrap_or_default();
        assert!(p.song_rev > rev, "the engine hears about it");
        assert!(p.tracks[t].regions.is_empty());
        assert_eq!(p.tracks[t].active_alt, 1);
        assert_eq!(p.tracks[t].alternatives[1].name, "B");
        assert_eq!(button_label(&p.tracks[t]), "B");
        p.add_midi_region(t, 9.0);
        run(&mut p, t, AltCmd::Switch(0)).unwrap_or_default();
        assert_eq!(bars(&p, t), [1.0, 5.0]);
        assert_eq!(button_label(&p.tracks[t]), "A");
        assert!(p.tracks[t].alternatives[0].regions.is_empty());
        assert_eq!(p.tracks[t].alternatives[1].regions.len(), 1);
        run(&mut p, t, AltCmd::Switch(1)).unwrap_or_default();
        assert_eq!(bars(&p, t), [9.0]);
        // Nothing happens for a version that is not there.
        run(&mut p, t, AltCmd::Switch(7)).unwrap_or_default();
        assert_eq!(p.tracks[t].active_alt, 1);
    }

    #[test]
    fn a_duplicate_is_a_copy_that_goes_its_own_way() {
        let (mut p, t) = project();
        run(&mut p, t, AltCmd::Duplicate).unwrap_or_default();
        assert_eq!(bars(&p, t), [1.0, 5.0]);
        p.tracks[t].regions.remove(0);
        run(&mut p, t, AltCmd::Switch(0)).unwrap_or_default();
        assert_eq!(bars(&p, t), [1.0, 5.0], "the first is as it was");
    }

    #[test]
    fn deleting_a_version_plays_the_next_and_the_last_one_stays() {
        let (mut p, t) = project();
        assert!(run(&mut p, t, AltCmd::Delete).is_err());
        run(&mut p, t, AltCmd::New).unwrap_or_default();
        p.add_midi_region(t, 9.0);
        run(&mut p, t, AltCmd::Delete).unwrap_or_default();
        assert_eq!(bars(&p, t), [1.0, 5.0]);
        assert_eq!(p.tracks[t].alternatives.len(), 1);
        assert_eq!(p.tracks[t].active_alt, 0);
    }

    #[test]
    fn nothing_changes_while_recording_on_a_frozen_track_or_a_track_without_regions() {
        let (mut p, t) = project();
        p.recording = true;
        assert!(run(&mut p, t, AltCmd::New).is_err());
        p.recording = false;
        p.tracks[t].frozen = Some(Box::new(p.tracks[t].regions[0].clone()));
        assert!(run(&mut p, t, AltCmd::New).is_err());
        assert_eq!(bars(&p, t), [1.0, 5.0]);
        let aux = p.add_track(TrackKind::Aux);
        assert!(run(&mut p, aux, AltCmd::New).is_err());
        assert!(run(&mut p, 99, AltCmd::New).is_err());
    }

    #[test]
    fn names_are_letters_then_numbers_and_never_taken() {
        let (mut p, t) = project();
        for _ in 0..27 {
            run(&mut p, t, AltCmd::New).unwrap_or_default();
        }
        let names: Vec<&str> = p.tracks[t]
            .alternatives
            .iter()
            .map(|a| a.name.as_str())
            .collect();
        assert_eq!(names[..3], ["A", "B", "C"]);
        assert_eq!(names[25], "Z");
        assert_eq!(names[26], "V1");
    }
}
