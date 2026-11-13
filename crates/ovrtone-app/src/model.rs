//! Project model. UI never mutates the engine directly (see AGENTS.md: Command pattern);
//! this is the UI-side view model until `ovrtone-engine` lands.
// Some fields (pan, instrument, Bus/Output kinds) are reserved for upcoming mixer/instrument work.
#![allow(dead_code)]
use crate::pattern::{drummer_notes, BarSpec, DrumStyle, DrummerParams, PatternParams};
use egui::Color32;
use ovrtone_engine::AudioFile;
use std::sync::{Arc, OnceLock};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrackKind {
    Instrument,
    Audio,
    Drummer,
    Bus,
    Output,
    /// Auxiliary channel strip: gets its sound from a bus, so it has no regions.
    Aux,
    /// Folder (track stack): holds the tracks nested under it and makes no sound itself.
    Folder,
    /// VCA: a fader with Mute and Solo that moves the level of the tracks assigned to it. It
    /// has no sound or regions of its own.
    Vca,
    /// External MIDI: its regions play on a MIDI port and channel (a synthesizer outside the
    /// app) instead of on a software instrument.
    ExternalMidi,
}

impl TrackKind {
    /// Whether tracks of this kind hold regions (Aux and Folder tracks do not).
    pub fn has_regions(self) -> bool {
        matches!(
            self,
            Self::Instrument | Self::Audio | Self::Drummer | Self::ExternalMidi
        )
    }
}

/// The bend range most instruments start with.
pub const DEFAULT_SLIDE_RANGE: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Note {
    pub start: f32, // beats from region start
    pub len: f32,
    pub pitch: u8,
    pub vel: u8,
    /// It stays in the region but does not play.
    pub muted: bool,
    /// Its colour group, 0..16, as in FL Studio. Selects notes together; changes no sound.
    pub color: u8,
    /// A slide note, as in FL Studio: it does not start a note, but bends what is sounding to its
    /// pitch over its length. With nothing sounding it plays as an ordinary note.
    pub slide: bool,
}

/// One pass of a cycle recording.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Take {
    pub notes: Vec<Note>,
    pub controls: Vec<Control>,
    /// A pass of an audio recording: seconds into the region's file where the region's start falls
    /// for this take. Below 0 when the recording began after the start of the cycle. All the takes
    /// of a folder are windows into the one file that was recorded.
    pub audio_offset: Option<f32>,
    /// A take that came from another recording: its own file. `None` is the region's.
    pub file: Option<TakeFile>,
    /// What of its file it may play, in seconds (a region that was trimmed before it became a take
    /// keeps hiding the rest). `None` is the whole file.
    pub window: Option<(f32, f32)>,
    /// The gain it had as a region, in dB, on top of the folder's.
    pub gain_db: f32,
    /// The name it shows in its row (empty: "Take n").
    pub name: String,
    /// The colour it is drawn in, when the user gave it one.
    pub color: Option<egui::Color32>,
}

/// The file of a take, with its waveform. Two are the same when they read the same file.
#[derive(Clone)]
pub struct TakeFile(pub AudioRegion);

impl PartialEq for TakeFile {
    fn eq(&self, other: &Self) -> bool {
        self.0.file.path == other.0.file.path
    }
}

impl std::fmt::Debug for TakeFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TakeFile({})", self.0.file.path.display())
    }
}

/// A stretch of a take folder (beats from the region start) that plays this take.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompSegment {
    pub start: f32,
    pub end: f32,
    pub take: usize,
}

/// A comp kept under a name, to go back to.
#[derive(Clone, Debug, PartialEq)]
pub struct SavedComp {
    pub name: String,
    pub comp: Vec<CompSegment>,
}

/// Logic's take folder: several takes of the same passage and the comp that plays one of
/// them in each stretch. See `takes.rs`.
#[derive(Clone, Debug, PartialEq)]
pub struct TakeFolder {
    pub takes: Vec<Take>,
    /// Sorted, without gaps or overlaps, covering `0..len`.
    pub comp: Vec<CompSegment>,
    /// Length of the folder in beats.
    pub len: f32,
    /// Shown opened, with a row for each take (not saved).
    pub open: bool,
    /// Comps kept to go back to.
    pub saved: Vec<SavedComp>,
    /// One take is playing throughout to be listened to, and this is the comp to return to (not
    /// saved).
    pub audition: Option<(usize, Vec<CompSegment>)>,
}

/// A MIDI controller event in a region (the sustain pedal is controller 64).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Control {
    pub start: f32, // beats from region start
    pub num: u8,
    pub val: u8,
}

#[derive(Clone)]
pub struct Region {
    pub name: String,
    pub start_bar: f32,
    pub len_bars: f32,
    pub notes: Vec<Note>,
    /// Controller events recorded with the notes.
    pub controls: Vec<Control>,
    /// A take folder: the takes of a cycle recording and the comp that picks among them. The
    /// region's own `notes` and `controls` are then the comp, kept up to date by
    /// [`TakeFolder::apply`].
    pub folder: Option<TakeFolder>,
    /// Set for audio regions backed by a file on disk.
    pub audio: Option<AudioRegion>,
    /// Region inspector parameters (non-destructive, applied when the song is built).
    pub params: RegionParams,
    /// Automation that belongs to the region and moves with it. Over the region's stretch a lane
    /// here takes the place of the track's lane of the same control.
    pub auto: Vec<RegionLane>,
    /// A Pattern or Drummer region: the settings its `notes` are made from.
    pub generated: Option<Generated>,
    /// Clip gain as a line (audio regions): (beats from the region's start, dB), in order.
    pub gain_line: Vec<(f32, f32)>,
}

/// What makes the notes of a Pattern or Drummer region (see `pattern.rs`).
#[derive(Clone, Debug, PartialEq)]
pub enum Generator {
    Pattern(PatternParams),
    Drummer(DrummerParams),
}

/// A region whose notes are made from settings. The notes are made again whenever the
/// settings, the length of the region or the bars under it change.
#[derive(Clone, Debug, PartialEq)]
pub struct Generated {
    pub source: Generator,
    /// What the notes were last made for: the bars the region covers (see
    /// [`Project::generated_key`]).
    pub made_for: u64,
    /// The region's start, length and the song revision when `made_for` was last compared with
    /// the bars, so a frame in which none changed does not look at the bars again.
    pub seen: (u32, u32, u64),
}

impl Generated {
    pub fn new(source: Generator) -> Self {
        Self {
            source,
            made_for: 0,
            seen: (0, 0, 0),
        }
    }
}

/// The colours offered for tracks and regions: strong ones, then the same lighter, then grey.
pub const TRACK_COLORS: [Color32; 24] = [
    Color32::from_rgb(0xd9, 0x4f, 0x4f),
    Color32::from_rgb(0xe0, 0x82, 0x3a),
    Color32::from_rgb(0xd9, 0xb4, 0x3a),
    Color32::from_rgb(0x8f, 0xb8, 0x3f),
    Color32::from_rgb(0x3f, 0xa8, 0x5a),
    Color32::from_rgb(0x2f, 0xa8, 0x9a),
    Color32::from_rgb(0x3f, 0x9f, 0xd0),
    Color32::from_rgb(0x3f, 0x5f, 0xc0),
    Color32::from_rgb(0x7a, 0x4f, 0xc0),
    Color32::from_rgb(0xb0, 0x4f, 0xb8),
    Color32::from_rgb(0xd0, 0x5a, 0x8f),
    Color32::from_rgb(0xec, 0x8f, 0x8f),
    Color32::from_rgb(0xf0, 0xb2, 0x82),
    Color32::from_rgb(0xec, 0xd5, 0x82),
    Color32::from_rgb(0xbf, 0xd9, 0x85),
    Color32::from_rgb(0x85, 0xcf, 0x98),
    Color32::from_rgb(0x7c, 0xcf, 0xc5),
    Color32::from_rgb(0x8a, 0xc6, 0xe5),
    Color32::from_rgb(0x8a, 0x9f, 0xe0),
    Color32::from_rgb(0xae, 0x92, 0xdf),
    Color32::from_rgb(0xd3, 0x92, 0xd8),
    Color32::from_rgb(0xe6, 0x9b, 0xbd),
    Color32::from_rgb(0x8a, 0x8a, 0x8a),
    Color32::from_rgb(0x5a, 0x5a, 0x5a),
];
/// New tracks take from the colours before the greys.
const AUTO_TRACK_COLORS: usize = 22;

/// Most bars the notes of a generated region are made for.
const MAX_GENERATED_BARS: usize = 1024;

/// Logic Pro Region Inspector values.
#[derive(Clone, Copy, PartialEq, Default)]
pub struct RegionParams {
    /// The Channel Rack pattern this region is placed from (see `song_pattern.rs`); 0 for none.
    pub pattern: u64,
    /// How long the region was when last made from its pattern, in beats. Not saved.
    pub pattern_len: f32,
    /// Its pattern's color, which it is drawn in. Not saved.
    pub pattern_color: Option<Color32>,
    /// Its own colour, instead of its track's.
    pub color: Option<Color32>,
    /// The clip group it is in, 0 for none. A click on one selects them all, so they move together.
    pub group: u32,
    pub mute: bool,
    /// The rows under More in the Region Inspector.
    pub shape: ovrtone_project::RegionShape,
    /// Playback quantize grid in beats (`None` = Off).
    pub quantize: Option<f32>,
    /// Semitones.
    pub transpose: i8,
    /// Added to every note velocity.
    pub velocity: i8,
    /// Audio regions: where in the file this one begins, and how much of the file it plays
    /// (0: on to the end), in seconds of the file. In seconds, so a change of tempo does not cut
    /// a clip short.
    pub source_start: f32,
    pub source_len: f32,
    /// Audio regions (Logic: Region Inspector Gain, Fade In / Out, Curve, Reverse), applied
    /// non-destructively by the engine. Gain in dB; fades in quarter-note beats.
    pub gain_db: f32,
    pub fade_in: f32,
    pub fade_out: f32,
    pub fade_curve: ovrtone_engine::FadeCurve,
    pub reverse: bool,
    /// Crossfade with the audio region this one overlaps (Logic: Drag > X-Fade): it fades in
    /// over the overlap while the earlier region fades out, with the region's curve.
    pub crossfade: bool,
}

/// Logic Pro's region gain range, in dB.
pub const MIN_REGION_GAIN: f32 = -96.0;
pub const MAX_REGION_GAIN: f32 = 24.0;

impl RegionParams {
    /// The audio edits as the engine applies them.
    pub fn clip_edit(&self) -> ovrtone_engine::ClipEdit {
        ovrtone_engine::ClipEdit {
            gain_db: self.gain_db,
            fade_in_beats: self.fade_in as f64,
            fade_out_beats: self.fade_out as f64,
            fade_curve: self.fade_curve,
            reverse: self.reverse,
            source_start_secs: f64::from(self.source_start),
            length_secs: f64::from(self.source_len),
            ..Default::default()
        }
    }

    /// Region gain that brings a file whose loudest sample is `peak` (0..1) to 0 dBFS
    /// (Logic: Normalize Region Gain). `None` for a silent file.
    pub fn normalize_gain(peak: f32) -> Option<f32> {
        (peak.is_finite() && peak > 1e-6)
            .then(|| (-20.0 * peak.log10()).clamp(MIN_REGION_GAIN, MAX_REGION_GAIN))
    }
}

/// The place the effect windows use for Stereo Out's own effects.
pub const MASTER_STRIP: usize = usize::MAX;

impl Project {
    /// The strip at `i`: a track, or [`MASTER_STRIP`] for Stereo Out's own effects.
    pub fn strip(&self, i: usize) -> Option<&Track> {
        if i == MASTER_STRIP {
            Some(&self.master_strip)
        } else {
            self.tracks.get(i)
        }
    }

    pub fn strip_mut(&mut self, i: usize) -> Option<&mut Track> {
        if i == MASTER_STRIP {
            Some(&mut self.master_strip)
        } else {
            self.tracks.get_mut(i)
        }
    }
}

/// A spatial bus of the project.
#[derive(Clone, Debug, PartialEq)]
pub struct SpatialBusDef {
    pub name: String,
    pub layout: ovrtone_engine::SpeakerLayout,
    pub place: ovrtone_engine::BusPlace,
}

/// One send slot of a channel strip (Logic: "Bus 1" with a level knob). The engine copies the
/// strip's signal to the bus; the Aux strip that listens to the bus plays it.
#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub struct SendSlot {
    /// Bus number from 1, or `None` for an empty slot.
    pub bus: Option<u16>,
    /// 0..=1 send level (the knob; see `ovrtone_engine::send_gain`).
    pub level: f32,
    /// Pre-fader send (shown with the "eye" icon).
    pub pre: bool,
}

/// Most sends in one channel strip.
pub const MAX_SENDS: usize = ovrtone_engine::MAX_SENDS;

/// One effect in a channel strip's Audio FX slot (Logic: a plug-in): what it is, whether it is
/// bypassed, and its settings in the units of `FxKind::params`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Insert {
    pub kind: ovrtone_engine::FxKind,
    pub bypass: bool,
    pub params: [f32; ovrtone_engine::MAX_FX_PARAMS],
    /// The track whose sound keys it (a side chain), for the effects that take one.
    pub sidechain: Option<usize>,
}

impl Insert {
    /// A new effect of `kind` with its default settings.
    pub fn new(kind: ovrtone_engine::FxKind) -> Self {
        Self {
            kind,
            bypass: false,
            params: kind.defaults(),
            sidechain: None,
        }
    }

    pub fn name(&self) -> &'static str {
        self.kind.name()
    }
}

/// The effects a Library patch names (names that are no effect here are left out).
pub fn inserts_from_names(names: &[&str]) -> Vec<Insert> {
    names
        .iter()
        .filter_map(|n| ovrtone_engine::FxKind::from_name(n))
        .map(Insert::new)
        .take(ovrtone_engine::MAX_INSERTS)
        .collect()
}

/// Logic's automation modes of a channel strip.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum AutoMode {
    Off,
    /// Plays the automation back.
    #[default]
    Read,
    /// Plays it back until the control is touched, writes while it is held, and returns to
    /// the automation when it is let go.
    Touch,
    /// Like Touch, but keeps writing after the control is let go, until playback stops.
    Latch,
    /// Writes the control's value all the time while playing.
    Write,
    /// While a control is held, moves the curve that is there by as much as the control moved,
    /// keeping its shape. Like Touch on a plugin's parameters.
    Trim,
}

impl AutoMode {
    pub const ALL: [AutoMode; 6] = [
        AutoMode::Off,
        AutoMode::Read,
        AutoMode::Touch,
        AutoMode::Latch,
        AutoMode::Write,
        AutoMode::Trim,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AutoMode::Off => "Off",
            AutoMode::Read => "Read",
            AutoMode::Touch => "Touch",
            AutoMode::Latch => "Latch",
            AutoMode::Write => "Write",
            AutoMode::Trim => "Trim",
        }
    }
}

/// What an automation lane moves and its points (the engine's own types).
pub use ovrtone_engine::{AutoPoint, AutoTarget};

/// A lane of a region's own automation: its points count their beats from the region's start.
#[derive(Clone, PartialEq, Debug)]
pub struct RegionLane {
    pub target: AutoTarget,
    pub points: Vec<AutoPoint>,
}

/// The automation of one effect parameter: slot and parameter number, and the points, whose
/// values are positions 0..=1 along the parameter's range.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct FxLane {
    pub slot: u8,
    pub param: u8,
    pub points: Vec<AutoPoint>,
}

/// The automation lane of a parameter of a hosted plugin: values are positions 0..=1.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct PluginLane {
    pub uid: u64,
    pub param: u32,
    pub points: Vec<AutoPoint>,
}

/// Automation of one channel strip: a lane for the fader (dB), one for the pan knob and one
/// for each effect parameter, as points in order of beat, and what the strip is doing with them.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Automation {
    pub mode: AutoMode,
    pub volume: Vec<AutoPoint>,
    pub pan: Vec<AutoPoint>,
    pub fx: Vec<FxLane>,
    pub plugin_lanes: Vec<PluginLane>,
    /// The lanes of a track's place as an object, by [`ovrtone_engine::ObjectAxis`].
    pub object: [Vec<AutoPoint>; 9],
    /// The control held (Touch), or held and let go but still writing (Latch): the strip
    /// plays that control's own value, not its lane. Not saved.
    pub held: Option<AutoTarget>,
    /// The lane the Automation editor shows, and Write mode writes. Not saved.
    pub shown: AutoTarget,
    /// Touch on a plugin: the beat it lets go at, if no move comes before. Not saved.
    pub plugin_touch_until: f64,
    /// What the strip plays where regions have lanes, by control: see
    /// `automation::merged_lanes`. Made once a frame, for the fader and knob display. Not saved.
    pub region_played: Vec<(AutoTarget, Vec<AutoPoint>)>,
    /// Trim mode with a control held: the curve as it was, and how far it has been moved. Not saved.
    pub trim: Option<(Vec<AutoPoint>, f32)>,
    /// The beat and value last written in this pass, so the next write replaces what lies
    /// between. Not saved.
    pub last: Option<(f64, f32)>,
}

/// Points closer than this (in beats) are written as one unless the value moves.
const WRITE_STEP: f64 = 0.125;

impl Automation {
    /// The points of the lane of `target` (none if it has no lane).
    pub fn lane(&self, target: AutoTarget) -> &[AutoPoint] {
        match target {
            AutoTarget::Volume => &self.volume,
            AutoTarget::Pan => &self.pan,
            AutoTarget::Object(axis) => &self.object[axis as usize],
            AutoTarget::Fx { slot, param } => self
                .fx
                .iter()
                .find(|l| l.slot == slot && l.param == param)
                .map_or(&[], |l| &l.points),
            AutoTarget::Plugin { uid, param } => self
                .plugin_lanes
                .iter()
                .find(|l| l.uid == uid && l.param == param)
                .map_or(&[], |l| &l.points),
        }
    }

    /// The points of the lane of `target`, made if it had none (see [`Automation::prune`]).
    pub fn lane_mut(&mut self, target: AutoTarget) -> &mut Vec<AutoPoint> {
        match target {
            AutoTarget::Volume => &mut self.volume,
            AutoTarget::Pan => &mut self.pan,
            AutoTarget::Object(axis) => &mut self.object[axis as usize],
            AutoTarget::Fx { slot, param } => {
                let i = match self
                    .fx
                    .iter()
                    .position(|l| l.slot == slot && l.param == param)
                {
                    Some(i) => i,
                    None => {
                        self.fx.push(FxLane {
                            slot,
                            param,
                            points: Vec::new(),
                        });
                        self.fx.len() - 1
                    }
                };
                &mut self.fx[i].points
            }
            AutoTarget::Plugin { uid, param } => {
                let i = match self
                    .plugin_lanes
                    .iter()
                    .position(|l| l.uid == uid && l.param == param)
                {
                    Some(i) => i,
                    None => {
                        self.plugin_lanes.push(PluginLane {
                            uid,
                            param,
                            points: Vec::new(),
                        });
                        self.plugin_lanes.len() - 1
                    }
                };
                &mut self.plugin_lanes[i].points
            }
        }
    }

    /// Every lane (the fader's and the pan's first), with its points; for walking over them
    /// without making a list.
    pub fn lanes(&self) -> impl Iterator<Item = (AutoTarget, &[AutoPoint])> {
        [
            (AutoTarget::Volume, &self.volume[..]),
            (AutoTarget::Pan, &self.pan[..]),
        ]
        .into_iter()
        .chain(
            ovrtone_engine::ObjectAxis::ALL
                .into_iter()
                .map(|axis| (AutoTarget::Object(axis), &self.object[axis as usize][..])),
        )
        .chain(self.fx.iter().map(|l| {
            (
                AutoTarget::Fx {
                    slot: l.slot,
                    param: l.param,
                },
                &l.points[..],
            )
        }))
        .chain(self.plugin_lanes.iter().map(|l| {
            (
                AutoTarget::Plugin {
                    uid: l.uid,
                    param: l.param,
                },
                &l.points[..],
            )
        }))
    }

    /// Whether any lane has points.
    pub fn has_lane(&self) -> bool {
        self.lanes().any(|(_, pts)| !pts.is_empty())
    }

    /// Takes away the effect lanes that have no points.
    pub fn prune(&mut self) {
        self.fx.retain(|l| !l.points.is_empty());
        self.plugin_lanes.retain(|l| !l.points.is_empty());
    }

    /// Whether the engine plays the lane of `target` (rather than the fader or knob).
    pub fn follows(&self, target: AutoTarget) -> bool {
        match self.mode {
            AutoMode::Off | AutoMode::Write => false,
            AutoMode::Read => true,
            AutoMode::Touch | AutoMode::Latch | AutoMode::Trim => self.held != Some(target),
        }
    }

    /// The lane's value at `beat`, if the strip plays the lane and it has points.
    pub fn played(&self, target: AutoTarget, beat: f64) -> Option<f32> {
        if !self.follows(target) {
            return None;
        }
        let lane = self
            .region_played
            .iter()
            .find(|l| l.0 == target)
            .map_or(self.lane(target), |l| &l.1[..]);
        ovrtone_engine::lane_value(lane, beat)
    }

    /// The most points one lane keeps.
    pub const MAX_POINTS: usize = ovrtone_engine::MAX_AUTO_POINTS;

    /// Sets the point at `beat` (adds it, or moves the value of the one already there),
    /// keeping the lane in order.
    pub fn set_point(&mut self, target: AutoTarget, beat: f64, value: f32) {
        if !beat.is_finite() || !value.is_finite() || beat < 0.0 {
            return;
        }
        let (lo, hi) = Self::range(target);
        let value = value.clamp(lo, hi);
        let pts = self.lane_mut(target);
        let i = pts.partition_point(|p| p.beat < beat);
        if i < pts.len() && (pts[i].beat - beat).abs() < 1e-9 {
            pts[i].value = value;
        } else if pts.len() < Self::MAX_POINTS {
            pts.insert(i, AutoPoint::new(beat, value));
        }
        self.prune();
    }

    /// The range of a lane's values (an effect's lane holds positions 0..=1).
    pub fn range(target: AutoTarget) -> (f32, f32) {
        match target {
            AutoTarget::Volume => (-60.0, 6.0),
            AutoTarget::Pan => (-1.0, 1.0),
            AutoTarget::Fx { .. } | AutoTarget::Plugin { .. } => (0.0, 1.0),
            AutoTarget::Object(axis) => axis.range(),
        }
    }

