//! Control surfaces in Mackie Control (MCU) mode: Mackie MCU, Behringer X-Touch, iCON, PreSonus
//! FaderPort in MCU mode, and most keyboards with a DAW mode. Eight channels at a time:
//! - faders (motorised ones follow the tracks and their automation), V-Pots for pan;
//! - Rec, Solo, Mute and Select per channel, with their lights;
//! - bank and channel buttons to move the eight along the tracks;
//! - transport: play, stop, record, rewind, fast forward, cycle, and the jog wheel;
//! - the display shows the eight track names, the meters their levels.
//!
//! The surface talks on its own MIDI port pair (chosen in Settings > General); that port is kept
//! out of note input, as its buttons send notes that are not music. HUI is not spoken yet.

use crate::model::Project;
use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use std::sync::mpsc::{sync_channel, Receiver};

const FILE: &str = "control-surface";

// Notes of the MCU's buttons (Logic Control layout).
const REC: u8 = 0x00;
const SOLO: u8 = 0x08;
const MUTE: u8 = 0x10;
const SELECT: u8 = 0x18;
const BANK_LEFT: u8 = 0x2E;
const BANK_RIGHT: u8 = 0x2F;
const CHANNEL_LEFT: u8 = 0x30;
const CHANNEL_RIGHT: u8 = 0x31;
const CYCLE: u8 = 0x56;
const REWIND: u8 = 0x5B;
const FORWARD: u8 = 0x5C;
const STOP: u8 = 0x5D;
const PLAY: u8 = 0x5E;
const RECORD: u8 = 0x5F;
/// The faders' touch sensors: a touched fader is not moved by the program under the hand.
const TOUCH: u8 = 0x68;
/// V-Pots send controllers 16-23, the jog wheel 60; both relative.
const VPOT: u8 = 0x10;
const JOG: u8 = 0x3C;

/// The port the user chose, kept on this computer.
pub fn chosen() -> Option<String> {
    crate::recent_files::config_dir()
        .filter(|_| !cfg!(test))
        .and_then(|d| std::fs::read_to_string(d.join(FILE)).ok())
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

pub fn choose(port: Option<&str>) {
    if let Some(dir) = crate::recent_files::config_dir().filter(|_| !cfg!(test)) {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join(FILE), port.unwrap_or(""));
    }
}

/// The surface, while one is connected.
pub struct Surface {
    pub port: String,
    _input: MidiInputConnection<()>,
    output: MidiOutputConnection,
    rx: Receiver<Vec<u8>>,
    /// The first track of the eight.
    bank: usize,
    /// What the surface shows now, so only changes are sent.
    faders: [Option<u16>; 8],
    lights: std::collections::HashMap<u8, bool>,
    text: String,
    touched: [bool; 8],
    /// Speaks HUI (Pro Tools' protocol) rather than Mackie Control.
    pub hui: bool,
    /// HUI: the zone the last button message named, each fader's high byte, the lights, the ping.
    hui_zone: u8,
    hui_msb: [u8; 8],
    hui_lights: std::collections::HashMap<(u8, u8), bool>,
    hui_names: [String; 8],
    last_ping: Option<std::time::Instant>,
}

