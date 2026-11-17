//! The spatial mix's two windows. Mix > Spatial Audio is the renderer: the bed format, where it is
//! heard, the room seen in 3D, every channel with its mute, solo and headphone distance, the loudness,
//! and the exports. The panner places one track: the room from above, its height from the side, and
//! its size, spread, centre and LFE.
use crate::model::{AutoTarget, Project};
use crate::UiState;
use egui::{pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use ovrtone_engine::{
    Distance, ObjectAxis, ObjectPlace, SpatialMode, SpeakerLayout, CHANNEL_NAMES, LFE_CHANNEL,
    SPATIAL_CHANNELS,
};

/// The spatial windows' own colours: a dark studio look whatever the theme, as the room reads best
/// on dark.
pub const BG: Color32 = Color32::from_rgb(0x16, 0x1a, 0x21);
pub const PANEL: Color32 = Color32::from_rgb(0x20, 0x26, 0x31);
pub const DARK_ROOM: Color32 = Color32::from_rgb(0x0d, 0x16, 0x24);
pub const RING: Color32 = Color32::from_rgb(0x34, 0x4a, 0x68);
pub const SPEAKER: Color32 = Color32::from_rgb(0x9a, 0xa6, 0xb8);
pub const OBJECT: Color32 = Color32::from_rgb(0x4c, 0xa6, 0xff);
pub const SURROUND: Color32 = Color32::from_rgb(0xf0, 0xb4, 0x3c);
pub const VALUE: Color32 = Color32::from_rgb(0x62, 0xb4, 0xff);
pub const TEXT: Color32 = Color32::from_rgb(0xd2, 0xd8, 0xe2);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x8a, 0x94, 0xa4);

/// Gives a window's contents the dark studio look.
fn studio_style(ui: &mut egui::Ui) {
    let mut v = egui::Visuals::dark();
    v.override_text_color = Some(TEXT);
    v.panel_fill = BG;
    v.window_fill = BG;
    v.extreme_bg_color = DARK_ROOM;
    v.faint_bg_color = PANEL;
    v.selection.bg_fill = Color32::from_rgb(0x2a, 0x6f, 0xc9);
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.inactive.weak_bg_fill = PANEL;
    v.widgets.inactive.bg_fill = PANEL;
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x2b, 0x33, 0x41);
    v.widgets.active.weak_bg_fill = Color32::from_rgb(0x2a, 0x6f, 0xc9);
    *ui.visuals_mut() = v;
}

/// A studio window's own title row (the window has no title bar, as it is drawn in its own
/// colours): the title, and a button that closes it.
fn studio_header(ui: &mut egui::Ui, title: &str, open: &mut bool) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(title)
                .size(15.0)
                .strong()
                .color(Color32::WHITE),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(egui::Button::new(egui::RichText::new("✕").color(TEXT_DIM)).frame(false))
                .on_hover_text("Close")
                .clicked()
            {
                *open = false;
            }
        });
    });
    // An accent line under the title, fading out, in place of a plain rule.
    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
    let g = ui.painter();
    let steps = 24;
    for k in 0..steps {
        let f = k as f32 / steps as f32;
        let x0 = line.left() + line.width() * f;
        let x1 = line.left() + line.width() * (f + 1.0 / steps as f32);
        g.line_segment(
            [pos2(x0, line.center().y), pos2(x1, line.center().y)],
            Stroke::new(1.5, OBJECT.gamma_multiply((1.0 - f) * 0.8)),
        );
    }
}

/// The size of the top view on screen.
const TOP_RADIUS: f32 = 88.0;
/// The most objects a spatial file holds alongside its bed.
const MAX_OBJECTS: usize = 58;

#[derive(Default)]
pub struct SpatialView {
    pub open: bool,
    /// The panner of the selected track.
    pub panner_open: bool,
    /// The outputs the device has, told by the program each frame.
    pub device_channels: u32,
    /// Each channel's level, falling back like a meter.
    pub peaks: [f32; SPATIAL_CHANNELS],
    /// How the 3D room is looked at.
    pub angle: RoomAngle,
    /// The Speaker Setup window.
    pub speakers_open: bool,
    /// Why the chosen HRTF file could not be used, as the program says each frame.
    pub hrtf_error: Option<String>,
}

/// Where the mix is heard, apart from its bed format.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Output {
    Off,
    Headphones,
    Speakers,
    Folded,
    Downmix,
}

impl Output {
    const ALL: [Output; 5] = [
        Output::Off,
        Output::Headphones,
        Output::Speakers,
        Output::Downmix,
        Output::Folded,
    ];

    fn name(self) -> &'static str {
        match self {
            Output::Off => "Off (plain stereo mix)",
            Output::Headphones => "Headphones",
            Output::Speakers => "Speakers",
            Output::Folded => "Stereo fold-down",
            Output::Downmix => "Smaller speaker layout",
        }
    }

    fn of(mode: SpatialMode) -> Self {
        match mode {
            SpatialMode::Off => Output::Off,
            SpatialMode::Binaural(_) => Output::Headphones,
            SpatialMode::Speakers(_) => Output::Speakers,
            SpatialMode::Folded(_) => Output::Folded,
            SpatialMode::Downmix(..) => Output::Downmix,
        }
    }

    /// The mode for this output with `bed` as the bed format.
    fn with(self, bed: SpeakerLayout) -> SpatialMode {
        match self {
            Output::Off => SpatialMode::Off,
            Output::Headphones => SpatialMode::Binaural(bed),
            Output::Speakers => SpatialMode::Speakers(bed),
            Output::Folded => SpatialMode::Folded(bed),
            Output::Downmix => {
                SpatialMode::Downmix(bed, smaller(bed).last().copied().unwrap_or(bed))
            }
        }
    }
}

/// The layouts with fewer speakers than `bed`, the largest last.
fn smaller(bed: SpeakerLayout) -> Vec<SpeakerLayout> {
    SpeakerLayout::ALL
        .into_iter()
        .filter(|l| l.channels() < bed.channels())
        .collect()
}

/// How the 3D room is looked at.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RoomAngle {
    #[default]
    Perspective,
    Top,
    Front,
    Side,
    Rear,
}

impl RoomAngle {
    const ALL: [RoomAngle; 5] = [
        Self::Perspective,
        Self::Top,
        Self::Front,
        Self::Side,
        Self::Rear,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Perspective => "3D",
            Self::Top => "Top",
            Self::Front => "Front",
            Self::Side => "Side",
            Self::Rear => "Rear",
        }
    }
}

/// A place from a point on the top view, relative to its middle: the edge is the height of the ears.
#[allow(dead_code)]
fn place_at(offset: Vec2, radius: f32) -> (f32, f32) {
    let reach = (offset.length() / radius).clamp(0.0, 1.0);
    (offset.x.atan2(-offset.y).to_degrees(), 90.0 * (1.0 - reach))
}

/// The point on the top view of a place, relative to its middle.
fn point_of(azimuth: f32, elevation: f32, radius: f32) -> Vec2 {
    let reach = (1.0 - elevation.clamp(0.0, 90.0) / 90.0) * radius;
    let a = azimuth.to_radians();
    vec2(a.sin(), -a.cos()) * reach
}

/// Where a track's object is at the playhead, its lanes and all.
fn place_now(t: &crate::model::Track, beat: f64) -> Option<ObjectPlace> {
    let mut place = t.object?;
    // Only the lanes that play: X, Y, Z and Distance are another view of Around and Height, and
    // setting them from the stored place would undo a lane of those.
    for axis in ObjectAxis::ALL {
        if let Some(v) = t.auto.played(AutoTarget::Object(axis), beat) {
            place.set(axis, v);
        }
    }
    Some(place)
}

/// Moves a control of a track's object as its automation mode says: recording in Write, Touch and
/// Latch, and not at all where Read plays a lane.
fn move_axis(p: &mut Project, track: usize, axis: ObjectAxis, value: f32) {
    if let Some(t) = p.tracks.get_mut(track) {
        if crate::automation::editable(t, AutoTarget::Object(axis)) {
            crate::automation::moved(t, AutoTarget::Object(axis), value);
        }
    }
}

