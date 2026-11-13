//! Themes: named colourways the user can pick from, remix, and pass around as files. A theme says the
//! colour of each step from the deepest surface to the brightest text; every grey in the program goes
//! through those steps ([`crate::theme::gray`]), so one file recolours all of it, dark or light.
use crate::theme;
use egui::{pos2, vec2, Align2, Color32, FontId, Rect, Sense, Stroke, Ui};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The extension of a theme file.
pub const EXTENSION: &str = "ovrtheme";
/// The file naming the theme in use.
const CHOSEN_FILE: &str = "theme";
/// The folder of the user's own themes.
const FOLDER: &str = "themes";

/// One colourway.
#[derive(Clone, PartialEq, Debug)]
pub struct Theme {
    pub name: String,
    /// Comes with the program: it can be remixed, not changed or deleted.
    pub built_in: bool,
    /// The condensed typeface instead of Inter.
    pub condensed: bool,
    /// How round controls are.
    pub corner: u8,
    /// The steps, from the deepest surface to the brightest text.
    pub deep: Color32,
    pub grid: Color32,
    pub surface: Color32,
    pub raised: Color32,
    pub control: Color32,
    pub dim: Color32,
    pub text: Color32,
    pub bright: Color32,
    /// Surfaces with a place of their own.
    pub toolbar: Color32,
    pub panel: Color32,
    pub mixer: Color32,
    pub border: Color32,
    pub accent: Color32,
    /// Knob arcs follow the accent, not the green.
    pub arc_accent: bool,
    /// Degrees, turning every surface and text colour.
    pub hue: f32,
    /// -1..=1 each.
    pub saturation: f32,
    pub brightness: f32,
    pub contrast: f32,
}

const fn c(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// The colours of a theme, in the order files and the editor have them.
const FIELDS: [(&str, &str); 13] = [
    ("deep", "Deepest"),
    ("grid", "Grid"),
    ("surface", "Surfaces"),
    ("raised", "Raised"),
    ("control", "Controls"),
    ("dim", "Dim text"),
    ("text", "Text"),
    ("bright", "Brightest"),
    ("toolbar", "Toolbars"),
    ("panel", "Panels"),
    ("mixer", "Mixer"),
    ("border", "Lines"),
    ("accent", "Accent"),
];

impl Theme {
    fn color(&self, key: &str) -> Option<Color32> {
        Some(match key {
            "deep" => self.deep,
            "grid" => self.grid,
            "surface" => self.surface,
            "raised" => self.raised,
            "control" => self.control,
            "dim" => self.dim,
            "text" => self.text,
            "bright" => self.bright,
            "toolbar" => self.toolbar,
            "panel" => self.panel,
            "mixer" => self.mixer,
            "border" => self.border,
            "accent" => self.accent,
            _ => return None,
        })
    }

    fn color_mut(&mut self, key: &str) -> Option<&mut Color32> {
        Some(match key {
            "deep" => &mut self.deep,
            "grid" => &mut self.grid,
            "surface" => &mut self.surface,
            "raised" => &mut self.raised,
            "control" => &mut self.control,
            "dim" => &mut self.dim,
            "text" => &mut self.text,
            "bright" => &mut self.bright,
            "toolbar" => &mut self.toolbar,
            "panel" => &mut self.panel,
            "mixer" => &mut self.mixer,
            "border" => &mut self.border,
            "accent" => &mut self.accent,
            _ => return None,
        })
    }

    /// Whether this is the program's own look, whose greys are used exactly as written.
    pub fn is_original(&self) -> bool {
        self.built_in
            && self.name == ORIGINAL
            && self.hue == 0.0
            && self.saturation == 0.0
            && self.brightness == 0.0
            && self.contrast == 0.0
    }

    /// Light surfaces with dark text.
    pub fn is_light(&self) -> bool {
        let [r, g, b, _] = self.surface.to_array();
        0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b) > 128.0
    }

    /// The theme as a file.
    pub fn to_text(&self) -> String {
        let mut text = format!(
            "# Ovrtone theme\nname = {}\ncondensed = {}\ncorner = {}\narc_accent = {}\nhue = {}\nsaturation = {}\nbrightness = {}\ncontrast = {}\n",
            self.name,
            self.condensed,
            self.corner,
            self.arc_accent,
            self.hue,
            self.saturation,
            self.brightness,
            self.contrast
        );
        for (key, _) in FIELDS {
            if let Some(color) = self.color(key) {
                let [r, g, b, _] = color.to_array();
                text.push_str(&format!("{key} = #{r:02x}{g:02x}{b:02x}\n"));
            }
        }
        text
    }

    /// A theme from a file. What it leaves out stays as in the program's own look, so that a theme from
    /// a newer version still opens. None if it holds no theme at all.
    pub fn from_text(text: &str) -> Option<Theme> {
        let mut theme = BUILT_IN_ORIGINAL();
        theme.built_in = false;
        theme.name = String::new();
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            match key {
                "name" => theme.name = value.chars().take(60).collect(),
                "condensed" => theme.condensed = value == "true",
                "arc_accent" => theme.arc_accent = value == "true",
                "corner" => theme.corner = value.parse::<u8>().unwrap_or(4).min(16),
                "hue" => theme.hue = number(value, -180.0, 180.0),
                "saturation" => theme.saturation = number(value, -1.0, 1.0),
                "brightness" => theme.brightness = number(value, -1.0, 1.0),
                "contrast" => theme.contrast = number(value, -1.0, 1.0),
                _ => {
                    if let (Some(slot), Some(color)) = (theme.color_mut(key), hex(value)) {
                        *slot = color;
                    }
                }
            }
        }
        (!theme.name.is_empty()).then_some(theme)
    }

    /// A colour of a surface or of text, with the theme's turning and levels on it.
    pub fn adjusted(&self, color: Color32) -> Color32 {
        if self.hue == 0.0 && self.saturation == 0.0 && self.brightness == 0.0 && self.contrast == 0.0
        {
            return color;
        }
        let [r, g, b, _] = color.to_array();
        let (h, s, l) = to_hsl(r, g, b);
        let s = (s * (1.0 + self.saturation)).clamp(0.0, 1.0);
        let contrast = 1.0 + self.contrast;
        let l = (((l - 0.5) * contrast + 0.5) + self.brightness * 0.5).clamp(0.0, 1.0);
        from_hsl(h + self.hue, s, l)
    }
}

