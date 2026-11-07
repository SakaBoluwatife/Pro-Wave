//! What a sample dropped on the Tracks area becomes: the user chooses, as the sample is let go.
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
    AudioClip,
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
            add_plugin(p, t, &sampler_info(&drop.path))?;
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
                item(ui, "Audio clip on the timeline", Placement::AudioClip);
                item(
                    ui,
                    "New track: play it in the piano roll",
                    Placement::PlayInPianoRoll,
                );
                item(ui, "New track: step pattern", Placement::StepPattern);
                if let Some(track) = &on_instrument {
                    item(ui, &format!("Play it on {track}"), Placement::OnThisTrack);
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