fn release_axes(p: &mut Project, track: usize, axes: &[ObjectAxis]) {
    let (beat, playing) = (p.position_beats, p.playing);
    if let Some(t) = p.tracks.get_mut(track) {
        for axis in axes {
            crate::automation::released(t, AutoTarget::Object(*axis), beat, playing);
        }
    }
}

/// A point of the room, as the box of the 3D view has it: X right, Y ahead, Z up, each -1..=1.
fn in_the_box(azimuth: f32, elevation: f32) -> [f32; 3] {
    let (a, e) = (azimuth.to_radians(), elevation.to_radians());
    let (x, y) = (a.sin(), a.cos());
    let wall = x.abs().max(y.abs()).max(1e-6);
    [x / wall * e.cos(), y / wall * e.cos(), e.sin()]
}

/// The room in 3D, seen from behind and above the listener: its box, the speakers of the bed, and the
/// objects where they are now.
fn room_view(ui: &mut egui::Ui, p: &Project, layout: SpeakerLayout, size: Vec2, angle: RoomAngle) {
    let (r, _) = ui.allocate_exact_size(size, Sense::hover());
    let g = ui.painter_at(r);
    g.rect_filled(r, 6.0, crate::theme::gray(0x0e));
    g.rect_stroke(
        r,
        6.0,
        Stroke::new(1.0, crate::theme::gray(0x2a)),
        egui::StrokeKind::Inside,
    );
    let persp = angle == RoomAngle::Perspective;
    let c = r.center() + if persp { vec2(0.0, 18.0) } else { Vec2::ZERO };
    let scale = r.width().min(r.height() * 1.4) * 0.34;
    let flat = r.width().min(r.height()) * 0.4;
    // A simple perspective (further away is smaller and higher on the screen), or flat from one side.
    let project = |[x, y, z]: [f32; 3]| -> Pos2 {
        match angle {
            RoomAngle::Perspective => {
                let depth = 1.0 / (1.6 + y * 0.45);
                c + vec2(
                    x * scale * depth * 1.6,
                    -(z * 0.9 + y * 0.35) * scale * depth * 1.6,
                )
            }
            RoomAngle::Top => c + vec2(x, -y) * flat,
            // From behind the listener, looking ahead.
            RoomAngle::Front => c + vec2(x, 0.5 - z) * flat,
            // From the right, the front to the right.
            RoomAngle::Side => c + vec2(y, 0.5 - z) * flat,
            // From the front, looking back at the listener.
            RoomAngle::Rear => c + vec2(-x, 0.5 - z) * flat,
        }
    };
    let line = Stroke::new(1.0, crate::theme::gray(0x40));
    let corners = |z: f32| {
        [
            [-1.0, -1.0, z],
            [1.0, -1.0, z],
            [1.0, 1.0, z],
            [-1.0, 1.0, z],
        ]
    };
    for z in [0.0, 1.0] {
        let k = corners(z).map(project);
        for i in 0..4 {
            g.line_segment([k[i], k[(i + 1) % 4]], line);
        }
    }
    for (a, b) in corners(0.0).iter().zip(corners(1.0)) {
        g.line_segment([project(*a), project(b)], line);
    }
    // The floor, lit faintly from the middle.
    let floor: Vec<Pos2> = corners(0.0).iter().map(|k| project(*k)).collect();
    g.add(egui::Shape::convex_polygon(
        floor,
        OBJECT.gamma_multiply(0.05),
        Stroke::NONE,
    ));
    // The floor's grid.
    for k in 1..4 {
        let f = -1.0 + k as f32 * 0.5;
        g.line_segment(
            [project([f, -1.0, 0.0]), project([f, 1.0, 0.0])],
            Stroke::new(1.0, crate::theme::gray(0x24)),
        );
        g.line_segment(
            [project([-1.0, f, 0.0]), project([1.0, f, 0.0])],
            Stroke::new(1.0, crate::theme::gray(0x24)),
        );
    }
    // Where the objects are now, to light the speakers near them.
    let beat = p.position_beats;
    let objects: Vec<[f32; 3]> = p
        .tracks
        .iter()
        .filter_map(|t| place_now(t, beat))
        .map(|o| o.xyz())
        .collect();
    let lit = |k: [f32; 3]| -> f32 {
        objects
            .iter()
            .map(|o| {
                let d =
                    ((o[0] - k[0]).powi(2) + (o[1] - k[1]).powi(2) + (o[2] - k[2]).powi(2)).sqrt();
                (1.0 - d / 0.9).clamp(0.0, 1.0)
            })
            .fold(0.0, f32::max)
    };
    // The listener.
    let me = project([0.0, 0.0, 0.0]);
    g.circle_filled(me, 11.0, crate::theme::gray(0x70).gamma_multiply(0.15));
    g.circle_filled(me, 6.0, crate::theme::gray(0x70));
    for (channel, name) in CHANNEL_NAMES.iter().enumerate().take(SPATIAL_CHANNELS) {
        if let Some((az, el)) = layout.place(channel) {
            let k = in_the_box(az, el);
            let at = project(k);
            let glow = lit(k);
            if glow > 0.0 {
                g.circle_filled(at, 10.0, VALUE.gamma_multiply(0.25 * glow));
            }
            speaker_box(&g, at, SPEAKER.lerp_to_gamma(VALUE, glow));
            g.text(
                at + vec2(0.0, 9.0),
                Align2::CENTER_TOP,
                *name,
                FontId::proportional(9.0),
                crate::theme::text_dim(),
            );
        }
    }
    for (i, t) in p.tracks.iter().enumerate() {
        if let Some(o) = place_now(t, beat) {
            let [x, y, z] = o.xyz();
            let at = project([x, y, z]);
            // A line down to the floor and a shadow there, so its height reads.
            let below = project([x, y, 0.0]);
            g.line_segment([at, below], Stroke::new(1.0, t.color.gamma_multiply(0.35)));
            g.circle_filled(below, 3.0, Color32::from_black_alpha(90));
            for (k, a) in [(14.0, 0.06), (10.0, 0.12)] {
                g.circle_filled(at, k, t.color.gamma_multiply(a));
            }
            g.circle_filled(at, if i == p.selected_track { 7.0 } else { 5.0 }, t.color);
            if i == p.selected_track {
                g.circle_stroke(at, 8.5, Stroke::new(1.5, Color32::WHITE));
            }
        }
    }
}

/// Each channel of the bed: its name, how far it sounds on headphones, mute and solo, and its level.
fn channels(ui: &mut egui::Ui, p: &mut Project, view: &SpatialView, layout: SpeakerLayout) {
    let headphones = matches!(p.spatial_mode, SpatialMode::Binaural(_));
    egui::Grid::new(egui::Id::unique("spatial_channels"))
        .num_columns(6)
        .spacing(vec2(8.0, 3.0))
        .show(ui, |ui| {
            ui.label("Channel");
            ui.label(if headphones { "Distance" } else { "" });
            ui.label("");
            ui.label("");
            ui.label("");
            ui.label("Level");
            ui.end_row();
            for (c, name) in CHANNEL_NAMES.iter().enumerate().take(SPATIAL_CHANNELS) {
                if !layout.has_channel(c) {
                    continue;
                }
                ui.label(*name);
                if headphones && c != LFE_CHANNEL {
                    let now = p.spatial_channels.distance[c];
                    egui::ComboBox::from_id_salt(egui::Id::unique(("distance", c)))
                        .width(60.0)
                        .selected_text(now.name())
                        .show_ui(ui, |ui| {
                            for d in Distance::ALL {
                                if ui.selectable_label(now == d, d.name()).clicked() {
                                    p.spatial_channels.distance[c] = d;
                                }
                            }
                        });
                } else {
                    ui.label("");
                }
                let muted = p.spatial_mute[c];
                if ui
                    .selectable_label(muted, "M")
                    .on_hover_text("Mute this channel")
                    .clicked()
                {
                    p.spatial_mute[c] = !muted;
                }
                let solo = p.spatial_solo[c];
                if ui
                    .selectable_label(solo, "S")
                    .on_hover_text("Hear only the soloed channels")
                    .clicked()
                {
                    p.spatial_solo[c] = !solo;
                }
                let testing = p.spatial_test == Some(c);
                if ui
                    .selectable_label(testing, "T")
                    .on_hover_text("Pink noise on this channel, to find its speaker")
                    .clicked()
                {
                    p.spatial_test = if testing { None } else { Some(c) };
                }
                let (r, _) = ui.allocate_exact_size(vec2(90.0, 8.0), Sense::hover());
                let g = ui.painter();
                g.rect_filled(r, 2.0, crate::theme::gray(0x12));
                let level = view.peaks[c].clamp(0.0, 1.0);
                let db = 20.0 * level.max(1e-4).log10();
                let mut lit = r;
                lit.set_width(((db + 60.0) / 60.0).clamp(0.0, 1.0) * r.width());
                g.rect_filled(lit, 2.0, crate::theme::GOOD);
                ui.end_row();
            }
        });
}

