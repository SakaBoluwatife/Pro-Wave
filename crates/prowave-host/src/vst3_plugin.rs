//! VST3 plugins, through the raw `vst3` bindings. Unlike CLAP nothing here is checked for us: every
//! call goes through a COM pointer the plugin gave out.
//!
//! The order of calls follows Steinberg's published hosting documentation: initialize the component
//! and controller, connect them, choose the buses, set up processing, activate, then process. Edits
//! made in a plugin's editor arrive at the host on the UI thread and go to the audio thread through
//! a queue, because in VST3 the host carries them from the controller to the processor.
use crate::clap::{fill_main_input, read_main_output};
use crate::editor_window::EditorWindow;
use crate::{HostError as Error, Loaded, LoadedPlugin, PluginFormat, PluginInfo};
use prowave_engine::{
    PluginEvent, PluginEventKind, PluginProcessor, PluginTransport, MAX_PLUGIN_EVENTS, PLUGIN_BLOCK,
};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::{c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use vst3::Steinberg::IPlugViewContentScaleSupport_::ScaleFactor;
use vst3::Steinberg::Vst::IAttributeList_::AttrID;
use vst3::Steinberg::Vst::ProcessContext_::StatesAndFlags_ as Flags;
use vst3::Steinberg::Vst::*;
use vst3::Steinberg::*;
use vst3::{Class, ComPtr, ComWrapper, Interface};

const AUDIO_CLASS: &str = "Audio Module Class";
/// Different parameters a block can change, and how many changes each.
const MAX_PARAM_QUEUES: usize = 64;
const MAX_PARAM_POINTS: usize = 16;
/// Edits made in an editor waiting for the next block.
const EDIT_QUEUE: usize = 1024;
/// `kPitchBend` and the controllers below it.
const MIDI_CONTROLLERS: usize = 130;
const STATE_MAGIC: &[u8; 4] = b"PW3S";

fn err(path: &Path, what: &str) -> Error {
    Error(format!("{}: {what}", path.display()))
}

/// A string from a fixed array of C characters, which may fill it with no end mark.
fn text_of(chars: &[std::ffi::c_char]) -> String {
    let bytes: Vec<u8> = chars
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| *c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// A class identifier as text, for saving in a project.
fn uid_text(uid: &TUID) -> String {
    uid.iter().map(|b| format!("{:02X}", *b as u8)).collect()
}

fn uid_from_text(text: &str) -> Option<TUID> {
    if text.len() != 32 || !text.is_ascii() {
        return None;
    }
    let mut uid: TUID = [0; 16];
    for (i, part) in uid.iter_mut().enumerate() {
        *part = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()? as std::ffi::c_char;
    }
    Some(uid)
}

/// The library inside a `.vst3`: a bundle folder holds it in a folder for the architecture.
fn module_file(path: &Path) -> PathBuf {
    if !path.is_dir() {
        return path.to_path_buf();
    }
    let name = path.file_name().map(PathBuf::from).unwrap_or_default();
    path.join("Contents").join("x86_64-win").join(name)
}

/// Where VST3 plugins are installed.
pub fn vst3_folders() -> Vec<PathBuf> {
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let mut dirs = Vec::new();
    if let Some(d) = var("COMMONPROGRAMFILES") {
        dirs.push(d.join("VST3"));
    }
    if let Some(d) = var("LOCALAPPDATA") {
        dirs.push(d.join("Programs").join("Common").join("VST3"));
    }
    dirs
}

fn is_vst3(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("vst3"))
}

fn find_in(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for e in read.flatten() {
        let path = e.path();
        if is_vst3(&path) {
            // A file, or a bundle folder, which is not looked into.
            out.push(path);
        } else if depth > 0 && path.is_dir() {
            find_in(&path, depth - 1, out);
        }
    }
}

/// Every `.vst3` in `dirs` and the folders below them, sorted, each once. Nothing is loaded.
pub fn find_vst3_files(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for d in dirs {
        find_in(d, 3, &mut found);
    }
    found.sort();
    found.dedup();
    found
}

// ------------------------------------------------------------------------- what the host provides

/// What a plugin knows the host by, and where it gets messages to talk to itself.
struct HostApp;

impl Class for HostApp {
    type Interfaces = (IHostApplication,);
}

impl IHostApplicationTrait for HostApp {
    unsafe fn getName(&self, name: *mut String128) -> tresult {
        // SAFETY: the caller gives room for a String128; the name is shorter than that.
        unsafe {
            let out = &mut *name;
            for (slot, c) in out.iter_mut().zip("Pro Wave".encode_utf16().chain([0])) {
                *slot = c as TChar;
            }
        }
        kResultOk
    }

    unsafe fn createInstance(
        &self,
        cid: *mut TUID,
        iid: *mut TUID,
        obj: *mut *mut c_void,
    ) -> tresult {
        // SAFETY: the caller passes valid identifiers and somewhere to put the result.
        unsafe {
            *obj = std::ptr::null_mut();
            let wanted: [u8; 16] = std::mem::transmute(*cid);
            let made = if wanted == IMessage::IID {
                ComWrapper::new(Message::new()).to_com_ptr::<FUnknown>()
            } else if wanted == IAttributeList::IID {
                ComWrapper::new(Attributes::default()).to_com_ptr::<FUnknown>()
            } else {
                None
            };
            match made {
                Some(made) => {
                    let ptr = made.as_ptr();
                    ((*(*ptr).vtbl).queryInterface)(ptr, iid, obj)
                }
                None => kNoInterface,
            }
        }
    }
}

/// A message between a plugin's two halves.
struct Message {
    id: RefCell<CString>,
    attributes: ComPtr<IAttributeList>,
    _keep: ComWrapper<Attributes>,
}

impl Message {
    fn new() -> Self {
        let keep = ComWrapper::new(Attributes::default());
        let attributes = keep.to_com_ptr::<IAttributeList>();
        Self {
            id: RefCell::new(CString::default()),
            // SAFETY of the expect-free path: a class always gives out the interface it declares.
            attributes: attributes.unwrap_or_else(|| unreachable!("declared in Interfaces")),
            _keep: keep,
        }
    }
}

impl Class for Message {
    type Interfaces = (IMessage,);
}

impl IMessageTrait for Message {
    unsafe fn getMessageID(&self) -> FIDString {
        self.id.borrow().as_ptr()
    }

    unsafe fn setMessageID(&self, id: FIDString) {
        if !id.is_null() {
            // SAFETY: the caller passes an ended string.
            *self.id.borrow_mut() = unsafe { CStr::from_ptr(id) }.to_owned();
        }
    }

    unsafe fn getAttributes(&self) -> *mut IAttributeList {
        self.attributes.as_ptr()
    }
}

#[derive(Clone)]
enum Attribute {
    Int(i64),
    Float(f64),
    Text(Vec<TChar>),
    Binary(Vec<u8>),
}

/// The contents of a [`Message`].
#[derive(Default)]
struct Attributes {
    values: RefCell<HashMap<String, Attribute>>,
}

impl Attributes {
    /// SAFETY: `id` must be an ended string, as the interface promises.
    unsafe fn key(id: AttrID) -> Option<String> {
        (!id.is_null()).then(|| unsafe { CStr::from_ptr(id) }.to_string_lossy().into_owned())
    }

    /// SAFETY: as for [`Self::key`].
    unsafe fn set(&self, id: AttrID, value: Attribute) -> tresult {
        match unsafe { Self::key(id) } {
            Some(key) => {
                self.values.borrow_mut().insert(key, value);
                kResultOk
            }
            None => kInvalidArgument,
        }
    }

    /// SAFETY: as for [`Self::key`].
    unsafe fn get(&self, id: AttrID) -> Option<Attribute> {
        self.values
            .borrow()
            .get(&unsafe { Self::key(id) }?)
            .cloned()
    }
}

impl Class for Attributes {
    type Interfaces = (IAttributeList,);
}

impl IAttributeListTrait for Attributes {
    unsafe fn setInt(&self, id: AttrID, value: int64) -> tresult {
        unsafe { self.set(id, Attribute::Int(value)) }
    }

    unsafe fn getInt(&self, id: AttrID, value: *mut int64) -> tresult {
        match unsafe { self.get(id) } {
            Some(Attribute::Int(v)) => {
                unsafe { *value = v };
                kResultOk
            }
            _ => kResultFalse,
        }
    }

    unsafe fn setFloat(&self, id: AttrID, value: f64) -> tresult {
        unsafe { self.set(id, Attribute::Float(value)) }
    }

    unsafe fn getFloat(&self, id: AttrID, value: *mut f64) -> tresult {
        match unsafe { self.get(id) } {
            Some(Attribute::Float(v)) => {
                unsafe { *value = v };
                kResultOk
            }
            _ => kResultFalse,
        }
    }

    unsafe fn setString(&self, id: AttrID, string: *const TChar) -> tresult {
        if string.is_null() {
            return kInvalidArgument;
        }
        let mut text = Vec::new();
        // SAFETY: the caller passes an ended string.
        unsafe {
            let mut at = string;
            while *at != 0 {
                text.push(*at);
                at = at.add(1);
            }
            self.set(id, Attribute::Text(text))
        }
    }

    unsafe fn getString(&self, id: AttrID, string: *mut TChar, size_in_bytes: uint32) -> tresult {
        let room = size_in_bytes as usize / std::mem::size_of::<TChar>();
        match unsafe { self.get(id) } {
            Some(Attribute::Text(text)) if room > 0 => {
                let n = text.len().min(room - 1);
                // SAFETY: the caller gives room for `size_in_bytes`, and `n` stays inside it.
                unsafe {
                    std::ptr::copy_nonoverlapping(text.as_ptr(), string, n);
                    *string.add(n) = 0;
                }
                kResultOk
            }
            _ => kResultFalse,
        }
    }

    unsafe fn setBinary(&self, id: AttrID, data: *const c_void, size_in_bytes: uint32) -> tresult {
        if data.is_null() && size_in_bytes > 0 {
            return kInvalidArgument;
        }
        // SAFETY: the caller passes that many bytes.
        unsafe {
            let bytes = if size_in_bytes == 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(data as *const u8, size_in_bytes as usize).to_vec()
            };
            self.set(id, Attribute::Binary(bytes))
        }
    }

    unsafe fn getBinary(
        &self,
        id: AttrID,
        data: *mut *const c_void,
        size_in_bytes: *mut uint32,
    ) -> tresult {
        let Some(key) = (unsafe { Self::key(id) }) else {
            return kInvalidArgument;
        };
        match self.values.borrow().get(&key) {
            Some(Attribute::Binary(bytes)) => {
                // SAFETY: the bytes stay where they are until the attribute is set again, which
                // is what the interface promises the plugin.
                unsafe {
                    *data = bytes.as_ptr() as *const c_void;
                    *size_in_bytes = bytes.len() as uint32;
                }
                kResultOk
            }
            _ => kResultFalse,
        }
    }
}

