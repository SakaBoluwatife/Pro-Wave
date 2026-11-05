//! CLAP plugins, through the `clack-host` bindings.
use crate::{HostError as Error, PluginFormat, PluginInfo};
use clack_extensions::audio_ports::{AudioPortFlags, AudioPortInfoBuffer, PluginAudioPorts};
use clack_extensions::gui::{
    GuiApiType, GuiConfiguration, GuiSize, HostGui, HostGuiImpl, PluginGui, Window,
};
use clack_extensions::latency::PluginLatency;
use clack_extensions::params::{ParamInfoBuffer, ParamInfoFlags, PluginParams};
use clack_extensions::state::{HostState, HostStateImpl, PluginState};
use clack_host::events::event_types::{MidiEvent, NoteOffEvent, NoteOnEvent, ParamValueEvent};
use clack_host::events::event_types::{TransportEvent, TransportFlags};
use clack_host::events::spaces::CoreEventSpace;
use clack_host::events::{EventFlags, Match};
use clack_host::prelude::*;
use clack_host::utils::{BeatTime, SecondsTime};
use prowave_engine::{
    PluginEvent, PluginEventKind, PluginProcessor, PluginTransport, MAX_PLUGIN_EVENTS, PLUGIN_BLOCK,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Folders inside a CLAP folder that are searched too (vendors keep plugins in subfolders).
const SCAN_DEPTH: usize = 3;
/// A plugin with more audio ports than this is not loaded: all of them need buffers.
const MAX_PORTS: usize = 16;
const MAX_CHANNELS: usize = 64;

fn host_info() -> Result<HostInfo, Error> {
    HostInfo::new(
        "Pro Wave",
        "Saka Audio",
        "https://github.com/SakaBoluwatife/Pro-Wave",
        env!("CARGO_PKG_VERSION"),
    )
    .map_err(|e| Error(e.to_string()))
}

/// What the plugin can ask of the host from any thread. It is answered on the UI thread.
struct Shared {
    callback: Arc<AtomicBool>,
    gui: Arc<GuiRequests>,
}

/// What the plugin asked of its editor window, for the next tick.
#[derive(Default)]
struct GuiRequests {
    /// The size it wants, packed; 0 is none.
    resize: AtomicU64,
    /// It ended its interface by itself.
    closed: AtomicBool,
}

impl HostGuiImpl for Shared {
    fn resize_hints_changed(&self) {}

    fn request_resize(&self, new_size: GuiSize) -> Result<(), HostError> {
        self.gui
            .resize
            .store(new_size.pack_to_u64(), Ordering::Release);
        Ok(())
    }

    fn request_show(&self) -> Result<(), HostError> {
        Ok(())
    }

    fn request_hide(&self) -> Result<(), HostError> {
        Ok(())
    }

    fn closed(&self, _was_destroyed: bool) {
        self.gui.closed.store(true, Ordering::Release);
    }
}

impl SharedHandler<'_> for Shared {
    fn request_restart(&self) {}
    fn request_process(&self) {}
    fn request_callback(&self) {
        self.callback.store(true, Ordering::Release);
    }
}

struct Handlers;

/// What the plugin tells the host on the main thread.
struct MainThread {
    /// Its state changed: the project has something new to save.
    dirty: Arc<AtomicBool>,
}

impl MainThreadHandler<'_> for MainThread {}

impl HostStateImpl for MainThread {
    fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Release);
    }
}

impl HostHandlers for Handlers {
    type Shared<'a> = Shared;
    type MainThread<'a> = MainThread;
    type AudioProcessor<'a> = ();

    fn declare_extensions(builder: &mut HostExtensions<Self>, _shared: &Self::Shared<'_>) {
        builder.register::<HostGui>().register::<HostState>();
    }
}

/// Where CLAP plugins are installed.
pub fn clap_folders() -> Vec<PathBuf> {
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let mut dirs = Vec::new();
    if let Some(d) = var("COMMONPROGRAMFILES") {
        dirs.push(d.join("CLAP"));
    }
    if let Some(d) = var("LOCALAPPDATA") {
        dirs.push(d.join("Programs").join("Common").join("CLAP"));
    }
    dirs
}

fn is_clap(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("clap"))
}

fn find_in(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for e in read.flatten() {
        let path = e.path();
        if is_clap(&path) && path.is_file() {
            out.push(path);
        } else if depth > 0 && path.is_dir() {
            find_in(&path, depth - 1, out);
        }
    }
}

/// Every `.clap` file in `dirs` and the folders below them, sorted, each once. Nothing is loaded.
pub fn find_clap_files(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for d in dirs {
        find_in(d, SCAN_DEPTH, &mut found);
    }
    found.sort();
    found.dedup();
    found
}

fn load_entry(path: &Path) -> Result<PluginEntry, Error> {
    // SAFETY: loading a plugin runs its code, which nothing here can check. That is what a host
    // is for: the file is one the user installed as a plugin, and it stays loaded for as long as
    // the entry and the instances made from it live.
    unsafe { PluginEntry::load(path.as_os_str()) }
        .map_err(|e| Error(format!("cannot load {}: {e}", path.display())))
}

