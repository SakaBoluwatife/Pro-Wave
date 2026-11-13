//! Piano Roll (Logic Pro: Piano Roll editor): editor tabs, local toolbar, the left inspector
//! (Time Quantize, Scale Quantize, Velocity), piano keys, ruler with the region strip, and
//! the note grid.
//!
//! Works today: select (click, Shift-click, marquee), move (time and pitch, with Snap),
//! resize from the right edge, create (click), drag-select, delete (Del or right-click), keyboard nudging, Time Quantize
//! with Strength and Swing, Scale Quantize, velocity editing, zoom and scrolling. Edits go to
//! the project and reach the engine when the mouse is released. Score, Step Sequencer and
//! Smart Tempo are not implemented yet and show dimmed.
//!
//! As in FL Studio, dragging the ruler with the right button makes a time selection, which takes
//! the notes that start in it; Ctrl+Delete and Ctrl+Insert take time out or open it up. The lane
//! under the notes shows velocity or any MIDI controller, chosen from its label.
use crate::icons::Icon;
use crate::i18n::{tr, trf};
use crate::model::{Note, Project, TrackKind};
use crate::note_edit::{self, GRIDS, MODES, ROOTS};
use crate::tracks_area::{snap_step, snap_to, Snap};
use crate::widgets::{
    back_chevron, blue, menu_title, popup_button, slider_h, btn_bg_col, text_col, text_dim_col,
};
use crate::UiState;
use egui::{
    pos2, vec2, Align2, Color32, FontId, Key, Painter, Pos2, Rect, Sense, Shape, Stroke,
    StrokeKind, Ui,
};

const TAB_H: f32 = 26.0;
/// The Audio Editor tab.
const AUDIO_TAB: usize = 5;
const TOOLBAR_H: f32 = 30.0;
const INSPECTOR_W: f32 = 240.0;
const KEYS_W: f32 = 48.0;
const RULER_H: f32 = 22.0;
const REGION_H: f32 = 18.0;
const SB_H: f32 = 12.0;
const VEL_H: f32 = 72.0;
/// A stacked controller lane, and the most of them.
const EXTRA_H: f32 = 44.0;
const MAX_EXTRA_LANES: usize = 4;

fn panel_bg_col() -> Color32 {
    crate::theme::panel_bg()
}
fn toolbar_bg_col() -> Color32 {
    crate::theme::toolbar()
}
fn row_white_col() -> Color32 {
    crate::theme::grey_rgb(0x0e, 0x0e, 0x0e)
}
fn row_black_col() -> Color32 {
    crate::theme::grey_rgb(0x0b, 0x0b, 0x0b)
}
/// A grid line or mark: `share` of the way from the grid to the text colour.
fn grid_ink(share: f32) -> Color32 {
    crate::theme::ink_on(row_white_col(), share)
}
const NOTE_SEL: Color32 = Color32::from_rgb(0xe4, 0xff, 0xb0);
/// What the colour of a note says.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NoteColors {
    #[default]
    Velocity,
    Track,
    Pitch,
    /// The note's colour group, as in FL Studio.
    Group,
}

/// How many colour groups there are.
const COLOR_GROUPS: u8 = 16;

/// The colour of a colour group.
fn group_color(group: u8) -> Color32 {
    let hue = f32::from(group % COLOR_GROUPS) / f32::from(COLOR_GROUPS);
    egui::ecolor::Hsva::new(hue, 0.65, 0.9, 1.0).into()
}

/// A number below `count`, for picking a note. Nothing depends on it being a good one.
fn random_below(count: usize) -> usize {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |d| d.subsec_nanos());
    (nanos.wrapping_mul(2_654_435_761) >> 8) as usize % count.max(1)
}

/// A colour for each of the twelve notes of the octave.
pub(crate) fn pitch_color(pitch: u8) -> Color32 {
    let hue = f32::from(pitch % 12) / 12.0;
    egui::ecolor::Hsva::new(hue, 0.62, 0.86, 1.0).into()
}

/// Logic's velocity colours for a note: green when soft, yellow, then red when hard.
pub(crate) fn velocity_color(vel: u8) -> Color32 {
    let t = (vel as f32 / 127.0).clamp(0.0, 1.0);
    let green = Color32::from_rgb(60, 185, 70);
    let yellow = Color32::from_rgb(232, 200, 40);
    let red = Color32::from_rgb(216, 43, 40);
    if t < 0.5 {
        green.lerp_to_gamma(yellow, t / 0.5)
    } else {
        yellow.lerp_to_gamma(red, ((t - 0.5) / 0.37).min(1.0))
    }
}

/// The bright bar colour Logic gives a region in the editor (a yellow track: 253, 197, 0).
fn region_bar_color(c: Color32) -> Color32 {
    let max = c.r().max(c.g()).max(c.b()).max(1) as f32;
    let s = |v: u8| v as f32 * 253.0 / max;
    let (r, g, b) = (s(c.r()), s(c.g()), s(c.b()));
    let lum = 0.3 * r + 0.59 * g + 0.11 * b;
    let k = |v: f32| (lum + (v - lum) * 1.3).clamp(0.0, 253.0) as u8;
    Color32::from_rgb(k(r), k(g), k(b))
}

/// Live notes the Piano Roll wants to play; `main` forwards them to the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Audition {
    On {
        pitch: u8,
        vel: u8,
        track: u16,
    },
    Off {
        pitch: u8,
    },
    /// A controller (a sustain pedal) for the instrument of `track`.
    Control {
        track: u16,
        number: u8,
        value: u8,
    },
}

/// The tools, as in FL Studio's piano roll, with their keys.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tool {
    /// P: click to place a note and drag it; click a note to move it, its end to resize it.
    Draw,
    /// B: drag to lay notes one after the other.
    Paint,
    /// D: drag across notes to delete them.
    Delete,
    /// T: drag across notes to mute or unmute them.
    Mute,
    /// C: drag to a place in time and let go to cut the notes there.
    Slice,
    /// E: drag a box to select.
    Select,
    /// Z: drag a box to zoom to it.
    Zoom,
    /// Y: click a note to hear it, drag across notes to hear them in turn.
    Playback,
}

impl Tool {
    const ALL: [Tool; 8] = [
        Tool::Draw,
        Tool::Paint,
        Tool::Delete,
        Tool::Mute,
        Tool::Slice,
        Tool::Select,
        Tool::Zoom,
        Tool::Playback,
    ];

    fn hint(self) -> &'static str {
        match self {
            Tool::Draw => "Draw (P)",
            Tool::Paint => "Paint (B)",
            Tool::Delete => "Delete (D)",
            Tool::Mute => "Mute (T)",
            Tool::Slice => "Slice (C)",
            Tool::Select => "Select (E)",
            Tool::Zoom => "Zoom (Z)",
            Tool::Playback => "Playback (Y)",
        }
    }

    /// The picture on its button.
    fn icon(self) -> Icon {
        match self {
            Tool::Draw => Icon::Pencil,
            Tool::Paint => Icon::Brush,
            Tool::Delete => Icon::Eraser,
            Tool::Mute => Icon::Mute,
            Tool::Slice => Icon::Scissors,
            Tool::Select => Icon::Pointer,
            Tool::Zoom => Icon::Zoom,
            Tool::Playback => Icon::Play,
        }
    }
}

enum Drag {
    None,
    Move {
        anchor_beat: f32,
        anchor_pitch: i32,
        orig: Vec<(usize, f32, u8)>,
    },
    Resize {
        anchor_beat: f32,
        orig: Vec<(usize, f32)>,
    },
    Marquee {
        from: Pos2,
        /// What stays selected whatever the box covers.
        base: Vec<usize>,
    },
    /// Right button, or the Delete tool.
    Erase,
    /// The Mute tool: what it sets the notes it crosses to, and those it has set.
    Mute {
        on: bool,
        done: Vec<usize>,
    },
    /// The Paint tool: where it started, on one key.
    Paint {
        pitch: u8,
        start: f32,
    },
    /// The Slice tool: where the cut would go.
    Slice {
        beat: f32,
    },
    /// The Zoom tool: where the box began.
    Zoom {
        from: Pos2,
    },
    /// The Playback tool: the note under the pointer sounds.
    Scrub,
}

/// What the lane under the notes shows and edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lane {
    Velocity,
    /// A MIDI controller of the region, by number.
    Control(u8),
}

/// The controllers offered by name. Any other number can be typed in the same menu.
const NAMED_CONTROLS: [(u8, &str, &str); 10] = [
    (ovrtone_engine::BEND_LANE_CONTROL, "Pitch Bend", "Bend"),
    (ovrtone_engine::PRESSURE_CONTROL, "Aftertouch", "AT"),
    (1, "Modulation", "Mod"),
    (2, "Breath", "Breath"),
    (7, "Volume", "Volume"),
    (10, "Pan", "Pan"),
    (11, "Expression", "Expr"),
    (64, "Sustain", "Sustain"),
    (71, "Resonance", "Reso"),
    (74, "Cutoff", "Cutoff"),
];

impl Lane {
    /// In full, for the menu and the inspector.
    fn name(self) -> String {
        match self {
            Lane::Velocity => tr("Note Velocity").to_string(),
            Lane::Control(n) => match NAMED_CONTROLS.iter().find(|c| c.0 == n) {
                Some(c) => format!("{} (CC {n})", c.1),
                None => format!("CC {n}"),
            },
        }
    }

    /// Short enough for the label beside the lane, which is as wide as the piano keys.
    fn short(self) -> String {
        match self {
            Lane::Velocity => tr("Velocity").to_string(),
            Lane::Control(n) => match NAMED_CONTROLS.iter().find(|c| c.0 == n) {
                Some(c) => c.2.to_string(),
                None => format!("CC {n}"),
            },
        }
    }
}

/// The menu that picks what the lane shows. `used` are the controllers the region already has.
fn lane_menu(ui: &mut Ui, lane: &mut Lane, used: &[u8]) {
    ui.set_min_width(170.0);
    if ui
        .selectable_label(*lane == Lane::Velocity, Lane::Velocity.name())
        .clicked()
    {
        *lane = Lane::Velocity;
        ui.close();
    }
    ui.separator();
    let others = used
        .iter()
        .copied()
        .filter(|n| !NAMED_CONTROLS.iter().any(|c| c.0 == *n));
    for n in NAMED_CONTROLS.iter().map(|c| c.0).chain(others) {
        let choice = Lane::Control(n);
        // A dot says the region has this controller already.
        let text = if used.contains(&n) {
            format!("{} •", choice.name())
        } else {
            choice.name()
        };
        if ui.selectable_label(*lane == choice, text).clicked() {
            *lane = choice;
            ui.close();
        }
    }
    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Other CC");
        let mut number = match *lane {
            Lane::Control(n) => n,
            Lane::Velocity => 0,
        };
        if ui
            .add(egui::DragValue::new(&mut number).range(0..=127))
            .changed()
        {
            *lane = Lane::Control(number);
        }
    });
}

/// A stroke in a controller lane.
#[derive(Clone, Copy, Debug, PartialEq)]
enum LaneDrag {
    /// Left button: draws from the last point it was at (beat, value).
    Draw { beat: f32, value: u8 },
    /// Right button: erases from the last beat it was at.
    Erase { beat: f32 },
}

pub struct PianoView {
    /// Step input: notes played on a MIDI keyboard go in at the step cursor, which then moves on
    /// by one grid step. Notes played together make a chord at one step.
    pub step_input: bool,
    /// The step cursor, in beats from the region's start.
    pub step_at: f32,
    /// Where the chord being played started, and when its last note came.
    step_chord: Option<(f32, std::time::Instant)>,
    /// The grid step the editor shows, in beats: how far a step moves.
    pub step_len: f32,
    /// View > Note Colors.
    pub note_colors: NoteColors,
    /// What the lane under the notes shows.
    lane: Lane,
    lane_drag: Option<LaneDrag>,
    /// More controller lanes, stacked above the main one (Logic's Step Editor): their numbers.
    extra_lanes: Vec<u8>,
    /// A draw or erase in one of them: which, and how far it has got.
    extra_drag: Option<(usize, LaneDrag)>,
    /// The time selection, in beats from the region's start: made by dragging the ruler with the
    /// right button, as in FL Studio.
    time_sel: Option<(f32, f32)>,
    /// Where that drag began.
    time_anchor: Option<f32>,
    /// The groove template Quantize uses, in place of the grid.
    groove: Option<usize>,
    tool: Tool,
    /// Pitch currently sounding because of the mouse (key press or note edit).
    held: Option<u8>,
    /// Fit the region to the width of the editor on the next frame.
    fit: bool,
    /// Note whose velocity stem is being dragged in the velocity lane.
    vel_drag: Option<usize>,
    /// Keep `held` sounding until this time even if the mouse is up (click-created notes).
    release_at: Option<f64>,
    pub beat_px: f32,
    pub key_h: f32,
    scroll_x: f32,
    scroll_y: f32,
    pub selected: Vec<usize>,
    grid: usize,
    strength: f32,
    swing: f32,
    scale_root: Option<u8>,
    scale_mode: usize,
    /// What a new note copies: the length and velocity of the last one placed or touched.
    last_len: f32,
    new_velocity: u8,
    /// The colour group new notes get.
    color_group: u8,
    /// New notes are slide notes.
    slide_mode: bool,
    /// The slide range to give the track, chosen in the inspector.
    slide_range_edit: Option<u8>,
    /// Ctrl+C and Ctrl+X.
    clipboard: Vec<Note>,
    drag: Drag,
    centered_for: Option<(usize, usize)>,
    /// Edits waiting for the mouse to be released before the engine song is rebuilt.
    dirty: bool,
    /// Show the notes of the other regions behind this one's (Alt+V).
    ghosts: bool,
    /// Where the note grid was last drawn.
    last_grid: Rect,
    /// Arpeggiate, Strum and the rest: their settings and the one open.
    tools: crate::note_tools::ToolSettings,
}

impl Default for PianoView {
    fn default() -> Self {
        Self {
            note_colors: NoteColors::default(),
            lane: Lane::Velocity,
            lane_drag: None,
            extra_lanes: Vec::new(),
            extra_drag: None,
            step_input: false,
            step_at: 0.0,
            step_chord: None,
            step_len: 0.25,
            time_sel: None,
            time_anchor: None,
            groove: None,
            tools: crate::note_tools::ToolSettings::default(),
            tool: Tool::Draw,
            held: None,
            fit: true,
            vel_drag: None,
            release_at: None,
            beat_px: 64.0,
            key_h: 12.0,
            scroll_x: 0.0,
            scroll_y: 0.0,
            selected: Vec::new(),
            grid: 3,
            strength: 1.0,
            swing: 0.0,
            scale_root: None,
            scale_mode: 0,
            last_len: 0.25,
            new_velocity: 100,
            color_group: 0,
            slide_mode: false,
            slide_range_edit: None,
            clipboard: Vec::new(),
            drag: Drag::None,
            centered_for: None,
            dirty: false,
            ghosts: false,
            last_grid: Rect::NOTHING,
        }
    }
}

/// Every note of the MIDI regions other than region `ri` of track `ti`, with where it is.
fn ghost_notes(p: &Project, ti: usize, ri: usize) -> Vec<(usize, usize, Note)> {
    let mut all = Vec::new();
    for (t, track) in p.tracks.iter().enumerate() {
        for (r, region) in track.regions.iter().enumerate() {
            if (t, r) != (ti, ri) && region.audio.is_none() {
                all.extend(region.notes.iter().map(|n| (t, r, *n)));
            }
        }
    }
    all
}

/// Starts, switches or stops the audition note; `pitch = None` releases.
fn audition(
    out: &mut Vec<Audition>,
    held: &mut Option<u8>,
    pitch: Option<u8>,
    vel: u8,
    track: u16,
) {
    if *held == pitch {
        return;
    }
    if let Some(old) = *held {
        out.push(Audition::Off { pitch: old });
    }
    if let Some(pitch) = pitch {
        out.push(Audition::On { pitch, vel, track });
    }
    *held = pitch;
}

