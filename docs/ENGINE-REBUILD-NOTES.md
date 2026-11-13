# Notes for the engine rebuild

What the app (`crates/ovrtone-app`) now relies on from `crates/ovrtone-engine`, and engine work that
was left for the rebuild. Keep these working, or tell the app side what replaces them.

## Commands and types the app uses (added in the uncommitted work after commit 027630c)

- `TrackMix.trim_db`: input trim in dB, applied before the strip's effects (Auto Gain Staging
  writes it). Applied in `strip_sound` before the chain and in `mix_strips` for simple strips.
- `Command::SetFxTap { track, tap: Option<FxTap> }`: a copy of one effect's input (mid) and output
  (left, right), three floats a frame, for the effect window's live display. The tap moves to a
  new chain on `SetInserts`.
- `Fx::with_side`, `Chain::feed_sides`, `Chain::side_sources`: side chains for the stock effects
  (Press, Compressor, Gate, Drift, Shut, Sweep). `sort_strips` marks those sources.
- `Chain::set_tempo`, `Fx::set_tempo`: tempo for synced effects (Drift, Tape Delay, Sweep, Pulse).
- `PluginEventKind::NoteTune { key, semitones }` and `PluginProcessor::note_tuning()`, with the
  control numbers `NOTE_TUNE_KEY_CONTROL` (130), `NOTE_TUNE_FINE_CONTROL` (131) and
  `NOTE_TUNE_CONTROL` (132): per-note slides. The app sends both the per-note tuning and the
  whole-instrument bend; the engine plays one, by `note_tuning()`.
- `ObjectPlace.distance` and `SpatialRender::add_object`: an object's own headphone distance.
- `MAX_INSERTS` is 128 (was 32). The engine keeps a fixed slot array of that size per track
  (`plugins: Vec<[Option<Box<PluginSlot>>; MAX_INSERTS]>`). Truly unlimited inserts need that to
  become a growable list.
- `EngineState.track_rms` and `track_rms(track)`: post-fader RMS per track (both sides), eased to read like
  a 300 ms average. Summed where the peaks are, in `strip_sound` and `mix_strips`; the mixer draws
  it over the peak meters.
- `MAX_FX_PARAMS` is 32. The stock effects live in `fx.rs` and `fx/*.rs`; `FxKind::ALL` has 34.

## Left for the rebuild

1. **Per-note slides for VST3** (roadmap item 6, on hold): VST3 note expression
   (`kTuningTypeID`, note IDs on note-on). Today VST3 instruments ignore `NoteTune` and get the
   whole-instrument bend.
2. **Per-step pan done properly**: the Channel Rack sends a step's pan as controller 10 before
   the step's note, so it moves the whole channel. A per-note pan path (CLAP note expression
   `PAN`, VST3 note expression) would pan single notes.
3. **Unlimited inserts**: see `MAX_INSERTS` above.
4. **A second output channel strip** / more output routing: the engine mixes to one stereo
   output plus the spatial bus.
5. **Block processing of strips and stock effects**: they run a sample at a time today (CPU).
6. **Modulation** (LFOs, envelope followers on parameters): not started; it belongs next to
   automation in the engine.
7. **Batch 0.2.3 additions the rebuild must keep**:
   - `PRESSURE_CONTROL` (133) and `BEND_LANE_CONTROL` (134) in `send_control`, with
     `PluginEventKind::ChannelPressure` (CLAP MIDI 0xD0, VST3 `kAfterTouch`; the sampler ignores
     it). External MIDI tracks do not send channel pressure yet.
   - `FxKind::Freq`, the frequency shifter in `fx/freq.rs` (5 params, per-sample today).
   - `TrackMix.trim_db` (input trim before the chain) and the per-track RMS in `EngineState`.
   - Reference tracks are only muted in renders by the app; live playback does not bypass the
     master chain for them. A true reference path (post-master, no master processing) belongs in
     the engine.
8. **Hardware-adaptive engine, left after the 2026-11-11 passes**:
   - Render ahead (`ahead.rs`) covers the whole mix at once. A per-track split (live track at the
     device buffer, the rest ahead) would let a played-in instrument skip the ring; today notes
     played in switch it off for 3 s, and the first note waits for the ring (about 50 ms) to drain.
   - Render ahead is off for devices with more than two outputs (the extra outputs come from the
     callback's own block).
   - Vector kernels: the mix and Utility have one. The other stock effects now run a span at a
     time (`Chain::process_block`) but still per frame inside; each needs its own `run_block`.
   - Simple strips (no chain) still mix on the audio thread.
