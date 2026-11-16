//! Keeps the plugins running in the engine in line with what the project says.
//!
//! The project holds plain data ([`PluginInsert`]), so Undo and duplicating a track work as for
//! anything else. Live instances cannot be cloned or rebuilt without losing their state, so each
//! has a `uid` and follows its insert when tracks and slots move.
use crate::model::{PluginInsert, Project, TrackKind};
use ovrtone_engine::{Command, Engine, PluginSlot, INSTRUMENT_SLOT, MAX_INSERTS, MAX_TRACKS};
use ovrtone_host::{
    load_plugin_for_audio, load_plugin_safely, AraRegion, HeaderClick, HeaderState, LoadedPlugin,
    PluginInfo,
};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Crashes of one plugin in a session before it stays off.
const MAX_RESTARTS: u32 = 3;

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
        // A track still called what it was made as ("Inst 3"), or after the instrument it had, takes
        // the new instrument's name. A name the user chose stays.
        let old = t.instrument_plugin.as_ref().map(|old| old.name.clone());
        if auto_named(&t.name) || old.as_deref() == Some(t.name.as_str()) {
            t.name = info.name.clone();
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
    // After everything on the strip, built-in effects included: they share one order.
    let mut new = PluginInsert::new(info);
    new.row = crate::model::next_row(t.inserts.len(), &t.plugins).min(usize::from(u8::MAX)) as u8;
    t.plugins.push(new);
    p.selected_track = track;
    Ok(())
}

/// An effect on track `track`, in insert row `row`.
pub fn add_effect_in_row(
    p: &mut Project,
    track: usize,
    info: &PluginInfo,
    row: usize,
) -> Result<(), String> {
    add_plugin(p, track, info)?;
    let t = &mut p.tracks[track];
    if let Some(new) = t.plugins.pop() {
        crate::model::place_plugin_in_row(0, &mut t.plugins, new, row);
    }
    Ok(())
}

