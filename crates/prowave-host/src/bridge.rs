//! Plugins in a process of their own, so that one that crashes does not take the program with it.
//!
//! Each plugin file gets one helper process: a copy of this program started with `--plugin-host`,
//! holding every instance from that file. A crash there ends those instances and no others. This
//! is crash isolation only, not a security sandbox: the helper has the same rights as the program.
//!
//! Two paths connect the halves:
//! - **Control**, on the UI thread: framed messages over the helper's stdin and stdout. It answers
//!   requests, and sends what changed in its plugins by itself, so a frame needs no round trip.
//! - **Audio**: a block of shared memory per instance. The audio thread writes a block, signals,
//!   and spins for the answer up to a deadline. It takes no lock and makes no blocking wait. A
//!   block that is late is silence; a helper that is gone passes the sound through.
//!
//! Both halves are the same program, so plain data is shared as it is laid out in memory.
use crate::{
    load_plugin, HeaderClick, HeaderState, HostError, Loaded, LoadedPlugin, PluginFormat,
    PluginParam,
};
use prowave_engine::{PluginEvent, PluginProcessor, PluginTransport, MAX_PLUGIN_EVENTS};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::mem::MaybeUninit;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::System::Memory::{
    CreateFileMappingW, MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, FILE_MAP_ALL_ACCESS,
    MEMORY_MAPPED_VIEW_ADDRESS, PAGE_READWRITE,
};
use windows::Win32::System::Threading::{
    CreateEventW, OpenEventW, SetEvent, WaitForSingleObject, EVENT_ALL_ACCESS,
};

/// The argument that starts a helper, followed by the plugin block size.
const HOST_ARG: &str = "--plugin-host";
/// Parameter moves one block can carry.
const MAX_PARAMS: usize = 256;
/// How long the audio thread waits for a block before giving up on it. Longer than this and the
/// audio has dropped out anyway.
const REALTIME_WAIT: Duration = Duration::from_millis(20);
/// An offline render has no hurry, and must not lose a block to a slow plugin.
const OFFLINE_WAIT: Duration = Duration::from_secs(600);
/// A block unanswered this long while playing: the plugin has hung, and its helper is ended.
const HUNG_AFTER: Duration = Duration::from_secs(10);
/// Loading can show a license dialog or read a large library.
const LOAD_WAIT: Duration = Duration::from_secs(15 * 60);
const SHORT_WAIT: Duration = Duration::from_secs(20);
const MENU_WAIT: Duration = Duration::from_secs(10 * 60);

static ISOLATED: AtomicBool = AtomicBool::new(false);
static HOST_EXE: Mutex<Option<PathBuf>> = Mutex::new(None);
static NEXT_NAME: AtomicU64 = AtomicU64::new(1);

/// Whether plugins load in a process of their own from now on.
pub fn set_isolation(on: bool) {
    ISOLATED.store(on, Ordering::Relaxed);
}

pub fn isolation_on() -> bool {
    ISOLATED.load(Ordering::Relaxed)
}

/// The program to start as a helper, in place of this one. For tests, whose own program is not it.
pub fn set_host_exe(exe: PathBuf) {
    if let Ok(mut slot) = HOST_EXE.lock() {
        *slot = Some(exe);
    }
}

/// The plugin block size to use, when this copy was started as a helper.
pub fn host_request(args: &[std::ffi::OsString]) -> Option<usize> {
    let at = args.iter().position(|a| a == HOST_ARG)?;
    args.get(at + 1)?.to_str()?.parse().ok()
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

// ------------------------------------------------------------------ the wire

#[derive(Default)]
struct Out(Vec<u8>);

impl Out {
    fn new(tag: u8) -> Self {
        Self(vec![tag])
    }
    fn u8(mut self, v: u8) -> Self {
        self.0.push(v);
        self
    }
    fn bool(self, v: bool) -> Self {
        self.u8(u8::from(v))
    }
    fn u32(mut self, v: u32) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn i32(self, v: i32) -> Self {
        self.u32(v as u32)
    }
    fn f64(mut self, v: f64) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn bytes(self, v: &[u8]) -> Self {
        let mut me = self.u32(v.len() as u32);
        me.0.extend_from_slice(v);
        me
    }
    fn text(self, v: &str) -> Self {
        self.bytes(v.as_bytes())
    }
}

struct In<'a>(&'a [u8]);

impl In<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Some(head)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    fn bool(&mut self) -> Option<bool> {
        self.u8().map(|b| b != 0)
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4)?.try_into().ok().map(u32::from_le_bytes)
    }
    fn i32(&mut self) -> Option<i32> {
        self.u32().map(|v| v as i32)
    }
    fn f64(&mut self) -> Option<f64> {
        self.take(8)?.try_into().ok().map(f64::from_le_bytes)
    }
    fn bytes(&mut self) -> Option<Vec<u8>> {
        let n = self.u32()? as usize;
        self.take(n).map(<[u8]>::to_vec)
    }
    fn text(&mut self) -> Option<String> {
        String::from_utf8(self.bytes()?).ok()
    }
}

fn write_frame(to: &mut impl Write, payload: &[u8]) -> std::io::Result<()> {
    to.write_all(&(payload.len() as u32).to_le_bytes())?;
    to.write_all(payload)?;
    to.flush()
}

fn read_frame(from: &mut impl Read) -> Option<Vec<u8>> {
    let mut len = [0u8; 4];
    from.read_exact(&mut len).ok()?;
    let mut payload = vec![0u8; u32::from_le_bytes(len) as usize];
    from.read_exact(&mut payload).ok()?;
    Some(payload)
}

// What the program asks. Every request carries an id and is answered with `REPLY`, the id, and an
// ok flag; the others are not answered.
const LOAD: u8 = 1;
const SAVE_STATE: u8 = 2;
const LOAD_STATE: u8 = 3;
const PARAM_VALUE: u8 = 5;
const HEADER_MENU: u8 = 6;
const DESTROY: u8 = 10;
const OPEN_EDITOR: u8 = 14;
const CLOSE_EDITOR: u8 = 11;
const SHOW_PARAM: u8 = 12;
const SHOW_HEADER: u8 = 13;
// What the helper says.
const REPLY: u8 = 100;
const NOTES: u8 = 101;
const FAILED: u8 = 102;

