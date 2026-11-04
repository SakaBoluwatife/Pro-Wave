# CLAUDE.md

@AGENTS.md

## What this is

Saka's fork of Pro Wave (Rust, egui, cpal): a Logic-Pro-style DAW being turned into **Logic Pro for Windows**, with an FL-Studio-style channel rack added. Owner: Saka (Saka Audio), mixing/mastering engineer and audio developer. Fork: https://github.com/SakaBoluwatife/Pro-Wave. Working branch: `browser-panel`.

Goal: daily-driver DAW. **No stock instruments, effects or sound library.** Users bring VST3/CLAP plugins and their own samples. See `docs/LOGIC-WIREFRAME.md` for the full feature map and milestone order.

## Rules from the owner

- **Do not push to git** until the owner says so. Commit locally only when asked.
- Windows is the target. Use Windows equivalents: WASAPI (ASIO optional), VST3/CLAP (no AU), Ctrl instead of Cmd, `.exe` + installer, `%APPDATA%` for config.
- Terse replies. Give complete, ready-to-paste files rather than partial snippets.
- Do not copy text, artwork or branding from Apple's Logic manual or app. Our own wording, "Pro Wave" naming.
- Behavior reference for FL-style features: https://github.com/ItsAlphaNeon/Lemon-Studio (read it, port algorithms; its UI is coupled to its own App/State so do not copy UI code).

## Priorities

1. VST3/CLAP plugin hosting (new crate, not yet started).
2. Browser: samples done; plugins and presets next.
3. Channel rack: notes are the source of truth; per-channel toggle between drum-rack and piano-roll views; patterns place a region on each channel track.
4. Edit menu (Cut/Copy/Paste/Split/Join), bounce/export, clip gain and fades, comping.

## Commands (PowerShell, from the repo root)

```
cargo run --release -p prowave-app     # run the app
cargo test                             # all tests
cargo test -p prowave-engine           # engine only
cargo clippy --all-targets -- -D warnings
cargo fmt
cargo xtask check                      # fmt, clippy, tests, rt-check, cargo-deny, wasm32 check
```

Open PowerShell in the repo first (`cd $HOME\dev\Pro-Wave`). Opening it in `system32` fails. Open a new terminal after installing Rust so `cargo` is on PATH.

## Architecture

- `crates/prowave-engine`: real-time audio. Lock-free, **no allocation, locking or I/O on the audio thread**. `#![deny(unwrap_used, expect_used)]`. Debug tests abort if the audio path allocates.
- `crates/prowave-app`: egui UI. Talks to the engine only by sending `Command`s on an rtrb queue. State comes back through a triple buffer and atomics. Freed engine state returns on the `Garbage` queue to be dropped on the UI thread.
- `crates/prowave-project`: `.prowave` zip project format.
- `xtask`: dev tasks.

Pattern for adding engine state: add a `Command` variant, a `Garbage` variant for anything the engine replaces, include it in `retires_state`, and add a unit test. See `Command::SetPreview` / `Preview` in `core.rs`.

## Conventions

- Diffs small (about 300 lines), one crate per change where possible.
- egui 0.37: use `Id::unique`, not `Id::new`; `ui.scope_builder(UiBuilder::new().max_rect(..))`; `show_rows` for long lists; `egui::DragAndDrop` for payloads.
- **i18n:** `crates/prowave-app/locales/*.txt` has 16 locales. A test fails if a literal `tr("...")` is missing from any. Until translations are added, new UI strings are plain English literals (no `tr()`).
- Keep CRLF line endings in files edited on Windows; do not reformat whole files.
- Param ranges match Logic's where a Logic equivalent exists.

## What exists from our work (uncommitted unless the owner says otherwise)

- `sample_browser.rs`: expandable folder tree over several sample locations, recursive search, favorites, preview through the engine (`Command::SetPreview`), drag a sample onto the Tracks area (audio or MIDI), double-click to add. Locations are in `browser-roots` and open folders in `browser-expanded` in the config dir (the old `browser-root` is migrated); it starts empty.
- `left_panel.rs`: tabs Library | Browser | Plugins. Plugins tab only lists installed `.vst3` files.
- `import.rs`: `import_audio_at` places audio at a bar on a chosen track.
- `tracks_area.rs`: drop handling for `SampleDrag`.
- `core.rs` / `lib.rs`: `Preview` playback in the engine, with test `preview_plays_once_and_stops`.

## Known gaps (see ROADMAP.md section 3b for the full list of greyed controls)

Plugin hosting, bounce/export, Cut/Copy/Paste/Split/Join, clip gain and fade UI, comping, Flex UI, key commands, automation UI, FL-style patterns. wasm32 check was not run for the browser work; run `cargo xtask check` before any push.

## Next step

Start M1 in `docs/LOGIC-WIREFRAME.md` section 11: a `prowave-host` crate that scans and loads a CLAP effect and runs it as an insert with preallocated buffers and PDC, then VST3.

## Output style (from drona23/claude-token-efficient, trimmed)

- Read existing files before writing. Do not re-read unless changed.
- Thorough in reasoning, concise in output.
- Skip files over 100KB unless required.
- No sycophantic openers or closing fluff.
- No emojis or em-dashes.
- Do not guess APIs, versions, flags, commit SHAs, or package names. Verify by reading code or docs before asserting.
