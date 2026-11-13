//! Shut, the gate and expander: below the threshold the sound is turned down by the ratio (10:1
//! and up is a gate) as far as the range lets it, with attack, hold and release, filters on what
//! the detector hears, and Flip, which turns it into a ducker (down while the key is loud). It can
//! be keyed by another strip.
use super::parts::{db, gain, speed};
use super::{Biquad, Shape, MAX_FX_PARAMS};

#[derive(Clone, Debug)]
pub struct Shut {
    sr: f32,
    hp: [Biquad; 2],
    lp: [Biquad; 2],
    env: f32,
    hold_left: u32,
    /// The reduction now, in dB.
    pub red: f32,
}

/// The gain in dB (0 or less) for a detected level in dB.
pub fn curve(p: &[f32; MAX_FX_PARAMS], level: f32) -> f32 {
    let (thr, range, ratio) = (p[0], p[1], p[5].max(1.0));
    let flip = p[8] >= 0.5;
    let under = if flip { level - thr } else { thr - level };
    if under <= 0.0 {
        return 0.0;
    }
    -(under * (ratio - 1.0)).min(-range)
}

impl Shut {
    pub fn new(sr: f32) -> Self {
        Self {
            sr,
            hp: [Biquad::unity(); 2],
            lp: [Biquad::unity(); 2],
            env: 0.0,
            hold_left: 0,
            red: 0.0,
        }
    }

    pub fn set(&mut self, p: &[f32; MAX_FX_PARAMS]) {
        let hp = Biquad::make(Shape::High, p[6], 0.707, 0.0, self.sr);
        let lp = Biquad::make(Shape::Low, p[7], 0.707, 0.0, self.sr);
        for k in 0..2 {
            self.hp[k] = Biquad {
                z1: self.hp[k].z1,
                z2: self.hp[k].z2,
                ..hp
            };
            self.lp[k] = Biquad {
                z1: self.lp[k].z1,
                z2: self.lp[k].z2,
                ..lp
            };
        }
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32, key: (f32, f32), p: &[f32; MAX_FX_PARAMS]) -> (f32, f32) {
        let a = self.lp[0].tick(self.hp[0].tick(key.0));
        let b = self.lp[1].tick(self.hp[1].tick(key.1));
        self.env = a.abs().max(b.abs()).max(self.env * (1.0 - 300.0 / self.sr));
        let target = -curve(p, db(self.env));
        if target < self.red {
            // Opening: fast.
            self.red += (target - self.red) * speed(p[2], self.sr);
            self.hold_left = (p[3] * 0.001 * self.sr) as u32;
        } else if self.hold_left > 0 {
            self.hold_left -= 1;
        } else {
            self.red += (target - self.red) * speed(p[4], self.sr);
        }
        let g = gain(-self.red);
        (l * g, r * g)
    }

    pub fn reset(&mut self) {
        for f in self.hp.iter_mut().chain(self.lp.iter_mut()) {
            *f = f.cleared();
        }
        self.env = 0.0;
        self.hold_left = 0;
        self.red = 0.0;
    }
}
