//! The Settings window (Ctrl+,): the audio driver, device and buffer size, the computer keyboard,
//! plugin folders, how the program looks and whether it looks for updates.
//!
//! Choices of the audio setup wait for Apply, as sound stops for a moment while it changes. The
//! others take effect at once.
use crate::UiState;
use egui::Ui;
use std::sync::mpsc::{channel, Receiver};

const AUDIO_FILE: &str = "audio-setup";
const TYPING_OFF_FILE: &str = "typing-off";
const BUFFERS: [u32; 7] = [32, 64, 128, 256, 512, 1024, 2048];
const RATES: [u32; 4] = [44_100, 48_000, 88_200, 96_000];

/// Which driver, device and buffer size to play through. Empty means the usual choice.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioSetup {
    pub system: String,
    pub device: String,
    pub buffer: Option<u32>,
    pub rate: Option<u32>,
}

impl AudioSetup {
    fn to_text(&self) -> String {
        let buffer = self.buffer.map_or(String::new(), |b| b.to_string());
        let rate = self.rate.map_or(String::new(), |r| r.to_string());
        format!("{}\n{}\n{buffer}\n{rate}\n", self.system, self.device)
    }

    fn from_text(text: &str) -> Self {
        let mut lines = text.lines();
        let mut next = || lines.next().unwrap_or("").trim().to_string();
        let (system, device) = (next(), next());
        let buffer = next().parse().ok().filter(|b| BUFFERS.contains(b));
        let rate = next().parse().ok().filter(|r| RATES.contains(r));
        Self {
            system,
            device,
            buffer,
            rate,
        }
    }

    /// What was saved, or the usual choice.
    pub fn load() -> Self {
        crate::recent_files::config_dir()
            .filter(|_| !cfg!(test))
            .and_then(|d| std::fs::read_to_string(d.join(AUDIO_FILE)).ok())
            .map_or_else(Self::default, |t| Self::from_text(&t))
    }

    fn save(&self) {
        let Some(dir) = crate::recent_files::config_dir().filter(|_| !cfg!(test)) else {
            return;
        };
        let saved = std::fs::create_dir_all(&dir)
            .and_then(|()| std::fs::write(dir.join(AUDIO_FILE), self.to_text()));
        if let Err(e) = saved {
            tracing::warn!("cannot save the audio setup: {e}");
        }
    }
}

/// Whether the computer keyboard plays notes when the program starts.
pub fn typing_at_start() -> bool {
    crate::recent_files::config_dir()
        .filter(|_| !cfg!(test))
        .is_none_or(|d| !d.join(TYPING_OFF_FILE).exists())
}

fn set_typing_at_start(on: bool) {
    let Some(dir) = crate::recent_files::config_dir().filter(|_| !cfg!(test)) else {
        return;
    };
    let file = dir.join(TYPING_OFF_FILE);
    if on {
        let _ = std::fs::remove_file(file);
    } else {
        let _ = std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(file, ""));
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Audio,
    Keyboard,
    Keys,
    Plugins,
    General,
}

impl Tab {
    const ALL: [Tab; 5] = [
        Tab::Audio,
        Tab::Keyboard,
        Tab::Keys,
        Tab::Plugins,
        Tab::General,
    ];

    fn name(self) -> &'static str {
        match self {
            Tab::Audio => "Audio",
            Tab::Keyboard => "Keyboard",
            Tab::Keys => "Key Commands",
            Tab::Plugins => "Plugins",
            Tab::General => "General",
        }
    }
}

#[derive(Default)]
pub struct SettingsView {
    pub open: bool,
    tab: Tab,
    /// The setup being chosen, until Apply.
    setup: AudioSetup,
    loaded: bool,
    systems: Vec<String>,
    devices: Vec<String>,
    /// The devices of the driver chosen, being looked for off the UI thread: a driver can be slow.
    looking: Option<Receiver<Vec<String>>>,
    /// Apply was pressed: `App` restarts the sound and takes this.
    pub apply: Option<AudioSetup>,
    /// What is playing now, as `App` says.
    pub current: String,
}

impl SettingsView {
    fn look_for_devices(&mut self, ctx: &egui::Context) {
        if cfg!(test) {
            return;
        }
        let (tx, rx) = channel();
        let (system, ctx) = (self.setup.system.clone(), ctx.clone());
        self.devices.clear();
        self.looking = Some(rx);
        let _ = std::thread::Builder::new()
            .name("prowave-audio-devices".into())
            .spawn(move || {
                let _ = tx.send(prowave_engine::Engine::audio_devices(&system));
                ctx.request_repaint();
            });
    }

