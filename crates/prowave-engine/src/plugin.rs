//! Hosted plugins as the engine sees them (Logic: Audio Units in the channel strip; here CLAP and
//! VST3 from the `prowave-host` crate). The engine knows only [`PluginProcessor`], so an instance
//! in this process and a bridge to another process look the same to it.
//!
//! The engine renders one sample at a time and plugins work in blocks, so a [`PluginSlot`] collects
//! [`PLUGIN_BLOCK`] frames in buffers made off the audio thread, runs the plugin when they are
//! full and plays out the result. That delays the sound by one block, which
//! [`PluginSlot::latency_samples`] reports for delay compensation.

/// Frames a plugin gets at a time: 1.3 ms at 48 kHz.
pub const PLUGIN_BLOCK: usize = 64;
/// Events one block can carry; past this they are dropped rather than allocating.
pub const MAX_PLUGIN_EVENTS: usize = 256;

/// A MIDI message for a plugin, at a frame of the block it is handed over with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PluginEvent {
    /// Frame of the block, `0..PLUGIN_BLOCK`.
    pub frame: u32,
    pub kind: PluginEventKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PluginEventKind {
    NoteOn {
        key: u8,
        velocity: f32,
    },
    NoteOff {
        key: u8,
    },
    Control {
        number: u8,
        value: u8,
    },
    /// -1..=1 of the plugin's bend range.
    PitchBend {
        value: f32,
    },
}

/// The audio-thread half of a loaded plugin. `process` runs on the audio thread and must not
/// allocate, lock or do I/O. It is dropped on the UI thread, after coming back as garbage.
pub trait PluginProcessor: Send {
    /// Processes one block: every slice is `PLUGIN_BLOCK` frames long.
    fn process(&mut self, input: [&[f32]; 2], output: [&mut [f32]; 2], events: &[PluginEvent]);

    /// The delay the plugin itself puts on the sound, in samples.
    fn latency_samples(&self) -> usize;
}

/// A loaded plugin with its preallocated buffers, ready to be sent to the engine.
pub struct PluginSlot {
    plugin: Box<dyn PluginProcessor>,
    in_l: Box<[f32]>,
    in_r: Box<[f32]>,
    out_l: Box<[f32]>,
    out_r: Box<[f32]>,
    events: Vec<PluginEvent>,
    /// The next frame of the block.
    pos: usize,
}

impl PluginSlot {
    /// Call off the audio thread: this allocates.
    pub fn new(plugin: Box<dyn PluginProcessor>) -> Self {
        let buffer = || vec![0.0; PLUGIN_BLOCK].into_boxed_slice();
        Self {
            plugin,
            in_l: buffer(),
            in_r: buffer(),
            out_l: buffer(),
            out_r: buffer(),
            events: Vec::with_capacity(MAX_PLUGIN_EVENTS),
            pos: 0,
        }
    }

    /// The whole delay of this slot: the plugin's own and the block it is collected in.
    pub fn latency_samples(&self) -> usize {
        self.plugin.latency_samples() + PLUGIN_BLOCK
    }

    /// An event for the frame being collected. Dropped when the block already holds
    /// [`MAX_PLUGIN_EVENTS`].
    pub fn push_event(&mut self, kind: PluginEventKind) {
        if self.events.len() < MAX_PLUGIN_EVENTS {
            self.events.push(PluginEvent {
                frame: self.pos as u32,
                kind,
            });
        }
    }

