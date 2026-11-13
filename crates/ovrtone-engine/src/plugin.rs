//! Hosted plugins as the engine sees them (Logic: Audio Units in the channel strip; here CLAP and
//! VST3 from the `ovrtone-host` crate). The engine knows only [`PluginProcessor`], so an instance
//! in this process and a bridge to another process look the same to it.
//!
//! The engine renders one sample at a time and plugins work in blocks, so a [`PluginSlot`] collects
//! [`PLUGIN_BLOCK`] frames in buffers made off the audio thread, runs the plugin when they are
//! full and plays out the result. That delays the sound by one block, which
//! [`PluginSlot::latency_samples`] reports for delay compensation.

/// Frames a plugin gets at a time unless a larger block was chosen: 1.3 ms at 48 kHz.
pub const PLUGIN_BLOCK: usize = 64;
/// The chosen block, 0 for [`PLUGIN_BLOCK`].
static CHOSEN_BLOCK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Chooses the block plugins work in. Call before any plugin is loaded: each is told its block
/// when it is set up. Larger blocks are often cheaper for heavy plugins and cost delay.
pub fn set_plugin_block(frames: usize) {
    let frames = if frames > PLUGIN_BLOCK {
        frames.min(4096)
    } else {
        0
    };
    CHOSEN_BLOCK.store(frames, std::sync::atomic::Ordering::Relaxed);
}

/// Frames a plugin gets at a time.
pub fn plugin_block() -> usize {
    match CHOSEN_BLOCK.load(std::sync::atomic::Ordering::Relaxed) {
        0 => PLUGIN_BLOCK,
        frames => frames,
    }
}

/// Whether a strip's plugins run in series within one block, so that the strip is delayed by one
/// block rather than one for each plugin. On with any block larger than the usual one.
pub fn plugins_chained() -> bool {
    plugin_block() > PLUGIN_BLOCK
}
/// The strip number that means the output: Stereo Out, after everything is mixed.
pub const MASTER_STRIP: u16 = u16::MAX;
/// The slot number that means a strip's instrument, in the commands that take a plugin's place.
pub const INSTRUMENT_SLOT: u8 = u8::MAX;
/// Events one block can carry; past this they are dropped rather than allocating.
pub const MAX_PLUGIN_EVENTS: usize = 1024;

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
    /// The pitch of the one sounding note `key`, in semitones from its own: a slide that bends only
    /// that note (CLAP's tuning note expression), for plugins that say they take it.
    NoteTune {
        key: u8,
        semitones: f32,
    },
    /// Channel pressure (aftertouch), 0..=1.
    ChannelPressure {
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

    /// The side-chain input of the block that comes next, for a plugin that has one. Called on the
    /// audio thread just before [`Self::process`]: it must not allocate. Each slice is
    /// `PLUGIN_BLOCK` frames long. Not called while no source is chosen, after one call with silence.
    fn set_side_input(&mut self, _side: [&[f32]; 2]) {}

    /// Says the blocks come from an offline render, with no deadline. Called off the UI thread
    /// and must not allocate.
    fn set_offline(&mut self, _offline: bool) {}

    /// Whether the plugin makes sound of its own with silence going in, so it must never rest.
    fn keeps_running(&self) -> bool {
        false
    }

    /// Whether it can bend one note while others hold ([`PluginEventKind::NoteTune`]). If not, a
    /// slide bends the whole instrument.
    fn note_tuning(&self) -> bool {
        false
    }
}

/// Running every hosted plugin's block at once, on several cores. A plugin's output reaches the mix
/// only from the next block on, so that within a block no plugin waits for another, even in one
/// chain: they have no order to keep, and any core can take any of them.
///
/// Made off the audio thread with room for every slot, so a round never allocates.
pub struct PluginRound {
    pool: std::sync::Arc<crate::parallel::Pool>,
    graph: crate::parallel::Graph,
    slots: Vec<SlotHandle>,
    /// The jobs of a round: where each one's slots start in `slots`, and how many, run in series.
    chains: Vec<(u32, u32)>,
    /// Frames since the last round.
    pub(crate) phase: usize,
    /// Frames between rounds.
    pub(crate) block: usize,
    chained: bool,
}

