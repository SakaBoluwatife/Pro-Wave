# Prompt for Claude Code (paste this as your first message)

Read `CLAUDE.md`, `AGENTS.md` and `docs/LOGIC-WIREFRAME.md` first. Then do the tasks below in order. Do not push to git. Commit locally only when I ask. Run `cargo xtask check` (or at least `cargo clippy --all-targets -- -D warnings` and `cargo test`) after each task. Keep diffs small and tell me what changed in a few lines.

Context: this is my fork of Ovrtone, becoming a Logic-Pro-style DAW for Windows with FL-Studio-style channel rack and playlist workflow. No stock instruments, effects or sound library. Users bring plugins and samples. Windows only for now: WASAPI/ASIO, Ctrl not Cmd, `.exe`.

## Task 1: Browser folder tree (replace the nested folder navigation)

Problem: in `crates/ovrtone-app/src/sample_browser.rs` (Browser tab), opening a folder replaces the list and you must press Up to go back. I want FL Studio's behavior: one expandable tree.

- Show folders as a collapsible tree (triangle to expand/collapse, indented children) with samples listed under their folder. Several folders can be open at once. Collapse closes it. Keep the whole tree visible and scrollable.
- Lazy-load children when a folder is first expanded; cache listings; handle unreadable folders and removed drives without crashing.
- Virtualize the rows (flatten the open tree into a row list and use `show_rows`) so huge packs stay fast.
- Support **multiple root folders** ("sample locations"): an "Add folder…" button (rfd folder picker), a remove action per root, roots persisted one per line in `browser-roots` in the Ovrtone config dir (migrate from the old single `browser-root`). Roots appear as top-level tree nodes. No hard-coded `D:\Packs`; start empty with an "Add folder…" prompt, or migrate the saved root.
- Persist which folders are expanded between sessions (optional, keep it simple).
- Keep: search (recursive, background thread, cancel), favorites, click-to-preview through `Command::SetPreview`, double-click to add, drag onto Tracks area (`SampleDrag`), ghost label while dragging.
- Search results can stay a flat list; clearing the search returns to the tree.
- Update the existing unit tests; add tests for flattening the tree, expand/collapse, adding and removing roots.
- New strings: plain English literals (no `tr()`), because the i18n test needs all 16 locales.

## Task 2: Plugin hosting (highest priority feature)

Build a new crate `ovrtone-host` (AGENTS.md names it `ovrtone-plugin-host`; pick one and keep it consistent) that hosts third-party plugins. Order:

