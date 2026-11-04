//! The left panel's tabs: Library | Browser | Plugins.
//!
//! - **Library:** Logic's patch browser (`library.rs`), as before.
//! - **Browser:** your sample folders (`sample_browser.rs`).
//! - **Plugins:** the plugins found on this computer. CLAP effects go on a track from here; VST3
//!   plugins cannot load yet, so they are only listed, and the tab says so.
use crate::i18n::tr;
use crate::model::Project;
use crate::plugin_scan::PluginScan;
use crate::widgets::{BLUE, TEXT, TEXT_DIM};
use crate::UiState;
use egui::{pos2, vec2, Align2, Color32, FontId, Rect, Sense, Stroke, Ui, UiBuilder};
use prowave_host::PluginInfo;
use std::path::{Path, PathBuf};

const TAB_H: f32 = 26.0;
const PANEL_BG: Color32 = Color32::from_rgb(0x38, 0x38, 0x38);
const ROW_H: f32 = 20.0;
/// Folders inside a VST3 folder that are searched too (vendors keep plugins in subfolders).
const SCAN_DEPTH: usize = 2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LeftTab {
    #[default]
    Library,
    Browser,
    Plugins,
}

impl LeftTab {
    const ALL: [LeftTab; 3] = [LeftTab::Library, LeftTab::Browser, LeftTab::Plugins];

    fn name(self) -> &'static str {
        match self {
            LeftTab::Library => "Library",
            LeftTab::Browser => "Browser",
            LeftTab::Plugins => "Plugins",
        }
    }
}

/// What a drag from the Plugins tab carries: the plugin to drop on a track.
#[derive(Clone, Debug)]
pub struct PluginDrag(pub PluginInfo);

/// The plugins found on this computer: CLAP, which can load, and VST3, which is only listed.
#[derive(Default)]
pub struct PluginList {
    clap: PluginScan,
    scanned: bool,
    found: Vec<(String, PathBuf)>,
}

/// Where VST3 plugins are installed.
fn vst3_folders() -> Vec<PathBuf> {
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let mut dirs = Vec::new();
    if cfg!(windows) {
        if let Some(d) = var("COMMONPROGRAMFILES") {
            dirs.push(d.join("VST3"));
        }
        if let Some(d) = var("LOCALAPPDATA") {
            dirs.push(d.join("Programs").join("Common").join("VST3"));
        }
    } else if cfg!(target_os = "macos") {
        dirs.push(PathBuf::from("/Library/Audio/Plug-Ins/VST3"));
        if let Some(h) = var("HOME") {
            dirs.push(h.join("Library/Audio/Plug-Ins/VST3"));
        }
    } else {
        dirs.push(PathBuf::from("/usr/lib/vst3"));
        dirs.push(PathBuf::from("/usr/local/lib/vst3"));
        if let Some(h) = var("HOME") {
            dirs.push(h.join(".vst3"));
        }
    }
    dirs
}

fn is_vst3(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("vst3"))
}

/// Plugins in `dir`: `.vst3` files or bundles, looking `depth` folders down into the others.
fn scan_dir(dir: &Path, depth: usize, out: &mut Vec<(String, PathBuf)>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for e in read.flatten() {
        let path = e.path();
        if is_vst3(&path) {
            let name = path
                .file_stem()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            out.push((name, path));
        } else if depth > 0 && path.is_dir() {
            scan_dir(&path, depth - 1, out);
        }
    }
}

/// Every VST3 plugin in `dirs`, sorted by name, each once.
fn scan(dirs: &[PathBuf]) -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    for d in dirs {
        scan_dir(d, SCAN_DEPTH, &mut found);
    }
    found.sort_by_key(|(n, p)| (n.to_lowercase(), p.clone()));
    found.dedup();
    found
}