fn number(text: &str, low: f32, high: f32) -> f32 {
    text.parse::<f32>()
        .ok()
        .filter(|v| v.is_finite())
        .map_or(0.0, |v| v.clamp(low, high))
}

fn hex(text: &str) -> Option<Color32> {
    let digits = text.strip_prefix('#')?;
    if digits.len() != 6 {
        return None;
    }
    u32::from_str_radix(digits, 16).ok().map(c)
}

fn to_hsl(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (r, g, b) = (f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0);
    let (high, low) = (r.max(g).max(b), r.min(g).min(b));
    let l = (high + low) / 2.0;
    if high == low {
        return (0.0, 0.0, l);
    }
    let d = high - low;
    let s = if l > 0.5 { d / (2.0 - high - low) } else { d / (high + low) };
    let h = if high == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if high == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s, l)
}

fn from_hsl(h: f32, s: f32, l: f32) -> Color32 {
    let chroma = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let part = h.rem_euclid(360.0) / 60.0;
    let x = chroma * (1.0 - (part % 2.0 - 1.0).abs());
    let (r, g, b) = match part as u32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let m = l - chroma / 2.0;
    let q = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(q(r), q(g), q(b))
}

/// The name of the program's own look.
pub const ORIGINAL: &str = "Ovrtone";

#[allow(clippy::too_many_arguments)]
fn way(
    name: &str,
    condensed: bool,
    corner: u8,
    steps: [u32; 8],
    places: [u32; 4],
    accent: u32,
    arc_accent: bool,
) -> Theme {
    Theme {
        name: name.to_string(),
        built_in: true,
        condensed,
        corner,
        deep: c(steps[0]),
        grid: c(steps[1]),
        surface: c(steps[2]),
        raised: c(steps[3]),
        control: c(steps[4]),
        dim: c(steps[5]),
        text: c(steps[6]),
        bright: c(steps[7]),
        toolbar: c(places[0]),
        panel: c(places[1]),
        mixer: c(places[2]),
        border: c(places[3]),
        accent: c(accent),
        arc_accent,
        hue: 0.0,
        saturation: 0.0,
        brightness: 0.0,
        contrast: 0.0,
    }
}

