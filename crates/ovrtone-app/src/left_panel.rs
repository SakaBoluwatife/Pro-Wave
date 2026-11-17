//! The left panel's tabs: Library | Browser | Plugins.
//!
//! - **Library:** Logic's patch browser (`library.rs`), as before.
//! - **Browser:** your sample folders (`sample_browser.rs`).
//! - **Plugins:** the CLAP and VST3 plugins found on this computer, ready to go on a track.
use crate::model::Project;
use crate::plugin_scan::PluginScan;
use crate::widgets::{text_col, text_dim_col};
use crate::UiState;
use egui::{pos2, vec2, Align2, Color32, FontId, Rect, Sense, Stroke, Ui, UiBuilder};
use ovrtone_host::PluginInfo;

fn panel_bg_col() -> Color32 {
    crate::theme::panel_bg()
}
const ROW_H: f32 = 20.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LeftTab {
    #[default]
    Browser,
    Plugins,
    Presets,
    ProjectAudio,
}

#[cfg(test)]
impl LeftTab {
    const ALL: [LeftTab; 4] = [
        LeftTab::Browser,
        LeftTab::Plugins,
        LeftTab::Presets,
        LeftTab::ProjectAudio,
    ];

    fn name(self) -> &'static str {
        match self {
            LeftTab::Browser => "Browser",
            LeftTab::Plugins => "Plugins",
            LeftTab::Presets => "Presets",
            LeftTab::ProjectAudio => "Project Audio",
        }
    }
}

/// What a drag from the Plugins tab carries: the plugin to drop on a track.
#[derive(Clone, Debug)]
pub struct PluginDrag(pub PluginInfo);

/// The plugins found on this computer.
#[derive(Default)]
pub struct PluginList {
    pub clap: PluginScan,
    kind: PluginKind,
    search: String,
    show_folders: bool,
    /// The Ovrtone folder is folded away.
    stock_closed: bool,
    /// The saved chains and templates folder is folded away.
    chains_closed: bool,
    /// Only the starred plugins are listed.
    only_favourites: bool,
    /// How the list is ordered.
    order: Order,
}

/// How the Plugins list is ordered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Order {
    #[default]
    Name,
    Maker,
    Kind,
}

impl PluginList {
    /// Looks for plugins again; with `retry`, also tries the ones that failed.
    pub fn rescan(&mut self, retry: bool, ctx: &egui::Context) {
        self.clap.start(retry, ctx);
    }

