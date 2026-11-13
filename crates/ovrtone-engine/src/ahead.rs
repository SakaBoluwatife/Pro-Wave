//! Render-ahead playback (anticipative processing): while nothing is live (no track recording or
//! monitoring its input, no notes being played in), a thread of its own renders the song ahead of
//! the device into a ring, and the device callback only copies from it. A heavy passage then has
//! the whole lookahead to finish in, not one buffer.
//!
//! The engine core lives in a [`CoreSlot`]: whoever takes it (the ahead thread for one chunk, or
//! the device callback) is the only one running it. The ahead thread puts a chunk into the ring
//! before it gives the core back, so the core's position is always the end of the ring. The
//! callback reads the ring first and renders only what is missing itself, so the sound runs on
//! without a seam when the ahead thread starts or stops. Nothing here allocates, locks or does I/O
//! once made.

use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread::{JoinHandle, Thread};
use std::time::{Duration, Instant};

use crate::core::{dsp_load, EngineCore, Shared};

/// Most frames the ring holds.
pub(crate) const MAX_AHEAD_FRAMES: usize = 16_384;
/// Frames the ahead thread renders at a time, holding the core.
const CHUNK: usize = 256;

/// The engine core, owned by whichever thread took it last. Lock-free.
pub(crate) struct CoreSlot {
    ptr: AtomicPtr<EngineCore>,
}

// SAFETY: the core is reached only through `take`, which hands it to one thread at a time.
unsafe impl Send for CoreSlot {}
// SAFETY: as above.
unsafe impl Sync for CoreSlot {}

impl CoreSlot {
    pub(crate) fn new(core: Box<EngineCore>) -> Self {
        Self {
            ptr: AtomicPtr::new(Box::into_raw(core)),
        }
    }

    /// The core, if no other thread has it.
    pub(crate) fn take(&self) -> Option<Box<EngineCore>> {
        let p = self.ptr.swap(ptr::null_mut(), Ordering::AcqRel);
        // SAFETY: a non-null pointer here came from `Box::into_raw` and is now ours alone.
        (!p.is_null()).then(|| unsafe { Box::from_raw(p) })
    }

    /// Gives the core back. Only the thread that took it calls this, so the slot is empty.
    pub(crate) fn put(&self, core: Box<EngineCore>) {
        let old = self.ptr.swap(Box::into_raw(core), Ordering::AcqRel);
        debug_assert!(old.is_null(), "the core was put back twice");
    }

    /// Takes the core, waiting up to `patience` for the thread holding it to finish its chunk.
    pub(crate) fn take_within(&self, patience: Duration) -> Option<Box<EngineCore>> {
        if let Some(core) = self.take() {
            return Some(core);
        }
        let began = Instant::now();
        loop {
            std::hint::spin_loop();
            if let Some(core) = self.take() {
                return Some(core);
            }
            if began.elapsed() > patience {
                return None;
            }
        }
    }
}

impl Drop for CoreSlot {
    fn drop(&mut self) {
        drop(self.take());
    }
}

/// What the device callback and the ahead thread share.
pub(crate) struct AheadControl {
    pub(crate) stop: AtomicBool,
    /// Frames the device asks for at a time, and its own delay in microseconds.
    pub(crate) device_frames: AtomicU32,
    pub(crate) device_latency_us: AtomicU32,
}

/// The ahead thread: renders chunks into `ring` while [`Shared::render_ahead`] asks for some.
pub(crate) fn spawn(
    slot: Arc<CoreSlot>,
    shared: Arc<Shared>,
    control: Arc<AheadControl>,
    mut ring: rtrb::Producer<f32>,
    sample_rate: u32,
) -> Option<(Thread, JoinHandle<()>)> {
    let handle = std::thread::Builder::new()
        .name("ovrtone-ahead".into())
        .spawn(move || {
            crate::output::worker_setup();
            let mut buf = vec![0f32; CHUNK * 2];
            let capacity = ring.buffer().capacity() / 2;
            while !control.stop.load(Ordering::Relaxed) {
                let wanted = shared.render_ahead() as usize;
                let device = control.device_frames.load(Ordering::Relaxed) as usize;
                let target = if wanted == 0 {
                    0
                } else {
                    wanted.max(device * 2).min(capacity)
                };
                let have = capacity - ring.slots() / 2;
                if have + CHUNK > target {
                    std::thread::park_timeout(Duration::from_millis(2));
                    continue;
                }
                let Some(mut core) = slot.take() else {
                    std::thread::yield_now();
                    continue;
                };
                let began = Instant::now();
                // What is heard now is the device's delay plus what waits in the ring.
                let ring_us = (have as u64 * 1_000_000 / u64::from(sample_rate.max(1))) as u32;
                let latency = control
                    .device_latency_us
                    .load(Ordering::Relaxed)
                    .saturating_add(ring_us);
                core.set_device_info(device as u32, latency);
                {
                    let core = &mut core;
                    let buf = &mut buf;
                    assert_no_alloc::assert_no_alloc(|| core.process(buf));
                }
                // Into the ring before the core goes back: its position stays the ring's end.
                for &s in buf.iter() {
                    let _ = ring.push(s);
                }
                slot.put(core);
                shared.record_dsp_load(dsp_load(began.elapsed(), CHUNK, sample_rate));
            }
        })
        .ok()?;
    Some((handle.thread().clone(), handle))
}
