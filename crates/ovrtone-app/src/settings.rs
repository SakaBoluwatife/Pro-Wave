//! The Settings window (Ctrl+,): the audio driver, device and buffer size, the computer keyboard,
//! plugin folders, how the program looks and whether it looks for updates.
//!
//! Choices of the audio setup wait for Apply, as sound stops for a moment while it changes. The
//! others take effect at once.
use crate::UiState;
use egui::Ui;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver};

const AUDIO_FILE: &str = "audio-setup";
const TYPING_OFF_FILE: &str = "typing-off";
pub(crate) const BUFFERS: [u32; 7] = [32, 64, 128, 256, 512, 1024, 2048];
pub(crate) const RATES: [u32; 4] = [44_100, 48_000, 88_200, 96_000];

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

    /// What was saved, or the usual choice. With nothing saved yet, the buffer size that suits
    /// this computer's class (see `hardware.rs`); the driver's own if it refuses that.
    pub fn load() -> Self {
        let Some(dir) = crate::recent_files::config_dir().filter(|_| !cfg!(test)) else {
            return Self::default();
        };
        match std::fs::read_to_string(dir.join(AUDIO_FILE)) {
            Ok(t) => Self::from_text(&t),
            Err(_) => Self {
                buffer: Some(crate::hardware::quick().tier().buffer()),
                ..Self::default()
            },
        }
    }

    pub(crate) fn save(&self) {
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
    Display,
}

impl Tab {
    const ALL: [Tab; 6] = [
        Tab::Audio,
        Tab::Keyboard,
        Tab::Keys,
        Tab::Plugins,
        Tab::General,
        Tab::Display,
    ];

    fn name(self) -> &'static str {
        match self {
            Tab::Audio => "Audio",
            Tab::Keyboard => "Keyboard",
            Tab::Keys => "Key Commands",
            Tab::Plugins => "Plugins",
            Tab::General => "General",
            Tab::Display => "Display",
        }
    }
}

impl SettingsView {
    /// Help > Logic-style Key Commands: the window, on its Key Commands tab.
    pub fn open_key_commands(&mut self) {
        self.open = true;
        self.tab = Tab::Keys;
    }
}

#[derive(Default)]
pub struct SettingsView {
    pub open: bool,
    tab: Tab,
    /// The setup being chosen, until Apply.
    pub setup: AudioSetup,
    loaded: bool,
    systems: Vec<String>,
    devices: Vec<String>,
    /// The devices of the driver chosen, being looked for off the UI thread: a driver can be slow.
    looking: Option<Receiver<Vec<String>>>,
    /// The driver whose control panel was asked for: `App` stops the sound and opens it.
    pub panel_wanted: Option<String>,
    /// The panel that is open, with the sound off until Done.
    pub panel: Option<ovrtone_engine::AsioPanel>,
    /// Done was pressed on the panel.
    pub panel_done: bool,
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
            .name("ovrtone-audio-devices".into())
            .spawn(move || {
                let _ = tx.send(ovrtone_engine::Engine::audio_devices(&system));
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
            self.systems = ovrtone_engine::Engine::audio_systems();
        }
        self.look_for_devices(ctx);
    }
}

/// Whether playback renders ahead of the device while nothing is live (on unless turned off).
pub fn render_ahead() -> bool {
    match RENDER_AHEAD_CACHE.load(std::sync::atomic::Ordering::Relaxed) {
        0 => false,
        1 => true,
        _ => {
            let on = !render_ahead_file().is_some_and(|f| f.exists());
            set_render_ahead_cache(on);
            on
        }
    }
}

fn set_render_ahead_cache(on: bool) {
    RENDER_AHEAD_CACHE.store(u8::from(on), std::sync::atomic::Ordering::Relaxed);
}

static RENDER_AHEAD_CACHE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(2);

/// Turns rendering ahead on or off, and remembers it (a marker file means off).
pub fn set_render_ahead(on: bool) {
    set_render_ahead_cache(on);
    let Some(f) = render_ahead_file() else {
        return;
    };
    if on {
        let _ = std::fs::remove_file(f);
    } else {
        if let Some(dir) = f.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(f, "off\n");
    }
}

