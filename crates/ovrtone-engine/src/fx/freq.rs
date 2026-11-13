//! Freq, the frequency shifter: every frequency in the sound moved up or down by the same number of
//! hertz (not semitones), so harmonics stop lining up: from a slow phasing swirl at a few hertz to
//! clangorous, metallic tones further out. A pair of allpass networks splits the sound into two
//! copies a quarter turn apart (a Hilbert transform), which a sine and cosine then turn round.
//! Width shifts the right side the other way; Feedback sends the result round again.
use std::f32::consts::TAU;

use super::MAX_FX_PARAMS;

/// The two allpass chains of the quarter-turn split, as their coefficients squared go in.
const A: [f32; 4] = [0.692_387_8, 0.936_065_4, 0.988_229_5, 0.998_748_8];
const B: [f32; 4] = [0.402_192_1, 0.856_171_1, 0.972_290_9, 0.995_288_5];

#[derive(Clone, Copy, Debug, Default)]
struct Stage {
    c: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Stage {
    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let y = self.c * (x + self.y2) - self.x2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

#[derive(Clone, Debug)]
struct Split {
    a: [Stage; 4],
    b: [Stage; 4],
    delayed: f32,
}

impl Split {
    fn new() -> Self {
        let chain = |k: [f32; 4]| {
            k.map(|v| Stage {
                c: v * v,
                ..Stage::default()
            })
        };
        Self {
            a: chain(A),
            b: chain(B),
            delayed: 0.0,
        }
    }

    /// The sound and its copy a quarter turn on.
    #[inline]
    fn tick(&mut self, x: f32) -> (f32, f32) {
        let i = self.a.iter_mut().fold(x, |v, s| s.tick(v));
        let q = self.b.iter_mut().fold(x, |v, s| s.tick(v));
        let out = (i, self.delayed);
        self.delayed = q;
        out
    }

    fn reset(&mut self) {
        *self = Self::new();
    }
}

#[derive(Clone, Debug)]
pub struct Freq {
    sr: f32,
    split: [Split; 2],
    phase: [f32; 2],
    last: [f32; 2],
}

impl Freq {
    pub fn new(sr: f32) -> Self {
        Self {
            sr,
            split: [Split::new(), Split::new()],
            phase: [0.0; 2],
            last: [0.0; 2],
        }
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32, p: &[f32; MAX_FX_PARAMS]) -> (f32, f32) {
        let shift = p[0] + p[1];
        let width = p[2].clamp(0.0, 1.0);
        let feedback = p[3].clamp(0.0, 0.9);
        let mix = p[4].clamp(0.0, 1.0);
        let input = [l, r];
        let mut out = [0f32; 2];
        for ch in 0..2 {
            // The right side goes the other way as Width comes up.
            let hz = if ch == 0 { shift } else { shift * (1.0 - 2.0 * width) };
            self.phase[ch] = (self.phase[ch] + hz / self.sr).rem_euclid(1.0);
            let (i, q) = self.split[ch].tick(input[ch] + self.last[ch] * feedback);
            let (s, c) = (TAU * self.phase[ch]).sin_cos();
            let y = i * c - q * s;
            self.last[ch] = y.clamp(-4.0, 4.0);
            out[ch] = input[ch] * (1.0 - mix) + y * mix;
        }
        (out[0], out[1])
    }

    pub fn reset(&mut self) {
        for s in self.split.iter_mut() {
            s.reset();
        }
        self.last = [0.0; 2];
    }
}