/// A window frame in the theme's panel colour, as the effect windows have.
fn themed_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(BG)
        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(0x2e, 0x35, 0x41)))
        .corner_radius(10)
        .inner_margin(egui::Margin::same(12))
        .shadow(egui::epaint::Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: egui::Color32::from_black_alpha(110),
        })
}

/// The renderer.
fn renderer(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    let mut open = s.spatial.open;
    egui::Window::new("Spatial Renderer")
        .id(egui::Id::unique("spatial_window"))
        .frame(themed_frame())
        .title_bar(false)
        .default_pos(pos2(40.0, 60.0))
        .resizable(false)
        .show(ctx, |ui| {
            studio_style(ui);
            studio_header(ui, "Spatial Renderer", &mut open);
            let bed = p.spatial_mode.layout().unwrap_or(p.spatial_bed);
            let output = Output::of(p.spatial_mode);
            ui.horizontal(|ui| {
                ui.label("Volume");
                let mut db = p.spatial_volume_db;
                if ui
                    .add(egui::Slider::new(&mut db, -24.0..=6.0).suffix(" dB").step_by(0.5))
                    .changed()
                {
                    p.spatial_volume_db = db;
                }
                ui.label("LFE");
                let lfe = p.spatial_channels.lfe_hz;
                let lfe_name = |hz: f32| if hz <= 0.0 { "Full range".to_string() } else { format!("{hz:.0} Hz low-pass") };
                egui::ComboBox::from_id_salt(egui::Id::unique("spatial_lfe"))
                    .selected_text(lfe_name(lfe))
                    .show_ui(ui, |ui| {
                        for hz in [0.0f32, 80.0, 120.0, 160.0] {
                            if ui.selectable_label(lfe == hz, lfe_name(hz)).clicked() {
                                p.spatial_channels.lfe_hz = hz;
                            }
                        }
                    })
                    .response
                    .on_hover_text("A low-pass on the LFE channel only, as a bass-management filter would have");
                ui.separator();
                ui.label("Bed Format");
                egui::ComboBox::from_id_salt(egui::Id::unique("spatial_bed"))
                    .selected_text(bed.name())
                    .show_ui(ui, |ui| {
                        for layout in SpeakerLayout::ALL {
                            if ui.selectable_label(bed == layout, layout.name()).clicked() {
                                p.spatial_bed = layout;
                                p.spatial_mode = output.with(layout);
                            }
                        }
                    });
                ui.label("Panning");
                egui::ComboBox::from_id_salt(egui::Id::unique("spatial_law"))
                    .selected_text(p.spatial_law.name())
                    .show_ui(ui, |ui| {
                        for law in ovrtone_engine::SpatialLaw::ALL {
                            let tip = match law {
                                ovrtone_engine::SpatialLaw::Vbap => {
                                    "The two or three speakers around each object: sharp, exact placement"
                                }
                                ovrtone_engine::SpatialLaw::Smooth => {
                                    "Every speaker by how near it is: softer, as projects made before VBAP"
                                }
                            };
                            if ui
                                .selectable_label(p.spatial_law == law, law.name())
                                .on_hover_text(tip)
                                .clicked()
                            {
                                p.spatial_law = law;
                            }
                        }
                    });
                ui.label("Output");
                egui::ComboBox::from_id_salt(egui::Id::unique("spatial_output"))
                    .selected_text(output.name())
                    .show_ui(ui, |ui| {
                        for o in Output::ALL {
                            if ui.selectable_label(output == o, o.name()).clicked() {
                                p.spatial_mode = o.with(bed);
                            }
                        }
                    });
            });
            match output {
                Output::Off => {
                    ui.label("The plain stereo mix. Objects keep their places for when this is on.");
                }
                Output::Headphones => {
                    let measured = crate::hrtf::name();
                    ui.horizontal(|ui| {
                        ui.label("Head:");
                        ui.label(measured.as_deref().unwrap_or("Modelled (no data needed)"));
                        if ui
                            .button("Load SOFA File…")
                            .on_hover_text("A measured head (AES69 .sofa): yours, or one of the free research sets")
                            .clicked()
                        {
                            if let Some(path) = rfd::FileDialog::new().add_filter("SOFA", &["sofa"]).pick_file() {
                                crate::hrtf::choose(Some(path));
                            }
                        }
                        if measured.is_some() && ui.button("Use Modelled Head").clicked() {
                            crate::hrtf::choose(None);
                        }
                    });
                    if let Some(e) = s.spatial.hrtf_error.as_ref() {
                        ui.colored_label(ui.visuals().warn_fg_color, e);
                    }
                    ui.horizontal(|ui| {
                        ui.add_enabled(
                            measured.is_none(),
                            egui::Slider::new(&mut p.spatial_room, 0.0..=1.0).text("Room"),
                        )
                        .on_hover_text("Early reflections, which help the sound sit outside your head (modelled head only)");
                    });
                }
                Output::Speakers => {
                    let have = s.spatial.device_channels as usize;
                    if have < bed.channels() {
                        ui.colored_label(
                            ui.visuals().warn_fg_color,
                            format!(
                                "This output has {have} channels and {} needs {}: you hear it folded down to stereo. Choose a device with more outputs in Settings > Audio.",
                                bed.name(),
                                bed.channels()
                            ),
                        );
                    } else {
                        ui.label(format!(
                            "Out on channels 1 to {} in this order: L R C LFE, rear, side, top.",
                            bed.channels()
                        ));
                    }
                }
                Output::Folded => {
                    ui.label("Folded down to plain stereo: how the mix sounds without a spatial setup.");
                }
                Output::Downmix => {
                    let to = p.spatial_mode.output_layout().unwrap_or(bed);
                    ui.horizontal(|ui| {
                        ui.label("Heard on");
                        egui::ComboBox::from_id_salt(egui::Id::unique("spatial_downmix"))
                            .selected_text(to.name())
                            .show_ui(ui, |ui| {
                                for l in smaller(bed) {
                                    if ui.selectable_label(to == l, l.name()).clicked() {
                                        p.spatial_mode = SpatialMode::Downmix(bed, l);
                                    }
                                }
                            });
                        ui.label(format!("speakers: {} channels out, in this order.", to.channels()));
                    });
                    egui::CollapsingHeader::new("Downmix matrix")
                        .id_salt(egui::Id::unique("downmix_matrix"))
                        .show(ui, |ui| downmix_grid(ui, bed, to));
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                for a in RoomAngle::ALL {
                    if ui.selectable_label(s.spatial.angle == a, a.name()).clicked() {
                        s.spatial.angle = a;
                    }
                }
                ui.separator();
                if ui
                    .button("Speaker Setup…")
                    .on_hover_text("Where your speakers really are, their levels, delays and a test signal")
                    .clicked()
                {
                    s.spatial.speakers_open = !s.spatial.speakers_open;
                }
            });
            ui.horizontal_top(|ui| {
                room_view(ui, p, bed, vec2(360.0, 260.0), s.spatial.angle);
                ui.vertical(|ui| channels(ui, p, &s.spatial, bed));
            });
            egui::CollapsingHeader::new(format!("Spatial Buses ({})", p.spatial_buses.len()))
                .id_salt(egui::Id::unique("spatial_buses"))
                .show(ui, |ui| {
                    let installed = s.mixer.installed.clone();
                    buses_section(ui, p, &installed, &mut s.bus_plugin_editor);
                });
            ui.separator();
            let objects = p.tracks.iter().filter(|t| t.object.is_some() && !t.bed_pan).count();
            let panned = p.tracks.iter().filter(|t| t.object.is_some() && t.bed_pan).count();
            ui.horizontal(|ui| {
                ui.label(format!("Objects: {objects}   Surround panned in the bed: {panned}"))
                    .on_hover_text(format!(
                        "No limit while mixing. An ADM file holds up to {MAX_OBJECTS} stereo objects beside its bed."
                    ));
                ui.separator();
                match s.mastering.readings() {
                    Some((integrated, short, true_peak)) => {
                        let lufs = |v: f64| {
                            if v <= -120.0 {
                                "--".to_string()
                            } else {
                                format!("{v:.1}")
                            }
                        };
                        let peak = if true_peak <= -100.0 {
                            "--".to_string()
                        } else {
                            format!("{true_peak:.1}")
                        };
                        ui.label(format!(
                            "Integrated {} LUFS   Short-term {} LUFS   True peak {peak} dBTP",
                            lufs(integrated),
                            lufs(short),
                        ))
                        .on_hover_text("EBU R128, on what goes out on left and right");
                    }
                    None => {
                        ui.label("Loudness shows while this plays");
                    }
                }
            });
            ui.horizontal(|ui| {
                let selected = p.selected_track;
                let name = p.tracks.get(selected).map(|t| t.name.clone()).unwrap_or_default();
                if ui
                    .button(format!("Panner: {name}"))
                    .on_hover_text("Place the selected track in the room")
                    .clicked()
                {
                    s.spatial.panner_open = true;
                }
                let there = crate::bounce::project_end(p) > 0.0 && !s.bounce.busy();
                let channel_file = ui
                    .add_enabled(there, egui::Button::new("Export ADM BW64…"))
                    .on_hover_text("7.1.4, 48 kHz, 24 bit, with the objects mixed into the channels");
                let with_objects = ui
                    .add_enabled(there, egui::Button::new("Export Bed + Objects…"))
                    .on_hover_text("ADM BW64: a 7.1.4 bed, then every object as its own tracks with its moves");
                use crate::bounce::FileFormat;
                let mut format = if channel_file.clicked() {
                    Some(FileFormat::Adm)
                } else if with_objects.clicked() {
                    Some(FileFormat::AdmObjects)
                } else {
                    None
                };
                ui.menu_button("Other Exports", |ui| {
                    ui.add_enabled_ui(there, |ui| {
                        let items = [
                            (FileFormat::Adm712, "ADM BW64, 7.1.2 Bed…".to_string(), "The Atmos-style bed: 7.1.2, 48 kHz, 24 bit, with the objects mixed in"),
                            (FileFormat::AdmObjects712, "ADM BW64, 7.1.2 Bed + Mono Objects…".to_string(), "The Atmos-style file: a 7.1.2 bed and up to 118 objects, one track each, with their moves"),
                            (FileFormat::SpatialLayout, format!("{} Multichannel WAV…", bed.name()), "The bed in its own channel order, 24 bit"),
                            (FileFormat::Binaural, "Binaural Stereo…".to_string(), "The mix through the modelled head, for headphones"),
                            (FileFormat::Ambix1, "Ambisonics, First Order…".to_string(), "AmbiX: 4 channels, ACN order, SN3D"),
                            (FileFormat::Ambix3, "Ambisonics, Third Order…".to_string(), "AmbiX: 16 channels, ACN order, SN3D"),
                        ];
                        for (f, label, tip) in items {
                            if ui.button(label).on_hover_text(tip).clicked() {
                                format = Some(f);
                                ui.close();
                            }
                        }
                    });
                });
                if let Some(format) = format {
                    let (name, filter) = match format {
                        FileFormat::Adm
                        | FileFormat::AdmObjects
                        | FileFormat::Adm712
                        | FileFormat::AdmObjects712 => ("Spatial mix.wav", "ADM BW64"),
                        FileFormat::Binaural => ("Binaural mix.wav", "WAV"),
                        FileFormat::Ambix1 | FileFormat::Ambix3 => ("Ambisonic mix.wav", "AmbiX WAV"),
                        _ => ("Multichannel mix.wav", "WAV"),
                    };
                    let file = rfd::FileDialog::new()
                        .set_file_name(name)
                        .add_filter(filter, &["wav"])
                        .save_file();
                    if let Some(path) = file {
                        s.bounce.queue.push(crate::bounce::Render {
                            mono: false,
                            normalize: None,
                            path,
                            format,
                            from_beat: 0.0,
                            to_beat: crate::bounce::project_end(p),
                            depth: crate::bounce::BitDepth::Int24,
                            out_rate: 0,
                            solo: None,
                            after: crate::bounce::After::Report,
                        });
                    }
                }
            });
        });
    s.spatial.open = open;
}