/// Loads a `.clap` file and reads what plugins it offers. This runs the file's code, so a plugin
/// that crashes here crashes the caller: scan in a process of its own.
pub fn scan_clap_file(path: &Path) -> Result<Vec<PluginInfo>, Error> {
    let entry = load_entry(path)?;
    let factory = entry
        .get_plugin_factory()
        .ok_or_else(|| Error(format!("{} has no plugin factory", path.display())))?;
    let text = |t: Option<&std::ffi::CStr>| {
        t.map_or_else(String::new, |t| t.to_string_lossy().into_owned())
    };
    Ok(factory
        .plugin_descriptors()
        .filter(|d| d.id().is_some())
        .map(|d| PluginInfo {
            id: text(d.id()),
            name: text(d.name()),
            vendor: text(d.vendor()),
            version: text(d.version()),
            format: PluginFormat::Clap,
            path: path.to_path_buf(),
            is_instrument: d.features().any(|f| f.to_bytes() == b"instrument"),
        })
        .collect())
}

/// The UI-thread half of a loaded plugin. It must outlive the processor [`load_clap`] returned
/// and stay on the UI thread, where CLAP wants it. Call [`LoadedClap::tick`] once a frame.
pub struct LoadedClap {
    /// Each parameter's id and range, by id.
    ranges: Vec<(u32, f64, f64)>,
    /// What the plugin's window moved.
    moved: rtrb::Consumer<(u32, f64)>,
    params: Vec<crate::PluginParam>,
    instance: PluginInstance<Handlers>,
    callback: Arc<AtomicBool>,
    released: Arc<AtomicBool>,
    latency: usize,
    gui: Arc<GuiRequests>,
    dirty: Arc<AtomicBool>,
    has_side: bool,
    #[cfg(windows)]
    editor: Option<OpenEditor>,
    /// Where the editor was when it last closed, until asked.
    closed_at: Option<(i32, i32)>,
}

/// A plugin's interface embedded in its window. Destroyed in that order: see `close_editor`.
#[cfg(windows)]
struct OpenEditor {
    gui: PluginGui,
    window: crate::editor_window::EditorWindow,
    /// The size the plugin was last told or asked for, so that the window's answer to a change
    /// is not taken for a new one.
    size: (u32, u32),
}

impl LoadedClap {
    /// Whether the audio half is gone. Keep this half until it is: an instance dropped while its
    /// audio half lives is never destroyed.
    pub fn released(&self) -> bool {
        self.released.load(Ordering::Acquire)
    }

    /// The delay the plugin reported when it was activated, in samples.
    pub fn latency_samples(&self) -> usize {
        self.latency
    }

    /// Runs what the plugin asked to have run on the main thread, and looks after its editor.
    pub fn tick(&mut self) {
        if self.callback.swap(false, Ordering::Acquire) {
            self.instance.call_on_main_thread_callback();
        }
        self.tick_editor();
    }

    /// The plugin's whole state, to give back with [`Self::load_state`]; `None` from a plugin that
    /// keeps none.
    pub fn save_state(&mut self) -> Result<Option<Vec<u8>>, Error> {
        let Some(state) = self
            .instance
            .plugin_shared_handle()
            .get_extension::<PluginState>()
        else {
            return Ok(None);
        };
        let mut bytes = Vec::new();
        state
            .save(&self.instance.plugin_handle(), &mut bytes)
            .map_err(|e| Error(e.to_string()))?;
        Ok(Some(bytes))
    }

    /// Puts back a state from [`Self::save_state`], perhaps of an earlier run of the program.
    pub fn load_state(&mut self, mut bytes: &[u8]) -> Result<(), Error> {
        let state = self
            .instance
            .plugin_shared_handle()
            .get_extension::<PluginState>()
            .ok_or_else(|| Error("the plugin cannot take back a saved state".to_string()))?;
        state
            .load(&self.instance.plugin_handle(), &mut bytes)
            .map_err(|e| Error(e.to_string()))?;
        self.dirty.store(false, Ordering::Release);
        Ok(())
    }

    /// Whether the plugin said its state changed, since last asked.
    pub fn take_dirty(&mut self) -> bool {
        self.dirty.swap(false, Ordering::AcqRel)
    }

    /// Whether the plugin's editor window is open.
    pub fn editor_is_open(&self) -> bool {
        #[cfg(windows)]
        return self.editor.is_some();
        #[cfg(not(windows))]
        false
    }

    /// Where the editor window was when it closed, once, to open there next time.
    pub fn take_closed_position(&mut self) -> Option<(i32, i32)> {
        self.closed_at.take()
    }

