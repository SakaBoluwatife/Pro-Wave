//! The Channel Rack (F6): a row for each instrument track, with its controls and the steps or
//! notes it has in the current pattern. The piano button opens the channel in the Piano Roll. Drag a
//! pattern's name onto the Tracks area to place it, and a sample from the Browser onto a row to
//! load it there.
use crate::icons::{button, Icon};
use crate::model::{Project, TrackKind};
use crate::piano_roll::Audition;
use crate::song_pattern::{self as sp, STEP_BEATS};
use crate::widgets::blue;
use crate::Bottom;
use crate::UiState;
use egui::{pos2, vec2, Color32, FontId, Rect, Sense, Stroke, Ui};

const STEP_W: f32 = 18.0;
const ROW_H: f32 = 24.0;
const STEP_CHOICES: [usize; 5] = [8, 16, 32, 48, 64];

/// A pattern being dragged to the Tracks area.
#[derive(Clone, Copy, Debug)]
pub struct PatternDrag(pub u64);

#[derive(Default)]
pub struct RackView {
    pub open: bool,
    current: u64,
    /// The note a step click is sounding, let go when the mouse button is.
    held: Option<u8>,
    /// A channel to delete once the rows are drawn.
    delete: Option<usize>,
    /// The step buttons set each step's pan instead of switching it.
    pan_mode: bool,
    /// The region selected in the Tracks area when last seen, so selecting a pattern clip
    /// there shows its pattern here.
    seen_selection: Option<(usize, usize)>,
    /// Auto Place off: channels stay off the Playlist until Dump to Playlist. Auto Place is on
    /// by default, so a channel shows up on the Playlist as soon as it has steps.
    pub manual_place: bool,
}

impl RackView {
    /// The pattern on show, making one when there is none.
    pub fn current_pattern(&mut self, p: &mut Project) -> u64 {
        if p.patterns.is_empty() {
            self.current = sp::new_pattern(p);
        }
        if !p.patterns.iter().any(|x| x.id == self.current) {
            self.current = p.patterns[0].id;
        }
        self.current
    }
}

fn toggle(ui: &mut Ui, on: bool, text: &str, hint: &str) -> bool {
    ui.selectable_label(on, text).on_hover_text(hint).clicked()
}

/// Opens the Piano Roll on the channel's part of the pattern, placing it first if it is nowhere
/// yet, as the Piano Roll edits a region.
fn open_in_piano_roll(p: &mut Project, s: &mut UiState, t: usize, id: u64) {
    let placed = p.tracks[t]
        .regions
        .iter()
        .position(|r| r.params.pattern == id);
    let bar = p.beats_to_bar(p.position_beats).floor() as f32 + 1.0;
    let Some(r) = placed.or_else(|| sp::place_on(p, id, t, bar)) else {
        return;
    };
    p.selected_track = t;
    s.tracks.select_only((t, r));
    s.bottom = Bottom::PianoRoll;
}

/// Sounds the channel's step pitch on its instrument, until the mouse button is let go.
fn sound_step(p: &mut Project, s: &mut UiState, t: usize, id: u64, channel: u64) {
    let Some(pitch) = p
        .patterns
        .iter_mut()
        .find(|x| x.id == id)
        .map(|x| x.lane_mut(channel).step_pitch)
    else {
        return;
    };
    if let Some(old) = s.rack.held.take() {
        s.audition.push(Audition::Off { pitch: old });
    }
    s.audition.push(Audition::On {
        pitch,
        vel: 100,
        track: t.min(usize::from(u16::MAX)) as u16,
    });
    s.rack.held = Some(pitch);
}

