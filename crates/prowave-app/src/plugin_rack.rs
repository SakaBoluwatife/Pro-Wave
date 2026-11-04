//! Keeps the plugins running in the engine in line with what the project says.
//!
//! The project holds plain data ([`PluginInsert`]), so Undo and duplicating a track work as for
//! anything else. Live instances cannot be cloned or rebuilt without losing their state, so each
//! has a `uid` and follows its insert when tracks and slots move.
use crate::model::{PluginInsert, Project, Track, TrackKind};
use prowave_engine::{
    Command, Engine, PluginSlot, INSTRUMENT_SLOT, MAX_INSERTS, MAX_TRACKS, PLUGIN_BLOCK,
};
use prowave_host::{load_clap, LoadedClap, PluginInfo};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// A plugin's place in the engine: track and insert slot.
pub type Place = (u16, u8);

static NEXT_UID: AtomicU64 = AtomicU64::new(1);

pub fn new_uid() -> u64 {
    NEXT_UID.fetch_add(1, Ordering::Relaxed)
}

/// What a click or drop on the Plugins tab asks for: `info` on track `track`. An instrument goes
/// on an instrument track, in place of what plays it now; an effect in the next insert slot.
pub fn add_plugin(p: &mut Project, track: usize, info: &PluginInfo) -> Result<(), String> {
    let Some(t) = p.tracks.get_mut(track) else {
        return Err("Select a track first".to_string());
    };
    if info.is_instrument {
        if !matches!(t.kind, TrackKind::Instrument | TrackKind::Drummer) {
            return Err(format!(
                "{} is an instrument: put it on an instrument track",
                info.name
            ));
        }
        t.instrument_plugin = Some(PluginInsert::new(info));
        p.selected_track = track;
        return Ok(());
    }
    if matches!(t.kind, TrackKind::Folder | TrackKind::Vca) {
        return Err(format!("{} cannot hold effects", t.name));
    }
    if t.inserts.len() + t.plugins.len() >= MAX_INSERTS {
        return Err(format!("{} has no free insert slot", t.name));
    }
    t.plugins.push(PluginInsert::new(info));
    p.selected_track = track;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Remove(u64),
    Swap(Place, Place),
    Load(u64, Place),
}

/// What to do so that `live` becomes `want`: removals first, which free slots; then moves, each of
/// which puts one plugin where it belongs for good; then loads into the slots left free.
fn plan(want: &[(u64, Place)], live: &HashMap<u64, Place>) -> Vec<Step> {
    let mut steps = Vec::new();
    let wanted: HashSet<u64> = want.iter().map(|(uid, _)| *uid).collect();
    let mut at = live.clone();
    let mut gone: Vec<u64> = live
        .keys()
        .copied()
        .filter(|uid| !wanted.contains(uid))
        .collect();
    gone.sort_unstable();
    for uid in gone {
        at.remove(&uid);
        steps.push(Step::Remove(uid));
    }
    for (uid, to) in want {
        let Some(from) = at.get(uid).copied() else {
            continue;
        };
        if from == *to {
            continue;
        }
        steps.push(Step::Swap(from, *to));
        let other = at.iter().find(|(_, place)| **place == *to).map(|(u, _)| *u);
        if let Some(other) = other {
            at.insert(other, from);
        }
        at.insert(*uid, *to);
    }
    for (uid, to) in want {
        if !at.contains_key(uid) {
            steps.push(Step::Load(*uid, *to));
        }
    }
    steps
}

/// Where each plugin of the project goes in the engine. One that failed to load is left out, and
/// so is what does not fit the engine, but the slots of those behind it stay as they are.
fn wanted_places(p: &Project) -> Vec<(u64, Place)> {
    let mut want = Vec::new();
    for (t, track) in p.tracks.iter().enumerate().take(MAX_TRACKS) {
        if let Some(plugin) = track
            .instrument_plugin
            .as_ref()
            .filter(|i| i.failed.is_none())
        {
            want.push((plugin.uid, (t as u16, INSTRUMENT_SLOT)));
        }
        for (k, plugin) in track.plugins.iter().enumerate().take(MAX_INSERTS) {
            if plugin.failed.is_none() {
                want.push((plugin.uid, (t as u16, k as u8)));
            }
        }
    }
    want
}