    /// Writes the control's value while playing: the points between the last write of this
    /// pass and `beat` are replaced by the new one. Writes close together are skipped unless
    /// the value moved, or `force` (the end of a pass).
    pub fn write(&mut self, target: AutoTarget, beat: f64, value: f32, force: bool) {
        if !beat.is_finite() || !value.is_finite() {
            return;
        }
        let moved = match target {
            AutoTarget::Volume => 0.1,
            AutoTarget::Pan => 0.01,
            AutoTarget::Fx { .. } | AutoTarget::Plugin { .. } => 0.004,
            AutoTarget::Object(ovrtone_engine::ObjectAxis::Around | ovrtone_engine::ObjectAxis::Height) => 0.5,
            AutoTarget::Object(_) => 0.01,
        };
        let last = self.last.filter(|(b, _)| beat >= *b);
        if let Some((lb, lv)) = last {
            if !force && beat - lb < WRITE_STEP && (value - lv).abs() < moved {
                return;
            }
            self.lane_mut(target)
                .retain(|p| p.beat <= lb || p.beat > beat);
        }
        self.set_point(target, beat, value);
        self.last = Some((beat, value));
    }

    /// Ends the pass: the next write starts a new one.
    pub fn end_pass(&mut self) {
        self.last = None;
    }

    /// Removes every point of every lane.
    pub fn clear(&mut self) {
        self.volume.clear();
        self.pan.clear();
        self.fx.clear();
        self.object.iter_mut().for_each(Vec::clear);
        self.last = None;
    }

    /// The effect in `slot` was taken away: its lanes go, those of later effects move up.
    pub fn insert_removed(&mut self, slot: usize) {
        self.fx.retain(|l| l.slot as usize != slot);
        for l in self.fx.iter_mut().filter(|l| l.slot as usize > slot) {
            l.slot -= 1;
        }
        self.drop_stale_state();
    }

    /// An effect was put in at `slot`: the lanes of it and the later effects move down one.
    pub fn insert_added(&mut self, slot: usize) {
        for l in self.fx.iter_mut().filter(|l| l.slot as usize >= slot) {
            l.slot = l.slot.saturating_add(1);
        }
        self.drop_stale_state();
    }

    /// The effects in slots `a` and `b` changed places.
    pub fn insert_swapped(&mut self, a: usize, b: usize) {
        for l in self.fx.iter_mut() {
            if l.slot as usize == a {
                l.slot = b as u8;
            } else if l.slot as usize == b {
                l.slot = a as u8;
            }
        }
        self.drop_stale_state();
    }

    /// The effect in `slot` was replaced by another kind: its lanes mean nothing now.
    pub fn insert_replaced(&mut self, slot: usize) {
        self.fx.retain(|l| l.slot as usize != slot);
        self.drop_stale_state();
    }

    /// Forget what was written or held about a lane that moved or went.
    fn drop_stale_state(&mut self) {
        self.last = None;
        if matches!(self.held, Some(AutoTarget::Fx { .. })) {
            self.held = None;
        }
        if matches!(self.shown, AutoTarget::Fx { .. }) {
            self.shown = AutoTarget::Volume;
        }
    }
}

/// A mix group (Logic: Group): tracks whose controls move together. The attributes say which.
#[derive(Clone, PartialEq, Debug)]
pub struct MixGroup {
    pub name: String,
    /// Moving one fader moves the others by the same number of dB.
    pub volume: bool,
    pub pan: bool,
    pub mute: bool,
    pub solo: bool,
}

/// The name of the Aux that Create Mastering Track makes.
pub const MASTERING_TRACK: &str = "Mastering";

/// Most mix groups in a project.
pub const MAX_GROUPS: usize = 64;

impl MixGroup {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            volume: true,
            pan: true,
            mute: true,
            solo: true,
        }
    }
}

/// Logic Pro Track Inspector values for instrument tracks.
#[derive(Clone, Copy, PartialEq)]
pub struct TrackParams {
    /// Its number as a channel of the Channel Rack, given when it is first used as one; else 0.
    pub channel: u64,
    pub transpose: i8,
    pub velocity: i8,
    /// Lowest/highest MIDI note that plays (Key Limit).
    pub key_limit: (u8, u8),
    /// Lowest/highest velocity that plays (Vel Limit).
    pub vel_limit: (u8, u8),
    /// The Groove Track: the one whose timing the following tracks take (see `groove.rs`).
    pub groove: bool,
    /// Plays with the Groove Track's timing.
    pub follow_groove: bool,
}

impl Default for TrackParams {
    fn default() -> Self {
        Self {
            channel: 0,
            transpose: 0,
            velocity: 0,
            key_limit: (0, 127),
            vel_limit: (1, 127),
            groove: false,
            follow_groove: false,
        }
    }
}

/// How much a quiet clip's waveform is enlarged at most: 12 dB, so that a quiet recording is easy
/// to see and still does not look as loud as one that clips. The waveform buttons do the rest.
pub const MAX_WAVEFORM_FIT: f32 = 4.0;

impl AudioRegion {
    /// The level to draw the whole clip with at `width` points: the overview, until the clip is
    /// wider than it has points.
    pub fn peaks_for(&self, width: f32) -> Option<&[(f32, f32)]> {
        let overview = self.peaks.get()?;
        match self.fine_peaks.get() {
            Some(fine) if width > overview.len() as f32 => Some(fine),
            _ => Some(overview),
        }
    }

    /// What to multiply the overview by so the clip fills its region. Only the drawing: the
    /// sound stays as it is.
    pub fn waveform_fit(&self) -> f32 {
        let Some(peaks) = self.peaks.get() else {
            return 1.0;
        };
        let top = *self.peak_top.get_or_init(|| {
            peaks
                .iter()
                .fold(0f32, |top, bin| top.max(bin.1).max(-bin.0))
        });
        if top <= 0.0 {
            return 1.0;
        }
        (0.95 / top).clamp(1.0, MAX_WAVEFORM_FIT)
    }
}

#[derive(Clone)]
pub struct AudioRegion {
    pub file: AudioFile,
    /// Min/max overview, filled by a background thread after import.
    pub peaks: Arc<OnceLock<Vec<(f32, f32)>>>,
    /// The same at a point per 64 frames, for when the clip is zoomed in past the overview.
    pub fine_peaks: Arc<OnceLock<Vec<(f32, f32)>>>,
    /// The loudest point of `peaks`, worked out once.
    pub peak_top: Arc<OnceLock<f32>>,
    /// Flex Time and Flex Pitch: when set, `file` is the sound made from this edit.
    pub flex: Option<crate::flex_edit::FlexEdit>,
    /// The tempo the audio was made at, when it stretches to follow the project's. A hit or a
    /// recording has none and stays as it is.
    pub follow_bpm: Option<f64>,
}

/// A hosted plugin on a channel strip: what the project knows of it. The running instance is kept
/// by [`crate::plugin_rack::Rack`], which finds it by `uid`.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginInsert {
    /// The insert row it was put in. 0 means the next one free after what comes before it.
    pub row: u8,
    /// Which instance this is, in this run of the program. Never saved.
    pub uid: u64,
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub path: std::path::PathBuf,
    pub bypass: bool,
    /// Why it did not load. Not saved: it is tried again next time the project opens.
    pub failed: Option<String>,
    /// The plugin's settings as last collected. The plugin has the current ones while it runs:
    /// this is what it starts from, and what goes in the file.
    pub state: Option<std::sync::Arc<Vec<u8>>>,
    /// What can be automated, once it has loaded. Not saved.
    pub params: std::sync::Arc<Vec<ovrtone_host::PluginParam>>,
    /// The track whose sound goes to its side-chain input.
    pub sidechain: Option<usize>,
    /// It said it has a side-chain input, once it has loaded. Not saved.
    pub side_offered: bool,
    /// The parameter on each Smart Controls knob, once the user has chosen.
    pub smart_knobs: Option<[Option<u32>; SMART_KNOBS]>,
    /// What part of its parameter each knob covers, low and high end, 0..=1. The high end below
    /// the low one turns the knob around. `None` is all of it for every knob.
    pub smart_ranges: Option<[(f32, f32); SMART_KNOBS]>,
}

/// What a hardware control moves (MIDI learn).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LearnTarget {
    Volume(usize),
    Pan(usize),
    MasterVolume,
    Plugin { uid: u64, param: u32 },
}

/// A hardware control and what it moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MidiMapping {
    /// 0-15.
    pub channel: u8,
    pub controller: u8,
    pub target: LearnTarget,
}

/// Knobs in the Smart Controls window.
pub const SMART_KNOBS: usize = 8;

impl PluginInsert {
    pub fn new(info: &ovrtone_host::PluginInfo) -> Self {
        Self {
            row: 0,
            uid: crate::plugin_rack::new_uid(),
            id: info.id.clone(),
            name: info.name.clone(),
            vendor: info.vendor.clone(),
            path: info.path.clone(),
            bypass: false,
            failed: None,
            state: None,
            params: std::sync::Arc::default(),
            sidechain: None,
            side_offered: false,
            smart_knobs: None,
            smart_ranges: None,
        }
    }
}

/// Whether a track of this kind has a sound that can key a plugin's side-chain input.
pub fn can_key(kind: Option<TrackKind>) -> bool {
    !matches!(kind, None | Some(TrackKind::Folder | TrackKind::Vca))
}

/// The insert row each of `plugins` shows in, with `first` rows taken above them. They stay in
/// order: a row already passed gives way to the next free one.
pub fn plugin_rows(first: usize, plugins: &[PluginInsert]) -> Vec<usize> {
    let mut next = first;
    plugins
        .iter()
        .map(|plugin| {
            let row = next.max(usize::from(plugin.row));
            next = row + 1;
            row
        })
        .collect()
}

/// The first row after `plugins`, where the next one goes.
pub fn next_plugin_row(first: usize, plugins: &[PluginInsert]) -> usize {
    plugin_rows(first, plugins)
        .last()
        .map_or(first, |row| row + 1)
}

/// Fixes each plugin in the row it shows in, so that taking one away moves no other.
pub fn settle_plugin_rows(first: usize, plugins: &mut [PluginInsert]) {
    let rows = plugin_rows(first, plugins);
    for (plugin, row) in plugins.iter_mut().zip(rows) {
        plugin.row = row.min(usize::from(u8::MAX)) as u8;
    }
}

/// Puts `new` in insert row `row`, which is free. The order of the list is the order of the sound.
pub fn place_plugin_in_row(
    first: usize,
    plugins: &mut Vec<PluginInsert>,
    mut new: PluginInsert,
    row: usize,
) {
    settle_plugin_rows(first, plugins);
    new.row = row.min(usize::from(u8::MAX)) as u8;
    let at = plugins.partition_point(|p| p.row < new.row);
    plugins.insert(at, new);
}

/// What `Track::instrument` holds when the user removed the built-in sound: the track plays
/// nothing until a plugin or a sample is put on it.
pub const NO_INSTRUMENT: &str = "";
/// Locators running this many bars past the end of the song are not a loop anyone asked for.
const CYCLE_OVERRUN_BARS: u32 = 4;

#[derive(Clone)]
pub struct Track {
    pub name: String,
    pub kind: TrackKind,
    pub color: Color32,
    pub mute: bool,
    pub solo: bool,
    pub rec: bool,
    /// Input monitoring ("I" button, audio tracks only).
    pub monitor: bool,
    /// The input an audio track records.
    pub input: ovrtone_engine::InputSource,
    /// The bus the strip plays into; 0 is Stereo Out (the mixer's Output slot).
    pub output: u16,
    /// The device's outputs it plays to, instead of Stereo Out or a bus: 1 is 3-4, 2 is 5-6. 0 for none.
    pub hardware_out: u8,
    /// The bus an Aux listens to, when it was chosen. 0: the next one free.
    pub aux_bus: u16,
    /// Its place in the room, if it is placed in the spatial mix: as an object, or (with `bed_pan`)
    /// panned around the bed.
    pub object: Option<ovrtone_engine::ObjectPlace>,
    /// Its place is a surround pan inside the bed (as a DAW's surround panner does), not an object:
    /// it is heard the same, and a spatial file has it mixed into the bed's channels.
    pub bed_pan: bool,
    /// Input trim in dB: the level going into the track's effects, the fader untouched. What
    /// Auto Gain Staging sets.
    pub trim_db: f32,
    /// How far a slide note can bend, in semitones: what the track's instrument is set to.
    pub slide_range: u8,
    pub volume_db: f32,
    pub pan: f32,
    pub inserts: Vec<Insert>,
    /// Hosted plugins of the strip, after `inserts`.
    pub plugins: Vec<PluginInsert>,
    /// The hosted plugin that plays the track's notes, in place of the built-in sound.
    pub instrument_plugin: Option<PluginInsert>,
    pub regions: Vec<Region>,
    pub params: TrackParams,
    /// Sends to buses, in slot order (up to [`MAX_SENDS`]; an emptied slot is removed).
    pub sends: Vec<SendSlot>,
    pub auto: Automation,
    /// The mix group (index into `Project::groups`) the track belongs to.
    pub group: Option<usize>,
    /// Frozen: this file plays in place of the track's regions, instrument and effects, which
    /// stay as they are but are not running. Not saved: a project opens with its tracks live.
    pub frozen: Option<Box<Region>>,
    /// An Aux shows in the Tracks area only when asked to; in the mixer it always does. Other
    /// kinds of track always show.
    pub on_timeline: bool,
    /// Switched off with its power button: silent, and its plugins rest.
    pub off: bool,
    /// Its regions cannot be selected, moved or edited.
    pub locked: bool,
    /// A reference track (a finished song to compare with): heard while mixing, left out of
    /// bounces, exports and gain staging.
    pub reference: bool,
    /// The icon chosen with Assign Track Icon; else one follows from the patch.
    pub icon: Option<crate::track_icons::IconKind>,
    /// Library category of the loaded patch (drives the track icon), if any.
    pub category: Option<&'static str>,
    /// Individual track height as a factor of the global track zoom (Logic: drag the lower
    /// edge of a track). 1.0 = the zoom slider's height.
    pub height: f32,
    /// Folder nesting level, 0 at the top. A track belongs to the closest `Folder` track above
    /// it with a lower level.
    pub indent: u8,
    /// A `Folder` track whose tracks are hidden in the Tracks area.
    pub folded: bool,
    /// The `Vca` track (its index in the project) that moves this track's level, if any.
    pub vca: Option<usize>,
    /// External MIDI tracks: the MIDI output port (by name) and the channel, 1-16.
    pub midi_port: Option<String>,
    pub midi_channel: u8,
    /// MIDI FX: effects that change the notes the track plays (see `midi_fx.rs`).
    pub midi_fx: Vec<crate::midi_fx::FxSlot>,
    /// The curve of its automation (the lane `auto.shown`) is drawn on the track in the Tracks area.
    pub show_auto: bool,
    /// Other versions of the regions (see `alternatives.rs`). Empty until a second one is made. The
    /// one at `active_alt` has its regions in `regions`, and none here.
    pub alternatives: Vec<TrackAlternative>,
    pub active_alt: usize,
}

/// One version of a track's regions.
#[derive(Clone)]
pub struct TrackAlternative {
    pub name: String,
    pub regions: Vec<Region>,
}

impl Track {
    /// Every hosted plugin of the track.
    pub fn hosted(&self) -> impl Iterator<Item = &PluginInsert> {
        self.instrument_plugin.iter().chain(&self.plugins)
    }

    pub fn hosted_mut(&mut self) -> impl Iterator<Item = &mut PluginInsert> {
        self.instrument_plugin.iter_mut().chain(&mut self.plugins)
    }

    /// Smallest and largest individual height factor.
    pub const MIN_HEIGHT: f32 = 0.5;
    pub const MAX_HEIGHT: f32 = 6.0;

    /// A new track of `kind` with default settings and no regions: software instruments get
    /// the Sampler, and every kind its own colour.
    pub fn new(name: impl Into<String>, kind: TrackKind) -> Self {
        let color = match kind {
            TrackKind::Instrument => Color32::from_rgb(0x4e, 0xb0, 0x4a),
            TrackKind::Drummer => Color32::from_rgb(0xd9, 0xaa, 0x2a),
            TrackKind::Audio => Color32::from_rgb(0x3f, 0x5f, 0xc0),
            TrackKind::Aux => Color32::from_rgb(0x2a, 0xa8, 0xa8),
            TrackKind::Folder => Color32::from_rgb(0x88, 0x8c, 0x98),
            TrackKind::Vca => Color32::from_rgb(0x9a, 0x6c, 0xd0),
            TrackKind::ExternalMidi => Color32::from_rgb(0xd0, 0x84, 0x3c),
            TrackKind::Bus | TrackKind::Output => Color32::from_rgb(0x7a, 0x4f, 0xc0),
        };
        Self {
            name: name.into(),
            kind,
            color,
            mute: false,
            solo: false,
            rec: false,
            monitor: false,
            input: ovrtone_engine::InputSource::Mono(0),
            output: 0,
            hardware_out: 0,
            aux_bus: 0,
            object: None,
            bed_pan: false,
            trim_db: 0.0,
            slide_range: DEFAULT_SLIDE_RANGE,
            volume_db: 0.0,
            pan: 0.0,
            inserts: vec![],
            plugins: Vec::new(),
            instrument_plugin: None,
            regions: vec![],
            params: TrackParams::default(),
            sends: Vec::new(),
            auto: Automation::default(),
            group: None,
            category: None,
            icon: None,
            frozen: None,
            on_timeline: false,
            off: false,
            locked: false,
            reference: false,
            height: 1.0,
            indent: 0,
            folded: false,
            vca: None,
            midi_port: None,
            midi_channel: 1,
            midi_fx: Vec::new(),
            show_auto: false,
            alternatives: Vec::new(),
            active_alt: 0,
        }
    }
}

/// A tempo change on the Tempo track (Logic: a tempo point): `bpm` from `beat` (quarter notes
/// from the start) until the next change, or, with `ramp`, moving in a straight line to the
/// next change's tempo (a tempo curve).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TempoPoint {
    pub beat: f64,
    pub bpm: f32,
    pub ramp: bool,
}

/// A marker on the Marker track (Logic: Intro, Verse, Chorus...): from `beat` (quarter notes
/// from the start) until the next marker.
#[derive(Clone, PartialEq, Debug)]
pub struct Marker {
    pub beat: f64,
    pub name: String,
    pub color: Color32,
}

/// A named section of the song on the Arrangement track, from `beat` for `len` beats. Moving one
/// takes everything in it along: see `arrangement_track.rs`.
#[derive(Clone, PartialEq, Debug)]
pub struct Section {
    pub beat: f64,
    pub len: f64,
    pub name: String,
    pub color: Color32,
}

/// A chord on the Chord track, from `beat` until the next. Only a name: it makes no sound.
#[derive(Clone, PartialEq, Debug)]
pub struct ChordMark {
    pub beat: f64,
    pub name: String,
}

/// Colours offered for markers, in the order new markers take them.
pub const MARKER_COLORS: [Color32; 8] = [
    Color32::from_rgb(0x3f, 0x8f, 0xd8),
    Color32::from_rgb(0x4e, 0xb0, 0x4a),
    Color32::from_rgb(0xd9, 0xaa, 0x2a),
    Color32::from_rgb(0xd8, 0x6a, 0x2c),
    Color32::from_rgb(0xc8, 0x3c, 0x3c),
    Color32::from_rgb(0xb0, 0x4f, 0xb8),
    Color32::from_rgb(0x7a, 0x4f, 0xc0),
    Color32::from_rgb(0x3c, 0xa8, 0xa0),
];

/// Logic Pro's tempo range.
pub const MIN_BPM: f32 = 5.0;
pub const MAX_BPM: f32 = 990.0;

#[derive(Clone)]
pub struct Project {
    /// Mixer > Select Audio Channel Strips: strips that move together until another is clicked.
    /// Not saved.
    pub linked_strips: Vec<usize>,
    pub name: String,
    /// Tempo at the start of the song (the first point of the Tempo track).
    pub tempo: f32,
    /// Whether the start tempo ramps to the first of `tempo_changes` (a tempo curve).
    pub tempo_ramp: bool,
    /// The hits of an audio Groove Track, in beats (found in the background, not saved).
    pub groove_onsets: std::sync::Arc<Vec<f64>>,
    /// What `groove_onsets` were found for (see `groove::poll`).
    pub groove_key: u64,
    /// Later tempo changes, sorted by beat, all after the start (see [`Project::tidy_tempo`]).
    pub tempo_changes: Vec<TempoPoint>,
    /// Marker track, sorted by beat (see [`Project::tidy_markers`]).
    pub markers: Vec<Marker>,
    /// Arrangement track, sorted by beat, not overlapping (see `arrangement_track::tidy`).
    pub sections: Vec<Section>,
    /// Chord track, sorted by beat.
    pub chords: Vec<ChordMark>,
    /// Time signature of the first bar. Later changes are in `sig_changes`.
    pub sig: (u8, u8),
    /// Signature track: signature changes after bar 1, sorted by bar (see
    /// [`Project::tidy_signatures`]).
    pub sig_changes: Vec<SigChange>,
    /// Frame rate of the SMPTE display and of MIDI Time Code (Logic: Project Settings >
    /// Synchronization > Frame Rate).
    pub smpte_rate: ovrtone_engine::SmpteRate,
    /// SMPTE time of the project start: hours, minutes, seconds, frames.
    pub smpte_start: [u8; 4],
    /// MIDI output port (by name) that sends MIDI Time Code while the project plays.
    pub mtc_out: Option<String>,
    /// The MIDI port MIDI Clock (24 a beat, with Start, Stop and the song position) goes to.
    pub clock_out: Option<String>,
    /// MIDI input port (by name) the transport chases MIDI Time Code from.
    pub mtc_in: Option<String>,
    pub key: &'static str,
    pub tracks: Vec<Track>,
    pub playing: bool,
    pub recording: bool,
    pub cycle: bool,
    pub metronome: bool,
    /// Count-in before recording (the purple "1234" button).
    pub count_in: bool,
    /// What the spatial bus goes to; off is plain stereo.
    pub spatial_mode: ovrtone_engine::SpatialMode,
    /// How much room the headphone render has, 0..=1.
    pub spatial_room: f32,
    /// The bed format, kept while spatial audio is off.
    pub spatial_bed: ovrtone_engine::SpeakerLayout,
    /// How objects are panned to the speakers. Projects from before VBAP keep the Smooth law.
    pub spatial_law: ovrtone_engine::SpatialLaw,
    /// The bus channel playing the test signal (Speaker Setup). Not saved.
    pub spatial_test: Option<usize>,
    /// Multichannel spatial buses (a 7.1.2 bus, say): objects routed to one are panned in its layout,
    /// and its panner places the whole of it in the room.
    pub spatial_buses: Vec<SpatialBusDef>,
    /// The renderer's volume, and how far each channel sounds on headphones.
    pub spatial_volume_db: f32,
    pub spatial_channels: ovrtone_engine::ChannelSetup,
    /// Channels muted and soloed in the renderer, to listen with. Not saved.
    pub spatial_mute: [bool; ovrtone_engine::SPATIAL_CHANNELS],
    pub spatial_solo: [bool; ovrtone_engine::SPATIAL_CHANNELS],
    /// Mixer snapshots, oldest first.
    pub mixer_snapshots: Vec<MixerSnapshot>,
    /// The count-in is running: Play is on, the song has not started yet. Not saved.
    pub counting_in: bool,
    /// The Note Pad's text.
    pub notes: String,
    /// Where the end marker was put, in beats. None: at the end of what is there.
    pub end_beat: Option<f64>,
    /// Cycle (left/right) locators, in beats.
    pub cycle_start: f64,
    pub cycle_end: f64,
    pub position_beats: f64, // 0-based
    pub selected_track: usize,
    pub master_vol: f32,
    /// Mix groups; a track's `group` is an index into this list.
    pub groups: Vec<MixGroup>,
    /// Bumped whenever tracks/regions change so the engine song is rebuilt.
    pub song_rev: u64,
    /// The Channel Rack's patterns.
    pub patterns: Vec<crate::song_pattern::SongPattern>,
    /// Plugins with a long delay step aside, for playing and recording with less delay. Not saved.
    pub low_latency: bool,
    /// Armed audio tracks are heard while stopped and while recording, not on playback.
    pub auto_monitor: bool,
    /// Recording waits until the input passes `trigger_db`.
    pub wait_for_sound: bool,
    pub trigger_db: f32,
    /// How far Nudge moves a region, in beats.
    pub nudge_beats: f64,
    /// The channels of the output device, for the Output menus. Not saved.
    pub output_channels: std::sync::Arc<std::sync::atomic::AtomicU32>,
    /// Varispeed: percent from normal speed, -50 to +100. Pitch moves with it. Bounces ignore it.
    pub varispeed: f32,
    /// What the speakers get, for checking a mix. Not saved, so that no one is caught out.
    pub out_mono: bool,
    pub out_dim: bool,
    /// Plugins go on running with nothing to process. For the few that need it. Not saved.
    pub keep_plugins_running: bool,
    /// Autopunch: recording happens only between these beats, however it is started.
    pub punch: bool,
    pub punch_start: f64,
    pub punch_end: f64,
    /// Bars played before a recording starts from a stop, and after it ends (0: none), as Pro
    /// Tools' pre-roll and post-roll.
    pub pre_roll: f32,
    pub post_roll: f32,
    /// While a pre-roll's recording runs: the punch settings to put back when the song stops.
    pub roll_restore: Option<(bool, f64, f64)>,
    /// Where the song stops on its own: the end of a post-roll.
    pub stop_at: Option<f64>,
    /// A new recording takes away what it was recorded over, instead of making takes of it.
    pub rec_replace: bool,
    /// Cuts and trims of audio regions land on the nearest place the waveform crosses zero.
    pub snap_zero: bool,
    /// The plugins on the output (Stereo Out), after everything is mixed.
    pub master_plugins: Vec<PluginInsert>,
    /// Stereo Out's own effects (Ovrtone's), before its plugins: kept as a strip of their own so the
    /// effect windows work on them as on a track's. See [`MASTER_STRIP`].
    pub master_strip: Track,
    /// Hardware controls assigned with MIDI learn.
    pub midi_map: Vec<MidiMapping>,
    /// Groove templates, offered by Quantize (see `groove_template`).
    pub grooves: Vec<ovrtone_project::GrooveDoc>,
    /// Panel layouts on the number keys (see `screensets`).
    pub screensets: Vec<ovrtone_project::ScreensetDoc>,
}

