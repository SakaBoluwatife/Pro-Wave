//! Bridges the UI project model to `ovrtone-engine` using only `Command`s (never direct mutation).
use crate::model::TrackKind;
use crate::model::{Insert, Project, Region, TempoPoint, Track};
use crate::plugin_rack::Rack;
use ovrtone_engine::{
    AutoLane, AutoTarget, Chain, Command, ControlEvent, Engine, Fx, InputSource, MonitorTrack,
    NoteEvent, Pdc, Route, SendSpec, Song, TrackMix, MAX_MONITORS, MAX_TRACKS,
};

#[derive(Clone, Copy, PartialEq)]
struct Mix {
    gain_db: f32,
    mute: bool,
    solo: bool,
    pan: f32,
    trim_db: f32,
}

/// Mirrors what the engine has accepted. Each field changes only when `Engine::send`
/// returns `true`, so a command rejected because the queue was full is retried on the next
/// `sync`. `None` means nothing has been accepted yet.
pub struct EngineSync {
    /// What the engine was last told about pre-fader metering.
    pre_fader: bool,
    pub engine: Engine,
    playing: bool,
    /// The tracks that hear their input, as last sent.
    monitors: Vec<MonitorTrack>,
    /// Tempo, tempo changes and signature last sent.
    tempo: Option<TempoKey>,
    metronome: bool,
    cycle: (bool, f64, f64),
    master: Option<f32>,
    position: Option<f64>,
    mixes: Vec<Mix>,
    /// What the spatial mix was last told to go to, and each track's way into it.
    spatial_mode: ovrtone_engine::SpatialMode,
    /// Stereo Out's own effects as last sent.
    master_fx: Vec<Insert>,
    spatial_law: ovrtone_engine::SpatialLaw,
    /// The speaker setup's change count when the renderer was made, and the test channel sent.
    spatial_speakers: u64,
    spatial_test: Option<usize>,
    /// The spatial buses' layouts the renderer was made with, and their places as last sent.
    spatial_bus_layouts: Vec<ovrtone_engine::SpeakerLayout>,
    /// The spatial buses' effects as last sent.
    spatial_bus_fx: Vec<Vec<crate::model::Insert>>,
    /// The HRTF choice's change count when the renderer was made.
    spatial_hrtf: u64,
    /// Why the chosen HRTF file could not be used, for the renderer to say.
    pub hrtf_error: Option<String>,
    spatial_bus_places: Vec<ovrtone_engine::BusPlace>,
    objects: Vec<Option<ovrtone_engine::ObjectPlace>>,
    spatial_room: f32,
    spatial_channels: ovrtone_engine::ChannelSetup,
    /// Which tracks the engine was told are external, by track.
    externals: Vec<bool>,
    /// The controller changes the engine has.
    controls: Vec<ControlEvent>,
    /// MIDI Time Code as the engine was last told: on, frame rate, project start in frames.
    mtc: Option<(bool, ovrtone_engine::SmpteRate, i64)>,
    song_rev: Option<u64>,
    /// Audio layout of the song last sent with `SetSong`.
    layout: Option<AudioLayout>,
    /// The effects, routes and automation the engine has, by track.
    fx: Vec<Vec<Insert>>,
    /// The row of each built-in effect in each strip's one order with its plugins, as last sent.
    fx_rows: Vec<Vec<u8>>,
    routes: Vec<Route>,
    /// The compensation delays the engine has, by track.
    pdc: Vec<(usize, usize)>,
    /// The plugins running in the engine. Stays below `engine`, which is dropped first, so
    /// instances outlast what the audio thread holds of them.
    rack: Rack,
    /// Plugins on the spatial buses.
    bus_plugins: crate::bus_plugins::BusPlugins,
    plugin_message: Option<String>,
    /// The Varispeed percent, as last told to the engine.
    varispeed: f32,
    /// Mono sum and dim, as last told to the engine.
    monitor_out: (bool, bool),
    plugins_retry: bool,
    automation: Vec<AutoLane>,
    /// A command was rejected during the last `sync`.
    retry: bool,
}

/// The channels of the device, for the Output menus. The project holds it only to show it.
fn p_output_channels(p: &Project, channels: u32) {
    p.output_channels
        .store(channels, std::sync::atomic::Ordering::Relaxed);
}

/// Play, or with Record and Count-in on, one bar of clicks first.
fn start_command(p: &Project) -> Command {
    if !(p.recording && p.count_in) {
        return Command::Play;
    }
    let bar = p.beats_to_bar(p.position_beats).floor().max(0.0) as u32;
    let (clicks, unit) = p.signature_of_bar(bar);
    Command::CountIn {
        clicks,
        beats_per_click: 4.0 / f64::from(unit.max(1)),
    }
}

/// The audio tracks whose I button is on.
fn monitors_of(p: &Project) -> Vec<MonitorTrack> {
    p.tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| {
            let automatic = p.auto_monitor && t.rec && (!p.playing || p.recording);
            t.kind == TrackKind::Audio && (t.monitor || automatic)
        })
        .take(MAX_MONITORS)
        .map(|(i, t)| {
            let (channel, stereo) = match t.input {
                InputSource::Mono(c) => (c, false),
                InputSource::Stereo(c) => (c, true),
            };
            MonitorTrack {
                track: i.min(usize::from(u16::MAX)) as u16,
                channel,
                stereo,
            }
        })
        .collect()
}

/// Where track `i`'s signal goes: its sends, and for an Aux strip the bus it listens to.
fn route_of(p: &Project, i: usize) -> Route {
    let Some(t) = p.tracks.get(i) else {
        return Route::default();
    };
    let mut route = Route {
        bus_in: p.aux_bus_of(i),
        output: t.output,
        hardware_out: t.hardware_out,
        rank: p.aux_rank(i),
        ..Route::default()
    };
    for (slot, s) in route.sends.iter_mut().zip(&t.sends) {
        if let Some(bus) = s.bus {
            *slot = SendSpec {
                bus,
                level: s.level,
                pre: s.pre,
            };
        }
    }
    route.sanitized()
}

/// The effects as a chain the audio thread can run (made here, off that thread).
fn chain_of(inserts: &[Insert], sample_rate: f32) -> Box<Chain> {
    Box::new(Chain::new(
        inserts
            .iter()
            .map(|i| {
                Fx::new(i.kind, i.params, i.bypass, sample_rate)
                    .with_side(i.sidechain.and_then(|s| u16::try_from(s).ok()))
            })
            .collect(),
    ))
}

/// How much of a lane's value is the VCA's: only a fader lane adds the VCA's level.
fn lane_offset(p: &Project, i: usize, target: AutoTarget) -> f32 {
    match target {
        AutoTarget::Volume => p.vca_of(i).map_or(0.0, |v| v.volume_db),
        _ => 0.0,
    }
}

/// What a strip plays: its own lanes, and where regions have lanes the two merged.
fn played_lanes<'a>(
    t: &'a crate::model::Track,
    merged: &'a [(AutoTarget, Vec<ovrtone_engine::AutoPoint>)],
) -> impl Iterator<Item = (AutoTarget, &'a [ovrtone_engine::AutoPoint])> {
    t.auto
        .lanes()
        .filter(|(target, _)| !merged.iter().any(|m| m.0 == *target))
        .chain(merged.iter().map(|m| (m.0, &m.1[..])))
}

/// The lanes the engine should have for the project's automation.
fn automation_lanes(p: &Project) -> Vec<AutoLane> {
    let mut lanes = Vec::new();
    for (i, t) in p.tracks.iter().enumerate().take(MAX_TRACKS) {
        let merged = crate::automation::merged_lanes(p, i);
        for (target, points) in played_lanes(t, &merged) {
            if !points.is_empty() {
                lanes.push(
                    AutoLane {
                        track: i as u16,
                        target,
                        follow: t.auto.follows(target),
                        offset_db: lane_offset(p, i, target),
                        points: points.to_vec(),
                    }
                    .sanitized(),
                );
            }
        }
    }
    lanes
}

/// Whether `sent` is what [`automation_lanes`] would make, without making it (it runs every
/// frame, and a long recording has many points).
fn automation_unchanged(p: &Project, sent: &[AutoLane]) -> bool {
    let mut it = sent.iter();
    for (i, t) in p.tracks.iter().enumerate().take(MAX_TRACKS) {
        let merged = crate::automation::merged_lanes(p, i);
        for (target, points) in played_lanes(t, &merged) {
            if points.is_empty() {
                continue;
            }
            let Some(lane) = it.next() else {
                return false;
            };
            let (lo, hi) = crate::model::Automation::range(target);
            let (lo, hi) = match target {
                AutoTarget::Volume => (-96.0f32.max(lo), hi),
                _ => (lo, hi),
            };
            if lane.track as usize != i
                || lane.target != target
                || lane.follow != t.auto.follows(target)
                || lane.offset_db != lane_offset(p, i, target)
                || lane.points.len() != points.len().min(ovrtone_engine::MAX_AUTO_POINTS)
                || !lane.points.iter().zip(points).all(|(a, b)| {
                    a.beat == b.beat && a.value == b.value.clamp(lo, hi) && a.curve == b.curve
                })
            {
                return false;
            }
        }
    }
    it.next().is_none()
}

/// The delay every effect of the strip puts on the sound, in samples.
fn latency_of(t: &Track, sample_rate: f32, rack: &Rack) -> usize {
    // The file has the delay taken out already, and nothing is running to add more.
    if t.frozen.is_some() {
        return 0;
    }
    let built_in: usize = t
        .inserts
        .iter()
        .filter(|i| !i.bypass)
        .map(|i| i.kind.latency_for(&i.params, sample_rate))
        .sum();
    let plugins =
        rack.chain_latency(t.instrument_plugin.iter()) + rack.chain_latency(t.plugins.iter());
    built_in + plugins
}

