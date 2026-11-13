//! Reel, the tape machine: a soft tape saturation with a bias control, the bump in the low end
//! that tape heads give, the top falling away (more at the slower speed), wow and flutter (the
//! speed wavering, by a short moving delay), and hiss.
use std::f32::consts::TAU;

use super::parts::{choice, gain, speed};
use super::{Biquad, Line, Shape, MAX_FX_PARAMS};

pub const SPEEDS: [&str; 2] = ["15 ips", "30 ips"];
/// The middle of the wavering delay, in ms: the effect's delay.
pub const CENTRE_MS: f32 = 3.0;

#[derive(Clone, Debug)]
pub struct Reel {
    sr: f32,
    bump: [Biquad; 2],
    top: [Biquad; 2],
    lines: [Line; 2],
    dry: [Line; 2],
    wow: f32,
    flutter: f32,
    drift: f32,
    seed: u32,
}

impl Reel {
    pub fn new(sr: f32) -> Self {
        let n = (0.02 * sr) as usize;
        Self {
            sr,
            bump: [Biquad::unity(); 2],
            top: [Biquad::unity(); 2],
            lines: [Line::new(n), Line::new(n)],
            dry: [Line::new(n), Line::new(n)],
            wow: 0.0,
            flutter: 0.0,
            drift: 0.0,
            seed: 0x2545_f491,
        }
    }

    pub fn set(&mut self, p: &[f32; MAX_FX_PARAMS]) {
        let fast = choice(p[6], 2) == 1;
        let bump_hz = if fast { 95.0 } else { 55.0 };
        let bump = Biquad::make(Shape::Peak, bump_hz, 1.1, p[4], self.sr);
        let top = Biquad::make(Shape::Low, p[5] * if fast { 1.4 } else { 1.0 }, 0.6, 0.0, self.sr);
        for k in 0..2 {
            self.bump[k] = Biquad {
                z1: self.bump[k].z1,
                z2: self.bump[k].z2,
                ..bump
            };
            self.top[k] = Biquad {
                z1: self.top[k].z1,
                z2: self.top[k].z2,
                ..top
            };
        }
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32, p: &[f32; MAX_FX_PARAMS]) -> (f32, f32) {
        let sr = self.sr;
        let drive = gain(p[0]);
        let bias = p[1] * 0.3;
        self.wow = (self.wow + 0.55 / sr) % 1.0;
        self.flutter = (self.flutter + 7.1 / sr) % 1.0;
        self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let noise = (self.seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
        self.drift += (noise - self.drift) * speed(200.0, sr);
        let ms = sr * 0.001;
        let waver = CENTRE_MS * ms
            + ms * (p[2] * 1.6 * (TAU * self.wow).sin() + p[3] * 0.25 * (TAU * self.flutter).sin())
            + ms * p[2] * 2.0 * self.drift;
        let hiss = gain(p[7]) * if p[7] <= -89.0 { 0.0 } else { 1.0 };
        let input = [l, r];
        let mut out = [0f32; 2];
        for ch in 0..2 {
            let x = input[ch] * drive;
            // Tape: soft, a touch lopsided with the bias, and back down.
            let sat = ((x + bias).tanh() - bias.tanh()) / drive.max(1.0).sqrt();
            let y = self.top[ch].tick(self.bump[ch].tick(sat));
            self.lines[ch].push(y);
            self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let n = ((self.seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * hiss;
            out[ch] = self.lines[ch].read(waver) + n;
        }
        let (mix, o) = (p[8].clamp(0.0, 1.0), gain(p[9]));
        // The dry sound waits as long as the tape does, so that mixing does not comb.
        let delay = CENTRE_MS * ms;
        self.dry[0].push(l);
        self.dry[1].push(r);
        let (dl, dr) = (self.dry[0].read(delay), self.dry[1].read(delay));
        (
            (dl * (1.0 - mix) + out[0] * mix) * o,
            (dr * (1.0 - mix) + out[1] * mix) * o,
        )
    }

    pub fn reset(&mut self) {
        for f in self.bump.iter_mut().chain(self.top.iter_mut()) {
            *f = f.cleared();
        }
        for l in self.lines.iter_mut().chain(self.dry.iter_mut()) {
            l.buf.fill(0.0);
        }
    }
}