/// Each step's pan: drag a step up or down (up is right), double-click to centre it.
fn pan_grid(ui: &mut Ui, p: &mut Project, id: u64, channel: u64) -> bool {
    let Some(pattern) = p.patterns.iter_mut().find(|x| x.id == id) else {
        return false;
    };
    let steps = pattern.steps;
    let lane = pattern.lane_mut(channel);
    let (rect, _) = ui.allocate_exact_size(vec2(steps as f32 * STEP_W, ROW_H), Sense::hover());
    let g = ui.painter_at(rect);
    let mut changed = false;
    for step in 0..steps {
        let cell = Rect::from_min_size(
            rect.min + vec2(step as f32 * STEP_W, 0.0),
            vec2(STEP_W, ROW_H),
        )
        .shrink(1.5);
        let resp = ui
            .interact(
                cell,
                egui::Id::unique(("rack_pan", id, channel, step)),
                Sense::click_and_drag(),
            )
            .on_hover_text("Pan: drag up for right, down for left; double-click to centre");
        let mut pan = lane.pan_of(step);
        if resp.double_clicked() {
            pan = 0;
        } else if resp.dragged() {
            let dy = resp.drag_delta().y;
            pan = (f32::from(pan) - dy).round().clamp(-64.0, 63.0) as i8;
        }
        if pan != lane.pan_of(step) {
            lane.set_pan(step, pan);
            changed = true;
        }
        let share = if (step / 4) % 2 == 0 { 0.22 } else { 0.13 };
        g.rect_filled(
            cell,
            2.0,
            crate::theme::ink_on(crate::theme::panel(), share),
        );
        let mid = cell.center().y;
        g.line_segment(
            [pos2(cell.left(), mid), pos2(cell.right(), mid)],
            Stroke::new(1.0, crate::theme::ink_on(crate::theme::panel(), 0.4)),
        );
        let h = f32::from(pan) / 64.0 * cell.height() * 0.5;
        if pan != 0 {
            let bar = Rect::from_min_max(
                pos2(cell.left() + 2.0, mid.min(mid - h)),
                pos2(cell.right() - 2.0, mid.max(mid - h)),
            );
            g.rect_filled(bar, 1.0, crate::theme::accent());
        }
        if !lane.step_is_on(step) {
            g.rect_filled(cell, 2.0, Color32::from_black_alpha(90));
        }
    }
    changed
}

/// The step of pattern `id` under the playhead, where the pattern is placed and the playhead is
/// inside one of its regions (a region longer than its pattern loops it).
fn playhead_step(p: &Project, id: u64) -> Option<usize> {
    let pattern = p.patterns.iter().find(|x| x.id == id)?;
    let len = f64::from(pattern.len_beats()).max(f64::from(STEP_BEATS));
    let at = p.position_beats;
    p.tracks.iter().flat_map(|t| &t.regions).find_map(|r| {
        if r.params.pattern != id {
            return None;
        }
        let (start, end) = (p.region_start_beats(r), p.region_end_beats(r));
        (at >= start && at < end).then(|| ((at - start) % len / f64::from(STEP_BEATS)) as usize)
    })
}

