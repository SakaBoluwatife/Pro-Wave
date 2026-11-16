//! AAF (`.aaf`) sessions, as Pro Tools, Logic, Media Composer, Premiere and Resolve export them for
//! audio post: each audio track of the composition becomes a track, with its clips where they sit
//! and the part of their file they play. Audio linked to the AAF is found where the AAF says it is
//! (or beside the AAF); audio embedded in it is written out as WAV files. Fades, clip gain from
//! effects and automation are not carried over; the log says so.
//!
//! An AAF file is a Microsoft compound file. Each object is a storage holding a `properties` stream
//! (byte order, version, entry count, then each entry's property id, form and size, then the
//! values); references to other objects name their storages. The objects followed here:
//! Root > Header > ContentStorage > Mobs. A composition's slots hold sequences of source clips and
//! fillers; a source clip points at a master clip, which points at a source mob, whose descriptor
//! names its file (a locator) or whose essence is in the AAF itself.

use std::collections::HashMap;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};

use crate::session_import::{Session, SessionClip, SessionTrack};

// How a property is stored.
const SF_DATA: u16 = 0x82;
const SF_DATA_STREAM: u16 = 0x42;
const SF_STRONG_REF: u16 = 0x22;
const SF_STRONG_REF_VECTOR: u16 = 0x32;
const SF_STRONG_REF_SET: u16 = 0x3A;

// Property ids (AAF object specification).
const ROOT_HEADER: u16 = 0x0002;
const HEADER_CONTENT: u16 = 0x3B03;
const CONTENT_MOBS: u16 = 0x1901;
const CONTENT_ESSENCE: u16 = 0x1902;
const MOB_ID: u16 = 0x4401;
const MOB_NAME: u16 = 0x4402;
const MOB_SLOTS: u16 = 0x4403;
const SLOT_ID: u16 = 0x4801;
const SLOT_NAME: u16 = 0x4802;
const SLOT_SEGMENT: u16 = 0x4803;
const SLOT_EDIT_RATE: u16 = 0x4B01;
const COMPONENT_LENGTH: u16 = 0x0202;
const SEQUENCE_COMPONENTS: u16 = 0x1001;
const SOURCE_ID: u16 = 0x1101;
const SOURCE_SLOT: u16 = 0x1102;
const CLIP_START: u16 = 0x1201;
const OPERATION_INPUTS: u16 = 0x0B02;
const TRANSITION_OPERATION: u16 = 0x1801;
const SOURCE_DESCRIPTOR: u16 = 0x4701;
const DESCRIPTOR_LOCATORS: u16 = 0x2F01;
const LOCATOR_URL: u16 = 0x4001;
const SOUND_BITS: u16 = 0x3D01;
const SOUND_RATE: u16 = 0x3D03;
const SOUND_CHANNELS: u16 = 0x3D07;
const ESSENCE_MOB_ID: u16 = 0x2701;
const ESSENCE_DATA: u16 = 0x2702;

/// The tempo a session without one is placed at; positions are kept in seconds through it.
const BPM: f64 = 120.0;

/// One object's properties: id, form, value.
struct Props(Vec<(u16, u16, Vec<u8>)>);

impl Props {
    fn get(&self, pid: u16) -> Option<(u16, &[u8])> {
        self.0.iter().find(|(p, _, _)| *p == pid).map(|(_, f, v)| (*f, v.as_slice()))
    }
    fn has(&self, pid: u16) -> bool {
        self.get(pid).is_some()
    }
    fn data(&self, pid: u16) -> Option<&[u8]> {
        self.get(pid).filter(|(f, _)| *f == SF_DATA).map(|(_, v)| v)
    }
    fn i64(&self, pid: u16) -> Option<i64> {
        self.data(pid).and_then(|v| v.get(..8)).map(|b| i64::from_le_bytes(b.try_into().unwrap_or([0; 8])))
    }
    fn u32(&self, pid: u16) -> Option<u32> {
        self.data(pid).and_then(|v| v.get(..4)).map(|b| u32::from_le_bytes(b.try_into().unwrap_or([0; 4])))
    }
    /// A rational (numerator, denominator), as `f64`.
    fn rational(&self, pid: u16) -> Option<f64> {
        let v = self.data(pid)?;
        let num = i32::from_le_bytes(v.get(..4)?.try_into().ok()?);
        let den = i32::from_le_bytes(v.get(4..8)?.try_into().ok()?);
        (num != 0 && den != 0).then(|| f64::from(num) / f64::from(den))
    }
    fn string(&self, pid: u16) -> Option<String> {
        self.data(pid).map(utf16)
    }
}

