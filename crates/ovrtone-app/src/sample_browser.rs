//! The Browser tab of the left panel (FL Studio: the Browser; Logic Pro: the Media Browser).
//!
//! Your sample locations as one expandable tree, with search, favorites and a preview: click a
//! folder to open or close it, click a sample to hear it, double-click it (or drag it onto the
//! Tracks area) to put it in the project. Folders are read on background threads the first time
//! they are opened, and search looks through every location on another, so a big library or a
//! removed drive never stalls the interface. The open tree is flattened into a list of rows and
//! only the visible ones are drawn. The preview is decoded off the audio thread and played by the
//! engine (`Command::SetPreview`), independently of the transport.
use crate::widgets::{text_col, text_dim_col};
use crate::UiState;
use egui::{pos2, vec2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, Ui, UiBuilder};
use ovrtone_engine::{AudioFile, Command, Engine, Preview};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

const ROW_H: f32 = 20.0;
const INDENT: f32 = 12.0;
fn panel_bg_col() -> Color32 {
    crate::theme::panel_bg()
}
fn sel_bg_col() -> Color32 {
    crate::theme::selected()
}
fn hover_bg_col() -> Color32 {
    crate::theme::hovered()
}
const GREEN: Color32 = crate::theme::GOOD;
const GOLD: Color32 = crate::theme::SOLO;
/// A folder with more entries than this shows only these.
const MAX_ENTRIES: usize = 5000;
/// A search stops at this many hits.
const MAX_HITS: usize = 3000;
/// A search goes this many folders deep.
const MAX_DEPTH: usize = 10;
/// A preview plays at most this much of a file.
const PREVIEW_SECONDS: f64 = 60.0;
const PREVIEW_GAIN: f32 = 0.8;
/// The panel under the list with the picked sample's waveform.
const WAVE_H: f32 = 84.0;
/// Columns of waveform read for it.
const WAVE_BINS: usize = 600;
const ROOTS_FILE: &str = "browser-roots";
/// The single folder earlier versions saved; read once to fill `ROOTS_FILE`.
const OLD_ROOT_FILE: &str = "browser-root";
const EXPANDED_FILE: &str = "browser-expanded";
const FAVORITES_FILE: &str = "browser-favorites";

/// What a drag from the Browser carries: the file to drop on the Tracks area.
#[derive(Clone, Debug)]
pub struct SampleDrag(pub PathBuf);

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    path: PathBuf,
    name: String,
    folder: bool,
}

/// The name a path shows with; a drive like `D:\` has no file name, so it shows whole.
pub(crate) fn display_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

impl Entry {
    fn folder(path: &Path) -> Self {
        Self {
            name: display_name(path),
            path: path.to_path_buf(),
            folder: true,
        }
    }
}

// ------------------------------------------------------------------------- the tree

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowKind {
    Folder {
        open: bool,
    },
    Sample,
    /// A line of text under a folder: still loading, unreadable, empty or cut short.
    Note,
}

/// One line of the flattened tree.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Row {
    path: PathBuf,
    name: String,
    depth: usize,
    kind: RowKind,
    /// A sample location: it has a remove button instead of a favorite star.
    location: bool,
}

/// What the tree knows: the locations, which folders are open and what has been read of them.
/// `None` is a folder that could not be read (no such folder, removed drive, no permission).
#[derive(Default)]
struct Tree {
    roots: Vec<PathBuf>,
    expanded: HashSet<PathBuf>,
    listings: HashMap<PathBuf, Option<Vec<Entry>>>,
}

impl Tree {
    /// Adds a location, open. False when it is already there.
    fn add_root(&mut self, path: PathBuf) -> bool {
        if self.roots.contains(&path) {
            return false;
        }
        self.expanded.insert(path.clone());
        self.roots.push(path);
        true
    }

    /// Removes a location and forgets its folders, unless another location holds them too.
    /// Nothing on disk is touched. False when it is not a location.
    fn remove_root(&mut self, path: &Path) -> bool {
        let Some(i) = self.roots.iter().position(|r| r == path) else {
            return false;
        };
        self.roots.remove(i);
        let roots = &self.roots;
        let kept = |p: &Path| roots.iter().any(|r| p.starts_with(r));
        self.expanded.retain(|p| kept(p));
        self.listings.retain(|p, _| kept(p));
        true
    }

    /// Opens or closes a folder. What is below it stays read and stays open for next time.
    fn toggle(&mut self, path: &Path) {
        if !self.expanded.remove(path) {
            self.expanded.insert(path.to_path_buf());
        }
    }

    /// Opens every folder from the location down to `path`, and `path` itself.
    fn reveal(&mut self, path: &Path) {
        let Some(root) = self.roots.iter().find(|r| path.starts_with(r)).cloned() else {
            return;
        };
        for folder in path.ancestors() {
            self.expanded.insert(folder.to_path_buf());
            if folder == root {
                break;
            }
        }
    }

    fn root_entries(&self) -> Vec<Entry> {
        self.roots.iter().map(|r| Entry::folder(r)).collect()
    }

    /// The rows of `tops` and of everything open below them, and the open folders that have not
    /// been read yet.
    fn flatten(&self, tops: &[Entry], location: bool) -> (Vec<Row>, Vec<PathBuf>) {
        let mut rows = Vec::new();
        let mut need = Vec::new();
        for e in tops {
            self.add_rows(e, 0, location, &mut rows, &mut need);
        }
        (rows, need)
    }

    fn add_rows(
        &self,
        e: &Entry,
        depth: usize,
        location: bool,
        rows: &mut Vec<Row>,
        need: &mut Vec<PathBuf>,
    ) {
        let open = e.folder && self.expanded.contains(&e.path);
        rows.push(Row {
            path: e.path.clone(),
            name: e.name.clone(),
            depth,
            kind: if e.folder {
                RowKind::Folder { open }
            } else {
                RowKind::Sample
            },
            location,
        });
        if !open {
            return;
        }
        let note = |rows: &mut Vec<Row>, text: String| {
            rows.push(Row {
                path: e.path.clone(),
                name: text,
                depth: depth + 1,
                kind: RowKind::Note,
                location: false,
            });
        };
        match self.listings.get(&e.path) {
            None => {
                need.push(e.path.clone());
                note(rows, "Loading…".to_string());
            }
            Some(None) => note(rows, "Cannot read this folder".to_string()),
            Some(Some(list)) if list.is_empty() => note(rows, "No samples here".to_string()),
            Some(Some(list)) => {
                for child in list {
                    self.add_rows(child, depth + 1, false, rows, need);
                }
                if list.len() >= MAX_ENTRIES {
                    note(
                        rows,
                        format!("Showing the first {MAX_ENTRIES}; search to narrow down"),
                    );
                }
            }
        }
    }
}

/// `roots` without those inside another one, so a search does not walk a folder twice.
fn outermost(roots: &[PathBuf]) -> Vec<PathBuf> {
    roots
        .iter()
        .filter(|r| !roots.iter().any(|o| o != *r && r.starts_with(o)))
        .cloned()
        .collect()
}

/// A folder's listing, from a background thread.
type Listed = (PathBuf, Option<Vec<Entry>>);

/// Reads folders on background threads, each once at a time.
#[derive(Default)]
struct Loader {
    channel: Option<(Sender<Listed>, Receiver<Listed>)>,
    in_flight: HashSet<PathBuf>,
}

impl Loader {
    fn request(&mut self, dir: PathBuf, ctx: &egui::Context) {
        if !self.in_flight.insert(dir.clone()) {
            return;
        }
        let (tx, _) = self.channel.get_or_insert_with(channel);
        let (tx, ctx) = (tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let listing = list(&dir);
            let _ = tx.send((dir, listing));
            ctx.request_repaint();
        });
    }

    /// Puts what has been read into the tree; true if anything was.
    fn drain(&mut self, tree: &mut Tree) -> bool {
        let Some((_, rx)) = &self.channel else {
            return false;
        };
        let mut changed = false;
        while let Ok((dir, listing)) = rx.try_recv() {
            self.in_flight.remove(&dir);
            tree.listings.insert(dir, listing);
            changed = true;
        }
        changed
    }
}

