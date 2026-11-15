//! What a sample dropped on the Tracks area becomes: a hit a Channel Rack channel, a loop a clip
//! (see [`menu`]); with Alt held, the user chooses from:
//!
//! - an audio clip on the timeline, which follows the project's tempo if it is a loop;
//! - a new instrument track that plays it from the piano roll;
//! - a new instrument track with a step pattern that plays it;
//! - or the instrument of the track it was dropped on.
use crate::model::{Generated, Generator, Project, TrackKind};
use crate::pattern::{PatternParams, PatternRow};
use crate::plugin_rack::add_plugin;
use crate::UiState;
use ovrtone_host::{PluginFormat, PluginInfo};
use std::path::{Path, PathBuf};

/// A sample let go over the Tracks area, waiting for the user's choice.
#[derive(Clone, Debug, PartialEq)]
pub struct SampleDrop {
    pub path: PathBuf,
    /// Where it was let go, for the menu.
    pub at: egui::Pos2,
    /// The bar under it, from 1.
    pub bar: f64,
    /// The track under it, if any.
    pub track: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// A one-shot: a new channel with a hit at the bar it was let go on.
    OneShot,
    AudioClip,
    /// A new channel in the Channel Rack that plays it, with nothing placed yet.
    ChannelRack,
    PlayInPianoRoll,
    StepPattern,
    /// As the instrument of the track it was dropped on.
    OnThisTrack,
}

fn stem_of(path: &Path) -> String {
    path.file_stem().map_or_else(
        || "Sample".to_string(),
        |s| s.to_string_lossy().into_owned(),
    )
}

/// A sample as something an instrument slot can hold.
pub fn sampler_info(path: &Path) -> PluginInfo {
    PluginInfo {
        id: "sampler".to_string(),
        name: stem_of(path),
        vendor: "Ovrtone".to_string(),
        version: String::new(),
        format: PluginFormat::Sampler,
        path: path.to_path_buf(),
        is_instrument: true,
    }
}

/// A new instrument track holding the sample. Returns its number.
pub(crate) fn new_sampler_track(p: &mut Project, path: &Path) -> Result<usize, String> {
    let t = p.add_track(TrackKind::Instrument);
    p.tracks[t].name = stem_of(path);
    add_plugin(p, t, &sampler_info(path))?;
    Ok(t)
}

/// Whether a track's instrument is a sample, so a new sound can take its place.
pub fn holds_sample(p: &Project, t: usize) -> bool {
    p.tracks
        .get(t)
        .and_then(|t| t.instrument_plugin.as_ref())
        .is_some_and(|i| PluginFormat::of(&i.path) == PluginFormat::Sampler)
}

/// Puts another sound on a track that holds one. Its notes and its settings stay, and its name follows
/// if it was named after the old sound.
pub fn replace_sample(p: &mut Project, t: usize, path: &Path) -> Result<(), String> {
    let track = p.tracks.get_mut(t).ok_or("No such track")?;
    let plugin = track
        .instrument_plugin
        .as_mut()
        .filter(|i| PluginFormat::of(&i.path) == PluginFormat::Sampler)
        .ok_or("This track does not hold a sample")?;
    let (old, new) = (stem_of(&plugin.path), stem_of(path));
    plugin.path = path.to_path_buf();
    plugin.name.clone_from(&new);
    if track.name == old {
        track.name = new;
    }
    p.song_rev += 1;
    Ok(())
}

/// Samples up to this long are hits to play, not clips to arrange: as the sampler also decides.
const ONE_SHOT_SECONDS: f64 = 2.0;

/// Whether a file is a one-shot, by its length.
pub fn is_one_shot(path: &Path) -> bool {
    ovrtone_engine::AudioFile::open(path).is_ok_and(|f| f.duration_secs() <= ONE_SHOT_SECONDS)
}

/// Longer than this, a sound is a stem or a song part: always audio on the Playlist.
const STEM_SECONDS: f64 = 15.0;

/// Where a dropped sound goes without asking: a stem (over 15 s) or a loop (a BPM in its name and
/// longer than a hit) is audio on the Playlist; anything else is a channel of the Channel Rack.
pub fn auto_placement(path: &Path) -> Placement {
    let seconds = ovrtone_engine::AudioFile::open(path).map_or(0.0, |f| f.duration_secs());
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let loop_tempo = crate::sample_tags::name_info(&name).0.is_some();
    if seconds > STEM_SECONDS || (loop_tempo && seconds > ONE_SHOT_SECONDS) {
        Placement::AudioClip
    } else {
        Placement::ChannelRack
    }
}