/// A UTF-16LE string, up to its terminating zero.
fn utf16(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|u| *u != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

/// The AAF file being read.
struct Aaf<F: Read + Seek> {
    cf: cfb::CompoundFile<F>,
}

impl<F: Read + Seek> Aaf<F> {
    fn stream(&mut self, path: &str) -> Option<Vec<u8>> {
        let mut s = self.cf.open_stream(path).ok()?;
        let mut out = Vec::new();
        s.read_to_end(&mut out).ok()?;
        Some(out)
    }

    /// The properties of the object stored at `path` (a storage).
    fn props(&mut self, path: &str) -> Option<Props> {
        let bytes = self.stream(&join(path, "properties"))?;
        let count = usize::from(u16::from_le_bytes(bytes.get(2..4)?.try_into().ok()?));
        let mut entries = Vec::with_capacity(count);
        let mut at = 4;
        for _ in 0..count {
            let e = bytes.get(at..at + 6)?;
            entries.push((
                u16::from_le_bytes([e[0], e[1]]),
                u16::from_le_bytes([e[2], e[3]]),
                usize::from(u16::from_le_bytes([e[4], e[5]])),
            ));
            at += 6;
        }
        let mut out = Vec::with_capacity(count);
        for (pid, form, size) in entries {
            out.push((pid, form, bytes.get(at..at + size)?.to_vec()));
            at += size;
        }
        Some(Props(out))
    }

    /// The object one strong reference names.
    fn child(&mut self, path: &str, props: &Props, pid: u16) -> Option<String> {
        let (form, value) = props.get(pid)?;
        (form == SF_STRONG_REF).then(|| join(path, &utf16(value)))
    }

    /// The objects a vector or set of strong references names, in order.
    fn children(&mut self, path: &str, props: &Props, pid: u16) -> Vec<String> {
        let Some((form, value)) = props.get(pid) else {
            return Vec::new();
        };
        if form != SF_STRONG_REF_VECTOR && form != SF_STRONG_REF_SET {
            return Vec::new();
        }
        let name = utf16(value);
        let Some(index) = self.stream(&join(path, &format!("{name} index"))) else {
            return Vec::new();
        };
        let count = index
            .get(..4)
            .map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap_or([0; 4])) as usize);
        // A vector's index: count, next key, last free key, then a key for each.
        // A set's: the same, then the key property and its size, then key, count and key bytes each.
        let (start, step, key_size) = if form == SF_STRONG_REF_VECTOR {
            (12, 4, 0)
        } else {
            let size = usize::from(*index.get(14).unwrap_or(&0));
            (15, 8 + size, size)
        };
        let _ = key_size;
        (0..count)
            .filter_map(|k| {
                let at = start + k * step;
                let key = u32::from_le_bytes(index.get(at..at + 4)?.try_into().ok()?);
                Some(join(path, &format!("{name}{{{key:x}}}")))
            })
            .collect()
    }
}

fn join(path: &str, name: &str) -> String {
    if path == "/" {
        format!("/{name}")
    } else {
        format!("{path}/{name}")
    }
}

/// A mob, as far as the clips need it.
struct Mob {
    path: String,
    name: String,
    source: bool,
    /// Its slots: id, edit rate (units a second), segment.
    slots: Vec<(u32, f64, Option<String>, String)>,
}