pub struct SampleBrowser {
    tree: Tree,
    loader: Loader,
    /// The flattened tree (or favorites), rebuilt only when `rows_dirty` is set.
    rows: Vec<Row>,
    rows_dirty: bool,
    /// A folder to bring into view once its row exists.
    scroll_to: Option<PathBuf>,
    search: String,
    hits: SearchState,
    favorites: Vec<PathBuf>,
    show_favorites: bool,
    selected: Option<PathBuf>,
    /// Single click plays the sample (as in FL Studio's browser).
    auto_preview: bool,
    preview: PreviewState,
    /// The sample shown in the waveform panel, and what has been read of it.
    wave_for: Option<PathBuf>,
    wave: Option<WaveInfo>,
    wave_rx: Option<Receiver<WaveInfo>>,
    loaded: bool,
    /// The last problem, shown under the list.
    message: Option<String>,
}

impl Default for SampleBrowser {
    fn default() -> Self {
        Self {
            tree: Tree::default(),
            loader: Loader::default(),
            rows: Vec::new(),
            rows_dirty: true,
            scroll_to: None,
            search: String::new(),
            hits: SearchState::default(),
            favorites: Vec::new(),
            show_favorites: false,
            selected: None,
            auto_preview: true,
            wave_for: None,
            wave: None,
            wave_rx: None,
            preview: PreviewState::default(),
            loaded: false,
            message: None,
        }
    }
}

// ------------------------------------------------------------------------- settings files

fn setting_file(name: &str) -> Option<PathBuf> {
    crate::recent_files::config_dir().map(|d| d.join(name))
}

fn read_setting(name: &str) -> Option<String> {
    std::fs::read_to_string(setting_file(name)?).ok()
}

fn write_setting(name: &str, text: &str) {
    // Tests click through the Browser; they must not change the settings of whoever runs them.
    if cfg!(test) {
        return;
    }
    let Some(file) = setting_file(name) else {
        return;
    };
    let saved = file
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&file, text));
    if let Err(e) = saved {
        tracing::warn!("could not save {name}: {e}");
    }
}

fn parse_paths(text: &str) -> Vec<PathBuf> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .collect()
}

fn paths_text<'a>(paths: impl IntoIterator<Item = &'a PathBuf>) -> String {
    paths
        .into_iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The locations from the two settings files, and whether the old single folder was used and
/// needs saving in the new file. Locations that are not there now are kept, since a drive may
/// only be unplugged. The list starts empty when neither file has any.
fn load_roots(roots_file: Option<&str>, old_root_file: Option<&str>) -> (Vec<PathBuf>, bool) {
    match roots_file {
        Some(text) => (parse_paths(text), false),
        None => {
            let roots = old_root_file.map(parse_paths).unwrap_or_default();
            let migrated = !roots.is_empty();
            (roots, migrated)
        }
    }
}

// ------------------------------------------------------------------------- listing and search

fn is_sample(path: &Path) -> bool {
    crate::import::is_audio_path(path) || crate::midi_file::is_midi_path(path)
}

/// The words of a search text, lower case.
fn terms(search: &str) -> Vec<String> {
    search.split_whitespace().map(str::to_lowercase).collect()
}

/// Whether `name` has every one of `terms` in it, in any order.
fn matches_all(name: &str, terms: &[String]) -> bool {
    let name = name.to_lowercase();
    terms.iter().all(|t| name.contains(t.as_str()))
}

/// The folders and samples in `dir`, folders first, each sorted by name, at most `MAX_ENTRIES`.
/// Hidden entries are left out. `None` when the folder cannot be read.
fn list(dir: &Path) -> Option<Vec<Entry>> {
    let read = std::fs::read_dir(dir).ok()?;
    let mut entries: Vec<Entry> = read
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = e.path();
            let kind = e.file_type().ok()?;
            let folder = kind.is_dir() || (kind.is_symlink() && path.is_dir());
            (!name.starts_with('.') && (folder || is_sample(&path))).then_some(Entry {
                path,
                name,
                folder,
            })
        })
        .collect();
    entries.sort_by_key(|e| (!e.folder, e.name.to_lowercase()));
    entries.truncate(MAX_ENTRIES);
    Some(entries)
}

/// One message from a search thread: its generation, what it found, and whether it is done.
type Found = (u64, Vec<Entry>, bool);

#[derive(Default)]
struct SearchState {
    /// The locations and text the hits are for.
    for_query: Option<(Vec<PathBuf>, String)>,
    hits: Vec<Entry>,
    running: bool,
    cancel: Option<Arc<AtomicBool>>,
    rx: Option<Receiver<Found>>,
    generation: u64,
}

impl SearchState {
    fn clear(&mut self) {
        if let Some(c) = self.cancel.take() {
            c.store(true, Ordering::Relaxed);
        }
        self.generation += 1;
        self.for_query = None;
        self.hits.clear();
        self.running = false;
        self.rx = None;
    }

    fn start(&mut self, roots: &[PathBuf], text: &str, ctx: &egui::Context) {
        self.clear();
        self.running = true;
        self.for_query = Some((roots.to_vec(), text.to_string()));
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Some(cancel.clone());
        self.rx = Some(rx);
        let (generation, roots, terms, ctx) =
            (self.generation, outermost(roots), terms(text), ctx.clone());
        std::thread::spawn(move || walk(&roots, &terms, generation, &cancel, &tx, &ctx));
    }

    /// Takes in what the search thread has found so far.
    fn drain(&mut self) {
        let mut changed = false;
        if let Some(rx) = &self.rx {
            while let Ok((generation, found, done)) = rx.try_recv() {
                if generation != self.generation {
                    continue;
                }
                changed |= !found.is_empty();
                self.hits.extend(found);
                if done {
                    self.running = false;
                }
            }
        }
        if changed {
            self.hits
                .sort_by_key(|e| (!e.folder, e.name.to_lowercase()));
        }
    }
}

/// Looks for folders and samples under `roots` whose names have all of `terms`, and sends them
/// in batches. Stops when `cancel` is set, at `MAX_HITS` hits or `MAX_DEPTH` folders deep.
fn walk(
    roots: &[PathBuf],
    terms: &[String],
    generation: u64,
    cancel: &AtomicBool,
    tx: &Sender<Found>,
    ctx: &egui::Context,
) {
    let mut stack: Vec<(PathBuf, usize)> = roots.iter().map(|r| (r.clone(), 0)).collect();
    let mut batch = Vec::new();
    let mut total = 0usize;
    'folders: while let Some((dir, depth)) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in read.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let path = e.path();
            let folder = e.file_type().is_ok_and(|t| t.is_dir());
            if folder && depth < MAX_DEPTH {
                stack.push((path.clone(), depth + 1));
            }
            let found = if folder {
                matches_all(&name, terms)
            } else {
                // The name, the tags, and the tempo and key the name gives ("101 bpm", "F#m").
                let tags = crate::sample_tags::tags_of(&path).join(" ");
                let (bpm, key) = crate::sample_tags::name_info(&name);
                let extra = format!(
                    "{} {} {}",
                    tags,
                    bpm.map_or_else(String::new, |b| format!("{b}bpm")),
                    key.unwrap_or_default()
                );
                matches_all(&format!("{name} {extra}"), terms)
            };
            if (folder || is_sample(&path)) && found {
                batch.push(Entry { path, name, folder });
                total += 1;
                if total >= MAX_HITS {
                    break 'folders;
                }
            }
        }
        if batch.len() >= 100 {
            if tx
                .send((generation, std::mem::take(&mut batch), false))
                .is_err()
            {
                return;
            }
            ctx.request_repaint();
        }
    }
    let _ = tx.send((generation, batch, true));
    ctx.request_repaint();
}

// ------------------------------------------------------------------------- preview

type Decoded = (u64, Result<Vec<f32>, String>);

/// What the waveform panel shows of a sample.
struct WaveInfo {
    peaks: Vec<(f32, f32)>,
    seconds: f64,
    rate: u32,
    bpm: Option<f64>,
}

/// Reads a sample for the waveform panel. Decodes all of it: run off the UI thread.
fn read_wave(path: &Path) -> Option<WaveInfo> {
    let file = AudioFile::open(path).ok()?;
    let peaks = file.peaks(WAVE_BINS).ok()?;
    Some(WaveInfo {
        peaks,
        seconds: file.duration_secs(),
        rate: file.sample_rate,
        bpm: crate::flex_edit::sample_bpm(&file),
    })
}