/// Delay compensation for every track: (before the fader and sends, after them), in samples.
/// Strips whose effects look ahead delay the sound; the others wait for them so everything
/// stays in time. The signal sent to a bus is aligned too, and an Aux strip's own effects
/// are made up for after the fader.
fn pdc_plan(p: &Project, sample_rate: f32, rack: &Rack) -> Vec<(usize, usize)> {
    let sounds = |t: &Track| !matches!(t.kind, TrackKind::Vca | TrackKind::Folder);
    let lat: Vec<usize> = p
        .tracks
        .iter()
        .map(|t| latency_of(t, sample_rate, rack))
        .collect();
    let (mut plain, mut aux) = (0, 0);
    for (i, _) in p.tracks.iter().enumerate().filter(|(_, t)| sounds(t)) {
        if p.aux_bus_of(i) > 0 {
            aux = aux.max(lat[i]);
        } else {
            plain = plain.max(lat[i]);
        }
    }
    p.tracks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            if !sounds(t) {
                (0, 0)
            } else if p.aux_bus_of(i) > 0 {
                (0, aux - lat[i])
            } else {
                (plain - lat[i], aux)
            }
        })
        .collect()
}

/// Engine track number for UI track `i`. Tracks past the engine's `MAX_TRACKS` have none:
/// they are left out of the song (silent) instead of being folded onto the last track.
fn track_index(i: usize) -> Option<u16> {
    (i < MAX_TRACKS).then_some(i as u16)
}

/// The mix the engine plays track `i` with: its fader and its VCA's, Mute and Solo from the
/// track or its VCA.
fn mix_of(p: &Project, i: usize) -> Mix {
    let (gain_db, mute, solo) = p.track_mix_of(i);
    Mix {
        gain_db,
        mute,
        solo,
        pan: p.tracks[i].pan,
        trim_db: p.tracks[i].trim_db,
    }
}

/// Every track's mix as an engine song carries it.
fn track_mixes(p: &Project) -> Vec<TrackMix> {
    mixes(p)
        .into_iter()
        .map(|m| TrackMix {
            gain_db: m.gain_db,
            mute: m.mute,
            solo: m.solo,
            pan: m.pan,
            trim_db: m.trim_db,
        })
        .collect()
}

fn mixes(p: &Project) -> Vec<Mix> {
    (0..p.tracks.len().min(MAX_TRACKS))
        .map(|i| mix_of(p, i))
        .collect()
}

/// Whether a track's instrument is the sampler that plays its samples, as FL Studio's channels do.
fn plays_samples(t: &Track) -> bool {
    t.instrument_plugin.as_ref().is_some_and(|i| {
        ovrtone_host::PluginFormat::of(&i.path) == ovrtone_host::PluginFormat::Sampler
    })
}

/// The slide notes of a region that have a note to bend: each with the note it bends, the one
/// sounding at its start that began last. With `rings` the sound goes on past its note, as a
/// one-shot such as an 808 does, so that is the last note begun, however short. One with nothing to
/// bend is an ordinary note.
fn slides_of(notes: &[crate::model::Note], rings: bool) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (i, slide) in notes.iter().enumerate() {
        if !slide.slide || slide.muted {
            continue;
        }
        let base = notes
            .iter()
            .enumerate()
            .filter(|(_, n)| {
                !n.slide
                    && !n.muted
                    && n.start <= slide.start
                    && (rings || n.start + n.len > slide.start)
            })
            .max_by(|a, b| a.1.start.total_cmp(&b.1.start))
            .map(|(k, _)| k);
        if let Some(base) = base {
            out.push((i, base));
        }
    }
    out
}

/// The pitch bends that play a region's slide notes: over each slide from where the last one got
/// to, to its pitch, and back to no bend when the bent note ends. `range` is the instrument's bend
/// range in semitones; a slide further than that goes as far as it can. With `rings` the bent sound
/// outlasts its note, so it lets go only just before the next note, or at the end of the region.
fn slide_bends(
    notes: &[crate::model::Note],
    origin: f64,
    track: u16,
    range: f32,
    rings: bool,
    length: f32,
) -> Vec<ControlEvent> {
    const STEP: f32 = 1.0 / 32.0;
    let mut slides = slides_of(notes, rings);
    slides.sort_by(|a, b| notes[a.0].start.total_cmp(&notes[b.0].start));
    // Just before a note, so that it starts at its own pitch.
    const BEFORE_NOTE: f32 = 1.0 / 512.0;
    let bend =
        |semitones: f32| (8192.0 + 8191.0 * (semitones / range).clamp(-1.0, 1.0)).round() as u16;
    let event = |beat: f32, semitones: f32| {
        let value = bend(semitones);
        [
            ControlEvent {
                beat: origin + f64::from(beat),
                track,
                number: ovrtone_engine::PITCH_BEND_FINE_CONTROL,
                value: (value & 0x7f) as u8,
            },
            ControlEvent {
                beat: origin + f64::from(beat),
                track,
                number: ovrtone_engine::PITCH_BEND_CONTROL,
                value: (value >> 7) as u8,
            },
        ]
    };
    // The same slide as the tuning of the bent note alone, for an instrument that can take it.
    let tune = |beat: f32, key: u8, semitones: f32| {
        let raw = (8192.0 + semitones.clamp(-64.0, 63.99) * 128.0).round() as u16;
        let at = origin + f64::from(beat);
        [
            ControlEvent {
                beat: at,
                track,
                number: ovrtone_engine::NOTE_TUNE_KEY_CONTROL,
                value: key & 0x7f,
            },
            ControlEvent {
                beat: at,
                track,
                number: ovrtone_engine::NOTE_TUNE_FINE_CONTROL,
                value: (raw & 0x7f) as u8,
            },
            ControlEvent {
                beat: at,
                track,
                number: ovrtone_engine::NOTE_TUNE_CONTROL,
                value: ((raw >> 7) & 0x7f) as u8,
            },
        ]
    };
    let mut out = Vec::new();
    let mut last: Option<(usize, f32)> = None;
    for (at, &(slide, base)) in slides.iter().enumerate() {
        let (s, b) = (notes[slide], notes[base]);
        let from = last.filter(|(k, _)| *k == base).map_or(0.0, |(_, to)| to);
        let to = f32::from(s.pitch) - f32::from(b.pitch);
        let end = if rings {
            notes
                .iter()
                .filter(|n| !n.slide && !n.muted && n.start > b.start)
                .map(|n| n.start - BEFORE_NOTE)
                .fold(length, f32::min)
        } else {
            b.start + b.len
        };
        let steps = (s.len / STEP).ceil().clamp(1.0, 64.0) as u32;
        for k in 1..=steps {
            let part = k as f32 / steps as f32;
            let beat = s.start + s.len * part;
            if beat < end {
                let semis = from + (to - from) * part;
                out.extend(event(beat, semis));
                out.extend(tune(beat, b.pitch, semis));
            }
        }
        last = Some((base, to));
        // The next slide on this note carries on from here; else its end lets go of the bend.
        if slides.get(at + 1).is_none_or(|next| next.1 != base) {
            out.extend(event(end, 0.0));
            out.extend(tune(end, b.pitch, 0.0));
        }
    }
    out
}

/// Notes of one MIDI region as the engine should play them, with the Region Inspector
/// (quantize, transpose, velocity) and Track Inspector (transpose, velocity, key and
/// velocity limits) applied non-destructively, as Logic does on playback.
pub(crate) fn region_notes(t: &Track, r: &Region, origin: f64, track: u16) -> Vec<NoteEvent> {
    let (rp, tp) = (r.params, t.params);
    let shape = rp.shape;
    let average_vel = if shape.q_velocity > 0 && !r.notes.is_empty() {
        r.notes.iter().map(|n| f32::from(n.vel)).sum::<f32>() / r.notes.len() as f32
    } else {
        0.0
    };
    let mut flams: std::collections::HashMap<i64, u32> = std::collections::HashMap::new();
    // Slide notes that bend what is sounding start no note. Only a hosted instrument can be bent.
    let bends = if t.instrument_plugin.is_some() {
        slides_of(&r.notes, plays_samples(t))
    } else {
        Vec::new()
    };
    r.notes
        .iter()
        .enumerate()
        .filter(|(i, n)| !n.muted && !bends.iter().any(|b| b.0 == *i))
        .filter_map(|(_, n)| {
            let (mut start, mut len) = (n.start, n.len);
            if let Some(q) = rp.quantize.filter(|q| *q > 0.0) {
                let target = (n.start / q).round() * q;
                let range = f32::from(shape.q_range) / 100.0;
                if shape.q_range == 0 || (target - n.start).abs() <= range {
                    let strength = 1.0 - f32::from(shape.q_soft) / 100.0;
                    start = n.start + (target - n.start) * strength;
                }
                if shape.q_length > 0 {
                    let on_grid = ((n.len / q).round() * q).max(q);
                    len += (on_grid - len) * f32::from(shape.q_length) / 100.0;
                }
                if shape.q_flam > 0 {
                    let count = flams.entry((target / q).round() as i64).or_insert(0);
                    start += *count as f32 * f32::from(shape.q_flam) / 100.0;
                    *count += 1;
                }
            }
            if shape.gate != 0 {
                len = (len * (1.0 + f32::from(shape.gate) / 100.0)).max(1.0 / 64.0);
            }
            let mut own_vel = f32::from(n.vel);
            if shape.q_velocity > 0 {
                own_vel += (average_vel - own_vel) * f32::from(shape.q_velocity) / 100.0;
            }
            if shape.dynamics != 0 {
                own_vel = 64.0 + (own_vel - 64.0) * (1.0 + f32::from(shape.dynamics) / 100.0);
            }
            let pitch = n.pitch as i32 + rp.transpose as i32 + tp.transpose as i32;
            let vel =
                (own_vel.round() as i32 + rp.velocity as i32 + tp.velocity as i32).clamp(1, 127);
            let in_keys = (tp.key_limit.0 as i32..=tp.key_limit.1 as i32).contains(&pitch);
            let in_vels = (tp.vel_limit.0 as i32..=tp.vel_limit.1 as i32).contains(&vel);
            (in_keys && in_vels && (0..=127).contains(&pitch)).then_some(NoteEvent {
                start_beat: origin + start as f64,
                end_beat: origin + (start + len) as f64,
                pitch: pitch as u8,
                vel: vel as u8,
                track,
            })
        })
        .collect()
}

/// Clip Length: what of `notes` falls inside the region, cut at its end.
fn clip_notes(mut notes: Vec<NoteEvent>, r: &Region, end: f64) -> Vec<NoteEvent> {
    if r.params.shape.clip_length {
        notes.retain(|n| n.start_beat < end);
        for n in &mut notes {
            n.end_beat = n.end_beat.min(end);
        }
    }
    notes
}

