//! Mix > Mastering: what a mix measures at the output. Loudness as ITU-R BS.1770 and EBU R128 define
//! it (momentary, short-term, integrated with its gates), sample peak, true peak by 4x oversampling,
//! a spectrum, and the same numbers for a reference file to hold the mix up against.
//!
//! The engine hands over the output through a lock-free queue while the window is open. All of the
//! measuring happens here, on the UI thread, never on the audio thread.
use crate::UiState;
use egui::{pos2, vec2, Color32, FontId, Rect, Sense, Stroke, Ui};
use rtrb::Consumer;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};

/// The queue from the engine holds this many seconds, for frames the UI takes a while to reach.
const TAP_SECONDS: usize = 1;
const FFT_SIZE: usize = 4096;
/// Below this a block does not count towards integrated loudness at all (BS.1770).
const ABSOLUTE_GATE: f64 = -70.0;
/// Then blocks more than this far below the average so far do not count.
const RELATIVE_GATE: f64 = -10.0;
const SILENCE_LUFS: f64 = -120.0;

fn to_db(power: f64) -> f64 {
    if power > 0.0 {
        10.0 * power.log10()
    } else {
        SILENCE_LUFS
    }
}

#[derive(Clone, Copy, Default)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}

impl Biquad {
    fn run(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// The two stages of BS.1770's K-weighting, worked out for any sample rate.
fn k_weighting(rate: f64) -> [Biquad; 2] {
    use std::f64::consts::PI;
    // The high shelf.
    let (g, q, f0) = (
        3.999_843_853_973_347,
        0.707_175_236_955_419_6,
        1_681.974_450_955_533,
    );
    let k = (PI * f0 / rate).tan();
    let vh = 10f64.powf(g / 20.0);
    let vb = vh.powf(0.499_666_774_154_541_6);
    let a0 = 1.0 + k / q + k * k;
    let shelf = Biquad {
        b: [
            (vh + vb * k / q + k * k) / a0,
            2.0 * (k * k - vh) / a0,
            (vh - vb * k / q + k * k) / a0,
        ],
        a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        z: [0.0; 2],
    };
    // The high pass.
    let (q, f0) = (0.500_327_037_323_877_3, 38.135_470_876_024_44);
    let k = (PI * f0 / rate).tan();
    let a0 = 1.0 + k / q + k * k;
    let pass = Biquad {
        b: [1.0, -2.0, 1.0],
        a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        z: [0.0; 2],
    };
    [shelf, pass]
}

/// Loudness of a stereo signal.
pub struct Loudness {
    filters: [[Biquad; 2]; 2],
    /// The power of each 100 ms step, weighted.
    steps: Vec<f64>,
    sum: f64,
    filled: usize,
    step_len: usize,
    /// The power of each 400 ms block, with 75 % overlap, as long as it plays.
    blocks: Vec<f64>,
}

impl Loudness {
    pub fn new(rate: u32) -> Self {
        let rate = f64::from(rate.max(1));
        let filter = k_weighting(rate);
        Self {
            filters: [filter, filter],
            steps: Vec::new(),
            sum: 0.0,
            filled: 0,
            step_len: (rate / 10.0) as usize,
            blocks: Vec::new(),
        }
    }

    fn add(&mut self, frame: [f32; 2]) {
        let mut power = 0.0;
        for (ch, sample) in frame.iter().enumerate() {
            let mut x = f64::from(*sample);
            for stage in &mut self.filters[ch] {
                x = stage.run(x);
            }
            power += x * x;
        }
        self.sum += power;
        self.filled += 1;
        if self.filled == self.step_len {
            self.steps.push(self.sum / self.step_len as f64);
            self.sum = 0.0;
            self.filled = 0;
            if self.steps.len() >= 4 {
                let last = &self.steps[self.steps.len() - 4..];
                self.blocks.push(last.iter().sum::<f64>() / 4.0);
            }
        }
    }

    fn window(&self, steps: usize) -> f64 {
        let n = self.steps.len();
        if n < steps {
            return SILENCE_LUFS;
        }
        let power = self.steps[n - steps..].iter().sum::<f64>() / steps as f64;
        -0.691 + to_db(power)
    }

    /// The last 400 ms.
    pub fn momentary(&self) -> f64 {
        self.window(4)
    }

    /// The last 3 s.
    pub fn short_term(&self) -> f64 {
        self.window(30)
    }

    /// All of it, with the two gates of BS.1770.
    pub fn integrated(&self) -> f64 {
        let lufs = |power: f64| -0.691 + to_db(power);
        let loud: Vec<f64> = self
            .blocks
            .iter()
            .copied()
            .filter(|b| lufs(*b) > ABSOLUTE_GATE)
            .collect();
        if loud.is_empty() {
            return SILENCE_LUFS;
        }
        let average = loud.iter().sum::<f64>() / loud.len() as f64;
        let gate = lufs(average) + RELATIVE_GATE;
        let kept: Vec<f64> = loud.into_iter().filter(|b| lufs(*b) > gate).collect();
        if kept.is_empty() {
            return SILENCE_LUFS;
        }
        lufs(kept.iter().sum::<f64>() / kept.len() as f64)
    }
}

/// Filter taps of the true peak oversampler, per oversampled position.
const TP_TAPS: usize = 12;
const TP_RATIO: usize = 4;

/// The highest level between the samples as well, by 4x oversampling (BS.1770 annex 2).
pub struct TruePeak {
    table: [[f32; TP_TAPS]; TP_RATIO],
    history: [[f32; TP_TAPS]; 2],
    at: usize,
    pub peak: f32,
    pub sample_peak: f32,
}

impl Default for TruePeak {
    fn default() -> Self {
        let mut table = [[0f32; TP_TAPS]; TP_RATIO];
        for (phase, row) in table.iter_mut().enumerate() {
            for (tap, value) in row.iter_mut().enumerate() {
                let x = tap as f64 - (TP_TAPS as f64 / 2.0 - 1.0) - phase as f64 / TP_RATIO as f64;
                let sinc = if x.abs() < 1e-9 {
                    1.0
                } else {
                    (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x)
                };
                let window = (std::f64::consts::PI * x / TP_TAPS as f64).cos().powi(2);
                *value = (sinc * window) as f32;
            }
        }
        Self {
            table,
            history: [[0.0; TP_TAPS]; 2],
            at: 0,
            peak: 0.0,
            sample_peak: 0.0,
        }
    }
}

impl TruePeak {
    fn add(&mut self, frame: [f32; 2]) {
        for (ch, sample) in frame.iter().enumerate() {
            self.history[ch][self.at] = *sample;
            self.sample_peak = self.sample_peak.max(sample.abs());
            for row in &self.table {
                let mut y = 0f32;
                for (tap, coefficient) in row.iter().enumerate() {
                    // The newest sample goes with the last tap.
                    let i = (self.at + 1 + tap) % TP_TAPS;
                    y += coefficient * self.history[ch][i];
                }
                self.peak = self.peak.max(y.abs());
            }
        }
        self.at = (self.at + 1) % TP_TAPS;
    }
}

fn level_db(level: f32) -> f64 {
    if level > 0.0 {
        20.0 * f64::from(level).log10()
    } else {
        SILENCE_LUFS
    }
}

/// An in-place radix-2 FFT. `re` and `im` must be a power of two long.
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = -2.0 * std::f64::consts::PI / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (s, c) = (angle * k as f64).sin_cos();
                let (c, s) = (c as f32, s as f32);
                let (a, b) = (start + k, start + k + len / 2);
                let (x, y) = (re[b] * c - im[b] * s, re[b] * s + im[b] * c);
                re[b] = re[a] - x;
                im[b] = im[a] - y;
                re[a] += x;
                im[a] += y;
            }
        }
        len <<= 1;
    }
}

