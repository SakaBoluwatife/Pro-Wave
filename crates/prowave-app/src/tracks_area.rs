//! Tracks area (Logic Pro: main window "Tracks" area): local toolbar, ruler with cycle area and
//! playhead, track headers and the region lanes.
//!
//! Works today: select/move regions with Snap, Delete and Repeat regions, Set Locators by
//! Regions, drag in the ruler to set the cycle, click to move the playhead, scroll, zoom
//! (sliders and Ctrl+wheel), Catch playhead, add/duplicate tracks, M/S/R/I, volume, pan.
//! Menu entries that are not implemented yet are shown disabled.
use crate::i18n::{tr, trf};
use crate::model::{Project, Region, Track, TrackKind};
use crate::track_icons::track_icon;
use crate::widgets::{
    context_click, context_menu, menu_title, popup_button, stroke, BLUE, BTN_BG, TEXT, TEXT_DIM,
};
use crate::{Bottom, UiState};
use egui::{
    pos2, vec2, Align2, Color32, FontId, Key, KeyboardShortcut, Modifiers, Painter, Pos2, Rect,
    Sense, Shape, Stroke, StrokeKind, Ui,
};

const TOOLBAR_H: f32 = 30.0;
const RULER_H: f32 = 26.0;
const HEADER_W: f32 = 340.0;
const NUM_W: f32 = 24.0;
/// How far the horizontal zoom goes, in points per bar: at the top, about two samples a point.
pub(crate) const MIN_BAR_W: f32 = 6.0;
pub(crate) const MAX_BAR_W: f32 = 40000.0;
/// How far the vertical zoom goes, in points per track.
const MIN_ROW_H: f32 = 14.0;
const MAX_ROW_H: f32 = 300.0;
/// How long the zoom's readout and the overview stay after the zoom last moved.
const ZOOM_NOTE_SECONDS: f32 = 1.5;
/// How far each folder nesting level moves a track header's contents to the right.
const INDENT_W: f32 = 14.0;

const TOOLBAR_BG: Color32 = crate::theme::TOOLBAR;
const HEADER_BG: Color32 = Color32::from_rgb(0x50, 0x50, 0x50);
const HEADER_SEL: Color32 = Color32::from_rgb(0x62, 0x62, 0x62);
const LANE_BG: Color32 = Color32::from_rgb(0x3a, 0x3a, 0x3a);
const LANE_SEL: Color32 = Color32::from_rgb(0x42, 0x42, 0x42);
const RULER_BG: Color32 = crate::theme::PANEL_BG;
const CYCLE: Color32 = Color32::from_rgb(0xd4, 0x9e, 0x2a);
const MUTE_ON: Color32 = Color32::from_rgb(0x3b, 0x8c, 0xe6);
const SOLO_ON: Color32 = Color32::from_rgb(0xe6, 0xc2, 0x2e);
const REC_ON: Color32 = Color32::from_rgb(0xe0, 0x3a, 0x30);
const MON_ON: Color32 = Color32::from_rgb(0xe8, 0x8a, 0x2c);
const AUTO_ON: Color32 = Color32::from_rgb(0x4e, 0xa8, 0x5a);
/// What the curve on a track leaves free at the top of the row: the regions' name bars, by which
/// they are still moved.
const AUTO_NAME_BAR: f32 = 14.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Snap {
    Smart,
    Bar,
    Beat,
    Division,
    Ticks,
}

impl Snap {
    pub(crate) const ALL: [Snap; 5] = [
        Snap::Smart,
        Snap::Bar,
        Snap::Beat,
        Snap::Division,
        Snap::Ticks,
    ];
    pub(crate) fn label(self) -> &'static str {
        match self {
            Snap::Smart => tr("Smart"),
            Snap::Bar => tr("Bar"),
            Snap::Beat => tr("Beat"),
            Snap::Division => tr("Division"),
            Snap::Ticks => tr("Ticks"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DragMode {
    Overlap,
    NoOverlap,
    XFade,
    ShuffleR,
    ShuffleL,
}

impl DragMode {
    const ALL: [DragMode; 5] = [
        DragMode::Overlap,
        DragMode::NoOverlap,
        DragMode::XFade,
        DragMode::ShuffleR,
        DragMode::ShuffleL,
    ];
    fn label(self) -> &'static str {
        match self {
            DragMode::Overlap => tr("Overlap"),
            DragMode::NoOverlap => tr("No Overlap"),
            DragMode::XFade => tr("X-Fade"),
            DragMode::ShuffleR => tr("Shuffle R"),
            DragMode::ShuffleL => tr("Shuffle L"),
        }
    }
}

/// Where the Fade tool is in [`TOOLS`].
const FADE_TOOL: usize = 9;
const SCISSORS_TOOL: usize = 4;
const GLUE_TOOL: usize = 5;

pub(crate) const TOOLS: [&str; 15] = [
    "Pointer Tool",
    "Pencil Tool",
    "Eraser Tool",
    "Text Tool",
    "Scissors Tool",
    "Glue Tool",
    "Solo Tool",
    "Mute Tool",
    "Zoom Tool",
    "Fade Tool",
    "Automation Select Tool",
    "Automation Curve Tool",
    "Marquee Tool",
    "Flex Tool",
    "Scrub Tool",
];

/// A Normalize Region Gain scan running on its own thread: reading a long recording to find
/// its loudest sample takes seconds, which would freeze the window if done on the UI thread.
pub struct NormalizeJob {
    track: usize,
    region: usize,
    path: std::path::PathBuf,
    /// The file's peak, or why it could not be read.
    result: std::sync::mpsc::Receiver<Result<f32, String>>,
}

/// The tool the pointer has in the Tracks area (Logic's left-click tool).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrackTool {
    Pointer,
    /// Click a region to cut it there.
    Scissors,
    /// Click a region to join it with the next one.
    Glue,
    /// Drag on an audio region to set its fade in (left half) or fade out (right half).
    Fade,
    /// Click an empty stretch of a MIDI track to make a region there.
    Pencil,
    /// Click a region to delete it.
    Eraser,
    /// Click a region to rename it.
    Text,
    /// Click a region to solo its track.
    Solo,
    /// Click a region to mute or unmute it.
    Mute,
    /// Click to zoom in.
    Zoom,
    /// Click a region to edit its own automation.
    Automation,
    /// Drag over regions to select them, without moving any.
    Marquee,
    /// Click an audio region to turn Flex on for it.
    Flex,
    /// Drag over audio to hear it where the pointer is.
    Scrub,
}

impl TrackTool {
    /// The tool of an entry of [`TOOLS`].
    fn of_entry(n: usize) -> Self {
        match n {
            1 => Self::Pencil,
            2 => Self::Eraser,
            3 => Self::Text,
            SCISSORS_TOOL => Self::Scissors,
            GLUE_TOOL => Self::Glue,
            6 => Self::Solo,
            7 => Self::Mute,
            8 => Self::Zoom,
            FADE_TOOL => Self::Fade,
            10 | 11 => Self::Automation,
            12 => Self::Marquee,
            13 => Self::Flex,
            14 => Self::Scrub,
            _ => Self::Pointer,
        }
    }

    /// The letter on the tool's button.
    fn letter(self) -> Option<&'static str> {
        Some(match self {
            Self::Scissors => "S",
            Self::Glue => "G",
            Self::Pencil => "P",
            Self::Eraser => "E",
            Self::Text => "T",
            Self::Solo => "O",
            Self::Mute => "M",
            Self::Zoom => "Z",
            Self::Automation => "A",
            Self::Marquee => "Q",
            Self::Flex => "X",
            Self::Scrub => "U",
            Self::Pointer | Self::Fade => return None,
        })
    }

    /// A tool that only clicks on a region: nothing moves.
    fn only_clicks(self) -> bool {
        !matches!(
            self,
            Self::Pointer | Self::Fade | Self::Pencil | Self::Marquee
        )
    }
}

/// What the headers of the tracks show (Track > Configure Track Header).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeaderOpts {
    pub number: bool,
    pub icon: bool,
    pub buttons: bool,
    pub level: bool,
}

impl Default for HeaderOpts {
    fn default() -> Self {
        Self {
            number: true,
            icon: true,
            buttons: true,
            level: true,
        }
    }
}

/// Regions with View > Show Colors off.
const NO_COLOR: Color32 = Color32::from_gray(0x7c);

/// The width of the narrow header left when Show Track Header is off.
const NARROW_HEADER_W: f32 = 112.0;

/// A region edit, from a menu or a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditCmd {
    Cut,
    Copy,
    Paste,
    PasteReplace,
    Split,
    Join,
}

/// Carries out a region edit. What it cannot do goes to the status line.
pub fn run_edit(p: &mut Project, v: &mut TracksView, status: &mut Option<String>, cmd: EditCmd) {
    use crate::region_edit as edit;
    let done = match cmd {
        EditCmd::Copy => edit::copy(p, v),
        EditCmd::Cut => edit::copy(p, v).map(|()| delete_selected(p, v)),
        EditCmd::Paste => edit::paste(p, v).map(|_| ()),
        EditCmd::PasteReplace => edit::paste_replace(p, v),
        EditCmd::Split => edit::split_at_playhead(p, v),
        EditCmd::Join => edit::join_selected(p, v),
    };
    if let Err(e) = done {
        *status = Some(e);
    }
}

/// Which edge of a region is being dragged.
enum TrimDrag {
    End,
    Start(crate::region_edit::TrimStart),
}

/// How close to a region's edge, in points, a drag trims it instead of moving it.
const TRIM_EDGE: f32 = 7.0;

pub struct TracksView {
    /// What Copy or Cut took, and the track it came from.
    pub clipboard: Vec<(usize, Region)>,
    /// Selected besides `selected`, the region last clicked.
    pub also: Vec<(usize, usize)>,
    /// A drag of the selected regions: each, and where it started in beats.
    group_drag: Vec<(usize, usize, f64)>,
    /// A box being dragged over the lanes: where it began, in beats and points down the tracks.
    marquee: Option<(f64, f32)>,
    /// The track under the last right-click on the lanes.
    lane_menu_row: Option<usize>,
    /// A drag on an edge of a region in progress.
    trim: Option<TrimDrag>,
    /// The points picked on the curve on a track, and a drag on one in progress.
    pub auto_pick: Option<crate::automation::LanePick>,
    auto_drag: Option<crate::automation::LaneDrag>,
    /// A track was clicked last, in its header or its mixer strip: Delete means that track.
    pub track_focus: bool,
    /// Width of one bar in points (horizontal zoom).
    pub bar_w: f32,
    /// Track height in points (vertical zoom).
    pub row_h: f32,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub snap: Snap,
    pub drag_mode: DragMode,
    pub tool: TrackTool,
    /// The tool used with Ctrl held (Logic's Command-click tool).
    pub tool2: TrackTool,
    /// A Fade tool drag in progress: `true` for a fade in, `false` for a fade out.
    fade_drag: Option<bool>,
    pub catch: bool,
    /// Global Tempo track above the tracks (Track > Show Tempo Track).
    pub show_tempo: bool,
    /// Global Marker track above the tracks (Track > Show Marker Track).
    pub show_markers: bool,
    /// Global Signature track above the tracks (Track > Show Signature Track).
    pub show_signatures: bool,
    /// Global Arrangement track above the tracks (Track > Show Arrangement Track).
    pub show_arrangement: bool,
    /// Global Chord track above the tracks (Track > Show Chord Track).
    pub show_chords: bool,
    /// (track, region)
    pub selected: Option<(usize, usize)>,
    /// Width of the lanes at the last frame (for Navigate > Zoom to Fit).
    pub lanes_w: f32,
    grab_beats: f64,
    cycle_anchor: Option<f64>,
    /// A swipe on a take row in progress: the comp as it was and where the swipe began.
    swipe: Option<(Vec<crate::model::CompSegment>, f32)>,
    /// Normalize Region Gain scans still running.
    pub normalize_jobs: Vec<NormalizeJob>,
    /// For the layout pictures (`--shots`) only: the right-click menu shown open, of a track
    /// (`(t, None)`) or of a region (`(t, Some(r))`).
    pub shot_menu: Option<(usize, Option<usize>)>,
    /// Smart Tempo analyses still running.
    pub tempo_jobs: Vec<crate::smart_tempo::Job>,
    /// The multitrack Smart Tempo analysis, while it runs.
    pub tempo_all: Option<crate::smart_tempo::AllJob>,
    /// The Smart Tempo Editor window.
    pub tempo_editor: crate::smart_tempo::Editor,
    /// View > Show Track Header: off leaves a narrow column of numbers and names.
    pub show_header: bool,
    pub show_ruler: bool,
    pub show_grid: bool,
    /// View > Show Colors: off draws the regions in gray.
    pub show_colors: bool,
    /// Flex markers are shown on the regions that have them, to drag.
    pub flex_view: bool,
    /// What Cut Section took, for Insert Cut Section.
    pub section_clip: Option<crate::arrangement_track::SectionClip>,
    /// The zoom when last drawn, and what to say about its last move, since when.
    zoom_seen: Option<(f32, f32)>,
    zoom_note: Option<(String, std::time::Instant)>,
    /// A Flex marker being dragged: track, region, marker, where it would land in seconds.
    flex_drag: Option<(usize, usize, usize, f64)>,
    /// What to hear of the Scrub tool: track and beat. `App` plays it.
    pub scrub: Option<(usize, f64)>,
    scrub_at: Option<(f32, std::time::Instant)>,
    /// Track > Configure Track Header.
    pub header_opts: HeaderOpts,
    /// Tracks picked besides the selected one, with Ctrl or Shift on their headers.
    pub picked_tracks: Vec<usize>,
    /// The track whose header is being dragged to a new place.
    track_drag: Option<usize>,
    /// How much larger waveforms are drawn than fits their regions. Not the sound.
    pub wave_zoom: f32,
    /// An entry of the Functions or View menu, for `menus::apply`.
    pub pending_menu: Option<&'static str>,
    /// The groove template the selected regions' notes should be quantized to.
    pub groove_quantize: Option<usize>,
}

impl TracksView {
    pub fn is_selected(&self, region: (usize, usize)) -> bool {
        self.selected == Some(region) || self.also.contains(&region)
    }

    /// Every selected region, the one last clicked first.
    pub fn all_selected(&self) -> Vec<(usize, usize)> {
        self.selected.iter().chain(&self.also).copied().collect()
    }

    pub fn select_only(&mut self, region: (usize, usize)) {
        self.selected = Some(region);
        self.also.clear();
    }

    pub fn select_many(&mut self, regions: Vec<(usize, usize)>) {
        let mut regions = regions.into_iter();
        self.selected = regions.next();
        self.also = regions.collect();
    }

    /// Ctrl+click: in if it was out, out if it was in.
    fn toggle(&mut self, region: (usize, usize)) {
        if self.selected == Some(region) {
            self.selected = self.also.pop();
        } else if let Some(at) = self.also.iter().position(|x| *x == region) {
            self.also.remove(at);
        } else {
            if let Some(last) = self.selected.replace(region) {
                self.also.push(last);
            }
        }
    }

    pub fn select_all(&mut self, p: &Project) {
        let all = p
            .tracks
            .iter()
            .enumerate()
            .flat_map(|(t, track)| (0..track.regions.len()).map(move |r| (t, r)))
            .collect();
        self.select_many(all);
    }

    /// Regions may have gone since they were selected.
    fn tidy_selection(&mut self, p: &Project) {
        let exists = |(t, r): (usize, usize)| p.tracks.get(t).is_some_and(|x| r < x.regions.len());
        self.also.retain(|x| exists(*x));
        if self.selected.is_some_and(|x| !exists(x)) {
            self.selected = None;
        }
        if self.selected.is_none() {
            self.selected = self.also.pop();
        }
    }
}

impl Default for TracksView {
    fn default() -> Self {
        Self {
            track_focus: false,
            clipboard: Vec::new(),
            also: Vec::new(),
            group_drag: Vec::new(),
            marquee: None,
            trim: None,
            auto_pick: None,
            auto_drag: None,
            lane_menu_row: None,
            bar_w: 62.0,
            row_h: 56.0,
            scroll_x: 0.0,
            scroll_y: 0.0,
            lanes_w: 1000.0,
            snap: Snap::Smart,
            drag_mode: DragMode::NoOverlap,
            wave_zoom: 1.0,
            picked_tracks: Vec::new(),
            track_drag: None,
            tool: TrackTool::Pointer,
            tool2: TrackTool::Pointer,
            fade_drag: None,
            catch: true,
            show_tempo: false,
            show_markers: false,
            show_signatures: false,
            show_arrangement: false,
            show_chords: false,
            selected: None,
            grab_beats: 0.0,
            cycle_anchor: None,
            swipe: None,
            normalize_jobs: Vec::new(),
            shot_menu: None,
            tempo_jobs: Vec::new(),
            tempo_all: None,
            tempo_editor: Default::default(),
            show_header: true,
            show_ruler: true,
            show_grid: true,
            show_colors: true,
            flex_view: false,
            section_clip: None,
            zoom_seen: None,
            zoom_note: None,
            flex_drag: None,
            scrub: None,
            scrub_at: None,
            header_opts: HeaderOpts::default(),
            pending_menu: None,
            groove_quantize: None,
        }
    }
}

/// True when another track is soloed and `track` is not: it is silent until solo is released
/// (Logic plays only soloed tracks and greys the others' regions).
pub(crate) fn silenced_by_solo(p: &Project, track: usize) -> bool {
    p.tracks.get(track).is_some() && !p.is_soloed(track) && p.any_solo()
}

/// A region while it is being recorded.
const RECORDING_FILL: Color32 = Color32::from_rgb(86, 14, 14);

/// The outline of a selected region.
const SELECTED_EDGE: Color32 = Color32::from_rgb(176, 200, 72);

/// The ruler over a selected region is gold.
const RULER_SELECTION: Color32 = Color32::from_rgb(0x58, 0x60, 0x6e);
const RULER_SELECTION_EDGE: Color32 = Color32::from_rgb(0x3a, 0x40, 0x4a);
/// The locators while Cycle is off.
const CYCLE_OFF: Color32 = Color32::from_rgb(0x7a, 0x62, 0x2a);

/// Logic's playhead is yellow: a pale yellow line and a yellow head.
pub(crate) const PLAYHEAD_LINE: Color32 = Color32::from_rgb(214, 196, 112);
pub(crate) const PLAYHEAD_HEAD: Color32 = Color32::from_rgb(240, 206, 84);
pub(crate) const PLAYHEAD_EDGE: Color32 = Color32::from_rgb(146, 118, 30);
/// While recording the playhead turns red.
const PLAYHEAD_LINE_REC: Color32 = Color32::from_rgb(176, 42, 38);
const PLAYHEAD_HEAD_REC: Color32 = Color32::from_rgb(206, 32, 26);
const PLAYHEAD_EDGE_REC: Color32 = Color32::from_rgb(112, 16, 14);

/// The playhead's line, head and outline colours.
pub(crate) fn playhead_colors(recording: bool) -> (Color32, Color32, Color32) {
    if recording {
        (PLAYHEAD_LINE_REC, PLAYHEAD_HEAD_REC, PLAYHEAD_EDGE_REC)
    } else {
        (PLAYHEAD_LINE, PLAYHEAD_HEAD, PLAYHEAD_EDGE)
    }
}

/// Half the width of the playhead head's grab zone in the ruler.
const PLAYHEAD_GRAB: f32 = 8.0;

/// Beat under screen x `x` while dragging the playhead (never before the start of the song).
fn playhead_beat(x: f32, lanes_left: f32, scroll_x: f32, beat_px: f32) -> f64 {
    ((x - lanes_left + scroll_x) / beat_px.max(1e-3)).max(0.0) as f64
}

/// Half the height of the grab zone on a track's lower edge.
const RESIZE_GRIP: f32 = 3.0;

/// `(top, height)` of every track row in points, from the top of the first track (unscrolled):
/// the global track zoom `row_h` times each track's own height factor.
fn row_layout(p: &Project, row_h: f32) -> Vec<(f32, f32)> {
    let mut top = 0.0;
    p.tracks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            // The tracks of a folded folder take no room (every loop below skips them).
            let h = if p.hidden_by_fold(i) || p.mixer_only(i) {
                0.0
            } else {
                (row_h * t.height).round().max(MIN_ROW_H) + take_rows_height(t)
            };
            let row = (top, h);
            top += h;
            row
        })
        .collect()
}

/// Index of the track row containing `y` (same origin as [`row_layout`]).
/// Regions dropped on another track: all of them, the one grabbed and where.
type TrackDrop = (
    Vec<(usize, usize)>,
    (usize, usize),
    crate::region_edit::DropRow,
);

/// A click on header `i`: Ctrl adds or takes away, Shift picks from the selected track `from` to it,
/// and a plain click starts over.
fn pick_track(picked: &mut Vec<usize>, from: usize, i: usize, ctrl: bool, shift: bool) {
    if shift {
        picked.clear();
        picked.extend(from.min(i)..=from.max(i));
    } else if ctrl {
        if picked.is_empty() {
            picked.push(from);
        }
        if let Some(at) = picked.iter().position(|t| *t == i) {
            picked.remove(at);
        } else {
            picked.push(i);
        }
    } else {
        picked.clear();
    }
}

/// The tracks picked if `i` is one of them, else `i`.
fn picked_or(picked: &[usize], i: usize) -> Vec<usize> {
    if picked.contains(&i) {
        picked.to_vec()
    } else {
        vec![i]
    }
}

/// Where a region dropped at height `y` of the lanes goes: under the last track counts as a new one.
fn drop_row(rows: &[(f32, f32)], y: f32) -> Option<crate::region_edit::DropRow> {
    use crate::region_edit::DropRow;
    let bottom = rows.iter().map(|r| r.0 + r.1).fold(0.0, f32::max);
    if y >= bottom {
        return Some(DropRow::Below);
    }
    row_at(rows, y).map(DropRow::Track)
}

fn row_at(rows: &[(f32, f32)], y: f32) -> Option<usize> {
    rows.iter().position(|&(top, h)| y >= top && y < top + h)
}

fn id(key: impl std::hash::Hash + std::fmt::Debug) -> egui::Id {
    egui::Id::unique(("tracks_area", key))
}

/// Whether the snap goes to bar lines (which is not a fixed step once signatures change).
fn snaps_to_bars(snap: Snap, beat_px: f32, beat_unit: f64) -> bool {
    match snap {
        Snap::Bar => true,
        Snap::Smart => beat_px * (beat_unit as f32) < 12.0,
        _ => false,
    }
}

/// Fade tool steps per quarter note (1/16 notes, as the Inspector's fades).
const FADE_STEPS_PER_BEAT: f64 = 4.0;

/// The fade (in quarter notes) the Fade tool sets when dragged to `at`, for a region that
/// starts at `start` and is `len` long: from its start for a fade in, from its end for a fade
/// out; in 1/16 notes and never longer than the region.
fn fade_for_drag(at: f64, start: f64, len: f64, fade_in: bool) -> f32 {
    let raw = if fade_in {
        at - start
    } else {
        start + len - at
    };
    ((raw * FADE_STEPS_PER_BEAT).round().max(0.0) / FADE_STEPS_PER_BEAT).min(len) as f32
}

/// Whether region `ri` of track `ti` overlaps another region of the track.
fn overlaps_another(p: &Project, ti: usize, ri: usize) -> bool {
    let track = &p.tracks[ti];
    let (s, e) = (
        p.region_start_beats(&track.regions[ri]),
        p.region_end_beats(&track.regions[ri]),
    );
    track.regions.iter().enumerate().any(|(j, o)| {
        j != ri && p.region_start_beats(o) < e - 1e-9 && p.region_end_beats(o) > s + 1e-9
    })
}

/// Snap step in beats (quarter notes) for the current setting and zoom. `beat_px` is pixels
/// per quarter note; `beat_unit` is the signature's beat in quarter notes (0.5 in 6/8), so
/// Beat snaps to eighths in x/8 as in Logic. Divisions stay 16th notes.
pub(crate) fn snap_step(snap: Snap, beat_px: f32, beats_per_bar: f64, beat_unit: f64) -> f64 {
    match snap {
        Snap::Smart if beat_px >= 48.0 => 0.25,
        Snap::Smart if beat_px * beat_unit as f32 >= 12.0 => beat_unit,
        Snap::Smart => beats_per_bar,
        Snap::Bar => beats_per_bar,
        Snap::Beat => beat_unit,
        Snap::Division => 0.25,
        Snap::Ticks => 1.0 / 960.0,
    }
}

pub(crate) fn snap_to(b: f64, step: f64) -> f64 {
    (b / step).round() * step
}

// ------------------------------------------------------------------ small widgets

fn icon_button(
    ui: &Ui,
    g: &Painter,
    rect: Rect,
    key: &str,
    on: bool,
    draw: impl FnOnce(&Painter, Rect, Color32),
) -> egui::Response {
    let resp = ui.interact(rect, id(("icon", key)), Sense::click());
    if on {
        g.rect_filled(rect, 4.0, BLUE);
    } else if resp.hovered() {
        g.rect_filled(rect, 4.0, Color32::from_white_alpha(14));
    }
    draw(g, rect, if on { Color32::WHITE } else { TEXT });
    resp
}

