//! Converts the app's project model to and from the `.ovrtone` document (`ovrtone-project`).
//!
//! Audio regions point at files on disk; saving copies each distinct file into the archive
//! (`media/…`) and loading extracts them into a per-project folder under the temp directory,
//! where the disk-streaming engine reads them.
use crate::import::audio_region;
use crate::model::{
    CompSegment, Control, Generated, Generator, Marker, Note, Project, Region, RegionParams,
    SendSlot, SigChange, Take, TakeFolder, TempoPoint, Track, TrackKind, TrackParams,
    MAX_REGION_GAIN, MIN_REGION_GAIN,
};
use crate::pattern::{
    Direction, DrumStyle, DrummerParams, PatternParams, PatternRow, StepRate, MAX_ROWS, MAX_STEPS,
};
use egui::Color32;
use ovrtone_engine::InputSource;
use ovrtone_engine::{AudioFile, FadeCurve, SmpteRate};
use ovrtone_project::{
    media_path, CompDoc, ControlDoc, DrummerDoc, MarkerDoc, MediaFile, MidiFxDoc, NoteDoc,
    PatternDoc, PatternRowDoc, ProjectDoc, RegionDoc, SigChangeDoc, TakeDoc, TempoChangeDoc,
    TrackDoc, TrackKindDoc,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn kind_to_doc(k: TrackKind) -> TrackKindDoc {
    match k {
        TrackKind::Instrument => TrackKindDoc::Instrument,
        TrackKind::Audio => TrackKindDoc::Audio,
        TrackKind::Drummer => TrackKindDoc::Drummer,
        TrackKind::Bus => TrackKindDoc::Bus,
        TrackKind::Output => TrackKindDoc::Output,
        TrackKind::Aux => TrackKindDoc::Aux,
        TrackKind::Folder => TrackKindDoc::Folder,
        TrackKind::Vca => TrackKindDoc::Vca,
        TrackKind::ExternalMidi => TrackKindDoc::ExternalMidi,
    }
}

fn kind_from_doc(k: TrackKindDoc) -> TrackKind {
    match k {
        TrackKindDoc::Instrument => TrackKind::Instrument,
        TrackKindDoc::Audio => TrackKind::Audio,
        TrackKindDoc::Drummer => TrackKind::Drummer,
        TrackKindDoc::Bus => TrackKind::Bus,
        TrackKindDoc::Output => TrackKind::Output,
        TrackKindDoc::Aux => TrackKind::Aux,
        TrackKindDoc::Folder => TrackKind::Folder,
        TrackKindDoc::Vca => TrackKind::Vca,
        TrackKindDoc::ExternalMidi => TrackKind::ExternalMidi,
    }
}

/// Names the model keeps as `&'static str` (plug-ins, categories, keys). Known names map to
/// the built-in static strings; anything else (from a hand-edited file) is leaked once per
/// distinct name for the whole session, so reopening files never leaks more.
fn intern(s: &str) -> &'static str {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};
    static OTHERS: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    if let Some(k) = builtin_name(s) {
        return k;
    }
    let mut others = match OTHERS.get_or_init(Default::default).lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(k) = others.get(s) {
        return k;
    }
    let k: &'static str = Box::leak(s.to_string().into_boxed_str());
    others.insert(k);
    k
}

/// Most Flex markers and pitch moves a region keeps from a file.
const MAX_FLEX_POINTS: usize = ovrtone_engine::MAX_FLEX_POINTS;

/// A region's Flex edit from the file: only finite times inside the limits, in order; the
/// sound is made again from it after the project opens.
fn flex_from_doc(d: &ovrtone_project::FlexDoc, original: AudioFile) -> crate::flex_edit::FlexEdit {
    use ovrtone_engine::{FlexMode, PitchShift};
    let mode = FlexMode::from_name(&d.mode).unwrap_or_default();
    let ok = |x: f64| x.is_finite() && (0.0..=2400.0).contains(&x);
    let markers = crate::flex_edit::markers_from(
        d.markers
            .iter()
            .take(MAX_FLEX_POINTS)
            .filter(|m| ok(m[0]) && ok(m[1]))
            .map(|m| (m[0], m[1])),
    );
    let pitch = d
        .pitch
        .iter()
        .take(MAX_FLEX_POINTS)
        .filter(|p| ok(p.start) && ok(p.end) && p.end > p.start && p.semitones.is_finite())
        .map(|p| PitchShift {
            start: p.start,
            end: p.end,
            semitones: p.semitones.clamp(-24.0, 24.0),
            formant: if p.formant.is_finite() { p.formant.clamp(-24.0, 24.0) } else { 0.0 },
            vibrato: if p.vibrato.is_finite() { p.vibrato.clamp(-1.0, 2.0) } else { 0.0 },
        })
        .collect();
    crate::flex_edit::FlexEdit {
        markers,
        pitch,
        ..crate::flex_edit::FlexEdit::new(original, mode)
    }
}

pub(crate) fn builtin_name(s: &str) -> Option<&'static str> {
    if let Some((k, _)) = crate::score::KEYS.iter().find(|(k, _)| *k == s) {
        return Some(k);
    }
    const EXTRA: &[&str] = &[
        "E-Piano",
        "Sampler",
        "Alchemy",
        "Drum Kit",
        "Drum Machine",
        "Tape Delay",
        "Channel EQ",
        "Compressor",
        "Chorus",
        "Space D",
        "MultiMeter",
        "Cmaj",
        "Cmin",
        "Amin",
        "Gmaj",
        "Dmaj",
        "Fmaj",
        "Emin",
    ];
    if let Some(k) = EXTRA.iter().find(|k| **k == s) {
        return Some(k);
    }
    None
}

/// Logic's longest project: 9999 bars.
const MAX_BARS: f32 = 9999.0;
/// Longest note / latest note start inside a region, in beats.
const MAX_BEATS: f32 = MAX_BARS * 99.0 * 4.0;

/// Values read from a file are untrusted: keep them in the ranges the editors and the engine
/// assume (a zero-length region or a 0 signature divides by zero, a pitch above 127 overflows
/// note arithmetic, a reversed key limit panics in `clamp`).
fn finite_in(x: f32, lo: f32, hi: f32, default: f32) -> f32 {
    if x.is_finite() {
        x.clamp(lo, hi)
    } else {
        default
    }
}

fn ordered(range: (u8, u8), min: u8) -> (u8, u8) {
    let (a, b) = (range.0.clamp(min, 127), range.1.clamp(min, 127));
    (a.min(b), a.max(b))
}

fn signature(sig: (u8, u8)) -> (u8, u8) {
    let den = if matches!(sig.1, 1 | 2 | 4 | 8 | 16 | 32 | 64) {
        sig.1
    } else {
        4
    };
    (sig.0.clamp(1, 99), den)
}

/// The effects of a track in a file: the detailed list, else (older files) the names.
fn inserts_from_doc(t: &ovrtone_project::TrackDoc) -> Vec<crate::model::Insert> {
    if t.fx.is_empty() {
        let names: Vec<&str> = t.inserts.iter().map(String::as_str).collect();
        return crate::model::inserts_from_names(&names);
    }
    inserts_from_docs(&t.fx)
}

/// Effects from the file, each with its settings kept in range.
fn inserts_from_docs(fx: &[ovrtone_project::InsertDoc]) -> Vec<crate::model::Insert> {
    fx.iter()
        .filter_map(|d| {
            let kind = ovrtone_engine::FxKind::from_name(&d.kind)?;
            let mut params = [0.0; ovrtone_engine::MAX_FX_PARAMS];
            for (slot, v) in params.iter_mut().zip(&d.params) {
                *slot = *v;
            }
            // Fewer numbers than the effect has: the rest are its defaults.
            for (k, slot) in params.iter_mut().enumerate().skip(d.params.len()) {
                *slot = kind.defaults()[k];
            }
            Some(crate::model::Insert {
                kind,
                bypass: d.bypass,
                params: kind.clamp_params(params),
                sidechain: d
                    .sidechain
                    .map(|v| v as usize)
                    .filter(|_| kind.takes_side()),
            })
        })
        .take(ovrtone_engine::MAX_INSERTS)
        .collect()
}

/// The sends of a track in a file: the list, else (older files) the single send.
fn sends_from_doc(t: &ovrtone_project::TrackDoc) -> Vec<SendSlot> {
    let valid = |bus: u16| {
        (1..=ovrtone_engine::MAX_BUSES as u16)
            .contains(&bus)
            .then_some(bus)
    };
    let mut sends: Vec<SendSlot> = t
        .sends
        .iter()
        .filter_map(|s| {
            Some(SendSlot {
                bus: Some(valid(s.bus)?),
                level: if s.level.is_finite() {
                    s.level.clamp(0.0, 1.0)
                } else {
                    0.0
                },
                pre: s.pre,
            })
        })
        .take(crate::model::MAX_SENDS)
        .collect();
    if sends.is_empty() {
        if let Some(bus) = t.send_bus.and_then(valid) {
            sends.push(SendSlot {
                bus: Some(bus),
                level: if t.send_level.is_finite() {
                    t.send_level.clamp(0.0, 1.0)
                } else {
                    0.0
                },
                pre: t.send_pre,
            });
        }
    }
    sends
}

/// The rows under More from a file, each inside its range.
fn shape_from_doc(s: ovrtone_project::RegionShape) -> ovrtone_project::RegionShape {
    ovrtone_project::RegionShape {
        dynamics: s.dynamics.clamp(-100, 100),
        gate: s.gate.clamp(-100, 100),
        q_velocity: s.q_velocity.clamp(0, 100),
        q_length: s.q_length.clamp(0, 100),
        q_flam: s.q_flam.clamp(0, 100),
        q_range: s.q_range.clamp(0, 100),
        q_soft: s.q_soft.clamp(0, 100),
        ..s
    }
}

/// A region's lanes as the file keeps them. A plugin is kept by where it is.
fn region_lanes_to_doc(t: &Track, r: &Region) -> Vec<ovrtone_project::RegionLaneDoc> {
    use crate::model::AutoTarget;
    r.auto
        .iter()
        .filter_map(|l| {
            let (kind, slot, param) = match l.target {
                AutoTarget::Volume => ("volume", 0, 0),
                AutoTarget::Pan => ("pan", 0, 0),
                AutoTarget::Object(axis) => ("object", 0, axis as u32),
                AutoTarget::Fx { slot, param } => ("fx", i32::from(slot), u32::from(param)),
                AutoTarget::Plugin { uid, param } => {
                    let slot = if t.instrument_plugin.as_ref().is_some_and(|i| i.uid == uid) {
                        -1
                    } else {
                        t.plugins.iter().position(|i| i.uid == uid)? as i32
                    };
                    ("plugin", slot, param)
                }
            };
            Some(ovrtone_project::RegionLaneDoc {
                kind: kind.into(),
                slot,
                param,
                points: points_to_doc(&l.points),
            })
        })
        .collect()
}

/// The spatial mode as the file keeps it: `headphones:7.1.2`, `speakers:5.1` or `stereo:7.1.4`, and
/// empty for off.
fn spatial_mode_text(mode: ovrtone_engine::SpatialMode) -> String {
    use ovrtone_engine::SpatialMode;
    match mode {
        SpatialMode::Off => String::new(),
        SpatialMode::Binaural(l) => format!("headphones:{}", l.name()),
        SpatialMode::Speakers(l) => format!("speakers:{}", l.name()),
        SpatialMode::Folded(l) => format!("stereo:{}", l.name()),
        SpatialMode::Downmix(l, to) => format!("downmix:{}:{}", l.name(), to.name()),
    }
}

/// The spatial mode in a file. Files from before the bed format was its own setting have `headphones`
/// alone, which was 7.1.4, or a layout alone, which was speakers.
fn spatial_mode_of(text: &str) -> ovrtone_engine::SpatialMode {
    use ovrtone_engine::{SpatialMode, SpeakerLayout};
    let layout_of = |name: &str| SpeakerLayout::ALL.into_iter().find(|l| l.name() == name);
    if let Some(rest) = text.strip_prefix("downmix:") {
        if let Some((bed, to)) = rest.split_once(':') {
            if let (Some(bed), Some(to)) = (layout_of(bed), layout_of(to)) {
                return SpatialMode::Downmix(bed, to);
            }
        }
        return SpatialMode::Off;
    }
    match text.split_once(':') {
        Some(("headphones", bed)) => layout_of(bed).map_or(SpatialMode::Off, SpatialMode::Binaural),
        Some(("speakers", bed)) => layout_of(bed).map_or(SpatialMode::Off, SpatialMode::Speakers),
        Some(("stereo", bed)) => layout_of(bed).map_or(SpatialMode::Off, SpatialMode::Folded),
        _ if text == "headphones" => SpatialMode::Binaural(SpeakerLayout::Surround714),
        _ => layout_of(text).map_or(SpatialMode::Off, SpatialMode::Speakers),
    }
}

/// Which lane a track shows: empty for the fader, `pan`, `fx:slot:param` or `plugin:slot:param`
/// (slot -1 is the instrument). A plugin's number changes from run to run, so its place is saved.
fn auto_lane_to_doc(t: &Track) -> String {
    use crate::model::AutoTarget;
    match t.auto.shown {
        AutoTarget::Volume => String::new(),
        AutoTarget::Pan => "pan".into(),
        AutoTarget::Object(axis) => format!("object:{}", axis as u8),
        AutoTarget::Fx { slot, param } => format!("fx:{slot}:{param}"),
        AutoTarget::Plugin { uid, param } => {
            let slot = if t.instrument_plugin.as_ref().is_some_and(|i| i.uid == uid) {
                Some(-1)
            } else {
                t.plugins
                    .iter()
                    .position(|i| i.uid == uid)
                    .map(|at| at as i64)
            };
            slot.map_or_else(String::new, |slot| format!("plugin:{slot}:{param}"))
        }
    }
}

/// The lane a saved track shows; the fader's if what is saved makes no sense for it.
fn auto_lane_from_doc(t: &Track, text: &str) -> crate::model::AutoTarget {
    use crate::model::AutoTarget;
    let mut parts = text.split(':');
    let target = match (parts.next(), parts.next(), parts.next()) {
        (Some("pan"), None, None) => Some(AutoTarget::Pan),
        (Some("object"), Some(axis), None) => axis
            .parse::<usize>()
            .ok()
            .and_then(|k| ovrtone_engine::ObjectAxis::ALL.get(k).copied())
            .map(AutoTarget::Object),
        (Some("fx"), Some(slot), Some(param)) => slot
            .parse()
            .ok()
            .zip(param.parse().ok())
            .map(|(slot, param)| AutoTarget::Fx { slot, param }),
        (Some("plugin"), Some(slot), Some(param)) => {
            let plugin = match slot.parse::<i64>().ok() {
                Some(-1) => t.instrument_plugin.as_ref(),
                Some(at) => usize::try_from(at).ok().and_then(|at| t.plugins.get(at)),
                None => None,
            };
            plugin
                .zip(param.parse().ok())
                .map(|(plugin, param)| AutoTarget::Plugin {
                    uid: plugin.uid,
                    param,
                })
        }
        _ => None,
    };
    target.unwrap_or(AutoTarget::Volume)
}