/// The level in dB of each of `bands` bands from 20 Hz to the top, spaced evenly by octave.
fn spectrum(samples: &[f32], rate: u32, bands: usize) -> Vec<f32> {
    let n = samples.len();
    let mut re: Vec<f32> = samples
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let w = (std::f64::consts::PI * i as f64 / n as f64).sin().powi(2);
            s * w as f32
        })
        .collect();
    let mut im = vec![0f32; n];
    fft(&mut re, &mut im);
    let nyquist = f64::from(rate) / 2.0;
    let hz_per_bin = f64::from(rate) / n as f64;
    (0..bands)
        .map(|b| {
            let edge = |k: usize| 20.0 * (nyquist / 20.0).powf(k as f64 / bands as f64);
            let first = ((edge(b) / hz_per_bin).floor() as usize).clamp(1, n / 2 - 1);
            let last = ((edge(b + 1) / hz_per_bin).ceil() as usize).clamp(first + 1, n / 2);
            let peak = (first..last)
                .map(|i| re[i] * re[i] + im[i] * im[i])
                .fold(0f32, f32::max);
            // A Hann window halves a sine's level, and half of it is in the mirror image.
            let level = peak.sqrt() * 4.0 / n as f32;
            20.0 * level.max(1e-7).log10()
        })
        .collect()
}

