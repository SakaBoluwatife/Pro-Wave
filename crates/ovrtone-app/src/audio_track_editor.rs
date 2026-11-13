//! Audio Track Editor (Logic: Audio Track Editor): the selected audio region in the bottom pane,
//! its whole file drawn across the width with the part it plays lit.
//!
//! Drag the lower handles at either end to trim the region's start or end (the audio stays where
//! it is in the song), the top handles to set its fades, click to move the playhead there. The
//! bar above holds the region's gain, reverse and Fit to Tempo, and opens the Audio File Editor.

use crate::model::{Project, TrackKind};
use crate::region_edit::{played_seconds, trim_end, trim_start, TrimStart};
use crate::widgets::{text_col, text_dim_col};
use crate::UiState;
use egui::{pos2, vec2, Align2, Color32, FontId, Rect, Sense, Stroke, StrokeKind, Ui};
use ovrtone_engine::FlexMode;
use std::cell::RefCell;

const BAR_H: f32 = 30.0;
const RULER_H: f32 = 18.0;
const HANDLE: f32 = 9.0;
const PAD: f32 = 12.0;

enum Grab {
    Start(TrimStart),
    End,
    FadeIn,
    FadeOut,
}

thread_local! {
    static GRAB: RefCell<Option<Grab>> = const { RefCell::new(None) };
}

/// The audio region to show on track `t`: the selected one, else the one under the playhead,
/// else the first.
fn region_on(p: &Project, s: &UiState, t: usize) -> Option<usize> {
    let track = p.tracks.get(t)?;
    if let Some((st, sr)) = s.tracks.selected {
        if st == t && track.regions.get(sr).is_some_and(|r| r.audio.is_some()) {
            return Some(sr);
        }
    }
    let at = p.position_beats;
    track
        .regions
        .iter()
        .position(|r| {
            r.audio.is_some() && p.region_start_beats(r) <= at && at < p.region_end_beats(r)
        })
        .or_else(|| track.regions.iter().position(|r| r.audio.is_some()))
}

/// Whether the bottom editor should show this instead of the Piano Roll.
pub fn wants(p: &Project) -> bool {
    p.tracks
        .get(p.selected_track)
        .is_some_and(|t| t.kind == TrackKind::Audio)
}

pub fn show(ui: &mut Ui, p: &mut Project, s: &mut UiState, area: Rect) {
    let g = ui.painter_at(area);
    g.rect_filled(area, 0.0, crate::theme::panel_bg());
    let ti = p.selected_track;
    let bar = Rect::from_min_size(area.min, vec2(area.width(), BAR_H));
    g.text(
        bar.left_center() + vec2(10.0, 0.0),
        Align2::LEFT_CENTER,
        "Audio Track Editor",
        FontId::proportional(12.0),
        crate::theme::accent(),
    );
    let Some(ri) = region_on(p, s, ti) else {
        g.text(
            area.center(),
            Align2::CENTER_CENTER,
            "No audio region on this track yet: record, or drag a file in.",
            FontId::proportional(13.0),
            text_dim_col(),
        );
        return;
    };
    toolbar(ui, p, s, bar, ti, ri);
    let body = Rect::from_min_max(pos2(area.left(), bar.bottom()), area.max);
    wave(ui, p, s, body, ti, ri);
}

fn toolbar(ui: &mut Ui, p: &mut Project, s: &mut UiState, bar: Rect, ti: usize, ri: usize) {
    let inner = Rect::from_min_max(pos2(bar.left() + 140.0, bar.top() + 3.0), bar.max - vec2(8.0, 3.0));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let ui = &mut child;
    let region = &p.tracks[ti].regions[ri];
    let following = region.audio.as_ref().and_then(|a| a.follow_bpm);
    let mode = region.audio.as_ref().and_then(|a| a.flex.as_ref()).map(|f| f.mode);
    let mut gain = region.params.gain_db;
    let mut reverse = region.params.reverse;
    let mut changed = false;

    ui.label(egui::RichText::new(&region.name).color(text_col()).strong());
    ui.add_space(10.0);
    ui.label("Gain");
    if ui
        .add(
            egui::DragValue::new(&mut gain)
                .range(crate::model::MIN_REGION_GAIN..=crate::model::MAX_REGION_GAIN)
                .speed(0.1)
                .suffix(" dB"),
        )
        .changed()
    {
        changed = true;
    }
    if ui.checkbox(&mut reverse, "Reverse").changed() {
        changed = true;
    }
    ui.add_space(10.0);
    match following {
        Some(bpm) => {
            ui.label(format!("Fit to tempo from {bpm:.1} BPM"));
            let stretch = mode != Some(FlexMode::Speed);
            if ui.selectable_label(stretch, "Stretch").on_hover_text("The pitch stays as it is").clicked() && !stretch {
                crate::flex_edit::set_mode(p, ti, ri, FlexMode::Complex);
                p.song_rev += 1;
            }
            if ui
                .selectable_label(!stretch, "Resample")
                .on_hover_text("Faster is higher, slower is lower, like a turntable")
                .clicked()
                && stretch
            {
                crate::flex_edit::set_mode(p, ti, ri, FlexMode::Speed);
                p.song_rev += 1;
            }
            if ui.button("Stop Following").clicked() {
                s.flex.remove_later(ti, ri);
            }
        }
        None => {
            if ui
                .button("Fit to Tempo")
                .on_hover_text("Follow the project tempo from now on, at the tempo the sample was made at")
                .clicked()
            {
                s.flex.fit_later(ti, ri);
            }
        }
    }
    ui.add_space(10.0);
    if ui
        .button("Audio File Editor…")
        .on_hover_text("Edits that change the file itself: normalize, gain, fades, silence, reverse, trim")
        .clicked()
    {
        if let Some(a) = p.tracks[ti].regions[ri].audio.clone() {
            s.file_edit.open(a);
        }
    }
    if changed {
        let params = &mut p.tracks[ti].regions[ri].params;
        params.gain_db = gain;
        params.reverse = reverse;
        p.song_rev += 1;
    }
}

