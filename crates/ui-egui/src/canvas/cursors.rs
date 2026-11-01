//! The pointer on the canvas: each engine cursor ([`figcraft_engine::tools::Cursor`]) as one of
//! the system's cursors, or as a small glyph FigCraft draws at the pointer with the system's
//! hidden (a rotate cursor turned to its corner, the pen's states, the eyedropper, the comment
//! pin). The glyphs are original, drawn from lines and the code-drawn icons: no cursor images.

use egui::{Color32, CursorIcon, Painter, Pos2, Shape, Stroke, pos2, vec2};
use figcraft_engine::tools::{Cursor, PenCursor};

use crate::icons::{self, Icon};

/// A cursor FigCraft draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Glyph {
    /// A curved double arrow about the direction (degrees, 0 = right, 90 = down) from the box's
    /// centre to the corner.
    Rotate(f32),
    Pen(PenCursor),
    Eyedropper,
    Comment,
}

/// What the pointer shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shown {
    System(CursorIcon),
    Drawn(Glyph),
}

/// The system's two-headed arrow nearest a resize along `degrees` (0 = horizontal, 90 = vertical;
/// screen angles, so 45 runs from the top left to the bottom right).
pub fn resize_icon(degrees: f64) -> CursorIcon {
    let a = degrees.rem_euclid(180.0);
    if !(22.5..157.5).contains(&a) {
        CursorIcon::ResizeHorizontal
    } else if a < 67.5 {
        CursorIcon::ResizeNwSe
    } else if a < 112.5 {
        CursorIcon::ResizeVertical
    } else {
        CursorIcon::ResizeNeSw
    }
}

/// What shows for the engine's `cursor`.
pub fn of(cursor: Cursor) -> Shown {
    use Shown::{Drawn, System};
    match cursor {
        Cursor::Default => System(CursorIcon::Default),
        Cursor::Crosshair => System(CursorIcon::Crosshair),
        Cursor::Move => System(CursorIcon::Move),
        Cursor::Resize(a) if a.is_finite() => System(resize_icon(a)),
        Cursor::Resize(_) => System(CursorIcon::Default),
        Cursor::Rotate(a) => Drawn(Glyph::Rotate(if a.is_finite() { a.rem_euclid(360.0) as f32 } else { 0.0 })),
        Cursor::Grab => System(CursorIcon::Grab),
        Cursor::Grabbing => System(CursorIcon::Grabbing),
        Cursor::Text => System(CursorIcon::Text),
        Cursor::Pen(p) => Drawn(Glyph::Pen(p)),
        Cursor::Copy => System(CursorIcon::Copy),
        Cursor::Pointer => System(CursorIcon::PointingHand),
        Cursor::Eyedropper => Drawn(Glyph::Eyedropper),
        Cursor::Comment => Drawn(Glyph::Comment),
        Cursor::NotAllowed => System(CursorIcon::NotAllowed),
    }
}

/// What shows on the canvas: panning (Space or the middle button: the open hand, closed while a
/// button is down) and the zoom tool (Z; `Some(true)`: Alt, zooming out) come before the tool's
/// cursor.
pub fn on_canvas(panning: bool, button_down: bool, zoom_out: Option<bool>, cursor: Cursor) -> Shown {
    match (panning, zoom_out) {
        (true, _) => Shown::System(if button_down { CursorIcon::Grabbing } else { CursorIcon::Grab }),
        (false, Some(out)) => Shown::System(if out { CursorIcon::ZoomOut } else { CursorIcon::ZoomIn }),
        (false, None) => of(cursor),
    }
}

impl Shown {
    /// The system cursor to ask for (none under a drawn one).
    pub fn icon(self) -> CursorIcon {
        match self {
            Shown::System(i) => i,
            Shown::Drawn(_) => CursorIcon::None,
        }
    }

    /// A name for it (`ui.inspect`): the CSS cursor names where there is one ("default",
    /// "nwse-resize", "copy"), and "rotate" (with its angle), "pen", "pen-close", "pen-add",
    /// "pen-bend", "eyedropper" and "comment" for the drawn ones.
    pub fn name(self) -> String {
        match self {
            Shown::System(i) => match i {
                CursorIcon::Default => "default",
                CursorIcon::Crosshair => "crosshair",
                CursorIcon::Move => "move",
                CursorIcon::ResizeHorizontal => "ew-resize",
                CursorIcon::ResizeVertical => "ns-resize",
                CursorIcon::ResizeNwSe => "nwse-resize",
                CursorIcon::ResizeNeSw => "nesw-resize",
                CursorIcon::Grab => "grab",
                CursorIcon::Grabbing => "grabbing",
                CursorIcon::Text => "text",
                CursorIcon::Copy => "copy",
                CursorIcon::PointingHand => "pointer",
                CursorIcon::NotAllowed => "not-allowed",
                CursorIcon::ZoomIn => "zoom-in",
                CursorIcon::ZoomOut => "zoom-out",
                _ => "other",
            }
            .to_string(),
            Shown::Drawn(Glyph::Rotate(a)) => format!("rotate {}", a.round()),
            Shown::Drawn(Glyph::Pen(PenCursor::New)) => "pen".into(),
            Shown::Drawn(Glyph::Pen(PenCursor::Close)) => "pen-close".into(),
            Shown::Drawn(Glyph::Pen(PenCursor::Add)) => "pen-add".into(),
            Shown::Drawn(Glyph::Pen(PenCursor::Bend)) => "pen-bend".into(),
            Shown::Drawn(Glyph::Eyedropper) => "eyedropper".into(),
            Shown::Drawn(Glyph::Comment) => "comment".into(),
        }
    }

