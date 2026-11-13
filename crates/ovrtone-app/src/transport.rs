//! Control bar (Logic Pro: "Control Bar"): view buttons, transport, LCD, count-in/metronome,
//! master volume, and the editor/browser buttons on the right.
//!
//! Every icon is drawn with vector shapes here (no bitmap or font glyphs), so it renders the
//! same on Windows, Linux and macOS.
use crate::file_browser::Tab;
use crate::i18n::{tr, trf};
use crate::{
    model::{Project, SigPart},
    Bottom, UiState,
};
use egui::epaint::text::{LayoutJob, TextFormat};
use egui::{
    pos2, vec2, Align2, Color32, CornerRadius, FontId, Painter, Pos2, Rect, Sense, Shape, Stroke,
    StrokeKind, Ui,
};

fn bar_bg_col() -> Color32 {
    crate::theme::grey_rgb(0x30, 0x30, 0x30)
}
fn seg_bg_col() -> Color32 {
    crate::theme::grey_rgb(0x3a, 0x3a, 0x3a)
}
fn seg_border_col() -> Color32 {
    crate::theme::panel_bg()
}
fn seg_on_col() -> Color32 {
    crate::theme::grey_rgb(0x1f, 0x1f, 0x1f)
}
fn icon_col() -> Color32 {
    crate::theme::grey_rgb(0xd2, 0xd2, 0xd2)
}
fn icon_on_col() -> Color32 {
    crate::theme::grey_rgb(0xec, 0xec, 0xec)
}
fn lcd_bg_col() -> Color32 {
    crate::theme::grey_rgb(0x0f, 0x0f, 0x11)
}
fn lcd_text_col() -> Color32 {
    crate::theme::grey_rgb(0xe8, 0xe8, 0xe8)
}
fn lcd_mid_col() -> Color32 {
    crate::theme::grey_rgb(0x88, 0x88, 0x88)
}
fn lcd_dim_col() -> Color32 {
    crate::theme::grey_rgb(0x2a, 0x2a, 0x2b)
}
/// The LCD while recording turns red, as in Logic.
const LCD_REC_BG: Color32 = Color32::from_rgb(19, 10, 10);
const LCD_REC_TEXT: Color32 = Color32::from_rgb(244, 98, 86);
const LCD_REC_MID: Color32 = Color32::from_rgb(170, 62, 54);
const LCD_REC_DIM: Color32 = Color32::from_rgb(84, 30, 28);
/// The record button's background while recording.
const REC_BUTTON_ON: Color32 = Color32::from_rgb(165, 26, 22);

/// The LCD's colours: normal or recording.
struct LcdColors {
    bg: Color32,
    text: Color32,
    mid: Color32,
    dim: Color32,
}

impl LcdColors {
    fn new(recording: bool) -> Self {
        if recording {
            Self {
                bg: LCD_REC_BG,
                text: LCD_REC_TEXT,
                mid: LCD_REC_MID,
                dim: LCD_REC_DIM,
            }
        } else {
            Self {
                bg: lcd_bg_col(),
                text: lcd_text_col(),
                mid: lcd_mid_col(),
                dim: lcd_dim_col(),
            }
        }
    }
}

const REC_RED: Color32 = Color32::from_rgb(0xe6, 0x34, 0x2c);
const PURPLE: Color32 = Color32::from_rgb(0x9b, 0x3f, 0xd6);


/// What a button of the control bar is, for when the pointer is on it.
fn button_name(key: &str) -> Option<&'static str> {
    Some(match key {
        "rew" => "Rewind",
        "ffw" => "Fast Forward",
        "start" => "Go to Beginning (Enter)",
        "play" => "Play or Stop (Space)",
        "rec" => "Record (R)",
        "capture" => "Capture Recording (Shift+R)",
        "cycle" => "Cycle (C)",
        "lowlat" => "Low Latency Mode: plugins with a long delay step aside",
        "typing" => "Typing Keyboard: play notes on the computer keyboard (Ctrl+K). Numpad + and - change the octave",
        "countin" => "Count-in",
        "metronome" => "Metronome (K)",
        "view0" => "Library (Y)",
        "view1" => "Inspector (I)",
        "view2" => "Quick Help",
        "view3" => "Toolbar",
        "panes0" => "Light or dark menus",
        "panes1" => "Mixer (X)",
        "panes2" => "Editors (P)",
        "right0" => "List Editors (D)",
        "right1" => "Project Notes",
        "right2" => "Loop Browser",
        "right3" => "Media Browser (F)",
        _ => return None,
    })
}

fn clicked(ui: &Ui, rect: Rect, key: &str) -> bool {
    let resp = ui.interact(rect, egui::Id::unique(("control_bar", key)), Sense::click());
    match button_name(key) {
        Some(name) => resp.on_hover_text(name).clicked(),
        None => resp.clicked(),
    }
}

// ---------------------------------------------------------------- icons

type IconFn = fn(&Painter, Rect, Color32);

/// The menu of the mode buttons: which are shown. The one ticked or unticked, if any.
fn modes_menu(ui: &mut egui::Ui, modes_now: &[HeaderMode]) -> Option<(HeaderMode, bool)> {
    let mut chosen = None;
    for (mode, label) in [
        (HeaderMode::Typing, "Typing Keyboard (Ctrl+K)"),
        (HeaderMode::LowLatency, "Low Latency Mode"),
    ] {
        let mut shown = modes_now.contains(&mode);
        if ui.checkbox(&mut shown, label).changed() {
            chosen = Some((mode, shown));
        }
    }
    chosen
}

/// How far apart the mode buttons after the display are.
const MODE_STEP: f32 = 32.0;

/// A mode button the control bar can show after the display (right-click there to choose).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderMode {
    /// Play notes on the computer keyboard (Ctrl+K).
    Typing,
    LowLatency,
}

fn icon_typing(g: &Painter, r: Rect, c: Color32) {
    crate::icons::paint(g, r.center(), 16.0, crate::icons::Icon::Keyboard, c);
}

fn icon_library(g: &Painter, r: Rect, c: Color32) {
    crate::icons::paint(g, r.center(), 17.0, crate::icons::Icon::Library, c);
}

fn icon_inspector(g: &Painter, r: Rect, c: Color32) {
    crate::icons::paint(g, r.center(), 17.0, crate::icons::Icon::Inspector, c);
}

fn icon_help(g: &Painter, r: Rect, c: Color32) {
    crate::icons::paint(g, r.center(), 17.0, crate::icons::Icon::Help, c);
}

fn icon_toolbar(g: &Painter, r: Rect, c: Color32) {
    crate::icons::paint(g, r.center(), 17.0, crate::icons::Icon::Toolbar, c);
}

fn icon_smart_controls(g: &Painter, r: Rect, c: Color32) {
    crate::icons::paint(g, r.center(), 17.0, crate::icons::Icon::SmartControls, c);
}

