# Ovrtone Spatial Audio: Phase 0 Audit

Read-only audit of the repository as of 2026-11-11 (branch `browser-panel`), for the Spatial Audio
roadmap. Nothing was changed to produce it. The licence notes below were checked against each
repository on GitHub on the same date; re-check them at the commit you integrate.

## 1. Audio engine architecture

- `crates/ovrtone-engine` is the real-time engine. `EngineCore::process` renders interleaved stereo.
  The UI sends `Command`s on an rtrb queue. State returns through a triple buffer (`EngineState`) and
  atomics (`Shared`). Freed state returns on the `Garbage` queue to be dropped on the UI thread.
- Work is cut into **spans** (`plan_span`, at most `SPAN` = `PLUGIN_BLOCK` frames). A span ends at
  every note, controller, cycle end and click, so parameters are constant within a span.
- Strips with effects or plugins run on a dependency-graph worker pool (`parallel.rs`). Simple strips
  mix on the audio thread (`mix_strips`, with an AVX2 kernel in `simd.rs`).
- Output goes through `output.rs` (cpal: WASAPI or ASIO) or `wasapi.rs` (IAudioClient3). Playback can
  render up to about 50 ms ahead while nothing is live (`ahead.rs`).
- Offline renders (bounces, ADM export) drive `EngineCore` directly through `Engine::offline`.

## 2. Mixer and routing

- **Strips are stereo.** Per track there is `in_l/in_r` (f64, one span), a stock effect `Chain`, a
  list of hosted plugins, PDC, a fader and pan, sends, and an output.
- Buses: `MAX_BUSES` stereo buses (`bus_l/bus_r`). Sends (`SendSpec`, pre/post) feed buses, and an
  Aux strip reads a bus (`route.bus_in`). A track's output is Stereo Out (0) or a bus.
- Sidechains: every strip writes its post-chain sound to `side_l/side_r`. Effects (`Fx.side`) and
  plugins (`side_source`) read the source strip, and the graph orders the source first.
- VCAs and groups live in the app. The master is a stereo strip with its own chain.

## 3. Multichannel capability (what exists today)

The spatial system is further along than the roadmap assumes.

| Area | Exists | Where |
| --- | --- | --- |
| Object model | Azimuth, elevation, size, spread, centre, LFE send, per-object headphone distance | `spatial.rs` `ObjectPlace` |
| Bed | Tracks not marked as objects stay on Stereo Out and form the bed's L/R. "Surround panned in the bed" (`Track.bed_pan`) puts a track's surround pan into the bed | `model.rs` `Track.object`, `bed_pan` |
| Bus | One fixed **12-channel 7.1.4 bus** (`SPATIAL_CHANNELS`), in Windows channel order | `spatial.rs` |
| Layouts | 5.1, 7.1, 7.1.2, 7.1.4, as subsets of the 12-channel bus, from a fixed table | `SpeakerLayout::place` |
| Panning law | Constant power, with each speaker weighted by `((cos+1)/2)^sharp`; size widens it. **Not VBAP** | `point_gains_with` |
| Modes | Off, Speakers (needs a device with that many outputs, else folds to stereo), Folded (stereo downmix), Binaural | `SpatialMode` |
| Binaural | Modelled head: Woodworth ITD, Brown-Duda head shadow, rear/top cues, a room send. **No HRTF data, no SOFA** | `spatial.rs` `Binaural` |
| Monitoring | Per-channel mute, solo, headphone distance, renderer volume, peak meters, loudness | `spatial_window.rs` |
| Room view | 3D room drawing, top view and side view panner, drag to place | `spatial_window.rs` |
| Automation | Lanes for Around, Height, Size and LFE (`AutoTarget::Object`), gains ramped across each span | `core.rs` `pan_object` |
| Export | ADM BW64 (BS.2076/2088) with a 7.1.4 bed (pack `AP_00010017`), either mixed or with objects as their own tracks plus their moves in `axml`/`chna`. RF64/ds64 for large files | `adm.rs`, `bounce.rs` |
| Object limit | `MAX_OBJECTS = 58` in the UI (sized for one ADM file next to the bed) | `spatial_window.rs` |