/// Horizontal mini slider; returns the new 0..1 value.
fn mini_slider(ui: &Ui, g: &Painter, rect: Rect, key: &str, value: f32) -> f32 {
    let resp = ui.interact(
        rect.expand2(vec2(4.0, 6.0)),
        id(("slider", key)),
        Sense::click_and_drag(),
    );
    let mut v = value;
    if let Some(p) = resp.interact_pointer_pos() {
        v = ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
    }
    let cy = rect.center().y;
    g.line_segment(
        [pos2(rect.left(), cy), pos2(rect.right(), cy)],
        Stroke::new(2.0, crate::theme::BORDER),
    );
    g.circle_filled(
        pos2(rect.left() + rect.width() * v, cy),
        5.5,
        Color32::from_gray(0xc8),
    );
    v
}

fn header_button_bg(selected_row: bool) -> Color32 {
    if selected_row {
        Color32::from_gray(0x72)
    } else {
        Color32::from_gray(0x5a)
    }
}

#[allow(clippy::too_many_arguments)]
fn header_button(
    ui: &Ui,
    g: &Painter,
    rect: Rect,
    key: (usize, &str),
    label: &str,
    on: bool,
    on_col: Color32,
    selected_row: bool,
) -> bool {
    let hint = match label {
        "M" => "Mute (M)",
        "S" => "Solo (S)",
        "R" => "Record Enable",
        "I" => "Input Monitoring",
        "A" => "Show the automation on the track",
        _ => "",
    };
    let mut resp = ui.interact(rect, id(("hdrbtn", key)), Sense::click());
    if !hint.is_empty() {
        resp = resp.on_hover_text(hint);
    }
    g.rect_filled(
        rect,
        3.0,
        if on {
            on_col
        } else {
            header_button_bg(selected_row)
        },
    );
    let txt = if on && on_col == SOLO_ON {
        Color32::BLACK
    } else {
        Color32::from_gray(0xe8)
    };
    g.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(10.5),
        txt,
    );
    resp.clicked()
}

// ------------------------------------------------------------------ toolbar icons

fn draw_grid_icon(g: &Painter, r: Rect, c: Color32) {
    for (dx, dy) in [(-3.5, -3.5), (3.5, -3.5), (-3.5, 3.5), (3.5, 3.5)] {
        g.rect_filled(
            Rect::from_center_size(r.center() + vec2(dx, dy), vec2(5.0, 5.0)),
            1.0,
            c,
        );
    }
}

fn draw_list_icon(g: &Painter, r: Rect, c: Color32) {
    for i in -1..=1 {
        let y = r.center().y + i as f32 * 4.0;
        g.rect_filled(
            Rect::from_center_size(pos2(r.center().x, y), vec2(13.0, 2.4)),
            1.0,
            c,
        );
    }
}

fn draw_curve_icon(g: &Painter, r: Rect, c: Color32) {
    let p = r.center();
    let pts: Vec<Pos2> = (0..=12)
        .map(|i| {
            let t = i as f32 / 12.0;
            p + vec2(-7.0 + 14.0 * t, 5.0 - 10.0 * (t * t * (3.0 - 2.0 * t)))
        })
        .collect();
    g.add(Shape::line(pts, stroke(c)));
    g.circle_filled(p + vec2(-7.0, 5.0), 1.8, c);
    g.circle_filled(p + vec2(7.0, -5.0), 1.8, c);
}

fn draw_flex_icon(g: &Painter, r: Rect, c: Color32) {
    // Hourglass on its side (Flex).
    let p = r.center();
    g.add(Shape::line(
        vec![
            p + vec2(-7.0, -5.0),
            p + vec2(7.0, 5.0),
            p + vec2(7.0, -5.0),
            p + vec2(-7.0, 5.0),
            p + vec2(-7.0, -5.0),
        ],
        stroke(c),
    ));
}

fn draw_catch_icon(g: &Painter, r: Rect, c: Color32) {
    // Playhead with a follow arrow.
    let p = r.center();
    g.line_segment([p + vec2(-4.0, -7.0), p + vec2(-4.0, 7.0)], stroke(c));
    g.add(Shape::convex_polygon(
        vec![
            p + vec2(-8.0, -7.0),
            p + vec2(0.0, -7.0),
            p + vec2(-4.0, -3.0),
        ],
        c,
        Stroke::NONE,
    ));
    g.line_segment([p + vec2(-1.0, 2.0), p + vec2(6.0, 2.0)], stroke(c));
    g.add(Shape::line(
        vec![p + vec2(3.5, -0.5), p + vec2(6.5, 2.0), p + vec2(3.5, 4.5)],
        stroke(c),
    ));
}

fn draw_pointer(g: &Painter, at: Pos2, c: Color32) {
    let pts = vec![
        at + vec2(-3.0, -7.0),
        at + vec2(5.0, 1.0),
        at + vec2(1.0, 1.5),
        at + vec2(-1.0, 6.0),
        at + vec2(-3.0, -7.0),
    ];
    g.add(Shape::line(pts, stroke(c)));
}

/// The Fade tool's icon: a ramp down to a baseline.
fn draw_fade_tool(g: &Painter, at: Pos2, c: Color32) {
    g.line_segment([at + vec2(-6.0, 5.0), at + vec2(6.0, 5.0)], stroke(c));
    g.line_segment([at + vec2(-6.0, 5.0), at + vec2(-6.0, -5.0)], stroke(c));
    g.line_segment([at + vec2(-6.0, -5.0), at + vec2(6.0, 5.0)], stroke(c));
}

fn draw_crosshair(g: &Painter, at: Pos2, c: Color32) {
    g.line_segment([at + vec2(-6.0, 0.0), at + vec2(6.0, 0.0)], stroke(c));
    g.line_segment([at + vec2(0.0, -6.0), at + vec2(0.0, 6.0)], stroke(c));
}

fn draw_zoom_icon(g: &Painter, at: Pos2, c: Color32, vertical: bool) {
    let (a, b) = if vertical {
        (vec2(0.0, -6.0), vec2(0.0, 6.0))
    } else {
        (vec2(-6.0, 0.0), vec2(6.0, 0.0))
    };
    g.line_segment([at + a, at + b], stroke(c));
    let n = if vertical {
        vec2(2.5, 0.0)
    } else {
        vec2(0.0, 2.5)
    };
    let d = if vertical {
        vec2(0.0, 2.5)
    } else {
        vec2(2.5, 0.0)
    };
    g.add(Shape::line(
        vec![at + a + n + d, at + a, at + a - n + d],
        stroke(c),
    ));
    g.add(Shape::line(
        vec![at + b + n - d, at + b, at + b - n - d],
        stroke(c),
    ));
}

fn shortcut(ui: &Ui, mods: Modifiers, key: Key) -> String {
    ui.ctx().format_shortcut(&KeyboardShortcut::new(mods, key))
}

// ------------------------------------------------------------------ toolbar

fn toolbar(ui: &Ui, p: &mut Project, s: &mut UiState, bar: Rect, lanes_w: f32) {
    let g = ui.painter_at(bar);
    g.rect_filled(bar, 0.0, TOOLBAR_BG);
    g.line_segment(
        [bar.left_bottom(), bar.right_bottom()],
        Stroke::new(1.0, crate::theme::BORDER),
    );
    let cy = bar.center().y;
    let (can_undo, can_redo) = (s.can_undo, s.can_redo);
    let mut undo_pick = None;
    let v = &mut s.tracks;

    // Back chevron (dim: no previous view to return to).
    let c = pos2(bar.left() + 16.0, cy);
    g.add(Shape::line(
        vec![c + vec2(3.0, -5.0), c + vec2(-2.0, 0.0), c + vec2(3.0, 5.0)],
        stroke(TEXT_DIM),
    ));

    let mut x = bar.left() + 32.0;
    let sel = v.selected;
    let mut edit_pick = None;
    let (edit, w) = menu_title(ui, &g, x, cy, tr("Edit"), id(("menu", "Edit")));
    x += w + 2.0;
    egui::Popup::menu(&edit).show(|ui| {
        ui.set_min_width(260.0);
        crate::widgets::undo_items(ui, can_undo, can_redo, &mut undo_pick);
        ui.separator();
        let has_clip = !v.clipboard.is_empty();
        for (label, key, enabled, action) in [
            (tr("Cut"), Key::X, sel.is_some(), EditCmd::Cut),
            (tr("Copy"), Key::C, sel.is_some(), EditCmd::Copy),
            (tr("Paste"), Key::V, has_clip, EditCmd::Paste),
        ] {
            let keys = shortcut(ui, Modifiers::COMMAND, key);
            if ui
                .add_enabled(enabled, egui::Button::new(label).shortcut_text(keys))
                .clicked()
            {
                edit_pick = Some(action);
                ui.close();
            }
        }
        if ui
            .add_enabled(
                sel.is_some(),
                egui::Button::new(tr("Delete")).shortcut_text(tr("Del")),
            )
            .clicked()
        {
            delete_selected(p, v);
            ui.close();
        }
        ui.separator();
        let select_keys = shortcut(ui, Modifiers::COMMAND, Key::A);
        if ui
            .add(egui::Button::new(tr("Select All")).shortcut_text(select_keys))
            .clicked()
        {
            v.select_all(p);
            ui.close();
        }
        let can_split = crate::region_edit::can_split_at_playhead(p, v);
        let can_join = crate::region_edit::can_join(p, v);
        for (label, key, enabled, action) in [
            (
                tr("Split Regions at Playhead"),
                Key::T,
                can_split,
                EditCmd::Split,
            ),
            (tr("Join Regions"), Key::J, can_join, EditCmd::Join),
        ] {
            let keys = shortcut(ui, Modifiers::COMMAND, key);
            if ui
                .add_enabled(enabled, egui::Button::new(label).shortcut_text(keys))
                .clicked()
            {
                edit_pick = Some(action);
                ui.close();
            }
        }
        let rep = shortcut(ui, Modifiers::COMMAND, Key::R);
        if ui
            .add_enabled(
                sel.is_some(),
                egui::Button::new(tr("Repeat Regions")).shortcut_text(rep),
            )
            .clicked()
        {
            repeat_selected(p, v);
            ui.close();
        }
    });
    if undo_pick.is_some() {
        s.undo_cmd = undo_pick;
    }
    if let Some(cmd) = edit_pick {
        run_edit(p, v, &mut s.status, cmd);
    }
    let (func, w) = menu_title(ui, &g, x, cy, tr("Functions"), id(("menu", "Functions")));
    x += w + 2.0;
    egui::Popup::menu(&func).show(|ui| {
        ui.set_min_width(260.0);
        if ui
            .add_enabled(
                sel.is_some(),
                egui::Button::new(tr("Set Locators by Regions")),
            )
            .clicked()
        {
            set_locators_by_region(p, v);
            ui.close();
        }
        ui.separator();
        if ui.button(tr("Folder")).clicked() {
            v.pending_menu = Some("Create Track Stack…");
            ui.close();
        }
        let any = v.selected.is_some();
        ui.add_enabled_ui(any, |ui| {
            ui.menu_button(tr("Quantize"), |ui| {
                for (text, label) in [
                    ("1/4 Note", "Quantize 1/4"),
                    ("1/8 Note", "Quantize 1/8"),
                    ("1/16 Note", "Quantize 1/16"),
                    ("1/32 Note", "Quantize 1/32"),
                ] {
                    if ui.button(text).clicked() {
                        v.pending_menu = Some(label);
                        ui.close();
                    }
                }
                if !p.grooves.is_empty() {
                    ui.separator();
                }
                for (i, groove) in p.grooves.iter().enumerate() {
                    if ui.button(format!("Groove: {}", groove.name)).clicked() {
                        v.groove_quantize = Some(i);
                        ui.close();
                    }
                }
            });
            if ui.button("Make Groove Template").clicked() {
                v.pending_menu = Some("Make Groove Template");
                ui.close();
            }
            if ui.button(tr("Bounce in Place…")).clicked() {
                v.pending_menu = Some("Bounce In Place");
                ui.close();
            }
            let audio = v.selected.is_some_and(|(t, r)| {
                p.tracks
                    .get(t)
                    .and_then(|t| t.regions.get(r))
                    .is_some_and(|r| r.audio.is_some())
            });
            let convert = egui::Button::new(tr("Convert to New Sampler Track…"));
            if ui.add_enabled(audio, convert).clicked() {
                v.pending_menu = Some("Convert to New Sampler Track");
                ui.close();
            }
            if ui
                .add_enabled(audio, egui::Button::new(tr("Strip Silence…")))
                .clicked()
            {
                v.pending_menu = Some("Strip Silence…");
                ui.close();
            }
            if ui.button(tr("Move to Playhead")).clicked() {
                v.pending_menu = Some("Move to Playhead");
                ui.close();
            }
        });
    });
    let (view, w) = menu_title(ui, &g, x, cy, tr("View"), id(("menu", "View")));
    x += w + 10.0;
    egui::Popup::menu(&view).show(|ui| {
        ui.set_min_width(260.0);
        if ui.button(tr("Zoom to Fit")).clicked() {
            zoom_to_fit(p, v, lanes_w);
            ui.close();
        }
        if ui.button(tr("Zoom In")).clicked() {
            v.bar_w = (v.bar_w * 1.25).min(600.0);
            ui.close();
        }
        if ui.button(tr("Zoom Out")).clicked() {
            v.bar_w = (v.bar_w / 1.25).max(6.0);
            ui.close();
        }
        ui.separator();
        if ui.checkbox(&mut v.catch, tr("Catch Playhead")).clicked() {
            ui.close();
        }
        if ui.button(tr("Show Global Tracks")).clicked() {
            v.pending_menu = Some("Show Global Tracks");
            ui.close();
        }
        if ui.button(tr("Configure Track Header…")).clicked() {
            v.pending_menu = Some("Configure Track Header…");
            ui.close();
        }
    });

    // View buttons: grid (Live Loops) / list (Tracks, active).
    let r = Rect::from_center_size(pos2(x + 13.0, cy), vec2(26.0, 20.0));
    let _ = icon_button(ui, &g, r, "grid", false, draw_grid_icon);
    let r = Rect::from_center_size(pos2(x + 41.0, cy), vec2(26.0, 20.0));
    let _ = icon_button(ui, &g, r, "list", true, draw_list_icon);
    x += 66.0;
    let r = Rect::from_center_size(pos2(x + 13.0, cy), vec2(26.0, 20.0));
    let shows_auto = p.tracks.get(p.selected_track).is_some_and(|t| t.show_auto);
    let auto_button = icon_button(ui, &g, r, "automation", shows_auto, draw_curve_icon)
        .on_hover_text("Show the selected track's automation on the track");
    if auto_button.clicked() {
        if let Some(t) = p.tracks.get_mut(p.selected_track) {
            t.show_auto = !t.show_auto;
        }
    }
    let r = Rect::from_center_size(pos2(x + 41.0, cy), vec2(26.0, 20.0));
    if icon_button(ui, &g, r, "flex", v.flex_view, draw_flex_icon).clicked() {
        v.flex_view = !v.flex_view;
    }
    let r = Rect::from_center_size(pos2(x + 69.0, cy), vec2(26.0, 20.0));
    if icon_button(ui, &g, r, "catch", v.catch, draw_catch_icon).clicked() {
        v.catch = !v.catch;
    }
    let left_end = x + 90.0;

    // Right-hand side, laid out from the right edge.
    // Horizontal zoom, on a log scale.
    let hz = Rect::from_min_size(pos2(bar.right() - 72.0, cy - 6.0), vec2(64.0, 12.0));
    let (lo, hi) = (MIN_BAR_W.ln(), MAX_BAR_W.ln());
    let t = ((v.bar_w.ln() - lo) / (hi - lo)).clamp(0.0, 1.0);
    let t2 = mini_slider(ui, &g, hz, "hzoom", t);
    if t2 != t {
        v.bar_w = (lo + t2 * (hi - lo)).exp();
    }
    draw_zoom_icon(&g, pos2(hz.left() - 13.0, cy), TEXT, false);
    // Vertical zoom, on a log scale: from a row with one line of text to a very tall one.
    let vz = Rect::from_min_size(pos2(hz.left() - 26.0 - 54.0, cy - 6.0), vec2(54.0, 12.0));
    let (lo, hi) = (MIN_ROW_H.ln(), MAX_ROW_H.ln());
    let t = ((v.row_h.ln() - lo) / (hi - lo)).clamp(0.0, 1.0);
    let t2 = mini_slider(ui, &g, vz, "vzoom", t);
    if t2 != t {
        v.row_h = (lo + t2 * (hi - lo)).exp();
    }
    draw_zoom_icon(&g, pos2(vz.left() - 13.0, cy), TEXT, true);
    // What fits between the two sides; the rest is left out rather than drawn on top.
    let mut rx = vz.left() - 30.0;
    // SNAP_W and DRAG_W allow for the English "Snap:" and "Drag:"; longer translations take
    // the difference from the room left.
    let label_w = |t: &str| {
        g.layout_no_wrap(t.to_owned(), FontId::proportional(12.5), TEXT_DIM)
            .size()
            .x
    };
    let longer = (label_w(tr("Snap:")) - label_w("Snap:")).max(0.0)
        + (label_w(tr("Drag:")) - label_w("Drag:")).max(0.0);
    let show = fit_toolbar(rx - left_end - longer);
    // Waveform size: only how they are drawn, the sound stays as it is.
    for i in 0..2 {
        if !show.wave {
            break;
        }
        let c = pos2(rx - 10.0 - i as f32 * 26.0, cy);
        let hover = if i == 0 {
            "Smaller waveforms. The sound does not change. Right-click to reset."
        } else {
            "Larger waveforms. The sound does not change. Right-click to reset."
        };
        let resp = ui
            .interact(
                Rect::from_center_size(c, vec2(24.0, 20.0)),
                id(("wave_size", i)),
                Sense::click(),
            )
            .on_hover_text(hover);
        if resp.clicked() {
            let by = if i == 0 {
                1.0 / WAVE_ZOOM_STEP
            } else {
                WAVE_ZOOM_STEP
            };
            v.wave_zoom = (v.wave_zoom * by).clamp(MIN_WAVE_ZOOM, MAX_WAVE_ZOOM);
        }
        if resp.secondary_clicked() {
            v.wave_zoom = 1.0;
        }
        let col = if resp.hovered() { TEXT } else { TEXT_DIM };
        g.line_segment([c + vec2(-5.0, -6.0), c + vec2(-5.0, 6.0)], stroke(col));
        g.line_segment([c + vec2(5.0, -6.0), c + vec2(5.0, 6.0)], stroke(col));
        g.line_segment([c + vec2(-3.0, 0.0), c + vec2(3.0, 0.0)], stroke(col));
        if i == 1 {
            g.line_segment([c + vec2(0.0, -3.0), c + vec2(0.0, 3.0)], stroke(col));
        }
    }
    if show.wave {
        rx -= 62.0;
    }

    if show.drag {
        rx = drag_menu(ui, &g, v, rx, cy);
    }
    if show.snap {
        rx = snap_menu(ui, &g, v, &mut p.snap_zero, rx, cy);
    }
    if show.tools {
        tool_menus(ui, &g, v, rx, cy);
    }
}

/// Which optional toolbar items are shown, given the free width between the left-hand group
/// and the zoom sliders. Logic hides toolbar items that do not fit; most useful first: Snap,
/// Drag, the two tool menus, then the waveform zoom buttons.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct ToolbarFit {
    snap: bool,
    drag: bool,
    tools: bool,
    wave: bool,
}

/// Widths (with their labels and gaps) of the optional toolbar items.
const SNAP_W: f32 = 204.0;
const DRAG_W: f32 = 160.0;
const TOOLS_W: f32 = 92.0;
const WAVE_W: f32 = 62.0;
/// One press of a waveform size button.
const WAVE_ZOOM_STEP: f32 = 1.4;
const MIN_WAVE_ZOOM: f32 = 0.25;
const MAX_WAVE_ZOOM: f32 = 8.0;
/// Pixels of dragging the clip gain handle for one dB.
const GAIN_DRAG_PX_PER_DB: f32 = 3.0;

fn fit_toolbar(free: f32) -> ToolbarFit {
    let mut left = free;
    let mut take = |w: f32| {
        let fits = left >= w;
        if fits {
            left -= w;
        }
        fits
    };
    let snap = take(SNAP_W);
    let drag = take(DRAG_W);
    let tools = take(TOOLS_W);
    let wave = take(WAVE_W);
    ToolbarFit {
        snap,
        drag,
        tools,
        wave,
    }
}

/// "Drag:" mode menu ending at `rx`; returns where the next item to its left ends.
fn drag_menu(ui: &Ui, g: &Painter, v: &mut TracksView, rx: f32, cy: f32) -> f32 {
    let drag_r = Rect::from_min_size(pos2(rx - 110.0, cy - 10.0), vec2(110.0, 20.0));
    let resp = popup_button(ui, g, drag_r, id(("popup", "drag")), v.drag_mode.label());
    egui::Popup::menu(&resp).show(|ui| {
        for m in DragMode::ALL {
            if ui.selectable_label(v.drag_mode == m, m.label()).clicked() {
                v.drag_mode = m;
                ui.close();
            }
        }
    });
    let label = g.text(
        pos2(drag_r.left() - 6.0, cy),
        Align2::RIGHT_CENTER,
        tr("Drag:"),
        FontId::proportional(12.5),
        TEXT_DIM,
    );
    label.left() - 8.0
}

/// "Snap:" menu ending at `rx`; returns where the next item to its left ends.
fn snap_menu(ui: &Ui, g: &Painter, v: &mut TracksView, zero: &mut bool, rx: f32, cy: f32) -> f32 {
    let snap_r = Rect::from_min_size(pos2(rx - 150.0, cy - 10.0), vec2(150.0, 20.0));
    let resp = popup_button(ui, g, snap_r, id(("popup", "snap")), v.snap.label());
    egui::Popup::menu(&resp).show(|ui| {
        for m in Snap::ALL {
            if ui.selectable_label(v.snap == m, m.label()).clicked() {
                v.snap = m;
                ui.close();
            }
        }
        ui.separator();
        if ui.checkbox(zero, tr("Snap to Zero Crossings")).clicked() {
            ui.close();
        }
    });
    let label = g.text(
        pos2(snap_r.left() - 6.0, cy),
        Align2::RIGHT_CENTER,
        tr("Snap:"),
        FontId::proportional(12.5),
        TEXT_DIM,
    );
    label.left() - 12.0
}

/// Left-click and Command-click tool menus ending at `rx` (only the Pointer works so far).
fn tool_menus(ui: &Ui, g: &Painter, v: &mut TracksView, rx: f32, cy: f32) {
    for (i, key) in ["tool_l", "tool_r"].iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(rx - 40.0 - (1 - i) as f32 * 44.0, cy - 10.0),
            vec2(40.0, 20.0),
        );
        let resp = ui.interact(r, id(*key), Sense::click());
        if resp.hovered() {
            g.rect_filled(r, 4.0, Color32::from_white_alpha(14));
        }
        let icon_at = pos2(r.left() + 13.0, cy);
        if i == 0 && v.tool == TrackTool::Fade {
            draw_fade_tool(g, icon_at, TEXT);
        } else if let (0, Some(letter)) = (i, v.tool.letter()) {
            g.text(
                icon_at,
                Align2::CENTER_CENTER,
                letter,
                FontId::proportional(11.0),
                TEXT,
            );
        } else if i == 0 {
            draw_pointer(g, icon_at, TEXT);
        } else {
            draw_crosshair(g, icon_at, TEXT);
        }
        let c = pos2(r.right() - 9.0, cy + 1.0);
        g.add(Shape::line(
            vec![
                c + vec2(-3.0, -1.5),
                c + vec2(0.0, 1.5),
                c + vec2(3.0, -1.5),
            ],
            stroke(TEXT_DIM),
        ));
        egui::Popup::menu(&resp).show(|ui| {
            ui.set_min_width(220.0);
            for (n, t) in TOOLS.iter().enumerate() {
                let tool = TrackTool::of_entry(n);
                let current = if i == 0 { v.tool } else { v.tool2 };
                if ui.selectable_label(current == tool, tr(t)).clicked() {
                    if i == 0 {
                        v.tool = tool;
                    } else {
                        v.tool2 = tool;
                    }
                    ui.close();
                }
            }
        });
    }
}

// ------------------------------------------------------------------ right-click menus

