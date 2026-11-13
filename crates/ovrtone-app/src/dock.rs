//! Dockable panels: each of the main window's side and bottom panels can sit on the left, on the
//! right or at the bottom, float as a window over the main one, or go into a window of its own that
//! can be moved to another monitor. The layout is kept in the settings folder, so the program opens
//! as it was left. Window > Panel Layout sets it.
use egui::{Rect, Ui};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Place {
    Left,
    Right,
    Bottom,
    /// A window over the main one.
    Float,
    /// A window of its own, which can go to another monitor.
    Window,
}

impl Place {
    fn word(self) -> &'static str {
        match self {
            Place::Left => "left",
            Place::Right => "right",
            Place::Bottom => "bottom",
            Place::Float => "float",
            Place::Window => "window",
        }
    }

    fn from_word(w: &str) -> Option<Self> {
        Some(match w {
            "left" => Place::Left,
            "right" => Place::Right,
            "bottom" => Place::Bottom,
            "float" => Place::Float,
            "window" => Place::Window,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Place::Left => "Left",
            Place::Right => "Right",
            Place::Bottom => "Bottom",
            Place::Float => "Floating",
            Place::Window => "Own Window",
        }
    }
}

/// The panels that can be moved.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pane {
    Library,
    Inspector,
    Bottom,
    Lists,
    Notes,
    Files,
}

impl Pane {
    pub const ALL: [Pane; 6] = [
        Pane::Library,
        Pane::Inspector,
        Pane::Bottom,
        Pane::Lists,
        Pane::Notes,
        Pane::Files,
    ];

    fn key(self) -> &'static str {
        match self {
            Pane::Library => "library",
            Pane::Inspector => "inspector",
            Pane::Bottom => "bottom",
            Pane::Lists => "lists",
            Pane::Notes => "notes",
            Pane::Files => "files",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Pane::Library => "Browser",
            Pane::Inspector => "Inspector",
            Pane::Bottom => "Mixer and Piano Roll",
            Pane::Lists => "Lists",
            Pane::Notes => "Notes",
            Pane::Files => "Project Files",
        }
    }

    /// Where it sits when nothing else is set.
    pub fn home(self) -> Place {
        match self {
            Pane::Library | Pane::Inspector => Place::Left,
            Pane::Bottom => Place::Bottom,
            Pane::Lists | Pane::Notes | Pane::Files => Place::Right,
        }
    }

    /// Where it may go: the bottom panel is too wide for a side, the side panels too tall for the
    /// bottom.
    pub fn places(self) -> &'static [Place] {
        match self {
            Pane::Bottom => &[Place::Bottom, Place::Float, Place::Window],
            _ => &[Place::Left, Place::Right, Place::Float, Place::Window],
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Layout {
    places: [Place; 6],
    /// The Panel Layout window is open.
    pub open: bool,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            places: Pane::ALL.map(Pane::home),
            open: false,
        }
    }
}

fn layout_file() -> Option<std::path::PathBuf> {
    crate::recent_files::config_dir().map(|d| d.join("panel-layout"))
}

impl Layout {
    pub fn place(&self, pane: Pane) -> Place {
        self.places[pane as usize]
    }

    pub fn set(&mut self, pane: Pane, place: Place) {
        if pane.places().contains(&place) {
            self.places[pane as usize] = place;
            self.save();
        }
    }

    /// The layout as it was left, or the usual one.
    pub fn load() -> Self {
        let mut layout = Self::default();
        let text = layout_file()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .unwrap_or_default();
        layout.read_text(&text);
        layout
    }

    /// The layout as text, one `panel=place` a line (what a screenset keeps).
    pub fn to_text(&self) -> String {
        Pane::ALL
            .iter()
            .map(|p| format!("{}={}\n", p.key(), self.place(*p).word()))
            .collect()
    }

    /// Takes on a layout from text (a screenset's), and keeps it as the one in use.
    pub fn apply_text(&mut self, text: &str) {
        if text.trim().is_empty() {
            return;
        }
        self.read_text(text);
        self.save();
    }

    fn read_text(&mut self, text: &str) {
        for line in text.lines() {
            let Some((key, word)) = line.split_once('=') else {
                continue;
            };
            let (Some(pane), Some(place)) = (
                Pane::ALL.into_iter().find(|p| p.key() == key.trim()),
                Place::from_word(word.trim()),
            ) else {
                continue;
            };
            if pane.places().contains(&place) {
                self.places[pane as usize] = place;
            }
        }
    }

