//! Drift, the delay: a stereo echo with a time offset for the right side, ping-pong, wow and
//! flutter (the time wanders like a tape machine's), low and high cuts and saturation inside
//! the repeats, and ducking (the echoes step back while the dry sound plays). Changing the
//! time glides to the new one, as a tape does, instead of clicking.
use std::f32::consts::TAU;

use super::parts::{speed, Follower};
use super::{Biquad, Line, Shape, MAX_FX_PARAMS};

pub const MAX_MS: f32 = 2000.0;

/// Sync: Off (the Time knob in ms) or a note value at the project tempo.
pub const SYNC: [&str; 14] = [
    "Off", "1/32", "1/16T", "1/16", "1/16D", "1/8T", "1/8", "1/8D", "1/4T", "1/4", "1/4D",
    "1/2", "1/2D", "1/1",
];
/// Each Sync choice's length in quarter notes (Off has none).
const BEATS: [f32; 14] = [
    0.0,
    0.125,
    1.0 / 6.0,
    0.25,
    0.375,
    1.0 / 3.0,
    0.5,
    0.75,
    2.0 / 3.0,
    1.0,
    1.5,
    2.0,
    3.0,
    4.0,
];
/// The Sync parameter's number.
pub const SYNC_PARAM: usize = 10;

/// The delay time in ms these settings give at `bpm`: the note value when synced, else Time.
pub fn time_ms(p: &[f32; MAX_FX_PARAMS], bpm: f32) -> f32 {
    synced(p[0], p[SYNC_PARAM], bpm)
}

/// `time` ms, or with a Sync choice (`sync`, an index into [`SYNC`]) its note value at `bpm`.
pub fn synced(time: f32, sync: f32, bpm: f32) -> f32 {
    let k = super::parts::choice(sync, SYNC.len());
    if k == 0 || !(bpm.is_finite() && bpm > 0.0) {
        return time;
    }
    (BEATS[k] * 60_000.0 / bpm).clamp(1.0, MAX_MS)
}

#[derive(Clone, Debug)]
pub struct Drift {
    sr: f32,
    lines: [Line; 2],
    low_cut: [Biquad; 2],
    high_cut: [Biquad; 2],
    /// The delay time each side reads at now, in samples (it glides to the setting).
    time: [f32; 2],
    wow: f32,
    flutter: f32,
    wander: f32,
    seed: u32,
    duck: Follower,
    /// The project tempo, for a synced time.
    pub bpm: f32,
    /// Another strip's sound, when it is what the echoes duck under.
    pub key: Option<(f32, f32)>,
}

impl Drift {
    pub fn new(sr: f32) -> Self {
        let n = ((MAX_MS + 60.0) * 0.001 * sr) as usize + 8;
        Self {
            sr,
            lines: [Line::new(n), Line::new(n)],
            low_cut: [Biquad::unity(); 2],
            high_cut: [Biquad::unity(); 2],
            time: [0.0; 2],
            wow: 0.0,
            flutter: 0.0,
            wander: 0.0,
            seed: 0x1234_5678,
            duck: Follower::new(5.0, 250.0, sr),
            bpm: 120.0,
            key: None,
        }
    }

    pub fn set(&mut self, p: &[f32; MAX_FX_PARAMS]) {
        let lc = Biquad::make(Shape::High, p[5], 0.707, 0.0, self.sr);
        let hc = Biquad::make(Shape::Low, p[6], 0.707, 0.0, self.sr);
        if self.time[0] <= 0.0 {
            // A new delay starts at its time instead of gliding up to it.
            let base = time_ms(p, self.bpm) * 0.001 * self.sr;
            self.time = [base, (base + p[2] * 0.001 * self.sr).max(1.0)];
        }
        for k in 0..2 {
            self.low_cut[k] = Biquad {
                z1: self.low_cut[k].z1,
                z2: self.low_cut[k].z2,
                ..lc
            };
            self.high_cut[k] = Biquad {
                z1: self.high_cut[k].z1,
                z2: self.high_cut[k].z2,
                ..hc
            };
        }
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32, p: &[f32; MAX_FX_PARAMS]) -> (f32, f32) {
        let sr = self.sr;
        let base = time_ms(p, self.bpm) * 0.001 * sr;
        let offset = p[2] * 0.001 * sr;
        let want = [base, (base + offset).max(1.0)];
        // Tape-like wandering: a slow wow, a quicker flutter and a little random drift.
        let amount = p[4].clamp(0.0, 1.0);
        self.wow = (self.wow + 0.6 / sr) % 1.0;
        self.flutter = (self.flutter + 6.3 / sr) % 1.0;
        self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let noise = (self.seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
        self.wander += (noise - self.wander) * speed(300.0, sr);
        let wobble = amount
            * sr
            * 0.001
            * (2.5 * (TAU * self.wow).sin() + 0.25 * (TAU * self.flutter).sin() + 6.0 * self.wander);
        let glide = speed(80.0, sr);
        for k in 0..2 {
            self.time[k] += (want[k] - self.time[k]) * glide;
        }
        let dl = self.lines[0].read(self.time[0] + wobble);
        let dr = self.lines[1].read(self.time[1] + wobble);
        let feedback = p[1].clamp(0.0, 1.05);
        let pong = p[3].clamp(0.0, 1.0);
        let heat = p[7].clamp(0.0, 1.0);
        // Ping-pong: the input goes into the left only, and each repeat crosses over.
        let into = [l + r * pong, r * (1.0 - pong)];
        let back = [
            dl * (1.0 - pong) + dr * pong,
            dr * (1.0 - pong) + dl * pong,
        ];
        for k in 0..2 {
            let mut f = self.high_cut[k].tick(self.low_cut[k].tick(back[k] * feedback));
            if heat > 0.0 {
                let drive = 1.0 + heat * 4.0;
                f = (f * drive).tanh() / drive.tanh().max(1e-3) * (1.0 - heat * 0.2);
            }
            // Never more than unity round the loop at full feedback, so it cannot run away.
            self.lines[k].push(into[k] + f.clamp(-4.0, 4.0));
        }
        let (dl, dr) = self.key.unwrap_or((l, r));
        let level = self.duck.tick(dl.abs().max(dr.abs()));
        let duck = 1.0 / (1.0 + level * p[8].clamp(0.0, 1.0) * 12.0);
        let mix = p[9].clamp(0.0, 1.0);
        (
            l * (1.0 - mix) + dl * mix * duck,
            r * (1.0 - mix) + dr * mix * duck,
        )
    }

    pub fn reset(&mut self) {
        for l in self.lines.iter_mut() {
            l.buf.fill(0.0);
        }
        for f in self.low_cut.iter_mut().chain(self.high_cut.iter_mut()) {
            *f = f.cleared();
        }
        self.duck.value = 0.0;
    }
}