fn icon_mixer(g: &Painter, r: Rect, c: Color32) {
    crate::icons::paint(g, r.center(), 17.0, crate::icons::Icon::Mixer, c);
}

fn icon_editors(g: &Painter, r: Rect, c: Color32) {
    crate::icons::paint(g, r.center(), 17.0, crate::icons::Icon::Editors, c);
}


fn icon_rewind(g: &Painter, r: Rect, c: Color32) {
    crate::icons::paint(g, r.center(), 17.0, crate::icons::Icon::Rewind, c);
}

fn icon_forward(g: &Painter, r: Rect, c: Color32) {
    crate::icons::paint(g, r.center(), 17.0, crate::icons::Icon::Forward, c);
}

fn icon_to_start(g: &Painter, r: Rect, c: Color32) {
    crate::icons::paint(g, r.center(), 17.0, crate::icons::Icon::ToStart, c);
}

fn icon_play(g: &Painter, r: Rect, c: Color32) {
    crate::icons::paint(g, r.center(), 17.0, crate::icons::Icon::Play, c);
}

fn icon_cycle(g: &Painter, r: Rect, c: Color32) {
    crate::icons::paint(g, r.center(), 17.0, crate::icons::Icon::Cycle, c);
}

fn icon_low_latency(g: &Painter, r: Rect, c: Color32) {
    crate::icons::paint(g, r.center(), 17.0, crate::icons::Icon::Bolt, c);
}

fn icon_metronome(g: &Painter, r: Rect, c: Color32) {
    crate::icons::paint(g, r.center(), 17.0, crate::icons::Icon::Metronome, c);
}

fn icon_list_editors(g: &Painter, r: Rect, c: Color32) {
    crate::icons::paint(g, r.center(), 17.0, crate::icons::Icon::ListEditors, c);
}

fn icon_note_pad(g: &Painter, r: Rect, c: Color32) {
    crate::icons::paint(g, r.center(), 17.0, crate::icons::Icon::NotePad, c);
}

fn icon_loops(g: &Painter, r: Rect, c: Color32) {
    crate::icons::paint(g, r.center(), 17.0, crate::icons::Icon::Loops, c);
}

fn icon_browsers(g: &Painter, r: Rect, c: Color32) {
    crate::icons::paint(g, r.center(), 17.0, crate::icons::Icon::Browsers, c);
}

// ---------------------------------------------------------------- widgets

/// A joined segmented control (Logic's grouped toolbar buttons). Returns the clicked index.
fn segmented(
    ui: &Ui,
    g: &Painter,
    left: f32,
    cy: f32,
    key: &str,
    items: &[(IconFn, bool)],
) -> Option<usize> {
    let seg = vec2(30.0, 24.0);
    let full = Rect::from_min_size(
        pos2(left, cy - seg.y / 2.0),
        vec2(seg.x * items.len() as f32, seg.y),
    );
    crate::theme::glass_plate(g, full, 6.0);
    let mut hit = None;
    for (i, (draw, on)) in items.iter().enumerate() {
        let r = Rect::from_min_size(pos2(left + i as f32 * seg.x, full.top()), seg);
        if *on {
            let cr = CornerRadius {
                nw: if i == 0 { 5 } else { 0 },
                sw: if i == 0 { 5 } else { 0 },
                ne: if i + 1 == items.len() { 5 } else { 0 },
                se: if i + 1 == items.len() { 5 } else { 0 },
            };
            g.rect_filled(r, cr, seg_on_col());
        }
        if i > 0 {
            g.line_segment(
                [
                    pos2(r.left(), r.top() + 4.0),
                    pos2(r.left(), r.bottom() - 4.0),
                ],
                Stroke::new(1.0, seg_border_col()),
            );
        }
        draw(g, r, if *on { icon_on_col() } else { icon_col() });
        if clicked(ui, r, &format!("{key}{i}")) {
            hit = Some(i);
        }
    }
    hit
}

/// Transport-style button: icon only, or icon on a coloured rounded square when active.
fn tbutton(
    ui: &Ui,
    g: &Painter,
    center: Pos2,
    key: &str,
    draw: IconFn,
    active_bg: Option<Color32>,
    icon: Color32,
) -> bool {
    let r = Rect::from_center_size(center, vec2(30.0, 26.0));
    let mut resp = ui.interact(r, egui::Id::unique(("control_bar", key)), Sense::click());
    if let Some(name) = button_name(key) {
        resp = resp.on_hover_text(name);
    }
    if let Some(bg) = active_bg {
        g.rect_filled(r, 6.0, bg);
    } else if resp.hovered() {
        g.rect_filled(r, 6.0, Color32::from_white_alpha(14));
    }
    draw(g, r, icon);
    resp.clicked()
}

/// Appends `text`, drawing the first `dim` characters in the dim LCD colour.
fn lcd_seg(
    job: &mut LayoutJob,
    text: &str,
    dim: usize,
    size: f32,
    color: Color32,
    dim_color: Color32,
) {
    let split = text.char_indices().nth(dim).map_or(text.len(), |(i, _)| i);
    let font = crate::theme::digit_font(size);
    if split > 0 {
        job.append(
            &text[..split],
            0.0,
            TextFormat {
                font_id: font.clone(),
                color: dim_color,
                ..Default::default()
            },
        );
    }
    job.append(
        &text[split..],
        0.0,
        TextFormat {
            font_id: font,
            color,
            ..Default::default()
        },
    );
}

/// "009 1 1 001"-style position with dim leading zeros.
fn position_job(
    bbdt: (u32, u32, u32, u32),
    size: f32,
    color: Color32,
    dim_color: Color32,
) -> LayoutJob {
    let (bar, beat, div, tick) = bbdt;
    let mut job = LayoutJob::default();
    let bar_s = format!("{bar:03}");
    let bar_dim = bar_s.len() - bar.to_string().len();
    lcd_seg(&mut job, &bar_s, bar_dim, size, color, dim_color);
    lcd_seg(
        &mut job,
        &format!("  {beat}  {div}  "),
        0,
        size,
        color,
        dim_color,
    );
    let tick_s = format!("{tick:03}");
    let tick_dim = tick_s.len() - tick.to_string().len();
    lcd_seg(&mut job, &tick_s, tick_dim, size, color, dim_color);
    job
}

/// Pixels of vertical drag per step when editing the time signature.
const SIG_PX_PER_STEP: f32 = 10.0;

/// Adds `delta_px` to the drag in progress and takes out the whole steps it now holds, keeping
/// the remainder for the next frame so slow drags still add up.
fn take_steps(leftover: &mut f32, delta_px: f32, px_per_step: f32) -> i32 {
    *leftover += delta_px;
    let steps = (*leftover / px_per_step).trunc();
    *leftover -= steps * px_per_step;
    steps as i32
}