    fn first_open(&mut self, ctx: &egui::Context) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        self.setup = AudioSetup::load();
        if !cfg!(test) {
            self.systems = prowave_engine::Engine::audio_systems();
        }
        self.look_for_devices(ctx);
    }
}

fn audio_tab(ui: &mut Ui, s: &mut UiState) {
    let ctx = ui.ctx().clone();
    let v = &mut s.settings;
    if let Some(found) = v.looking.as_ref().and_then(|rx| rx.try_recv().ok()) {
        v.devices = found;
        v.looking = None;
    }
    ui.label(egui::RichText::new(format!("Playing through: {}", v.current)).small());
    ui.add_space(6.0);
    egui::Grid::new(egui::Id::unique("settings_audio"))
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Driver");
            let shown = if v.setup.system.is_empty() {
                "Automatic (WASAPI, low latency)"
            } else {
                v.setup.system.as_str()
            };
            let before = v.setup.system.clone();
            egui::ComboBox::from_id_salt(egui::Id::unique("settings_driver"))
                .width(280.0)
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut v.setup.system,
                        String::new(),
                        "Automatic (WASAPI, low latency)",
                    );
                    for system in &v.systems {
                        ui.selectable_value(&mut v.setup.system, system.clone(), system.as_str());
                    }
                });
            if v.setup.system != before {
                v.setup.device.clear();
                v.look_for_devices(&ctx);
            }
            ui.end_row();

            ui.label("Device");
            let shown = if v.setup.device.is_empty() {
                "Default"
            } else {
                v.setup.device.as_str()
            };
            egui::ComboBox::from_id_salt(egui::Id::unique("settings_device"))
                .width(280.0)
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut v.setup.device, String::new(), "Default");
                    for device in &v.devices {
                        ui.selectable_value(&mut v.setup.device, device.clone(), device.as_str());
                    }
                    if v.looking.is_some() {
                        ui.label("Looking…");
                    }
                });
            ui.end_row();

            ui.label("Sample rate");
            let shown = v
                .setup
                .rate
                .map_or("The driver's default".to_string(), |r| format!("{r} Hz"));
            egui::ComboBox::from_id_salt(egui::Id::unique("settings_rate"))
                .width(280.0)
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut v.setup.rate, None, "The driver's default");
                    for r in RATES {
                        ui.selectable_value(&mut v.setup.rate, Some(r), format!("{r} Hz"));
                    }
                });
            ui.end_row();

            ui.label("Buffer size");
            let shown = v
                .setup
                .buffer
                .map_or("The driver's default".to_string(), |b| {
                    format!("{b} samples")
                });
            egui::ComboBox::from_id_salt(egui::Id::unique("settings_buffer"))
                .width(280.0)
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut v.setup.buffer, None, "The driver's default");
                    for b in BUFFERS {
                        ui.selectable_value(&mut v.setup.buffer, Some(b), format!("{b} samples"));
                    }
                });
            ui.end_row();
        });
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new("A smaller buffer plays sooner and costs more CPU. ASIO uses what is set in its own driver when it refuses this.")
            .small(),
    );
    ui.add_space(6.0);
    if ui
        .button("Apply")
        .on_hover_text("Sound stops for a moment while it changes")
        .clicked()
    {
        v.setup.save();
        v.apply = Some(v.setup.clone());
    }
}

fn keyboard_tab(ui: &mut Ui, s: &mut UiState) {
    let mut at_start = typing_at_start();
    if ui
        .checkbox(
            &mut at_start,
            "Computer keyboard plays notes when Pro Wave starts",
        )
        .on_hover_text("While it plays notes, its note keys do not work as key commands")
        .changed()
    {
        set_typing_at_start(at_start);
    }
    ui.checkbox(
        &mut s.typing.on,
        "Computer keyboard plays notes now (Ctrl+K)",
    );
    ui.add(egui::Slider::new(&mut s.typing.octave, -5..=4).text("Octave (numpad + and -)"));
}

