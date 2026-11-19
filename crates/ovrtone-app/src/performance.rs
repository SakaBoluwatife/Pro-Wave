//! The Performance window (Window > Performance…): how hard the audio engine is working and what
//! to do about it. The load over the last minute, the longest callback against its deadline, the
//! deadlines missed, the device's dropouts and the disk's, the tracks that cost the most, and
//! advice: a larger buffer, or freezing the heaviest track. Nothing changes until a button is
//! pressed: the project's sound and settings are never altered behind the user's back.

use std::collections::VecDeque;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use egui::{Color32, RichText, Sense, Stroke, Ui};

use crate::hardware::Hardware;
use crate::model::{Project, TrackKind};
use crate::UiState;

/// Load samples kept for the graph, one every [`STEP`].
const HISTORY: usize = 300;
const STEP: Duration = Duration::from_millis(200);
/// How far back "recent" misses and dropouts count.
const RECENT: Duration = Duration::from_secs(30);
/// Tracks shown in the costliest list.
const TOP: usize = 8;

#[derive(Default)]
pub struct PerformanceView {
    pub open: bool,
    hw: Option<Hardware>,
    looking: Option<Receiver<Hardware>>,
    /// Engine counters as last read, and when the counts went up.
    history: VecDeque<f32>,
    step_load: f32,
    step_at: Option<Instant>,
    window_max: Duration,
    shown_max: Duration,
    session_max: Duration,
    budget: Duration,
    block: u32,
    rate: u32,
    latency_us: u32,
    /// Frames rendered ahead right now (0: live, the callback renders).
    ahead: u32,
    misses: u32,
    xruns: u32,
    underruns: u32,
    events: VecDeque<(Instant, Event)>,
    /// Time per track over the last step, as a share of one core, smoothed.
    cost: Vec<f32>,
    cost_ns: Vec<u64>,
    measuring: bool,
    /// Whether an engine is running.
    live: bool,
    message: Option<String>,
}

/// The longest window frame lately, in microseconds: how long building and drawing one frame took
/// on the UI thread. Falls back slowly so a slow frame stays readable.
static FRAME_US: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Times one window frame. Made at the start of the frame; dropping it at the end records the time.
pub struct FrameTimer(Instant);

impl FrameTimer {
    pub fn start() -> Self {
        Self(Instant::now())
    }
}

