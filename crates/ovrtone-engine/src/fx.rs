//! Effects for the mixer's Audio FX slots (Logic: Channel EQ, Compressor, Space Designer, Tape
//! Delay, Chorus, Phaser, Overdrive and a filter).
//!
//! An [`Fx`] is made on the UI thread (its delay lines are allocated there) and sent to the
//! audio thread inside a [`Chain`]; `process` then runs sample by sample without allocating.
//! Every effect has up to [`MAX_FX_PARAMS`] parameters in natural units (dB, Hz, ms, 0..1);
//! [`FxKind::params`] lists their names, ranges and defaults, and anything out of range or
//! not finite is clamped, so a damaged project cannot make the output blow up.
use std::f32::consts::{PI, TAU};

#[cfg(test)]
mod checks;
mod clip;
pub(crate) mod drift;
mod ember;
mod halo;
mod heat;
mod parts;
mod press;
mod prism;
mod snap;
mod spectral;
mod swirl;
mod bands;
mod freq;
mod flange;
mod grit;
mod pulse;
mod reel;
mod shift;
mod shut;
mod sweep;
mod utility;
mod wall;

/// Most parameters one effect has.
pub const MAX_FX_PARAMS: usize = 32;
/// Most effects in one channel strip: far more than a mix uses, so that memory and CPU are the
/// limit, not this. Slot numbers are a byte, and 255 means the instrument.
pub const MAX_INSERTS: usize = 128;

/// The kinds of effect, in the order the mixer's menu lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FxKind {
    ChannelEq,
    Compressor,
    Reverb,
    Delay,
    Chorus,
    Phaser,
    Overdrive,
    Filter,
    Gate,
    Limiter,
    MultiEq,
    // The Ovrtone set.
    Ember,
    Heat,
    Calm,
    Prism,
    Press,
    Wall,
    Snap,
    Clip,
    Drift,
    Halo,
    Swirl,
    // The second set.
    Bands,
    Width,
    Shut,
    Reel,
    Grit,
    Flange,
    Sweep,
    Pulse,
    Shift,
    Utility,
    Meter,
    Air,
    Freq,
}

/// One parameter of an effect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamInfo {
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub unit: &'static str,
    /// The slider moves in equal steps of the logarithm (frequencies, times).
    pub log: bool,
}

impl ParamInfo {
    /// The value at position `n` (0..=1) along the range (equal steps of the logarithm for
    /// a `log` parameter).
    pub fn from_norm(&self, n: f32) -> f32 {
        let n = if n.is_finite() {
            n.clamp(0.0, 1.0)
        } else {
            0.0
        };
        if self.log && self.min > 0.0 {
            self.min * (self.max / self.min).powf(n)
        } else {
            self.min + (self.max - self.min) * n
        }
    }

    /// The position (0..=1) of `v` along the range; the inverse of [`ParamInfo::from_norm`].
    pub fn to_norm(&self, v: f32) -> f32 {
        let v = if v.is_finite() {
            v.clamp(self.min, self.max)
        } else {
            self.default
        };
        let n = if self.log && self.min > 0.0 {
            (v / self.min).ln() / (self.max / self.min).ln()
        } else {
            (v - self.min) / (self.max - self.min)
        };
        n.clamp(0.0, 1.0)
    }
}

const fn p(name: &'static str, min: f32, max: f32, default: f32, unit: &'static str) -> ParamInfo {
    ParamInfo {
        name,
        min,
        max,
        default,
        unit,
        log: false,
    }
}

const fn pl(name: &'static str, min: f32, max: f32, default: f32, unit: &'static str) -> ParamInfo {
    ParamInfo {
        name,
        min,
        max,
        default,
        unit,
        log: true,
    }
}

const EQ_PARAMS: [ParamInfo; 5] = [
    p("Low", -18.0, 18.0, 0.0, "dB"),
    p("Mid", -18.0, 18.0, 0.0, "dB"),
    pl("Mid Freq", 100.0, 8000.0, 1000.0, "Hz"),
    pl("Mid Q", 0.3, 8.0, 1.0, ""),
    p("High", -18.0, 18.0, 0.0, "dB"),
];
const COMP_PARAMS: [ParamInfo; 5] = [
    p("Threshold", -60.0, 0.0, -18.0, "dB"),
    p("Ratio", 1.0, 20.0, 3.0, ":1"),
    pl("Attack", 0.1, 100.0, 10.0, "ms"),
    pl("Release", 10.0, 1000.0, 120.0, "ms"),
    p("Make-up", 0.0, 24.0, 0.0, "dB"),
];
const REVERB_PARAMS: [ParamInfo; 4] = [
    p("Size", 0.0, 1.0, 0.6, ""),
    p("Damping", 0.0, 1.0, 0.5, ""),
    p("Width", 0.0, 1.0, 1.0, ""),
    p("Mix", 0.0, 1.0, 0.3, ""),
];
const DELAY_PARAMS: [ParamInfo; 6] = [
    pl("Time", 1.0, 2000.0, 375.0, "ms"),
    p("Feedback", 0.0, 0.95, 0.35, ""),
    pl("Tone", 500.0, 16000.0, 6000.0, "Hz"),
    p("Ping-Pong", 0.0, 1.0, 0.0, ""),
    p("Mix", 0.0, 1.0, 0.3, ""),
    p("Sync", 0.0, 13.0, 0.0, ""),
];
const CHORUS_PARAMS: [ParamInfo; 3] = [
    pl("Rate", 0.05, 5.0, 0.6, "Hz"),
    p("Depth", 0.0, 8.0, 3.0, "ms"),
    p("Mix", 0.0, 1.0, 0.5, ""),
];
const PHASER_PARAMS: [ParamInfo; 4] = [
    pl("Rate", 0.05, 8.0, 0.4, "Hz"),
    p("Depth", 0.0, 1.0, 0.7, ""),
    p("Feedback", 0.0, 0.9, 0.4, ""),
    p("Mix", 0.0, 1.0, 0.5, ""),
];
const DRIVE_PARAMS: [ParamInfo; 4] = [
    p("Drive", 0.0, 40.0, 6.0, "dB"),
    pl("Tone", 500.0, 16000.0, 8000.0, "Hz"),
    p("Level", -24.0, 6.0, -3.0, "dB"),
    p("Mix", 0.0, 1.0, 1.0, ""),
];
const FILTER_PARAMS: [ParamInfo; 3] = [
    pl("Cutoff", 20.0, 20000.0, 1000.0, "Hz"),
    pl("Resonance", 0.5, 10.0, 0.7, ""),
    p("Mode", 0.0, 2.0, 0.0, ""),
];
const GATE_PARAMS: [ParamInfo; 5] = [
    p("Threshold", -80.0, 0.0, -50.0, "dB"),
    p("Range", -80.0, 0.0, -60.0, "dB"),
    pl("Attack", 0.1, 50.0, 1.0, "ms"),
    p("Hold", 0.0, 500.0, 50.0, "ms"),
    pl("Release", 5.0, 1000.0, 150.0, "ms"),
];
const LIMIT_PARAMS: [ParamInfo; 3] = [
    p("Gain", 0.0, 24.0, 0.0, "dB"),
    p("Ceiling", -12.0, 0.0, -0.3, "dB"),
    pl("Release", 10.0, 1000.0, 100.0, "ms"),
];
const MULTI_PARAMS: [ParamInfo; 16] = [
    pl("Low Freq", 30.0, 400.0, 80.0, "Hz"),
    p("Low Gain", -18.0, 18.0, 0.0, "dB"),
    pl("Band 2 Freq", 60.0, 1200.0, 250.0, "Hz"),
    p("Band 2 Gain", -18.0, 18.0, 0.0, "dB"),
    pl("Band 2 Q", 0.3, 8.0, 1.0, ""),
    pl("Band 3 Freq", 200.0, 4000.0, 800.0, "Hz"),
    p("Band 3 Gain", -18.0, 18.0, 0.0, "dB"),
    pl("Band 3 Q", 0.3, 8.0, 1.0, ""),
    pl("Band 4 Freq", 500.0, 8000.0, 2500.0, "Hz"),
    p("Band 4 Gain", -18.0, 18.0, 0.0, "dB"),
    pl("Band 4 Q", 0.3, 8.0, 1.0, ""),
    pl("Band 5 Freq", 1000.0, 16000.0, 6000.0, "Hz"),
    p("Band 5 Gain", -18.0, 18.0, 0.0, "dB"),
    pl("Band 5 Q", 0.3, 8.0, 1.0, ""),
    pl("High Freq", 2000.0, 18000.0, 10000.0, "Hz"),
    p("High Gain", -18.0, 18.0, 0.0, "dB"),
];

