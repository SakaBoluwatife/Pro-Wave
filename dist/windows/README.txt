Ovrtone - Windows x64
======================

Ovrtone is a DAW (music production app), written in Rust.
This is version 1.0. Each save keeps a backup beside the project, and a crash-recovery copy is
kept while you work. It has no stock instruments or sounds: bring your own VST3 or CLAP plugins and
samples.
It does come with its own effects: EQ, compressor, multiband, limiter, gate, saturation, tape,
distortion, lo-fi, clipper, exciter, transient designer, resonance suppressor, stereo imager,
reverb, delay, chorus, flanger, phaser, auto filter, tremolo, pitch shifter, utility and meter.

What to try, and tell us about
------------------------------

Send what you find to the person who gave you this installer: what you did, what you expected,
what happened, and a screenshot if you can.

- The Ovrtone effects: in a mixer slot's menu (the Ovrtone folder) or the Plugins list. Click a
  slot to open its window, click again to close it. Several can be open; drag a window's edge to
  make it larger. Click the number under a knob to type a value.
- Side chains: the compressor, gate, auto filter and the delay's ducking can listen to another
  track (SIDE CHAIN in the effect's window); VST3 and CLAP plugins with a side-chain input have
  Side Chain in their slot's menu.
- Ctrl and the mouse wheel zoom the tracks and the Piano Roll: wheel down zooms in.
- Your own plugins: the first launch reads them all on the launch screen.
- Stem Splitter (Mix menu): vocals, drums, bass, guitar, piano, other, and drum pieces.
- Spatial audio (Mix > Spatial Audio, or File > New Surround Project): headphones, 5.1 up to
  7.1.4 speakers, and ADM BW64 export. A track can stay stereo, be surround panned in the bed, or
  be an object.
- Comping: record several passes in a cycle, then swipe across the takes. Right-click a take to
  name and colour it.
- Slide notes in the Piano Roll: CLAP instruments that take single-note tuning bend just the one
  note; others bend the whole instrument.
- Themes and fonts in Settings.

If a plugin crashes Ovrtone, try Settings > "Run plugins in their own process" and tell us which
plugin it was.

Run it
------

Start Ovrtone from the Start menu.

The first launch reads every plugin you have, on the launch screen. That can take several minutes
with a big collection, and it happens once: after that only new or changed plugins are read, and
the program starts quickly. A plugin that crashes or stops responding is left out and remembered;
Rescan on the Plugins tab tries it again.

Windows may show "Windows protected your PC" (SmartScreen) because the program is not code
signed yet. Choose "More info", then "Run anyway".

Some antivirus programs (for example Norton, Avast, AVG or McAfee) block new unsigned
programs they have not seen before. Windows then says "Windows cannot access the specified
device, path or file" or "Access is denied". Restoring the file from the antivirus's
history is not always enough: add the Ovrtone folder to its exclusions. In Norton: Settings >
Antivirus > Scans and Risks > "Items to Exclude from Auto-Protect, Script Control, SONAR and
Download Intelligence Detection" > Configure > Add Folders. Then double-click ovrtone.exe again.

From a terminal:
  ovrtone.exe song.ovrtone          open a project
  ovrtone.exe drums.wav bass.flac   start a new project and import audio files
  ovrtone.exe --play                start playing right away
  ovrtone.exe --piano               open the Piano Roll on start

Audio files supported: WAV, AIFF, FLAC, MP3, CAF, Ogg Vorbis, M4A and MP4 (AAC, ALAC), ADPCM. You
can also drag audio files onto the window.

Requirements
------------

- Windows 10 or 11, 64-bit.
- A graphics driver with DirectX 12 or Vulkan (any recent Intel, AMD or NVIDIA driver).
- Sound goes through WASAPI (shared mode). Builds made with the asio feature can also use ASIO
  drivers: choose in Settings (Ctrl+,).

Where it keeps things
---------------------

- The list of recent projects: %APPDATA%\ovrtone\recent.txt
- Projects you open are unpacked to a temporary folder (%TEMP%\ovrtone-media) while they are
  open; it is cleaned up when you close the program.
- If it crashes, it prints the path of a crash report in %TEMP%.

Keyboard shortcuts
------------------

  Ctrl+N new    Ctrl+O open    Ctrl+S save    Ctrl+Shift+S save as
  Space play/stop    Ctrl+R repeat region    Delete delete selection
  I Inspector    X Mixer    Q quantize (Piano Roll)

Publisher: Bankz Sound Lab
Based on Pro Wave by the original authors: https://github.com/fabox-os/Pro-Wave
Licensed under GPL-3.0-or-later (see LICENSE).
