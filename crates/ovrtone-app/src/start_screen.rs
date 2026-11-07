//! Project Chooser, shown when the program starts and by File > New from Template…: start a new
//! project with its tempo, key and audio devices set, reopen a recent one from a picture of it, or
//! start from a template of your own. Choosing queues a File command (`UiState::file_cmd`), so
//! unsaved changes are asked about like everywhere else.
use crate::file_commands::FileCmd;
use crate::i18n::tr;
use crate::model::{Project, Track, TrackKind};
use crate::track_icons::{draw_icon, IconKind};
use crate::UiState;
use egui::{pos2, vec2, Align2, Color32, FontId, Key, Rect, Sense, Stroke, StrokeKind};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// A project to start from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Template {
    /// One software instrument track (File > New).
    Empty,
    Songwriter,
    Electronic,
    Podcast,
}

impl Template {
    pub const ALL: [Template; 4] = [
        Template::Empty,
        Template::Songwriter,
        Template::Electronic,
        Template::Podcast,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Template::Empty => "Empty Project",
            Template::Songwriter => "Songwriter",
            Template::Electronic => "Electronic",
            Template::Podcast => "Podcast",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Template::Empty => "One software instrument track.",
            Template::Songwriter => "Piano, drummer, bass and vocal tracks.",
            Template::Electronic => "Synths, drum machine and a reverb aux.",
            Template::Podcast => "Two voice tracks and a music bed.",
        }
    }

    fn icon(self) -> IconKind {
        match self {
            Template::Empty => IconKind::Note,
            Template::Songwriter => IconKind::Piano,
            Template::Electronic => IconKind::Synth,
            Template::Podcast => IconKind::Mic,
        }
    }

    /// The new, untitled project this template starts.
    pub fn project(self) -> Project {
        let track = |name: &str, kind| Track::new(name, kind);
        let mut p = Project::empty();
        match self {
            Template::Empty => return p,
            Template::Songwriter => {
                p.tracks = vec![
                    track("Keys", TrackKind::Instrument),
                    track("Drums", TrackKind::Drummer),
                    track("Bass", TrackKind::Audio),
                    track("Lead Vocal", TrackKind::Audio),
                ];
            }
            Template::Electronic => {
                p.tempo = 124.0;
                p.tracks = vec![
                    track("Drums", TrackKind::Drummer),
                    track("Bass", TrackKind::Instrument),
                    track("Pad", TrackKind::Instrument),
                    track("Reverb", TrackKind::Aux),
                ];
            }
            Template::Podcast => {
                p.tracks = vec![
                    track("Host", TrackKind::Audio),
                    track("Guest", TrackKind::Audio),
                    track("Music Bed", TrackKind::Audio),
                ];
            }
        }
        p.name = "Untitled".into();
        p.selected_track = 0;
        p
    }
}

/// Sidebar sections of the chooser.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    NewProject,
    Recent,
    Templates,
}

impl Section {
    const ALL: [Section; 3] = [Section::NewProject, Section::Recent, Section::Templates];

    fn label(self) -> &'static str {
        match self {
            Section::NewProject => tr("New Project"),
            Section::Recent => tr("Recent"),
            Section::Templates => tr("Project Templates"),
        }
    }
}

/// The settings of a project about to be started. Only what was changed here is applied, so a
/// template keeps its own tempo and key otherwise.
#[derive(Clone, Debug, PartialEq)]
pub struct Details {
    tempo: f32,
    key: &'static str,
    sig: (u8, u8),
    smpte: ovrtone_engine::SmpteRate,
    tempo_set: bool,
    key_set: bool,
    sig_set: bool,
    smpte_set: bool,
    /// The input's id, `None` for the system's own; the output's name, empty for the system's own.
    input: Option<String>,
    output: String,
    rate: Option<u32>,
}