/// Deterministic pseudo-random so demo content is stable across runs.
fn lcg(seed: &mut u32) -> u32 {
    *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
    *seed >> 8
}

fn demo_notes(seed: u32, bars: f32, lo: u8, hi: u8) -> Vec<Note> {
    let mut s = seed;
    let mut out = Vec::new();
    let mut t = 0.0;
    while t < bars * 4.0 {
        let len = [0.5, 0.75, 1.0, 1.5, 0.25][(lcg(&mut s) % 5) as usize];
        let pitch = lo + (lcg(&mut s) % (hi - lo) as u32) as u8;
        out.push(Note {
            start: t,
            len: len * 0.9,
            pitch,
            vel: 70 + (lcg(&mut s) % 50) as u8,
            muted: false,
            color: 0,
            slide: false,
        });
        t += [0.5, 1.0, 0.25, 0.75][(lcg(&mut s) % 4) as usize];
    }
    out
}

impl Project {
    /// A new, empty project with one software instrument track (File > New).
    pub fn empty() -> Self {
        let mut p = Self::demo();
        p.name = "Untitled".into();
        p.tracks = vec![Track {
            name: "Inst 1".into(),
            kind: TrackKind::Instrument,
            color: Color32::from_rgb(0x4e, 0xb0, 0x4a),
            mute: false,
            solo: false,
            rec: false,
            monitor: false,
            input: ovrtone_engine::InputSource::Mono(0),
            output: 0,
            hardware_out: 0,
            aux_bus: 0,
            object: None,
            bed_pan: false,
            trim_db: 0.0,
            slide_range: DEFAULT_SLIDE_RANGE,
            volume_db: 0.0,
            pan: 0.0,
            inserts: vec![],
            plugins: Vec::new(),
            instrument_plugin: None,
            regions: vec![],
            params: TrackParams::default(),
            category: None,
            icon: None,
            frozen: None,
            on_timeline: false,
            off: false,
            locked: false,
            reference: false,
            sends: Vec::new(),
            auto: Automation::default(),
            group: None,
            height: 1.0,
            indent: 0,
            folded: false,
            vca: None,
            midi_port: None,
            midi_channel: 1,
            midi_fx: Vec::new(),
            show_auto: false,
            alternatives: Vec::new(),
            active_alt: 0,
        }];
        p.position_beats = 0.0;
        p.cycle = false;
        p.cycle_start = 0.0;
        p.cycle_end = 16.0;
        p.selected_track = 0;
        p
    }

