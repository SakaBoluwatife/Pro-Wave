//! A small built-in sample player: one audio file played from the track's notes, at the pitch of
//! each note against C4 (FL Studio: a channel holding a sample; Logic: Quick Sampler). It is not a
//! sound library, only a way to play the user's own sample from the piano roll, and it goes in an
//! instrument slot like any plugin.
//!
//! A short sample plays to its end whatever the note's length, as a drum hit should. A longer one
//! stops, with a short fade, when the note does.
use crate::{HostError, Loaded, LoadedPlugin};
use prowave_engine::{AudioFile, PluginEvent, PluginEventKind, PluginProcessor, PluginTransport};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Notes sounding at once; past this the oldest gives way.
const VOICES: usize = 32;
/// The note that plays the sample as recorded.
const ROOT_KEY: i32 = 60;
/// Samples up to this long play to their end.
const ONE_SHOT_SECONDS: f64 = 2.0;
/// How long a held note takes to fade when let go.
const RELEASE_SECONDS: f64 = 0.012;

#[derive(Clone, Copy, Default)]
struct Voice {
    active: bool,
    releasing: bool,
    key: u8,
    /// Where it is in the sample, in frames.
    position: f64,
    /// Frames of the sample per frame of output.
    step: f64,
    gain: f32,
    envelope: f32,
    /// Counts up, so the oldest voice can be told.
    age: u64,
}

struct SamplerProcessor {
    /// Interleaved stereo.
    samples: Arc<Vec<f32>>,
    /// Frames of the sample per frame of output at the root key.
    base_step: f64,
    one_shot: bool,
    release_per_frame: f32,
    voices: [Voice; VOICES],
    started: u64,
    released: Arc<AtomicBool>,
}

impl SamplerProcessor {
    fn note_on(&mut self, key: u8, velocity: f32) {
        // A free voice, or else the one that began longest ago.
        let at = self
            .voices
            .iter()
            .position(|v| !v.active)
            .or_else(|| {
                self.voices
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, v)| v.age)
                    .map(|(i, _)| i)
            })
            .unwrap_or(0);
        self.started += 1;
        let semitones = (i32::from(key) - ROOT_KEY) as f64;
        self.voices[at] = Voice {
            active: true,
            releasing: false,
            key,
            position: 0.0,
            step: self.base_step * (semitones / 12.0).exp2(),
            gain: velocity.clamp(0.0, 1.0),
            envelope: 1.0,
            age: self.started,
        };
    }

    fn note_off(&mut self, key: u8) {
        if self.one_shot {
            return;
        }
        for v in self.voices.iter_mut().filter(|v| v.active && v.key == key) {
            v.releasing = true;
        }
    }

    /// Adds one frame of every voice to (l, r).
    #[inline]
    fn render(&mut self) -> (f32, f32) {
        let frames = self.samples.len() / 2;
        let (mut l, mut r) = (0.0, 0.0);
        for v in self.voices.iter_mut().filter(|v| v.active) {
            let at = v.position as usize;
            if at + 1 >= frames {
                v.active = false;
                continue;
            }
            let t = (v.position - at as f64) as f32;
            let s = &self.samples;
            let gain = v.gain * v.envelope;
            l += (s[at * 2] + (s[at * 2 + 2] - s[at * 2]) * t) * gain;
            r += (s[at * 2 + 1] + (s[at * 2 + 3] - s[at * 2 + 1]) * t) * gain;
            v.position += v.step;
            if v.releasing {
                v.envelope -= self.release_per_frame;
                if v.envelope <= 0.0 {
                    v.active = false;
                }
            }
        }
        (l, r)
    }
}

