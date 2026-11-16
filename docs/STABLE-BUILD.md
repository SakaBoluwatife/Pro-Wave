# Ovrtone 1.0 stable: what goes in

The owner's list for the first stable build (after 1.0 Beta 1). Nothing here is built yet. Do not make the stable installer until the owner says so.

## 1. Safe project files

**Status (2026-11-15): built.** `plugin_trust.rs`: plugins load only from the scanned list (id, then path); a plugin installed elsewhere loads from there; files in project media are refused; missing ones retry after a Rescan. Tracks and buses.

A `.ovrtone` file names each plugin by file path, and Ovrtone loads that path as written (`project_io.rs` `plugin_from_doc`, then `plugin_rack.rs` `load_plugin_safely`). A project from someone else can therefore make Ovrtone load any plugin file, including one packed in the project's own media.

- Load a plugin only if its file is in the user's scanned plugin list (match by id, then path). Otherwise show it as missing ("not in your plugins: install it, then Rescan") and keep its settings for when it is installed.
- Never load a plugin file from inside a project's media folder.
- Do the same for bus plugins (`bus_plugins.rs`, `load_for_bus`).

## 2. Several inputs at once (multi-input interfaces)

**Status (2026-11-15): built.** Several armed tracks already recorded their own inputs together, with per-track input meters. Added: Name Inputs in the Input slot menu (saved per device in `input-names`). Driver channel names: cpal does not give them.

Today a track picks one input (`InputSource::Mono(n)` or `Stereo(n)` in `ovrtone-engine/src/capture.rs`).

- Check, then make sure, that several armed tracks each record their own input at the same time (a drummer on 8 mics, a band live).
- Input names from the interface's driver in the menu, and user labels ("Kick In", "Vox Mic").
- An input monitor per track that follows its own input.
- Record-arm several tracks at once, with the same count-in and punch for all.

## 3. Several MIDI devices at once

**Status (2026-11-15): built.** Inspector MIDI In Port and MIDI In Channel work and are saved. When an armed track listens to its own device or channel, every armed track plays and records only what it hears, each with its own recorder (`midi_routing.rs`).

Today every MIDI input port is opened and their notes are merged (`midi_in.rs`), so a keyboard and a drum pad both play the same track.

- Each instrument track chooses its MIDI input: All, or one device (and channel), as Logic's MIDI input per track.
- Keys on one track and a drum pad on another, played and recorded together.
- The device a note came from is kept with it, so a track that listens to one device ignores the others.

## 4. Controllers: MIDI CC and Mackie Control

**Status (2026-11-16): Mackie Control built.** `control_surface.rs`, Settings > General > Control surface: faders (motorised ones follow, not under a touching hand), V-Pots for pan, Rec, Solo, Mute and Select with lights, bank and channel buttons, transport and jog, track names on the display, meters. Its port plays no notes. Left: HUI, sends on the V-Pots, an assignment page for single knobs and faders.

MIDI learn exists (`midi_learn.rs`) for single controls.

- Controller assignments: a page of what each knob and fader does, saved per controller and per project.
- Mackie Control (MCU) and HUI: 8 faders per bank, bank and channel buttons, transport, jog, select, mute, solo, rec, the V-Pots for pan and sends, the display and meters sent back to the controller. Covers Mackie, Behringer X-Touch, iCON, PreSonus FaderPort in MCU mode and most keyboards with a DAW mode.
- Motorized faders follow automation.

## 5. Waves plugins: Ovrtone stops responding

**Status (2026-11-15): built.** Plugins in their own process load without blocking the window (`PendingLoad` in `bridge.rs`); after 2 minutes the status line says the plugin may be waiting on its own window; removing it from the slot stops the wait; the helper may bring its windows to the front; the launch screen stops staying on top once plugins load; the plugin load screen can be clicked away after 10 seconds. Since: no Loading label (owner: other DAWs have none); one plugin process is started ahead of time so the next plugin file opens without waiting for one; a project's plugins in their own processes load side by side; a CLAP file stays loaded for the session once opened, as `.vst3` files already did, so its next insert, in any project, is quick. Plugins load inside Ovrtone (quickest); one that closed Ovrtone while loading runs in its own process from then on, by itself (as Reaper and Bitwig do), instead of being blocked. Left: in-process loads still run on the UI thread, the Lemon ideas below.

