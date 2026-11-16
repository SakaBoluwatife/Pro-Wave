//! The window of an effect in a channel strip's Audio FX slot: a faceplate in one look shared by
//! every effect. A header with the power switch (bypass), the name, Reset and Close; a display
//! that draws what the settings do (the EQ curve with its bands to drag, a compressor's curve,
//! a shaper's curve, the echoes of a delay, the decay of a reverb, the sweep of a chorus, the
//! envelope a transient designer makes), with a gain reduction meter for the effects that
//! reduce; choices as segmented buttons and switches; one large knob for the main setting and
//! small ones for the rest. Every control is automatable: moving one while the project plays
//! writes it, and it moves by itself when its lane is played.
use crate::automation;
use crate::icons::{self, Icon};
use crate::mixer::Meters;
use crate::model::{AutoTarget, Project, Track};
use crate::theme;
use crate::UiState;
use egui::text::{LayoutJob, TextFormat};
use egui::{pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui};
use ovrtone_engine::{FxKind, ParamInfo, MAX_FX_PARAMS};

/// Sample rate the curves are drawn for.
const CURVE_RATE: f32 = 48_000.0;
const DISPLAY_H: f32 = 176.0;
const HERO_R: f32 = 34.0;
const SMALL_R: f32 = 16.0;

type Params = [f32; MAX_FX_PARAMS];

/// A control the user moved: parameter number, new value, and whether the gesture ended.
struct Edit {
    param: usize,
    value: f32,
    ended: bool,
}

/// The colours of the faceplate, from the theme in use.
#[derive(Clone, Copy)]
struct Look {
    plate: Color32,
    display: Color32,
    grid: Color32,
    text: Color32,
    dim: Color32,
    accent: Color32,
    knob: Color32,
    edge: Color32,
    /// The project tempo where the playhead is, for a synced delay.
    bpm: f32,
    /// How large the knobs are drawn: 1, or more in a window made larger.
    scale: f32,
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

fn look() -> Look {
    let base = theme::panel_bg();
    let text = theme::text();
    // A light theme keeps a light display, a dark one a dark display; the grid always stands out.
    let light = 0.299 * f32::from(base.r()) + 0.587 * f32::from(base.g()) + 0.114 * f32::from(base.b()) > 140.0;
    let display = if light {
        mix(base, Color32::BLACK, 0.10)
    } else {
        mix(base, Color32::BLACK, 0.45)
    };
    Look {
        plate: mix(base, text, 0.03),
        display,
        grid: mix(display, text, if light { 0.22 } else { 0.16 }),
        text,
        dim: mix(theme::text_dim(), text, 0.35),
        accent: theme::accent(),
        knob: mix(base, text, 0.10),
        edge: mix(base, Color32::BLACK, 0.5),
        bpm: 120.0,
        scale: 1.0,
    }
}

/// What the display of each effect draws.
#[derive(Clone, Copy, PartialEq)]
enum View {
    Eq,
    Transfer,
    Shape,
    Envelope,
    Echoes,
    Decay,
    Sweep,
    Focus,
    /// The sound going by, and its levels.
    Wave,
    /// Left against right (goniometer) and how alike they are.
    Scope,
    /// Loudness, peak, spectrum and image.
    Meter,
    Nothing,
}

fn view(kind: FxKind) -> View {
    match kind {
        FxKind::ChannelEq | FxKind::MultiEq | FxKind::Prism => View::Eq,
        FxKind::Press | FxKind::Wall => View::Transfer,
        FxKind::Ember | FxKind::Heat | FxKind::Clip => View::Shape,
        FxKind::Snap => View::Envelope,
        FxKind::Drift => View::Echoes,
        FxKind::Halo => View::Decay,
        FxKind::Swirl => View::Sweep,
        FxKind::Calm => View::Focus,
        FxKind::Shut => View::Transfer,
        FxKind::Width => View::Scope,
        FxKind::Meter => View::Meter,
        FxKind::Bands
        | FxKind::Reel
        | FxKind::Grit
        | FxKind::Flange
        | FxKind::Sweep
        | FxKind::Pulse
        | FxKind::Shift
        | FxKind::Utility
        | FxKind::Air
        | FxKind::Freq => View::Wave,
        _ => View::Nothing,
    }
}

/// Whether the effect turns the sound down, so the display carries a reduction meter.
fn reduces(kind: FxKind) -> bool {
    matches!(
        kind,
        FxKind::Press
            | FxKind::Wall
            | FxKind::Heat
            | FxKind::Calm
            | FxKind::Prism
            | FxKind::Clip
            | FxKind::Compressor
            | FxKind::Gate
            | FxKind::Limiter
            | FxKind::Bands
            | FxKind::Shut
    )
}

/// The settings drawn as large knobs.
fn heroes(kind: FxKind) -> &'static [usize] {
    match kind {
        FxKind::Ember | FxKind::Heat => &[1],
        FxKind::Halo => &[1],
        FxKind::Swirl => &[2],
        FxKind::Snap => &[0, 1],
        FxKind::Prism | FxKind::Bands | FxKind::Meter => &[],
        FxKind::Sweep => &[1],
        FxKind::Pulse => &[3],
        FxKind::Air => &[2],
        _ => &[0],
    }
}

/// The band of Prism being edited, kept with the window.
fn chosen_band(ui: &Ui) -> usize {
    ui.ctx()
        .data(|d| d.get_temp::<usize>(egui::Id::unique("prism_band")))
        .unwrap_or(2)
        .min(5)
}

fn choose_band(ui: &Ui, band: usize) {
    ui.ctx()
        .data_mut(|d| d.insert_temp(egui::Id::unique("prism_band"), band.min(5)));
}

/// The settings drawn as small knobs, in order.
fn small_knobs(kind: FxKind, band: usize) -> Vec<usize> {
    if kind == FxKind::Prism {
        let at = band * 5;
        return vec![at, at + 1, at + 2, at + 3, at + 4, 30, 31];
    }
    (0..kind.params().len())
        .filter(|k| !heroes(kind).contains(k) && kind.choices(*k).is_none())
        .collect()
}

/// A value as the faceplate writes it.
/// A typed value in the setting's own units ("2.5k" or "2.5 kHz" is 2500 Hz; a setting shown
/// in % takes a percentage), put into its range. None when it is not a number.
fn typed_value(info: &ParamInfo, typed: &str) -> Option<f32> {
    let low = typed.trim().to_lowercase();
    let kilo = low.contains('k');
    let number: String = low
        .chars()
        .filter(|ch| ch.is_ascii_digit() || *ch == '.' || *ch == '-' || *ch == ',')
        .map(|ch| if ch == ',' { '.' } else { ch })
        .collect();
    let mut v: f32 = number.parse().ok()?;
    if kilo && info.unit == "Hz" {
        v *= 1000.0;
    }
    // Settings shown as a percentage of 0..1 (or -1..1) are typed as that percentage.
    if info.unit.is_empty() && (info.max <= 1.0) {
        v /= 100.0;
    }
    v.is_finite().then(|| v.clamp(info.min, info.max))
}

fn value_text(info: &ParamInfo, v: f32) -> String {
    match info.unit {
        "dB" => format!("{v:+.1} dB"),
        "Hz" if v >= 1000.0 => format!("{:.2} kHz", v / 1000.0),
        "Hz" => format!("{v:.0} Hz"),
        "ms" if v >= 100.0 => format!("{v:.0} ms"),
        "ms" => format!("{v:.1} ms"),
        "s" => format!("{v:.2} s"),
        "%" => format!("{v:+.0} %"),
        ":1" => format!("{v:.1}:1"),
        "st" => format!("{v:+.1} st"),
        "ct" => format!("{v:+.0} ct"),
        "oct" => format!("{v:+.1} oct"),
        "°" => format!("{v:.0}°"),
        "bit" => format!("{v:.0} bit"),
        "×" => format!("{v:.2}×"),
        "LUFS" => format!("{v:.0} LUFS"),
        _ if info.min < 0.0 => format!("{:+.0} %", v * 100.0),
        _ if info.max <= 1.0 => format!("{:.0} %", v * 100.0),
        _ => format!("{v:.2}"),
    }
}

/// The label a setting has on the faceplate (Prism's bands drop their number: the band
/// buttons say which one it is).
fn label(kind: FxKind, info: &ParamInfo) -> &'static str {
    if kind == FxKind::Prism && info.name.starts_with("Band ") {
        return info.name.get(7..).unwrap_or(info.name);
    }
    info.name
}

/// Draws a window for every effect the Mixer opened: several can be open at once, each stays
/// until it is closed, and clicking an effect's slot again brings its window to the front.
pub fn show(ctx: &egui::Context, p: &mut Project, s: &mut UiState, meters: &Meters) {
    let v = &mut s.mixer;
    if let Some(w) = v.fx_window {
        if !v.fx_open.contains(&w) {
            v.fx_open.push(w);
        }
    }
    // Effects that went away take their windows with them.
    v.fx_open
        .retain(|(ti, slot)| p.strip(*ti).is_some_and(|t| *slot < t.inserts.len()));
    if v.fx_window.is_some_and(|w| !v.fx_open.contains(&w)) {
        v.fx_window = None;
    }
    let front = v.fx_window;
    let raise = front.is_some() && front != v.fx_front;
    v.fx_front = front;
    if let Some(l) = v.listen.as_mut() {
        l.update(ctx.input(|i| i.stable_dt).clamp(0.0, 0.2));
        ctx.request_repaint();
    }
    let mut closed = Vec::new();
    for w in v.fx_open.clone() {
        let to_top = raise && Some(w) == front;
        let heard = v.listen.as_ref().filter(|l| l.key == w);
        if !one_window(ctx, p, w, to_top, meters, heard) {
            closed.push(w);
        }
    }
    for w in closed {
        v.fx_open.retain(|x| *x != w);
        if v.fx_window == Some(w) {
            v.fx_window = v.fx_open.last().copied();
            v.fx_front = v.fx_window;
        }
    }
}

