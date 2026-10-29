use super::{
    Color32, FRAC_PI_2, FRAC_PI_4, Icon, Mesh, PI, Painter, Pos2, Rect, Shape, Stroke, TAU, Vec2, arc_pts, capsule, pos2, triangulate, vec2,
};

struct P<'a> {
    painter: &'a Painter,
    o: Pos2,
    s: f32,
    /// One physical pixel, in points.
    px: f32,
    stroke: Stroke,
    color: Color32,
}

impl P<'_> {
    fn raw(&self, x: f32, y: f32) -> Pos2 {
        pos2(self.o.x + x * self.s, self.o.y + y * self.s)
    }
    /// A stroke coordinate on the pixel grid: centred on a pixel for odd widths, on an edge for even.
    fn snap_c(&self, v: f32) -> f32 {
        let odd = (self.stroke.width / self.px).round() as i32 % 2 == 1;
        let off = if odd { 0.5 } else { 0.0 };
        ((v / self.px - off).round() + off) * self.px
    }
    /// A fill edge on the pixel grid.
    fn snap_e(&self, v: f32) -> f32 {
        (v / self.px).round() * self.px
    }
    /// A hinted stroke vertex.
    fn sp(&self, x: f32, y: f32) -> Pos2 {
        let q = self.raw(x, y);
        pos2(self.snap_c(q.x), self.snap_c(q.y))
    }
    /// A straight stroke; axis-aligned ones are hinted (crossways onto pixel centres, lengthways
    /// onto pixel edges so they end sharp).
    fn line(&self, x0: f32, y0: f32, x1: f32, y1: f32) {
        let (a, b) = (self.raw(x0, y0), self.raw(x1, y1));
        let (a, b) = if y0 == y1 {
            let y = self.snap_c(a.y);
            (pos2(self.snap_e(a.x), y), pos2(self.snap_e(b.x), y))
        } else if x0 == x1 {
            let x = self.snap_c(a.x);
            (pos2(x, self.snap_e(a.y)), pos2(x, self.snap_e(b.y)))
        } else {
            (a, b)
        };
        self.painter.line_segment([a, b], self.stroke);
    }
    /// A polyline of straight segments through hinted vertices.
    fn poly(&self, pts: &[(f32, f32)], closed: bool) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.sp(x, y)).collect();
        self.stroke_pts(v, closed);
    }
    /// A path with curves (arcs, capsules): drawn as given, unhinted.
    fn path(&self, pts: &[(f32, f32)], closed: bool) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.raw(x, y)).collect();
        self.stroke_pts(v, closed);
    }
    fn stroke_pts(&self, v: Vec<Pos2>, closed: bool) {
        if closed {
            self.painter.add(Shape::closed_line(v, self.stroke));
        } else {
            self.painter.add(Shape::line(v, self.stroke));
        }
    }
    /// A filled polygon, convex or not.
    fn fill(&self, pts: &[(f32, f32)]) {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| self.raw(x, y)).collect();
        let n = v.len();
        let turn = |i: usize| {
            let (a, b, c) = (v[i], v[(i + 1) % n], v[(i + 2) % n]);
            (b - a).x * (c - b).y - (b - a).y * (c - b).x
        };
        let convex = n >= 3 && ((0..n).all(|i| turn(i) >= 0.0) || (0..n).all(|i| turn(i) <= 0.0));
        if convex {
            self.painter.add(Shape::convex_polygon(v, self.color, Stroke::NONE));
            return;
        }
        let indices = triangulate(&v);
        if !indices.is_empty() {
            let mut mesh = Mesh::default();
            for p in &v {
                mesh.colored_vertex(*p, self.color);
            }
            mesh.indices = indices;
            self.painter.add(Shape::mesh(mesh));
        }
        // A hairline around the mesh gives it the same soft edge as the convex fills.
        self.painter.add(Shape::closed_line(v, Stroke::new(self.px, self.color)));
    }
    /// Triangles `tris` over `verts` (x, y, alpha): the colour fades between the vertices.
    fn shade(&self, verts: &[(f32, f32, f32)], tris: &[u32]) {
        let mut mesh = Mesh::default();
        for &(x, y, a) in verts {
            mesh.colored_vertex(self.raw(x, y), self.color.gamma_multiply(a));
        }
        mesh.indices = tris.to_vec();
        self.painter.add(Shape::mesh(mesh));
    }
    /// A disc of radius `r` at the centre, as a fan from the top, clockwise: each slice's rim
    /// alphas from `rim` and its centre's from `centre`, both of how far round it is (0..1).
    fn shade_fan(&self, r: f32, centre: impl Fn(f32) -> f32, rim: impl Fn(f32) -> f32) {
        const SLICES: u32 = 32;
        let at = |t: f32| {
            let a = -FRAC_PI_2 + t * TAU;
            (8.0 + r * a.cos(), 8.0 + r * a.sin())
        };
        let mut verts = Vec::new();
        for i in 0..SLICES {
            let (t0, t1) = (i as f32 / SLICES as f32, (i + 1) as f32 / SLICES as f32);
            let ((x0, y0), (x1, y1)) = (at(t0), at(t1));
            verts.extend([(8.0, 8.0, centre((t0 + t1) / 2.0)), (x0, y0, rim(t0)), (x1, y1, rim(t1))]);
        }
        let tris: Vec<u32> = (0..SLICES * 3).collect();
        self.shade(&verts, &tris);
    }
    fn rect(&self, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) {
        let rr = Rect::from_min_max(self.sp(x0, y0), self.sp(x1, y1));
        self.painter.rect_stroke(rr, egui::CornerRadius::same((r * self.s).round() as u8), self.stroke, egui::StrokeKind::Middle);
    }
    fn fill_rect(&self, x0: f32, y0: f32, x1: f32, y1: f32, r: f32) {
        let (a, b) = (self.raw(x0, y0), self.raw(x1, y1));
        let rr = Rect::from_min_max(pos2(self.snap_e(a.x), self.snap_e(a.y)), pos2(self.snap_e(b.x), self.snap_e(b.y)));
        self.painter.rect_filled(rr, egui::CornerRadius::same((r * self.s).round() as u8), self.color);
    }
    fn circle(&self, x: f32, y: f32, r: f32) {
        self.painter.circle_stroke(self.raw(x, y), r * self.s, self.stroke);
    }
    fn dot(&self, x: f32, y: f32, r: f32) {
        self.painter.circle_filled(self.raw(x, y), r * self.s, self.color);
    }
    fn arc(&self, cx: f32, cy: f32, r: f32, a0: f32, a1: f32) {
        self.path(&arc_pts(cx, cy, r, a0, a1).collect::<Vec<_>>(), false);
    }
    /// An open arrowhead at `tip` pointing along angle `dir`, arms `len` long.
    fn head(&self, tip: (f32, f32), dir: f32, len: f32) {
        let arm = |a: f32| (tip.0 + len * a.cos(), tip.1 + len * a.sin());
        self.path(&[arm(dir + PI - FRAC_PI_4), tip, arm(dir + PI + FRAC_PI_4)], false);
    }
    /// An arc with an arrowhead at its end, pointing the way it runs.
    fn arc_arrow(&self, cx: f32, cy: f32, r: f32, a0: f32, a1: f32) {
        self.arc(cx, cy, r, a0, a1);
        let tip = (cx + r * a1.cos(), cy + r * a1.sin());
        let dir = a1 + if a1 > a0 { FRAC_PI_2 } else { -FRAC_PI_2 };
        self.head(tip, dir, 2.6);
    }
    /// A rounded corner mark: an L at (`x`, `y`) opening towards (`dx`, `dy`), `len` long.
    fn corner(&self, x: f32, y: f32, dx: f32, dy: f32, len: f32, r: f32) {
        let a0 = 0.0f32.atan2(-dx);
        let mut a1 = (-dy).atan2(0.0);
        if a1 - a0 > PI {
            a1 -= TAU;
        } else if a0 - a1 > PI {
            a1 += TAU;
        }
        let mut c = vec![(x, y + len * dy)];
        c.extend(arc_pts(x + r * dx, y + r * dy, r, a0, a1));
        c.push((x + len * dx, y));
        self.path(&c, false);
    }
    /// Short dashes along the outline of a box (`n` per side).
    fn dashed_box(&self, x0: f32, y0: f32, x1: f32, y1: f32, n: usize) {
        let seg = |a: f32, b: f32, i: usize| {
            let step = (b - a) / (2 * n - 1) as f32;
            (a + 2.0 * i as f32 * step, a + (2 * i + 1) as f32 * step)
        };
        for i in 0..n {
            let (a, b) = seg(x0 - 0.5, x1 + 0.5, i);
            self.line(a, y0, b, y0);
            self.line(a, y1, b, y1);
            let (a, b) = seg(y0 - 0.5, y1 + 0.5, i);
            self.line(x0, a, x0, b);
            self.line(x1, a, x1, b);
        }
    }
}