/// (uid, bypass) of each plugin, and its place.
fn wanted_bypass(p: &Project) -> Vec<((u64, bool), Place)> {
    let mut all = Vec::new();
    for (t, track) in p.tracks.iter().enumerate().take(MAX_TRACKS) {
        if let Some(plugin) = &track.instrument_plugin {
            all.push(((plugin.uid, plugin.bypass), (t as u16, INSTRUMENT_SLOT)));
        }
        for (k, plugin) in track.plugins.iter().enumerate().take(MAX_INSERTS) {
            all.push(((plugin.uid, plugin.bypass), (t as u16, k as u8)));
        }
    }
    all
}

/// Copies of a track (Duplicate, a pasted strip) come with the uids of the originals. Each needs
/// an instance of its own, so all but the first get a new uid. Returns (original, copy).
pub fn give_copies_their_own_uids(p: &mut Project) -> Vec<(u64, u64)> {
    let mut seen = HashSet::new();
    let mut copies = Vec::new();
    for plugin in p.tracks.iter_mut().flat_map(Track::hosted_mut) {
        if !seen.insert(plugin.uid) {
            let original = plugin.uid;
            plugin.uid = new_uid();
            copies.push((original, plugin.uid));
        }
    }
    copies
}

/// Settings of plugins that are not running but may be back (Undo, a copy of a track). The oldest
/// go first past this many.
const KEPT_STATES: usize = 64;

struct Live {
    place: Place,
    /// The plugin's identifier: where its editor window was is remembered per plugin.
    id: String,
    /// As last told to the engine.
    bypass: bool,
    loaded: LoadedClap,
}

const WINDOWS_FILE: &str = "plugin-windows";

/// Where each plugin's editor window was, by the plugin's identifier: a line of `x y id`.
fn parse_positions(text: &str) -> HashMap<String, (i32, i32)> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.trim().splitn(3, ' ');
            let (x, y, id) = (parts.next()?, parts.next()?, parts.next()?);
            Some((id.to_string(), (x.parse().ok()?, y.parse().ok()?)))
        })
        .collect()
}

fn positions_text(positions: &HashMap<String, (i32, i32)>) -> String {
    let mut lines: Vec<String> = positions
        .iter()
        .map(|(id, (x, y))| format!("{x} {y} {id}"))
        .collect();
    lines.sort();
    lines.join("\n")
}

fn windows_file() -> Option<std::path::PathBuf> {
    crate::recent_files::config_dir().map(|d| d.join(WINDOWS_FILE))
}

/// The plugins running in one engine. Dropped after the engine, so instances go last.
#[derive(Default)]
pub struct Rack {
    live: HashMap<u64, Live>,
    /// Taken out of the engine, kept until the engine has given the audio half back: an instance
    /// dropped before that is never destroyed.
    retiring: Vec<LoadedClap>,
    /// Where editor windows were. `None` until first needed.
    positions: Option<HashMap<String, (i32, i32)>>,
    /// The settings a plugin with this uid starts from, newest last.
    kept_states: Vec<(u64, Arc<Vec<u8>>)>,
    /// A plugin may have changed since the last save (it said so, or its editor was opened).
    touched: bool,
}

impl Rack {
    /// The delay of the plugin with this uid in its slot, 0 when it is not running.
    pub fn latency_of(&self, uid: u64) -> usize {
        self.live
            .get(&uid)
            .map_or(0, |l| l.loaded.latency_samples() + PLUGIN_BLOCK)
    }

    /// Whether plugins may hold changes the last save does not have. Their settings are their own,
    /// so this is the only sign of an unsaved change in them.
    pub fn touched(&self) -> bool {
        self.touched
    }

    /// The project was just saved, or a new one opened.
    pub fn clear_touched(&mut self) {
        self.touched = false;
    }

    /// Collects the current settings of every running plugin into the project, for saving.
    pub fn collect_states(&mut self, p: &mut Project, message: &mut Option<String>) {
        for plugin in p.tracks.iter_mut().flat_map(Track::hosted_mut) {
            let Some(live) = self.live.get_mut(&plugin.uid) else {
                continue;
            };
            match live.loaded.save_state() {
                Ok(Some(bytes)) => plugin.state = Some(Arc::new(bytes)),
                Ok(None) => {}
                Err(e) => *message = Some(format!("{}: {e}", plugin.name)),
            }
        }
    }