    /// Opens the plugin's own interface in a window above `owner`, or brings it to the front.
    #[cfg(windows)]
    pub fn open_editor(
        &mut self,
        title: &str,
        owner: Option<isize>,
        position: Option<(i32, i32)>,
    ) -> Result<(), Error> {
        use crate::editor_window::EditorWindow;
        if let Some(open) = &self.editor {
            open.window.raise();
            return Ok(());
        }
        let no_editor = || Error(format!("{title} has no editor window"));
        let gui = self
            .instance
            .plugin_shared_handle()
            .get_extension::<PluginGui>()
            .ok_or_else(no_editor)?;
        let config = GuiConfiguration {
            api_type: GuiApiType::WIN32,
            is_floating: false,
        };
        let plugin = self.instance.plugin_handle();
        if !gui.is_api_supported(&plugin, config) {
            return Err(no_editor());
        }
        gui.create(&plugin, config)
            .map_err(|e| Error(format!("cannot open {title}: {e}")))?;
        let made = (|| {
            let window = EditorWindow::new(title, gui.can_resize(&plugin), owner, position)?;
            // A plugin that sizes itself by the operating system says no to this, which is fine.
            let _ = gui.set_scale(&plugin, window.scale());
            let size = gui
                .get_size(&plugin)
                .ok_or_else(|| Error(format!("{title} did not say how big it is")))?;
            window.set_client_size(size.width, size.height);
            // SAFETY: the window outlives the plugin's interface: `close_editor` destroys the
            // interface first, and it is the only place the window is dropped.
            unsafe { gui.set_parent(&plugin, Window::from_win32_hwnd(window.handle())) }
                .map_err(|e| Error(format!("cannot open {title}: {e}")))?;
            window.show();
            // Some plugins answer no here and show themselves anyway: a failure here is only noted.
            if let Err(e) = gui.show(&plugin) {
                tracing::warn!("{title}: {e}");
            }
            let _ = window.take_new_size();
            Ok((window, (size.width, size.height)))
        })();
        match made {
            Ok((window, size)) => {
                self.editor = Some(OpenEditor { gui, window, size });
                Ok(())
            }
            Err(e) => {
                gui.destroy(&plugin);
                Err(e)
            }
        }
    }

    /// Without a window system to embed in, there is no editor.
    #[cfg(not(windows))]
    pub fn open_editor(
        &mut self,
        title: &str,
        _owner: Option<isize>,
        _position: Option<(i32, i32)>,
    ) -> Result<(), Error> {
        Err(Error(format!("{title} has no editor window here")))
    }

    /// Closes the editor, if it is open: the plugin's interface first, while its window lives.
    pub fn close_editor(&mut self) {
        #[cfg(windows)]
        if let Some(open) = self.editor.take() {
            if let Some(at) = open.window.position() {
                self.closed_at = Some(at);
            }
            open.window.hide();
            let plugin = self.instance.plugin_handle();
            let _ = open.gui.hide(&plugin);
            open.gui.destroy(&plugin);
        }
    }

    #[cfg(windows)]
    fn tick_editor(&mut self) {
        let Some(open) = self.editor.as_mut() else {
            return;
        };
        let closed = self.gui.closed.swap(false, Ordering::AcqRel);
        if closed || open.window.take_close_request() {
            self.close_editor();
            return;
        }
        // What the plugin asked for: the window follows.
        let asked = self.gui.resize.swap(0, Ordering::AcqRel);
        if asked != 0 {
            let size = GuiSize::unpack_from_u64(asked);
            open.size = (size.width, size.height);
            open.window.set_client_size(size.width, size.height);
            let _ = open.window.take_new_size();
        }
        // What the user dragged the frame to: the plugin says what it can do, and the window
        // takes that size if it differs.
        if let Some((width, height)) = open.window.take_new_size() {
            if (width, height) != open.size {
                let plugin = self.instance.plugin_handle();
                let wanted = GuiSize { width, height };
                let fits = open.gui.adjust_size(&plugin, wanted).unwrap_or(wanted);
                if open.gui.set_size(&plugin, fits).is_ok() {
                    open.size = (fits.width, fits.height);
                    if fits != wanted {
                        open.window.set_client_size(fits.width, fits.height);
                        let _ = open.window.take_new_size();
                    }
                }
            }
        }
    }

    #[cfg(not(windows))]
    fn tick_editor(&mut self) {}
}

impl Drop for LoadedClap {
    fn drop(&mut self) {
        self.close_editor();
    }
}

impl crate::LoadedPlugin for LoadedClap {
    fn params(&self) -> Vec<crate::PluginParam> {
        self.params.clone()
    }

    fn has_side_input(&self) -> bool {
        self.has_side
    }

    #[cfg(windows)]
    fn show_header(&mut self, state: &crate::HeaderState) {
        if let Some(open) = &self.editor {
            open.window.show_header(state);
        }
    }

    #[cfg(windows)]
    fn take_header_click(&mut self) -> Option<crate::HeaderClick> {
        self.editor.as_ref()?.window.take_header_click()
    }

    #[cfg(windows)]
    fn header_menu(&mut self, items: &[String]) -> Option<usize> {
        self.editor.as_ref()?.window.header_menu(items)
    }

    fn param_value(&mut self, id: u32) -> Option<f64> {
        let at = self.ranges.binary_search_by_key(&id, |r| r.0).ok()?;
        let (_, min, max) = self.ranges[at];
        let ext = self
            .instance
            .plugin_shared_handle()
            .get_extension::<PluginParams>()?;
        let value = ext.get_value(&self.instance.plugin_handle(), ClapId::new(id))?;
        Some(((value - min) / (max - min).max(1e-9)).clamp(0.0, 1.0))
    }

    fn take_edits(&mut self) -> Vec<(u32, f64)> {
        let mut out = Vec::new();
        while let Ok(edit) = self.moved.pop() {
            out.push(edit);
        }
        out
    }

    fn latency_samples(&self) -> usize {
        LoadedClap::latency_samples(self)
    }

    fn released(&self) -> bool {
        LoadedClap::released(self)
    }

    fn tick(&mut self) {
        LoadedClap::tick(self);
    }

    fn editor_is_open(&self) -> bool {
        LoadedClap::editor_is_open(self)
    }

    fn take_closed_position(&mut self) -> Option<(i32, i32)> {
        LoadedClap::take_closed_position(self)
    }

