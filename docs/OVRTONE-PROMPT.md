# Claude Code prompt: rename to Ovrtone, install brand assets, replace icons

Read CLAUDE.md, AGENTS.md and docs/LOGIC-WIREFRAME.md first. Do not push. Do not commit unless I ask. Work in the order below. After each step run `cargo fmt`, `cargo clippy --all-targets -- -D warnings` and `cargo test`, and tell me in a few lines what changed.

Product name: **Ovrtone** (written "Ovrtone" in text, "ovrtone" in the wordmark and in code). Publisher: **Bankz Sound Lab**. Project extension: **.ovrtone**. Based on Pro Wave (GPL-3.0-or-later).

## Step 1: Inventory (report before changing anything)
Find every variant: "Pro Wave", "ProWave", "prowave", "PROWAVE", "pro-wave", crate prefixes, env vars, window titles, menu text, About box, file dialogs, installer or package names, docs, CLAUDE.md, AGENTS.md, ROADMAP.md, locales/*.txt (all 16 locales must stay consistent so the i18n test passes), Cargo.toml names and metadata, config folder names. List what you found and where.

## Step 2: Rename
- Crates prowave-engine, prowave-app, prowave-project (and any prowave-host or similar) become ovrtone-*. Update paths, workspace members, xtask, CI, Cargo.lock. Binary becomes ovrtone.exe.
- Do NOT change: the LICENSE file, upstream copyright headers, git history, or the upstream repository link.
- About box and README: "Ovrtone by Bankz Sound Lab. Based on Pro Wave by the original authors" with the upstream link, and "Licensed under GPL-3.0-or-later".
- Project files: new extension .ovrtone, defined in one constant. Keep opening .prowave projects forever, and offer to save as .ovrtone. Update file dialogs and the Windows file association.
- Settings migration: on first run, if the old config folder (prowave) exists and the new one (ovrtone) does not, copy the settings once (browser roots, favorites, key commands, skins, plugin cache, window layout). Never delete the old folder.
- Tests: an old .prowave project loads; settings migration works.

## Step 3: Install the brand assets
The files are in `assets/brand/` (SVG masters, PNG renders, ovrtone.ico). Use the SVGs as masters.
- Palette: charcoal #17191D, near-black #08090B, white #F7F7F5, electric blue #1677FF. Make these the default theme tokens (dark skin) and keep them in one place.
- Embed `assets/brand/ovrtone.ico` as the Windows exe icon (build.rs with winres or equivalent) and as the window and taskbar icon (load the right size for each).
- Use ovrtone-symbol and ovrtone-wordmark in the About box, splash or empty-project screen, and README header. Render SVGs with resvg or egui_extras, tinted per theme where it makes sense.
- File association icon for .ovrtone uses ovrtone.ico.
- Installer (if one exists): name, icon and publisher updated.
- Remove every TODO(logo) placeholder. Keep the old logos out of the app and the repo.
- Bundle Inter (SIL OFL 1.1) as the UI font with its license file, and JetBrains Mono (OFL) for the time display. Add both to third-party-licenses.md.

## Step 4: Replace all icons
- Inventory every icon, image, emoji, glyph and embedded SVG (track icons such as the music note and drum kit, transport, mixer, inspector, menus, window icons). Write docs/ASSETS.md: asset, where used, source, license.
- Put all icons behind one module (icons.rs) that maps a semantic name (play, record, mute, solo, bypass, etc.) to an SVG. The family must be swappable by changing one folder or one table.
- Default family: **Phosphor Regular** (MIT). Support Lucide (ISC) and Tabler (MIT) as drop-in alternatives in `assets/icons/<family>/`, selectable in Settings > Appearance and by skins. Tint by theme color, render at the correct size for high DPI, 16px for mixer and 20-24px for transport. Heavier weight for active toggles, lighter for idle, if the family has weights.
- Draw an original set of at least 24 track icons (keys, drums, guitar, bass, vocal, synth, strings, brass, FX, bus, aux, master and more) and DAW-specific glyphs the families lack (fader cap, automation modes, punch markers) using the same stroke width as the chosen family. Do not reproduce any Apple icon style or shape, and do not use SF Symbols or anything traced from them.
- Any other icon family I name later must have its license checked and approved by me before it is added.
- Keep third-party-licenses.md updated with every license text.

## Step 5: Apple terms scan
List every UI string and doc that uses an Apple brand or Apple-specific name (Logic, Apple, Klopfgeist, Alchemy, Drummer, Quick Sampler, Smart Controls, Flex Time and similar). Rename the metronome sound. For feature names propose neutral alternatives and do not rename the rest until I choose.

## Step 6: Docs
Update CLAUDE.md, AGENTS.md, ROADMAP.md, docs/LOGIC-WIREFRAME.md (rename the file to docs/OVRTONE-WIREFRAME.md and fix links), README. Add docs/BRAND.md describing the assets, palette and fonts.

Finish with a summary of anything the rename could not cover.