/// Paint `icon` centred in `rect` (a 16-unit glyph scaled to fit) in `color`.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let ppp = painter.ctx().pixels_per_point();
    let px = 1.0 / ppp;
    // Physical pixels per grid unit: whole steps from 16 px up (quarter steps on fractional
    // display scales), the exact fit below.
    let k = rect.width().min(rect.height()) * ppp / 16.0;
    let k = if k >= 1.0 { (k * 4.0).floor() / 4.0 } else { k };
    let s = k * px;
    let snap = |v: f32| (v * ppp).round() * px;
    let o = rect.center() - vec2(8.0 * s, 8.0 * s);
    let o = pos2(snap(o.x), snap(o.y));
    let w = (k * 0.9).round().max(1.0) * px;
    let p = P { painter, o, s, px, stroke: Stroke::new(w, color), color };
    draw(&p, icon);
}

/// An open hand: four fingers (the middle one tallest), then the palm and the thumb.
fn hand(p: &P) {
    for (x, top) in [(4.5, 5.0), (6.5, 3.5), (8.5, 4.0), (10.5, 5.5)] {
        let mut f = vec![(x, 8.0)];
        f.extend(arc_pts(x + 1.0, top, 1.0, PI, TAU));
        f.push((x + 2.0, 8.0));
        p.path(&f, false);
    }
    let mut h = vec![(12.5, 8.0), (12.5, 10.0)];
    h.extend(arc_pts(9.5, 10.0, 3.0, 0.0, FRAC_PI_2));
    h.extend([(8.0, 13.0), (3.0, 9.4)]);
    h.extend(arc_pts(3.6, 8.6, 1.0, 2.2, 5.3));
    h.push((4.5, 8.5));
    p.path(&h, false);
}

/// A curve with a smooth point and its handle line.
fn bend(p: &P) {
    let pts: Vec<(f32, f32)> = (0..=16)
        .map(|i| {
            let t = i as f32 / 16.0;
            let u = 1.0 - t;
            (u * u * 2.5 + 2.0 * u * t * 8.0 + t * t * 13.5, u * u * 13.5 + 2.0 * u * t * 1.5 + t * t * 13.5)
        })
        .collect();
    p.path(&pts, false);
    p.line(3.0, 7.5, 13.0, 7.5);
    p.dot(3.0, 7.5, 1.3);
    p.dot(13.0, 7.5, 1.3);
    p.fill_rect(7.0, 6.0, 9.0, 9.0, 0.0);
}

/// A five-pointed star, filled.
fn star(p: &P) {
    let pts: Vec<(f32, f32)> = (0..10)
        .map(|i| {
            let a = -FRAC_PI_2 + PI * i as f32 / 5.0;
            let r = if i % 2 == 0 { 6.0 } else { 2.7 };
            (8.0 + r * a.cos(), 8.6 + r * a.sin())
        })
        .collect();
    p.path(&pts, true);
}

/// The Component icon.
fn draw_component(p: &P) {
    for (cx, cy) in [(8.0, 4.2), (11.8, 8.0), (8.0, 11.8), (4.2, 8.0)] {
        p.fill(&[(cx, cy - 2.5), (cx + 2.5, cy), (cx, cy + 2.5), (cx - 2.5, cy)]);
    }
}

/// The ComponentSet icon.
fn draw_componentset(p: &P) {
    for (cx, cy) in [(8.0, 4.3), (11.7, 8.0), (8.0, 11.7), (4.3, 8.0)] {
        p.path(&[(cx, cy - 2.2), (cx + 2.2, cy), (cx, cy + 2.2), (cx - 2.2, cy)], true);
    }
}

/// The Group icon.
fn draw_group(p: &P) {
    for (x, y, dx, dy) in [(2.5, 2.5, 1.0, 1.0), (13.5, 2.5, -1.0, 1.0), (2.5, 13.5, 1.0, -1.0), (13.5, 13.5, -1.0, -1.0)] {
        p.poly(&[(x + 3.5 * dx, y), (x, y), (x, y + 3.5 * dy)], false);
    }
}

/// The Grid/LayoutGrid icons.
fn draw_grid_layoutgrid(p: &P) {
    for i in [2.5, 6.5, 10.5] {
        for j in [2.5, 6.5, 10.5] {
            p.rect(i, j, i + 3.0, j + 3.0, 0.0);
        }
    }
}

/// The FlipH icon.
fn draw_fliph(p: &P) {
    for y in [1.0, 5.0, 9.0, 13.0] {
        p.line(7.5, y, 7.5, y + 2.0);
    }
    p.poly(&[(5.5, 3.5), (5.5, 12.5), (1.5, 12.5)], true);
    p.fill(&[(9.0, 3.0), (14.0, 13.0), (9.0, 13.0)]);
}