/// How the panel's text reads.
fn wave_text(info: &WaveInfo) -> String {
    let khz = f64::from(info.rate) / 1000.0;
    let bpm = info
        .bpm
        .map_or_else(String::new, |b| format!("   {b:.0} BPM"));
    format!("{khz:.1} kHz{bpm}   {:.1} s", info.seconds)
}

impl SampleBrowser {
    /// Shows `path` in the waveform panel.
    fn show_wave(&mut self, path: &Path, ctx: &egui::Context) {
        if self.wave_for.as_deref() == Some(path) {
            return;
        }
        self.wave_for = Some(path.to_path_buf());
        self.wave = None;
        let (tx, rx) = channel();
        // An earlier read still running finds nobody to tell.
        self.wave_rx = Some(rx);
        let (path, ctx) = (path.to_path_buf(), ctx.clone());
        let _ = std::thread::Builder::new()
            .name("ovrtone-sample-wave".into())
            .spawn(move || {
                if let Some(info) = read_wave(&path) {
                    let _ = tx.send(info);
                }
                ctx.request_repaint();
            });
    }

    /// Plays the sample or stops it, from a row or from the panel.
    pub(crate) fn toggle_preview(&mut self, path: PathBuf) {
        if self.preview.playing.as_ref() == Some(&path) {
            self.preview.stop = true;
        } else {
            self.preview.want = Some(path);
        }
    }

    /// How far through the sample playback is, 0 to 1, while it plays.
    fn wave_position(&self) -> Option<f32> {
        let info = self.wave.as_ref()?;
        if self.preview.playing != self.wave_for || info.seconds <= 0.0 {
            return None;
        }
        let ends = self.preview.ends_at?;
        let left = ends.saturating_duration_since(Instant::now()).as_secs_f64();
        let played = info.seconds.min(PREVIEW_SECONDS) - left;
        Some(((played / info.seconds) as f32).clamp(0.0, 1.0))
    }
}

/// The panel: the waveform with its details, a click plays or stops it.
fn wave_panel(ui: &mut Ui, b: &mut SampleBrowser) {
    let Some(path) = b.wave_for.clone() else {
        return;
    };
    if let Some(info) = b.wave_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
        b.wave = Some(info);
    }
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), WAVE_H), Sense::click());
    let playing = b.preview.playing.as_ref() == Some(&path);
    let g = ui.painter_at(rect);
    g.rect_filled(rect, 2.0, crate::theme::border());
    let mid = rect.center().y;
    let half = rect.height() * 0.5 - 4.0;
    let position = b.wave_position();
    // The zero line across the whole length, so silence shows as silence, as in an audio region.
    g.line_segment(
        [pos2(rect.left(), mid), pos2(rect.right(), mid)],
        Stroke::new(1.0, crate::theme::accent().gamma_multiply(0.55)),
    );
    match &b.wave {
        Some(info) if !info.peaks.is_empty() => {
            let peak = info
                .peaks
                .iter()
                .fold(0f32, |m, (lo, hi)| m.max(lo.abs()).max(hi.abs()))
                .max(1e-4);
            let columns = rect.width().max(1.0) as usize;
            let bins = info.peaks.len();
            for c in 0..columns {
                let (from, to) = (c * bins / columns, ((c + 1) * bins / columns).min(bins));
                let part = &info.peaks[from.min(bins - 1)..to.max(from + 1).min(bins)];
                let (lo, hi) = part
                    .iter()
                    .fold((0f32, 0f32), |(l, h), (a, b)| (l.min(*a), h.max(*b)));
                let x = rect.left() + c as f32 + 0.5;
                g.line_segment(
                    [
                        pos2(x, mid - hi / peak * half),
                        pos2(x, mid - lo / peak * half + 1.0),
                    ],
                    Stroke::new(1.0, crate::theme::accent()),
                );
            }
            g.text(
                rect.right_bottom() - vec2(5.0, 3.0),
                egui::Align2::RIGHT_BOTTOM,
                wave_text(info),
                FontId::proportional(10.5),
                Color32::WHITE,
            );
        }
        _ => {
            g.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Reading…",
                FontId::proportional(11.0),
                text_dim_col(),
            );
        }
    }
    if let Some(at) = position {
        let x = rect.left() + rect.width() * at;
        g.line_segment(
            [pos2(x, rect.top()), pos2(x, rect.bottom())],
            Stroke::new(1.0, Color32::WHITE),
        );
        ui.ctx().request_repaint_after(Duration::from_millis(33));
    }
    // Play or stop, in the corner.
    let icon = Rect::from_min_size(rect.left_bottom() + vec2(5.0, -17.0), vec2(12.0, 12.0));
    if playing {
        g.rect_filled(icon.shrink(1.0), 1.0, Color32::WHITE);
    } else {
        g.add(egui::Shape::convex_polygon(
            vec![
                icon.left_top(),
                pos2(icon.right(), icon.center().y),
                icon.left_bottom(),
            ],
            Color32::WHITE,
            Stroke::NONE,
        ));
    }
    let hint = if playing {
        "Click to stop (Esc)"
    } else {
        "Click to play"
    };
    if resp.on_hover_text(hint).clicked() {
        b.toggle_preview(path);
    }
}

#[derive(Default)]
struct PreviewState {
    /// Counts previews, so a slow decode of an earlier click is ignored.
    id: u64,
    /// A sample to start playing (set by a click, taken by `pump`).
    want: Option<PathBuf>,
    /// Stop what is playing (taken by `pump`).
    stop: bool,
    /// The sample being decoded or played, for the green marker in the list.
    playing: Option<PathBuf>,
    /// When the sample that was handed to the engine has played out.
    ends_at: Option<Instant>,
    tx: Option<Sender<Decoded>>,
    rx: Option<Receiver<Decoded>>,
}

impl PreviewState {
    fn sender(&mut self) -> Sender<Decoded> {
        if let Some(tx) = &self.tx {
            return tx.clone();
        }
        let (tx, rx) = channel();
        self.tx = Some(tx.clone());
        self.rx = Some(rx);
        tx
    }
}

/// `stereo` (interleaved, at `src_rate`) as interleaved stereo at `dst_rate`, cut to
/// `max_seconds`, by linear interpolation.
fn resample(stereo: &[f32], src_rate: u32, dst_rate: u32, max_seconds: f64) -> Vec<f32> {
    let src = f64::from(src_rate.max(1));
    let dst = f64::from(dst_rate.max(1));
    let frames = (stereo.len() / 2).min((max_seconds * src) as usize);
    if frames == 0 {
        return Vec::new();
    }
    if src_rate == dst_rate {
        return stereo[..frames * 2].to_vec();
    }
    let step = src / dst;
    let out_frames = (frames as f64 / step).floor() as usize;
    let mut out = Vec::with_capacity(out_frames * 2);
    for i in 0..out_frames {
        let pos = i as f64 * step;
        let k = (pos.floor() as usize).min(frames - 1);
        let k1 = (k + 1).min(frames - 1);
        let f = (pos - k as f64) as f32;
        for ch in 0..2 {
            let (a, b) = (stereo[k * 2 + ch], stereo[k1 * 2 + ch]);
            out.push(a + (b - a) * f);
        }
    }
    out
}

/// Decodes the start of a sample for the engine, which plays stereo at its own rate.
fn decode(path: &Path, engine_rate: u32) -> Result<Vec<f32>, String> {
    let name = display_name(path);
    let file = AudioFile::open(path).map_err(|e| format!("cannot preview {name}: {e}"))?;
    let stereo = file
        .read_stereo_for(PREVIEW_SECONDS)
        .map_err(|e| format!("cannot preview {name}: {e}"))?;
    let out = resample(&stereo, file.sample_rate, engine_rate, PREVIEW_SECONDS);
    if out.is_empty() {
        Err(format!("cannot preview {name}: it is empty"))
    } else {
        Ok(out)
    }
}