fn render_ahead_file() -> Option<std::path::PathBuf> {
    crate::recent_files::config_dir()
        .filter(|_| !cfg!(test))
        .map(|d| d.join("render-ahead-off"))
}

fn worker_limit_file() -> Option<std::path::PathBuf> {
    crate::recent_files::config_dir().map(|d| d.join("audio-threads"))
}

/// How many threads help run plugins, as chosen here: 0 is automatic.
pub fn worker_limit() -> usize {
    worker_limit_file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0)
}

/// Which device recording comes from, apart from the one playback goes to.
fn input_row(ui: &mut Ui, inputs: &mut crate::audio_rec::InputChoice) {
    ui.separator();
    if inputs.devices.is_empty() {
        inputs.refresh();
    }
    let mut chosen = inputs.device.clone();
    ui.horizontal(|ui| {
        ui.label("Input device");
        let shown = match &chosen {
            None => "System default".to_string(),
            Some(id) => inputs
                .devices
                .iter()
                .find(|d| d.id == *id)
                .map_or_else(|| id.clone(), |d| d.name.clone()),
        };
        egui::ComboBox::from_id_salt(egui::Id::unique("settings_input"))
            .selected_text(shown)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut chosen, None, "System default");
                for device in &inputs.devices {
                    ui.selectable_value(&mut chosen, Some(device.id.clone()), device.name.as_str());
                }
            });
    });
    inputs.pick(chosen);
}

fn isolation_file() -> Option<std::path::PathBuf> {
    crate::recent_files::config_dir().map(|d| d.join("plugin-isolation"))
}

/// Whether plugins run in processes of their own: no, unless turned on here. They run inside Ovrtone.
pub fn plugin_isolation() -> bool {
    isolation_file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .is_some_and(|text| text.trim() == "1")
}

fn chooser_file() -> Option<std::path::PathBuf> {
    crate::recent_files::config_dir().map(|d| d.join("chooser-at-launch"))
}

/// Whether the Project Chooser comes up when the program starts. It does, unless turned off here.
pub fn chooser_at_launch() -> bool {
    chooser_file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .is_none_or(|text| text.trim() != "0")
}

fn chooser_row(ui: &mut Ui) {
    let saved = chooser_at_launch();
    let mut on = saved;
    ui.checkbox(&mut on, "Show the Project Chooser at startup")
        .on_hover_text("Off: Ovrtone starts on an empty project.");
    if on != saved {
        let Some(file) = chooser_file() else {
            return;
        };
        let result = file
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&file, if on { "1" } else { "0" }));
        if let Err(e) = result {
            tracing::warn!("could not save the chooser setting: {e}");
        }
    }
}

fn ara_file() -> Option<std::path::PathBuf> {
    crate::recent_files::config_dir().map(|d| d.join("plugin-ara"))
}

/// Whether a plugin on an audio track that offers ARA plays the track's audio itself. On, unless
/// turned off here.
pub fn use_ara() -> bool {
    ara_file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .is_none_or(|text| text.trim() != "0")
}

fn ara_row(ui: &mut Ui) {
    let saved = use_ara();
    let mut on = saved;
    ui.checkbox(&mut on, "Use ARA with plugins that offer it")
        .on_hover_text(
            "A plugin such as a pitch editor reads the audio regions of its track directly, with \
             no need to play them into it first. Only on audio tracks. Applies when a plugin is \
             next loaded.",
        );
    if on != saved {
        let Some(file) = ara_file() else {
            return;
        };
        let result = file
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&file, if on { "1" } else { "0" }));
        if let Err(e) = result {
            tracing::warn!("could not save the ARA setting: {e}");
        }
    }
}