/// The bars that start between `from` and `to` (beats from the region's start `origin`), as
/// (beats from `origin`, 0-based bar number).
fn bar_starts(p: &Project, origin: f64, from: f32, to: f32) -> Vec<(f64, u32)> {
    p.bars_in(origin + from as f64, origin + to as f64)
        .into_iter()
        .map(|b| (b.start - origin, b.bar0))
        .filter(|(rel, _)| *rel >= from as f64 - 1e-9)
        .collect()
}

/// Snap step in beats for the Piano Roll. Smart follows the zoom, like Logic's; when zoomed
/// out it falls back to the signature's beat (an eighth in 6/8).
fn piano_step(snap: Snap, beat_px: f32, beats_per_bar: f64, beat_unit: f64) -> f32 {
    match snap {
        Snap::Smart => match beat_px {
            x if x >= 160.0 => 0.0625,
            x if x >= 80.0 => 0.125,
            x if x >= 24.0 => 0.25,
            _ => beat_unit as f32,
        },
        other => snap_step(other, beat_px, beats_per_bar, beat_unit) as f32,
    }
}

fn id(key: impl std::hash::Hash + std::fmt::Debug) -> egui::Id {
    egui::Id::unique(("piano_roll", key))
}

fn is_black(pitch: i32) -> bool {
    matches!(pitch.rem_euclid(12), 1 | 3 | 6 | 8 | 10)
}

/// Logic octave naming: pitch 60 is C3.
fn c_label(pitch: i32) -> String {
    format!("C{}", pitch / 12 + crate::settings::octave_naming())
}

/// The region the editor shows: the selected MIDI region, else the first one of the track.
pub(crate) fn target_region(p: &Project, s: &UiState) -> Option<(usize, usize)> {
    if let Some((t, r)) = s.tracks.selected {
        if p.tracks
            .get(t)
            .is_some_and(|tr| tr.kind != TrackKind::Audio && r < tr.regions.len())
        {
            return Some((t, r));
        }
    }
    let t = p.selected_track.min(p.tracks.len().checked_sub(1)?);
    let tr = &p.tracks[t];
    (tr.kind != TrackKind::Audio && !tr.regions.is_empty()).then_some((t, 0))
}

/// Moves the editor to a track of the other kind: an audio track for the Audio Editor, else an
/// instrument track: the one whose region is selected, else the first.
fn switch_track(p: &mut Project, s: &mut UiState, audio: bool) {
    let fits = |k: TrackKind| if audio { k == TrackKind::Audio } else { k == TrackKind::Instrument };
    let Some(t) = p.tracks.iter().position(|t| fits(t.kind) && !t.regions.is_empty())
        .or_else(|| p.tracks.iter().position(|t| fits(t.kind)))
    else {
        s.status = Some(if audio { "No audio track yet".to_string() } else { "No instrument track yet".to_string() });
        return;
    };
    p.selected_track = t;
    s.tracks.selected = (!p.tracks[t].regions.is_empty()).then_some((t, 0));
}

/// The editor tabs; the one for the region on show is lit, the rest are not there yet.
/// Returns the tab clicked (the Piano Roll and the Score switch when `switch` allows).
fn tabs(ui: &Ui, g: &Painter, bar: Rect, active: usize, switch: bool) -> Option<usize> {
    let mut clicked = None;
    g.rect_filled(bar, 0.0, panel_bg_col());
    let tabs = [
        tr("Piano Roll"),
        tr("Score"),
        tr("Step Sequencer"),
        tr("Drummer Editor"),
        tr("Smart Tempo"),
        "Audio Editor",
    ];
    let font = FontId::proportional(12.5);
    let widths: Vec<f32> = tabs
        .iter()
        .map(|t| g.layout_no_wrap((*t).into(), font.clone(), text_col()).size().x + 28.0)
        .collect();
    let total: f32 = widths.iter().sum();
    let mut x = bar.center().x - total / 2.0;
    for (i, (t, w)) in tabs.iter().zip(&widths).enumerate() {
        let r = Rect::from_min_size(pos2(x, bar.top() + 3.0), vec2(*w, TAB_H - 6.0));
        // The Piano Roll and the Audio Editor can always be picked: they move to a track of
        // their kind. The Score switches with the Piano Roll when it can.
        let live = i == 0 || i == AUDIO_TAB || (switch && i == 1);
        if live {
            let resp = ui.interact(r, egui::Id::unique(("editor_tab", i)), Sense::click());
            if resp.hovered() && i != active {
                g.rect_filled(r, 4.0, Color32::from_white_alpha(14));
            }
            if resp.clicked() {
                clicked = Some(i);
            }
        }
        if i == active {
            g.rect_filled(r, 4.0, blue());
            g.text(
                r.center(),
                Align2::CENTER_CENTER,
                *t,
                font.clone(),
                Color32::WHITE,
            );
        } else {
            // The editors that are not there yet are dimmed and inert.
            g.text(
                r.center(),
                Align2::CENTER_CENTER,
                *t,
                font.clone(),
                if live { text_col() } else { crate::theme::gray(0x4c) },
            );
        }
        x += *w;
    }
    clicked
}

struct Actions {
    delete: bool,
    select_all: bool,
    quantize: bool,
    scale: bool,
    flip_time: bool,
    flip_pitch: bool,
    same_pitch: bool,
    same_start: bool,
    /// Open the MIDI Transform window, with this preset (or as it was left).
    transform: Option<Option<crate::transform::Preset>>,
    /// Whether there is something to undo / redo, and the one picked in the Edit menu.
    can_undo: bool,
    can_redo: bool,
    undo: Option<crate::undo::UndoCmd>,
    toggle_typing: bool,
    /// There is a time selection, and what was asked of it.
    has_time: bool,
    delete_time: bool,
    insert_time: bool,
    loop_time: bool,
    /// Make a groove template of this region, or delete the one in use.
    make_groove: bool,
    delete_groove: bool,
}

/// What an edit with no notes picked works on: the selection, or all the notes when nothing is
/// picked. A time selection with no notes in it means no notes, never all of them.
fn targets_of(selected: &[usize], count: usize, time_sel: bool) -> Vec<usize> {
    if selected.is_empty() && !time_sel {
        (0..count).collect()
    } else {
        selected.to_vec()
    }
}

fn toolbar(
    ui: &Ui,
    bar: Rect,
    v: &mut PianoView,
    has_sel: bool,
    snap: &mut Snap,
    act: &mut Actions,
) {
    const NS: &str = "piano_toolbar";
    let g = ui.painter_at(bar);
    g.rect_filled(bar, 0.0, toolbar_bg_col());
    g.line_segment(
        [bar.left_bottom(), bar.right_bottom()],
        Stroke::new(1.0, crate::theme::border()),
    );
    let cy = bar.center().y;
    back_chevron(&g, pos2(bar.left() + 16.0, cy));
    let mut x = bar.left() + 32.0;
    let uid = |k: &str| egui::Id::unique((NS, k.to_string()));

    let (edit, w) = menu_title(ui, &g, x, cy, tr("Edit"), uid("Edit"));
    x += w + 2.0;
    egui::Popup::menu(&edit).show(|ui| {
        ui.set_min_width(230.0);
        crate::widgets::undo_items(ui, act.can_undo, act.can_redo, &mut act.undo);
        ui.separator();
        if ui
            .add_enabled(
                has_sel,
                egui::Button::new(tr("Delete")).shortcut_text(tr("Del")),
            )
            .clicked()
        {
            act.delete = true;
            ui.close();
        }
        let sc = ui.ctx().format_shortcut(&egui::KeyboardShortcut::new(
            egui::Modifiers::COMMAND,
            Key::A,
        ));
        if ui
            .add(egui::Button::new(tr("Select All")).shortcut_text(sc))
            .clicked()
        {
            act.select_all = true;
            ui.close();
        }
        ui.separator();
        if ui
            .add(egui::Button::new(tr("Quantize")).shortcut_text("Q"))
            .clicked()
        {
            act.quantize = true;
            ui.close();
        }
    });
    let (func, w) = menu_title(ui, &g, x, cy, tr("Functions"), uid("Functions"));
    x += w + 2.0;
    egui::Popup::menu(&func).show(|ui| {
        ui.set_min_width(230.0);
        if ui
            .add_enabled(
                v.scale_root.is_some(),
                egui::Button::new(tr("Scale Quantize")),
            )
            .clicked()
        {
            act.scale = true;
            ui.close();
        }
        ui.separator();
        use crate::transform::Preset;
        for (label, preset) in [
            (tr("MIDI Transform…"), None),
            (tr("Transpose…"), Some(Preset::Transpose)),
            (tr("Legato"), Some(Preset::Legato)),
            (tr("Humanize…"), Some(Preset::Humanize)),
            (tr("Fixed Velocity…"), Some(Preset::FixedVelocity)),
        ] {
            if ui.button(label).clicked() {
                act.transform = Some(preset);
                ui.close();
            }
        }
        ui.separator();
        use crate::note_tools::NoteTool;
        for tool in NoteTool::ALL {
            // This name has its translations.
            let name = if tool == NoteTool::NoteRepeat {
                tr("Note Repeat…")
            } else {
                tool.name()
            };
            if ui.button(name).clicked() {
                v.tools.open = Some(tool);
                ui.close();
            }
        }
        ui.separator();
        if ui.button("Flip in Time").clicked() {
            act.flip_time = true;
            ui.close();
        }
        let mut on = v.step_input;
        if ui
            .checkbox(&mut on, "Step Input")
            .on_hover_text(
                "Notes from a MIDI keyboard go in at the cursor one grid step at a time; a chord goes in \
                 together. Right Arrow skips a step (a rest), Left Arrow steps back, Backspace takes \
                 back the last step.",
            )
            .changed()
        {
            v.step_input = on;
            v.step_chord = None;
        }
        ui.separator();
        if ui.button("Flip in Pitch").clicked() {
            act.flip_pitch = true;
            ui.close();
        }
        ui.separator();
        if ui.button("Select Same Pitch").clicked() {
            act.same_pitch = true;
            ui.close();
        }
        if ui.button("Select Same Position").clicked() {
            act.same_start = true;
            ui.close();
        }
        ui.separator();
        for (text, key, flag) in [
            ("Delete Time", "Ctrl+Delete", &mut act.delete_time),
            ("Insert Time", "Ctrl+Insert", &mut act.insert_time),
            ("Set Locators to Time Selection", "", &mut act.loop_time),
        ] {
            let button = egui::Button::new(text).shortcut_text(key);
            if ui
                .add_enabled(act.has_time, button)
                .on_disabled_hover_text("Drag the ruler with the right button first")
                .clicked()
            {
                *flag = true;
                ui.close();
            }
        }
    });
    let (view, w) = menu_title(ui, &g, x, cy, tr("View"), uid("View"));
    x += w + 14.0;
    egui::Popup::menu(&view).show(|ui| {
        ui.set_min_width(230.0);
        if ui.button(tr("Zoom In")).clicked() {
            v.beat_px = (v.beat_px * 1.25).min(400.0);
            ui.close();
        }
        if ui.button(tr("Zoom Out")).clicked() {
            v.beat_px = (v.beat_px / 1.25).max(12.0);
            ui.close();
        }
        ui.separator();
        if ui.button(tr("Zoom to Fit")).clicked() {
            v.fit = true;
            ui.close();
        }
        ui.menu_button(tr("Note Colors"), |ui| {
            for (colors, text) in [
                (NoteColors::Velocity, "By Velocity"),
                (NoteColors::Track, "By Track"),
                (NoteColors::Pitch, "By Pitch"),
                (NoteColors::Group, "By Color Group"),
            ] {
                if ui.selectable_label(v.note_colors == colors, text).clicked() {
                    v.note_colors = colors;
                    ui.close();
                }
            }
        });
    });

    // Tool buttons, with their keys.
    for (i, tool) in Tool::ALL.into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(x + i as f32 * 28.0, cy - 10.0), vec2(26.0, 20.0));
        let resp = ui
            .interact(r, uid(tool.hint()), Sense::click())
            .on_hover_text(tool.hint());
        let on = v.tool == tool;
        if on {
            g.rect_filled(r, 4.0, blue());
        } else if resp.hovered() {
            g.rect_filled(r, 4.0, Color32::from_white_alpha(14));
        }
        let col = if on { crate::theme::on_accent() } else { text_col() };
        crate::icons::paint(&g, r.center(), 15.0, tool.icon(), col);
        if resp.clicked() {
            v.tool = tool;
        }
    }

    // Ghost notes. (Typing to MIDI, Ctrl+K, is on the control bar.)
    let extras_x = x + Tool::ALL.len() as f32 * 28.0 + 8.0;
    let ghost_r = Rect::from_min_size(pos2(extras_x, cy - 10.0), vec2(26.0, 20.0));
    if toggle_button(ui, &g, ghost_r, uid("ghosts"), Icon::Ghost, v.ghosts)
        .on_hover_text("Show the notes of the other regions behind these (Alt+V)")
        .clicked()
    {
        v.ghosts = !v.ghosts;
    }

    // Right side: zoom sliders, then Snap.
    let (lo, hi) = (12f32.ln(), 400f32.ln());
    let hz = Rect::from_min_size(pos2(bar.right() - 76.0, cy - 6.0), vec2(64.0, 12.0));
    let t = ((v.beat_px.ln() - lo) / (hi - lo)).clamp(0.0, 1.0);
    if let Some(n) = slider_h(ui, &g, hz, uid("hz"), t, true) {
        v.beat_px = (lo + n * (hi - lo)).exp();
    }
    let vz = Rect::from_min_size(hz.min - vec2(86.0, 0.0), vec2(54.0, 12.0));
    let t = (v.key_h - 6.0) / 18.0;
    if let Some(n) = slider_h(ui, &g, vz, uid("vz"), t, true) {
        v.key_h = 6.0 + n * 18.0;
    }
    let snap_r = Rect::from_min_size(pos2(vz.left() - 190.0, cy - 10.0), vec2(150.0, 20.0));
    let resp = popup_button(ui, &g, snap_r, uid("snap"), snap.label());
    egui::Popup::menu(&resp).show(|ui| {
        for m in Snap::ALL {
            if ui.selectable_label(*snap == m, m.label()).clicked() {
                *snap = m;
                ui.close();
            }
        }
    });
    g.text(
        pos2(snap_r.left() - 6.0, cy),
        Align2::RIGHT_CENTER,
        tr("Snap:"),
        FontId::proportional(12.5),
        text_dim_col(),
    );
}

/// A small button that stays lit while its setting is on.
fn toggle_button(
    ui: &Ui,
    g: &Painter,
    r: Rect,
    id: egui::Id,
    icon: Icon,
    on: bool,
) -> egui::Response {
    let resp = ui.interact(r, id, Sense::click());
    if on {
        g.rect_filled(r, 4.0, blue());
    } else {
        g.rect_filled(
            r,
            4.0,
            if resp.hovered() {
                Color32::from_white_alpha(14)
            } else {
                btn_bg_col()
            },
        );
    }
    crate::icons::paint(g, r.center(), 15.0, icon, if on { crate::theme::on_accent() } else { text_col() });
    resp
}

fn small_q(ui: &Ui, g: &Painter, r: Rect, key: &str, enabled: bool) -> bool {
    let resp = enabled.then(|| ui.interact(r, id(("q", key)), Sense::click()));
    let hovered = resp.as_ref().is_some_and(|r| r.hovered());
    g.rect_filled(
        r,
        4.0,
        if hovered {
            crate::theme::gray(0x34)
        } else {
            btn_bg_col()
        },
    );
    g.text(
        r.center(),
        Align2::CENTER_CENTER,
        "Q",
        FontId::proportional(12.0),
        if enabled {
            text_col()
        } else {
            crate::theme::gray(0x40)
        },
    );
    resp.is_some_and(|r| r.clicked())
}

/// The slide button and key: makes the selected notes slide, or not if they all do. With none
/// selected it says whether new notes are slide notes.
fn set_slide(notes: &mut [Note], selected: &[usize], mode: &mut bool) {
    if selected.is_empty() {
        *mode = !*mode;
        return;
    }
    let on = !selected.iter().all(|&i| notes[i].slide);
    for &i in selected {
        notes[i].slide = on;
    }
}

