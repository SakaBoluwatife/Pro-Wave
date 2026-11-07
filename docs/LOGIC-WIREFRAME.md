# Ovrtone: Logic-for-Windows Wireframe

Feature map for turning Ovrtone into a Logic-style DAW on Windows with FL Studio's piano roll and channel rack. Structure follows the chapter layout of Apple's public Logic Pro User Guide and the inventory in `ROADMAP.md` section 1. Written in our own words. No Apple text, artwork or branding goes into the app.

**Scope rule:** the DAW ships with no stock instruments, stock effects or sound library. Everything sonic is brought by the user as **VST3 or CLAP** plugins and their own sample folders. What stays built in is the host, the editors, the mixer and the utilities (metronome, a one-shot sampler, meters).

Status key: **HAVE** works now · **PART** partly done · **GREY** menu/control exists but does nothing · **TODO** not started · **OUT** deliberately not built.

Windows swaps used throughout: Cmd becomes Ctrl, AU becomes VST3/CLAP, CoreAudio becomes WASAPI (default) plus optional ASIO, CoreMIDI becomes WinMM/WinRT MIDI, Finder becomes File Explorer, app bundle becomes `.exe` plus installer.

**Last checked against the code:** 2026-11-05. HAVE means built and tested in code; the owner's own testing by ear and by hand is still going on.

---

## 0. What is left, in the suggested order

1. **Crash isolation:** plugins in a process of their own. A crash while loading is handled; one while playing or in a plugin window still ends the program.
2. **CPU, second part:** a clean measurement on an idle machine (`perf.rs`, ignored tests), then block processing of strips if sessions with many built-in effects or clips need it.
3. **Comping, further:** real overlapping crossfades at comp joins, take names and colours.
4. **Piano roll, further:** per-note pitch bend for slides (they bend the whole instrument for now).
5. **Audio setup, second part:** the ASIO driver's control panel, separate input and output devices.
6. **Theme, second part:** the shared colours and sizes are in `theme.rs`; about 300 one-off colours remain in the screens. Dockable panels.
7. **Tuner, controller lanes in a Step Editor.**
8. **Plugin presets in the Browser**, project audio browser, Plugin Manager window.
9. **Small menu entries** (section 16).
10. **Installer run for real**, code signing, Mac and Linux plugin windows.
11. **Later or to decide:** Live Loops, score printing and MusicXML, Mackie control, Alternatives, the system-wide audio driver idea.

---

## 1. Main window layout

| Area | Logic role | Ovrtone | Notes |
|---|---|---|---|
| Control bar / transport | Play, record, cycle, tempo, LCD | HAVE | Tempo can be typed; buttons name themselves on hover. Low Latency works. Tuner not built |
| Left panel | Library | HAVE | Tabs: Library, Browser, Plugins |
| Left panel, Browser tab | Sample/loop browser | HAVE | Folder tree, search, favorites, preview with stop, waveform panel with kHz, BPM and length |
| Left panel, Plugins tab | Plugin manager | HAVE | Filter by kind, search, folders, rescan, double-click or drag to a track |
| Inspector | Track and region parameters | PART | Sample BPM row added. Channel, Freeze, Flex mode and the Q-rows are GREY |
| Tracks area | Arrange | HAVE | Pointer, Fade, Scissors, Glue, multi-select, marquee, right-click menus |
| Bottom editor area | Mixer, Piano Roll, Score, Step Seq, Audio editor | PART | Mixer and Piano Roll switch by tab; the others open from the selected region |
| Channel Rack window | Not in Logic | HAVE | F6 |
| Right side | Media/Loop browser, Notepad, Quick Help | PART | |
| Smart Controls | 8 macro knobs per plugin | HAVE | View > Show Smart Controls. Knob choices are not saved in the project yet |
| Toolbar customize, full screen | | GREY | |

## 2. Projects and files (chapters: projects, sharing)

| Feature | Status | Windows note |
|---|---|---|
| New/Open/Save `.ovrtone` (zip) | HAVE | |
| Save as Template, Close | HAVE | Your templates show under New from Template |
| Alternatives, Backups, Clean Up, Consolidate | TODO | Alternatives = named snapshots of the arrangement |
| Import audio (drag in), MIDI files | HAVE | The File > Import > Audio File entry is GREY |
| Import other projects | HAVE | Chosen tracks with their regions, effects and plugins |
| Export MIDI selection | HAVE | |
| Bounce project or section, track, all tracks (stems) | HAVE | Through a second engine faster than real time: WAV, FLAC, MP3 320. Plugins, automation, delay compensation, tails |
| Bounce regions in place, Bounce and Join | HAVE | Tracks in Place is GREY |
| Export MusicXML, AAF | TODO | Low priority |
| Show in File Manager | HAVE | Opens File Explorer on the project |
| Share to services | OUT | |
| Page Setup, Print | GREY | Low priority |

## 3. Tracks (chapter: tracks)

