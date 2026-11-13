//! MIDI out for External MIDI tracks: the notes the engine sends for those tracks go to the
//! MIDI port and channel each track is set to. A thread of its own does the sending, so a slow
//! or missing MIDI device never holds up the audio or the interface.
//!
//! The notes are sent as soon as the thread sees them (a millisecond after the audio block
//! that played them), not at their exact sample, which is fine for a synthesizer on a cable.
//!
//! The same thread sends MIDI Time Code, which the engine makes on the audio clock, to the port
//! the project names (`Project::mtc_out`).

use crate::model::{Project, TrackKind};
use midir::{MidiOutput, MidiOutputConnection};
use ovrtone_engine::{MidiOutEvent, MtcMessage};
use rtrb::Consumer;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How long the sending thread sleeps between looks at the queue.
const POLL: Duration = Duration::from_millis(1);
/// How often to try again to open a port that could not be opened.
const RETRY: Duration = Duration::from_secs(3);

/// Where one track's notes go: a port (by name) and a channel, 1-16.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Route {
    pub port: Option<String>,
    pub channel: u8,
}

/// The raw MIDI message for a note.
pub fn message(on: bool, channel: u8, pitch: u8, vel: u8) -> [u8; 3] {
    let status = if on { 0x90 } else { 0x80 };
    [
        status | (channel.clamp(1, 16) - 1),
        pitch.min(127),
        vel.min(127),
    ]
}

/// The raw MIDI message for a controller change (the sustain pedal is controller 64).
pub fn controller_message(channel: u8, number: u8, value: u8) -> [u8; 3] {
    [
        0xb0 | (channel.clamp(1, 16) - 1),
        number.min(127),
        value.min(127),
    ]
}

/// The names of the MIDI output ports on this computer, without the system's "Through" port.
pub fn ports() -> Vec<String> {
    let Ok(out) = MidiOutput::new("Ovrtone") else {
        return Vec::new();
    };
    out.ports()
        .iter()
        .filter_map(|p| out.port_name(p).ok())
        .filter(|n| !n.to_lowercase().contains("through"))
        .collect()
}

/// The routes of a project, by track: only External MIDI tracks have a port.
pub fn routes_of(p: &Project) -> Vec<Route> {
    p.tracks
        .iter()
        .map(|t| {
            if t.kind == TrackKind::ExternalMidi {
                Route {
                    port: t.midi_port.clone(),
                    channel: t.midi_channel,
                }
            } else {
                Route::default()
            }
        })
        .collect()
}

/// What MIDI Clock follows: the port, and where the song was and how fast, at a moment.
#[derive(Clone, Debug, Default)]
struct ClockState {
    port: Option<String>,
    playing: bool,
    beat: f64,
    bpm: f64,
    at: Option<Instant>,
}

