//! The fill-rule panel the Fill section opens: the selected vector's filled areas drawn with the
//! rule each one fills by (non-zero in blue, even-odd in orange, hatched like the vector edit mode's
//! hover), every loop with the direction it runs in. A click on an area switches it between the two
//! rules, a click on a loop turns the loop round — which is how a hole gets its winding back. The
//! commands behind it, `vector.fillRule` and `vector.reverseLoop`, live in the engine pull request
//! #740 (this panel is the UI half, split out so it can be reviewed on its own).
//!
//! The panel is a popover to the left of the Design panel, opened from the Fill section's header
//! (beside Styles and Add fill). It edits the layer it opened for and closes when that layer leaves
//! the selection, on Escape and on a press outside it.

use egui::{Align2, Color32, CornerRadius, Id, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use figcraft_doc::{Document, NodeId, NodeKind, VectorNetwork, WindingRule};
use figcraft_engine::vectoredit::regions::{HATCH_DIRECTION, clip_to_area, hatch_lines};
use figcraft_geom::kurbo::{ParamCurve, ParamCurveDeriv, ParamCurveNearest, PathEl};
use figcraft_geom::{BezPath, Point, Rect as Bounds, Shape};
use serde_json::json;

use crate::FigApp;
use crate::i18n::tr;
use crate::icons::Icon;
use crate::theme::{Tokens, regular, semibold};
use crate::widgets;

/// The hatched areas of each region, in preview space, for the cached preview.
type Hatches = Vec<Vec<[Pos2; 2]>>;
/// The cached preview geometry: the document revision it was built for, the preview rect, and the
/// hatched areas of each region (in preview space).
type PreviewCache = Option<(u64, Rect, Hatches)>;

/// The panel's width and the height of the preview in it.
pub const W: f32 = 300.0;
const PREVIEW_H: f32 = 190.0;
/// The room the shape leaves around itself in the preview (screen pixels).
const PAD: f64 = 16.0;
/// The distance between hatch lines in the preview, and how thick they are (screen pixels).
const HATCH_STEP: f64 = 5.0;
const HATCH_W: f32 = 2.4;
/// A loop this close to the pointer (screen pixels) is the one a click turns round.
const LOOP_TOL: f64 = 3.5;
/// Non-zero areas are blue, even-odd ones orange (the legend's two swatches).
const NONZERO: Color32 = Color32::from_rgb(0x3F, 0x9F, 0xFF);
const EVENODD: Color32 = Color32::from_rgb(0xFF, 0x7F, 0x00);

/// What the preview shows under the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// A filled area, which a click switches between non-zero and even-odd.
    Area(usize),
    /// Loop `loop_` of area `region`, which a click turns round.
    Loop { region: usize, loop_: usize },
}

/// The fill-rule panel, while open.
pub struct Panel {
    /// The vector layer it edits.
    pub id: NodeId,
    /// The screen y it opened at (the Fill section's).
    pub y: f32,
    /// The preview as drawn last frame: where its clicks land.
    pub preview: Rect,
    /// The area or loop under the pointer.
    pub hover: Option<Hit>,
    /// The preview's hatched areas, cached for `(revision, preview_rect)` so they are not
    /// re-flattened and re-clipped every frame the panel is open; rebuilt when the document or
    /// the panel's size changes.
    pub cache: PreviewCache,
}

impl Panel {
    pub fn open(id: NodeId, y: f32) -> Self {
        Self { id, y, preview: Rect::NOTHING, hover: None, cache: None }
    }
}

/// What a click in the preview asks for.
enum Act {
    Rule { region: usize, rule: WindingRule },
    Reverse { region: usize, loop_: usize },
}

/// The vector network of `id`, when it is a vector layer.
pub fn network(doc: &Document, id: NodeId) -> Option<&VectorNetwork> {
    match &doc.node(id)?.kind {
        NodeKind::Vector { network } => Some(network),
        _ => None,
    }
}

/// The first vector layer of `ids`.
pub fn first_vector(doc: &Document, ids: &[NodeId]) -> Option<NodeId> {
    ids.iter().copied().find(|i| network(doc, *i).is_some())
}