/// An effect on the output, in insert row `row`.
pub fn add_master_effect(p: &mut Project, info: &PluginInfo, row: usize) -> Result<(), String> {
    if p.master_plugins.len() >= MAX_INSERTS {
        return Err("Stereo Out has no free insert slot".to_string());
    }
    let new = PluginInsert::new(info);
    crate::model::place_plugin_in_row(0, &mut p.master_plugins, new, row);
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
/// The song's tempo map and bar signatures, as an ARA plugin reads them (Melodyne puts its notes
/// and the MIDI it makes on this grid). Ramps are followed beat by beat.
fn ara_timeline(p: &Project) -> ovrtone_host::AraTimeline {
    let map = p.tempo_map();
    let mut beats = vec![0.0f64];
    let mut from = 0.0f64;
    let mut ramp = p.tempo_ramp;
    for c in &p.tempo_changes {
        if ramp {
            let mut b = from + 1.0;
            while b < c.beat && beats.len() < 4096 {
                beats.push(b);
                b += 1.0;
            }
        }
        beats.push(c.beat);
        from = c.beat;
        ramp = c.ramp;
    }
    // ARA wants two entries at least: one more, a bar on, holds the last tempo.
    beats.push(from + 4.0);
    let tempo = beats.iter().map(|b| (map.seconds_at(*b), *b)).collect();
    let mut bars = vec![(i32::from(p.sig.0), i32::from(p.sig.1), 0.0)];
    for c in &p.sig_changes {
        let at = p.bar_to_beats(f64::from(c.bar.saturating_sub(1)));
        bars.push((i32::from(c.num), i32::from(c.den), at));
    }
    ovrtone_host::AraTimeline { tempo, bars }
}

/// The audio regions a track plays, for a plugin that plays them itself.
fn ara_regions(p: &Project, t: usize) -> Vec<AraRegion> {
    let Some(track) = p.tracks.get(t) else {
        return Vec::new();
    };
    let map = p.tempo_map();
    track
        .regions
        .iter()
        .filter(|r| !r.params.mute)
        .filter_map(|r| {
            let audio = r.audio.as_ref()?;
            let start = map.seconds_at(p.region_start_beats(r));
            let end_bar = f64::from(r.start_bar) - 1.0 + f64::from(r.len_bars);
            let end = map.seconds_at(p.bar_to_beats(end_bar));
            let offset = f64::from(r.params.source_start);
            let in_file = if r.params.source_len > 0.0 {
                f64::from(r.params.source_len)
            } else {
                audio.file.duration_secs() - offset
            };
            let duration = in_file.min(end - start);
            (duration > 0.0).then(|| AraRegion {
                file: audio.file.clone(),
                name: r.name.clone(),
                start,
                offset,
                duration,
            })
        })
        .collect()
}

fn wanted_places(p: &Project) -> Vec<(u64, Place)> {
    let mut want = Vec::new();
    for (t, track) in p.tracks.iter().enumerate().take(MAX_TRACKS) {
        // A frozen track's plugins are let go, which is what freezing is for. Their settings are
        // kept, and they come back when it thaws.
        if track.frozen.is_some() {
            continue;
        }
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
    for (k, plugin) in p.master_plugins.iter().enumerate().take(MAX_INSERTS) {
        if plugin.failed.is_none() {
            want.push((plugin.uid, (ovrtone_engine::MASTER_STRIP, k as u8)));
        }
    }
    want
}

/// (uid, bypass) of each plugin, and its place.
fn wanted_bypass(p: &Project) -> Vec<((u64, bool), Place)> {
    let mut all = Vec::new();
    for (t, track) in p.tracks.iter().enumerate().take(MAX_TRACKS) {
        if track.frozen.is_some() {
            continue;
        }
        if let Some(plugin) = &track.instrument_plugin {
            all.push((
                (plugin.uid, plugin.bypass || track.off),
                (t as u16, INSTRUMENT_SLOT),
            ));
        }
        for (k, plugin) in track.plugins.iter().enumerate().take(MAX_INSERTS) {
            all.push((
                (plugin.uid, plugin.bypass || track.off),
                (t as u16, k as u8),
            ));
        }
    }
    for (k, plugin) in p.master_plugins.iter().enumerate().take(MAX_INSERTS) {
        all.push((
            (plugin.uid, plugin.bypass),
            (ovrtone_engine::MASTER_STRIP, k as u8),
        ));
    }
    all
}

/// (uid, side-chain source) of each plugin, and its place. A source the engine has no strip for, or
/// a frozen plugin, has none.
fn wanted_sides(p: &Project) -> Vec<(u64, Option<u16>, Place)> {
    let source = |i: &PluginInsert| {
        i.sidechain
            .filter(|s| *s < MAX_TRACKS && *s < p.tracks.len())
            .map(|s| s as u16)
    };
    let mut all = Vec::new();
    for (t, track) in p.tracks.iter().enumerate().take(MAX_TRACKS) {
        if track.frozen.is_some() {
            continue;
        }
        if let Some(plugin) = &track.instrument_plugin {
            all.push((plugin.uid, source(plugin), (t as u16, INSTRUMENT_SLOT)));
        }
        for (k, plugin) in track.plugins.iter().enumerate().take(MAX_INSERTS) {
            all.push((plugin.uid, source(plugin), (t as u16, k as u8)));
        }
    }
    for (k, plugin) in p.master_plugins.iter().enumerate().take(MAX_INSERTS) {
        all.push((
            plugin.uid,
            source(plugin),
            (ovrtone_engine::MASTER_STRIP, k as u8),
        ));
    }
    all
}

/// Copies of a track (Duplicate, a pasted strip) come with the uids of the originals. Each needs
/// an instance of its own, so all but the first get a new uid. Returns (original, copy).
pub fn give_copies_their_own_uids(p: &mut Project) -> Vec<(u64, u64)> {
    let mut seen = HashSet::new();
    let mut copies = Vec::new();
    for plugin in p.all_hosted_mut() {
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
    /// The file it was loaded from: a new one means loading again, keeping its settings.
    path: std::path::PathBuf,
    /// For a plugin that plays its track's audio: the project and track it was last told of.
    ara_seen: Option<(u64, u16)>,
    /// The plugin's identifier: where its editor window was is remembered per plugin.
    id: String,
    /// As last told to the engine.
    bypass: bool,
    /// Out of the way for Low Latency mode, as last told to the engine.
    low: bool,
    /// The side-chain source, as last told to the engine.
    side: Option<u16>,
    /// The preset last loaded or saved, for the editor window's bar.
    preset: Option<String>,
    loaded: Box<dyn LoadedPlugin>,
}

/// The name a preset goes by: its file's.
fn preset_name(file: &std::path::Path) -> Option<String> {
    Some(file.file_stem()?.to_str()?.to_string())
}

/// A/B compare of a plugin's settings: the set it is not using now. The first switch keeps A here
/// and carries on from the same settings as B, so that what is changed next is the difference.
#[derive(Default)]
struct Compare {
    other: Option<Arc<Vec<u8>>>,
    on_b: bool,
}

impl Compare {
    /// Switches sides, `now` being the settings in use. What to load, if there is anything yet.
    fn flip(&mut self, now: Arc<Vec<u8>>) -> Option<Arc<Vec<u8>>> {
        self.on_b = !self.on_b;
        self.other.replace(now)
    }
}

const WINDOWS_FILE: &str = "plugin-windows";
/// In Low Latency mode, plugins that delay the sound by this many samples or more step aside.
const LOW_LATENCY_LIMIT: usize = 256;

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
    /// How many times each plugin's process has ended this session.
    crashes: HashMap<u64, u32>,
    /// The engine runs plugins on several cores.
    rounds_sent: bool,
    live: HashMap<u64, Live>,
    /// Taken out of the engine, kept until the engine has given the audio half back: an instance
    /// dropped before that is never destroyed.
    retiring: Vec<Box<dyn LoadedPlugin>>,
    /// Where editor windows were. `None` until first needed.
    positions: Option<HashMap<String, (i32, i32)>>,
    /// The settings a plugin with this uid starts from, newest last.
    kept_states: Vec<(u64, Arc<Vec<u8>>)>,
    /// A plugin may have changed since the last save (it said so, or its editor was opened).
    touched: bool,
    /// What the engine was last told about plugins resting.
    keep_running: bool,
    /// Plugins loaded again for their side chain, whose editors were open: these open once more.
    reopen: Vec<u64>,
    /// The main window, as last told: what reopened editors stay above.
    owner: Option<isize>,
    /// A/B compare, by plugin. For this run: what goes in the project is the side in use.
    compare: HashMap<u64, Compare>,
    /// The scanned list as last seen (`plugin_trust::generation`).
    trust_seen: u64,
    /// Plugins loading in their helper processes, by uid.
    pending: HashMap<u64, ovrtone_host::PendingLoad>,
    /// Plugins whose slow load the user was told about.
    slow_told: Vec<u64>,
    /// Where each instance's window was in this project (from `PluginInsert::window_at`).
    instance_at: HashMap<u64, (i32, i32)>,
}

/// A load this long gets a word to the user: the plugin may be waiting on a window of its own.
const SLOW_LOAD: std::time::Duration = std::time::Duration::from_secs(120);

impl Rack {
    /// The delay of the plugin with this uid in its slot, 0 when it is not running.
    /// Writes what was moved in plugins' windows into their lanes, and shows in their windows what
    /// their lanes are doing. Once a frame.
    fn follow_lanes(&mut self, p: &mut Project) {
        let (beat, playing) = (p.position_beats, p.playing);
        for t in &mut p.tracks {
            let uids: Vec<u64> = t.hosted().map(|i| i.uid).collect();
            for uid in uids {
                let Some(live) = self.live.get_mut(&uid) else {
                    continue;
                };
                for (param, value) in live.loaded.take_edits() {
                    let target = crate::model::AutoTarget::Plugin { uid, param };
                    crate::automation::plugin_moved(t, target, value as f32, beat, playing);
                }
                let (editing, ended) = (live.loaded.editing(), live.loaded.take_edit_ended());
                crate::automation::plugin_gesture(t, uid, editing, ended, beat);
            }
            if !playing {
                continue;
            }
            for lane in &t.auto.plugin_lanes {
                let target = crate::model::AutoTarget::Plugin {
                    uid: lane.uid,
                    param: lane.param,
                };
                if !t.auto.follows(target) {
                    continue;
                }
                let value = ovrtone_engine::lane_value(&lane.points, beat);
                if let (Some(value), Some(live)) = (value, self.live.get_mut(&lane.uid)) {
                    live.loaded.show_param(lane.param, f64::from(value));
                }
            }
        }
    }

    /// Moves a parameter from a Smart Control. False when the engine could not take it now.
    pub fn smart_move(&mut self, uid: u64, param: u32, value: f64, engine: &mut Engine) -> bool {
        let Some(live) = self.live.get_mut(&uid) else {
            return true;
        };
        live.loaded.show_param(param, value);
        self.touched = true;
        engine.send(Command::SetPluginParam {
            track: live.place.0,
            slot: live.place.1,
            param,
            value,
        })
    }

    /// Where a parameter of a running plugin is now.
    /// Sets a parameter from a window of the program's own, which the plugin keeps to itself.
    pub fn show_param(&mut self, uid: u64, param: u32, value: f64) {
        if let Some(live) = self.live.get_mut(&uid) {
            live.loaded.show_param(param, value);
        }
    }

    pub fn param_value(&mut self, uid: u64, param: u32) -> Option<f64> {
        self.live.get_mut(&uid)?.loaded.param_value(param)
    }

    /// Writes plugin `uid`'s state to a preset file.
    pub fn save_preset(&mut self, uid: u64, file: &std::path::Path) -> Result<(), String> {
        let live = self.live.get_mut(&uid).ok_or("The plugin is not loaded")?;
        let state = live
            .loaded
            .save_state()
            .map_err(|e| e.to_string())?
            .ok_or("This plugin keeps no state to save")?;
        std::fs::write(file, state).map_err(|e| e.to_string())?;
        live.preset = preset_name(file);
        Ok(())
    }

    /// Puts a preset file's state into plugin `uid`.
    pub fn load_preset(&mut self, uid: u64, file: &std::path::Path) -> Result<(), String> {
        let state = std::fs::read(file).map_err(|e| e.to_string())?;
        let live = self.live.get_mut(&uid).ok_or("The plugin is not loaded")?;
        live.loaded.load_state(&state).map_err(|e| e.to_string())?;
        live.preset = preset_name(file);
        self.touched = true;
        Ok(())
    }

    /// Answers the buttons in the bars of the open editor windows, and keeps those bars current.
    fn answer_headers(&mut self, p: &mut Project, message: &mut Option<String>) {
        let open: Vec<u64> = self
            .live
            .iter()
            .filter(|(_, l)| l.loaded.editor_is_open())
            .map(|(uid, _)| *uid)
            .collect();
        for uid in open {
            let click = self
                .live
                .get_mut(&uid)
                .and_then(|l| l.loaded.take_header_click());
            let Some(plugin) = p.all_hosted_mut().find(|i| i.uid == uid) else {
                continue;
            };
            let answer = match click {
                Some(HeaderClick::Bypass) => {
                    plugin.bypass = !plugin.bypass;
                    Ok(())
                }
                Some(HeaderClick::Compare) => self.compare_flip(uid),
                Some(HeaderClick::CopyCompare) => self.compare_copy(uid),
                Some(HeaderClick::Presets) => {
                    let (id, name) = (plugin.id.clone(), plugin.name.clone());
                    self.preset_menu(uid, &id, &name)
                }
                None => Ok(()),
            };
            if let Err(e) = answer {
                *message = Some(e);
            }
            let bypass = plugin.bypass;
            let on_b = self.compare.get(&uid).is_some_and(|c| c.on_b);
            if let Some(live) = self.live.get_mut(&uid) {
                let preset = live.preset.clone();
                live.loaded.show_header(&HeaderState {
                    bypass,
                    on_b,
                    preset,
                });
            }
        }
    }

    /// The presets of plugin `uid` under its window's preset button, and what was chosen there.
    fn preset_menu(&mut self, uid: u64, id: &str, name: &str) -> Result<(), String> {
        let presets = crate::plugin_presets::list(id);
        let mut items = vec!["Save Preset…".to_string()];
        if !presets.is_empty() {
            items.push(String::new());
        }
        items.extend(presets.iter().map(|(name, _)| name.clone()));
        let live = self.live.get_mut(&uid).ok_or("The plugin is not loaded")?;
        match live.loaded.header_menu(&items) {
            Some(0) => match crate::plugin_presets::ask_where(id, name) {
                Some(file) => self.save_preset(uid, &file),
                None => Ok(()),
            },
            // The first preset comes after the save item and the separator.
            Some(at) => match presets.get(at.wrapping_sub(2)) {
                Some((_, file)) => self.load_preset(uid, file),
                None => Ok(()),
            },
            None => Ok(()),
        }
    }

    /// A/B compare: switches plugin `uid` to its other set of settings.
    pub fn compare_flip(&mut self, uid: u64) -> Result<(), String> {
        let live = self.live.get_mut(&uid).ok_or("The plugin is not loaded")?;
        let now = live
            .loaded
            .save_state()
            .map_err(|e| e.to_string())?
            .ok_or("This plugin keeps no settings to compare")?;
        let compare = self.compare.entry(uid).or_default();
        if let Some(other) = compare.flip(Arc::new(now)) {
            if let Err(e) = live.loaded.load_state(&other) {
                // It stays where it was.
                compare.flip(other);
                return Err(e.to_string());
            }
        }
        self.touched = true;
        Ok(())
    }

    /// A/B compare: the settings of plugin `uid` also become its other set.
    pub fn compare_copy(&mut self, uid: u64) -> Result<(), String> {
        let live = self.live.get_mut(&uid).ok_or("The plugin is not loaded")?;
        let now = live
            .loaded
            .save_state()
            .map_err(|e| e.to_string())?
            .ok_or("This plugin keeps no settings to compare")?;
        self.compare.entry(uid).or_default().other = Some(Arc::new(now));
        Ok(())
    }

    /// The plugins showing the B side.
    pub fn showing_b(&self) -> Vec<u64> {
        let on_b = self.compare.iter().filter(|(_, c)| c.on_b);
        on_b.map(|(uid, _)| *uid).collect()
    }

    /// Tells the engine to run plugins on several cores, the first time there are any.
    fn start_rounds(&mut self, engine: &mut Engine) {
        if self.rounds_sent {
            return;
        }
        let capacity = (MAX_INSERTS + 1) * MAX_TRACKS + MAX_INSERTS;
        if engine.send(Command::SetPluginRound(Engine::plugin_round(capacity))) {
            self.rounds_sent = true;
        }
    }

    pub fn latency_of(&self, uid: u64) -> usize {
        self.live.get(&uid).map_or(0, |l| {
            let own = if l.low { 0 } else { l.loaded.latency_samples() };
            own + ovrtone_engine::plugin_block()
        })
    }

    /// The delay of plugins in series: each one's own, and the blocks they are collected in.
    pub fn chain_latency<'a>(&self, plugins: impl Iterator<Item = &'a PluginInsert>) -> usize {
        let block = ovrtone_engine::plugin_block();
        let total: Vec<usize> = plugins.map(|i| self.latency_of(i.uid)).collect();
        let live = total.iter().filter(|l| **l >= block).count();
        let sum: usize = total.iter().sum();
        if ovrtone_engine::plugins_chained() && live > 1 {
            sum - (live - 1) * block
        } else {
            sum
        }
    }

    /// Runs what the plugins asked for on this thread.
    pub fn tick(&mut self) {
        for live in self.live.values_mut() {
            live.loaded.tick();
        }
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
        for plugin in p.all_hosted_mut() {
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
        self.owner = owner;
        let Some(id) = self.live.get(&uid).map(|l| l.id.clone()) else {
            // Still loading (Waves and other big plugins take seconds): its window opens by itself
            // when it is ready, rather than the click being lost.
            if !self.reopen.contains(&uid) {
                self.reopen.push(uid);
            }
            return Err(if self.pending.contains_key(&uid) {
                format!("{title} is still loading: its window opens when it is ready")
            } else {
                format!("{title} is not loaded yet: its window opens when it is")
            });
        };
        let at = self.instance_at.get(&uid).copied().or_else(|| self.positions().get(&id).copied());
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
    /// Tells plugins that play their track's audio about changes to the regions.
    /// The notes the ARA plugin on track `t` (Melodyne) found in the track's audio. Err with why not.
    pub fn ara_notes(&mut self, t: usize) -> Result<Vec<(f64, f64, f32, f32)>, String> {
        let live = self
            .live
            .values_mut()
            .find(|l| usize::from(l.place.0) == t && l.loaded.is_ara())
            .ok_or("Put Melodyne (or another ARA plugin) on this track first")?;
        live.loaded
            .ara_notes()
            .ok_or_else(|| "The plugin has no notes yet: let it finish analysing, then try again".to_string())
    }

    fn sync_ara(&mut self, p: &Project, message: &mut Option<String>) {
        let mut timeline = None;
        for live in self.live.values_mut() {
            if !live.loaded.is_ara() {
                continue;
            }
            // The song's tempo and signatures first, so the plugin's grid is the song's.
            let timeline = timeline.get_or_insert_with(|| ara_timeline(p));
            live.loaded.ara_timeline(timeline);
            let now = (p.song_rev, live.place.0);
            if live.ara_seen == Some(now) {
                continue;
            }
            match live
                .loaded
                .ara_sync(&ara_regions(p, usize::from(live.place.0)))
            {
                Ok(true) => live.ara_seen = Some(now),
                // Tried again on the next call.
                Ok(false) => {}
                Err(e) => {
                    live.ara_seen = Some(now);
                    *message = Some(e.to_string());
                }
            }
        }
    }

    /// How many of the project's plugins are still to load, and the name of the next.
    pub fn loads_left(&self, p: &mut Project) -> (usize, Option<String>) {
        let places: HashMap<u64, Place> = self.live.iter().map(|(u, l)| (*u, l.place)).collect();
        let mut left = 0;
        let mut next = None;
        for step in plan(&wanted_places(p), &places) {
            if let Step::Load(uid, _) = step {
                left += 1;
                if next.is_none() {
                    next = p.all_hosted_mut().find(|i| i.uid == uid).map(|i| i.name.clone());
                }
            }
        }
        (left, next)
    }

    pub fn sync(
        &mut self,
        p: &mut Project,
        engine: &mut Engine,
        message: &mut Option<String>,
    ) -> bool {
        self.follow_lanes(p);
        // The scan found more: plugins missing before are tried again.
        let trust = crate::plugin_trust::generation();
        if trust != self.trust_seen {
            self.trust_seen = trust;
            for i in p.all_hosted_mut() {
                if i.failed.as_deref() == Some(crate::plugin_trust::NOT_SCANNED) {
                    i.failed = None;
                }
            }
        }
        // A plugin removed while it was loading: its load is given up.
        if !self.pending.is_empty() {
            let present: std::collections::HashSet<u64> = p.all_hosted_mut().map(|i| i.uid).collect();
            self.pending.retain(|uid, _| present.contains(uid));
            self.slow_told.retain(|uid| present.contains(uid));
        }
        let mut closed = Vec::new();
        let mut closed_uids: Vec<(u64, (i32, i32))> = Vec::new();
        for (uid, live) in self.live.iter_mut() {
            live.loaded.tick();
            self.touched |= live.loaded.take_dirty();
            if let Some(why) = live.loaded.take_failure() {
                *message = Some(why);
            }
            if let Some(at) = live.loaded.take_closed_position() {
                closed.push((live.id.clone(), at));
                closed_uids.push((*uid, at));
            }
        }
        for (id, at) in closed {
            self.remember_position(&id, at);
        }
        // Each instance's own place, kept in the project, so its window opens there again.
        for (uid, at) in closed_uids {
            if let Some(plugin) = p.all_hosted_mut().find(|i| i.uid == uid) {
                if plugin.window_at != Some(at) {
                    plugin.window_at = Some(at);
                }
            }
        }
        self.instance_at.clear();
        for plugin in p.all_hosted_mut() {
            if let Some(at) = plugin.window_at {
                self.instance_at.insert(plugin.uid, at);
            }
        }
        self.restart_crashed(p, engine, message);
        self.retiring.retain(|r| !r.released());
        self.sync_ara(p, message);
        // A sound replaced on its slot: out with the old file. The settings are kept, for the load.
        let swapped: Vec<u64> = p
            .all_hosted_mut()
            .filter(|i| self.live.get(&i.uid).is_some_and(|l| l.path != i.path))
            .map(|i| i.uid)
            .collect();
        for uid in swapped {
            if !self.take_out(uid, engine) {
                return false;
            }
        }
        let places: HashMap<u64, Place> = self.live.iter().map(|(u, l)| (*u, l.place)).collect();
        let mut loaded_one = false;
        let mut more = false;
        for step in plan(&wanted_places(p), &places) {
            match step {
                Step::Remove(uid) => {
                    if !self.take_out(uid, engine) {
                        return false;
                    }
                    self.compare.remove(&uid);
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
                    self.start_rounds(engine);
                    if loaded_one {
                        more = true;
                        continue;
                    }
                    loaded_one = true;
                    if !self.load(p, uid, place, engine, message) {
                        return false;
                    }
                    // Still loading in its own process: the next one starts too, so a project's
                    // plugins load side by side rather than one after another.
                    if self.pending.contains_key(&uid) {
                        loaded_one = false;
                    }
                }
            }
        }
        for (uid, source, place) in wanted_sides(p) {
            let Some(live) = self.live.get_mut(&uid) else {
                continue;
            };
            // Its settings are kept, and the plan above brings it back next time.
            if live.loaded.side_needs_reload(source.is_some()) {
                if live.loaded.editor_is_open() {
                    self.reopen.push(uid);
                }
                if !self.take_out(uid, engine) {
                    return false;
                }
                more = true;
                continue;
            }
            if live.side != source {
                if !engine.send(Command::SetPluginSidechain {
                    track: place.0,
                    slot: place.1,
                    source,
                }) {
                    return false;
                }
                live.side = source;
            }
        }
        self.answer_headers(p, message);
        if self.keep_running != p.keep_plugins_running {
            if !engine.send(Command::SetPluginsMayRest(!p.keep_plugins_running)) {
                return false;
            }
            self.keep_running = p.keep_plugins_running;
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
            let low = p.low_latency && live.loaded.latency_samples() >= LOW_LATENCY_LIMIT;
            if live.low != low {
                if !engine.send(Command::SetPluginLowLatency {
                    track: place.0,
                    slot: place.1,
                    on: low,
                }) {
                    return false;
                }
                live.low = low;
            }
        }
        !more
    }

    /// Takes a plugin out of the engine. False when the engine could not take that now.
    fn take_out(&mut self, uid: u64, engine: &mut Engine) -> bool {
        let Some(place) = self.live.get(&uid).map(|l| l.place) else {
            return true;
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
        true
    }

    /// Plugins whose process ended come out of the engine. The next step loads them again from
    /// their last saved settings, unless they keep doing it.
    fn restart_crashed(
        &mut self,
        p: &mut Project,
        engine: &mut Engine,
        message: &mut Option<String>,
    ) {
        let crashed: Vec<u64> = self
            .live
            .iter()
            .filter(|(_, l)| l.loaded.crashed())
            .map(|(uid, _)| *uid)
            .collect();
        for uid in crashed {
            if !self.take_out(uid, engine) {
                return;
            }
            let times = self.crashes.entry(uid).or_insert(0);
            *times += 1;
            let times = *times;
            let Some(plugin) = p.all_hosted_mut().find(|i| i.uid == uid) else {
                continue;
            };
            if times >= MAX_RESTARTS {
                let why = "It keeps crashing, so it is off. The rest of the project is fine.";
                *message = Some(format!("{} crashed again. {why}", plugin.name));
                plugin.failed = Some(why.to_string());
            } else {
                *message = Some(format!(
                    "{} crashed. Ovrtone is fine: the plugin is being started again from its \
                     last saved settings.",
                    plugin.name
                ));
            }
        }
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
        let Some(plugin) = p.all_hosted_mut().find(|i| i.uid == uid) else {
            return true;
        };
        // Only plugin files the user's own scan found; one installed elsewhere loads from there.
        match crate::plugin_trust::allowed(&plugin.path, &plugin.id) {
            Ok(found) => {
                if found != plugin.path {
                    plugin.path = found;
                }
            }
            Err(why) => {
                *message = Some(format!("{}: {why}", plugin.name));
                plugin.failed = Some(why.to_string());
                return true;
            }
        }
        // Plugins load inside Ovrtone, which is quickest, unless the setting puts them all in
        // processes of their own. One that closed Ovrtone while loading runs in its own process from
        // then on, as Reaper and Bitwig do: its next crash is only a failed load.
        let crashed_before =
            !ovrtone_host::isolation_on() && crate::plugin_guard::is_blocked(&plugin.path);
        let guarded = !ovrtone_host::isolation_on() && !crashed_before;
        if crashed_before && !self.pending.contains_key(&uid) {
            *message = Some(format!(
                "{} closed Ovrtone while loading last time, so it now runs in its own process.",
                plugin.name
            ));
        }
        if guarded {
            crate::plugin_guard::before_load(&plugin.path);
        }
        let side = plugin.sidechain.is_some();
        let path = plugin.path.clone();
        let id = plugin.id.clone();
        let name = plugin.name.clone();
        // Only an audio track has regions for ARA to play: elsewhere it would be silent.
        let on_audio = place.1 != INSTRUMENT_SLOT
            && p.tracks
                .get(usize::from(place.0))
                .is_some_and(|t| t.kind == TrackKind::Audio);
        let ara = on_audio && crate::settings::use_ara() && !crashed_before;
        // In a helper process the load goes on there while the window keeps running: a slow
        // plugin, or one showing a licence window, no longer freezes Ovrtone.
        let in_helper =
            !guarded && !ara && ovrtone_host::PluginFormat::of(&path) != ovrtone_host::PluginFormat::Sampler;
        let result = if in_helper {
            let started = match self.pending.remove(&uid) {
                Some(pending) => Ok(pending),
                None => {
                    *message = Some(format!("Loading {name}..."));
                    ovrtone_host::start_isolated(&path, &id, engine.sample_rate(), side)
                }
            };
            match started {
                Err(e) => Err(e),
                Ok(mut pending) => match pending.poll() {
                    Some(done) => done,
                    None => {
                        let waited = pending.waited();
                        if waited >= SLOW_LOAD && !self.slow_told.contains(&uid) {
                            self.slow_told.push(uid);
                            *message = Some(format!(
                                "{} is still loading after {} minutes. It may be waiting on its \
                                 own window (a licence or sign-in): look for it on the taskbar. \
                                 Remove it from its slot to stop waiting.",
                                name,
                                waited.as_secs() / 60
                            ));
                        }
                        self.pending.insert(uid, pending);
                        return true;
                    }
                },
            }
        } else if ara {
            load_plugin_for_audio(&path, &id, engine.sample_rate(), side)
        } else {
            load_plugin_safely(&path, &id, engine.sample_rate(), side)
        };
        self.slow_told.retain(|u| *u != uid);
        let Some(plugin) = p.all_hosted_mut().find(|i| i.uid == uid) else {
            return true;
        };
        let (mut loaded, processor) = match result {
            Ok(both) => both,
            Err(e) => {
                if guarded {
                    crate::plugin_guard::after_load();
                }
                *message = Some(e.to_string());
                plugin.failed = Some(e.to_string());
                return true;
            }
        };
        plugin.params = Arc::new(loaded.params());
        plugin.side_offered = loaded.has_side_input();
        let kept = self.kept_states.iter().find(|(u, _)| *u == uid);
        let state = kept
            .map(|(_, s)| s.clone())
            .or_else(|| plugin.state.clone());
        // Newly inserted, so nothing kept: its window comes up, if that is the setting.
        if state.is_none() && self.owner.is_some() && crate::settings::open_on_insert() {
            self.reopen.push(uid);
        }
        if let Some(state) = state {
            if let Err(e) = loaded.load_state(&state) {
                // It runs from its defaults. Its old settings stay in the project.
                *message = Some(format!("{}: settings not restored: {e}", plugin.name));
            }
        }
        if guarded {
            crate::plugin_guard::after_load();
        }
        let sent = engine.send(Command::SetPlugin {
            track: place.0,
            slot: place.1,
            plugin: Some(Box::new(PluginSlot::new(processor).with_uid(uid))),
        });
        if sent {
            let id = plugin.id.clone();
            if let Some(at) = self.reopen.iter().position(|u| *u == uid) {
                self.reopen.remove(at);
                let name = plugin.name.clone();
                let place = self.instance_at.get(&uid).copied().or_else(|| self.positions().get(&id).copied());
                if let Err(e) = loaded.open_editor(&name, self.owner, place) {
                    *message = Some(e.to_string());
                }
            }
            self.live.insert(
                uid,
                Live {
                    place,
                    id,
                    // The slot starts running; the next `sync` tells it if it should not be.
                    bypass: false,
                    low: false,
                    side: None,
                    preset: None,
                    ara_seen: None,
                    path: path.clone(),
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
    use crate::model::Track;
    use ovrtone_engine::PLUGIN_BLOCK;
    use std::path::PathBuf;

    fn effect(name: &str) -> PluginInfo {
        PluginInfo {
            id: name.to_string(),
            name: name.to_string(),
            vendor: String::new(),
            version: String::new(),
            format: ovrtone_host::PluginFormat::Clap,
            path: format!("{name}.clap").into(),
            is_instrument: false,
        }
    }

    #[test]
    fn an_effect_goes_in_the_slot_it_was_put_in_and_the_order_follows_the_rows() {
        let mut p = Project::empty();
        let t = p.add_track(TrackKind::Audio);
        add_effect_in_row(&mut p, t, &effect("Third"), 5).unwrap();
        add_effect_in_row(&mut p, t, &effect("First"), 1).unwrap();
        add_effect_in_row(&mut p, t, &effect("Second"), 3).unwrap();
        let names: Vec<_> = p.tracks[t]
            .plugins
            .iter()
            .map(|x| x.name.as_str())
            .collect();
        assert_eq!(names, ["First", "Second", "Third"]);
        assert_eq!(
            crate::model::plugin_rows(0, &p.tracks[t].plugins),
            [1, 3, 5]
        );
        // One added the old way goes after the rest.
        add_plugin(&mut p, t, &effect("Last")).unwrap();
        assert_eq!(
            crate::model::plugin_rows(0, &p.tracks[t].plugins),
            [1, 3, 5, 6]
        );
    }

    #[test]
    fn plugins_on_the_output_load_at_their_own_place() {
        let mut p = Project::empty();
        add_master_effect(&mut p, &effect("Glue"), 2).unwrap();
        assert_eq!(crate::model::plugin_rows(0, &p.master_plugins), [2]);
        let want = wanted_places(&p);
        assert!(want
            .iter()
            .any(|(_, place)| *place == (ovrtone_engine::MASTER_STRIP, 0)));
        for _ in 1..ovrtone_engine::MAX_INSERTS {
            add_master_effect(&mut p, &effect("More"), 7).unwrap();
        }
        assert!(add_master_effect(&mut p, &effect("Too many"), 7).is_err());
    }

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
    fn a_side_chain_is_asked_for_only_where_there_is_a_track_for_it() {
        let mut p = Project::empty();
        let a = p.add_track(TrackKind::Audio);
        let b = p.add_track(TrackKind::Audio);
        add_plugin(&mut p, a, &effect("Comp")).unwrap();
        add_master_effect(&mut p, &effect("Duck"), 0).unwrap();
        p.tracks[a].plugins[0].sidechain = Some(b);
        p.master_plugins[0].sidechain = Some(9999);
        let sides = wanted_sides(&p);
        assert_eq!(sides[0].1, Some(b as u16));
        assert_eq!(sides[0].2, (a as u16, 0));
        assert_eq!(sides[1].1, None, "there is no such track");
        // Frozen, the plugin is not running.
        p.tracks[a].frozen = Some(Box::new(
            p.tracks[0].regions.first().cloned().unwrap_or_else(|| {
                let mut demo = Project::demo();
                demo.tracks
                    .iter_mut()
                    .find_map(|t| t.regions.pop())
                    .unwrap()
            }),
        ));
        assert_eq!(wanted_sides(&p).len(), 1);
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
            format: ovrtone_host::PluginFormat::Clap,
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
    /// for. `cargo test -p ovrtone-app rack_runs_a_real -- --ignored --nocapture`, with
    /// `OVRTONE_TEST_CLAP` naming a `.clap` file, or else the first plugin installed.
    #[test]
    #[ignore = "needs a sound device and an installed CLAP plugin"]
    fn rack_runs_a_real_plugin_in_the_engine() {
        use ovrtone_host::{clap_folders, find_clap_files, scan_plugin_file};
        let file = std::env::var_os("OVRTONE_TEST_CLAP")
            .map(PathBuf::from)
            .or_else(|| find_clap_files(&clap_folders()).into_iter().next())
            .unwrap();
        let info = scan_plugin_file(&file).unwrap().remove(0);
        println!("{} / {}", info.vendor, info.name);
        let mut p = Project::empty();
        p.tracks.push(p.tracks.first().cloned().unwrap_or_else(|| {
            let mut demo = Project::demo();
            demo.tracks.remove(0)
        }));
        p.tracks.truncate(1);
        p.tracks[0].kind = if info.is_instrument {
            TrackKind::Instrument
        } else {
            TrackKind::Audio
        };
        p.tracks[0].inserts.clear();
        p.tracks[0].plugins.clear();
        p.tracks[0].instrument_plugin = None;
        add_plugin(&mut p, 0, &info).unwrap();
        let uid = p.tracks[0].hosted().next().unwrap().uid;

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
        p.tracks[0].instrument_plugin = None;
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
        assert!(p.tracks[0].hosted().next().unwrap().failed.is_some());
        assert!(rack.sync(&mut p, &mut engine, &mut message));
        assert_eq!(message, None, "it is not tried again");
    }
}

/// Whether a track still has the name it was made with: "Inst 3", "Drummer 1", "Instrument" and
/// the like.
fn auto_named(name: &str) -> bool {
    let base = name.trim_end_matches(|c: char| c.is_ascii_digit() || c == ' ');
    matches!(base, "Inst" | "Instrument" | "Drummer" | "Software Instrument" | "")
}

#[cfg(test)]
mod naming_tests {
    use super::auto_named;

    #[test]
    fn a_track_with_its_first_name_takes_its_instrument_name() {
        assert!(auto_named("Inst 3"));
        assert!(auto_named("Instrument"));
        assert!(auto_named("Drummer 12"));
        assert!(!auto_named("Lead Vox"));
        assert!(!auto_named("Keys 2"));
    }
}