const EMBER_PARAMS: [ParamInfo; 8] = [
    p("Input", -24.0, 24.0, 0.0, "dB"),
    p("Drive", 0.0, 1.0, 0.4, ""),
    p("Model", 0.0, 5.0, 0.0, ""),
    p("Bias", -1.0, 1.0, 0.0, ""),
    pl("Tone", 400.0, 18000.0, 9000.0, "Hz"),
    pl("Low Cut", 20.0, 1000.0, 20.0, "Hz"),
    p("Mix", 0.0, 1.0, 1.0, ""),
    p("Output", -24.0, 12.0, 0.0, "dB"),
];
const HEAT_PARAMS: [ParamInfo; 8] = [
    p("Input", -12.0, 24.0, 0.0, "dB"),
    p("Amount", 0.0, 1.0, 0.5, ""),
    p("Squash", 0.0, 1.0, 0.3, ""),
    p("Spread", 0.0, 1.0, 0.0, ""),
    p("Character", 0.0, 2.0, 0.0, ""),
    p("Tone", -1.0, 1.0, 0.0, ""),
    p("Mix", 0.0, 1.0, 1.0, ""),
    p("Output", -24.0, 12.0, 0.0, "dB"),
];
const CALM_PARAMS: [ParamInfo; 10] = [
    p("Depth", 0.0, 1.0, 0.5, ""),
    p("Sharpness", 0.0, 1.0, 0.5, ""),
    p("Selectivity", 0.0, 1.0, 0.5, ""),
    p("Speed", 0.0, 1.0, 0.5, ""),
    pl("Low", 20.0, 20000.0, 200.0, "Hz"),
    pl("High", 20.0, 20000.0, 16000.0, "Hz"),
    p("Mode", 0.0, 1.0, 0.0, ""),
    p("Delta", 0.0, 1.0, 0.0, ""),
    p("Mix", 0.0, 1.0, 1.0, ""),
    p("Output", -24.0, 12.0, 0.0, "dB"),
];
macro_rules! band {
    ($n:literal, $f:literal) => {
        [
            pl(concat!("Band ", $n, " Freq"), 20.0, 20000.0, $f, "Hz"),
            p(concat!("Band ", $n, " Gain"), -24.0, 24.0, 0.0, "dB"),
            pl(concat!("Band ", $n, " Q"), 0.1, 10.0, 0.9, ""),
            p(concat!("Band ", $n, " Dynamic"), -18.0, 18.0, 0.0, "dB"),
            p(concat!("Band ", $n, " Threshold"), -60.0, 0.0, -24.0, "dB"),
        ]
    };
}
const PRISM_BANDS: [[ParamInfo; 5]; 6] = [
    band!("1", 80.0),
    band!("2", 250.0),
    band!("3", 800.0),
    band!("4", 2500.0),
    band!("5", 6000.0),
    band!("6", 12000.0),
];
const PRISM_PARAMS: [ParamInfo; 32] = {
    let mut out = [p("Tame", 0.0, 1.0, 0.0, ""); 32];
    let mut k = 0;
    while k < 30 {
        out[k] = PRISM_BANDS[k / 5][k % 5];
        k += 1;
    }
    out[31] = p("Output", -24.0, 12.0, 0.0, "dB");
    out
};
const PRESS_PARAMS: [ParamInfo; 16] = [
    p("Threshold", -60.0, 0.0, -18.0, "dB"),
    p("Ratio", 1.0, 20.0, 4.0, ":1"),
    p("Knee", 0.0, 24.0, 6.0, "dB"),
    pl("Attack", 0.05, 200.0, 10.0, "ms"),
    pl("Release", 5.0, 2000.0, 150.0, "ms"),
    p("Hold", 0.0, 500.0, 0.0, "ms"),
    p("Range", 0.0, 60.0, 40.0, "dB"),
    p("Lookahead", 0.0, 10.0, 0.0, "ms"),
    p("Make-up", 0.0, 24.0, 0.0, "dB"),
    p("Auto Gain", 0.0, 1.0, 0.0, ""),
    p("Detector", 0.0, 1.0, 0.0, ""),
    p("Link", 0.0, 1.0, 1.0, ""),
    pl("SC High Pass", 20.0, 500.0, 20.0, "Hz"),
    p("Mix", 0.0, 1.0, 1.0, ""),
    p("Output", -24.0, 12.0, 0.0, "dB"),
    p("Style", 0.0, 2.0, 0.0, ""),
];
const WALL_PARAMS: [ParamInfo; 5] = [
    p("Gain", 0.0, 24.0, 0.0, "dB"),
    p("Ceiling", -12.0, 0.0, -0.3, "dB"),
    pl("Release", 1.0, 1000.0, 60.0, "ms"),
    p("Lookahead", 0.5, 10.0, 2.0, "ms"),
    p("Style", 0.0, 2.0, 0.0, ""),
];
const SNAP_PARAMS: [ParamInfo; 6] = [
    p("Attack", -100.0, 100.0, 0.0, "%"),
    p("Sustain", -100.0, 100.0, 0.0, "%"),
    p("Speed", 0.0, 2.0, 1.0, ""),
    p("Clip", 0.0, 1.0, 0.0, ""),
    p("Mix", 0.0, 1.0, 1.0, ""),
    p("Output", -24.0, 12.0, 0.0, "dB"),
];
const CLIP_PARAMS: [ParamInfo; 5] = [
    p("Input", -12.0, 24.0, 0.0, "dB"),
    p("Ceiling", -24.0, 0.0, -0.3, "dB"),
    p("Shape", 0.0, 5.0, 1.0, ""),
    p("Oversample", 0.0, 1.0, 1.0, ""),
    p("Output", -24.0, 12.0, 0.0, "dB"),
];
const DRIFT_PARAMS: [ParamInfo; 11] = [
    pl("Time", 1.0, 2000.0, 375.0, "ms"),
    p("Feedback", 0.0, 1.0, 0.4, ""),
    p("Offset", -50.0, 50.0, 0.0, "ms"),
    p("Ping-Pong", 0.0, 1.0, 0.0, ""),
    p("Wow", 0.0, 1.0, 0.15, ""),
    pl("Low Cut", 20.0, 2000.0, 120.0, "Hz"),
    pl("High Cut", 500.0, 20000.0, 7000.0, "Hz"),
    p("Saturation", 0.0, 1.0, 0.2, ""),
    p("Duck", 0.0, 1.0, 0.0, ""),
    p("Mix", 0.0, 1.0, 0.3, ""),
    p("Sync", 0.0, 13.0, 6.0, ""),
];
const HALO_PARAMS: [ParamInfo; 10] = [
    p("Size", 0.0, 1.0, 0.5, ""),
    pl("Decay", 0.2, 30.0, 2.5, "s"),
    p("Pre-delay", 0.0, 250.0, 12.0, "ms"),
    p("Diffusion", 0.0, 1.0, 0.75, ""),
    p("Modulation", 0.0, 1.0, 0.3, ""),
    pl("Low Cut", 20.0, 1000.0, 80.0, "Hz"),
    pl("High Cut", 1000.0, 20000.0, 8000.0, "Hz"),
    p("Width", 0.0, 1.0, 1.0, ""),
    p("Early", 0.0, 1.0, 0.3, ""),
    p("Mix", 0.0, 1.0, 0.25, ""),
];
const SWIRL_PARAMS: [ParamInfo; 6] = [
    p("Mode", 0.0, 3.0, 0.0, ""),
    pl("Rate", 0.05, 10.0, 0.5, "Hz"),
    p("Depth", 0.0, 1.0, 0.6, ""),
    p("Width", 0.0, 1.0, 1.0, ""),
    pl("Tone", 1000.0, 20000.0, 9000.0, "Hz"),
    p("Mix", 0.0, 1.0, 0.5, ""),
];
const OFF_ON: [&str; 2] = ["Off", "On"];
const BANDS_PARAMS: [ParamInfo; 15] = [
    pl("Low Split", 40.0, 1000.0, 200.0, "Hz"),
    pl("High Split", 1000.0, 12000.0, 2500.0, "Hz"),
    p("Low Threshold", -60.0, 0.0, -24.0, "dB"),
    p("Low Ratio", 1.0, 20.0, 3.0, ":1"),
    p("Low Gain", -12.0, 24.0, 0.0, "dB"),
    p("Mid Threshold", -60.0, 0.0, -24.0, "dB"),
    p("Mid Ratio", 1.0, 20.0, 3.0, ":1"),
    p("Mid Gain", -12.0, 24.0, 0.0, "dB"),
    p("High Threshold", -60.0, 0.0, -24.0, "dB"),
    p("High Ratio", 1.0, 20.0, 3.0, ":1"),
    p("High Gain", -12.0, 24.0, 0.0, "dB"),
    pl("Attack", 0.1, 100.0, 5.0, "ms"),
    pl("Release", 10.0, 1000.0, 120.0, "ms"),
    p("Mix", 0.0, 1.0, 1.0, ""),
    p("Output", -24.0, 12.0, 0.0, "dB"),
];
const WIDTH_PARAMS: [ParamInfo; 4] = [
    p("Width", 0.0, 2.0, 1.0, "×"),
    pl("Mono Below", 20.0, 500.0, 20.0, "Hz"),
    p("Pan", -1.0, 1.0, 0.0, ""),
    p("Output", -24.0, 12.0, 0.0, "dB"),
];
const SHUT_PARAMS: [ParamInfo; 9] = [
    p("Threshold", -80.0, 0.0, -40.0, "dB"),
    p("Range", -80.0, 0.0, -60.0, "dB"),
    pl("Attack", 0.05, 50.0, 0.5, "ms"),
    p("Hold", 0.0, 500.0, 30.0, "ms"),
    pl("Release", 5.0, 2000.0, 150.0, "ms"),
    p("Ratio", 1.0, 20.0, 20.0, ":1"),
    pl("SC High Pass", 20.0, 2000.0, 20.0, "Hz"),
    pl("SC Low Pass", 500.0, 20000.0, 20000.0, "Hz"),
    p("Flip", 0.0, 1.0, 0.0, ""),
];
const REEL_PARAMS: [ParamInfo; 10] = [
    p("Drive", -12.0, 24.0, 3.0, "dB"),
    p("Bias", -1.0, 1.0, 0.0, ""),
    p("Wow", 0.0, 1.0, 0.2, ""),
    p("Flutter", 0.0, 1.0, 0.2, ""),
    p("Head Bump", 0.0, 6.0, 2.0, "dB"),
    pl("Top", 4000.0, 20000.0, 15000.0, "Hz"),
    p("Speed", 0.0, 1.0, 0.0, ""),
    p("Hiss", -90.0, -40.0, -90.0, "dB"),
    p("Mix", 0.0, 1.0, 1.0, ""),
    p("Output", -24.0, 12.0, 0.0, "dB"),
];
const GRIT_PARAMS: [ParamInfo; 7] = [
    pl("Rate", 500.0, 48000.0, 48000.0, "Hz"),
    p("Bits", 2.0, 24.0, 24.0, "bit"),
    p("Noise", 0.0, 1.0, 0.0, ""),
    p("Crackle", 0.0, 1.0, 0.0, ""),
    pl("Tone", 500.0, 20000.0, 20000.0, "Hz"),
    p("Mix", 0.0, 1.0, 1.0, ""),
    p("Output", -24.0, 12.0, 0.0, "dB"),
];
const FLANGE_PARAMS: [ParamInfo; 7] = [
    pl("Rate", 0.02, 10.0, 0.3, "Hz"),
    p("Depth", 0.0, 1.0, 0.7, ""),
    p("Feedback", -0.95, 0.95, 0.5, ""),
    p("Delay", 0.1, 12.0, 2.0, "ms"),
    p("Width", 0.0, 1.0, 0.5, ""),
    p("Mix", 0.0, 1.0, 0.5, ""),
    p("Through-Zero", 0.0, 1.0, 0.0, ""),
];
const SWEEP_PARAMS: [ParamInfo; 10] = [
    p("Mode", 0.0, 2.0, 0.0, ""),
    pl("Cutoff", 20.0, 20000.0, 1000.0, "Hz"),
    p("Resonance", 0.0, 1.0, 0.4, ""),
    pl("Rate", 0.02, 20.0, 1.0, "Hz"),
    p("Sync", 0.0, 13.0, 0.0, ""),
    p("LFO Depth", 0.0, 4.0, 1.0, "oct"),
    p("Envelope", -4.0, 4.0, 0.0, "oct"),
    p("Env Speed", 0.0, 1.0, 0.3, ""),
    p("Shape", 0.0, 3.0, 0.0, ""),
    p("Mix", 0.0, 1.0, 1.0, ""),
];
const PULSE_PARAMS: [ParamInfo; 6] = [
    p("Mode", 0.0, 1.0, 0.0, ""),
    pl("Rate", 0.05, 20.0, 4.0, "Hz"),
    p("Sync", 0.0, 13.0, 0.0, ""),
    p("Depth", 0.0, 1.0, 0.5, ""),
    p("Shape", 0.0, 3.0, 0.0, ""),
    p("Phase", 0.0, 180.0, 0.0, "°"),
];
const SHIFT_PARAMS: [ParamInfo; 5] = [
    p("Semitones", -12.0, 12.0, 0.0, "st"),
    p("Cents", -50.0, 50.0, 0.0, "ct"),
    p("Delay", 0.0, 50.0, 0.0, "ms"),
    p("Width", 0.0, 30.0, 0.0, "ct"),
    p("Mix", 0.0, 1.0, 1.0, ""),
];
const UTILITY_PARAMS: [ParamInfo; 8] = [
    p("Gain", -36.0, 36.0, 0.0, "dB"),
    p("Pan", -1.0, 1.0, 0.0, ""),
    p("Width", 0.0, 2.0, 1.0, "×"),
    p("Flip Left", 0.0, 1.0, 0.0, ""),
    p("Flip Right", 0.0, 1.0, 0.0, ""),
    p("Mono", 0.0, 1.0, 0.0, ""),
    p("Swap", 0.0, 1.0, 0.0, ""),
    p("DC Filter", 0.0, 1.0, 0.0, ""),
];
const FREQ_PARAMS: [ParamInfo; 5] = [
    p("Shift", -2000.0, 2000.0, 20.0, "Hz"),
    p("Fine", -10.0, 10.0, 0.0, "Hz"),
    p("Width", 0.0, 1.0, 0.0, ""),
    p("Feedback", 0.0, 0.9, 0.0, ""),
    p("Mix", 0.0, 1.0, 1.0, ""),
];
const METER_PARAMS: [ParamInfo; 1] = [p("Target", -24.0, -6.0, -14.0, "LUFS")];
const AIR_PARAMS: [ParamInfo; 4] = [
    pl("Frequency", 1000.0, 16000.0, 5000.0, "Hz"),
    p("Drive", 0.0, 1.0, 0.5, ""),
    p("Amount", 0.0, 1.0, 0.3, ""),
    p("Output", -24.0, 12.0, 0.0, "dB"),
];

/// How long the Limiter looks ahead, in seconds.
const LOOKAHEAD: f32 = 0.002;

/// The Limiter's look-ahead window in samples (its delay is one less).
fn limiter_len(sr: f32) -> usize {
    ((LOOKAHEAD * sr).round() as usize).max(2)
}

impl FxKind {
    pub const ALL: [FxKind; 35] = [
        FxKind::ChannelEq,
        FxKind::Compressor,
        FxKind::Reverb,
        FxKind::Delay,
        FxKind::Chorus,
        FxKind::Phaser,
        FxKind::Overdrive,
        FxKind::Filter,
        FxKind::Gate,
        FxKind::Limiter,
        FxKind::MultiEq,
        FxKind::Ember,
        FxKind::Heat,
        FxKind::Calm,
        FxKind::Prism,
        FxKind::Press,
        FxKind::Wall,
        FxKind::Snap,
        FxKind::Clip,
        FxKind::Drift,
        FxKind::Halo,
        FxKind::Swirl,
        FxKind::Bands,
        FxKind::Width,
        FxKind::Shut,
        FxKind::Reel,
        FxKind::Grit,
        FxKind::Flange,
        FxKind::Sweep,
        FxKind::Pulse,
        FxKind::Shift,
        FxKind::Utility,
        FxKind::Meter,
        FxKind::Air,
        FxKind::Freq,
    ];