/// Quieter than this counts as silence: about -100 dB.
const QUIET: f32 = 1e-5;
/// Blocks of silence, in and out, before a plugin rests: about five seconds at 48 kHz. Long enough
/// for the gap before a long delay's next echo.
const REST_AFTER_BLOCKS: u32 = 4096;

fn is_quiet(block: &[f32]) -> bool {
    block.iter().all(|s| s.abs() < QUIET)
}

/// A slot of a round. It stays where it is as long as the engine holds its box.
#[derive(Clone, Copy)]
struct SlotHandle(*mut PluginSlot);

// SAFETY: a round is the only thing touching the slots while it runs, each handle goes to one job, and
// the plugin half in a slot is `Send`.
unsafe impl Send for SlotHandle {}
unsafe impl Sync for SlotHandle {}

impl PluginRound {
    /// Call off the audio thread: this allocates. `capacity` is how many slots there can be.
    pub fn new(pool: std::sync::Arc<crate::parallel::Pool>, capacity: usize) -> Self {
        Self {
            pool,
            graph: crate::parallel::Graph::with_capacity(capacity),
            slots: Vec::with_capacity(capacity),
            chains: Vec::with_capacity(capacity),
            phase: 0,
            block: plugin_block(),
            chained: plugins_chained(),
        }
    }

    /// The slots a round runs from now on, each put in step.
    pub(crate) fn set_slots<'a>(
        &mut self,
        slots: impl Iterator<Item = (bool, &'a mut PluginSlot)>,
    ) {
        self.slots.clear();
        self.chains.clear();
        let phase = self.phase;
        for (after_another, slot) in slots {
            if self.slots.len() < self.slots.capacity() {
                if !slot.in_rounds {
                    slot.join_rounds(phase);
                }
                let follows = self.chained && after_another && !self.chains.is_empty();
                slot.follows = follows;
                match self.chains.last_mut() {
                    Some(chain) if follows => chain.1 += 1,
                    _ => self.chains.push((self.slots.len() as u32, 1)),
                }
                self.slots.push(SlotHandle(slot));
            }
        }
        self.graph.set(self.chains.len(), &mut Vec::new());
    }

    pub(crate) fn pool(&self) -> &crate::parallel::Pool {
        &self.pool
    }

    /// Runs every slot's block, here and on the workers. If the pool is busy (a bounce is rendering
    /// while the program plays), on this thread alone, with the same result.
    pub(crate) fn run(&self, transport: &PluginTransport) {
        let (slots, chains) = (&self.slots, &self.chains);
        let one = |j: usize, _lane: usize| {
            let (first, count) = chains[j];
            let (first, count) = (first as usize, count as usize);
            for k in first..first + count {
                let SlotHandle(slot) = slots[k];
                // SAFETY: see `SlotHandle`. The graph hands each job out once, each slot is in one
                // job, and the engine waits for the run to finish before it touches a slot again.
                unsafe {
                    if k > first {
                        (*slot).take_input(&*slots[k - 1].0);
                    }
                    (*slot).run_round(transport);
                }
            }
        };
        // Waking the workers costs more than a resting plugin does.
        // SAFETY: as above: nothing else touches the slots between runs.
        let working = chains
            .iter()
            .filter(|(first, count)| {
                let (first, count) = (*first as usize, *count as usize);
                slots[first..first + count]
                    .iter()
                    .any(|s| unsafe { (*s.0).has_work() })
            })
            .count();
        if working < 2 || !self.pool.run_wanting(&self.graph, &one, working - 1) {
            for j in 0..chains.len() {
                one(j, 0);
            }
        }
    }
}