impl Surface {
    /// Opens `port`: its input by that name, and the output whose name matches it best.
    pub fn open(port: &str) -> Result<Self, String> {
        let input = MidiInput::new("Ovrtone surface").map_err(|e| e.to_string())?;
        let in_port = input
            .ports()
            .into_iter()
            .find(|p| input.port_name(p).ok().as_deref() == Some(port))
            .ok_or_else(|| format!("{port} is not connected"))?;
        let output = MidiOutput::new("Ovrtone surface").map_err(|e| e.to_string())?;
        let out_ports = output.ports();
        let out_port = out_ports
            .iter()
            .find(|p| output.port_name(p).ok().as_deref() == Some(port))
            .or_else(|| {
                let lower = port.to_lowercase();
                out_ports.iter().find(|p| {
                    output.port_name(p).ok().is_some_and(|n| {
                        let n = n.to_lowercase();
                        n.contains(&lower) || lower.contains(&n)
                    })
                })
            })
            .ok_or_else(|| format!("{port} has no output to answer on"))?
            .clone();
        // Bounded: a surface that floods the port cannot fill the memory.
        let (tx, rx) = sync_channel(4096);
        let input = input
            .connect(
                &in_port,
                "ovrtone-surface-in",
                move |_, msg, _| {
                    let _ = tx.try_send(msg.to_vec());
                },
                (),
            )
            .map_err(|e| e.to_string())?;
        let output = output
            .connect(&out_port, "ovrtone-surface-out")
            .map_err(|e| e.to_string())?;
        Ok(Self {
            port: port.to_string(),
            _input: input,
            output,
            rx,
            bank: 0,
            faders: [None; 8],
            lights: std::collections::HashMap::new(),
            text: String::new(),
            touched: [false; 8],
            hui: hui_chosen(),
            hui_zone: 0,
            hui_msb: [0; 8],
            hui_lights: std::collections::HashMap::new(),
            hui_names: Default::default(),
            last_ping: None,
        })
    }

    fn send(&mut self, bytes: &[u8]) {
        let _ = self.output.send(bytes);
    }

    /// Once a frame: what the surface did goes to the project, and what changed goes back to it.
    /// `meters` is each track's level, 0..=1.
    pub fn update(&mut self, p: &mut Project, meters: &[f32]) -> bool {
        let mut changed = false;
        while let Ok(msg) = self.rx.try_recv() {
            changed |= if self.hui { self.take_hui(p, &msg) } else { self.take(p, &msg) };
        }
        if self.hui {
            self.show_hui(p);
        } else {
            self.show(p, meters);
        }
        changed
    }

    /// One message from the surface. True if the project changed.
    fn take(&mut self, p: &mut Project, msg: &[u8]) -> bool {
        let tracks = p.tracks.len();
        match *msg {
            // A fader: pitch bend on its channel, 14 bits.
            [status, lo, hi] if status & 0xF0 == 0xE0 => {
                let ch = usize::from(status & 0x0F);
                let t = self.bank + ch;
                if ch < 8 && t < tracks {
                    let v = u16::from(lo) | (u16::from(hi) << 7);
                    let frac = f32::from(v) / 16383.0;
                    p.tracks[t].volume_db = crate::mixer::frac_to_db(frac).clamp(-60.0, 6.0);
                    self.faders[ch] = Some(v);
                    return true;
                }
                false
            }
            // A V-Pot or the jog wheel: relative, bit 6 for the direction.
            [0xB0, cc, value] => {
                let steps = f32::from(value & 0x3F) * if value & 0x40 != 0 { -1.0 } else { 1.0 };
                if (VPOT..VPOT + 8).contains(&cc) {
                    let t = self.bank + usize::from(cc - VPOT);
                    if t < tracks {
                        p.tracks[t].pan = (p.tracks[t].pan + steps / 64.0).clamp(-1.0, 1.0);
                        return true;
                    }
                } else if cc == JOG {
                    // A beat a step, stopped or playing.
                    p.position_beats = (p.position_beats + f64::from(steps)).max(0.0);
                    return true;
                }
                false
            }
            // A button: note on, velocity 127 down, 0 up.
            [0x90, note, velocity] => {
                if (TOUCH..TOUCH + 8).contains(&note) {
                    self.touched[usize::from(note - TOUCH)] = velocity > 0;
                    return false;
                }
                if velocity == 0 {
                    return false;
                }
                self.press(p, note)
            }
            _ => false,
        }
    }