/// The spatial buses: each a layout (a 7.1.2 bus, say) that tracks are placed in, and a surround
/// panner that places the whole bus in the room.
fn buses_section(
    ui: &mut egui::Ui,
    p: &mut Project,
    installed: &[ovrtone_host::PluginInfo],
    open: &mut Option<(u64, String)>,
) {
    ui.label(
        egui::RichText::new(
            "Route a track to a bus in its panner (Goes to). The track is placed among the bus's own \
             speakers; the bus's panner turns, raises, narrows and widens all of them together.",
        )
        .small(),
    );
    let mut remove = None;
    for (k, b) in p.spatial_buses.iter_mut().enumerate() {
        ui.separator();
        ui.horizontal_top(|ui| {
            bus_view(ui, b.layout, b.place);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut b.name).desired_width(120.0));
                    egui::ComboBox::from_id_salt(egui::Id::unique(("bus_layout", k)))
                        .selected_text(b.layout.name())
                        .show_ui(ui, |ui| {
                            for l in SpeakerLayout::ALL {
                                ui.selectable_value(&mut b.layout, l, l.name());
                            }
                        });
                    ui.toggle_value(&mut b.place.muted, "M")
                        .on_hover_text("Mute this bus");
                    if ui.small_button("Remove").clicked() {
                        remove = Some(k);
                    }
                });
                egui::Grid::new(egui::Id::unique(("bus_grid", k)))
                    .num_columns(2)
                    .show(ui, |ui| {
                        let row = |ui: &mut egui::Ui,
                                   name: &str,
                                   v: &mut f32,
                                   lo: f32,
                                   hi: f32,
                                   suffix: &str| {
                            ui.label(name);
                            ui.add(
                                egui::Slider::new(v, lo..=hi)
                                    .suffix(suffix)
                                    .fixed_decimals(if suffix == "°" { 0 } else { 2 }),
                            );
                            ui.end_row();
                        };
                        row(ui, "Direction", &mut b.place.direction, -180.0, 180.0, "°");
                        row(ui, "Elevation", &mut b.place.elevation, 0.0, 90.0, "°");
                        row(ui, "Spread", &mut b.place.spread, 0.0, 1.0, "");
                        row(ui, "Size", &mut b.place.size, 0.0, 1.0, "");
                        row(ui, "Centre level", &mut b.place.centre, 0.0, 1.0, "");
                        row(ui, "LFE level", &mut b.place.lfe, 0.0, 1.0, "");
                        let mut db = 20.0 * b.place.gain.max(1e-4).log10();
                        ui.label("Level");
                        if ui
                            .add(egui::Slider::new(&mut db, -60.0..=12.0).suffix(" dB"))
                            .changed()
                        {
                            b.place.gain = 10f32.powf(db / 20.0);
                        }
                        ui.end_row();
                    });
                bus_effects(ui, &mut b.fx, k);
                bus_plugins(ui, &mut b.plugins, k, installed, open);
            });
        });
    }
    if let Some(k) = remove {
        p.spatial_buses.remove(k);
        // Tracks on it go back to the main bus; those after it move up one.
        let gone = (k + 1) as u8;
        for t in &mut p.tracks {
            if let Some(o) = t.object.as_mut() {
                if o.bus == gone {
                    o.bus = 0;
                } else if o.bus > gone {
                    o.bus -= 1;
                }
            }
        }
    }
    if p.spatial_buses.len() < ovrtone_engine::MAX_SPATIAL_BUSES
        && ui.button("Add Spatial Bus").clicked()
    {
        let n = p.spatial_buses.len() + 1;
        p.spatial_buses.push(crate::model::SpatialBusDef {
            name: format!("Bus {n}"),
            layout: SpeakerLayout::Surround712,
            place: ovrtone_engine::BusPlace::default(),
            fx: Vec::new(),
            plugins: Vec::new(),
        });
    }
}

