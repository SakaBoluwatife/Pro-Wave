//! Mixer (Logic Pro: Mixer in the main window): local toolbar, row labels and channel strips
//! with Setting, Gain Reduction, EQ, Input, Audio FX, Sends, Output, Group, Automation, Pan,
//! dB readouts, fader with scale, stereo meter, M/S and the coloured name label.
//!
//! Works today: fader, pan, mute, solo (all reach the engine), real post-fader meters with
//! peak hold, strip selection, Single/Tracks/All, the strip-type filters, scrolling; the Audio
//! FX slots (pick an effect, open its window, bypass, reorder), the Gain Reduction meter and
//! the EQ curve, Sends to buses (with Sends on Faders) and the Aux strips that play them, and
//! the Automation modes (Read, Touch, Latch, Write) with the Automation editor. Groups are
//! drawn but not functional yet.
//!
//! [`channel_strip`] is also used by the Inspector's two strips.
use crate::i18n::{tr, trf};
use crate::model::{AutoMode, AutoTarget, Insert, Project, SendSlot, TrackKind, MAX_SENDS};
use crate::widgets::{
    back_chevron, blue, menu_title, popup_button, stroke, text_col, text_dim_col,
};
use crate::UiState;
use egui::{pos2, vec2, Align2, Color32, FontId, Painter, Rect, Sense, Stroke, StrokeKind, Ui};
use ovrtone_host::PluginInfo;
use std::collections::{BTreeMap, HashMap};

const TOOLBAR_H: f32 = 30.0;
const LABEL_W: f32 = 88.0;
const GAP: f32 = 2.0;

fn toolbar_bg_col() -> Color32 {
    crate::theme::toolbar()
}
fn mixer_bg_col() -> Color32 {
    crate::theme::mixer_bg()
}
fn strip_bg_col() -> Color32 {
    crate::theme::strip()
}
fn strip_sel_col() -> Color32 {
    crate::theme::strip_selected()
}
fn slot_empty_col() -> Color32 {
    crate::theme::slot_empty()
}
fn slot_gray_col() -> Color32 {
    crate::theme::slot()
}
/// A strip that makes no sound (muted, or another track is soloed): instruments, plug-ins
/// and sends turn grey, and so do the meters.
fn plugin_off_col() -> Color32 {
    crate::theme::slot_off()
}
fn meter_off_col() -> Color32 {
    crate::theme::meter_off()
}
const READ_GREEN: Color32 = crate::theme::GOOD;
const MUTE_ON: Color32 = crate::theme::MUTE;
const SOLO_ON: Color32 = crate::theme::SOLO;
const OUT_PURPLE: Color32 = Color32::from_rgb(0x8a, 0x3f, 0xc8);

/// Fader/meter scale marks in dB, top to bottom (Logic's channel strip scale).
const SCALE: [f32; 11] = [
    6.0, 3.0, 0.0, -3.0, -6.0, -10.0, -15.0, -20.0, -30.0, -40.0, -60.0,
];
pub(crate) const FILTERS: [&str; 8] = [
    "Audio",
    "Inst",
    "Aux",
    "Bus",
    "Input",
    "Output",
    "Master/VCA",
    "MIDI",
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MixerMode {
    Single,
    Tracks,
    All,
}

/// What a mixer strip's menu leaves to the Tracks area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StripRequest {
    PianoRoll(usize),
    Delete(usize),
    Color(usize),
    Duplicate(usize, bool),
}

pub struct MixerView {
    /// The insert slot at the top, when they do not all fit above the fader.
    pub fx_scroll: f32,
    /// The plugins on this computer, for the slots' menus.
    pub installed: std::sync::Arc<Vec<PluginInfo>>,
    /// A preset to save or load (`App` carries it out).
    pub preset_cmd: Option<crate::plugin_presets::PresetCmd>,
    pub strip_cmd: Option<crate::plugin_presets::StripCmd>,
    /// What the strip's menu wants a hardware control on (`UiState::midi_learn` takes it), and that
    /// it asked for the list of assignments.
    pub learn: Option<crate::model::LearnTarget>,
    pub learn_window: bool,
    /// Clip indicators to put out (`App` tells the engine).
    pub clip_resets: Vec<ClipReset>,
    /// The plugins showing the B side of A/B compare (`App` keeps this current).
    pub showing_b: Vec<u64>,
    /// View > Show Signal Flow Channel Strips: only the path of the selected strip.
    pub signal_flow: bool,
    /// Mix > Pre-Fader Metering.
    pub pre_fader: bool,
    pub strip_error: Option<String>,
    /// What went wrong putting one on, for the status line.
    pub message: Option<String>,
    pub mode: MixerMode,
    /// Visibility per strip type, in `FILTERS` order.
    pub filters: [bool; 8],
    pub wide: bool,
    pub scroll_x: f32,
    /// Peak hold per strip (linear), shared with the Inspector strips.
    pub peak_hold: HashMap<StripKey, f32>,
    /// When each of those was last reached, for a hold that lets go after a while.
    pub peak_hold_at: HashMap<StripKey, std::time::Instant>,
    /// The output strip's Bnc was pressed.
    pub want_bounce: bool,
    /// The effect whose window was opened last, and is in front: (track, slot).
    pub fx_window: Option<(usize, usize)>,
    /// Every effect window open, oldest first (the one in front among them).
    pub fx_open: Vec<(usize, usize)>,
    /// The window last brought to the front, so a click on a slot brings its window forward once.
    pub fx_front: Option<(usize, usize)>,
    /// What the window in front hears from its effect (the analyzer and its meters).
    pub listen: Option<crate::fx_ui::Listen>,
    /// A hosted plugin whose editor was asked for: its uid and its name. Run by `App`, which has
    /// the running plugins.
    pub open_plugin: Option<(u64, String)>,
    /// A track's strip was clicked: Delete now means that track.
    pub track_clicked: bool,
    /// What a strip's right-click menu asked of the Tracks area.
    pub strip_request: Option<StripRequest>,
    /// A click asked for the Spatial Audio window.
    pub open_spatial: bool,
    /// The renderer was asked for from Stereo Out's strip.
    pub open_renderer: bool,
    /// An effect copied from a slot, to paste into another: an Ovrtone effect, or a plugin.
    pub fx_clipboard: Option<Insert>,
    pub plugin_clipboard: Option<crate::model::PluginInsert>,
    /// A template to save or load, asked for from a strip's menu.
    pub file_task: Option<crate::file_commands::FileTask>,
    /// Auto Gain Staging was asked for from a strip's menu.
    pub open_gain_staging: bool,
    /// The Automation editor window.
    pub auto_editor: crate::automation::AutoEditor,
    /// Sends on Faders: the bus whose send levels the faders set.
    pub sends_on_faders: Option<u16>,
    /// A strip asked for a new Aux: its place, and whether as a send rather than its output.
    pub new_aux: Option<(usize, bool)>,
    /// A strip's MIDI FX slot was clicked.
    pub open_midi_fx: bool,
    /// A strip asked to be renamed.
    pub rename: Option<usize>,
    /// Edit > Copy Channel Strip.
    pub copied: Option<CopiedStrip>,
    /// The audio input device that the Input slots choose from.
    pub inputs: crate::audio_rec::InputChoice,
}

/// A channel strip's effects, sends, level and pan, as Copy Channel Strip keeps them.
#[derive(Clone)]
pub struct CopiedStrip {
    inserts: Vec<Insert>,
    plugins: Vec<crate::model::PluginInsert>,
    sends: Vec<SendSlot>,
    volume_db: f32,
    pan: f32,
}

/// Where track `t`'s sound goes: the strip, the Aux strips of the buses it reaches and of the ones
/// those reach, then Stereo Out if it gets there.
pub fn signal_path(p: &Project, t: usize) -> Vec<StripKey> {
    let mut path = vec![t];
    let mut k = 0;
    while k < path.len() {
        let track = &p.tracks[path[k]];
        let buses = track
            .sends
            .iter()
            .filter_map(|s| s.bus)
            .chain((track.output > 0).then_some(track.output));
        for bus in buses {
            for i in 0..p.tracks.len() {
                if p.aux_bus_of(i) == bus && !path.contains(&i) {
                    path.push(i);
                }
            }
        }
        k += 1;
    }
    // Stereo Out only if something on the way plays there. Sound can end on a bus nothing listens to,
    // or go only to another pair of outputs.
    let reaches_out = path
        .iter()
        .any(|i| p.tracks[*i].output == 0 && p.tracks[*i].hardware_out == 0);
    let mut keys: Vec<StripKey> = path.into_iter().map(StripKey::Track).collect();
    if reaches_out {
        keys.push(StripKey::StereoOut);
    }
    keys
}

impl CopiedStrip {
    /// The strip as it is now.
    pub fn of(t: &crate::model::Track) -> Self {
        Self {
            inserts: t.inserts.clone(),
            plugins: t.plugins.clone(),
            sends: t.sends.clone(),
            volume_db: t.volume_db,
            pan: t.pan,
        }
    }

    /// Puts it on a track. The plugins are new instances with the same settings.
    pub fn paste_on(&self, t: &mut crate::model::Track) {
        t.inserts = self.inserts.clone();
        t.plugins = self.plugins.clone();
        for plugin in &mut t.plugins {
            plugin.uid = crate::plugin_rack::new_uid();
        }
        t.sends = self.sends.clone();
        t.volume_db = self.volume_db;
        t.pan = self.pan;
    }
}

impl Default for MixerView {
    fn default() -> Self {
        Self {
            open_spatial: false,
            open_renderer: false,
            file_task: None,
            fx_clipboard: None,
            plugin_clipboard: None,
            open_gain_staging: false,
            fx_scroll: 0.0,
            installed: std::sync::Arc::default(),
            preset_cmd: None,
            learn: None,
            learn_window: false,
            clip_resets: Vec::new(),
            showing_b: Vec::new(),
            strip_cmd: None,
            signal_flow: false,
            pre_fader: false,
            strip_error: None,
            message: None,
            mode: MixerMode::Tracks,
            filters: [true; 8],
            wide: false,
            scroll_x: 0.0,
            peak_hold: HashMap::new(),
            peak_hold_at: HashMap::new(),
            want_bounce: false,
            fx_window: None,
            fx_open: Vec::new(),
            fx_front: None,
            listen: None,
            open_plugin: None,
            track_clicked: false,
            strip_request: None,
            auto_editor: Default::default(),
            sends_on_faders: None,
            new_aux: None,
            open_midi_fx: false,
            rename: None,
            copied: None,
            inputs: crate::audio_rec::InputChoice::default(),
        }
    }
}

/// Live meter values handed in by the app each frame.
#[derive(Default)]
pub struct Meters {
    pub tracks: Vec<(f32, f32)>,
    /// Each track's RMS, linear.
    pub rms: Vec<f32>,
    /// Gain reduction in dB of each track's compressors.
    pub reduction: Vec<f32>,
    pub master: (f32, f32),
    /// Each track's meter has gone over full scale since it was last reset.
    pub clips: Vec<bool>,
    pub master_clip: bool,
}

/// Which clip indicators to put out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipReset {
    Track(usize),
    Master,
    All,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum StripKey {
    Track(usize),
    StereoOut,
}

fn id(ns: &'static str, key: impl std::hash::Hash + std::fmt::Debug) -> egui::Id {
    egui::Id::unique((ns, key))
}

/// 0 (bottom, -60 dB and below) .. 1 (top, +6 dB).
pub(crate) fn db_to_frac(db: f32) -> f32 {
    if db >= SCALE[0] {
        return 1.0;
    }
    let n = SCALE.len() - 1;
    for i in 0..n {
        let (hi, lo) = (SCALE[i], SCALE[i + 1]);
        if db <= hi && db >= lo {
            let idx = i as f32 + (hi - db) / (hi - lo);
            return 1.0 - idx / n as f32;
        }
    }
    0.0
}

pub(crate) fn frac_to_db(f: f32) -> f32 {
    let n = SCALE.len() - 1;
    let idx = ((1.0 - f.clamp(0.0, 1.0)) * n as f32).min(n as f32 - 1e-4);
    let i = idx.floor() as usize;
    SCALE[i] + (SCALE[i + 1] - SCALE[i]) * (idx - i as f32)
}

pub(crate) fn lin_to_db(x: f32) -> f32 {
    if x <= 1e-6 {
        -120.0
    } else {
        20.0 * x.log10()
    }
}

/// Shortens `text` with a trailing "…" so it fits in `max_w` (Logic truncates strip names).
pub fn elide(g: &Painter, text: &str, font: &FontId, max_w: f32) -> String {
    let width = |t: &str| {
        g.layout_no_wrap(t.to_string(), font.clone(), Color32::WHITE)
            .size()
            .x
    };
    if width(text) <= max_w {
        return text.to_string();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>() + "…";
        if width(&candidate) <= max_w {
            return candidate;
        }
    }
    String::new()
}

fn slot(g: &Painter, r: Rect, text: &str, fill: Color32, color: Color32) {
    g.rect_filled(r, 3.0, fill);
    if !text.is_empty() {
        let font = FontId::proportional(10.5);
        let shown = elide(g, text, &font, r.width() - 6.0);
        g.text(r.center(), Align2::CENTER_CENTER, shown, font, color);
    }
}

fn small_button(
    ui: &Ui,
    g: &Painter,
    r: Rect,
    id: egui::Id,
    label: &str,
    on: bool,
    on_col: Color32,
) -> bool {
    let hint = match label {
        "M" => "Mute",
        "S" => "Solo",
        "R" => "Record Enable",
        "I" => "Input Monitoring",
        "" => "Bounce",
        _ => "",
    };
    let mut resp = ui.interact(r, id, Sense::click());
    if !hint.is_empty() {
        resp = resp.on_hover_text(hint);
    }
    g.rect_filled(r, 3.0, if on { on_col } else { slot_gray_col() });
    let txt = if on && on_col == SOLO_ON {
        Color32::BLACK
    } else {
        crate::theme::gray(0xe8)
    };
    g.text(
        r.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(10.5),
        txt,
    );
    resp.clicked()
}

/// What a fader or knob did this frame.
#[derive(Default, Clone, Copy)]
struct Moved {
    /// The new value, when it was dragged or double-clicked.
    value: Option<f32>,
    /// The gesture began / ended (a double click is both).
    started: bool,
    stopped: bool,
    /// The clip light was clicked; true with Alt held, for all of them.
    reset_clip: Option<bool>,
}

/// The pan knob at `c` showing `value` (-1..1); drag it, or double-click for the centre. Not
/// moved when `editable` is false (the strip plays its automation, or has no pan).
fn pan_knob(
    ui: &Ui,
    g: &Painter,
    c: egui::Pos2,
    id: egui::Id,
    value: f32,
    editable: bool,
) -> Moved {
    let r = Rect::from_center_size(c, vec2(26.0, 26.0));
    let mut out = Moved::default();
    if editable {
        let resp = ui
            .interact(r, id, Sense::click_and_drag())
            .on_hover_text("Drag to pan (Shift for fine), wheel to nudge, Alt-click or double-click for centre");
        if let Some(v) = crate::widgets::value_gesture(ui, &resp, value, 0.0, 0.01, 1.0 / 64.0) {
            out.value = Some(v.clamp(-1.0, 1.0));
            // A reset or a wheel notch is a whole move of its own.
            if !resp.dragged() {
                out.started = true;
                out.stopped = true;
            }
        }
        out.started |= resp.drag_started();
        out.stopped |= resp.drag_stopped();
    }
    crate::widgets::draw_knob(g, c, 12.0, value);
    // Off centre, the knob says how far.
    let amount = (value * 64.0).round() as i32;
    if amount != 0 {
        g.circle_filled(c, 8.5, crate::theme::gray(0x68));
        g.text(
            c,
            Align2::CENTER_CENTER,
            format!("{amount:+}"),
            FontId::proportional(9.5),
            Color32::BLACK,
        );
    }
    out
}

/// Logic-style meter colour for a level in dB: green, yellow from -6 dB, red above 0 dB.
pub(crate) fn meter_colour(db: f32) -> Color32 {
    if db > 0.0 {
        Color32::from_rgb(0xe8, 0x3a, 0x2c)
    } else if db > -6.0 {
        Color32::from_rgb(0xe6, 0xc8, 0x2a)
    } else {
        Color32::from_rgb(0x34, 0xc2, 0x3e)
    }
}

/// A level meter bar as Logic draws it: one smooth bar whose colour runs from green through
/// yellow (around -6 dB) and orange to red at full scale, with no segments. `bar` is the whole
/// well; `to_frac` places a level in dB along it (0 at the bottom or left, 1 at the top or right).
pub(crate) fn gradient_meter(
    g: &Painter,
    bar: Rect,
    vertical: bool,
    to_frac: &dyn Fn(f32) -> f32,
    level_db: f32,
    dim: bool,
) {
    let f = to_frac(level_db.min(12.0)).clamp(0.0, 1.0);
    if f <= 0.0 {
        return;
    }
    const STOPS: [(f32, [u8; 3]); 6] = [
        (-60.0, [0x1f, 0x9a, 0x38]),
        (-18.0, [0x36, 0xc4, 0x40]),
        (-6.0, [0xe2, 0xcc, 0x2c]),
        (-2.0, [0xf0, 0x8c, 0x26]),
        (0.0, [0xe8, 0x3a, 0x2c]),
        (12.0, [0xe8, 0x3a, 0x2c]),
    ];
    let colour = |rgb: [u8; 3]| {
        let c = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
        if dim {
            meter_off_col()
        } else {
            c
        }
    };
    // A point `t` (0..1) of the way along the bar, on the side `side` (0 or 1) across it.
    let at = |t: f32, side: f32| {
        if vertical {
            pos2(bar.left() + side * bar.width(), bar.bottom() - t * bar.height())
        } else {
            pos2(bar.left() + t * bar.width(), bar.top() + side * bar.height())
        }
    };
    let mut mesh = egui::Mesh::default();
    for pair in STOPS.windows(2) {
        let ((d0, c0), (d1, c1)) = (pair[0], pair[1]);
        let (f0, f1) = (to_frac(d0).max(0.0), to_frac(d1));
        if f0 >= f {
            break;
        }
        if f1 <= f0 {
            continue;
        }
        let end = f1.min(f);
        let (a, b) = (colour(c0), colour(c1));
        let b = if end < f1 { a.lerp_to_gamma(b, (end - f0) / (f1 - f0)) } else { b };
        let k = mesh.vertices.len() as u32;
        mesh.colored_vertex(at(f0, 0.0), a);
        mesh.colored_vertex(at(f0, 1.0), a);
        mesh.colored_vertex(at(end, 1.0), b);
        mesh.colored_vertex(at(end, 0.0), b);
        mesh.add_triangle(k, k + 1, k + 2);
        mesh.add_triangle(k, k + 2, k + 3);
    }
    g.add(egui::Shape::mesh(mesh));
}

/// Where the clip light of a meter goes, over the well that holds its bars.
fn clip_light_rect(housing: Rect) -> Rect {
    Rect::from_min_max(
        pos2(housing.left(), housing.top() - 6.0),
        pos2(housing.right(), housing.top() - 1.5),
    )
}

