//! Spatial files other than ADM: the bed as a multichannel WAV in its own speaker order, and the mix
//! as Ambisonics (AmbiX: ACN channel order, SN3D normalisation) of the first or third order.
//!
//! Both are made from the spatial bus as the engine captures it during a render: every channel of
//! the bus a frame, in bus order (see `ovrtone_engine::SPATIAL_CHANNELS`).

use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use ovrtone_engine::{SpeakerLayout, LFE_CHANNEL, SPATIAL_CHANNELS};

/// What a multichannel file holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The channels of a layout, in its order.
    Layout(SpeakerLayout),
    /// Ambisonics of this order (1 to 3).
    Ambix(usize),
}

impl Kind {
    pub fn channels(self) -> usize {
        match self {
            Kind::Layout(l) => l.channels(),
            Kind::Ambix(order) => (order + 1) * (order + 1),
        }
    }
}

/// The speaker bits of WAVE_FORMAT_EXTENSIBLE for the bus channels that have one; a layout with a
/// channel that has none is written with no mask.
const MASK: [u32; 12] = [
    0x1, 0x2, 0x4, 0x8, 0x10, 0x20, 0x200, 0x400, 0x1000, 0x4000, 0x8000, 0x20000,
];

/// Real spherical harmonics up to the third order, ACN order, SN3D, for a direction: AmbiX has x
/// ahead, y to the left and z up, azimuth anticlockwise; the bus has azimuth to the right.
pub fn sn3d(azimuth: f32, elevation: f32) -> [f32; 16] {
    let (a, e) = ((-azimuth).to_radians(), elevation.to_radians());
    let (x, y, z) = (e.cos() * a.cos(), e.cos() * a.sin(), e.sin());
    let s3 = 3f32.sqrt();
    [
        1.0,
        y,
        z,
        x,
        s3 * x * y,
        s3 * y * z,
        0.5 * (3.0 * z * z - 1.0),
        s3 * x * z,
        s3 / 2.0 * (x * x - y * y),
        (5.0f32 / 8.0).sqrt() * y * (3.0 * x * x - y * y),
        15f32.sqrt() * x * y * z,
        (3.0f32 / 8.0).sqrt() * y * (5.0 * z * z - 1.0),
        0.5 * z * (5.0 * z * z - 3.0),
        (3.0f32 / 8.0).sqrt() * x * (5.0 * z * z - 1.0),
        15f32.sqrt() / 2.0 * z * (x * x - y * y),
        (5.0f32 / 8.0).sqrt() * x * (x * x - 3.0 * y * y),
    ]
}

/// A multichannel 24-bit WAV, written as the render goes and finished with its sizes at the end.
pub struct MultiWav {
    file: std::io::BufWriter<std::fs::File>,
    path: PathBuf,
    kind: Kind,
    /// The bed's layout: where each bus channel's speaker is, for Ambisonics.
    bed: SpeakerLayout,
    /// Each bus channel's harmonics, worked out once.
    harmonics: [[f32; 16]; SPATIAL_CHANNELS],
    frames: u64,
    rate: u32,
}

const HEADER: u64 = 68;

impl MultiWav {
    pub fn create(path: &Path, rate: u32, kind: Kind, bed: SpeakerLayout) -> std::io::Result<Self> {
        let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
        file.write_all(&[0u8; HEADER as usize])?;
        let harmonics = std::array::from_fn(|c| {
            // The bed's left and right are at the speakers' places; the LFE has no place.
            match bed.place(c) {
                Some((az, el)) if c != LFE_CHANNEL => sn3d(az, el),
                _ if c == 0 => sn3d(-30.0, 0.0),
                _ if c == 1 => sn3d(30.0, 0.0),
                _ => [0.0; 16],
            }
        });
        Ok(Self {
            file,
            path: path.to_path_buf(),
            kind,
            bed,
            harmonics,
            frames: 0,
            rate,
        })
    }