    /// A copy of a track starts from what the original sounds like now.
    pub fn copy_states(&mut self, copies: &[(u64, u64)]) {
        for (original, copy) in copies {
            let state = self
                .live
                .get_mut(original)
                .and_then(|l| l.loaded.save_state().ok().flatten());
            if let Some(bytes) = state {
                self.keep_state(*copy, Arc::new(bytes));
            }
        }
    }

    fn keep_state(&mut self, uid: u64, state: Arc<Vec<u8>>) {
        self.kept_states.retain(|(u, _)| *u != uid);
        self.kept_states.push((uid, state));
        if self.kept_states.len() > KEPT_STATES {
            self.kept_states.remove(0);
        }
    }

    /// Whether any plugin runs: they may ask for things on this thread at any time, and their
    /// editors need looking after, so the program must not sleep through them.
    pub fn has_plugins(&self) -> bool {
        !self.live.is_empty() || !self.retiring.is_empty()
    }

    /// Whether any editor window is open, which wants a quicker answer than `has_plugins`.
    pub fn has_open_editor(&self) -> bool {
        self.live.values().any(|l| l.loaded.editor_is_open())
    }

    fn positions(&mut self) -> &mut HashMap<String, (i32, i32)> {
        self.positions.get_or_insert_with(|| {
            windows_file()
                .and_then(|f| std::fs::read_to_string(f).ok())
                .map(|t| parse_positions(&t))
                .unwrap_or_default()
        })
    }