/// What a whole file measures.
#[derive(Clone, Debug, PartialEq)]
pub struct Reference {
    pub name: String,
    pub integrated: f64,
    pub true_peak: f64,
}

/// Measures a file. Reads all of it: run off the UI thread.
fn measure_file(path: &std::path::Path) -> Result<Reference, String> {
    let file = prowave_engine::AudioFile::open(path).map_err(|e| e.to_string())?;
    let stereo = file.read_stereo().map_err(|e| e.to_string())?;
    let mut loudness = Loudness::new(file.sample_rate);
    let mut peak = TruePeak::default();
    for frame in stereo.as_chunks::<2>().0 {
        loudness.add(*frame);
        peak.add(*frame);
    }
    Ok(Reference {
        name: path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
        integrated: loudness.integrated(),
        true_peak: level_db(peak.peak),
    })
}

/// The window's state.
#[derive(Default)]
pub struct MasteringView {
    pub open: bool,
    /// The queue from the engine, while the window is open. `App` connects it.
    pub tap: Option<Consumer<f32>>,
    rate: u32,
    loudness: Option<Loudness>,
    peak: TruePeak,
    window: Vec<f32>,
    bands: Vec<f32>,
    reference: Option<Reference>,
    measuring: Option<Receiver<Result<Reference, String>>>,
    message: Option<String>,
}

impl MasteringView {
    /// How long the queue from the engine should be.
    pub fn tap_capacity(rate: u32) -> usize {
        rate as usize * 2 * TAP_SECONDS
    }

    /// Starts measuring again from now.
    pub fn reset(&mut self, rate: u32) {
        self.rate = rate;
        self.loudness = Some(Loudness::new(rate));
        self.peak = TruePeak::default();
        self.window.clear();
        self.bands.clear();
    }

    fn take_in(&mut self) {
        let Some(tap) = self.tap.as_mut() else {
            return;
        };
        let Some(loudness) = self.loudness.as_mut() else {
            return;
        };
        while tap.slots() >= 2 {
            let (Ok(l), Ok(r)) = (tap.pop(), tap.pop()) else {
                break;
            };
            loudness.add([l, r]);
            self.peak.add([l, r]);
            self.window.push((l + r) * 0.5);
        }
        if self.window.len() >= FFT_SIZE {
            let from = self.window.len() - FFT_SIZE;
            self.bands = spectrum(&self.window[from..], self.rate.max(1), 64);
            self.window.clear();
        }
    }
}

fn row(ui: &mut Ui, name: &str, value: f64, unit: &str) {
    ui.label(name);
    if value <= SILENCE_LUFS {
        ui.label(format!("-- {unit}"));
    } else {
        ui.label(format!("{value:.1} {unit}"));
    }
    ui.end_row();
}

