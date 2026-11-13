//! A small built-in sample player: one audio file played from the track's notes, at the pitch of
//! each note against C4 (FL Studio: a channel holding a sample; Logic: Quick Sampler). It is not a
//! sound library, only a way to play the user's own sample from the piano roll, and it goes in an
//! instrument slot like any plugin.
//!
//! A short sample plays to its end whatever the note's length, as a drum hit should. A longer one
//! stops, with a short fade, when the note does. [`params`] are its settings: pitch, root key, level,
//! pan, where it starts and ends, reverse, an envelope and more. The window for them is the program's.
use crate::{HostError, Loaded, LoadedPlugin};
use ovrtone_engine::{AudioFile, PluginEvent, PluginEventKind, PluginProcessor, PluginTransport};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

/// Notes sounding at once; past this the oldest gives way.
const VOICES: usize = 32;
/// Samples up to this long play to their end.
const ONE_SHOT_SECONDS: f64 = 2.0;

/// The sampler's settings, each kept as a position 0..=1 as plugin parameters are, and what those
/// positions mean.
pub mod params {
    pub const VOLUME: u32 = 0;
    pub const PAN: u32 = 1;
    pub const PITCH: u32 = 2;
    pub const FINE: u32 = 3;
    pub const ROOT: u32 = 4;
    pub const START: u32 = 5;
    pub const END: u32 = 6;
    pub const REVERSE: u32 = 7;
    pub const ATTACK: u32 = 8;
    pub const DECAY: u32 = 9;
    pub const SUSTAIN: u32 = 10;
    pub const RELEASE: u32 = 11;
    pub const CUT_SELF: u32 = 12;
    pub const TO_END: u32 = 13;
    pub const COUNT: usize = 14;

    pub const NAMES: [&str; COUNT] = [
        "Volume",
        "Pan",
        "Pitch",
        "Fine",
        "Root Key",
        "Start",
        "End",
        "Reverse",
        "Attack",
        "Decay",
        "Sustain",
        "Release",
        "Cut Itself",
        "Play to End",
    ];

    const MIN_DB: f32 = -60.0;
    const MAX_DB: f32 = 12.0;

    pub fn volume_db(v: f32) -> f32 {
        MIN_DB + v * (MAX_DB - MIN_DB)
    }
    pub fn pitch_semitones(v: f32) -> f32 {
        ((v - 0.5) * 96.0).round()
    }
    pub fn fine_cents(v: f32) -> f32 {
        (v - 0.5) * 200.0
    }
    pub fn root_key(v: f32) -> u8 {
        (v * 127.0).round().clamp(0.0, 127.0) as u8
    }
    pub fn of_root_key(key: u8) -> f32 {
        f32::from(key.min(127)) / 127.0
    }
    /// Attack, decay and release, in seconds: fine at the short end.
    pub fn seconds(v: f32) -> f32 {
        v * v * 4.0
    }

    /// Where each starts: a sample as recorded, a short fade when let go.
    pub fn defaults() -> [f32; COUNT] {
        [
            -MIN_DB / (MAX_DB - MIN_DB),
            0.5,
            0.5,
            0.5,
            60.0 / 127.0,
            0.0,
            1.0,
            0.0,
            0.0,
            0.0,
            1.0,
            0.055,
            0.0,
            0.0,
        ]
    }
}

/// The settings, shared by the window and the audio thread: it reads them once a block.
struct Settings([AtomicU32; params::COUNT]);

impl Settings {
    fn new(to_end: bool) -> Self {
        let mut values = params::defaults();
        values[params::TO_END as usize] = if to_end { 1.0 } else { 0.0 };
        Self(values.map(|v| AtomicU32::new(v.to_bits())))
    }

    fn get(&self, id: u32) -> f32 {
        self.0
            .get(id as usize)
            .map_or(0.0, |v| f32::from_bits(v.load(Ordering::Relaxed)))
    }

