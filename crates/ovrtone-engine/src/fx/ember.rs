//! Ember, the distortion: six ways of bending the wave (tube, diode, fuzz, wave folding,
//! rectifying and bit crushing), with an input stage, a pre filter, a bias that makes the
//! shape lopsided (even harmonics), a tone filter after, dry/wet and output. Everything but
//! the crusher runs at twice the sample rate, so heavy drive stays clean of aliasing.
use std::f32::consts::FRAC_PI_2;

use super::parts::{choice, gain, Oversampler};
use super::{Biquad, Shape, MAX_FX_PARAMS};

pub const MODELS: [&str; 6] = ["Tube", "Diode", "Fuzz", "Fold", "Rectify", "Crush"];

/// The curve of model `model` for an input already multiplied by the drive.
#[inline]
pub fn shape(model: usize, x: f32, bias: f32) -> f32 {
    let b = bias * 0.6;
    match model {
        // A triode-like curve: soft, and lopsided by the bias.
        0 => (x + b).tanh() - b.tanh(),
        // A diode pair: sharper knee, one side earlier than the other with bias.
        1 => {
            let t = 1.0 + b.abs();
            if x >= 0.0 {
                (1.0 - (-x * t).exp()) / t.max(1.0)
            } else {
                -(1.0 - (x * (2.0 - t).max(0.2)).exp())
            }
        }
        // Fuzz: driven hard into a flat top, the bias sets the gating of small signals.
        2 => {
            let y = (3.0 * (x + b)).tanh() - (3.0 * b).tanh();
            y.clamp(-0.85, 0.85) / 0.85
        }
        // A sine folder: the louder it is driven, the more it folds back on itself.
        3 => (FRAC_PI_2 * (x + b)).sin(),
        // Full-wave rectifying, blended in by the bias, then rounded off.
        4 => {
            let amount = 0.5 + 0.5 * bias;
            let r = x * (1.0 - amount) + x.abs() * amount;
            (r * 1.2).tanh()
        }
        _ => x.clamp(-1.0, 1.0),
    }
}

#[derive(Clone, Debug)]
pub struct Ember {
    sr: f32,
    over: Oversampler,
    low_cut: [Biquad; 2],
    tone: [Biquad; 2],
    // Crusher: the held sample and how long until the next one.
    held: [f32; 2],
    hold: f32,
    dc: [(f32, f32); 2],
}

impl Ember {
    pub fn new(sr: f32) -> Self {
        Self {
            sr,
            over: Oversampler::new(sr),
            low_cut: [Biquad::unity(); 2],
            tone: [Biquad::unity(); 2],
            held: [0.0; 2],
            hold: 0.0,
            dc: [(0.0, 0.0); 2],
        }
    }

    pub fn set(&mut self, p: &[f32; MAX_FX_PARAMS]) {
        let lc = Biquad::make(Shape::High, p[5], 0.707, 0.0, self.sr);
        let tone = Biquad::make(Shape::Low, p[4], 0.707, 0.0, self.sr);
        for k in 0..2 {
            self.low_cut[k] = Biquad {
                z1: self.low_cut[k].z1,
                z2: self.low_cut[k].z2,
                ..lc
            };
            self.tone[k] = Biquad {
                z1: self.tone[k].z1,
                z2: self.tone[k].z2,
                ..tone
            };
        }
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32, p: &[f32; MAX_FX_PARAMS]) -> (f32, f32) {
        let input = gain(p[0]);
        let drive = p[1].clamp(0.0, 1.0);
        let model = choice(p[2], MODELS.len());
        let bias = p[3];
        let (mix, out) = (p[6], gain(p[7]));
        // Up to +36 dB into the curve, and back down so turning drive up is not only louder.
        let pre = gain(drive * 36.0);
        let make_up = 1.0 / (1.0 + drive * 2.0);
        let dry = [l, r];
        let mut wet = [0f32; 2];
        if model == 5 {
            // Fewer bits and a lower rate as the drive goes up.
            let bits = 16.0 - drive * 13.0;
            let steps = 2f32.powf(bits - 1.0);
            let every = 1.0 + drive * drive * 30.0;
            self.hold -= 1.0;
            let take = self.hold <= 0.0;
            if take {
                self.hold += every;
            }
            for ch in 0..2 {
                let x = self.low_cut[ch].tick(dry[ch] * input).clamp(-1.0, 1.0);
                if take {
                    self.held[ch] = (x * steps).round() / steps;
                }
                wet[ch] = self.held[ch];
            }
        } else {
            for ch in 0..2 {
                let x = self.low_cut[ch].tick(dry[ch] * input);
                wet[ch] = self.over.run(ch, x, |s| shape(model, s * pre, bias)) * make_up;
            }
        }
        let mut o = [0f32; 2];
        for ch in 0..2 {
            // The bias leaves DC behind: take it out.
            let (xi, yo) = self.dc[ch];
            let y = wet[ch] - xi + 0.995 * yo;
            self.dc[ch] = (wet[ch], y);
            let w = self.tone[ch].tick(y);
            o[ch] = (dry[ch] * (1.0 - mix) + w * mix) * out;
        }
        (o[0], o[1])
    }

    pub fn reset(&mut self) {
        self.over.reset();
        for f in self.low_cut.iter_mut().chain(self.tone.iter_mut()) {
            *f = f.cleared();
        }
        self.held = [0.0; 2];
        self.hold = 0.0;
        self.dc = [(0.0, 0.0); 2];
    }
}

/// The curve Ember applies at `drive` (0..1), for drawing: output for an input of -1..1.
pub fn curve(model: usize, drive: f32, bias: f32, x: f32) -> f32 {
    if model == 5 {
        let steps = 2f32.powf(16.0 - drive * 13.0 - 1.0).min(64.0);
        return (x * steps).round() / steps;
    }
    let pre = gain(drive.clamp(0.0, 1.0) * 36.0);
    let y = shape(model, x * pre, bias) - shape(model, 0.0, bias);
    // Drawn at full height whatever the make-up gain.
    let top = (1..=8)
        .map(|k| (shape(model, pre * k as f32 / 8.0, bias) - shape(model, 0.0, bias)).abs())
        .fold(1e-3f32, f32::max);
    (y / top).clamp(-1.0, 1.0)
}
