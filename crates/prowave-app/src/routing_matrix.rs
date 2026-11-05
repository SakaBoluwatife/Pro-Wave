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
pub fn toggle_send(p: &mut Project, t: usize, bus: u16) -> bool {
    let Some(track) = p.tracks.get_mut(t) else {
        return false;
    };
    if let Some(at) = track.sends.iter().position(|s| s.bus == Some(bus)) {
        track.sends.remove(at);
    } else {
        if track.sends.len() >= MAX_SENDS {
            return false;
        }
        track.sends.push(SendSlot {
            bus: Some(bus),
            level: 0.75,
            pre: false,
        });
    }
    p.song_rev += 1;
    true
}

/// Sends the strip's output to `bus`, or back to Stereo Out if it was there already.
pub fn toggle_output(p: &mut Project, t: usize, bus: u16) {
    if let Some(track) = p.tracks.get_mut(t) {
        track.output = if track.output == bus { 0 } else { bus };
        p.song_rev += 1;
    }
}

/// The window.
pub fn show(ctx: &egui::Context, p: &mut Project, s: &mut UiState) {
    if !s.dialogs.routing {
        return;
    }
    let mut open = true;
    let mut message = None;
    // A column for each Aux, and at least the first eight.
    let buses = (p.aux_buses().len() as u16).max(8);
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
                                    (Color32::from_gray(0x33), "")
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
                                    if resp.clicked() && !toggle_send(p, t, bus) {
                                        message =
                                            Some(format!("{} has no free send", p.tracks[t].name));
                                    }
                                    if resp.secondary_clicked() {
                                        toggle_output(p, t, bus);
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
        assert!(toggle_send(&mut p, t, 2));
        assert!(sends_to(&p, t, 2));
        assert!(toggle_send(&mut p, t, 2));
        assert!(!sends_to(&p, t, 2));
        toggle_output(&mut p, t, 3);
        assert_eq!(p.tracks[t].output, 3);
        toggle_output(&mut p, t, 3);
        assert_eq!(p.tracks[t].output, 0, "back to Stereo Out");
        for bus in 1..=MAX_SENDS as u16 {
            assert!(toggle_send(&mut p, t, bus));
        }
        assert!(!toggle_send(&mut p, t, 8), "no free slot");
    }
}
