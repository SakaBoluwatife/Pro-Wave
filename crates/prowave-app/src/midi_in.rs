//! MIDI keyboard input: connects to every MIDI input port (not the system's "Through"
//! port) and turns its note messages into [`LiveNote`]s for the app to play and record.
//! Without a MIDI device nothing happens; the app looks again every few seconds.

use crate::recorder::LiveNote;
use midir::{MidiInput, MidiInputConnection};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::time::{Duration, Instant};

/// How often to look for a MIDI keyboard while none is connected.
const RESCAN: Duration = Duration::from_secs(3);

/// A controller moved on a device, for MIDI learn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Control {
    /// 0-15.
    pub channel: u8,
    pub controller: u8,
    pub value: u8,
}

/// The controller change in a raw MIDI message, if it is one.
pub fn parse_control(msg: &[u8]) -> Option<Control> {
    let [status, controller, value, ..] = *msg else {
        return None;
    };
    (status & 0xf0 == 0xb0 && controller < 128 && value < 128).then_some(Control {
        channel: status & 0x0f,
        controller,
        value,
    })
}

pub struct MidiIn {
    /// Every input open now, by name.
    connections: Vec<(String, MidiInputConnection<()>)>,
    /// Inputs that would not open, so the log says so once.
    refused: Vec<String>,
    /// A port another input uses (MIDI Time Code): this one leaves it alone, since some systems
    /// let only one program or connection open a port.
    avoided: Option<String>,
    /// Each note with when it arrived.
    tx: SyncSender<(Instant, LiveNote)>,
    rx: Receiver<(Instant, LiveNote)>,
    controls_tx: SyncSender<Control>,
    controls_rx: Receiver<Control>,
    last_try: Option<Instant>,
}

/// The note message in a raw MIDI message, if it is one (Note On with velocity 0 is a Note
/// Off). Channel and everything else are ignored.
pub fn parse(msg: &[u8]) -> Option<LiveNote> {
    let [status, pitch, vel, ..] = *msg else {
        return None;
    };
    if pitch > 127 {
        return None;
    }
    match status & 0xf0 {
        0xb0 if pitch == crate::recorder::SUSTAIN => Some(LiveNote::Sustain { val: vel.min(127) }),
        0x90 if vel > 0 => Some(LiveNote::On {
            pitch,
            vel: vel.min(127),
        }),
        0x90 | 0x80 => Some(LiveNote::Off { pitch }),
        _ => None,
    }
}

impl MidiIn {
    pub fn new() -> Self {
        // Bounded: a device that floods the port cannot fill the memory.
        let (tx, rx) = sync_channel(4096);
        let (controls_tx, controls_rx) = sync_channel(4096);
        let mut midi = Self {
            connections: Vec::new(),
            refused: Vec::new(),
            avoided: None,
            tx,
            rx,
            controls_tx,
            controls_rx,
            last_try: None,
        };
        midi.connect();
        midi
    }

    /// Keeps away from the port named `name` (or stops doing so with `None`): a connection to
    /// it is closed, and the keyboard is looked for on the other ports.
    pub fn avoid(&mut self, name: Option<&str>) {
        if self.avoided.as_deref() == name {
            return;
        }
        self.avoided = name.map(str::to_string);
        if let Some(name) = name {
            self.connections.retain(|(open, _)| open != name);
        }
        // Look again at once: the port that was avoided may be the keyboard.
        self.last_try = None;
    }

    /// Opens every input not open yet, closes those that are gone. A keyboard is not always the
    /// first port: drivers and other programs make ports of their own.
    fn connect(&mut self) {
        self.last_try = Some(Instant::now());
        let Ok(input) = MidiInput::new("Pro Wave") else {
            return;
        };
        let names: Vec<String> = input
            .ports()
            .iter()
            .filter_map(|port| input.port_name(port).ok())
            .collect();
        self.connections
            .retain(|(open, _)| names.iter().any(|n| n == open));
        for name in names {
            let wanted = !name.to_lowercase().contains("through")
                && self.avoided.as_deref() != Some(&name)
                && !self.connections.iter().any(|(open, _)| *open == name);
            if wanted {
                self.open(name);
            }
        }
    }

    fn open(&mut self, name: String) {
        // A connection takes the input object with it, so each gets its own.
        let Ok(input) = MidiInput::new("Pro Wave") else {
            return;
        };
        let ports = input.ports();
        let Some(port) = ports
            .iter()
            .find(|port| input.port_name(port).ok().as_deref() == Some(&name))
        else {
            return;
        };
        let tx = self.tx.clone();
        let controls_tx = self.controls_tx.clone();
        match input.connect(
            port,
            "prowave-in",
            move |_stamp, msg, _| {
                if let Some(note) = parse(msg) {
                    let _ = tx.try_send((Instant::now(), note));
                }
                if let Some(control) = parse_control(msg) {
                    let _ = controls_tx.try_send(control);
                }
            },
            (),
        ) {
            Ok(connection) => {
                tracing::info!("MIDI input: {name}");
                self.refused.retain(|r| *r != name);
                self.connections.push((name, connection));
            }
            Err(e) => {
                // On Windows another program holding the port is the usual reason.
                if !self.refused.contains(&name) {
                    tracing::warn!("cannot open MIDI input {name}: {e}");
                    self.refused.push(name);
                }
            }
        }
    }

    /// Appends the notes played since the last call to `out`, and to `ages` how many seconds ago
    /// each arrived.
    pub fn poll(&mut self, out: &mut Vec<LiveNote>, ages: &mut Vec<f64>) {
        if self.last_try.is_none_or(|t| t.elapsed() >= RESCAN) {
            self.connect();
        }
        while let Ok((at, note)) = self.rx.try_recv() {
            out.push(note);
            ages.push(at.elapsed().as_secs_f64());
        }
    }

    /// Appends the controllers moved since the last call to `out`.
    pub fn poll_controls(&mut self, out: &mut Vec<Control>) {
        while let Ok(control) = self.controls_rx.try_recv() {
            out.push(control);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_messages_become_live_notes_on_any_channel() {
        assert_eq!(
            parse(&[0x90, 60, 100]),
            Some(LiveNote::On {
                pitch: 60,
                vel: 100
            })
        );
        assert_eq!(
            parse(&[0x93, 72, 1]),
            Some(LiveNote::On { pitch: 72, vel: 1 })
        );
        assert_eq!(parse(&[0x80, 60, 64]), Some(LiveNote::Off { pitch: 60 }));
        // Note On with velocity 0 is a Note Off.
        assert_eq!(parse(&[0x90, 60, 0]), Some(LiveNote::Off { pitch: 60 }));
    }

    #[test]
    fn the_sustain_pedal_is_controller_64() {
        assert_eq!(
            parse(&[0xb0, 64, 127]),
            Some(LiveNote::Sustain { val: 127 })
        );
        assert_eq!(parse(&[0xb2, 64, 0]), Some(LiveNote::Sustain { val: 0 }));
    }

    #[test]
    fn other_messages_are_ignored() {
        assert_eq!(parse(&[0xb0, 1, 127]), None, "modulation");
        assert_eq!(parse(&[0xe0, 0, 64]), None, "pitch bend");
        assert_eq!(parse(&[0xf8]), None, "clock");
        assert_eq!(parse(&[0x90, 60]), None, "cut short");
        assert_eq!(parse(&[0x90, 200, 100]), None, "pitch out of range");
        assert_eq!(parse(&[]), None);
    }
}