fn click_code(click: Option<HeaderClick>) -> u8 {
    match click {
        None => 0,
        Some(HeaderClick::Bypass) => 1,
        Some(HeaderClick::Presets) => 2,
        Some(HeaderClick::Compare) => 3,
        Some(HeaderClick::CopyCompare) => 4,
    }
}

fn click_of(code: u8) -> Option<HeaderClick> {
    match code {
        1 => Some(HeaderClick::Bypass),
        2 => Some(HeaderClick::Presets),
        3 => Some(HeaderClick::Compare),
        4 => Some(HeaderClick::CopyCompare),
        _ => None,
    }
}

// ------------------------------------------------------------------ shared memory

#[repr(C)]
struct Header {
    /// The block last handed over, and the block last answered.
    go: AtomicU64,
    done: AtomicU64,
    quit: AtomicU32,
    events: u32,
    params: u32,
    has_side: u32,
    transport: MaybeUninit<PluginTransport>,
    event_list: MaybeUninit<[PluginEvent; MAX_PLUGIN_EVENTS]>,
    param_list: MaybeUninit<[(u32, f64); MAX_PARAMS]>,
}

/// One instance's shared memory: the header, then six buffers of a block each.
struct Shm {
    map: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
    signal: HANDLE,
    block: usize,
    /// In an offline render, so no deadline to speak of.
    offline: AtomicBool,
}

// SAFETY: the handles may be used from any thread, and the view stays mapped as long as this lives.
// Who touches which part of the memory, and when, is settled by `go` and `done`.
unsafe impl Send for Shm {}
unsafe impl Sync for Shm {}

const IN_L: usize = 0;
const IN_R: usize = 1;
const SIDE_L: usize = 2;
const SIDE_R: usize = 3;
const OUT_L: usize = 4;
const OUT_R: usize = 5;

impl Shm {
    fn size(block: usize) -> usize {
        std::mem::size_of::<Header>() + 6 * block * std::mem::size_of::<f32>()
    }

    /// Makes the memory, cleared to zero by the system.
    fn create(name: &str, block: usize) -> Result<Self, HostError> {
        let failed = |e: windows::core::Error| HostError(format!("no shared memory: {e}"));
        let (map_name, event_name) = (wide(&format!("{name}-m")), wide(&format!("{name}-e")));
        let size = Self::size(block);
        // SAFETY: the names outlive the calls, and each handle is checked before it is used.
        unsafe {
            let map = CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                None,
                PAGE_READWRITE,
                0,
                size as u32,
                PCWSTR(map_name.as_ptr()),
            )
            .map_err(failed)?;
            let view = MapViewOfFile(map, FILE_MAP_ALL_ACCESS, 0, 0, size);
            let signal = CreateEventW(None, false, false, PCWSTR(event_name.as_ptr()));
            if view.Value.is_null() || signal.is_err() {
                let _ = CloseHandle(map);
                return Err(HostError("no shared memory".to_string()));
            }
            Ok(Self {
                map,
                view,
                signal: signal.map_err(failed)?,
                block,
                offline: AtomicBool::new(false),
            })
        }
    }

    fn open(name: &str, block: usize) -> Result<Self, HostError> {
        let failed = |e: windows::core::Error| HostError(format!("no shared memory: {e}"));
        let (map_name, event_name) = (wide(&format!("{name}-m")), wide(&format!("{name}-e")));
        // SAFETY: as in `create`.
        unsafe {
            let map = OpenFileMappingW(FILE_MAP_ALL_ACCESS.0, false, PCWSTR(map_name.as_ptr()))
                .map_err(failed)?;
            let view = MapViewOfFile(map, FILE_MAP_ALL_ACCESS, 0, 0, Self::size(block));
            let signal = OpenEventW(EVENT_ALL_ACCESS, false, PCWSTR(event_name.as_ptr()));
            if view.Value.is_null() || signal.is_err() {
                let _ = CloseHandle(map);
                return Err(HostError("no shared memory".to_string()));
            }
            Ok(Self {
                map,
                view,
                signal: signal.map_err(failed)?,
                block,
                offline: AtomicBool::new(false),
            })
        }
    }

    fn header(&self) -> *mut Header {
        self.view.Value.cast()
    }

    /// The atomics at the head of the memory.
    fn go(&self) -> &AtomicU64 {
        // SAFETY: the view is mapped and large enough, and these are only used atomically.
        unsafe { &*std::ptr::addr_of!((*self.header()).go) }
    }
    fn done(&self) -> &AtomicU64 {
        // SAFETY: as above.
        unsafe { &*std::ptr::addr_of!((*self.header()).done) }
    }
    fn quit(&self) -> &AtomicU32 {
        // SAFETY: as above.
        unsafe { &*std::ptr::addr_of!((*self.header()).quit) }
    }

    fn buffer(&self, which: usize) -> *mut f32 {
        // SAFETY: the buffers lie after the header, inside the mapped size.
        unsafe {
            self.view
                .Value
                .cast::<u8>()
                .add(std::mem::size_of::<Header>())
                .cast::<f32>()
                .add(which * self.block)
        }
    }

    fn signal(&self) {
        // SAFETY: the handle is valid as long as this lives. Setting an event does not wait.
        unsafe {
            let _ = SetEvent(self.signal);
        }
    }
}

impl Drop for Shm {
    fn drop(&mut self) {
        // SAFETY: nothing uses these once the last owner is gone.
        unsafe {
            let _ = UnmapViewOfFile(self.view);
            let _ = CloseHandle(self.map);
            let _ = CloseHandle(self.signal);
        }
    }
}

// ------------------------------------------------------------------ the audio half, in the program

