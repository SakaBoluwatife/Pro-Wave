//! The spatial mix as a file: an ADM BW64, the open format of ITU-R BS.2076 and BS.2088, for other
//! spatial tools to read. Two shapes ([`Shape`]):
//!
//! - a 7.1.4 bed under the common definitions of ITU-R BS.2094 (pack `AP_00010017`), with the objects
//!   mixed into it or following it as stereo pairs (two tracks each);
//! - a 7.1.2 bed (the bed of the Atmos workflow), defined in the file itself as DirectSpeakers, with
//!   the objects mixed into it or following it as mono tracks: up to 118 beside the bed's 10.
//!
//! Every object has its moves written down.
use crate::model::{AutoTarget, Project};
use ovrtone_engine::{lane_value, ObjectAxis, ObjectPlace, SPATIAL_CHANNELS as BUS_CHANNELS};

/// The bed's channels in the file: 7.1.4, the first twelve of the bus.
const SPATIAL_CHANNELS: usize = 12;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// The pack of the common definitions for 7.1.4.
const PACK: &str = "AP_00010017";
/// The channel of the common definitions for each channel of the spatial bus: the bus has rear before
/// side and left before right, and the definitions' numbers do not follow that order.
const TRACK_FORMATS: [&str; SPATIAL_CHANNELS] = [
    "AT_00010001_01", // M+030, left
    "AT_00010002_01", // M-030, right
    "AT_00010003_01", // M+000, centre
    "AT_00010004_01", // LFE
    "AT_0001001c_01", // M+135, rear left
    "AT_0001001d_01", // M-135, rear right
    "AT_0001000a_01", // M+090, side left
    "AT_0001000b_01", // M-090, side right
    "AT_00010022_01", // U+045, top front left
    "AT_00010023_01", // U-045, top front right
    "AT_0001001e_01", // U+135, top rear left
    "AT_0001001f_01", // U-135, top rear right
];
/// What the placeholder at the start of the file comes to if it must become a `ds64`.
const DS64_BODY: usize = 28;
/// Past this, the sizes do not fit the fields of a plain RIFF.
const RIFF_LIMIT: u64 = 0xFFFF_FFFF;
/// The most tracks a file has, which is what spatial tools take.
const MAX_FILE_TRACKS: usize = 128;

/// The bed and the objects of a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    /// A 7.1.2 bed (else 7.1.4).
    pub bed712: bool,
    /// Each object one mono track (else a stereo pair).
    pub mono: bool,
}

impl Shape {
    pub const SEVEN_ONE_FOUR: Shape = Shape { bed712: false, mono: false };
    pub const SEVEN_ONE_TWO: Shape = Shape { bed712: true, mono: true };

    /// The bus channels of the bed, in the file's order.
    pub fn bed(self) -> &'static [usize] {
        if self.bed712 {
            &[0, 1, 2, 3, 4, 5, 6, 7, 14, 15]
        } else {
            &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
        }
    }

    fn per_object(self) -> usize {
        if self.mono { 1 } else { 2 }
    }

    /// The tracks of a file with `objects` objects.
    fn tracks(self, objects: usize) -> usize {
        self.bed().len() + objects * self.per_object()
    }

    /// The most objects a file of this shape holds.
    pub fn max_objects(self) -> usize {
        (MAX_FILE_TRACKS - self.bed().len()) / self.per_object()
    }

    /// The pack, and each bed track's format and (for a bed defined in the file) its speaker label,
    /// ADM azimuth and elevation.
    fn bed_formats(self) -> (&'static str, Vec<(String, Option<(&'static str, f32, f32)>)>) {
        if !self.bed712 {
            return (PACK, TRACK_FORMATS.iter().map(|f| (f.to_string(), None)).collect());
        }
        // The format's azimuth is to the left; the bus's to the right.
        const SPEAKERS: [(&str, f32, f32); 10] = [
            ("M+030", 30.0, 0.0),
            ("M-030", -30.0, 0.0),
            ("M+000", 0.0, 0.0),
            ("LFE1", 0.0, -30.0),
            ("M+135", 135.0, 0.0),
            ("M-135", -135.0, 0.0),
            ("M+090", 90.0, 0.0),
            ("M-090", -90.0, 0.0),
            ("U+090", 90.0, 45.0),
            ("U-090", -90.0, 45.0),
        ];
        let formats = SPEAKERS
            .iter()
            .enumerate()
            .map(|(k, s)| (format!("AT_0001{:04x}_01", 0x1001 + k), Some(*s)))
            .collect();
        (PACK_712, formats)
    }
}

