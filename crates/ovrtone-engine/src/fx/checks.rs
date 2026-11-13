//! Checks every effect of the Ovrtone set must pass.
use super::{Fx, FxKind, MAX_FX_PARAMS};

const NEW: [FxKind; 23] = [
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
];

fn noise(n: usize) -> Vec<f32> {
    let mut seed = 7u32;
    (0..n)
        .map(|_| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 1.6
        })
        .collect()
}

/// Every parameter at its lowest, highest and default.
fn settings(kind: FxKind) -> Vec<[f32; MAX_FX_PARAMS]> {
    let mut low = [0.0; MAX_FX_PARAMS];
    let mut high = [0.0; MAX_FX_PARAMS];
    for (k, info) in kind.params().iter().enumerate() {
        low[k] = info.min;
        high[k] = info.max;
    }
    vec![kind.defaults(), low, high]
}

#[test]
fn every_new_effect_stays_finite_and_in_bounds_with_any_settings() {
    let sr = 48_000.0;
    let input = noise(48_000);
    for kind in NEW {
        for p in settings(kind) {
            let mut fx = Fx::new(kind, p, false, sr);
            for x in &input {
                let (l, r) = fx.process(*x, -*x * 0.7);
                assert!(l.is_finite() && r.is_finite(), "{kind:?}");
                // Six EQ bands all at +24 dB on one frequency are allowed to be loud.
                let bound = if kind == FxKind::Prism { 1e9 } else { 100.0 };
                assert!(l.abs() < bound && r.abs() < bound, "{kind:?} ran away: {l}");
            }
        }
    }
}

#[test]
fn every_new_effect_turns_silence_into_silence() {
    for kind in NEW {
        let mut fx = Fx::new(kind, kind.defaults(), false, 48_000.0);
        for _ in 0..24_000 {
            let (l, r) = fx.process(0.0, 0.0);
            assert!(l.abs() < 1e-4 && r.abs() < 1e-4, "{kind:?}: {l}");
        }
    }
}

#[test]
fn the_effects_that_look_ahead_say_how_late_they_are() {
    let sr = 48_000.0;
    for kind in [FxKind::Calm, FxKind::Wall, FxKind::Press] {
        let mut p = kind.defaults();
        if kind == FxKind::Press {
            // 5 ms of look-ahead, nothing compressed.
            p[0] = 0.0;
            p[7] = 5.0;
        }
        if kind == FxKind::Calm {
            p[0] = 0.0;
        }
        let fx = Fx::new(kind, p, false, sr);
        let late = fx.latency_samples();
        assert!(late > 0, "{kind:?}");
        let mut fx = fx;
        let mut first = None;
        for i in 0..4096 {
            let x = if i == 0 { 0.25 } else { 0.0 };
            let (l, _) = fx.process(x, x);
            if first.is_none() && l.abs() > 0.05 {
                first = Some(i);
            }
        }
        assert_eq!(first, Some(late), "{kind:?}");
    }
}

#[test]
fn prism_only_delays_the_sound_while_tame_is_on() {
    let mut p = FxKind::Prism.defaults();
    assert_eq!(FxKind::Prism.latency_for(&p, 48_000.0), 0);
    p[super::prism::TAME] = 0.5;
    assert!(FxKind::Prism.latency_for(&p, 48_000.0) > 0);
}

#[test]
fn a_dynamic_band_moves_with_the_level() {
    let sr = 48_000.0;
    let mut p = FxKind::Prism.defaults();
    // Band 3 (800 Hz) cuts by up to 12 dB above -30 dB.
    p[10] = 800.0;
    p[13] = -12.0;
    p[14] = -30.0;
    let mut fx = Fx::new(FxKind::Prism, p, false, sr);
    let tone = |i: usize| (std::f32::consts::TAU * 800.0 * i as f32 / sr).sin() * 0.8;
    let mut out = 0.0;
    let mut inp = 0.0;
    for i in 0..24_000 {
        let x = tone(i);
        let (y, _) = fx.process(x, x);
        if i > 12_000 {
            out += y * y;
            inp += x * x;
        }
    }
    assert!(out < inp * 0.5, "a loud tone at the band is cut: {out} vs {inp}");
}

#[test]
fn a_tapped_chain_hands_its_window_the_input_and_output() {
    let (tx, mut rx) = rtrb::RingBuffer::new(64);
    let mut chain = super::Chain::new(vec![Fx::new(FxKind::Clip, FxKind::Clip.defaults(), false, 48_000.0)]);
    chain.tap = Some(super::FxTap {
        slot: 0,
        out: Box::new(tx),
    });
    for _ in 0..8 {
        chain.process(0.5, 0.5);
    }
    assert_eq!(rx.slots(), 24);
    assert_eq!(rx.pop().ok(), Some(0.5));
}
