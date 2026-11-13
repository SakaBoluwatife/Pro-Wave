//! Halo, the reverb. Its design follows the one Cloud Seed (Valdemar Erlingsson, MIT) made
//! popular: a pre-delay, a set of early reflections, a chain of allpass diffusers that smears
//! the input into a dense wash, then a network of eight delay lines that feed one another
//! (a feedback delay network with a Householder matrix), each slowly modulated so the tail
//! never rings metallic, with a damping filter in every loop. Decay is set in seconds: each
//! line's feedback is worked out from its length so the tail falls by 60 dB in that time.
use std::f32::consts::TAU;

use super::parts::speed;
use super::{Biquad, Line, Shape, MAX_FX_PARAMS};

/// The delay lines' lengths at the middle size, in ms (no two share a factor, so the echoes
/// never line up).
const LINES_MS: [f32; 8] = [29.7, 37.1, 41.1, 43.7, 53.9, 61.3, 67.1, 79.3];
const DIFFUSERS_MS: [f32; 4] = [4.7, 3.6, 12.7, 9.3];
/// Early reflections: time in ms and level.
const EARLY: [(f32, f32); 8] = [
    (7.0, 0.85),
    (11.3, -0.7),
    (17.9, 0.6),
    (23.1, -0.55),
    (31.7, 0.45),
    (41.3, -0.38),
    (52.9, 0.3),
    (67.1, -0.22),
];
const MAX_SIZE: f32 = 2.0;
const MAX_PRE_MS: f32 = 250.0;

/// The size knob (0..1) as a multiple of the middle lengths.
fn scale(size: f32) -> f32 {
    0.35 + size.clamp(0.0, 1.0) * (MAX_SIZE - 0.35)
}

#[derive(Clone, Debug)]
struct Allpass {
    line: Line,
}

impl Allpass {
    #[inline]
    fn tick(&mut self, x: f32, delay: f32, g: f32) -> f32 {
        let d = self.line.read(delay);
        let w = x + d * g;
        self.line.push(w);
        d - w * g
    }
}

#[derive(Clone, Debug)]
pub struct Halo {
    sr: f32,
    pre: [Line; 2],
    low_cut: [Biquad; 2],
    diffusers: [[Allpass; 4]; 2],
    lines: [Line; 8],
    damp: [f32; 8],
    damp_k: f32,
    phase: [f32; 8],
    /// The size the lines read at now (it glides to the setting so turning it does not click).
    size_now: f32,
    /// Each line's feedback, and the size and decay it was worked out for.
    gains: [f32; 8],
    gains_for: (f32, f32),
}

impl Halo {
    pub fn new(sr: f32) -> Self {
        let ms = |v: f32| (v * 0.001 * sr) as usize + 16;
        let pre = || Line::new(ms(MAX_PRE_MS + 80.0 * MAX_SIZE));
        let ap = |v: f32| Allpass {
            line: Line::new(ms(v * MAX_SIZE)),
        };
        let side = || DIFFUSERS_MS.map(ap);
        Self {
            sr,
            pre: [pre(), pre()],
            low_cut: [Biquad::unity(); 2],
            diffusers: [side(), side()],
            lines: LINES_MS.map(|v| Line::new(ms(v * MAX_SIZE + 4.0))),
            damp: [0.0; 8],
            damp_k: 1.0,
            phase: std::array::from_fn(|k| k as f32 / 8.0),
            size_now: -1.0,
            gains: [0.0; 8],
            gains_for: (-1.0, -1.0),
        }
    }