/// The sending thread and the routes it follows.
pub struct MidiOut {
    routes: Arc<Mutex<Vec<Route>>>,
    mtc_port: Arc<Mutex<Option<String>>>,
    clock: Arc<Mutex<ClockState>>,
    last: Vec<Route>,
    last_mtc: Option<String>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl MidiOut {
    /// Starts sending what `events` and `mtc` deliver.
    pub fn start(events: Consumer<MidiOutEvent>, mtc: Consumer<MtcMessage>) -> Self {
        let routes = Arc::new(Mutex::new(Vec::new()));
        let mtc_port = Arc::new(Mutex::new(None));
        let clock = Arc::new(Mutex::new(ClockState::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (r, m, s, c) = (routes.clone(), mtc_port.clone(), stop.clone(), clock.clone());
        let thread = std::thread::Builder::new()
            .name("ovrtone-midi-out".into())
            .spawn(move || {
                let mut sender = Sender::new(events, mtc, r, m, s);
                sender.clock = Some(c);
                sender.run()
            })
            .ok();
        Self {
            routes,
            mtc_port,
            clock,
            last: Vec::new(),
            last_mtc: None,
            stop,
            thread,
        }
    }

    /// Call once per frame: passes the tracks' ports and channels, and the port of MIDI Time
    /// Code, on when they changed.
    pub fn sync(&mut self, p: &Project) {
        let now = routes_of(p);
        if now != self.last {
            if let Ok(mut routes) = self.routes.lock() {
                routes.clone_from(&now);
            }
            self.last = now;
        }
        // MIDI Clock: the port, and the song's place and speed now, for the thread to run on from.
        if let Ok(mut c) = self.clock.lock() {
            c.port.clone_from(&p.clock_out);
            c.playing = p.playing;
            c.beat = p.position_beats;
            c.bpm = f64::from(p.tempo_at(p.position_beats));
            c.at = Some(Instant::now());
        }
        if p.mtc_out != self.last_mtc {
            if let Ok(mut port) = self.mtc_port.lock() {
                port.clone_from(&p.mtc_out);
            }
            self.last_mtc.clone_from(&p.mtc_out);
        }
    }
}

impl Drop for MidiOut {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Sender {
    events: Consumer<MidiOutEvent>,
    mtc: Consumer<MtcMessage>,
    routes: Arc<Mutex<Vec<Route>>>,
    mtc_port: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
    connections: HashMap<String, MidiOutputConnection>,
    failed: HashMap<String, Instant>,
    /// Notes that were started and not yet released: (port, channel, pitch).
    sounding: HashSet<(String, u8, u8)>,
    clock: Option<Arc<Mutex<ClockState>>>,
    /// The clock ticks sent since the song started playing, and whether it is playing.
    clock_ticks: i64,
    clock_running: bool,
}

impl Sender {
    fn new(
        events: Consumer<MidiOutEvent>,
        mtc: Consumer<MtcMessage>,
        routes: Arc<Mutex<Vec<Route>>>,
        mtc_port: Arc<Mutex<Option<String>>>,
        stop: Arc<AtomicBool>,
    ) -> Self {
        Self {
            events,
            mtc,
            routes,
            mtc_port,
            stop,
            connections: HashMap::new(),
            failed: HashMap::new(),
            sounding: HashSet::new(),
            clock: None,
            clock_ticks: 0,
            clock_running: false,
        }
    }

    fn run(mut self) {
        while !self.stop.load(Ordering::Acquire) {
            while let Ok(ev) = self.events.pop() {
                self.send(ev);
            }
            while let Ok(m) = self.mtc.pop() {
                self.send_mtc(&m);
            }
            self.send_clock();
            std::thread::sleep(POLL);
        }
        self.release_all();
    }

    fn route_of(&self, track: u16) -> Option<(String, u8)> {
        let routes = self.routes.lock().ok()?;
        let r = routes.get(track as usize)?;
        Some((r.port.clone()?, r.channel.clamp(1, 16)))
    }

    fn send(&mut self, ev: MidiOutEvent) {
        let Some((port, channel)) = self.route_of(ev.track) else {
            return;
        };
        if ev.controller {
            if let Some(conn) = self.connection(&port) {
                let _ = conn.send(&controller_message(channel, ev.pitch, ev.vel));
            }
            return;
        }
        let key = (port.clone(), channel, ev.pitch);
        if ev.on {
            self.sounding.insert(key);
        } else if !self.sounding.remove(&key) {
            // Its Note On went somewhere else (the track was re-routed): nothing to release.
            return;
        }
        if let Some(conn) = self.connection(&port) {
            let _ = conn.send(&message(ev.on, channel, ev.pitch, ev.vel));
        }
    }

    /// MIDI Clock: 24 ticks a beat while the song plays, worked out from the last place and speed
    /// the interface gave and the time since; Start (with the song position) and Stop around them.
    fn send_clock(&mut self) {
        let Some(state) = self.clock.as_ref().and_then(|c| c.lock().ok().map(|s| s.clone())) else {
            return;
        };
        let Some(port) = state.port.clone() else {
            self.clock_running = false;
            return;
        };
        let beat_now = match state.at {
            Some(at) if state.playing => {
                state.beat + at.elapsed().as_secs_f64() * state.bpm.max(1.0) / 60.0
            }
            _ => state.beat,
        };
        if state.playing && !self.clock_running {
            // Song position in sixteenths, then Continue from there.
            let sixteenths = (beat_now.max(0.0) * 4.0) as u32;
            let spp = [0xF2, (sixteenths & 0x7f) as u8, ((sixteenths >> 7) & 0x7f) as u8];
            if let Some(conn) = self.connection(&port) {
                let _ = conn.send(&spp);
                let _ = conn.send(&[if sixteenths == 0 { 0xFA } else { 0xFB }]);
            }
            self.clock_ticks = (beat_now.max(0.0) * 24.0).floor() as i64;
            self.clock_running = true;
        } else if !state.playing && self.clock_running {
            if let Some(conn) = self.connection(&port) {
                let _ = conn.send(&[0xFC]);
            }
            self.clock_running = false;
        }
        if !self.clock_running {
            return;
        }
        // The ticks due by now; a jump of the song (a cycle back, a locate) starts the count again.
        let due = (beat_now.max(0.0) * 24.0).floor() as i64;
        if due < self.clock_ticks || due - self.clock_ticks > 96 {
            self.clock_running = false;
            return;
        }
        while self.clock_ticks < due {
            if let Some(conn) = self.connection(&port) {
                let _ = conn.send(&[0xF8]);
            }
            self.clock_ticks += 1;
        }
    }

    /// Sends a MIDI Time Code message to the project's MTC port, if it names one.
    fn send_mtc(&mut self, m: &MtcMessage) {
        let port = self.mtc_port.lock().ok().and_then(|p| p.clone());
        let Some(port) = port else {
            return;
        };
        if let Some(conn) = self.connection(&port) {
            let _ = conn.send(m.bytes());
        }
    }

    /// The open connection to `port`, opening it when needed (and again every few seconds
    /// while it cannot be opened).
    fn connection(&mut self, port: &str) -> Option<&mut MidiOutputConnection> {
        if !self.connections.contains_key(port) {
            if self.failed.get(port).is_some_and(|t| t.elapsed() < RETRY) {
                return None;
            }
            match open(port) {
                Some(conn) => {
                    self.failed.remove(port);
                    self.connections.insert(port.to_string(), conn);
                }
                None => {
                    self.failed.insert(port.to_string(), Instant::now());
                    return None;
                }
            }
        }
        self.connections.get_mut(port)
    }

    /// Releases every note still sounding (the app is closing, or the thread stopped).
    fn release_all(&mut self) {
        let sounding: Vec<_> = self.sounding.drain().collect();
        for (port, channel, pitch) in sounding {
            if let Some(conn) = self.connections.get_mut(&port) {
                let _ = conn.send(&message(false, channel, pitch, 0));
            }
        }
    }
}

fn open(port: &str) -> Option<MidiOutputConnection> {
    let out = MidiOutput::new("Ovrtone").ok()?;
    let found = out
        .ports()
        .into_iter()
        .find(|p| out.port_name(p).is_ok_and(|n| n == port))?;
    out.connect(&found, "ovrtone-out").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_are_raw_midi_on_their_channel() {
        assert_eq!(message(true, 1, 60, 100), [0x90, 60, 100]);
        assert_eq!(message(false, 1, 60, 0), [0x80, 60, 0]);
        assert_eq!(message(true, 16, 127, 127), [0x9f, 127, 127]);
        assert_eq!(
            message(true, 0, 200, 200),
            [0x90, 127, 127],
            "out of range is repaired"
        );
        assert_eq!(
            message(false, 10, 36, 64),
            [0x89, 36, 64],
            "channel 10 is drums"
        );
    }

    #[test]
    fn controllers_are_raw_midi_on_their_channel() {
        assert_eq!(controller_message(1, 64, 127), [0xb0, 64, 127]);
        assert_eq!(controller_message(16, 1, 0), [0xbf, 1, 0]);
        assert_eq!(controller_message(0, 200, 200), [0xb0, 127, 127]);
    }

    #[test]
    fn only_external_tracks_have_a_route() {
        let mut p = Project::empty();
        p.tracks.clear();
        p.tracks
            .push(crate::model::Track::new("Inst", TrackKind::Instrument));
        let mut ext = crate::model::Track::new("Ext", TrackKind::ExternalMidi);
        ext.midi_port = Some("Synth".into());
        ext.midi_channel = 3;
        p.tracks.push(ext);
        let routes = routes_of(&p);
        assert_eq!(routes[0], Route::default());
        assert_eq!(
            routes[1],
            Route {
                port: Some("Synth".into()),
                channel: 3
            }
        );
    }

    #[test]
    fn a_track_without_a_port_sends_nothing_and_a_missing_port_is_not_fatal() {
        let (mut tx, rx) = rtrb::RingBuffer::new(8);
        let (_mtc_tx, mtc_rx) = rtrb::RingBuffer::new(8);
        let routes = Arc::new(Mutex::new(vec![
            Route::default(),
            Route {
                port: Some("No Such Port 7f3a".into()),
                channel: 1,
            },
        ]));
        let mut sender = Sender::new(
            rx,
            mtc_rx,
            routes,
            Arc::new(Mutex::new(None)),
            Arc::new(AtomicBool::new(false)),
        );
        for track in [0u16, 1, 9] {
            assert!(tx
                .push(MidiOutEvent {
                    track,
                    on: true,
                    pitch: 60,
                    vel: 90,
                    controller: false,
                })
                .is_ok());
        }
        while let Ok(ev) = sender.events.pop() {
            sender.send(ev);
        }
        // The note is remembered as sounding on the missing port (so it is released if the
        // port appears), and the failed open is not retried at once.
        assert!(sender
            .sounding
            .contains(&("No Such Port 7f3a".to_string(), 1, 60)));
        assert!(sender.failed.contains_key("No Such Port 7f3a"));
        assert!(sender.connections.is_empty());
        sender.release_all();
        assert!(sender.sounding.is_empty());
    }
}