fn isolation_row(ui: &mut Ui) {
    let saved = plugin_isolation();
    let mut on = saved;
    ui.checkbox(&mut on, "Run plugins in their own process")
        .on_hover_text(
            "A plugin that crashes does not close Ovrtone, and is started again. Plugins from one \
             file share a process, so a crash stops the others from that file too. This protects \
             against crashes only, and uses more CPU and memory than running inside Ovrtone.",
        );
    ui.label(egui::RichText::new("Applies the next time Ovrtone starts.").small());
    if on != saved {
        let Some(file) = isolation_file() else {
            return;
        };
        let result = file
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&file, if on { "1" } else { "0" }));
        if let Err(e) = result {
            tracing::warn!("could not save the plugin process setting: {e}");
        }
    }
}

static WIDE_PLAYHEAD: AtomicBool = AtomicBool::new(false);
/// What is added to the octave of a note to name it: -2 calls middle C (pitch 60) C3.
static OCTAVE_NAMING: AtomicI32 = AtomicI32::new(-2);
/// How long a strip's peak readout holds, 0 until it is clicked.
static PEAK_HOLD_MS: AtomicU32 = AtomicU32::new(0);
static OPEN_ON_INSERT: AtomicBool = AtomicBool::new(true);
static SHOW_RECENT: AtomicBool = AtomicBool::new(true);
/// Pictures of instruments on the track headers and mixer strips: off unless asked for.
static SHOW_ICONS: AtomicBool = AtomicBool::new(false);

/// Stereo Out on the mixer's left edge rather than its right.
static MASTER_LEFT: AtomicBool = AtomicBool::new(false);

pub fn master_on_left() -> bool {
    MASTER_LEFT.load(Ordering::Relaxed)
}

pub fn show_track_icons() -> bool {
    SHOW_ICONS.load(Ordering::Relaxed)
}

pub fn wide_playhead() -> bool {
    WIDE_PLAYHEAD.load(Ordering::Relaxed)
}

pub fn octave_naming() -> i32 {
    OCTAVE_NAMING.load(Ordering::Relaxed)
}

pub fn peak_hold_ms() -> u32 {
    PEAK_HOLD_MS.load(Ordering::Relaxed)
}

pub fn open_on_insert() -> bool {
    OPEN_ON_INSERT.load(Ordering::Relaxed)
}

pub fn show_recent_plugins() -> bool {
    SHOW_RECENT.load(Ordering::Relaxed)
}

fn view_file() -> Option<std::path::PathBuf> {
    crate::recent_files::config_dir().map(|d| d.join("display-settings"))
}

/// Takes in the Display settings. Call once, at the start.
pub fn load_view() {
    let Some(text) = view_file().and_then(|f| std::fs::read_to_string(f).ok()) else {
        return;
    };
    let mut lines = text.lines().map(str::trim);
    let mut next = || lines.next().and_then(|l| l.parse::<i64>().ok());
    if let Some(v) = next() {
        WIDE_PLAYHEAD.store(v != 0, Ordering::Relaxed);
    }
    if let Some(v) = next() {
        OCTAVE_NAMING.store((v as i32).clamp(-2, 0), Ordering::Relaxed);
    }
    if let Some(v) = next() {
        PEAK_HOLD_MS.store(v.clamp(0, 60_000) as u32, Ordering::Relaxed);
    }
    if let Some(v) = next() {
        OPEN_ON_INSERT.store(v != 0, Ordering::Relaxed);
    }
    if let Some(v) = next() {
        SHOW_RECENT.store(v != 0, Ordering::Relaxed);
    }
    if let Some(v) = next() {
        SHOW_ICONS.store(v != 0, Ordering::Relaxed);
    }
    if let Some(v) = next() {
        MASTER_LEFT.store(v != 0, Ordering::Relaxed);
    }
}

fn save_view() {
    let Some(file) = view_file() else {
        return;
    };
    let text = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        i32::from(wide_playhead()),
        octave_naming(),
        peak_hold_ms(),
        i32::from(open_on_insert()),
        i32::from(show_recent_plugins()),
        i32::from(show_track_icons()),
        i32::from(master_on_left()),
    );
    let saved = file
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&file, text));
    if let Err(e) = saved {
        tracing::warn!("could not save the display settings: {e}");
    }
}

