//! Bands, the multiband compressor: three bands split by Linkwitz-Riley crossovers (fourth
//! order, so the bands add back up flat), each with its own threshold, ratio and gain, one
//! attack and release for all, and dry/wet for the squashed-and-mixed sound.
use super::parts::{db, gain, speed};
use super::{Biquad, Shape, MAX_FX_PARAMS};

#[derive(Clone, Debug)]
pub struct Bands {
    sr: f32,
    /// Per channel: low pass twice and high pass twice at the low split, then at the high one.
    lo: [[Biquad; 4]; 2],
    hi: [[Biquad; 4]; 2],
    red: [f32; 3],
}

fn lr4(shape: Shape, f: f32, sr: f32) -> Biquad {
    Biquad::make(shape, f, std::f32::consts::FRAC_1_SQRT_2, 0.0, sr)
}

impl Bands {
    pub fn new(sr: f32) -> Self {
        Self {
            sr,
            lo: [[Biquad::unity(); 4]; 2],
            hi: [[Biquad::unity(); 4]; 2],
            red: [0.0; 3],
        }
    }

    pub fn set(&mut self, p: &[f32; MAX_FX_PARAMS]) {
        let (f1, f2) = (p[0].min(p[1] * 0.8), p[1]);
        let make = |f: f32, sr: f32| {
            [
                lr4(Shape::Low, f, sr),
                lr4(Shape::Low, f, sr),
                lr4(Shape::High, f, sr),
                lr4(Shape::High, f, sr),
            ]
        };
        let (a, b) = (make(f1, self.sr), make(f2, self.sr));
        for ch in 0..2 {
            for k in 0..4 {
                self.lo[ch][k] = Biquad {
                    z1: self.lo[ch][k].z1,
                    z2: self.lo[ch][k].z2,
                    ..a[k]
                };
                self.hi[ch][k] = Biquad {
                    z1: self.hi[ch][k].z1,
                    z2: self.hi[ch][k].z2,
                    ..b[k]
                };
            }
        }
    }

    pub fn reduction_db(&self) -> f32 {
        self.red.iter().fold(0.0, |a, b| a.max(*b))
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32, p: &[f32; MAX_FX_PARAMS]) -> (f32, f32) {
        let input = [l, r];
        let mut band = [[0f32; 3]; 2];
        for ch in 0..2 {
            let x = input[ch];
            let [a, b, c, d] = &mut self.lo[ch];
            let low = b.tick(a.tick(x));
            let rest = d.tick(c.tick(x));
            let [a, b, c, d] = &mut self.hi[ch];
            let mid = b.tick(a.tick(rest));
            let high = d.tick(c.tick(rest));
            band[ch] = [low, mid, high];
        }
        let (att, rel) = (speed(p[11], self.sr), speed(p[12], self.sr));
        let mut out = [0f32; 2];
        for b in 0..3 {
            let (thr, ratio, make_up) = (p[2 + b * 3], p[3 + b * 3], p[4 + b * 3]);
            let level = db(band[0][b].abs().max(band[1][b].abs()));
            let target = ((level - thr).max(0.0) * (1.0 - 1.0 / ratio.max(1.0))).min(48.0);
            let k = if target > self.red[b] { att } else { rel };
            self.red[b] += (target - self.red[b]) * k;
            let g = gain(make_up - self.red[b]);
            for ch in 0..2 {
                out[ch] += band[ch][b] * g;
            }
        }
        let (mix, o) = (p[13].clamp(0.0, 1.0), gain(p[14]));
        (
            (l * (1.0 - mix) + out[0] * mix) * o,
            (r * (1.0 - mix) + out[1] * mix) * o,
        )
    }

    pub fn reset(&mut self) {
        for f in self.lo.iter_mut().chain(self.hi.iter_mut()).flatten() {
            *f = f.cleared();
        }
        self.red = [0.0; 3];
    }
}
