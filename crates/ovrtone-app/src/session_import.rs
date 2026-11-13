//! Opening another DAW's session (File > Open): its tracks named and in place, their audio and MIDI
//! clips, the tempo, the time signature and the markers. The file is read directly, so the DAW
//! that made it is not needed. Anything that cannot be carried over (plugins, sends, automation,
//! warping) is named in the log the program shows afterwards, never dropped without a word.
//!
//! Read today:
//! - DAWproject (`.dawproject`): the open format of Bitwig, Studio One and Cubase.
//! - REAPER (`.rpp`): its text project.
//! - Ableton Live (`.als`): its compressed XML set.
//! - Ardour and Mixbus (`.ardour`): its XML session.
//! - Pro Tools: its File > Export > Session Info as Text (`.txt`), with the session's Audio Files.
//! - Logic Pro (`.logicx`, or any file inside one): partly, as its arrangement is undocumented.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use egui::Color32;

use crate::model::{Note, Project, Region, RegionParams, Track, TrackKind};

/// Whether `path` is a session of another DAW that this can open.
pub fn can_open(path: &Path) -> bool {
    if logic_folder(path).is_some() || is_pro_tools_text(path) {
        return true;
    }
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["dawproject", "rpp", "als", "ardour"].iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// A session as read, before it becomes a project. Times are in quarter-note beats.
#[derive(Default, Debug)]
pub struct Session {
    pub bpm: f64,
    pub sig: (u8, u8),
    /// Tempo changes after the start: (beat, bpm).
    pub tempo: Vec<(f64, f64)>,
    pub markers: Vec<(f64, String)>,
    pub tracks: Vec<SessionTrack>,
    /// What could not be carried over, or was found missing.
    pub log: Vec<String>,
}

#[derive(Default, Debug)]
pub struct SessionTrack {
    pub name: String,
    pub midi: bool,
    pub volume_db: f32,
    /// -1 (left) to 1 (right).
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub color: Option<Color32>,
    pub clips: Vec<SessionClip>,
}

#[derive(Default, Debug)]
pub struct SessionClip {
    pub name: String,
    pub start: f64,
    pub length: f64,
    pub muted: bool,
    /// Audio: the file, and how far into it the clip starts, in seconds.
    pub file: Option<PathBuf>,
    pub offset_secs: f64,
    pub gain_db: f32,
    /// MIDI: (start from the clip's start, length, pitch, velocity), in beats.
    pub notes: Vec<(f64, f64, u8, u8)>,
}

/// Reads the session at `path` and makes a project of it, with the log of what was left behind.
pub fn open(path: &Path, ctx: &egui::Context) -> Result<(Project, Vec<String>), String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let session = match ext.as_str() {
        _ if logic_folder(path).is_some() => logic(&logic_folder(path).unwrap_or_default())?,
        "txt" if is_pro_tools_text(path) => pro_tools_text(path)?,
        "ardour" => ardour(path)?,
        "dawproject" => dawproject(path)?,
        "rpp" => reaper(path)?,
        "als" => ableton(path)?,
        _ => return Err(format!("{} is not a session this can open", path.display())),
    };
    Ok(to_project(session, path, ctx))
}

/// The project made from a session.
fn to_project(mut s: Session, path: &Path, ctx: &egui::Context) -> (Project, Vec<String>) {
    let mut p = Project::empty();
    p.tracks.clear();
    p.name = path
        .file_stem()
        .map_or_else(|| "Imported".to_string(), |n| n.to_string_lossy().into_owned());
    if s.bpm.is_finite() && s.bpm > 0.0 {
        p.tempo = s.bpm.clamp(20.0, 999.0) as f32;
    }
    if s.sig.0 > 0 && s.sig.1 > 0 {
        p.sig = s.sig;
    }
    p.tempo_changes = s
        .tempo
        .iter()
        .filter(|(b, bpm)| *b > 0.0 && bpm.is_finite() && *bpm > 0.0)
        .map(|(beat, bpm)| crate::model::TempoPoint {
            beat: *beat,
            bpm: bpm.clamp(20.0, 999.0) as f32,
            ramp: false,
        })
        .collect();
    p.markers = s
        .markers
        .iter()
        .map(|(beat, name)| crate::model::Marker {
            beat: beat.max(0.0),
            name: name.clone(),
            color: Color32::from_rgb(0x8a, 0x8a, 0x8a),
        })
        .collect();
    let map = p.tempo_map();
    let mut missing = 0;
    for st in std::mem::take(&mut s.tracks) {
        let kind = if st.midi { TrackKind::Instrument } else { TrackKind::Audio };
        let mut t = Track::new(st.name.clone(), kind);
        t.volume_db = st.volume_db.clamp(-60.0, 6.0);
        t.pan = st.pan.clamp(-1.0, 1.0);
        t.mute = st.mute;
        t.solo = st.solo;
        if let Some(c) = st.color {
            t.color = c;
        }
        for clip in st.clips {
            if clip.length <= 0.0 {
                continue;
            }
            let start_bar = p.beats_to_bar(clip.start.max(0.0)) + 1.0;
            let len_bars = p.beats_to_bar(clip.start.max(0.0) + clip.length) - (start_bar - 1.0);
            let mut params = RegionParams {
                mute: clip.muted,
                gain_db: clip.gain_db.clamp(-96.0, 24.0),
                ..RegionParams::default()
            };
            let mut region = Region {
                name: clip.name.clone(),
                start_bar: start_bar as f32,
                len_bars: (len_bars as f32).max(1.0 / 64.0),
                notes: Vec::new(),
                controls: Vec::new(),
                folder: None,
                audio: None,
                params: RegionParams::default(),
                auto: Vec::new(),
                generated: None,
                gain_line: Vec::new(),
            };
            if st.midi {
                region.notes = clip
                    .notes
                    .iter()
                    .filter(|n| n.1 > 0.0 && n.0 < clip.length)
                    .map(|&(start, len, pitch, vel)| Note {
                        start: start.max(0.0) as f32,
                        len: len as f32,
                        pitch: pitch.min(127),
                        vel: vel.clamp(1, 127),
                        muted: false,
                        color: 0,
                        slide: false,
                    })
                    .collect();
            } else {
                let Some(file) = clip.file.as_ref() else {
                    continue;
                };
                match ovrtone_engine::AudioFile::open(file) {
                    Ok(audio) => {
                        let seconds = map.seconds_at(clip.start + clip.length) - map.seconds_at(clip.start);
                        params.source_start = clip.offset_secs.max(0.0) as f32;
                        params.source_len = seconds.max(0.0) as f32;
                        region.audio = Some(crate::import::audio_region(audio, ctx));
                    }
                    Err(e) => {
                        missing += 1;
                        s.log.push(format!("Missing audio on \"{}\": {} ({e})", st.name, file.display()));
                        continue;
                    }
                }
            }
            region.params = params;
            t.regions.push(region);
        }
        p.tracks.push(t);
    }
    if missing > 0 {
        s.log.insert(0, format!("{missing} audio files could not be found: their clips were left out."));
    }
    p.selected_track = 0;
    p.song_rev += 1;
    (p, s.log)
}