/// The colour of what is selected or on, everywhere: a ready one, or any the user picks.
fn accent_row(ui: &mut Ui) {
    let now = crate::theme::accent();
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.label("Accent");
        for (name, color) in crate::theme::ACCENTS {
            if ui.selectable_label(now == color, name).clicked() {
                picked = Some(color);
            }
        }
        let mut custom = now;
        if ui.color_edit_button_srgba(&mut custom).changed() {
            picked = Some(custom);
        }
        ui.label("Your own");
    });
    if let Some(color) = picked {
        crate::theme::set_accent(ui.ctx(), color);
    }
}

/// The typeface and the size of text.
fn type_row(ui: &mut Ui) {
    use crate::theme::{chosen_face, set_type, text_percent, FACES, TEXT_PERCENT_RANGE};
    let (face, percent) = (chosen_face(), text_percent());
    let (mut new_face, mut new_percent) = (face, percent);
    ui.horizontal(|ui| {
        ui.label("Type");
        let shown = face.map_or("As the theme has it", |f| FACES[f].name);
        egui::ComboBox::from_id_salt(egui::Id::unique("type_face"))
            .selected_text(shown)
            .show_ui(ui, |ui| {
                if ui.selectable_label(face.is_none(), "As the theme has it").clicked() {
                    new_face = None;
                }
                for (k, f) in FACES.iter().enumerate() {
                    if ui.selectable_label(face == Some(k), f.name).clicked() {
                        new_face = Some(k);
                    }
                }
            });
        ui.add(
            egui::Slider::new(&mut new_percent, TEXT_PERCENT_RANGE)
                .step_by(5.0)
                .suffix("%")
                .text("Text size"),
        )
        .on_hover_text("For a bigger interface overall, use View > Interface Size");
    });
    if (new_face, new_percent) != (face, percent) {
        set_type(ui.ctx(), new_face, new_percent);
    }
}

fn display_tab(ui: &mut Ui) {
    crate::themes::section(ui);
    ui.add_space(6.0);
    accent_row(ui);
    ui.add_space(6.0);
    type_row(ui);
    ui.add_space(6.0);
    let (mut high, mut ring) = crate::theme::access();
    let a = ui
        .checkbox(&mut high, "High contrast")
        .on_hover_text("Text at full strength and every control's edge drawn plainly");
    let b = ui
        .checkbox(&mut ring, "Show keyboard focus")
        .on_hover_text("A strong ring round the control Tab has moved to");
    if a.changed() || b.changed() {
        crate::theme::set_access(ui.ctx(), high, ring);
    }
    ui.add_space(6.0);
    let mut changed = false;
    let mut wide = wide_playhead();
    changed |= ui.checkbox(&mut wide, "Wide playhead").changed();
    WIDE_PLAYHEAD.store(wide, Ordering::Relaxed);
    let mut icons = show_track_icons();
    changed |= ui
        .checkbox(&mut icons, "Show track icons")
        .on_hover_text("Pictures of instruments on the track headers and mixer strips")
        .changed();
    SHOW_ICONS.store(icons, Ordering::Relaxed);
    let mut left = master_on_left();
    changed |= ui
        .checkbox(&mut left, "Stereo Out on the left of the mixer")
        .on_hover_text("Where the master strip stays while the tracks scroll")
        .changed();
    MASTER_LEFT.store(left, Ordering::Relaxed);
    ui.add_space(6.0);
    let mut naming = octave_naming();
    ui.horizontal(|ui| {
        ui.label("Display middle C as");
        for (name, value) in [("C3 (Yamaha)", -2), ("C4 (Roland)", -1), ("C5", 0)] {
            changed |= ui.selectable_value(&mut naming, value, name).changed();
        }
    });
    OCTAVE_NAMING.store(naming, Ordering::Relaxed);
    ui.add_space(6.0);
    let mut hold = peak_hold_ms();
    ui.horizontal(|ui| {
        ui.label("Peak hold time");
        for (name, value) in [
            ("Until clicked", 0),
            ("800 ms", 800),
            ("2 s", 2000),
            ("6 s", 6000),
        ] {
            changed |= ui.selectable_value(&mut hold, value, name).changed();
        }
    });
    PEAK_HOLD_MS.store(hold, Ordering::Relaxed);
    ui.add_space(6.0);
    let mut insert = open_on_insert();
    changed |= ui
        .checkbox(&mut insert, "Open plugin window on insertion")
        .changed();
    OPEN_ON_INSERT.store(insert, Ordering::Relaxed);
    let mut recent = show_recent_plugins();
    changed |= ui
        .checkbox(&mut recent, "Show recent plugins in the plugin menu")
        .changed();
    SHOW_RECENT.store(recent, Ordering::Relaxed);
    if changed {
        save_view();
    }
}

