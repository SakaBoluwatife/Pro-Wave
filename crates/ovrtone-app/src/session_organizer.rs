//! Organize Mix Session (Mix menu): a session of many tracks put in order in one go. Each track is
//! recognised by its name (Kick, Snare, 808, Lead Vox, BGV, Gtr…) and given a group; then, as chosen,
//! every track is coloured by its group, the tracks are sorted group by group, and each group becomes
//! a stack whose tracks play through a bus of its own. What it will do is shown first, and any
//! track's group can be changed before applying. The naming rules can be edited, and are kept.

use egui::{Color32, RichText};

use crate::model::{Project, TrackKind};
use crate::UiState;

const FILE: &str = "session-groups";

/// A group: its name and colour, and the words in a track's name that put it there. A word of up
/// to three letters must be a whole word of the name (OH, HH, LV); a longer one may be anywhere in it.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub name: String,
    pub color: Color32,
    pub words: Vec<String>,
}

fn group(name: &str, rgb: u32, words: &str) -> Group {
    Group {
        name: name.to_string(),
        color: Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8),
        words: words.split(',').map(|w| w.trim().to_lowercase()).filter(|w| !w.is_empty()).collect(),
    }
}

/// The usual groups, in the order the session is sorted in.
pub fn default_groups() -> Vec<Group> {
    vec![
        group(
            "Drums",
            0xd9_4b_3c,
            "kick, kik, bass drum, bassdrum, bd, snare, snr, sd, hat, hihat, hi-hat, hh, tom, overhead, oh, \
             room, perc, percussion, clap, ride, crash, cymbal, shaker, tamb, tambourine, conga, bongo, rim, \
             drum, drums, loop, break, top",
        ),
        group("Bass", 0xe0_8a_2c, "bass, 808, sub, di bass, synth bass, bassline"),
        group("Guitars", 0xd6_c2_3a, "gtr, guitar, acoustic, ac gtr, elec, electric, strat, tele, lead gtr, rhythm"),
        group(
            "Keys & Synths",
            0x4c_b0_5a,
            "piano, pno, keys, rhodes, wurli, organ, synth, pad, pluck, arp, lead synth, bell, mallet, epiano",
        ),
        group("Strings & Orchestra", 0x3a_a6_a0, "string, strings, violin, viola, cello, brass, horn, trumpet, sax, flute, choir, orch"),
        group(
            "Backing Vocals",
            0x8a_5c_d6,
            "bgv, bv, backing, harmony, harm, adlib, ad lib, double, dbl, stack, oohs, aahs, response, gang",
        ),
        group("Lead Vocals", 0xc9_4c_b4, "lead vox, lead vocal, lv, vox, vocal, vocals, voc, main vox, verse, hook, chorus vox, rap"),
        group("FX", 0x5a_7c_d9, "fx, sfx, riser, sweep, impact, noise, reverse, rev, uplifter, downlifter, whoosh, transition"),
    ]
}

/// The group a track's name falls in: the longest of the words found in it wins (so "Bass Drum"
/// is a drum, and "Vox Double" a backing vocal); a tie goes to the group listed first.
pub fn classify(name: &str, groups: &[Group]) -> Option<usize> {
    let lower = name.to_lowercase();
    let tokens: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    let mut best: Option<(usize, usize)> = None;
    for (g, grp) in groups.iter().enumerate() {
        for w in &grp.words {
            let found = if w.len() <= 3 && !w.contains(' ') {
                tokens.iter().any(|t| t == w || t.trim_end_matches(char::is_numeric) == w.as_str())
            } else {
                lower.contains(w.as_str())
            };
            if found && best.is_none_or(|(_, len)| w.len() > len) {
                best = Some((g, w.len()));
            }
        }
    }
    best.map(|(g, _)| g)
}

fn load() -> Vec<Group> {
    let text = crate::recent_files::config_dir()
        .filter(|_| !cfg!(test))
        .and_then(|d| std::fs::read_to_string(d.join(FILE)).ok());
    let Some(text) = text else {
        return default_groups();
    };
    // One group a line: `name | rrggbb | word, word, …`.
    let groups: Vec<Group> = text
        .lines()
        .filter_map(|l| {
            let mut parts = l.splitn(3, '|');
            let name = parts.next()?.trim();
            let rgb = u32::from_str_radix(parts.next()?.trim(), 16).ok()?;
            let words = parts.next().unwrap_or("");
            (!name.is_empty()).then(|| group(name, rgb, words))
        })
        .collect();
    if groups.is_empty() {
        default_groups()
    } else {
        groups
    }
}

