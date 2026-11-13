//! The toolbar icons, from Tabler Icons (MIT, see `assets/icons/TABLER-LICENSE.txt`). Each SVG is read
//! into lines once and drawn with the painter, so an icon is sharp at any size and display scale and
//! takes any colour, as text does.
use egui::{pos2, vec2, Color32, Painter, Pos2, Response, Sense, Shape, Stroke, Ui};
use std::sync::OnceLock;

/// The grid the icons are drawn on.
const GRID: f32 = 24.0;
/// The width of their lines on that grid.
const LINE: f32 = 2.0;

macro_rules! icons {
    ($($name:ident => $file:literal,)+) => {
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum Icon {
            $($name,)+
        }

        const SVGS: &[&str] = &[
            $(include_str!(concat!("../assets/icons/", $file, ".svg")),)+
        ];
    };
}

icons! {
    Pointer => "pointer",
    Pencil => "pencil",
    Eraser => "eraser",
    Text => "cursor-text",
    Scissors => "scissors",
    Glue => "bandage",
    Solo => "headphones",
    Mute => "volume-off",
    Zoom => "zoom-in",
    Automation => "timeline",
    Crosshair => "crosshair",
    Flex => "arrows-horizontal",
    Scrub => "hand-move",
    Play => "player-play",
    Stop => "player-stop",
    Brush => "brush",
    Keyboard => "keyboard",
    Ghost => "ghost",
    Refresh => "refresh",
    Folder => "folder",
    Close => "x",
    Plus => "plus",
    Minus => "minus",
    Earlier => "chevron-left",
    Later => "chevron-right",
    Copy => "copy",
    Trash => "trash",
    Mixer => "adjustments-horizontal",
    Bounce => "file-export",
    Piano => "piano",
    Mic => "microphone-2",
    Drum => "ovrtone-drum",
    Pick => "guitar-pick",
    Wave => "wave-sine",
    Other => "wave-saw-tool",
    Stack => "stack-2",
}

/// One stroke of an icon, on its grid.
struct Line {
    points: Vec<Pos2>,
    closed: bool,
}

fn parsed() -> &'static [Vec<Line>] {
    static PARSED: OnceLock<Vec<Vec<Line>>> = OnceLock::new();
    PARSED.get_or_init(|| SVGS.iter().map(|svg| read_svg(svg)).collect())
}

/// Draws `icon` centred on `at`, `size` points high and wide.
pub fn paint(g: &Painter, at: Pos2, size: f32, icon: Icon, color: Color32) {
    let Some(lines) = parsed().get(icon as usize) else {
        return;
    };
    let scale = size / GRID;
    let width = (LINE * scale).max(1.1);
    let place = |p: &Pos2| at + vec2((p.x - GRID / 2.0) * scale, (p.y - GRID / 2.0) * scale);
    for line in lines {
        let points: Vec<Pos2> = line.points.iter().map(place).collect();
        let (Some(&first), Some(&last)) = (points.first(), points.last()) else {
            continue;
        };
        // The ends are round, and a dot is only its ends.
        if !line.closed {
            g.circle_filled(first, width / 2.0, color);
            g.circle_filled(last, width / 2.0, color);
        }
        let stroke = Stroke::new(width, color);
        g.add(if line.closed {
            Shape::closed_line(points, stroke)
        } else {
            Shape::line(points, stroke)
        });
    }
}

/// A small button with an icon for a label. `hint` says what it does, as there are no words on it.
pub fn button(ui: &mut Ui, icon: Icon, hint: &str) -> Response {
    toggle(ui, icon, false, hint)
}

/// [`button`], which shows when it is on.
pub fn toggle(ui: &mut Ui, icon: Icon, on: bool, hint: &str) -> Response {
    let (r, resp) = ui.allocate_exact_size(vec2(22.0, 20.0), Sense::click());
    let resp = resp.on_hover_text(hint);
    let g = ui.painter();
    let color = if on {
        crate::theme::on_accent()
    } else if ui.is_enabled() {
        crate::theme::text()
    } else {
        crate::theme::text_dim()
    };
    if on {
        g.rect_filled(r, 4.0, crate::theme::accent());
    } else if resp.hovered() {
        g.rect_filled(r, 4.0, Color32::from_white_alpha(14));
    }
    paint(g, r.center(), 15.0, icon, color);
    resp
}

/// The lines of every `<path>` that is drawn. The first path of a Tabler icon is an invisible box.
fn read_svg(svg: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    for part in svg.split("<path").skip(1) {
        let tag = part.split('>').next().unwrap_or_default();
        if tag.contains("stroke=\"none\"") {
            continue;
        }
        let Some((_, rest)) = tag.split_once(" d=\"") else {
            continue;
        };
        read_path(rest.split('"').next().unwrap_or_default(), &mut lines);
    }
    lines
}

