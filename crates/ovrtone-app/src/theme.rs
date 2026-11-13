//! The look of the program in one place: a dark charcoal base with one accent colour, which the
//! user chooses. Colours and sizes used by more than one
//! screen are named here, so changing one changes it everywhere.
use egui::{Color32, CornerRadius, Stroke};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// The accents to choose from, by name. The first is the one a new install starts with.
pub const ACCENTS: [(&str, Color32); 3] = [
    ("Ovrtone Blue", Color32::from_rgb(0x16, 0x77, 0xff)),
    ("Ovrtone Amber", Color32::from_rgb(0xf0, 0x9e, 0x32)),
    ("Studio Yellow", Color32::from_rgb(0xff, 0xdd, 0x55)),
];

/// The theme in force. While `TABLE_ON` is off the program's greys are used exactly as written, which
/// is the original look; a read of an atomic is all a colour costs.
static TABLE_ON: AtomicBool = AtomicBool::new(false);
/// What each grey level is in the theme, as red, green and blue in one number.
static TABLE: [AtomicU32; 256] = [const { AtomicU32::new(0) }; 256];
/// Toolbars, panels, the mixer and lines, which have colours of their own in a theme.
static PLACES: [AtomicU32; 4] = [const { AtomicU32::new(0) }; 4];
static CONDENSED: AtomicBool = AtomicBool::new(false);
static ARC_ACCENT: AtomicBool = AtomicBool::new(false);
static LIGHT: AtomicBool = AtomicBool::new(false);
static CORNERS: AtomicU32 = AtomicU32::new(CORNER as u32);
static THEME_NAME: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// The levels the steps of a theme stand at, and which they are: between them, colours are mixed.
const STEP_LEVELS: [u8; 8] = [0x00, 0x12, 0x1f, 0x2b, 0x3a, 0x90, 0xe6, 0xff];

fn number_of(c: Color32) -> u32 {
    let [r, g, b, _] = c.to_array();
    u32::from_be_bytes([0, r, g, b])
}