impl Details {
    fn of(p: &Project) -> Self {
        Self {
            tempo: p.tempo,
            key: p.key,
            sig: p.sig,
            smpte: p.smpte_rate,
            tempo_set: false,
            key_set: false,
            sig_set: false,
            smpte_set: false,
            input: None,
            output: String::new(),
            rate: None,
        }
    }

    /// Sets on `p` what was changed in the panel.
    pub fn apply(&self, p: &mut Project) {
        if self.tempo_set {
            p.tempo = self.tempo;
        }
        if self.key_set {
            p.key = self.key;
        }
        if self.sig_set {
            p.sig = self.sig;
        }
        if self.smpte_set {
            p.smpte_rate = self.smpte;
        }
    }
}

/// State of the open chooser.
#[derive(Clone, Debug, PartialEq)]
pub struct Chooser {
    pub section: Section,
    /// Highlighted tile in the section.
    pub selected: usize,
    details: Details,
    details_open: bool,
    /// The times Tap Tempo was pressed.
    taps: Vec<Instant>,
    /// The audio devices have been asked for.
    looked: bool,
}

impl Chooser {
    fn new(section: Section) -> Self {
        Self {
            section,
            selected: 0,
            details: Details::of(&Template::Empty.project()),
            details_open: true,
            taps: Vec::new(),
            looked: false,
        }
    }

    /// File > New from Template…: over the open project, which Cancel goes back to.
    pub fn templates() -> Self {
        Self::new(Section::NewProject)
    }

    /// When the program starts: what was worked on last, if there is any.
    pub fn at_launch(has_recent: bool) -> Self {
        Self::new(if has_recent {
            Section::Recent
        } else {
            Section::NewProject
        })
    }
}

/// The devices found for the panel, by a thread: asking a driver can take a while.
#[derive(Default)]
pub struct FoundDevices {
    outputs: Arc<Mutex<Vec<String>>>,
}

/// One thing that can be chosen, as a tile.
#[derive(Clone, Debug, PartialEq)]
struct Item {
    name: String,
    about: String,
    icon: Option<IconKind>,
    choice: Choice,
}

#[derive(Clone, Debug, PartialEq)]
enum Choice {
    Template(Template),
    Recent(PathBuf),
    UserTemplate(PathBuf),
}

impl Choice {
    /// The project file with a picture, if it has one.
    fn file(&self) -> Option<&PathBuf> {
        match self {
            Choice::Template(_) => None,
            Choice::Recent(path) | Choice::UserTemplate(path) => Some(path),
        }
    }
}

/// What a section offers.
fn items(section: Section, recent: &[PathBuf], mine: &[(String, PathBuf)]) -> Vec<Item> {
    match section {
        Section::NewProject => Template::ALL
            .iter()
            .map(|t| Item {
                name: tr(t.name()).to_string(),
                about: tr(t.about()).to_string(),
                icon: Some(t.icon()),
                choice: Choice::Template(*t),
            })
            .collect(),
        Section::Recent => recent
            .iter()
            .map(|path| Item {
                name: path
                    .file_stem()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?")
                    .to_string(),
                about: path.display().to_string(),
                icon: None,
                choice: Choice::Recent(path.clone()),
            })
            .collect(),
        Section::Templates => mine
            .iter()
            .map(|(name, path)| Item {
                name: name.clone(),
                about: name.clone(),
                icon: None,
                choice: Choice::UserTemplate(path.clone()),
            })
            .collect(),
    }
}

/// Where File > Save as Template puts templates.
pub fn template_folder() -> Option<PathBuf> {
    Some(crate::recent_files::config_dir()?.join("templates"))
}

/// The project files in `dir`: name and file, by name.
fn templates_in(dir: &std::path::Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<(String, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| ovrtone_project::is_project_file(p))
        .filter_map(|p| Some((p.file_stem()?.to_str()?.to_string(), p)))
        .collect();
    out.sort();
    out
}

