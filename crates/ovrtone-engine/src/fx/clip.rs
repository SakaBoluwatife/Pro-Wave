//! Clip, the clipper: drive into a ceiling with one of six curves, from a hard edge to a
//! gentle rounding, optionally at twice the rate so the clipped edge does not alias.
use std::f32::consts::{FRAC_2_PI, FRAC_PI_2};

use super::parts::{choice, gain, Oversampler};
use super::MAX_FX_PARAMS;

pub const SHAPES: [&str; 6] = ["Hard", "Soft", "Tanh", "Cubic", "Sine", "Arctan"];

/// Curve `shape` for an input scaled so that the ceiling is 1: output stays within -1..1.
#[inline]
pub fn shape(shape: usize, x: f32) -> f32 {
    match shape {
        0 => x.clamp(-1.0, 1.0),
        // Straight up to two thirds, then a smooth bend into the ceiling.
        1 => {
            let a = x.abs();
            let knee = 2.0 / 3.0;
            let y = if a <= knee {
                a
            } else if a >= 4.0 / 3.0 {
                1.0
            } else {
                let t = a - knee;
                knee + t - t * t * 0.75
            };
            y.min(1.0).copysign(x)
        }
        2 => x.tanh(),
        3 => {
            let c = x.clamp(-1.5, 1.5) / 1.5;
            1.5 * (c - c * c * c / 3.0)
        }
        4 => {
            if x.abs() >= 1.0 {
                1f32.copysign(x)
            } else {
                (FRAC_PI_2 * x).sin()
            }
        }
        _ => FRAC_2_PI * (FRAC_PI_2 * x * 1.5).atan() / (FRAC_2_PI * (FRAC_PI_2 * 1.5).atan()),
    }
    .clamp(-1.0, 1.0)
}

#[derive(Clone, Debug)]
pub struct Clip {
    over: Oversampler,
    /// How far over the ceiling the input went lately, in dB, for the meter.
    pub over_db: f32,
}

impl Clip {
    pub fn new(sr: f32) -> Self {
        Self {
            over: Oversampler::new(sr),
            over_db: 0.0,
        }
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32, p: &[f32; MAX_FX_PARAMS]) -> (f32, f32) {
        let input = gain(p[0]);
        let ceiling = gain(p[1]);
        let curve = choice(p[2], SHAPES.len());
        let twice = p[3] >= 0.5;
        let out = gain(p[4]);
        let x = [l * input / ceiling, r * input / ceiling];
        let peak = x[0].abs().max(x[1].abs());
        let over = if peak > 1.0 { 20.0 * peak.log10() } else { 0.0 };
        self.over_db = if over > self.over_db {
            over
        } else {
            self.over_db * 0.9995
        };
        let mut y = [0f32; 2];
        for ch in 0..2 {
            y[ch] = if twice {
                self.over.run(ch, x[ch], |s| shape(curve, s))
            } else {
                shape(curve, x[ch])
            } * ceiling
                * out;
        }
        (y[0], y[1])
    }

    pub fn reset(&mut self) {
        self.over.reset();
        self.over_db = 0.0;
    }
}