/// The window of the effect in `slot` of track `ti`. False when the user closed it.
fn one_window(
    ctx: &egui::Context,
    p: &mut Project,
    (ti, slot): (usize, usize),
    to_top: bool,
    meters: &Meters,
    heard: Option<&Listen>,
) -> bool {
    let Some(insert) = p.strip(ti).and_then(|t| t.inserts.get(slot)).copied() else {
        return false;
    };
    let mut open = true;
    let title = format!("{} · {}", p.strip(ti).map_or("", |t| t.name.as_str()), insert.name());
    let reduction = meters.reduction.get(ti).copied().unwrap_or(0.0);
    let beat = p.position_beats;
    let mut edited = insert;
    let mut edits: Vec<Edit> = Vec::new();
    // The tracks that can key it: any other that makes sound.
    let keys: Vec<(usize, String)> = p
        .tracks
        .iter()
        .enumerate()
        .filter(|(k, t)| *k != ti && crate::model::can_key(Some(t.kind)))
        .map(|(k, t)| (k, t.name.clone()))
        .collect();
    let Some(track) = p.strip(ti) else {
        return false;
    };
    let lk = Look {
        bpm: p.tempo_at(beat),
        ..look()
    };
    // At least as wide as its knobs and choices at their normal size, so they never push it wider.
    let band_now = ctx.data(|d| d.get_temp::<usize>(egui::Id::unique("prism_band"))).unwrap_or(2).min(5);
    // The knob rows' width at the normal size, measured when they were last drawn.
    let natural_id = egui::Id::unique(("fx_knobs_natural", ti, slot, band_now));
    let knobs_w = ctx
        .data(|d| d.get_temp::<f32>(natural_id))
        .unwrap_or_else(|| knobs_width(insert.kind, band_now));
    let width = (if insert.kind == FxKind::Prism { 640.0f32 } else { 560.0 })
        .max(knobs_w)
        .max(choices_width(insert.kind));
    let frame = egui::Frame::new()
        .fill(lk.plate)
        .stroke(Stroke::new(1.0, lk.edge))
        .corner_radius(12)
        .inner_margin(egui::Margin::same(14))
        .shadow(egui::epaint::Shadow {
            offset: [0, 10],
            blur: 30,
            spread: 0,
            color: Color32::from_black_alpha(120),
        });
    let id = egui::Id::unique(("fx_window", ti, slot));
    if to_top {
        ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, id));
    }
    egui::Window::new(title)
        .id(id)
        .title_bar(false)
        .resizable(true)
        .default_width(width)
        .min_width(width)
        .min_height(200.0)
        .collapsible(false)
        .frame(frame)
        .show(ctx, |ui| {
            let lk = Look {
                // The knobs grow only as the user widens the window: measured against their own
                // width at the normal size, so growing them never widens the window in turn.
                scale: ((ui.available_width() - 2.0) / knobs_w).clamp(1.0, 2.0),
                ..lk
            };
            let lk = &lk;
            // What each setting is now: a played lane wins over the setting.
            let mut shown = edited.params;
            for (k, info) in edited.kind.params().iter().enumerate() {
                let target = AutoTarget::Fx {
                    slot: slot as u8,
                    param: k as u8,
                };
                if let Some(n) = track.auto.played(target, beat) {
                    shown[k] = info.from_norm(n);
                }
            }
            let (reset, close) = header(ui, lk, edited.kind, &mut edited.bypass);
            if reset {
                edited.params = edited.kind.defaults();
            }
            if close {
                open = false;
            }
            ui.add_space(6.0);
            preset_row(ui, lk, edited.kind, &mut edited.params);
            ui.add_space(6.0);
            let band = chosen_band(ui);
            // The display takes what the window has beyond the controls under it, measured last
            // frame. Only dragging the window's edge changes its size: nothing here makes it grow.
            let below_id = egui::Id::unique(("fx_below", ti, slot));
            let measured: Option<f32> = ui.ctx().data(|d| d.get_temp(below_id));
            let y0 = ui.cursor().top();
            let mut high_used = 0.0;
            if view(edited.kind) != View::Nothing {
                let high = match measured {
                    Some(below) => (ui.available_height() - below).max(DISPLAY_H),
                    None => DISPLAY_H,
                };
                display(ui, lk, edited.kind, &shown, reduction, edited.bypass, band, heard, high, |e| {
                    edits.push(e)
                });
                high_used = high;
                ui.add_space(12.0);
            }
            if edited.kind == FxKind::Prism {
                band_buttons(ui, lk, &shown);
                ui.add_space(8.0);
            }
            if edited.kind.takes_side() {
                side_chain_row(ui, lk, &mut edited.sidechain, &keys, ti);
            }
            choice_row(ui, lk, edited.kind, &shown, track, slot, &mut edits, ui.available_width().min(width));
            let drawn = knob_rows(ui, lk, edited.kind, &shown, track, slot, band, &mut edits);
            let natural = drawn / lk.scale;
            ui.ctx().data_mut(|d| d.insert_temp(natural_id, natural));
            // Everything after the display: the gap, the choices and the knobs, spacing and all.
            let below = (ui.min_rect().bottom() - y0 - high_used).max(0.0);
            ui.ctx().data_mut(|d| d.insert_temp(below_id, below));
        });
    if edited != insert {
        edited.params = edited.kind.clamp_params(edited.params);
        if let Some(t) = p.strip_mut(ti) {
            t.inserts[slot] = edited;
        }
    }
    let playing = p.playing;
    for e in edits {
        let target = AutoTarget::Fx {
            slot: slot as u8,
            param: e.param as u8,
        };
        let Some(t) = p.strip_mut(ti) else {
            continue;
        };
        automation::moved(t, target, e.value);
        if e.ended {
            automation::released(t, target, beat, playing);
        }
    }
    open
}

/// Text with space between the letters, for names.
fn spaced(
    ui: &Ui,
    text: &str,
    size: f32,
    color: Color32,
    spacing: f32,
    display_face: usize,
) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::default();
    job.append(
        text,
        0.0,
        TextFormat {
            font_id: theme::own_font(size, display_face),
            color,
            extra_letter_spacing: spacing,
            ..Default::default()
        },
    );
    ui.painter().layout_job(job)
}

/// Where the presets of an effect are kept: a folder for each effect in the settings folder.
fn preset_dir(kind: FxKind) -> Option<std::path::PathBuf> {
    crate::recent_files::config_dir().map(|d| d.join("fx-presets").join(kind.name()))
}

/// A name that is safe as a file's: letters, digits, spaces, dashes and underscores.
fn preset_file_name(name: &str) -> String {
    let clean: String = name
        .trim()
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        .take(60)
        .collect();
    clean.trim().to_string()
}

fn preset_names(kind: FxKind) -> Vec<String> {
    let Some(dir) = preset_dir(kind) else {
        return Vec::new();
    };
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|it| {
            it.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "txt"))
                .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
                .collect()
        })
        .unwrap_or_default();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

/// The settings a preset holds, put into the effect's ranges; settings it does not hold (an older
/// preset) are the defaults.
fn load_preset(kind: FxKind, name: &str) -> Option<Params> {
    let file = preset_dir(kind)?.join(format!("{name}.txt"));
    let text = std::fs::read_to_string(file).ok()?;
    let mut params = kind.defaults();
    for (slot, word) in params.iter_mut().zip(text.split_whitespace()) {
        if let Ok(v) = word.parse::<f32>() {
            *slot = v;
        }
    }
    Some(kind.clamp_params(params))
}

fn save_preset(kind: FxKind, name: &str, params: &Params) -> Result<(), String> {
    let name = preset_file_name(name);
    if name.is_empty() {
        return Err("Give the preset a name".to_string());
    }
    let dir = preset_dir(kind).ok_or("no settings folder")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let text: Vec<String> = params[..kind.params().len()]
        .iter()
        .map(|v| v.to_string())
        .collect();
    std::fs::write(dir.join(format!("{name}.txt")), text.join(" ")).map_err(|e| e.to_string())
}

/// The presets of the effect: choose one, or name the settings as they are and save them.
fn preset_row(ui: &mut Ui, lk: &Look, kind: FxKind, params: &mut Params) {
    let id = egui::Id::unique(("fx_preset_name", kind.name()));
    let mut name: String = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_default();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("PRESET").size(10.5).color(lk.dim));
        let shown = if name.is_empty() {
            "Choose…".to_string()
        } else {
            name.clone()
        };
        egui::ComboBox::from_id_salt(("fx_presets", kind.name()))
            .selected_text(shown)
            .width(170.0)
            .show_ui(ui, |ui| {
                // Read only while the list is open.
                let names = preset_names(kind);
                if names.is_empty() {
                    ui.weak("None saved yet");
                }
                for n in names {
                    ui.horizontal(|ui| {
                        if ui.selectable_label(n == name, n.as_str()).clicked() {
                            if let Some(loaded) = load_preset(kind, &n) {
                                *params = loaded;
                                name = n.clone();
                            }
                        }
                        if ui.small_button("Delete").clicked() {
                            if let Some(dir) = preset_dir(kind) {
                                let _ = std::fs::remove_file(dir.join(format!("{n}.txt")));
                            }
                            if name == n {
                                name.clear();
                            }
                        }
                    });
                }
            });
        ui.add(
            egui::TextEdit::singleline(&mut name)
                .hint_text("Name")
                .desired_width(130.0),
        );
        let can_save = !preset_file_name(&name).is_empty();
        if ui
            .add_enabled(can_save, egui::Button::new("Save"))
            .on_hover_text("Keeps these settings under this name, for every project")
            .clicked()
        {
            if save_preset(kind, &name, params).is_ok() {
                name = preset_file_name(&name);
            }
        }
    });
    ui.ctx().data_mut(|d| d.insert_temp(id, name));
}

