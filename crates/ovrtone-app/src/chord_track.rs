//! The Chord track, shown above the tracks with Track > Show Chord Track: the song's chords, each
//! written where it starts and holding until the next. It is a chart to read and to keep with the
//! song: it makes no sound and changes no notes. Chords move and copy with arrangement sections.
//!
//! - Double-click an empty spot: a new chord there, with its name ready to type.
//! - Click a chord: the playhead goes to it. Drag it to move it (snapped to the grid).
//! - Double-click a chord, or right-click > Rename Chord: edit its name.
//! - Right-click > Delete Chord.
use crate::model::{ChordMark, Project};
use crate::widgets::context_menu;
use egui::{pos2, vec2, Align2, Color32, FontId, Rect, Sense, Stroke, Ui};

/// Height of the Chord track, in points.
pub const HEIGHT: f32 = 24.0;
/// The longest chord name.
const MAX_NAME: usize = 16;

/// Puts a chord at `beat`, or finds the one already there. Returns its place in the list.
pub fn add_chord(p: &mut Project, beat: f64) -> usize {
    let beat = beat.max(0.0);
    if let Some(i) = p.chords.iter().position(|c| (c.beat - beat).abs() < 1e-6) {
        return i;
    }
    let at = p.chords.partition_point(|c| c.beat < beat);
    // A new chord starts as what the last one was, or the song's key note.
    let name = at
        .checked_sub(1)
        .and_then(|k| p.chords.get(k))
        .map_or_else(|| key_note(p.key), |c| c.name.clone());
    p.chords.insert(at, ChordMark { beat, name });
    at
}

/// The note of a key like `Cmaj` or `F#min`.
fn key_note(key: &str) -> String {
    let mut chars = key.chars();
    let mut out = String::new();
    if let Some(root) = chars.next() {
        out.push(root);
    }
    if let Some(sign) = chars.next().filter(|c| matches!(c, '#' | 'b')) {
        out.push(sign);
    }
    if out.is_empty() {
        out.push('C');
    }
    out
}

/// A name as it is kept: trimmed and not too long. `None` if nothing is left.
fn clean_name(text: &str) -> Option<String> {
    let name: String = text.trim().chars().take(MAX_NAME).collect();
    (!name.is_empty()).then_some(name)
}

/// What a right-click or double-click asked for, run after drawing.
enum Action {
    Rename(usize),
    Delete(usize),
}

