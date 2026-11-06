//! ARA 2: an audio plugin such as a pitch editor reads a track's audio files itself and plays the
//! track's regions, in place of the sound fed to it.
//!
//! Each bound plugin has a document of its own, holding the audio of the track it sits on. A plugin
//! then only knows its own track, and what it saves goes with it in the project.
//!
//! The rules followed here are those of Celemony's `ARAInterface.h` and `ARAVST3.h`. Everything in
//! this file runs on the UI thread, except the audio reader's callbacks, which the plugin calls
//! from threads of its own: those only touch the reader they are given.

#[allow(
    non_upper_case_globals,
    non_camel_case_types,
    non_snake_case,
    dead_code,
    clippy::all,
    clippy::pedantic
)]
mod bindings;

use crate::AraRegion;
use bindings::*;
use prowave_engine::AudioFile;
use std::collections::HashMap;
use std::ffi::{c_void, CStr, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use vst3::Steinberg::TUID;

/// The newest generation asked for. Every field used here exists in it.
const GENERATION: ARAAPIGeneration = kARAAPIGeneration_2_0_Final as ARAAPIGeneration;
const ROLES: ARAPlugInInstanceRoleFlags =
    (kARAPlaybackRendererRole | kARAEditorRendererRole | kARAEditorViewRole) as i32;
/// A reader decodes this much at a time: each read of a compressed file starts over from a packet.
const READ_AHEAD: usize = 1 << 17;

const IID_ENTRY_POINT: TUID = vst3::uid(0x12814E54, 0xA1CE4076, 0x82B96813, 0x16950BD6);
const IID_ENTRY_POINT_2: TUID = vst3::uid(0xCD9A5913, 0xC9EB46D7, 0x96CA53AD, 0xD1DB89F5);

type QueryInterface = unsafe extern "system" fn(*mut c_void, *const TUID, *mut *mut c_void) -> i32;
type Count = unsafe extern "system" fn(*mut c_void) -> u32;

/// `IPlugInEntryPoint`, as the plugin lays it out.
#[repr(C)]
struct EntryPoint {
    query: QueryInterface,
    add_ref: Count,
    release: Count,
    get_factory: unsafe extern "system" fn(*mut c_void) -> *const ARAFactory,
}

/// `IPlugInEntryPoint2`, as the plugin lays it out.
#[repr(C)]
struct EntryPoint2 {
    query: QueryInterface,
    add_ref: Count,
    release: Count,
    bind: unsafe extern "system" fn(
        *mut c_void,
        ARADocumentControllerRef,
        ARAPlugInInstanceRoleFlags,
        ARAPlugInInstanceRoleFlags,
    ) -> *const ARAPlugInExtensionInstance,
}

/// Asks a COM object for an interface. The result is held, and released by the caller.
///
/// SAFETY: `object` must be a live object that starts with a COM table.
unsafe fn query(object: *mut c_void, iid: &TUID) -> Option<*mut c_void> {
    let table = *object.cast::<*const EntryPoint>();
    let mut found = std::ptr::null_mut();
    let ok = ((*table).query)(object, iid, &mut found) == 0;
    (ok && !found.is_null()).then_some(found)
}

/// SAFETY: as for [`query`], with a reference this side holds.
unsafe fn release(object: *mut c_void) {
    let table = *object.cast::<*const EntryPoint>();
    ((*table).release)(object);
}

/// Factories that have been started, by address. They stay loaded while the program runs.
static STARTED: Mutex<Vec<usize>> = Mutex::new(Vec::new());
/// What the plugin calls on a failed check of its own. Null: this host has none.
static mut ASSERT_FUNCTION: ARAAssertFunction = None;

// --------------------------------------------------------------- what the plugin calls on the host

/// What the plugin tells the host about, reached from any of the callbacks that need it.
struct Notes {
    dirty: Arc<AtomicBool>,
}

/// An audio file in the document.
struct Source {
    file: AudioFile,
    name: CString,
    id: CString,
    modification_id: CString,
    reference: ARAAudioSourceRef,
    modification: ARAAudioModificationRef,
}

/// One of the plugin's readers of an audio file. The plugin uses each from one thread at a time.
struct Reader {
    file: AudioFile,
    wide: bool,
    /// The file's frame where `chunk` starts, and that stretch as interleaved stereo.
    at: i64,
    chunk: Vec<f32>,
}

impl Reader {
    /// Whether `frame` is in the chunk, after reading it if not. False at or past the end of the
    /// file, or if it cannot be read.
    fn load(&mut self, frame: i64, wanted: usize) -> Result<bool, ()> {
        let held = (self.chunk.len() / 2) as i64;
        if frame >= self.at && frame < self.at + held {
            return Ok(true);
        }
        if frame < 0 || frame as u64 >= self.file.frames {
            return Ok(false);
        }
        let chunk = self
            .file
            .read_stereo_at(frame as u64, wanted.max(READ_AHEAD))
            .map_err(|_| ())?;
        self.at = frame;
        self.chunk = chunk;
        Ok(!self.chunk.is_empty())
    }
}

unsafe extern "C" fn create_reader(
    _: ARAAudioAccessControllerHostRef,
    source: ARAAudioSourceHostRef,
    wide: ARABool,
) -> ARAAudioReaderHostRef {
    let source = &*source.cast::<Source>();
    Box::into_raw(Box::new(Reader {
        file: source.file.clone(),
        wide: wide != 0,
        at: 0,
        chunk: Vec::new(),
    }))
    .cast()
}

/// SAFETY: the two buffers must each hold `count` samples of the reader's sample size.
unsafe fn write_channel(
    reader: &Reader,
    buffer: *mut c_void,
    at: usize,
    from: usize,
    n: usize,
    c: usize,
) {
    for k in 0..n {
        let sample = reader.chunk[(from + k) * 2 + c];
        if reader.wide {
            *buffer.cast::<f64>().add(at + k) = f64::from(sample);
        } else {
            *buffer.cast::<f32>().add(at + k) = sample;
        }
    }
}

unsafe extern "C" fn read_samples(
    _: ARAAudioAccessControllerHostRef,
    reader: ARAAudioReaderHostRef,
    position: ARASamplePosition,
    count: ARASampleCount,
    buffers: *const *mut c_void,
) -> ARABool {
    let reader = &mut *reader.cast::<Reader>();
    let total = count.max(0) as usize;
    let mut done = 0usize;
    let mut failed = false;
    while done < total {
        match reader.load(position + done as i64, total - done) {
            Ok(true) => {}
            Ok(false) => break,
            Err(()) => {
                failed = true;
                break;
            }
        }
        let from = (position + done as i64 - reader.at) as usize;
        let n = (reader.chunk.len() / 2 - from).min(total - done);
        for c in 0..2 {
            write_channel(reader, *buffers.add(c), done, from, n, c);
        }
        done += n;
    }
    // What could not be read is silence, as the interface asks.
    for c in 0..2 {
        let buffer = *buffers.add(c);
        for k in done..total {
            if reader.wide {
                *buffer.cast::<f64>().add(k) = 0.0;
            } else {
                *buffer.cast::<f32>().add(k) = 0.0;
            }
        }
    }
    ARABool::from(!failed && done == total)
}

unsafe extern "C" fn destroy_reader(
    _: ARAAudioAccessControllerHostRef,
    reader: ARAAudioReaderHostRef,
) {
    drop(Box::from_raw(reader.cast::<Reader>()));
}

static AUDIO_ACCESS: ARAAudioAccessControllerInterface = ARAAudioAccessControllerInterface {
    structSize: std::mem::size_of::<ARAAudioAccessControllerInterface>(),
    createAudioReaderForSource: Some(create_reader),
    readAudioSamples: Some(read_samples),
    destroyAudioReader: Some(destroy_reader),
};

/// An archive being read or written. Used only inside the store and restore calls.
struct Archive {
    bytes: Vec<u8>,
    id: CString,
}

unsafe extern "C" fn archive_size(
    _: ARAArchivingControllerHostRef,
    archive: ARAArchiveReaderHostRef,
) -> ARASize {
    (*archive.cast::<Archive>()).bytes.len()
}

unsafe extern "C" fn archive_read(
    _: ARAArchivingControllerHostRef,
    archive: ARAArchiveReaderHostRef,
    position: ARASize,
    length: ARASize,
    buffer: *mut ARAByte,
) -> ARABool {
    let archive = &*archive.cast::<Archive>();
    let part = position
        .checked_add(length)
        .and_then(|end| archive.bytes.get(position..end));
    match part {
        Some(part) => {
            std::ptr::copy_nonoverlapping(part.as_ptr(), buffer, part.len());
            1
        }
        None => 0,
    }
}

unsafe extern "C" fn archive_write(
    _: ARAArchivingControllerHostRef,
    archive: ARAArchiveWriterHostRef,
    position: ARASize,
    length: ARASize,
    buffer: *const ARAByte,
) -> ARABool {
    let archive = &mut *archive.cast::<Archive>();
    let Some(end) = position.checked_add(length) else {
        return 0;
    };
    if archive.bytes.len() < end {
        archive.bytes.resize(end, 0);
    }
    std::ptr::copy_nonoverlapping(buffer, archive.bytes[position..end].as_mut_ptr(), length);
    1
}

unsafe extern "C" fn archive_progress(_: ARAArchivingControllerHostRef, _: f32) {}

unsafe extern "C" fn archive_id(
    _: ARAArchivingControllerHostRef,
    archive: ARAArchiveReaderHostRef,
) -> ARAPersistentID {
    (*archive.cast::<Archive>()).id.as_ptr()
}

static ARCHIVING: ARAArchivingControllerInterface = ARAArchivingControllerInterface {
    structSize: std::mem::size_of::<ARAArchivingControllerInterface>(),
    getArchiveSize: Some(archive_size),
    readBytesFromArchive: Some(archive_read),
    writeBytesToArchive: Some(archive_write),
    notifyDocumentArchivingProgress: Some(archive_progress),
    notifyDocumentUnarchivingProgress: Some(archive_progress),
    getDocumentArchiveID: Some(archive_id),
};

// The project's tempo and the like are not offered to the plugin yet, which the interface allows.
unsafe extern "C" fn context_available(
    _: ARAContentAccessControllerHostRef,
    _: ARAMusicalContextHostRef,
    _: ARAContentType,
) -> ARABool {
    0
}

unsafe extern "C" fn context_grade(
    _: ARAContentAccessControllerHostRef,
    _: ARAMusicalContextHostRef,
    _: ARAContentType,
) -> ARAContentGrade {
    kARAContentGradeInitial as ARAContentGrade
}

unsafe extern "C" fn context_reader(
    _: ARAContentAccessControllerHostRef,
    _: ARAMusicalContextHostRef,
    _: ARAContentType,
    _: *const ARAContentTimeRange,
) -> ARAContentReaderHostRef {
    std::ptr::null_mut()
}

unsafe extern "C" fn source_available(
    _: ARAContentAccessControllerHostRef,
    _: ARAAudioSourceHostRef,
    _: ARAContentType,
) -> ARABool {
    0
}

unsafe extern "C" fn source_grade(
    _: ARAContentAccessControllerHostRef,
    _: ARAAudioSourceHostRef,
    _: ARAContentType,
) -> ARAContentGrade {
    kARAContentGradeInitial as ARAContentGrade
}

unsafe extern "C" fn source_reader(
    _: ARAContentAccessControllerHostRef,
    _: ARAAudioSourceHostRef,
    _: ARAContentType,
    _: *const ARAContentTimeRange,
) -> ARAContentReaderHostRef {
    std::ptr::null_mut()
}

unsafe extern "C" fn reader_count(
    _: ARAContentAccessControllerHostRef,
    _: ARAContentReaderHostRef,
) -> ARAInt32 {
    0
}

unsafe extern "C" fn reader_event(
    _: ARAContentAccessControllerHostRef,
    _: ARAContentReaderHostRef,
    _: ARAInt32,
) -> *const c_void {
    std::ptr::null()
}

unsafe extern "C" fn reader_destroy(
    _: ARAContentAccessControllerHostRef,
    _: ARAContentReaderHostRef,
) {
}

static CONTENT_ACCESS: ARAContentAccessControllerInterface = ARAContentAccessControllerInterface {
    structSize: std::mem::size_of::<ARAContentAccessControllerInterface>(),
    isMusicalContextContentAvailable: Some(context_available),
    getMusicalContextContentGrade: Some(context_grade),
    createMusicalContextContentReader: Some(context_reader),
    isAudioSourceContentAvailable: Some(source_available),
    getAudioSourceContentGrade: Some(source_grade),
    createAudioSourceContentReader: Some(source_reader),
    getContentReaderEventCount: Some(reader_count),
    getContentReaderDataForEvent: Some(reader_event),
    destroyContentReader: Some(reader_destroy),
};

unsafe extern "C" fn analysis_progress(
    _: ARAModelUpdateControllerHostRef,
    _: ARAAudioSourceHostRef,
    _: ARAAnalysisProgressState,
    _: f32,
) {
}

unsafe extern "C" fn source_changed(
    _: ARAModelUpdateControllerHostRef,
    _: ARAAudioSourceHostRef,
    _: *const ARAContentTimeRange,
    _: ARAContentUpdateFlags,
) {
}

unsafe extern "C" fn modification_changed(
    host: ARAModelUpdateControllerHostRef,
    _: ARAAudioModificationHostRef,
    _: *const ARAContentTimeRange,
    _: ARAContentUpdateFlags,
) {
    (*host.cast::<Notes>()).dirty.store(true, Ordering::Release);
}

unsafe extern "C" fn region_changed(
    host: ARAModelUpdateControllerHostRef,
    _: ARAPlaybackRegionHostRef,
    _: *const ARAContentTimeRange,
    _: ARAContentUpdateFlags,
) {
    (*host.cast::<Notes>()).dirty.store(true, Ordering::Release);
}

unsafe extern "C" fn document_changed(host: ARAModelUpdateControllerHostRef) {
    (*host.cast::<Notes>()).dirty.store(true, Ordering::Release);
}

static MODEL_UPDATES: ARAModelUpdateControllerInterface = ARAModelUpdateControllerInterface {
    structSize: std::mem::size_of::<ARAModelUpdateControllerInterface>(),
    notifyAudioSourceAnalysisProgress: Some(analysis_progress),
    notifyAudioSourceContentChanged: Some(source_changed),
    notifyAudioModificationContentChanged: Some(modification_changed),
    notifyPlaybackRegionContentChanged: Some(region_changed),
    notifyDocumentDataChanged: Some(document_changed),
};

// A plugin's own transport requests are not followed yet. They are requests: ignoring one is allowed.
unsafe extern "C" fn no_request(_: ARAPlaybackControllerHostRef) {}
unsafe extern "C" fn no_position(_: ARAPlaybackControllerHostRef, _: ARATimePosition) {}
unsafe extern "C" fn no_cycle(
    _: ARAPlaybackControllerHostRef,
    _: ARATimePosition,
    _: ARATimeDuration,
) {
}
unsafe extern "C" fn no_enable(_: ARAPlaybackControllerHostRef, _: ARABool) {}

static PLAYBACK: ARAPlaybackControllerInterface = ARAPlaybackControllerInterface {
    structSize: std::mem::size_of::<ARAPlaybackControllerInterface>(),
    requestStartPlayback: Some(no_request),
    requestStopPlayback: Some(no_request),
    requestSetPlaybackPosition: Some(no_position),
    requestSetCycleRange: Some(no_cycle),
    requestEnableCycle: Some(no_enable),
};

// -------------------------------------------------------------------------------- the document

/// A region as last told to the plugin.
struct Placed {
    reference: ARAPlaybackRegionRef,
    times: (f64, f64, f64),
}

/// A plugin's ARA document and its side of the binding.
pub struct AraDocument {
    factory: *const ARAFactory,
    instance: *const ARADocumentControllerInstance,
    extension: *const ARAPlugInExtensionInstance,
    context: ARAMusicalContextRef,
    sequence: ARARegionSequenceRef,
    sources: HashMap<String, Box<Source>>,
    regions: HashMap<(String, usize), Placed>,
    /// A saved archive, waiting for the audio it is about to be in the document.
    pending: Option<Archive>,
    _host: Box<ARADocumentControllerHostInstance>,
    _notes: Box<Notes>,
}

/// Calls a function of the document controller, if the plugin has it.
macro_rules! controller {
    ($doc:expr, $call:ident $(, $arg:expr)*) => {{
        let instance = &*$doc.instance;
        (*instance.documentControllerInterface)
            .$call
            .map(|call| call(instance.documentControllerRef $(, $arg)*))
    }};
}

/// A stable name for a file, whichever folder the project has moved to.
fn source_key(file: &AudioFile) -> String {
    let name = file
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in name.bytes().chain(file.frames.to_le_bytes()) {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    format!("prowave-{hash:016x}-{}", file.sample_rate)
}

fn text(value: &str) -> CString {
    CString::new(value.replace('\0', " ")).unwrap_or_default()
}

/// The key of each wanted region: its file, and which of that file's regions it is.
fn keys_of(wanted: &[AraRegion]) -> Vec<(String, usize)> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    wanted
        .iter()
        .map(|region| {
            let key = source_key(&region.file);
            let count = seen.entry(key.clone()).or_default();
            *count += 1;
            (key, *count - 1)
        })
        .collect()
}

impl AraDocument {
    /// Binds a plugin that offers ARA to a new document. `None` for any other plugin, which then
    /// goes on as it would have. Call before anything has set up or activated the plugin.
    ///
    /// SAFETY: `component` must be a live VST3 component that was only initialized so far.
    pub unsafe fn bind(component: *mut c_void, name: &str, dirty: Arc<AtomicBool>) -> Option<Self> {
        let first = query(component, &IID_ENTRY_POINT)?;
        let second = query(component, &IID_ENTRY_POINT_2);
        let table = *first.cast::<*const EntryPoint>();
        let factory = ((*table).get_factory)(first);
        release(first);
        let second = second?;
        let bound = Self::bind_to(second, factory, name, dirty);
        release(second);
        bound
    }

    /// SAFETY: as for [`Self::bind`], with `entry` its `IPlugInEntryPoint2`.
    unsafe fn bind_to(
        entry: *mut c_void,
        factory: *const ARAFactory,
        name: &str,
        dirty: Arc<AtomicBool>,
    ) -> Option<Self> {
        if factory.is_null()
            || (*factory).lowestSupportedApiGeneration > GENERATION
            || (*factory).highestSupportedApiGeneration < GENERATION
        {
            return None;
        }
        {
            let mut started = STARTED.lock().ok()?;
            if !started.contains(&(factory as usize)) {
                let config = ARAInterfaceConfiguration {
                    structSize: std::mem::size_of::<ARAInterfaceConfiguration>(),
                    desiredApiGeneration: GENERATION,
                    assertFunctionAddress: std::ptr::addr_of_mut!(ASSERT_FUNCTION),
                };
                (*factory).initializeARAWithConfiguration?(&config);
                started.push(factory as usize);
            }
        }
        let mut notes = Box::new(Notes { dirty });
        let notes_ref: *mut Notes = &mut *notes;
        let mut host = Box::new(ARADocumentControllerHostInstance {
            structSize: std::mem::size_of::<ARADocumentControllerHostInstance>(),
            audioAccessControllerHostRef: notes_ref.cast(),
            audioAccessControllerInterface: &AUDIO_ACCESS,
            archivingControllerHostRef: notes_ref.cast(),
            archivingControllerInterface: &ARCHIVING,
            contentAccessControllerHostRef: notes_ref.cast(),
            contentAccessControllerInterface: &CONTENT_ACCESS,
            modelUpdateControllerHostRef: notes_ref.cast(),
            modelUpdateControllerInterface: &MODEL_UPDATES,
            playbackControllerHostRef: notes_ref.cast(),
            playbackControllerInterface: &PLAYBACK,
        });
        let title = text(name);
        let properties = ARADocumentProperties {
            structSize: std::mem::size_of::<ARADocumentProperties>(),
            name: title.as_ptr(),
        };
        let instance = (*factory).createDocumentControllerWithDocument?(&mut *host, &properties);
        if instance.is_null() {
            return None;
        }
        let mut document = Self {
            factory,
            instance,
            extension: std::ptr::null(),
            context: std::ptr::null_mut(),
            sequence: std::ptr::null_mut(),
            sources: HashMap::new(),
            regions: HashMap::new(),
            pending: None,
            _host: host,
            _notes: notes,
        };
        controller!(document, beginEditing);
        let context_properties = ARAMusicalContextProperties {
            structSize: std::mem::size_of::<ARAMusicalContextProperties>(),
            name: title.as_ptr(),
            orderIndex: 0,
            color: std::ptr::null(),
        };
        // Host references are only handed back to the host, which has no use for these two.
        let notes_host: *mut Notes = notes_ref;
        document.context = controller!(
            document,
            createMusicalContext,
            notes_host.cast(),
            &context_properties
        )
        .unwrap_or(std::ptr::null_mut());
        let sequence_properties = ARARegionSequenceProperties {
            structSize: std::mem::size_of::<ARARegionSequenceProperties>(),
            name: title.as_ptr(),
            orderIndex: 0,
            musicalContextRef: document.context,
            color: std::ptr::null(),
        };
        document.sequence = controller!(
            document,
            createRegionSequence,
            notes_host.cast(),
            &sequence_properties
        )
        .unwrap_or(std::ptr::null_mut());
        controller!(document, endEditing);
        let table = *entry.cast::<*const EntryPoint2>();
        let reference = (*instance).documentControllerRef;
        document.extension = ((*table).bind)(entry, reference, ROLES, ROLES);
        if document.extension.is_null() {
            document.destroy();
            return None;
        }
        Some(document)
    }

    /// Whether going to `wanted` adds or takes away regions, which the plugin only allows while it
    /// is not active. Moving a region does not.
    pub fn changes_assignment(&self, wanted: &[AraRegion]) -> bool {
        let keys = keys_of(wanted);
        keys.len() != self.regions.len() || keys.iter().any(|k| !self.regions.contains_key(k))
    }

    /// Brings the document to `wanted`. A region taken away keeps its file's edits in the document,
    /// so undoing the deletion finds them again.
    ///
    /// SAFETY: if [`Self::changes_assignment`] says so, the plugin must not be active.
    pub unsafe fn apply(&mut self, wanted: &[AraRegion]) -> Result<(), String> {
        let keys = keys_of(wanted);
        let mut new_sources = Vec::new();
        controller!(self, beginEditing);
        for region in wanted {
            let key = source_key(&region.file);
            if !self.sources.contains_key(&key) {
                self.add_source(&key, region);
                new_sources.push(key);
            }
        }
        let mut problem = None;
        if !self.sources.is_empty() {
            if let Some(mut archive) = self.pending.take() {
                let reference: *mut Archive = &mut archive;
                let restored = controller!(
                    self,
                    restoreObjectsFromArchive,
                    reference.cast(),
                    std::ptr::null()
                );
                if restored != Some(1) {
                    problem = Some("the plugin could not read its saved edits".to_string());
                }
            }
        }
        // What is gone.
        let gone: Vec<(String, usize)> = self
            .regions
            .keys()
            .filter(|k| !keys.contains(k))
            .cloned()
            .collect();
        for key in gone {
            if let Some(placed) = self.regions.remove(&key) {
                self.unassign(placed.reference);
                controller!(self, destroyPlaybackRegion, placed.reference);
            }
        }
        for (region, key) in wanted.iter().zip(keys) {
            let times = (region.start, region.offset, region.duration);
            let title = text(&region.name);
            let properties = ARAPlaybackRegionProperties {
                structSize: std::mem::size_of::<ARAPlaybackRegionProperties>(),
                transformationFlags: kARAPlaybackTransformationNoChanges as _,
                startInModificationTime: region.offset,
                durationInModificationTime: region.duration,
                startInPlaybackTime: region.start,
                durationInPlaybackTime: region.duration,
                musicalContextRef: self.context,
                regionSequenceRef: self.sequence,
                name: title.as_ptr(),
                color: std::ptr::null(),
            };
            match self.regions.get_mut(&key) {
                Some(placed) => {
                    if placed.times != times {
                        placed.times = times;
                        let reference = placed.reference;
                        controller!(self, updatePlaybackRegionProperties, reference, &properties);
                    }
                }
                None => {
                    let Some(source) = self.sources.get(&key.0) else {
                        continue;
                    };
                    let modification = source.modification;
                    let host: *const Source = &**source;
                    let made = controller!(
                        self,
                        createPlaybackRegion,
                        modification,
                        host as ARAPlaybackRegionHostRef,
                        &properties
                    );
                    if let Some(reference) = made.filter(|r| !r.is_null()) {
                        self.assign(reference);
                        self.regions.insert(key, Placed { reference, times });
                    }
                }
            }
        }
        controller!(self, endEditing);
        // Read only now: the plugin may leave out what a restored archive already holds.
        for key in &new_sources {
            if let Some(source) = self.sources.get(key) {
                let reference = source.reference;
                controller!(self, enableAudioSourceSamplesAccess, reference, 1);
            }
        }
        self.show_everything();
        controller!(self, notifyModelUpdates);
        problem.map_or(Ok(()), Err)
    }

    /// SAFETY: inside an editing cycle.
    unsafe fn add_source(&mut self, key: &str, region: &AraRegion) {
        let name = region
            .file
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut source = Box::new(Source {
            file: region.file.clone(),
            name: text(&name),
            id: text(key),
            modification_id: text(&format!("{key}:0")),
            reference: std::ptr::null_mut(),
            modification: std::ptr::null_mut(),
        });
        let host: *mut Source = &mut *source;
        let properties = ARAAudioSourceProperties {
            structSize: std::mem::size_of::<ARAAudioSourceProperties>(),
            name: source.name.as_ptr(),
            persistentID: source.id.as_ptr(),
            sampleCount: source.file.frames as ARASampleCount,
            sampleRate: f64::from(source.file.sample_rate),
            // Readers always hand out two channels: a mono file reads as the same on both.
            channelCount: 2,
            merits64BitSamples: 0,
            channelArrangementDataType: kARAChannelArrangementUndefined as _,
            channelArrangement: std::ptr::null(),
        };
        let reference = controller!(self, createAudioSource, host.cast(), &properties);
        let Some(reference) = reference.filter(|r| !r.is_null()) else {
            return;
        };
        source.reference = reference;
        let properties = ARAAudioModificationProperties {
            structSize: std::mem::size_of::<ARAAudioModificationProperties>(),
            name: source.name.as_ptr(),
            persistentID: source.modification_id.as_ptr(),
        };
        source.modification = controller!(
            self,
            createAudioModification,
            reference,
            host.cast(),
            &properties
        )
        .unwrap_or(std::ptr::null_mut());
        self.sources.insert(key.to_string(), source);
    }

    /// SAFETY: `reference` is a live region of this document.
    unsafe fn assign(&self, reference: ARAPlaybackRegionRef) {
        let e = &*self.extension;
        if !e.playbackRendererInterface.is_null() {
            if let Some(add) = (*e.playbackRendererInterface).addPlaybackRegion {
                add(e.playbackRendererRef, reference);
            }
        }
        if !e.editorRendererInterface.is_null() {
            if let Some(add) = (*e.editorRendererInterface).addPlaybackRegion {
                add(e.editorRendererRef, reference);
            }
        }
    }

    /// SAFETY: `reference` is a live region of this document that was assigned.
    unsafe fn unassign(&self, reference: ARAPlaybackRegionRef) {
        let e = &*self.extension;
        if !e.playbackRendererInterface.is_null() {
            if let Some(remove) = (*e.playbackRendererInterface).removePlaybackRegion {
                remove(e.playbackRendererRef, reference);
            }
        }
        if !e.editorRendererInterface.is_null() {
            if let Some(remove) = (*e.editorRendererInterface).removePlaybackRegion {
                remove(e.editorRendererRef, reference);
            }
        }
    }

    /// Tells the plugin's window to show all of the track's regions.
    ///
    /// SAFETY: outside an editing cycle.
    unsafe fn show_everything(&self) {
        let e = &*self.extension;
        if e.editorViewInterface.is_null() || self.regions.is_empty() {
            return;
        }
        let Some(notify) = (*e.editorViewInterface).notifySelection else {
            return;
        };
        let regions: Vec<ARAPlaybackRegionRef> =
            self.regions.values().map(|p| p.reference).collect();
        let selection = ARAViewSelection {
            structSize: std::mem::size_of::<ARAViewSelection>(),
            playbackRegionRefsCount: regions.len(),
            playbackRegionRefs: regions.as_ptr(),
            regionSequenceRefsCount: 1,
            regionSequenceRefs: &self.sequence,
            timeRange: std::ptr::null(),
        };
        notify(e.editorViewRef, &selection);
    }

    /// Lets the plugin send its updates. Call regularly from the UI thread.
    pub fn tick(&mut self) {
        // SAFETY: the document lives as long as `self`.
        unsafe {
            controller!(self, notifyModelUpdates);
        }
    }

    /// The document's saved form: the archive's identifier, and the archive.
    pub fn store(&mut self) -> Option<(String, Vec<u8>)> {
        if let Some(pending) = &self.pending {
            return Some((
                pending.id.to_string_lossy().into_owned(),
                pending.bytes.clone(),
            ));
        }
        // SAFETY: the archive outlives the call, and the factory's identifier lives as it does.
        unsafe {
            let id = CStr::from_ptr((*self.factory).documentArchiveID).to_owned();
            let mut archive = Archive {
                bytes: Vec::new(),
                id,
            };
            let reference: *mut Archive = &mut archive;
            let stored = controller!(
                self,
                storeObjectsToArchive,
                reference.cast(),
                std::ptr::null()
            );
            (stored == Some(1) && !archive.bytes.is_empty())
                .then(|| (archive.id.to_string_lossy().into_owned(), archive.bytes))
        }
    }

    /// Keeps a saved archive for when the audio it is about is in the document. Refused if this
    /// version of the plugin cannot read it.
    pub fn restore_later(&mut self, id: &str, bytes: Vec<u8>) -> bool {
        // SAFETY: the factory's fields live as long as its plugin file is loaded.
        let readable = unsafe {
            let factory = &*self.factory;
            let same = |other: ARAPersistentID| {
                !other.is_null() && CStr::from_ptr(other).to_bytes() == id.as_bytes()
            };
            same(factory.documentArchiveID)
                || (0..factory.compatibleDocumentArchiveIDsCount)
                    .any(|k| same(*factory.compatibleDocumentArchiveIDs.add(k)))
        };
        if readable {
            self.pending = Some(Archive {
                bytes,
                id: text(id),
            });
        }
        readable
    }

    /// Takes the document down, once the plugin has been stopped.
    ///
    /// SAFETY: nothing may use the document after this.
    pub unsafe fn destroy(&mut self) {
        for source in self.sources.values() {
            if !source.reference.is_null() {
                controller!(self, enableAudioSourceSamplesAccess, source.reference, 0);
            }
        }
        controller!(self, beginEditing);
        for placed in self.regions.values() {
            controller!(self, destroyPlaybackRegion, placed.reference);
        }
        self.regions.clear();
        for source in self.sources.values() {
            if !source.modification.is_null() {
                controller!(self, destroyAudioModification, source.modification);
            }
            if !source.reference.is_null() {
                controller!(self, destroyAudioSource, source.reference);
            }
        }
        if !self.sequence.is_null() {
            controller!(self, destroyRegionSequence, self.sequence);
        }
        if !self.context.is_null() {
            controller!(self, destroyMusicalContext, self.context);
        }
        controller!(self, endEditing);
        controller!(self, destroyDocumentController);
        self.sources.clear();
    }
}