/// The header: power (on is lit; off bypasses), the name and what it is, Reset and Close.
fn header(ui: &mut Ui, lk: &Look, kind: FxKind, bypass: &mut bool) -> (bool, bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    let g = ui.painter().clone();
    let power = Rect::from_center_size(pos2(rect.left() + 13.0, rect.center().y), vec2(26.0, 26.0));
    let resp = ui
        .interact(power, ui.scope_id().with("fx_power"), Sense::click())
        .on_hover_text(if *bypass { "Turn on" } else { "Bypass" });
    if resp.clicked() {
        *bypass = !*bypass;
    }
    let on = !*bypass;
    let col = if on { lk.accent } else { lk.dim };
    g.circle_filled(power.center(), 12.0, lk.display);
    g.circle_stroke(power.center(), 12.0, Stroke::new(1.0, lk.edge));
    // The power sign: a ring open at the top and a stroke through the gap.
    let c = power.center();
    let ring: Vec<Pos2> = (0..=24)
        .map(|i| {
            let a = (-60.0 + 300.0 * i as f32 / 24.0 - 90.0f32).to_radians();
            c + vec2(a.cos(), a.sin()) * 6.0
        })
        .collect();
    g.add(Shape::line(ring, Stroke::new(1.6, col)));
    g.line_segment([c + vec2(0.0, -8.0), c + vec2(0.0, -2.0)], Stroke::new(1.6, col));
    let name = spaced(ui, &kind.name().to_uppercase(), 15.0, if on { lk.text } else { lk.dim }, 2.2, 0);
    let name_w = name.size().x;
    g.galley(pos2(power.right() + 10.0, rect.center().y - name.size().y / 2.0), name, lk.text);
    let word = spaced(ui, &kind.kind_word().to_uppercase(), 11.0, lk.dim, 1.6, 1);
    g.galley(
        pos2(power.right() + 22.0 + name_w, rect.center().y - word.size().y / 2.0 + 1.0),
        word,
        lk.dim,
    );
    let button = |icon: Icon, at: f32, hint: &str, id: &str| {
        let r = Rect::from_center_size(pos2(at, rect.center().y), vec2(24.0, 24.0));
        let resp = ui.interact(r, ui.scope_id().with(id), Sense::click()).on_hover_text(hint);
        let col = if resp.hovered() { lk.text } else { lk.dim };
        if resp.hovered() {
            g.rect_filled(r, 6.0, lk.display);
        }
        icons::paint(&g, r.center(), 15.0, icon, col);
        resp.clicked()
    };
    let close = button(Icon::Close, rect.right() - 12.0, "Close", "fx_close");
    let reset = button(Icon::Refresh, rect.right() - 42.0, "Reset to the defaults", "fx_reset");
    (reset, close)
}

/// The display: the picture of what the settings do, and a reduction meter when it reduces.
#[allow(clippy::too_many_arguments)]
fn display(
    ui: &mut Ui,
    lk: &Look,
    kind: FxKind,
    p: &Params,
    reduction: f32,
    bypassed: bool,
    band: usize,
    heard: Option<&Listen>,
    high: f32,
    mut edit: impl FnMut(Edit),
) {
    let (outer, resp) = ui.allocate_exact_size(
        vec2(ui.available_width(), high),
        Sense::click_and_drag(),
    );
    let g = ui.painter_at(outer);
    g.rect_filled(outer, 8.0, lk.display);
    g.rect_stroke(outer, 8.0, Stroke::new(1.0, lk.edge), StrokeKind::Inside);
    let meter_w = if reduces(kind) { 30.0 } else { 0.0 };
    let rect = Rect::from_min_max(
        outer.min + vec2(10.0, 10.0),
        outer.max - vec2(10.0 + meter_w, 10.0),
    );
    if meter_w > 0.0 {
        let m = Rect::from_min_max(
            pos2(rect.right() + 10.0, rect.top()),
            pos2(outer.right() - 10.0, rect.bottom()),
        );
        reduction_meter(&g, lk, m, if bypassed { 0.0 } else { reduction });
    }
    let faint = if bypassed { 0.45 } else { 1.0 };
    let accent = lk.accent.gamma_multiply(faint);
    // What the effect hears: a spectrum behind the frequency views, levels at the side of the rest.
    let rect = match heard {
        Some(_) if matches!(view(kind), View::Scope | View::Meter) => rect,
        Some(h) if matches!(view(kind), View::Eq | View::Focus) => {
            spectrum_layer(&g, lk, rect, h);
            rect
        }
        Some(h) => {
            io_meters(&g, lk, Rect::from_min_max(rect.min, pos2(rect.left() + 22.0, rect.bottom())), h);
            let rest = Rect::from_min_max(pos2(rect.left() + 30.0, rect.top()), rect.max);
            wave_layer(&g, lk, rest, h, accent);
            rest
        }
        None => rect,
    };
    match view(kind) {
        View::Eq => eq_view(ui, &g, lk, rect, kind, p, accent, band, &resp, &mut edit),
        View::Transfer => transfer_view(&g, lk, rect, kind, p, accent, heard),
        View::Shape => shape_view(&g, lk, rect, kind, p, accent, heard),
        View::Envelope => envelope_view(&g, lk, rect, p, accent),
        View::Echoes => echoes_view(&g, lk, rect, p, accent),
        View::Decay => decay_view(&g, lk, rect, p, accent),
        View::Sweep => sweep_view(&g, lk, rect, p, accent),
        View::Focus => focus_view(&g, lk, rect, p, accent, &resp, &mut edit),
        View::Wave => {
            if heard.is_none() {
                small_text(&g, rect.center(), Align2::CENTER_CENTER, "Play to see the sound", lk.dim);
            }
        }
        View::Scope => scope_view(&g, lk, rect, heard, accent),
        View::Meter => meter_view(&g, lk, rect, p, heard, accent),
        View::Nothing => {}
    }
}

fn small_text(g: &egui::Painter, at: Pos2, align: Align2, text: &str, col: Color32) {
    g.text(at, align, text, FontId::proportional(11.0), col);
}

/// Gain reduction from the top down, 0 to 24 dB, with its number.
fn reduction_meter(g: &egui::Painter, lk: &Look, r: Rect, db: f32) {
    let bar = Rect::from_min_max(pos2(r.center().x - 4.0, r.top() + 14.0), pos2(r.center().x + 4.0, r.bottom()));
    g.rect_filled(bar, 3.0, lk.grid);
    let shown = db.clamp(0.0, 24.0);
    let fill = Rect::from_min_max(bar.min, pos2(bar.right(), bar.top() + bar.height() * shown / 24.0));
    g.rect_filled(fill, 3.0, theme::SOLO);
    small_text(g, pos2(r.center().x, r.top()), Align2::CENTER_TOP, &format!("{shown:.1}"), lk.dim);
    for d in [6.0f32, 12.0, 18.0] {
        let y = bar.top() + bar.height() * d / 24.0;
        g.line_segment([pos2(bar.left() - 3.0, y), pos2(bar.left() - 1.0, y)], Stroke::new(1.0, lk.dim));
    }
}

fn freq_x(rect: Rect, hz: f32) -> f32 {
    rect.left() + (hz.max(20.0) / 20.0).log10() / 3.0 * rect.width()
}

fn x_freq(rect: Rect, x: f32) -> f32 {
    20.0 * 1000f32.powf(((x - rect.left()) / rect.width()).clamp(0.0, 1.0))
}

fn freq_grid(g: &egui::Painter, lk: &Look, rect: Rect) {
    for hz in [50.0f32, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0] {
        let x = freq_x(rect, hz);
        g.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(1.0, lk.grid));
        let text = if hz >= 1000.0 {
            format!("{}k", hz / 1000.0)
        } else {
            format!("{hz}")
        };
        small_text(g, pos2(x + 2.0, rect.bottom() - 1.0), Align2::LEFT_BOTTOM, &text, lk.dim);
    }
}

/// A filled curve: the line, and a soft fill down (or up) to `base_y`.
fn filled_curve(g: &egui::Painter, pts: &[Pos2], base_y: f32, col: Color32) {
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        g.add(Shape::convex_polygon(
            vec![a, b, pos2(b.x, base_y), pos2(a.x, base_y)],
            col.gamma_multiply(0.16),
            Stroke::NONE,
        ));
    }
    g.add(Shape::line(pts.to_vec(), Stroke::new(2.0, col)));
}

#[allow(clippy::too_many_arguments)]
fn eq_view(
    ui: &Ui,
    g: &egui::Painter,
    lk: &Look,
    rect: Rect,
    kind: FxKind,
    p: &Params,
    accent: Color32,
    band: usize,
    resp: &egui::Response,
    edit: &mut impl FnMut(Edit),
) {
    let range = if kind == FxKind::Prism { 24.0 } else { 18.0 };
    let y_of = |db: f32| rect.center().y - db.clamp(-range, range) / range * rect.height() / 2.0;
    freq_grid(g, lk, rect);
    for db in [-12.0f32, 0.0, 12.0] {
        let y = y_of(db);
        let col = if db == 0.0 { lk.dim.gamma_multiply(0.6) } else { lk.grid };
        g.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], Stroke::new(1.0, col));
        small_text(g, pos2(rect.left() + 2.0, y - 1.0), Align2::LEFT_BOTTOM, &format!("{db:+.0}"), lk.dim);
    }
    let n = 180;
    let freqs: Vec<f32> = (0..=n).map(|i| 20.0 * 1000f32.powf(i as f32 / n as f32)).collect();
    if kind == FxKind::Prism && (0..6).any(|k| p[k * 5 + 3].abs() >= 0.05) {
        // How far the dynamic bands can move the curve.
        let reach = ovrtone_engine::prism_reach_db(*p, &freqs, CURVE_RATE);
        let pts: Vec<Pos2> = freqs.iter().zip(&reach).map(|(f, d)| pos2(freq_x(rect, *f), y_of(*d))).collect();
        g.extend(Shape::dashed_line(&pts, Stroke::new(1.2, accent.gamma_multiply(0.7)), 4.0, 3.0));
    }
    let curve = ovrtone_engine::response_db(kind, *p, &freqs, CURVE_RATE);
    let pts: Vec<Pos2> = freqs.iter().zip(&curve).map(|(f, d)| pos2(freq_x(rect, *f), y_of(*d))).collect();
    filled_curve(g, &pts, y_of(0.0), accent);
    if kind != FxKind::Prism {
        return;
    }
    // The bands as points to drag (frequency and gain); the wheel sets the Q.
    let node = |k: usize| pos2(freq_x(rect, p[k * 5]), y_of(p[k * 5 + 1]));
    let hover = ui.ctx().pointer_hover_pos().filter(|h| rect.expand(8.0).contains(*h));
    let near = |at: Pos2| (0..6).min_by(|a, b| node(*a).distance(at).total_cmp(&node(*b).distance(at))).filter(|k| node(*k).distance(at) < 14.0);
    if resp.drag_started() || resp.clicked() {
        if let Some(k) = resp.interact_pointer_pos().and_then(near) {
            choose_band(ui, k);
        }
    }
    let chosen = chosen_band(ui);
    if resp.dragged() {
        if let Some(at) = resp.interact_pointer_pos() {
            let hz = x_freq(rect, at.x).clamp(20.0, 20_000.0);
            let db = ((rect.center().y - at.y) / (rect.height() / 2.0) * range).clamp(-24.0, 24.0);
            edit(Edit { param: chosen * 5, value: hz, ended: false });
            edit(Edit { param: chosen * 5 + 1, value: db, ended: false });
        }
    }
    if resp.drag_stopped() {
        edit(Edit { param: chosen * 5, value: p[chosen * 5], ended: true });
        edit(Edit { param: chosen * 5 + 1, value: p[chosen * 5 + 1], ended: true });
    }
    if hover.is_some() {
        let wheel = ui.ctx().input(|i| i.smooth_scroll_delta().y);
        if wheel != 0.0 {
            let q = (p[chosen * 5 + 2] * 1.1f32.powf(wheel.signum())).clamp(0.1, 10.0);
            edit(Edit { param: chosen * 5 + 2, value: q, ended: true });
        }
    }
    for k in 0..6 {
        let c = node(k);
        let sel = k == band;
        let col = band_color(k);
        if p[k * 5 + 3].abs() >= 0.05 {
            // A dynamic band: a line to where it can go.
            let to = pos2(c.x, y_of(p[k * 5 + 1] + p[k * 5 + 3]));
            g.line_segment([c, to], Stroke::new(1.5, col.gamma_multiply(0.7)));
            g.circle_stroke(to, 3.0, Stroke::new(1.0, col));
        }
        g.circle_filled(c, if sel { 7.5 } else { 6.0 }, col.gamma_multiply(if sel { 1.0 } else { 0.75 }));
        g.circle_stroke(c, if sel { 7.5 } else { 6.0 }, Stroke::new(1.2, lk.display));
        g.text(c, Align2::CENTER_CENTER, format!("{}", k + 1), FontId::proportional(9.0), lk.display);
    }
}