    /// Ask for the system cursor and draw the glyph, if it is one, at `at` over everything else.
    pub fn show(self, ctx: &egui::Context, at: Pos2) {
        ctx.set_cursor_icon(self.icon());
        if let Shown::Drawn(g) = self {
            paint(&ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("canvas_cursor"))), at, g);
        }
    }
}

/// Dark lines over a light edge: readable on any page and artwork.
const INK: Color32 = Color32::from_rgb(0x1E, 0x1E, 0x1E);
const EDGE: Color32 = Color32::WHITE;

/// A line through `pts` with the light edge under it.
fn line(painter: &Painter, pts: &[Pos2]) {
    painter.add(Shape::line(pts.to_vec(), Stroke::new(3.5, EDGE)));
    painter.add(Shape::line(pts.to_vec(), Stroke::new(1.5, INK)));
}

/// `icon` in a 16 px box at `min`, with the light edge around it.
fn icon(painter: &Painter, min: Pos2, icon: Icon) {
    let r = egui::Rect::from_min_size(min, vec2(16.0, 16.0));
    for d in [vec2(-1.0, 0.0), vec2(1.0, 0.0), vec2(0.0, -1.0), vec2(0.0, 1.0)] {
        icons::paint(painter, r.translate(d), icon, EDGE);
    }
    icons::paint(painter, r, icon, INK);
}

/// Points of the arc of radius `r` about `c` from angle `a0` to `a1` (radians).
fn arc(c: Pos2, r: f32, a0: f32, a1: f32) -> Vec<Pos2> {
    (0..=12).map(|i| a0 + (a1 - a0) * i as f32 / 12.0).map(|a| c + vec2(a.cos(), a.sin()) * r).collect()
}

/// Draw `glyph` with its hot spot at `at`.
pub fn paint(painter: &Painter, at: Pos2, glyph: Glyph) {
    match glyph {
        Glyph::Rotate(degrees) => {
            // An arc bulging away from the box, through the pointer, with an arrowhead at each end.
            let (dir, r, half) = (degrees.to_radians(), 11.0, 0.8);
            let c = at - vec2(dir.cos(), dir.sin()) * r;
            let pts = arc(c, r, dir - half, dir + half);
            line(painter, &pts);
            for (end, sign) in [(dir - half, -1.0), (dir + half, 1.0)] {
                let tip = c + vec2(end.cos(), end.sin()) * r;
                // Along the arc at its end, pointing onwards.
                let t = vec2(-end.sin(), end.cos()) * sign;
                let n = vec2(end.cos(), end.sin());
                line(painter, &[tip - t * 4.5 + n * 3.5, tip + t * 1.0, tip - t * 4.5 - n * 3.5]);
            }
        }
        Glyph::Pen(state) => {
            // A cross open at the centre (the point it lands on stays visible), and what a click
            // does at its lower right.
            for (a, b) in [
                (vec2(-8.0, 0.0), vec2(-2.5, 0.0)),
                (vec2(2.5, 0.0), vec2(8.0, 0.0)),
                (vec2(0.0, -8.0), vec2(0.0, -2.5)),
                (vec2(0.0, 2.5), vec2(0.0, 8.0)),
            ] {
                line(painter, &[at + a, at + b]);
            }
            let b = at + vec2(10.0, 10.0);
            match state {
                PenCursor::New => {}
                PenCursor::Close => {
                    let mut ring = arc(b, 3.0, 0.0, std::f32::consts::TAU);
                    ring.push(pos2(b.x + 3.0, b.y));
                    line(painter, &ring);
                }
                PenCursor::Add => {
                    line(painter, &[b - vec2(3.5, 0.0), b + vec2(3.5, 0.0)]);
                    line(painter, &[b - vec2(0.0, 3.5), b + vec2(0.0, 3.5)]);
                }
                PenCursor::Bend => line(painter, &arc(b + vec2(-3.5, 3.5), 7.0, -std::f32::consts::FRAC_PI_2, 0.0)),
            }
        }
        // The dropper's tip and the pin's tail, at the bottom left of their icons, are the hot spots.
        Glyph::Eyedropper => icon(painter, at - vec2(2.5, 13.5), Icon::Eyedropper),
        Glyph::Comment => icon(painter, at - vec2(2.5, 13.5), Icon::Comment),
    }
}
