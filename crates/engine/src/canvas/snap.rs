//! Snapping and smart guides: while moving or resizing, edges and centres snap to nearby layers'
//! edges/centres, the parent frame, layout grid columns and ruler guides (the page's and the
//! top-level frame's); the matches come back as
//! guide lines (with spacing labels) for the canvas to draw in red. Vector points also snap to
//! other points (see `vectoredit::snap`); landing on one shows a marker (a zero-length guide).

use figcraft_doc::{Document, NodeId};
use figcraft_geom::{Affine, Point, Rect, Vec2, transform_rect_bbox};
use serde::Serialize;

use crate::guides;

/// A guide line to draw: from (x0, y0) to (x1, y1) in page coordinates, with an optional label
/// (a distance) at its middle; `dashed` marks a measurement's extension line.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Guide {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub label: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub dashed: bool,
}

impl Guide {
    /// A plain line from `a` to `b` (a marker when they are the same point).
    pub fn new(a: Point, b: Point) -> Self {
        Self { x0: a.x, y0: a.y, x1: b.x, y1: b.y, label: None, dashed: false }
    }
}

/// A line to snap: its position on one axis and the span it covers on the other (x, y0, y1 for a
/// vertical line; y, x0, x1 for a horizontal one).
pub type Span = (f64, f64, f64);

/// Snap candidates gathered once at the start of a gesture.
#[derive(Clone, Debug, Default)]
pub struct Targets {
    /// Vertical lines (x) with the y-span of the rect they came from.
    pub xs: Vec<Span>,
    /// Horizontal lines (y) with the x-span.
    pub ys: Vec<Span>,
    /// Sibling boxes (for spacing labels).
    pub boxes: Vec<Rect>,
    /// Points to land on exactly (see [`Targets::add_point`]).
    pub points: Vec<Point>,
    /// Page ← snap space. The lines above are in snap space: the page's, or the moving layers'
    /// parent frame's own when it is rotated, so that its edges, layout grids and guides (and the
    /// siblings turned with it) run along the snap axes.
    pub space: Affine,
    /// The moving layers' box in snap space when gathered.
    pub moving: Option<Rect>,
}

/// A gap along a row: from, to, and where across it is drawn.
type Gap = (f64, f64, f64);

impl Targets {
    /// Gather snap lines from the siblings of `moving` (and the parent frame), page guides and
    /// layout grids, excluding the moving nodes and their descendants. Inside a rotated frame they
    /// are gathered in its space.
    pub fn gather(doc: &Document, page: NodeId, moving: &[NodeId]) -> Targets {
        let parent = parent_of(doc, page, moving);
        let xf = doc.world_transform(parent);
        let space = if parent != page && !guides::axis_aligned(xf) { xf } else { Affine::IDENTITY };
        Self::gather_in(doc, page, moving, space)
    }

    /// [`Self::gather`] in snap space `space` (page ← snap space; the page's is the identity).
    /// Lines that don't run along its axes are left out.
    pub fn gather_in(doc: &Document, page: NodeId, moving: &[NodeId], space: Affine) -> Targets {
        let to = space.inverse();
        // A layer's box in snap space.
        let bbox = |id: NodeId| doc.node(id).map(|n| transform_rect_bbox(to * doc.world_transform(id), n.local_rect())).filter(|b| b.is_finite());
        let mut t = Targets { space, moving: moving.iter().filter_map(|m| bbox(*m)).reduce(|a, b| a.union(b)), ..Default::default() };
        let parent = parent_of(doc, page, moving);
        // The parent frame's own box (and its layout grids).
        if parent != page
            && let Some(b) = bbox(parent)
        {
            for x in [b.x0, b.center().x, b.x1] {
                t.xs.push((x, b.y0, b.y1));
            }
            for y in [b.y0, b.center().y, b.y1] {
                t.ys.push((y, b.x0, b.x1));
            }
            let (gx, gy) = figcraft_layout::grid_snap_lines_in(doc, parent, to * doc.world_transform(parent));
            for x in gx {
                t.xs.push((x, b.y0, b.y1));
            }
            for y in gy {
                t.ys.push((y, b.x0, b.x1));
            }
        }
        // Siblings and, at the top level, all top-level layers.
        if let Some(p) = doc.node(parent) {
            for c in &p.children {
                if moving.contains(c) || !doc.node(*c).is_some_and(|n| n.visible) {
                    continue;
                }
                if let Some(b) = bbox(*c) {
                    t.add_box(b);
                }
            }
        }
        // The guides of `owner` mapped into snap space by `m`, across `span` (the owner's box
        // there; None: everywhere).
        let lines = |owner: NodeId, m: Affine, span: Option<Rect>, t: &mut Targets| {
            for g in doc.node(owner).map(|n| n.guides()).unwrap_or_default() {
                match (guides::line_in(m, g.vertical, g.offset), span) {
                    (Some((true, x)), Some(b)) => t.xs.push((x, b.y0, b.y1)),
                    (Some((false, y)), Some(b)) => t.ys.push((y, b.x0, b.x1)),
                    (Some((true, x)), None) => t.xs.push((x, -1e7, 1e7)),
                    (Some((false, y)), None) => t.ys.push((y, -1e7, 1e7)),
                    (None, _) => {}
                }
            }
        };
        lines(page, to, None, &mut t);
        // The guides of the top-level frame the moving layers are in, across it.
        if let Some(f) = doc.top_level_frame(parent)
            && let Some(b) = bbox(f)
        {
            lines(f, to * doc.world_transform(f), Some(b), &mut t);
        }
        t
    }

