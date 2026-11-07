//! The Arrangement track, shown above the tracks with Track > Show Arrangement Track: named sections
//! of the song (Intro, Verse, Chorus...). Moving a section takes everything in it along, the regions,
//! automation, markers and chords of every track, and the rest of the song closes up and makes room.
//! Copying it puts a copy in between, and what follows moves on.
//!
//! - Double-click an empty spot: a new section there.
//! - Click a section: the playhead goes to its start. Drag its right edge to make it longer.
//! - Drag a section: it moves to the bar it is let go on. With Ctrl held, a copy goes there.
//! - Right-click: rename, colour, move or copy to the playhead, or delete the section's name.
//!
//! The tempo and the signatures belong to the song's time, not to a section, and stay where they are.
use crate::model::{AutoPoint, AutoTarget, ChordMark, Marker, Project, Section, MARKER_COLORS};
use crate::widgets::context_menu;
use egui::{pos2, vec2, Align2, Color32, FontId, Rect, Sense, Stroke, Ui};

/// Height of the Arrangement track, in points.
pub const HEIGHT: f32 = 24.0;
/// How long a new section is, in bars, if nothing comes before that.
const NEW_SECTION_BARS: f64 = 8.0;
/// Closer than this, in beats, is the same place.
const NEAR: f64 = 1e-4;
/// How far before a section's edge a lane is pinned to the value it has there, in beats.
const PIN: f64 = 1e-3;
/// The width of the grip on a section's right edge.
const EDGE_GRIP: f32 = 6.0;

/// Sorted, with sections that make no sense left out and those that overlap cut short.
pub fn tidy(p: &mut Project) {
    p.sections
        .retain(|x| x.beat.is_finite() && x.beat >= 0.0 && x.len.is_finite() && x.len > 0.0);
    p.sections.sort_by(|a, b| a.beat.total_cmp(&b.beat));
    let starts: Vec<f64> = p.sections.iter().map(|x| x.beat).collect();
    for (i, section) in p.sections.iter_mut().enumerate() {
        if let Some(next) = starts.get(i + 1) {
            section.len = section.len.min(next - section.beat);
        }
    }
    p.sections.retain(|x| x.len > NEAR);
    p.chords.retain(|c| c.beat.is_finite() && c.beat >= 0.0);
    p.chords.sort_by(|a, b| a.beat.total_cmp(&b.beat));
}

/// Makes a section from `beat` on, up to the next one. Returns its place in the list; inside a
/// section it is that section.
pub fn add_section(p: &mut Project, beat: f64) -> usize {
    let beat = beat.max(0.0);
    if let Some(i) = p
        .sections
        .iter()
        .position(|x| beat >= x.beat - NEAR && beat < x.beat + x.len - NEAR)
    {
        return i;
    }
    let bar = p.beats_to_bar(beat);
    let mut end = p.bar_to_beats(bar + NEW_SECTION_BARS);
    if let Some(next) = p.sections.iter().find(|x| x.beat > beat) {
        end = end.min(next.beat);
    }
    let n = (1..)
        .find(|n| !p.sections.iter().any(|x| x.name == format!("Section {n}")))
        .unwrap_or(1);
    let at = p.sections.partition_point(|x| x.beat < beat);
    let color = MARKER_COLORS[p.sections.len() % MARKER_COLORS.len()];
    p.sections.insert(
        at,
        Section {
            beat,
            len: end - beat,
            name: format!("Section {n}"),
            color,
        },
    );
    at
}

/// Where section `i` can go for a pointer at `beat`: a bar line, and not in the middle of another
/// section, where it lands on the nearer edge. `None` inside the section itself.
pub fn landing(p: &Project, i: usize, beat: f64) -> Option<f64> {
    let own = p.sections.get(i)?;
    let mut to = p.sigs().nearest_bar_line(beat.max(0.0)).max(0.0);
    for (k, other) in p.sections.iter().enumerate() {
        let end = other.beat + other.len;
        if k != i && to > other.beat + NEAR && to < end - NEAR {
            to = if to - other.beat < end - to {
                other.beat
            } else {
                end
            };
        }
    }
    let inside = to > own.beat + NEAR && to < own.beat + own.len - NEAR;
    (!inside).then_some(to)
}

/// Runs `edit` on track `t` once for each version of its regions, with that version in
/// `Track::regions`.
fn each_version(
    q: &mut Project,
    t: usize,
    edit: &mut dyn FnMut(&mut Project) -> Result<(), String>,
) -> Result<(), String> {
    let active = q.tracks[t].active_alt;
    for k in 0..q.tracks[t].alternatives.len() {
        if k != active {
            let track = &mut q.tracks[t];
            std::mem::swap(&mut track.regions, &mut track.alternatives[k].regions);
            let done = edit(q);
            let track = &mut q.tracks[t];
            std::mem::swap(&mut track.regions, &mut track.alternatives[k].regions);
            done?;
        }
    }
    edit(q)
}

/// Cuts every region of track `t` that lies across `edge`.
fn cut_at(q: &mut Project, t: usize, edge: f64) -> Result<(), String> {
    const CUT: f64 = crate::region_edit::EDGE_BEATS;
    while let Some(r) = q.tracks[t]
        .regions
        .iter()
        .position(|x| q.region_start_beats(x) + CUT < edge && edge < q.region_end_beats(x) - CUT)
    {
        crate::region_edit::split(q, t, r, edge)?;
    }
    Ok(())
}

