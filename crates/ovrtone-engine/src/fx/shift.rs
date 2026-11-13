//! Shift, the pitch shifter and doubler: two read heads sweep through a short delay at the speed
//! that changes the pitch, each faded in and out so the jump back is never heard (the classic
//! delay-line shifter). Width detunes the two sides opposite ways for a double, Delay sets the
//! copy late like a second take.
use std::f32::consts::PI;

use super::{Line, MAX_FX_PARAMS};

const WINDOW_MS: f32 = 40.0;

#[derive(Clone, Debug)]
pub struct Shift {
    sr: f32,
    lines: [Line; 2],
    phase: [f32; 2],
}

impl Shift {
    pub fn new(sr: f32) -> Self {
        let n = ((WINDOW_MS + 60.0) * 0.001 * sr) as usize + 8;
        Self {
            sr,
            lines: [Line::new(n), Line::new(n)],
            phase: [0.0, 0.5],
        }
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32, p: &[f32; MAX_FX_PARAMS]) -> (f32, f32) {
        let sr = self.sr;
        let window = WINDOW_MS * 0.001 * sr;
        let pre = p[2] * 0.001 * sr;
        let input = [l, r];
        let mut out = [0f32; 2];
        for ch in 0..2 {
            let spread = if ch == 0 { -p[3] } else { p[3] };
            let semis = p[0] + (p[1] + spread) / 100.0;
            let ratio = 2f32.powf(semis / 12.0);
            // How fast the heads move through the window: 0 for no change of pitch.
            let rate = (1.0 - ratio) / window;
            self.phase[ch] = (self.phase[ch] + rate).rem_euclid(1.0);
            self.lines[ch].push(input[ch]);
            let mut y = 0.0;
            for head in [0.0, 0.5] {
                let ph = (self.phase[ch] + head) % 1.0;
                let fade = (PI * ph).sin();
                y += self.lines[ch].read(pre + 1.0 + ph * window) * fade * fade;
            }
            out[ch] = y;
        }
        let mix = p[4].clamp(0.0, 1.0);
        (l * (1.0 - mix) + out[0] * mix, r * (1.0 - mix) + out[1] * mix)
    }

    pub fn reset(&mut self) {
        for l in self.lines.iter_mut() {
            l.buf.fill(0.0);
        }
    }
}