    /// A box to snap to (in snap space): its edges and centre lines, and equal spacing with it.
    pub fn add_box(&mut self, r: Rect) {
        for x in [r.x0, r.center().x, r.x1] {
            self.xs.push((x, r.y0, r.y1));
        }
        for y in [r.y0, r.center().y, r.y1] {
            self.ys.push((y, r.x0, r.x1));
        }
        self.boxes.push(r);
    }

    /// Take back what [`Targets::add_box`] added for `r`.
    pub fn remove_box(&mut self, r: Rect) {
        let take = |v: &mut Vec<Span>, s: Span| {
            if let Some(i) = v.iter().rposition(|x| *x == s) {
                v.remove(i);
            }
        };
        for x in [r.x0, r.center().x, r.x1] {
            take(&mut self.xs, (x, r.y0, r.y1));
        }
        for y in [r.y0, r.center().y, r.y1] {
            take(&mut self.ys, (y, r.x0, r.x1));
        }
        if let Some(i) = self.boxes.iter().rposition(|b| *b == r) {
            self.boxes.remove(i);
        }
    }

    /// A point to snap onto (a vector point, a segment midpoint): lines through it on both axes,
    /// and a marker when a moving point lands on it.
    pub fn add_point(&mut self, p: Point) {
        self.xs.push((p.x, p.y, p.y));
        self.ys.push((p.y, p.x, p.x));
        self.points.push(p);
    }

    /// Snap a moving box `r` given in snap space: returns the correction (a page vector) and the
    /// guides to draw.
    pub fn snap_rect(&self, r: Rect, threshold: f64) -> (f64, f64, Vec<Guide>) {
        let (xs, ys) = rect_spans(r);
        let (dx, dy, guides) = self.snap_spans(&xs, &ys, threshold);
        let d = self.to_page_vec(Vec2::new(dx.unwrap_or(0.0), dy.unwrap_or(0.0)));
        (d.x, d.y, self.to_page(guides))
    }

    /// Snap the moving layers moved by page vector `d`: the correction to add to it (a page
    /// vector) and the guides to draw. Besides lines, the box snaps to equal spacing with the
    /// layers beside it (whichever is nearer on each axis), and equal gaps show labelled.
    pub fn snap_move(&self, d: Vec2, threshold: f64) -> (Vec2, Vec<Guide>) {
        let Some(b) = self.moving else { return (Vec2::ZERO, Vec::new()) };
        let r = b + self.to_space_vec(d);
        let (xs, ys) = rect_spans(r);
        let (mut dx, mut dy, _) = self.snap_spans(&xs, &ys, threshold);
        for (horizontal, slot) in [(true, &mut dx), (false, &mut dy)] {
            if let Some(e) = self.equal_spacing(r, threshold, horizontal)
                && slot.is_none_or(|v| e.abs() < v.abs())
            {
                *slot = Some(e);
            }
        }
        let c = Vec2::new(dx.unwrap_or(0.0), dy.unwrap_or(0.0));
        let at = r + c;
        let mut guides = self.guides_for(at);
        guides.extend(self.spacing_guides(at, true));
        guides.extend(self.spacing_guides(at, false));
        (self.to_page_vec(c), self.to_page(guides))
    }