/// Fader (scale + slot + cap) and stereo meter with peak-hold lines, laid out like Logic's
/// channel strip: fader scale on the left, the cap, the meter scale numbers, then L/R bars.
/// Returns the new fader value in dB when it was dragged or double-clicked.
#[allow(clippy::too_many_arguments)]
fn fader_and_meter(
    ui: &Ui,
    g: &Painter,
    area: Rect,
    id: egui::Id,
    db: f32,
    editable: bool,
    peaks: (f32, f32),
    rms: f32,
    inactive: bool,
    clipped: bool,
) -> Moved {
    let top = area.top() + 8.0;
    let bottom = area.bottom() - 6.0;
    let h = (bottom - top).max(1.0);
    let y_of = |f: f32| bottom - f * h;
    let w = area.width();
    let scale_col = crate::theme::gray(0x6c);
    let tiny = FontId::proportional(6.5);

    // Columns, proportional to the strip width (Logic: fader left of centre, meter right).
    let fader_x = area.left() + (w * 0.30).max(22.0);
    let num_x = area.left() + w * 0.56;
    let bar_w = 5.0;
    let meter_left = (area.left() + w * 0.66).min(area.right() - 2.0 * bar_w - 6.0);

    // Fader scale: tick marks with small numbers, left of the slot.
    for mark in SCALE {
        let y = y_of(db_to_frac(mark));
        let tick_len = if mark == 0.0 { 6.0 } else { 4.0 };
        let tx = fader_x - 13.0;
        g.line_segment(
            [pos2(tx - tick_len, y), pos2(tx, y)],
            Stroke::new(1.0, scale_col),
        );
        if w >= 70.0 {
            g.text(
                pos2(tx - tick_len - 2.0, y),
                Align2::RIGHT_CENTER,
                format!("{}", mark.abs() as i32),
                tiny.clone(),
                scale_col,
            );
        }
        // Meter scale numbers between the fader and the meter.
        g.text(
            pos2(num_x, y),
            Align2::CENTER_CENTER,
            format!("{}", mark.abs() as i32),
            tiny.clone(),
            scale_col,
        );
    }

    // Fader slot.
    let slot_r = Rect::from_min_max(pos2(fader_x - 2.0, top), pos2(fader_x + 2.0, bottom));
    g.rect_filled(slot_r, 2.0, crate::theme::gray(0x0b));
    g.line_segment(
        [slot_r.right_top(), slot_r.right_bottom()],
        Stroke::new(1.0, crate::theme::gray(0x2b)),
    );

    // Meter bars with peak-hold lines (hold falls back at 12 dB/s).
    let hold_id = id.with("hold");
    let dt = ui.input(|i| i.stable_dt).min(0.1);
    let mut hold: (f32, f32) = ui
        .ctx()
        .data(|d| d.get_temp(hold_id).unwrap_or((-120.0, -120.0)));
    let levels = (lin_to_db(peaks.0), lin_to_db(peaks.1));
    hold.0 = levels.0.max(hold.0 - 12.0 * dt);
    hold.1 = levels.1.max(hold.1 - 12.0 * dt);
    ui.ctx().data_mut(|d| d.insert_temp(hold_id, hold));
    // One dark housing holds both bars (Logic draws L and R side by side in one well).
    let housing = Rect::from_min_max(
        pos2(meter_left - 2.0, top - 1.0),
        pos2(meter_left + 2.0 * bar_w + 3.0, bottom + 1.0),
    );
    g.rect_filled(housing, 2.5, crate::theme::gray(0x09));
    g.rect_stroke(
        housing,
        2.5,
        Stroke::new(1.0, crate::theme::gray(0x19)),
        StrokeKind::Inside,
    );
    // The clip light above the bars: on since the meter last went over full scale.
    let light = clip_light_rect(housing);
    let light_color = if clipped {
        Color32::from_rgb(0xf0, 0x30, 0x24)
    } else {
        crate::theme::gray(0x14)
    };
    g.rect_filled(light, 1.0, light_color);
    for (i, (lvl, hd)) in [(levels.0, hold.0), (levels.1, hold.1)]
        .into_iter()
        .enumerate()
    {
        let x0 = meter_left + i as f32 * (bar_w + 1.0);
        let bar = Rect::from_min_max(pos2(x0, top), pos2(x0 + bar_w, bottom));
        g.rect_filled(bar, 0.5, crate::theme::gray(0x04));
        gradient_meter(g, bar, true, &db_to_frac, lvl, inactive);
        if hd > -60.0 {
            let y = y_of(db_to_frac(hd));
            g.line_segment(
                [pos2(bar.left(), y), pos2(bar.right(), y)],
                Stroke::new(
                    1.5,
                    if inactive {
                        meter_off_col()
                    } else {
                        meter_colour(hd)
                    }
                    .lerp_to_gamma(Color32::WHITE, 0.4),
                ),
            );
        }
    }
    // RMS: a bright line across both bars at the average level, with the body below it lit.
    let rms_db = lin_to_db(rms);
    if rms_db > -60.0 && !inactive {
        let y = y_of(db_to_frac(rms_db));
        let both = Rect::from_min_max(
            pos2(meter_left, y),
            pos2(meter_left + 2.0 * bar_w + 1.0, bottom),
        );
        g.rect_filled(both, 0.0, Color32::from_white_alpha(28));
        g.line_segment(
            [pos2(both.left(), y), pos2(both.right(), y)],
            Stroke::new(1.5, Color32::from_white_alpha(200)),
        );
    }
    if levels.0 > -60.0 || levels.1 > -60.0 || hold.0 > -60.0 || hold.1 > -60.0 {
        ui.ctx().request_repaint();
    }

    // Fader cap: light grey block with a dark centre groove and fine ridges.
    let cap_y = y_of(db_to_frac(db));
    let cap = Rect::from_center_size(pos2(fader_x, cap_y), vec2(16.0, 27.0));
    let mut out = Moved::default();
    if clipped {
        let resp = ui
            .interact(light, id.with("clip"), Sense::click())
            .on_hover_text_at_pointer(
                "This meter went over 0 dB. Click to reset, Alt-click for all",
            );
        if resp.clicked() {
            out.reset_clip = Some(ui.input(|i| i.modifiers.alt));
        }
    }
    if editable {
        let hit = Rect::from_min_max(
            pos2(fader_x - 14.0, top - 17.0),
            pos2(fader_x + 14.0, bottom + 17.0),
        );
        let resp = ui.interact(hit, id, Sense::click_and_drag());
        // Moved along the fader's own travel, so a drag follows the pointer; reset to 0 dB.
        let frac = db_to_frac(db);
        let reset = db_to_frac(0.0);
        if let Some(f) = crate::widgets::value_gesture(ui, &resp, frac, reset, 1.0 / h, 0.01) {
            out.value = Some(if f == reset { 0.0 } else { frac_to_db(f).clamp(-60.0, 6.0) });
            if !resp.dragged() {
                out.started = true;
                out.stopped = true;
            }
        }
        out.started |= resp.drag_started();
        out.stopped |= resp.drag_stopped();
    }
    // Logic's cap: a narrow metal block. Soft shadow, silver body that is a little darker at the
    // edges, a dark groove across the middle with a bright line under it, and engraved ridges
    // (a dark line with a light line under it) above and below.
    g.rect_filled(
        cap.translate(vec2(0.5, 1.5)),
        2.0,
        Color32::from_black_alpha(110),
    );
    g.rect_filled(cap, 2.0, crate::theme::gray(0x88));
    g.rect_filled(
        Rect::from_min_max(
            pos2(cap.left() + 1.5, cap.top() + 1.0),
            pos2(cap.right() - 1.5, cap.center().y),
        ),
        egui::CornerRadius {
            nw: 1,
            ne: 1,
            sw: 0,
            se: 0,
        },
        crate::theme::gray(0xd4),
    );
    g.rect_filled(
        Rect::from_min_max(
            pos2(cap.left() + 1.5, cap.center().y),
            pos2(cap.right() - 1.5, cap.bottom() - 1.0),
        ),
        egui::CornerRadius {
            nw: 0,
            ne: 0,
            sw: 1,
            se: 1,
        },
        crate::theme::gray(0xb2),
    );
    g.rect_stroke(
        cap,
        2.0,
        Stroke::new(1.0, crate::theme::gray(0x25)),
        StrokeKind::Inside,
    );
    for dy in [-9.0f32, -6.0, -3.0, 4.5, 7.5, 10.5] {
        g.line_segment(
            [
                pos2(cap.left() + 2.5, cap_y + dy),
                pos2(cap.right() - 2.5, cap_y + dy),
            ],
            Stroke::new(0.8, crate::theme::gray(0x50)),
        );
        g.line_segment(
            [
                pos2(cap.left() + 2.5, cap_y + dy + 1.0),
                pos2(cap.right() - 2.5, cap_y + dy + 1.0),
            ],
            Stroke::new(0.6, crate::theme::gray(0xe4)),
        );
    }
    g.line_segment(
        [
            pos2(cap.left() + 1.0, cap_y),
            pos2(cap.right() - 1.0, cap_y),
        ],
        Stroke::new(1.8, crate::theme::border()),
    );
    g.line_segment(
        [
            pos2(cap.left() + 1.0, cap_y + 1.4),
            pos2(cap.right() - 1.0, cap_y + 1.4),
        ],
        Stroke::new(0.8, crate::theme::gray(0xee)),
    );
    out
}

/// Vertical positions of the strip rows (shared by every strip so rows line up).
pub struct Rows {
    /// Insert slots there are, and the first shown, when not all of them fit.
    pub(crate) fx_total: usize,
    fx_first: usize,
    setting: f32,
    gain_red: f32,
    midi_fx: f32,
    icon: f32,
    input: f32,
    pub(crate) fx: f32,
    pub(crate) fx_rows: usize,
    sends: f32,
    send_rows: usize,
    output: f32,
    group: f32,
    automation: f32,
    pan: f32,
    db: f32,
    fader_top: f32,
}

/// The height a fader is never squeezed below: the insert slots scroll first.
const MIN_FADER_H: f32 = 110.0;
/// What goes under the fader: the Mute and Solo buttons and the name.
const BELOW_FADER: f32 = 66.0;

/// The rows for a strip between `top` and `bottom`. The insert slots that do not leave room for the
/// fader are scrolled to, starting from slot `scroll`.
pub fn rows_fit(top: f32, bottom: f32, fx_rows: usize, send_rows: usize, scroll: f32) -> Rows {
    let total = fx_rows.max(2);
    let full = rows(top, total, send_rows);
    let spare = bottom - BELOW_FADER - MIN_FADER_H - full.fader_top;
    if spare >= 0.0 {
        return full;
    }
    // At least two, so the last effect and the plus stay in sight; the wheel reaches the rest. Three
    // took the fader below its least height in a short mixer once the strip had grown.
    let shown = total
        .saturating_sub((-spare / 17.0).ceil() as usize)
        .max(2)
        .min(total);
    let mut fit = rows(top, shown, send_rows);
    fit.fx_total = total;
    // Counted back from the end: the newest slots and the one with the plus are in sight, and the
    // wheel goes back to the first ones.
    fit.fx_first = (total - shown).saturating_sub(scroll.max(0.0).round() as usize);
    fit
}

pub fn rows(top: f32, fx_rows: usize, send_rows: usize) -> Rows {
    let fx_rows = fx_rows.max(2);
    let send_rows = send_rows.clamp(1, MAX_SENDS);
    let setting = top + 6.0;
    let gain_red = setting + 20.0;
    let midi_fx = gain_red + 12.0;
    let input = midi_fx + 20.0;
    // Room under the input for the line that parts an instrument from its effects.
    let fx = input + 32.0;
    let sends = fx + fx_rows as f32 * 17.0 + 4.0;
    let output = sends + send_rows as f32 * 17.0 + 3.0;
    let group = output + 20.0;
    let automation = group + 20.0;
    let icon = automation + 22.0;
    let pan = icon + 28.0;
    let db = pan + 30.0;
    Rows {
        fx_total: fx_rows,
        fx_first: 0,
        setting,
        gain_red,
        midi_fx,
        icon,
        input,
        fx,
        fx_rows,
        sends,
        send_rows,
        output,
        group,
        automation,
        pan,
        db,
        fader_top: db + 18.0,
    }
}

/// Audio FX rows needed so every strip in `keys` fits (inserts + one empty slot).
pub fn fx_rows_for(p: &Project, keys: &[StripKey]) -> usize {
    keys.iter()
        .map(|k| match k {
            StripKey::Track(i) => p.tracks.get(*i).map_or(0, |t| {
                crate::model::next_plugin_row(t.inserts.len(), &t.plugins)
            }),
            StripKey::StereoOut => {
                crate::model::next_plugin_row(p.master_strip.inserts.len(), &p.master_plugins)
            }
        })
        .max()
        .unwrap_or(0)
        // The row with the plus: one more appears each time a slot is used.
        + 1
}

/// Send slots needed so every strip in `keys` fits (its sends + one empty slot).
pub fn send_rows_for(p: &Project, keys: &[StripKey]) -> usize {
    keys.iter()
        .map(|k| match k {
            StripKey::Track(i) => p.tracks.get(*i).map_or(0, |t| t.sends.len()),
            _ => 0,
        })
        .max()
        .unwrap_or(0)
        + 1
}

/// The strip's send slots, one per row: the sends it has, then one empty slot to add another.
/// `x`, `w` and `top` place the rows (see [`Rows`]).
#[allow(clippy::too_many_arguments)]
fn send_slots(
    ui: &Ui,
    g: &Painter,
    area: Rect,
    top: f32,
    rows: usize,
    sends: &mut Vec<SendSlot>,
    uid: egui::Id,
    inactive: bool,
    buses: &[(u16, String)],
    new_aux: &mut bool,
) {
    let mut remove = None;
    for k in 0..rows {
        let r = Rect::from_min_size(
            pos2(area.left(), top + k as f32 * 17.0),
            vec2(area.width(), 16.0),
        );
        if k < sends.len() {
            if !send_slot(ui, g, r, &mut sends[k], uid.with(k), inactive, buses) {
                remove = Some(k);
            }
        } else if k == sends.len() && sends.len() < MAX_SENDS {
            let resp = ui.interact(r, uid.with(("empty", k)), Sense::click());
            let resp = resp.on_hover_text("Add a send");
            slot(g, r, "+", slot_empty_col(), text_col());
            let mut picked = None;
            egui::Popup::menu(&resp).show(|ui| {
                ui.set_min_width(130.0);
                if ui.button("New Aux").clicked() {
                    *new_aux = true;
                    ui.close();
                }
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .show(ui, |ui| {
                        for (b, name) in buses {
                            if ui.button(format!("Bus {b}: {name}")).clicked() {
                                picked = Some(*b);
                                ui.close();
                            }
                        }
                    });
            });
            if let Some(bus) = picked {
                sends.push(SendSlot {
                    bus: Some(bus),
                    level: 0.5,
                    pre: false,
                });
            }
        }
    }
    if let Some(k) = remove {
        sends.remove(k);
    }
}

/// Logic's send slot: a blue "Bus N" button with a round level knob beside it. Click the slot
/// to pick a bus; the eye marks a pre-fader send. The engine copies the strip's signal to the
/// bus, where the Aux strip that listens to it plays it. Returns false when the user chose
/// "No Send".
fn send_slot(
    ui: &Ui,
    g: &Painter,
    r: Rect,
    send: &mut crate::model::SendSlot,
    uid: egui::Id,
    inactive: bool,
    buses: &[(u16, String)],
) -> bool {
    let Some(bus) = send.bus else {
        return false;
    };
    let mut keep = true;
    let knob_c = pos2(r.right() - 8.0, r.center().y);
    let slot_r = Rect::from_min_max(r.min, pos2(r.right() - 19.0, r.bottom()));
    let resp = ui.interact(slot_r, uid, Sense::click());
    slot(
        g,
        slot_r,
        &trf("Bus {}", &[&bus]),
        if inactive {
            plugin_off_col()
        } else {
            crate::theme::accent()
        },
        if inactive {
            text_dim_col()
        } else {
            crate::theme::on_accent()
        },
    );
    if send.pre {
        // The eye icon Logic shows on pre-fader sends.
        let c = pos2(slot_r.left() + 9.0, slot_r.center().y);
        g.circle_stroke(c, 4.5, Stroke::new(1.0, Color32::WHITE));
        g.circle_filled(c, 1.6, Color32::WHITE);
    }
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(130.0);
        egui::ScrollArea::vertical()
            .max_height(300.0)
            .show(ui, |ui| {
                for (b, name) in buses {
                    if ui
                        .selectable_label(send.bus == Some(*b), format!("Bus {b}: {name}"))
                        .clicked()
                    {
                        send.bus = Some(*b);
                        ui.close();
                    }
                }
            });
        ui.separator();
        if ui.selectable_label(send.pre, tr("Pre Fader")).clicked() {
            send.pre = !send.pre;
            ui.close();
        }
        if ui.button(tr("No Send")).clicked() {
            keep = false;
            ui.close();
        }
    });
    // Level knob: drag vertically; the indicator sweeps like a pan knob.
    let kresp = ui.interact(
        Rect::from_center_size(knob_c, vec2(16.0, 16.0)),
        uid.with("level"),
        Sense::click_and_drag(),
    );
    // The same gestures as every knob; reset is no send.
    if let Some(v) = crate::widgets::value_gesture(ui, &kresp, send.level, 0.0, 0.01, 0.02) {
        send.level = v.clamp(0.0, 1.0);
    }
    crate::widgets::value_arc(g, knob_c, 8.2, -1.0, send.level * 2.0 - 1.0);
    g.circle_filled(knob_c, 6.5, crate::theme::gray(0x32));
    g.circle_stroke(knob_c, 6.5, Stroke::new(1.0, crate::theme::gray(0x15)));
    let a = -std::f32::consts::FRAC_PI_2 - 2.3 + send.level * 4.6;
    g.line_segment(
        [
            knob_c + vec2(a.cos(), a.sin()) * 2.0,
            knob_c + vec2(a.cos(), a.sin()) * 5.5,
        ],
        Stroke::new(1.4, Color32::WHITE),
    );
    keep
}

/// Ovrtone's own effects as a maker's folder in a slot's menu: the current set, and the first set
/// under Classic. The kind picked, if any (`now` is ticked).
fn stock_folder(ui: &mut Ui, now: Option<ovrtone_engine::FxKind>) -> Option<ovrtone_engine::FxKind> {
    let mut picked = None;
    let mut entry = |ui: &mut Ui, kind: ovrtone_engine::FxKind| {
        let text = format!("{}  ·  {}", kind.name(), kind.kind_word());
        if ui.selectable_label(now == Some(kind), text).clicked() {
            picked = Some(kind);
            ui.close();
        }
    };
    ui.menu_button("Ovrtone", |ui| {
        for kind in ovrtone_engine::FxKind::MENU {
            entry(ui, kind);
        }
        ui.separator();
        ui.menu_button("Classic", |ui| {
            for kind in ovrtone_engine::FxKind::CLASSIC {
                entry(ui, kind);
            }
        });
    });
    picked
}

/// A stock effect carried from the Plugins list to a slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StockDrag(pub ovrtone_engine::FxKind);

/// Puts a stock effect after the effects of track `i`.
pub fn add_stock(p: &mut Project, i: usize, kind: ovrtone_engine::FxKind) -> Result<(), String> {
    let Some(t) = p.tracks.get_mut(i) else {
        return Err("Select a track first.".to_string());
    };
    if t.inserts.len() + t.plugins.len() >= ovrtone_engine::MAX_INSERTS {
        return Err("This track has no free effect slot.".to_string());
    }
    t.inserts.push(Insert::new(kind));
    Ok(())
}

/// A stock effect dropped on slot `k` of track `i` goes in there, and its window opens.
fn stock_drop(ui: &Ui, g: &Painter, r: Rect, p: &mut Project, v: &mut MixerView, i: usize, k: usize) {
    let Some(drag) = egui::DragAndDrop::payload::<StockDrag>(ui.ctx()) else {
        return;
    };
    let over = ui
        .input(|inp| inp.pointer.hover_pos())
        .is_some_and(|at| r.contains(at));
    if !over {
        return;
    }
    g.rect_stroke(r, 2.0, Stroke::new(1.5, blue()), egui::StrokeKind::Outside);
    if ui.input(|inp| inp.pointer.any_released()) {
        egui::DragAndDrop::clear_payload(ui.ctx());
        if add_stock(p, i, drag.0).is_ok() {
            let last = p.tracks[i].inserts.len() - 1;
            apply_insert(p, v, i, last, InsertAct::MoveTo(k));
            let at = k.min(last);
            v.fx_window = Some((i, at));
        }
    }
}

/// What the user chose in an Audio FX slot's menu.
enum InsertAct {
    Add(ovrtone_engine::FxKind),
    Replace(ovrtone_engine::FxKind),
    Remove,
    Bypass,
    Open,
    Move(isize),
    /// To slot n (past the last: to the end).
    MoveTo(usize),
    /// A copy right after it.
    Duplicate,
    /// The copied effect into this slot.
    Paste(Insert),
}

/// The right-click menu of a track's strip.
fn t_is_aux(p: &Project, i: usize) -> bool {
    p.tracks.get(i).is_some_and(|t| t.kind == TrackKind::Aux)
}

