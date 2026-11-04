//! The Piano Roll's bigger note tools: what each does to the notes (no UI, so they can be tested),
//! and the small window that asks for their settings. Behaviour follows what FL Studio's and Lemon
//! Studio's tools do; the code is our own.
//!
//! Every tool takes the notes and the indices to work on, and leaves the notes it changed anywhere
//! in the list, so a selection does not survive it.
use crate::model::Note;
use crate::note_edit::GRIDS;
use egui::Ui;

const SHORTEST: f32 = 1.0 / 64.0;
/// Notes starting this close together count as one chord.
const SAME_START: f32 = 0.002;

fn clamp_vel(v: f32) -> u8 {
    v.round().clamp(1.0, 127.0) as u8
}

/// The targets as chords: groups starting together, each sorted by pitch, the groups by time.
fn chords(notes: &[Note], targets: &[usize]) -> Vec<Vec<Note>> {
    let mut picked: Vec<Note> = targets
        .iter()
        .filter_map(|&i| notes.get(i).copied())
        .collect();
    picked.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
    let mut out: Vec<Vec<Note>> = Vec::new();
    for n in picked {
        match out.last_mut() {
            Some(c) if (c[0].start - n.start).abs() <= SAME_START => c.push(n),
            _ => out.push(vec![n]),
        }
    }
    out
}

