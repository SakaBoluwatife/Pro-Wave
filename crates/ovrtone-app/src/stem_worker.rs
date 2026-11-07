//! The Stem Splitter's work: HTDemucs through ONNX Runtime, on the CPU. A copy of this program started
//! just for it (`--split-stems`) does the work, so that a failure costs only the split and the ONNX
//! Runtime library is never loaded into the program that plays audio. See `stem_splitter.rs` for the
//! window that starts it and reads what it prints.
use ovrtone_engine::AudioFile;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const ARG: &str = "--split-stems";
/// The ONNX Runtime library, in the stems folder.
pub const RUNTIME: &str = "onnxruntime.dll";
/// The model, in the stems folder.
pub const MODEL: &str = "htdemucs_fp16weights.onnx";
/// The stems the model gives, in its order.
const SOURCES: [&str; 4] = ["drums", "bass", "other", "vocals"];
/// The rate the model works at.
const MODEL_RATE: u32 = 44_100;
/// The frames in the one length the model takes: 7.8 seconds.
const SEGMENT: usize = 343_980;
/// How much two parts next to each other share, faded across.
const OVERLAP: usize = SEGMENT / 4;
const STRIDE: usize = SEGMENT - OVERLAP;

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

fn split(request: &Request) -> Result<(), String> {
    let wanted: Vec<usize> = request
        .stems
        .iter()
        .map(|stem| {
            SOURCES
                .iter()
                .position(|s| s == stem)
                .ok_or_else(|| format!("no stem called {stem}"))
        })
        .collect::<Result<_, _>>()?;
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
    let mut session = ort::session::Session::builder(&environment)
        .map_err(|e| e.to_string())?
        .with_optimization_level(ort::session::builder::GraphOptimizationLevel::All)
        .map_err(|e| e.to_string())?
        .commit_from_file(request.dir.join(MODEL))
        .map_err(|e| format!("the model: {e}"))?;

    report("Separating");
    let window = window();
    let total = part_count(frames);
    let mut stems = vec![vec![0f32; frames * 2]; wanted.len()];
    let mut weight = vec![0f32; frames];
    let mut input = vec![0f32; SEGMENT * 2];
    for part in 0..total {
        let start = part * STRIDE;
        let len = SEGMENT.min(frames - start);
        // The model takes each channel whole, and the end of the song padded with silence.
        input.fill(0.0);
        for k in 0..len {
            input[k] = mix[(start + k) * 2];
            input[SEGMENT + k] = mix[(start + k) * 2 + 1];
        }
        let tensor =
            ort::value::TensorRef::from_array_view(([1usize, 2, SEGMENT], input.as_slice()))
                .map_err(|e| e.to_string())?;
        let outputs = session
            .run(ort::inputs!["mix" => tensor])
            .map_err(|e| e.to_string())?;
        let (_, parts) = outputs["stems"]
            .try_extract_tensor::<f32>()
            .map_err(|e| e.to_string())?;
        if parts.len() != SOURCES.len() * 2 * SEGMENT {
            return Err("the model gave an unexpected shape".to_string());
        }
        for (stem, source) in stems.iter_mut().zip(&wanted) {
            let left = source * 2 * SEGMENT;
            let right = left + SEGMENT;
            for k in 0..len {
                stem[(start + k) * 2] += parts[left + k] * window[k];
                stem[(start + k) * 2 + 1] += parts[right + k] * window[k];
            }
        }
        for k in 0..len {
            weight[start + k] += window[k];
        }
        report(&format!("Separated chunk {}/{total}", part + 1));
    }

    report("Wrote");
    std::fs::create_dir_all(&request.out).map_err(|e| e.to_string())?;
    for (stem, name) in stems.iter_mut().zip(&request.stems) {
        for (frame, w) in stem.chunks_exact_mut(2).zip(&weight) {
            let w = w.max(1e-8);
            frame[0] /= w;
            frame[1] /= w;
        }
        if !stem.iter().all(|v| v.is_finite()) {
            return Err("the model gave invalid audio".to_string());
        }
        // At the file's own rate, the same length as the file.
        let mut audio = crate::audio_encode::resample(stem, MODEL_RATE, rate);
        audio.resize(
            (frames as u64 * u64::from(rate) / u64::from(MODEL_RATE)) as usize * 2,
            0.0,
        );
        write_wav(&request.out.join(format!("{name}.wav")), &audio, rate)?;
    }
    Ok(())
}

/// How many parts a song takes.
fn part_count(frames: usize) -> usize {
    frames.div_ceil(STRIDE).max(1)
}

/// How much of a part counts at each frame: fading in and out where parts share frames.
fn window() -> Vec<f32> {
    let mut w = vec![1f32; SEGMENT];
    for k in 0..OVERLAP {
        let fade = k as f32 / (OVERLAP - 1) as f32;
        w[k] = fade;
        w[SEGMENT - 1 - k] = fade;
    }
    w
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
        for frames in [1, STRIDE, STRIDE + 1, SEGMENT, SEGMENT * 3 + 17] {
            let last = (part_count(frames) - 1) * STRIDE;
            assert!(last < frames, "{frames}: the last part starts inside");
            assert!(last + SEGMENT >= frames, "{frames}: the last part reaches the end");
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
    fn a_stem_reads_back_as_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vocals.wav");
        write_wav(&path, &[0.5, -0.25, 0.125, 1.0], 48_000).unwrap();
        let file = AudioFile::open(&path).unwrap();
        assert_eq!(file.sample_rate, 48_000);
        assert_eq!(file.read_stereo().unwrap(), [0.5, -0.25, 0.125, 1.0]);
    }
}