struct BridgeProcessor {
    shm: Arc<Shm>,
    dead: Arc<AtomicBool>,
    released: Arc<AtomicBool>,
    sent: u64,
    latency: usize,
    params: Box<[(u32, f64)]>,
    param_count: usize,
    side: Box<[f32]>,
    side_on: bool,
}

impl BridgeProcessor {
    /// Spins until the helper answers block `self.sent`. Takes no lock and makes no system wait
    /// while playing. Offline it gives up its turn, since nothing is waiting on it.
    fn answered(&self) -> bool {
        let offline = self.shm.offline.load(Ordering::Relaxed);
        let limit = if offline { OFFLINE_WAIT } else { REALTIME_WAIT };
        let began = Instant::now();
        let mut turns = 0u32;
        loop {
            if self.shm.done().load(Ordering::Acquire) == self.sent {
                return true;
            }
            turns = turns.wrapping_add(1);
            if turns % 256 == 0 {
                if self.dead.load(Ordering::Relaxed) || began.elapsed() >= limit {
                    return false;
                }
                if offline {
                    std::thread::yield_now();
                }
            }
            std::hint::spin_loop();
        }
    }
}

impl PluginProcessor for BridgeProcessor {
    fn process(
        &mut self,
        input: [&[f32]; 2],
        output: [&mut [f32]; 2],
        events: &[PluginEvent],
        transport: &PluginTransport,
    ) {
        let [out_l, out_r] = output;
        // Gone: the sound passes by, and an effect's track goes on playing.
        if self.dead.load(Ordering::Relaxed) {
            out_l.copy_from_slice(input[0]);
            out_r.copy_from_slice(input[1]);
            return;
        }
        // Still on an earlier block: nothing may touch the memory yet.
        if self.shm.done().load(Ordering::Acquire) != self.sent {
            out_l.fill(0.0);
            out_r.fill(0.0);
            return;
        }
        let (shm, block) = (&self.shm, self.shm.block);
        let frames = block.min(input[0].len()).min(out_l.len());
        let header = shm.header();
        // SAFETY: `done` equals `sent`, so the helper is waiting and these parts are ours until
        // `go` moves. The buffers hold a block each, and the lists are as long as the counts say.
        unsafe {
            std::ptr::copy_nonoverlapping(input[0].as_ptr(), shm.buffer(IN_L), frames);
            std::ptr::copy_nonoverlapping(input[1].as_ptr(), shm.buffer(IN_R), frames);
            if self.side_on {
                let side = self.side.as_ptr();
                std::ptr::copy_nonoverlapping(side, shm.buffer(SIDE_L), block);
                std::ptr::copy_nonoverlapping(side.add(block), shm.buffer(SIDE_R), block);
            }
            let count = events.len().min(MAX_PLUGIN_EVENTS);
            let list = std::ptr::addr_of_mut!((*header).event_list).cast::<PluginEvent>();
            std::ptr::copy_nonoverlapping(events.as_ptr(), list, count);
            let moves = std::ptr::addr_of_mut!((*header).param_list).cast::<(u32, f64)>();
            std::ptr::copy_nonoverlapping(self.params.as_ptr(), moves, self.param_count);
            std::ptr::addr_of_mut!((*header).events).write(count as u32);
            std::ptr::addr_of_mut!((*header).params).write(self.param_count as u32);
            std::ptr::addr_of_mut!((*header).has_side).write(u32::from(self.side_on));
            std::ptr::addr_of_mut!((*header).transport).write(MaybeUninit::new(*transport));
        }
        self.param_count = 0;
        self.side_on = false;
        self.sent += 1;
        shm.go().store(self.sent, Ordering::Release);
        shm.signal();
        if self.answered() {
            // SAFETY: the helper has answered this block and waits for the next.
            unsafe {
                std::ptr::copy_nonoverlapping(shm.buffer(OUT_L), out_l.as_mut_ptr(), frames);
                std::ptr::copy_nonoverlapping(shm.buffer(OUT_R), out_r.as_mut_ptr(), frames);
            }
        } else if self.dead.load(Ordering::Relaxed) {
            out_l.copy_from_slice(input[0]);
            out_r.copy_from_slice(input[1]);
        } else {
            out_l.fill(0.0);
            out_r.fill(0.0);
        }
    }

    fn latency_samples(&self) -> usize {
        self.latency
    }

    fn set_param(&mut self, id: u32, value: f64) {
        if self.param_count < self.params.len() {
            self.params[self.param_count] = (id, value);
            self.param_count += 1;
        }
    }

    fn set_side_input(&mut self, side: [&[f32]; 2]) {
        let block = self.shm.block;
        if side[0].len() >= block && side[1].len() >= block {
            self.side[..block].copy_from_slice(&side[0][..block]);
            self.side[block..].copy_from_slice(&side[1][..block]);
            self.side_on = true;
        }
    }

    fn set_offline(&mut self, offline: bool) {
        self.shm.offline.store(offline, Ordering::Relaxed);
    }
}

impl Drop for BridgeProcessor {
    fn drop(&mut self) {
        self.released.store(true, Ordering::Release);
    }
}

// ------------------------------------------------------------------ the helper, from the program

/// What a plugin's helper said about it, kept until the rack asks.
#[derive(Default)]
struct Notes {
    edits: Vec<(u32, f64)>,
    editing: bool,
    ended: bool,
    dirty: bool,
    click: Option<HeaderClick>,
    closed: Option<(i32, i32)>,
    open: bool,
    latency: usize,
    /// Something that went wrong with no request waiting to hear it.
    failure: Option<String>,
}

struct Helper {
    child: Mutex<Child>,
    pid: u32,
    stdin: Mutex<ChildStdin>,
    replies: Mutex<Receiver<Vec<u8>>>,
    dead: Arc<AtomicBool>,
    notes: Arc<Mutex<HashMap<u32, Arc<Mutex<Notes>>>>>,
    watched: Mutex<Vec<Weak<Shm>>>,
    next_id: AtomicU32,
}