/// A stream in memory, for saving and loading state.
#[derive(Default)]
struct Stream {
    bytes: RefCell<Vec<u8>>,
    at: Cell<usize>,
}

impl Stream {
    fn with(bytes: Vec<u8>) -> Self {
        Self {
            bytes: RefCell::new(bytes),
            at: Cell::new(0),
        }
    }
}

impl Class for Stream {
    type Interfaces = (IBStream,);
}

impl IBStreamTrait for Stream {
    unsafe fn read(&self, buffer: *mut c_void, count: int32, read: *mut int32) -> tresult {
        let bytes = self.bytes.borrow();
        let at = self.at.get().min(bytes.len());
        let n = (count.max(0) as usize).min(bytes.len() - at);
        // SAFETY: the caller gives room for `count` bytes, and `n` is no more than that.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes[at..].as_ptr(), buffer as *mut u8, n);
            if !read.is_null() {
                *read = n as int32;
            }
        }
        self.at.set(at + n);
        kResultOk
    }

    unsafe fn write(&self, buffer: *mut c_void, count: int32, written: *mut int32) -> tresult {
        let n = count.max(0) as usize;
        let mut bytes = self.bytes.borrow_mut();
        let at = self.at.get();
        if bytes.len() < at + n {
            bytes.resize(at + n, 0);
        }
        // SAFETY: the caller passes `count` bytes, and there is room for them now.
        unsafe {
            std::ptr::copy_nonoverlapping(buffer as *const u8, bytes[at..].as_mut_ptr(), n);
            if !written.is_null() {
                *written = n as int32;
            }
        }
        self.at.set(at + n);
        kResultOk
    }

    unsafe fn seek(&self, pos: int64, mode: int32, result: *mut int64) -> tresult {
        let base = match mode {
            0 => 0,
            1 => self.at.get() as i64,
            2 => self.bytes.borrow().len() as i64,
            _ => return kInvalidArgument,
        };
        let to = base + pos;
        if to < 0 {
            return kResultFalse;
        }
        self.at.set(to as usize);
        if !result.is_null() {
            // SAFETY: the caller gives somewhere to put it.
            unsafe { *result = to };
        }
        kResultOk
    }

    unsafe fn tell(&self, pos: *mut int64) -> tresult {
        if pos.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: checked above.
        unsafe { *pos = self.at.get() as i64 };
        kResultOk
    }
}

/// The notes of one block, in room made beforehand.
struct EventList {
    events: RefCell<Vec<Event>>,
}

impl EventList {
    fn new() -> Self {
        Self {
            events: RefCell::new(Vec::with_capacity(MAX_PLUGIN_EVENTS)),
        }
    }

    fn clear(&self) {
        self.events.borrow_mut().clear();
    }

    fn push(&self, event: Event) -> bool {
        let mut events = self.events.borrow_mut();
        let room = events.len() < events.capacity();
        if room {
            events.push(event);
        }
        room
    }
}

impl Class for EventList {
    type Interfaces = (IEventList,);
}

impl IEventListTrait for EventList {
    unsafe fn getEventCount(&self) -> int32 {
        self.events.borrow().len() as int32
    }

    unsafe fn getEvent(&self, index: int32, e: *mut Event) -> tresult {
        match self.events.borrow().get(index.max(0) as usize) {
            Some(event) if index >= 0 => {
                // SAFETY: the caller gives somewhere to put it.
                unsafe { *e = *event };
                kResultOk
            }
            _ => kResultFalse,
        }
    }

    unsafe fn addEvent(&self, e: *mut Event) -> tresult {
        // SAFETY: the caller passes an event.
        if self.push(unsafe { *e }) {
            kResultOk
        } else {
            kResultFalse
        }
    }
}

/// The changes of one parameter in a block.
struct ParamQueue {
    id: Cell<ParamID>,
    points: RefCell<Vec<(int32, ParamValue)>>,
}

impl Class for ParamQueue {
    type Interfaces = (IParamValueQueue,);
}

impl IParamValueQueueTrait for ParamQueue {
    unsafe fn getParameterId(&self) -> ParamID {
        self.id.get()
    }

    unsafe fn getPointCount(&self) -> int32 {
        self.points.borrow().len() as int32
    }

    unsafe fn getPoint(&self, index: int32, offset: *mut int32, value: *mut ParamValue) -> tresult {
        match self.points.borrow().get(index.max(0) as usize) {
            Some((frame, v)) if index >= 0 => {
                // SAFETY: the caller gives somewhere to put them.
                unsafe {
                    *offset = *frame;
                    *value = *v;
                }
                kResultOk
            }
            _ => kResultFalse,
        }
    }