    /// Notes where an editor that closed was, and saves it for the next run.
    fn remember_position(&mut self, id: &str, at: (i32, i32)) {
        self.positions().insert(id.to_string(), at);
        if cfg!(test) {
            return;
        }
        let text = positions_text(self.positions());
        let saved = windows_file()
            .ok_or_else(|| std::io::Error::other("no config folder"))
            .and_then(|file| {
                file.parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|()| std::fs::write(&file, text))
            });
        if let Err(e) = saved {
            tracing::warn!("could not save where plugin windows were: {e}");
        }
    }

    /// Opens the editor of a plugin, above the main window `owner`.
    pub fn open_editor(
        &mut self,
        uid: u64,
        title: &str,
        owner: Option<isize>,
    ) -> Result<(), String> {
        let Some(id) = self.live.get(&uid).map(|l| l.id.clone()) else {
            return Err(format!("{title} is not running"));
        };
        let at = self.positions().get(&id).copied();
        self.touched = true;
        let Some(live) = self.live.get_mut(&uid) else {
            return Err(format!("{title} is not running"));
        };
        live.loaded
            .open_editor(title, owner, at)
            .map_err(|e| e.to_string())
    }

    /// Runs what the plugins asked for on this thread and brings the engine in line with `p`.
    /// Returns false when the engine's queue was full, so the caller tries again next frame;
    /// `message` gets what went wrong for the status line. At most one plugin loads per call, so
    /// a project with many stays responsive while it opens.
    pub fn sync(
        &mut self,
        p: &mut Project,
        engine: &mut Engine,
        message: &mut Option<String>,
    ) -> bool {
        let mut closed = Vec::new();
        for live in self.live.values_mut() {
            live.loaded.tick();
            self.touched |= live.loaded.take_dirty();
            if let Some(at) = live.loaded.take_closed_position() {
                closed.push((live.id.clone(), at));
            }
        }
        for (id, at) in closed {
            self.remember_position(&id, at);
        }
        self.retiring.retain(|r| !r.released());
        let places: HashMap<u64, Place> = self.live.iter().map(|(u, l)| (*u, l.place)).collect();
        let mut loaded_one = false;
        let mut more = false;
        for step in plan(&wanted_places(p), &places) {
            match step {
                Step::Remove(uid) => {
                    let Some(place) = self.live.get(&uid).map(|l| l.place) else {
                        continue;
                    };
                    if !engine.send(Command::SetPlugin {
                        track: place.0,
                        slot: place.1,
                        plugin: None,
                    }) {
                        return false;
                    }
                    if let Some(mut live) = self.live.remove(&uid) {
                        // Undo may bring it back, as it was.
                        if let Ok(Some(bytes)) = live.loaded.save_state() {
                            self.keep_state(uid, Arc::new(bytes));
                        }
                        // Its window goes at once; the rest when the engine has let go of it.
                        live.loaded.close_editor();
                        if let Some(at) = live.loaded.take_closed_position() {
                            self.remember_position(&live.id, at);
                        }
                        self.retiring.push(live.loaded);
                    }
                }
                Step::Swap(a, b) => {
                    if !engine.send(Command::SwapPlugins { a, b }) {
                        return false;
                    }
                    for live in self.live.values_mut() {
                        if live.place == a {
                            live.place = b;
                        } else if live.place == b {
                            live.place = a;
                        }
                    }
                }
                Step::Load(uid, place) => {
                    if loaded_one {
                        more = true;
                        continue;
                    }
                    loaded_one = true;
                    if !self.load(p, uid, place, engine, message) {
                        return false;
                    }
                }
            }
        }
        for (plugin, place) in wanted_bypass(p) {
            let Some(live) = self.live.get_mut(&plugin.0) else {
                continue;
            };
            if live.bypass != plugin.1 {
                if !engine.send(Command::SetPluginBypass {
                    track: place.0,
                    slot: place.1,
                    bypass: plugin.1,
                }) {
                    return false;
                }
                live.bypass = plugin.1;
            }
        }
        !more
    }

    /// Loads one plugin into the engine. A plugin that will not load is marked in the project,
    /// and shown as such; false only when the engine could not take it now.
    fn load(
        &mut self,
        p: &mut Project,
        uid: u64,
        place: Place,
        engine: &mut Engine,
        message: &mut Option<String>,
    ) -> bool {
        let Some(plugin) = p
            .tracks
            .iter_mut()
            .flat_map(Track::hosted_mut)
            .find(|i| i.uid == uid)
        else {
            return true;
        };
        let (mut loaded, processor) =
            match load_clap(&plugin.path, &plugin.id, engine.sample_rate()) {
                Ok(both) => both,
                Err(e) => {
                    *message = Some(e.to_string());
                    plugin.failed = Some(e.to_string());
                    return true;
                }
            };
        let kept = self.kept_states.iter().find(|(u, _)| *u == uid);
        let state = kept
            .map(|(_, s)| s.clone())
            .or_else(|| plugin.state.clone());
        if let Some(state) = state {
            if let Err(e) = loaded.load_state(&state) {
                // It runs from its defaults. Its old settings stay in the project.
                *message = Some(format!("{}: settings not restored: {e}", plugin.name));
            }
        }
        let sent = engine.send(Command::SetPlugin {
            track: place.0,
            slot: place.1,
            plugin: Some(Box::new(PluginSlot::new(processor))),
        });
        if sent {
            let id = plugin.id.clone();
            self.live.insert(
                uid,
                Live {
                    place,
                    id,
                    // The slot starts running; the next `sync` tells it if it should not be.
                    bypass: false,
                    loaded,
                },
            );
        }
        sent
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn map(items: &[(u64, Place)]) -> HashMap<u64, Place> {
        items.iter().copied().collect()
    }

    /// What `steps` leave running when done to `live`, checking each step is possible.
    fn apply(live: &[(u64, Place)], steps: &[Step]) -> HashMap<u64, Place> {
        let mut at = map(live);
        for step in steps {
            match *step {
                Step::Remove(uid) => {
                    at.remove(&uid).unwrap();
                }
                Step::Swap(a, b) => {
                    for place in at.values_mut() {
                        if *place == a {
                            *place = b;
                        } else if *place == b {
                            *place = a;
                        }
                    }
                }
                Step::Load(uid, place) => {
                    assert!(at.values().all(|p| *p != place), "slot {place:?} is taken");
                    assert!(at.insert(uid, place).is_none());
                }
            }
        }
        at
    }

    #[test]
    fn editor_positions_round_trip_with_spaces_in_identifiers() {
        let mut positions = HashMap::new();
        positions.insert("com.maker.plug".to_string(), (10, -20));
        positions.insert("odd id with spaces".to_string(), (0, 5));
        assert_eq!(parse_positions(&positions_text(&positions)), positions);
        assert!(parse_positions("nonsense\nx y z\n").is_empty());
    }

    #[test]
    fn only_so_many_settings_are_kept_and_the_newest_of_each() {
        let mut rack = Rack::default();
        let state = |n: u8| Arc::new(vec![n]);
        rack.keep_state(1, state(1));
        rack.keep_state(1, state(2));
        assert_eq!(rack.kept_states, [(1, state(2))]);
        for uid in 2..(KEPT_STATES as u64 + 2) {
            rack.keep_state(uid, state(0));
        }
        assert_eq!(rack.kept_states.len(), KEPT_STATES);
        assert!(rack.kept_states.iter().all(|(u, _)| *u != 1), "oldest gone");
    }

    #[test]
    fn nothing_to_do_when_everything_is_in_place() {
        let same = [(1, (0, 0)), (2, (3, 1))];
        assert!(plan(&same, &map(&same)).is_empty());
    }

    #[test]
    fn plugins_load_are_removed_and_follow_their_inserts() {
        for (live, want) in [
            (vec![], vec![(1, (0, 0))]),
            (vec![(1, (0, 0))], vec![]),
            // A track was deleted: what was on the track below moves up.
            (vec![(1, (0, 0)), (2, (1, 0))], vec![(2, (0, 0))]),
            // Two slots traded places.
            (
                vec![(1, (0, 0)), (2, (0, 1))],
                vec![(2, (0, 0)), (1, (0, 1))],
            ),
            // Around in a circle, with one new and one gone.
            (
                vec![(1, (0, 0)), (2, (0, 1)), (3, (0, 2)), (4, (5, 5))],
                vec![(3, (0, 0)), (1, (0, 1)), (2, (0, 2)), (9, (5, 5))],
            ),
        ] {
            let steps = plan(&want, &map(&live));
            assert_eq!(apply(&live, &steps), map(&want), "{live:?} to {want:?}");
        }
    }

    #[test]
    fn a_moved_plugin_is_not_reloaded() {
        let steps = plan(&[(1, (2, 0))], &map(&[(1, (0, 0))]));
        assert_eq!(steps, [Step::Swap((0, 0), (2, 0))]);
    }

    fn info(is_instrument: bool) -> PluginInfo {
        PluginInfo {
            id: "com.test".into(),
            name: "Test".into(),
            vendor: "Maker".into(),
            version: "1".into(),
            format: prowave_host::PluginFormat::Clap,
            path: PathBuf::from("test.clap"),
            is_instrument,
        }
    }

    #[test]
    fn instruments_go_on_instrument_tracks_and_replace_each_other() {
        let mut p = Project::empty();
        p.tracks.push(Track::new("Vox", TrackKind::Audio));
        p.tracks.push(Track::new("Keys", TrackKind::Instrument));
        let (vox, keys) = (p.tracks.len() - 2, p.tracks.len() - 1);
        assert!(add_plugin(&mut p, vox, &info(true)).is_err());
        add_plugin(&mut p, keys, &info(true)).unwrap();
        let first = p.tracks[keys].instrument_plugin.as_ref().unwrap().uid;
        assert!(wanted_places(&p).contains(&(first, (keys as u16, INSTRUMENT_SLOT))));
        add_plugin(&mut p, keys, &info(true)).unwrap();
        let second = p.tracks[keys].instrument_plugin.as_ref().unwrap().uid;
        assert_ne!(first, second, "a new instance");
        let places = wanted_places(&p);
        assert!(!places.iter().any(|(u, _)| *u == first));
        assert_eq!(p.tracks[keys].hosted().count(), 1);
    }

    #[test]
    fn effects_go_on_tracks_with_room_and_a_plugin_that_failed_is_not_retried() {
        let mut p = Project::demo();
        let track = p
            .tracks
            .iter()
            .position(|t| t.kind == TrackKind::Audio || t.kind == TrackKind::Instrument)
            .unwrap();
        p.tracks[track].inserts.clear();
        add_plugin(&mut p, track, &info(false)).unwrap();
        assert_eq!(p.tracks[track].plugins[0].name, "Test");
        assert_eq!(p.selected_track, track);
        assert!(add_plugin(&mut p, 9999, &info(false)).is_err());
        for _ in 1..MAX_INSERTS {
            add_plugin(&mut p, track, &info(false)).unwrap();
        }
        assert!(add_plugin(&mut p, track, &info(false)).is_err(), "full");

        let uid = p.tracks[track].plugins[1].uid;
        assert!(wanted_places(&p).iter().any(|(u, _)| *u == uid));
        p.tracks[track].plugins[1].failed = Some("broken".into());
        assert!(wanted_places(&p).iter().all(|(u, _)| *u != uid));
    }

    #[test]
    fn a_copied_track_gets_instances_of_its_own() {
        let mut p = Project::demo();
        let track = 0;
        p.tracks[track].kind = TrackKind::Audio;
        p.tracks[track].inserts.clear();
        add_plugin(&mut p, track, &info(false)).unwrap();
        let copy = p.tracks[track].clone();
        p.tracks.push(copy);
        let copies = give_copies_their_own_uids(&mut p);
        let last = p.tracks.len() - 1;
        assert_eq!(
            copies,
            [(
                p.tracks[track].plugins[0].uid,
                p.tracks[last].plugins[0].uid
            )]
        );
        assert_ne!(
            p.tracks[track].plugins[0].uid,
            p.tracks[last].plugins[0].uid
        );
    }

    /// The real path: the sound device, an installed plugin, and the delay compensation it asks
    /// for. `cargo test -p prowave-app rack_runs_a_real -- --ignored --nocapture`, with
    /// `PROWAVE_TEST_CLAP` naming a `.clap` file, or else the first plugin installed.
    #[test]
    #[ignore = "needs a sound device and an installed CLAP plugin"]
    fn rack_runs_a_real_plugin_in_the_engine() {
        use prowave_host::{clap_folders, find_clap_files, scan_clap_file};
        let file = std::env::var_os("PROWAVE_TEST_CLAP")
            .map(PathBuf::from)
            .or_else(|| find_clap_files(&clap_folders()).into_iter().next())
            .unwrap();
        let info = scan_clap_file(&file).unwrap().remove(0);
        println!("{} / {}", info.vendor, info.name);
        let mut p = Project::empty();
        p.tracks.push(p.tracks.first().cloned().unwrap_or_else(|| {
            let mut demo = Project::demo();
            demo.tracks.remove(0)
        }));
        p.tracks.truncate(1);
        p.tracks[0].kind = TrackKind::Audio;
        p.tracks[0].inserts.clear();
        p.tracks[0].plugins.clear();
        add_plugin(&mut p, 0, &info).unwrap();
        let uid = p.tracks[0].plugins[0].uid;

        let mut engine = Engine::start().unwrap();
        let mut rack = Rack::default();
        let mut message = None;
        assert!(rack.sync(&mut p, &mut engine, &mut message));
        assert_eq!(message, None);
        assert!(rack.latency_of(uid) >= PLUGIN_BLOCK);
        println!("latency in its slot: {}", rack.latency_of(uid));
        std::thread::sleep(std::time::Duration::from_millis(300));

        // Taken off the track, the instance waits until the engine has given it back.
        p.tracks[0].plugins.clear();
        assert!(rack.sync(&mut p, &mut engine, &mut message));
        assert_eq!(rack.latency_of(uid), 0);
        assert_eq!(rack.retiring.len(), 1);
        assert!(
            rack.kept_states.iter().any(|(u, _)| *u == uid),
            "Undo can bring back its settings"
        );
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !rack.retiring.is_empty() && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(20));
            // Sending is what takes garbage back from the engine.
            let _ = engine.send(Command::SetPlugin {
                track: 0,
                slot: 0,
                plugin: None,
            });
            assert!(rack.sync(&mut p, &mut engine, &mut message));
        }
        assert!(rack.retiring.is_empty(), "the instance was given back");

        // A plugin that will not load is marked and not tried again; the program goes on.
        let mut missing = info.clone();
        missing.path = PathBuf::from("no-such-plugin.clap");
        add_plugin(&mut p, 0, &missing).unwrap();
        assert!(rack.sync(&mut p, &mut engine, &mut message));
        assert!(message.take().is_some());
        assert!(p.tracks[0].plugins[0].failed.is_some());
        assert!(rack.sync(&mut p, &mut engine, &mut message));
        assert_eq!(message, None, "it is not tried again");
    }
}
