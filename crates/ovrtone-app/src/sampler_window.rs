//! The window of a sample held on a track: its waveform, where it starts and ends, its key and
//! level, and its envelope. FL Studio's channel settings are the model for what it does, with
//! Ovrtone's own look. Double-click the track's icon, or click its instrument slot, to open it.
use crate::engine_sync::EngineSync;
use crate::model::Project;
use crate::piano_roll::Audition;
use crate::UiState;
use egui::{pos2, vec2, Align2, Color32, FontId, Rect, Sense, Stroke};
use ovrtone_host::sampler_params as x;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn text_col() -> Color32 {
    crate::theme::grey_rgb(0xe6, 0xe6, 0xe6)
}
fn dim_col() -> Color32 {
    crate::theme::grey_rgb(0x96, 0x96, 0x9a)
}
fn panel_col() -> Color32 {
    crate::theme::grey_rgb(0x1f, 0x20, 0x23)
}
const WAVE: Color32 = Color32::from_rgb(0x5f, 0xd0, 0x80);
const KNOB_W: f32 = 62.0;
const WIDTH: f32 = 540.0;
/// Points across the waveform.
const WAVE_POINTS: usize = 500;
/// The keys the strip shows, C0 up, enough for any root a sample has.
const KEYS: std::ops::RangeInclusive<u8> = 24..=108;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Sample,
    Envelope,
}

/// What the window keeps while it is open.
#[derive(Default)]
pub struct SamplerView {
    tab: Tab,
    /// The file the waveform is of, and the waveform, which a thread reads.
    wave_of: Option<PathBuf>,
    wave: Arc<Mutex<Vec<f32>>>,
    /// The sample's length in seconds, rate and channels, read with the waveform.
    info: Arc<Mutex<Option<(f64, u32, u16)>>>,
    /// The key being auditioned while the mouse holds it down.
    held: Option<u8>,
}

/// A key's name: 60 is C4.
pub fn key_name(key: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!(
        "{}{}",
        NAMES[usize::from(key % 12)],
        i32::from(key / 12) + crate::settings::octave_naming()
    )
}

/// Reads the waveform of `path` off this thread: a long sample takes a moment.
fn look_at_wave(view: &mut SamplerView, path: &PathBuf, ctx: &egui::Context) {
    if view.wave_of.as_ref() == Some(path) {
        return;
    }
    view.wave_of = Some(path.clone());
    let out = Arc::new(Mutex::new(Vec::new()));
    view.wave = out.clone();
    let info = Arc::new(Mutex::new(None));
    view.info = info.clone();
    let (path, ctx) = (path.clone(), ctx.clone());
    let _ = std::thread::Builder::new()
        .name("ovrtone-sampler-wave".into())
        .spawn(move || {
            let Ok(file) = ovrtone_engine::AudioFile::open(&path) else {
                return;
            };
            if let Ok(mut i) = info.lock() {
                *i = Some((
                    file.duration_secs(),
                    file.sample_rate,
                    file.channels.min(usize::from(u16::MAX)) as u16,
                ));
            }
            let frames = (file.frames as usize / WAVE_POINTS).max(1);
            if let (Ok(levels), Ok(mut out)) = (file.levels(frames), out.lock()) {
                *out = levels;
            }
            ctx.request_repaint();
        });
}

/// A knob with its name and value. Drag up and down; a double click goes back to `default`.
fn knob(
    ui: &mut egui::Ui,
    name: &str,
    value: f32,
    default: f32,
    shown: String,
    id: impl std::hash::Hash + std::fmt::Debug,
) -> Option<f32> {
    let (r, _) = ui.allocate_exact_size(vec2(KNOB_W, 66.0), Sense::hover());
    let spot = Rect::from_center_size(pos2(r.center().x, r.top() + 22.0), vec2(36.0, 36.0));
    let resp = ui.interact(spot, egui::Id::unique(id), Sense::click_and_drag());
    let g = ui.painter();
    // A lit ring says where it is at a glance: from the middle for one that goes both ways (its
    // default is the middle), from the start for the rest.
    let from = if (default - 0.5).abs() < 1e-3 {
        0.0
    } else {
        -1.0
    };
    crate::widgets::value_arc(g, spot.center(), 16.5, from, value * 2.0 - 1.0);
    crate::widgets::draw_knob(g, spot.center(), 13.0, value);
    g.text(
        pos2(r.center().x, r.top() + 46.0),
        Align2::CENTER_CENTER,
        name,
        FontId::proportional(10.5),
        dim_col(),
    );
    g.text(
        pos2(r.center().x, r.top() + 59.0),
        Align2::CENTER_CENTER,
        shown,
        FontId::proportional(10.5),
        text_col(),
    );
    crate::widgets::value_gesture(ui, &resp, value, default, 0.005, 0.02).map(|v| v.clamp(0.0, 1.0))
}