| Feature | Status | Notes |
|---|---|---|
| Audio, Software Instrument, Aux, Bus, Output, Master | HAVE | Plugins on Stereo Out and Master, one chain shown on both |
| External MIDI track | HAVE | |
| Drummer track | HAVE (generated notes) | Stock kit goes with Task 3; needs a user plugin then |
| Folder / Summing stacks | HAVE | |
| Delete with the key, right-click menus | HAVE | The last track cannot be deleted yet |
| Track rename, color | HAVE | Track menu |
| Track icon, header config | HAVE | Assign Track Icon is saved; Configure Track Header is for this run |
| VCA | HAVE | |
| Freeze | HAVE | Right-click a track, or the Inspector. Not saved: a project opens with its tracks live |
| Track alternatives (playlists) | TODO | |
| Groove track | PART | MIDI followers only |
| Global tracks (Marker, Tempo, Signature) | HAVE | Chord, Arrangement, Movie GREY or OUT |

## 4. Recording and comping

| Feature | Status | Notes |
|---|---|---|
| MIDI and audio recording | HAVE | Keyboard, MIDI ports and laptop keys |
| Record enable, input monitor | HAVE | |
| Capture Recording, count-in, metronome | HAVE | Metronome is a built-in click |
| Autopunch | HAVE | One punch a pass; recording stops at the punch out. Locators come from the selection or Cycle |
| Replace mode, Record Repeat | GREY | |
| MIDI take folders, swipe comping | HAVE | |
| Audio take folders, swipe comping | HAVE | From Cycle recording, and recording over a region. Right click a take: hear it alone, keep comps by name, copy a take or the comp to a new track. Joins between stretches can be dragged |
| Low latency mode | HAVE | Transport button and Project Settings |

## 5. Audio editing (chapters: loops/media, Audio Track Editor, Flex)

| Feature | Status | Notes |
|---|---|---|
| Trim, move, fade handles, Split, Join, Cut, Copy, Paste | HAVE | |
| Clip gain, fade curves, crossfades | HAVE | Gain in the Region panel; no gain handle on the region itself |
| Normalize, Reverse | HAVE | |
| Tempo following | HAVE | Sample BPM typed or found; other regions resize live with the tempo |
| Flex Time / Flex Pitch | PART | Modes, quantize, correct to key work. Markers by mouse and per-note pitch editing TODO |
| Transient detection, Smart Tempo | HAVE | |
| Strip silence | TODO | |
| Project audio browser | TODO | |

## 6. MIDI editing

| Feature | Status | Notes |
|---|---|---|
| Piano Roll, FL behaviour | HAVE | Draw, Paint, Delete, Mute, Slice, Select; type to MIDI; ghost notes |
| Note tools | HAVE | Arpeggiate, Strum, Flam, Note Repeat, Claw, Limit, Levels, Velocity LFO, Randomize, Chord Stamp, Flip, select by pitch or position |
| Quantize, swing, scale quantize, scale highlight, humanize | HAVE | |
| Time selection, slide notes, note colors, Zoom and Playback tools, Shift+R and Shift+M | HAVE | As in FL Studio. Slides bend a hosted instrument, with its bend range set in the inspector |
| Step Sequencer / Pattern region | HAVE | 16 rows by 64 steps |
| Score editor | PART | Clefs, keys, rests and ties; no printing or MusicXML |
| Event List | HAVE | |
| Step Editor (controller lanes) | TODO | `step_editor.rs` is the Pattern editor |
| MIDI Transform | HAVE | |
| MIDI FX (arp, chord trigger, randomizer, Scripter) | HAVE | |

## 7. Channel rack (FL-style, our addition)

HAVE (`channel_rack.rs`, `song_pattern.rs`), F6.

- Pattern lanes are the source of truth. One row per instrument track: mute, solo, volume, pan, mixer shortcut.
- Per channel: step buttons or a small piano roll over the same notes. Notes the steps cannot show get a mark and are never deleted.
- Patterns: new, clone, rename, color, length; drag one onto the Tracks area. Every placed clip follows the pattern; a Piano Roll edit of a clip goes back to it.
- Make Unique and Clone Pattern from the clip's menu. Split, Join and trimming the start detach a clip.
- A sample dropped on a row loads there; on Add Channel it makes a channel.
- Limits: only the first pass of a clip longer than its pattern edits back; all steps of a pattern have one velocity.

## 8. Mixing and automation

| Feature | Status | Notes |
|---|---|---|
| Faders, pan, mute, solo, sends, buses | HAVE | Lit arcs on pan and send knobs |
| Inserts with delay compensation | HAVE | Up to 32, ending in a plus; they scroll when the fader needs the room |
| Effect plugins in any slot, instrument plugins on the Inst slot | HAVE | Bypass and remove buttons, hover names |
| Mixer groups, VCA | HAVE | |
| Automation lanes and modes | HAVE | Fader, pan, built-in effects. Show/Create/Convert menu entries GREY |
| Plugin parameter automation | PART | Drawn lanes play, in bounces too. Writing from the plugin window, and on-screen knobs following, TODO |
| Metering (peak) | HAVE | LUFS, true peak, pre-fader TODO |
| Channel strip settings save/load | HAVE | Setting menu: effects and plugins with their settings, as `.pwstrip` files |
| Signal flow view | HAVE | Mixer View menu |

