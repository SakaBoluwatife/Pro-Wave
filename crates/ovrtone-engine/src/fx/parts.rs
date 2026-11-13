//! Pieces the Ovrtone effects share: level conversions, envelope speeds, a 2x oversampler and
//! a small FFT. Everything here is made off the audio thread and then runs without allocating.
use std::f32::consts::TAU;

use super::{Biquad, Shape};

/// dB to a gain.
#[inline]
pub fn gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// A gain to dB (silence is -120 dB).
#[inline]
pub fn db(g: f32) -> f32 {
    20.0 * g.abs().max(1e-6).log10()
}

/// The one-pole coefficient that reaches about 63 % of a step in `ms` milliseconds.
#[inline]
pub fn speed(ms: f32, sr: f32) -> f32 {
    1.0 - (-1.0 / (ms.max(0.01) * 0.001 * sr)).exp()
}

/// A parameter that is a choice, as its index.
#[inline]
pub fn choice(v: f32, count: usize) -> usize {
    (v.round().max(0.0) as usize).min(count.saturating_sub(1))
}

/// Runs a shaping function at twice the sample rate, so that the harmonics it makes above
/// half the rate are filtered away instead of folding back as noise. Fourth-order
/// Butterworth filters on the way up and down, per channel.
#[derive(Clone, Debug)]
pub struct Oversampler {
    up: [[Biquad; 2]; 2],
    down: [[Biquad; 2]; 2],
}

impl Oversampler {
    pub fn new(sr: f32) -> Self {
        // Designed at the doubled rate: the cutoff just under the original half rate.
        let up_rate = sr * 2.0;
        let cut = sr * 0.45;
        let pair = [
            Biquad::make(Shape::Low, cut, 0.541_196, 0.0, up_rate),
            Biquad::make(Shape::Low, cut, 1.306_563, 0.0, up_rate),
        ];
        Self {
            up: [pair, pair],
            down: [pair, pair],
        }
    }

    /// `x` of channel `ch` through `shape` at twice the rate.
    #[inline]
    pub fn run(&mut self, ch: usize, x: f32, mut shape: impl FnMut(f32) -> f32) -> f32 {
        let ch = ch & 1;
        let mut out = 0.0;
        for k in 0..2 {
            // A zero between each sample, made up for by doubling.
            let s = if k == 0 { x * 2.0 } else { 0.0 };
            let [a, b] = &mut self.up[ch];
            let s = b.tick(a.tick(s));
            let y = shape(s);
            let [a, b] = &mut self.down[ch];
            out = b.tick(a.tick(y));
        }
        out
    }

    pub fn reset(&mut self) {
        for f in self.up.iter_mut().chain(self.down.iter_mut()).flatten() {
            *f = f.cleared();
        }
    }
}

/// A level follower with separate attack and release times.
#[derive(Clone, Copy, Debug, Default)]
pub struct Follower {
    pub value: f32,
    attack: f32,
    release: f32,
}

impl Follower {
    pub fn new(attack_ms: f32, release_ms: f32, sr: f32) -> Self {
        Self {
            value: 0.0,
            attack: speed(attack_ms, sr),
            release: speed(release_ms, sr),
        }
    }

    pub fn times(&mut self, attack_ms: f32, release_ms: f32, sr: f32) {
        self.attack = speed(attack_ms, sr);
        self.release = speed(release_ms, sr);
    }

    #[inline]
    pub fn tick(&mut self, x: f32) -> f32 {
        let k = if x > self.value {
            self.attack
        } else {
            self.release
        };
        self.value += (x - self.value) * k;
        self.value
    }
}

/// An in-place radix-2 FFT of one size, with its tables made up front.
#[derive(Clone, Debug)]
pub struct Fft {
    n: usize,
    cos: Vec<f32>,
    sin: Vec<f32>,
    rev: Vec<usize>,
}

impl Fft {
    /// `n` must be a power of two.
    pub fn new(n: usize) -> Self {
        let n = n.next_power_of_two().max(4);
        let bits = n.trailing_zeros();
        let rev = (0..n)
            .map(|i| i.reverse_bits() >> (usize::BITS - bits))
            .collect();
        let (cos, sin) = (0..n / 2)
            .map(|k| {
                let a = -TAU * k as f32 / n as f32;
                (a.cos(), a.sin())
            })
            .unzip();
        Self { n, cos, sin, rev }
    }

    /// Forward (`inverse` false) or inverse transform of `re` and `im`, each `len` long. The
    /// inverse is scaled by 1/n, so a round trip gives the input back.
    pub fn run(&self, re: &mut [f32], im: &mut [f32], inverse: bool) {
        let n = self.n;
        if re.len() < n || im.len() < n {
            return;
        }
        for i in 0..n {
            let j = self.rev[i];
            if j > i {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let sign = if inverse { -1.0 } else { 1.0 };
        let mut size = 2;
        while size <= n {
            let half = size / 2;
            let step = n / size;
            for start in (0..n).step_by(size) {
                for k in 0..half {
                    let (wr, wi) = (self.cos[k * step], sign * self.sin[k * step]);
                    let (a, b) = (start + k, start + k + half);
                    let tr = re[b] * wr - im[b] * wi;
                    let ti = re[b] * wi + im[b] * wr;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            size *= 2;
        }
        if inverse {
            let scale = 1.0 / n as f32;
            re[..n].iter_mut().for_each(|v| *v *= scale);
            im[..n].iter_mut().for_each(|v| *v *= scale);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_round_trip_through_the_fft_gives_the_sound_back() {
        let fft = Fft::new(64);
        let input: Vec<f32> = (0..64).map(|i| (i as f32 * 0.3).sin()).collect();
        let (mut re, mut im) = (input.clone(), vec![0.0; 64]);
        fft.run(&mut re, &mut im, false);
        fft.run(&mut re, &mut im, true);
        for (a, b) in re.iter().zip(&input) {
            assert!((a - b).abs() < 1e-4);
        }
    }

    #[test]
    fn the_fft_finds_a_sine_in_its_bin() {
        let fft = Fft::new(64);
        let mut re: Vec<f32> = (0..64).map(|i| (TAU * 5.0 * i as f32 / 64.0).cos()).collect();
        let mut im = vec![0.0; 64];
        fft.run(&mut re, &mut im, false);
        let loudest = (0..32)
            .max_by(|a, b| re[*a].hypot(im[*a]).total_cmp(&re[*b].hypot(im[*b])))
            .unwrap_or(0);
        assert_eq!(loudest, 5);
    }

    #[test]
    fn the_oversampler_passes_a_low_tone_at_its_level() {
        let sr = 48_000.0;
        let mut o = Oversampler::new(sr);
        let mut peak = 0f32;
        for i in 0..4800 {
            let x = (TAU * 200.0 * i as f32 / sr).sin();
            let y = o.run(0, x, |s| s);
            if i > 2400 {
                peak = peak.max(y.abs());
            }
        }
        assert!((peak - 1.0).abs() < 0.05, "peak {peak}");
    }
}