    fn open_editor(
        &mut self,
        title: &str,
        owner: Option<isize>,
        position: Option<(i32, i32)>,
    ) -> Result<(), Error> {
        LoadedClap::open_editor(self, title, owner, position)
    }

    fn close_editor(&mut self) {
        LoadedClap::close_editor(self);
    }

    fn save_state(&mut self) -> Result<Option<Vec<u8>>, Error> {
        LoadedClap::save_state(self)
    }

    fn load_state(&mut self, bytes: &[u8]) -> Result<(), Error> {
        LoadedClap::load_state(self, bytes)
    }

    fn take_dirty(&mut self) -> bool {
        LoadedClap::take_dirty(self)
    }
}

/// How many channels each audio port in one direction has.
fn port_layout(
    instance: &mut PluginInstance<Handlers>,
    is_input: bool,
) -> Result<Vec<usize>, Error> {
    let Some(ports) = instance
        .plugin_shared_handle()
        .get_extension::<PluginAudioPorts>()
    else {
        // No audio-ports extension means no audio ports.
        return Ok(Vec::new());
    };
    let plugin = instance.plugin_handle();
    let count = ports.count(&plugin, is_input) as usize;
    if count > MAX_PORTS {
        return Err(Error(format!("the plugin has {count} audio ports")));
    }
    let mut layout = Vec::with_capacity(count);
    let mut buffer = AudioPortInfoBuffer::new();
    for i in 0..count {
        let info = ports
            .get(&plugin, i as u32, is_input, &mut buffer)
            .ok_or_else(|| Error(format!("the plugin did not describe audio port {i}")))?;
        layout.push(info.channel_count as usize);
    }
    if layout.iter().sum::<usize>() > MAX_CHANNELS {
        return Err(Error("the plugin has too many audio channels".into()));
    }
    Ok(layout)
}

/// The input port for a side chain: the first with channels that is not the main one.
fn side_port(instance: &mut PluginInstance<Handlers>) -> Option<usize> {
    let ports = instance
        .plugin_shared_handle()
        .get_extension::<PluginAudioPorts>()?;
    let plugin = instance.plugin_handle();
    let mut buffer = AudioPortInfoBuffer::new();
    (0..ports.count(&plugin, true)).find_map(|i| {
        let info = ports.get(&plugin, i, true, &mut buffer)?;
        (!info.flags.contains(AudioPortFlags::IS_MAIN) && info.channel_count > 0)
            .then_some(i as usize)
    })
}

/// Loads plugin `id` of a `.clap` file and activates it for the engine's sample rate and block.
/// Failure is an error to show, never a panic.
pub fn load_clap(
    path: &Path,
    id: &str,
    sample_rate: u32,
) -> Result<(LoadedClap, Box<dyn PluginProcessor>), Error> {
    let entry = load_entry(path)?;
    let id = std::ffi::CString::new(id).map_err(|e| Error(e.to_string()))?;
    let callback = Arc::new(AtomicBool::new(false));
    let for_plugin = callback.clone();
    let gui = Arc::new(GuiRequests::default());
    let gui_for_plugin = gui.clone();
    let dirty = Arc::new(AtomicBool::new(false));
    let dirty_for_plugin = dirty.clone();
    let mut instance = PluginInstance::<Handlers>::new(
        move |_| Shared {
            callback: for_plugin,
            gui: gui_for_plugin,
        },
        move |_| MainThread {
            dirty: dirty_for_plugin,
        },
        &entry,
        &id,
        &host_info()?,
    )
    .map_err(|e| Error(format!("cannot start {}: {e}", path.display())))?;

    let inputs = port_layout(&mut instance, true)?;
    let outputs = port_layout(&mut instance, false)?;
    let side = side_port(&mut instance);
    let config = PluginAudioConfiguration {
        sample_rate: f64::from(sample_rate),
        min_frames_count: PLUGIN_BLOCK as u32,
        max_frames_count: PLUGIN_BLOCK as u32,
    };
    let stopped = instance
        .activate(|_, _| (), config)
        .map_err(|e| Error(format!("cannot activate {}: {e}", path.display())))?;
    // CLAP only answers this while activated: the delay depends on the sample rate.
    let latency = match instance
        .plugin_shared_handle()
        .get_extension::<PluginLatency>()
    {
        Some(latency) => latency.get(&instance.plugin_handle()) as usize,
        None => 0,
    };
    let released = Arc::new(AtomicBool::new(false));
    let (params, mut ranges) = clap_params(&mut instance);
    ranges.sort_by_key(|r| r.0);
    let mut processor =
        ClapProcessor::new(stopped.into(), &inputs, &outputs, latency, released.clone());
    processor.ranges = ranges.clone();
    processor.side_port = side;
    let (moved_in, moved) = rtrb::RingBuffer::new(PENDING_PARAMS);
    processor.moved = Some(moved_in);
    Ok((
        LoadedClap {
            ranges,
            moved,
            params,
            instance,
            callback,
            released,
            latency,
            gui,
            dirty,
            has_side: side.is_some(),
            #[cfg(windows)]
            editor: None,
            closed_at: None,
        },
        Box::new(processor),
    ))
}

/// Buffers for the channels of one direction's audio ports, made before the audio thread needs
/// them.
struct PortBuffers {
    /// Channels of each port.
    ports: Vec<Vec<Vec<f32>>>,
    /// The handles `clack` passes to the plugin.
    handles: AudioPorts,
}