/// Opens the panel for the selection's first vector layer at `y` (false when there is none).
pub fn open(app: &mut FigApp, y: f32) -> bool {
    let Some(id) = app.session.doc().ok().and_then(|d| first_vector(d, &app.session.selection())) else { return false };
    app.ui.fill_rule = Some(Panel::open(id, y));
    true
}

/// The Fill section's button: opens the panel for the selection's first vector layer at `y`, or
/// closes it when it is open for that layer already.
pub fn toggle(app: &mut FigApp, y: f32) -> bool {
    let Some(id) = app.session.doc().ok().and_then(|d| first_vector(d, &app.session.selection())) else { return false };
    app.ui.fill_rule = match app.ui.fill_rule.take() {
        Some(p) if p.id == id => None,
        _ => Some(Panel::open(id, y)),
    };
    true
}

/// Where local point `q` of `net` shows in `preview` (the fit the preview draws with).
pub fn preview_point(net: &VectorNetwork, preview: Rect, q: Point) -> Option<Pos2> {
    Fit::new(net, preview).map(|f| f.to_screen(q))
}

/// The hatch of area `region` as drawn in `preview`: one segment per run of a hatch line inside
/// the area (clipped by the area's own fill rule, so holes stay clear).
pub fn area_hatch(net: &VectorNetwork, region: usize, preview: Rect) -> Vec<[Pos2; 2]> {
    let Some(area) = net.regions.get(region) else { return Vec::new() };
    let (Some(fit), Some(path)) = (Fit::new(net, preview), net.region_path(region)) else { return Vec::new() };
    let path = fit.path(&path);
    let polys = polygons(&path, 0.3);
    let full = path.bounding_box();
    let clip = Bounds::new(f64::from(preview.min.x), f64::from(preview.min.y), f64::from(preview.max.x), f64::from(preview.max.y)).inset(-1.0);
    let b = full.intersect(clip);
    if polys.is_empty() || b.width() <= 0.0 || b.height() <= 0.0 {
        return Vec::new();
    }
    let anchor = Point::new(f64::from(preview.min.x), f64::from(preview.min.y));
    let reach = full.width().hypot(full.height());
    let mut out = Vec::new();
    for l in hatch_lines(b, anchor, HATCH_DIRECTION, HATCH_STEP) {
        let mid = l.a.midpoint(l.b);
        let r = reach + (mid - full.center()).hypot() + HATCH_STEP;
        for seg in clip_to_area(&polys, area.winding, mid - HATCH_DIRECTION * r, mid + HATCH_DIRECTION * r) {
            // The polygons and the lines are in preview space already (the fit mapped the path).
            out.push([pos2(seg[0].x as f32, seg[0].y as f32), pos2(seg[1].x as f32, seg[1].y as f32)]);
        }
    }
    out
}

/// The hatched areas for `net`, cached on `p` by `(revision, preview_rect)` so the flatten + clip
/// the preview does every frame is only redone when the document or the panel's size changes.
fn cached_hatch(p: &mut Panel, net: &VectorNetwork, rect: Rect, rev: u64) -> Hatches {
    if let Some((r, rc, h)) = &p.cache
        && *r == rev
        && *rc == rect
    {
        return h.clone();
    }
    let mut h = Vec::with_capacity(net.regions.len());
    for i in 0..net.regions.len() {
        h.push(area_hatch(net, i, rect));
    }
    p.cache = Some((rev, rect, h.clone()));
    h
}

/// The network as the fill draws it: rounded points drawn in (area indexes are kept).
fn drawn(net: &VectorNetwork) -> std::borrow::Cow<'_, VectorNetwork> {
    match net.rounded() {
        Some(r) => std::borrow::Cow::Owned(r),
        None => std::borrow::Cow::Borrowed(net),
    }
}

/// A rule's hatch colour, stronger under the pointer.
fn hatch(rule: WindingRule, hovered: bool) -> Color32 {
    let c = match rule {
        WindingRule::NonZero => NONZERO,
        WindingRule::EvenOdd => EVENODD,
    };
    c.gamma_multiply(if hovered { 0.85 } else { 0.4 })
}

/// Where the network sits in the preview: `scale` preview pixels per local unit about `from`→`to`.
struct Fit {
    scale: f64,
    from: Point,
    to: Pos2,
}