#[allow(non_snake_case)]
fn BUILT_IN_ORIGINAL() -> Theme {
    way(
        ORIGINAL,
        false,
        4,
        [0x000000, 0x121316, 0x202227, 0x2b2e33, 0x3a3d43, 0x8f9299, 0xe6e7ea, 0xffffff],
        [0x1c1e22, 0x1b1d21, 0x17191d, 0x2a2c31],
        0x1677ff,
        false,
    )
}

/// The themes that come with the program.
pub fn built_in() -> Vec<Theme> {
    vec![
        BUILT_IN_ORIGINAL(),
        way("Amber Charcoal", false, 4,
            [0x000000, 0x121316, 0x202227, 0x2b2e33, 0x3a3d43, 0x8f9299, 0xe6e7ea, 0xffffff],
            [0x1c1e22, 0x1b1d21, 0x17191d, 0x2a2c31], 0xf09e32, true),
        way("Studio", true, 6,
            [0x101010, 0x282828, 0x2e2e2e, 0x383838, 0x4a4a4a, 0xa3a3a3, 0xececec, 0xffffff],
            [0x3b3b3b, 0x2e2e2e, 0x262626, 0x1c1c1c], 0xffdd55, true),
        way("Midnight Lemon", false, 4,
            [0x000000, 0x08090c, 0x101116, 0x191a20, 0x26282f, 0x777b85, 0xdedfe3, 0xffffff],
            [0x0c0d10, 0x0e0f13, 0x0a0b0e, 0x1c1d22], 0xffd21f, true),
        way("Mono", false, 2,
            [0x000000, 0x050505, 0x0c0c0c, 0x161616, 0x262626, 0x8a8a8a, 0xf2f2f2, 0xffffff],
            [0x0a0a0a, 0x080808, 0x060606, 0x2c2c2c], 0xffffff, true),
        way("Violet Hour", false, 4,
            [0x05040c, 0x0e0c1c, 0x17152a, 0x211e38, 0x302c4c, 0x8a86a8, 0xe4e2f2, 0xffffff],
            [0x12101f, 0x141224, 0x0f0d1a, 0x2a2740], 0x9b4dff, true),
        way("Neon Grid", true, 3,
            [0x04060c, 0x0b0f1a, 0x131826, 0x1c2335, 0x2a3350, 0x7c88a8, 0xd6e4f0, 0xffffff],
            [0x0e1220, 0x101524, 0x0c101a, 0x263049], 0xd61fa0, true),
        way("Terminal", false, 2,
            [0x020502, 0x0a120a, 0x121c12, 0x1a281a, 0x263a26, 0x6f9a6f, 0xc8f0c8, 0xeaffea],
            [0x0e170e, 0x101a10, 0x0c130c, 0x233423], 0x46c21a, true),
        way("Ember", false, 4,
            [0x070303, 0x130b0a, 0x1e1412, 0x2a1c19, 0x3c2925, 0x9c8680, 0xefe3df, 0xffffff],
            [0x18100e, 0x1a1210, 0x140d0b, 0x33221e], 0xd8322a, true),
        way("Parchment", false, 8,
            [0xf8e6b8, 0xfdeec6, 0xfde8bb, 0xf2d79f, 0xe6bf80, 0x8a6f46, 0x241a0c, 0x000000],
            [0xe6bf80, 0xfde8bb, 0xf6dcab, 0x8a6f46], 0xff5a0f, true),
        way("Daylight", false, 6,
            [0xffffff, 0xf0f0f0, 0xe4e4e4, 0xd6d6d6, 0xc2c2c2, 0x70747a, 0x1c1e22, 0x000000],
            [0xdcdcdc, 0xe6e6e6, 0xd8d8d8, 0xa8a8a8], 0xff9d0a, true),
        way("Sky Salmon", false, 8,
            [0xf4f9fd, 0xdfeaf3, 0xc4d9e8, 0xa9c6dc, 0x8fb2cd, 0x4e6a80, 0x10283a, 0x000000],
            [0x9bbdd8, 0xc4d9e8, 0xb2cde0, 0x6f8fa8], 0xe2705f, true),
        way("Bubblegum", false, 8,
            [0xffeef7, 0xffd9ec, 0xf8a8cf, 0xf08fbf, 0xdd6fa6, 0x7a2c55, 0x2b0a1c, 0x000000],
            [0xe86aa6, 0xf8a8cf, 0xf29cc6, 0xa9427a], 0xb9205f, true),
    ]
}