impl Helper {
    fn spawn() -> Result<Arc<Self>, HostError> {
        let exe = match HOST_EXE.lock().ok().and_then(|slot| slot.clone()) {
            Some(exe) => exe,
            None => std::env::current_exe().map_err(|e| HostError(e.to_string()))?,
        };
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut child = Command::new(exe)
            .arg(HOST_ARG)
            .arg(prowave_engine::plugin_block().to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| HostError(format!("could not start the plugin's process: {e}")))?;
        let missing = || HostError("the plugin's process has no pipes".to_string());
        let stdin = child.stdin.take().ok_or_else(missing)?;
        let mut stdout = child.stdout.take().ok_or_else(missing)?;
        let (tx, rx) = channel();
        let pid = child.id();
        let helper = Arc::new(Self {
            child: Mutex::new(child),
            pid,
            stdin: Mutex::new(stdin),
            replies: Mutex::new(rx),
            dead: Arc::new(AtomicBool::new(false)),
            notes: Arc::default(),
            watched: Mutex::default(),
            next_id: AtomicU32::new(1),
        });
        let (dead, notes) = (helper.dead.clone(), helper.notes.clone());
        std::thread::spawn(move || read_helper(&mut stdout, &tx, &notes, &dead));
        let watch = Arc::downgrade(&helper);
        std::thread::spawn(move || watch_helper(&watch));
        Ok(helper)
    }

    fn is_dead(&self) -> bool {
        self.dead.load(Ordering::Relaxed)
    }

    /// A message with no answer. Lost if the helper is gone, which is seen elsewhere.
    fn tell(&self, message: &Out) {
        if let Ok(mut stdin) = self.stdin.lock() {
            let _ = write_frame(&mut *stdin, &message.0);
        }
    }

    /// Sends what `build` makes from the request's id, and waits up to `wait` for the answer.
    fn ask(&self, wait: Duration, build: impl FnOnce(u32) -> Out) -> Result<Vec<u8>, HostError> {
        let gone = || HostError("the plugin's process has stopped".to_string());
        if self.is_dead() {
            return Err(gone());
        }
        // One request at a time: this lock is what keeps answers in order.
        let replies = self.replies.lock().map_err(|_| gone())?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.tell(&build(id));
        let began = Instant::now();
        loop {
            // A plugin's window is tied to the program's. Messages the helper sends this thread
            // must be answered, or it would be waiting on us while we wait on it.
            answer_sent_messages();
            match replies.recv_timeout(Duration::from_millis(1)) {
                Ok(reply) => {
                    let mut r = In(&reply);
                    // An answer that came after its request gave up is not this one's.
                    if r.u32() != Some(id) {
                        continue;
                    }
                    return if r.bool() == Some(true) {
                        Ok(r.0.to_vec())
                    } else {
                        Err(HostError(r.text().unwrap_or_else(|| "failed".to_string())))
                    };
                }
                Err(_) if self.is_dead() => return Err(gone()),
                Err(_) if began.elapsed() >= wait => {
                    return Err(HostError("the plugin did not answer in time".to_string()));
                }
                Err(_) => {}
            }
        }
    }
}

/// Runs the messages other threads and processes have sent this thread and are waiting on. Posted
/// ones stay in the queue, so the main loop is not re-entered with its usual work.
fn answer_sent_messages() {
    use windows::Win32::UI::WindowsAndMessaging::{
        PeekMessageW, MSG, PM_NOREMOVE, PM_QS_SENDMESSAGE,
    };
    let mut msg = MSG::default();
    // SAFETY: `msg` is a valid local. Peeking dispatches sent messages to their own windows.
    let _ = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE | PM_QS_SENDMESSAGE) };
}

fn read_helper(
    stdout: &mut impl Read,
    replies: &Sender<Vec<u8>>,
    notes: &Mutex<HashMap<u32, Arc<Mutex<Notes>>>>,
    dead: &AtomicBool,
) {
    while let Some(frame) = read_frame(stdout) {
        let Some((&tag, rest)) = frame.split_first() else {
            continue;
        };
        match tag {
            REPLY => {
                let _ = replies.send(rest.to_vec());
            }
            NOTES => {
                let _ = take_notes(rest, notes);
            }
            FAILED => {
                let _ = take_failure(rest, notes);
            }
            _ => {}
        }
    }
    dead.store(true, Ordering::Relaxed);
}

fn take_failure(frame: &[u8], notes: &Mutex<HashMap<u32, Arc<Mutex<Notes>>>>) -> Option<()> {
    let mut r = In(frame);
    let instance = r.u32()?;
    let why = r.text()?;
    let target = notes.lock().ok()?.get(&instance)?.clone();
    let mut n = target.lock().ok()?;
    n.failure = Some(why);
    n.open = false;
    Some(())
}

fn take_notes(frame: &[u8], notes: &Mutex<HashMap<u32, Arc<Mutex<Notes>>>>) -> Option<()> {
    let mut r = In(frame);
    let instance = r.u32()?;
    let target = notes.lock().ok()?.get(&instance)?.clone();
    let mut n = target.lock().ok()?;
    for _ in 0..r.u32()? {
        let (id, value) = (r.u32()?, r.f64()?);
        n.edits.push((id, value));
    }
    n.editing = r.bool()?;
    n.ended |= r.bool()?;
    n.dirty |= r.bool()?;
    if let Some(click) = click_of(r.u8()?) {
        n.click = Some(click);
    }
    if r.bool()? {
        n.closed = Some((r.i32()?, r.i32()?));
    }
    n.open = r.bool()?;
    n.latency = r.u32()? as usize;
    Some(())
}