fn strip_menu(resp: &egui::Response, p: &mut Project, v: &mut MixerView, i: usize) {
    let Some(t) = p.tracks.get(i) else {
        return;
    };
    let (mute, solo, rec) = (t.mute, t.solo, t.rec);
    let takes_midi = matches!(t.kind, TrackKind::Instrument | TrackKind::ExternalMidi);
    let last = p.tracks.len() == 1;
    let can_copy = p.can_duplicate(i);
    let feeds = p.feeds_of(i);
    let is_aux = t_is_aux(p, i);
    crate::widgets::context_menu(resp, |ui| {
        // The level going into the strip's effects; the fader comes after them.
        ui.horizontal(|ui| {
            ui.label("Input Trim");
            if let Some(t) = p.tracks.get_mut(i) {
                ui.add(
                    egui::DragValue::new(&mut t.trim_db)
                        .speed(0.1)
                        .range(-36.0..=36.0)
                        .suffix(" dB"),
                );
            }
        });
        {
            if let Some(t) = p.tracks.get_mut(i) {
                let now = match (t.object.is_some(), t.bed_pan) {
                    (false, _) => 0,
                    (true, true) => 1,
                    (true, false) => 2,
                };
                ui.menu_button("Spatial", |ui| {
                    for (k, name) in [(0, "Stereo"), (1, "Surround Pan"), (2, "Object")] {
                        if ui.selectable_label(now == k, name).clicked() {
                            match k {
                                0 => t.object = None,
                                _ => {
                                    t.object.get_or_insert_with(ovrtone_engine::ObjectPlace::default);
                                    t.bed_pan = k == 1;
                                }
                            }
                            ui.close();
                        }
                    }
                    ui.separator();
                    if ui.button("Open Panner…").clicked() {
                        p.selected_track = i;
                        v.open_spatial = true;
                        ui.close();
                    }
                });
            }
        }
        if ui.button("Auto Gain Staging…").clicked() {
            v.open_gain_staging = true;
            ui.close();
        }
        // Templates: the track, its effect chain, or the whole mixer, for other projects.
        ui.menu_button("Templates", |ui| {
            use crate::file_commands::{templates, FileTask, TemplateKind};
            if ui.button("Save Track as Template…").clicked() {
                v.file_task = Some(FileTask::SaveTrackTemplate(i));
                ui.close();
            }
            if ui.button("Save Effect Chain…").clicked() {
                v.file_task = Some(FileTask::SaveChainPreset(i));
                ui.close();
            }
            if ui.button("Save Mixer as Template…").clicked() {
                v.file_task = Some(FileTask::SaveMixerTemplate);
                ui.close();
            }
            ui.separator();
            for (label, kind) in [
                ("New Track from Template", TemplateKind::Track),
                ("Load Effect Chain", TemplateKind::Chain),
                ("New Tracks from Mixer Template", TemplateKind::Mixer),
            ] {
                ui.menu_button(label, |ui| {
                    let list = templates(kind);
                    if list.is_empty() {
                        ui.weak("None saved yet");
                    }
                    for (name, file) in list {
                        if ui.button(name.as_str()).clicked() {
                            v.file_task = Some(match kind {
                                TemplateKind::Chain => FileTask::LoadChain(file, i),
                                _ => FileTask::LoadTracks(file),
                            });
                            ui.close();
                        }
                    }
                });
            }
        });
        // Which strips the mixer shows and how: kept for every project.
        ui.menu_button("Mixer Layouts", |ui| {
            if ui
                .button("Save Layout")
                .on_hover_text("Single, Tracks or All, the strip types shown, and wide or narrow strips")
                .clicked()
            {
                save_layout(v);
            }
            let layouts = load_layouts();
            let mut forget = None;
            for (k, (name, mode, filters, wide)) in layouts.iter().enumerate() {
                ui.horizontal(|ui| {
                    if ui.button(name.as_str()).clicked() {
                        v.mode = *mode;
                        v.filters = *filters;
                        v.wide = *wide;
                        ui.close();
                    }
                    if ui.small_button("Delete").clicked() {
                        forget = Some(k);
                    }
                });
            }
            if let Some(k) = forget {
                let mut layouts = layouts;
                layouts.remove(k);
                write_layouts(&layouts);
            }
        });
        // The whole mixer kept as it is, to go back to.
        ui.menu_button("Mixer Snapshots", |ui| {
            if ui
                .button("Save Snapshot")
                .on_hover_text("Faders, pans, mutes, solos, input trims, Ovrtone effects and sends of every strip")
                .clicked()
            {
                p.take_mixer_snapshot();
            }
            let (mut recall, mut delete) = (None, None);
            for (k, m) in p.mixer_snapshots.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut m.name).desired_width(120.0));
                    if ui.button("Recall").clicked() {
                        recall = Some(k);
                    }
                    if ui.small_button("Delete").clicked() {
                        delete = Some(k);
                    }
                });
            }
            if let Some(k) = recall {
                p.recall_mixer_snapshot(k);
                ui.close();
            }
            if let Some(k) = delete {
                p.mixer_snapshots.remove(k);
            }
        });
        ui.separator();
        if is_aux {
            ui.menu_button("Fed By", |ui| {
                if feeds.is_empty() {
                    ui.weak("Nothing feeds this bus");
                }
                for (track, how) in &feeds {
                    if ui.button(how.as_str()).clicked() {
                        p.selected_track = *track;
                        v.track_clicked = true;
                        ui.close();
                    }
                }
            });
        }
        if can_copy && ui.button("Duplicate Track").clicked() {
            v.strip_request = Some(StripRequest::Duplicate(i, true));
            ui.close();
        }
        if can_copy && ui.button("New Track with Same Settings").clicked() {
            v.strip_request = Some(StripRequest::Duplicate(i, false));
            ui.close();
        }
        if ui.button("Track Color…").clicked() {
            v.strip_request = Some(StripRequest::Color(i));
            ui.close();
        }
        if takes_midi && ui.button(tr("Open in Piano Roll")).clicked() {
            v.strip_request = Some(StripRequest::PianoRoll(i));
            ui.close();
        }
        let label = if mute {
            tr("Unmute Track")
        } else {
            tr("Mute Track")
        };
        if ui.button(label).clicked() {
            p.tracks[i].mute = !mute;
            ui.close();
        }
        let label = if solo {
            tr("Unsolo Track")
        } else {
            tr("Solo Track")
        };
        if ui.button(label).clicked() {
            p.tracks[i].solo = !solo;
            ui.close();
        }
        let label = if rec {
            "Disarm Recording"
        } else {
            "Arm for Recording"
        };
        if ui.button(label).clicked() {
            p.tracks[i].rec = !rec;
            ui.close();
        }
        ui.separator();
        if ui.button(tr("Learn MIDI for Level")).clicked() {
            v.learn = Some(crate::model::LearnTarget::Volume(i));
            ui.close();
        }
        if ui.button(tr("Learn MIDI for Pan")).clicked() {
            v.learn = Some(crate::model::LearnTarget::Pan(i));
            ui.close();
        }
        if ui.button(tr("MIDI Assignments")).clicked() {
            v.learn_window = true;
            ui.close();
        }
        ui.separator();
        if ui
            .add_enabled(!last, egui::Button::new(tr("Delete Track")))
            .clicked()
        {
            v.strip_request = Some(StripRequest::Delete(i));
            ui.close();
        }
    });
}

/// The right button's menu of a slot; `also_left` opens it with the left one too.
fn slot_menu(resp: &egui::Response, also_left: bool) -> egui::Popup<'_> {
    let open = resp.secondary_clicked() || (also_left && resp.clicked());
    egui::Popup::menu(resp).open_memory(open.then_some(egui::SetOpenCommand::Toggle))
}

/// Room a slot's buttons take at its right end.
const SLOT_BUTTON_W: f32 = 13.0;

/// A slot with room at its right end for `buttons` buttons, its name in what is left.
fn named_slot(g: &Painter, r: Rect, text: &str, buttons: usize, fill: Color32, color: Color32) {
    g.rect_filled(r, 3.0, fill);
    let font = FontId::proportional(10.5);
    let room = r.width() - 6.0 - SLOT_BUTTON_W * buttons as f32;
    let shown = elide(g, text, &font, room);
    let centre = pos2(
        r.left() + 2.0 + (r.width() - SLOT_BUTTON_W * buttons as f32) / 2.0,
        r.center().y,
    );
    g.text(centre, Align2::CENTER_CENTER, shown, font, color);
}

/// The rectangle of a slot's button, counted from the right.
fn slot_button_rect(r: Rect, from_right: usize) -> Rect {
    let right = r.right() - from_right as f32 * SLOT_BUTTON_W;
    Rect::from_min_max(
        pos2(right - SLOT_BUTTON_W, r.top()),
        pos2(right, r.bottom()),
    )
}

/// The button that takes what is in a slot away.
fn slot_remove_button(ui: &Ui, g: &Painter, r: Rect, uid: egui::Id, color: Color32) -> bool {
    let b = slot_button_rect(r, 0);
    let resp = ui
        .interact(b, uid.with("remove"), Sense::click())
        .on_hover_text("Remove from this slot");
    if resp.hovered() {
        g.rect_filled(b, 2.0, Color32::from_black_alpha(70));
    }
    g.text(
        b.center(),
        Align2::CENTER_CENTER,
        "×",
        FontId::proportional(12.0),
        color,
    );
    resp.clicked()
}

/// The button that bypasses what is in a slot: filled while bypassed.
fn slot_bypass_button(
    ui: &Ui,
    g: &Painter,
    r: Rect,
    uid: egui::Id,
    bypassed: bool,
    color: Color32,
) -> bool {
    let b = slot_button_rect(r, 1);
    let resp = ui
        .interact(b, uid.with("bypass"), Sense::click())
        .on_hover_text("Bypass: let the sound pass by this effect");
    if resp.hovered() {
        g.rect_filled(b, 2.0, Color32::from_black_alpha(70));
    }
    // The power sign: an open ring with a stroke through its gap. Dim and filled while bypassed.
    let c = b.center();
    let pen = Stroke::new(1.3, color);
    if bypassed {
        g.circle_filled(c, 4.0, color.gamma_multiply(0.45));
    }
    let ring: Vec<egui::Pos2> = (0..=16)
        .map(|k| {
            let a = (-55.0 + 290.0 * k as f32 / 16.0).to_radians();
            c + vec2(a.cos(), a.sin()) * 4.0
        })
        .collect();
    g.add(egui::Shape::line(ring, pen));
    g.line_segment([c + vec2(0.0, -5.0), c + vec2(0.0, -1.0)], pen);
    resp.clicked()
}

/// Delete, with the pointer on a slot, for what is in it.
fn delete_pressed(ui: &Ui, resp: &egui::Response) -> bool {
    resp.hovered()
        && !ui.ctx().egui_wants_keyboard_input()
        && ui.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)
        })
}

/// One Audio FX slot of track `i`: the effect in it (blue, grey when bypassed or silenced),
/// or the empty slot after the last. Click for the menu, double-click for the effect's window.
#[allow(clippy::too_many_arguments)]
fn insert_slot(
    ui: &Ui,
    g: &Painter,
    r: Rect,
    p: &mut Project,
    v: &mut MixerView,
    i: usize,
    k: usize,
    uid: egui::Id,
    inactive: bool,
) {
    let have = p.tracks[i].inserts.get(k).copied();
    let count = p.tracks[i].inserts.len();
    let plugins = p.tracks[i].plugins.len();
    let rows = crate::model::plugin_rows(count, &p.tracks[i].plugins);
    if let Some(j) = rows.iter().position(|row| *row == k) {
        plugin_slot(ui, g, r, p, v, Home::Track(i), j, uid, inactive);
        slot_drop(ui, g, r, p, Home::Track(i), k);
        return;
    }
    slot_drop(ui, g, r, p, Home::Track(i), k);
    fx_drop(ui, g, r, p, v, i, k);
    if i >= p.tracks.len() {
        return;
    }
    // The next free slot has a plus. Every empty slot takes an effect, gaps included.
    let next = rows.last().map_or(count, |row| row + 1);
    let full = count + plugins >= ovrtone_engine::MAX_INSERTS;
    match have {
        Some(fx) => {
            let off = inactive || fx.bypass;
            named_slot(
                g,
                r,
                fx.name(),
                2,
                if off {
                    plugin_off_col()
                } else {
                    crate::theme::accent()
                },
                if off {
                    text_dim_col()
                } else {
                    crate::theme::on_accent()
                },
            );
        }
        None => slot(
            g,
            r,
            if k == next && !full { "+" } else { "" },
            slot_empty_col(),
            text_col(),
        ),
    }
    if have.is_none() && full {
        return;
    }
    let sense = if have.is_some() {
        Sense::click_and_drag()
    } else {
        Sense::click()
    };
    let resp = ui.interact(r, uid, sense);
    if have.is_some() && resp.drag_started() {
        egui::DragAndDrop::set_payload(ui.ctx(), FxDrag(i, k));
    }
    if let (Some(fx), true) = (have, resp.dragged()) {
        // The effect's name follows the pointer while it is carried.
        if let Some(at) = ui.ctx().pointer_interact_pos() {
            let layer = egui::LayerId::new(egui::Order::Tooltip, uid.with("carried"));
            let tag = Rect::from_center_size(at + vec2(0.0, -14.0), r.size());
            let lg = ui.ctx().layer_painter(layer);
            named_slot(&lg, tag, fx.name(), 2, crate::theme::accent(), crate::theme::on_accent());
        }
    }
    let resp = if have.is_none() {
        resp.on_hover_text_at_pointer("Add an effect")
    } else {
        resp
    };
    // A click opens the effect. An empty slot's click offers what can go in it.
    if have.is_some() && resp.clicked() {
        // A click opens the window, brings it forward, or (in front already) closes it.
        if v.fx_window == Some((i, k)) && v.fx_open.contains(&(i, k)) {
            v.fx_open.retain(|w| *w != (i, k));
            v.fx_window = v.fx_open.last().copied();
            v.fx_front = v.fx_window;
        } else {
            v.fx_window = Some((i, k));
        }
    }
    let mut act = (have.is_some() && delete_pressed(ui, &resp)).then_some(InsertAct::Remove);
    if let Some(fx) = have {
        let color = if inactive || fx.bypass {
            text_dim_col()
        } else {
            Color32::WHITE
        };
        if slot_bypass_button(ui, g, r, uid, fx.bypass, color) {
            act = Some(InsertAct::Bypass);
        }
        if slot_remove_button(ui, g, r, uid, color) {
            act = Some(InsertAct::Remove);
        }
    }
    let installed = v.installed.clone();
    let mut pick = None;
    let (clipboard, plugin_clip) = (v.fx_clipboard, v.plugin_clipboard.clone());
    let mut copied: Option<Insert> = None;
    let mut paste_plugin = false;
    let mut items = |ui: &mut Ui| {
        ui.set_min_width(150.0);
        if let Some(fx) = have {
            if ui.button(tr("Open")).clicked() {
                act = Some(InsertAct::Open);
                ui.close();
            }
            if ui.selectable_label(fx.bypass, tr("Bypass")).clicked() {
                act = Some(InsertAct::Bypass);
                ui.close();
            }
            ui.add_enabled_ui(k > 0, |ui| {
                if ui.button(tr("Move Up")).clicked() {
                    act = Some(InsertAct::Move(-1));
                    ui.close();
                }
            });
            ui.add_enabled_ui(k + 1 < count, |ui| {
                if ui.button(tr("Move Down")).clicked() {
                    act = Some(InsertAct::Move(1));
                    ui.close();
                }
            });
            ui.separator();
            if ui.button("Copy Effect").clicked() {
                copied = Some(fx);
                ui.close();
            }
            if ui.button("Duplicate Effect").clicked() {
                act = Some(InsertAct::Duplicate);
                ui.close();
            }
            ui.menu_button(tr("Replace With"), |ui| {
                if let Some(kind) = stock_folder(ui, Some(fx.kind)) {
                    act = Some(InsertAct::Replace(kind));
                }
            });
            if ui.button(tr("Remove")).clicked() {
                act = Some(InsertAct::Remove);
                ui.close();
            }
        } else {
            // What was copied from another slot, then Ovrtone's own effects in a folder like a
            // plugin maker's.
            if let Some(fx) = clipboard {
                if ui.button(format!("Paste {}", fx.name())).clicked() {
                    act = Some(InsertAct::Paste(fx));
                    ui.close();
                }
            }
            if let Some(plugin) = plugin_clip.as_ref() {
                if ui.button(format!("Paste {}", plugin.name)).clicked() {
                    paste_plugin = true;
                    ui.close();
                }
            }
            if clipboard.is_some() || plugin_clip.is_some() {
                ui.separator();
            }
            if let Some(kind) = stock_folder(ui, None) {
                act = Some(InsertAct::Add(kind));
            }
            if installed.iter().any(|info| !info.is_instrument) {
                ui.separator();
                plugin_menu(ui, &installed, false, &mut pick);
            }
        }
    };
    slot_menu(&resp, have.is_none()).show(&mut items);
    if let Some(fx) = copied {
        v.fx_clipboard = Some(fx);
    }
    if paste_plugin {
        if let Some(mut plugin) = plugin_clip {
            // A new instance of it, with the settings it had when copied.
            plugin.uid = crate::plugin_rack::new_uid();
            plugin.sidechain = None;
            plugin.failed = None;
            let t = &mut p.tracks[i];
            if t.inserts.len() + t.plugins.len() < ovrtone_engine::MAX_INSERTS {
                let first = t.inserts.len();
                crate::model::place_plugin_in_row(first, &mut t.plugins, plugin, k);
                p.song_rev += 1;
            }
        }
    }
    if let Some(info) = pick {
        if let Err(e) = crate::plugin_rack::add_effect_in_row(p, i, &info, k) {
            v.message = Some(e);
        }
    }
    if let Some(act) = act {
        apply_insert(p, v, i, k, act);
    }
}

/// The Presets entries of a plugin slot's menu.
fn preset_items(
    ui: &mut Ui,
    uid: u64,
    id: &str,
    name: &str,
    cmd: &mut Option<crate::plugin_presets::PresetCmd>,
) {
    use crate::plugin_presets::{ask_where, list, PresetCmd};
    ui.menu_button("Presets", |ui| {
        if ui.button("Save Preset…").clicked() {
            if let Some(file) = ask_where(id, name) {
                *cmd = Some(PresetCmd::Save { uid, file });
            }
            ui.close();
        }
        let presets = list(id);
        if !presets.is_empty() {
            ui.separator();
        }
        for (name, file) in presets {
            if ui.button(name).clicked() {
                *cmd = Some(PresetCmd::Load { uid, file });
                ui.close();
            }
        }
    });
}

/// A/B compare of plugin `uid`, in a slot's menu.
fn compare_items(
    ui: &mut Ui,
    uid: u64,
    on_b: bool,
    cmd: &mut Option<crate::plugin_presets::PresetCmd>,
) {
    use crate::plugin_presets::PresetCmd;
    if ui.selectable_label(on_b, tr("Compare A/B")).clicked() {
        *cmd = Some(PresetCmd::Compare { uid });
        ui.close();
    }
    let copy = if on_b {
        tr("Copy B to A")
    } else {
        tr("Copy A to B")
    };
    if ui.button(copy).clicked() {
        *cmd = Some(PresetCmd::CopyCompare { uid });
        ui.close();
    }
}

/// The Side Chain menu of a plugin that offers a side-chain input: every other track that makes
/// sound. What was picked goes in `pick`, `Some(None)` for none.
fn side_chain_items(
    ui: &mut Ui,
    sources: &[(usize, String)],
    now: Option<usize>,
    pick: &mut Option<Option<usize>>,
) {
    ui.menu_button(tr("Side Chain"), |ui| {
        if ui.selectable_label(now.is_none(), tr("Off")).clicked() {
            *pick = Some(None);
            ui.close();
        }
        for (t, name) in sources {
            if ui
                .selectable_label(now == Some(*t), name.as_str())
                .clicked()
            {
                *pick = Some(Some(*t));
                ui.close();
            }
        }
    });
}

/// The tracks that can key a plugin on track `own` (`None`: the output).
fn side_chain_sources(p: &Project, own: Option<usize>) -> Vec<(usize, String)> {
    p.tracks
        .iter()
        .enumerate()
        .filter(|(i, t)| Some(*i) != own && crate::model::can_key(Some(t.kind)))
        .map(|(i, t)| (i, t.name.clone()))
        .collect()
}

/// A stock effect being dragged to another slot of its track: (track, slot).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FxDrag(usize, usize);

/// Drops a dragged stock effect on slot `k` of track `i`: it moves there (past the last effect:
/// to the end), and the others close up around it.
#[allow(clippy::too_many_arguments)]
fn fx_drop(ui: &Ui, g: &Painter, r: Rect, p: &mut Project, v: &mut MixerView, i: usize, k: usize) {
    stock_drop(ui, g, r, p, v, i, k);
    let Some(drag) = egui::DragAndDrop::payload::<FxDrag>(ui.ctx()) else {
        return;
    };
    let FxDrag(from_t, from) = *drag;
    let over = ui
        .input(|inp| inp.pointer.hover_pos())
        .is_some_and(|at| r.contains(at));
    if !over || from_t != i {
        return;
    }
    g.rect_stroke(r, 2.0, Stroke::new(1.5, blue()), egui::StrokeKind::Outside);
    if ui.input(|inp| inp.pointer.any_released()) {
        egui::DragAndDrop::clear_payload(ui.ctx());
        apply_insert(p, v, i, from, InsertAct::MoveTo(k));
    }
}

/// A plugin being dragged to another slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SlotDrag {
    home: Home,
    index: usize,
}

/// Moves a plugin to insert row `row` of `to`, which may be another strip. The instance goes with it,
/// found by its uid, and the sound follows the new order.
fn move_plugin(p: &mut Project, from: SlotDrag, to: Home, row: usize) {
    let first_from = from.home.first_row(p);
    crate::model::settle_plugin_rows(first_from, from.home.plugins(p));
    let list = from.home.plugins(p);
    if from.index >= list.len() {
        return;
    }
    let plugin = list.remove(from.index);
    let first = to.first_row(p);
    let row = row.max(first);
    let target = to.plugins(p);
    crate::model::settle_plugin_rows(first, target);
    let free = !target.iter().any(|x| usize::from(x.row) == row);
    if free {
        crate::model::place_plugin_in_row(first, target, plugin, row);
    } else {
        // On a plugin: it goes before that one, which moves down as far as it must.
        let at = target.partition_point(|x| usize::from(x.row) < row);
        target.insert(at, plugin);
        target[at].row = row.min(usize::from(u8::MAX)) as u8;
        for later in at + 1..target.len() {
            let before = target[later - 1].row;
            if target[later].row <= before {
                target[later].row = before.saturating_add(1);
            }
        }
    }
    p.selected_track = match to {
        Home::Track(i) => i,
        Home::Master => p.selected_track,
    };
}

/// A plugin from the Plugins tab let go on an insert: it takes the place of what is there, in the same
/// row, or fills it if empty. An instrument cannot go in an insert.
fn replace_in_slot(
    p: &mut Project,
    info: &ovrtone_host::PluginInfo,
    home: Home,
    row: usize,
) -> bool {
    if info.is_instrument {
        return false;
    }
    let first = home.first_row(p);
    let row = row.max(first);
    let list = home.plugins(p);
    crate::model::settle_plugin_rows(first, list);
    let mut new = crate::model::PluginInsert::new(info);
    if let Some(there) = list.iter_mut().find(|x| usize::from(x.row) == row) {
        new.row = there.row;
        *there = new;
    } else {
        crate::model::place_plugin_in_row(first, list, new, row);
    }
    true
}

/// A plugin let go on slot `r`, row `k` of `home`.
fn slot_drop(ui: &Ui, g: &Painter, r: Rect, p: &mut Project, home: Home, k: usize) {
    if let Some(drag) = egui::DragAndDrop::payload::<crate::left_panel::PluginDrag>(ui.ctx()) {
        let over = ui
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|at| r.contains(at));
        if over && !drag.0.is_instrument {
            g.rect_stroke(r, 2.0, Stroke::new(1.5, blue()), egui::StrokeKind::Outside);
            if ui.input(|i| i.pointer.any_released()) && replace_in_slot(p, &drag.0, home, k) {
                egui::DragAndDrop::clear_payload(ui.ctx());
            }
        }
    }
    let Some(drag) = egui::DragAndDrop::payload::<SlotDrag>(ui.ctx()) else {
        return;
    };
    let over = ui
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|at| r.contains(at));
    if !over {
        return;
    }
    g.rect_stroke(r, 2.0, Stroke::new(1.5, blue()), egui::StrokeKind::Outside);
    if ui.input(|i| i.pointer.any_released()) {
        egui::DragAndDrop::clear_payload(ui.ctx());
        move_plugin(p, *drag, home, k);
    }
}