    /// What has been found so far, with the scan started and its news taken in.
    pub fn installed(&mut self, ctx: &egui::Context) -> &[PluginInfo] {
        if !cfg!(test) {
            self.clap.ensure_started(ctx);
        }
        self.clap.drain();
        &self.clap.plugins
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum PluginKind {
    #[default]
    All,
    Instruments,
    Effects,
}

impl PluginKind {
    const ALL: [PluginKind; 3] = [
        PluginKind::All,
        PluginKind::Instruments,
        PluginKind::Effects,
    ];

    fn name(self) -> &'static str {
        match self {
            PluginKind::All => "All",
            PluginKind::Instruments => "Instruments",
            PluginKind::Effects => "Effects",
        }
    }

    /// Whether `info` is shown for this kind and this search, already in lower case.
    fn shows(self, info: &PluginInfo, search: &str) -> bool {
        let kind = match self {
            PluginKind::All => true,
            PluginKind::Instruments => info.is_instrument,
            PluginKind::Effects => !info.is_instrument,
        };
        kind && (info.name.to_lowercase().contains(search)
            || info.vendor.to_lowercase().contains(search))
    }
}

/// The width of the list of what to browse.
const SIDEBAR_W: f32 = 126.0;
/// The height of the Browser and Inspector tabs.
const TABS_H: f32 = 32.0;

/// The small picture beside a kind of thing to browse.
fn side_icon(g: &egui::Painter, c: egui::Pos2, which: usize, color: egui::Color32) {
    let line = Stroke::new(1.2, color);
    match which {
        // Files: a sheet with a corner folded.
        0 => {
            g.rect_stroke(
                Rect::from_center_size(c, vec2(9.0, 11.0)),
                1.5,
                line,
                egui::StrokeKind::Middle,
            );
            g.line_segment([c + vec2(-2.0, -1.5), c + vec2(2.5, -1.5)], line);
            g.line_segment([c + vec2(-2.0, 1.5), c + vec2(2.5, 1.5)], line);
        }
        // Instruments: a note.
        1 => {
            g.circle_stroke(c + vec2(-2.0, 3.5), 2.0, line);
            g.line_segment([c + vec2(0.0, 3.5), c + vec2(0.0, -5.0)], line);
            g.line_segment([c + vec2(0.0, -5.0), c + vec2(4.0, -3.0)], line);
        }
        // Effects: two sliders.
        2 => {
            for (x, y) in [(-2.5, 2.0), (2.5, -2.0)] {
                g.line_segment([c + vec2(x, -5.0), c + vec2(x, 5.0)], line);
                g.circle_filled(c + vec2(x, y), 2.0, color);
            }
        }
        // Plugins: a plug.
        _ => {
            g.rect_stroke(
                Rect::from_center_size(c + vec2(0.0, 0.5), vec2(8.0, 6.0)),
                1.5,
                line,
                egui::StrokeKind::Middle,
            );
            g.line_segment([c + vec2(-2.0, -2.5), c + vec2(-2.0, -5.5)], line);
            g.line_segment([c + vec2(2.0, -2.5), c + vec2(2.0, -5.5)], line);
            g.line_segment([c + vec2(0.0, 3.5), c + vec2(0.0, 6.0)], line);
        }
    }
}

/// The tabs along the top: the Browser is this panel, and the Inspector goes beside it.
fn tabs(ui: &mut Ui, s: &mut UiState, r: Rect) {
    let g = ui.painter_at(r);
    g.rect_filled(r, 0.0, crate::theme::panel_bg());
    for (i, name) in ["Browser", "Inspector"].into_iter().enumerate() {
        let tab = Rect::from_min_size(
            pos2(r.left() + 8.0 + i as f32 * 88.0, r.top() + 5.0),
            vec2(82.0, 22.0),
        );
        let resp = ui.interact(tab, egui::Id::unique(("left_tabs", i)), Sense::click());
        let on = i == 0 || s.show_inspector;
        if on {
            g.rect_filled(tab, 5.0, crate::theme::accent());
        } else if resp.hovered() {
            g.rect_filled(tab, 5.0, crate::theme::hovered());
        }
        g.text(
            tab.center(),
            Align2::CENTER_CENTER,
            name,
            FontId::proportional(12.5),
            if on { crate::theme::on_accent() } else { text_col() },
        );
        if resp.clicked() {
            if i == 0 {
                s.show_library = false;
            } else {
                s.show_inspector = !s.show_inspector;
            }
        }
    }
}

pub fn show(ui: &mut Ui, p: &mut Project, s: &mut UiState) {
    let whole = ui.available_rect_before_wrap();
    ui.allocate_rect(whole, Sense::hover());
    tabs(
        ui,
        s,
        Rect::from_min_size(whole.min, vec2(whole.width(), TABS_H)),
    );
    let panel = Rect::from_min_max(whole.min + vec2(0.0, TABS_H), whole.max);
    // One panel: what to browse on the left, and the browsing beside it.
    let side = Rect::from_min_size(panel.min, vec2(SIDEBAR_W, panel.height()));
    let rest = Rect::from_min_max(pos2(side.right(), panel.top()), panel.max);
    {
        let g = ui.painter_at(side);
        g.rect_filled(side, 0.0, crate::theme::header());
        g.line_segment(
            [side.right_top(), side.right_bottom()],
            Stroke::new(1.0, crate::theme::border()),
        );
    }
    let entries = [
        ("All Files", LeftTab::Browser, None),
        (
            "Instruments",
            LeftTab::Plugins,
            Some(PluginKind::Instruments),
        ),
        ("Effects", LeftTab::Plugins, Some(PluginKind::Effects)),
        ("All Plug-ins", LeftTab::Plugins, Some(PluginKind::All)),
        ("Presets", LeftTab::Presets, None),
        ("Project Audio", LeftTab::ProjectAudio, None),
    ];
    for (i, (name, tab, kind)) in entries.into_iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(side.left() + 4.0, side.top() + 8.0 + i as f32 * 28.0),
            vec2(SIDEBAR_W - 8.0, 26.0),
        );
        let resp = ui.interact(r, egui::Id::unique(("browser_side", i)), Sense::click());
        if resp.clicked() {
            s.left_tab = tab;
            if let Some(kind) = kind {
                s.plugins.kind = kind;
            }
        }
        let on = s.left_tab == tab && kind.is_none_or(|k| s.plugins.kind == k);
        let g = ui.painter_at(r);
        if on {
            g.rect_filled(r, 5.0, crate::theme::selected());
            g.rect_filled(
                Rect::from_min_size(r.min + vec2(0.0, 5.0), vec2(2.5, r.height() - 10.0)),
                1.0,
                crate::theme::accent(),
            );
        } else if resp.hovered() {
            g.rect_filled(r, 5.0, crate::theme::hovered());
        }
        let ink = if on {
            crate::theme::accent()
        } else {
            crate::theme::text_dim()
        };
        side_icon(&g, r.left_center() + vec2(15.0, 0.0), i, ink);
        g.text(
            r.left_center() + vec2(29.0, 0.0),
            Align2::LEFT_CENTER,
            name,
            FontId::proportional(12.5),
            text_col(),
        );
    }
    ui.scope_builder(UiBuilder::new().max_rect(rest), |ui| match s.left_tab {
        LeftTab::Browser => crate::sample_browser::show(ui, s),
        LeftTab::Plugins => plugins(ui, p, s),
        LeftTab::Presets => presets(ui, p, s),
        LeftTab::ProjectAudio => project_audio(ui, p, s),
    });
}

