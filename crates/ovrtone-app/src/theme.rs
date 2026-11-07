//! The look of the program in one place: a dark charcoal base with one accent colour, which the
//! user chooses. Colours and sizes used by more than one
//! screen are named here, so changing one changes it everywhere.
use egui::{Color32, CornerRadius, Stroke};
use std::sync::atomic::{AtomicU32, Ordering};

/// The accents to choose from, by name. The first is the one a new install starts with.
pub const ACCENTS: [(&str, Color32); 2] = [
    ("Ovrtone Blue", Color32::from_rgb(0x16, 0x77, 0xff)),
    ("Ovrtone Amber", Color32::from_rgb(0xf0, 0x9e, 0x32)),
];

/// The accent in use, as red, green and blue in one number.
static ACCENT: AtomicU32 = AtomicU32::new(0x16_77_ff);

/// The accent colour: what is selected, on, or chosen, everywhere.
pub fn accent() -> Color32 {
    let v = ACCENT.load(Ordering::Relaxed);
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// What reads on the accent: dark on a bright one such as amber, white on a deep one such as blue.
pub fn on_accent() -> Color32 {
    let [r, g, b, _] = accent().to_array();
    let light = 0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b);
    if light > 140.0 {
        Color32::from_rgb(0x17, 0x19, 0x1d)
    } else {
        Color32::WHITE
    }
}

/// How round a panel's corners are.
const PANEL_RADIUS: f32 = 8.0;

/// Makes a panel look like a rounded card on the window's background: paints the gap around it and a
/// thin edge. Call after the panel is drawn, so that this goes over what is in its corners.
pub fn panel_edge(g: &egui::Painter, r: egui::Rect) {
    let gap = 5.0;
    g.rect_stroke(
        r.expand(gap / 2.0 - 1.0),
        PANEL_RADIUS + gap / 2.0,
        Stroke::new(gap, BG),
        egui::StrokeKind::Middle,
    );
    g.rect_stroke(
        r.shrink(1.0),
        PANEL_RADIUS,
        Stroke::new(1.0, BORDER),
        egui::StrokeKind::Middle,
    );
}

fn store_accent(c: Color32) {
    let [r, g, b, _] = c.to_array();
    ACCENT.store(u32::from_be_bytes([0, r, g, b]), Ordering::Relaxed);
}

/// The accent as it is kept in the settings folder: six hex digits.
fn accent_from_text(text: &str) -> Option<Color32> {
    let text = text.trim();
    (text.len() == 6)
        .then(|| u32::from_str_radix(text, 16).ok())?
        .map(|v| Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

fn accent_file() -> Option<std::path::PathBuf> {
    crate::recent_files::config_dir().map(|d| d.join("accent"))
}

/// Takes the accent the user chose last. Call before [`apply`].
pub fn load_accent() {
    if let Some(c) = accent_file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .as_deref()
        .and_then(accent_from_text)
    {
        store_accent(c);
    }
}

/// Changes the accent everywhere at once, and remembers it.
pub fn set_accent(ctx: &egui::Context, c: Color32) {
    store_accent(c);
    apply(ctx);
    let [r, g, b, _] = c.to_array();
    if let Some(file) = accent_file() {
        let saved = file
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&file, format!("{r:02x}{g:02x}{b:02x}")));
        if let Err(e) = saved {
            tracing::warn!("could not save the accent: {e}");
        }
    }
}

// Surfaces, from the back of the window forwards.
pub const BG: Color32 = Color32::from_rgb(0x16, 0x18, 0x1c);
pub const LANE: Color32 = Color32::from_rgb(0x15, 0x17, 0x1a);
pub const GRID_BG: Color32 = Color32::from_rgb(0x12, 0x13, 0x16);
pub const MIXER_BG: Color32 = Color32::from_rgb(0x17, 0x19, 0x1d);
pub const PANEL_BG: Color32 = Color32::from_rgb(0x1b, 0x1d, 0x21);
pub const TOOLBAR: Color32 = Color32::from_rgb(0x1c, 0x1e, 0x22);
pub const STRIP: Color32 = Color32::from_rgb(0x20, 0x22, 0x27);
pub const HEADER: Color32 = Color32::from_rgb(0x26, 0x28, 0x2d);
pub const PANEL: Color32 = Color32::from_rgb(0x22, 0x24, 0x29);
pub const BORDER: Color32 = Color32::from_rgb(0x2a, 0x2c, 0x31);

