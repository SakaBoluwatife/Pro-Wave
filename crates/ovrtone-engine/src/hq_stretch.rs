// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// Adapted from Lemon Studio (crates/os-dsp/src/stretch.rs): https://github.com/ItsAlphaNeon/Lemon-Studio

//! The Complex Flex mode: pitch-keeping time stretch by Signalsmith Stretch (MIT licence, Signalsmith
//! Audio), through the `signalsmith-stretch` binding (MIT licence, Colin Marc). Offline only: nothing
//! here runs on the audio thread.
//!
//! - [`whole`]: a stereo buffer to a given length, at the same pitch.
//! - [`warp`]: piecewise, between anchors. Each piece is stretched with a little of its neighbours
//!   around it, and the overlaps are crossfaded, so there is no hard join at an anchor. Pieces with
//!   the same ratio side by side are stretched as one.

use signalsmith_stretch::Stretch;

/// Stretches `left`/`right` (`rate` Hz) to `frames` frames, the pitch kept. The result starts where
/// the input starts: the stretcher's latency is taken out.
pub fn whole(left: &[f32], right: &[f32], rate: u32, frames: usize) -> (Vec<f32>, Vec<f32>) {
    let n = left.len().min(right.len());
    if frames == 0 || n == 0 {
        return (vec![0.0; frames], vec![0.0; frames]);
    }
    let mut st = Stretch::preset_default(2, rate.max(1));
    // The stretcher needs an output of at least twice its latency: a shorter one is made from the input
    // padded with silence in proportion, and the padding's part of the output is cut off.
    let out_n = frames.max(2 * st.output_latency() + 1);
    let in_n = if out_n > frames {
        ((n as f64 * out_n as f64 / frames as f64).round() as usize).max(n)
    } else {
        n
    };
    let mut input = vec![0.0f32; in_n * 2];
    for i in 0..n {
        input[2 * i] = left[i];
        input[2 * i + 1] = right[i];
    }
    let mut out = vec![0.0f32; out_n * 2];
    exact(&mut st, &input, &mut out);
    let l = out.iter().step_by(2).take(frames).copied().collect();
    let r = out.iter().skip(1).step_by(2).take(frames).copied().collect();
    (l, r)
}

/// `frames` of stereo `piece` (interleaved) at the same length, moved in pitch by `transpose_at(frame)`
/// semitones block by block (so it can follow a pitch curve), the formants kept and then moved
/// `formant` semitones. The stretcher's latency is taken out.
pub fn retune(
    piece: &[f32],
    frames: usize,
    rate: u32,
    formant: f32,
    transpose_at: impl Fn(usize) -> f32,
) -> Vec<f32> {
    const BLOCK: usize = 256;
    let mut st = Stretch::preset_default(2, rate.max(1));
    st.set_formant_factor(2f32.powf(formant / 12.0), true);
    let latency = st.input_latency() + st.output_latency();
    let total = frames + latency;
    let mut out = vec![0.0f32; total * 2];
    let mut input = vec![0.0f32; BLOCK * 2];
    let mut at = 0;
    while at < total {
        let n = BLOCK.min(total - at);
        for i in 0..n {
            let k = at + i;
            input[2 * i] = piece.get(2 * k).copied().unwrap_or(0.0);
            input[2 * i + 1] = piece.get(2 * k + 1).copied().unwrap_or(0.0);
        }
        st.set_transpose_factor(2f32.powf(transpose_at(at) / 12.0), None);
        st.process(&input[..n * 2], &mut out[at * 2..(at + n) * 2]);
        at += n;
    }
    out.drain(..latency * 2);
    out.truncate(frames * 2);
    out
}

