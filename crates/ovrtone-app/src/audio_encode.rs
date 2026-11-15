//! Turns a finished WAV file into FLAC or MP3. A bounce is rendered to WAV first and encoded after,
//! so the render itself is the same for every format.
use mp3lame_encoder::{
    max_required_buffer_size, Bitrate, Builder, FlushNoGap, InterleavedPcm, Quality, VbrMode,
};
use std::sync::atomic::{AtomicU8, Ordering};
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Where the WAV writer in bounce.rs starts the samples.
const WAV_HEADER: u64 = 44;
/// Frames given to the MP3 encoder at a time.
const MP3_FRAMES: usize = 8192;
/// The highest rate an MP3 file can have.
const MP3_MAX_RATE: u32 = 48_000;

/// How MP3 files are made: a fixed bit rate, or VBR at its best (V0).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mp3Quality {
    #[default]
    Kbps320,
    V0,
    Kbps256,
    Kbps192,
    Kbps128,
}

impl Mp3Quality {
    pub const ALL: [Self; 5] = [Self::Kbps320, Self::V0, Self::Kbps256, Self::Kbps192, Self::Kbps128];

    pub fn name(self) -> &'static str {
        match self {
            Self::Kbps320 => "320 kbps",
            Self::V0 => "VBR V0",
            Self::Kbps256 => "256 kbps",
            Self::Kbps192 => "192 kbps",
            Self::Kbps128 => "128 kbps",
        }
    }
}

/// The choice in the Bounce window, for every MP3 made after it.
static MP3_QUALITY: AtomicU8 = AtomicU8::new(0);

pub fn set_mp3_quality(q: Mp3Quality) {
    MP3_QUALITY.store(q as u8, Ordering::Relaxed);
}

fn mp3_quality() -> Mp3Quality {
    let n = usize::from(MP3_QUALITY.load(Ordering::Relaxed));
    Mp3Quality::ALL.into_iter().find(|q| *q as usize == n).unwrap_or_default()
}

fn part_of(out: &Path) -> PathBuf {
    let mut name = out.as_os_str().to_owned();
    name.push(".part");
    PathBuf::from(name)
}

/// A file from the 16-bit or 24-bit `wav` into `out`.
pub fn flac(wav: &Path, out: &Path, bits: usize, rate: u32) -> Result<(), String> {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;
    let bytes = std::fs::read(wav).map_err(|e| e.to_string())?;
    let data = bytes.get(WAV_HEADER as usize..).unwrap_or(&[]);
    let samples: Vec<i32> = if bits == 16 {
        data.as_chunks::<2>()
            .0
            .iter()
            .map(|b| i32::from(i16::from_le_bytes(*b)))
            .collect()
    } else {
        data.as_chunks::<3>()
            .0
            .iter()
            .map(|b| i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8)
            .collect()
    };
    drop(bytes);
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|(_, e)| format!("{e:?}"))?;
    let source = flacenc::source::MemSource::from_samples(&samples, 2, bits, rate as usize);
    let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size)
        .map_err(|e| format!("{e:?}"))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).map_err(|e| format!("{e:?}"))?;
    let mut coded = sink.as_slice().to_vec();
    same_block_sizes(&mut coded);
    let part = part_of(out);
    std::fs::write(&part, &coded).map_err(|e| e.to_string())?;
    std::fs::rename(&part, out).map_err(|e| e.to_string())
}

/// The file header gives the smallest and largest block size, and the smallest is not meant to count
/// the last block, which is only as long as what is left. The encoder counts it, and strict readers
/// then take the file for one of varying blocks and find no frame they accept.
fn same_block_sizes(flac: &mut [u8]) {
    // "fLaC", four bytes of block header, then the two sizes.
    if flac.len() >= 12 && flac.starts_with(b"fLaC") {
        flac.copy_within(10..12, 8);
    }
}

/// Filter taps on each side of a sample in [`resample`], and the steps between two samples its table
/// has. Plenty for a sinc whose first stop-band lobe is well below 16-bit noise.
const SRC_TAPS: usize = 32;
const SRC_PHASES: usize = 1024;