/// Takes the targets out of `notes`.
fn without(notes: &mut Vec<Note>, targets: &[usize]) {
    crate::note_edit::remove_notes(notes, targets);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ArpOrder {
    #[default]
    Up,
    Down,
    UpDown,
}

/// Plays each chord one note at a time, for as long as the chord lasted.
pub fn arpeggiate(notes: &mut Vec<Note>, targets: &[usize], step: f32, order: ArpOrder, gate: f32) {
    let step = step.max(SHORTEST);
    let groups = chords(notes, targets);
    without(notes, targets);
    for chord in groups {
        let mut seq = chord.clone();
        match order {
            ArpOrder::Up => {}
            ArpOrder::Down => seq.reverse(),
            ArpOrder::UpDown => {
                let back: Vec<Note> = chord.iter().rev().skip(1).copied().collect();
                seq.extend(back.iter().take(back.len().saturating_sub(1)));
            }
        }
        let start = chord[0].start;
        let len = chord.iter().fold(0f32, |m, n| m.max(n.len));
        let hits = ((len / step).floor() as usize).max(1);
        for k in 0..hits {
            let from = seq[k % seq.len()];
            notes.push(Note {
                start: start + k as f32 * step,
                len: (step * gate).max(SHORTEST),
                ..from
            });
        }
    }
}

/// Spreads each chord out in time, as a strum across strings, all ending where they did.
pub fn strum(notes: &mut [Note], targets: &[usize], spread: f32, down: bool) {
    let mut order: Vec<usize> = targets
        .iter()
        .copied()
        .filter(|&i| i < notes.len())
        .collect();
    order.sort_by(|&a, &b| {
        notes[a]
            .start
            .total_cmp(&notes[b].start)
            .then(notes[a].pitch.cmp(&notes[b].pitch))
    });
    let mut at = 0;
    while at < order.len() {
        let first = notes[order[at]].start;
        let size = order[at..]
            .iter()
            .take_while(|&&i| (notes[i].start - first).abs() <= SAME_START)
            .count();
        for (place, &i) in order[at..at + size].iter().enumerate() {
            let turn = if down { size - 1 - place } else { place };
            let delay = turn as f32 * spread;
            let n = &mut notes[i];
            n.start += delay;
            n.len = (n.len - delay).max(SHORTEST);
        }
        at += size;
    }
}

/// Puts a quieter grace note just before each note.
pub fn flam(notes: &mut Vec<Note>, targets: &[usize], offset: f32, level: f32) {
    let offset = offset.max(SHORTEST);
    let graces: Vec<Note> = targets
        .iter()
        .filter_map(|&i| notes.get(i).copied())
        .filter(|n| n.start >= offset)
        .map(|n| Note {
            start: n.start - offset,
            len: offset,
            vel: clamp_vel(f32::from(n.vel) * level),
            ..n
        })
        .collect();
    notes.extend(graces);
}

/// Turns each note into repeated hits, each `decay` of the one before.
pub fn note_repeat(notes: &mut Vec<Note>, targets: &[usize], rate: f32, gate: f32, decay: f32) {
    let rate = rate.max(SHORTEST);
    let picked: Vec<Note> = targets
        .iter()
        .filter_map(|&i| notes.get(i).copied())
        .collect();
    without(notes, targets);
    for n in picked {
        let hits = ((n.len / rate).ceil() as usize).max(1);
        for k in 0..hits {
            notes.push(Note {
                start: n.start + k as f32 * rate,
                len: (rate * gate).max(SHORTEST),
                vel: clamp_vel(f32::from(n.vel) * decay.powi(k as i32)),
                ..n
            });
        }
    }
}

/// Cuts each note into steps and keeps `keep` of every `keep + skip`.
pub fn claw(notes: &mut Vec<Note>, targets: &[usize], step: f32, keep: usize, skip: usize) {
    let step = step.max(SHORTEST);
    let (keep, cycle) = (keep.max(1), keep.max(1) + skip);
    let picked: Vec<Note> = targets
        .iter()
        .filter_map(|&i| notes.get(i).copied())
        .collect();
    without(notes, targets);
    for n in picked {
        let hits = ((n.len / step).ceil() as usize).max(1);
        for k in (0..hits).filter(|k| k % cycle < keep) {
            let start = n.start + k as f32 * step;
            notes.push(Note {
                start,
                len: step.min(n.start + n.len - start).max(SHORTEST),
                ..n
            });
        }
    }
}

/// Moves notes by octaves until they are within `low..=high`.
pub fn limit(notes: &mut [Note], targets: &[usize], low: u8, high: u8) {
    let (low, high) = (i32::from(low.min(high)), i32::from(low.max(high)));
    for &i in targets {
        if let Some(n) = notes.get_mut(i) {
            let mut pitch = i32::from(n.pitch);
            while pitch > high && pitch - 12 >= 0 {
                pitch -= 12;
            }
            while pitch < low && pitch + 12 <= 127 {
                pitch += 12;
            }
            // A range under an octave may have no such pitch: the nearest will do.
            n.pitch = pitch.clamp(low, high) as u8;
        }
    }
}

/// Mirrors the targets in time, within the span they take up.
pub fn flip_time(notes: &mut [Note], targets: &[usize]) {
    let picked = || targets.iter().filter_map(|&i| notes.get(i));
    let first = picked().fold(f32::MAX, |m, n| m.min(n.start));
    let last = picked().fold(0f32, |m, n| m.max(n.start + n.len));
    for &i in targets {
        if let Some(n) = notes.get_mut(i) {
            n.start = first + last - (n.start + n.len);
        }
    }
}

/// Mirrors the targets in pitch, within the range they take up.
pub fn flip_pitch(notes: &mut [Note], targets: &[usize]) {
    let picked = || targets.iter().filter_map(|&i| notes.get(i));
    let low = picked().map(|n| n.pitch).min().unwrap_or(0);
    let high = picked().map(|n| n.pitch).max().unwrap_or(0);
    for &i in targets {
        if let Some(n) = notes.get_mut(i) {
            n.pitch = low + high - n.pitch;
        }
    }
}

/// Scales the velocities, then adds to them.
pub fn levels(notes: &mut [Note], targets: &[usize], scale: f32, add: f32) {
    for &i in targets {
        if let Some(n) = notes.get_mut(i) {
            n.vel = clamp_vel(f32::from(n.vel) * scale + add);
        }
    }
}

/// Moves velocities up and down in a wave over time, one wave every `period` beats.
pub fn velocity_lfo(notes: &mut [Note], targets: &[usize], period: f32, depth: f32) {
    let period = period.max(SHORTEST);
    for &i in targets {
        if let Some(n) = notes.get_mut(i) {
            let wave = (n.start / period * std::f32::consts::TAU).sin();
            n.vel = clamp_vel(f32::from(n.vel) * (1.0 + depth * wave));
        }
    }
}

/// Varies velocity, timing and pitch by up to these amounts. The same seed gives the same result.
pub fn randomize(
    notes: &mut [Note],
    targets: &[usize],
    seed: u32,
    velocity: f32,
    timing: f32,
    pitch: i32,
) {
    let mut x = seed.max(1);
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        // -1..1
        x as f32 / u32::MAX as f32 * 2.0 - 1.0
    };
    for &i in targets {
        let (v, t, q) = (next(), next(), next());
        if let Some(n) = notes.get_mut(i) {
            n.vel = clamp_vel(f32::from(n.vel) + v * velocity);
            n.start = (n.start + t * timing).max(0.0);
            let moved = i32::from(n.pitch) + (q * pitch as f32).round() as i32;
            n.pitch = moved.clamp(0, 127) as u8;
        }
    }
}