// Text.
pub const TEXT: Color32 = Color32::from_rgb(0xe6, 0xe7, 0xea);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x8f, 0x92, 0x99);

// Controls and their states.
pub const BUTTON: Color32 = Color32::from_rgb(0x32, 0x35, 0x3b);
pub const BUTTON_HOVERED: Color32 = Color32::from_rgb(0x3e, 0x41, 0x48);
pub const HOVERED: Color32 = Color32::from_rgb(0x2b, 0x2e, 0x33);
pub const SELECTED: Color32 = Color32::from_rgb(0x33, 0x37, 0x3e);
pub const DISABLED_ALPHA: f32 = 0.45;

// Mixer strips.
pub const STRIP_SELECTED: Color32 = Color32::from_rgb(0x2a, 0x2d, 0x33);
pub const SLOT_EMPTY: Color32 = Color32::from_rgb(0x1c, 0x1e, 0x22);
pub const SLOT: Color32 = Color32::from_rgb(0x30, 0x33, 0x39);
pub const SLOT_OFF: Color32 = Color32::from_rgb(0x26, 0x28, 0x2d);
pub const METER_OFF: Color32 = Color32::from_rgb(0x3a, 0x3d, 0x43);

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
    (0x24 * GLASS_ALPHA as u16 / 255) as u8,
    (0x26 * GLASS_ALPHA as u16 / 255) as u8,
    (0x2b * GLASS_ALPHA as u16 / 255) as u8,
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

/// Whether the fonts below are in place. Tests draw without them.
static FONTS_READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// The family for numbers that change as they play.
const DIGITS: &str = "digits";

/// Inter for all text, with egui's own fonts behind it for what Inter does not cover. Inter's
/// digits differ in width and egui cannot ask for its even ones, so the numbers that run in the
/// display keep the old font: see [`digit_font`]. Inter is under the SIL Open Font License, which is
/// beside the file.
pub fn install_fonts(ctx: &egui::Context) {
    use egui::{FontData, FontDefinitions, FontFamily, FontTweak};
    let mut fonts = FontDefinitions::default();
    let inter = FontData::from_static(include_bytes!("../assets/fonts/Inter-Regular.ttf")).tweak(
        FontTweak {
            // Inter is drawn larger than the size it is asked for, next to the old font.
            scale: 0.93,
            ..FontTweak::default()
        },
    );
    fonts.font_data.insert("Inter".to_owned(), inter.into());
    let old = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    fonts.families.insert(FontFamily::Name(DIGITS.into()), old);
    if let Some(all) = fonts.families.get_mut(&FontFamily::Proportional) {
        all.insert(0, "Inter".to_owned());
    }
    if let Some(all) = fonts.families.get_mut(&FontFamily::Monospace) {
        all.push("Inter".to_owned());
    }
    ctx.set_fonts(fonts);
    FONTS_READY.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// A font whose digits are all one width, so that a running number stands still.
pub fn digit_font(size: f32) -> egui::FontId {
    if FONTS_READY.load(std::sync::atomic::Ordering::Relaxed) {
        egui::FontId::new(size, egui::FontFamily::Name(DIGITS.into()))
    } else {
        egui::FontId::proportional(size)
    }
}

pub fn apply(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    // Letters keep the shapes they were drawn with, not bent to the pixel grid: smoother, and evenly
    // spaced at any display scale.
    v.text_options.font_hinting = false;
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
    v.widgets.active.weak_bg_fill = accent();
    v.selection.bg_fill = accent();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_accent_is_kept_as_text_and_a_bad_file_is_ignored() {
        assert_eq!(accent_from_text("16 77 ff"), None, "not six hex digits");
        assert_eq!(
            accent_from_text("1677ff\n"),
            Some(Color32::from_rgb(0x16, 0x77, 0xff))
        );
        assert_eq!(accent_from_text("nonsense"), None);
        assert_eq!(accent_from_text(""), None);
    }
}