fn draw_spectrum(ui: &mut Ui, bands: &[f32]) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 120.0), Sense::hover());
    let g = ui.painter_at(rect);
    g.rect_filled(rect, 2.0, Color32::from_gray(0x20));
    for db in [-20.0f32, -40.0, -60.0] {
        let y = rect.top() + rect.height() * (-db / 80.0);
        g.line_segment(
            [pos2(rect.left(), y), pos2(rect.right(), y)],
            Stroke::new(1.0, Color32::from_gray(0x38)),
        );
        g.text(
            pos2(rect.left() + 3.0, y),
            egui::Align2::LEFT_BOTTOM,
            format!("{db:.0}"),
            FontId::proportional(9.0),
            Color32::from_gray(0x80),
        );
    }
    if bands.is_empty() {
        return;
    }
    let w = rect.width() / bands.len() as f32;
    for (i, db) in bands.iter().enumerate() {
        let height = ((db + 80.0) / 80.0).clamp(0.0, 1.0) * rect.height();
        let x = rect.left() + i as f32 * w;
        g.rect_filled(
            Rect::from_min_max(
                pos2(x, rect.bottom() - height),
                pos2(x + w - 1.0, rect.bottom()),
            ),
            0.0,
            Color32::from_rgb(0x4f, 0xc3, 0xf7),
        );
    }
}

