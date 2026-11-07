//! Automation (Logic: the Automation menu of the channel strip and the automation lanes).
//!
//! The fader, the pan knob and every parameter of the effects of a strip can be written while
//! the project plays (Touch, Latch, Write) and are played back by the engine (Read). This
//! module records them ([`tick`] and the helpers the Mixer and the effect windows call when a
//! control is moved) and has the Automation editor window, where the points of a lane are
//! added, moved and removed and the stretches between them are bent.
use crate::icons::{button, Icon};
use crate::i18n::{tr, trf};
use crate::mixer::{db_to_frac, frac_to_db};
use crate::model::{AutoMode, AutoPoint, AutoTarget, Automation, Project, Track};
use crate::UiState;
use egui::{pos2, vec2, Align2, Color32, FontId, Rect, Sense, Shape, Stroke, StrokeKind, Ui};
use ovrtone_engine::curve_shape;

/// What is being dragged in the editor.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Drag {
    /// A point.
    Point(usize),
    /// The handle in the middle of the stretch that starts at a point (it bends it).
    Curve(usize),
}

/// The editor window's state.
#[derive(Default)]
pub struct AutoEditor {
    /// The track whose lanes are shown, when the window is open.
    pub track: Option<usize>,
    /// The region whose own lanes are shown instead, when that is what is edited.
    pub region: Option<usize>,
    /// The first beat shown and how many beats are shown (0: not set yet).
    pub from: f64,
    pub span: f64,
    drag: Option<Drag>,
}

/// The value of a control in its lane's units: dB, pan, or for an effect parameter its
/// position 0..=1 along its range.
fn current(t: &Track, target: AutoTarget) -> f32 {
    match target {
        AutoTarget::Volume => t.volume_db,
        AutoTarget::Pan => t.pan,
        AutoTarget::Fx { slot, param } => t
            .inserts
            .get(slot as usize)
            .and_then(|i| {
                let info = i.kind.params().get(param as usize)?;
                Some(info.to_norm(i.params[param as usize]))
            })
            .unwrap_or(0.0),
        // The plugin keeps it: the lane's first value stands in.
        AutoTarget::Plugin { .. } => t.auto.lane(target).first().map_or(0.5, |p| p.value),
    }
}

/// A lane value in the control's own units (an effect parameter's position 0..=1 becomes
/// its value in dB, Hz, ms...).
pub fn natural(t: &Track, target: AutoTarget, v: f32) -> f32 {
    match target {
        AutoTarget::Fx { slot, param } => t
            .inserts
            .get(slot as usize)
            .and_then(|i| i.kind.params().get(param as usize))
            .map_or(v, |info| info.from_norm(v)),
        _ => v,
    }
}

/// Once a frame, after the playhead was read from the engine: writes what the strips that
/// record (Write, and Touch or Latch with a control held) have now.
pub fn tick(p: &mut Project) {
    for i in 0..p.tracks.len() {
        let merged = merged_lanes(p, i);
        if !merged.is_empty() || !p.tracks[i].auto.region_played.is_empty() {
            p.tracks[i].auto.region_played = merged;
        }
    }
    let (beat, playing) = (p.position_beats, p.playing);
    for t in p.tracks.iter_mut() {
        if !playing {
            // Latch stops writing when playback stops; the next pass starts afresh.
            if t.auto.mode == AutoMode::Latch {
                t.auto.held = None;
            }
            t.auto.trim = None;
            t.auto.end_pass();
            continue;
        }
        let target = match t.auto.mode {
            AutoMode::Write => Some(t.auto.shown),
            AutoMode::Touch | AutoMode::Latch | AutoMode::Trim => t.auto.held,
            AutoMode::Off | AutoMode::Read => None,
        };
        // A plugin's moves are written as they come: see `plugin_moved`.
        if matches!(t.auto.held, Some(AutoTarget::Plugin { .. }))
            && matches!(t.auto.mode, AutoMode::Touch | AutoMode::Trim)
            && beat > t.auto.plugin_touch_until
        {
            t.auto.held = None;
            t.auto.end_pass();
        }
        if let Some(target) = target.filter(|t| !matches!(t, AutoTarget::Plugin { .. })) {
            // Trim: the curve as it was, moved by what the control has moved.
            if let Some((base, by)) = &t.auto.trim {
                let by = *by;
                if let Some(old) = ovrtone_engine::lane_value(base, beat) {
                    set_control(t, target, old + by);
                }
            }
            let value = current(t, target);
            t.auto.write(target, beat, value, false);
        }
    }
}

/// How long after a move in a plugin's window Touch keeps writing, in beats.
const PLUGIN_TOUCH_BEATS: f64 = 1.0;

/// The user moved a parameter in a plugin's window. While playing, and with the strip recording,
/// it goes in the parameter's lane. Whether Touch still holds is worked out in [`tick`].
pub fn plugin_moved(t: &mut Track, target: AutoTarget, value: f32, beat: f64, playing: bool) {
    if !playing {
        return;
    }
    match t.auto.mode {
        AutoMode::Off | AutoMode::Read => return,
        AutoMode::Touch | AutoMode::Latch | AutoMode::Trim => {
            t.auto.held = Some(target);
            t.auto.plugin_touch_until = beat + PLUGIN_TOUCH_BEATS;
        }
        AutoMode::Write => t.auto.shown = target,
    }
    t.auto.write(target, beat, value, false);
}

/// What the plugin says about the hand on its control: holding a still control keeps Touch on the
/// lane, and letting go gives the lane back at once rather than after the timer.
pub fn plugin_gesture(t: &mut Track, uid: u64, editing: bool, ended: bool, beat: f64) {
    let holds_it = matches!(t.auto.held, Some(AutoTarget::Plugin { uid: held, .. }) if held == uid);
    if !holds_it {
        return;
    }
    if editing {
        t.auto.plugin_touch_until = beat + PLUGIN_TOUCH_BEATS;
    } else if ended {
        t.auto.plugin_touch_until = f64::NEG_INFINITY;
    }
}

/// A lane of a region is this long before and after its stretch, so the track's value comes back.
const REGION_EDGE: f64 = 1e-3;

/// What track `i` plays for the controls that have lanes on its regions: the track's lane, with
/// the stretch of each region that has a lane for it replaced by that lane. Controls with no
/// region lanes are not in the list. Nothing is made for a track without any.
pub fn merged_lanes(p: &Project, i: usize) -> Vec<(AutoTarget, Vec<AutoPoint>)> {
    let Some(t) = p.tracks.get(i) else {
        return Vec::new();
    };
    if t.regions.iter().all(|r| r.auto.is_empty()) {
        return Vec::new();
    }
    let mut regions: Vec<&crate::model::Region> =
        t.regions.iter().filter(|r| !r.auto.is_empty()).collect();
    regions.sort_by(|a, b| a.start_bar.total_cmp(&b.start_bar));
    let mut out: Vec<(AutoTarget, Vec<AutoPoint>)> = Vec::new();
    for region in regions {
        let (from, to) = (p.region_start_beats(region), p.region_end_beats(region));
        for lane in region.auto.iter().filter(|l| !l.points.is_empty()) {
            let at = match out.iter().position(|l| l.0 == lane.target) {
                Some(at) => at,
                None => {
                    out.push((lane.target, t.auto.lane(lane.target).to_vec()));
                    out.len() - 1
                }
            };
            let merged = &mut out[at].1;
            // With no lane the control's own setting is what the strip goes back to. A plugin
            // keeps its own, so there it holds the region's last value.
            let own = match lane.target {
                AutoTarget::Plugin { .. } => None,
                target => Some(current(t, target)),
            };
            let before = ovrtone_engine::lane_value(merged, from).or(own);
            let after = ovrtone_engine::lane_value(merged, to).or(own);
            merged.retain(|q| q.beat < from - REGION_EDGE || q.beat > to + REGION_EDGE);
            let point = |beat: f64, value: f32| AutoPoint {
                beat,
                value,
                curve: 0.0,
            };
            if let (Some(v), true) = (before, from >= REGION_EDGE) {
                merged.push(point(from - REGION_EDGE, v));
            }
            merged.extend(
                lane.points
                    .iter()
                    .filter(|q| q.beat >= 0.0 && from + q.beat <= to)
                    .map(|q| AutoPoint {
                        beat: from + q.beat,
                        ..*q
                    }),
            );
            if let Some(v) = after {
                merged.push(point(to + REGION_EDGE, v));
            }
            merged.sort_by(|a, b| a.beat.total_cmp(&b.beat));
        }
    }
    out
}

/// The lane of `target` on a region, made if it had none.
fn region_lane_mut(
    lanes: &mut Vec<crate::model::RegionLane>,
    target: AutoTarget,
) -> &mut Vec<AutoPoint> {
    let at = match lanes.iter().position(|l| l.target == target) {
        Some(at) => at,
        None => {
            lanes.push(crate::model::RegionLane {
                target,
                points: Vec::new(),
            });
            lanes.len() - 1
        }
    };
    &mut lanes[at].points
}

/// A region is cut at `at` beats from its start: `lanes` keep what is before it and the lanes of
/// the part after it are returned. Both parts have a point at the cut, so the value carries on.
pub fn cut_lanes(lanes: &mut [crate::model::RegionLane], at: f64) -> Vec<crate::model::RegionLane> {
    let mut right = Vec::with_capacity(lanes.len());
    for lane in lanes.iter_mut() {
        let Some(value) = ovrtone_engine::lane_value(&lane.points, at) else {
            continue;
        };
        let flat = AutoPoint {
            beat: 0.0,
            value,
            curve: 0.0,
        };
        let mut after = vec![flat];
        after.extend(
            lane.points
                .iter()
                .filter(|q| q.beat > at)
                .map(|q| AutoPoint {
                    beat: q.beat - at,
                    ..*q
                }),
        );
        lane.points.retain(|q| q.beat < at);
        lane.points.push(AutoPoint { beat: at, ..flat });
        right.push(crate::model::RegionLane {
            target: lane.target,
            points: after,
        });
    }
    right
}

