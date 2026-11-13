# Bankz Sound Lab Plugins: Study Before Building

What each plugin in the reference set does, the real signal processing behind it, what separates a
good one from a forgettable one, and what ours can do better because it runs inside Ovrtone. Written
2026-11-12. The plugins come out as CLAP and VST3 and run in any DAW. Inside Ovrtone they also learn
from the session itself (see section 2).

The names below describe the job of each plugin, not any product. No artwork, names or wording from
any other maker are to be copied.

---

## 0. The rule against "AI slop"

A plugin is not finished because it makes a sound. Each one ships only when it meets a written spec,
measured, not assumed:

| Check | Bar |
|---|---|
| Neutral settings | Null to below -120 dBFS against the dry signal (or report its fixed latency and colour) |
| Aliasing | Any nonlinear stage: a 0 dBFS sine sweep to 20 kHz at 44.1 kHz shows aliases below -90 dB in the audible band (oversampling and/or antiderivative anti-aliasing) |
| Latency | Reported exactly to the host for delay compensation; constant, never changing while playing |
| Parameters | Every move smoothed (no zipper noise), every one automatable, and sample-rate independent (same sound at 44.1 to 192 kHz) |
| Real time | No allocation, locks or file access in the audio thread; denormals flushed; NaN and Inf guarded |
| CPU | A budget per instance, written down, measured at a 64-sample buffer |
| State | Versioned presets and project state: an old session always reopens exactly |
| Listening | Blind A/B against a reference tool on the same material, by the owner, before release |

Each plugin gets a one-page spec sheet with these numbers. If a number is missed, it does not ship.

---

## 1. The plugins, grouped by the engine underneath

Many plugins share one core. Building the cores well once is what makes the whole family good.

### Core A: filters and EQ

**Channel EQ, Group/Bus EQ, Graphic-style EQ**
- *How it works:* cascaded second-order sections (biquads) for bells, shelves and cuts. Use the
  topology-preserving-transform state-variable filter (Zavalishin, *The Art of VA Filter Design*):
  it stays stable and smooth when the frequency is automated quickly. Plain direct-form biquads
  click and blow up under fast modulation.
- *Quality markers:*
  - Analog-matched response near Nyquist. A plain bilinear transform "cramps" bells above 10 kHz.
    Use Vicanek's matched second-order designs, or oversample the EQ.
  - Optional linear phase (FIR by FFT convolution, with the latency reported).
  - Optional "dynamic" bands: each band's gain driven by its own envelope follower.
  - A real-time analyser (STFT with a proper window and averaging) behind the curve.
- *Pitfalls:* coefficient jumps on automation (recompute per sample or per small block, with
  smoothing), and gain staging (auto-gain after the EQ, as an option).

**Rhythmic filter (tempo-synced filter movement)**
- A resonant filter (Moog-style ladder or state-variable filter) whose cutoff follows a drawn or
  step pattern synced to the song.
- *Quality:* zero-delay-feedback ladder (no tuning error at high resonance); saturation inside the
  feedback loop for the classic character; pattern steps smoothed with a short glide.

### Core B: dynamics

**Compressor, multiband compressor, multiband "character" processor**
- *How it works:* level detection (peak or RMS, linked or unlinked across channels), a gain
  computer (threshold, ratio, soft knee), and attack/release smoothing in the log domain, branching
  or decoupled. Giannoulis, Massberg and Reiss, *Digital Dynamic Range Compressor Design: A Tutorial
  and Analysis* (JAES 2012) is the reference: the decoupled-smoothing designs avoid the "pumping
  artefacts" of naive designs.
- *Multiband:* Linkwitz-Riley crossovers (4th-order; the bands sum flat in magnitude), or a
  linear-phase FIR crossover with its latency reported.
- *Character:* per-band saturation after the gain stage (Core C).
- *Quality markers:*
  - Program-dependent release (two time constants blended by how long the signal has stayed over
    threshold, as the classic opto and VCA designs behave).
  - An external sidechain with its own filter.
  - Lookahead with the latency reported.
  - A gain-reduction trace drawn over the input waveform (the big visual of this family).

**True-peak limiter**
- *How it works:* lookahead peak limiting with a smooth gain curve (gain computed on a lookahead
  window, then smoothed, for example by a moving minimum and windowed attack).