    /// The ways `r` can be spaced equally in its row (`horizontal`) or column, worked out as a row
    /// (a column's boxes are transposed): centred between its nearest neighbours, as far after the
    /// one before it as that one is from the next, or as far before the one after it. Each: how far
    /// `r` would move along the row, and the two gaps (from, to, across) that are equal then.
    fn spacings(&self, r: Rect, horizontal: bool) -> Vec<(f64, [Gap; 2])> {
        let t = |b: Rect| if horizontal { b } else { Rect::new(b.y0, b.x0, b.y1, b.x1) };
        let r = t(r);
        let row = self.boxes.iter().map(|b| t(*b)).filter(|b| b.y0 < r.y1 && b.y1 > r.y0);
        let (mut before, mut after): (Vec<Rect>, Vec<Rect>) = row.partition(|b| b.center().x < r.center().x);
        before.sort_by(|a, b| b.x1.total_cmp(&a.x1));
        after.sort_by(|a, b| a.x0.total_cmp(&b.x0));
        // Across the middle of what two boxes share.
        let mid = |a: &Rect, b: &Rect| (a.y0.max(b.y0) + a.y1.min(b.y1)) / 2.0;
        let w = r.width();
        let mut out = Vec::new();
        if let (Some(l), Some(n)) = (before.first(), after.first())
            && n.x0 - l.x1 > w
        {
            let x0 = (l.x1 + n.x0 - w) / 2.0;
            out.push((x0, [(l.x1, x0, mid(l, &r)), (x0 + w, n.x0, mid(&r, n))]));
        }
        if let (Some(l), Some(l2)) = (before.first(), before.get(1))
            && l.x0 > l2.x1
        {
            let x0 = l.x1 + (l.x0 - l2.x1);
            out.push((x0, [(l2.x1, l.x0, mid(l2, l)), (l.x1, x0, mid(l, &r))]));
        }
        if let (Some(n), Some(n2)) = (after.first(), after.get(1))
            && n2.x0 > n.x1
        {
            let x0 = n.x0 - (n2.x0 - n.x1) - w;
            out.push((x0, [(x0 + w, n.x0, mid(&r, n)), (n.x1, n2.x0, mid(n, n2))]));
        }
        out.into_iter().map(|(x0, gaps)| (x0 - r.x0, gaps)).filter(|(d, _)| d.is_finite()).collect()
    }

    /// The correction (snap space, along the row or column) that spaces `r` equally, if one is in
    /// reach.
    fn equal_spacing(&self, r: Rect, threshold: f64, horizontal: bool) -> Option<f64> {
        self.spacings(r, horizontal).into_iter().map(|(d, _)| d).filter(|d| d.abs() <= threshold).min_by(|a, b| a.abs().total_cmp(&b.abs()))
    }

    /// Labelled lines across the gaps that are equal around `r` in its row or column.
    fn spacing_guides(&self, r: Rect, horizontal: bool) -> Vec<Guide> {
        let mut out: Vec<Guide> = Vec::new();
        for (_, gaps) in self.spacings(r, horizontal).into_iter().filter(|(d, _)| d.abs() < 1e-6) {
            for (from, to, across) in gaps {
                let (p, q) =
                    if horizontal { (Point::new(from, across), Point::new(to, across)) } else { (Point::new(across, from), Point::new(across, to)) };
                let line = Guide { label: Some(crate::distance::label(to - from)), ..Guide::new(p, q) };
                if !out.contains(&line) {
                    out.push(line);
                }
            }
        }
        out
    }

    /// Page move `d` adjusted so that the moving layers' box keeps its origin on whole pixels of
    /// the snap space (a rotated frame's own, inside one).
    pub fn round_move(&self, d: Vec2) -> Vec2 {
        let Some(b) = self.moving else { return d };
        let l = self.to_space_vec(d);
        self.to_page_vec(Vec2::new((b.x0 + l.x).round() - b.x0, (b.y0 + l.y).round() - b.y0))
    }

    /// Snap a single page point (resize handles, drawing): the correction (a page vector) and
    /// the guides to draw.
    pub fn snap_point(&self, x: f64, y: f64, threshold: f64) -> (f64, f64, Vec<Guide>) {
        let q = self.space.inverse() * Point::new(x, y);
        let (dx, dy, guides) = self.snap_spans(&[(q.x, q.y, q.y)], &[(q.y, q.x, q.x)], threshold);
        let d = self.to_page_vec(Vec2::new(dx.unwrap_or(0.0), dy.unwrap_or(0.0)));
        (d.x, d.y, self.to_page(guides))
    }

    /// Page vector `v` in snap space.
    fn to_space_vec(&self, v: Vec2) -> Vec2 {
        linear(self.space.inverse(), v)
    }

    /// Snap-space vector `v` on the page.
    fn to_page_vec(&self, v: Vec2) -> Vec2 {
        linear(self.space, v)
    }

    /// Guides found in snap space, on the page.
    fn to_page(&self, guides: Vec<Guide>) -> Vec<Guide> {
        if self.space == Affine::IDENTITY {
            return guides;
        }
        guides
            .into_iter()
            .map(|g| {
                let (a, b) = (self.space * Point::new(g.x0, g.y0), self.space * Point::new(g.x1, g.y1));
                Guide { x0: a.x, y0: a.y, x1: b.x, y1: b.y, ..g }
            })
            .collect()
    }

