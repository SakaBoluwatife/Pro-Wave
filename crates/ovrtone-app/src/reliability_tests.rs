//! Tests that cross modules: what must keep working whatever state a project is in.
use crate::file_commands::content;
use crate::model::{AutoTarget, Note, Project, TrackKind};
use crate::undo::History;
use crate::UiState;
use ovrtone_host::{PluginFormat, PluginInfo};

fn effect(name: &str) -> PluginInfo {
    PluginInfo {
        id: format!("id.{name}"),
        name: name.to_string(),
        vendor: "Test".to_string(),
        version: String::new(),
        format: PluginFormat::Clap,
        path: format!("{name}.clap").into(),
        is_instrument: false,
    }
}

fn note(start: f32, pitch: u8) -> Note {
    Note {
        start,
        len: 0.25,
        pitch,
        vel: 90,
        muted: false,
        color: 0,
        slide: false,
    }
}

/// A project using everything that gets saved.
fn busy_project() -> Project {
    let mut p = Project::empty();
    p.tracks.clear();
    let keys = p.add_track(TrackKind::Instrument);
    let audio = p.add_track(TrackKind::Audio);
    p.add_midi_region(keys, 2.0).unwrap();
    p.tracks[keys].regions[0].notes = vec![note(0.0, 60), note(1.0, 64)];

    crate::plugin_rack::add_effect_in_row(&mut p, audio, &effect("Gap"), 3).unwrap();
    crate::plugin_rack::add_effect_in_row(&mut p, audio, &effect("First"), 1).unwrap();
    crate::plugin_rack::add_master_effect(&mut p, &effect("Glue"), 2).unwrap();
    p.tracks[audio].plugins[0].state = Some(std::sync::Arc::new(vec![1, 2, 3, 250]));
    p.master_plugins[0].state = Some(std::sync::Arc::new(vec![9, 8, 7]));
    let uid = p.tracks[audio].plugins[1].uid;
    p.tracks[audio]
        .auto
        .set_point(AutoTarget::Plugin { uid, param: 42 }, 2.0, 0.75);
    p.tracks[audio]
        .auto
        .set_point(AutoTarget::Volume, 1.0, -6.0);

    let id = crate::song_pattern::new_pattern(&mut p);
    let channel = crate::song_pattern::channel_of(&mut p, keys);
    let pattern = p.patterns.iter_mut().find(|x| x.id == id).unwrap();
    pattern.name = "Hook".to_string();
    pattern.lane_mut(channel).toggle_step(4);
    crate::song_pattern::place(&mut p, id, 9.0);
    p
}

#[test]
fn every_part_of_a_project_comes_back_from_its_file() {
    let dir = tempfile::tempdir().unwrap();
    let p = busy_project();
    let path = crate::project_io::save_project(&p, &dir.path().join("all")).unwrap();
    let ctx = egui::Context::default();
    let (back, warnings) = crate::project_io::open_project(&path, &ctx).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");

    // Saving what came back gives the same document, including what uids stand for.
    let (mut a, _) = crate::project_io::doc_for_saving(&p);
    let (mut b, _) = crate::project_io::doc_for_saving(&back);
    a.name = String::new();
    b.name = String::new();
    assert_eq!(a, b);

    // And in detail, the parts most likely to go astray.
    let audio = &back.tracks[1];
    assert_eq!(audio.plugins[0].name, "First");
    assert_eq!(
        audio.plugins[0].state.as_deref().map(Vec::as_slice),
        Some(&[1u8, 2, 3, 250][..])
    );
    assert_eq!(crate::model::plugin_rows(0, &audio.plugins), [1, 3]);
    let lane = &audio.auto.plugin_lanes[0];
    assert_eq!(lane.uid, audio.plugins[1].uid, "the lane found its plugin");
    assert_eq!((lane.param, lane.points[0].value), (42, 0.75));
    assert_eq!(back.master_plugins[0].name, "Glue");
    assert_eq!(
        back.master_plugins[0].state.as_deref().map(Vec::as_slice),
        Some(&[9u8, 8, 7][..])
    );
    assert_eq!(back.patterns[0].name, "Hook");
    let placed = &back.tracks[0].regions[1];
    assert_eq!(placed.params.pattern, back.patterns[0].id);
}