/// The templates the user saved.
fn user_templates() -> Vec<(String, PathBuf)> {
    if cfg!(test) {
        return Vec::new();
    }
    template_folder().map_or_else(Vec::new, |dir| templates_in(&dir))
}

/// Queues the File command for `choice`, with what the panel says for a new project.
fn choose(choice: Choice, details: &Details, s: &mut UiState) {
    match choice {
        Choice::Template(t) => {
            s.new_details = Some(details.clone());
            set_audio(details, s);
            s.file_cmd = Some(FileCmd::NewFrom(t));
        }
        Choice::UserTemplate(path) => {
            s.new_details = Some(details.clone());
            set_audio(details, s);
            s.recent_pick = Some(path);
            s.file_cmd = Some(FileCmd::NewFromFile);
        }
        Choice::Recent(path) => {
            s.recent_pick = Some(path);
            s.file_cmd = Some(FileCmd::OpenRecent);
        }
    }
}

/// Makes the panel's devices and sample rate the ones in use, if they are not already.
fn set_audio(details: &Details, s: &mut UiState) {
    s.mixer.inputs.pick(details.input.clone());
    let saved = crate::settings::AudioSetup::load();
    if saved.device != details.output || saved.rate != details.rate {
        let mut setup = saved;
        setup.device.clone_from(&details.output);
        setup.rate = details.rate;
        setup.save();
        s.settings.setup = setup.clone();
        s.settings.apply = Some(setup);
    }
}

const SCRIM: Color32 = Color32::from_rgb(0x12, 0x12, 0x14);
const BG: Color32 = Color32::from_rgb(0x1f, 0x20, 0x23);
const SIDEBAR: Color32 = Color32::from_rgb(0x28, 0x29, 0x2c);
const TEXT: Color32 = Color32::from_rgb(0xe6, 0xe6, 0xe6);
const DIM: Color32 = Color32::from_rgb(0x96, 0x96, 0x9a);
const TILE: Color32 = Color32::from_rgb(0x3a, 0x3a, 0x3d);
const LINE: Color32 = Color32::from_rgb(0x38, 0x39, 0x3c);
const TILE_W: f32 = 168.0;
const PICTURE_H: f32 = 105.0;
const CELL_H: f32 = PICTURE_H + 46.0;
const DETAILS_H: f32 = 270.0;