/// A switch for a setting that is on or off.
fn switch(ui: &mut egui::Ui, name: &str, on: bool) -> bool {
    let mut now = on;
    ui.checkbox(&mut now, name);
    now != on
}

/// The waveform, with the part that plays lit and its two ends to drag.
fn wave_view(
    ui: &mut egui::Ui,
    levels: &[f32],
    start: f32,
    end: f32,
    reverse: bool,
) -> (Option<f32>, Option<f32>) {
    // Reversed, the sample is drawn back to front, as it plays; start and end stay where they
    // are in the file, so their handles are mirrored too.
    let shown = |v: f32| if reverse { 1.0 - v } else { v };
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 112.0), Sense::hover());
    let g = ui.painter();
    g.rect_filled(r, 6.0, crate::theme::grey_rgb(0x12, 0x13, 0x15));
    g.rect_stroke(
        r,
        6.0,
        Stroke::new(1.0, Color32::from_white_alpha(12)),
        egui::StrokeKind::Inside,
    );
    let mid = r.center().y;
    // Quiet guide lines: the centre, and half level above and below.
    for y in [mid, mid - r.height() / 4.0, mid + r.height() / 4.0] {
        g.line_segment(
            [pos2(r.left() + 4.0, y), pos2(r.right() - 4.0, y)],
            Stroke::new(
                1.0,
                Color32::from_white_alpha(if y == mid { 16 } else { 7 }),
            ),
        );
    }
    let top = levels.iter().fold(0.0f32, |m, v| m.max(*v)).max(1e-3);
    let count = levels.len().max(1) as f32;
    // Filled, as a shape, each column from its level above the centre to the same below.
    for (k, level) in levels.iter().enumerate() {
        let part = k as f32 / count;
        let x0 = r.left() + shown(part) * r.width();
        let x1 = r.left() + shown((k + 1) as f32 / count) * r.width();
        let h = ((level / top) * (r.height() / 2.0 - 6.0)).max(0.5);
        let playing = part >= start && part <= end;
        let color = if playing {
            WAVE
        } else {
            WAVE.gamma_multiply(0.22)
        };
        g.rect_filled(
            Rect::from_min_max(pos2(x0.min(x1), mid - h), pos2(x0.max(x1) + 0.6, mid + h)),
            0.0,
            color,
        );
    }
    // What does not play is shaded over.
    let (a, b) = (
        r.left() + shown(start) * r.width(),
        r.left() + shown(end) * r.width(),
    );
    let (a, b) = (a.min(b), a.max(b));
    g.rect_filled(
        Rect::from_min_max(r.min, pos2(a, r.bottom())),
        6.0,
        Color32::from_black_alpha(90),
    );
    g.rect_filled(
        Rect::from_min_max(pos2(b, r.top()), r.max),
        6.0,
        Color32::from_black_alpha(90),
    );
    let handle = |value: f32, key: &str| -> Option<f32> {
        let x = r.left() + shown(value) * r.width();
        let spot = Rect::from_center_size(pos2(x, r.center().y), vec2(12.0, r.height()));
        let resp = ui.interact(spot, egui::Id::unique(("sampler_end", key)), Sense::drag());
        let g = ui.painter();
        let lit = resp.hovered() || resp.dragged();
        let ink = if lit {
            Color32::WHITE
        } else {
            crate::theme::accent()
        };
        g.line_segment(
            [pos2(x, r.top()), pos2(x, r.bottom())],
            Stroke::new(if lit { 2.0 } else { 1.5 }, ink),
        );
        // A flag at the top says which end it is.
        let flag = Rect::from_min_size(
            pos2(if key == "start" { x } else { x - 14.0 }, r.top()),
            vec2(14.0, 13.0),
        );
        g.rect_filled(flag, 2.0, ink);
        g.text(
            flag.center(),
            Align2::CENTER_CENTER,
            if key == "start" { "S" } else { "E" },
            FontId::proportional(9.5),
            Color32::BLACK,
        );
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        (resp.dragged())
            .then(|| resp.interact_pointer_pos())
            .flatten()
            .map(|pt| shown(((pt.x - r.left()) / r.width()).clamp(0.0, 1.0)))
    };
    (handle(start, "start"), handle(end, "end"))
}