/// A spatial bus's effects: each runs on every pair of its channels, so the layout keeps its places.
fn bus_effects(ui: &mut egui::Ui, fx: &mut Vec<crate::model::Insert>, bus: usize) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new("Effects").strong());
    let mut remove = None;
    for (i, e) in fx.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.checkbox(&mut e.bypass, "Bypass");
            egui::CollapsingHeader::new(e.kind.name())
                .id_salt(("bus_fx", bus, i))
                .show(ui, |ui| {
                    for (k, info) in e.kind.params().iter().enumerate() {
                        let mut slider = egui::Slider::new(&mut e.params[k], info.min..=info.max)
                            .text(info.name)
                            .suffix(info.unit);
                        if info.log && info.min > 0.0 {
                            slider = slider.logarithmic(true);
                        }
                        ui.add(slider);
                    }
                });
            if ui
                .small_button("\u{00d7}")
                .on_hover_text("Remove this effect")
                .clicked()
            {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        fx.remove(i);
    }
    if fx.len() < ovrtone_engine::MAX_INSERTS {
        ui.menu_button("+ Effect", |ui| {
            for kind in ovrtone_engine::FxKind::ALL {
                if ui.button(kind.name()).clicked() {
                    fx.push(crate::model::Insert::new(kind));
                    ui.close();
                }
            }
        });
    }
}

/// A spatial bus's plugins: a VST3 effect that takes the bus's layout gets every channel; any other
/// runs as linked stereo copies, one per pair, all moved by its window.
fn bus_plugins(
    ui: &mut egui::Ui,
    plugins: &mut Vec<crate::model::PluginInsert>,
    bus: usize,
    installed: &[ovrtone_host::PluginInfo],
    open: &mut Option<(u64, String)>,
) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new("Plugins").strong())
        .on_hover_text("A surround plugin gets every channel of the bus. A stereo one runs as linked copies, one per pair of channels, all following its window.");
    let mut remove = None;
    for (i, ins) in plugins.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.checkbox(&mut ins.bypass, "Bypass");
            if ui
                .button(&ins.name)
                .on_hover_text("Open its window")
                .clicked()
            {
                *open = Some((ins.uid, ins.name.clone()));
            }
            if let Some(why) = &ins.failed {
                ui.colored_label(egui::Color32::from_rgb(0xe0, 0x70, 0x60), "failed")
                    .on_hover_text(why);
            }
            if ui
                .small_button("\u{00d7}")
                .on_hover_text("Remove this plugin")
                .clicked()
            {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        plugins.remove(i);
    }
    let effects: Vec<&ovrtone_host::PluginInfo> =
        installed.iter().filter(|i| !i.is_instrument).collect();
    if effects.is_empty() {
        return;
    }
    ui.menu_button("+ Plugin", |ui| {
        let search_id = egui::Id::unique(("bus_plugin_search", bus));
        let mut search = ui
            .data_mut(|d| d.get_temp::<String>(search_id))
            .unwrap_or_default();
        ui.add(egui::TextEdit::singleline(&mut search).hint_text("Search"));
        ui.data_mut(|d| d.insert_temp(search_id, search.clone()));
        let wanted = search.to_lowercase();
        egui::ScrollArea::vertical()
            .max_height(320.0)
            .show(ui, |ui| {
                for info in effects
                    .iter()
                    .filter(|i| wanted.is_empty() || i.name.to_lowercase().contains(&wanted))
                {
                    if ui
                        .button(format!("{} ({})", info.name, info.vendor))
                        .clicked()
                    {
                        plugins.push(crate::model::PluginInsert::new(info));
                        ui.close();
                    }
                }
            });
    });
}

/// A bus's channels from above, as its panner places them.
fn bus_view(ui: &mut egui::Ui, layout: SpeakerLayout, place: ovrtone_engine::BusPlace) {
    let radius = 56.0;
    let (r, _) = ui.allocate_exact_size(Vec2::splat(radius * 2.0 + 20.0), Sense::hover());
    let c = r.center();
    let g = ui.painter();
    g.circle_filled(c, radius, crate::theme::gray(0x12));
    g.circle_stroke(c, radius, Stroke::new(1.0, crate::theme::gray(0x30)));
    g.circle_filled(c, 4.0, crate::theme::gray(0x70));
    for &ch in layout.device_order() {
        let Some((az, el)) = layout.place(ch) else {
            continue;
        };
        let az = place.direction + az * place.spread;
        let el = (el + place.elevation).clamp(0.0, 90.0);
        let at = c + point_of(az, el, radius);
        let color = if place.muted {
            crate::theme::gray(0x50)
        } else {
            crate::theme::GOOD
        };
        g.circle_filled(at, 7.0, color.gamma_multiply(0.35));
        g.circle_stroke(at, 7.0, Stroke::new(1.0, color));
        g.text(
            at,
            Align2::CENTER_CENTER,
            CHANNEL_NAMES[ch],
            FontId::proportional(7.5),
            crate::theme::text_dim(),
        );
    }
    label_at(g, c - vec2(0.0, radius + 6.0), "Front");
}

/// The downmix from `bed` to `to` as a table: each channel of the bed (rows) into each speaker of the
/// smaller layout (columns), in dB.
fn downmix_grid(ui: &mut egui::Ui, bed: SpeakerLayout, to: SpeakerLayout) {
    let m = ovrtone_engine::downmix_matrix(bed, to);
    egui::Grid::new(egui::Id::unique("downmix_grid"))
        .spacing(vec2(6.0, 2.0))
        .show(ui, |ui| {
            ui.label("");
            for &t in to.device_order() {
                ui.label(CHANNEL_NAMES[t]);
            }
            ui.end_row();
            for &c in bed.device_order() {
                ui.label(CHANNEL_NAMES[c]);
                for &t in to.device_order() {
                    let g = m[c][t];
                    let text = if g <= 1e-4 {
                        "".to_string()
                    } else if (g - 1.0).abs() < 1e-4 {
                        "0".to_string()
                    } else {
                        format!("{:.1}", 20.0 * g.log10())
                    };
                    ui.label(egui::RichText::new(text).monospace().small());
                }
                ui.end_row();
            }
        });
    ui.label(
        egui::RichText::new(
            "Channels the smaller layout has go straight across; the others are panned to where they would \
             be among its speakers, power kept. The LFE goes only to an LFE.",
        )
        .small(),
    );
}

fn label_at(g: &egui::Painter, at: Pos2, text: &str) {
    g.text(
        at,
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(10.5),
        crate::theme::text_dim(),
    );
}