    unsafe fn addPoint(&self, offset: int32, value: ParamValue, index: *mut int32) -> tresult {
        let mut points = self.points.borrow_mut();
        if points.len() >= points.capacity() {
            return kResultFalse;
        }
        points.push((offset, value));
        if !index.is_null() {
            // SAFETY: the caller gives somewhere to put it.
            unsafe { *index = points.len() as int32 - 1 };
        }
        kResultOk
    }
}

/// The parameter changes of one block, in room made beforehand.
struct ParamChanges {
    queues: Vec<ComWrapper<ParamQueue>>,
    used: Cell<usize>,
}

impl ParamChanges {
    fn new() -> Self {
        Self {
            queues: (0..MAX_PARAM_QUEUES)
                .map(|_| {
                    ComWrapper::new(ParamQueue {
                        id: Cell::new(0),
                        points: RefCell::new(Vec::with_capacity(MAX_PARAM_POINTS)),
                    })
                })
                .collect(),
            used: Cell::new(0),
        }
    }

    fn clear(&self) {
        for q in &self.queues[..self.used.get()] {
            q.points.borrow_mut().clear();
        }
        self.used.set(0);
    }

    /// The queue of a parameter and its index, a new one if it has none yet, while there is room.
    fn queue_of(&self, id: ParamID) -> Option<usize> {
        let used = self.used.get();
        if let Some(i) = self.queues[..used].iter().position(|q| q.id.get() == id) {
            return Some(i);
        }
        let q = self.queues.get(used)?;
        q.id.set(id);
        self.used.set(used + 1);
        Some(used)
    }

    /// A change at a frame of the block. Past the room made, it is dropped rather than allocating.
    fn add(&self, id: ParamID, frame: i32, value: f64) {
        if let Some(i) = self.queue_of(id) {
            let mut points = self.queues[i].points.borrow_mut();
            if points.len() < points.capacity() {
                points.push((frame, value));
            }
        }
    }

    fn pointer_to(&self, i: usize) -> *mut IParamValueQueue {
        self.queues
            .get(i)
            .and_then(|q| q.as_com_ref::<IParamValueQueue>())
            .map_or(std::ptr::null_mut(), |r| r.as_ptr())
    }
}

impl Class for ParamChanges {
    type Interfaces = (IParameterChanges,);
}

impl IParameterChangesTrait for ParamChanges {
    unsafe fn getParameterCount(&self) -> int32 {
        self.used.get() as int32
    }

    unsafe fn getParameterData(&self, index: int32) -> *mut IParamValueQueue {
        if index < 0 || index as usize >= self.used.get() {
            return std::ptr::null_mut();
        }
        self.pointer_to(index as usize)
    }

    unsafe fn addParameterData(
        &self,
        id: *const ParamID,
        index: *mut int32,
    ) -> *mut IParamValueQueue {
        // SAFETY: the caller passes an identifier and somewhere to put the index.
        unsafe {
            match self.queue_of(*id) {
                Some(i) => {
                    if !index.is_null() {
                        *index = i as int32;
                    }
                    self.pointer_to(i)
                }
                None => std::ptr::null_mut(),
            }
        }
    }
}

/// What the plugin's controller tells the host, on the UI thread.
struct Handler {
    edits: RefCell<rtrb::Producer<(ParamID, ParamValue)>>,
    /// The same moves, for automation. On the UI thread, as `performEdit` is.
    moved: RefCell<Vec<(ParamID, ParamValue)>>,
    /// Controls held in the plugin's window, and whether one was let go. On the UI thread too.
    held: std::cell::Cell<u32>,
    ended: std::cell::Cell<bool>,
    dirty: Arc<AtomicBool>,
}

impl Class for Handler {
    type Interfaces = (IComponentHandler,);
}

impl IComponentHandlerTrait for Handler {
    unsafe fn beginEdit(&self, _id: ParamID) -> tresult {
        self.held.set(self.held.get().saturating_add(1));
        kResultOk
    }

    unsafe fn performEdit(&self, id: ParamID, value: ParamValue) -> tresult {
        self.dirty.store(true, Ordering::Release);
        self.moved.borrow_mut().push((id, value));
        match self.edits.borrow_mut().push((id, value)) {
            Ok(()) => kResultOk,
            Err(_) => kResultFalse,
        }
    }

    unsafe fn endEdit(&self, _id: ParamID) -> tresult {
        self.held.set(self.held.get().saturating_sub(1));
        self.ended.set(true);
        kResultOk
    }

    unsafe fn restartComponent(&self, _flags: int32) -> tresult {
        self.dirty.store(true, Ordering::Release);
        kResultOk
    }
}

/// Where a plugin's view asks for another size. Packed width and height; 0 is nothing asked.
struct Frame {
    resize: Arc<AtomicU64>,
}

impl Class for Frame {
    type Interfaces = (IPlugFrame,);
}

impl IPlugFrameTrait for Frame {
    unsafe fn resizeView(&self, _view: *mut IPlugView, size: *mut ViewRect) -> tresult {
        if size.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: checked above; the caller passes a rectangle.
        let rect = unsafe { *size };
        let (w, h) = (
            (rect.right - rect.left).max(0),
            (rect.bottom - rect.top).max(0),
        );
        self.resize
            .store(((w as u64) << 32) | h as u64, Ordering::Release);
        kResultOk
    }
}

// ------------------------------------------------------------------------- loading

/// A loaded `.vst3` library. It stays loaded until the program ends: several plugins misbehave
/// when unloaded and loaded again, and the memory is small next to what their instances take.
struct Module {
    factory: ComPtr<IPluginFactory>,
    host: ComWrapper<HostApp>,
    _library: libloading::Library,
}

// SAFETY: the factory is only used on the thread that loads plugins, as the modules below are
// reached through a lock held while using them. The library handle has no thread of its own.
unsafe impl Send for Module {}
unsafe impl Sync for Module {}

fn modules() -> &'static Mutex<HashMap<PathBuf, Arc<Module>>> {
    static MODULES: OnceLock<Mutex<HashMap<PathBuf, Arc<Module>>>> = OnceLock::new();
    MODULES.get_or_init(Mutex::default)
}

fn load_module(path: &Path) -> Result<Arc<Module>, Error> {
    let mut all = modules()
        .lock()
        .map_err(|_| err(path, "the plugin list is unusable after an earlier failure"))?;
    if let Some(module) = all.get(path) {
        return Ok(module.clone());
    }
    let file = module_file(path);
    // SAFETY: loading a plugin runs its code, which nothing here can check. That is what a host is
    // for: the file is one the user installed as a plugin. `InitDll` and `GetPluginFactory` have
    // the signatures the VST3 documentation gives them, and the factory reference is taken over.
    let module = unsafe {
        let library = libloading::Library::new(&file)
            .map_err(|e| err(path, &format!("cannot load it: {e}")))?;
        if let Ok(init) = library.get::<unsafe extern "system" fn() -> bool>(b"InitDll\0") {
            if !init() {
                return Err(err(path, "it refused to start"));
            }
        }
        let get = library
            .get::<unsafe extern "system" fn() -> *mut IPluginFactory>(b"GetPluginFactory\0")
            .map_err(|_| err(path, "it is not a VST3 plugin"))?;
        let factory =
            ComPtr::from_raw(get()).ok_or_else(|| err(path, "it has no plugin factory"))?;
        Arc::new(Module {
            factory,
            host: ComWrapper::new(HostApp),
            _library: library,
        })
    };
    all.insert(path.to_path_buf(), module.clone());
    Ok(module)
}

