//! The spectral processor behind Calm (and the Tame stage of Prism): it finds the parts of
//! the spectrum that stick out above their surroundings (resonances, harsh peaks, sibilance)
//! and turns them down as they happen, frame by frame.
//!
//! Short-time Fourier transform: 1024-point frames every 256 samples, square-root Hann
//! windows in and out (they overlap-add back to the original), so the sound comes out
//! [`LATENCY`] samples late. In each frame the level of every bin is compared with a
//! smoothed copy of the spectrum (smoothed over a share of an octave, so "surroundings" means
//! the same musically at any pitch); what is more than the selectivity above it is reduced in
//! proportion to Depth, with attack and release per bin. Both channels get the same
//! reduction, so the image does not move.
use std::f32::consts::TAU;

use super::parts::Fft;

pub const SIZE: usize = 1024;
pub const HOP: usize = SIZE / 4;
/// How late the sound comes out, in samples.
pub const LATENCY: usize = SIZE;
/// Where a frame's new samples start.
const START: usize = SIZE - HOP;
const BINS: usize = SIZE / 2 + 1;

/// What to listen for.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Settings {
    /// 0..1: how much of what sticks out is taken away.
    pub depth: f32,
    /// 0..1: narrow peaks only (1) or broader areas too (0).
    pub sharpness: f32,
    /// 0..1: how far a bin must stick out before it is touched (2 to 14 dB).
    pub selectivity: f32,
    /// 0..1: slow (0) to fast (1) reaction.
    pub speed: f32,
    pub low_hz: f32,
    pub high_hz: f32,
    /// Listen for sibilance against the body of the sound instead of for resonances.
    pub de_ess: bool,
    /// Hear only what is taken away.
    pub delta: bool,
    pub mix: f32,
}

#[derive(Clone, Debug)]
pub struct Spectral {
    sr: f32,
    fft: Fft,
    window: Vec<f32>,
    input: [Vec<f32>; 2],
    output: [Vec<f32>; 2],
    acc: [Vec<f32>; 2],
    dry: [Vec<f32>; 2],
    rover: usize,
    dry_at: usize,
    re: [Vec<f32>; 2],
    im: [Vec<f32>; 2],
    level: Vec<f32>,
    smooth: Vec<f32>,
    /// The one-pole coefficient of the smoothing at each bin.
    spread: Vec<f32>,
    /// The reduction of each bin now, in dB.
    pub reduction: Vec<f32>,
    settings: Settings,
    /// The largest reduction in the last frame, in dB.
    pub most: f32,
}

impl Spectral {
    pub fn new(sr: f32) -> Self {
        let window = (0..SIZE)
            .map(|i| (0.5 - 0.5 * (TAU * i as f32 / SIZE as f32).cos()).sqrt())
            .collect();
        let two = |n: usize| [vec![0.0; n], vec![0.0; n]];
        let mut s = Self {
            sr,
            fft: Fft::new(SIZE),
            window,
            input: two(SIZE),
            output: two(SIZE),
            acc: two(2 * SIZE),
            dry: two(SIZE),
            rover: START,
            dry_at: 0,
            re: two(SIZE),
            im: two(SIZE),
            level: vec![0.0; BINS],
            smooth: vec![0.0; BINS],
            spread: vec![0.0; BINS],
            reduction: vec![0.0; BINS],
            settings: Settings::default(),
            most: 0.0,
        };
        s.configure(Settings {
            depth: 0.5,
            sharpness: 0.5,
            selectivity: 0.5,
            speed: 0.5,
            low_hz: 20.0,
            high_hz: 20_000.0,
            de_ess: false,
            delta: false,
            mix: 1.0,
        });
        s
    }

    /// New settings; cheap enough for the audio thread (no allocation).
    pub fn configure(&mut self, s: Settings) {
        if s.sharpness != self.settings.sharpness {
            // From two thirds of an octave (broad) down to a twelfth (sharp).
            let octaves = 0.66 - s.sharpness.clamp(0.0, 1.0) * 0.58;
            for (k, a) in self.spread.iter_mut().enumerate() {
                let width = (k.max(1) as f32 * (2f32.powf(octaves) - 1.0)).max(1.0);
                *a = (-1.0 / width).exp();
            }
        }
        self.settings = s;
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        let i = self.rover;
        let back = i - START;
        self.input[0][i] = l;
        self.input[1][i] = r;
        let (wl, wr) = (self.output[0][back], self.output[1][back]);
        let d = self.dry_at;
        let (dl, dr) = (self.dry[0][d], self.dry[1][d]);
        self.dry[0][d] = l;
        self.dry[1][d] = r;
        self.dry_at = (d + 1) % SIZE;
        self.rover += 1;
        if self.rover >= SIZE {
            self.rover = START;
            self.frame();
        }
        let s = self.settings;
        let mix = s.mix.clamp(0.0, 1.0);
        if s.delta {
            ((dl - wl) * mix, (dr - wr) * mix)
        } else {
            (dl + (wl - dl) * mix, dr + (wr - dr) * mix)
        }
    }