impl Fit {
    /// The network `net` fitted into `preview`, its box centred with `PAD` to spare.
    fn new(net: &VectorNetwork, preview: Rect) -> Option<Self> {
        let b = bounds(net)?;
        let w = (f64::from(preview.width()) - 2.0 * PAD).max(1.0);
        let h = (f64::from(preview.height()) - 2.0 * PAD).max(1.0);
        let scale = (w / b.width().max(1e-6)).min(h / b.height().max(1e-6));
        if !scale.is_finite() || scale <= 0.0 {
            return None;
        }
        Some(Self { scale, from: b.center(), to: preview.center() })
    }

    /// A local point in preview space.
    fn map(&self, q: Point) -> Point {
        Point::new(f64::from(self.to.x) + (q.x - self.from.x) * self.scale, f64::from(self.to.y) + (q.y - self.from.y) * self.scale)
    }

    fn to_screen(&self, q: Point) -> Pos2 {
        let m = self.map(q);
        pos2(m.x as f32, m.y as f32)
    }

    fn to_local(&self, q: Pos2) -> Point {
        Point::new(
            self.from.x + (f64::from(q.x) - f64::from(self.to.x)) / self.scale,
            self.from.y + (f64::from(q.y) - f64::from(self.to.y)) / self.scale,
        )
    }

    fn path(&self, path: &BezPath) -> BezPath {
        let p = |q: Point| self.map(q);
        let mut out = BezPath::new();
        for el in path.elements() {
            match *el {
                PathEl::MoveTo(q) => out.move_to(p(q)),
                PathEl::LineTo(q) => out.line_to(p(q)),
                PathEl::QuadTo(a, b) => out.quad_to(p(a), p(b)),
                PathEl::CurveTo(a, b, c) => out.curve_to(p(a), p(b), p(c)),
                PathEl::ClosePath => out.close_path(),
            }
        }
        out
    }
}

/// The network's box: its curves', else its points'.
fn bounds(net: &VectorNetwork) -> Option<Bounds> {
    if let Some(b) = net.bounds().filter(|b| b.width() > 0.0 || b.height() > 0.0) {
        return Some(b);
    }
    let mut b: Option<Bounds> = None;
    for v in &net.vertices {
        if !(v.x.is_finite() && v.y.is_finite()) {
            continue;
        }
        b = Some(match b {
            None => Bounds::new(v.x, v.y, v.x, v.y),
            Some(r) => Bounds::new(r.x0.min(v.x), r.y0.min(v.y), r.x1.max(v.x), r.y1.max(v.y)),
        });
    }
    b
}

/// The closed polygons of a path, flattened `tol` close.
fn polygons(path: &BezPath, tol: f64) -> Vec<Vec<Point>> {
    let mut polys: Vec<Vec<Point>> = Vec::new();
    figcraft_geom::kurbo::flatten(path.iter(), tol, |el| match el {
        PathEl::MoveTo(p) => polys.push(vec![p]),
        PathEl::LineTo(p) => {
            if let Some(l) = polys.last_mut() {
                l.push(p);
            }
        }
        _ => {}
    });
    polys.retain(|p| p.len() > 2);
    polys
}

/// The area or loop under `at` (screen): a loop first, so a hole's edge is the loop rather than the
/// area around it.
fn hit(net: &VectorNetwork, fit: &Fit, at: Pos2) -> Option<Hit> {
    let q = fit.to_local(at);
    if !(q.x.is_finite() && q.y.is_finite()) {
        return None;
    }
    let tol = LOOP_TOL / fit.scale;
    for (region, r) in net.regions.iter().enumerate() {
        for (loop_, lp) in r.loops.iter().enumerate() {
            for (seg, _) in net.loop_walk(lp) {
                if let Some(c) = net.cubic(seg)
                    && c.nearest(q, 0.05).distance_sq.sqrt() <= tol
                {
                    return Some(Hit::Loop { region, loop_ });
                }
            }
        }
    }
    net.region_at(q).map(Hit::Area)
}