/// Loads a `.vst3` and reads what plugins it offers. This runs the file's code, so a plugin that
/// crashes here crashes the caller: scan in a process of its own.
pub fn scan_vst3_file(path: &Path) -> Result<Vec<PluginInfo>, Error> {
    let module = load_module(path)?;
    let mut found = Vec::new();
    // SAFETY: the factory is valid while the module lives, and each info struct is plain data
    // that the plugin fills in.
    unsafe {
        let mut factory_info: PFactoryInfo = std::mem::zeroed();
        let vendor = if module.factory.getFactoryInfo(&mut factory_info) == kResultOk {
            text_of(&factory_info.vendor)
        } else {
            String::new()
        };
        let second = module.factory.cast::<IPluginFactory2>();
        for i in 0..module.factory.countClasses() {
            let mut info: PClassInfo = std::mem::zeroed();
            if module.factory.getClassInfo(i, &mut info) != kResultOk
                || text_of(&info.category) != AUDIO_CLASS
            {
                continue;
            }
            let mut plugin = PluginInfo {
                id: uid_text(&info.cid),
                name: text_of(&info.name),
                vendor: vendor.clone(),
                version: String::new(),
                format: PluginFormat::Vst3,
                path: path.to_path_buf(),
                is_instrument: false,
            };
            if let Some(second) = &second {
                let mut more: PClassInfo2 = std::mem::zeroed();
                if second.getClassInfo2(i, &mut more) == kResultOk {
                    plugin.version = text_of(&more.version);
                    let own = text_of(&more.vendor);
                    if !own.is_empty() {
                        plugin.vendor = own;
                    }
                    plugin.is_instrument = text_of(&more.subCategories)
                        .split('|')
                        .any(|c| c.eq_ignore_ascii_case("Instrument"));
                }
            }
            found.push(plugin);
        }
    }
    Ok(found)
}

/// SAFETY: `factory` must be valid, and `cid` a class of it.
unsafe fn create<I: Interface>(factory: &ComPtr<IPluginFactory>, cid: &TUID) -> Option<ComPtr<I>> {
    let mut made: *mut c_void = std::ptr::null_mut();
    let result =
        unsafe { factory.createInstance(cid.as_ptr(), I::IID.as_ptr() as FIDString, &mut made) };
    if result != kResultOk {
        return None;
    }
    unsafe { ComPtr::from_raw(made as *mut I) }
}

/// The channels each audio bus in one direction has.
///
/// SAFETY: `component` must be initialized.
unsafe fn bus_layout(component: &ComPtr<IComponent>, direction: i32) -> Vec<usize> {
    let count = unsafe { component.getBusCount(MediaTypes_::kAudio as MediaType, direction) };
    (0..count.max(0))
        .map(|i| unsafe {
            let mut info: BusInfo = std::mem::zeroed();
            if component.getBusInfo(MediaTypes_::kAudio as MediaType, direction, i, &mut info)
                == kResultOk
            {
                info.channelCount.max(0) as usize
            } else {
                0
            }
        })
        .collect()
}

/// Asks for stereo on the main buses, keeps what the others have, and turns on the main ones and
/// the first note input. A plugin may say no: its layout is read afterwards, whatever it is.
///
/// SAFETY: both must be the initialized halves of one plugin, not yet active.
unsafe fn set_up_buses(component: &ComPtr<IComponent>, processor: &ComPtr<IAudioProcessor>) {
    use BusDirections_::{kInput, kOutput};
    use MediaTypes_::{kAudio, kEvent};
    let stereo = SpeakerArr::kStereo;
    let arrangement = |direction: i32| -> Vec<SpeakerArrangement> {
        let count = unsafe { component.getBusCount(kAudio as MediaType, direction) };
        (0..count.max(0))
            .map(|i| {
                let mut now = stereo;
                if i > 0 {
                    unsafe { processor.getBusArrangement(direction, i, &mut now) };
                }
                now
            })
            .collect()
    };
    let mut inputs = arrangement(kInput);
    let mut outputs = arrangement(kOutput);
    unsafe {
        processor.setBusArrangements(
            inputs.as_mut_ptr(),
            inputs.len() as int32,
            outputs.as_mut_ptr(),
            outputs.len() as int32,
        );
        if !inputs.is_empty() {
            component.activateBus(kAudio as MediaType, kInput, 0, 1);
        }
        if !outputs.is_empty() {
            component.activateBus(kAudio as MediaType, kOutput, 0, 1);
        }
        if component.getBusCount(kEvent as MediaType, kInput) > 0 {
            component.activateBus(kEvent as MediaType, kInput, 0, 1);
        }
    }
}

/// Loads plugin `id` of a `.vst3` and activates it for the engine's sample rate and block. Failure
/// is an error to show, never a panic.
pub fn load_vst3(path: &Path, id: &str, sample_rate: u32) -> Result<Loaded, Error> {
    let module = load_module(path)?;
    let cid = uid_from_text(id).ok_or_else(|| err(path, "the plugin's identifier is not valid"))?;
    let dirty = Arc::new(AtomicBool::new(false));
    let released = Arc::new(AtomicBool::new(false));
    let resize = Arc::new(AtomicU64::new(0));
    let (edits_in, edits_out) = rtrb::RingBuffer::new(EDIT_QUEUE);
    let handler = ComWrapper::new(Handler {
        edits: RefCell::new(edits_in),
        moved: RefCell::new(Vec::new()),
        held: std::cell::Cell::new(0),
        ended: std::cell::Cell::new(false),
        dirty: dirty.clone(),
    });
    let frame = ComWrapper::new(Frame {
        resize: resize.clone(),
    });
    // SAFETY: every pointer comes from the plugin and is used as the VST3 documentation says, in
    // the order it gives. The objects passed in live as long as the `LoadedVst3` holding them.
    unsafe {
        let host = module
            .host
            .to_com_ptr::<FUnknown>()
            .ok_or_else(|| err(path, "the host interface is missing"))?;
        let component = create::<IComponent>(&module.factory, &cid)
            .ok_or_else(|| err(path, "the plugin could not be created"))?;
        if component.initialize(host.as_ptr()) != kResultOk {
            return Err(err(path, "the plugin did not start"));
        }
        let Some(processor) = component.cast::<IAudioProcessor>() else {
            component.terminate();
            return Err(err(path, "the plugin has no audio processor"));
        };
        // A controller in the same object, or in a class of its own.
        let (controller, separate) = match component.cast::<IEditController>() {
            Some(c) => (Some(c), false),
            None => {
                let mut controller_cid: TUID = [0; 16];
                let made = if component.getControllerClassId(&mut controller_cid) == kResultOk {
                    create::<IEditController>(&module.factory, &controller_cid)
                } else {
                    None
                };
                match made {
                    Some(c) if c.initialize(host.as_ptr()) == kResultOk => (Some(c), true),
                    _ => (None, false),
                }
            }
        };
        let mut points = None;
        if let Some(c) = &controller {
            if let Some(h) = handler.to_com_ptr::<IComponentHandler>() {
                c.setComponentHandler(h.as_ptr());
            }
            if separate {
                if let (Some(a), Some(b)) = (
                    component.cast::<IConnectionPoint>(),
                    c.cast::<IConnectionPoint>(),
                ) {
                    a.connect(b.as_ptr());
                    b.connect(a.as_ptr());
                    points = Some((a, b));
                }
                // The controller starts from what the processor holds.
                let stream = ComWrapper::new(Stream::default());
                if let Some(s) = stream.to_com_ptr::<IBStream>() {
                    if component.getState(s.as_ptr()) == kResultOk {
                        stream.at.set(0);
                        c.setComponentState(s.as_ptr());
                    }
                }
            }
        }
        set_up_buses(&component, &processor);
        let mut setup = ProcessSetup {
            processMode: ProcessModes_::kRealtime as int32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as int32,
            maxSamplesPerBlock: PLUGIN_BLOCK as int32,
            sampleRate: f64::from(sample_rate),
        };
        if processor.setupProcessing(&mut setup) != kResultOk {
            component.terminate();
            return Err(err(path, &format!("the plugin refused {sample_rate} Hz")));
        }
        let inputs = bus_layout(&component, BusDirections_::kInput);
        let outputs = bus_layout(&component, BusDirections_::kOutput);
        if component.setActive(1) != kResultOk {
            component.terminate();
            return Err(err(path, "the plugin would not activate"));
        }
        let latency = processor.getLatencySamples() as usize;
        let midi = controller
            .as_ref()
            .and_then(|c| c.cast::<IMidiMapping>())
            .map(|mapping| midi_table(&mapping))
            .unwrap_or_else(|| Box::new([None; MIDI_CONTROLLERS]));
        let params = controller
            .as_ref()
            .map_or_else(Vec::new, |c| vst3_params(c));
        let audio = Vst3Processor::new(
            processor,
            &inputs,
            &outputs,
            edits_out,
            midi,
            latency,
            f64::from(sample_rate),
            released.clone(),
        );
        Ok((
            Box::new(LoadedVst3 {
                params,
                component,
                controller,
                separate,
                points,
                handler,
                frame,
                latency,
                dirty,
                released,
                resize,
                editor: None,
                closed_at: None,
                _module: module,
            }),
            Box::new(audio),
        ))
    }
}