/// Chord shapes for [`chord_stamp`]: name and semitones above the note.
pub const CHORDS: [(&str, &[u8]); 10] = [
    ("Major", &[4, 7]),
    ("Minor", &[3, 7]),
    ("Dominant 7", &[4, 7, 10]),
    ("Major 7", &[4, 7, 11]),
    ("Minor 7", &[3, 7, 10]),
    ("Sus 2", &[2, 7]),
    ("Sus 4", &[5, 7]),
    ("Diminished", &[3, 6]),
    ("Augmented", &[4, 8]),
    ("Power", &[7, 12]),
];

/// Builds a chord on each note.
pub fn chord_stamp(notes: &mut Vec<Note>, targets: &[usize], intervals: &[u8]) {
    let mut added = Vec::new();
    for n in targets.iter().filter_map(|&i| notes.get(i)) {
        for &up in intervals {
            if let Some(pitch) = n.pitch.checked_add(up).filter(|p| *p <= 127) {
                added.push(Note { pitch, ..*n });
            }
        }
    }
    notes.extend(added);
}

/// Every note of a pitch the selected notes have.
pub fn same_pitch(notes: &[Note], selected: &[usize]) -> Vec<usize> {
    let pitches: Vec<u8> = selected
        .iter()
        .filter_map(|&i| notes.get(i).map(|n| n.pitch))
        .collect();
    (0..notes.len())
        .filter(|&i| pitches.contains(&notes[i].pitch))
        .collect()
}