Missing compared with the roadmap:
- Stereo and 9.1.6 as layouts.
- Custom or asymmetric speakers, and per-speaker distance, gain and delay.
- VBAP.
- XYZ positions and distance.
- Width and divergence as separate controls.
- Ambisonics.
- HRTF/SOFA.
- Spatial buses, multichannel strips and multichannel beds.
- Layout-to-layout downmix matrices other than the fold to stereo.
- ADM import, IAMF, and LFE filtering.

## 4. Plugin channel layouts

- `ovrtone-host` CLAP (`clap.rs`) reads the plugin's audio ports but feeds **stereo into the first
  two channels** of the main port (mixed down to mono for a mono port). The side-chain port is
  filled the same way.
- VST3 is likewise stereo in and stereo out.
- There is no multichannel plugin path, so a plugin on a spatial bed or bus cannot see more than two
  channels today. The roadmap's "do not silently insert a stereo-only processor" holds trivially,
  because there is no multichannel strip to insert into.

## 5. Automation

- Lanes are `AutoLane { target: AutoTarget, points }`. Targets cover Volume, Pan, Fx params, Plugin
  params, sends, and `Object(axis)`.
- Lanes are evaluated per span in the engine with `lane_value`, in modes Read, Touch, Latch, Write
  and Trim (app side).
- Object moves are smoothed by ramping each channel gain linearly across the span. This is
  deterministic.
- Adding new axes (X/Y/Z, distance, width, divergence) means adding `ObjectAxis` variants and their
  `get/set`. The lanes, the saved data and the ADM writer then follow, since they all index by axis.

## 6. Project serialization

- `.ovrtone` is a zip (`ovrtone-project`). `TrackDoc.object: Option<ObjectDoc>` holds the object.
  `ProjectDoc` holds `spatial_mode`, `spatial_room`, `spatial_volume_db` and `spatial_distance`.
  Object lanes are saved with the other lanes.
- Fields use `#[serde(default)]` patterns, so new optional fields keep old projects loading.
- Undo, copy, paste and duplicate copy the `Track`, so `object` travels with it.

## 7. Real-time constraints

- The engine crate has `#![deny(clippy::unwrap_used, clippy::expect_used)]`. Debug tests abort on
  allocation inside `assert_no_alloc`.
- `SpatialRender` is built on the UI thread and swapped in with `Command::SetSpatial`; the old one
  returns as Garbage. Any layout table, VBAP triangulation or HRTF set must be built the same way:
  prepared off-thread and handed over whole.
- `object_gains` runs `powf` per channel on the audio thread whenever an automated value changes.
  This is acceptable at 12 channels. VBAP gains are cheaper (a 3x3 matrix per triangle).

## 8. Recommended integration points

1. **`spatial.rs` stays the spatial core** (data model and renderer). Grow it into a folder,
   `crates/ovrtone-engine/src/spatial/`, with `layout.rs` (speakers, roles, LFE), `vbap.rs`,
   `binaural.rs`, `ambisonic.rs`, `downmix.rs` and `render.rs`. Do not add new crates: the engine is
   one crate with module boundaries, and the UI already talks to it only through `Command`s.
2. **Bus width**: make the bus `MAX_SPATIAL_CHANNELS` (16, for 9.1.6) with a channel map per layout,
   instead of the fixed 7.1.4 order. Keep the Windows order at the device boundary only.
3. **Object input**: keep the per-strip tap where `pan_object` reads `object_l/object_r`. It is the
   right place: post fader, after the strip's plugins.
4. **Spatial buses and beds**: these need a multichannel strip type in `core.rs`, which is the
   largest change. Defer it until objects, layouts and VBAP are solid.
5. **UI**: `spatial_window.rs` (renderer, room, panner) and `adm.rs` (format adapter). Keep egui out
   of the engine, as now.