    pub fn demo() -> Self {
        let green = Color32::from_rgb(0x4e, 0xb0, 0x4a);
        let blue = Color32::from_rgb(0x3f, 0x5f, 0xc0);
        let yellow = Color32::from_rgb(0xd9, 0xaa, 0x2a);
        let purple = Color32::from_rgb(0x7a, 0x4f, 0xc0);
        let mk = |name: &str,
                  kind,
                  color,
                  _inst: Option<&'static str>,
                  ins: Vec<&'static str>,
                  seed,
                  vol| Track {
            name: name.into(),
            kind,
            color,
            mute: false,
            solo: false,
            rec: false,
            monitor: false,
            input: ovrtone_engine::InputSource::Mono(0),
            output: 0,
            hardware_out: 0,
            aux_bus: 0,
            object: None,
            bed_pan: false,
            trim_db: 0.0,
            slide_range: DEFAULT_SLIDE_RANGE,
            volume_db: vol,
            pan: 0.0,
            inserts: inserts_from_names(&ins),
            plugins: Vec::new(),
            instrument_plugin: None,
            regions: vec![Region {
                name: name.into(),
                start_bar: 1.0,
                len_bars: 12.0,
                notes: if matches!(kind, TrackKind::Audio) {
                    vec![]
                } else {
                    demo_notes(seed, 12.0, 50, 76)
                },
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
            on_timeline: false,
            off: false,
            locked: false,
            reference: false,
            sends: Vec::new(),
            auto: Automation::default(),
            group: None,
            height: 1.0,
            indent: 0,
            folded: false,
            vca: None,
            midi_port: None,
            midi_channel: 1,
            midi_fx: Vec::new(),
            show_auto: false,
            alternatives: Vec::new(),
            active_alt: 0,
        };
        let tracks = vec![
            mk(
                "Classic Electric Piano",
                TrackKind::Instrument,
                green,
                Some("E-Piano"),
                vec!["Tape Delay", "Channel EQ"],
                1,
                -3.2,
            ),
            mk(
                "Steinway Grand Piano",
                TrackKind::Instrument,
                green,
                Some("Sampler"),
                vec!["Channel EQ", "Compressor"],
                2,
                -2.7,
            ),
            mk(
                "Analog Pad",
                TrackKind::Instrument,
                green,
                Some("Alchemy"),
                vec!["Chorus", "Space D"],
                3,
                -5.6,
            ),
            mk(
                "Drummer",
                TrackKind::Drummer,
                yellow,
                Some("Drum Kit"),
                vec!["Compressor"],
                4,
                -4.0,
            ),
            mk(
                "Electric Bass",
                TrackKind::Audio,
                blue,
                None,
                vec!["Channel EQ"],
                5,
                -8.2,
            ),
            mk(
                "Lead Vocal",
                TrackKind::Audio,
                purple,
                None,
                vec!["Channel EQ", "Compressor", "Space D"],
                6,
                -7.8,
            ),
        ];
        let mut tracks = tracks;
        for (t, bus) in tracks
            .iter_mut()
            .zip([Some(1u16), Some(3), Some(3), Some(3)])
        {
            t.sends = vec![SendSlot {
                bus,
                level: 0.0,
                pre: false,
            }];
        }
        Self {
            name: "Untitled".into(),
            tempo: 120.0,
            patterns: Vec::new(),
            master_plugins: Vec::new(),
            master_strip: Track::new("Stereo Out", TrackKind::Output),
            midi_map: Vec::new(),
            grooves: Vec::new(),
            screensets: Vec::new(),
            low_latency: false,
            auto_monitor: false,
            wait_for_sound: false,
            trigger_db: -40.0,
            nudge_beats: 0.25,
            varispeed: 0.0,
            output_channels: std::sync::Arc::new(std::sync::atomic::AtomicU32::new(2)),
            out_mono: false,
            out_dim: false,
            keep_plugins_running: false,
            punch: false,
            pre_roll: 0.0,
            post_roll: 0.0,
            roll_restore: None,
            stop_at: None,
            punch_start: 0.0,
            punch_end: 0.0,
            rec_replace: false,
            sections: Vec::new(),
            chords: Vec::new(),
            snap_zero: false,
            sig: (4, 4),
            key: "Cmaj",
            tracks,
            playing: false,
            recording: false,
            cycle: false,
            metronome: false,
            count_in: false,
            spatial_mode: ovrtone_engine::SpatialMode::Off,
            spatial_room: 0.3,
            spatial_bed: ovrtone_engine::SpeakerLayout::Surround714,
            spatial_law: ovrtone_engine::SpatialLaw::Vbap,
            spatial_test: None,
            spatial_buses: Vec::new(),
            spatial_volume_db: 0.0,
            spatial_channels: ovrtone_engine::ChannelSetup::default(),
            spatial_mute: [false; ovrtone_engine::SPATIAL_CHANNELS],
            spatial_solo: [false; ovrtone_engine::SPATIAL_CHANNELS],
            mixer_snapshots: Vec::new(),
            counting_in: false,
            notes: String::new(),
            end_beat: None,
            cycle_start: 8.0,
            cycle_end: 88.0,
            position_beats: 16.0,
            selected_track: 0,
            master_vol: 0.8,
            groups: Vec::new(),
            linked_strips: Vec::new(),
            song_rev: 0,
            tempo_ramp: false,
            groove_onsets: Default::default(),
            groove_key: 0,
            tempo_changes: Vec::new(),
            markers: Vec::new(),
            sig_changes: Vec::new(),
            smpte_rate: ovrtone_engine::SmpteRate::Fps25,
            smpte_start: [1, 0, 0, 0],
            mtc_out: None,
            clock_out: None,
            mtc_in: None,
        }
    }

    /// Adds a marker at `beat` named "Marker <n>" (or replaces the name of one already there).
    /// Returns its index.
    pub fn add_marker(&mut self, beat: f64) -> usize {
        let beat = beat.max(0.0);
        if let Some(i) = self
            .markers
            .iter()
            .position(|m| (m.beat - beat).abs() <= 1e-6)
        {
            return i;
        }
        let n = (1..)
            .find(|n| !self.markers.iter().any(|m| m.name == format!("Marker {n}")))
            .unwrap_or(1);
        let i = self.markers.partition_point(|m| m.beat < beat);
        let color = MARKER_COLORS[self.markers.len() % MARKER_COLORS.len()];
        self.markers.insert(
            i,
            Marker {
                beat,
                name: format!("Marker {n}"),
                color,
            },
        );
        i
    }

    /// The marker the playhead at `beat` is in (the last one at or before it).
    pub fn marker_at(&self, beat: f64) -> Option<usize> {
        let n = self.markers.partition_point(|m| m.beat <= beat + 1e-6);
        n.checked_sub(1)
    }

    /// Start of the first marker after `beat`.
    pub fn next_marker(&self, beat: f64) -> Option<f64> {
        self.markers
            .iter()
            .map(|m| m.beat)
            .find(|b| *b > beat + 1e-6)
    }

    /// Start of the last marker before `beat`.
    pub fn previous_marker(&self, beat: f64) -> Option<f64> {
        self.markers
            .iter()
            .rev()
            .map(|m| m.beat)
            .find(|b| *b < beat - 1e-6)
    }

    /// Sorts the markers and drops invalid ones (negative or non-finite positions, a second
    /// marker on the same beat).
    pub fn tidy_markers(&mut self) {
        self.markers.retain(|m| m.beat.is_finite() && m.beat >= 0.0);
        self.markers.sort_by(|a, b| a.beat.total_cmp(&b.beat));
        self.markers
            .dedup_by(|b, a| (a.beat - b.beat).abs() <= 1e-6);
    }

    /// The Tempo track as (beat, bpm, ramps to the next) points; the first is the start tempo.
    pub fn tempo_points(&self) -> Vec<TempoPoint> {
        std::iter::once(TempoPoint {
            beat: 0.0,
            bpm: self.tempo,
            ramp: self.tempo_ramp,
        })
        .chain(self.tempo_changes.iter().copied())
        .collect()
    }

    /// The tempo in effect at `beat`: a point's own tempo until the next one, or, on a tempo
    /// curve, the straight line from its tempo to the next point's.
    pub fn tempo_at(&self, beat: f64) -> f32 {
        let i = self.tempo_changes.partition_point(|t| t.beat <= beat);
        let (from, ramp) = match i.checked_sub(1) {
            Some(j) => (self.tempo_changes[j], self.tempo_changes[j].ramp),
            None => (
                TempoPoint {
                    beat: 0.0,
                    bpm: self.tempo,
                    ramp: self.tempo_ramp,
                },
                self.tempo_ramp,
            ),
        };
        match self.tempo_changes.get(i) {
            Some(next) if ramp && next.beat > from.beat => {
                let t = ((beat - from.beat) / (next.beat - from.beat)).clamp(0.0, 1.0) as f32;
                from.bpm + (next.bpm - from.bpm) * t
            }
            _ => from.bpm,
        }
    }

    /// Sets the tempo in effect at `beat`, as dragging the tempo in the control bar does in
    /// Logic: the change before it (or the start tempo) takes the new tempo; on a tempo curve
    /// that change moves by as much as the displayed tempo did.
    pub fn set_tempo_at(&mut self, beat: f64, bpm: f32) {
        let shown = self.tempo_at(beat);
        let bpm = bpm.clamp(MIN_BPM, MAX_BPM);
        let moved = |own: f32| (own + (bpm - shown)).clamp(MIN_BPM, MAX_BPM);
        match self.tempo_changes.iter_mut().rev().find(|t| t.beat <= beat) {
            Some(t) if t.ramp => t.bpm = moved(t.bpm),
            Some(t) => t.bpm = bpm,
            None if self.tempo_ramp => self.tempo = moved(self.tempo),
            None => self.tempo = bpm,
        }
    }

    /// Adds a tempo change at `beat` (replacing one already there). Returns its index. A change
    /// inside a tempo curve keeps the curve going.
    pub fn add_tempo_change(&mut self, beat: f64, bpm: f32) -> usize {
        let beat = beat.max(0.0);
        let bpm = bpm.clamp(MIN_BPM, MAX_BPM);
        if beat <= 0.0 {
            self.tempo = bpm;
            return usize::MAX;
        }
        self.tempo_changes.retain(|t| (t.beat - beat).abs() > 1e-6);
        let i = self.tempo_changes.partition_point(|t| t.beat < beat);
        // Inside a curve (a point before and another after it) the new point continues it.
        let ramp = i < self.tempo_changes.len()
            && match i.checked_sub(1) {
                Some(j) => self.tempo_changes[j].ramp,
                None => self.tempo_ramp,
            };
        self.tempo_changes.insert(i, TempoPoint { beat, bpm, ramp });
        i
    }

    /// Makes the tempo point `i` (0 is the start tempo) ramp to the next point, or hold. The
    /// last point has nothing to ramp to.
    pub fn set_tempo_ramp(&mut self, i: usize, ramp: bool) {
        if i >= self.tempo_changes.len() {
            // Nothing follows the last point: a flag there would only surprise the next point
            // added.
            return;
        }
        if i == 0 {
            self.tempo_ramp = ramp;
        } else {
            self.tempo_changes[i - 1].ramp = ramp;
        }
    }

    /// Sorts the tempo changes and drops invalid ones: at or before the start, duplicates of a
    /// beat (the last one wins) and non-finite values.
    pub fn tidy_tempo(&mut self) {
        self.tempo_changes
            .retain(|t| t.beat.is_finite() && t.beat > 1e-6 && t.bpm.is_finite());
        for t in &mut self.tempo_changes {
            t.bpm = t.bpm.clamp(MIN_BPM, MAX_BPM);
        }
        self.tempo_changes.sort_by(|a, b| a.beat.total_cmp(&b.beat));
        let mut out: Vec<TempoPoint> = Vec::with_capacity(self.tempo_changes.len());
        for t in self.tempo_changes.drain(..) {
            match out.last_mut() {
                Some(last) if (last.beat - t.beat).abs() <= 1e-6 => *last = t,
                _ => out.push(t),
            }
        }
        self.tempo_changes = out;
        // The last point has nothing to ramp to.
        match self.tempo_changes.last_mut() {
            Some(last) => last.ramp = false,
            None => self.tempo_ramp = false,
        }
    }

    /// The tempo and signature as the engine's tempo map.
    pub fn tempo_map(&self) -> ovrtone_engine::TempoMap {
        use ovrtone_engine::{SignatureEvent, TempoCurve, TempoEvent};
        let curve = |ramp: bool| {
            if ramp {
                TempoCurve::Linear
            } else {
                TempoCurve::Step
            }
        };
        let tempos = self
            .tempo_points()
            .into_iter()
            .map(|t| TempoEvent {
                beat: t.beat,
                bpm: t.bpm,
                curve: curve(t.ramp),
            })
            .collect();
        let sigs = std::iter::once(SignatureEvent {
            bar: 0,
            num: self.sig.0,
            den: self.sig.1,
        })
        .chain(self.sig_changes.iter().map(|c| SignatureEvent {
            bar: c.bar - 1,
            num: c.num,
            den: c.den,
        }))
        .collect();
        ovrtone_engine::TempoMap::new(tempos, sigs)
    }

    /// The signature track as a borrowed view with the bar maths on it.
    pub fn sigs(&self) -> SigRef<'_> {
        SigRef {
            first: self.sig,
            changes: &self.sig_changes,
        }
    }

    /// Time signature of the 0-based bar `bar0`.
    pub fn signature_of_bar(&self, bar0: u32) -> (u8, u8) {
        self.sigs().signature_of_bar(bar0)
    }

    /// Quarter notes in the 0-based bar `bar0`: 4 for 4/4, 3 for 6/8 and 3/4, 3.5 for 7/8.
    pub fn bar_len(&self, bar0: u32) -> f64 {
        self.sigs().bar_len(bar0)
    }

    /// Quarter notes from the start of the song to the start of the 0-based bar `bar0`, which
    /// may be fractional (half a bar in is `bar0 = n + 0.5`). Positions ("beats" elsewhere in
    /// the app) are quarter notes, as in the engine's tempo map and Logic, whatever the
    /// signature.
    pub fn bar_to_beats(&self, bar0: f64) -> f64 {
        self.sigs().bar_to_beats(bar0)
    }

    /// The 0-based bar (with a fraction) that the position `beats` is in; the inverse of
    /// [`Project::bar_to_beats`].
    pub fn beats_to_bar(&self, beats: f64) -> f64 {
        self.sigs().beats_to_bar(beats)
    }

    /// Every bar that overlaps `from..=to` (quarter notes), in order. At most 4096 are given,
    /// so a huge range cannot hang a screen redraw.
    pub fn bars_in(&self, from: f64, to: f64) -> Vec<BarInfo> {
        self.sigs().bars_in(from, to)
    }

    /// The fades region `ri` of track `ti` really has, in quarter notes: its own, or longer
    /// where it crosses another audio region. A region with `crossfade` that starts inside an
    /// earlier region fades in over the overlap, and that earlier region fades out over it
    /// (when the later one reaches at least as far).
    pub fn effective_fades(&self, ti: usize, ri: usize) -> (f32, f32) {
        let Some(track) = self.tracks.get(ti) else {
            return (0.0, 0.0);
        };
        let Some(r) = track.regions.get(ri) else {
            return (0.0, 0.0);
        };
        let (mut fade_in, mut fade_out) = (r.params.fade_in as f64, r.params.fade_out as f64);
        if track.kind == TrackKind::Audio {
            let (s, e) = (self.region_start_beats(r), self.region_end_beats(r));
            for (j, o) in track.regions.iter().enumerate() {
                if j == ri || o.params.mute || o.audio.is_none() {
                    continue;
                }
                let (os, oe) = (self.region_start_beats(o), self.region_end_beats(o));
                if r.params.crossfade && os < s && s < oe {
                    fade_in = fade_in.max(oe.min(e) - s);
                }
                if o.params.crossfade && s < os && os < e && oe >= e {
                    fade_out = fade_out.max(e - os);
                }
            }
        }
        (fade_in as f32, fade_out as f32)
    }

    /// The audio edits of region `ri` of track `ti` as the engine applies them: the region's
    /// own plus its crossfades.
    pub fn clip_edit_of(&self, ti: usize, ri: usize) -> ovrtone_engine::ClipEdit {
        let mut edit = self.tracks[ti].regions[ri].params.clip_edit();
        let (fade_in, fade_out) = self.effective_fades(ti, ri);
        edit.fade_in_beats = fade_in as f64;
        edit.fade_out_beats = fade_out as f64;
        // Fit to Tempo: the engine plays it faster or slower, live, from the tempo it was made at.
        if let Some(a) = self.tracks[ti].regions[ri].audio.as_ref() {
            if let Some(bpm) = a.follow_bpm {
                edit.follow_bpm = bpm;
                edit.keep_pitch = a
                    .flex
                    .as_ref()
                    .is_none_or(|f| f.mode != ovrtone_engine::FlexMode::Speed);
            }
        }
        edit
    }

    /// Where a region starts, in quarter notes.
    pub fn region_start_beats(&self, r: &Region) -> f64 {
        self.bar_to_beats(r.start_bar as f64 - 1.0)
    }

    /// Where a region ends, in quarter notes.
    pub fn region_end_beats(&self, r: &Region) -> f64 {
        self.bar_to_beats(r.start_bar as f64 - 1.0 + r.len_bars as f64)
    }

    /// How long a region is, in quarter notes.
    pub fn region_len_beats(&self, r: &Region) -> f64 {
        self.region_end_beats(r) - self.region_start_beats(r)
    }

    /// Length of one signature beat (in quarter notes) at the position `beats`: 1 in x/4, 0.5
    /// in x/8, 2 in x/2.
    pub fn beat_unit_at(&self, beats: f64) -> f64 {
        self.sigs().beat_unit_at(beats)
    }

    /// Quarter notes in the bar the position `beats` is in.
    pub fn bar_len_at(&self, beats: f64) -> f64 {
        self.sigs().bar_len_at(beats)
    }

    /// Start of the bar that holds `beats`.
    pub fn bar_start(&self, beats: f64) -> f64 {
        self.sigs().bar_start(beats)
    }

    /// Start of the bar after the one that holds `beats`.
    pub fn next_bar_start(&self, beats: f64) -> f64 {
        self.sigs().next_bar_start(beats)
    }

    /// Sets the signature from the 1-based `bar` on and returns whether anything changed. Bar 1
    /// is the song's own signature. Regions sit on bars, so this moves them in time: the song
    /// revision is bumped to have the engine reposition them.
    pub fn set_signature_from(&mut self, bar: u32, sig: (u8, u8)) -> bool {
        let bar = bar.clamp(1, MAX_SIG_BAR);
        let sig = (sig.0.clamp(1, 99), valid_denominator(sig.1));
        let before = (self.sig, self.sig_changes.clone());
        let change = SigChange {
            bar,
            num: sig.0,
            den: sig.1,
        };
        if bar == 1 {
            self.sig = sig;
        } else {
            match self.sig_changes.binary_search_by_key(&bar, |c| c.bar) {
                Ok(i) => self.sig_changes[i] = change,
                Err(i) => self.sig_changes.insert(i, change),
            }
        }
        self.tidy_signatures();
        let changed = (self.sig, self.sig_changes.clone()) != before;
        if changed {
            self.song_rev += 1;
        }
        changed
    }

    /// Removes the signature change that starts at the 1-based `bar` (not bar 1 itself).
    pub fn remove_signature_at(&mut self, bar: u32) -> bool {
        let before = self.sig_changes.len();
        self.sig_changes.retain(|c| c.bar != bar);
        let changed = self.sig_changes.len() != before;
        if changed {
            self.song_rev += 1;
        }
        changed
    }

    /// Sorts the signature changes, fixes invalid values, and drops any at bar 1 or beyond
    /// [`MAX_SIG_BAR`] or a second one on the same bar.
    pub fn tidy_signatures(&mut self) {
        self.sig = (self.sig.0.clamp(1, 99), valid_denominator(self.sig.1));
        self.sig_changes
            .retain(|c| (2..=MAX_SIG_BAR).contains(&c.bar));
        self.sig_changes.sort_by_key(|c| c.bar);
        self.sig_changes.dedup_by_key(|c| c.bar);
        for c in &mut self.sig_changes {
            c.num = c.num.clamp(1, 99);
            c.den = valid_denominator(c.den);
        }
    }

    /// Appends a new track of `kind` (named "Aux 1", "Inst 3"...), selects it and tells the
    /// engine the song changed. Returns its index.
    /// `count` new tracks of `kind`, as many as there is room for. Returns how many were made.
    pub fn add_tracks(&mut self, kind: TrackKind, count: usize) -> usize {
        let mut room = ovrtone_engine::MAX_TRACKS.saturating_sub(self.tracks.len());
        if kind == TrackKind::Aux {
            let auxes = self.tracks.iter().filter(|t| t.kind == kind).count();
            room = room.min(ovrtone_engine::MAX_BUSES.saturating_sub(auxes));
        }
        let made = count.min(room);
        for _ in 0..made {
            self.add_track(kind);
        }
        made
    }

    /// Renames a track. A summing stack and its Aux are one thing and keep one name.
    pub fn rename_track(&mut self, t: usize, name: &str) {
        let Some(old) = self.tracks.get(t).map(|track| track.name.clone()) else {
            return;
        };
        let stack = self.tracks[t].kind == TrackKind::Folder;
        if let Some(aux) = self
            .tracks
            .get_mut(t + 1)
            .filter(|next| stack && next.kind == TrackKind::Aux && next.name == old)
        {
            aux.name = name.to_string();
        }
        self.tracks[t].name = name.to_string();
    }

    /// The pairs of outputs past the first two on this device, by number: 1 is outputs 3-4.
    pub fn hardware_pairs(&self) -> u8 {
        let channels = self
            .output_channels
            .load(std::sync::atomic::Ordering::Relaxed);
        (channels.saturating_sub(2) / 2).min(u32::from(u8::MAX)) as u8
    }

    /// An output strip: an Aux with its own fader that plays to a pair of the device's outputs.
    pub fn new_output_strip(&mut self, pair: u8) -> usize {
        let t = self.add_track(TrackKind::Aux);
        let first = u16::from(pair) * 2 + 1;
        self.tracks[t].name = format!("Output {}-{}", first, first + 1);
        self.tracks[t].hardware_out = pair;
        t
    }

    /// Beat mapping: makes `beat` fall at `at_seconds` from the start of the song, by changing the
    /// tempo since the last tempo point. Audio regions stay at the same time, so that the song's
    /// beats come to fit a recording; MIDI stays on its beats. False if that tempo is out of range.
    pub fn map_beat(&mut self, beat: f64, at_seconds: f64) -> bool {
        let map = self.tempo_map();
        let i = self.tempo_changes.partition_point(|t| t.beat < beat - 1e-6);
        let anchor = i.checked_sub(1).map_or(0.0, |j| self.tempo_changes[j].beat);
        let from = map.seconds_at(anchor);
        if beat <= anchor + 1e-3 || at_seconds <= from {
            return false;
        }
        let bpm = (beat - anchor) * 60.0 / (at_seconds - from);
        if !(f64::from(MIN_BPM)..=f64::from(MAX_BPM)).contains(&bpm) {
            return false;
        }
        // Where the audio is in time, before any of that moves.
        let mut audio = Vec::new();
        for (t, track) in self.tracks.iter().enumerate() {
            for (r, region) in track.regions.iter().enumerate() {
                if region.audio.is_some() {
                    audio.push((t, r, map.seconds_at(self.region_start_beats(region))));
                }
            }
        }
        let after = self.tempo_at(beat);
        match i.checked_sub(1) {
            Some(j) => {
                self.tempo_changes[j].bpm = bpm as f32;
                self.tempo_changes[j].ramp = false;
            }
            None => {
                self.tempo = bpm as f32;
                self.tempo_ramp = false;
            }
        }
        self.add_tempo_change(beat, after);
        let map = self.tempo_map();
        for (t, r, seconds) in audio {
            let start_bar = self.beats_to_bar(map.beat_at(seconds)) as f32 + 1.0;
            self.tracks[t].regions[r].start_bar = start_bar;
        }
        self.song_rev += 1;
        true
    }

    /// Moves regions along the song by `beats`, stopping at its start.
    pub fn nudge_regions(&mut self, regions: &[(usize, usize)], beats: f64) {
        let mut moved = false;
        for &(t, r) in regions {
            let Some(region) = self.tracks.get(t).and_then(|t| t.regions.get(r)) else {
                continue;
            };
            let to = (self.region_start_beats(region) + beats).max(0.0);
            let start_bar = self.beats_to_bar(to) as f32 + 1.0;
            self.tracks[t].regions[r].start_bar = start_bar;
            moved = true;
        }
        if moved {
            self.song_rev += 1;
        }
    }

    /// What feeds the bus that strip `i` plays: each track and how, by its output or a send.
    pub fn feeds_of(&self, i: usize) -> Vec<(usize, String)> {
        let bus = self.aux_bus_of(i);
        if bus == 0 {
            return Vec::new();
        }
        let mut feeds = Vec::new();
        for (t, track) in self.tracks.iter().enumerate() {
            if t == i {
                continue;
            }
            if track.output == bus {
                feeds.push((t, format!("{} (output)", track.name)));
            }
            for send in track.sends.iter().filter(|s| s.bus == Some(bus)) {
                let when = if send.pre { "pre" } else { "post" };
                let percent = (send.level * 100.0).round();
                feeds.push((t, format!("{} (send, {when}, {percent}%)", track.name)));
            }
        }
        feeds
    }

    fn new_region_group(&self) -> u32 {
        let top = self.tracks.iter().flat_map(|t| &t.regions);
        top.map(|r| r.params.group).max().unwrap_or(0) + 1
    }

    /// Makes one clip group of `regions`. Fewer than two make none.
    pub fn group_regions(&mut self, regions: &[(usize, usize)]) -> bool {
        if regions.len() < 2 {
            return false;
        }
        let group = self.new_region_group();
        for &(t, r) in regions {
            if let Some(region) = self.tracks.get_mut(t).and_then(|t| t.regions.get_mut(r)) {
                region.params.group = group;
            }
        }
        self.song_rev += 1;
        true
    }

    /// Ends the clip groups that `regions` are in.
    pub fn ungroup_regions(&mut self, regions: &[(usize, usize)]) {
        let groups: Vec<u32> = regions
            .iter()
            .filter_map(|&(t, r)| self.tracks.get(t)?.regions.get(r))
            .map(|r| r.params.group)
            .filter(|g| *g != 0)
            .collect();
        for region in self.tracks.iter_mut().flat_map(|t| &mut t.regions) {
            if groups.contains(&region.params.group) {
                region.params.group = 0;
            }
        }
        self.song_rev += 1;
    }

    /// Every region in clip group `group`.
    pub fn regions_in_group(&self, group: u32) -> Vec<(usize, usize)> {
        let mut found = Vec::new();
        for (t, track) in self.tracks.iter().enumerate() {
            for (r, region) in track.regions.iter().enumerate() {
                if group != 0 && region.params.group == group {
                    found.push((t, r));
                }
            }
        }
        found
    }

    /// A region loop: `times` more of region `r` of track `t` end to end after it, all in one clip
    /// group with it. They are copies, which can be edited or ungrouped one by one.
    pub fn loop_region(&mut self, t: usize, r: usize, times: usize) -> usize {
        let Some(region) = self.tracks.get(t).and_then(|x| x.regions.get(r)).cloned() else {
            return 0;
        };
        let (start, len) = (
            self.region_start_beats(&region),
            self.region_len_beats(&region),
        );
        if len <= 0.0 || times == 0 {
            return 0;
        }
        let group = if region.params.group != 0 {
            region.params.group
        } else {
            self.new_region_group()
        };
        self.tracks[t].regions[r].params.group = group;
        for k in 1..=times {
            let mut copy = region.clone();
            copy.name = region.name.clone();
            copy.params.group = group;
            copy.start_bar = self.beats_to_bar(start + len * k as f64) as f32 + 1.0;
            self.tracks[t].regions.push(copy);
        }
        self.song_rev += 1;
        times
    }

    /// The Record button and key: starts recording now, from where the song is. While recording,
    /// ends the recording and the song plays on.
    pub fn press_record(&mut self) {
        if self.recording && self.playing {
            self.recording = false;
            // Post-roll: the song plays on for a while, then stops.
            if self.post_roll > 0.0 {
                self.stop_at = Some(self.position_beats + self.bars_as_beats(self.post_roll));
            }
        } else {
            // Pre-roll: from a stop, the song starts earlier and the recording comes in where the
            // playhead was (a punch-in there, put back when the song stops).
            if !self.playing && self.pre_roll > 0.0 && self.roll_restore.is_none() {
                let at = self.position_beats.max(0.0);
                self.roll_restore = Some((self.punch, self.punch_start, self.punch_end));
                self.punch = true;
                self.punch_start = at;
                self.punch_end = f64::from(f32::MAX);
                self.position_beats = (at - self.bars_as_beats(self.pre_roll)).max(0.0);
            }
            self.recording = true;
            self.playing = true;
        }
    }

    /// `bars` bars of the first time signature, in beats.
    pub fn bars_as_beats(&self, bars: f32) -> f64 {
        let (num, den) = (f64::from(self.sig.0.max(1)), f64::from(self.sig.1.max(1)));
        f64::from(bars) * num * 4.0 / den
    }

    /// Call once per frame: ends a post-roll, and puts back what a pre-roll changed once the song
    /// stops.
    pub fn follow_rolls(&mut self) {
        if self.playing {
            // Punching out with a post-roll set: stop that long after the punch's end.
            if self.recording && self.punch && self.post_roll > 0.0 && self.stop_at.is_none()
                && self.roll_restore.is_none() && self.position_beats >= self.punch_end
            {
                self.stop_at = Some(self.punch_end + self.bars_as_beats(self.post_roll));
            }
            if self.stop_at.is_some_and(|at| self.position_beats >= at) {
                self.playing = false;
                self.recording = false;
            }
        }
        if !self.playing {
            self.stop_at = None;
            if let Some((on, start, end)) = self.roll_restore.take() {
                self.punch = on;
                self.punch_start = start;
                self.punch_end = end;
            }
        }
    }

    /// Whether a track of this kind can be copied whole. A stack's contents would have to come too.
    pub fn can_duplicate(&self, t: usize) -> bool {
        t < self.tracks.len()
    }

    /// A copy of track `t` right below it, with its plugins, sends and output, and with its
    /// regions or without. An Aux copy gets a bus of its own. Returns where it is.
    pub fn duplicate_track(&mut self, t: usize, with_regions: bool) -> Option<usize> {
        self.duplicate_tracks(&[t], with_regions).first().copied()
    }

    /// A copy of each of `picked`, of any kind, right below it; a folder's copy has copies of what
    /// is inside it. Returns where all the copies are, and the first of them is then selected.
    pub fn duplicate_tracks(&mut self, picked: &[usize], with_regions: bool) -> Vec<usize> {
        let mut tops: Vec<usize> = picked
            .iter()
            .copied()
            .filter(|t| *t < self.tracks.len())
            .collect();
        tops.sort_unstable();
        tops.dedup();
        // What is inside a picked folder comes with it.
        let mut blocks: Vec<(usize, usize)> = Vec::new();
        for top in tops {
            if blocks.last().is_some_and(|b| top < b.1) {
                continue;
            }
            let end = if self.tracks[top].kind == TrackKind::Folder {
                self.folder_end(top)
            } else {
                top + 1
            };
            blocks.push((top, end));
        }
        // From the last up, so that the places of those still to do stay as they are.
        for &(top, end) in blocks.iter().rev() {
            let copies: Vec<Track> = self.tracks[top..end]
                .iter()
                .map(|track| {
                    let mut copy = track.clone();
                    copy.name = format!("{} copy", copy.name);
                    copy.aux_bus = 0;
                    copy.rec = false;
                    copy.monitor = false;
                    copy.solo = false;
                    if !with_regions {
                        copy.regions.clear();
                        copy.frozen = None;
                    }
                    copy
                })
                .collect();
            let levels: Vec<u8> = copies.iter().map(|c| c.indent).collect();
            let from = self.tracks.len();
            self.tracks.extend(copies);
            let moving: Vec<usize> = (from..self.tracks.len()).collect();
            self.move_tracks(&moving, end);
            // At the level of the originals, not in whatever stack ends just above.
            for (k, level) in levels.into_iter().enumerate() {
                self.tracks[end + k].indent = level;
            }
        }
        self.normalize_nesting();
        let mut made = Vec::new();
        for (top, end) in blocks {
            let at = end + made.len();
            made.extend(at..at + (end - top));
        }
        if let Some(first) = made.first() {
            self.selected_track = *first;
            self.song_rev += 1;
        }
        made
    }

    /// Deletes each of `picked`, as [`Project::delete_track`] does. False if none was there.
    pub fn delete_tracks(&mut self, picked: &[usize]) -> bool {
        let mut going = picked.to_vec();
        going.sort_unstable();
        going.dedup();
        // From the last up, so that the places of those still to go stay as they are.
        let mut any = false;
        for i in going.into_iter().rev() {
            any |= self.delete_track(i);
        }
        any
    }

    /// The colour for a new track: the one in the palette that the fewest tracks have.
    pub fn next_track_color(&self) -> Color32 {
        let uses = |c: &Color32| self.tracks.iter().filter(|t| t.color == *c).count();
        let mut best = TRACK_COLORS[0];
        for color in TRACK_COLORS.iter().take(AUTO_TRACK_COLORS) {
            if uses(color) < uses(&best) {
                best = *color;
            }
        }
        best
    }

    /// In a spatial project a new track that makes sound starts surround panned in the bed (as a
    /// spatial DAW's tracks do); it can be set to Stereo or Object from there.
    pub fn spatial_default(&self, track: &mut Track) {
        let sounds = matches!(
            track.kind,
            TrackKind::Instrument | TrackKind::Audio | TrackKind::Drummer | TrackKind::Aux
        );
        if sounds && self.spatial_mode != ovrtone_engine::SpatialMode::Off && track.object.is_none() {
            track.object = Some(ovrtone_engine::ObjectPlace::default());
            track.bed_pan = true;
        }
    }

    pub fn add_track(&mut self, kind: TrackKind) -> usize {
        let n = self.tracks.iter().filter(|t| t.kind == kind).count() + 1;
        let name = match kind {
            TrackKind::Instrument => format!("Inst {n}"),
            TrackKind::Audio => format!("Audio {n}"),
            TrackKind::Drummer => format!("Drummer {n}"),
            TrackKind::Aux => format!("Aux {n}"),
            TrackKind::Folder => format!("Folder {n}"),
            TrackKind::Vca => format!("VCA {n}"),
            TrackKind::ExternalMidi => format!("External MIDI {n}"),
            TrackKind::Bus => format!("Bus {n}"),
            TrackKind::Output => format!("Output {n}"),
        };
        let mut track = Track::new(name, kind);
        if !matches!(kind, TrackKind::Bus | TrackKind::Output) {
            track.color = self.next_track_color();
        }
        self.spatial_default(&mut track);
        self.tracks.push(track);
        self.selected_track = self.tracks.len() - 1;
        self.song_rev += 1;
        self.selected_track
    }

    /// The bars `r` covers, for making the notes of a generated region: each one's length in
    /// quarter notes and signature, the last cut off where the region ends.
    pub fn region_bars(&self, r: &Region) -> Vec<BarSpec> {
        let len = self.region_len_beats(r);
        let (mut at, mut covered) = (self.region_start_beats(r), 0.0f64);
        let mut bars = Vec::new();
        while covered < len - 1e-9 && bars.len() < MAX_GENERATED_BARS {
            let bar0 = self.beats_to_bar(at + 1e-9).floor().max(0.0) as u32;
            let (num, den) = self.signature_of_bar(bar0);
            let full = self.bar_len(bar0);
            bars.push(BarSpec {
                beats: full.min(len - covered),
                num,
                den,
            });
            covered += full;
            at += full;
        }
        bars
    }

    /// What a generated region's notes are made for: a number that changes with the bars the
    /// region covers (their lengths and signatures), and so with its length.
    pub fn generated_key(&self, r: &Region) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        let mut mix = |word: u64| {
            hash = (hash ^ word).wrapping_mul(0x0000_0100_0000_01b3);
        };
        for bar in self.region_bars(r) {
            mix(bar.beats.to_bits());
            mix(u64::from(bar.num) << 8 | u64::from(bar.den));
        }
        hash
    }

    /// Makes the notes of the generated region `ri` of track `ti` again from its settings and
    /// tells the engine.
    pub fn regenerate(&mut self, ti: usize, ri: usize) {
        let Some(r) = self.tracks.get(ti).and_then(|t| t.regions.get(ri)) else {
            return;
        };
        let Some(generated) = &r.generated else {
            return;
        };
        let notes = match &generated.source {
            Generator::Pattern(p) => p.notes(self.region_len_beats(r)),
            Generator::Drummer(d) => drummer_notes(d, &self.region_bars(r)),
        };
        let key = self.generated_key(r);
        let region = &mut self.tracks[ti].regions[ri];
        region.notes = notes;
        if let Some(g) = region.generated.as_mut() {
            g.made_for = key;
        }
        self.song_rev += 1;
    }

    /// Makes the notes of every generated region whose bars changed (it was lengthened, or a
    /// signature changed under it). Returns whether any was.
    pub fn refresh_generated(&mut self) -> bool {
        let rev = self.song_rev;
        let (mut stale, mut fresh) = (Vec::new(), Vec::new());
        for (ti, t) in self.tracks.iter().enumerate() {
            for (ri, r) in t.regions.iter().enumerate() {
                let Some(g) = &r.generated else {
                    continue;
                };
                // The bars under a region only change when it is moved or resized (its own
                // numbers) or a signature changes (the song revision).
                let seen = (r.start_bar.to_bits(), r.len_bars.to_bits(), rev);
                if g.seen == seen {
                    continue;
                }
                if g.made_for != self.generated_key(r) {
                    stale.push((ti, ri));
                } else {
                    fresh.push((ti, ri, seen));
                }
            }
        }
        for (ti, ri, seen) in fresh {
            if let Some(g) = self.tracks[ti].regions[ri].generated.as_mut() {
                g.seen = seen;
            }
        }
        for &(ti, ri) in &stale {
            self.regenerate(ti, ri);
        }
        !stale.is_empty()
    }