/// The lanes of a region whose start moved `moved` beats later (earlier when negative).
pub fn trimmed_lanes(
    lanes: &[crate::model::RegionLane],
    moved: f64,
) -> Vec<crate::model::RegionLane> {
    lanes
        .iter()
        .map(|lane| {
            let mut points: Vec<AutoPoint> = lane
                .points
                .iter()
                .filter(|q| q.beat >= moved)
                .map(|q| AutoPoint {
                    beat: q.beat - moved,
                    ..*q
                })
                .collect();
            if moved > 0.0 && points.first().is_none_or(|q| q.beat > 0.0) {
                if let Some(value) = ovrtone_engine::lane_value(&lane.points, moved) {
                    points.insert(
                        0,
                        AutoPoint {
                            beat: 0.0,
                            value,
                            curve: 0.0,
                        },
                    );
                }
            }
            crate::model::RegionLane {
                target: lane.target,
                points,
            }
        })
        .collect()
}

/// Lanes of the second region of a join, added to those of the first; it starts `shift` beats later.
pub fn join_lanes(
    lanes: &mut Vec<crate::model::RegionLane>,
    second: &[crate::model::RegionLane],
    shift: f64,
) {
    for lane in second {
        let points = region_lane_mut(lanes, lane.target);
        points.extend(lane.points.iter().map(|q| AutoPoint {
            beat: q.beat + shift,
            ..*q
        }));
        points.sort_by(|a, b| a.beat.total_cmp(&b.beat));
    }
}

/// Whether Convert Automation has something to do for region `r` of track `t`.
pub fn can_convert(p: &Project, t: usize, r: usize) -> bool {
    let Some(region) = p.tracks.get(t).and_then(|x| x.regions.get(r)) else {
        return false;
    };
    if !region.auto.is_empty() {
        return true;
    }
    let (from, to) = (p.region_start_beats(region), p.region_end_beats(region));
    p.tracks[t]
        .auto
        .lanes()
        .any(|(_, points)| points.iter().any(|q| q.beat >= from && q.beat <= to))
}

/// Mix > Convert Automation: the track's points over the region become the region's.
pub fn convert_to_region(p: &mut Project, t: usize, r: usize) {
    let Some(region) = p.tracks.get(t).and_then(|x| x.regions.get(r)) else {
        return;
    };
    let (from, to) = (p.region_start_beats(region), p.region_end_beats(region));
    let track = &mut p.tracks[t];
    let targets: Vec<AutoTarget> = track.auto.lanes().map(|l| l.0).collect();
    for target in targets {
        let lane = track.auto.lane_mut(target);
        let inside: Vec<AutoPoint> = lane
            .iter()
            .filter(|q| q.beat >= from && q.beat <= to)
            .map(|q| AutoPoint {
                beat: q.beat - from,
                ..*q
            })
            .collect();
        if inside.is_empty() {
            continue;
        }
        lane.retain(|q| q.beat < from || q.beat > to);
        *region_lane_mut(&mut track.regions[r].auto, target) = inside;
    }
    track.auto.prune();
    p.song_rev += 1;
}

/// Mix > Convert Automation: the region's lanes go onto the track's, replacing what was there
/// over the region's stretch.
pub fn convert_to_track(p: &mut Project, t: usize, r: usize) {
    let Some(region) = p.tracks.get(t).and_then(|x| x.regions.get(r)) else {
        return;
    };
    let (from, to) = (p.region_start_beats(region), p.region_end_beats(region));
    let track = &mut p.tracks[t];
    for lane in std::mem::take(&mut track.regions[r].auto) {
        let points = track.auto.lane_mut(lane.target);
        points.retain(|q| q.beat < from || q.beat > to);
        points.extend(lane.points.iter().map(|q| AutoPoint {
            beat: from + q.beat,
            ..*q
        }));
        points.sort_by(|a, b| a.beat.total_cmp(&b.beat));
    }
    p.song_rev += 1;
}

/// The region's own automation drawn over it in the Tracks area.
pub fn draw_region_lanes(g: &egui::Painter, r: Rect, region: &crate::model::Region, beat_px: f32) {
    for lane in &region.auto {
        let to_pos = |q: &AutoPoint| {
            pos2(
                r.left() + q.beat as f32 * beat_px,
                r.bottom() - frac_of(lane.target, q.value) * r.height(),
            )
        };
        let Some(first) = lane.points.first() else {
            continue;
        };
        let mut line = vec![pos2(r.left(), to_pos(first).y)];
        line.extend(lane.points.iter().map(to_pos));
        if let Some(last) = lane.points.last() {
            line.push(pos2(r.right(), to_pos(last).y));
        }
        g.add(Shape::line(
            line,
            Stroke::new(1.2, Color32::from_rgb(0xff, 0xe0, 0x66)),
        ));
        for q in &lane.points {
            g.circle_filled(to_pos(q), 2.2, Color32::from_rgb(0xff, 0xe0, 0x66));
        }
    }
}

/// Whether the user can move the control: not while the strip plays its lane in Read mode.
pub fn editable(t: &Track, target: AutoTarget) -> bool {
    let played =
        !t.auto.lane(target).is_empty() || t.auto.region_played.iter().any(|l| l.0 == target);
    !(t.auto.mode == AutoMode::Read && played)
}

/// The value the control shows, in its lane's units: the lane's at `beat` when the strip
/// plays it, else its own.
pub fn shown_value(t: &Track, target: AutoTarget, beat: f64) -> f32 {
    t.auto
        .played(target, beat)
        .unwrap_or_else(|| current(t, target))
}

/// The user moved a control of the strip to `value`: dB for the fader, -1..1 for the pan,
/// the parameter's own value for an effect. In Touch and Latch the control is now held: the
/// strip stops playing its lane and writes.
pub fn moved(t: &mut Track, target: AutoTarget, value: f32) {
    if t.auto.mode == AutoMode::Trim {
        if t.auto.held != Some(target) || t.auto.trim.is_none() {
            t.auto.trim = Some((t.auto.lane(target).to_vec(), 0.0));
        }
        let change = value - current(t, target);
        if let Some((_, by)) = &mut t.auto.trim {
            *by += change;
        }
    }
    if matches!(
        t.auto.mode,
        AutoMode::Touch | AutoMode::Latch | AutoMode::Trim
    ) && t.auto.held != Some(target)
    {
        t.auto.held = Some(target);
    }
    set_control(t, target, value);
}

/// Sets a control without any automation state changing.
fn set_control(t: &mut Track, target: AutoTarget, value: f32) {
    match target {
        AutoTarget::Volume => t.volume_db = value.clamp(-60.0, 6.0),
        AutoTarget::Pan => t.pan = value.clamp(-1.0, 1.0),
        AutoTarget::Fx { slot, param } => {
            if let Some(ins) = t.inserts.get_mut(slot as usize) {
                if let Some(info) = ins.kind.params().get(param as usize) {
                    if value.is_finite() {
                        ins.params[param as usize] = value.clamp(info.min, info.max);
                    }
                }
            }
        }
        AutoTarget::Plugin { .. } => {}
    }
}

/// The user let go of a control. Touch returns to the lane (after writing the last value);
/// Latch keeps writing until playback stops.
pub fn released(t: &mut Track, target: AutoTarget, beat: f64, playing: bool) {
    if matches!(t.auto.mode, AutoMode::Touch | AutoMode::Trim) && t.auto.held == Some(target) {
        if playing {
            let value = current(t, target);
            t.auto.write(target, beat, value, true);
        }
        t.auto.end_pass();
        t.auto.held = None;
        t.auto.trim = None;
    }
}

/// Changes the strip's automation mode. Leaving a mode that plays the lanes, the controls
/// keep the value the lanes had at `beat`, as in Logic.
pub fn set_mode(t: &mut Track, mode: AutoMode, beat: f64) {
    if t.auto.mode == mode {
        return;
    }
    let targets: Vec<AutoTarget> = t.auto.lanes().map(|(target, _)| target).collect();
    for target in targets {
        if let Some(v) = t.auto.played(target, beat) {
            let own = natural(t, target, v);
            set_control(t, target, own);
        }
    }
    t.auto.mode = mode;
    t.auto.held = None;
    t.auto.trim = None;
    t.auto.end_pass();
}

/// The name of a lane: "Volume", "Pan", or the effect and its parameter ("?" when the effect
/// is not there any more).
pub fn lane_label(t: &Track, target: AutoTarget) -> String {
    match target {
        AutoTarget::Volume => tr("Volume").to_string(),
        AutoTarget::Pan => tr("Pan").to_string(),
        AutoTarget::Fx { slot, param } => {
            match t
                .inserts
                .get(slot as usize)
                .and_then(|i| Some((i, i.kind.params().get(param as usize)?)))
            {
                Some((i, info)) => format!("{} · {}", i.name(), info.name),
                None => "?".to_string(),
            }
        }
        AutoTarget::Plugin { uid, param } => {
            let plugin = t.hosted().find(|i| i.uid == uid);
            match plugin.and_then(|i| Some((i, i.params.iter().find(|x| x.id == param)?))) {
                Some((i, info)) => format!("{} · {}", i.name, info.name),
                // Not loaded (yet): its lane still plays when it does.
                None => plugin.map_or("?".to_string(), |i| format!("{} · {param}", i.name)),
            }
        }
    }
}

/// The text of a lane value: "-3.2 dB", "L 40" / "C" / "R 40" for the pan, an effect
/// parameter in its own units.
pub fn value_text(t: &Track, target: AutoTarget, v: f32) -> String {
    match target {
        AutoTarget::Volume => format!("{v:.1} dB"),
        AutoTarget::Pan => {
            let n = (v * 64.0).round() as i32;
            match n {
                0 => "C".into(),
                n if n < 0 => format!("L {}", -n),
                n => format!("R {n}"),
            }
        }
        AutoTarget::Fx { slot, param } => {
            let Some(info) = t
                .inserts
                .get(slot as usize)
                .and_then(|i| i.kind.params().get(param as usize))
            else {
                return format!("{v:.2}");
            };
            let x = info.from_norm(v);
            if info.unit.is_empty() {
                if info.max <= 1.0 {
                    format!("{:.0} %", x * 100.0)
                } else {
                    format!("{x:.2}")
                }
            } else if x.abs() >= 100.0 {
                format!("{x:.0} {}", info.unit)
            } else {
                format!("{x:.1} {}", info.unit)
            }
        }
        AutoTarget::Plugin { .. } => format!("{:.0} %", v * 100.0),
    }
}