    fn set(&self, id: u32, value: f32) {
        if let Some(slot) = self.0.get(id as usize) {
            slot.store(value.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        }
    }
}

/// The settings as the audio thread uses them, worked out once a block.
#[derive(Clone, Copy, Default)]
struct Live {
    left: f32,
    right: f32,
    root: i32,
    /// Semitones, fine tune included.
    offset: f64,
    start: f64,
    end: f64,
    reverse: bool,
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
    cut_self: bool,
    to_end: bool,
}

#[derive(Clone, Copy, Default)]
struct Voice {
    active: bool,
    releasing: bool,
    /// Still rising to full level.
    rising: bool,
    key: u8,
    /// Where it is in the sample, in frames.
    position: f64,
    /// Frames of the sample per frame of output.
    step: f64,
    gain: f32,
    envelope: f32,
    /// Counts up, so the oldest voice can be told.
    age: u64,
    /// A slide of this note alone (per-note tuning), in semitones: where it is and where it is
    /// going, and the speed it makes.
    tune_now: f32,
    tune_wanted: f32,
    tune_speed: f64,
}

/// How far the sampler bends at the top of the range, either way: as far as a slide could want.
pub const SAMPLER_BEND_SEMITONES: f32 = 48.0;
/// How much of the way to the wanted pitch each frame goes, so that steps between bends glide.
const BEND_GLIDE_PER_FRAME: f32 = 0.004;

struct SamplerProcessor {
    /// The bend wanted and the bend now, in semitones, and what the now means for playing speed.
    bend_wanted: f32,
    bend_now: f32,
    bend_speed: f64,
    /// Interleaved stereo.
    samples: Arc<Vec<f32>>,
    /// Frames of the sample per frame of output at the root key.
    base_step: f64,
    settings: Arc<Settings>,
    live: Live,
    rate: f32,
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
        let live = self.live;
        if live.cut_self {
            for v in self.voices.iter_mut().filter(|v| v.active) {
                v.releasing = true;
            }
        }
        let semitones = f64::from(i32::from(key) - live.root) + live.offset;
        let rising = live.attack < 1.0;
        self.voices[at] = Voice {
            active: true,
            releasing: false,
            rising,
            key,
            position: 0.0,
            step: self.base_step * (semitones / 12.0).exp2(),
            gain: velocity.clamp(0.0, 1.0),
            envelope: if rising { 0.0 } else { 1.0 },
            age: self.started,
            tune_now: 0.0,
            tune_wanted: 0.0,
            tune_speed: 1.0,
        };
    }

    fn note_off(&mut self, key: u8) {
        if self.live.to_end {
            return;
        }
        for v in self.voices.iter_mut().filter(|v| v.active && v.key == key) {
            v.releasing = true;
        }
    }

    /// Takes in the settings for this block. No allocation or lock: only atomics.
    fn read_settings(&mut self) {
        use params as x;
        let s = &self.settings;
        let frames = (self.samples.len() / 2).saturating_sub(1) as f64;
        let gain = if s.get(x::VOLUME) <= 0.0 {
            0.0
        } else {
            10f32.powf(x::volume_db(s.get(x::VOLUME)) / 20.0)
        };
        let pan = s.get(x::PAN);
        let per_frame = |seconds: f32, rate: f32| {
            if seconds <= 0.0 {
                1.0
            } else {
                (1.0 / (seconds * rate)).min(1.0)
            }
        };
        let (first, last) = (s.get(x::START), s.get(x::END));
        let (first, last) = (first.min(last), first.max(last));
        self.live = Live {
            left: gain * (2.0 - 2.0 * pan).min(1.0),
            right: gain * (2.0 * pan).min(1.0),
            root: i32::from(x::root_key(s.get(x::ROOT))),
            offset: f64::from(
                x::pitch_semitones(s.get(x::PITCH)) + x::fine_cents(s.get(x::FINE)) / 100.0,
            ),
            start: f64::from(first) * frames,
            end: f64::from(last) * frames,
            reverse: s.get(x::REVERSE) >= 0.5,
            attack: per_frame(x::seconds(s.get(x::ATTACK)), self.rate),
            decay: per_frame(x::seconds(s.get(x::DECAY)), self.rate),
            sustain: s.get(x::SUSTAIN),
            release: per_frame(x::seconds(s.get(x::RELEASE)), self.rate),
            cut_self: s.get(x::CUT_SELF) >= 0.5,
            to_end: s.get(x::TO_END) >= 0.5,
        };
    }