/// The themes at hand: the built-in ones, then the user's.
static THEMES: Mutex<Vec<Theme>> = Mutex::new(Vec::new());

fn folder() -> Option<PathBuf> {
    crate::recent_files::config_dir().map(|d| d.join(FOLDER))
}

fn safe_file_name(name: &str) -> String {
    let safe: String = name
        .chars()
        .map(|ch| if ch.is_alphanumeric() || ch == ' ' || ch == '-' { ch } else { '_' })
        .collect();
    format!("{}.{EXTENSION}", safe.trim())
}

fn reload() -> Vec<Theme> {
    let mut all = built_in();
    if let Some(entries) = folder().and_then(|d| std::fs::read_dir(d).ok()) {
        let mut own: Vec<Theme> = entries
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|x| x == EXTENSION))
            .filter_map(|e| std::fs::read_to_string(e.path()).ok())
            .filter_map(|text| Theme::from_text(&text))
            .collect();
        own.sort_by_key(|t| t.name.to_lowercase());
        all.extend(own);
    }
    if let Ok(mut themes) = THEMES.lock() {
        themes.clone_from(&all);
    }
    all
}

fn all() -> Vec<Theme> {
    let held = THEMES.lock().map(|t| t.clone()).unwrap_or_default();
    if held.is_empty() {
        reload()
    } else {
        held
    }
}

fn chosen_file() -> Option<PathBuf> {
    crate::recent_files::config_dir().map(|d| d.join(CHOSEN_FILE))
}

/// Puts the theme chosen last in force. Call before the fonts and [`theme::apply`].
pub fn load_chosen() {
    let name = chosen_file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .map(|t| t.trim().to_string());
    let themes = all();
    let picked = name
        .and_then(|n| themes.iter().find(|t| t.name == n).cloned())
        .unwrap_or_else(BUILT_IN_ORIGINAL);
    theme::use_theme(&picked);
}

/// Changes the theme everywhere at once, and remembers it.
pub fn choose(ctx: &egui::Context, picked: &Theme) {
    show(ctx, picked);
    // What was picked by hand before is over: the accent file is the theme's now.
    theme::set_accent(ctx, picked.accent);
    if let Some(file) = chosen_file() {
        let saved = file
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&file, &picked.name));
        if let Err(e) = saved {
            tracing::warn!("could not save the theme: {e}");
        }
    }
}

/// Puts a theme on the screen without remembering it, as the editor does while colours change.
fn show(ctx: &egui::Context, picked: &Theme) {
    let fonts_change = theme::chosen_face().is_none() && theme::condensed() != picked.condensed;
    theme::use_theme(picked);
    if fonts_change {
        theme::install_fonts(ctx);
    }
    theme::apply(ctx);
}

fn save_own(picked: &Theme) -> Result<(), String> {
    let dir = folder().ok_or_else(|| "no settings folder".to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(safe_file_name(&picked.name)), picked.to_text())
        .map_err(|e| e.to_string())
}

/// A name no theme at hand has.
fn free_name(base: &str) -> String {
    let themes = all();
    let taken = |n: &str| themes.iter().any(|t| t.name == n);
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|k| format!("{base} {k}"))
        .find(|n| !taken(n))
        .unwrap_or_else(|| base.to_string())
}

/// Takes a theme file in, and returns its theme.
pub fn import(path: &Path) -> Result<Theme, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut picked = Theme::from_text(&text).ok_or_else(|| "this is not a theme".to_string())?;
    picked.name = free_name(&picked.name);
    save_own(&picked)?;
    reload();
    Ok(picked)
}

/// What the window keeps between frames.
#[derive(Clone, Default)]
struct View {
    editing: Option<Theme>,
    search: String,
    message: Option<String>,
}