    fn press(&mut self, p: &mut Project, note: u8) -> bool {
        let tracks = p.tracks.len();
        let on = |base: u8| (base..base + 8).contains(&note).then(|| usize::from(note - base));
        let track = |k: usize| Some(self.bank + k).filter(|t| *t < tracks);
        if let Some(t) = on(REC).and_then(track) {
            p.tracks[t].rec = !p.tracks[t].rec;
        } else if let Some(t) = on(SOLO).and_then(track) {
            p.tracks[t].solo = !p.tracks[t].solo;
        } else if let Some(t) = on(MUTE).and_then(track) {
            p.tracks[t].mute = !p.tracks[t].mute;
        } else if let Some(t) = on(SELECT).and_then(track) {
            p.selected_track = t;
        } else {
            match note {
                BANK_LEFT => self.move_bank(-8, tracks),
                BANK_RIGHT => self.move_bank(8, tracks),
                CHANNEL_LEFT => self.move_bank(-1, tracks),
                CHANNEL_RIGHT => self.move_bank(1, tracks),
                PLAY => p.playing = true,
                STOP => {
                    if !p.playing {
                        // Stop when stopped goes to the start, as on most DAWs.
                        p.position_beats = 0.0;
                    }
                    p.playing = false;
                }
                RECORD => p.recording = !p.recording,
                CYCLE => p.cycle = !p.cycle,
                REWIND | FORWARD => {
                    let bar = p.beats_to_bar(p.position_beats).floor();
                    let to = if note == REWIND { (bar - 1.0).max(0.0) } else { bar + 1.0 };
                    p.position_beats = p.bar_to_beats(to);
                }
                _ => return false,
            }
        }
        true
    }

    fn move_bank(&mut self, by: isize, tracks: usize) {
        let last = tracks.saturating_sub(1) as isize;
        self.bank = (self.bank as isize + by).clamp(0, last.max(0)) as usize;
        // All shown again for the new tracks.
        self.faders = [None; 8];
        self.lights.clear();
        self.text.clear();
    }

    /// Sends the surface what changed: faders, lights, names, meters.
    fn show(&mut self, p: &Project, meters: &[f32]) {
        for ch in 0..8usize {
            let t = self.bank + ch;
            let track = p.tracks.get(t);
            // Motorised faders follow, unless a hand is on them.
            if !self.touched[ch] {
                let v = track.map_or(0, |tr| {
                    (crate::mixer::db_to_frac(tr.volume_db).clamp(0.0, 1.0) * 16383.0).round() as u16
                });
                if self.faders[ch] != Some(v) {
                    self.faders[ch] = Some(v);
                    self.send(&[0xE0 | ch as u8, (v & 0x7F) as u8, (v >> 7) as u8]);
                }
            }
            let (rec, solo, mute, sel) = track.map_or((false, false, false, false), |tr| {
                (tr.rec, tr.solo, tr.mute, p.selected_track == t)
            });
            for (base, lit) in [(REC, rec), (SOLO, solo), (MUTE, mute), (SELECT, sel)] {
                self.light(base + ch as u8, lit);
            }
            // Meters: channel pressure, the channel in the high bits, 0-12 in the low.
            let level = meters.get(t).copied().unwrap_or(0.0).clamp(0.0, 1.0);
            let step = (level * 12.0).round() as u8;
            self.send(&[0xD0, ((ch as u8) << 4) | step]);
        }
        self.light(PLAY, p.playing);
        self.light(STOP, !p.playing);
        self.light(RECORD, p.recording);
        self.light(CYCLE, p.cycle);
        // The display's top line: seven letters for each of the eight tracks.
        let text: String = (0..8)
            .map(|ch| {
                let name = p.tracks.get(self.bank + ch).map_or("", |t| t.name.as_str());
                let short: String = name.chars().filter(char::is_ascii).take(7).collect();
                format!("{short:<7}")
            })
            .collect();
        if text != self.text {
            self.text = text.clone();
            let mut sysex = vec![0xF0, 0x00, 0x00, 0x66, 0x14, 0x12, 0x00];
            sysex.extend(text.bytes().map(|b| b & 0x7F));
            sysex.push(0xF7);
            self.send(&sysex);
        }
    }

    fn light(&mut self, note: u8, lit: bool) {
        if self.lights.get(&note) != Some(&lit) {
            self.lights.insert(note, lit);
            self.send(&[0x90, note, if lit { 0x7F } else { 0x00 }]);
        }
    }
}

const HUI_FILE: &str = "control-surface-hui";

/// The surface speaks HUI (Pro Tools' protocol; older Mackie, many controllers' "HUI mode").
pub fn hui_chosen() -> bool {
    crate::recent_files::config_dir()
        .filter(|_| !cfg!(test))
        .is_some_and(|d| d.join(HUI_FILE).exists())
}