/// A tile: the picture or icon, and the name under it. Returns (clicked, double-clicked).
fn tile(ui: &mut egui::Ui, item: &Item, selected: bool, key: usize) -> (bool, bool) {
    let (r, _) = ui.allocate_exact_size(vec2(TILE_W, CELL_H), Sense::hover());
    let resp = ui.interact(r, egui::Id::unique(("chooser_tile", key)), Sense::click());
    let picture = Rect::from_min_size(r.min, vec2(TILE_W, PICTURE_H));
    let thumb = item
        .choice
        .file()
        .and_then(|path| crate::project_thumbs::texture(ui.ctx(), path));
    let g = ui.painter();
    match thumb {
        Some(thumb) => {
            g.image(
                thumb.id(),
                picture,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            let fill = if resp.hovered() {
                TILE.lerp_to_gamma(Color32::WHITE, 0.06)
            } else {
                TILE
            };
            g.rect_filled(picture, 6.0, fill);
            let icon = Rect::from_center_size(picture.center(), vec2(52.0, 52.0));
            draw_icon(g, icon, item.icon.unwrap_or(IconKind::Note));
        }
    }
    if selected {
        g.rect_stroke(
            picture,
            6.0,
            Stroke::new(2.0, crate::theme::accent()),
            StrokeKind::Outside,
        );
    }
    let name = g.layout_no_wrap(
        item.name.clone(),
        FontId::proportional(13.0),
        if selected {
            crate::theme::on_accent()
        } else {
            TEXT
        },
    );
    let width = name.size().x.min(TILE_W);
    let at = pos2(r.center().x - width / 2.0, picture.bottom() + 11.0);
    if selected {
        let pill = Rect::from_min_size(at, vec2(width, name.size().y)).expand2(vec2(7.0, 2.0));
        g.rect_filled(pill, 4.0, crate::theme::accent());
    }
    g.with_clip_rect(r.expand2(vec2(8.0, 0.0)))
        .galley(at, name, TEXT);
    (resp.clicked(), resp.double_clicked())
}

/// A key's name as the panel shows it: `F#min` is F# Minor.
fn key_label(key: &str) -> String {
    if let Some(root) = key.strip_suffix("maj") {
        format!("{root} Major")
    } else if let Some(root) = key.strip_suffix("min") {
        format!("{root} Minor")
    } else {
        key.to_string()
    }
}

/// The tempo the last few taps add up to, once there are enough to say.
fn tapped_tempo(taps: &[Instant]) -> Option<f32> {
    let (first, last) = (taps.first()?, taps.last()?);
    let seconds = last.duration_since(*first).as_secs_f32();
    (taps.len() >= 2 && seconds > 0.0)
        .then(|| (60.0 * (taps.len() - 1) as f32 / seconds).clamp(20.0, 999.0))
}

/// The panel of settings for a new project.
fn details_panel(ui: &mut egui::Ui, c: &mut Chooser, s: &mut UiState) {
    let d = &mut c.details;
    egui::Grid::new(egui::Id::unique("chooser_details"))
        .num_columns(2)
        .spacing([10.0, 7.0])
        .show(ui, |ui| {
            ui.label("Tempo");
            ui.horizontal(|ui| {
                let tempo = egui::DragValue::new(&mut d.tempo)
                    .range(20.0..=999.0)
                    .speed(0.2)
                    .fixed_decimals(1);
                d.tempo_set |= ui.add(tempo).changed();
                if ui.button("Tap Tempo").clicked() {
                    let now = Instant::now();
                    // A long gap starts over.
                    if c.taps
                        .last()
                        .is_some_and(|t| now.duration_since(*t).as_secs_f32() > 2.0)
                    {
                        c.taps.clear();
                    }
                    c.taps.push(now);
                    if c.taps.len() > 8 {
                        c.taps.remove(0);
                    }
                    if let Some(tempo) = tapped_tempo(&c.taps) {
                        d.tempo = (tempo * 10.0).round() / 10.0;
                        d.tempo_set = true;
                    }
                }
            });
            ui.end_row();

            ui.label("Key");
            egui::ComboBox::from_id_salt(egui::Id::unique("chooser_key"))
                .selected_text(key_label(d.key))
                .height(260.0)
                .show_ui(ui, |ui| {
                    for (key, _) in crate::score::KEYS {
                        if ui.selectable_label(d.key == key, key_label(key)).clicked() {
                            d.key = key;
                            d.key_set = true;
                        }
                    }
                });
            ui.end_row();

            ui.label("Time signature");
            ui.horizontal(|ui| {
                let beats = egui::DragValue::new(&mut d.sig.0).range(1..=32).speed(0.1);
                d.sig_set |= ui.add(beats).changed();
                ui.label("/");
                egui::ComboBox::from_id_salt(egui::Id::unique("chooser_unit"))
                    .selected_text(d.sig.1.to_string())
                    .width(50.0)
                    .show_ui(ui, |ui| {
                        for unit in [2u8, 4, 8, 16] {
                            if ui
                                .selectable_label(d.sig.1 == unit, unit.to_string())
                                .clicked()
                            {
                                d.sig.1 = unit;
                                d.sig_set = true;
                            }
                        }
                    });
            });
            ui.end_row();

            ui.label("Input device");
            let shown = d.input.as_ref().map_or("System default".to_string(), |id| {
                s.mixer
                    .inputs
                    .devices
                    .iter()
                    .find(|x| x.id == *id)
                    .map_or_else(|| id.clone(), |x| x.name.clone())
            });
            egui::ComboBox::from_id_salt(egui::Id::unique("chooser_input"))
                .selected_text(shown)
                .width(260.0)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(d.input.is_none(), "System default")
                        .clicked()
                    {
                        d.input = None;
                    }
                    for device in &s.mixer.inputs.devices {
                        let on = d.input.as_deref() == Some(device.id.as_str());
                        if ui.selectable_label(on, &device.name).clicked() {
                            d.input = Some(device.id.clone());
                        }
                    }
                });
            ui.end_row();

            ui.label("Output device");
            let outputs = s
                .chooser_devices
                .outputs
                .lock()
                .map(|o| o.clone())
                .unwrap_or_default();
            let shown = if d.output.is_empty() {
                "System default".to_string()
            } else {
                d.output.clone()
            };
            egui::ComboBox::from_id_salt(egui::Id::unique("chooser_output"))
                .selected_text(shown)
                .width(260.0)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(d.output.is_empty(), "System default")
                        .clicked()
                    {
                        d.output.clear();
                    }
                    for device in outputs {
                        let on = d.output == device;
                        if ui.selectable_label(on, &device).clicked() {
                            d.output = device;
                        }
                    }
                });
            ui.end_row();

            ui.label("Sample rate");
            let shown = d.rate.map_or("Device default".to_string(), |r| {
                format!("{} kHz", f64::from(r) / 1000.0)
            });
            egui::ComboBox::from_id_salt(egui::Id::unique("chooser_rate"))
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(d.rate.is_none(), "Device default")
                        .clicked()
                    {
                        d.rate = None;
                    }
                    for rate in crate::settings::RATES {
                        let text = format!("{} kHz", f64::from(rate) / 1000.0);
                        if ui.selectable_label(d.rate == Some(rate), text).clicked() {
                            d.rate = Some(rate);
                        }
                    }
                });
            ui.end_row();

            ui.label("Frame rate");
            egui::ComboBox::from_id_salt(egui::Id::unique("chooser_fps"))
                .selected_text(crate::sync_menu::rate_label(d.smpte))
                .show_ui(ui, |ui| {
                    for rate in ovrtone_engine::SmpteRate::ALL {
                        let text = crate::sync_menu::rate_label(rate);
                        if ui.selectable_label(d.smpte == rate, text).clicked() {
                            d.smpte = rate;
                            d.smpte_set = true;
                        }
                    }
                });
            ui.end_row();
        });
}