/// The box the tempo is typed in, while it is open: Enter or a click elsewhere sets it, Esc leaves
/// it as it was. True while it is showing.
fn tempo_field(ui: &mut Ui, p: &mut Project, open: bool, field: Rect) -> bool {
    let key = egui::Id::unique(("control_bar", "tempo_edit"));
    let field_id = key.with("field");
    let beat = p.position_beats;
    let held: Option<String> = ui.data(|d| d.get_temp(key));
    let opening = open && held.is_none();
    let Some(mut text) = held.or_else(|| opening.then(|| format!("{:.2}", p.tempo_at(beat))))
    else {
        return false;
    };
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(field));
    let edit = child.add_sized(
        field.size(),
        egui::TextEdit::singleline(&mut text)
            .id(field_id)
            .font(FontId::proportional(16.0))
            .align(Align2::CENTER_CENTER),
    );
    if opening {
        edit.request_focus();
        let ctx = ui.ctx();
        if let Some(mut state) = egui::TextEdit::load_state(ctx, field_id) {
            let all = egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(text.chars().count()),
            );
            state.cursor.set_char_range(Some(all));
            state.store(ctx, field_id);
        }
    }
    if edit.lost_focus() || (!opening && !edit.has_focus()) {
        let cancelled = ui.input(|i| i.key_pressed(egui::Key::Escape));
        let typed = text.trim().parse::<f32>().ok().filter(|b| b.is_finite());
        if let (false, Some(bpm)) = (cancelled, typed) {
            p.set_tempo_at(beat, bpm);
        }
        ui.data_mut(|d| d.remove::<String>(key));
    } else {
        ui.data_mut(|d| d.insert_temp(key, text));
    }
    true
}

/// The display in its everyday form: bar, beat, tempo, signature and key. The arrow at its right
/// end changes to the full display and back.
/// The saved default that means the Custom form.
pub(crate) const CUSTOM_LCD: u8 = 9;

fn lcd_compact(ui: &mut Ui, g: &Painter, p: &mut Project, rect: Rect) {
    let pal = LcdColors::new(p.recording);
    g.rect_filled(rect, 6.0, pal.bg);
    g.rect_stroke(
        rect,
        6.0,
        Stroke::new(1.0, crate::theme::gray(0x09)),
        StrokeKind::Inside,
    );
    let x0 = rect.left();
    let (top, cy) = (rect.top(), rect.center().y);
    let small = FontId::proportional(7.5);
    let smpte = ui
        .interact(
            Rect::from_min_size(rect.min, vec2(92.0, rect.height())),
            egui::Id::unique(("control_bar", "smpte")),
            Sense::click(),
        )
        .on_hover_text(tr("Right-click for the SMPTE and synchronization settings"));
    crate::widgets::context_menu(&smpte, |ui| crate::sync_menu::items(ui, p));
    let (bar, beat, _, _) = p.bbdt(p.position_beats);
    let bar_s = format!("{bar:03}");
    let dim = bar_s.len() - bar.to_string().len();
    let mut job = LayoutJob::default();
    lcd_seg(&mut job, &bar_s, dim, 25.0, pal.text, pal.dim);
    g.galley(pos2(x0 + 7.0, top + 3.0), g.layout_job(job), pal.text);
    g.text(
        pos2(x0 + 26.0, top + 33.0),
        Align2::CENTER_TOP,
        "BAR",
        small.clone(),
        pal.mid,
    );
    g.text(
        pos2(x0 + 72.0, top + 3.0),
        Align2::CENTER_TOP,
        beat.to_string(),
        crate::theme::digit_font(25.0),
        pal.text,
    );
    g.text(
        pos2(x0 + 72.0, top + 33.0),
        Align2::CENTER_TOP,
        "BEAT",
        small.clone(),
        pal.mid,
    );
    // Tempo: drag it up and down, or click to type.
    let tempo_rect = Rect::from_min_size(pos2(x0 + 92.0, top), vec2(54.0, rect.height()));
    let resp = ui
        .interact(
            tempo_rect,
            egui::Id::unique(("control_bar", "tempo")),
            Sense::click_and_drag(),
        )
        .on_hover_text("Click to type the tempo, or drag up and down");
    let tempo_now = p.tempo_at(p.position_beats);
    if resp.dragged() {
        p.set_tempo_at(p.position_beats, tempo_now - resp.drag_delta().y * 0.5);
    }
    let field = Rect::from_min_size(pos2(x0 + 94.0, top + 4.0), vec2(50.0, 18.0));
    if !tempo_field(ui, p, resp.clicked(), field) {
        let shown = p.tempo_at(p.position_beats);
        let text = if shown.fract().abs() < 0.005 {
            format!("{shown:.0}")
        } else {
            format!("{shown:.2}")
        };
        g.text(
            pos2(tempo_rect.center().x, top + 6.0),
            Align2::CENTER_TOP,
            text,
            FontId::proportional(14.0),
            pal.text,
        );
    }
    g.text(
        pos2(tempo_rect.center().x, cy + 3.0),
        Align2::CENTER_TOP,
        "KEEP",
        small.clone(),
        pal.mid,
    );
    g.text(
        pos2(tempo_rect.center().x, cy + 12.0),
        Align2::CENTER_TOP,
        "TEMPO",
        small,
        pal.mid,
    );
    // Signature (drag each number) over the key.
    let sig_x = x0 + 172.0;
    signature_drag(
        ui,
        p,
        [
            Rect::from_min_size(pos2(sig_x - 22.0, top), vec2(22.0, cy - top)),
            Rect::from_min_size(pos2(sig_x, top), vec2(22.0, cy - top)),
        ],
    );
    let (num, den) = p.signature_of_bar(p.beats_to_bar(p.position_beats) as u32);
    g.text(
        pos2(sig_x, top + 6.0),
        Align2::CENTER_TOP,
        format!("{num}/{den}"),
        FontId::proportional(14.0),
        pal.text,
    );
    g.text(
        pos2(sig_x, cy + 4.0),
        Align2::CENTER_TOP,
        p.key,
        fitting_font(g, p.key, 12.0, 46.0),
        pal.text,
    );
    // Click the key to set the project's key, so it is always to hand.
    let key_r = Rect::from_center_size(pos2(sig_x, cy + 12.0), vec2(48.0, 18.0));
    let key_resp = ui
        .interact(key_r, egui::Id::unique(("control_bar", "key")), Sense::click())
        .on_hover_text("The project's key: click to set it");
    if key_resp.hovered() {
        g.rect_stroke(key_r, 3.0, Stroke::new(1.0, pal.text.gamma_multiply(0.4)), egui::StrokeKind::Inside);
    }
    egui::Popup::menu(&key_resp).show(|ui| {
        ui.set_min_width(180.0);
        ui.label(egui::RichText::new("Project key").strong());
        egui::Grid::new(egui::Id::unique("key_grid")).num_columns(2).show(ui, |ui| {
            let majors: Vec<_> = crate::score::KEYS.iter().filter(|k| k.0.ends_with("maj")).collect();
            let minors: Vec<_> = crate::score::KEYS.iter().filter(|k| k.0.ends_with("min")).collect();
            for row in 0..majors.len().max(minors.len()) {
                for list in [&majors, &minors] {
                    match list.get(row) {
                        Some(k) => {
                            if ui.selectable_label(p.key == k.0, crate::score::key_label(k.0)).clicked() {
                                p.key = k.0;
                                p.song_rev += 1;
                                ui.close();
                            }
                        }
                        None => {
                            ui.label("");
                        }
                    }
                }
                ui.end_row();
            }
        });
    });
}

