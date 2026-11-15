//! MIDI input per track (Logic: MIDI In Port and MIDI In Channel in the track's inspector). A track
//! can listen to one device, one channel, or both. While armed tracks listen to devices of their
//! own, each armed track plays and records only what it hears: keys on one track and a drum pad on
//! another, together. With no armed track set that way, every note goes to the record track as
//! before.

use crate::midi_in::NoteFrom;
use crate::model::{Project, Track, TrackKind};
use crate::piano_roll::Audition;
use crate::recorder::LiveNote;

/// Whether `t` hears a note from `from`. Notes typed on the computer keyboard go to tracks that
/// listen to every device.
pub fn hears(t: &Track, from: &NoteFrom) -> bool {
    match from {
        None => t.midi_in_port.is_none(),
        Some((port, channel)) => {
            t.midi_in_port.as_deref().is_none_or(|p| p == &**port)
                && (t.midi_in_channel == 0 || t.midi_in_channel == *channel)
        }
    }
}

fn plays_midi(t: &Track) -> bool {
    matches!(t.kind, TrackKind::Instrument | TrackKind::ExternalMidi)
}

/// The armed MIDI tracks, in order, each with what it hears of this frame's notes and when they
/// arrived. `None` when no armed track listens to a device or channel of its own: then every
/// note goes the usual way.
pub fn split(
    p: &Project,
    input: &[LiveNote],
    ages: &[f64],
    from: &[NoteFrom],
) -> Option<Vec<(usize, Vec<LiveNote>, Vec<f64>)>> {
    let armed: Vec<usize> = (0..p.tracks.len())
        .filter(|&t| p.tracks[t].rec && plays_midi(&p.tracks[t]))
        .collect();
    let own = armed.iter().any(|&t| {
        let t = &p.tracks[t];
        t.midi_in_port.is_some() || t.midi_in_channel != 0
    });
    if !own {
        return None;
    }
    let tracks = armed
        .into_iter()
        .map(|t| {
            let mut notes = Vec::new();
            let mut at = Vec::new();
            for (i, note) in input.iter().enumerate() {
                let heard = from.get(i).is_none_or(|f| hears(&p.tracks[t], f));
                if heard {
                    notes.push(*note);
                    at.push(ages.get(i).copied().unwrap_or(0.0));
                }
            }
            (t, notes, at)
        })
        .collect();
    Some(tracks)
}

/// Plays `notes` on track `t`.
pub fn audition(out: &mut Vec<Audition>, t: usize, notes: &[LiveNote]) {
    let track = t.min(usize::from(u16::MAX)) as u16;
    out.extend(notes.iter().map(|note| match *note {
        LiveNote::On { pitch, vel } => Audition::On { pitch, vel, track },
        LiveNote::Off { pitch } => Audition::Off { pitch },
        LiveNote::Sustain { val } => Audition::Control {
            track,
            number: 64,
            value: val,
        },
    }));
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn a_track_hears_its_own_device_and_channel() {
        let mut t = Track::new("Keys", TrackKind::Instrument);
        let keys: NoteFrom = Some((Arc::from("Keys"), 1));
        let pad: NoteFrom = Some((Arc::from("Pad"), 10));
        assert!(hears(&t, &keys) && hears(&t, &pad) && hears(&t, &None));
        t.midi_in_port = Some("Keys".into());
        assert!(hears(&t, &keys) && !hears(&t, &pad) && !hears(&t, &None));
        t.midi_in_port = None;
        t.midi_in_channel = 10;
        assert!(!hears(&t, &keys) && hears(&t, &pad));
    }
}