impl PluginProcessor for SamplerProcessor {
    fn process(
        &mut self,
        input: [&[f32]; 2],
        output: [&mut [f32]; 2],
        events: &[PluginEvent],
        _transport: &PluginTransport,
    ) {
        let [out_l, out_r] = output;
        let mut next = 0;
        for frame in 0..out_l.len() {
            while let Some(e) = events.get(next).filter(|e| e.frame as usize <= frame) {
                match e.kind {
                    PluginEventKind::NoteOn { key, velocity } => self.note_on(key, velocity),
                    PluginEventKind::NoteOff { key } => self.note_off(key),
                    PluginEventKind::Control { .. } | PluginEventKind::PitchBend { .. } => {}
                }
                next += 1;
            }
            let (l, r) = self.render();
            out_l[frame] = l + input[0][frame];
            out_r[frame] = r + input[1][frame];
        }
    }

    fn latency_samples(&self) -> usize {
        0
    }
}

impl Drop for SamplerProcessor {
    fn drop(&mut self) {
        self.released.store(true, Ordering::Release);
    }
}

/// The UI side has nothing to do yet: the sound lives in the audio half.
struct LoadedSampler {
    released: Arc<AtomicBool>,
}

impl LoadedPlugin for LoadedSampler {
    fn latency_samples(&self) -> usize {
        0
    }

    fn released(&self) -> bool {
        self.released.load(Ordering::Acquire)
    }

    fn tick(&mut self) {}

    fn editor_is_open(&self) -> bool {
        false
    }

    fn take_closed_position(&mut self) -> Option<(i32, i32)> {
        None
    }

    fn open_editor(
        &mut self,
        title: &str,
        _owner: Option<isize>,
        _position: Option<(i32, i32)>,
    ) -> Result<(), HostError> {
        Err(HostError(format!(
            "{title} plays a sample and has no window yet"
        )))
    }

    fn close_editor(&mut self) {}

    fn save_state(&mut self) -> Result<Option<Vec<u8>>, HostError> {
        Ok(None)
    }

    fn load_state(&mut self, _bytes: &[u8]) -> Result<(), HostError> {
        Ok(())
    }

    fn take_dirty(&mut self) -> bool {
        false
    }
}