/// A channel strip being dragged by its name to another place among the strips.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StripDrag(usize);

/// The track a strip dropped at `x` goes before: the one of the strip whose left half or the gap
/// before it `x` is over. Past the last track's strip it is `tracks`, the end. The strips of the
/// output stay last. `first` is the left edge of the first strip, `step` from one to the next.
fn strip_drop_target(keys: &[StripKey], tracks: usize, first: f32, step: f32, x: f32) -> usize {
    let strips = keys
        .iter()
        .filter(|k| matches!(k, StripKey::Track(_)))
        .count();
    let gap = ((x - first + step * 0.5) / step).floor().max(0.0) as usize;
    match keys.get(gap.min(strips)) {
        Some(StripKey::Track(t)) if gap < strips => *t,
        _ => tracks,
    }
}

/// Whose effects a plugin slot shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Home {
    Track(usize),
    /// The output.
    Master,
}

impl Home {
    fn first_row(self, p: &Project) -> usize {
        match self {
            Home::Track(i) => p.tracks[i].inserts.len(),
            Home::Master => 0,
        }
    }

    fn plugins(self, p: &mut Project) -> &mut Vec<crate::model::PluginInsert> {
        match self {
            Home::Track(i) => &mut p.tracks[i].plugins,
            Home::Master => &mut p.master_plugins,
        }
    }
}

/// One Audio FX slot of the output, which Stereo Out and Master share.
#[allow(clippy::too_many_arguments)]
fn master_slot(
    ui: &Ui,
    g: &Painter,
    r: Rect,
    p: &mut Project,
    v: &mut MixerView,
    k: usize,
    uid: egui::Id,
) {
    // Ovrtone's own effects first, then the plugins, as on a track.
    let own = p.master_strip.inserts.len();
    if let Some(fx) = p.master_strip.inserts.get(k).copied() {
        let (fill, ink) = if fx.bypass {
            (plugin_off_col(), text_dim_col())
        } else {
            (crate::theme::accent(), crate::theme::on_accent())
        };
        named_slot(g, r, fx.name(), 0, fill, ink);
        let resp = ui
            .interact(r, uid, Sense::click())
            .on_hover_text_at_pointer("Click to open; right-click to bypass or remove");
        if resp.clicked() {
            v.fx_window = Some((crate::model::MASTER_STRIP, k));
        }
        resp.context_menu(|ui| {
            if ui.button(if fx.bypass { "Turn On" } else { "Bypass" }).clicked() {
                p.master_strip.inserts[k].bypass = !fx.bypass;
                ui.close();
            }
            if ui.button("Remove").clicked() {
                p.master_strip.inserts.remove(k);
                ui.close();
            }
        });
        return;
    }
    let rows = crate::model::plugin_rows(own, &p.master_plugins);
    if let Some(j) = rows.iter().position(|row| *row == k) {
        plugin_slot(ui, g, r, p, v, Home::Master, j, uid, false);
        slot_drop(ui, g, r, p, Home::Master, k);
        return;
    }
    slot_drop(ui, g, r, p, Home::Master, k);
    let next = rows.last().map_or(own, |row| row + 1);
    let full = p.master_plugins.len() + own >= ovrtone_engine::MAX_INSERTS;
    slot(
        g,
        r,
        if k == next && !full { "+" } else { "" },
        slot_empty_col(),
        text_col(),
    );
    if full {
        return;
    }
    let resp = ui
        .interact(r, uid, Sense::click())
        .on_hover_text_at_pointer("Add an effect plugin on the whole mix");
    let installed = v.installed.clone();
    let mut pick = None;
    let mut stock = None;
    slot_menu(&resp, true).show(|ui: &mut Ui| {
        ui.set_min_width(170.0);
        // Ovrtone's own effects, then the plugins.
        if let Some(kind) = stock_folder(ui, None) {
            stock = Some(kind);
        }
        if installed.iter().any(|info| !info.is_instrument) {
            plugin_menu(ui, &installed, false, &mut pick);
        } else {
            ui.label("No effect plugins found");
        }
    });
    if let Some(kind) = stock {
        // Before the plugins, as on a track; the plugins keep their order after them.
        p.master_strip.inserts.push(Insert::new(kind));
        v.fx_window = Some((crate::model::MASTER_STRIP, p.master_strip.inserts.len() - 1));
    }
    if let Some(info) = pick {
        if let Err(e) = crate::plugin_rack::add_master_effect(p, &info, k) {
            v.message = Some(e);
        }
    }
}

/// Keeps the slots' menus up to date with the plugins found. Called every frame.
pub fn refresh_installed(ctx: &egui::Context, s: &mut UiState) {
    let _ = s.plugins.installed(ctx);
    let list = &s.plugins.clap;
    let shown = list.plugins.iter().filter(|i| !list.hidden.contains(&i.id));
    if shown.clone().count() != s.mixer.installed.len() {
        s.mixer.installed = std::sync::Arc::new(shown.cloned().collect());
    }
    if let Some(message) = s.mixer.message.take() {
        s.status = Some(message);
    }
}

/// The instruments or the effects on this computer, a submenu for each maker. What was clicked
/// goes in `pick`.
fn plugin_menu(
    ui: &mut Ui,
    installed: &[PluginInfo],
    instruments: bool,
    pick: &mut Option<PluginInfo>,
) {
    let search_id = egui::Id::unique(("plugin_menu_search", instruments));
    let recent_id = egui::Id::unique(("plugin_menu_recent", instruments));
    let mut search = ui
        .data_mut(|d| d.get_temp::<String>(search_id))
        .unwrap_or_default();
    ui.add(
        egui::TextEdit::singleline(&mut search)
            .hint_text("Search")
            .desired_width(170.0),
    );
    ui.data_mut(|d| d.insert_temp(search_id, search.clone()));
    let wanted = search.trim().to_lowercase();
    let before = pick.is_some();
    if !wanted.is_empty() {
        egui::ScrollArea::vertical()
            .max_height(320.0)
            .show(ui, |ui| {
                let found = installed.iter().filter(|i| {
                    i.is_instrument == instruments
                        && (i.name.to_lowercase().contains(&wanted)
                            || i.vendor.to_lowercase().contains(&wanted))
                });
                for info in found {
                    if ui.button(info.name.as_str()).clicked() {
                        *pick = Some(info.clone());
                        ui.close();
                    }
                }
            });
        remember_plugin(ui, recent_id, search_id, before, pick);
        return;
    }
    let recent: Vec<String> = ui.data_mut(|d| d.get_temp(recent_id)).unwrap_or_default();
    let recent: Vec<&PluginInfo> = recent
        .iter()
        .filter_map(|id| installed.iter().find(|i| i.id == *id))
        .collect();
    if !recent.is_empty() && crate::settings::show_recent_plugins() {
        ui.weak("Recent");
        for info in recent {
            if ui.button(info.name.as_str()).clicked() {
                *pick = Some(info.clone());
                ui.close();
            }
        }
        ui.separator();
    }
    let favourites: Vec<&PluginInfo> = installed
        .iter()
        .filter(|i| i.is_instrument == instruments && crate::plugin_favourites::is_favourite(&i.id))
        .collect();
    if !favourites.is_empty() {
        ui.menu_button("★ Favourites", |ui| {
            for info in &favourites {
                if ui.button(info.name.as_str()).clicked() {
                    *pick = Some((*info).clone());
                    ui.close();
                }
            }
        });
    }
    if !instruments {
        ui.menu_button("By Type", |ui| {
            for kind in crate::plugin_favourites::KINDS.iter().skip(1) {
                let of_kind: Vec<&PluginInfo> = installed
                    .iter()
                    .filter(|i| !i.is_instrument && crate::plugin_favourites::kind_of(i) == *kind)
                    .collect();
                if of_kind.is_empty() {
                    continue;
                }
                ui.menu_button(*kind, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(ui.ctx().content_rect().height() * 0.7)
                        .show(ui, |ui| {
                            for info in &of_kind {
                                if ui.button(info.name.as_str()).clicked() {
                                    *pick = Some((*info).clone());
                                    ui.close();
                                }
                            }
                        });
                });
            }
        });
        ui.separator();
    }
    let mut makers: BTreeMap<String, Vec<&PluginInfo>> = BTreeMap::new();
    for info in installed.iter().filter(|i| i.is_instrument == instruments) {
        let maker = if info.vendor.trim().is_empty() {
            "Other".to_string()
        } else {
            info.vendor.clone()
        };
        makers.entry(maker).or_default().push(info);
    }
    egui::ScrollArea::vertical()
        .max_height(ui.ctx().content_rect().height() * 0.7)
        .show(ui, |ui| {
            for (maker, plugins) in &makers {
                ui.menu_button(maker.as_str(), |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(ui.ctx().content_rect().height() * 0.7)
                        .show(ui, |ui| {
                            for info in plugins {
                                if ui.button(info.name.as_str()).clicked() {
                                    *pick = Some((*info).clone());
                                    ui.close();
                                }
                            }
                        });
                });
            }
        });
    remember_plugin(ui, recent_id, search_id, before, pick);
}

/// When a plugin was just picked: it goes first among the recent ones, and the search empties.
fn remember_plugin(
    ui: &Ui,
    recent_id: egui::Id,
    search_id: egui::Id,
    before: bool,
    pick: &Option<PluginInfo>,
) {
    let Some(info) = pick.as_ref().filter(|_| !before) else {
        return;
    };
    ui.data_mut(|d| {
        let mut recent: Vec<String> = d.get_temp(recent_id).unwrap_or_default();
        recent.retain(|id| *id != info.id);
        recent.insert(0, info.id.clone());
        recent.truncate(5);
        d.insert_temp(recent_id, recent);
        d.insert_temp(search_id, String::new());
    });
}

/// Puts what was picked in a slot's menu on track `i`.
fn put_plugin(p: &mut Project, v: &mut MixerView, i: usize, pick: Option<PluginInfo>) {
    if let Some(info) = pick {
        if let Err(e) = crate::plugin_rack::add_plugin(p, i, &info) {
            v.message = Some(e);
        }
    }
}

/// The right-click menu (and left-click, if `also_left`) that sets the instrument of track `i`.
fn instrument_menu(
    resp: &egui::Response,
    also_left: bool,
    p: &mut Project,
    v: &mut MixerView,
    i: usize,
) {
    let installed = v.installed.clone();
    let mut pick = None;
    slot_menu(resp, also_left).show(|ui: &mut Ui| {
        ui.set_min_width(170.0);
        if installed.iter().any(|info| info.is_instrument) {
            plugin_menu(ui, &installed, true, &mut pick);
        } else {
            ui.label("No instrument plugins found");
        }
    });
    put_plugin(p, v, i, pick);
}

/// One slot holding hosted plugin `j` of track `i`: a click opens its editor, the buttons at its
/// end bypass and remove it, the right button has the rest.
#[allow(clippy::too_many_arguments)]
fn plugin_slot(
    ui: &Ui,
    g: &Painter,
    r: Rect,
    p: &mut Project,
    v: &mut MixerView,
    home: Home,
    j: usize,
    uid: egui::Id,
    inactive: bool,
) {
    let plugin = home.plugins(p)[j].clone();
    let plugin = &plugin;
    let (plugin_uid, name, loaded) = (plugin.uid, plugin.name.clone(), plugin.failed.is_none());
    let off = inactive || plugin.bypass || plugin.failed.is_some();
    let bypass = plugin.bypass;
    let text_color = if off { text_dim_col() } else { Color32::WHITE };
    named_slot(
        g,
        r,
        &plugin.name,
        2,
        if off {
            plugin_off_col()
        } else {
            crate::theme::accent()
        },
        text_color,
    );
    let hover = match &plugin.failed {
        Some(why) => format!("{}: {why}", plugin.name),
        None => format!("{} ({})", plugin.name, plugin.vendor),
    };
    let hover = format!("{hover}\nClick to open. Drag to move it. Right-click for more.");
    let count = home.plugins(p).len();
    let plugin_id = plugin.id.clone();
    let resp = ui
        .interact(r, uid, Sense::click_and_drag())
        .on_hover_text_at_pointer(hover);
    resp.dnd_set_drag_payload(SlotDrag { home, index: j });
    let mut moved = None;
    let mut copy_plugin = false;
    let mut remove = delete_pressed(ui, &resp);
    let mut flip_bypass = slot_bypass_button(ui, g, r, uid, bypass, text_color);
    remove |= slot_remove_button(ui, g, r, uid, text_color);
    let mut open = loaded && resp.clicked();
    let mut preset = None;
    let own = match home {
        Home::Track(i) => Some(i),
        Home::Master => None,
    };
    let sources = if plugin.side_offered {
        side_chain_sources(p, own)
    } else {
        Vec::new()
    };
    let side_now = plugin.sidechain;
    let side_offered = plugin.side_offered;
    let mut side_pick = None;
    let on_b = v.showing_b.contains(&plugin_uid);
    let mut items = |ui: &mut Ui| {
        ui.set_min_width(150.0);
        ui.add_enabled_ui(loaded, |ui| {
            if ui.button(tr("Open")).clicked() {
                open = true;
                ui.close();
            }
        });
        if ui.selectable_label(bypass, tr("Bypass")).clicked() {
            flip_bypass = true;
            ui.close();
        }
        ui.add_enabled_ui(j > 0, |ui| {
            if ui.button(tr("Move Up")).clicked() {
                moved = Some(j - 1);
                ui.close();
            }
        });
        ui.add_enabled_ui(j + 1 < count, |ui| {
            if ui.button(tr("Move Down")).clicked() {
                moved = Some(j + 1);
                ui.close();
            }
        });
        if ui
            .button("Copy Plugin")
            .on_hover_text("With its settings as they were last saved")
            .clicked()
        {
            copy_plugin = true;
            ui.close();
        }
        preset_items(ui, plugin_uid, &plugin_id, &name, &mut preset);
        if loaded {
            compare_items(ui, plugin_uid, on_b, &mut preset);
        }
        if side_offered {
            side_chain_items(ui, &sources, side_now, &mut side_pick);
        }
        ui.separator();
        if ui.button(tr("Remove")).clicked() {
            remove = true;
            ui.close();
        }
    };
    slot_menu(&resp, false).show(&mut items);
    if copy_plugin {
        v.plugin_clipboard = Some(plugin.clone());
    }
    if preset.is_some() {
        v.preset_cmd = preset;
    }
    if let Some(source) = side_pick {
        home.plugins(p)[j].sidechain = source;
    }
    if open {
        v.open_plugin = Some((plugin_uid, name));
    }
    let first = home.first_row(p);
    let plugins = home.plugins(p);
    if flip_bypass {
        plugins[j].bypass = !bypass;
    }
    if remove || moved.is_some() {
        // The others stay in the rows they are in.
        crate::model::settle_plugin_rows(first, plugins);
    }
    if remove {
        plugins.remove(j);
    } else if let Some(to) = moved {
        plugins.swap(j, to);
        let (a, b) = (plugins[j].row, plugins[to].row);
        plugins[j].row = b;
        plugins[to].row = a;
    }
}

/// The input slot of track `i` when a hosted plugin plays its notes. A click opens its editor.
fn instrument_plugin_slot(
    ui: &Ui,
    g: &Painter,
    r: Rect,
    p: &mut Project,
    v: &mut MixerView,
    i: usize,
    uid: egui::Id,
) {
    let Some(plugin) = p.tracks[i].instrument_plugin.clone() else {
        return;
    };
    // A new sound or instrument let go on it takes its place.
    let hovering = ui
        .input(|m| m.pointer.hover_pos())
        .is_some_and(|at| r.contains(at));
    let released = ui.input(|m| m.pointer.any_released());
    if hovering {
        if let Some(drag) =
            egui::DragAndDrop::payload::<crate::sample_browser::SampleDrag>(ui.ctx())
        {
            g.rect_stroke(r, 2.0, Stroke::new(1.5, blue()), egui::StrokeKind::Outside);
            if released {
                egui::DragAndDrop::clear_payload(ui.ctx());
                let info = crate::sample_place::sampler_info(&drag.0);
                let _ = if crate::sample_place::holds_sample(p, i) {
                    crate::sample_place::replace_sample(p, i, &drag.0)
                } else {
                    crate::plugin_rack::add_plugin(p, i, &info)
                };
            }
        }
        if let Some(drag) = egui::DragAndDrop::payload::<crate::left_panel::PluginDrag>(ui.ctx()) {
            if drag.0.is_instrument {
                g.rect_stroke(r, 2.0, Stroke::new(1.5, blue()), egui::StrokeKind::Outside);
                if released {
                    egui::DragAndDrop::clear_payload(ui.ctx());
                    let _ = crate::plugin_rack::add_plugin(p, i, &drag.0);
                }
            }
        }
    }
    let loaded = plugin.failed.is_none();
    let on = loaded && !plugin.bypass;
    let text_color = if on { crate::theme::on_accent() } else { text_dim_col() };
    named_slot(
        g,
        r,
        &plugin.name,
        2,
        if on {
            crate::theme::accent()
        } else {
            plugin_off_col()
        },
        text_color,
    );
    let hover = match &plugin.failed {
        Some(why) => format!("{}: {why}", plugin.name),
        None => format!("{} ({})", plugin.name, plugin.vendor),
    };
    let hover = format!("{hover}\nClick to open. Right-click to replace it.");
    let resp = ui
        .interact(r, uid, Sense::click())
        .on_hover_text_at_pointer(hover);
    let mut open = loaded && resp.clicked();
    let mut remove = delete_pressed(ui, &resp);
    remove |= slot_remove_button(ui, g, r, uid, text_color);
    if slot_bypass_button(ui, g, r, uid, plugin.bypass, text_color) {
        if let Some(live) = p.tracks[i].instrument_plugin.as_mut() {
            live.bypass = !live.bypass;
        }
    }
    let installed = v.installed.clone();
    let mut pick = None;
    let sources = if plugin.side_offered {
        side_chain_sources(p, Some(i))
    } else {
        Vec::new()
    };
    let mut side_pick = None;
    let on_b = v.showing_b.contains(&plugin.uid);
    let mut compare = None;
    let mut items = |ui: &mut Ui| {
        ui.set_min_width(150.0);
        ui.add_enabled_ui(loaded, |ui| {
            if ui.button(tr("Open")).clicked() {
                open = true;
                ui.close();
            }
        });
        if loaded {
            compare_items(ui, plugin.uid, on_b, &mut compare);
        }
        if plugin.side_offered {
            side_chain_items(ui, &sources, plugin.sidechain, &mut side_pick);
        }
        ui.separator();
        if installed.iter().any(|info| info.is_instrument) {
            ui.menu_button("Replace With", |ui| {
                plugin_menu(ui, &installed, true, &mut pick);
            });
        }
        if ui.button(tr("Remove")).clicked() {
            remove = true;
            ui.close();
        }
    };
    slot_menu(&resp, false).show(&mut items);
    if compare.is_some() {
        v.preset_cmd = compare;
    }
    if let (Some(source), Some(live)) = (side_pick, p.tracks[i].instrument_plugin.as_mut()) {
        live.sidechain = source;
    }
    if open {
        v.open_plugin = Some((plugin.uid, plugin.name));
    }
    if remove {
        p.tracks[i].instrument_plugin = None;
    }
    put_plugin(p, v, i, pick);
}

/// Carries out what the user chose in the menu of Audio FX slot `k` of track `i`. Choices that
/// do not fit (a slot that is not there) do nothing.
fn apply_insert(p: &mut Project, v: &mut MixerView, i: usize, k: usize, act: InsertAct) {
    if let InsertAct::MoveTo(to) = act {
        let count = p.tracks.get(i).map_or(0, |t| t.inserts.len());
        if k >= count {
            return;
        }
        // One step at a time, so automation lanes and open windows follow as for Move.
        let to = to.min(count - 1);
        let mut at = k;
        while at != to {
            let d: isize = if to > at { 1 } else { -1 };
            apply_insert(p, v, i, at, InsertAct::Move(d));
            at = (at as isize + d) as usize;
        }
        return;
    }
    let Some(t) = p.tracks.get_mut(i) else {
        return;
    };
    let inserts = &mut t.inserts;
    let filled = k < inserts.len();
    match act {
        InsertAct::Add(kind) => {
            if inserts.len() < ovrtone_engine::MAX_INSERTS {
                inserts.push(Insert::new(kind));
                v.fx_window = Some((i, inserts.len() - 1));
            }
        }
        InsertAct::Replace(kind) if filled => {
            inserts[k] = Insert::new(kind);
            t.auto.insert_replaced(k);
        }
        InsertAct::Remove if filled => {
            inserts.remove(k);
            t.auto.insert_removed(k);
            let after = |w: (usize, usize)| match w {
                (t, s) if t == i && s == k => None,
                (t, s) if t == i && s > k => Some((t, s - 1)),
                other => Some(other),
            };
            v.fx_window = v.fx_window.and_then(after);
            v.fx_open = v.fx_open.iter().filter_map(|w| after(*w)).collect();
        }
        InsertAct::Bypass if filled => inserts[k].bypass = !inserts[k].bypass,
        InsertAct::Duplicate if filled => {
            if inserts.len() < ovrtone_engine::MAX_INSERTS {
                let copy = inserts[k];
                inserts.insert(k + 1, copy);
                t.auto.insert_added(k + 1);
            }
        }
        InsertAct::Paste(fx) => {
            if inserts.len() < ovrtone_engine::MAX_INSERTS {
                inserts.push(Insert {
                    sidechain: None,
                    ..fx
                });
                v.fx_window = Some((i, inserts.len() - 1));
            }
        }
        InsertAct::Open if filled => v.fx_window = Some((i, k)),
        InsertAct::Move(d) if filled => {
            let to = k as isize + d;
            if to < 0 || to as usize >= inserts.len() {
                return;
            }
            let to = to as usize;
            inserts.swap(k, to);
            t.auto.insert_swapped(k, to);
            let swap = |w: &mut (usize, usize)| {
                if *w == (i, k) {
                    *w = (i, to);
                } else if *w == (i, to) {
                    *w = (i, k);
                }
            };
            if let Some(w) = v.fx_window.as_mut() {
                swap(w);
            }
            v.fx_open.iter_mut().for_each(swap);
        }
        _ => {}
    }
}