/// How many samples the analyzer looks at, and how many bands it shows.
const LISTEN_N: usize = 4096;
const LISTEN_BANDS: usize = 120;

/// What the window in front hears from its effect: the input and output, as levels and spectra.
pub struct Listen {
    pub key: (usize, usize),
    rx: rtrb::Consumer<f32>,
    rate: u32,
    pre: Vec<f32>,
    post: Vec<f32>,
    at: usize,
    fresh: usize,
    /// Seconds since anything came.
    quiet: f32,
    since_frame: f32,
    pub pre_bands: Vec<f32>,
    pub post_bands: Vec<f32>,
    pub in_db: f32,
    pub out_db: f32,
    /// The loudest input and output of each short block, oldest first from `hist_at`.
    pub history: Vec<(f32, f32)>,
    pub hist_at: usize,
    block: (f32, f32, usize),
    /// Loudness, peaks and the stereo image of the output.
    pub stereo: Stereo,
}

impl Listen {
    /// Room for a quarter of a second of input and output.
    pub fn capacity(rate: u32) -> usize {
        (rate as usize / 4).max(1024) * 3
    }

    pub fn new(key: (usize, usize), rx: rtrb::Consumer<f32>, rate: u32) -> Self {
        Self {
            key,
            rx,
            rate,
            pre: vec![0.0; LISTEN_N],
            post: vec![0.0; LISTEN_N],
            at: 0,
            fresh: 0,
            quiet: 0.0,
            since_frame: 0.0,
            pre_bands: vec![-120.0; LISTEN_BANDS],
            post_bands: vec![-120.0; LISTEN_BANDS],
            in_db: -120.0,
            out_db: -120.0,
            history: vec![(0.0, 0.0); HISTORY],
            hist_at: 0,
            block: (0.0, 0.0, 0),
            stereo: Stereo::new(rate),
        }
    }

    /// Nothing has come for a while: the engine did not take the tap.
    pub fn lost(&self) -> bool {
        self.quiet > 1.5
    }

    /// Takes what came since the last frame, `dt` seconds ago.
    pub fn update(&mut self, dt: f32) {
        let mut peak_in = 0f32;
        let mut peak_out = 0f32;
        let mut got = 0;
        while self.rx.slots() >= 3 {
            let (Ok(a), Ok(ol), Ok(or)) = (self.rx.pop(), self.rx.pop(), self.rx.pop()) else {
                break;
            };
            let b = (ol + or) * 0.5;
            self.stereo.add(ol, or);
            self.pre[self.at] = a;
            self.post[self.at] = b;
            self.at = (self.at + 1) % LISTEN_N;
            peak_in = peak_in.max(a.abs());
            peak_out = peak_out.max(b.abs());
            got += 1;
            let (bi, bo, bn) = &mut self.block;
            *bi = bi.max(a.abs());
            *bo = bo.max(b.abs());
            *bn += 1;
            if *bn >= HISTORY_BLOCK {
                self.history[self.hist_at] = (*bi, *bo);
                self.hist_at = (self.hist_at + 1) % HISTORY;
                self.block = (0.0, 0.0, 0);
            }
        }
        self.quiet = if got > 0 { 0.0 } else { self.quiet + dt };
        self.fresh += got;
        // Meters: up at once, down 24 dB a second.
        let fall = 24.0 * dt;
        let db = |v: f32| 20.0 * v.max(1e-6).log10();
        self.in_db = db(peak_in).max(self.in_db - fall);
        self.out_db = db(peak_out).max(self.out_db - fall);
        // The spectra about 30 times a second, falling smoothly between.
        self.since_frame += dt;
        if self.fresh >= LISTEN_N / 8 && self.since_frame >= 0.033 {
            self.since_frame = 0.0;
            self.fresh = 0;
            let ordered = |ring: &[f32]| -> Vec<f32> {
                ring[self.at..].iter().chain(&ring[..self.at]).copied().collect()
            };
            let fall = 30.0 * 0.033;
            for (bands, ring) in [(&mut self.pre_bands, &self.pre), (&mut self.post_bands, &self.post)] {
                let now = crate::mastering::spectrum(&ordered(ring), self.rate, LISTEN_BANDS);
                for (old, new) in bands.iter_mut().zip(now) {
                    *old = if new > *old { *old + (new - *old) * 0.6 } else { (*old - fall).max(new) };
                }
            }
        }
    }

    /// The middle frequency of band `b`.
    fn band_hz(&self, b: usize) -> f32 {
        let top = self.rate as f32 / 2.0;
        20.0 * (top / 20.0).powf((b as f32 + 0.5) / LISTEN_BANDS as f32)
    }
}

/// How many short blocks the waveform keeps, and how many samples each is.
const HISTORY: usize = 360;
const HISTORY_BLOCK: usize = 256;

/// The sound going by, behind a display: the input as a soft grey shape, the output in the accent
/// over it, newest on the right.
fn wave_layer(g: &egui::Painter, lk: &Look, rect: Rect, h: &Listen, accent: Color32) {
    let mid = rect.center().y;
    let half = rect.height() * 0.5;
    // Levels in dB from -48 to 0, so quiet parts still show.
    let size = |v: f32| ((20.0 * v.max(1e-6).log10() + 48.0) / 48.0).clamp(0.0, 1.0) * half;
    let n = h.history.len();
    let step = rect.width() / n.max(1) as f32;
    for (pick, col) in [(0usize, lk.dim.gamma_multiply(0.22)), (1, accent.gamma_multiply(0.30))] {
        for i in 0..n {
            let (a, b) = h.history[(h.hist_at + i) % n];
            let v = if pick == 0 { a } else { b };
            let y = size(v);
            if y < 0.5 {
                continue;
            }
            let x = rect.left() + i as f32 * step;
            g.rect_filled(
                Rect::from_min_max(pos2(x, mid - y), pos2(x + step + 0.5, mid + y)),
                0.0,
                col,
            );
        }
    }
}

/// How many frames the goniometer shows.
const SCOPE_N: usize = 768;

/// What the Meter and Width windows read from the output: loudness (BS.1770, as the Mastering window
/// measures it), the true peak, how alike the two sides are, and the last frames for the scope.
pub struct Stereo {
    pub loudness: crate::mastering::Loudness,
    pub peak: crate::mastering::TruePeak,
    /// The running sums of left times right and of each squared, slowly forgetting.
    lr: f32,
    ll: f32,
    rr: f32,
    pub scope: Vec<(f32, f32)>,
    at: usize,
}

impl Stereo {
    pub fn new(rate: u32) -> Self {
        Self {
            loudness: crate::mastering::Loudness::new(rate),
            peak: crate::mastering::TruePeak::default(),
            lr: 0.0,
            ll: 0.0,
            rr: 0.0,
            scope: vec![(0.0, 0.0); SCOPE_N],
            at: 0,
        }
    }

    fn add(&mut self, l: f32, r: f32) {
        self.loudness.add([l, r]);
        self.peak.add([l, r]);
        let k = 0.9995;
        self.lr = self.lr * k + l * r;
        self.ll = self.ll * k + l * l;
        self.rr = self.rr * k + r * r;
        self.scope[self.at] = (l, r);
        self.at = (self.at + 1) % SCOPE_N;
    }

    /// -1 (opposite) to +1 (the same, mono); 0 for silence.
    pub fn correlation(&self) -> f32 {
        let d = (self.ll * self.rr).sqrt();
        if d < 1e-9 {
            0.0
        } else {
            (self.lr / d).clamp(-1.0, 1.0)
        }
    }
}