fn plugins(ui: &mut Ui, p: &mut Project, s: &mut UiState) {
    let panel = ui.available_rect_before_wrap();
    ui.allocate_rect(panel, Sense::hover());
    ui.painter_at(panel).rect_filled(panel, 0.0, panel_bg_col());
    let ctx = ui.ctx().clone();
    let list = &mut s.plugins;
    list.clap.ensure_started(&ctx);
    list.clap.drain();
    let mut add = None;
    let mut stock_add = None;
    let mut template_task: Option<(crate::file_commands::TemplateKind, std::path::PathBuf)> = None;
    let inner = panel.shrink2(vec2(8.0, 6.0));
    ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
        ui.horizontal(|ui| {
            let counts = format!("{} plugins", list.clap.plugins.len());
            ui.label(egui::RichText::new(counts).color(text_col()));
            let hint = "Look for new plugins and try the ones that failed again";
            if crate::icons::button(ui, crate::icons::Icon::Refresh, hint).clicked()
            {
                crate::plugin_guard::clear_blocked();
                list.clap.start(true, &ctx);
            }
            let hint = "Choose more folders to look for plugins in";
            if crate::icons::toggle(ui, crate::icons::Icon::Folder, list.show_folders, hint)
                .clicked()
            {
                list.show_folders = !list.show_folders;
            }
        });
        if list.show_folders {
            let mut rescan = false;
            for folder in crate::plugin_guard::plugin_folders() {
                ui.horizontal(|ui| {
                    if crate::icons::button(ui, crate::icons::Icon::Close, "Stop looking here")
                        .clicked()
                    {
                        crate::plugin_guard::remove_folder(&folder);
                        rescan = true;
                    }
                    ui.label(egui::RichText::new(folder.display().to_string()).small());
                });
            }
            if ui.small_button("Add Folder…").clicked() {
                if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                    crate::plugin_guard::add_folder(&folder);
                    rescan = true;
                }
            }
            if rescan {
                list.clap.start(false, &ctx);
            }
        }
        let note = |ui: &mut Ui, text: String| {
            ui.add(egui::Label::new(egui::RichText::new(text).small().color(text_dim_col())).wrap())
        };
        note(
            ui,
            "Double-click a plugin to add it to the selected track, drag it onto a track, or click an empty mixer slot."
                .to_string(),
        );
        if let Some((done, of)) = list.clap.progress {
            note(ui, format!("Scanning {done} of {of}…"));
        }
        if !list.clap.failed.is_empty() {
            let why: Vec<String> = list
                .clap
                .failed
                .iter()
                .map(|(path, e)| format!("{}: {e}", path.display()))
                .collect();
            note(
                ui,
                format!("{} files would not load", list.clap.failed.len()),
            )
            .on_hover_text(why.join("\n"));
        }
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            for kind in PluginKind::ALL {
                ui.selectable_value(&mut list.kind, kind, kind.name());
            }
        });
        ui.add(
            egui::TextEdit::singleline(&mut list.search)
                .hint_text("Search")
                .desired_width(f32::INFINITY),
        );
        ui.add_space(4.0);
        let search = list.search.trim().to_lowercase();
        if list.kind != PluginKind::Instruments {
            stock_add = stock_rows(ui, &mut list.stock_closed, &search).or(stock_add);
            if let Some(t) = template_rows(ui, &mut list.chains_closed, &search) {
                template_task = Some(t);
            }
        }
        ui.horizontal(|ui| {
            ui.toggle_value(&mut list.only_favourites, "★ Favourites")
                .on_hover_text("Only the plugins you starred");
            ui.label("Order");
            ui.selectable_value(&mut list.order, Order::Name, "Name");
            ui.selectable_value(&mut list.order, Order::Maker, "Maker");
            ui.selectable_value(&mut list.order, Order::Kind, "Type")
                .on_hover_text("EQ, dynamics, reverb and so on, worked out from the plugin's name");
        });
        let mut clap: Vec<&PluginInfo> = list
            .clap
            .plugins
            .iter()
            .filter(|info| list.kind.shows(info, &search) && !list.clap.hidden.contains(&info.id))
            .filter(|info| {
                !list.only_favourites || crate::plugin_favourites::is_favourite(&info.id)
            })
            .collect();
        let kind_rank = |info: &PluginInfo| {
            let k = crate::plugin_favourites::kind_of(info);
            crate::plugin_favourites::KINDS.iter().position(|x| *x == k).unwrap_or(99)
        };
        match list.order {
            Order::Name => clap.sort_by_key(|i| i.name.to_lowercase()),
            Order::Maker => clap.sort_by_key(|i| (i.vendor.to_lowercase(), i.name.to_lowercase())),
            Order::Kind => clap.sort_by_key(|i| (kind_rank(i), i.name.to_lowercase())),
        }
        // `show_rows` places rows by this spacing, so it is set before the call.
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, ROW_H, clap.len(), |ui, range| {
                for info in &clap[range] {
                    if clap_row(ui, info) {
                        add = Some((*info).clone());
                    }
                }
            });
    });
    if let Some((kind, file)) = template_task {
        use crate::file_commands::{FileTask, TemplateKind};
        s.file_task = Some(match kind {
            TemplateKind::Chain => FileTask::LoadChain(file, p.selected_track),
            _ => FileTask::LoadTracks(file),
        });
    }
    if let Some(kind) = stock_add {
        if let Err(e) = crate::mixer::add_stock(p, p.selected_track, kind) {
            s.status = Some(e);
        }
    }
    if let Some(info) = add {
        if let Err(e) = crate::plugin_rack::add_plugin(p, p.selected_track, &info) {
            s.status = Some(e);
        }
    }
    // The plugin being dragged follows the pointer until it is dropped.
    if let Some(drag) = egui::DragAndDrop::payload::<PluginDrag>(&ctx) {
        if let Some(pos) = ctx.pointer_interact_pos() {
            egui::Area::new(egui::Id::unique("plugin_drag_ghost"))
                .order(egui::Order::Tooltip)
                .fixed_pos(pos + vec2(14.0, 10.0))
                .interactable(false)
                .show(&ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| ui.label(drag.0.name.as_str()));
                });
        }
    }
}