/// While a project is read, a region's plugin lane holds the plugin's place (the slot plus one, 0
/// for the instrument) where the instance's number goes: see [`resolve_region_plugins`].
fn region_lanes_from_doc(r: &RegionDoc) -> Vec<crate::model::RegionLane> {
    use crate::model::AutoTarget;
    r.auto
        .iter()
        .filter_map(|l| {
            let target = match l.kind.as_str() {
                "volume" => AutoTarget::Volume,
                "pan" => AutoTarget::Pan,
                "object" => match ovrtone_engine::ObjectAxis::ALL.get(l.param as usize) {
                    Some(axis) => AutoTarget::Object(*axis),
                    None => return None,
                },
                "fx" => AutoTarget::Fx {
                    slot: u8::try_from(l.slot).ok()?,
                    param: u8::try_from(l.param).ok()?,
                },
                "plugin" => AutoTarget::Plugin {
                    uid: u64::try_from(i64::from(l.slot) + 1).ok()?,
                    param: l.param,
                },
                _ => return None,
            };
            let points = points_from_doc(target, &l.points);
            (!points.is_empty()).then_some(crate::model::RegionLane { target, points })
        })
        .collect()
}

/// Gives the plugin lanes of a track's regions the numbers of the instances that were just made.
fn resolve_region_plugins(track: &mut Track) {
    use crate::model::AutoTarget;
    let instrument = track.instrument_plugin.as_ref().map(|i| i.uid);
    let plugins: Vec<u64> = track.plugins.iter().map(|i| i.uid).collect();
    let hidden = track.alternatives.iter_mut().flat_map(|a| &mut a.regions);
    for region in track.regions.iter_mut().chain(hidden) {
        region.auto.retain_mut(|lane| {
            let AutoTarget::Plugin { uid, param } = lane.target else {
                return true;
            };
            let real = match uid {
                0 => instrument,
                place => plugins.get(place as usize - 1).copied(),
            };
            match real {
                Some(uid) => {
                    lane.target = AutoTarget::Plugin { uid, param };
                    true
                }
                None => false,
            }
        });
    }
}

/// The automation of a track in a file, with every point made safe.
fn automation_from_doc(t: &ovrtone_project::TrackDoc) -> crate::model::Automation {
    use crate::model::{AutoMode, AutoTarget, Automation};
    let mut a = Automation {
        mode: AutoMode::ALL
            .into_iter()
            .find(|m| m.label() == t.auto_mode)
            .unwrap_or_default(),
        ..Automation::default()
    };
    a.volume = points_from_doc(AutoTarget::Volume, &t.auto_volume);
    a.pan = points_from_doc(AutoTarget::Pan, &t.auto_pan);
    for (axis, lane) in ovrtone_engine::ObjectAxis::ALL.into_iter().zip(&t.auto_object) {
        a.object[axis as usize] = points_from_doc(AutoTarget::Object(axis), lane);
    }
    for l in t.auto_fx.iter().take(MAX_FX_LANES) {
        let known = usize::from(l.slot) < ovrtone_engine::MAX_INSERTS
            && usize::from(l.param) < ovrtone_engine::MAX_FX_PARAMS;
        let target = AutoTarget::Fx {
            slot: l.slot,
            param: l.param,
        };
        if known && a.lane(target).is_empty() {
            let points = points_from_doc(target, &l.points);
            if !points.is_empty() {
                a.fx.push(crate::model::FxLane {
                    slot: l.slot,
                    param: l.param,
                    points,
                });
            }
        }
    }
    a
}

/// Most effect lanes of one track a file may have (8 effects, 16 parameters each).
const MAX_FX_LANES: usize = ovrtone_engine::MAX_INSERTS * ovrtone_engine::MAX_FX_PARAMS;

/// Points as the file keeps them: [beat, value, curve].
fn points_to_doc(points: &[crate::model::AutoPoint]) -> Vec<[f64; 3]> {
    points
        .iter()
        .map(|p| [p.beat, f64::from(p.value), f64::from(p.curve)])
        .collect()
}

/// The points of a lane from a file: only finite ones, values and bends in range, in order of
/// beat, one at each beat, and no more than a lane keeps.
fn points_from_doc(
    target: crate::model::AutoTarget,
    doc: &[[f64; 3]],
) -> Vec<crate::model::AutoPoint> {
    let (lo, hi) = crate::model::Automation::range(target);
    let mut v: Vec<crate::model::AutoPoint> = doc
        .iter()
        .filter(|p| p[0].is_finite() && p[0] >= 0.0 && p[1].is_finite())
        .map(|p| crate::model::AutoPoint {
            beat: p[0],
            value: (p[1] as f32).clamp(lo, hi),
            curve: if p[2].is_finite() {
                (p[2] as f32).clamp(-1.0, 1.0)
            } else {
                0.0
            },
        })
        .collect();
    v.sort_by(|a, b| a.beat.total_cmp(&b.beat));
    v.dedup_by(|b, a| (a.beat - b.beat).abs() < 1e-9);
    v.truncate(crate::model::Automation::MAX_POINTS);
    v
}

/// A region as it is saved. Its files are added to `media`, each once.
fn region_to_doc(
    t: &Track,
    r: &Region,
    media: &mut Vec<MediaFile>,
    by_source: &mut HashMap<PathBuf, String>,
) -> RegionDoc {
    RegionDoc {
        auto: region_lanes_to_doc(t, r),
        gain_line: r.gain_line.iter().map(|(b, db)| [*b, *db]).collect(),
        song_pattern: r.params.pattern,
        name: r.name.clone(),
        start_bar: r.start_bar,
        len_bars: r.len_bars,
        notes: r
            .notes
            .iter()
            .map(|n| NoteDoc {
                start: n.start,
                len: n.len,
                pitch: n.pitch,
                vel: n.vel,
                muted: n.muted,
                color: n.color,
                slide: n.slide,
            })
            .collect(),
        controls: r
            .controls
            .iter()
            .map(|c| ControlDoc {
                start: c.start,
                num: c.num,
                val: c.val,
            })
            .collect(),
        takes: r.folder.as_ref().map_or_else(Vec::new, |f| {
            f.takes
                .iter()
                .map(|t| TakeDoc {
                    audio_offset: t.audio_offset,
                    audio: t.file.as_ref().map(|f| {
                        by_source
                            .entry(f.0.file.path.clone())
                            .or_insert_with(|| {
                                let zip_path = media_path(media.len(), &f.0.file.path);
                                media.push(MediaFile {
                                    zip_path: zip_path.clone(),
                                    source: f.0.file.path.clone(),
                                });
                                zip_path
                            })
                            .clone()
                    }),
                    window: t.window.map(|(a, b)| [a, b]),
                    gain_db: t.gain_db,
                    name: t.name.clone(),
                    color: t.color.map(|c| [c.r(), c.g(), c.b()]),
                    notes: t
                        .notes
                        .iter()
                        .map(|n| NoteDoc {
                            start: n.start,
                            len: n.len,
                            pitch: n.pitch,
                            vel: n.vel,
                            muted: n.muted,
                            color: n.color,
                            slide: n.slide,
                        })
                        .collect(),
                    controls: t
                        .controls
                        .iter()
                        .map(|c| ControlDoc {
                            start: c.start,
                            num: c.num,
                            val: c.val,
                        })
                        .collect(),
                })
                .collect()
        }),
        // What a take that is only being listened to has set aside is what gets saved.
        comp: r
            .folder
            .as_ref()
            .map_or_else(Vec::new, |f| comp_to_doc(f.real_comp())),
        saved_comps: r.folder.as_ref().map_or_else(Vec::new, |f| {
            f.saved
                .iter()
                .map(|c| ovrtone_project::SavedCompDoc {
                    name: c.name.clone(),
                    comp: comp_to_doc(&c.comp),
                })
                .collect()
        }),
        // A Flex edit is saved as the recording plus the edit, never the sound
        // made from it.
        audio: r.audio.as_ref().map(|a| {
            let file = a.flex.as_ref().map_or(&a.file, |f| &f.original);
            by_source
                .entry(file.path.clone())
                .or_insert_with(|| {
                    let zip_path = media_path(media.len(), &file.path);
                    media.push(MediaFile {
                        zip_path: zip_path.clone(),
                        source: file.path.clone(),
                    });
                    zip_path
                })
                .clone()
        }),
        follow_bpm: r.audio.as_ref().and_then(|a| a.follow_bpm),
        flex: r
            .audio
            .as_ref()
            .and_then(|a| a.flex.as_ref())
            .map(|f| ovrtone_project::FlexDoc {
                mode: f.mode.name().into(),
                markers: f.markers.iter().map(|m| [m.src, m.dst]).collect(),
                pitch: f
                    .pitch
                    .iter()
                    .map(|p| ovrtone_project::FlexPitchDoc {
                        start: p.start,
                        end: p.end,
                        semitones: p.semitones,
                        formant: p.formant,
                        vibrato: p.vibrato,
                    })
                    .collect(),
            }),
        shape: r.params.shape,
        mute: r.params.mute,
        color: r.params.color.map(|c| [c.r(), c.g(), c.b()]),
        group: r.params.group,
        quantize: r.params.quantize,
        transpose: r.params.transpose,
        velocity: r.params.velocity,
        source_start: r.params.source_start,
        source_len: r.params.source_len,
        gain_db: r.params.gain_db,
        fade_in: r.params.fade_in,
        fade_out: r.params.fade_out,
        fade_curve: curve_name(r.params.fade_curve).into(),
        reverse: r.params.reverse,
        crossfade: r.params.crossfade,
        pattern: match r.generated.as_ref().map(|g| &g.source) {
            Some(Generator::Pattern(p)) => Some(pattern_to_doc(p)),
            _ => None,
        },
        drummer: match r.generated.as_ref().map(|g| &g.source) {
            Some(Generator::Drummer(d)) => Some(drummer_to_doc(d)),
            _ => None,
        },
    }
}

