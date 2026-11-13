//! The Stem Splitter's work, on the CPU through ONNX Runtime: HTDemucs splits a song, and DrumSep splits
//! its drums into the pieces of the kit. A copy of this program started just for it (`--split-stems`)
//! does the work, so that a failure costs only the split and the ONNX Runtime library is never loaded
//! into the program that plays audio. See `stem_splitter.rs` for the window that starts it and reads
//! what it prints.
use ovrtone_engine::AudioFile;
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::f32::consts::TAU;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const ARG: &str = "--split-stems";
/// The ONNX Runtime library, in the stems folder.
pub const RUNTIME: &str = "onnxruntime.dll";
/// The four stem model, in the stems folder: used unless Guitar or Piano is asked for.
pub const MODEL: &str = "htdemucs_fp16weights.onnx";
/// The six stem model.
pub const MODEL_SIX: &str = "htdemucs_6s_fp16weights.onnx";
/// The model for the pieces of the kit.
pub const MODEL_KIT: &str = "drumsep_v2_w10_f16w.onnx";
/// What the models give, in their order.
const SOURCES: [&str; 4] = ["drums", "bass", "other", "vocals"];
const SOURCES_SIX: [&str; 6] = ["drums", "bass", "other", "vocals", "guitar", "piano"];
const KIT: [&str; 4] = ["kick", "snare", "cymbals", "toms"];
/// A file made of all that was left out.
pub const SUBMIX: &str = "submix";
/// The rate the models work at.
const MODEL_RATE: u32 = 44_100;
/// The frames in the one length HTDemucs takes: 7.8 seconds.
const SEGMENT: usize = 343_980;
/// The frames in the one length DrumSep takes: 10 seconds.
const KIT_SEGMENT: usize = 441_000;
/// DrumSep's STFT, as its HDemucs was trained with: the model takes and gives spectra.
const NFFT: usize = 4096;
const HOP: usize = NFFT / 4;
/// The bins it uses, which leaves out the last.
const BINS: usize = NFFT / 2;

/// What a copy started for a split is asked to do.
pub struct Request {
    dir: PathBuf,
    input: PathBuf,
    out: PathBuf,
    stems: Vec<String>,
}

/// The arguments that ask for a split.
pub fn arguments(dir: &Path, input: &Path, out: &Path, stems: &[&str]) -> Vec<OsString> {
    vec![
        ARG.into(),
        dir.into(),
        input.into(),
        out.into(),
        stems.join(",").into(),
    ]
}

/// The request in this program's arguments, if it was started for one.
pub fn request(args: &[OsString]) -> Option<Request> {
    let [_, arg, dir, input, out, stems] = args else {
        return None;
    };
    (arg == ARG).then(|| Request {
        dir: dir.into(),
        input: input.into(),
        out: out.into(),
        stems: stems
            .to_string_lossy()
            .split(',')
            .map(str::to_string)
            .collect(),
    })
}