/// Dragging the numerator or denominator of the signature, in `areas`, up or down.
fn signature_drag(ui: &Ui, p: &mut Project, areas: [Rect; 2]) {
    for (part, area) in [SigPart::Numerator, SigPart::Denominator]
        .into_iter()
        .zip(areas)
    {
        let id = egui::Id::unique(("control_bar", "signature", part as u8));
        let resp = ui
            .interact(area, id, Sense::drag())
            .on_hover_cursor(egui::CursorIcon::ResizeVertical)
            .on_hover_text(tr("Drag to change the time signature"));
        let mut leftover = ui.data(|d| d.get_temp::<f32>(id)).unwrap_or(0.0);
        if resp.drag_started() {
            leftover = 0.0;
        }
        let steps = if resp.dragged() {
            take_steps(&mut leftover, -resp.drag_delta().y, SIG_PX_PER_STEP)
        } else {
            0
        };
        ui.data_mut(|d| d.insert_temp(id, leftover));
        if steps != 0 {
            let bar0 = p.beats_to_bar(p.position_beats) as u32;
            p.step_signature_at(bar0, part, steps);
        }
    }
}

/// The arrow at the right end of the display, which changes its form. True when clicked.
fn lcd_arrow(ui: &Ui, g: &Painter, rect: Rect, recording: bool) -> egui::Response {
    let pal = LcdColors::new(recording);
    let area = Rect::from_min_max(pos2(rect.right() - 22.0, rect.top()), rect.max);
    let resp = ui
        .interact(
            area,
            egui::Id::unique(("control_bar", "lcd_form")),
            Sense::click(),
        )
        .on_hover_text("What the display shows");
    let c = area.center();
    g.add(Shape::line(
        vec![
            c + vec2(-4.0, -2.0),
            c + vec2(0.0, 2.0),
            c + vec2(4.0, -2.0),
        ],
        Stroke::new(1.6, pal.mid),
    ));
    resp
}

/// Where the song is, as bar, beat, sixteenth and tick.
pub(crate) fn beats_text(p: &Project) -> String {
    let at = p.position_beats.max(0.0);
    let bar = p.beats_to_bar(at + 1e-9).floor().max(0.0) as u32;
    let (_, unit) = p.signature_of_bar(bar);
    let unit = 4.0 / f64::from(unit.max(1));
    let into = (at - p.bar_to_beats(f64::from(bar))).max(0.0);
    let beat = (into / unit).floor();
    let rest = (into - beat * unit).max(0.0);
    let sixteenth = (rest / 0.25).floor();
    let tick = ((rest - sixteenth * 0.25) * 960.0).floor() as u32;
    format!(
        "{} {} {} {:03}",
        bar + 1,
        beat as u32 + 1,
        sixteenth as u32 + 1,
        tick + 1
    )
}

/// Where the song is in time, from its start.
pub(crate) fn time_text(p: &Project) -> String {
    let seconds = p.tempo_map().seconds_at(p.position_beats.max(0.0)).max(0.0);
    let whole = seconds as u64;
    format!(
        "{:02}:{:02}:{:02}.{:02}",
        whole / 3600,
        whole / 60 % 60,
        whole % 60,
        ((seconds - whole as f64) * 100.0) as u32
    )
}

/// The display's simpler forms: beats, time, or both.
fn lcd_text(g: &Painter, p: &Project, rect: Rect, mode: u8) {
    g.rect_filled(rect, 5.0, crate::theme::grey_rgb(0x0a, 0x0a, 0x0b));
    let ink = if p.recording {
        Color32::from_rgb(0xff, 0x70, 0x60)
    } else {
        crate::theme::gray(0xe8)
    };
    let text = |at: egui::Pos2, what: String, size: f32| {
        g.text(
            at,
            Align2::CENTER_CENTER,
            what,
            FontId::monospace(size),
            ink,
        );
    };
    let c = rect.center();
    let room = vec2(-11.0, 0.0);
    match mode {
        1 => {
            text(c + room + vec2(0.0, -8.0), beats_text(p), 13.0);
            text(c + room + vec2(0.0, 8.0), time_text(p), 13.0);
        }
        2 => text(c + room, beats_text(p), 18.0),
        _ => text(c + room, time_text(p), 18.0),
    }
}

/// The display form kept as the default.
pub(crate) fn lcd_default() -> Option<u8> {
    if cfg!(test) {
        return None;
    }
    let file = crate::recent_files::config_dir()?.join("lcd-default");
    std::fs::read_to_string(file).ok()?.trim().parse().ok()
}

fn save_lcd_default(mode: u8) {
    let Some(file) = crate::recent_files::config_dir().map(|d| d.join("lcd-default")) else {
        return;
    };
    let saved = file
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&file, mode.to_string()));
    if let Err(e) = saved {
        tracing::warn!("could not save the display form: {e}");
    }
}