/// Every unmuted region on an engine track, with its engine track number and start beat
/// (quarter notes, as the engine counts them).
fn playing_regions(p: &Project) -> impl Iterator<Item = (u16, &Track, &Region, f64)> {
    p.tracks
        .iter()
        .enumerate()
        .filter_map(|(ti, t)| Some((track_index(ti)?, t)))
        .flat_map(move |(track, t)| {
            // A frozen track plays its file, and only that.
            let frozen = t.frozen.as_deref();
            frozen
                .into_iter()
                .chain(
                    t.sounding_regions()
                        .iter()
                        .filter(move |r| frozen.is_none() && !r.params.mute),
                )
                .map(move |r| (track, t, r, p.region_start_beats(r)))
        })
}

/// The controller changes external instruments get: those recorded in the regions of External
/// MIDI tracks (the sustain pedal among them), at their place on the timeline.
fn song_controls(p: &Project) -> Vec<ControlEvent> {
    p.tracks
        .iter()
        .enumerate()
        .filter_map(|(ti, t)| Some((track_index(ti)?, t)))
        // Those whose instrument is outside the engine: a MIDI port or a hosted plugin.
        .filter(|(_, t)| t.kind == TrackKind::ExternalMidi || t.instrument_plugin.is_some())
        .flat_map(|(track, t)| {
            let mut out = Vec::new();
            let (mut from, mut to) = (f64::MAX, 0.0f64);
            for r in t
                .regions
                .iter()
                .filter(|r| !r.params.mute && t.frozen.is_none())
            {
                let origin = p.region_start_beats(r);
                from = from.min(origin);
                to = to.max(p.region_end_beats(r));
                out.extend(r.controls.iter().map(|c| ControlEvent {
                    beat: origin + f64::from(c.start),
                    track,
                    number: c.num.min(127),
                    value: c.val.min(127),
                }));
                if t.instrument_plugin.is_some() {
                    let range = if plays_samples(t) {
                        ovrtone_host::SAMPLER_BEND_SEMITONES
                    } else {
                        f32::from(t.slide_range.max(1))
                    };
                    let length = p.region_len_beats(r) as f32;
                    out.extend(slide_bends(
                        &r.notes,
                        origin,
                        track,
                        range,
                        plays_samples(t),
                        length,
                    ));
                }
            }
            // A Modulator on the track sends its controller along all the regions.
            out.extend(
                crate::midi_fx::controls(&t.midi_fx, from, to)
                    .into_iter()
                    .map(|m| ControlEvent {
                        beat: m.beat,
                        track,
                        number: m.number,
                        value: m.value,
                    }),
            );
            out
        })
        .collect()
}

#[cfg(test)]
pub(crate) fn song_notes_for_test(p: &Project) -> Vec<NoteEvent> {
    song_notes(p)
}

fn song_notes(p: &Project) -> Vec<NoteEvent> {
    // The Groove Track's timing, for the tracks that follow it.
    let groove = p
        .tracks
        .iter()
        .any(|t| t.params.follow_groove)
        .then(|| crate::groove::template(p))
        .flatten();
    p.tracks
        .iter()
        .enumerate()
        .filter_map(|(ti, t)| Some((track_index(ti)?, t)))
        .flat_map(|(track, t)| {
            let mut notes: Vec<NoteEvent> = t
                .sounding_regions()
                .iter()
                .filter(|r| !r.params.mute && t.frozen.is_none())
                .flat_map(|r| {
                    let notes = region_notes(t, r, p.region_start_beats(r), track);
                    clip_notes(notes, r, p.region_end_beats(r))
                })
                .collect();
            if let Some(g) = groove
                .as_ref()
                .filter(|_| t.params.follow_groove && !t.params.groove)
            {
                crate::groove::apply(&mut notes, g);
            }
            // The track's MIDI FX change what it plays.
            if t.midi_fx.iter().any(|s| s.on) {
                crate::midi_fx::apply(&t.midi_fx, notes)
            } else {
                notes
            }
        })
        .collect()
}

/// What the engine's streamed audio depends on: the track count and each playing audio
/// region's file, track and start. When an edit leaves it unchanged (MIDI edits, mixer
/// changes), only the notes are swapped and the audio streams keep running.
#[derive(PartialEq, Default, Debug)]
struct AudioLayout {
    tracks: usize,
    clips: Vec<(std::path::PathBuf, u16, u64, String)>,
}

/// The edits of region `r` of track `t`: its own gain, fades and reverse, with the fades
/// lengthened where it crossfades with another region.
fn clip_edit(p: &Project, t: &Track, r: &Region) -> ovrtone_engine::ClipEdit {
    let ti = p.tracks.iter().position(|x| std::ptr::eq(x, t));
    let ri = t.regions.iter().position(|x| std::ptr::eq(x, r));
    match (ti, ri) {
        (Some(ti), Some(ri)) => p.clip_edit_of(ti, ri),
        _ => r.params.clip_edit(),
    }
}

/// The clips that play audio region `r`: itself, or the stretches of its comp of takes.
fn region_clips<'a>(
    p: &Project,
    t: &Track,
    r: &'a Region,
    origin: f64,
) -> Vec<(f64, ovrtone_engine::ClipEdit, &'a ovrtone_engine::AudioFile)> {
    let edit = clip_edit(p, t, r);
    match (&r.folder, &r.audio) {
        (Some(folder), Some(a)) if folder.is_audio() => {
            crate::takes::audio_comp_clips(folder, &p.tempo_map(), origin, edit, &a.file)
        }
        (_, Some(a)) => vec![(origin, edit, &a.file)],
        (_, None) => Vec::new(),
    }
}

fn audio_layout(p: &Project) -> AudioLayout {
    AudioLayout {
        tracks: p.tracks.len(),
        clips: playing_regions(p)
            .filter(|(_, _, r, _)| r.audio.is_some())
            .flat_map(|(track, t, r, origin)| {
                region_clips(p, t, r, origin)
                    .into_iter()
                    .map(move |(at, edit, file)| {
                        // Gain, fades, the gain line and reverse are set when a clip starts streaming.
                        let edit = format!("{edit:?}{:?}", r.gain_line);
                        (file.path.clone(), track, at.to_bits(), edit)
                    })
            })
            .collect(),
    }
}

fn build_song(engine: &Engine, p: &Project) -> Song {
    // Every clip reads ahead as much as the whole song's prefetch memory allows.
    let streamed = playing_regions(p)
        .filter(|(_, _, r, _)| r.audio.is_some())
        .map(|(_, t, r, origin)| region_clips(p, t, r, origin).len())
        .sum();
    let clips = playing_regions(p)
        .flat_map(|(track, t, r, origin)| {
            region_clips(p, t, r, origin)
                .into_iter()
                .map(move |(at, edit, file)| {
                    let line = r.gain_line.iter().map(|(b, db)| (origin + f64::from(*b), *db)).collect();
                    engine.stream_clip_in(file, track, at, edit, streamed).with_gain_line(line)
                })
        })
        .collect();
    let notes = song_notes(p);
    let tracks = track_mixes(p);
    Song::new(notes, tracks).with_clips(clips)
}

/// The project's MIDI as an engine song, without audio regions (which need a running
/// `Engine` to stream). Used by the offline benchmark.
pub fn offline_song(p: &Project) -> Song {
    Song::new(song_notes(p), track_mixes(p))
}

/// The project's Tempo track and time signature as an engine tempo map.
fn tempo_map(p: &Project) -> Command {
    Command::SetTempoMap(Box::new(p.tempo_map()))
}

/// What the engine's tempo map depends on: the start tempo, the tempo changes, the first
/// signature and the signature changes.
type TempoKey = (f32, Vec<TempoPoint>, (u8, u8), Vec<crate::model::SigChange>);

fn tempo_key(p: &Project) -> TempoKey {
    (p.tempo, p.tempo_points(), p.sig, p.sig_changes.clone())
}

impl EngineSync {
    pub fn new(engine: Engine, p: &Project) -> Self {
        let mut sync = Self {
            pre_fader: false,
            engine,
            playing: false,
            monitors: Vec::new(),
            tempo: None,
            metronome: false,
            cycle: (false, 0.0, 0.0),
            master: None,
            position: None,
            mixes: Vec::new(),
            spatial_mode: ovrtone_engine::SpatialMode::Off,
            master_fx: Vec::new(),
            spatial_law: ovrtone_engine::SpatialLaw::Smooth,
            spatial_speakers: 0,
            spatial_test: None,
            spatial_bus_layouts: Vec::new(),
            spatial_bus_fx: Vec::new(),
            spatial_hrtf: 0,
            hrtf_error: None,
            spatial_bus_places: Vec::new(),
            objects: Vec::new(),
            spatial_room: 0.0,
            spatial_channels: ovrtone_engine::ChannelSetup::default(),
            externals: Vec::new(),
            controls: Vec::new(),
            mtc: None,
            song_rev: None,
            layout: None,
            fx: Vec::new(),
            fx_rows: Vec::new(),
            routes: Vec::new(),
            pdc: Vec::new(),
            bus_plugins: Default::default(),
            rack: Rack::default(),
            plugin_message: None,
            monitor_out: (false, false),
            varispeed: 0.0,
            plugins_retry: false,
            automation: Vec::new(),
            retry: false,
        };
        sync.push(p);
        sync
    }

    /// How many of the project's plugins are still to load, and the name of the next.
    pub fn plugins_left(&self, p: &mut Project) -> (usize, Option<String>) {
        self.rack.loads_left(p)
    }

    /// Call once per frame: push UI-side changes to the engine, then pull the playhead back.
    pub fn sync(&mut self, p: &mut Project) {
        let counted = !self.playing && p.playing && p.recording && p.count_in;
        let copies = crate::plugin_rack::give_copies_their_own_uids(p);
        self.rack.copy_states(&copies);
        self.plugins_retry = !self
            .rack
            .sync(p, &mut self.engine, &mut self.plugin_message);
        let seek_done = self.push(p);
        if counted && self.playing {
            p.counting_in = true;
        } else if p.counting_in && (!p.playing || self.engine.state().playing) {
            p.counting_in = false;
        }
        // Engine owns time while playing, unless a seek is still waiting to be accepted
        // (then the UI keeps the requested position and `push` retries it).
        if seek_done {
            if p.playing {
                p.position_beats = self.engine.state().position_beats;
            }
            self.position = Some(p.position_beats);
        }
    }

