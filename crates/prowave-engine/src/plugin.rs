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
/// The strip number that means the output: Stereo Out, after everything is mixed.
pub const MASTER_STRIP: u16 = u16::MAX;
/// The slot number that means a strip's instrument, in the commands that take a plugin's place.
pub const INSTRUMENT_SLOT: u8 = u8::MAX;
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

/// Where the song is, for plugins that follow it (a synced delay, an arpeggiator).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PluginTransport {
    pub playing: bool,
    /// Quarter notes from the start of the song.
    pub beat: f64,
    pub seconds: f64,
    /// Quarter notes per minute.
    pub tempo: f64,
    pub signature: (u8, u8),
    /// Where the current bar starts, and its number from 0.
    pub bar_start: f64,
    pub bar_number: i32,
    /// The Cycle, in beats, when it is on.
    pub cycle: Option<(f64, f64)>,
}

impl Default for PluginTransport {
    fn default() -> Self {
        Self {
            playing: false,
            beat: 0.0,
            seconds: 0.0,
            tempo: 120.0,
            signature: (4, 4),
            bar_start: 0.0,
            bar_number: 0,
            cycle: None,
        }
    }
}

/// The audio-thread half of a loaded plugin. `process` runs on the audio thread and must not
/// allocate, lock or do I/O. It is dropped on the UI thread, after coming back as garbage.
pub trait PluginProcessor: Send {
    /// Processes one block: every slice is `PLUGIN_BLOCK` frames long.
    fn process(
        &mut self,
        input: [&[f32]; 2],
        output: [&mut [f32]; 2],
        events: &[PluginEvent],
        transport: &PluginTransport,
    );

    /// The delay the plugin itself puts on the sound, in samples.
    fn latency_samples(&self) -> usize;

    /// Moves a parameter, as a position 0..=1 along its range, from the next block on. Called on
    /// the audio thread: it must not allocate.
    fn set_param(&mut self, _id: u32, _value: f64) {}
}

/// Automated parameters of one plugin whose last value is remembered.
const SENT_PARAMS: usize = 32;

/// A loaded plugin with its preallocated buffers, ready to be sent to the engine.
pub struct PluginSlot {
    /// Which instance this is, for the automation lanes that move it. 0 for none.
    uid: u64,
    /// The values last sent, so that a lane holding still does not keep telling the plugin.
    sent: [(u32, f64); SENT_PARAMS],
    sent_next: usize,
    plugin: Box<dyn PluginProcessor>,
    in_l: Box<[f32]>,
    in_r: Box<[f32]>,
    out_l: Box<[f32]>,
    out_r: Box<[f32]>,
    events: Vec<PluginEvent>,
    /// The next frame of the block.
    pos: usize,
    /// While on, the plugin is not run and the sound passes by it, delayed as much as the plugin
    /// would so that nothing shifts in time.
    bypass: bool,
    /// The sound of the last `latency` frames, for that. Empty for a plugin without latency.
    dry_l: Box<[f32]>,
    dry_r: Box<[f32]>,
    dry_pos: usize,
}

impl PluginSlot {
    /// Call off the audio thread: this allocates.
    pub fn new(plugin: Box<dyn PluginProcessor>) -> Self {
        let buffer = || vec![0.0; PLUGIN_BLOCK].into_boxed_slice();
        let dry = vec![0.0; plugin.latency_samples()].into_boxed_slice();
        Self {
            uid: 0,
            sent: [(u32::MAX, f64::NAN); SENT_PARAMS],
            sent_next: 0,
            plugin,
            in_l: buffer(),
            in_r: buffer(),
            out_l: buffer(),
            out_r: buffer(),
            events: Vec::with_capacity(MAX_PLUGIN_EVENTS),
            pos: 0,
            bypass: false,
            dry_l: dry.clone(),
            dry_r: dry,
            dry_pos: 0,
        }
    }

    /// The whole delay of this slot: the plugin's own and the block it is collected in.
    /// Says which instance this is, for its automation lanes.
    pub fn with_uid(mut self, uid: u64) -> Self {
        self.uid = uid;
        self
    }