    /// Notes the keys of the generated regions as they are, without making the notes again (a
    /// project was opened with the notes it was saved with).
    pub fn stamp_generated(&mut self) {
        let keys: Vec<(usize, usize, u64)> = self
            .tracks
            .iter()
            .enumerate()
            .flat_map(|(ti, t)| {
                t.regions
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.generated.is_some())
                    .map(move |(ri, r)| (ti, ri, r))
            })
            .map(|(ti, ri, r)| (ti, ri, self.generated_key(r)))
            .collect();
        for (ti, ri, key) in keys {
            if let Some(g) = self.tracks[ti].regions[ri].generated.as_mut() {
                g.made_for = key;
            }
        }
    }

    /// Adds a Pattern or Drummer region of 4 bars to track `ti`, starting at the bar `at_bar`
    /// (1-based), with the notes its settings make. Returns its index; `None` for a track that
    /// holds no MIDI regions.
    /// An empty MIDI region on track `ti`, four bars from `at_bar`. Returns its number.
    pub fn add_midi_region(&mut self, ti: usize, at_bar: f32) -> Option<usize> {
        let track = self.tracks.get_mut(ti)?;
        if !matches!(track.kind, TrackKind::Instrument | TrackKind::ExternalMidi) {
            return None;
        }
        track.regions.push(Region {
            name: track.name.clone(),
            start_bar: at_bar.max(1.0),
            len_bars: 4.0,
            notes: Vec::new(),
            controls: Vec::new(),
            folder: None,
            auto: Vec::new(),
            generated: None,
            gain_line: Vec::new(),
            audio: None,
            params: RegionParams::default(),
        });
        self.song_rev += 1;
        Some(self.tracks[ti].regions.len() - 1)
    }

    pub fn add_generated_region(&mut self, ti: usize, drummer: bool, at_bar: f32) -> Option<usize> {
        let track = self.tracks.get_mut(ti)?;
        if !track.kind.has_regions() || track.kind == TrackKind::Audio {
            return None;
        }
        let drums = track.kind == TrackKind::Drummer;
        let (name, source) = if drummer {
            (
                "Drummer",
                Generator::Drummer(DrummerParams::new(DrumStyle::Rock)),
            )
        } else if drums {
            ("Pattern", Generator::Pattern(PatternParams::drums()))
        } else {
            ("Pattern", Generator::Pattern(PatternParams::melodic()))
        };
        track.regions.push(Region {
            name: name.into(),
            start_bar: at_bar.max(1.0),
            len_bars: 4.0,
            notes: Vec::new(),
            controls: Vec::new(),
            folder: None,
            audio: None,
            params: RegionParams::default(),
            auto: Vec::new(),
            generated: Some(Generated::new(source)),
            gain_line: Vec::new(),
        });
        let ri = track.regions.len() - 1;
        self.regenerate(ti, ri);
        Some(ri)
    }

    /// Turns a generated region into an ordinary MIDI region with the notes it has now, which
    /// can then be edited note by note.
    pub fn convert_to_midi_region(&mut self, ti: usize, ri: usize) {
        if let Some(r) = self.tracks.get_mut(ti).and_then(|t| t.regions.get_mut(ri)) {
            if r.generated.take().is_some() {
                self.song_rev += 1;
            }
        }
    }

    /// The channel name the Inspector shows for track `i`: "Audio 2", "Aux 1" or "Inst 3",
    /// counting the tracks of the same class up to and including it.
    pub fn channel_name(&self, i: usize) -> String {
        let class = |k: TrackKind| match k {
            TrackKind::Audio => 0,
            TrackKind::Aux => 1,
            _ => 2,
        };
        let c = class(self.tracks[i].kind);
        let nth = self.tracks[..=i]
            .iter()
            .filter(|t| class(t.kind) == c)
            .count();
        let label = ["Audio", "Aux", "Inst"][c];
        format!("{label} {nth}")
    }

    /// Index just past the last track inside folder `i` (a track with a higher nesting level
    /// than the folder, directly after it). For a track that is not a folder this is `i + 1`.
    pub fn folder_end(&self, i: usize) -> usize {
        let level = self.tracks[i].indent;
        let mut end = i + 1;
        while end < self.tracks.len() && self.tracks[end].indent > level {
            end += 1;
        }
        end
    }

    /// Whether track `i` is hidden because a folder around it is folded.
    pub fn hidden_by_fold(&self, i: usize) -> bool {
        let mut level = self.tracks[i].indent;
        for j in (0..i).rev() {
            let t = &self.tracks[j];
            if t.indent < level {
                if t.kind == TrackKind::Folder && t.folded {
                    return true;
                }
                level = t.indent;
            }
        }
        false
    }

    /// Whether track `i` is an Aux that shows only in the mixer.
    pub fn mixer_only(&self, i: usize) -> bool {
        self.tracks
            .get(i)
            .is_some_and(|t| t.kind == TrackKind::Aux && !t.on_timeline)
    }

    /// Track > Create Track Stack: puts track `i` (with everything inside it, if it is a
    /// folder) into a new folder at its place, selects the folder and returns its index.
    /// Tracks after it move down one slot, so the engine is told the song changed.
    pub fn create_track_stack(&mut self, i: usize) -> usize {
        if i >= self.tracks.len() {
            return i;
        }
        let (end, level) = (self.folder_end(i), self.tracks[i].indent);
        for t in &mut self.tracks[i..end] {
            t.indent = t.indent.saturating_add(1);
        }
        let n = self
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Folder)
            .count()
            + 1;
        let mut folder = Track::new(format!("Folder {n}"), TrackKind::Folder);
        folder.indent = level;
        self.tracks.insert(i, folder);
        self.vca_inserted(i);
        self.selected_track = i;
        self.song_rev += 1;
        i
    }

    /// `picked`, each with what is inside it when it is a folder, in the order they are in.
    fn with_contents(&self, picked: &[usize]) -> Vec<usize> {
        let mut all = Vec::new();
        for &i in picked.iter().filter(|i| **i < self.tracks.len()) {
            let end = if self.tracks[i].kind == TrackKind::Folder {
                self.folder_end(i)
            } else {
                i + 1
            };
            all.extend(i..end);
        }
        all.sort_unstable();
        all.dedup();
        all
    }

    /// Moves `picked` (and what is inside the folders among them) together to before the track now
    /// at `to`; the length of the list means the end. They join the stack they land in. What refers
    /// to tracks by position goes with them: VCAs, the selected track and, as an Aux strip hears the
    /// bus of its place among the Auxes, the buses of outputs and sends. False if nothing moved.
    pub fn move_tracks(&mut self, picked: &[usize], to: usize) -> bool {
        let moving = self.with_contents(picked);
        let to = to.min(self.tracks.len());
        let (Some(&first), Some(&last)) = (moving.first(), moving.last()) else {
            return false;
        };
        let contiguous = last - first + 1 == moving.len();
        if contiguous && (first..=last + 1).contains(&to) {
            return false;
        }
        // Inside a block that moves there is nowhere to land.
        if moving.contains(&to) && !contiguous {
            return false;
        }
        let stay = |i: &usize| !moving.contains(i);
        let mut order: Vec<usize> = (0..to).filter(stay).collect();
        let at = order.len();
        order.extend(&moving);
        order.extend((to..self.tracks.len()).filter(stay));
        let old_bus: Vec<u16> = (0..self.tracks.len()).map(|i| self.aux_bus_of(i)).collect();
        let mut new_place = vec![0; order.len()];
        for (new, &old) in order.iter().enumerate() {
            new_place[old] = new;
        }
        // What they were in the old order, before the tracks change places.
        let root_level = moving
            .iter()
            .map(|&i| self.tracks[i].indent)
            .min()
            .unwrap_or(0);
        let mut slots: Vec<Option<Track>> = std::mem::take(&mut self.tracks)
            .into_iter()
            .map(Some)
            .collect();
        self.tracks = order.iter().filter_map(|&i| slots[i].take()).collect();
        for t in &mut self.tracks {
            if let Some(v) = t.vca.as_mut() {
                *v = new_place.get(*v).copied().unwrap_or(*v);
            }
        }
        for plugin in self.all_hosted_mut() {
            if let Some(source) = plugin.sidechain.as_mut() {
                *source = new_place.get(*source).copied().unwrap_or(*source);
            }
        }
        for fx in self.tracks.iter_mut().flat_map(|t| t.inserts.iter_mut()) {
            if let Some(source) = fx.sidechain.as_mut() {
                *source = new_place.get(*source).copied().unwrap_or(*source);
            }
        }
        for m in &mut self.midi_map {
            if let LearnTarget::Volume(t) | LearnTarget::Pan(t) = &mut m.target {
                *t = new_place.get(*t).copied().unwrap_or(*t);
            }
        }
        self.selected_track = new_place
            .get(self.selected_track)
            .copied()
            .unwrap_or(self.selected_track);
        // Who hears which bus now: a bus keeps its Aux, under the number of the Aux's new place.
        let mut bus_map: Vec<u16> = (0..=ovrtone_engine::MAX_BUSES as u16).collect();
        for (new, &old) in order.iter().enumerate() {
            let (was, now) = (old_bus[old], self.aux_bus_of(new));
            if was > 0 && now > 0 {
                bus_map[usize::from(was)] = now;
            }
        }
        for t in &mut self.tracks {
            t.output = bus_map[usize::from(t.output)];
            for send in &mut t.sends {
                send.bus = send.bus.map(|b| bus_map[usize::from(b)]);
            }
        }
        // They take the level of the place they land in.
        let level = match at.checked_sub(1).and_then(|i| self.tracks.get(i)) {
            Some(t) if t.kind == TrackKind::Folder => t.indent + 1,
            Some(t) => t.indent,
            None => 0,
        };
        for t in &mut self.tracks[at..at + moving.len()] {
            t.indent = (t.indent - root_level.min(t.indent)).saturating_add(level);
        }
        self.normalize_nesting();
        self.song_rev += 1;
        true
    }

    /// Wraps `picked` in a new stack, bringing them together under the first. Its place.
    pub fn stack_tracks(&mut self, picked: &[usize]) -> Option<usize> {
        let mut roots: Vec<usize> = self
            .with_contents(picked)
            .into_iter()
            .filter(|i| picked.contains(i))
            .collect();
        roots.sort_unstable();
        let &first = roots.first()?;
        self.move_tracks(&roots[1..], self.folder_end(first));
        // Everything picked now follows the first, so the stack goes round the block.
        let count = self.with_contents(&roots).len().max(1);
        let level = self.tracks[first].indent;
        let end = (first + count).min(self.tracks.len());
        for t in &mut self.tracks[first..end] {
            t.indent = t.indent.saturating_sub(level).saturating_add(level + 1);
        }
        let n = self
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Folder)
            .count()
            + 1;
        let mut folder = Track::new(format!("Folder {n}"), TrackKind::Folder);
        folder.indent = level;
        self.tracks.insert(first, folder);
        self.vca_inserted(first);
        self.selected_track = first;
        self.normalize_nesting();
        self.song_rev += 1;
        Some(first)
    }

    /// The buses track `i` plays into, by its output and sends.
    fn buses_fed_by(&self, i: usize) -> impl Iterator<Item = u16> + '_ {
        let t = &self.tracks[i];
        t.sends
            .iter()
            .filter_map(|s| s.bus)
            .chain((t.output > 0).then_some(t.output))
    }

    /// How many Auxes in a row feed track `i`: an Aux runs after those that feed it. A loop, which
    /// the menus do not offer but an edited file could hold, stops counting.
    pub fn aux_rank(&self, i: usize) -> u16 {
        if self.aux_bus_of(i) == 0 {
            return 0;
        }
        let auxes: Vec<usize> = (0..self.tracks.len())
            .filter(|a| self.aux_bus_of(*a) > 0)
            .collect();
        let mut ranks = vec![0u16; self.tracks.len()];
        for _ in 0..auxes.len() {
            let mut changed = false;
            for &from in &auxes {
                for bus in self.buses_fed_by(from) {
                    for &to in &auxes {
                        if to != from
                            && self.aux_bus_of(to) == bus
                            && ranks[to] <= ranks[from]
                            && usize::from(ranks[from]) < auxes.len()
                        {
                            ranks[to] = ranks[from] + 1;
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        ranks[i]
    }

    /// Whether sending track `i` into `bus` would bring its own sound back to it.
    pub fn bus_would_loop(&self, i: usize, bus: u16) -> bool {
        let mut seen = Vec::new();
        let mut waiting = vec![bus];
        while let Some(b) = waiting.pop() {
            if seen.contains(&b) {
                continue;
            }
            seen.push(b);
            for a in 0..self.tracks.len() {
                if self.aux_bus_of(a) == b {
                    if a == i {
                        return true;
                    }
                    waiting.extend(self.buses_fed_by(a));
                }
            }
        }
        false
    }

    /// The buses Aux strips listen to, with the Aux's name, in order.
    pub fn aux_buses(&self) -> Vec<(u16, String)> {
        (0..self.tracks.len())
            .filter_map(|i| {
                let bus = self.aux_bus_of(i);
                (bus > 0).then(|| (bus, self.tracks[i].name.clone()))
            })
            .collect()
    }

    /// Makes a new Aux for track `t`: as its output, so its whole sound goes through the Aux (a
    /// group), or as a send, so a copy of it does (an effect). The Aux's place, or `None` when the
    /// strip has no free send or there can be no more tracks.
    pub fn new_aux_for(&mut self, t: usize, as_send: bool) -> Option<usize> {
        let track = self.tracks.get(t)?;
        if as_send && track.sends.len() >= MAX_SENDS {
            return None;
        }
        // At the end, so no other Aux changes its number.
        let aux = self.add_track(TrackKind::Aux);
        let bus = self.aux_bus_of(aux);
        if bus == 0 {
            self.delete_track(aux);
            return None;
        }
        if as_send {
            self.tracks[t].sends.push(SendSlot {
                bus: Some(bus),
                level: 0.71,
                pre: false,
            });
        } else {
            self.tracks[t].output = bus;
        }
        self.selected_track = t;
        self.song_rev += 1;
        Some(aux)
    }

    /// A stack whose tracks all go through one Aux, which is first inside it: one fader for the
    /// lot. The folder's place, or `None` when no bus is free.
    pub fn summing_stack(&mut self, picked: &[usize]) -> Option<usize> {
        let auxes = self
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Aux)
            .count();
        if auxes >= ovrtone_engine::MAX_BUSES {
            return None;
        }
        let folder = self.stack_tracks(picked)?;
        let end = self.folder_end(folder);
        let aux = self.add_track(TrackKind::Aux);
        self.move_tracks(&[aux], folder + 1);
        let aux = folder + 1;
        // The stack in the Tracks area and the Aux in the mixer are one thing: one name, one row.
        let n = self
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Folder && t.name.starts_with("Summing "))
            .count()
            + 1;
        let name = format!("Summing {n}");
        self.tracks[aux].name = name.clone();
        self.tracks[aux].on_timeline = false;
        self.tracks[folder].name = name;
        let bus = self.aux_bus_of(aux);
        // The move put the Aux one further into the stack.
        for t in &mut self.tracks[aux + 1..=end] {
            if !matches!(t.kind, TrackKind::Folder | TrackKind::Vca) {
                t.output = bus;
            }
        }
        self.selected_track = folder;
        self.song_rev += 1;
        Some(folder)
    }

    /// Track > Release Track Stack: removes folder `i` and lifts the tracks inside it one
    /// level. Returns false (and changes nothing) when `i` is not a folder.
    pub fn release_track_stack(&mut self, i: usize) -> bool {
        if self
            .tracks
            .get(i)
            .is_none_or(|t| t.kind != TrackKind::Folder)
        {
            return false;
        }
        let end = self.folder_end(i);
        for t in &mut self.tracks[i + 1..end] {
            t.indent = t.indent.saturating_sub(1);
        }
        self.tracks.remove(i);
        self.vca_removed(i);
        if self.selected_track > i {
            self.selected_track -= 1;
        }
        self.selected_track = self.selected_track.min(self.tracks.len().saturating_sub(1));
        self.song_rev += 1;
        true
    }

    /// Keeps VCA assignments pointing at the right tracks after a track was inserted at `at`.
    pub fn vca_inserted(&mut self, at: usize) {
        for t in &mut self.tracks {
            if let Some(v) = t.vca.as_mut() {
                if *v >= at {
                    *v += 1;
                }
            }
        }
        for plugin in self.all_hosted_mut() {
            if let Some(source) = plugin.sidechain.as_mut() {
                if *source >= at {
                    *source += 1;
                }
            }
        }
        for fx in self.tracks.iter_mut().flat_map(|t| t.inserts.iter_mut()) {
            if let Some(source) = fx.sidechain.as_mut() {
                if *source >= at {
                    *source += 1;
                }
            }
        }
        for m in &mut self.midi_map {
            if let LearnTarget::Volume(t) | LearnTarget::Pan(t) = &mut m.target {
                if *t >= at {
                    *t += 1;
                }
            }
        }
    }

    /// Keeps VCA assignments right after track `removed` was taken out: tracks that were
    /// assigned to it have no VCA any more.
    pub fn vca_removed(&mut self, removed: usize) {
        for t in &mut self.tracks {
            t.vca = match t.vca {
                Some(v) if v == removed => None,
                Some(v) if v > removed => Some(v - 1),
                other => other,
            };
        }
        // The same for side chains: a plugin keyed by it has no source any more.
        for plugin in self.all_hosted_mut() {
            plugin.sidechain = match plugin.sidechain {
                Some(s) if s == removed => None,
                Some(s) if s > removed => Some(s - 1),
                other => other,
            };
        }
        for fx in self.tracks.iter_mut().flat_map(|t| t.inserts.iter_mut()) {
            fx.sidechain = match fx.sidechain {
                Some(s) if s == removed => None,
                Some(s) if s > removed => Some(s - 1),
                other => other,
            };
        }
        // And for hardware controls: the ones on it go with it.
        self.midi_map.retain_mut(|m| match &mut m.target {
            LearnTarget::Volume(t) | LearnTarget::Pan(t) => {
                if *t == removed {
                    return false;
                }
                if *t > removed {
                    *t -= 1;
                }
                true
            }
            _ => true,
        });
    }

    /// The VCA track that controls track `i`, if it has one.
    pub fn vca_of(&self, i: usize) -> Option<&Track> {
        let t = self.tracks.get(i)?;
        let vca = self.tracks.get(t.vca?)?;
        (vca.kind == TrackKind::Vca && t.kind != TrackKind::Vca).then_some(vca)
    }

    /// Assigns track `i` to the VCA track `vca` (or to none). Only a track that is not a VCA
    /// itself can be assigned, and only to a VCA track; returns whether it changed.
    pub fn assign_vca(&mut self, i: usize, vca: Option<usize>) -> bool {
        let valid =
            vca.is_none_or(|v| self.tracks.get(v).is_some_and(|t| t.kind == TrackKind::Vca));
        match self.tracks.get(i) {
            Some(t) if valid && t.kind != TrackKind::Vca && t.vca != vca => {
                self.tracks[i].vca = vca;
                true
            }
            _ => false,
        }
    }

    /// Makes a mix group named `name` (with every attribute on) and returns its index, or
    /// `None` at the limit.
    pub fn add_group(&mut self, name: &str) -> Option<usize> {
        if self.groups.len() >= MAX_GROUPS {
            return None;
        }
        self.groups.push(MixGroup::new(name));
        Some(self.groups.len() - 1)
    }

    /// Puts track `i` in group `g` (or in none). Folders and VCA tracks have no strip to group.
    pub fn assign_group(&mut self, i: usize, g: Option<usize>) -> bool {
        let valid = g.is_none_or(|g| g < self.groups.len());
        match self.tracks.get_mut(i) {
            Some(t) if valid && !matches!(t.kind, TrackKind::Folder | TrackKind::Vca) => {
                let changed = t.group != g;
                t.group = g;
                changed
            }
            _ => false,
        }
    }

    /// Takes group `g` away; its tracks are left without one, and the groups after it move up.
    pub fn delete_group(&mut self, g: usize) {
        if g >= self.groups.len() {
            return;
        }
        self.groups.remove(g);
        for t in self.tracks.iter_mut() {
            t.group = match t.group {
                Some(x) if x == g => None,
                Some(x) if x > g => Some(x - 1),
                other => other,
            };
        }
    }

    /// File > Import > Other Projects: copies of the chosen tracks of another project, with their
    /// regions, effects and plugins. Its buses and groups mean nothing here, so they go to Stereo
    /// Out with no sends. How many came.
    pub fn import_tracks(&mut self, from: &Project, picks: &[bool]) -> usize {
        let mut count = 0;
        for (t, _) in from.tracks.iter().zip(picks).filter(|(_, on)| **on) {
            if self.tracks.len() >= ovrtone_engine::MAX_TRACKS
                || matches!(t.kind, TrackKind::Folder | TrackKind::Vca)
            {
                continue;
            }
            let mut copy = t.clone();
            copy.output = 0;
            copy.sends.clear();
            copy.group = None;
            copy.vca = None;
            copy.indent = 0;
            copy.frozen = None;
            for plugin in copy.hosted_mut() {
                plugin.sidechain = None;
            }
            self.tracks.push(copy);
            count += 1;
        }
        if count > 0 {
            self.song_rev += 1;
        }
        count
    }

    /// The Aux that Create Mastering Track made, if it is still there.
    pub fn mastering_track(&self) -> Option<usize> {
        self.tracks
            .iter()
            .position(|t| t.kind == TrackKind::Aux && t.name == MASTERING_TRACK)
    }

    /// Mix > Create Mastering Track: an Aux at the end of the mix that every strip going to Stereo
    /// Out now goes through, for what works on the whole mix and for its automation. `None` when
    /// every bus is taken.
    pub fn create_mastering_track(&mut self) -> Option<usize> {
        if let Some(t) = self.mastering_track() {
            return Some(t);
        }
        let aux = self.add_track(TrackKind::Aux);
        let bus = self.aux_bus_of(aux);
        if bus == 0 {
            self.tracks.remove(aux);
            return None;
        }
        self.tracks[aux].name = MASTERING_TRACK.to_string();
        for (i, t) in self.tracks.iter_mut().enumerate() {
            let routed = !matches!(t.kind, TrackKind::Folder | TrackKind::Vca);
            if i != aux && routed && t.output == 0 {
                t.output = bus;
            }
        }
        self.song_rev += 1;
        Some(aux)
    }

    /// The other tracks of track `i`'s group, when the group has the attribute `attr` (a
    /// function of the group).
    pub fn group_mates(&self, i: usize, attr: impl Fn(&MixGroup) -> bool) -> Vec<usize> {
        let mut mates: Vec<usize> = match self.tracks.get(i).and_then(|t| t.group) {
            Some(g) if self.groups.get(g).is_some_and(attr) => (0..self.tracks.len())
                .filter(|j| *j != i && self.tracks[*j].group == Some(g))
                .collect(),
            _ => Vec::new(),
        };
        // Strips selected together move together for as long as they stay selected.
        if self.linked_strips.contains(&i) {
            for &j in &self.linked_strips {
                if j != i && j < self.tracks.len() && !mates.contains(&j) {
                    mates.push(j);
                }
            }
        }
        mates
    }

    /// Sets Mute on track `i` and on the tracks of its group, if the group has Mute.
    pub fn set_mute(&mut self, i: usize, on: bool) {
        for j in std::iter::once(i).chain(self.group_mates(i, |g| g.mute)) {
            if let Some(t) = self.tracks.get_mut(j) {
                t.mute = on;
            }
        }
    }

    /// Sets Solo on track `i` and on the tracks of its group, if the group has Solo.
    pub fn set_solo(&mut self, i: usize, on: bool) {
        for j in std::iter::once(i).chain(self.group_mates(i, |g| g.solo)) {
            if let Some(t) = self.tracks.get_mut(j) {
                t.solo = on;
            }
        }
    }

    /// The bus an Aux strip listens to: Aux 1 takes Bus 1, the next Aux Bus 2, and so on up to
    /// Bus 8. 0 for any other strip, and for an Aux strip past the eighth.
    pub fn aux_bus_of(&self, i: usize) -> u16 {
        let is_aux = |t: &Track| t.kind == TrackKind::Aux;
        let Some(t) = self.tracks.get(i).filter(|t| is_aux(t)) else {
            return 0;
        };
        let max = ovrtone_engine::MAX_BUSES;
        if t.aux_bus > 0 {
            return t.aux_bus.min(max as u16);
        }
        // Those not chosen by hand take the buses left, in the order of the tracks.
        let nth = self.tracks[..=i]
            .iter()
            .filter(|t| is_aux(t) && t.aux_bus == 0)
            .count();
        let chosen: Vec<u16> = self
            .tracks
            .iter()
            .filter(|t| is_aux(t))
            .map(|t| t.aux_bus)
            .collect();
        if chosen.iter().all(|b| *b == 0) {
            return if nth <= max { nth as u16 } else { 0 };
        }
        (1..=max as u16)
            .filter(|b| !chosen.contains(b))
            .nth(nth - 1)
            .unwrap_or(0)
    }

    /// Has the Aux at `i` listen to `bus`. What plays into a bus stays on it, so the Aux hears what
    /// plays there now. An Aux already on that bus takes this one's, and every other Aux keeps its
    /// number from here on, so that none changes by this.
    pub fn set_aux_bus(&mut self, i: usize, bus: u16) {
        if bus == 0 || usize::from(bus) > ovrtone_engine::MAX_BUSES || self.aux_bus_of(i) == 0 {
            return;
        }
        let now: Vec<u16> = (0..self.tracks.len()).map(|t| self.aux_bus_of(t)).collect();
        let old = now[i];
        for (t, number) in now.iter().enumerate() {
            let track = &mut self.tracks[t];
            if track.kind == TrackKind::Aux {
                track.aux_bus = if t == i {
                    bus
                } else if *number == bus {
                    old
                } else {
                    *number
                };
            }
        }
        self.song_rev += 1;
    }

    /// Whether track `i` is muted, by its own Mute or by its VCA's.
    pub fn is_muted(&self, i: usize) -> bool {
        self.tracks.get(i).is_some_and(|t| t.mute || t.off)
            || self.vca_of(i).is_some_and(|v| v.mute)
    }

    /// Whether track `i` is soloed, by its own Solo or by its VCA's. A VCA track is never
    /// soloed itself: it makes no sound, its Solo reaches the tracks assigned to it.
    pub fn is_soloed(&self, i: usize) -> bool {
        self.tracks
            .get(i)
            .is_some_and(|t| t.kind != TrackKind::Vca && t.solo)
            || self.vca_of(i).is_some_and(|v| v.solo)
    }

    /// Turns Cycle on or off. Coming on, it takes the range of what is selected. With nothing
    /// selected it keeps the locators, unless they would not loop what is there: past the end of
    /// the song, or running on long after it. Then it takes the whole song.
    pub fn toggle_cycle(&mut self, selection: Option<(f64, f64)>) {
        self.cycle = !self.cycle;
        if !self.cycle {
            return;
        }
        if let Some((start, end)) = selection.filter(|(a, b)| b > a) {
            self.cycle_start = start;
            self.cycle_end = end;
            return;
        }
        let song_end = crate::bounce::project_end(self);
        let out_of_reach = self.cycle_start >= song_end
            || self.cycle_end - song_end >= f64::from(CYCLE_OVERRUN_BARS) * self.bar_len_at(0.0);
        if song_end > 0.0 && out_of_reach {
            self.cycle_start = 0.0;
            self.cycle_end = self.next_bar_start(song_end - 1e-9);
        }
    }

    /// Whether Autopunch lets recording happen at the playhead.
    pub fn punched_in(&self) -> bool {
        !self.punch || (self.punch_start..self.punch_end).contains(&self.position_beats)
    }

    /// Every hosted plugin in the project.
    pub fn all_hosted_mut(&mut self) -> impl Iterator<Item = &mut PluginInsert> {
        self.tracks
            .iter_mut()
            .flat_map(Track::hosted_mut)
            .chain(&mut self.master_plugins)
    }

    /// Whether any track is soloed (the others then fall silent).
    pub fn any_solo(&self) -> bool {
        (0..self.tracks.len()).any(|i| self.is_soloed(i))
    }

    /// What the engine mixes track `i` with: its level plus its VCA's, and Mute and Solo from
    /// the track or its VCA. A VCA track itself is left at unity: it has no sound.
    pub fn track_mix_of(&self, i: usize) -> (f32, bool, bool) {
        let Some(t) = self.tracks.get(i) else {
            return (0.0, false, false);
        };
        if t.kind == TrackKind::Vca {
            return (0.0, false, false);
        }
        let vca_db = self.vca_of(i).map_or(0.0, |v| v.volume_db);
        (t.volume_db + vca_db, self.is_muted(i), self.is_soloed(i))
    }

    /// Track > Delete Track: removes track `i` with its regions. Deleting a folder keeps the
    /// tracks inside it (as Release Track Stack does). A project can end up with no tracks.
    /// Returns false when `i` does not exist.
    pub fn delete_track(&mut self, i: usize) -> bool {
        if i >= self.tracks.len() {
            return false;
        }
        if self.tracks[i].kind == TrackKind::Folder {
            return self.release_track_stack(i);
        }
        self.tracks.remove(i);
        self.vca_removed(i);
        if self.selected_track > i {
            self.selected_track -= 1;
        }
        self.selected_track = self.selected_track.min(self.tracks.len().saturating_sub(1));
        self.normalize_nesting();
        self.song_rev += 1;
        true
    }

    /// Sets Mute and/or Solo on folder `i` and on every track inside it, as a folder's M and S
    /// buttons do.
    pub fn set_folder_flags(&mut self, i: usize, mute: Option<bool>, solo: Option<bool>) {
        let end = self.folder_end(i);
        for t in &mut self.tracks[i..end] {
            t.mute = mute.unwrap_or(t.mute);
            t.solo = solo.unwrap_or(t.solo);
        }
    }

    /// Repairs folder nesting: a track can sit at most one level below the track before it,
    /// and only when that one is a `Folder`. Run after loading a file that may be hand-edited.
    pub fn normalize_nesting(&mut self) {
        let mut allowed = 0u8;
        for t in &mut self.tracks {
            t.indent = t.indent.min(allowed);
            allowed = t.indent + u8::from(t.kind == TrackKind::Folder);
        }
    }

    /// Moves one half of the time signature by `steps` (up is positive) and returns whether it
    /// changed. The numerator runs 1-99, the denominator through the powers of two 1-64; both
    /// stop at their ends. Regions sit on bars, so a change moves them in time: the song
    /// revision is bumped to have the engine reposition them.
    pub fn step_signature(&mut self, part: SigPart, steps: i32) -> bool {
        self.set_signature_from(1, stepped_signature(self.sig, part, steps))
    }

    /// [`Project::step_signature`] for the signature in force at the 0-based bar `bar0`: the
    /// change that holds there (or the song's own) is the one that moves.
    pub fn step_signature_at(&mut self, bar0: u32, part: SigPart, steps: i32) -> bool {
        let from = self
            .sig_changes
            .iter()
            .rev()
            .find(|c| c.bar <= bar0 + 1)
            .map_or(1, |c| c.bar);
        let now = self.signature_of_bar(bar0);
        self.set_signature_from(from, stepped_signature(now, part, steps))
    }

    /// Logic-style position for a beat: (bar, beat, division, tick), all 1-based, where
    /// "beat" is the signature's beat (an eighth in 6/8), divisions are 16th notes and there
    /// are 240 ticks per division.
    pub fn bbdt(&self, beats: f64) -> (u32, u32, u32, u32) {
        let beats = beats.max(0.0);
        let bar0 = self.beats_to_bar(beats) as u32;
        let unit = unit_of(self.signature_of_bar(bar0).1);
        let bar = bar0 + 1;
        let in_beats = (beats - self.bar_to_beats(bar0 as f64)).max(0.0) / unit;
        let beat = in_beats as u32 + 1;
        let divs = (unit * 4.0).max(1.0);
        let sixteenths = in_beats.fract() * divs;
        let div = sixteenths as u32 + 1;
        let tick = (sixteenths.fract() * 240.0) as u32 + 1;
        (bar, beat, div, tick)
    }

    pub fn bar_beat_tick(&self) -> (u32, u32, u32) {
        let beats = self.position_beats.max(0.0);
        let bar0 = self.beats_to_bar(beats) as u32;
        let unit = unit_of(self.signature_of_bar(bar0).1);
        let bar = bar0 + 1;
        let in_beats = (beats - self.bar_to_beats(bar0 as f64)).max(0.0) / unit;
        let beat = in_beats as u32 + 1;
        let tick = (in_beats.fract() * 960.0) as u32;
        (bar, beat, tick)
    }

    /// The project start time, in frames of the SMPTE frame rate.
    pub fn smpte_offset_frames(&self) -> i64 {
        let [h, m, s, f] = self.smpte_start;
        self.smpte_rate.frames_of(ovrtone_engine::Smpte {
            h,
            m,
            s,
            f,
            subframe: 0,
        })
    }

    /// The SMPTE time of `beat` as Logic's display shows it: `hh:mm:ss:ff.ss`, 80 subframes
    /// to a frame, with a `;` before the frames in drop frame. It follows every tempo change
    /// and curve.
    pub fn timecode_at(&self, beat: f64) -> String {
        let rate = self.smpte_rate;
        let frames =
            self.tempo_map()
                .smpte_frames_at(beat.max(0.0), rate, self.smpte_offset_frames());
        let t = rate.smpte_of_frames(frames);
        // The same rounding the label got, so a position on a frame shows `.00`.
        let whole = (frames * 1e6).round() / 1e6;
        let sub = ((whole - whole.floor()) * 80.0) as u32;
        let sep = if rate.is_drop_frame() { ';' } else { ':' };
        format!("{:02}:{:02}:{:02}{sep}{:02}.{sub:02}", t.h, t.m, t.s, t.f)
    }

    /// [`Project::timecode_at`] the playhead.
    pub fn timecode(&self) -> String {
        self.timecode_at(self.position_beats)
    }
}

/// `sig` with one half moved by `steps`: the numerator runs 1-99, the denominator through the
/// powers of two 1-64; both stop at their ends.
fn stepped_signature(sig: (u8, u8), part: SigPart, steps: i32) -> (u8, u8) {
    match part {
        SigPart::Numerator => ((i32::from(sig.0) + steps).clamp(1, 99) as u8, sig.1),
        SigPart::Denominator => {
            let at = SIGNATURE_DENOMINATORS
                .iter()
                .position(|d| *d == sig.1)
                .unwrap_or(2);
            let last = SIGNATURE_DENOMINATORS.len() as i32 - 1;
            let to = (at as i32 + steps).clamp(0, last) as usize;
            (sig.0, SIGNATURE_DENOMINATORS[to])
        }
    }
}

/// A time-signature change on the Signature track: `num/den` from the start of the 1-based
/// `bar` (always 2 or later; bar 1 is [`Project::sig`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SigChange {
    pub bar: u32,
    pub num: u8,
    pub den: u8,
}

/// The signature track seen from outside the [`Project`]: the first bar's signature and the
/// changes after it, with the bar maths. Copy it from [`Project::sigs`]; [`SigRef::to_owned`]
/// gives a copy that does not borrow the project.
#[derive(Clone, Copy)]
pub struct SigRef<'a> {
    pub first: (u8, u8),
    pub changes: &'a [SigChange],
}