/// Left against right: a mono sound is a vertical line, wide sounds spread sideways.
fn scope_view(g: &egui::Painter, lk: &Look, rect: Rect, heard: Option<&Listen>, accent: Color32) {
    let side = rect.height().min(rect.width() * 0.6);
    let sq = Rect::from_center_size(pos2(rect.left() + side * 0.5 + 4.0, rect.center().y), vec2(side, side));
    let c = sq.center();
    g.line_segment([pos2(c.x, sq.top()), pos2(c.x, sq.bottom())], Stroke::new(1.0, lk.grid));
    g.line_segment([pos2(sq.left(), c.y), pos2(sq.right(), c.y)], Stroke::new(1.0, lk.grid));
    g.circle_stroke(c, side * 0.5, Stroke::new(1.0, lk.grid));
    small_text(g, sq.left_top(), Align2::LEFT_TOP, "L", lk.dim);
    small_text(g, sq.right_top(), Align2::RIGHT_TOP, "R", lk.dim);
    let Some(h) = heard else {
        small_text(g, c, Align2::CENTER_CENTER, "Play to see the image", lk.dim);
        return;
    };
    let s = &h.stereo;
    let half = side * 0.5;
    for &(l, r) in &s.scope {
        // Turned 45°: the middle up, the sides across.
        let (m, d) = ((l + r) * 0.707, (l - r) * 0.707);
        let at = c + vec2(-d, -m) * half;
        if sq.contains(at) {
            g.circle_filled(at, 1.0, accent.gamma_multiply(0.6));
        }
    }
    corr_bar(g, lk, Rect::from_min_max(pos2(sq.right() + 24.0, rect.bottom() - 20.0), pos2(rect.right(), rect.bottom() - 6.0)), s.correlation(), accent);
}

/// How alike the sides are, -1 to +1, as a bar from the middle.
fn corr_bar(g: &egui::Painter, lk: &Look, r: Rect, c: f32, accent: Color32) {
    g.rect_filled(r, 3.0, lk.grid);
    let mid = r.center().x;
    let x = mid + c * r.width() * 0.5;
    let col = if c < 0.0 { theme::DANGER } else { accent };
    g.rect_filled(Rect::from_min_max(pos2(mid.min(x), r.top()), pos2(mid.max(x), r.bottom())), 3.0, col);
    small_text(g, pos2(r.left(), r.top() - 2.0), Align2::LEFT_BOTTOM, &format!("CORRELATION {c:+.2}"), lk.text);
}

/// Loudness against a target, the true peak, the spectrum and the image.
fn meter_view(g: &egui::Painter, lk: &Look, rect: Rect, p: &Params, heard: Option<&Listen>, accent: Color32) {
    let Some(h) = heard else {
        small_text(g, rect.center(), Align2::CENTER_CENTER, "Play to measure", lk.dim);
        return;
    };
    let split = rect.left() + rect.width() * 0.55;
    let spec = Rect::from_min_max(rect.min, pos2(split - 10.0, rect.bottom()));
    freq_grid(g, lk, spec);
    spectrum_layer(g, lk, spec, h);
    let s = &h.stereo;
    let rows = [
        ("MOMENTARY", s.loudness.momentary()),
        ("SHORT TERM", s.loudness.short_term()),
        ("INTEGRATED", s.loudness.integrated()),
    ];
    let x = split + 4.0;
    let mut y = rect.top();
    for (name, v) in rows {
        let text = if v <= -69.0 { "-inf".to_string() } else { format!("{v:.1} LUFS") };
        small_text(g, pos2(x, y), Align2::LEFT_TOP, name, lk.dim);
        g.text(pos2(rect.right(), y - 2.0), Align2::RIGHT_TOP, text, FontId::proportional(16.0), lk.text);
        y += 26.0;
    }
    let target = p[0];
    let off = s.loudness.integrated() - f64::from(target);
    small_text(g, pos2(x, y), Align2::LEFT_TOP, &format!("TARGET {target:.0} LUFS"), lk.dim);
    if s.loudness.integrated() > -69.0 {
        let col = if off.abs() <= 1.0 { theme::GOOD } else { theme::SOLO };
        g.text(pos2(rect.right(), y - 2.0), Align2::RIGHT_TOP, format!("{off:+.1} dB"), FontId::proportional(14.0), col);
    }
    y += 24.0;
    let tp = 20.0 * s.peak.peak.max(1e-6).log10();
    small_text(g, pos2(x, y), Align2::LEFT_TOP, "TRUE PEAK", lk.dim);
    let col = if tp > -1.0 { theme::DANGER } else { lk.text };
    g.text(pos2(rect.right(), y - 2.0), Align2::RIGHT_TOP, format!("{tp:.1} dBTP"), FontId::proportional(14.0), col);
    corr_bar(g, lk, Rect::from_min_max(pos2(x, rect.bottom() - 14.0), pos2(rect.right(), rect.bottom() - 2.0)), s.correlation(), accent);
}

/// The spectra behind a frequency view: the input as a line, the output filled.
fn spectrum_layer(g: &egui::Painter, lk: &Look, rect: Rect, h: &Listen) {
    let y_of = |db: f32| rect.bottom() - ((db + 90.0) / 90.0).clamp(0.0, 1.0) * rect.height();
    let points = |bands: &[f32]| -> Vec<Pos2> {
        bands
            .iter()
            .enumerate()
            .filter(|(b, _)| h.band_hz(*b) <= 20_000.0)
            .map(|(b, db)| pos2(freq_x(rect, h.band_hz(b)), y_of(*db)))
            .collect()
    };
    let post = points(&h.post_bands);
    let fill = lk.accent.gamma_multiply(0.13);
    for w in post.windows(2) {
        g.add(Shape::convex_polygon(
            vec![w[0], w[1], pos2(w[1].x, rect.bottom()), pos2(w[0].x, rect.bottom())],
            fill,
            Stroke::NONE,
        ));
    }
    g.add(Shape::line(post, Stroke::new(1.2, lk.text.gamma_multiply(0.7))));
    g.add(Shape::line(points(&h.pre_bands), Stroke::new(1.0, lk.dim.gamma_multiply(0.6))));
}

/// Input and output levels side by side, -60 to 0 dB.
fn io_meters(g: &egui::Painter, lk: &Look, r: Rect, h: &Listen) {
    let top = r.top() + 14.0;
    for (k, db) in [h.in_db, h.out_db].into_iter().enumerate() {
        let x = r.left() + 4.0 + k as f32 * 10.0;
        let bar = Rect::from_min_max(pos2(x, top), pos2(x + 6.0, r.bottom()));
        g.rect_filled(bar, 2.0, lk.grid);
        let n = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
        let lit = Rect::from_min_max(pos2(bar.left(), bar.bottom() - bar.height() * n), bar.max);
        let col = if db > -0.5 { theme::DANGER } else { theme::GOOD };
        g.rect_filled(lit, 2.0, col);
    }
    small_text(g, pos2(r.left() + 2.0, r.top()), Align2::LEFT_TOP, "I O", lk.dim);
}

/// Each EQ band has a colour of its own, the same in the display and on its button.
fn band_color(k: usize) -> Color32 {
    const COLS: [Color32; 6] = [
        Color32::from_rgb(0xe8, 0x5d, 0x5d),
        Color32::from_rgb(0xf0, 0x9a, 0x3e),
        Color32::from_rgb(0xe6, 0xd0, 0x4a),
        crate::theme::GOOD,
        Color32::from_rgb(0x4a, 0xb5, 0xe6),
        Color32::from_rgb(0xa3, 0x7c, 0xf0),
    ];
    COLS[k.min(5)]
}

fn transfer_view(
    g: &egui::Painter,
    lk: &Look,
    rect: Rect,
    kind: FxKind,
    p: &Params,
    accent: Color32,
    heard: Option<&Listen>,
) {
    // Input and output from -60 to +6 dB, in a square.
    let side = rect.height();
    let r = Rect::from_min_size(pos2(rect.left() + (rect.width() - side) * 0.5, rect.top()), vec2(side, side));
    let to = |db: f32| (db.clamp(-60.0, 6.0) + 60.0) / 66.0;
    let at = |i: f32, o: f32| pos2(r.left() + to(i) * r.width(), r.bottom() - to(o) * r.height());
    for d in [-48.0f32, -36.0, -24.0, -12.0, 0.0] {
        let x = at(d, d);
        g.line_segment([pos2(x.x, r.top()), pos2(x.x, r.bottom())], Stroke::new(1.0, lk.grid));
        g.line_segment([pos2(r.left(), x.y), pos2(r.right(), x.y)], Stroke::new(1.0, lk.grid));
        small_text(g, pos2(x.x + 2.0, r.bottom() - 1.0), Align2::LEFT_BOTTOM, &format!("{d:.0}"), lk.dim);
    }
    g.line_segment([at(-60.0, -60.0), at(6.0, 6.0)], Stroke::new(1.0, lk.dim.gamma_multiply(0.5)));
    let pts: Vec<Pos2> = (0..=132).map(|i| {
        let x = -60.0 + i as f32 * 0.5;
        at(x, ovrtone_engine::curve_point(kind, *p, x))
    }).collect();
    filled_curve(g, &pts, r.bottom(), accent);
    if let Some(h) = heard.filter(|h| h.in_db > -60.0) {
        // Where the sound is on the curve now.
        let dot = at(h.in_db, ovrtone_engine::curve_point(kind, *p, h.in_db));
        g.circle_filled(dot, 5.0, lk.text);
        g.circle_stroke(dot, 8.0, Stroke::new(1.5, accent));
    }
    let (label, level) = match kind {
        FxKind::Wall => ("Ceiling", p[1]),
        _ => ("Threshold", p[0]),
    };
    let y = at(level, level).y;
    g.extend(Shape::dashed_line(&[pos2(r.left(), y), pos2(r.right(), y)], Stroke::new(1.0, lk.dim), 3.0, 3.0));
    small_text(g, pos2(r.right() + 6.0, y), Align2::LEFT_CENTER, &format!("{label} {level:.1} dB"), lk.text);
    let side_text = match kind {
        FxKind::Press => format!("{:.1}:1   knee {:.0} dB", p[1], p[2]),
        _ => format!("+{:.1} dB in", p[0]),
    };
    small_text(g, pos2(rect.left() + 2.0, rect.top()), Align2::LEFT_TOP, &side_text, lk.dim);
}