pub fn set_hui(on: bool) {
    if let Some(dir) = crate::recent_files::config_dir().filter(|_| !cfg!(test)) {
        let file = dir.join(HUI_FILE);
        if on {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(file, "1");
        } else {
            let _ = std::fs::remove_file(file);
        }
    }
}

// HUI: a button is a zone and a port. Channel strips are zones 0-7.
const HUI_TOUCH: u8 = 0;
const HUI_SELECT: u8 = 1;
const HUI_MUTE: u8 = 2;
const HUI_SOLO: u8 = 3;
const HUI_REC: u8 = 7;
/// Zone of the bank and channel buttons: channel left, bank left, channel right, bank right.
const HUI_SCROLL: u8 = 0x0A;
/// Transport: rewind 1, fast forward 2, stop 3, play 4, record 5.
const HUI_TRANSPORT: u8 = 0x0E;
/// Loop is port 3 of this zone.
const HUI_TRANSPORT_MORE: u8 = 0x0F;

/// A relative turn of a HUI V-Pot or the jog wheel: 7 bits, two's complement.
fn hui_steps(value: u8) -> f32 {
    let v = i32::from(value & 0x7F);
    (if v >= 64 { v - 128 } else { v }) as f32
}

impl Surface {
    /// One HUI message from the surface. True if the project changed. The layout follows the
    /// published HUI protocol; it has not been tried on a HUI device here.
    fn take_hui(&mut self, p: &mut Project, msg: &[u8]) -> bool {
        let tracks = p.tracks.len();
        let [0xB0, cc, value] = *msg else {
            return false;
        };
        match cc {
            // Fader: the high 7 bits, then the low 7 bits.
            0x00..=0x07 => {
                self.hui_msb[usize::from(cc)] = value;
                false
            }
            0x20..=0x27 => {
                let ch = usize::from(cc - 0x20);
                let t = self.bank + ch;
                if t >= tracks {
                    return false;
                }
                let v = (u16::from(self.hui_msb[ch]) << 7) | u16::from(value);
                p.tracks[t].volume_db =
                    crate::mixer::frac_to_db(f32::from(v) / 16383.0).clamp(-60.0, 6.0);
                self.faders[ch] = Some(v);
                true
            }
            // V-Pots: pan.
            0x40..=0x47 => {
                let t = self.bank + usize::from(cc - 0x40);
                if t < tracks {
                    p.tracks[t].pan = (p.tracks[t].pan + hui_steps(value) / 64.0).clamp(-1.0, 1.0);
                    return true;
                }
                false
            }
            // Jog wheel: a beat a step.
            0x0D => {
                p.position_beats = (p.position_beats + f64::from(hui_steps(value))).max(0.0);
                true
            }
            0x0F => {
                self.hui_zone = value;
                false
            }
            0x2F => {
                let (port, down) = (value & 0x0F, value & 0x40 != 0);
                self.hui_button(p, self.hui_zone, port, down)
            }
            _ => false,
        }
    }

    fn hui_button(&mut self, p: &mut Project, zone: u8, port: u8, down: bool) -> bool {
        let tracks = p.tracks.len();
        if zone < 8 {
            let ch = usize::from(zone);
            if port == HUI_TOUCH {
                self.touched[ch] = down;
                return false;
            }
            let t = self.bank + ch;
            if !down || t >= tracks {
                return false;
            }
            match port {
                HUI_SELECT => p.selected_track = t,
                HUI_MUTE => p.tracks[t].mute = !p.tracks[t].mute,
                HUI_SOLO => p.tracks[t].solo = !p.tracks[t].solo,
                HUI_REC => p.tracks[t].rec = !p.tracks[t].rec,
                _ => return false,
            }
            return true;
        }
        if !down {
            return false;
        }
        match (zone, port) {
            (HUI_SCROLL, 0) => self.move_bank(-1, tracks),
            (HUI_SCROLL, 1) => self.move_bank(-8, tracks),
            (HUI_SCROLL, 2) => self.move_bank(1, tracks),
            (HUI_SCROLL, 3) => self.move_bank(8, tracks),
            (HUI_TRANSPORT, 1 | 2) => {
                let bar = p.beats_to_bar(p.position_beats).floor();
                let to = if port == 1 { (bar - 1.0).max(0.0) } else { bar + 1.0 };
                p.position_beats = p.bar_to_beats(to);
            }
            (HUI_TRANSPORT, 3) => {
                if !p.playing {
                    p.position_beats = 0.0;
                }
                p.playing = false;
            }
            (HUI_TRANSPORT, 4) => p.playing = true,
            (HUI_TRANSPORT, 5) => p.recording = !p.recording,
            (HUI_TRANSPORT_MORE, 3) => p.cycle = !p.cycle,
            _ => return false,
        }
        if zone == HUI_SCROLL {
            self.hui_lights.clear();
            self.hui_names = Default::default();
        }
        true
    }