/// [`SigRef`] that owns its changes, to move into closures while the project is edited.
#[derive(Clone, Debug, PartialEq)]
pub struct SigOwned {
    pub first: (u8, u8),
    pub changes: Vec<SigChange>,
}

impl SigOwned {
    pub fn as_ref(&self) -> SigRef<'_> {
        SigRef {
            first: self.first,
            changes: &self.changes,
        }
    }
}

impl SigRef<'_> {
    pub fn to_owned(self) -> SigOwned {
        SigOwned {
            first: self.first,
            changes: self.changes.to_vec(),
        }
    }

    pub fn signature_of_bar(self, bar0: u32) -> (u8, u8) {
        let n = self.changes.partition_point(|c| c.bar <= bar0 + 1);
        n.checked_sub(1)
            .map_or(self.first, |i| (self.changes[i].num, self.changes[i].den))
    }

    pub fn bar_len(self, bar0: u32) -> f64 {
        let (num, den) = self.signature_of_bar(bar0);
        num.max(1) as f64 * unit_of(den)
    }

    pub fn bar_to_beats(self, bar0: f64) -> f64 {
        let bar0 = bar0.max(0.0);
        let (mut beats, mut at, mut len) = (0.0, 0.0, self.bar_len(0));
        for c in self.changes {
            let start = (c.bar - 1) as f64;
            if start >= bar0 {
                break;
            }
            beats += (start - at) * len;
            at = start;
            len = c.num.max(1) as f64 * unit_of(c.den);
        }
        beats + (bar0 - at) * len
    }

    pub fn beats_to_bar(self, beats: f64) -> f64 {
        let beats = beats.max(0.0);
        let (mut at_beats, mut at, mut len) = (0.0, 0.0, self.bar_len(0));
        for c in self.changes {
            let start = (c.bar - 1) as f64;
            let start_beats = at_beats + (start - at) * len;
            if start_beats > beats {
                break;
            }
            at_beats = start_beats;
            at = start;
            len = c.num.max(1) as f64 * unit_of(c.den);
        }
        at + (beats - at_beats) / len
    }

    pub fn beat_unit_at(self, beats: f64) -> f64 {
        unit_of(self.signature_of_bar(self.beats_to_bar(beats) as u32).1)
    }

    pub fn bar_len_at(self, beats: f64) -> f64 {
        self.bar_len(self.beats_to_bar(beats) as u32)
    }

    pub fn bar_start(self, beats: f64) -> f64 {
        self.bar_to_beats(self.beats_to_bar(beats).floor())
    }

    pub fn next_bar_start(self, beats: f64) -> f64 {
        self.bar_to_beats(self.beats_to_bar(beats).floor() + 1.0)
    }

    /// The bar line nearest to `beats`.
    pub fn nearest_bar_line(self, beats: f64) -> f64 {
        self.bar_to_beats(self.beats_to_bar(beats).round())
    }

    pub fn bars_in(self, from: f64, to: f64) -> Vec<BarInfo> {
        let first = self.beats_to_bar(from.max(0.0)).floor() as u32;
        let mut out = Vec::new();
        for bar0 in first..first.saturating_add(4096) {
            let start = self.bar_to_beats(bar0 as f64);
            if start > to {
                break;
            }
            let (num, den) = self.signature_of_bar(bar0);
            out.push(BarInfo {
                bar0,
                start,
                num,
                unit: unit_of(den),
            });
        }
        out
    }
}

/// One bar of the song, for drawing grids and rulers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarInfo {
    /// 0-based bar number.
    pub bar0: u32,
    /// Where it starts, in quarter notes.
    pub start: f64,
    /// Beats in the bar (the signature's numerator).
    pub num: u8,
    /// Quarter notes in one beat of the bar.
    pub unit: f64,
}

/// The last bar a signature change may start at.
pub const MAX_SIG_BAR: u32 = 9999;

/// Length of one signature beat in quarter notes: 1 for x/4, 0.5 for x/8, 2 for x/2. An
/// invalid denominator counts as 4.
fn unit_of(den: u8) -> f64 {
    match den {
        d @ (1 | 2 | 4 | 8 | 16 | 32 | 64) => 4.0 / d as f64,
        _ => 1.0,
    }
}

/// `den` if a time signature can have it, else 4.
fn valid_denominator(den: u8) -> u8 {
    if SIGNATURE_DENOMINATORS.contains(&den) {
        den
    } else {
        4
    }
}

/// The beat lengths a time signature can have (its denominator), as Logic offers them.
pub const SIGNATURE_DENOMINATORS: [u8; 7] = [1, 2, 4, 8, 16, 32, 64];