/// Draws and edits the steps. Returns whether they changed, and whether a step was switched on (to
/// sound it, as FL Studio does).
fn step_grid(ui: &mut Ui, p: &mut Project, id: u64, channel: u64) -> (bool, bool) {
    let now = playhead_step(p, id);
    if p.playing {
        ui.ctx().request_repaint();
    }
    let Some(pattern) = p.patterns.iter_mut().find(|x| x.id == id) else {
        return (false, false);
    };
    let steps = pattern.steps;
    let lane = pattern.lane_mut(channel);
    let (rect, _) =
        ui.allocate_exact_size(vec2(steps as f32 * STEP_W, ROW_H), Sense::click_and_drag());
    let g = ui.painter_at(rect);
    let mut changed = false;
    let mut switched_on = false;
    // FL-style painting: press a step and drag across the row. Left paints steps on (or off,
    // when the first one was on), right drags erase. What it paints is kept while a button is held.
    let paint_key = egui::Id::unique("rack_step_paint");
    let (pointer, primary, secondary, pressed_primary, pressed_secondary) = ui.input(|i| {
        (
            i.pointer.hover_pos(),
            i.pointer.primary_down(),
            i.pointer.secondary_down(),
            i.pointer.primary_pressed(),
            i.pointer.secondary_pressed(),
        )
    });
    if !primary && !secondary {
        ui.ctx()
            .data_mut(|d| d.remove::<(u64, u64, bool)>(paint_key));
    }
    let under = pointer
        .filter(|pt| rect.contains(*pt))
        .map(|pt| (((pt.x - rect.left()) / STEP_W) as usize).min(steps.saturating_sub(1)));
    if let Some(step) = under {
        if pressed_primary || pressed_secondary {
            let target = pressed_primary && !lane.step_is_on(step);
            ui.ctx()
                .data_mut(|d| d.insert_temp(paint_key, (id, channel, target)));
        }
        let paint = ui.ctx().data(|d| d.get_temp::<(u64, u64, bool)>(paint_key));
        if let Some((pid, pch, target)) = paint {
            if pid == id && pch == channel && lane.step_is_on(step) != target {
                lane.toggle_step(step);
                changed = true;
                switched_on = target;
            }
        }
    }
    for step in 0..steps {
        let cell = Rect::from_min_size(
            rect.min + vec2(step as f32 * STEP_W, 0.0),
            vec2(STEP_W, ROW_H),
        )
        .shrink(1.5);
        let resp = ui.interact(
            cell,
            egui::Id::unique(("rack_step", id, channel, step)),
            Sense::hover(),
        );
        // Off steps: a share of the text colour on the panel, alternating by beat, so they show in
        // every theme.
        let share = if (step / 4) % 2 == 0 { 0.22 } else { 0.13 };
        let fill = if lane.step_is_on(step) {
            Color32::from_rgb(0x8f, 0xd0, 0x6a)
        } else if resp.hovered() {
            crate::theme::ink_on(crate::theme::panel(), share + 0.12)
        } else {
            crate::theme::ink_on(crate::theme::panel(), share)
        };
        g.rect_filled(cell, 2.0, fill);
        // The playhead: the step playing now lights up.
        if now == Some(step) {
            g.rect_filled(cell, 2.0, Color32::from_white_alpha(70));
            g.rect_stroke(
                cell,
                2.0,
                Stroke::new(1.5, crate::theme::accent()),
                egui::StrokeKind::Inside,
            );
        }
    }
    // What the buttons cannot show is marked, never lost.
    for note in lane.off_grid() {
        let x = rect.left() + note.start / STEP_BEATS * STEP_W;
        if x < rect.right() {
            g.line_segment(
                [pos2(x, rect.bottom() - 5.0), pos2(x, rect.bottom() - 1.0)],
                Stroke::new(2.0, Color32::from_rgb(0xe6, 0xa0, 0x2a)),
            );
        }
    }
    (changed, switched_on)
}

