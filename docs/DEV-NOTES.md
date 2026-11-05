# Pro Wave development notes

The running record of the autonomous development pass: what was done, what is left, known issues and decisions. `docs/LOGIC-WIREFRAME.md` stays the feature map.

Test results here are automated unless a line says otherwise. Nothing in this file has been checked by ear or by hand unless it says so.

## Decisions

- **Reuse before writing.** The owner's rule: no custom work where something proven exists. Plugins run on Lemon Studio's worker pool (`prowave-engine/src/parallel.rs`, GPL-3.0-or-later, copyright kept), not a new one.
- **Plugins in parallel.** A plugin's output reaches the mix only from the next block, so within a block no plugin depends on another, even in one chain. All of a tick's plugin blocks run across the pool with no dependency graph edges. The sound is identical to one thread (tested).
- **The rest of the mix stays on the audio thread, sample by sample.** Running strips in parallel as Lemon does would mean rewriting `core.rs` around blocks. Worth doing only if sessions with many built-in effects or clips need it; the plugin load, which dominates real sessions, is already spread out.
- **Autosave** copies media on a background thread and never touches the project file. It writes per process, so two running copies do not take each other's recovery file for a crash.
- **Exports render in floating point first**, then one conversion to the file's rate, depth and format. Sound cards still get a clamp; files do not unless they are fixed point.
- **Audio takes are windows into one recorded file**, so comping is non-destructive and a take costs no disk.

## Measurements

From `perf.rs`, ignored tests, release build with thin LTO, 8 logical cores, offline with no driver. Load is of one core's time for the callback. Before the multi-core work:

| Case | Load | p99 | Worst | Missed |
|---|---|---|---|---|
| 24 audio tracks, 256 frames | 5.4 % | 10.1 % | 14.8 % | 0 |
| 128 audio tracks, 256 frames | 36.5 % | 90.1 % | 109.4 % | 2 |
| 24 built-in instrument tracks, 256 frames | 54.2 % | 86.9 % | 100.2 % | 1 |
| 24 built-in instrument tracks, 96 kHz | 98.5 % | 177 % | 245.8 % | 765 |
| 8 x Pro-C 3, 256 frames | 18.7 % | 28.7 % | 31.0 % | 0 |
| 8 x Pro-DS, 256 frames | 22.8 % | 35.1 % | 39.2 % | 0 |

After the multi-core work, in one run with a compile going on the same machine, so the worst cases are not trustworthy: 8 x Pro-C 3 went from 18.7 % to about 10.6 % and 8 x Pro-DS from 22.8 % to about 12.5 %. **No clean measurement of worst cases has been made since**, nor of the clip loop change.

## Done, by stage

- **Stage 2, reliability:** crash recovery; any track can be deleted, and a project with no tracks takes every menu entry and key; regression tests for save and load of every part, older files, undo and redo. Two wall-clock test limits that failed under load are generous now.
- **Stage 3, performance:** plugins on several cores; the sample loop visits only clips that can sound; the mix ramp visits only the song's tracks; denormals flushed on the audio thread and workers.
- **Stage 4, recording and editing:** audio take folders and swipe comping from Cycle recording; Autopunch; track rename and colour; Go to Position.
- **Stage 5, mixer:** drag plugins between slots, strips and the output; routing matrix; a test that renders the routing and checks what comes out; Low Latency mode.
- **Stage 6, plugins:** moves in plugin windows recorded to lanes; VST3 windows follow lanes; plugin presets as state files.
- **Stage 7, export:** float headroom kept; sample rate conversion; selected regions; Mastering window with BS.1770 loudness, true peak, spectrum and a reference file (checked against EBU tones).
- **Stage 8, Logic features:** Undo History; key command editor; Project Settings. The Tuner button did nothing and was removed.
- **Stage 10, Windows:** sample rate in Settings.

## Remaining and known issues

- **Automation:** Touch on a plugin lets go a beat after the last move, not at the mouse release: `beginEdit` and `endEdit` are not used yet. CLAP windows are expected to follow by themselves; not tried.
- **Audio takes:** only from Cycle recording. Recording over an existing region does not make a folder.
- **Autopunch:** recording stops at the punch out, one punch a pass. The locators are set from the selection or Cycle, not by dragging.
- **ASIO control panel:** neither cpal nor `asio-sys` offers it. Needs a small binding of its own.
- **Plugin isolation:** not built. Plugins run inside the program, unsandboxed. A crash on load is blocked until Rescan; a crash while playing ends the program.
- **Not built:** Smart Controls, Tuner, Step Editor, Live Loops, central design system, resizable and dockable panels, WAV past 4 GB (RF64).
- **Not run:** the installer (Inno Setup is not installed here), long playback and recording sessions, multi-monitor.
- **Not verified by ear or by hand:** all of the above, including every window added in this pass.