    fn save(&self) {
        let Some(file) = layout_file() else {
            return;
        };
        let text = self.to_text();
        let saved = file
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&file, text));
        if let Err(e) = saved {
            tracing::warn!("could not save the panel layout: {e}");
        }
    }
}

/// How a panel is sized where it is docked.
pub struct Size {
    pub default: f32,
    pub range: std::ops::RangeInclusive<f32>,
    /// A fixed size that cannot be dragged (the Inspector).
    pub exact: Option<f32>,
}

/// Shows `pane` where the layout puts it, with `contents` in it. Returns its rectangle when it is
/// docked in the main window (for the panel edges), and `false` in the second place when the user
/// closed its window: it then goes back home.
pub fn show(
    ui: &mut Ui,
    layout: &mut Layout,
    pane: Pane,
    size: Size,
    frame: egui::Frame,
    contents: impl FnOnce(&mut Ui),
) -> Option<Rect> {
    let id = pane.key();
    match layout.place(pane) {
        place @ (Place::Left | Place::Right | Place::Bottom) => {
            let panel = match place {
                Place::Left => egui::Panel::left(id),
                Place::Right => egui::Panel::right(id),
                _ => egui::Panel::bottom(id),
            }
            .frame(frame);
            let panel = match size.exact {
                Some(w) => panel.exact_size(w),
                None => panel
                    .resizable(true)
                    .default_size(size.default)
                    .size_range(size.range),
            };
            Some(panel.show(ui, contents).response.rect)
        }
        Place::Float => {
            let mut open = true;
            let mut contents = Some(contents);
            egui::Window::new(pane.title())
                .id(egui::Id::unique(("dock_float", id)))
                .open(&mut open)
                .resizable(true)
                .default_size([size.exact.unwrap_or(size.default).max(320.0), 520.0])
                .frame(
                    egui::Frame::new()
                        .fill(crate::theme::panel_bg())
                        .stroke(egui::Stroke::new(1.0, crate::theme::border()))
                        .corner_radius(8)
                        .inner_margin(egui::Margin::same(4)),
                )
                .show(ui.ctx(), |ui| {
                    if let Some(f) = contents.take() {
                        f(ui);
                    }
                });
            if !open {
                layout.set(pane, pane.home());
            }
            None
        }
        Place::Window => {
            let mut contents = Some(contents);
            let w = size.exact.unwrap_or(size.default).max(360.0);
            let builder = egui::ViewportBuilder::default()
                .with_title(format!("Ovrtone · {}", pane.title()))
                .with_inner_size([w, if pane == Pane::Bottom { 420.0 } else { 720.0 }]);
            let closed = ui.ctx().show_viewport_immediate(
                egui::ViewportId::from_hash_of(("dock_window", id)),
                builder,
                |ui, _class| {
                    let rect = ui.available_rect_before_wrap();
                    ui.painter().rect_filled(rect, 0.0, crate::theme::panel_bg());
                    if let Some(f) = contents.take() {
                        f(ui);
                    }
                    ui.ctx().input(|i| i.viewport().close_requested())
                },
            );
            if closed {
                layout.set(pane, pane.home());
            }
            None
        }
    }
}

/// Window > Panel Layout: where each panel goes.
pub fn layout_window(ctx: &egui::Context, layout: &mut Layout) {
    if !layout.open {
        return;
    }
    let mut open = true;
    egui::Window::new("Panel Layout")
        .id(egui::Id::unique("panel_layout"))
        .open(&mut open)
        .resizable(false)
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new(
                    "Dock each panel on a side, float it over the window, or give it a window of its \
                     own to move to another monitor. Closing a panel's window puts it back.",
                )
                .small()
                .color(crate::theme::text_dim()),
            );
            ui.add_space(4.0);
            egui::Grid::new(egui::Id::unique("panel_layout_grid"))
                .num_columns(2)
                .spacing([16.0, 6.0])
                .show(ui, |ui| {
                    for pane in Pane::ALL {
                        ui.label(pane.title());
                        let now = layout.place(pane);
                        let mut pick = now;
                        egui::ComboBox::from_id_salt(("panel_place", pane.key()))
                            .selected_text(now.name())
                            .show_ui(ui, |ui| {
                                for place in pane.places() {
                                    ui.selectable_value(&mut pick, *place, place.name());
                                }
                            });
                        if pick != now {
                            layout.set(pane, pick);
                        }
                        ui.end_row();
                    }
                });
            ui.add_space(4.0);
            if ui.button("Reset Layout").clicked() {
                for pane in Pane::ALL {
                    layout.set(pane, pane.home());
                }
            }
        });
    if !open {
        layout.open = false;
    }
}
