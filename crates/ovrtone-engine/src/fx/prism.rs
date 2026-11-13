//! Calm, the resonance suppressor and de-esser, and Prism, the EQ: six bands that can each
//! move with the level of their own part of the spectrum (a dynamic EQ), followed by a
//! spectral Tame stage that smooths whatever still sticks out.
use super::parts::{choice, db, gain, Follower};
use super::spectral::{Settings, Spectral};
use super::{Biquad, Shape, MAX_FX_PARAMS};

pub const CALM_MODES: [&str; 2] = ["Resonance", "De-ess"];
pub const BANDS: usize = 6;
/// Numbers per Prism band: frequency, gain, Q, dynamic range, threshold.
pub const PER_BAND: usize = 5;
/// Prism's Tame and Output.
pub const TAME: usize = 30;
pub const OUTPUT: usize = 31;
/// How often a moving band works its filter out again, in samples.
const RECOMPUTE: u32 = 16;

fn calm_settings(p: &[f32; MAX_FX_PARAMS]) -> Settings {
    Settings {
        depth: p[0],
        sharpness: p[1],
        selectivity: p[2],
        speed: p[3],
        low_hz: p[4],
        high_hz: p[5],
        de_ess: choice(p[6], 2) == 1,
        delta: p[7] >= 0.5,
        mix: p[8],
    }
}

#[derive(Clone, Debug)]
pub struct Calm {
    pub spectral: Box<Spectral>,
}

impl Calm {
    pub fn new(sr: f32) -> Self {
        Self {
            spectral: Box::new(Spectral::new(sr)),
        }
    }

    pub fn set(&mut self, p: &[f32; MAX_FX_PARAMS]) {
        self.spectral.configure(calm_settings(p));
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32, p: &[f32; MAX_FX_PARAMS]) -> (f32, f32) {
        let (a, b) = self.spectral.tick(l, r);
        let g = gain(p[9]);
        (a * g, b * g)
    }

    pub fn reset(&mut self) {
        self.spectral.reset();
    }
}

/// The shape of Prism band `k`.
fn band_shape(k: usize) -> Shape {
    match k {
        0 => Shape::LowShelf,
        5 => Shape::HighShelf,
        _ => Shape::Peak,
    }
}

/// Band `k`'s filter at gain `gain_db`.
fn band_filter(p: &[f32; MAX_FX_PARAMS], k: usize, gain_db: f32, sr: f32) -> Biquad {
    let at = k * PER_BAND;
    Biquad::make(band_shape(k), p[at], p[at + 2], gain_db, sr)
}

/// Prism's six filters with their set (not dynamic) gains, for drawing its curve.
pub fn static_filters(p: &[f32; MAX_FX_PARAMS], sr: f32) -> [Biquad; BANDS] {
    std::array::from_fn(|k| band_filter(p, k, p[k * PER_BAND + 1], sr))
}

/// The gain band `k` has at its most when its dynamic range is all used.
pub fn band_reach(p: &[f32; MAX_FX_PARAMS], k: usize) -> f32 {
    p[k * PER_BAND + 1] + p[k * PER_BAND + 3]
}

/// Whether Tame is on: the spectral stage only runs (and only delays the sound) then.
pub fn tame_on(p: &[f32; MAX_FX_PARAMS]) -> bool {
    p[TAME] > 0.0
}

#[derive(Clone, Debug)]
pub struct Prism {
    sr: f32,
    filters: [[Biquad; BANDS]; 2],
    detectors: [Biquad; BANDS],
    followers: [Follower; BANDS],
    /// The gain each band has now, in dB.
    pub now_db: [f32; BANDS],
    countdown: u32,
    tame: Box<Spectral>,
    tame_on: bool,
}

impl Prism {
    pub fn new(sr: f32) -> Self {
        Self {
            sr,
            filters: [[Biquad::unity(); BANDS]; 2],
            detectors: [Biquad::unity(); BANDS],
            followers: [Follower::new(5.0, 120.0, sr); BANDS],
            now_db: [0.0; BANDS],
            countdown: 0,
            tame: Box::new(Spectral::new(sr)),
            tame_on: false,
        }
    }

    /// The settings the user set (`base`), which decide whether Tame runs, and those played.
    pub fn set(&mut self, p: &[f32; MAX_FX_PARAMS], base: &[f32; MAX_FX_PARAMS]) {
        for k in 0..BANDS {
            let at = k * PER_BAND;
            let det = Biquad::make(Shape::Band, p[at], p[at + 2].max(0.5), 0.0, self.sr);
            self.detectors[k] = Biquad {
                z1: self.detectors[k].z1,
                z2: self.detectors[k].z2,
                ..det
            };
            self.now_db[k] = p[at + 1];
            self.refilter(p, k);
        }
        let on = tame_on(base);
        if on != self.tame_on {
            self.tame.reset();
            self.tame_on = on;
        }
        self.tame.configure(Settings {
            depth: p[TAME],
            sharpness: 0.55,
            selectivity: 0.45,
            speed: 0.5,
            low_hz: 60.0,
            high_hz: 20_000.0,
            de_ess: false,
            delta: false,
            mix: 1.0,
        });
    }

    fn refilter(&mut self, p: &[f32; MAX_FX_PARAMS], k: usize) {
        let f = band_filter(p, k, self.now_db[k], self.sr);
        for ch in self.filters.iter_mut() {
            ch[k] = Biquad {
                z1: ch[k].z1,
                z2: ch[k].z2,
                ..f
            };
        }
    }

    pub fn reduction_db(&self) -> f32 {
        if self.tame_on {
            self.tame.most
        } else {
            0.0
        }
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32, p: &[f32; MAX_FX_PARAMS]) -> (f32, f32) {
        let mono = (l + r) * 0.5;
        let again = self.countdown == 0;
        self.countdown = if again { RECOMPUTE } else { self.countdown - 1 };
        for k in 0..BANDS {
            let at = k * PER_BAND;
            let range = p[at + 3];
            if range.abs() < 0.05 {
                continue;
            }
            let env = self.followers[k].tick(self.detectors[k].tick(mono).abs());
            if again {
                // Fully moved 12 dB above the threshold.
                let amount = ((db(env) - p[at + 4]) / 12.0).clamp(0.0, 1.0);
                let want = p[at + 1] + range * amount;
                if (want - self.now_db[k]).abs() > 0.05 {
                    self.now_db[k] = want;
                    self.refilter(p, k);
                }
            }
        }
        let [fl, fr] = &mut self.filters;
        let a = fl.iter_mut().fold(l, |x, f| f.tick(x));
        let b = fr.iter_mut().fold(r, |x, f| f.tick(x));
        let (a, b) = if self.tame_on {
            self.tame.tick(a, b)
        } else {
            (a, b)
        };
        let g = gain(p[OUTPUT]);
        (a * g, b * g)
    }

    pub fn reset(&mut self) {
        for f in self.filters.iter_mut().flatten() {
            *f = f.cleared();
        }
        for f in self.detectors.iter_mut() {
            *f = f.cleared();
        }
        for f in self.followers.iter_mut() {
            f.value = 0.0;
        }
        self.tame.reset();
    }
}
