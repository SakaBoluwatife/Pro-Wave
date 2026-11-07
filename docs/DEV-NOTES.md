# Ovrtone development notes

The running record of the autonomous development pass: what was done, what is left, known issues and decisions. `docs/LOGIC-WIREFRAME.md` stays the feature map.

Test results here are automated unless a line says otherwise. Nothing in this file has been checked by ear or by hand unless it says so.

## Decisions

- **Reuse before writing.** The owner's rule: no custom work where something proven exists. Plugins run on Lemon Studio's worker pool (`ovrtone-engine/src/parallel.rs`, GPL-3.0-or-later, copyright kept), not a new one.
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
- **Stage 5, mixer:** drag plugins between slots, strips and the output; routing matrix; a test that renders the routing and checks what comes out; Low Latency mode; channel strip settings files; Copy, Paste and Reset Channel Strip now include plugins (they left them behind before).
- **Stage 6, plugins:** moves in plugin windows recorded to lanes; VST3 windows follow lanes; plugin presets as state files; Touch follows the hand on a VST3 control (`beginEdit` and `endEdit`).
- **Stage 7, export:** float headroom kept; sample rate conversion; selected regions; Mastering window with BS.1770 loudness, true peak, spectrum and a reference file (checked against EBU tones).
- **Stage 8, Logic features:** Undo History; key command editor; Project Settings; Smart Controls. The Tuner button did nothing and was removed.
- **Stage 9, look:** `theme.rs` names the shared colours, states, spacing and type, and the screens' panel, slot, accent and status colours now come from it. Library, Browser, Notes, Lists and the bottom panel can be dragged wider or taller, and the mixer starts taller, so faders have more room.
- **Stage 10, Windows:** sample rate in Settings.

## Greyed menus

**Focus for now, from the owner:** mixing and beat making.

Built: window commands, Close, Score, Step Sequencer and Smart Tempo views, global track toggles, Show Track Header, Ruler, Colors and Grid, Configure Track Header, Assign Track Icon, Record Enable and Toggle, automation show, create, delete and hide, region automation with Convert Automation, Group and Edit Groups, Import Audio File and Other Projects, Rename Project, Consolidate, Clean Up, Save as Template, Tracks in Place, the Functions menu, Paste Replace, Note Colors, Pre-Fader Metering, all Tracks-area tools and a Ctrl tool, Signal Flow, Select Audio Channel Strips, Flex Mode and the rows under More in the Inspector, Freeze, Create Mastering Track, Show Master Track, Plugin Manager.

Choices made:

- **Region automation** is merged over the track's lane when lanes go to the engine, so the audio thread does nothing new. Writing with Touch, Latch or Write still goes to the track's lane; a region's lane wins over its stretch.
- **Freeze** renders the strip before its fader and lets its plugins and effects go. Fader, pan and sends stay live. It is not saved: a project opens with its tracks live. Regions can still be edited while frozen and have no effect until it thaws.
- **Create Mastering Track** is an Aux every strip going to Stereo Out now goes through. **Show Master Track** opens its automation.
- **Consolidate** saves: a saved project already holds a copy of every audio file it uses. **Clean Up** offers to delete freeze files no track here plays.
- **Q-Velocity** moves notes towards the region's average velocity, as there are no groove templates to take it from.
- **Configure Track Header** choices and **Select Audio Channel Strips** last for the run; they are not saved.

### On hold

Apple's, with no Windows alternative: Song to Mobile Device, Open in Sound Library Manager. By the owner: Movie, Video. Nothing behind them: Session Player, Performers, Tuning. They stay greyed. `docs/LOGIC-WIREFRAME.md` section 16 has the rest of what is waiting.

### Windows alternatives added

- **Show in File Manager** opens File Explorer.

### Stem Splitter

One version is kept: `stem_splitter.rs`, with the CPU helper, as the owner chose.

## Notes from the owner's testing, 2026-11-05

- **Stem Splitter:** bundled, CPU helper only for now, to optimize later. The owner tests it; no split has finished on this machine yet. The model's licence is unresolved: see `dist/windows/stems-README.md`.
- **Recording:** the owner reports left-only, distorted-sounding takes with Input 1 on an Audient iD4. Their saved input was WASAPI (`Analogue 1/2`) while output was the Audient ASIO driver. A test shows a mono input reads back the same on both sides. Later reported fixed by the owner after checks on their side; what fixed it is not recorded, and no code was changed for it specifically.
- **Drivers:** the owner wants WASAPI, ASIO and Audient's own driver available. Audient's driver is the entry `Audient USB Audio ASIO Driver`, among the ASIO drivers.
- **Look:** Logic's layout and feel with our own artwork, with frosted plates on floating things. Next: stop button, Ctrl+K keyboard icon.
- **Order after this:** plugin process isolation, then the licence review before any release. Installer: Inno Setup.
- **Skipped for now, by the owner:** Session Player, Performers, Song to Mobile Device, Sound Library Manager, Logic Remote, GarageBand, iPad, AirDrop, Music app and Voice Memos sharing, surround and binaural panning, Apple Loops library features, Live Loops, full score engraving. **Open for later:** Dolby Atmos.

## Remaining and known issues

- **Automation:** on a CLAP plugin, Touch lets go a beat after the last move, not at the mouse release: CLAP's gesture events are not read yet. CLAP windows are expected to follow lanes by themselves; not tried.
- **Audio takes:** only from Cycle recording. Recording over an existing region does not make a folder.
- **Autopunch:** recording stops at the punch out, one punch a pass. The locators are set from the selection or Cycle, not by dragging.
- **ASIO control panel:** neither cpal nor `asio-sys` offers it. Needs a small binding of its own.
- **Plugin isolation:** not built. Plugins run inside the program, unsandboxed. A crash on load is blocked until Rescan; a crash while playing ends the program.
- **Not built:** Tuner, Step Editor controller lanes, Live Loops, dockable panels, independent input and output devices, WAV past 4 GB (RF64). The Inspector panel stays a fixed width because its strips are. Smart Control knob choices are not saved in the project. About 300 one-off colours remain in the screens.
- **Tests that use the wall clock** (`capture`, `disk`, flex `huge_edits`) fail when the machine sleeps or is very busy during the run. Rerun them alone. Stopping a recording now reads everything delivered before the stop first (`capture.rs`), which also removed a cause of those failures.
- **Not built:** Create New Output Channel Strip: the engine mixes to one stereo output.
- **Not run:** the installer (Inno Setup is not installed here), long playback and recording sessions, multi-monitor.
- **Not verified by ear or by hand:** all of the above, including every window added in this pass.
