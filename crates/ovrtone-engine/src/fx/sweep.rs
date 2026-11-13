//! Sweep, the auto-filter: a resonant state-variable filter (low, band or high pass) whose cutoff
//! is moved by a wave (free or in note values at the project tempo) and by an envelope that
//! follows the level, of its own sound or of another strip (a side-chain).
use std::f32::consts::{PI, TAU};

use super::parts::{choice, Follower};
use super::MAX_FX_PARAMS;

pub const MODES: [&str; 3] = ["Low Pass", "Band Pass", "High Pass"];
pub const SHAPES: [&str; 4] = ["Sine", "Triangle", "Square", "Saw"];

/// Wave `shape` at `phase` (0..1), -1..1.
#[inline]
pub fn wave(shape: usize, phase: f32) -> f32 {
    let f = phase.rem_euclid(1.0);
    match shape {
        0 => (TAU * f).sin(),
        1 => {
            if f < 0.5 {
                4.0 * f - 1.0
            } else {
                3.0 - 4.0 * f
            }
        }
        2 => {
            if f < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        _ => 2.0 * f - 1.0,
    }
}

#[derive(Clone, Debug)]
pub struct Sweep {
    sr: f32,
    ic1: [f32; 2],
    ic2: [f32; 2],
    phase: f32,
    env: Follower,
    pub bpm: f32,
}

impl Sweep {
    pub fn new(sr: f32) -> Self {
        Self {
            sr,
            ic1: [0.0; 2],
            ic2: [0.0; 2],
            phase: 0.0,
            env: Follower::new(5.0, 150.0, sr),
            bpm: 120.0,
        }
    }

    pub fn set(&mut self, p: &[f32; MAX_FX_PARAMS]) {
        let release = 40.0 + p[7] * 600.0;
        self.env.times(2.0 + p[7] * 40.0, release, self.sr);
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32, key: (f32, f32), p: &[f32; MAX_FX_PARAMS]) -> (f32, f32) {
        let sr = self.sr;
        // The wave's rate: Hz, or a note value at the tempo.
        let ms = super::drift::synced(1000.0 / p[3].max(0.01), p[4], self.bpm);
        self.phase = (self.phase + 1000.0 / ms.max(1.0) / sr) % 1.0;
        let lfo = wave(choice(p[8], SHAPES.len()), self.phase);
        let env = self.env.tick(key.0.abs().max(key.1.abs())).min(1.0);
        let octaves = lfo * p[5] + env * p[6];
        let cutoff = (p[1] * 2f32.powf(octaves)).clamp(20.0, sr * 0.45);
        let g = (PI * cutoff / sr).tan();
        let k = 2.0 - 1.9 * p[2].clamp(0.0, 1.0);
        let a1 = 1.0 / (1.0 + g * (g + k));
        let (a2, a3) = (g * a1, g * g * a1);
        let mode = choice(p[0], MODES.len());
        let input = [l, r];
        let mut out = [0f32; 2];
        for ch in 0..2 {
            let v3 = input[ch] - self.ic2[ch];
            let v1 = a1 * self.ic1[ch] + a2 * v3;
            let v2 = self.ic2[ch] + a2 * self.ic1[ch] + a3 * v3;
            self.ic1[ch] = 2.0 * v1 - self.ic1[ch];
            self.ic2[ch] = 2.0 * v2 - self.ic2[ch];
            out[ch] = match mode {
                0 => v2,
                1 => v1,
                _ => input[ch] - k * v1 - v2,
            };
        }
        let mix = p[9].clamp(0.0, 1.0);
        (l * (1.0 - mix) + out[0] * mix, r * (1.0 - mix) + out[1] * mix)
    }

    pub fn reset(&mut self) {
        self.ic1 = [0.0; 2];
        self.ic2 = [0.0; 2];
        self.env.value = 0.0;
    }
}
