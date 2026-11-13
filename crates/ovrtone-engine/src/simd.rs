//! Vector kernels for the mix, chosen once for the processor the program runs on: AVX2 where the
//! processor has it, otherwise the portable code (SSE2 on every x86-64, NEON on ARM).
//!
//! Every path runs the same Rust body in the same order: the AVX2 build only lets the compiler
//! use wider registers. Rust never fuses a multiply and an add on its own, and each sum keeps
//! four lanes added in a fixed order, so every processor gets the same bits.

use std::sync::atomic::{AtomicU8, Ordering};

/// Which kernels run on this processor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimdLevel {
    /// The portable code (SSE2 on x86-64, NEON on 64-bit ARM).
    Portable,
    /// 256-bit vectors.
    Avx2,
}

impl SimdLevel {
    pub fn name(self) -> &'static str {
        match self {
            Self::Portable if cfg!(target_arch = "x86_64") => "SSE2",
            Self::Portable if cfg!(target_arch = "aarch64") => "NEON",
            Self::Portable => "Portable",
            Self::Avx2 => "AVX2",
        }
    }
}

/// 0: not looked yet, 1: portable, 2: AVX2.
static LEVEL: AtomicU8 = AtomicU8::new(0);

/// The kernels in use. The first call asks the processor; later ones read what it said.
pub fn simd_level() -> SimdLevel {
    match LEVEL.load(Ordering::Relaxed) {
        1 => SimdLevel::Portable,
        2 => SimdLevel::Avx2,
        _ => {
            let level = detect();
            let code = if level == SimdLevel::Avx2 { 2 } else { 1 };
            LEVEL.store(code, Ordering::Relaxed);
            level
        }
    }
}

fn detect() -> SimdLevel {
    #[cfg(target_arch = "x86_64")]
    if std::is_x86_feature_detected!("avx2") {
        return SimdLevel::Avx2;
    }
    SimdLevel::Portable
}

/// What [`mix_into`] measured on the way: the loudest sample of each side, and the sum of the mean
/// square of the two sides over the frames.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MixMeasure {
    pub peak_l: f64,
    pub peak_r: f64,
    pub square_sum: f64,
}

/// Adds a strip's sound to the mix: `x * trim * gain` per side, measuring either what went in
/// (`pre_fader`) or what came out. The four slices are cut to the shortest.
#[allow(clippy::too_many_arguments)]
#[inline]
pub fn mix_into(
    in_l: &[f64],
    in_r: &[f64],
    trim: f64,
    gain_l: f64,
    gain_r: f64,
    pre_fader: bool,
    mix_l: &mut [f64],
    mix_r: &mut [f64],
) -> MixMeasure {
    #[cfg(target_arch = "x86_64")]
    if simd_level() == SimdLevel::Avx2 {
        // SAFETY: `simd_level` said this processor has AVX2.
        return unsafe { mix_avx2(in_l, in_r, trim, gain_l, gain_r, pre_fader, mix_l, mix_r) };
    }
    mix_body(in_l, in_r, trim, gain_l, gain_r, pre_fader, mix_l, mix_r)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[allow(clippy::too_many_arguments)]
unsafe fn mix_avx2(
    in_l: &[f64],
    in_r: &[f64],
    trim: f64,
    gain_l: f64,
    gain_r: f64,
    pre_fader: bool,
    mix_l: &mut [f64],
    mix_r: &mut [f64],
) -> MixMeasure {
    mix_body(in_l, in_r, trim, gain_l, gain_r, pre_fader, mix_l, mix_r)
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn mix_body(
    in_l: &[f64],
    in_r: &[f64],
    trim: f64,
    gain_l: f64,
    gain_r: f64,
    pre_fader: bool,
    mix_l: &mut [f64],
    mix_r: &mut [f64],
) -> MixMeasure {
    let n = in_l.len().min(in_r.len()).min(mix_l.len()).min(mix_r.len());
    let (in_l, in_r) = (&in_l[..n], &in_r[..n]);
    let (mix_l, mix_r) = (&mut mix_l[..n], &mut mix_r[..n]);
    let mut pk_l = [0f64; 4];
    let mut pk_r = [0f64; 4];
    let mut sq = [0f64; 4];
    let mut lane = |k: usize, i: usize, mix_l: &mut [f64], mix_r: &mut [f64]| {
        let (xl, xr) = (in_l[i] * trim, in_r[i] * trim);
        let (yl, yr) = (xl * gain_l, xr * gain_r);
        mix_l[i] += yl;
        mix_r[i] += yr;
        let (ml, mr) = if pre_fader { (xl, xr) } else { (yl, yr) };
        let (al, ar) = (ml.abs(), mr.abs());
        pk_l[k] = if al > pk_l[k] { al } else { pk_l[k] };
        pk_r[k] = if ar > pk_r[k] { ar } else { pk_r[k] };
        sq[k] += (ml * ml + mr * mr) * 0.5;
    };
    let whole = n / 4 * 4;
    let mut i = 0;
    while i < whole {
        lane(0, i, mix_l, mix_r);
        lane(1, i + 1, mix_l, mix_r);
        lane(2, i + 2, mix_l, mix_r);
        lane(3, i + 3, mix_l, mix_r);
        i += 4;
    }
    while i < n {
        lane(i % 4, i, mix_l, mix_r);
        i += 1;
    }
    let max4 = |v: [f64; 4]| v.iter().fold(0f64, |m, &x| if x > m { x } else { m });
    MixMeasure {
        peak_l: max4(pk_l),
        peak_r: max4(pk_r),
        square_sum: (sq[0] + sq[1]) + (sq[2] + sq[3]),
    }
}