impl PortBuffers {
    fn new(layout: &[usize]) -> Self {
        let channels = layout.iter().sum();
        Self {
            ports: layout
                .iter()
                .map(|&n| (0..n).map(|_| vec![0.0; PLUGIN_BLOCK]).collect())
                .collect(),
            handles: AudioPorts::with_capacity(channels, layout.len()),
        }
    }
}

/// Copies `src` into the first port: stereo into its first two channels, mixed down for a mono
/// port. Everything else (a side chain, say) is silent.
pub(crate) fn fill_main_input(ports: &mut [Vec<Vec<f32>>], src: [&[f32]; 2]) {
    let Some(main) = ports.first_mut() else {
        return;
    };
    if let [mono] = main.as_mut_slice() {
        for (m, (l, r)) in mono.iter_mut().zip(src[0].iter().zip(src[1])) {
            *m = (*l + *r) * 0.5;
        }
    } else if let [l, r, ..] = main.as_mut_slice() {
        l.copy_from_slice(src[0]);
        r.copy_from_slice(src[1]);
    }
}

/// Copies `src` into the side-chain port `index`: stereo into its first two channels, mixed down
/// for a mono port.
pub(crate) fn fill_side_input(ports: &mut [Vec<Vec<f32>>], index: usize, src: [&[f32]; 2]) {
    let Some(side) = ports.get_mut(index) else {
        return;
    };
    if let [mono] = side.as_mut_slice() {
        for (m, (l, r)) in mono.iter_mut().zip(src[0].iter().zip(src[1])) {
            *m = (*l + *r) * 0.5;
        }
    } else if let [l, r, ..] = side.as_mut_slice() {
        l.copy_from_slice(src[0]);
        r.copy_from_slice(src[1]);
    }
}

/// Copies the first port to `dst`: a mono port goes to both sides, no port gives silence.
pub(crate) fn read_main_output(ports: &[Vec<Vec<f32>>], dst: [&mut [f32]; 2]) {
    let [dst_l, dst_r] = dst;
    match ports.first().map(Vec::as_slice) {
        Some([l, r, ..]) => {
            dst_l.copy_from_slice(l);
            dst_r.copy_from_slice(r);
        }
        Some([mono]) => {
            dst_l.copy_from_slice(mono);
            dst_r.copy_from_slice(mono);
        }
        _ => {
            dst_l.fill(0.0);
            dst_r.fill(0.0);
        }
    }
}

/// CLAP's events for one of ours, on the plugin's first note port and channel.
fn push_event(buffer: &mut EventBuffer, event: &PluginEvent) {
    let time = event.frame;
    let on_key = |key: u8| Pckn::new(0u16, 0u16, u16::from(key), Match::All);
    match event.kind {
        PluginEventKind::NoteOn { key, velocity } => {
            buffer.push(&NoteOnEvent::new(
                time,
                on_key(key),
                f64::from(velocity.clamp(0.0, 1.0)),
            ));
        }
        PluginEventKind::NoteOff { key } => {
            buffer.push(&NoteOffEvent::new(time, on_key(key), 0.0));
        }
        PluginEventKind::Control { number, value } => {
            buffer.push(&MidiEvent::new(
                time,
                0,
                [0xB0, number & 0x7f, value & 0x7f],
            ));
        }
        PluginEventKind::PitchBend { value } => {
            let bend = ((value.clamp(-1.0, 1.0) + 1.0) * 8191.5) as u16;
            buffer.push(&MidiEvent::new(
                time,
                0,
                [0xE0, (bend & 0x7f) as u8, (bend >> 7) as u8],
            ));
        }
    }
}

/// CLAP's words for where the song is.
fn transport_event(now: &PluginTransport) -> TransportEvent {
    let mut flags = TransportFlags::HAS_TEMPO
        | TransportFlags::HAS_BEATS_TIMELINE
        | TransportFlags::HAS_SECONDS_TIMELINE
        | TransportFlags::HAS_TIME_SIGNATURE;
    if now.playing {
        flags |= TransportFlags::IS_PLAYING;
    }
    if now.cycle.is_some() {
        flags |= TransportFlags::IS_LOOP_ACTIVE;
    }
    let (loop_start, loop_end) = now.cycle.unwrap_or((0.0, 0.0));
    TransportEvent {
        header: EventHeader::new_core(0, EventFlags::empty()),
        flags,
        song_pos_beats: BeatTime::from_float(now.beat),
        song_pos_seconds: SecondsTime::from_float(now.seconds),
        tempo: now.tempo,
        tempo_inc: 0.0,
        loop_start_beats: BeatTime::from_float(loop_start),
        loop_end_beats: BeatTime::from_float(loop_end),
        loop_start_seconds: SecondsTime::from_float(0.0),
        loop_end_seconds: SecondsTime::from_float(0.0),
        bar_start: BeatTime::from_float(now.bar_start),
        bar_number: now.bar_number,
        time_signature_numerator: u16::from(now.signature.0),
        time_signature_denominator: u16::from(now.signature.1),
    }
}

/// Parameter moves one block can carry.
const PENDING_PARAMS: usize = 256;