    /// Sends a HUI surface what changed: the ping it needs to stay online, faders, lights, names.
    fn show_hui(&mut self, p: &Project) {
        // A HUI goes offline without the host's ping about once a second.
        if self.last_ping.is_none_or(|t| t.elapsed() >= std::time::Duration::from_millis(900)) {
            self.last_ping = Some(std::time::Instant::now());
            self.send(&[0x90, 0x00, 0x00]);
        }
        for ch in 0..8usize {
            let t = self.bank + ch;
            let track = p.tracks.get(t);
            if !self.touched[ch] {
                let v = track.map_or(0, |tr| {
                    (crate::mixer::db_to_frac(tr.volume_db).clamp(0.0, 1.0) * 16383.0).round() as u16
                });
                if self.faders[ch] != Some(v) {
                    self.faders[ch] = Some(v);
                    self.send(&[0xB0, ch as u8, (v >> 7) as u8]);
                    self.send(&[0xB0, 0x20 + ch as u8, (v & 0x7F) as u8]);
                }
            }
            let (sel, mute, solo, rec) = track.map_or((false, false, false, false), |tr| {
                (p.selected_track == t, tr.mute, tr.solo, tr.rec)
            });
            for (port, lit) in [(HUI_SELECT, sel), (HUI_MUTE, mute), (HUI_SOLO, solo), (HUI_REC, rec)] {
                self.hui_light(ch as u8, port, lit);
            }
            // Four letters a channel on the scribble strip.
            let name: String = track
                .map_or("", |tr| tr.name.as_str())
                .chars()
                .filter(char::is_ascii)
                .take(4)
                .collect();
            let name = format!("{name:<4}");
            if self.hui_names[ch] != name {
                let mut sysex = vec![0xF0, 0x00, 0x00, 0x66, 0x05, 0x00, 0x10, ch as u8];
                sysex.extend(name.bytes().map(|b| b & 0x7F));
                sysex.push(0xF7);
                self.send(&sysex);
                self.hui_names[ch] = name;
            }
        }
        self.hui_light(HUI_TRANSPORT, 4, p.playing);
        self.hui_light(HUI_TRANSPORT, 3, !p.playing);
        self.hui_light(HUI_TRANSPORT, 5, p.recording);
        self.hui_light(HUI_TRANSPORT_MORE, 3, p.cycle);
    }

    fn hui_light(&mut self, zone: u8, port: u8, lit: bool) {
        if self.hui_lights.get(&(zone, port)) != Some(&lit) {
            self.hui_lights.insert((zone, port), lit);
            self.send(&[0xB0, 0x0C, zone]);
            self.send(&[0xB0, 0x2C, if lit { 0x40 | port } else { port }]);
        }
    }
}

/// The input ports a surface could be on.
pub fn ports() -> Vec<String> {
    crate::midi_in::ports()
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_v_pot_turn_is_relative() {
        // Bit 6 says left: 0x41 is one step left, 0x01 one step right.
        let steps = |v: u8| f32::from(v & 0x3F) * if v & 0x40 != 0 { -1.0 } else { 1.0 };
        assert_eq!(steps(0x01), 1.0);
        assert_eq!(steps(0x41), -1.0);
        assert_eq!(steps(0x05), 5.0);
    }
}