fn lcd(ui: &mut Ui, g: &Painter, p: &mut Project, rect: Rect, cpu_load: f32, disk_load: f32) {
    let pal = LcdColors::new(p.recording);
    g.rect_filled(rect, 6.0, pal.bg);
    g.rect_stroke(
        rect,
        6.0,
        Stroke::new(1.0, crate::theme::gray(0x09)),
        StrokeKind::Inside,
    );
    let top = rect.top() + 4.0;
    let bot = rect.center().y + 1.0;
    let x0 = rect.left();

    // Column 1: SMPTE over bars/beats. Right-click the time for its frame rate and the MIDI
    // Time Code ports.
    let smpte = ui
        .interact(
            Rect::from_min_size(pos2(x0 + 4.0, rect.top()), vec2(150.0, bot - rect.top())),
            egui::Id::unique(("control_bar", "smpte")),
            Sense::click(),
        )
        .on_hover_text(tr("Right-click for the SMPTE and synchronization settings"));
    crate::widgets::context_menu(&smpte, |ui| crate::sync_menu::items(ui, p));
    let mut job = LayoutJob::default();
    lcd_seg(&mut job, &p.timecode(), 0, 18.0, pal.text, pal.dim);
    g.galley(pos2(x0 + 8.0, top), g.layout_job(job), pal.text);
    let pos = p.bbdt(p.position_beats);
    g.galley(
        pos2(x0 + 14.0, bot),
        g.layout_job(position_job(pos, 18.0, pal.text, pal.dim)),
        pal.text,
    );

    // Column 2: left/right locators (dimmer).
    let l = p.bbdt(p.cycle_start);
    let r = p.bbdt(p.cycle_end);
    g.galley(
        pos2(x0 + 158.0, top + 1.0),
        g.layout_job(position_job(l, 15.0, pal.mid, pal.dim)),
        pal.mid,
    );
    g.galley(
        pos2(x0 + 158.0, bot + 1.0),
        g.layout_job(position_job(r, 15.0, pal.mid, pal.dim)),
        pal.mid,
    );

    // Column 3: tempo (drag vertically to change it, like Logic).
    let tempo_rect = Rect::from_min_size(pos2(x0 + 252.0, rect.top()), vec2(84.0, rect.height()));
    let resp = ui
        .interact(
            tempo_rect,
            egui::Id::unique(("control_bar", "tempo")),
            Sense::click_and_drag(),
        )
        .on_hover_text("Click to type the tempo, or drag up and down");
    // With a Tempo track, this is the tempo at the playhead, and dragging changes that one.
    let tempo_now = p.tempo_at(p.position_beats);
    if resp.dragged() {
        p.set_tempo_at(p.position_beats, tempo_now - resp.drag_delta().y * 0.5);
    }
    let field = Rect::from_min_max(
        pos2(tempo_rect.left() + 2.0, top - 1.0),
        pos2(tempo_rect.right() - 2.0, top + 21.0),
    );
    if !tempo_field(ui, p, resp.clicked(), field) {
        g.text(
            pos2(tempo_rect.center().x, top),
            Align2::CENTER_TOP,
            format!("{:.4}", p.tempo_at(p.position_beats)),
            crate::theme::digit_font(18.0),
            pal.text,
        );
    }
    g.text(
        pos2(tempo_rect.center().x, bot + 2.0),
        Align2::CENTER_TOP,
        tr("Keep Tempo"),
        fitting_font(g, tr("Keep Tempo"), 13.5, 82.0),
        pal.text,
    );

    // Column 4: time signature / division. Drag the numerator or the denominator up or down,
    // like the tempo.
    let sig_x = x0 + 360.0;
    let half = vec2(24.0, bot - rect.top());
    signature_drag(
        ui,
        p,
        [
            Rect::from_min_size(pos2(sig_x - 24.0, rect.top()), half),
            Rect::from_min_size(pos2(sig_x, rect.top()), half),
        ],
    );
    g.text(
        pos2(sig_x, top),
        Align2::CENTER_TOP,
        {
            let (num, den) = p.signature_of_bar(p.beats_to_bar(p.position_beats) as u32);
            format!("{num}/{den}")
        },
        FontId::proportional(18.0),
        pal.text,
    );
    g.text(
        pos2(sig_x + 3.0, bot + 1.0),
        Align2::CENTER_TOP,
        "/16",
        FontId::proportional(16.0),
        pal.text,
    );

    // Column 5: MIDI in/out activity.
    g.text(
        pos2(x0 + 386.0, top + 3.0),
        Align2::LEFT_TOP,
        tr("No In"),
        fitting_font(g, tr("No In"), 13.5, 84.0),
        pal.text,
    );
    g.text(
        pos2(x0 + 386.0, bot + 2.0),
        Align2::LEFT_TOP,
        tr("No Out"),
        fitting_font(g, tr("No Out"), 13.5, 84.0),
        pal.text,
    );

    // Column 6: CPU (audio-callback DSP load) / HD (how busy reading audio files is) meters.
    for (i, label) in ["CPU", "HD"].iter().enumerate() {
        let y = rect.top() + 12.0 + i as f32 * 18.0;
        g.text(
            pos2(x0 + 474.0, y),
            Align2::LEFT_CENTER,
            *label,
            FontId::proportional(7.0),
            pal.dim,
        );
        let m = Rect::from_min_size(pos2(x0 + 490.0, y - 2.0), vec2(40.0, 4.0));
        g.rect_filled(m, 1.0, crate::theme::gray(0x18));
        if i == 1 {
            // HD, as in Logic: how much of its time the disk thread spends reading. Red when the drive
            // cannot keep up (a slow or external drive, many tracks at once).
            let load = disk_load.clamp(0.0, 1.0);
            let fill = Rect::from_min_size(m.min, vec2(m.width() * load, m.height()));
            let color = if disk_load >= 0.8 { Color32::from_rgb(0xe0, 0x40, 0x30) } else { pal.text };
            g.rect_filled(fill, 1.0, color);
            let hover = m.expand2(vec2(14.0, 4.0)).with_min_x(x0 + 470.0);
            let resp = ui
                .interact(hover, egui::Id::unique(("lcd", "hd_meter")), egui::Sense::click())
                .on_hover_text(format!(
                    "Disk (HD) load: {:.0}%. Reading audio files for playback. High or red: the drive is too slow for this many tracks; copy the samples to an internal drive, or bounce tracks. Click for the Performance window.",
                    disk_load * 100.0
                ));
            if resp.clicked() {
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique("open_performance"), true));
            }
        }
        if i == 0 {
            let load = cpu_load.clamp(0.0, 1.0);
            let fill = Rect::from_min_size(m.min, vec2(m.width() * load, m.height()));
            let color = if cpu_load >= 0.9 {
                Color32::from_rgb(0xe0, 0x40, 0x30)
            } else {
                pal.text
            };
            g.rect_filled(fill, 1.0, color);
            let hover = m.expand2(vec2(14.0, 4.0)).with_min_x(x0 + 470.0);
            ui.interact(
                hover,
                egui::Id::unique(("lcd", "cpu_meter")),
                egui::Sense::hover(),
            )
            .on_hover_text(trf(
                "Audio DSP load: {}%",
                &[&format!("{:.0}", cpu_load * 100.0)],
            ));
        }
    }

    g.line_segment(
        [
            pos2(rect.right() - 22.0, rect.top() + 6.0),
            pos2(rect.right() - 22.0, rect.bottom() - 6.0),
        ],
        Stroke::new(1.0, crate::theme::gray(0x17)),
    );
}

/// Level (0..1) for a peak amplitude: -60 dB at the left end, +6 dB at the right end.
fn meter_frac(peak: f32) -> f32 {
    if peak <= 1e-6 {
        return 0.0;
    }
    ((20.0 * peak.log10() + 60.0) / 66.0).clamp(0.0, 1.0)
}

