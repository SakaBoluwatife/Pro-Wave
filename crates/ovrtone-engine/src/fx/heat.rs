//! Heat, saturation with a compressor in front of it: the input pushes the sound into both,
//! so turning it up makes it denser and warmer together. Three characters (Warm: tape-like
//! and rounded, Bright: lopsided like a valve with more top, Crush: the shaped sound with a
//! lower bit depth folded in), a Spread that saturates the sides more than the middle, a tilt
//! Tone, dry/wet and output.
use super::parts::{choice, db, gain, speed, Oversampler};
use super::{Biquad, Shape, MAX_FX_PARAMS};

pub const CHARACTERS: [&str; 3] = ["Warm", "Bright", "Crush"];

/// The saturation curve of `character` for an input already multiplied by the drive.
#[inline]
pub fn shape(character: usize, x: f32) -> f32 {
    match character {
        0 => {
            // Tape: rounded, a little compression before it bends.
            let y = x / (1.0 + x.abs()).sqrt();
            y.tanh()
        }
        1 => (x + 0.15).tanh() - 0.15f32.tanh(),
        _ => {
            let y = (x * 1.4).tanh();
            (y * 24.0).round() / 24.0 * 0.6 + y * 0.4
        }
    }
}

#[derive(Clone, Debug)]
pub struct Heat {
    sr: f32,
    over: Oversampler,
    /// Gain reduction now, in dB (0 or more).
    pub reduction: f32,
    tilt_low: [Biquad; 2],
    tilt_high: [Biquad; 2],
    dc: [(f32, f32); 2],
}

impl Heat {
    pub fn new(sr: f32) -> Self {
        Self {
            sr,
            over: Oversampler::new(sr),
            reduction: 0.0,
            tilt_low: [Biquad::unity(); 2],
            tilt_high: [Biquad::unity(); 2],
            dc: [(0.0, 0.0); 2],
        }
    }

    pub fn set(&mut self, p: &[f32; MAX_FX_PARAMS]) {
        // A tilt round 1 kHz: up to 6 dB more top and less bottom, or the other way.
        let t = p[5].clamp(-1.0, 1.0) * 6.0;
        let lo = Biquad::make(Shape::LowShelf, 250.0, 0.7, -t, self.sr);
        let hi = Biquad::make(Shape::HighShelf, 4000.0, 0.7, t, self.sr);
        for k in 0..2 {
            self.tilt_low[k] = Biquad {
                z1: self.tilt_low[k].z1,
                z2: self.tilt_low[k].z2,
                ..lo
            };
            self.tilt_high[k] = Biquad {
                z1: self.tilt_high[k].z1,
                z2: self.tilt_high[k].z2,
                ..hi
            };
        }
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32, p: &[f32; MAX_FX_PARAMS]) -> (f32, f32) {
        let input = gain(p[0]);
        let amount = p[1].clamp(0.0, 1.0);
        let squash = p[2].clamp(0.0, 1.0);
        let spread = p[3].clamp(0.0, 1.0);
        let character = choice(p[4], CHARACTERS.len());
        let (mix, out) = (p[6], gain(p[7]));
        let (xl, xr) = (l * input, r * input);
        // The compressor: a threshold that comes down with Squash, ratio 4:1, fast attack.
        let level = db(xl.abs().max(xr.abs()));
        let threshold = -squash * 30.0;
        let over = (level - threshold).max(0.0);
        // 4:1, faded in over the first quarter of the knob so that 0 is no compression.
        let target = over * 0.75 * (squash * 4.0).min(1.0);
        let k = if target > self.reduction {
            speed(3.0, self.sr)
        } else {
            speed(120.0, self.sr)
        };
        self.reduction += (target - self.reduction) * k;
        let g = gain(-self.reduction + squash * 8.0);
        // Middle and sides, the sides driven harder by Spread.
        let (m, s) = ((xl + xr) * 0.5 * g, (xl - xr) * 0.5 * g);
        let drive = gain(amount * 24.0);
        let back = 1.0 / (1.0 + amount * 1.5);
        let sm = self.over.run(0, m, |x| shape(character, x * drive)) * back;
        let side_drive = drive * (1.0 + spread * 2.0);
        let ss = self.over.run(1, s, |x| shape(character, x * side_drive)) * back * (1.0 + spread);
        let wet = [sm + ss, sm - ss];
        let dry = [l, r];
        let mut o = [0f32; 2];
        for ch in 0..2 {
            let (xi, yo) = self.dc[ch];
            let y = wet[ch] - xi + 0.995 * yo;
            self.dc[ch] = (wet[ch], y);
            let w = self.tilt_high[ch].tick(self.tilt_low[ch].tick(y));
            o[ch] = (dry[ch] * (1.0 - mix) + w * mix) * out;
        }
        (o[0], o[1])
    }

    pub fn reset(&mut self) {
        self.over.reset();
        self.reduction = 0.0;
        for f in self.tilt_low.iter_mut().chain(self.tilt_high.iter_mut()) {
            *f = f.cleared();
        }
        self.dc = [(0.0, 0.0); 2];
    }
}
