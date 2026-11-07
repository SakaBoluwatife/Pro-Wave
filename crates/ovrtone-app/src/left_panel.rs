//! The left panel's tabs: Library | Browser | Plugins.
//!
//! - **Library:** Logic's patch browser (`library.rs`), as before.
//! - **Browser:** your sample folders (`sample_browser.rs`).
//! - **Plugins:** the CLAP and VST3 plugins found on this computer, ready to go on a track.
use crate::model::Project;
use crate::plugin_scan::PluginScan;
use crate::widgets::{TEXT, TEXT_DIM};
use crate::UiState;
use egui::{pos2, vec2, Align2, Color32, FontId, Rect, Sense, Stroke, Ui, UiBuilder};
use ovrtone_host::PluginInfo;

const PANEL_BG: Color32 = crate::theme::PANEL_BG;
const ROW_H: f32 = 20.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LeftTab {
    #[default]
    Browser,
    Plugins,
}

#[cfg(test)]
impl LeftTab {
    const ALL: [LeftTab; 2] = [LeftTab::Browser, LeftTab::Plugins];

    fn name(self) -> &'static str {
        match self {
            LeftTab::Browser => "Browser",
            LeftTab::Plugins => "Plugins",
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
    g.rect_filled(r, 0.0, crate::theme::PANEL_BG);
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
            g.rect_filled(tab, 5.0, crate::theme::HOVERED);
        }
        g.text(
            tab.center(),
            Align2::CENTER_CENTER,
            name,
            FontId::proportional(12.5),
            if on { crate::theme::on_accent() } else { TEXT },
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
        g.rect_filled(side, 0.0, crate::theme::HEADER);
        g.line_segment(
            [side.right_top(), side.right_bottom()],
            Stroke::new(1.0, crate::theme::BORDER),
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
            g.rect_filled(r, 5.0, crate::theme::SELECTED);
            g.rect_filled(
                Rect::from_min_size(r.min + vec2(0.0, 5.0), vec2(2.5, r.height() - 10.0)),
                1.0,
                crate::theme::accent(),
            );
        } else if resp.hovered() {
            g.rect_filled(r, 5.0, crate::theme::HOVERED);
        }
        let ink = if on {
            crate::theme::accent()
        } else {
            crate::theme::TEXT_DIM
        };
        side_icon(&g, r.left_center() + vec2(15.0, 0.0), i, ink);
        g.text(
            r.left_center() + vec2(29.0, 0.0),
            Align2::LEFT_CENTER,
            name,
            FontId::proportional(12.5),
            TEXT,
        );
    }
    ui.scope_builder(UiBuilder::new().max_rect(rest), |ui| match s.left_tab {
        LeftTab::Browser => crate::sample_browser::show(ui, s),
        LeftTab::Plugins => plugins(ui, p, s),
    });
}

fn plugins(ui: &mut Ui, p: &mut Project, s: &mut UiState) {
    let panel = ui.available_rect_before_wrap();
    ui.allocate_rect(panel, Sense::hover());
    ui.painter_at(panel).rect_filled(panel, 0.0, PANEL_BG);
    let ctx = ui.ctx().clone();
    let list = &mut s.plugins;
    list.clap.ensure_started(&ctx);
    list.clap.drain();
    let mut add = None;
    let inner = panel.shrink2(vec2(8.0, 6.0));
    ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
        ui.horizontal(|ui| {
            let counts = format!("{} plugins", list.clap.plugins.len());
            ui.label(egui::RichText::new(counts).color(TEXT));
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
            ui.add(egui::Label::new(egui::RichText::new(text).small().color(TEXT_DIM)).wrap())
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
        let clap: Vec<&PluginInfo> = list
            .clap
            .plugins
            .iter()
            .filter(|info| list.kind.shows(info, &search) && !list.clap.hidden.contains(&info.id))
            .collect();
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

/// One plugin: its name, vendor and format, ready to drag. True when double-clicked.
fn clap_row(ui: &mut Ui, info: &PluginInfo) -> bool {
    let (rect, resp) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click_and_drag());
    resp.dnd_set_drag_payload(PluginDrag(info.clone()));
    let g = ui.painter_at(rect);
    if resp.hovered() {
        g.rect_filled(rect, 0.0, Color32::from_rgb(0x46, 0x46, 0x46));
    }
    let name = g.text(
        pos2(rect.left() + 4.0, rect.center().y),
        Align2::LEFT_CENTER,
        &info.name,
        FontId::proportional(12.5),
        TEXT,
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
        TEXT_DIM,
    );
    let double = resp.double_clicked();
    resp.on_hover_text(info.path.display().to_string());
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
        assert_eq!(names, ["Browser", "Plugins"]);
        assert_eq!(LeftTab::default(), LeftTab::Browser);
    }
}