fn save(groups: &[Group]) {
    let Some(dir) = crate::recent_files::config_dir().filter(|_| !cfg!(test)) else {
        return;
    };
    let text: String = groups
        .iter()
        .map(|g| {
            let [r, gg, b, _] = g.color.to_array();
            format!("{} | {r:02x}{gg:02x}{b:02x} | {}\n", g.name, g.words.join(", "))
        })
        .collect();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join(FILE), text);
}

#[derive(Default)]
pub struct OrganizerView {
    pub open: bool,
    groups: Vec<Group>,
    /// Each track's group as shown (None: left as it is), by track; made again when the tracks change.
    picks: Vec<Option<usize>>,
    seen_tracks: usize,
    color: bool,
    sort: bool,
    buses: bool,
    gain_after: bool,
    loaded: bool,
    /// The words of each group as being typed in the rules editor.
    words_text: Vec<String>,
}

/// The tracks this works on: those that make sound, not inside a stack already.
fn eligible(p: &Project, i: usize) -> bool {
    let t = &p.tracks[i];
    t.indent == 0
        && matches!(
            t.kind,
            TrackKind::Audio | TrackKind::Instrument | TrackKind::Drummer | TrackKind::ExternalMidi
        )
}

impl OrganizerView {
    fn ready(&mut self, p: &Project) {
        if !self.loaded {
            self.loaded = true;
            self.groups = load();
            self.color = true;
            self.sort = true;
            self.buses = true;
            self.words_text = self.groups.iter().map(|g| g.words.join(", ")).collect();
        }
        if self.seen_tracks != p.tracks.len() || self.picks.len() != p.tracks.len() {
            self.seen_tracks = p.tracks.len();
            self.picks = (0..p.tracks.len())
                .map(|i| eligible(p, i).then(|| classify(&p.tracks[i].name, &self.groups)).flatten())
                .collect();
        }
    }
}

