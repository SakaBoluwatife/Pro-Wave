//! Playing notes on the computer keyboard (FL Studio: Typing keyboard to piano; Logic: Musical
//! Typing). Two rows make two octaves: Z to / starting on one C, Q to P an octave above, the keys
//! between them the black notes. Ctrl+K turns it on and off, numpad + and - change the octave.
//!
//! The notes go the way a MIDI keyboard's do, so they play the selected track's instrument and
//! are recorded. While it is on, the note keys are taken before anything else sees them, and
//! Shift picks the piano roll's tools.
use crate::recorder::LiveNote;
use egui::{Event, Key};

const VELOCITY: u8 = 100;
/// The note the lower row starts on (C4 in the MIDI numbering used here).
const BASE: i32 = 60;

/// The semitone a key plays above the lower row's first note.
fn semitone_of(key: Key) -> Option<i32> {
    Some(match key {
        Key::Z => 0,
        Key::S => 1,
        Key::X => 2,
        Key::D => 3,
        Key::C => 4,
        Key::V => 5,
        Key::G => 6,
        Key::B => 7,
        Key::H => 8,
        Key::N => 9,
        Key::J => 10,
        Key::M => 11,
        Key::Comma => 12,
        Key::L => 13,
        Key::Period => 14,
        Key::Semicolon => 15,
        Key::Slash => 16,
        Key::Q => 12,
        Key::Num2 => 13,
        Key::W => 14,
        Key::Num3 => 15,
        Key::E => 16,
        Key::R => 17,
        Key::Num5 => 18,
        Key::T => 19,
        Key::Num6 => 20,
        Key::Y => 21,
        Key::Num7 => 22,
        Key::U => 23,
        Key::I => 24,
        Key::Num9 => 25,
        Key::O => 26,
        Key::Num0 => 27,
        Key::P => 28,
        _ => return None,
    })
}

#[derive(Default)]
pub struct Typing {
    pub on: bool,
    /// Octaves up or down from the middle.
    pub octave: i32,
    /// Keys held and the note each started, so the note that ends is the one that began, even if
    /// the octave changed meanwhile.
    held: Vec<(Key, u8)>,
}

impl Typing {
    fn note_of(&self, key: Key) -> Option<u8> {
        let note = BASE + self.octave * 12 + semitone_of(key)?;
        (0..=127).contains(&note).then_some(note as u8)
    }

    fn release_all(&mut self, out: &mut Vec<LiveNote>) {
        for (_, pitch) in self.held.drain(..) {
            out.push(LiveNote::Off { pitch });
        }
    }

