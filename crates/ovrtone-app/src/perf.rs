//! Measurements of the audio engine, as ignored tests, to be run in a release build:
//!
//! `cargo test --release -p ovrtone-app perf:: -- --ignored --nocapture --test-threads=1`
//!
//! Each prints how long the engine took per callback against the time it had: average load, the
//! 99th percentile and the worst callback, and how many missed their deadline. This runs as fast as
//! it can with no sound device, so it measures the engine alone, not a driver's scheduling.
use crate::engine_sync::EngineSync;
use crate::model::{Project, TrackKind};
use ovrtone_engine::{
    Command, Engine, EngineCore, PluginEvent, PluginProcessor, PluginSlot, PluginTransport,
};
use std::time::{Duration, Instant};

const SECONDS: f64 = 5.0;

struct Stats {
    load: f64,
    p99: f64,
    worst: f64,
    missed: usize,
}

fn report(name: &str, tracks: usize, rate: u32, block: usize, s: &Stats) {
    println!(
        "{name:<22} {tracks:>5} tracks {rate:>6} Hz {block:>5} frames: load {:>6.1} %  p99 {:>6.1} %  worst {:>6.1} %  missed {}",
        s.load * 100.0,
        s.p99 * 100.0,
        s.worst * 100.0,
        s.missed
    );
}

/// Runs `core` for [`SECONDS`] of audio.
fn measure(core: &mut EngineCore, rate: u32, block: usize) -> Stats {
    let mut out = vec![0.0f32; block * 2];
    let budget = block as f64 / f64::from(rate);
    let blocks = ((SECONDS * f64::from(rate)) as usize / block).max(1);
    let mut times = Vec::with_capacity(blocks);
    for _ in 0..blocks {
        let waited = Instant::now();
        while !core.clips_ready(block) && waited.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_micros(200));
        }
        let t0 = Instant::now();
        core.process(&mut out);
        times.push(t0.elapsed().as_secs_f64());
    }
    let total: f64 = times.iter().sum();
    let missed = times.iter().filter(|t| **t > budget).count();
    times.sort_by(f64::total_cmp);
    Stats {
        load: total / (blocks as f64 * budget),
        p99: times[(blocks * 99 / 100).min(blocks - 1)] / budget,
        worst: times[blocks - 1] / budget,
        missed,
    }
}

/// An engine with `p` loaded and playing from the start.
fn engine_for(p: &mut Project, rate: u32) -> (EngineSync, EngineCore) {
    let (engine, mut core) = Engine::offline(rate);
    let mut sync = EngineSync::new(engine, p);
    let mut settled = 0;
    for _ in 0..4096 {
        sync.sync(p);
        core.process(&mut []);
        settled = if sync.needs_retry() { 0 } else { settled + 1 };
        if settled >= 3 {
            break;
        }
    }
    assert!(sync.engine.send(Command::SetPosition { beat: 0.0 }));
    assert!(sync.engine.send(Command::Play));
    core.process(&mut []);
    (sync, core)
}

fn instrument_tracks(n: usize) -> Project {
    let mut p = crate::bench::dsp_project();
    let template = p.tracks[0].clone();
    p.tracks = (0..n).map(|_| template.clone()).collect();
    p.cycle = true;
    p.cycle_start = 0.0;
    p.cycle_end = 8.0;
    p
}

#[test]
#[ignore = "a measurement"]
fn built_in_instruments() {
    for (tracks, rate, block) in [
        (1, 48_000, 256),
        (24, 48_000, 256),
        (64, 48_000, 256),
        (128, 48_000, 256),
        (24, 44_100, 64),
        (24, 48_000, 64),
        (24, 48_000, 1024),
        (24, 96_000, 256),
    ] {
        let mut p = instrument_tracks(tracks);
        let (_sync, mut core) = engine_for(&mut p, rate);
        report(
            "built-in instruments",
            tracks,
            rate,
            block,
            &measure(&mut core, rate, block),
        );
    }
}

