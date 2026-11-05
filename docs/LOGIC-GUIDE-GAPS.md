# Logic guide comparison

Pro Wave compared with the chapters of the Logic Pro for Mac User Guide (1,324 pages), to decide what to improve. Written in our own words. The guide is a checklist of features, not a source of text or artwork.

This compares chapter headings against the feature map and a search of the code. "Have" does not mean it matches Logic in depth, and nothing here has been checked by hand. Nothing is removed by this plan.

## Skip for now: stays greyed or unbuilt

The owner said not to build these:

- Session Player, Performers
- Song to Mobile Device, Open in Sound Library Manager

Proposed, for the owner to confirm:

- Logic Remote, GarageBand and iPad project sharing, AirDrop, Music app and Voice Memos: Apple-only
- Surround and binaural panning: a large job for a stereo workflow
- Apple Loops library features: there is no stock library, though users' own loops still work
- Live Loops, full score engraving (staff styles, score sets, parts): large, and outside the priorities so far

## Improve: built, but shallower than Logic

| Area | Today | Better |
|---|---|---|
| Comping | Audio takes from Cycle recording | Takes when recording over a region; New Take, Delete Take |
| Recording | Autopunch, one punch a pass | Record Repeat, replace mode, drag punch locators |
| Fades | Fade tool and region values | Curve shapes on the region, crossfade editing |
| Automation | Lanes in a window, region automation | Curves on the track itself in the Tracks area, select and move groups of points, snap to grid, Trim |
| Smart Controls | Eight knobs per plugin, choices not saved | Save choices with the project, rename, mapping ranges |
| Plugins | Hosted, presets, automation | Sidechain input, compare A/B, plugin window header with bypass and presets |
| Channel strips | Settings files, copy and paste | Browse saved settings from the Setting menu, include sends |
| Mixer | Groups, VCA, routing matrix | Reorder strips by dragging, I/O labels |
| Piano roll | FL-style tools | Time selection, automation area under the notes, several regions at once |
| Tempo | Tempo track, Smart Tempo | Beat mapping by hand, tempo curves between points |
| Flex | Modes, quantize, pitch correction | A proper Flex view, Varispeed |
| Stem Splitter | CPU helper, not proven yet | Stop button, progress, then speed |
| Metering | Peak meters, pre-fader | Clip indicators with reset, input format choice per strip |

## Build next: not found in the code

Most useful for daily mixing and production first:

1. **Track alternatives:** other versions of a track's regions, switched from its header.
2. **Selection-based processing:** run plugins on part of a region and print the result.
3. **Snap to zero crossings** when cutting and trimming audio.
4. **Strip Silence:** cut silent passages out of a region.
5. **Project Audio Browser:** every file in the project, with unused ones marked.
6. **MIDI learn:** assign hardware controls to knobs and faders.
7. **Audio File Editor:** destructive edits on a file, after a backup.
8. **Chord track and arrangement markers:** move whole song sections.
9. **Groove templates:** make a quantize grid from a region.
10. **Screensets:** save and recall window layouts.
11. **Tuning:** Project Settings' page, the one still greyed there.
12. **Project Alternatives and backups:** named versions of a project.

## Smaller entries still greyed

Quick: About Pro Wave, Logic-style Key Commands (open the tab in Settings).

Later: Customize Toolbar, Advanced Tools, Print and Page Setup, MusicXML, Movie and Video, Create New Output Channel Strip.

## Suggested order

1. The owner's test results and the recording bug.
2. Plugin process isolation, as already decided.
3. Automation on the tracks, Track Alternatives, New Take and Record Repeat.
4. Zero crossings, Strip Silence, Project Audio Browser, selection-based processing.
5. Sidechain, MIDI learn.
6. Pro Tools list, on the build list by the owner's choice, once the three sessions now running are done: dynamic plugin processing first, then nudge and tab to transient, then scrub, clip groups and loops, commit, bus interrogation, mono sum and dim.
7. Everything else in the lists above.

## Pro Tools guide: what to note
From the chapter list of the Pro Tools Reference Guide 2023.3 (1,565 pages) and a search of the code by keyword. A word not found in the code does not prove a feature is missing, so check each one before building it. Our own wording.
Not found, and useful for editing and mixing:
- **Tab to transient:** move the cursor to the next hit in an audio clip.
- **Nudge:** move clips or their edges by a set amount from the keyboard.
- **Scrub:** drag over audio to hear it.
- **Clip groups and clip loops:** treat several clips as one, and repeat a clip by dragging.
- **Dynamic plugin processing:** plugins with nothing to process stop using CPU. This fits the CPU priority.
- **Commit:** print a track through its plugins to a new track and keep the original. Freeze exists; this goes one step further.
- **Bus interrogation:** select a bus and see what feeds it.
- **Monitor controls:** mono sum and dim on the output, for checking a mix.
Already on the list above under another name: playlists (track alternatives), AudioSuite (selection-based processing), memory locations (markers and screensets), Elastic Audio (Flex).
Skip: Beat Detective (the owner's choice), Satellite, video, field recorders, machine control, HEAT, Track Collaboration, DSP mode, Avid Link.
## Spatial audio, for later
The owner wants Atmos left open. `libspatialaudio` (https://github.com/videolan/libspatialaudio) is a candidate: C++, ambisonics to 3rd order, objects, speaker layouts, binaural over headphones, and ADM rendering. LGPLv2.1 or later, or a commercial licence.
- **What it is not:** its page does not mention Dolby Atmos or say it reads or authors Atmos files. Calling a feature Atmos, and working with Dolby's formats, may need Dolby's own terms. Check before using the name.
- **What it needs here:** the mix is stereo throughout, so strips, buses, panners and meters must first carry more channels. That is the large part, whichever library renders it.
- **If adopted:** read its licence file first, and keep the rendering outside the audio thread's allocations.
6. Everything else in the lists above.
## Built since this comparison (compiled, not yet tried by hand)
- Automation curves on the track: A button, A key, points, groups, snap.
- Record Repeat, Replace Mode, New Take, Delete Take, takes when recording over an audio region.
- Track alternatives, from a button on the track header.
- Snap to Zero Crossings, in the Snap and Edit menus.
- Strip Silence, in the Edit and Functions menus and on the region.
- Chord track and Arrangement track: drag a section to move it, with Ctrl to copy it.