fn channel_row(ui: &mut Ui, p: &mut Project, s: &mut UiState, t: usize, id: u64) {
    let channel = sp::channel_of(p, t);
    let row = ui.horizontal(|ui| {
        let track = &mut p.tracks[t];
        if toggle(ui, track.mute, "M", "Mute") {
            track.mute = !track.mute;
        }
        if toggle(ui, track.solo, "S", "Solo") {
            track.solo = !track.solo;
        }
        let selected = p.selected_track == t;
        let track = &mut p.tracks[t];
        let name = ui
            .add_sized(
                vec2(110.0, ROW_H - 4.0),
                egui::Button::selectable(selected, egui::RichText::new(&track.name).small())
                    .truncate(),
            )
            .on_hover_text(
                "Click to select. Click again to open its instrument. Right-click to delete it.",
            );
        let mut open = None;
        let mut to_roll = false;
        if name.clicked() {
            if selected {
                open = track
                    .instrument_plugin
                    .as_ref()
                    .map(|i| (i.uid, i.name.clone()));
            }
            p.selected_track = t;
        }
        name.context_menu(|ui| {
            if ui.button("Open in the Piano Roll").clicked() {
                to_roll = true;
                ui.close();
            }
            if ui
                .button("Delete Channel")
                .on_hover_text("Removes the channel's track, its steps and its clips")
                .clicked()
            {
                s.rack.delete = Some(t);
                ui.close();
            }
        });
        if open.is_some() {
            s.mixer.open_plugin = open;
        }
        if to_roll {
            open_in_piano_roll(p, s, t, id);
        }
        let track = &mut p.tracks[t];
        ui.add_sized(
            [62.0, 18.0],
            egui::DragValue::new(&mut track.volume_db)
                .range(-60.0..=6.0)
                .speed(0.2)
                .max_decimals(1)
                .suffix(" dB"),
        )
        .on_hover_text("Volume");
        ui.add_sized(
            [42.0, 18.0],
            egui::DragValue::new(&mut track.pan)
                .range(-1.0..=1.0)
                .speed(0.01)
                .max_decimals(2),
        )
        .on_hover_text("Pan");
        if button(ui, Icon::Mixer, "Show this channel in the mixer").clicked() {
            p.selected_track = t;
            s.show_mixer = true;
            s.bottom = Bottom::Mixer;
        }
        if button(ui, Icon::Piano, "Open in the Piano Roll").clicked() {
            open_in_piano_roll(p, s, t, id);
        }
        let scale = crate::note_edit::key_scale(p.key);
        if let Some(pattern) = p.patterns.iter_mut().find(|x| x.id == id) {
            let lane = pattern.lane_mut(channel);
            ui.add_sized(
                [40.0, 18.0],
                egui::DragValue::new(&mut lane.step_pitch)
                    .range(0..=127)
                    .custom_formatter(|v, _| {
                        // A dot on the notes of the project's key.
                        let name = crate::step_editor::note_name(v as u8);
                        match scale.and_then(|s| crate::note_edit::scale_degree(v as i32, s)) {
                            Some(_) => format!("{name}\u{2022}"),
                            None => name,
                        }
                    }),
            )
            .on_hover_text("The pitch of the step buttons");
        }
        let changed = if s.rack.pan_mode {
            pan_grid(ui, p, id, channel)
        } else {
            let (changed, switched_on) = step_grid(ui, p, id, channel);
            if switched_on {
                sound_step(p, s, t, id, channel);
            }
            changed
        };
        if changed {
            sp::push(p, id);
            // With Auto Place, a channel given its first steps shows up on the Playlist at once.
            if !s.rack.manual_place {
                let bar = p.beats_to_bar(p.position_beats).floor() as f32 + 1.0;
                sp::fill_placements(p, id, bar, None);
            }
        }
    });
    // A sample let go on the row is loaded there.
    let rect = row.response.rect;
    if let Some(drag) = egui::DragAndDrop::payload::<crate::sample_browser::SampleDrag>(ui.ctx()) {
        let over = ui
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|at| rect.contains(at));
        if over {
            ui.painter().rect_stroke(
                rect,
                2.0,
                Stroke::new(1.5, blue()),
                egui::StrokeKind::Outside,
            );
            if ui.input(|i| i.pointer.any_released()) {
                egui::DragAndDrop::clear_payload(ui.ctx());
                let info = crate::sample_place::sampler_info(&drag.0);
                let done = if crate::sample_place::holds_sample(p, t) {
                    crate::sample_place::replace_sample(p, t, &drag.0)
                } else {
                    crate::plugin_rack::add_plugin(p, t, &info)
                };
                if let Err(e) = done {
                    s.status = Some(e);
                } else {
                    p.tracks[t].name = crate::sample_browser::display_name(&drag.0);
                }
            }
        }
    }
}

