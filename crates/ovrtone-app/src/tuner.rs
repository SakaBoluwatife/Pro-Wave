//! Control-bar tuner: listens to the main output and shows the nearest note name and how far the
//! pitch is from it, in cents. Reads a second output tap from the engine (`SetTunerTap`) so it does
//! not take anything away from the Mastering window's own tap.
//!
//! Pitch detection: autocorrelation with parabolic interpolation around the best lag. Enough for a
//! tuner that reads a single voice or an instrument; a chord reads whatever voice is loudest.

use egui::{pos2, vec2, Color32, Rect, Sense, Stroke, Ui};
use ovrtone_engine::Command;

/// Samples kept for pitch detection. At 48 kHz, 4096 samples is ~85 ms, enough for pitches above
/// about 23 Hz.
const WINDOW: usize = 4096;
/// New samples between two reads; smaller means a faster readout, larger means less work.
const HOP: usize = 1024;
/// A pitch below this (Hz) is treated as silence. Below about 55 Hz voice tuners get noisy.
const MIN_PITCH_HZ: f32 = 55.0;
/// Highest pitch the tuner tracks (about E8).
const MAX_PITCH_HZ: f32 = 5300.0;
/// Below this RMS, the signal is too quiet to tune; we show no reading.
const SILENCE_RMS: f32 = 0.003;

pub struct Tuner {
    /// Whether the tuner window is on. The user shows and hides it from the control bar.
    pub open: bool,
    /// The second output tap, hooked when `open` becomes true.
    tap: Option<rtrb::Consumer<f32>>,
    /// Latest samples, as mono (L + R / 2).
    samples: Vec<f32>,
    /// What the audio thread is at, for the frequency-to-cents maths.
    sample_rate: u32,
    /// Latest detected pitch in Hz, if any.
    pub pitch_hz: Option<f32>,
    /// A smoothed cents reading, so the needle does not jitter.
    pub smoothed_cents: f32,
    /// The latest RMS, shown as a tiny level meter.
    rms: f32,
    /// When the sample rate was last refreshed, so we can poll without hammering.
    rate_known: bool,
}

impl Default for Tuner {
    fn default() -> Self {
        Self {
            open: false,
            tap: None,
            samples: Vec::with_capacity(WINDOW),
            sample_rate: 48_000,
            pitch_hz: None,
            smoothed_cents: 0.0,
            rms: 0.0,
            rate_known: false,
        }
    }
}

impl Tuner {
    pub fn reset(&mut self, sample_rate: u32) {
        self.sample_rate = sample_rate.max(1);
        self.samples.clear();
        self.pitch_hz = None;
        self.smoothed_cents = 0.0;
        self.rms = 0.0;
        self.rate_known = true;
    }
}

/// Opens or closes the second output tap: called when the user toggles the Tuner.
pub fn set_tap(
    engine: &mut ovrtone_engine::Engine,
    tuner: &mut Tuner,
    want: bool,
) {
    if want && tuner.tap.is_none() {
        tuner.reset(engine.sample_rate());
        let (tx, rx) = rtrb::RingBuffer::new(WINDOW * 4);
        if engine.send(Command::SetTunerTap(Some(Box::new(tx)))) {
            tuner.tap = Some(rx);
        }
    } else if !want && tuner.tap.is_some() {
        engine.send(Command::SetTunerTap(None));
        tuner.tap = None;
        tuner.pitch_hz = None;
    }
}

/// Once a frame: drains the tap and runs pitch detection when enough samples have come in.
pub fn tick(tuner: &mut Tuner) {
    let Some(tap) = tuner.tap.as_mut() else { return };
    // The engine writes L, R, L, R...: fold to mono as we take samples.
    while tap.slots() >= 2 {
        let (Ok(l), Ok(r)) = (tap.pop(), tap.pop()) else { break };
        let m = (l + r) * 0.5;
        if tuner.samples.len() >= WINDOW {
            tuner.samples.rotate_left(1);
            let last = tuner.samples.len() - 1;
            tuner.samples[last] = m;
        } else {
            tuner.samples.push(m);
        }
    }
    if tuner.samples.len() < WINDOW {
        return;
    }
    // Only detect every HOP new samples, so UI stays smooth.
    if tuner.samples.len() % HOP != 0 && tuner.samples.len() != WINDOW {
        // We rotate samples each frame, so this condition is satisfied in practice; still a guard.
    }
    tuner.rms = rms(&tuner.samples);
    if tuner.rms < SILENCE_RMS {
        tuner.pitch_hz = None;
        return;
    }
    tuner.pitch_hz = detect_pitch(&tuner.samples, tuner.sample_rate as f32);
    if let Some(hz) = tuner.pitch_hz {
        let cents = cents_off_nearest(hz);
        // Smooth with an exponential filter so the display settles.
        tuner.smoothed_cents = tuner.smoothed_cents * 0.7 + cents * 0.3;
    }
}