/// Reads a sample for the engine's sample rate. Failure is an error to show, never a panic.
pub fn load_sampler(path: &Path, sample_rate: u32) -> Result<Loaded, HostError> {
    let name = path.display();
    let file = AudioFile::open(path).map_err(|e| HostError(format!("cannot load {name}: {e}")))?;
    let samples = file
        .read_stereo()
        .map_err(|e| HostError(format!("cannot read {name}: {e}")))?;
    if samples.len() < 4 {
        return Err(HostError(format!("{name} is empty")));
    }
    let rate = f64::from(sample_rate.max(1));
    let released = Arc::new(AtomicBool::new(false));
    let audio = SamplerProcessor {
        samples: Arc::new(samples),
        base_step: f64::from(file.sample_rate.max(1)) / rate,
        one_shot: file.duration_secs() <= ONE_SHOT_SECONDS,
        release_per_frame: (1.0 / (RELEASE_SECONDS * rate)) as f32,
        voices: [Voice::default(); VOICES],
        started: 0,
        released: released.clone(),
    };
    Ok((Box::new(LoadedSampler { released }), Box::new(audio)))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use prowave_engine::PLUGIN_BLOCK;

    const RATE: u32 = 48_000;

    fn wav(dir: &Path, seconds: f64) -> std::path::PathBuf {
        let path = dir.join("hit.wav");
        let frames = (seconds * f64::from(RATE)) as usize;
        prowave_engine::write_wav(&path, &vec![0.5; frames * 2], RATE).unwrap();
        path
    }

    /// Runs blocks, with `events` in the first, and gives how many frames made a sound.
    fn sounding(
        audio: &mut Box<dyn PluginProcessor>,
        events: &[PluginEvent],
        blocks: usize,
    ) -> usize {
        let silence = vec![0.0; PLUGIN_BLOCK];
        let (mut l, mut r) = (vec![0.0; PLUGIN_BLOCK], vec![0.0; PLUGIN_BLOCK]);
        let mut count = 0;
        for b in 0..blocks {
            let now = if b == 0 { events } else { &[] };
            assert_no_alloc::assert_no_alloc(|| {
                audio.process(
                    [&silence, &silence],
                    [&mut l, &mut r],
                    now,
                    &PluginTransport::default(),
                );
            });
            assert!(l.iter().chain(&r).all(|x| x.is_finite()));
            count += l.iter().filter(|x| x.abs() > 1e-4).count();
        }
        count
    }

    fn on(frame: u32, key: u8) -> PluginEvent {
        PluginEvent {
            frame,
            kind: PluginEventKind::NoteOn { key, velocity: 1.0 },
        }
    }

    fn off(key: u8) -> PluginEvent {
        PluginEvent {
            frame: 0,
            kind: PluginEventKind::NoteOff { key },
        }
    }

    #[test]
    fn it_is_silent_until_a_note_and_a_note_plays_the_sample_at_its_pitch() {
        let dir = tempfile::tempdir().unwrap();
        let (_, mut audio) = load_sampler(&wav(dir.path(), 0.01), RATE).unwrap();
        assert_eq!(sounding(&mut audio, &[], 4), 0);
        // 480 frames at the root key.
        let root = sounding(&mut audio, &[on(0, 60)], 20);
        assert!((478..=480).contains(&root), "{root}");
        // An octave up plays twice as fast, so half as long; an octave down twice as long.
        let up = sounding(&mut audio, &[on(0, 72)], 20);
        assert!((238..=241).contains(&up), "{up}");
        let down = sounding(&mut audio, &[on(0, 48)], 40);
        assert!((958..=961).contains(&down), "{down}");
    }

    #[test]
    fn a_note_starts_at_its_frame() {
        let dir = tempfile::tempdir().unwrap();
        let (_, mut audio) = load_sampler(&wav(dir.path(), 1.0), RATE).unwrap();
        let silence = vec![0.0; PLUGIN_BLOCK];
        let (mut l, mut r) = (vec![0.0; PLUGIN_BLOCK], vec![0.0; PLUGIN_BLOCK]);
        audio.process(
            [&silence, &silence],
            [&mut l, &mut r],
            &[on(10, 60)],
            &PluginTransport::default(),
        );
        assert_eq!(l[9], 0.0);
        assert!(l[10] > 0.0);
    }

    #[test]
    fn a_short_sample_rings_on_and_a_long_one_stops_with_its_note() {
        let dir = tempfile::tempdir().unwrap();
        let (_, mut hit) = load_sampler(&wav(dir.path(), 0.1), RATE).unwrap();
        sounding(&mut hit, &[on(0, 60)], 1);
        assert!(
            sounding(&mut hit, &[off(60)], 40) > PLUGIN_BLOCK * 20,
            "rings"
        );

        let (_, mut long) = load_sampler(&wav(dir.path(), 3.0), RATE).unwrap();
        sounding(&mut long, &[on(0, 60)], 1);
        let after = sounding(&mut long, &[off(60)], 40);
        assert!(after > 0 && after < 700, "fades, then stops: {after}");
    }

    #[test]
    fn past_its_voices_the_oldest_gives_way_and_nothing_breaks() {
        let dir = tempfile::tempdir().unwrap();
        let (_, mut audio) = load_sampler(&wav(dir.path(), 1.0), RATE).unwrap();
        let many: Vec<PluginEvent> = (0..VOICES as u8 + 10).map(|k| on(0, 40 + k)).collect();
        assert!(sounding(&mut audio, &many, 2) > 0);
    }

    #[test]
    fn a_file_that_cannot_be_read_is_an_error_and_the_audio_half_says_when_it_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("gone.wav");
        assert!(load_sampler(&missing, RATE).is_err());
        let (loaded, audio) = load_sampler(&wav(dir.path(), 0.1), RATE).unwrap();
        assert!(!loaded.released());
        drop(audio);
        assert!(loaded.released());
    }
}