    pub fn uid(&self) -> u64 {
        self.uid
    }

    /// Moves a parameter, unless it is there already.
    pub fn set_param(&mut self, id: u32, value: f64) {
        match self.sent.iter_mut().find(|(known, _)| *known == id) {
            Some((_, last)) if *last == value => return,
            Some((_, last)) => *last = value,
            None => {
                self.sent[self.sent_next] = (id, value);
                self.sent_next = (self.sent_next + 1) % SENT_PARAMS;
            }
        }
        self.plugin.set_param(id, value);
    }

    pub fn latency_samples(&self) -> usize {
        self.plugin.latency_samples() + PLUGIN_BLOCK
    }

    pub fn set_bypass(&mut self, bypass: bool) {
        self.bypass = bypass;
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
    pub fn process(&mut self, l: f32, r: f32, transport: &PluginTransport) -> (f32, f32) {
        let i = self.pos;
        let out = (self.out_l[i], self.out_r[i]);
        self.in_l[i] = l;
        self.in_r[i] = r;
        self.pos += 1;
        if self.pos == PLUGIN_BLOCK {
            self.pos = 0;
            self.run_block(transport);
        }
        out
    }

    fn run_block(&mut self, transport: &PluginTransport) {
        self.pass_by();
        if !self.bypass {
            self.plugin.process(
                [&self.in_l, &self.in_r],
                [&mut self.out_l, &mut self.out_r],
                &self.events,
                transport,
            );
        }
        self.events.clear();
        // A plugin that misbehaves must not reach the mix: NaN stays in filters and meters.
        for s in self.out_l.iter_mut().chain(self.out_r.iter_mut()) {
            if !s.is_finite() {
                *s = 0.0;
            }
        }
    }

    /// Keeps the delayed dry sound going, always, so a bypass starts without a gap, and writes it
    /// to the output while bypassed.
    fn pass_by(&mut self) {
        let length = self.dry_l.len();
        for i in 0..PLUGIN_BLOCK {
            let (l, r) = (self.in_l[i], self.in_r[i]);
            let (dry_l, dry_r) = if length == 0 {
                (l, r)
            } else {
                let delayed = (self.dry_l[self.dry_pos], self.dry_r[self.dry_pos]);
                self.dry_l[self.dry_pos] = l;
                self.dry_r[self.dry_pos] = r;
                self.dry_pos = (self.dry_pos + 1) % length;
                delayed
            };
            if self.bypass {
                self.out_l[i] = dry_l;
                self.out_r[i] = dry_r;
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
    use std::sync::Arc;

    #[test]
    fn a_parameter_is_told_only_when_it_moves() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        struct Counting(Arc<AtomicUsize>);
        impl PluginProcessor for Counting {
            fn process(
                &mut self,
                _: [&[f32]; 2],
                _: [&mut [f32]; 2],
                _: &[PluginEvent],
                _: &PluginTransport,
            ) {
            }
            fn latency_samples(&self) -> usize {
                0
            }
            fn set_param(&mut self, _: u32, _: f64) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
        let told = Arc::new(AtomicUsize::new(0));
        let mut slot = PluginSlot::new(Box::new(Counting(told.clone()))).with_uid(7);
        assert_eq!(slot.uid(), 7);
        slot.set_param(1, 0.5);
        slot.set_param(1, 0.5);
        slot.set_param(2, 0.5);
        slot.set_param(1, 0.6);
        assert_eq!(told.load(Ordering::Relaxed), 3);
    }

    /// A stand-in for a real plugin: gain, an added constant, a reported latency, and what it
    /// was sent. `poison` makes it put NaN in the output.
    pub(crate) struct FakePlugin {
        pub gain: f32,
        pub add: f32,
        pub latency: usize,
        pub poison: bool,
        pub notes_on: Arc<AtomicUsize>,
        pub notes_off: Arc<AtomicUsize>,
        pub last_frame: Arc<AtomicUsize>,
        /// Times it was run.
        pub runs: Arc<AtomicUsize>,
        /// The beat it was last told, in thousandths.
        pub last_beat: Arc<AtomicUsize>,
    }

    impl Default for FakePlugin {
        fn default() -> Self {
            Self {
                gain: 1.0,
                add: 0.0,
                latency: 0,
                poison: false,
                notes_on: Arc::default(),
                notes_off: Arc::default(),
                last_frame: Arc::default(),
                runs: Arc::default(),
                last_beat: Arc::default(),
            }
        }
    }

    impl PluginProcessor for FakePlugin {
        fn process(
            &mut self,
            input: [&[f32]; 2],
            output: [&mut [f32]; 2],
            events: &[PluginEvent],
            transport: &PluginTransport,
        ) {
            self.runs.fetch_add(1, Relaxed);
            self.last_beat
                .store((transport.beat * 1000.0) as usize, Relaxed);
            for e in events {
                if matches!(e.kind, PluginEventKind::NoteOff { .. }) {
                    self.notes_off.fetch_add(1, Relaxed);
                }
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
            .map(|i| slot.process(i as f32, -(i as f32), &PluginTransport::default()))
            .collect();
        assert!(out[..PLUGIN_BLOCK].iter().all(|f| *f == (0.0, 0.0)));
        for i in 0..PLUGIN_BLOCK + 5 {
            let v = 2.0 * i as f32;
            assert_eq!(out[PLUGIN_BLOCK + i], (v, -v), "frame {i}");
        }
    }

    #[test]
    fn a_bypassed_plugin_is_not_run_and_the_sound_stays_in_time() {
        for latency in [0, 10, PLUGIN_BLOCK * 3 + 7] {
            let plugin = FakePlugin {
                gain: 0.5,
                latency,
                ..FakePlugin::default()
            };
            let runs = plugin.runs.clone();
            let mut slot = PluginSlot::new(Box::new(plugin));
            let delay = slot.latency_samples();
            let now = PluginTransport::default();
            slot.set_bypass(true);
            let frames = delay + PLUGIN_BLOCK * 2;
            let out: Vec<f32> = (0..frames)
                .map(|i| slot.process(i as f32 + 1.0, 0.0, &now).0)
                .collect();
            assert_eq!(runs.load(Relaxed), 0, "not run while bypassed");
            // The block's own delay is there; the plugin's is made up by the dry line.
            for (i, value) in out.iter().enumerate().skip(delay) {
                assert_eq!(
                    *value,
                    (i - delay) as f32 + 1.0,
                    "frame {i}, latency {latency}"
                );
            }
            // On again, it runs.
            slot.set_bypass(false);
            for i in 0..PLUGIN_BLOCK {
                slot.process(i as f32, 0.0, &now);
            }
            assert!(runs.load(Relaxed) > 0);
        }
    }

    #[test]
    fn what_a_plugin_gets_wrong_does_not_reach_the_mix() {
        let mut slot = PluginSlot::new(Box::new(FakePlugin {
            poison: true,
            ..FakePlugin::default()
        }));
        for _ in 0..PLUGIN_BLOCK * 4 {
            let (l, r) = slot.process(0.0, 0.0, &PluginTransport::default());
            assert!(l.is_finite() && r.is_finite());
        }
    }

    #[test]
    fn events_keep_their_frame_and_too_many_are_dropped_without_allocating() {
        let plugin = FakePlugin::default();
        let (on, frame) = (plugin.notes_on.clone(), plugin.last_frame.clone());
        let mut slot = PluginSlot::new(Box::new(plugin));
        let now = PluginTransport::default();
        for _ in 0..7 {
            slot.process(0.0, 0.0, &now);
        }
        assert_no_alloc::assert_no_alloc(|| {
            for _ in 0..MAX_PLUGIN_EVENTS + 50 {
                slot.push_event(PluginEventKind::NoteOn {
                    key: 60,
                    velocity: 1.0,
                });
            }
            for _ in 7..PLUGIN_BLOCK {
                slot.process(0.0, 0.0, &now);
            }
        });
        assert_eq!(on.load(Relaxed), MAX_PLUGIN_EVENTS);
        assert_eq!(frame.load(Relaxed), 7);
    }
}