impl Drop for FrameTimer {
    fn drop(&mut self) {
        let us = self.0.elapsed().as_micros() as u64;
        let held = FRAME_US.load(Relaxed);
        FRAME_US.store(us.max(held - held / 32), Relaxed);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Event {
    Miss,
    Xrun,
    Underrun,
}

impl PerformanceView {
    fn recent(&self, kind: Event) -> usize {
        self.events.iter().filter(|(_, e)| *e == kind).count()
    }
}

/// Every frame, with the engine: reads its counters. `load` is the frame's DSP load (main.rs takes
/// it for the CPU meter too).
pub fn sample(
    v: &mut PerformanceView,
    sync: &mut crate::engine_sync::EngineSync, tracks: usize, load: f32) {
    v.live = true;
    if v.open != v.measuring {
        sync.engine.shared().measure_strips(v.open);
        v.measuring = v.open;
        v.cost_ns.clear();
        v.cost.clear();
    }
    if !v.open {
        return;
    }
    let now = Instant::now();
    let state = *sync.engine.state();
    v.block = state.block_frames;
    v.latency_us = state.output_latency_us;
    let underruns = sync.underruns();
    let shared = sync.engine.shared();
    v.ahead = shared.render_ahead();
    v.rate = sync.engine.sample_rate();
    v.budget = if v.rate > 0 {
        Duration::from_secs_f64(f64::from(v.block) / f64::from(v.rate))
    } else {
        Duration::ZERO
    };
    let longest = shared.take_callback_max();
    v.window_max = v.window_max.max(longest);
    v.session_max = v.session_max.max(longest);
    v.step_load = v.step_load.max(load);
    // Counts that went up since the last frame.
    let misses = shared.deadline_misses.load(Relaxed);
    let xruns = shared.xruns.load(Relaxed);
    for (was, is, kind) in [
        (v.misses, misses, Event::Miss),
        (v.xruns, xruns, Event::Xrun),
        (v.underruns, underruns, Event::Underrun),
    ] {
        if is > was && v.step_at.is_some() {
            v.events.push_back((now, kind));
        }
    }
    (v.misses, v.xruns, v.underruns) = (misses, xruns, underruns);
    while v.events.front().is_some_and(|(at, _)| now - *at > RECENT) {
        v.events.pop_front();
    }
    v.cost_ns.resize(tracks, 0);
    v.cost.resize(tracks, 0.0);
    for (t, ns) in v.cost_ns.iter_mut().enumerate() {
        *ns += shared.take_strip_time(t).as_nanos() as u64;
    }
    let Some(at) = v.step_at else {
        v.step_at = Some(now);
        return;
    };
    let span = now - at;
    if span < STEP {
        return;
    }
    v.step_at = Some(now);
    if v.history.len() == HISTORY {
        v.history.pop_front();
    }
    v.history.push_back(v.step_load);
    v.step_load = 0.0;
    v.shown_max = std::mem::take(&mut v.window_max);
    let wall = span.as_nanos().max(1) as f32;
    for (c, ns) in v.cost.iter_mut().zip(v.cost_ns.iter_mut()) {
        let share = std::mem::take(ns) as f32 / wall;
        *c = if share > *c { share } else { *c * 0.8 + share * 0.2 };
    }
}

/// Engine not running: the window says so.
pub fn no_engine(v: &mut PerformanceView) {
    v.live = false;
    v.measuring = false;
}

pub fn show(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    if !s.performance.open {
        return;
    }
    if s.performance.hw.is_none() && s.performance.looking.is_none() {
        let (tx, rx) = channel();
        let ctx2 = ctx.clone();
        let _ = std::thread::Builder::new()
            .name("ovrtone-hardware".into())
            .spawn(move || {
                let _ = tx.send(crate::hardware::detect());
                ctx2.request_repaint();
            });
        s.performance.looking = Some(rx);
    }
    if let Some(hw) = s.performance.looking.as_ref().and_then(|rx| rx.try_recv().ok()) {
        s.performance.hw = Some(hw);
        s.performance.looking = None;
    }
    let mut open = true;
    let mut buffer_to: Option<u32> = None;
    let mut freeze: Option<usize> = None;
    let mut save_report = false;
    let current_device = s.settings.current.clone();
    egui::Window::new("Performance")
        .id(egui::Id::unique("ovrtone_performance"))
        .open(&mut open)
        .default_width(460.0)
        .resizable(true)
        .show(ctx, |ui| {
            ctx.request_repaint_after(STEP);
            let v = &s.performance;
            system_section(ui, v);
            ui.separator();
            if !v.live {
                ui.label("The audio engine is not running. Choose a device in Settings.");
                return;
            }
            audio_section(ui, v, &current_device);
            ui.separator();
            load_section(ui, v);
            ui.separator();
            freeze = costly_section(ui, v, p);
            ui.separator();
            buffer_to = advice_section(ui, v, p);
            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .button("Save Report…")
                    .on_hover_text(
                        "A text file with this computer's parts and these numbers, and no names \
                         from your projects. Nothing is sent anywhere: share it if you choose to.",
                    )
                    .clicked()
                {
                    save_report = true;
                }
                if let Some(m) = &v.message {
                    ui.label(RichText::new(m).weak());
                }
            });
        });
    s.performance.open = open;
    if let Some(n) = buffer_to {
        let mut setup = crate::settings::AudioSetup::load();
        setup.buffer = Some(n);
        setup.save();
        s.settings.apply = Some(setup);
        s.performance.message = Some(format!("Restarting the sound at {n} samples"));
        s.performance.session_max = Duration::ZERO;
        s.performance.events.clear();
    }
    if let Some(t) = freeze {
        crate::bounce::toggle_freeze(p, s, t);
    }
    if save_report {
        write_report(p, s);
    }
}

fn row(ui: &mut Ui, name: &str, value: impl Into<egui::WidgetText>) {
    ui.label(RichText::new(name).weak());
    ui.label(value);
    ui.end_row();
}

fn system_section(ui: &mut Ui, v: &PerformanceView) {
    ui.strong("This computer");
    let Some(hw) = &v.hw else {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Looking at the hardware…");
        });
        return;
    };
    egui::Grid::new(egui::Id::unique("perf_sys")).num_columns(2).show(ui, |ui| {
        row(ui, "Processor", &hw.cpu);
        let cores = match hw.physical {
            Some(p) => format!("{p} cores, {} threads", hw.logical),
            None => format!("{} threads", hw.logical),
        };
        row(ui, "Cores", cores);
        row(ui, "Vector sets", hw.simd.join(" "));
        row(
            ui,
            "Engine kernels",
            format!("{} path", ovrtone_engine::simd_level().name()),
        );
        row(
            ui,
            "Memory",
            format!("{:.1} GB ({:.1} GB free)", hw.ram_gb, hw.free_gb),
        );
        if !hw.os.is_empty() {
            row(ui, "System", &hw.os);
        }
        row(
            ui,
            "Class",
            format!(
                "{}: suggested buffer {} samples, {} helper threads",
                hw.tier().name(),
                hw.suggested_buffer(None),
                hw.suggested_workers()
            ),
        );
    });
}

