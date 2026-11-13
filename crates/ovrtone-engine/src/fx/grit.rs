//! Grit, the lo-fi box: a lower sample rate (held samples, with the aliasing that brings), fewer
//! bits, the hiss and crackle of a worn record, and a tone filter to darken it.
use super::parts::gain;
use super::{Biquad, Shape, MAX_FX_PARAMS};

#[derive(Clone, Debug)]
pub struct Grit {
    sr: f32,
    held: [f32; 2],
    phase: f32,
    tone: [Biquad; 2],
    seed: u32,
    crackle: f32,
}

impl Grit {
    pub fn new(sr: f32) -> Self {
        Self {
            sr,
            held: [0.0; 2],
            phase: 1.0,
            tone: [Biquad::unity(); 2],
            seed: 0x1357_9bdf,
            crackle: 0.0,
        }
    }

    pub fn set(&mut self, p: &[f32; MAX_FX_PARAMS]) {
        let f = Biquad::make(Shape::Low, p[4], 0.707, 0.0, self.sr);
        for t in self.tone.iter_mut() {
            *t = Biquad {
                z1: t.z1,
                z2: t.z2,
                ..f
            };
        }
    }

    fn rand(&mut self) -> f32 {
        self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.seed >> 8) as f32 / (1u32 << 24) as f32
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32, p: &[f32; MAX_FX_PARAMS]) -> (f32, f32) {
        let step = (p[0] / self.sr).min(1.0);
        self.phase += step;
        let take = self.phase >= 1.0;
        if take {
            self.phase -= 1.0;
        }
        let steps = 2f32.powf(p[1] - 1.0);
        let input = [l, r];
        let noise = p[2];
        // A crackle now and then, more of them as the knob goes up, each a short decaying click.
        if self.rand() < p[3] * p[3] * 0.0015 {
            self.crackle = (self.rand() - 0.5) * 0.6;
        }
        self.crackle *= 0.92;
        let mut out = [0f32; 2];
        for ch in 0..2 {
            if take {
                self.held[ch] = (input[ch].clamp(-1.0, 1.0) * steps).round() / steps;
            }
            let hiss = (self.rand() - 0.5) * noise * 0.05;
            out[ch] = self.tone[ch].tick(self.held[ch] + hiss + self.crackle * p[3]);
        }
        let (mix, o) = (p[5].clamp(0.0, 1.0), gain(p[6]));
        (
            (l * (1.0 - mix) + out[0] * mix) * o,
            (r * (1.0 - mix) + out[1] * mix) * o,
        )
    }

    pub fn reset(&mut self) {
        self.held = [0.0; 2];
        self.crackle = 0.0;
        for t in self.tone.iter_mut() {
            *t = t.cleared();
        }
    }
}
