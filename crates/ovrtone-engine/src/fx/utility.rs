//! Utility, Width, Air and Meter: the small tools.
//!
//! - Utility: gain, pan, width, either side's polarity flipped, mono, the sides swapped, and a
//!   filter for DC.
//! - Width: the stereo image wider or narrower, the low end made mono below a frequency, and pan.
//! - Air: an exciter: the top of the sound driven into harmonics and added back.
//! - Meter: passes the sound through untouched; its window does the measuring.
use super::parts::{choice, gain};
use super::{Biquad, Shape, MAX_FX_PARAMS};

/// Equal-power pan gains for `pan` (-1..1).
#[inline]
fn pan_gains(pan: f32) -> (f32, f32) {
    let a = (pan.clamp(-1.0, 1.0) + 1.0) * 0.25 * std::f32::consts::PI;
    (
        a.cos() * std::f32::consts::SQRT_2,
        a.sin() * std::f32::consts::SQRT_2,
    )
}

#[derive(Clone, Debug, Default)]
pub struct Utility {
    dc: [(f32, f32); 2],
}

impl Utility {
    #[inline]
    pub fn tick(&mut self, l: f32, r: f32, p: &[f32; MAX_FX_PARAMS]) -> (f32, f32) {
        let g = gain(p[0]);
        let (mut a, mut b) = (l * g, r * g);
        if choice(p[3], 2) == 1 {
            a = -a;
        }
        if choice(p[4], 2) == 1 {
            b = -b;
        }
        if choice(p[6], 2) == 1 {
            std::mem::swap(&mut a, &mut b);
        }
        let (m, s) = ((a + b) * 0.5, (a - b) * 0.5 * p[2]);
        let (mut a, mut b) = (m + s, m - s);
        if choice(p[5], 2) == 1 {
            let mono = (a + b) * 0.5;
            a = mono;
            b = mono;
        }
        if choice(p[7], 2) == 1 {
            for (x, (xi, yo)) in [&mut a, &mut b].into_iter().zip(self.dc.iter_mut()) {
                let y = *x - *xi + 0.9995 * *yo;
                *xi = *x;
                *yo = y;
                *x = y;
            }
        }
        let (pl, pr) = pan_gains(p[1]);
        (a * pl, b * pr)
    }

    /// [`Self::tick`] over a span: the gain, the switches and the pan worked out once, not per
    /// frame, so the loop is plain arithmetic the compiler turns into vector code. The same sums
    /// in the same order, so the same sound.
    pub fn run_block(&mut self, l: &mut [f64], r: &mut [f64], p: &[f32; MAX_FX_PARAMS]) {
        let g = gain(p[0]);
        let (flip_l, flip_r, swap) = (choice(p[3], 2) == 1, choice(p[4], 2) == 1, choice(p[6], 2) == 1);
        let (mono, dc) = (choice(p[5], 2) == 1, choice(p[7], 2) == 1);
        let width = p[2];
        let (pl, pr) = pan_gains(p[1]);
        for (x, y) in l.iter_mut().zip(r.iter_mut()) {
            let (mut a, mut b) = (*x as f32 * g, *y as f32 * g);
            if flip_l {
                a = -a;
            }
            if flip_r {
                b = -b;
            }
            if swap {
                std::mem::swap(&mut a, &mut b);
            }
            let (m, s) = ((a + b) * 0.5, (a - b) * 0.5 * width);
            let (mut a, mut b) = (m + s, m - s);
            if mono {
                let both = (a + b) * 0.5;
                a = both;
                b = both;
            }
            if dc {
                for (v, (xi, yo)) in [&mut a, &mut b].into_iter().zip(self.dc.iter_mut()) {
                    let o = *v - *xi + 0.9995 * *yo;
                    *xi = *v;
                    *yo = o;
                    *v = o;
                }
            }
            *x = f64::from(a * pl);
            *y = f64::from(b * pr);
        }
    }

    pub fn reset(&mut self) {
        self.dc = [(0.0, 0.0); 2];
    }
}

#[derive(Clone, Debug)]
pub struct Width {
    sr: f32,
    side_hp: Biquad,
}

impl Width {
    pub fn new(sr: f32) -> Self {
        Self {
            sr,
            side_hp: Biquad::unity(),
        }
    }

    pub fn set(&mut self, p: &[f32; MAX_FX_PARAMS]) {
        let f = if p[1] > 21.0 {
            Biquad::make(Shape::High, p[1], 0.707, 0.0, self.sr)
        } else {
            Biquad::unity()
        };
        self.side_hp = Biquad {
            z1: self.side_hp.z1,
            z2: self.side_hp.z2,
            ..f
        };
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32, p: &[f32; MAX_FX_PARAMS]) -> (f32, f32) {
        let m = (l + r) * 0.5;
        // The sides above the mono frequency only, made wider or narrower.
        let s = self.side_hp.tick((l - r) * 0.5) * p[0];
        let (pl, pr) = pan_gains(p[2]);
        let o = gain(p[3]);
        ((m + s) * pl * o, (m - s) * pr * o)
    }

    pub fn reset(&mut self) {
        self.side_hp = self.side_hp.cleared();
    }
}

#[derive(Clone, Debug)]
pub struct Air {
    sr: f32,
    pre: [Biquad; 2],
    post: [Biquad; 2],
}

impl Air {
    pub fn new(sr: f32) -> Self {
        Self {
            sr,
            pre: [Biquad::unity(); 2],
            post: [Biquad::unity(); 2],
        }
    }

    pub fn set(&mut self, p: &[f32; MAX_FX_PARAMS]) {
        let f = Biquad::make(Shape::High, p[0], 0.707, 0.0, self.sr);
        for k in 0..2 {
            self.pre[k] = Biquad {
                z1: self.pre[k].z1,
                z2: self.pre[k].z2,
                ..f
            };
            self.post[k] = Biquad {
                z1: self.post[k].z1,
                z2: self.post[k].z2,
                ..f
            };
        }
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32, p: &[f32; MAX_FX_PARAMS]) -> (f32, f32) {
        let drive = gain(p[1] * 24.0);
        let amount = p[2].clamp(0.0, 1.0);
        let input = [l, r];
        let mut out = [0f32; 2];
        for ch in 0..2 {
            let top = self.pre[ch].tick(input[ch]);
            // Harmonics of the top only, then only the new top is kept.
            let h = self.post[ch].tick((top * drive).tanh() / drive.sqrt());
            out[ch] = input[ch] + h * amount;
        }
        let o = gain(p[3]);
        (out[0] * o, out[1] * o)
    }

    pub fn reset(&mut self) {
        for f in self.pre.iter_mut().chain(self.post.iter_mut()) {
            *f = f.cleared();
        }
    }
}