/// The colour of an automation mode's label in the strip.
fn mode_colour(mode: AutoMode, held: bool) -> Color32 {
    match mode {
        AutoMode::Off => text_dim_col(),
        AutoMode::Read => READ_GREEN,
        AutoMode::Touch | AutoMode::Latch | AutoMode::Trim if held => {
            Color32::from_rgb(0xff, 0xa8, 0x3c)
        }
        AutoMode::Touch | AutoMode::Latch | AutoMode::Trim => Color32::from_rgb(0xe6, 0x9a, 0x2e),
        AutoMode::Write => Color32::from_rgb(0xff, 0x5a, 0x4a),
    }
}

/// The strip's Automation slot: the mode, in its colour. Click to pick the mode or open the
/// Automation editor.
fn automation_slot(
    ui: &Ui,
    g: &Painter,
    r: Rect,
    p: &mut Project,
    v: &mut MixerView,
    i: usize,
    uid: egui::Id,
) {
    let (mode, held) = (p.tracks[i].auto.mode, p.tracks[i].auto.held.is_some());
    slot(g, r, tr(mode.label()), slot_empty_col(), mode_colour(mode, held));
    let has_lane = !p.tracks[i].auto.volume.is_empty() || !p.tracks[i].auto.pan.is_empty();
    if has_lane {
        // A small dot: this strip has automation.
        g.circle_filled(
            pos2(r.left() + 7.0, r.center().y),
            2.0,
            mode_colour(mode, held),
        );
    }
    let resp = ui.interact(r, uid, Sense::click());
    let mut picked = None;
    let mut open_editor = false;
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(150.0);
        for m in AutoMode::ALL {
            if ui.selectable_label(m == mode, tr(m.label())).clicked() {
                picked = Some(m);
                ui.close();
            }
        }
        ui.separator();
        if ui.button(tr("Show Automation")).clicked() {
            open_editor = true;
            ui.close();
        }
    });
    if let Some(m) = picked {
        let beat = p.position_beats;
        crate::automation::set_mode(&mut p.tracks[i], m, beat);
    }
    if open_editor {
        v.auto_editor.track = Some(i);
    }
}

/// How the strip's name is shown at the bottom.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NameStyle {
    /// Coloured label (Mixer).
    Coloured,
    /// Plain text (Inspector).
    Plain,
}

/// Draws one channel strip in `strip` and applies its edits to the project.
/// `ns` keeps widget ids unique when the same strip is shown in two places.
#[allow(clippy::too_many_arguments)]
pub fn channel_strip(
    ui: &Ui,
    g: &Painter,
    strip: Rect,
    key: StripKey,
    p: &mut Project,
    v: &mut MixerView,
    meters: &Meters,
    r: &Rows,
    ns: &'static str,
    name_style: NameStyle,
) {
    let selected = matches!(key, StripKey::Track(i) if i == p.selected_track);
    let inactive = matches!(key, StripKey::Track(i)
        if p.is_muted(i) || crate::tracks_area::silenced_by_solo(p, i));
    let (plug, plug_text) = if inactive {
        (plugin_off_col(), text_dim_col())
    } else {
        (crate::theme::accent(), crate::theme::on_accent())
    };
    // Whole-strip click selects (registered first so the controls on top win).
    if let StripKey::Track(i) = key {
        let resp = ui.interact(strip, id(ns, (key, "strip")), Sense::click());
        if resp.clicked() || resp.secondary_clicked() {
            p.selected_track = i;
            p.linked_strips.clear();
            v.track_clicked = true;
        }
        strip_menu(&resp, p, v, i);
    }
    let linked = matches!(key, StripKey::Track(i) if p.linked_strips.contains(&i));
    let bg = if (selected && name_style == NameStyle::Coloured) || linked {
        strip_sel_col()
    } else {
        strip_bg_col()
    };
    g.rect_filled(strip, 2.0, bg);
    let inner_w = strip.width() - 8.0;
    let sx = strip.left() + 4.0;
    let row = |y: f32, h: f32| Rect::from_min_size(pos2(sx, y), vec2(inner_w, h));

    let (name, color, _inserts, kind, instrument) = match key {
        StripKey::Track(i) => {
            let t = &p.tracks[i];
            (
                t.name.clone(),
                t.color,
                t.inserts
                    .iter()
                    .map(|x| (x.name(), x.bypass))
                    .collect::<Vec<_>>(),
                Some(t.kind),
                // An instrument slot to fill, as there are no built-in sounds.
                matches!(t.kind, TrackKind::Instrument | TrackKind::Drummer)
                    .then_some(crate::model::NO_INSTRUMENT),
            )
        }
        StripKey::StereoOut => (tr("Stereo Out").to_string(), OUT_PURPLE, vec![], None, None),
    };
    let beat = p.position_beats;
    let reduction = match key {
        StripKey::Track(i) => meters.reduction.get(i).copied().unwrap_or(0.0),
        _ => 0.0,
    };

    // A VCA strip is a fader with Mute and Solo: none of the channel strip's slots.
    let vca_strip = kind == Some(TrackKind::Vca);
    if vca_strip {
        g.text(
            pos2(strip.center().x, r.setting + 8.0),
            Align2::CENTER_CENTER,
            tr("VCA"),
            FontId::proportional(12.0),
            text_dim_col(),
        );
    }
    if !vca_strip {
        slot(g, row(r.setting, 16.0), tr("Setting"), slot_gray_col(), text_col());
        if let StripKey::Track(i) = key {
            setting_menu(ui, row(r.setting, 16.0), p, v, i, id(ns, (key, "setting")));
        }
        let gr_bar = row(r.gain_red + 1.0, 6.0);
        g.rect_filled(gr_bar, 1.0, slot_empty_col());
        if reduction > 0.05 {
            // Gain reduction grows from the right, in orange.
            let w = gr_bar.width() * (reduction / 24.0).min(1.0);
            g.rect_filled(
                Rect::from_min_max(pos2(gr_bar.right() - w, gr_bar.top()), gr_bar.max),
                1.0,
                if inactive {
                    meter_off_col()
                } else {
                    Color32::from_rgb(0xe6, 0xa0, 0x2a)
                },
            );
        }
        if let StripKey::Track(i) = key {
            if matches!(
                p.tracks[i].kind,
                TrackKind::Instrument | TrackKind::Drummer | TrackKind::ExternalMidi
            ) {
                let fx_r = row(r.midi_fx, 16.0);
                let count = p.tracks[i].midi_fx.len();
                let text = if count > 0 {
                    format!("MIDI FX {count}")
                } else {
                    "MIDI FX".to_string()
                };
                slot(g, fx_r, &text, slot_empty_col(), text_dim_col());
                let opened = ui
                    .interact(fx_r, id(ns, (key, "midi_fx")), Sense::click())
                    .on_hover_text("Open this track's MIDI FX")
                    .clicked();
                if opened {
                    p.selected_track = i;
                    v.open_midi_fx = true;
                }
            }
        }
        let input_r = row(r.input, 16.0);
        let hosted = match key {
            StripKey::Track(i) => p.tracks[i].instrument_plugin.is_some().then_some(i),
            _ => None,
        };
        match (kind, instrument) {
            _ if hosted.is_some() => {
                if let Some(i) = hosted {
                    instrument_plugin_slot(ui, g, input_r, p, v, i, id(ns, (key, "instrument")));
                }
            }
            (Some(TrackKind::Instrument | TrackKind::Drummer), Some(inst)) => {
                slot(g, input_r, inst, slot_empty_col(), text_col());
                if let StripKey::Track(i) = key {
                    let resp = ui
                        .interact(input_r, id(ns, (key, "choose")), Sense::click())
                        .on_hover_text_at_pointer("Click to choose an instrument plugin");
                    instrument_menu(&resp, true, p, v, i);
                }
            }
            // An aux strip's input is the bus it listens to: Aux 1 takes Bus 1, and so on.
            (Some(TrackKind::Aux), _) => {
                let n = match key {
                    StripKey::Track(i) => p.aux_bus_of(i),
                    _ => 1,
                };
                let text = if n == 0 {
                    String::new()
                } else {
                    trf("Bus {}", &[&n])
                };
                slot(g, input_r, &text, plug, plug_text);
                if let StripKey::Track(i) = key {
                    let resp = ui.interact(input_r, id(ns, (key, "aux_bus")), Sense::click());
                    egui::Popup::menu(&resp).show(|ui| {
                        ui.set_min_width(170.0);
                        let names = p.aux_buses();
                        egui::ScrollArea::vertical()
                            .max_height(320.0)
                            .show(ui, |ui| {
                                for bus in 1..=64u16 {
                                    let taken = names.iter().find(|(b, _)| *b == bus && bus != n);
                                    let text = match taken {
                                        Some((_, name)) => format!("Bus {bus}: {name}"),
                                        None => format!("Bus {bus}"),
                                    };
                                    if ui.selectable_label(bus == n, text).clicked() {
                                        p.set_aux_bus(i, bus);
                                        ui.close();
                                    }
                                }
                            });
                    });
                }
            }
            // An external MIDI strip shows where its notes go: the port and the channel.
            (Some(TrackKind::ExternalMidi), _) => {
                let text = match key {
                    StripKey::Track(i) => {
                        let t = &p.tracks[i];
                        let port = t
                            .midi_port
                            .clone()
                            .unwrap_or_else(|| tr("No Port").to_string());
                        format!("{port} · {}", t.midi_channel)
                    }
                    _ => String::new(),
                };
                let fitted = elide(g, &text, &FontId::proportional(10.5), input_r.width() - 8.0);
                slot(g, input_r, &fitted, plug, plug_text);
            }
            (Some(TrackKind::Audio), _) => {
                if let StripKey::Track(i) = key {
                    let slot_id = id(ns, (key, "input"));
                    input_slot(ui, g, input_r, p, &mut v.inputs, i, slot_id);
                }
            }
            _ => slot(g, input_r, "", slot_empty_col(), text_col()),
        }
        // An instrument strip: a line with its name between the instrument and the effects after it.
        if matches!(kind, Some(TrackKind::Instrument | TrackKind::Drummer)) {
            let y = (input_r.bottom() + row(r.fx, 15.0).top()) * 0.5;
            let line = Stroke::new(1.0, crate::theme::accent().gamma_multiply(0.55));
            let mid = input_r.center().x;
            g.line_segment([pos2(input_r.left() + 2.0, y), pos2(mid - 18.0, y)], line);
            g.line_segment([pos2(mid + 18.0, y), pos2(input_r.right() - 2.0, y)], line);
            g.text(
                pos2(mid, y),
                Align2::CENTER_CENTER,
                "FX",
                FontId::proportional(9.0),
                text_dim_col(),
            );
        }
        for shown in 0..r.fx_rows {
            let k = shown + r.fx_first;
            let rr = row(r.fx + shown as f32 * 17.0, 15.0);
            match key {
                StripKey::Track(i) => {
                    insert_slot(ui, g, rr, p, v, i, k, id(ns, (key, "fx", k)), inactive)
                }
                _ => master_slot(ui, g, rr, p, v, k, id(ns, (key, "fx", k))),
            }
        }
        match key {
            StripKey::Track(i) => {
                let area = row(r.sends, 16.0);
                let mut buses = p.aux_buses();
                buses.retain(|(bus, _)| !p.bus_would_loop(i, *bus));
                let mut new_aux = false;
                send_slots(
                    ui,
                    g,
                    area,
                    r.sends,
                    r.send_rows,
                    &mut p.tracks[i].sends,
                    id(ns, (key, "send")),
                    inactive,
                    &buses,
                    &mut new_aux,
                );
                if new_aux {
                    v.new_aux = Some((i, true));
                }
            }
            StripKey::StereoOut => slot(g, row(r.sends, 16.0), "", slot_empty_col(), text_col()),
        }
        if let StripKey::Track(_) = key {
            let out = if name_style == NameStyle::Plain {
                tr("Stereo Out")
            } else {
                tr("St Out")
            };
            if let StripKey::Track(i) = key {
                output_slot(
                    ui,
                    g,
                    row(r.output, 16.0),
                    p,
                    i,
                    out,
                    id(ns, (key, "output")),
                );
                group_slot(ui, g, row(r.group, 16.0), p, i, id(ns, (key, "vca")));
            }
        }
        if key == StripKey::StereoOut {
            let rr = row(r.automation, 16.0);
            slot(
                g,
                rr,
                tr("Read"),
                slot_empty_col(),
                Color32::from_rgb(0xa8, 0x9c, 0xf2),
            );
            g.rect_stroke(
                rr,
                3.0,
                Stroke::new(1.0, Color32::from_rgb(0x6a, 0x5a, 0xc0)),
                StrokeKind::Inside,
            );
        } else if let StripKey::Track(i) = key {
            automation_slot(
                ui,
                g,
                row(r.automation, 16.0),
                p,
                v,
                i,
                id(ns, (key, "auto")),
            );
        }
    }
    if let (StripKey::Track(i), true) = (key, crate::settings::show_track_icons()) {
        let icon = Rect::from_center_size(pos2(strip.center().x, r.icon + 12.0), vec2(22.0, 22.0));
        crate::track_icons::draw_tinted(
            g,
            icon,
            crate::track_icons::track_icon(&p.tracks[i]),
            color,
        );
    }
    let pan_c = pos2(strip.center().x, r.pan + 12.0);
    match key {
        _ if vca_strip => {}
        // The spatial mix's renderer lives on the master: its bed format and where it is heard.
        StripKey::StereoOut if p.spatial_mode != ovrtone_engine::SpatialMode::Off => {
            let spot = Rect::from_center_size(pan_c, vec2(strip.width() - 10.0, 22.0));
            let resp = ui
                .interact(spot, id(ns, (key, "renderer")), Sense::click())
                .on_hover_text("The spatial renderer: bed format, output, speakers and exports");
            let fill = if resp.hovered() {
                crate::spatial_window::PANEL.gamma_multiply(1.3)
            } else {
                crate::spatial_window::PANEL
            };
            g.rect_filled(spot, 4.0, fill);
            let bed = p.spatial_mode.layout().map_or("", |l| l.name());
            g.text(
                spot.center(),
                Align2::CENTER_CENTER,
                format!("Renderer {bed}"),
                FontId::proportional(9.5),
                crate::spatial_window::VALUE,
            );
            if resp.clicked() {
                v.open_renderer = true;
            }
        }
        // An object has a place in the room, not a pan: its knob opens where that is set.
        StripKey::Track(i)
            if p.tracks[i].object.is_some() =>
        {
            let spot = Rect::from_center_size(pan_c, vec2(26.0, 26.0));
            let resp = ui
                .interact(spot, id(ns, (key, "object")), Sense::click())
                .on_hover_text(if p.tracks[i].bed_pan {
                    "Surround panned in the bed. Click to place it."
                } else {
                    "An object of the spatial mix. Click to place it."
                });
            // A small room seen from above, with the track where it is.
            let room = crate::spatial_window::DARK_ROOM;
            g.circle_filled(pan_c, 12.0, room);
            g.circle_stroke(pan_c, 12.0, Stroke::new(1.0, crate::spatial_window::RING));
            g.circle_stroke(pan_c, 6.0, Stroke::new(0.6, crate::spatial_window::RING));
            if let Some(o) = p.tracks[i].object {
                let reach = (1.0 - o.elevation.clamp(0.0, 90.0) / 90.0) * 10.0 * o.radius.clamp(0.0, 1.0).max(0.15);
                let a = o.azimuth.to_radians();
                let at = pan_c + vec2(a.sin(), -a.cos()) * reach;
                let dot = if p.tracks[i].bed_pan {
                    crate::spatial_window::SURROUND
                } else {
                    crate::spatial_window::OBJECT
                };
                g.line_segment([pan_c, at], Stroke::new(1.0, dot.gamma_multiply(0.6)));
                g.circle_filled(at, 3.2, dot);
            }
            if resp.hovered() {
                g.circle_stroke(pan_c, 13.0, Stroke::new(1.0, crate::theme::accent()));
            }
            if resp.clicked() {
                p.selected_track = i;
                v.open_spatial = true;
            }
        }
        StripKey::Track(i) => {
            let t = &p.tracks[i];
            let shown = crate::automation::shown_value(t, AutoTarget::Pan, beat);
            let editable = crate::automation::editable(t, AutoTarget::Pan);
            let m = pan_knob(ui, g, pan_c, id(ns, (key, "pan")), shown, editable);
            if let Some(value) = m.value {
                let delta = value - shown;
                crate::automation::moved(&mut p.tracks[i], AutoTarget::Pan, value);
                for j in p.group_mates(i, |g| g.pan) {
                    let to = p.tracks[j].pan + delta;
                    crate::automation::moved(&mut p.tracks[j], AutoTarget::Pan, to);
                }
            }
            if m.stopped {
                let playing = p.playing;
                for j in std::iter::once(i).chain(p.group_mates(i, |g| g.pan)) {
                    crate::automation::released(&mut p.tracks[j], AutoTarget::Pan, beat, playing);
                }
            }
        }
        _ => {
            pan_knob(ui, g, pan_c, id(ns, (key, "pan")), 0.0, false);
        }
    }

    // Volume (dB) and peak-hold readouts.
    // Sends on Faders: the fader sets the strip's send to that bus instead of its level.
    let send_bus = match key {
        // An Aux can send too, only not into what would come back to it.
        StripKey::Track(i) if p.tracks[i].kind != TrackKind::Vca => {
            v.sends_on_faders.filter(|bus| !p.bus_would_loop(i, *bus))
        }
        _ => None,
    };
    let send_db = |sends: &[SendSlot], bus: u16| {
        let level = sends
            .iter()
            .find(|s| s.bus == Some(bus))
            .map_or(0.0, |s| s.level);
        lin_to_db(ovrtone_engine::send_gain(level)).max(-60.0)
    };
    let vol_db = match key {
        StripKey::Track(i) => match send_bus {
            Some(bus) => send_db(&p.tracks[i].sends, bus),
            None => crate::automation::shown_value(&p.tracks[i], AutoTarget::Volume, beat),
        },
        StripKey::StereoOut => lin_to_db(p.master_vol).max(-60.0),
    };
    let peaks = match key {
        StripKey::Track(i) => meters.tracks.get(i).copied().unwrap_or((0.0, 0.0)),
        _ => meters.master,
    };
    let (clip_reset, clipped) = match key {
        StripKey::Track(i) => (
            ClipReset::Track(i),
            meters.clips.get(i).copied().unwrap_or(false),
        ),
        _ => (ClipReset::Master, meters.master_clip),
    };
    let hold = v.peak_hold.entry(key).or_insert(0.0);
    let peak_now = peaks.0.max(peaks.1);
    let since = v
        .peak_hold_at
        .entry(key)
        .or_insert_with(std::time::Instant::now);
    let lets_go = crate::settings::peak_hold_ms();
    if peak_now >= *hold || (lets_go > 0 && since.elapsed().as_millis() >= u128::from(lets_go)) {
        *hold = peak_now;
        *since = std::time::Instant::now();
    }
    let half = (inner_w - 4.0) / 2.0;
    let vol_r = Rect::from_min_size(pos2(sx, r.db), vec2(half, 14.0));
    let pk_r = Rect::from_min_size(pos2(sx + half + 4.0, r.db), vec2(half, 14.0));
    slot(
        g,
        vol_r,
        &format!("{vol_db:.1}"),
        crate::theme::border(),
        text_col(),
    );
    let hold_db = lin_to_db(*hold);
    let pk_text = if *hold <= 1e-5 {
        String::new()
    } else {
        format!("{hold_db:.1}")
    };
    let pk_col = if hold_db > 0.0 {
        Color32::from_rgb(0xff, 0x5a, 0x4a)
    } else {
        READ_GREEN
    };
    // An over fills the readout, so it is seen from across the room.
    if hold_db > 0.0 {
        slot(g, pk_r, &pk_text, crate::theme::DANGER, Color32::WHITE);
    } else {
        slot(g, pk_r, &pk_text, crate::theme::border(), pk_col);
    }
    if ui
        .interact(pk_r, id(ns, (key, "peak")), Sense::click())
        .clicked()
    {
        *hold = 0.0;
        v.clip_resets.push(clip_reset);
    }

    // Fader + scale + meter.
    let name_h = 22.0;
    let fader_area = Rect::from_min_max(
        pos2(sx, r.fader_top),
        pos2(strip.right() - 4.0, strip.bottom() - 44.0 - name_h),
    );
    let fader_free = match key {
        StripKey::Track(i) => {
            send_bus.is_some() || crate::automation::editable(&p.tracks[i], AutoTarget::Volume)
        }
        StripKey::StereoOut => true,
    };
    let moved = fader_and_meter(
        ui,
        g,
        fader_area,
        id(ns, (key, "fader")),
        vol_db,
        fader_free,
        peaks,
        match key {
            StripKey::Track(i) => meters.rms.get(i).copied().unwrap_or(0.0),
            StripKey::StereoOut => 0.0,
        },
        inactive,
        clipped,
    );
    if let Some(all) = moved.reset_clip {
        v.clip_resets
            .push(if all { ClipReset::All } else { clip_reset });
        v.peak_hold.remove(&key);
    }
    if let Some(db) = moved.value {
        match key {
            StripKey::Track(i) => match send_bus {
                Some(bus) => set_send_level(&mut p.tracks[i].sends, bus, db),
                None => {
                    let delta = db - vol_db;
                    crate::automation::moved(&mut p.tracks[i], AutoTarget::Volume, db);
                    for j in p.group_mates(i, |g| g.volume) {
                        let to = p.tracks[j].volume_db + delta;
                        crate::automation::moved(&mut p.tracks[j], AutoTarget::Volume, to);
                    }
                }
            },
            StripKey::StereoOut => p.master_vol = 10f32.powf(db / 20.0).min(2.0),
        }
    }
    if let (true, StripKey::Track(i), None) = (moved.stopped, key, send_bus) {
        let playing = p.playing;
        for j in std::iter::once(i).chain(p.group_mates(i, |g| g.volume)) {
            crate::automation::released(&mut p.tracks[j], AutoTarget::Volume, beat, playing);
        }
    }

    // R / I, above M / S, on strips that record.
    if let StripKey::Track(i) = key {
        let t = &mut p.tracks[i];
        if t.kind.has_regions() {
            let y = strip.bottom() - 40.0 - name_h;
            let w = (inner_w - 6.0) / 2.0;
            let rec_r = Rect::from_min_size(pos2(sx, y), vec2(w, 15.0));
            if small_button(
                ui,
                g,
                rec_r,
                id(ns, (key, "rec")),
                "R",
                t.rec,
                crate::theme::DANGER,
            ) {
                t.rec = !t.rec;
            }
            if t.kind == TrackKind::Audio {
                let mon_r = Rect::from_min_size(pos2(sx + w + 6.0, y), vec2(w, 15.0));
                if small_button(ui, g, mon_r, id(ns, (key, "mon")), "I", t.monitor, SOLO_ON) {
                    t.monitor = !t.monitor;
                }
            }
        }
    }

    // M / S
    let by = strip.bottom() - 22.0 - name_h;
    let bw = (inner_w - 6.0) / 2.0;
    let m_r = Rect::from_min_size(pos2(sx, by), vec2(bw, 16.0));
    let s_r = Rect::from_min_size(pos2(sx + bw + 6.0, by), vec2(bw, 16.0));
    match key {
        StripKey::Track(i) => {
            let t = &mut p.tracks[i];
            if small_button(ui, g, m_r, id(ns, (key, "m")), "M", t.mute, MUTE_ON) {
                t.mute = !t.mute;
            }
            if small_button(ui, g, s_r, id(ns, (key, "s")), "S", t.solo, SOLO_ON) {
                t.solo = !t.solo;
            }
        }
        _ => {
            if key == StripKey::StereoOut {
                // Dim and mono sum, for the speakers, and the bounce.
                let half = m_r.width() / 2.0;
                let dim = Rect::from_min_size(m_r.min, vec2(half - 1.0, m_r.height()));
                let mono = Rect::from_min_size(m_r.min + vec2(half + 1.0, 0.0), dim.size());
                if small_button(ui, g, dim, id(ns, (key, "dim")), "D", p.out_dim, SOLO_ON) {
                    p.out_dim = !p.out_dim;
                }
                if small_button(ui, g, mono, id(ns, (key, "mono")), "1", p.out_mono, SOLO_ON) {
                    p.out_mono = !p.out_mono;
                }
                if small_button(ui, g, s_r, id(ns, (key, "bnc")), "", false, SOLO_ON) {
                    v.want_bounce = true;
                }
                let bounce = crate::icons::Icon::Bounce;
                crate::icons::paint(g, s_r.center(), 12.0, bounce, crate::theme::gray(0xe8));
            } else {
                slot(g, m_r, "M", slot_gray_col(), text_dim_col());
            }
        }
    }

    // Name. A double click renames, and leaves clicks and drags to what they do.
    let name_r = Rect::from_min_max(pos2(strip.left(), strip.bottom() - name_h), strip.max);
    if let StripKey::Track(i) = key {
        let renames = ui.input(|input| {
            input
                .pointer
                .button_double_clicked(egui::PointerButton::Primary)
                && input
                    .pointer
                    .interact_pos()
                    .is_some_and(|at| name_r.contains(at))
        });
        if renames {
            v.rename = Some(i);
        }
    }
    if let (StripKey::Track(i), NameStyle::Coloured) = (key, name_style) {
        // Only a drag: a click or the right button goes through to the strip underneath.
        ui.interact(name_r, id(ns, (key, "name")), Sense::drag())
            .on_hover_text_at_pointer("Drag to move this strip")
            .dnd_set_drag_payload(StripDrag(i));
    }
    let font = FontId::proportional(10.5);
    let shown = elide(g, &name, &font, name_r.width() - 6.0);
    match name_style {
        // A plate in the accent, with the track's own colour kept along its top to tell it by.
        NameStyle::Coloured | NameStyle::Plain => {
            let plate = name_r.shrink2(vec2(1.0, 1.0));
            g.rect_filled(plate, 4.0, crate::theme::accent());
            g.rect_filled(
                Rect::from_min_size(plate.min + vec2(4.0, 0.0), vec2(plate.width() - 8.0, 2.0)),
                1.0,
                color,
            );
            g.text(
                plate.center() + vec2(0.0, 1.0),
                Align2::CENTER_CENTER,
                shown,
                font,
                crate::theme::on_accent(),
            );
        }
    }
    if key == StripKey::StereoOut && name_style == NameStyle::Coloured {
        g.rect_stroke(
            strip,
            2.0,
            Stroke::new(1.5, Color32::from_rgb(0xb8, 0x3c, 0xc8)),
            StrokeKind::Inside,
        );
    }
    if selected && name_style == NameStyle::Coloured {
        g.rect_stroke(
            strip,
            2.0,
            Stroke::new(1.0, crate::theme::gray(0x64)),
            StrokeKind::Inside,
        );
    }
}