/// The Presets tab: every saved plugin preset in the user's folder, grouped by plugin. Click a
/// preset to load it into any instance of its plugin on the selected track (instrument or any FX
/// slot). Says so in the status line if no matching plugin is loaded.
fn presets(ui: &mut Ui, p: &mut Project, s: &mut UiState) {
    let panel = ui.available_rect_before_wrap();
    ui.allocate_rect(panel, Sense::hover());
    ui.painter_at(panel).rect_filled(panel, 0.0, panel_bg_col());
    let inner = panel.shrink2(vec2(8.0, 6.0));
    let list = crate::plugin_presets::list_all();
    // Current group being drawn.
    let mut asked: Option<std::path::PathBuf> = None;
    ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("{} presets", list.len())).color(text_col()),
            );
            if let Some(dir) = crate::recent_files::config_dir() {
                let folder = dir.join("presets");
                if ui
                    .small_button("Open folder")
                    .on_hover_text("Shows the presets folder in Explorer")
                    .clicked()
                {
                    let _ = std::fs::create_dir_all(&folder);
                    let _ = std::process::Command::new("explorer").arg(&folder).spawn();
                }
            }
        });
        ui.add_space(4.0);
        if list.is_empty() {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(
                        "No presets saved yet. Save one from a plugin's bar \
                         (the Preset menu > Save As…) and it shows up here.",
                    )
                    .small()
                    .color(text_dim_col()),
                )
                .wrap(),
            );
            return;
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            let mut last_group: Option<&str> = None;
            for (group, name, path) in &list {
                let group_name = group.replace('_', " ");
                if last_group != Some(group.as_str()) {
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(group_name.clone())
                            .color(text_dim_col())
                            .small(),
                    );
                    last_group = Some(group.as_str());
                }
                let resp = ui.selectable_label(false, name.as_str());
                if resp.double_clicked() || resp.clicked() {
                    asked = Some(path.clone());
                }
                resp.on_hover_text(path.display().to_string());
            }
        });
    });
    if let Some(file) = asked {
        apply_preset(p, s, &file);
    }
}