- True peak by ITU-R BS.1770: detect on the signal oversampled 4× (inter-sample peaks).
- *Quality:* no distortion on bass (release long enough not to follow single cycles); output never
  above the ceiling in true-peak terms (measured with a BS.1770 meter); a loudness readout (LUFS
  integrated, short term, LRA).

**Volume shaper ("pump") and ducking**
- A drawn gain curve, one bar or one beat long, repeated in sync with the song, or triggered by a
  sidechain hit.
- *Quality:* sample-accurate sync to the host's position (not to the plugin's own clock); curve
  smoothing to avoid clicks at sharp corners.

**De-esser**
- *How it works:* split-band. A detector tuned to sibilance (5 to 10 kHz, adaptive) drives gain
  reduction on that band only (dynamic EQ), not on the whole signal.
- *Better:* spectral de-essing (STFT: reduce only the bins that rise above a smoothed spectral
  envelope during sibilant frames), which keeps the air of the voice. Sibilance detection by
  zero-crossing rate plus spectral centroid avoids triggering on hi-hats bleeding into the mic.

**Vocal level rider**
- *How it works:* tracks the vocal's short-term loudness (BS.1770 K-weighting over about 400 ms)
  against a target, and writes small, slow gain moves before the compressor, so the compressor works
  less.
- *Better:* the target follows the loudness of the backing music, so the vocal sits at a set
  distance above the beat, not at a fixed level. Silence and breaths are recognised and left alone.

### Core C: saturation and nonlinear tone

**Character distortion, saturation, tape, "finisher"**
- *How it works:* static waveshapers (tanh, asymmetric curves for even harmonics), or better,
  models with memory (hysteresis for tape, a clipping stage with its own filtering).
- *The quality line:*
  - **Anti-aliasing** by oversampling (polyphase halfband filters, 2× to 16×) and/or
    antiderivative anti-aliasing (ADAA, Parker, Zavalishin and Le Bivic, DAFx 2016), which removes
    most aliasing at 1× or 2×.
  - **Level-dependent tone:** a pre-emphasis filter before the shaper and the matching
    de-emphasis after it, so the distortion lives where real gear's does.
- **Mix finisher** (a "one more polish" plugin): a gentle chain of tilt EQ, glue compression, soft
  saturation, width and a limiter, with one macro control that moves all of them along a curve
  tuned by ear. Its worth is in the tuning of the macro, not the parts.

### Core D: time and space (delay lines, modulation)

**Delay**
- Fractional delay lines (cubic or Thiran allpass interpolation for clean modulated delay).
- Feedback path with a filter and saturation (tape and analog character), tempo sync, ping-pong
  and stereo offset, ducking by the dry signal.
- *Quality:* no zipper or pitch glitches when the time changes. Crossfade between two read heads,
  or glide like tape, as the user chooses.

**Chorus, phaser, flanger**
- Chorus and flanger: modulated delay lines (several voices, with spread LFO phases).
- Phaser: a chain of first-order allpass stages swept by an LFO, with feedback.
- *Quality:* bucket-brigade-style low-pass and noise for vintage modes; the LFO shape and rate
  smoothing matter more than the voice count.

**Reverb**
- *How it works:* a feedback delay network (Jot, Stautner and Puckette), using a unitary feedback
  matrix (Hadamard or Householder), with modulation to break up metallic ringing, early reflections
  from a tapped delay line, and frequency-dependent decay (a filter per line). A Dattorro plate
  (JAES 1997) for plates.
- *Quality markers:* decay times per band that match what the display says; no ringing on
  transients; modulation that is not audible as chorus; stereo decorrelation; freeze; and low CPU
  so it can sit on many buses.
- **Reverse riser** (reverse vocal swell): reverse the audio (offline or in a buffer, a beat or a bar
  ahead), reverb it, reverse it back, and place it so it ends on the downbeat. The DAW's help
  matters here (section 2).

**Doubler**
- *How it works:* one to four copies of the voice, each with a slowly drifting small delay (5 to
  40 ms), a tiny pitch drift (a few cents, by a smooth random walk), a little EQ difference, and
  panned apart.
- *Better:* true doubling varies the timing of each phrase, not the whole track at a fixed offset.
  An onset detector finds the phrases and nudges each copy's timing a little differently, as a
  second take would.