/// Treats the smallest floating-point numbers as zero on this thread. A decaying tail would otherwise
/// reach them, where every operation on them takes many times as long, as every serious DAW guards
/// against. Costs nothing noticeable, and nobody can hear them.
#[inline]
pub(crate) fn flush_denormals() {
    #[cfg(target_arch = "x86_64")]
    // SAFETY: only sets the flush-to-zero and denormals-are-zero bits of this thread's control word.
    #[allow(deprecated)]
    unsafe {
        use std::arch::x86_64::{_mm_getcsr, _mm_setcsr};
        _mm_setcsr(_mm_getcsr() | 0x8040);
    }
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
    /// Low Latency mode has this one out of the way: the sound passes with no delay but its block.
    low_latency: bool,
    /// The engine runs a block when it runs all the others, on several cores ([`PluginRound`]).
    in_rounds: bool,
    /// Frames in its block.
    block: usize,
    /// In a chain and not its first: its input comes from the slot before it, at the round.
    follows: bool,
    /// Blocks of silence before it rests.
    rest_after: u32,
    /// A block is collected, waiting for that.
    full: bool,
    /// While on, the plugin is not run and the sound passes by it, delayed as much as the plugin
    /// would so that nothing shifts in time.
    bypass: bool,
    /// The sound of the last `latency` frames, for that. Empty for a plugin without latency.
    dry_l: Box<[f32]>,
    dry_r: Box<[f32]>,
    dry_pos: usize,
    /// The strip whose sound goes to the plugin's side-chain input.
    side_source: Option<u16>,
    side_l: Box<[f32]>,
    side_r: Box<[f32]>,
    /// The source was taken away: the plugin is told silence once.
    side_flush: bool,
    /// Blocks in a row with silence going in and coming out.
    quiet_blocks: u32,
    /// Resting: with nothing to process, the plugin is not called, and uses no CPU.
    resting: bool,
    may_rest: bool,
    /// Its plugin plays by itself (see [`PluginProcessor::keeps_running`]).
    keeps_running: bool,
    /// Whether the song was playing at the last block: a plugin may follow it.
    was_playing: bool,
}

impl PluginSlot {
    /// Call off the audio thread: this allocates.
    pub fn new(plugin: Box<dyn PluginProcessor>) -> Self {
        Self::sized(plugin, plugin_block())
    }