/// Where a file a session names is: as it is, beside the session, or in a folder beside it with
/// the same name.
fn find_file(base: &Path, named: &str) -> PathBuf {
    let raw = PathBuf::from(named.replace('\\', std::path::MAIN_SEPARATOR_STR));
    if raw.is_absolute() && raw.is_file() {
        return raw;
    }
    let beside = base.join(&raw);
    if beside.is_file() {
        return beside;
    }
    if let Some(name) = raw.file_name() {
        for folder in ["", "Audio", "Audio Files", "Media", "Samples", "Recorded", "audio"] {
            let there = base.join(folder).join(name);
            if there.is_file() {
                return there;
            }
        }
    }
    if raw.is_absolute() {
        raw
    } else {
        beside
    }
}

fn number<T: std::str::FromStr>(s: Option<&str>) -> Option<T> {
    s.and_then(|v| v.trim().parse().ok())
}

fn db_of(gain: f64) -> f32 {
    if gain <= 1e-6 {
        -60.0
    } else {
        (20.0 * gain.log10()) as f32
    }
}

fn hex_color(text: &str) -> Option<Color32> {
    let t = text.trim().trim_start_matches('#');
    if t.len() < 6 {
        return None;
    }
    let v = u32::from_str_radix(&t[..6], 16).ok()?;
    Some(Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

// ---------------------------------------------------------------- DAWproject

fn dawproject(path: &Path) -> Result<Session, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("not a DAWproject file: {e}"))?;
    let mut xml = String::new();
    zip.by_name("project.xml")
        .map_err(|_| "no project.xml in the file".to_string())?
        .read_to_string(&mut xml)
        .map_err(|e| e.to_string())?;
    // Its audio files go to a folder beside it, where the project can find them.
    let media = path.with_extension("").with_file_name(format!(
        "{} Media",
        path.file_stem().map_or_else(String::new, |n| n.to_string_lossy().into_owned())
    ));
    for i in 0..zip.len() {
        let Ok(mut entry) = zip.by_index(i) else {
            continue;
        };
        let name = entry.name().map(|n| n.to_string()).unwrap_or_default();
        if entry.is_dir() || name.ends_with(".xml") {
            continue;
        }
        let Some(rel) = entry.enclosed_name() else {
            continue;
        };
        let out = media.join(rel);
        if out.is_file() {
            continue;
        }
        if let Some(dir) = out.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut bytes = Vec::new();
        if entry.read_to_end(&mut bytes).is_ok() {
            let _ = std::fs::write(&out, bytes);
        }
    }
    let doc = roxmltree::Document::parse(&xml).map_err(|e| format!("project.xml: {e}"))?;
    let root = doc.root_element();
    let mut s = Session {
        bpm: 120.0,
        sig: (4, 4),
        ..Session::default()
    };
    if let Some(t) = root.descendants().find(|n| n.has_tag_name("Transport")) {
        if let Some(tempo) = t.children().find(|n| n.has_tag_name("Tempo")) {
            s.bpm = number(tempo.attribute("value")).unwrap_or(120.0);
        }
        if let Some(sig) = t.children().find(|n| n.has_tag_name("TimeSignature")) {
            s.sig = (
                number(sig.attribute("numerator")).unwrap_or(4),
                number(sig.attribute("denominator")).unwrap_or(4),
            );
        }
    }
    // Tracks by id: their channel's level, pan and mute.
    let mut ids: Vec<(String, usize)> = Vec::new();
    for t in root.descendants().filter(|n| n.has_tag_name("Track")) {
        let content = t.attribute("contentType").unwrap_or("");
        if !content.split_whitespace().any(|c| c == "audio" || c == "notes") {
            continue;
        }
        let mut st = SessionTrack {
            name: t.attribute("name").unwrap_or("Track").to_string(),
            midi: content.contains("notes"),
            color: t.attribute("color").and_then(hex_color),
            ..SessionTrack::default()
        };
        if let Some(ch) = t.children().find(|n| n.has_tag_name("Channel")) {
            for c in ch.children() {
                let value = c.attribute("value");
                match c.tag_name().name() {
                    "Volume" => {
                        let v: f64 = number(value).unwrap_or(1.0);
                        st.volume_db = if c.attribute("unit") == Some("decibel") { v as f32 } else { db_of(v) };
                    }
                    "Pan" => {
                        let v: f32 = number(value).unwrap_or(0.5);
                        st.pan = if c.attribute("unit") == Some("normalized") { v * 2.0 - 1.0 } else { v };
                    }
                    "Mute" => st.mute = value == Some("true"),
                    "Solo" => st.solo = value == Some("true"),
                    "Devices" if c.children().any(|d| d.is_element()) => {
                        s.log.push(format!("\"{}\": its plugins were not carried over", st.name));
                    }
                    "Sends" if c.children().any(|d| d.is_element()) => {
                        s.log.push(format!("\"{}\": its sends were not carried over", st.name));
                    }
                    _ => {}
                }
            }
        }
        if let Some(id) = t.attribute("id") {
            ids.push((id.to_string(), s.tracks.len()));
        }
        s.tracks.push(st);
    }
    let base = media.clone();
    let seconds_to_beats = |secs: f64, bpm: f64| secs * bpm / 60.0;
    for lanes in root.descendants().filter(|n| n.has_tag_name("Lanes")) {
        let Some(k) = lanes
            .attribute("track")
            .and_then(|id| ids.iter().find(|(x, _)| x == id))
            .map(|(_, k)| *k)
        else {
            continue;
        };
        let in_seconds = lanes
            .ancestors()
            .find_map(|a| a.attribute("timeUnit"))
            .is_some_and(|u| u == "seconds");
        let bpm = s.bpm;
        let to_beats = |v: f64| if in_seconds { seconds_to_beats(v, bpm) } else { v };
        for clip in lanes.descendants().filter(|n| n.has_tag_name("Clip")) {
            let mut c = SessionClip {
                name: clip.attribute("name").unwrap_or("").to_string(),
                start: to_beats(number(clip.attribute("time")).unwrap_or(0.0)),
                length: to_beats(number(clip.attribute("duration")).unwrap_or(0.0)),
                ..SessionClip::default()
            };
            if let Some(file) = clip.descendants().find(|n| n.has_tag_name("File")) {
                if let Some(rel) = file.attribute("path") {
                    c.file = Some(find_file(&base, rel));
                }
                let play_start: f64 = number(clip.attribute("playStart")).unwrap_or(0.0);
                c.offset_secs = if in_seconds { play_start } else { play_start * 60.0 / bpm };
                if clip.descendants().any(|n| n.has_tag_name("Warp")) {
                    s.log.push(format!("Clip \"{}\": its warp markers were not carried over", c.name));
                }
            }
            for n in clip.descendants().filter(|n| n.has_tag_name("Note")) {
                let vel: f64 = number(n.attribute("vel")).unwrap_or(0.8);
                c.notes.push((
                    number(n.attribute("time")).unwrap_or(0.0),
                    number(n.attribute("duration")).unwrap_or(0.25),
                    number(n.attribute("key")).unwrap_or(60),
                    (vel.clamp(0.0, 1.0) * 127.0).round() as u8,
                ));
            }
            if let Some(t) = s.tracks.get_mut(k) {
                t.clips.push(c);
            }
        }
    }
    for m in root.descendants().filter(|n| n.has_tag_name("Marker")) {
        s.markers.push((
            number(m.attribute("time")).unwrap_or(0.0),
            m.attribute("name").unwrap_or("Marker").to_string(),
        ));
    }
    if root.descendants().any(|n| n.has_tag_name("Points")) {
        s.log.push("Automation was not carried over".to_string());
    }
    Ok(s)
}