/// The keys: a click on one makes it the root, the note that plays the sample as recorded.
fn root_keys(ui: &mut egui::Ui, root: u8) -> Option<u8> {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click());
    let white = |key: u8| !matches!(key % 12, 1 | 3 | 6 | 8 | 10);
    let whites = KEYS.filter(|k| white(*k)).count() as f32;
    let w = r.width() / whites;
    let g = ui.painter();
    let mut at = r.left();
    let mut spots: Vec<(u8, Rect)> = Vec::new();
    for key in KEYS.filter(|k| white(*k)) {
        let spot = Rect::from_min_size(pos2(at, r.top()), vec2(w - 1.0, r.height()));
        g.rect_filled(
            spot,
            1.0,
            if key == root {
                crate::theme::accent()
            } else {
                // Keys look like keys in any theme.
                Color32::from_gray(0xe8)
            },
        );
        // Each C named, and the root, so the keys can be read at a glance.
        if key % 12 == 0 || key == root {
            g.text(
                pos2(spot.center().x, spot.bottom() - 7.0),
                Align2::CENTER_CENTER,
                key_name(key),
                FontId::proportional(8.5),
                if key == root {
                    crate::theme::on_accent()
                } else {
                    Color32::from_gray(0x60)
                },
            );
        }
        spots.push((key, spot));
        at += w;
    }
    // The black keys go on top, and take the click there.
    let mut blacks: Vec<(u8, Rect)> = Vec::new();
    for key in KEYS.filter(|k| !white(*k)) {
        let Some((_, left)) = spots.iter().find(|(k, _)| *k == key - 1) else {
            continue;
        };
        let spot = Rect::from_center_size(
            pos2(left.right() + 0.5, r.top() + r.height() * 0.3),
            vec2(w * 0.65, r.height() * 0.6),
        );
        g.rect_filled(
            spot,
            1.0,
            if key == root {
                crate::theme::accent()
            } else {
                Color32::from_gray(0x22)
            },
        );
        blacks.push((key, spot));
    }
    let clicked = resp.interact_pointer_pos().filter(|_| resp.clicked())?;
    blacks
        .iter()
        .chain(&spots)
        .find(|(_, spot)| spot.contains(clicked))
        .map(|(key, _)| *key)
}

/// A picture of the envelope, to see what the four knobs make.
fn envelope_view(ui: &mut egui::Ui, attack: f32, decay: f32, sustain: f32, release: f32) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 96.0), Sense::hover());
    let g = ui.painter();
    g.rect_filled(r, 4.0, crate::theme::grey_rgb(0x14, 0x15, 0x17));
    let (a, d, rel) = (x::seconds(attack), x::seconds(decay), x::seconds(release));
    let hold = 0.5f32;
    let total = (a + d + hold + rel).max(0.1);
    let across = |t: f32| r.left() + 6.0 + t / total * (r.width() - 12.0);
    let up = |level: f32| r.bottom() - 6.0 - level * (r.height() - 12.0);
    let points = vec![
        pos2(across(0.0), up(0.0)),
        pos2(across(a), up(1.0)),
        pos2(across(a + d), up(sustain)),
        pos2(across(a + d + hold), up(sustain)),
        pos2(across(total), up(0.0)),
    ];
    // Filled under the curve (a column at a time, so any shape fills), then the line, then a dot
    // and a letter at each stage.
    let fill = WAVE.gamma_multiply(0.14);
    for w in points.windows(2) {
        let steps = ((w[1].x - w[0].x).abs() / 2.0).ceil().max(1.0) as usize;
        for k in 0..steps {
            let t = k as f32 / steps as f32;
            let x = w[0].x + (w[1].x - w[0].x) * t;
            let y = w[0].y + (w[1].y - w[0].y) * t;
            g.line_segment(
                [pos2(x, y), pos2(x, r.bottom() - 6.0)],
                Stroke::new(2.0, fill),
            );
        }
    }
    g.add(egui::Shape::line(points.clone(), Stroke::new(2.0, WAVE)));
    for (pt, name) in points[1..].iter().zip(["A", "D", "S", "R"]) {
        g.circle_filled(*pt, 3.5, WAVE);
        g.text(
            *pt + vec2(0.0, -10.0),
            Align2::CENTER_CENTER,
            name,
            FontId::proportional(9.5),
            dim_col(),
        );
    }
}