/// The parameters that can be automated, and their ranges.
fn clap_params(
    instance: &mut PluginInstance<Handlers>,
) -> (Vec<crate::PluginParam>, Vec<(u32, f64, f64)>) {
    let Some(ext) = instance
        .plugin_shared_handle()
        .get_extension::<PluginParams>()
    else {
        return (Vec::new(), Vec::new());
    };
    let plugin = instance.plugin_handle();
    let mut buffer = ParamInfoBuffer::new();
    let (mut params, mut ranges) = (Vec::new(), Vec::new());
    for index in 0..ext.count(&plugin) {
        let Some(info) = ext.get_info(&plugin, index, &mut buffer) else {
            continue;
        };
        if !info.flags.contains(ParamInfoFlags::IS_AUTOMATABLE) {
            continue;
        }
        let end = info
            .name
            .iter()
            .position(|b| *b == 0)
            .unwrap_or(info.name.len());
        params.push(crate::PluginParam {
            id: info.id.get(),
            name: String::from_utf8_lossy(&info.name[..end]).into_owned(),
        });
        ranges.push((info.id.get(), info.min_value, info.max_value));
    }
    (params, ranges)
}

/// The audio-thread half of a loaded CLAP plugin.
struct ClapProcessor {
    processor: PluginAudioProcessor<Handlers>,
    input: PortBuffers,
    output: PortBuffers,
    events_in: EventBuffer,
    events_out: EventBuffer,
    latency: usize,
    released: Arc<AtomicBool>,
    /// Each parameter's id and range, by id: CLAP's values are in the parameter's own units.
    ranges: Vec<(u32, f64, f64)>,
    /// Moves waiting for the next block. Past its capacity they wait for the next value.
    pending: Vec<(u32, f64)>,
    /// What the plugin's window moved, as positions 0..=1, for automation. Lost if nobody reads.
    moved: Option<rtrb::Producer<(u32, f64)>>,
    /// The input port a side chain goes in.
    side_port: Option<usize>,
}

impl Drop for ClapProcessor {
    fn drop(&mut self) {
        self.released.store(true, Ordering::Release);
    }
}

impl ClapProcessor {
    fn new(
        processor: PluginAudioProcessor<Handlers>,
        inputs: &[usize],
        outputs: &[usize],
        latency: usize,
        released: Arc<AtomicBool>,
    ) -> Self {
        Self {
            ranges: Vec::new(),
            pending: Vec::with_capacity(PENDING_PARAMS),
            moved: None,
            side_port: None,
            processor,
            input: PortBuffers::new(inputs),
            output: PortBuffers::new(outputs),
            events_in: EventBuffer::with_capacity(MAX_PLUGIN_EVENTS),
            events_out: EventBuffer::with_capacity(MAX_PLUGIN_EVENTS),
            latency,
            released,
        }
    }
}

impl PluginProcessor for ClapProcessor {
    fn set_side_input(&mut self, side: [&[f32]; 2]) {
        if let Some(port) = self.side_port {
            fill_side_input(&mut self.input.ports, port, side);
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        let Ok(at) = self.ranges.binary_search_by_key(&id, |r| r.0) else {
            return;
        };
        if self.pending.len() < self.pending.capacity() {
            let (_, min, max) = self.ranges[at];
            self.pending
                .push((id, min + value.clamp(0.0, 1.0) * (max - min)));
        }
    }

    fn process(
        &mut self,
        input: [&[f32]; 2],
        output: [&mut [f32]; 2],
        events: &[PluginEvent],
        transport: &PluginTransport,
    ) {
        let transport = transport_event(transport);
        // A plugin whose only input is its side chain has no main one to fill.
        if self.side_port != Some(0) {
            fill_main_input(&mut self.input.ports, input);
        }
        self.events_in.clear();
        self.events_out.clear();
        for (id, value) in self.pending.drain(..) {
            let id = ClapId::new(id);
            self.events_in
                .push(&ParamValueEvent::new(0, id, Pckn::match_all(), value));
        }
        for e in events {
            push_event(&mut self.events_in, e);
        }
        let ok = match self.processor.ensure_processing_started() {
            Ok(started) => {
                let audio_in =
                    self.input
                        .handles
                        .with_input_buffers(self.input.ports.iter_mut().map(|channels| {
                            AudioPortBuffer {
                                latency: 0,
                                channels: AudioPortBufferType::f32_input_only(
                                    channels
                                        .iter_mut()
                                        .map(|c| InputChannel::variable(c.as_mut_slice())),
                                ),
                            }
                        }));
                let mut audio_out =
                    self.output
                        .handles
                        .with_output_buffers(self.output.ports.iter_mut().map(|channels| {
                            AudioPortBuffer {
                                latency: 0,
                                channels: AudioPortBufferType::f32_output_only(
                                    channels.iter_mut().map(Vec::as_mut_slice),
                                ),
                            }
                        }));
                started
                    .process(
                        &audio_in,
                        &mut audio_out,
                        &self.events_in.as_input(),
                        &mut self.events_out.as_output(),
                        None,
                        Some(&transport),
                    )
                    .is_ok()
            }
            Err(_) => false,
        };
        if let Some(moved) = self.moved.as_mut() {
            for event in self.events_out.iter() {
                let Some(CoreEventSpace::ParamValue(param)) = event.as_core_event() else {
                    continue;
                };
                let Some(id) = param.param_id() else {
                    continue;
                };
                let id = id.get();
                if let Ok(at) = self.ranges.binary_search_by_key(&id, |r| r.0) {
                    let (_, min, max) = self.ranges[at];
                    let position = (param.value() - min) / (max - min).max(1e-9);
                    let _ = moved.push((id, position.clamp(0.0, 1.0)));
                }
            }
        }
        if ok {
            read_main_output(&self.output.ports, output);
        } else {
            let [l, r] = output;
            l.fill(0.0);
            r.fill(0.0);
        }
    }

    fn latency_samples(&self) -> usize {
        self.latency
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn clap_files_are_found_in_vendor_folders_and_nothing_else_is() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let vendor = root.join("Vendor").join("Sub");
        std::fs::create_dir_all(&vendor).unwrap();
        std::fs::write(root.join("B.clap"), b"").unwrap();
        std::fs::write(root.join("Vendor").join("a.CLAP"), b"").unwrap();
        std::fs::write(vendor.join("c.clap"), b"").unwrap();
        std::fs::write(root.join("notes.txt"), b"").unwrap();
        std::fs::create_dir(root.join("Folder.clap")).unwrap();
        let found = find_clap_files(&[root.to_path_buf(), root.to_path_buf()]);
        let names: Vec<_> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["B.clap", "c.clap", "a.CLAP"]);
        assert!(find_clap_files(&[root.join("gone")]).is_empty());
    }

