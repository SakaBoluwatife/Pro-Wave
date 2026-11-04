//! Keeps the plugins running in the engine in line with what the project says.
//!
//! The project holds plain data ([`PluginInsert`]), so Undo and duplicating a track work as for
//! anything else. Live instances cannot be cloned or rebuilt without losing their state, so each
//! has a `uid` and follows its insert when tracks and slots move.
use crate::model::{PluginInsert, Project, TrackKind};
use prowave_engine::{Command, Engine, PluginSlot, MAX_INSERTS, MAX_TRACKS, PLUGIN_BLOCK};
use prowave_host::{load_clap, LoadedClap, PluginInfo};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

/// A plugin's place in the engine: track and insert slot.
pub type Place = (u16, u8);

static NEXT_UID: AtomicU64 = AtomicU64::new(1);

pub fn new_uid() -> u64 {
    NEXT_UID.fetch_add(1, Ordering::Relaxed)
}

/// What a click or drop on the Plugins tab asks for: `info` on track `track`.
pub fn add_effect(p: &mut Project, track: usize, info: &PluginInfo) -> Result<(), String> {
    if info.is_instrument {
        return Err(format!(
            "{} is an instrument; instrument plugins are not hosted yet",
            info.name
        ));
    }
    let Some(t) = p.tracks.get_mut(track) else {
        return Err("Select a track first".to_string());
    };
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
        for (k, plugin) in track.plugins.iter().enumerate().take(MAX_INSERTS) {
            if plugin.failed.is_none() {
                want.push((plugin.uid, (t as u16, k as u8)));
            }
        }
    }
    want
}

/// Copies of a track (Duplicate, a pasted strip) come with the uids of the originals. Each needs
/// an instance of its own, so all but the first get a new uid.
pub fn give_copies_their_own_uids(p: &mut Project) {
    let mut seen = HashSet::new();
    for plugin in p.tracks.iter_mut().flat_map(|t| t.plugins.iter_mut()) {
        if !seen.insert(plugin.uid) {
            plugin.uid = new_uid();
        }
    }
}

struct Live {
    place: Place,
    loaded: LoadedClap,
}

/// The plugins running in one engine. Dropped after the engine, so instances go last.
#[derive(Default)]
pub struct Rack {
    live: HashMap<u64, Live>,
    /// Taken out of the engine, kept until the engine has given the audio half back: an instance
    /// dropped before that is never destroyed.
    retiring: Vec<LoadedClap>,
}

impl Rack {
    /// The delay of the plugin with this uid in its slot, 0 when it is not running.
    pub fn latency_of(&self, uid: u64) -> usize {
        self.live
            .get(&uid)
            .map_or(0, |l| l.loaded.latency_samples() + PLUGIN_BLOCK)
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
        for live in self.live.values_mut() {
            live.loaded.tick();
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
                    if let Some(live) = self.live.remove(&uid) {
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
            .flat_map(|t| t.plugins.iter_mut())
            .find(|i| i.uid == uid)
        else {
            return true;
        };
        let (loaded, processor) = match load_clap(&plugin.path, &plugin.id, engine.sample_rate()) {
            Ok(both) => both,
            Err(e) => {
                *message = Some(e.to_string());
                plugin.failed = Some(e.to_string());
                return true;
            }
        };
        let sent = engine.send(Command::SetPlugin {
            track: place.0,
            slot: place.1,
            plugin: Some(Box::new(PluginSlot::new(processor))),
        });
        if sent {
            self.live.insert(uid, Live { place, loaded });
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
    fn effects_go_on_tracks_with_room_and_a_plugin_that_failed_is_not_retried() {
        let mut p = Project::demo();
        let track = p
            .tracks
            .iter()
            .position(|t| t.kind == TrackKind::Audio || t.kind == TrackKind::Instrument)
            .unwrap();
        p.tracks[track].inserts.clear();
        add_effect(&mut p, track, &info(false)).unwrap();
        assert_eq!(p.tracks[track].plugins[0].name, "Test");
        assert_eq!(p.selected_track, track);
        assert!(add_effect(&mut p, track, &info(true)).is_err());
        assert!(add_effect(&mut p, 9999, &info(false)).is_err());
        for _ in 1..MAX_INSERTS {
            add_effect(&mut p, track, &info(false)).unwrap();
        }
        assert!(add_effect(&mut p, track, &info(false)).is_err(), "full");

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
        add_effect(&mut p, track, &info(false)).unwrap();
        let copy = p.tracks[track].clone();
        p.tracks.push(copy);
        give_copies_their_own_uids(&mut p);
        let last = p.tracks.len() - 1;
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
        add_effect(&mut p, 0, &info).unwrap();
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
        add_effect(&mut p, 0, &missing).unwrap();
        assert!(rack.sync(&mut p, &mut engine, &mut message));
        assert!(message.take().is_some());
        assert!(p.tracks[0].plugins[0].failed.is_some());
        assert!(rack.sync(&mut p, &mut engine, &mut message));
        assert_eq!(message, None, "it is not tried again");
    }
}