/// Interleaved stereo at another sample rate, by a windowed sinc. Goes down without aliasing: the
/// cutoff follows the lower of the two rates.
pub fn resample(stereo: &[f32], from: u32, to: u32) -> Vec<f32> {
    use std::f64::consts::PI;
    if from == to || from == 0 || to == 0 {
        return stereo.to_vec();
    }
    let cutoff = (f64::from(to) / f64::from(from)).min(1.0) * 0.98;
    let taps = (SRC_TAPS as f64 / cutoff).ceil() as usize;
    let width = 2 * taps;
    let beta = 9.0;
    // The modified Bessel function a Kaiser window is made from.
    let bessel = |x: f64| {
        let (mut sum, mut term) = (1.0f64, 1.0f64);
        for k in 1..32 {
            term *= (x / 2.0 / f64::from(k)).powi(2);
            sum += term;
        }
        sum
    };
    let norm = bessel(beta);
    let mut table = vec![0f32; (SRC_PHASES + 1) * width];
    for phase in 0..=SRC_PHASES {
        let fraction = phase as f64 / SRC_PHASES as f64;
        for tap in 0..width {
            let x = tap as f64 - (taps as f64 - 1.0) - fraction;
            let sinc = if x.abs() < 1e-9 {
                cutoff
            } else {
                (PI * x * cutoff).sin() / (PI * x)
            };
            let w = 1.0 - (x / taps as f64).powi(2);
            let window = if w > 0.0 {
                bessel(beta * w.sqrt()) / norm
            } else {
                0.0
            };
            table[phase * width + tap] = (sinc * window) as f32;
        }
    }
    let frames = stereo.len() / 2;
    let step = f64::from(from) / f64::from(to);
    let out_frames = (frames as f64 / step).ceil() as usize;
    let mut out = Vec::with_capacity(out_frames * 2);
    for i in 0..out_frames {
        let at = i as f64 * step;
        let whole = at.floor() as i64;
        let position = (at - whole as f64) * SRC_PHASES as f64;
        let phase = position.floor() as usize;
        let blend = (position - phase as f64) as f32;
        let (near, far) = (
            &table[phase * width..(phase + 1) * width],
            &table[(phase + 1) * width..(phase + 2) * width],
        );
        let (mut l, mut r) = (0f32, 0f32);
        for tap in 0..width {
            let source = whole + tap as i64 - (taps as i64 - 1);
            if source >= 0 && (source as usize) < frames {
                let c = near[tap] + (far[tap] - near[tap]) * blend;
                l += c * stereo[source as usize * 2];
                r += c * stereo[source as usize * 2 + 1];
            }
        }
        out.push(l);
        out.push(r);
    }
    out
}

