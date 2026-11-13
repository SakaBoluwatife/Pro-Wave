//! Mix > Routing Matrix: every strip against every bus, in one place. A cell can be the strip's
//! output (its whole sound goes there instead of Stereo Out) or a send (a copy goes there, at the
//! send's level). A bus is heard through the Aux strip of its number.
use crate::model::{Project, SendSlot, TrackKind, MAX_SENDS};
use crate::UiState;
use egui::{vec2, Color32, Sense};

/// Whether track `t` sends to `bus`.
pub fn sends_to(p: &Project, t: usize, bus: u16) -> bool {
    p.tracks
        .get(t)
        .is_some_and(|x| x.sends.iter().any(|s| s.bus == Some(bus)))
}

/// Starts or stops a send. The slots are full at [`MAX_SENDS`]: false then.
pub fn toggle_send(p: &mut Project, t: usize, bus: u16) -> Result<(), String> {
    let loops = p.bus_would_loop(t, bus);
    let Some(track) = p.tracks.get_mut(t) else {
        return Err("No such track".to_string());
    };
    if let Some(at) = track.sends.iter().position(|s| s.bus == Some(bus)) {
        track.sends.remove(at);
    } else {
        if loops {
            return Err(format!(
                "Bus {bus} would bring {} back to itself",
                track.name
            ));
        }
        if track.sends.len() >= MAX_SENDS {
            return Err(format!("{} has no free send", track.name));
        }
        track.sends.push(SendSlot {
            bus: Some(bus),
            level: 0.75,
            pre: false,
        });
    }
    p.song_rev += 1;
    Ok(())
}

/// Sends the strip's output to `bus`, or back to Stereo Out if it was there already.
pub fn toggle_output(p: &mut Project, t: usize, bus: u16) -> Result<(), String> {
    let loops = p.bus_would_loop(t, bus);
    let track = p.tracks.get_mut(t).ok_or("No such track")?;
    if track.output == bus {
        track.output = 0;
    } else if loops {
        return Err(format!(
            "Bus {bus} would bring {} back to itself",
            track.name
        ));
    } else {
        track.output = bus;
    }
    p.song_rev += 1;
    Ok(())
}

/// How many bus columns to show: at least the first eight, and as far as anything uses one, since an
/// Aux can be on any bus and a strip can play into one with no Aux on it.
fn columns(p: &Project) -> u16 {
    let auxes = p.aux_buses().into_iter().map(|(bus, _)| bus);
    let used = p.tracks.iter().flat_map(|t| {
        t.sends
            .iter()
            .filter_map(|s| s.bus)
            .chain(std::iter::once(t.output))
    });
    auxes.chain(used).max().unwrap_or(0).max(8)
}