fn shape_view(
    g: &egui::Painter,
    lk: &Look,
    rect: Rect,
    kind: FxKind,
    p: &Params,
    accent: Color32,
    heard: Option<&Listen>,
) {
    let side = rect.height();
    let r = Rect::from_min_size(pos2(rect.left() + (rect.width() - side) * 0.5, rect.top()), vec2(side, side));
    let at = |x: f32, y: f32| pos2(r.center().x + x * r.width() * 0.5, r.center().y - y.clamp(-1.0, 1.0) * r.height() * 0.5);
    for v in [-0.5f32, 0.0, 0.5] {
        let a = at(v, v);
        g.line_segment([pos2(a.x, r.top()), pos2(a.x, r.bottom())], Stroke::new(1.0, lk.grid));
        g.line_segment([pos2(r.left(), a.y), pos2(r.right(), a.y)], Stroke::new(1.0, lk.grid));
    }
    g.line_segment([at(-1.0, -1.0), at(1.0, 1.0)], Stroke::new(1.0, lk.dim.gamma_multiply(0.5)));
    let pts: Vec<Pos2> = (0..=200).map(|i| {
        let x = -1.0 + i as f32 / 100.0;
        at(x, ovrtone_engine::curve_point(kind, *p, x))
    }).collect();
    g.add(Shape::line(pts, Stroke::new(2.2, accent)));
    if let Some(h) = heard.filter(|h| h.in_db > -60.0) {
        // How far up the curve the sound reaches now.
        let x = 10f32.powf(h.in_db / 20.0).min(1.0);
        let dot = at(x, ovrtone_engine::curve_point(kind, *p, x));
        g.circle_filled(dot, 5.0, lk.text);
        g.circle_stroke(dot, 8.0, Stroke::new(1.5, accent));
    }
    let name = match kind {
        FxKind::Ember => kind.choices(2).and_then(|c| c.get(p[2].round() as usize)).copied(),
        FxKind::Heat => kind.choices(4).and_then(|c| c.get(p[4].round() as usize)).copied(),
        FxKind::Clip => kind.choices(2).and_then(|c| c.get(p[2].round() as usize)).copied(),
        _ => None,
    };
    if let Some(n) = name {
        small_text(g, pos2(rect.left() + 2.0, rect.top()), Align2::LEFT_TOP, &n.to_uppercase(), lk.text);
    }
    if kind == FxKind::Clip {
        let c = 10f32.powf(p[1] / 20.0);
        for y in [c, -c] {
            let at_y = at(0.0, y).y;
            g.extend(Shape::dashed_line(&[pos2(r.left(), at_y), pos2(r.right(), at_y)], Stroke::new(1.0, theme::DANGER.gamma_multiply(0.7)), 3.0, 3.0));
        }
    }
}

fn envelope_view(g: &egui::Painter, lk: &Look, rect: Rect, p: &Params, accent: Color32) {
    // A drum hit as it comes in (dim) and as Snap leaves it.
    let hit = |t: f32| if t < 0.04 { t / 0.04 } else { (-(t - 0.04) * 7.0).exp() };
    let attack = 10f32.powf(p[0] * 0.01 * 12.0 / 20.0);
    let sustain = 10f32.powf(p[1] * 0.01 * 12.0 / 20.0);
    let shaped = |t: f32| {
        let a = if t < 0.12 { 1.0 + (attack - 1.0) * (1.0 - t / 0.12) } else { 1.0 };
        let s = if t > 0.08 { 1.0 + (sustain - 1.0) * ((t - 0.08) / 0.2).min(1.0) } else { 1.0 };
        hit(t) * a * s
    };
    let top = 4f32;
    let at = |t: f32, v: f32| pos2(rect.left() + t / 0.8 * rect.width(), rect.bottom() - (v / top).sqrt().min(1.0) * rect.height());
    let before: Vec<Pos2> = (0..=160).map(|i| { let t = i as f32 * 0.005; at(t, hit(t)) }).collect();
    let after: Vec<Pos2> = (0..=160).map(|i| { let t = i as f32 * 0.005; at(t, shaped(t)) }).collect();
    g.add(Shape::line(before, Stroke::new(1.2, lk.dim.gamma_multiply(0.7))));
    filled_curve(g, &after, rect.bottom(), accent);
    small_text(g, pos2(rect.left() + 2.0, rect.top()), Align2::LEFT_TOP, &format!("ATTACK {:+.0} %   SUSTAIN {:+.0} %", p[0], p[1]), lk.text);
}

fn echoes_view(g: &egui::Painter, lk: &Look, rect: Rect, p: &Params, accent: Color32) {
    let time = ovrtone_engine::drift_time_ms(*p, lk.bpm);
    let span = (time * 8.0).clamp(400.0, 4000.0);
    let x_of = |ms: f32| rect.left() + ms / span * rect.width();
    for k in 1..8 {
        let ms = span * k as f32 / 8.0;
        g.line_segment([pos2(x_of(ms), rect.top()), pos2(x_of(ms), rect.bottom())], Stroke::new(1.0, lk.grid));
    }
    let mid = rect.center().y;
    g.line_segment([pos2(rect.left(), mid), pos2(rect.right(), mid)], Stroke::new(1.0, lk.grid));
    let h = rect.height() * 0.46;
    // The dry sound, then each repeat; ping-pong alternates sides, the offset moves the right.
    g.line_segment([pos2(x_of(0.0) + 2.0, mid - h), pos2(x_of(0.0) + 2.0, mid + h)], Stroke::new(3.0, lk.dim));
    let pong = p[3];
    let mut level = p[9].max(0.05);
    for n in 1..40 {
        let t = time * n as f32;
        if t > span {
            break;
        }
        let odd = n % 2 == 1;
        let left = level * if odd { 1.0 } else { 1.0 - pong };
        let right = level * if odd { 1.0 - pong } else { 1.0 };
        let xl = x_of(t);
        let xr = x_of(t + p[2]);
        g.line_segment([pos2(xl, mid), pos2(xl, mid - h * left.min(1.0))], Stroke::new(3.0, accent));
        g.line_segment([pos2(xr, mid), pos2(xr, mid + h * right.min(1.0))], Stroke::new(3.0, accent.gamma_multiply(0.75)));
        level *= p[1];
        if level < 0.01 {
            break;
        }
    }
    small_text(g, pos2(rect.left() + 2.0, rect.top()), Align2::LEFT_TOP, "L", lk.dim);
    small_text(g, pos2(rect.left() + 2.0, rect.bottom()), Align2::LEFT_BOTTOM, "R", lk.dim);
    small_text(g, pos2(rect.right(), rect.top()), Align2::RIGHT_TOP, &format!("{:.0} ms   feedback {:.0} %", time, p[1] * 100.0), lk.text);
}

fn decay_view(g: &egui::Painter, lk: &Look, rect: Rect, p: &Params, accent: Color32) {
    let span = (p[1] * 1.3 + p[2] * 0.001).clamp(0.5, 32.0);
    let x_of = |s: f32| rect.left() + s / span * rect.width();
    let y_of = |db: f32| rect.top() + (-db / 60.0).clamp(0.0, 1.0) * rect.height();
    for db in [-20.0f32, -40.0] {
        g.line_segment([pos2(rect.left(), y_of(db)), pos2(rect.right(), y_of(db))], Stroke::new(1.0, lk.grid));
    }
    let pre = p[2] * 0.001;
    // Early reflections as ticks, then the tail falling 60 dB over the decay time.
    for (k, ms) in [7.0f32, 11.3, 17.9, 23.1, 31.7, 41.3, 52.9, 67.1].iter().enumerate() {
        let t = pre + ms * 0.001 * (0.35 + p[0] * 1.65);
        let top = y_of(-6.0 - k as f32 * 3.0 - (1.0 - p[8]) * 30.0);
        g.line_segment([pos2(x_of(t), rect.bottom()), pos2(x_of(t), top)], Stroke::new(1.5, lk.text.gamma_multiply(0.6)));
    }
    let pts: Vec<Pos2> = (0..=120).map(|i| {
        let t = pre + (span - pre) * i as f32 / 120.0;
        pos2(x_of(t), y_of(-60.0 * (t - pre) / p[1].max(0.1)))
    }).collect();
    filled_curve(g, &pts, rect.bottom(), accent);
    g.extend(Shape::dashed_line(&[pos2(x_of(pre), rect.top()), pos2(x_of(pre), rect.bottom())], Stroke::new(1.0, lk.dim), 3.0, 3.0));
    small_text(g, pos2(rect.right(), rect.top()), Align2::RIGHT_TOP, &format!("DECAY {:.2} s   PRE {:.0} ms", p[1], p[2]), lk.text);
}

fn sweep_view(g: &egui::Painter, lk: &Look, rect: Rect, p: &Params, accent: Color32) {
    let mid = rect.center().y;
    g.line_segment([pos2(rect.left(), mid), pos2(rect.right(), mid)], Stroke::new(1.0, lk.grid));
    let tri = |x: f32| {
        let f = x.rem_euclid(1.0);
        if f < 0.5 { 4.0 * f - 1.0 } else { 3.0 - 4.0 * f }
    };
    let seconds = 4.0;
    let mode = p[0].round() as usize;
    let rate = p[1] / 0.5;
    let wave = |t: f32, side: f32| {
        let a = tri(t * 0.5 * rate) * side;
        let b = tri(t * 0.83 * rate) * side * 1.3;
        match mode {
            0 => a,
            1 => b,
            2 => (a + b * 0.6) * 0.6,
            _ => (a + tri(t * 0.5 * rate + 0.25)) * 0.6,
        }
    };
    let h = rect.height() * 0.42 * p[2].max(0.05);
    for (side, col) in [(1.0f32, accent), (-1.0, accent.gamma_multiply(0.55))] {
        let pts: Vec<Pos2> = (0..=240).map(|i| {
            let t = seconds * i as f32 / 240.0;
            pos2(rect.left() + rect.width() * i as f32 / 240.0, mid - wave(t, side) * h)
        }).collect();
        g.add(Shape::line(pts, Stroke::new(2.0, col)));
    }
    small_text(g, pos2(rect.left() + 2.0, rect.top()), Align2::LEFT_TOP, &format!("MODE {}   {:.2} Hz", ovrtone_engine::FxKind::Swirl.choices(0).and_then(|c| c.get(mode)).copied().unwrap_or(""), p[1]), lk.text);
}