/// A small arrowhead on segment `c` at its middle, pointing the way the loop runs it.
fn arrow(painter: &egui::Painter, c: figcraft_geom::kurbo::CubicBez, back: bool, fit: &Fit, col: Color32) {
    let mut d = c.deriv().eval(0.5).to_vec2();
    if back {
        d = -d;
    }
    let len = d.hypot();
    if !len.is_finite() || len <= 1e-9 {
        return;
    }
    let (ux, uy) = (d.x / len, d.y / len);
    let at = fit.to_screen(c.eval(0.5));
    let (dx, dy) = ((ux * 4.6) as f32, (uy * 4.6) as f32);
    let (px, py) = (-dy * 0.42, dx * 0.42);
    let head = vec![pos2(at.x + dx, at.y + dy), pos2(at.x - dx + px, at.y - dy + py), pos2(at.x - dx - px, at.y - dy - py)];
    painter.add(egui::Shape::convex_polygon(head, col, Stroke::NONE));
}

/// A swatch of `rule`'s hatching (the legend's icon), on a field-coloured square.
fn swatch(painter: &egui::Painter, rect: Rect, rule: WindingRule, t: &Tokens) {
    painter.rect_filled(rect, CornerRadius::same(3), t.field);
    let p = painter.with_clip_rect(rect);
    let col = hatch(rule, false).gamma_multiply(1.6);
    let mut x = rect.min.x - rect.height();
    while x < rect.max.x {
        p.line_segment([pos2(x, rect.max.y), pos2(x + rect.height(), rect.min.y)], Stroke::new(1.8, col));
        x += 4.0;
    }
    painter.rect_stroke(rect, CornerRadius::same(3), Stroke::new(1.0, t.border_strong), egui::StrokeKind::Inside);
}

/// A legend entry: the rule's swatch and its name, side by side.
fn legend(ui: &mut Ui, t: &Tokens, rule: WindingRule, label: &str) {
    let galley = ui.painter().layout_no_wrap(label.to_owned(), regular(11.0), t.text_secondary);
    let (rect, _) = ui.allocate_exact_size(vec2(16.0 + 6.0 + galley.size().x, 16.0), Sense::hover());
    let sw = Rect::from_min_size(pos2(rect.min.x, rect.center().y - 8.0), vec2(16.0, 16.0));
    swatch(ui.painter(), sw, rule, t);
    ui.painter().galley(pos2(sw.max.x + 6.0, rect.center().y - galley.size().y / 2.0), galley, t.text_secondary);
}

/// The panel's contents: the title and the X that closes it, the preview, then the legend and what
/// the clicks do.
fn contents(ui: &mut Ui, t: &Tokens, p: &mut Panel, net: &VectorNetwork, act: &mut Option<Act>, rev: u64) -> bool {
    ui.set_width(W);
    // The fill draws rounded points in: the preview shows the areas as they fill.
    let net = drawn(net);
    let net = net.as_ref();
    // 1. The title, with the X at its right.
    let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
    ui.painter().text(pos2(head.min.x + 2.0, head.center().y), Align2::LEFT_CENTER, tr("Fill rule"), semibold(11.5), t.text);
    let close = Rect::from_center_size(pos2(head.max.x - 12.0, head.center().y), vec2(24.0, 24.0));
    let closed = widgets::icon_button_at(ui, Id::new("fill_rule_close"), close, Icon::Close, tr("Close"));
    ui.add_space(8.0);
    // 2. The preview: the areas hatched by their rule, the loops with the way they run.
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), PREVIEW_H), Sense::click());
    p.preview = rect;
    ui.painter().rect_filled(rect, CornerRadius::same(6), t.field);
    ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    if net.regions.is_empty() {
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, tr("This vector has no filled areas"), regular(11.0), t.text_secondary);
    } else if let Some(fit) = Fit::new(net, rect) {
        let painter = ui.painter().with_clip_rect(rect);
        p.hover = resp.hover_pos().and_then(|at| hit(net, &fit, at));
        if resp.clicked()
            && let Some(h) = resp.interact_pointer_pos().and_then(|at| hit(net, &fit, at))
        {
            *act = Some(match h {
                Hit::Area(region) => {
                    let now = net.regions.get(region).map(|r| r.winding).unwrap_or_default();
                    let rule = if now == WindingRule::NonZero { WindingRule::EvenOdd } else { WindingRule::NonZero };
                    Act::Rule { region, rule }
                }
                Hit::Loop { region, loop_ } => Act::Reverse { region, loop_ },
            });
        }
        if p.hover.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        // The areas: hatch lines clipped by the area's own fill rule, so holes stay clear. The
        // hatch is cached by document revision so it is not re-flattened and re-clipped each frame.
        let hatches = cached_hatch(p, net, rect, rev);
        for (i, region) in net.regions.iter().enumerate() {
            let col = hatch(region.winding, p.hover == Some(Hit::Area(i)));
            for &[a, b] in &hatches[i] {
                painter.line_segment([a, b], Stroke::new(HATCH_W, col));
            }
        }
        // The loops over them, each segment with an arrow showing the way it is walked.
        for (region, r) in net.regions.iter().enumerate() {
            for (loop_, lp) in r.loops.iter().enumerate() {
                let col = if p.hover == Some(Hit::Loop { region, loop_ }) { t.text } else { t.text_secondary };
                for (seg, back) in net.loop_walk(lp) {
                    let Some(c) = net.cubic(seg) else { continue };
                    let pts = if back { [c.p3, c.p2, c.p1, c.p0] } else { [c.p0, c.p1, c.p2, c.p3] };
                    let s = pts.map(|q| fit.to_screen(q));
                    painter.add(egui::epaint::CubicBezierShape::from_points_stroke(s, false, Color32::TRANSPARENT, Stroke::new(1.0, col)));
                    painter.circle_filled(s[0], 1.7, col);
                    arrow(&painter, c, back, &fit, col);
                }
            }
        }
    }
    // 3. The two rules side by side, and what the clicks do.
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        legend(ui, t, WindingRule::NonZero, tr("Non-zero"));
        legend(ui, t, WindingRule::EvenOdd, tr("Even-odd"));
    });
    ui.add_space(4.0);
    widgets::note(ui, tr("Click an area to switch its rule; click a loop to turn it round"), t.text_secondary);
    closed
}