/// Sees the helper end, and ends one whose plugin has hung while playing.
fn watch_helper(helper: &Weak<Helper>) {
    let mut stuck: HashMap<usize, (u64, Instant)> = HashMap::new();
    loop {
        std::thread::sleep(Duration::from_millis(100));
        let Some(helper) = helper.upgrade() else {
            return;
        };
        let Ok(mut child) = helper.child.lock() else {
            return;
        };
        if !matches!(child.try_wait(), Ok(None)) {
            helper.dead.store(true, Ordering::Relaxed);
            return;
        }
        let Ok(mut watched) = helper.watched.lock() else {
            return;
        };
        watched.retain(|shm| shm.strong_count() > 0);
        for shm in watched.iter().filter_map(Weak::upgrade) {
            let key = Arc::as_ptr(&shm) as usize;
            let (go, done) = (
                shm.go().load(Ordering::Acquire),
                shm.done().load(Ordering::Acquire),
            );
            if go == done || shm.offline.load(Ordering::Relaxed) {
                stuck.remove(&key);
                continue;
            }
            let since = stuck.entry(key).or_insert((done, Instant::now()));
            if since.0 != done {
                *since = (done, Instant::now());
            } else if since.1.elapsed() >= HUNG_AFTER {
                tracing::warn!("a plugin stopped responding: ending its process");
                let _ = child.kill();
            }
        }
    }
}

fn helpers() -> &'static Mutex<HashMap<PathBuf, Arc<Helper>>> {
    static HELPERS: OnceLock<Mutex<HashMap<PathBuf, Arc<Helper>>>> = OnceLock::new();
    HELPERS.get_or_init(Mutex::default)
}

/// The helper for a plugin file, started if there is none or the last one ended.
fn helper_for(path: &Path) -> Result<Arc<Helper>, HostError> {
    let mut all = helpers()
        .lock()
        .map_err(|_| HostError("the plugin processes are unusable".to_string()))?;
    if let Some(helper) = all.get(path).filter(|h| !h.is_dead()) {
        return Ok(helper.clone());
    }
    let helper = Helper::spawn()?;
    all.insert(path.to_path_buf(), helper.clone());
    Ok(helper)
}

/// The UI-thread half of a plugin in a helper.
struct BridgeLoaded {
    helper: Arc<Helper>,
    instance: u32,
    notes: Arc<Mutex<Notes>>,
    released: Arc<AtomicBool>,
    params: Vec<PluginParam>,
    has_side: bool,
    reload_side: (bool, bool),
    shown: HashMap<u32, f64>,
    header: Option<HeaderState>,
}

impl BridgeLoaded {
    fn with_notes<T: Default>(&self, read: impl FnOnce(&mut Notes) -> T) -> T {
        self.notes
            .lock()
            .map(|mut n| read(&mut n))
            .unwrap_or_default()
    }
}

impl LoadedPlugin for BridgeLoaded {
    fn params(&self) -> Vec<PluginParam> {
        self.params.clone()
    }

    fn take_edits(&mut self) -> Vec<(u32, f64)> {
        self.with_notes(|n| std::mem::take(&mut n.edits))
    }

    fn editing(&self) -> bool {
        self.with_notes(|n| n.editing)
    }

    fn take_edit_ended(&mut self) -> bool {
        self.with_notes(|n| std::mem::take(&mut n.ended))
    }

    fn param_value(&mut self, id: u32) -> Option<f64> {
        let instance = self.instance;
        let reply = self
            .helper
            .ask(SHORT_WAIT, |rid| {
                Out::new(PARAM_VALUE).u32(rid).u32(instance).u32(id)
            })
            .ok()?;
        let mut r = In(&reply);
        if r.bool()? {
            r.f64()
        } else {
            None
        }
    }

    fn show_param(&mut self, id: u32, value: f64) {
        if self.shown.insert(id, value) != Some(value) {
            self.helper
                .tell(&Out::new(SHOW_PARAM).u32(self.instance).u32(id).f64(value));
        }
    }

    fn has_side_input(&self) -> bool {
        self.has_side
    }

    fn side_needs_reload(&self, on: bool) -> bool {
        if on {
            self.reload_side.0
        } else {
            self.reload_side.1
        }
    }

    fn show_header(&mut self, state: &HeaderState) {
        if self.header.as_ref() != Some(state) {
            let preset = state.preset.clone();
            self.helper.tell(
                &Out::new(SHOW_HEADER)
                    .u32(self.instance)
                    .bool(state.bypass)
                    .bool(state.on_b)
                    .bool(preset.is_some())
                    .text(preset.as_deref().unwrap_or_default()),
            );
            self.header = Some(state.clone());
        }
    }

    fn take_header_click(&mut self) -> Option<HeaderClick> {
        self.with_notes(|n| n.click.take())
    }

    fn header_menu(&mut self, items: &[String]) -> Option<usize> {
        let instance = self.instance;
        let reply = self
            .helper
            .ask(MENU_WAIT, |rid| {
                let mut out = Out::new(HEADER_MENU)
                    .u32(rid)
                    .u32(instance)
                    .u32(items.len() as u32);
                for item in items {
                    out = out.text(item);
                }
                out
            })
            .ok()?;
        let mut r = In(&reply);
        if r.bool()? {
            r.u32().map(|i| i as usize)
        } else {
            None
        }
    }

    fn latency_samples(&self) -> usize {
        self.with_notes(|n| n.latency)
    }

    fn released(&self) -> bool {
        self.released.load(Ordering::Acquire)
    }

    fn tick(&mut self) {}

    fn editor_is_open(&self) -> bool {
        self.with_notes(|n| n.open)
    }

    fn take_closed_position(&mut self) -> Option<(i32, i32)> {
        self.with_notes(|n| n.closed.take())
    }

    /// Tells the helper and does not wait: a window can take seconds, and this is the UI thread.
    /// If it fails, that comes back through `take_failure`.
    fn open_editor(
        &mut self,
        title: &str,
        owner: Option<isize>,
        position: Option<(i32, i32)>,
    ) -> Result<(), HostError> {
        if self.helper.is_dead() {
            return Err(HostError("the plugin's process has stopped".to_string()));
        }
        let (x, y) = position.unwrap_or((0, 0));
        self.helper.tell(
            &Out::new(OPEN_EDITOR)
                .u32(self.instance)
                .text(title)
                .bool(owner.is_some())
                .f64(owner.unwrap_or(0) as f64)
                .bool(position.is_some())
                .i32(x)
                .i32(y),
        );
        self.with_notes(|n| n.open = true);
        Ok(())
    }