#[allow(clippy::too_many_arguments)]
fn focus_view(
    g: &egui::Painter,
    lk: &Look,
    rect: Rect,
    p: &Params,
    accent: Color32,
    resp: &egui::Response,
    edit: &mut impl FnMut(Edit),
) {
    freq_grid(g, lk, rect);
    let (lo, hi) = (freq_x(rect, p[4].min(p[5])), freq_x(rect, p[4].max(p[5])));
    let band = Rect::from_min_max(pos2(lo, rect.top()), pos2(hi, rect.bottom()));
    g.rect_filled(band, 0.0, accent.gamma_multiply(0.10));
    for x in [lo, hi] {
        g.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(2.0, accent));
        g.circle_filled(pos2(x, rect.center().y), 5.0, accent);
    }
    // What Depth and Selectivity do to a peak: the dim line is the peak, the bright one what is
    // left of it.
    let depth = p[0];
    let thresh = 2.0 + p[2] * 12.0;
    let y_of = |db: f32| rect.bottom() - 12.0 - db / 30.0 * (rect.height() - 24.0);
    let floor = 4.0;
    let peaks = [(0.25f32, 18.0f32), (0.45, 12.0), (0.62, 22.0), (0.8, 9.0)];
    let level = |x: f32| {
        let mut v = floor;
        for (at, h) in peaks {
            let w = 0.012 + (1.0 - p[1]) * 0.03;
            v += h * (-((x - at) / w).powi(2)).exp();
        }
        v
    };
    let mut before = Vec::new();
    let mut after = Vec::new();
    for i in 0..=240 {
        let f = i as f32 / 240.0;
        let x = rect.left() + f * rect.width();
        let v = level(f);
        let inside = x >= lo && x <= hi;
        let cut = if inside { ((v - floor - thresh).max(0.0) * depth * 1.5).min(30.0) } else { 0.0 };
        before.push(pos2(x, y_of(v)));
        after.push(pos2(x, y_of(v - cut)));
    }
    g.add(Shape::line(before, Stroke::new(1.2, lk.dim.gamma_multiply(0.7))));
    filled_curve(g, &after, rect.bottom(), accent);
    let mode = if p[6] >= 0.5 { "DE-ESS" } else { "RESONANCE" };
    small_text(g, pos2(rect.left() + 2.0, rect.top()), Align2::LEFT_TOP, &format!("{mode}   {:.0} Hz to {:.0} Hz", p[4].min(p[5]), p[4].max(p[5])), lk.text);
    // Drag either edge of the band.
    if resp.dragged() {
        if let Some(at) = resp.interact_pointer_pos() {
            let low_edge = (at.x - freq_x(rect, p[4])).abs() <= (at.x - freq_x(rect, p[5])).abs();
            let hz = x_freq(rect, at.x).clamp(20.0, 20_000.0);
            edit(Edit { param: if low_edge { 4 } else { 5 }, value: hz, ended: false });
        }
    }
    if resp.drag_stopped() {
        edit(Edit { param: 4, value: p[4], ended: true });
        edit(Edit { param: 5, value: p[5], ended: true });
    }
}

/// Prism's band buttons, in the bands' colours.
fn band_buttons(ui: &mut Ui, lk: &Look, p: &Params) {
    let band = chosen_band(ui);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for k in 0..6 {
            let (r, resp) = ui.allocate_exact_size(vec2(72.0, 24.0), Sense::click());
            let g = ui.painter();
            let sel = k == band;
            let col = band_color(k);
            g.rect_filled(r, 12.0, if sel { col.gamma_multiply(0.25) } else { lk.display });
            g.rect_stroke(r, 12.0, Stroke::new(1.0, if sel { col } else { lk.edge }), StrokeKind::Inside);
            g.circle_filled(pos2(r.left() + 12.0, r.center().y), 4.0, col);
            let what = match k {
                0 => "Low",
                5 => "High",
                _ => "Bell",
            };
            let gain = p[k * 5 + 1];
            let text = if p[k * 5 + 3].abs() >= 0.05 {
                format!("{} {}*", k + 1, what)
            } else if gain.abs() >= 0.05 {
                format!("{} {:+.0}", k + 1, gain)
            } else {
                format!("{} {}", k + 1, what)
            };
            g.text(pos2(r.left() + 22.0, r.center().y), Align2::LEFT_CENTER, text, FontId::proportional(11.0), if sel { lk.text } else { lk.dim });
            if resp.clicked() {
                choose_band(ui, k);
            }
        }
    });
}

/// Which track keys the effect: its own sound, or another track's (a side chain).
fn side_chain_row(ui: &mut Ui, lk: &Look, side: &mut Option<usize>, keys: &[(usize, String)], ti: usize) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("SIDE CHAIN").size(10.5).color(lk.dim));
        let now = side
            .and_then(|s| keys.iter().find(|(k, _)| *k == s))
            .map_or_else(|| "Off (its own sound)".to_string(), |(_, n)| n.clone());
        egui::ComboBox::from_id_salt(("fx_side", ti))
            .selected_text(now)
            .width(200.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(side, None, "Off (its own sound)");
                for (k, name) in keys {
                    ui.selectable_value(side, Some(*k), name.as_str());
                }
            });
    });
    ui.add_space(8.0);
}

/// The choices (segmented buttons) and switches.
fn choice_row(ui: &mut Ui, lk: &Look, kind: FxKind, shown: &Params, track: &Track, slot: usize, edits: &mut Vec<Edit>, wrap_at: f32) {
    let list: Vec<usize> = (0..kind.params().len()).filter(|k| kind.choices(*k).is_some()).collect();
    if list.is_empty() {
        return;
    }
    // Wrapped at the window's width, so a long row of choices goes onto a second line
    // instead of making the window wider.
    let group_w = |k: usize| {
        kind.choices(k).map_or(0.0, |names| {
            names.iter().map(|n| 16.0 + n.len() as f32 * 7.0).sum::<f32>()
        }) + 14.0
    };
    let mut lines: Vec<Vec<usize>> = vec![Vec::new()];
    let mut used = 0.0;
    for k in list {
        let w = group_w(k);
        if used + w > wrap_at && lines.last().is_some_and(|l| !l.is_empty()) {
            lines.push(Vec::new());
            used = 0.0;
        }
        used += w;
        if let Some(line) = lines.last_mut() {
            line.push(k);
        }
    }
    for line in lines {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(14.0, 8.0);
        for k in line {
            let Some(names) = kind.choices(k) else {
                continue;
            };
            let info = &kind.params()[k];
            let target = AutoTarget::Fx { slot: slot as u8, param: k as u8 };
            let enabled = automation::editable(track, target);
            let now = (shown[k].round().max(0.0) as usize).min(names.len() - 1);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.label(egui::RichText::new(info.name.to_uppercase()).size(10.5).color(lk.dim));
                let w: f32 = names.iter().map(|n| 16.0 + n.len() as f32 * 7.0).sum();
                let (r, _) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
                let g = ui.painter();
                g.rect_filled(r, 12.0, lk.display);
                g.rect_stroke(r, 12.0, Stroke::new(1.0, lk.edge), StrokeKind::Inside);
                let mut x = r.left();
                for (i, n) in names.iter().enumerate() {
                    let cw = 16.0 + n.len() as f32 * 7.0;
                    let cell = Rect::from_min_size(pos2(x, r.top()), vec2(cw, r.height()));
                    x += cw;
                    let resp = ui.interact(cell, ui.scope_id().with(("fx_choice", k, i)), Sense::click());
                    let sel = i == now;
                    if sel {
                        g.rect_filled(cell.shrink(2.0), 10.0, lk.accent);
                    } else if resp.hovered() && enabled {
                        g.rect_filled(cell.shrink(2.0), 10.0, lk.knob);
                    }
                    let col = if sel { theme::on_accent() } else if enabled { lk.text } else { lk.dim };
                    g.text(cell.center(), Align2::CENTER_CENTER, *n, FontId::proportional(12.0), col);
                    if resp.clicked() && enabled && !sel {
                        edits.push(Edit { param: k, value: i as f32, ended: true });
                    }
                }
            });
        }
    });
    }
    ui.add_space(10.0);
}

/// The large knobs, then the small ones in rows.
#[allow(clippy::too_many_arguments)]
/// Returns how wide the rows came out.
fn knob_rows(ui: &mut Ui, lk: &Look, kind: FxKind, shown: &Params, track: &Track, slot: usize, band: usize, edits: &mut Vec<Edit>) -> f32 {
    let heroes = heroes(kind);
    let small = small_knobs(kind, band);
    let rows = ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0 * lk.scale;
        for &k in heroes {
            let sync = match kind {
                FxKind::Drift => Some(10),
                FxKind::Delay => Some(5),
                _ => None,
            };
            let note = sync.filter(|sp| k == 0 && shown[*sp].round() >= 1.0).map(|sp| {
                let names = kind.choices(sp).unwrap_or(&[]);
                let name = names.get(shown[sp].round() as usize).copied().unwrap_or("");
                format!("{name} · {:.0} ms", ovrtone_engine::synced_ms(shown[0], shown[sp], lk.bpm))
            });
            knob(ui, lk, kind, k, shown[k], HERO_R, track, slot, edits, note);
        }
        if !heroes.is_empty() {
            let (r, _) = ui.allocate_exact_size(vec2(1.0, (HERO_R * 2.0 + 50.0) * lk.scale), Sense::hover());
            ui.painter().line_segment([r.center_top(), r.center_bottom()], Stroke::new(1.0, lk.grid));
        }
        ui.vertical(|ui| {
            let per_row = if heroes.is_empty() { 7 } else if heroes.len() > 1 { 4 } else { 6 };
            for row in small.chunks(per_row) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0 * lk.scale;
                    for &k in row {
                        knob(ui, lk, kind, k, shown[k], SMALL_R, track, slot, edits, None);
                    }
                });
                ui.add_space(4.0);
            }
        });
    });
    rows.response.rect.width()
}

/// How wide the knob rows are at the normal size.
fn knobs_width(kind: FxKind, band: usize) -> f32 {
    let heroes = heroes(kind);
    let small = small_knobs(kind, band).len();
    let per_row = if heroes.is_empty() {
        7
    } else if heroes.len() > 1 {
        4
    } else {
        6
    };
    let cols = small.min(per_row) as f32;
    let hero_w = heroes.len() as f32 * (HERO_R * 2.0 + 48.0 + 10.0);
    let divider = if heroes.is_empty() { 0.0 } else { 11.0 };
    let small_w = cols * 80.0 + (cols - 1.0).max(0.0) * 6.0;
    hero_w + divider + small_w + 4.0
}