    fn sized(plugin: Box<dyn PluginProcessor>, block: usize) -> Self {
        let keeps_running = plugin.keeps_running();
        let buffer = || vec![0.0; block].into_boxed_slice();
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
            low_latency: false,
            in_rounds: false,
            block,
            follows: false,
            rest_after: (REST_AFTER_BLOCKS as usize * PLUGIN_BLOCK / block).max(1) as u32,
            full: false,
            bypass: false,
            dry_l: dry.clone(),
            dry_r: dry,
            dry_pos: 0,
            side_source: None,
            side_l: buffer(),
            side_r: buffer(),
            side_flush: false,
            quiet_blocks: 0,
            resting: false,
            may_rest: !keeps_running,
            keeps_running,
            was_playing: false,
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

    /// Whether its plugin bends single notes.
    pub fn note_tuning(&self) -> bool {
        self.plugin.note_tuning()
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
        self.wake();
        self.plugin.set_param(id, value);
    }

    pub fn latency_samples(&self) -> usize {
        self.plugin.latency_samples() + self.block
    }

    /// Tells the plugin whether it is in an offline render.
    pub fn set_offline(&mut self, offline: bool) {
        self.plugin.set_offline(offline);
    }

    /// Whether the plugin may rest when it has nothing to process. Waking it if not.
    pub fn set_may_rest(&mut self, may: bool) {
        self.may_rest = may && !self.keeps_running;
        if !self.may_rest {
            self.wake();
        }
    }

    fn wake(&mut self) {
        self.resting = false;
        self.quiet_blocks = 0;
    }

    /// Whether nothing is going into the plugin now, with the song as it was.
    fn nothing_coming(&self, transport: &PluginTransport) -> bool {
        self.events.is_empty()
            && !self.side_flush
            && transport.playing == self.was_playing
            && is_quiet(&self.in_l)
            && is_quiet(&self.in_r)
            && (self.side_source.is_none() || (is_quiet(&self.side_l) && is_quiet(&self.side_r)))
    }

    /// Whether a block is ready that will have the plugin working.
    fn has_work(&self) -> bool {
        self.full
            && (!self.resting
                || !(self.events.is_empty() && is_quiet(&self.in_l) && is_quiet(&self.in_r)))
    }

    pub fn set_bypass(&mut self, bypass: bool) {
        self.wake();
        self.bypass = bypass;
    }

    /// The strip whose sound goes to the side-chain input from now on.
    pub fn set_side_source(&mut self, source: Option<u16>) {
        if source.is_none() && self.side_source.is_some() {
            self.side_l.fill(0.0);
            self.side_r.fill(0.0);
            self.side_flush = true;
        }
        self.side_source = source;
    }

    #[inline]
    pub fn side_source(&self) -> Option<u16> {
        self.side_source
    }

    /// The side-chain input of the frame [`Self::process`] is about to take.
    #[inline]
    pub fn feed_side(&mut self, l: f32, r: f32) {
        self.side_l[self.pos] = l;
        self.side_r[self.pos] = r;
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
        if !self.follows {
            self.in_l[i] = l;
            self.in_r[i] = r;
        }
        self.pos += 1;
        if self.pos == self.block {
            self.pos = 0;
            if self.in_rounds {
                self.full = true;
            } else {
                self.run_block(transport);
            }
        }
        out
    }

    /// From now on its blocks wait for the engine's round, which comes at `phase` frames into the
    /// block a slot collects: the slots of a round all finish a block together.
    pub fn join_rounds(&mut self, phase: usize) {
        self.in_rounds = true;
        self.pos = phase % self.block;
        self.full = false;
    }

    /// Takes the sound the slot before it in a chain just made.
    fn take_input(&mut self, from: &PluginSlot) {
        if self.full {
            self.in_l.copy_from_slice(&from.out_l);
            self.in_r.copy_from_slice(&from.out_r);
        }
    }

    /// Runs the block, if one is ready.
    pub fn run_round(&mut self, transport: &PluginTransport) {
        if self.full {
            self.full = false;
            self.run_block(transport);
        }
    }

    /// Takes the plugin out of the way for Low Latency mode, or brings it back.
    pub fn set_low_latency(&mut self, on: bool) {
        self.low_latency = on;
    }

    fn run_block(&mut self, transport: &PluginTransport) {
        if self.low_latency {
            self.out_l.copy_from_slice(&self.in_l);
            self.out_r.copy_from_slice(&self.in_r);
            self.events.clear();
            return;
        }
        self.pass_by();
        if !self.bypass {
            let quiet = self.nothing_coming(transport);
            self.was_playing = transport.playing;
            if quiet && self.resting {
                return;
            }
            self.resting = false;
            if self.side_source.is_some() || self.side_flush {
                self.side_flush = false;
                self.plugin.set_side_input([&self.side_l, &self.side_r]);
            }
            self.plugin.process(
                [&self.in_l, &self.in_r],
                [&mut self.out_l, &mut self.out_r],
                &self.events,
                transport,
            );
            if quiet && is_quiet(&self.out_l) && is_quiet(&self.out_r) {
                self.quiet_blocks = self.quiet_blocks.saturating_add(1);
                if self.may_rest && self.quiet_blocks >= self.rest_after {
                    self.resting = true;
                    self.out_l.fill(0.0);
                    self.out_r.fill(0.0);
                }
            } else {
                self.quiet_blocks = 0;
            }
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
        for i in 0..self.block {
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
    fn plugins_in_a_chain_share_one_block_of_delay() {
        const BLOCK: usize = 256;
        let pool = crate::parallel::Pool::new(1, None);
        let mut round = PluginRound::new(pool, 8);
        round.block = BLOCK;
        round.chained = true;
        let plugin = || {
            Box::new(FakePlugin {
                gain: 0.5,
                ..FakePlugin::default()
            })
        };
        let (mut a, mut b) = (
            PluginSlot::sized(plugin(), BLOCK),
            PluginSlot::sized(plugin(), BLOCK),
        );
        round.set_slots([(false, &mut a), (true, &mut b)].into_iter());
        let t = PluginTransport::default();
        let mut heard = Vec::new();
        for i in 0..BLOCK * 4 {
            if round.phase == BLOCK {
                round.phase = 0;
                round.run(&t);
            }
            let x = if i == 10 { 1.0 } else { 0.0 };
            let (l, r) = a.process(x, x, &t);
            heard.push(b.process(l, r, &t).0);
            round.phase += 1;
        }
        let at = heard.iter().position(|s| *s != 0.0);
        assert_eq!(at, Some(10 + BLOCK), "one block, not two");
        assert_eq!(heard[10 + BLOCK], 0.25, "through both");
    }

    #[test]
    fn a_plugin_with_nothing_to_do_rests_and_wakes_on_sound_or_a_note() {
        let runs = Arc::new(AtomicUsize::new(0));
        let mut slot = PluginSlot::new(Box::new(FakePlugin {
            runs: runs.clone(),
            ..FakePlugin::default()
        }));
        let t = PluginTransport::default();
        let blocks = |slot: &mut PluginSlot, n: usize, x: f32| {
            let mut last = 0.0;
            assert_no_alloc::assert_no_alloc(|| {
                for _ in 0..n * PLUGIN_BLOCK {
                    last = slot.process(x, x, &t).0;
                }
            });
            last
        };
        blocks(&mut slot, REST_AFTER_BLOCKS as usize + 1, 0.0);
        assert!(slot.resting);
        let before = runs.load(Relaxed);
        blocks(&mut slot, 50, 0.0);
        assert_eq!(runs.load(Relaxed), before, "not called while it rests");
        blocks(&mut slot, 2, 0.25);
        assert!(!slot.resting, "sound wakes it");
        assert!(runs.load(Relaxed) > before);
        // A plugin that may not rest keeps running.
        slot.set_may_rest(false);
        blocks(&mut slot, REST_AFTER_BLOCKS as usize + 2, 0.0);
        assert!(!slot.resting);
    }

    #[test]
    fn low_latency_lets_the_sound_by_with_only_its_block_of_delay() {
        let mut slot = PluginSlot::new(Box::new(FakePlugin {
            add: 0.5,
            ..FakePlugin::default()
        }));
        slot.set_low_latency(true);
        let t = PluginTransport::default();
        let mut last = (0.0, 0.0);
        for _ in 0..PLUGIN_BLOCK * 3 {
            last = slot.process(0.25, 0.25, &t);
        }
        assert_eq!(last, (0.25, 0.25), "the plugin did not run");
    }

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

    /// Passes on what its side-chain input holds, as if that were its output.
    pub(crate) struct KeyPlugin(pub Box<[f32]>);

    impl KeyPlugin {
        pub(crate) fn new() -> Self {
            Self(vec![0.0; PLUGIN_BLOCK].into_boxed_slice())
        }
    }

    impl PluginProcessor for KeyPlugin {
        fn process(
            &mut self,
            _: [&[f32]; 2],
            output: [&mut [f32]; 2],
            _: &[PluginEvent],
            _: &PluginTransport,
        ) {
            let [l, r] = output;
            l.copy_from_slice(&self.0);
            r.copy_from_slice(&self.0);
        }

        fn latency_samples(&self) -> usize {
            0
        }

        fn set_side_input(&mut self, side: [&[f32]; 2]) {
            self.0.copy_from_slice(side[0]);
        }
    }

    #[test]
    fn the_side_input_reaches_the_plugin_with_its_block_and_goes_silent_when_taken_away() {
        let mut slot = PluginSlot::new(Box::new(KeyPlugin::new()));
        let t = PluginTransport::default();
        slot.set_side_source(Some(3));
        assert_eq!(slot.side_source(), Some(3));
        let mut last = 0.0;
        assert_no_alloc::assert_no_alloc(|| {
            for i in 0..PLUGIN_BLOCK * 2 {
                slot.feed_side(i as f32, 0.0);
                last = slot.process(0.0, 0.0, &t).0;
            }
        });
        // The first block comes out during the second, in order.
        assert_eq!(last, (PLUGIN_BLOCK - 1) as f32);
        slot.set_side_source(None);
        for _ in 0..PLUGIN_BLOCK * 3 {
            last = slot.process(0.0, 0.0, &t).0;
        }
        assert_eq!(last, 0.0, "no key without a source");
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
