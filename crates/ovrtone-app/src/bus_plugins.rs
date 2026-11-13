//! Plugins on the spatial buses (Logic: plugins on a surround or Atmos bus). A VST3 effect that takes
//! the bus's speaker layout gets every channel of it (native); any other runs as linked stereo copies,
//! one per pair of the bus's channels (multi-mono, as Pro Tools does), the copies following the first
//! one's window. They run in the program itself, not in a helper process.

use crate::model::Project;
use ovrtone_engine::{BusPlugin, Command, Engine, SpeakerLayout};
use ovrtone_host::LoadedPlugin;
use std::collections::HashMap;

/// A plugin running on a bus: the UI halves of its instances (the first has the window).
struct Live {
    bus: usize,
    slot: usize,
    units: Vec<Box<dyn LoadedPlugin>>,
    native: bool,
}

/// The bus plugins this program is running, kept in line with the project.
#[derive(Default)]
pub struct BusPlugins {
    live: HashMap<u64, Live>,
    /// What each bus was last given: its layout and its plugins.
    sent: Vec<Option<(SpeakerLayout, Vec<u64>)>>,
    /// The buses' layouts as last seen: when they change, everything is loaded again.
    layouts: Vec<SpeakerLayout>,
    bypass: HashMap<u64, bool>,
    /// Settings of plugins let go of to be loaded again, by uid: newer than the project's.
    kept: HashMap<u64, std::sync::Arc<Vec<u8>>>,
    /// Why a plugin would not load, by uid.
    pub failed: HashMap<u64, String>,
}

impl BusPlugins {
    /// Loads what the buses gained, lets go of what they lost, and passes the first copy's moves on
    /// to the others. Once a frame, after the spatial renderer is sent.
    pub fn sync(&mut self, p: &Project, engine: &mut Engine, message: &mut Option<String>) {
        let layouts: Vec<SpeakerLayout> = p.spatial_buses.iter().map(|b| b.layout).collect();
        if layouts != self.layouts {
            self.keep_all();
            self.live.clear();
            self.sent.clear();
            self.layouts = layouts;
        }
        self.sent.resize(p.spatial_buses.len(), None);
        let rate = engine.sample_rate();
        for k in 0..p.spatial_buses.len().min(ovrtone_engine::MAX_SPATIAL_BUSES) {
            let layout = p.spatial_buses[k].layout;
            let uids: Vec<u64> = p.spatial_buses[k].plugins.iter().map(|i| i.uid).collect();
            if self.sent[k].as_ref() == Some(&(layout, uids.clone())) {
                continue;
            }
            // What was running keeps its settings.
            self.keep_bus(k);
            self.live.retain(|_, l| l.bus != k);
            let bus_name = p.spatial_buses[k].name.clone();
            let pairs = layout.device_order().len().div_ceil(2);
            let mut list = Vec::new();
            for (slot, ins) in p.spatial_buses[k].plugins.iter().enumerate() {
                match ovrtone_host::load_for_bus(&ins.path, &ins.id, rate, layout.name(), pairs) {
                    Ok((loaded, native)) => {
                        let (mut units, procs): (Vec<_>, Vec<_>) = loaded.into_iter().unzip();
                        if let Some(state) = self.kept.remove(&ins.uid).or_else(|| ins.state.clone()) {
                            for u in &mut units {
                                let _ = u.load_state(&state);
                            }
                        }
                        self.failed.remove(&ins.uid);
                        list.push(BusPlugin::new(procs, native, ins.bypass));
                        self.bypass.insert(ins.uid, ins.bypass);
                        self.live.insert(ins.uid, Live { bus: k, slot, units, native });
                    }
                    Err(e) => {
                        *message = Some(format!("{} on {bus_name}: {}", ins.name, e.0));
                        self.failed.insert(ins.uid, e.0);
                        // Kept, silent, so the slots after it stay where they are.
                        list.push(BusPlugin::new(Vec::new(), false, true));
                    }
                }
            }
            if !engine.send(Command::SetBusPlugins {
                bus: k as u8,
                plugins: Box::new(list),
            }) {
                return;
            }
            self.sent[k] = Some((layout, uids));
        }
        for (k, b) in p.spatial_buses.iter().enumerate() {
            for (slot, ins) in b.plugins.iter().enumerate() {
                if self.bypass.get(&ins.uid) == Some(&ins.bypass) || !self.live.contains_key(&ins.uid) {
                    continue;
                }
                if engine.send(Command::SetBusPluginBypass {
                    bus: k as u8,
                    slot: slot as u8,
                    bypass: ins.bypass,
                }) {
                    self.bypass.insert(ins.uid, ins.bypass);
                }
            }
        }
        // The first copy's window moves the linked copies too.
        for l in self.live.values_mut() {
            for u in &mut l.units {
                u.tick();
            }
            let Some(first) = l.units.first_mut() else { continue };
            let edits = first.take_edits();
            if l.native || l.units.len() < 2 {
                continue;
            }
            for (id, value) in edits {
                let _ = engine.send(Command::SetBusPluginParam {
                    bus: l.bus as u8,
                    slot: l.slot as u8,
                    id,
                    value,
                });
            }
        }
    }

    /// The settings of every running bus plugin into the project, for saving.
    pub fn collect_states(&mut self, p: &mut Project) {
        for k in 0..p.spatial_buses.len() {
            self.collect_bus(p, k);
        }
    }

    /// Keeps the settings of the plugins of every bus, to load them again.
    fn keep_all(&mut self) {
        let buses: Vec<usize> = self.live.values().map(|l| l.bus).collect();
        for k in buses {
            self.keep_bus(k);
        }
    }

    fn keep_bus(&mut self, bus: usize) {
        for (uid, l) in self.live.iter_mut().filter(|(_, l)| l.bus == bus) {
            if let Some(Ok(Some(bytes))) = l.units.first_mut().map(|u| u.save_state()) {
                self.kept.insert(*uid, std::sync::Arc::new(bytes));
            }
        }
    }

    fn collect_bus(&mut self, p: &mut Project, bus: usize) {
        for l in self.live.values_mut().filter(|l| l.bus == bus) {
            let Some(ins) = p.spatial_buses.get_mut(bus).and_then(|b| b.plugins.get_mut(l.slot)) else {
                continue;
            };
            if let Some(Ok(Some(bytes))) = l.units.first_mut().map(|u| u.save_state()) {
                ins.state = Some(std::sync::Arc::new(bytes));
            }
        }
    }

    /// Opens the window of bus plugin `uid` (its first copy, which moves the others).
    pub fn open_editor(&mut self, uid: u64, title: &str, owner: Option<isize>) -> Result<(), String> {
        let unit = self
            .live
            .get_mut(&uid)
            .and_then(|l| l.units.first_mut())
            .ok_or_else(|| format!("{title} is not running"))?;
        unit.open_editor(title, owner, None).map_err(|e| e.to_string())
    }
}