    /// Sends every change the engine has not accepted yet. Returns `false` while a playhead
    /// move is still pending.
    fn push(&mut self, p: &Project) -> bool {
        self.retry = false;
        // Tracks or regions changed. Re-streaming audio reopens every file and briefly drops
        // it out, so swap only the notes unless the audio layout itself changed.
        if self.song_rev != Some(p.song_rev) {
            let layout = audio_layout(p);
            let sent = if self.layout.as_ref() == Some(&layout) {
                self.send(Command::set_notes(song_notes(p)))
            } else {
                let song = build_song(&self.engine, p);
                let sent = self.send(Command::SetSong(Box::new(song)));
                if sent {
                    self.mixes = mixes(p);
                    self.layout = Some(layout);
                    // The new song has no controllers yet: they follow below.
                    self.controls.clear();
                }
                sent
            };
            if sent {
                self.song_rev = Some(p.song_rev);
            }
        }
        let controls = song_controls(p);
        if self.song_rev == Some(p.song_rev)
            && controls != self.controls
            && self.send(Command::set_controls(controls.clone()))
        {
            self.controls = controls;
        }
        // The user moved the playhead (ruler drag, rewind buttons, menu).
        let seek_done = match self.position {
            Some(q) if (p.position_beats - q).abs() <= 1e-9 => true,
            _ => self.send(Command::SetPosition {
                beat: p.position_beats,
            }),
        };
        let monitors = monitors_of(p);
        if monitors != self.monitors {
            let mut tracks = [MonitorTrack::default(); MAX_MONITORS];
            tracks[..monitors.len()].copy_from_slice(&monitors);
            let count = monitors.len() as u8;
            if self.send(Command::SetMonitors { tracks, count }) {
                self.monitors = monitors;
            }
        }
        if p.playing != self.playing
            && self.send(if p.playing {
                start_command(p)
            } else {
                Command::Pause
            })
        {
            self.playing = p.playing;
        }
        let key = tempo_key(p);
        if self.tempo.as_ref() != Some(&key) && self.send(tempo_map(p)) {
            self.tempo = Some(key);
        }
        let mtc = (p.mtc_out.is_some(), p.smpte_rate, p.smpte_offset_frames());
        if self.mtc != Some(mtc)
            && self.send(Command::SetMtc {
                on: mtc.0,
                rate: mtc.1,
                offset_frames: mtc.2,
            })
        {
            self.mtc = Some(mtc);
        }
        if p.metronome != self.metronome && self.send(Command::SetMetronome { on: p.metronome }) {
            self.metronome = p.metronome;
        }
        let cycle = (p.cycle, p.cycle_start, p.cycle_end);
        if cycle != self.cycle
            && self.send(Command::SetCycle {
                on: p.cycle,
                start_beat: p.cycle_start,
                end_beat: p.cycle_end,
            })
        {
            self.cycle = cycle;
        }
        p_output_channels(p, self.engine.output_channels());
        if self.varispeed != p.varispeed
            && self.send(Command::SetVarispeed(1.0 + f64::from(p.varispeed) / 100.0))
        {
            self.varispeed = p.varispeed;
        }
        let monitor_out = (p.out_mono, p.out_dim);
        if self.monitor_out != monitor_out
            && self.send(Command::SetMonitorOut {
                mono: monitor_out.0,
                dim: monitor_out.1,
            })
        {
            self.monitor_out = monitor_out;
        }
        if self.master != Some(p.master_vol)
            && self.send(Command::SetMasterGain {
                linear: p.master_vol,
            })
        {
            self.master = Some(p.master_vol);
        }
        // External MIDI tracks: their notes go out as MIDI instead of sounding here.
        for (i, t) in p.tracks.iter().enumerate().take(MAX_TRACKS) {
            let external = t.kind == TrackKind::ExternalMidi;
            if self.externals.get(i).copied().unwrap_or(false) == external {
                continue;
            }
            if !self.send(Command::SetTrackExternal {
                track: i as u16,
                external,
            }) {
                break;
            }
            self.externals
                .resize(self.externals.len().max(i + 1), false);
            self.externals[i] = external;
        }
        self.sync_strips(p);
        self.sync_spatial(p);
        self.bus_plugins.sync(p, &mut self.engine, &mut self.plugin_message);
        // Mixer moves. Until a song is accepted, its `SetSong` carries the mix instead.
        if self.layout.is_some() {
            for i in 0..p.tracks.len().min(MAX_TRACKS) {
                let m = mix_of(p, i);
                if self.mixes.get(i) == Some(&m) {
                    continue;
                }
                let mix = TrackMix {
                    gain_db: m.gain_db,
                    mute: m.mute,
                    solo: m.solo,
                    pan: m.pan,
                    trim_db: m.trim_db,
                };
                if !self.send(Command::SetTrackMix {
                    track: i as u16,
                    mix,
                }) {
                    break;
                }
                if let Some(slot) = self.mixes.get_mut(i) {
                    *slot = m;
                }
            }
        }
        seek_done
    }

    /// The spatial mix: where it goes, and how each object feeds it. Only what changed is sent.
    fn sync_spatial(&mut self, p: &Project) {
        use ovrtone_engine::{SpatialMode, SpatialRender};
        // A stereo project with spatial tracks: they are rendered for headphones on their own, the
        // rest of the mix as it is.
        let light = p.spatial_mode == SpatialMode::Off && p.tracks.iter().any(|t| t.object.is_some());
        let mode = if light {
            SpatialMode::Binaural(p.spatial_bed)
        } else {
            p.spatial_mode
        };
        let (speakers, generation) = crate::speaker_setup::current();
        let layouts: Vec<_> = p.spatial_buses.iter().map(|b| b.layout).collect();
        let (_, hrtf_generation) = crate::hrtf::current();
        if mode != self.spatial_mode
            || p.spatial_law != self.spatial_law
            || generation != self.spatial_speakers
            || layouts != self.spatial_bus_layouts
            // A new renderer only when effects come or go; new settings go to the running ones.
            || p.spatial_buses
                .iter()
                .map(|b| b.fx.iter().map(|s| s.kind).collect::<Vec<_>>())
                .ne(self.spatial_bus_fx.iter().map(|f| f.iter().map(|s| s.kind).collect::<Vec<_>>()))
            || hrtf_generation != self.spatial_hrtf
        {
            let buses: Vec<_> = p.spatial_buses.iter().map(|b| (b.layout, b.place)).collect();
            let render = SpatialRender::with_speakers(
                mode,
                self.engine.sample_rate(),
                p.spatial_law,
                speakers,
            )
            .with_buses(&buses)
            .with_bus_effects(
                &p.spatial_buses
                    .iter()
                    .map(|b| b.fx.iter().map(|s| (s.kind, s.params, s.bypass)).collect())
                    .collect::<Vec<_>>(),
                self.engine.sample_rate() as f32,
            );
            // A measured head for headphones, when one is chosen; the modelled one otherwise.
            let render = if light { render.with_dry_bed() } else { render };
            let render = match (mode, crate::hrtf::current().0.is_some()) {
                (SpatialMode::Binaural(layout), true) => {
                    match crate::hrtf::set_for(layout, self.engine.sample_rate()) {
                        Ok(Some(set)) => {
                            self.hrtf_error = None;
                            render.with_hrtf(&set)
                        }
                        Ok(None) => render,
                        Err(e) => {
                            self.hrtf_error = Some(e);
                            render
                        }
                    }
                }
                _ => render,
            };
            if !self.send(Command::SetSpatial(render)) {
                return;
            }
            self.spatial_mode = mode;
            self.spatial_law = p.spatial_law;
            self.spatial_speakers = generation;
            self.spatial_bus_layouts = layouts;
            self.spatial_bus_fx = p.spatial_buses.iter().map(|b| b.fx.clone()).collect();
            self.spatial_hrtf = hrtf_generation;
            self.spatial_bus_places = p.spatial_buses.iter().map(|b| b.place).collect();
            // A new renderer has no test signal on.
            self.spatial_test = None;
        }
        for (k, b) in p.spatial_buses.iter().enumerate() {
            for (slot, e) in b.fx.iter().enumerate() {
                let sent = self.spatial_bus_fx.get(k).and_then(|f| f.get(slot));
                if sent == Some(e) {
                    continue;
                }
                if !self.send(Command::SetSpatialBusFx {
                    bus: k as u8,
                    slot: slot as u8,
                    params: e.params,
                    bypass: e.bypass,
                }) {
                    return;
                }
                if let Some(s) = self.spatial_bus_fx.get_mut(k).and_then(|f| f.get_mut(slot)) {
                    *s = *e;
                }
            }
            if self.spatial_bus_places.get(k) == Some(&b.place) {
                continue;
            }
            if !self.send(Command::SetSpatialBus {
                bus: k as u8,
                place: b.place,
            }) {
                return;
            }
            if let Some(slot) = self.spatial_bus_places.get_mut(k) {
                *slot = b.place;
            }
        }
        if p.spatial_test != self.spatial_test {
            let channel = p.spatial_test.and_then(|c| u8::try_from(c).ok());
            if !self.send(Command::SetSpatialTest(channel)) {
                return;
            }
            self.spatial_test = p.spatial_test;
        }
        let soloing = p.spatial_solo.contains(&true);
        let mut channels = p.spatial_channels;
        for c in 0..ovrtone_engine::SPATIAL_CHANNELS {
            channels.muted[c] = p.spatial_mute[c] || (soloing && !p.spatial_solo[c]);
        }
        channels.gain = 10f32.powf(p.spatial_volume_db / 20.0);
        if channels != self.spatial_channels {
            if !self.send(Command::SetSpatialChannels(channels)) {
                return;
            }
            self.spatial_channels = channels;
        }
        if p.spatial_room != self.spatial_room {
            if !self.send(Command::SetSpatialRoom(p.spatial_room)) {
                return;
            }
            self.spatial_room = p.spatial_room;
        }
        let count = p.tracks.len().min(MAX_TRACKS);
        // Off, every strip is in the bed: this need not even be looked at.
        if mode == SpatialMode::Off && self.objects.is_empty() {
            return;
        }
        let known = self.objects.len();
        if known < count {
            self.objects.resize(count, None);
        }
        for i in 0..self.objects.len() {
            let wanted = (i < count)
                .then(|| p.tracks[i].object)
                .flatten();
            if self.objects[i] == wanted {
                continue;
            }
            if !self.send(Command::SetStripObject {
                track: i as u16,
                place: wanted,
            }) {
                return;
            }
            self.objects[i] = wanted;
        }
    }

