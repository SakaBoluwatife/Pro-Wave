//! Title, artist, album and comment written into an exported file, in the way each format keeps
//! them: a LIST/INFO chunk in WAV, NAME/AUTH/ANNO chunks in AIFF, a Vorbis comment block in FLAC
//! and an ID3v2.3 tag in MP3. Run on the finished file, off the audio thread.
use std::path::Path;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tags {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub comment: String,
}

impl Tags {
    pub fn is_empty(&self) -> bool {
        [&self.title, &self.artist, &self.album, &self.comment]
            .iter()
            .all(|s| s.trim().is_empty())
    }
}

/// Writes `tags` into the file at `path`, by its extension. Files of other kinds are left alone.
pub fn write(path: &Path, tags: &Tags) -> Result<(), String> {
    if tags.is_empty() {
        return Ok(());
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let mut data = std::fs::read(path).map_err(|e| e.to_string())?;
    match ext.as_str() {
        "wav" => wav(&mut data, tags)?,
        "aif" | "aiff" => aiff(&mut data, tags)?,
        "flac" => flac(&mut data, tags)?,
        "mp3" => data = mp3(data, tags),
        _ => return Ok(()),
    }
    let part = path.with_extension("tagging");
    std::fs::write(&part, &data).map_err(|e| e.to_string())?;
    std::fs::rename(&part, path).map_err(|e| e.to_string())
}

fn fields(tags: &Tags) -> [(&'static str, &str); 4] {
    [
        ("title", tags.title.trim()),
        ("artist", tags.artist.trim()),
        ("album", tags.album.trim()),
        ("comment", tags.comment.trim()),
    ]
}

/// A LIST/INFO chunk at the end; the RIFF size grows to hold it.
fn wav(data: &mut Vec<u8>, tags: &Tags) -> Result<(), String> {
    if data.len() < 12 || &data[..4] != b"RIFF" {
        return Err("not a WAV file".into());
    }
    let mut info = b"INFO".to_vec();
    for (key, value) in fields(tags) {
        if value.is_empty() {
            continue;
        }
        let id: &[u8; 4] = match key {
            "title" => b"INAM",
            "artist" => b"IART",
            "album" => b"IPRD",
            _ => b"ICMT",
        };
        let mut text = value.as_bytes().to_vec();
        text.push(0);
        info.extend_from_slice(id);
        info.extend_from_slice(&(text.len() as u32).to_le_bytes());
        info.extend_from_slice(&text);
        if text.len() % 2 == 1 {
            info.push(0);
        }
    }
    if data.len() % 2 == 1 {
        data.push(0);
    }
    data.extend_from_slice(b"LIST");
    data.extend_from_slice(&(info.len() as u32).to_le_bytes());
    data.extend_from_slice(&info);
    let riff = (data.len() - 8) as u32;
    data[4..8].copy_from_slice(&riff.to_le_bytes());
    Ok(())
}

/// NAME, AUTH and ANNO chunks at the end; the FORM size grows to hold them.
fn aiff(data: &mut Vec<u8>, tags: &Tags) -> Result<(), String> {
    if data.len() < 12 || &data[..4] != b"FORM" {
        return Err("not an AIFF file".into());
    }
    let mut add = |id: &[u8; 4], text: &str| {
        if text.is_empty() {
            return;
        }
        data.extend_from_slice(id);
        data.extend_from_slice(&(text.len() as u32).to_be_bytes());
        data.extend_from_slice(text.as_bytes());
        if text.len() % 2 == 1 {
            data.push(0);
        }
    };
    add(b"NAME", tags.title.trim());
    add(b"AUTH", tags.artist.trim());
    let mut anno = String::new();
    if !tags.album.trim().is_empty() {
        anno = format!("Album: {}", tags.album.trim());
    }
    if !tags.comment.trim().is_empty() {
        if !anno.is_empty() {
            anno.push_str(". ");
        }
        anno.push_str(tags.comment.trim());
    }
    add(b"ANNO", &anno);
    let form = (data.len() - 8) as u32;
    data[4..8].copy_from_slice(&form.to_be_bytes());
    Ok(())
}

/// A Vorbis comment block after the stream information, the last-block flag moved to stay right.
fn flac(data: &mut Vec<u8>, tags: &Tags) -> Result<(), String> {
    if data.len() < 8 || &data[..4] != b"fLaC" {
        return Err("not a FLAC file".into());
    }
    // Walk the metadata blocks to the last one, dropping an earlier Vorbis comment.
    let mut at = 4;
    let mut last_start = None;
    loop {
        if at + 4 > data.len() {
            return Err("damaged FLAC header".into());
        }
        let header = data[at];
        let len = u32::from_be_bytes([0, data[at + 1], data[at + 2], data[at + 3]]) as usize;
        let is_last = header & 0x80 != 0;
        let kind = header & 0x7f;
        if kind == 4 {
            data.drain(at..at + 4 + len);
            if is_last {
                if let Some(prev) = last_start {
                    let b: &mut u8 = &mut data[prev];
                    *b |= 0x80;
                }
                break;
            }
            continue;
        }
        last_start = Some(at);
        at += 4 + len;
        if is_last {
            break;
        }
    }
    let Some(prev) = last_start else {
        return Err("damaged FLAC header".into());
    };
    data[prev] &= 0x7f;
    let vendor = b"Ovrtone";
    let mut block = Vec::new();
    block.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    block.extend_from_slice(vendor);
    let entries: Vec<String> = fields(tags)
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| format!("{}={v}", k.to_ascii_uppercase()))
        .collect();
    block.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for e in &entries {
        block.extend_from_slice(&(e.len() as u32).to_le_bytes());
        block.extend_from_slice(e.as_bytes());
    }
    let len = block.len() as u32;
    let mut header = vec![0x80 | 4];
    header.extend_from_slice(&len.to_be_bytes()[1..]);
    header.extend_from_slice(&block);
    data.splice(at..at, header);
    Ok(())
}

/// An ID3v2.3 tag in front of the MP3 frames.
fn mp3(data: Vec<u8>, tags: &Tags) -> Vec<u8> {
    let mut frames = Vec::new();
    let mut text_frame = |id: &[u8; 4], text: &str| {
        if text.is_empty() {
            return;
        }
        // Encoding 1: UTF-16 with a byte order mark, so any language is kept.
        let mut body = vec![1u8, 0xff, 0xfe];
        for u in text.encode_utf16() {
            body.extend_from_slice(&u.to_le_bytes());
        }
        frames.extend_from_slice(id);
        frames.extend_from_slice(&(body.len() as u32).to_be_bytes());
        frames.extend_from_slice(&[0, 0]);
        frames.extend_from_slice(&body);
    };
    text_frame(b"TIT2", tags.title.trim());
    text_frame(b"TPE1", tags.artist.trim());
    text_frame(b"TALB", tags.album.trim());
    let comment = tags.comment.trim();
    if !comment.is_empty() {
        let mut body = vec![1u8, b'e', b'n', b'g', 0xff, 0xfe, 0, 0, 0xff, 0xfe];
        for u in comment.encode_utf16() {
            body.extend_from_slice(&u.to_le_bytes());
        }
        frames.extend_from_slice(b"COMM");
        frames.extend_from_slice(&(body.len() as u32).to_be_bytes());
        frames.extend_from_slice(&[0, 0]);
        frames.extend_from_slice(&body);
    }
    let size = frames.len() as u32;
    // The tag size is written seven bits to a byte.
    let synchsafe = [
        ((size >> 21) & 0x7f) as u8,
        ((size >> 14) & 0x7f) as u8,
        ((size >> 7) & 0x7f) as u8,
        (size & 0x7f) as u8,
    ];
    let mut out = Vec::with_capacity(10 + frames.len() + data.len());
    out.extend_from_slice(b"ID3");
    out.extend_from_slice(&[3, 0, 0]);
    out.extend_from_slice(&synchsafe);
    out.extend_from_slice(&frames);
    out.extend_from_slice(&data);
    out
}