/// The whole of `input` into the whole of `out` (interleaved stereo, `out` at least twice the output
/// latency), the latency taken out. The first input latency's worth it seeks on is zero-padded, so a
/// sample shorter than that is never read past its end.
fn exact(st: &mut Stretch, input: &[f32], out: &mut [f32]) {
    let (n, frames) = (input.len() / 2, out.len() / 2);
    let (lat_in, lat_out) = (st.input_latency(), st.output_latency());
    if frames < 2 * lat_out {
        out.fill(0.0);
        return;
    }
    let mut head = vec![0.0f32; lat_in * 2];
    let m = n.min(lat_in) * 2;
    head[..m].copy_from_slice(&input[..m]);
    st.seek(&head, n as f64 / frames as f64);
    let mut rest = vec![0.0f32; n * 2];
    if n > lat_in {
        rest[..(n - lat_in) * 2].copy_from_slice(&input[lat_in * 2..]);
    }
    st.process(&rest, &mut *out);
    // Fold the first bit of the output back onto itself, then shift along by the output latency.
    for i in 0..(frames - lat_out).min(lat_out) {
        for c in 0..2 {
            out[(i + lat_out) * 2 + c] -= out[(lat_out - 1 - i) * 2 + c];
        }
    }
    out.copy_within(lat_out * 2.., 0);
    st.flush(&mut out[(frames - lat_out) * 2..]);
}

/// Piecewise warp of `left`/`right` between `points` (source frame, destination frame), which start at
/// destination 0 and rise in both. Err when the anchors do not make sense or a piece would stretch past
/// 1/16 or 16 times.
pub fn warp(
    left: &[f32],
    right: &[f32],
    rate: u32,
    points: &[(usize, usize)],
) -> Result<(Vec<f32>, Vec<f32>), String> {
    let n = left.len().min(right.len());
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return Err("No anchors".into());
    };
    if points.len() < 2
        || first.1 != 0
        || last.0 > n
        || points.windows(2).any(|w| w[1].0 <= w[0].0 || w[1].1 <= w[0].1)
    {
        return Err("Warp anchors must stay in order and inside the sample".into());
    }
    // Pieces of the same ratio side by side go as one.
    let ratio = |a: (usize, usize), b: (usize, usize)| (b.1 - a.1) as f64 / (b.0 - a.0) as f64;
    let mut anchors = vec![*first];
    for (k, p) in points.iter().enumerate().skip(1) {
        let keep = match points.get(k + 1) {
            Some(next) => {
                let prev = anchors.last().copied().unwrap_or(*first);
                (ratio(prev, *p) - ratio(*p, *next)).abs() > 1e-4
            }
            None => true,
        };
        if keep {
            anchors.push(*p);
        }
    }
    let frames = last.1;
    let start = first.0;
    if anchors.iter().all(|&(s, d)| s - start == d) {
        return Ok((
            left[start..start + frames].to_vec(),
            right[start..start + frames].to_vec(),
        ));
    }
    let (mut l, mut r, mut weights) = (vec![0f32; frames], vec![0f32; frames], vec![0f32; frames]);
    for pair in anchors.windows(2) {
        let ((a, x), (b, y)) = (pair[0], pair[1]);
        if b - a == y - x {
            for j in 0..(b - a) {
                l[x + j] += left[a + j];
                r[x + j] += right[a + j];
                weights[x + j] += 1.0;
            }
            continue;
        }
        let k = (y - x) as f64 / (b - a) as f64;
        if !(0.0625..=16.0).contains(&k) {
            return Err("Keep each piece between 1/16 and 16 times its length".into());
        }
        let pad = (rate as usize / 100).min((b - a) / 4).max(1);
        let lo = a.saturating_sub(pad);
        let hi = (b + pad).min(n);
        let before = ((a - lo) as f64 * k).round() as usize;
        let after = ((hi - b) as f64 * k).round() as usize;
        let (sl, sr) = whole(&left[lo..hi], &right[lo..hi], rate, y - x + before + after);
        for j in 0..sl.len() {
            let dest = x as isize + j as isize - before as isize;
            if dest < 0 || dest >= frames as isize {
                continue;
            }
            let w = if j < before {
                (j + 1) as f32 / (before + 1) as f32
            } else if j >= before + y - x {
                (sl.len() - j) as f32 / (after + 1) as f32
            } else {
                1.0
            };
            let d = dest as usize;
            l[d] += sl[j] * w;
            r[d] += sr[j] * w;
            weights[d] += w;
        }
    }
    for i in 0..frames {
        if weights[i] > 0.0 {
            l[i] /= weights[i];
            r[i] /= weights[i];
        }
    }
    // A mono source stays mono: the stretcher may tell identical inputs apart.
    if left[..n] == right[..n] {
        r.clone_from(&l);
    }
    Ok((l, r))
}