/// An MP3 (320 kbps unless chosen otherwise) from the 32-bit float `wav` into `out`, gapless, turned down instead of clipped
/// when the mix goes over full scale.
pub fn mp3(wav: &Path, out: &Path, rate: u32) -> Result<(), String> {
    let lame = |e: mp3lame_encoder::BuildError| format!("MP3: {e:?}");
    let mut builder = Builder::new().ok_or("MP3: the encoder could not start")?;
    builder.set_num_channels(2).map_err(lame)?;
    builder.set_sample_rate(rate).map_err(lame)?;
    match mp3_quality() {
        Mp3Quality::V0 => {
            builder.set_vbr_mode(VbrMode::Mtrh).map_err(lame)?;
            builder.set_vbr_quality(Quality::Best).map_err(lame)?;
        }
        fixed => {
            let rate = match fixed {
                Mp3Quality::Kbps256 => Bitrate::Kbps256,
                Mp3Quality::Kbps192 => Bitrate::Kbps192,
                Mp3Quality::Kbps128 => Bitrate::Kbps128,
                _ => Bitrate::Kbps320,
            };
            builder.set_brate(rate).map_err(lame)?;
        }
    }
    builder.set_quality(Quality::Best).map_err(lame)?;
    // The info frame at the start says how much the encoder added before and after the sound,
    // so players and other programs trim it (gapless).
    builder.set_to_write_vbr_tag(true).map_err(lame)?;
    if rate > MP3_MAX_RATE {
        builder
            .set_output_sample_rate(std::num::NonZeroU32::new(MP3_MAX_RATE))
            .map_err(lame)?;
    }
    let mut encoder = builder.build().map_err(lame)?;

    // A mix over full scale is turned down to just under it rather than clipped.
    let peak = peak_of(wav).map_err(|e| e.to_string())?;
    let gain = if peak > 1.0 { 0.989 / peak } else { 1.0 };
    let mut input = BufReader::new(std::fs::File::open(wav).map_err(|e| e.to_string())?);
    input
        .seek(SeekFrom::Start(WAV_HEADER))
        .map_err(|e| e.to_string())?;
    let part = part_of(out);
    let mut write = || -> Result<(), String> {
        let mut file =
            std::io::BufWriter::new(std::fs::File::create(&part).map_err(|e| e.to_string())?);
        let mut raw = vec![0u8; MP3_FRAMES * 2 * 4];
        let mut samples: Vec<f32> = Vec::with_capacity(MP3_FRAMES * 2);
        let mut coded: Vec<u8> = Vec::new();
        loop {
            let got = read_all(&mut input, &mut raw).map_err(|e| e.to_string())?;
            if got == 0 {
                break;
            }
            samples.clear();
            samples.extend(
                raw[..got]
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| (f32::from_le_bytes(*b) * gain).clamp(-1.0, 1.0)),
            );
            let frames = samples.len() / 2;
            coded.clear();
            coded.reserve(max_required_buffer_size(frames));
            encoder
                .encode_to_vec(InterleavedPcm(&samples[..frames * 2]), &mut coded)
                .map_err(|e| format!("MP3: {e:?}"))?;
            file.write_all(&coded).map_err(|e| e.to_string())?;
        }
        coded.clear();
        coded.reserve(max_required_buffer_size(MP3_FRAMES));
        encoder
            .flush_to_vec::<FlushNoGap>(&mut coded)
            .map_err(|e| format!("MP3: {e:?}"))?;
        file.write_all(&coded).map_err(|e| e.to_string())?;
        file.flush().map_err(|e| e.to_string())?;
        // The info frame, now that the length is known, over the placeholder at the start.
        let mut tag = Vec::with_capacity(encoder.lame_tag_size());
        if encoder.lame_tag_encode_to_vec(&mut tag).is_some() {
            let file = file.get_mut();
            file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
            file.write_all(&tag).map_err(|e| e.to_string())?;
        }
        Ok(())
    };
    let written = write();
    if written.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    written?;
    std::fs::rename(&part, out).map_err(|e| e.to_string())
}

/// The highest sample in the 32-bit float `wav`.
fn peak_of(wav: &Path) -> std::io::Result<f32> {
    let mut input = BufReader::new(std::fs::File::open(wav)?);
    input.seek(SeekFrom::Start(WAV_HEADER))?;
    let mut raw = vec![0u8; MP3_FRAMES * 2 * 4];
    let mut peak = 0.0f32;
    loop {
        let got = read_all(&mut input, &mut raw)?;
        if got == 0 {
            return Ok(peak);
        }
        for b in raw[..got].as_chunks::<4>().0 {
            let v = f32::from_le_bytes(*b).abs();
            if v.is_finite() {
                peak = peak.max(v);
            }
        }
    }
}