fn wave(ui: &mut Ui, p: &mut Project, s: &mut UiState, body: Rect, ti: usize, ri: usize) {
    let _ = s;
    let g = ui.painter_at(body);
    let region = &p.tracks[ti].regions[ri];
    let Some(audio) = region.audio.as_ref() else { return };
    let duration = audio.file.duration_secs().max(1e-3) as f32;
    let map = p.tempo_map();
    let start_beat = p.region_start_beats(region);
    let end_beat = p.region_end_beats(region);
    let src = region.params.source_start;
    // Seconds of the file per second of the song: not 1 when the region follows the tempo.
    let speed = crate::region_edit::file_speed(p, region);
    let length = (map.seconds_at(end_beat) - map.seconds_at(start_beat)) as f32 * speed;
    let played = played_seconds(region).min(length.max(0.0));
    let src_end = src + played;
    let fade_secs = |beats: f32, from_end: bool| -> f32 {
        if from_end {
            ((map.seconds_at(end_beat) - map.seconds_at(end_beat - f64::from(beats)).max(0.0)) as f32) * speed
        } else {
            ((map.seconds_at(start_beat + f64::from(beats)) - map.seconds_at(start_beat)) as f32) * speed
        }
    };
    let fade_in = fade_secs(region.params.fade_in, false).min(played);
    let fade_out = fade_secs(region.params.fade_out, true).min(played);
    let color = region.params.color.unwrap_or(p.tracks[ti].color);

    let ruler = Rect::from_min_size(body.min, vec2(body.width(), RULER_H));
    let lane = Rect::from_min_max(
        pos2(body.left() + PAD, ruler.bottom() + 4.0),
        pos2(body.right() - PAD, body.bottom() - 6.0),
    );
    if lane.width() < 20.0 || lane.height() < 20.0 {
        return;
    }
    let x_of = |sec: f32| lane.left() + sec / duration * lane.width();
    let sec_of = |x: f32| ((x - lane.left()) / lane.width() * duration).clamp(0.0, duration);

    // Ruler: seconds into the file.
    g.rect_filled(ruler, 0.0, crate::theme::toolbar());
    let step = [0.1f32, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0]
        .into_iter()
        .find(|st| st / duration * lane.width() >= 60.0)
        .unwrap_or(120.0);
    let mut t = 0.0;
    while t <= duration {
        let x = x_of(t);
        g.line_segment([pos2(x, ruler.bottom() - 5.0), pos2(x, ruler.bottom())], Stroke::new(1.0, text_dim_col()));
        let label = if step < 1.0 { format!("{t:.2}s") } else { format!("{}:{:02}", (t / 60.0) as u32, (t % 60.0) as u32) };
        g.text(pos2(x + 3.0, ruler.center().y), Align2::LEFT_CENTER, label, FontId::proportional(9.5), text_dim_col());
        t += step;
    }

    // The whole file, dim; the played part lit.
    g.rect_filled(lane, 3.0, Color32::from_black_alpha(60));
    let lit = Rect::from_min_max(pos2(x_of(src), lane.top()), pos2(x_of(src_end), lane.bottom()));
    g.rect_filled(lit, 3.0, color.gamma_multiply(0.35));
    if let Some(peaks) = audio.peaks_for(lane.width()) {
        let fit = audio.waveform_fit();
        crate::tracks_area::draw_peaks(&g, lane, (lane.left(), lane.width()), peaks, fit, false, color.gamma_multiply(0.45));
        let lit_peaks = g.with_clip_rect(lit.intersect(body));
        crate::tracks_area::draw_peaks(&lit_peaks, lane, (lane.left(), lane.width()), peaks, fit, false, Color32::from_gray(235));
    }
    g.rect_stroke(lit, 3.0, Stroke::new(1.0, color), StrokeKind::Inside);

    // Fades.
    let fade_stroke = Stroke::new(1.5, Color32::from_gray(250));
    if fade_in > 0.0 {
        g.line_segment([pos2(lit.left(), lit.bottom()), pos2(x_of(src + fade_in), lit.top())], fade_stroke);
    }
    if fade_out > 0.0 {
        g.line_segment([pos2(x_of(src_end - fade_out), lit.top()), pos2(lit.right(), lit.bottom())], fade_stroke);
    }

    // Handles: trims at the bottom corners, fades at the top.
    let handles = [
        (Rect::from_center_size(pos2(lit.left(), lit.bottom() - HANDLE), vec2(HANDLE, HANDLE * 2.0)), 0u8),
        (Rect::from_center_size(pos2(lit.right(), lit.bottom() - HANDLE), vec2(HANDLE, HANDLE * 2.0)), 1),
        (Rect::from_center_size(pos2(x_of(src + fade_in), lit.top() + HANDLE * 0.5), vec2(HANDLE, HANDLE)), 2),
        (Rect::from_center_size(pos2(x_of(src_end - fade_out), lit.top() + HANDLE * 0.5), vec2(HANDLE, HANDLE)), 3),
    ];
    let mut hovered_handle = None;
    for (r, k) in handles {
        let resp = ui.interact(r.expand(3.0), egui::Id::unique(("audio_track_editor_handle", k)), Sense::drag());
        let hot = resp.hovered() || resp.dragged();
        let fill = if hot { crate::theme::accent() } else { Color32::from_gray(220) };
        g.rect_filled(r, 2.0, fill);
        if hot {
            hovered_handle = Some(k);
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if resp.drag_started() {
            let grab = match k {
                0 => TrimStart::of(p, ti, ri).ok().map(Grab::Start),
                1 => Some(Grab::End),
                2 => Some(Grab::FadeIn),
                _ => Some(Grab::FadeOut),
            };
            GRAB.with(|gr| *gr.borrow_mut() = grab);
        }
        if resp.dragged() {
            if let Some(pt) = resp.interact_pointer_pos() {
                let sec = sec_of(pt.x);
                GRAB.with(|gr| {
                    let grab = gr.borrow();
                    match grab.as_ref() {
                        Some(Grab::Start(from)) => {
                            let new_src = sec.min(src_end - 0.01).max(0.0);
                            let at = map.seconds_at(from.beat) + f64::from((new_src - from.source_start) / speed);
                            trim_start(p, ti, ri, from, map.beat_at(at.max(0.0)));
                        }
                        Some(Grab::End) => {
                            let new_end = sec.max(src + 0.01);
                            let at = map.seconds_at(start_beat) + f64::from((new_end - src) / speed);
                            let _ = trim_end(p, ti, ri, map.beat_at(at));
                        }
                        Some(Grab::FadeIn) => {
                            let secs = (sec - src).clamp(0.0, played - fade_out);
                            let beats = map.beat_at(map.seconds_at(start_beat) + f64::from(secs / speed)) - start_beat;
                            p.tracks[ti].regions[ri].params.fade_in = beats.max(0.0) as f32;
                        }
                        Some(Grab::FadeOut) => {
                            let secs = (src_end - sec).clamp(0.0, played - fade_in);
                            let beats = end_beat - map.beat_at(map.seconds_at(end_beat) - f64::from(secs / speed));
                            p.tracks[ti].regions[ri].params.fade_out = beats.max(0.0) as f32;
                        }
                        None => {}
                    }
                });
            }
        }
        if resp.drag_stopped() {
            GRAB.with(|gr| *gr.borrow_mut() = None);
            p.song_rev += 1;
        }
    }

    // A click anywhere else in the played part moves the playhead there.
    let click = ui.interact(lit, egui::Id::unique("audio_track_editor_lane"), Sense::click());
    if hovered_handle.is_none() && click.clicked() {
        if let Some(pt) = click.interact_pointer_pos() {
            let into = (sec_of(pt.x) - src).clamp(0.0, played);
            p.position_beats = map.beat_at(map.seconds_at(start_beat) + f64::from(into / speed));
        }
    }

    // The playhead, where it is inside this region.
    let pos = p.position_beats;
    if pos >= start_beat && pos <= end_beat {
        let into = (map.seconds_at(pos) - map.seconds_at(start_beat)) as f32 * speed;
        let x = x_of(src + into);
        g.line_segment([pos2(x, ruler.top()), pos2(x, lane.bottom())], Stroke::new(1.0, Color32::WHITE));
    }

    g.text(
        pos2(lane.left(), body.bottom() - 2.0),
        Align2::LEFT_BOTTOM,
        format!("Plays {src:.2}s to {src_end:.2}s of {duration:.2}s. Drag the lower handles to trim, the upper ones to fade."),
        FontId::proportional(9.5),
        text_dim_col(),
    );
}
