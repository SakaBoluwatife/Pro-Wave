//! The look of the program in one place: dark gray chrome. Colours and sizes used by more than one
//! screen are named here, so changing one changes it everywhere.
use egui::{Color32, CornerRadius, Stroke};

// Surfaces, from the back of the window forwards.
pub const BG: Color32 = Color32::from_rgb(0x3a, 0x3a, 0x3c);
pub const LANE: Color32 = Color32::from_rgb(0x26, 0x26, 0x26);
pub const GRID_BG: Color32 = Color32::from_rgb(0x2b, 0x2b, 0x2b);
pub const MIXER_BG: Color32 = Color32::from_rgb(0x33, 0x33, 0x33);
pub const PANEL_BG: Color32 = Color32::from_rgb(0x3a, 0x3a, 0x3a);
pub const TOOLBAR: Color32 = Color32::from_rgb(0x3c, 0x3c, 0x3c);
pub const STRIP: Color32 = Color32::from_rgb(0x3e, 0x3e, 0x3e);
pub const HEADER: Color32 = Color32::from_rgb(0x42, 0x42, 0x42);
pub const PANEL: Color32 = Color32::from_rgb(0x4a, 0x4a, 0x4c);
pub const BORDER: Color32 = Color32::from_gray(0x24);

// Text.
pub const TEXT: Color32 = Color32::from_rgb(0xdc, 0xdc, 0xdc);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x96, 0x96, 0x96);

// Controls and their states.
pub const BUTTON: Color32 = Color32::from_rgb(0x58, 0x58, 0x58);
pub const BUTTON_HOVERED: Color32 = Color32::from_rgb(0x6a, 0x6a, 0x6c);
pub const HOVERED: Color32 = Color32::from_rgb(0x46, 0x46, 0x46);
pub const SELECTED: Color32 = Color32::from_rgb(0x5a, 0x5a, 0x5a);
pub const ACCENT_BLUE: Color32 = Color32::from_rgb(0x2f, 0x6f, 0xd8);
pub const DISABLED_ALPHA: f32 = 0.45;

// Mixer strips.
pub const STRIP_SELECTED: Color32 = Color32::from_rgb(0x4d, 0x4d, 0x4d);
pub const SLOT_EMPTY: Color32 = Color32::from_rgb(0x2c, 0x2c, 0x2c);
pub const SLOT: Color32 = Color32::from_rgb(0x55, 0x55, 0x55);
pub const SLOT_OFF: Color32 = Color32::from_rgb(0x4a, 0x4a, 0x4a);
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

pub fn apply(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.extreme_bg_color = GRID_BG;
    v.override_text_color = Some(TEXT);
    v.warn_fg_color = SOLO;
    v.error_fg_color = DANGER;
    v.disabled_alpha = DISABLED_ALPHA;
    v.window_stroke = Stroke::new(1.0, BORDER);
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