## 9. Open-source components to evaluate

| Project | Licence (checked 2026-11-11) | Language | Classification | Use |
| --- | --- | --- | --- | --- |
| AOMediaCodec/oar (Open Audio Renderer) | BSD-3-Clause-Clear, plus a separate AOM `PATENTS` licence | C++ (Bazel/CMake) | Reference only | IAMF channel, scene, object and binaural rendering structure |
| AOMediaCodec/libiamf | BSD-3-Clause-Clear | C | Reference first; approved integration later if IAMF export is built | IAMF bitstream |
| AOMediaCodec/iamf (spec) | No licence file reported | Spec | Reference | IAMF |
| videolan/libspatialaudio | LGPL-2.1-or-later, also sold under a commercial licence | C++ | Reference; adaptation permitted (LGPL is GPL-3 compatible) | ADM renderer, HOA, binaural |
| hoene/libmysofa | BSD-3-Clause (Symonics) | C | Approved integration candidate (FFI) | SOFA HRTF loading |
| ryanniemi/strato-render | MIT | C/C++ | Reference; adaptation permitted | VBAP, HOA, ADM BWF, custom layouts |

All of these are C or C++. Integrating any of them means a C build step in a Rust workspace (cc or
cmake in `build.rs`), plus MSVC toolchain issues on Windows. The core algorithms (VBAP, SN3D/N3D
HOA encoding and decoding, downmix matrices) are well published and small: implement them
independently in Rust. Use FFI only for SOFA loading (HDF5-based files), unless a Rust SOFA reader
is found and audited.

## 10. Licence and commercial-use concerns

- **Ovrtone itself is GPL-3.0-or-later** (every crate's `Cargo.toml`, plus GPL code ported from
  Lemon Studio, such as `parallel.rs`). Selling it is allowed, but every binary must ship with, or
  offer, its full corresponding source under the GPL, and it cannot be relicensed as closed source
  without the consent of every copyright holder, including Lemon Studio's author. **This is the
  biggest open decision for "commercially distributed".**
- BSD-3-Clause-Clear grants **no patent licence**. AOM's `PATENTS` file is a separate grant with its
  own defensive-termination terms, and it must be read before shipping IAMF.
- LGPL-2.1+ (libspatialaudio) is compatible with GPL-3 builds. If Ovrtone ever goes closed, it would
  need dynamic linking or VideoLabs' commercial licence.
- `deny.toml` does not allow `BSD-3-Clause-Clear` or `LGPL-2.1-or-later` yet. Add them only when a
  dependency actually comes in.
- **Dolby**: no Atmos or Dolby wording was found in the code or the user-facing text. Keep it that
  way. ADM BW64 export is an open ITU format and must be described as such, never as Atmos-compatible.
  Atmos delivery requires a Dolby licence and certification, which is a separate business task.
- Missing files the roadmap asks for: `THIRD_PARTY_NOTICES.md` and an SBOM (`cargo cyclonedx` or
  `cargo sbom` could generate it; verify the tool before adopting). `dist/windows/third_party` holds
  the installer's notices.

## 11. Proposed implementation sequence

Adapted to what already exists:

1. **Stage 1, the smallest viable step (proposed below)**: data-driven speaker layouts (Stereo, 5.1,
   7.1, 5.1.2, 5.1.4, 7.1.2, 7.1.4, 9.1.6, Custom); a 16-channel bus; 3D VBAP; XYZ and distance on
   objects; the layout picker and numeric XYZ in the panner.
2. Width and divergence as their own controls and lanes; distance attenuation; LFE crossover/filter
   options; explicit downmix matrices between layouts (7.1.4 to 5.1.4, 5.1, and stereo).
3. Speaker setup window: custom positions, gain, delay, calibration, test signal, channel
   identification. Lift the object limit (bounded by `MAX_TRACKS`, not 58), with the ADM writer
   splitting large object counts as the format allows.
4. Multichannel strips: beds and spatial buses as N-channel strips, spatial sends and returns, and
   stereo-only plugin handling (explicit per-pair or downmix insert rules).