/// Gives a lane points at `edges`, so that what lies between them keeps its values when it moves.
fn pin_lane(points: &mut Vec<AutoPoint>, edges: &[f64]) {
    if points.is_empty() {
        return;
    }
    for &edge in edges {
        if points.len() + 2 > crate::model::Automation::MAX_POINTS {
            return;
        }
        let Some(value) = ovrtone_engine::lane_value(points, edge) else {
            continue;
        };
        for beat in [edge - PIN, edge] {
            if beat >= 0.0 && !points.iter().any(|q| (q.beat - beat).abs() < PIN / 2.0) {
                let at = points.partition_point(|q| q.beat < beat);
                points.insert(at, AutoPoint::new(beat, value));
            }
        }
    }
}

/// Every lane of a strip, by what it moves.
fn targets(q: &Project, t: usize) -> Vec<AutoTarget> {
    q.tracks[t].auto.lanes().map(|l| l.0).collect()
}

/// A region's first beat, after what `f` does to the song's beats, as its `start_bar`.
fn moved_regions(q: &mut Project, t: usize, f: &dyn Fn(f64) -> f64) {
    let bars: Vec<f32> = q.tracks[t]
        .regions
        .iter()
        .map(|x| q.beats_to_bar(f(q.region_start_beats(x))) as f32 + 1.0)
        .collect();
    for (region, bar) in q.tracks[t].regions.iter_mut().zip(bars) {
        region.start_bar = bar;
    }
}

/// Whether the song can be cut up now.
fn can_rearrange(p: &Project) -> Result<(), String> {
    if p.recording {
        return Err("Sections cannot move during a recording".to_string());
    }
    if p.tracks.iter().any(|t| t.frozen.is_some()) {
        return Err(
            "Unfreeze the tracks first: their files were made from the song as it is".into(),
        );
    }
    Ok(())
}

/// What Cut Section took: how long it was, and each region with its track and its place in it.
#[derive(Clone)]
pub struct SectionClip {
    len: f64,
    regions: Vec<(usize, crate::model::Region, f64)>,
}

/// Opens a gap of `by` beats at `at`, moving all that comes after along.
fn shift_from(q: &mut Project, at: f64, by: f64) -> Result<(), String> {
    let f = move |x: f64| if x >= at - NEAR { x + by } else { x };
    for t in 0..q.tracks.len() {
        each_version(q, t, &mut |q| {
            cut_at(q, t, at)?;
            moved_regions(q, t, &f);
            Ok(())
        })?;
        for target in targets(q, t) {
            let lane = q.tracks[t].auto.lane_mut(target);
            pin_lane(lane, &[at]);
            for point in lane.iter_mut() {
                point.beat = f(point.beat);
            }
            lane.sort_by(|x, y| x.beat.total_cmp(&y.beat));
        }
    }
    for marker in &mut q.markers {
        marker.beat = f(marker.beat);
    }
    q.tidy_markers();
    for chord in &mut q.chords {
        chord.beat = f(chord.beat);
    }
    for section in &mut q.sections {
        section.beat = f(section.beat);
    }
    tidy(q);
    Ok(())
}

/// Insert Silence: the stretch `a..b` of every track is pushed along, leaving nothing there.
pub fn insert_silence(p: &mut Project, a: f64, b: f64) -> Result<(), String> {
    can_rearrange(p)?;
    let mut q = p.clone();
    shift_from(&mut q, a, b - a)?;
    q.song_rev += 1;
    *p = q;
    Ok(())
}

/// Cut Section: takes the stretch `a..b` out of every track and closes the gap.
pub fn cut_section(p: &mut Project, a: f64, b: f64) -> Result<SectionClip, String> {
    can_rearrange(p)?;
    let (mut q, len) = (p.clone(), b - a);
    let mut regions = Vec::new();
    let inside = move |x: f64| x >= a - NEAR && x < b - NEAR;
    for t in 0..q.tracks.len() {
        cut_at(&mut q, t, a)?;
        cut_at(&mut q, t, b)?;
        for region in &q.tracks[t].regions {
            let start = q.region_start_beats(region);
            if inside(start) {
                regions.push((t, region.clone(), start - a));
            }
        }
        each_version(&mut q, t, &mut |q| {
            cut_at(q, t, a)?;
            cut_at(q, t, b)?;
            let keep: Vec<bool> = q.tracks[t]
                .regions
                .iter()
                .map(|x| !inside(q.region_start_beats(x)))
                .collect();
            let mut kept = keep.iter();
            q.tracks[t]
                .regions
                .retain(|_| *kept.next().unwrap_or(&true));
            moved_regions(q, t, &|x| if x >= b - NEAR { x - len } else { x });
            Ok(())
        })?;
        for target in targets(&q, t) {
            let lane = q.tracks[t].auto.lane_mut(target);
            pin_lane(lane, &[a, b]);
            lane.retain(|point| !(point.beat > a + NEAR && point.beat < b - NEAR));
            for point in lane.iter_mut() {
                if point.beat >= b - NEAR {
                    point.beat -= len;
                }
            }
            lane.sort_by(|x, y| x.beat.total_cmp(&y.beat));
        }
    }
    q.markers.retain(|m| !inside(m.beat));
    q.chords.retain(|c| !inside(c.beat));
    let close = move |x: f64| if x >= b - NEAR { x - len } else { x };
    for marker in &mut q.markers {
        marker.beat = close(marker.beat);
    }
    q.tidy_markers();
    for chord in &mut q.chords {
        chord.beat = close(chord.beat);
    }
    q.sections.retain(|s| !inside(s.beat));
    for section in &mut q.sections {
        section.beat = close(section.beat);
    }
    tidy(&mut q);
    q.song_rev += 1;
    *p = q;
    Ok(SectionClip { len, regions })
}