/// An entry picked in one of the Tracks area's right-click menus; run once the menus are drawn.
#[derive(Clone, Copy, PartialEq, Debug)]
enum CtxAction {
    Edit(EditCmd),
    NewRegion(usize),
    OpenTrackPianoRoll(usize),
    ToggleRecord(usize),
    ToggleTrackAuto(usize),
    Alternative(usize, crate::alternatives::AltCmd),
    RenameAlternative(usize),
    NewTrack(TrackKind),
    ZoomToFit,
    OpenPianoRoll,
    /// A Pattern or Drummer region becomes an ordinary MIDI region.
    ConvertToMidi(usize, usize),
    /// A clip of a Channel Rack pattern goes its own way, or gets a copy of the pattern.
    MakeUnique(usize, usize),
    ClonePattern(usize, usize),
    ToggleRegionMute(usize, usize),
    TrackColor(usize),
    RenameTrack(usize),
    SplitStems(usize, usize),
    DuplicateTrack(usize, bool),
    RegionColor(usize, usize),
    ToggleReverse(usize, usize),
    ToggleTakeFolder(usize, usize),
    FlattenTakeFolder(usize, usize),
    DeleteTake(usize, usize, usize),
    Normalize(usize, usize),
    /// Smart Tempo: say the region's tempo (false) or make it the project's (true).
    DetectTempo(usize, usize, bool),
    /// Smart Tempo of every audio region together (multitrack).
    AdaptTempoToAll,
    SmartTempoEditor(usize, usize),
    ToggleFreeze(usize),
    FlexMode(usize, usize, prowave_engine::FlexMode),
    Flex(usize, usize, crate::flex_edit::Command),
    FlexRemove(usize, usize),
    StripSilence(usize, usize),
    RepeatRegion,
    LocatorsByRegion,
    DeleteRegion,
    CreateStack(usize),
    CreateSummingStack(usize),
    ReleaseStack(usize),
    ToggleMute(usize),
    ToggleSolo(usize),
    ResetHeight(usize),
    DeleteTrack(usize),
    /// Makes the track the Groove Track, or (if it is) no track.
    ToggleGrooveTrack(usize),
    ToggleFollowGroove(usize),
}

/// What the track menu needs to know about the track that was right-clicked.
struct TrackInfo {
    show_auto: bool,
    /// The versions of its regions and the one playing; `None` for a track without regions.
    alternatives: Option<(Vec<String>, usize)>,
    rec: bool,
    takes_midi: bool,
    i: usize,
    folder: bool,
    mute: bool,
    solo: bool,
    custom_height: bool,
    frozen: bool,
    can_freeze: bool,
    /// The Groove Track, and whether this track follows it (`None`: there is none).
    groove: bool,
    follow_groove: Option<bool>,
}

fn pick(ui: &mut Ui, label: &str, key: &str, action: CtxAction, picked: &mut Option<CtxAction>) {
    if ui
        .add(egui::Button::new(label).shortcut_text(key))
        .clicked()
    {
        *picked = Some(action);
    }
}

fn new_track_items(ui: &mut Ui, picked: &mut Option<CtxAction>) {
    for (label, kind) in [
        (tr("New Software Instrument Track"), TrackKind::Instrument),
        (tr("New Audio Track"), TrackKind::Audio),
        (tr("New Aux Track"), TrackKind::Aux),
        (tr("New VCA Track"), TrackKind::Vca),
        (tr("New External MIDI Track"), TrackKind::ExternalMidi),
    ] {
        pick(ui, label, "", CtxAction::NewTrack(kind), picked);
    }
}

#[allow(clippy::too_many_arguments)]
fn region_items(
    ui: &mut Ui,
    is_audio: bool,
    muted: bool,
    folder: Option<bool>,
    generated: bool,
    linked: bool,
    ti: usize,
    ri: usize,
    picked: &mut Option<CtxAction>,
) {
    if !is_audio {
        let open = if generated {
            tr("Open Editor")
        } else {
            tr("Open in Piano Roll")
        };
        pick(ui, open, "", CtxAction::OpenPianoRoll, picked);
        if generated {
            pick(
                ui,
                tr("Convert to MIDI Region"),
                "",
                CtxAction::ConvertToMidi(ti, ri),
                picked,
            );
        }
        if linked {
            pick(ui, "Make Unique", "", CtxAction::MakeUnique(ti, ri), picked);
            pick(
                ui,
                "Clone Pattern",
                "",
                CtxAction::ClonePattern(ti, ri),
                picked,
            );
        }
        ui.separator();
    }
    if let Some(open) = folder {
        let toggle = if open {
            tr("Close Take Folder")
        } else {
            tr("Open Take Folder")
        };
        pick(ui, toggle, "", CtxAction::ToggleTakeFolder(ti, ri), picked);
        pick(
            ui,
            tr("Flatten Take Folder"),
            "",
            CtxAction::FlattenTakeFolder(ti, ri),
            picked,
        );
        ui.separator();
    }
    let mute = if muted {
        tr("Unmute Region")
    } else {
        tr("Mute Region")
    };
    pick(ui, mute, "", CtxAction::ToggleRegionMute(ti, ri), picked);
    pick(
        ui,
        "Region Color…",
        "",
        CtxAction::RegionColor(ti, ri),
        picked,
    );
    if is_audio {
        pick(
            ui,
            "Split Stems…",
            "",
            CtxAction::SplitStems(ti, ri),
            picked,
        );
    }
    if is_audio {
        pick(
            ui,
            tr("Normalize Region Gain"),
            "",
            CtxAction::Normalize(ti, ri),
            picked,
        );
        ui.menu_button(tr("Smart Tempo"), |ui| {
            pick(
                ui,
                tr("Smart Tempo Editor…"),
                "",
                CtxAction::SmartTempoEditor(ti, ri),
                picked,
            );
            pick(
                ui,
                tr("Detect Tempo"),
                "",
                CtxAction::DetectTempo(ti, ri, false),
                picked,
            );
            pick(
                ui,
                tr("Adapt Project Tempo to Region"),
                "",
                CtxAction::DetectTempo(ti, ri, true),
                picked,
            );
            pick(
                ui,
                tr("Adapt Project Tempo to All Audio Regions"),
                "",
                CtxAction::AdaptTempoToAll,
                picked,
            );
            pick(
                ui,
                tr("Region Follows Project Tempo"),
                "",
                CtxAction::Flex(ti, ri, crate::flex_edit::Command::FollowProjectTempo),
                picked,
            );
        });
        ui.menu_button(tr("Flex Time"), |ui| {
            ui.menu_button(tr("Flex Mode"), |ui| {
                for mode in prowave_engine::FlexMode::ALL {
                    pick(
                        ui,
                        crate::flex_edit::mode_label(mode),
                        "",
                        CtxAction::FlexMode(ti, ri, mode),
                        picked,
                    );
                }
            });
            pick(
                ui,
                tr("Quantize Audio to 1/16"),
                "",
                CtxAction::Flex(ti, ri, crate::flex_edit::Command::Quantize { grid: 0.25 }),
                picked,
            );
            pick(
                ui,
                tr("Quantize Audio to 1/8"),
                "",
                CtxAction::Flex(ti, ri, crate::flex_edit::Command::Quantize { grid: 0.5 }),
                picked,
            );
            pick(
                ui,
                tr("Remove Flex Edits"),
                "",
                CtxAction::FlexRemove(ti, ri),
                picked,
            );
        });
        ui.menu_button(tr("Flex Pitch"), |ui| {
            pick(
                ui,
                tr("Correct Pitch to Key"),
                "",
                CtxAction::Flex(ti, ri, crate::flex_edit::Command::CorrectPitch),
                picked,
            );
            pick(
                ui,
                tr("Create MIDI Track from Flex Pitch"),
                "",
                CtxAction::Flex(ti, ri, crate::flex_edit::Command::MidiFromPitch),
                picked,
            );
        });
        pick(
            ui,
            tr("Reverse Region"),
            "",
            CtxAction::ToggleReverse(ti, ri),
            picked,
        );
        pick(
            ui,
            tr("Strip Silence…"),
            "",
            CtxAction::StripSilence(ti, ri),
            picked,
        );
    }
    let repeat_key = ui
        .ctx()
        .format_shortcut(&egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::R));
    pick(
        ui,
        tr("Repeat Region"),
        &repeat_key,
        CtxAction::RepeatRegion,
        picked,
    );
    pick(
        ui,
        tr("Set Locators by Region"),
        "",
        CtxAction::LocatorsByRegion,
        picked,
    );
    ui.separator();
    pick(
        ui,
        tr("Cut"),
        "Ctrl+X",
        CtxAction::Edit(EditCmd::Cut),
        picked,
    );
    pick(
        ui,
        tr("Copy"),
        "Ctrl+C",
        CtxAction::Edit(EditCmd::Copy),
        picked,
    );
    pick(
        ui,
        tr("Split at Playhead"),
        "Ctrl+T",
        CtxAction::Edit(EditCmd::Split),
        picked,
    );
    pick(
        ui,
        tr("Join"),
        "Ctrl+J",
        CtxAction::Edit(EditCmd::Join),
        picked,
    );
    pick(ui, tr("Delete"), "⌫", CtxAction::DeleteRegion, picked);
}

/// What the menus need to know of track `i`.
fn track_info(p: &Project, i: usize) -> Option<TrackInfo> {
    let t = p.tracks.get(i)?;
    Some(TrackInfo {
        show_auto: t.show_auto,
        alternatives: t.kind.has_regions().then(|| {
            let names = t.alternatives.iter().map(|a| a.name.clone()).collect();
            (names, t.active_alt)
        }),
        i,
        folder: t.kind == TrackKind::Folder,
        mute: t.mute,
        solo: t.solo,
        rec: t.rec,
        takes_midi: matches!(t.kind, TrackKind::Instrument | TrackKind::ExternalMidi),
        custom_height: t.height != 1.0,
        frozen: t.frozen.is_some(),
        can_freeze: crate::bounce::can_freeze(t),
        groove: t.params.groove,
        follow_groove: crate::groove::groove_track(p)
            .filter(|g| *g != i)
            .map(|_| t.params.follow_groove),
    })
}

/// The entries for the versions of track `i`'s regions.
fn alternative_items(
    ui: &mut Ui,
    i: usize,
    names: &[String],
    active: usize,
    picked: &mut Option<CtxAction>,
) {
    use crate::alternatives::AltCmd;
    for (k, name) in names.iter().enumerate() {
        if ui.selectable_label(k == active, name).clicked() {
            *picked = Some(CtxAction::Alternative(i, AltCmd::Switch(k)));
            ui.close();
        }
    }
    if !names.is_empty() {
        ui.separator();
    }
    for (label, cmd) in [
        ("New Alternative", AltCmd::New),
        ("Duplicate Alternative", AltCmd::Duplicate),
    ] {
        if ui.button(label).clicked() {
            *picked = Some(CtxAction::Alternative(i, cmd));
            ui.close();
        }
    }
    if ui
        .add_enabled(
            active < names.len(),
            egui::Button::new("Rename Alternative"),
        )
        .clicked()
    {
        *picked = Some(CtxAction::RenameAlternative(i));
        ui.close();
    }
    let delete = egui::Button::new("Delete Alternative");
    if ui.add_enabled(names.len() > 1, delete).clicked() {
        *picked = Some(CtxAction::Alternative(i, AltCmd::Delete));
        ui.close();
    }
}

/// What the menus of a track have first, wherever it is right-clicked.
fn track_action_items(ui: &mut Ui, t: &TrackInfo, picked: &mut Option<CtxAction>) {
    if t.takes_midi {
        pick(
            ui,
            tr("Open in Piano Roll"),
            "",
            CtxAction::OpenTrackPianoRoll(t.i),
            picked,
        );
    }
    let mute = if t.mute {
        tr("Unmute Track")
    } else {
        tr("Mute Track")
    };
    pick(ui, mute, "M", CtxAction::ToggleMute(t.i), picked);
    let solo = if t.solo {
        tr("Unsolo Track")
    } else {
        tr("Solo Track")
    };
    pick(ui, solo, "S", CtxAction::ToggleSolo(t.i), picked);
    let record = if t.rec {
        "Disarm Recording"
    } else {
        "Arm for Recording"
    };
    pick(ui, record, "", CtxAction::ToggleRecord(t.i), picked);
    if !t.folder {
        let auto = if t.show_auto {
            "Hide Automation on Track"
        } else {
            "Show Automation on Track"
        };
        pick(ui, auto, "", CtxAction::ToggleTrackAuto(t.i), picked);
    }
    let delete = ui.button(tr("Delete Track"));
    if delete.clicked() {
        *picked = Some(CtxAction::DeleteTrack(t.i));
        ui.close();
    }
}

fn track_items(ui: &mut Ui, t: &TrackInfo, picked: &mut Option<CtxAction>) {
    track_action_items(ui, t, picked);
    pick(ui, "Rename Track", "", CtxAction::RenameTrack(t.i), picked);
    pick(ui, "Track Color…", "", CtxAction::TrackColor(t.i), picked);
    pick(
        ui,
        "Duplicate Track",
        "",
        CtxAction::DuplicateTrack(t.i, true),
        picked,
    );
    pick(
        ui,
        "New Track with Same Settings",
        "Ctrl+D",
        CtxAction::DuplicateTrack(t.i, false),
        picked,
    );
    ui.separator();
    new_track_items(ui, picked);
    ui.separator();
    if let Some((names, active)) = &t.alternatives {
        ui.menu_button("Track Alternatives", |ui| {
            alternative_items(ui, t.i, names, *active, picked);
        });
    }
    pick(
        ui,
        tr("Create Track Stack"),
        "",
        CtxAction::CreateStack(t.i),
        picked,
    );
    pick(
        ui,
        "Create Summing Stack",
        "",
        CtxAction::CreateSummingStack(t.i),
        picked,
    );
    if t.folder {
        pick(
            ui,
            tr("Release Track Stack"),
            "",
            CtxAction::ReleaseStack(t.i),
            picked,
        );
    }
    ui.separator();
    if t.custom_height {
        pick(
            ui,
            tr("Reset Track Height"),
            "",
            CtxAction::ResetHeight(t.i),
            picked,
        );
    }
    ui.separator();
    let groove = if t.groove {
        tr("Clear Groove Track")
    } else {
        tr("Set as Groove Track")
    };
    pick(ui, groove, "", CtxAction::ToggleGrooveTrack(t.i), picked);
    if t.frozen || t.can_freeze {
        let label = if t.frozen {
            "Unfreeze Track"
        } else {
            "Freeze Track"
        };
        pick(ui, label, "", CtxAction::ToggleFreeze(t.i), picked);
    }
    if let Some(follows) = t.follow_groove {
        let label = if follows {
            tr("Stop Following Groove Track")
        } else {
            tr("Follow Groove Track")
        };
        pick(ui, label, "", CtxAction::ToggleFollowGroove(t.i), picked);
    }
}

/// Runs a right-click menu entry.
fn apply_ctx(action: CtxAction, p: &mut Project, s: &mut UiState, lanes_w: f32) {
    let v = &mut s.tracks;
    match action {
        CtxAction::Edit(cmd) => run_edit(p, v, &mut s.status, cmd),
        CtxAction::MakeUnique(t, r) => crate::song_pattern::make_unique(p, t, r),
        CtxAction::ClonePattern(t, r) => {
            crate::song_pattern::clone_for_region(p, t, r);
        }
        CtxAction::NewRegion(t) => {
            let bar = p.beats_to_bar(p.position_beats).floor() as f32 + 1.0;
            if let Some(r) = p.add_midi_region(t, bar) {
                v.select_only((t, r));
                p.selected_track = t;
                s.bottom = Bottom::PianoRoll;
            }
        }
        CtxAction::OpenTrackPianoRoll(t) => {
            p.selected_track = t;
            v.select_many(Vec::new());
            s.bottom = Bottom::PianoRoll;
        }
        CtxAction::ToggleRecord(t) => {
            if let Some(track) = p.tracks.get_mut(t) {
                track.rec = !track.rec;
            }
        }
        CtxAction::ToggleTrackAuto(t) => {
            if let Some(track) = p.tracks.get_mut(t) {
                track.show_auto = !track.show_auto;
            }
        }
        CtxAction::RenameAlternative(t) => {
            let name = p
                .tracks
                .get(t)
                .and_then(|x| x.alternatives.get(x.active_alt))
                .map(|a| a.name.clone());
            if let Some(name) = name {
                s.dialogs.rename_alternative = Some((t, name));
            }
        }
        CtxAction::Alternative(t, cmd) => match crate::alternatives::run(p, t, cmd) {
            Ok(()) => {
                // The region numbers now mean other regions.
                v.select_many(Vec::new());
                s.piano.selected.clear();
            }
            Err(e) => s.status = Some(e),
        },
        CtxAction::NewTrack(kind) => {
            let t = p.add_track(kind);
            // Asked for from the Tracks area, so it shows there.
            p.tracks[t].on_timeline = true;
        }
        CtxAction::ZoomToFit => zoom_to_fit(p, v, lanes_w),
        CtxAction::OpenPianoRoll => s.bottom = Bottom::PianoRoll,
        CtxAction::ConvertToMidi(t, r) => p.convert_to_midi_region(t, r),
        CtxAction::ToggleRegionMute(t, r) => {
            let group = if v.is_selected((t, r)) {
                v.all_selected()
            } else {
                vec![(t, r)]
            };
            let on = p
                .tracks
                .get(t)
                .and_then(|x| x.regions.get(r))
                .is_some_and(|x| !x.params.mute);
            for (t, r) in group {
                if let Some(region) = p.tracks.get_mut(t).and_then(|x| x.regions.get_mut(r)) {
                    region.params.mute = on;
                }
            }
            p.song_rev += 1;
        }
        CtxAction::ToggleTakeFolder(t, r) => {
            if let Some(f) = p
                .tracks
                .get_mut(t)
                .and_then(|t| t.regions.get_mut(r))
                .and_then(|r| r.folder.as_mut())
            {
                f.open = !f.open;
            }
        }
        // The comp as it is becomes an ordinary region: the other takes are let go.
        CtxAction::FlattenTakeFolder(t, r) => {
            let made = crate::takes::flatten(p, t, r);
            v.select_many(made.into_iter().map(|r| (t, r)).collect());
        }
        CtxAction::DeleteTake(t, r, take) => {
            let left = crate::takes::delete_take(p, t, r, take);
            v.select_many(left.into_iter().map(|r| (t, r)).collect());
        }
        CtxAction::ToggleReverse(t, r) => {
            if let Some(region) = p.tracks.get_mut(t).and_then(|t| t.regions.get_mut(r)) {
                region.params.reverse = !region.params.reverse;
            }
        }
        CtxAction::Normalize(t, r) => start_normalize(p, s, t, r),
        CtxAction::DetectTempo(t, r, adapt) => crate::smart_tempo::start(p, s, t, r, adapt),
        CtxAction::AdaptTempoToAll => crate::smart_tempo::start_all(p, s),
        CtxAction::ToggleFreeze(i) => crate::bounce::toggle_freeze(p, s, i),
        CtxAction::ToggleGrooveTrack(i) => {
            let was = p.tracks.get(i).is_some_and(|t| t.params.groove);
            crate::groove::set_groove_track(p, (!was).then_some(i));
        }
        CtxAction::ToggleFollowGroove(i) => {
            if let Some(t) = p.tracks.get_mut(i) {
                t.params.follow_groove = !t.params.follow_groove;
                p.song_rev += 1;
            }
        }
        CtxAction::SmartTempoEditor(t, r) => crate::smart_tempo::open_editor(p, s, t, r),
        CtxAction::FlexMode(t, r, mode) => {
            crate::flex_edit::set_mode(p, t, r, mode);
            p.song_rev += 1;
        }
        CtxAction::Flex(t, r, command) => crate::flex_edit::start(p, s, t, r, command),
        CtxAction::FlexRemove(t, r) => s.flex.remove_later(t, r),
        CtxAction::StripSilence(t, r) => crate::strip_silence::open(p, s, t, r),
        CtxAction::RepeatRegion => repeat_selected(p, v),
        CtxAction::LocatorsByRegion => set_locators_by_region(p, v),
        CtxAction::DeleteRegion => delete_selected(p, v),
        CtxAction::TrackColor(i) => s.dialogs.color = Some(i),
        CtxAction::RenameTrack(i) => {
            if let Some(name) = p.tracks.get(i).map(|t| t.name.clone()) {
                s.dialogs.open_rename(i, &name);
            }
        }
        CtxAction::SplitStems(t, r) => crate::stem_splitter::open(p, s, t, r),
        CtxAction::DuplicateTrack(i, with_regions) => {
            if p.duplicate_track(i, with_regions).is_some() {
                forget_track_indexes(s);
            }
        }
        CtxAction::RegionColor(t, r) => {
            let group = if v.is_selected((t, r)) {
                v.all_selected()
            } else {
                vec![(t, r)]
            };
            s.dialogs.region_color = group;
        }
        CtxAction::CreateStack(i) => {
            let tracks = picked_or(&s.tracks.picked_tracks, i);
            p.stack_tracks(&tracks);
            forget_track_indexes(s);
        }
        CtxAction::CreateSummingStack(i) => {
            let tracks = picked_or(&s.tracks.picked_tracks, i);
            if p.summing_stack(&tracks).is_none() {
                s.status = Some("No bus is free for a Summing Stack".to_string());
            }
            forget_track_indexes(s);
        }
        CtxAction::ReleaseStack(i) => {
            if p.release_track_stack(i) {
                forget_track_indexes(s);
            }
        }
        CtxAction::ToggleMute(i) => {
            let mute = !p.tracks[i].mute;
            p.set_folder_flags(i, Some(mute), None);
        }
        CtxAction::ToggleSolo(i) => {
            let solo = !p.tracks[i].solo;
            p.set_folder_flags(i, None, Some(solo));
        }
        CtxAction::ResetHeight(i) => p.tracks[i].height = 1.0,
        CtxAction::DeleteTrack(i) => {
            if p.delete_track(i) {
                forget_track_indexes(s);
            }
        }
    }
}

/// Logic's Normalize Region Gain: finds the region's loudest sample on a background thread,
/// then [`poll_normalize`] sets the gain that brings it to 0 dBFS. The file is not changed.
fn start_normalize(p: &Project, s: &mut UiState, track: usize, region: usize) {
    let Some(audio) = p
        .tracks
        .get(track)
        .and_then(|t| t.regions.get(region))
        .and_then(|r| r.audio.as_ref())
    else {
        s.status = Some(tr("Normalize: this region has no recorded audio").into());
        return;
    };
    let jobs = &mut s.tracks.normalize_jobs;
    if jobs.iter().any(|j| (j.track, j.region) == (track, region)) {
        return;
    }
    let file = audio.file.clone();
    let path = file.path.clone();
    let (tx, result) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("prowave-normalize".into())
        .spawn(move || {
            let peak = prowave_engine::peak_of(&file).map_err(|e| e.to_string());
            let _ = tx.send(peak);
        });
    match spawned {
        Ok(_) => {
            jobs.push(NormalizeJob {
                track,
                region,
                path,
                result,
            });
            s.status = Some(tr("Normalizing…").into());
        }
        Err(e) => s.status = Some(format!("cannot start the normalize scan: {e}")),
    }
}