fn plugin_block_file() -> Option<std::path::PathBuf> {
    crate::recent_files::config_dir().map(|d| d.join("plugin-block"))
}

/// The block plugins work in, as chosen here: 0 is the usual one.
pub fn plugin_block() -> usize {
    plugin_block_file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0)
}

fn block_row(ui: &mut Ui) {
    let saved = ovrtone_engine::PLUGIN_BLOCK.max(plugin_block());
    let mut chosen = saved;
    ui.horizontal(|ui| {
        ui.label("Plugin block size");
        egui::ComboBox::from_id_salt(egui::Id::unique("settings_plugin_block"))
            .selected_text(chosen.to_string())
            .show_ui(ui, |ui| {
                for n in [64, 128, 256, 512, 1024] {
                    ui.selectable_value(&mut chosen, n, n.to_string());
                }
            });
    })
    .response
    .on_hover_text(
        "Larger blocks often use less CPU with heavy plugins. Each strip with plugins is then \
         later by one block: tracks stay in time, but what you monitor through plugins is late. \
         Try the same size as your buffer. 64 is the lowest delay.",
    );
    ui.label(egui::RichText::new("Applies the next time Ovrtone starts.").small());
    if chosen != saved {
        let Some(file) = plugin_block_file() else {
            return;
        };
        let result = file
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&file, chosen.to_string()));
        if let Err(e) = result {
            tracing::warn!("could not save the plugin block size: {e}");
        }
    }
}

fn threads_row(ui: &mut Ui) {
    let saved = worker_limit();
    let mut chosen = saved;
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Plugin threads");
        let shown = if chosen == 0 {
            "Automatic".to_string()
        } else {
            chosen.to_string()
        };
        egui::ComboBox::from_id_salt(egui::Id::unique("settings_threads"))
            .selected_text(shown)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut chosen, 0, "Automatic");
                for n in 1..=cores {
                    ui.selectable_value(&mut chosen, n, n.to_string());
                }
            });
    })
    .response
    .on_hover_text("Fewer leaves more for plugins that use several cores themselves.");
    ui.label(egui::RichText::new("Applies the next time Ovrtone starts.").small());
    if chosen != saved {
        let Some(file) = worker_limit_file() else {
            return;
        };
        let result = file
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&file, chosen.to_string()));
        if let Err(e) = result {
            tracing::warn!("could not save the plugin threads: {e}");
        }
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
    let mut ahead = render_ahead();
    if ui
        .checkbox(&mut ahead, "Render ahead during playback")
        .on_hover_text(
            "While nothing is recording, monitoring or being played in, the song is worked out \
             about 50 ms early, so heavy passages have more time. Live playing still uses the \
             buffer above.",
        )
        .changed()
    {
        set_render_ahead(ahead);
    }
    ui.add_space(6.0);
    if ui
        .button("Apply")
        .on_hover_text("Sound stops for a moment while it changes")
        .clicked()
    {
        v.setup.save();
        v.apply = Some(v.setup.clone());
    }
    if v.setup.system.eq_ignore_ascii_case("asio") {
        ui.separator();
        if v.panel.is_some() {
            ui.label(
                "The driver's control panel is open, and the sound is off until you are done.",
            );
            if ui.button("Done").clicked() {
                v.panel_done = true;
            }
        } else {
            let chosen = !v.setup.device.is_empty();
            let button = ui
                .add_enabled(chosen, egui::Button::new("Driver Control Panel"))
                .on_hover_text("The sound stops while it is open")
                .on_disabled_hover_text("Choose a device above first");
            if button.clicked() {
                v.panel_wanted = Some(v.setup.device.clone());
            }
        }
    }
    input_row(ui, &mut s.mixer.inputs);
    threads_row(ui);
    block_row(ui);
}

