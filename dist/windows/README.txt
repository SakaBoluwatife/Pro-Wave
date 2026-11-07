Ovrtone - Windows x64
======================

Ovrtone is a DAW (music production app) in the style of Apple Logic Pro, written in Rust.
This is an early development build: expect missing features and rough edges.

Run it
------

1. Unzip the folder anywhere (it needs no installer and writes nothing outside your user
   folders).
2. Double-click ovrtone.exe.

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
  Y Library    I Inspector    X Mixer    Q quantize (Piano Roll)

Publisher: Bankz Sound Lab
Based on Pro Wave by the original authors: https://github.com/fabox-os/Pro-Wave
Licensed under GPL-3.0-or-later (see LICENSE).
