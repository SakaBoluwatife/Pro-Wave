# Pro Wave: Logic-for-Windows Wireframe

Feature map for turning Pro Wave into a Logic-style DAW on Windows. Structure follows the chapter layout of Apple's public Logic Pro User Guide and the inventory in `ROADMAP.md` section 1. Written in our own words. No Apple text, artwork or branding goes into the app.

**Scope rule:** the DAW ships with no stock instruments, stock effects or sound library. Everything sonic is brought by the user as **VST3 or CLAP** plugins and their own sample folders. What stays built in is the host, the editors, the mixer and the utilities (metronome, tuner-free tools, meters).

Status key: **HAVE** works now · **PART** partly done · **GREY** menu/control exists but does nothing · **TODO** not started · **OUT** deliberately not built.

Windows swaps used throughout: Cmd becomes Ctrl, AU becomes VST3/CLAP, CoreAudio becomes WASAPI (default) plus optional ASIO, CoreMIDI becomes WinMM/WinRT MIDI, Finder becomes File Explorer, app bundle becomes `.exe` plus installer.

---

## 0. Priority order

1. **Plugin hosting** (VST3, then CLAP). Nothing else matters if sound cannot come from user plugins.
2. **Browser** (done: samples; next: plugins, presets, project audio).
3. **Channel rack / pattern model** with drum-rack and piano-roll views over the same notes.
4. Export/bounce, audio clip gain and fades, comping.
5. Everything else below, in chapter order.

---

## 1. Main window layout

| Area | Logic role | Pro Wave | Notes |
|---|---|---|---|
| Control bar / transport | Play, record, cycle, tempo, LCD | HAVE | Count-in and Capture Recording are GREY |
| Left panel | Library | HAVE | Now tabs: Library, Browser, Plugins |
| Left panel, Browser tab | Sample/loop browser | HAVE (new) | Folder tree over several locations, search, favorites, preview, drag to track |
| Left panel, Plugins tab | Plugin manager | PART | Lists installed VST3 only, cannot load |
| Inspector | Track and region parameters | PART | Several rows GREY (Channel, Freeze, Flex mode, Q-rows) |
| Tracks area | Arrange | HAVE | Most tools except Pointer and Fade are GREY |
| Bottom editor area | Mixer, Piano Roll, Score, Step Seq, Audio editor | PART | Mixer and Piano Roll work |
| Right side | Media/Loop browser, Notepad, Quick Help | PART | Loop Browser and All Files toggle exists |
| Smart Controls | 8 macro knobs per plugin | TODO | Needs plugin parameter list first |
| Toolbar customize, full screen | | GREY | |

## 2. Projects and files (chapters: projects, sharing)

| Feature | Status | Windows note |
|---|---|---|
| New/Open/Save `.prowave` (zip) | HAVE | |
| Save as Template, Close, Project Settings | GREY | |
| Alternatives, Backups, Clean Up | TODO | Alternatives = named snapshots of the arrangement |
| Import audio (drag in) | HAVE | |
| Import other projects, MIDI file import | PART | |
| Export MIDI selection | HAVE | |
| Bounce project/region/track, export stems | TODO | Needs an offline render module in the engine |
| Export MusicXML, AAF | TODO | Low priority |
| Show in File Manager | GREY | Becomes "Show in Explorer" |
| Share to services | OUT | |
| Page Setup, Print | GREY | Low priority |

## 3. Tracks (chapter: tracks)

| Feature | Status | Notes |
|---|---|---|
| Audio, Software Instrument, Aux, Bus, Output, Master | HAVE | Instrument tracks need a hosted plugin to make sound |
| External MIDI track | PART | Port and channel fields exist in the model |
| Drummer track | HAVE (generated notes) | Uses internal note generation, no stock kit; needs user plugin |
| Folder / Summing stacks | PART | Folder in model, Functions > Folder GREY |
| Track rename, color, icon, header config | GREY | Quick wins |
| VCA | PART | Field exists |
| Freeze, Bounce in Place | GREY | Needs offline render |
| Track alternatives (playlists) | TODO | |
| Groove track | TODO | |
| Global tracks (Arrangement, Chord, Marker, Tempo, Signature, Movie) | PART | Tempo works; others GREY |

## 4. Recording and comping

| Feature | Status | Notes |
|---|---|---|
| MIDI and audio recording | HAVE | WASAPI shared; ASIO is a later cargo feature |
| Record enable on instrument tracks, input monitor on audio | GREY | |
| Punch in/out, replace mode, Record Repeat | GREY | |
| Take folders, quick swipe comping | TODO | Needs clip-take model |
| Count-in, metronome | PART | Metronome is a built-in click, not a sample instrument |
| Low latency mode | TODO | Needs PDC awareness of plugins |

## 5. Audio editing (chapters: loops/media, Audio Track Editor, Flex)

| Feature | Status | Notes |
|---|---|---|
| Region trim, move, fade tool | HAVE | |
| Clip gain, fade curves, crossfades | PART | Engine has `ClipEdit` and `FadeCurve`, UI is thin |
| Split, Join, Cut, Copy, Paste | GREY | **High value**, add soon |
| Flex Time / Flex Pitch | PART | Engine has warp and pitch modules; UI mode picker GREY |
| Transient detection, strip silence | PART | `analysis` module exists |
| Marquee, Normalize, Reverse | TODO | |
| Project audio browser | TODO | |

## 6. MIDI editing