    #[test]
    fn a_file_that_is_not_a_plugin_is_an_error_not_a_crash() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("fake.clap");
        std::fs::write(&fake, b"not a plugin").unwrap();
        assert!(scan_clap_file(&fake).is_err());
        assert!(load_clap(&fake, "x", 48_000).is_err());
    }

    #[test]
    fn audio_goes_in_and_comes_out_for_any_port_layout() {
        let (l, r) = (vec![0.5; PLUGIN_BLOCK], vec![0.25; PLUGIN_BLOCK]);
        let (mut out_l, mut out_r) = (vec![1.0; PLUGIN_BLOCK], vec![1.0; PLUGIN_BLOCK]);

        let mut stereo = PortBuffers::new(&[2, 2]);
        fill_main_input(&mut stereo.ports, [&l, &r]);
        assert_eq!(stereo.ports[0][0][0], 0.5);
        assert_eq!(stereo.ports[0][1][0], 0.25);
        assert_eq!(stereo.ports[1][0][0], 0.0, "a side chain stays silent");
        read_main_output(&stereo.ports, [&mut out_l, &mut out_r]);
        assert_eq!((out_l[0], out_r[0]), (0.5, 0.25));

        let mut mono = PortBuffers::new(&[1]);
        fill_main_input(&mut mono.ports, [&l, &r]);
        assert_eq!(mono.ports[0][0][0], 0.375);
        read_main_output(&mono.ports, [&mut out_l, &mut out_r]);
        assert_eq!((out_l[0], out_r[0]), (0.375, 0.375));

        let mut none = PortBuffers::new(&[]);
        fill_main_input(&mut none.ports, [&l, &r]);
        read_main_output(&none.ports, [&mut out_l, &mut out_r]);
        assert_eq!((out_l[0], out_r[0]), (0.0, 0.0));
    }

    #[test]
    fn a_side_chain_goes_to_its_own_port_for_any_layout() {
        let (l, r) = (vec![0.5; PLUGIN_BLOCK], vec![0.25; PLUGIN_BLOCK]);
        let mut stereo = PortBuffers::new(&[2, 2]);
        fill_side_input(&mut stereo.ports, 1, [&l, &r]);
        assert_eq!(stereo.ports[0][0][0], 0.0, "not the main input");
        assert_eq!(stereo.ports[1][0][0], 0.5);
        assert_eq!(stereo.ports[1][1][0], 0.25);
        let mut mono = PortBuffers::new(&[2, 1]);
        fill_side_input(&mut mono.ports, 1, [&l, &r]);
        assert_eq!(mono.ports[1][0][0], 0.375);
        // A port that is not there is no crash.
        fill_side_input(&mut mono.ports, 5, [&l, &r]);
    }

    #[test]
    fn where_the_song_is_goes_over_in_clap_terms() {
        let event = transport_event(&PluginTransport {
            playing: true,
            beat: 5.5,
            seconds: 2.75,
            tempo: 120.0,
            signature: (6, 8),
            bar_start: 3.0,
            bar_number: 1,
            cycle: Some((4.0, 8.0)),
        });
        assert!(event.flags.contains(TransportFlags::IS_PLAYING));
        assert!(event.flags.contains(TransportFlags::IS_LOOP_ACTIVE));
        assert_eq!(event.song_pos_beats.to_float(), 5.5);
        assert_eq!(event.song_pos_seconds.to_float(), 2.75);
        assert_eq!(event.time_signature_numerator, 6);
        assert_eq!(event.time_signature_denominator, 8);
        assert_eq!(event.bar_number, 1);
        assert_eq!(event.loop_end_beats.to_float(), 8.0);
        let stopped = transport_event(&PluginTransport::default());
        assert!(!stopped.flags.contains(TransportFlags::IS_PLAYING));
        assert!(!stopped.flags.contains(TransportFlags::IS_LOOP_ACTIVE));
    }

    #[test]
    fn events_become_clap_events_at_their_frame() {
        let mut buffer = EventBuffer::with_capacity(8);
        for kind in [
            PluginEventKind::NoteOn {
                key: 60,
                velocity: 0.5,
            },
            PluginEventKind::NoteOff { key: 60 },
            PluginEventKind::Control {
                number: 1,
                value: 64,
            },
            PluginEventKind::PitchBend { value: 0.0 },
        ] {
            push_event(&mut buffer, &PluginEvent { frame: 9, kind });
        }
        assert_eq!(buffer.len(), 4);
        assert!(buffer.iter().all(|e| e.header().time() == 9));
    }

    /// Opens real editors on the screen, which only some machines have, and closes them as a user
    /// would. `cargo test -p prowave-host installed_editors -- --ignored --nocapture`, with
    /// `PROWAVE_TEST_CLAP` naming a `.clap` file, or else every plugin installed is tried.
    #[cfg(windows)]
    #[test]
    #[ignore = "opens the editors of the plugins installed on this computer"]
    fn installed_editors_open_embed_and_close() {
        use crate::editor_window::pump_messages_for;
        use std::time::Duration;
        let files = match std::env::var_os("PROWAVE_TEST_CLAP") {
            Some(one) => vec![PathBuf::from(one)],
            None => find_clap_files(&clap_folders()),
        };
        assert!(!files.is_empty(), "no CLAP plugins here");
        for file in files {
            for info in scan_clap_file(&file).unwrap() {
                let (mut loaded, processor) = load_clap(&info.path, &info.id, 48_000).unwrap();
                match loaded.open_editor(&info.name, None, Some((60, 60))) {
                    Ok(()) => {}
                    Err(e) => {
                        println!("{}: {e}", info.name);
                        continue;
                    }
                }
                assert!(loaded.editor_is_open());
                pump_messages_for(Duration::from_millis(400));
                loaded.tick();
                let open = loaded.editor.as_ref().unwrap();
                let (visible, size, embedded) = open.window.describe();
                println!("{}: {}x{}, embedded {embedded}", info.name, size.0, size.1);
                assert!(visible, "{} is on the screen", info.name);
                assert!(size.0 > 0 && size.1 > 0, "{} has a size", info.name);
                assert!(embedded, "{} made its window inside ours", info.name);

                // Asking twice does not make a second window.
                loaded.open_editor(&info.name, None, None).unwrap();

                // The close button, as the user presses it.
                loaded.editor.as_ref().unwrap().window.request_close();
                loaded.tick();
                assert!(!loaded.editor_is_open(), "{} closed", info.name);
                assert_eq!(loaded.take_closed_position().map(|_| ()), Some(()));
                pump_messages_for(Duration::from_millis(50));

                // And again, then gone with the plugin while open.
                loaded.open_editor(&info.name, None, None).unwrap();
                pump_messages_for(Duration::from_millis(50));
                drop(processor);
                drop(loaded);
                pump_messages_for(Duration::from_millis(50));
            }
        }
    }

    /// Runs on real plugins, which only some machines have, and a crash in one ends the test run.
    /// `cargo test -p prowave-host installed -- --ignored --nocapture`, with `PROWAVE_TEST_CLAP`
    /// naming a `.clap` file, or else every plugin installed is tried.
    #[test]
    #[ignore = "loads the plugins installed on this computer"]
    fn installed_plugins_load_and_process_silence_into_finite_audio() {
        let files = match std::env::var_os("PROWAVE_TEST_CLAP") {
            Some(one) => vec![PathBuf::from(one)],
            None => find_clap_files(&clap_folders()),
        };
        assert!(!files.is_empty(), "no CLAP plugins here");
        for file in files {
            for info in scan_clap_file(&file).unwrap() {
                println!("{} / {} ({})", info.vendor, info.name, info.id);
                let (mut loaded, mut processor) = load_clap(&info.path, &info.id, 48_000).unwrap();
                // A 220 Hz tone, long enough to come through whatever the plugin delays by.
                let blocks = (loaded.latency_samples() + 48_000) / PLUGIN_BLOCK;
                let (mut l, mut r) = (vec![9.0; PLUGIN_BLOCK], vec![9.0; PLUGIN_BLOCK]);
                let mut peak = 0f32;
                for b in 0..blocks {
                    let tone: Vec<f32> = (0..PLUGIN_BLOCK)
                        .map(|i| {
                            let t = (b * PLUGIN_BLOCK + i) as f32 / 48_000.0;
                            (t * 220.0 * std::f32::consts::TAU).sin() * 0.25
                        })
                        .collect();
                    processor.process(
                        [&tone, &tone],
                        [&mut l, &mut r],
                        &[],
                        &PluginTransport::default(),
                    );
                    loaded.tick();
                    assert!(l.iter().chain(&r).all(|x| x.is_finite()), "{}", info.name);
                    peak = l.iter().chain(&r).fold(peak, |m, x| m.max(x.abs()));
                }
                println!("  latency {}, peak {peak:.3}", loaded.latency_samples());

                // What it saves, a new instance takes back and saves again.
                if let Some(state) = loaded.save_state().unwrap() {
                    let (mut other, other_audio) = load_clap(&info.path, &info.id, 48_000).unwrap();
                    other.load_state(&state).unwrap();
                    assert!(!other.take_dirty(), "loading is not an edit");
                    assert!(other.save_state().unwrap().is_some());
                    println!("  state {} bytes", state.len());
                    drop(other_audio);
                }
                assert!(!loaded.released());
                drop(processor);
                assert!(loaded.released(), "the audio half says it has gone");
                drop(loaded);
            }
        }
    }
}
