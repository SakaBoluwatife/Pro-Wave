# Claude Code prompt: track Apple icons in use and make them replaceable

Run this after docs/OVRTONE-PROMPT.md steps 4 and 4b. Do not push. Do not commit unless I ask.

Goal: I am drawing my own replacements for every Apple icon the app uses. I need to know exactly which ones are used, and each of my icons must drop in without code changes.

1. Load order for every semantic icon name: assets/icons/mine/<name>.svg first, then the Local family, then Phosphor. If I add a file to assets/icons/mine/, the app uses it on the next run, with no code change. Watch the folder and hot-reload in debug builds.
2. Generate docs/ICONS-IN-USE.md automatically (a cargo xtask command: `cargo xtask icons`). For each semantic name list: where it is used in the UI (file and screen), which family currently supplies it (mine, Local/Apple, Phosphor), the Apple symbol file name if Local, and whether assets/icons/mine/<name>.svg exists. Sort the unreplaced Apple ones first. End with counts: total, replaced, still Apple.
3. The same command writes assets/icons/contact-sheet.png: a grid of every in-use icon at 16, 20 and 24 px with its name, marked Apple / mine / Phosphor, so I can see what to redraw.
4. Write assets/icons/mine/README.md with my drawing spec: 24x24 viewBox, single path or strokes, stroke width and corner radius matching the chosen family, filled or outline style per the semantic name, naming = semantic name, and how to test (run the app, or `cargo xtask icons`).
5. In debug builds show a small overlay option (Settings > Appearance > "Mark icons by source") that tints Apple-sourced icons red so I can spot them in the live UI.
6. Release and installer builds must fail with a clear error if any in-use icon still comes from the Local family.
7. Tests: load order, hot-reload of a new file, missing-file fallback, and the report generator on a sample set. Run fmt, clippy -D warnings and tests.