fn audio_section(ui: &mut Ui, v: &PerformanceView, device: &str) {
    ui.strong("Audio");
    egui::Grid::new(egui::Id::unique("perf_audio")).num_columns(2).show(ui, |ui| {
        if !device.is_empty() {
            row(ui, "Playing through", device);
        }
        let ms = v.budget.as_secs_f64() * 1000.0;
        row(
            ui,
            "Buffer",
            format!("{} samples at {} Hz ({ms:.1} ms)", v.block, v.rate),
        );
        row(
            ui,
            "Render ahead",
            if v.ahead > 0 {
                format!("On, {:.0} ms ahead", f64::from(v.ahead) * 1000.0 / f64::from(v.rate.max(1)))
            } else if crate::settings::render_ahead() {
                "Waiting: only while playing and nothing is live".to_string()
            } else {
                "Off (Settings)".to_string()
            },
        );
        if v.latency_us > 0 {
            row(
                ui,
                "Output delay",
                format!("{:.1} ms", f64::from(v.latency_us) / 1000.0),
            );
        }
        if let Some(caps) = v.hw.as_ref().and_then(|hw| hw.device(device)) {
            if !caps.rates.is_empty() {
                let rates: Vec<String> = caps.rates.iter().map(|r| r.to_string()).collect();
                row(ui, "Device rates", rates.join(", "));
            }
            if let Some((lo, hi)) = caps.buffer {
                row(ui, "Device buffers", format!("{lo} to {hi} samples"));
            }
        }
    });
}

fn load_section(ui: &mut Ui, v: &PerformanceView) {
    ui.strong("Load");
    let now = v.history.back().copied().unwrap_or(0.0);
    let (w, h) = (ui.available_width(), 60.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);
    // The deadline: above this line the sound may break up.
    let y_of = |load: f32| rect.bottom() - (load.clamp(0.0, 1.25) / 1.25) * rect.height();
    painter.hline(
        rect.x_range(),
        y_of(1.0),
        Stroke::new(1.0, Color32::from_rgb(200, 70, 60)),
    );
    if v.history.len() > 1 {
        let step = rect.width() / (HISTORY - 1) as f32;
        let start = HISTORY - v.history.len();
        let points: Vec<egui::Pos2> = v
            .history
            .iter()
            .enumerate()
            .map(|(i, l)| egui::pos2(rect.left() + (start + i) as f32 * step, y_of(*l)))
            .collect();
        let colour = if now > 0.85 {
            Color32::from_rgb(230, 150, 50)
        } else {
            Color32::from_rgb(90, 180, 110)
        };
        painter.add(egui::Shape::line(points, Stroke::new(1.5, colour)));
    }
    egui::Grid::new(egui::Id::unique("perf_load")).num_columns(2).show(ui, |ui| {
        row(ui, "Now", format!("{:.0}% of the deadline", now * 100.0));
        let us = |d: Duration| d.as_secs_f64() * 1e6;
        row(
            ui,
            "Longest callback",
            format!(
                "{:.0} µs now, {:.0} µs at worst (deadline {:.0} µs)",
                us(v.shown_max),
                us(v.session_max),
                us(v.budget)
            ),
        );
        // The window's own time, for telling a slow redraw from a slow mix. The audio threads
        // all outrank the window, so a slow frame here no longer breaks the sound up.
        row(
            ui,
            "Window frame",
            format!("{:.1} ms at most lately", FRAME_US.load(Relaxed) as f64 / 1000.0),
        );
        row(
            ui,
            "Deadlines missed",
            format!("{} ({} in the last 30 s)", v.misses, v.recent(Event::Miss)),
        );
        row(
            ui,
            "Device dropouts",
            format!("{} ({} recent)", v.xruns, v.recent(Event::Xrun)),
        );
        row(
            ui,
            "Disk too slow",
            format!("{} ({} recent)", v.underruns, v.recent(Event::Underrun)),
        );
    });
}

/// The costliest tracks; returns a track to freeze when its button is pressed.
fn costly_section(ui: &mut Ui, v: &PerformanceView, p: &Project) -> Option<usize> {
    ui.strong("Costliest tracks");
    let mut order: Vec<(usize, f32)> = v
        .cost
        .iter()
        .copied()
        .enumerate()
        .filter(|(t, c)| *c > 0.005 && *t < p.tracks.len())
        .collect();
    order.sort_by(|a, b| b.1.total_cmp(&a.1));
    order.truncate(TOP);
    if order.is_empty() {
        ui.label(RichText::new("No track's plugins or effects are costing much.").weak());
        return None;
    }
    let mut freeze = None;
    egui::Grid::new(egui::Id::unique("perf_cost")).num_columns(3).show(ui, |ui| {
        for (t, cost) in order {
            let track = &p.tracks[t];
            ui.label(&track.name);
            ui.add(
                egui::ProgressBar::new((cost / 0.5).min(1.0))
                    .desired_width(160.0)
                    .text(format!("{:.1}% of a core", cost * 100.0)),
            );
            let can = matches!(
                track.kind,
                TrackKind::Instrument | TrackKind::Audio | TrackKind::Drummer
            );
            let label = if track.frozen.is_some() {
                "Unfreeze"
            } else {
                "Freeze"
            };
            if ui
                .add_enabled(can, egui::Button::new(label).small())
                .on_hover_text(
                    "Plays the track from a file instead of running its plugins and effects. \
                     Its fader, pan and sends still work.",
                )
                .clicked()
            {
                freeze = Some(t);
            }
            ui.end_row();
        }
    });
    freeze
}

