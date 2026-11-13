//! Pulse, tremolo and auto-pan: the level (Tremolo) or the place (Pan) moved by a wave, free or in
//! note values at the project tempo, with the right side's wave set apart by Phase for a stereo
//! tremolo.
use super::parts::choice;
use super::sweep::wave;
use super::MAX_FX_PARAMS;

pub const MODES: [&str; 2] = ["Tremolo", "Pan"];

#[derive(Clone, Debug)]
pub struct Pulse {
    sr: f32,
    phase: f32,
    smooth: [f32; 2],
    pub bpm: f32,
}

impl Pulse {
    pub fn new(sr: f32) -> Self {
        Self {
            sr,
            phase: 0.0,
            smooth: [1.0; 2],
            bpm: 120.0,
        }
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32, p: &[f32; MAX_FX_PARAMS]) -> (f32, f32) {
        let ms = super::drift::synced(1000.0 / p[1].max(0.01), p[2], self.bpm);
        self.phase = (self.phase + 1000.0 / ms.max(1.0) / self.sr) % 1.0;
        let shape = choice(p[4], 4);
        let depth = p[3].clamp(0.0, 1.0);
        // A square wave is softened a little so it does not click.
        let soft = 1.0 - (-2000.0 / self.sr).exp();
        let (wl, wr) = (
            wave(shape, self.phase),
            wave(shape, self.phase + p[5] / 360.0),
        );
        let gains = if choice(p[0], 2) == 0 {
            [1.0 - depth * (0.5 - 0.5 * wl), 1.0 - depth * (0.5 - 0.5 * wr)]
        } else {
            // Equal power across the two sides.
            let pan = (wl * depth + 1.0) * 0.25 * std::f32::consts::PI;
            [pan.cos() * std::f32::consts::SQRT_2, pan.sin() * std::f32::consts::SQRT_2]
        };
        for (s, g) in self.smooth.iter_mut().zip(gains) {
            *s += (g - *s) * soft;
        }
        (l * self.smooth[0], r * self.smooth[1])
    }

    pub fn reset(&mut self) {
        self.smooth = [1.0; 2];
    }
}