/// Applies the Normalize Region Gain scans that have finished. Call once per frame.
pub fn poll_normalize(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    use std::sync::mpsc::TryRecvError;
    if s.tracks.normalize_jobs.is_empty() {
        return;
    }
    let mut finished = Vec::new();
    let mut message = None;
    s.tracks
        .normalize_jobs
        .retain(|job| match job.result.try_recv() {
            Err(TryRecvError::Empty) => true,
            Err(TryRecvError::Disconnected) => false,
            Ok(peak) => {
                finished.push((job.track, job.region, job.path.clone(), peak));
                false
            }
        });
    for (track, region, path, peak) in finished {
        // The project may have changed while the file was being read.
        let Some(r) = p
            .tracks
            .get_mut(track)
            .and_then(|t| t.regions.get_mut(region))
            .filter(|r| r.audio.as_ref().is_some_and(|a| a.file.path == path))
        else {
            continue;
        };
        match peak {
            Ok(peak) => match crate::model::RegionParams::normalize_gain(peak) {
                Some(gain) => r.params.gain_db = gain,
                None => message = Some(tr("Normalize: the region is silent").to_string()),
            },
            Err(e) => {
                message = Some(trf("Normalize: cannot read {}: {}", &[&path.display(), &e]));
            }
        }
    }
    if s.tracks.normalize_jobs.is_empty() {
        // Clears "Normalizing…" (or shows what went wrong).
        s.status = message;
    } else {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

/// Track numbers shifted, so a remembered region or note selection may point at another track.
pub(crate) fn forget_track_indexes(s: &mut UiState) {
    s.tracks.picked_tracks.clear();
    s.tracks.track_drag = None;
    s.tracks.selected = None;
    s.piano.selected.clear();
}

// ------------------------------------------------------------------ actions

pub(crate) fn delete_selected(p: &mut Project, v: &mut TracksView) {
    let mut gone = v.all_selected();
    v.select_many(Vec::new());
    // From the back, so the numbers of those still to go stay right.
    gone.sort_unstable_by(|a, b| b.cmp(a));
    gone.dedup();
    for (t, r) in gone {
        if let Some(track) = p.tracks.get_mut(t) {
            if r < track.regions.len() {
                track.regions.remove(r);
                p.song_rev += 1;
            }
        }
    }
}

/// Copies of the selected regions right after them, which end up selected.
pub(crate) fn repeat_selected(p: &mut Project, v: &mut TracksView) {
    let selected = v.all_selected();
    let regions = || {
        selected
            .iter()
            .filter_map(|(t, r)| p.tracks.get(*t).and_then(|x| x.regions.get(*r)))
    };
    let start = regions().map(|r| r.start_bar).fold(f32::INFINITY, f32::min);
    let end = regions()
        .map(|r| r.start_bar + r.len_bars)
        .fold(f32::NEG_INFINITY, f32::max);
    if selected.is_empty() || !start.is_finite() {
        return;
    }
    let span = end - start;
    let mut copies = Vec::with_capacity(selected.len());
    for (t, r) in selected.clone() {
        let Some(src) = p.tracks.get(t).and_then(|x| x.regions.get(r)).cloned() else {
            continue;
        };
        let start_bar = src.start_bar + span;
        p.tracks[t].regions.push(Region { start_bar, ..src });
        copies.push((t, p.tracks[t].regions.len() - 1));
    }
    v.select_many(copies);
    p.song_rev += 1;
}

fn set_locators_by_region(p: &mut Project, v: &TracksView) {
    let Some((t, r)) = v.selected else { return };
    let Some(reg) = p.tracks.get(t).and_then(|tr| tr.regions.get(r)) else {
        return;
    };
    let (start, end) = (p.region_start_beats(reg), p.region_end_beats(reg));
    p.cycle_start = start;
    p.cycle_end = end;
    p.cycle = true;
}

pub(crate) fn zoom_to_fit(p: &Project, v: &mut TracksView, lanes_w: f32) {
    let end = p
        .tracks
        .iter()
        .flat_map(|t| t.regions.iter())
        .map(|r| r.start_bar - 1.0 + r.len_bars)
        .fold(0.0f32, f32::max);
    if end > 0.0 {
        v.bar_w = ((lanes_w - 20.0) / end).clamp(MIN_BAR_W, MAX_BAR_W);
        v.scroll_x = 0.0;
    }
}

// ------------------------------------------------------------------ drawing

/// The body colour of a region. Logic draws regions deeper than the track colour; these four
/// were measured from Logic Pro itself (yellow is estimated from the others).
const LOGIC_REGION_COLORS: [([u8; 3], [u8; 3]); 4] = [
    ([0x4e, 0xb0, 0x4a], [43, 150, 53]),
    ([0x3f, 0x5f, 0xc0], [60, 102, 176]),
    ([0xd9, 0xaa, 0x2a], [196, 158, 26]),
    ([0x7a, 0x4f, 0xc0], [97, 63, 175]),
];

/// A selected region keeps its colour but gets deeper and more saturated (Logic: a green
/// region goes from 43, 150, 53 to 15, 136, 29).
fn selected_fill(c: Color32) -> Color32 {
    let (r, g, b) = (c.r() as f32, c.g() as f32, c.b() as f32);
    let lum = 0.3 * r + 0.59 * g + 0.11 * b;
    let k = |v: f32| ((lum + (v - lum) * 1.45) * 0.8).clamp(0.0, 255.0) as u8;
    Color32::from_rgb(k(r), k(g), k(b))
}

fn region_fill(c: Color32) -> Color32 {
    if let Some((_, [r, g, b])) = LOGIC_REGION_COLORS
        .iter()
        .find(|(track, _)| *track == [c.r(), c.g(), c.b()])
    {
        return Color32::from_rgb(*r, *g, *b);
    }
    // Any other colour gets the same kind of darkening.
    let ch = |v: u8| v as f32 * 0.72;
    let (r, g, b) = (ch(c.r()), ch(c.g()), ch(c.b()));
    let lum = 0.3 * r + 0.59 * g + 0.11 * b;
    let sat = |v: f32| (lum + (v - lum) * 1.1).clamp(0.0, 255.0) as u8;
    Color32::from_rgb(sat(r), sat(g), sat(b))
}

// ------------------------------------------------------------------ take folders

/// Height of one take's row in an opened take folder.
const TAKE_ROW_H: f32 = 22.0;
/// The yellow header of a take folder.
const FOLDER_HEAD_H: f32 = 14.0;
/// Logic's take folder header is yellow.
const FOLDER_YELLOW: Color32 = Color32::from_rgb(253, 197, 0);

/// Extra height a track's row needs for its opened take folders (one row per take).
fn take_rows_height(t: &Track) -> f32 {
    t.regions
        .iter()
        .filter_map(|r| r.folder.as_ref())
        .filter(|f| f.open)
        .map(|f| f.takes.len() as f32 * TAKE_ROW_H)
        .fold(0.0, f32::max)
}

/// The triangle that opens and closes a take folder, at the left of its header.
fn folder_toggle_rect(full: Rect) -> Rect {
    Rect::from_min_size(full.min, vec2(16.0, FOLDER_HEAD_H))
}

/// The row of take number `take` (0 = first take) in an opened folder: the latest take is on
/// top, as in Logic. `full` is the folder's own rectangle.
fn take_row_rect(full: Rect, takes: usize, take: usize) -> Rect {
    let k = takes - 1 - take;
    Rect::from_min_size(
        pos2(full.left(), full.bottom() + 1.0 + k as f32 * TAKE_ROW_H),
        vec2(full.width(), TAKE_ROW_H - 2.0),
    )
}

/// Thin note lines (pitch up, time right) of `notes` in `inner`, clipped to `clip`.
#[allow(clippy::too_many_arguments)]
fn draw_note_lines<'a>(
    g: &Painter,
    inner: Rect,
    left: f32,
    beat_px: f32,
    notes: impl Iterator<Item = &'a crate::model::Note>,
    lo: f32,
    hi: f32,
    ink: Color32,
) {
    let step = (2.0f32).min((inner.height() - 4.0) / (hi - lo).max(1.0));
    let mid = (lo + hi) / 2.0;
    for n in notes {
        let nx = left + n.start * beat_px;
        if nx > inner.right() || nx + n.len * beat_px < inner.left() {
            continue;
        }
        let ny = inner.center().y - (n.pitch as f32 - mid) * step;
        let nr = Rect::from_min_max(
            pos2(nx.max(inner.left()), ny - 0.75),
            pos2(
                (nx + (n.len * beat_px).max(2.0)).min(inner.right()),
                ny + 0.75,
            ),
        );
        g.rect_filled(nr, 0.0, ink);
    }
}

/// A take folder as Logic draws it: a yellow header with the name and a triangle, and under
/// it the comp, the stretches of the takes that play, side by side with thin lines between.
/// Opened, a row for each take follows, the stretches the comp uses lit in the region colour.
#[allow(clippy::too_many_arguments)]
fn draw_take_folder(
    g: &Painter,
    full: Rect,
    track: &Track,
    region: &Region,
    folder: &crate::model::TakeFolder,
    beat_px: f32,
    seconds_per_beat: f32,
    selected: bool,
    silenced: bool,
    colored: bool,
    wave_zoom: f32,
) {
    let wave_fit = region.audio.as_ref().map_or(1.0, |a| a.waveform_fit()) * wave_zoom;
    let base = if region.params.mute {
        Color32::from_gray(0x6a)
    } else if colored {
        region
            .params
            .pattern_color
            .or(region.params.color)
            .unwrap_or(track.color)
    } else {
        NO_COLOR
    };
    let fill = if silenced {
        Color32::from_rgb(0x2e, 0x2e, 0x30)
    } else if selected {
        selected_fill(region_fill(base))
    } else {
        region_fill(base)
    };
    let ink = if silenced {
        Color32::from_gray(0x8c)
    } else {
        fill.lerp_to_gamma(Color32::WHITE, if selected { 0.75 } else { 0.6 })
    };
    let head_h = FOLDER_HEAD_H.min(full.height() * 0.4);
    let round = 4;
    let body = Rect::from_min_max(pos2(full.left(), full.top() + head_h), full.max);
    // The comp: one block per stretch, rounded at the ends of the folder only.
    let last = folder.comp.len().saturating_sub(1);
    for (i, seg) in folder.comp.iter().enumerate() {
        let x0 = (full.left() + seg.start * beat_px).max(full.left());
        let x1 = (full.left() + seg.end * beat_px).min(full.right());
        if x1 <= x0 {
            continue;
        }
        g.rect_filled(
            Rect::from_min_max(pos2(x0, body.top()), pos2(x1, body.bottom())),
            egui::CornerRadius {
                nw: 0,
                ne: 0,
                sw: if i == 0 { round } else { 0 },
                se: if i == last { round } else { 0 },
            },
            fill,
        );
    }
    // The header.
    let head = Rect::from_min_size(full.min, vec2(full.width(), head_h));
    let head_fill = if silenced {
        Color32::from_gray(0x70)
    } else {
        FOLDER_YELLOW
    };
    g.rect_filled(
        head,
        egui::CornerRadius {
            nw: round,
            ne: round,
            sw: 0,
            se: 0,
        },
        head_fill,
    );
    let dark = Color32::from_rgb(40, 30, 0);
    // The triangle: pointing right when closed, down when opened.
    let c = pos2(full.left() + 8.0, head.center().y);
    let tri = if folder.open {
        vec![
            c + vec2(-3.5, -2.0),
            c + vec2(3.5, -2.0),
            c + vec2(0.0, 3.0),
        ]
    } else {
        vec![
            c + vec2(-2.0, -3.5),
            c + vec2(-2.0, 3.5),
            c + vec2(3.0, 0.0),
        ]
    };
    g.add(Shape::convex_polygon(tri, dark, Stroke::NONE));
    g.text(
        head.left_center() + vec2(16.0, 0.0),
        Align2::LEFT_CENTER,
        &region.name,
        FontId::proportional(10.5),
        dark,
    );
    // Where one stretch gives way to the next: a notch in the header and a line below it.
    for seg in folder.comp.iter().skip(1) {
        let x = full.left() + seg.start * beat_px;
        if x <= full.left() || x >= full.right() {
            continue;
        }
        g.line_segment(
            [pos2(x, head.bottom() - 4.0), pos2(x, head.bottom())],
            Stroke::new(1.0, dark),
        );
        g.line_segment(
            [pos2(x, body.top()), pos2(x, body.bottom())],
            Stroke::new(1.0, Color32::from_black_alpha(150)),
        );
    }
    // What the comp plays.
    let main_audio = region.audio.as_ref().filter(|_| folder.is_audio());
    let take_wave = |g: &Painter, rect: Rect, take: usize, seg: (f32, f32), col: Color32| {
        let Some(take) = folder.takes.get(take) else {
            return;
        };
        let Some(offset) = take.audio_offset else {
            return;
        };
        let audio = take.file.as_ref().map(|f| &f.0).or(main_audio);
        let Some((peaks, duration)) =
            audio.and_then(|a| Some((a.peaks_for(full.width())?, a.file.duration_secs() as f32)))
        else {
            return;
        };
        if duration <= 0.0 || peaks.is_empty() {
            return;
        }
        let at = |beat: f32| (offset + beat * seconds_per_beat) / duration * peaks.len() as f32;
        let from = (at(seg.0).max(0.0) as usize).min(peaks.len());
        let to = (at(seg.1).max(0.0) as usize).clamp(from, peaks.len());
        let x = (full.left() + seg.0 * beat_px, (seg.1 - seg.0) * beat_px);
        draw_peaks(g, rect, x, &peaks[from..to], wave_fit, false, col);
    };
    let (lo, hi) = pitch_range(&region.notes);
    let inner = Rect::from_min_max(
        pos2(body.left(), body.top() + 2.0),
        body.max - vec2(0.0, 2.0),
    );
    draw_note_lines(
        g,
        inner,
        full.left(),
        beat_px,
        region.notes.iter(),
        lo,
        hi,
        ink,
    );
    for seg in &folder.comp {
        take_wave(g, inner, seg.take, (seg.start, seg.end), ink);
    }
    let border = if selected {
        SELECTED_EDGE
    } else {
        Color32::from_black_alpha(110)
    };
    g.rect_stroke(
        full,
        egui::CornerRadius::same(round),
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );

    // The takes, newest on top.
    if folder.open {
        for (take_idx, take) in folder.takes.iter().enumerate() {
            let row = take_row_rect(full, folder.takes.len(), take_idx);
            g.rect_filled(row, 3.0, Color32::from_gray(0x3a));
            let row_inner =
                Rect::from_min_max(pos2(row.left(), row.top() + 2.0), row.max - vec2(0.0, 2.0));
            let (lo, hi) = pitch_range(&take.notes);
            // The stretches of this take that the comp plays are lit.
            for seg in folder.comp.iter().filter(|s| s.take == take_idx) {
                let x0 = (full.left() + seg.start * beat_px).max(full.left());
                let x1 = (full.left() + seg.end * beat_px).min(full.right());
                if x1 > x0 {
                    g.rect_filled(
                        Rect::from_min_max(pos2(x0, row.top()), pos2(x1, row.bottom())),
                        3.0,
                        fill,
                    );
                }
            }
            take_wave(
                g,
                row_inner,
                take_idx,
                (0.0, folder.len),
                Color32::from_gray(0xa0),
            );
            let used = |start: f32| {
                folder
                    .comp
                    .iter()
                    .any(|s| s.take == take_idx && start >= s.start && start < s.end)
            };
            draw_note_lines(
                g,
                row_inner,
                full.left(),
                beat_px,
                take.notes.iter().filter(|n| used(n.start)),
                lo,
                hi,
                ink,
            );
            draw_note_lines(
                g,
                row_inner,
                full.left(),
                beat_px,
                take.notes.iter().filter(|n| !used(n.start)),
                lo,
                hi,
                Color32::from_gray(0x86),
            );
            g.text(
                row.left_top() + vec2(5.0, 1.0),
                Align2::LEFT_TOP,
                trf("Take {}", &[&(take_idx + 1)]),
                FontId::proportional(9.5),
                Color32::from_white_alpha(200),
            );
        }
    }
}

/// Lowest and highest pitch of `notes` (a default span when there are none).
fn pitch_range(notes: &[crate::model::Note]) -> (f32, f32) {
    let lo = notes.iter().map(|n| n.pitch).min().unwrap_or(48) as f32;
    let hi = notes.iter().map(|n| n.pitch).max().unwrap_or(72) as f32;
    (lo, hi)
}

#[allow(clippy::too_many_arguments)]
fn draw_region(
    g: &Painter,
    rr: Rect,
    full: Rect,
    track: &Track,
    region: &Region,
    fades: (f32, f32),
    beat_px: f32,
    selected: bool,
    recording: bool,
    silenced: bool,
    live: Option<crate::audio_rec::LiveWave<'_>>,
    colored: bool,
    wave_zoom: f32,
) {
    let wave_fit = region.audio.as_ref().map_or(1.0, |a| a.waveform_fit()) * wave_zoom;
    // Muted regions are drawn grey, like Logic.
    let base = if region.params.mute {
        Color32::from_gray(0x6a)
    } else if colored {
        region
            .params
            .pattern_color
            .or(region.params.color)
            .unwrap_or(track.color)
    } else {
        NO_COLOR
    };
    let body = if recording {
        // The region being recorded is red until the recording stops.
        RECORDING_FILL
    } else if silenced {
        // Another track is soloed: dark grey body, grey content, name in the track colour.
        Color32::from_rgb(0x2e, 0x2e, 0x30)
            .lerp_to_gamma(Color32::WHITE, if selected { 0.06 } else { 0.0 })
    } else if selected {
        selected_fill(region_fill(base))
    } else {
        region_fill(base)
    };
    g.rect_filled(rr, 3.0, body);
    let head_h = 14.0f32.min(rr.height() * 0.4);
    let head = Rect::from_min_size(rr.min, vec2(rr.width(), head_h));
    g.rect_filled(
        head,
        egui::CornerRadius {
            nw: 3,
            ne: 3,
            sw: 0,
            se: 0,
        },
        body.lerp_to_gamma(Color32::WHITE, if silenced { 0.03 } else { 0.0 }),
    );
    let name_color = if silenced {
        base.lerp_to_gamma(Color32::WHITE, 0.15)
    } else {
        body.lerp_to_gamma(Color32::WHITE, if selected { 0.7 } else { 0.85 })
    };
    if let Some(generated) = &region.generated {
        if head.width() > 44.0 {
            draw_generated_mark(
                g,
                head,
                matches!(generated.source, crate::model::Generator::Drummer(_)),
                name_color,
            );
        }
    }
    g.text(
        head.left_center() + vec2(5.0, 0.0),
        Align2::LEFT_CENTER,
        &region.name,
        FontId::proportional(10.5),
        name_color,
    );
    let inner = Rect::from_min_max(
        pos2(rr.left(), head.bottom() + 2.0),
        rr.max - vec2(0.0, 2.0),
    );
    // A Flex edit: its mode in the corner of the name bar and its markers over the waveform.
    if let Some((a, f)) = region
        .audio
        .as_ref()
        .and_then(|a| a.flex.as_ref().map(|f| (a, f)))
    {
        if head.width() > 90.0 {
            let mark = if f.pitch.is_empty() {
                crate::flex_edit::mode_label(f.mode).to_string()
            } else {
                format!("{} ♪", crate::flex_edit::mode_label(f.mode))
            };
            g.text(
                head.right_center() - vec2(5.0, 0.0),
                Align2::RIGHT_CENTER,
                mark,
                FontId::proportional(9.5),
                name_color,
            );
        }
        let secs = a.file.duration_secs().max(1e-3);
        for m in &f.markers {
            let x = full.left() + (m.dst / secs) as f32 * full.width();
            if x > inner.left() && x < inner.right() {
                g.line_segment(
                    [pos2(x, inner.top()), pos2(x, inner.bottom())],
                    Stroke::new(1.0, Color32::from_rgb(0xff, 0xb0, 0x30).gamma_multiply(0.7)),
                );
            }
        }
    }
    let ink = if recording {
        Color32::from_rgb(236, 152, 155)
    } else if silenced {
        Color32::from_gray(0x8c)
    } else {
        // Logic draws the content in a pale tint of the region colour, not pure white.
        body.lerp_to_gamma(Color32::WHITE, if selected { 0.75 } else { 0.6 })
    };
    let edit = region.audio.is_some() || track.kind == TrackKind::Audio;
    if let Some(wave) = live {
        let from = full.left() + wave.from_beats as f32 * beat_px;
        let width = wave.len_beats as f32 * beat_px;
        draw_peaks(
            g,
            inner,
            (from, width),
            wave.peaks,
            wave.fit * wave_zoom,
            false,
            ink,
        );
    } else if let Some(peaks) = region
        .audio
        .as_ref()
        .and_then(|a| a.peaks_for(full.width()))
    {
        let gain = 10f32.powf(region.params.gain_db / 20.0) * wave_fit;
        let full_x = (full.left(), full.width());
        let part = played_part(region, peaks);
        draw_peaks(g, inner, full_x, part, gain, region.params.reverse, ink);
    } else if region.audio.is_some() || track.kind == TrackKind::Audio {
        g.line_segment(
            [
                pos2(inner.left(), inner.center().y),
                pos2(inner.right(), inner.center().y),
            ],
            Stroke::new(1.0, ink),
        );
    } else if !region.notes.is_empty() {
        let lo = region.notes.iter().map(|n| n.pitch).min().unwrap_or(48) as f32;
        let hi = region.notes.iter().map(|n| n.pitch).max().unwrap_or(72) as f32;
        // Logic keeps a fixed, small step per semitone and centres the notes in the region.
        let step = (2.0f32).min((inner.height() - 4.0) / (hi - lo).max(1.0));
        let mid = (lo + hi) / 2.0;
        for n in &region.notes {
            let nx = full.left() + n.start * beat_px;
            if nx > inner.right() || nx + n.len * beat_px < inner.left() {
                continue;
            }
            let ny = inner.center().y - (n.pitch as f32 - mid) * step;
            let nr = Rect::from_min_max(
                pos2(nx.max(inner.left()), ny - 0.75),
                pos2(
                    (nx + (n.len * beat_px).max(2.0)).min(inner.right()),
                    ny + 0.75,
                ),
            );
            g.rect_filled(nr, 0.0, ink);
        }
    }
    if edit {
        draw_fades(g, rr, inner, full, region, fades, beat_px);
    }
    let border = if recording {
        Color32::from_rgb(120, 22, 22)
    } else if selected {
        SELECTED_EDGE
    } else if silenced {
        Color32::from_gray(0x44)
    } else {
        Color32::from_black_alpha(110)
    };
    if selected && !recording {
        // Logic outlines a selected region in yellow-green with a dark line around it.
        g.rect_stroke(
            rr,
            3.0,
            Stroke::new(1.0, Color32::from_black_alpha(220)),
            StrokeKind::Outside,
        );
    }
    g.rect_stroke(rr, 3.0, Stroke::new(1.0, border), StrokeKind::Inside);
}

/// A small mark at the right of a region's header that tells a Drummer region (a drum with
/// its sticks) from a Pattern region (a grid of steps).
fn draw_generated_mark(g: &Painter, head: Rect, drummer: bool, color: Color32) {
    let c = pos2(head.right() - 10.0, head.center().y);
    let stroke = Stroke::new(1.0, color);
    if drummer {
        g.rect_stroke(
            Rect::from_center_size(c + vec2(0.0, 1.5), vec2(8.0, 4.5)),
            1.0,
            stroke,
            StrokeKind::Middle,
        );
        g.line_segment([c + vec2(-4.0, -2.0), c + vec2(-1.0, -4.5)], stroke);
        g.line_segment([c + vec2(4.0, -2.0), c + vec2(1.0, -4.5)], stroke);
    } else {
        for row in 0..2 {
            for col in 0..3 {
                let at = c + vec2(col as f32 * 3.6 - 3.6, row as f32 * 3.6 - 1.8);
                g.rect_filled(Rect::from_center_size(at, vec2(2.6, 2.6)), 0.5, color);
            }
        }
    }
}

/// Real waveform from min/max peaks. `x0`/`full_w` describe the unclipped region.
/// Shades what the region's fades take away: a triangle from the region's start up to where
/// the fade in ends, and one down to the region's end for the fade out, with the curve's line.
fn draw_fades(
    g: &Painter,
    rr: Rect,
    inner: Rect,
    full: Rect,
    region: &Region,
    (fin, fout): (f32, f32),
    beat_px: f32,
) {
    if fin <= 0.0 && fout <= 0.0 {
        return;
    }
    let g = g.with_clip_rect(g.clip_rect().intersect(rr));
    let shade = Color32::from_black_alpha(110);
    let line = Stroke::new(1.2, Color32::from_white_alpha(220));
    let curve = |x: f32| match region.params.fade_curve {
        prowave_engine::FadeCurve::Linear => x,
        prowave_engine::FadeCurve::EqualPower => (x * std::f32::consts::FRAC_PI_2).sin(),
        prowave_engine::FadeCurve::SCurve => x * x * (3.0 - 2.0 * x),
    };
    // `x0` where the fade is silent, `x1` where it reaches full level.
    let fade = |x0: f32, x1: f32| {
        const STEPS: usize = 16;
        let pts: Vec<Pos2> = (0..=STEPS)
            .map(|k| {
                let t = k as f32 / STEPS as f32;
                let y = inner.bottom() - curve(t) * inner.height();
                pos2(x0 + (x1 - x0) * t, y)
            })
            .collect();
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            g.add(Shape::convex_polygon(
                vec![pos2(a.x, inner.top()), pos2(b.x, inner.top()), b, a],
                shade,
                Stroke::NONE,
            ));
        }
        g.add(Shape::line(pts, line));
    };
    if fin > 0.0 {
        fade(full.left(), full.left() + fin * beat_px);
    }
    if fout > 0.0 {
        fade(full.right(), full.right() - fout * beat_px);
    }
}

/// Draws a recorded waveform: `full_x` is the left edge and width of the whole region on
/// screen, `gain` its region gain (linear) and `reverse` plays it back to front.
/// The part of a file's overview that an audio region plays.
fn played_part<'a>(region: &Region, peaks: &'a [(f32, f32)]) -> &'a [(f32, f32)] {
    let Some(audio) = region.audio.as_ref() else {
        return peaks;
    };
    let seconds = audio.file.duration_secs() as f32;
    if seconds <= 0.0 {
        return peaks;
    }
    let start = (region.params.source_start / seconds).clamp(0.0, 1.0);
    let end = if region.params.source_len > 0.0 {
        ((region.params.source_start + region.params.source_len) / seconds).clamp(start, 1.0)
    } else {
        1.0
    };
    let bin = |f: f32| ((f * peaks.len() as f32) as usize).min(peaks.len());
    &peaks[bin(start)..bin(end)]
}

/// The whole project small in a corner, with a frame on the part in view.
fn draw_overview(
    g: &Painter,
    p: &Project,
    rows: &[(f32, f32)],
    lanes: Rect,
    (scroll_x, scroll_y): (f32, f32),
    beat_px: f32,
    end_beat: f64,
) {
    let map = Rect::from_min_size(
        lanes.right_bottom() - vec2(198.0, 118.0),
        vec2(190.0, 110.0),
    );
    g.rect_filled(
        map,
        3.0,
        Color32::from_rgba_unmultiplied(0x1a, 0x1a, 0x1e, 235),
    );
    let height = rows
        .last()
        .map_or(1.0, |(top, h)| top + h)
        .max(lanes.height())
        .max(1.0);
    let last_in_view = f64::from((scroll_x + lanes.width()) / beat_px.max(1e-3));
    let beats = end_beat.max(last_in_view).max(1.0) as f32;
    let (sx, sy) = (map.width() / beats, map.height() / height);
    for (t, &(top, row)) in p.tracks.iter().zip(rows) {
        if row <= 0.0 {
            continue;
        }
        for r in &t.regions {
            let from = p.region_start_beats(r) as f32;
            let to = p.region_end_beats(r) as f32;
            let block = Rect::from_min_size(
                map.min + vec2(from * sx, top * sy),
                vec2(((to - from) * sx).max(1.0), (row * sy).max(1.0)),
            );
            g.rect_filled(block.intersect(map), 0.0, t.color);
        }
    }
    let view = Rect::from_min_size(
        map.min + vec2(scroll_x / beat_px.max(1e-3) * sx, scroll_y * sy),
        vec2(lanes.width() / beat_px.max(1e-3) * sx, lanes.height() * sy),
    )
    .intersect(map);
    g.rect_stroke(
        view,
        0.0,
        Stroke::new(1.0, Color32::WHITE),
        egui::StrokeKind::Inside,
    );
}

