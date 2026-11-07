# What other DAWs do, and what Ovrtone can take from it

Source study of nine open-source projects, CPU first. Our own wording: no code was copied.

**How deep this went.** Ardour's graph and route code, Tracktion Engine's graph player and Audacity's waveform cache were read in part. Zrythm, LMMS, Giada and Qtractor got a narrower look at single files. MuSE and Rosegarden were only located: their file layout, not their logic. Nothing here was measured, so each gain is expected, not proven.

## Licences

Check each project's file headers before reusing any code. Studying ideas needs no licence; copying code does.

| Project | Licence file says | Reuse in Ovrtone (GPL-3.0-or-later) |
|---|---|---|
| Tracktion Engine | GPL3 or later, or commercial | Compatible under the GPL. Some files may differ, so check them |
| Giada | GPL 3 | Compatible if the headers agree |
| Ardour, LMMS, Qtractor, Rosegarden | GPL 2 | Only where headers say "or later". Not checked |
| Audacity | GNU GPL, version not read | Check the headers |
| MuSE | Version not found in the file | Check first |
| Zrythm | Its own licence, `LicenseRef-ZrythmLicense` | **Do not copy.** Read its terms before even close adaptation |

## CPU: what to change, in order

### 1. Skip plugins with nothing to do (small job, large gain in big sessions)

Ardour has separate code paths for silence and for routes that are not rolling. Ovrtone's `plugin.rs` has no silence check: `run_block` runs every plugin every 64 frames, even on a silent track.

Plan: when a plugin's input has been silent for longer than its tail, stop calling it and output silence, and wake it on the first sound. Instruments wake on a note. This is the Pro Tools list's dynamic plugin processing. The risk is plugins with long or unreported tails, so it needs an off switch per plugin.

### 2. Mix in blocks, not samples (large job, largest gain)

Every engine looked at works a buffer at a time: Ardour per cycle, Tracktion through pooled buffers, LMMS per period. Ovrtone's `core.rs` loops over each frame and runs each strip for it, which has two costs: a function call and its branches for every strip on every sample, and code the compiler cannot vectorize.

Plan: render each strip's block into a buffer, then apply gain and sum across whole buffers. Ardour also picks fast mixing routines for the processor at run time (`x86_functions_avx512f.cc`, `x86_functions_fma.cc`); in Rust, plain loops over slices often vectorize without that. Automation and fades then need to step inside a block without clicks. This is the largest change to the engine, and audio output must stay the same, so it is the one place to compare old and new renders with a test.

### 3. A graph for strips, once blocks exist

Ardour and Tracktion both run routes in parallel from a dependency graph: a node runs when its inputs are done, and finishing it may release the next. Ovrtone runs plugin blocks in parallel but the mix on one thread. With blocks, strips can join the pool, with Aux strips waiting for what feeds them.

### 4. Worker threads

- **Wake only what is needed.** Ardour counts the work available and the idle threads before waking any.
- **Choose how they wait.** Tracktion's thread pools can be swapped to change how threads wait. Ovrtone's workers spin and then park. Spinning eats CPU and laptop battery, so the wait should shorten when plugin load is low.

### 5. Reuse buffers

Tracktion prepares a pool of audio buffers sized from the graph before playback, so a node borrows one and hands it on. This keeps memory use and cache misses down once strips work in blocks.

## Other things worth taking

- **Waveforms for long files (Audacity, Qtractor).** Ovrtone keeps one overview of 4096 points per file, so a long recording turns blocky when zoomed in, and the file is decoded again each time it opens. Audacity keeps summaries at two sizes, per 256 and per 65536 samples. Qtractor builds a peak file on a background thread and keeps it on disk. Plan: two levels, saved beside the media.
- **Disk streaming (Ardour).** A background thread refills ring buffers in chunks, with the buffer at least two chunks long, and short fades at loop points to prevent clicks. Compare with `disk.rs` before changing anything: Ovrtone already streams on its own thread.
- **Swapping the graph (Ardour).** A new graph is built away from the audio thread and swapped in. Ovrtone's Command and Garbage queues already do this.
- **Quantize kinds (Rosegarden).** Separate basic, legato and notation quantizers exist. Located only, not read.

## Recording: Tracktion Engine and Reason

Tracktion Engine's recording source was read. Reason is closed source, so only its Reason 12 manual pages were read, and they may be out of date. Ideas only, in our own code.

| Idea | Seen in | Ovrtone |
|---|---|---|
| Monitoring that follows record arm | Tracktion, Reason | Added: Automatic input monitoring |
| Capture what the input heard without Record on | Tracktion | Added: Capture Recording now covers audio, up to 60 seconds |
| Check disk space around recording | Tracktion | Added: warns, refuses, or stops and keeps the take |
| Big input meter to read from across the room | Reason | Added: Recording Meter |
| Throw a take away and go back | Tracktion | Added: Discard Recording |
| Start a take when sound arrives | Tracktion | Added: Start recording when sound arrives |
| New take with the old one muted, or layered | Reason (Alt, Dub) | Already covered by take folders and New Take |

Reason's manual says it applies recording latency compensation only when not monitoring through Reason. Ovrtone always compensates. Not changed: the choice was not studied further.

## CPU: Reason

From the manual's performance page. Its DSP meter and load balancing are not described there.

- **Max audio threads.** Added as Plugin threads in Settings > Audio.
- **Render at the card's buffer size.** Reason offers this or a fixed 64 frames, and recommends the first for plugin-heavy work: larger batches are often more efficient. Ovrtone ran plugins in fixed blocks of 64 frames, one block of delay per plugin. Added: Plugin block size in Settings > Audio, read at start. Above 64, a strip's plugins run in series inside one block, so a strip is delayed by one block however many plugins it has (an instrument adds one more, and so does the output strip). 64 stays the default and behaves as before. Unlike Reason, a block of delay remains. Removing it would mean running plugins inside the mix itself. The gain has not been measured.

## Not studied yet

The piano roll and pattern code in LMMS, MIDI timing in MuSE, score code in Rosegarden, plugin sandboxing in Zrythm, session recovery in Ardour, and Giada's real-time scheduling.