/// A card: a small picture of the theme, and its name.
fn card(ui: &mut Ui, t: &Theme, on: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(150.0, 92.0), Sense::click());
    let g = ui.painter();
    let plate = Rect::from_min_size(r.min, vec2(r.width(), 66.0));
    g.rect_filled(plate, 6.0, t.surface);
    let bar = |x: f32, y: f32, w: f32, color: Color32| {
        g.rect_filled(
            Rect::from_min_size(plate.min + vec2(x, y), vec2(w, 9.0)),
            3.0,
            color,
        );
    };
    bar(8.0, 8.0, 54.0, t.accent);
    bar(8.0, 21.0, 44.0, t.control);
    bar(8.0, 34.0, 54.0, t.raised);
    bar(8.0, 47.0, 54.0, t.toolbar);
    g.line_segment(
        [plate.min + vec2(68.0, 6.0), plate.min + vec2(68.0, 60.0)],
        Stroke::new(1.0, t.border),
    );
    bar(76.0, 8.0, 30.0, t.grid);
    bar(110.0, 8.0, 32.0, t.accent);
    bar(76.0, 21.0, 66.0, t.mixer);
    bar(76.0, 34.0, 20.0, t.text);
    bar(100.0, 34.0, 42.0, t.dim);
    bar(76.0, 47.0, 66.0, t.panel);
    let edge = if on {
        Stroke::new(2.0, theme::accent())
    } else if resp.hovered() {
        Stroke::new(1.0, theme::text_dim())
    } else {
        Stroke::new(1.0, theme::border())
    };
    g.rect_stroke(plate, 6.0, edge, egui::StrokeKind::Middle);
    g.text(
        pos2(r.left() + 2.0, plate.bottom() + 13.0),
        Align2::LEFT_CENTER,
        &t.name,
        FontId::proportional(12.5),
        theme::text(),
    );
    resp
}

/// The themes section of the settings: the cards, and the editor when one is being remixed.
pub fn section(ui: &mut Ui) {
    let key = egui::Id::unique("themes_view");
    let mut view: View = ui.data(|d| d.get_temp(key)).unwrap_or_default();
    let ctx = ui.ctx().clone();
    if let Some(editing) = &mut view.editing {
        editor(ui, editing);
    }
    let mut stop_editing = false;
    if view.editing.is_some() {
        ui.horizontal(|ui| {
            if ui.button("Done").clicked() {
                stop_editing = true;
            }
        });
    }
    if stop_editing {
        if let Some(done) = view.editing.take() {
            match save_own(&done) {
                Ok(()) => {
                    reload();
                    choose(&ctx, &done);
                }
                Err(e) => view.message = Some(e),
            }
        }
    }
    let current = theme::theme_name();
    let themes = all();
    let active = themes.iter().find(|t| t.name == current).cloned();
    ui.horizontal(|ui| {
        ui.label("Theme");
        ui.add(
            egui::TextEdit::singleline(&mut view.search)
                .hint_text("Search themes")
                .desired_width(150.0),
        );
        if crate::icons::button(ui, crate::icons::Icon::Copy, "Remix: make your own copy to change")
            .clicked()
        {
            if let Some(base) = &active {
                let mut copy = base.clone();
                copy.built_in = false;
                copy.name = free_name(&format!("{} Remix", base.name));
                view.editing = Some(copy);
            }
        }
        if ui.button("Import…").clicked() {
            let file = rfd::FileDialog::new()
                .add_filter("Ovrtone theme", &[EXTENSION])
                .pick_file();
            if let Some(file) = file {
                match import(&file) {
                    Ok(t) => choose(&ctx, &t),
                    Err(e) => view.message = Some(e),
                }
            }
        }
        if ui.button("Export…").clicked() {
            if let Some(base) = &active {
                let file = rfd::FileDialog::new()
                    .set_file_name(safe_file_name(&base.name))
                    .add_filter("Ovrtone theme", &[EXTENSION])
                    .save_file();
                if let Some(file) = file {
                    if let Err(e) = std::fs::write(file, base.to_text()) {
                        view.message = Some(e.to_string());
                    }
                }
            }
        }
        let own = active.as_ref().is_some_and(|t| !t.built_in);
        if own
            && crate::icons::button(ui, crate::icons::Icon::Trash, "Delete this theme").clicked()
        {
            if let Some(file) = folder().map(|d| d.join(safe_file_name(&current))) {
                if let Err(e) = std::fs::remove_file(file) {
                    view.message = Some(e.to_string());
                }
            }
            reload();
            choose(&ctx, &BUILT_IN_ORIGINAL());
        }
    });
    if let Some(message) = &view.message {
        ui.colored_label(ui.visuals().error_fg_color, message);
    }
    let needle = view.search.to_lowercase();
    ui.horizontal_wrapped(|ui| {
        for t in themes.iter().filter(|t| t.name.to_lowercase().contains(&needle)) {
            if card(ui, t, t.name == current).clicked() {
                view.editing = None;
                choose(&ctx, t);
            }
        }
    });
    ui.data_mut(|d| d.insert_temp(key, view));
}