/// Sets the send to `bus` from a fader position in dB (a new send is made when needed, and
/// one turned right down is taken away).
fn set_send_level(sends: &mut Vec<SendSlot>, bus: u16, db: f32) {
    let level = if db <= -59.5 {
        0.0
    } else {
        (10f32.powf(db / 20.0) / 2.0).sqrt().clamp(0.0, 1.0)
    };
    match sends.iter().position(|s| s.bus == Some(bus)) {
        Some(k) => sends[k].level = level,
        None if level > 0.0 && sends.len() < MAX_SENDS => sends.push(SendSlot {
            bus: Some(bus),
            level,
            pre: false,
        }),
        None => {}
    }
}

/// The strip's Group slot: the mix group of the track, else the VCA track that moves its level
/// (empty for none). Click to pick a mix group, make a new one, change what the group moves
/// together, or pick one of the project's VCA tracks.
/// A strip's Setting slot: copy, paste or clear what is on the strip.
fn setting_menu(ui: &Ui, r: Rect, p: &mut Project, v: &mut MixerView, i: usize, id: egui::Id) {
    let resp = ui.interact(r, id, Sense::click());
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(170.0);
        if ui.button(tr("Copy Channel Strip")).clicked() {
            // `App` takes the copy, once the plugins have given their current settings.
            v.strip_cmd = Some(crate::plugin_presets::StripCmd::Copy { track: i });
            ui.close();
        }
        let paste = egui::Button::new(tr("Paste Channel Strip"));
        if ui.add_enabled(v.copied.is_some(), paste).clicked() {
            if let Some(c) = &v.copied {
                c.paste_on(&mut p.tracks[i]);
                p.song_rev += 1;
            }
            ui.close();
        }
        ui.separator();
        if ui.button("Save Channel Strip...").clicked() {
            let name = p.tracks[i].name.clone();
            if let Some(file) = crate::plugin_presets::ask_where_strip(&name) {
                v.strip_cmd = Some(crate::plugin_presets::StripCmd::Save { track: i, file });
            }
            ui.close();
        }
        let saved = crate::plugin_presets::list_strips();
        ui.add_enabled_ui(!saved.is_empty(), |ui| {
            ui.menu_button("Load Channel Strip", |ui| {
                for (name, file) in saved {
                    if ui.button(name).clicked() {
                        let read = std::fs::read_to_string(&file).map_err(|e| e.to_string());
                        let done = read.and_then(|text| {
                            crate::project_io::strip_from_text(&mut p.tracks[i], &text)
                        });
                        match done {
                            Ok(()) => p.song_rev += 1,
                            Err(e) => v.strip_error = Some(e),
                        }
                        ui.close();
                    }
                }
            });
        });
        ui.separator();
        if ui.button("Rename...").clicked() {
            v.rename = Some(i);
            ui.close();
        }
        if p.tracks[i].kind == TrackKind::Aux {
            let on = p.tracks[i].on_timeline;
            let label = if on {
                "Remove from Tracks Area"
            } else {
                "Create Track"
            };
            if ui.button(label).clicked() {
                p.tracks[i].on_timeline = !on;
                ui.close();
            }
        }
        ui.separator();
        if ui.button(tr("Reset Channel Strip")).clicked() {
            let t = &mut p.tracks[i];
            t.inserts.clear();
            t.plugins.clear();
            t.sends.clear();
            t.volume_db = 0.0;
            t.pan = 0.0;
            p.song_rev += 1;
            ui.close();
        }
    });
}

/// A strip's Output slot: Stereo Out or a bus (an Aux strip plays that bus). An Aux strip
/// always goes to Stereo Out.
fn output_slot(
    ui: &Ui,
    g: &Painter,
    r: Rect,
    p: &mut Project,
    i: usize,
    stereo_out: &str,
    id: egui::Id,
) {
    let current = p.tracks[i].output;
    let hardware = p.tracks[i].hardware_out;
    let text = if hardware > 0 {
        let first = u16::from(hardware) * 2 + 1;
        format!("Output {}-{}", first, first + 1)
    } else if current == 0 {
        stereo_out.to_string()
    } else {
        trf("Bus {}", &[&current])
    };
    slot(g, r, &text, slot_gray_col(), text_col());
    let resp = ui.interact(r, id, Sense::click());
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(150.0);
        if ui
            .selectable_label(current == 0 && hardware == 0, tr("Stereo Out"))
            .clicked()
        {
            p.tracks[i].output = 0;
            p.tracks[i].hardware_out = 0;
            ui.close();
        }
        for pair in 1..=p.hardware_pairs() {
            let first = u16::from(pair) * 2 + 1;
            let name = format!("Output {}-{}", first, first + 1);
            if ui.selectable_label(hardware == pair, name).clicked() {
                p.tracks[i].output = 0;
                p.tracks[i].hardware_out = pair;
                ui.close();
            }
        }
        if ui.button("New Aux").clicked() {
            p.new_aux_for(i, false);
            ui.close();
        }
        ui.separator();
        let mut buses = p.aux_buses();
        buses.retain(|(bus, _)| !p.bus_would_loop(i, *bus));
        egui::ScrollArea::vertical()
            .max_height(300.0)
            .show(ui, |ui| {
                for (bus, name) in &buses {
                    if ui
                        .selectable_label(current == *bus, format!("Bus {bus}: {name}"))
                        .clicked()
                    {
                        p.tracks[i].output = *bus;
                        p.tracks[i].hardware_out = 0;
                        ui.close();
                    }
                }
            });
    });
}

/// An audio strip's Input slot: which inputs the track records, and from which device.
fn input_slot(
    ui: &Ui,
    g: &Painter,
    r: Rect,
    p: &mut Project,
    inputs: &mut crate::audio_rec::InputChoice,
    i: usize,
    id: egui::Id,
) {
    use ovrtone_engine::InputSource;
    let source = p.tracks[i].input;
    slot(g, r, &inputs.name(source), slot_gray_col(), text_col());
    // One ring for a mono input, two for a stereo pair.
    let ring = pos2(r.left() + 8.0, r.center().y);
    g.circle_stroke(ring, 3.5, stroke(text_col()));
    if matches!(source, InputSource::Stereo(_)) {
        g.circle_stroke(ring + vec2(3.0, 0.0), 3.5, stroke(text_col()));
    }
    let resp = ui.interact(r, id, Sense::click());
    if resp.clicked() {
        inputs.refresh();
    }
    let channels = inputs.channels();
    let (mut picked, mut device) = (None, None);
    let mut renamed: Option<(u16, String)> = None;
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(150.0);
        let names: Vec<String> = (0..channels)
            .map(|c| inputs.name(InputSource::Mono(c)))
            .collect();
        let pairs: Vec<String> = (0..channels.saturating_sub(1))
            .step_by(2)
            .map(|c| inputs.name(InputSource::Stereo(c)))
            .collect();
        let mut item = |ui: &mut Ui, s: InputSource, name: &str| {
            if ui.selectable_label(s == source, name).clicked() {
                picked = Some(s);
                ui.close();
            }
        };
        for (c, name) in (0..channels).zip(&names) {
            item(ui, InputSource::Mono(c), name);
        }
        ui.separator();
        for (c, name) in (0..channels.saturating_sub(1)).step_by(2).zip(&pairs) {
            item(ui, InputSource::Stereo(c), name);
        }
        ui.separator();
        // The user's names for the inputs of this device, kept on this computer.
        ui.menu_button("Name Inputs", |ui| {
            for c in 0..channels {
                ui.horizontal(|ui| {
                    ui.label(format!("{}", c + 1));
                    let mut text = inputs.label(c).unwrap_or_default().to_string();
                    let edit = egui::TextEdit::singleline(&mut text)
                        .hint_text(format!("Input {}", c + 1))
                        .desired_width(120.0);
                    if ui.add(edit).changed() {
                        renamed = Some((c, text));
                    }
                });
            }
        });
        ui.separator();
        ui.menu_button(tr("Input Device"), |ui| {
            if ui
                .selectable_label(inputs.device.is_none(), tr("System Default"))
                .clicked()
            {
                device = Some(None);
                ui.close();
            }
            for d in &inputs.devices {
                let on = inputs.device.as_deref() == Some(d.id.as_str());
                if ui.selectable_label(on, &d.name).clicked() {
                    device = Some(Some(d.id.clone()));
                    ui.close();
                }
            }
            if inputs.devices.is_empty() {
                ui.label(tr("No audio input found"));
            }
        });
    });
    if let Some(s) = picked {
        p.tracks[i].input = s;
    }
    if let Some(d) = device {
        inputs.pick(d);
    }
    if let Some((c, name)) = renamed {
        inputs.set_label(c, &name);
    }
}

fn group_slot(ui: &Ui, g: &Painter, r: Rect, p: &mut Project, i: usize, id: egui::Id) {
    let group = p.tracks[i].group.filter(|g| *g < p.groups.len());
    let name = match group {
        Some(g) => p.groups[g].name.clone(),
        None => p.vca_of(i).map(|v| v.name.clone()).unwrap_or_default(),
    };
    let fill = if name.is_empty() {
        slot_empty_col()
    } else {
        slot_gray_col()
    };
    slot(
        g,
        r,
        &elide(g, &name, &FontId::proportional(10.5), r.width() - 8.0),
        fill,
        text_col(),
    );
    let vcas: Vec<(usize, String)> = p
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.kind == TrackKind::Vca)
        .map(|(j, t)| (j, t.name.clone()))
        .collect();
    let resp = ui.interact(r, id, Sense::click());
    let current = p.tracks[i].vca;
    let mut picked = None;
    let mut group_pick: Option<Option<usize>> = None;
    let mut new_group = false;
    let mut delete = None;
    let names: Vec<String> = p.groups.iter().map(|g| g.name.clone()).collect();
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(150.0);
        if ui
            .selectable_label(group.is_none(), tr("No Group"))
            .clicked()
        {
            group_pick = Some(None);
            ui.close();
        }
        for (k, name) in names.iter().enumerate() {
            if ui.selectable_label(group == Some(k), name).clicked() {
                group_pick = Some(Some(k));
                ui.close();
            }
        }
        if ui.button(tr("New Group")).clicked() {
            new_group = true;
            ui.close();
        }
        if let Some(k) = group {
            ui.menu_button(tr("Group Settings"), |ui| {
                let grp = &mut p.groups[k];
                ui.checkbox(&mut grp.volume, tr("Volume"));
                ui.checkbox(&mut grp.pan, tr("Pan"));
                ui.checkbox(&mut grp.mute, tr("Mute"));
                ui.checkbox(&mut grp.solo, tr("Solo"));
            });
            if ui.button(tr("Delete Group")).clicked() {
                delete = Some(k);
                ui.close();
            }
        }
        if !vcas.is_empty() {
            ui.separator();
            if ui
                .selectable_label(current.is_none(), tr("No VCA"))
                .clicked()
            {
                picked = Some(None);
                ui.close();
            }
            for (j, name) in &vcas {
                if ui.selectable_label(current == Some(*j), name).clicked() {
                    picked = Some(Some(*j));
                    ui.close();
                }
            }
        }
    });
    if let Some(vca) = picked {
        p.assign_vca(i, vca);
    }
    if let Some(pick) = group_pick {
        p.assign_group(i, pick);
    }
    if new_group {
        let n = p.groups.len() + 1;
        if let Some(k) = p.add_group(&format!("Group {n}")) {
            p.assign_group(i, Some(k));
        }
    }
    if let Some(k) = delete {
        p.delete_group(k);
    }
}

fn toolbar(ui: &Ui, p: &mut Project, s: &mut UiState, bar: Rect) {
    const NS: &str = "mixer_toolbar";
    let g = ui.painter_at(bar);
    g.rect_filled(bar, 0.0, toolbar_bg_col());
    g.line_segment(
        [bar.left_bottom(), bar.right_bottom()],
        Stroke::new(1.0, crate::theme::border()),
    );
    let cy = bar.center().y;
    back_chevron(&g, pos2(bar.left() + 16.0, cy));
    let (can_undo, can_redo) = (s.can_undo, s.can_redo);
    let mut undo_pick = None;
    let v = &mut s.mixer;

    let mut x = bar.left() + 32.0;
    let (edit, w) = menu_title(ui, &g, x, cy, tr("Edit"), id(NS, "Edit"));
    x += w + 2.0;
    egui::Popup::menu(&edit).show(|ui| {
        ui.set_min_width(220.0);
        crate::widgets::undo_items(ui, can_undo, can_redo, &mut undo_pick);
        if ui.button(tr("Copy Channel Strip")).clicked() {
            v.strip_cmd = Some(crate::plugin_presets::StripCmd::Copy {
                track: p.selected_track,
            });
            ui.close();
        }
        let can_paste = v.copied.is_some() && !p.tracks.is_empty();
        if ui
            .add_enabled(can_paste, egui::Button::new(tr("Paste Channel Strip")))
            .clicked()
        {
            if let (Some(c), Some(t)) = (&v.copied, p.tracks.get_mut(p.selected_track)) {
                c.paste_on(t);
                p.song_rev += 1;
            }
            ui.close();
        }
        if ui.button(tr("Select Audio Channel Strips")).clicked() {
            p.linked_strips = (0..p.tracks.len())
                .filter(|i| p.tracks[*i].kind == TrackKind::Audio)
                .collect();
            ui.close();
        }
    });
    if undo_pick.is_some() {
        s.undo_cmd = undo_pick;
    }
    let (options, w) = menu_title(ui, &g, x, cy, tr("Options"), id(NS, "Options"));
    x += w + 2.0;
    egui::Popup::menu(&options).show(|ui| {
        ui.set_min_width(220.0);
        if ui
            .button(tr("Create New Auxiliary Channel Strip"))
            .clicked()
        {
            p.add_track(TrackKind::Aux);
            ui.close();
        }
        let pairs = p.hardware_pairs();
        if pairs == 0 {
            ui.add_enabled(false, egui::Button::new(tr("Create New Output Channel Strip")))
                .on_disabled_hover_text("Your audio device has two outputs. Pick one with more (Settings > Audio) for Output 3-4 and up.");
        } else {
            ui.menu_button(tr("Create New Output Channel Strip"), |ui| {
                for pair in 1..=pairs {
                    let first = u16::from(pair) * 2 + 1;
                    if ui
                        .button(format!("Output {}-{}", first, first + 1))
                        .clicked()
                    {
                        p.new_output_strip(pair);
                        ui.close();
                    }
                }
            });
        }
        if ui.button(tr("Create New VCA")).clicked() {
            p.add_track(TrackKind::Vca);
            ui.close();
        }
        if ui.button(tr("Create Group…")).clicked() {
            let n = p.groups.len() + 1;
            if let Some(k) = p.add_group(&format!("Group {n}")) {
                let sel = p.selected_track;
                p.assign_group(sel, Some(k));
            }
            ui.close();
        }
    });
    let (view, w) = menu_title(ui, &g, x, cy, tr("View"), id(NS, "View"));
    x += w + 12.0;
    egui::Popup::menu(&view).show(|ui| {
        ui.set_min_width(220.0);
        if ui
            .selectable_label(v.wide, tr("Wide Channel Strips"))
            .clicked()
        {
            v.wide = !v.wide;
            ui.close();
        }
        if ui
            .selectable_label(v.signal_flow, tr("Show Signal Flow Channel Strips"))
            .clicked()
        {
            v.signal_flow = !v.signal_flow;
            ui.close();
        }
    });

    let menus_end = x;
    // Right-hand side first (strip width buttons, then filters), so the rest can be fitted
    // into what is left.
    let mut rx = bar.right() - 8.0;
    for (wide, key) in [(true, "wide"), (false, "narrow")] {
        let r = Rect::from_min_size(pos2(rx - 26.0, cy - 10.0), vec2(26.0, 20.0));
        let resp = ui.interact(r, id(NS, key), Sense::click());
        if v.wide == wide {
            g.rect_filled(r, 4.0, crate::theme::gray(0x30));
        }
        let c = r.center();
        let iw = if wide { 13.0 } else { 9.0 };
        g.rect_stroke(
            Rect::from_center_size(c, vec2(iw, 11.0)),
            1.5,
            stroke(text_col()),
            StrokeKind::Middle,
        );
        g.line_segment([c + vec2(0.0, -5.5), c + vec2(0.0, 5.5)], stroke(text_col()));
        if resp.clicked() {
            v.wide = wide;
        }
        rx = r.left() - 4.0;
    }
    rx -= 10.0;
    let filter_widths: Vec<f32> = FILTERS
        .iter()
        .map(|label| {
            g.layout_no_wrap(tr(label).into(), FontId::proportional(12.5), text_col())
                .size()
                .x
                + 16.0
        })
        .collect();
    let filters_w = filter_widths.iter().map(|w| w + 3.0).sum::<f32>();
    let fit = fit_toolbar(menus_end, bar.center().x, rx, filters_w);

    if fit.filters_inline {
        for (i, label) in FILTERS.iter().enumerate().rev() {
            let w = filter_widths[i];
            let r = Rect::from_min_size(pos2(rx - w, cy - 10.0), vec2(w, 20.0));
            let resp = ui.interact(r, id(NS, ("filter", i)), Sense::click());
            let on = v.filters[i];
            g.rect_filled(r, 4.0, if on { blue() } else { crate::theme::gray(0x2b) });
            g.text(
                r.center(),
                Align2::CENTER_CENTER,
                tr(label),
                FontId::proportional(12.5),
                if on { crate::theme::on_accent() } else { text_dim_col() },
            );
            if resp.clicked() {
                v.filters[i] = !v.filters[i];
            }
            rx = r.left() - 3.0;
        }
    } else {
        // Not enough room for a button per strip type: one menu with a check for each.
        let r = Rect::from_min_size(
            pos2(rx - FILTER_MENU_W, cy - 10.0),
            vec2(FILTER_MENU_W, 20.0),
        );
        let resp = popup_button(ui, &g, r, id(NS, "filters_menu"), tr("Show"));
        egui::Popup::menu(&resp).show(|ui| {
            ui.set_min_width(160.0);
            for (i, label) in FILTERS.iter().enumerate() {
                if ui.checkbox(&mut v.filters[i], tr(label)).clicked() {
                    ui.close();
                }
            }
        });
    }

    if fit.show_sends {
        let buses = p.aux_buses();
        draw_sends_on_faders(ui, &g, menus_end, cy, &buses, &mut v.sends_on_faders);
    }
    let left = fit.modes_left;

    // Single | Tracks | All
    let modes = [
        (MixerMode::Single, tr("Single")),
        (MixerMode::Tracks, tr("Tracks")),
        (MixerMode::All, tr("All")),
    ];
    let seg_w = MODE_W / 3.0;
    for (i, (m, label)) in modes.iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(left + i as f32 * seg_w, cy - 10.0),
            vec2(seg_w - 2.0, 20.0),
        );
        let resp = ui.interact(r, id(NS, ("mode", *label)), Sense::click());
        let on = v.mode == *m;
        if on {
            g.rect_filled(r, 4.0, blue());
        } else if resp.hovered() {
            g.rect_filled(r, 4.0, Color32::from_white_alpha(14));
        }
        g.text(
            r.center(),
            Align2::CENTER_CENTER,
            *label,
            FontId::proportional(12.5),
            if on { crate::theme::on_accent() } else { text_col() },
        );
        if resp.clicked() {
            v.mode = *m;
        }
    }
}