/// The pack of the 7.1.2 bed, defined in the file (IDs from 0x1000 are for definitions of one's own).
const PACK_712: &str = "AP_00011001";
/// How often an object's place is written down, in seconds, when it moves.
const BLOCK_SECONDS: f64 = 0.1;

/// A stretch of time with an object in one place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Block {
    pub start: f64,
    pub duration: f64,
    pub place: ObjectPlace,
}

/// A strip that goes to the file as tracks of its own.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectTrack {
    pub track: u16,
    pub name: String,
    pub blocks: Vec<Block>,
}

/// The tracks of `p` that go to a file as objects: only what would have played from Stereo Out, as a
/// strip sent to a bus or an output is not an object. With their places over the range.
pub fn object_tracks(
    p: &Project,
    from_beat: f64,
    to_beat: f64,
    shape: Shape,
) -> Result<Vec<ObjectTrack>, String> {
    let map = p.tempo_map();
    let (start, end) = (map.seconds_at(from_beat), map.seconds_at(to_beat));
    let length = (end - start).max(0.0);
    let mut found = Vec::new();
    for (i, t) in p.tracks.iter().enumerate().take(ovrtone_engine::MAX_TRACKS) {
        let Some(own) = t.object else {
            continue;
        };
        // Panned in the bed: it is in the bed's channels already.
        if t.bed_pan {
            continue;
        }
        if t.output != 0 || t.hardware_out != 0 {
            continue;
        }
        // Going through a spatial bus: it is in the bed as the bus places it.
        if own.bus != 0 {
            continue;
        }
        let merged = crate::automation::merged_lanes(p, i);
        let lanes: Vec<(ObjectAxis, Vec<ovrtone_engine::AutoPoint>)> = ObjectAxis::ALL
            .into_iter()
            .map(|axis| {
                let target = AutoTarget::Object(axis);
                let from_regions = merged.iter().find(|(x, _)| *x == target).map(|(_, l)| l.clone());
                let lane = from_regions.unwrap_or_else(|| t.auto.lane(target).to_vec());
                (axis, if t.auto.follows(target) { lane } else { Vec::new() })
            })
            .collect();
        let place_at = |seconds: f64| {
            let beat = map.beat_at(seconds);
            let mut place = own;
            for (axis, lane) in &lanes {
                if let Some(v) = lane_value(lane, beat) {
                    place.set(*axis, v);
                }
            }
            place
        };
        let moves = lanes.iter().any(|(_, l)| !l.is_empty());
        let step = if moves { BLOCK_SECONDS } else { length.max(BLOCK_SECONDS) };
        let mut blocks: Vec<Block> = Vec::new();
        let mut at = 0.0;
        while at < length || blocks.is_empty() {
            let place = place_at(start + at);
            let next = (at + step).min(length.max(at + step.min(BLOCK_SECONDS)));
            match blocks.last_mut() {
                Some(last) if last.place == place => last.duration = next - last.start,
                _ => blocks.push(Block {
                    start: at,
                    duration: next - at,
                    place,
                }),
            }
            at = next;
        }
        found.push(ObjectTrack {
            track: i as u16,
            name: t.name.clone(),
            blocks,
        });
    }
    if found.len() > shape.max_objects() {
        return Err(format!(
            "{} objects need {} tracks and the file can hold {MAX_FILE_TRACKS}: {} objects at most",
            found.len(),
            shape.tracks(found.len()),
            shape.max_objects()
        ));
    }
    Ok(found)
}

/// The name of an audioTrackUID, by its place from 1.
fn track_uid(track: usize) -> String {
    format!("ATU_{track:08x}")
}

/// The IDs of the object channel `n` (from 0): pack, channel, stream and track format.
fn object_ids(n: usize) -> (String, String, String, String) {
    let id = 0x1001 + n;
    (
        format!("AP_0003{id:04x}"),
        format!("AC_0003{id:04x}"),
        format!("AS_0003{id:04x}"),
        format!("AT_0003{id:04x}_01"),
    )
}