/// Master volume as in Logic Pro: a dark pill holding two thin stereo meter bars (green, yellow
/// above -6 dB, red above 0 dB) with a translucent round knob sliding over them.
fn master_volume(ui: &Ui, g: &Painter, p: &mut Project, rect: Rect, peaks: Option<(f32, f32)>) {
    const RAD: f32 = 10.0;
    let travel = (rect.left() + RAD + 1.0, rect.right() - RAD - 1.0);
    let resp = ui.interact(
        rect.expand2(vec2(4.0, 4.0)),
        egui::Id::unique(("control_bar", "master")),
        Sense::click_and_drag(),
    );
    if let Some(pt) = resp.interact_pointer_pos() {
        p.master_vol = ((pt.x - travel.0) / (travel.1 - travel.0)).clamp(0.0, 1.0);
    }

    // Pill with a soft inner edge.
    let r = rect.height() / 2.0;
    g.rect_filled(rect, r, crate::theme::gray(0x18));
    g.rect_stroke(
        rect,
        r,
        Stroke::new(1.0, crate::theme::gray(0x29)),
        egui::StrokeKind::Inside,
    );
    g.rect_stroke(
        rect.shrink(1.5),
        r - 1.5,
        Stroke::new(1.0, crate::theme::gray(0x0f)),
        egui::StrokeKind::Inside,
    );

    // Two stereo meter bars.
    let inner = Rect::from_min_max(
        pos2(rect.left() + 9.0, rect.top() + 5.0),
        pos2(rect.right() - 9.0, rect.bottom() - 5.0),
    );
    let bar_h = ((inner.height() - 2.0) / 2.0).floor().max(3.0);
    let green = Color32::from_rgb(0x2f, 0xa8, 0x3c);
    let yellow = Color32::from_rgb(0xb4, 0xa0, 0x20);
    let red = Color32::from_rgb(0xd8, 0x3a, 0x2c);
    let (l, rr) = peaks.unwrap_or((0.0, 0.0));
    for (i, peak) in [l, rr].into_iter().enumerate() {
        let y = inner.top() + i as f32 * (bar_h + 2.0);
        let bar = Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), bar_h));
        g.rect_filled(bar, 2.0, crate::theme::gray(0x10));
        let f = meter_frac(peak);
        for (lo, hi, col) in [
            (0.0f32, 0.82f32, green),
            (0.82, 0.91, yellow),
            (0.91, 1.0, red),
        ] {
            let x1 = bar.left() + bar.width() * lo;
            let x2 = bar.left() + bar.width() * hi.min(f);
            if x2 > x1 {
                let seg = Rect::from_min_max(pos2(x1, bar.top()), pos2(x2, bar.bottom()));
                g.rect_filled(seg, 1.5, col);
                // Light centre line and a darker lower edge give the bar its soft round look.
                let mid = seg.center().y;
                g.line_segment(
                    [pos2(seg.left(), mid), pos2(seg.right(), mid)],
                    Stroke::new(1.0, col.lerp_to_gamma(Color32::WHITE, 0.35)),
                );
                g.line_segment(
                    [
                        pos2(seg.left(), seg.bottom() - 0.5),
                        pos2(seg.right(), seg.bottom() - 0.5),
                    ],
                    Stroke::new(1.0, col.lerp_to_gamma(Color32::BLACK, 0.35)),
                );
            }
        }
    }

    // Translucent glassy knob.
    let c = pos2(
        travel.0 + (travel.1 - travel.0) * p.master_vol,
        rect.center().y,
    );
    g.circle_filled(c + vec2(0.0, 1.0), RAD + 1.0, Color32::from_black_alpha(70));
    g.circle_filled(
        c,
        RAD,
        Color32::from_rgba_unmultiplied(0xd2, 0xd2, 0xd2, 0xc8),
    );
    g.circle_stroke(
        c,
        RAD,
        Stroke::new(1.2, Color32::from_rgba_unmultiplied(0xf2, 0xf2, 0xf2, 0xe6)),
    );
    g.circle_stroke(
        c,
        RAD - 3.5,
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(0x8a, 0x8a, 0x8a, 0x90)),
    );
}