/// One half of a time signature.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SigPart {
    Numerator,
    Denominator,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_comes_on_over_the_selection_or_where_it_can_be_heard() {
        let mut p = Project::demo();
        p.cycle = false;
        p.toggle_cycle(Some((4.0, 12.0)));
        assert!(p.cycle);
        assert_eq!((p.cycle_start, p.cycle_end), (4.0, 12.0));
        p.toggle_cycle(Some((8.0, 16.0)));
        assert!(!p.cycle, "off again");
        assert_eq!(p.cycle_end, 12.0, "going off moves nothing");
        // Locators far past the song loop nothing that is there.
        let end = crate::bounce::project_end(&p);
        p.cycle_start = end + 40.0;
        p.cycle_end = end + 80.0;
        p.toggle_cycle(None);
        assert_eq!(p.cycle_start, 0.0);
        assert!(
            p.cycle_end >= end && p.cycle_end < end + 4.0,
            "{} {end}",
            p.cycle_end
        );
        // Locators inside the song stay.
        p.cycle = false;
        p.cycle_start = 4.0;
        p.cycle_end = 8.0;
        p.toggle_cycle(None);
        assert_eq!((p.cycle_start, p.cycle_end), (4.0, 8.0));
    }

    #[test]
    fn signature_parts_step_and_stop_at_their_ends() {
        let mut p = Project::demo();
        assert_eq!(p.sig, (4, 4));
        assert!(p.step_signature(SigPart::Numerator, 2));
        assert!(p.step_signature(SigPart::Denominator, 1));
        assert_eq!(p.sig, (6, 8), "the denominator steps through powers of two");
        assert!(p.step_signature(SigPart::Denominator, 3));
        assert_eq!(p.sig, (6, 64));
        assert!(!p.step_signature(SigPart::Denominator, 5), "stops at 64");
        assert!(p.step_signature(SigPart::Denominator, -9));
        assert_eq!(p.sig.1, 1, "and at 1");
        p.step_signature(SigPart::Numerator, 500);
        assert_eq!(p.sig.0, 99);
        p.step_signature(SigPart::Numerator, -500);
        assert_eq!(p.sig.0, 1);
    }

    #[test]
    fn a_changed_signature_moves_bars_and_asks_the_engine_to_follow() {
        let mut p = Project::demo();
        let rev = p.song_rev;
        assert_eq!(p.bar_len(0), 4.0);
        assert!(p.step_signature(SigPart::Numerator, -1));
        assert!(p.step_signature(SigPart::Denominator, 1));
        assert_eq!(p.sig, (3, 8));
        assert_eq!(p.bar_len(0), 1.5);
        assert_eq!(p.song_rev, rev + 2);

        // Nothing changes at an end stop, so there is nothing to resend.
        let mut q = Project::demo();
        let rev = q.song_rev;
        assert!(!q.step_signature(SigPart::Numerator, 0));
        assert_eq!(q.song_rev, rev);
    }

    /// 4/4 for two bars, 3/4 for two, 6/8 for two, then 7/8: bars 3, 5 and 7 (1-based).
    fn mixed_signatures() -> Project {
        let mut p = Project::demo();
        assert!(p.set_signature_from(3, (3, 4)));
        assert!(p.set_signature_from(5, (6, 8)));
        assert!(p.set_signature_from(7, (7, 8)));
        p
    }

    #[test]
    fn bars_have_the_length_of_their_signature() {
        let p = mixed_signatures();
        let lens: Vec<f64> = (0..8).map(|b| p.bar_len(b)).collect();
        assert_eq!(lens, [4.0, 4.0, 3.0, 3.0, 3.0, 3.0, 3.5, 3.5]);
        assert_eq!(p.signature_of_bar(1), (4, 4));
        assert_eq!(p.signature_of_bar(2), (3, 4));
        assert_eq!(p.signature_of_bar(4), (6, 8));
        assert_eq!(p.signature_of_bar(99), (7, 8));
        // Quarter notes at the start of each bar.
        let starts: Vec<f64> = (0..9).map(|b| p.bar_to_beats(b as f64)).collect();
        assert_eq!(starts, [0.0, 4.0, 8.0, 11.0, 14.0, 17.0, 20.0, 23.5, 27.0]);
        // Half way through the 3/4 bar.
        assert_eq!(p.bar_to_beats(2.5), 9.5);
        assert_eq!(p.beats_to_bar(9.5), 2.5);
    }

    #[test]
    fn beats_and_bars_convert_back_and_forth() {
        let p = mixed_signatures();
        for i in 0..400 {
            let beats = i as f64 * 0.37;
            let back = p.bar_to_beats(p.beats_to_bar(beats));
            assert!((back - beats).abs() < 1e-9, "{beats} -> {back}");
        }
        assert_eq!(p.bar_start(12.0), 11.0);
        assert_eq!(p.next_bar_start(12.0), 14.0);
        assert_eq!(p.next_bar_start(14.0), 17.0);
        assert_eq!(p.beat_unit_at(1.0), 1.0, "x/4 beat");
        assert_eq!(p.beat_unit_at(15.0), 0.5, "x/8 beat");
    }

    #[test]
    fn positions_read_in_the_signature_of_their_bar() {
        let mut p = mixed_signatures();
        // 6/8: bar 5 starts at 14; one eighth in is 14.5 -> bar 5, beat 2.
        assert_eq!(p.bbdt(14.5), (5, 2, 1, 1));
        // 3/4: bar 3 starts at 8; beat 3 of that bar is at 10.
        assert_eq!(p.bbdt(10.0), (3, 3, 1, 1));
        p.position_beats = 10.0;
        assert_eq!(p.bar_beat_tick(), (3, 3, 0));
        p.position_beats = 23.5;
        assert_eq!(p.bar_beat_tick(), (8, 1, 0));
    }

    #[test]
    fn regions_sit_on_bars_whatever_the_signatures() {
        let p = mixed_signatures();
        let mut r = p.tracks[0].regions[0].clone();
        r.start_bar = 4.0;
        r.len_bars = 2.0;
        assert_eq!(p.region_start_beats(&r), 11.0);
        assert_eq!(p.region_end_beats(&r), 17.0);
        assert_eq!(p.region_len_beats(&r), 6.0);
    }

    #[test]
    fn signature_changes_are_kept_in_order_and_edited_in_place() {
        let mut p = Project::demo();
        let rev = p.song_rev;
        assert!(p.set_signature_from(9, (5, 4)));
        assert!(p.set_signature_from(3, (3, 4)));
        assert!(
            !p.set_signature_from(3, (3, 4)),
            "the same again changes nothing"
        );
        assert!(
            p.set_signature_from(3, (3, 8)),
            "edits the change on that bar"
        );
        let bars: Vec<u32> = p.sig_changes.iter().map(|c| c.bar).collect();
        assert_eq!(bars, [3, 9]);
        assert_eq!(
            p.sig_changes[0],
            SigChange {
                bar: 3,
                num: 3,
                den: 8
            }
        );
        assert!(p.song_rev > rev, "the engine is asked to follow");
        // Bar 1 is the song's own signature.
        assert!(p.set_signature_from(1, (6, 8)));
        assert_eq!(p.sig, (6, 8));
        assert_eq!(p.sig_changes.len(), 2);
        // Bad values are fixed; a change can be removed, bar 1's cannot.
        assert!(p.set_signature_from(20, (0, 5)));
        assert_eq!(p.signature_of_bar(25), (1, 4));
        assert!(p.remove_signature_at(20));
        assert!(!p.remove_signature_at(20));
        assert!(!p.remove_signature_at(1));
    }

    #[test]
    fn stepping_edits_the_signature_in_force_at_a_bar() {
        let mut p = Project::demo();
        p.set_signature_from(5, (3, 4));
        // Bar 7 (0-based 6) is in the 3/4 block: that block changes, not the first.
        assert!(p.step_signature_at(6, SigPart::Numerator, 1));
        assert_eq!(p.sig, (4, 4));
        assert_eq!(
            p.sig_changes[0],
            SigChange {
                bar: 5,
                num: 4,
                den: 4
            }
        );
        // Bar 2 is in the first block.
        assert!(p.step_signature_at(1, SigPart::Denominator, 1));
        assert_eq!(p.sig, (4, 8));
        assert_eq!(p.sig_changes.len(), 1);
    }

    #[test]
    fn bars_in_a_range_are_listed_with_their_signature() {
        let p = mixed_signatures();
        let bars = p.bars_in(9.0, 15.0);
        let got: Vec<(u32, f64, u8)> = bars.iter().map(|b| (b.bar0, b.start, b.num)).collect();
        // Bar 3 holds beat 9 (it starts at 8); bar 5 starts at 14 and is the last one at 15.
        assert_eq!(got, [(2, 8.0, 3), (3, 11.0, 3), (4, 14.0, 6)]);
        assert_eq!(bars[2].unit, 0.5);
    }

    /// An audio track with regions at the given (start bar, length in bars), 1-based, each
    /// with a file-less audio marker so they count as audio regions.
    fn audio_regions(spans: &[(f32, f32)]) -> Project {
        let mut p = Project::empty();
        p.tracks.clear();
        let mut t = Track::new("Audio 1", TrackKind::Audio);
        for &(start_bar, len_bars) in spans {
            t.regions.push(Region {
                name: "r".into(),
                start_bar,
                len_bars,
                notes: Vec::new(),
                controls: Vec::new(),
                folder: None,
                auto: Vec::new(),
                generated: None,
                gain_line: Vec::new(),
                audio: Some(test_audio()),
                params: RegionParams::default(),
            });
        }
        p.tracks.push(t);
        p
    }

    fn test_audio() -> AudioRegion {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../ovrtone-engine/tests/fixtures");
        AudioRegion {
            file: ovrtone_engine::AudioFile::open(dir.join("tone.caf")).unwrap(),
            peaks: Default::default(),
            fine_peaks: Default::default(),
            peak_top: Default::default(),
            flex: None,
            follow_bpm: None,
        }
    }

    #[test]
    fn overlapping_regions_crossfade_over_the_overlap_when_asked() {
        // Region 0 plays bars 1-3, region 1 bars 3-5: one bar (4 quarter notes) overlaps.
        let mut p = audio_regions(&[(1.0, 3.0), (3.0, 3.0)]);
        assert_eq!(
            p.effective_fades(0, 0),
            (0.0, 0.0),
            "no crossfade unless asked"
        );
        assert_eq!(p.effective_fades(0, 1), (0.0, 0.0));
        p.tracks[0].regions[1].params.crossfade = true;
        assert_eq!(
            p.effective_fades(0, 0),
            (0.0, 4.0),
            "the earlier one fades out"
        );
        assert_eq!(
            p.effective_fades(0, 1),
            (4.0, 0.0),
            "the later one fades in"
        );
        // The engine gets the same.
        let edit = p.clip_edit_of(0, 1);
        assert_eq!((edit.fade_in_beats, edit.fade_out_beats), (4.0, 0.0));
        // A fade the region already had stays when it is longer.
        p.tracks[0].regions[1].params.fade_in = 10.0;
        p.tracks[0].regions[0].params.fade_out = 12.0;
        assert_eq!(p.effective_fades(0, 0).1, 12.0);
        assert_eq!(p.effective_fades(0, 1).0, 10.0);
    }

    #[test]
    fn crossfades_follow_the_signatures() {
        let mut p = audio_regions(&[(1.0, 3.0), (3.0, 3.0)]);
        p.set_signature_from(3, (3, 4));
        p.tracks[0].regions[1].params.crossfade = true;
        // The overlap is bar 3, which is 3 quarter notes now.
        assert_eq!(p.effective_fades(0, 1).0, 3.0);
    }

    #[test]
    fn regions_that_do_not_overlap_or_are_muted_do_not_crossfade() {
        let mut p = audio_regions(&[(1.0, 2.0), (3.0, 2.0)]);
        p.tracks[0].regions[1].params.crossfade = true;
        assert_eq!(p.effective_fades(0, 1), (0.0, 0.0), "they only touch");
        let mut p = audio_regions(&[(1.0, 3.0), (3.0, 3.0)]);
        p.tracks[0].regions[1].params.crossfade = true;
        p.tracks[0].regions[0].params.mute = true;
        assert_eq!(
            p.effective_fades(0, 1),
            (0.0, 0.0),
            "a muted region is not heard"
        );
        // A region wholly inside another one does not cut it short with a fade out.
        let mut p = audio_regions(&[(1.0, 6.0), (3.0, 1.0)]);
        p.tracks[0].regions[1].params.crossfade = true;
        assert_eq!(p.effective_fades(0, 0).1, 0.0);
    }

    /// Instrument tracks 0 and 1, a VCA at 2 and an instrument at 3; 0 and 1 follow the VCA.
    fn with_vca() -> Project {
        let mut p = Project::empty();
        p.tracks.clear();
        for n in 0..2 {
            p.tracks
                .push(Track::new(format!("Inst {n}"), TrackKind::Instrument));
        }
        p.tracks.push(Track::new("VCA 1", TrackKind::Vca));
        p.tracks.push(Track::new("Inst 3", TrackKind::Instrument));
        assert!(p.assign_vca(0, Some(2)));
        assert!(p.assign_vca(1, Some(2)));
        p
    }

    #[test]
    fn a_vca_moves_the_level_of_its_tracks() {
        let mut p = with_vca();
        p.tracks[0].volume_db = -3.0;
        p.tracks[2].volume_db = -6.0;
        assert_eq!(p.track_mix_of(0), (-9.0, false, false), "track + VCA");
        assert_eq!(p.track_mix_of(1), (-6.0, false, false));
        assert_eq!(p.track_mix_of(3), (0.0, false, false), "not assigned");
        assert_eq!(
            p.track_mix_of(2),
            (0.0, false, false),
            "the VCA itself makes no sound"
        );
        assert!(p.vca_of(0).is_some() && p.vca_of(3).is_none());
    }

    #[test]
    fn a_vcas_mute_and_solo_reach_its_tracks() {
        let mut p = with_vca();
        p.tracks[2].mute = true;
        assert!(p.is_muted(0) && p.is_muted(1) && !p.is_muted(3));
        assert!(p.track_mix_of(0).1);
        assert!(
            !p.track_mix_of(2).1,
            "the VCA track is not muted in the engine"
        );
        p.tracks[2].mute = false;
        p.tracks[2].solo = true;
        assert!(p.is_soloed(0) && p.is_soloed(1) && !p.is_soloed(3));
        assert!(p.any_solo(), "the others fall silent");
        assert!(
            !p.track_mix_of(2).2,
            "the VCA itself is not soloed in the engine"
        );
        // A VCA with nobody assigned solos nothing.
        let mut q = Project::empty();
        q.tracks.clear();
        q.tracks.push(Track::new("VCA 1", TrackKind::Vca));
        q.tracks.push(Track::new("Inst", TrackKind::Instrument));
        q.tracks[0].solo = true;
        assert!(!q.any_solo());
    }

    #[test]
    fn only_a_non_vca_track_can_be_assigned_to_a_vca_track() {
        let mut p = with_vca();
        assert!(!p.assign_vca(2, Some(2)), "a VCA cannot follow a VCA");
        assert!(!p.assign_vca(3, Some(1)), "the target must be a VCA");
        assert!(!p.assign_vca(3, Some(9)), "and it must exist");
        assert!(!p.assign_vca(0, Some(2)), "already assigned");
        assert!(p.assign_vca(0, None));
        assert_eq!(p.tracks[0].vca, None);
    }

    #[test]
    fn assignments_follow_the_tracks_when_tracks_come_and_go() {
        let mut p = with_vca();
        // A track is deleted before the VCA: the VCA moves up one place, and so do the links.
        assert!(p.delete_track(0));
        assert_eq!(p.tracks[1].kind, TrackKind::Vca);
        assert_eq!(p.tracks[0].vca, Some(1));
        // A folder is created in front: everything shifts down.
        p.create_track_stack(0);
        assert_eq!(
            p.tracks[1].vca,
            Some(2),
            "{:?}",
            p.tracks.iter().map(|t| t.vca).collect::<Vec<_>>()
        );
        // Deleting the VCA frees its tracks.
        let vca = p
            .tracks
            .iter()
            .position(|t| t.kind == TrackKind::Vca)
            .unwrap();
        assert!(p.delete_track(vca));
        assert!(p.tracks.iter().all(|t| t.vca.is_none()));
    }

    #[test]
    fn a_nudge_moves_regions_by_its_beats_and_stops_at_the_start() {
        let mut p = Project::demo();
        let at = |p: &Project| p.region_start_beats(&p.tracks[0].regions[0]);
        let before = at(&p);
        p.nudge_regions(&[(0, 0)], 0.25);
        assert!((at(&p) - before - 0.25).abs() < 1e-3);
        p.nudge_regions(&[(0, 0)], -1000.0);
        assert!(at(&p).abs() < 1e-3);
        p.nudge_regions(&[(99, 0)], 1.0);
    }

    #[test]
    fn a_bus_says_what_feeds_it() {
        let mut p = Project::empty();
        p.tracks.clear();
        let aux = p.add_track(TrackKind::Aux);
        let bus = p.aux_bus_of(aux);
        let a = p.add_track(TrackKind::Audio);
        let b = p.add_track(TrackKind::Audio);
        p.add_track(TrackKind::Audio);
        p.tracks[a].output = bus;
        p.tracks[b].sends.push(SendSlot {
            bus: Some(bus),
            level: 0.5,
            pre: true,
        });
        let feeds = p.feeds_of(aux);
        assert_eq!(feeds.len(), 2);
        assert_eq!(feeds[0].0, a);
        assert_eq!(feeds[1].0, b);
        assert!(feeds[1].1.contains("pre") && feeds[1].1.contains("50"));
        assert!(p.feeds_of(a).is_empty(), "only a bus is fed");
    }

    #[test]
    fn a_region_loop_goes_end_to_end_in_one_group_and_ungroups() {
        let mut p = Project::demo();
        let (start, len) = {
            let r = &p.tracks[0].regions[0];
            (p.region_start_beats(r), p.region_len_beats(r))
        };
        let before = p.tracks[0].regions.len();
        assert_eq!(p.loop_region(0, 0, 3), 3);
        assert_eq!(p.tracks[0].regions.len(), before + 3);
        let group = p.tracks[0].regions[0].params.group;
        assert_ne!(group, 0);
        assert_eq!(p.regions_in_group(group).len(), 4);
        let last = p.tracks[0].regions.last().unwrap();
        assert!((p.region_start_beats(last) - start - len * 3.0).abs() < 1e-2);
        p.ungroup_regions(&[(0, 0)]);
        assert!(p.regions_in_group(group).is_empty());
        assert!(!p.group_regions(&[(0, 0)]), "one is not a group");
        assert_eq!(p.loop_region(99, 0, 3), 0);
    }

    #[test]
    fn a_mapped_beat_falls_at_its_time_and_the_next_mapping_keeps_it() {
        let mut p = Project::empty();
        p.tempo = 120.0;
        // Beat 4 is at 2 seconds. Make it fall at 2.5.
        assert!(p.map_beat(4.0, 2.5));
        let map = p.tempo_map();
        assert!((map.seconds_at(4.0) - 2.5).abs() < 1e-3);
        assert!((f64::from(p.tempo) - 96.0).abs() < 1e-2);
        // After it, the old tempo goes on.
        assert!((p.tempo_at(4.5) - 120.0).abs() < 1e-3);
        assert!(p.map_beat(8.0, 4.5));
        let map = p.tempo_map();
        assert!((map.seconds_at(4.0) - 2.5).abs() < 1e-3, "the first stays");
        assert!((map.seconds_at(8.0) - 4.5).abs() < 1e-3);
        assert!(!p.map_beat(8.5, 4.4), "time cannot go back");
        assert!(!p.map_beat(12.0, 4.5001), "no such tempo");
    }

    #[test]
    fn an_output_strip_is_an_aux_on_its_pair_of_outputs() {
        let mut p = Project::empty();
        assert_eq!(p.hardware_pairs(), 0, "a stereo device has none");
        p.output_channels
            .store(6, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(p.hardware_pairs(), 2);
        let t = p.new_output_strip(2);
        assert_eq!(p.tracks[t].name, "Output 5-6");
        assert_eq!(p.tracks[t].kind, TrackKind::Aux);
        assert_eq!(p.tracks[t].hardware_out, 2);
    }

    #[test]
    fn an_aux_takes_the_bus_chosen_and_no_other_changes_by_it() {
        let mut p = Project::empty();
        p.tracks.clear();
        let (a, b) = (p.add_track(TrackKind::Aux), p.add_track(TrackKind::Aux));
        assert_eq!((p.aux_bus_of(a), p.aux_bus_of(b)), (1, 2));
        p.set_aux_bus(a, 16);
        assert_eq!((p.aux_bus_of(a), p.aux_bus_of(b)), (16, 2));
        let c = p.add_track(TrackKind::Aux);
        assert_eq!(p.aux_bus_of(c), 1, "the next free one");
        p.set_aux_bus(b, 16);
        assert_eq!(
            (p.aux_bus_of(a), p.aux_bus_of(b)),
            (2, 16),
            "they swap rather than share"
        );
        let audio = p.add_track(TrackKind::Audio);
        p.set_aux_bus(audio, 3);
        assert_eq!(p.aux_bus_of(audio), 0);
    }

    #[test]
    fn new_tracks_come_in_the_number_asked_for_and_a_summing_stack_has_one_name() {
        let mut p = Project::empty();
        p.tracks.clear();
        assert_eq!(p.add_tracks(TrackKind::Aux, 20), 20);
        assert_eq!(p.tracks.len(), 20);
        assert_eq!(p.aux_bus_of(19), 20);
        assert!(p.mixer_only(19), "an Aux waits in the mixer");
        assert_eq!(
            p.add_tracks(TrackKind::Audio, usize::MAX),
            ovrtone_engine::MAX_TRACKS - 20
        );
        let mut p = Project::empty();
        p.tracks.clear();
        let a = p.add_track(TrackKind::Audio);
        let stack = p.summing_stack(&[a]).unwrap();
        assert_eq!(p.tracks[stack].name, "Summing 1");
        assert_eq!(p.tracks[stack + 1].name, "Summing 1");
        assert!(p.mixer_only(stack + 1));
        assert_eq!(p.aux_buses(), [(1, "Summing 1".to_string())]);
        p.rename_track(stack, "Drums");
        assert_eq!(p.aux_buses(), [(1, "Drums".to_string())]);
    }

    #[test]
    fn a_duplicate_sits_below_its_track_and_an_aux_keeps_its_own_bus() {
        let mut p = Project::empty();
        p.tracks.clear();
        let a = p.add_track(TrackKind::Aux);
        let b = p.add_track(TrackKind::Aux);
        let bus_b = p.aux_bus_of(b);
        p.tracks.push(Track::new("Vox", TrackKind::Audio));
        p.tracks[2].output = bus_b;
        assert_eq!(p.duplicate_track(a, false), Some(a + 1));
        assert_eq!(p.tracks[a + 1].name, "Aux 1 copy");
        assert_eq!(p.tracks[a + 1].kind, TrackKind::Aux);
        assert_ne!(p.aux_bus_of(a), p.aux_bus_of(a + 1));
        // The Vox still plays into the same Aux, which has moved down one.
        assert_eq!(p.tracks[3].output, p.aux_bus_of(2));
        assert_eq!(p.tracks[2].name, "Aux 2");
    }

    #[test]
    fn picked_tracks_of_any_kind_are_duplicated_and_deleted_together() {
        let mut p = Project::empty();
        p.tracks.clear();
        for (name, kind) in [
            ("Vox", TrackKind::Audio),
            ("Box", TrackKind::Folder),
            ("In", TrackKind::Instrument),
            ("Bus", TrackKind::Bus),
            ("Out", TrackKind::Output),
        ] {
            p.tracks.push(Track::new(name, kind));
        }
        p.tracks[2].indent = 1;
        let made = p.duplicate_tracks(&[0, 1, 3, 4], true);
        let names: Vec<&str> = p.tracks.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Vox", "Vox copy", "Box", "In", "Box copy", "In copy", "Bus", "Bus copy", "Out",
                "Out copy"
            ]
        );
        assert_eq!(made, [1, 4, 5, 7, 9]);
        assert_eq!(p.selected_track, 1);
        let levels: Vec<u8> = p.tracks.iter().map(|t| t.indent).collect();
        assert_eq!(levels, [0, 0, 0, 1, 0, 1, 0, 0, 0, 0]);
        assert!(p.delete_tracks(&made));
        let names: Vec<&str> = p.tracks.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["Vox", "Box", "In", "Bus", "Out"]);
    }

    #[test]
    fn new_tracks_have_the_defaults_of_their_kind() {
        let inst = Track::new("Inst 1", TrackKind::Instrument);
        assert!(inst.instrument_plugin.is_none() && inst.regions.is_empty());
        for kind in [TrackKind::Aux, TrackKind::Folder] {
            let t = Track::new("x", kind);
            assert!(!kind.has_regions());
            assert_eq!(
                (t.indent, t.folded, t.volume_db, t.height),
                (0, false, 0.0, 1.0)
            );
        }
        assert!(TrackKind::Audio.has_regions() && TrackKind::Drummer.has_regions());
    }

    #[test]
    fn deleting_tracks_keeps_the_selection_valid_and_never_empties_the_project() {
        let mut p = Project::demo();
        let n = p.tracks.len();
        p.selected_track = n - 1;
        let rev = p.song_rev;
        assert!(p.delete_track(1));
        assert_eq!(p.tracks.len(), n - 1);
        assert_eq!(p.selected_track, n - 2, "the selection follows its track");
        assert_ne!(p.song_rev, rev);
        assert!(!p.delete_track(99));

        // A folder goes, the tracks inside it stay.
        let stacked = p.tracks[0].name.clone();
        p.create_track_stack(0);
        assert!(p.delete_track(0));
        assert_eq!(
            (p.tracks[0].name.as_str(), p.tracks[0].indent),
            (stacked.as_str(), 0)
        );

        while !p.tracks.is_empty() {
            assert!(p.delete_track(0));
        }
        assert!(!p.delete_track(0), "nothing left to delete");
        assert_eq!(p.selected_track, 0);
    }

    #[test]
    fn nesting_is_repaired_to_what_the_folders_allow() {
        let mut p = Project::demo();
        let (inst, folder) = (TrackKind::Instrument, TrackKind::Folder);
        let at = |k, indent| {
            let mut t = Track::new("t", k);
            t.indent = indent;
            t
        };
        p.tracks = vec![
            at(inst, 5), // first track: nothing above it
            at(folder, 0),
            at(inst, 9), // right under a folder: at most one level
            at(folder, 1),
            at(inst, 2), // nested folder: two levels is fine
            at(inst, 7), // after a plain track: no deeper than it
        ];
        p.normalize_nesting();
        let got: Vec<u8> = p.tracks.iter().map(|t| t.indent).collect();
        assert_eq!(got, [0, 0, 1, 1, 2, 2]);
    }
    #[test]
    fn added_tracks_are_named_per_kind_selected_and_announced() {
        let mut p = Project::demo();
        let (n, rev) = (p.tracks.len(), p.song_rev);
        let aux = p.add_track(TrackKind::Aux);
        assert_eq!((aux, p.selected_track, p.tracks.len()), (n, n, n + 1));
        assert_eq!(p.tracks[aux].name, "Aux 1");
        assert_eq!(p.tracks[aux].kind, TrackKind::Aux);
        assert!(p.tracks[aux].regions.is_empty());
        assert_ne!(p.song_rev, rev, "the engine is told the track list changed");

        p.add_track(TrackKind::Aux);
        assert_eq!(p.tracks.last().unwrap().name, "Aux 2");
        p.add_track(TrackKind::Instrument);
        assert_eq!(
            p.tracks.last().unwrap().name,
            "Inst 4",
            "the demo has 3 instruments"
        );
    }

    #[test]
    fn channel_names_count_within_their_class() {
        let mut p = Project::demo();
        let a1 = p.add_track(TrackKind::Aux);
        let a2 = p.add_track(TrackKind::Aux);
        let audio = p
            .tracks
            .iter()
            .position(|t| t.kind == TrackKind::Audio)
            .unwrap();
        assert_eq!(p.channel_name(a1), "Aux 1");
        assert_eq!(p.channel_name(a2), "Aux 2");
        assert_eq!(p.channel_name(audio), "Audio 1");
        assert_eq!(p.channel_name(0), "Inst 1");
    }

    /// Tracks "a b c d e": b and c in a folder (a), d outside, e in a nested folder inside d.
    fn stack_fixture() -> Project {
        let mut p = Project::demo();
        let k = |name: &str, kind, indent| {
            let mut t = Track::new(name, kind);
            t.indent = indent;
            t
        };
        let (inst, folder) = (TrackKind::Instrument, TrackKind::Folder);
        p.tracks = vec![
            k("F", folder, 0),
            k("b", inst, 1),
            k("c", inst, 1),
            k("d", inst, 0),
            k("G", folder, 0),
            k("e", inst, 1),
        ];
        p
    }

    #[test]
    fn folders_own_the_deeper_tracks_after_them() {
        let p = stack_fixture();
        assert_eq!(p.folder_end(0), 3);
        assert_eq!(p.folder_end(4), 6);
        assert_eq!(p.folder_end(3), 4, "a plain track is its own range");
    }

    #[test]
    fn folding_hides_exactly_the_tracks_inside() {
        let mut p = stack_fixture();
        let hidden = |p: &Project| {
            (0..p.tracks.len())
                .filter(|i| p.hidden_by_fold(*i))
                .collect::<Vec<_>>()
        };
        assert!(hidden(&p).is_empty());
        p.tracks[0].folded = true;
        assert_eq!(hidden(&p), [1, 2], "the folder itself and the others stay");
        p.tracks[4].folded = true;
        assert_eq!(hidden(&p), [1, 2, 5]);
        // A folded outer folder hides an open inner one's tracks as well.
        let mut p = stack_fixture();
        p.tracks[0].folded = false;
        p.create_track_stack(0);
        p.tracks[0].folded = true;
        assert_eq!((1..4).filter(|i| p.hidden_by_fold(*i)).count(), 3);
    }

    #[test]
    fn moved_tracks_keep_their_buses_vcas_and_contents() {
        let mut p = Project::empty();
        p.tracks.clear();
        let a = p.add_track(TrackKind::Audio);
        let aux1 = p.add_track(TrackKind::Aux);
        let aux2 = p.add_track(TrackKind::Aux);
        let vca = p.add_track(TrackKind::Vca);
        p.tracks[a].name = "a".into();
        p.tracks[aux1].name = "one".into();
        p.tracks[aux2].name = "two".into();
        p.tracks[a].output = p.aux_bus_of(aux2);
        p.tracks[a].vca = Some(vca);
        p.selected_track = a;
        assert!(p.move_tracks(&[aux2], aux1), "the second Aux goes first");
        let two = p.tracks.iter().position(|t| t.name == "two").unwrap();
        let track = p.tracks.iter().position(|t| t.name == "a").unwrap();
        assert_eq!(
            p.tracks[track].output,
            p.aux_bus_of(two),
            "still hears the same Aux"
        );
        assert_eq!(p.tracks[p.tracks[track].vca.unwrap()].kind, TrackKind::Vca);
        assert_eq!(p.selected_track, track);
        assert!(!p.move_tracks(&[track], track), "already there");
        assert!(p.move_tracks(&[track], p.tracks.len()), "to the end");
        assert_eq!(p.tracks.last().unwrap().name, "a");
    }

    #[test]
    fn a_side_chain_follows_its_source_and_goes_with_it() {
        let info = ovrtone_host::PluginInfo {
            id: "c".into(),
            name: "Comp".into(),
            vendor: String::new(),
            version: String::new(),
            format: ovrtone_host::PluginFormat::Clap,
            path: "c.clap".into(),
            is_instrument: false,
        };
        let mut p = Project::empty();
        p.tracks.clear();
        let kick = p.add_track(TrackKind::Audio);
        let bass = p.add_track(TrackKind::Audio);
        let other = p.add_track(TrackKind::Audio);
        p.tracks[kick].name = "kick".into();
        p.tracks[other].plugins.push(PluginInsert::new(&info));
        p.tracks[other].plugins[0].sidechain = Some(kick);
        p.master_plugins.push(PluginInsert::new(&info));
        p.master_plugins[0].sidechain = Some(bass);
        let key_of = |p: &Project| {
            let t = p.tracks.iter().find(|t| !t.plugins.is_empty()).unwrap();
            t.plugins[0].sidechain.map(|s| p.tracks[s].name.clone())
        };

        assert!(p.move_tracks(&[kick], p.tracks.len()), "kick to the end");
        assert_eq!(key_of(&p).as_deref(), Some("kick"));
        assert_eq!(p.master_plugins[0].sidechain, Some(0), "bass is first now");

        // A track put in before it moves it along.
        p.tracks.insert(0, Track::new("new", TrackKind::Audio));
        p.vca_inserted(0);
        assert_eq!(key_of(&p).as_deref(), Some("kick"));
        assert_eq!(p.master_plugins[0].sidechain, Some(1));

        // One taken away before it too, and when the source itself goes the plugin has none.
        assert!(p.delete_track(0));
        assert_eq!(key_of(&p).as_deref(), Some("kick"));
        let kick = p.tracks.iter().position(|t| t.name == "kick").unwrap();
        assert!(p.delete_track(kick));
        assert_eq!(key_of(&p), None);
    }

    #[test]
    fn tracks_picked_anywhere_make_one_stack_and_a_summing_stack_goes_through_one_aux() {
        let mut p = Project::empty();
        p.tracks.clear();
        for name in ["a", "b", "c", "d"] {
            let t = p.add_track(TrackKind::Audio);
            p.tracks[t].name = name.into();
        }
        let folder = p.stack_tracks(&[0, 2]).unwrap();
        assert_eq!(p.tracks[folder].kind, TrackKind::Folder);
        let inside: Vec<&str> = p.tracks[folder + 1..p.folder_end(folder)]
            .iter()
            .map(|t| t.name.as_str())
            .collect();
        assert_eq!(inside, ["a", "c"]);
        let d = p.tracks.iter().position(|t| t.name == "d").unwrap();
        let b = p.tracks.iter().position(|t| t.name == "b").unwrap();
        let sum = p.summing_stack(&[b, d]).unwrap();
        let aux = sum + 1;
        assert_eq!(p.tracks[aux].kind, TrackKind::Aux);
        let bus = p.aux_bus_of(aux);
        assert!(bus > 0);
        for name in ["b", "d"] {
            let t = p.tracks.iter().find(|t| t.name == name).unwrap();
            assert_eq!(t.output, bus, "{name}");
        }
        assert_eq!(p.tracks.iter().find(|t| t.name == "a").unwrap().output, 0);
        assert_eq!(p.tracks[aux].output, 0, "the Aux does not feed itself");
    }

    #[test]
    fn a_track_stack_wraps_the_track_and_can_be_released_again() {
        let mut p = stack_fixture();
        let (rev, names) = (p.song_rev, |p: &Project| {
            p.tracks
                .iter()
                .map(|t| (t.name.clone(), t.indent))
                .collect::<Vec<_>>()
        });
        let before = names(&p);

        // Wrap the plain track "d" (index 3).
        let at = p.create_track_stack(3);
        assert_eq!(
            (at, p.selected_track, p.tracks[3].kind),
            (3, 3, TrackKind::Folder)
        );
        assert_eq!(p.tracks[3].name, "Folder 3");
        assert_eq!((p.tracks[4].name.as_str(), p.tracks[4].indent), ("d", 1));
        assert_ne!(p.song_rev, rev);

        assert!(p.release_track_stack(3));
        assert_eq!(names(&p), before, "release undoes create");
        assert!(!p.release_track_stack(3), "d is not a folder");

        // Wrapping a folder takes everything inside it one level deeper.
        p.create_track_stack(0);
        let nested: Vec<u8> = p.tracks[..4].iter().map(|t| t.indent).collect();
        assert_eq!(nested, [0, 1, 2, 2]);
        assert_eq!(p.folder_end(0), 4);
    }

    #[test]
    fn a_folders_mute_and_solo_reach_every_track_inside_it() {
        let mut p = stack_fixture();
        p.set_folder_flags(0, Some(true), None);
        let muted: Vec<bool> = p.tracks.iter().map(|t| t.mute).collect();
        assert_eq!(muted, [true, true, true, false, false, false]);
        p.set_folder_flags(0, None, Some(true));
        assert!(p.tracks[..3].iter().all(|t| t.solo && t.mute));
        assert!(!p.tracks[3].solo, "tracks outside the folder are untouched");
        p.set_folder_flags(0, Some(false), Some(false));
        assert!(p.tracks.iter().all(|t| !t.mute && !t.solo));
    }

    #[test]
    fn an_invalid_denominator_steps_from_quarter_notes() {
        let mut p = Project::demo();
        p.sig = (4, 3);
        assert!(p.step_signature(SigPart::Denominator, 1));
        assert_eq!(p.sig.1, 8);
    }

    #[test]
    fn tempo_changes_hold_until_the_next_one() {
        let mut p = Project::empty();
        p.tempo = 120.0;
        p.add_tempo_change(16.0, 90.0);
        p.add_tempo_change(8.0, 100.0);
        assert_eq!(p.tempo_at(0.0), 120.0);
        assert_eq!(p.tempo_at(8.0), 100.0);
        assert_eq!(p.tempo_at(15.9), 100.0);
        assert_eq!(p.tempo_at(400.0), 90.0);
        // Adding at the same beat replaces; at the start sets the start tempo.
        p.add_tempo_change(8.0, 110.0);
        assert_eq!(p.tempo_changes.len(), 2);
        p.add_tempo_change(0.0, 60.0);
        assert_eq!((p.tempo, p.tempo_changes.len()), (60.0, 2));
        // Dragging the LCD tempo edits the change in effect at the playhead.
        p.set_tempo_at(20.0, 95.0);
        assert_eq!(p.tempo_changes[1].bpm, 95.0);
        p.set_tempo_at(3.0, 70.0);
        assert_eq!(p.tempo, 70.0);
    }

    #[test]
    fn the_engine_tempo_map_follows_the_tempo_track() {
        let mut p = Project::empty();
        p.tempo = 120.0;
        p.add_tempo_change(8.0, 60.0);
        let map = p.tempo_map();
        // 8 beats at 120 = 4 s, then 4 beats at 60 = 4 s.
        assert!((map.seconds_at(8.0) - 4.0).abs() < 1e-9);
        assert!((map.seconds_at(12.0) - 8.0).abs() < 1e-9);
        assert_eq!(map.tempo_at(10.0), 60.0);
    }

    #[test]
    fn a_drummer_region_makes_notes_and_follows_its_length_and_the_bars_under_it() {
        let mut p = Project::empty();
        let ti = p.add_track(TrackKind::Drummer);
        let ri = p.add_generated_region(ti, true, 3.0).unwrap();
        let r = &p.tracks[ti].regions[ri];
        assert!(r.generated.is_some() && !r.notes.is_empty());
        assert!(r.notes.iter().all(|n| n.start < 16.0), "four bars of 4/4");
        assert_eq!((r.start_bar, r.len_bars), (3.0, 4.0));
        let rev = p.song_rev;
        assert!(!p.refresh_generated(), "nothing has changed");
        assert_eq!(p.song_rev, rev);

        // Lengthened to eight bars: more notes, and the engine is told.
        let four = p.tracks[ti].regions[ri].notes.len();
        p.tracks[ti].regions[ri].len_bars = 8.0;
        assert!(p.refresh_generated());
        assert!(p.song_rev > rev);
        let notes = &p.tracks[ti].regions[ri].notes;
        assert!(notes.len() > four && notes.iter().all(|n| n.start < 32.0));
        // Moved: the bars under it are the same, so the notes are left alone.
        p.tracks[ti].regions[ri].start_bar = 10.0;
        assert!(!p.refresh_generated());
        // A signature change under it makes the bars shorter: 3/4 from bar 12.
        p.set_signature_from(12, (3, 4));
        let lens: Vec<f64> = p
            .region_bars(&p.tracks[ti].regions[ri])
            .iter()
            .map(|b| b.beats)
            .collect();
        assert_eq!(lens, [4.0, 4.0, 3.0, 3.0, 3.0, 3.0, 3.0, 3.0]);
        assert!(p.refresh_generated());
        let notes = &p.tracks[ti].regions[ri].notes;
        assert!(notes.iter().all(|n| n.start < 26.0), "26 beats long now");
        // A region cut short in the middle of a bar ends with a short bar.
        p.tracks[ti].regions[ri].len_bars = 2.5;
        let region = &p.tracks[ti].regions[ri];
        let total: f64 = p.region_bars(region).iter().map(|b| b.beats).sum();
        assert!((total - p.region_len_beats(region)).abs() < 1e-9);
        // Made an ordinary region, it keeps its notes and is left alone from then on.
        p.refresh_generated();
        let kept = p.tracks[ti].regions[ri].notes.clone();
        p.convert_to_midi_region(ti, ri);
        assert!(p.tracks[ti].regions[ri].generated.is_none());
        p.tracks[ti].regions[ri].len_bars = 6.0;
        assert!(!p.refresh_generated());
        assert_eq!(p.tracks[ti].regions[ri].notes, kept);
    }

    #[test]
    fn a_pattern_region_starts_with_the_rows_of_its_instrument() {
        let mut p = Project::empty();
        let drums = p.add_track(TrackKind::Drummer);
        let r = p.add_generated_region(drums, false, 1.0).unwrap();
        let notes = &p.tracks[drums].regions[r].notes;
        assert!(notes.iter().any(|n| n.pitch == 36) && notes.iter().any(|n| n.pitch == 38));
        let synth = p.add_track(TrackKind::Instrument);
        let r = p.add_generated_region(synth, false, 1.0).unwrap();
        assert!(
            p.tracks[synth].regions[r].notes.is_empty(),
            "an empty grid to draw in"
        );
        // Audio, Aux and VCA tracks hold no such regions.
        for kind in [
            TrackKind::Audio,
            TrackKind::Aux,
            TrackKind::Vca,
            TrackKind::Folder,
        ] {
            let t = p.add_track(kind);
            assert!(p.add_generated_region(t, true, 1.0).is_none(), "{kind:?}");
        }
    }

    #[test]
    fn a_tempo_curve_moves_in_a_straight_line_to_the_next_change() {
        let mut p = Project::empty();
        p.tempo = 100.0;
        p.add_tempo_change(16.0, 140.0);
        p.add_tempo_change(32.0, 80.0);
        p.set_tempo_ramp(0, true);
        assert_eq!(p.tempo_at(0.0), 100.0);
        assert_eq!(p.tempo_at(8.0), 120.0);
        assert_eq!(p.tempo_at(15.99).round(), 140.0);
        assert_eq!(p.tempo_at(16.0), 140.0, "the second segment holds");
        assert_eq!(p.tempo_at(40.0), 80.0);
        // The engine integrates the same curve: 16 beats from 100 to 140 BPM.
        let map = p.tempo_map();
        let secs = 60.0 * 16.0 / 40.0 * (140.0f64 / 100.0).ln();
        assert!((map.seconds_at(16.0) - secs).abs() < 1e-9);
        assert!((map.tempo_at(8.0) - 120.0).abs() < 1e-4);
        // The last change has nothing to ramp to.
        let last = p.tempo_changes.len();
        p.set_tempo_ramp(last, true);
        assert!(!p.tempo_changes[last - 1].ramp);
        // Dragging the tempo on a curve moves the curve's start by as much as the display.
        p.set_tempo_at(8.0, 130.0);
        assert_eq!(p.tempo, 110.0);
        assert_eq!(p.tempo_at(8.0), 125.0);
    }

    #[test]
    fn the_smpte_display_follows_the_tempo_map_rate_and_start() {
        use ovrtone_engine::SmpteRate;
        let mut p = Project::empty();
        p.tempo = 120.0;
        p.position_beats = 0.0;
        assert_eq!(p.timecode(), "01:00:00:00.00", "the default start");
        p.position_beats = 2.0;
        assert_eq!(p.timecode(), "01:00:01:00.00");
        // A slower tempo after beat 4 stretches the seconds: 4 beats at 120 = 2 s, then 2
        // beats at 60 = 2 s.
        p.add_tempo_change(4.0, 60.0);
        p.position_beats = 6.0;
        assert_eq!(p.timecode(), "01:00:04:00.00");
        // A half frame (40 ms at 25 fps is a frame; 20 ms is half of one = subframe 40).
        p.position_beats = 0.04;
        assert_eq!(p.timecode(), "01:00:00:00.40");
        // Another start and rate.
        p.smpte_start = [0, 0, 0, 0];
        p.smpte_rate = SmpteRate::Fps30;
        p.position_beats = 2.0;
        assert_eq!(p.timecode(), "00:00:01:00.00");
        p.smpte_rate = SmpteRate::Fps29_97Drop;
        p.smpte_start = [0, 1, 0, 2];
        assert_eq!(p.smpte_offset_frames(), 1800);
        p.position_beats = 0.0;
        assert_eq!(p.timecode(), "00:01:00;02.00", "drop frame has a semicolon");
        // A tempo curve: 100 -> 200 BPM over 16 beats takes 60 * 16 / 100 * ln 2 seconds.
        let mut c = Project::empty();
        c.smpte_start = [0, 0, 0, 0];
        c.smpte_rate = SmpteRate::Fps24;
        c.tempo = 100.0;
        c.add_tempo_change(16.0, 200.0);
        c.set_tempo_ramp(0, true);
        c.position_beats = 16.0;
        let secs = 60.0 * 16.0 / 100.0 * 2f64.ln();
        let frames = (secs * 24.0).floor() as u32;
        let expect = format!("00:00:{:02}:{:02}", frames / 24, frames % 24);
        assert!(
            c.timecode().starts_with(&expect),
            "{} vs {expect}",
            c.timecode()
        );
    }

    #[test]
    fn a_tempo_change_inside_a_curve_keeps_the_curve_going() {
        let mut p = Project::empty();
        p.tempo = 60.0;
        p.add_tempo_change(32.0, 120.0);
        p.set_tempo_ramp(0, true);
        let i = p.add_tempo_change(16.0, 90.0);
        assert!(p.tempo_ramp && p.tempo_changes[i].ramp);
        assert_eq!(p.tempo_at(8.0), 75.0);
        assert_eq!(p.tempo_at(24.0), 105.0);
        // Deleting the end of the curve leaves a point with nothing to ramp to.
        p.tempo_changes.pop();
        p.tidy_tempo();
        assert!(!p.tempo_changes[0].ramp);
        assert_eq!(p.tempo_at(100.0), 90.0);
        p.tempo_changes.clear();
        p.tidy_tempo();
        assert!(!p.tempo_ramp);
    }

    #[test]
    fn markers_are_named_kept_in_order_and_navigable() {
        let mut p = Project::empty();
        assert_eq!(p.add_marker(16.0), 0);
        assert_eq!(p.add_marker(4.0), 0, "sorted by position");
        assert_eq!(p.add_marker(16.0), 1, "one marker per position");
        let names: Vec<&str> = p.markers.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["Marker 2", "Marker 1"]);
        assert_ne!(p.markers[0].color, p.markers[1].color);
        assert_eq!(p.marker_at(2.0), None);
        assert_eq!(p.marker_at(4.0), Some(0));
        assert_eq!(p.marker_at(30.0), Some(1));
        assert_eq!(p.next_marker(4.0), Some(16.0));
        assert_eq!(p.next_marker(16.0), None);
        assert_eq!(p.previous_marker(16.0), Some(4.0));
        assert_eq!(p.previous_marker(4.0), None);
        // A deleted name is reused.
        p.markers.remove(1);
        p.add_marker(32.0);
        assert_eq!(p.markers[1].name, "Marker 1");
    }

    #[test]
    fn tidy_markers_sorts_and_drops_bad_or_doubled_ones() {
        let mut p = Project::empty();
        for b in [8.0, -1.0, f64::NAN, 2.0, 8.0] {
            p.markers.push(Marker {
                beat: b,
                name: format!("{b}"),
                color: MARKER_COLORS[0],
            });
        }
        p.tidy_markers();
        let beats: Vec<f64> = p.markers.iter().map(|m| m.beat).collect();
        assert_eq!(beats, [2.0, 8.0]);
    }

    #[test]
    fn normalize_brings_the_peak_to_full_scale() {
        let g = RegionParams::normalize_gain(0.5).unwrap();
        assert!((g - 6.0206).abs() < 1e-3, "half scale needs +6 dB, got {g}");
        assert_eq!(RegionParams::normalize_gain(1.0), Some(0.0));
        assert_eq!(RegionParams::normalize_gain(0.0), None, "silence");
        assert_eq!(RegionParams::normalize_gain(f32::NAN), None);
        assert_eq!(
            RegionParams::normalize_gain(1e-5),
            Some(MAX_REGION_GAIN),
            "never past +24 dB"
        );
        let edit = RegionParams {
            gain_db: -2.0,
            fade_in: 1.5,
            reverse: true,
            ..RegionParams::default()
        }
        .clip_edit();
        assert_eq!(
            (edit.gain_db, edit.fade_in_beats, edit.reverse),
            (-2.0, 1.5, true)
        );
    }

    #[test]
    fn a_frame_in_which_nothing_changed_does_not_look_at_the_bars_again() {
        let mut p = Project::empty();
        let ti = p.add_track(TrackKind::Drummer);
        let ri = p.add_generated_region(ti, true, 1.0).unwrap();
        assert!(!p.refresh_generated());
        let g = p.tracks[ti].regions[ri].generated.as_ref().unwrap();
        let r = &p.tracks[ti].regions[ri];
        assert_eq!(
            g.seen,
            (r.start_bar.to_bits(), r.len_bars.to_bits(), p.song_rev)
        );
        // Break the stored key on purpose: while the fingerprint matches, it is not looked at...
        p.tracks[ti].regions[ri]
            .generated
            .as_mut()
            .unwrap()
            .made_for ^= 1;
        assert!(!p.refresh_generated(), "same bars, same song: no look");
        // ...and a change of the region or the song brings the comparison (and the new notes).
        p.tracks[ti].regions[ri].len_bars = 6.0;
        assert!(p.refresh_generated());
        p.tracks[ti].regions[ri]
            .generated
            .as_mut()
            .unwrap()
            .made_for ^= 1;
        p.song_rev += 1;
        assert!(p.refresh_generated());
    }
}