    pub fn set(&mut self, p: &[f32; MAX_FX_PARAMS]) {
        let lc = Biquad::make(Shape::High, p[5], 0.707, 0.0, self.sr);
        for f in self.low_cut.iter_mut() {
            *f = Biquad {
                z1: f.z1,
                z2: f.z2,
                ..lc
            };
        }
        self.damp_k = 1.0 - (-TAU * p[6] / self.sr).exp();
        if self.size_now < 0.0 {
            self.size_now = scale(p[0]);
        }
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32, p: &[f32; MAX_FX_PARAMS]) -> (f32, f32) {
        let sr = self.sr;
        let ms = sr * 0.001;
        let want = scale(p[0]);
        if (want - self.size_now).abs() < 1e-4 {
            self.size_now = want;
        } else {
            self.size_now += (want - self.size_now) * speed(200.0, sr);
        }
        let size = self.size_now;
        let decay = p[1].max(0.1);
        let pre_d = (p[2] * ms).max(1.0);
        let diffusion = p[3].clamp(0.0, 1.0) * 0.72;
        let depth = p[4].clamp(0.0, 1.0) * 0.6 * ms;
        let width = p[7].clamp(0.0, 1.0);
        let early_mix = p[8].clamp(0.0, 1.0);
        let mix = p[9].clamp(0.0, 1.0);
        let input = [self.low_cut[0].tick(l), self.low_cut[1].tick(r)];
        let mut fed = [0f32; 2];
        let mut early = [0f32; 2];
        for ch in 0..2 {
            self.pre[ch].push(input[ch]);
            let x = self.pre[ch].read(pre_d);
            // The two sides hear the reflections at slightly different times.
            let skew = if ch == 0 { 1.0 } else { 1.07 };
            for (t, g) in EARLY {
                early[ch] += self.pre[ch].read(pre_d + t * size * skew * ms) * g;
            }
            early[ch] *= 0.35;
            let mut y = x;
            for (k, a) in self.diffusers[ch].iter_mut().enumerate() {
                let d = DIFFUSERS_MS[k] * size * skew * ms;
                y = a.tick(y, d, diffusion);
            }
            fed[ch] = y;
        }
        // The network: read every line, mix them with a Householder matrix, damp, feed back.
        let mut outs = [0f32; 8];
        let mut sum = 0.0;
        for k in 0..8 {
            self.phase[k] = (self.phase[k] + (0.13 + k as f32 * 0.037) / sr) % 1.0;
            let len = LINES_MS[k] * size * ms;
            let d = len + depth * (1.0 + (TAU * self.phase[k]).sin());
            outs[k] = self.lines[k].read(d);
            sum += outs[k];
        }
        let house = sum * 0.25;
        if (size, decay) != self.gains_for {
            self.gains_for = (size, decay);
            for k in 0..8 {
                let len_s = LINES_MS[k] * size * 0.001;
                self.gains[k] = 10f32.powf(-3.0 * len_s / decay);
            }
        }
        let mut wet = [0f32; 2];
        for k in 0..8 {
            let g = self.gains[k];
            let mixed = outs[k] - house;
            self.damp[k] += (mixed - self.damp[k]) * self.damp_k;
            let back = self.damp[k] * g;
            self.lines[k].push(fed[k & 1] + back);
            let sign = if k & 2 == 0 { 1.0 } else { -1.0 };
            wet[k & 1] += outs[k] * sign;
        }
        let late = [wet[0] * 0.3, wet[1] * 0.3];
        let (wl, wr) = (
            late[0] * (1.0 - early_mix * 0.5) + early[0] * early_mix,
            late[1] * (1.0 - early_mix * 0.5) + early[1] * early_mix,
        );
        // Width: from mono (0) to the full spread (1).
        let (m, s) = ((wl + wr) * 0.5, (wl - wr) * 0.5 * width);
        (l * (1.0 - mix) + (m + s) * mix, r * (1.0 - mix) + (m - s) * mix)
    }

    pub fn reset(&mut self) {
        for l in self
            .pre
            .iter_mut()
            .chain(self.lines.iter_mut())
            .chain(self.diffusers.iter_mut().flatten().map(|a| &mut a.line))
        {
            l.buf.fill(0.0);
        }
        self.damp = [0.0; 8];
        for f in self.low_cut.iter_mut() {
            *f = f.cleared();
        }
    }
}