pub fn control_bar(
    ui: &mut Ui,
    p: &mut Project,
    s: &mut UiState,
    master_peaks: Option<(f32, f32)>,
) {
    let bar = ui.max_rect();
    let g = ui.painter_at(bar);
    g.rect_filled(bar, 0.0, bar_bg_col());
    g.line_segment(
        [bar.left_bottom(), bar.right_bottom()],
        Stroke::new(1.0, crate::theme::border()),
    );
    let cy = bar.center().y;

    // Left: Library / Inspector / Quick Help / Toolbar.
    let left = [
        (icon_library as IconFn, s.show_library),
        (icon_inspector, s.show_inspector),
        (icon_help, s.show_quick_help),
        (icon_toolbar, s.show_toolbar),
    ];
    match segmented(ui, &g, bar.left() + 8.0, cy, "view", &left) {
        Some(0) => s.show_library = !s.show_library,
        Some(1) => s.show_inspector = !s.show_inspector,
        Some(2) => s.show_quick_help = !s.show_quick_help,
        Some(3) => s.show_toolbar = !s.show_toolbar,
        _ => {}
    }
    // Smart Controls / Mixer / Editors.
    let editors_on = s.bottom == Bottom::PianoRoll;
    let mid = [
        (icon_smart_controls as IconFn, s.light_menus),
        (icon_mixer, s.show_mixer && s.bottom == Bottom::Mixer),
        (icon_editors, editors_on),
    ];
    match segmented(ui, &g, bar.left() + 140.0, cy, "panes", &mid) {
        // Switches the menu bar and its menus between dark and light (white).
        Some(0) => s.light_menus = !s.light_menus,
        Some(1) => {
            s.show_mixer = !(s.show_mixer && s.bottom == Bottom::Mixer);
            s.bottom = Bottom::Mixer;
        }
        Some(2) => {
            s.bottom = if editors_on {
                Bottom::Mixer
            } else {
                Bottom::PianoRoll
            };
        }
        _ => {}
    }

    // Centre: LCD, transport to its left, count-in & co. to its right.
    let lcd_w = if s.lcd_full { 555.0 } else { 226.0 };
    let lcd_rect = Rect::from_center_size(pos2(bar.center().x, cy), vec2(lcd_w, 44.0));
    if s.lcd_full {
        lcd(ui, &g, p, lcd_rect, s.cpu_load, s.disk_load);
        if ui.ctx().data_mut(|d| d.remove_temp::<bool>(egui::Id::unique("open_performance"))).unwrap_or(false) {
            s.performance.open = true;
        }
    } else {
        match s.lcd_mode {
            0 => lcd_compact(ui, &g, p, lcd_rect),
            mode => lcd_text(&g, p, lcd_rect, mode),
        }
    }
    let form = lcd_arrow(ui, &g, lcd_rect, p.recording);
    egui::Popup::menu(&form).show(|ui| {
        ui.set_min_width(190.0);
        for (name, mode) in [
            ("Beats & Project", 0u8),
            ("Beats & Time", 1),
            ("Beats", 2),
            ("Time", 3),
        ] {
            if ui
                .selectable_label(!s.lcd_full && s.lcd_mode == mode, name)
                .clicked()
            {
                s.lcd_full = false;
                s.lcd_mode = mode;
                ui.close();
            }
        }
        if ui.selectable_label(s.lcd_full, "Custom").clicked() {
            s.lcd_full = true;
            ui.close();
        }
        ui.separator();
        if ui.button("Open Giant Beats Display").clicked() {
            s.dialogs.giant_beats = true;
            ui.close();
        }
        if ui.button("Open Giant Time Display").clicked() {
            s.dialogs.giant_time = true;
            ui.close();
        }
        ui.separator();
        if ui.button("Apply Defaults").clicked() {
            let chosen = lcd_default().unwrap_or(0);
            s.lcd_full = chosen == CUSTOM_LCD;
            s.lcd_mode = if s.lcd_full { 0 } else { chosen };
            ui.close();
        }
        if ui.button("Save As Defaults").clicked() {
            save_lcd_default(if s.lcd_full { CUSTOM_LCD } else { s.lcd_mode });
            ui.close();
        }
    });
    // The transport buttons sit on one rounded plate, as do those to the right of the display.
    crate::theme::glass_plate(
        &g,
        Rect::from_min_max(
            pos2(lcd_rect.left() - 214.0, cy - 15.0),
            pos2(lcd_rect.left() - 4.0, cy + 15.0),
        ),
        7.0,
    );

    let tx = |dx: f32| pos2(lcd_rect.left() + dx, cy);
    if tbutton(ui, &g, tx(-198.0), "rew", icon_rewind, None, icon_col()) {
        // Back to the start of this bar, or of the one before when already there.
        let bar = p.beats_to_bar(p.position_beats);
        let back = if bar.fract() < 1e-6 {
            bar - 1.0
        } else {
            bar.floor()
        };
        p.position_beats = p.bar_to_beats(back.max(0.0));
    }
    if tbutton(ui, &g, tx(-170.0), "ffw", icon_forward, None, icon_col()) {
        p.position_beats = p.next_bar_start(p.position_beats);
    }
    if tbutton(ui, &g, tx(-142.0), "start", icon_to_start, None, icon_col()) {
        p.position_beats = 0.0;
    }
    // Logic keeps the triangle while playing; only the background turns green.
    let play_icon: IconFn = icon_play;
    let play_bg = p.playing.then_some(crate::theme::accent());
    if tbutton(
        ui,
        &g,
        tx(-108.0),
        "play",
        play_icon,
        play_bg,
        if p.playing {
            crate::theme::on_accent()
        } else {
            icon_col()
        },
    ) {
        p.playing = !p.playing;
    }
    // Record: red dot (white dot on red while recording).
    let rec_r = Rect::from_center_size(tx(-76.0), vec2(30.0, 26.0));
    if p.recording {
        g.rect_filled(rec_r, 6.0, REC_BUTTON_ON);
    }
    g.circle_filled(
        rec_r.center(),
        6.5,
        if p.recording { Color32::WHITE } else { REC_RED },
    );
    if clicked(ui, rec_r, "rec") {
        p.press_record();
    }
    // Capture Recording: red dot inside a ring.
    let cap_r = Rect::from_center_size(tx(-48.0), vec2(30.0, 26.0));
    g.circle_stroke(
        cap_r.center(),
        7.5,
        Stroke::new(1.6, Color32::from_rgb(0xe8, 0x6a, 0x3a)),
    );
    g.circle_filled(cap_r.center(), 4.8, REC_RED);
    if clicked(ui, cap_r, "capture") {
        s.capture_cmd = true;
    }
    let cyc_bg = p.cycle.then_some(crate::theme::accent());
    if tbutton(
        ui,
        &g,
        tx(-18.0),
        "cycle",
        icon_cycle,
        cyc_bg,
        if p.cycle {
            crate::theme::on_accent()
        } else {
            icon_col()
        },
    ) {
        crate::key_commands::toggle_cycle(p, &s.tracks);
    }

    let rx0 = |dx: f32| pos2(lcd_rect.right() + dx, cy);
    // Mode buttons after the display: Typing Keyboard by default; right-click to choose.
    let modes = s.header_modes.clone();
    // Each button turns its mode on and off; a right-click on any of them chooses which show.
    let mut chosen: Option<(HeaderMode, bool)> = None;
    for (k, mode) in modes.iter().enumerate() {
        let c = rx0(24.0 + k as f32 * MODE_STEP);
        let on = match mode {
            HeaderMode::Typing => s.typing.on,
            HeaderMode::LowLatency => p.low_latency,
        };
        let (key, draw): (&str, IconFn) = match mode {
            HeaderMode::Typing => ("typing", icon_typing),
            HeaderMode::LowLatency => ("lowlat", icon_low_latency),
        };
        let r = Rect::from_center_size(c, vec2(30.0, 26.0));
        let mut resp = ui.interact(r, egui::Id::unique(("control_bar", key)), Sense::click());
        if let Some(name) = button_name(key) {
            resp = resp.on_hover_text(format!("{name}\nRight-click to choose the mode buttons shown here"));
        }
        if on {
            g.rect_filled(r, 6.0, crate::theme::accent());
        } else if resp.hovered() {
            g.rect_filled(r, 6.0, Color32::from_white_alpha(14));
        }
        draw(&g, r, if on { Color32::WHITE } else { icon_col() });
        if resp.clicked() {
            match mode {
                HeaderMode::Typing => s.typing.on = !s.typing.on,
                HeaderMode::LowLatency => p.low_latency = !p.low_latency,
            }
        }
        resp.context_menu(|ui| {
            if let Some(c) = modes_menu(ui, &modes) {
                chosen = Some(c);
            }
        });
    }
    if modes.is_empty() {
        let r = Rect::from_center_size(rx0(24.0), vec2(30.0, 26.0));
        g.text(r.center(), Align2::CENTER_CENTER, "+", FontId::proportional(14.0), icon_col());
        let resp = ui
            .interact(r, egui::Id::unique(("control_bar", "modes_menu")), Sense::click())
            .on_hover_text("Click to show the Typing Keyboard button; right-click to choose");
        if resp.clicked() {
            chosen = Some((HeaderMode::Typing, true));
        }
        resp.context_menu(|ui| {
            if let Some(c) = modes_menu(ui, &modes) {
                chosen = Some(c);
            }
        });
    }
    if let Some((mode, shown)) = chosen {
        if shown {
            s.header_modes.push(mode);
        } else {
            s.header_modes.retain(|m| *m != mode);
        }
    }
    // What follows moves along by the extra mode buttons.
    let shift = modes.len().saturating_sub(1) as f32 * MODE_STEP;
    let rx = |dx: f32| pos2(lcd_rect.right() + dx + shift, cy);
    // Count-in "1234": purple when on.
    let ci = Rect::from_center_size(rx(68.0), vec2(28.0, 24.0));
    g.rect_filled(ci, 5.0, if p.count_in { PURPLE } else { seg_bg_col() });
    g.text(
        ci.center(),
        Align2::CENTER_CENTER,
        "1234",
        FontId::proportional(10.0),
        if p.count_in { Color32::WHITE } else { icon_col() },
    );
    let resp = ui
        .interact(ci, egui::Id::unique(("control_bar", "countin")), Sense::click())
        .on_hover_text("Count-in. Right-click for pre-roll and post-roll");
    if resp.clicked() {
        p.count_in = !p.count_in;
    }
    if p.pre_roll > 0.0 || p.post_roll > 0.0 {
        // A dot under it says a roll is set.
        g.circle_filled(ci.center_bottom() + vec2(0.0, 3.0), 2.0, PURPLE);
    }
    resp.context_menu(|ui| {
        for (label, roll) in [("Pre-roll", &mut p.pre_roll), ("Post-roll", &mut p.post_roll)] {
            ui.menu_button(label, |ui| {
                for (name, bars) in [("Off", 0.0f32), ("1 bar", 1.0), ("2 bars", 2.0), ("4 bars", 4.0)] {
                    if ui.selectable_label(*roll == bars, name).clicked() {
                        *roll = bars;
                        ui.close();
                    }
                }
            });
        }
        ui.label(egui::RichText::new("Pre-roll: recording from a stop starts earlier and comes in where the playhead was. Post-roll: the song plays on after the recording ends.").small());
    });
    let met_bg = p.metronome.then_some(PURPLE);
    if tbutton(
        ui,
        &g,
        rx(98.0),
        "metronome",
        icon_metronome,
        met_bg,
        if p.metronome { Color32::WHITE } else { icon_col() },
    ) {
        p.metronome = !p.metronome;
    }

    // Right: master volume, then List Editors / Note Pads / Loop Browser / Browsers.
    // The volume sits after the metronome and before the icons, narrower if it must be.
    let (vol_left, vol_w) = volume_span(rx(114.0).x, bar.right());
    if vol_w > 0.0 {
        let vol = Rect::from_min_size(pos2(vol_left, cy - 11.0), vec2(vol_w, 22.0));
        master_volume(ui, &g, p, vol, master_peaks);
    }
    let right: [IconFn; 4] = [icon_list_editors, icon_note_pad, icon_loops, icon_browsers];
    for (i, draw) in right.iter().enumerate() {
        let c = pos2(bar.right() - 106.0 + i as f32 * 27.5, cy);
        let shown = match i {
            0 => s.show_lists,
            1 => s.show_notes,
            2 => s.browser.open == Some(Tab::Loops),
            _ => s.browser.open == Some(Tab::Files),
        };
        let on = shown.then_some(crate::widgets::blue());
        let color = if on.is_some() { Color32::WHITE } else { icon_col() };
        if tbutton(ui, &g, c, &format!("right{i}"), *draw, on, color) {
            match i {
                0 => s.show_lists = !s.show_lists,
                1 => s.show_notes = !s.show_notes,
                2 => s.browser.toggle(Tab::Loops),
                _ => s.browser.toggle(Tab::Files),
            }
        }
    }
}