    /// The effects the mixer's menu offers first: one of each kind.
    pub const MENU: [FxKind; 25] = [
        FxKind::Prism,
        FxKind::Press,
        FxKind::Bands,
        FxKind::Wall,
        FxKind::Shut,
        FxKind::Heat,
        FxKind::Ember,
        FxKind::Reel,
        FxKind::Grit,
        FxKind::Clip,
        FxKind::Air,
        FxKind::Snap,
        FxKind::Calm,
        FxKind::Width,
        FxKind::Halo,
        FxKind::Drift,
        FxKind::Swirl,
        FxKind::Flange,
        FxKind::Phaser,
        FxKind::Sweep,
        FxKind::Pulse,
        FxKind::Shift,
        FxKind::Freq,
        FxKind::Utility,
        FxKind::Meter,
    ];

    /// The first set of effects, kept so that projects made with them play as they did.
    pub const CLASSIC: [FxKind; 10] = [
        FxKind::Gate,
        FxKind::Filter,
        FxKind::ChannelEq,
        FxKind::MultiEq,
        FxKind::Compressor,
        FxKind::Limiter,
        FxKind::Overdrive,
        FxKind::Reverb,
        FxKind::Delay,
        FxKind::Chorus,
    ];

    /// The name the mixer shows in the slot.
    pub fn name(self) -> &'static str {
        match self {
            FxKind::ChannelEq => "Channel EQ",
            FxKind::Compressor => "Compressor",
            FxKind::Reverb => "Space D",
            FxKind::Delay => "Tape Delay",
            FxKind::Chorus => "Chorus",
            FxKind::Phaser => "Phaser",
            FxKind::Overdrive => "Overdrive",
            FxKind::Filter => "Filter",
            FxKind::Gate => "Noise Gate",
            FxKind::Limiter => "Limiter",
            FxKind::MultiEq => "Multi-Band EQ",
            FxKind::Ember => "Ember",
            FxKind::Heat => "Heat",
            FxKind::Calm => "Calm",
            FxKind::Prism => "Prism EQ",
            FxKind::Press => "Press",
            FxKind::Wall => "Wall",
            FxKind::Snap => "Snap",
            FxKind::Clip => "Clip",
            FxKind::Drift => "Drift",
            FxKind::Halo => "Halo",
            FxKind::Swirl => "Swirl",
            FxKind::Bands => "Bands",
            FxKind::Width => "Width",
            FxKind::Shut => "Shut",
            FxKind::Reel => "Reel",
            FxKind::Grit => "Grit",
            FxKind::Flange => "Flange",
            FxKind::Sweep => "Sweep",
            FxKind::Pulse => "Pulse",
            FxKind::Shift => "Shift",
            FxKind::Utility => "Utility",
            FxKind::Meter => "Meter",
            FxKind::Air => "Air",
            FxKind::Freq => "Freq",
        }
    }

    /// What the effect is, in a word, for the menu and the window.
    pub fn kind_word(self) -> &'static str {
        match self {
            FxKind::ChannelEq | FxKind::MultiEq | FxKind::Prism => "EQ",
            FxKind::Compressor | FxKind::Press => "Compressor",
            FxKind::Limiter | FxKind::Wall => "Limiter",
            FxKind::Reverb | FxKind::Halo => "Reverb",
            FxKind::Delay | FxKind::Drift => "Delay",
            FxKind::Chorus | FxKind::Swirl => "Chorus",
            FxKind::Phaser => "Modulation",
            FxKind::Overdrive | FxKind::Ember => "Distortion",
            FxKind::Filter => "Filter",
            FxKind::Gate => "Gate",
            FxKind::Heat => "Saturation",
            FxKind::Calm => "Resonance",
            FxKind::Snap => "Transient",
            FxKind::Clip => "Clipper",
            FxKind::Bands => "Multiband",
            FxKind::Width => "Imager",
            FxKind::Shut => "Gate",
            FxKind::Reel => "Tape",
            FxKind::Grit => "Lo-Fi",
            FxKind::Flange => "Flanger",
            FxKind::Sweep => "Auto Filter",
            FxKind::Pulse => "Tremolo",
            FxKind::Shift => "Pitch",
            FxKind::Utility => "Utility",
            FxKind::Meter => "Metering",
            FxKind::Air => "Exciter",
            FxKind::Freq => "Frequency Shifter",
        }
    }

    /// How many samples the effect delays the sound by (its look-ahead), at `sample_rate`.
    /// The mixer delays every other strip by the same amount.
    pub fn latency_samples(self, sample_rate: f32) -> usize {
        self.latency_for(&self.defaults(), sample_rate)
    }

    /// The delay with these settings (look-ahead and spectral effects depend on them).
    pub fn latency_for(self, params: &[f32; MAX_FX_PARAMS], sample_rate: f32) -> usize {
        let sr = sample_rate.clamp(8_000.0, 384_000.0);
        let params = self.clamp_params(*params);
        match self {
            FxKind::Limiter => limiter_len(sr) - 1,
            FxKind::Calm => spectral::LATENCY,
            FxKind::Prism if prism::tame_on(&params) => spectral::LATENCY,
            FxKind::Press => press::Press::lookahead(&params, sr),
            FxKind::Wall => wall::window(&params, sr) - 1,
            FxKind::Reel => (reel::CENTRE_MS * 0.001 * sr) as usize,
            FxKind::Flange => flange::latency(&params, sr),
            _ => 0,
        }
    }

    /// Whether it can be keyed by another strip's sound (a side-chain): the dynamics, the ducking
    /// delay and the envelope-following filter.
    pub fn takes_side(self) -> bool {
        matches!(
            self,
            FxKind::Press | FxKind::Compressor | FxKind::Gate | FxKind::Drift | FxKind::Shut | FxKind::Sweep
        )
    }

    /// The names of the choices of parameter `i`, when it is a choice or a switch.
    pub fn choices(self, i: usize) -> Option<&'static [&'static str]> {
        Some(match (self, i) {
            (FxKind::Ember, 2) => &ember::MODELS,
            (FxKind::Heat, 4) => &heat::CHARACTERS,
            (FxKind::Calm, 6) => &prism::CALM_MODES,
            (FxKind::Calm, 7) | (FxKind::Press, 9) | (FxKind::Snap, 3) | (FxKind::Clip, 3) => {
                &OFF_ON
            }
            (FxKind::Press, 10) => &["Peak", "RMS"],
            (FxKind::Press, 15) => &press::STYLES,
            (FxKind::Wall, 4) => &wall::STYLES,
            (FxKind::Snap, 2) => &snap::SPEEDS,
            (FxKind::Clip, 2) => &clip::SHAPES,
            (FxKind::Swirl, 0) => &swirl::MODES,
            (FxKind::Drift, drift::SYNC_PARAM)
            | (FxKind::Delay, 5)
            | (FxKind::Sweep, 4)
            | (FxKind::Pulse, 2) => &drift::SYNC,
            (FxKind::Reel, 6) => &reel::SPEEDS,
            (FxKind::Sweep, 0) => &sweep::MODES,
            (FxKind::Sweep, 8) | (FxKind::Pulse, 4) => &sweep::SHAPES,
            (FxKind::Pulse, 0) => &pulse::MODES,
            (FxKind::Shut, 8) | (FxKind::Flange, 6) | (FxKind::Utility, 3..=7) => &OFF_ON,
            (FxKind::Filter, 2) => &["Low Pass", "High Pass", "Band Pass"],
            _ => return None,
        })
    }

    /// The kind for a name from a project file or a Library patch (several spellings).
    pub fn from_name(name: &str) -> Option<FxKind> {
        let n = name.trim().to_ascii_lowercase();
        Some(match n.as_str() {
            "channel eq" | "eq" => FxKind::ChannelEq,
            "compressor" => FxKind::Compressor,
            "space d" | "space designer" | "reverb" => FxKind::Reverb,
            "tape delay" | "delay" | "echo" => FxKind::Delay,
            "chorus" => FxKind::Chorus,
            "phaser" => FxKind::Phaser,
            "overdrive" | "distortion" => FxKind::Overdrive,
            "filter" => FxKind::Filter,
            "noise gate" | "gate" => FxKind::Gate,
            "limiter" | "adaptive limiter" => FxKind::Limiter,
            "multi-band eq" | "multiband eq" | "multi band eq" => FxKind::MultiEq,
            "ember" => FxKind::Ember,
            "heat" => FxKind::Heat,
            "calm" => FxKind::Calm,
            "prism eq" | "prism" => FxKind::Prism,
            "press" => FxKind::Press,
            "wall" => FxKind::Wall,
            "snap" => FxKind::Snap,
            "clip" => FxKind::Clip,
            "drift" => FxKind::Drift,
            "halo" => FxKind::Halo,
            "swirl" => FxKind::Swirl,
            "bands" => FxKind::Bands,
            "width" => FxKind::Width,
            "shut" => FxKind::Shut,
            "reel" => FxKind::Reel,
            "grit" => FxKind::Grit,
            "flange" | "flanger" => FxKind::Flange,
            "sweep" | "auto filter" => FxKind::Sweep,
            "pulse" | "tremolo" => FxKind::Pulse,
            "shift" | "pitch shifter" => FxKind::Shift,
            "utility" => FxKind::Utility,
            "meter" => FxKind::Meter,
            "air" | "exciter" => FxKind::Air,
            "freq" | "frequency shifter" => FxKind::Freq,
            _ => return None,
        })
    }

    /// Names, ranges and defaults of the parameters.
    pub fn params(self) -> &'static [ParamInfo] {
        match self {
            FxKind::ChannelEq => &EQ_PARAMS,
            FxKind::Compressor => &COMP_PARAMS,
            FxKind::Reverb => &REVERB_PARAMS,
            FxKind::Delay => &DELAY_PARAMS,
            FxKind::Chorus => &CHORUS_PARAMS,
            FxKind::Phaser => &PHASER_PARAMS,
            FxKind::Overdrive => &DRIVE_PARAMS,
            FxKind::Filter => &FILTER_PARAMS,
            FxKind::Gate => &GATE_PARAMS,
            FxKind::Limiter => &LIMIT_PARAMS,
            FxKind::MultiEq => &MULTI_PARAMS,
            FxKind::Ember => &EMBER_PARAMS,
            FxKind::Heat => &HEAT_PARAMS,
            FxKind::Calm => &CALM_PARAMS,
            FxKind::Prism => &PRISM_PARAMS,
            FxKind::Press => &PRESS_PARAMS,
            FxKind::Wall => &WALL_PARAMS,
            FxKind::Snap => &SNAP_PARAMS,
            FxKind::Clip => &CLIP_PARAMS,
            FxKind::Drift => &DRIFT_PARAMS,
            FxKind::Halo => &HALO_PARAMS,
            FxKind::Swirl => &SWIRL_PARAMS,
            FxKind::Bands => &BANDS_PARAMS,
            FxKind::Width => &WIDTH_PARAMS,
            FxKind::Shut => &SHUT_PARAMS,
            FxKind::Reel => &REEL_PARAMS,
            FxKind::Grit => &GRIT_PARAMS,
            FxKind::Flange => &FLANGE_PARAMS,
            FxKind::Sweep => &SWEEP_PARAMS,
            FxKind::Pulse => &PULSE_PARAMS,
            FxKind::Shift => &SHIFT_PARAMS,
            FxKind::Utility => &UTILITY_PARAMS,
            FxKind::Meter => &METER_PARAMS,
            FxKind::Air => &AIR_PARAMS,
            FxKind::Freq => &FREQ_PARAMS,
        }
    }

    /// The parameters of a new effect of this kind.
    pub fn defaults(self) -> [f32; MAX_FX_PARAMS] {
        let mut v = [0.0; MAX_FX_PARAMS];
        for (slot, info) in v.iter_mut().zip(self.params()) {
            *slot = info.default;
        }
        v
    }

    /// `values` with each parameter put into its range (not finite: its default); the unused
    /// ones are zero.
    pub fn clamp_params(self, values: [f32; MAX_FX_PARAMS]) -> [f32; MAX_FX_PARAMS] {
        let mut v = [0.0; MAX_FX_PARAMS];
        for ((slot, info), x) in v.iter_mut().zip(self.params()).zip(values) {
            *slot = if x.is_finite() {
                x.clamp(info.min, info.max)
            } else {
                info.default
            };
        }
        v
    }
}