1. **CLAP** first (simplest; try the `clack-host` crate). Effect plugins as track inserts, then instruments.
2. **VST3** second (Steinberg's `vst3` bindings crate, host written by hand; check the current VST3 SDK license at the time, it is MIT/GPLv3 and this project is GPL-3.0-or-later).
3. **VST2**: skip until I confirm. Steinberg no longer licenses the VST2 SDK, so shipping VST2 hosting has legal risk. When we get there, make it a separate optional feature flag, loading existing `.dll` plugins only, and tell me the risks before you write it.

Requirements:
- Engine side: a plugin slot trait object with preallocated input/output buffers. No allocation, locks or I/O on the audio thread. Engine stays `#![deny(unwrap_used, expect_used)]`. Plugin loading, scanning and editor windows live on the UI/host thread; hand the loaded instance to the engine through a `Command` and return the old one through `Garbage`.
- Instrument tracks: note-on/off/CC/pitch bend from regions and live MIDI into the plugin; audio out into the track strip. Effects: insert slots with PDC using the plugin's reported latency (engine already has PDC).
- Parameters: list, read, set, and make them automatable (engine has `AutoLane`).
- Plugin editor window: open the plugin's GUI in its own native window on Windows (HWND child via `raw-window-handle`), close cleanly, remember window position.
- State: save and restore each plugin's state inside the `.ovrtone` project (bump the project format version, keep old projects loading).
- Scanning: replace the folder-listing in `left_panel.rs` (Plugins tab) with a real scanner. Windows folders: `%COMMONPROGRAMFILES%\VST3`, `%LOCALAPPDATA%\Programs\Common\VST3`, `%COMMONPROGRAMFILES%\CLAP`, `%LOCALAPPDATA%\Programs\Common\CLAP`, plus user-added folders. Scan in a child process or with a timeout per plugin, blacklist plugins that crash or hang, cache results (name, vendor, format, path, is-instrument).
- Plugins tab: show name, vendor, format; drag a plugin onto a track to add it (instrument onto an instrument track, effect onto the insert slots); double-click a plugin to add it to the selected track.
- Crash isolation: start in-process but put the plugin behind a trait so a separate bridge process can replace it later (ROADMAP Phase 7). If a plugin panics or fails to load, show a message and never take down the DAW.
- Tests: a fake in-process "plugin" implementing the trait to test the engine path (silence in, no NaN/Inf, latency reported, no audio-thread allocation).

## Task 3: Remove the stock instruments and effects

Do this **after** Task 2 can load and play a plugin, so the DAW is never silent with nothing to replace it.

- Remove the built-in effects (11 kinds in `ovrtone-engine` `fx.rs`) from the UI: insert menus, library patches, Plugin Manager. Remove built-in synth/kit patches (`synth.rs`) from the instrument pickers and Library presets, and the Drummer/Pattern region default sound.
- Keep what is not a "sound": metronome click, the sample-preview player, meters, PDC, and the engine's routing. Keep the FX trait/chain machinery, because hosted plugins use it.
- New tracks start empty (no stock Tape Delay or default instrument). Let me delete any track. Default project should be empty.
- Library tab: keep the Library for saving and loading the user's own channel-strip settings, but with no factory content.
- Remove or mark clearly the leftover tests and fixtures that depend on the stock sounds. Update `ROADMAP.md` 3b and the wireframe doc to say which things were removed.
- Tell me exactly what you deleted in the final summary.

## Task 4: Channel rack and Playlist (FL-style, notes as the source of truth)

Read how Lemon Studio does it (https://github.com/ItsAlphaNeon/Lemon-Studio, `rack.rs`, `playlist`/`history.rs`, piano roll tools) for behavior only. Do not copy its UI code; it depends on its own App/State. Algorithms on notes (arpeggiate, strum, chop, flip, etc.) can be ported.

- **Pattern model:** a named pattern is a set of note lanes, one per channel. Notes are the single source of truth.
- **Channel rack:** one row per channel (a channel = a track with a hosted instrument or a sample). Each row has mute/solo, volume, pan, a mixer-strip shortcut (Logic-style, no routing window), and step buttons.
- **Per-channel view toggle:** drum-rack (step grid) or piano roll, editing the same notes. Switching must not lose or change notes. Handle notes that do not fit the step grid (show them as non-grid markers, never delete them).
- Browser drop onto a channel loads the sample for that channel (use a sample-player plugin the user hosts, or a minimal one-shot sample player if needed; ask me before adding a built-in one).
- **Playlist:** the existing Tracks area becomes the playlist. Add pattern clips: placing a pattern clip creates a region on every channel track it uses, linked to the pattern, so editing the pattern updates all placed clips. Support make-unique (detach a clip from its pattern), clone, rename, color, and drag from a pattern list.
- Piano roll tools from Lemon Studio and Logic: paint/brush, slice, select by equal pitch/position, scale quantize, quantize with swing/strength, humanize, velocity tool, note repeat, flam, chord stamp, scale highlight. Implement as small separate commits-worth of changes, with tests on pure note functions.
- Undo/redo must cover all of it (use the existing snapshot undo).

## Task 5: Follow-ups (only after the above, one at a time, ask me first)

Edit menu (Cut/Copy/Paste/Split/Join/Select All), audio export/bounce, clip gain and fades UI, comping, key commands, installer `.exe`, ASIO behind a cargo feature.

## Rules to follow

- Windows PowerShell is the shell; run commands from the repo root (`cd $HOME\dev\Pro-Wave`).
- No allocation or locking on the audio thread. Commands in, Garbage out.
- egui 0.37: `Id::unique` not `Id::new`.
- i18n: no new literal `tr("...")` strings unless you add all 16 locale entries.
- Keep CRLF line endings in files I edit on Windows. Do not reformat whole files.
- Do not copy Apple text, artwork or branding. UI says "Ovrtone".
- Don't push. Don't commit unless asked.
- Ask me before: adding a dependency with a non-GPL-compatible license, adding VST2, or deleting anything outside Task 3's list.