fn keyboard_tab(ui: &mut Ui, s: &mut UiState) {
    let mut at_start = typing_at_start();
    if ui
        .checkbox(
            &mut at_start,
            "Computer keyboard plays notes when Ovrtone starts",
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
        ui.label("Keys like");
        let now = s.keys.preset;
        egui::ComboBox::from_id_salt(egui::Id::unique("settings_key_preset"))
            .selected_text(now.name())
            .show_ui(ui, |ui| {
                for preset in crate::key_commands::KeyPreset::ALL {
                    if ui.selectable_label(now == preset, preset.name()).clicked() {
                        s.keys.choose_preset(preset);
                    }
                }
            });
    })
    .response
    .on_hover_text(
        "A whole set of keys. Each keeps the changes you make to it. Space, Undo, the File commands \
         and the piano roll's keys are the same in all of them.",
    );
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
                        let text = if s.keys.capturing == Some(i) {
                            "Press a key…".to_string()
                        } else {
                            s.keys
                                .key_of(i)
                                .map_or("No key".to_string(), |(key, m)| key_text(key, m))
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
    chooser_row(ui);
    ara_row(ui);
    isolation_row(ui);
    ui.separator();
    let ctx = ui.ctx().clone();
    ui.label("Folders to look for plugins in, besides the usual ones:");
    let mut rescan = false;
    for folder in crate::plugin_guard::plugin_folders() {
        ui.horizontal(|ui| {
            if crate::icons::button(ui, crate::icons::Icon::Close, "Stop looking here").clicked()
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

/// How many backups of a project are kept beside it (Settings > General); 0 keeps none.
pub fn backups_kept() -> usize {
    crate::recent_files::config_dir()
        .filter(|_| !cfg!(test))
        .and_then(|d| std::fs::read_to_string(d.join("backups-kept")).ok())
        .and_then(|t| t.trim().parse().ok())
        .unwrap_or(10)
}

fn set_backups_kept(n: usize) {
    if let Some(dir) = crate::recent_files::config_dir().filter(|_| !cfg!(test)) {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("backups-kept"), n.to_string());
    }
}

/// Minutes between crash-recovery copies (Settings > General); 0 is never.
pub fn autosave_minutes() -> u32 {
    crate::recent_files::config_dir()
        .filter(|_| !cfg!(test))
        .and_then(|d| std::fs::read_to_string(d.join("autosave-minutes")).ok())
        .and_then(|t| t.trim().parse().ok())
        .unwrap_or(2)
}

fn set_autosave_minutes(m: u32) {
    if let Some(dir) = crate::recent_files::config_dir().filter(|_| !cfg!(test)) {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("autosave-minutes"), m.to_string());
    }
}

fn general_tab(ui: &mut Ui, s: &mut UiState) {
    let mut every = s.autosave_minutes;
    ui.horizontal(|ui| {
        ui.label("Crash-recovery copy");
        for (name, m) in [("Every minute", 1), ("2 minutes", 2), ("5 minutes", 5), ("10 minutes", 10), ("Off", 0)] {
            ui.selectable_value(&mut every, m, name);
        }
    })
    .response
    .on_hover_text("While a project has unsaved changes, a copy is kept this often and offered after a crash");
    if every != s.autosave_minutes {
        s.autosave_minutes = every;
        set_autosave_minutes(every);
    }
    let mut kept = s.backups_kept;
    ui.horizontal(|ui| {
        ui.label("Backups kept on save");
        if ui.add(egui::DragValue::new(&mut kept).range(0..=100)).changed() {
            s.backups_kept = kept;
            set_backups_kept(kept);
        }
    });
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
    if crate::updater::UPDATES
        && ui
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
                Tab::Display => display_tab(ui),
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
