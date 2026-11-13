//! Flange, the flanger: a very short delay swept by a slow wave and fed back, mixed with the dry
//! sound for the comb that moves. Through-zero delays the dry sound too, so the swept copy can pass
//! it and come back, as two tape machines did. Width sets the two sides' sweeps apart.
use std::f32::consts::TAU;

use super::{Line, MAX_FX_PARAMS};

/// The most delay it sweeps over, in ms.
pub const MAX_MS: f32 = 12.0;

/// Its own delay with these settings, in samples (through-zero only).
pub fn latency(p: &[f32; MAX_FX_PARAMS], sr: f32) -> usize {
    if p[6] >= 0.5 {
        (p[3].clamp(0.1, MAX_MS) * 0.001 * sr) as usize
    } else {
        0
    }
}

#[derive(Clone, Debug)]
pub struct Flange {
    sr: f32,
    lines: [Line; 2],
    dry: [Line; 2],
    phase: f32,
    last: [f32; 2],
}

impl Flange {
    pub fn new(sr: f32) -> Self {
        let n = ((MAX_MS * 2.2) * 0.001 * sr) as usize + 8;
        Self {
            sr,
            lines: [Line::new(n), Line::new(n)],
            dry: [Line::new(n), Line::new(n)],
            phase: 0.0,
            last: [0.0; 2],
        }
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32, p: &[f32; MAX_FX_PARAMS], base: &[f32; MAX_FX_PARAMS]) -> (f32, f32) {
        let sr = self.sr;
        self.phase = (self.phase + p[0] / sr) % 1.0;
        let centre = p[3].clamp(0.1, MAX_MS) * 0.001 * sr;
        let depth = p[1].clamp(0.0, 1.0);
        let feedback = p[2].clamp(-0.95, 0.95);
        let through_zero = base[6] >= 0.5;
        let input = [l, r];
        let mut wet = [0f32; 2];
        for ch in 0..2 {
            let offset = if ch == 0 { 0.0 } else { p[4] * 0.5 };
            let wave = (TAU * (self.phase + offset)).sin();
            let d = if through_zero {
                // Around the dry sound's own delay, both sides of it.
                centre * (1.0 + depth * wave)
            } else {
                1.0 + centre * (0.5 + 0.5 * depth * wave)
            };
            self.lines[ch].push(input[ch] + self.last[ch] * feedback);
            let y = self.lines[ch].read(d.max(1.0));
            self.last[ch] = y;
            wet[ch] = y;
        }
        let dry = if through_zero {
            self.dry[0].push(l);
            self.dry[1].push(r);
            [self.dry[0].read(centre.max(1.0)), self.dry[1].read(centre.max(1.0))]
        } else {
            input
        };
        let mix = p[5].clamp(0.0, 1.0);
        (
            dry[0] * (1.0 - mix * 0.5) + wet[0] * mix * 0.7,
            dry[1] * (1.0 - mix * 0.5) + wet[1] * mix * 0.7,
        )
    }

    pub fn reset(&mut self) {
        for l in self.lines.iter_mut().chain(self.dry.iter_mut()) {
            l.buf.fill(0.0);
        }
        self.last = [0.0; 2];
    }
}