/// Loads `file` into the first plugin on the selected track whose preset folder matches the file's
/// parent folder. Status says what happened.
fn apply_preset(p: &mut Project, s: &mut UiState, file: &std::path::Path) {
    let group = file
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(|s| s.to_string());
    let Some(group) = group else {
        s.status = Some("Could not read the preset's folder".to_string());
        return;
    };
    let name = file.file_stem().and_then(|n| n.to_str()).unwrap_or("Preset");
    let t = p.selected_track;
    // The instrument first (if any), then the FX slots in order.
    let uid = p.tracks.get(t).and_then(|track| {
        track
            .instrument_plugin
            .as_ref()
            .filter(|ins| crate::plugin_presets::folder_name(&ins.id) == group)
            .map(|ins| ins.uid)
            .or_else(|| {
                track
                    .plugins
                    .iter()
                    .find(|ins| crate::plugin_presets::folder_name(&ins.id) == group)
                    .map(|ins| ins.uid)
            })
    });
    let Some(uid) = uid else {
        s.status = Some(format!(
            "Load the matching plugin on the selected track first ({group}): the preset {name} \
             is for that plugin."
        ));
        return;
    };
    s.browser.preset_cmd = Some(crate::plugin_presets::PresetCmd::Load {
        uid,
        file: file.to_path_buf(),
    });
    s.status = Some(format!("Loading preset {name}…"));
}