5. Binaural with HRTF: SOFA loading, interpolation, partitioned convolution off the hot path
   (prepared on the UI thread). The modelled head stays as the zero-data fallback.
6. Ambisonics: an HOA bus (ACN, SN3D/N3D), encoding objects into it, rotation, and decoding to
   speakers (AllRAD or mode-matching) and to binaural.
7. Interoperability: ADM BWF import, an IAMF investigation, then an IAMF adapter.
8. Spatial stems, binaural export, and offline/real-time parity for every renderer.
9. Performance work and user feedback.

## 12. Technical risks and open decisions

- **Licensing model** (section 10): decide GPL or closed before integrating any third-party
  renderer code.
- **9.1.6 on Windows**: `WAVEFORMATEXTENSIBLE` channel masks have no top-side speaker bits, so a
  9.1.6 device order has to be by convention (ASIO channel numbers). Decide the output channel map
  per driver.
- **Bus format change**: moving from the fixed 12-channel order to a layout-mapped 16-channel bus
  touches `adm.rs`, the fold, the meters and the device output. ADM export stays 7.1.4 until there
  are more packs.
- **VBAP versus the current law**: projects mixed with the current law will sound slightly
  different. Keep the current law as a selectable "Smooth" option, and default old projects to it.
- **Elevation below 0**: objects are limited to 0..90 degrees today. VBAP with no bottom speakers
  must clamp or fold, and that needs a decision.
- **Multichannel plugins**: a host-side change in both CLAP and VST3. It is the riskiest piece for
  crash isolation.
- **HRTF convolution cost** per object or per virtual speaker. Virtual-speaker binaural (render to
  the layout, then convolve 16 channels) keeps the cost fixed, and is the recommended start.

## Proposed Stage 1 (waiting for approval)

Scope (engine `spatial` module, app `spatial_window.rs`, `model.rs`, `project_io.rs`, about 900
lines):

1. `SpeakerLayout` becomes data. Each `Speaker` has an azimuth, elevation, distance, role
   (full-range or LFE), and a channel index. It covers Stereo, 5.1, 7.1, 5.1.2, 5.1.4, 7.1.2, 7.1.4,
   9.1.6 and Custom, with presets built in. The bus widens to 16 channels.
2. 3D VBAP: the convex hull of the speakers is triangulated on the UI thread, with inverted 3x3
   matrices. On the audio thread, finding the triangle and computing the gains allocates nothing.
   Gains are power-normalised. For 2D layouts (no height) it falls back to pairwise panning. The
   current law stays selectable as "Smooth".
3. Objects gain an `x/y/z` view (forward +y, right +x, up +z, unit cube to the walls, as `adm.rs`
   already uses) and a distance, converted to and from azimuth and elevation. New lanes are added
   for X, Y, Z and Distance.
4. UI: Stereo, 5.1.2, 5.1.4 and 9.1.6 in the layout picker; numeric X/Y/Z boxes in the panner; the
   room view draws the actual speaker set.
5. Saving: optional new fields, so old projects load unchanged.

That gives the roadmap's first prototype (mono source, XYZ, VBAP, stereo/5.1/7.1.4, real-time
output, automation, 3D room) on top of what is already there. Custom-layout editing, downmix
matrices and multichannel strips come after.

## Stage 1: done (2026-11-11, compiled, not yet evaluated by hand)

- Layouts: Stereo, 5.1, 5.1.2, 5.1.4, 7.1, 7.1.2, 7.1.4 and 9.1.6. The bus is 16 channels:
  L R C LFE Lrs Rrs Lss Rss Ltf Rtf Ltr Rtr (the Windows 7.1.4 order) then Lw Rw Ltm Rtm.
  `SpeakerLayout::device_order` maps the bus to device outputs. 9.1.6 goes out in bus order.