    /// The loudest of each spatial channel since this was last asked.
    pub fn take_spatial_peaks(&self) -> [f32; ovrtone_engine::SPATIAL_CHANNELS] {
        self.engine.take_spatial_peaks()
    }

    /// How many channels the output device has.
    pub fn output_channels(&self) -> u32 {
        self.engine.output_channels()
    }

    /// Effects, sends and automation. Each is sent only when it differs from what the engine
    /// has; a rejected command is tried again next frame.
    fn sync_strips(&mut self, p: &Project) {
        let sr = self.engine.sample_rate() as f32;
        let n = p.tracks.len().min(MAX_TRACKS).max(self.fx.len());
        self.fx.resize_with(n, Vec::new);
        self.fx_rows.resize_with(n, Vec::new);
        self.routes.resize(n, Route::default());
        for i in 0..n {
            let want: &[Insert] = p
                .tracks
                .get(i)
                .filter(|t| i < MAX_TRACKS && t.frozen.is_none())
                .map_or(&[], |t| &t.inserts);
            let want_rows: Vec<u8> = p
                .tracks
                .get(i)
                .map_or_else(Vec::new, |t| crate::model::fx_rows(want.len(), &t.plugins))
                .into_iter()
                .map(|r| r.min(255) as u8)
                .collect();
            let same_kinds = self.fx[i].len() == want.len()
                && self.fx[i]
                    .iter()
                    .zip(want)
                    .all(|(a, b)| a.kind == b.kind && a.sidechain == b.sidechain);
            if !same_kinds || self.fx_rows[i] != want_rows {
                let chain = Box::new((*chain_of(want, sr)).with_rows(want_rows.clone()));
                if !self.send(Command::SetInserts {
                    track: i as u16,
                    chain,
                }) {
                    return;
                }
                self.fx[i] = want.to_vec();
                self.fx_rows[i] = want_rows;
            } else {
                for (slot, (have, w)) in self.fx[i].iter_mut().zip(want).enumerate() {
                    if have == w {
                        continue;
                    }
                    let cmd = Command::SetFx {
                        track: i as u16,
                        slot: slot as u8,
                        params: w.params,
                        bypass: w.bypass,
                    };
                    if !self.engine.send(cmd) {
                        self.retry = true;
                        return;
                    }
                    *have = *w;
                }
            }
            let route = route_of(p, i);
            if self.routes[i] != route {
                if !self.send(Command::SetRoute {
                    track: i as u16,
                    route,
                }) {
                    return;
                }
                self.routes[i] = route;
            }
        }
        // Stereo Out's own effects, as a track's are sent.
        {
            let master = ovrtone_engine::MASTER_STRIP;
            let want = &p.master_strip.inserts;
            let same_kinds = self.master_fx.len() == want.len()
                && self.master_fx.iter().zip(want).all(|(a, b)| a.kind == b.kind);
            if !same_kinds {
                if !self.send(Command::SetInserts {
                    track: master,
                    chain: chain_of(want, sr),
                }) {
                    return;
                }
                self.master_fx = want.clone();
            } else {
                for (slot, (have, w)) in self.master_fx.iter_mut().zip(want).enumerate() {
                    if have == w {
                        continue;
                    }
                    let cmd = Command::SetFx {
                        track: master,
                        slot: slot as u8,
                        params: w.params,
                        bypass: w.bypass,
                    };
                    if !self.engine.send(cmd) {
                        self.retry = true;
                        return;
                    }
                    *have = *w;
                }
            }
        }
        // Delay compensation follows the effects (a limiter looks ahead).
        let plan = pdc_plan(p, sr, &self.rack);
        self.pdc.resize(n, (0, 0));
        for (i, want) in plan.iter().copied().enumerate().take(n) {
            if self.pdc[i] != want {
                let cmd = Command::SetPdc {
                    track: i as u16,
                    pdc: Box::new(Pdc::new(want.0, want.1)),
                };
                if !self.send(cmd) {
                    return;
                }
                self.pdc[i] = want;
            }
        }
        // Tracks that are gone have no compensation either.
        for i in plan.len()..n {
            if self.pdc[i] != (0, 0) {
                let cmd = Command::SetPdc {
                    track: i as u16,
                    pdc: Box::new(Pdc::default()),
                };
                if !self.send(cmd) {
                    return;
                }
                self.pdc[i] = (0, 0);
            }
        }
        if !automation_unchanged(p, &self.automation) {
            let lanes = automation_lanes(p);
            if self.send(Command::SetAutomation(lanes.clone())) {
                self.automation = lanes;
            }
        }
    }

    /// Collects the plugins' current settings into the project, right before it is saved.
    /// The notes the ARA plugin on track `t` found (see [`crate::plugin_rack::Rack::ara_notes`]).
    pub fn ara_notes(&mut self, t: usize) -> Result<Vec<(f64, f64, f32, f32)>, String> {
        self.rack.ara_notes(t)
    }

    pub fn collect_plugin_states(&mut self, p: &mut Project) {
        self.rack.collect_states(p, &mut self.plugin_message);
        self.bus_plugins.collect_states(p);
    }

    /// Opens the window of a spatial bus's plugin.
    pub fn open_bus_plugin_editor(&mut self, uid: u64, title: &str, owner: Option<isize>) {
        if let Err(e) = self.bus_plugins.open_editor(uid, title, owner) {
            self.plugin_message = Some(e);
        }
    }

    /// Whether plugins may hold changes the last save does not have.
    pub fn plugins_touched(&self) -> bool {
        self.rack.touched()
    }

    /// The project was just saved, or another one opened.
    pub fn clear_plugins_touched(&mut self) {
        self.rack.clear_touched();
    }

    /// Opens the editor of the plugin `uid`, above the main window `owner`.
    pub fn open_plugin_editor(&mut self, uid: u64, title: &str, owner: Option<isize>) {
        if let Err(e) = self.rack.open_editor(uid, title, owner) {
            self.plugin_message = Some(e);
        }
    }

    /// How soon the program must run again for its plugins, however quiet it is otherwise.
    pub fn plugins_want_repaint_after(&self) -> Option<std::time::Duration> {
        if self.rack.has_open_editor() {
            Some(std::time::Duration::from_millis(33))
        } else if self.rack.has_plugins() {
            Some(std::time::Duration::from_millis(100))
        } else {
            None
        }
    }

    /// How late the whole mix comes out because of delay compensation, in samples.
    pub fn total_latency(&self, p: &Project) -> usize {
        let sr = self.engine.sample_rate() as f32;
        let sounds = |t: &Track| !matches!(t.kind, TrackKind::Vca | TrackKind::Folder);
        pdc_plan(p, sr, &self.rack)
            .iter()
            .zip(&p.tracks)
            .filter(|(_, t)| sounds(t))
            .map(|(plan, t)| plan.0 + plan.1 + latency_of(t, sr, &self.rack))
            .max()
            .unwrap_or(0)
            + self.rack.chain_latency(p.master_plugins.iter())
    }

    /// Where the strip meters read, when it changed. Tried again next frame if the queue is full.
    pub fn set_pre_fader(&mut self, on: bool) {
        if self.pre_fader != on && self.engine.send(Command::SetPreFaderMeter(on)) {
            self.pre_fader = on;
        }
    }

    /// Moves a plugin parameter from a Smart Control.
    pub fn smart_move(&mut self, uid: u64, param: u32, value: f64) -> bool {
        self.rack.smart_move(uid, param, value, &mut self.engine)
    }

    /// Where a plugin parameter is now.
    pub fn show_param(&mut self, uid: u64, param: u32, value: f64) {
        self.rack.show_param(uid, param, value);
    }

    pub fn param_value(&mut self, uid: u64, param: u32) -> Option<f64> {
        self.rack.param_value(uid, param)
    }

    /// Saves or loads a plugin preset.
    pub fn run_preset(&mut self, cmd: &crate::plugin_presets::PresetCmd) -> Result<(), String> {
        use crate::plugin_presets::PresetCmd;
        match cmd {
            PresetCmd::Save { uid, file } => self.rack.save_preset(*uid, file),
            PresetCmd::Load { uid, file } => self.rack.load_preset(*uid, file),
            PresetCmd::Compare { uid } => self.rack.compare_flip(*uid),
            PresetCmd::CopyCompare { uid } => self.rack.compare_copy(*uid),
        }
    }

    /// The plugins showing the B side of A/B compare.
    pub fn showing_b(&self) -> Vec<u64> {
        self.rack.showing_b()
    }

    /// Runs what the plugins asked for on this thread, with no other change.
    pub fn tick_plugins(&mut self) {
        self.rack.tick();
    }

    /// What went wrong with a plugin since last asked, for the status line.
    pub fn take_plugin_message(&mut self) -> Option<String> {
        self.plugin_message.take()
    }

    /// `Engine::send`, remembering a rejection so the caller keeps redrawing to retry.
    fn send(&mut self, cmd: Command) -> bool {
        let sent = self.engine.send(cmd);
        self.retry |= !sent;
        sent
    }

    /// The MIDI the engine sends to external instruments (once).
    pub fn take_midi_out(&mut self) -> Option<rtrb::Consumer<ovrtone_engine::MidiOutEvent>> {
        self.engine.take_midi_out()
    }

    /// The MIDI Time Code the engine sends (once).
    pub fn take_mtc_out(&mut self) -> Option<rtrb::Consumer<ovrtone_engine::MtcMessage>> {
        self.engine.take_mtc_out()
    }

    /// Whether a command was rejected (queue full) and is waiting to be retried.
    pub fn needs_retry(&self) -> bool {
        self.retry || self.plugins_retry
    }