    /// One frame in, one frame out.
    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        let i = self.pos;
        let out = (self.out_l[i], self.out_r[i]);
        self.in_l[i] = l;
        self.in_r[i] = r;
        self.pos += 1;
        if self.pos == PLUGIN_BLOCK {
            self.pos = 0;
            self.run_block();
        }
        out
    }

    fn run_block(&mut self) {
        self.plugin.process(
            [&self.in_l, &self.in_r],
            [&mut self.out_l, &mut self.out_r],
            &self.events,
        );
        self.events.clear();
        // A plugin that misbehaves must not reach the mix: NaN stays in filters and meters.
        for s in self.out_l.iter_mut().chain(self.out_r.iter_mut()) {
            if !s.is_finite() {
                *s = 0.0;
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
    use std::sync::Arc;

    /// A stand-in for a real plugin: gain, an added constant, a reported latency, and what it
    /// was sent. `poison` makes it put NaN in the output.
    pub(crate) struct FakePlugin {
        pub gain: f32,
        pub add: f32,
        pub latency: usize,
        pub poison: bool,
        pub notes_on: Arc<AtomicUsize>,
        pub last_frame: Arc<AtomicUsize>,
    }

    impl Default for FakePlugin {
        fn default() -> Self {
            Self {
                gain: 1.0,
                add: 0.0,
                latency: 0,
                poison: false,
                notes_on: Arc::default(),
                last_frame: Arc::default(),
            }
        }
    }

    impl PluginProcessor for FakePlugin {
        fn process(&mut self, input: [&[f32]; 2], output: [&mut [f32]; 2], events: &[PluginEvent]) {
            for e in events {
                if matches!(e.kind, PluginEventKind::NoteOn { .. }) {
                    self.notes_on.fetch_add(1, Relaxed);
                    self.last_frame.store(e.frame as usize, Relaxed);
                }
            }
            let [out_l, out_r] = output;
            for (o, i) in out_l.iter_mut().zip(input[0]) {
                *o = *i * self.gain + self.add;
            }
            for (o, i) in out_r.iter_mut().zip(input[1]) {
                *o = *i * self.gain + self.add;
            }
            if self.poison {
                out_l[3] = f32::NAN;
                out_r[4] = f32::INFINITY;
            }
        }

        fn latency_samples(&self) -> usize {
            self.latency
        }
    }

    #[test]
    fn a_slot_delays_by_one_block_and_reports_it() {
        let mut slot = PluginSlot::new(Box::new(FakePlugin {
            gain: 2.0,
            latency: 10,
            ..FakePlugin::default()
        }));
        assert_eq!(slot.latency_samples(), 10 + PLUGIN_BLOCK);
        let out: Vec<(f32, f32)> = (0..PLUGIN_BLOCK * 2 + 5)
            .map(|i| slot.process(i as f32, -(i as f32)))
            .collect();
        assert!(out[..PLUGIN_BLOCK].iter().all(|f| *f == (0.0, 0.0)));
        for i in 0..PLUGIN_BLOCK + 5 {
            let v = 2.0 * i as f32;
            assert_eq!(out[PLUGIN_BLOCK + i], (v, -v), "frame {i}");
        }
    }

    #[test]
    fn what_a_plugin_gets_wrong_does_not_reach_the_mix() {
        let mut slot = PluginSlot::new(Box::new(FakePlugin {
            poison: true,
            ..FakePlugin::default()
        }));
        for _ in 0..PLUGIN_BLOCK * 4 {
            let (l, r) = slot.process(0.0, 0.0);
            assert!(l.is_finite() && r.is_finite());
        }
    }

    #[test]
    fn events_keep_their_frame_and_too_many_are_dropped_without_allocating() {
        let plugin = FakePlugin::default();
        let (on, frame) = (plugin.notes_on.clone(), plugin.last_frame.clone());
        let mut slot = PluginSlot::new(Box::new(plugin));
        for _ in 0..7 {
            slot.process(0.0, 0.0);
        }
        assert_no_alloc::assert_no_alloc(|| {
            for _ in 0..MAX_PLUGIN_EVENTS + 50 {
                slot.push_event(PluginEventKind::NoteOn {
                    key: 60,
                    velocity: 1.0,
                });
            }
            for _ in 7..PLUGIN_BLOCK {
                slot.process(0.0, 0.0);
            }
        });
        assert_eq!(on.load(Relaxed), MAX_PLUGIN_EVENTS);
        assert_eq!(frame.load(Relaxed), 7);
    }
}