/// Insert Cut Section: puts what was cut back in at `at`, moving what comes after along. Its
/// automation stays out: only regions come back.
pub fn insert_clip(p: &mut Project, clip: &SectionClip, at: f64) -> Result<(), String> {
    can_rearrange(p)?;
    let mut q = p.clone();
    shift_from(&mut q, at, clip.len)?;
    for (t, region, rel) in &clip.regions {
        let start_bar = q.beats_to_bar(at + rel) as f32 + 1.0;
        if let Some(track) = q.tracks.get_mut(*t) {
            let mut region = region.clone();
            region.start_bar = start_bar;
            track.regions.push(region);
        }
    }
    q.song_rev += 1;
    *p = q;
    Ok(())
}

/// Repeat Section: `a..b` of every track comes again straight after itself, regions and curves.
pub fn repeat_section(p: &mut Project, a: f64, b: f64) -> Result<(), String> {
    can_rearrange(p)?;
    let (mut q, len) = (p.clone(), b - a);
    shift_from(&mut q, b, len)?;
    for t in 0..q.tracks.len() {
        cut_at(&mut q, t, a)?;
        let copies: Vec<crate::model::Region> = q.tracks[t]
            .regions
            .iter()
            .filter(|x| {
                let start = q.region_start_beats(x);
                start >= a - NEAR && start < b - NEAR
            })
            .map(|x| {
                let mut copy = x.clone();
                copy.start_bar = q.beats_to_bar(q.region_start_beats(x) + len) as f32 + 1.0;
                copy
            })
            .collect();
        q.tracks[t].regions.extend(copies);
        for target in targets(&q, t) {
            let lane = q.tracks[t].auto.lane_mut(target);
            let again: Vec<_> = lane
                .iter()
                .filter(|point| point.beat >= a - NEAR && point.beat < b - NEAR)
                .map(|point| {
                    let mut point = *point;
                    point.beat += len;
                    point
                })
                .collect();
            lane.extend(again);
            lane.sort_by(|x, y| x.beat.total_cmp(&y.beat));
        }
    }
    q.song_rev += 1;
    *p = q;
    Ok(())
}

/// Split by Locators: cuts the regions of these tracks at `a` and `b`.
pub fn split_by_locators(p: &mut Project, tracks: &[usize], a: f64, b: f64) -> Result<(), String> {
    can_rearrange(p)?;
    let mut q = p.clone();
    for &t in tracks {
        if t < q.tracks.len() {
            cut_at(&mut q, t, a)?;
            cut_at(&mut q, t, b)?;
        }
    }
    q.song_rev += 1;
    *p = q;
    Ok(())
}

/// What moving the stretch `a..b` to `d` does to a beat: it goes along, or what lay between the two
/// places moves over to close the gap and make room.
fn rotated(x: f64, a: f64, b: f64, d: f64) -> f64 {
    let len = b - a;
    if x >= a - NEAR && x < b - NEAR {
        if d <= a {
            d + (x - a)
        } else {
            d - len + (x - a)
        }
    } else if d <= a && x >= d - NEAR && x < a - NEAR {
        x + len
    } else if d >= b && x >= b - NEAR && x < d - NEAR {
        x - len
    } else {
        x
    }
}

/// Moves section `i` so that it starts at `to` (before something) or ends at `to` (after something),
/// with everything in it on every track. All of it happens, or none: a take folder across one of the
/// cuts stops it and says so.
pub fn move_section(p: &mut Project, i: usize, to: f64) -> Result<(), String> {
    can_rearrange(p)?;
    let section = p
        .sections
        .get(i)
        .ok_or_else(|| "That section is gone".to_string())?;
    let (a, b) = (section.beat, section.beat + section.len);
    if to > a + NEAR && to < b - NEAR {
        return Err("A section cannot move inside itself".to_string());
    }
    if (to - a).abs() <= NEAR || (to - b).abs() <= NEAR {
        return Ok(());
    }
    let mut q = p.clone();
    let f = move |x: f64| rotated(x, a, b, to);
    for t in 0..q.tracks.len() {
        each_version(&mut q, t, &mut |q| {
            for edge in [a, b, to] {
                cut_at(q, t, edge)?;
            }
            moved_regions(q, t, &f);
            Ok(())
        })?;
        for target in targets(&q, t) {
            let lane = q.tracks[t].auto.lane_mut(target);
            pin_lane(lane, &[a, b, to]);
            for point in lane.iter_mut() {
                point.beat = f(point.beat);
            }
            lane.sort_by(|x, y| x.beat.total_cmp(&y.beat));
        }
    }
    for marker in &mut q.markers {
        marker.beat = f(marker.beat);
    }
    q.tidy_markers();
    for chord in &mut q.chords {
        chord.beat = f(chord.beat);
    }
    for section in &mut q.sections {
        section.beat = f(section.beat);
    }
    tidy(&mut q);
    q.song_rev += 1;
    *p = q;
    Ok(())
}