/// One biquad filter (RBJ cookbook), per channel state in transposed direct form II.
#[derive(Clone, Copy, Debug, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

#[derive(Clone, Copy)]
enum Shape {
    LowShelf,
    HighShelf,
    Peak,
    Low,
    High,
    Band,
}

impl Biquad {
    fn unity() -> Self {
        Self {
            b0: 1.0,
            ..Self::default()
        }
    }

    fn make(shape: Shape, freq: f32, q: f32, gain_db: f32, sr: f32) -> Self {
        let f = freq.clamp(10.0, sr * 0.45);
        let w = TAU * f / sr;
        let (sin, cos) = (w.sin(), w.cos());
        let q = q.max(0.1);
        let alpha = sin / (2.0 * q);
        let a = 10f32.powf(gain_db / 40.0);
        let (b0, b1, b2, a0, a1, a2) = match shape {
            Shape::Peak => (
                1.0 + alpha * a,
                -2.0 * cos,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cos,
                1.0 - alpha / a,
            ),
            Shape::LowShelf => {
                let s = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * cos + s),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                    a * ((a + 1.0) - (a - 1.0) * cos - s),
                    (a + 1.0) + (a - 1.0) * cos + s,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                    (a + 1.0) + (a - 1.0) * cos - s,
                )
            }
            Shape::HighShelf => {
                let s = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * cos + s),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                    a * ((a + 1.0) + (a - 1.0) * cos - s),
                    (a + 1.0) - (a - 1.0) * cos + s,
                    2.0 * ((a - 1.0) - (a + 1.0) * cos),
                    (a + 1.0) - (a - 1.0) * cos - s,
                )
            }
            Shape::Low => (
                (1.0 - cos) * 0.5,
                1.0 - cos,
                (1.0 - cos) * 0.5,
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            ),
            Shape::High => (
                (1.0 + cos) * 0.5,
                -(1.0 + cos),
                (1.0 + cos) * 0.5,
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            ),
            Shape::Band => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
        };
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            ..Self::default()
        }
    }

    /// The same filter with its memory cleared.
    fn cleared(self) -> Self {
        Self {
            z1: 0.0,
            z2: 0.0,
            ..self
        }
    }

    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    /// Gain in dB at `freq`.
    fn response_db(&self, freq: f32, sr: f32) -> f32 {
        let w = TAU * freq.clamp(1.0, sr * 0.5) / sr;
        let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
        let nr = self.b0 + self.b1 * c1 + self.b2 * c2;
        let ni = -(self.b1 * s1 + self.b2 * s2);
        let dr = 1.0 + self.a1 * c1 + self.a2 * c2;
        let di = -(self.a1 * s1 + self.a2 * s2);
        let num = nr * nr + ni * ni;
        let den = (dr * dr + di * di).max(1e-20);
        10.0 * (num / den).max(1e-20).log10()
    }
}

/// The Channel EQ's three filters for `params`.
fn eq_filters(params: &[f32; MAX_FX_PARAMS], sr: f32) -> [Biquad; 3] {
    [
        Biquad::make(Shape::LowShelf, 120.0, 0.7, params[0], sr),
        Biquad::make(Shape::Peak, params[2], params[3], params[1], sr),
        Biquad::make(Shape::HighShelf, 8000.0, 0.7, params[4], sr),
    ]
}

/// The Multi-Band EQ's six filters: a low shelf, four bells and a high shelf.
fn multi_filters(pr: &[f32; MAX_FX_PARAMS], sr: f32) -> [Biquad; 6] {
    [
        Biquad::make(Shape::LowShelf, pr[0], 0.7, pr[1], sr),
        Biquad::make(Shape::Peak, pr[2], pr[4], pr[3], sr),
        Biquad::make(Shape::Peak, pr[5], pr[7], pr[6], sr),
        Biquad::make(Shape::Peak, pr[8], pr[10], pr[9], sr),
        Biquad::make(Shape::Peak, pr[11], pr[13], pr[12], sr),
        Biquad::make(Shape::HighShelf, pr[14], 0.7, pr[15], sr),
    ]
}

/// The gain in dB of an EQ (`ChannelEq` or `MultiEq`; flat for any other kind) at each of
/// `freqs`, for drawing its curve.
pub fn response_db(kind: FxKind, params: [f32; MAX_FX_PARAMS], freqs: &[f32], sr: f32) -> Vec<f32> {
    let params = kind.clamp_params(params);
    let filters: Vec<Biquad> = match kind {
        FxKind::ChannelEq => eq_filters(&params, sr).to_vec(),
        FxKind::MultiEq => multi_filters(&params, sr).to_vec(),
        FxKind::Prism => prism::static_filters(&params, sr).to_vec(),
        _ => Vec::new(),
    };
    freqs
        .iter()
        .map(|f| filters.iter().map(|b| b.response_db(*f, sr)).sum())
        .collect()
}

/// The time in ms Drift repeats at with these settings and tempo.
pub fn drift_time_ms(params: [f32; MAX_FX_PARAMS], bpm: f32) -> f32 {
    drift::time_ms(&FxKind::Drift.clamp_params(params), bpm)
}

/// A delay time in ms, or the note value of Sync choice `sync` at `bpm` when it is not Off.
pub fn synced_ms(time: f32, sync: f32, bpm: f32) -> f32 {
    drift::synced(time, sync, bpm)
}

/// Prism's gain in dB at each of `freqs` with every band at the far end of its dynamic range:
/// the most the curve can move to.
pub fn prism_reach_db(params: [f32; MAX_FX_PARAMS], freqs: &[f32], sr: f32) -> Vec<f32> {
    let mut p = FxKind::Prism.clamp_params(params);
    for k in 0..prism::BANDS {
        p[k * prism::PER_BAND + 1] = prism::band_reach(&p, k);
    }
    response_db(FxKind::Prism, p, freqs, sr)
}

/// The curve a dynamics or shaping effect draws: for a compressor or limiter the output
/// level in dB for an input level in dB; for a shaper the output (-1..1) for an input (-1..1).
pub fn curve_point(kind: FxKind, params: [f32; MAX_FX_PARAMS], x: f32) -> f32 {
    let p = kind.clamp_params(params);
    match kind {
        FxKind::Press => press::transfer(&p, x),
        FxKind::Wall => (x + p[0]).min(p[1]),
        FxKind::Shut => x + shut::curve(&p, x),
        FxKind::Clip => {
            let c = parts::gain(p[1]);
            let g = parts::gain(p[0]);
            clip::shape(parts::choice(p[2], clip::SHAPES.len()), x * g / c) * c
        }
        FxKind::Ember => ember::curve(parts::choice(p[2], ember::MODELS.len()), p[1], p[3], x),
        FxKind::Heat => {
            let drive = parts::gain(p[1] * 24.0 + p[0]);
            let ch = parts::choice(p[4], heat::CHARACTERS.len());
            heat::shape(ch, x * drive) / heat::shape(ch, drive).abs().max(1e-3)
        }
        _ => x,
    }
}

/// The Channel EQ's gain in dB at each of `freqs` (for drawing its curve).
pub fn eq_response_db(params: [f32; MAX_FX_PARAMS], freqs: &[f32], sr: f32) -> Vec<f32> {
    response_db(FxKind::ChannelEq, params, freqs, sr)
}

/// A delay line with linear interpolation. The buffer is a power of two long.
#[derive(Clone, Debug)]
struct Line {
    buf: Vec<f32>,
    pos: usize,
    mask: usize,
}

impl Line {
    fn new(max_samples: usize) -> Self {
        let len = (max_samples + 2).next_power_of_two().max(4);
        Self {
            buf: vec![0.0; len],
            pos: 0,
            mask: len - 1,
        }
    }

    #[inline]
    fn push(&mut self, x: f32) {
        self.pos = (self.pos + 1) & self.mask;
        self.buf[self.pos] = x;
    }

    /// The sample written `delay` samples ago (a fraction is interpolated).
    #[inline]
    fn read(&self, delay: f32) -> f32 {
        let d = delay.clamp(1.0, (self.mask - 1) as f32);
        let whole = d as usize;
        let frac = d - whole as f32;
        let a = self.buf[self.pos.wrapping_sub(whole) & self.mask];
        let b = self.buf[self.pos.wrapping_sub(whole + 1) & self.mask];
        a + (b - a) * frac
    }
}

/// Freeverb's comb and allpass delays, in samples at 44.1 kHz.
const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASSES: [usize; 4] = [556, 441, 341, 225];
const STEREO_SPREAD: usize = 23;

#[derive(Clone, Debug)]
struct Comb {
    line: Vec<f32>,
    pos: usize,
    store: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Self {
            line: vec![0.0; len.max(2)],
            pos: 0,
            store: 0.0,
        }
    }

    #[inline]
    fn tick(&mut self, x: f32, feedback: f32, damp: f32) -> f32 {
        let out = self.line[self.pos];
        self.store = out * (1.0 - damp) + self.store * damp;
        self.line[self.pos] = x + self.store * feedback;
        self.pos += 1;
        if self.pos == self.line.len() {
            self.pos = 0;
        }
        out
    }
}

#[derive(Clone, Debug)]
struct Allpass {
    line: Vec<f32>,
    pos: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Self {
            line: vec![0.0; len.max(2)],
            pos: 0,
        }
    }

    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let buffered = self.line[self.pos];
        let out = buffered - x;
        self.line[self.pos] = x + buffered * 0.5;
        self.pos += 1;
        if self.pos == self.line.len() {
            self.pos = 0;
        }
        out
    }
}

#[derive(Clone, Debug)]
struct ReverbState {
    combs: [Vec<Comb>; 2],
    allpasses: [Vec<Allpass>; 2],
}

impl ReverbState {
    fn new(sr: f32) -> Self {
        let scale = sr / 44_100.0;
        let side = |extra: usize| {
            (
                COMBS
                    .iter()
                    .map(|n| Comb::new(((*n + extra) as f32 * scale) as usize))
                    .collect::<Vec<_>>(),
                ALLPASSES
                    .iter()
                    .map(|n| Allpass::new(((*n + extra) as f32 * scale) as usize))
                    .collect::<Vec<_>>(),
            )
        };
        let (cl, al) = side(0);
        let (cr, ar) = side(STEREO_SPREAD);
        Self {
            combs: [cl, cr],
            allpasses: [al, ar],
        }
    }
}

#[derive(Clone, Debug)]
struct CompState {
    /// The smoothed gain reduction, in dB (0 or more).
    reduction: f32,
}

#[derive(Clone, Debug)]
struct DelayState {
    lines: [Line; 2],
    tone: [f32; 2],
}

#[derive(Clone, Debug)]
struct ChorusState {
    lines: [Line; 2],
    phase: f32,
}

#[derive(Clone, Debug)]
struct PhaserState {
    // First-order allpass memory per stage and channel, and the last output for feedback.
    x1: [[f32; 4]; 2],
    y1: [[f32; 4]; 2],
    last: [f32; 2],
    phase: f32,
}

#[derive(Clone, Debug)]
struct DriveState {
    tone: [f32; 2],
    // One-pole DC blocker per channel.
    dc_in: [f32; 2],
    dc_out: [f32; 2],
}

#[derive(Clone, Debug)]
struct GateState {
    /// Level follower, linear.
    env: f32,
    open: bool,
    hold_left: u32,
    gain: f32,
}

#[derive(Clone, Debug)]
struct LimiterState {
    /// The sound delayed by the look-ahead, per channel.
    delay: [Vec<f32>; 2],
    /// The gain each of the last samples needed, and the same after the sliding minimum
    /// and the moving average (which make the gain ramp down before the peak arrives).
    needs: Vec<f32>,
    avg: Vec<f32>,
    avg_sum: f32,
    pos: usize,
    /// The gain after the release.
    gain: f32,
}