/// The window. Sends what changes straight to the sampler, which reads it on the audio thread.
pub fn show(ctx: &egui::Context, p: &mut Project, s: &mut UiState, sync: &mut EngineSync) {
    let Some(uid) = s.sampler_window else {
        return;
    };
    let Some((name, path)) = p
        .all_hosted_mut()
        .find(|i| i.uid == uid)
        .map(|i| (i.name.clone(), i.path.clone()))
    else {
        s.sampler_window = None;
        return;
    };
    let track = p
        .tracks
        .iter()
        .position(|t| t.instrument_plugin.as_ref().is_some_and(|i| i.uid == uid));
    look_at_wave(&mut s.sampler_view, &path, ctx);
    let mut values = x::defaults();
    for (id, value) in values.iter_mut().enumerate() {
        if let Some(v) = sync.param_value(uid, id as u32) {
            *value = v as f32;
        }
    }
    let at = |values: &[f32; x::COUNT], id: u32| values[id as usize];
    let defaults = x::defaults();
    let start_default = |id: u32| defaults[id as usize];
    let mut moved: Vec<(u32, f32)> = Vec::new();
    let mut open = true;
    let mut replace: Option<PathBuf> = None;
    let mut press: Option<bool> = None;
    let window = egui::Window::new(format!("Sampler: {name}"))
        .id(egui::Id::unique("sampler_window"))
        .open(&mut open)
        .resizable(false)
        .default_width(WIDTH)
        .frame(egui::Frame::window(&ctx.global_style()).fill(panel_col()))
        .show(ctx, |ui| {
            ui.set_width(WIDTH);
            ui.horizontal(|ui| {
                for (tab, label) in [(Tab::Sample, "Sample"), (Tab::Envelope, "Envelope")] {
                    if ui
                        .selectable_label(s.sampler_view.tab == tab, label)
                        .clicked()
                    {
                        s.sampler_view.tab = tab;
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Replace…").clicked() {
                        replace = rfd::FileDialog::new()
                            .add_filter("Audio", &["wav", "aif", "aiff", "flac", "mp3", "ogg"])
                            .pick_file();
                    }
                    let play = ui.add(egui::Button::new("Play").sense(Sense::click_and_drag()));
                    if play.is_pointer_button_down_on() {
                        press = Some(true);
                    } else if s.sampler_view.held.is_some() {
                        press = Some(false);
                    }
                });
            });
            ui.add_space(4.0);
            match s.sampler_view.tab {
                Tab::Sample => {
                    let levels = s
                        .sampler_view
                        .wave
                        .lock()
                        .map(|w| w.clone())
                        .unwrap_or_default();
                    let (start, end) = (at(&values, x::START), at(&values, x::END));
                    let reverse = at(&values, x::REVERSE) >= 0.5;
                    let (new_start, new_end) = wave_view(ui, &levels, start, end, reverse);
                    if let Some(v) = new_start {
                        moved.push((x::START, v.min(end)));
                    }
                    if let Some(v) = new_end {
                        moved.push((x::END, v.max(start)));
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let v = at(&values, x::VOLUME);
                        let db = x::volume_db(v);
                        let text = if v <= 0.0 {
                            "Off".to_string()
                        } else {
                            format!("{db:.1} dB")
                        };
                        if let Some(n) = knob(ui, "Volume", v, start_default(x::VOLUME), text, "sv")
                        {
                            moved.push((x::VOLUME, n));
                        }
                        let v = at(&values, x::PAN);
                        let text = match ((v - 0.5) * 200.0).round() as i32 {
                            0 => "Center".to_string(),
                            n if n < 0 => format!("{}L", -n),
                            n => format!("{n}R"),
                        };
                        if let Some(n) = knob(ui, "Pan", v, 0.5, text, "sp") {
                            moved.push((x::PAN, n));
                        }
                        let v = at(&values, x::PITCH);
                        let text = format!("{:+} st", x::pitch_semitones(v) as i32);
                        if let Some(n) = knob(ui, "Pitch", v, 0.5, text, "st") {
                            moved.push((x::PITCH, n));
                        }
                        let v = at(&values, x::FINE);
                        let text = format!("{:+.0} ct", x::fine_cents(v));
                        if let Some(n) = knob(ui, "Fine", v, 0.5, text, "sf") {
                            moved.push((x::FINE, n));
                        }
                        ui.add_space(14.0);
                        ui.vertical(|ui| {
                            for (id, label) in [
                                (x::REVERSE, "Reverse"),
                                (x::TO_END, "Play to end"),
                                (x::CUT_SELF, "Cut itself"),
                            ] {
                                let on = at(&values, id) >= 0.5;
                                if switch(ui, label, on) {
                                    moved.push((id, if on { 0.0 } else { 1.0 }));
                                }
                            }
                        });
                    });
                    ui.add_space(6.0);
                    let root = x::root_key(at(&values, x::ROOT));
                    ui.colored_label(
                        dim_col(),
                        format!(
                            "Root key: {}. Click a key: the sample plays as recorded on that note.",
                            key_name(root)
                        ),
                    );
                    if let Some(key) = root_keys(ui, root) {
                        moved.push((x::ROOT, x::of_root_key(key)));
                    }
                }
                Tab::Envelope => {
                    let (a, d, su, re) = (
                        at(&values, x::ATTACK),
                        at(&values, x::DECAY),
                        at(&values, x::SUSTAIN),
                        at(&values, x::RELEASE),
                    );
                    envelope_view(ui, a, d, su, re);
                    ui.add_space(6.0);
                    let time = |v: f32| {
                        let secs = x::seconds(v);
                        if secs < 1.0 {
                            format!("{:.0} ms", secs * 1000.0)
                        } else {
                            format!("{secs:.2} s")
                        }
                    };
                    ui.horizontal(|ui| {
                        if let Some(n) = knob(ui, "Attack", a, 0.0, time(a), "ea") {
                            moved.push((x::ATTACK, n));
                        }
                        if let Some(n) = knob(ui, "Decay", d, 0.0, time(d), "ed") {
                            moved.push((x::DECAY, n));
                        }
                        let text = format!("{:.0}%", su * 100.0);
                        if let Some(n) = knob(ui, "Sustain", su, 1.0, text, "es") {
                            moved.push((x::SUSTAIN, n));
                        }
                        let rd = start_default(x::RELEASE);
                        if let Some(n) = knob(ui, "Release", re, rd, time(re), "er") {
                            moved.push((x::RELEASE, n));
                        }
                    });
                    ui.colored_label(
                        dim_col(),
                        "With Play to end on, the sample plays through whatever the note's length.",
                    );
                }
            }
            ui.add_space(4.0);
            // What the sample is, at a glance; the whole path on hover.
            let file = path
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            let about = match s.sampler_view.info.lock().ok().and_then(|i| *i) {
                Some((secs, rate, ch)) => format!(
                    "{file}   {secs:.2} s   {:.1} kHz   {}",
                    f64::from(rate) / 1000.0,
                    match ch {
                        1 => "Mono".to_string(),
                        2 => "Stereo".to_string(),
                        n => format!("{n} channels"),
                    }
                ),
                None => file,
            };
            ui.colored_label(dim_col(), about)
                .on_hover_text(path.display().to_string());
        });
    // A sound let go on the window takes the place of this one.
    if let (Some(window), Some(drag)) = (
        window.as_ref(),
        egui::DragAndDrop::payload::<crate::sample_browser::SampleDrag>(ctx),
    ) {
        let hovering = ctx
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|at| window.response.rect.contains(at));
        if hovering {
            ctx.debug_painter().rect_stroke(
                window.response.rect,
                6.0,
                Stroke::new(2.0, crate::theme::accent()),
                egui::StrokeKind::Outside,
            );
            if ctx.input(|i| i.pointer.any_released()) {
                egui::DragAndDrop::clear_payload(ctx);
                replace = Some(drag.0.clone());
            }
        }
    }
    for (id, value) in moved {
        sync.show_param(uid, id, f64::from(value));
    }
    if let (Some(path), Some(t)) = (replace, track) {
        if let Err(e) = crate::sample_place::replace_sample(p, t, &path) {
            s.status = Some(e);
        }
    }
    // Play sounds the root key for as long as it is held.
    if let (Some(down), Some(t)) = (press, track) {
        let root = x::root_key(values[x::ROOT as usize]);
        match (down, s.sampler_view.held) {
            (true, None) => {
                s.audition.push(Audition::On {
                    pitch: root,
                    vel: 100,
                    track: t as u16,
                });
                s.sampler_view.held = Some(root);
            }
            (false, Some(pitch)) => {
                s.audition.push(Audition::Off { pitch });
                s.sampler_view.held = None;
            }
            _ => {}
        }
    }
    if !open {
        s.sampler_window = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_named_as_logic_does_with_c3_at_60() {
        // The default naming (Settings > Display), as Logic and Ableton: 60 is C3.
        assert_eq!(key_name(60), "C3");
        assert_eq!(key_name(61), "C#3");
        assert_eq!(key_name(48), "C2");
        assert_eq!(key_name(24), "C0");
    }
}