/// The parameters that can be automated. VST3's values are positions 0..=1 already.
///
/// SAFETY: `controller` must be valid.
unsafe fn vst3_params(controller: &ComPtr<IEditController>) -> Vec<crate::PluginParam> {
    let mut params = Vec::new();
    for index in 0..controller.getParameterCount() {
        // SAFETY: the struct is plain data, and the plugin fills it in.
        let mut info: ParameterInfo = std::mem::zeroed();
        if controller.getParameterInfo(index, &mut info) != kResultOk {
            continue;
        }
        if info.flags & ParameterInfo_::ParameterFlags_::kCanAutomate == 0 {
            continue;
        }
        let title: Vec<u16> = info.title.iter().copied().take_while(|c| *c != 0).collect();
        params.push(crate::PluginParam {
            id: info.id,
            name: String::from_utf16_lossy(&title),
        });
    }
    params
}

/// VST3 has no MIDI controllers: the plugin says which parameter each one moves.
///
/// SAFETY: `mapping` must be valid.
unsafe fn midi_table(mapping: &ComPtr<IMidiMapping>) -> Box<[Option<ParamID>; MIDI_CONTROLLERS]> {
    let mut table = Box::new([None; MIDI_CONTROLLERS]);
    for (number, slot) in table.iter_mut().enumerate() {
        let mut id: ParamID = 0;
        let found =
            unsafe { mapping.getMidiControllerAssignment(0, 0, number as CtrlNumber, &mut id) };
        if found == kResultOk {
            *slot = Some(id);
        }
    }
    table
}

// ------------------------------------------------------------------------- the UI-thread half

struct OpenEditor {
    view: ComPtr<IPlugView>,
    window: EditorWindow,
    size: (u32, u32),
}

struct LoadedVst3 {
    component: ComPtr<IComponent>,
    controller: Option<ComPtr<IEditController>>,
    params: Vec<crate::PluginParam>,
    separate: bool,
    points: Option<(ComPtr<IConnectionPoint>, ComPtr<IConnectionPoint>)>,
    handler: ComWrapper<Handler>,
    frame: ComWrapper<Frame>,
    latency: usize,
    dirty: Arc<AtomicBool>,
    released: Arc<AtomicBool>,
    resize: Arc<AtomicU64>,
    editor: Option<OpenEditor>,
    closed_at: Option<(i32, i32)>,
    _module: Arc<Module>,
}

/// Runs `fill` on a new stream and gives back what it wrote.
fn collect_from(fill: impl FnOnce(*mut IBStream) -> tresult) -> Option<Vec<u8>> {
    let stream = ComWrapper::new(Stream::default());
    let pointer = stream.to_com_ptr::<IBStream>()?;
    if fill(pointer.as_ptr()) != kResultOk {
        return None;
    }
    let bytes = stream.bytes.borrow().clone();
    Some(bytes)
}

/// The two parts of a VST3 plugin's state in one block of bytes.
fn pack_state(component: &[u8], controller: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + component.len() + controller.len());
    out.extend_from_slice(STATE_MAGIC);
    out.extend_from_slice(&(component.len() as u32).to_le_bytes());
    out.extend_from_slice(component);
    out.extend_from_slice(controller);
    out
}

fn unpack_state(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let rest = bytes.strip_prefix(STATE_MAGIC)?;
    let (length, rest) = rest.split_first_chunk::<4>()?;
    let length = u32::from_le_bytes(*length) as usize;
    (rest.len() >= length).then(|| rest.split_at(length))
}

impl LoadedPlugin for LoadedVst3 {
    fn params(&self) -> Vec<crate::PluginParam> {
        self.params.clone()
    }

    fn take_edits(&mut self) -> Vec<(u32, f64)> {
        std::mem::take(&mut *self.handler.moved.borrow_mut())
    }

    fn editing(&self) -> bool {
        self.handler.held.get() > 0
    }

    fn take_edit_ended(&mut self) -> bool {
        self.handler.ended.take()
    }

    fn param_value(&mut self, id: u32) -> Option<f64> {
        let controller = self.controller.as_ref()?;
        // SAFETY: the controller is valid while this half lives, and this is its thread.
        Some(unsafe { controller.getParamNormalized(id) })
    }

    fn show_param(&mut self, id: u32, value: f64) {
        if let Some(controller) = &self.controller {
            // SAFETY: the controller is valid while this half lives, and this is its thread.
            unsafe {
                controller.setParamNormalized(id, value);
            }
        }
    }

    fn latency_samples(&self) -> usize {
        self.latency
    }

    fn released(&self) -> bool {
        self.released.load(Ordering::Acquire)
    }

    fn tick(&mut self) {
        let Some(open) = self.editor.as_mut() else {
            return;
        };
        if open.window.take_close_request() {
            self.close_editor();
            return;
        }
        // What the plugin asked for: the window follows, then the plugin is told.
        let asked = self.resize.swap(0, Ordering::AcqRel);
        if asked != 0 {
            let (width, height) = ((asked >> 32) as u32, asked as u32);
            open.size = (width, height);
            open.window.set_client_size(width, height);
            let _ = open.window.take_new_size();
            let mut rect = ViewRect {
                left: 0,
                top: 0,
                right: width as i32,
                bottom: height as i32,
            };
            // SAFETY: the view lives while the editor is open.
            unsafe { open.view.onSize(&mut rect) };
        }
        // What the user dragged the frame to: the plugin says what it can do.
        if let Some((width, height)) = open.window.take_new_size() {
            if (width, height) != open.size {
                let mut rect = ViewRect {
                    left: 0,
                    top: 0,
                    right: width as i32,
                    bottom: height as i32,
                };
                // SAFETY: the view lives while the editor is open, and `rect` is a valid local.
                unsafe {
                    open.view.checkSizeConstraint(&mut rect);
                    open.view.onSize(&mut rect);
                }
                let fits = (
                    (rect.right - rect.left) as u32,
                    (rect.bottom - rect.top) as u32,
                );
                open.size = fits;
                if fits != (width, height) {
                    open.window.set_client_size(fits.0, fits.1);
                    let _ = open.window.take_new_size();
                }
            }
        }
    }

    fn editor_is_open(&self) -> bool {
        self.editor.is_some()
    }

    fn take_closed_position(&mut self) -> Option<(i32, i32)> {
        self.closed_at.take()
    }