#[test]
fn a_file_from_before_these_features_still_opens() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = Project::empty();
    p.tracks[0].name = "Old".to_string();
    let path = crate::project_io::save_project(&p, &dir.path().join("old")).unwrap();
    // The same file with the newer fields taken out, as an older version wrote it.
    let doc = ovrtone_project::load(&path, dir.path()).unwrap();
    let mut json = serde_json::to_value(&doc).unwrap();
    let root = json.as_object_mut().unwrap();
    root.remove("song_patterns");
    root.remove("master_plugins");
    let older: ovrtone_project::ProjectDoc = serde_json::from_value(json).unwrap();
    assert!(older.song_patterns.is_empty() && older.master_plugins.is_empty());
    assert_eq!(older.tracks[0].name, "Old");
}

#[test]
fn undo_and_redo_bring_back_every_kind_of_change() {
    let mut p = busy_project();
    let mut history = History::new(&p);
    let start = content(&p);
    let uid = p.tracks[1].plugins[0].uid;

    // A plugin keeps its running instance through undo: it is found by its uid.
    p.tracks[1].plugins.remove(0);
    assert!(history.record(&p));
    assert!(history.undo(&mut p));
    assert_eq!(content(&p), start);
    assert_eq!(p.tracks[1].plugins[0].uid, uid);

    for change in [
        |p: &mut Project| p.tempo = 77.0,
        |p: &mut Project| p.master_plugins.clear(),
        |p: &mut Project| p.patterns[0].name = "Other".to_string(),
        |p: &mut Project| p.tracks[0].regions[0].notes.clear(),
        |p: &mut Project| {
            p.tracks.remove(1);
        },
        |p: &mut Project| p.tracks[1].auto.plugin_lanes.clear(),
    ] {
        change(&mut p);
        let changed = content(&p);
        assert_ne!(changed, start);
        assert!(history.record(&p));
        assert!(history.undo(&mut p));
        assert_eq!(content(&p), start);
        assert!(history.redo(&mut p));
        assert_eq!(content(&p), changed);
        assert!(history.undo(&mut p));
    }
}

#[test]
fn a_project_with_no_tracks_draws_and_takes_every_command() {
    let mut p = Project::empty();
    while !p.tracks.is_empty() {
        assert!(p.delete_track(0));
    }
    let mut s = UiState::new();
    let meters = crate::mixer::Meters::default();
    let mut h = crate::ui_test::Harness::new(1600.0, 1000.0);
    s.rack.open = true;
    s.settings.open = true;
    s.show_lists = true;
    h.settle(|ui| {
        crate::tracks_area::show(ui, &mut p, &mut s, &meters);
        crate::mixer::show(ui, &mut p, &mut s, &meters);
        crate::inspector::show(ui, &mut p, &mut s, &meters);
        crate::piano_roll::show(ui, &mut p, &mut s);
        crate::event_list::show(ui, &mut p, &mut s);
        crate::channel_rack::show(ui.ctx(), &mut p, &mut s);
        crate::settings::show(ui.ctx(), &mut s);
        crate::automation::show(ui.ctx(), &mut p, &mut s);
    });
    assert!(p.tracks.is_empty());

    // Any key and any menu entry is safe. Some make a track, so it is emptied each time.
    let mut broke = Vec::new();
    for label in crate::menus::all_entries() {
        let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if crate::menus::enabled(label, &p, &s) {
                crate::menus::apply(label, &mut p, &mut s);
            }
        }));
        if ran.is_err() {
            broke.push(label.to_string());
        }
        p.tracks.clear();
        p.selected_track = 0;
        s.tracks.select_many(Vec::new());
    }
    for command in crate::key_commands::COMMANDS {
        let action = command.action;
        let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::key_commands::perform(action, &mut p, &mut s);
        }));
        if ran.is_err() {
            broke.push(format!("{action:?}"));
        }
        p.tracks.clear();
        p.selected_track = 0;
        s.tracks.select_many(Vec::new());
    }

    assert!(broke.is_empty(), "{broke:?}");

    // The engine takes it as well, and a new track works after that.
    let (engine, mut core) = ovrtone_engine::Engine::offline(48_000);
    let mut sync = crate::engine_sync::EngineSync::new(engine, &p);
    sync.sync(&mut p);
    let mut buf = vec![0.0f32; 1024];
    core.process(&mut buf);
    assert!(buf.iter().all(|s| s.is_finite()));
    let t = p.add_track(TrackKind::Instrument);
    assert_eq!(p.selected_track, t);
    sync.sync(&mut p);
    core.process(&mut buf);
}