**Stereo imager**
- Mid/side processing per band (Linkwitz-Riley split), width by side gain, a mono bass-maker below a
  set frequency, and decorrelation for mono sources (allpass networks; not just a Haas delay, which
  combs in mono).
- *Quality:* a correlation meter and a goniometer; mono-compatibility checked live (warn when width
  causes cancellation).

### Core E: spectral (STFT) processing

**Harsh resonance smoother**
- *How it works:* per short frame, compare each bin's level with a smoothed spectral envelope (the
  "expected" spectrum, by cepstral smoothing or a moving median across bins), and reduce only the
  bins sticking out, with attack/release per bin. In effect a dynamic EQ with hundreds of bands that
  places itself.
- *Quality:* phase-coherent overlap-add (no smearing at fast settings); a sidechain/delta listen
  mode (hear only what is removed); transient-aware so drum attacks are not dulled.

**Noise suppression, and "AI" vocal cleanup**
- *Classic:* spectral subtraction and Wiener filtering with a noise profile learnt from a quiet
  stretch; the MMSE-STSA estimator (Ephraim and Malah, 1984) gives less "musical noise".
- *Learned:* hybrid DSP plus a small neural network predicting band gains (RNNoise, Valin 2018,
  BSD licence) runs in real time on a CPU, with no GPU.
- For reverb and noise removal on voice, a trained model is the state of the art. If we do one, it
  is trained on licensed data and runs on the CPU, with its latency reported. Be honest in the UI
  about what it is.

**Vocoder**
- *Channel vocoder:* a bank of band-pass filters (16 to 64 bands) on the voice (modulator)
  measures envelopes that set the levels of the same bands on the carrier (synth or noise).
- *Quality:* sibilance pass-through (high band from the voice itself, so consonants stay
  intelligible), formant shift by moving the carrier bands, and a built-in carrier synth playing
  from MIDI.

### Core F: pitch (the hardest family)

**Pitch detection** (used by tuner, pitch editor, harmony, key detector and doubler)
- *How it works:* YIN (de Cheveigné and Kawahara, JASA 2002) and its probabilistic version pYIN
  (Mauch and Dixon, ICASSP 2014), which tracks voiced/unvoiced with an HMM. That makes it much
  steadier on real vocals.
- Modern options: small neural pitch trackers (CREPE-style) are more robust in noise, but heavier.
  Start with pYIN; consider a model later.

**Pitch correction (real-time auto-tune)**
- *How it works:* detect the pitch, choose the target note (from a key or scale, or MIDI), and
  shift by the difference with a speed control (retune time). Shift by PSOLA (pitch-synchronous
  overlap-add, natural for monophonic voice and formant-preserving by nature), with very low
  latency.
- *Quality markers:* no warble on vibrato (vibrato recognised and kept, not flattened); natural
  transitions between notes; humanise; formant kept.

**Pitch and formant shifter**
- PSOLA for voice, or a phase vocoder with phase locking (Laroche and Dolson, 1999) for polyphonic
  material; formant handled separately by the spectral envelope (cepstral or LPC envelope:
  remove it, shift the pitch, put back a shifted or unshifted envelope).
- We already have Signalsmith Stretch (MIT) in Ovrtone for polyphonic shifting.

**Harmony generator**
- Pitch shift copies of the voice to intervals that stay inside the key, or to chords from MIDI,
  each with its own small timing and pitch humanising (as the doubler does).

**Pitch editor (Melodyne-style)**
- *How it works:* an offline analysis of the whole recording.
  - Note segmentation: onsets, plus pitch continuity from pYIN.
  - Each note becomes a "blob" with its centre pitch, its pitch drift curve, its vibrato, its
    amplitude, its formant and its sibilants.
  - Edits re-render that note with PSOLA, or with a phase vocoder plus envelope for polyphonic
    material.
- *What makes it great:*
  - Accurate segmentation, and an easy way to correct it (split and join).
  - Pitch drift and vibrato as separate controls.
  - Sibilant and breath handling (not pitch-shifted).
  - Formant independent of pitch.
  - Fast, artefact-free rendering of only the notes changed.
  - Polyphonic mode (detecting notes in chords) is research-level. Start monophonic, and say so.
- It needs the whole region, not a live stream. That is what ARA gives a plugin, and Ovrtone
  already hosts ARA.

