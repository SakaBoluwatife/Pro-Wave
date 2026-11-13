//! Wall, the limiter: gain into a ceiling that nothing passes, with look-ahead so the gain is
//! already down when a peak arrives (no clipping, no clicks), a release, and three styles:
//! Clear (as set), Punch (lets go faster after short peaks) and Loud (rounds the peaks off
//! before limiting, for more level with less pumping).
//!
//! The lowest gain over the look-ahead window is kept with a running minimum (constant work
//! per sample, whatever the window) and smoothed with a running average of the same length.
use super::parts::{choice, gain, speed};
use super::MAX_FX_PARAMS;

pub const STYLES: [&str; 3] = ["Clear", "Punch", "Loud"];
pub const MAX_LOOKAHEAD_MS: f32 = 10.0;

/// The look-ahead window of these settings, in samples (the delay is one less).
pub fn window(p: &[f32; MAX_FX_PARAMS], sr: f32) -> usize {
    ((p[3].clamp(0.5, MAX_LOOKAHEAD_MS) * 0.001 * sr).round() as usize).max(2)
}

#[derive(Clone, Debug)]
pub struct Wall {
    sr: f32,
    n: usize,
    delay: [Vec<f32>; 2],
    pos: usize,
    // The running minimum: positions and values, oldest at `head`.
    min_at: Vec<usize>,
    min_v: Vec<f32>,
    head: usize,
    len: usize,
    avg: Vec<f32>,
    avg_sum: f64,
    count: usize,
    pub gain: f32,
    /// How long since the gain last went down, for Punch.
    since: f32,
}

impl Wall {
    pub fn new(sr: f32) -> Self {
        let cap = (MAX_LOOKAHEAD_MS * 0.001 * sr).round() as usize + 2;
        Self {
            sr,
            n: 2,
            delay: [vec![0.0; cap], vec![0.0; cap]],
            pos: 0,
            min_at: vec![0; cap],
            min_v: vec![1.0; cap],
            head: 0,
            len: 0,
            avg: vec![1.0; cap],
            avg_sum: 2.0,
            count: 0,
            gain: 1.0,
            since: 0.0,
        }
    }

    pub fn set(&mut self, p: &[f32; MAX_FX_PARAMS]) {
        let n = window(p, self.sr).min(self.delay[0].len() - 1);
        if n != self.n {
            self.n = n;
            self.reset();
        }
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32, p: &[f32; MAX_FX_PARAMS]) -> (f32, f32) {
        let n = self.n;
        let cap = self.min_v.len();
        let gain_in = gain(p[0]);
        let ceiling = gain(p[1]);
        let style = choice(p[4], STYLES.len());
        let (mut xl, mut xr) = (l * gain_in, r * gain_in);
        if style == 2 {
            let k = ceiling * 1.4;
            xl = k * (xl / k).tanh();
            xr = k * (xr / k).tanh();
        }
        let peak = xl.abs().max(xr.abs());
        let need = if peak > ceiling { ceiling / peak } else { 1.0 };
        let t = self.count;
        self.count = self.count.wrapping_add(1);
        // Drop the larger values from the back, then what fell out of the window in front.
        while self.len > 0 {
            let back = (self.head + self.len - 1) % cap;
            if self.min_v[back] >= need {
                self.len -= 1;
            } else {
                break;
            }
        }
        let at = (self.head + self.len) % cap;
        self.min_at[at] = t;
        self.min_v[at] = need;
        self.len += 1;
        while self.len > 0 && t.wrapping_sub(self.min_at[self.head]) >= n {
            self.head = (self.head + 1) % cap;
            self.len -= 1;
        }
        let lowest = self.min_v[self.head];
        let slot = t % n;
        self.avg_sum += f64::from(lowest) - f64::from(self.avg[slot]);
        self.avg[slot] = lowest;
        let smooth = ((self.avg_sum / n as f64) as f32).clamp(0.0, 1.0);
        if smooth < self.gain {
            self.gain = smooth;
            self.since = 0.0;
        } else {
            self.since += 1.0 / self.sr;
            let mut ms = p[2];
            if style == 1 {
                // A short peak lets go at a fifth of the release, a long run at all of it.
                ms *= (0.2 + self.since * 4.0).min(1.0);
            }
            self.gain += (smooth - self.gain) * speed(ms, self.sr);
        }
        self.delay[0][self.pos] = xl;
        self.delay[1][self.pos] = xr;
        let out_at = (self.pos + cap - (n - 1)) % cap;
        self.pos = (self.pos + 1) % cap;
        let (dl, dr) = (self.delay[0][out_at], self.delay[1][out_at]);
        (
            (dl * self.gain).clamp(-ceiling, ceiling),
            (dr * self.gain).clamp(-ceiling, ceiling),
        )
    }

    pub fn reset(&mut self) {
        self.delay.iter_mut().for_each(|d| d.fill(0.0));
        self.pos = 0;
        self.head = 0;
        self.len = 0;
        self.avg.fill(1.0);
        self.avg_sum = self.n as f64;
        self.count = 0;
        self.gain = 1.0;
        self.since = 0.0;
    }
}