// ---------------------------------------------------------------- REAPER

fn reaper(path: &Path) -> Result<Session, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let base = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut s = Session {
        bpm: 120.0,
        sig: (4, 4),
        ..Session::default()
    };
    // Words of a line, with "quoted parts" kept together.
    fn words(line: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = String::new();
        let mut quote: Option<char> = None;
        for ch in line.trim().chars() {
            match quote {
                Some(q) if ch == q => quote = None,
                Some(_) => cur.push(ch),
                None if ch == '"' || ch == '\'' || ch == '`' => quote = Some(ch),
                None if ch.is_whitespace() => {
                    if !cur.is_empty() {
                        out.push(std::mem::take(&mut cur));
                    }
                }
                None => cur.push(ch),
            }
        }
        if !cur.is_empty() {
            out.push(cur);
        }
        out
    }
    let mut stack: Vec<String> = Vec::new();
    let mut track: Option<SessionTrack> = None;
    let mut clip: Option<SessionClip> = None;
    let mut clip_secs = (0.0f64, 0.0f64);
    let mut midi_ppq = 960.0f64;
    let mut midi_at = 0u64;
    let mut open_notes: Vec<(u8, u64, u8)> = Vec::new();
    let mut plugins_on = false;
    let mut markers_secs: Vec<(f64, String)> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix('<') {
            let w = words(rest);
            let tag = w.first().cloned().unwrap_or_default();
            match tag.as_str() {
                "TRACK" => track = Some(SessionTrack::default()),
                "ITEM" => {
                    clip = Some(SessionClip::default());
                    clip_secs = (0.0, 0.0);
                }
                "SOURCE" => {
                    if w.get(1).is_some_and(|k| k == "MIDI") {
                        if let Some(c) = clip.as_mut() {
                            c.file = None;
                        }
                        midi_at = 0;
                        open_notes.clear();
                        if let Some(t) = track.as_mut() {
                            t.midi = true;
                        }
                    }
                }
                "FXCHAIN" => plugins_on = true,
                _ => {}
            }
            stack.push(tag);
            continue;
        }
        if line == ">" {
            match stack.pop().as_deref() {
                Some("ITEM") => {
                    if let (Some(mut c), Some(t)) = (clip.take(), track.as_mut()) {
                        c.start = clip_secs.0;
                        c.length = clip_secs.1;
                        t.clips.push(c);
                    }
                }
                Some("TRACK") => {
                    if let Some(t) = track.take() {
                        if plugins_on {
                            s.log.push(format!("\"{}\": its plugins were not carried over", t.name));
                        }
                        s.tracks.push(t);
                    }
                    plugins_on = false;
                }
                _ => {}
            }
            continue;
        }
        let w = words(line);
        let Some(key) = w.first().map(String::as_str) else {
            continue;
        };
        let arg = |i: usize| w.get(i).map(String::as_str);
        let inside = stack.last().map(String::as_str);
        match (inside, key) {
            (Some("REAPER_PROJECT"), "TEMPO") => {
                s.bpm = number(arg(1)).unwrap_or(120.0);
                s.sig = (number(arg(2)).unwrap_or(4), number(arg(3)).unwrap_or(4));
            }
            (Some("REAPER_PROJECT"), "MARKER") => {
                markers_secs.push((number(arg(2)).unwrap_or(0.0), arg(3).unwrap_or("Marker").to_string()));
            }
            (Some("TEMPOENVEX"), "PT") => {
                if let (Some(sec), Some(bpm)) = (number::<f64>(arg(1)), number::<f64>(arg(2))) {
                    s.tempo.push((sec, bpm));
                }
            }
            (Some("TRACK"), "NAME") => {
                if let Some(t) = track.as_mut() {
                    t.name = arg(1).unwrap_or("Track").to_string();
                }
            }
            (Some("TRACK"), "VOLPAN") => {
                if let Some(t) = track.as_mut() {
                    t.volume_db = db_of(number(arg(1)).unwrap_or(1.0));
                    t.pan = number(arg(2)).unwrap_or(0.0);
                }
            }
            (Some("TRACK"), "MUTESOLO") => {
                if let Some(t) = track.as_mut() {
                    t.mute = arg(1) == Some("1");
                    t.solo = arg(2).is_some_and(|v| v != "0");
                }
            }
            (Some("TRACK"), "AUXRECV") => {
                if let Some(t) = track.as_ref() {
                    s.log.push(format!("\"{}\": a send to it was not carried over", t.name));
                }
            }
            (Some("ITEM"), "POSITION") => clip_secs.0 = number(arg(1)).unwrap_or(0.0),
            (Some("ITEM"), "LENGTH") => clip_secs.1 = number(arg(1)).unwrap_or(0.0),
            (Some("ITEM"), "SOFFS") => {
                if let Some(c) = clip.as_mut() {
                    c.offset_secs = number(arg(1)).unwrap_or(0.0);
                }
            }
            (Some("ITEM"), "NAME") => {
                if let Some(c) = clip.as_mut() {
                    c.name = arg(1).unwrap_or("").to_string();
                }
            }
            (Some("ITEM"), "MUTE") => {
                if let Some(c) = clip.as_mut() {
                    c.muted = arg(1) == Some("1");
                }
            }
            (Some("ITEM"), "VOLPAN") => {
                if let Some(c) = clip.as_mut() {
                    c.gain_db = db_of(number(arg(3)).unwrap_or(1.0));
                }
            }
            (Some("SOURCE"), "FILE") => {
                if let Some(c) = clip.as_mut() {
                    c.file = arg(1).map(|f| find_file(&base, f));
                }
            }
            (Some("SOURCE"), "HASDATA") => midi_ppq = number(arg(2)).unwrap_or(960.0),
            (Some("SOURCE"), "E" | "e") => {
                // A MIDI event: ticks since the last, then the status and data bytes in hex.
                midi_at += number::<u64>(arg(1)).unwrap_or(0);
                let byte = |i: usize| arg(i).and_then(|v| u8::from_str_radix(v, 16).ok()).unwrap_or(0);
                let (status, d1, d2) = (byte(2), byte(3), byte(4));
                let beat = midi_at as f64 / midi_ppq.max(1.0);
                match status & 0xF0 {
                    0x90 if d2 > 0 => open_notes.push((d1, midi_at, d2)),
                    0x80 | 0x90 => {
                        if let Some(k) = open_notes.iter().position(|n| n.0 == d1) {
                            let (pitch, from, vel) = open_notes.remove(k);
                            let start = from as f64 / midi_ppq.max(1.0);
                            if let Some(c) = clip.as_mut() {
                                c.notes.push((start, beat - start, pitch, vel));
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    // Seconds to beats at the project tempo (the tempo map's changes are kept, but positions use the
    // first tempo, and the log says so when it changes).
    let bpm = s.bpm.max(1.0);
    let beats = |secs: f64| secs * bpm / 60.0;
    for t in &mut s.tracks {
        for c in &mut t.clips {
            c.start = beats(c.start);
            c.length = beats(c.length);
        }
    }
    s.tempo = s.tempo.iter().map(|(sec, b)| (beats(*sec), *b)).collect();
    if s.tempo.len() > 1 {
        s.log.push("The tempo changes; clips are placed by the first tempo, so later ones may need moving".to_string());
    }
    s.markers = markers_secs.into_iter().map(|(sec, n)| (beats(sec), n)).collect();
    s.log.push("Automation envelopes were not carried over".to_string());
    Ok(s)
}

// ---------------------------------------------------------------- Ableton Live

fn ableton(path: &Path) -> Result<Session, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut xml = String::new();
    flate2::read::GzDecoder::new(file)
        .read_to_string(&mut xml)
        .map_err(|e| format!("not a Live set: {e}"))?;
    let doc = roxmltree::Document::parse(&xml).map_err(|e| format!("Live set: {e}"))?;
    let base = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let root = doc.root_element();
    let value = |n: roxmltree::Node, tag: &str| -> Option<String> {
        n.children()
            .find(|c| c.has_tag_name(tag))
            .and_then(|c| c.attribute("Value"))
            .map(str::to_string)
    };
    let manual = |n: roxmltree::Node, tag: &str| -> Option<f64> {
        n.descendants()
            .find(|c| c.has_tag_name(tag))
            .and_then(|c| c.children().find(|x| x.has_tag_name("Manual")))
            .and_then(|m| number(m.attribute("Value")))
    };
    let mut s = Session {
        bpm: 120.0,
        sig: (4, 4),
        ..Session::default()
    };
    if let Some(master) = root.descendants().find(|n| n.has_tag_name("MasterTrack") || n.has_tag_name("MainTrack")) {
        if let Some(bpm) = manual(master, "Tempo") {
            s.bpm = bpm;
        }
    }
    let Some(tracks) = root.descendants().find(|n| n.has_tag_name("Tracks")) else {
        return Err("no tracks in the set".to_string());
    };
    for t in tracks.children().filter(|n| n.is_element()) {
        let midi = match t.tag_name().name() {
            "AudioTrack" => false,
            "MidiTrack" => true,
            "ReturnTrack" | "GroupTrack" => {
                s.log.push(format!("A {} was not carried over", t.tag_name().name()));
                continue;
            }
            _ => continue,
        };
        let name = t
            .children()
            .find(|c| c.has_tag_name("Name"))
            .and_then(|n| value(n, "EffectiveName").or_else(|| value(n, "UserName")))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "Track".to_string());
        let mut st = SessionTrack {
            name: name.clone(),
            midi,
            ..SessionTrack::default()
        };
        if let Some(mixer) = t.descendants().find(|n| n.has_tag_name("Mixer")) {
            st.volume_db = db_of(manual(mixer, "Volume").unwrap_or(1.0));
            st.pan = manual(mixer, "Pan").unwrap_or(0.0) as f32;
            st.mute = mixer
                .descendants()
                .find(|n| n.has_tag_name("Speaker"))
                .and_then(|n| n.children().find(|c| c.has_tag_name("Manual")))
                .and_then(|m| m.attribute("Value"))
                == Some("false");
        }
        let devices = t
            .descendants()
            .filter(|n| n.has_tag_name("PluginDevice") || n.tag_name().name().ends_with("Device"))
            .count();
        if devices > 0 {
            s.log.push(format!("\"{name}\": {devices} devices and plugins were not carried over"));
        }
        let clip_tag = if midi { "MidiClip" } else { "AudioClip" };
        // Only the arrangement's clips: the session view's slots have no place in time.
        for seq in t.descendants().filter(|n| n.has_tag_name("ArrangerAutomation")) {
            for clip in seq.descendants().filter(|n| n.has_tag_name(clip_tag)) {
                let start: f64 = number(clip.attribute("Time")).unwrap_or(0.0);
                let from: f64 = value(clip, "CurrentStart").and_then(|v| v.parse().ok()).unwrap_or(start);
                let to: f64 = value(clip, "CurrentEnd").and_then(|v| v.parse().ok()).unwrap_or(from);
                let mut c = SessionClip {
                    name: value(clip, "Name").unwrap_or_default(),
                    start: from,
                    length: (to - from).max(0.0),
                    muted: value(clip, "Disabled").as_deref() == Some("true"),
                    ..SessionClip::default()
                };
                if !midi {
                    let file_ref = clip.descendants().find(|n| n.has_tag_name("FileRef"));
                    let named = file_ref.and_then(|f| {
                        value(f, "Path").filter(|p| !p.is_empty()).or_else(|| {
                            let rel = f.descendants().find(|n| n.has_tag_name("RelativePath"))?;
                            let parts: Vec<String> = rel
                                .children()
                                .filter_map(|e| e.attribute("Dir").map(str::to_string))
                                .collect();
                            let name = value(f, "Name")?;
                            Some(parts.into_iter().chain(std::iter::once(name)).collect::<Vec<_>>().join("/"))
                        })
                    });
                    c.file = named.map(|n| find_file(&base, &n));
                    if value(clip, "IsWarped").as_deref() == Some("true") {
                        s.log.push(format!(
                            "Clip \"{}\": warped in Live; it plays at its own speed here (set its BPM to follow the tempo)",
                            c.name
                        ));
                    }
                } else {
                    for key in clip.descendants().filter(|n| n.has_tag_name("KeyTrack")) {
                        let pitch: u8 = key
                            .children()
                            .find(|n| n.has_tag_name("MidiKey"))
                            .and_then(|n| number(n.attribute("Value")))
                            .unwrap_or(60);
                        for ev in key.descendants().filter(|n| n.has_tag_name("MidiNoteEvent")) {
                            let vel: f64 = number(ev.attribute("Velocity")).unwrap_or(100.0);
                            let time: f64 = number(ev.attribute("Time")).unwrap_or(0.0);
                            c.notes.push((
                                time - (from - start).max(0.0),
                                number(ev.attribute("Duration")).unwrap_or(0.25),
                                pitch,
                                vel.clamp(1.0, 127.0) as u8,
                            ));
                        }
                    }
                }
                st.clips.push(c);
            }
        }
        s.tracks.push(st);
    }
    for loc in root.descendants().filter(|n| n.has_tag_name("Locator")) {
        let time: f64 = value(loc, "Time").and_then(|v| v.parse().ok()).unwrap_or(0.0);
        s.markers.push((time, value(loc, "Name").unwrap_or_else(|| "Marker".to_string())));
    }
    s.log.push("Automation was not carried over".to_string());
    Ok(s)
}

// ---------------------------------------------------------------- Ardour (and Mixbus)

/// Ardour times: samples, or (Ardour 7 and up) `a` + superclock ticks or `b` + beat ticks.
fn ardour_seconds(text: &str, rate: f64, bpm: f64) -> f64 {
    const SUPERCLOCK: f64 = 282_240_000.0;
    const BEAT_TICKS: f64 = 1920.0;
    if let Some(v) = text.strip_prefix('a') {
        v.parse::<f64>().unwrap_or(0.0) / SUPERCLOCK
    } else if let Some(v) = text.strip_prefix('b') {
        v.parse::<f64>().unwrap_or(0.0) / BEAT_TICKS * 60.0 / bpm.max(1.0)
    } else {
        text.parse::<f64>().unwrap_or(0.0) / rate.max(1.0)
    }
}

fn ardour(path: &Path) -> Result<Session, String> {
    let xml = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let doc = roxmltree::Document::parse(&xml).map_err(|e| format!("Ardour session: {e}"))?;
    let root = doc.root_element();
    let base = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let rate: f64 = number(root.attribute("sample-rate")).unwrap_or(48_000.0);
    let mut s = Session {
        bpm: 120.0,
        sig: (4, 4),
        ..Session::default()
    };
    if let Some(tempo) = root.descendants().find(|n| n.has_tag_name("Tempo")) {
        let bpm = number::<f64>(tempo.attribute("npm"))
            .or_else(|| number(tempo.attribute("beats-per-minute")))
            .unwrap_or(120.0);
        // Ardour 7 counts in whole notes per minute of its note type; a quarter note is usual.
        let note: f64 = number(tempo.attribute("note-type")).unwrap_or(4.0);
        s.bpm = bpm * 4.0 / note.max(1.0);
    }
    if let Some(meter) = root.descendants().find(|n| n.has_tag_name("Meter")) {
        s.sig = (
            number::<f64>(meter.attribute("divisions-per-bar")).map_or(4, |v| v as u8),
            number::<f64>(meter.attribute("note-value")).map_or(4, |v| v as u8),
        );
    }
    let bpm = s.bpm;
    let beats = |secs: f64| secs * bpm / 60.0;
    // Audio files by source id; they live in interchange/<name>/audiofiles.
    let mut folders: Vec<PathBuf> = vec![base.clone()];
    if let Ok(dirs) = std::fs::read_dir(base.join("interchange")) {
        for d in dirs.flatten() {
            folders.push(d.path().join("audiofiles"));
        }
    }
    let file_of = |name: &str| -> PathBuf {
        for f in &folders {
            let there = f.join(name);
            if there.is_file() {
                return there;
            }
        }
        find_file(&base, name)
    };
    let mut sources: Vec<(String, PathBuf)> = Vec::new();
    for src in root.descendants().filter(|n| n.has_tag_name("Source")) {
        if src.attribute("type") == Some("audio") {
            if let (Some(id), Some(name)) = (src.attribute("id"), src.attribute("name")) {
                sources.push((id.to_string(), file_of(name)));
            }
        }
    }
    let playlists: Vec<roxmltree::Node> = root
        .descendants()
        .filter(|n| n.has_tag_name("Playlist") && n.parent().is_some_and(|p| p.has_tag_name("Playlists")))
        .collect();
    for route in root.descendants().filter(|n| n.has_tag_name("Route")) {
        let name = route.attribute("name").unwrap_or("Track").to_string();
        let kind = route.attribute("default-type").unwrap_or("audio");
        // The master and monitor buses have flags; a bus has no playlist.
        if route.attribute("flags").is_some_and(|f| f.contains("Master") || f.contains("Monitor")) {
            continue;
        }
        let id = route.attribute("id").unwrap_or("");
        let playlist = playlists.iter().find(|pl| {
            pl.attribute("orig-track-id") == Some(id)
                || pl.attribute("name").is_some_and(|n| n.starts_with(&format!("{name}.")))
        });
        let mut st = SessionTrack {
            name: name.clone(),
            midi: kind == "midi",
            ..SessionTrack::default()
        };
        if let Some(gain) = route
            .descendants()
            .find(|n| n.has_tag_name("Controllable") && n.attribute("name") == Some("gaincontrol"))
        {
            st.volume_db = db_of(number(gain.attribute("value")).unwrap_or(1.0));
        }
        if route.descendants().any(|n| n.has_tag_name("Processor") && n.attribute("type").is_some_and(|t| t.contains("plugin") || t == "lv2" || t == "ladspa" || t == "vst" || t == "luaproc")) {
            s.log.push(format!("\"{name}\": its plugins were not carried over"));
        }
        let Some(playlist) = playlist else {
            if kind == "audio" {
                s.log.push(format!("\"{name}\": a bus, not carried over"));
            }
            continue;
        };
        if st.midi {
            s.log.push(format!("\"{name}\": a MIDI track; its notes were not carried over (export them as MIDI from Ardour)"));
        }
        for region in playlist.children().filter(|n| n.has_tag_name("Region")) {
            let at = ardour_seconds(region.attribute("position").unwrap_or("0"), rate, bpm);
            let len = ardour_seconds(region.attribute("length").unwrap_or("0"), rate, bpm);
            let offset = ardour_seconds(region.attribute("start").unwrap_or("0"), rate, bpm);
            let source = region.attribute("source-0").unwrap_or("");
            let gain: f64 = number(region.attribute("scale-amplitude")).unwrap_or(1.0);
            st.clips.push(SessionClip {
                name: region.attribute("name").unwrap_or("").to_string(),
                start: beats(at),
                length: beats(len),
                muted: region.attribute("muted") == Some("1"),
                file: sources.iter().find(|(i, _)| i == source).map(|(_, f)| f.clone()),
                offset_secs: offset,
                gain_db: db_of(gain),
                notes: Vec::new(),
            });
        }
        s.tracks.push(st);
    }
    for loc in root.descendants().filter(|n| n.has_tag_name("Location")) {
        if loc.attribute("flags").is_some_and(|f| f.contains("IsMark")) {
            let at = ardour_seconds(loc.attribute("start").unwrap_or("0"), rate, bpm);
            s.markers.push((beats(at), loc.attribute("name").unwrap_or("Marker").to_string()));
        }
    }
    s.log.push("Automation was not carried over".to_string());
    Ok(s)
}

// ---------------------------------------------------------------- Logic Pro (partial)

/// The `.logicx` folder `path` is, or is inside.
pub fn logic_folder(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|a| a.extension().is_some_and(|e| e.eq_ignore_ascii_case("logicx")))
        .map(Path::to_path_buf)
}

/// What a Logic project gives up without its undocumented arrangement data: its audio files, each
/// on a track of its own from the start, and its tempo and time signature when its MetaData is a
/// plain (XML) property list.
fn logic(folder: &Path) -> Result<Session, String> {
    let mut s = Session {
        bpm: 120.0,
        sig: (4, 4),
        ..Session::default()
    };
    let meta = std::fs::read_dir(folder.join("Alternatives"))
        .ok()
        .and_then(|it| {
            it.flatten()
                .map(|e| e.path().join("MetaData.plist"))
                .find(|p| p.is_file())
        });
    match meta.and_then(|m| std::fs::read(m).ok()) {
        Some(bytes) if bytes.starts_with(b"<?xml") => {
            let text = String::from_utf8_lossy(&bytes).into_owned();
            if let Ok(doc) = roxmltree::Document::parse(&text) {
                let value_of = |key: &str| -> Option<String> {
                    let k = doc.descendants().find(|n| n.has_tag_name("key") && n.text() == Some(key))?;
                    k.next_siblings().find(|n| n.is_element()).and_then(|v| v.text().map(str::to_string))
                };
                if let Some(bpm) = value_of("BeatsPerMinute").or_else(|| value_of("tempo")).and_then(|v| v.parse().ok()) {
                    s.bpm = bpm;
                }
                let num = value_of("SongSignatureNumerator").and_then(|v| v.parse().ok());
                let den = value_of("SongSignatureDenominator").and_then(|v| v.parse().ok());
                if let (Some(n), Some(d)) = (num, den) {
                    s.sig = (n, d);
                }
            }
        }
        Some(_) => s.log.push("The tempo is in a binary property list, which is not read yet: set it by hand".to_string()),
        None => s.log.push("No MetaData.plist: the tempo was not found".to_string()),
    }
    let media = folder.join("Media").join("Audio Files");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&media)
        .map(|it| {
            it.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| ["wav", "aif", "aiff", "caf", "mp3", "m4a", "flac"].iter().any(|x| e.eq_ignore_ascii_case(x)))
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    if files.is_empty() {
        return Err("no audio files in the project's Media folder".to_string());
    }
    let bpm = s.bpm.max(1.0);
    for f in files {
        let seconds = ovrtone_engine::AudioFile::open(&f).map_or(0.0, |a| a.duration_secs());
        let name = f.file_stem().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        s.tracks.push(SessionTrack {
            name: name.clone(),
            clips: vec![SessionClip {
                name,
                start: 0.0,
                length: seconds * bpm / 60.0,
                file: Some(f),
                ..SessionClip::default()
            }],
            ..SessionTrack::default()
        });
    }
    s.log.insert(
        0,
        "Logic keeps where its regions are in an undocumented file, so each audio file is on a track of its own \
         from the start: drag them into place. For the whole arrangement, export from Logic as AAF or \
         DAWproject, or the tracks as stems."
            .to_string(),
    );
    s.log.push("MIDI regions, plugins and automation were not carried over".to_string());
    Ok(s)
}

// ---------------------------------------------------------------- Pro Tools (Session Info as Text)

/// Whether `path` is a Pro Tools "Session Info as Text" export.
pub fn is_pro_tools_text(path: &Path) -> bool {
    let txt = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("txt"));
    txt && std::fs::read(path)
        .map(|b| String::from_utf8_lossy(&b[..b.len().min(64 * 1024)]).contains("T R A C K  L I S T I N G"))
        .unwrap_or(false)
}

/// A time in the text: samples, minutes:seconds, timecode or bars|beats. Seconds, or beats with
/// the flag set.
fn pro_tools_time(text: &str, rate: f64, fps: f64) -> Option<(f64, bool)> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    if let Ok(samples) = t.parse::<f64>() {
        return Some((samples / rate.max(1.0), false));
    }
    if t.contains('|') {
        // Bars|Beats|ticks (960 a beat), counted from 1.
        let parts: Vec<f64> = t.split('|').map(|x| x.trim().parse().unwrap_or(0.0)).collect();
        let bar = parts.first().copied().unwrap_or(1.0) - 1.0;
        let beat = parts.get(1).copied().unwrap_or(1.0) - 1.0;
        let ticks = parts.get(2).copied().unwrap_or(0.0);
        return Some((bar * 4.0 + beat + ticks / 960.0, true));
    }
    let parts: Vec<&str> = t.split(':').collect();
    match parts.len() {
        // Minutes:seconds.milliseconds
        2 => Some((parts[0].parse::<f64>().ok()? * 60.0 + parts[1].parse::<f64>().ok()?, false)),
        // Hours:minutes:seconds
        3 => {
            let h: f64 = parts[0].parse().ok()?;
            let m: f64 = parts[1].parse().ok()?;
            let s: f64 = parts[2].parse().ok()?;
            Some((h * 3600.0 + m * 60.0 + s, false))
        }
        // Hours:minutes:seconds:frames (frames may carry subframes after a dot)
        4 => {
            let h: f64 = parts[0].parse().ok()?;
            let m: f64 = parts[1].parse().ok()?;
            let s: f64 = parts[2].parse().ok()?;
            let f: f64 = parts[3].parse().unwrap_or(0.0);
            Some((h * 3600.0 + m * 60.0 + s + f / fps.max(1.0), false))
        }
        _ => None,
    }
}