/// The Flex markers of a region on the region itself: drag one to move that moment of the sound,
/// right-click one to remove it, and double-click the region to add one.
#[allow(clippy::too_many_arguments)]
fn flex_markers(
    ui: &Ui,
    g: &Painter,
    p: &mut Project,
    v: &mut TracksView,
    (ti, ri): (usize, usize),
    rr: Rect,
    full: Rect,
    region_resp: &egui::Response,
) {
    let Some(audio) = p.tracks[ti].regions[ri].audio.as_mut() else {
        return;
    };
    let seconds = audio.file.duration_secs().max(1e-3);
    let Some(flex) = audio.flex.as_mut() else {
        return;
    };
    let x_of = |dst: f64| full.left() + (dst / seconds) as f32 * full.width();
    let mut remove = None;
    let mut changed = false;
    for k in 0..flex.markers.len() {
        let dragged = v.flex_drag.filter(|d| (d.0, d.1, d.2) == (ti, ri, k));
        let dst = dragged.map_or(flex.markers[k].dst, |d| d.3);
        let x = x_of(dst);
        let handle = Rect::from_center_size(pos2(x, rr.top() + 6.0), vec2(9.0, 12.0));
        let resp = ui
            .interact(
                handle,
                id(("flex_marker", ti, ri, k)),
                Sense::click_and_drag(),
            )
            .on_hover_text("Drag to move this moment. Right-click to remove it.");
        let colour = Color32::from_rgb(0xff, 0xb0, 0x30);
        g.line_segment(
            [pos2(x, rr.top()), pos2(x, rr.bottom())],
            Stroke::new(1.0, colour),
        );
        g.rect_filled(handle.shrink(1.0), 1.0, colour);
        if resp.dragged() {
            let low = k.checked_sub(1).map_or(0.0, |i| flex.markers[i].dst) + 0.01;
            let high = flex.markers.get(k + 1).map_or(seconds * 4.0, |m| m.dst) - 0.01;
            let moved = dst + f64::from(resp.drag_delta().x) * seconds / f64::from(full.width());
            v.flex_drag = Some((ti, ri, k, moved.clamp(low, high.max(low))));
        }
        if resp.drag_stopped() {
            if let Some(done) = dragged {
                flex.markers[k].dst = done.3;
                changed = true;
            }
            v.flex_drag = None;
        }
        if resp.secondary_clicked() {
            remove = Some(k);
        }
    }
    if let Some(k) = remove {
        flex.markers.remove(k);
        changed = true;
    }
    if region_resp.double_clicked() {
        if let Some(at) = ui.input(|i| i.pointer.latest_pos()) {
            let dst = f64::from((at.x - full.left()) / full.width().max(1.0)) * seconds;
            // The moment of the recording that plays there now.
            let next = flex.markers.iter().position(|m| m.dst > dst);
            let src = match next {
                Some(0) | None if flex.markers.is_empty() => dst,
                Some(0) => dst * flex.markers[0].src / flex.markers[0].dst.max(1e-6),
                None => {
                    let last = &flex.markers[flex.markers.len() - 1];
                    last.src + (dst - last.dst)
                }
                Some(n) => {
                    let (a, b) = (&flex.markers[n - 1], &flex.markers[n]);
                    let along = (dst - a.dst) / (b.dst - a.dst).max(1e-6);
                    a.src + along * (b.src - a.src)
                }
            };
            let at = next.unwrap_or(flex.markers.len());
            flex.markers
                .insert(at, prowave_engine::WarpPoint { src, dst });
            changed = true;
        }
    }
    if changed {
        p.song_rev += 1;
    }
}

/// How a waveform's height follows the level: below 1, quiet parts are drawn taller.
const WAVE_CURVE: f32 = 0.6;

fn draw_peaks(
    g: &Painter,
    body: Rect,
    (x0, full_w): (f32, f32),
    peaks: &[(f32, f32)],
    gain: f32,
    reverse: bool,
    col: Color32,
) {
    if peaks.is_empty() || full_w <= 0.0 {
        return;
    }
    let mid = body.center().y;
    let half = body.height() * 0.48;
    // One straight line per screen pixel, on whole pixels, so none blurs or fades into a dash.
    let ppp = g.ctx().pixels_per_point();
    let px = 1.0 / ppp;
    let snap_up = |y: f32| (y * ppp).floor() / ppp;
    let snap_down = |y: f32| (y * ppp).ceil() / ppp;
    // Quiet passages stand out without zooming in: the height follows a curve, not the level.
    let height = |level: f32| level.abs().min(1.0).powf(WAVE_CURVE) * half;
    let mut mesh = egui::Mesh::default();
    let mut x = (body.left() * ppp).floor() / ppp;
    while x < body.right() {
        if x + px <= x0 {
            x += px;
            continue;
        }
        let a = (((x - x0) / full_w) * peaks.len() as f32).max(0.0) as usize;
        if a >= peaks.len() {
            break;
        }
        let b =
            ((((x + px - x0) / full_w) * peaks.len() as f32) as usize).clamp(a + 1, peaks.len());
        let (a, b) = if reverse {
            (peaks.len() - b, peaks.len() - a)
        } else {
            (a, b)
        };
        let (lo, hi) = peaks[a..b]
            .iter()
            .fold((0f32, 0f32), |(l, h), p| (l.min(p.0), h.max(p.1)));
        let top = snap_up(mid - height(hi * gain));
        // Silence is still a thin line, with no gaps in it.
        let bottom = snap_down(mid + height(lo * gain)).max(top + px);
        mesh.add_colored_rect(Rect::from_min_max(pos2(x, top), pos2(x + px, bottom)), col);
        x += px;
    }
    g.add(egui::Shape::mesh(mesh));
}

/// Triangle pointing down (open) or right (folded), drawn as a shape so it needs no font glyph.
fn disclosure_triangle(g: &Painter, c: Pos2, open: bool, col: Color32) {
    let pts = if open {
        vec![
            c + vec2(-4.0, -2.5),
            c + vec2(4.0, -2.5),
            c + vec2(0.0, 3.5),
        ]
    } else {
        vec![
            c + vec2(-2.5, -4.0),
            c + vec2(-2.5, 4.0),
            c + vec2(3.5, 0.0),
        ]
    };
    g.add(Shape::convex_polygon(pts, col, Stroke::NONE));
}

/// A folder's M and S buttons: they set the same state on every track inside the folder.
fn folder_buttons(ui: &Ui, g: &Painter, p: &mut Project, i: usize, x: f32, y: f32, selected: bool) {
    let size = vec2(18.0, 15.0);
    let (mute, solo) = (p.tracks[i].mute, p.tracks[i].solo);
    if header_button(
        ui,
        g,
        Rect::from_min_size(pos2(x, y), size),
        (i, "m"),
        "M",
        mute,
        MUTE_ON,
        selected,
    ) {
        p.set_folder_flags(i, Some(!mute), None);
    }
    if header_button(
        ui,
        g,
        Rect::from_min_size(pos2(x + 21.0, y), size),
        (i, "s"),
        "S",
        solo,
        SOLO_ON,
        selected,
    ) {
        p.set_folder_flags(i, None, Some(!solo));
    }
}

const SLIDER_W: f32 = 78.0;
const SLIDER_H: f32 = 14.0;

/// Where 0 dB sits on a track header's volume slider (Logic puts unity about two thirds up).
const SLIDER_UNITY: f32 = 0.65;

/// Slider position (0..1) for a volume in dB (-60..+6): most of the travel is below unity,
/// the last third is the boost to +6 dB.
fn db_to_slider(db: f32) -> f32 {
    let db = db.clamp(-60.0, 6.0);
    if db >= 0.0 {
        SLIDER_UNITY + (1.0 - SLIDER_UNITY) * db / 6.0
    } else {
        SLIDER_UNITY * ((db + 60.0) / 60.0).sqrt()
    }
}

fn slider_to_db(f: f32) -> f32 {
    let f = f.clamp(0.0, 1.0);
    if f >= SLIDER_UNITY {
        (f - SLIDER_UNITY) / (1.0 - SLIDER_UNITY) * 6.0
    } else {
        -60.0 + 60.0 * (f / SLIDER_UNITY).powi(2)
    }
}

/// The room the mode and parameter of the curve on a track take on the name's row.
const LANE_POPUPS_W: f32 = 196.0;

/// The automation mode of track `t` and the control whose curve it shows.
fn lane_popups(ui: &Ui, g: &Painter, t: &mut Track, i: usize, hdr: Rect, y: f32, beat: f64) {
    use crate::automation::{lane_choices, lane_label, set_mode};
    let mode = Rect::from_min_size(pos2(hdr.right() - 76.0, y - 9.0), vec2(70.0, 18.0));
    let resp = popup_button(ui, g, mode, id(("lane_mode", i)), tr(t.auto.mode.label()));
    egui::Popup::menu(&resp).show(|ui| {
        for m in crate::model::AutoMode::ALL {
            if ui
                .selectable_label(t.auto.mode == m, tr(m.label()))
                .clicked()
            {
                set_mode(t, m, beat);
                ui.close();
            }
        }
    });
    let lane = Rect::from_min_size(pos2(mode.left() - 114.0, y - 9.0), vec2(110.0, 18.0));
    let label = lane_label(t, t.auto.shown);
    let resp = popup_button(ui, g, lane, id(("lane_target", i)), &label);
    egui::Popup::menu(&resp).show(|ui| {
        egui::ScrollArea::vertical()
            .max_height(320.0)
            .show(ui, |ui| {
                for c in lane_choices(t) {
                    let dot = if t.auto.lane(c).is_empty() {
                        "  "
                    } else {
                        "• "
                    };
                    let name = format!("{dot}{}", lane_label(t, c));
                    if ui.selectable_label(t.auto.shown == c, name).clicked() {
                        t.auto.shown = c;
                        ui.close();
                    }
                }
            });
    });
}

/// Draws track `i`'s header and returns the response of its background (for the right-click
/// menu; the buttons on it handle their own clicks).
#[allow(clippy::too_many_arguments)]
fn header(
    ui: &Ui,
    g: &Painter,
    p: &mut Project,
    i: usize,
    hdr: Rect,
    peaks: (f32, f32),
    silenced: bool,
    opts: HeaderOpts,
    picked: &mut Option<CtxAction>,
) -> (egui::Response, Vec<egui::Response>) {
    let selected = p.selected_track == i;
    g.rect_filled(hdr, 0.0, if selected { HEADER_SEL } else { HEADER_BG });
    g.rect_filled(hdr, 0.0, p.tracks[i].color.gamma_multiply(0.07));
    if selected {
        let mark = Rect::from_min_size(hdr.min, vec2(3.0, hdr.height()));
        g.rect_filled(mark, 0.0, crate::theme::ACCENT_BLUE);
    }
    g.line_segment(
        [hdr.left_bottom(), hdr.right_bottom()],
        Stroke::new(1.0, Color32::from_gray(0x2a)),
    );
    let num_w = if opts.number { NUM_W } else { 0.0 };
    if opts.number {
        let num = Rect::from_min_size(hdr.min, vec2(NUM_W, hdr.height()));
        g.rect_filled(num, 0.0, p.tracks[i].color);
        g.text(
            num.center(),
            Align2::CENTER_CENTER,
            (i + 1).to_string(),
            FontId::proportional(12.0),
            Color32::WHITE,
        );
    }
    let resp = ui.interact(hdr, id(("hdr", i)), Sense::click_and_drag());
    if resp.clicked() || context_click(&resp) {
        p.selected_track = i;
    }

    let h = hdr.height();
    // A tall row keeps its name and controls at the top, with the icon large beneath them.
    let stacked = h >= 110.0 && opts.icon;
    let mid = if stacked {
        hdr.top() + 30.0
    } else {
        hdr.center().y
    };

    // It grows with the row, and on a thin row fills what there is.
    let icon_size = if stacked {
        (h - 58.0).min(150.0)
    } else if h < 36.0 {
        (h - 2.0).max(8.0)
    } else {
        (h - 14.0).min(40.0)
    };
    let indent = f32::from(p.tracks[i].indent) * INDENT_W;
    let folder = p.tracks[i].kind == TrackKind::Folder;
    let content_x = hdr.left() + num_w + 6.0 + indent;
    if folder {
        // Fold arrow: click to hide or show the tracks inside.
        let arrow = Rect::from_center_size(pos2(content_x + 5.0, hdr.center().y), vec2(14.0, 20.0));
        if ui
            .interact(arrow, id(("fold", i)), Sense::click())
            .clicked()
        {
            p.tracks[i].folded = !p.tracks[i].folded;
        }
        disclosure_triangle(g, arrow.center(), !p.tracks[i].folded, TEXT_DIM);
    }
    let icon = Rect::from_min_size(
        if stacked {
            pos2(
                (content_x + hdr.right() - icon_size) / 2.0,
                hdr.top() + 50.0 + (h - 56.0 - icon_size) / 2.0,
            )
        } else {
            pos2(
                content_x + if folder { 14.0 } else { 0.0 },
                hdr.center().y - icon_size / 2.0,
            )
        },
        vec2(icon_size, icon_size),
    );
    if opts.icon {
        crate::track_icons::draw_tinted(g, icon, track_icon(&p.tracks[i]), p.tracks[i].color);
    }

    let name_x = if stacked {
        content_x + if folder { 14.0 } else { 0.0 }
    } else if opts.icon {
        icon.right() + 10.0
    } else {
        icon.left()
    };
    let compact = h < 46.0;
    // On a compact row M and S stay, ahead of the name.
    let inline_buttons = compact && opts.buttons && !folder;
    let name_x = if inline_buttons {
        name_x + 42.0
    } else {
        name_x
    };
    let name_y = if compact { hdr.center().y } else { mid - 9.0 };
    let lane_controls = p.tracks[i].show_auto && !folder && !compact && opts.buttons;
    let name_right = if lane_controls {
        LANE_POPUPS_W
    } else if opts.level {
        8.0
    } else {
        6.0
    };
    let name_w = hdr.right() - name_right - name_x;
    let shown_name = if p.tracks[i].frozen.is_some() {
        format!("{} (Frozen)", p.tracks[i].name)
    } else {
        p.tracks[i].name.clone()
    };
    let name_galley = g.layout_no_wrap(
        shown_name,
        FontId::proportional((h - 2.0).clamp(9.0, 13.0)),
        TEXT,
    );
    let clip = g.with_clip_rect(
        Rect::from_min_size(pos2(name_x, hdr.top()), vec2(name_w.max(20.0), h))
            .intersect(g.clip_rect()),
    );
    clip.galley(
        pos2(name_x, name_y - name_galley.size().y / 2.0),
        name_galley,
        TEXT,
    );

    if lane_controls {
        let beat = p.position_beats;
        lane_popups(ui, g, &mut p.tracks[i], i, hdr, name_y, beat);
    }
    // A folder only has M and S, which act on everything inside it, and no volume or pan.
    if folder {
        if !compact && opts.buttons {
            folder_buttons(ui, g, p, i, name_x, mid + 3.0, selected);
        }
        return (resp, Vec::new());
    }

    // M S R (I) buttons under the name (compact tracks: hidden, like Logic at small heights).
    let audio = p.tracks[i].kind == TrackKind::Audio;
    let t = &mut p.tracks[i];
    if inline_buttons {
        let size = vec2(18.0, (h - 2.0).min(15.0));
        let by = hdr.center().y - size.y / 2.0;
        if header_button(
            ui,
            g,
            Rect::from_min_size(pos2(name_x - 42.0, by), size),
            (i, "m"),
            "M",
            t.mute,
            MUTE_ON,
            selected,
        ) {
            t.mute = !t.mute;
        }
        if header_button(
            ui,
            g,
            Rect::from_min_size(pos2(name_x - 21.0, by), size),
            (i, "s"),
            "S",
            t.solo,
            SOLO_ON,
            selected,
        ) {
            t.solo = !t.solo;
        }
    }
    if !compact && opts.buttons {
        let by = mid + 3.0;
        let mut bx = name_x;
        let size = vec2(18.0, 15.0);
        // Power: off makes it silent and rests its plugins.
        let power = Rect::from_min_size(pos2(bx, by), size);
        if header_button(
            ui,
            g,
            power,
            (i, "power"),
            "",
            !t.off,
            crate::theme::ACCENT_BLUE,
            selected,
        ) {
            t.off = !t.off;
        }
        let (c, pen) = (power.center(), Stroke::new(1.2, Color32::WHITE));
        g.add(Shape::line(
            (0..=12)
                .map(|k| {
                    let a = (-35.0 + 250.0 * k as f32 / 12.0_f32).to_radians();
                    c + vec2(a.cos(), a.sin()) * 4.0
                })
                .collect(),
            pen,
        ));
        g.line_segment([c + vec2(0.0, -5.0), c + vec2(0.0, -1.0)], pen);
        bx += 21.0;
        if header_button(
            ui,
            g,
            Rect::from_min_size(pos2(bx, by), size),
            (i, "m"),
            "M",
            t.mute,
            MUTE_ON,
            selected,
        ) {
            t.mute = !t.mute;
        }
        bx += 21.0;
        if header_button(
            ui,
            g,
            Rect::from_min_size(pos2(bx, by), size),
            (i, "s"),
            "S",
            t.solo,
            SOLO_ON,
            selected,
        ) {
            t.solo = !t.solo;
        }
        bx += 21.0;
        // Aux and Folder tracks hold no regions, so there is nothing to record into.
        if t.kind.has_regions()
            && header_button(
                ui,
                g,
                Rect::from_min_size(pos2(bx, by), size),
                (i, "r"),
                "R",
                t.rec,
                REC_ON,
                selected,
            )
        {
            t.rec = !t.rec;
        }
        if audio {
            bx += 21.0;
            if header_button(
                ui,
                g,
                Rect::from_min_size(pos2(bx, by), size),
                (i, "i"),
                "I",
                t.monitor,
                MON_ON,
                selected,
            ) {
                t.monitor = !t.monitor;
            }
        }
        bx += 21.0;
        if header_button(
            ui,
            g,
            Rect::from_min_size(pos2(bx, by), size),
            (i, "a"),
            "A",
            t.show_auto,
            AUTO_ON,
            selected,
        ) {
            t.show_auto = !t.show_auto;
        }
    }

    if t.kind.has_regions() && opts.buttons && !compact {
        // Freeze: a snowflake of three strokes.
        let frost = Rect::from_center_size(
            pos2(
                name_x + (if audio { 6.0 } else { 5.0 }) * 21.0 + 7.0,
                mid + 10.5,
            ),
            vec2(14.0, 15.0),
        );
        let frozen = t.frozen.is_some();
        let hit = ui
            .interact(frost, id((i, "freeze")), Sense::click())
            .on_hover_text(if frozen { "Unfreeze" } else { "Freeze" });
        if hit.clicked() {
            *picked = Some(CtxAction::ToggleFreeze(i));
        }
        let ink = if frozen {
            Color32::from_rgb(0x7f, 0xc8, 0xf0)
        } else {
            TEXT_DIM
        };
        for k in 0..3 {
            let a = (k as f32 * 60.0 + 30.0).to_radians();
            let d = vec2(a.cos(), a.sin()) * 5.0;
            g.line_segment(
                [frost.center() - d, frost.center() + d],
                Stroke::new(1.1, ink),
            );
        }
    }
    if !opts.level {
        // A dot for the level, with the fader hidden.
        let lit = peaks.0.max(peaks.1) > 0.001;
        let dot = pos2(hdr.right() - 12.0, mid);
        g.circle_filled(
            dot,
            3.0,
            if lit {
                crate::theme::GOOD
            } else {
                Color32::from_gray(0x2a)
            },
        );
        return (resp, Vec::new());
    }
    // A row this thin has room for no more than its name.
    if h < 20.0 {
        return (resp, Vec::new());
    }
    // Volume slider (dB, -60..+6; double-click resets to 0 dB) and pan knob, on the buttons' row.
    let level_y = if compact { hdr.center().y } else { mid + 10.0 };
    // Logic's slider: a dark pill with a grey thumb that stays inside it.
    let sl = Rect::from_min_size(
        pos2(hdr.right() - 118.0, level_y - SLIDER_H / 2.0),
        vec2(SLIDER_W, SLIDER_H),
    );
    let travel = (sl.left() + SLIDER_H / 2.0, sl.right() - SLIDER_H / 2.0);
    let volume = ui.interact(
        sl.expand2(vec2(4.0, 6.0)),
        id(("vol", i)),
        Sense::click_and_drag(),
    );
    if let Some(pt) = volume.interact_pointer_pos() {
        let f = (pt.x - travel.0) / (travel.1 - travel.0);
        t.volume_db = slider_to_db(f.clamp(0.0, 1.0));
    }
    if volume.double_clicked() {
        t.volume_db = 0.0;
    }
    g.rect_filled(sl, SLIDER_H / 2.0, Color32::from_rgb(55, 54, 54));
    // Level meter inside the pill, as in Logic: two thin bars (L over R) that follow the
    // slider's own scale, green, yellow from -6 dB and red above 0 dB.
    let silent = t.mute || silenced;
    for (k, peak) in [peaks.0, peaks.1].into_iter().enumerate() {
        let db = crate::mixer::lin_to_db(peak);
        if db <= -60.0 {
            continue;
        }
        let y = sl.center().y - 3.0 + k as f32 * 3.5;
        let x0 = sl.left() + 3.0;
        let x_at = |db: f32| x0 + (sl.right() - 3.0 - x0) * db_to_slider(db);
        let mut from = -60.0f32;
        for edge in [-6.0f32, 0.0, 6.0] {
            let to = db.min(edge);
            if to > from {
                let r = Rect::from_min_max(pos2(x_at(from), y), pos2(x_at(to), y + 2.5));
                let color = if silent {
                    Color32::from_gray(0x70)
                } else {
                    crate::mixer::meter_colour(to - 0.01)
                };
                g.rect_filled(r, 1.0, color);
            }
            from = edge;
        }
    }
    let knob = pos2(
        travel.0 + (travel.1 - travel.0) * db_to_slider(t.volume_db),
        sl.center().y,
    );
    g.circle_filled(knob, SLIDER_H / 2.0 - 1.0, Color32::from_rgb(136, 134, 135));
    g.circle_stroke(
        knob,
        SLIDER_H / 2.0 - 1.0,
        Stroke::new(1.0, Color32::from_rgb(98, 97, 97)),
    );

    let kc = pos2(hdr.right() - 22.0, level_y);
    let kresp = ui.interact(
        Rect::from_center_size(kc, vec2(22.0, 22.0)),
        id(("pan", i)),
        Sense::click_and_drag(),
    );
    if kresp.dragged() {
        t.pan = (t.pan - kresp.drag_delta().y * 0.01).clamp(-1.0, 1.0);
    }
    if kresp.double_clicked() {
        t.pan = 0.0;
    }
    crate::widgets::draw_knob(g, kc, 9.0, t.pan);
    (resp, vec![volume, kresp])
}

// ------------------------------------------------------------------ main view