/// The room from above: the speakers of the bed, the track's left and right where they are.
fn top_view(ui: &mut egui::Ui, p: &mut Project, track: usize, layout: SpeakerLayout) {
    let (r, resp) = ui.allocate_exact_size(
        Vec2::splat(TOP_RADIUS * 2.0 + 64.0),
        Sense::click_and_drag(),
    );
    let c = r.center();
    let g = ui.painter();
    g.rect_filled(r, 8.0, DARK_ROOM.gamma_multiply(0.6));
    sphere(g, c, TOP_RADIUS);
    for ring in [2.0 / 3.0, 1.0 / 3.0] {
        g.circle_stroke(
            c,
            TOP_RADIUS * ring,
            Stroke::new(1.0, RING.gamma_multiply(0.7)),
        );
    }
    g.circle_stroke(c, TOP_RADIUS, Stroke::new(1.5, RING));
    for a in (0..360).step_by(45) {
        let a = (a as f32).to_radians();
        g.line_segment(
            [
                c + vec2(a.sin(), -a.cos()) * 14.0,
                c + vec2(a.sin(), -a.cos()) * TOP_RADIUS,
            ],
            Stroke::new(1.0, RING.gamma_multiply(0.45)),
        );
    }
    // Where the track is now, so the speakers it plays on light up with it.
    let now = p
        .tracks
        .get(track)
        .and_then(|t| place_now(t, p.position_beats));
    let lit = |az: f32, el: f32| -> f32 {
        let Some(o) = now else { return 0.0 };
        let around = ((az - o.azimuth + 540.0).rem_euclid(360.0) - 180.0).abs();
        let up = (el - o.elevation).abs();
        ((1.0 - around / 70.0) * (1.0 - up / 60.0)).clamp(0.0, 1.0)
    };
    // The speakers: those at ear height just outside the ring, pointing in; the tall ones above.
    // Each named, and lit by how near the track is to it.
    for channel in 0..SPATIAL_CHANNELS {
        if let Some((az, el)) = layout.place(channel) {
            let glow = lit(az, el);
            let ink = SPEAKER.lerp_to_gamma(VALUE, glow);
            let name = CHANNEL_NAMES.get(channel).copied().unwrap_or("");
            if el > 0.0 {
                let at = c + point_of(az, el, TOP_RADIUS);
                if glow > 0.05 {
                    g.circle_filled(at, 7.0, VALUE.gamma_multiply(0.25 * glow));
                }
                g.circle_stroke(at, 4.0, Stroke::new(1.2, ink));
            } else {
                let at = c + point_of(az, 0.0, TOP_RADIUS + 11.0);
                if glow > 0.05 {
                    g.circle_filled(at, 9.0, VALUE.gamma_multiply(0.25 * glow));
                }
                speaker_mark(g, at, c, ink);
                g.text(
                    c + point_of(az, 0.0, TOP_RADIUS + 23.0),
                    Align2::CENTER_CENTER,
                    name,
                    FontId::proportional(8.5),
                    TEXT_DIM.lerp_to_gamma(TEXT, glow),
                );
            }
        }
    }
    head(g, c, 1.0);
    g.text(
        c - vec2(0.0, TOP_RADIUS - 14.0),
        Align2::CENTER_CENTER,
        "FRONT",
        FontId::proportional(9.0),
        TEXT_DIM,
    );
    let beat = p.position_beats;
    let Some(t) = p.tracks.get(track) else {
        return;
    };
    let Some(o) = place_now(t, beat) else {
        return;
    };
    let color = if t.bed_pan { SURROUND } else { OBJECT };
    // The top view is the room seen from above: direction and distance alone. Elevation is set
    // in the side view below, so the glyph here keeps moving with direction at any height.
    let reach = TOP_RADIUS * o.radius.clamp(0.0, 1.0).max(0.12);
    let middle = c + point_of(o.azimuth, 0.0, reach);
    if o.size > 0.01 {
        g.circle_filled(middle, 8.0 + o.size * 40.0, color.gamma_multiply(0.12));
    }
    g.line_segment([c, middle], Stroke::new(1.2, color.gamma_multiply(0.7)));
    // A soft glow where it sounds.
    for k in 1..=4 {
        g.circle_filled(middle, 8.0 + k as f32 * 5.0, color.gamma_multiply(0.05));
    }
    // Its two sides, apart by the spread.
    for (word, side) in [("L", -1.0f32), ("R", 1.0f32)] {
        let at = c + point_of(o.azimuth + side * o.spread * 45.0, 0.0, reach);
        g.circle_filled(at + vec2(0.0, 1.5), 9.5, Color32::from_black_alpha(80));
        g.circle_filled(at, 9.5, color.gamma_multiply(0.45));
        g.circle_filled(at - vec2(2.5, 2.5), 4.0, Color32::from_white_alpha(40));
        g.circle_stroke(at, 9.5, Stroke::new(1.5, color));
        g.text(
            at,
            Align2::CENTER_CENTER,
            word,
            FontId::proportional(9.5),
            Color32::WHITE,
        );
    }
    if resp.dragged() {
        if let Some(at) = resp.interact_pointer_pos() {
            // Direction from the angle, distance from how far out; the height stays as it is.
            let off = at - c;
            let az = off.x.atan2(-off.y).to_degrees();
            let distance = (off.length() / TOP_RADIUS).clamp(0.0, 1.0);
            move_axis(p, track, ObjectAxis::Around, az);
            move_axis(p, track, ObjectAxis::Distance, distance);
        }
    }
    if resp.drag_stopped() {
        release_axes(p, track, &[ObjectAxis::Around, ObjectAxis::Distance]);
    }
}

/// A shaded disc: darker at its rim, lighter towards the middle, as a dome seen from above.
fn sphere(g: &egui::Painter, c: Pos2, radius: f32) {
    let steps = 14;
    for k in 0..steps {
        let f = k as f32 / steps as f32;
        let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * f) as u8;
        let col = Color32::from_rgb(mix(0x0d, 0x1f), mix(0x18, 0x3a), mix(0x2a, 0x5e));
        g.circle_filled(c, radius * (1.0 - f * 0.92), col);
    }
}

/// A listener seen from above (`top`) or from the side, at `c`, `scale` times its usual size.
fn head(g: &egui::Painter, c: Pos2, scale: f32) {
    let skin = Color32::from_rgb(0x56, 0x63, 0x78);
    g.circle_filled(c - vec2(10.0, 0.0) * scale, 3.0 * scale, skin);
    g.circle_filled(c + vec2(10.0, 0.0) * scale, 3.0 * scale, skin);
    g.circle_filled(c, 10.0 * scale, skin);
    g.circle_filled(
        c - vec2(0.0, 2.0) * scale,
        8.5 * scale,
        Color32::from_rgb(0x6c, 0x7a, 0x91),
    );
    // The nose shows which way is ahead.
    g.add(egui::Shape::convex_polygon(
        vec![
            c + vec2(-3.0, -9.0) * scale,
            c + vec2(3.0, -9.0) * scale,
            c + vec2(0.0, -13.0) * scale,
        ],
        skin,
        Stroke::NONE,
    ));
}

/// A speaker at `at`, a small wedge facing `towards`.
/// A speaker seen from the front: a cabinet with a woofer and a tweeter, in `color`.
fn speaker_box(g: &egui::Painter, at: Pos2, color: Color32) {
    let body = Rect::from_center_size(at, vec2(11.0, 15.0));
    g.rect_filled(
        body.translate(vec2(1.0, 1.5)),
        2.0,
        Color32::from_black_alpha(70),
    );
    g.rect_filled(body, 2.0, crate::theme::gray(0x1c));
    g.rect_stroke(body, 2.0, Stroke::new(1.0, color), egui::StrokeKind::Inside);
    g.circle_filled(pos2(at.x, body.top() + 4.0), 1.6, color);
    g.circle_stroke(
        pos2(at.x, body.bottom() - 4.5),
        3.2,
        Stroke::new(1.0, color),
    );
    g.circle_filled(pos2(at.x, body.bottom() - 4.5), 1.2, color);
}