/// The document for `p` plus the media files to embed (each distinct audio file once).
pub fn to_doc(p: &Project) -> (ProjectDoc, Vec<MediaFile>) {
    let mut media: Vec<MediaFile> = Vec::new();
    let mut by_source: HashMap<PathBuf, String> = HashMap::new();
    let tracks = p
        .tracks
        .iter()
        .map(|t| TrackDoc {
            channel: t.params.channel,
            auto_plugins: t
                .auto
                .plugin_lanes
                .iter()
                .filter_map(|l| {
                    let slot = if t.instrument_plugin.as_ref().is_some_and(|i| i.uid == l.uid) {
                        -1
                    } else {
                        t.plugins.iter().position(|i| i.uid == l.uid)? as i32
                    };
                    Some(ovrtone_project::AutoPluginDoc {
                        slot,
                        param: l.param,
                        points: l
                            .points
                            .iter()
                            .map(|q| [q.beat, f64::from(q.value), f64::from(q.curve)])
                            .collect(),
                    })
                })
                .collect(),
            name: t.name.clone(),
            kind: kind_to_doc(t.kind),
            color: [t.color.r(), t.color.g(), t.color.b()],
            mute: t.mute,
            solo: t.solo,
            volume_db: t.volume_db,
            pan: t.pan,
            bed_pan: t.bed_pan && t.object.is_some(),
            locked: t.locked,
            reference: t.reference,
            trim_db: if t.trim_db.is_finite() {
                t.trim_db.clamp(-36.0, 36.0)
            } else {
                0.0
            },
            object: t.object.map(|o| ovrtone_project::ObjectDoc {
                azimuth: o.azimuth,
                elevation: o.elevation,
                size: o.size,
                lfe: o.lfe,
                spread: o.spread,
                center: o.center,
                distance: o
                    .distance
                    .and_then(|d| ovrtone_engine::Distance::ALL.iter().position(|x| *x == d))
                    .map(|k| k as u8),
                radius: o.radius,
                snap: o.snap,
                divergence: o.divergence,
                bus: o.bus,
            }),
            inserts: t.inserts.iter().map(|s| s.name().to_string()).collect(),
            plugins: t.plugins.iter().map(plugin_to_doc).collect(),
            instrument_plugin: t.instrument_plugin.as_ref().map(plugin_to_doc),
            fx: t
                .inserts
                .iter()
                .map(|s| ovrtone_project::InsertDoc {
                    kind: s.name().into(),
                    bypass: s.bypass,
                    params: s.params[..s.kind.params().len()].to_vec(),
                    sidechain: s.sidechain.map(|v| v as u32),
                })
                .collect(),
            sends: t
                .sends
                .iter()
                .filter_map(|s| {
                    Some(ovrtone_project::SendDoc {
                        bus: s.bus?,
                        level: s.level,
                        pre: s.pre,
                    })
                })
                .collect(),
            alternatives: t
                .alternatives
                .iter()
                .map(|a| ovrtone_project::AlternativeDoc {
                    name: a.name.clone(),
                    regions: a
                        .regions
                        .iter()
                        .map(|r| region_to_doc(t, r, &mut media, &mut by_source))
                        .collect(),
                })
                .collect(),
            active_alternative: t.active_alt,
            auto_mode: t.auto.mode.label().into(),
            show_auto: t.show_auto,
            auto_lane: auto_lane_to_doc(t),
            auto_volume: points_to_doc(&t.auto.volume),
            auto_pan: points_to_doc(&t.auto.pan),
            auto_object: if t.auto.object.iter().all(Vec::is_empty) {
                Vec::new()
            } else {
                t.auto.object.iter().map(|l| points_to_doc(l)).collect()
            },
            auto_fx: t
                .auto
                .fx
                .iter()
                .map(|l| ovrtone_project::AutoFxDoc {
                    slot: l.slot,
                    param: l.param,
                    points: points_to_doc(&l.points),
                })
                .collect(),
            group: t.group,
            // No stock instruments any more; an older file's entry is left unread.
            instrument: None,
            category: t.category.map(str::to_string),
            icon: t
                .icon
                .and_then(crate::track_icons::icon_name)
                .map(str::to_string),
            transpose: t.params.transpose,
            velocity: t.params.velocity,
            key_limit: t.params.key_limit,
            vel_limit: t.params.vel_limit,
            send_bus: t.sends.first().and_then(|s| s.bus),
            send_level: t.sends.first().map_or(0.0, |s| s.level),
            send_pre: t.sends.first().is_some_and(|s| s.pre),
            height: t.height,
            indent: t.indent,
            folded: t.folded,
            on_timeline: t.on_timeline,
            off: t.off,
            vca: t.vca,
            midi_port: t.midi_port.clone(),
            midi_channel: t.midi_channel,
            input: match t.input {
                InputSource::Mono(c) | InputSource::Stereo(c) => c,
            },
            input_stereo: matches!(t.input, InputSource::Stereo(_)),
            output: t.output,
            hardware_out: t.hardware_out,
            aux_bus: t.aux_bus,
            slide_range: t.slide_range,
            groove: t.params.groove,
            follow_groove: t.params.follow_groove,
            midi_fx: t
                .midi_fx
                .iter()
                .map(|s| MidiFxDoc {
                    kind: s.fx.name().into(),
                    on: s.on,
                    settings: s.fx.numbers(),
                    script: match &s.fx {
                        crate::midi_fx::MidiFx::Scripter { source } => source.clone(),
                        _ => String::new(),
                    },
                })
                .collect(),
            regions: t
                .regions
                .iter()
                .map(|r| region_to_doc(t, r, &mut media, &mut by_source))
                .collect(),
        })
        .collect();
    let doc = ProjectDoc {
        name: p.name.clone(),
        tempo: p.tempo,
        sig: p.sig,
        punch: p.punch,
        pre_roll: p.pre_roll,
        post_roll: p.post_roll,
        punch_start: p.punch_start,
        punch_end: p.punch_end,
        rec_replace: p.rec_replace,
        snap_zero: p.snap_zero,
        master_plugins: p.master_plugins.iter().map(plugin_to_doc).collect(),
        master_fx: p
            .master_strip
            .inserts
            .iter()
            .map(|s| ovrtone_project::InsertDoc {
                kind: s.name().into(),
                bypass: s.bypass,
                params: s.params[..s.kind.params().len()].to_vec(),
                sidechain: None,
            })
            .collect(),
        midi_map: midi_map_to_doc(p),
        grooves: p.grooves.clone(),
        screensets: p.screensets.clone(),
        song_patterns: p
            .patterns
            .iter()
            .map(|x| ovrtone_project::SongPatternDoc {
                id: x.id,
                name: x.name.clone(),
                color: [x.color.r(), x.color.g(), x.color.b()],
                steps: x.steps,
                lanes: x
                    .lanes
                    .iter()
                    .map(|l| ovrtone_project::LaneDoc {
                        channel: l.channel,
                        step_pitch: l.step_pitch,
                        pan: l.pan.clone(),
                        notes: l
                            .notes
                            .iter()
                            .map(|n| NoteDoc {
                                start: n.start,
                                len: n.len,
                                pitch: n.pitch,
                                vel: n.vel,
                                muted: n.muted,
                                color: n.color,
                                slide: n.slide,
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect(),
        key: p.key.to_string(),
        position_beats: p.position_beats,
        cycle: p.cycle,
        cycle_start: p.cycle_start,
        cycle_end: p.cycle_end,
        metronome: p.metronome,
        count_in: p.count_in,
        spatial_mode: spatial_mode_text(p.spatial_mode),
        spatial_law: p.spatial_law.name().to_ascii_lowercase(),
        spatial_room: p.spatial_room,
        spatial_volume_db: p.spatial_volume_db,
        spatial_distance: p
            .spatial_channels
            .distance
            .iter()
            .map(|d| d.name().to_lowercase())
            .collect(),
        spatial_lfe_hz: p.spatial_channels.lfe_hz,
        spatial_buses: p
            .spatial_buses
            .iter()
            .map(|b| ovrtone_project::SpatialBusDoc {
                name: b.name.clone(),
                layout: b.layout.name().to_string(),
                plugins: b.plugins.iter().map(plugin_to_doc).collect(),
                fx: b
                    .fx
                    .iter()
                    .map(|s| ovrtone_project::InsertDoc {
                        kind: s.name().into(),
                        bypass: s.bypass,
                        params: s.params[..s.kind.params().len()].to_vec(),
                        sidechain: None,
                    })
                    .collect(),
                direction: b.place.direction,
                elevation: b.place.elevation,
                spread: b.place.spread,
                size: b.place.size,
                centre: b.place.centre,
                lfe: b.place.lfe,
                gain: b.place.gain,
                muted: b.place.muted,
            })
            .collect(),
        mixer_snapshots: p
            .mixer_snapshots
            .iter()
            .map(|m| ovrtone_project::MixerSnapshotDoc {
                name: m.name.clone(),
                strips: m
                    .strips
                    .iter()
                    .map(|s| ovrtone_project::SnapshotStripDoc {
                        name: s.name.clone(),
                        volume_db: s.volume_db,
                        pan: s.pan,
                        mute: s.mute,
                        solo: s.solo,
                        trim_db: s.trim_db,
                        fx: s
                            .inserts
                            .iter()
                            .map(|i| ovrtone_project::InsertDoc {
                                kind: i.name().into(),
                                bypass: i.bypass,
                                params: i.params[..i.kind.params().len()].to_vec(),
                                sidechain: i.sidechain.map(|v| v as u32),
                            })
                            .collect(),
                        sends: s
                            .sends
                            .iter()
                            .filter_map(|s| {
                                Some(ovrtone_project::SendDoc {
                                    bus: s.bus?,
                                    level: s.level,
                                    pre: s.pre,
                                })
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect(),
        auto_monitor: p.auto_monitor,
        wait_for_sound: p.wait_for_sound,
        trigger_db: p.trigger_db,
        nudge_beats: p.nudge_beats,
        varispeed: p.varispeed,
        notes: p.notes.clone(),
        end_beat: p.end_beat,
        master_vol: p.master_vol,
        selected_track: p.selected_track,
        tracks,
        groups: p
            .groups
            .iter()
            .map(|g| ovrtone_project::GroupDoc {
                name: g.name.clone(),
                volume: g.volume,
                pan: g.pan,
                mute: g.mute,
                solo: g.solo,
            })
            .collect(),
        tempo_changes: p
            .tempo_changes
            .iter()
            .map(|t| TempoChangeDoc {
                beat: t.beat,
                bpm: t.bpm,
                ramp: t.ramp,
            })
            .collect(),
        tempo_ramp: p.tempo_ramp,
        smpte_rate: smpte_rate_name(p.smpte_rate).into(),
        smpte_start: p.smpte_start,
        mtc_out: p.mtc_out.clone(),
        clock_out: p.clock_out.clone(),
        mtc_in: p.mtc_in.clone(),
        sig_changes: p
            .sig_changes
            .iter()
            .map(|c| SigChangeDoc {
                bar: c.bar,
                num: c.num,
                den: c.den,
            })
            .collect(),
        markers: p
            .markers
            .iter()
            .map(|m| MarkerDoc {
                beat: m.beat,
                name: m.name.clone(),
                color: [m.color.r(), m.color.g(), m.color.b()],
            })
            .collect(),
        sections: p
            .sections
            .iter()
            .map(|x| ovrtone_project::SectionDoc {
                beat: x.beat,
                len: x.len,
                name: x.name.clone(),
                color: [x.color.r(), x.color.g(), x.color.b()],
            })
            .collect(),
        chords: p
            .chords
            .iter()
            .map(|c| ovrtone_project::ChordDoc {
                beat: c.beat,
                name: c.name.clone(),
            })
            .collect(),
        ..ProjectDoc::default()
    };
    (doc, media)
}

/// Builds a project from `doc`; audio regions open their files from `media_dir`. Regions whose
/// file is missing or unreadable are dropped and reported in the returned warnings.
/// The take folder of a region document, if it has a sensible one: two or more takes and a
/// comp of ordered stretches that point at them.
fn folder_from_doc(r: &RegionDoc) -> Option<TakeFolder> {
    if r.takes.len() < 2 || r.comp.is_empty() {
        return None;
    }
    let takes: Vec<Take> = r
        .takes
        .iter()
        .map(|t| Take {
            name: t.name.chars().take(80).collect(),
            color: t.color.map(|[r, g, b]| egui::Color32::from_rgb(r, g, b)),
            audio_offset: t.audio_offset.filter(|o| o.is_finite()),
            file: None,
            window: t
                .window
                .filter(|w| w[0].is_finite() && w[1].is_finite())
                .map(|w| (w[0], w[1])),
            gain_db: finite_in(t.gain_db, MIN_REGION_GAIN, MAX_REGION_GAIN, 0.0),
            notes: t
                .notes
                .iter()
                .map(|n| Note {
                    start: finite_in(n.start, 0.0, MAX_BEATS, 0.0),
                    len: finite_in(n.len, 1.0 / 960.0, MAX_BEATS, 0.25),
                    pitch: n.pitch.min(127),
                    vel: n.vel.clamp(1, 127),
                    muted: n.muted,
                    color: n.color,
                    slide: n.slide,
                })
                .collect(),
            controls: t
                .controls
                .iter()
                .map(|c| Control {
                    start: finite_in(c.start, 0.0, MAX_BEATS, 0.0),
                    num: c.num.min(127),
                    val: c.val.min(127),
                })
                .collect(),
        })
        .collect();
    let comp = comp_from_doc(&r.comp, takes.len());
    let len = comp.last()?.end;
    let saved = r
        .saved_comps
        .iter()
        .map(|c| crate::model::SavedComp {
            name: c.name.clone(),
            comp: comp_from_doc(&c.comp, takes.len()),
        })
        // One that comes to nothing, or that does not fit the folder, is no use to go back to.
        .filter(|c| c.comp.last().is_some_and(|last| last.end <= len + 1e-3))
        .collect();
    Some(TakeFolder {
        takes,
        comp,
        len,
        open: false,
        saved,
        audition: None,
    })
}

fn comp_to_doc(comp: &[CompSegment]) -> Vec<CompDoc> {
    comp.iter()
        .map(|s| CompDoc {
            start: s.start,
            end: s.end,
            take: s.take as u32,
        })
        .collect()
}

fn comp_from_doc(doc: &[CompDoc], takes: usize) -> Vec<CompSegment> {
    let mut comp = Vec::new();
    for s in doc {
        let (start, end) = (
            finite_in(s.start, 0.0, MAX_BEATS, 0.0),
            finite_in(s.end, 0.0, MAX_BEATS, 0.0),
        );
        if end > start && (s.take as usize) < takes {
            comp.push(CompSegment {
                start,
                end,
                take: s.take as usize,
            });
        }
    }
    comp
}

/// A list of saved regions. A region whose file is missing or unreadable is left out and reported.
fn regions_from_doc(
    list: &[RegionDoc],
    opened: &mut HashMap<String, Option<AudioFile>>,
    warnings: &mut Vec<String>,
    media_dir: &Path,
    ctx: &egui::Context,
) -> Vec<Region> {
    let mut regions = Vec::new();
    for r in list {
        let audio = match &r.audio {
            None => None,
            Some(zip_path) if !ovrtone_project::is_media_path(zip_path) => {
                // Never follow a path out of the project's media (e.g. `C:\…`, `../…`).
                warnings.push(format!("{zip_path}: not a media path in the project"));
                continue;
            }
            Some(zip_path) => {
                let file = opened
                    .entry(zip_path.clone())
                    .or_insert_with(|| match AudioFile::open(media_dir.join(zip_path)) {
                        Ok(f) => Some(f),
                        Err(e) => {
                            warnings.push(format!("{}: {e}", zip_path));
                            None
                        }
                    });
                match file {
                    Some(f) => {
                        let mut a = audio_region(f.clone(), ctx);
                        a.follow_bpm = r.follow_bpm.filter(|b| b.is_finite() && *b > 0.0);
                        a.flex = r.flex.as_ref().map(|d| flex_from_doc(d, f.clone()));
                        Some(a)
                    }
                    None => continue,
                }
            }
        };
        let mut folder = folder_from_doc(r);
        // A take with a file of its own. One that cannot be read stays in the folder, silent.
        if let Some(folder) = folder.as_mut() {
            for (take, d) in folder.takes.iter_mut().zip(&r.takes) {
                let Some(zip_path) = &d.audio else { continue };
                let file = if ovrtone_project::is_media_path(zip_path) {
                    opened
                        .entry(zip_path.clone())
                        .or_insert_with(|| match AudioFile::open(media_dir.join(zip_path)) {
                            Ok(f) => Some(f),
                            Err(e) => {
                                warnings.push(format!("{zip_path}: {e}"));
                                None
                            }
                        })
                        .clone()
                } else {
                    warnings.push(format!("{zip_path}: not a media path in the project"));
                    None
                };
                match file {
                    Some(f) => {
                        take.file = Some(crate::model::TakeFile(audio_region(f, ctx)));
                    }
                    None => take.window = Some((0.0, 0.0)),
                }
            }
        }
        let mut region = Region {
            name: r.name.clone(),
            start_bar: finite_in(r.start_bar, 1.0, MAX_BARS, 1.0),
            len_bars: finite_in(r.len_bars, 1.0 / 64.0, MAX_BARS, 1.0),
            notes: r
                .notes
                .iter()
                .map(|n| Note {
                    start: finite_in(n.start, 0.0, MAX_BEATS, 0.0),
                    len: finite_in(n.len, 1.0 / 960.0, MAX_BEATS, 0.25),
                    pitch: n.pitch.min(127),
                    vel: n.vel.clamp(1, 127),
                    muted: n.muted,
                    color: n.color,
                    slide: n.slide,
                })
                .collect(),
            controls: r
                .controls
                .iter()
                .map(|c| Control {
                    start: finite_in(c.start, 0.0, MAX_BEATS, 0.0),
                    num: c.num.min(127),
                    val: c.val.min(127),
                })
                .collect(),
            folder: None,
            auto: region_lanes_from_doc(r),
            gain_line: r
                .gain_line
                .iter()
                .filter(|[b, db]| b.is_finite() && db.is_finite())
                .map(|[b, db]| (b.max(0.0), db.clamp(-96.0, 24.0)))
                .collect(),
            generated: generated_from_doc(r),
            audio,
            params: RegionParams {
                pattern: 0,
                pattern_len: 0.0,
                pattern_color: None,
                color: r.color.map(|c| Color32::from_rgb(c[0], c[1], c[2])),
                group: r.group,
                shape: shape_from_doc(r.shape),
                mute: r.mute,
                quantize: r.quantize.filter(|q| q.is_finite() && *q > 0.0),
                transpose: r.transpose,
                velocity: r.velocity,
                source_start: finite_in(r.source_start, 0.0, 1.0e7, 0.0),
                source_len: finite_in(r.source_len, 0.0, 1.0e7, 0.0),
                gain_db: finite_in(r.gain_db, MIN_REGION_GAIN, MAX_REGION_GAIN, 0.0),
                fade_in: finite_in(r.fade_in, 0.0, MAX_FADE, 0.0),
                fade_out: finite_in(r.fade_out, 0.0, MAX_FADE, 0.0),
                fade_curve: curve_from_name(&r.fade_curve),
                reverse: r.reverse,
                crossfade: r.crossfade,
            },
        };
        // A take folder's notes are what its comp plays.
        region.params.pattern = r.song_pattern;
        if let Some(folder) = folder {
            folder.apply(&mut region.notes, &mut region.controls);
            region.folder = Some(folder);
        }
        regions.push(region);
    }
    regions
}

pub fn from_doc(doc: &ProjectDoc, media_dir: &Path, ctx: &egui::Context) -> (Project, Vec<String>) {
    let mut warnings = Vec::new();
    let mut opened: HashMap<String, Option<AudioFile>> = HashMap::new();
    let mut tracks = Vec::new();
    for t in &doc.tracks {
        let regions = regions_from_doc(&t.regions, &mut opened, &mut warnings, media_dir, ctx);
        tracks.push(Track {
            name: t.name.clone(),
            kind: kind_from_doc(t.kind),
            color: Color32::from_rgb(t.color[0], t.color[1], t.color[2]),
            mute: t.mute,
            solo: t.solo,
            rec: false,
            monitor: false,
            volume_db: t.volume_db.clamp(-60.0, 6.0),
            pan: t.pan.clamp(-1.0, 1.0),
            bed_pan: t.bed_pan && t.object.is_some(),
            locked: t.locked,
            reference: t.reference,
            trim_db: if t.trim_db.is_finite() {
                t.trim_db.clamp(-36.0, 36.0)
            } else {
                0.0
            },
            object: t.object.map(|o| ovrtone_engine::ObjectPlace {
                azimuth: o.azimuth,
                elevation: o.elevation,
                size: o.size,
                lfe: o.lfe,
                spread: o.spread,
                center: o.center,
                distance: o
                    .distance
                    .and_then(|k| ovrtone_engine::Distance::ALL.get(usize::from(k)).copied()),
                radius: if o.radius.is_finite() { o.radius.clamp(0.0, 1.0) } else { 1.0 },
                snap: o.snap,
                divergence: if o.divergence.is_finite() { o.divergence.clamp(0.0, 1.0) } else { 0.0 },
                bus: o.bus.min(ovrtone_engine::MAX_SPATIAL_BUSES as u8),
            }),
            inserts: inserts_from_doc(t),
            plugins: t
                .plugins
                .iter()
                .filter(|d| !d.id.is_empty())
                .take(ovrtone_engine::MAX_INSERTS)
                .map(plugin_from_doc)
                .collect(),
            instrument_plugin: t
                .instrument_plugin
                .as_ref()
                .filter(|d| !d.id.is_empty())
                .map(plugin_from_doc),
            regions,
            params: TrackParams {
                channel: t.channel,
                transpose: t.transpose,
                velocity: t.velocity,
                key_limit: ordered(t.key_limit, 0),
                vel_limit: ordered(t.vel_limit, 1),
                groove: t.groove,
                follow_groove: t.follow_groove && !t.groove,
            },
            sends: sends_from_doc(t),
            auto: automation_from_doc(t),
            group: t.group,
            category: t.category.as_deref().map(intern),
            icon: t.icon.as_deref().and_then(crate::track_icons::icon_named),
            frozen: None,
            on_timeline: t.on_timeline,
            off: t.off,
            height: finite_in(t.height, Track::MIN_HEIGHT, Track::MAX_HEIGHT, 1.0),
            indent: t.indent,
            folded: t.folded,
            vca: t.vca,
            midi_port: port_name(&t.midi_port),
            midi_channel: t.midi_channel.clamp(1, 16),
            input: input_source(t.input, t.input_stereo),
            hardware_out: t.hardware_out,
            aux_bus: t.aux_bus.min(ovrtone_engine::MAX_BUSES as u16),
            slide_range: match t.slide_range {
                0 => crate::model::DEFAULT_SLIDE_RANGE,
                range => range.min(48),
            },
            output: if usize::from(t.output) <= ovrtone_engine::MAX_BUSES {
                t.output
            } else {
                0
            },
            show_auto: t.show_auto,
            alternatives: t
                .alternatives
                .iter()
                .map(|a| crate::model::TrackAlternative {
                    name: a.name.chars().take(32).collect(),
                    regions: regions_from_doc(
                        &a.regions,
                        &mut opened,
                        &mut warnings,
                        media_dir,
                        ctx,
                    ),
                })
                .collect(),
            active_alt: t
                .active_alternative
                .min(t.alternatives.len().saturating_sub(1)),
            midi_fx: t
                .midi_fx
                .iter()
                .take(crate::midi_fx::MAX_FX)
                .filter_map(|d| {
                    Some(crate::midi_fx::FxSlot {
                        fx: crate::midi_fx::MidiFx::from_parts(&d.kind, &d.settings, &d.script)?,
                        on: d.on,
                    })
                })
                .collect(),
        });
    }
    let mut p = Project::empty();
    p.name = doc.name.clone();
    p.tempo = doc.tempo.clamp(5.0, 990.0);
    p.tempo_changes = doc
        .tempo_changes
        .iter()
        .map(|t| TempoPoint {
            beat: t.beat,
            bpm: t.bpm,
            ramp: t.ramp,
        })
        .collect();
    p.tempo_ramp = doc.tempo_ramp;
    p.rec_replace = doc.rec_replace;
    p.snap_zero = doc.snap_zero;
    p.tidy_tempo();
    p.smpte_rate = smpte_rate_from_name(&doc.smpte_rate);
    p.smpte_start = valid_smpte_start(doc.smpte_start, p.smpte_rate);
    p.mtc_out = port_name(&doc.mtc_out);
    p.clock_out = port_name(&doc.clock_out);
    p.mtc_in = port_name(&doc.mtc_in);
    p.markers = doc
        .markers
        .iter()
        .map(|m| Marker {
            beat: m.beat,
            name: m.name.chars().take(64).collect(),
            color: Color32::from_rgb(m.color[0], m.color[1], m.color[2]),
        })
        .collect();
    p.tidy_markers();
    p.sections = doc
        .sections
        .iter()
        .map(|x| crate::model::Section {
            beat: x.beat,
            len: x.len,
            name: x.name.chars().take(64).collect(),
            color: Color32::from_rgb(x.color[0], x.color[1], x.color[2]),
        })
        .collect();
    p.chords = doc
        .chords
        .iter()
        .map(|c| crate::model::ChordMark {
            beat: c.beat,
            name: c.name.chars().take(16).collect(),
        })
        .collect();
    crate::arrangement_track::tidy(&mut p);
    p.sig = signature(doc.sig);
    p.sig_changes = doc
        .sig_changes
        .iter()
        .map(|c| SigChange {
            bar: c.bar,
            num: c.num,
            den: c.den,
        })
        .collect();
    p.tidy_signatures();
    p.key = intern(&doc.key);
    p.position_beats = doc.position_beats.max(0.0);
    p.patterns = doc
        .song_patterns
        .iter()
        .filter(|x| x.id != 0)
        .map(|x| crate::song_pattern::SongPattern {
            id: x.id,
            name: x.name.clone(),
            color: Color32::from_rgb(x.color[0], x.color[1], x.color[2]),
            steps: x.steps.clamp(1, 256),
            lanes: x
                .lanes
                .iter()
                .map(|l| crate::song_pattern::Lane {
                    channel: l.channel,
                    step_pitch: l.step_pitch.min(127),
                    pan: l.pan.iter().take(64).map(|v| (*v).clamp(-64, 63)).collect(),
                    notes: l
                        .notes
                        .iter()
                        .map(|n| Note {
                            start: finite_in(n.start, 0.0, MAX_BEATS, 0.0),
                            len: finite_in(n.len, 1.0 / 960.0, MAX_BEATS, 0.25),
                            pitch: n.pitch.min(127),
                            vel: n.vel.clamp(1, 127),
                            muted: n.muted,
                            color: n.color,
                            slide: n.slide,
                        })
                        .collect(),
                })
                .collect(),
        })
        .collect();
    p.grooves = crate::groove_template::from_docs(&doc.grooves);
    p.screensets = crate::screensets::from_doc(&doc.screensets);
    p.punch = doc.punch;
    p.pre_roll = if doc.pre_roll.is_finite() { doc.pre_roll.clamp(0.0, 8.0) } else { 0.0 };
    p.post_roll = if doc.post_roll.is_finite() { doc.post_roll.clamp(0.0, 8.0) } else { 0.0 };
    p.punch_start = doc.punch_start.max(0.0);
    p.punch_end = doc.punch_end.max(p.punch_start);
    p.cycle = doc.cycle;
    p.cycle_start = doc.cycle_start.max(0.0);
    p.cycle_end = doc.cycle_end.max(p.cycle_start);
    p.metronome = doc.metronome;
    p.count_in = doc.count_in;
    p.spatial_mode = spatial_mode_of(&doc.spatial_mode);
    p.spatial_law = ovrtone_engine::SpatialLaw::ALL
        .into_iter()
        .find(|l| l.name().eq_ignore_ascii_case(&doc.spatial_law))
        .unwrap_or(ovrtone_engine::SpatialLaw::Smooth);
    p.spatial_volume_db = doc.spatial_volume_db.clamp(-24.0, 6.0);
    p.mixer_snapshots = doc
        .mixer_snapshots
        .iter()
        .take(64)
        .map(|m| crate::model::MixerSnapshot {
            name: m.name.chars().take(60).collect(),
            strips: m
                .strips
                .iter()
                .take(ovrtone_engine::MAX_TRACKS)
                .map(|s| {
                    // Read as a track's effects and sends are, with the same care.
                    let as_track = ovrtone_project::TrackDoc {
                        fx: s.fx.clone(),
                        sends: s.sends.clone(),
                        ..Default::default()
                    };
                    let number = |v: f32, lo: f32, hi: f32| if v.is_finite() { v.clamp(lo, hi) } else { 0.0 };
                    crate::model::StripSnapshot {
                        name: s.name.clone(),
                        volume_db: number(s.volume_db, -60.0, 6.0),
                        pan: number(s.pan, -1.0, 1.0),
                        mute: s.mute,
                        solo: s.solo,
                        trim_db: number(s.trim_db, -36.0, 36.0),
                        inserts: inserts_from_doc(&as_track),
                        sends: sends_from_doc(&as_track),
                    }
                })
                .collect(),
        })
        .collect();
    p.spatial_buses = doc
        .spatial_buses
        .iter()
        .take(ovrtone_engine::MAX_SPATIAL_BUSES)
        .map(|b| {
            let f = |v: f32, lo: f32, hi: f32, or: f32| if v.is_finite() { v.clamp(lo, hi) } else { or };
            crate::model::SpatialBusDef {
                fx: inserts_from_docs(&b.fx),
                plugins: b.plugins.iter().map(plugin_from_doc).collect(),
                name: b.name.clone(),
                layout: ovrtone_engine::SpeakerLayout::ALL
                    .into_iter()
                    .find(|l| l.name() == b.layout)
                    .unwrap_or(ovrtone_engine::SpeakerLayout::Surround712),
                place: ovrtone_engine::BusPlace {
                    direction: f(b.direction, -180.0, 180.0, 0.0),
                    elevation: f(b.elevation, -90.0, 90.0, 0.0),
                    spread: f(b.spread, 0.0, 1.0, 1.0),
                    size: f(b.size, 0.0, 1.0, 0.0),
                    centre: f(b.centre, 0.0, 1.0, 1.0),
                    lfe: f(b.lfe, 0.0, 1.0, 1.0),
                    gain: f(b.gain, 0.0, 4.0, 1.0),
                    muted: b.muted,
                },
            }
        })
        .collect();
    p.spatial_channels.lfe_hz = if doc.spatial_lfe_hz.is_finite() {
        doc.spatial_lfe_hz.clamp(0.0, 500.0)
    } else {
        0.0
    };
    for (slot, text) in p.spatial_channels.distance.iter_mut().zip(&doc.spatial_distance) {
        if let Some(d) = ovrtone_engine::Distance::ALL
            .into_iter()
            .find(|d| d.name().eq_ignore_ascii_case(text))
        {
            *slot = d;
        }
    }
    if let Some(bed) = p.spatial_mode.layout() {
        p.spatial_bed = bed;
    }
    if doc.spatial_room >= 0.0 {
        p.spatial_room = doc.spatial_room.min(1.0);
    }
    p.auto_monitor = doc.auto_monitor;
    p.wait_for_sound = doc.wait_for_sound;
    p.trigger_db = doc.trigger_db.clamp(-60.0, -6.0);
    if doc.varispeed.is_finite() {
        p.varispeed = doc.varispeed.clamp(-50.0, 100.0);
    }
    if doc.nudge_beats.is_finite() && doc.nudge_beats > 0.0 {
        p.nudge_beats = doc.nudge_beats;
    }
    p.end_beat = doc.end_beat.filter(|b| b.is_finite() && *b > 0.0);
    p.notes = doc
        .notes
        .chars()
        .take(crate::side_panes::MAX_NOTES)
        .collect();
    p.master_vol = doc.master_vol.clamp(0.0, 2.0);
    p.groups = doc
        .groups
        .iter()
        .take(crate::model::MAX_GROUPS)
        .map(|g| crate::model::MixGroup {
            name: g.name.chars().take(64).collect(),
            volume: g.volume,
            pan: g.pan,
            mute: g.mute,
            solo: g.solo,
        })
        .collect();
    // A group number has to name a group, and a folder or VCA track has none.
    for t in tracks.iter_mut() {
        if t.group.is_some_and(|g| g >= p.groups.len())
            || matches!(t.kind, TrackKind::Folder | TrackKind::Vca)
        {
            t.group = None;
        }
    }
    p.selected_track = doc.selected_track.min(tracks.len().saturating_sub(1));
    // A VCA assignment has to point at a VCA track (never at the track itself), and a VCA
    // track cannot be assigned to another.
    let kinds: Vec<TrackKind> = tracks.iter().map(|t| t.kind).collect();
    for (i, t) in tracks.iter_mut().enumerate() {
        t.vca = t.vca.filter(|&v| {
            kinds.get(v) == Some(&TrackKind::Vca) && v != i && t.kind != TrackKind::Vca
        });
        // A side chain has to come from another track that makes sound.
        for plugin in t.hosted_mut() {
            plugin.sidechain = plugin
                .sidechain
                .filter(|&s| s != i && crate::model::can_key(kinds.get(s).copied()));
        }
        for fx in t.inserts.iter_mut() {
            fx.sidechain = fx
                .sidechain
                .filter(|&s| s != i && crate::model::can_key(kinds.get(s).copied()));
        }
    }
    p.tracks = tracks;
    p.master_strip.inserts = inserts_from_doc(&ovrtone_project::TrackDoc {
        fx: doc.master_fx.clone(),
        ..ovrtone_project::TrackDoc::default()
    });
    p.master_plugins = doc
        .master_plugins
        .iter()
        .filter(|d| !d.id.is_empty())
        .take(ovrtone_engine::MAX_INSERTS)
        .map(plugin_from_doc)
        .collect();
    p.midi_map = midi_map_from_doc(&p, &doc.midi_map);
    // Plugin lanes are saved by where the plugin is, as an instance's number lasts one run.
    for (track, d) in p.tracks.iter_mut().zip(&doc.tracks) {
        resolve_region_plugins(track);
        track.auto.shown = auto_lane_from_doc(track, &d.auto_lane);
        for l in &d.auto_plugins {
            let plugin = if l.slot < 0 {
                track.instrument_plugin.as_ref()
            } else {
                track.plugins.get(l.slot as usize)
            };
            let Some(uid) = plugin.map(|i| i.uid) else {
                continue;
            };
            let target = crate::model::AutoTarget::Plugin {
                uid,
                param: l.param,
            };
            let points = points_from_doc(target, &l.points);
            if !points.is_empty() {
                track.auto.plugin_lanes.push(crate::model::PluginLane {
                    uid,
                    param: l.param,
                    points,
                });
            }
        }
    }
    // A hand-edited file cannot nest tracks deeper than the folders above them allow.
    p.normalize_nesting();
    p.playing = false;
    p.recording = false;
    // The notes of Pattern and Drummer regions are the ones the file holds.
    p.stamp_generated();
    (p, warnings)
}

/// Longest fade a file may ask for, in beats.
const MAX_FADE: f32 = 4096.0;

fn curve_name(c: FadeCurve) -> &'static str {
    match c {
        FadeCurve::Linear => "linear",
        FadeCurve::EqualPower => "equal_power",
        FadeCurve::SCurve => "s_curve",
    }
}

fn curve_from_name(name: &str) -> FadeCurve {
    match name {
        "linear" => FadeCurve::Linear,
        "s_curve" => FadeCurve::SCurve,
        _ => FadeCurve::EqualPower,
    }
}

fn pattern_to_doc(p: &PatternParams) -> PatternDoc {
    PatternDoc {
        steps: p.steps,
        rate: p.rate.name().into(),
        swing: p.swing,
        gate: p.gate,
        rows: p
            .rows
            .iter()
            .map(|r| PatternRowDoc {
                pitch: r.pitch,
                // Trailing steps that are as a new step is are not written.
                steps: trimmed(&r.steps, 0),
                chance: trimmed(&r.chance, 100),
                tie: trimmed(&r.tie, false),
                repeat: trimmed(&r.repeat, 1),
            })
            .collect(),
        direction: p.direction.name().into(),
        seed: p.seed,
    }
}

/// `list` without the values at its end that are `default`.
fn trimmed<T: PartialEq + Copy>(list: &[T], default: T) -> Vec<T> {
    let end = list
        .iter()
        .rposition(|v| *v != default)
        .map_or(0, |i| i + 1);
    list[..end].to_vec()
}

fn drummer_to_doc(d: &DrummerParams) -> DrummerDoc {
    DrummerDoc {
        style: d.style.name().into(),
        complexity: d.complexity,
        loudness: d.loudness,
        swing: d.swing,
        fills: d.fills,
        parts: d.parts,
        seed: d.seed,
    }
}

/// The settings a region document holds for a Pattern or Drummer region, repaired. The key of
/// the notes is set once the whole project is built (`Project::stamp_generated`).
fn generated_from_doc(r: &RegionDoc) -> Option<Generated> {
    if let Some(d) = &r.pattern {
        let mut p = PatternParams {
            steps: d.steps,
            rate: StepRate::from_name(&d.rate),
            swing: d.swing,
            gate: d.gate,
            rows: d
                .rows
                .iter()
                .take(MAX_ROWS)
                .map(|row| PatternRow {
                    pitch: row.pitch,
                    steps: row.steps.iter().copied().take(MAX_STEPS).collect(),
                    chance: row.chance.iter().copied().take(MAX_STEPS).collect(),
                    tie: row.tie.iter().copied().take(MAX_STEPS).collect(),
                    repeat: row.repeat.iter().copied().take(MAX_STEPS).collect(),
                })
                .collect(),
            direction: Direction::from_name(&d.direction),
            seed: d.seed,
        };
        p.tidy();
        return Some(Generated::new(Generator::Pattern(p)));
    }
    r.drummer.as_ref().map(|d| {
        let mut p = DrummerParams {
            style: DrumStyle::from_name(&d.style),
            complexity: d.complexity,
            loudness: d.loudness,
            swing: d.swing,
            fills: d.fills,
            parts: d.parts,
            seed: d.seed,
        };
        p.tidy();
        Generated::new(Generator::Drummer(p))
    })
}

/// How a SMPTE frame rate is written in the project file.
pub fn smpte_rate_name(r: SmpteRate) -> &'static str {
    match r {
        SmpteRate::Fps24 => "24",
        SmpteRate::Fps25 => "25",
        SmpteRate::Fps29_97NonDrop => "29.97",
        SmpteRate::Fps29_97Drop => "29.97df",
        SmpteRate::Fps30 => "30",
    }
}

fn smpte_rate_from_name(name: &str) -> SmpteRate {
    SmpteRate::ALL
        .into_iter()
        .find(|r| smpte_rate_name(*r) == name)
        .unwrap_or(SmpteRate::Fps25)
}

/// A start time that exists at `rate`: hours under 24, minutes and seconds under 60, frames
/// under the rate's count, and no frame label that drop frame skips.
pub fn valid_smpte_start(start: [u8; 4], rate: SmpteRate) -> [u8; 4] {
    let [h, m, s, f] = start;
    let (h, m, s) = (h.min(23), m.min(59), s.min(59));
    let mut f = f.min(rate.nominal_fps() as u8 - 1);
    if rate.is_drop_frame() && s == 0 && m % 10 != 0 && f < 2 {
        f = 2;
    }
    [h, m, s, f]
}

/// A MIDI port name as stored: not too long, and not empty.
fn port_name(name: &Option<String>) -> Option<String> {
    name.as_ref()
        .map(|n| n.chars().take(128).collect::<String>())
        .filter(|n| !n.is_empty())
}

/// Root of all extracted project media.
fn media_root() -> PathBuf {
    std::env::temp_dir().join("ovrtone-media")
}

/// Where this session's audio recordings go until the project is saved.
pub fn recordings_dir() -> PathBuf {
    session_media_dir().join("recordings")
}

/// A track's input from a project file, which can hold any number.
fn input_source(channel: u16, stereo: bool) -> InputSource {
    let channel = channel.min(ovrtone_engine::MAX_INPUT_CHANNELS as u16 - 2);
    if stereo {
        InputSource::Stereo(channel)
    } else {
        InputSource::Mono(channel)
    }
}

/// This process's media folder (`<temp>/ovrtone-media/<pid>`), removed on exit.
fn session_media_dir() -> PathBuf {
    media_root().join(std::process::id().to_string())
}

/// Folder the media of an opened project is extracted to (unique per open).
pub fn media_dir_for(path: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("project");
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    session_media_dir().join(format!("{stem}-{n}"))
}

/// On exit (after the engine has closed its files): delete the media this session extracted.
pub fn remove_session_media() {
    let dir = session_media_dir();
    if dir.exists() {
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            tracing::warn!("cannot remove {}: {e}", dir.display());
        }
    }
}

/// On launch: delete media left by sessions that crashed more than `max_age` ago.
pub fn remove_stale_media(max_age: std::time::Duration) {
    let Ok(entries) = std::fs::read_dir(media_root()) else {
        return;
    };
    for e in entries.flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > max_age);
        if old && e.path() != session_media_dir() {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// Saves `p` to `path` (adds the `.ovrtone` extension if missing). Returns the final path.
/// The document and media a save writes, with the plugins' settings in it.
pub fn doc_for_saving(p: &Project) -> (ProjectDoc, Vec<MediaFile>) {
    let (mut doc, media) = to_doc(p);
    add_plugin_states(p, &mut doc);
    (doc, media)
}

pub fn save_project(p: &Project, path: &Path) -> Result<PathBuf, ovrtone_project::ProjectError> {
    let mut path = path.to_path_buf();
    // A project opened from an old `.prowave` file stays in it until the user says otherwise.
    if !ovrtone_project::is_project_file(&path) {
        path.set_extension(ovrtone_project::EXTENSION);
    }
    let (doc, media) = doc_for_saving(p);
    ovrtone_project::save(&path, &doc, &media)?;
    Ok(path)
}

fn plugin_to_doc(i: &crate::model::PluginInsert) -> ovrtone_project::PluginDoc {
    ovrtone_project::PluginDoc {
        row: i.row,
        id: i.id.clone(),
        name: i.name.clone(),
        vendor: i.vendor.clone(),
        format: ovrtone_host::PluginFormat::of(&i.path).name().into(),
        path: i.path.display().to_string(),
        bypass: i.bypass,
        // Only when saving, as this runs after every gesture: see `save_project`.
        state: String::new(),
        sidechain: i.sidechain.map(|s| s as u32),
        smart_knobs: i.smart_knobs.map_or_else(Vec::new, |knobs| knobs.to_vec()),
        smart_ranges: i
            .smart_ranges
            .map_or_else(Vec::new, |ranges| ranges.to_vec()),
    }
}

fn plugin_from_doc(d: &ovrtone_project::PluginDoc) -> crate::model::PluginInsert {
    crate::model::PluginInsert {
        row: d.row,
        uid: crate::plugin_rack::new_uid(),
        id: d.id.clone(),
        name: d.name.clone(),
        vendor: d.vendor.clone(),
        path: d.path.clone().into(),
        bypass: d.bypass,
        failed: None,
        state: plugin_state_from_doc(&d.state),
        params: std::sync::Arc::default(),
        sidechain: d.sidechain.map(|s| s as usize),
        side_offered: false,
        smart_knobs: smart_knobs_from_doc(&d.smart_knobs),
        smart_ranges: <[(f32, f32); crate::model::SMART_KNOBS]>::try_from(
            d.smart_ranges.as_slice(),
        )
        .ok()
        .filter(|all| all.iter().all(|(lo, hi)| lo.is_finite() && hi.is_finite())),
    }
}

/// The hardware controls for the file. Plugins go by place there, as their uids are for this run;
/// one on a plugin that is gone is not kept.
fn midi_map_to_doc(p: &Project) -> Vec<ovrtone_project::MidiMapDoc> {
    use crate::model::LearnTarget;
    use ovrtone_project::MidiMapDoc;
    p.midi_map
        .iter()
        .filter_map(|m| {
            let doc = |kind: &str, track: Option<usize>, slot: usize, param: u32| MidiMapDoc {
                channel: m.channel,
                controller: m.controller,
                kind: kind.to_string(),
                track: track.map(|t| t as u32),
                slot: slot as u32,
                param,
            };
            match m.target {
                LearnTarget::Volume(t) => Some(doc("volume", Some(t), 0, 0)),
                LearnTarget::Pan(t) => Some(doc("pan", Some(t), 0, 0)),
                LearnTarget::MasterVolume => Some(doc("master", None, 0, 0)),
                LearnTarget::Plugin { uid, param } => {
                    let on_track = p.tracks.iter().enumerate().find_map(|(t, x)| {
                        let slot = x.hosted().position(|i| i.uid == uid)?;
                        Some(doc("plugin", Some(t), slot, param))
                    });
                    on_track.or_else(|| {
                        let slot = p.master_plugins.iter().position(|i| i.uid == uid)?;
                        Some(doc("plugin", None, slot, param))
                    })
                }
            }
        })
        .collect()
}

/// The hardware controls from the file, for a project whose tracks and plugins are in place. What
/// makes no sense in it is left out.
fn midi_map_from_doc(
    p: &Project,
    docs: &[ovrtone_project::MidiMapDoc],
) -> Vec<crate::model::MidiMapping> {
    use crate::model::{LearnTarget, MidiMapping};
    docs.iter()
        .filter_map(|d| {
            let track = d.track.map(|t| t as usize);
            let on_track = track.filter(|t| *t < p.tracks.len());
            let target = match d.kind.as_str() {
                "volume" => LearnTarget::Volume(on_track?),
                "pan" => LearnTarget::Pan(on_track?),
                "master" => LearnTarget::MasterVolume,
                "plugin" => {
                    let slot = d.slot as usize;
                    let uid = match track {
                        Some(_) => p.tracks[on_track?].hosted().nth(slot)?.uid,
                        None => p.master_plugins.get(slot)?.uid,
                    };
                    LearnTarget::Plugin {
                        uid,
                        param: d.param,
                    }
                }
                _ => return None,
            };
            (d.channel < 16 && d.controller < 128).then_some(MidiMapping {
                channel: d.channel,
                controller: d.controller,
                target,
            })
        })
        .collect()
}

/// Smart Controls knobs from the file, with knobs the file has too few of off and any more ignored.
fn smart_knobs_from_doc(knobs: &[Option<u32>]) -> Option<[Option<u32>; crate::model::SMART_KNOBS]> {
    if knobs.is_empty() {
        return None;
    }
    let mut out = [None; crate::model::SMART_KNOBS];
    for (slot, knob) in out.iter_mut().zip(knobs) {
        *slot = *knob;
    }
    Some(out)
}

/// The text of a strip's settings file. The plugins' states must have been collected first.
pub fn strip_to_text(t: &Track) -> Result<String, String> {
    use base64::Engine as _;
    let doc = ovrtone_project::StripDoc {
        fx: t
            .inserts
            .iter()
            .map(|s| ovrtone_project::InsertDoc {
                kind: s.name().into(),
                bypass: s.bypass,
                params: s.params[..s.kind.params().len()].to_vec(),
                sidechain: s.sidechain.map(|v| v as u32),
            })
            .collect(),
        plugins: t
            .plugins
            .iter()
            .map(|i| {
                let mut d = plugin_to_doc(i);
                // Tracks mean nothing in another project.
                d.sidechain = None;
                if let Some(state) = &i.state {
                    d.state = base64::engine::general_purpose::STANDARD.encode(state.as_slice());
                }
                d
            })
            .collect(),
    };
    serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())
}

/// Puts a strip's settings file on a track, in place of its effects and plugins.
pub fn strip_from_text(t: &mut Track, text: &str) -> Result<(), String> {
    let doc: ovrtone_project::StripDoc = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let track = ovrtone_project::TrackDoc {
        fx: doc.fx,
        ..Default::default()
    };
    t.inserts = inserts_from_doc(&track);
    t.plugins = doc.plugins.iter().map(plugin_from_doc).collect();
    Ok(())
}

/// Writes the plugins' settings into the document that goes in the file.
fn add_plugin_states(p: &Project, doc: &mut ProjectDoc) {
    use base64::Engine as _;
    let inserts = p
        .tracks
        .iter()
        .flat_map(|t| t.hosted())
        .chain(&p.master_plugins);
    let docs = doc
        .tracks
        .iter_mut()
        .flat_map(|t| t.instrument_plugin.iter_mut().chain(&mut t.plugins))
        .chain(&mut doc.master_plugins);
    for (insert, plugin_doc) in inserts.zip(docs) {
        if let Some(state) = &insert.state {
            plugin_doc.state = base64::engine::general_purpose::STANDARD.encode(state.as_slice());
        }
    }
}

/// A plugin's settings from the file. Text that is not base64 counts as none: the plugin starts
/// from its defaults rather than the project failing to open.
fn plugin_state_from_doc(text: &str) -> Option<std::sync::Arc<Vec<u8>>> {
    use base64::Engine as _;
    if text.is_empty() {
        return None;
    }
    base64::engine::general_purpose::STANDARD
        .decode(text)
        .ok()
        .map(std::sync::Arc::new)
}

/// Opens a project file; returns the project and warnings about media that could not be read.
pub fn open_project(
    path: &Path,
    ctx: &egui::Context,
) -> Result<(Project, Vec<String>), ovrtone_project::ProjectError> {
    let media_dir = media_dir_for(path);
    let doc = ovrtone_project::load(path, &media_dir)?;
    let (mut p, warnings) = from_doc(&doc, &media_dir, ctx);
    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
        p.name = stem.to_string();
    }
    Ok((p, warnings))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn region_automation_survives_a_save_with_its_plugin_found_again() {
        use crate::model::{AutoPoint, AutoTarget, RegionLane};
        let mut t = Track::new("t", TrackKind::Audio);
        let mut region = Project::empty();
        let n = region.add_track(TrackKind::Instrument);
        region.add_midi_region(n, 1.0);
        let mut r = region.tracks[n].regions.remove(0);
        let plugin = crate::model::PluginInsert {
            row: 0,
            uid: 77,
            id: "x".into(),
            name: "x".into(),
            vendor: String::new(),
            path: "x.clap".into(),
            bypass: false,
            failed: None,
            state: None,
            params: std::sync::Arc::default(),
            sidechain: None,
            side_offered: false,
            smart_knobs: None,
            smart_ranges: None,
        };
        t.plugins.push(plugin);
        let points = vec![AutoPoint {
            beat: 1.0,
            value: 0.5,
            curve: 0.0,
        }];
        for target in [
            AutoTarget::Volume,
            AutoTarget::Plugin { uid: 77, param: 9 },
            AutoTarget::Plugin {
                uid: 12345,
                param: 9,
            },
        ] {
            r.auto.push(RegionLane {
                target,
                points: points.clone(),
            });
        }
        let doc = RegionDoc {
            auto: region_lanes_to_doc(&t, &r),
            ..RegionDoc::default()
        };
        assert_eq!(
            doc.auto.len(),
            2,
            "a lane of a plugin that is gone is not kept"
        );
        r.auto = region_lanes_from_doc(&doc);
        t.regions.push(r);
        t.plugins[0].uid = 88;
        resolve_region_plugins(&mut t);
        let lanes = &t.regions[0].auto;
        assert_eq!(lanes[0].target, AutoTarget::Volume);
        assert_eq!(lanes[1].target, AutoTarget::Plugin { uid: 88, param: 9 });
        assert!((lanes[1].points[0].beat - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_side_chain_is_kept_in_the_file_and_one_that_makes_no_sense_is_dropped() {
        let mut d = ovrtone_project::PluginDoc {
            sidechain: Some(2),
            ..Default::default()
        };
        assert_eq!(plugin_from_doc(&d).sidechain, Some(2));
        let insert = plugin_from_doc(&d);
        assert_eq!(plugin_to_doc(&insert).sidechain, Some(2));
        d.sidechain = None;
        assert_eq!(plugin_from_doc(&d).sidechain, None);
        // A file from before there were side chains reads as none, and none stays out of files.
        let old: ovrtone_project::PluginDoc = serde_json::from_str(r#"{"id":"x"}"#).unwrap();
        assert_eq!(old.sidechain, None);
        assert!(!serde_json::to_string(&old).unwrap().contains("sidechain"));

        let mut p = Project::empty();
        let a = p.add_track(TrackKind::Audio);
        let b = p.add_track(TrackKind::Vca);
        let plugin = |sidechain| crate::model::PluginInsert {
            sidechain,
            ..plugin_from_doc(&ovrtone_project::PluginDoc {
                id: "x".into(),
                ..Default::default()
            })
        };
        p.tracks[a].plugins = vec![
            plugin(Some(a)),
            plugin(Some(b)),
            plugin(Some(99)),
            plugin(Some(0)),
        ];
        let (doc, _) = doc_for_saving(&p);
        let (q, _) = from_doc(&doc, std::path::Path::new("."), &egui::Context::default());
        let kept: Vec<_> = q.tracks[a].plugins.iter().map(|x| x.sidechain).collect();
        assert_eq!(kept, [None, None, None, Some(0)]);
    }

    #[test]
    fn a_channel_strip_file_brings_back_effects_and_plugins_with_settings() {
        let mut p = Project::empty();
        let a = p.add_track(TrackKind::Audio);
        let b = p.add_track(TrackKind::Audio);
        p.tracks[a].inserts = crate::model::inserts_from_names(&["Channel EQ"]);
        p.tracks[a].plugins.push(crate::model::PluginInsert {
            row: 3,
            uid: 77,
            id: "x.y".into(),
            name: "Y".into(),
            vendor: String::new(),
            path: "y.clap".into(),
            bypass: true,
            failed: None,
            state: Some(std::sync::Arc::new(vec![1, 2, 3])),
            params: std::sync::Arc::default(),
            sidechain: Some(a),
            side_offered: false,
            smart_knobs: Some([Some(7); crate::model::SMART_KNOBS]),
            smart_ranges: Some([(0.25, 0.75); crate::model::SMART_KNOBS]),
        });
        let text = strip_to_text(&p.tracks[a]).unwrap();
        strip_from_text(&mut p.tracks[b], &text).unwrap();
        assert_eq!(p.tracks[b].inserts.len(), p.tracks[a].inserts.len());
        let got = &p.tracks[b].plugins[0];
        assert_ne!(got.uid, 77, "a new instance");
        assert_eq!((got.row, got.bypass, got.id.as_str()), (3, true, "x.y"));
        assert_eq!(got.state.as_deref(), Some(&vec![1, 2, 3]));
        let before = p.tracks[b].plugins.len();
        assert!(strip_from_text(&mut p.tracks[b], "not json").is_err());
        assert_eq!(
            p.tracks[b].plugins.len(),
            before,
            "a bad file changes nothing"
        );
    }

    /// 0.5 s, mono, 16-bit, 44.1 kHz silence.
    pub(crate) fn write_wav(path: &Path) {
        let n = 22_050u32;
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + n * 2).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&44_100u32.to_le_bytes());
        b.extend_from_slice(&88_200u32.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(n * 2).to_le_bytes());
        b.resize(b.len() + (n * 2) as usize, 0);
        std::fs::write(path, b).unwrap();
    }

    #[test]
    fn vca_assignments_are_saved_and_hand_edits_cannot_break_them() {
        let mut p = Project::empty();
        p.tracks.clear();
        p.tracks.push(Track::new("Inst", TrackKind::Instrument));
        p.tracks.push(Track::new("VCA 1", TrackKind::Vca));
        p.tracks.push(Track::new("Inst 2", TrackKind::Instrument));
        assert!(p.assign_vca(0, Some(1)));
        p.tracks[1].volume_db = -4.0;
        let (doc, media) = to_doc(&p);
        assert_eq!(doc.tracks[0].vca, Some(1));
        let _ = media;
        let (q, _) = from_doc(&doc, Path::new("."), &egui::Context::default());
        assert_eq!(q.tracks[1].kind, TrackKind::Vca);
        assert_eq!(q.tracks[0].vca, Some(1));
        assert_eq!(q.tracks[1].volume_db, -4.0);
        // Pointing at a track that is not a VCA, at itself, at nothing, or a VCA at a VCA.
        let mut bad = doc.clone();
        bad.tracks[0].vca = Some(2);
        bad.tracks[2].vca = Some(2);
        bad.tracks[1].vca = Some(1);
        let (r, _) = from_doc(&bad, Path::new("."), &egui::Context::default());
        assert!(r.tracks.iter().all(|t| t.vca.is_none()));
        bad.tracks[0].vca = Some(99);
        let (r, _) = from_doc(&bad, Path::new("."), &egui::Context::default());
        assert_eq!(r.tracks[0].vca, None);
    }

    #[test]
    fn audio_tracks_keep_their_input() {
        let mut p = Project::empty();
        p.tracks.clear();
        p.tracks.push(Track::new("Vox", TrackKind::Audio));
        p.tracks.push(Track::new("Guitar", TrackKind::Audio));
        p.tracks[0].input = InputSource::Mono(3);
        p.tracks[1].input = InputSource::Stereo(2);
        let (mut doc, _) = to_doc(&p);
        let (q, _) = from_doc(&doc, Path::new("."), &egui::Context::default());
        assert_eq!(q.tracks[0].input, InputSource::Mono(3));
        assert_eq!(q.tracks[1].input, InputSource::Stereo(2));
        // A damaged or hand-edited file cannot ask for a channel that cannot exist.
        doc.tracks[1].input = u16::MAX;
        let (r, _) = from_doc(&doc, Path::new("."), &egui::Context::default());
        let top = ovrtone_engine::MAX_INPUT_CHANNELS as u16 - 2;
        assert_eq!(r.tracks[1].input, InputSource::Stereo(top));
    }

    #[test]
    fn external_midi_tracks_keep_their_port_and_channel() {
        let mut p = Project::empty();
        p.tracks.clear();
        let mut t = Track::new("Synth", TrackKind::ExternalMidi);
        t.midi_port = Some("USB MIDI Interface".into());
        t.midi_channel = 10;
        p.tracks.push(t);
        p.tracks.push(Track::new("Piano", TrackKind::Instrument));
        let (doc, _) = to_doc(&p);
        let (q, _) = from_doc(&doc, Path::new("."), &egui::Context::default());
        assert_eq!(q.tracks[0].kind, TrackKind::ExternalMidi);
        assert_eq!(q.tracks[0].midi_port.as_deref(), Some("USB MIDI Interface"));
        assert_eq!(q.tracks[0].midi_channel, 10);
        assert!(q.tracks[0].kind.has_regions());
        assert_eq!(
            (q.tracks[1].midi_port.clone(), q.tracks[1].midi_channel),
            (None, 1)
        );
        // A hand-edited file cannot give a channel the MIDI cable does not have.
        let mut bad = doc.clone();
        bad.tracks[0].midi_channel = 99;
        bad.tracks[1].midi_channel = 0;
        bad.tracks[0].midi_port = Some(String::new());
        let (r, _) = from_doc(&bad, Path::new("."), &egui::Context::default());
        assert_eq!(
            (r.tracks[0].midi_channel, r.tracks[1].midi_channel),
            (16, 1)
        );
        assert_eq!(r.tracks[0].midi_port, None);
    }

    #[test]
    fn hand_edited_signature_changes_are_sorted_and_cleaned() {
        let mut doc = to_doc(&Project::empty()).0;
        doc.sig_changes = [(9, 7, 8), (1, 5, 4), (3, 0, 3), (3, 6, 8), (100_000, 2, 4)]
            .into_iter()
            .map(|(bar, num, den)| SigChangeDoc { bar, num, den })
            .collect();
        let (p, _) = from_doc(&doc, Path::new("."), &egui::Context::default());
        let got: Vec<(u32, u8, u8)> = p
            .sig_changes
            .iter()
            .map(|c| (c.bar, c.num, c.den))
            .collect();
        // Bar 1 and bars past the limit are dropped, a bad signature is repaired, and two
        // changes on one bar leave one.
        assert_eq!(got, vec![(3, 1, 4), (9, 7, 8)]);
    }

    #[test]
    fn hand_edited_tempo_changes_are_sorted_and_cleaned() {
        let mut doc = to_doc(&Project::empty()).0;
        doc.tempo_changes = [(64.0, 90.0), (0.0, 50.0), (16.0, f32::NAN), (8.0, 2000.0)]
            .into_iter()
            .map(|(beat, bpm)| TempoChangeDoc {
                beat,
                bpm,
                ramp: true,
            })
            .collect();
        let (p, _) = from_doc(&doc, Path::new("."), &egui::Context::default());
        let got: Vec<(f64, f32)> = p.tempo_changes.iter().map(|t| (t.beat, t.bpm)).collect();
        assert_eq!(got, vec![(8.0, 990.0), (64.0, 90.0)]);
        assert!(
            !p.tempo_changes.last().is_some_and(|t| t.ramp),
            "the last change has nothing to ramp to"
        );
    }

    #[test]
    fn pattern_and_drummer_regions_are_saved_with_their_settings_and_notes() {
        use crate::model::Generator;
        use crate::pattern::{parts, DrumStyle, StepRate};
        let mut p = Project::empty();
        let drums = p.add_track(TrackKind::Drummer);
        let d = p.add_generated_region(drums, true, 2.0).unwrap();
        let q = p.add_generated_region(drums, false, 8.0).unwrap();
        if let Some(Generator::Drummer(params)) = p.tracks[drums].regions[d]
            .generated
            .as_mut()
            .map(|g| &mut g.source)
        {
            params.style = DrumStyle::Funk;
            params.complexity = 77;
            params.seed = 12345;
            params.parts = parts::ALL & !parts::TOMS;
        }
        p.regenerate(drums, d);
        if let Some(Generator::Pattern(pattern)) = p.tracks[drums].regions[q]
            .generated
            .as_mut()
            .map(|g| &mut g.source)
        {
            pattern.steps = 12;
            pattern.rate = StepRate::EighthTriplet;
            pattern.rows[0].steps[11] = 64;
            pattern.rows[0].chance[3] = 40;
            pattern.rows[0].tie[5] = true;
            pattern.rows[1].repeat[2] = 3;
            pattern.direction = crate::pattern::Direction::PingPong;
            pattern.seed = 99;
        }
        p.regenerate(drums, q);

        let (doc, _) = to_doc(&p);
        let r = &doc.tracks[drums].regions;
        assert_eq!(
            r[d].drummer.as_ref().map(|x| x.style.as_str()),
            Some("funk")
        );
        assert!(r[d].pattern.is_none() && r[q].drummer.is_none());
        assert_eq!(r[q].pattern.as_ref().map(|x| x.rate.as_str()), Some("1/8T"));
        let (back, _) = from_doc(&doc, Path::new("."), &egui::Context::default());
        for ri in [d, q] {
            let (a, b) = (
                &p.tracks[drums].regions[ri],
                &back.tracks[drums].regions[ri],
            );
            assert_eq!(
                a.generated.as_ref().map(|g| &g.source),
                b.generated.as_ref().map(|g| &g.source)
            );
            assert_eq!(a.notes, b.notes, "the notes are the ones that were saved");
        }
        // Opening does not remake the notes, and a different length does.
        let mut back = back;
        assert!(!back.refresh_generated());
        back.tracks[drums].regions[d].len_bars = 6.0;
        assert!(back.refresh_generated());
        // Hand-edited files: nothing breaks.
        let mut bad = doc.clone();
        bad.tracks[drums].regions[q].pattern = Some(ovrtone_project::PatternDoc {
            steps: 9999,
            rate: "weird".into(),
            swing: f32::NAN,
            gate: -4.0,
            direction: "sideways".into(),
            seed: 3,
            rows: (0..40)
                .map(|_| ovrtone_project::PatternRowDoc {
                    pitch: 200,
                    steps: vec![255; 500],
                    chance: vec![255; 500],
                    tie: vec![true; 500],
                    repeat: vec![0; 500],
                })
                .collect(),
        });
        bad.tracks[drums].regions[d].drummer = Some(ovrtone_project::DrummerDoc {
            style: "polka".into(),
            complexity: 255,
            parts: 255,
            ..Default::default()
        });
        let (fixed, _) = from_doc(&bad, Path::new("."), &egui::Context::default());
        match fixed.tracks[drums].regions[q]
            .generated
            .as_ref()
            .map(|g| &g.source)
        {
            Some(Generator::Pattern(x)) => {
                assert_eq!((x.steps, x.rows.len()), (64, 16));
                assert_eq!(x.rate, StepRate::Sixteenth);
                assert!(x.rows.iter().all(|r| r.pitch == 127 && r.steps.len() == 64));
                assert!(x.rows.iter().all(
                    |r| r.chance.iter().all(|c| *c == 100) && r.repeat.iter().all(|c| *c == 1)
                ));
                assert_eq!(x.direction, crate::pattern::Direction::Forward);
            }
            other => panic!("{other:?}"),
        }
        match fixed.tracks[drums].regions[d]
            .generated
            .as_ref()
            .map(|g| &g.source)
        {
            Some(Generator::Drummer(x)) => {
                assert_eq!((x.complexity, x.parts), (100, 63));
                assert_eq!(x.style, DrumStyle::Rock);
            }
            other => panic!("{other:?}"),
        }
        // A file from before these regions has neither.
        assert!(RegionDoc::default().pattern.is_none() && RegionDoc::default().drummer.is_none());
    }

    #[test]
    fn midi_fx_are_saved_in_order_and_unknown_ones_are_dropped() {
        use crate::midi_fx::{ArpPattern, FxSlot, MidiFx};
        let mut p = Project::demo();
        p.tracks[0].midi_fx = vec![
            FxSlot {
                fx: MidiFx::Arpeggiator {
                    rate: crate::pattern::StepRate::Eighth,
                    pattern: ArpPattern::UpDown,
                    octaves: 2,
                    gate: 60,
                },
                on: true,
            },
            FxSlot {
                fx: MidiFx::Transposer { semitones: -5 },
                on: false,
            },
        ];
        let (mut doc, _) = to_doc(&p);
        assert_eq!(doc.tracks[0].midi_fx[0].kind, "arpeggiator");
        let (q, _) = from_doc(&doc, Path::new("."), &egui::Context::default());
        assert_eq!(q.tracks[0].midi_fx, p.tracks[0].midi_fx);
        assert!(q.tracks[1].midi_fx.is_empty(), "older files have none");
        doc.tracks[0].midi_fx.insert(
            1,
            ovrtone_project::MidiFxDoc {
                kind: "from the future".into(),
                ..Default::default()
            },
        );
        let (r, _) = from_doc(&doc, Path::new("."), &egui::Context::default());
        assert_eq!(r.tracks[0].midi_fx, p.tracks[0].midi_fx);
    }

    #[test]
    fn tempo_curves_and_smpte_settings_are_saved() {
        let mut p = Project::empty();
        p.tempo = 100.0;
        p.add_tempo_change(16.0, 140.0);
        p.add_tempo_change(32.0, 90.0);
        p.set_tempo_ramp(0, true);
        p.set_tempo_ramp(1, true);
        let (doc, _) = to_doc(&p);
        let (q, _) = from_doc(&doc, Path::new("."), &egui::Context::default());
        assert!(q.tempo_ramp && q.tempo_changes[0].ramp && !q.tempo_changes[1].ramp);
        assert_eq!(q.tempo_points(), p.tempo_points());
        assert_eq!(q.tempo_at(8.0), 120.0, "halfway up the first curve");
        // A file from before tempo curves has none.
        let old = ProjectDoc::default();
        assert!(!old.tempo_ramp);

        // SMPTE and MIDI Time Code settings.
        p.smpte_rate = SmpteRate::Fps29_97Drop;
        p.smpte_start = [10, 5, 3, 7];
        p.mtc_out = Some("Out Port".into());
        p.mtc_in = Some("In Port".into());
        let (doc, _) = to_doc(&p);
        assert_eq!(doc.smpte_rate, "29.97df");
        let (q, _) = from_doc(&doc, Path::new("."), &egui::Context::default());
        assert_eq!(q.smpte_rate, SmpteRate::Fps29_97Drop);
        assert_eq!(q.smpte_start, [10, 5, 3, 7]);
        assert_eq!(q.mtc_out.as_deref(), Some("Out Port"));
        assert_eq!(q.mtc_in.as_deref(), Some("In Port"));
        for rate in SmpteRate::ALL {
            assert_eq!(smpte_rate_from_name(smpte_rate_name(rate)), rate);
        }
        // Hand-edited values are repaired: a rate that does not exist, a start that does not
        // exist, a drop-frame label that is skipped.
        let mut bad = doc.clone();
        bad.smpte_rate = "48".into();
        bad.smpte_start = [99, 99, 99, 99];
        bad.mtc_out = Some(String::new());
        let (q, _) = from_doc(&bad, Path::new("."), &egui::Context::default());
        assert_eq!(q.smpte_rate, SmpteRate::Fps25);
        assert_eq!(q.smpte_start, [23, 59, 59, 24]);
        assert_eq!(q.mtc_out, None);
        assert_eq!(
            valid_smpte_start([0, 1, 0, 0], SmpteRate::Fps29_97Drop),
            [0, 1, 0, 2]
        );
        assert_eq!(
            valid_smpte_start([0, 10, 0, 0], SmpteRate::Fps29_97Drop),
            [0, 10, 0, 0]
        );
    }

    #[test]
    fn plugins_and_their_settings_are_saved_but_only_when_saving() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut p = Project::empty();
        p.tracks.push(Track::new("Vox", TrackKind::Audio));
        let info = ovrtone_host::PluginInfo {
            id: "com.maker.eq".into(),
            name: "EQ".into(),
            vendor: "Maker".into(),
            version: "1".into(),
            format: ovrtone_host::PluginFormat::Clap,
            path: "C:/Plugins/eq.clap".into(),
            is_instrument: false,
        };
        let t = p.tracks.len() - 1;
        crate::plugin_rack::add_plugin(&mut p, t, &info).unwrap();
        crate::plugin_rack::add_plugin(&mut p, t, &info).unwrap();
        p.tracks.push(Track::new("Keys", TrackKind::Instrument));
        let synth = ovrtone_host::PluginInfo {
            id: "com.maker.synth".into(),
            is_instrument: true,
            ..info.clone()
        };
        crate::plugin_rack::add_plugin(&mut p, t + 1, &synth).unwrap();
        p.tracks[t + 1].instrument_plugin.as_mut().unwrap().state =
            Some(std::sync::Arc::new(vec![7, 8, 9]));
        let bytes: Vec<u8> = (0..=255).collect();
        p.tracks[t].plugins[0].state = Some(std::sync::Arc::new(bytes.clone()));
        p.tracks[t].plugins[1].failed = Some("it was gone".into());
        p.tracks[t].plugins[1].bypass = true;
        assert!(
            to_doc(&p).0.tracks[t].plugins[0].state.is_empty(),
            "not on every gesture, where it would cost time and look like an edit"
        );
        let path = save_project(&p, &dir.path().join("with plugins")).unwrap();
        let (back, _) = open_project(&path, &ctx).unwrap();
        let plugins = &back.tracks[t].plugins;
        assert_eq!(plugins.len(), 2);
        assert_eq!(plugins[0].id, "com.maker.eq");
        assert_eq!(
            plugins[0].path,
            std::path::PathBuf::from("C:/Plugins/eq.clap")
        );
        assert_eq!(plugins[0].state.as_deref(), Some(&bytes));
        assert_eq!(plugins[1].state, None);
        assert_eq!(plugins[1].failed, None, "tried again after opening");
        assert!(plugins[1].bypass && !plugins[0].bypass);
        assert_ne!(plugins[0].uid, plugins[1].uid);
        assert_eq!(plugin_state_from_doc("not base64!"), None);
        let synth = back.tracks[t + 1].instrument_plugin.as_ref().unwrap();
        assert_eq!(synth.id, "com.maker.synth");
        assert_eq!(synth.state.as_deref(), Some(&vec![7, 8, 9]));
        assert!(back.tracks[t].instrument_plugin.is_none());
    }
    #[test]
    fn flex_edits_and_the_groove_track_are_saved_and_made_again() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut p = Project::empty();
        let wav = dir.path().join("take.wav");
        write_wav(&wav);
        crate::import::import_audio(&mut p, &wav, &ctx).unwrap();
        let t = p.tracks.len() - 1;
        crate::flex_edit::set_mode(&mut p, t, 0, ovrtone_engine::FlexMode::Slicing);
        {
            let f = p.tracks[t].regions[0]
                .audio
                .as_mut()
                .unwrap()
                .flex
                .as_mut()
                .unwrap();
            f.markers = crate::flex_edit::markers_from([(0.1, 0.15), (0.3, 0.32)]);
            f.pitch = vec![ovrtone_engine::PitchShift {
                start: 0.0,
                end: 0.2,
                semitones: -1.5,
                ..Default::default()
            }];
            // The region plays a made file that is not the recording.
            f.rendered = 7;
        }
        p.tracks[t].regions[0].audio.as_mut().unwrap().file.path = dir.path().join("made.wav");
        crate::groove::set_groove_track(&mut p, Some(t));
        p.tracks[0].params.follow_groove = true;
        let path = save_project(&p, &dir.path().join("flex")).unwrap();
        let (q, warnings) = open_project(&path, &ctx).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let a = q.tracks[t].regions[0].audio.as_ref().unwrap();
        let f = a.flex.as_ref().unwrap();
        assert_eq!(f.mode, ovrtone_engine::FlexMode::Slicing);
        assert_eq!(f.markers.len(), 2);
        assert_eq!((f.markers[1].src, f.markers[1].dst), (0.3, 0.32));
        assert_eq!(f.pitch.len(), 1);
        assert_eq!(f.pitch[0].semitones, -1.5);
        // The recording was saved (not the made file), and the sound is made again.
        assert_eq!(a.file.path, f.original.path);
        assert_eq!(f.rendered, 0);
        assert!(q.tracks[t].params.groove);
        assert!(q.tracks[0].params.follow_groove);
    }

    #[test]
    fn effects_sends_and_automation_are_saved_and_damaged_ones_are_tamed() {
        use crate::model::{AutoMode, AutoTarget, Insert};
        use ovrtone_engine::FxKind;
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut p = Project::demo();
        let mut eq = Insert::new(FxKind::ChannelEq);
        eq.params[1] = 7.5;
        let mut delay = Insert::new(FxKind::Delay);
        delay.bypass = true;
        delay.params[0] = 250.0;
        p.tracks[1].inserts = vec![eq, delay];
        p.tracks[1].sends = vec![
            SendSlot {
                bus: Some(2),
                level: 0.6,
                pre: true,
            },
            SendSlot {
                bus: Some(5),
                level: 0.2,
                pre: false,
            },
        ];
        p.tracks[1].auto.mode = AutoMode::Touch;
        p.tracks[1].auto.set_point(AutoTarget::Volume, 4.0, -12.0);
        p.tracks[1].auto.set_point(AutoTarget::Volume, 1.0, -3.0);
        p.tracks[1].auto.set_point(AutoTarget::Pan, 2.0, 0.5);
        p.tracks[1].auto.volume[0].curve = -0.4;
        let delay_time = AutoTarget::Fx { slot: 1, param: 0 };
        p.tracks[1].auto.set_point(delay_time, 2.0, 0.25);
        p.tracks[1].auto.set_point(delay_time, 6.0, 0.75);
        p.tracks[1].auto.fx[0].points[1].curve = 0.5;
        let g = p.add_group("Keys").unwrap();
        p.groups[g].pan = false;
        assert!(p.assign_group(1, Some(g)));
        assert!(p.assign_group(2, Some(g)));
        let path = save_project(&p, &dir.path().join("fx")).unwrap();
        let (q, warnings) = open_project(&path, &ctx).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let t = &q.tracks[1];
        assert_eq!(t.inserts, p.tracks[1].inserts);
        assert_eq!(t.sends, p.tracks[1].sends);
        assert_eq!(t.auto.mode, AutoMode::Touch);
        assert_eq!(t.auto.volume.len(), 2);
        assert_eq!(
            (
                t.auto.volume[0].beat,
                t.auto.volume[0].value,
                t.auto.volume[0].curve
            ),
            (1.0, -3.0, -0.4)
        );
        assert_eq!(t.auto.volume[1], crate::model::AutoPoint::new(4.0, -12.0));
        assert_eq!(t.auto.pan, [crate::model::AutoPoint::new(2.0, 0.5)]);
        assert_eq!(t.auto.fx, p.tracks[1].auto.fx);
        assert_eq!(t.auto.lane(delay_time)[1].curve, 0.5);
        assert_eq!(q.groups, p.groups);
        assert_eq!(
            (q.tracks[1].group, q.tracks[2].group, q.tracks[0].group),
            (Some(g), Some(g), None)
        );
        // Runtime state is not saved.
        assert_eq!(t.auto.held, None);

        // Files from before: names only and one send.
        let mut doc = to_doc(&p).0;
        doc.tracks[1].fx.clear();
        doc.tracks[1].sends.clear();
        doc.tracks[1].send_bus = Some(4);
        doc.tracks[1].send_level = 0.9;
        let old = from_doc(&doc, dir.path(), &ctx).0;
        assert_eq!(
            old.tracks[1].inserts.len(),
            2,
            "{:?}",
            doc.tracks[1].inserts
        );
        assert_eq!(old.tracks[1].inserts[0], Insert::new(FxKind::ChannelEq));
        assert_eq!(old.tracks[1].sends.len(), 1);
        assert_eq!(old.tracks[1].sends[0].bus, Some(4));

        // Damaged: unknown effects, wild numbers, too many of everything.
        let mut doc = to_doc(&p).0;
        let wild = ovrtone_project::InsertDoc { sidechain: None,
            kind: "Delay".into(),
            bypass: false,
            params: vec![f32::NAN, 1e30, -5.0],
        };
        doc.tracks[1].fx = vec![
            ovrtone_project::InsertDoc {
                kind: "Mystery Plug-in".into(),
                ..Default::default()
            },
            wild,
        ];
        doc.tracks[1]
            .fx
            .extend((0..40).map(|_| ovrtone_project::InsertDoc {
                kind: "Chorus".into(),
                ..Default::default()
            }));
        doc.tracks[1].sends = (0..30)
            .map(|k| ovrtone_project::SendDoc {
                bus: if k % 2 == 0 { 2000 } else { k },
                level: 9.0,
                pre: false,
            })
            .collect();
        doc.tracks[1].auto_mode = "Nonsense".into();
        doc.tracks[1].auto_volume = vec![
            [f64::NAN, 1.0, 0.0],
            [1.0, 1e9, 9.0],
            [-4.0, 0.0, 0.0],
            [2.0, -1e9, f64::NAN],
            [2.0, 3.0, 0.0],
        ];
        doc.tracks[1].auto_fx = (0..300)
            .map(|k| ovrtone_project::AutoFxDoc {
                slot: (k % 40) as u8,
                param: (k % 90) as u8,
                points: vec![[1.0, 7.0, 0.0]],
            })
            .collect();
        doc.groups = (0..100)
            .map(|k| ovrtone_project::GroupDoc {
                name: "g".repeat(200 + k),
                ..Default::default()
            })
            .collect();
        doc.tracks[1].group = Some(500);
        doc.tracks[2].group = Some(3);
        let q = from_doc(&doc, dir.path(), &ctx).0;
        let t = &q.tracks[1];
        assert!(t.inserts.len() <= ovrtone_engine::MAX_INSERTS);
        assert_eq!(
            t.inserts[0].kind,
            FxKind::Delay,
            "the unknown one is left out"
        );
        for (v, info) in t.inserts[0].params.iter().zip(t.inserts[0].kind.params()) {
            assert!((info.min..=info.max).contains(v), "{v}");
        }
        assert!(t.sends.len() <= crate::model::MAX_SENDS);
        assert!(t.sends.iter().all(|s| s
            .bus
            .is_some_and(|b| (1..=ovrtone_engine::MAX_BUSES as u16).contains(&b))));
        assert!(t.sends.iter().all(|s| s.level <= 1.0));
        assert_eq!(t.auto.mode, AutoMode::default());
        assert_eq!(
            t.auto.volume,
            [
                crate::model::AutoPoint {
                    beat: 1.0,
                    value: 6.0,
                    curve: 1.0
                },
                crate::model::AutoPoint::new(2.0, -60.0)
            ]
        );
        assert!(t.auto.fx.len() <= 128);
        for l in &t.auto.fx {
            assert!(usize::from(l.slot) < ovrtone_engine::MAX_INSERTS);
            assert!(usize::from(l.param) < ovrtone_engine::MAX_FX_PARAMS);
            assert!(l.points.iter().all(|p| p.value == 1.0));
        }
        assert_eq!(q.groups.len(), crate::model::MAX_GROUPS);
        assert!(q.groups.iter().all(|g| g.name.chars().count() <= 64));
        assert_eq!(q.tracks[1].group, None, "no such group");
        assert_eq!(q.tracks[2].group, Some(3));
    }

    #[test]
    fn wild_flex_data_in_a_file_is_tamed() {
        let original = AudioFile {
            path: "x.wav".into(),
            sample_rate: 44_100,
            frames: 1,
            channels: 2,
        };
        let doc = ovrtone_project::FlexDoc {
            mode: "Nonsense".into(),
            markers: vec![
                [f64::NAN, 1.0],
                [1.0, 1e300],
                [2.0, 1.0],
                [1.0, 2.0],
                [-1.0, 3.0],
            ],
            pitch: vec![
                ovrtone_project::FlexPitchDoc {
                    start: 0.0,
                    end: 1.0,
                    semitones: 500.0,
                    ..Default::default()
                },
                ovrtone_project::FlexPitchDoc {
                    start: 2.0,
                    end: 1.0,
                    semitones: 1.0,
                    ..Default::default()
                },
            ],
        };
        let f = flex_from_doc(&doc, original);
        assert_eq!(f.mode, ovrtone_engine::FlexMode::default());
        assert_eq!(f.markers.len(), 1);
        assert_eq!(f.pitch.len(), 1);
        assert_eq!(f.pitch[0].semitones, 24.0);
    }

    #[test]
    fn save_and_open_round_trip_the_demo_with_edits() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut p = Project::demo();
        p.tempo = 101.0;
        p.add_tempo_change(32.0, 140.0);
        let m = p.add_marker(16.0);
        p.markers[m].name = "Chorus".into();
        p.set_signature_from(9, (3, 4));
        p.set_signature_from(17, (7, 8));
        p.cycle = true;
        p.tracks[0].regions[0].params.transpose = 5;
        p.tracks[4].regions[0].params.gain_db = -3.5;
        p.tracks[4].regions[0].params.fade_in = 2.0;
        p.tracks[4].regions[0].params.fade_curve = FadeCurve::SCurve;
        p.tracks[4].regions[0].params.reverse = true;
        p.tracks[4].regions[0].params.crossfade = true;
        p.tracks[0].params.key_limit = (40, 90);
        p.tracks[1].category = Some("Piano");
        p.tracks[2].sends[0].pre = true;
        p.tracks[3].height = 2.5;
        let wav = dir.path().join("loop.wav");
        write_wav(&wav);
        crate::import::import_audio(&mut p, &wav, &ctx).unwrap();
        // The same file used twice is embedded once.
        let dup = p.tracks.last().unwrap().regions[0].clone();
        p.tracks.last_mut().unwrap().regions.push(Region {
            start_bar: 9.0,
            ..dup
        });

        let file = save_project(&p, &dir.path().join("song")).unwrap();
        assert_eq!(file.extension().unwrap(), "ovrtone");
        assert_eq!(to_doc(&p).1.len(), 1);

        let (q, warnings) = open_project(&file, &ctx).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(q.name, "song");
        assert_eq!((q.tempo, q.cycle), (101.0, true));
        assert_eq!(q.tempo_changes, p.tempo_changes, "the Tempo track is saved");
        assert_eq!(q.markers, p.markers, "and the Marker track");
        assert_eq!(
            (q.sig, q.sig_changes.clone()),
            (p.sig, p.sig_changes.clone()),
            "and the Signature track"
        );
        assert_eq!(q.tracks.len(), p.tracks.len());
        assert_eq!(
            q.tracks[0].regions[0].notes.len(),
            p.tracks[0].regions[0].notes.len()
        );
        assert_eq!(q.tracks[0].regions[0].params.transpose, 5);
        assert!(
            q.tracks[4].regions[0].params == p.tracks[4].regions[0].params,
            "region gain, fade and reverse are saved"
        );
        assert_eq!(q.tracks[0].params.key_limit, (40, 90));
        assert_eq!(q.tracks[1].category, Some("Piano"));
        assert!(q.tracks[2].sends[0].pre);
        assert_eq!(q.tracks[3].height, 2.5);
        assert_eq!(q.tracks[0].inserts, p.tracks[0].inserts);
        let audio = q.tracks.last().unwrap();
        assert_eq!(audio.regions.len(), 2);
        assert!(audio.regions.iter().all(|r| r.audio.is_some()));
        assert_eq!(audio.regions[1].start_bar, 9.0);
    }

    #[test]
    fn aux_and_folder_tracks_and_their_nesting_survive_a_save() {
        let mut p = Project::demo();
        let (kinds, extra): (Vec<_>, Vec<_>) = (
            vec![TrackKind::Folder, TrackKind::Instrument, TrackKind::Aux],
            vec![(0u8, true), (1, false), (0, false)],
        );
        for ((kind, (indent, folded)), name) in
            kinds.into_iter().zip(extra).zip(["Stack", "In", "Rev"])
        {
            let mut t = Track::new(name, kind);
            (t.indent, t.folded) = (indent, folded);
            p.tracks.push(t);
        }
        let first_new = p.tracks.len() - 3;
        let (doc, _) = to_doc(&p);
        let (back, _) = from_doc(&doc, Path::new("."), &egui::Context::default());
        let got: Vec<_> = back.tracks[first_new..]
            .iter()
            .map(|t| (t.name.as_str(), t.kind, t.indent, t.folded))
            .collect();
        assert_eq!(
            got,
            [
                ("Stack", TrackKind::Folder, 0, true),
                ("In", TrackKind::Instrument, 1, false),
                ("Rev", TrackKind::Aux, 0, false),
            ]
        );
    }

    #[test]
    fn missing_media_drops_the_region_and_warns() {
        let dir = tempfile::tempdir().unwrap();
        let doc = ProjectDoc {
            tracks: vec![TrackDoc {
                name: "Vox".into(),
                kind: TrackKindDoc::Audio,
                regions: vec![RegionDoc {
                    name: "gone".into(),
                    audio: Some("media/0_gone.wav".into()),
                    ..RegionDoc::default()
                }],
                ..TrackDoc::default()
            }],
            ..ProjectDoc::default()
        };
        let (p, warnings) = from_doc(&doc, dir.path(), &egui::Context::default());
        assert_eq!(p.tracks.len(), 1);
        assert!(p.tracks[0].regions.is_empty());
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn hostile_values_are_brought_into_range() {
        let dir = tempfile::tempdir().unwrap();
        // A real audio file outside the media folder that a crafted project points at.
        let outside = dir.path().join("private.wav");
        write_wav(&outside);
        let doc = ProjectDoc {
            sig: (0, 3),
            cycle_start: -8.0,
            cycle_end: -16.0,
            tracks: vec![TrackDoc {
                key_limit: (200, 10),
                vel_limit: (0, 0),
                height: 1000.0,
                regions: vec![
                    RegionDoc {
                        start_bar: f32::INFINITY,
                        len_bars: 0.0,
                        quantize: Some(0.0),
                        notes: vec![NoteDoc {
                            start: -4.0,
                            len: 0.0,
                            pitch: 255,
                            vel: 0,
                            muted: false,
                            color: 0,
                            slide: false,
                        }],
                        ..RegionDoc::default()
                    },
                    RegionDoc {
                        audio: Some(outside.to_string_lossy().into_owned()),
                        ..RegionDoc::default()
                    },
                    RegionDoc {
                        audio: Some("media/../../private.wav".into()),
                        ..RegionDoc::default()
                    },
                ],
                ..TrackDoc::default()
            }],
            ..ProjectDoc::default()
        };
        let media = dir.path().join("media_root");
        let (p, warnings) = from_doc(&doc, &media, &egui::Context::default());
        assert_eq!(p.sig, (1, 4));
        assert_eq!((p.cycle_start, p.cycle_end), (0.0, 0.0));
        let t = &p.tracks[0];
        assert_eq!(t.height, Track::MAX_HEIGHT);
        assert_eq!(
            (t.params.key_limit, t.params.vel_limit),
            ((10, 127), (1, 1))
        );
        assert_eq!(t.regions.len(), 1, "both escaping audio paths are dropped");
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        let r = &t.regions[0];
        assert_eq!(
            (r.start_bar, r.len_bars, r.params.quantize),
            (1.0, 1.0 / 64.0, None)
        );
        let n = &r.notes[0];
        assert_eq!((n.start, n.pitch, n.vel), (0.0, 127, 1));
        assert!(n.len > 0.0);
    }

    #[test]
    fn unknown_names_are_leaked_once_per_session() {
        let a = intern("Hostile Plug-in Name");
        let b = intern(&String::from("Hostile Plug-in Name"));
        assert!(std::ptr::eq(a, b));
    }

    #[test]
    fn known_names_intern_to_static_strings() {
        assert!(std::ptr::eq(intern("Channel EQ"), intern("Channel EQ")));
        assert_eq!(
            intern("Some Third-Party Plug-in"),
            "Some Third-Party Plug-in"
        );
    }

    #[test]
    fn the_curve_shown_on_a_track_survives_saving_and_old_files_have_none() {
        let mut p = Project::empty();
        p.tracks[0].show_auto = true;
        p.tracks[0].auto.shown = crate::model::AutoTarget::Pan;
        let (doc, _) = to_doc(&p);
        let ctx = egui::Context::default();
        let (q, _) = from_doc(&doc, Path::new("."), &ctx);
        assert!(q.tracks[0].show_auto);
        assert_eq!(q.tracks[0].auto.shown, crate::model::AutoTarget::Pan);
        let mut old = doc.clone();
        old.tracks[0].show_auto = false;
        old.tracks[0].auto_lane = String::new();
        let (q, _) = from_doc(&old, Path::new("."), &ctx);
        assert!(!q.tracks[0].show_auto);
        assert_eq!(q.tracks[0].auto.shown, crate::model::AutoTarget::Volume);
    }

    #[test]
    fn track_alternatives_survive_saving_with_their_audio() {
        use crate::alternatives::{run, AltCmd};
        let dir = tempfile::tempdir().expect("a folder");
        let wav = dir.path().join("other.wav");
        ovrtone_engine::write_wav(&wav, &vec![0.1; 48_000], 48_000).expect("a file");
        let ctx = egui::Context::default();
        let mut p = Project::empty();
        p.tracks[0].regions.clear();
        crate::import::import_audio_at(&mut p, &wav, &ctx, 1.0, None).expect("the import");
        let t = p.tracks.len() - 1;
        run(&mut p, t, AltCmd::New).expect("a new version");
        p.tracks[t].alternatives[1].name = "Chorus idea".into();
        let (doc, media) = to_doc(&p);
        assert_eq!(
            media.len(),
            1,
            "the file of the version not playing is kept"
        );
        assert!(doc.tracks[t].regions.is_empty());
        assert_eq!(doc.tracks[t].alternatives[0].regions.len(), 1);
        std::fs::create_dir_all(dir.path().join("media")).expect("a folder");
        std::fs::copy(&wav, dir.path().join(&media[0].zip_path)).expect("a copy");
        let (mut q, warnings) = from_doc(&doc, dir.path(), &ctx);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(q.tracks[t].active_alt, 1);
        assert_eq!(q.tracks[t].alternatives[1].name, "Chorus idea");
        run(&mut q, t, AltCmd::Switch(0)).expect("the other version");
        assert_eq!(q.tracks[t].regions.len(), 1);
        assert!(q.tracks[t].regions[0].audio.is_some());
        // A file from before this has one version, and a number that makes no sense is let go.
        let mut old = doc.clone();
        old.tracks[t].alternatives.clear();
        old.tracks[t].active_alternative = 5;
        let (q, _) = from_doc(&old, dir.path(), &ctx);
        assert_eq!(q.tracks[t].active_alt, 0);
        assert!(q.tracks[t].alternatives.is_empty());
    }

    #[test]
    fn sections_and_chords_survive_saving_and_bad_ones_are_let_go() {
        let mut p = Project::empty();
        let i = crate::arrangement_track::add_section(&mut p, 8.0);
        p.sections[i].name = "Chorus".into();
        p.chords.clear();
        crate::chord_track::add_chord(&mut p, 4.0);
        p.chords[0].name = "Am7".into();
        let (mut doc, _) = to_doc(&p);
        let ctx = egui::Context::default();
        let (q, _) = from_doc(&doc, Path::new("."), &ctx);
        assert_eq!(q.sections, p.sections);
        assert_eq!(q.chords, p.chords);
        doc.sections[0].len = f64::NAN;
        doc.chords[0].beat = f64::INFINITY;
        let (q, _) = from_doc(&doc, Path::new("."), &ctx);
        assert!(q.sections.is_empty() && q.chords.is_empty());
        doc.sections.clear();
        doc.chords.clear();
        let (q, _) = from_doc(&doc, Path::new("."), &ctx);
        assert!(q.sections.is_empty() && q.chords.is_empty(), "old files");
    }

    #[test]
    fn a_take_folder_and_its_sustain_events_survive_saving() {
        use crate::model::{Control, Take, TakeFolder};
        let mut p = Project::demo();
        let note = |start: f32, pitch: u8| Note {
            start,
            len: 0.5,
            pitch,
            vel: 90,
            muted: false,
            color: 0,
            slide: false,
        };
        let pedal = |val: u8| Control {
            start: 0.0,
            num: 64,
            val,
        };
        let takes = vec![
            Take {
                notes: vec![note(0.0, 60), note(2.0, 62)],
                controls: vec![pedal(127)],
                audio_offset: None,
                ..Take::default()
            },
            Take {
                notes: vec![note(1.0, 64)],
                controls: vec![pedal(0)],
                audio_offset: None,
                ..Take::default()
            },
        ];
        let mut folder = TakeFolder::new(takes, 8.0);
        folder.swipe(0, 0.0, 1.5);
        let r = &mut p.tracks[0].regions[0];
        folder.apply(&mut r.notes, &mut r.controls);
        r.folder = Some(folder.clone());

        let (doc, _) = to_doc(&p);
        let (q, _) = from_doc(&doc, Path::new("."), &egui::Context::default());
        let got = &q.tracks[0].regions[0];
        let f = got.folder.as_ref().expect("the folder is back");
        assert_eq!(f.takes, folder.takes);
        assert_eq!(f.comp, folder.comp);
        assert_eq!(f.len, 8.0);
        assert!(!f.open, "opened or closed is not saved");
        assert_eq!(got.notes, p.tracks[0].regions[0].notes);
        assert_eq!(got.controls, p.tracks[0].regions[0].controls);
        // An ordinary region stays ordinary.
        assert!(q.tracks[1].regions[0].folder.is_none());
    }

    #[test]
    fn notes_and_outputs_are_saved() {
        let mut p = Project::empty();
        p.notes = "Second verse: double the vocal".into();
        p.tracks[0].output = 2;
        let (mut doc, _) = to_doc(&p);
        let (q, _) = from_doc(&doc, Path::new("."), &egui::Context::default());
        assert_eq!(q.notes, p.notes);
        assert_eq!(q.tracks[0].output, 2);
        doc.tracks[0].output = 200;
        let kept = from_doc(&doc, Path::new("."), &egui::Context::default()).0;
        assert_eq!(kept.tracks[0].output, 200, "any bus can be used");
        doc.tracks[0].output = ovrtone_engine::MAX_BUSES as u16 + 1;
        doc.notes = "x".repeat(crate::side_panes::MAX_NOTES + 10);
        let (r, _) = from_doc(&doc, Path::new("."), &egui::Context::default());
        assert_eq!(r.tracks[0].output, 0, "no such bus");
        assert_eq!(r.notes.len(), crate::side_panes::MAX_NOTES);
    }
}