    fn frame(&mut self) {
        let s = self.settings;
        for ch in 0..2 {
            for k in 0..SIZE {
                self.re[ch][k] = self.input[ch][k] * self.window[k];
                self.im[ch][k] = 0.0;
            }
            let (re, im) = (&mut self.re[ch], &mut self.im[ch]);
            self.fft.run(re, im, false);
        }
        for k in 0..BINS {
            let p = (0..2)
                .map(|ch| self.re[ch][k] * self.re[ch][k] + self.im[ch][k] * self.im[ch][k])
                .fold(0.0f32, f32::max);
            self.level[k] = 10.0 * (p + 1e-12).log10();
        }
        // The surroundings: smoothed forwards and backwards across the bins.
        let mut v = self.level[0];
        for k in 0..BINS {
            v = self.level[k] + (v - self.level[k]) * self.spread[k];
            self.smooth[k] = v;
        }
        let mut v = self.smooth[BINS - 1];
        for k in (0..BINS).rev() {
            v = self.smooth[k] + (v - self.smooth[k]) * self.spread[k];
            self.smooth[k] = v;
        }
        let hz_per_bin = self.sr / SIZE as f32;
        let lo = (s.low_hz.min(s.high_hz) / hz_per_bin) as usize;
        let hi = ((s.low_hz.max(s.high_hz) / hz_per_bin) as usize).min(BINS - 1);
        let body = if s.de_ess {
            // The average level of the body of the sound, 150 Hz to 3 kHz.
            let (a, b) = ((150.0 / hz_per_bin) as usize, (3000.0 / hz_per_bin) as usize);
            let b = b.clamp(a + 1, BINS - 1);
            self.level[a..b].iter().sum::<f32>() / (b - a) as f32
        } else {
            f32::MIN
        };
        let threshold = 2.0 + s.selectivity.clamp(0.0, 1.0) * 12.0;
        let frame_ms = HOP as f32 / self.sr * 1000.0;
        let ms = |base: f32| base * (1.6 - s.speed.clamp(0.0, 1.0) * 1.4);
        let attack = 1.0 - (-frame_ms / ms(8.0)).exp();
        let release = 1.0 - (-frame_ms / ms(120.0)).exp();
        let depth = s.depth.clamp(0.0, 1.0);
        let mut most = 0f32;
        for k in 0..BINS {
            let target = if k < lo || k > hi || depth <= 0.0 {
                0.0
            } else {
                let reference = if s.de_ess {
                    self.smooth[k].max(body)
                } else {
                    self.smooth[k]
                };
                ((self.level[k] - reference - threshold).max(0.0) * depth * 1.5).min(30.0)
            };
            let g = &mut self.reduction[k];
            *g += (target - *g) * if target > *g { attack } else { release };
            most = most.max(*g);
        }
        self.most = most;
        for ch in 0..2 {
            for k in 0..BINS {
                let g = 10f32.powf(-self.reduction[k] / 20.0);
                self.re[ch][k] *= g;
                self.im[ch][k] *= g;
                if k > 0 && k < SIZE / 2 {
                    // The mirror half of a real signal.
                    self.re[ch][SIZE - k] = self.re[ch][k];
                    self.im[ch][SIZE - k] = -self.im[ch][k];
                }
            }
            let (re, im) = (&mut self.re[ch], &mut self.im[ch]);
            self.fft.run(re, im, true);
            // Square-root Hann twice is Hann, and Hann frames a quarter apart add up to 2.
            for k in 0..SIZE {
                self.acc[ch][k] += self.re[ch][k] * self.window[k] * 0.5;
            }
            self.output[ch][..HOP].copy_from_slice(&self.acc[ch][..HOP]);
            self.acc[ch].copy_within(HOP..2 * SIZE, 0);
            let end = 2 * SIZE;
            self.acc[ch][end - HOP..end].fill(0.0);
            self.input[ch].copy_within(HOP..SIZE, 0);
        }
    }

    pub fn reset(&mut self) {
        for ch in 0..2 {
            self.input[ch].fill(0.0);
            self.output[ch].fill(0.0);
            self.acc[ch].fill(0.0);
            self.dry[ch].fill(0.0);
        }
        self.reduction.fill(0.0);
        self.rover = START;
        self.dry_at = 0;
        self.most = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_no_depth_the_sound_comes_through_late_but_unchanged() {
        let mut s = Spectral::new(48_000.0);
        let mut set = s.settings;
        set.depth = 0.0;
        s.configure(set);
        let input: Vec<f32> = (0..8192).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        let out: Vec<f32> = input.iter().map(|x| s.tick(*x, *x).0).collect();
        for i in 3000..8000 {
            assert!((out[i] - input[i - LATENCY]).abs() < 1e-3, "at {i}");
        }
    }

    #[test]
    fn a_whistle_over_noise_is_turned_down() {
        let mut s = Spectral::new(48_000.0);
        let mut seed = 1u32;
        let mut noise = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        };
        let mut before = 0.0;
        let mut after = 0.0;
        for i in 0..48_000 {
            let tone = (TAU * 3000.0 * i as f32 / 48_000.0).sin() * 0.5;
            let x = noise() * 0.05 + tone;
            let (y, _) = s.tick(x, x);
            if i > 24_000 {
                before += tone * tone;
                after += y * y;
            }
        }
        assert!(after < before * 0.7, "{after} vs {before}");
    }
}