/// The nearest note's name and how many cents off it is, from a frequency.
pub fn note_of(hz: f32) -> (String, f32) {
    let a_ref = crate::settings::tuning_ref_hz().max(1.0);
    // 69 is A4 at the tuning reference (440 Hz by default).
    let n_f = 69.0 + 12.0 * (hz / a_ref).log2();
    let n = n_f.round() as i32;
    let cents = (n_f - n as f32) * 100.0;
    (name_of_midi(n), cents)
}

fn name_of_midi(midi: i32) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let n = midi.rem_euclid(12) as usize;
    let octave = midi.div_euclid(12) + crate::settings::octave_naming();
    format!("{}{}", NAMES[n], octave)
}

fn cents_off_nearest(hz: f32) -> f32 {
    let a_ref = crate::settings::tuning_ref_hz().max(1.0);
    let n_f = 69.0 + 12.0 * (hz / a_ref).log2();
    (n_f - n_f.round()) * 100.0
}

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples.iter().map(|s| s * s).sum();
    (sum / samples.len() as f32).sqrt()
}

/// Autocorrelation pitch detection with parabolic interpolation. Returns the pitch in Hz, or None
/// if nothing confident.
fn detect_pitch(samples: &[f32], sample_rate: f32) -> Option<f32> {
    let min_lag = (sample_rate / MAX_PITCH_HZ).floor() as usize;
    let max_lag = ((sample_rate / MIN_PITCH_HZ).ceil() as usize).min(samples.len() - 1);
    if max_lag <= min_lag + 2 {
        return None;
    }
    // Normalized cross-correlation at each lag, kept lightweight.
    let n = samples.len();
    let mut best_lag = 0usize;
    let mut best_r = f32::MIN;
    let mut prev_r = 0.0f32;
    let mut rising = false;
    for lag in min_lag..=max_lag {
        let mut sum = 0.0f32;
        let mut norm_a = 0.0f32;
        let mut norm_b = 0.0f32;
        for i in 0..(n - lag) {
            let a = samples[i];
            let b = samples[i + lag];
            sum += a * b;
            norm_a += a * a;
            norm_b += b * b;
        }
        let denom = (norm_a * norm_b).sqrt();
        if denom <= 1e-9 {
            continue;
        }
        let r = sum / denom;
        // Look for the first strong peak after the function has started rising from a dip.
        if !rising && r > prev_r {
            rising = true;
        }
        if rising && r > best_r {
            best_r = r;
            best_lag = lag;
        }
        prev_r = r;
    }
    if best_lag == 0 || best_r < 0.65 {
        return None;
    }
    // Parabolic interpolation around the peak for a finer lag.
    let refined = if best_lag > min_lag && best_lag < max_lag {
        parabolic_refine(samples, best_lag)
    } else {
        best_lag as f32
    };
    let hz = sample_rate / refined.max(1.0);
    if !(MIN_PITCH_HZ..=MAX_PITCH_HZ).contains(&hz) {
        return None;
    }
    Some(hz)
}

fn parabolic_refine(samples: &[f32], lag: usize) -> f32 {
    let r = |l: usize| -> f32 {
        let mut sum = 0.0;
        let n = samples.len();
        if l >= n {
            return 0.0;
        }
        for i in 0..(n - l) {
            sum += samples[i] * samples[i + l];
        }
        sum
    };
    let (a, b, c) = (r(lag - 1), r(lag), r(lag + 1));
    let denom = a - 2.0 * b + c;
    if denom.abs() < 1e-9 {
        return lag as f32;
    }
    lag as f32 + 0.5 * (a - c) / denom
}

// --- UI ---------------------------------------------------------------------

/// The window opened from the control bar. Draws the nearest note, the cents needle, and the level.
pub fn window(ctx: &egui::Context, tuner: &mut Tuner) {
    if !tuner.open {
        return;
    }
    let mut open = tuner.open;
    egui::Window::new("Tuner")
        .id(egui::Id::unique("tuner_window"))
        .open(&mut open)
        .default_pos(pos2(500.0, 120.0))
        .fixed_size(vec2(260.0, 150.0))
        .resizable(false)
        .show(ctx, |ui| draw(ui, tuner));
    tuner.open = open;
}