/// A one-shot let go on the timeline: its own sampler track, with a note where it was let go, so that
/// it sounds at once and its key can be changed. Returns the region.
pub fn place_one_shot(p: &mut Project, path: &Path, bar: f32) -> Result<(usize, usize), String> {
    let t = new_sampler_track(p, path)?;
    let r = p
        .add_midi_region(t, bar)
        .ok_or("Could not make a region for it")?;
    p.tracks[t].regions[r].notes.push(crate::model::Note {
        start: 0.0,
        len: 1.0,
        pitch: 60,
        vel: 100,
        muted: false,
        color: 0,
        slide: false,
    });
    p.song_rev += 1;
    Ok((t, r))
}

/// A step pattern with one row, which plays the sample as it was recorded.
fn one_row_pattern() -> PatternParams {
    PatternParams {
        rows: vec![PatternRow::new(60)],
        ..PatternParams::melodic()
    }
}

/// Does what the user chose. Returns the region to show, if it is one that opens in the editor.
pub fn place(
    p: &mut Project,
    s: &mut UiState,
    drop: &SampleDrop,
    choice: Placement,
    ctx: &egui::Context,
) -> Result<Option<(usize, usize)>, String> {
    let bar = drop.bar as f32;
    match choice {
        Placement::OneShot => place_one_shot(p, &drop.path, bar).map(Some),
        Placement::ChannelRack => {
            // A channel of the pattern on show, on the Playlist at once (where the pattern is
            // already placed, or at the bar it was let go on), with the rack open on it.
            let t = new_sampler_track(p, &drop.path)?;
            crate::song_pattern::channel_of(p, t);
            let id = s.rack.current_pattern(p);
            crate::song_pattern::fill_placements(p, id, bar, Some(t));
            p.selected_track = t;
            s.rack.open = true;
            Ok(None)
        }
        Placement::AudioClip => {
            crate::import::import_audio_at(p, &drop.path, ctx, drop.bar, drop.track)
                .map_err(|e| format!("cannot import {}: {e}", drop.path.display()))?;
            let t = p.selected_track;
            if let Some(r) = p.tracks[t].regions.len().checked_sub(1) {
                s.flex.detect_tempo(p, t, r, ctx);
            }
            Ok(None)
        }
        Placement::PlayInPianoRoll => {
            let t = new_sampler_track(p, &drop.path)?;
            Ok(p.add_midi_region(t, bar).map(|r| (t, r)))
        }
        Placement::StepPattern => {
            let t = new_sampler_track(p, &drop.path)?;
            let Some(r) = p.add_generated_region(t, false, bar) else {
                return Ok(None);
            };
            p.tracks[t].regions[r].generated =
                Some(Generated::new(Generator::Pattern(one_row_pattern())));
            p.song_rev += 1;
            Ok(Some((t, r)))
        }
        Placement::OnThisTrack => {
            let Some(t) = drop.track else {
                return Err("Drop it on an instrument track".to_string());
            };
            if holds_sample(p, t) {
                replace_sample(p, t, &drop.path)?;
            } else {
                add_plugin(p, t, &sampler_info(&drop.path))?;
            }
            let first = p.tracks[t].regions.first().map(|_| 0);
            Ok(first.or_else(|| p.add_midi_region(t, bar)).map(|r| (t, r)))
        }
    }
}