    /// Takes a block of the bus (every channel a frame), leaving out the first `skip` frames.
    pub fn write(&mut self, bus: &[f32], skip: usize) -> std::io::Result<()> {
        let frames = bus.len() / SPATIAL_CHANNELS;
        let mut out = [0f32; 16];
        for f in skip..frames {
            let frame = &bus[f * SPATIAL_CHANNELS..(f + 1) * SPATIAL_CHANNELS];
            let n = match self.kind {
                Kind::Layout(l) => {
                    for (k, &c) in l.device_order().iter().enumerate() {
                        out[k] = frame[c];
                    }
                    l.channels()
                }
                Kind::Ambix(order) => {
                    let n = (order + 1) * (order + 1);
                    out[..n].fill(0.0);
                    for (c, v) in frame.iter().enumerate() {
                        if *v == 0.0 {
                            continue;
                        }
                        for (k, o) in out[..n].iter_mut().enumerate() {
                            *o += v * self.harmonics[c][k];
                        }
                    }
                    n
                }
            };
            for v in &out[..n] {
                let s = (v.clamp(-1.0, 1.0) * 8_388_607.0).round() as i32;
                self.file.write_all(&s.to_le_bytes()[..3])?;
            }
            self.frames += 1;
        }
        Ok(())
    }

    /// Writes the header with the sizes now known, and gives the file's path.
    pub fn finish(mut self) -> std::io::Result<PathBuf> {
        let channels = self.kind.channels() as u16;
        let block = u32::from(channels) * 3;
        let data = self.frames * u64::from(block);
        let mask = match self.kind {
            Kind::Layout(l) => {
                let order = l.device_order();
                if order.iter().all(|c| *c < MASK.len()) {
                    order.iter().map(|c| MASK[*c]).fold(0, |a, b| a | b)
                } else {
                    0
                }
            }
            Kind::Ambix(_) => 0,
        };
        let _ = self.bed;
        let mut h = Vec::with_capacity(HEADER as usize);
        h.extend_from_slice(b"RIFF");
        h.extend_from_slice(&((HEADER - 8 + data).min(u64::from(u32::MAX)) as u32).to_le_bytes());
        h.extend_from_slice(b"WAVEfmt ");
        h.extend_from_slice(&40u32.to_le_bytes());
        h.extend_from_slice(&0xFFFEu16.to_le_bytes());
        h.extend_from_slice(&channels.to_le_bytes());
        h.extend_from_slice(&self.rate.to_le_bytes());
        h.extend_from_slice(&(self.rate * block).to_le_bytes());
        h.extend_from_slice(&(block as u16).to_le_bytes());
        h.extend_from_slice(&24u16.to_le_bytes());
        h.extend_from_slice(&22u16.to_le_bytes());
        h.extend_from_slice(&24u16.to_le_bytes());
        h.extend_from_slice(&mask.to_le_bytes());
        // KSDATAFORMAT_SUBTYPE_PCM.
        h.extend_from_slice(&[
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38,
            0x9B, 0x71,
        ]);
        h.extend_from_slice(b"data");
        h.extend_from_slice(&(data.min(u64::from(u32::MAX)) as u32).to_le_bytes());
        self.file.flush()?;
        let file = self.file.get_mut();
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&h)?;
        file.flush()?;
        Ok(self.path)
    }
}

/// Where a spatial render goes: an ADM file, or one of these.
pub enum Sink {
    Adm(crate::adm::AdmWriter),
    Multi(MultiWav),
}

impl Sink {
    pub fn write(&mut self, bus: &[f32], objects: &[Vec<f32>], skip: usize) -> std::io::Result<()> {
        match self {
            Sink::Adm(a) => a.write(bus, objects, skip),
            Sink::Multi(m) => m.write(bus, skip),
        }
    }

    pub fn finish(self, title: &str, objects: &[crate::adm::ObjectTrack]) -> std::io::Result<PathBuf> {
        match self {
            Sink::Adm(a) => a.finish(title, objects),
            Sink::Multi(m) => m.finish(),
        }
    }
}
