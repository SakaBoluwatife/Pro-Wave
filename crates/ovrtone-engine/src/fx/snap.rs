//! Snap, the transient designer: more or less attack, more or less sustain, whatever the
//! level (it follows the shape of the sound, not a threshold).
//!
//! Attack compares a fast and a slow follower that let go at the same speed: at a hit the
//! fast one is ahead, and the gap is the attack. Sustain compares two that rise at the same
//! speed but let go at different ones: in the tail the slow one stays up, and that gap is the
//! sustain. Each gap becomes a gain in dB, scaled by its knob. Fast, Medium and Slow set the
//! followers' times; Clip rounds off what goes over 0 dB.
use super::parts::{choice, gain, speed, Follower};
use super::MAX_FX_PARAMS;

pub const SPEEDS: [&str; 3] = ["Fast", "Medium", "Slow"];

/// Attack and release times in ms for each speed: (attack fast, attack slow, sustain short,
/// sustain long).
const TIMES: [(f32, f32, f32, f32); 3] = [
    (0.3, 12.0, 25.0, 180.0),
    (1.0, 25.0, 50.0, 350.0),
    (3.0, 50.0, 90.0, 700.0),
];

#[derive(Clone, Debug)]
pub struct Snap {
    sr: f32,
    speed_now: usize,
    attack_fast: Follower,
    attack_slow: Follower,
    sustain_short: Follower,
    sustain_long: Follower,
    gain_db: f32,
    /// The gain applied now, in dB (positive or negative), for the meter.
    pub change_db: f32,
}

impl Snap {
    pub fn new(sr: f32) -> Self {
        let mut s = Self {
            sr,
            speed_now: usize::MAX,
            attack_fast: Follower::default(),
            attack_slow: Follower::default(),
            sustain_short: Follower::default(),
            sustain_long: Follower::default(),
            gain_db: 0.0,
            change_db: 0.0,
        };
        s.times(1);
        s
    }

    fn times(&mut self, k: usize) {
        if k == self.speed_now {
            return;
        }
        self.speed_now = k;
        let (af, asl, ss, sl) = TIMES[k.min(2)];
        let sr = self.sr;
        self.attack_fast.times(af, sl * 0.5, sr);
        self.attack_slow.times(asl, sl * 0.5, sr);
        self.sustain_short.times(af, ss, sr);
        self.sustain_long.times(af, sl, sr);
    }

    pub fn set(&mut self, p: &[f32; MAX_FX_PARAMS]) {
        self.times(choice(p[2], SPEEDS.len()));
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32, p: &[f32; MAX_FX_PARAMS]) -> (f32, f32) {
        let x = l.abs().max(r.abs()) + 1e-6;
        let af = self.attack_fast.tick(x);
        let asl = self.attack_slow.tick(x);
        let ss = self.sustain_short.tick(x);
        let sl = self.sustain_long.tick(x);
        // The gaps in dB, kept within reason.
        let hit = (20.0 * (af / asl.max(1e-6)).log10()).clamp(0.0, 24.0);
        let tail = (20.0 * (sl / ss.max(1e-6)).log10()).clamp(0.0, 24.0);
        let target = (p[0] * 0.01 * hit + p[1] * 0.01 * tail).clamp(-24.0, 24.0);
        self.gain_db += (target - self.gain_db) * speed(0.5, self.sr);
        self.change_db = self.gain_db;
        let g = gain(self.gain_db);
        let (mix, out) = (p[4].clamp(0.0, 1.0), gain(p[5]));
        let mut y = [l * (g * mix + 1.0 - mix) * out, r * (g * mix + 1.0 - mix) * out];
        if p[3] >= 0.5 {
            for v in y.iter_mut() {
                if v.abs() > 0.7 {
                    let over = v.abs() - 0.7;
                    *v = (0.7 + 0.3 * (over / 0.3).tanh()).copysign(*v);
                }
            }
        }
        (y[0], y[1])
    }

    pub fn reset(&mut self) {
        for f in [
            &mut self.attack_fast,
            &mut self.attack_slow,
            &mut self.sustain_short,
            &mut self.sustain_long,
        ] {
            f.value = 0.0;
        }
        self.gain_db = 0.0;
        self.change_db = 0.0;
    }
}