    fn close_editor(&mut self) {
        self.helper.tell(&Out::new(CLOSE_EDITOR).u32(self.instance));
    }

    fn save_state(&mut self) -> Result<Option<Vec<u8>>, HostError> {
        let instance = self.instance;
        let reply = self.helper.ask(SHORT_WAIT, |rid| {
            Out::new(SAVE_STATE).u32(rid).u32(instance)
        })?;
        let mut r = In(&reply);
        match r.bool() {
            Some(true) => Ok(r.bytes()),
            _ => Ok(None),
        }
    }

    fn load_state(&mut self, bytes: &[u8]) -> Result<(), HostError> {
        let instance = self.instance;
        self.helper
            .ask(LOAD_WAIT, |rid| {
                Out::new(LOAD_STATE).u32(rid).u32(instance).bytes(bytes)
            })
            .map(|_| ())
    }

    fn take_dirty(&mut self) -> bool {
        self.with_notes(|n| std::mem::take(&mut n.dirty))
    }

    fn crashed(&self) -> bool {
        self.helper.is_dead()
    }

    fn process_id(&self) -> Option<u32> {
        Some(self.helper.pid)
    }

    fn take_failure(&mut self) -> Option<String> {
        self.with_notes(|n| n.failure.take())
    }
}

impl Drop for BridgeLoaded {
    fn drop(&mut self) {
        self.helper.tell(&Out::new(DESTROY).u32(self.instance));
        if let Ok(mut notes) = self.helper.notes.lock() {
            notes.remove(&self.instance);
        }
    }
}

/// Loads plugin `id` of a file in its file's helper process.
pub fn load_isolated(
    path: &Path,
    id: &str,
    sample_rate: u32,
    side: bool,
) -> Result<Loaded, HostError> {
    let helper = helper_for(path)?;
    let block = prowave_engine::plugin_block();
    let name = format!(
        "ProWaveBridge-{}-{}",
        std::process::id(),
        NEXT_NAME.fetch_add(1, Ordering::Relaxed)
    );
    let shm = Arc::new(Shm::create(&name, block)?);
    let instance = helper.next_id.fetch_add(1, Ordering::Relaxed);
    let notes = Arc::new(Mutex::new(Notes::default()));
    helper
        .notes
        .lock()
        .map_err(|_| HostError("the plugin's process is unusable".to_string()))?
        .insert(instance, notes.clone());
    let reply = helper.ask(LOAD_WAIT, |rid| {
        Out::new(LOAD)
            .u32(rid)
            .u32(instance)
            .text(&path.to_string_lossy())
            .text(id)
            .u32(sample_rate)
            .bool(side)
            .text(&name)
    });
    let reply = match reply {
        Ok(reply) => reply,
        Err(e) => {
            if let Ok(mut all) = helper.notes.lock() {
                all.remove(&instance);
            }
            if helper.is_dead() {
                return Err(HostError(format!("{id}: crashed while loading")));
            }
            return Err(e);
        }
    };
    let malformed = || HostError("the plugin's process gave a wrong answer".to_string());
    let mut r = In(&reply);
    let latency = r.u32().ok_or_else(malformed)? as usize;
    let has_side = r.bool().ok_or_else(malformed)?;
    let reload_side = (
        r.bool().ok_or_else(malformed)?,
        r.bool().ok_or_else(malformed)?,
    );
    let mut params = Vec::new();
    for _ in 0..r.u32().ok_or_else(malformed)? {
        params.push(PluginParam {
            id: r.u32().ok_or_else(malformed)?,
            name: r.text().ok_or_else(malformed)?,
        });
    }
    if let Ok(mut n) = notes.lock() {
        n.latency = latency;
    }
    if let Ok(mut watched) = helper.watched.lock() {
        watched.push(Arc::downgrade(&shm));
    }
    let released = Arc::new(AtomicBool::new(false));
    let processor = BridgeProcessor {
        shm,
        dead: helper.dead.clone(),
        released: released.clone(),
        sent: 0,
        latency,
        params: vec![(0, 0.0); MAX_PARAMS].into_boxed_slice(),
        param_count: 0,
        side: vec![0.0; block * 2].into_boxed_slice(),
        side_on: false,
    };
    let loaded = BridgeLoaded {
        helper,
        instance,
        notes,
        released,
        params,
        has_side,
        reload_side,
        shown: HashMap::new(),
        header: None,
    };
    Ok((Box::new(loaded), Box::new(processor)))
}

/// Loads a plugin the way the settings say: in a helper when that is on.
pub fn load_plugin_safely(
    path: &Path,
    id: &str,
    sample_rate: u32,
    side: bool,
) -> Result<Loaded, HostError> {
    if isolation_on() && PluginFormat::of(path) != PluginFormat::Sampler {
        load_isolated(path, id, sample_rate, side)
    } else {
        load_plugin(path, id, sample_rate, side)
    }
}

// ------------------------------------------------------------------ the helper's own side