**Key detector**
- *How it works:* a chromagram (energy per pitch class, from a constant-Q or STFT spectrum,
  tuning-corrected), averaged over the song, correlated with major/minor key profiles
  (Krumhansl-Schmuckler, or the Temperley and Albrecht profiles). The best match wins, with a
  confidence score.
- *Better:* report the key per section (songs modulate), and the tuning offset (A = 432 vs 440).

**Vocal alignment (aligning doubles and harmonies to the lead)**
- *How it works:* features per frame (MFCC or energy and onsets), dynamic time warping (DTW)
  against a guide take, then time-warp the dub along the path (pitch-preserving stretch).
- *Better:* align only where the timing is off by more than a threshold, and keep consonant
  attacks aligned tightly while letting vowels breathe.

### Core G: tempo-synced creative tools

**Beat repeater and stutter; one-knob beat roll**
- A tempo-synced capture buffer: on trigger (a step pattern, MIDI or a knob), it captures 1/4 to
  1/64 of a beat and repeats it, with an optional pitch ramp, gate and filter.
- *Quality:* grid-accurate capture (from the host's transport, not wall time), short crossfades at
  slice edges, and swing.
- *One-knob versions:* a single macro that moves rate, gate, filter and mix along a curve tuned by
  ear.

**One-knob riser and sweep**
- One macro moving filter, reverb send, noise layer, pitch-up, width and level together.
- Value comes from the tuned curve and from syncing to the bar (section 2).

### Core H: the host and the stream

**Plugin host (a chain or rack in a plugin)**
- Hosting CLAP/VST3 inside a plugin. Ovrtone already has this code (`ovrtone-host`), but it is
  GPL. A closed plugin host must be written fresh or under permission.

**DAW streaming** (sending the mix to a client in real time)
- Low-latency network audio: Opus at 48 kHz, WebRTC or a web page the client opens. Includes
  talkback and a lossless option. This is a network service product as much as a plugin.

### The "complete chain" and "AI mastering" products

- A complete vocal chain or mastering chain is mostly the cores above, plus good macro tuning.
- **Mastering assistant:** analyse the loudness (BS.1770), the spectral balance (long-term average
  spectrum against genre reference curves we measure ourselves from licensed references) and the
  dynamics and width. Then set a chain (EQ, multiband, saturation, width and limiter) toward a
  target, and show every setting so the user can change it. Honest assistant, not a black box.

---

## 2. Where pairing with Ovrtone gives an advantage

Any plugin only hears its own input. Inside Ovrtone, ours can also know the session. The plan is a
small, documented **Ovrtone extension**: a CLAP extension, with a VST3 interface for the same thing.
Our plugins ask for it; in other DAWs it is absent and they fall back to working alone.

What the DAW can hand a plugin, from what Ovrtone already has:

| Ovrtone knows | Which plugins use it | What it gives |
|---|---|---|
| The project key and scale, the chord track, the tempo map and the signature | Tuner, harmony, pitch editor, vocoder, rhythmic filter, beat roll, riser | The tuner and harmoniser follow the song's chords bar by bar with no setup; synced effects follow tempo changes exactly |
| Song sections (arrangement and marker track) | Riser, sweep, reverse riser, beat repeater, finisher | "Rise into the chorus": a riser that knows where the chorus starts and ends there, with no drawing |
| Each track's role (Organize Mix Session: Lead Vocal, Drums, Bass…) | EQ, compressor, rider, de-esser, resonance smoother, mastering | Sensible starting points per role, without presets to hunt |
| Other tracks' spectra and loudness (cheap summaries the DAW already measures) | EQ ("unmask the vocal from the beat"), rider (sit the vocal above the music), imager, mastering | Masking-aware EQ, and a rider that hears the beat without any sidechain routing |
| The whole region as audio, with its position (ARA, already hosted) | Pitch editor, alignment, key detector, reverse riser, noise cleanup | Edits on the timeline; alignment to the lead vocal track by name; the reverse riser built from the region ahead |
| Clip gain lines and automation | Rider | The rider writes its moves as clip gain or automation the user can see and edit, not hidden inside the plugin |
| Spatial objects (position, size) | Reverb, doubler, imager | Reverb and doubles placed in the 3D mix to match the source |
| Render-ahead and the Performance window | Heavy spectral and pitch plugins | They can use more lookahead on playback tracks, and report their cost in Ovrtone's Performance window |
| Ovrtone's own effects | Every plugin | Our plugins replace them over time with no change in projects (the same parameter IDs where it matches) |

Two rules keep this honest:
1. Every plugin is complete and good in any DAW. The extension adds convenience and intelligence;
   it never decides whether the plugin works.
2. The extension is specified as a public document, so the plugins do not depend on GPL code, and
   other hosts could adopt it.

---

## 3. Order of work

The shared cores come first, then the plugins that need the fewest cores, then the hard ones.

1. **Framework and UI kit:**
   - CLAP and VST3 wrappers (licence-checked), parameters, smoothing, state and versioning, presets
     and A/B.
   - The family UI kit: top bar, knobs, value labels, meters and themes. Each plugin has its own
     style on shared bones.
   - The measurement harness for the spec sheets (section 0): null tests, aliasing sweeps, latency
     and CPU.
2. **Cores A and B, then plugins:** Channel EQ, Compressor, True-Peak Limiter, De-esser, Volume
   Shaper.
3. **Cores C and D, then plugins:** Saturation, Delay, Chorus/Phaser, Reverb, Doubler, Imager.
4. **Analysis:** Key detector, Vocal level rider, Multiband, Mix finisher.
5. **Creative:** Beat repeater, Beat roll, Riser, Sweep, Rhythmic filter, Reverse riser.
6. **Core E, then plugins:** Resonance smoother, Noise suppression, Vocoder.
7. **Core F (pitch), then plugins:** Tuner, Pitch and formant shifter, Harmony, Alignment, then the
   Pitch editor (ARA).
8. **Assistants and services:** Mastering assistant, AI vocal cleanup (needs licensed training
   data), Plugin host (fresh code), Streaming.
9. **The Ovrtone extension**, used from step 2 on as each plugin ships.

## 4. Decisions to make before step 1

- **Repository and licence:** a separate, closed repository for the plugins. Nothing copied from
  the GPL DAW unless it is the owner's own code; get legal advice on reusing the Ovrtone effects.
- **Formats:** CLAP (clack, MIT/Apache) and VST3. Check the licence of the VST3 route chosen
  (Steinberg's current SDK terms; the `vst3-sys` crate is GPL and must be avoided for closed
  plugins).
- **UI:** egui (fast to build, already known), or a custom GPU renderer for richer finishes.
- **Brand:** the plugin family's name, and each plugin's own name.
- **Copy protection, if any:** licence keys are a business decision that changes the plugin code.

## References

- U. Zölzer (ed.), *DAFX: Digital Audio Effects*, 2nd ed., Wiley 2011.
- V. Zavalishin, *The Art of VA Filter Design*, Native Instruments, rev. 2.1.2.
- D. Giannoulis, M. Massberg, J. Reiss, "Digital Dynamic Range Compressor Design: A Tutorial and
  Analysis", JAES 60(6), 2012.
- ITU-R BS.1770, "Algorithms to measure audio programme loudness and true-peak audio level".
- J. Parker, V. Zavalishin, E. Le Bivic, "Reducing the Aliasing of Nonlinear Waveshaping Using
  Continuous-Time Convolution", DAFx 2016.
- J.-M. Jot, A. Chaigne, "Digital Delay Networks for Designing Artificial Reverberators", AES 1991;
  J. Dattorro, "Effect Design Part 1: Reverberator and Other Filters", JAES 1997.
- A. de Cheveigné, H. Kawahara, "YIN, a fundamental frequency estimator for speech and music", JASA
  2002; M. Mauch, S. Dixon, "pYIN", ICASSP 2014.
- J. Laroche, M. Dolson, "Improved Phase Vocoder Time-Scale Modification of Audio", IEEE TSAP 1999.
- Y. Ephraim, D. Malah, "Speech enhancement using a minimum mean-square error short-time spectral
  amplitude estimator", IEEE TASSP 1984.
- J.-M. Valin, "A Hybrid DSP/Deep Learning Approach to Real-Time Full-Band Speech Enhancement"
  (RNNoise), 2018.
- C. Krumhansl, *Cognitive Foundations of Musical Pitch*, Oxford 1990 (key profiles).