/// Fills `buf` as far as the file goes.
pub fn read_all(input: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut got = 0;
    while got < buf.len() {
        match input.read(&mut buf[got..])? {
            0 => break,
            n => got += n,
        }
    }
    Ok(got)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f64, rate: u32, seconds: f64) -> Vec<f32> {
        let frames = (seconds * f64::from(rate)) as usize;
        (0..frames)
            .flat_map(|i| {
                let s =
                    (0.5 * (std::f64::consts::TAU * hz * i as f64 / f64::from(rate)).sin()) as f32;
                [s, s]
            })
            .collect()
    }

    fn rms(stereo: &[f32]) -> f32 {
        let middle = &stereo[stereo.len() / 4..stereo.len() * 3 / 4];
        (middle.iter().map(|s| s * s).sum::<f32>() / middle.len() as f32).sqrt()
    }

    #[test]
    fn a_tone_keeps_its_pitch_level_and_length_at_another_rate() {
        for (from, to) in [
            (48_000, 44_100),
            (44_100, 48_000),
            (48_000, 96_000),
            (96_000, 44_100),
        ] {
            let out = resample(&sine(1000.0, from, 0.5), from, to);
            let expected = 0.5 * f64::from(to);
            assert!(
                ((out.len() / 2) as f64 - expected).abs() < 2.0,
                "{from} to {to}"
            );
            assert!(
                (rms(&out) - rms(&sine(1000.0, to, 0.5))).abs() < 0.002,
                "{from} to {to}"
            );
            // The same as the tone made at that rate, by how often it crosses zero.
            let left: Vec<f32> = out.iter().step_by(2).copied().collect();
            let crossings = left
                .windows(2)
                .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
                .count();
            assert!(crossings.abs_diff(500) <= 2, "{from} to {to}: {crossings}");
        }
    }

    #[test]
    fn going_down_leaves_out_what_the_lower_rate_cannot_hold() {
        // 30 kHz cannot exist at 44.1 kHz. It must vanish, not fold down to 14.1 kHz.
        let out = resample(&sine(30_000.0, 96_000, 0.25), 96_000, 44_100);
        assert!(rms(&out) < 0.0005, "{}", rms(&out));
    }

    #[test]
    fn the_same_rate_is_left_alone() {
        let tone = sine(1000.0, 48_000, 0.1);
        assert_eq!(resample(&tone, 48_000, 48_000), tone);
    }
}

/// A sample rate as the 80-bit number AIFF keeps it in.
fn extended(rate: u32) -> [u8; 10] {
    let mut out = [0u8; 10];
    if rate == 0 {
        return out;
    }
    let shift = rate.leading_zeros();
    let exponent = (16383 + 31 - shift) as u16;
    let mantissa = u64::from(rate) << (32 + shift);
    out[..2].copy_from_slice(&exponent.to_be_bytes());
    out[2..].copy_from_slice(&mantissa.to_be_bytes());
    out
}

/// An AIFF file from the 16-bit or 24-bit stereo `wav`: the same samples, most significant byte
/// first, as AIFF has them.
pub fn aiff(wav: &Path, out: &Path, bits: usize, rate: u32) -> Result<(), String> {
    let bytes = std::fs::read(wav).map_err(|e| e.to_string())?;
    let data = bytes.get(WAV_HEADER as usize..).unwrap_or(&[]);
    let width = if bits == 16 { 2 } else { 3 };
    let frames = data.len() / (width * 2);
    let sound = frames * width * 2;
    let mut file = Vec::with_capacity(sound + 54);
    file.extend_from_slice(b"FORM");
    file.extend_from_slice(&((4 + 26 + 16 + sound) as u32).to_be_bytes());
    file.extend_from_slice(b"AIFF");
    file.extend_from_slice(b"COMM");
    file.extend_from_slice(&18u32.to_be_bytes());
    file.extend_from_slice(&2u16.to_be_bytes());
    file.extend_from_slice(&(frames as u32).to_be_bytes());
    file.extend_from_slice(&((width * 8) as u16).to_be_bytes());
    file.extend_from_slice(&extended(rate));
    file.extend_from_slice(b"SSND");
    file.extend_from_slice(&((8 + sound) as u32).to_be_bytes());
    file.extend_from_slice(&[0u8; 8]);
    for sample in data[..sound].chunks_exact(width) {
        file.extend(sample.iter().rev());
    }
    let part = part_of(out);
    std::fs::write(&part, &file).map_err(|e| e.to_string())?;
    std::fs::rename(&part, out).map_err(|e| e.to_string())
}
