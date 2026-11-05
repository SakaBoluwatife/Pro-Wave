//! The look of the program in one place: dark gray chrome. Colours and sizes used by more than one
//! screen are named here, so changing one changes it everywhere.
use egui::{Color32, CornerRadius, Stroke};

// Surfaces, from the back of the window forwards.
pub const BG: Color32 = Color32::from_rgb(0x4e, 0x4e, 0x50);
pub const LANE: Color32 = Color32::from_rgb(0x3a, 0x3a, 0x3a);
pub const GRID_BG: Color32 = Color32::from_rgb(0x33, 0x33, 0x33);
pub const MIXER_BG: Color32 = Color32::from_rgb(0x46, 0x46, 0x46);
pub const PANEL_BG: Color32 = Color32::from_rgb(0x50, 0x50, 0x50);
pub const TOOLBAR: Color32 = Color32::from_rgb(0x4c, 0x4c, 0x4c);
pub const STRIP: Color32 = Color32::from_rgb(0x50, 0x50, 0x50);
pub const HEADER: Color32 = Color32::from_rgb(0x58, 0x58, 0x58);
pub const PANEL: Color32 = Color32::from_rgb(0x56, 0x56, 0x58);
pub const BORDER: Color32 = Color32::from_gray(0x24);

// Text.
pub const TEXT: Color32 = Color32::from_rgb(0xdc, 0xdc, 0xdc);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x96, 0x96, 0x96);

// Controls and their states.
pub const BUTTON: Color32 = Color32::from_rgb(0x6c, 0x6c, 0x6c);
pub const BUTTON_HOVERED: Color32 = Color32::from_rgb(0x7a, 0x7a, 0x7c);
pub const HOVERED: Color32 = Color32::from_rgb(0x5e, 0x5e, 0x5e);
pub const SELECTED: Color32 = Color32::from_rgb(0x6a, 0x6a, 0x6a);
pub const ACCENT_BLUE: Color32 = Color32::from_rgb(0x2f, 0x6f, 0xd8);
pub const DISABLED_ALPHA: f32 = 0.45;

// Mixer strips.
pub const STRIP_SELECTED: Color32 = Color32::from_rgb(0x60, 0x60, 0x60);
pub const SLOT_EMPTY: Color32 = Color32::from_rgb(0x46, 0x46, 0x46);
pub const SLOT: Color32 = Color32::from_rgb(0x6e, 0x6e, 0x6e);
pub const SLOT_OFF: Color32 = Color32::from_rgb(0x5a, 0x5a, 0x5a);
pub const METER_OFF: Color32 = Color32::from_rgb(0x70, 0x70, 0x70);

// Meanings.
pub const GOOD: Color32 = Color32::from_rgb(0x5f, 0xc8, 0x6a);
pub const MUTE: Color32 = Color32::from_rgb(0x3b, 0x8c, 0xe6);
pub const SOLO: Color32 = Color32::from_rgb(0xe6, 0xc2, 0x2e);
pub const DANGER: Color32 = Color32::from_rgb(0xe6, 0x34, 0x2c);

// Spacing and type.
pub const ITEM_SPACING: f32 = 6.0;
pub const TEXT_SIZE: f32 = 12.5;
pub const CORNER: u8 = 4;

// Frosted glass: floating things let a little of what is behind them through.
/// How opaque a floating window or menu is.
const GLASS_ALPHA: u8 = 228;
/// The fill of floating windows and menus.
pub const GLASS_FILL: Color32 = Color32::from_rgba_premultiplied(
    (0x56 * GLASS_ALPHA as u16 / 255) as u8,
    (0x56 * GLASS_ALPHA as u16 / 255) as u8,
    (0x58 * GLASS_ALPHA as u16 / 255) as u8,
    GLASS_ALPHA,
);
/// The soft light edge of frosted things.
pub const GLASS_EDGE: Color32 = Color32::from_rgba_premultiplied(44, 44, 44, 44);

/// Paints a frosted plate: lighter at the top, with a soft highlight along its upper edge. A few
/// flat shapes, so it costs next to nothing to draw.
pub fn glass_plate(g: &egui::Painter, r: egui::Rect, radius: f32) {
    let radius8 = radius.clamp(0.0, 255.0) as u8;
    g.rect_filled(r, radius8, Color32::from_white_alpha(16));
    let mut top = r;
    top.set_height(r.height() * 0.5);
    let corners = egui::CornerRadius {
        nw: radius8,
        ne: radius8,
        sw: 0,
        se: 0,
    };
    g.rect_filled(top, corners, Color32::from_white_alpha(12));
    g.rect_stroke(
        r,
        radius8,
        Stroke::new(1.0, GLASS_EDGE),
        egui::StrokeKind::Inside,
    );
    let inset = radius + 1.0;
    g.line_segment(
        [
            egui::pos2(r.left() + inset, r.top() + 1.0),
            egui::pos2(r.right() - inset, r.top() + 1.0),
        ],
        Stroke::new(1.0, Color32::from_white_alpha(40)),
    );
}

pub fn apply(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = GLASS_FILL;
    v.extreme_bg_color = GRID_BG;
    v.override_text_color = Some(TEXT);
    v.warn_fg_color = SOLO;
    v.error_fg_color = DANGER;
    v.disabled_alpha = DISABLED_ALPHA;
    v.window_stroke = Stroke::new(1.0, GLASS_EDGE);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v.widgets.noninteractive.bg_fill = PANEL;
    v.widgets.inactive.bg_fill = BUTTON;
    v.widgets.inactive.weak_bg_fill = BUTTON;
    v.widgets.hovered.weak_bg_fill = BUTTON_HOVERED;
    v.widgets.active.weak_bg_fill = ACCENT_BLUE;
    v.selection.bg_fill = ACCENT_BLUE;
    let r = CornerRadius::same(CORNER);
    v.widgets.inactive.corner_radius = r;
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.corner_radius = r;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(ITEM_SPACING, 4.0);
        s.spacing.button_padding = egui::vec2(6.0, 2.0);
        s.text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(TEXT_SIZE));
        s.text_styles.insert(
            egui::TextStyle::Button,
            egui::FontId::proportional(TEXT_SIZE),
        );
    });
}