    /// Snap several moving points together (vector points): the correction on each axis (`None`
    /// where nothing was in reach), the guides, and a marker (a zero-length guide) on every target
    /// point a moving point lands on.
    pub fn snap_points(&self, pts: &[Point], threshold: f64) -> (Option<f64>, Option<f64>, Vec<Guide>) {
        let xs: Vec<Span> = pts.iter().map(|p| (p.x, p.y, p.y)).collect();
        let ys: Vec<Span> = pts.iter().map(|p| (p.y, p.x, p.x)).collect();
        let (dx, dy, mut guides) = self.snap_spans(&xs, &ys, threshold);
        let d = Vec2::new(dx.unwrap_or(0.0), dy.unwrap_or(0.0));
        for p in pts.iter().map(|p| *p + d) {
            let marker = Guide::new(p, p);
            if self.points.iter().any(|t| (*t - p).hypot() < 1e-6) && !guides.contains(&marker) {
                guides.push(marker);
            }
        }
        (dx, dy, guides)
    }

    /// Snap moving lines: `xs` are vertical ones (x with the y-span they cover), `ys` horizontal
    /// ones. Returns the nearest correction on each axis within `threshold` and the guides for the
    /// lines that coincide with targets afterwards.
    pub fn snap_spans(&self, xs: &[Span], ys: &[Span], threshold: f64) -> (Option<f64>, Option<f64>, Vec<Guide>) {
        let best = |cands: &[Span], vals: &[Span]| -> Option<f64> {
            let mut best: Option<f64> = None;
            for (line, _, _) in cands {
                for (v, _, _) in vals {
                    let d = line - v;
                    if d.abs() <= threshold && best.is_none_or(|b| d.abs() < b.abs()) {
                        best = Some(d);
                    }
                }
            }
            best
        };
        let (dx, dy) = (best(&self.xs, xs), best(&self.ys, ys));
        let (ox, oy) = (dx.unwrap_or(0.0), dy.unwrap_or(0.0));
        let xs: Vec<Span> = xs.iter().map(|(x, a, b)| (x + ox, a + oy, b + oy)).collect();
        let ys: Vec<Span> = ys.iter().map(|(y, a, b)| (y + oy, a + ox, b + ox)).collect();
        (dx, dy, self.guides_spans(&xs, &ys))
    }

    /// Guide lines for edges of `r` that coincide with targets.
    pub fn guides_for(&self, r: Rect) -> Vec<Guide> {
        let (xs, ys) = rect_spans(r);
        self.guides_spans(&xs, &ys)
    }

    /// Guides for moving lines (as in [`Self::snap_spans`]) that coincide with targets, each
    /// spanning the line and the targets on it.
    fn guides_spans(&self, xs: &[Span], ys: &[Span]) -> Vec<Guide> {
        let span = |cands: &[Span], (v, a, b): Span| -> Option<(f64, f64)> {
            let hits: Vec<&Span> = cands.iter().filter(|(l, _, _)| (l - v).abs() < 1e-6).collect();
            (!hits.is_empty()).then(|| (hits.iter().map(|h| h.1).fold(a, f64::min).max(-1e6), hits.iter().map(|h| h.2).fold(b, f64::max).min(1e6)))
        };
        let mut out = Vec::new();
        for x in xs {
            if let Some((y0, y1)) = span(&self.xs, *x) {
                out.push(Guide::new(Point::new(x.0, y0), Point::new(x.0, y1)));
            }
        }
        for y in ys {
            if let Some((x0, x1)) = span(&self.ys, *y) {
                out.push(Guide::new(Point::new(x0, y.0), Point::new(x1, y.0)));
            }
        }
        out
    }
}

/// The parent of the moving layers (the first one's), or the page.
fn parent_of(doc: &Document, page: NodeId, moving: &[NodeId]) -> NodeId {
    moving.first().and_then(|m| doc.node(*m)).and_then(|n| n.parent).unwrap_or(page)
}

/// Vector `v` mapped by `m` (without its translation).
fn linear(m: Affine, v: Vec2) -> Vec2 {
    let [a, b, c, d, _, _] = m.as_coeffs();
    Vec2::new(a * v.x + c * v.y, b * v.x + d * v.y)
}

/// A box's edges and centre lines as moving lines (see [`Targets::snap_spans`]).
fn rect_spans(r: Rect) -> ([Span; 3], [Span; 3]) {
    let c = r.center();
    ([(r.x0, r.y0, r.y1), (c.x, r.y0, r.y1), (r.x1, r.y0, r.y1)], [(r.y0, r.x0, r.x1), (c.y, r.x0, r.x1), (r.y1, r.x0, r.x1)])
}