    /// Adds one frame of every voice to (l, r).
    #[inline]
    fn render(&mut self) -> (f32, f32) {
        let frames = self.samples.len() / 2;
        let live = self.live;
        let (mut l, mut r) = (0.0, 0.0);
        for v in self.voices.iter_mut().filter(|v| v.active) {
            if live.start + v.position >= live.end {
                v.active = false;
                continue;
            }
            // Where it is in the file: from the start on, or back from the end when reversed.
            let place = if live.reverse {
                (live.end - v.position - 1.0).max(0.0)
            } else {
                live.start + v.position
            };
            let at = place as usize;
            if at + 1 >= frames {
                v.active = false;
                continue;
            }
            let t = (place - at as f64) as f32;
            let s = &self.samples;
            let gain = v.gain * v.envelope;
            l += (s[at * 2] + (s[at * 2 + 2] - s[at * 2]) * t) * gain * live.left;
            r += (s[at * 2 + 1] + (s[at * 2 + 3] - s[at * 2 + 1]) * t) * gain * live.right;
            if v.tune_now != v.tune_wanted {
                let gap = v.tune_wanted - v.tune_now;
                v.tune_now = if gap.abs() < 0.001 {
                    v.tune_wanted
                } else {
                    v.tune_now + gap * BEND_GLIDE_PER_FRAME
                };
                v.tune_speed = f64::from(v.tune_now / 12.0).exp2();
            }
            v.position += v.step * self.bend_speed * v.tune_speed;
            if v.releasing {
                v.envelope -= live.release;
                if v.envelope <= 0.0 {
                    v.active = false;
                }
            } else if v.rising {
                v.envelope += live.attack;
                if v.envelope >= 1.0 {
                    v.envelope = 1.0;
                    v.rising = false;
                }
            } else if v.envelope > live.sustain {
                v.envelope = (v.envelope - live.decay).max(live.sustain);
            }
        }
        (l, r)
    }
}

impl PluginProcessor for SamplerProcessor {
    fn note_tuning(&self) -> bool {
        true
    }

    fn process(
        &mut self,
        input: [&[f32]; 2],
        output: [&mut [f32]; 2],
        events: &[PluginEvent],
        _transport: &PluginTransport,
    ) {
        self.read_settings();
        let [out_l, out_r] = output;
        let mut next = 0;
        for frame in 0..out_l.len() {
            while let Some(e) = events.get(next).filter(|e| e.frame as usize <= frame) {
                match e.kind {
                    PluginEventKind::NoteOn { key, velocity } => self.note_on(key, velocity),
                    PluginEventKind::NoteOff { key } => self.note_off(key),
                    PluginEventKind::PitchBend { value } => {
                        self.bend_wanted = value * SAMPLER_BEND_SEMITONES;
                    }
                    // A slide that bends only its own note (an 808 sliding under the others).
                    PluginEventKind::NoteTune { key, semitones } => {
                        for v in self.voices.iter_mut().filter(|v| v.active && !v.releasing && v.key == key) {
                            v.tune_wanted = semitones;
                        }
                    }
                    PluginEventKind::Control { .. } | PluginEventKind::ChannelPressure { .. } => {}
                }
                next += 1;
            }
            if self.bend_now != self.bend_wanted {
                let gap = self.bend_wanted - self.bend_now;
                self.bend_now = if gap.abs() < 0.001 {
                    self.bend_wanted
                } else {
                    self.bend_now + gap * BEND_GLIDE_PER_FRAME
                };
                self.bend_speed = f64::from(self.bend_now / 12.0).exp2();
            }
            let (l, r) = self.render();
            out_l[frame] = l + input[0][frame];
            out_r[frame] = r + input[1][frame];
        }
    }

    fn latency_samples(&self) -> usize {
        0
    }

    fn set_param(&mut self, id: u32, value: f64) {
        self.settings.set(id, value as f32);
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
    settings: Arc<Settings>,
    dirty: bool,
}

/// Starts the sampler's saved settings.
const STATE_MAGIC: &[u8; 4] = b"PWS1";

impl LoadedPlugin for LoadedSampler {
    fn params(&self) -> Vec<crate::PluginParam> {
        params::NAMES
            .iter()
            .enumerate()
            .map(|(id, name)| crate::PluginParam {
                id: id as u32,
                name: (*name).to_string(),
            })
            .collect()
    }

    fn param_value(&mut self, id: u32) -> Option<f64> {
        ((id as usize) < params::COUNT).then(|| f64::from(self.settings.get(id)))
    }

    fn show_param(&mut self, id: u32, value: f64) {
        self.settings.set(id, value as f32);
        self.dirty = true;
    }

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
        let mut out = STATE_MAGIC.to_vec();
        for id in 0..params::COUNT as u32 {
            out.extend_from_slice(&self.settings.get(id).to_le_bytes());
        }
        Ok(Some(out))
    }

    fn load_state(&mut self, bytes: &[u8]) -> Result<(), HostError> {
        let saved = bytes
            .strip_prefix(STATE_MAGIC)
            .ok_or_else(|| HostError("the sampler's saved settings are damaged".to_string()))?;
        // A later version may save more; one that saved less leaves the rest as they are.
        for (id, value) in saved.as_chunks::<4>().0.iter().enumerate() {
            self.settings.set(id as u32, f32::from_le_bytes(*value));
        }
        Ok(())
    }

    fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
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
    let settings = Arc::new(Settings::new(file.duration_secs() <= ONE_SHOT_SECONDS));
    let audio = SamplerProcessor {
        samples: Arc::new(samples),
        base_step: f64::from(file.sample_rate.max(1)) / rate,
        bend_wanted: 0.0,
        bend_now: 0.0,
        bend_speed: 1.0,
        settings: settings.clone(),
        live: Live::default(),
        rate: rate as f32,
        voices: [Voice::default(); VOICES],
        started: 0,
        released: released.clone(),
    };
    let loaded = LoadedSampler {
        released,
        settings,
        dirty: false,
    };
    Ok((Box::new(loaded), Box::new(audio)))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use ovrtone_engine::PLUGIN_BLOCK;

    const RATE: u32 = 48_000;

    fn wav(dir: &Path, seconds: f64) -> std::path::PathBuf {
        let path = dir.join("hit.wav");
        let frames = (seconds * f64::from(RATE)) as usize;
        ovrtone_engine::write_wav(&path, &vec![0.5; frames * 2], RATE).unwrap();
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
    fn a_bend_changes_how_fast_a_one_shot_plays_and_with_no_steps_or_breaks() {
        let dir = tempfile::tempdir().unwrap();
        let path = wav(dir.path(), 0.25);
        let rate = RATE;
        let played = |bend: Option<f32>| {
            let (_, mut audio) = load_sampler(&path, rate).unwrap();
            let mut events = vec![on(0, 60)];
            if let Some(value) = bend {
                events.push(PluginEvent {
                    frame: 0,
                    kind: PluginEventKind::PitchBend { value },
                });
            }
            sounding(&mut audio, &events, 400)
        };
        let (plain, up) = (played(None), played(Some(12.0 / SAMPLER_BEND_SEMITONES)));
        // An octave up plays it in half the time, give or take the first moments of the glide.
        assert!(up < plain * 3 / 5, "{up} frames against {plain}");
        assert!(up > plain * 2 / 5, "{up} frames against {plain}");
        // Down, it takes longer, and a bend back to nothing leaves it as it was.
        assert!(played(Some(-12.0 / SAMPLER_BEND_SEMITONES)) > plain);
        assert_eq!(played(Some(0.0)), plain);
    }

    #[test]
    fn the_settings_change_the_sound_and_are_saved() {
        use params as x;
        let dir = tempfile::tempdir().unwrap();
        let path = wav(dir.path(), 0.25);
        let (mut loaded, mut audio) = load_sampler(&path, RATE).unwrap();
        let plain = sounding(&mut audio, &[on(0, 60)], 400);
        // An octave higher by the Pitch setting plays in half the time, as a root key an octave
        // lower does.
        loaded.show_param(x::PITCH, 0.5 + 12.0 / 96.0);
        assert!(loaded.take_dirty());
        let up = sounding(&mut audio, &[on(0, 60)], 400);
        assert!(up < plain * 3 / 5 && up > plain * 2 / 5, "{up} of {plain}");
        loaded.show_param(x::PITCH, 0.5);
        loaded.show_param(x::ROOT, f64::from(x::of_root_key(48)));
        let by_root = sounding(&mut audio, &[on(0, 60)], 400);
        assert!(by_root < plain * 3 / 5, "{by_root} of {plain}");
        loaded.show_param(x::ROOT, f64::from(x::of_root_key(60)));
        // Half of it, by where it ends.
        loaded.show_param(x::END, 0.5);
        let half = sounding(&mut audio, &[on(0, 60)], 400);
        assert!(half < plain * 3 / 5, "{half} of {plain}");
        // Reversed, it is as long.
        loaded.show_param(x::END, 1.0);
        loaded.show_param(x::REVERSE, 1.0);
        let back = sounding(&mut audio, &[on(0, 60)], 400);
        assert!(back.abs_diff(plain) < plain / 20, "{back} of {plain}");
        // Silent at the bottom of the fader.
        loaded.show_param(x::VOLUME, 0.0);
        assert_eq!(sounding(&mut audio, &[on(0, 60)], 100), 0);
        // What was set comes back in a new one.
        let state = loaded.save_state().unwrap().unwrap();
        let (mut again, _audio) = load_sampler(&path, RATE).unwrap();
        again.load_state(&state).unwrap();
        assert_eq!(again.param_value(x::REVERSE), Some(1.0));
        assert_eq!(again.param_value(x::VOLUME), Some(0.0));
        assert!(again.load_state(b"nonsense").is_err());
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