fn speaker_mark(g: &egui::Painter, at: Pos2, towards: Pos2, color: Color32) {
    let d = (towards - at).normalized();
    let side = vec2(-d.y, d.x);
    g.add(egui::Shape::convex_polygon(
        vec![
            at + d * 5.0,
            at - d * 4.0 + side * 5.0,
            at - d * 4.0 - side * 5.0,
        ],
        color.gamma_multiply(0.35),
        Stroke::new(1.0, color),
    ));
}

/// The room from the side: the listener at the floor's middle, the height of the track on an arc over
/// them. Dragging sets the height alone.
fn side_view(ui: &mut egui::Ui, p: &mut Project, track: usize) {
    let width = TOP_RADIUS * 2.0 + 64.0;
    let (r, resp) = ui.allocate_exact_size(vec2(width, TOP_RADIUS + 30.0), Sense::click_and_drag());
    let g = ui.painter();
    g.rect_filled(r, 8.0, DARK_ROOM.gamma_multiply(0.6));
    let base = pos2(r.center().x, r.bottom() - 12.0);
    let radius = TOP_RADIUS;
    // The dome, shaded, with lines of height every 30 degrees.
    let steps = 12;
    for k in 0..steps {
        let f = k as f32 / steps as f32;
        let rr = radius * (1.0 - f * 0.9);
        let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * f) as u8;
        let col = Color32::from_rgb(mix(0x0d, 0x1f), mix(0x18, 0x3a), mix(0x2a, 0x5e));
        let half: Vec<Pos2> = (0..=32)
            .map(|i| {
                let a = std::f32::consts::PI * i as f32 / 32.0;
                base + vec2(-a.cos(), -a.sin()) * rr
            })
            .collect();
        g.add(egui::Shape::convex_polygon(half, col, Stroke::NONE));
    }
    let arc = |rr: f32| -> Vec<Pos2> {
        (0..=48)
            .map(|k| {
                let a = std::f32::consts::PI * k as f32 / 48.0;
                base + vec2(-a.cos(), -a.sin()) * rr
            })
            .collect()
    };
    g.add(egui::Shape::line(arc(radius), Stroke::new(1.5, RING)));
    for deg in [30.0f32, 60.0] {
        let a = deg.to_radians();
        let y = base.y - a.sin() * radius;
        let half = a.cos() * radius;
        g.line_segment(
            [pos2(base.x - half, y), pos2(base.x + half, y)],
            Stroke::new(1.0, RING.gamma_multiply(0.5)),
        );
    }
    g.line_segment(
        [
            base - vec2(radius + 6.0, 0.0),
            base + vec2(radius + 6.0, 0.0),
        ],
        Stroke::new(1.0, RING),
    );
    // A listener from the side: head and shoulders.
    let skin = Color32::from_rgb(0x6c, 0x7a, 0x91);
    g.circle_filled(base - vec2(0.0, 18.0), 8.0, skin);
    g.add(egui::Shape::convex_polygon(
        vec![
            base - vec2(13.0, 0.0),
            base + vec2(13.0, 0.0),
            base + vec2(8.0, -9.0),
            base - vec2(8.0, -9.0),
        ],
        Color32::from_rgb(0x56, 0x63, 0x78),
        Stroke::NONE,
    ));
    g.text(
        base + vec2(-radius - 10.0, -10.0),
        Align2::CENTER_CENTER,
        "F",
        FontId::proportional(10.0),
        TEXT_DIM,
    );
    g.text(
        base + vec2(radius + 10.0, -10.0),
        Align2::CENTER_CENTER,
        "R",
        FontId::proportional(10.0),
        TEXT_DIM,
    );
    let beat = p.position_beats;
    let Some(o) = p.tracks.get(track).and_then(|t| place_now(t, beat)) else {
        return;
    };
    let color = if p.tracks[track].bed_pan {
        SURROUND
    } else {
        OBJECT
    };
    // Front is to the left: an object ahead sits left of the middle, one behind to the right.
    let front = o.azimuth.to_radians().cos();
    let e = o.elevation.to_radians();
    let at = base + vec2(-front * e.cos(), -e.sin()) * radius * o.radius.clamp(0.0, 1.0).max(0.12);
    g.line_segment(
        [base - vec2(0.0, 18.0), at],
        Stroke::new(1.0, color.gamma_multiply(0.6)),
    );
    for k in 1..=4 {
        g.circle_filled(at, 7.0 + k as f32 * 4.5, color.gamma_multiply(0.05));
    }
    g.circle_filled(at, 8.0, color.gamma_multiply(0.45));
    g.circle_filled(at - vec2(2.0, 2.0), 3.5, Color32::from_white_alpha(40));
    g.circle_stroke(at, 8.0, Stroke::new(1.5, color));
    if resp.dragged() {
        if let Some(pt) = resp.interact_pointer_pos() {
            // From the pointer's height alone: at the sides (90° either way) the dot sits in the
            // middle of this view, and an angle from the listener read nearly 90° wherever the
            // pointer was, so the elevation could not come down.
            let reach = radius * o.radius.clamp(0.0, 1.0).max(0.12);
            let up = ((base.y - pt.y) / reach).clamp(0.0, 1.0);
            let height = up.asin().to_degrees().clamp(0.0, 90.0);
            move_axis(p, track, ObjectAxis::Height, height);
        }
    }
    if resp.drag_stopped() {
        release_axes(p, track, &[ObjectAxis::Height]);
    }
}