/// 0 (bottom) to 1 (top) for a lane value.
fn frac_of(target: AutoTarget, v: f32) -> f32 {
    match target {
        AutoTarget::Volume => db_to_frac(v),
        AutoTarget::Pan => (v.clamp(-1.0, 1.0) + 1.0) / 2.0,
        AutoTarget::Fx { .. } | AutoTarget::Plugin { .. } => v.clamp(0.0, 1.0),
    }
}

fn value_of(target: AutoTarget, f: f32) -> f32 {
    let (lo, hi) = Automation::range(target);
    match target {
        AutoTarget::Volume => frac_to_db(f).clamp(lo, hi),
        AutoTarget::Pan => (f.clamp(0.0, 1.0) * 2.0 - 1.0).clamp(lo, hi),
        AutoTarget::Fx { .. } | AutoTarget::Plugin { .. } => f.clamp(lo, hi),
    }
}

/// The bend that puts the value at the middle of a stretch at `f` of the way from the first
/// value to the second (the inverse of what [`curve_shape`] does at 0.5).
fn curve_from_fraction(f: f32) -> f32 {
    let f = f.clamp(0.02, 0.98);
    let ln_half = 0.5f32.ln();
    if (f - 0.5).abs() < 1e-3 {
        0.0
    } else if f < 0.5 {
        ((f.ln() / ln_half - 1.0) / 4.0).clamp(0.0, 1.0)
    } else {
        (-((1.0 - f).ln() / ln_half - 1.0) / 4.0).clamp(-1.0, 0.0)
    }
}

/// How far a click can be from a point and still be on it, in pixels.
const GRAB: f32 = 8.0;
/// The editor's drawing area height.
const CANVAS_H: f32 = 190.0;

/// Where a point is on `canvas` for the beats `from..from + span`.
fn point_pos(
    canvas: Rect,
    from: f64,
    span: f64,
    target: AutoTarget,
    beat: f64,
    value: f32,
) -> egui::Pos2 {
    pos2(
        canvas.left() + ((beat - from) / span) as f32 * canvas.width(),
        canvas.bottom() - frac_of(target, value) * canvas.height(),
    )
}

/// The beat and value under `pos`.
fn beat_value_at(
    canvas: Rect,
    from: f64,
    span: f64,
    target: AutoTarget,
    pos: egui::Pos2,
) -> (f64, f32) {
    let beat = from + ((pos.x - canvas.left()) / canvas.width()).clamp(0.0, 1.0) as f64 * span;
    let f = (canvas.bottom() - pos.y) / canvas.height();
    (beat, value_of(target, f))
}

/// The Automation editor: a window with the lanes of one track.
pub fn show(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    let Some(ti) = s.mixer.auto_editor.track else {
        return;
    };
    if ti >= p.tracks.len() {
        s.mixer.auto_editor.track = None;
        return;
    }
    let ed = &mut s.mixer.auto_editor;
    if ed.span <= 0.0 {
        let end = p
            .tracks
            .iter()
            .flat_map(|t| &t.regions)
            .map(|r| p.region_end_beats(r))
            .fold(16.0f64, f64::max);
        ed.from = 0.0;
        ed.span = end.max(8.0);
    }
    let mut open = true;
    let title = trf("Automation: {}", &[&p.tracks[ti].name]);
    let beat_now = p.position_beats;
    egui::Window::new(title)
        .id(egui::Id::unique("automation_editor"))
        .open(&mut open)
        .default_size([680.0, 330.0])
        .show(ctx, |ui| {
            // Track or region: a region's lanes are edited as if they were the track's, then
            // put back (below), so the canvas has one way of working.
            let picked = s.tracks.selected.filter(|x| x.0 == ti).map(|x| x.1);
            {
                let ed = &mut s.mixer.auto_editor;
                if ed.region.is_some() && picked.is_some() {
                    ed.region = picked;
                }
                if ed.region.is_some_and(|r| r >= p.tracks[ti].regions.len()) {
                    ed.region = None;
                }
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(ed.region.is_none(), "Track Automation")
                        .clicked()
                    {
                        ed.region = None;
                    }
                    let button = egui::Button::selectable(ed.region.is_some(), "Region Automation");
                    if ui.add_enabled(picked.is_some(), button).clicked() {
                        ed.region = picked;
                    }
                    if let Some(r) = ed.region {
                        ui.label(p.tracks[ti].regions[r].name.clone());
                    }
                });
            }
            let mut mode = p.tracks[ti].auto.mode;
            let mut target = p.tracks[ti].auto.shown;
            // A lane of an effect that is not there any more shows the fader's.
            if lane_label(&p.tracks[ti], target) == "?" {
                target = AutoTarget::Volume;
            }
            ui.horizontal(|ui| {
                ui.label(tr("Mode"));
                egui::ComboBox::from_id_salt("auto_mode")
                    .selected_text(tr(mode.label()))
                    .show_ui(ui, |ui| {
                        for m in AutoMode::ALL {
                            ui.selectable_value(&mut mode, m, tr(m.label()));
                        }
                    });
                ui.label(tr("Parameter"));
                let t = &p.tracks[ti];
                egui::ComboBox::from_id_salt("auto_param")
                    .selected_text(lane_label(t, target))
                    .width(190.0)
                    .show_ui(ui, |ui| {
                        for c in lane_choices(t) {
                            let dot = if t.auto.lane(c).is_empty() {
                                "  "
                            } else {
                                "• "
                            };
                            ui.selectable_value(
                                &mut target,
                                c,
                                format!("{dot}{}", lane_label(t, c)),
                            );
                        }
                    });
                ui.separator();
                let ed = &mut s.mixer.auto_editor;
                if button(ui, Icon::Minus, tr("Zoom Out")).clicked() {
                    ed.span = (ed.span * 2.0).min(4096.0);
                }
                if button(ui, Icon::Plus, tr("Zoom In")).clicked() {
                    ed.span = (ed.span / 2.0).max(1.0);
                }
                if button(ui, Icon::Earlier, "Earlier").clicked() {
                    ed.from = (ed.from - ed.span / 2.0).max(0.0);
                }
                if button(ui, Icon::Later, "Later").clicked() {
                    ed.from += ed.span / 2.0;
                }
                ui.separator();
                if ui.button(tr("Clear Lane")).clicked() {
                    match s.mixer.auto_editor.region {
                        Some(r) => p.tracks[ti].regions[r].auto.retain(|l| l.target != target),
                        None => {
                            p.tracks[ti].auto.lane_mut(target).clear();
                            p.tracks[ti].auto.prune();
                        }
                    }
                }
            });
            if mode != p.tracks[ti].auto.mode {
                set_mode(&mut p.tracks[ti], mode, beat_now);
            }
            p.tracks[ti].auto.shown = target;
            let region = s.mixer.auto_editor.region;
            let stretch = region.map(|r| {
                let x = &p.tracks[ti].regions[r];
                (p.region_start_beats(x), p.region_end_beats(x))
            });
            let track_points = region.zip(stretch).map(|(r, (from, _))| {
                let own: Vec<AutoPoint> = p.tracks[ti].regions[r]
                    .auto
                    .iter()
                    .find(|l| l.target == target)
                    .map_or_else(Vec::new, |l| {
                        l.points
                            .iter()
                            .map(|q| AutoPoint {
                                beat: from + q.beat,
                                ..*q
                            })
                            .collect()
                    });
                std::mem::replace(p.tracks[ti].auto.lane_mut(target), own)
            });
            canvas(ui, p, &mut s.mixer.auto_editor, ti, target, stretch);
            let mut n = p.tracks[ti].auto.lane(target).len();
            if let (Some(r), Some((from, to)), Some(saved)) = (region, stretch, track_points) {
                let edited = std::mem::replace(p.tracks[ti].auto.lane_mut(target), saved);
                p.tracks[ti].auto.prune();
                let mut own: Vec<AutoPoint> = edited
                    .into_iter()
                    .map(|q| AutoPoint {
                        beat: q.beat.clamp(from, to) - from,
                        ..q
                    })
                    .collect();
                own.dedup_by(|a, b| a.beat == b.beat);
                n = own.len();
                let lanes = &mut p.tracks[ti].regions[r].auto;
                if own.is_empty() {
                    lanes.retain(|l| l.target != target);
                } else {
                    *region_lane_mut(lanes, target) = own;
                }
            }
            ui.label(trf("{} points", &[&n]));
        });
    if !open {
        s.mixer.auto_editor.track = None;
        s.mixer.auto_editor.region = None;
        s.mixer.auto_editor.drag = None;
    }
}

/// Every control of track `t` that can have a lane.
pub fn lane_choices(t: &Track) -> Vec<AutoTarget> {
    let mut choices = vec![AutoTarget::Volume, AutoTarget::Pan];
    for (slot, ins) in t.inserts.iter().enumerate() {
        for k in 0..ins.kind.params().len() {
            choices.push(AutoTarget::Fx {
                slot: slot as u8,
                param: k as u8,
            });
        }
    }
    for plugin in t.hosted() {
        for param in plugin.params.iter() {
            choices.push(AutoTarget::Plugin {
                uid: plugin.uid,
                param: param.id,
            });
        }
    }
    choices
}

/// The points picked on a track's lane in the Tracks area.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct LanePick {
    pub track: usize,
    pub target: AutoTarget,
    pub points: Vec<usize>,
}