fn pattern_bar(ui: &mut Ui, p: &mut Project, s: &mut UiState) {
    let mut current = s.rack.current;
    ui.horizontal_wrapped(|ui| {
        for pattern in &p.patterns {
            let text = egui::RichText::new(&pattern.name).color(pattern.color);
            let chip = ui
                .add(
                    egui::Button::selectable(pattern.id == current, text)
                        .sense(Sense::click_and_drag()),
                )
                .on_hover_text("Click to edit. Drag onto the Tracks area to place it.");
            chip.dnd_set_drag_payload(PatternDrag(pattern.id));
            if chip.clicked() {
                current = pattern.id;
            }
        }
        if button(ui, Icon::Plus, "New pattern").clicked() {
            current = sp::new_pattern(p);
        }
    });
    s.rack.current = current;
    let Some(at) = p.patterns.iter().position(|x| x.id == current) else {
        return;
    };
    ui.horizontal(|ui| {
        let pattern = &mut p.patterns[at];
        ui.add(egui::TextEdit::singleline(&mut pattern.name).desired_width(120.0));
        let before = pattern.color;
        ui.color_edit_button_srgba(&mut pattern.color);
        let mut steps = pattern.steps;
        egui::ComboBox::from_id_salt(egui::Id::unique("rack_steps"))
            .width(70.0)
            .selected_text(format!("{steps} steps"))
            .show_ui(ui, |ui| {
                for n in STEP_CHOICES {
                    ui.selectable_value(&mut steps, n, format!("{n} steps"));
                }
            });
        let resized = steps != pattern.steps;
        ui.checkbox(&mut s.rack.pan_mode, "Step Pan")
            .on_hover_text("The steps show and set each step's pan, sent to the instrument as controller 10");
        pattern.steps = steps;
        if resized || before != pattern.color {
            sp::push(p, current);
        }
        if ui
            .button("Place at Playhead")
            .on_hover_text("A region on every channel with notes in this pattern")
            .clicked()
        {
            let bar = p.beats_to_bar(p.position_beats).floor() as f32 + 1.0;
            if sp::place(p, current, bar) == 0 {
                s.status = Some("This pattern has no notes yet".to_string());
            }
        }
        if ui
            .button("Dump to Playlist")
            .on_hover_text("Every pattern not on the Playlist yet, one after another from the playhead, each channel on its own track")
            .clicked()
        {
            let mut bar = p.beats_to_bar(p.position_beats).floor() as f32 + 1.0;
            let mut placed = 0;
            let ids: Vec<u64> = p.patterns.iter().map(|x| x.id).collect();
            for id in ids {
                let on_playlist = p
                    .tracks
                    .iter()
                    .any(|t| t.regions.iter().any(|r| r.params.pattern == id));
                if on_playlist {
                    continue;
                }
                let len = p.patterns.iter().find(|x| x.id == id).map_or(0.0, |x| f64::from(x.len_beats()));
                if sp::place(p, id, bar) > 0 {
                    placed += 1;
                    let beats = p.bar_to_beats(f64::from(bar) - 1.0) + len;
                    bar = (p.beats_to_bar(beats).ceil() as f32 + 1.0).max(bar + 1.0);
                }
            }
            s.status = Some(if placed == 0 {
                "Every pattern with notes is on the Playlist already".to_string()
            } else {
                format!("{placed} patterns on the Playlist, each channel on its own track")
            });
        }
        let mut auto = !s.rack.manual_place;
        if ui
            .checkbox(&mut auto, "Auto Place")
            .on_hover_text("On: a channel shows up on the Playlist as soon as it has steps. Off: make your patterns first, then Dump to Playlist")
            .changed()
        {
            s.rack.manual_place = !auto;
        }
        if button(ui, Icon::Copy, "Clone this pattern").clicked() {
            if let Some(new) = sp::clone_pattern(p, current) {
                s.rack.current = new;
            }
        }
        if button(
            ui,
            Icon::Trash,
            "Delete this pattern. Placed regions keep their notes",
        )
        .clicked()
        {
            sp::delete_pattern(p, current);
            s.rack.current = p.patterns.first().map_or(0, |x| x.id);
        }
    });
}