/// Reads the numbers of a path, which may run together as in `1.5.5-2`.
struct Numbers<'a> {
    text: &'a [u8],
    at: usize,
}

impl Numbers<'_> {
    fn skip_gaps(&mut self) {
        while self
            .text
            .get(self.at)
            .is_some_and(|c| c.is_ascii_whitespace() || *c == b',')
        {
            self.at += 1;
        }
    }

    /// Whether a number comes next, not a command.
    fn more(&mut self) -> bool {
        self.skip_gaps();
        self.text
            .get(self.at)
            .is_some_and(|c| c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.'))
    }

    fn number(&mut self) -> f32 {
        self.skip_gaps();
        let start = self.at;
        if matches!(self.text.get(self.at), Some(b'-' | b'+')) {
            self.at += 1;
        }
        let mut dot = false;
        while let Some(c) = self.text.get(self.at) {
            match c {
                b'0'..=b'9' => {}
                b'.' if !dot => dot = true,
                b'e' | b'E' => {
                    self.at += 1;
                    if matches!(self.text.get(self.at), Some(b'-' | b'+')) {
                        self.at += 1;
                    }
                    continue;
                }
                _ => break,
            }
            self.at += 1;
        }
        std::str::from_utf8(&self.text[start..self.at])
            .ok()
            .and_then(|t| t.parse().ok())
            .unwrap_or(0.0)
    }

    /// An arc's flag: a single digit, which need not be followed by a gap.
    fn flag(&mut self) -> bool {
        self.skip_gaps();
        let on = self.text.get(self.at) == Some(&b'1');
        self.at += 1;
        on
    }
}

fn read_path(d: &str, lines: &mut Vec<Line>) {
    let mut n = Numbers {
        text: d.as_bytes(),
        at: 0,
    };
    let mut points: Vec<Pos2> = Vec::new();
    let mut now = Pos2::ZERO;
    let mut start = Pos2::ZERO;
    // The last control point, for the smooth curves that mirror it.
    let mut control = Pos2::ZERO;
    let mut last = b' ';
    loop {
        n.skip_gaps();
        let Some(&c) = n.text.get(n.at) else { break };
        let command = if c.is_ascii_alphabetic() {
            n.at += 1;
            c
        } else if !n.more() {
            n.at += 1;
            continue;
        } else if last == b'M' {
            // Further pairs after a move are lines.
            b'L'
        } else if last == b'm' {
            b'l'
        } else {
            last
        };
        let relative = command.is_ascii_lowercase();
        let base = if relative { now } else { Pos2::ZERO };
        let xy = |n: &mut Numbers<'_>| {
            let x = n.number();
            let y = n.number();
            base + vec2(x, y)
        };
        match command.to_ascii_uppercase() {
            b'M' => {
                finish(lines, &mut points, false);
                now = xy(&mut n);
                start = now;
                points.push(now);
            }
            b'L' => {
                now = xy(&mut n);
                points.push(now);
            }
            b'H' => {
                now.x = base.x + n.number();
                points.push(now);
            }
            b'V' => {
                now.y = base.y + n.number();
                points.push(now);
            }
            b'C' | b'S' => {
                let c1 = if command.eq_ignore_ascii_case(&b'C') {
                    xy(&mut n)
                } else if last.eq_ignore_ascii_case(&b'C') || last.eq_ignore_ascii_case(&b'S') {
                    now + (now - control)
                } else {
                    now
                };
                let c2 = xy(&mut n);
                let end = xy(&mut n);
                cubic(&mut points, now, c1, c2, end);
                control = c2;
                now = end;
            }
            b'Q' | b'T' => {
                let c1 = if command.eq_ignore_ascii_case(&b'Q') {
                    xy(&mut n)
                } else if last.eq_ignore_ascii_case(&b'Q') || last.eq_ignore_ascii_case(&b'T') {
                    now + (now - control)
                } else {
                    now
                };
                let end = xy(&mut n);
                quadratic(&mut points, now, c1, end);
                control = c1;
                now = end;
            }
            b'A' => {
                let (rx, ry) = (n.number(), n.number());
                let turn = n.number();
                let (large, sweep) = (n.flag(), n.flag());
                let end = xy(&mut n);
                arc(&mut points, now, (rx, ry), turn, large, sweep, end);
                now = end;
            }
            b'Z' => {
                finish(lines, &mut points, true);
                now = start;
                points.push(now);
            }
            _ => {}
        }
        last = command;
    }
    finish(lines, &mut points, false);
}