/// The file's `chna` chunk body: which track is which speaker or object.
fn chna(objects: &[ObjectTrack], shape: Shape) -> Vec<u8> {
    let tracks = shape.tracks(objects.len());
    let bed = shape.bed().len();
    let (bed_pack, bed_formats) = shape.bed_formats();
    let mut body = Vec::new();
    body.extend_from_slice(&(tracks as u16).to_le_bytes());
    body.extend_from_slice(&(tracks as u16).to_le_bytes());
    let mut entry = |track: usize, format: &str, pack: &str| {
        body.extend_from_slice(&(track as u16).to_le_bytes());
        body.extend_from_slice(track_uid(track).as_bytes());
        body.extend_from_slice(format.as_bytes());
        body.extend_from_slice(pack.as_bytes());
        body.push(0);
    };
    for (k, (format, _)) in bed_formats.iter().enumerate() {
        entry(k + 1, format, bed_pack);
    }
    for n in 0..objects.len() * shape.per_object() {
        let (pack, _, _, track_format) = object_ids(n);
        entry(bed + n + 1, &track_format, &pack);
    }
    body
}

fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Seconds as the format writes times.
fn time(seconds: f64) -> String {
    let seconds = seconds.max(0.0);
    let hours = (seconds / 3600.0) as u64;
    let minutes = ((seconds - hours as f64 * 3600.0) / 60.0) as u64;
    let rest = seconds - hours as f64 * 3600.0 - minutes as f64 * 60.0;
    format!("{hours:02}:{minutes:02}:{rest:08.5}")
}

/// A place in the room as the format's box: X to the right, Y ahead, Z up, each -1..=1. The walls of
/// the box are where the speakers of the room are.
fn in_the_box(azimuth: f32, elevation: f32) -> [f32; 3] {
    let (a, e) = (azimuth.to_radians(), elevation.to_radians());
    let (x, y) = (a.sin(), a.cos());
    let wall = x.abs().max(y.abs()).max(1e-6);
    [x / wall * e.cos(), y / wall * e.cos(), e.sin()]
}