pub fn show(ui: &mut Ui, p: &mut Project, s: &mut UiState, meters: &crate::mixer::Meters) {
    let rec_targets: Vec<(usize, usize)> = s
        .rec
        .target()
        .into_iter()
        .chain(s.audio_rec.regions())
        .collect();
    let area = ui.available_rect_before_wrap();
    ui.allocate_rect(area, Sense::hover());
    let tb = Rect::from_min_size(area.min, vec2(area.width(), TOOLBAR_H));
    let ruler_y = tb.bottom();
    // Global tracks between the ruler and the tracks: Arrangement, Chords, Marker, Signature, Tempo.
    let arrangement_h = if s.tracks.show_arrangement {
        crate::arrangement_track::HEIGHT
    } else {
        0.0
    };
    let chord_h = if s.tracks.show_chords {
        crate::chord_track::HEIGHT
    } else {
        0.0
    };
    let marker_h = if s.tracks.show_markers {
        crate::marker_track::HEIGHT
    } else {
        0.0
    };
    let tempo_h = if s.tracks.show_tempo {
        crate::tempo_track::HEIGHT
    } else {
        0.0
    };
    let signature_h = if s.tracks.show_signatures {
        crate::signature_track::HEIGHT
    } else {
        0.0
    };
    let ruler_h = if s.tracks.show_ruler { RULER_H } else { 0.0 };
    let header_w = if s.tracks.show_header {
        HEADER_W
    } else {
        NARROW_HEADER_W
    };
    let globals_y = ruler_y + ruler_h + arrangement_h + chord_h;
    let tracks_y = globals_y + marker_h + signature_h + tempo_h;
    let headers = Rect::from_min_max(
        pos2(area.left(), tracks_y),
        pos2(area.left() + header_w, area.bottom()),
    );
    let lanes = Rect::from_min_max(pos2(area.left() + header_w, tracks_y), area.max);
    let ruler = Rect::from_min_max(
        pos2(lanes.left(), ruler_y),
        pos2(area.right(), ruler_y + ruler_h),
    );
    let corner = Rect::from_min_max(
        pos2(area.left(), ruler_y),
        pos2(headers.right(), ruler.bottom()),
    );

    toolbar(ui, p, s, tb, lanes.width());

    // Zoom is counted in pixels per bar of the first bar's length.
    let bpb = p.bar_len(0);
    let v = &mut s.tracks;
    v.lanes_w = lanes.width();
    // A selected region brings its clip group with it.
    let grouped: Vec<u32> = v
        .all_selected()
        .iter()
        .filter_map(|&(t, r)| p.tracks.get(t)?.regions.get(r))
        .map(|r| r.params.group)
        .filter(|g| *g != 0)
        .collect();
    for group in grouped {
        for member in p.regions_in_group(group) {
            if !v.is_selected(member) {
                v.also.push(member);
            }
        }
    }

    // ---- wheel: along the song; over the headers, or with Shift or Alt, through the tracks;
    // Ctrl/Cmd = horizontal zoom around the pointer
    let hover = ui.input(|i| i.pointer.hover_pos());
    if hover.is_some_and(|pt| lanes.contains(pt) || headers.contains(pt) || ruler.contains(pt)) {
        let (delta, mods) = ui.input(|i| (i.smooth_scroll_delta, i.modifiers));
        if mods.command && delta.y != 0.0 {
            let pointer_x = hover.map_or(lanes.left(), |p| p.x);
            let beat_at = (pointer_x - lanes.left() + v.scroll_x) / (v.bar_w / bpb as f32);
            v.bar_w = (v.bar_w * (1.0 + delta.y * 0.003)).clamp(MIN_BAR_W, MAX_BAR_W);
            v.scroll_x = beat_at * (v.bar_w / bpb as f32) - (pointer_x - lanes.left());
        } else if mods.shift || mods.alt || hover.is_some_and(|pt| headers.contains(pt)) {
            // Up and down through the tracks: over their headers, or when asked for.
            v.scroll_y -= delta.y + delta.x;
        } else {
            // Along the song. The tracks stay where they are.
            v.scroll_x -= delta.y + delta.x;
        }
    }
    let rows = row_layout(p, v.row_h);
    let content_h = rows.last().map_or(0.0, |(top, h)| top + h);
    v.scroll_y = v
        .scroll_y
        .clamp(0.0, (content_h - lanes.height() + 40.0).max(0.0));
    v.scroll_x = v.scroll_x.clamp(0.0, 1000.0 * v.bar_w);

    let beat_px = v.bar_w / bpb as f32;
    // Catch: page the view when the playhead leaves it.
    if v.catch && p.playing {
        let px = p.position_beats as f32 * beat_px - v.scroll_x;
        if px > lanes.width() - 30.0 || px < 0.0 {
            v.scroll_x = (p.position_beats as f32 * beat_px - lanes.width() * 0.1).max(0.0);
        }
    }
    let (scroll_x, scroll_y) = (v.scroll_x, v.scroll_y);
    let x_of = |beat: f64| lanes.left() + beat as f32 * beat_px - scroll_x;
    let beat_of = |x: f32| ((x - lanes.left() + scroll_x) / beat_px).max(0.0) as f64;
    if v.tool == TrackTool::Scrub {
        let (down, origin, at) = ui.input(|i| {
            (
                i.pointer.primary_down(),
                i.pointer.press_origin(),
                i.pointer.latest_pos(),
            )
        });
        let held = down && origin.is_some_and(|o| lanes.contains(o));
        match at.filter(|_| held) {
            Some(at) => {
                let due = v.scrub_at.is_none_or(|(x, when)| {
                    (x - at.x).abs() >= 1.0
                        && when.elapsed() >= std::time::Duration::from_millis(30)
                });
                if due {
                    let beat = beat_of(at.x);
                    let row = origin.and_then(|o| row_at(&rows, o.y - lanes.top() + scroll_y));
                    v.scrub = Some((row.unwrap_or(p.selected_track), beat));
                    v.scrub_at = Some((at.x, std::time::Instant::now()));
                    if !p.playing {
                        p.position_beats = beat;
                    }
                }
            }
            None => v.scrub_at = None,
        }
    }

    // ---- lanes background + grid
    let g = ui.painter_at(lanes);
    g.rect_filled(lanes, 0.0, LANE_BG);
    let bars = p.bars_in(beat_of(lanes.left()), beat_of(lanes.right()));
    for (i, &(top, row_h)) in rows.iter().enumerate() {
        if row_h <= 0.0 {
            continue;
        }
        let y = lanes.top() + top - scroll_y;
        let row = Rect::from_min_size(pos2(lanes.left(), y), vec2(lanes.width(), row_h));
        if p.selected_track == i {
            g.rect_filled(row, 0.0, LANE_SEL);
        }
        if p.tracks[i].kind == TrackKind::Folder {
            g.rect_filled(row, 0.0, Color32::from_white_alpha(10));
        }
        g.line_segment(
            [row.left_bottom(), row.right_bottom()],
            Stroke::new(1.0, crate::theme::BORDER),
        );
    }
    let grid_bars: &[_] = if v.show_grid { &bars } else { &[] };
    for bar in grid_bars {
        let x = x_of(bar.start);
        g.line_segment(
            [pos2(x, lanes.top()), pos2(x, lanes.bottom())],
            Stroke::new(1.0, Color32::from_gray(0x38)),
        );
        if beat_px * bar.unit as f32 >= 14.0 {
            for k in 1..bar.num {
                let xb = x_of(bar.start + k as f64 * bar.unit);
                g.line_segment(
                    [pos2(xb, lanes.top()), pos2(xb, lanes.bottom())],
                    Stroke::new(1.0, Color32::from_gray(0x30)),
                );
            }
        }
    }

    // Click on an empty lane: select that track, deselect regions.
    let lane_resp = ui.interact(lanes, id("lanes"), Sense::click_and_drag());
    v.tidy_selection(p);
    if lane_resp.drag_started() {
        let origin = ui.input(|i| i.pointer.press_origin());
        if let Some(pt) = origin.or(lane_resp.interact_pointer_pos()) {
            v.marquee = Some((beat_of(pt.x), pt.y - lanes.top() + scroll_y));
            v.track_focus = false;
        }
    }
    let mut pencil_at: Option<(usize, f64)> = None;
    let mut picked: Option<CtxAction> = None;
    if lane_resp.clicked() || context_click(&lane_resp) {
        v.auto_pick = None;
        if let Some(pt) = lane_resp.interact_pointer_pos() {
            if let Some(row) = row_at(&rows, pt.y - lanes.top() + scroll_y) {
                p.selected_track = row;
                if lane_resp.clicked() && v.tool == TrackTool::Pencil {
                    pencil_at = Some((row, beat_of(pt.x)));
                }
            }
        }
        if lane_resp.clicked() && v.tool == TrackTool::Zoom {
            v.bar_w = (v.bar_w * 1.5).min(600.0);
        }
        v.track_focus = false;
        v.selected = None;
    }
    // Double-click on an empty stretch of a Drummer track: a Drummer region there, as in Logic.
    if lane_resp.double_clicked() {
        if let Some(pt) = lane_resp.interact_pointer_pos() {
            let drummer_row = row_at(&rows, pt.y - lanes.top() + scroll_y)
                .filter(|row| p.tracks[*row].kind == TrackKind::Drummer);
            if let Some(row) = drummer_row {
                let bar = p.beats_to_bar(beat_of(pt.x)).floor() as f32 + 1.0;
                if let Some(ri) = p.add_generated_region(row, true, bar) {
                    p.selected_track = row;
                    v.selected = Some((row, ri));
                }
            }
            // On an instrument track: an empty region, ready to draw in.
            let midi_row = row_at(&rows, pt.y - lanes.top() + scroll_y)
                .filter(|row| matches!(p.tracks[*row].kind, TrackKind::Instrument));
            if let Some(row) = midi_row {
                let bar = p.beats_to_bar(beat_of(pt.x)).floor() as f32 + 1.0;
                if let Some(ri) = p.add_midi_region(row, bar) {
                    p.selected_track = row;
                    v.selected = Some((row, ri));
                    s.bottom = Bottom::PianoRoll;
                }
            }
        }
    }
    // A sample dragged from the Browser: a line where it will start (the bar under the pointer),
    // and when it is let go over the lanes the user is asked what it should become (a MIDI file
    // is imported as tracks).
    if let Some(drag) = egui::DragAndDrop::payload::<crate::sample_browser::SampleDrag>(ui.ctx()) {
        let over = ui
            .input(|i| i.pointer.hover_pos())
            .filter(|pt| lanes.contains(*pt));
        if let Some(pt) = over {
            let bar0 = p.beats_to_bar(beat_of(pt.x)).floor();
            let x = x_of(p.bar_to_beats(bar0));
            g.line_segment(
                [pos2(x, lanes.top()), pos2(x, lanes.bottom())],
                Stroke::new(2.0, crate::widgets::BLUE),
            );
            if ui.input(|i| i.pointer.any_released()) {
                egui::DragAndDrop::clear_payload(ui.ctx());
                let path = &drag.0;
                if crate::midi_file::is_midi_path(path) {
                    s.open_paths.push(path.clone());
                } else {
                    // An audio clip, or something to play? The user says.
                    s.sample_drop = Some(crate::sample_place::SampleDrop {
                        path: path.clone(),
                        at: pt,
                        bar: bar0 + 1.0,
                        track: row_at(&rows, pt.y - lanes.top() + scroll_y),
                    });
                }
            }
        }
    }
    // A pattern dragged from the Channel Rack: placed at the bar it is let go on.
    if let Some(drag) = egui::DragAndDrop::payload::<crate::channel_rack::PatternDrag>(ui.ctx()) {
        let over = ui
            .input(|i| i.pointer.hover_pos())
            .filter(|pt| lanes.contains(*pt));
        if let Some(pt) = over {
            let bar0 = p.beats_to_bar(beat_of(pt.x)).floor();
            let x = x_of(p.bar_to_beats(bar0));
            g.line_segment(
                [pos2(x, lanes.top()), pos2(x, lanes.bottom())],
                Stroke::new(2.0, crate::widgets::BLUE),
            );
            if ui.input(|i| i.pointer.any_released()) {
                egui::DragAndDrop::clear_payload(ui.ctx());
                if crate::song_pattern::place(p, drag.0, bar0 as f32 + 1.0) == 0 {
                    s.status = Some("This pattern has no notes yet".to_string());
                }
            }
        }
    }
    // A plugin dragged from the Plugins tab: the track it will go on, and it goes there when let go.
    if let Some(drag) = egui::DragAndDrop::payload::<crate::left_panel::PluginDrag>(ui.ctx()) {
        let over = ui
            .input(|i| i.pointer.hover_pos())
            .filter(|pt| area.contains(*pt) && pt.y >= lanes.top());
        if let Some(pt) = over {
            let row = row_at(&rows, pt.y - lanes.top() + scroll_y);
            if let Some(&(top, h)) = row.and_then(|t| rows.get(t)) {
                let y = lanes.top() + top - scroll_y;
                g.rect_filled(
                    Rect::from_min_size(pos2(lanes.left(), y), vec2(lanes.width(), h)),
                    0.0,
                    crate::widgets::BLUE.gamma_multiply(0.25),
                );
            }
            if ui.input(|i| i.pointer.any_released()) {
                egui::DragAndDrop::clear_payload(ui.ctx());
                let added = match row {
                    Some(t) => crate::plugin_rack::add_plugin(p, t, &drag.0),
                    None => Err("Drop a plugin on a track".to_string()),
                };
                if let Err(e) = added {
                    s.status = Some(e);
                }
            }
        }
    }
    if context_click(&lane_resp) {
        v.lane_menu_row = lane_resp
            .interact_pointer_pos()
            .and_then(|pt| row_at(&rows, pt.y - lanes.top() + scroll_y));
    }
    let lane_track = v.lane_menu_row.and_then(|i| track_info(p, i));
    let can_paste = !v.clipboard.is_empty();
    context_menu(&lane_resp, |ui| {
        if let Some(info) = &lane_track {
            track_action_items(ui, info, &mut picked);
            ui.separator();
            if info.takes_midi {
                pick(
                    ui,
                    "New MIDI Region",
                    "",
                    CtxAction::NewRegion(info.i),
                    &mut picked,
                );
            }
        }
        if ui
            .add_enabled(can_paste, egui::Button::new(tr("Paste")))
            .clicked()
        {
            picked = Some(CtxAction::Edit(EditCmd::Paste));
            ui.close();
        }
        ui.separator();
        new_track_items(ui, &mut picked);
        ui.separator();
        if ui.button(tr("Zoom to Fit")).clicked() {
            picked = Some(CtxAction::ZoomToFit);
        }
    });

    // ---- regions
    let step = snap_step(v.snap, beat_px, bpb, p.beat_unit_at(beat_of(lanes.left())));
    // Snapping to bars follows the signature, so it goes to the nearest bar line.
    let snap_bars = snaps_to_bars(v.snap, beat_px, p.beat_unit_at(beat_of(lanes.left())));
    let sig_owned = p.sigs().to_owned();
    let snap_pos = move |b: f64| -> f64 {
        if snap_bars {
            sig_owned.as_ref().nearest_bar_line(b)
        } else {
            snap_to(b, step)
        }
    };
    let mut moved = false;
    // A region dragged: the one grabbed and where the pointer is, and what to do when it is dropped.
    let mut dragged_over: Option<(usize, f32)> = None;
    let mut track_drop: Option<TrackDrop> = None;
    // The clip gain handle on show: where, and the gain it stands for.
    let mut gain_handle: Option<(Rect, f32, bool)> = None;
    // What the Scissors or Glue tool was clicked on, done once the regions have been drawn.
    let mut region_click: Option<(usize, usize, TrackTool, f64)> = None;
    for (ti, &(top, row_h)) in rows.iter().enumerate() {
        if row_h <= 0.0 {
            continue;
        }
        let y = lanes.top() + top - scroll_y;
        if y + row_h < lanes.top() || y > lanes.bottom() {
            continue;
        }
        let base_h = row_h - take_rows_height(&p.tracks[ti]);
        for ri in 0..p.tracks[ti].regions.len() {
            let start_beat = p.region_start_beats(&p.tracks[ti].regions[ri]);
            let len_beats = p.region_len_beats(&p.tracks[ti].regions[ri]);
            let full = Rect::from_min_max(
                pos2(x_of(start_beat), y + 1.0),
                pos2(x_of(start_beat + len_beats), y + base_h - 1.0),
            );
            let rr = full.intersect(lanes);
            if rr.width() <= 0.0 || rr.height() <= 0.0 {
                continue;
            }
            // With the Marquee tool regions take no drags, so one that starts on them selects.
            let sense = if v.tool == TrackTool::Marquee {
                Sense::hover()
            } else {
                Sense::click_and_drag()
            };
            let resp = ui.interact(rr, id(("region", ti, ri)), sense);
            let active = if ui.input(|i| i.modifiers.command) && v.tool2 != TrackTool::Pointer {
                v.tool2
            } else {
                v.tool
            };
            if resp.clicked() || resp.drag_started() || context_click(&resp) {
                v.auto_pick = None;
                let ctrl = ui.input(|i| i.modifiers.command);
                let here = (ti, ri);
                if context_click(&resp) || resp.drag_started() {
                    // On the group, or dragging: the group stays.
                    if !v.is_selected(here) {
                        if ctrl {
                            v.toggle(here);
                        } else {
                            v.select_only(here);
                        }
                    }
                } else if ctrl {
                    v.toggle(here);
                } else {
                    v.select_only(here);
                }
                v.track_focus = false;
                p.selected_track = ti;
            }
            if resp.double_clicked() && p.tracks[ti].kind != TrackKind::Audio {
                s.bottom = Bottom::PianoRoll;
            }
            let fade_tool = active == TrackTool::Fade && p.tracks[ti].kind == TrackKind::Audio;
            if fade_tool && resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            let click_tool = active.only_clicks();
            if click_tool && resp.clicked() {
                if let Some(pt) = resp.interact_pointer_pos() {
                    region_click = Some((ti, ri, active, snap_pos(beat_of(pt.x))));
                }
            }
            if click_tool {
                // It only clicks: nothing moves.
            } else if fade_tool {
                // The Fade tool: the left half of the region gets a fade in, the right half a
                // fade out, as long as the pointer is dragged (in 1/16 notes, as the Inspector).
                if resp.drag_started() {
                    if let Some(pt) = resp.interact_pointer_pos() {
                        v.fade_drag = Some(beat_of(pt.x) < start_beat + len_beats / 2.0);
                    }
                }
                if let (true, Some(fade_in), Some(pt)) =
                    (resp.dragged(), v.fade_drag, resp.interact_pointer_pos())
                {
                    let beats = fade_for_drag(beat_of(pt.x), start_beat, len_beats, fade_in);
                    let params = &mut p.tracks[ti].regions[ri].params;
                    if fade_in {
                        params.fade_in = beats;
                    } else {
                        params.fade_out = beats;
                    }
                }
                if resp.drag_stopped() {
                    v.fade_drag = None;
                    moved = true;
                }
            } else {
                // Near an edge a drag trims the region, anywhere else it moves it.
                let trimmable =
                    p.tracks[ti].regions[ri].folder.is_none() && full.width() > TRIM_EDGE * 3.0;
                let hover_edge = resp.hover_pos().filter(|_| trimmable).is_some_and(|pt| {
                    pt.x - full.left() < TRIM_EDGE || full.right() - pt.x < TRIM_EDGE
                });
                if hover_edge || (v.trim.is_some() && resp.dragged()) {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
                if resp.drag_started() {
                    let origin = ui.input(|i| i.pointer.press_origin());
                    if let Some(pt) = origin.or(resp.interact_pointer_pos()) {
                        v.grab_beats = beat_of(pt.x) - start_beat;
                        v.group_drag = v
                            .all_selected()
                            .into_iter()
                            .filter_map(|(t, r)| {
                                let region = p.tracks.get(t)?.regions.get(r)?;
                                Some((t, r, p.region_start_beats(region)))
                            })
                            .collect();
                        v.trim = if !trimmable {
                            None
                        } else if full.right() - pt.x < TRIM_EDGE {
                            Some(TrimDrag::End)
                        } else if pt.x - full.left() < TRIM_EDGE {
                            crate::region_edit::TrimStart::of(p, ti, ri)
                                .ok()
                                .map(TrimDrag::Start)
                        } else {
                            None
                        };
                    }
                }
                if resp.dragged() {
                    if let Some(pt) = resp.interact_pointer_pos() {
                        let at = snap_pos(beat_of(pt.x).max(0.0));
                        match &v.trim {
                            Some(TrimDrag::End) => {
                                let at = crate::region_edit::zero_crossing_beat(p, ti, ri, at);
                                let _ = crate::region_edit::trim_end(p, ti, ri, at);
                            }
                            Some(TrimDrag::Start(from)) => {
                                let at = crate::region_edit::zero_crossing_beat(p, ti, ri, at);
                                crate::region_edit::trim_start(p, ti, ri, from, at);
                            }
                            None => {
                                dragged_over = Some((ti, pt.y));
                                // All of them by what this one moved, none before the start.
                                let nb = snap_pos((beat_of(pt.x) - v.grab_beats).max(0.0));
                                let was = v
                                    .group_drag
                                    .iter()
                                    .find(|(t, r, _)| (*t, *r) == (ti, ri))
                                    .map_or(start_beat, |g| g.2);
                                let earliest = v
                                    .group_drag
                                    .iter()
                                    .map(|g| g.2)
                                    .fold(f64::INFINITY, f64::min);
                                let by = (nb - was).max(-earliest);
                                for &(t, r, from) in &v.group_drag {
                                    let bar = p.beats_to_bar(from + by) as f32 + 1.0;
                                    if let Some(region) =
                                        p.tracks.get_mut(t).and_then(|x| x.regions.get_mut(r))
                                    {
                                        region.start_bar = bar;
                                    }
                                }
                            }
                        }
                    }
                }
                if resp.drag_stopped() {
                    moved = true;
                    let was_moving = v.trim.is_none();
                    v.trim = None;
                    let group: Vec<(usize, usize)> =
                        v.group_drag.drain(..).map(|g| (g.0, g.1)).collect();
                    let dropped_at = ui.input(|i| i.pointer.latest_pos());
                    if let (true, Some(pt)) = (was_moving, dropped_at) {
                        let y = pt.y - lanes.top() + scroll_y;
                        let row = drop_row(&rows, y);
                        if let Some(row) =
                            row.filter(|r| *r != crate::region_edit::DropRow::Track(ti))
                        {
                            let regions = if group.is_empty() {
                                vec![(ti, ri)]
                            } else {
                                group
                            };
                            track_drop = Some((regions, (ti, ri), row));
                        }
                    }
                    // Dropped on another audio region: X-Fade crossfades them, the other
                    // modes leave them overlapping.
                    if p.tracks[ti].kind == TrackKind::Audio && overlaps_another(p, ti, ri) {
                        p.tracks[ti].regions[ri].params.crossfade = v.drag_mode == DragMode::XFade;
                    }
                }
            }
            let (is_audio, muted) = (
                p.tracks[ti].kind == TrackKind::Audio,
                p.tracks[ti].regions[ri].params.mute,
            );
            let folder_open = p.tracks[ti].regions[ri].folder.as_ref().map(|f| f.open);
            let generated = p.tracks[ti].regions[ri].generated.is_some();
            let linked = p.tracks[ti].regions[ri].params.pattern != 0;
            let shown = v.shot_menu == Some((ti, Some(ri)));
            crate::widgets::context_menu_shown(&resp, shown, |ui| {
                region_items(
                    ui,
                    is_audio,
                    muted,
                    folder_open,
                    generated,
                    linked,
                    ti,
                    ri,
                    &mut picked,
                );
            });
            let selected = v.is_selected((ti, ri));
            if folder_open.is_some() {
                // The triangle opens and closes the folder; the rows of an opened one are
                // swiped to pick which take plays where.
                let tri = ui.interact(
                    folder_toggle_rect(full).intersect(lanes),
                    id(("take_toggle", ti, ri)),
                    Sense::click(),
                );
                if tri.clicked() {
                    picked = Some(CtxAction::ToggleTakeFolder(ti, ri));
                    v.selected = Some((ti, ri));
                }
                if folder_open == Some(true) {
                    let n = p.tracks[ti].regions[ri]
                        .folder
                        .as_ref()
                        .map_or(0, |f| f.takes.len());
                    for take in 0..n {
                        let row = take_row_rect(full, n, take).intersect(lanes);
                        if row.width() <= 0.0 || row.height() <= 0.0 {
                            continue;
                        }
                        let resp = ui.interact(
                            row,
                            id(("take_row", ti, ri, take)),
                            Sense::click_and_drag(),
                        );
                        let Some(region) = p.tracks[ti].regions.get_mut(ri) else {
                            continue;
                        };
                        let Some(folder) = region.folder.as_mut() else {
                            continue;
                        };
                        let grab =
                            |pt: egui::Pos2| snap_pos((beat_of(pt.x) - start_beat).max(0.0)) as f32;
                        if resp.drag_started() {
                            if let Some(pt) = resp.interact_pointer_pos() {
                                v.swipe = Some((folder.comp.clone(), grab(pt)));
                            }
                        }
                        if resp.dragged() {
                            if let (Some((orig, anchor)), Some(pt)) =
                                (v.swipe.as_ref(), resp.interact_pointer_pos())
                            {
                                folder.comp = orig.clone();
                                folder.swipe(take, *anchor, grab(pt));
                                folder.apply(&mut region.notes, &mut region.controls);
                            }
                        }
                        if resp.drag_stopped() {
                            v.swipe = None;
                            moved = true;
                        }
                        if resp.clicked() {
                            // A click picks the whole take.
                            let len = folder.len;
                            folder.swipe(take, 0.0, len);
                            folder.apply(&mut region.notes, &mut region.controls);
                            moved = true;
                        }
                        context_menu(&resp, |ui| {
                            pick(
                                ui,
                                tr("Delete Take"),
                                "",
                                CtxAction::DeleteTake(ti, ri, take),
                                &mut picked,
                            );
                        });
                    }
                }
                if let Some(folder) = p.tracks[ti].regions[ri].folder.as_ref() {
                    draw_take_folder(
                        &g,
                        full,
                        &p.tracks[ti],
                        &p.tracks[ti].regions[ri],
                        folder,
                        beat_px,
                        60.0 / p.tempo_at(start_beat).max(1.0),
                        selected,
                        silenced_by_solo(p, ti),
                        v.show_colors,
                        v.wave_zoom,
                    );
                    continue;
                }
            }
            // The clip gain handle, on an audio region under the pointer.
            if p.tracks[ti].regions[ri].audio.is_some()
                && v.tool == TrackTool::Pointer
                && rr.width() > 70.0
                && rr.height() > 24.0
            {
                let handle =
                    Rect::from_center_size(pos2(rr.center().x, rr.top() + 8.0), vec2(58.0, 14.0));
                let h = ui
                    .interact(handle, id(("clip_gain", ti, ri)), Sense::click_and_drag())
                    .on_hover_text("Clip gain: drag up or down. Double-click for 0 dB.");
                let gain = &mut p.tracks[ti].regions[ri].params.gain_db;
                if h.dragged() {
                    let by = -h.drag_delta().y / GAIN_DRAG_PX_PER_DB;
                    *gain = (*gain + by)
                        .clamp(crate::model::MIN_REGION_GAIN, crate::model::MAX_REGION_GAIN);
                    moved = true;
                }
                if h.double_clicked() {
                    *gain = 0.0;
                    moved = true;
                }
                if resp.hovered() || h.hovered() || h.dragged() {
                    gain_handle = Some((handle, *gain, h.hovered() || h.dragged()));
                }
            }
            let tr = &p.tracks[ti];
            draw_region(
                &g,
                rr,
                full,
                tr,
                &tr.regions[ri],
                p.effective_fades(ti, ri),
                beat_px,
                selected,
                rec_targets.contains(&(ti, ri)),
                silenced_by_solo(p, ti),
                s.audio_rec.live_wave(p, ti, ri),
                v.show_colors,
                v.wave_zoom,
            );
            crate::automation::draw_region_lanes(&g, full, &tr.regions[ri], beat_px);
            if v.flex_view {
                flex_markers(ui, &g, p, v, (ti, ri), rr, full, &resp);
            }
        }
    }
    // The curves of the tracks that show theirs, on top of the regions: registered after them, so
    // the row's body edits the curve and the name bars still move the regions.
    let tool_on_lane = matches!(v.tool, TrackTool::Pointer | TrackTool::Automation)
        && !(ui.input(|i| i.modifiers.command) && v.tool2 != TrackTool::Pointer);
    for (ti, &(top, row_h)) in rows.iter().enumerate() {
        if row_h <= 0.0 || !p.tracks[ti].show_auto || p.tracks[ti].kind == TrackKind::Folder {
            continue;
        }
        let y = lanes.top() + top - scroll_y;
        let base_h = row_h - take_rows_height(&p.tracks[ti]);
        if y + base_h < lanes.top() || y > lanes.bottom() {
            continue;
        }
        let view = crate::automation::LaneView {
            rect: Rect::from_min_max(
                pos2(lanes.left(), y + AUTO_NAME_BAR.min(base_h * 0.3)),
                pos2(lanes.right(), y + base_h - 3.0),
            ),
            x_of: &x_of,
            beat_of: &beat_of,
            snap: &snap_pos,
            active: tool_on_lane,
        };
        let pressed =
            crate::automation::track_lane(ui, &g, p, ti, &view, &mut v.auto_pick, &mut v.auto_drag);
        if pressed {
            v.select_many(Vec::new());
            v.track_focus = false;
        }
    }
    if moved {
        p.song_rev += 1;
    }
    if let Some((r, db, active)) = gain_handle {
        let fill = if active {
            crate::theme::ACCENT_BLUE
        } else {
            Color32::from_black_alpha(150)
        };
        g.rect_filled(r, 3.0, fill);
        g.text(
            r.center(),
            Align2::CENTER_CENTER,
            format!("{db:+.1} dB"),
            FontId::proportional(10.5),
            Color32::WHITE,
        );
    }
    // Where a region being dragged up or down would land.
    if let Some((from, y)) = dragged_over {
        use crate::region_edit::DropRow;
        match drop_row(&rows, y - lanes.top() + scroll_y) {
            Some(DropRow::Track(t)) if t != from => {
                let (top, h) = rows[t];
                let r = Rect::from_min_size(
                    pos2(lanes.left(), lanes.top() + top - scroll_y),
                    vec2(lanes.width(), h),
                );
                g.rect_filled(
                    r.intersect(lanes),
                    0.0,
                    crate::theme::ACCENT_BLUE.gamma_multiply(0.18),
                );
            }
            Some(DropRow::Below) => {
                let bottom = rows.iter().map(|r| r.0 + r.1).fold(0.0, f32::max);
                let top = lanes.top() + bottom - scroll_y;
                let r = Rect::from_min_size(pos2(lanes.left(), top), vec2(lanes.width(), 40.0));
                g.rect_filled(
                    r.intersect(lanes),
                    0.0,
                    crate::theme::ACCENT_BLUE.gamma_multiply(0.18),
                );
                g.text(
                    r.left_center() + vec2(8.0, 0.0),
                    Align2::LEFT_CENTER,
                    "New track",
                    FontId::proportional(11.5),
                    TEXT,
                );
            }
            _ => {}
        }
    }
    if let Some((regions, lead, row)) = track_drop {
        crate::region_edit::move_to_track(p, v, &regions, lead, row);
    }
    // The box over the lanes: everything it touches is selected, and Ctrl keeps what already was.
    if let (Some((beat0, y0)), Some(pt)) = (v.marquee, lane_resp.interact_pointer_pos()) {
        let (beat1, y1) = (beat_of(pt.x), pt.y - lanes.top() + scroll_y);
        let (b0, b1) = (beat0.min(beat1), beat0.max(beat1));
        let (top, bottom) = (y0.min(y1), y0.max(y1));
        let mut inside = Vec::new();
        for (t, &(row_top, row_h)) in rows.iter().enumerate() {
            if row_h > 0.0 && row_top < bottom && row_top + row_h > top {
                for (r, region) in p.tracks[t].regions.iter().enumerate() {
                    if p.region_start_beats(region) < b1 && p.region_end_beats(region) > b0 {
                        inside.push((t, r));
                    }
                }
            }
        }
        v.select_many(inside);
        let screen = |beat: f64, y: f32| pos2(x_of(beat), y + lanes.top() - scroll_y);
        let boxed = Rect::from_two_pos(screen(b0, top), screen(b1, bottom));
        g.rect_filled(boxed, 0.0, Color32::from_white_alpha(22));
        g.rect_stroke(
            boxed,
            0.0,
            Stroke::new(1.0, Color32::from_white_alpha(150)),
            StrokeKind::Inside,
        );
    }
    if lane_resp.drag_stopped() {
        v.marquee = None;
    }
    if let Some((row, beat)) = pencil_at {
        let kind = p.tracks[row].kind;
        if kind.has_regions() && kind != TrackKind::Audio {
            let bar = p.beats_to_bar(snap_pos(beat)) as f32 + 1.0;
            p.add_midi_region(row, bar);
        }
    }
    if let Some((t, r, tool, beat)) = region_click {
        use crate::region_edit as edit;
        let done = match tool {
            TrackTool::Glue => edit::join_with_next(p, t, r).map(|_| ()),
            TrackTool::Scissors => {
                let cut = edit::zero_crossing_beat(p, t, r, beat);
                edit::split(p, t, r, cut).map(|_| ())
            }
            TrackTool::Eraser => {
                p.tracks[t].regions.remove(r);
                p.song_rev += 1;
                Ok(())
            }
            TrackTool::Mute => {
                let params = &mut p.tracks[t].regions[r].params;
                params.mute = !params.mute;
                p.song_rev += 1;
                Ok(())
            }
            TrackTool::Solo => {
                let on = !p.tracks[t].solo;
                p.set_solo(t, on);
                Ok(())
            }
            TrackTool::Text => {
                s.dialogs
                    .open_rename_region(t, r, &p.tracks[t].regions[r].name);
                Ok(())
            }
            TrackTool::Zoom => {
                v.bar_w = (v.bar_w * 1.5).min(600.0);
                Ok(())
            }
            TrackTool::Automation => {
                let ed = &mut s.mixer.auto_editor;
                ed.track = Some(t);
                ed.region = Some(r);
                Ok(())
            }
            TrackTool::Flex => {
                if p.tracks[t].regions[r].audio.is_some() {
                    crate::flex_edit::set_mode(p, t, r, prowave_engine::FlexMode::default());
                    p.song_rev += 1;
                    Ok(())
                } else {
                    Err("Flex is for audio regions".to_string())
                }
            }
            TrackTool::Pointer
            | TrackTool::Fade
            | TrackTool::Pencil
            | TrackTool::Marquee
            | TrackTool::Scrub => Ok(()),
        };
        let keeps_selection = matches!(
            tool,
            TrackTool::Solo
                | TrackTool::Text
                | TrackTool::Zoom
                | TrackTool::Automation
                | TrackTool::Mute
                | TrackTool::Flex
        );
        match done {
            Ok(()) if !keeps_selection => {
                v.selected = None;
                v.also.clear();
            }
            Ok(()) => {}
            Err(e) => s.status = Some(e),
        }
    }

    if arrangement_h > 0.0 {
        let top = ruler_y + ruler_h;
        let done = crate::arrangement_track::show(
            ui,
            p,
            Rect::from_min_max(
                pos2(area.left(), top),
                pos2(headers.right(), top + arrangement_h),
            ),
            Rect::from_min_max(
                pos2(lanes.left(), top),
                pos2(area.right(), top + arrangement_h),
            ),
            x_of,
            beat_of,
        );
        match done {
            Some(Ok(())) => {
                // The region numbers now mean other regions.
                v.select_many(Vec::new());
                v.auto_pick = None;
                s.piano.selected.clear();
            }
            Some(Err(e)) => s.status = Some(e),
            None => {}
        }
    }
    if chord_h > 0.0 {
        let top = ruler_y + ruler_h + arrangement_h;
        crate::chord_track::show(
            ui,
            p,
            Rect::from_min_max(pos2(area.left(), top), pos2(headers.right(), top + chord_h)),
            Rect::from_min_max(pos2(lanes.left(), top), pos2(area.right(), top + chord_h)),
            x_of,
            beat_of,
            &snap_pos,
        );
    }
    if marker_h > 0.0 {
        let top = globals_y;
        crate::marker_track::show(
            ui,
            p,
            Rect::from_min_max(
                pos2(area.left(), top),
                pos2(headers.right(), top + marker_h),
            ),
            Rect::from_min_max(pos2(lanes.left(), top), pos2(area.right(), top + marker_h)),
            x_of,
            beat_of,
            &snap_pos,
        );
    }
    if signature_h > 0.0 {
        let top = globals_y + marker_h;
        crate::signature_track::show(
            ui,
            p,
            Rect::from_min_max(
                pos2(area.left(), top),
                pos2(headers.right(), top + signature_h),
            ),
            Rect::from_min_max(
                pos2(lanes.left(), top),
                pos2(area.right(), top + signature_h),
            ),
            x_of,
            beat_of,
        );
    }
    if tempo_h > 0.0 {
        let top = globals_y + marker_h + signature_h;
        crate::tempo_track::show(
            ui,
            p,
            Rect::from_min_max(pos2(area.left(), top), pos2(headers.right(), tracks_y)),
            Rect::from_min_max(pos2(lanes.left(), top), pos2(area.right(), tracks_y)),
            x_of,
            beat_of,
            &snap_pos,
        );
    }

    if let Some(request) = s.mixer.strip_request.take() {
        use crate::mixer::StripRequest;
        picked = Some(match request {
            StripRequest::PianoRoll(t) => CtxAction::OpenTrackPianoRoll(t),
            StripRequest::Color(t) => CtxAction::TrackColor(t),
            StripRequest::Duplicate(t, regions) => CtxAction::DuplicateTrack(t, regions),
            StripRequest::Delete(t) => CtxAction::DeleteTrack(t),
        });
    }
    if std::mem::take(&mut s.mixer.track_clicked) {
        v.track_focus = true;
        v.selected = None;
    }
    // ---- keyboard: Delete, Cmd/Ctrl+R
    if !ui.ctx().egui_wants_keyboard_input() && !s.piano_focus {
        let del = ui.input(|i| {
            !i.modifiers.command && (i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace))
        });
        if del && !crate::automation::delete_picked(p, &mut v.auto_pick) {
            if v.track_focus && v.selected.is_none() {
                picked = Some(CtxAction::DeleteTrack(p.selected_track));
            } else {
                delete_selected(p, v);
            }
        }
    }

    // ---- headers
    let gh = ui.painter_at(headers);
    gh.rect_filled(headers, 0.0, Color32::from_gray(0x3a));
    // Below the last track too, a right-click can make one.
    let blank = ui.interact(headers, id("headers_blank"), Sense::click());
    context_menu(&blank, |ui| new_track_items(ui, &mut picked));
    for (i, &(top, row_h)) in rows.iter().enumerate() {
        if row_h <= 0.0 {
            continue;
        }
        let y = headers.top() + top - scroll_y;
        if y + row_h < headers.top() || y > headers.bottom() {
            continue;
        }
        let hdr = Rect::from_min_size(pos2(headers.left(), y), vec2(header_w, row_h));
        let opts = if s.tracks.show_header {
            s.tracks.header_opts
        } else {
            HeaderOpts {
                number: true,
                icon: false,
                buttons: false,
                level: false,
            }
        };
        let peaks = meters.tracks.get(i).copied().unwrap_or((0.0, 0.0));
        let silenced = silenced_by_solo(p, i);
        let before = p.selected_track;
        let has_number = opts.number;
        let (resp, controls) = header(ui, &gh, p, i, hdr, peaks, silenced, opts, &mut picked);
        if resp.clicked() {
            s.tracks.track_focus = true;
            s.tracks.selected = None;
            let mods = ui.input(|m| m.modifiers);
            // A plain click on the coloured number is for its colour.
            let at = ui.input(|m| m.pointer.interact_pos());
            let on_number = at.is_some_and(|at| at.x < hdr.left() + NUM_W);
            if has_number && on_number && !mods.command && !mods.shift {
                s.dialogs.color = Some(i);
            }
            pick_track(
                &mut s.tracks.picked_tracks,
                before,
                i,
                mods.command,
                mods.shift,
            );
        }
        // A double click on a track's name renames it. On the number, it is the colour.
        if resp.double_clicked() {
            let at = ui.input(|m| m.pointer.interact_pos());
            if at.is_some_and(|at| at.x >= hdr.left() + NUM_W) {
                if let Some(name) = p.tracks.get(i).map(|t| t.name.clone()) {
                    s.dialogs.open_rename(i, &name);
                }
            }
        }
        if resp.drag_started() {
            s.tracks.track_drag = Some(i);
            if !s.tracks.picked_tracks.contains(&i) {
                s.tracks.picked_tracks.clear();
            }
        }
        if s.tracks.picked_tracks.contains(&i) && p.selected_track != i {
            gh.rect_filled(hdr, 0.0, crate::theme::ACCENT_BLUE.gamma_multiply(0.22));
        }
        let Some(info) = track_info(p, i) else {
            continue;
        };
        let shown = s.tracks.shot_menu == Some((i, None));
        crate::widgets::context_menu_shown(&resp, shown, |ui| track_items(ui, &info, &mut picked));
        // The controls on top of the header have the same menu.
        for control in &controls {
            context_menu(control, |ui| track_items(ui, &info, &mut picked));
        }
    }
    if let Some(action) = picked {
        apply_ctx(action, p, s, lanes.width());
    }
    // A header being dragged: where it would land, and the move when it is let go.
    if let Some(from) = s.tracks.track_drag {
        let (pointer, down) = ui.input(|i| (i.pointer.latest_pos(), i.pointer.primary_down()));
        if let Some(pt) = pointer {
            let y = pt.y - lanes.top() + scroll_y;
            let to = rows
                .iter()
                .position(|(top, h)| *h > 0.0 && y < top + h / 2.0)
                .unwrap_or(rows.len());
            let line_y = rows.get(to).map_or_else(
                || rows.iter().map(|r| r.0 + r.1).fold(0.0, f32::max),
                |r| r.0,
            );
            let line = lanes.top() + line_y - scroll_y;
            gh.line_segment(
                [pos2(headers.left(), line), pos2(headers.right(), line)],
                Stroke::new(2.0, crate::theme::ACCENT_BLUE),
            );
            if !down {
                let moving = if s.tracks.picked_tracks.contains(&from) {
                    s.tracks.picked_tracks.clone()
                } else {
                    vec![from]
                };
                if p.move_tracks(&moving, to) {
                    forget_track_indexes(s);
                }
            }
        }
        if !down {
            s.tracks.track_drag = None;
        }
    }
    let v = &mut s.tracks;

    // ---- individual track height: drag the lower edge of a track (header or lane), as in
    // Logic. Registered after headers and regions so the edge wins over them.
    let edges = Rect::from_min_max(pos2(headers.left(), lanes.top()), lanes.right_bottom());
    for (i, &(top, row_h)) in rows.iter().enumerate() {
        if row_h <= 0.0 {
            continue;
        }
        let bottom = lanes.top() + top + row_h - scroll_y;
        if bottom < edges.top() || bottom > edges.bottom() {
            continue;
        }
        let edge = Rect::from_min_max(
            pos2(edges.left(), bottom - RESIZE_GRIP),
            pos2(edges.right(), bottom + RESIZE_GRIP),
        )
        .intersect(edges);
        let resp = ui
            .interact(edge, id(("track_edge", i)), Sense::drag())
            .on_hover_cursor(egui::CursorIcon::ResizeVertical);
        if resp.dragged() {
            if let (Some(pt), Some(t)) = (resp.interact_pointer_pos(), p.tracks.get_mut(i)) {
                let wanted = pt.y - (bottom - row_h) - take_rows_height(t);
                t.height = (wanted / v.row_h).clamp(Track::MIN_HEIGHT, Track::MAX_HEIGHT);
            }
        }
        if resp.double_clicked() {
            if let Some(t) = p.tracks.get_mut(i) {
                t.height = 1.0;
            }
        }
    }
    gh.line_segment(
        [headers.right_top(), headers.right_bottom()],
        Stroke::new(1.0, crate::theme::BORDER),
    );

    // ---- corner above the headers: "+" (new track), duplicate track, global tracks
    let gc = ui.painter_at(corner);
    gc.rect_filled(corner, 0.0, Color32::from_gray(0x3a));
    gc.line_segment(
        [corner.left_bottom(), corner.right_bottom()],
        Stroke::new(1.0, crate::theme::BORDER),
    );
    let plus = Rect::from_center_size(
        pos2(corner.left() + 18.0, corner.center().y),
        vec2(24.0, 18.0),
    );
    let pr = ui.interact(plus, id("plus"), Sense::click());
    gc.rect_filled(
        plus,
        4.0,
        if pr.hovered() {
            Color32::from_gray(0x66)
        } else {
            BTN_BG
        },
    );
    gc.line_segment(
        [
            plus.center() + vec2(-5.0, 0.0),
            plus.center() + vec2(5.0, 0.0),
        ],
        stroke(TEXT),
    );
    gc.line_segment(
        [
            plus.center() + vec2(0.0, -5.0),
            plus.center() + vec2(0.0, 5.0),
        ],
        stroke(TEXT),
    );
    if pr.clicked() {
        p.add_track(TrackKind::Instrument);
    }
    let dup = Rect::from_center_size(
        pos2(plus.right() + 18.0, corner.center().y),
        vec2(24.0, 18.0),
    );
    let dr = ui.interact(dup, id("dup"), Sense::click());
    let dup_bg = if dr.hovered() {
        Color32::from_gray(0x66)
    } else {
        BTN_BG
    };
    gc.rect_filled(dup, 4.0, dup_bg);
    gc.rect_stroke(
        Rect::from_center_size(dup.center() + vec2(1.5, -1.5), vec2(8.0, 7.0)),
        1.0,
        stroke(TEXT),
        StrokeKind::Middle,
    );
    gc.rect_filled(
        Rect::from_center_size(dup.center() + vec2(-1.5, 1.5), vec2(8.0, 7.0)),
        1.0,
        dup_bg,
    );
    gc.rect_stroke(
        Rect::from_center_size(dup.center() + vec2(-1.5, 1.5), vec2(8.0, 7.0)),
        1.0,
        stroke(TEXT),
        StrokeKind::Middle,
    );
    if dr.clicked() {
        if let Some(t) = p.tracks.get(p.selected_track).cloned() {
            p.tracks.insert(p.selected_track + 1, t);
            p.vca_inserted(p.selected_track + 1);
            // The copy keeps its VCA (the shift above also moved the copy's own entry).
            let copy = p.selected_track + 1;
            let vca = p.tracks.get(p.selected_track).and_then(|t| t.vca);
            p.tracks[copy].vca = vca;
            p.selected_track += 1;
            p.song_rev += 1;
        }
    }
    let gt = Rect::from_center_size(
        pos2(corner.right() - 18.0, corner.center().y),
        vec2(24.0, 18.0),
    );
    gc.rect_stroke(
        Rect::from_center_size(gt.center(), vec2(14.0, 10.0)),
        1.5,
        stroke(TEXT_DIM),
        StrokeKind::Middle,
    );
    gc.line_segment(
        [
            gt.center() + vec2(-7.0, -1.5),
            gt.center() + vec2(7.0, -1.5),
        ],
        stroke(TEXT_DIM),
    );

    // ---- ruler
    let gr = ui.painter_at(ruler);
    gr.rect_filled(ruler, 0.0, RULER_BG);
    gr.line_segment(
        [ruler.left_bottom(), ruler.right_bottom()],
        Stroke::new(1.0, crate::theme::BORDER),
    );
    // The stretch of the ruler over the selected region is marked, in gray: gold is for Cycle.
    let selection = v
        .selected
        .and_then(|(t, r)| p.tracks.get(t)?.regions.get(r))
        .map(|r| (p.region_start_beats(r), p.region_end_beats(r)));
    if let Some((start, end)) = selection {
        let band = Rect::from_min_max(
            pos2(x_of(start).max(ruler.left()), ruler.top()),
            pos2(x_of(end).min(ruler.right()), ruler.bottom()),
        );
        if band.width() > 0.0 {
            gr.rect_filled(band, 0.0, RULER_SELECTION);
            gr.line_segment(
                [band.left_bottom(), band.right_bottom()],
                Stroke::new(1.0, RULER_SELECTION_EDGE),
            );
        }
    }
    if p.cycle {
        let band = Rect::from_min_max(
            pos2(x_of(p.cycle_start), ruler.top() + 2.0),
            pos2(x_of(p.cycle_end), ruler.bottom() - 1.0),
        );
        gr.rect_filled(band, 2.0, CYCLE);
    } else {
        gr.line_segment(
            [
                pos2(x_of(p.cycle_start), ruler.top() + 1.0),
                pos2(x_of(p.cycle_end), ruler.top() + 1.0),
            ],
            Stroke::new(2.0, CYCLE_OFF),
        );
    }
    if p.punch {
        gr.line_segment(
            [
                pos2(x_of(p.punch_start), ruler.bottom() - 2.0),
                pos2(x_of(p.punch_end), ruler.bottom() - 2.0),
            ],
            Stroke::new(3.0, Color32::from_rgb(0xd9, 0x4f, 0x4f)),
        );
    }
    let label_every = [1i64, 2, 4, 8, 16, 32]
        .into_iter()
        .find(|n| *n as f32 * v.bar_w >= 34.0)
        .unwrap_or(64);
    for bar in &bars {
        let b = bar.bar0 as i64;
        let x = x_of(bar.start);
        let beat = bar.start;
        let in_cycle = p.cycle && beat >= p.cycle_start && beat < p.cycle_end;
        let in_selection = selection.is_some_and(|(s0, s1)| beat >= s0 && beat < s1);
        let col = if in_cycle {
            Color32::from_gray(0x2a)
        } else if in_selection {
            Color32::WHITE
        } else {
            TEXT
        };
        if b % label_every == 0 {
            gr.line_segment(
                [pos2(x, ruler.top() + 4.0), pos2(x, ruler.bottom())],
                Stroke::new(1.0, col.gamma_multiply(0.7)),
            );
            gr.text(
                pos2(x + 3.0, ruler.top() + 4.0),
                Align2::LEFT_TOP,
                (b + 1).to_string(),
                FontId::proportional(11.5),
                col,
            );
        } else {
            gr.line_segment(
                [pos2(x, ruler.bottom() - 7.0), pos2(x, ruler.bottom())],
                Stroke::new(1.0, col.gamma_multiply(0.6)),
            );
        }
        let unit_px = beat_px * bar.unit as f32;
        if unit_px >= 10.0 {
            // Beats, and halves of them for as far as there is room, down to single ticks' worth.
            let mut parts = 1u32;
            while unit_px / (parts * 2) as f32 >= 12.0 && parts < 256 {
                parts *= 2;
            }
            for k in 0..bar.num as u32 * parts {
                let on_beat = k % parts == 0;
                if k == 0 {
                    continue;
                }
                let at = bar.start + f64::from(k) * bar.unit / f64::from(parts);
                let xb = x_of(at);
                if xb < ruler.left() || xb > ruler.right() {
                    continue;
                }
                let tall = if on_beat { 4.0 } else { 2.5 };
                gr.line_segment(
                    [pos2(xb, ruler.bottom() - tall), pos2(xb, ruler.bottom())],
                    Stroke::new(1.0, col.gamma_multiply(if on_beat { 0.45 } else { 0.3 })),
                );
                // Zoomed in, a bar's number may be out of sight: each beat says where it is.
                if on_beat && unit_px >= 60.0 {
                    gr.text(
                        pos2(xb + 3.0, ruler.top() + 4.0),
                        Align2::LEFT_TOP,
                        format!("{}.{}", b + 1, k / parts + 1),
                        FontId::proportional(10.5),
                        col.gamma_multiply(0.8),
                    );
                }
            }
        }
    }
    // Upper half: click toggles cycle, drag sets it (snapped to bars). Lower half: locate.
    let upper = Rect::from_min_max(ruler.min, pos2(ruler.right(), ruler.center().y));
    let lower = Rect::from_min_max(pos2(ruler.left(), ruler.center().y), ruler.max);
    let up = ui.interact(upper, id("ruler_cycle"), Sense::click_and_drag());
    if up.drag_started() {
        v.cycle_anchor = up
            .interact_pointer_pos()
            .map(|pt| p.sigs().nearest_bar_line(beat_of(pt.x)));
    }
    if let (Some(anchor), true, Some(pt)) =
        (v.cycle_anchor, up.dragged(), up.interact_pointer_pos())
    {
        let other = p.sigs().nearest_bar_line(beat_of(pt.x));
        if (other - anchor).abs() >= p.bar_len_at(anchor.min(other)) - 1e-9 {
            p.cycle_start = anchor.min(other);
            p.cycle_end = anchor.max(other);
            p.cycle = true;
        }
    }
    if up.drag_stopped() {
        v.cycle_anchor = None;
    }
    if up.clicked() {
        crate::key_commands::toggle_cycle(p, v);
    }
    let low = ui.interact(lower, id("ruler_locate"), Sense::click_and_drag());
    if let Some(pt) = low.interact_pointer_pos() {
        if low.clicked() || low.dragged() {
            p.position_beats = beat_of(pt.x);
        }
    }

    // ---- playhead
    let px = x_of(p.position_beats);
    // Grab the playhead's head in the ruler and drag it anywhere in the song (back to the
    // start, past the end of the view, ...). Registered after the ruler so the head wins over
    // the cycle area; kept alive while dragging even when the head leaves the view.
    let head_id = id("playhead_head");
    let visible = px >= lanes.left() && px <= lanes.right();
    if visible || ui.ctx().is_being_dragged(head_id) {
        let grab = Rect::from_min_max(
            pos2(px - PLAYHEAD_GRAB, ruler.top()),
            pos2(px + PLAYHEAD_GRAB, ruler.bottom()),
        );
        let resp = ui
            .interact(grab, head_id, Sense::drag())
            .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        if resp.dragged() {
            if let Some(pt) = resp.interact_pointer_pos() {
                p.position_beats = playhead_beat(pt.x, lanes.left(), scroll_x, beat_px);
            }
        }
    }
    // The end of the project: a marker to drag. Past it is dimmed, and playback stops there.
    let shown_end = p
        .end_beat
        .unwrap_or_else(|| crate::bounce::project_end(p).max(p.bar_to_beats(8.0)));
    let end_x = x_of(shown_end);
    if end_x < lanes.right() {
        let past = Rect::from_min_max(pos2(end_x.max(lanes.left()), lanes.top()), lanes.max);
        ui.painter_at(lanes)
            .rect_filled(past, 0.0, Color32::from_black_alpha(70));
    }
    let grip = Rect::from_center_size(pos2(end_x, ruler.center().y), vec2(12.0, ruler.height()));
    let end_resp = ui
        .interact(grip, id("project_end"), Sense::drag())
        .on_hover_text("The end of the project");
    gr.add(Shape::convex_polygon(
        vec![
            pos2(end_x, ruler.top() + 3.0),
            pos2(end_x, ruler.bottom() - 3.0),
            pos2(end_x - 7.0, ruler.center().y),
        ],
        if p.end_beat.is_some() {
            Color32::from_gray(0xd8)
        } else {
            Color32::from_gray(0x90)
        },
        Stroke::NONE,
    ));
    if end_resp.dragged() {
        if let Some(pt) = end_resp.interact_pointer_pos() {
            let bar_line = p
                .sigs()
                .nearest_bar_line(beat_of(pt.x))
                .max(p.bar_to_beats(1.0));
            p.end_beat = Some(bar_line);
        }
    }
    if end_resp.dragged() || end_resp.hovered() {
        let bar = p.beats_to_bar(shown_end).round() as i64 + 1;
        ui.painter_at(area).text(
            pos2(end_x - 10.0, ruler.bottom() + 14.0),
            Align2::RIGHT_CENTER,
            format!("End {bar} 1 1 1"),
            FontId::proportional(11.5),
            Color32::from_rgb(0xe8, 0xcf, 0x70),
        );
    }
    if p.playing
        && !p.recording
        && !p.cycle
        && p.end_beat.is_some_and(|end| p.position_beats >= end)
    {
        p.playing = false;
    }
    // While the zoom moves: what it is at, and where the view is in the whole project.
    let zoom_now = (v.bar_w, v.row_h);
    if v.zoom_seen != Some(zoom_now) {
        if let Some(was) = v.zoom_seen {
            let text = if was.0 == zoom_now.0 {
                let (lo, hi) = (MIN_ROW_H.ln(), MAX_ROW_H.ln());
                format!("{:.0}", (v.row_h.ln() - lo) / (hi - lo) * 100.0)
            } else {
                let bars = lanes.width() / v.bar_w;
                if bars >= 1.0 {
                    format!("{bars:.1} bars in view")
                } else {
                    format!("{:.2} beats in view", bars * bpb as f32)
                }
            };
            v.zoom_note = Some((text, std::time::Instant::now()));
        }
        v.zoom_seen = Some(zoom_now);
    }
    if let Some((text, at)) = v.zoom_note.clone() {
        if at.elapsed().as_secs_f32() < ZOOM_NOTE_SECONDS {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
            let gz = ui.painter_at(area);
            let tag = Rect::from_min_size(
                pos2(area.right() - 150.0, ruler.top() + 2.0),
                vec2(142.0, 22.0),
            );
            gz.rect_filled(tag, 4.0, Color32::from_black_alpha(220));
            gz.text(
                tag.center(),
                Align2::CENTER_CENTER,
                text,
                FontId::proportional(12.0),
                Color32::WHITE,
            );
            draw_overview(
                &gz,
                p,
                &rows,
                lanes,
                (v.scroll_x, v.scroll_y),
                beat_px,
                shown_end,
            );
        } else {
            v.zoom_note = None;
        }
    }
    if visible {
        let (line_c, head_c, edge_c) = playhead_colors(p.recording);
        let gp = ui.painter_at(Rect::from_min_max(ruler.min, area.max));
        gp.line_segment(
            [pos2(px, ruler.top() + 6.0), pos2(px, area.bottom())],
            Stroke::new(
                if crate::settings::wide_playhead() {
                    2.5
                } else {
                    1.0
                },
                line_c,
            ),
        );
        let head = vec![
            pos2(px - 6.0, ruler.top() + 2.0),
            pos2(px + 6.0, ruler.top() + 2.0),
            pos2(px + 6.0, ruler.top() + 9.0),
            pos2(px, ruler.top() + 15.0),
            pos2(px - 6.0, ruler.top() + 9.0),
        ];
        gp.add(Shape::convex_polygon(
            head,
            head_c,
            Stroke::new(1.0, edge_c),
        ));
    }
}

