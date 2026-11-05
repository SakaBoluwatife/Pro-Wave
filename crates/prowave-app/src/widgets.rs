//! Small Logic-style widgets shared by the local toolbars (Tracks area, Mixer).
use crate::i18n::tr;
use egui::{
    pos2, vec2, Align2, Color32, FontId, Id, Painter, Pos2, Rect, Response, Sense, Shape, Stroke,
    Ui,
};

pub const TEXT: Color32 = crate::theme::TEXT;
pub const TEXT_DIM: Color32 = crate::theme::TEXT_DIM;
pub const BTN_BG: Color32 = crate::theme::BUTTON;
pub const BLUE: Color32 = crate::theme::ACCENT_BLUE;

pub fn stroke(c: Color32) -> Stroke {
    Stroke::new(1.3, c)
}

/// Whether `resp` asks for its right-click menu this frame: a right-click (or long touch), and
/// on macOS also a Control-click, as with a one-button mouse or a trackpad.
pub fn context_click(resp: &Response) -> bool {
    let control_click =
        cfg!(target_os = "macos") && resp.clicked() && resp.ctx.input(|i| i.modifiers.ctrl);
    resp.secondary_clicked() || control_click
}

/// Right-click menu at the pointer for `resp` (see [`context_click`]). Closes when an entry is
/// clicked, the pointer clicks elsewhere or Escape is pressed.
pub fn context_menu(resp: &Response, add_contents: impl FnOnce(&mut Ui)) {
    context_menu_shown(resp, false, add_contents);
}

/// [`context_menu`], or with `shown` the menu open over the middle of `resp` without a click
/// (only for the layout pictures, `--shots`).
pub fn context_menu_shown(resp: &Response, shown: bool, add_contents: impl FnOnce(&mut Ui)) {
    let open = if context_click(resp) || shown {
        Some(egui::SetOpenCommand::Bool(true))
    } else if resp.clicked() {
        Some(egui::SetOpenCommand::Bool(false))
    } else {
        None
    };
    let popup = egui::Popup::menu(resp).open_memory(open);
    let popup = if shown {
        popup.at_position(resp.rect.center())
    } else {
        popup.at_pointer_fixed()
    };
    popup.show(|ui| {
        ui.set_min_width(190.0);
        add_contents(ui);
    });
}

/// Text button with a small down chevron (Logic's "Edit ⌄"). Returns the response and width.
pub fn menu_title(
    ui: &Ui,
    g: &Painter,
    left: f32,
    cy: f32,
    label: &str,
    id: Id,
) -> (Response, f32) {
    let label = tr(label);
    let font = FontId::proportional(13.0);
    let w = g.layout_no_wrap(label.into(), font.clone(), TEXT).size().x + 22.0;
    let r = Rect::from_min_size(pos2(left, cy - 10.0), vec2(w, 20.0));
    let resp = ui.interact(r, id, Sense::click());
    let fill = if resp.hovered() {
        crate::theme::BUTTON_HOVERED
    } else {
        crate::theme::BUTTON
    };
    g.rect_filled(r, 4.0, fill);
    g.text(
        pos2(r.left() + 6.0, cy),
        Align2::LEFT_CENTER,
        label,
        font,
        TEXT,
    );
    let c = pos2(r.right() - 8.0, cy + 1.0);
    g.add(Shape::line(
        vec![
            c + vec2(-3.0, -1.5),
            c + vec2(0.0, 1.5),
            c + vec2(3.0, -1.5),
        ],
        stroke(TEXT_DIM),
    ));
    (resp, w)
}

/// Popup button with a double chevron (Logic's "Smart ⇕").
pub fn popup_button(ui: &Ui, g: &Painter, rect: Rect, id: Id, text: &str) -> Response {
    let resp = ui.interact(rect, id, Sense::click());
    g.rect_filled(
        rect,
        4.0,
        if resp.hovered() {
            Color32::from_gray(0x60)
        } else {
            BTN_BG
        },
    );
    // Long values (some languages) end in "…" instead of running under the arrows.
    let font = FontId::proportional(12.5);
    let shown = crate::mixer::elide(g, text, &font, rect.width() - 26.0);
    g.text(
        pos2(rect.left() + 8.0, rect.center().y),
        Align2::LEFT_CENTER,
        shown,
        font,
        TEXT,
    );
    let c = pos2(rect.right() - 9.0, rect.center().y);
    g.add(Shape::line(
        vec![
            c + vec2(-2.5, -1.5),
            c + vec2(0.0, -4.0),
            c + vec2(2.5, -1.5),
        ],
        stroke(TEXT),
    ));
    g.add(Shape::line(
        vec![c + vec2(-2.5, 1.5), c + vec2(0.0, 4.0), c + vec2(2.5, 1.5)],
        stroke(TEXT),
    ));
    resp
}

/// Back chevron at the start of a local toolbar (dim: nothing to go back to yet).
pub fn back_chevron(g: &Painter, at: egui::Pos2) {
    g.add(Shape::line(
        vec![
            at + vec2(3.0, -5.0),
            at + vec2(-2.0, 0.0),
            at + vec2(3.0, 5.0),
        ],
        stroke(TEXT_DIM),
    ));
}