    /// Takes this frame's key events: its own go no further, and what they play goes to `out`.
    /// Text boxes keep their letters.
    pub fn poll(&mut self, ctx: &egui::Context, out: &mut Vec<LiveNote>) {
        let toggled = ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::K));
        if toggled {
            self.on = !self.on;
            if !self.on {
                self.release_all(out);
            }
        }
        if !self.on || ctx.egui_wants_keyboard_input() {
            self.release_all(out);
            return;
        }
        ctx.input_mut(|i| {
            let mut played = Vec::new();
            i.events.retain(|event| match event {
                Event::WindowFocused(false) => {
                    self.release_all(&mut played);
                    true
                }
                Event::Key {
                    key,
                    pressed,
                    repeat,
                    modifiers,
                    ..
                } => {
                    let own = !modifiers.command && !modifiers.alt && !modifiers.shift;
                    match (*key, *pressed) {
                        (Key::Plus, true) if own => {
                            self.octave = (self.octave + 1).min(4);
                            false
                        }
                        (Key::Minus, true) if own => {
                            self.octave = (self.octave - 1).max(-5);
                            false
                        }
                        (k, true) if own && semitone_of(k).is_some() => {
                            if !*repeat && !self.held.iter().any(|(h, _)| *h == k) {
                                if let Some(pitch) = self.note_of(k) {
                                    self.held.push((k, pitch));
                                    played.push(LiveNote::On {
                                        pitch,
                                        vel: VELOCITY,
                                    });
                                }
                            }
                            false
                        }
                        // Let go, whatever else is held by now.
                        (k, false) => match self.held.iter().position(|(h, _)| *h == k) {
                            Some(at) => {
                                let (_, pitch) = self.held.remove(at);
                                played.push(LiveNote::Off { pitch });
                                false
                            }
                            None => true,
                        },
                        _ => true,
                    }
                }
                _ => true,
            });
            out.extend(played);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Modifiers, RawInput};

    fn key(key: Key, pressed: bool, modifiers: Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        }
    }

    /// One frame with these events: what was played, and which events are left for the others.
    fn frame(t: &mut Typing, ctx: &egui::Context, events: Vec<Event>) -> (Vec<LiveNote>, usize) {
        let mut played = Vec::new();
        let mut left = 0;
        let mut output = ctx.run_ui(
            RawInput {
                events,
                ..RawInput::default()
            },
            |ui| {
                t.poll(ui.ctx(), &mut played);
                left = ui.input(|i| i.events.len());
            },
        );
        // Nothing paints these frames.
        output.textures_delta.clear();
        (played, left)
    }

    #[test]
    fn the_two_rows_lay_out_like_a_piano() {
        // White keys on the letters, black on the keys above them.
        let t = Typing::default();
        assert_eq!(t.note_of(Key::Z), Some(60));
        assert_eq!(t.note_of(Key::S), Some(61));
        assert_eq!(t.note_of(Key::M), Some(71));
        assert_eq!(t.note_of(Key::Q), Some(72), "the upper row is an octave up");
        assert_eq!(t.note_of(Key::P), Some(88));
        assert_eq!(t.note_of(Key::A), None);
        let high = Typing {
            octave: 9,
            ..Typing::default()
        };
        assert_eq!(high.note_of(Key::P), None, "past the top of MIDI");
    }

    #[test]
    fn off_it_takes_nothing_and_on_it_takes_its_keys() {
        let ctx = egui::Context::default();
        let mut t = Typing::default();
        let z = key(Key::Z, true, Modifiers::NONE);
        let (played, left) = frame(&mut t, &ctx, vec![z.clone()]);
        assert!(played.is_empty());
        assert_eq!(left, 1, "the key goes on to the rest of the program");
        let _ = frame(&mut t, &ctx, vec![key(Key::Z, false, Modifiers::NONE)]);

        let (_, _) = frame(&mut t, &ctx, vec![key(Key::K, true, Modifiers::COMMAND)]);
        assert!(t.on);
        let (played, left) = frame(&mut t, &ctx, vec![z]);
        assert_eq!(
            played,
            [LiveNote::On {
                pitch: 60,
                vel: 100
            }]
        );
        assert_eq!(left, 0, "nothing else hears it");
    }

    #[test]
    fn a_key_ends_the_note_it_began_and_combinations_stay_shortcuts() {
        let ctx = egui::Context::default();
        let mut t = Typing {
            on: true,
            ..Typing::default()
        };
        let none = Modifiers::NONE;
        let _ = frame(&mut t, &ctx, vec![key(Key::Z, true, none)]);
        // The octave moves while it is held.
        let (played, _) = frame(&mut t, &ctx, vec![key(Key::Plus, true, none)]);
        assert!(played.is_empty());
        assert_eq!(t.octave, 1);
        let (played, _) = frame(&mut t, &ctx, vec![key(Key::Z, false, none)]);
        assert_eq!(played, [LiveNote::Off { pitch: 60 }]);
        let (played, _) = frame(&mut t, &ctx, vec![key(Key::Z, true, none)]);
        assert_eq!(
            played,
            [LiveNote::On {
                pitch: 72,
                vel: 100
            }]
        );

        // Ctrl+Z, Shift+D and Alt+Q are shortcuts as ever.
        for (k, m) in [
            (Key::X, Modifiers::COMMAND),
            (Key::D, Modifiers::SHIFT),
            (Key::Q, Modifiers::ALT),
        ] {
            let (played, left) = frame(&mut t, &ctx, vec![key(k, true, m)]);
            assert!(played.is_empty());
            assert_eq!(left, 1);
        }

        // Turned off, or losing the window, ends what is held.
        let (played, _) = frame(&mut t, &ctx, vec![Event::WindowFocused(false)]);
        assert_eq!(played, [LiveNote::Off { pitch: 72 }]);
        let _ = frame(&mut t, &ctx, vec![key(Key::C, true, none)]);
        let (played, _) = frame(&mut t, &ctx, vec![key(Key::K, true, Modifiers::COMMAND)]);
        assert_eq!(played, [LiveNote::Off { pitch: 76 }]);
        assert!(!t.on);
    }
}