/// The Project Audio tab: audio files used by the regions in this project, grouped by track. Click
/// a file to select its region, double-click to open it in the audio editor, and the small "Show in
/// Explorer" button reveals it on disk.
fn project_audio(ui: &mut Ui, p: &mut Project, s: &mut UiState) {
    let panel = ui.available_rect_before_wrap();
    ui.allocate_rect(panel, Sense::hover());
    ui.painter_at(panel).rect_filled(panel, 0.0, panel_bg_col());
    let inner = panel.shrink2(vec2(8.0, 6.0));
    // (track_index, track_name, Vec<(region_index, region_name, file)>).
    let mut groups: Vec<(usize, String, Vec<(usize, String, std::path::PathBuf)>)> = Vec::new();
    for (ti, track) in p.tracks.iter().enumerate() {
        let mut rows: Vec<(usize, String, std::path::PathBuf)> = Vec::new();
        for (ri, region) in track.regions.iter().enumerate() {
            if let Some(a) = region.audio.as_ref() {
                rows.push((ri, region.name.clone(), a.file.path.clone()));
            }
        }
        if !rows.is_empty() {
            groups.push((ti, track.name.clone(), rows));
        }
    }
    let total: usize = groups.iter().map(|(_, _, r)| r.len()).sum();
    let mut pick: Option<(usize, usize)> = None;
    let mut reveal: Option<std::path::PathBuf> = None;
    ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("{total} audio files")).color(text_col()));
            let dir = crate::project_io::recordings_dir();
            if ui
                .small_button("Recordings folder")
                .on_hover_text("This session's recordings folder in Explorer")
                .clicked()
            {
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::process::Command::new("explorer").arg(&dir).spawn();
            }
        });
        ui.add_space(4.0);
        if groups.is_empty() {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(
                        "No audio regions in this project yet. Record on an audio track or drop a \
                         file onto the Tracks area.",
                    )
                    .small()
                    .color(text_dim_col()),
                )
                .wrap(),
            );
            return;
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (ti, track_name, rows) in &groups {
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(track_name)
                        .color(text_dim_col())
                        .small(),
                );
                for (ri, region_name, file) in rows {
                    ui.horizontal(|ui| {
                        let is_sel = s.tracks.selected == Some((*ti, *ri));
                        let resp = ui.selectable_label(is_sel, region_name.as_str());
                        if resp.clicked() {
                            pick = Some((*ti, *ri));
                        }
                        resp.on_hover_text(file.display().to_string());
                        if ui
                            .small_button("↗")
                            .on_hover_text("Show in Explorer")
                            .clicked()
                        {
                            reveal = Some(file.clone());
                        }
                    });
                }
            }
        });
    });
    if let Some(sel) = pick {
        s.tracks.select_only(sel);
        p.selected_track = sel.0;
    }
    if let Some(path) = reveal {
        // Highlights the file when Explorer opens.
        let _ = std::process::Command::new("explorer")
            .arg("/select,")
            .arg(&path)
            .spawn();
    }
}

