//! Swirl, the chorus. Modes I, II and I+II work the way the classic two-button analog
//! string-machine chorus does (one triangle wave sweeping a short delay, opposite on the
//! two sides, at a slow rate for I, a faster and deeper one for II, and both together);
//! Wide adds two more voices a quarter turn apart. Rate scales the sweep, Depth its size,
//! Tone darkens the delayed copies the way a bucket-brigade chip does.
use super::parts::choice;
use super::{Line, MAX_FX_PARAMS};

pub const MODES: [&str; 4] = ["I", "II", "I+II", "Wide"];

#[inline]
fn triangle(phase: f32) -> f32 {
    let p = phase.fract();
    if p < 0.5 {
        4.0 * p - 1.0
    } else {
        3.0 - 4.0 * p
    }
}

#[derive(Clone, Debug)]
pub struct Swirl {
    sr: f32,
    lines: [Line; 2],
    phase: [f32; 2],
    tone: [f32; 2],
}

impl Swirl {
    pub fn new(sr: f32) -> Self {
        Self {
            sr,
            lines: [Line::new((0.03 * sr) as usize), Line::new((0.03 * sr) as usize)],
            phase: [0.0, 0.31],
            tone: [0.0; 2],
        }
    }

    #[inline]
    pub fn tick(&mut self, l: f32, r: f32, p: &[f32; MAX_FX_PARAMS]) -> (f32, f32) {
        let sr = self.sr;
        let mode = choice(p[0], MODES.len());
        let rate = p[1];
        let depth = p[2].clamp(0.0, 1.0);
        let width = p[3].clamp(0.0, 1.0);
        let tone = 1.0 - (-std::f32::consts::TAU * p[4] / sr).exp();
        let mix = p[5].clamp(0.0, 1.0);
        // Rates and sweep sizes (ms) of the two waves; Rate 0.5 Hz is the original I speed.
        let scale = rate / 0.5;
        let (r1, r2) = (0.5 * scale, 0.83 * scale);
        self.phase[0] = (self.phase[0] + r1 / sr) % 1.0;
        self.phase[1] = (self.phase[1] + r2 / sr) % 1.0;
        let ms = sr * 0.001;
        let centre = 3.5 * ms;
        let swing = depth * 1.9 * ms;
        let (a, b) = (triangle(self.phase[0]), triangle(self.phase[1]));
        self.lines[0].push(l);
        self.lines[1].push(r);
        let read = |line: &Line, m: f32| line.read(centre + swing * m);
        let (wl, wr) = match mode {
            0 => (read(&self.lines[0], a), read(&self.lines[1], -a)),
            1 => (read(&self.lines[0], b * 1.3), read(&self.lines[1], -b * 1.3)),
            2 => (
                (read(&self.lines[0], a) + read(&self.lines[0], b * 0.6)) * 0.6,
                (read(&self.lines[1], -a) + read(&self.lines[1], -b * 0.6)) * 0.6,
            ),
            _ => {
                let c = triangle(self.phase[0] + 0.25);
                (
                    (read(&self.lines[0], a) + read(&self.lines[1], c)) * 0.6,
                    (read(&self.lines[1], -a) + read(&self.lines[0], -c)) * 0.6,
                )
            }
        };
        self.tone[0] += (wl - self.tone[0]) * tone;
        self.tone[1] += (wr - self.tone[1]) * tone;
        let (m, s) = (
            (self.tone[0] + self.tone[1]) * 0.5,
            (self.tone[0] - self.tone[1]) * 0.5 * width,
        );
        (l * (1.0 - mix * 0.5) + (m + s) * mix, r * (1.0 - mix * 0.5) + (m - s) * mix)
    }

    pub fn reset(&mut self) {
        for l in self.lines.iter_mut() {
            l.buf.fill(0.0);
        }
        self.tone = [0.0; 2];
    }
}