/// Width of the Single | Tracks | All switch, of the "Sends on Faders" group and of the
/// "Show" menu that replaces the filter buttons when they do not fit.
const MODE_W: f32 = 174.0;
const SENDS_W: f32 = 256.0;
const FILTER_MENU_W: f32 = 70.0;
const TOOLBAR_GAP: f32 = 10.0;

/// Where the Mixer toolbar's groups go so that none overlaps another: the mode switch at the
/// centre when there is room, else just left of the filters; the filters as buttons when they
/// fit, else as one menu; "Sends on Faders" only in the room left over.
#[derive(Clone, Copy, PartialEq, Debug)]
struct ToolbarFit {
    modes_left: f32,
    filters_inline: bool,
    show_sends: bool,
}

fn fit_toolbar(menus_end: f32, center: f32, right_end: f32, filters_w: f32) -> ToolbarFit {
    let place = |filters_left: f32| {
        let centred = center - MODE_W / 2.0 + 60.0;
        centred.min(filters_left - TOOLBAR_GAP - MODE_W)
    };
    let mut filters_inline = true;
    let mut modes_left = place(right_end - filters_w);
    if modes_left < menus_end + TOOLBAR_GAP {
        filters_inline = false;
        modes_left = place(right_end - FILTER_MENU_W);
    }
    // When even that is too tight, the modes stay just after the menus (the panel has a
    // minimum width, so this only happens in very narrow windows).
    modes_left = modes_left.max(menus_end + TOOLBAR_GAP);
    let show_sends = menus_end + SENDS_W + TOOLBAR_GAP <= modes_left;
    ToolbarFit {
        modes_left,
        filters_inline,
        show_sends,
    }
}

fn draw_sends_on_faders(
    ui: &Ui,
    g: &egui::Painter,
    x: f32,
    cy: f32,
    buses: &[(u16, String)],
    bus: &mut Option<u16>,
) {
    const NS: &str = "mixer_toolbar";
    // Sends on Faders: the faders set the sends to a bus.
    // Placed by the width of the words, so the power symbol never sits on them in any language.
    let words = g.layout_no_wrap(tr("Sends on Faders:").to_string(), FontId::proportional(12.5), text_dim_col());
    let words_w = words.size().x;
    g.galley(pos2(x, cy - words.size().y / 2.0), words, text_dim_col());
    let pw = pos2(x + words_w + 12.0, cy + 1.0);
    g.circle_stroke(pw, 5.5, stroke(text_dim_col()));
    g.line_segment(
        [pw + vec2(0.0, -7.5), pw + vec2(0.0, -1.5)],
        Stroke::new(3.0, toolbar_bg_col()),
    );
    g.line_segment(
        [pw + vec2(0.0, -7.0), pw + vec2(0.0, -2.0)],
        stroke(text_dim_col()),
    );
    let sof = Rect::from_min_size(pos2(pw.x + 14.0, cy - 10.0), vec2(110.0, 20.0));
    let label = match bus {
        Some(b) => trf("Bus {}", &[b]),
        None => tr("Off").to_string(),
    };
    let resp = popup_button(ui, g, sof, id(NS, "sends_on_faders"), &label);
    egui::Popup::menu(&resp).show(|ui| {
        if ui.selectable_label(bus.is_none(), tr("Off")).clicked() {
            *bus = None;
            ui.close();
        }
        for (b, name) in buses {
            if ui
                .selectable_label(*bus == Some(*b), format!("Bus {b}: {name}"))
                .clicked()
            {
                *bus = Some(*b);
                ui.close();
            }
        }
    });
}

/// Whether a strip of `kind` is shown with the toolbar's filter buttons set as `filters`
/// (Audio, Inst, Aux, Bus, Input, Output, Master/VCA, MIDI).
fn strip_visible(kind: TrackKind, filters: &[bool; 8]) -> bool {
    match kind {
        TrackKind::Audio => filters[0],
        TrackKind::Instrument | TrackKind::Drummer => filters[1],
        TrackKind::Aux => filters[2],
        TrackKind::Bus => filters[3],
        TrackKind::Output => filters[5],
        TrackKind::Vca => filters[6],
        TrackKind::ExternalMidi => filters[7],
        // A folder is only a way to group tracks; it has no channel strip.
        TrackKind::Folder => false,
    }
}