/// Asks for the audio devices once, off this thread for the outputs.
fn look_for_devices(c: &mut Chooser, s: &mut UiState, ctx: &egui::Context) {
    if c.looked || cfg!(test) {
        return;
    }
    c.looked = true;
    s.mixer.inputs.refresh();
    let setup = crate::settings::AudioSetup::load();
    c.details.input.clone_from(&s.mixer.inputs.device);
    c.details.output.clone_from(&setup.device);
    c.details.rate = setup.rate;
    let (found, ctx) = (s.chooser_devices.outputs.clone(), ctx.clone());
    let _ = std::thread::Builder::new()
        .name("ovrtone-chooser-devices".into())
        .spawn(move || {
            let list = ovrtone_engine::Engine::audio_devices(&setup.system);
            if let Ok(mut outputs) = found.lock() {
                *outputs = list;
            }
            ctx.request_repaint();
        });
}

/// Draws the chooser over the whole window.
pub fn show(ui: &mut egui::Ui, s: &mut UiState) {
    let Some(mut c) = s.chooser.clone() else {
        return;
    };
    let ctx = ui.ctx().clone();
    // Its own dark panel, whatever the rest of the program looks like: buttons and menus included.
    ui.style_mut().visuals = egui::Visuals::dark();
    look_for_devices(&mut c, s, &ctx);
    let area = ui.available_rect_before_wrap();
    ui.allocate_rect(area, Sense::hover());
    ui.painter().rect_filled(area, 0.0, SCRIM);
    let panel = Rect::from_center_size(
        area.center(),
        vec2(
            area.width().min(1060.0) - 24.0,
            area.height().min(900.0) - 24.0,
        ),
    );
    let g = ui.painter().clone();
    g.rect_filled(panel, 10.0, BG);

    // Sidebar.
    let side = Rect::from_min_size(panel.min, vec2(210.0, panel.height()));
    g.rect_filled(
        side,
        egui::CornerRadius {
            nw: 10,
            sw: 10,
            ne: 0,
            se: 0,
        },
        SIDEBAR,
    );
    match crate::quick_dialogs::wordmark(&ctx, true) {
        Some(logo) => {
            // The artwork has room around the letters, so it starts left of the sections' text.
            let size = logo.size_vec2() * (150.0 / logo.size_vec2().x);
            g.image(
                logo.id(),
                Rect::from_min_size(side.left_top() + vec2(6.0, 30.0 - size.y / 2.0), size),
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            g.text(
                side.left_top() + vec2(20.0, 30.0),
                Align2::LEFT_CENTER,
                "Ovrtone",
                FontId::proportional(18.0),
                TEXT,
            );
        }
    }
    let recent: Vec<PathBuf> = s.recent.paths().to_vec();
    let mine = user_templates();
    for (k, sec) in Section::ALL.into_iter().enumerate() {
        let r = Rect::from_min_size(
            side.left_top() + vec2(10.0, 66.0 + k as f32 * 30.0),
            vec2(side.width() - 20.0, 26.0),
        );
        let resp = ui.interact(r, egui::Id::unique(("chooser_section", k)), Sense::click());
        if c.section == sec {
            g.rect_filled(r, 5.0, Color32::from_white_alpha(26));
        } else if resp.hovered() {
            g.rect_filled(r, 5.0, Color32::from_white_alpha(12));
        }
        g.text(
            r.left_center() + vec2(10.0, 0.0),
            Align2::LEFT_CENTER,
            sec.label(),
            FontId::proportional(13.0),
            TEXT,
        );
        if resp.clicked() && c.section != sec {
            c.section = sec;
            c.selected = 0;
        }
    }

    // The tiles, the panel under them, the buttons at the bottom.
    let main = Rect::from_min_max(pos2(side.right(), panel.top()), panel.max);
    let footer_h = 54.0;
    let title_h = 34.0;
    g.text(
        pos2(main.center().x, main.top() + title_h / 2.0),
        Align2::CENTER_CENTER,
        "Start or open a project",
        FontId::proportional(13.0),
        DIM,
    );
    g.line_segment(
        [
            pos2(main.left(), main.top() + title_h),
            pos2(main.right(), main.top() + title_h),
        ],
        Stroke::new(1.0, LINE),
    );
    let list = items(c.section, &recent, &mine);
    let takes_details = c.section != Section::Recent;
    let bar_h = 34.0;
    let panel_h = if takes_details && c.details_open {
        (main.height() - title_h - footer_h - bar_h - CELL_H - 30.0).clamp(0.0, DETAILS_H)
    } else {
        0.0
    };
    let footer = Rect::from_min_max(pos2(main.left(), main.bottom() - footer_h), main.max);
    let details = Rect::from_min_max(
        pos2(main.left(), footer.top() - panel_h),
        pos2(main.right(), footer.top()),
    );
    let bar = Rect::from_min_max(
        pos2(main.left(), details.top() - bar_h),
        pos2(main.right(), details.top()),
    );
    let tiles = Rect::from_min_max(
        pos2(main.left(), main.top() + title_h),
        pos2(main.right(), bar.top()),
    );

    let mut chosen = None;
    let before = c.selected;
    ui.scope_builder(egui::UiBuilder::new().max_rect(tiles.shrink(14.0)), |ui| {
        egui::ScrollArea::vertical()
            .id_salt(egui::Id::unique("chooser_tiles"))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if list.is_empty() {
                    let words = if c.section == Section::Recent {
                        tr("No recent projects. Start one from New Project or Project Templates.")
                    } else {
                        "No templates of your own yet. Save one with File > Save as Template."
                    };
                    ui.add_space(8.0);
                    ui.colored_label(DIM, words);
                }
                // Room for the highlight around the first row and column.
                ui.add_space(4.0);
                ui.spacing_mut().item_spacing = vec2(26.0, 10.0);
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(4.0);
                    for (k, item) in list.iter().enumerate() {
                        let (clicked, double) = tile(ui, item, c.selected == k, k);
                        if clicked {
                            c.selected = k;
                        }
                        if double {
                            c.selected = k;
                            chosen = Some(item.choice.clone());
                        }
                    }
                });
            });
    });
    // A different template brings its own tempo and key, and forgets what was set for the last.
    if c.selected != before {
        if let Some(Choice::Template(t)) = list.get(c.selected).map(|i| &i.choice) {
            let kept = c.details.clone();
            c.details = Details {
                input: kept.input,
                output: kept.output,
                rate: kept.rate,
                ..Details::of(&t.project())
            };
        }
    }

    // The bar over the panel: says what is chosen, and folds the panel.
    let g = ui.painter().clone();
    g.line_segment([bar.left_top(), bar.right_top()], Stroke::new(1.0, LINE));
    g.line_segment(
        [bar.left_bottom(), bar.right_bottom()],
        Stroke::new(1.0, LINE),
    );
    let about = list.get(c.selected).map_or("", |i| i.about.as_str());
    g.with_clip_rect(bar).text(
        bar.center(),
        Align2::CENTER_CENTER,
        about,
        FontId::proportional(12.5),
        DIM,
    );
    if takes_details {
        let fold = Rect::from_min_size(bar.min, vec2(90.0, bar_h));
        let resp = ui.interact(fold, egui::Id::unique("chooser_fold"), Sense::click());
        let at = fold.left_center() + vec2(18.0, 0.0);
        let mark = if c.details_open {
            vec![
                at + vec2(-4.0, -2.5),
                at + vec2(4.0, -2.5),
                at + vec2(0.0, 3.5),
            ]
        } else {
            vec![
                at + vec2(-2.5, -4.0),
                at + vec2(3.5, 0.0),
                at + vec2(-2.5, 4.0),
            ]
        };
        g.add(egui::Shape::convex_polygon(mark, TEXT, Stroke::NONE));
        g.text(
            fold.left_center() + vec2(28.0, 0.0),
            Align2::LEFT_CENTER,
            "Details",
            FontId::proportional(12.5),
            TEXT,
        );
        if resp.clicked() {
            c.details_open = !c.details_open;
        }
    }
    if panel_h > 0.0 {
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(details.shrink2(vec2(40.0, 12.0))),
            |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(egui::Id::unique("chooser_details_scroll"))
                    .auto_shrink([false, false])
                    .show(ui, |ui| details_panel(ui, &mut c, s));
            },
        );
    }

    // Footer: Open… on the left; Cancel and Choose on the right.
    ui.painter().line_segment(
        [footer.left_top(), footer.right_top()],
        Stroke::new(1.0, LINE),
    );
    let mut cancel = false;
    let picked = || list.get(c.selected).map(|i| i.choice.clone());
    let footer_ui = footer.shrink2(vec2(20.0, 11.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(footer_ui), |ui| {
        ui.horizontal_centered(|ui| {
            if ui.button(tr("Open an existing project…")).clicked() {
                s.file_cmd = Some(FileCmd::Open);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let choose_btn = egui::Button::new(
                    egui::RichText::new(tr("Choose")).color(crate::theme::on_accent()),
                )
                .fill(crate::theme::accent());
                if ui.add_enabled(picked().is_some(), choose_btn).clicked() {
                    chosen = picked();
                }
                if ui.button(tr("Cancel")).clicked() {
                    cancel = true;
                }
            });
        });
    });

    // Return chooses, Escape goes back to the open project.
    if !ui.ctx().egui_wants_keyboard_input() {
        if ui.input(|i| i.key_pressed(Key::Enter)) {
            chosen = chosen.or_else(picked);
        }
        if ui.input(|i| i.key_pressed(Key::Escape)) {
            cancel = true;
        }
    }

    if cancel {
        s.chooser = None;
        return;
    }
    // The chooser stays up until the File command has opened a project, so a cancelled
    // "save changes?" prompt or a failed open lands back here.
    let details = c.details.clone();
    s.chooser = Some(c);
    if let Some(choice) = chosen {
        choose(choice, &details, s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_start_untitled_projects_with_their_tracks() {
        for t in Template::ALL {
            let p = t.project();
            assert_eq!(p.name, "Untitled", "{t:?}");
            assert!(!p.tracks.is_empty(), "{t:?}");
            assert_eq!(p.selected_track, 0);
        }
        assert_eq!(Template::Empty.project().tracks.len(), 1);
        let kinds: Vec<_> = Template::Electronic
            .project()
            .tracks
            .iter()
            .map(|t| t.kind)
            .collect();
        assert!(kinds.contains(&TrackKind::Aux) && kinds.contains(&TrackKind::Drummer));
    }

    #[test]
    fn choosing_queues_the_matching_file_command() {
        let recent = vec![PathBuf::from("/music/Night Drive.ovrtone")];
        let mine = vec![("Mine".to_string(), PathBuf::from("/t/Mine.ovrtone"))];
        let new = items(Section::NewProject, &recent, &mine);
        assert_eq!(new[1].choice, Choice::Template(Template::Songwriter));
        let details = Details::of(&Project::empty());
        let mut s = UiState::new();
        let again = items(Section::Recent, &recent, &mine);
        choose(again[0].choice.clone(), &details, &mut s);
        assert_eq!(s.file_cmd, Some(FileCmd::OpenRecent));
        assert_eq!(s.recent_pick, Some(recent[0].clone()));
        assert!(
            s.new_details.is_none(),
            "nothing to set on a project opened"
        );

        assert!(again.get(3).is_none(), "nothing to choose past the list");
        let own = items(Section::Templates, &recent, &mine);
        choose(own[0].choice.clone(), &details, &mut s);
        assert_eq!(s.file_cmd, Some(FileCmd::NewFromFile));
        assert!(s.new_details.is_some());
    }

    #[test]
    fn it_opens_on_what_was_worked_on_last() {
        assert_eq!(Chooser::at_launch(true).section, Section::Recent);
        assert_eq!(Chooser::at_launch(false).section, Section::NewProject);
    }

    #[test]
    fn only_what_was_changed_is_set_on_the_new_project() {
        let mut d = Details::of(&Project::empty());
        d.tempo = 90.0;
        d.key = "F#min";
        let mut p = Template::Electronic.project();
        d.apply(&mut p);
        assert_eq!(p.tempo, 124.0, "not touched: the template's own");
        d.tempo_set = true;
        d.key_set = true;
        d.apply(&mut p);
        assert_eq!((p.tempo, p.key), (90.0, "F#min"));
    }

    #[test]
    fn tapping_makes_a_tempo_and_keys_read_well() {
        let at = Instant::now();
        let half = std::time::Duration::from_millis(500);
        assert_eq!(tapped_tempo(&[at]), None);
        let tempo = tapped_tempo(&[at, at + half, at + half * 2]).unwrap_or(0.0);
        assert!((tempo - 120.0).abs() < 0.01);
        assert_eq!(key_label("F#min"), "F# Minor");
        assert_eq!(key_label("Cmaj"), "C Major");
    }
}