/// The file's `axml` chunk body: the programme, what is in it, and its tracks.
fn axml(title: &str, objects: &[ObjectTrack], shape: Shape) -> String {
    use std::fmt::Write as _;
    let title = escaped(title);
    let bed = shape.bed().len();
    let (bed_pack, bed_formats) = shape.bed_formats();
    let bed_uids: String = (1..=bed)
        .map(|t| format!("          <audioTrackUIDRef>{}</audioTrackUIDRef>\n", track_uid(t)))
        .collect();
    let mut contents = String::from("          <audioObjectIDRef>AO_1001</audioObjectIDRef>\n");
    let mut objects_xml = format!(
        "        <audioObject audioObjectID=\"AO_1001\" audioObjectName=\"Bed\">\n          <audioPackFormatIDRef>{bed_pack}</audioPackFormatIDRef>\n{bed_uids}        </audioObject>\n"
    );
    let mut formats = String::new();
    // A bed defined in the file: its pack, and each speaker's channel, stream and track formats.
    if bed_formats.iter().any(|(_, s)| s.is_some()) {
        let refs: String = (0..bed_formats.len())
            .map(|k| format!("          <audioChannelFormatIDRef>AC_0001{:04x}</audioChannelFormatIDRef>\n", 0x1001 + k))
            .collect();
        let _ = write!(
            formats,
            "        <audioPackFormat audioPackFormatID=\"{bed_pack}\" audioPackFormatName=\"7.1.2\" typeLabel=\"0001\" typeDefinition=\"DirectSpeakers\">\n{refs}        </audioPackFormat>\n"
        );
        for (k, (track_format, speaker)) in bed_formats.iter().enumerate() {
            let Some((label, az, el)) = speaker else {
                continue;
            };
            let id = format!("{:04x}", 0x1001 + k);
            let lfe = if *label == "LFE1" {
                "          <frequency typeDefinition=\"lowPass\">120</frequency>\n"
            } else {
                ""
            };
            let _ = write!(
                formats,
                "        <audioChannelFormat audioChannelFormatID=\"AC_0001{id}\" audioChannelFormatName=\"{label}\" typeLabel=\"0001\" typeDefinition=\"DirectSpeakers\">\n{lfe}          <audioBlockFormat audioBlockFormatID=\"AB_0001{id}_00000001\">\n            <speakerLabel>{label}</speakerLabel>\n            <position coordinate=\"azimuth\">{az:.1}</position>\n            <position coordinate=\"elevation\">{el:.1}</position>\n            <position coordinate=\"distance\">1.0</position>\n          </audioBlockFormat>\n        </audioChannelFormat>\n        <audioStreamFormat audioStreamFormatID=\"AS_0001{id}\" audioStreamFormatName=\"PCM_{label}\" formatLabel=\"0001\" formatDefinition=\"PCM\">\n          <audioChannelFormatIDRef>AC_0001{id}</audioChannelFormatIDRef>\n          <audioTrackFormatIDRef>{track_format}</audioTrackFormatIDRef>\n        </audioStreamFormat>\n        <audioTrackFormat audioTrackFormatID=\"{track_format}\" audioTrackFormatName=\"PCM_{label}\" formatLabel=\"0001\" formatDefinition=\"PCM\">\n          <audioStreamFormatIDRef>AS_0001{id}</audioStreamFormatIDRef>\n        </audioTrackFormat>\n"
            );
        }
    }
    let mut uids: String = bed_formats
        .iter()
        .enumerate()
        .map(|(k, (format, _))| {
            format!(
                "        <audioTrackUID UID=\"{}\" sampleRate=\"48000\" bitDepth=\"24\">\n          <audioTrackFormatIDRef>{format}</audioTrackFormatIDRef>\n          <audioPackFormatIDRef>{bed_pack}</audioPackFormatIDRef>\n        </audioTrackUID>\n",
                track_uid(k + 1)
            )
        })
        .collect();
    let sides: &[(&str, f32)] = if shape.mono { &[("", 0.0)] } else { &[(" L", -1.0), (" R", 1.0)] };
    for (o, object) in objects.iter().enumerate() {
        for (side, &(word, offset)) in sides.iter().enumerate() {
            let n = o * sides.len() + side;
            let (pack, channel, stream, track_format) = object_ids(n);
            let name = escaped(&format!("{}{word}", object.name));
            let object_id = format!("AO_{:04x}", 0x1002 + n);
            let uid = track_uid(bed + n + 1);
            let _ = writeln!(contents, "          <audioObjectIDRef>{object_id}</audioObjectIDRef>");
            let _ = write!(
                objects_xml,
                "        <audioObject audioObjectID=\"{object_id}\" audioObjectName=\"{name}\">\n          <audioPackFormatIDRef>{pack}</audioPackFormatIDRef>\n          <audioTrackUIDRef>{uid}</audioTrackUIDRef>\n        </audioObject>\n"
            );
            let _ = write!(
                formats,
                "        <audioPackFormat audioPackFormatID=\"{pack}\" audioPackFormatName=\"{name}\" typeLabel=\"0003\" typeDefinition=\"Objects\">\n          <audioChannelFormatIDRef>{channel}</audioChannelFormatIDRef>\n        </audioPackFormat>\n        <audioChannelFormat audioChannelFormatID=\"{channel}\" audioChannelFormatName=\"{name}\" typeLabel=\"0003\" typeDefinition=\"Objects\">\n"
            );
            let channel_digits = &channel[3..];
            for (b, block) in object.blocks.iter().enumerate() {
                let place = block.place;
                let [x, y, z] = in_the_box(place.azimuth + offset * place.spread * 45.0, place.elevation)
                    .map(|v| v * place.radius.clamp(0.0, 1.0));
                // Snap is the format's channel lock; divergence its phantom split.
                let mut extra = String::new();
                if place.snap {
                    extra.push_str("            <channelLock>1</channelLock>\n");
                }
                if place.divergence > 0.0 {
                    let _ = writeln!(
                        extra,
                        "            <objectDivergence positionRange=\"0.5\">{:.5}</objectDivergence>",
                        place.divergence.clamp(0.0, 1.0)
                    );
                }
                let _ = write!(
                    formats,
                    "          <audioBlockFormat audioBlockFormatID=\"AB_{channel_digits}_{:08x}\" rtime=\"{}\" duration=\"{}\">\n            <cartesian>1</cartesian>\n            <position coordinate=\"X\">{x:.5}</position>\n            <position coordinate=\"Y\">{y:.5}</position>\n            <position coordinate=\"Z\">{z:.5}</position>\n            <width>{:.5}</width>\n{extra}          </audioBlockFormat>\n",
                    b + 1,
                    time(block.start),
                    time(block.duration),
                    place.size,
                );
            }
            let _ = write!(
                formats,
                "        </audioChannelFormat>\n        <audioStreamFormat audioStreamFormatID=\"{stream}\" audioStreamFormatName=\"PCM_{name}\" formatLabel=\"0001\" formatDefinition=\"PCM\">\n          <audioChannelFormatIDRef>{channel}</audioChannelFormatIDRef>\n          <audioTrackFormatIDRef>{track_format}</audioTrackFormatIDRef>\n        </audioStreamFormat>\n        <audioTrackFormat audioTrackFormatID=\"{track_format}\" audioTrackFormatName=\"PCM_{name}\" formatLabel=\"0001\" formatDefinition=\"PCM\">\n          <audioStreamFormatIDRef>{stream}</audioStreamFormatIDRef>\n        </audioTrackFormat>\n"
            );
            let _ = write!(
                uids,
                "        <audioTrackUID UID=\"{uid}\" sampleRate=\"48000\" bitDepth=\"24\">\n          <audioTrackFormatIDRef>{track_format}</audioTrackFormatIDRef>\n          <audioPackFormatIDRef>{pack}</audioPackFormatIDRef>\n        </audioTrackUID>\n"
            );
        }
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<ebuCoreMain xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns=\"urn:ebu:metadata-schema:ebuCore_2014\" schema=\"EBU_CORE_20140201.xsd\" xml:lang=\"en\">
  <coreMetadata>
    <format>
      <audioFormatExtended version=\"ITU-R_BS.2076-2\">
        <audioProgramme audioProgrammeID=\"APR_1001\" audioProgrammeName=\"{title}\">
          <audioContentIDRef>ACO_1001</audioContentIDRef>
        </audioProgramme>
        <audioContent audioContentID=\"ACO_1001\" audioContentName=\"{title}\">
{contents}        </audioContent>
{objects_xml}{formats}{uids}      </audioFormatExtended>
    </format>
  </coreMetadata>
</ebuCoreMain>
"
    )
}