/// The window.
pub fn show(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    if !s.dialogs.routing {
        return;
    }
    let mut open = true;
    let mut message = None;
    let buses = columns(p);
    egui::Window::new("Routing Matrix")
        .id(egui::Id::unique("routing_matrix"))
        .open(&mut open)
        .default_width(520.0)
        .show(ctx, |ui| {
            ui.label("Click: send a copy to that bus. Right-click: make it the strip's output.");
            ui.add_space(4.0);
            egui::ScrollArea::both().max_height(420.0).show(ui, |ui| {
                egui::Grid::new(egui::Id::unique("routing_grid"))
                    .spacing(vec2(3.0, 3.0))
                    .show(ui, |ui| {
                        ui.label("");
                        for bus in 1..=buses {
                            ui.label(format!("Bus {bus}"));
                        }
                        ui.end_row();
                        for t in 0..p.tracks.len() {
                            let kind = p.tracks[t].kind;
                            if matches!(kind, TrackKind::Folder | TrackKind::Vca) {
                                continue;
                            }
                            ui.label(p.tracks[t].name.clone());
                            let listens = p.aux_bus_of(t);
                            for bus in 1..=buses {
                                let (rect, resp) =
                                    ui.allocate_exact_size(vec2(44.0, 20.0), Sense::click());
                                let out = p.tracks[t].output == bus;
                                let send = sends_to(p, t, bus);
                                let (fill, text) = if listens == bus {
                                    (Color32::from_rgb(0x2f, 0x7a, 0x70), "In")
                                } else if out {
                                    (Color32::from_rgb(0x36, 0x6f, 0xd2), "Out")
                                } else if send {
                                    (Color32::from_rgb(0x8a, 0x6a, 0x2a), "Send")
                                } else {
                                    (crate::theme::gray(0x1c), "")
                                };
                                let g = ui.painter_at(rect);
                                g.rect_filled(rect, 2.0, fill);
                                g.text(
                                    rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    text,
                                    egui::FontId::proportional(10.5),
                                    Color32::WHITE,
                                );
                                // An Aux cannot feed the bus it listens to.
                                if listens != bus {
                                    if resp.clicked() {
                                        if let Err(why) = toggle_send(p, t, bus) {
                                            message = Some(why);
                                        }
                                    }
                                    if resp.secondary_clicked() {
                                        if let Err(why) = toggle_output(p, t, bus) {
                                            message = Some(why);
                                        }
                                    }
                                }
                            }
                            ui.end_row();
                        }
                    });
            });
        });
    if let Some(message) = message {
        s.status = Some(message);
    }
    s.dialogs.routing = open;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sends_and_outputs_switch_on_and_off() {
        let mut p = Project::empty();
        let t = p.add_track(TrackKind::Audio);
        assert!(toggle_send(&mut p, t, 2).is_ok());
        assert!(sends_to(&p, t, 2));
        assert!(toggle_send(&mut p, t, 2).is_ok());
        assert!(!sends_to(&p, t, 2));
        toggle_output(&mut p, t, 3).unwrap();
        assert_eq!(p.tracks[t].output, 3);
        toggle_output(&mut p, t, 3).unwrap();
        assert_eq!(p.tracks[t].output, 0, "back to Stereo Out");
        for bus in 1..=MAX_SENDS as u16 {
            toggle_send(&mut p, t, bus).unwrap();
        }
        let full = toggle_send(&mut p, t, MAX_SENDS as u16 + 1).unwrap_err();
        assert!(full.contains("no free send"), "{full}");
        // One already there still comes off.
        toggle_send(&mut p, t, 1).unwrap();
        assert!(!sends_to(&p, t, 1));
    }

    #[test]
    fn it_will_not_make_a_strip_feed_itself() {
        let mut p = Project::empty();
        let a = p.add_track(TrackKind::Aux);
        let b = p.add_track(TrackKind::Aux);
        let (bus_a, bus_b) = (p.aux_bus_of(a), p.aux_bus_of(b));
        // A into B is fine, and then B back into A would go round for ever.
        toggle_output(&mut p, a, bus_b).unwrap();
        assert!(toggle_send(&mut p, b, bus_a).is_err());
        assert!(toggle_output(&mut p, b, bus_a).is_err());
        assert_eq!(p.tracks[b].output, 0);
        assert!(p.tracks[b].sends.is_empty());
        // Taking the first away makes room for the other.
        toggle_output(&mut p, a, bus_b).unwrap();
        toggle_output(&mut p, b, bus_a).unwrap();
    }

    #[test]
    fn every_bus_in_use_has_its_column() {
        let mut p = Project::empty();
        assert_eq!(columns(&p), 8, "the first eight at least");
        let a = p.add_track(TrackKind::Aux);
        p.set_aux_bus(a, 40);
        assert_eq!(columns(&p), 40, "one Aux, on a high bus");
        p.tracks[0].output = 60;
        assert_eq!(columns(&p), 60);
        p.tracks[0].sends.push(SendSlot {
            bus: Some(70),
            level: 0.5,
            pre: false,
        });
        assert_eq!(columns(&p), 70);
    }
}