/// Draw `icon` on the 16-unit grid through `p`, which maps grid units to the target box.
fn draw(p: &P, icon: Icon) {
    use Icon::*;
    match icon {
        Move => p.fill(&[(4.0, 2.0), (4.0, 13.6), (6.9, 10.9), (8.8, 14.8), (10.6, 14.0), (8.7, 10.2), (12.6, 10.2)]),
        Scale => {
            p.rect(2.5, 7.5, 8.5, 13.5, 0.0);
            p.line(6.0, 10.0, 13.0, 3.0);
            p.poly(&[(9.5, 2.5), (13.5, 2.5), (13.5, 6.5)], false);
        }
        Hand => {
            hand(p);
        }
        Frame => {
            // Inset a unit from the box: four long hinted strokes read heavier than the
            // anti-aliased outlines beside them at full size (#737).
            p.line(5.5, 3.0, 5.5, 13.0);
            p.line(10.5, 3.0, 10.5, 13.0);
            p.line(3.0, 5.5, 13.0, 5.5);
            p.line(3.0, 10.5, 13.0, 10.5);
        }
        Section | Section2 => {
            p.rect(2.5, 5.5, 13.5, 13.5, 1.0);
            p.fill_rect(2.0, 2.0, 9.0, 6.0, 1.0);
        }
        Slice => {
            // A knife, one silhouette (#739): a slim blade to the top right, outlined like the
            // rest of the row, and a solid handle whose end overlaps the blade's base so the two
            // join at every size.
            p.poly(&[(13.5, 2.5), (5.0, 8.0), (8.0, 11.0)], true);
            p.fill(&capsule((2.8, 13.2), (5.9, 10.1), 1.3));
        }
        Bend => {
            bend(p);
        }
        PaintBucket => {
            p.poly(&[(2.5, 8.0), (7.5, 3.0), (12.5, 8.0), (7.5, 13.0)], true);
            p.fill(&[(2.5, 8.0), (12.5, 8.0), (7.5, 13.0)]);
            p.line(7.5, 3.0, 5.0, 0.5);
            p.dot(13.5, 11.5, 1.5);
        }
        Lasso => {
            p.path(&arc_pts(8.5, 6.5, 5.0, 0.0, TAU).map(|(x, y)| (x, 6.5 + (y - 6.5) * 0.7)).collect::<Vec<_>>(), true);
            p.path(&[(5.0, 9.3), (4.5, 11.5), (6.0, 12.5), (4.5, 14.5)], false);
        }
        Cut => {
            p.circle(4.5, 11.5, 2.0);
            p.circle(11.5, 11.5, 2.0);
            p.line(6.0, 10.0, 11.5, 2.5);
            p.line(10.0, 10.0, 4.5, 2.5);
        }
        Crop => {
            // Two right angles overlapping into a frame, each running past the other's corner.
            p.poly(&[(4.5, 1.5), (4.5, 11.5), (14.5, 11.5)], false);
            p.poly(&[(1.5, 4.5), (11.5, 4.5), (11.5, 14.5)], false);
        }
        Rectangle => p.rect(2.5, 2.5, 13.5, 13.5, 1.0),
        Line => p.line(2.5, 13.5, 13.5, 2.5),
        Arrow => {
            p.line(2.5, 13.5, 13.0, 3.0);
            p.poly(&[(7.5, 2.5), (13.5, 2.5), (13.5, 8.5)], false);
        }
        Ellipse => p.circle(8.0, 8.0, 5.5),
        Polygon => p.poly(&[(8.0, 2.5), (13.5, 12.5), (2.5, 12.5)], true),
        Star => star(p),
        Image => {
            p.rect(2.5, 3.5, 13.5, 12.5, 1.0);
            p.poly(&[(2.5, 11.5), (6.0, 8.0), (9.0, 11.0), (10.5, 9.5), (13.5, 12.5)], false);
            p.dot(10.5, 6.5, 1.1);
        }
        Pen => {
            p.poly(&[(8.0, 2.0), (12.5, 8.5), (10.5, 13.5), (5.5, 13.5), (3.5, 8.5)], true);
            p.line(8.0, 2.0, 8.0, 7.0);
            p.dot(8.0, 8.8, 1.2);
        }
        Pencil => {
            p.poly(&[(2.5, 13.5), (3.2, 10.3), (10.5, 3.0), (13.0, 5.5), (5.7, 12.8)], true);
            p.line(9.0, 4.5, 11.5, 7.0);
        }
        Text => {
            // Inset like the Frame mark, so its long strokes don't outweigh the outlines (#737).
            p.line(3.0, 3.5, 12.0, 3.5);
            p.line(7.5, 3.0, 7.5, 12.5);
            p.line(3.5, 3.0, 3.5, 5.5);
            p.line(11.5, 3.0, 11.5, 5.5);
            p.line(5.5, 12.5, 9.5, 12.5);
        }
        TextOnPath => {
            // A small T above the curve it follows.
            p.line(4.5, 3.0, 11.5, 3.0);
            p.line(8.0, 3.0, 8.0, 8.5);
            p.poly(&[(2.0, 13.5), (5.0, 11.5), (8.0, 10.8), (11.0, 11.5), (14.0, 13.5)], false);
        }
        Comment => {
            let mut b: Vec<(f32, f32)> = arc_pts(8.5, 7.5, 5.0, 2.75, TAU + 1.95).collect();
            b.push((2.5, 13.5));
            p.path(&b, true);
        }
        Actions => p.poly(&[(9.0, 2.0), (3.5, 9.0), (7.5, 9.0), (6.5, 14.0), (12.5, 7.0), (8.5, 7.0)], true),
        Chevron => p.poly(&[(4.5, 6.0), (8.0, 9.5), (11.5, 6.0)], false),
        ChevronUp => p.poly(&[(4.5, 10.0), (8.0, 6.5), (11.5, 10.0)], false),
        ChevronRight => p.poly(&[(6.0, 4.5), (9.5, 8.0), (6.0, 11.5)], false),
        ChevronLeft => p.poly(&[(10.0, 4.5), (6.5, 8.0), (10.0, 11.5)], false),
        Plus => {
            p.line(7.5, 2.0, 7.5, 13.0);
            p.line(2.0, 7.5, 13.0, 7.5);
        }
        Minus => p.line(2.0, 7.5, 13.0, 7.5),
        Eye | EyeOff => {
            // An almond (two arcs meeting at the corners) with the pupil.
            let (r, cy, half) = (7.0f32, 7.5f32, 5.5f32);
            let a = (half / r).asin();
            let d = r * a.cos();
            let mut e: Vec<(f32, f32)> = arc_pts(8.0, cy + d, r, -FRAC_PI_2 - a, -FRAC_PI_2 + a).collect();
            e.extend(arc_pts(8.0, cy - d, r, FRAC_PI_2 - a, FRAC_PI_2 + a));
            p.path(&e, true);
            if icon == Eye {
                p.circle(8.0, cy, 2.0);
            } else {
                p.line(3.0, 13.0, 13.0, 2.0);
            }
        }
        Lock | Unlock => {
            p.rect(3.5, 7.5, 12.5, 13.5, 1.0);
            p.dot(8.0, 10.5, 1.0);
            if icon == Lock {
                p.line(5.5, 7.5, 5.5, 5.5);
                p.line(10.5, 7.5, 10.5, 5.5);
                p.arc(8.0, 5.5, 2.5, PI, TAU);
            } else {
                p.line(10.5, 7.5, 10.5, 4.5);
                p.arc(13.0, 4.5, 2.5, PI, TAU);
            }
        }
        Component => draw_component(p),
        ComponentSet => draw_componentset(p),
        Instance => p.path(&[(8.0, 2.0), (14.0, 8.0), (8.0, 14.0), (2.0, 8.0)], true),
        Group => draw_group(p),
        Vector => {
            p.path(&[(3.0, 12.5), (6.0, 4.0), (10.0, 12.0), (13.0, 3.5)], false);
            p.fill_rect(1.5, 11.0, 4.5, 14.0, 0.0);
            p.fill_rect(11.5, 2.0, 14.5, 5.0, 0.0);
        }
        Boolean | Union => {
            p.rect(2.5, 2.5, 9.5, 9.5, 0.0);
            p.rect(6.5, 6.5, 13.5, 13.5, 0.0);
        }
        Subtract => {
            p.fill_rect(2.0, 2.0, 10.0, 10.0, 0.0);
            p.rect(6.5, 6.5, 13.5, 13.5, 0.0);
        }
        Intersect => {
            p.rect(2.5, 2.5, 9.5, 9.5, 0.0);
            p.rect(6.5, 6.5, 13.5, 13.5, 0.0);
            p.fill_rect(6.0, 6.0, 10.0, 10.0, 0.0);
        }
        Exclude => {
            p.fill_rect(2.0, 2.0, 10.0, 6.0, 0.0);
            p.fill_rect(2.0, 6.0, 6.0, 10.0, 0.0);
            p.fill_rect(10.0, 6.0, 14.0, 14.0, 0.0);
            p.fill_rect(6.0, 10.0, 10.0, 14.0, 0.0);
            p.rect(6.5, 6.5, 9.5, 9.5, 0.0);
        }
        Page => {
            p.poly(&[(3.5, 2.5), (9.5, 2.5), (12.5, 5.5), (12.5, 13.5), (3.5, 13.5)], true);
            p.poly(&[(9.5, 2.5), (9.5, 5.5), (12.5, 5.5)], false);
        }
        Check => p.poly(&[(3.0, 8.5), (6.5, 12.0), (13.0, 4.5)], false),
        Close => {
            p.line(4.0, 4.0, 12.0, 12.0);
            p.line(12.0, 4.0, 4.0, 12.0);
        }
        WinMinimize => p.line(3.0, 8.5, 13.0, 8.5),
        WinMaximize => p.rect(3.5, 3.5, 12.5, 12.5, 0.0),
        WinRestore => {
            // A window in front of another.
            p.rect(3.5, 5.5, 10.5, 12.5, 0.0);
            p.poly(&[(5.5, 5.5), (5.5, 3.5), (12.5, 3.5), (12.5, 10.5), (10.5, 10.5)], false);
        }
        Search => {
            p.circle(7.0, 7.0, 4.5);
            p.line(10.5, 10.5, 14.0, 14.0);
        }
        Grid | LayoutGrid => draw_grid_layoutgrid(p),
        AlignLeft => {
            p.line(2.5, 2.0, 2.5, 14.0);
            p.fill_rect(4.0, 4.0, 13.0, 7.0, 0.0);
            p.fill_rect(4.0, 9.0, 9.0, 12.0, 0.0);
        }
        AlignHCenter => {
            p.line(7.5, 2.0, 7.5, 14.0);
            p.fill_rect(2.0, 4.0, 13.0, 7.0, 0.0);
            p.fill_rect(4.0, 9.0, 11.0, 12.0, 0.0);
        }
        AlignRight => {
            p.line(13.5, 2.0, 13.5, 14.0);
            p.fill_rect(3.0, 4.0, 12.0, 7.0, 0.0);
            p.fill_rect(7.0, 9.0, 12.0, 12.0, 0.0);
        }
        AlignTop => {
            p.line(2.0, 2.5, 14.0, 2.5);
            p.fill_rect(4.0, 4.0, 7.0, 13.0, 0.0);
            p.fill_rect(9.0, 4.0, 12.0, 9.0, 0.0);
        }
        AlignVCenter => {
            p.line(2.0, 7.5, 14.0, 7.5);
            p.fill_rect(4.0, 2.0, 7.0, 13.0, 0.0);
            p.fill_rect(9.0, 4.0, 12.0, 11.0, 0.0);
        }
        AlignBottom => {
            p.line(2.0, 13.5, 14.0, 13.5);
            p.fill_rect(4.0, 3.0, 7.0, 12.0, 0.0);
            p.fill_rect(9.0, 7.0, 12.0, 12.0, 0.0);
        }
        DistributeH | Gap => {
            p.line(2.5, 2.0, 2.5, 14.0);
            p.line(13.5, 2.0, 13.5, 14.0);
            p.fill_rect(6.0, 4.0, 10.0, 12.0, 0.0);
        }
        DistributeV => {
            p.line(2.0, 2.5, 14.0, 2.5);
            p.line(2.0, 13.5, 14.0, 13.5);
            p.fill_rect(4.0, 6.0, 12.0, 10.0, 0.0);
        }
        Rotate => p.arc_arrow(8.0, 8.5, 5.0, -PI * 0.55, PI * 1.25),
        Reset => p.arc_arrow(8.0, 8.5, 5.0, -PI * 0.45, -PI * 2.25),
        FlipH => draw_fliph(p),
        FlipV => {
            for x in [1.0, 5.0, 9.0, 13.0] {
                p.line(x, 7.5, x + 2.0, 7.5);
            }
            p.poly(&[(3.5, 5.5), (12.5, 5.5), (12.5, 1.5)], true);
            p.fill(&[(3.0, 9.0), (13.0, 9.0), (13.0, 14.0)]);
        }
        CornerRadius | CornerTopLeft => p.corner(2.5, 2.5, 1.0, 1.0, 11.5, 6.0),
        CornerTopRight => p.corner(13.5, 2.5, -1.0, 1.0, 11.5, 6.0),
        CornerBottomRight => p.corner(13.5, 13.5, -1.0, -1.0, 11.5, 6.0),
        CornerBottomLeft => p.corner(2.5, 13.5, 1.0, -1.0, 11.5, 6.0),
        Angle => {
            // Two rays from a vertex, the arc between them marking the angle (all unhinted, so
            // the arc meets the rays at every size).
            p.path(&[(13.5, 13.0), (2.5, 13.0), (10.5, 3.0)], false);
            p.path(&arc_pts(2.5, 13.0, 6.0, (-10.0f32).atan2(8.0), 0.0).collect::<Vec<_>>(), false);
        }
        IndependentCorners => {
            for (x, y, dx, dy) in [(2.5, 2.5, 1.0, 1.0), (13.5, 2.5, -1.0, 1.0), (2.5, 13.5, 1.0, -1.0), (13.5, 13.5, -1.0, -1.0)] {
                p.corner(x, y, dx, dy, 4.0, 2.0);
            }
        }
        IndependentPadding => {
            // A frame outline with one inward tick per side: each side's padding, set alone.
            p.rect(2.5, 2.5, 13.5, 13.5, 2.0);
            p.line(8.0, 2.5, 8.0, 5.0);
            p.line(13.5, 8.0, 11.0, 8.0);
            p.line(8.0, 13.5, 8.0, 11.0);
            p.line(2.5, 8.0, 5.0, 8.0);
        }
        LinkOn | Link => {
            p.path(&capsule((4.0, 12.0), (6.5, 9.5), 2.4), true);
            p.path(&capsule((9.5, 6.5), (12.0, 4.0), 2.4), true);
            p.line(6.2, 9.8, 9.8, 6.2);
        }
        LinkOff => {
            p.path(&capsule((3.5, 12.5), (5.5, 10.5), 2.2), true);
            p.path(&capsule((10.5, 5.5), (12.5, 3.5), 2.2), true);
            p.line(9.5, 12.0, 9.5, 14.0);
            p.line(12.0, 9.5, 14.0, 9.5);
            p.line(2.0, 6.5, 4.0, 6.5);
            p.line(6.5, 2.0, 6.5, 4.0);
        }
        LayoutH => {
            p.fill_rect(2.0, 3.0, 7.0, 13.0, 1.0);
            p.fill_rect(9.0, 3.0, 14.0, 13.0, 1.0);
        }
        LayoutV => {
            p.fill_rect(3.0, 2.0, 13.0, 7.0, 1.0);
            p.fill_rect(3.0, 9.0, 13.0, 14.0, 1.0);
        }
        LayoutWrap => {
            let mut w = vec![(2.0, 4.5), (10.5, 4.5)];
            w.extend(arc_pts(10.5, 7.5, 3.0, -FRAC_PI_2, FRAC_PI_2));
            w.push((4.0, 10.5));
            p.path(&w, false);
            p.head((3.5, 10.5), PI, 2.8);
        }
        AutoLayout => {
            p.rect(2.5, 2.5, 13.5, 13.5, 2.0);
            p.fill_rect(5.0, 5.0, 7.0, 11.0, 0.0);
            p.fill_rect(9.0, 5.0, 11.0, 11.0, 0.0);
        }
        ClipContent => {
            p.rect(4.5, 4.5, 13.5, 13.5, 0.0);
            p.line(2.5, 1.0, 2.5, 11.0);
            p.line(1.0, 2.5, 11.0, 2.5);
        }
        Effects | Sun => {
            p.circle(8.0, 8.0, if icon == Sun { 2.5 } else { 3.0 });
            for i in 0..8 {
                let a = i as f32 * FRAC_PI_4;
                let (r0, r1) = if i % 2 == 0 { (4.8, 6.6) } else { (4.6, 5.8) };
                p.line(8.0 + r0 * a.cos(), 8.0 + r0 * a.sin(), 8.0 + r1 * a.cos(), 8.0 + r1 * a.sin());
            }
        }
        Styles => {
            for (x, y) in [(5.0, 5.0), (11.0, 5.0), (5.0, 11.0), (11.0, 11.0)] {
                p.dot(x, y, 2.2);
            }
        }
        Menu => {
            for y in [3.5, 7.5, 11.5] {
                p.line(2.0, y, 14.0, y);
            }
        }
        Play => p.fill(&[(4.5, 2.5), (13.5, 8.0), (4.5, 13.5)]),
        Home => {
            // Unhinted so the roof's peak stays centred; the floor stops at the door.
            p.path(&[(6.5, 13.5), (3.5, 13.5), (3.5, 7.0), (8.0, 2.8), (12.5, 7.0), (12.5, 13.5), (9.5, 13.5)], false);
            p.poly(&[(6.5, 13.5), (6.5, 9.5), (9.5, 9.5), (9.5, 13.5)], false);
        }
        Bell => {
            let mut b = vec![(2.5, 11.5), (4.0, 10.0), (4.0, 7.0)];
            b.extend(arc_pts(8.0, 7.0, 4.0, PI, TAU));
            b.extend([(12.0, 10.0), (13.5, 11.5)]);
            p.path(&b, true);
            p.arc(8.0, 12.0, 1.8, 0.0, PI);
        }
        Panels => {
            p.rect(2.5, 3.5, 13.5, 12.5, 1.0);
            p.line(5.5, 4.0, 5.5, 12.0);
            p.line(10.5, 4.0, 10.5, 12.0);
        }
        Help => {
            p.circle(8.0, 8.0, 6.0);
            p.arc(8.0, 6.4, 2.0, PI * 1.05, TAU + 0.7);
            p.path(&[(9.5, 7.7), (8.0, 8.8), (8.0, 9.8)], false);
            p.dot(8.0, 11.6, 0.9);
        }
        Dots => {
            for x in [3.5, 8.0, 12.5] {
                p.dot(x, 8.0, 1.25);
            }
        }
        Mask | Opacity => {
            p.circle(8.0, 8.0, 5.5);
            let side = if icon == Mask { -FRAC_PI_2 } else { FRAC_PI_2 };
            p.fill(&arc_pts(8.0, 8.0, 5.5, side, side + PI).collect::<Vec<_>>());
        }
        TextAlignLeft => {
            for (y, x1) in [(3.5, 14.0), (6.5, 10.0), (9.5, 14.0), (12.5, 9.0)] {
                p.line(2.0, y, x1, y);
            }
        }
        TextAlignCenter => {
            for (y, h) in [(3.5, 6.0), (6.5, 4.0), (9.5, 6.0), (12.5, 3.0)] {
                p.line(8.0 - h, y, 8.0 + h, y);
            }
        }
        TextAlignRight => {
            for (y, x0) in [(3.5, 2.0), (6.5, 6.0), (9.5, 2.0), (12.5, 7.0)] {
                p.line(x0, y, 14.0, y);
            }
        }
        TextAlignJustify => {
            for y in [3.5, 6.5, 9.5, 12.5] {
                p.line(2.0, y, 14.0, y);
            }
        }
        TextTop => {
            p.line(2.0, 2.5, 14.0, 2.5);
            p.line(7.5, 5.0, 7.5, 14.0);
            p.head((7.5, 5.0), -FRAC_PI_2, 3.2);
        }
        TextMiddle => {
            p.line(2.0, 7.5, 14.0, 7.5);
            p.line(7.5, 1.0, 7.5, 5.0);
            p.head((7.5, 5.0), FRAC_PI_2, 2.6);
            p.line(7.5, 10.0, 7.5, 14.0);
            p.head((7.5, 10.0), -FRAC_PI_2, 2.6);
        }
        TextBottom => {
            p.line(2.0, 13.5, 14.0, 13.5);
            p.line(7.5, 2.0, 7.5, 11.0);
            p.head((7.5, 11.0), FRAC_PI_2, 3.2);
        }
        Eyedropper => {
            p.poly(&[(2.5, 13.5), (3.0, 11.0), (8.5, 5.5), (10.5, 7.5), (5.0, 13.0)], true);
            p.path(&capsule((10.5, 5.5), (12.0, 4.0), 1.6), true);
            p.line(7.5, 4.5, 11.5, 8.5);
        }
        Code => {
            p.poly(&[(5.5, 4.0), (2.0, 8.0), (5.5, 12.0)], false);
            p.poly(&[(10.5, 4.0), (14.0, 8.0), (10.5, 12.0)], false);
            p.line(9.0, 3.0, 7.0, 13.0);
        }
        Assets => {
            p.path(&[(5.0, 2.0), (8.0, 5.0), (5.0, 8.0), (2.0, 5.0)], true);
            p.circle(11.0, 5.0, 2.5);
            p.rect(2.5, 9.5, 7.5, 13.5, 0.0);
            p.poly(&[(11.0, 9.0), (13.5, 13.5), (8.5, 13.5)], true);
        }
        Library => {
            p.poly(&[(7.5, 4.5), (5.5, 3.5), (1.5, 3.5), (1.5, 12.5), (5.5, 12.5), (7.5, 13.5)], false);
            p.poly(&[(8.5, 4.5), (10.5, 3.5), (14.5, 3.5), (14.5, 12.5), (10.5, 12.5), (8.5, 13.5)], false);
            p.line(7.5, 4.5, 7.5, 13.5);
            p.line(8.5, 4.5, 8.5, 13.5);
        }
        Layers => {
            p.poly(&[(8.0, 2.5), (14.0, 5.5), (8.0, 8.5), (2.0, 5.5)], true);
            p.poly(&[(2.0, 8.0), (8.0, 11.0), (14.0, 8.0)], false);
            p.poly(&[(2.0, 10.5), (8.0, 13.5), (14.0, 10.5)], false);
        }
        Blend => {
            p.circle(6.0, 8.0, 4.0);
            p.circle(10.0, 8.0, 4.0);
        }
        Export => {
            p.line(7.5, 2.0, 7.5, 10.0);
            p.head((7.5, 2.0), -FRAC_PI_2, 3.5);
            p.poly(&[(2.5, 8.5), (2.5, 13.5), (12.5, 13.5), (12.5, 8.5)], false);
        }
        StrokeIcon => p.rect(2.5, 2.5, 13.5, 13.5, 2.0),
        Fill => p.fill_rect(2.0, 2.0, 14.0, 14.0, 2.0),
        GradientLinear => {
            p.rect(2.5, 2.5, 13.5, 13.5, 2.0);
            p.shade(&[(4.0, 4.0, 1.0), (12.0, 4.0, 0.0), (12.0, 12.0, 0.0), (4.0, 12.0, 1.0)], &[0, 1, 2, 0, 2, 3]);
        }
        GradientRadial => {
            p.rect(2.5, 2.5, 13.5, 13.5, 2.0);
            p.shade_fan(5.5, |_| 1.0, |_| 0.0);
        }
        GradientAngular => {
            // Once round from the top: the seam shows where the sweep starts.
            p.rect(2.5, 2.5, 13.5, 13.5, 2.0);
            p.shade_fan(4.0, |t| 1.0 - t, |t| 1.0 - t);
        }
        GradientDiamond => {
            p.rect(2.5, 2.5, 13.5, 13.5, 2.0);
            let v = [(8.0, 8.0, 1.0), (8.0, 2.5, 0.0), (13.5, 8.0, 0.0), (8.0, 13.5, 0.0), (2.5, 8.0, 0.0)];
            p.shade(&v, &[0, 1, 2, 0, 2, 3, 0, 3, 4, 0, 4, 1]);
        }
        Moon => {
            // A crescent: the outer rim, then the shadowing circle's edge back. Both arcs stop short
            // of where the circles meet, so each tip is two blunt joins (a single cusp join folds).
            let mut m: Vec<(f32, f32)> = arc_pts(8.0, 8.0, 5.5, 0.166 + 0.12, TAU - 1.737 - 0.12).collect();
            m.extend(arc_pts(11.0, 5.0, 4.6, -2.586 - 0.18, 1.016 - TAU + 0.18));
            p.path(&m, true);
        }
        Undo | Redo => {
            // A U-turn arrow: back along the top, round, and out along the bottom.
            let undo = icon == Undo;
            let f = |x: f32| if undo { x } else { 16.0 - x };
            let mut u = vec![(f(3.0), 5.5), (f(9.5), 5.5)];
            let a1 = if undo { FRAC_PI_2 } else { -3.0 * FRAC_PI_2 };
            u.extend(arc_pts(f(9.5), 9.0, 3.5, -FRAC_PI_2, a1));
            u.push((f(6.0), 12.5));
            p.path(&u, false);
            p.head((f(3.0), 5.5), if undo { PI } else { 0.0 }, 3.0);
        }
        Hug => {
            p.poly(&[(2.5, 4.5), (5.5, 7.5), (2.5, 10.5)], false);
            p.poly(&[(13.5, 4.5), (10.5, 7.5), (13.5, 10.5)], false);
            p.line(6.0, 7.5, 10.0, 7.5);
        }
        FillContainer => {
            p.poly(&[(5.0, 4.5), (2.0, 7.5), (5.0, 10.5)], false);
            p.poly(&[(11.0, 4.5), (14.0, 7.5), (11.0, 10.5)], false);
            p.line(2.0, 7.5, 14.0, 7.5);
        }
        Fixed => {
            p.line(2.5, 4.0, 2.5, 11.0);
            p.line(13.5, 4.0, 13.5, 11.0);
            p.line(3.0, 7.5, 13.0, 7.5);
        }
        PaddingH => {
            // Horizontal padding: bars down the padded left and right sides in their frame.
            p.rect(2.5, 2.5, 13.5, 13.5, 1.0);
            p.fill_rect(2.5, 2.5, 5.5, 13.5, 1.0);
            p.fill_rect(10.5, 2.5, 13.5, 13.5, 1.0);
        }
        PaddingV => {
            // Vertical padding: bars across the padded top and bottom in their frame.
            p.rect(2.5, 2.5, 13.5, 13.5, 1.0);
            p.fill_rect(2.5, 2.5, 13.5, 5.5, 1.0);
            p.fill_rect(2.5, 10.5, 13.5, 13.5, 1.0);
        }
        PaddingTop => {
            p.rect(2.5, 2.5, 13.5, 13.5, 1.0);
            p.fill_rect(2.5, 2.5, 13.5, 5.5, 1.0);
        }
        PaddingRight => {
            p.rect(2.5, 2.5, 13.5, 13.5, 1.0);
            p.fill_rect(10.5, 2.5, 13.5, 13.5, 1.0);
        }
        PaddingBottom => {
            p.rect(2.5, 2.5, 13.5, 13.5, 1.0);
            p.fill_rect(2.5, 10.5, 13.5, 13.5, 1.0);
        }
        PaddingLeft => {
            p.rect(2.5, 2.5, 13.5, 13.5, 1.0);
            p.fill_rect(2.5, 2.5, 5.5, 13.5, 1.0);
        }
        Online => {
            p.circle(8.0, 8.0, 5.5);
            p.line(2.5, 8.0, 13.5, 8.0);
            let meridian: Vec<(f32, f32)> = arc_pts(0.0, 0.0, 1.0, 0.0, TAU).map(|(x, y)| (8.0 + 2.4 * x, 8.0 + 5.5 * y)).collect();
            p.path(&meridian, true);
        }
        LiveShared => {
            p.dot(8.0, 8.0, 1.6);
            for r in [3.6, 6.0] {
                p.arc(8.0, 8.0, r, -FRAC_PI_4, FRAC_PI_4);
                p.arc(8.0, 8.0, r, PI - FRAC_PI_4, PI + FRAC_PI_4);
            }
        }
        LivePrivate => {
            p.dot(8.0, 8.0, 1.6);
            p.circle(8.0, 8.0, 5.0);
        }
        Clock => {
            p.circle(7.5, 7.5, 5.5);
            p.line(7.5, 4.0, 7.5, 8.0);
            p.line(7.5, 7.5, 10.0, 10.0);
        }
        ShapeBuilder | ShapeBuilderRemove => {
            // Two overlapping rings; their shared lens filled (merge) or crossed out (remove).
            let (r, d) = (4.5f32, 2.0f32);
            let a = (d / r).acos();
            p.circle(8.0 - d, 8.0, r);
            p.circle(8.0 + d, 8.0, r);
            if icon == ShapeBuilder {
                let lens: Vec<(f32, f32)> = arc_pts(8.0 - d, 8.0, r, -a, a).chain(arc_pts(8.0 + d, 8.0, r, PI - a, PI + a)).collect();
                p.fill(&lens);
            } else {
                p.line(6.2, 9.8, 9.8, 6.2);
            }
        }
        Eraser => {
            // A tilted block with a band near its tip, over a short trace line.
            p.poly(&[(2.5, 10.0), (9.5, 3.0), (13.5, 7.0), (7.0, 13.5), (6.0, 13.5)], true);
            p.line(5.5, 7.0, 9.5, 11.0);
            p.line(7.0, 13.5, 14.0, 13.5);
        }
        VariableWidth => {
            // A lens-shaped stroke, thin at both ends, and a width handle on either side of its
            // widest point.
            let edge = |i: i32, side: f32| {
                let f = i as f32 / 12.0;
                (2.0 + 11.0 * f, 7.5 + side * 3.0 * (PI * f).sin())
            };
            let lens: Vec<(f32, f32)> = (0..=12).map(|i| edge(i, -1.0)).chain((0..=12).rev().map(|i| edge(i, 1.0))).collect();
            p.fill(&lens);
            p.line(7.5, 2.0, 7.5, 13.0);
            p.dot(7.5, 2.0, 1.5);
            p.dot(7.5, 13.0, 1.5);
        }
        Variable => {
            // A hexagon with a dot: an original mark for "bind a value".
            let pts: Vec<(f32, f32)> = (0..6)
                .map(|i| {
                    let a = std::f32::consts::FRAC_PI_3 * i as f32 + std::f32::consts::FRAC_PI_6;
                    (8.0 + 6.0 * a.cos(), 8.0 + 6.0 * a.sin())
                })
                .collect();
            p.path(&pts, true);
            p.dot(8.0, 8.0, 1.7);
        }
        Detach => {
            p.path(&[(8.0, 2.0), (14.0, 8.0), (8.0, 14.0), (2.0, 8.0)], true);
            p.line(3.0, 13.0, 13.0, 3.0);
        }
        Swap => {
            p.line(2.0, 4.5, 13.0, 4.5);
            p.head((13.0, 4.5), 0.0, 2.8);
            p.line(3.0, 11.5, 14.0, 11.5);
            p.head((3.0, 11.5), PI, 2.8);
        }
        Prototype => {
            p.circle(4.5, 11.5, 2.0);
            p.line(6.0, 10.0, 13.0, 3.0);
            p.poly(&[(8.5, 2.5), (13.5, 2.5), (13.5, 7.5)], false);
        }
        Preview => {
            // A window with a title strip and a play triangle in it.
            p.rect(2.5, 2.5, 13.5, 13.5, 1.0);
            p.line(3.0, 5.5, 13.0, 5.5);
            p.fill(&[(6.5, 7.5), (10.5, 10.0), (6.5, 12.5)]);
        }
        AbsolutePosition => {
            // The frame, a small layer near its top-right corner, and the two offsets pinning it.
            p.rect(2.5, 2.5, 13.5, 13.5, 1.0);
            p.fill_rect(6.0, 6.0, 10.0, 10.0, 0.0);
            p.line(10.0, 7.5, 13.0, 7.5);
            p.line(7.5, 3.0, 7.5, 6.0);
        }
        Slot => {
            p.dashed_box(2.5, 2.5, 12.5, 12.5, 3);
            p.line(7.5, 5.0, 7.5, 10.0);
            p.line(5.0, 7.5, 10.0, 7.5);
        }
        Adjust => {
            p.line(2.0, 4.5, 14.0, 4.5);
            p.line(2.0, 11.5, 14.0, 11.5);
            p.dot(10.5, 4.5, 2.2);
            p.dot(5.5, 11.5, 2.2);
        }
        ViewList => {
            for y in [2.0, 6.0, 10.0] {
                p.fill_rect(2.0, y, 5.0, y + 3.0, 0.0);
                p.line(7.0, y + 1.5, 14.0, y + 1.5);
            }
        }
        Reaction => {
            p.circle(8.0, 8.0, 5.5);
            p.dot(6.0, 6.5, 0.9);
            p.dot(10.0, 6.5, 0.9);
            p.arc(8.0, 8.5, 3.0, 0.6, PI - 0.6);
        }
        DragHandle => {
            for y in [4.0, 8.0, 12.0] {
                p.dot(6.0, y, 1.1);
                p.dot(10.0, y, 1.1);
            }
        }
        ViewGrid => {
            for (x, y) in [(2.5, 2.5), (9.5, 2.5), (2.5, 9.5), (9.5, 9.5)] {
                p.rect(x, y, x + 4.0, y + 4.0, 1.0);
            }
        }
        ArcStart => {
            p.circle(8.0, 8.0, 5.5);
            p.line(8.0, 8.0, 13.5, 8.0);
            p.dot(13.5, 8.0, 1.6);
        }
        ArcSweep => {
            let mut wedge = vec![(8.0, 8.0)];
            wedge.extend(arc_pts(8.0, 8.0, 5.5, 0.0, 1.5 * PI));
            p.path(&wedge, true);
        }
        ArcRatio => {
            p.circle(8.0, 8.0, 5.5);
            p.circle(8.0, 8.0, 2.5);
        }
        GamutWarning => {
            p.poly(&[(8.0, 2.0), (14.5, 13.5), (1.5, 13.5)], true);
            p.line(8.0, 6.0, 8.0, 9.5);
            p.dot(8.0, 11.5, 0.9);
        }
        PenNib => {
            // Drawn pointing down, then leant 35° to the left about the middle.
            let (sin, cos) = (0.61f32, 0.79f32);
            let at = |x: f32, y: f32| (8.0 + x * cos - y * sin, 8.0 + x * sin + y * cos);
            let nib: Vec<(f32, f32)> = [(0.0, 6.5), (3.4, 1.0), (2.4, -2.5), (-2.4, -2.5), (-3.4, 1.0)].iter().map(|&(x, y)| at(x, y)).collect();
            p.path(&nib, true);
            let (a, b) = (at(0.0, 6.5), at(0.0, 2.0));
            p.path(&[a, b], false);
            let (dx, dy) = at(0.0, 0.6);
            p.dot(dx, dy, 1.0);
            let grip: Vec<(f32, f32)> = [(-2.4, -3.4), (2.4, -3.4), (2.4, -5.6), (-2.4, -5.6)].iter().map(|&(x, y)| at(x, y)).collect();
            p.fill(&grip);
        }
        Board => {
            p.poly(&[(2.5, 2.5), (13.5, 2.5), (13.5, 9.5), (9.5, 13.5), (2.5, 13.5)], true);
            p.fill(&[(9.5, 13.5), (9.5, 9.5), (13.5, 9.5)]);
            p.line(5.0, 6.5, 11.0, 6.5);
            p.line(5.0, 9.5, 8.0, 9.5);
        }
        Templates => {
            p.rect(1.5, 2.5, 14.5, 13.5, 1.5);
            p.fill_rect(3.5, 4.5, 7.5, 8.5, 0.5);
            p.line(9.5, 5.5, 12.5, 5.5);
            p.line(9.5, 7.5, 12.5, 7.5);
            p.line(3.5, 11.0, 12.5, 11.0);
        }
        Table => {
            p.rect(1.5, 2.5, 14.5, 13.5, 1.0);
            p.line(1.5, 6.5, 14.5, 6.5);
            p.line(1.5, 10.0, 14.5, 10.0);
            p.line(6.0, 2.5, 6.0, 13.5);
            p.line(10.0, 2.5, 10.0, 13.5);
        }
        AddRow | RemoveRow => {
            p.rect(1.5, 2.5, 14.5, 8.5, 1.0);
            p.line(8.0, 2.5, 8.0, 8.5);
            p.line(5.5, 12.5, 10.5, 12.5);
            if icon == AddRow {
                p.line(8.0, 10.0, 8.0, 15.0);
            }
        }
        AddColumn | RemoveColumn => {
            p.rect(2.5, 1.5, 8.5, 14.5, 1.0);
            p.line(2.5, 8.0, 8.5, 8.0);
            p.line(10.0, 8.0, 15.0, 8.0);
            if icon == AddColumn {
                p.line(12.5, 5.5, 12.5, 10.5);
            }
        }
        Marker => {
            p.poly(&[(3.0, 13.0), (3.0, 10.5), (10.0, 3.5), (12.5, 6.0), (5.5, 13.0)], true);
            p.line(8.5, 5.0, 11.0, 7.5);
            p.fill(&[(2.0, 14.0), (2.0, 11.5), (4.5, 14.0)]);
        }
        Highlighter => {
            p.poly(&[(4.0, 10.0), (10.5, 3.5), (13.5, 6.5), (7.0, 13.0)], true);
            p.poly(&[(4.0, 10.0), (2.5, 12.5), (4.5, 12.5), (7.0, 13.0)], false);
            p.fill_rect(1.0, 14.0, 15.0, 15.5, 0.0);
        }
        Connector => {
            p.rect(1.5, 1.5, 6.5, 6.5, 1.0);
            p.rect(9.5, 9.5, 14.5, 14.5, 1.0);
            p.poly(&[(6.5, 4.0), (12.0, 4.0), (12.0, 9.0)], false);
            p.head((12.0, 9.0), FRAC_PI_2, 2.4);
        }
        CurvedConnector => {
            p.rect(1.5, 1.5, 6.5, 6.5, 1.0);
            p.rect(9.5, 9.5, 14.5, 14.5, 1.0);
            p.arc(6.5, 9.0, 5.5, -FRAC_PI_2, 0.0);
            p.head((12.0, 9.0), FRAC_PI_2, 2.4);
        }
        Shapes => {
            p.rect(1.5, 5.5, 9.5, 13.5, 1.0);
            p.circle(10.5, 5.5, 4.0);
        }
        Stamp => {
            p.circle(8.0, 8.0, 6.0);
            p.poly(&[(5.0, 8.3), (7.2, 10.5), (11.0, 6.0)], false);
        }
        Timer => {
            p.circle(8.0, 9.0, 5.5);
            p.line(8.0, 9.0, 8.0, 6.0);
            p.line(6.0, 1.5, 10.0, 1.5);
            p.line(8.0, 1.5, 8.0, 3.5);
            p.line(12.0, 4.0, 13.5, 2.5);
        }
        Pause => {
            p.fill_rect(4.0, 3.0, 6.5, 13.0, 0.5);
            p.fill_rect(9.5, 3.0, 12.0, 13.0, 0.5);
        }
        Vote => {
            p.rect(2.5, 8.5, 13.5, 13.5, 1.0);
            p.poly(&[(5.5, 8.5), (5.5, 2.5), (10.5, 2.5), (10.5, 8.5)], false);
            p.poly(&[(6.6, 5.6), (7.7, 6.7), (9.4, 4.6)], false);
        }
        Measure => {
            p.line(2.5, 4.0, 2.5, 12.0);
            p.line(13.5, 4.0, 13.5, 12.0);
            p.line(3.0, 8.0, 13.0, 8.0);
            p.poly(&[(5.5, 5.5), (3.0, 8.0), (5.5, 10.5)], false);
            p.poly(&[(10.5, 5.5), (13.0, 8.0), (10.5, 10.5)], false);
        }
        Logo => {
            // FigCraft's own mark: the curled tail of the chameleon on the app icon. One bold
            // stroke: a shaft along the bottom, then three half turns, each tighter than the last.
            let mut tail = vec![(13.75, 13.0)];
            tail.extend(arc_pts(7.25, 8.0, 5.0, FRAC_PI_2, 3.0 * FRAC_PI_2));
            tail.extend(arc_pts(7.25, 6.5, 3.5, 3.0 * FRAC_PI_2, 5.0 * FRAC_PI_2));
            tail.extend(arc_pts(7.25, 8.25, 1.75, FRAC_PI_2, 3.0 * FRAC_PI_2));
            let v: Vec<Pos2> = tail.iter().map(|&(x, y)| p.raw(x, y)).collect();
            p.painter.add(Shape::line(v, Stroke::new(1.7 * p.s, p.color)));
            p.dot(13.75, 13.0, 0.85);
            p.dot(7.25, 6.5, 0.85);
        }
    }
}

