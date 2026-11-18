//! Text on a path: each glyph of a laid-out text turned and moved onto the path, its baseline on
//! the path at the point its advance centre falls on (see [`figcraft_doc::TextPath`]).

use figcraft_doc::{TextAlignH, TextPath};
use figcraft_geom::measure::{Polyline, polylines};
use figcraft_geom::{Affine, Point, Vec2};

use crate::TextLayout;

/// Where a laid-out text sits on its path.
#[derive(Clone, Debug)]
pub struct OnPath {
    // shortcut: the text follows the path's first subpath only; follow the next ones when a use shows up.
    line: Polyline,
    /// Path length before layout x = 0 (start offset and alignment).
    origin: f64,
    flip: bool,
    /// The first line's baseline: layout y that sits on the path.
    baseline: f64,
}

impl OnPath {
    /// Text on `tp` whose layout x = 0 sits `shift` before the start point and whose layout y
    /// `baseline` sits on the path. `None` for an empty or unmeasurable path.
    pub fn new(tp: &TextPath, shift: f64, baseline: f64) -> Option<Self> {
        let p = tp.path();
        if p.elements().len() > 20_000 {
            return None;
        }
        let mut line = polylines(&p)?.into_iter().next()?;
        if line.points.len() > 40_000 {
            return None;
        }
        // A subpath ending where it starts (an ellipse) goes round like a closed one.
        line.closed |= matches!((line.points.first(), line.points.last()), (Some(a), Some(b)) if (*a - *b).hypot() < 1e-6);
        Some(Self { origin: tp.start() * line.length() - shift, line, flip: tp.flip, baseline })
    }

    /// The point on the path `along` units from its start (flipped: from its end) and the
    /// direction there. `None` past an open path's ends.
    fn at(&self, along: f64) -> Option<(Point, Vec2)> {
        let len = self.line.length();
        if len <= 1e-9 || !along.is_finite() {
            return None;
        }
        let s = if self.line.closed { along.rem_euclid(len) } else { along };
        if !(0.0..=len).contains(&s) {
            return None;
        }
        let t = if self.flip { 1.0 - s / len } else { s / len };
        let (p, n) = self.line.at(t)?;
        // The normal is the direction turned 90° (y down): the direction is the normal turned back.
        let dir = Vec2::new(n.y, -n.x);
        Some((p, if self.flip { -dir } else { dir }))
    }

    /// The transform placing what lies around layout point `(x, y)` on the path: `(x, y)` lands
    /// `y - baseline` below the path point `x` along, turned with the path. `None` past an open
    /// path's ends.
    pub fn frame(&self, x: f64, y: f64) -> Option<Affine> {
        let (p, dir) = self.at(self.origin + x)?;
        let below = Vec2::new(-dir.y, dir.x) * (y - self.baseline);
        Some(Affine::translate((p + below).to_vec2()) * Affine::rotate(dir.atan2()) * Affine::translate((-x, -y)))
    }
}

/// Put `out` (laid out without a box) on `tp`. Glyphs past an open path's ends are hidden;
/// decorations are dropped.
pub(crate) fn place(out: &mut TextLayout, tp: &TextPath, align: TextAlignH) {
    let shift = match align {
        TextAlignH::Left | TextAlignH::Justified => 0.0,
        TextAlignH::Center => out.width / 2.0,
        TextAlignH::Right => out.width,
    };
    let Some(on) = OnPath::new(tp, shift, out.lines.first().map_or(0.0, |l| l.baseline)) else {
        out.glyphs.clear();
        out.decorations.clear();
        return;
    };
    for g in &mut out.glyphs {
        // A hidden glyph collapses to a point.
        g.warp = on.frame(g.x + g.advance / 2.0, g.y).unwrap_or(Affine::scale(0.0));
    }
    // shortcut: underline and strikethrough aren't drawn on a path; bend them along when asked for.
    out.decorations.clear();
    out.on_path = Some(on);
}