/// A drag on a track's lane in the Tracks area.
#[derive(Clone, Debug, PartialEq)]
pub enum LaneDrag {
    /// The picked points: the one grabbed, the lane as it was, and where the pointer began.
    Points {
        grabbed: usize,
        from: Vec<AutoPoint>,
        at: (f64, f32),
    },
    /// The handle that bends the stretch after a point.
    Curve(usize),
    /// A box that picks what it covers: where it began, in beats and 0..=1 up the lane.
    Box(f64, f32),
}

/// Points of a lane never get closer than this, in beats, when a group is dragged.
const POINT_GAP: f64 = 1e-3;

/// The lane `from` with the `picked` points moved together by `by_beats` and `by_frac` (of the
/// lane's height). A group keeps its shape: it stops at the first point it would pass, at the
/// start of the song, and when its highest or lowest point reaches the edge of the lane.
pub fn moved_group(
    from: &[AutoPoint],
    picked: &[usize],
    target: AutoTarget,
    by_beats: f64,
    by_frac: f32,
) -> Vec<AutoPoint> {
    let (mut lo, mut hi) = (f64::NEG_INFINITY, f64::INFINITY);
    let (mut low_frac, mut high_frac) = (1.0f32, 0.0f32);
    for &i in picked {
        let Some(point) = from.get(i) else { continue };
        let before = (0..i).rev().find(|k| !picked.contains(k));
        let after = (i + 1..from.len()).find(|k| !picked.contains(k));
        lo = lo.max(before.map_or(-point.beat, |k| from[k].beat + POINT_GAP - point.beat));
        hi = hi.min(after.map_or(f64::INFINITY, |k| from[k].beat - POINT_GAP - point.beat));
        let f = frac_of(target, point.value);
        low_frac = low_frac.min(f);
        high_frac = high_frac.max(f);
    }
    let by_beats = if lo <= hi {
        by_beats.clamp(lo, hi)
    } else {
        0.0
    };
    let by_frac = by_frac.clamp(-low_frac, (1.0 - high_frac).max(-low_frac));
    let mut lane = from.to_vec();
    for &i in picked {
        if let Some(point) = lane.get_mut(i) {
            point.beat += by_beats;
            if by_frac != 0.0 {
                point.value = value_of(target, frac_of(target, point.value) + by_frac);
            }
        }
    }
    lane
}

/// The points of `lane` inside the beats `beats` and the heights `fracs` (0..=1 up the lane).
pub fn points_in_box(
    lane: &[AutoPoint],
    target: AutoTarget,
    beats: (f64, f64),
    fracs: (f32, f32),
) -> Vec<usize> {
    lane.iter()
        .enumerate()
        .filter(|(_, q)| {
            let f = frac_of(target, q.value);
            q.beat >= beats.0.min(beats.1)
                && q.beat <= beats.0.max(beats.1)
                && f >= fracs.0.min(fracs.1)
                && f <= fracs.0.max(fracs.1)
        })
        .map(|(i, _)| i)
        .collect()
}

/// Takes the `picked` points out of `lane`.
pub fn delete_points(lane: &mut Vec<AutoPoint>, picked: &[usize]) {
    let mut at = 0;
    lane.retain(|_| {
        at += 1;
        !picked.contains(&(at - 1))
    });
}

/// A point edited by hand plays: a strip with its automation Off is put in Read.
fn plays_when_edited(t: &mut Track, beat: f64) {
    if t.auto.mode == AutoMode::Off {
        set_mode(t, AutoMode::Read, beat);
    }
}

/// Deletes the points picked on a lane. Returns whether there were any.
pub fn delete_picked(p: &mut Project, pick: &mut Option<LanePick>) -> bool {
    let Some(picked) = pick.take().filter(|x| !x.points.is_empty()) else {
        return false;
    };
    if let Some(t) = p.tracks.get_mut(picked.track) {
        delete_points(t.auto.lane_mut(picked.target), &picked.points);
        t.auto.prune();
    }
    true
}

/// What the lane on a track needs from the Tracks area.
pub struct LaneView<'a> {
    /// The whole width of the track's row, under the name bars of its regions.
    pub rect: Rect,
    pub x_of: &'a dyn Fn(f64) -> f32,
    pub beat_of: &'a dyn Fn(f32) -> f64,
    pub snap: &'a dyn Fn(f64) -> f64,
    /// It takes the mouse: with another tool in hand it is only drawn.
    pub active: bool,
}

/// What was asked of the lane's menu.
enum LaneMenu {
    SelectAll,
    Delete,
    Clear,
    Hide,
}

/// Height in the lane (0 at the bottom, 1 at the top) of a pointer at `y`.
fn frac_at(rect: Rect, y: f32) -> f32 {
    ((rect.bottom() - y) / rect.height().max(1.0)).clamp(0.0, 1.0)
}