fn color_of(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// What grey level `v` is, given the colour of each step.
fn stepped(steps: &[Color32; 8], v: u8) -> Color32 {
    let at = STEP_LEVELS.iter().rposition(|l| *l <= v).unwrap_or(0);
    let Some(next) = STEP_LEVELS.get(at + 1) else {
        return steps[at];
    };
    let t = f32::from(v - STEP_LEVELS[at]) / f32::from(next - STEP_LEVELS[at]);
    let (a, b) = (steps[at].to_array(), steps[at + 1].to_array());
    let mix = |k: usize| (f32::from(a[k]) + (f32::from(b[k]) - f32::from(a[k])) * t).round() as u8;
    Color32::from_rgb(mix(0), mix(1), mix(2))
}

/// Puts a theme in force. Call [`install_fonts`] if its typeface differs, then [`apply`].
pub fn use_theme(t: &crate::themes::Theme) {
    let steps = [
        t.deep, t.grid, t.surface, t.raised, t.control, t.dim, t.text, t.bright,
    ]
    .map(|c| t.adjusted(c));
    for (v, slot) in TABLE.iter().enumerate() {
        slot.store(number_of(stepped(&steps, v as u8)), Ordering::Relaxed);
    }
    for (slot, c) in PLACES.iter().zip([t.toolbar, t.panel, t.mixer, t.border]) {
        slot.store(number_of(t.adjusted(c)), Ordering::Relaxed);
    }
    TABLE_ON.store(!t.is_original(), Ordering::Relaxed);
    CONDENSED.store(t.condensed, Ordering::Relaxed);
    ARC_ACCENT.store(t.arc_accent, Ordering::Relaxed);
    LIGHT.store(t.is_light(), Ordering::Relaxed);
    CORNERS.store(u32::from(t.corner), Ordering::Relaxed);
    store_accent(t.accent);
    if let Ok(mut name) = THEME_NAME.lock() {
        name.clone_from(&t.name);
    }
}

/// The name of the theme in force.
pub fn theme_name() -> String {
    THEME_NAME.lock().map(|n| n.clone()).unwrap_or_default()
}

/// Whether the theme asks for the condensed typeface.
pub fn condensed() -> bool {
    CONDENSED.load(Ordering::Relaxed)
}

/// Whether knob arcs take the accent.
pub fn arc_accent() -> bool {
    ARC_ACCENT.load(Ordering::Relaxed)
}

/// Whether anything but the original look is in force.
pub fn themed() -> bool {
    TABLE_ON.load(Ordering::Relaxed)
}

/// A grey, in the theme in force.
pub fn gray(v: u8) -> Color32 {
    if themed() {
        color_of(TABLE[usize::from(v)].load(Ordering::Relaxed))
    } else {
        Color32::from_gray(v)
    }
}

/// A colour that is as good as grey, in the theme in force: exactly itself in the original look.
pub fn grey_rgb(r: u8, g: u8, b: u8) -> Color32 {
    if themed() {
        let level = (u16::from(r) + u16::from(g) + u16::from(b)) / 3;
        color_of(TABLE[usize::from(level)].load(Ordering::Relaxed))
    } else {
        Color32::from_rgb(r, g, b)
    }
}

macro_rules! greys {
    ($($(#[$doc:meta])* $name:ident = ($r:literal, $g:literal, $b:literal);)+) => {
        $($(#[$doc])* pub fn $name() -> Color32 {
            grey_rgb($r, $g, $b)
        })+
    };
}

greys! {
    // Surfaces, from the back of the window forwards.
    bg = (0x16, 0x18, 0x1c);
    lane = (0x15, 0x17, 0x1a);
    grid_bg = (0x12, 0x13, 0x16);
    strip = (0x20, 0x22, 0x27);
    header = (0x26, 0x28, 0x2d);
    panel = (0x22, 0x24, 0x29);
    // Text.
    text = (0xe6, 0xe7, 0xea);
    text_dim = (0x8f, 0x92, 0x99);
    // Controls and their states.
    button = (0x32, 0x35, 0x3b);
    button_hovered = (0x3e, 0x41, 0x48);
    hovered = (0x2b, 0x2e, 0x33);
    selected = (0x33, 0x37, 0x3e);
    // Mixer strips.
    strip_selected = (0x2a, 0x2d, 0x33);
    slot_empty = (0x1c, 0x1e, 0x22);
    slot = (0x30, 0x33, 0x39);
    slot_off = (0x26, 0x28, 0x2d);
    meter_off = (0x3a, 0x3d, 0x43);
}

/// A line or mark drawn on `bg`: `share` of the way to the text colour, so it shows the same in
/// a light theme as in a dark one.
pub fn ink_on(bg: Color32, share: f32) -> Color32 {
    blend(bg, text(), share)
}

/// `t` of the way from `a` to `b`.
fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// The bar lines over the track lanes: a share of the text colour over the lanes, so they stand out
/// the same in a light theme as in a dark one.
pub fn grid_bar() -> Color32 {
    blend(lane(), text(), 0.20)
}

/// The beat lines between the bar lines, fainter.
pub fn grid_beat() -> Color32 {
    blend(lane(), text(), 0.09)
}

/// A surface with a colour of its own in a theme.
fn surface(original: (u8, u8, u8), place: usize) -> Color32 {
    if themed() {
        color_of(PLACES[place].load(Ordering::Relaxed))
    } else {
        Color32::from_rgb(original.0, original.1, original.2)
    }
}

pub fn mixer_bg() -> Color32 {
    surface((0x17, 0x19, 0x1d), 2)
}

pub fn panel_bg() -> Color32 {
    surface((0x1b, 0x1d, 0x21), 1)
}

pub fn toolbar() -> Color32 {
    surface((0x1c, 0x1e, 0x22), 0)
}

/// Lines between things.
pub fn border() -> Color32 {
    surface((0x2a, 0x2c, 0x31), 3)
}

/// The fill of floating windows and menus.
pub fn glass_fill() -> Color32 {
    let [r, g, b, _] = grey_rgb(0x24, 0x26, 0x2b).to_array();
    Color32::from_rgba_unmultiplied(r, g, b, GLASS_ALPHA)
}

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
        Stroke::new(gap, bg()),
        egui::StrokeKind::Middle,
    );
    g.rect_stroke(
        r.shrink(1.0),
        PANEL_RADIUS,
        Stroke::new(1.0, border()),
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


// Controls and their states.
pub const DISABLED_ALPHA: f32 = 0.45;

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

/// A typeface to choose from: its name, its file, and what makes it sit as Inter does. Each is under
/// the SIL Open Font License, which is beside its file.
pub struct Face {
    pub name: &'static str,
    bytes: &'static [u8],
    /// Brings its small letters to the height of Inter's.
    scale: f32,
    /// Moves its capitals to the middle of the line box, as a share of the size.
    y_offset: f32,
    /// The weight of a variable face, as it has no regular of its own.
    weight: Option<f32>,
}

/// The typefaces to choose from. Each scale and offset comes from the face's own metrics: its x-height
/// against Inter's, and the middle of its capitals against the middle of its line.
pub const FACES: [Face; 7] = [
    Face {
        name: "Inter",
        bytes: include_bytes!("../assets/fonts/Inter-Regular.ttf"),
        // Inter is drawn larger than the size it is asked for, next to the old font.
        scale: 0.93,
        y_offset: 0.0,
        weight: None,
    },
    Face {
        name: "Cuprum (condensed)",
        bytes: include_bytes!("../assets/fonts/Cuprum.ttf"),
        // Its capitals fill the top of its line box: 0.35 of the size from the top against 0.45.
        scale: 1.08,
        y_offset: 0.11,
        weight: None,
    },
    Face {
        name: "Barlow",
        bytes: include_bytes!("../assets/fonts/Barlow-Regular.ttf"),
        scale: 1.0,
        y_offset: -0.05,
        weight: None,
    },
    Face {
        name: "Lato",
        bytes: include_bytes!("../assets/fonts/Lato-Regular.ttf"),
        scale: 1.0,
        y_offset: -0.03,
        weight: None,
    },
    Face {
        name: "IBM Plex Sans",
        bytes: include_bytes!("../assets/fonts/IBMPlexSans-Variable.ttf"),
        scale: 0.98,
        y_offset: -0.03,
        weight: Some(400.0),
    },
    Face {
        name: "Source Sans 3",
        bytes: include_bytes!("../assets/fonts/SourceSans3-Variable.ttf"),
        scale: 1.06,
        y_offset: 0.02,
        weight: Some(400.0),
    },
    Face {
        name: "Atkinson Hyperlegible",
        bytes: include_bytes!("../assets/fonts/AtkinsonHyperlegible-Regular.ttf"),
        scale: 1.02,
        y_offset: 0.0,
        weight: None,
    },
];

/// The face chosen in the settings, or none to have the theme's.
static FACE: AtomicU32 = AtomicU32::new(NO_FACE);
const NO_FACE: u32 = u32::MAX;
/// The size of text, in percent.
static TEXT_PERCENT: AtomicU32 = AtomicU32::new(100);
/// How far text may grow or shrink: past this it would not fit the boxes it is in.
pub const TEXT_PERCENT_RANGE: std::ops::RangeInclusive<u32> = 90..=115;

/// The face chosen in the settings, or none for the theme's.
pub fn chosen_face() -> Option<usize> {
    let at = FACE.load(Ordering::Relaxed);
    (at != NO_FACE && (at as usize) < FACES.len()).then_some(at as usize)
}

pub fn text_percent() -> u32 {
    TEXT_PERCENT.load(Ordering::Relaxed)
}

fn type_file() -> Option<std::path::PathBuf> {
    crate::recent_files::config_dir().map(|d| d.join("type"))
}

/// Takes the face and size chosen last. Call before [`install_fonts`].
pub fn load_type() {
    let text = type_file().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("face = ") {
            if let Some(at) = FACES.iter().position(|f| f.name == name.trim()) {
                FACE.store(at as u32, Ordering::Relaxed);
            }
        } else if let Some(size) = line.strip_prefix("size = ") {
            if let Ok(size) = size.trim().parse::<u32>() {
                TEXT_PERCENT.store(
                    size.clamp(*TEXT_PERCENT_RANGE.start(), *TEXT_PERCENT_RANGE.end()),
                    Ordering::Relaxed,
                );
            }
        }
    }
}

/// Changes the face or the size of text everywhere, and remembers it.
pub fn set_type(ctx: &egui::Context, face: Option<usize>, percent: u32) {
    FACE.store(face.map_or(NO_FACE, |f| f as u32), Ordering::Relaxed);
    let percent = percent.clamp(*TEXT_PERCENT_RANGE.start(), *TEXT_PERCENT_RANGE.end());
    TEXT_PERCENT.store(percent, Ordering::Relaxed);
    install_fonts(ctx);
    if let Some(file) = type_file() {
        let face = face.map_or("theme", |f| FACES[f].name);
        let saved = file
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&file, format!("face = {face}\nsize = {percent}\n")));
        if let Err(e) = saved {
            tracing::warn!("could not save the type: {e}");
        }
    }
}