- VBAP in `crates/ovrtone-engine/src/vbap.rs`: the convex hull is triangulated once with inverted
  matrices. There is a virtual top speaker for layouts without height speakers (spread over the ear
  ring) and a virtual bottom speaker (dropped). Stereo pans pairwise, with the rear mirrored. The
  centre level crossfades with a mesh built without the centre speaker.
- The law (`SpatialLaw::Vbap` or `Smooth`) is a project setting. Old projects load as Smooth; new
  projects are VBAP.
- Objects have `radius` (0 at the listener, 1 at the walls) and an X/Y/Z box view. Lanes X, Y, Z
  and Distance were added. Inside the room, an object spreads toward all speakers.
- The ADM export is still a 7.1.4 bed (the first 12 bus channels), and object positions are scaled
  by `radius`.
- Not done yet: custom speaker editing (layouts are presets), per-speaker gain and delay, LFE
  filtering, downmix matrices between layouts other than the fold to stereo.

## Pass 2: done (2026-11-11, compiled, not yet evaluated by hand)

- Panner: Snap (to the nearest speaker; written as `channelLock` in ADM) and Divergence (a
  power split to two phantom places 45 degrees either side; written as `objectDivergence`, and
  automatable).
- LFE: an optional low-pass (80, 120 or 160 Hz) on the LFE channel (`ChannelSetup::lfe_hz`).
- Downmix output: `SpatialMode::Downmix(bed, to)`. Channels the smaller layout has pass straight
  across; the others are VBAP-panned to their place among its speakers, power kept. The LFE
  goes only to an LFE. The matrix is shown in the renderer.
- Speaker Setup window (`speaker_setup.rs`, kept in the settings folder, not in projects): the real
  azimuth and elevation of each speaker (VBAP and Snap use them), level -24..+12 dB, delay
  0..50 ms. These apply only when the bus goes straight out to speakers. Pink-noise test per
  channel (`Command::SetSpatialTest`), also as a T button on each channel in the renderer.
- Room view angles: 3D, Top, Front, Side, Rear. Objects are drawn at their X/Y/Z (distance
  included).
- Exports: the bed as a multichannel WAV in its layout order (WAVE_FORMAT_EXTENSIBLE, with a
  speaker mask up to 7.1.4), binaural stereo, and AmbiX first and third order (ACN, SN3D),
  encoded from the bus at its speakers' directions.

## Still to do

- Multichannel strips (beds and spatial buses as N-channel strips, the Balance panner mode,
  multichannel plugin hosting): the largest piece.
- HRTF/SOFA binaural: needs a SOFA reader (libmysofa via FFI, or a Rust crate after review).
  Downloading it needs the owner's approval.
- Ambisonic import and an Ambisonic bus (decoding to speakers or headphones). Today Ambisonics is
  export only.
- ADM import, and other ADM bed packs; IAMF.
- Spatial stems (each object as its own file), and distance attenuation.

## Pass 3: done (2026-11-11, compiled, not yet evaluated by hand)

- 7.1.2 now uses the top middle pair (Ltm/Rtm), as Atmos beds do. Device order: L R C LFE Lrs Rrs
  Lss Rss Ltm Rtm.
- ADM export, Atmos shape: a 7.1.2 bed (defined in the file as DirectSpeakers, pack AP_00011001)
  plus up to 118 mono objects, or the bed alone with the objects mixed in. The 7.1.4 + stereo
  objects shape stays.
- Spatial buses (up to 8): each has its own layout. Tracks routed to one ("Goes to" in the panner)
  are panned among its speakers, and the bus's panner (direction, elevation, spread, size,
  centre, LFE, level, mute) places the whole bus in the room. ADM object exports leave tracks on a
  bus in the bed.
- Measured heads: `HrtfSet` and the convolution are in the engine, and SOFA loading is in
  `hrtf.rs`. The reader crate (`sofar` 0.4.0, MIT/Apache) is waiting on the owner's approval. Until
  then the `sofa` feature is off and the modelled head is used.
- Not covered: multichannel audio files on tracks, plugins on a spatial bus (it has no insert
  chain), and multichannel plugin hosting.