pub fn show(ui: &mut Ui, p: &mut Project, s: &mut UiState) {
    let area = ui.available_rect_before_wrap();
    ui.allocate_rect(area, Sense::hover());
    let g = ui.painter_at(area);
    g.rect_filled(area, 0.0, panel_bg_col());
    let tab_bar = Rect::from_min_size(area.min, vec2(area.width(), TAB_H));
    // An audio track gets the Audio Track Editor instead (Logic shows it for audio regions).
    if crate::audio_track_editor::wants(p) {
        // It has no keys of its own: every key command works over it.
        s.piano_focus = false;
        if tabs(ui, &g, tab_bar, AUDIO_TAB, false) == Some(0) {
            switch_track(p, s, false);
            return;
        }
        let body = Rect::from_min_max(pos2(area.left(), tab_bar.bottom()), area.max);
        crate::audio_track_editor::show(ui, p, s, body);
        return;
    }
    // An instrument track with nothing on it gets a region, so there is always a grid to draw on.
    if target_region(p, s).is_none() {
        let ti = p.selected_track;
        let bar = p.beats_to_bar(p.position_beats).floor() as f32 + 1.0;
        if let Some(ri) = p.add_midi_region(ti, bar) {
            s.tracks.selected = Some((ti, ri));
        }
    }
    let region = target_region(p, s);
    // A Pattern or Drummer region has its own editor.
    let generated = region.and_then(|(t, r)| {
        p.tracks[t].regions[r]
            .generated
            .as_ref()
            .map(|g| matches!(g.source, crate::model::Generator::Drummer(_)))
    });
    // Ordinary MIDI regions can be shown as a score as well.
    let ordinary =
        region.is_some_and(|(t, r)| generated.is_none() && p.tracks[t].regions[r].folder.is_none());
    let active = match generated {
        None if ordinary && s.score_tab => 1,
        None => 0,
        Some(false) => 2,
        Some(true) => 3,
    };
    match tabs(ui, &g, tab_bar, active, ordinary) {
        Some(AUDIO_TAB) => {
            switch_track(p, s, true);
            return;
        }
        Some(i) => s.score_tab = i == 1,
        None => {}
    }
    if let (Some((ti, ri)), true) = (region, active == 1) {
        s.piano_focus = ui
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|pt| area.contains(pt));
        let body = Rect::from_min_max(pos2(area.left(), tab_bar.bottom()), area.max);
        crate::score::show(ui, p, s, ti, ri, body);
        return;
    }
    if let (Some((ti, ri)), Some(_)) = (region, generated) {
        s.piano_focus = ui
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|pt| area.contains(pt));
        let body = Rect::from_min_max(pos2(area.left(), tab_bar.bottom()), area.max);
        crate::step_editor::show(ui, p, s, ti, ri, body);
        return;
    }
    let bar = Rect::from_min_size(
        pos2(area.left(), tab_bar.bottom()),
        vec2(area.width(), TOOLBAR_H),
    );
    let body = Rect::from_min_max(pos2(area.left(), bar.bottom()), area.max);
    let hovered = ui
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|pt| area.contains(pt));
    s.piano_focus = hovered;

    let has_sel = !s.piano.selected.is_empty();
    let mut act = Actions {
        delete: false,
        select_all: false,
        quantize: false,
        scale: false,
        flip_time: false,
        flip_pitch: false,
        same_pitch: false,
        same_start: false,
        transform: None,
        can_undo: s.can_undo,
        can_redo: s.can_redo,
        undo: None,
        toggle_typing: false,
        has_time: s.piano.time_sel.is_some(),
        delete_time: false,
        insert_time: false,
        loop_time: false,
        make_groove: false,
        delete_groove: false,
    };
    let mut snap = s.tracks.snap;
    toolbar(ui, bar, &mut s.piano, has_sel, &mut snap, &mut act);
    if act.toggle_typing {
        s.typing.on = !s.typing.on;
    }
    let typing_on = s.typing.on;
    s.tracks.snap = snap;
    if let Some(preset) = act.transform {
        s.transform.open_with(preset);
    }
    if act.undo.is_some() {
        s.undo_cmd = act.undo;
    }

    let Some((ti, ri)) = region else {
        // An audio track has no notes. One click makes a track that does.
        g.text(
            body.center() - vec2(0.0, 18.0),
            Align2::CENTER_CENTER,
            "This track holds audio. Notes go on an instrument track.",
            FontId::proportional(13.0),
            text_dim_col(),
        );
        let button = Rect::from_center_size(body.center() + vec2(0.0, 10.0), vec2(190.0, 24.0));
        if ui
            .put(button, egui::Button::new("New instrument track"))
            .clicked()
        {
            let ti = p.add_track(TrackKind::Instrument);
            let bar = p.beats_to_bar(p.position_beats).floor() as f32 + 1.0;
            if let Some(ri) = p.add_midi_region(ti, bar) {
                s.tracks.selected = Some((ti, ri));
            }
        }
        return;
    };
    // A take folder's notes are its comp: they are edited after it is flattened.
    if p.tracks[ti].regions[ri].folder.is_some() {
        g.text(
            body.center(),
            Align2::CENTER_CENTER,
            tr("This is a take folder: flatten it to edit its notes"),
            FontId::proportional(13.0),
            text_dim_col(),
        );
        return;
    }
    let origin = p.region_start_beats(&p.tracks[ti].regions[ri]);
    // The bar and beat length where the region starts (the grid draws every bar on its own).
    let bpb = p.bar_len_at(origin);
    let groove_offset = crate::groove_template::bar_offset(p, origin) as f32;
    let unit = p.beat_unit_at(origin);
    let (region_name, start_bar, len_beats) = {
        let r = &p.tracks[ti].regions[ri];
        (r.name.clone(), r.start_bar, p.region_len_beats(r) as f32)
    };
    let track_name = p.tracks[ti].name.clone();
    let region_color = region_bar_color(p.tracks[ti].color);
    let position = p.position_beats - origin;

    // Layout.
    let insp = Rect::from_min_size(body.min, vec2(INSPECTOR_W, body.height()));
    let editor = Rect::from_min_max(pos2(insp.right(), body.top()), body.max);
    let extra_h = s.piano.extra_lanes.len() as f32 * (EXTRA_H + 3.0);
    let lower = editor.bottom() - SB_H - VEL_H - 4.0 - extra_h;
    let keys = Rect::from_min_max(
        pos2(editor.left(), editor.top() + RULER_H + REGION_H),
        pos2(editor.left() + KEYS_W, lower),
    );
    let grid = Rect::from_min_max(
        pos2(keys.right(), keys.top()),
        pos2(editor.right() - SB_H, lower),
    );
    let hbar = Rect::from_min_max(
        pos2(grid.left(), grid.bottom()),
        pos2(grid.right(), grid.bottom() + SB_H),
    );
    let vbar = Rect::from_min_max(
        pos2(grid.right(), grid.top()),
        pos2(editor.right(), grid.bottom()),
    );
    let vel_rect = Rect::from_min_max(
        pos2(grid.left(), hbar.bottom() + 4.0 + extra_h),
        pos2(grid.right(), editor.bottom()),
    );
    let vel_label = Rect::from_min_max(
        pos2(keys.left(), vel_rect.top()),
        pos2(keys.right(), vel_rect.bottom()),
    );
    let ruler = Rect::from_min_max(
        pos2(grid.left(), editor.top()),
        pos2(editor.right(), editor.top() + RULER_H),
    );
    let region_bar = Rect::from_min_max(
        pos2(grid.left(), ruler.bottom()),
        pos2(editor.right(), grid.top()),
    );

    let v = &mut s.piano;
    let (down, now) = ui.input(|i| (i.pointer.primary_down(), i.time));
    v.last_grid = grid;
    if v.held.is_some() && !down {
        match v.release_at {
            Some(t) if now < t => ui
                .ctx()
                .request_repaint_after(std::time::Duration::from_millis(30)),
            _ => {
                audition(&mut s.audition, &mut v.held, None, 0, 0);
                v.release_at = None;
            }
        }
    }
    let track_u8 = ti.min(u16::MAX as usize) as u16;
    let notes_len = p.tracks[ti].regions[ri].notes.len();
    v.selected.retain(|i| *i < notes_len);

    // Centre on the notes when a different region is opened.
    if v.centered_for != Some((ti, ri)) {
        v.centered_for = Some((ti, ri));
        v.fit = true;
        let notes = &p.tracks[ti].regions[ri].notes;
        let (lo, hi) = notes
            .iter()
            .fold((127u8, 0u8), |(a, b), n| (a.min(n.pitch), b.max(n.pitch)));
        let mid = if notes.is_empty() {
            60.0
        } else {
            (lo as f32 + hi as f32) / 2.0
        };
        v.scroll_y = (127.0 - mid) * v.key_h - grid.height() / 2.0;
        v.scroll_x = 0.0;
        v.selected.clear();
        v.time_sel = None;
        v.time_anchor = None;
        v.lane_drag = None;
    }

    if v.fit {
        v.fit = false;
        v.beat_px = ((grid.width() - 16.0) / len_beats.max(1.0)).clamp(12.0, 400.0);
        v.scroll_x = 0.0;
    }

    // Wheel: vertical scroll; Shift = horizontal; Ctrl/Cmd = horizontal zoom.
    if crate::widgets::wheel_pointer(ui).is_some_and(|pt| editor.contains(pt)) {
        let (d, m, pinch) = ui.input(|i| (i.smooth_scroll_delta, i.modifiers, i.zoom_delta()));
        if let Some(factor) = crate::widgets::ctrl_wheel_zoom(m.command, d.y, pinch) {
            v.beat_px = (v.beat_px * factor).clamp(12.0, 400.0);
        } else if m.shift {
            v.scroll_x -= d.y + d.x;
        } else {
            v.scroll_x -= d.x;
            v.scroll_y -= d.y;
        }
    }
    v.scroll_y = v
        .scroll_y
        .clamp(0.0, (128.0 * v.key_h - grid.height()).max(0.0));
    let content_w = ((len_beats + 2.0) * v.beat_px).max(grid.width());
    let content_h = 128.0 * v.key_h;
    {
        let gs = ui.painter_at(Rect::from_min_max(hbar.min, vbar.max));
        gs.rect_filled(hbar, 0.0, crate::theme::gray(0x17));
        gs.rect_filled(vbar, 0.0, crate::theme::gray(0x17));
        let tw = hbar.width();
        let thumb_w = (tw * grid.width() / content_w).clamp(24.0, tw);
        let max_x = (content_w - grid.width()).max(1.0);
        let thumb_x = hbar.left() + (tw - thumb_w) * (v.scroll_x / max_x).clamp(0.0, 1.0);
        let thumb = Rect::from_min_size(
            pos2(thumb_x, hbar.top() + 2.0),
            vec2(thumb_w, hbar.height() - 4.0),
        );
        let r = ui.interact(thumb, id("hthumb"), Sense::drag());
        if r.dragged() {
            v.scroll_x += r.drag_delta().x * max_x / (tw - thumb_w).max(1.0);
        }
        gs.rect_filled(
            thumb,
            4.0,
            crate::theme::gray(if r.hovered() || r.dragged() {
                0x88
            } else {
                0x66
            }),
        );

        let th = vbar.height();
        let thumb_h = (th * grid.height() / content_h).clamp(24.0, th);
        let max_y = (content_h - grid.height()).max(1.0);
        let thumb_y = vbar.top() + (th - thumb_h) * (v.scroll_y / max_y).clamp(0.0, 1.0);
        let thumb = Rect::from_min_size(
            pos2(vbar.left() + 2.0, thumb_y),
            vec2(vbar.width() - 4.0, thumb_h),
        );
        let r = ui.interact(thumb, id("vthumb"), Sense::drag());
        if r.dragged() {
            v.scroll_y += r.drag_delta().y * max_y / (th - thumb_h).max(1.0);
        }
        gs.rect_filled(
            thumb,
            4.0,
            crate::theme::gray(if r.hovered() || r.dragged() {
                0x88
            } else {
                0x66
            }),
        );
    }
    v.scroll_y = v.scroll_y.clamp(0.0, (content_h - grid.height()).max(0.0));
    v.scroll_x = v.scroll_x.clamp(0.0, (content_w - grid.width()).max(0.0));
    let (beat_px, key_h, scroll_x, scroll_y) = (v.beat_px, v.key_h, v.scroll_x, v.scroll_y);
    let x_of = |beat: f32| grid.left() + beat * beat_px - scroll_x;
    let y_of = |pitch: i32| grid.top() + (127 - pitch) as f32 * key_h - scroll_y;
    let beat_at = |x: f32| (x - grid.left() + scroll_x) / beat_px;
    let pitch_at =
        |y: f32| (127 - ((y - grid.top() + scroll_y) / key_h).floor() as i32).clamp(0, 127);
    let step = piano_step(s.tracks.snap, beat_px, bpb, unit);
    v.step_len = if step > 0.0 { step } else { 0.25 };
    let bars = bar_starts(
        p,
        origin,
        scroll_x / beat_px,
        (scroll_x + grid.width()) / beat_px,
    );

    // ---- Grid background
    let gg = ui.painter_at(grid);
    gg.rect_filled(grid, 0.0, row_white_col());
    let top_pitch = pitch_at(grid.top());
    let bottom_pitch = pitch_at(grid.bottom());
    for pitch in bottom_pitch..=top_pitch {
        let y = y_of(pitch);
        let row = Rect::from_min_size(pos2(grid.left(), y), vec2(grid.width(), key_h));
        gg.rect_filled(
            row,
            0.0,
            if is_black(pitch) {
                row_black_col()
            } else {
                row_white_col()
            },
        );
        // The notes of the scale picked for Scale Quantize stand out.
        let in_scale = v.scale_root.is_some_and(|root| {
            let degree = (pitch - i32::from(root)).rem_euclid(12) as u8;
            MODES[v.scale_mode].1.contains(&degree)
        });
        if in_scale {
            gg.rect_filled(row, 0.0, blue().gamma_multiply(0.14));
        }
        if pitch % 12 == 0 {
            gg.line_segment(
                [row.left_bottom(), row.right_bottom()],
                Stroke::new(1.0, grid_ink(0.18)),
            );
        }
    }
    let first = (scroll_x / beat_px).floor() as i32;
    let last = ((scroll_x + grid.width()) / beat_px).ceil() as i32;
    let sub = (step < 1.0 && step * beat_px >= 8.0).then_some(step);
    for b in first..=last {
        let bx = x_of(b as f32);
        gg.line_segment(
            [pos2(bx, grid.top()), pos2(bx, grid.bottom())],
            Stroke::new(1.0, grid_ink(0.11)),
        );
        if let Some(st) = sub {
            let mut k = st;
            while k < 1.0 {
                let sx = x_of(b as f32 + k);
                gg.line_segment(
                    [pos2(sx, grid.top()), pos2(sx, grid.bottom())],
                    Stroke::new(1.0, grid_ink(0.06)),
                );
                k += st;
            }
        }
    }
    for &(rel, _) in &bars {
        let bx = x_of(rel as f32);
        gg.line_segment(
            [pos2(bx, grid.top()), pos2(bx, grid.bottom())],
            Stroke::new(1.0, grid_ink(0.26)),
        );
    }
    // Outside the region: dim.
    let (rx0, rx1) = (x_of(0.0), x_of(len_beats));
    if rx0 > grid.left() {
        gg.rect_filled(
            Rect::from_min_max(grid.min, pos2(rx0.min(grid.right()), grid.bottom())),
            0.0,
            Color32::from_black_alpha(70),
        );
    }
    if rx1 < grid.right() {
        gg.rect_filled(
            Rect::from_min_max(pos2(rx1.max(grid.left()), grid.top()), grid.max),
            0.0,
            Color32::from_black_alpha(70),
        );
    }

    // ---- Piano keys: press to play, drag to glissando
    let kresp = ui.interact(keys, id("keys"), Sense::click_and_drag());
    if kresp.is_pointer_button_down_on() {
        if let Some(pt) = kresp.interact_pointer_pos() {
            audition(
                &mut s.audition,
                &mut v.held,
                Some(pitch_at(pt.y) as u8),
                100,
                track_u8,
            );
            v.release_at = None;
        }
    }

    let p_position = p.position_beats;
    // ---- Interaction, as in FL Studio's piano roll: things happen on the press, not on letting go.
    let resp = ui.interact(grid, id("grid"), Sense::click_and_drag());
    let (shift, ctrl, alt) =
        ui.input(|i| (i.modifiers.shift, i.modifiers.command, i.modifiers.alt));
    // Alt turns the snap off for as long as it is held.
    let snap = if alt { 1.0 / 64.0 } else { step };
    let edge_beats = 10.0 / beat_px;
    if let Some(range) = v.slide_range_edit.take() {
        p.tracks[ti].slide_range = range;
        p.song_rev += 1;
    }
    let slide_range = p.tracks[ti].slide_range;
    let shown = &mut p.tracks[ti].regions[ri];
    let (notes, controls) = (&mut shown.notes, &mut shown.controls);
    let (primary_press, secondary_press, hover, down_now, secondary_down) = ui.input(|i| {
        (
            i.pointer.button_pressed(egui::PointerButton::Primary),
            i.pointer.button_pressed(egui::PointerButton::Secondary),
            i.pointer.hover_pos(),
            i.pointer.primary_down(),
            i.pointer.secondary_down(),
        )
    });
    if let Some(pt) = hover.filter(|pt| grid.contains(*pt) && resp.hovered()) {
        let (beat, pitch) = (beat_at(pt.x), pitch_at(pt.y));
        let over_note = note_edit::hit_note(notes, beat, pitch as u8, edge_beats);
        let resizes = matches!(v.tool, Tool::Draw | Tool::Select | Tool::Paint);
        if (resizes && over_note.is_some_and(|(_, on_edge)| on_edge))
            || matches!(v.drag, Drag::Resize { .. })
        {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if secondary_press {
            v.drag = Drag::Erase;
        } else if primary_press {
            let hit = over_note;
            // Ctrl selects, whatever the tool that draws: a box, or the note under the pointer.
            if ctrl && matches!(v.tool, Tool::Draw | Tool::Paint) {
                let base = hit.map(|(i, _)| vec![i]).unwrap_or_default();
                v.selected = base.clone();
                v.drag = Drag::Marquee { from: pt, base };
            } else {
                match (v.tool, hit) {
                    (Tool::Delete, _) => v.drag = Drag::Erase,
                    (Tool::Mute, Some((i, _))) => {
                        let on = !notes[i].muted;
                        v.drag = Drag::Mute {
                            on,
                            done: Vec::new(),
                        };
                    }
                    (Tool::Mute, None) => {}
                    (Tool::Slice, _) => {
                        v.drag = Drag::Slice {
                            beat: snap_to(beat as f64, snap as f64) as f32,
                        };
                    }
                    (Tool::Zoom, _) => v.drag = Drag::Zoom { from: pt },
                    (Tool::Playback, _) => {
                        v.drag = Drag::Scrub;
                        let under = note_edit::hit_note(notes, beat, pitch as u8, 0.0)
                            .map(|(i, _)| notes[i].pitch);
                        audition(
                            &mut s.audition,
                            &mut v.held,
                            under,
                            v.new_velocity,
                            track_u8,
                        );
                        v.release_at = None;
                    }
                    (Tool::Select, None) => {
                        let base = if ctrl { v.selected.clone() } else { Vec::new() };
                        v.selected = base.clone();
                        v.drag = Drag::Marquee { from: pt, base };
                    }
                    (Tool::Paint, None) if beat >= 0.0 => {
                        let start = (beat / snap).floor() * snap;
                        v.selected.clear();
                        v.drag = Drag::Paint {
                            pitch: pitch as u8,
                            start,
                        };
                        audition(
                            &mut s.audition,
                            &mut v.held,
                            Some(pitch as u8),
                            v.new_velocity,
                            track_u8,
                        );
                        v.release_at = None;
                    }
                    (Tool::Draw, None) if beat >= 0.0 => {
                        // A new note with the length and velocity of the last one, which moves
                        // with the pointer until let go.
                        let start = (beat / snap).floor() * snap;
                        notes.push(Note {
                            start,
                            len: v.last_len.max(1.0 / 64.0),
                            pitch: pitch as u8,
                            vel: v.new_velocity,
                            muted: false,
                            color: v.color_group,
                            slide: v.slide_mode,
                        });
                        let index = notes.len() - 1;
                        v.selected.clear();
                        v.drag = Drag::Move {
                            anchor_beat: beat,
                            anchor_pitch: pitch,
                            orig: vec![(index, start, pitch as u8)],
                        };
                        audition(
                            &mut s.audition,
                            &mut v.held,
                            Some(pitch as u8),
                            v.new_velocity,
                            track_u8,
                        );
                        v.release_at = None;
                        v.dirty = true;
                    }
                    (Tool::Draw | Tool::Paint | Tool::Select, Some((i, on_edge))) => {
                        // A click on a note makes it the one new notes copy.
                        v.last_len = notes[i].len;
                        v.new_velocity = notes[i].vel;
                        let group = v.selected.contains(&i) && v.tool != Tool::Paint;
                        if !group {
                            if v.tool == Tool::Select && ctrl {
                                v.selected.push(i);
                            } else if v.tool == Tool::Select {
                                v.selected = vec![i];
                            } else {
                                v.selected.clear();
                            }
                        }
                        let mut moving = if group || (v.tool == Tool::Select) {
                            v.selected.clone()
                        } else {
                            vec![i]
                        };
                        if on_edge {
                            v.drag = Drag::Resize {
                                anchor_beat: beat,
                                orig: moving.iter().map(|&k| (k, notes[k].len)).collect(),
                            };
                        } else {
                            // Shift drags copies and leaves the originals.
                            if shift {
                                let clip: Vec<Note> = moving.iter().map(|&k| notes[k]).collect();
                                let first = notes.len();
                                notes.extend(clip);
                                moving = (first..notes.len()).collect();
                                v.selected = moving.clone();
                                v.dirty = true;
                            }
                            v.drag = Drag::Move {
                                anchor_beat: beat,
                                anchor_pitch: pitch,
                                orig: moving
                                    .iter()
                                    .map(|&k| (k, notes[k].start, notes[k].pitch))
                                    .collect(),
                            };
                            audition(
                                &mut s.audition,
                                &mut v.held,
                                Some(notes[i].pitch),
                                notes[i].vel,
                                track_u8,
                            );
                            v.release_at = None;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    // A drag goes on until its button is let go, wherever the pointer goes.
    let erasing = matches!(v.drag, Drag::Erase);
    let held_down = if erasing && secondary_down {
        true
    } else {
        down_now
    };
    if held_down {
        if let Some(pt) = hover {
            let (beat, pitch) = (beat_at(pt.x), pitch_at(pt.y));
            match &mut v.drag {
                Drag::Move {
                    anchor_beat,
                    anchor_pitch,
                    orig,
                } => {
                    let delta = snap_to((beat - *anchor_beat) as f64, snap as f64) as f32;
                    let dp = pitch - *anchor_pitch;
                    for &(i, start, pc) in orig.iter() {
                        if let Some(n) = notes.get_mut(i) {
                            n.start = (start + delta).max(0.0);
                            n.pitch = (pc as i32 + dp).clamp(0, 127) as u8;
                        }
                    }
                    let first = orig.first().map_or(0, |o| o.2 as i32);
                    let now_pitch = (first + dp).clamp(0, 127) as u8;
                    audition(&mut s.audition, &mut v.held, Some(now_pitch), 100, track_u8);
                }
                Drag::Resize { anchor_beat, orig } => {
                    let delta = snap_to((beat - *anchor_beat) as f64, snap as f64) as f32;
                    for &(i, len) in orig.iter() {
                        if let Some(n) = notes.get_mut(i) {
                            n.len = (len + delta).max(snap.max(1.0 / 64.0));
                            v.last_len = n.len;
                        }
                    }
                }
                Drag::Marquee { from, base } => {
                    let (b0, p0) = (beat_at(from.x), pitch_at(from.y));
                    let mut picked = note_edit::marquee(notes, b0, beat, p0 as u8, pitch as u8);
                    for i in base.iter() {
                        if !picked.contains(i) {
                            picked.push(*i);
                        }
                    }
                    v.selected = picked;
                }
                Drag::Erase => {
                    if let Some((i, _)) = note_edit::hit_note(notes, beat, pitch as u8, 0.0) {
                        notes.remove(i);
                        v.selected.clear();
                        v.dirty = true;
                    }
                }
                Drag::Mute { on, done } => {
                    if let Some((i, _)) = note_edit::hit_note(notes, beat, pitch as u8, 0.0) {
                        if !done.contains(&i) {
                            notes[i].muted = *on;
                            done.push(i);
                            v.dirty = true;
                        }
                    }
                }
                Drag::Paint {
                    pitch: key, start, ..
                } => {
                    let template = Note {
                        start: 0.0,
                        len: v.last_len,
                        pitch: *key,
                        vel: v.new_velocity,
                        muted: false,
                        color: v.color_group,
                        slide: v.slide_mode,
                    };
                    if note_edit::paint(notes, template, *key, *start, beat) > 0 {
                        v.dirty = true;
                    }
                }
                Drag::Slice { beat: at } => {
                    *at = snap_to(beat as f64, snap as f64) as f32;
                }
                Drag::Zoom { .. } => {}
                Drag::Scrub => {
                    // The note under the pointer, or else any note at this moment.
                    let under = note_edit::hit_note(notes, beat, pitch as u8, 0.0)
                        .map(|(i, _)| notes[i].pitch)
                        .or_else(|| {
                            notes
                                .iter()
                                .find(|n| !n.muted && n.start <= beat && beat < n.start + n.len)
                                .map(|n| n.pitch)
                        });
                    audition(
                        &mut s.audition,
                        &mut v.held,
                        under,
                        v.new_velocity,
                        track_u8,
                    );
                }
                Drag::None => {}
            }
        }
    } else if !matches!(v.drag, Drag::None) {
        // Let go. What the drag changed goes to the engine now.
        match std::mem::replace(&mut v.drag, Drag::None) {
            Drag::Slice { beat } => {
                if note_edit::slice_at(notes, beat) > 0 {
                    v.selected.clear();
                    v.dirty = true;
                }
            }
            Drag::Zoom { from } => {
                if let Some(to) = hover {
                    let (x0, x1) = (from.x.min(to.x), from.x.max(to.x));
                    let (y0, y1) = (from.y.min(to.y), from.y.max(to.y));
                    // A click, not a box: nothing to zoom to.
                    if x1 - x0 > 4.0 && y1 - y0 > 4.0 {
                        let (first_beat, last_beat) = (beat_at(x0), beat_at(x1));
                        let top = pitch_at(y0) as f32 + 1.0;
                        let span = (top - pitch_at(y1) as f32).max(1.0);
                        v.beat_px = (grid.width() / (last_beat - first_beat).max(1.0 / 64.0))
                            .clamp(12.0, 400.0);
                        v.key_h = (grid.height() / span).clamp(6.0, 24.0);
                        v.scroll_x = (first_beat * v.beat_px).max(0.0);
                        v.scroll_y = ((127.0 - top + 1.0) * v.key_h).max(0.0);
                    }
                }
            }
            Drag::Scrub => audition(&mut s.audition, &mut v.held, None, 0, track_u8),
            Drag::Marquee { .. } | Drag::None => {}
            Drag::Move { .. }
            | Drag::Resize { .. }
            | Drag::Erase
            | Drag::Mute { .. }
            | Drag::Paint { .. } => v.dirty = true,
        }
    }
    // ---- Time selection, as in FL Studio: drag the ruler with the right button. It takes the
    // notes that start in it, so every tool and key works on them.
    let ruler_resp = ui.interact(ruler, id("ruler"), Sense::click_and_drag());
    let in_ruler = hover.is_some_and(|pt| ruler.contains(pt)) && ruler_resp.hovered();
    let on_step = |x: f32| (snap_to(beat_at(x) as f64, snap as f64) as f32).max(0.0);
    // FL Studio's other way, Ctrl with the left button, works too.
    if in_ruler && (secondary_press || (ctrl && primary_press)) {
        v.time_anchor = hover.map(|pt| on_step(pt.x));
        v.time_sel = None;
    }
    if ruler_resp.clicked() {
        v.time_sel = None;
    }
    // A left click or drag on the ruler moves the playhead there, as on the Tracks area's ruler.
    let left = ui.input(|i| i.pointer.primary_down()) || ruler_resp.clicked();
    if (ruler_resp.clicked() || ruler_resp.dragged()) && left && !ctrl && v.time_anchor.is_none() {
        if let Some(pt) = ruler_resp.interact_pointer_pos() {
            p.position_beats = (origin + f64::from(beat_at(pt.x))).max(0.0);
        }
    }
    if let Some(anchor) = v.time_anchor {
        if !secondary_down && !down_now {
            v.time_anchor = None;
        } else if let Some(pt) = hover {
            let now = on_step(pt.x);
            if (now - anchor).abs() > 1e-4 {
                let range = (anchor.min(now), anchor.max(now));
                v.time_sel = Some(range);
                v.selected = note_edit::in_time(notes, range.0, range.1);
            }
        }
    }
    // ---- Velocity lane (FL Studio style): drag a stem up or down to set that note's velocity.
    let vresp = ui.interact(vel_rect, id("vel_lane"), Sense::click_and_drag());
    // A controller lane: the left button draws the values, the right one rubs them out.
    if let Lane::Control(number) = v.lane {
        let beat_of = |x: f32| beat_at(x).clamp(0.0, len_beats);
        let value_of = |y: f32| {
            (((vel_rect.bottom() - y) / vel_rect.height()) * 127.0)
                .round()
                .clamp(0.0, 127.0) as u8
        };
        if let Some(pt) = hover.filter(|pt| vel_rect.contains(*pt) && vresp.hovered()) {
            if primary_press {
                v.lane_drag = Some(LaneDrag::Draw {
                    beat: beat_of(pt.x),
                    value: value_of(pt.y),
                });
            } else if secondary_press {
                v.lane_drag = Some(LaneDrag::Erase {
                    beat: beat_of(pt.x),
                });
            }
        }
        // Fine enough for a smooth line whatever the snap is; Alt is finest, as on the notes.
        let every = snap.clamp(1.0 / 64.0, 0.125);
        match (v.lane_drag, hover) {
            (Some(LaneDrag::Draw { beat, value }), Some(pt)) if down_now => {
                let (now, level) = (beat_of(pt.x), value_of(pt.y));
                note_edit::draw_controls(controls, number, (beat, value), (now, level), every);
                v.lane_drag = Some(LaneDrag::Draw {
                    beat: now,
                    value: level,
                });
                v.dirty = true;
            }
            (Some(LaneDrag::Erase { beat }), Some(pt)) if secondary_down => {
                let now = beat_of(pt.x);
                note_edit::erase_controls(controls, number, beat, now);
                v.lane_drag = Some(LaneDrag::Erase { beat: now });
                v.dirty = true;
            }
            (Some(_), _) if !down_now && !secondary_down => v.lane_drag = None,
            _ => {}
        }
    } else if vresp.drag_started() || vresp.clicked() {
        let origin = ui.input(|i| i.pointer.press_origin());
        if let Some(pt) = origin.or(vresp.interact_pointer_pos()) {
            let near = |n: &Note| (x_of(n.start) - pt.x).abs();
            let best = notes
                .iter()
                .enumerate()
                .filter(|(_, n)| near(n) <= 7.0)
                .min_by(|a, b| near(a.1).total_cmp(&near(b.1)))
                .map(|(i, _)| i);
            v.vel_drag = best;
            if let Some(i) = best {
                if !v.selected.contains(&i) {
                    v.selected = vec![i];
                }
            }
        }
    }
    if let Some(i) = v.vel_drag {
        match (down_now, hover) {
            (true, Some(pt)) => {
                let nv = (((vel_rect.bottom() - pt.y) / vel_rect.height()) * 127.0)
                    .round()
                    .clamp(1.0, 127.0) as i32;
                if let Some(cur) = notes.get(i).map(|n| n.vel as i32) {
                    let delta = nv - cur;
                    let targets = if v.selected.contains(&i) {
                        v.selected.clone()
                    } else {
                        vec![i]
                    };
                    for k in targets {
                        if let Some(n) = notes.get_mut(k) {
                            n.vel = (n.vel as i32 + delta).clamp(1, 127) as u8;
                        }
                    }
                    v.dirty = true;
                }
            }
            (true, None) => {}
            (false, _) => v.vel_drag = None,
        }
    }

    // ---- Keyboard, as in FL Studio's piano roll (only while the pointer is over the editor)
    if hovered && !ui.ctx().egui_wants_keyboard_input() {
        let (m, pressed): (egui::Modifiers, Vec<Key>) = ui.input(|i| {
            (
                i.modifiers,
                i.events
                    .iter()
                    .filter_map(|e| match e {
                        egui::Event::Key {
                            key, pressed: true, ..
                        } => Some(*key),
                        _ => None,
                    })
                    .collect(),
            )
        });
        let (ctrl, shift, alt) = (m.command, m.shift, m.alt);
        let plain = !ctrl && !shift && !alt;
        // While the keyboard plays notes, Shift picks the tools.
        let tool_key = plain || (typing_on && shift && !ctrl && !alt);
        for key in pressed {
            let targets = targets_of(&v.selected, notes.len(), v.time_sel.is_some());
            let mut edited = true;
            match key {
                // As in FL Studio: pick notes at random, and by the chosen colour group.
                Key::R if shift && !ctrl && !alt && !typing_on && !notes.is_empty() => {
                    v.selected = vec![random_below(notes.len())];
                    edited = false;
                }
                Key::M if shift && !ctrl && !alt && !typing_on => {
                    let left: Vec<usize> = (0..notes.len())
                        .filter(|i| !v.selected.contains(i))
                        .collect();
                    if !left.is_empty() {
                        v.selected.push(left[random_below(left.len())]);
                    }
                    edited = false;
                }
                Key::C if shift && !ctrl && !alt && !typing_on => {
                    v.selected = (0..notes.len())
                        .filter(|&i| notes[i].color == v.color_group)
                        .collect();
                    edited = false;
                }
                Key::P if tool_key => v.tool = Tool::Draw,
                Key::B if tool_key => v.tool = Tool::Paint,
                Key::D if tool_key => v.tool = Tool::Delete,
                Key::T if tool_key => v.tool = Tool::Mute,
                Key::C if tool_key => v.tool = Tool::Slice,
                Key::E if tool_key => v.tool = Tool::Select,
                Key::Z if tool_key => v.tool = Tool::Zoom,
                Key::Y if tool_key => v.tool = Tool::Playback,
                Key::S if tool_key => {
                    set_slide(notes, &v.selected, &mut v.slide_mode);
                    v.dirty |= !v.selected.is_empty();
                    edited = false;
                }
                Key::Escape => {
                    v.selected.clear();
                    v.time_sel = None;
                }
                Key::Delete if ctrl && v.time_sel.is_some() => act.delete_time = true,
                Key::Insert if ctrl && v.time_sel.is_some() => act.insert_time = true,
                Key::PageUp => {
                    v.beat_px = (v.beat_px * 1.25).min(400.0);
                    edited = false;
                }
                Key::PageDown => {
                    v.beat_px = (v.beat_px / 1.25).max(12.0);
                    edited = false;
                }
                Key::V if alt => {
                    v.ghosts = !v.ghosts;
                    edited = false;
                }
                Key::Delete | Key::Backspace => act.delete = true,
                Key::A if ctrl => act.select_all = true,
                Key::D if ctrl => v.selected.clear(),
                Key::Q if ctrl => act.quantize = true,
                Key::Q if shift && !alt => note_edit::quantize_starts(notes, &targets, step),
                Key::L if ctrl => note_edit::legato(notes, &targets, len_beats),
                Key::U if ctrl => {
                    let pieces = note_edit::chop(notes, &targets, step);
                    v.selected = pieces;
                }
                Key::G if ctrl => {
                    let joined = note_edit::glue(notes, &targets);
                    v.selected = joined;
                }
                Key::B if ctrl => {
                    let copies = note_edit::duplicate(notes, &targets);
                    v.selected = copies;
                }
                Key::M if alt => {
                    for &i in &targets {
                        notes[i].muted = !shift;
                    }
                }
                Key::C if ctrl => {
                    v.clipboard = v.selected.iter().map(|&i| notes[i]).collect();
                    edited = false;
                }
                Key::X if ctrl => {
                    v.clipboard = v.selected.iter().map(|&i| notes[i]).collect();
                    note_edit::remove_notes(notes, &v.selected);
                    v.selected.clear();
                }
                Key::V if ctrl && !v.clipboard.is_empty() => {
                    let at = (p_position - origin) as f32;
                    let at = (at.max(0.0) / step).floor() * step;
                    v.selected = note_edit::paste(notes, &v.clipboard, at);
                }
                Key::I if shift => {
                    v.selected = note_edit::inverted(notes.len(), &v.selected);
                    edited = false;
                }
                Key::ArrowUp | Key::ArrowDown if ctrl != shift => {
                    let amount = if ctrl { 12 } else { 1 };
                    let dp = if key == Key::ArrowUp { amount } else { -amount };
                    for &i in &targets {
                        notes[i].pitch = (notes[i].pitch as i32 + dp).clamp(0, 127) as u8;
                    }
                }
                Key::ArrowLeft | Key::ArrowRight if shift => {
                    let dt = if key == Key::ArrowLeft { -step } else { step };
                    for &i in &targets {
                        notes[i].start = (notes[i].start + dt).max(0.0);
                    }
                    // The time selection goes with its notes.
                    if let Some((a, b)) = v.time_sel {
                        let d = dt.max(-a);
                        v.time_sel = Some((a + d, b + d));
                    }
                }
                Key::ArrowLeft | Key::ArrowRight if plain => {
                    let bar = bpb as f32 * beat_px;
                    let d = if key == Key::ArrowLeft { -bar } else { bar };
                    v.scroll_x = (v.scroll_x + d).max(0.0);
                    edited = false;
                }
                _ => edited = false,
            }
            v.dirty |= edited;
        }
    }
    if act.select_all {
        v.selected = (0..notes.len()).collect();
    }

    // ---- Inspector (left): quantize, scale quantize, velocity
    let gi = ui.painter_at(insp);
    gi.rect_filled(insp, 0.0, panel_bg_col());
    gi.line_segment(
        [insp.right_top(), insp.right_bottom()],
        Stroke::new(1.0, crate::theme::border()),
    );
    let ix = insp.left() + 12.0;
    let iw = insp.width() - 24.0;
    let mut y = insp.top() + 10.0;
    let n_sel = v.selected.len();
    // Region header: icon, name and track, like Logic's MIDI region inspector.
    let icon = Rect::from_min_size(pos2(ix, y - 2.0), vec2(26.0, 22.0));
    crate::track_icons::draw_icon(&gi, icon, crate::track_icons::IconKind::Note);
    let font = FontId::proportional(12.0);
    let name = crate::mixer::elide(&gi, &region_name, &font, iw - 70.0);
    gi.text(
        pos2(insp.center().x, y + 2.0),
        Align2::CENTER_CENTER,
        name,
        font,
        text_col(),
    );
    let font = FontId::proportional(10.0);
    let sub = crate::mixer::elide(&gi, &trf("on Track {}", &[&track_name]), &font, iw - 70.0);
    gi.text(
        pos2(insp.center().x, y + 16.0),
        Align2::CENTER_CENTER,
        sub,
        font,
        text_dim_col(),
    );
    let box_r = Rect::from_min_size(pos2(ix + iw - 24.0, y), vec2(24.0, 18.0));
    gi.rect_filled(box_r, 3.0, crate::theme::gray(0x28));
    gi.rect_stroke(
        Rect::from_center_size(box_r.center(), vec2(10.0, 8.0)),
        1.5,
        Stroke::new(1.0, crate::theme::gray(0xc0)),
        StrokeKind::Inside,
    );
    y += 46.0;

    gi.text(
        pos2(ix, y),
        Align2::LEFT_TOP,
        tr("Time Quantize (classic)"),
        FontId::proportional(11.5),
        text_dim_col(),
    );
    y += 18.0;
    let grid_r = Rect::from_min_size(pos2(ix, y), vec2(iw - 38.0, 20.0));
    v.groove = v.groove.filter(|i| *i < p.grooves.len());
    let grid_text = match v.groove {
        Some(i) => format!("Groove: {}", p.grooves[i].name),
        None => tr(GRIDS[v.grid].0).to_string(),
    };
    let gresp = popup_button(ui, &gi, grid_r, id("grid_popup"), &grid_text);
    egui::Popup::menu(&gresp).show(|ui| {
        for (i, (label, _)) in GRIDS.iter().enumerate() {
            let on = v.groove.is_none() && v.grid == i;
            if ui.selectable_label(on, tr(label)).clicked() {
                v.grid = i;
                v.groove = None;
                ui.close();
            }
        }
        ui.separator();
        for (i, groove) in p.grooves.iter().enumerate() {
            let text = format!("Groove: {}", groove.name);
            if ui.selectable_label(v.groove == Some(i), text).clicked() {
                v.groove = Some(i);
                ui.close();
            }
        }
        if ui.button("Make Groove from This Region").clicked() {
            act.make_groove = true;
            ui.close();
        }
        if v.groove.is_some() && ui.button("Delete This Groove").clicked() {
            act.delete_groove = true;
            ui.close();
        }
    });
    if small_q(
        ui,
        &gi,
        Rect::from_min_size(pos2(ix + iw - 30.0, y), vec2(30.0, 20.0)),
        "time",
        true,
    ) {
        act.quantize = true;
    }
    y += 34.0;
    for (label, key, value) in [
        (tr("Strength"), "strength", &mut v.strength),
        (tr("Swing"), "swing", &mut v.swing),
    ] {
        gi.text(
            pos2(ix, y),
            Align2::LEFT_TOP,
            label,
            FontId::proportional(11.5),
            text_dim_col(),
        );
        gi.text(
            pos2(ix + iw, y),
            Align2::RIGHT_TOP,
            format!("{}", (*value * 100.0).round() as i32),
            FontId::proportional(11.5),
            text_col(),
        );
        y += 18.0;
        let r = Rect::from_min_size(pos2(ix, y), vec2(iw, 14.0));
        if let Some(n) = slider_h(ui, &gi, r, id(key), *value, true) {
            *value = n;
        }
        y += 28.0;
    }
    gi.line_segment(
        [
            pos2(insp.left() + 6.0, y - 8.0),
            pos2(insp.right() - 6.0, y - 8.0),
        ],
        Stroke::new(1.0, crate::theme::gray(0x18)),
    );
    y += 4.0;

    gi.text(
        pos2(ix, y),
        Align2::LEFT_TOP,
        tr("Scale Quantize"),
        FontId::proportional(11.5),
        text_dim_col(),
    );
    y += 18.0;
    let root_r = Rect::from_min_size(pos2(ix, y), vec2(70.0, 20.0));
    let root_text = v.scale_root.map_or(tr("Off"), |r| ROOTS[r as usize]);
    let rresp = popup_button(ui, &gi, root_r, id("root_popup"), root_text);
    egui::Popup::menu(&rresp).show(|ui| {
        if ui
            .selectable_label(v.scale_root.is_none(), tr("Off"))
            .clicked()
        {
            v.scale_root = None;
            ui.close();
        }
        for (i, r) in ROOTS.iter().enumerate() {
            if ui
                .selectable_label(v.scale_root == Some(i as u8), *r)
                .clicked()
            {
                v.scale_root = Some(i as u8);
                ui.close();
            }
        }
    });
    let mode_r = Rect::from_min_size(pos2(ix + 76.0, y), vec2(iw - 76.0 - 38.0, 20.0));
    let mresp = popup_button(ui, &gi, mode_r, id("mode_popup"), tr(MODES[v.scale_mode].0));
    egui::Popup::menu(&mresp).show(|ui| {
        for (i, (label, _)) in MODES.iter().enumerate() {
            if ui.selectable_label(v.scale_mode == i, tr(label)).clicked() {
                v.scale_mode = i;
                ui.close();
            }
        }
    });
    if small_q(
        ui,
        &gi,
        Rect::from_min_size(pos2(ix + iw - 30.0, y), vec2(30.0, 20.0)),
        "scale",
        v.scale_root.is_some(),
    ) {
        act.scale = true;
    }
    y += 38.0;
    gi.line_segment(
        [
            pos2(insp.left() + 6.0, y - 10.0),
            pos2(insp.right() - 6.0, y - 10.0),
        ],
        Stroke::new(1.0, crate::theme::gray(0x18)),
    );

    // Velocity of the selection (average), or the velocity for new notes.
    let vel = if n_sel == 0 {
        v.new_velocity as f32
    } else {
        v.selected.iter().map(|&i| notes[i].vel as f32).sum::<f32>() / n_sel as f32
    };
    gi.text(
        pos2(ix, y),
        Align2::LEFT_TOP,
        tr("Velocity"),
        FontId::proportional(11.5),
        text_dim_col(),
    );
    gi.text(
        pos2(ix + iw, y),
        Align2::RIGHT_TOP,
        format!("{}", vel.round() as i32),
        FontId::proportional(11.5),
        text_col(),
    );
    y += 18.0;
    if let Some(n) = slider_h(
        ui,
        &gi,
        Rect::from_min_size(pos2(ix, y), vec2(iw, 14.0)),
        id("velocity"),
        vel / 127.0,
        true,
    ) {
        let new = (n * 127.0).round().clamp(1.0, 127.0) as u8;
        if n_sel == 0 {
            v.new_velocity = new;
        } else {
            let d = new as i32 - vel.round() as i32;
            for &i in &v.selected {
                notes[i].vel = (notes[i].vel as i32 + d).clamp(1, 127) as u8;
            }
            v.dirty = true;
        }
    }

    // Colour groups and slide notes, as in FL Studio.
    y += 22.0;
    gi.text(
        pos2(ix, y),
        Align2::LEFT_TOP,
        "Color Group",
        FontId::proportional(11.5),
        text_dim_col(),
    );
    y += 16.0;
    let swatch = (iw / f32::from(COLOR_GROUPS / 2)).floor();
    for group in 0..COLOR_GROUPS {
        let (col, row) = (f32::from(group % 8), f32::from(group / 8));
        let r = Rect::from_min_size(
            pos2(ix + col * swatch, y + row * 16.0),
            vec2(swatch - 2.0, 14.0),
        );
        gi.rect_filled(r, 2.0, group_color(group));
        if group == v.color_group {
            gi.rect_stroke(r, 2.0, Stroke::new(1.5, Color32::WHITE), StrokeKind::Inside);
        }
        let resp = ui
            .interact(r, id(("color_group", group)), Sense::click())
            .on_hover_text(
                "New notes and the selection take this color. Shift+C selects its notes.",
            );
        if resp.clicked() {
            v.color_group = group;
            v.note_colors = NoteColors::Group;
            for &i in &v.selected {
                notes[i].color = group;
            }
            v.dirty |= !v.selected.is_empty();
        }
    }
    y += 36.0;
    let slide_button = Rect::from_min_size(pos2(ix, y), vec2(76.0, 20.0));
    let slide_resp = ui
        .interact(slide_button, id("slide_mode"), Sense::click())
        .on_hover_text(
            "New notes are slide notes. With notes selected, makes them slide or not (S).",
        );
    gi.rect_filled(
        slide_button,
        3.0,
        if v.slide_mode { blue() } else { btn_bg_col() },
    );
    gi.text(
        slide_button.center(),
        Align2::CENTER_CENTER,
        "Slide (S)",
        FontId::proportional(11.5),
        text_col(),
    );
    if slide_resp.clicked() {
        set_slide(notes, &v.selected, &mut v.slide_mode);
        v.dirty |= !v.selected.is_empty();
    }
    gi.text(
        pos2(ix + iw, y + 10.0),
        Align2::RIGHT_CENTER,
        format!("Range {slide_range}"),
        FontId::proportional(11.5),
        text_col(),
    );
    y += 24.0;
    if let Some(n) = slider_h(
        ui,
        &gi,
        Rect::from_min_size(pos2(ix, y), vec2(iw, 14.0)),
        id("slide_range"),
        (f32::from(slide_range) - 1.0) / 47.0,
        true,
    ) {
        v.slide_range_edit = Some((1.0 + n * 47.0).round().clamp(1.0, 48.0) as u8);
    }

    // Automation / MIDI: what the lane under the notes shows (Note Velocity).
    let ay = insp.bottom() - 112.0;
    if ay > y + 24.0 {
        gi.text(
            pos2(ix, ay),
            Align2::LEFT_TOP,
            tr("Automation/MIDI"),
            FontId::proportional(11.5),
            text_dim_col(),
        );
        let power = Rect::from_min_size(pos2(ix, ay + 18.0), vec2(26.0, 20.0));
        gi.rect_filled(power, 3.0, Color32::from_rgb(0x2a, 0x5a, 0xb0));
        gi.circle_stroke(power.center(), 5.0, Stroke::new(1.3, Color32::WHITE));
        let region_btn =
            Rect::from_min_size(pos2(ix + 28.0, ay + 18.0), vec2(iw - 28.0 - 34.0, 20.0));
        gi.rect_filled(region_btn, 0.0, blue());
        gi.text(
            region_btn.center(),
            Align2::CENTER_CENTER,
            tr("Region"),
            FontId::proportional(11.5),
            Color32::WHITE,
        );
        let curve = Rect::from_min_size(pos2(ix + iw - 30.0, ay + 18.0), vec2(30.0, 20.0));
        gi.rect_filled(curve, 3.0, crate::theme::gray(0x28));
        let box_r = Rect::from_min_size(pos2(ix, ay + 42.0), vec2(iw, 44.0));
        gi.rect_filled(box_r, 3.0, crate::theme::gray(0x1a));
        gi.text(
            pos2(box_r.center().x, box_r.top() + 10.0),
            Align2::CENTER_CENTER,
            v.lane.name(),
            FontId::proportional(11.5),
            Color32::from_rgb(0x4c, 0xe0, 0x4c),
        );
        gi.text(
            pos2(box_r.center().x, box_r.top() + 30.0),
            Align2::CENTER_CENTER,
            "Click to choose",
            FontId::proportional(10.0),
            text_dim_col(),
        );
        let pick = ui.interact(box_r, id("lane_box"), Sense::click());
        let mut used: Vec<u8> = controls.iter().map(|c| c.num).collect();
        used.sort_unstable();
        used.dedup();
        egui::Popup::menu(&pick).show(|ui| lane_menu(ui, &mut v.lane, &used));
    }

    let hint = "Click: place · Right-drag: delete · Ctrl-drag: select";
    let hint_font = FontId::proportional(10.5);
    let hint_text = crate::mixer::elide(&gi, hint, &hint_font, iw);
    gi.text(
        pos2(ix, insp.bottom() - 18.0),
        Align2::LEFT_TOP,
        hint_text,
        hint_font,
        crate::theme::gray(0x60),
    );

    // ---- Apply toolbar/inspector actions
    let targets = targets_of(&v.selected, notes.len(), v.time_sel.is_some());
    if let (true, Some((a, b))) = (act.delete_time, v.time_sel) {
        note_edit::delete_time(notes, controls, a, b);
        v.selected.clear();
        v.time_sel = None;
        v.dirty = true;
    }
    if let (true, Some((a, b))) = (act.insert_time, v.time_sel) {
        note_edit::insert_time(notes, controls, a, b);
        v.selected.clear();
        v.dirty = true;
    }
    if act.quantize {
        match v.groove.and_then(|i| p.grooves.get(i)) {
            Some(groove) => {
                crate::groove_template::quantize(notes, &targets, groove, groove_offset, v.strength)
            }
            None => note_edit::quantize(notes, &targets, GRIDS[v.grid].1, v.strength, v.swing),
        }
        v.dirty = true;
    }
    if act.scale {
        if let Some(root) = v.scale_root {
            note_edit::scale_quantize(notes, &targets, root, MODES[v.scale_mode].1);
            v.dirty = true;
        }
    }
    if act.same_pitch {
        v.selected = crate::note_tools::same_pitch(notes, &v.selected);
    }
    if act.same_start {
        v.selected = crate::note_tools::same_start(notes, &v.selected);
    }
    if act.flip_time {
        crate::note_tools::flip_time(notes, &targets);
        v.dirty = true;
    }
    if act.flip_pitch {
        crate::note_tools::flip_pitch(notes, &targets);
        v.dirty = true;
    }
    if v.tools.window(ui.ctx()) {
        v.tools.apply(notes, &targets);
        v.selected.clear();
        v.dirty = true;
    }
    if act.delete && !v.selected.is_empty() {
        let mut sel = std::mem::take(&mut v.selected);
        sel.sort_unstable_by(|a, b| b.cmp(a));
        for i in sel {
            if i < notes.len() {
                notes.remove(i);
            }
        }
        v.dirty = true;
    }

    if let (true, Some(i)) = (act.delete_groove, v.groove) {
        p.grooves.remove(i);
        v.groove = None;
    }
    if let (true, Some((a, b))) = (act.loop_time, v.time_sel) {
        p.cycle_start = origin + f64::from(a);
        p.cycle_end = origin + f64::from(b);
        p.cycle = true;
    }

    // ---- Draw keys, notes, ruler, playhead (after edits so they show immediately)
    let notes = &p.tracks[ti].regions[ri].notes;
    let gk = ui.painter_at(keys);
    gk.rect_filled(keys, 0.0, crate::theme::gray(0x12));
    for pitch in bottom_pitch..=top_pitch {
        let y = y_of(pitch);
        let row = Rect::from_min_size(pos2(keys.left(), y), vec2(KEYS_W, key_h));
        if is_black(pitch) {
            gk.rect_filled(
                Rect::from_min_size(row.min, vec2(KEYS_W * 0.62, key_h)),
                0.0,
                crate::theme::gray(0x0b),
            );
            gk.rect_filled(
                Rect::from_min_max(pos2(row.left() + KEYS_W * 0.62, row.top()), row.max),
                0.0,
                crate::theme::gray(0xe2),
            );
        } else {
            gk.rect_filled(row, 0.0, crate::theme::gray(0xee));
            gk.line_segment(
                [row.left_bottom(), row.right_bottom()],
                Stroke::new(1.0, crate::theme::gray(0x84)),
            );
        }
        if v.held == Some(pitch as u8) {
            gk.rect_filled(
                row,
                0.0,
                Color32::from_rgba_unmultiplied(0x2f, 0x6f, 0xd8, 200),
            );
        }
        if pitch % 12 == 0 {
            gk.text(
                pos2(keys.right() - 4.0, y + key_h / 2.0),
                Align2::RIGHT_CENTER,
                c_label(pitch),
                FontId::proportional(9.5),
                crate::theme::gray(0x1a),
            );
        }
    }
    if v.ghosts {
        for (t, r_at, n) in ghost_notes(p, ti, ri) {
            let start =
                (p.region_start_beats(&p.tracks[t].regions[r_at]) - origin) as f32 + n.start;
            let r = Rect::from_min_size(
                pos2(x_of(start), y_of(n.pitch as i32) + 1.0),
                vec2((n.len * beat_px).max(3.0), key_h - 2.0),
            );
            if r.right() >= grid.left() && r.left() <= grid.right() {
                // Ghost notes: a shade of the text colour, seen in every theme.
                gg.rect_filled(r, 2.0, grid_ink(0.22));
                gg.rect_stroke(r, 2.0, Stroke::new(1.0, grid_ink(0.38)), StrokeKind::Inside);
            }
        }
    }
    for (i, n) in notes.iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(x_of(n.start), y_of(n.pitch as i32) + 1.0),
            vec2((n.len * beat_px).max(3.0), key_h - 2.0),
        );
        if r.right() < grid.left()
            || r.left() > grid.right()
            || r.bottom() < grid.top()
            || r.top() > grid.bottom()
        {
            continue;
        }
        let selected = v.selected.contains(&i);
        // Logic draws a note with a light edge of its velocity colour around a deeper body.
        let color = match v.note_colors {
            NoteColors::Velocity => velocity_color(n.vel),
            NoteColors::Track => p.tracks[ti].color,
            NoteColors::Pitch => pitch_color(n.pitch),
            NoteColors::Group => group_color(n.color),
        };
        let (fill, edge) = if n.muted {
            (crate::theme::gray(0x2e), crate::theme::gray(0x3f))
        } else if selected {
            (NOTE_SEL, Color32::WHITE)
        } else {
            (
                color.lerp_to_gamma(crate::theme::gray(0x50), 0.3),
                color.lerp_to_gamma(Color32::WHITE, 0.65),
            )
        };
        gg.rect_filled(r, 1.5, fill);
        gg.rect_stroke(r, 1.5, Stroke::new(1.0, edge), StrokeKind::Inside);
        if n.slide {
            // A slide note says so with a rising wedge.
            gg.add(Shape::convex_polygon(
                vec![r.left_bottom(), r.right_bottom(), r.right_top()],
                Color32::from_black_alpha(90),
                Stroke::NONE,
            ));
        }
    }
    if let Drag::Slice { beat } = &v.drag {
        let lx = x_of(*beat);
        gg.line_segment(
            [pos2(lx, grid.top()), pos2(lx, grid.bottom())],
            Stroke::new(1.5, blue()),
        );
    }
    if let Drag::Marquee { from, .. } | Drag::Zoom { from } = &v.drag {
        if let Some(now) = ui.input(|i| i.pointer.hover_pos()) {
            let r = Rect::from_two_pos(*from, now);
            gg.rect_filled(r, 0.0, Color32::from_white_alpha(22));
            gg.rect_stroke(
                r,
                0.0,
                Stroke::new(1.0, Color32::from_white_alpha(150)),
                StrokeKind::Inside,
            );
        }
    }

    // The stacked controller lanes: each one draws with the left button, rubs out with the right.
    let mut remove_lane = None;
    for (k, num) in v.extra_lanes.clone().into_iter().enumerate() {
        let rect = Rect::from_min_size(
            pos2(grid.left(), hbar.bottom() + 4.0 + k as f32 * (EXTRA_H + 3.0)),
            vec2(grid.width(), EXTRA_H),
        );
        let label = Rect::from_min_max(pos2(keys.left(), rect.top()), pos2(keys.right(), rect.bottom()));
        let gl = ui.painter_at(Rect::from_min_max(label.min, rect.max));
        gl.rect_filled(rect, 0.0, row_white_col());
        gl.rect_filled(label, 0.0, crate::theme::gray(0x18));
        gl.text(
            label.left_center() + vec2(5.0, 0.0),
            Align2::LEFT_CENTER,
            Lane::Control(num).short(),
            FontId::proportional(10.0),
            text_dim_col(),
        );
        let close = Rect::from_center_size(pos2(label.right() - 8.0, label.top() + 8.0), vec2(14.0, 14.0));
        let closed = ui
            .interact(close, id(("extra_lane_close", k)), Sense::click())
            .on_hover_text("Remove this lane (its values stay)");
        gl.text(close.center(), Align2::CENTER_CENTER, "\u{00d7}", FontId::proportional(12.0), text_col());
        if closed.clicked() {
            remove_lane = Some(k);
        }
        for &(rel, _) in &bars {
            let bx = x_of(rel as f32);
            gl.line_segment([pos2(bx, rect.top()), pos2(bx, rect.bottom())], Stroke::new(1.0, grid_ink(0.18)));
        }
        let y_of_val = |val: u8| rect.bottom() - f32::from(val.min(127)) / 127.0 * (rect.height() - 2.0);
        let mut points: Vec<(f32, u8)> = p.tracks[ti].regions[ri]
            .controls
            .iter()
            .filter(|c| c.num == num)
            .map(|c| (c.start, c.val))
            .collect();
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (i, &(at, val)) in points.iter().enumerate() {
            let next = points.get(i + 1).map_or(len_beats, |n| n.0);
            let (x0, x1, y) = (x_of(at), x_of(next), y_of_val(val));
            gl.rect_filled(Rect::from_min_max(pos2(x0, y), pos2(x1.max(x0 + 1.0), rect.bottom())), 0.0, blue().gamma_multiply(0.25));
            gl.line_segment([pos2(x0, y), pos2(x1, y)], Stroke::new(1.5, blue()));
        }
        let resp = ui.interact(rect, id(("extra_lane", k)), Sense::click_and_drag());
        let beat_of = |x: f32| beat_at(x).clamp(0.0, len_beats);
        let value_of = |y: f32| (((rect.bottom() - y) / rect.height()) * 127.0).round().clamp(0.0, 127.0) as u8;
        if let Some(pt) = hover.filter(|pt| rect.contains(*pt) && resp.hovered()) {
            if primary_press {
                v.extra_drag = Some((k, LaneDrag::Draw { beat: beat_of(pt.x), value: value_of(pt.y) }));
            } else if secondary_press {
                v.extra_drag = Some((k, LaneDrag::Erase { beat: beat_of(pt.x) }));
            }
        }
        let every = snap.clamp(1.0 / 64.0, 0.125);
        let controls = &mut p.tracks[ti].regions[ri].controls;
        match (v.extra_drag, hover) {
            (Some((lane, LaneDrag::Draw { beat, value })), Some(pt)) if lane == k && down_now => {
                let (now, level) = (beat_of(pt.x), value_of(pt.y));
                note_edit::draw_controls(controls, num, (beat, value), (now, level), every);
                v.extra_drag = Some((k, LaneDrag::Draw { beat: now, value: level }));
                p.song_rev += 1;
            }
            (Some((lane, LaneDrag::Erase { beat })), Some(pt)) if lane == k && secondary_down => {
                let now = beat_of(pt.x);
                note_edit::erase_controls(controls, num, beat, now);
                v.extra_drag = Some((k, LaneDrag::Erase { beat: now }));
                p.song_rev += 1;
            }
            (Some((lane, _)), _) if lane == k && !down_now && !secondary_down => v.extra_drag = None,
            _ => {}
        }
    }
    if let Some(k) = remove_lane {
        v.extra_lanes.remove(k);
        v.extra_drag = None;
    }

    // Velocity or controller lane. Its label picks which.
    let gv = ui.painter_at(Rect::from_min_max(vel_label.min, vel_rect.max));
    gv.rect_filled(vel_label, 0.0, crate::theme::gray(0x18));
    gv.text(
        vel_label.center(),
        Align2::CENTER_CENTER,
        v.lane.short(),
        FontId::proportional(10.5),
        text_dim_col(),
    );
    let pick = ui
        .interact(vel_label, id("lane_label"), Sense::click())
        .on_hover_text("Choose what this lane shows: velocity or a MIDI controller");
    let mut used: Vec<u8> = p.tracks[ti].regions[ri]
        .controls
        .iter()
        .map(|c| c.num)
        .collect();
    used.sort_unstable();
    used.dedup();
    egui::Popup::menu(&pick).show(|ui| lane_menu(ui, &mut v.lane, &used));
    gv.rect_filled(vel_rect, 0.0, row_white_col());
    // Another lane, stacked above: any controller, shown with the others at once.
    let add = Rect::from_min_size(vel_label.left_bottom() - vec2(-4.0, 16.0), vec2(vel_label.width() - 8.0, 14.0));
    if v.extra_lanes.len() < MAX_EXTRA_LANES {
        let add_resp = ui
            .interact(add, id("extra_lane_add"), Sense::click())
            .on_hover_text("Show another controller lane above this one");
        gv.text(add.center(), Align2::CENTER_CENTER, "+ Lane", FontId::proportional(9.5), text_dim_col());
        egui::Popup::menu(&add_resp).show(|ui| {
            for n in NAMED_CONTROLS.iter().map(|c| c.0).chain(used.iter().copied()) {
                if v.extra_lanes.contains(&n) || v.lane == Lane::Control(n) {
                    continue;
                }
                if ui.button(Lane::Control(n).name()).clicked() {
                    v.extra_lanes.push(n);
                    ui.close();
                }
            }
        });
    }
    for f in [0.25f32, 0.5, 0.75] {
        let y = vel_rect.bottom() - f * vel_rect.height();
        gv.line_segment(
            [pos2(vel_rect.left(), y), pos2(vel_rect.right(), y)],
            Stroke::new(1.0, grid_ink(0.08)),
        );
    }
    for &(rel, _) in &bars {
        let bx = x_of(rel as f32);
        gv.line_segment(
            [pos2(bx, vel_rect.top()), pos2(bx, vel_rect.bottom())],
            Stroke::new(1.0, grid_ink(0.18)),
        );
    }
    if let Lane::Control(number) = v.lane {
        // A controller holds its value until the next event.
        let shape = p.tracks[ti].regions[ri]
            .controls
            .iter()
            .filter(|c| c.num == number);
        let y_of_val =
            |val: u8| vel_rect.bottom() - 3.0 - f32::from(val) / 127.0 * (vel_rect.height() - 6.0);
        let col = Color32::from_rgb(115, 197, 113);
        let mut last: Option<(f32, f32)> = None;
        for c in shape {
            let (x, y) = (x_of(c.start), y_of_val(c.val));
            if let Some((px, py)) = last {
                gv.line_segment([pos2(px, py), pos2(x, py)], Stroke::new(1.5, col));
                gv.line_segment([pos2(x, py), pos2(x, y)], Stroke::new(1.5, col));
            }
            last = Some((x, y));
        }
        if let Some((px, py)) = last {
            gv.line_segment([pos2(px, py), pos2(rx1, py)], Stroke::new(1.5, col));
        }
    }
    let velocities: &[Note] = if v.lane == Lane::Velocity {
        &p.tracks[ti].regions[ri].notes
    } else {
        &[]
    };
    for (i, n) in velocities.iter().enumerate() {
        let x = x_of(n.start);
        if x < vel_rect.left() - 4.0 || x > vel_rect.right() + 4.0 {
            continue;
        }
        let top = vel_rect.bottom() - n.vel as f32 / 127.0 * (vel_rect.height() - 6.0);
        let col = if v.selected.contains(&i) {
            NOTE_SEL
        } else {
            Color32::from_rgb(115, 197, 113)
        };
        // Logic shows each note's velocity as a dot with a line along the note.
        let end = x_of(n.start + n.len);
        gv.line_segment(
            [pos2(x, top), pos2(end.max(x + 2.0), top)],
            Stroke::new(1.5, col),
        );
        gv.circle_filled(pos2(x, top), 2.5, Color32::from_rgb(131, 218, 127));
    }

    // Ruler (absolute bar numbers) and region strip.
    let gr = ui.painter_at(Rect::from_min_max(
        pos2(keys.left(), editor.top()),
        editor.max,
    ));
    gr.rect_filled(
        Rect::from_min_max(
            pos2(keys.left(), editor.top()),
            pos2(editor.right(), grid.top()),
        ),
        0.0,
        crate::theme::gray(0x11),
    );
    // The part of the ruler over the region is gold, as in Logic.
    let ruler_gold = Color32::from_rgb(
        (region_color.r() as f32 * 0.66) as u8,
        (region_color.g() as f32 * 0.64) as u8,
        (region_color.b() as f32 * 0.64) as u8,
    );
    let gold = Rect::from_min_max(
        pos2(rx0.max(grid.left()), ruler.top()),
        pos2(rx1.min(grid.right()), ruler.bottom()),
    );
    if gold.width() > 0.0 {
        gr.rect_filled(gold, 0.0, ruler_gold);
    }
    let label_every = [1i32, 2, 4, 8, 16]
        .into_iter()
        .find(|n| *n as f32 * bpb as f32 * beat_px >= 36.0)
        .unwrap_or(32);
    for &(rel, bar0) in &bars {
        let x = x_of(rel as f32);
        gr.line_segment(
            [pos2(x, ruler.top() + 3.0), pos2(x, ruler.bottom())],
            Stroke::new(1.0, crate::theme::gray(0x40)),
        );
        if (bar0 as i32 - (start_bar as i32 - 1)) % label_every == 0 {
            gr.text(
                pos2(x + 3.0, ruler.top() + 4.0),
                Align2::LEFT_TOP,
                format!("{}", bar0 + 1),
                FontId::proportional(11.0),
                text_col(),
            );
        }
    }
    let strip = Rect::from_min_max(
        pos2(rx0.max(region_bar.left()), region_bar.top() + 1.0),
        pos2(rx1.min(region_bar.right()), region_bar.bottom()),
    );
    if strip.width() > 0.0 {
        gr.rect_filled(strip, 0.0, region_color);
        gr.rect_stroke(
            strip,
            0.0,
            Stroke::new(1.0, Color32::from_rgb(27, 21, 14)),
            StrokeKind::Inside,
        );
        gr.text(
            pos2(strip.left() + 6.0, strip.center().y),
            Align2::LEFT_CENTER,
            &region_name,
            FontId::proportional(10.5),
            crate::theme::gray(0x12),
        );
    }
    gr.line_segment(
        [
            pos2(keys.left(), grid.top()),
            pos2(editor.right(), grid.top()),
        ],
        Stroke::new(1.0, crate::theme::gray(0x12)),
    );
    // The time selection shades the grid and the lane, and shows on the ruler.
    if let Some((a, b)) = v.time_sel {
        let (x0, x1) = (x_of(a), x_of(b));
        let tint = Color32::from_rgba_unmultiplied(0x5a, 0xa0, 0xff, 34);
        for (painter, area) in [(&gg, grid), (&gv, vel_rect)] {
            let band = Rect::from_min_max(pos2(x0, area.top()), pos2(x1, area.bottom()));
            painter.rect_filled(band, 0.0, tint);
            for x in [x0, x1] {
                painter.line_segment(
                    [pos2(x, area.top()), pos2(x, area.bottom())],
                    Stroke::new(1.0, blue()),
                );
            }
        }
        let top = Rect::from_min_max(pos2(x0, ruler.top()), pos2(x1, ruler.bottom()));
        gr.rect_filled(top, 0.0, blue().gamma_multiply(0.75));
    }
    // The step input cursor.
    if v.step_input {
        let sx = x_of(v.step_at);
        if sx >= grid.left() && sx <= grid.right() {
            let col = crate::theme::accent();
            gg.rect_filled(
                Rect::from_min_max(
                    pos2(sx, grid.top()),
                    pos2(sx + v.step_len * beat_px, grid.bottom()),
                ),
                0.0,
                col.gamma_multiply(0.12),
            );
            gg.line_segment([pos2(sx, grid.top()), pos2(sx, grid.bottom())], Stroke::new(2.0, col));
        }
    }
    let px = x_of(position as f32);
    if px >= grid.left() && px <= grid.right() {
        gg.line_segment(
            [pos2(px, grid.top()), pos2(px, grid.bottom())],
            Stroke::new(1.0, crate::tracks_area::playhead_colors(p.recording).0),
        );
        gr.add(Shape::convex_polygon(
            vec![
                pos2(px - 5.0, ruler.top() + 2.0),
                pos2(px + 5.0, ruler.top() + 2.0),
                pos2(px, ruler.top() + 10.0),
            ],
            crate::tracks_area::playhead_colors(p.recording).1,
            Stroke::NONE,
        ));
    }

    // Rebuild the engine song once per gesture, not on every frame of a drag.
    if v.dirty && !ui.input(|i| i.pointer.any_down()) {
        v.dirty = false;
        p.song_rev += 1;
    }
    if act.make_groove {
        crate::groove_template::make(p, s, ti, ri);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoomed_out_smart_snap_uses_the_signature_beat() {
        assert_eq!(piano_step(Snap::Smart, 10.0, 4.0, 1.0), 1.0);
        assert_eq!(piano_step(Snap::Smart, 10.0, 3.0, 0.5), 0.5);
        assert_eq!(piano_step(Snap::Smart, 100.0, 3.0, 0.5), 0.125);
        assert_eq!(piano_step(Snap::Beat, 100.0, 3.0, 0.5), 0.5);
    }

    use crate::ui_test::Harness;
    use egui::{Modifiers, PointerButton};

    /// A piano roll on an empty MIDI region, its snap one beat, ready to be clicked on.
    struct Rig {
        p: Project,
        s: UiState,
        h: Harness,
        t: usize,
    }

    impl Rig {
        fn new() -> Self {
            let mut p = Project::demo();
            let t = p
                .tracks
                .iter()
                .position(|t| {
                    t.kind == TrackKind::Instrument
                        && t.regions
                            .first()
                            .is_some_and(|r| r.generated.is_none() && r.folder.is_none())
                })
                .unwrap();
            p.tracks[t].regions[0].notes.clear();
            p.tracks[t].regions[0].len_bars = 16.0;
            let mut s = UiState::new();
            s.tracks.selected = Some((t, 0));
            s.tracks.snap = Snap::Beat;
            s.score_tab = false;
            let mut rig = Self {
                p,
                s,
                h: Harness::new(1400.0, 800.0),
                t,
            };
            rig.settle();
            rig
        }

        fn settle(&mut self) {
            let (p, s) = (&mut self.p, &mut self.s);
            self.h.settle(|ui| show(ui, p, s));
        }

        fn notes(&self) -> &Vec<Note> {
            &self.p.tracks[self.t].regions[0].notes
        }

        /// A row in the middle of the grid, whatever it has scrolled to.
        fn row(&self, up: i32) -> i32 {
            let v = &self.s.piano;
            127 - ((v.scroll_y + v.last_grid.height() * 0.5) / v.key_h) as i32 + up
        }

        fn at(&self, beat: f32, row: i32) -> Pos2 {
            let v = &self.s.piano;
            let g = v.last_grid;
            pos2(
                g.left() + beat * v.beat_px - v.scroll_x,
                g.top() + (127 - row) as f32 * v.key_h - v.scroll_y + v.key_h * 0.5,
            )
        }

        fn click(&mut self, pos: Pos2) {
            let (p, s) = (&mut self.p, &mut self.s);
            self.h.click(pos, |ui| show(ui, p, s));
        }

        fn drag(&mut self, path: &[Pos2], button: PointerButton) {
            let (p, s) = (&mut self.p, &mut self.s);
            self.h.drag(path, button, |ui| show(ui, p, s));
        }

        /// Keys only count with the pointer over the editor.
        fn key(&mut self, key: Key) {
            let centre = self.s.piano.last_grid.center();
            let (p, s) = (&mut self.p, &mut self.s);
            self.h.move_to(centre, |ui| show(ui, p, s));
            self.h.key(key, |ui| show(ui, p, s));
        }

        fn with(&mut self, modifiers: Modifiers, then: impl FnOnce(&mut Self)) {
            self.h.modifiers = modifiers;
            self.settle();
            then(self);
            self.h.modifiers = Modifiers::NONE;
            self.settle();
        }

        fn add(&mut self, start: f32, row: i32, len: f32) {
            self.p.tracks[self.t].regions[0].notes.push(Note {
                start,
                len,
                pitch: row as u8,
                vel: 90,
                muted: false,
                color: 0,
                slide: false,
            });
        }
    }

    #[test]
    fn a_press_places_a_note_at_once_and_a_drag_moves_it_with_the_last_length() {
        let mut r = Rig::new();
        let row = r.row(0);
        let (from, to) = (r.at(2.5, row), r.at(4.5, row + 2));
        let (p, s) = (&mut r.p, &mut r.s);
        r.h.press(from, PointerButton::Primary, |ui| show(ui, p, s));
        assert_eq!(r.notes().len(), 1, "it is there before letting go");
        assert_eq!(r.notes()[0].start, 2.0, "on the step it was pressed in");
        let (p, s) = (&mut r.p, &mut r.s);
        r.h.move_to(to, |ui| show(ui, p, s));
        r.h.frame(vec![], |ui| show(ui, p, s));
        assert_eq!(r.notes()[0].start, 4.0, "it moves, it does not get longer");
        assert_eq!(r.notes()[0].pitch as i32, row + 2);
        assert_eq!(r.notes()[0].len, 0.25);
        let (p, s) = (&mut r.p, &mut r.s);
        r.h.release(to, PointerButton::Primary, |ui| show(ui, p, s));
        r.settle();
        assert_eq!(r.notes().len(), 1);

        // Its end makes it longer, and the next note takes that length.
        r.p.tracks[r.t].regions[0].notes[0].len = 1.0;
        let row2 = row + 2;
        r.settle();
        let a = r.at(4.95, row2);
        let b = r.at(6.95, row2);
        r.drag(&[a, b], PointerButton::Primary);
        assert_eq!(r.notes()[0].len, 3.0);
        assert_eq!(r.notes()[0].start, 4.0);
        let empty = r.at(9.5, row);
        r.click(empty);
        assert_eq!(r.notes().len(), 2);
        assert_eq!(r.notes()[1].len, 3.0, "the last length");
    }

    #[test]
    fn the_right_button_deletes_what_it_is_dragged_across() {
        let mut r = Rig::new();
        let row = r.row(0);
        for beat in 0..4 {
            r.add(beat as f32, row, 0.5);
        }
        r.add(8.0, row, 0.5);
        r.settle();
        let path: Vec<Pos2> = (0..8).map(|k| r.at(k as f32 * 0.5 + 0.2, row)).collect();
        r.drag(&path, PointerButton::Secondary);
        assert_eq!(r.notes().len(), 1);
        assert_eq!(r.notes()[0].start, 8.0, "only what it crossed");
    }

    #[test]
    fn the_tool_keys_pick_tools_and_each_tool_does_its_job() {
        let mut r = Rig::new();
        let row = r.row(0);
        assert_eq!(r.s.piano.tool, Tool::Draw);

        // Delete: the left button deletes.
        r.add(1.0, row, 0.5);
        r.settle();
        r.key(Key::D);
        assert_eq!(r.s.piano.tool, Tool::Delete);
        let at = r.at(1.2, row);
        r.click(at);
        assert!(r.notes().is_empty());

        // Mute: across notes, all to the same state; muted notes stay.
        r.add(1.0, row, 0.5);
        r.add(2.0, row, 0.5);
        r.settle();
        r.key(Key::T);
        let path: Vec<Pos2> = (0..5).map(|k| r.at(1.1 + k as f32 * 0.25, row)).collect();
        r.drag(&path, PointerButton::Primary);
        assert!(r.notes().iter().all(|n| n.muted));
        assert_eq!(r.notes().len(), 2);
        let at = r.at(1.2, row);
        r.click(at);
        assert!(!r.notes()[0].muted && r.notes()[1].muted, "and back");

        // Slice: cuts where it is let go, on the snap.
        r.p.tracks[r.t].regions[0].notes.clear();
        r.add(0.0, row, 4.0);
        r.settle();
        r.key(Key::C);
        let (a, b) = (r.at(1.1, row + 3), r.at(2.1, row + 3));
        r.drag(&[a, b], PointerButton::Primary);
        assert_eq!(r.notes().len(), 2);
        assert_eq!((r.notes()[0].len, r.notes()[1].start), (2.0, 2.0));

        // Paint: notes one after the other, as long as the last one.
        r.p.tracks[r.t].regions[0].notes.clear();
        r.s.piano.last_len = 1.0;
        r.settle();
        r.key(Key::B);
        let (a, b) = (r.at(2.1, row), r.at(5.5, row));
        r.drag(&[a, b], PointerButton::Primary);
        let mut starts: Vec<f32> = r.notes().iter().map(|n| n.start).collect();
        starts.sort_by(f32::total_cmp);
        assert_eq!(starts, [2.0, 3.0, 4.0, 5.0]);

        // Select: a box, and no note is made.
        r.key(Key::E);
        let (a, b) = (r.at(1.5, row + 2), r.at(3.5, row - 2));
        r.drag(&[a, b], PointerButton::Primary);
        assert_eq!(r.notes().len(), 4);
        assert_eq!(r.s.piano.selected.len(), 2);
        r.key(Key::P);
        assert_eq!(r.s.piano.tool, Tool::Draw);
    }

    #[test]
    fn ctrl_selects_while_drawing_and_shift_drags_copies() {
        let mut r = Rig::new();
        let row = r.row(0);
        r.add(1.0, row, 0.5);
        r.add(2.0, row, 0.5);
        r.add(6.0, row, 0.5);
        r.settle();
        let (a, b) = (r.at(0.5, row + 2), r.at(3.5, row - 2));
        r.with(Modifiers::COMMAND, |r| {
            r.drag(&[a, b], PointerButton::Primary)
        });
        assert_eq!(r.notes().len(), 3, "no note is made");
        let mut picked = r.s.piano.selected.clone();
        picked.sort_unstable();
        assert_eq!(picked, [0, 1]);

        // A selected note drags the whole selection.
        let (a, b) = (r.at(1.2, row), r.at(2.2, row + 1));
        r.drag(&[a, b], PointerButton::Primary);
        assert_eq!((r.notes()[0].start, r.notes()[1].start), (2.0, 3.0));
        assert_eq!(r.notes()[0].pitch as i32, row + 1);
        assert_eq!(r.notes()[2].start, 6.0, "the other stays");

        // Shift drags copies and leaves the originals.
        let (a, b) = (r.at(2.2, row + 1), r.at(10.2, row + 1));
        r.with(Modifiers::SHIFT, |r| {
            r.drag(&[a, b], PointerButton::Primary)
        });
        assert_eq!(r.notes().len(), 5);
        assert_eq!(r.notes()[0].start, 2.0);
        assert_eq!((r.notes()[3].start, r.notes()[4].start), (10.0, 11.0));

        // Alt turns the snap off.
        let c = r.at(6.1, row);
        let d = r.at(6.6, row);
        r.with(Modifiers::ALT, |r| r.drag(&[c, d], PointerButton::Primary));
        let moved = r
            .notes()
            .iter()
            .find(|n| n.pitch as i32 == row && n.start > 6.0);
        let moved = moved.unwrap().start;
        assert!((moved - 6.5).abs() < 0.03, "{moved}");
    }

    #[test]
    fn the_editing_shortcuts_work_on_the_selection_or_on_everything() {
        let mut r = Rig::new();
        let row = r.row(0);
        r.add(0.0, row, 0.25);
        r.add(1.0, row, 0.25);
        r.settle();
        // Nothing selected: Ctrl+L takes all the notes.
        r.with(Modifiers::COMMAND, |r| r.key(Key::L));
        assert_eq!(r.notes()[0].len, 1.0);

        // Ctrl+B copies right after; the copies are selected.
        r.s.piano.selected = vec![0];
        r.settle();
        r.with(Modifiers::COMMAND, |r| r.key(Key::B));
        assert_eq!(r.notes().len(), 3);
        assert_eq!(r.notes()[2].start, 1.0);
        assert_eq!(r.s.piano.selected, [2]);

        // Ctrl+Up an octave, Shift+Up a semitone, Shift+Right a step.
        r.with(Modifiers::COMMAND, |r| r.key(Key::ArrowUp));
        assert_eq!(r.notes()[2].pitch as i32, row + 12);
        r.with(Modifiers::SHIFT, |r| {
            r.key(Key::ArrowUp);
            r.key(Key::ArrowRight);
        });
        assert_eq!(r.notes()[2].pitch as i32, row + 13);
        assert_eq!(r.notes()[2].start, 2.0);

        // Alt+M mutes, Alt+Shift+M brings back; Shift+I selects the rest.
        r.with(Modifiers::ALT, |r| r.key(Key::M));
        assert!(r.notes()[2].muted && !r.notes()[0].muted);
        let both = Modifiers::ALT | Modifiers::SHIFT;
        r.with(both, |r| r.key(Key::M));
        assert!(!r.notes()[2].muted);
        r.with(Modifiers::SHIFT, |r| r.key(Key::I));
        assert_eq!(r.s.piano.selected, [0, 1]);

        // Ctrl+X, then Ctrl+V at the playhead.
        r.with(Modifiers::COMMAND, |r| r.key(Key::X));
        assert_eq!(r.notes().len(), 1);
        let origin = r.p.region_start_beats(&r.p.tracks[r.t].regions[0]);
        r.p.position_beats = origin + 8.0;
        r.with(Modifiers::COMMAND, |r| r.key(Key::V));
        assert_eq!(r.notes().len(), 3);
        assert_eq!(r.notes()[1].start, 8.0);
        // Ctrl+Up on a selection stays in the piano range; Ctrl+A takes all.
        r.with(Modifiers::COMMAND, |r| r.key(Key::A));
        assert_eq!(r.s.piano.selected.len(), 3);
    }

    #[test]
    fn a_muted_note_is_saved_and_does_not_play() {
        let mut r = Rig::new();
        let row = r.row(0);
        r.add(1.0, row, 0.5);
        r.add(2.0, row, 0.5);
        r.p.tracks[r.t].regions[0].notes[0].muted = true;
        let t = &r.p.tracks[r.t];
        let played = crate::engine_sync::region_notes(t, &t.regions[0], 0.0, 0);
        assert_eq!(played.len(), 1);
        assert_eq!(played[0].start_beat, 2.0);
        let doc = crate::project_io::to_doc(&r.p).0;
        assert!(doc.tracks[r.t].regions[0].notes[0].muted);
        assert!(!doc.tracks[r.t].regions[0].notes[1].muted);
    }

    #[test]
    fn an_empty_instrument_track_gets_a_region_to_draw_in_and_an_audio_track_offers_one() {
        let mut p = Project::empty();
        let keys = p.add_track(TrackKind::Instrument);
        let mut s = UiState::new();
        let mut h = Harness::new(1400.0, 800.0);
        h.settle(|ui| show(ui, &mut p, &mut s));
        assert_eq!(p.tracks[keys].regions.len(), 1, "opened on an empty track");
        assert!(target_region(&p, &s).is_some());
        h.settle(|ui| show(ui, &mut p, &mut s));
        assert_eq!(p.tracks[keys].regions.len(), 1, "only the one");

        // On an audio track nothing is made, but a click makes an instrument track.
        let audio = p.add_track(TrackKind::Audio);
        s.tracks.selected = None;
        p.selected_track = audio;
        let tracks = p.tracks.len();
        h.settle(|ui| show(ui, &mut p, &mut s));
        assert!(p.tracks[audio].regions.is_empty());
        assert_eq!(p.tracks.len(), tracks);
    }

    #[test]
    fn ghost_notes_are_those_of_the_other_regions_and_alt_v_shows_them() {
        let mut r = Rig::new();
        let row = r.row(0);
        r.add(1.0, row, 0.5);
        let others = ghost_notes(&r.p, r.t, 0);
        assert!(others
            .iter()
            .all(|(t, region, _)| (*t, *region) != (r.t, 0)));
        assert!(!others.is_empty(), "the demo has other regions");
        assert!(!r.s.piano.ghosts);
        r.with(Modifiers::ALT, |r| r.key(Key::V));
        assert!(r.s.piano.ghosts);
        r.settle();
    }

    /// A point on the ruler at `beat`.
    fn on_ruler(r: &Rig, beat: f32) -> Pos2 {
        let v = &r.s.piano;
        let g = v.last_grid;
        pos2(
            g.left() + beat * v.beat_px - v.scroll_x,
            g.top() - REGION_H - RULER_H / 2.0,
        )
    }

    /// A point in the lane under the notes, `level` of the way up.
    fn in_lane(r: &Rig, beat: f32, level: f32) -> Pos2 {
        let v = &r.s.piano;
        let g = v.last_grid;
        let top = g.bottom() + SB_H + 4.0;
        pos2(
            g.left() + beat * v.beat_px - v.scroll_x,
            top + (1.0 - level) * VEL_H,
        )
    }

    #[test]
    fn right_dragging_the_ruler_selects_the_time_and_the_notes_in_it() {
        let mut r = Rig::new();
        let row = r.row(0);
        r.add(1.0, row, 0.5);
        r.add(2.0, row, 0.5);
        r.add(5.0, row, 0.5);
        r.settle();
        let (a, b) = (on_ruler(&r, 0.9), on_ruler(&r, 3.1));
        r.drag(&[a, b], PointerButton::Secondary);
        assert_eq!(r.s.piano.time_sel, Some((1.0, 3.0)));
        assert_eq!(r.notes().len(), 3, "the right button deletes nothing here");
        let mut picked = r.s.piano.selected.clone();
        picked.sort_unstable();
        assert_eq!(picked, [0, 1]);

        // The tools work on what it took, and an empty one takes nothing, never every note.
        r.with(Modifiers::SHIFT, |r| r.key(Key::ArrowUp));
        assert_eq!(r.notes()[0].pitch as i32, row + 1);
        assert_eq!(r.notes()[2].pitch as i32, row);
        let (a, b) = (on_ruler(&r, 9.0), on_ruler(&r, 11.0));
        r.drag(&[a, b], PointerButton::Secondary);
        assert!(r.s.piano.selected.is_empty());
        r.with(Modifiers::SHIFT, |r| r.key(Key::ArrowUp));
        assert_eq!(r.notes()[2].pitch as i32, row, "nothing moved");

        // Escape lets it go.
        r.key(Key::Escape);
        assert_eq!(r.s.piano.time_sel, None);
    }

    #[test]
    fn time_can_be_deleted_and_inserted_by_key() {
        let mut r = Rig::new();
        let row = r.row(0);
        r.add(1.0, row, 0.5);
        r.add(6.0, row, 0.5);
        r.settle();
        let (a, b) = (on_ruler(&r, 2.0), on_ruler(&r, 4.0));
        r.drag(&[a, b], PointerButton::Secondary);
        r.with(Modifiers::COMMAND, |r| r.key(Key::Insert));
        assert_eq!(r.notes()[1].start, 8.0);
        r.with(Modifiers::COMMAND, |r| r.key(Key::Delete));
        assert_eq!(r.notes()[1].start, 6.0);
        assert_eq!(r.notes()[0].start, 1.0);
        assert_eq!(r.s.piano.time_sel, None);
    }

    #[test]
    fn the_lane_draws_controllers_that_reach_a_plugin_and_are_saved() {
        let mut r = Rig::new();
        r.s.piano.lane = Lane::Control(1);
        r.settle();
        let (a, b) = (in_lane(&r, 1.0, 0.0), in_lane(&r, 2.0, 1.0));
        r.drag(&[a, b], PointerButton::Primary);
        let controls = r.p.tracks[r.t].regions[0].controls.clone();
        assert!(controls.len() > 3, "{controls:?}");
        assert!(controls.iter().all(|c| c.num == 1));
        assert!(controls.first().unwrap().val < 20);
        assert!(controls.last().unwrap().val > 110);
        assert!(r.notes().is_empty(), "no notes come of it");
        // The right button rubs them out.
        let (c, d) = (in_lane(&r, 0.5, 0.5), in_lane(&r, 2.5, 0.5));
        r.drag(&[c, d], PointerButton::Secondary);
        assert!(r.p.tracks[r.t].regions[0].controls.is_empty());
        // Velocity is left to itself, and what is drawn goes in the file.
        r.s.piano.lane = Lane::Velocity;
        r.p.tracks[r.t].regions[0].controls = controls;
        let doc = crate::project_io::to_doc(&r.p).0;
        assert!(!doc.tracks[r.t].regions[0].controls.is_empty());
    }

    #[test]
    fn quantize_uses_the_groove_picked_in_place_of_the_grid() {
        let mut r = Rig::new();
        let row = r.row(0);
        r.add(0.5, row, 0.25);
        r.p.grooves.push(ovrtone_project::GrooveDoc {
            name: "Late".into(),
            length: 4.0,
            hits: vec![0.0, 0.6, 1.0],
        });
        r.settle();
        r.s.piano.groove = Some(0);
        r.with(Modifiers::COMMAND, |r| r.key(Key::Q));
        assert!((r.notes()[0].start - 0.6).abs() < 1e-3, "{:?}", r.notes());
        // Gone from the project, the grid is back.
        r.p.grooves.clear();
        r.settle();
        assert_eq!(r.s.piano.groove, None);
    }

    #[test]
    fn while_typing_is_on_the_keys_play_and_shift_picks_the_tools() {
        let mut r = Rig::new();
        r.s.typing.on = true;
        // The app takes the note keys before the piano roll sees them, so here D reaches it only
        // with Shift held, which is what picks the tool.
        r.with(Modifiers::SHIFT, |r| r.key(Key::D));
        assert_eq!(r.s.piano.tool, Tool::Delete);
        r.with(Modifiers::SHIFT, |r| r.key(Key::P));
        assert_eq!(r.s.piano.tool, Tool::Draw);
    }
}

/// Step input: puts the notes played on a MIDI keyboard into region `(t, r)` at the step cursor,
/// moving the cursor on by a grid step for each new step. Notes that come within a moment of one
/// another form a chord at one step. The region grows to hold them.
pub fn step_input(p: &mut crate::model::Project, v: &mut PianoView, (t, r): (usize, usize), played: &[(u8, u8)]) {
    const CHORD: std::time::Duration = std::time::Duration::from_millis(90);
    if played.is_empty() {
        return;
    }
    let step = v.step_len.max(1.0 / 64.0);
    let now = std::time::Instant::now();
    for &(pitch, vel) in played {
        let at = match v.step_chord {
            Some((at, last)) if now.duration_since(last) <= CHORD => at,
            _ => {
                let at = v.step_at;
                v.step_at += step;
                at
            }
        };
        v.step_chord = Some((at, now));
        let Some(region) = p.tracks.get_mut(t).and_then(|x| x.regions.get_mut(r)) else {
            return;
        };
        region.notes.push(Note {
            start: at,
            len: step * 0.95,
            pitch: pitch.min(127),
            vel: vel.clamp(1, 127),
            muted: false,
            color: 0,
            slide: false,
        });
        region.notes.sort_by(|a, b| a.start.total_cmp(&b.start));
    }
    grow_to_hold(p, t, r, v.step_at);
    p.song_rev += 1;
}

/// Makes region `(t, r)` long enough to reach `beats` from its start, in whole bars.
fn grow_to_hold(p: &mut crate::model::Project, t: usize, r: usize, beats: f32) {
    let Some(region) = p.tracks.get(t).and_then(|x| x.regions.get(r)) else {
        return;
    };
    let start = p.region_start_beats(region);
    let end_bar = p.beats_to_bar(start + f64::from(beats)).ceil();
    let want = (end_bar - (f64::from(region.start_bar) - 1.0)) as f32;
    if let Some(region) = p.tracks.get_mut(t).and_then(|x| x.regions.get_mut(r)) {
        if want > region.len_bars {
            region.len_bars = want;
        }
    }
}

/// The arrow keys and Backspace while step input is on: a rest, a step back, or the last step
/// taken back (its notes removed).
pub fn step_keys(p: &mut crate::model::Project, v: &mut PianoView, (t, r): (usize, usize), ctx: &egui::Context) {
    if !v.step_input || ctx.egui_wants_keyboard_input() {
        return;
    }
    let (right, left, back) = ctx.input(|i| {
        (
            i.key_pressed(Key::ArrowRight) && i.modifiers.is_none(),
            i.key_pressed(Key::ArrowLeft) && i.modifiers.is_none(),
            i.key_pressed(Key::Backspace),
        )
    });
    let step = v.step_len.max(1.0 / 64.0);
    if right {
        v.step_at += step;
        v.step_chord = None;
        grow_to_hold(p, t, r, v.step_at);
    }
    if left {
        v.step_at = (v.step_at - step).max(0.0);
        v.step_chord = None;
    }
    if back {
        v.step_at = (v.step_at - step).max(0.0);
        v.step_chord = None;
        let from = v.step_at;
        if let Some(region) = p.tracks.get_mut(t).and_then(|x| x.regions.get_mut(r)) {
            let before = region.notes.len();
            region
                .notes
                .retain(|n| n.start < from - 1e-4 || n.start >= from + step - 1e-4);
            if region.notes.len() != before {
                p.song_rev += 1;
            }
        }
    }
}