/// What runs a plugin's blocks in the helper.
fn audio_worker(shm: &Shm, mut processor: Box<dyn PluginProcessor>) {
    prowave_engine::Engine::prepare_worker_thread();
    let block = shm.block;
    let header = shm.header();
    let mut last = 0u64;
    loop {
        // A short spin catches the next block while it is close, then the thread sleeps.
        let mut turns = 0u32;
        loop {
            if shm.quit().load(Ordering::Relaxed) != 0 {
                return;
            }
            if shm.go().load(Ordering::Acquire) != last {
                break;
            }
            turns += 1;
            if turns < 2000 {
                std::hint::spin_loop();
            } else {
                // SAFETY: the handle lives as long as `shm`.
                unsafe {
                    let _ = WaitForSingleObject(shm.signal, 100);
                }
            }
        }
        last = shm.go().load(Ordering::Acquire);
        // SAFETY: `go` moved, so the program has finished writing and waits for `done`. All of this
        // is ours until then, and it holds what the counts say.
        unsafe {
            let moves = std::ptr::addr_of!((*header).params).read() as usize;
            let list = std::ptr::addr_of!((*header).param_list).cast::<(u32, f64)>();
            for &(id, value) in std::slice::from_raw_parts(list, moves.min(MAX_PARAMS)) {
                processor.set_param(id, value);
            }
            if std::ptr::addr_of!((*header).has_side).read() != 0 {
                processor.set_side_input([
                    std::slice::from_raw_parts(shm.buffer(SIDE_L), block),
                    std::slice::from_raw_parts(shm.buffer(SIDE_R), block),
                ]);
            }
            let count = std::ptr::addr_of!((*header).events).read() as usize;
            let events = std::ptr::addr_of!((*header).event_list).cast::<PluginEvent>();
            let events = std::slice::from_raw_parts(events, count.min(MAX_PLUGIN_EVENTS));
            let transport = std::ptr::addr_of!((*header).transport).read().assume_init();
            processor.process(
                [
                    std::slice::from_raw_parts(shm.buffer(IN_L), block),
                    std::slice::from_raw_parts(shm.buffer(IN_R), block),
                ],
                [
                    std::slice::from_raw_parts_mut(shm.buffer(OUT_L), block),
                    std::slice::from_raw_parts_mut(shm.buffer(OUT_R), block),
                ],
                events,
                &transport,
            );
        }
        shm.done().store(last, Ordering::Release);
    }
}

struct HostedInstance {
    loaded: Box<dyn LoadedPlugin>,
    worker: Option<std::thread::JoinHandle<()>>,
    shm: Arc<Shm>,
    was_open: bool,
    was_editing: bool,
    latency: usize,
}