impl SampleBrowser {
    /// Runs the preview: starts a click's decode, hands the decoded sample to the engine, and
    /// takes it back when it has played out. Called every frame by `App`; `engine` is `None`
    /// while there is no audio output.
    pub fn pump(&mut self, engine: Option<&mut Engine>, ctx: &egui::Context) {
        let pv = &mut self.preview;
        let Some(engine) = engine else {
            if pv.want.take().is_some() {
                self.message = Some("No audio output: cannot preview".into());
            }
            pv.playing = None;
            pv.ends_at = None;
            pv.stop = false;
            return;
        };
        if pv.ends_at.is_some_and(|t| Instant::now() >= t) {
            pv.stop = true;
        }
        if pv.stop {
            pv.id += 1;
            pv.want = None;
            pv.ends_at = None;
            if engine.send(Command::SetPreview(None)) {
                pv.stop = false;
                pv.playing = None;
            }
        }
        if let Some(path) = pv.want.take() {
            pv.id += 1;
            pv.ends_at = None;
            pv.playing = Some(path.clone());
            let tx = pv.sender();
            let (id, rate, ctx) = (pv.id, engine.sample_rate(), ctx.clone());
            std::thread::spawn(move || {
                let out = decode(&path, rate);
                let _ = tx.send((id, out));
                ctx.request_repaint();
            });
        }
        let mut ready = None;
        if let Some(rx) = &pv.rx {
            while let Ok((id, result)) = rx.try_recv() {
                if id != pv.id {
                    continue;
                }
                match result {
                    Ok(samples) => ready = Some(samples),
                    Err(e) => {
                        // Whatever was playing goes on no longer than the failed click.
                        pv.stop = true;
                        self.message = Some(e);
                    }
                }
            }
        }
        if let Some(samples) = ready {
            let seconds = (samples.len() / 2) as f64 / f64::from(engine.sample_rate().max(1));
            let preview = Box::new(Preview {
                samples,
                gain: PREVIEW_GAIN,
            });
            if engine.send(Command::SetPreview(Some(preview))) {
                let length = Duration::from_secs_f64(seconds);
                pv.ends_at = Some(Instant::now() + length);
                ctx.request_repaint_after(length);
            } else {
                pv.playing = None;
                self.message = Some("The audio engine is busy; click the sample again".into());
            }
        }
    }

    fn ensure_loaded(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        let (roots, migrated) = load_roots(
            read_setting(ROOTS_FILE).as_deref(),
            read_setting(OLD_ROOT_FILE).as_deref(),
        );
        self.tree.roots = roots;
        if migrated {
            self.save_roots();
        }
        self.favorites = read_setting(FAVORITES_FILE)
            .map(|t| parse_paths(&t))
            .unwrap_or_default();
        self.tree.expanded = read_setting(EXPANDED_FILE)
            .map(|t| parse_paths(&t))
            .unwrap_or_default()
            .into_iter()
            .collect();
        self.rows_dirty = true;
    }

    fn save_roots(&self) {
        write_setting(ROOTS_FILE, &paths_text(&self.tree.roots));
    }

    fn save_expanded(&self) {
        let mut open: Vec<&PathBuf> = self.tree.expanded.iter().collect();
        open.sort();
        write_setting(EXPANDED_FILE, &paths_text(open));
    }

    fn add_root(&mut self, path: PathBuf) {
        if self.tree.add_root(path) {
            self.save_roots();
            self.save_expanded();
            self.show_favorites = false;
            self.rows_dirty = true;
        }
    }

    fn remove_root(&mut self, path: &Path) {
        if self.tree.remove_root(path) {
            self.save_roots();
            self.save_expanded();
            self.rows_dirty = true;
        }
    }

    fn toggle_favorite(&mut self, path: &Path) {
        if let Some(i) = self.favorites.iter().position(|f| f == path) {
            self.favorites.remove(i);
        } else {
            self.favorites.push(path.to_path_buf());
        }
        write_setting(FAVORITES_FILE, &paths_text(&self.favorites));
        self.rows_dirty = self.show_favorites;
    }

    /// Flattens the tree again, or the favorites, and asks for the folders still to be read.
    fn rebuild_rows(&mut self, ctx: &egui::Context) {
        self.rows_dirty = false;
        let (rows, need) = if self.show_favorites {
            let tops: Vec<Entry> = self
                .favorites
                .iter()
                .map(|p| Entry {
                    name: display_name(p),
                    folder: p.is_dir(),
                    path: p.clone(),
                })
                .collect();
            self.tree.flatten(&tops, false)
        } else {
            self.tree.flatten(&self.tree.root_entries(), true)
        };
        self.rows = rows;
        for dir in need {
            self.loader.request(dir, ctx);
        }
    }
}

// ------------------------------------------------------------------------- drawing

fn star(g: &egui::Painter, c: Pos2, filled: bool, color: Color32) {
    let r = 6.0;
    let pts: Vec<Pos2> = (0..10)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
            let radius = if i % 2 == 0 { r } else { r * 0.42 };
            c + vec2(a.cos(), a.sin()) * radius
        })
        .collect();
    if filled {
        let inner: Vec<Pos2> = pts.iter().skip(1).step_by(2).copied().collect();
        g.add(Shape::convex_polygon(inner, color, Stroke::NONE));
        for k in 0..5 {
            let tip = pts[2 * k];
            let (a, b) = (pts[(2 * k + 9) % 10], pts[2 * k + 1]);
            g.add(Shape::convex_polygon(vec![a, tip, b], color, Stroke::NONE));
        }
    } else {
        g.add(Shape::closed_line(pts, Stroke::new(1.0, color)));
    }
}

/// The triangle of a folder: pointing right when closed, down when open.
/// A folder's colour, by the words in its name. So the eye finds the drums without reading.
fn folder_color(name: &str) -> Color32 {
    let name = name.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| name.contains(w));
    if has(&["drum", "kick", "snare", "hat", "perc", "808", "clap"]) {
        Color32::from_rgb(0x4f, 0xb8, 0x6a)
    } else if has(&["loop"]) {
        Color32::from_rgb(0x2f, 0xb0, 0xa8)
    } else if has(&["one shot", "oneshot", "one-shot", "shot"]) {
        Color32::from_rgb(0x4a, 0x96, 0xe6)
    } else if has(&["fx", "effect"]) {
        Color32::from_rgb(0x7a, 0x6a, 0xe0)
    } else if has(&["vocal", "vox", "voice"]) {
        Color32::from_rgb(0xd8, 0x8a, 0x5a)
    } else if has(&["template", "preset"]) {
        Color32::from_rgb(0xc8, 0x5a, 0xb4)
    } else if has(&["bass", "synth", "melod", "keys", "piano"]) {
        Color32::from_rgb(0xd8, 0xb0, 0x4a)
    } else {
        Color32::from_rgb(0x7f, 0x86, 0x92)
    }
}

fn disclosure(g: &egui::Painter, c: Pos2, open: bool) {
    let pts = if open {
        vec![
            c + vec2(-4.0, -2.5),
            c + vec2(4.0, -2.5),
            c + vec2(0.0, 3.5),
        ]
    } else {
        vec![
            c + vec2(-2.5, -4.0),
            c + vec2(3.5, 0.0),
            c + vec2(-2.5, 4.0),
        ]
    };
    g.add(Shape::convex_polygon(pts, text_dim_col(), Stroke::NONE));
}

/// What a click on a row asked for.
enum RowAct {
    Toggle(PathBuf),
    /// A folder found by search: back to the tree, open down to it.
    Reveal(PathBuf),
    Select(PathBuf),
    Add(PathBuf),
    Favorite(PathBuf),
    RemoveRoot(PathBuf),
}

/// How a row is drawn besides what it holds.
struct RowLook<'a> {
    selected: bool,
    playing: bool,
    favorite: bool,
    /// The folder a search hit or favorite is in.
    parent: Option<&'a str>,
    /// This row is a search hit.
    hit: bool,
}

fn note_row(ui: &mut Ui, r: &Row) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
    let g = ui.painter_at(rect);
    g.text(
        pos2(
            rect.left() + 20.0 + INDENT * r.depth as f32,
            rect.center().y,
        ),
        egui::Align2::LEFT_CENTER,
        &r.name,
        FontId::proportional(11.5),
        text_dim_col(),
    );
}

