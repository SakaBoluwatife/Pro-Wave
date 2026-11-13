//! The room's speakers (Mix > Spatial Audio > Speaker Setup): where each really is, and the level
//! and delay that line them up at the listening place. It belongs to this computer's room, not to a
//! project, so it is kept in the settings folder, and no project changes when it does.

use std::sync::Mutex;

use egui::{vec2, Ui};
use ovrtone_engine::{SpeakerLayout, SpeakerSetup, CHANNEL_NAMES, LFE_CHANNEL, MAX_SPEAKER_DELAY_MS, SPATIAL_CHANNELS};

const FILE: &str = "speaker-setup";

/// The setup, and a count that goes up each time it changes (the engine sync watches it).
static SETUP: Mutex<Option<(SpeakerSetup, u64)>> = Mutex::new(None);

/// The setup now, and its change count.
pub fn current() -> (SpeakerSetup, u64) {
    let Ok(mut held) = SETUP.lock() else {
        return (SpeakerSetup::default(), 0);
    };
    *held.get_or_insert_with(|| (load(), 1))
}

fn set(setup: SpeakerSetup) {
    if let Ok(mut held) = SETUP.lock() {
        let generation = held.map_or(1, |(_, g)| g + 1);
        *held = Some((setup, generation));
    }
    save(&setup);
}

fn path() -> Option<std::path::PathBuf> {
    crate::recent_files::config_dir()
        .filter(|_| !cfg!(test))
        .map(|d| d.join(FILE))
}

/// One line a channel: `channel azimuth elevation gain_db delay_ms`, with `-` for a place the layout
/// gives.
fn load() -> SpeakerSetup {
    let mut s = SpeakerSetup::default();
    let Some(text) = path().and_then(|p| std::fs::read_to_string(p).ok()) else {
        return s;
    };
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        let [c, az, el, gain, delay] = parts[..] else {
            continue;
        };
        let Some(c) = c.parse::<usize>().ok().filter(|c| *c < SPATIAL_CHANNELS) else {
            continue;
        };
        if let (Ok(az), Ok(el)) = (az.parse::<f32>(), el.parse::<f32>()) {
            s.place[c] = Some((az.clamp(-180.0, 180.0), el.clamp(-90.0, 90.0)));
        }
        s.gain_db[c] = gain.parse::<f32>().unwrap_or(0.0).clamp(-24.0, 12.0);
        s.delay_ms[c] = delay.parse::<f32>().unwrap_or(0.0).clamp(0.0, MAX_SPEAKER_DELAY_MS);
    }
    s
}

fn save(s: &SpeakerSetup) {
    let Some(path) = path() else {
        return;
    };
    let mut text = String::new();
    for c in 0..SPATIAL_CHANNELS {
        let (az, el) = match s.place[c] {
            Some((a, e)) => (format!("{a}"), format!("{e}")),
            None => ("-".to_string(), "-".to_string()),
        };
        text.push_str(&format!("{c} {az} {el} {} {}\n", s.gain_db[c], s.delay_ms[c]));
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&path, text) {
        tracing::warn!("cannot save the speaker setup: {e}");
    }
}

/// The Speaker Setup window, for the speakers of `layout`. `test` is the channel playing the test
/// signal (a click on a channel's Test plays it there; again stops it).
pub fn show(ctx: &egui::Context, open: &mut bool, layout: SpeakerLayout, test: &mut Option<usize>) {
    if !*open {
        return;
    }
    let (mut s, _) = current();
    let before = s;
    egui::Window::new("Speaker Setup")
        .id(egui::Id::unique("speaker_setup"))
        .open(open)
        .resizable(false)
        .show(ctx, |ui| {
            ui.label(format!(
                "The speakers of your room for {}: where each really is, and the level and delay that line \
                 them up where you sit. Used when the mix goes straight out to speakers.",
                layout.name()
            ));
            ui.add_space(4.0);
            egui::Grid::new(egui::Id::unique("speaker_grid"))
                .num_columns(7)
                .spacing(vec2(8.0, 4.0))
                .show(ui, |ui| {
                    for h in ["Speaker", "Direction", "Elevation", "", "Level", "Delay", ""] {
                        ui.label(h);
                    }
                    ui.end_row();
                    for &c in layout.device_order() {
                        row(ui, &mut s, layout, c, test);
                        ui.end_row();
                    }
                });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("Reset All").clicked() {
                    s = SpeakerSetup::default();
                    *test = None;
                }
                if test.is_some() && ui.button("Stop Test").clicked() {
                    *test = None;
                }
            });
        });
    if s != before {
        set(s);
    }
}

fn row(ui: &mut Ui, s: &mut SpeakerSetup, layout: SpeakerLayout, c: usize, test: &mut Option<usize>) {
    ui.label(CHANNEL_NAMES[c]);
    match layout.place(c) {
        Some(base) => {
            let (mut az, mut el) = s.place[c].unwrap_or(base);
            let a = ui.add(egui::DragValue::new(&mut az).range(-180.0..=180.0).suffix("°").speed(0.5));
            let e = ui.add(egui::DragValue::new(&mut el).range(-90.0..=90.0).suffix("°").speed(0.5));
            if a.changed() || e.changed() {
                s.place[c] = Some((az, el));
            }
            if ui
                .add_enabled(s.place[c].is_some(), egui::Button::new("Standard").small())
                .on_hover_text("Back to where the layout puts it")
                .clicked()
            {
                s.place[c] = None;
            }
        }
        None => {
            ui.label(if c == LFE_CHANNEL { "LFE" } else { "" });
            ui.label("");
            ui.label("");
        }
    }
    ui.add(
        egui::DragValue::new(&mut s.gain_db[c])
            .range(-24.0..=12.0)
            .suffix(" dB")
            .speed(0.1),
    );
    ui.add(
        egui::DragValue::new(&mut s.delay_ms[c])
            .range(0.0..=MAX_SPEAKER_DELAY_MS)
            .suffix(" ms")
            .speed(0.05),
    )
    .on_hover_text("Delay a nearer speaker so its sound arrives with the furthest: about 2.9 ms a metre");
    let on = *test == Some(c);
    if ui
        .selectable_label(on, "Test")
        .on_hover_text("Pink noise on this speaker, to check it is the right one and set its level")
        .clicked()
    {
        *test = if on { None } else { Some(c) };
    }
}