impl HostedInstance {
    /// Ends its audio thread, and with it the audio half, before the rest goes.
    fn end_audio(&mut self) {
        self.shm.quit().store(1, Ordering::Relaxed);
        self.shm.signal();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn reply(out: &mut impl Write, id: u32, result: Result<Out, String>) {
    let message = match result {
        Ok(body) => {
            let mut message = Out::new(REPLY).u32(id).bool(true);
            message.0.extend_from_slice(&body.0);
            message
        }
        Err(why) => Out::new(REPLY).u32(id).bool(false).text(&why),
    };
    let _ = write_frame(out, &message.0);
}

fn host_load(
    frame: &mut In<'_>,
    sample_rate_block: usize,
) -> Option<Result<(u32, HostedInstance, Out), String>> {
    let (instance, path, id) = (frame.u32()?, frame.text()?, frame.text()?);
    let (sample_rate, side, name) = (frame.u32()?, frame.bool()?, frame.text()?);
    let result = (|| {
        let shm = Arc::new(Shm::open(&name, sample_rate_block).map_err(|e| e.0)?);
        let (loaded, processor) =
            load_plugin(Path::new(&path), &id, sample_rate, side).map_err(|e| e.0)?;
        let params = loaded.params();
        let latency = loaded.latency_samples();
        let mut body = Out::default()
            .u32(latency as u32)
            .bool(loaded.has_side_input())
            .bool(loaded.side_needs_reload(true))
            .bool(loaded.side_needs_reload(false))
            .u32(params.len() as u32);
        for param in &params {
            body = body.u32(param.id).text(&param.name);
        }
        let for_worker = shm.clone();
        let worker = std::thread::spawn(move || audio_worker(&for_worker, processor));
        Ok((
            instance,
            HostedInstance {
                loaded,
                worker: Some(worker),
                shm,
                was_open: false,
                was_editing: false,
                latency,
            },
            body,
        ))
    })();
    Some(result)
}

/// Runs one message from the program. `false` when it could not be read.
fn host_command(
    frame: &[u8],
    instances: &mut HashMap<u32, HostedInstance>,
    block: usize,
    out: &mut impl Write,
) -> Option<()> {
    let mut r = In(frame);
    let tag = r.u8()?;
    match tag {
        LOAD => {
            let id = r.u32()?;
            match host_load(&mut r, block)? {
                Ok((instance, hosted, body)) => {
                    instances.insert(instance, hosted);
                    reply(out, id, Ok(body));
                }
                Err(why) => reply(out, id, Err(why)),
            }
        }
        SAVE_STATE | LOAD_STATE | PARAM_VALUE | HEADER_MENU => {
            let (id, instance) = (r.u32()?, r.u32()?);
            let Some(hosted) = instances.get_mut(&instance) else {
                reply(out, id, Err("that plugin is not here".to_string()));
                return Some(());
            };
            let result = match tag {
                SAVE_STATE => hosted.loaded.save_state().map_err(|e| e.0).map(|state| {
                    let has = state.is_some();
                    Out::default().bool(has).bytes(&state.unwrap_or_default())
                }),
                LOAD_STATE => {
                    let bytes = r.bytes()?;
                    hosted
                        .loaded
                        .load_state(&bytes)
                        .map(|()| Out::default())
                        .map_err(|e| e.0)
                }
                PARAM_VALUE => {
                    let value = hosted.loaded.param_value(r.u32()?);
                    Ok(Out::default()
                        .bool(value.is_some())
                        .f64(value.unwrap_or(0.0)))
                }
                _ => {
                    let mut items = Vec::new();
                    for _ in 0..r.u32()? {
                        items.push(r.text()?);
                    }
                    let chosen = hosted.loaded.header_menu(&items);
                    Ok(Out::default()
                        .bool(chosen.is_some())
                        .u32(chosen.unwrap_or(0) as u32))
                }
            };
            reply(out, id, result);
        }
        OPEN_EDITOR => {
            let instance = r.u32()?;
            let title = r.text()?;
            let owner = (r.bool()?, r.f64()?);
            let position = (r.bool()?, r.i32()?, r.i32()?);
            // Windows want this. Loading must not have it: a plugin that hands work to a thread set
            // up this way, while that thread is busy loading it, waits for ever.
            static OLE: std::sync::Once = std::sync::Once::new();
            // SAFETY: on this process's main thread, once.
            OLE.call_once(|| unsafe {
                let _ = windows::Win32::System::Ole::OleInitialize(None);
            });
            if let Some(hosted) = instances.get_mut(&instance) {
                let shown = hosted.loaded.open_editor(
                    &title,
                    owner.0.then_some(owner.1 as isize),
                    position.0.then_some((position.1, position.2)),
                );
                if let Err(e) = shown {
                    let _ = write_frame(out, &Out::new(FAILED).u32(instance).text(&e.0).0);
                }
            }
        }
        DESTROY => {
            if let Some(mut hosted) = instances.remove(&r.u32()?) {
                hosted.loaded.close_editor();
                hosted.end_audio();
            }
        }
        CLOSE_EDITOR => {
            if let Some(hosted) = instances.get_mut(&r.u32()?) {
                hosted.loaded.close_editor();
            }
        }
        SHOW_PARAM => {
            let instance = r.u32()?;
            let (id, value) = (r.u32()?, r.f64()?);
            if let Some(hosted) = instances.get_mut(&instance) {
                hosted.loaded.show_param(id, value);
            }
        }
        SHOW_HEADER => {
            let instance = r.u32()?;
            let (bypass, on_b, has, name) = (r.bool()?, r.bool()?, r.bool()?, r.text()?);
            if let Some(hosted) = instances.get_mut(&instance) {
                hosted.loaded.show_header(&HeaderState {
                    bypass,
                    on_b,
                    preset: has.then_some(name),
                });
            }
        }
        _ => {}
    }
    Some(())
}

/// Tells the program what changed in a plugin since last time, if anything did.
fn send_notes(instance: u32, hosted: &mut HostedInstance, out: &mut impl Write) {
    let loaded = &mut hosted.loaded;
    loaded.tick();
    let edits = loaded.take_edits();
    let editing = loaded.editing();
    let ended = loaded.take_edit_ended();
    let dirty = loaded.take_dirty();
    let click = loaded.take_header_click();
    let closed = loaded.take_closed_position();
    let open = loaded.editor_is_open();
    let latency = loaded.latency_samples();
    let quiet = edits.is_empty()
        && !ended
        && !dirty
        && click.is_none()
        && closed.is_none()
        && editing == hosted.was_editing
        && open == hosted.was_open
        && latency == hosted.latency;
    if quiet {
        return;
    }
    hosted.was_editing = editing;
    hosted.was_open = open;
    hosted.latency = latency;
    let mut message = Out::new(NOTES).u32(instance).u32(edits.len() as u32);
    for (id, value) in edits {
        message = message.u32(id).f64(value);
    }
    message = message
        .bool(editing)
        .bool(ended)
        .bool(dirty)
        .u8(click_code(click))
        .bool(closed.is_some());
    if let Some((x, y)) = closed {
        message = message.i32(x).i32(y);
    }
    message = message.bool(open).u32(latency as u32);
    let _ = write_frame(out, &message.0);
}

/// The helper's whole life: runs until the program closes its end of the pipe, which it also does
/// by ending or crashing. Returns the exit code.
pub fn run_plugin_host(block: usize) -> i32 {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::System::Console::{
        GetStdHandle, SetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, MsgWaitForMultipleObjects, PeekMessageW, PostThreadMessageW,
        TranslateMessage, MSG, PM_REMOVE, QS_ALLINPUT, WM_NULL,
    };
    prowave_engine::set_plugin_block(block);
    // Plugins print things. The pipes are kept for this program, and they get nothing, before any
    // plugin loads and looks at where its output goes.
    let Ok(nul) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("NUL")
    else {
        return 1;
    };
    // SAFETY: the handles are this process's own, each taken once and owned by the file made from
    // it. `nul` stays open for as long as the process, as the handles it stands in for.
    let (mut from_program, mut to_program) = unsafe {
        let (Ok(input), Ok(output)) = (
            GetStdHandle(STD_INPUT_HANDLE),
            GetStdHandle(STD_OUTPUT_HANDLE),
        ) else {
            return 1;
        };
        let stand_in = HANDLE(nul.as_raw_handle());
        let _ = SetStdHandle(STD_INPUT_HANDLE, stand_in);
        let _ = SetStdHandle(STD_OUTPUT_HANDLE, stand_in);
        let _ = SetStdHandle(STD_ERROR_HANDLE, stand_in);
        (
            std::fs::File::from_raw_handle(input.0 as _),
            std::fs::File::from_raw_handle(output.0 as _),
        )
    };
    std::mem::forget(nul);
    let (tx, rx) = channel::<Vec<u8>>();
    // SAFETY: asks for this thread's own number, with nothing to go wrong.
    let main_thread = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
    std::thread::spawn(move || {
        while let Some(frame) = read_frame(&mut from_program) {
            if tx.send(frame).is_err() {
                return;
            }
            // Wakes the main thread now rather than at its next look.
            // SAFETY: an empty message to a thread of this process. If it has gone, this fails.
            unsafe {
                let _ = PostThreadMessageW(main_thread, WM_NULL, WPARAM(0), LPARAM(0));
            }
        }
        // The program has gone: nothing is left to do. The channel closing says so.
    });
    let mut instances: HashMap<u32, HostedInstance> = HashMap::new();
    loop {
        // SAFETY: the usual message loop, on the thread that made the plugins' windows.
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        loop {
            match rx.try_recv() {
                Ok(frame) => {
                    let _ = host_command(&frame, &mut instances, block, &mut to_program);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    for hosted in instances.values_mut() {
                        hosted.end_audio();
                    }
                    return 0;
                }
            }
        }
        for (instance, hosted) in &mut instances {
            send_notes(*instance, hosted, &mut to_program);
        }
        // SAFETY: waits for a window message or the time, with no handles to go wrong.
        unsafe {
            let _ = MsgWaitForMultipleObjects(None, false, 8, QS_ALLINPUT);
        }
    }
}