impl LimiterState {
    fn new(sr: f32) -> Self {
        let n = limiter_len(sr);
        Self {
            delay: [vec![0.0; n], vec![0.0; n]],
            needs: vec![1.0; n],
            avg: vec![1.0; n],
            avg_sum: n as f32,
            pos: 0,
            gain: 1.0,
        }
    }
}

#[derive(Clone, Debug)]
enum State {
    MultiEq([[Biquad; 6]; 2]),
    Gate(GateState),
    Limiter(Box<LimiterState>),
    Eq([[Biquad; 3]; 2]),
    Comp(CompState),
    Reverb(Box<ReverbState>),
    Delay(Box<DelayState>),
    Chorus(Box<ChorusState>),
    Phaser(Box<PhaserState>),
    Drive(DriveState),
    Filter([Biquad; 2]),
    Ember(Box<ember::Ember>),
    Heat(Box<heat::Heat>),
    Calm(Box<prism::Calm>),
    Prism(Box<prism::Prism>),
    Press(Box<press::Press>),
    Wall(Box<wall::Wall>),
    Snap(Box<snap::Snap>),
    Clip(Box<clip::Clip>),
    Drift(Box<drift::Drift>),
    Halo(Box<halo::Halo>),
    Swirl(Box<swirl::Swirl>),
    Bands(Box<bands::Bands>),
    Width(Box<utility::Width>),
    Shut(Box<shut::Shut>),
    Reel(Box<reel::Reel>),
    Grit(Box<grit::Grit>),
    Flange(Box<flange::Flange>),
    Sweep(Box<sweep::Sweep>),
    Pulse(Box<pulse::Pulse>),
    Shift(Box<shift::Shift>),
    Utility(Box<utility::Utility>),
    Meter,
    Air(Box<utility::Air>),
    Freq(Box<freq::Freq>),
}

/// One effect with its state. Make it with [`Fx::new`] off the audio thread.
#[derive(Clone, Debug)]
pub struct Fx {
    kind: FxKind,
    sr: f32,
    /// What is played now, and what the user set (automation moves the first and
    /// [`Fx::restore`] goes back to the second).
    params: [f32; MAX_FX_PARAMS],
    base: [f32; MAX_FX_PARAMS],
    bypass: bool,
    state: State,
    /// The project tempo, for effects synced to it.
    bpm: f32,
    /// The strip whose sound keys it (a side-chain), and that sound now.
    side: Option<u16>,
    key: (f32, f32),
}

impl Fx {
    /// The same effect keyed by strip `source` (`None`: by its own sound). Off the audio thread.
    pub fn with_side(mut self, source: Option<u16>) -> Self {
        self.side = source.filter(|_| self.kind.takes_side());
        self
    }

    /// The strip that keys it, if any.
    pub fn side_source(&self) -> Option<u16> {
        self.side
    }

    /// A whole span at once, for the effects that have a block form (no key, no tap). False:
    /// run it frame by frame.
    fn run_block(&mut self, l: &mut [f64], r: &mut [f64]) -> bool {
        if self.bypass {
            return true;
        }
        match &mut self.state {
            State::Utility(s) => {
                s.run_block(l, r, &self.params);
                true
            }
            State::Meter => true,
            _ => false,
        }
    }

    /// The keying strip's sound for the sample about to be processed.
    #[inline]
    pub fn feed_side(&mut self, l: f32, r: f32) {
        self.key = (l, r);
    }

    pub fn new(kind: FxKind, params: [f32; MAX_FX_PARAMS], bypass: bool, sample_rate: f32) -> Self {
        let sr = if sample_rate.is_finite() {
            sample_rate.clamp(8_000.0, 384_000.0)
        } else {
            48_000.0
        };
        let state = match kind {
            FxKind::ChannelEq => State::Eq([[Biquad::unity(); 3]; 2]),
            FxKind::Compressor => State::Comp(CompState { reduction: 0.0 }),
            FxKind::Reverb => State::Reverb(Box::new(ReverbState::new(sr))),
            FxKind::Delay => State::Delay(Box::new(DelayState {
                lines: [
                    Line::new((2.05 * sr) as usize),
                    Line::new((2.05 * sr) as usize),
                ],
                tone: [0.0; 2],
            })),
            FxKind::Chorus => State::Chorus(Box::new(ChorusState {
                lines: [
                    Line::new((0.05 * sr) as usize),
                    Line::new((0.05 * sr) as usize),
                ],
                phase: 0.0,
            })),
            FxKind::Phaser => State::Phaser(Box::new(PhaserState {
                x1: [[0.0; 4]; 2],
                y1: [[0.0; 4]; 2],
                last: [0.0; 2],
                phase: 0.0,
            })),
            FxKind::Overdrive => State::Drive(DriveState {
                tone: [0.0; 2],
                dc_in: [0.0; 2],
                dc_out: [0.0; 2],
            }),
            FxKind::Filter => State::Filter([Biquad::unity(); 2]),
            FxKind::Gate => State::Gate(GateState {
                env: 0.0,
                open: false,
                hold_left: 0,
                gain: 0.0,
            }),
            FxKind::Limiter => State::Limiter(Box::new(LimiterState::new(sr))),
            FxKind::MultiEq => State::MultiEq([[Biquad::unity(); 6]; 2]),
            FxKind::Ember => State::Ember(Box::new(ember::Ember::new(sr))),
            FxKind::Heat => State::Heat(Box::new(heat::Heat::new(sr))),
            FxKind::Calm => State::Calm(Box::new(prism::Calm::new(sr))),
            FxKind::Prism => State::Prism(Box::new(prism::Prism::new(sr))),
            FxKind::Press => State::Press(Box::new(press::Press::new(sr))),
            FxKind::Wall => State::Wall(Box::new(wall::Wall::new(sr))),
            FxKind::Snap => State::Snap(Box::new(snap::Snap::new(sr))),
            FxKind::Clip => State::Clip(Box::new(clip::Clip::new(sr))),
            FxKind::Drift => State::Drift(Box::new(drift::Drift::new(sr))),
            FxKind::Halo => State::Halo(Box::new(halo::Halo::new(sr))),
            FxKind::Swirl => State::Swirl(Box::new(swirl::Swirl::new(sr))),
            FxKind::Bands => State::Bands(Box::new(bands::Bands::new(sr))),
            FxKind::Width => State::Width(Box::new(utility::Width::new(sr))),
            FxKind::Shut => State::Shut(Box::new(shut::Shut::new(sr))),
            FxKind::Reel => State::Reel(Box::new(reel::Reel::new(sr))),
            FxKind::Grit => State::Grit(Box::new(grit::Grit::new(sr))),
            FxKind::Flange => State::Flange(Box::new(flange::Flange::new(sr))),
            FxKind::Sweep => State::Sweep(Box::new(sweep::Sweep::new(sr))),
            FxKind::Pulse => State::Pulse(Box::new(pulse::Pulse::new(sr))),
            FxKind::Shift => State::Shift(Box::new(shift::Shift::new(sr))),
            FxKind::Utility => State::Utility(Box::default()),
            FxKind::Meter => State::Meter,
            FxKind::Air => State::Air(Box::new(utility::Air::new(sr))),
            FxKind::Freq => State::Freq(Box::new(freq::Freq::new(sr))),
        };
        let mut fx = Self {
            kind,
            sr,
            params: kind.defaults(),
            base: kind.defaults(),
            bypass,
            state,
            bpm: 120.0,
            side: None,
            key: (0.0, 0.0),
        };
        fx.set(params, bypass);
        fx
    }

    pub fn kind(&self) -> FxKind {
        self.kind
    }

    /// New parameter values (put into range here) and bypass. Cheap and allocation free, so
    /// it can run on the audio thread while the effect plays.
    pub fn set(&mut self, params: [f32; MAX_FX_PARAMS], bypass: bool) {
        self.base = self.kind.clamp_params(params);
        self.params = self.base;
        self.bypass = bypass;
        self.apply();
    }

    /// Moves parameter `i` to `value` (put into range) without changing what the user set:
    /// this is what automation does. Out of range indexes do nothing.
    pub fn set_param(&mut self, i: usize, value: f32) {
        if let Some(info) = self.kind.params().get(i) {
            if value.is_finite() {
                self.params[i] = value.clamp(info.min, info.max);
                self.apply();
            }
        }
    }

    /// Goes back to the parameters the user set (automation stopped).
    pub fn restore(&mut self) {
        if self.params != self.base {
            self.params = self.base;
            self.apply();
        }
    }

    /// The parameter values being played.
    pub fn current(&self) -> [f32; MAX_FX_PARAMS] {
        self.params
    }

    /// The delay this effect puts on the sound now, in samples (none while bypassed).
    pub fn latency_samples(&self) -> usize {
        if self.bypass {
            0
        } else {
            self.kind.latency_for(&self.base, self.sr)
        }
    }

    /// Recomputes the filters from `params`, keeping their memory so a moving knob does not
    /// click.
    fn apply(&mut self) {
        let pr = self.params;
        let base = self.base;
        let sr = self.sr;
        match &mut self.state {
            State::Ember(s) => s.set(&pr),
            State::Heat(s) => s.set(&pr),
            State::Calm(s) => s.set(&pr),
            State::Prism(s) => s.set(&pr, &base),
            State::Press(s) => s.set(&pr),
            State::Wall(s) => s.set(&base),
            State::Snap(s) => s.set(&pr),
            State::Drift(s) => s.set(&pr),
            State::Halo(s) => s.set(&pr),
            State::Bands(s) => s.set(&pr),
            State::Width(s) => s.set(&pr),
            State::Shut(s) => s.set(&pr),
            State::Reel(s) => s.set(&pr),
            State::Grit(s) => s.set(&pr),
            State::Sweep(s) => s.set(&pr),
            State::Air(s) => s.set(&pr),
            State::Eq(f) => {
                let new = eq_filters(&pr, sr);
                for ch in f.iter_mut() {
                    for (old, n) in ch.iter_mut().zip(new) {
                        // Keep the memory so changing a knob does not click.
                        *old = Biquad {
                            z1: old.z1,
                            z2: old.z2,
                            ..n
                        };
                    }
                }
            }
            State::MultiEq(f) => {
                let new = multi_filters(&pr, sr);
                for ch in f.iter_mut() {
                    for (old, n) in ch.iter_mut().zip(new) {
                        *old = Biquad {
                            z1: old.z1,
                            z2: old.z2,
                            ..n
                        };
                    }
                }
            }
            State::Filter(f) => {
                let shape = match pr[2].round() as i32 {
                    1 => Shape::High,
                    2 => Shape::Band,
                    _ => Shape::Low,
                };
                let n = Biquad::make(shape, pr[0], pr[1], 0.0, sr);
                for old in f.iter_mut() {
                    *old = Biquad {
                        z1: old.z1,
                        z2: old.z2,
                        ..n
                    };
                }
            }
            _ => {}
        }
    }

    /// The project tempo now: effects that follow it (a synced delay) use it.
    #[inline]
    pub fn set_tempo(&mut self, bpm: f32) {
        self.bpm = bpm;
        match &mut self.state {
            State::Drift(d) => d.bpm = bpm,
            State::Sweep(s) => s.bpm = bpm,
            State::Pulse(s) => s.bpm = bpm,
            _ => {}
        }
    }

    pub fn bypassed(&self) -> bool {
        self.bypass
    }

    /// Gain reduction in dB (0 or more) of a compressor, gate or limiter, else 0.
    pub fn reduction_db(&self) -> f32 {
        let db = |g: f32| (-20.0 * g.max(1e-6).log10()).max(0.0);
        match &self.state {
            State::Comp(c) if !self.bypass => c.reduction,
            State::Gate(g) if !self.bypass => db(g.gain),
            State::Limiter(l) if !self.bypass => db(l.gain),
            State::Wall(w) if !self.bypass => db(w.gain),
            State::Press(c) if !self.bypass => c.reduction_db(),
            State::Heat(c) if !self.bypass => c.reduction,
            State::Calm(c) if !self.bypass => c.spectral.most,
            State::Prism(c) if !self.bypass => c.reduction_db(),
            State::Clip(c) if !self.bypass => c.over_db,
            State::Bands(c) if !self.bypass => c.reduction_db(),
            State::Shut(c) if !self.bypass => c.red,
            _ => 0.0,
        }
    }