/// The saved effect chains, track templates and mixer templates, in a folder under the Ovrtone one.
/// Double-click a chain to put it on the selected track, a template to add its tracks.
fn template_rows(
    ui: &mut Ui,
    closed: &mut bool,
    search: &str,
) -> Option<(crate::file_commands::TemplateKind, std::path::PathBuf)> {
    use crate::file_commands::{templates, TemplateKind};
    let mut all: Vec<(TemplateKind, String, std::path::PathBuf)> = Vec::new();
    for kind in [TemplateKind::Chain, TemplateKind::Track, TemplateKind::Mixer] {
        for (name, file) in templates(kind) {
            if search.is_empty() || name.to_lowercase().contains(search) {
                all.push((kind, name, file));
            }
        }
    }
    if all.is_empty() {
        return None;
    }
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    let g = ui.painter_at(rect);
    if resp.hovered() {
        g.rect_filled(rect, 0.0, crate::theme::gray(0x46));
    }
    let arrow = if *closed { "▸" } else { "▾" };
    g.text(
        pos2(rect.left() + 4.0, rect.center().y),
        Align2::LEFT_CENTER,
        format!("{arrow}  Chains and Templates  ({})", all.len()),
        FontId::proportional(12.5),
        text_col(),
    );
    if resp.clicked() {
        *closed = !*closed;
    }
    if *closed {
        return None;
    }
    let mut picked = None;
    for (kind, name, file) in all {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
        let g = ui.painter_at(rect);
        if resp.hovered() {
            g.rect_filled(rect, 0.0, crate::theme::gray(0x46));
        }
        let shown = g.text(
            pos2(rect.left() + 18.0, rect.center().y),
            Align2::LEFT_CENTER,
            &name,
            FontId::proportional(12.5),
            text_col(),
        );
        let what = match kind {
            TemplateKind::Chain => "Effect chain",
            TemplateKind::Track => "Track template",
            TemplateKind::Mixer => "Mixer template",
        };
        g.text(
            pos2(shown.right() + 8.0, rect.center().y),
            Align2::LEFT_CENTER,
            what,
            FontId::proportional(11.0),
            text_dim_col(),
        );
        let hint = match kind {
            TemplateKind::Chain => "Double-click: put this chain on the selected track, in place of its effects",
            _ => "Double-click: add its tracks to the project",
        };
        if resp.on_hover_text(hint).double_clicked() {
            picked = Some((kind, file));
        }
    }
    ui.add_space(4.0);
    picked
}

/// Ovrtone's own effects at the top of the Plugins list, in a folder that folds. Double-click
/// one to put it on the selected track, or drag it to a mixer slot. The effect double-clicked.
fn stock_rows(ui: &mut Ui, closed: &mut bool, search: &str) -> Option<ovrtone_engine::FxKind> {
    let kinds: Vec<ovrtone_engine::FxKind> = ovrtone_engine::FxKind::MENU
        .iter()
        .chain(ovrtone_engine::FxKind::CLASSIC.iter())
        .copied()
        .filter(|k| {
            search.is_empty()
                || k.name().to_lowercase().contains(search)
                || k.kind_word().to_lowercase().contains(search)
                || "ovrtone".contains(search)
        })
        .collect();
    if kinds.is_empty() {
        return None;
    }
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    let g = ui.painter_at(rect);
    if resp.hovered() {
        g.rect_filled(rect, 0.0, crate::theme::gray(0x46));
    }
    let arrow = if *closed { "▸" } else { "▾" };
    g.text(
        pos2(rect.left() + 4.0, rect.center().y),
        Align2::LEFT_CENTER,
        format!("{arrow}  Ovrtone  ({})", kinds.len()),
        FontId::proportional(12.5),
        text_col(),
    );
    if resp.clicked() {
        *closed = !*closed;
    }
    if *closed {
        return None;
    }
    let mut picked = None;
    for kind in kinds {
        let (rect, resp) =
            ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click_and_drag());
        resp.dnd_set_drag_payload(crate::mixer::StockDrag(kind));
        let g = ui.painter_at(rect);
        if resp.hovered() {
            g.rect_filled(rect, 0.0, crate::theme::gray(0x46));
        }
        let name = g.text(
            pos2(rect.left() + 18.0, rect.center().y),
            Align2::LEFT_CENTER,
            kind.name(),
            FontId::proportional(12.5),
            text_col(),
        );
        g.text(
            pos2(name.right() + 8.0, rect.center().y),
            Align2::LEFT_CENTER,
            format!("Ovrtone, {}", kind.kind_word()),
            FontId::proportional(11.0),
            text_dim_col(),
        );
        if resp.double_clicked() {
            picked = Some(kind);
        }
        if resp.dragged() {
            if let Some(at) = ui.ctx().pointer_interact_pos() {
                let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("stock_drag"));
                ui.ctx().layer_painter(layer).text(
                    at + vec2(14.0, 10.0),
                    Align2::LEFT_TOP,
                    kind.name(),
                    FontId::proportional(12.5),
                    text_col(),
                );
            }
        }
    }
    ui.add_space(4.0);
    picked
}