Seen with Clarity Vx Pro, with "Run plugins in their own process" on: Windows says Ovrtone stopped responding. Cause found: `PluginRack::sync` (`plugin_rack.rs`) loads plugins on the UI thread, and an isolated load waits up to `LOAD_WAIT` (15 minutes, `bridge.rs`) for the helper. A slow plugin (Clarity loads an AI model) or one waiting on a hidden licence dialog freezes the window.

- Load plugins on a background thread; the slot shows "Loading <name>..." with Cancel, and the engine gets the plugin when it is ready.
- After about 2 minutes, ask whether to keep waiting instead of waiting 15 minutes silently.
- Bring a plugin's own dialogs (licence, activation) in the helper process to the front.

From Lemon Studio (behaviour reference; port the ideas, not its UI code):

- Keep ticking during a plugin's modal loop (`os-app/src/stall.rs`): a watchdog wakes the app when a plugin's menu, licence or file dialog holds the UI thread, so the playhead, meters and window keep going.
- In-process crash containment (`os-host/src/guard.rs`, `guard.c`): every call into a plugin under Windows structured exception handling; a crash marks the plugin file crashed, stops all its instances and cuts its windows off. No extra latency, unlike the helper process. Heap corruption, stack overflows and crashes on the plugin's own threads still end the process.
- Project loading spread over frames with a progress panel (`os-app/src/loading.rs`): samples decoded on worker threads, plugins started a few per frame.

The rest, if it still crashes:

To find out: which Waves plugin, and when (scan, load, opening its window, or playing). Waves VST3 ships as one shell file (WaveShell) holding many plugins, and needs Waves Central licences.

- Read the crash report and Ovrtone's log for the crash.
- Load each plugin of a shell by its own class id; scan shells in their own process with a longer time limit.
- If a plugin still crashes, run it in its own process automatically (Settings > Run plugins in their own process, per plugin).

## 6. Dropping samples (owner's notes, 2026-11-15)

**Status (2026-11-15): built.** Audio let go on the Playlist is audio there; let go on its edge (a blue band shows it) it asks; Browser right-click has Use it as... (at the playhead); Alt still asks.

Replaces the drop rule in `sample_place.rs` (hits to the Channel Rack, stems and loops to the Playlist):

- Any audio dragged onto the Playlist becomes audio there, whatever its length.
- Dragged onto the Playlist's border (the edge of the track area) it asks what to use it for: the old choices (Channel Rack channel, step pattern, play in the Piano Roll, on this track).
- Right-click a sound in the Browser for the same choices: send to the Channel Rack, add as audio at the playhead, open in the sampler, and so on.

## 7. Updates between builds