/// Reads the AAF at `path`.
pub fn read(path: &Path) -> Result<Session, String> {
    let cf = cfb::open(path).map_err(|e| format!("not an AAF file: {e}"))?;
    let mut aaf = Aaf { cf };
    let mut s = Session {
        bpm: BPM,
        sig: (4, 4),
        ..Session::default()
    };
    let root = aaf.props("/").ok_or("not an AAF file: it has no root object")?;
    let header = aaf.child("/", &root, ROOT_HEADER).ok_or("no AAF header")?;
    let hp = aaf.props(&header).ok_or("no AAF header")?;
    let content = aaf.child(&header, &hp, HEADER_CONTENT).ok_or("no content in the AAF")?;
    let cp = aaf.props(&content).ok_or("no content in the AAF")?;

    // Every mob, by its id.
    let mut mobs: HashMap<Vec<u8>, Mob> = HashMap::new();
    for path in aaf.children(&content, &cp, CONTENT_MOBS) {
        let Some(mp) = aaf.props(&path) else { continue };
        let Some(id) = mp.data(MOB_ID).map(<[u8]>::to_vec) else { continue };
        let mut slots = Vec::new();
        for slot in aaf.children(&path, &mp, MOB_SLOTS) {
            let Some(sp) = aaf.props(&slot) else { continue };
            let rate = sp.rational(SLOT_EDIT_RATE).unwrap_or(0.0);
            let segment = aaf.child(&slot, &sp, SLOT_SEGMENT);
            slots.push((
                sp.u32(SLOT_ID).unwrap_or(0),
                rate,
                segment,
                sp.string(SLOT_NAME).unwrap_or_default(),
            ));
        }
        mobs.insert(
            id,
            Mob {
                name: mp.string(MOB_NAME).unwrap_or_default(),
                source: mp.has(SOURCE_DESCRIPTOR),
                path,
                slots,
            },
        );
    }

    // Audio embedded in the AAF, by the source mob it belongs to.
    let mut essence: HashMap<Vec<u8>, String> = HashMap::new();
    for path in aaf.children(&content, &cp, CONTENT_ESSENCE) {
        if let Some(ep) = aaf.props(&path) {
            if let (Some(id), Some((SF_DATA_STREAM, v))) = (ep.data(ESSENCE_MOB_ID), ep.get(ESSENCE_DATA)) {
                essence.insert(id.to_vec(), join(&path, &utf16(v.get(1..).unwrap_or_default())));
            }
        }
    }

    // The composition: a mob that no clip of another mob points at, and that is not a file.
    let mut pointed: std::collections::HashSet<Vec<u8>> = std::collections::HashSet::new();
    let paths: Vec<(Vec<u8>, Vec<String>)> = mobs
        .iter()
        .map(|(id, m)| (id.clone(), m.slots.iter().filter_map(|s| s.2.clone()).collect()))
        .collect();
    for (_, segments) in &paths {
        for seg in segments {
            for clip in clips_in(&mut aaf, seg) {
                if let Some(id) = clip.source {
                    pointed.insert(id);
                }
            }
        }
    }
    let composition = mobs
        .iter()
        .filter(|(id, m)| !m.source && !pointed.contains(*id))
        .max_by_key(|(_, m)| m.slots.len())
        .map(|(id, _)| id.clone())
        .ok_or("no composition in the AAF")?;

    let base = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let media = std::env::temp_dir()
        .join("ovrtone-aaf")
        .join(path.file_stem().map_or_else(|| "aaf".into(), |n| n.to_string_lossy().into_owned()));
    let mut written: HashMap<Vec<u8>, Option<PathBuf>> = HashMap::new();
    let slots = mobs.get(&composition).map(|m| m.slots.clone()).unwrap_or_default();
    let mut skipped_other = 0;
    for (n, (_, rate, segment, name)) in slots.iter().enumerate() {
        let Some(segment) = segment else { continue };
        if *rate <= 0.0 {
            continue;
        }
        let mut track = SessionTrack {
            name: if name.is_empty() { format!("Audio {}", n + 1) } else { name.clone() },
            ..SessionTrack::default()
        };
        for clip in clips_in(&mut aaf, segment) {
            let Some(source) = clip.source.clone() else { continue };
            let Some(found) = resolve(&mut aaf, &mobs, &source, clip.source_slot, clip.start as f64 / rate, 0) else {
                skipped_other += 1;
                continue;
            };
            let file = match written.get(&found.mob) {
                Some(f) => f.clone(),
                None => {
                    let f = file_of(&mut aaf, &mobs, &found.mob, &essence, &base, &media, &mut s.log);
                    written.insert(found.mob.clone(), f.clone());
                    f
                }
            };
            let Some(file) = file else { continue };
            let secs = clip.position as f64 / rate;
            let len = clip.length as f64 / rate;
            track.clips.push(SessionClip {
                name: found.name.clone(),
                start: secs * BPM / 60.0,
                length: len * BPM / 60.0,
                file: Some(file),
                offset_secs: found.offset_secs,
                ..SessionClip::default()
            });
        }
        if !track.clips.is_empty() {
            s.tracks.push(track);
        }
    }
    if skipped_other > 0 {
        s.log.push(format!("{skipped_other} clips are not audio (video or other) or point at nothing, and were left out"));
    }
    s.log.push("AAF: fades, crossfades, clip effects and automation are not carried over; the session is placed at 120 BPM with positions kept in time".to_string());
    if s.tracks.is_empty() {
        return Err("no audio tracks found in the AAF".to_string());
    }
    Ok(s)
}