fn pro_tools_text(path: &Path) -> Result<Session, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let base = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut s = Session {
        bpm: 120.0,
        sig: (4, 4),
        ..Session::default()
    };
    let (mut rate, mut fps, mut tc_start) = (48_000.0, 30.0, 0.0);
    // Clip name, and its source file.
    let mut clip_files: Vec<(String, String)> = Vec::new();
    let mut section = "";
    let mut track: Option<SessionTrack> = None;
    // Where, in seconds (or beats when the flag is set), the clips and markers are.
    let mut placed: Vec<(usize, usize, bool)> = Vec::new();
    let mut markers: Vec<(f64, bool, String)> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim_end();
        let compact = line.trim();
        let field = |key: &str| compact.strip_prefix(key).map(|v| v.trim().to_string());
        if compact.starts_with("F I L E S") {
            section = "files";
            continue;
        }
        if compact.starts_with("O N L I N E  C L I P S") {
            section = "clips";
            continue;
        }
        if compact.starts_with("O F F L I N E") || compact.starts_with("P L U G - I N S") {
            section = "";
            continue;
        }
        if compact.starts_with("T R A C K  L I S T I N G") {
            section = "tracks";
            continue;
        }
        if compact.starts_with("M A R K E R S") {
            if let Some(t) = track.take() {
                s.tracks.push(t);
            }
            section = "markers";
            continue;
        }
        if let Some(v) = field("SAMPLE RATE:") {
            rate = v.parse().unwrap_or(48_000.0);
            continue;
        }
        if let Some(v) = field("TIMECODE FORMAT:") {
            fps = v.split_whitespace().next().and_then(|n| n.parse().ok()).unwrap_or(30.0);
            continue;
        }
        if let Some(v) = field("SESSION START TIMECODE:") {
            tc_start = pro_tools_time(&v, rate, fps).map_or(0.0, |t| t.0);
            continue;
        }
        let cols: Vec<&str> = line.split('\t').map(str::trim).collect();
        match section {
            "clips" => {
                if cols.len() >= 2 && cols[0] != "CLIP NAME" && !cols[0].is_empty() {
                    let file = cols[1].trim_start_matches('[').trim_end_matches(']').trim();
                    clip_files.push((cols[0].to_string(), file.to_string()));
                }
            }
            "tracks" => {
                if let Some(name) = field("TRACK NAME:") {
                    if let Some(t) = track.take() {
                        s.tracks.push(t);
                    }
                    track = Some(SessionTrack {
                        name,
                        ..SessionTrack::default()
                    });
                    continue;
                }
                if let Some(v) = field("PLUG-INS:") {
                    if let (false, Some(t)) = (v.is_empty(), track.as_ref()) {
                        s.log.push(format!("\"{}\": its plug-ins were not carried over ({v})", t.name));
                    }
                    continue;
                }
                // CHANNEL, EVENT, CLIP NAME, START, END, DURATION, STATE
                if cols.len() >= 6 && cols[0].parse::<u32>().is_ok() {
                    if cols[0] != "1" {
                        continue;
                    }
                    let (Some((start, bars)), Some((end, _))) =
                        (pro_tools_time(cols[3], rate, fps), pro_tools_time(cols[4], rate, fps))
                    else {
                        continue;
                    };
                    let clip_name = cols[2].to_string();
                    let file = clip_files
                        .iter()
                        .find(|(c, _)| *c == clip_name)
                        .map(|(_, f)| f.clone())
                        .unwrap_or_else(|| format!("{clip_name}.wav"));
                    let shift = if bars { 0.0 } else { tc_start };
                    if let Some(t) = track.as_mut() {
                        t.clips.push(SessionClip {
                            name: clip_name,
                            start: (start - shift).max(0.0),
                            length: (end - start).max(0.0),
                            muted: cols.get(6).is_some_and(|st| st.contains("Muted")),
                            file: Some(find_file(&base, &file)),
                            ..SessionClip::default()
                        });
                        placed.push((s.tracks.len(), t.clips.len() - 1, bars));
                    }
                }
            }
            "markers" => {
                // #, LOCATION, TIME REFERENCE, UNITS, NAME
                if cols.len() >= 5 && cols[0].parse::<u32>().is_ok() {
                    if let Some((at, bars)) = pro_tools_time(cols[1], rate, fps) {
                        markers.push((if bars { at } else { at - tc_start }, bars, cols[4].to_string()));
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(t) = track.take() {
        s.tracks.push(t);
    }
    // Seconds to beats at the session's tempo: the text has no tempo map, so 120 until set here.
    let bpm = s.bpm;
    let mut any_bars = false;
    for (t, c, bars) in placed {
        any_bars |= bars;
        if let Some(clip) = s.tracks.get_mut(t).and_then(|t| t.clips.get_mut(c)) {
            if !bars {
                clip.start *= bpm / 60.0;
                clip.length *= bpm / 60.0;
            }
        }
    }
    if any_bars {
        s.log.push("Times were in bars and beats, read at 4/4".to_string());
    }
    s.markers = markers
        .into_iter()
        .map(|(at, bars, name)| (if bars { at } else { at * bpm / 60.0 }, name))
        .collect();
    if s.tracks.is_empty() {
        return Err("no tracks in the text".to_string());
    }
    s.log.insert(
        0,
        "Pro Tools' text export has no tempo map, and does not say where in its file a trimmed clip starts, \
         so clips play from their file's start. For exact edits, consolidate the clips in Pro Tools before \
         exporting, then set the tempo here."
            .to_string(),
    );
    Ok(s)
}