fn keys_tab(ui: &mut Ui, s: &mut UiState) {
    use crate::key_commands::{key_text, name_of, COMMANDS};
    // Waiting for a key: the next one pressed is it, and Esc gives up.
    if let Some(index) = s.keys.capturing {
        let pressed = ui.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => Some((*key, *modifiers)),
                _ => None,
            })
        });
        match pressed {
            Some((egui::Key::Escape, _)) => s.keys.capturing = None,
            Some((key, modifiers)) => {
                s.keys.capturing = None;
                if let Some(other) = s.keys.taken_by(key, modifiers, index) {
                    s.status = Some(format!(
                        "{} is already on {}",
                        key_text(key, modifiers),
                        name_of(other.action)
                    ));
                } else {
                    s.keys.set(index, key, modifiers);
                }
            }
            None => {}
        }
    }
    ui.horizontal(|ui| {
        ui.label("Click a key, then press the new one. Esc gives up.");
        if ui.small_button("Back to the usual keys").clicked() {
            s.keys.reset();
        }
    });
    egui::ScrollArea::vertical()
        .max_height(360.0)
        .show(ui, |ui| {
            egui::Grid::new(egui::Id::unique("settings_keys"))
                .num_columns(2)
                .show(ui, |ui| {
                    for (i, command) in COMMANDS.iter().enumerate() {
                        ui.label(name_of(command.action));
                        let (key, m) = s.keys.key_of(command);
                        let text = if s.keys.capturing == Some(i) {
                            "Press a key…".to_string()
                        } else {
                            key_text(key, m)
                        };
                        if ui.button(text).clicked() {
                            s.keys.capturing = Some(i);
                        }
                        ui.end_row();
                    }
                });
        });
}

fn plugins_tab(ui: &mut Ui, s: &mut UiState) {
    let ctx = ui.ctx().clone();
    ui.label("Folders to look for plugins in, besides the usual ones:");
    let mut rescan = false;
    for folder in crate::plugin_guard::plugin_folders() {
        ui.horizontal(|ui| {
            if ui
                .small_button("×")
                .on_hover_text("Stop looking here")
                .clicked()
            {
                crate::plugin_guard::remove_folder(&folder);
                rescan = true;
            }
            ui.label(egui::RichText::new(folder.display().to_string()).small());
        });
    }
    ui.horizontal(|ui| {
        if ui.button("Add Folder…").clicked() {
            if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                crate::plugin_guard::add_folder(&folder);
                rescan = true;
            }
        }
        if ui
            .button("Rescan")
            .on_hover_text("Also tries again the plugins that failed or crashed")
            .clicked()
        {
            crate::plugin_guard::clear_blocked();
            s.plugins.rescan(true, &ctx);
        }
    });
    if rescan {
        s.plugins.rescan(false, &ctx);
    }
}

fn general_tab(ui: &mut Ui, s: &mut UiState) {
    ui.horizontal(|ui| {
        ui.label(format!("Interface size: {:.0} %", s.ui_scale * 100.0));
        for (text, dir) in [("Smaller", -1), ("100 %", 0), ("Bigger", 1)] {
            if ui.small_button(text).clicked() {
                s.ui_scale = crate::ui_scale::step(s.ui_scale, dir);
                crate::ui_scale::save(s.ui_scale);
            }
        }
    });
    ui.checkbox(&mut s.light_menus, "Light menu bar");
    if ui
        .checkbox(&mut s.update_auto, "Check for updates automatically")
        .changed()
    {
        crate::updater::set_auto_enabled(s.update_auto);
    }
}

/// The window.
pub fn show(ctx: &egui::Context, s: &mut UiState) {
    if !s.settings.open {
        return;
    }
    s.settings.first_open(ctx);
    let mut open = true;
    egui::Window::new("Settings")
        .id(egui::Id::unique("settings_window"))
        .open(&mut open)
        .collapsible(false)
        .default_width(460.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                for tab in Tab::ALL {
                    ui.selectable_value(&mut s.settings.tab, tab, tab.name());
                }
            });
            ui.separator();
            match s.settings.tab {
                Tab::Audio => audio_tab(ui, s),
                Tab::Keyboard => keyboard_tab(ui, s),
                Tab::Keys => keys_tab(ui, s),
                Tab::Plugins => plugins_tab(ui, s),
                Tab::General => general_tab(ui, s),
            }
        });
    s.settings.open = open;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_audio_setup_reads_back_and_bad_text_gives_the_usual_choice() {
        let setup = AudioSetup {
            system: "ASIO".to_string(),
            device: "Audient USB Audio ASIO Driver".to_string(),
            buffer: Some(128),
            rate: Some(96_000),
        };
        assert_eq!(AudioSetup::from_text(&setup.to_text()), setup);
        assert_eq!(AudioSetup::from_text(""), AudioSetup::default());
        assert_eq!(
            AudioSetup::from_text("ASIO\n\nsoon\n").buffer,
            None,
            "not a number"
        );
        assert_eq!(
            AudioSetup::from_text("\n\n999\n").buffer,
            None,
            "not offered"
        );
    }

    #[test]
    fn every_tab_draws() {
        let mut s = UiState::new();
        s.settings.open = true;
        let mut h = crate::ui_test::Harness::new(900.0, 700.0);
        for tab in Tab::ALL {
            s.settings.tab = tab;
            h.settle(|ui| show(ui.ctx(), &mut s));
            assert!(s.settings.open);
        }
    }
}