/// The panner of the selected track.
fn panner(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    let track = p.selected_track;
    let Some(name) = p.tracks.get(track).map(|t| t.name.clone()) else {
        return;
    };
    let mut open = s.spatial.panner_open;
    egui::Window::new(format!("Panner: {name}"))
        .id(egui::Id::unique("spatial_panner"))
        .frame(themed_frame())
        .title_bar(false)
        // Beside the renderer, not on it.
        .default_pos(pos2(760.0, 60.0))
        .resizable(false)
        .show(ctx, |ui| {
            studio_style(ui);
            studio_header(ui, &format!("Panner · {name}"), &mut open);
            let can_be = !matches!(
                p.tracks[track].kind,
                crate::model::TrackKind::Folder | crate::model::TrackKind::Vca
            );
            // As DAWs do: a track stays in the bed with a stereo pan, is panned around the bed
            // (surround), or is an object with its own place in a spatial file.
            let now = match (p.tracks[track].object.is_some(), p.tracks[track].bed_pan) {
                (false, _) => 0,
                (true, true) => 1,
                (true, false) => 2,
            };
            let mut chosen = now;
            ui.add_enabled_ui(can_be, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Output");
                    ui.selectable_value(&mut chosen, 0, "Stereo")
                        .on_hover_text("In the bed, on its left and right only, with the track's own pan");
                    ui.selectable_value(&mut chosen, 1, "Surround Pan")
                        .on_hover_text("In the bed, panned anywhere around and above. A spatial file mixes it into the bed's channels");
                    ui.selectable_value(&mut chosen, 2, "Object")
                        .on_hover_text("An object: its own place, and its own tracks and moves in a spatial file");
                });
            });
            if chosen != now {
                let t = &mut p.tracks[track];
                if chosen == 0 {
                    t.object = None;
                    t.bed_pan = false;
                } else {
                    t.object = Some(t.object.unwrap_or_default());
                    t.bed_pan = chosen == 1;
                }
            }
            if p.spatial_mode == SpatialMode::Off && p.tracks[track].object.is_some() {
                ui.label(egui::RichText::new("ⓘ Heard on headphones").small().color(TEXT_DIM)).on_hover_text(
                    "This project is stereo: spatial tracks are rendered for headphones on their own and the rest of \
                     the mix stays as it is. For speakers or a spatial file, turn on the Spatial Renderer.",
                );
            }
            // Routed to a spatial bus, it is placed among that bus's speakers.
            let through = p.tracks[track]
                .object
                .and_then(|o| usize::from(o.bus).checked_sub(1))
                .and_then(|k| p.spatial_buses.get(k))
                .map(|b| b.layout);
            let layout = through.unwrap_or_else(|| p.spatial_mode.layout().unwrap_or(p.spatial_bed));
            if let Some(l) = through {
                ui.label(format!("Placed in the {} bus's layout", l.name()));
            }
            top_view(ui, p, track, layout);
            if let Some(o) = p.tracks.get(track).and_then(|t| place_now(t, p.position_beats)) {
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new(format!("Direction {:.0}°", o.azimuth)).small().color(TEXT_DIM));
                });
            }
            side_view(ui, p, track);
            if let Some(o) = p.tracks.get(track).and_then(|t| place_now(t, p.position_beats)) {
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new(format!("Elevation {:.0}°", o.elevation)).small().color(TEXT_DIM));
                });
            }
            let beat = p.position_beats;
            let Some(o) = p.tracks.get(track).and_then(|t| place_now(t, beat)) else {
                return;
            };
            // Size and spread stay out of sight under More, as the views set the rest.
            egui::CollapsingHeader::new("More")
                .id_salt(egui::Id::unique("panner_more"))
                .default_open(false)
                .show(ui, |ui| {
            egui::Grid::new(egui::Id::unique("panner_values")).num_columns(2).show(ui, |ui| {
                // Direction and elevation are set by dragging in the views above, as on other
                // panners; only the size has a slider of its own.
                {
                    let axis = ObjectAxis::Size;
                    ui.label("Size");
                    let (lo, hi) = axis.range();
                    let mut value = o.get(axis);
                    let resp = ui.add(egui::Slider::new(&mut value, lo..=hi).fixed_decimals(2));
                    if resp.changed() {
                        move_axis(p, track, axis, value);
                    }
                    if resp.drag_stopped() {
                        release_axes(p, track, &[axis]);
                    }
                    ui.end_row();
                }
                let mut spread = o.spread;
                ui.label("Spread");
                // Straight up, left and right meet over the head: spread has nothing to move.
                let overhead = o.elevation > 85.0;
                if ui
                    .add_enabled(!overhead, egui::Slider::new(&mut spread, 0.0..=1.0).fixed_decimals(2))
                    .on_hover_text("How far apart its left and right are")
                    .on_disabled_hover_text("Straight overhead, left and right meet: lower the elevation to spread them")
                    .changed()
                {
                    if let Some(object) = p.tracks[track].object.as_mut() {
                        object.spread = spread;
                    }
                }
                ui.end_row();
            });
                if !p.tracks[track].bed_pan {
                    // As Logic: each object can sound nearer or further on headphones than its channels.
                    ui.horizontal(|ui| {
                        ui.label("Headphone distance");
                        let now = o.distance;
                        let name = now.map_or("As its channels", |d| d.name());
                        egui::ComboBox::from_id_salt("object_distance")
                            .selected_text(name)
                            .show_ui(ui, |ui| {
                                let mut pick = now;
                                ui.selectable_value(&mut pick, None, "As its channels");
                                for d in ovrtone_engine::Distance::ALL {
                                    ui.selectable_value(&mut pick, Some(d), d.name());
                                }
                                if pick != now {
                                    if let Some(obj) = p.tracks[track].object.as_mut() {
                                        obj.distance = pick;
                                    }
                                }
                            });
                    });
                }
                egui::Grid::new(egui::Id::unique("panner_more_values")).num_columns(2).show(ui, |ui| {
                    for (axis, label) in [
                        (ObjectAxis::Distance, "Distance"),
                        (ObjectAxis::Divergence, "Divergence"),
                        (ObjectAxis::Lfe, "LFE level"),
                        (ObjectAxis::X, "X (left, right)"),
                        (ObjectAxis::Y, "Y (back, front)"),
                        (ObjectAxis::Z, "Z (ear, ceiling)"),
                    ] {
                        ui.label(label);
                        let (lo, hi) = axis.range();
                        let mut value = o.get(axis);
                        let resp = ui.add(egui::Slider::new(&mut value, lo..=hi).fixed_decimals(2));
                        if resp.changed() {
                            move_axis(p, track, axis, value);
                        }
                        if resp.drag_stopped() {
                            release_axes(p, track, &[axis]);
                        }
                        ui.end_row();
                    }
                    ui.label("Goes to");
                    let names: Vec<String> = p.spatial_buses.iter().map(|b| format!("{} ({})", b.name, b.layout.name())).collect();
                    let now = usize::from(o.bus);
                    let shown = if now == 0 {
                        "Main bus".to_string()
                    } else {
                        names.get(now - 1).cloned().unwrap_or_else(|| "Main bus".to_string())
                    };
                    egui::ComboBox::from_id_salt(egui::Id::unique("object_bus"))
                        .selected_text(shown)
                        .show_ui(ui, |ui| {
                            let mut pick = now;
                            ui.selectable_value(&mut pick, 0, "Main bus");
                            for (k, name) in names.iter().enumerate() {
                                ui.selectable_value(&mut pick, k + 1, name);
                            }
                            if pick != now {
                                if let Some(object) = p.tracks[track].object.as_mut() {
                                    object.bus = pick as u8;
                                }
                            }
                        })
                        .response
                        .on_hover_text("A spatial bus: the track is placed in the bus's own layout, and the bus's panner places the whole bus in the room");
                    ui.end_row();
                    let mut snap = o.snap;
                    ui.label("Snap");
                    if ui
                        .checkbox(&mut snap, "To the nearest speaker")
                        .on_hover_text("Plays on one speaker only, the one nearest to where it is")
                        .changed()
                    {
                        if let Some(object) = p.tracks[track].object.as_mut() {
                            object.snap = snap;
                        }
                    }
                    ui.end_row();
                    ui.label("Centre level");
                    ui.horizontal(|ui| {
                        let mut center = o.center;
                        let mut leave_out = center <= 0.0;
                        let slid = ui.add(egui::Slider::new(&mut center, 0.0..=1.0)).changed();
                        let ticked = ui.checkbox(&mut leave_out, "No centre").changed();
                        let value = if ticked {
                            if leave_out { 0.0 } else { 1.0 }
                        } else {
                            center
                        };
                        if slid || ticked {
                            if let Some(object) = p.tracks[track].object.as_mut() {
                                object.center = value;
                            }
                        }
                    });
                    ui.end_row();
                });
                ui.label("The track's automation mode applies: Write, Touch and Latch record each control you move. X, Y and Z move Direction, Elevation and Distance together.");
            });
        });
    s.spatial.panner_open = open;
}

/// The windows that are open.
pub fn show(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    if s.spatial.open {
        renderer(ctx, p, s);
    }
    if s.spatial.panner_open {
        panner(ctx, p, s);
    }
    if s.spatial.speakers_open {
        let layout = p.spatial_mode.output_layout().unwrap_or(p.spatial_bed);
        crate::speaker_setup::show(
            ctx,
            &mut s.spatial.speakers_open,
            layout,
            &mut p.spatial_test,
        );
    }
    if s.spatial.open || s.spatial.panner_open {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_top_view_and_the_places_agree() {
        for (az, el) in [(0.0, 0.0), (90.0, 0.0), (-135.0, 45.0), (30.0, 60.0)] {
            let (a, e) = place_at(point_of(az, el, 100.0), 100.0);
            assert!((a - az).abs() < 0.1, "{az} {a}");
            assert!((e - el).abs() < 0.1, "{el} {e}");
        }
        assert!(point_of(0.0, 0.0, 100.0).y < 0.0, "ahead is up");
        assert!(point_of(90.0, 0.0, 100.0).x > 0.0, "right is right");
    }

    #[test]
    fn an_output_keeps_the_bed_it_is_given() {
        for o in Output::ALL {
            let mode = o.with(SpeakerLayout::Surround712);
            assert_eq!(Output::of(mode), o);
            if o != Output::Off {
                assert_eq!(mode.layout(), Some(SpeakerLayout::Surround712));
            }
        }
    }
}