#[cfg(test)]
mod row_tests {
    use super::*;

    #[test]
    fn volume_slider_round_trips_and_puts_unity_two_thirds_up() {
        assert_eq!(db_to_slider(0.0), SLIDER_UNITY);
        assert_eq!(db_to_slider(-60.0), 0.0);
        assert_eq!(db_to_slider(6.0), 1.0);
        for db in [-60.0, -30.0, -12.5, -0.1, 0.0, 2.0, 6.0] {
            assert!((slider_to_db(db_to_slider(db)) - db).abs() < 1e-3, "{db}");
        }
    }

    #[test]
    fn the_toolbar_drops_its_least_used_items_first_when_narrow() {
        let all = SNAP_W + DRAG_W + TOOLS_W + WAVE_W;
        let f = fit_toolbar(all);
        assert!(f.snap && f.drag && f.tools && f.wave);
        let f = fit_toolbar(all - 1.0);
        assert!(
            f.snap && f.drag && f.tools && !f.wave,
            "waveform buttons go first"
        );
        let f = fit_toolbar(SNAP_W + DRAG_W);
        assert!(f.snap && f.drag && !f.tools && !f.wave);
        let f = fit_toolbar(SNAP_W);
        assert!(f.snap && !f.drag);
        assert_eq!(
            fit_toolbar(-50.0),
            ToolbarFit {
                snap: false,
                drag: false,
                tools: false,
                wave: false
            }
        );
    }

