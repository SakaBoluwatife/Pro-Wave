# Feature overlap check (2026-11-12, 0.3.1)

Where two or more features do the same job. "Keep" means the overlap is deliberate (Logic has
it too, or the two work at different levels); "Merge" or "Remove" is a suggestion for the owner.

## Real duplicates

| Job | Where | Suggestion |
|---|---|---|
| Make a sample follow the tempo | Region menu > Fit to Tempo; Smart Tempo > Region Follows Project Tempo; Inspector > Sample BPM + Find; Audio Track Editor > Fit to Tempo; Alt-drag a region's end | The first four now run the same code. Remove "Region Follows Project Tempo" from the Smart Tempo submenu (Fit to Tempo is at the top of the same menu). Alt-drag stays: it sets the tempo by length. |
| Step-sequence drums | Channel Rack patterns (`song_pattern.rs`); Pattern Regions / Step Sequencer (`pattern.rs`, `step_editor.rs`); Drummer regions | Channel Rack and Pattern Regions both step-sequence one or more instruments. Make the Channel Rack the main way; either drop New Pattern Region or have it open the Channel Rack. Drummer (generated parts) is a different job: keep. |
| Turn a track or regions into audio | Record > Bounce In Place; File > Bounce > Regions in Place / Tracks in Place; Edit > Bounce and Join; Record > Commit Track; Freeze | Logic has all of these, but five entries in three menus is a lot. Keep Bounce in Place (regions), Bounce and Join, Freeze and Commit; remove Record > Bounce In Place (same as Regions in Place). |
| Bounce, Export, Import menus | File menu and Mix menu both have Bounce, Export and Import | Remove them from the Mix menu. |
| Revert | File > Revert to > Revert to Saved, Project History… | Keep: Project History lists every backup, Revert to Saved is the last save. |

## Same setting, several places (keep)

| Setting | Where |
|---|---|
| Region gain, fades, reverse | Inspector, Audio Track Editor bar, Smart Tool corners, Fade tool |
| Trim | Region edges in the Tracks area, Audio Track Editor handles |
| Normalize | Region menu > Normalize Region Gain (non-destructive, sets gain) vs Audio File Editor > Normalize (changes a copy of the file). Different levels; the labels already say which. |
| Automation menus | Mix > Show/Create/Convert Automation and View > Show Track/Region Automation |
| Editors | View > Show Piano Roll / Score / Step Sequencer / Mixer and Window > Piano Roll / Score Editor / Mixer |
| Quick Help | View > Show Quick Help and Help > Quick Help |
| Mastering | Mix > Mastering, Create Mastering Track, Show Master Track (one feature, three entries) |

## Code that is not compiled

Found while checking: these files are in `crates/ovrtone-app/src` but no `mod` declares them, so
they are not in the app. Both expect model fields that no longer exist (`Project::audio_pool`,
`AltSnapshot`).

| File | Feature | State |
|---|---|---|
| `project_alternatives.rs` | Project Alternatives (File > Alternatives) | Unfinished; the menu entries are greyed |
| `project_audio.rs` | Project Audio Browser | Unfinished |

`audio_file_editor.rs` was in the same state; it is now wired in (0.3.1), opened from the Audio
Track Editor.