    pub fn underruns(&self) -> u32 {
        self.engine
            .shared()
            .underruns
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Post-fader stereo peaks for the first `n` tracks (linear), for the mixer meters.
    /// The RMS of the first `n` tracks, linear.
    pub fn track_rms(&mut self, n: usize) -> Vec<f32> {
        let state = self.engine.state();
        (0..n).map(|i| state.track_rms(i)).collect()
    }

    pub fn track_meters(&mut self, n: usize) -> Vec<(f32, f32)> {
        let state = self.engine.state();
        (0..n).map(|i| state.track_peaks(i)).collect()
    }

    /// The clip indicators of the first `n` tracks, for the mixer.
    pub fn track_clips(&mut self, n: usize) -> Vec<bool> {
        let state = self.engine.state();
        (0..n).map(|i| state.track_clip(i)).collect()
    }

    /// The output's clip indicator.
    pub fn master_clip(&mut self) -> bool {
        self.engine.state().master_clip
    }

    /// Puts out clip indicators. False when the engine could not take that now.
    pub fn reset_clip(&mut self, reset: crate::mixer::ClipReset) -> bool {
        use crate::mixer::ClipReset;
        self.engine.send(match reset {
            ClipReset::Track(t) => Command::ResetClip {
                track: track_index(t),
            },
            ClipReset::Master => Command::ResetClip { track: None },
            ClipReset::All => Command::ResetAllClips,
        })
    }

    /// Gain reduction in dB of the compressors of the first `n` tracks, for the mixer.
    pub fn gain_reduction(&mut self, n: usize) -> Vec<f32> {
        let state = self.engine.state();
        (0..n).map(|i| state.gain_reduction(i)).collect()
    }

    /// Highest audio-callback DSP load since the last call (1.0 = full budget).
    pub fn take_dsp_load(&self) -> f32 {
        self.engine.shared().take_dsp_load()
    }

    pub fn master_peaks(&mut self) -> (f32, f32) {
        self.engine.state().peaks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Note, RegionParams, TrackKind, TrackParams};
    use ovrtone_engine::TempoMap;

    #[test]
    fn slide_notes_bend_what_is_sounding_and_let_go_at_its_end() {
        let note = |start: f32, len: f32, pitch: u8, slide: bool| Note {
            start,
            len,
            pitch,
            vel: 100,
            muted: false,
            color: 0,
            slide,
        };
        // Two up over a beat, within a note of four beats.
        let notes = [note(0.0, 4.0, 60, false), note(1.0, 1.0, 62, true)];
        assert_eq!(slides_of(&notes, false), [(1, 0)]);
        // The high part of each bend, which comes after its low part.
        let high = |events: &[ControlEvent]| -> Vec<(f64, u8)> {
            events
                .iter()
                .filter(|b| b.number == ovrtone_engine::PITCH_BEND_CONTROL)
                .map(|b| (b.beat, b.value))
                .collect()
        };
        let bends = slide_bends(&notes, 8.0, 3, 2.0, false, 16.0);
        assert_eq!(bends[0].number, ovrtone_engine::PITCH_BEND_FINE_CONTROL);
        assert_eq!(bends[0].track, 3);
        assert!(bends[0].beat > 9.0);
        let steps = high(&bends);
        assert_eq!(steps[steps.len() - 2], (10.0, 127));
        assert_eq!(steps[steps.len() - 1], (12.0, 64));
        // With nothing sounding it is an ordinary note.
        assert!(slides_of(&[note(1.0, 1.0, 62, true)], false).is_empty());
        assert!(slide_bends(&[note(1.0, 1.0, 62, true)], 0.0, 0, 2.0, false, 4.0).is_empty());
        // Further than the range goes as far as it can.
        let far = slide_bends(
            &[note(0.0, 2.0, 60, false), note(0.0, 1.0, 72, true)],
            0.0,
            0,
            2.0,
            false,
            4.0,
        );
        let steps = high(&far);
        assert_eq!(steps[steps.len() - 2].1, 127);
    }

    #[test]
    fn a_slide_bends_a_one_shot_after_its_short_note_and_lets_go_before_the_next() {
        let note = |start: f32, len: f32, pitch: u8, slide: bool| Note {
            start,
            len,
            pitch,
            vel: 100,
            muted: false,
            color: 0,
            slide,
        };
        // An 808: a short note that rings on, a slide two beats later, then another note.
        let notes = [
            note(0.0, 0.25, 36, false),
            note(2.0, 0.5, 24, true),
            note(4.0, 0.25, 36, false),
        ];
        assert!(slides_of(&notes, false).is_empty(), "nothing held");
        assert_eq!(slides_of(&notes, true), [(1, 0)]);
        let bends = slide_bends(&notes, 0.0, 0, 48.0, true, 8.0);
        let last = bends.last().unwrap();
        assert!(
            last.beat < 4.0 && last.beat > 3.9,
            "just before the next note"
        );
        assert_eq!(last.value, 64);
        // A quarter of the way down the range: 12 of 48 semitones, with 14 bits to say it in.
        let lowest = bends
            .iter()
            .filter(|b| b.number == ovrtone_engine::PITCH_BEND_CONTROL)
            .map(|b| b.value)
            .min();
        assert_eq!(lowest, Some(48));
    }

    #[test]
    fn the_cycle_loops_the_playhead_through_the_real_engine() {
        let mut p = Project::empty();
        p.tempo = 120.0;
        p.cycle = true;
        p.cycle_start = 0.0;
        p.cycle_end = 4.0;
        let (engine, mut core) = Engine::offline(48_000);
        let mut sync = EngineSync::new(engine, &p);
        sync.sync(&mut p);
        p.playing = true;
        sync.sync(&mut p);
        let mut buf = vec![0.0f32; 1024];
        let mut highest = 0f64;
        let mut wrapped = false;
        // Five seconds is ten beats: past the end of the cycle twice.
        for _ in 0..(48_000 * 5 / 512) {
            core.process(&mut buf);
            let before = p.position_beats;
            sync.sync(&mut p);
            highest = highest.max(p.position_beats);
            wrapped |= p.position_beats < before;
        }
        assert!(wrapped, "it went back to the start");
        assert!(highest < 4.1, "it never went past the end: {highest}");
    }

    fn track(notes: Vec<Note>) -> Track {
        Track {
            name: "t".into(),
            kind: TrackKind::Instrument,
            color: egui::Color32::GREEN,
            mute: false,
            solo: false,
            rec: false,
            monitor: false,
            hardware_out: 0,
            aux_bus: 0,
            object: None,
            bed_pan: false,
            trim_db: 0.0,
            slide_range: crate::model::DEFAULT_SLIDE_RANGE,
            input: ovrtone_engine::InputSource::Mono(0),
            output: 0,
            volume_db: 0.0,
            pan: 0.0,
            inserts: vec![],
            plugins: Vec::new(),
            instrument_plugin: None,
            regions: vec![Region {
                name: "r".into(),
                start_bar: 1.0,
                len_bars: 1.0,
                notes,
                controls: Vec::new(),
                folder: None,
                auto: Vec::new(),
                generated: None,
                gain_line: Vec::new(),
                audio: None,
                params: RegionParams::default(),
            }],
            params: TrackParams::default(),
            category: None,
            icon: None,
            frozen: None,
            sends: Vec::new(),
            auto: crate::model::Automation::default(),
            group: None,
            height: 1.0,
            indent: 0,
            folded: false,
            vca: None,
            on_timeline: false,
            off: false,
            locked: false,
        reference: false,
            midi_port: None,
            midi_channel: 1,
            midi_in_port: None,
            midi_in_channel: 0,
            cells: Vec::new(),
            launched: None,
            launch_loops: Vec::new(),
            midi_fx: Vec::new(),
            show_auto: false,
            alternatives: Vec::new(),
            active_alt: 0,
        }
    }

    fn note(start: f32, pitch: u8, vel: u8) -> Note {
        Note {
            start,
            len: 0.5,
            pitch,
            vel,
            muted: false,
            color: 0,
            slide: false,
        }
    }

    #[test]
    fn sends_aux_buses_and_automation_reach_the_engine_as_the_project_says() {
        use crate::model::{AutoMode, AutoTarget, SendSlot};
        let mut p = Project::demo();
        let a1 = p.add_track(TrackKind::Aux);
        let a2 = p.add_track(TrackKind::Aux);
        p.tracks[0].sends = vec![
            SendSlot {
                bus: Some(2),
                level: 0.5,
                pre: true,
            },
            SendSlot {
                bus: None,
                level: 0.9,
                pre: false,
            },
        ];
        // Aux strips take Bus 1, Bus 2... in order, and can send, only not into their own bus.
        p.tracks[a1].sends = p.tracks[0].sends.clone();
        assert_eq!(
            (p.aux_bus_of(a1), p.aux_bus_of(a2), p.aux_bus_of(0)),
            (1, 2, 0)
        );
        assert_eq!(route_of(&p, a2).bus_in, 2);
        assert!(route_of(&p, a1).has_sends(), "Bus 1 sends to Bus 2");
        p.tracks[a2].sends = p.tracks[0].sends.clone();
        assert!(!route_of(&p, a2).has_sends(), "Bus 2 cannot send to itself");
        p.tracks[a2].sends.clear();
        let r = route_of(&p, 0);
        assert_eq!((r.bus_in, r.sends[0].bus, r.sends[0].pre), (0, 2, true));
        assert_eq!(r.sends[1].bus, 0, "an empty slot sends nothing");
        // More than eight Aux strips: each still has a bus of its own.
        for _ in 0..8 {
            p.add_track(TrackKind::Aux);
        }
        assert_eq!(p.aux_bus_of(p.tracks.len() - 1), 10);

        // Automation: the lanes sent are what the project has, and the comparison knows it.
        assert!(automation_unchanged(&p, &[]));
        p.tracks[1].auto.mode = AutoMode::Read;
        p.tracks[1].auto.set_point(AutoTarget::Volume, 2.0, -9.0);
        p.tracks[1].auto.set_point(AutoTarget::Pan, 1.0, 0.25);
        let lanes = automation_lanes(&p);
        assert_eq!(lanes.len(), 2);
        assert!(lanes.iter().all(|l| l.track == 1 && l.follow));
        assert!(automation_unchanged(&p, &lanes));
        p.tracks[1].auto.set_point(AutoTarget::Volume, 3.0, -9.0);
        assert!(!automation_unchanged(&p, &lanes));
        let lanes = automation_lanes(&p);
        p.tracks[1].auto.mode = AutoMode::Write;
        assert!(!automation_unchanged(&p, &lanes), "no longer played");
        assert!(automation_lanes(&p).iter().all(|l| !l.follow));
        // A VCA's level is added to the lane.
        let lanes = automation_lanes(&p);
        let vca = p.add_track(TrackKind::Vca);
        p.tracks[vca].volume_db = -4.0;
        p.assign_vca(1, Some(vca));
        assert!(!automation_unchanged(&p, &lanes));
        assert_eq!(automation_lanes(&p)[0].offset_db, -4.0);
        // The effects as the audio thread gets them.
        let chain = chain_of(&p.tracks[0].inserts, 48_000.0);
        assert_eq!(chain.fx.len(), p.tracks[0].inserts.len());
    }

    #[test]
    fn a_look_ahead_effect_delays_the_other_strips_so_everything_stays_in_time() {
        use crate::model::Insert;
        use ovrtone_engine::FxKind;
        let sr = 48_000.0;
        let mut p = Project::empty();
        let a = p.add_track(TrackKind::Audio);
        let b = p.add_track(TrackKind::Audio);
        let aux = p.add_track(TrackKind::Aux);
        let vca = p.add_track(TrackKind::Vca);
        // Nothing looks ahead: nothing is delayed.
        assert!(pdc_plan(&p, sr, &Rack::default())
            .iter()
            .all(|d| *d == (0, 0)));
        let look = FxKind::Limiter.latency_samples(sr);
        assert!(look > 0);
        p.tracks[a].inserts = vec![Insert::new(FxKind::Limiter)];
        let plan = pdc_plan(&p, sr, &Rack::default());
        assert_eq!(latency_of(&p.tracks[a], sr, &Rack::default()), look);
        // The limiter's own strip is the latest: the others wait for it before their sends
        // and faders; none of them needs more after.
        assert_eq!(plan[a], (0, 0));
        assert_eq!(plan[b], (look, 0));
        assert_eq!(plan[0], (look, 0));
        assert_eq!(plan[vca], (0, 0), "a VCA has no sound");
        // A bypassed limiter delays nothing.
        p.tracks[a].inserts[0].bypass = true;
        assert!(pdc_plan(&p, sr, &Rack::default())
            .iter()
            .all(|d| *d == (0, 0)));
        p.tracks[a].inserts[0].bypass = false;
        // A limiter on an Aux strip: every other strip is delayed after its fader, by as much
        // as the Aux strip is late; the Aux strip itself needs nothing.
        p.tracks[aux].inserts = vec![Insert::new(FxKind::Limiter)];
        let plan = pdc_plan(&p, sr, &Rack::default());
        assert_eq!(plan[aux], (0, 0));
        assert_eq!(plan[b], (look, look));
        assert_eq!(plan[a], (0, look));
        // Two limiters in a row add up.
        p.tracks[b].inserts = vec![Insert::new(FxKind::Limiter), Insert::new(FxKind::Limiter)];
        assert_eq!(latency_of(&p.tracks[b], sr, &Rack::default()), 2 * look);
        assert_eq!(pdc_plan(&p, sr, &Rack::default())[a].0, look);
    }

    #[test]
    fn region_automation_reaches_the_engine_and_is_not_sent_again_and_again() {
        use crate::model::{AutoMode, RegionLane, TrackKind};
        let mut p = Project::empty();
        let t = p.add_track(TrackKind::Instrument);
        p.add_midi_region(t, 3.0);
        p.tracks[t].auto.mode = AutoMode::Read;
        p.tracks[t].regions[0].auto.push(RegionLane {
            target: AutoTarget::Volume,
            points: vec![ovrtone_engine::AutoPoint {
                beat: 1.0,
                value: -6.0,
                curve: 0.0,
            }],
        });
        let lanes = automation_lanes(&p);
        let lane = lanes.iter().find(|l| l.track as usize == t);
        let Some(lane) = lane else {
            panic!("no lane for the track");
        };
        let start = p.region_start_beats(&p.tracks[t].regions[0]);
        let inside = ovrtone_engine::lane_value(&lane.points, start + 1.0);
        assert!(inside.is_some_and(|v| (v + 6.0).abs() < 0.01));
        assert!(automation_unchanged(&p, &lanes));
        p.tracks[t].regions[0].start_bar += 1.0;
        assert!(!automation_unchanged(&p, &lanes), "a move is a change");
    }

    #[test]
    fn the_rows_under_more_change_the_notes_as_they_play() {
        use crate::model::{Note, TrackKind};
        let mut p = Project::empty();
        let i = p.add_track(TrackKind::Instrument);
        p.add_midi_region(i, 1.0);
        let t = &mut p.tracks[i];
        let note = |start: f32, pitch: u8, vel: u8| Note {
            start,
            len: 0.3,
            pitch,
            vel,
            muted: false,
            color: 0,
            slide: false,
        };
        t.regions[0].notes = vec![note(0.1, 60, 40), note(0.1, 64, 100), note(0.9, 67, 100)];
        t.regions[0].params.quantize = Some(1.0);
        let play = |t: &Track| region_notes(t, &t.regions[0], 0.0, 0);
        let plain = play(t);
        assert!(plain[0].start_beat.abs() < 1e-6);
        // Half strength: half way to the grid.
        t.regions[0].params.shape.q_soft = 50;
        assert!((play(t)[0].start_beat - 0.05).abs() < 1e-5);
        t.regions[0].params.shape.q_soft = 0;
        // Range: only what is near the grid moves.
        t.regions[0].params.shape.q_range = 5;
        assert!((play(t)[0].start_beat - 0.1).abs() < 1e-5);
        t.regions[0].params.shape.q_range = 0;
        // Flam: notes on one place are spread.
        t.regions[0].params.shape.q_flam = 10;
        let flam = play(t);
        assert!((flam[1].start_beat - flam[0].start_beat - 0.1).abs() < 1e-5);
        t.regions[0].params.shape.q_flam = 0;
        // Length, gate, velocity and dynamics.
        t.regions[0].params.shape.q_length = 100;
        let n = play(t)[0];
        assert!(
            ((n.end_beat - n.start_beat) - 1.0).abs() < 1e-5,
            "a whole step of the grid"
        );
        t.regions[0].params.shape.q_length = 0;
        t.regions[0].params.shape.gate = -50;
        let n = play(t)[0];
        assert!(((n.end_beat - n.start_beat) - 0.15).abs() < 1e-5);
        t.regions[0].params.shape.gate = 0;
        t.regions[0].params.shape.q_velocity = 100;
        assert_eq!(play(t)[0].vel, 80, "the average");
        t.regions[0].params.shape.q_velocity = 0;
        t.regions[0].params.shape.dynamics = 50;
        assert_eq!(play(t)[1].vel, 118, "further from the middle");
        t.regions[0].params.shape.dynamics = 0;
        // Clip Length.
        t.regions[0].params.shape.clip_length = true;
        let clipped = clip_notes(play(t), &t.regions[0], 0.2);
        assert_eq!(clipped.len(), 2);
        assert!(clipped.iter().all(|n| n.end_beat <= 0.2));
    }

    #[test]
    fn a_frozen_track_plays_its_file_and_not_its_notes_or_effects() {
        use crate::model::{Note, TrackKind};
        let mut p = Project::empty();
        let t = p.add_track(TrackKind::Instrument);
        p.add_midi_region(t, 1.0);
        p.tracks[t].regions[0].notes = vec![Note {
            start: 0.0,
            len: 1.0,
            pitch: 60,
            vel: 100,
            muted: false,
            color: 0,
            slide: false,
        }];
        assert_eq!(song_notes(&p).len(), 1);
        p.tracks[t].frozen = Some(Box::new(p.tracks[t].regions[0].clone()));
        assert!(song_notes(&p).is_empty(), "the notes stay quiet");
        assert_eq!(playing_regions(&p).count(), 1, "only the frozen file");
        p.tracks[t].frozen = None;
        assert_eq!(song_notes(&p).len(), 1, "it thaws as it was");
    }

    #[test]
    fn quantize_transpose_and_velocity_are_applied() {
        let mut t = track(vec![note(0.9, 60, 100)]);
        t.regions[0].params = RegionParams {
            mute: false,
            quantize: Some(0.25),
            transpose: 12,
            velocity: -10,
            ..RegionParams::default()
        };
        t.params.transpose = -2;
        let out = region_notes(&t, &t.regions[0], 4.0, 3);
        assert_eq!(out.len(), 1);
        assert!(
            (out[0].start_beat - 5.0).abs() < 1e-6,
            "{}",
            out[0].start_beat
        );
        assert_eq!((out[0].pitch, out[0].vel, out[0].track), (70, 90, 3));
    }

    #[test]
    fn key_and_velocity_limits_filter_notes() {
        let mut t = track(vec![
            note(0.0, 40, 100),
            note(1.0, 60, 100),
            note(2.0, 60, 20),
        ]);
        t.params.key_limit = (48, 72);
        t.params.vel_limit = (30, 127);
        let out = region_notes(&t, &t.regions[0], 0.0, 0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].pitch, 60);
        assert!((out[0].start_beat - 1.0).abs() < 1e-6);
    }