/// The panel, drawn once a frame while it is open.
pub fn show(app: &mut FigApp, ctx: &egui::Context) {
    let Some(mut p) = app.ui.fill_rule.take() else { return };
    let Ok(doc) = app.session.doc() else { return };
    // The document revision: the cached hatch is rebuilt only when the vector network changes.
    let rev = app.session.ds().map_or(0, |d| d.revision);
    // It edits the layer it opened for: another selection, or no vector any more, closes it.
    let Some(net) = network(doc, p.id).filter(|_| app.session.selection().contains(&p.id)) else { return };
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    // Left of the Design panel (inside the window even when it is narrow).
    let x = (screen.max.x - app.ui.right_width - W - 32.0).max(screen.min.x + 8.0);
    let y = p.y.clamp(screen.min.y + 48.0, (screen.max.y - 420.0).max(screen.min.y + 48.0));
    let mut act = None;
    let area = egui::Area::new(Id::new("fill_rule_area")).order(egui::Order::Foreground).fixed_pos(pos2(x, y)).show(ctx, |ui| {
        egui::Frame::NONE
            .fill(t.panel)
            .stroke(Stroke::new(1.0, t.border))
            .corner_radius(CornerRadius::same(10))
            .shadow(egui::epaint::Shadow { offset: [0, 2], blur: 16, spread: 0, color: t.shadow })
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| contents(ui, &t, &mut p, net, &mut act, rev))
            .inner
    });
    let rect = area.response.rect;
    let outside = ctx.input(|i| {
        i.pointer.any_pressed()
            && i.pointer.interact_pos().is_some_and(|at| {
                // A press on the Fill section's fill-rule button is not "outside": it toggles the
                // panel, so counting it as outside would close the panel and then the click would
                // reopen it (see app.ui.fill_rule_button, set in right.rs).
                !rect.contains(at) && !app.ui.fill_rule_button.is_some_and(|b| b.contains(at))
            })
    });
    let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    match act {
        Some(Act::Rule { region, rule }) => {
            let rule = if rule == WindingRule::NonZero { "nonZero" } else { "evenOdd" };
            app.run("vector.fillRule", json!({"id": p.id, "region": region, "rule": rule}));
        }
        Some(Act::Reverse { region, loop_ }) => {
            app.run("vector.reverseLoop", json!({"id": p.id, "region": region, "loop": loop_}));
        }
        None => {}
    }
    if !(area.inner || outside || escape) {
        app.ui.fill_rule = Some(p);
    }
}