    #[test]
    fn rows_follow_each_tracks_height() {
        let mut p = Project::demo();
        p.tracks.truncate(3);
        p.tracks[1].height = 2.0;
        p.tracks[2].height = 0.5;
        let rows = row_layout(&p, 56.0);
        assert_eq!(rows, vec![(0.0, 56.0), (56.0, 112.0), (168.0, 28.0)]);
        assert_eq!(row_at(&rows, 0.0), Some(0));
        assert_eq!(row_at(&rows, 167.9), Some(1));
        assert_eq!(row_at(&rows, 168.0), Some(2));
        assert_eq!(row_at(&rows, 196.0), None);
        assert_eq!(row_at(&rows, -1.0), None);
    }

    #[test]
    fn the_tracks_of_a_folded_folder_take_no_room() {
        let mut p = Project::demo();
        p.tracks.truncate(4);
        let folder = p.create_track_stack(1); // wraps track 1: [t0, F, t1, t2, t3]
        assert_eq!(folder, 1);
        let open = row_layout(&p, 56.0);
        assert_eq!(open.len(), 5);
        assert!(open.iter().all(|&(_, h)| h == 56.0));

        p.tracks[1].folded = true;
        let folded = row_layout(&p, 56.0);
        assert_eq!(folded[2].1, 0.0, "the wrapped track is hidden");
        assert_eq!(
            folded[3],
            (112.0, 56.0),
            "the next track moves up under the folder"
        );
        assert_eq!(row_at(&folded, 100.0), Some(1));
        assert_eq!(row_at(&folded, 112.0), Some(3), "hidden rows are never hit");
    }

    #[test]
    fn dragging_the_playhead_follows_the_pointer_and_stops_at_the_start() {
        // Lanes start at x = 100, scrolled by 50 px, 20 px per beat.
        assert_eq!(playhead_beat(150.0, 100.0, 50.0, 20.0), 5.0);
        assert_eq!(playhead_beat(100.0, 100.0, 0.0, 20.0), 0.0);
        // Dragged left of the lanes: pinned to the beginning of the song.
        assert_eq!(playhead_beat(20.0, 100.0, 0.0, 20.0), 0.0);
    }

    #[test]
    fn solo_silences_every_other_track_until_released() {
        let mut p = Project::demo();
        assert!((0..p.tracks.len()).all(|i| !silenced_by_solo(&p, i)));
        p.tracks[2].solo = true;
        assert!(!silenced_by_solo(&p, 2));
        assert!((0..p.tracks.len())
            .filter(|&i| i != 2)
            .all(|i| silenced_by_solo(&p, i)));
        // Soloing a second track adds it to what plays.
        p.tracks[4].solo = true;
        assert!(!silenced_by_solo(&p, 4) && silenced_by_solo(&p, 0));
        p.tracks[2].solo = false;
        p.tracks[4].solo = false;
        assert!((0..p.tracks.len()).all(|i| !silenced_by_solo(&p, i)));
    }

    #[test]
    fn beat_snap_follows_the_signature_beat() {
        // 4/4: a beat is a quarter note, as before.
        assert_eq!(snap_step(Snap::Beat, 20.0, 4.0, 1.0), 1.0);
        assert_eq!(snap_step(Snap::Smart, 20.0, 4.0, 1.0), 1.0);
        assert_eq!(snap_step(Snap::Bar, 20.0, 4.0, 1.0), 4.0);
        // 6/8: a beat is an eighth (0.5 quarter notes), a bar 3 quarter notes.
        assert_eq!(snap_step(Snap::Beat, 20.0, 3.0, 0.5), 0.5);
        assert_eq!(snap_step(Snap::Bar, 20.0, 3.0, 0.5), 3.0);
        // Smart snaps to eighths once an eighth is at least 12 px wide, else to bars.
        assert_eq!(snap_step(Snap::Smart, 24.0, 3.0, 0.5), 0.5);
        assert_eq!(snap_step(Snap::Smart, 20.0, 3.0, 0.5), 3.0);
        // Divisions are 16th notes in any signature.
        assert_eq!(snap_step(Snap::Division, 20.0, 3.0, 0.5), 0.25);
        assert_eq!(snap_step(Snap::Smart, 60.0, 3.0, 0.5), 0.25);
    }

    /// A 0.5 s mono 16-bit 44.1 kHz WAV: a square wave at `amp` (0..1), or silence for 0.
    fn write_square_wav(path: &std::path::Path, amp: f32) {
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
        for i in 0..n {
            let s = if (i / 50) % 2 == 0 { amp } else { -amp };
            b.extend_from_slice(&((s * 32767.0) as i16).to_le_bytes());
        }
        std::fs::write(path, b).unwrap();
    }

    /// Runs `poll_normalize` until the scan is over (it reads the file on another thread).
    fn wait_for_normalize(p: &mut Project, s: &mut UiState) {
        let ctx = egui::Context::default();
        let began = std::time::Instant::now();
        while !s.tracks.normalize_jobs.is_empty() {
            assert!(began.elapsed().as_secs() < 10, "the scan never finished");
            poll_normalize(&ctx, p, s);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn project_with_wav(amp: f32) -> (Project, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("take.wav");
        write_square_wav(&wav, amp);
        let mut p = Project::empty();
        crate::import::import_audio(&mut p, &wav, &egui::Context::default()).unwrap();
        (p, dir)
    }

    #[test]
    fn normalize_reads_the_file_in_the_background_and_sets_the_gain() {
        let (mut p, _dir) = project_with_wav(0.25);
        let (t, mut s) = (p.tracks.len() - 1, UiState::new());
        start_normalize(&p, &mut s, t, 0);
        assert_eq!(s.tracks.normalize_jobs.len(), 1);
        assert_eq!(s.status.as_deref(), Some("Normalizing…"));
        start_normalize(&p, &mut s, t, 0);
        assert_eq!(s.tracks.normalize_jobs.len(), 1, "one scan per region");
        wait_for_normalize(&mut p, &mut s);
        // A peak of 0.25 needs +12.04 dB to reach full scale.
        let gain = p.tracks[t].regions[0].params.gain_db;
        assert!((gain - 12.04).abs() < 0.05, "gain {gain}");
        assert_eq!(s.status, None, "the message goes away when it is done");
    }

    #[test]
    fn normalize_reports_silence_and_regions_without_audio() {
        let (mut p, _dir) = project_with_wav(0.0);
        let (t, mut s) = (p.tracks.len() - 1, UiState::new());
        start_normalize(&p, &mut s, t, 0);
        wait_for_normalize(&mut p, &mut s);
        assert_eq!(p.tracks[t].regions[0].params.gain_db, 0.0);
        assert_eq!(s.status.as_deref(), Some("Normalize: the region is silent"));
        // A MIDI region has nothing to scan.
        let mut s = UiState::new();
        start_normalize(&Project::demo(), &mut s, 0, 0);
        assert!(s.tracks.normalize_jobs.is_empty());
        assert_eq!(
            s.status.as_deref(),
            Some("Normalize: this region has no recorded audio")
        );
    }

    #[test]
    fn a_scan_for_a_region_that_has_gone_changes_nothing() {
        let (mut p, _dir) = project_with_wav(0.25);
        let (t, mut s) = (p.tracks.len() - 1, UiState::new());
        start_normalize(&p, &mut s, t, 0);
        // The user deletes the region while the file is being read.
        p.tracks[t].regions.clear();
        wait_for_normalize(&mut p, &mut s);
        assert!(p.tracks[t].regions.is_empty());
        assert_eq!(s.status, None);
    }

    #[test]
    fn the_fade_tool_measures_from_the_edge_in_sixteenths() {
        // A region from beat 8, 4 beats long.
        assert_eq!(fade_for_drag(9.0, 8.0, 4.0, true), 1.0);
        assert_eq!(
            fade_for_drag(9.1, 8.0, 4.0, true),
            1.0,
            "snaps to 1/16 notes"
        );
        assert_eq!(fade_for_drag(9.2, 8.0, 4.0, true), 1.25);
        assert_eq!(fade_for_drag(11.0, 8.0, 4.0, false), 1.0, "from the end");
        assert_eq!(
            fade_for_drag(7.0, 8.0, 4.0, true),
            0.0,
            "not before the start"
        );
        assert_eq!(
            fade_for_drag(13.0, 8.0, 4.0, false),
            0.0,
            "not past the end"
        );
        assert_eq!(
            fade_for_drag(30.0, 8.0, 4.0, true),
            4.0,
            "not longer than the region"
        );
        assert_eq!(fade_for_drag(-5.0, 8.0, 4.0, false), 4.0);
    }

    #[test]
    fn delete_removes_the_track_last_clicked_and_a_region_comes_first() {
        let meters = crate::mixer::Meters::default();
        let mut p = Project::empty();
        let keep = p.add_track(TrackKind::Instrument);
        let goes = p.add_track(TrackKind::Audio);
        let mut s = UiState::new();
        let mut h = crate::ui_test::Harness::new(1200.0, 700.0);
        let count = p.tracks.len();
        p.selected_track = goes;

        // Nothing clicked: Delete leaves the tracks alone.
        h.settle(|ui| show(ui, &mut p, &mut s, &meters));
        h.key(Key::Delete, |ui| show(ui, &mut p, &mut s, &meters));
        assert_eq!(p.tracks.len(), count);

        // A click on its mixer strip, then Delete.
        s.mixer.track_clicked = true;
        h.settle(|ui| show(ui, &mut p, &mut s, &meters));
        assert!(s.tracks.track_focus);
        h.key(Key::Delete, |ui| show(ui, &mut p, &mut s, &meters));
        h.frame(vec![], |ui| show(ui, &mut p, &mut s, &meters));
        assert_eq!(p.tracks.len(), count - 1);
        assert_eq!(p.tracks[keep].kind, TrackKind::Instrument);

        // A selected region goes first; the track stays.
        p.selected_track = keep;
        let region = p.add_midi_region(keep, 1.0).unwrap();
        s.tracks.track_focus = true;
        s.tracks.selected = Some((keep, region));
        let tracks = p.tracks.len();
        h.settle(|ui| show(ui, &mut p, &mut s, &meters));
        h.key(Key::Delete, |ui| show(ui, &mut p, &mut s, &meters));
        assert_eq!(p.tracks.len(), tracks);
        assert!(p.tracks[keep].regions.is_empty());
    }

    #[test]
    fn the_edit_keys_cut_copy_paste_split_and_join_the_selected_region() {
        let meters = crate::mixer::Meters::default();
        let mut p = Project::empty();
        let t = p.add_track(TrackKind::Instrument);
        let r = p.add_midi_region(t, 1.0).unwrap();
        let mut s = UiState::new();
        s.tracks.selected = Some((t, r));
        let mut h = crate::ui_test::Harness::new(1200.0, 700.0);
        h.settle(|ui| show(ui, &mut p, &mut s, &meters));
        let mut press = |key: Key, p: &mut Project, s: &mut UiState| {
            h.modifiers = Modifiers::COMMAND;
            h.frame(vec![], |ui| show(ui, p, s, &meters));
            h.key(key, |ui| {
                crate::key_commands::run(ui.ctx(), p, s);
                show(ui, p, s, &meters)
            });
            h.modifiers = Modifiers::NONE;
            h.frame(vec![], |ui| show(ui, p, s, &meters));
        };

        // Split at the playhead, and join again.
        p.position_beats = 8.0;
        press(Key::T, &mut p, &mut s);
        assert_eq!(p.tracks[t].regions.len(), 2);
        s.tracks.selected = Some((t, 0));
        press(Key::J, &mut p, &mut s);
        assert_eq!(p.tracks[t].regions.len(), 1);
        assert_eq!(p.tracks[t].regions[0].len_bars, 4.0);

        // Copy, and paste at the playhead.
        s.tracks.selected = Some((t, 0));
        press(Key::C, &mut p, &mut s);
        p.position_beats = 32.0;
        press(Key::V, &mut p, &mut s);
        assert_eq!(p.tracks[t].regions.len(), 2);
        assert_eq!(p.tracks[t].regions[1].start_bar, 9.0);

        // Cut takes it away, and it can come back.
        s.tracks.selected = Some((t, 1));
        press(Key::X, &mut p, &mut s);
        assert_eq!(p.tracks[t].regions.len(), 1);
        assert!(!s.tracks.clipboard.is_empty());
        press(Key::V, &mut p, &mut s);
        assert_eq!(p.tracks[t].regions.len(), 2);
    }

    /// Whether a right-click at `pos` opens a menu.
    fn right_click_opens_a_menu(pos: egui::Pos2, p: &mut Project, s: &mut UiState) -> bool {
        let meters = crate::mixer::Meters::default();
        let mut h = crate::ui_test::Harness::new(1200.0, 700.0);
        h.settle(|ui| show(ui, p, s, &meters));
        h.right_click(pos, |ui| show(ui, p, s, &meters));
        h.settle(|ui| show(ui, p, s, &meters));
        egui::Popup::is_any_open(&h.ctx)
    }

    #[test]
    fn right_clicking_anywhere_on_the_tracks_area_has_a_menu() {
        let mut p = Project::empty();
        let t = p.add_track(TrackKind::Instrument);
        p.add_midi_region(t, 1.0).unwrap();
        let header_x = 40.0;
        let lane_x = HEADER_W + 600.0;
        let mut s = UiState::new();
        assert!(
            right_click_opens_a_menu(egui::pos2(header_x, 660.0), &mut p, &mut s),
            "under the headers"
        );
        let mut s = UiState::new();
        assert!(
            right_click_opens_a_menu(egui::pos2(lane_x, 660.0), &mut p, &mut s),
            "under the tracks"
        );
        let mut s = UiState::new();
        assert!(
            right_click_opens_a_menu(egui::pos2(header_x, 130.0), &mut p, &mut s),
            "on a header"
        );
        let mut s = UiState::new();
        assert!(
            right_click_opens_a_menu(egui::pos2(lane_x, 130.0), &mut p, &mut s),
            "on a lane"
        );
    }

    #[test]
    fn selection_grows_shrinks_and_a_box_selects_what_it_touches() {
        let mut v = TracksView::default();
        v.select_only((0, 0));
        v.toggle((1, 0));
        v.toggle((2, 0));
        assert_eq!(v.all_selected().len(), 3);
        assert!(v.is_selected((0, 0)) && v.is_selected((2, 0)));
        v.toggle((2, 0));
        assert!(!v.is_selected((2, 0)));
        v.select_only((5, 5));
        assert_eq!(v.all_selected(), [(5, 5)]);

        let mut p = Project::empty();
        let t = p.add_track(TrackKind::Instrument);
        p.add_midi_region(t, 1.0).unwrap();
        p.add_midi_region(t, 9.0).unwrap();
        v.select_all(&p);
        assert_eq!(v.all_selected().len(), 2);
        p.tracks[t].regions.pop();
        v.tidy_selection(&p);
        assert_eq!(v.all_selected(), [(t, 0)], "gone regions drop out");
    }

    #[test]
    fn a_selected_group_deletes_and_repeats_together() {
        let mut p = Project::empty();
        let t = p.add_track(TrackKind::Instrument);
        for bar in [1.0, 5.0, 9.0] {
            let r = p.add_midi_region(t, bar).unwrap();
            p.tracks[t].regions[r].len_bars = 2.0;
        }
        let mut v = TracksView::default();
        v.select_many(vec![(t, 0), (t, 1)]);
        repeat_selected(&mut p, &mut v);
        assert_eq!(p.tracks[t].regions.len(), 5);
        assert_eq!(
            p.tracks[t].regions[3].start_bar, 7.0,
            "after the group's end"
        );
        assert_eq!(v.all_selected().len(), 2);
        v.select_many(vec![(t, 0), (t, 2)]);
        delete_selected(&mut p, &mut v);
        assert_eq!(p.tracks[t].regions.len(), 3);
        assert_eq!(p.tracks[t].regions[0].start_bar, 5.0);
        assert!(v.all_selected().is_empty());
    }
}

#[cfg(test)]
mod tool_tests {
    use super::*;

    #[test]
    fn every_tool_in_the_menu_is_a_real_tool() {
        for (n, name) in TOOLS.iter().enumerate().skip(1) {
            assert_ne!(TrackTool::of_entry(n), TrackTool::Pointer, "{name}");
        }
        assert_eq!(TrackTool::of_entry(0), TrackTool::Pointer);
        assert_eq!(TrackTool::of_entry(SCISSORS_TOOL), TrackTool::Scissors);
        assert!(!TrackTool::Pointer.only_clicks() && !TrackTool::Fade.only_clicks());
    }
}
