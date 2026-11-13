//! Press, the compressor: threshold, ratio, a soft knee, attack, release, hold, a range that
//! caps the reduction, look-ahead, make-up (by hand or automatic), peak or RMS detection,
//! stereo link, a high-pass on the detector, three styles and dry/wet.
//!
//! Clean is feed-forward with a log-domain detector; Punch listens to its own output
//! (feedback, as older hardware does), which lets the first hit through; Glue lets go
//! slower after long reduction and faster after short peaks.
use super::parts::{choice, db, gain, speed};
use super::{Biquad, Shape, MAX_FX_PARAMS};

pub const STYLES: [&str; 3] = ["Clean", "Punch", "Glue"];
/// The longest look-ahead, in milliseconds.
pub const MAX_LOOKAHEAD_MS: f32 = 10.0;

/// The reduction in dB (0 or more) for a level `over` dB above the threshold.
#[inline]
pub fn reduction_for(over: f32, ratio: f32, knee: f32) -> f32 {
    let slope = 1.0 - 1.0 / ratio.max(1.0);
    let half = knee * 0.5;
    if over <= -half {
        0.0
    } else if over >= half || knee <= 0.01 {
        over * slope
    } else {
        let x = over + half;
        slope * x * x / (2.0 * knee)
    }
}

/// Output level in dB for an input level in dB, for drawing the curve (no make-up).
pub fn transfer(p: &[f32; MAX_FX_PARAMS], input_db: f32) -> f32 {
    let r = reduction_for(input_db - p[0], p[1], p[2]).min(p[6]);
    input_db - r
}

/// The make-up gain the automatic setting gives: half the reduction a 0 dB signal gets.
pub fn auto_make_up(p: &[f32; MAX_FX_PARAMS]) -> f32 {
    reduction_for(-p[0], p[1], p[2]).min(p[6]) * 0.5
}

#[derive(Clone, Debug)]
pub struct Press {
    sr: f32,
    /// The reduction now, in dB (0 or more), per channel.
    red: [f32; 2],
    rms: [f32; 2],
    hold_left: [u32; 2],
    /// How long the reduction has been high, for the Glue release.
    held_for: [f32; 2],
    last_out: [f32; 2],
    detector_hp: [Biquad; 2],
    delay: [Vec<f32>; 2],
    pos: usize,
    /// Another strip's sound, when it keys the compressor (a side-chain).
    pub key: Option<(f32, f32)>,
}

impl Press {
    pub fn new(sr: f32) -> Self {
        let n = (MAX_LOOKAHEAD_MS * 0.001 * sr) as usize + 2;
        Self {
            sr,
            red: [0.0; 2],
            rms: [0.0; 2],
            hold_left: [0; 2],
            held_for: [0.0; 2],
            last_out: [0.0; 2],
            detector_hp: [Biquad::unity(); 2],
            delay: [vec![0.0; n], vec![0.0; n]],
            pos: 0,
            key: None,
        }
    }

    /// The look-ahead of these settings, in samples.
    pub fn lookahead(p: &[f32; MAX_FX_PARAMS], sr: f32) -> usize {
        (p[7].clamp(0.0, MAX_LOOKAHEAD_MS) * 0.001 * sr) as usize
    }

    pub fn set(&mut self, p: &[f32; MAX_FX_PARAMS]) {
        let hp = if p[12] > 21.0 {
            Biquad::make(Shape::High, p[12], 0.707, 0.0, self.sr)
        } else {
            Biquad::unity()
        };
        for f in self.detector_hp.iter_mut() {
            *f = Biquad {
                z1: f.z1,
                z2: f.z2,
                ..hp
            };
        }
    }

    pub fn reduction_db(&self) -> f32 {
        self.red[0].max(self.red[1])
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32, p: &[f32; MAX_FX_PARAMS], look: usize) -> (f32, f32) {
        let sr = self.sr;
        let style = choice(p[15], STYLES.len());
        let link = p[11].clamp(0.0, 1.0);
        let input = [l, r];
        // What the detector hears: the input, or (Punch) the last output.
        let heard = match self.key {
            Some((a, b)) => [a, b],
            None if style == 1 => self.last_out,
            None => input,
        };
        let mut level = [0f32; 2];
        for ch in 0..2 {
            let x = self.detector_hp[ch].tick(heard[ch]);
            level[ch] = if p[10] >= 0.5 {
                // RMS over about 10 ms.
                self.rms[ch] += (x * x - self.rms[ch]) * speed(10.0, sr);
                db(self.rms[ch].max(0.0).sqrt() * std::f32::consts::SQRT_2)
            } else {
                db(x)
            };
        }
        let loud = level[0].max(level[1]);
        let mut g = [1f32; 2];
        for ch in 0..2 {
            let lv = level[ch] + (loud - level[ch]) * link;
            let target = reduction_for(lv - p[0], p[1], p[2]).min(p[6]);
            if target >= self.red[ch] {
                let k = speed(p[3], sr);
                self.red[ch] += (target - self.red[ch]) * k;
                self.hold_left[ch] = (p[5] * 0.001 * sr) as u32;
                self.held_for[ch] += 1.0 / sr;
            } else if self.hold_left[ch] > 0 {
                self.hold_left[ch] -= 1;
            } else {
                let mut ms = p[4];
                if style == 2 {
                    // Long reduction lets go slowly, a short peak quickly.
                    let t = (self.held_for[ch] / 0.5).min(1.0);
                    ms *= 0.4 + 1.6 * t;
                }
                self.red[ch] += (target - self.red[ch]) * speed(ms, sr);
                self.held_for[ch] *= 0.9995;
            }
            g[ch] = gain(-self.red[ch]);
        }
        let make_up = if p[9] >= 0.5 {
            auto_make_up(p)
        } else {
            p[8]
        };
        let out = gain(make_up + p[14]);
        let mix = p[13].clamp(0.0, 1.0);
        // The look-ahead: the sound comes out late, the gain does not.
        let n = self.delay[0].len();
        self.delay[0][self.pos] = l;
        self.delay[1][self.pos] = r;
        let back = (self.pos + n - look.min(n - 1)) % n;
        let (dl, dr) = (self.delay[0][back], self.delay[1][back]);
        self.pos = (self.pos + 1) % n;
        let ol = (dl * g[0] * mix + dl * (1.0 - mix)) * out;
        let or = (dr * g[1] * mix + dr * (1.0 - mix)) * out;
        self.last_out = [dl * g[0], dr * g[1]];
        (ol, or)
    }

    pub fn reset(&mut self) {
        self.red = [0.0; 2];
        self.rms = [0.0; 2];
        self.hold_left = [0; 2];
        self.held_for = [0.0; 2];
        self.last_out = [0.0; 2];
        self.detector_hp.iter_mut().for_each(|f| *f = f.cleared());
        self.delay.iter_mut().for_each(|d| d.fill(0.0));
        self.pos = 0;
    }
}