/// Every note starting where a selected note does.
pub fn same_start(notes: &[Note], selected: &[usize]) -> Vec<usize> {
    let starts: Vec<f32> = selected
        .iter()
        .filter_map(|&i| notes.get(i).map(|n| n.start))
        .collect();
    (0..notes.len())
        .filter(|&i| {
            starts
                .iter()
                .any(|s| (notes[i].start - s).abs() <= SAME_START)
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteTool {
    Arpeggiate,
    Strum,
    Flam,
    NoteRepeat,
    Claw,
    Limit,
    Levels,
    VelocityLfo,
    Randomize,
    Chord,
}

impl NoteTool {
    pub const ALL: [NoteTool; 10] = [
        NoteTool::Arpeggiate,
        NoteTool::Strum,
        NoteTool::Flam,
        NoteTool::NoteRepeat,
        NoteTool::Claw,
        NoteTool::Limit,
        NoteTool::Levels,
        NoteTool::VelocityLfo,
        NoteTool::Randomize,
        NoteTool::Chord,
    ];

    pub fn name(self) -> &'static str {
        match self {
            NoteTool::Arpeggiate => "Arpeggiate…",
            NoteTool::Strum => "Strum…",
            NoteTool::Flam => "Flam…",
            NoteTool::NoteRepeat => "Note Repeat…",
            NoteTool::Claw => "Claw…",
            NoteTool::Limit => "Limit…",
            NoteTool::Levels => "Levels…",
            NoteTool::VelocityLfo => "Velocity LFO…",
            NoteTool::Randomize => "Randomize…",
            NoteTool::Chord => "Chord Stamp…",
        }
    }
}

/// The settings of the tools, kept between uses.
pub struct ToolSettings {
    pub open: Option<NoteTool>,
    grid: usize,
    gate: f32,
    order: ArpOrder,
    spread: f32,
    down: bool,
    level: f32,
    decay: f32,
    keep: usize,
    skip: usize,
    low: u8,
    high: u8,
    scale: f32,
    add: f32,
    period: f32,
    depth: f32,
    seed: u32,
    vel_amount: f32,
    timing: f32,
    pitch_amount: i32,
    chord: usize,
}

impl Default for ToolSettings {
    fn default() -> Self {
        Self {
            open: None,
            grid: 3,
            gate: 0.9,
            order: ArpOrder::Up,
            spread: 0.05,
            down: false,
            level: 0.6,
            decay: 1.0,
            keep: 1,
            skip: 1,
            low: 48,
            high: 72,
            scale: 1.0,
            add: 0.0,
            period: 4.0,
            depth: 0.3,
            seed: 1,
            vel_amount: 20.0,
            timing: 0.0,
            pitch_amount: 0,
            chord: 0,
        }
    }
}

impl ToolSettings {
    fn step(&self) -> f32 {
        GRIDS[self.grid.min(GRIDS.len() - 1)].1
    }

    /// Runs the open tool on `targets`.
    pub fn apply(&self, notes: &mut Vec<Note>, targets: &[usize]) {
        let step = self.step();
        match self.open {
            Some(NoteTool::Arpeggiate) => {
                arpeggiate(notes, targets, step, self.order, self.gate);
            }
            Some(NoteTool::Strum) => strum(notes, targets, self.spread, self.down),
            Some(NoteTool::Flam) => flam(notes, targets, step, self.level),
            Some(NoteTool::NoteRepeat) => {
                note_repeat(notes, targets, step, self.gate, self.decay);
            }
            Some(NoteTool::Claw) => claw(notes, targets, step, self.keep, self.skip),
            Some(NoteTool::Limit) => limit(notes, targets, self.low, self.high),
            Some(NoteTool::Levels) => levels(notes, targets, self.scale, self.add),
            Some(NoteTool::VelocityLfo) => velocity_lfo(notes, targets, self.period, self.depth),
            Some(NoteTool::Randomize) => randomize(
                notes,
                targets,
                self.seed,
                self.vel_amount,
                self.timing,
                self.pitch_amount,
            ),
            Some(NoteTool::Chord) => chord_stamp(notes, targets, CHORDS[self.chord].1),
            None => {}
        }
    }

    fn step_row(&mut self, ui: &mut Ui) {
        egui::ComboBox::from_label("Step")
            .selected_text(GRIDS[self.grid.min(GRIDS.len() - 1)].0)
            .show_ui(ui, |ui| {
                for (i, (name, _)) in GRIDS.iter().enumerate() {
                    ui.selectable_value(&mut self.grid, i, *name);
                }
            });
    }

    fn controls(&mut self, ui: &mut Ui, tool: NoteTool) {
        let percent = |ui: &mut Ui, value: &mut f32, name: &str, max: f32| {
            ui.add(egui::Slider::new(value, 0.0..=max).text(name));
        };
        match tool {
            NoteTool::Arpeggiate => {
                self.step_row(ui);
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.order, ArpOrder::Up, "Up");
                    ui.selectable_value(&mut self.order, ArpOrder::Down, "Down");
                    ui.selectable_value(&mut self.order, ArpOrder::UpDown, "Up and down");
                });
                percent(ui, &mut self.gate, "Length", 1.0);
            }
            NoteTool::Strum => {
                percent(ui, &mut self.spread, "Spread (beats)", 0.5);
                ui.checkbox(&mut self.down, "Top note first");
            }
            NoteTool::Flam => {
                self.step_row(ui);
                percent(ui, &mut self.level, "Grace note level", 1.0);
            }
            NoteTool::NoteRepeat => {
                self.step_row(ui);
                percent(ui, &mut self.gate, "Length", 1.0);
                percent(ui, &mut self.decay, "Each hit's level", 1.0);
            }
            NoteTool::Claw => {
                self.step_row(ui);
                ui.add(egui::Slider::new(&mut self.keep, 1..=8).text("Keep"));
                ui.add(egui::Slider::new(&mut self.skip, 0..=8).text("Skip"));
            }
            NoteTool::Limit => {
                ui.add(egui::Slider::new(&mut self.low, 0..=127).text("Lowest pitch"));
                ui.add(egui::Slider::new(&mut self.high, 0..=127).text("Highest pitch"));
            }
            NoteTool::Levels => {
                percent(ui, &mut self.scale, "Scale", 2.0);
                ui.add(egui::Slider::new(&mut self.add, -64.0..=64.0).text("Add"));
            }
            NoteTool::VelocityLfo => {
                ui.add(egui::Slider::new(&mut self.period, 0.25..=32.0).text("Beats a wave"));
                percent(ui, &mut self.depth, "Depth", 1.0);
            }
            NoteTool::Randomize => {
                percent(ui, &mut self.vel_amount, "Velocity", 64.0);
                percent(ui, &mut self.timing, "Timing (beats)", 0.25);
                ui.add(egui::Slider::new(&mut self.pitch_amount, 0..=12).text("Pitch"));
                if ui.button("New variation").clicked() {
                    self.seed = self.seed.wrapping_mul(31).wrapping_add(7);
                }
            }
            NoteTool::Chord => {
                egui::ComboBox::from_label("Chord")
                    .selected_text(CHORDS[self.chord].0)
                    .show_ui(ui, |ui| {
                        for (i, (name, _)) in CHORDS.iter().enumerate() {
                            ui.selectable_value(&mut self.chord, i, *name);
                        }
                    });
            }
        }
    }

    /// The open tool's window. True when Apply was pressed.
    pub fn window(&mut self, ctx: &egui::Context) -> bool {
        let Some(tool) = self.open else {
            return false;
        };
        let (mut apply, mut close) = (false, false);
        egui::Window::new(tool.name().trim_end_matches('…'))
            .id(egui::Id::unique("note_tool_window"))
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                self.controls(ui, tool);
                ui.label(
                    egui::RichText::new("Works on the selected notes, or all if none are.").small(),
                );
                ui.horizontal(|ui| {
                    apply = ui.button("Apply").clicked();
                    close = ui.button("Close").clicked();
                });
            });
        if close {
            self.open = None;
        }
        apply
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(start: f32, len: f32, pitch: u8) -> Note {
        Note {
            start,
            len,
            pitch,
            vel: 100,
            muted: false,
        }
    }

    fn all(notes: &[Note]) -> Vec<usize> {
        (0..notes.len()).collect()
    }

    fn sorted(notes: &[Note]) -> Vec<(f32, u8)> {
        let mut out: Vec<_> = notes.iter().map(|n| (n.start, n.pitch)).collect();
        out.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        out
    }

    #[test]
    fn a_chord_arpeggiates_in_order_for_as_long_as_it_lasted() {
        let chord = vec![note(1.0, 1.0, 64), note(1.0, 1.0, 60), note(1.0, 1.0, 67)];
        let mut up = chord.clone();
        arpeggiate(&mut up, &[0, 1, 2], 0.25, ArpOrder::Up, 1.0);
        assert_eq!(sorted(&up), [(1.0, 60), (1.25, 64), (1.5, 67), (1.75, 60)]);
        let mut down = chord.clone();
        arpeggiate(&mut down, &[0, 1, 2], 0.25, ArpOrder::Down, 1.0);
        assert_eq!(sorted(&down)[0], (1.0, 67));
        let mut both = chord;
        arpeggiate(&mut both, &[0, 1, 2], 0.25, ArpOrder::UpDown, 1.0);
        assert_eq!(
            sorted(&both),
            [(1.0, 60), (1.25, 64), (1.5, 67), (1.75, 64)]
        );
    }

    #[test]
    fn notes_left_out_are_left_alone() {
        let mut notes = vec![note(0.0, 1.0, 60), note(4.0, 1.0, 72)];
        arpeggiate(&mut notes, &[0], 0.5, ArpOrder::Up, 1.0);
        assert!(notes.iter().any(|n| n.start == 4.0 && n.pitch == 72));
        assert_eq!(notes.len(), 3);
    }

    #[test]
    fn a_strum_spreads_a_chord_and_keeps_its_end() {
        let mut notes = vec![note(0.0, 1.0, 60), note(0.0, 1.0, 64), note(2.0, 1.0, 50)];
        strum(&mut notes, &[0, 1, 2], 0.1, false);
        assert_eq!(notes[0].start, 0.0);
        assert!((notes[1].start - 0.1).abs() < 1e-6);
        assert!((notes[1].start + notes[1].len - 1.0).abs() < 1e-6);
        assert_eq!(notes[2].start, 2.0, "a chord of its own");
        let mut top_first = vec![note(0.0, 1.0, 60), note(0.0, 1.0, 64)];
        strum(&mut top_first, &[0, 1], 0.1, true);
        assert!(top_first[1].start < top_first[0].start);
    }

    #[test]
    fn a_flam_adds_a_quiet_note_before_unless_there_is_no_room() {
        let mut notes = vec![note(1.0, 1.0, 60), note(0.0, 1.0, 62)];
        flam(&mut notes, &[0, 1], 0.25, 0.5);
        assert_eq!(notes.len(), 3);
        assert_eq!(
            (notes[2].start, notes[2].pitch, notes[2].vel),
            (0.75, 60, 50)
        );
    }

    #[test]
    fn a_note_repeats_and_fades() {
        let mut notes = vec![note(0.0, 1.0, 60)];
        note_repeat(&mut notes, &[0], 0.25, 1.0, 0.5);
        assert_eq!(notes.len(), 4);
        assert_eq!(notes[1].vel, 50);
        assert_eq!(notes[3].start, 0.75);
    }

    #[test]
    fn a_claw_keeps_some_of_the_steps() {
        let mut notes = vec![note(0.0, 2.0, 60)];
        claw(&mut notes, &[0], 0.25, 1, 1);
        let starts: Vec<f32> = notes.iter().map(|n| n.start).collect();
        assert_eq!(starts, [0.0, 0.5, 1.0, 1.5]);
    }

    #[test]
    fn limit_moves_by_octaves_into_the_range() {
        let mut notes = vec![note(0.0, 1.0, 96), note(0.0, 1.0, 24), note(0.0, 1.0, 60)];
        limit(&mut notes, &[0, 1, 2], 48, 72);
        assert_eq!(notes[0].pitch, 72);
        assert_eq!(notes[1].pitch, 48);
        assert_eq!(notes[2].pitch, 60);
    }

    #[test]
    fn flips_mirror_and_twice_gives_back_the_same() {
        let start = vec![note(0.0, 1.0, 60), note(3.0, 1.0, 72)];
        let mut notes = start.clone();
        flip_time(&mut notes, &[0, 1]);
        assert_eq!(notes[0].start, 3.0);
        assert_eq!(notes[1].start, 0.0);
        flip_time(&mut notes, &[0, 1]);
        assert_eq!(notes, start);
        flip_pitch(&mut notes, &[0, 1]);
        assert_eq!((notes[0].pitch, notes[1].pitch), (72, 60));
    }

    #[test]
    fn levels_and_the_lfo_stay_in_range() {
        let mut notes = vec![note(0.0, 1.0, 60), note(1.0, 1.0, 60)];
        levels(&mut notes, &[0], 2.0, 0.0);
        assert_eq!(notes[0].vel, 127);
        levels(&mut notes, &[0], 0.0, 0.0);
        assert_eq!(notes[0].vel, 1, "a note stays audible");
        velocity_lfo(&mut notes, &[1], 4.0, 0.5);
        assert_eq!(
            notes[1].vel, 127,
            "a quarter through the wave, at its top: 150 cut to 127"
        );
    }

    #[test]
    fn randomize_repeats_for_a_seed_and_stays_valid() {
        let start = vec![note(0.0, 1.0, 0), note(1.0, 1.0, 127)];
        let (mut a, mut b, mut c) = (start.clone(), start.clone(), start.clone());
        randomize(&mut a, &[0, 1], 5, 30.0, 0.1, 12);
        randomize(&mut b, &[0, 1], 5, 30.0, 0.1, 12);
        randomize(&mut c, &[0, 1], 6, 30.0, 0.1, 12);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(a
            .iter()
            .all(|n| n.start >= 0.0 && n.pitch <= 127 && n.vel >= 1));
    }

    #[test]
    fn a_chord_is_stamped_and_stays_on_the_keyboard() {
        let mut notes = vec![note(0.0, 1.0, 60), note(1.0, 1.0, 125)];
        chord_stamp(&mut notes, &[0, 1], CHORDS[0].1);
        assert_eq!(notes.len(), 4, "125 has no room for a third or a fifth");
        assert!(notes.iter().any(|n| n.pitch == 67 && n.start == 0.0));
    }

    #[test]
    fn selecting_by_pitch_and_by_position() {
        let notes = vec![note(0.0, 1.0, 60), note(1.0, 1.0, 60), note(1.0, 1.0, 64)];
        assert_eq!(same_pitch(&notes, &[0]), [0, 1]);
        assert_eq!(same_start(&notes, &[1]), [1, 2]);
        assert_eq!(all(&notes).len(), 3);
    }

    #[test]
    fn every_tool_runs_from_its_settings() {
        for tool in NoteTool::ALL {
            let settings = ToolSettings {
                open: Some(tool),
                ..ToolSettings::default()
            };
            let mut notes = vec![note(1.0, 1.0, 60), note(1.0, 1.0, 64)];
            let targets = all(&notes);
            settings.apply(&mut notes, &targets);
            assert!(!notes.is_empty(), "{tool:?}");
            assert!(notes.iter().all(|n| n.len > 0.0 && n.pitch <= 127));
        }
    }
}