pub fn show(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    if !s.organizer.open {
        return;
    }
    let v = &mut s.organizer;
    v.ready(p);
    let mut open = true;
    let mut apply = false;
    egui::Window::new("Organize Mix Session")
        .id(egui::Id::unique("session_organizer"))
        .open(&mut open)
        .default_width(560.0)
        .show(ctx, |ui| {
            ui.label(
                "Each track is put in a group by its name. Check the groups below, change any you like, then \
                 Organize.",
            );
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut v.color, "Colour tracks by group");
                ui.checkbox(&mut v.sort, "Sort tracks by group");
                ui.checkbox(&mut v.buses, "A bus for each group")
                    .on_hover_text("Each group becomes a stack whose tracks play through an Aux of its own, named and coloured for the group");
                ui.checkbox(&mut v.gain_after, "Then gain stage");
            });
            ui.separator();
            let counts: Vec<usize> = (0..v.groups.len())
                .map(|g| v.picks.iter().filter(|x| **x == Some(g)).count())
                .collect();
            ui.horizontal_wrapped(|ui| {
                for (g, grp) in v.groups.iter().enumerate() {
                    ui.label(RichText::new(format!("■ {} {}", grp.name, counts[g])).color(grp.color));
                }
                let none = (0..p.tracks.len()).filter(|i| eligible(p, *i) && v.picks[*i].is_none()).count();
                ui.label(RichText::new(format!("■ Not recognised {none}")).weak());
            });
            egui::ScrollArea::vertical().max_height(340.0).show(ui, |ui| {
                egui::Grid::new(egui::Id::unique("organizer_grid"))
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        for i in 0..p.tracks.len() {
                            if !eligible(p, i) {
                                continue;
                            }
                            ui.label(&p.tracks[i].name);
                            let now = v.picks[i];
                            let shown = now.map_or("Leave as it is", |g| v.groups[g].name.as_str());
                            egui::ComboBox::from_id_salt(egui::Id::unique(("organizer_pick", i)))
                                .selected_text(shown)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut v.picks[i], None, "Leave as it is");
                                    for (g, grp) in v.groups.iter().enumerate() {
                                        ui.selectable_value(
                                            &mut v.picks[i],
                                            Some(g),
                                            RichText::new(&grp.name).color(grp.color),
                                        );
                                    }
                                });
                            ui.end_row();
                        }
                    });
            });
            egui::CollapsingHeader::new("Naming rules")
                .id_salt(egui::Id::unique("organizer_rules"))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(
                            "The words that put a track in each group, separated by commas. Up to three letters \
                             must be a whole word (OH, HH); longer ones may be anywhere in the name.",
                        )
                        .small(),
                    );
                    let mut changed = false;
                    for (g, grp) in v.groups.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            changed |= ui.color_edit_button_srgba(&mut grp.color).changed();
                            ui.add(egui::TextEdit::singleline(&mut grp.name).desired_width(130.0));
                            if let Some(text) = v.words_text.get_mut(g) {
                                if ui.add(egui::TextEdit::singleline(text).desired_width(300.0)).changed() {
                                    grp.words = text
                                        .split(',')
                                        .map(|w| w.trim().to_lowercase())
                                        .filter(|w| !w.is_empty())
                                        .collect();
                                    changed = true;
                                }
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Use the Usual Rules").clicked() {
                            v.groups = default_groups();
                            v.words_text = v.groups.iter().map(|g| g.words.join(", ")).collect();
                            changed = true;
                        }
                    });
                    if changed {
                        save(&v.groups);
                        // The picks follow the new rules.
                        v.seen_tracks = usize::MAX;
                    }
                });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Organize").clicked() {
                    apply = true;
                }
                ui.label(RichText::new("Edit > Undo puts it all back").weak());
            });
        });
    s.organizer.open = open;
    if apply {
        let made = organize(p, &mut s.organizer);
        s.status = Some(made);
        s.organizer.open = false;
        s.organizer.seen_tracks = usize::MAX;
        if s.organizer.gain_after {
            s.gain_staging.open = true;
        }
    }
}

/// Does it: colours, sorts and stacks as chosen. Says what it did.
fn organize(p: &mut Project, v: &mut OrganizerView) -> String {
    let groups = v.groups.clone();
    let mut picks = v.picks.clone();
    picks.resize(p.tracks.len(), None);
    if v.color {
        for (i, pick) in picks.iter().enumerate() {
            if let Some(g) = pick {
                p.tracks[i].color = groups[*g].color;
            }
        }
    }
    let mut stacks = 0;
    let mut skipped = 0;
    if v.sort || v.buses {
        // Group by group, the tracks are moved up to where the last one ended; the rest follow.
        let mut at = 0;
        for g in 0..groups.len() {
            loop {
                let Some(j) = (at..p.tracks.len()).find(|j| picks[*j] == Some(g) && eligible(p, *j)) else {
                    break;
                };
                if j != at {
                    p.move_tracks(&[j], at);
                    let moved = picks.remove(j);
                    picks.insert(at, moved);
                }
                at += 1;
            }
        }
    }
    if v.buses {
        // From the last group back, so the places of those before stay as they are.
        for g in (0..groups.len()).rev() {
            let members: Vec<usize> = (0..p.tracks.len()).filter(|i| picks.get(*i) == Some(&Some(g))).collect();
            if members.is_empty() {
                continue;
            }
            match p.summing_stack(&members) {
                Some(folder) => {
                    stacks += 1;
                    for k in [folder, folder + 1] {
                        if let Some(t) = p.tracks.get_mut(k) {
                            t.name = groups[g].name.clone();
                            t.color = groups[g].color;
                        }
                    }
                }
                None => skipped += 1,
            }
        }
    }
    p.song_rev += 1;
    let tracks = picks.iter().filter(|x| x.is_some()).count();
    let mut said = format!("Organized {tracks} tracks");
    if stacks > 0 {
        said.push_str(&format!(" into {stacks} buses"));
    }
    if skipped > 0 {
        said.push_str(&format!("; {skipped} groups had no bus free"));
    }
    said
}
