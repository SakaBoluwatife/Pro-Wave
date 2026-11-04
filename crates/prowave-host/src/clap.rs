//! CLAP plugins, through the `clack-host` bindings.
use crate::{HostError, PluginFormat, PluginInfo};
use clack_extensions::audio_ports::{AudioPortInfoBuffer, PluginAudioPorts};
use clack_extensions::latency::PluginLatency;
use clack_host::events::event_types::{MidiEvent, NoteOffEvent, NoteOnEvent};
use clack_host::events::Match;
use clack_host::prelude::*;
use prowave_engine::{
    PluginEvent, PluginEventKind, PluginProcessor, MAX_PLUGIN_EVENTS, PLUGIN_BLOCK,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Folders inside a CLAP folder that are searched too (vendors keep plugins in subfolders).
const SCAN_DEPTH: usize = 3;
/// A plugin with more audio ports than this is not loaded: all of them need buffers.
const MAX_PORTS: usize = 16;
const MAX_CHANNELS: usize = 64;

fn host_info() -> Result<HostInfo, HostError> {
    HostInfo::new(
        "Pro Wave",
        "Saka Audio",
        "https://github.com/SakaBoluwatife/Pro-Wave",
        env!("CARGO_PKG_VERSION"),
    )
    .map_err(|e| HostError(e.to_string()))
}

/// What the plugin can ask of the host from any thread. It is answered on the UI thread.
struct Shared {
    callback: Arc<AtomicBool>,
}

impl SharedHandler<'_> for Shared {
    fn request_restart(&self) {}
    fn request_process(&self) {}
    fn request_callback(&self) {
        self.callback.store(true, Ordering::Release);
    }
}

struct Handlers;

impl HostHandlers for Handlers {
    type Shared<'a> = Shared;
    type MainThread<'a> = ();
    type AudioProcessor<'a> = ();
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

fn load_entry(path: &Path) -> Result<PluginEntry, HostError> {
    // SAFETY: loading a plugin runs its code, which nothing here can check. That is what a host
    // is for: the file is one the user installed as a plugin, and it stays loaded for as long as
    // the entry and the instances made from it live.
    unsafe { PluginEntry::load(path.as_os_str()) }
        .map_err(|e| HostError(format!("cannot load {}: {e}", path.display())))
}

/// Loads a `.clap` file and reads what plugins it offers. This runs the file's code, so a plugin
/// that crashes here crashes the caller: scan in a process of its own.
pub fn scan_clap_file(path: &Path) -> Result<Vec<PluginInfo>, HostError> {
    let entry = load_entry(path)?;
    let factory = entry
        .get_plugin_factory()
        .ok_or_else(|| HostError(format!("{} has no plugin factory", path.display())))?;
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
    instance: PluginInstance<Handlers>,
    callback: Arc<AtomicBool>,
    released: Arc<AtomicBool>,
    latency: usize,
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

    /// Runs what the plugin asked to have run on the main thread.
    pub fn tick(&mut self) {
        if self.callback.swap(false, Ordering::Acquire) {
            self.instance.call_on_main_thread_callback();
        }
    }
}

/// How many channels each audio port in one direction has.
fn port_layout(
    instance: &mut PluginInstance<Handlers>,
    is_input: bool,
) -> Result<Vec<usize>, HostError> {
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
        return Err(HostError(format!("the plugin has {count} audio ports")));
    }
    let mut layout = Vec::with_capacity(count);
    let mut buffer = AudioPortInfoBuffer::new();
    for i in 0..count {
        let info = ports
            .get(&plugin, i as u32, is_input, &mut buffer)
            .ok_or_else(|| HostError(format!("the plugin did not describe audio port {i}")))?;
        layout.push(info.channel_count as usize);
    }
    if layout.iter().sum::<usize>() > MAX_CHANNELS {
        return Err(HostError("the plugin has too many audio channels".into()));
    }
    Ok(layout)
}

/// Loads plugin `id` of a `.clap` file and activates it for the engine's sample rate and block.
/// Failure is an error to show, never a panic.
pub fn load_clap(
    path: &Path,
    id: &str,
    sample_rate: u32,
) -> Result<(LoadedClap, Box<dyn PluginProcessor>), HostError> {
    let entry = load_entry(path)?;
    let id = std::ffi::CString::new(id).map_err(|e| HostError(e.to_string()))?;
    let callback = Arc::new(AtomicBool::new(false));
    let for_plugin = callback.clone();
    let mut instance = PluginInstance::<Handlers>::new(
        move |_| Shared {
            callback: for_plugin,
        },
        |_| (),
        &entry,
        &id,
        &host_info()?,
    )
    .map_err(|e| HostError(format!("cannot start {}: {e}", path.display())))?;

    let inputs = port_layout(&mut instance, true)?;
    let outputs = port_layout(&mut instance, false)?;
    let config = PluginAudioConfiguration {
        sample_rate: f64::from(sample_rate),
        min_frames_count: PLUGIN_BLOCK as u32,
        max_frames_count: PLUGIN_BLOCK as u32,
    };
    let stopped = instance
        .activate(|_, _| (), config)
        .map_err(|e| HostError(format!("cannot activate {}: {e}", path.display())))?;
    // CLAP only answers this while activated: the delay depends on the sample rate.
    let latency = match instance
        .plugin_shared_handle()
        .get_extension::<PluginLatency>()
    {
        Some(latency) => latency.get(&instance.plugin_handle()) as usize,
        None => 0,
    };
    let released = Arc::new(AtomicBool::new(false));
    let processor =
        ClapProcessor::new(stopped.into(), &inputs, &outputs, latency, released.clone());
    Ok((
        LoadedClap {
            instance,
            callback,
            released,
            latency,
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
fn fill_main_input(ports: &mut [Vec<Vec<f32>>], src: [&[f32]; 2]) {
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

/// Copies the first port to `dst`: a mono port goes to both sides, no port gives silence.
fn read_main_output(ports: &[Vec<Vec<f32>>], dst: [&mut [f32]; 2]) {
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

/// The audio-thread half of a loaded CLAP plugin.
struct ClapProcessor {
    processor: PluginAudioProcessor<Handlers>,
    input: PortBuffers,
    output: PortBuffers,
    events_in: EventBuffer,
    events_out: EventBuffer,
    latency: usize,
    released: Arc<AtomicBool>,
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
    fn process(&mut self, input: [&[f32]; 2], output: [&mut [f32]; 2], events: &[PluginEvent]) {
        fill_main_input(&mut self.input.ports, input);
        self.events_in.clear();
        self.events_out.clear();
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
                        None,
                    )
                    .is_ok()
            }
            Err(_) => false,
        };
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
                    processor.process([&tone, &tone], [&mut l, &mut r], &[]);
                    loaded.tick();
                    assert!(l.iter().chain(&r).all(|x| x.is_finite()), "{}", info.name);
                    peak = l.iter().chain(&r).fold(peak, |m, x| m.max(x.abs()));
                }
                println!("  latency {}, peak {peak:.3}", loaded.latency_samples());
                assert!(!loaded.released());
                drop(processor);
                assert!(loaded.released(), "the audio half says it has gone");
                drop(loaded);
            }
        }
    }
}
