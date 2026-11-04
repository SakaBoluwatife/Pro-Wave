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

1. CPU optimization.
2. Remove stock instruments and effects (Task 3).
3. Audio comping, crash isolation, the rest of automation and the piano roll.

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

## Known gaps and next step

`docs/LOGIC-WIREFRAME.md` is the up-to-date feature map: section 0 is the order of what is left, section 16 the greyed entries. Next: CPU optimization (run `cargo xtask bench`), then removing stock instruments and effects (Task 3), then audio comping. ASIO builds need LLVM and `--features asio`. wasm32 check and cargo-deny are not installed here; run `cargo xtask check` steps individually before any push.

## Output style (from drona23/claude-token-efficient, trimmed)

- Read existing files before writing. Do not re-read unless changed.
- Thorough in reasoning, concise in output.
- Skip files over 100KB unless required.
- No sycophantic openers or closing fluff.
- No emojis or em-dashes.
- Do not guess APIs, versions, flags, commit SHAs, or package names. Verify by reading code or docs before asserting.