| Feature | Status | Notes |
|---|---|---|
| Piano Roll | HAVE | Note Repeat and Note Colors GREY |
| Step Sequencer / Pattern region | HAVE | Max 16 rows by 64 steps; target for the channel rack |
| Score editor | PART | Window GREY |
| Event List | TODO | |
| Step Editor (hyper) | PART | `step_editor.rs` exists |
| MIDI Transform | TODO | |
| Quantize, swing, humanize | PART | Q-rows GREY in Inspector |
| MIDI FX (arp, chord trigger, randomizer) | PART | `midi_fx` field exists. Own implementations, not stock "sounds" |
| Piano-roll tools (arpeggiate, strum, chop, flip, etc.) | TODO | Port algorithms from Lemon Studio |

## 7. Channel rack (FL-style, our addition)

Not in Logic. Design in short:

- Notes stay the single source of truth. A channel is a track plus a note lane.
- **Rack view** shows one row per channel with step buttons. **Piano-roll view** shows the same notes. A toggle per channel switches views. Editing in either updates both.
- A **pattern** is a named set of channel note lanes. Placing a pattern clip puts one region on every channel track it touches. Edit the pattern, every placed clip updates.
- Each channel strip has its own mixer slot (Logic style, no routing window).
- Channel's sound = a hosted plugin or a sample from the Browser (Browser drop on a channel makes a sampler channel once a user sampler plugin or a minimal built-in one-shot player exists).

Status: TODO. Depends on section 11 (hosting).

## 8. Mixing and automation

| Feature | Status | Notes |
|---|---|---|
| Faders, pan, mute, solo, sends, buses | HAVE | |
| Inserts with PDC | HAVE | Today these are the 11 built-in effects. Keep the engine, but the user's plugins become the main content |
| Mixer groups, VCA | PART | Menu GREY |
| Automation lanes and modes | PART | Engine `AutoLane`; Show/Create/Convert GREY |
| Metering (peak) | HAVE | LUFS, true peak, pre-fader TODO |
| Channel strip settings save/load | TODO | |
| Signal flow view | GREY | |

## 9. Tempo, time, sync

| Feature | Status | Notes |
|---|---|---|
| Tempo map, signatures | HAVE | |
| Smart Tempo | PART | `TempoEstimate` in engine; window GREY |
| MTC out/in | HAVE | |
| SMPTE ruler, Go to Position | GREY | |
| Tuning (Scala .tun) | TODO | |
| Video track | OUT for now | |

## 10. Live Loops

TODO. Grid of cells and scenes, quantized launch, record performance to arrangement. Needs engine clip launcher. After section 7.

## 11. Plugin hosting (the big one)

| Piece | Status | Plan |
|---|---|---|
| Scan VST3 folders | HAVE (child process) | `%COMMONPROGRAMFILES%\VST3`, `%LOCALAPPDATA%\Programs\Common\VST3` |
| Scan CLAP folders | TODO | `%COMMONPROGRAMFILES%\CLAP`, `%LOCALAPPDATA%\Programs\Common\CLAP` |
| Load plugin, process audio on audio thread | TODO | New crate `prowave-host`; engine only sees a trait object slot with preallocated buffers |
| Instruments: MIDI/note events in, audio out | TODO | |
| Parameters, automation, Smart Controls | TODO | |
| Plugin editor window (HWND child or separate window) | TODO | Windows-specific; use `raw-window-handle` |
| State save/load inside `.prowave` | TODO | Format change, bump project version |
| Latency report into PDC | TODO | Engine PDC already exists |
| Crash isolation (out-of-process) | TODO | ROADMAP Phase 7. Start in-process, design the trait so a bridge can replace it |
| Scanner timeout and blacklist | TODO | Scan in a child process |
| Plugin manager UI | PART | Plugins tab lists names only |

Crate choice to evaluate: CLAP via `clack-host`; VST3 via `vst3` (bindings) with a hand-written host. VST3 uses the `vst3` crate; the Steinberg SDK is MIT-licensed (checked 2026-11-04).

## 12. Settings, key commands, control surfaces

| Feature | Status | Notes |
|---|---|---|
| Audio device and buffer settings | PART | WASAPI; add ASIO behind cargo feature |
| MIDI ports | HAVE | |
| Key commands (editor and remap) | GREY | Ctrl-based defaults |
| MIDI learn, controller assignments | TODO | |
| Mackie / HUI support | TODO | |
| Remote control from phone/web | TODO | ROADMAP suggests WebSocket |
| Environment window | OUT | Not needed |

## 13. Explicitly OUT (user brings their own)

- All stock instruments (Alchemy, ES2, samplers, Drum Kit Designer, etc.) and the sound library.
- All stock effects beyond the existing built-ins, which stay as basic utilities only.
- Apple Loops content, Session Player content.
- Any Apple artwork, names, icons, or trademarked terms. UI says "Pro Wave", not "Logic".

## 14. Windows-specific tasks

- Installer (`cargo xtask` target producing an `.exe`/MSI, e.g. cargo-wix or Inno Setup).
- File associations for `.prowave`.
- High-DPI manifest, dark title bar.
- ASIO cargo feature and driver picker.
- Per-user config dir under `%APPDATA%` (already used by `sample_browser`).
- Code signing later.

## 15. Suggested next milestones

1. **M1 Plugin host, CLAP effect loads** (simplest to prove the audio path).
2. **M2 VST3 instrument plays from a MIDI region**, editor window opens.
3. **M3 Plugin state in project**, Plugins tab can drag a plugin onto a track.
4. **M4 Pattern model and channel rack**, rack/piano-roll toggle.
5. **M5 Edit menu (Cut/Copy/Paste/Split/Join/Select All)** and Undo History.
6. **M6 Bounce and export**.
7. **M7 Comping, clip gain/fades UI, Flex UI.**
8. **M8 Key commands, Smart Controls, automation UI.**