## 9. Tempo, time, sync

| Feature | Status | Notes |
|---|---|---|
| Tempo map, signatures, Cycle | HAVE | Cycle takes the selection when turned on |
| Smart Tempo | HAVE | |
| MTC out/in, SMPTE | HAVE | |
| Go to Position | HAVE | Navigate menu |
| Tuning (Scala .tun) | TODO | |
| Video track | OUT for now | |

## 10. Live Loops

TODO. Grid of cells and scenes, quantized launch, record performance to arrangement. Needs an engine clip launcher. To decide if it is wanted now that the channel rack exists.

## 11. Plugin hosting

| Piece | Status | Notes |
|---|---|---|
| Scan VST3 and CLAP folders, the user's own too | HAVE | In child processes with a timeout and a cache |
| Effects and instruments | HAVE | Tried on 20 CLAP plugins, and Lounge Lizard and an ERA 6 plugin as VST3 |
| Plugin editor windows | HAVE | Windows only |
| State in `.ovrtone`, undo | HAVE | |
| Latency into delay compensation | HAVE | |
| Parameters | PART | Listed and moved by automation; tried on Lounge Lizard, Pro-C 3, Pro-DS |
| Blocked after crashing on load | HAVE | Until Rescan |
| Crash isolation (out-of-process) | TODO | The engine only sees a trait, so a bridge can replace an instance |
| Presets in the Browser | TODO | |
| Plugin Manager window | HAVE | What was found, what failed, Rescan |
| VST2 | OUT unless asked | |

## 12. Settings, key commands, control surfaces

| Feature | Status | Notes |
|---|---|---|
| Settings window (Ctrl+,) | HAVE | Audio, Keyboard, Plugins, General |
| Audio driver, device, buffer size | HAVE | WASAPI, and ASIO with `--features asio`. All five outputs of the dev PC start. Sample rate and the driver's panel TODO |
| Project Settings entries | HAVE | General, Synchronization, Metronome, Recording, Audio and MIDI. Tuning is GREY |
| MIDI ports | HAVE | All ports, automatically |
| Key commands | HAVE | Logic's, moved to Ctrl and Alt (`key_commands.rs`), with an editor to remap them in Settings |
| MIDI learn, controller assignments | TODO | |
| Mackie / HUI support | TODO | |
| Remote control from phone/web | TODO | ROADMAP suggests WebSocket |
| Environment window | OUT | Not needed |

## 13. Explicitly OUT (user brings their own)

- All stock instruments (Alchemy, ES2, samplers, Drum Kit Designer, etc.) and the sound library.
- All stock effects. The built-in ones are still in the slot menus until Task 3.
- Apple Loops content, Session Player content.
- Any Apple artwork, names, icons, or trademarked terms. UI says "Ovrtone", not "Logic".

## 14. Windows-specific tasks

| Task | Status | Notes |
|---|---|---|
| ASIO cargo feature and driver picker | HAVE | SDK in `third_party/asio-sdk` under its GPL3 option. Building it needs LLVM |
| Installer and `.ovrtone` file association | PART | `dist/windows/installer.iss`, never run: Inno Setup is not installed here |
| Per-user config under `%APPDATA%` | HAVE | |
| High-DPI, dark title bar | PART | The program draws its own dark title bar; high-DPI not checked on its own |
| Code signing | TODO | |

## 15. Other systems

CLAP and the engine build elsewhere; plugin editor windows and VST3 are Windows only (`editor_window.rs`, `vst3_plugin.rs`). Mac needs a Cocoa window, signing and notarization; Linux an X11 one. The name "Ovrtone" and the UI can change later: the audio engine only talks to the UI through commands, but colours are written into each screen and need moving into one theme first.

## 16. Menu entries still greyed (checked 2026-11-05)

For now the work is on mixing and beat making, so these wait. Kept in the menus, greyed.

- **On hold, Apple's with no Windows alternative:** Song to Mobile Device, Open in Sound Library Manager
- **On hold by the owner:** Movie, Show Movie, Video (no movie support for now)
- **On hold, nothing behind them:** Session Player, Performers, Project Settings (Tuning: plugins have no general way to follow it)
- **Waiting:** Alternatives and Project Alternatives, Page Setup, Print, Score as MusicXML, Show Chord Track, Show Arrangement Track, Record / Record Repeat, New Take, Delete Take, Show Advanced Tools (nothing in Ovrtone is hidden behind it), Show Flex Editor
- **Mixer:** Create New Output Channel Strip (the engine mixes to one stereo output; a second needs engine work)
- **Inspector:** Q-Reference, MIDI In Port, MIDI In Channel, MIDI Out Channel
- **Mix:** Stem Splitter (two versions were made at once; the owner chooses which to keep)
