//! A plugin in a process of its own: its sound comes back through the bridge, and when the process
//! is killed this one lives on and the sound passes by. Uses the built-in sampler, so no plugin
//! has to be installed, and the real program as the helper.
#![cfg(windows)]
#![allow(clippy::unwrap_used)]

use ovrtone_engine::{PluginEvent, PluginEventKind, PluginTransport, PLUGIN_BLOCK};
use ovrtone_host::{load_isolated, set_host_exe};
use std::path::Path;
use std::time::{Duration, Instant};

/// A second of a loud square wave, as a 16-bit WAV file.
fn write_wav(path: &Path) {
    let rate = 48_000u32;
    let samples: Vec<i16> = (0..rate)
        .map(|i| if (i / 50) % 2 == 0 { 20_000 } else { -20_000 })
        .collect();
    let data_len = samples.len() as u32 * 2;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn a_plugin_sounds_through_the_bridge_and_its_crash_leaves_this_process_alive() {
    set_host_exe(env!("CARGO_BIN_EXE_ovrtone").into());
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("hit.wav");
    write_wav(&wav);
    let (mut loaded, mut audio) = load_isolated(&wav, "sampler", 48_000, false).unwrap();
    assert!(!loaded.crashed());
    let transport = PluginTransport {
        playing: false,
        beat: 0.0,
        seconds: 0.0,
        tempo: 120.0,
        signature: (4, 4),
        bar_start: 0.0,
        bar_number: 0,
        cycle: None,
    };
    let silence = vec![0.0f32; PLUGIN_BLOCK];
    let (mut left, mut right) = (vec![0.0f32; PLUGIN_BLOCK], vec![0.0f32; PLUGIN_BLOCK]);
    let note = [PluginEvent {
        frame: 0,
        kind: PluginEventKind::NoteOn {
            key: 60,
            velocity: 1.0,
        },
    }];
    let mut loudest = 0f32;
    for block in 0..200 {
        let events: &[PluginEvent] = if block == 0 { &note } else { &[] };
        audio.process(
            [&silence, &silence],
            [&mut left, &mut right],
            events,
            &transport,
        );
        loudest = left.iter().fold(loudest, |top, s| top.max(s.abs()));
    }
    assert!(loudest > 0.05, "the note came back: {loudest}");

    // The process dies, as a crash would end it.
    let pid = loaded.process_id().unwrap().to_string();
    let killed = std::process::Command::new("taskkill")
        .args(["/F", "/PID", &pid])
        .output()
        .unwrap();
    assert!(killed.status.success());
    let began = Instant::now();
    while !loaded.crashed() {
        assert!(
            began.elapsed() < Duration::from_secs(5),
            "it was not noticed"
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    // No hang, and the sound passes by.
    let input = vec![0.25f32; PLUGIN_BLOCK];
    let began = Instant::now();
    audio.process([&input, &input], [&mut left, &mut right], &[], &transport);
    assert!(began.elapsed() < Duration::from_millis(5));
    assert_eq!(left, input);
    assert!(loaded.save_state().is_err(), "it says it cannot, at once");

    // A new load starts a new process.
    let (again, _audio) = load_isolated(&wav, "sampler", 48_000, false).unwrap();
    assert_ne!(again.process_id().unwrap().to_string(), pid);
    assert!(!again.crashed());
}

/// Time per block with a real plugin, inside this process and through the bridge. Set
/// `OVRTONE_TEST_PLUGIN` to `file|id` and `OVRTONE_TEST_BLOCK` to the block size.
#[test]
#[ignore = "loads a plugin installed on this computer and prints timings"]
fn timing_of_a_real_plugin() {
    set_host_exe(env!("CARGO_BIN_EXE_ovrtone").into());
    let chosen = std::env::var("OVRTONE_TEST_PLUGIN").unwrap();
    let (file, id) = chosen.split_once('|').unwrap();
    let block: usize = std::env::var("OVRTONE_TEST_BLOCK")
        .unwrap()
        .parse()
        .unwrap();
    ovrtone_engine::set_plugin_block(block);
    let transport = PluginTransport {
        playing: true,
        beat: 0.0,
        seconds: 0.0,
        tempo: 120.0,
        signature: (4, 4),
        bar_start: 0.0,
        bar_number: 0,
        cycle: None,
    };
    let input: Vec<f32> = (0..block).map(|i| (i as f32 * 0.05).sin() * 0.25).collect();
    let (mut left, mut right) = (vec![0.0f32; block], vec![0.0f32; block]);
    let period = Duration::from_secs_f64(block as f64 / 48_000.0);
    let mut measure = |name: &str, audio: &mut Box<dyn ovrtone_engine::PluginProcessor>| {
        let blocks = 48_000 * 3 / block;
        let (mut total, mut worst, mut loudest) = (Duration::ZERO, Duration::ZERO, 0f32);
        for _ in 0..blocks {
            let began = Instant::now();
            audio.process([&input, &input], [&mut left, &mut right], &[], &transport);
            let took = began.elapsed();
            total += took;
            worst = worst.max(took);
            loudest = left.iter().fold(loudest, |top, s| top.max(s.abs()));
            std::thread::sleep(period.saturating_sub(took));
        }
        println!(
            "{name}: block {block}, mean {:?}, worst {:?}, peak {loudest}",
            total / blocks as u32,
            worst
        );
    };
    // `OVRTONE_TEST_BRIDGE_ONLY`: not in this process as well, for a plugin that may mind that.
    let mut kept = None;
    if std::env::var_os("OVRTONE_TEST_BRIDGE_ONLY").is_none() {
        let began = Instant::now();
        let (loaded, mut direct) =
            ovrtone_host::load_plugin(Path::new(file), id, 48_000, false).unwrap();
        println!("in this process: loaded in {:?}", began.elapsed());
        measure("in this process", &mut direct);
        kept = Some((loaded, direct));
    }
    let began = Instant::now();
    let (mut bridged_ui, mut bridged) = load_isolated(Path::new(file), id, 48_000, false).unwrap();
    println!("bridge: loaded in {:?}", began.elapsed());
    measure("bridge", &mut bridged);
    let began = Instant::now();
    let state = bridged_ui.save_state().unwrap();
    println!(
        "bridge: state {} bytes in {:?}",
        state.map_or(0, |s| s.len()),
        began.elapsed()
    );
    println!("bridge: params {}", bridged_ui.params().len());
    bridged_ui.open_editor("Bridge test", None, None).unwrap();
    println!("bridge: editor open {}", bridged_ui.editor_is_open());
    std::thread::sleep(Duration::from_secs(2));
    bridged_ui.close_editor();
    std::mem::forget(kept);
    std::process::exit(0);
}