pub fn show(ui: &mut Ui, p: &mut Project, s: &mut UiState, meters: &Meters) {
    if std::mem::take(&mut s.mixer.want_bounce) {
        crate::menus::apply("Bounce", p, s);
    }
    let area = ui.available_rect_before_wrap();
    ui.allocate_rect(area, Sense::hover());
    let bar = Rect::from_min_size(area.min, vec2(area.width(), TOOLBAR_H));
    toolbar(ui, p, s, bar);
    let v = &mut s.mixer;

    let body = Rect::from_min_max(pos2(area.left(), bar.bottom()), area.max);
    let labels = Rect::from_min_max(body.min, pos2(body.left() + LABEL_W, body.bottom()));
    let strips_area = Rect::from_min_max(pos2(labels.right(), body.top()), body.max);
    let g = ui.painter_at(body);
    g.rect_filled(body, 0.0, mixer_bg_col());

    // Which strips, in order.
    let filters = v.filters;
    let mut keys: Vec<StripKey> = if p.tracks.is_empty() {
        Vec::new()
    } else {
        match v.mode {
            MixerMode::Single => vec![StripKey::Track(p.selected_track.min(p.tracks.len() - 1))],
            MixerMode::Tracks | MixerMode::All => (0..p.tracks.len())
                .filter(|i| strip_visible(p.tracks[*i].kind, &filters))
                .map(StripKey::Track)
                .collect(),
        }
    };
    if filters[5] {
        keys.push(StripKey::StereoOut);
    }
    if v.signal_flow && !p.tracks.is_empty() {
        keys = signal_path(p, p.selected_track.min(p.tracks.len() - 1));
    }

    let strip_w = if v.wide { 96.0 } else { 80.0 };
    // Stereo Out stays at the mixer's right edge (its left, in Settings > Display), out of the
    // scroll, as the master strip of a console does. The signal-flow view keeps it in its place.
    let all_keys = keys.clone();
    let pinned = !v.signal_flow && keys.contains(&StripKey::StereoOut);
    if pinned {
        keys.retain(|k| *k != StripKey::StereoOut);
    }
    let on_left = crate::settings::master_on_left();
    let (strips_area, master_area) = if pinned {
        let w = strip_w + 10.0;
        if on_left {
            (
                Rect::from_min_max(pos2(strips_area.left() + w, strips_area.top()), strips_area.max),
                Some(Rect::from_min_max(strips_area.min, pos2(strips_area.left() + w, strips_area.bottom()))),
            )
        } else {
            (
                Rect::from_min_max(strips_area.min, pos2(strips_area.right() - w, strips_area.bottom())),
                Some(Rect::from_min_max(pos2(strips_area.right() - w, strips_area.top()), strips_area.max)),
            )
        }
    } else {
        (strips_area, None)
    };
    let total_w = keys.len() as f32 * (strip_w + GAP);
    let r = rows_fit(
        body.top(),
        body.bottom(),
        fx_rows_for(p, &all_keys),
        send_rows_for(p, &all_keys),
        v.fx_scroll,
    );
    let hover = crate::widgets::wheel_pointer(ui).filter(|pt| strips_area.contains(*pt));
    if let Some(pt) = hover {
        let d = ui.input(|i| i.smooth_scroll_delta);
        // The wheel over the insert slots goes through them, when some are out of sight.
        let over_slots = pt.y >= r.fx && pt.y < r.fx + r.fx_rows as f32 * 17.0;
        if over_slots && r.fx_total > r.fx_rows {
            let last = (r.fx_total - r.fx_rows) as f32;
            v.fx_scroll = (v.fx_scroll + d.y / 17.0).clamp(0.0, last);
        } else {
            v.scroll_x -= d.x + d.y;
        }
    }
    v.scroll_x = v
        .scroll_x
        .clamp(0.0, (total_w - strips_area.width() + 8.0).max(0.0));

    // Row labels.
    let gl = ui.painter_at(labels);
    let lx = labels.right() - 6.0;
    let label_at = |y: f32, h: f32, t: &str| {
        gl.text(
            pos2(lx, y + h / 2.0),
            Align2::RIGHT_CENTER,
            t,
            FontId::proportional(10.5),
            text_dim_col(),
        );
    };
    label_at(r.setting, 16.0, tr("Setting"));
    label_at(r.gain_red, 8.0, tr("Gain Reduction"));
    label_at(r.midi_fx, 16.0, "MIDI FX");
    label_at(r.input, 16.0, tr("Input"));
    label_at(r.fx, 15.0, tr("Audio FX"));
    if r.fx_total > r.fx_rows {
        let last = r.fx_first + r.fx_rows;
        let range = format!("{}-{last} of {}", r.fx_first + 1, r.fx_total);
        label_at(r.fx + 17.0, 15.0, &range);
    }
    label_at(r.sends, 16.0, tr("Sends"));
    label_at(r.output, 16.0, tr("Output"));
    label_at(r.group, 16.0, tr("Group"));
    label_at(r.automation, 16.0, tr("Automation"));
    label_at(r.pan, 24.0, tr("Pan"));
    label_at(r.db, 14.0, "dB");

    let gs = ui.painter_at(strips_area);
    for (n, key) in keys.iter().enumerate() {
        let x = strips_area.left() + 4.0 + n as f32 * (strip_w + GAP) - v.scroll_x;
        let strip = Rect::from_min_max(
            pos2(x, body.top() + 2.0),
            pos2(x + strip_w, body.bottom() - 2.0),
        );
        if strip.right() < strips_area.left() || strip.left() > strips_area.right() {
            continue;
        }
        channel_strip(
            ui,
            &gs,
            strip,
            *key,
            p,
            v,
            meters,
            &r,
            "mixer",
            NameStyle::Coloured,
        );
    }

    if let Some(area) = master_area {
        let gm = ui.painter_at(area);
        gm.rect_filled(area, 0.0, mixer_bg_col());
        let edge = if on_left { area.right() - 1.0 } else { area.left() + 1.0 };
        gm.line_segment(
            [pos2(edge, area.top()), pos2(edge, area.bottom())],
            Stroke::new(1.0, crate::theme::border()),
        );
        let x = area.left() + 5.0;
        let strip = Rect::from_min_max(pos2(x, body.top() + 2.0), pos2(x + strip_w, body.bottom() - 2.0));
        channel_strip(
            ui,
            &gm,
            strip,
            StripKey::StereoOut,
            p,
            v,
            meters,
            &r,
            "mixer",
            NameStyle::Coloured,
        );
    }

    // A strip dragged by its name, while there is an order to change.
    let reorders = v.mode != MixerMode::Single && !v.signal_flow;
    let drag = egui::DragAndDrop::payload::<StripDrag>(ui.ctx());
    if let (Some(drag), true) = (drag, reorders) {
        let first = strips_area.left() + 4.0 - v.scroll_x;
        let step = strip_w + GAP;
        if let Some(at) = ui.input(|i| i.pointer.latest_pos()) {
            let to = strip_drop_target(&keys, p.tracks.len(), first, step, at.x);
            let place = keys
                .iter()
                .position(|k| *k == StripKey::Track(to))
                .unwrap_or_else(|| {
                    keys.iter()
                        .filter(|k| matches!(k, StripKey::Track(_)))
                        .count()
                });
            let line = first + place as f32 * step - GAP * 0.5;
            gs.line_segment(
                [pos2(line, body.top()), pos2(line, body.bottom())],
                Stroke::new(2.0, crate::theme::accent()),
            );
            if ui.input(|i| i.pointer.any_released()) {
                egui::DragAndDrop::clear_payload(ui.ctx());
                if p.move_tracks(&[drag.0], to) {
                    p.linked_strips.clear();
                    v.peak_hold.clear();
                    // The lights are by place, which has changed.
                    v.clip_resets.push(ClipReset::All);
                    crate::tracks_area::forget_track_indexes(s);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::PointerButton;
    use ovrtone_engine::FxKind;

    /// A project with an audio-style track and an Aux track, and its mixer drawn in a window
    /// 1400 by 700 points.
    #[test]
    fn the_signal_path_follows_sends_and_outputs_through_auxes() {
        let mut p = Project::empty();
        let source = p.add_track(TrackKind::Audio);
        let other = p.add_track(TrackKind::Audio);
        let first = p.add_track(TrackKind::Aux);
        let second = p.add_track(TrackKind::Aux);
        let (bus1, bus2) = (p.aux_bus_of(first), p.aux_bus_of(second));
        p.tracks[source].output = bus1;
        p.tracks[first].output = bus2;
        let path = signal_path(&p, source);
        assert_eq!(
            path,
            [
                StripKey::Track(source),
                StripKey::Track(first),
                StripKey::Track(second),
                StripKey::StereoOut,
            ]
        );
        assert!(!path.contains(&StripKey::Track(other)));
        // A loop ends, and nothing in it reaches Stereo Out.
        p.tracks[second].output = bus1;
        assert_eq!(
            signal_path(&p, source),
            [
                StripKey::Track(source),
                StripKey::Track(first),
                StripKey::Track(second),
            ]
        );
        // A bus nothing listens to is the end of the sound, and a send also keeps the way out.
        p.tracks[second].output = 0;
        p.tracks[source].output = 900;
        assert_eq!(signal_path(&p, source), [StripKey::Track(source)]);
        p.tracks[source].output = 0;
        p.tracks[source].sends.push(crate::model::SendSlot {
            bus: Some(bus2),
            level: 0.5,
            pre: false,
        });
        assert_eq!(
            signal_path(&p, source),
            [
                StripKey::Track(source),
                StripKey::Track(second),
                StripKey::StereoOut,
            ]
        );
    }

    #[test]
    fn strips_selected_together_move_together() {
        let mut p = Project::empty();
        let a = p.add_track(TrackKind::Audio);
        let b = p.add_track(TrackKind::Audio);
        let c = p.add_track(TrackKind::Instrument);
        assert!(p.group_mates(a, |g| g.volume).is_empty());
        p.linked_strips = vec![a, b];
        assert_eq!(p.group_mates(a, |g| g.volume), [b]);
        assert!(p.group_mates(c, |g| g.volume).is_empty());
    }

    #[test]
    fn a_mastering_track_takes_the_whole_mix_and_is_made_once() {
        let mut p = Project::empty();
        let a = p.add_track(TrackKind::Audio);
        let sub = p.add_track(TrackKind::Aux);
        let bus = p.aux_bus_of(sub);
        let b = p.add_track(TrackKind::Instrument);
        p.tracks[b].output = bus;
        let Some(m) = p.create_mastering_track() else {
            panic!("a bus is free");
        };
        let master_bus = p.aux_bus_of(m);
        assert_eq!(p.tracks[a].output, master_bus);
        assert_eq!(p.tracks[sub].output, master_bus);
        assert_eq!(p.tracks[b].output, bus, "what went to a bus still does");
        assert_eq!(p.tracks[m].output, 0, "it goes to Stereo Out");
        assert_eq!(p.create_mastering_track(), Some(m));
        assert_eq!(
            p.tracks.iter().filter(|t| t.kind == TrackKind::Aux).count(),
            2
        );
    }

    #[test]
    fn tracks_come_from_another_project_with_their_regions_and_no_routing() {
        let mut other = Project::empty();
        let a = other.add_track(TrackKind::Instrument);
        other.add_midi_region(a, 2.0);
        other.tracks[a].output = 3;
        other.tracks[a].group = Some(0);
        let skip = other.add_track(TrackKind::Audio);
        other.tracks[skip].name = "Not this".to_string();
        let mut picks = vec![false; other.tracks.len()];
        picks[a] = true;
        let mut p = Project::empty();
        let before = p.tracks.len();
        assert_eq!(p.import_tracks(&other, &picks), 1);
        let got = &p.tracks[before];
        assert_eq!(got.regions.len(), 1);
        assert_eq!((got.output, got.group), (0, None));
        assert!(p.tracks.iter().all(|t| t.name != "Not this"));
    }

    fn mixer_project() -> Project {
        let mut p = Project::empty();
        p.add_track(TrackKind::Aux);
        p.selected_track = 0;
        p
    }

    /// Where the fader cap of the first strip sits at `db`, and the rows of the mixer.
    fn fader_at(p: &Project, db: f32) -> egui::Pos2 {
        let keys: Vec<StripKey> = (0..p.tracks.len())
            .map(StripKey::Track)
            .chain([StripKey::StereoOut])
            .collect();
        let r = rows(TOOLBAR_H, fx_rows_for(p, &keys), send_rows_for(p, &keys));
        let strip_bottom = 700.0 - 2.0;
        let top = r.fader_top + 8.0;
        // As `fader_area` and `fader_and_meter` have it.
        let bottom = strip_bottom - 44.0 - 22.0 - 6.0;
        let sx = LABEL_W + 4.0 + 4.0;
        pos2(sx + 22.0, bottom - db_to_frac(db) * (bottom - top))
    }

    fn run(h: &mut crate::ui_test::Harness, p: &mut Project, s: &mut UiState, n: usize) {
        for _ in 0..n {
            h.frame(vec![], |ui| show(ui, p, s, &Meters::default()));
        }
    }

    #[test]
    fn the_fader_moves_the_level_unless_the_strip_plays_its_automation() {
        let mut h = crate::ui_test::Harness::new(1400.0, 700.0);
        let mut s = UiState::new();
        let mut p = mixer_project();
        run(&mut h, &mut p, &mut s, 2);
        let cap = fader_at(&p, 0.0);
        let up = cap - vec2(0.0, 40.0);
        let frame = |h: &mut crate::ui_test::Harness, p: &mut Project, s: &mut UiState| {
            h.drag(
                &[cap, cap - vec2(0.0, 20.0), up],
                PointerButton::Primary,
                |ui| show(ui, p, s, &Meters::default()),
            )
        };
        frame(&mut h, &mut p, &mut s);
        assert!(p.tracks[0].volume_db > 0.5, "{}", p.tracks[0].volume_db);

        // Read mode with a lane: the fader shows the lane and cannot be moved.
        p.tracks[0].volume_db = -6.0;
        p.tracks[0].auto.mode = AutoMode::Read;
        p.tracks[0].auto.set_point(AutoTarget::Volume, 0.0, -6.0);
        let cap = fader_at(&p, -6.0);
        h.drag(
            &[cap, cap - vec2(0.0, 40.0)],
            PointerButton::Primary,
            |ui| show(ui, &mut p, &mut s, &Meters::default()),
        );
        assert_eq!(p.tracks[0].volume_db, -6.0);

        // Touch: the same drag holds the control for its length and lets go after.
        p.tracks[0].auto.mode = AutoMode::Touch;
        h.press(cap, PointerButton::Primary, |ui| {
            show(ui, &mut p, &mut s, &Meters::default())
        });
        h.frame(
            vec![egui::Event::PointerMoved(cap - vec2(0.0, 30.0))],
            |ui| show(ui, &mut p, &mut s, &Meters::default()),
        );
        assert_eq!(p.tracks[0].auto.held, Some(AutoTarget::Volume));
        assert!(p.tracks[0].volume_db > -6.0);
        h.release(cap - vec2(0.0, 30.0), PointerButton::Primary, |ui| {
            show(ui, &mut p, &mut s, &Meters::default())
        });
        run(&mut h, &mut p, &mut s, 2);
        assert_eq!(p.tracks[0].auto.held, None);
    }

    #[test]
    fn sends_on_faders_set_the_send_to_the_bus_and_a_fader_at_the_bottom_takes_it_away() {
        let mut h = crate::ui_test::Harness::new(1400.0, 700.0);
        let mut s = UiState::new();
        let mut p = mixer_project();
        s.mixer.sends_on_faders = Some(2);
        run(&mut h, &mut p, &mut s, 2);
        let bottom = fader_at(&p, -60.0);
        let top = fader_at(&p, 0.0);
        h.drag(
            &[bottom, bottom - vec2(0.0, 20.0), top],
            PointerButton::Primary,
            |ui| show(ui, &mut p, &mut s, &Meters::default()),
        );
        let sends = &p.tracks[0].sends;
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].bus, Some(2));
        assert!(sends[0].level > 0.3, "{}", sends[0].level);
        // The track's own level was not touched.
        assert_eq!(p.tracks[0].volume_db, 0.0);
        // The conversions agree both ways, and the bottom of the scale removes the send.
        let mut list = vec![];
        set_send_level(&mut list, 3, 0.0);
        assert!((ovrtone_engine::send_gain(list[0].level) - 1.0).abs() < 0.01);
        set_send_level(&mut list, 3, -60.0);
        assert_eq!(list[0].level, 0.0);
        set_send_level(&mut list, 4, -60.0);
        assert_eq!(list.len(), 1, "nothing to add for a silent send");
    }

    #[test]
    fn sends_on_faders_work_on_an_aux_but_not_into_its_own_bus() {
        let mut h = crate::ui_test::Harness::new(1400.0, 700.0);
        let mut s = UiState::new();
        let mut p = Project::empty();
        p.tracks.clear();
        let aux = p.add_track(TrackKind::Aux);
        let own = p.aux_bus_of(aux);
        s.mixer.sends_on_faders = Some(own + 1);
        run(&mut h, &mut p, &mut s, 2);
        let (bottom, top) = (fader_at(&p, -60.0), fader_at(&p, 0.0));
        let drag = |h: &mut crate::ui_test::Harness, p: &mut Project, s: &mut UiState| {
            h.drag(
                &[bottom, bottom - vec2(0.0, 20.0), top],
                PointerButton::Primary,
                |ui| show(ui, p, s, &Meters::default()),
            );
        };
        drag(&mut h, &mut p, &mut s);
        assert_eq!(p.tracks[aux].sends.len(), 1, "it sends to the next bus");
        assert_eq!(p.tracks[aux].sends[0].bus, Some(own + 1));
        // On its own bus the fader is the level again, and no send appears.
        p.tracks[aux].sends.clear();
        s.mixer.sends_on_faders = Some(own);
        run(&mut h, &mut p, &mut s, 2);
        drag(&mut h, &mut p, &mut s);
        assert!(p.tracks[aux].sends.is_empty());
    }

    #[test]
    fn the_fader_keeps_its_room_and_the_slots_end_at_a_plus() {
        let mut p = mixer_project();
        let keys = [StripKey::Track(0)];
        p.tracks[0].inserts.clear();
        p.tracks[0].plugins.clear();
        assert_eq!(fx_rows_for(&p, &keys), 1, "only the plus");
        // A short mixer: however many slots, the fader keeps its room.
        let (top, bottom) = (0.0, 450.0);
        for total in [1, 3, 8, 32] {
            let r = rows_fit(top, bottom, total, 1, 0.0);
            assert!(
                bottom - BELOW_FADER - r.fader_top >= MIN_FADER_H - 17.0,
                "{total}: {}",
                r.fader_top
            );
            assert!(r.fx_first + r.fx_rows <= r.fx_total);
        }
        // The last slots, with the plus, are in sight; scrolled back, the first ones, and no further.
        let r = rows_fit(top, bottom, 32, 1, 0.0);
        assert_eq!(r.fx_first + r.fx_rows, 32);
        let r = rows_fit(top, bottom, 32, 1, 1000.0);
        assert_eq!(r.fx_first, 0);
        // A tall one shows them all.
        let r = rows_fit(top, 2000.0, 32, 1, 5.0);
        assert_eq!((r.fx_first, r.fx_rows), (0, 32));
    }

    #[test]
    fn a_plugin_dragged_to_another_slot_goes_there_with_its_instance() {
        use crate::model::plugin_rows;
        let info = |name: &str| ovrtone_host::PluginInfo {
            id: name.to_string(),
            name: name.to_string(),
            vendor: String::new(),
            version: String::new(),
            format: ovrtone_host::PluginFormat::Clap,
            path: format!("{name}.clap").into(),
            is_instrument: false,
        };
        let mut p = mixer_project();
        p.tracks[0].inserts.clear();
        p.tracks[1].inserts.clear();
        for (row, name) in [(0, "A"), (1, "B"), (2, "C")] {
            crate::plugin_rack::add_effect_in_row(&mut p, 0, &info(name), row).unwrap();
        }
        let uid = p.tracks[0].plugins[0].uid;
        let names = |p: &Project, t: usize| -> Vec<String> {
            p.tracks[t].plugins.iter().map(|x| x.name.clone()).collect()
        };
        // To an empty slot further down: it comes last.
        let from = SlotDrag {
            home: Home::Track(0),
            index: 0,
        };
        move_plugin(&mut p, from, Home::Track(0), 5);
        assert_eq!(names(&p, 0), ["B", "C", "A"]);
        assert_eq!(plugin_rows(0, &p.tracks[0].plugins), [1, 2, 5]);
        assert_eq!(p.tracks[0].plugins[2].uid, uid, "the same instance");
        // Onto another plugin: it goes before it, and that one moves down.
        let from = SlotDrag {
            home: Home::Track(0),
            index: 2,
        };
        move_plugin(&mut p, from, Home::Track(0), 1);
        assert_eq!(names(&p, 0), ["A", "B", "C"]);
        assert_eq!(plugin_rows(0, &p.tracks[0].plugins), [1, 2, 3]);
        // To another strip, and to the output.
        let from = SlotDrag {
            home: Home::Track(0),
            index: 0,
        };
        move_plugin(&mut p, from, Home::Track(1), 0);
        assert_eq!(names(&p, 0), ["B", "C"]);
        assert_eq!(names(&p, 1), ["A"]);
        let from = SlotDrag {
            home: Home::Track(1),
            index: 0,
        };
        move_plugin(&mut p, from, Home::Master, 2);
        assert_eq!(p.master_plugins[0].uid, uid);
        assert!(p.tracks[1].plugins.is_empty());
    }

    #[test]
    fn a_fader_moves_its_whole_group_by_the_same_amount() {
        let mut h = crate::ui_test::Harness::new(1400.0, 700.0);
        let mut s = UiState::new();
        let mut p = mixer_project();
        // Track 0 (the instrument) and a new second track are one group; the aux is not.
        let second = p.add_track(TrackKind::Instrument);
        let g = p.add_group("Keys").unwrap_or(0);
        p.assign_group(0, Some(g));
        p.assign_group(second, Some(g));
        p.tracks[second].volume_db = -10.0;
        run(&mut h, &mut p, &mut s, 2);
        let cap = fader_at(&p, 0.0);
        let up = cap - vec2(0.0, 30.0);
        h.drag(
            &[cap, cap - vec2(0.0, 15.0), up],
            PointerButton::Primary,
            |ui| show(ui, &mut p, &mut s, &Meters::default()),
        );
        let moved = p.tracks[0].volume_db;
        assert!(moved > 0.5, "{moved}");
        assert!(
            (p.tracks[second].volume_db - (-10.0 + moved)).abs() < 0.01,
            "{} vs {}",
            p.tracks[second].volume_db,
            moved
        );
        assert_eq!(
            p.tracks[1].volume_db, 0.0,
            "the Aux strip is not in the group"
        );
        // With Volume off in the group the others stay.
        p.groups[g].volume = false;
        let cap = fader_at(&p, moved);
        let before = p.tracks[second].volume_db;
        h.drag(
            &[cap, cap + vec2(0.0, 10.0), cap + vec2(0.0, 20.0)],
            PointerButton::Primary,
            |ui| show(ui, &mut p, &mut s, &Meters::default()),
        );
        assert!(p.tracks[0].volume_db < moved);
        assert_eq!(p.tracks[second].volume_db, before);
    }

    #[test]
    fn the_slot_menus_add_replace_move_bypass_and_remove_effects() {
        let mut p = mixer_project();
        let mut v = MixerView::default();
        for kind in [FxKind::ChannelEq, FxKind::Compressor, FxKind::Reverb] {
            apply_insert(&mut p, &mut v, 0, 99, InsertAct::Add(kind));
        }
        let kinds = |p: &Project| {
            p.tracks[0]
                .inserts
                .iter()
                .map(|x| x.kind)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            kinds(&p),
            [FxKind::ChannelEq, FxKind::Compressor, FxKind::Reverb]
        );
        assert_eq!(v.fx_window, Some((0, 2)), "a new effect opens its window");
        apply_insert(&mut p, &mut v, 0, 2, InsertAct::Move(-1));
        assert_eq!(
            kinds(&p),
            [FxKind::ChannelEq, FxKind::Reverb, FxKind::Compressor]
        );
        assert_eq!(v.fx_window, Some((0, 1)), "the window follows its effect");
        apply_insert(&mut p, &mut v, 0, 0, InsertAct::Move(-1));
        apply_insert(&mut p, &mut v, 0, 2, InsertAct::Move(1));
        assert_eq!(kinds(&p).len(), 3, "moving past an end does nothing");
        apply_insert(&mut p, &mut v, 0, 1, InsertAct::Bypass);
        assert!(p.tracks[0].inserts[1].bypass);
        apply_insert(&mut p, &mut v, 0, 1, InsertAct::Replace(FxKind::Delay));
        assert_eq!(p.tracks[0].inserts[1], Insert::new(FxKind::Delay));
        apply_insert(&mut p, &mut v, 0, 0, InsertAct::Remove);
        assert_eq!(
            v.fx_window,
            Some((0, 0)),
            "the window moved up with the others"
        );
        apply_insert(&mut p, &mut v, 0, 0, InsertAct::Remove);
        assert_eq!(v.fx_window, None, "its effect is gone");
        apply_insert(&mut p, &mut v, 0, 5, InsertAct::Remove);
        apply_insert(&mut p, &mut v, 9, 0, InsertAct::Open);
        for _ in 0..ovrtone_engine::MAX_INSERTS + 4 {
            apply_insert(&mut p, &mut v, 0, 0, InsertAct::Add(FxKind::Chorus));
        }
        assert_eq!(p.tracks[0].inserts.len(), ovrtone_engine::MAX_INSERTS);
    }

    #[test]
    fn rows_make_room_for_every_send_and_the_strip_draws_with_all_of_it() {
        let mut p = mixer_project();
        p.tracks[0].sends = vec![
            SendSlot {
                bus: Some(1),
                level: 0.5,
                pre: true,
            },
            SendSlot {
                bus: Some(2),
                level: 0.7,
                pre: false,
            },
        ];
        p.tracks[0].inserts = vec![
            Insert::new(FxKind::ChannelEq),
            Insert::new(FxKind::Compressor),
        ];
        p.tracks[0].auto.mode = AutoMode::Latch;
        p.tracks[0].auto.set_point(AutoTarget::Pan, 0.0, 0.5);
        let keys = [StripKey::Track(0), StripKey::Track(1)];
        assert_eq!(send_rows_for(&p, &keys), 3);
        assert_eq!(send_rows_for(&Project::empty(), &[StripKey::Track(0)]), 1);
        let r = rows(0.0, 3, 3);
        assert!(r.output - r.sends >= 3.0 * 17.0);
        let mut h = crate::ui_test::Harness::new(1400.0, 700.0);
        let mut s = UiState::new();
        let meters = Meters {
            tracks: vec![(0.5, 0.4), (0.0, 0.0)],
            reduction: vec![6.0, 0.0],
            master: (0.3, 0.3),
            ..Meters::default()
        };
        for _ in 0..3 {
            h.frame(vec![], |ui| show(ui, &mut p, &mut s, &meters));
        }
    }

    #[test]
    fn toolbar_groups_never_overlap_and_give_way_in_order() {
        // Wide panel: modes centred, every filter a button, Sends on Faders shown.
        let f = fit_toolbar(200.0, 1200.0, 2300.0, 520.0);
        assert!(f.filters_inline && f.show_sends);
        assert_eq!(f.modes_left, 1200.0 - MODE_W / 2.0 + 60.0);

        // Narrower: the modes move left of the filters, Sends on Faders goes first.
        let f = fit_toolbar(200.0, 700.0, 1100.0, 520.0);
        assert!(f.filters_inline && !f.show_sends);
        assert!(f.modes_left + MODE_W + TOOLBAR_GAP <= 1100.0 - 520.0);
        assert!(f.modes_left >= 200.0 + TOOLBAR_GAP);

        // Narrower still: the filters fold into one menu.
        let f = fit_toolbar(200.0, 500.0, 700.0, 520.0);
        assert!(!f.filters_inline);
        assert!(f.modes_left + MODE_W + TOOLBAR_GAP <= 700.0 - FILTER_MENU_W);
    }

    #[test]
    fn strips_follow_the_filter_buttons_and_folders_have_none() {
        let all = [true; 8];
        let mut only_aux = [false; 8];
        only_aux[2] = true;
        assert!(strip_visible(TrackKind::Aux, &all));
        assert!(strip_visible(TrackKind::Aux, &only_aux));
        assert!(!strip_visible(TrackKind::Audio, &only_aux));
        assert!(!strip_visible(TrackKind::Instrument, &only_aux));
        assert!(!strip_visible(TrackKind::Aux, &[false; 8]));
        assert!(
            !strip_visible(TrackKind::Folder, &all),
            "a folder has no channel strip"
        );
    }

    fn draw_slot(ui: &mut Ui, p: &mut Project, v: &mut MixerView) {
        let g = ui.painter().clone();
        insert_slot(ui, &g, SLOT, p, v, 0, 0, egui::Id::unique("slot"), false);
    }

    fn plugin_project() -> Project {
        let mut p = mixer_project();
        p.tracks[0].inserts.clear();
        p.tracks[0].plugins = vec![crate::model::PluginInsert::new(&ovrtone_host::PluginInfo {
            id: "x".into(),
            name: "Plug".into(),
            vendor: "Maker".into(),
            version: "1".into(),
            format: ovrtone_host::PluginFormat::Clap,
            path: "x.clap".into(),
            is_instrument: false,
        })];
        p
    }

    fn effect_project() -> Project {
        let mut p = mixer_project();
        p.tracks[0].plugins.clear();
        p.tracks[0].inserts = vec![Insert::new(FxKind::Chorus)];
        p
    }

    const SLOT: Rect = Rect {
        min: egui::Pos2 { x: 50.0, y: 40.0 },
        max: egui::Pos2 { x: 170.0, y: 55.0 },
    };

    #[test]
    fn a_click_on_a_slot_opens_what_is_in_it() {
        let mut h = crate::ui_test::Harness::new(500.0, 500.0);
        let mut p = plugin_project();
        let mut v = MixerView::default();
        let uid = p.tracks[0].plugins[0].uid;
        h.settle(|ui| draw_slot(ui, &mut p, &mut v));
        h.click(SLOT.left_center() + vec2(20.0, 0.0), |ui| {
            draw_slot(ui, &mut p, &mut v)
        });
        assert_eq!(v.open_plugin.map(|o| o.0), Some(uid));

        let mut h = crate::ui_test::Harness::new(500.0, 500.0);
        let mut p = effect_project();
        let mut v = MixerView::default();
        h.settle(|ui| draw_slot(ui, &mut p, &mut v));
        h.click(SLOT.left_center() + vec2(20.0, 0.0), |ui| {
            draw_slot(ui, &mut p, &mut v)
        });
        assert_eq!(v.fx_window, Some((0, 0)));
    }

    #[test]
    fn the_buttons_on_a_slot_bypass_and_remove_without_opening_it() {
        for plugin in [true, false] {
            let mut h = crate::ui_test::Harness::new(500.0, 500.0);
            let mut p = if plugin {
                plugin_project()
            } else {
                effect_project()
            };
            let mut v = MixerView::default();
            let bypassed = |p: &Project| {
                if plugin {
                    p.tracks[0].plugins[0].bypass
                } else {
                    p.tracks[0].inserts[0].bypass
                }
            };
            h.settle(|ui| draw_slot(ui, &mut p, &mut v));
            let bypass = slot_button_rect(SLOT, 1).center();
            h.click(bypass, |ui| draw_slot(ui, &mut p, &mut v));
            assert!(bypassed(&p), "bypassed, plugin {plugin}");
            h.settle(|ui| draw_slot(ui, &mut p, &mut v));
            h.click(bypass, |ui| draw_slot(ui, &mut p, &mut v));
            assert!(!bypassed(&p), "and back");
            assert!(
                v.open_plugin.is_none() && v.fx_window.is_none(),
                "not opened"
            );

            h.settle(|ui| draw_slot(ui, &mut p, &mut v));
            let remove = slot_button_rect(SLOT, 0).center();
            h.click(remove, |ui| draw_slot(ui, &mut p, &mut v));
            assert!(p.tracks[0].plugins.is_empty() && p.tracks[0].inserts.is_empty());
            assert!(
                v.open_plugin.is_none() && v.fx_window.is_none(),
                "not opened"
            );
        }
    }

    #[test]
    fn the_right_button_still_has_the_menu_and_delete_removes_what_is_under_the_pointer() {
        // Remove in the menu, wherever the fonts put it.
        let mut found = false;
        let mut p = effect_project();
        let mut h = crate::ui_test::Harness::new(500.0, 500.0);
        let mut v = MixerView::default();
        for offset in (6..240).step_by(6) {
            h.settle(|ui| draw_slot(ui, &mut p, &mut v));
            // Duplicate Effect is above Remove in the menu: what counts is one going.
            let before = p.tracks[0].inserts.len();
            let centre = SLOT.center();
            h.right_click(centre, |ui| draw_slot(ui, &mut p, &mut v));
            h.settle(|ui| draw_slot(ui, &mut p, &mut v));
            h.click(pos2(centre.x + 10.0, SLOT.bottom() + offset as f32), |ui| {
                draw_slot(ui, &mut p, &mut v)
            });
            if p.tracks[0].inserts.len() < before {
                found = true;
                break;
            }
        }
        assert!(found, "Remove is in the right button's menu");

        let mut p = plugin_project();
        let mut h = crate::ui_test::Harness::new(500.0, 500.0);
        let mut v = MixerView::default();
        h.settle(|ui| draw_slot(ui, &mut p, &mut v));
        h.move_to(SLOT.center(), |ui| draw_slot(ui, &mut p, &mut v));
        h.key(egui::Key::Delete, |ui| draw_slot(ui, &mut p, &mut v));
        assert!(p.tracks[0].plugins.is_empty());
    }
}

/// A saved mixer layout: its name, the mode, the strip types shown and wide strips.
type SavedLayout = (String, MixerMode, [bool; 8], bool);

fn layouts_file() -> Option<std::path::PathBuf> {
    crate::recent_files::config_dir().map(|d| d.join("mixer-layouts"))
}

/// The saved mixer layouts, one a line: name, mode, the eight filters as 0 and 1, wide.
fn load_layouts() -> Vec<SavedLayout> {
    let text = layouts_file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .unwrap_or_default();
    text.lines()
        .filter_map(|line| {
            let mut parts = line.rsplitn(4, '\t');
            let wide = parts.next()? == "1";
            let bits = parts.next()?;
            let mode = match parts.next()? {
                "single" => MixerMode::Single,
                "all" => MixerMode::All,
                _ => MixerMode::Tracks,
            };
            let name = parts.next()?.to_string();
            let mut filters = [true; 8];
            for (f, c) in filters.iter_mut().zip(bits.chars()) {
                *f = c == '1';
            }
            Some((name, mode, filters, wide))
        })
        .collect()
}

fn write_layouts(layouts: &[SavedLayout]) {
    let Some(file) = layouts_file() else {
        return;
    };
    let text: String = layouts
        .iter()
        .map(|(name, mode, filters, wide)| {
            let mode = match mode {
                MixerMode::Single => "single",
                MixerMode::Tracks => "tracks",
                MixerMode::All => "all",
            };
            let bits: String = filters.iter().map(|f| if *f { '1' } else { '0' }).collect();
            let name = name.replace('\t', " ");
            format!("{name}\t{mode}\t{bits}\t{}\n", if *wide { "1" } else { "0" })
        })
        .collect();
    let saved = file
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&file, text));
    if let Err(e) = saved {
        tracing::warn!("could not save the mixer layouts: {e}");
    }
}

/// Keeps the mixer's layout as it is now, as "Layout n".
fn save_layout(v: &MixerView) {
    let mut layouts = load_layouts();
    let name = format!("Layout {}", layouts.len() + 1);
    layouts.push((name, v.mode, v.filters, v.wide));
    write_layouts(&layouts);
}