    fn open_editor(
        &mut self,
        title: &str,
        owner: Option<isize>,
        position: Option<(i32, i32)>,
    ) -> Result<(), Error> {
        if let Some(open) = &self.editor {
            open.window.raise();
            return Ok(());
        }
        let no_editor = || Error(format!("{title} has no editor window"));
        let controller = self.controller.as_ref().ok_or_else(no_editor)?;
        // SAFETY: the view is the plugin's to give, and owned from here. The window outlives the
        // attachment: `close_editor` removes the view first, and it is the only place the window
        // is dropped. The frame lives as long as `self`.
        unsafe {
            let view = ComPtr::from_raw(controller.createView(c"editor".as_ptr()))
                .ok_or_else(no_editor)?;
            if view.isPlatformTypeSupported(kPlatformTypeHWND) != kResultTrue {
                return Err(no_editor());
            }
            let resizable = view.canResize() == kResultTrue;
            let window = EditorWindow::new(title, resizable, owner, position)?;
            if let Some(scale) = view.cast::<IPlugViewContentScaleSupport>() {
                scale.setContentScaleFactor(window.scale() as ScaleFactor);
            }
            if let Some(frame) = self.frame.to_com_ptr::<IPlugFrame>() {
                view.setFrame(frame.as_ptr());
            }
            let mut rect: ViewRect = std::mem::zeroed();
            if view.getSize(&mut rect) != kResultOk {
                return Err(Error(format!("{title} did not say how big it is")));
            }
            window.set_client_size(
                (rect.right - rect.left).max(1) as u32,
                (rect.bottom - rect.top).max(1) as u32,
            );
            if view.attached(window.handle(), kPlatformTypeHWND) != kResultOk {
                view.setFrame(std::ptr::null_mut());
                return Err(Error(format!("cannot open {title}")));
            }
            // A plugin knows its real size only once it has its window.
            if view.getSize(&mut rect) == kResultOk {
                window.set_client_size(
                    (rect.right - rect.left).max(1) as u32,
                    (rect.bottom - rect.top).max(1) as u32,
                );
            }
            window.show();
            let _ = window.take_new_size();
            self.editor = Some(OpenEditor {
                view,
                window,
                size: (
                    (rect.right - rect.left).max(1) as u32,
                    (rect.bottom - rect.top).max(1) as u32,
                ),
            });
        }
        Ok(())
    }

    fn close_editor(&mut self) {
        if let Some(open) = self.editor.take() {
            if let Some(at) = open.window.position() {
                self.closed_at = Some(at);
            }
            open.window.hide();
            // SAFETY: the view is attached to this window, which is dropped after this.
            unsafe {
                open.view.removed();
                open.view.setFrame(std::ptr::null_mut());
            }
        }
    }

    fn save_state(&mut self) -> Result<Option<Vec<u8>>, Error> {
        // SAFETY: both pointers live as long as `self`, and the stream outlives the calls.
        unsafe {
            let component = collect_from(|s| self.component.getState(s))
                .ok_or_else(|| Error("the plugin would not give its settings".to_string()))?;
            let controller = self
                .controller
                .as_ref()
                .and_then(|c| collect_from(|s| c.getState(s)))
                .unwrap_or_default();
            Ok(Some(pack_state(&component, &controller)))
        }
    }

    fn load_state(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let (component, controller) = unpack_state(bytes)
            .ok_or_else(|| Error("the saved settings are damaged".to_string()))?;
        let stream = |bytes: &[u8]| ComWrapper::new(Stream::with(bytes.to_vec()));
        // SAFETY: both pointers live as long as `self`, and each stream outlives its call.
        unsafe {
            let s = stream(component);
            let pointer = s
                .to_com_ptr::<IBStream>()
                .ok_or_else(|| Error("the stream interface is missing".to_string()))?;
            if self.component.setState(pointer.as_ptr()) != kResultOk {
                return Err(Error("the plugin did not take its settings".to_string()));
            }
            if let Some(c) = &self.controller {
                if self.separate {
                    s.at.set(0);
                    c.setComponentState(pointer.as_ptr());
                }
                if !controller.is_empty() {
                    let s = stream(controller);
                    if let Some(pointer) = s.to_com_ptr::<IBStream>() {
                        c.setState(pointer.as_ptr());
                    }
                }
            }
        }
        self.dirty.store(false, Ordering::Release);
        Ok(())
    }

    fn take_dirty(&mut self) -> bool {
        self.dirty.swap(false, Ordering::AcqRel)
    }
}

impl Drop for LoadedVst3 {
    fn drop(&mut self) {
        self.close_editor();
        if !self.released() {
            // The audio half is still out there: taking the plugin down under it would crash, so
            // it is left running. The rack waits for it, so this is only the program ending.
            std::mem::forget(self.component.clone());
            return;
        }
        // SAFETY: nothing else uses the plugin now, and this is the order the documentation gives.
        unsafe {
            self.component.setActive(0);
            if let Some((a, b)) = &self.points {
                a.disconnect(b.as_ptr());
                b.disconnect(a.as_ptr());
            }
            if let Some(c) = &self.controller {
                c.setComponentHandler(std::ptr::null_mut());
                if self.separate {
                    c.terminate();
                }
            }
            self.component.terminate();
        }
        // The handler is the plugin's until here.
        let _ = &self.handler;
    }
}

// ------------------------------------------------------------------------- the audio-thread half

/// Audio buffers for every bus in one direction and the pointers the plugin reads them through.
struct Buses {
    channels: Vec<Vec<Vec<f32>>>,
    /// Kept for the addresses in `buses`.
    _pointers: Vec<Vec<*mut f32>>,
    buses: Vec<AudioBusBuffers>,
}

impl Buses {
    fn new(layout: &[usize]) -> Self {
        let mut channels: Vec<Vec<Vec<f32>>> = layout
            .iter()
            .map(|&n| (0..n).map(|_| vec![0.0; PLUGIN_BLOCK]).collect())
            .collect();
        let mut pointers: Vec<Vec<*mut f32>> = channels
            .iter_mut()
            .map(|bus| bus.iter_mut().map(|c| c.as_mut_ptr()).collect())
            .collect();
        let buses = pointers
            .iter_mut()
            .map(|p| AudioBusBuffers {
                numChannels: p.len() as int32,
                silenceFlags: 0,
                __field0: AudioBusBuffers__type0 {
                    channelBuffers32: p.as_mut_ptr(),
                },
            })
            .collect();
        Self {
            channels,
            _pointers: pointers,
            buses,
        }
    }
}

/// Parameter moves one block can carry.
const PENDING_PARAMS: usize = 256;

struct Vst3Processor {
    processor: ComPtr<IAudioProcessor>,
    inputs: Buses,
    outputs: Buses,
    events: ComWrapper<EventList>,
    events_out: ComWrapper<EventList>,
    params: ComWrapper<ParamChanges>,
    params_out: ComWrapper<ParamChanges>,
    edits: rtrb::Consumer<(ParamID, ParamValue)>,
    /// Moves from automation, waiting for the next block. Past its capacity they wait for the
    /// next value.
    pending: Vec<(ParamID, ParamValue)>,
    midi: Box<[Option<ParamID>; MIDI_CONTROLLERS]>,
    latency: usize,
    sample_rate: f64,
    continuous: i64,
    started: bool,
    released: Arc<AtomicBool>,
}

// SAFETY: this half is handed to the audio thread once and used only there until it comes back.
// The COM objects it holds are the audio half of the plugin, or objects of ours used only here.
unsafe impl Send for Vst3Processor {}

impl Vst3Processor {
    #[allow(clippy::too_many_arguments)]
    fn new(
        processor: ComPtr<IAudioProcessor>,
        inputs: &[usize],
        outputs: &[usize],
        edits: rtrb::Consumer<(ParamID, ParamValue)>,
        midi: Box<[Option<ParamID>; MIDI_CONTROLLERS]>,
        latency: usize,
        sample_rate: f64,
        released: Arc<AtomicBool>,
    ) -> Self {
        Self {
            processor,
            inputs: Buses::new(inputs),
            outputs: Buses::new(outputs),
            events: ComWrapper::new(EventList::new()),
            events_out: ComWrapper::new(EventList::new()),
            params: ComWrapper::new(ParamChanges::new()),
            params_out: ComWrapper::new(ParamChanges::new()),
            edits,
            pending: Vec::with_capacity(PENDING_PARAMS),
            midi,
            latency,
            sample_rate,
            continuous: 0,
            started: false,
            released,
        }
    }