    /// One stereo sample through the effect.
    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        if self.bypass {
            return (l, r);
        }
        let pr = self.params;
        let sr = self.sr;
        // What the detectors listen to: the keying strip, or the effect's own input.
        let keyed = self.side.map(|_| self.key);
        let (kl, kr) = keyed.unwrap_or((l, r));
        let (ol, or) = match &mut self.state {
            State::Ember(s) => s.tick(l, r, &pr),
            State::Heat(s) => s.tick(l, r, &pr),
            State::Calm(s) => s.tick(l, r, &pr),
            State::Prism(s) => s.tick(l, r, &pr),
            State::Press(s) => {
                let look = press::Press::lookahead(&self.base, sr);
                s.key = keyed;
                s.tick(l, r, &pr, look)
            }
            State::Wall(s) => s.tick(l, r, &pr),
            State::Snap(s) => s.tick(l, r, &pr),
            State::Clip(s) => s.tick(l, r, &pr),
            State::Drift(s) => {
                s.key = keyed;
                s.tick(l, r, &pr)
            }
            State::Halo(s) => s.tick(l, r, &pr),
            State::Swirl(s) => s.tick(l, r, &pr),
            State::Bands(s) => s.tick(l, r, &pr),
            State::Width(s) => s.tick(l, r, &pr),
            State::Shut(s) => s.tick(l, r, (kl, kr), &pr),
            State::Reel(s) => s.tick(l, r, &pr),
            State::Grit(s) => s.tick(l, r, &pr),
            State::Flange(s) => s.tick(l, r, &pr, &self.base),
            State::Sweep(s) => s.tick(l, r, (kl, kr), &pr),
            State::Pulse(s) => s.tick(l, r, &pr),
            State::Shift(s) => s.tick(l, r, &pr),
            State::Utility(s) => s.tick(l, r, &pr),
            State::Meter => (l, r),
            State::Air(s) => s.tick(l, r, &pr),
            State::Freq(s) => s.tick(l, r, &pr),
            State::Eq(f) => {
                let [left, right] = f;
                let a = left.iter_mut().fold(l, |x, filter| filter.tick(x));
                let b = right.iter_mut().fold(r, |x, filter| filter.tick(x));
                (a, b)
            }
            State::Comp(c) => {
                let level = kl.abs().max(kr.abs()).max(1e-6);
                let level_db = 20.0 * level.log10();
                let over = level_db - pr[0];
                let knee = 6.0;
                let slope = 1.0 - 1.0 / pr[1];
                let target = if over <= -knee / 2.0 {
                    0.0
                } else if over >= knee / 2.0 {
                    over * slope
                } else {
                    let x = over + knee / 2.0;
                    slope * x * x / (2.0 * knee)
                };
                // Attack when the reduction grows, release when it falls.
                let ms = if target > c.reduction { pr[2] } else { pr[3] };
                let k = 1.0 - (-1.0 / (ms * 0.001 * sr)).exp();
                c.reduction += (target - c.reduction) * k;
                let g = 10f32.powf((pr[4] - c.reduction) / 20.0);
                (l * g, r * g)
            }
            State::Reverb(s) => {
                let feedback = 0.7 + pr[0] * 0.28;
                let damp = pr[1] * 0.4;
                let input = (l + r) * 0.015;
                let mut out = [0f32; 2];
                for (ch, o) in out.iter_mut().enumerate() {
                    let mut acc = 0.0;
                    for c in s.combs[ch].iter_mut() {
                        acc += c.tick(input, feedback, damp);
                    }
                    for a in s.allpasses[ch].iter_mut() {
                        acc = a.tick(acc);
                    }
                    *o = acc;
                }
                let wet_l = out[0] * (0.5 + 0.5 * pr[2]) + out[1] * (0.5 - 0.5 * pr[2]);
                let wet_r = out[1] * (0.5 + 0.5 * pr[2]) + out[0] * (0.5 - 0.5 * pr[2]);
                let wet = pr[3] * 3.0;
                (
                    l * (1.0 - pr[3]) + wet_l * wet,
                    r * (1.0 - pr[3]) + wet_r * wet,
                )
            }
            State::Delay(s) => {
                let ms = drift::synced(pr[0], pr[5], self.bpm);
                let d = (ms * 0.001 * sr).max(1.0);
                let tone = 1.0 - (-TAU * pr[2] / sr).exp();
                let (dl, dr) = (s.lines[0].read(d), s.lines[1].read(d));
                // Ping-pong sends each repeat to the other side.
                let (fl, fr) = (
                    dl * (1.0 - pr[3]) + dr * pr[3],
                    dr * (1.0 - pr[3]) + dl * pr[3],
                );
                s.tone[0] += (fl - s.tone[0]) * tone;
                s.tone[1] += (fr - s.tone[1]) * tone;
                s.lines[0].push(l + s.tone[0] * pr[1]);
                s.lines[1].push(r + s.tone[1] * pr[1]);
                (l + dl * pr[4], r + dr * pr[4])
            }
            State::Chorus(s) => {
                s.phase += pr[0] / sr;
                if s.phase >= 1.0 {
                    s.phase -= 1.0;
                }
                let base = 0.012 * sr;
                let depth = pr[1] * 0.001 * sr;
                let mod_l = (s.phase * TAU).sin();
                let mod_r = ((s.phase + 0.25) * TAU).sin();
                s.lines[0].push(l);
                s.lines[1].push(r);
                let wl = s.lines[0].read(base + depth * mod_l);
                let wr = s.lines[1].read(base + depth * mod_r);
                (
                    l * (1.0 - pr[2] * 0.5) + wl * pr[2],
                    r * (1.0 - pr[2] * 0.5) + wr * pr[2],
                )
            }
            State::Phaser(s) => {
                s.phase += pr[0] / sr;
                if s.phase >= 1.0 {
                    s.phase -= 1.0;
                }
                // The notch sweeps between 300 Hz and 3 kHz, in equal musical steps.
                let sweep = 0.5 + 0.5 * (s.phase * TAU).sin();
                let hz = 300.0 * 10f32.powf(sweep * pr[1]);
                let t = (PI * hz / sr).tan();
                let a = (t - 1.0) / (t + 1.0);
                let input = [l, r];
                let mut out = [0f32; 2];
                for ch in 0..2 {
                    let mut x = input[ch] + s.last[ch] * pr[2];
                    for st in 0..4 {
                        let y = a * x + s.x1[ch][st] - a * s.y1[ch][st];
                        s.x1[ch][st] = x;
                        s.y1[ch][st] = y;
                        x = y;
                    }
                    s.last[ch] = x;
                    out[ch] = input[ch] * (1.0 - pr[3] * 0.5) + x * pr[3] * 0.5;
                }
                (out[0], out[1])
            }
            State::Drive(s) => {
                let drive = 10f32.powf(pr[0] / 20.0);
                let level = 10f32.powf(pr[2] / 20.0);
                let tone = 1.0 - (-TAU * pr[1] / sr).exp();
                let input = [l, r];
                let mut out = [0f32; 2];
                for ch in 0..2 {
                    let shaped = (input[ch] * drive).tanh();
                    // Remove the DC the shaping can add.
                    let dc = shaped - s.dc_in[ch] + 0.995 * s.dc_out[ch];
                    s.dc_in[ch] = shaped;
                    s.dc_out[ch] = dc;
                    s.tone[ch] += (dc - s.tone[ch]) * tone;
                    out[ch] = input[ch] * (1.0 - pr[3]) + s.tone[ch] * level * pr[3];
                }
                (out[0], out[1])
            }
            State::Filter(f) => (f[0].tick(l), f[1].tick(r)),
            State::MultiEq(f) => {
                let [left, right] = f;
                let a = left.iter_mut().fold(l, |x, filter| filter.tick(x));
                let b = right.iter_mut().fold(r, |x, filter| filter.tick(x));
                (a, b)
            }
            State::Gate(s) => {
                // About 5 ms of memory in the level, so one low sample of a wave does not
                // close the gate.
                s.env = kl.abs().max(kr.abs()).max(s.env * (1.0 - 200.0 / sr));
                let open_at = 10f32.powf(pr[0] / 20.0);
                if s.env > open_at {
                    s.open = true;
                    s.hold_left = (pr[3] * 0.001 * sr) as u32;
                } else if s.open && s.env < open_at * 0.708 {
                    if s.hold_left > 0 {
                        s.hold_left -= 1;
                    } else {
                        s.open = false;
                    }
                }
                let target = if s.open {
                    1.0
                } else {
                    10f32.powf(pr[1] / 20.0)
                };
                let ms = if target > s.gain { pr[2] } else { pr[4] };
                s.gain += (target - s.gain) * (1.0 - (-1.0 / (ms * 0.001 * sr)).exp());
                (l * s.gain, r * s.gain)
            }
            State::Limiter(s) => {
                let gain_in = 10f32.powf(pr[0] / 20.0);
                let ceiling = 10f32.powf(pr[1] / 20.0);
                let n = s.needs.len();
                let (xl, xr) = (l * gain_in, r * gain_in);
                let peak = xl.abs().max(xr.abs());
                let need = if peak > ceiling { ceiling / peak } else { 1.0 };
                let pos = s.pos;
                s.needs[pos] = need;
                // The gain the loudest of the last n samples asks for, averaged over n
                // samples: it is reached before that loudest sample comes out.
                let lowest = s.needs.iter().fold(1.0f32, |m, v| m.min(*v));
                s.avg_sum += lowest - s.avg[pos];
                s.avg[pos] = lowest;
                let smooth = (s.avg_sum / n as f32).clamp(0.0, 1.0);
                let release = 1.0 - (-1.0 / (pr[2] * 0.001 * sr)).exp();
                if smooth < s.gain {
                    s.gain = smooth;
                } else {
                    s.gain += (smooth - s.gain) * release;
                }
                s.delay[0][pos] = xl;
                s.delay[1][pos] = xr;
                s.pos = (pos + 1) % n;
                // The sound that went in n - 1 samples ago.
                let (dl, dr) = (s.delay[0][s.pos], s.delay[1][s.pos]);
                (
                    (dl * s.gain).clamp(-ceiling, ceiling),
                    (dr * s.gain).clamp(-ceiling, ceiling),
                )
            }
        };
        // A bad sample (NaN, infinity) never travels on: it becomes silence and the
        // effect's memory is dropped by the caller's next reset.
        if ol.is_finite() && or.is_finite() {
            (ol, or)
        } else {
            self.reset();
            (0.0, 0.0)
        }
    }

    /// Clears the effect's memory (tails, filter state).
    pub fn reset(&mut self) {
        let sr = self.sr;
        match &mut self.state {
            State::Eq(f) => f.iter_mut().flatten().for_each(|b| *b = b.cleared()),
            State::Filter(f) => f.iter_mut().for_each(|b| *b = b.cleared()),
            State::MultiEq(f) => f.iter_mut().flatten().for_each(|b| *b = b.cleared()),
            State::Gate(g) => {
                g.env = 0.0;
                g.open = false;
                g.hold_left = 0;
                g.gain = 0.0;
            }
            State::Limiter(l) => **l = LimiterState::new(sr),
            State::Comp(c) => c.reduction = 0.0,
            State::Reverb(s) => **s = ReverbState::new(sr),
            State::Delay(s) => {
                s.lines.iter_mut().for_each(|l| l.buf.fill(0.0));
                s.tone = [0.0; 2];
            }
            State::Chorus(s) => s.lines.iter_mut().for_each(|l| l.buf.fill(0.0)),
            State::Ember(s) => s.reset(),
            State::Heat(s) => s.reset(),
            State::Calm(s) => s.reset(),
            State::Prism(s) => s.reset(),
            State::Press(s) => s.reset(),
            State::Wall(s) => s.reset(),
            State::Snap(s) => s.reset(),
            State::Clip(s) => s.reset(),
            State::Drift(s) => s.reset(),
            State::Halo(s) => s.reset(),
            State::Swirl(s) => s.reset(),
            State::Bands(s) => s.reset(),
            State::Width(s) => s.reset(),
            State::Shut(s) => s.reset(),
            State::Reel(s) => s.reset(),
            State::Grit(s) => s.reset(),
            State::Flange(s) => s.reset(),
            State::Sweep(s) => s.reset(),
            State::Pulse(s) => s.reset(),
            State::Shift(s) => s.reset(),
            State::Utility(s) => s.reset(),
            State::Meter => {}
            State::Air(s) => s.reset(),
            State::Freq(s) => s.reset(),
            State::Phaser(s) => {
                s.x1 = [[0.0; 4]; 2];
                s.y1 = [[0.0; 4]; 2];
                s.last = [0.0; 2];
            }
            State::Drive(s) => {
                s.tone = [0.0; 2];
                s.dc_in = [0.0; 2];
                s.dc_out = [0.0; 2];
            }
        }
    }
}