/// One strip as a mixer snapshot holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct StripSnapshot {
    pub name: String,
    pub volume_db: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub trim_db: f32,
    pub inserts: Vec<Insert>,
    pub sends: Vec<SendSlot>,
}

/// The mixer at one moment, to go back to: every strip's fader, pan, mute, solo and input trim,
/// its built-in effects with their settings, and its sends. Hosted plugins keep their own state
/// and are not part of it.
#[derive(Clone, Debug, PartialEq)]
pub struct MixerSnapshot {
    pub name: String,
    pub strips: Vec<StripSnapshot>,
}

impl Project {
    /// Keeps the mixer as it is now as a new snapshot.
    pub fn take_mixer_snapshot(&mut self) {
        let strips = self
            .tracks
            .iter()
            .map(|t| StripSnapshot {
                name: t.name.clone(),
                volume_db: t.volume_db,
                pan: t.pan,
                mute: t.mute,
                solo: t.solo,
                trim_db: t.trim_db,
                inserts: t.inserts.clone(),
                sends: t.sends.clone(),
            })
            .collect();
        let name = format!("Snapshot {}", self.mixer_snapshots.len() + 1);
        self.mixer_snapshots.push(MixerSnapshot { name, strips });
    }

    /// Puts the mixer back as snapshot `k` has it. A strip is found by its place, or by its name
    /// if the tracks were moved since; one that is gone is skipped.
    pub fn recall_mixer_snapshot(&mut self, k: usize) {
        let Some(snapshot) = self.mixer_snapshots.get(k).cloned() else {
            return;
        };
        for (i, strip) in snapshot.strips.iter().enumerate() {
            let at = if self.tracks.get(i).is_some_and(|t| t.name == strip.name) {
                Some(i)
            } else {
                self.tracks.iter().position(|t| t.name == strip.name)
            };
            let Some(t) = at.and_then(|at| self.tracks.get_mut(at)) else {
                continue;
            };
            t.volume_db = strip.volume_db;
            t.pan = strip.pan;
            t.mute = strip.mute;
            t.solo = strip.solo;
            t.trim_db = strip.trim_db;
            if t.inserts != strip.inserts {
                t.inserts = strip.inserts.clone();
            }
            if t.sends != strip.sends {
                t.sends = strip.sends.clone();
            }
        }
    }
}