    /// Our events as VST3's, and controllers as the parameters they are mapped to.
    fn take_events(&self, events: &[PluginEvent]) {
        for e in events {
            match e.kind {
                PluginEventKind::NoteOn { key, velocity } => {
                    self.events.push(note_on(e.frame, key, velocity));
                }
                PluginEventKind::NoteOff { key } => {
                    self.events.push(note_off(e.frame, key));
                }
                PluginEventKind::Control { number, value } => {
                    if let Some(Some(id)) = self.midi.get(number as usize) {
                        self.params
                            .add(*id, e.frame as i32, f64::from(value) / 127.0);
                    }
                }
                PluginEventKind::PitchBend { value } => {
                    if let Some(id) = self.midi[ControllerNumbers_::kPitchBend as usize] {
                        let v = (f64::from(value.clamp(-1.0, 1.0)) + 1.0) * 0.5;
                        self.params.add(id, e.frame as i32, v);
                    }
                }
            }
        }
    }
}

fn empty_event(frame: u32, kind: u32) -> Event {
    // SAFETY: an event is plain data, for which zero is a valid value in every field.
    let mut event: Event = unsafe { std::mem::zeroed() };
    event.sampleOffset = frame as int32;
    event.flags = Event_::EventFlags_::kIsLive as uint16;
    event.r#type = kind as uint16;
    event
}

fn note_on(frame: u32, key: u8, velocity: f32) -> Event {
    let mut event = empty_event(frame, Event_::EventTypes_::kNoteOnEvent as u32);
    event.__field0.noteOn = NoteOnEvent {
        channel: 0,
        pitch: i16::from(key),
        tuning: 0.0,
        velocity: velocity.clamp(0.0, 1.0),
        length: 0,
        noteId: -1,
    };
    event
}

fn note_off(frame: u32, key: u8) -> Event {
    let mut event = empty_event(frame, Event_::EventTypes_::kNoteOffEvent as u32);
    event.__field0.noteOff = NoteOffEvent {
        channel: 0,
        pitch: i16::from(key),
        velocity: 0.0,
        noteId: -1,
        tuning: 0.0,
    };
    event
}

/// Where the song is, in VST3's words.
fn process_context(now: &PluginTransport, sample_rate: f64, continuous: i64) -> ProcessContext {
    let mut state = Flags::kTempoValid
        | Flags::kTimeSigValid
        | Flags::kProjectTimeMusicValid
        | Flags::kBarPositionValid
        | Flags::kContTimeValid;
    if now.playing {
        state |= Flags::kPlaying;
    }
    let (cycle_start, cycle_end) = now.cycle.unwrap_or((0.0, 0.0));
    if now.cycle.is_some() {
        state |= Flags::kCycleActive | Flags::kCycleValid;
    }
    // SAFETY: a context is plain data, for which zero is a valid value in every field.
    let mut context: ProcessContext = unsafe { std::mem::zeroed() };
    context.state = state as uint32;
    context.sampleRate = sample_rate;
    context.projectTimeSamples = (now.seconds * sample_rate) as TSamples;
    context.continousTimeSamples = continuous;
    context.projectTimeMusic = now.beat;
    context.barPositionMusic = now.bar_start;
    context.cycleStartMusic = cycle_start;
    context.cycleEndMusic = cycle_end;
    context.tempo = now.tempo;
    context.timeSigNumerator = i32::from(now.signature.0);
    context.timeSigDenominator = i32::from(now.signature.1);
    context
}

impl PluginProcessor for Vst3Processor {
    fn set_param(&mut self, id: u32, value: f64) {
        if self.pending.len() < self.pending.capacity() {
            self.pending.push((id, value.clamp(0.0, 1.0)));
        }
    }

    fn process(
        &mut self,
        input: [&[f32]; 2],
        output: [&mut [f32]; 2],
        events: &[PluginEvent],
        transport: &PluginTransport,
    ) {
        self.events.clear();
        self.events_out.clear();
        self.params.clear();
        self.params_out.clear();
        while let Ok((id, value)) = self.edits.pop() {
            self.params.add(id, 0, value);
        }
        for (id, value) in self.pending.drain(..) {
            self.params.add(id, 0, value);
        }
        self.take_events(events);
        fill_main_input(&mut self.inputs.channels, input);
        let mut context = process_context(transport, self.sample_rate, self.continuous);
        self.continuous += PLUGIN_BLOCK as i64;
        let params = self.params.as_com_ref::<IParameterChanges>();
        let params_out = self.params_out.as_com_ref::<IParameterChanges>();
        let events_in = self.events.as_com_ref::<IEventList>();
        let events_out = self.events_out.as_com_ref::<IEventList>();
        let mut data = ProcessData {
            processMode: ProcessModes_::kRealtime as int32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as int32,
            numSamples: PLUGIN_BLOCK as int32,
            numInputs: self.inputs.buses.len() as int32,
            numOutputs: self.outputs.buses.len() as int32,
            inputs: self.inputs.buses.as_mut_ptr(),
            outputs: self.outputs.buses.as_mut_ptr(),
            inputParameterChanges: params.map_or(std::ptr::null_mut(), |r| r.as_ptr()),
            outputParameterChanges: params_out.map_or(std::ptr::null_mut(), |r| r.as_ptr()),
            inputEvents: events_in.map_or(std::ptr::null_mut(), |r| r.as_ptr()),
            outputEvents: events_out.map_or(std::ptr::null_mut(), |r| r.as_ptr()),
            processContext: &mut context,
        };
        // SAFETY: every pointer in `data` points at something this struct owns, which stays put
        // for the call, and the block is the size promised in `setupProcessing`.
        let ok = unsafe {
            if !self.started {
                self.processor.setProcessing(1);
                self.started = true;
            }
            self.processor.process(&mut data) == kResultOk
        };
        if ok {
            read_main_output(&self.outputs.channels, output);
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

impl Drop for Vst3Processor {
    fn drop(&mut self) {
        if self.started {
            // SAFETY: nothing else processes now: this came back from the audio thread.
            unsafe { self.processor.setProcessing(0) };
        }
        self.released.store(true, Ordering::Release);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::editor_window::pump_messages_for;
    use std::time::Duration;

    #[test]
    fn class_identifiers_go_to_text_and_back() {
        let mut uid: TUID = [0; 16];
        for (i, part) in uid.iter_mut().enumerate() {
            *part = (i as i32 * 17 - 100) as std::ffi::c_char;
        }
        let text = uid_text(&uid);
        assert_eq!(text.len(), 32);
        assert_eq!(uid_from_text(&text), Some(uid));
        assert_eq!(uid_from_text("short"), None);
        assert_eq!(uid_from_text(&"Z".repeat(32)), None);
    }

    #[test]
    fn vst3_files_and_bundles_are_found_and_bundles_are_not_looked_into() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let bundle = root.join("Bundle.vst3");
        std::fs::create_dir_all(bundle.join("Contents").join("x86_64-win")).unwrap();
        std::fs::write(
            bundle
                .join("Contents")
                .join("x86_64-win")
                .join("Bundle.vst3"),
            b"",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("Vendor")).unwrap();
        std::fs::write(root.join("Vendor").join("One.VST3"), b"").unwrap();
        std::fs::write(root.join("Vendor").join("readme.txt"), b"").unwrap();
        let found = find_vst3_files(&[root.to_path_buf(), root.to_path_buf()]);
        assert_eq!(
            found,
            [bundle.clone(), root.join("Vendor").join("One.VST3")]
        );
        assert_eq!(
            module_file(&bundle),
            bundle
                .join("Contents")
                .join("x86_64-win")
                .join("Bundle.vst3")
        );
        let one = root.join("Vendor").join("One.VST3");
        assert_eq!(module_file(&one), one);
    }

    #[test]
    fn a_file_that_is_not_a_plugin_is_an_error_not_a_crash() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("fake.vst3");
        std::fs::write(&fake, b"not a plugin").unwrap();
        assert!(scan_vst3_file(&fake).is_err());
        assert!(load_vst3(&fake, &"0".repeat(32), 48_000).is_err());
    }

    #[test]
    fn the_stream_reads_writes_and_seeks_like_a_file() {
        let stream = Stream::default();
        let mut data = [1u8, 2, 3, 4];
        let (mut n, mut at) = (0, 0);
        unsafe {
            stream.write(data.as_mut_ptr().cast(), 4, &mut n);
            assert_eq!(n, 4);
            stream.tell(&mut at);
            assert_eq!(at, 4);
            stream.seek(1, 0, &mut at);
            let mut out = [0u8; 8];
            stream.read(out.as_mut_ptr().cast(), 8, &mut n);
            assert_eq!((n, &out[..3]), (3, &[2u8, 3, 4][..]));
            stream.read(out.as_mut_ptr().cast(), 8, &mut n);
            assert_eq!(n, 0, "at the end");
            assert_eq!(stream.seek(-99, 1, &mut at), kResultFalse);
            stream.seek(0, 2, &mut at);
            assert_eq!(at, 4);
        }
    }

    #[test]
    fn state_parts_pack_and_unpack() {
        let packed = pack_state(&[1, 2, 3], &[9]);
        assert_eq!(unpack_state(&packed), Some((&[1u8, 2, 3][..], &[9u8][..])));
        let none = pack_state(&[], &[]);
        assert_eq!(unpack_state(&none), Some((&[][..], &[][..])));
        assert_eq!(unpack_state(b"nonsense"), None);
        assert_eq!(unpack_state(&packed[..7]), None, "cut short");
    }

    #[test]
    fn notes_and_controllers_become_events_and_parameters_without_allocating() {
        let params = ParamChanges::new();
        let events = EventList::new();
        assert_no_alloc::assert_no_alloc(|| {
            events.push(note_on(5, 60, 0.5));
            events.push(note_off(9, 60));
            params.add(7, 0, 0.25);
            params.add(7, 10, 0.75);
            params.add(8, 3, 1.0);
        });
        let list = events.events.borrow();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].sampleOffset, 5);
        unsafe {
            assert_eq!(list[0].__field0.noteOn.pitch, 60);
            assert_eq!(list[0].__field0.noteOn.velocity, 0.5);
            assert_eq!(list[1].__field0.noteOff.noteId, -1);
            assert_eq!(params.getParameterCount(), 2);
            let queue = params.queues[0].as_com_ref::<IParamValueQueue>().unwrap();
            assert_eq!(queue.getParameterId(), 7);
            assert_eq!(queue.getPointCount(), 2);
            let (mut frame, mut value) = (0, 0.0);
            queue.getPoint(1, &mut frame, &mut value);
            assert_eq!((frame, value), (10, 0.75));
        }
        drop(list);
        params.clear();
        assert_eq!(unsafe { params.getParameterCount() }, 0);
        for id in 0..MAX_PARAM_QUEUES as u32 + 5 {
            params.add(id, 0, 0.0);
        }
        assert_eq!(
            unsafe { params.getParameterCount() },
            MAX_PARAM_QUEUES as i32,
            "dropped past the room made"
        );
    }

