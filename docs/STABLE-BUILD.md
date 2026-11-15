# Ovrtone 1.0 stable: what goes in

The owner's list for the first stable build (after 1.0 Beta 1). Nothing here is built yet. Do not make the stable installer until the owner says so.

## 1. Safe project files

A `.ovrtone` file names each plugin by file path, and Ovrtone loads that path as written (`project_io.rs` `plugin_from_doc`, then `plugin_rack.rs` `load_plugin_safely`). A project from someone else can therefore make Ovrtone load any plugin file, including one packed in the project's own media.

- Load a plugin only if its file is in the user's scanned plugin list (match by id, then path). Otherwise show it as missing ("not in your plugins: install it, then Rescan") and keep its settings for when it is installed.
- Never load a plugin file from inside a project's media folder.
- Do the same for bus plugins (`bus_plugins.rs`, `load_for_bus`).

## 2. Several inputs at once (multi-input interfaces)

Today a track picks one input (`InputSource::Mono(n)` or `Stereo(n)` in `ovrtone-engine/src/capture.rs`).

- Check, then make sure, that several armed tracks each record their own input at the same time (a drummer on 8 mics, a band live).
- Input names from the interface's driver in the menu, and user labels ("Kick In", "Vox Mic").
- An input monitor per track that follows its own input.
- Record-arm several tracks at once, with the same count-in and punch for all.

## 3. Several MIDI devices at once

Today every MIDI input port is opened and their notes are merged (`midi_in.rs`), so a keyboard and a drum pad both play the same track.

- Each instrument track chooses its MIDI input: All, or one device (and channel), as Logic's MIDI input per track.
- Keys on one track and a drum pad on another, played and recorded together.
- The device a note came from is kept with it, so a track that listens to one device ignores the others.

## 4. Controllers: MIDI CC and Mackie Control

MIDI learn exists (`midi_learn.rs`) for single controls.

- Controller assignments: a page of what each knob and fader does, saved per controller and per project.
- Mackie Control (MCU) and HUI: 8 faders per bank, bank and channel buttons, transport, jog, select, mute, solo, rec, the V-Pots for pan and sends, the display and meters sent back to the controller. Covers Mackie, Behringer X-Touch, iCON, PreSonus FaderPort in MCU mode and most keyboards with a DAW mode.
- Motorized faders follow automation.

## 5. Waves plugins: Ovrtone stops responding

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

Replaces the drop rule in `sample_place.rs` (hits to the Channel Rack, stems and loops to the Playlist):

- Any audio dragged onto the Playlist becomes audio there, whatever its length.
- Dragged onto the Playlist's border (the edge of the track area) it asks what to use it for: the old choices (Channel Rack channel, step pattern, play in the Piano Roll, on this track).
- Right-click a sound in the Browser for the same choices: send to the Channel Rack, add as audio at the playhead, open in the sampler, and so on.

## 7. Updates between builds

- From now on, changes ship as an update patch, not a full installer: a small Inno Setup installer with the same AppId that replaces only what changed (usually `ovrtone.exe` and the docs) in an existing install, and refuses to run where Ovrtone is not installed.
- The full installer (with the Stem Splitter files and the VC++ runtime) is made once, for the final build.