/// Draws the Chord track's `header` and `lane` and handles its editing. `snap` snaps a position.
pub fn show(
    ui: &mut Ui,
    p: &mut Project,
    header: Rect,
    lane: Rect,
    x_of: impl Fn(f64) -> f32,
    beat_of: impl Fn(f32) -> f64,
    snap: &dyn Fn(f64) -> f64,
) {
    let rename_id = egui::Id::unique("chord_track_rename");
    let grab_id = egui::Id::unique("chord_track_grab");
    let gh = ui.painter_at(header);
    gh.rect_filled(header, 0.0, crate::theme::panel_bg());
    gh.line_segment(
        [header.left_bottom(), header.right_bottom()],
        Stroke::new(1.0, crate::theme::border()),
    );
    gh.text(
        pos2(header.left() + 12.0, header.center().y),
        Align2::LEFT_CENTER,
        "Chords",
        FontId::proportional(13.0),
        crate::theme::gray(0xdc),
    );
    let g = ui.painter_at(lane);
    g.rect_filled(lane, 0.0, crate::theme::lane());
    let lane_resp = ui.interact(lane, egui::Id::unique("chord_track_lane"), Sense::click());
    if lane_resp.double_clicked() {
        if let Some(pt) = lane_resp.interact_pointer_pos() {
            let i = add_chord(p, snap(beat_of(pt.x)));
            let name = p.chords[i].name.clone();
            ui.ctx().data_mut(|m| m.insert_temp(rename_id, (i, name)));
        }
    }
    let mut action = None;
    let mut moved = false;
    let fill = Color32::from_rgb(0x4a, 0x5a, 0x6e);
    for i in 0..p.chords.len() {
        let start = p.chords[i].beat;
        let x0 = x_of(start);
        let x1 = p
            .chords
            .get(i + 1)
            .map_or(lane.right() + 1.0, |c| x_of(c.beat));
        if x1 < lane.left() || x0 > lane.right() {
            continue;
        }
        let block = Rect::from_min_max(pos2(x0, lane.top() + 2.0), pos2(x1, lane.bottom() - 2.0));
        let resp = ui.interact(
            block.intersect(lane),
            egui::Id::unique(("chord", i)),
            Sense::click_and_drag(),
        );
        if resp.clicked() {
            p.position_beats = start;
        }
        if resp.double_clicked() {
            action = Some(Action::Rename(i));
        }
        if resp.drag_started() {
            // From where the mouse went down: the drag only registers a few points later.
            let pressed = ui
                .input(|i| i.pointer.press_origin())
                .or_else(|| resp.interact_pointer_pos());
            if let Some(pt) = pressed {
                ui.ctx()
                    .data_mut(|m| m.insert_temp(grab_id, beat_of(pt.x) - start));
            }
        }
        if resp.dragged() {
            if let Some(pt) = resp.interact_pointer_pos() {
                let grab = ui.ctx().data(|m| m.get_temp::<f64>(grab_id)).unwrap_or(0.0);
                p.chords[i].beat = snap((beat_of(pt.x) - grab).max(0.0)).max(0.0);
            }
        }
        if resp.drag_stopped() {
            moved = true;
        }
        context_menu(&resp, |ui| {
            if ui.button("Rename Chord").clicked() {
                action = Some(Action::Rename(i));
                ui.close();
            }
            if ui.button("Delete Chord").clicked() {
                action = Some(Action::Delete(i));
                ui.close();
            }
        });
        g.rect_filled(block, 3.0, fill);
        g.line_segment(
            [pos2(x0, lane.top()), pos2(x0, lane.bottom())],
            Stroke::new(1.5, crate::theme::gray(0xc8)),
        );
        let text = g.layout_no_wrap(
            p.chords[i].name.clone(),
            FontId::proportional(12.0),
            Color32::WHITE,
        );
        let text_x = x0.max(lane.left()) + 5.0;
        if text_x + 6.0 < x1 {
            g.with_clip_rect(block.shrink(1.0).intersect(lane)).galley(
                pos2(text_x, block.center().y - text.size().y / 2.0),
                text,
                Color32::WHITE,
            );
        }
    }
    if moved {
        crate::arrangement_track::tidy(p);
    }
    match action {
        Some(Action::Rename(i)) => {
            let name = p.chords[i].name.clone();
            ui.ctx().data_mut(|m| m.insert_temp(rename_id, (i, name)));
        }
        Some(Action::Delete(i)) => {
            p.chords.remove(i);
        }
        None => {}
    }
    if let Some((i, mut text)) = ui.ctx().data(|m| m.get_temp::<(usize, String)>(rename_id)) {
        let Some(chord) = p.chords.get(i) else {
            ui.ctx()
                .data_mut(|m| m.remove::<(usize, String)>(rename_id));
            return;
        };
        let x0 = x_of(chord.beat).max(lane.left());
        let r = Rect::from_min_size(pos2(x0, lane.top() + 1.0), vec2(90.0, HEIGHT - 2.0));
        let edit = ui.put(
            r,
            egui::TextEdit::singleline(&mut text)
                .id(rename_id.with("field"))
                .char_limit(MAX_NAME)
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
            if let Some(name) = clean_name(&text) {
                p.chords[i].name = name;
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
        Stroke::new(1.0, crate::theme::gray(0x0e)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_stay_in_order_and_a_new_one_starts_as_the_one_before_it() {
        let mut p = Project::empty();
        p.chords.clear();
        p.key = "F#min";
        let a = add_chord(&mut p, 8.0);
        assert_eq!(p.chords[a].name, "F#", "the key's note to begin with");
        p.chords[a].name = "Am7".into();
        let b = add_chord(&mut p, 4.0);
        assert_eq!(b, 0, "in order");
        assert_eq!(p.chords[1].name, "Am7");
        let c = add_chord(&mut p, 12.0);
        assert_eq!(p.chords[c].name, "Am7", "as the one before it");
        assert_eq!(add_chord(&mut p, 12.0), c, "one per place");
        assert_eq!(add_chord(&mut p, -5.0), 0, "not before the song");
        assert_eq!(p.chords[0].beat, 0.0);
    }

    #[test]
    fn a_name_is_trimmed_and_cannot_be_empty_or_endless() {
        assert_eq!(clean_name("  Dm7b5 ").as_deref(), Some("Dm7b5"));
        assert_eq!(clean_name("   "), None);
        assert_eq!(
            clean_name(&"x".repeat(100)).map(|n| n.len()),
            Some(MAX_NAME)
        );
    }

    #[test]
    fn the_track_adds_moves_and_deletes_chords_with_the_mouse() {
        let mut p = Project::empty();
        p.chords.clear();
        p.position_beats = 100.0;
        let mut h = crate::ui_test::Harness::new(1000.0, 100.0);
        let header = Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, HEIGHT));
        let lane = Rect::from_min_size(pos2(100.0, 0.0), vec2(900.0, HEIGHT));
        macro_rules! run {
            () => {
                |ui: &mut Ui| {
                    show(
                        ui,
                        &mut p,
                        header,
                        lane,
                        |b| 100.0 + b as f32 * 10.0,
                        |x| f64::from(x - 100.0) / 10.0,
                        &|b: f64| b.round(),
                    );
                }
            };
        }
        h.settle(run!());
        // A double click on an empty spot: a chord on the grid.
        h.click(pos2(204.0, 12.0), run!());
        h.click(pos2(204.0, 12.0), run!());
        h.key(egui::Key::Escape, run!());
        assert_eq!(p.chords.len(), 1);
        assert_eq!(p.chords[0].beat, 10.0);
        // Dragged four beats later.
        h.drag(
            &[pos2(210.0, 12.0), pos2(230.0, 12.0), pos2(250.0, 12.0)],
            egui::PointerButton::Primary,
            run!(),
        );
        assert_eq!(p.chords[0].beat, 14.0);
        // A click takes the playhead there.
        h.click(pos2(260.0, 12.0), run!());
        assert_eq!(p.position_beats, 14.0);
    }
}
