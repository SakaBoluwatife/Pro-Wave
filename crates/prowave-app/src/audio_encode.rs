//! Turns a finished WAV file into FLAC or MP3. A bounce is rendered to WAV first and encoded after,
//! so the render itself is the same for every format.
use mp3lame_encoder::{
    max_required_buffer_size, Bitrate, Builder, FlushNoGap, InterleavedPcm, Quality,
};
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Where the WAV writer in bounce.rs starts the samples.
const WAV_HEADER: u64 = 44;
/// Frames given to the MP3 encoder at a time.
const MP3_FRAMES: usize = 8192;
/// The highest rate an MP3 file can have.
const MP3_MAX_RATE: u32 = 48_000;

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

/// A 320 kbps file from the 32-bit float `wav` into `out`.
pub fn mp3(wav: &Path, out: &Path, rate: u32) -> Result<(), String> {
    let lame = |e: mp3lame_encoder::BuildError| format!("MP3: {e:?}");
    let mut builder = Builder::new().ok_or("MP3: the encoder could not start")?;
    builder.set_num_channels(2).map_err(lame)?;
    builder.set_sample_rate(rate).map_err(lame)?;
    builder.set_brate(Bitrate::Kbps320).map_err(lame)?;
    builder.set_quality(Quality::Best).map_err(lame)?;
    if rate > MP3_MAX_RATE {
        builder
            .set_output_sample_rate(std::num::NonZeroU32::new(MP3_MAX_RATE))
            .map_err(lame)?;
    }
    let mut encoder = builder.build().map_err(lame)?;

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
                    .map(|b| f32::from_le_bytes(*b)),
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
        file.flush().map_err(|e| e.to_string())
    };
    let written = write();
    if written.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    written?;
    std::fs::rename(&part, out).map_err(|e| e.to_string())
}

/// Fills `buf` as far as the file goes.
fn read_all(input: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut got = 0;
    while got < buf.len() {
        match input.read(&mut buf[got..])? {
            0 => break,
            n => got += n,
        }
    }
    Ok(got)
}