/// A clip of a slot: where it sits and for how long (edit units), and what it plays.
struct Clip {
    position: i64,
    length: i64,
    start: i64,
    source: Option<Vec<u8>>,
    source_slot: u32,
}

/// The source clips of a segment, in order, with their place on the slot.
fn clips_in<F: Read + Seek>(aaf: &mut Aaf<F>, segment: &str) -> Vec<Clip> {
    let mut out = Vec::new();
    let mut at = 0i64;
    walk(aaf, segment, &mut at, &mut out, 0);
    out
}

fn walk<F: Read + Seek>(aaf: &mut Aaf<F>, path: &str, at: &mut i64, out: &mut Vec<Clip>, depth: usize) {
    if depth > 8 {
        return;
    }
    let Some(p) = aaf.props(path) else { return };
    let length = p.i64(COMPONENT_LENGTH).unwrap_or(0);
    if p.has(SEQUENCE_COMPONENTS) {
        for c in aaf.children(path, &p, SEQUENCE_COMPONENTS) {
            walk(aaf, &c, at, out, depth + 1);
        }
        return;
    }
    if p.has(TRANSITION_OPERATION) {
        // A transition overlaps the clips either side: the timeline goes back by its length.
        *at -= length;
        return;
    }
    if p.has(OPERATION_INPUTS) {
        // An effect on clips: the first input is what plays.
        let start = *at;
        if let Some(first) = aaf.children(path, &p, OPERATION_INPUTS).into_iter().next() {
            walk(aaf, &first, at, out, depth + 1);
        }
        *at = start + length;
        return;
    }
    if p.has(CLIP_START) {
        out.push(Clip {
            position: *at,
            length,
            start: p.i64(CLIP_START).unwrap_or(0),
            source: p.data(SOURCE_ID).map(<[u8]>::to_vec).filter(|id| id.iter().any(|b| *b != 0)),
            source_slot: p.u32(SOURCE_SLOT).unwrap_or(0),
        });
    }
    // A filler, or anything else with a length: time passes.
    *at += length;
}

/// Where a clip ends up: the source mob holding its audio, how far into it, and a name.
struct Found {
    mob: Vec<u8>,
    offset_secs: f64,
    name: String,
}

/// Follows a clip from mob to mob (composition > master clip > file) to its source mob.
fn resolve<F: Read + Seek>(
    aaf: &mut Aaf<F>,
    mobs: &HashMap<Vec<u8>, Mob>,
    id: &[u8],
    slot: u32,
    offset_secs: f64,
    depth: usize,
) -> Option<Found> {
    let mob = mobs.get(id)?;
    if mob.source {
        return Some(Found {
            mob: id.to_vec(),
            offset_secs,
            name: mob.name.clone(),
        });
    }
    if depth > 6 {
        return None;
    }
    let (_, rate, segment, _) = mob.slots.iter().find(|s| s.0 == slot)?;
    let clip = clips_in(aaf, segment.as_ref()?).into_iter().find(|c| c.source.is_some())?;
    let inner = clip.start as f64 / rate.max(1e-9);
    let mut found = resolve(aaf, mobs, clip.source.as_ref()?, clip.source_slot, offset_secs + inner, depth + 1)?;
    // The master clip's name is the one an editor shows.
    if !mob.name.is_empty() {
        found.name = mob.name.clone();
    }
    Some(found)
}