/// Ends a stroke. The point a closed one leaves behind, for what may follow, is no stroke by itself.
fn finish(lines: &mut Vec<Line>, points: &mut Vec<Pos2>, closed: bool) {
    if points.len() >= 2 {
        lines.push(Line {
            points: std::mem::take(points),
            closed,
        });
    } else {
        points.clear();
    }
}

fn cubic(points: &mut Vec<Pos2>, a: Pos2, b: Pos2, c: Pos2, d: Pos2) {
    const STEPS: usize = 10;
    for i in 1..=STEPS {
        let t = i as f32 / STEPS as f32;
        let u = 1.0 - t;
        let at = a.to_vec2() * (u * u * u)
            + b.to_vec2() * (3.0 * u * u * t)
            + c.to_vec2() * (3.0 * u * t * t)
            + d.to_vec2() * (t * t * t);
        points.push(at.to_pos2());
    }
}

fn quadratic(points: &mut Vec<Pos2>, a: Pos2, b: Pos2, c: Pos2) {
    const STEPS: usize = 8;
    for i in 1..=STEPS {
        let t = i as f32 / STEPS as f32;
        let u = 1.0 - t;
        let at = a.to_vec2() * (u * u) + b.to_vec2() * (2.0 * u * t) + c.to_vec2() * (t * t);
        points.push(at.to_pos2());
    }
}

/// An SVG arc, from its end points to its centre as the SVG specification gives it, then in steps.
fn arc(
    points: &mut Vec<Pos2>,
    from: Pos2,
    radius: (f32, f32),
    turn_degrees: f32,
    large: bool,
    sweep: bool,
    to: Pos2,
) {
    let (mut rx, mut ry) = (radius.0.abs(), radius.1.abs());
    if rx < 1e-4 || ry < 1e-4 || from == to {
        points.push(to);
        return;
    }
    let (sin, cos) = turn_degrees.to_radians().sin_cos();
    let half = (from - to) / 2.0;
    let x1 = cos * half.x + sin * half.y;
    let y1 = -sin * half.x + cos * half.y;
    // Radii too small to reach are scaled up until they just do.
    let reach = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if reach > 1.0 {
        rx *= reach.sqrt();
        ry *= reach.sqrt();
    }
    let top = (rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1).max(0.0);
    let bottom = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let side = if large == sweep { -1.0 } else { 1.0 };
    let factor = side * (top / bottom.max(1e-9)).sqrt();
    let cx1 = factor * rx * y1 / ry;
    let cy1 = -factor * ry * x1 / rx;
    let middle = (from.to_vec2() + to.to_vec2()) / 2.0;
    let center = vec2(cos * cx1 - sin * cy1, sin * cx1 + cos * cy1) + middle;
    let angle = |x: f32, y: f32| y.atan2(x);
    let begin = angle((x1 - cx1) / rx, (y1 - cy1) / ry);
    let end = angle((-x1 - cx1) / rx, (-y1 - cy1) / ry);
    let mut span = end - begin;
    let whole = std::f32::consts::TAU;
    if sweep && span < 0.0 {
        span += whole;
    } else if !sweep && span > 0.0 {
        span -= whole;
    }
    let steps = ((span.abs() / 0.26).ceil() as usize).max(2);
    for i in 1..=steps {
        let a = begin + span * i as f32 / steps as f32;
        let (s, c) = a.sin_cos();
        let (x, y) = (rx * c, ry * s);
        points.push(pos2(
            center.x + cos * x - sin * y,
            center.y + sin * x + cos * y,
        ));
    }
    if let Some(last) = points.last_mut() {
        *last = to;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_read_and_stays_on_its_grid() {
        for (n, lines) in parsed().iter().enumerate() {
            assert!(!lines.is_empty(), "icon {n} has no lines");
            for point in lines.iter().flat_map(|l| &l.points) {
                assert!(
                    (-0.5..=24.5).contains(&point.x) && (-0.5..=24.5).contains(&point.y),
                    "icon {n} went off its grid: {point:?}"
                );
            }
        }
    }

    #[test]
    fn the_scissors_have_their_two_rings_and_two_blades() {
        let lines = &parsed()[Icon::Scissors as usize];
        assert_eq!(lines.len(), 4);
        // The first ring has its centre at (6, 7) and a radius of 3.
        for point in &lines[0].points {
            let d = (*point - pos2(6.0, 7.0)).length();
            assert!((d - 3.0).abs() < 0.01, "{point:?} is {d} from the centre");
        }
    }

    #[test]
    fn numbers_may_run_together() {
        let mut n = Numbers {
            text: b"1.5.5-2,3e1",
            at: 0,
        };
        assert_eq!(n.number(), 1.5);
        assert_eq!(n.number(), 0.5);
        assert_eq!(n.number(), -2.0);
        assert_eq!(n.number(), 30.0);
        assert!(!n.more());
    }
}