/// Puts a copy of section `i` at `to`, with everything in it on every track. What was at `to` and
/// after moves on by the section's length. All of it happens, or none.
pub fn copy_section(p: &mut Project, i: usize, to: f64) -> Result<(), String> {
    can_rearrange(p)?;
    let section = p
        .sections
        .get(i)
        .ok_or_else(|| "That section is gone".to_string())?
        .clone();
    let (a, b, len) = (section.beat, section.beat + section.len, section.len);
    if to > a + NEAR && to < b - NEAR {
        return Err("A section cannot be copied inside itself".to_string());
    }
    let mut q = p.clone();
    let inside = |x: f64| x >= a - NEAR && x < b - NEAR;
    let there = |x: f64| to + (x - a);
    let on = |x: f64| if x >= to - NEAR { x + len } else { x };
    for t in 0..q.tracks.len() {
        each_version(&mut q, t, &mut |q| {
            for edge in [a, b, to] {
                cut_at(q, t, edge)?;
            }
            let mut copies = Vec::new();
            for region in &q.tracks[t].regions {
                let start = q.region_start_beats(region);
                if inside(start) {
                    let mut copy = region.clone();
                    copy.start_bar = q.beats_to_bar(there(start)) as f32 + 1.0;
                    copies.push(copy);
                }
            }
            moved_regions(q, t, &on);
            // Now the bars after the copy's place are free, in the song's own bar lengths.
            q.tracks[t].regions.extend(copies);
            Ok(())
        })?;
        for target in targets(&q, t) {
            let lane = q.tracks[t].auto.lane_mut(target);
            pin_lane(lane, &[a, b, to]);
            let copies: Vec<AutoPoint> = lane
                .iter()
                .filter(|point| inside(point.beat))
                .map(|point| AutoPoint {
                    beat: there(point.beat),
                    ..*point
                })
                .collect();
            for point in lane.iter_mut() {
                point.beat = on(point.beat);
            }
            lane.extend(copies);
            lane.sort_by(|x, y| x.beat.total_cmp(&y.beat));
            lane.truncate(crate::model::Automation::MAX_POINTS);
        }
    }
    let markers: Vec<Marker> = q
        .markers
        .iter()
        .filter(|m| inside(m.beat))
        .map(|m| Marker {
            beat: there(m.beat),
            ..m.clone()
        })
        .collect();
    for marker in &mut q.markers {
        marker.beat = on(marker.beat);
    }
    q.markers.extend(markers);
    q.tidy_markers();
    let chords: Vec<ChordMark> = q
        .chords
        .iter()
        .filter(|c| inside(c.beat))
        .map(|c| ChordMark {
            beat: there(c.beat),
            ..c.clone()
        })
        .collect();
    for chord in &mut q.chords {
        chord.beat = on(chord.beat);
    }
    q.chords.extend(chords);
    for other in &mut q.sections {
        other.beat = on(other.beat);
    }
    q.sections.push(Section {
        beat: to,
        ..section
    });
    tidy(&mut q);
    q.song_rev += 1;
    *p = q;
    Ok(())
}

/// A drag on a section: moving it, or its right edge.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Grab {
    Section(usize, f64),
    Edge(usize),
}

/// What a right-click or double-click asked for, run after drawing.
enum Action {
    Rename(usize),
    Color(usize, Color32),
    Delete(usize),
    MoveHere(usize),
    CopyHere(usize),
    Land(usize, f64, bool),
}