fn body(ui: &mut Ui, p: &mut Project, s: &mut UiState) {
    if p.patterns.is_empty() {
        s.rack.current = sp::new_pattern(p);
    }
    if !p.patterns.iter().any(|x| x.id == s.rack.current) {
        s.rack.current = p.patterns[0].id;
    }
    // A pattern clip selected on the Playlist opens its pattern here, and its channel is selected.
    if s.tracks.selected != s.rack.seen_selection {
        s.rack.seen_selection = s.tracks.selected;
        if let Some((t, r)) = s.tracks.selected {
            let id = p
                .tracks
                .get(t)
                .and_then(|x| x.regions.get(r))
                .map_or(0, |x| x.params.pattern);
            if p.patterns.iter().any(|x| x.id == id) {
                s.rack.current = id;
                p.selected_track = t;
            }
        }
    }
    // The note a step click sounded ends with the click.
    if s.rack.held.is_some() && !ui.input(|i| i.pointer.any_down()) {
        if let Some(pitch) = s.rack.held.take() {
            s.audition.push(Audition::Off { pitch });
        }
    }
    pattern_bar(ui, p, s);
    ui.separator();
    let id = s.rack.current;
    egui::ScrollArea::both().max_height(420.0).show(ui, |ui| {
        for t in sp::channels(p) {
            channel_row(ui, p, s, t, id);
        }
        // Delete Channel, from a name's right-click: the track goes, and its steps with it.
        if let Some(t) = s.rack.delete.take() {
            let channel = p.tracks.get(t).map_or(0, |x| x.params.channel);
            let name = p.tracks.get(t).map(|x| x.name.clone()).unwrap_or_default();
            if p.delete_tracks(&[t]) {
                for pattern in &mut p.patterns {
                    pattern
                        .lanes
                        .retain(|l| channel == 0 || l.channel != channel);
                }
                crate::tracks_area::forget_track_indexes(s);
                s.status = Some(format!("Deleted the channel {name}"));
            }
        }
        let channels = sp::channels(p).len();
        let add = ui
            .button("Add Channel")
            .on_hover_text("A new instrument track. Or drop a sample here for one that plays it.");
        if add.clicked() {
            p.add_track(TrackKind::Instrument);
        }
        if channels == 0 {
            ui.label(egui::RichText::new("No instrument tracks yet.").small());
        }
        let _ = add;
    });
    // A sample let go anywhere in the rack but on a channel's row (which loads it there) makes a
    // channel for it, on the Playlist at once: where the pattern on show is placed, or at the
    // playhead.
    if let Some(drag) = egui::DragAndDrop::payload::<crate::sample_browser::SampleDrag>(ui.ctx()) {
        let over = ui
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|at| ui.min_rect().contains(at));
        if over && ui.input(|i| i.pointer.any_released()) {
            egui::DragAndDrop::clear_payload(ui.ctx());
            match crate::sample_place::new_sampler_track(p, &drag.0) {
                Ok(t) => {
                    sp::channel_of(p, t);
                    let id = s.rack.current_pattern(p);
                    let bar = p.beats_to_bar(p.position_beats).floor() as f32 + 1.0;
                    sp::fill_placements(p, id, bar, Some(t));
                    p.selected_track = t;
                }
                Err(e) => s.status = Some(e),
            }
        }
    }
    ui.label(
        egui::RichText::new("Orange marks are notes the step buttons cannot show.")
            .font(FontId::proportional(10.5)),
    );
}

/// The window.
pub fn show(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    if !s.rack.open {
        return;
    }
    let mut open = true;
    egui::Window::new("Channel Rack")
        .id(egui::Id::unique("channel_rack"))
        .open(&mut open)
        .default_width(720.0)
        .show(ctx, |ui| body(ui, p, s));
    s.rack.open = open;
}