/// The curve of a lane drawn on its track, with its editing. A click adds a point (it lands on
/// the snap grid, or anywhere with Alt held), a drag on a point moves it and the others picked with
/// it, a drag from an empty spot picks with a box, and a double click or right click on a point
/// deletes it.
pub fn track_lane(
    ui: &Ui,
    g: &egui::Painter,
    p: &mut Project,
    ti: usize,
    view: &LaneView,
    pick: &mut Option<LanePick>,
    drag: &mut Option<LaneDrag>,
) -> bool {
    let rect = view.rect;
    if rect.height() < 6.0 || p.tracks.get(ti).is_none() {
        return false;
    }
    let mut target = p.tracks[ti].auto.shown;
    if lane_label(&p.tracks[ti], target) == "?" {
        target = AutoTarget::Volume;
        p.tracks[ti].auto.shown = target;
    }
    let colour = lane_colour(target);
    let to_pos = |q: &AutoPoint| {
        pos2(
            (view.x_of)(q.beat),
            rect.bottom() - frac_of(target, q.value) * rect.height(),
        )
    };
    g.rect_filled(rect, 0.0, Color32::from_black_alpha(70));
    let points: Vec<AutoPoint> = p.tracks[ti].auto.lane(target).to_vec();
    let handle_pos = |i: usize| {
        let (a, b) = (points[i], points[i + 1]);
        let v = a.value + (b.value - a.value) * curve_shape(0.5, a.curve);
        to_pos(&AutoPoint {
            beat: (a.beat + b.beat) / 2.0,
            value: v,
            curve: 0.0,
        })
    };
    let picked: Vec<usize> = pick
        .as_ref()
        .filter(|x| x.track == ti && x.target == target)
        .map_or_else(Vec::new, |x| x.points.clone());

    // The curve: it holds its first value before the first point and its last after the last.
    if points.is_empty() {
        let own = current(&p.tracks[ti], target);
        let y = rect.bottom() - frac_of(target, own) * rect.height();
        g.line_segment(
            [pos2(rect.left(), y), pos2(rect.right(), y)],
            Stroke::new(1.0, colour.gamma_multiply(0.5)),
        );
    } else {
        let mut line = vec![pos2(rect.left(), to_pos(&points[0]).y)];
        for (i, a) in points.iter().enumerate() {
            line.push(to_pos(a));
            if let (Some(b), true) = (points.get(i + 1), a.curve != 0.0) {
                for k in 1..16 {
                    let t = k as f32 / 16.0;
                    line.push(to_pos(&AutoPoint {
                        beat: a.beat + (b.beat - a.beat) * f64::from(t),
                        value: a.value + (b.value - a.value) * curve_shape(t, a.curve),
                        curve: 0.0,
                    }));
                }
            }
        }
        line.push(pos2(rect.right(), line[line.len() - 1].y));
        g.add(Shape::line(line, Stroke::new(1.6, colour)));
        for i in 0..points.len() - 1 {
            let c = handle_pos(i);
            if points[i].value != points[i + 1].value && rect.expand(3.0).contains(c) {
                let r = 3.0;
                let shape = vec![
                    c + vec2(0.0, -r),
                    c + vec2(r, 0.0),
                    c + vec2(0.0, r),
                    c + vec2(-r, 0.0),
                ];
                let fill = if points[i].curve != 0.0 {
                    colour
                } else {
                    crate::theme::BORDER
                };
                g.add(Shape::convex_polygon(shape, fill, Stroke::new(1.0, colour)));
            }
        }
        for (i, q) in points.iter().enumerate() {
            let c = to_pos(q);
            if rect.expand(5.0).contains(c) {
                let fill = if picked.contains(&i) {
                    Color32::WHITE
                } else {
                    colour
                };
                g.circle_filled(c, 3.5, fill);
                g.circle_stroke(c, 3.5, Stroke::new(1.0, Color32::from_gray(0x09)));
            }
        }
    }
    if !view.active {
        return false;
    }

    let resp = ui.interact(
        rect,
        egui::Id::unique(("track_automation", ti)),
        Sense::click_and_drag(),
    );
    let pressed = resp.clicked() || resp.drag_started() || resp.secondary_clicked();
    let near_point = |pos: egui::Pos2| {
        let closest = points
            .iter()
            .enumerate()
            .map(|(i, q)| (i, to_pos(q).distance(pos)))
            .filter(|(_, d)| *d <= GRAB)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        closest.map(|(i, _)| i)
    };
    let near_handle = |pos: egui::Pos2| {
        (0..points.len().saturating_sub(1)).find(|i| {
            points[*i].value != points[*i + 1].value && handle_pos(*i).distance(pos) <= GRAB
        })
    };
    let (ctrl, free) = ui.input(|i| (i.modifiers.command, i.modifiers.alt));
    let beat_at = |x: f32| {
        let beat = (view.beat_of)(x).max(0.0);
        if free {
            beat
        } else {
            (view.snap)(beat).max(0.0)
        }
    };
    let beat_now = p.position_beats;
    if pressed {
        p.selected_track = ti;
    }
    if resp.drag_started() {
        let origin = ui.input(|i| i.pointer.press_origin());
        *drag = origin.or(resp.interact_pointer_pos()).map(|pos| {
            let start = ((view.beat_of)(pos.x), frac_at(rect, pos.y));
            if let Some(i) = near_point(pos) {
                if !picked.contains(&i) {
                    let mut now = if ctrl { picked.clone() } else { Vec::new() };
                    now.push(i);
                    *pick = Some(LanePick {
                        track: ti,
                        target,
                        points: now,
                    });
                }
                LaneDrag::Points {
                    grabbed: i,
                    from: points.clone(),
                    at: start,
                }
            } else if let Some(i) = near_handle(pos) {
                LaneDrag::Curve(i)
            } else {
                LaneDrag::Box(start.0, start.1)
            }
        });
    }
    if let (true, Some(pos)) = (resp.dragged(), resp.interact_pointer_pos()) {
        match drag.as_ref() {
            Some(LaneDrag::Points { grabbed, from, at }) => {
                let moving = pick.as_ref().map_or_else(Vec::new, |x| x.points.clone());
                let by_beats = from.get(*grabbed).map_or(0.0, |lead| {
                    let wanted = lead.beat + (view.beat_of)(pos.x) - at.0;
                    let landed = if free { wanted } else { (view.snap)(wanted) };
                    landed - lead.beat
                });
                let lane =
                    moved_group(from, &moving, target, by_beats, frac_at(rect, pos.y) - at.1);
                plays_when_edited(&mut p.tracks[ti], beat_now);
                *p.tracks[ti].auto.lane_mut(target) = lane;
            }
            Some(LaneDrag::Curve(i)) => {
                let i = *i;
                let lane = p.tracks[ti].auto.lane_mut(target);
                if i + 1 < lane.len() && lane[i].value != lane[i + 1].value {
                    let v = value_of(target, frac_at(rect, pos.y));
                    lane[i].curve = curve_from_fraction(
                        (v - lane[i].value) / (lane[i + 1].value - lane[i].value),
                    );
                }
            }
            Some(LaneDrag::Box(beat, frac)) => {
                let beats = (*beat, (view.beat_of)(pos.x));
                let fracs = (*frac, frac_at(rect, pos.y));
                *pick = Some(LanePick {
                    track: ti,
                    target,
                    points: points_in_box(&points, target, beats, fracs),
                });
                let top = rect.bottom() - frac * rect.height();
                let boxed = Rect::from_two_pos(pos2((view.x_of)(*beat), top), pos);
                g.rect_filled(boxed, 0.0, Color32::from_white_alpha(22));
                g.rect_stroke(
                    boxed,
                    0.0,
                    Stroke::new(1.0, Color32::from_white_alpha(150)),
                    StrokeKind::Inside,
                );
            }
            None => {}
        }
    }
    if resp.drag_stopped() {
        *drag = None;
    }
    if resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            match near_point(pos) {
                Some(i) if resp.double_clicked() => {
                    p.tracks[ti].auto.lane_mut(target).remove(i);
                    p.tracks[ti].auto.prune();
                    *pick = None;
                }
                Some(i) => {
                    let mut now = if ctrl { picked.clone() } else { Vec::new() };
                    match now.iter().position(|k| *k == i) {
                        Some(at) => {
                            now.remove(at);
                        }
                        None => now.push(i),
                    }
                    *pick = Some(LanePick {
                        track: ti,
                        target,
                        points: now,
                    });
                }
                None if near_handle(pos).is_some() => {
                    if let (true, Some(i)) = (resp.double_clicked(), near_handle(pos)) {
                        p.tracks[ti].auto.lane_mut(target)[i].curve = 0.0;
                    }
                }
                // With points picked, a click away lets them go; the next one adds a point.
                None if !picked.is_empty() => *pick = None,
                None => {
                    plays_when_edited(&mut p.tracks[ti], beat_now);
                    let value = value_of(target, frac_at(rect, pos.y));
                    p.tracks[ti].auto.set_point(target, beat_at(pos.x), value);
                }
            }
        }
    }
    let on_point = resp.interact_pointer_pos().and_then(near_point);
    if let (true, Some(i)) = (resp.secondary_clicked(), on_point) {
        p.tracks[ti].auto.lane_mut(target).remove(i);
        p.tracks[ti].auto.prune();
        *pick = None;
        return pressed;
    }
    let mut asked = None;
    crate::widgets::context_menu(&resp, |ui| {
        if ui.button("Select All Points").clicked() {
            asked = Some(LaneMenu::SelectAll);
            ui.close();
        }
        if ui
            .add_enabled(
                !picked.is_empty(),
                egui::Button::new("Delete Selected Points"),
            )
            .clicked()
        {
            asked = Some(LaneMenu::Delete);
            ui.close();
        }
        if ui.button(tr("Clear Lane")).clicked() {
            asked = Some(LaneMenu::Clear);
            ui.close();
        }
        ui.separator();
        if ui.button("Hide Automation on Track").clicked() {
            asked = Some(LaneMenu::Hide);
            ui.close();
        }
    });
    match asked {
        Some(LaneMenu::SelectAll) => {
            *pick = Some(LanePick {
                track: ti,
                target,
                points: (0..points.len()).collect(),
            });
        }
        Some(LaneMenu::Delete) => {
            delete_picked(p, pick);
        }
        Some(LaneMenu::Clear) => {
            p.tracks[ti].auto.lane_mut(target).clear();
            p.tracks[ti].auto.prune();
            *pick = None;
        }
        Some(LaneMenu::Hide) => {
            p.tracks[ti].show_auto = false;
            *pick = None;
        }
        None => {}
    }
    if let Some(pos) = resp.hover_pos() {
        let (b, v) = match near_point(pos) {
            Some(i) => (points[i].beat, points[i].value),
            None => (
                (view.beat_of)(pos.x),
                value_of(target, frac_at(rect, pos.y)),
            ),
        };
        let bar = p.beats_to_bar(b) + 1.0;
        let text = format!("{} · {:.2}", value_text(&p.tracks[ti], target, v), bar);
        resp.on_hover_text(text);
    }
    pressed
}

/// The colour of a lane's curve.
fn lane_colour(target: AutoTarget) -> Color32 {
    match target {
        AutoTarget::Volume => Color32::from_rgb(0x6c, 0xd0, 0x5a),
        AutoTarget::Pan => Color32::from_rgb(0x5a, 0xa8, 0xf0),
        AutoTarget::Fx { .. } | AutoTarget::Plugin { .. } => Color32::from_rgb(0xe6, 0x9a, 0x2e),
    }
}