/// Draws the Arrangement track's `header` and `lane` and handles its editing. Returns the result of a
/// section that moved or was copied, as region numbers then mean other regions.
pub fn show(
    ui: &mut Ui,
    p: &mut Project,
    header: Rect,
    lane: Rect,
    x_of: impl Fn(f64) -> f32,
    beat_of: impl Fn(f32) -> f64,
) -> Option<Result<(), String>> {
    let rename_id = egui::Id::unique("arrangement_rename");
    let grab_id = egui::Id::unique("arrangement_grab");
    let gh = ui.painter_at(header);
    gh.rect_filled(header, 0.0, crate::theme::PANEL_BG);
    gh.line_segment(
        [header.left_bottom(), header.right_bottom()],
        Stroke::new(1.0, crate::theme::BORDER),
    );
    gh.text(
        pos2(header.left() + 12.0, header.center().y),
        Align2::LEFT_CENTER,
        "Arrangement",
        FontId::proportional(13.0),
        Color32::from_gray(0xdc),
    );
    let g = ui.painter_at(lane);
    g.rect_filled(lane, 0.0, crate::theme::LANE);
    let lane_resp = ui.interact(lane, egui::Id::unique("arrangement_lane"), Sense::click());
    if lane_resp.double_clicked() {
        if let Some(pt) = lane_resp.interact_pointer_pos() {
            let beat = p.sigs().nearest_bar_line(beat_of(pt.x)).max(0.0);
            let i = add_section(p, beat);
            let name = p.sections[i].name.clone();
            ui.ctx().data_mut(|m| m.insert_temp(rename_id, (i, name)));
        }
    }
    let ctrl = ui.input(|i| i.modifiers.command);
    let grabbed = ui.ctx().data(|m| m.get_temp::<Grab>(grab_id));
    let mut action = None;
    for i in 0..p.sections.len() {
        let (start, len) = (p.sections[i].beat, p.sections[i].len);
        let (x0, x1) = (x_of(start), x_of(start + len));
        if x1 < lane.left() || x0 > lane.right() {
            continue;
        }
        let block = Rect::from_min_max(pos2(x0, lane.top() + 2.0), pos2(x1, lane.bottom() - 2.0));
        let resp = ui.interact(
            block.intersect(lane),
            egui::Id::unique(("arrangement_section", i)),
            Sense::click_and_drag(),
        );
        if resp.clicked() {
            p.position_beats = start;
        }
        if resp.double_clicked() {
            action = Some(Action::Rename(i));
        }
        let grip = Rect::from_min_max(pos2(x1 - EDGE_GRIP, block.top()), block.max);
        if resp.drag_started() {
            let origin = ui.input(|m| m.pointer.press_origin());
            if let Some(pt) = origin.or(resp.interact_pointer_pos()) {
                let grab = if grip.contains(pt) {
                    Grab::Edge(i)
                } else {
                    Grab::Section(i, beat_of(pt.x) - start)
                };
                ui.ctx().data_mut(|m| m.insert_temp(grab_id, grab));
            }
        }
        if resp.dragged() {
            if let Some(pt) = resp.interact_pointer_pos() {
                match grabbed {
                    Some(Grab::Edge(k)) if k == i => {
                        let end = p.sigs().nearest_bar_line(beat_of(pt.x));
                        let limit = p.sections.get(i + 1).map_or(f64::MAX, |x| x.beat);
                        let first = p.bar_to_beats(p.beats_to_bar(start) + 1.0);
                        p.sections[i].len = end.max(first).min(limit) - start;
                    }
                    Some(Grab::Section(k, _)) if k == i => {
                        if let Some(to) = landing(p, i, beat_of(pt.x)) {
                            let x = x_of(to);
                            g.line_segment(
                                [pos2(x, lane.top()), pos2(x, lane.bottom())],
                                Stroke::new(2.0, crate::theme::ACCENT_BLUE),
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
        if resp.drag_stopped() {
            if let (Some(Grab::Section(k, _)), Some(pt)) =
                (grabbed, ui.input(|m| m.pointer.latest_pos()))
            {
                if k == i {
                    if let Some(to) = landing(p, i, beat_of(pt.x)) {
                        action = Some(Action::Land(i, to, ctrl));
                    }
                }
            }
            ui.ctx().data_mut(|m| m.remove::<Grab>(grab_id));
        }
        if resp.hovered()
            && ui
                .input(|m| m.pointer.hover_pos())
                .is_some_and(|q| grip.contains(q))
        {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        context_menu(&resp, |ui| {
            if ui.button("Rename Section").clicked() {
                action = Some(Action::Rename(i));
                ui.close();
            }
            ui.horizontal(|ui| {
                for c in MARKER_COLORS {
                    let (r, sw) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
                    ui.painter().rect_filled(r.shrink(1.0), 3.0, c);
                    if sw.clicked() {
                        action = Some(Action::Color(i, c));
                    }
                }
            });
            ui.separator();
            if ui.button("Move Section to Playhead").clicked() {
                action = Some(Action::MoveHere(i));
                ui.close();
            }
            if ui.button("Copy Section to Playhead").clicked() {
                action = Some(Action::CopyHere(i));
                ui.close();
            }
            ui.separator();
            if ui.button("Delete Section Name").clicked() {
                action = Some(Action::Delete(i));
                ui.close();
            }
        });
        let section = &p.sections[i];
        g.rect_filled(block, 3.0, section.color.gamma_multiply(0.7));
        g.line_segment(
            [pos2(x0, lane.top()), pos2(x0, lane.bottom())],
            Stroke::new(1.5, section.color),
        );
        let text = g.layout_no_wrap(
            section.name.clone(),
            FontId::proportional(12.0),
            Color32::WHITE,
        );
        let text_x = x0.max(lane.left()) + 5.0;
        if text_x + 10.0 < x1 {
            g.with_clip_rect(block.shrink(1.0).intersect(lane)).galley(
                pos2(text_x, block.center().y - text.size().y / 2.0),
                text,
                Color32::WHITE,
            );
        }
    }
    let mut result = None;
    let at_playhead = |p: &Project, i: usize| landing(p, i, p.position_beats);
    match action {
        Some(Action::Rename(i)) => {
            let name = p.sections[i].name.clone();
            ui.ctx().data_mut(|m| m.insert_temp(rename_id, (i, name)));
        }
        Some(Action::Color(i, c)) => p.sections[i].color = c,
        Some(Action::Delete(i)) => {
            p.sections.remove(i);
        }
        Some(Action::MoveHere(i)) => {
            result = Some(match at_playhead(p, i) {
                Some(to) => move_section(p, i, to),
                None => Err("The playhead is inside this section".to_string()),
            });
        }
        Some(Action::CopyHere(i)) => {
            result = Some(match at_playhead(p, i) {
                Some(to) => copy_section(p, i, to),
                None => Err("The playhead is inside this section".to_string()),
            });
        }
        Some(Action::Land(i, to, copy)) => {
            result = Some(if copy {
                copy_section(p, i, to)
            } else {
                move_section(p, i, to)
            });
        }
        None => {}
    }
    if let Some((i, mut text)) = ui.ctx().data(|m| m.get_temp::<(usize, String)>(rename_id)) {
        let Some(section) = p.sections.get(i) else {
            ui.ctx()
                .data_mut(|m| m.remove::<(usize, String)>(rename_id));
            return result;
        };
        let x0 = x_of(section.beat).max(lane.left());
        let r = Rect::from_min_size(pos2(x0, lane.top() + 1.0), vec2(160.0, HEIGHT - 2.0));
        let edit = ui.put(
            r,
            egui::TextEdit::singleline(&mut text)
                .id(rename_id.with("field"))
                .char_limit(64)
                .font(FontId::proportional(12.0)),
        );
        if !edit.has_focus() && !edit.lost_focus() {
            edit.request_focus();
        }
        let (enter, escape) = ui.input(|m| {
            (
                m.key_pressed(egui::Key::Enter),
                m.key_pressed(egui::Key::Escape),
            )
        });
        if escape {
            ui.ctx()
                .data_mut(|m| m.remove::<(usize, String)>(rename_id));
        } else if enter || edit.lost_focus() {
            let name = text.trim();
            if !name.is_empty() {
                p.sections[i].name = name.to_string();
            }
            ui.ctx()
                .data_mut(|m| m.remove::<(usize, String)>(rename_id));
        } else {
            ui.ctx().data_mut(|m| m.insert_temp(rename_id, (i, text)));
        }
    }
    let x = x_of(p.position_beats);
    if x >= lane.left() && x <= lane.right() {
        g.line_segment(
            [pos2(x, lane.top()), pos2(x, lane.bottom())],
            Stroke::new(1.0, Color32::WHITE),
        );
    }
    g.line_segment(
        [lane.left_bottom(), lane.right_bottom()],
        Stroke::new(1.0, Color32::from_gray(0x1a)),
    );
    result
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn sections_open_close_repeat_and_come_back() {
        let mut p = Project::empty();
        p.tracks.clear();
        let t = p.add_track(crate::model::TrackKind::Instrument);
        p.add_midi_region(t, 0.0);
        p.add_midi_region(t, 8.0);
        let starts = |p: &Project| {
            let mut all: Vec<i64> = p.tracks[0]
                .regions
                .iter()
                .map(|r| p.region_start_beats(r).round() as i64)
                .collect();
            all.sort_unstable();
            all
        };
        let second = starts(&p)[1];
        insert_silence(&mut p, 16.0, 24.0).unwrap();
        assert_eq!(starts(&p)[1], second + 8);
        let clip = cut_section(&mut p, 16.0, 24.0).unwrap();
        assert_eq!(starts(&p)[1], second, "the gap closes");
        assert!(clip.regions.is_empty(), "nothing was in the gap");
        let clip = cut_section(
            &mut p,
            f64::from(second as i32),
            f64::from(second as i32) + 4.0,
        )
        .unwrap();
        assert_eq!(clip.regions.len(), 1);
        insert_clip(&mut p, &clip, f64::from(second as i32)).unwrap();
        assert!(starts(&p).contains(&second), "it comes back");
        let before = p.tracks[0].regions.len();
        repeat_section(&mut p, 0.0, 4.0).unwrap();
        assert!(p.tracks[0].regions.len() > before);
    }
    use crate::automation::lane_label;

    /// The automation lanes that have points.
    fn lanes_with_points(p: &Project, t: usize) -> Vec<String> {
        let track = &p.tracks[t];
        track
            .auto
            .lanes()
            .filter(|l| !l.1.is_empty())
            .map(|l| lane_label(track, l.0))
            .collect()
    }
    use crate::model::{Note, Take, TakeFolder, TrackKind};

    /// 4/4 at 120 BPM. Sections A (bars 1-2), B (bars 3-4), C (bars 5-6), and on an instrument track a
    /// region of two bars at the start of each, its note saying which it is.
    fn song() -> (Project, usize) {
        let mut p = Project::empty();
        p.tempo = 120.0;
        p.tracks.clear();
        p.markers.clear();
        p.chords.clear();
        let t = p.add_track(TrackKind::Instrument);
        for (i, name) in ["A", "B", "C"].into_iter().enumerate() {
            let beat = i as f64 * 8.0;
            let r = p.add_midi_region(t, beat as f32 / 4.0 + 1.0).unwrap();
            let region = &mut p.tracks[t].regions[r];
            region.len_bars = 2.0;
            region.notes = vec![Note {
                start: 0.0,
                len: 1.0,
                pitch: 60 + i as u8,
                vel: 100,
                muted: false,
            }];
            let at = add_section(&mut p, beat);
            p.sections[at].name = name.into();
            p.sections[at].len = 8.0;
        }
        (p, t)
    }

    /// What plays on the track, in order: (start bar, the pitch of its first note).
    fn plays(p: &Project, t: usize) -> Vec<(f32, u8)> {
        let mut out: Vec<(f32, u8)> = p.tracks[t]
            .regions
            .iter()
            .map(|r| (r.start_bar, r.notes[0].pitch))
            .collect();
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        out
    }

    fn names(p: &Project) -> Vec<(f64, &str)> {
        p.sections
            .iter()
            .map(|x| (x.beat, x.name.as_str()))
            .collect()
    }

    #[test]
    fn a_section_moved_earlier_takes_its_regions_and_the_rest_makes_room() {
        let (mut p, t) = song();
        let rev = p.song_rev;
        move_section(&mut p, 2, 0.0).unwrap();
        assert!(p.song_rev > rev, "the engine hears about it");
        assert_eq!(plays(&p, t), [(1.0, 62), (3.0, 60), (5.0, 61)]);
        assert_eq!(names(&p), [(0.0, "C"), (8.0, "A"), (16.0, "B")]);
    }

    #[test]
    fn a_section_moved_later_ends_where_it_was_let_go_and_the_gap_closes() {
        let (mut p, t) = song();
        move_section(&mut p, 0, 24.0).unwrap();
        assert_eq!(plays(&p, t), [(1.0, 61), (3.0, 62), (5.0, 60)]);
        assert_eq!(names(&p), [(0.0, "B"), (8.0, "C"), (16.0, "A")]);
    }

    #[test]
    fn moving_a_section_to_its_own_edges_or_inside_it_changes_nothing() {
        let (mut p, t) = song();
        let before = plays(&p, t);
        move_section(&mut p, 1, 8.0).unwrap();
        move_section(&mut p, 1, 16.0).unwrap();
        assert!(move_section(&mut p, 1, 12.0).is_err());
        assert!(move_section(&mut p, 9, 0.0).is_err());
        assert_eq!(plays(&p, t), before);
        assert_eq!(landing(&p, 1, 12.0), None, "inside itself");
        assert_eq!(landing(&p, 1, 1.0), Some(0.0), "the nearest bar line");
        assert_eq!(landing(&p, 1, 5.0), Some(8.0), "not inside another section");
    }

    #[test]
    fn regions_across_a_section_edge_are_cut_there_on_every_track() {
        let (mut p, t) = song();
        let other = p.add_track(TrackKind::Instrument);
        let r = p.add_midi_region(other, 2.0).unwrap();
        let long = &mut p.tracks[other].regions[r];
        // From bar 2 to bar 6, with a note in bar 2 and one in bar 5.
        long.len_bars = 4.0;
        long.notes = vec![
            Note {
                start: 1.0,
                len: 1.0,
                pitch: 40,
                vel: 100,
                muted: false,
            },
            Note {
                start: 13.0,
                len: 1.0,
                pitch: 50,
                vel: 100,
                muted: false,
            },
        ];
        // C goes first.
        move_section(&mut p, 2, 0.0).unwrap();
        assert_eq!(plays(&p, t)[0], (1.0, 62));
        let mut parts: Vec<(f32, f32, usize)> = p.tracks[other]
            .regions
            .iter()
            .map(|x| (x.start_bar, x.len_bars, x.notes.len()))
            .collect();
        parts.sort_by(|a, b| a.0.total_cmp(&b.0));
        // The part in C (one bar with one note) came along; the part before it moved on by two bars.
        assert_eq!(parts, [(1.0, 1.0, 1), (4.0, 3.0, 1)]);
    }

    #[test]
    fn automation_markers_and_chords_go_with_the_section_and_keep_their_values() {
        let (mut p, t) = song();
        p.tracks[t].auto.set_point(AutoTarget::Volume, 0.0, 0.0);
        p.tracks[t].auto.set_point(AutoTarget::Volume, 8.0, -8.0);
        p.tracks[t].auto.set_point(AutoTarget::Volume, 16.0, -16.0);
        p.tracks[t].auto.set_point(AutoTarget::Volume, 24.0, -24.0);
        let value_at = |p: &Project, beat: f64| {
            ovrtone_engine::lane_value(&p.tracks[t].auto.volume, beat).unwrap()
        };
        let halfway = value_at(&p, 12.0);
        let m = p.add_marker(12.0);
        p.markers[m].name = "In B".into();
        p.chords.push(ChordMark {
            beat: 12.0,
            name: "Am".into(),
        });
        p.chords.push(ChordMark {
            beat: 20.0,
            name: "F".into(),
        });
        // B goes to the very start.
        move_section(&mut p, 1, 0.0).unwrap();
        assert!(
            (value_at(&p, 4.0) - halfway).abs() < 0.01,
            "the value moved"
        );
        assert!(p.tracks[t]
            .auto
            .volume
            .windows(2)
            .all(|w| w[0].beat <= w[1].beat));
        assert_eq!(p.markers[0].beat, 4.0);
        assert_eq!(p.chords[0].beat, 4.0);
        assert_eq!(p.chords[0].name, "Am");
        assert_eq!(p.chords[1].beat, 20.0, "in C: it stays");
        assert_eq!(lanes_with_points(&p, t).len(), 1);
    }

    #[test]
    fn a_copy_goes_in_between_with_its_regions_and_what_follows_moves_on() {
        let (mut p, t) = song();
        p.chords.push(ChordMark {
            beat: 8.0,
            name: "G".into(),
        });
        p.tracks[t].auto.set_point(AutoTarget::Pan, 8.0, 0.5);
        p.tracks[t].auto.set_point(AutoTarget::Pan, 16.0, -0.5);
        // B again, after C.
        copy_section(&mut p, 1, 24.0).unwrap();
        assert_eq!(plays(&p, t), [(1.0, 60), (3.0, 61), (5.0, 62), (7.0, 61)]);
        assert_eq!(
            names(&p),
            [(0.0, "A"), (8.0, "B"), (16.0, "C"), (24.0, "B")]
        );
        assert_eq!(
            p.chords.iter().map(|c| c.beat).collect::<Vec<_>>(),
            [8.0, 24.0]
        );
        let pan = |b: f64| ovrtone_engine::lane_value(&p.tracks[t].auto.pan, b).unwrap();
        assert!((pan(24.0) - pan(8.0)).abs() < 0.01);
        // A copy before the original pushes the original on.
        let (mut p, t) = song();
        copy_section(&mut p, 2, 0.0).unwrap();
        assert_eq!(plays(&p, t), [(1.0, 62), (3.0, 60), (5.0, 61), (7.0, 62)]);
        assert!(copy_section(&mut p, 0, 2.0).is_err(), "inside itself");
    }

    #[test]
    fn every_version_of_a_track_moves_along() {
        let (mut p, t) = song();
        crate::alternatives::run(&mut p, t, crate::alternatives::AltCmd::Duplicate).unwrap();
        move_section(&mut p, 2, 0.0).unwrap();
        assert_eq!(plays(&p, t), [(1.0, 62), (3.0, 60), (5.0, 61)]);
        crate::alternatives::run(&mut p, t, crate::alternatives::AltCmd::Switch(0)).unwrap();
        assert_eq!(plays(&p, t), [(1.0, 62), (3.0, 60), (5.0, 61)]);
    }

    #[test]
    fn a_take_folder_across_a_cut_stops_it_and_nothing_has_changed() {
        let (mut p, t) = song();
        let r = p.add_midi_region(t, 2.0).unwrap();
        let region = &mut p.tracks[t].regions[r];
        region.len_bars = 2.0;
        let take = |pitch: u8| Take {
            notes: vec![Note {
                start: 0.0,
                len: 1.0,
                pitch,
                vel: 100,
                muted: false,
            }],
            ..Take::default()
        };
        region.folder = Some(TakeFolder::new(vec![take(1), take(2)], 8.0));
        region.notes = vec![Note {
            start: 0.0,
            len: 1.0,
            pitch: 2,
            vel: 100,
            muted: false,
        }];
        let before = plays(&p, t);
        assert!(move_section(&mut p, 1, 0.0).is_err());
        assert_eq!(plays(&p, t), before);
        assert_eq!(names(&p)[0], (0.0, "A"));
    }

    #[test]
    fn nothing_moves_while_recording_or_with_a_frozen_track() {
        let (mut p, t) = song();
        p.recording = true;
        assert!(move_section(&mut p, 2, 0.0).is_err());
        assert!(copy_section(&mut p, 2, 0.0).is_err());
        p.recording = false;
        p.tracks[t].frozen = Some(Box::new(p.tracks[t].regions[0].clone()));
        assert!(move_section(&mut p, 2, 0.0).is_err());
    }

    #[test]
    fn audio_cut_by_a_section_keeps_playing_the_right_part_of_its_file() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("long.wav");
        // Twelve seconds at 120 BPM: six bars.
        ovrtone_engine::write_wav(&wav, &vec![0.1; 48_000 * 2 * 12], 48_000).unwrap();
        let (mut p, _) = song();
        let ctx = egui::Context::default();
        crate::import::import_audio_at(&mut p, &wav, &ctx, 1.0, None).unwrap();
        let t = p.tracks.len() - 1;
        move_section(&mut p, 2, 0.0).unwrap();
        let mut parts: Vec<(f32, f32)> = p.tracks[t]
            .regions
            .iter()
            .map(|r| (r.start_bar, r.params.source_start))
            .collect();
        parts.sort_by(|a, b| a.0.total_cmp(&b.0));
        // Bars 5-6 are the file's seconds 8 to 12, and now come first.
        assert_eq!(parts, [(1.0, 8.0), (3.0, 0.0)]);
    }

    #[test]
    fn sections_are_added_up_to_the_next_and_tidied() {
        let mut p = Project::empty();
        let a = add_section(&mut p, 0.0);
        assert_eq!(p.sections[a].len, 32.0, "eight bars");
        let b = add_section(&mut p, 8.0);
        assert_eq!(b, a, "inside it: that section");
        p.sections[0].len = 8.0;
        let c = add_section(&mut p, 16.0);
        p.sections[c].name = "Later".into();
        let d = add_section(&mut p, 8.0);
        assert_eq!(p.sections[d].len, 8.0, "up to the next");
        p.sections.push(Section {
            beat: f64::NAN,
            len: 4.0,
            name: "Bad".into(),
            color: Color32::WHITE,
        });
        p.sections.push(Section {
            beat: 4.0,
            len: 100.0,
            name: "Over".into(),
            color: Color32::WHITE,
        });
        tidy(&mut p);
        assert!(p.sections.iter().all(|x| x.name != "Bad"));
        assert!(p
            .sections
            .windows(2)
            .all(|w| w[0].beat + w[0].len <= w[1].beat + 1e-9));
    }

    #[test]
    fn dragging_a_section_on_the_track_moves_it_and_with_ctrl_copies_it() {
        let (mut p, t) = song();
        let mut last = None;
        let mut h = crate::ui_test::Harness::new(1000.0, 100.0);
        let header = Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, HEIGHT));
        let lane = Rect::from_min_size(pos2(100.0, 0.0), vec2(900.0, HEIGHT));
        // Ten points a beat.
        macro_rules! run {
            () => {
                |ui: &mut Ui| {
                    let got = show(
                        ui,
                        &mut p,
                        header,
                        lane,
                        |b| 100.0 + b as f32 * 10.0,
                        |x| f64::from(x - 100.0) / 10.0,
                    );
                    if got.is_some() {
                        last = got;
                    }
                }
            };
        }
        h.settle(run!());
        // C (beats 16..24) dragged to the start of the song.
        h.drag(
            &[pos2(300.0, 12.0), pos2(200.0, 12.0), pos2(101.0, 12.0)],
            egui::PointerButton::Primary,
            run!(),
        );
        assert_eq!(last, Some(Ok(())));
        assert_eq!(plays(&p, t), [(1.0, 62), (3.0, 60), (5.0, 61)]);
        // With Ctrl: the first section copied after the last.
        h.modifiers = egui::Modifiers::COMMAND;
        h.drag(
            &[pos2(140.0, 12.0), pos2(300.0, 12.0), pos2(340.0, 12.0)],
            egui::PointerButton::Primary,
            run!(),
        );
        h.modifiers = egui::Modifiers::NONE;
        assert_eq!(plays(&p, t), [(1.0, 62), (3.0, 60), (5.0, 61), (7.0, 62)]);
        // A section's right edge makes it longer, up to the next one.
        p.sections.remove(3);
        p.sections[2].len = 8.0;
        h.settle(run!());
        h.drag(
            &[pos2(338.0, 12.0), pos2(380.0, 12.0), pos2(420.0, 12.0)],
            egui::PointerButton::Primary,
            run!(),
        );
        assert_eq!(p.sections[2].len, 16.0);
    }
}
