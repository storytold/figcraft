use super::{
    Arc, COMPONENT, Color32, CornerRadius, Document, FigApp, NodeId, NodeKind, Point, Pos2, Rect, Shape, Stroke, pos2, regular, semibold, tr, vec2,
};

pub(super) fn stroke(c: Color32, w: f32) -> Stroke {
    Stroke::new(w, c)
}

/// A text layer's layout placed on the canvas: where carets and selected characters show (yours
/// and other people's in a live session).
pub(crate) struct TextOnScreen {
    pub layout: Arc<figcraft_text::TextLayout>,
    /// How many characters the text has.
    pub chars: usize,
    /// The layout's vertical offset in the box (vertical alignment).
    dy: f64,
    world: figcraft_geom::Affine,
}

impl TextOnScreen {
    /// Text layer `id` of `doc` (`None`: not a text layer).
    pub(crate) fn of(doc: &Document, id: NodeId) -> Option<Self> {
        let n = doc.node(id)?;
        let NodeKind::Text(t) = &n.kind else { return None };
        let (layout, dy) = figcraft_text::layout_node(n)?;
        Some(Self { layout, chars: t.char_count(), dy, world: doc.world_transform(id) })
    }

    /// Layout point `(x, y)` on screen (on the path, for text on a path).
    pub(crate) fn at(&self, app: &FigApp, x: f64, y: f64) -> Pos2 {
        let q = self.layout.warp_point(x, y);
        let p = self.world * Point::new(q.x, q.y + self.dy);
        app.to_screen(p.x, p.y)
    }

    /// The highlight of characters `a..b`, one screen quad per line.
    pub(crate) fn selection(&self, app: &FigApp, a: usize, b: usize) -> Vec<[Pos2; 4]> {
        let rects = self.layout.selection_rects(a, b);
        rects.iter().map(|r| [self.at(app, r.x0, r.y0), self.at(app, r.x1, r.y0), self.at(app, r.x1, r.y1), self.at(app, r.x0, r.y1)]).collect()
    }

    /// The caret before character `index`: its top and bottom on screen (an empty layer: a line
    /// of default height at its origin).
    pub(crate) fn caret(&self, app: &FigApp, index: usize, zoom: f64) -> [Pos2; 2] {
        let (cx, li) = self.layout.caret(index);
        match self.layout.lines.get(li).or(self.layout.lines.last()) {
            Some(line) => [self.at(app, cx, line.top), self.at(app, cx, line.top + line.height)],
            None => {
                let p = self.at(app, 0.0, 0.0);
                [p, p + vec2(0.0, 14.0 * zoom as f32)]
            }
        }
    }
}

/// A node's oriented outline in screen space.
pub(crate) fn node_quad(app: &FigApp, doc: &Document, id: NodeId) -> Option<[Pos2; 4]> {
    let n = doc.node(id)?;
    let w = doc.world_transform(id);
    let corners = [Point::new(0.0, 0.0), Point::new(n.width, 0.0), Point::new(n.width, n.height), Point::new(0.0, n.height)];
    Some(corners.map(|c| {
        let p = w * c;
        app.to_screen(p.x, p.y)
    }))
}

pub(super) fn quad(painter: &egui::Painter, q: [Pos2; 4], s: Stroke) {
    painter.add(Shape::closed_line(q.to_vec(), s));
}

/// The slots of the selected components and instances (and a slot being dropped onto): a dashed
/// outline and the slot's name above its top-left corner, like a frame label (never over the
/// content).
pub(super) fn slot_badges(app: &FigApp, painter: &egui::Painter, doc: &Arc<Document>, sel: &[NodeId], drop: Option<NodeId>) {
    let key = [drop.map_or(0, |d| d.0.wrapping_add(1))];
    // Which slots show doesn't change while the selection is only moved: the gesture's start keeps it.
    let base = app.session.ds().ok().and_then(crate::selection_cache::moved).map(|(b, _)| b);
    let from = base.as_ref().unwrap_or(doc);
    let slots = crate::selection_cache::cached(painter.ctx(), "slots", from, sel, &key, || slots_shown(from, sel, drop));
    for (id, name) in slots {
        let Some(q) = node_quad(app, doc, id) else { continue };
        if !Rect::from_points(&q).intersects(app.canvas.rect) {
            continue;
        }
        painter.extend(Shape::dashed_line(&[q[0], q[1], q[2], q[3], q[0]], stroke(COMPONENT, 1.0), 4.0, 3.0));
        // The label, when the slot is wide enough on screen to hold it.
        let galley = painter.layout_no_wrap(name, regular(10.0), COMPONENT);
        let (w, h) = (galley.size().x + 14.0, galley.size().y.max(11.0));
        if (q[1] - q[0]).length() < w {
            continue;
        }
        let at = q[0] - vec2(0.0, h + 2.0);
        crate::icons::paint(painter, Rect::from_min_size(pos2(at.x, at.y + (h - 11.0) / 2.0), vec2(11.0, 11.0)), crate::icons::Icon::Slot, COMPONENT);
        painter.galley(pos2(at.x + 14.0, at.y + (h - galley.size().y) / 2.0), galley, COMPONENT);
    }
}