/// The lane's drawing area and its mouse editing.
fn canvas(
    ui: &mut Ui,
    p: &mut Project,
    ed: &mut AutoEditor,
    ti: usize,
    target: AutoTarget,
    stretch: Option<(f64, f64)>,
) {
    let width = ui.available_width().max(200.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, CANVAS_H), Sense::click_and_drag());
    let g = ui.painter_at(rect);
    g.rect_filled(rect, 3.0, crate::theme::BORDER);
    let (from, span) = (ed.from, ed.span.max(1.0));
    // Bar lines.
    for bar in p.bars_in(from, from + span) {
        let x = rect.left() + ((bar.start - from) / span) as f32 * rect.width();
        if x < rect.left() || x > rect.right() {
            continue;
        }
        g.line_segment(
            [pos2(x, rect.top()), pos2(x, rect.bottom())],
            Stroke::new(1.0, Color32::from_gray(0x1d)),
        );
        if span <= 128.0 || bar.bar0 % 4 == 0 {
            g.text(
                pos2(x + 3.0, rect.top() + 2.0),
                Align2::LEFT_TOP,
                format!("{}", bar.bar0 + 1),
                FontId::proportional(10.0),
                Color32::from_gray(0x50),
            );
        }
    }
    // Outside a region's stretch is dimmed: a point dragged there lands on its edge.
    if let Some((a, b)) = stretch {
        let x_of = |beat: f64| rect.left() + ((beat - from) / span) as f32 * rect.width();
        let dim = Color32::from_black_alpha(110);
        let (xa, xb) = (
            x_of(a).clamp(rect.left(), rect.right()),
            x_of(b).clamp(rect.left(), rect.right()),
        );
        g.rect_filled(
            Rect::from_min_max(rect.min, pos2(xa, rect.bottom())),
            0.0,
            dim,
        );
        g.rect_filled(Rect::from_min_max(pos2(xb, rect.top()), rect.max), 0.0, dim);
    }
    // Value lines: 0 dB, or the centre.
    let marks: &[f32] = match target {
        AutoTarget::Volume => &[6.0, 0.0, -6.0, -20.0, -40.0],
        AutoTarget::Pan => &[-1.0, 0.0, 1.0],
        AutoTarget::Fx { .. } | AutoTarget::Plugin { .. } => &[0.0, 0.25, 0.5, 0.75, 1.0],
    };
    for m in marks {
        let y = rect.bottom() - frac_of(target, *m) * rect.height();
        g.line_segment(
            [pos2(rect.left(), y), pos2(rect.right(), y)],
            Stroke::new(1.0, Color32::from_gray(if *m == 0.0 { 0x4a } else { 0x30 })),
        );
        g.text(
            pos2(rect.right() - 4.0, y - 1.0),
            Align2::RIGHT_BOTTOM,
            value_text(&p.tracks[ti], target, *m),
            FontId::proportional(9.0),
            Color32::from_gray(0x40),
        );
    }
    let colour = lane_colour(target);
    let points: Vec<AutoPoint> = p.tracks[ti].auto.lane(target).to_vec();
    let to_pos = |beat: f64, value: f32| point_pos(rect, from, span, target, beat, value);
    // Where the handle that bends the stretch after point `i` is.
    let handle_pos = |i: usize| {
        let (a, b) = (points[i], points[i + 1]);
        let v = a.value + (b.value - a.value) * curve_shape(0.5, a.curve);
        to_pos((a.beat + b.beat) / 2.0, v)
    };
    if !points.is_empty() {
        let mut line: Vec<egui::Pos2> = Vec::with_capacity(points.len() * 8 + 2);
        line.push(pos2(rect.left(), to_pos(points[0].beat, points[0].value).y));
        for (i, a) in points.iter().enumerate() {
            line.push(to_pos(a.beat, a.value));
            if let Some(b) = points.get(i + 1) {
                if a.curve != 0.0 {
                    for k in 1..16 {
                        let t = k as f32 / 16.0;
                        let v = a.value + (b.value - a.value) * curve_shape(t, a.curve);
                        line.push(to_pos(a.beat + (b.beat - a.beat) * t as f64, v));
                    }
                }
            }
        }
        let last = points[points.len() - 1];
        line.push(pos2(rect.right(), to_pos(last.beat, last.value).y));
        g.add(Shape::line(line, Stroke::new(1.6, colour)));
        for i in 0..points.len().saturating_sub(1) {
            if points[i].value == points[i + 1].value {
                continue;
            }
            let c = handle_pos(i);
            if !rect.expand(4.0).contains(c) {
                continue;
            }
            let r = 3.5;
            let diamond = vec![
                c + vec2(0.0, -r),
                c + vec2(r, 0.0),
                c + vec2(0.0, r),
                c + vec2(-r, 0.0),
            ];
            let bent = points[i].curve != 0.0;
            g.add(Shape::convex_polygon(
                diamond,
                if bent || ed.drag == Some(Drag::Curve(i)) {
                    colour
                } else {
                    crate::theme::BORDER
                },
                Stroke::new(1.0, colour),
            ));
        }
        for (i, a) in points.iter().enumerate() {
            let c = to_pos(a.beat, a.value);
            if !rect.expand(4.0).contains(c) {
                continue;
            }
            g.circle_filled(
                c,
                4.0,
                if ed.drag == Some(Drag::Point(i)) {
                    Color32::WHITE
                } else {
                    colour
                },
            );
            g.circle_stroke(c, 4.0, Stroke::new(1.0, Color32::from_gray(0x09)));
        }
    } else {
        g.text(
            rect.center(),
            Align2::CENTER_CENTER,
            tr("Click to add a point"),
            FontId::proportional(12.0),
            Color32::from_gray(0x40),
        );
    }
    // The playhead.
    let x = rect.left() + ((p.position_beats - from) / span) as f32 * rect.width();
    if x >= rect.left() && x <= rect.right() {
        g.line_segment(
            [pos2(x, rect.top()), pos2(x, rect.bottom())],
            Stroke::new(1.0, crate::tracks_area::PLAYHEAD_LINE),
        );
    }
    g.rect_stroke(
        rect,
        3.0,
        Stroke::new(1.0, Color32::from_gray(0x09)),
        StrokeKind::Inside,
    );

    // Editing.
    let near_point = |pos: egui::Pos2| {
        points
            .iter()
            .position(|pt| to_pos(pt.beat, pt.value).distance(pos) <= GRAB)
    };
    let near_handle = |pos: egui::Pos2| {
        (0..points.len().saturating_sub(1)).find(|i| {
            points[*i].value != points[*i + 1].value && handle_pos(*i).distance(pos) <= GRAB
        })
    };
    if resp.drag_started() {
        ed.drag = resp.interact_pointer_pos().and_then(|pos| {
            if let Some(i) = near_point(pos) {
                Some(Drag::Point(i))
            } else if let Some(i) = near_handle(pos) {
                Some(Drag::Curve(i))
            } else {
                // Dragging from empty space adds a point and moves it.
                let (b, v) = beat_value_at(rect, from, span, target, pos);
                p.tracks[ti].auto.set_point(target, b, v);
                p.tracks[ti]
                    .auto
                    .lane(target)
                    .iter()
                    .position(|pt| (pt.beat - b).abs() < 1e-9)
                    .map(Drag::Point)
            }
        });
    }
    if let (Some(drag), Some(pos), true) = (ed.drag, resp.interact_pointer_pos(), resp.dragged()) {
        let (b, v) = beat_value_at(rect, from, span, target, pos);
        let lane = p.tracks[ti].auto.lane_mut(target);
        match drag {
            Drag::Point(i) if i < lane.len() => {
                // A point stays between its neighbours.
                let lo = if i == 0 { 0.0 } else { lane[i - 1].beat + 1e-4 };
                let hi = lane.get(i + 1).map_or(f64::MAX, |n| n.beat - 1e-4);
                lane[i].beat = b.clamp(lo, hi.max(lo));
                lane[i].value = v;
            }
            Drag::Curve(i) if i + 1 < lane.len() => {
                let (a, c) = (lane[i], lane[i + 1]);
                if c.value != a.value {
                    lane[i].curve = curve_from_fraction((v - a.value) / (c.value - a.value));
                }
            }
            _ => {}
        }
    }
    if resp.drag_stopped() {
        ed.drag = None;
    }
    if resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            if near_point(pos).is_none() && near_handle(pos).is_none() {
                let (b, v) = beat_value_at(rect, from, span, target, pos);
                p.tracks[ti].auto.set_point(target, b, v);
            }
        }
    }
    if resp.secondary_clicked() || resp.double_clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let lane = p.tracks[ti].auto.lane_mut(target);
            if let Some(i) = near_point(pos) {
                if i < lane.len() {
                    lane.remove(i);
                }
            } else if let Some(i) = near_handle(pos) {
                // A double click on a handle makes the stretch straight again.
                if i < lane.len() {
                    lane[i].curve = 0.0;
                }
            }
        }
    }
    p.tracks[ti].auto.prune();
    // Hover: the value under the pointer.
    if let Some(pos) = resp.hover_pos() {
        let (b, v) = match near_point(pos) {
            Some(i) => (points[i].beat, points[i].value),
            None => beat_value_at(rect, from, span, target, pos),
        };
        let bar = p.beats_to_bar(b) + 1.0;
        let text = format!("{} · {:.2}", value_text(&p.tracks[ti], target, v), bar);
        resp.on_hover_text(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TrackKind;

    /// A track with one MIDI region at bar 3 and a volume lane of its own.
    fn region_project() -> (Project, usize) {
        let mut p = Project::empty();
        let t = p.add_track(TrackKind::Instrument);
        p.add_midi_region(t, 3.0);
        p.tracks[t].regions[0].auto.push(crate::model::RegionLane {
            target: AutoTarget::Volume,
            points: vec![
                AutoPoint {
                    beat: 0.0,
                    value: -12.0,
                    curve: 0.0,
                },
                AutoPoint {
                    beat: 2.0,
                    value: -3.0,
                    curve: 0.0,
                },
            ],
        });
        (p, t)
    }

    #[test]
    fn a_region_lane_plays_over_its_stretch_and_the_strip_comes_back_after_it() {
        let (mut p, t) = region_project();
        p.tracks[t].volume_db = -1.0;
        let from = p.region_start_beats(&p.tracks[t].regions[0]);
        let to = p.region_end_beats(&p.tracks[t].regions[0]);
        let merged = merged_lanes(&p, t);
        assert_eq!(merged.len(), 1);
        let lane = &merged[0].1;
        let at = |b: f64| ovrtone_engine::lane_value(lane, b).unwrap_or(f32::NAN);
        assert!((at(from - 1.0) + 1.0).abs() < 1e-3, "before: the fader");
        assert!((at(from) + 12.0).abs() < 0.1, "at its start");
        assert!((at(from + 2.0) + 3.0).abs() < 0.1, "inside");
        assert!((at(to + 1.0) + 1.0).abs() < 1e-3, "after: the fader again");
        assert!(lane.windows(2).all(|w| w[0].beat < w[1].beat), "in order");
        p.tracks[t].regions[0].start_bar += 4.0;
        let moved = merged_lanes(&p, t);
        let start = p.region_start_beats(&p.tracks[t].regions[0]);
        let value = ovrtone_engine::lane_value(&moved[0].1, start).unwrap_or(f32::NAN);
        assert!((value + 12.0).abs() < 0.1, "it moves with the region");
    }

    #[test]
    fn a_track_with_no_region_lanes_gets_nothing_merged() {
        let mut p = Project::empty();
        let t = p.add_track(TrackKind::Instrument);
        p.add_midi_region(t, 1.0);
        p.tracks[t].auto.set_point(AutoTarget::Volume, 1.0, -6.0);
        assert!(merged_lanes(&p, t).is_empty());
    }

    #[test]
    fn convert_moves_points_between_track_and_region_and_back() {
        let (mut p, t) = region_project();
        convert_to_track(&mut p, t, 0);
        assert!(p.tracks[t].regions[0].auto.is_empty());
        let from = p.region_start_beats(&p.tracks[t].regions[0]);
        assert_eq!(p.tracks[t].auto.lane(AutoTarget::Volume).len(), 2);
        assert!((p.tracks[t].auto.lane(AutoTarget::Volume)[0].beat - from).abs() < 1e-9);
        assert!(can_convert(&p, t, 0));
        p.tracks[t].auto.set_point(AutoTarget::Volume, 0.0, -20.0);
        convert_to_region(&mut p, t, 0);
        assert_eq!(p.tracks[t].regions[0].auto[0].points.len(), 2);
        assert!(p.tracks[t].regions[0].auto[0].points[0].beat.abs() < 1e-9);
        assert_eq!(
            p.tracks[t].auto.lane(AutoTarget::Volume).len(),
            1,
            "outside stays"
        );
    }

    #[test]
    fn cutting_and_trimming_keep_each_point_where_it_was_in_the_song() {
        let (p, t) = region_project();
        let mut left = p.tracks[t].regions[0].auto.clone();
        let right = cut_lanes(&mut left, 1.0);
        let value = |l: &[crate::model::RegionLane], b: f64| {
            ovrtone_engine::lane_value(&l[0].points, b).unwrap_or(f32::NAN)
        };
        let middle = -7.5;
        assert!((value(&left, 1.0) - middle).abs() < 0.01);
        assert!((value(&right, 0.0) - middle).abs() < 0.01);
        assert!((value(&right, 1.0) + 3.0).abs() < 0.01);
        let trimmed = trimmed_lanes(&p.tracks[t].regions[0].auto, 1.0);
        assert!((value(&trimmed, 0.0) - middle).abs() < 0.01);
        assert!((value(&trimmed, 1.0) + 3.0).abs() < 0.01);
        let mut joined = left;
        join_lanes(&mut joined, &right, 1.0);
        assert!((value(&joined, 2.0) + 3.0).abs() < 0.01);
    }

    #[test]
    fn a_plugin_control_held_keeps_touch_and_letting_go_ends_it() {
        let mut t = crate::model::Track::new("t", crate::model::TrackKind::Audio);
        t.auto.mode = AutoMode::Touch;
        let target = AutoTarget::Plugin { uid: 5, param: 1 };
        plugin_moved(&mut t, target, 0.5, 1.0, true);
        plugin_gesture(&mut t, 5, true, false, 100.0);
        assert!(t.auto.plugin_touch_until > 100.0, "still held");
        plugin_gesture(&mut t, 9, false, true, 100.0);
        assert!(t.auto.plugin_touch_until > 100.0, "another plugin");
        plugin_gesture(&mut t, 5, false, true, 100.0);
        assert!(t.auto.plugin_touch_until < 100.0, "let go");
    }
    use ovrtone_engine::FxKind;

    fn lane(beats: &[f64]) -> Vec<AutoPoint> {
        beats.iter().map(|b| AutoPoint::new(*b, 0.5)).collect()
    }

    #[test]
    fn a_group_of_points_moves_together_and_stops_at_its_neighbours() {
        let target = AutoTarget::Fx { slot: 0, param: 0 };
        let from = lane(&[1.0, 2.0, 3.0, 8.0]);
        let moved = moved_group(&from, &[1, 2], target, 1.0, 0.25);
        assert_eq!(moved[0], from[0], "not picked");
        assert_eq!((moved[1].beat, moved[2].beat), (3.0, 4.0));
        assert!((moved[1].value - 0.75).abs() < 1e-6);
        assert_eq!(moved[3], from[3], "not picked");
        // It cannot pass the point after it, or the one before it.
        let far = moved_group(&from, &[1, 2], target, 100.0, 0.0);
        assert!((far[2].beat - (8.0 - POINT_GAP)).abs() < 1e-9);
        assert!((far[1].beat - (7.0 - POINT_GAP)).abs() < 1e-9, "same shape");
        let back = moved_group(&from, &[1, 2], target, -100.0, 0.0);
        assert!((back[1].beat - (1.0 + POINT_GAP)).abs() < 1e-9);
        // Nor go before the song, or off the lane: the group keeps its shape.
        let all = moved_group(&from, &[0, 1, 2, 3], target, -100.0, 0.0);
        assert_eq!(all[0].beat, 0.0);
        assert_eq!(all[3].beat, 7.0);
        let mut uneven = from.clone();
        uneven[1].value = 0.9;
        let up = moved_group(&uneven, &[0, 1], target, 0.0, 0.5);
        assert!((up[1].value - 1.0).abs() < 1e-6);
        assert!((up[0].value - 0.6).abs() < 1e-6);
    }

    #[test]
    fn a_box_picks_what_it_covers_and_picked_points_can_be_deleted() {
        let target = AutoTarget::Pan;
        let mut points = lane(&[1.0, 2.0, 3.0]);
        points[1].value = -1.0;
        // Backwards too. A pan of 0.5 is three quarters up the lane.
        assert_eq!(
            points_in_box(&points, target, (3.5, 0.5), (1.0, 0.5)),
            [0, 2]
        );
        assert!(points_in_box(&points, target, (4.0, 5.0), (0.0, 1.0)).is_empty());
        delete_points(&mut points, &[0, 2]);
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].beat, 2.0);
        let mut p = Project::empty();
        p.tracks[0].auto.volume = lane(&[1.0, 2.0]);
        let mut pick = Some(LanePick {
            track: 0,
            target: AutoTarget::Volume,
            points: vec![0],
        });
        assert!(delete_picked(&mut p, &mut pick));
        assert_eq!(p.tracks[0].auto.volume.len(), 1);
        assert!(pick.is_none());
        assert!(!delete_picked(&mut p, &mut pick), "nothing picked");
    }

    /// One track's lane, drawn over the whole 800 by 100 window.
    fn lane_frame(
        ui: &mut Ui,
        p: &mut Project,
        pick: &mut Option<LanePick>,
        drag: &mut Option<LaneDrag>,
    ) {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 100.0));
        ui.allocate_rect(rect, Sense::hover());
        let g = ui.painter_at(rect);
        let view = LaneView {
            rect,
            x_of: &|b| b as f32 * 100.0,
            beat_of: &|x| f64::from(x) / 100.0,
            snap: &|b| b.round(),
            active: true,
        };
        track_lane(ui, &g, p, 0, &view, pick, drag);
    }

    #[test]
    fn the_lane_on_a_track_adds_snaps_moves_and_deletes_points_with_the_mouse() {
        let mut p = Project::empty();
        let target = AutoTarget::Fx { slot: 0, param: 0 };
        p.tracks[0].inserts = vec![crate::model::Insert::new(FxKind::Limiter)];
        p.tracks[0].auto.shown = target;
        p.tracks[0].auto.mode = AutoMode::Off;
        let (mut pick, mut drag) = (None, None);
        let mut h = crate::ui_test::Harness::new(800.0, 100.0);
        macro_rules! run {
            () => {
                |ui: &mut Ui| lane_frame(ui, &mut p, &mut pick, &mut drag)
            };
        }
        h.settle(run!());
        // A click adds a point, on the grid; the track now plays its lane.
        h.click(pos2(210.0, 50.0), run!());
        h.frame(vec![], run!());
        assert_eq!(p.tracks[0].auto.lane(target).len(), 1);
        assert_eq!(p.tracks[0].auto.lane(target)[0].beat, 2.0);
        assert_eq!(p.tracks[0].auto.mode, AutoMode::Read);
        h.click(pos2(590.0, 25.0), run!());
        h.frame(vec![], run!());
        assert_eq!(p.tracks[0].auto.lane(target)[1].beat, 6.0);
        assert!((p.tracks[0].auto.lane(target)[1].value - 0.75).abs() < 0.01);
        // Alt: off the grid.
        h.modifiers = egui::Modifiers::ALT;
        h.click(pos2(410.0, 50.0), run!());
        h.modifiers = egui::Modifiers::NONE;
        h.frame(vec![], run!());
        assert!((p.tracks[0].auto.lane(target)[1].beat - 4.1).abs() < 0.01);
        assert_eq!(p.tracks[0].auto.lane(target).len(), 3);
        // A box picks the first two; dragging one moves both, by whole beats.
        h.drag(
            &[pos2(100.0, 95.0), pos2(300.0, 60.0), pos2(450.0, 5.0)],
            egui::PointerButton::Primary,
            run!(),
        );
        assert_eq!(pick.as_ref().map(|x| x.points.clone()), Some(vec![0, 1]));
        let was = p.tracks[0].auto.lane(target).to_vec();
        h.drag(
            &[pos2(200.0, 50.0), pos2(250.0, 50.0), pos2(300.0, 50.0)],
            egui::PointerButton::Primary,
            run!(),
        );
        let now = p.tracks[0].auto.lane(target).to_vec();
        assert_eq!(now[0].beat, 3.0, "grabbed: lands on a beat");
        assert!((now[1].beat - (was[1].beat + 1.0)).abs() < 1e-6, "with it");
        assert_eq!(now[2], was[2], "not picked");
        // Right click on a point deletes it.
        let at = pos2(now[2].beat as f32 * 100.0, 25.0);
        h.right_click(at, run!());
        assert_eq!(p.tracks[0].auto.lane(target).len(), 2);
    }

    fn project() -> Project {
        let mut p = Project::empty();
        p.tracks[0].auto.mode = AutoMode::Touch;
        p
    }

    #[test]
    fn moves_in_a_plugin_window_are_written_when_the_strip_records() {
        let mut p = crate::model::Project::empty();
        let t = &mut p.tracks[0];
        let target = AutoTarget::Plugin { uid: 5, param: 9 };
        t.auto.mode = AutoMode::Read;
        plugin_moved(t, target, 0.5, 1.0, true);
        assert!(t.auto.lane(target).is_empty(), "Read only plays it");
        t.auto.mode = AutoMode::Touch;
        plugin_moved(t, target, 0.5, 1.0, false);
        assert!(t.auto.lane(target).is_empty(), "not while stopped");
        plugin_moved(t, target, 0.5, 1.0, true);
        plugin_moved(t, target, 0.8, 1.5, true);
        assert_eq!(t.auto.lane(target).len(), 2);
        assert_eq!(t.auto.held, Some(target));
        // Let go: after a while the lane plays again.
        p.playing = true;
        p.position_beats = 10.0;
        tick(&mut p);
        assert_eq!(p.tracks[0].auto.held, None);
        assert_eq!(
            p.tracks[0].auto.lane(target).len(),
            2,
            "tick writes nothing of its own"
        );
    }

    #[test]
    fn touch_writes_while_held_and_returns_to_the_lane() {
        let mut p = project();
        p.tracks[0].auto.set_point(AutoTarget::Volume, 0.0, -6.0);
        p.tracks[0].auto.set_point(AutoTarget::Volume, 100.0, -6.0);
        p.playing = true;
        p.position_beats = 8.0;
        // Nothing held: the strip plays the lane and tick writes nothing.
        tick(&mut p);
        assert_eq!(p.tracks[0].auto.volume.len(), 2);
        assert_eq!(shown_value(&p.tracks[0], AutoTarget::Volume, 8.0), -6.0);
        // The fader is touched and pulled down while playing.
        moved(&mut p.tracks[0], AutoTarget::Volume, -20.0);
        assert_eq!(p.tracks[0].auto.held, Some(AutoTarget::Volume));
        for k in 0..8 {
            p.position_beats = 8.0 + k as f64 * 0.25;
            tick(&mut p);
        }
        released(&mut p.tracks[0], AutoTarget::Volume, p.position_beats, true);
        assert_eq!(p.tracks[0].auto.held, None);
        let written: Vec<_> = p.tracks[0]
            .auto
            .volume
            .iter()
            .filter(|pt| (8.0..=10.0).contains(&pt.beat))
            .collect();
        assert!(written.len() >= 8 && written.iter().all(|pt| pt.value == -20.0));
        // Before and after the written stretch the old lane remains.
        assert_eq!(p.tracks[0].auto.volume[0], AutoPoint::new(0.0, -6.0));
        assert_eq!(
            p.tracks[0].auto.volume.last().copied(),
            Some(AutoPoint::new(100.0, -6.0))
        );
        // The lane plays again once the control is let go: it climbs back to the old value.
        assert_eq!(shown_value(&p.tracks[0], AutoTarget::Volume, 100.0), -6.0);
        assert!(shown_value(&p.tracks[0], AutoTarget::Volume, 50.0) < -6.0);
    }

    #[test]
    fn write_overwrites_the_passed_stretch_and_latch_stops_with_playback() {
        let mut p = project();
        p.tracks[0].auto.mode = AutoMode::Write;
        p.tracks[0].auto.set_point(AutoTarget::Volume, 4.5, 0.0);
        p.tracks[0].volume_db = -12.0;
        p.playing = true;
        for k in 0..12 {
            p.position_beats = 4.0 + k as f64 * 0.25;
            tick(&mut p);
        }
        // The old point at 4.5 lies inside the written stretch and is gone.
        assert!(p.tracks[0].auto.volume.iter().all(|pt| pt.value == -12.0));
        assert!(p.tracks[0].auto.volume.len() >= 6);
        // Write mode is not played back, so the fader shows its own value.
        assert_eq!(shown_value(&p.tracks[0], AutoTarget::Volume, 5.0), -12.0);
        // Latch: held after a touch, still writing after the release, cleared by stop.
        set_mode(&mut p.tracks[0], AutoMode::Latch, 6.0);
        moved(&mut p.tracks[0], AutoTarget::Pan, 0.5);
        released(&mut p.tracks[0], AutoTarget::Pan, 6.0, true);
        assert_eq!(p.tracks[0].auto.held, Some(AutoTarget::Pan));
        p.position_beats = 6.0;
        tick(&mut p);
        assert!(!p.tracks[0].auto.pan.is_empty());
        p.playing = false;
        tick(&mut p);
        assert_eq!(p.tracks[0].auto.held, None);
    }

    #[test]
    fn leaving_read_keeps_the_value_and_read_locks_the_control() {
        let mut p = project();
        let t = &mut p.tracks[0];
        t.auto.mode = AutoMode::Read;
        assert!(editable(t, AutoTarget::Volume), "no lane yet: free");
        t.auto.set_point(AutoTarget::Volume, 0.0, -9.0);
        assert!(!editable(t, AutoTarget::Volume));
        assert!(editable(t, AutoTarget::Pan));
        set_mode(t, AutoMode::Off, 3.0);
        assert_eq!(t.volume_db, -9.0);
        assert!(editable(t, AutoTarget::Volume));
        assert_eq!(shown_value(t, AutoTarget::Volume, 3.0), -9.0);
    }

    #[test]
    fn points_stay_sorted_in_range_and_bounded() {
        let mut a = Automation::default();
        a.set_point(AutoTarget::Volume, 4.0, 3.0);
        a.set_point(AutoTarget::Volume, 2.0, -500.0);
        a.set_point(AutoTarget::Volume, 4.0, -3.0);
        a.set_point(AutoTarget::Volume, f64::NAN, 0.0);
        a.set_point(AutoTarget::Pan, 1.0, 7.0);
        assert_eq!(
            a.volume,
            [AutoPoint::new(2.0, -60.0), AutoPoint::new(4.0, -3.0)]
        );
        assert_eq!(a.pan, [AutoPoint::new(1.0, 1.0)]);
        let t = Project::empty().tracks.remove(0);
        assert_eq!(value_text(&t, AutoTarget::Pan, 0.0), "C");
        assert_eq!(value_text(&t, AutoTarget::Pan, -0.5), "L 32");
        assert_eq!(value_text(&t, AutoTarget::Volume, -3.25), "-3.2 dB");
        for target in [AutoTarget::Volume, AutoTarget::Pan] {
            let (lo, hi) = Automation::range(target);
            assert!((value_of(target, frac_of(target, lo)) - lo).abs() < 0.5);
            assert!((value_of(target, frac_of(target, hi)) - hi).abs() < 0.5);
        }
    }

    #[test]
    fn the_bend_handle_puts_the_middle_of_a_stretch_where_it_is_dragged() {
        for f in [0.05f32, 0.2, 0.4, 0.5, 0.6, 0.8, 0.95] {
            let c = curve_from_fraction(f);
            assert!((-1.0..=1.0).contains(&c));
            let back = curve_shape(0.5, c);
            // Within what the bend can reach (the ends of its range hold the nearest).
            let expected = f.clamp(curve_shape(0.5, 1.0), curve_shape(0.5, -1.0));
            assert!((back - expected).abs() < 0.01, "{f}: {back} vs {expected}");
        }
        assert_eq!(curve_from_fraction(0.5), 0.0);
    }

    #[test]
    fn effect_parameters_are_written_played_and_follow_their_effects() {
        let mut p = project();
        let t = &mut p.tracks[0];
        t.inserts = vec![
            crate::model::Insert::new(FxKind::Compressor),
            crate::model::Insert::new(FxKind::ChannelEq),
        ];
        let thr = AutoTarget::Fx { slot: 0, param: 0 };
        // Touch the Threshold slider while playing: its value is written as a position.
        p.playing = true;
        p.position_beats = 2.0;
        moved(&mut p.tracks[0], thr, -30.0);
        assert_eq!(p.tracks[0].auto.held, Some(thr));
        assert_eq!(p.tracks[0].inserts[0].params[0], -30.0);
        tick(&mut p);
        released(&mut p.tracks[0], thr, 2.5, true);
        let lane = p.tracks[0].auto.lane(thr).to_vec();
        assert!(!lane.is_empty());
        // -30 dB in -60..0 is half way.
        assert!((lane[0].value - 0.5).abs() < 1e-5, "{}", lane[0].value);
        assert_eq!(p.tracks[0].auto.fx.len(), 1);
        // Played back: the slider shows the lane's value in dB.
        p.tracks[0].auto.mode = AutoMode::Read;
        let v = shown_value(&p.tracks[0], thr, 3.0);
        assert!((natural(&p.tracks[0], thr, v) + 30.0).abs() < 1e-3);
        assert_eq!(value_text(&p.tracks[0], thr, v), "-30.0 dB");
        assert_eq!(lane_label(&p.tracks[0], thr), "Compressor · Threshold");
        assert!(!editable(&p.tracks[0], thr));
        // Leaving Read keeps the value the lane had.
        p.tracks[0].inserts[0].params[0] = -10.0;
        set_mode(&mut p.tracks[0], AutoMode::Off, 3.0);
        assert!((p.tracks[0].inserts[0].params[0] + 30.0).abs() < 1e-3);
        // Effects that move or go take their lanes with them.
        p.tracks[0]
            .auto
            .set_point(AutoTarget::Fx { slot: 1, param: 1 }, 1.0, 0.8);
        p.tracks[0].auto.insert_swapped(0, 1);
        assert!(!p.tracks[0]
            .auto
            .lane(AutoTarget::Fx { slot: 1, param: 0 })
            .is_empty());
        assert!(!p.tracks[0]
            .auto
            .lane(AutoTarget::Fx { slot: 0, param: 1 })
            .is_empty());
        p.tracks[0].auto.insert_removed(0);
        assert_eq!(p.tracks[0].auto.fx.len(), 1);
        assert_eq!(p.tracks[0].auto.fx[0].slot, 0, "the later effect moved up");
        p.tracks[0].auto.insert_replaced(0);
        assert!(p.tracks[0].auto.fx.is_empty());
        // A lane of an effect that is not there has no name and does not break anything.
        let gone = AutoTarget::Fx { slot: 9, param: 0 };
        assert_eq!(lane_label(&p.tracks[0], gone), "?");
        moved(&mut p.tracks[0], gone, 1.0);
        assert_eq!(current(&p.tracks[0], gone), 0.0);
    }

    #[test]
    fn the_editor_opens_draws_edits_and_bends_a_lane() {
        let mut p = Project::empty();
        let mut s = UiState::new();
        s.mixer.auto_editor.track = Some(0);
        let mut h = crate::ui_test::Harness::new(900.0, 600.0);
        for _ in 0..3 {
            h.frame(vec![], |ui| show(ui.ctx(), &mut p, &mut s));
        }
        assert!(s.mixer.auto_editor.span >= 8.0, "a range was chosen");
        p.tracks[0].auto.set_point(AutoTarget::Volume, 2.0, -6.0);
        p.tracks[0].auto.set_point(AutoTarget::Volume, 6.0, -30.0);
        p.tracks[0].auto.volume[0].curve = 0.6;
        // An effect lane shows too.
        p.tracks[0].inserts = vec![crate::model::Insert::new(FxKind::Limiter)];
        p.tracks[0].auto.shown = AutoTarget::Fx { slot: 0, param: 1 };
        let fx = p.tracks[0].auto.shown;
        p.tracks[0].auto.set_point(fx, 1.0, 0.3);
        p.tracks[0].auto.set_point(fx, 4.0, 0.9);
        for _ in 0..3 {
            h.frame(vec![], |ui| show(ui.ctx(), &mut p, &mut s));
        }
        assert_eq!(p.tracks[0].auto.volume.len(), 2);
        assert_eq!(p.tracks[0].auto.volume[0].curve, 0.6);
        // The effect goes away: the editor falls back to the fader.
        p.tracks[0].inserts.clear();
        for _ in 0..2 {
            h.frame(vec![], |ui| show(ui.ctx(), &mut p, &mut s));
        }
        assert_eq!(p.tracks[0].auto.shown, AutoTarget::Volume);
        // A track that goes away closes the editor.
        p.tracks.clear();
        h.frame(vec![], |ui| show(ui.ctx(), &mut p, &mut s));
        assert!(s.mixer.auto_editor.track.is_none());
    }
}