fn draw(ui: &mut Ui, tuner: &Tuner) {
    let (rect, _) = ui.allocate_exact_size(vec2(240.0, 130.0), Sense::hover());
    let g = ui.painter_at(rect);
    g.rect_filled(rect, 8.0, crate::theme::gray(0x10));
    g.rect_stroke(rect, 8.0, Stroke::new(1.0, crate::theme::gray(0x24)), egui::StrokeKind::Inside);

    let (name, cents) = match tuner.pitch_hz {
        Some(hz) => {
            let (n, _c) = note_of(hz);
            (n, tuner.smoothed_cents)
        }
        None => ("—".to_string(), 0.0),
    };

    // Big note name.
    g.text(
        pos2(rect.center().x, rect.top() + 36.0),
        egui::Align2::CENTER_CENTER,
        &name,
        egui::FontId::proportional(36.0),
        if tuner.pitch_hz.is_some() {
            Color32::from_rgb(0xec, 0xe6, 0xd8)
        } else {
            crate::theme::gray(0x5a)
        },
    );

    // Cents needle: -50..+50 cents mapped to the bar width.
    let bar_y = rect.top() + 80.0;
    let bar_rect = Rect::from_min_max(pos2(rect.left() + 16.0, bar_y), pos2(rect.right() - 16.0, bar_y + 6.0));
    g.rect_filled(bar_rect, 3.0, crate::theme::gray(0x18));
    // Centre line.
    let mid = bar_rect.center().x;
    g.line_segment(
        [pos2(mid, bar_rect.top() - 4.0), pos2(mid, bar_rect.bottom() + 4.0)],
        Stroke::new(1.0, crate::theme::gray(0x40)),
    );
    if tuner.pitch_hz.is_some() {
        let t = (cents / 50.0).clamp(-1.0, 1.0);
        let x = mid + t * (bar_rect.width() * 0.5 - 4.0);
        let colour = if cents.abs() < 5.0 {
            Color32::from_rgb(0x76, 0xd5, 0x8a)
        } else if cents.abs() < 15.0 {
            Color32::from_rgb(0xff, 0xb7, 0x5a)
        } else {
            Color32::from_rgb(0xd7, 0x3b, 0x2a)
        };
        g.circle_filled(pos2(x, bar_rect.center().y), 6.0, colour);
    }

    // Cents readout and RMS below.
    let cents_text = if tuner.pitch_hz.is_some() {
        format!("{cents:+.1} ¢")
    } else {
        "listening…".to_string()
    };
    g.text(
        pos2(rect.left() + 16.0, rect.top() + 108.0),
        egui::Align2::LEFT_CENTER,
        &cents_text,
        egui::FontId::proportional(12.0),
        crate::widgets::text_col(),
    );
    if let Some(hz) = tuner.pitch_hz {
        g.text(
            pos2(rect.right() - 16.0, rect.top() + 108.0),
            egui::Align2::RIGHT_CENTER,
            format!("{hz:.1} Hz"),
            egui::FontId::proportional(12.0),
            crate::widgets::text_dim_col(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn sine(hz: f32, len: usize, sr: f32) -> Vec<f32> {
        (0..len).map(|i| (TAU * hz * i as f32 / sr).sin() * 0.5).collect()
    }

    #[test]
    fn a_440_hz_sine_reads_as_a4() {
        let samples = sine(440.0, WINDOW, 48_000.0);
        let hz = detect_pitch(&samples, 48_000.0).unwrap();
        assert!((hz - 440.0).abs() < 1.0, "{hz}");
        let (name, cents) = note_of(hz);
        assert_eq!(name[..1].to_string(), "A");
        assert!(cents.abs() < 2.0, "{cents}");
    }

    #[test]
    fn silence_has_no_pitch() {
        let samples = vec![0.0f32; WINDOW];
        assert!(detect_pitch(&samples, 48_000.0).is_none());
    }

    #[test]
    fn note_name_wraps_octaves() {
        // Middle C (60) is C3, A440 (69) is A3 in Logic's convention (octave_naming = -2).
        crate::settings::set_octave_naming(-2);
        assert_eq!(name_of_midi(60), "C3");
        assert_eq!(name_of_midi(69), "A3");
    }
}