/// How wide the widest group of choices is (the row wraps between groups).
fn choices_width(kind: FxKind) -> f32 {
    (0..kind.params().len())
        .filter_map(|k| kind.choices(k))
        .map(|names| names.iter().map(|n| 16.0 + n.len() as f32 * 7.0).sum::<f32>() + 14.0)
        .fold(0.0, f32::max)
}

/// One knob: its name, the knob with a lit arc from its rest point to its value, and the value.
/// Drag up and down (Shift for fine steps); double-click for the default.
#[allow(clippy::too_many_arguments)]
fn knob(ui: &mut Ui, lk: &Look, kind: FxKind, k: usize, value: f32, radius: f32, track: &Track, slot: usize, edits: &mut Vec<Edit>, note: Option<String>) {
    let Some(info) = kind.params().get(k) else {
        return;
    };
    let target = AutoTarget::Fx { slot: slot as u8, param: k as u8 };
    // A note value in charge: the knob shows it and does not move.
    let enabled = automation::editable(track, target) && note.is_none();
    let big = radius >= HERO_R;
    let s = lk.scale;
    let radius = radius * s;
    let ring = if big { 7.0 } else { 4.5 } * s;
    let label_h = if big { 18.0 } else { 16.0 } * s;
    let w = if big { radius * 2.0 + 48.0 * s } else { 80.0 * s };
    let (r, _) = ui.allocate_exact_size(
        vec2(w, label_h + ring * 2.0 + radius * 2.0 + 24.0 * s),
        Sense::hover(),
    );
    // The name sits above the lit ring, never under it.
    let c = pos2(r.center().x, r.top() + label_h + ring + radius);
    let hit = Rect::from_center_size(c, vec2(radius * 2.0 + 8.0, radius * 2.0 + 8.0));
    let resp = ui.interact(hit, ui.scope_id().with(("fx_knob", k)), Sense::click_and_drag());
    let g = ui.painter().clone();
    let name_col = if enabled { lk.text.gamma_multiply(0.85) } else { lk.dim.gamma_multiply(0.6) };
    g.text(pos2(c.x, r.top()), Align2::CENTER_TOP, label(kind, info).to_uppercase(), FontId::proportional(if big { 12.0 } else { 10.5 } * s), name_col);
    let n = info.to_norm(value);
    // Lit from the middle for settings that go both ways, from the start for the rest.
    let from = if info.min < 0.0 && info.max > 0.0 { info.to_norm(0.0) } else { 0.0 };
    let arc_r = radius + ring;
    let width = if big { 4.0 } else { 2.6 } * s.sqrt();
    let arc = |a: f32, b: f32| -> Vec<Pos2> {
        let steps = (((b - a).abs() * 40.0).ceil() as usize).max(2);
        (0..=steps)
            .map(|i| {
                let v = a + (b - a) * i as f32 / steps as f32;
                let angle = (135.0 + v * 270.0f32).to_radians();
                c + vec2(angle.cos(), angle.sin()) * arc_r
            })
            .collect()
    };
    g.add(Shape::line(arc(0.0, 1.0), Stroke::new(width, lk.display)));
    if (n - from).abs() > 0.002 {
        let col = if enabled { lk.accent } else { lk.accent.gamma_multiply(0.5) };
        g.add(Shape::line(arc(from.min(n), from.max(n)), Stroke::new(width, col)));
    }
    // The body: a soft rim and a flat top.
    g.circle_filled(c + vec2(0.0, 1.5), radius, Color32::from_black_alpha(70));
    g.circle_filled(c, radius, lk.knob);
    g.circle_stroke(c, radius, Stroke::new(1.0, mix(lk.knob, lk.text, 0.12)));
    g.circle_filled(c, radius * 0.78, mix(lk.knob, lk.plate, 0.35));
    let angle = (135.0 + n * 270.0f32).to_radians();
    let d = vec2(angle.cos(), angle.sin());
    g.line_segment([c + d * (radius * 0.38), c + d * (radius * 0.86)], Stroke::new(if big { 2.6 } else { 1.8 }, lk.text));
    let text = note.unwrap_or_else(|| value_text(info, value));
    let val_col = if resp.dragged() || resp.hovered() { lk.accent } else { lk.text };
    let font = FontId::proportional(if big { 13.5 } else { 11.5 } * s);
    let at = pos2(c.x, c.y + radius + ring + 3.0 * s);
    let typing_id = ui.scope_id().with(("fx_typing", k));
    let typing: Option<String> = ui.ctx().data(|d| d.get_temp(typing_id));
    if let Some(mut typed) = typing {
        let field = Rect::from_center_size(at + vec2(0.0, 9.0 * s), vec2(w - 6.0, 20.0 * s));
        let edit = ui.put(
            field,
            egui::TextEdit::singleline(&mut typed)
                .font(font.clone())
                .align(egui::Align2::CENTER_CENTER),
        );
        if !edit.has_focus() && !edit.lost_focus() {
            edit.request_focus();
        }
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if edit.lost_focus() || escape {
            ui.ctx().data_mut(|d| d.remove::<String>(typing_id));
            if enter && !escape {
                if let Some(v) = typed_value(info, &typed) {
                    edits.push(Edit { param: k, value: v, ended: true });
                }
            }
        } else {
            ui.ctx().data_mut(|d| d.insert_temp(typing_id, typed));
        }
    } else {
        let shown = g.text(at, Align2::CENTER_TOP, text, font, val_col);
        if enabled {
            let hit = shown.expand2(vec2(6.0, 2.0));
            let resp = ui
                .interact(hit, ui.scope_id().with(("fx_value", k)), Sense::click())
                .on_hover_text("Click to type a value");
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
            }
            if resp.clicked() {
                let start = value_text(info, value)
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim_start_matches('+')
                    .to_string();
                ui.ctx().data_mut(|d| d.insert_temp(typing_id, start));
            }
        }
    }
    if !track.auto.lane(target).is_empty() {
        g.circle_filled(pos2(c.x + radius + 4.0, c.y - radius), 2.5, theme::SOLO);
    }
    if !enabled {
        return;
    }
    // The same gestures as every knob: Shift or Ctrl for fine, Alt-click or double-click to reset,
    // the wheel to nudge.
    let reset = info.to_norm(info.default);
    if let Some(to) = crate::widgets::value_gesture(ui, &resp, n, reset, 1.0 / 220.0, 0.01) {
        let whole = !resp.dragged();
        let v = if whole && to == reset {
            info.default
        } else {
            info.from_norm(to.clamp(0.0, 1.0))
        };
        edits.push(Edit { param: k, value: v, ended: whole });
        if whole {
            return;
        }
    }
    if resp.drag_stopped() {
        edits.push(Edit { param: k, value, ended: true });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Insert;

    #[test]
    fn the_window_edits_the_insert_and_closes_with_it() {
        let mut p = Project::empty();
        p.tracks[0].inserts = FxKind::ALL.iter().map(|k| Insert::new(*k)).collect();
        let mut s = UiState::new();
        let meters = Meters::default();
        let mut h = crate::ui_test::Harness::new(900.0, 800.0);
        for slot in 0..FxKind::ALL.len() {
            s.mixer.fx_window = Some((0, slot));
            for _ in 0..3 {
                h.frame(vec![], |ui| show(ui.ctx(), &mut p, &mut s, &meters));
            }
            assert_eq!(s.mixer.fx_window, Some((0, slot)));
        }
        // Nothing was changed by looking.
        for (k, kind) in FxKind::ALL.iter().enumerate() {
            assert_eq!(p.tracks[0].inserts[k], Insert::new(*kind));
        }
        // Every window stayed open.
        assert_eq!(s.mixer.fx_open.len(), FxKind::ALL.len());
        // The inserts go away: their windows close.
        p.tracks[0].inserts.clear();
        h.frame(vec![], |ui| show(ui.ctx(), &mut p, &mut s, &meters));
        assert_eq!(s.mixer.fx_window, None);
        assert!(s.mixer.fx_open.is_empty());
    }

    #[test]
    fn an_effect_window_keeps_its_size_until_the_user_drags_it() {
        for (slot, kind) in FxKind::ALL.iter().enumerate() {
            let mut p = Project::empty();
            p.tracks[0].inserts = FxKind::ALL.iter().map(|k| Insert::new(*k)).collect();
            let mut s = UiState::new();
            s.mixer.fx_window = Some((0, slot));
            let meters = Meters::default();
            let mut h = crate::ui_test::Harness::new(1600.0, 1200.0);
            let mut sizes = Vec::new();
            for _ in 0..40 {
                h.frame(vec![], |ui| {
                    show(ui.ctx(), &mut p, &mut s, &meters);
                    let id = egui::Id::unique(("fx_window", 0usize, slot));
                    if let Some(r) = ui.ctx().memory(|m| m.area_rect(id)) {
                        sizes.push(r.size());
                    }
                });
            }
            let settled = &sizes[sizes.len() - 20..];
            let first = settled[0];
            for size in settled {
                assert!(
                    (size.x - first.x).abs() < 0.5 && (size.y - first.y).abs() < 0.5,
                    "{kind:?} went from {first:?} to {size:?}"
                );
            }
            assert!(first.x < 900.0, "{kind:?} grew to {first:?}");
        }
    }

    #[test]
    fn a_played_lane_moves_the_knob_and_locks_it_in_read_mode() {
        use crate::model::AutoMode;
        let mut p = Project::empty();
        p.tracks[0].inserts = vec![Insert::new(FxKind::Compressor)];
        let thr = AutoTarget::Fx { slot: 0, param: 0 };
        p.tracks[0].auto.mode = AutoMode::Read;
        p.tracks[0].auto.set_point(thr, 0.0, 0.25);
        let mut s = UiState::new();
        s.mixer.fx_window = Some((0, 0));
        let meters = Meters::default();
        let mut h = crate::ui_test::Harness::new(800.0, 600.0);
        for _ in 0..4 {
            h.frame(vec![], |ui| show(ui.ctx(), &mut p, &mut s, &meters));
        }
        // Looking at a played lane changes neither the setting nor the lane.
        assert_eq!(p.tracks[0].inserts[0].params[0], -18.0);
        assert_eq!(p.tracks[0].auto.lane(thr).len(), 1);
        assert_eq!(p.tracks[0].auto.held, None);
        assert!(!automation::editable(&p.tracks[0], thr));
    }
}