/// Writes one chunk, padded to an even length as the format wants.
fn write_chunk(file: &mut impl Write, id: &[u8; 4], body: &[u8]) -> std::io::Result<()> {
    file.write_all(id)?;
    file.write_all(&(body.len() as u32).to_le_bytes())?;
    file.write_all(body)?;
    if body.len() % 2 == 1 {
        file.write_all(&[0])?;
    }
    Ok(())
}

/// A file being written, a block at a time: the bed's channels, then two for each object.
pub struct AdmWriter {
    shape: Shape,
    file: std::io::BufWriter<std::fs::File>,
    path: PathBuf,
    frames: u64,
    tracks: usize,
    /// Where the sound begins.
    data_at: u64,
}

fn sample_bytes(v: f32) -> [u8; 3] {
    let v = if v.is_finite() { v.clamp(-1.0, 1.0) } else { 0.0 };
    let scaled = (f64::from(v) * 8_388_607.0).round() as i32;
    let [a, b, c, _] = scaled.to_le_bytes();
    [a, b, c]
}

impl AdmWriter {
    pub fn create(path: &Path, rate: u32, objects: usize, shape: Shape) -> std::io::Result<Self> {
        let tracks = shape.tracks(objects);
        let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
        file.write_all(b"RIFF\0\0\0\0WAVE")?;
        // Room for a `ds64`, should the file come out too long for plain sizes.
        write_chunk(&mut file, b"JUNK", &[0; DS64_BODY])?;
        let frame_bytes = (tracks * 3) as u16;
        let mut fmt = Vec::new();
        fmt.extend_from_slice(&1u16.to_le_bytes());
        fmt.extend_from_slice(&(tracks as u16).to_le_bytes());
        fmt.extend_from_slice(&rate.to_le_bytes());
        fmt.extend_from_slice(&(rate * u32::from(frame_bytes)).to_le_bytes());
        fmt.extend_from_slice(&frame_bytes.to_le_bytes());
        fmt.extend_from_slice(&24u16.to_le_bytes());
        write_chunk(&mut file, b"fmt ", &fmt)?;
        file.write_all(b"data\0\0\0\0")?;
        let data_at = file.stream_position()?;
        Ok(Self {
            shape,
            file,
            path: path.to_path_buf(),
            frames: 0,
            tracks,
            data_at,
        })
    }