fn row(ui: &mut Ui, r: &Row, look: &RowLook) -> Option<RowAct> {
    if r.kind == RowKind::Note {
        note_row(ui, r);
        return None;
    }
    let is_folder = matches!(r.kind, RowKind::Folder { .. });
    let (rect, resp) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click_and_drag());
    let end_rect = Rect::from_min_size(pos2(rect.right() - 20.0, rect.top()), vec2(20.0, ROW_H));
    let end_resp = ui.interact(end_rect, resp.id.with("row_end"), Sense::click());
    if !is_folder {
        resp.dnd_set_drag_payload(SampleDrag(r.path.clone()));
    }
    let g = ui.painter();
    if look.selected {
        g.rect_filled(rect, 0.0, sel_bg_col());
    } else if resp.hovered() {
        g.rect_filled(rect, 0.0, hover_bg_col());
    }
    let text_rect = Rect::from_min_max(rect.min, pos2(end_rect.left(), rect.bottom()));
    let clip = g.with_clip_rect(text_rect);
    let x = rect.left() + 4.0 + INDENT * r.depth as f32;
    let mark = pos2(x + 5.0, rect.center().y);
    // A folder has its picture, in a colour by what it holds.
    let name_x = if is_folder { x + 32.0 } else { x + 14.0 };
    if is_folder {
        let at = Rect::from_center_size(pos2(x + 23.0, rect.center().y), vec2(13.0, 10.0));
        let color = folder_color(&r.name);
        clip.rect_filled(
            Rect::from_min_size(at.min - vec2(0.0, 1.5), vec2(6.0, 3.0)),
            1.0,
            color,
        );
        clip.rect_filled(at, 1.5, color);
    }
    if let RowKind::Folder { open } = r.kind {
        disclosure(&clip, mark, open);
    } else if look.playing {
        clip.add(Shape::convex_polygon(
            vec![
                mark + vec2(-3.5, -4.5),
                mark + vec2(4.0, 0.0),
                mark + vec2(-3.5, 4.5),
            ],
            GREEN,
            Stroke::NONE,
        ));
    }
    let name = clip.text(
        pos2(name_x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        &r.name,
        FontId::proportional(12.5),
        text_col(),
    );
    if let Some(parent) = look.parent {
        clip.text(
            pos2(name.right() + 8.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            parent,
            FontId::proportional(11.0),
            text_dim_col(),
        );
    }
    if !is_folder {
        let (bpm, key) = crate::sample_tags::name_info(&r.name);
        let tags = crate::sample_tags::tags_of(&r.path);
        let mut info: Vec<String> = Vec::new();
        if let Some(b) = bpm {
            info.push(format!("{b} BPM"));
        }
        if let Some(k) = key {
            info.push(k);
        }
        info.extend(tags.iter().map(|t| format!("#{t}")));
        if !info.is_empty() {
            let text = info.join(" · ");
            let at = pos2(text_rect.right() - 4.0, rect.center().y);
            let galley = g.layout_no_wrap(text, FontId::proportional(10.5), text_dim_col());
            let room = Rect::from_min_max(pos2(at.x - galley.size().x - 6.0, rect.top()), pos2(at.x + 2.0, rect.bottom()));
            // Only where the name leaves room for it.
            if room.left() > name.right() + 6.0 {
                g.galley(pos2(room.left() + 4.0, rect.center().y - galley.size().y / 2.0), galley, text_dim_col());
            }
        }
        crate::widgets::context_menu(&resp, |ui| {
            ui.label(egui::RichText::new("Tags, with commas between").small());
            let id = egui::Id::unique(("sample_tags_edit", r.path.display().to_string()));
            let mut text: String = ui
                .ctx()
                .data(|d| d.get_temp(id))
                .unwrap_or_else(|| crate::sample_tags::tags_of(&r.path).join(", "));
            let edit = ui.add(egui::TextEdit::singleline(&mut text).hint_text("kick, dark, 808").desired_width(200.0));
            if edit.changed() {
                crate::sample_tags::set_tags(&r.path, crate::sample_tags::parse(&text));
            }
            ui.ctx().data_mut(|d| d.insert_temp(id, text));
        });
    }
    let hot = end_resp.hovered() || resp.hovered();
    if r.location {
        if hot {
            g.text(
                end_rect.center(),
                egui::Align2::CENTER_CENTER,
                "×",
                FontId::proportional(14.0),
                if end_resp.hovered() { text_col() } else { text_dim_col() },
            );
        }
    } else if look.favorite || hot {
        star(
            g,
            end_rect.center(),
            look.favorite,
            if look.favorite { GOLD } else { text_dim_col() },
        );
    }
    if r.location {
        end_resp.clone().on_hover_text(
            "Remove this location from the Browser (the folder on disk is not touched)",
        );
    }
    if look.parent.is_some() || name.right() > text_rect.right() {
        resp.clone().on_hover_text(r.path.display().to_string());
    }
    if end_resp.clicked() {
        Some(if r.location {
            RowAct::RemoveRoot(r.path.clone())
        } else {
            RowAct::Favorite(r.path.clone())
        })
    } else if is_folder && resp.clicked() {
        Some(if look.hit {
            RowAct::Reveal(r.path.clone())
        } else {
            RowAct::Toggle(r.path.clone())
        })
    } else if !is_folder && resp.double_clicked() {
        Some(RowAct::Add(r.path.clone()))
    } else if !is_folder && resp.clicked() {
        Some(RowAct::Select(r.path.clone()))
    } else {
        None
    }
}

fn hit_row(e: &Entry) -> Row {
    Row {
        path: e.path.clone(),
        name: e.name.clone(),
        depth: 0,
        kind: if e.folder {
            RowKind::Folder { open: false }
        } else {
            RowKind::Sample
        },
        location: false,
    }
}

/// The Browser tab: buttons, search, the tree, and the message line.
pub fn show(ui: &mut Ui, s: &mut UiState) {
    let panel = ui.available_rect_before_wrap();
    ui.allocate_rect(panel, Sense::hover());
    let g = ui.painter_at(panel);
    g.rect_filled(panel, 0.0, panel_bg_col());
    g.line_segment(
        [panel.right_top(), panel.right_bottom()],
        Stroke::new(1.0, crate::theme::border()),
    );
    let ctx = ui.ctx().clone();
    let inner = panel.shrink2(vec2(6.0, 4.0));
    let UiState {
        samples: b,
        open_paths,
        ..
    } = s;
    b.ensure_loaded();
    ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
        content(ui, b, open_paths, &ctx);
    });
    // The sample being dragged follows the pointer until it is dropped.
    if let Some(drag) = egui::DragAndDrop::payload::<SampleDrag>(&ctx) {
        if let Some(pos) = ctx.pointer_interact_pos() {
            let name = display_name(&drag.0);
            egui::Area::new(egui::Id::unique("sample_drag_ghost"))
                .order(egui::Order::Tooltip)
                .fixed_pos(pos + vec2(14.0, 10.0))
                .interactable(false)
                .show(&ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| ui.label(name));
                });
        }
    }
}

/// Asks for a folder and adds it as a location.
fn pick_root(b: &mut SampleBrowser) {
    let mut dialog = rfd::FileDialog::new();
    if let Some(last) = b.tree.roots.last() {
        dialog = dialog.set_directory(last);
    }
    if let Some(picked) = dialog.pick_folder() {
        b.add_root(picked);
    }
}