    #[test]
    fn the_song_position_goes_over_in_vst3_terms() {
        let context = process_context(
            &PluginTransport {
                playing: true,
                beat: 5.5,
                seconds: 2.75,
                tempo: 120.0,
                signature: (6, 8),
                bar_start: 3.0,
                bar_number: 1,
                cycle: Some((4.0, 8.0)),
            },
            48_000.0,
            640,
        );
        assert_ne!(context.state & Flags::kPlaying as u32, 0);
        assert_ne!(context.state & Flags::kCycleActive as u32, 0);
        assert_eq!(context.projectTimeMusic, 5.5);
        assert_eq!(context.projectTimeSamples, 132_000);
        assert_eq!(
            (context.timeSigNumerator, context.timeSigDenominator),
            (6, 8)
        );
        assert_eq!(context.barPositionMusic, 3.0);
        assert_eq!(context.cycleEndMusic, 8.0);
        assert_eq!(context.continousTimeSamples, 640);
    }

    /// Real plugins, which only some machines have, on the screen: an effect and an instrument are
    /// enough. `cargo test -p prowave-host installed_vst3 -- --ignored --nocapture --test-threads=1`,
    /// with `PROWAVE_TEST_VST3` naming files separated by `;`.
    #[test]
    #[ignore = "loads VST3 plugins installed on this computer and opens their editors"]
    fn installed_vst3_plugins_process_save_and_show_their_editors() {
        let chosen = std::env::var("PROWAVE_TEST_VST3").unwrap_or_else(|_| {
            let folder = std::env::var("COMMONPROGRAMFILES").unwrap();
            format!(
                "{folder}\\VST3\\AAS\\Lounge Lizard EP-5.vst3;\
                 {folder}\\VST3\\accusonus\\ERA6_DeEsserPro.vst3"
            )
        });
        for file in chosen.split(';') {
            let file = PathBuf::from(file);
            for info in scan_vst3_file(&file).unwrap() {
                println!(
                    "{} / {} ({}) instrument {}",
                    info.vendor, info.name, info.id, info.is_instrument
                );
                let (mut loaded, mut audio) = load_vst3(&info.path, &info.id, 48_000).unwrap();
                println!("  latency {}", loaded.latency_samples());
                let (mut l, mut r) = (vec![9.0; PLUGIN_BLOCK], vec![9.0; PLUGIN_BLOCK]);
                let mut peak = 0f32;
                let blocks = (loaded.latency_samples() + 48_000) / PLUGIN_BLOCK;
                for b in 0..blocks {
                    let tone: Vec<f32> = (0..PLUGIN_BLOCK)
                        .map(|i| {
                            if info.is_instrument {
                                return 0.0;
                            }
                            let t = (b * PLUGIN_BLOCK + i) as f32 / 48_000.0;
                            (t * 220.0 * std::f32::consts::TAU).sin() * 0.25
                        })
                        .collect();
                    let events: Vec<PluginEvent> = if !info.is_instrument {
                        Vec::new()
                    } else if b == 40 {
                        vec![PluginEvent {
                            frame: 3,
                            kind: PluginEventKind::NoteOn {
                                key: 60,
                                velocity: 0.8,
                            },
                        }]
                    } else if b == blocks - 40 {
                        vec![PluginEvent {
                            frame: 0,
                            kind: PluginEventKind::NoteOff { key: 60 },
                        }]
                    } else {
                        Vec::new()
                    };
                    audio.process(
                        [&tone, &tone],
                        [&mut l, &mut r],
                        &events,
                        &PluginTransport::default(),
                    );
                    assert!(l.iter().chain(&r).all(|x| x.is_finite()), "{}", info.name);
                    peak = l.iter().chain(&r).fold(peak, |m, x| m.max(x.abs()));
                }
                println!("  peak {peak:.3}");
                assert!(peak > 0.001, "{} made a sound", info.name);

                let state = loaded.save_state().unwrap().unwrap();
                println!("  state {} bytes", state.len());
                let (mut other, other_audio) = load_vst3(&info.path, &info.id, 48_000).unwrap();
                other.load_state(&state).unwrap();
                assert!(!other.take_dirty(), "loading is not an edit");
                drop(other_audio);
                drop(other);

                loaded
                    .open_editor(&info.name, None, Some((60, 60)))
                    .unwrap();
                pump_messages_for(Duration::from_millis(700));
                loaded.tick();
                assert!(loaded.editor_is_open());
                println!("  editor open");
                loaded.close_editor();
                pump_messages_for(Duration::from_millis(100));
                assert!(!loaded.editor_is_open());
                assert!(loaded.take_closed_position().is_some());
                assert!(!loaded.released());
                drop(audio);
                assert!(loaded.released());
                drop(loaded);
            }
        }
    }
}