/// One plugin: its name, vendor and format, ready to drag. True when double-clicked.
fn clap_row(ui: &mut Ui, info: &PluginInfo) -> bool {
    let (rect, resp) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click_and_drag());
    resp.dnd_set_drag_payload(PluginDrag(info.clone()));
    let g = ui.painter_at(rect);
    if resp.hovered() {
        g.rect_filled(rect, 0.0, crate::theme::gray(0x46));
    }
    let name = g.text(
        pos2(rect.left() + 4.0, rect.center().y),
        Align2::LEFT_CENTER,
        &info.name,
        FontId::proportional(12.5),
        text_col(),
    );
    let kind = if info.is_instrument {
        ", instrument"
    } else {
        ""
    };
    let vendor = format!("{}, {}{kind}", info.vendor, info.format.name());
    g.text(
        pos2(name.right() + 8.0, rect.center().y),
        Align2::LEFT_CENTER,
        vendor,
        FontId::proportional(11.0),
        text_dim_col(),
    );
    // The star: click to make it a favourite or not.
    let fav = crate::plugin_favourites::is_favourite(&info.id);
    let star = Rect::from_center_size(pos2(rect.right() - 12.0, rect.center().y), vec2(18.0, ROW_H));
    let star_resp = ui
        .interact(star, egui::Id::unique(("plugin_star", info.id.as_str())), Sense::click())
        .on_hover_text(if fav { "Remove from Favourites" } else { "Add to Favourites" });
    let col = if fav {
        crate::theme::SOLO
    } else if star_resp.hovered() || resp.hovered() {
        text_dim_col()
    } else {
        Color32::TRANSPARENT
    };
    g.text(star.center(), Align2::CENTER_CENTER, if fav { "★" } else { "☆" }, FontId::proportional(13.0), col);
    if star_resp.clicked() {
        crate::plugin_favourites::toggle(&info.id);
    }
    let double = resp.double_clicked();
    resp.on_hover_text(format!(
        "{}\n{}",
        crate::plugin_favourites::kind_of(info),
        info.path.display()
    ));
    double
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn plugins_can_be_picked_by_kind_and_by_name() {
        let plugin = |name: &str, is_instrument| PluginInfo {
            id: name.to_string(),
            name: name.to_string(),
            vendor: "Bankz Sound Lab".to_string(),
            version: String::new(),
            format: ovrtone_host::PluginFormat::Clap,
            path: name.into(),
            is_instrument,
        };
        let (synth, comp) = (plugin("Keys", true), plugin("Squeeze", false));
        assert!(PluginKind::Instruments.shows(&synth, ""));
        assert!(!PluginKind::Instruments.shows(&comp, ""));
        assert!(PluginKind::Effects.shows(&comp, ""));
        assert!(!PluginKind::Effects.shows(&synth, ""));
        assert!(PluginKind::All.shows(&synth, "bankz"));
        assert!(!PluginKind::All.shows(&synth, "squeeze"));
    }

    #[test]
    fn the_tabs_have_their_names() {
        let names: Vec<_> = LeftTab::ALL.iter().map(|t| t.name()).collect();
        assert_eq!(names, ["Browser", "Plugins", "Presets", "Project Audio"]);
        assert_eq!(LeftTab::default(), LeftTab::Browser);
    }
}