/// The face in use: the one chosen, else the theme's.
fn face_in_use() -> usize {
    chosen_face().unwrap_or(if condensed() { 1 } else { 0 })
}

/// The face in use for all text, with Inter and egui's own fonts behind it for what it does not
/// cover. Inter's digits differ in width and egui cannot ask for its even ones, so the numbers that run
/// in the display keep the old font: see [`digit_font`].
pub fn install_fonts(ctx: &egui::Context) {
    use egui::{FontData, FontDefinitions, FontFamily, FontTweak};
    let mut fonts = FontDefinitions::default();
    let size = text_percent() as f32 / 100.0;
    let first = face_in_use();
    for (k, face) in FACES.iter().enumerate() {
        // Only the face in use and Inter, behind it.
        if k != first && k != 0 {
            continue;
        }
        let mut tweak = FontTweak {
            scale: face.scale * size,
            y_offset_factor: face.y_offset,
            ..FontTweak::default()
        };
        if let Some(weight) = face.weight {
            tweak.coords = egui::epaint::text::VariationCoords::new([("wght", weight)]);
        }
        let data = FontData::from_static(face.bytes).tweak(tweak);
        fonts.font_data.insert(face.name.to_owned(), data.into());
    }
    let old = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    fonts.families.insert(FontFamily::Name(DIGITS.into()), old);
    if let Some(all) = fonts.families.get_mut(&FontFamily::Proportional) {
        all.insert(0, FACES[0].name.to_owned());
        if first != 0 {
            all.insert(0, FACES[first].name.to_owned());
        }
    }
    if let Some(all) = fonts.families.get_mut(&FontFamily::Monospace) {
        all.push(FACES[0].name.to_owned());
    }
    add_own_fonts(&mut fonts);
    ctx.set_fonts(fonts);
    FONTS_READY.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Display faces the user put in `<config folder>/fonts` (.ttf or .otf), in name order. They are
/// read from the user's own computer and never shipped with Ovrtone, so a font licensed for
/// personal use stays on the machine it was licensed for.
fn own_font_files() -> Vec<std::path::PathBuf> {
    let Some(dir) = crate::recent_files::config_dir().map(|d| d.join("fonts")) else {
        return Vec::new();
    };
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map(|it| {
            it.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| e.eq_ignore_ascii_case("ttf") || e.eq_ignore_ascii_case("otf"))
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files.truncate(2);
    files
}

/// How many of the user's own display faces were loaded.
static OWN_FONTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn own_family(k: usize) -> String {
    format!("own_display_{k}")
}

/// Adds the user's display faces as their own families, with the usual fonts behind them for
/// any letter they lack.
fn add_own_fonts(fonts: &mut egui::FontDefinitions) {
    use egui::{FontData, FontFamily};
    let behind = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let mut count = 0;
    for path in own_font_files() {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let name = own_family(count);
        fonts
            .font_data
            .insert(name.clone(), FontData::from_owned(bytes).into());
        let mut family = vec![name.clone()];
        family.extend(behind.iter().cloned());
        fonts.families.insert(FontFamily::Name(name.into()), family);
        count += 1;
    }
    OWN_FONTS.store(count, std::sync::atomic::Ordering::Relaxed);
}

/// The user's display face number `k` (the first, else the second, else the usual font).
pub fn own_font(size: f32, k: usize) -> egui::FontId {
    let count = OWN_FONTS.load(std::sync::atomic::Ordering::Relaxed);
    if count == 0 {
        return egui::FontId::proportional(size);
    }
    egui::FontId::new(size, egui::FontFamily::Name(own_family(k.min(count - 1)).into()))
}

/// A font whose digits are all one width, so that a running number stands still.
pub fn digit_font(size: f32) -> egui::FontId {
    if FONTS_READY.load(std::sync::atomic::Ordering::Relaxed) {
        egui::FontId::new(size, egui::FontFamily::Name(DIGITS.into()))
    } else {
        egui::FontId::proportional(size)
    }
}

/// Accessibility (Settings > Display): text and edges at full contrast, and a strong ring round the
/// control that has the keyboard's focus. Kept in the settings folder.
pub static HIGH_CONTRAST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub static FOCUS_RING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
static ACCESS_LOADED: std::sync::Once = std::sync::Once::new();
const ACCESS_FILE: &str = "accessibility";

fn load_access() {
    ACCESS_LOADED.call_once(|| {
        let text = crate::recent_files::config_dir()
            .filter(|_| !cfg!(test))
            .and_then(|d| std::fs::read_to_string(d.join(ACCESS_FILE)).ok())
            .unwrap_or_default();
        for line in text.lines() {
            match line.trim() {
                "high-contrast" => HIGH_CONTRAST.store(true, Ordering::Relaxed),
                "no-focus-ring" => FOCUS_RING.store(false, Ordering::Relaxed),
                _ => {}
            }
        }
    });
}

/// Saves the accessibility choices and shows them at once.
pub fn set_access(ctx: &egui::Context, high_contrast: bool, focus_ring: bool) {
    load_access();
    HIGH_CONTRAST.store(high_contrast, Ordering::Relaxed);
    FOCUS_RING.store(focus_ring, Ordering::Relaxed);
    if let Some(dir) = crate::recent_files::config_dir().filter(|_| !cfg!(test)) {
        let mut text = String::new();
        if high_contrast {
            text.push_str("high-contrast\n");
        }
        if !focus_ring {
            text.push_str("no-focus-ring\n");
        }
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join(ACCESS_FILE), text);
    }
    apply(ctx);
}

/// The accessibility choices now: (high contrast, focus ring).
pub fn access() -> (bool, bool) {
    load_access();
    (HIGH_CONTRAST.load(Ordering::Relaxed), FOCUS_RING.load(Ordering::Relaxed))
}

pub fn apply(ctx: &egui::Context) {
    load_access();
    let mut v = if LIGHT.load(Ordering::Relaxed) {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };
    // Letters keep the shapes they were drawn with, not bent to the pixel grid: smoother, and evenly
    // spaced at any display scale.
    v.text_options.font_hinting = false;
    v.panel_fill = bg();
    v.window_fill = glass_fill();
    v.extreme_bg_color = grid_bg();
    v.override_text_color = Some(text());
    v.warn_fg_color = SOLO;
    v.error_fg_color = DANGER;
    v.disabled_alpha = DISABLED_ALPHA;
    v.window_stroke = Stroke::new(1.0, GLASS_EDGE);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v.widgets.noninteractive.bg_fill = panel();
    v.widgets.inactive.bg_fill = button();
    v.widgets.inactive.weak_bg_fill = button();
    v.widgets.hovered.weak_bg_fill = button_hovered();
    v.widgets.active.weak_bg_fill = accent();
    v.selection.bg_fill = accent();
    let r = CornerRadius::same(CORNERS.load(Ordering::Relaxed).min(16) as u8);
    v.widgets.inactive.corner_radius = r;
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.corner_radius = r;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, border());
    if HIGH_CONTRAST.load(Ordering::Relaxed) {
        // Text at full strength, and every edge of a control drawn plainly.
        let ink = if LIGHT.load(Ordering::Relaxed) { Color32::BLACK } else { Color32::WHITE };
        v.override_text_color = Some(ink);
        let edge = Stroke::new(1.5, ink.gamma_multiply(0.85));
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, ink.gamma_multiply(0.6));
        v.widgets.inactive.bg_stroke = edge;
        v.widgets.inactive.fg_stroke = Stroke::new(1.5, ink);
        v.widgets.hovered.fg_stroke = Stroke::new(1.5, ink);
        v.widgets.active.fg_stroke = Stroke::new(1.5, ink);
        v.window_stroke = Stroke::new(1.5, ink.gamma_multiply(0.8));
        v.disabled_alpha = 0.6;
    }
    if FOCUS_RING.load(Ordering::Relaxed) {
        // A control with the keyboard's focus is drawn as hovered: give that a ring that shows.
        v.widgets.hovered.bg_stroke = Stroke::new(2.0, accent());
        v.selection.stroke = Stroke::new(2.0, accent());
    }
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