/// The audio file of a source mob: linked (a locator), or embedded (written out as WAV).
fn file_of<F: Read + Seek>(
    aaf: &mut Aaf<F>,
    mobs: &HashMap<Vec<u8>, Mob>,
    id: &[u8],
    essence: &HashMap<Vec<u8>, String>,
    base: &Path,
    media: &Path,
    log: &mut Vec<String>,
) -> Option<PathBuf> {
    let mob = mobs.get(id)?;
    let mp = aaf.props(&mob.path)?;
    let desc_path = aaf.child(&mob.path, &mp, SOURCE_DESCRIPTOR)?;
    let dp = aaf.props(&desc_path)?;
    let is_sound = dp.has(SOUND_RATE) || dp.has(SOUND_CHANNELS);
    if !is_sound {
        return None;
    }
    // Embedded audio.
    if let Some(stream) = essence.get(id) {
        let bytes = aaf.stream(stream)?;
        let _ = std::fs::create_dir_all(media);
        let safe: String = mob
            .name
            .chars()
            .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' })
            .collect();
        let stem = if safe.trim().is_empty() { "clip".to_string() } else { safe };
        let n = id.iter().fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(u32::from(*b)));
        if bytes.starts_with(b"RIFF") || bytes.starts_with(b"RF64") {
            let out = media.join(format!("{stem}-{n:08x}.wav"));
            std::fs::write(&out, &bytes).ok()?;
            return Some(out);
        }
        if bytes.starts_with(b"FORM") {
            let out = media.join(format!("{stem}-{n:08x}.aif"));
            std::fs::write(&out, &bytes).ok()?;
            return Some(out);
        }
        // Raw PCM, as Pro Tools embeds it: a WAV header from the descriptor.
        let rate = dp.rational(SOUND_RATE).unwrap_or(48_000.0).round() as u32;
        let channels = dp.u32(SOUND_CHANNELS).unwrap_or(1).clamp(1, 32) as u16;
        let bits = dp.u32(SOUND_BITS).unwrap_or(24).clamp(8, 32) as u16;
        let out = media.join(format!("{stem}-{n:08x}.wav"));
        std::fs::write(&out, wav_of(&bytes, rate, channels, bits)).ok()?;
        return Some(out);
    }
    // Linked audio: the locator's address, else the same name beside the AAF.
    let mut tried = String::new();
    for loc in aaf.children(&desc_path, &dp, DESCRIPTOR_LOCATORS) {
        let Some(lp) = aaf.props(&loc) else { continue };
        let Some(url) = lp.string(LOCATOR_URL) else { continue };
        tried = url.clone();
        let file = file_from_url(&url);
        if file.exists() {
            return Some(file);
        }
        if let Some(name) = file.file_name() {
            for dir in [base.to_path_buf(), base.join("Audio Files"), base.join("Media Files")] {
                let near = dir.join(name);
                if near.exists() {
                    return Some(near);
                }
            }
        }
    }
    log.push(if tried.is_empty() {
        format!("{}: its audio is neither in the AAF nor linked", mob.name)
    } else {
        format!("{}: its audio file was not found ({tried})", mob.name)
    });
    None
}

/// A path from a locator's `file://` address (or a plain path).
fn file_from_url(url: &str) -> PathBuf {
    let rest = url
        .strip_prefix("file:///")
        .or_else(|| url.strip_prefix("file://localhost/"))
        .or_else(|| url.strip_prefix("file://"))
        .unwrap_or(url);
    // Percent-escapes, as `%20` for a space.
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    PathBuf::from(String::from_utf8_lossy(&out).into_owned())
}

/// A WAV file of raw little-endian PCM.
fn wav_of(pcm: &[u8], rate: u32, channels: u16, bits: u16) -> Vec<u8> {
    let block = channels * bits.div_ceil(8);
    let data_len = pcm.len() as u32;
    let mut out = Vec::with_capacity(pcm.len() + 44);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * u32::from(block)).to_le_bytes());
    out.extend_from_slice(&block.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locators_become_paths() {
        assert_eq!(
            file_from_url("file:///C:/Audio%20Files/Kick.wav"),
            PathBuf::from("C:/Audio Files/Kick.wav")
        );
        assert_eq!(file_from_url("D:/x.wav"), PathBuf::from("D:/x.wav"));
    }

    #[test]
    fn raw_pcm_gets_a_wav_header() {
        let wav = wav_of(&[0u8; 12], 48_000, 2, 24);
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(wav.len(), 44 + 12);
        assert_eq!(u16::from_le_bytes([wav[32], wav[33]]), 6, "a frame of two 24-bit samples");
    }

    #[test]
    fn strings_are_utf16_up_to_the_zero() {
        let mut b: Vec<u8> = "Kick".encode_utf16().flat_map(u16::to_le_bytes).collect();
        b.extend_from_slice(&[0, 0, b'x', 0]);
        assert_eq!(utf16(&b), "Kick");
    }
}