/// The menu of a sample waiting for a choice. Clicking elsewhere or Escape forgets the sample.
pub fn menu(ui: &egui::Ui, p: &mut Project, s: &mut UiState) {
    let Some(drop) = s.sample_drop.clone() else {
        return;
    };
    let ctx = ui.ctx().clone();
    let name = stem_of(&drop.path);
    let on_instrument = drop
        .track
        .and_then(|t| p.tracks.get(t))
        .filter(|t| t.kind == TrackKind::Instrument)
        .map(|t| t.name.clone());
    let replaces = drop.track.is_some_and(|t| holds_sample(p, t));
    // Whether it is a hit, read once per sample (it opens the file).
    let key = egui::Id::unique(("sample_place_one_shot", drop.path.clone()));
    let one_shot = match ctx.data(|d| d.get_temp::<bool>(key)) {
        Some(v) => v,
        None => {
            let v = is_one_shot(&drop.path);
            ctx.data_mut(|d| d.insert_temp(key, v));
            v
        }
    };
    // As FL Studio: no question. A hit becomes a channel of the Channel Rack's pattern, on the
    // Playlist at once and on a mixer strip of its own; a loop becomes a clip that follows the
    // tempo. Alt held while letting go asks, with the rarer choices.
    let asking = egui::Id::unique(("sample_place_asking", drop.path.clone()));
    let asked = ctx.data(|d| d.get_temp::<bool>(asking)).unwrap_or(false);
    if !asked {
        if ctx.input(|i| i.modifiers.alt) {
            ctx.data_mut(|d| d.insert_temp(asking, true));
        } else {
            s.sample_drop = None;
            let what = auto_placement(&drop.path);
            let one_shot = what == Placement::ChannelRack;
            if let Err(e) = place(p, s, &drop, what, &ctx) {
                s.status = Some(e);
            } else if one_shot {
                s.status = Some(format!("{name}: a new channel in the Channel Rack (Alt-drop for other choices)"));
            }
            return;
        }
    }
    let mut choice = None;
    let area = egui::Area::new(egui::Id::unique("sample_place_menu"))
        .order(egui::Order::Foreground)
        .fixed_pos(drop.at)
        .show(&ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(230.0);
                ui.label(egui::RichText::new(name).strong());
                ui.separator();
                let mut item = |ui: &mut egui::Ui, label: &str, what: Placement| {
                    if ui.button(label).clicked() {
                        choice = Some(what);
                    }
                };
                if one_shot {
                    item(ui, &format!("Hit at bar {:.0} (new channel)", drop.bar), Placement::OneShot);
                }
                item(ui, "Audio clip on the timeline", Placement::AudioClip);
                item(ui, "New channel in the Channel Rack", Placement::ChannelRack);
                item(ui, "New channel with a step pattern", Placement::StepPattern);
                item(ui, "New channel: play it in the Piano Roll", Placement::PlayInPianoRoll);
                if let Some(track) = &on_instrument {
                    let label = if replaces {
                        format!("Replace the sound on {track}")
                    } else {
                        format!("Play it on {track}")
                    };
                    item(ui, &label, Placement::OnThisTrack);
                }
            });
        });
    let dismissed = ctx.input(|i| i.key_pressed(egui::Key::Escape))
        || ctx.input(|i| {
            i.pointer.any_pressed()
                && i.pointer
                    .interact_pos()
                    .is_some_and(|pt| !area.response.rect.contains(pt))
        });
    if let Some(what) = choice {
        s.sample_drop = None;
        ctx.data_mut(|d| d.remove::<bool>(asking));
        match place(p, s, &drop, what, &ctx) {
            Ok(Some(region)) => {
                s.tracks.selected = Some(region);
                p.selected_track = region.0;
                s.bottom = crate::Bottom::PianoRoll;
            }
            Ok(None) => {}
            Err(e) => s.status = Some(e),
        }
    } else if dismissed {
        s.sample_drop = None;
        ctx.data_mut(|d| d.remove::<bool>(asking));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn rig() -> (Project, UiState, egui::Context, tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("Kick 01.wav");
        ovrtone_engine::write_wav(&wav, &vec![0.1; 48_000], 48_000).unwrap();
        (
            Project::empty(),
            UiState::new(),
            egui::Context::default(),
            dir,
            wav,
        )
    }

    #[test]
    fn a_new_sound_takes_the_place_of_the_old_and_keeps_the_notes_and_settings() {
        let (mut p, _s, _ctx, dir, wav) = rig();
        let (t, r) = place_one_shot(&mut p, &wav, 3.0).unwrap();
        assert!(holds_sample(&p, t));
        assert_eq!(p.tracks[t].regions[r].notes.len(), 1, "it sounds at once");
        let uid = p.tracks[t].instrument_plugin.as_ref().unwrap().uid;
        let other = dir.path().join("Snare 02.wav");
        replace_sample(&mut p, t, &other).unwrap();
        let held = p.tracks[t].instrument_plugin.as_ref().unwrap();
        assert_eq!((held.uid, held.path.clone()), (uid, other.clone()));
        assert_eq!(p.tracks[t].name, "Snare 02", "named after the sound");
        assert_eq!(p.tracks[t].regions[r].notes.len(), 1);
        // A name the user gave stays.
        p.tracks[t].name = "Backbeat".to_string();
        replace_sample(&mut p, t, &wav).unwrap();
        assert_eq!(p.tracks[t].name, "Backbeat");
        // One second is a hit; a track with no sample has nothing to replace.
        assert!(is_one_shot(&wav));
        let empty = p.add_track(TrackKind::Audio);
        assert!(replace_sample(&mut p, empty, &wav).is_err());
    }

    fn dropped(path: &Path, track: Option<usize>) -> SampleDrop {
        SampleDrop {
            path: path.to_path_buf(),
            at: egui::Pos2::ZERO,
            bar: 3.0,
            track,
        }
    }

    #[test]
    fn it_becomes_an_audio_clip_at_the_bar_it_was_dropped_on() {
        let (mut p, mut s, ctx, _dir, wav) = rig();
        let shown = place(
            &mut p,
            &mut s,
            &dropped(&wav, None),
            Placement::AudioClip,
            &ctx,
        );
        assert_eq!(shown, Ok(None));
        let t = p.tracks.last().unwrap();
        assert_eq!(t.kind, TrackKind::Audio);
        assert_eq!(t.regions[0].start_bar, 3.0);
        assert!(t.regions[0].audio.is_some());
    }

    #[test]
    fn it_becomes_a_track_that_plays_it_from_the_piano_roll() {
        let (mut p, mut s, ctx, _dir, wav) = rig();
        let (t, r) = place(
            &mut p,
            &mut s,
            &dropped(&wav, None),
            Placement::PlayInPianoRoll,
            &ctx,
        )
        .unwrap()
        .unwrap();
        let track = &p.tracks[t];
        assert_eq!(track.kind, TrackKind::Instrument);
        assert_eq!(track.name, "Kick 01");
        let sampler = track.instrument_plugin.as_ref().unwrap();
        assert_eq!(sampler.path, wav);
        assert_eq!(track.regions[r].start_bar, 3.0);
        assert!(track.regions[r].generated.is_none());
    }

    #[test]
    fn it_becomes_a_pattern_of_one_row_at_its_own_pitch() {
        let (mut p, mut s, ctx, _dir, wav) = rig();
        let (t, r) = place(
            &mut p,
            &mut s,
            &dropped(&wav, None),
            Placement::StepPattern,
            &ctx,
        )
        .unwrap()
        .unwrap();
        let region = &p.tracks[t].regions[r];
        match &region.generated.as_ref().unwrap().source {
            Generator::Pattern(pattern) => {
                assert_eq!(pattern.rows.len(), 1);
                assert_eq!(pattern.rows[0].pitch, 60, "the pitch it plays as recorded");
            }
            Generator::Drummer(_) => panic!("a pattern"),
        }
        assert!(p.tracks[t].instrument_plugin.is_some());
    }

    #[test]
    fn it_can_go_on_the_track_under_it_which_keeps_its_region() {
        let (mut p, mut s, ctx, _dir, wav) = rig();
        let keys = p.add_track(TrackKind::Instrument);
        let (t, r) = place(
            &mut p,
            &mut s,
            &dropped(&wav, Some(keys)),
            Placement::OnThisTrack,
            &ctx,
        )
        .unwrap()
        .unwrap();
        assert_eq!((t, r), (keys, 0));
        assert!(p.tracks[keys].instrument_plugin.is_some());
        // A second time keeps the same region, with another sample in the slot.
        let (_, r) = place(
            &mut p,
            &mut s,
            &dropped(&wav, Some(keys)),
            Placement::OnThisTrack,
            &ctx,
        )
        .unwrap()
        .unwrap();
        assert_eq!(r, 0);
        assert_eq!(p.tracks[keys].regions.len(), 1);
        // Nothing under it to go on.
        assert!(place(
            &mut p,
            &mut s,
            &dropped(&wav, None),
            Placement::OnThisTrack,
            &ctx
        )
        .is_err());
    }
}