/// The effects of one channel strip, in order.
/// A copy of what goes into and comes out of one effect, for its window to draw: three numbers a
/// sample, the input's middle (left plus right, halved), then the output's left and right.
pub struct FxTap {
    pub slot: usize,
    pub out: Box<rtrb::Producer<f32>>,
}

impl std::fmt::Debug for FxTap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FxTap({})", self.slot)
    }
}

#[derive(Debug, Default)]
pub struct Chain {
    pub fx: Vec<Fx>,
    bpm: f32,
    /// The effect whose window is in front listens here.
    pub tap: Option<FxTap>,
}

impl Clone for Chain {
    /// A copy plays the same, without the tap (only one window listens).
    fn clone(&self) -> Self {
        Self {
            fx: self.fx.clone(),
            bpm: self.bpm,
            tap: None,
        }
    }
}

impl Chain {
    /// A chain of at most [`MAX_INSERTS`] effects.
    pub fn new(mut fx: Vec<Fx>) -> Self {
        fx.truncate(MAX_INSERTS);
        Self {
            fx,
            bpm: 0.0,
            tap: None,
        }
    }

    #[inline]
    pub fn process(&mut self, mut l: f32, mut r: f32) -> (f32, f32) {
        if let Some(tap) = self.tap.as_mut() {
            for (k, f) in self.fx.iter_mut().enumerate() {
                let before = (l + r) * 0.5;
                (l, r) = f.process(l, r);
                // If nobody is reading, it is lost: the audio does not wait.
                if k == tap.slot && tap.out.slots() >= 3 {
                    let _ = tap.out.push(before);
                    let _ = tap.out.push(l);
                    let _ = tap.out.push(r);
                }
            }
            return (l, r);
        }
        for f in self.fx.iter_mut() {
            (l, r) = f.process(l, r);
        }
        (l, r)
    }

    /// The whole span through the chain, one effect at a time over every frame (rather than every
    /// effect for one frame): each effect's code and state stay in cache and its branches settle.
    /// The sound is the same as [`Chain::process`] frame by frame, as the settings do not change
    /// within a span. The frames are 32-bit values held in 64 bits. `side(strip, frame)` is the
    /// key of a keyed effect.
    pub fn process_block(
        &mut self,
        l: &mut [f64],
        r: &mut [f64],
        side: impl Fn(usize, usize) -> (f32, f32),
    ) {
        let n = l.len().min(r.len());
        let tap = &mut self.tap;
        for (k, f) in self.fx.iter_mut().enumerate() {
            let tapped = tap.as_ref().is_some_and(|t| t.slot == k);
            if f.bypassed() && !tapped {
                continue;
            }
            let key = f.side;
            if key.is_none() && !tapped && f.run_block(&mut l[..n], &mut r[..n]) {
                continue;
            }
            for j in 0..n {
                if let Some(s) = key {
                    let (a, b) = side(s as usize, j);
                    f.feed_side(a, b);
                }
                let (x, y) = (l[j] as f32, r[j] as f32);
                let (a, b) = f.process(x, y);
                if tapped {
                    if let Some(tap) = tap.as_mut() {
                        // If nobody is reading, it is lost: the audio does not wait.
                        if tap.out.slots() >= 3 {
                            let _ = tap.out.push((x + y) * 0.5);
                            let _ = tap.out.push(a);
                            let _ = tap.out.push(b);
                        }
                    }
                }
                l[j] = f64::from(a);
                r[j] = f64::from(b);
            }
        }
    }

    /// Gives each keyed effect its key's sound for this sample: `side(strip)` is that strip's.
    #[inline]
    pub fn feed_sides(&mut self, side: impl Fn(usize) -> (f32, f32)) {
        for f in self.fx.iter_mut() {
            if let Some(s) = f.side {
                let (l, r) = side(s as usize);
                f.feed_side(l, r);
            }
        }
    }

    /// The strips that key effects of this chain.
    pub fn side_sources(&self) -> impl Iterator<Item = u16> + '_ {
        self.fx.iter().filter_map(Fx::side_source)
    }

    /// Tells the effects the project tempo (only passed on when it changes).
    #[inline]
    pub fn set_tempo(&mut self, bpm: f32) {
        if bpm != self.bpm {
            self.bpm = bpm;
            self.fx.iter_mut().for_each(|f| f.set_tempo(bpm));
        }
    }

    /// The largest gain reduction of the compressors in the chain, in dB.
    pub fn reduction_db(&self) -> f32 {
        self.fx.iter().map(Fx::reduction_db).fold(0.0, f32::max)
    }

    /// The delay of the whole chain, in samples.
    pub fn latency_samples(&self) -> usize {
        self.fx.iter().map(Fx::latency_samples).sum()
    }

    /// Automation: moves parameter `param` of effect `slot` (no effect there: nothing).
    pub fn set_param(&mut self, slot: usize, param: usize, value: f32) {
        if let Some(fx) = self.fx.get_mut(slot) {
            fx.set_param(param, value);
        }
    }

    /// Automation: moves parameter `param` of effect `slot` to position `norm` (0..=1) along
    /// the parameter's range.
    pub fn set_param_norm(&mut self, slot: usize, param: usize, norm: f32) {
        if let Some(fx) = self.fx.get_mut(slot) {
            if let Some(info) = fx.kind().params().get(param) {
                fx.set_param(param, info.from_norm(norm));
            }
        }
    }

    /// Puts every effect back to the parameters the user set.
    pub fn restore(&mut self) {
        self.fx.iter_mut().for_each(Fx::restore);
    }
}

/// The compensation delays of one strip (see `EngineCore`): `before` delays the sound
/// ahead of the strip's sends and fader, `after` the sound that goes to the mix. Both are
/// made off the audio thread, with their exact lengths.
#[derive(Clone, Debug, Default)]
pub struct Pdc {
    before: [Line; 2],
    after: [Line; 2],
    before_len: usize,
    after_len: usize,
}

impl Default for Line {
    fn default() -> Self {
        Line::new(1)
    }
}