fn content(ui: &mut Ui, b: &mut SampleBrowser, open_paths: &mut Vec<PathBuf>, ctx: &egui::Context) {
    // Locations.
    let mut add = false;
    ui.horizontal(|ui| {
        add = ui
            .button("Add folder…")
            .on_hover_text("Add a sample location to the Browser")
            .clicked();
        if ui
            .button("Refresh")
            .on_hover_text("Read the open folders again")
            .clicked()
        {
            b.tree.listings.clear();
            b.rows_dirty = true;
        }
    });

    // Search.
    ui.horizontal(|ui| {
        let width = ui.available_width() - 26.0;
        ui.add(
            egui::TextEdit::singleline(&mut b.search)
                .hint_text("Search all locations")
                .desired_width(width.max(40.0)),
        );
        if !b.search.is_empty()
            && crate::icons::button(ui, crate::icons::Icon::Close, "Clear the search").clicked()
        {
            b.search.clear();
        }
    });
    let query = b.search.trim().to_string();
    let want = (!query.is_empty()).then(|| (b.tree.roots.clone(), query));
    if b.hits.for_query != want {
        match want {
            Some((roots, q)) => b.hits.start(&roots, &q, ctx),
            None => b.hits.clear(),
        }
    }
    b.hits.drain();

    // Options.
    ui.horizontal(|ui| {
        ui.checkbox(&mut b.auto_preview, "Preview");
        if ui.selectable_label(b.show_favorites, "Favorites").clicked() {
            b.show_favorites = !b.show_favorites;
            b.rows_dirty = true;
        }
        if b.preview.playing.is_some()
            && crate::icons::button(ui, crate::icons::Icon::Stop, "Stop the preview").clicked()
        {
            b.preview.stop = true;
        }
    });
    ui.add_space(2.0);

    b.rows_dirty |= b.loader.drain(&mut b.tree);
    if b.rows_dirty {
        b.rebuild_rows(ctx);
    }

    // The list: search hits, favorites, or the tree.
    let searching = !b.search.trim().is_empty() && !b.show_favorites;
    let count = if searching {
        b.hits.hits.len()
    } else {
        b.rows.len()
    };
    let mut act = None;
    if b.tree.roots.is_empty() && !b.show_favorites && !searching {
        ui.add_space(8.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new("Add a folder with your samples to start.").color(text_dim_col()),
            )
            .wrap(),
        );
        add |= ui.button("Add folder…").clicked();
    }
    let footer = 18.0
        + if b.wave_for.is_some() {
            WAVE_H + 4.0
        } else {
            0.0
        };
    let list_h = (ui.available_height() - footer).max(40.0);
    let mut area = egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .max_height(list_h);
    if !searching {
        if let Some(target) = &b.scroll_to {
            let found = b
                .rows
                .iter()
                .position(|r| r.kind != RowKind::Note && &r.path == target);
            if let Some(i) = found {
                area = area.vertical_scroll_offset(i as f32 * ROW_H);
                b.scroll_to = None;
            } else if b.loader.in_flight.is_empty() {
                b.scroll_to = None;
            }
        }
    }
    ui.scope(|ui| {
        // `show_rows` places rows by this spacing, so it is set before the call: set inside, the
        // rows drift against their positions and jump while scrolling.
        ui.spacing_mut().item_spacing.y = 0.0;
        area.show_rows(ui, ROW_H, count, |ui, range| {
            for i in range {
                let hit;
                let r = if searching {
                    hit = hit_row(&b.hits.hits[i]);
                    &hit
                } else {
                    &b.rows[i]
                };
                let flat = searching || (b.show_favorites && r.depth == 0);
                let parent = flat.then(|| r.path.parent().map(display_name)).flatten();
                let look = RowLook {
                    selected: b.selected.as_ref() == Some(&r.path),
                    playing: b.preview.playing.as_ref() == Some(&r.path),
                    favorite: b.favorites.contains(&r.path),
                    parent: parent.as_deref(),
                    hit: searching,
                };
                if let Some(a) = row(ui, r, &look) {
                    act = Some(a);
                }
            }
        });
    });
    let status = if b.show_favorites && count == 0 {
        Some("Click the star on a folder or sample to keep it here".to_string())
    } else if searching && b.hits.running {
        Some("Searching…".to_string())
    } else if searching && count == 0 {
        Some("Nothing found".to_string())
    } else if searching && count >= MAX_HITS {
        Some("Showing the first results; search to narrow down".to_string())
    } else {
        b.message.clone()
    };
    if let Some(text) = status {
        ui.add(egui::Label::new(egui::RichText::new(text).small().color(text_dim_col())).truncate());
    }

    wave_panel(ui, b);
    if b.preview.playing.is_some()
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
    {
        b.preview.stop = true;
    }

    if add {
        pick_root(b);
    }
    // Applied after the list is drawn, which only reads the browser.
    match act {
        Some(RowAct::Toggle(path)) => {
            b.tree.toggle(&path);
            b.save_expanded();
            b.rows_dirty = true;
            b.message = None;
        }
        Some(RowAct::Reveal(path)) => {
            b.tree.reveal(&path);
            b.save_expanded();
            b.search.clear();
            b.selected = Some(path.clone());
            b.scroll_to = Some(path);
            b.rows_dirty = true;
            b.message = None;
        }
        Some(RowAct::Select(path)) => {
            b.selected = Some(path.clone());
            b.message = None;
            if crate::import::is_audio_path(&path) {
                b.show_wave(&path, ctx);
                // A click on what is playing stops it.
                if b.auto_preview || b.preview.playing.as_ref() == Some(&path) {
                    b.toggle_preview(path);
                }
            }
        }
        Some(RowAct::Add(path)) => {
            b.selected = Some(path.clone());
            b.preview.stop = true;
            open_paths.push(path);
        }
        Some(RowAct::Favorite(path)) => b.toggle_favorite(&path),
        Some(RowAct::RemoveRoot(path)) => b.remove_root(&path),
        None => {}
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_click_on_what_is_playing_stops_it() {
        let mut b = SampleBrowser::default();
        let path = PathBuf::from("a.wav");
        b.toggle_preview(path.clone());
        assert_eq!(b.preview.want.as_ref(), Some(&path));
        b.preview.want = None;
        b.preview.playing = Some(path.clone());
        b.toggle_preview(path.clone());
        assert!(b.preview.stop);
        assert!(b.preview.want.is_none());
        // Another sample takes over instead.
        b.preview.stop = false;
        b.toggle_preview("b.wav".into());
        assert!(!b.preview.stop);
        assert!(b.preview.want.is_some());
    }

    #[test]
    fn the_waveform_panel_reads_a_sample_and_says_what_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("Loop 98 BPM.wav");
        ovrtone_engine::write_wav(&wav, &vec![0.5; 48_000 * 2], 48_000).unwrap();
        let info = read_wave(&wav).unwrap();
        assert!(info.peaks.iter().any(|(_, hi)| *hi > 0.4));
        assert_eq!(wave_text(&info), "48.0 kHz   98 BPM   1.0 s");
        assert!(read_wave(&dir.path().join("none.wav")).is_none());
    }

    fn names(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    fn file(path: &str) -> Entry {
        let path = PathBuf::from(path);
        Entry {
            name: display_name(&path),
            path,
            folder: false,
        }
    }

    fn outline(rows: &[Row]) -> Vec<(usize, &str)> {
        rows.iter().map(|r| (r.depth, r.name.as_str())).collect()
    }

    /// `/lib` with a folder `Drums`, holding `Kicks` and `a.wav`; `Kicks` holds `k.wav`.
    fn tree() -> Tree {
        let mut t = Tree::default();
        assert!(t.add_root(PathBuf::from("/lib")));
        t.listings.insert(
            PathBuf::from("/lib"),
            Some(vec![Entry::folder(Path::new("/lib/Drums"))]),
        );
        t.listings.insert(
            PathBuf::from("/lib/Drums"),
            Some(vec![
                Entry::folder(Path::new("/lib/Drums/Kicks")),
                file("/lib/Drums/a.wav"),
            ]),
        );
        t.listings.insert(
            PathBuf::from("/lib/Drums/Kicks"),
            Some(vec![file("/lib/Drums/Kicks/k.wav")]),
        );
        t
    }

    fn rows_of(t: &Tree) -> Vec<Row> {
        t.flatten(&t.root_entries(), true).0
    }

    #[test]
    fn the_tree_flattens_with_open_folders_indented_and_closed_ones_hiding_their_children() {
        let mut t = tree();
        assert_eq!(outline(&rows_of(&t)), [(0, "lib"), (1, "Drums")]);
        t.toggle(Path::new("/lib/Drums"));
        t.toggle(Path::new("/lib/Drums/Kicks"));
        let rows = rows_of(&t);
        assert_eq!(
            outline(&rows),
            [
                (0, "lib"),
                (1, "Drums"),
                (2, "Kicks"),
                (3, "k.wav"),
                (2, "a.wav"),
            ]
        );
        assert!(rows[0].location, "only a location can be removed");
        assert!(!rows[1].location);
        assert_eq!(rows[1].kind, RowKind::Folder { open: true });
        assert_eq!(rows[4].kind, RowKind::Sample);
    }

    #[test]
    fn closing_a_folder_keeps_what_is_open_below_it_for_next_time() {
        let mut t = tree();
        t.toggle(Path::new("/lib/Drums"));
        t.toggle(Path::new("/lib/Drums/Kicks"));
        t.toggle(Path::new("/lib/Drums"));
        assert_eq!(outline(&rows_of(&t)), [(0, "lib"), (1, "Drums")]);
        assert_eq!(rows_of(&t)[1].kind, RowKind::Folder { open: false });
        t.toggle(Path::new("/lib/Drums"));
        assert_eq!(rows_of(&t).len(), 5);
    }

    #[test]
    fn several_folders_can_be_open_at_once() {
        let mut t = tree();
        assert!(t.add_root(PathBuf::from("/other")));
        t.listings
            .insert(PathBuf::from("/other"), Some(vec![file("/other/o.wav")]));
        t.toggle(Path::new("/lib/Drums"));
        assert_eq!(
            outline(&rows_of(&t)),
            [
                (0, "lib"),
                (1, "Drums"),
                (2, "Kicks"),
                (2, "a.wav"),
                (0, "other"),
                (1, "o.wav"),
            ]
        );
    }

    #[test]
    fn an_open_folder_not_yet_read_is_asked_for_and_says_so() {
        let mut t = Tree::default();
        t.add_root(PathBuf::from("/new"));
        let (rows, need) = t.flatten(&t.root_entries(), true);
        assert_eq!(need, [PathBuf::from("/new")]);
        assert_eq!(rows[1].kind, RowKind::Note);
        assert_eq!(rows[1].name, "Loading…");
        // A closed folder is not read at all.
        t.toggle(Path::new("/new"));
        assert!(t.flatten(&t.root_entries(), true).1.is_empty());
    }

    #[test]
    fn an_unreadable_or_empty_folder_shows_a_note_and_is_not_asked_for_again() {
        let mut t = Tree::default();
        t.add_root(PathBuf::from("/gone"));
        t.add_root(PathBuf::from("/empty"));
        t.listings.insert(PathBuf::from("/gone"), None);
        t.listings.insert(PathBuf::from("/empty"), Some(Vec::new()));
        let (rows, need) = t.flatten(&t.root_entries(), true);
        assert!(need.is_empty());
        assert_eq!(
            outline(&rows),
            [
                (0, "gone"),
                (1, "Cannot read this folder"),
                (0, "empty"),
                (1, "No samples here"),
            ]
        );
    }

    #[test]
    fn a_full_folder_says_it_is_cut_short() {
        let mut t = Tree::default();
        t.add_root(PathBuf::from("/big"));
        let many = (0..MAX_ENTRIES)
            .map(|i| file(&format!("/big/{i}.wav")))
            .collect();
        t.listings.insert(PathBuf::from("/big"), Some(many));
        let rows = rows_of(&t);
        assert_eq!(rows.len(), MAX_ENTRIES + 2);
        assert_eq!(rows.last().unwrap().kind, RowKind::Note);
    }

    #[test]
    fn locations_are_added_once_and_removing_one_forgets_its_folders() {
        let mut t = tree();
        assert!(!t.add_root(PathBuf::from("/lib")), "already there");
        assert!(t.add_root(PathBuf::from("/other")));
        t.toggle(Path::new("/lib/Drums"));
        assert!(t.remove_root(Path::new("/lib")));
        assert!(!t.remove_root(Path::new("/lib")), "already gone");
        assert_eq!(t.roots, [PathBuf::from("/other")]);
        assert!(!t.expanded.contains(Path::new("/lib/Drums")));
        assert!(t.listings.is_empty());
        assert!(t.expanded.contains(Path::new("/other")));
    }

    #[test]
    fn removing_a_location_keeps_folders_another_location_holds() {
        let mut t = tree();
        t.add_root(PathBuf::from("/lib/Drums"));
        assert!(t.remove_root(Path::new("/lib")));
        assert!(t.expanded.contains(Path::new("/lib/Drums")));
        assert!(t.listings.contains_key(Path::new("/lib/Drums")));
        assert!(!t.listings.contains_key(Path::new("/lib")));
    }

    #[test]
    fn revealing_a_folder_opens_the_way_down_to_it() {
        let mut t = tree();
        t.reveal(Path::new("/lib/Drums/Kicks"));
        assert_eq!(rows_of(&t).len(), 5);
        let before = t.expanded.clone();
        t.reveal(Path::new("/elsewhere/Kicks"));
        assert_eq!(t.expanded, before, "outside every location nothing opens");
    }

    #[test]
    fn a_folder_inside_another_location_is_searched_once() {
        let roots = [
            PathBuf::from("/lib/Drums"),
            PathBuf::from("/lib"),
            PathBuf::from("/other"),
        ];
        assert_eq!(
            outermost(&roots),
            [PathBuf::from("/lib"), PathBuf::from("/other")]
        );
    }

    #[test]
    fn a_folder_lists_folders_first_then_samples_and_midi() {
        let dir = tempfile::tempdir().unwrap();
        for f in ["b.wav", "A.flac", "beat.mid", "notes.txt", ".hidden.wav"] {
            std::fs::write(dir.path().join(f), b"").unwrap();
        }
        std::fs::create_dir(dir.path().join("Drums")).unwrap();
        assert_eq!(
            names(&list(dir.path()).unwrap()),
            ["Drums", "A.flac", "b.wav", "beat.mid"]
        );
        assert!(list(&dir.path().join("gone")).is_none());
    }

    #[test]
    fn files_that_are_not_samples_do_not_use_up_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("z.wav"), b"").unwrap();
        for i in 0..MAX_ENTRIES + 10 {
            std::fs::write(dir.path().join(format!("{i}.txt")), b"").unwrap();
        }
        assert_eq!(names(&list(dir.path()).unwrap()), ["z.wav"]);
    }

    #[test]
    fn the_background_loader_reads_a_folder_into_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("k.wav"), b"").unwrap();
        let mut t = Tree::default();
        t.add_root(dir.path().to_path_buf());
        let mut loader = Loader::default();
        let ctx = egui::Context::default();
        loader.request(dir.path().to_path_buf(), &ctx);
        loader.request(dir.path().join("gone"), &ctx);
        let until = Instant::now() + Duration::from_secs(10);
        while !loader.in_flight.is_empty() && Instant::now() < until {
            loader.drain(&mut t);
            std::thread::yield_now();
        }
        assert!(loader.in_flight.is_empty());
        assert_eq!(outline(&rows_of(&t))[1].1, "k.wav");
        assert_eq!(t.listings[&dir.path().join("gone")], None);
    }

    #[test]
    fn search_words_match_in_any_order_ignoring_case() {
        let t = terms("  808  KICK ");
        assert!(matches_all("Hard Kick 808 (Dry).wav", &t));
        assert!(!matches_all("Hard Kick.wav", &t));
        assert!(matches_all("anything", &terms("")));
    }

    #[test]
    fn a_search_finds_samples_and_folders_in_nested_folders_of_every_location() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let deep = dir.path().join("Pack").join("Kicks").join("Dry");
        std::fs::create_dir_all(&deep).unwrap();
        for f in [
            deep.join("Kick 01.wav"),
            deep.join("Snare 01.wav"),
            deep.join("kick notes.txt"),
            dir.path().join("Pack").join("kick.flac"),
            other.path().join("Other kick.wav"),
        ] {
            std::fs::write(f, b"").unwrap();
        }
        let (tx, rx) = channel();
        let cancel = AtomicBool::new(false);
        walk(
            &[dir.path().to_path_buf(), other.path().to_path_buf()],
            &terms("kick"),
            7,
            &cancel,
            &tx,
            &egui::Context::default(),
        );
        let mut found = Vec::new();
        let mut done = false;
        while let Ok((generation, batch, last)) = rx.try_recv() {
            assert_eq!(generation, 7);
            found.extend(batch);
            done |= last;
        }
        assert!(done, "the search says when it has finished");
        found.sort_by_key(|e| e.name.clone());
        assert_eq!(
            names(&found),
            ["Kick 01.wav", "Kicks", "Other kick.wav", "kick.flac"]
        );
        assert!(found.iter().any(|e| e.folder && e.name == "Kicks"));
    }

    #[test]
    fn a_cancelled_search_sends_nothing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("kick.wav"), b"").unwrap();
        let (tx, rx) = channel();
        let cancel = AtomicBool::new(true);
        walk(
            &[dir.path().to_path_buf()],
            &terms("kick"),
            1,
            &cancel,
            &tx,
            &egui::Context::default(),
        );
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn resampling_keeps_the_pitch_and_cuts_the_length() {
        // A ramp: each frame is (i, -i) so any mistake in position or channel shows.
        let src: Vec<f32> = (0..1000).flat_map(|i| [i as f32, -(i as f32)]).collect();
        assert_eq!(resample(&src, 48_000, 48_000, 60.0), src);
        let half = resample(&src, 48_000, 24_000, 60.0);
        assert_eq!(half.len(), 500 * 2);
        assert_eq!(half[2 * 100], 200.0);
        assert_eq!(half[2 * 100 + 1], -200.0);
        let double = resample(&src, 24_000, 48_000, 60.0);
        assert_eq!(double.len(), 2000 * 2);
        assert!((double[2 * 3] - 1.5).abs() < 1e-4, "a frame between two");
        let short = resample(&src, 1000, 1000, 0.25);
        assert_eq!(short.len(), 250 * 2, "cut to max_seconds");
        assert!(resample(&[], 44_100, 48_000, 60.0).is_empty());
    }

    #[test]
    fn a_preview_decodes_only_the_part_it_plays() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("long.wav");
        ovrtone_engine::write_wav(&wav, &vec![0.1; 48_000 * 2 * 2], 48_000).unwrap();
        let file = AudioFile::open(&wav).unwrap();
        assert_eq!(file.read_stereo_for(0.5).unwrap().len(), 48_000);
        assert_eq!(file.read_stereo().unwrap().len(), 48_000 * 2 * 2);
    }

    /// A Browser showing `/lib`, holding a closed `Drums` (with `d.wav`) and 200 samples.
    fn browser_with_library() -> SampleBrowser {
        let mut samples = vec![Entry::folder(Path::new("/lib/Drums"))];
        samples.extend((0..200).map(|i| file(&format!("/lib/s{i:03}.wav"))));
        let mut b = SampleBrowser {
            loaded: true,
            ..SampleBrowser::default()
        };
        b.tree.add_root(PathBuf::from("/lib"));
        b.tree.listings.insert(PathBuf::from("/lib"), Some(samples));
        b.tree.listings.insert(
            PathBuf::from("/lib/Drums"),
            Some(vec![file("/lib/Drums/d.wav")]),
        );
        b
    }

    fn draw<'a>(b: &'a mut SampleBrowser, open: &'a mut Vec<PathBuf>) -> impl FnMut(&mut Ui) + 'a {
        move |ui| {
            let ctx = ui.ctx().clone();
            content(ui, b, open, &ctx);
        }
    }

    /// Where a row sits: `y` is known to hold `row`; rows are `ROW_H` apart.
    fn row_pos(y: f32, row: usize, wanted: usize) -> Pos2 {
        pos2(100.0, y + (wanted as f32 - row as f32) * ROW_H)
    }

    fn selected_row(b: &SampleBrowser) -> usize {
        let selected = b.selected.as_ref().unwrap();
        b.rows.iter().position(|r| &r.path == selected).unwrap()
    }

    #[test]
    fn clicks_open_and_close_folders_and_play_samples_in_the_window() {
        let mut h = crate::ui_test::Harness::new(290.0, 600.0);
        let mut b = browser_with_library();
        let mut open = Vec::new();
        h.settle(draw(&mut b, &mut open));
        // A click somewhere in the list says which row is there, and so where they all are.
        let y = 250.0;
        h.click(pos2(100.0, y), draw(&mut b, &mut open));
        let at_y = selected_row(&b);
        assert!(b.preview.want.is_some(), "a click on a sample plays it");

        h.click(row_pos(y, at_y, 1), draw(&mut b, &mut open));
        assert_eq!(b.rows[1].kind, RowKind::Folder { open: true });
        assert_eq!(b.rows[2].name, "d.wav");
        assert_eq!(b.rows[2].depth, 2);
        h.click(row_pos(y, at_y, 1), draw(&mut b, &mut open));
        assert_eq!(b.rows[1].kind, RowKind::Folder { open: false });
        assert_eq!(b.rows[2].name, "s000.wav");

        h.click(row_pos(y, at_y, 0), draw(&mut b, &mut open));
        assert_eq!(b.rows.len(), 1, "closing the location hides all of it");
        assert_eq!(b.tree.roots.len(), 1, "and does not remove it");
    }

    #[test]
    fn the_end_of_a_location_row_removes_it() {
        let mut h = crate::ui_test::Harness::new(290.0, 600.0);
        let mut b = browser_with_library();
        let mut open = Vec::new();
        h.settle(draw(&mut b, &mut open));
        let y = 250.0;
        h.click(pos2(100.0, y), draw(&mut b, &mut open));
        let at_y = selected_row(&b);
        let end = pos2(290.0 - 20.0, row_pos(y, at_y, 0).y);
        h.click(end, draw(&mut b, &mut open));
        assert!(b.tree.roots.is_empty());
        assert!(b.rows.is_empty());
    }

    #[test]
    fn rows_stay_where_they_are_drawn_when_scrolled() {
        let mut h = crate::ui_test::Harness::new(290.0, 600.0);
        let mut b = browser_with_library();
        let mut open = Vec::new();
        h.settle(draw(&mut b, &mut open));
        let y = 200.0;
        h.click(pos2(100.0, y), draw(&mut b, &mut open));
        let at_y = selected_row(&b);

        // Scrolled by whole rows, the same place on screen holds exactly that many rows further.
        b.scroll_to = Some(b.rows[100].path.clone());
        h.settle(draw(&mut b, &mut open));
        h.click(pos2(100.0, y), draw(&mut b, &mut open));
        assert_eq!(selected_row(&b), 100 + at_y);
        h.click(pos2(100.0, y + 5.0 * ROW_H), draw(&mut b, &mut open));
        assert_eq!(selected_row(&b), 105 + at_y);
    }

    #[test]
    fn double_click_adds_and_a_drag_carries_the_sample() {
        let mut h = crate::ui_test::Harness::new(290.0, 600.0);
        let mut b = browser_with_library();
        let mut open = Vec::new();
        h.settle(draw(&mut b, &mut open));
        let pos = pos2(100.0, 250.0);
        h.click(pos, draw(&mut b, &mut open));
        let sample = b.selected.clone().unwrap();
        h.click(pos, draw(&mut b, &mut open));
        assert_eq!(open, std::slice::from_ref(&sample));
        assert!(b.preview.stop, "adding it stops the preview");

        h.press(pos, egui::PointerButton::Primary, draw(&mut b, &mut open));
        for step in 1..4 {
            h.move_to(pos + vec2(step as f32 * 20.0, 0.0), draw(&mut b, &mut open));
        }
        let drag = egui::DragAndDrop::payload::<SampleDrag>(&h.ctx).unwrap();
        assert_eq!(drag.0, sample);
    }

    #[test]
    fn settings_round_trip() {
        let favs = vec![PathBuf::from("D:\\Packs\\Kicks"), PathBuf::from("x.wav")];
        assert_eq!(parse_paths(&paths_text(&favs)), favs);
        assert!(parse_paths("\n  \n").is_empty());
    }

    #[test]
    fn the_locations_start_empty_or_come_from_the_old_single_folder() {
        let lib = PathBuf::from("D:\\Lib");
        assert_eq!(load_roots(None, None), (Vec::new(), false));
        assert_eq!(load_roots(None, Some("  \n")), (Vec::new(), false));
        assert_eq!(
            load_roots(None, Some("D:\\Lib\n")),
            (vec![lib.clone()], true),
            "the old folder moves over"
        );
        assert_eq!(
            load_roots(Some("D:\\Lib\nE:\\Other\n"), Some("F:\\Old")),
            (vec![lib, PathBuf::from("E:\\Other")], false),
            "the new file wins"
        );
        assert_eq!(
            load_roots(Some(""), Some("F:\\Old")),
            (Vec::new(), false),
            "removing every location is remembered"
        );
    }
}