    #[test]
    fn velocity_is_clamped_and_out_of_range_pitch_dropped() {
        let mut t = track(vec![note(0.0, 120, 120), note(1.0, 60, 120)]);
        t.regions[0].params.transpose = 12;
        t.regions[0].params.velocity = 50;
        let out = region_notes(&t, &t.regions[0], 0.0, 0);
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].pitch, out[0].vel), (72, 127));
    }

    #[test]
    fn regions_start_where_the_engine_puts_the_bar_in_any_signature() {
        let mut p = Project::demo();
        p.tracks.truncate(1);
        p.tracks[0].regions.truncate(1);
        p.tracks[0].regions[0].params.mute = false;
        p.tracks[0].regions[0].start_bar = 3.0;
        for (sig, quarters_per_bar) in [((4, 4), 4.0), ((6, 8), 3.0), ((7, 8), 3.5), ((2, 2), 4.0)]
        {
            p.sig = sig;
            assert_eq!(p.bar_len(0), quarters_per_bar, "{sig:?}");
            let (_, _, _, origin) = playing_regions(&p).next().unwrap();
            // Bar 3 (1-based) is the engine's bar 2 (0-based).
            let engine_beat = TempoMap::constant(p.tempo, sig).beat_of_bar(2);
            assert!(
                (origin - engine_beat).abs() < 1e-9,
                "{sig:?}: {origin} vs {engine_beat}"
            );
        }
    }

    #[test]
    fn external_tracks_send_the_controllers_recorded_in_their_regions() {
        use crate::model::{Control, TrackKind};
        let mut p = Project::demo();
        let ext = p.add_track(TrackKind::ExternalMidi);
        let plain = p.add_track(TrackKind::Instrument);
        for t in [ext, plain] {
            p.tracks[t].regions.push(crate::model::Region {
                name: "r".into(),
                start_bar: 3.0,
                len_bars: 2.0,
                notes: vec![],
                controls: vec![
                    Control {
                        start: 1.0,
                        num: 64,
                        val: 127,
                    },
                    Control {
                        start: 2.0,
                        num: 64,
                        val: 0,
                    },
                    Control {
                        start: 0.5,
                        num: 1,
                        val: 200,
                    },
                ],
                folder: None,
                audio: None,
                params: RegionParams::default(),
                auto: Vec::new(),
                generated: None,
                gain_line: Vec::new(),
            });
        }
        let c = song_controls(&p);
        assert_eq!(c.len(), 3, "only the External MIDI track: {c:?}");
        assert!(c.iter().all(|e| e.track == ext as u16));
        // Bar 3 starts at beat 8.
        let mut beats: Vec<f64> = c.iter().map(|e| e.beat).collect();
        beats.sort_by(f64::total_cmp);
        assert_eq!(beats, [8.5, 9.0, 10.0]);
        assert!(c.iter().all(|e| e.value <= 127), "wild values are repaired");
        p.tracks[ext].regions[0].params.mute = true;
        assert!(song_controls(&p).is_empty(), "a muted region sends nothing");
        // A hosted instrument gets them too: the piano roll's controller lane draws for it.
        p.tracks[plain].instrument_plugin = Some(crate::model::PluginInsert {
            row: 0,
            uid: 5,
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
            window_at: None,
            smart_knobs: None,
            smart_ranges: None,
        });
        let c = song_controls(&p);
        assert_eq!(c.len(), 3);
        assert!(c.iter().all(|e| e.track == plain as u16));
    }

    #[test]
    fn a_tracks_midi_fx_change_the_notes_the_engine_gets() {
        use crate::midi_fx::{FxSlot, MidiFx};
        let mut p = Project::demo();
        let plain = song_notes(&p);
        p.tracks[0].midi_fx = vec![FxSlot {
            fx: MidiFx::Transposer { semitones: 7 },
            on: true,
        }];
        let fx = song_notes(&p);
        assert_eq!(plain.len(), fx.len());
        let first =
            |notes: &[NoteEvent]| notes.iter().filter(|n| n.track == 0).map(|n| n.pitch).max();
        assert_eq!(first(&fx), first(&plain).map(|x| x + 7));
        // The other tracks are as they were.
        let other = |notes: &[NoteEvent]| notes.iter().filter(|n| n.track == 1).count();
        assert_eq!(other(&fx), other(&plain));
        // Switched off, the notes are the original ones.
        p.tracks[0].midi_fx[0].on = false;
        assert_eq!(song_notes(&p).len(), plain.len());
        assert_eq!(first(&song_notes(&p)), first(&plain));
    }

    #[test]
    fn regions_and_the_engine_agree_on_bars_with_several_signatures() {
        let mut p = Project::demo();
        p.set_signature_from(3, (3, 4));
        p.set_signature_from(5, (6, 8));
        p.set_signature_from(7, (7, 8));
        p.set_signature_from(10, (5, 4));
        let map = p.tempo_map();
        for bar in 0..14u32 {
            let engine = map.beat_of_bar(bar);
            let app = p.bar_to_beats(bar as f64);
            assert!((engine - app).abs() < 1e-9, "bar {bar}: {engine} vs {app}");
        }
        for beats in [0.0, 3.9, 8.0, 12.5, 14.0, 20.0, 23.4, 30.0, 41.0] {
            let pos = map.position(beats);
            let bars = p.beats_to_bar(beats);
            assert_eq!(pos.bar as f64, bars.floor(), "bar of {beats}");
        }
        // A change of signature is sent to the engine as a change of the tempo map.
        let before = tempo_key(&p);
        p.set_signature_from(12, (2, 4));
        assert_ne!(tempo_key(&p), before);
    }

    #[test]
    fn editing_the_signature_repositions_the_notes_sent_to_the_engine() {
        let mut p = Project::demo();
        p.tracks.truncate(1);
        p.tracks[0].regions.truncate(1);
        p.tracks[0].regions[0].params.mute = false;
        p.tracks[0].regions[0].start_bar = 3.0;
        let first = |p: &Project| {
            song_notes(p)
                .iter()
                .map(|n| n.start_beat)
                .fold(f64::MAX, f64::min)
        };
        let (before, rev) = (first(&p), p.song_rev);
        // 4/4 -> 3/4: bar 3 starts at quarter note 6 instead of 8, so every note moves 2 earlier.
        assert!(p.step_signature(crate::model::SigPart::Numerator, -1));
        assert!(
            (before - first(&p) - 2.0).abs() < 1e-9,
            "{before} -> {}",
            first(&p)
        );
        assert_ne!(p.song_rev, rev, "the engine is told to rebuild the song");
    }

    #[test]
    fn bar_display_counts_signature_beats() {
        let mut p = Project::demo();
        // 4.75 quarter notes: bar 2 + 1.75 quarters.
        assert_eq!(p.bbdt(4.75), (2, 1, 4, 1));
        // In 6/8 a bar is 3 quarters: bar 2 + 1.75 quarters = 3.5 eighths -> beat 4, 16th 2.
        p.sig = (6, 8);
        assert_eq!(p.bbdt(4.75), (2, 4, 2, 1));
        p.position_beats = 4.75;
        assert_eq!(p.bar_beat_tick(), (2, 4, 480));
    }

    #[test]
    fn tracks_past_the_engine_limit_are_left_out_not_aliased() {
        let mut p = Project::demo();
        p.tracks.truncate(1);
        p.tracks[0].regions.truncate(1);
        p.tracks[0].regions[0].params.mute = false;
        p.tracks = vec![p.tracks[0].clone(); MAX_TRACKS + 3];
        let notes = song_notes(&p);
        let per_track = notes.iter().filter(|n| n.track == 0).count();
        assert!(per_track > 0);
        assert_eq!(notes.len(), per_track * MAX_TRACKS);
        assert_eq!(
            notes
                .iter()
                .filter(|n| usize::from(n.track) == MAX_TRACKS - 1)
                .count(),
            per_track,
            "the last engine track must not collect notes from later tracks"
        );
        assert_eq!(mixes(&p).len(), MAX_TRACKS);
        assert_eq!(track_index(MAX_TRACKS - 1), Some((MAX_TRACKS - 1) as u16));
        assert_eq!(track_index(MAX_TRACKS), None);
    }

    #[test]
    fn only_audio_changes_need_a_full_song_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("loop.wav");
        crate::project_io::tests::write_wav(&wav);
        let ctx = egui::Context::default();
        let mut p = Project::demo();
        crate::import::import_audio(&mut p, &wav, &ctx).unwrap();
        let before = audio_layout(&p);
        assert_eq!(before.clips.len(), 1);

        // MIDI edits and mixer moves keep the streamed audio.
        p.tracks[0].regions[0].notes[0].pitch += 1;
        p.tracks[0].regions[0].start_bar += 1.0;
        p.tracks[1].volume_db = -12.0;
        assert_eq!(audio_layout(&p), before);

        // Moving or muting the audio region, or adding a track, does not.
        let audio = p.tracks.len() - 1;
        p.tracks[audio].regions[0].start_bar += 1.0;
        assert_ne!(audio_layout(&p), before);
        p.tracks[audio].regions[0].start_bar -= 1.0;
        p.tracks[audio].regions[0].params.mute = true;
        assert_ne!(audio_layout(&p), before);
        p.tracks[audio].regions[0].params.mute = false;
        // Nor do gain, fades or reverse, which a clip gets when it starts streaming.
        for edit in [
            |r: &mut RegionParams| r.gain_db = -6.0,
            |r: &mut RegionParams| r.fade_out = 1.0,
            |r: &mut RegionParams| r.reverse = true,
        ] {
            let mut q = Project::demo();
            q.tracks = p.tracks.clone();
            let before = audio_layout(&q);
            edit(&mut q.tracks[audio].regions[0].params);
            assert_ne!(audio_layout(&q), before);
        }
        p.tracks.push(p.tracks[0].clone());
        assert_ne!(audio_layout(&p), before);
    }

    #[test]
    fn record_with_count_in_clicks_one_bar_of_the_signature() {
        let mut p = Project::empty();
        assert!(matches!(start_command(&p), Command::Play), "not recording");
        p.recording = true;
        p.count_in = false;
        assert!(matches!(start_command(&p), Command::Play), "count-in off");
        p.count_in = true;
        p.sig = (6, 8);
        match start_command(&p) {
            Command::CountIn {
                clicks,
                beats_per_click,
            } => assert_eq!((clicks, beats_per_click), (6, 0.5)),
            _ => panic!("no count-in"),
        }
    }

    #[test]
    fn the_i_button_makes_a_monitor_of_the_tracks_input() {
        let mut p = Project::empty();
        p.tracks.clear();
        p.tracks.push(Track::new("Keys", TrackKind::Instrument));
        p.tracks.push(Track::new("Vox", TrackKind::Audio));
        p.tracks[0].monitor = true;
        assert!(monitors_of(&p).is_empty(), "audio tracks only");
        p.tracks[1].monitor = true;
        p.tracks[1].input = InputSource::Stereo(2);
        assert_eq!(
            monitors_of(&p),
            vec![MonitorTrack {
                track: 1,
                channel: 2,
                stereo: true
            }]
        );
    }

    #[test]
    fn a_track_or_an_aux_can_play_into_a_bus_but_not_the_one_it_listens_to() {
        let mut p = Project::empty();
        p.tracks.clear();
        p.tracks.push(Track::new("Vox", TrackKind::Audio));
        p.tracks.push(Track::new("Aux 1", TrackKind::Aux));
        p.tracks[0].output = 3;
        p.tracks[1].output = 3;
        assert_eq!(route_of(&p, 0).output, 3);
        assert_eq!(route_of(&p, 1).output, 3, "an Aux can feed another");
        // Into itself would go round for ever: it plays to Stereo Out instead.
        p.tracks[1].output = p.aux_bus_of(1);
        assert_eq!(route_of(&p, 1).output, 0);
    }
}