/// The colours of a remix and its levels. What is changed shows at once.
fn editor(ui: &mut Ui, t: &mut Theme) {
    let before = t.clone();
    ui.group(|ui| {
        ui.horizontal(|ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut t.name);
        });
        egui::Grid::new(egui::Id::unique("theme_colors"))
            .num_columns(4)
            .show(ui, |ui| {
                for (k, (key, label)) in FIELDS.iter().enumerate() {
                    if let Some(color) = t.color_mut(key) {
                        ui.color_edit_button_srgba(color);
                        ui.label(*label);
                    }
                    if k % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
        ui.add(egui::Slider::new(&mut t.hue, -180.0..=180.0).text("Hue"));
        ui.add(egui::Slider::new(&mut t.saturation, -1.0..=1.0).text("Saturation"));
        ui.add(egui::Slider::new(&mut t.brightness, -1.0..=1.0).text("Brightness"));
        ui.add(egui::Slider::new(&mut t.contrast, -1.0..=1.0).text("Contrast"));
        ui.horizontal(|ui| {
            ui.add(egui::Slider::new(&mut t.corner, 0..=12).text("Corners"));
            ui.checkbox(&mut t.condensed, "Condensed type")
                .on_hover_text("Used unless a typeface is chosen under Type");
            ui.checkbox(&mut t.arc_accent, "Accent on knobs");
        });
    });
    if *t != before && !t.name.trim().is_empty() {
        show(ui.ctx(), t);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_theme_survives_the_trip_through_a_file() {
        for mut t in built_in() {
            t.built_in = false;
            t.hue = 12.5;
            t.contrast = -0.25;
            assert_eq!(Theme::from_text(&t.to_text()).unwrap(), t);
        }
    }

    #[test]
    fn a_bad_file_is_no_theme_and_bad_lines_are_left_out() {
        assert_eq!(Theme::from_text("nonsense"), None);
        assert_eq!(Theme::from_text(""), None);
        let t = Theme::from_text("name = X\ntext = red\naccent = #ff0000\nhue = 900\n").unwrap();
        assert_eq!(t.text, BUILT_IN_ORIGINAL().text);
        assert_eq!(t.accent, Color32::from_rgb(0xff, 0, 0));
        assert_eq!(t.hue, 180.0);
    }

    #[test]
    fn names_are_not_taken_twice_and_only_the_plain_original_is_exact() {
        let themes = built_in();
        for (k, t) in themes.iter().enumerate() {
            assert!(!themes[k + 1..].iter().any(|o| o.name == t.name), "{}", t.name);
        }
        assert!(themes[0].is_original());
        assert!(!themes[1].is_original());
        let mut turned = themes[0].clone();
        turned.hue = 10.0;
        assert!(!turned.is_original());
        assert!(!themes[0].is_light());
        assert!(themes.iter().any(Theme::is_light));
    }

    #[test]
    fn levels_leave_a_colour_alone_until_asked() {
        let t = BUILT_IN_ORIGINAL();
        assert_eq!(t.adjusted(t.text), t.text);
        let mut lighter = t.clone();
        lighter.brightness = 0.5;
        let [r, ..] = lighter.adjusted(t.surface).to_array();
        assert!(r > t.surface.r());
    }
}