/// What to do, from the numbers; returns a buffer size to change to when its button is pressed.
fn advice_section(ui: &mut Ui, v: &PerformanceView, p: &Project) -> Option<u32> {
    ui.strong("Advice");
    let misses = v.recent(Event::Miss) + v.recent(Event::Xrun);
    let peak = v.history.iter().copied().fold(0.0f32, f32::max);
    let heaviest = v
        .cost
        .iter()
        .copied()
        .enumerate()
        .filter(|(t, _)| *t < p.tracks.len())
        .max_by(|a, b| a.1.total_cmp(&b.1));
    let bigger = crate::settings::BUFFERS
        .iter()
        .copied()
        .find(|b| *b > v.block);
    let mut pick = None;
    if misses >= 3 {
        ui.label(format!(
            "The sound missed {misses} deadlines in the last 30 seconds: it may be breaking up."
        ));
        if let Some(b) = bigger {
            if ui
                .button(format!("Use {b} samples"))
                .on_hover_text("A larger buffer gives the engine more time, with a longer delay when playing live.")
                .clicked()
            {
                pick = Some(b);
            }
        }
    } else if peak > 0.85 {
        ui.label("Close to the limit: a heavier passage may break up. A larger buffer or freezing a track gives room.");
    } else if v.underruns > 0 && v.recent(Event::Underrun) > 0 {
        ui.label("The disk could not keep up. Keep projects and samples on an internal SSD.");
    } else {
        ui.label(format!(
            "Running smoothly at {} samples ({:.0}% at most lately).",
            v.block,
            peak * 100.0
        ));
    }
    if let Some((t, cost)) = heaviest {
        if cost > 0.3 && p.tracks[t].frozen.is_none() {
            ui.label(format!(
                "\"{}\" takes {:.0}% of a core: freezing it would free most of that.",
                p.tracks[t].name,
                cost * 100.0
            ));
        }
    }
    if let Some(hw) = &v.hw {
        if v.block > 0 && v.block < hw.tier().buffer() / 2 && misses > 0 {
            ui.label(format!(
                "For this computer, {} samples is a safe place to start.",
                hw.tier().buffer()
            ));
        }
    }
    pick
}

fn write_report(p: &Project, s: &mut UiState) {
    use std::fmt::Write as _;
    let v = &s.performance;
    let mut r = String::from("[ovrtone performance report]\n");
    let _ = writeln!(r, "version={}", env!("CARGO_PKG_VERSION"));
    if let Some(hw) = &v.hw {
        r.push_str(&hw.report());
    }
    let _ = writeln!(r, "buffer={} rate={}", v.block, v.rate);
    let _ = writeln!(r, "output_delay_us={}", v.latency_us);
    let _ = writeln!(r, "callback_worst_us={:.0}", v.session_max.as_secs_f64() * 1e6);
    let _ = writeln!(r, "deadline_us={:.0}", v.budget.as_secs_f64() * 1e6);
    let _ = writeln!(r, "deadline_misses={}", v.misses);
    let _ = writeln!(r, "xruns={}", v.xruns);
    let _ = writeln!(r, "disk_underruns={}", v.underruns);
    let peak = v.history.iter().copied().fold(0.0f32, f32::max);
    let _ = writeln!(r, "load_peak_last_minute={peak:.3}");
    let _ = writeln!(r, "tracks={}", p.tracks.len());
    let plugins: usize = p.tracks.iter().map(|t| t.plugins.len()).sum();
    let effects: usize = p.tracks.iter().map(|t| t.inserts.len()).sum();
    let _ = writeln!(r, "plugins={plugins} stock_effects={effects}");
    for (t, c) in v.cost.iter().enumerate().filter(|(_, c)| **c > 0.01) {
        let _ = writeln!(r, "track_{}_core_share={c:.3}", t + 1);
    }
    let Some(path) = rfd::FileDialog::new()
        .set_file_name("ovrtone-performance.txt")
        .add_filter("Text", &["txt"])
        .save_file()
    else {
        return;
    };
    s.performance.message = Some(match std::fs::write(&path, r) {
        Ok(()) => format!("Saved {}", path.display()),
        Err(e) => e.to_string(),
    });
}