/// Entries that exist in Logic but are not implemented yet are drawn disabled.
/// Undo and Redo entries for the local Edit menus, greyed when there is nothing to undo or
/// redo. The one picked goes to `pick` (and from there to `UiState::undo_cmd`).
pub fn undo_items(
    ui: &mut Ui,
    can_undo: bool,
    can_redo: bool,
    pick: &mut Option<crate::undo::UndoCmd>,
) {
    use egui::{Key, KeyboardShortcut, Modifiers};
    let key = |m, k| ui.ctx().format_shortcut(&KeyboardShortcut::new(m, k));
    let (undo_key, redo_key) = (
        key(Modifiers::COMMAND, Key::Z),
        key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z),
    );
    if ui
        .add_enabled(
            can_undo,
            egui::Button::new(tr("Undo")).shortcut_text(undo_key),
        )
        .clicked()
    {
        *pick = Some(crate::undo::UndoCmd::Undo);
        ui.close();
    }
    if ui
        .add_enabled(
            can_redo,
            egui::Button::new(tr("Redo")).shortcut_text(redo_key),
        )
        .clicked()
    {
        *pick = Some(crate::undo::UndoCmd::Redo);
        ui.close();
    }
}

pub fn todo_item(ui: &mut Ui, label: &str) {
    ui.add_enabled(false, egui::Button::new(tr(label)));
}

/// Horizontal slider (thin groove, round knob). Returns the new 0..1 value when dragged.
pub fn slider_h(
    ui: &Ui,
    g: &Painter,
    rect: Rect,
    id: Id,
    value: f32,
    enabled: bool,
) -> Option<f32> {
    let cy = rect.center().y;
    let v = value.clamp(0.0, 1.0);
    let groove = Rect::from_min_max(pos2(rect.left(), cy - 2.0), pos2(rect.right(), cy + 2.0));
    g.rect_filled(groove, 2.0, crate::theme::BORDER);
    let knob = pos2(rect.left() + 6.0 + (rect.width() - 12.0) * v, cy);
    let (fill, edge) = if enabled {
        (Color32::from_gray(0xc4), Color32::from_gray(0x70))
    } else {
        (Color32::from_gray(0x6a), Color32::from_gray(0x50))
    };
    g.circle_filled(knob, 6.0, fill);
    g.circle_stroke(knob, 6.0, Stroke::new(1.0, edge));
    if !enabled {
        return None;
    }
    let resp = ui.interact(rect.expand2(vec2(2.0, 5.0)), id, Sense::click_and_drag());
    resp.interact_pointer_pos()
        .map(|p| ((p.x - rect.left() - 6.0) / (rect.width() - 12.0)).clamp(0.0, 1.0))
}

/// Logic's rotary knob: dark bezel, light grey domed body, a white pointer, and two small tick
/// marks flanking the bottom. `value` runs from -1 (full left) to 1 (full right), 0 = up.
/// The colour of a knob's arc.
const KNOB_GLOW: Color32 = Color32::from_rgb(0x4f, 0xc3, 0xf7);

/// The lit arc around a knob, from `from` to `to` (each -1..=1 of its sweep), with a soft glow, on
/// a dim ring showing the whole sweep. It tells at a glance how far a knob is turned, and which way.
pub fn value_arc(g: &Painter, c: Pos2, r: f32, from: f32, to: f32) {
    const SWEEP: f32 = 135.0;
    let arc = |a: f32, b: f32| -> Vec<Pos2> {
        let steps = (((b - a).abs() * 16.0).ceil() as usize).max(1);
        (0..=steps)
            .map(|k| {
                let v = a + (b - a) * k as f32 / steps as f32;
                let angle = (-90.0 + v.clamp(-1.0, 1.0) * SWEEP).to_radians();
                c + vec2(angle.cos(), angle.sin()) * r
            })
            .collect()
    };
    g.add(egui::Shape::line(
        arc(-1.0, 1.0),
        Stroke::new(1.6, crate::theme::BORDER),
    ));
    if (to - from).abs() < 0.01 {
        // Centred: a small lit point says so.
        let angle = (-90.0 + to.clamp(-1.0, 1.0) * SWEEP).to_radians();
        let at = c + vec2(angle.cos(), angle.sin()) * r;
        g.circle_filled(at, 2.6, KNOB_GLOW.gamma_multiply(0.35));
        g.circle_filled(at, 1.2, KNOB_GLOW);
        return;
    }
    let lit = arc(from, to);
    g.add(egui::Shape::line(
        lit.clone(),
        Stroke::new(4.0, KNOB_GLOW.gamma_multiply(0.3)),
    ));
    g.add(egui::Shape::line(lit, Stroke::new(1.6, KNOB_GLOW)));
}

pub fn draw_knob(g: &Painter, c: Pos2, r: f32, value: f32) {
    value_arc(g, c, r + 3.6, 0.0, value);
    // Soft shadow and dark bezel.
    g.circle_filled(c + vec2(0.0, 1.0), r + 2.2, Color32::from_black_alpha(80));
    g.circle_filled(c, r + 1.6, Color32::from_gray(0x2a));
    // Domed body: darker edge, lighter centre and a highlight up and to the left.
    g.circle_filled(c, r, Color32::from_gray(0x7c));
    g.circle_filled(
        c + vec2(-r * 0.12, -r * 0.18),
        r * 0.8,
        Color32::from_gray(0x92),
    );
    g.circle_filled(
        c + vec2(-r * 0.2, -r * 0.3),
        r * 0.45,
        Color32::from_gray(0xa2),
    );
    g.circle_stroke(c, r, Stroke::new(1.0, Color32::from_gray(0x56)));
    // Pointer.
    let a = (-90.0 + value.clamp(-1.0, 1.0) * 135.0).to_radians();
    let d = vec2(a.cos(), a.sin());
    g.line_segment(
        [c + d * (r * 0.18), c + d * (r * 0.9)],
        Stroke::new(1.8, Color32::from_gray(0xf6)),
    );
}