/// The slots [`slot_badges`] shows, with their names: those of the selected components and
/// instances (and of the ones the selected layers are in), and the slot being dropped onto.
fn slots_shown(doc: &Document, sel: &[NodeId], drop: Option<NodeId>) -> Vec<(NodeId, String)> {
    use figcraft_engine::components::{owning_component, owning_instance, slot_name, slots_of};
    let mut roots: Vec<NodeId> = Vec::new();
    for id in sel {
        let root = match doc.node(*id).map(|n| &n.kind) {
            Some(NodeKind::Instance { .. } | NodeKind::Component { .. } | NodeKind::ComponentSet { .. }) => Some(*id),
            _ => owning_instance(doc, *id).or_else(|| owning_component(doc, *id)),
        };
        if let Some(r) = root.filter(|r| !roots.contains(r)) {
            roots.push(r);
        }
    }
    let mut slots: Vec<(NodeId, String)> = roots.iter().flat_map(|r| slots_of(doc, *r)).collect();
    if let Some(d) = drop
        && !slots.iter().any(|(s, _)| *s == d)
        && let Some(name) = slot_name(doc, d)
    {
        slots.push((d, name));
    }
    slots
}

/// The instance an Alt-drop of a component from the Assets panel swaps: tinted and outlined in
/// the component colour, with a "Swap instance" label above its top-left corner.
pub(super) fn swap_highlight(app: &FigApp, painter: &egui::Painter, doc: &Document, id: NodeId) {
    let Some(q) = node_quad(app, doc, id) else { return };
    // White on the component colour, in both themes (like the canvas's selection labels).
    let ink = Color32::WHITE;
    painter.add(Shape::convex_polygon(q.to_vec(), COMPONENT.gamma_multiply(0.12), Stroke::NONE));
    quad(painter, q, stroke(COMPONENT, 2.0));
    let galley = painter.layout_no_wrap(tr("Swap instance").to_string(), semibold(10.0), ink);
    let size = vec2(galley.size().x + 26.0, 18.0);
    let top = Rect::from_points(&q).min;
    let pill = Rect::from_min_size(pos2(top.x, top.y - size.y - 4.0), size);
    painter.rect_filled(pill, CornerRadius::same(4), COMPONENT);
    crate::icons::paint(
        painter,
        Rect::from_center_size(pos2(pill.min.x + 11.0, pill.center().y), vec2(10.0, 10.0)),
        crate::icons::Icon::Instance,
        ink,
    );
    painter.galley(pos2(pill.min.x + 20.0, pill.center().y - galley.size().y / 2.0), galley, ink);
}

/// The outline colour for a layer: purple for components, sets and instances and every layer in
/// them, the page's overlay `green` otherwise.
pub(super) fn outline_colour(doc: &Document, id: NodeId, green: Color32) -> Color32 {
    use figcraft_engine::components::{owning_component, owning_instance};
    let set = matches!(doc.node(id).map(|n| &n.kind), Some(NodeKind::ComponentSet { .. }));
    if set || owning_component(doc, id).is_some() || owning_instance(doc, id).is_some() { COMPONENT } else { green }
}

/// The path a layer's hover and selection outline follows, in its own space, when that isn't just
/// its box: the shape of ellipses, vectors, polygons, stars and boolean groups, and of rectangles
/// and frames with rounded corners (each corner's radius and the smoothing, as the fill draws them,
/// whatever the stroke's alignment).
pub(super) fn outline_path(doc: &Document, n: &figcraft_doc::Node) -> Option<figcraft_geom::kurbo::BezPath> {
    let shaped = match n.kind {
        NodeKind::Ellipse { .. } | NodeKind::Vector { .. } | NodeKind::Polygon { .. } | NodeKind::Star { .. } | NodeKind::BooleanOp { .. } => true,
        NodeKind::Rectangle
        | NodeKind::Frame
        | NodeKind::Component { .. }
        | NodeKind::ComponentSet { .. }
        | NodeKind::Instance { .. }
        | NodeKind::Section { .. } => n.corners.radii.iter().any(|r| *r > 0.0),
        _ => false,
    };
    if !shaped {
        return None;
    }
    figcraft_render::node_outline(doc, n).map(|(p, _)| p)
}

/// [`outline_path`] on screen, flattened into polylines.
fn outline_shape(app: &FigApp, doc: &Document, id: NodeId) -> Option<Vec<Vec<Pos2>>> {
    let n = doc.node(id)?;
    let path = outline_path(doc, n)?;
    let w = doc.world_transform(id);
    let mut out = Vec::new();
    let mut cur: Vec<Pos2> = Vec::new();
    figcraft_geom::kurbo::flatten(path.iter(), 0.5 / app.session.ds().map(|d| d.view.zoom).unwrap_or(1.0), |el| {
        use figcraft_geom::kurbo::PathEl::*;
        match el {
            MoveTo(p) => {
                if cur.len() > 1 {
                    out.push(std::mem::take(&mut cur));
                }
                cur.clear();
                let q = w * p;
                cur.push(app.to_screen(q.x, q.y));
            }
            LineTo(p) => {
                let q = w * p;
                cur.push(app.to_screen(q.x, q.y));
            }
            ClosePath => {
                if let Some(f) = cur.first().copied() {
                    cur.push(f);
                }
            }
            _ => {}
        }
    });
    if cur.len() > 1 {
        out.push(cur);
    }
    Some(out)
}

/// A layer's outline on the canvas: its shape where [`outline_path`] has one, its box otherwise.
pub(super) fn draw_outline(app: &FigApp, painter: &egui::Painter, doc: &Document, id: NodeId, s: Stroke) {
    painter.extend(outline_shapes(app, doc, id, s));
}

/// The shapes [`draw_outline`] draws.
pub(super) fn outline_shapes(app: &FigApp, doc: &Document, id: NodeId, s: Stroke) -> Vec<Shape> {
    match outline_shape(app, doc, id) {
        Some(lines) => lines.into_iter().map(|l| Shape::line(l, s)).collect(),
        None => node_quad(app, doc, id).map(|q| Shape::closed_line(q.to_vec(), s)).into_iter().collect(),
    }
}