impl Pdc {
    /// Delays of `before` and `after` samples (at most 192 000 each, 4 s at 48 kHz).
    pub fn new(before: usize, after: usize) -> Self {
        let (before, after) = (before.min(192_000), after.min(192_000));
        Self {
            before: [Line::new(before), Line::new(before)],
            after: [Line::new(after), Line::new(after)],
            before_len: before,
            after_len: after,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.before_len == 0 && self.after_len == 0
    }

    pub fn lengths(&self) -> (usize, usize) {
        (self.before_len, self.after_len)
    }

    #[inline]
    pub fn delay_before(&mut self, l: f32, r: f32) -> (f32, f32) {
        Self::through(&mut self.before, self.before_len, l, r)
    }

    #[inline]
    pub fn delay_after(&mut self, l: f32, r: f32) -> (f32, f32) {
        Self::through(&mut self.after, self.after_len, l, r)
    }

    #[inline]
    fn through(lines: &mut [Line; 2], len: usize, l: f32, r: f32) -> (f32, f32) {
        if len == 0 {
            return (l, r);
        }
        lines[0].push(l);
        lines[1].push(r);
        (lines[0].read(len as f32), lines[1].read(len as f32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn sine(hz: f32, secs: f32, amp: f32) -> Vec<f32> {
        (0..(secs * SR) as usize)
            .map(|i| (i as f32 * TAU * hz / SR).sin() * amp)
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    fn run(fx: &mut Fx, input: &[f32]) -> Vec<f32> {
        input.iter().map(|x| fx.process(*x, *x).0).collect()
    }

    fn with(kind: FxKind, set: &[(usize, f32)]) -> Fx {
        let mut p = kind.defaults();
        for (i, v) in set {
            p[*i] = *v;
        }
        Fx::new(kind, p, false, SR)
    }

    #[test]
    fn every_kind_has_names_and_consistent_defaults() {
        for k in FxKind::ALL {
            assert_eq!(FxKind::from_name(k.name()), Some(k), "{k:?}");
            let d = k.defaults();
            assert_eq!(k.clamp_params(d), d, "{k:?}");
            for info in k.params() {
                assert!(info.min < info.max && (info.min..=info.max).contains(&info.default));
                assert!(!info.log || info.min > 0.0);
            }
            assert!(k.params().len() <= MAX_FX_PARAMS);
        }
        assert_eq!(FxKind::from_name("Space Designer"), Some(FxKind::Reverb));
        assert_eq!(FxKind::from_name("MultiMeter"), None);
    }

    #[test]
    fn wild_parameters_are_put_into_range() {
        for k in FxKind::ALL {
            let bad = [f32::NAN, f32::INFINITY, -1e30, 1e30, f32::NEG_INFINITY, 7.0];
            let wild: [f32; MAX_FX_PARAMS] = std::array::from_fn(|i| bad[i % bad.len()]);
            let fixed = k.clamp_params(wild);
            for (v, info) in fixed.iter().zip(k.params()) {
                assert!((info.min..=info.max).contains(v), "{k:?} {v}");
            }
            let mut fx = Fx::new(k, wild, false, SR);
            let out = run(&mut fx, &sine(440.0, 0.3, 0.8));
            assert!(out.iter().all(|x| x.is_finite() && x.abs() < 40.0), "{k:?}");
        }
    }

    #[test]
    fn a_bypassed_effect_passes_the_sound_through() {
        for k in FxKind::ALL {
            let mut fx = Fx::new(k, k.defaults(), true, SR);
            assert_eq!(fx.process(0.25, -0.5), (0.25, -0.5), "{k:?}");
        }
    }

    #[test]
    fn the_eq_boosts_and_cuts_where_asked() {
        let mut up = with(FxKind::ChannelEq, &[(1, 12.0), (2, 1000.0)]);
        let tone = sine(1000.0, 0.5, 0.1);
        let boosted = rms(&run(&mut up, &tone)[12_000..]) / rms(&tone[12_000..]);
        assert!((boosted - 4.0).abs() < 0.3, "{boosted}");
        let mut far = with(FxKind::ChannelEq, &[(1, 12.0), (2, 1000.0), (3, 4.0)]);
        let low = sine(100.0, 0.5, 0.1);
        let ratio = rms(&run(&mut far, &low)[12_000..]) / rms(&low[12_000..]);
        assert!((ratio - 1.0).abs() < 0.1, "{ratio}");
        // The drawn curve agrees with the sound.
        let curve = eq_response_db(
            with(FxKind::ChannelEq, &[(1, 12.0), (2, 1000.0)]).params,
            &[1000.0, 50.0],
            SR,
        );
        assert!(
            (curve[0] - 12.0).abs() < 0.5 && curve[1].abs() < 1.5,
            "{curve:?}"
        );
        // A flat EQ changes nothing.
        let mut flat = with(FxKind::ChannelEq, &[]);
        let same = run(&mut flat, &tone);
        assert!(same.iter().zip(&tone).all(|(a, b)| (a - b).abs() < 1e-5));
    }

    #[test]
    fn the_compressor_reduces_loud_sound_and_leaves_quiet_alone() {
        let set = [(0, -20.0), (1, 4.0), (2, 1.0), (3, 50.0)];
        let mut c = with(FxKind::Compressor, &set);
        let loud = sine(440.0, 1.0, 0.8); // -2 dB
        let out = run(&mut c, &loud);
        let reduction = 20.0 * (rms(&loud[30_000..]) / rms(&out[30_000..])).log10();
        // 18 dB over a 4:1 ratio leaves 4.5 dB, so about 13.5 dB are taken away.
        assert!((reduction - 13.5).abs() < 1.5, "{reduction}");
        assert!(c.reduction_db() > 10.0);
        let mut quiet = with(FxKind::Compressor, &set);
        let soft = sine(440.0, 0.5, 0.01);
        let out = run(&mut quiet, &soft);
        assert!((rms(&out) / rms(&soft) - 1.0).abs() < 0.02);
        let mut makeup = with(FxKind::Compressor, &[(0, 0.0), (4, 6.0)]);
        let out = run(&mut makeup, &soft);
        assert!((rms(&out) / rms(&soft) - 2.0).abs() < 0.1);
    }

    #[test]
    fn the_delay_repeats_and_the_reverb_rings_out() {
        let mut d = with(FxKind::Delay, &[(0, 100.0), (1, 0.5), (4, 1.0)]);
        let mut hits = Vec::new();
        for i in 0..(0.5 * SR) as usize {
            let x = if i == 0 { 1.0 } else { 0.0 };
            let (l, _) = d.process(x, x);
            if l.abs() > 0.05 && i > 10 {
                hits.push(i);
            }
        }
        // Repeats 100 ms apart (4800 samples), each about half as loud as the one before.
        assert!(
            hits.iter().any(|i| (*i as i32 - 4800).abs() < 5),
            "{hits:?}"
        );
        assert!(
            hits.iter().any(|i| (*i as i32 - 9600).abs() < 5),
            "{hits:?}"
        );

        let mut rv = with(FxKind::Reverb, &[(3, 1.0)]);
        let mut tail = 0.0f32;
        let mut late = 0.0f32;
        for i in 0..(2.0 * SR) as usize {
            let x = if i == 0 { 1.0 } else { 0.0 };
            let (l, _) = rv.process(x, x);
            if i > 24_000 {
                tail = tail.max(l.abs());
            }
            if i > 90_000 {
                late = late.max(l.abs());
            }
        }
        assert!(tail > 1e-4, "no tail: {tail}");
        assert!(late < tail, "the tail does not die");
        // Reset silences it at once.
        rv.reset();
        assert_eq!(rv.process(0.0, 0.0), (0.0, 0.0));
    }

    #[test]
    fn filter_overdrive_chorus_and_phaser_do_what_they_say() {
        let hi = sine(8000.0, 0.3, 0.5);
        let lo = sine(100.0, 0.3, 0.5);
        let mut lp = with(FxKind::Filter, &[(0, 500.0)]);
        assert!(rms(&run(&mut lp, &hi)[5000..]) < 0.02);
        let mut lp = with(FxKind::Filter, &[(0, 500.0)]);
        assert!(rms(&run(&mut lp, &lo)[5000..]) > 0.3);
        let mut hp = with(FxKind::Filter, &[(0, 2000.0), (2, 1.0)]);
        assert!(rms(&run(&mut hp, &lo)[5000..]) < 0.02);

        // Overdrive squashes the peaks: the output is flatter than a plain gain would be.
        let mut od = with(FxKind::Overdrive, &[(0, 30.0), (1, 16000.0), (2, 0.0)]);
        let out = run(&mut od, &sine(200.0, 0.3, 0.5));
        let peak = out[5000..].iter().fold(0f32, |m, x| m.max(x.abs()));
        assert!(peak < 1.3 && rms(&out[5000..]) > 0.5, "{peak}");

        let mut ch = with(FxKind::Chorus, &[]);
        let out = run(&mut ch, &sine(440.0, 0.5, 0.5));
        assert!(rms(&out[10_000..]) > 0.1);
        let mut ph = with(FxKind::Phaser, &[]);
        let out = run(&mut ph, &sine(440.0, 0.5, 0.5));
        assert!(rms(&out[10_000..]) > 0.1 && out.iter().all(|x| x.abs() < 3.0));
    }

    #[test]
    fn changing_a_knob_does_not_click_and_a_chain_runs_in_order() {
        let mut fx = with(FxKind::ChannelEq, &[(1, 6.0)]);
        let tone = sine(300.0, 0.4, 0.5);
        let mut last = 0.0f32;
        let mut worst = 0.0f32;
        for (i, x) in tone.iter().enumerate() {
            if i == 9000 {
                let mut p = fx.params;
                p[1] = -12.0;
                fx.set(p, false);
            }
            let y = fx.process(*x, *x).0;
            worst = worst.max((y - last).abs());
            last = y;
        }
        assert!(worst < 0.2, "{worst}");

        let mut chain = Chain::new(vec![
            with(FxKind::Compressor, &[(0, -30.0), (1, 10.0)]),
            with(FxKind::ChannelEq, &[]),
        ]);
        let mut peak = 0f32;
        for (i, x) in sine(440.0, 0.5, 0.9).into_iter().enumerate() {
            let y = chain.process(x, x).0.abs();
            if i > 2000 {
                peak = peak.max(y);
            }
        }
        assert!(peak < 0.5 && chain.reduction_db() > 5.0, "{peak}");
        let too_many = MAX_INSERTS + 4;
        let many = Chain::new((0..too_many).map(|_| with(FxKind::Chorus, &[])).collect());
        assert_eq!(many.fx.len(), MAX_INSERTS);
    }

    #[test]
    fn the_gate_closes_on_quiet_sound_and_opens_on_loud_sound() {
        let mut g = with(
            FxKind::Gate,
            &[(0, -30.0), (1, -60.0), (2, 0.5), (3, 20.0), (4, 30.0)],
        );
        // Noise at -50 dB is shut out: -60 dB range means a thousandth of the level.
        let quiet = sine(300.0, 0.5, 0.003);
        let out = run(&mut g, &quiet);
        assert!(
            rms(&out[20_000..]) < rms(&quiet) * 0.02,
            "{}",
            rms(&out[20_000..])
        );
        assert!(g.reduction_db() > 40.0);
        // A loud tone opens it.
        let loud = sine(300.0, 0.3, 0.5);
        let out = run(&mut g, &loud);
        assert!((rms(&out[6_000..]) / rms(&loud[6_000..]) - 1.0).abs() < 0.05);
        assert!(g.reduction_db() < 1.0);
        // It stays open for the hold time and then closes again.
        let silence = vec![0.0f32; 48_000];
        let out = run(&mut g, &silence);
        assert!(out.iter().all(|x| x.is_finite()));
        let mut quiet_again = with(FxKind::Gate, &[(0, -30.0), (1, -80.0)]);
        run(&mut quiet_again, &loud);
        let tail = run(&mut quiet_again, &sine(300.0, 0.4, 0.003));
        assert!(rms(&tail[12_000..]) < 0.0005);
    }

    #[test]
    fn the_limiter_never_passes_the_ceiling_and_delays_by_its_look_ahead() {
        let mut lim = with(FxKind::Limiter, &[(1, -6.0), (2, 50.0)]);
        let loud = sine(200.0, 0.5, 1.5);
        let out = run(&mut lim, &loud);
        let ceiling = 10f32.powf(-6.0 / 20.0);
        assert!(
            out.iter().all(|x| x.abs() <= ceiling + 1e-5),
            "{}",
            out.iter().fold(0f32, |m, x| m.max(x.abs()))
        );
        // It does limit: the tone is pushed down to the ceiling, not cut away.
        assert!(rms(&out[20_000..]) > 0.3);
        assert!(lim.reduction_db() > 6.0);
        // Quiet sound passes at its own level, a look-ahead late.
        let mut lim = with(FxKind::Limiter, &[]);
        let delay = FxKind::Limiter.latency_samples(SR);
        assert_eq!(delay, lim.latency_samples());
        assert!(delay > 50 && delay < 200, "{delay}");
        let mut got = Vec::new();
        for i in 0..2000 {
            let x = if i == 100 { 0.25 } else { 0.0 };
            got.push(lim.process(x, x).0);
        }
        let at = got.iter().position(|v| v.abs() > 0.01);
        assert_eq!(
            at,
            Some(100 + delay),
            "the click comes out one look-ahead later"
        );
        assert!((got[100 + delay] - 0.25).abs() < 1e-5);
        // A bypassed limiter adds no delay.
        let mut off = Fx::new(FxKind::Limiter, FxKind::Limiter.defaults(), true, SR);
        assert_eq!(off.latency_samples(), 0);
        assert_eq!(off.process(0.5, 0.5), (0.5, 0.5));
        // A single sharp peak is held to the ceiling too (the gain is down before it comes).
        let mut lim = with(FxKind::Limiter, &[(1, -3.0), (2, 20.0)]);
        let mut peak = 0f32;
        for i in 0..4000 {
            let x = if i == 2000 { 4.0 } else { 0.05 };
            peak = peak.max(lim.process(x, -x).0.abs());
        }
        assert!(peak <= 10f32.powf(-3.0 / 20.0) + 1e-5, "{peak}");
    }

    #[test]
    fn the_multi_band_eq_has_six_bands_and_the_curve_follows_them() {
        let mut pr = FxKind::MultiEq.defaults();
        pr[1] = 9.0; // low shelf up
        pr[5] = 800.0;
        pr[6] = -12.0; // a notch at 800 Hz
        pr[7] = 2.0;
        pr[15] = 6.0; // high shelf up
        let curve = response_db(FxKind::MultiEq, pr, &[30.0, 800.0, 17_000.0, 2_000.0], SR);
        assert!(curve[0] > 6.0, "{curve:?}");
        assert!((curve[1] + 12.0).abs() < 1.5, "{curve:?}");
        assert!(curve[2] > 4.0, "{curve:?}");
        assert!(curve[3].abs() < 4.0);
        // The sound agrees with the curve.
        let mut fx = Fx::new(FxKind::MultiEq, pr, false, SR);
        let tone = sine(800.0, 0.5, 0.2);
        let out = run(&mut fx, &tone);
        let db = 20.0 * (rms(&out[12_000..]) / rms(&tone[12_000..])).log10();
        assert!((db + 12.0).abs() < 1.5, "{db}");
        // A flat one changes nothing, and it does not click when a band moves.
        let mut flat = Fx::new(FxKind::MultiEq, FxKind::MultiEq.defaults(), false, SR);
        let same = run(&mut flat, &tone);
        assert!(same.iter().zip(&tone).all(|(a, b)| (a - b).abs() < 1e-4));
        assert_eq!(response_db(FxKind::Reverb, pr, &[1000.0], SR), [0.0]);
    }

    #[test]
    fn automation_moves_a_parameter_and_restore_puts_the_users_value_back() {
        let mut fx = with(FxKind::ChannelEq, &[(1, 6.0)]);
        let tone = sine(1000.0, 0.4, 0.1);
        let base = rms(&run(&mut fx, &tone)[10_000..]);
        fx.set_param(1, -12.0);
        assert_eq!(fx.current()[1], -12.0);
        let moved = rms(&run(&mut fx, &tone)[10_000..]);
        assert!(moved < base * 0.4, "{moved} {base}");
        fx.set_param(1, 500.0);
        assert_eq!(fx.current()[1], 18.0, "put into range");
        fx.set_param(99, 1.0);
        fx.set_param(0, f32::NAN);
        fx.restore();
        assert_eq!(fx.current()[1], 6.0);
        // Setting new base values forgets an automated one.
        fx.set_param(1, 0.0);
        let mut p = fx.current();
        p[1] = 3.0;
        fx.set(p, false);
        assert_eq!(fx.current()[1], 3.0);
        let mut chain = Chain::new(vec![fx]);
        chain.set_param(0, 1, -3.0);
        chain.set_param(7, 1, -3.0);
        chain.restore();
        assert_eq!(chain.fx[0].current()[1], 3.0);
    }

    #[test]
    fn delay_compensation_lines_delay_exactly() {
        let mut pdc = Pdc::new(10, 25);
        assert_eq!(pdc.lengths(), (10, 25));
        assert!(!pdc.is_empty() && Pdc::new(0, 0).is_empty());
        let mut a = Vec::new();
        let mut b = Vec::new();
        for i in 0..80 {
            let x = if i == 3 { 1.0 } else { 0.0 };
            a.push(pdc.delay_before(x, -x).0);
            b.push(pdc.delay_after(x, -x).1);
        }
        assert_eq!(a.iter().position(|v| *v == 1.0), Some(13));
        assert_eq!(b.iter().position(|v| *v == -1.0), Some(28));
        let mut none = Pdc::new(0, 0);
        assert_eq!(none.delay_before(0.5, 0.25), (0.5, 0.25));
        assert_eq!(Pdc::new(usize::MAX, 0).lengths().0, 192_000);
    }
}