/// Master volume slider: its usual place and width (140 points, 188 from the right edge),
/// moved right of the metronome (which ends at `after`) and narrowed so it never reaches the
/// icons at the right edge. Width 0 = no room at all, leave it out.
fn volume_span(after: f32, right: f32) -> (f32, f32) {
    const WIDTH: f32 = 140.0;
    const MIN_WIDTH: f32 = 60.0;
    let icons_left = right - 120.0;
    let left = (right - 188.0 - WIDTH).max(after + 14.0);
    let width = WIDTH.min(icons_left - 8.0 - left);
    if width < MIN_WIDTH {
        (left, 0.0)
    } else {
        (left, width)
    }
}

/// The largest font, up to `size`, that fits `text` into `max_w` points: an LCD column has a
/// fixed width, and some languages need more room (German, Finnish).
fn fitting_font(g: &Painter, text: &str, size: f32, max_w: f32) -> FontId {
    let w = g
        .layout_no_wrap(text.to_owned(), FontId::proportional(size), lcd_text_col())
        .size()
        .x;
    FontId::proportional(if w > max_w {
        (size * max_w / w).max(8.0)
    } else {
        size
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tempo_can_be_typed_and_esc_leaves_it() {
        let mut p = Project::empty();
        p.tempo = 120.0;
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 44.0));
        let at = pos2(rect.left() + 252.0 + 42.0, rect.center().y - 8.0);
        let mut h = crate::ui_test::Harness::new(700.0, 200.0);
        let draw = |ui: &mut Ui, p: &mut Project| {
            let g = ui.painter().clone();
            lcd(ui, &g, p, rect, 0.0, 0.0);
        };
        h.settle(|ui| draw(ui, &mut p));
        h.click(at, |ui| draw(ui, &mut p));
        h.frame(vec![egui::Event::Text("98.5".into())], |ui| {
            draw(ui, &mut p)
        });
        assert_eq!(p.tempo, 120.0, "not until it is entered");
        h.key(egui::Key::Enter, |ui| draw(ui, &mut p));
        assert_eq!(p.tempo, 98.5);
        h.settle(|ui| draw(ui, &mut p));
        h.click(at, |ui| draw(ui, &mut p));
        h.frame(vec![egui::Event::Text("200".into())], |ui| draw(ui, &mut p));
        h.key(egui::Key::Escape, |ui| draw(ui, &mut p));
        assert_eq!(p.tempo, 98.5);
    }

    #[test]
    fn the_volume_never_covers_the_metronome_or_the_icons() {
        // Wide bar: the usual place and width.
        assert_eq!(volume_span(1200.0, 2000.0), (2000.0 - 328.0, 140.0));
        // Narrow bar: after the metronome, narrowed before the icons.
        let (left, width) = volume_span(1300.0, 1470.0);
        assert!(left >= 1300.0 + 14.0);
        assert!(left + width <= 1470.0 - 120.0 - 8.0 + 1e-3);
        assert!(width >= 60.0 || width == 0.0);
        // No room: left out.
        assert_eq!(volume_span(1400.0, 1470.0).1, 0.0);
    }

    #[test]
    fn slow_drags_add_up_to_whole_steps() {
        let mut left = 0.0;
        let total: i32 = (0..25).map(|_| take_steps(&mut left, 1.0, 10.0)).sum();
        assert_eq!(total, 2);
        assert!((left - 5.0).abs() < 1e-4);

        // Dragging down gives negative steps, and a reversal cancels what was built up.
        let mut left = 0.0;
        assert_eq!(take_steps(&mut left, -25.0, 10.0), -2);
        assert_eq!(take_steps(&mut left, 25.0, 10.0), 2);
        assert!(left.abs() < 1e-4);
    }
}