**Status (2026-11-15): script written.** `dist/windows/patch.iss`: same AppId, installs only `ovrtone.exe` and the documents into the folder the full installer used (from Windows' uninstall entry), refuses to run where Ovrtone is not installed, updates the version Windows shows. Compiles (checked with `ISCC /O-`); no patch made yet.

- From now on, changes ship as an update patch, not a full installer: a small Inno Setup installer with the same AppId that replaces only what changed (usually `ovrtone.exe` and the docs) in an existing install, and refuses to run where Ovrtone is not installed.
- The full installer (with the Stem Splitter files and the VC++ runtime) is made once, for the final build.

## 8. Capture Recording for audio, longer and wider

**Status (2026-11-15): partly built.** Settings > General: Capture Recording reaches back 1, 5 or 10 minutes; over a minute is kept in a file on disk by the writer thread (`DiskHistory` in `capture.rs`, at most 4 GB). Left: capture while stopped, capture from inputs no track is armed on.

Capture Recording (Shift+R) already works for audio: the last 60 seconds of the inputs are kept in memory (`ovrtone-engine/src/capture.rs`, `HISTORY_SECONDS`), and the last playback is cut out onto the armed or monitoring tracks (`audio_rec.rs` `capture_played`).

- A setting for how far back: 1, 5 or 10 minutes, kept in a temporary file on disk rather than in memory (about 10 MB a minute per stereo input).
- Capture what was played while stopped, placed at the playhead.
- Capture from any input, not only armed tracks.

## 9. "Coming from" another DAW (owner's idea, 2026-11-15)

**Status (2026-11-15): partly built.** A first-launch window, also in Settings > General: Logic, FL Studio, Ableton Live, Pro Tools. It sets the key commands (an Ableton Live set is new) and the starting layout. Left: Pro Tools edit modes and Tab to transient, a clip launcher, tooltips with the other DAW's words.

Not a copy of another DAW's look (their artwork and trade dress stay theirs), but its habits:

- A first-launch choice, "I'm coming from: Logic, FL Studio, Ableton Live, Pro Tools", also in Settings. It sets the key commands, the starting layout and a few mouse habits.
- Key command sets: Logic, FL Studio and Pro Tools exist (`key_commands.rs` `KeyPreset`). Add Ableton Live.
- Layouts from screensets (`screensets.rs`): FL (Channel Rack beside the Playlist), Ableton (a clip launcher grid and the device chain at the bottom), Pro Tools (Edit and Mix windows, the Smart Tool, Shuffle, Slip, Spot and Grid edit modes, Tab to transient).
- An Ableton-style clip launcher (Session view) is a feature of its own, as Logic's Live Loops: plan it on its own if wanted.
- Tooltips can name the other DAW's word for a thing (Insert = Mixer slot, Bus = Return track).

## 10. Audio and comping that look and feel like Logic

Logic's behaviour, our own artwork.

- Waveforms: sharp at every zoom, filled in the region's colour on a darker body, a centre line, clipped peaks marked red, a waveform zoom that changes only the view.
- Regions: a name strip, fades drawn as curves with corner handles on hover, the gain line on the region, loop repeats as ghosted copies, a clear outline when selected.
- Take folders: closed shows the comp and the take number; open shows a lane per take, the parts in use lit and the rest greyed; swiping paints smoothly and shows the crossfade at each join; take names and colours; Flatten and New Comp. Overlapping crossfades at comp joins (feature map, section 0).
- Flex: transient markers on the waveform, smooth marker dragging.
- Feel: eased zoom and scroll, a smooth playhead, hover highlights, one set of corner radii and spacing, and the ~300 one-off colours moved into `theme.rs`.

## 11. Also to account for

1. Licensing before a public release: Ovrtone is a fork of GPL Pro Wave. Publish the source, get the authors' permission, or rewrite their parts (see the licensing note in memory).
2. Session exchange: AAF and OMF import, and the open DAWproject format.
3. The audio interface unplugged or asleep: pause cleanly and come back when it returns. **Built (2026-11-15):** the engine notices the device going (`device_lost`), Ovrtone stops, says so, and starts again on the same device when it is back, looking every 2 seconds.
4. Old projects keep opening: a set of saved test projects opened on every build.
5. High-DPI scaling and windows on a second monitor.
6. Code signing for the installer and the program.
7. A short first-run tour, and the manual one key away.

## 12. Scale highlighting in the Piano Roll

**Status (2026-11-15): built.** Rows follow the project key unless Scale Quantize has its own scale; root rows stronger; marks on the piano keys; Stick to Scale; Fold to Scale; the Channel Rack's small piano roll shows the key's rows. Left: the step pitch box marking scale notes.

Today the rows of a scale are tinted only once a scale is picked for Scale Quantize (`piano_roll.rs`, `scale_root`, `MODES`).

- Follows the project's key (the key picker on the control bar) by default, with its own choice of root and scale in the Piano Roll.
- The root note's rows marked more strongly than the other scale notes; the piano keys tinted too.
- Fold to scale: show only the rows in the scale.
- Stick to scale: new and moved notes land on scale notes.
- The Channel Rack's small piano roll and the step pitch follow the same scale.

## 13. MP3 quality, in and out

**Status (2026-11-15): built and tested.** The round-trip test (length and line-up within a frame) found the end of every MP3 cut short by the encoder's flush; fixed. Gapless import (symphonia) and export (LAME info frame); a mix over full scale is turned down instead of clipped; above 48 kHz Ovrtone's own resampler; Quality in the Bounce window: 320, VBR V0, 256, 192, 128.

Export today (`audio_encode.rs` `mp3`): 320 kbps CBR, LAME quality Best, samples hard-clamped to full scale, rates above 48 kHz resampled by LAME. Import (`ovrtone-engine/src/disk.rs`): symphonia with `FormatOptions::default()`, so gapless trimming is off.

- Import gapless: turn on symphonia's gapless option so the encoder delay and end padding are trimmed. Imported MP3 loops then start on the beat and loop without a gap.
- Export gapless: write the LAME/Xing info frame at the start of the file (delay and padding), so players and other DAWs trim it too. Check what `mp3lame-encoder` writes today.
- No hard clipping: a true-peak limit (about -1 dBTP) before encoding, or a warning when the mix goes over, instead of `clamp(-1.0, 1.0)`. MP3 decoding overshoots peaks that are near full scale.
- Resample above 48 kHz with Ovrtone's own resampler (the one used for bounces), not LAME's.
- A choice in the Bounce window: 320 kbps CBR (default, beat stores), VBR V0, and 128/192/256 kbps for previews.
- Test: encode, decode, line up and compare with the WAV (level, length and start within a few samples).

## 13b. Beat stores of the user's choosing

**Status (2026-11-15): built.** File > Export > For Beat Store opens a window: the stores to open after the export (Trakroom to begin with), add a store by name and address, remove one. Kept in `beat-stores` in the config folder; only http and https addresses open.

## 14. From other DAWs, made better

None of these are stock instruments or effects. Their habits, not their look.

- **Command palette** (Ctrl+K, as Reaper's actions and Ableton's search): every action, plugin, preset, track and setting in one search box, recent first, each with its key command.
- **Modulators** (Bitwig, FL's Formula Controller): LFO, envelope, envelope follower and sidechain-to-parameter on any parameter, a hosted plugin's too. Drag from the modulator to a knob; the amount shows as a ring on the knob.
- **Plugin racks** (Ableton's Racks, FL's Patcher): parallel branches, wet and dry per branch, 8 macros tied to Smart Controls, saved as presets.
- **Clip FX** (Studio One's Event FX, Reaper's item FX): plugins on one region, and rendering a plugin onto a clip offline (Pro Tools' AudioSuite).
- **CPU savers:** plugins that sleep on silence, plugins processed ahead of time on a buffer (Reaper's anticipative FX), a CPU meter per plugin.
- **Reference A/B** (Cubase's Control Room): a reference track matched in loudness, mono and side checks, solo one band. A utility, not an effect.
- **Arranger track** (Studio One; Logic's is greyed in Ovrtone): move or copy whole song sections, Playlist and Channel Rack patterns together.
- **Chord track** (Studio One, Cubase): regions follow the chords; feeds the scale highlighting of section 12.
- **Per-note chance, conditions and repeats** (Bitwig, FL, Ableton), on Channel Rack steps too.
- **Audio to MIDI** (Ableton): drums, melody and chords, after the Stem Splitter if wanted.
- **Scratch pads** (Studio One): try an arrangement without touching the song.
- **Several projects open as tabs** (Reaper, Studio One), and an undo history window.
- **Voice tag on MP3 exports** for beat stores, one click in For Beat Store.

## 15. A premium feel

**Status (2026-11-16):** the Project Chooser has tiles that lift on hover, rounded pictures, when each project was last saved, a fade-in, arrow keys and a welcome line; send knobs take the same gestures; an empty project says how to begin. Before that: **Status (2026-11-15): started.** Soft shadows under windows and menus, controls that grow a hair on hover, 0.12 s motion, quicker tooltips; one set of gestures on the mixer fader, pan knob, effect knobs and sampler knobs (`widgets::value_gesture`: Shift or Ctrl for fine, Alt-click or double-click to reset, wheel to nudge); each tool's picture as the pointer in the Tracks area and the Piano Roll. Left: send knobs and other sliders, the start screen, empty-place hints, plugin windows remembering place per project, the colours left outside `theme.rs`.

Adds to section 10.

- One set of controls (button, knob, fader, menu, field) used on every screen, and every value edited the same way: double-click to type, Alt+click to reset, Ctrl+drag for fine moves, wheel to change.
- One font with tabular numbers on meters and the LCD (Inter, OFL), an 8 px spacing grid, one set of corner radii.
- Motion: 100 to 150 ms transitions on hover, press and panels; eased zoom. Redraw only what changed, which also saves CPU.
- Feedback: a ghost image while dragging, snap lines, a cursor for each tool, messages with an Undo button instead of the status line, tooltips that name the key command.
- A start screen with recent projects and templates; hints in empty places ("Drop audio here").
- Plugin windows: pin on top, the track name in the title, positions remembered per project.
- A UI scale slider, an accent colour of the user's choice, colour-blind-safe track colours, sharp at every DPI.
- The window up in under 2 seconds; big projects load over several frames with a progress panel (section 5).

## 16. Closed source: what to rewrite

Not legal advice; get a lawyer before a public release. Builds already given out (1.0 Beta 1) stay GPL. See section 11, item 1.

### GPL code in the program: get written permission or rewrite

1. **Pro Wave (Fábox, GPL-3.0).** On 2026-11-15, `git blame` gave 53,248 of 134,917 Rust lines (39%) to Fábox: about 37,000 in `ovrtone-app`, 14,000 in `ovrtone-engine`, all of `ovrtone-project` (903) and 1,000 in `xtask`. Lines changed on top of theirs still derive from them, so the real share is higher.
   - Engine: `core.rs` (mix graph, transport), `disk.rs` (streaming), `tempo_map.rs`, `capture.rs` (recording), `flex.rs` (stretch and pitch), the `.ovrtone` format crate. `fx.rs` goes with Task 3.
   - App, most lines first: `tracks_area`, `model`, `mixer`, `project_io`, `updater`, `pattern`, `score`, `piano_roll`, `step_editor`, `midi_fx`, `engine_sync`, `main`, `menus`, `midi_file`, `flex_edit`, `smart_tempo`, `inspector`, `automation`, `recorder`, `transport`.
2. **Lemon Studio (ItsAlphaNeon, GPL-3.0).** `parallel.rs` (588 lines, the worker pool, copied whole), `hq_stretch.rs` (200 lines, adapted), the algorithms in `note_edit.rs` (Quick Legato, Chop, Glue and others). The Lemon ideas in section 5 are written from their behaviour, not their code.

Order: ask Fábox and ItsAlphaNeon first. If not, rewrite the Lemon parts (small), then the project format, then the engine files. The rewrite is done from a written description of behaviour, not from the old code.

### Libraries: keep, with conditions

- **ASIO SDK:** used under its GPL-3 option today. Closed source needs Steinberg's proprietary ASIO licence agreement, signed by Steinberg.
- **LAME** (`mp3lame-sys`, LGPL-3.0, built in): ship it as a separate, replaceable DLL.
- **MPL-2.0** (symphonia, audio_thread_priority, smartstring): fine while their files are not changed.
- **self_cell, r-efi:** take their MIT or Apache option.
- **ARA (Apache-2.0), CLAP and clack, ONNX Runtime:** fine; keep the notices.
- **To check:** the DrumSep and HTDemucs model licences, the VST3 SDK licence and Steinberg's terms for the VST logo.

### Then

Change `LICENSE`, every crate's `license`, `deny.toml`, the About box (`quick_dialogs.rs`), `native_menu.rs`. Closed work goes to the `private` remote only, not the public fork.