/// The copy's whole job. Returns the process's exit code.
pub fn run(request: &Request) -> i32 {
    // The program that asked keeps this pipe open. If it goes, for any reason, so does this copy: it
    // would otherwise go on using every core with nobody to take the result.
    std::thread::spawn(|| {
        let mut sink = Vec::new();
        let _ = std::io::stdin().read_to_end(&mut sink);
        crate::leave_now(2);
    });
    match split(request) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// A line for the program that asked. It reads them as they come.
fn report(line: &str) {
    let mut out = std::io::stdout();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

/// Counts the parts of the whole job, across the models.
struct Parts {
    done: usize,
    total: usize,
}

impl Parts {
    fn one_more(&mut self) {
        self.done += 1;
        report(&format!("Separated chunk {}/{}", self.done, self.total));
    }
}

/// What the request asks of the models.
struct Plan {
    /// The stems of the song model, by name.
    sources: &'static [&'static str],
    model: &'static str,
    /// The pieces of the kit are wanted, or the drums must be split so that some go in the submix.
    kit: bool,
}

fn plan(stems: &[String]) -> Result<Plan, String> {
    let wants = |name: &str| stems.iter().any(|s| s == name);
    for stem in stems {
        let known = stem == SUBMIX || SOURCES_SIX.contains(&stem.as_str()) || KIT.contains(&stem.as_str());
        if !known {
            return Err(format!("no stem called {stem}"));
        }
    }
    let six = wants("guitar") || wants("piano");
    Ok(Plan {
        sources: if six { &SOURCES_SIX } else { &SOURCES },
        model: if six { MODEL_SIX } else { MODEL },
        kit: KIT.iter().any(|piece| wants(piece)),
    })
}

fn split(request: &Request) -> Result<(), String> {
    let plan = plan(&request.stems)?;
    report("Reading");
    let file = AudioFile::open(&request.input).map_err(|e| e.to_string())?;
    let rate = file.sample_rate;
    let stereo = file.read_stereo().map_err(|e| e.to_string())?;
    let mix = crate::audio_encode::resample(&stereo, rate, MODEL_RATE);
    drop(stereo);
    let frames = mix.len() / 2;
    if frames == 0 {
        return Err("the file is empty".to_string());
    }

    report("Loading");
    let environment = ort::init_from(request.dir.join(RUNTIME))
        .map_err(|e| format!("ONNX Runtime: {e}"))?
        .build()
        .map_err(|e| format!("ONNX Runtime: {e}"))?;
    let open_model = |name: &str| {
        ort::session::Session::builder(&environment)
            .map_err(|e| e.to_string())?
            .with_optimization_level(ort::session::builder::GraphOptimizationLevel::All)
            .map_err(|e| e.to_string())?
            .commit_from_file(request.dir.join(name))
            .map_err(|e| format!("the model: {e}"))
    };
    let mut session = open_model(plan.model)?;

    report("Separating");
    let mut parts = Parts {
        done: 0,
        total: part_count(frames, SEGMENT)
            + if plan.kit {
                part_count(frames, KIT_SEGMENT)
            } else {
                0
            },
    };
    let song = separate(&mut session, &mix, plan.sources.len(), &mut parts)?;
    drop(session);
    drop(mix);
    let mut stems: Vec<(&str, Vec<f32>)> = plan.sources.iter().copied().zip(song).collect();

    if plan.kit {
        let drums = stems
            .iter()
            .position(|(name, _)| *name == "drums")
            .map(|at| stems.remove(at))
            .ok_or_else(|| "the model gave no drums".to_string())?;
        let mut kit_model = open_model(MODEL_KIT)?;
        let pieces = separate_kit(&mut kit_model, &drums.1, &mut parts)?;
        stems.extend(KIT.iter().copied().zip(pieces));
    }

    report("Wrote");
    std::fs::create_dir_all(&request.out).map_err(|e| e.to_string())?;
    let wanted = |name: &str| request.stems.iter().any(|s| s == name);
    let target_frames = (frames as u64 * u64::from(rate) / u64::from(MODEL_RATE)) as usize;
    let write = |name: &str, audio: &[f32]| -> Result<(), String> {
        if !audio.iter().all(|v| v.is_finite()) {
            return Err("the model gave invalid audio".to_string());
        }
        // At the file's own rate, the same length as the file.
        let mut audio = crate::audio_encode::resample(audio, MODEL_RATE, rate);
        audio.resize(target_frames * 2, 0.0);
        write_wav(&request.out.join(format!("{name}.wav")), &audio, rate)
    };
    for (name, audio) in stems.iter().filter(|(name, _)| wanted(name)) {
        write(name, audio)?;
    }
    if wanted(SUBMIX) {
        let mut rest = vec![0f32; frames * 2];
        for (_, audio) in stems.iter().filter(|(name, _)| !wanted(name)) {
            for (sum, v) in rest.iter_mut().zip(audio) {
                *sum += v;
            }
        }
        write(SUBMIX, &rest)?;
    }
    Ok(())
}

/// Runs HTDemucs over the song in overlapping parts. Gives each source, interleaved.
fn separate(
    session: &mut ort::session::Session,
    mix: &[f32],
    sources: usize,
    parts: &mut Parts,
) -> Result<Vec<Vec<f32>>, String> {
    let frames = mix.len() / 2;
    let mut joiner = Joiner::new(frames, sources, SEGMENT);
    for part in 0..part_count(frames, SEGMENT) {
        let (start, len) = joiner.place(part);
        let input = planar_part(mix, start, len, SEGMENT);
        let tensor =
            ort::value::TensorRef::from_array_view(([1usize, 2, SEGMENT], input.as_slice()))
                .map_err(|e| e.to_string())?;
        let outputs = session
            .run(ort::inputs!["mix" => tensor])
            .map_err(|e| e.to_string())?;
        let (_, out) = outputs["stems"]
            .try_extract_tensor::<f32>()
            .map_err(|e| e.to_string())?;
        if out.len() != sources * 2 * SEGMENT {
            return Err("the model gave an unexpected shape".to_string());
        }
        joiner.add(part, out);
        parts.one_more();
    }
    Ok(joiner.finish())
}

/// Runs DrumSep over a drum stem in overlapping parts. Its model takes the sound and its spectrum and
/// gives a sound and a spectrum for each piece, to be added.
fn separate_kit(
    session: &mut ort::session::Session,
    drums: &[f32],
    parts: &mut Parts,
) -> Result<Vec<Vec<f32>>, String> {
    let frames = drums.len() / 2;
    let stft = Stft::new();
    let bins_frames = KIT_SEGMENT.div_ceil(HOP);
    let mut joiner = Joiner::new(frames, KIT.len(), KIT_SEGMENT);
    for part in 0..part_count(frames, KIT_SEGMENT) {
        let (start, len) = joiner.place(part);
        let input = planar_part(drums, start, len, KIT_SEGMENT);
        let spectrum = stft.channels_of(&input);
        let mix = ort::value::TensorRef::from_array_view(([1usize, 2, KIT_SEGMENT], input.as_slice()))
            .map_err(|e| e.to_string())?;
        let mag = ort::value::TensorRef::from_array_view((
            [1usize, 4, BINS, bins_frames],
            spectrum.as_slice(),
        ))
        .map_err(|e| e.to_string())?;
        let outputs = session
            .run(ort::inputs!["mix" => mix, "mag" => mag])
            .map_err(|e| e.to_string())?;
        let (_, freq) = outputs["freq_out"]
            .try_extract_tensor::<f32>()
            .map_err(|e| e.to_string())?;
        let (_, time) = outputs["time_out"]
            .try_extract_tensor::<f32>()
            .map_err(|e| e.to_string())?;
        let sounds = KIT.len() * 2 * KIT_SEGMENT;
        if time.len() != sounds || freq.len() != KIT.len() * 4 * BINS * bins_frames {
            return Err("the model gave an unexpected shape".to_string());
        }
        let mut sum = time.to_vec();
        let per_piece = 4 * BINS * bins_frames;
        for (piece, spectra) in freq.chunks_exact(per_piece).enumerate() {
            for channel in 0..2 {
                let at = (piece * 2 + channel) * KIT_SEGMENT;
                let sound = stft.sound_of(
                    &spectra[channel * 2 * BINS * bins_frames..(channel + 1) * 2 * BINS * bins_frames],
                    bins_frames,
                );
                for (s, v) in sum[at..at + KIT_SEGMENT].iter_mut().zip(&sound) {
                    *s += v;
                }
            }
        }
        joiner.add(part, &sum);
        parts.one_more();
    }
    Ok(joiner.finish())
}

/// One part of interleaved stereo, each channel whole, the end of the song padded with silence.
fn planar_part(stereo: &[f32], start: usize, len: usize, size: usize) -> Vec<f32> {
    let mut input = vec![0f32; size * 2];
    for k in 0..len {
        input[k] = stereo[(start + k) * 2];
        input[size + k] = stereo[(start + k) * 2 + 1];
    }
    input
}

/// Overlaps parts of a model's output back into whole stems, faded across where they share frames.
struct Joiner {
    frames: usize,
    size: usize,
    window: Vec<f32>,
    stems: Vec<Vec<f32>>,
    weight: Vec<f32>,
}

impl Joiner {
    fn new(frames: usize, stems: usize, size: usize) -> Self {
        Self {
            frames,
            size,
            window: window(size),
            stems: vec![vec![0f32; frames * 2]; stems],
            weight: vec![0f32; frames],
        }
    }

    /// Where a part starts in the song, and how much of it is song.
    fn place(&self, part: usize) -> (usize, usize) {
        let start = part * stride(self.size);
        (start, self.size.min(self.frames - start))
    }

    /// Takes in a part: for each stem, a whole left channel and a whole right one.
    fn add(&mut self, part: usize, out: &[f32]) {
        let (start, len) = self.place(part);
        for (s, stem) in self.stems.iter_mut().enumerate() {
            let left = s * 2 * self.size;
            let right = left + self.size;
            for k in 0..len {
                stem[(start + k) * 2] += out[left + k] * self.window[k];
                stem[(start + k) * 2 + 1] += out[right + k] * self.window[k];
            }
        }
        for k in 0..len {
            self.weight[start + k] += self.window[k];
        }
    }

    fn finish(mut self) -> Vec<Vec<f32>> {
        for stem in &mut self.stems {
            for (frame, w) in stem.chunks_exact_mut(2).zip(&self.weight) {
                let w = w.max(1e-8);
                frame[0] /= w;
                frame[1] /= w;
            }
        }
        self.stems
    }
}

/// How far apart parts start: they share a quarter of their length.
fn stride(size: usize) -> usize {
    size - size / 4
}

/// How many parts a song takes.
fn part_count(frames: usize, size: usize) -> usize {
    frames.div_ceil(stride(size)).max(1)
}

/// How much of a part counts at each frame: fading in and out where parts share frames.
fn window(size: usize) -> Vec<f32> {
    let overlap = size / 4;
    let mut w = vec![1f32; size];
    for k in 0..overlap {
        let fade = k as f32 / (overlap - 1) as f32;
        w[k] = fade;
        w[size - 1 - k] = fade;
    }
    w
}

/// The STFT and its inverse as HDemucs has them: a Hann window, normalized, centred on frames.
struct Stft {
    forward: Arc<dyn Fft<f32>>,
    inverse: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
}

impl Stft {
    fn new() -> Self {
        let mut planner = FftPlanner::new();
        let window = (0..NFFT)
            .map(|k| 0.5 - 0.5 * (TAU * k as f32 / NFFT as f32).cos())
            .collect();
        Self {
            forward: planner.plan_fft_forward(NFFT),
            inverse: planner.plan_fft_inverse(NFFT),
            window,
        }
    }

    /// The spectrum of one channel: real then imaginary for each bin of each frame, `frames` of them,
    /// in the order the model wants: (part, bin, frame).
    fn spectrum(&self, sound: &[f32], frames: usize) -> Vec<(f32, f32)> {
        // Padding that centres the kept frames, then the padding of a centred STFT.
        let edge = HOP / 2 * 3;
        let total = sound.len() + edge * 2 + (frames * HOP - sound.len());
        let reflect = |at: isize, len: isize| -> usize {
            let mut k = at;
            loop {
                if k < 0 {
                    k = -k;
                } else if k >= len {
                    k = 2 * (len - 1) - k;
                } else {
                    return k as usize;
                }
            }
        };
        let padded: Vec<f32> = (0..total)
            .map(|k| sound[reflect(k as isize - edge as isize, sound.len() as isize)])
            .collect();
        let scale = 1.0 / (NFFT as f32).sqrt();
        let mut out = vec![(0f32, 0f32); BINS * frames];
        let mut buffer = vec![Complex::new(0f32, 0f32); NFFT];
        // The two frames the padding adds at the start are left out.
        for frame in 0..frames {
            let middle = (frame + 2) * HOP;
            for (k, b) in buffer.iter_mut().enumerate() {
                let at = reflect(middle as isize + k as isize - (NFFT / 2) as isize, total as isize);
                *b = Complex::new(padded[at] * self.window[k], 0.0);
            }
            self.forward.process(&mut buffer);
            for bin in 0..BINS {
                out[bin * frames + frame] = (buffer[bin].re * scale, buffer[bin].im * scale);
            }
        }
        out
    }

    /// The model's `mag` input from a part with its channels whole: left real, left imaginary, right
    /// real, right imaginary, each (bin, frame).
    fn channels_of(&self, planar: &[f32]) -> Vec<f32> {
        let size = planar.len() / 2;
        let frames = size.div_ceil(HOP);
        let mut out = vec![0f32; 4 * BINS * frames];
        let plane = BINS * frames;
        for channel in 0..2 {
            let sound = &planar[channel * size..(channel + 1) * size];
            for (k, (re, im)) in self.spectrum(sound, frames).into_iter().enumerate() {
                out[channel * 2 * plane + k] = re;
                out[(channel * 2 + 1) * plane + k] = im;
            }
        }
        out
    }

    /// The sound of a spectrum of one channel: all real values (bin, frame) then all imaginary ones.
    fn sound_of(&self, spectrum: &[f32], frames: usize) -> Vec<f32> {
        let plane = BINS * frames;
        let length = KIT_SEGMENT.min(frames * HOP);
        let edge = HOP / 2 * 3;
        // The frames left out on the way in come back as silence.
        let all_frames = frames + 4;
        let total = (all_frames - 1) * HOP;
        let mut sound = vec![0f32; total + NFFT];
        let mut weight = vec![0f32; total + NFFT];
        let scale = (NFFT as f32).sqrt() / NFFT as f32;
        let mut buffer = vec![Complex::new(0f32, 0f32); NFFT];
        for frame in 0..frames {
            buffer.fill(Complex::new(0.0, 0.0));
            for bin in 0..BINS {
                let value = Complex::new(
                    spectrum[bin * frames + frame],
                    spectrum[plane + bin * frames + frame],
                );
                buffer[bin] = value;
                if bin > 0 {
                    buffer[NFFT - bin] = value.conj();
                }
            }
            self.inverse.process(&mut buffer);
            let at = (frame + 2) * HOP;
            for k in 0..NFFT {
                sound[at + k] += buffer[k].re * scale * self.window[k];
                weight[at + k] += self.window[k] * self.window[k];
            }
        }
        // Back from the centred frames and the padding.
        let first = NFFT / 2 + edge;
        (0..length)
            .map(|k| {
                let at = first + k;
                sound[at] / weight[at].max(1e-8)
            })
            .collect()
    }
}

/// A 32-bit float stereo WAV, by way of a temporary name: a file cut short must not pass for a stem.
fn write_wav(path: &Path, stereo: &[f32], rate: u32) -> Result<(), String> {
    let bytes = wav_bytes(stereo, rate).ok_or_else(|| "the stem is too long".to_string())?;
    let partial = path.with_extension("part");
    std::fs::write(&partial, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&partial, path).map_err(|e| e.to_string())
}

fn wav_bytes(stereo: &[f32], rate: u32) -> Option<Vec<u8>> {
    let data = u32::try_from(stereo.len().checked_mul(4)?).ok()?;
    let riff = data.checked_add(36)?;
    let mut bytes = Vec::with_capacity(44 + data as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&riff.to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    // Float samples, two channels.
    bytes.extend_from_slice(&3u16.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 8).to_le_bytes());
    bytes.extend_from_slice(&8u16.to_le_bytes());
    bytes.extend_from_slice(&32u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for v in stereo {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    Some(bytes)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_parts_cover_the_whole_song() {
        for size in [SEGMENT, KIT_SEGMENT] {
            let step = stride(size);
            for frames in [1, step, step + 1, size, size * 3 + 17] {
                let last = (part_count(frames, size) - 1) * step;
                assert!(last < frames, "{frames}: the last part starts inside");
                assert!(last + size >= frames, "{frames}: the last part reaches the end");
            }
        }
    }

    #[test]
    fn a_request_survives_the_trip_through_the_arguments() {
        let mut args: Vec<OsString> = vec!["ovrtone".into()];
        args.extend(arguments(
            Path::new("a b"),
            Path::new("song.mp3"),
            Path::new("out"),
            &["vocals", "drums"],
        ));
        let request = request(&args).unwrap();
        assert_eq!(request.dir, Path::new("a b"));
        assert_eq!(request.stems, ["vocals", "drums"]);
    }

    #[test]
    fn the_models_are_chosen_by_what_is_asked_for() {
        let asked = |names: &[&str]| plan(&names.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        let four = asked(&["vocals", "bass"]).unwrap();
        assert_eq!((four.model, four.kit), (MODEL, false));
        let six = asked(&["guitar", SUBMIX]).unwrap();
        assert_eq!((six.model, six.kit), (MODEL_SIX, false));
        assert!(asked(&["kick"]).unwrap().kit);
        assert!(asked(&["kazoo"]).is_err());
    }

    #[test]
    fn a_stem_reads_back_as_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vocals.wav");
        write_wav(&path, &[0.5, -0.25, 0.125, 1.0], 48_000).unwrap();
        let file = AudioFile::open(&path).unwrap();
        assert_eq!(file.sample_rate, 48_000);
        assert_eq!(file.read_stereo().unwrap(), [0.5, -0.25, 0.125, 1.0]);
    }

    #[test]
    fn the_spectrum_turns_back_into_the_same_sound() {
        let stft = Stft::new();
        let sound: Vec<f32> = (0..KIT_SEGMENT)
            .map(|k| (k as f32 * 0.03).sin() * 0.5 + (k as f32 * 0.31).sin() * 0.2)
            .collect();
        let frames = KIT_SEGMENT.div_ceil(HOP);
        assert_eq!(frames, 431, "the shape the model has");
        let spectrum = stft.spectrum(&sound, frames);
        let plane = BINS * frames;
        let mut flat = vec![0f32; plane * 2];
        for (k, (re, im)) in spectrum.into_iter().enumerate() {
            flat[k] = re;
            flat[plane + k] = im;
        }
        let back = stft.sound_of(&flat, frames);
        assert_eq!(back.len(), KIT_SEGMENT);
        // Away from the very ends, where frames are missing on purpose.
        for k in NFFT * 2..KIT_SEGMENT - NFFT * 2 {
            assert!((back[k] - sound[k]).abs() < 1e-2, "{k}: {} {}", back[k], sound[k]);
        }
    }
}