    /// Takes the frames of a block, leaving out the first `skip`: the bus's, [`BUS_CHANNELS`] to a
    /// frame of which the bed's first [`SPATIAL_CHANNELS`] are kept, and two to a frame for each object.
    pub fn write(&mut self, bed: &[f32], objects: &[Vec<f32>], skip: usize) -> std::io::Result<()> {
        let frames = bed.len() / BUS_CHANNELS;
        for f in skip..frames {
            let frame = &bed[f * BUS_CHANNELS..(f + 1) * BUS_CHANNELS];
            for &c in self.shape.bed() {
                self.file.write_all(&sample_bytes(frame[c]))?;
            }
            for object in objects {
                let l = object.get(f * 2).copied().unwrap_or(0.0);
                let r = object.get(f * 2 + 1).copied().unwrap_or(0.0);
                if self.shape.mono {
                    // One track: both sides together, at the level the pair had.
                    self.file.write_all(&sample_bytes((l + r) * 0.5))?;
                } else {
                    self.file.write_all(&sample_bytes(l))?;
                    self.file.write_all(&sample_bytes(r))?;
                }
            }
            self.frames += 1;
        }
        Ok(())
    }

    /// Ends the file: its metadata, then the sizes, now they are known.
    pub fn finish(mut self, title: &str, objects: &[ObjectTrack]) -> std::io::Result<PathBuf> {
        let data = self.frames * (self.tracks as u64 * 3);
        if data % 2 == 1 {
            self.file.write_all(&[0])?;
        }
        write_chunk(&mut self.file, b"chna", &chna(objects, self.shape))?;
        write_chunk(&mut self.file, b"axml", axml(title, objects, self.shape).as_bytes())?;
        let end = self.file.stream_position()?;
        let riff = end - 8;
        if riff > RIFF_LIMIT || data > RIFF_LIMIT {
            // Too long for plain sizes: BW64, with the real ones in the `ds64` the space was kept for.
            self.file.seek(SeekFrom::Start(0))?;
            self.file.write_all(b"BW64")?;
            self.file.write_all(&u32::MAX.to_le_bytes())?;
            self.file.seek(SeekFrom::Start(12))?;
            self.file.write_all(b"ds64")?;
            self.file.write_all(&(DS64_BODY as u32).to_le_bytes())?;
            self.file.write_all(&riff.to_le_bytes())?;
            self.file.write_all(&data.to_le_bytes())?;
            self.file.write_all(&self.frames.to_le_bytes())?;
            self.file.write_all(&0u32.to_le_bytes())?;
            self.file.seek(SeekFrom::Start(self.data_at - 4))?;
            self.file.write_all(&u32::MAX.to_le_bytes())?;
        } else {
            self.file.seek(SeekFrom::Start(4))?;
            self.file.write_all(&(riff as u32).to_le_bytes())?;
            self.file.seek(SeekFrom::Start(self.data_at - 4))?;
            self.file.write_all(&(data as u32).to_le_bytes())?;
        }
        self.file.flush()?;
        Ok(self.path)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn chunk<'a>(file: &'a [u8], id: &[u8; 4]) -> &'a [u8] {
        let mut at = 12;
        while at + 8 <= file.len() {
            let size = u32::from_le_bytes(file[at + 4..at + 8].try_into().unwrap()) as usize;
            if &file[at..at + 4] == id {
                return &file[at + 8..at + 8 + size];
            }
            at += 8 + size + size % 2;
        }
        panic!("no chunk {}", String::from_utf8_lossy(id));
    }

    #[test]
    fn the_file_holds_its_sound_and_says_which_track_is_which_speaker() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mix.wav");
        let mut writer = AdmWriter::create(&path, 48_000, 0, Shape::SEVEN_ONE_FOUR).unwrap();
        let mut frame = [0f32; BUS_CHANNELS];
        frame[7] = 0.5;
        writer.write(&frame, &[], 0).unwrap();
        writer.write(&frame, &[], 0).unwrap();
        writer.finish("A & B", &[]).unwrap();
        let file = std::fs::read(&path).unwrap();
        assert_eq!(&file[..4], b"RIFF");
        let riff = u32::from_le_bytes(file[4..8].try_into().unwrap()) as usize;
        assert_eq!(riff + 8, file.len());
        let fmt = chunk(&file, b"fmt ");
        assert_eq!(u16::from_le_bytes([fmt[2], fmt[3]]), 12);
        assert_eq!(u32::from_le_bytes(fmt[4..8].try_into().unwrap()), 48_000);
        let data = chunk(&file, b"data");
        assert_eq!(data.len(), 2 * 3 * 12);
        let at = 7 * 3;
        let v = i32::from_le_bytes([0, data[at], data[at + 1], data[at + 2]]) >> 8;
        assert!((f64::from(v) / 8_388_607.0 - 0.5).abs() < 1e-4);
        let table = chunk(&file, b"chna");
        assert_eq!(u16::from_le_bytes([table[0], table[1]]), 12);
        assert_eq!(table.len(), 4 + 12 * 40, "each entry is 40 bytes");
        // The eighth track, the right side speaker, is M-090.
        let eighth = &table[4 + 7 * 40..4 + 8 * 40];
        assert_eq!(&eighth[..2], &[8, 0]);
        assert_eq!(&eighth[2..14], b"ATU_00000008");
        assert_eq!(&eighth[14..28], b"AT_0001000b_01");
        assert_eq!(&eighth[28..39], b"AP_00010017");
        let xml = String::from_utf8(chunk(&file, b"axml").to_vec()).unwrap();
        assert!(xml.contains("audioProgrammeName=\"A &amp; B\""));
        assert_eq!(xml.matches("<audioTrackUID UID=").count(), 12);
        assert_eq!(xml.matches("<audioTrackUIDRef>").count(), 12);
    }

    #[test]
    fn the_box_has_the_room_in_it() {
        let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3);
        assert!(near(in_the_box(0.0, 0.0), [0.0, 1.0, 0.0]), "ahead");
        assert!(near(in_the_box(90.0, 0.0), [1.0, 0.0, 0.0]), "right");
        assert!(near(in_the_box(-45.0, 0.0), [-1.0, 1.0, 0.0]), "a front corner");
        assert!(near(in_the_box(30.0, 90.0), [0.0, 0.0, 1.0]), "overhead");
    }

    #[test]
    fn times_are_as_the_format_writes_them() {
        assert_eq!(time(0.0), "00:00:00.00000");
        assert_eq!(time(3661.5), "01:01:01.50000");
    }

    #[test]
    fn each_object_has_its_ids_and_tracks_in_the_metadata() {
        let place = ObjectPlace {
            azimuth: 0.0,
            elevation: 0.0,
            size: 0.0,
            lfe: 0.0,
            ..ObjectPlace::default()
        };
        let object = ObjectTrack {
            track: 0,
            name: "Vox".to_string(),
            blocks: vec![Block {
                start: 0.0,
                duration: 1.0,
                place,
            }],
        };
        let table = chna(std::slice::from_ref(&object), Shape::SEVEN_ONE_FOUR);
        assert_eq!(u16::from_le_bytes([table[0], table[1]]), 14);
        let thirteenth = &table[4 + 12 * 40..4 + 13 * 40];
        assert_eq!(&thirteenth[2..14], b"ATU_0000000d");
        assert_eq!(&thirteenth[14..28], b"AT_00031001_01");
        assert_eq!(&thirteenth[28..39], b"AP_00031001");
        let xml = axml("x", &[object], Shape::SEVEN_ONE_FOUR);
        assert!(xml.contains("audioObjectName=\"Vox L\""));
        assert!(xml.contains("audioBlockFormatID=\"AB_00031002_00000001\""));
        assert_eq!(xml.matches("<audioObject ").count(), 3, "the bed and two");
    }
}