#[test]
#[ignore = "a measurement"]
fn streaming_audio_from_disk() {
    const RATE: u32 = 48_000;
    let dir = tempfile::tempdir().expect("temp");
    let wav = dir.path().join("clip.wav");
    let noise: Vec<f32> = (0..RATE as usize * 2 * 20)
        .map(|i| (i as f32 * 0.37).sin() * 0.2)
        .collect();
    ovrtone_engine::write_wav(&wav, &noise, RATE).expect("wav");
    let ctx = egui::Context::default();
    for (tracks, block) in [(1, 256), (24, 256), (64, 256), (128, 256), (24, 64)] {
        let mut p = Project::empty();
        p.tempo = 120.0;
        p.tracks.clear();
        for _ in 0..tracks {
            crate::import::import_audio_at(&mut p, &wav, &ctx, 1.0, None).expect("import");
        }
        let (_sync, mut core) = engine_for(&mut p, RATE);
        report(
            "audio from disk",
            tracks,
            RATE,
            block,
            &measure(&mut core, RATE, block),
        );
    }
}

/// A plugin that does nothing: what remains is the cost of hosting one.
struct Idle;

impl PluginProcessor for Idle {
    fn process(
        &mut self,
        input: [&[f32]; 2],
        output: [&mut [f32]; 2],
        _: &[PluginEvent],
        _: &PluginTransport,
    ) {
        let [l, r] = output;
        l.copy_from_slice(input[0]);
        r.copy_from_slice(input[1]);
    }

    fn latency_samples(&self) -> usize {
        0
    }
}

#[test]
#[ignore = "a measurement"]
fn the_cost_of_hosting_a_plugin() {
    const RATE: u32 = 48_000;
    for (tracks, per_track, block) in [(1, 1, 256), (24, 1, 256), (24, 4, 256), (128, 2, 256)] {
        let mut p = Project::empty();
        p.tracks.clear();
        for _ in 0..tracks {
            p.add_track(TrackKind::Audio);
        }
        let (mut sync, mut core) = engine_for(&mut p, RATE);
        for t in 0..tracks {
            for slot in 0..per_track {
                let mut plugin = Some(Box::new(PluginSlot::new(Box::new(Idle))));
                while let Some(next) = plugin.take() {
                    if !sync.engine.send(Command::SetPlugin {
                        track: t as u16,
                        slot: slot as u8,
                        plugin: Some(next),
                    }) {
                        core.process(&mut []);
                        plugin = Some(Box::new(PluginSlot::new(Box::new(Idle))));
                    }
                }
            }
            core.process(&mut []);
        }
        let name = format!("{per_track} idle plugins each");
        report(&name, tracks, RATE, block, &measure(&mut core, RATE, block));
    }
}

#[test]
#[ignore = "a measurement, and it loads the plugins installed on this computer"]
fn real_plugins() {
    const RATE: u32 = 48_000;
    let files = ovrtone_host::find_plugin_files(&[]);
    let wanted = ["Pro-C", "Pro-DS", "De-Esser"];
    let found: Vec<_> = files
        .iter()
        .filter(|f| wanted.iter().any(|w| f.display().to_string().contains(w)))
        .take(4)
        .collect();
    for file in found {
        let Some(info) = ovrtone_host::scan_plugin_file(file)
            .ok()
            .and_then(|mut v| (!v.is_empty()).then(|| v.remove(0)))
        else {
            continue;
        };
        for (copies, block) in [(1, 256), (8, 256), (8, 64)] {
            let mut p = Project::empty();
            p.tracks.clear();
            let t = p.add_track(TrackKind::Audio);
            for _ in 0..copies {
                crate::plugin_rack::add_plugin(&mut p, t, &info).expect("add");
            }
            let (_sync, mut core) = engine_for(&mut p, RATE);
            let name = format!("{} x{copies}", info.name);
            report(&name, 1, RATE, block, &measure(&mut core, RATE, block));
        }
    }
}