/// SVG path data outlines on a `size`-unit box, painted into `rect`: each part filled, or
/// stroked `width` box units wide. Board shapes and stamp marks are drawn this way in the UI from
/// the same outlines the canvas has.
pub fn paint_outlines(painter: &Painter, rect: Rect, size: f64, parts: &[(&figcraft_geom::BezPath, bool)], color: Color32, width: f32) {
    let s = rect.width().min(rect.height()) / size as f32;
    let px = 1.0 / painter.ctx().pixels_per_point();
    let o = rect.center() - Vec2::splat(size as f32 * s / 2.0);
    let p = P { painter, o, s, px, stroke: Stroke::new((width * s).max(px), color), color };
    for (path, filled) in parts {
        let mut sub: Vec<(f32, f32)> = Vec::new();
        let flush = |sub: &mut Vec<(f32, f32)>, closed: bool| {
            if sub.len() >= 2 {
                if *filled { p.fill(sub) } else { p.path(sub, closed) }
            }
            sub.clear();
        };
        let mut closed = false;
        figcraft_geom::kurbo::flatten(path.iter(), 0.25, |el| match el {
            figcraft_geom::kurbo::PathEl::MoveTo(q) => {
                flush(&mut sub, closed);
                closed = false;
                sub.push((q.x as f32, q.y as f32));
            }
            figcraft_geom::kurbo::PathEl::LineTo(q) => sub.push((q.x as f32, q.y as f32)),
            figcraft_geom::kurbo::PathEl::ClosePath => closed = true,
            _ => {}
        });
        flush(&mut sub, closed);
    }
}