pub fn show(ui: &mut Ui, p: &mut Project, s: &mut UiState) {
    let panel = ui.available_rect_before_wrap();
    ui.allocate_rect(panel, Sense::hover());
    let strip = Rect::from_min_size(panel.min, vec2(panel.width(), TAB_H));
    let rest = Rect::from_min_max(pos2(panel.left(), strip.bottom()), panel.max);
    {
        let g = ui.painter_at(strip);
        g.rect_filled(strip, 0.0, PANEL_BG);
        g.line_segment(
            [strip.left_bottom(), strip.right_bottom()],
            Stroke::new(1.0, Color32::from_gray(0x22)),
        );
        g.line_segment(
            [strip.right_top(), strip.right_bottom()],
            Stroke::new(1.0, Color32::from_gray(0x22)),
        );
    }
    let w = strip.width() / LeftTab::ALL.len() as f32;
    for (i, tab) in LeftTab::ALL.into_iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(strip.left() + w * i as f32, strip.top()),
            vec2(w, TAB_H),
        );
        let resp = ui.interact(r, egui::Id::unique(("left_tab", i)), Sense::click());
        if resp.clicked() {
            s.left_tab = tab;
        }
        let on = s.left_tab == tab;
        let g = ui.painter_at(r);
        g.text(
            r.center(),
            Align2::CENTER_CENTER,
            tr(tab.name()),
            FontId::proportional(13.0),
            if on || resp.hovered() { TEXT } else { TEXT_DIM },
        );
        if on {
            g.line_segment(
                [
                    r.left_bottom() + vec2(6.0, -1.0),
                    r.right_bottom() + vec2(-6.0, -1.0),
                ],
                Stroke::new(2.0, BLUE),
            );
        }
    }
    ui.scope_builder(UiBuilder::new().max_rect(rest), |ui| match s.left_tab {
        LeftTab::Library => crate::library::show(ui, p, s),
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
    if !list.scanned {
        list.found = scan(&vst3_folders());
        list.scanned = true;
    }
    list.clap.ensure_started(&ctx);
    list.clap.drain();
    let mut add = None;
    let inner = panel.shrink2(vec2(8.0, 6.0));
    ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
        ui.horizontal(|ui| {
            let counts = format!(
                "{} CLAP, {} VST3",
                list.clap.plugins.len(),
                list.found.len()
            );
            ui.label(egui::RichText::new(counts).color(TEXT));
            if ui
                .small_button("Rescan")
                .on_hover_text("Look for new plugins and try the ones that failed again")
                .clicked()
            {
                list.scanned = false;
                list.clap.start(true, &ctx);
            }
        });
        let note = |ui: &mut Ui, text: String| {
            ui.add(egui::Label::new(egui::RichText::new(text).small().color(TEXT_DIM)).wrap())
        };
        note(
            ui,
            "Double-click an effect to add it to the selected track, or drag it onto a track. \
             Instruments and VST3 cannot load yet."
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
        ui.add_space(4.0);
        let clap = &list.clap.plugins;
        // `show_rows` places rows by this spacing, so it is set before the call.
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, ROW_H, clap.len() + list.found.len(), |ui, range| {
                for i in range {
                    if let Some(info) = clap.get(i) {
                        if clap_row(ui, info) {
                            add = Some(info.clone());
                        }
                    } else if let Some((name, path)) = list.found.get(i - clap.len()) {
                        let (rect, resp) = ui
                            .allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
                        ui.painter_at(rect).text(
                            pos2(rect.left() + 4.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            format!("{name}  (VST3)"),
                            FontId::proportional(12.5),
                            TEXT_DIM,
                        );
                        resp.on_hover_text(path.display().to_string());
                    }
                }
            });
    });
    if let Some(info) = add {
        if let Err(e) = crate::plugin_rack::add_effect(p, p.selected_track, &info) {
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

/// One CLAP plugin: its name and vendor, ready to drag. True when double-clicked.
fn clap_row(ui: &mut Ui, info: &PluginInfo) -> bool {
    let (rect, resp) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click_and_drag());
    if !info.is_instrument {
        resp.dnd_set_drag_payload(PluginDrag(info.clone()));
    }
    let g = ui.painter_at(rect);
    if resp.hovered() {
        g.rect_filled(rect, 0.0, Color32::from_rgb(0x46, 0x46, 0x46));
    }
    let color = if info.is_instrument { TEXT_DIM } else { TEXT };
    let name = g.text(
        pos2(rect.left() + 4.0, rect.center().y),
        Align2::LEFT_CENTER,
        &info.name,
        FontId::proportional(12.5),
        color,
    );
    g.text(
        pos2(name.right() + 8.0, rect.center().y),
        Align2::LEFT_CENTER,
        &info.vendor,
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
    fn the_scan_finds_vst3_files_and_bundles_in_vendor_folders() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Zeta.vst3"), b"").unwrap();
        std::fs::create_dir_all(root.join("Alpha.vst3").join("Contents")).unwrap();
        std::fs::create_dir_all(root.join("Vendor").join("Sub")).unwrap();
        std::fs::write(root.join("Vendor").join("beta.VST3"), b"").unwrap();
        std::fs::write(root.join("Vendor").join("Sub").join("Gamma.vst3"), b"").unwrap();
        std::fs::write(root.join("Vendor").join("readme.txt"), b"").unwrap();
        let names: Vec<String> = scan(&[root.to_path_buf(), root.to_path_buf()])
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(names, ["Alpha", "beta", "Gamma", "Zeta"]);
        assert!(scan(&[root.join("missing")]).is_empty());
    }

    #[test]
    fn the_tabs_have_their_names() {
        let names: Vec<_> = LeftTab::ALL.iter().map(|t| t.name()).collect();
        assert_eq!(names, ["Library", "Browser", "Plugins"]);
        assert_eq!(LeftTab::default(), LeftTab::Library);
    }
}