/// The window.
pub fn show(ctx: &egui::Context, s: &mut UiState) {
    if !s.mastering.open {
        return;
    }
    let v = &mut s.mastering;
    v.take_in();
    if let Some(done) = v.measuring.as_ref().and_then(|rx| rx.try_recv().ok()) {
        v.measuring = None;
        match done {
            Ok(reference) => {
                v.reference = Some(reference);
                v.message = None;
            }
            Err(e) => v.message = Some(e),
        }
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(50));
    let mut open = true;
    egui::Window::new("Mastering")
        .id(egui::Id::unique("mastering_window"))
        .open(&mut open)
        .default_width(420.0)
        .show(ctx, |ui| {
            let (m, st, integrated) = v
                .loudness
                .as_ref()
                .map_or((SILENCE_LUFS, SILENCE_LUFS, SILENCE_LUFS), |l| {
                    (l.momentary(), l.short_term(), l.integrated())
                });
            let true_peak = level_db(v.peak.peak);
            egui::Grid::new(egui::Id::unique("mastering_numbers"))
                .num_columns(2)
                .show(ui, |ui| {
                    row(ui, "Momentary", m, "LUFS");
                    row(ui, "Short-term", st, "LUFS");
                    row(ui, "Integrated", integrated, "LUFS");
                    row(ui, "Sample peak", level_db(v.peak.sample_peak), "dBFS");
                    row(ui, "True peak", true_peak, "dBTP");
                });
            if true_peak > 0.0 {
                ui.colored_label(
                    Color32::from_rgb(0xe0, 0x5a, 0x4a),
                    "Over 0 dBTP: this will clip in fixed-point files and on playback.",
                );
            }
            if ui
                .button("Reset")
                .on_hover_text("Start measuring from now")
                .clicked()
            {
                let rate = v.rate;
                v.reset(rate);
            }
            ui.add_space(6.0);
            draw_spectrum(ui, &v.bands);
            ui.add_space(6.0);
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Reference File…").clicked() {
                    let picked: Option<PathBuf> = rfd::FileDialog::new().pick_file();
                    if let Some(path) = picked {
                        let (tx, rx) = channel();
                        v.measuring = Some(rx);
                        let ctx = ui.ctx().clone();
                        let spawned = std::thread::Builder::new()
                            .name("prowave-reference".into())
                            .spawn(move || {
                                let _ = tx.send(measure_file(&path));
                                ctx.request_repaint();
                            });
                        if spawned.is_err() {
                            v.measuring = None;
                        }
                    }
                }
                if v.measuring.is_some() {
                    ui.label("Measuring…");
                }
            });
            if let Some(reference) = &v.reference {
                ui.label(egui::RichText::new(&reference.name).small());
                ui.label(format!(
                    "{:.1} LUFS, {:.1} dBTP. The mix is {:+.1} LU from it.",
                    reference.integrated,
                    reference.true_peak,
                    integrated - reference.integrated
                ));
            }
            if let Some(message) = &v.message {
                ui.label(message.as_str());
            }
        });
    if !open {
        v.open = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn sine(hz: f64, db: f64, seconds: f64, phase: f64) -> Vec<[f32; 2]> {
        let level = 10f64.powf(db / 20.0);
        (0..(seconds * f64::from(RATE)) as usize)
            .map(|i| {
                let t = i as f64 / f64::from(RATE);
                let s = (level * (std::f64::consts::TAU * hz * t + phase).sin()) as f32;
                [s, s]
            })
            .collect()
    }

    fn loudness_of(frames: &[[f32; 2]]) -> Loudness {
        let mut l = Loudness::new(RATE);
        for f in frames {
            l.add(*f);
        }
        l
    }

    /// EBU Tech 3341: a stereo 1 kHz sine at -23 dBFS measures -23 LUFS.
    #[test]
    fn a_reference_tone_measures_what_the_standard_says() {
        let l = loudness_of(&sine(1000.0, -23.0, 5.0, 0.0));
        assert!((l.integrated() + 23.0).abs() < 0.1, "{}", l.integrated());
        assert!((l.momentary() + 23.0).abs() < 0.1, "{}", l.momentary());
        assert!((l.short_term() + 23.0).abs() < 0.1, "{}", l.short_term());
    }

    /// EBU Tech 3341 case 3: the quiet parts fall below the relative gate.
    #[test]
    fn the_gates_leave_out_quiet_passages() {
        let mut signal = sine(1000.0, -36.0, 2.0, 0.0);
        signal.extend(sine(1000.0, -23.0, 12.0, 0.0));
        signal.extend(sine(1000.0, -36.0, 2.0, 0.0));
        let l = loudness_of(&signal);
        assert!((l.integrated() + 23.0).abs() < 0.2, "{}", l.integrated());
        // Silence does not count at all.
        let mut gaps = vec![[0f32; 2]; RATE as usize * 4];
        gaps.extend(sine(1000.0, -23.0, 4.0, 0.0));
        let l = loudness_of(&gaps);
        assert!((l.integrated() + 23.0).abs() < 0.2, "{}", l.integrated());
    }

    #[test]
    fn silence_reads_as_nothing() {
        let l = loudness_of(&vec![[0f32; 2]; RATE as usize * 2]);
        assert_eq!(l.integrated(), SILENCE_LUFS);
    }

    /// A sine at a quarter of the sample rate, at 45 degrees: every sample is 0.707 of the peak
    /// between them, the textbook case of a peak that sample meters miss.
    #[test]
    fn true_peak_finds_what_lies_between_the_samples() {
        let mut p = TruePeak::default();
        for f in sine(f64::from(RATE) / 4.0, 0.0, 0.1, std::f64::consts::FRAC_PI_4) {
            p.add(f);
        }
        assert!((p.sample_peak - 0.707).abs() < 0.01, "{}", p.sample_peak);
        assert!(level_db(p.peak) > -0.5, "{}", level_db(p.peak));
        assert!(level_db(p.peak) < 0.5, "{}", level_db(p.peak));
    }

    #[test]
    fn the_spectrum_puts_a_tone_where_it_is_at_its_level() {
        let tone: Vec<f32> = sine(1000.0, -12.0, 0.2, 0.0)
            .iter()
            .take(FFT_SIZE)
            .map(|f| f[0])
            .collect();
        let bands = spectrum(&tone, RATE, 64);
        let (loudest, level) =
            bands
                .iter()
                .enumerate()
                .fold((0, -200f32), |m, (i, b)| if *b > m.1 { (i, *b) } else { m });
        let edge = |k: usize| 20.0 * (24_000f64 / 20.0).powf(k as f64 / 64.0);
        assert!(
            edge(loudest) <= 1100.0 && edge(loudest + 1) >= 900.0,
            "{loudest}"
        );
        assert!((level + 12.0).abs() < 2.0, "{level}");
    }

    #[test]
    fn a_file_is_measured_as_a_whole() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join("reference.wav");
        let frames = sine(1000.0, -14.0, 2.0, 0.0);
        let flat: Vec<f32> = frames.iter().flatten().copied().collect();
        prowave_engine::write_wav(&path, &flat, RATE).unwrap_or_else(|e| panic!("{e:?}"));
        let found = measure_file(&path).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            (found.integrated + 14.0).abs() < 0.2,
            "{}",
            found.integrated
        );
        assert!((found.true_peak + 14.0).abs() < 0.3, "{}", found.true_peak);
        assert_eq!(found.name, "reference.wav");
    }
}
