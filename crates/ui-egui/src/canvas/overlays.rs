use std::sync::Arc;

use super::builder::{builder_overlay, hatched, region_hatch};
use super::outline::{TextOnScreen, draw_outline, node_quad, outline_colour, outline_shapes, quad, slot_badges, stroke, swap_highlight};
use super::rulers::{boards, fmt, rulers};
use super::{
    Align2, COMPONENT, Color32, CornerRadius, Document, FigApp, NodeId, NodeKind, Point, Pos2, Rect, Shape, Stroke, StrokeKind, Tokens, ToolId,
    doc_c32, pos2, selection_box, selection_frame, semibold, tr, vec2,
};
use figcraft_doc::{HConstraint, VConstraint};

/// Everything drawn over the rendered page; returns the frame labels drawn (frame, corners on the
/// page) for the engine to hit-test.
pub(super) fn overlays(app: &FigApp, painter: &egui::Painter, t: &Tokens) -> Vec<(NodeId, [Point; 4])> {
    let mut labels = Vec::new();
    let Ok(ds) = app.session.ds() else { return labels };
    let doc = ds.doc.clone();
    let page = ds.page;
    let sel = ds.selection.clone();
    let zoom = ds.view.zoom;
    let view = ds.view.clone();
    // Pixel grid at high zoom.
    if view.pixel_grid_shown() {
        pixel_grid(app, painter);
    }
    // Layout grids and frame labels for top-level frames (also those in sections); the focus view
    // labels only its frame.
    let focus = ds.focused();
    for c in boards(&doc, page).iter().filter(|c| focus.is_none_or(|f| f == **c)) {
        let Some(n) = doc.node(*c) else { continue };
        let Some(b) = doc.world_bounds(*c) else { continue };
        // Off-screen frames draw neither grids nor a label (labels sit just outside the frame).
        let on_screen = Rect::from_two_pos(app.to_screen(b.x0, b.y0), app.to_screen(b.x1, b.y1)).expand(20.0);
        if !on_screen.intersects(app.canvas.rect) {
            continue;
        }
        if view.layout_grids && !n.grids.is_empty() && figcraft_layout::grids_shown(&doc, *c) {
            layout_grids(app, painter, &doc, *c, n);
        }
        // The focus view's bar names its frame.
        // Board objects (stickies, shapes with text, stamps) are frames without a name label.
        let labelled = focus.is_none() && (n.kind.is_frame_like() || matches!(n.kind, NodeKind::Section { .. })) && !n.is_board_atom();
        if labelled && let Some(q) = crate::frame_labels::draw(app, painter, t, &doc, *c, sel.contains(c), ds.hover == Some(*c)) {
            labels.push((*c, q.map(|p| app.to_page(p))));
        }
    }
    // Prototype connections.
    crate::proto_canvas::draw(app, painter);
    // Hover outline (measuring outlines the measured box in red instead).
    if let Some(h) = ds.hover
        && !sel.contains(&h)
        && matches!(app.session.tool.tool, ToolId::Move | ToolId::Scale)
        && app.session.prefs.highlight_on_hover
        && app.session.tool.overlay.measure_box.is_none()
    {
        draw_outline(app, painter, &doc, h, stroke(outline_colour(&doc, h, accent(painter)), 1.5));
    }
    // Inside a boolean group (an operand selected or in vector edit mode): every operand's outline,
    // so the shapes making the live result stay visible while one of them changes.
    let editing = ds.vector_edit.as_ref().map(|ve| ve.id);
    let unmoved = crate::selection_cache::unmoved(ds, &doc);
    let key: Vec<u64> = editing.map(|e| e.0).into_iter().collect();
    let operands = crate::selection_cache::cached(painter.ctx(), "operands", &unmoved, &sel, &key, || {
        figcraft_engine::booleans::outlined_operands(&unmoved, &sel, editing)
    });
    for id in operands.iter().filter(|o| !sel.contains(o)) {
        draw_outline(app, painter, &doc, *id, stroke(outline_colour(&doc, *id, accent(painter)).gamma_multiply(0.6), 1.0));
    }
    // Mask outlines (a mask doesn't draw itself), frame outlines and slices, as the View menu shows.
    if view.mask_outlines || view.frame_outlines || view.show_slices {
        view_outlines(app, painter, t, ds, &doc);
    }
    // Drop target / insertion indicator.
    let ov = &app.session.tool.overlay;
    if let Some(d) = ov.drop_target
        && let Some(q) = node_quad(app, &doc, d)
    {
        quad(painter, q, stroke(if figcraft_engine::components::is_slot(&doc, d) { COMPONENT } else { accent(painter) }, 1.0));
    }
    // Boards: the object a stamp would stick to, outlined as a hovered layer is.
    if let Some(s) = ov.stamp_target {
        draw_outline(app, painter, &doc, s, stroke(outline_colour(&doc, s, accent(painter)), 1.5));
    }
    slot_badges(app, painter, &doc, &sel, ov.drop_target);
    attach_points(app, painter, &ov.attach);
    if let Some(t) = ov.swap_target {
        swap_highlight(app, painter, &doc, t);
    }
    if let Some([x0, y0, x1, y1]) = ov.insert_line {
        painter.line_segment([app.to_screen(x0, y0), app.to_screen(x1, y1)], stroke(accent(painter), 2.0));
    }
    if let Some([x0, y0, x1, y1]) = ov.drop_cell {
        let r = Rect::from_two_pos(app.to_screen(x0, y0), app.to_screen(x1, y1));
        painter.rect_filled(r, CornerRadius::same(2), accent(painter).gamma_multiply(0.12));
        let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
        painter.add(Shape::dashed_line(&pts, stroke(accent(painter), 1.5), 4.0, 3.0));
    }
    // Vector edit mode, with the transform box around several selected points.
    // The selection's size label, which measurement labels keep clear of.
    let mut size_label = None;
    if let Some(ve) = ds.vector_edit.clone()
        && let Some(o) = figcraft_engine::vectoredit::overlay(&doc, &ve)
    {
        // The Variable width tool shows width points in place of the path's points.
        let width_tool = ve.tool == figcraft_engine::vectoredit::VTool::Width;
        // Other layers edited at once go under the active one; the hover shows on its own layer.
        let ov = &app.session.tool.overlay;
        let hover_on = |id| ov.vector_hover.filter(|_| ov.vector_hover_on.unwrap_or(ve.id) == id);
        for part in &ve.others {
            if let Some(po) = figcraft_engine::vectoredit::overlay(&doc, &ve.view(part)) {
                vector_overlay(app, painter, &po, !width_tool, hover_on(part.id), false, None);
            }
        }
        vector_overlay(app, painter, &o, !width_tool, hover_on(ve.id), true, ov.vector_add);
        if width_tool {
            width_overlay(app, painter, &doc, &ve, zoom);
        } else if let Some(c) = app.session.vector_edit_box() {
            // The box around several selected points draws no corner handles: they would read as
            // points. Its corners and edges still scale the points when dragged.
            let label = format!("{} × {}", fmt((c[1] - c[0]).hypot()), fmt((c[3] - c[0]).hypot()));
            size_label = transform_box(app, painter, c.map(|p| app.to_screen(p.x, p.y)), accent(painter), true, 0, Some(label));
        }
    }
    // Selection.
    let editing = ds.text_edit.clone();
    if !sel.is_empty() && editing.is_none() && ds.vector_edit.is_none() {
        size_label = selection_overlay(app, painter, ds, &doc, &sel, zoom);
    }
    if app.session.tool.tool == ToolId::ShapeBuilder
        && let Some(arr) = figcraft_engine::shapebuilder::current(&app.session)
    {
        builder_overlay(app, painter, t, &arr);
    }
    // Text editing: caret and selection, or the IME composition underlined at the caret.
    if let Some(te) = editing {
        text_edit_overlay(app, painter, t, &doc, &te, zoom);
    }
    // Smart guides and measurements (their labels kept clear of the size label).
    crate::measure_overlay::draw(app, painter, t, size_label);
    // Marquee.
    if let Some([x0, y0, x1, y1]) = ov.marquee.or(app.ui.key_box) {
        let r = Rect::from_two_pos(app.to_screen(x0, y0), app.to_screen(x1, y1));
        painter.rect_filled(r, CornerRadius::ZERO, accent(painter).gamma_multiply(0.1));
        painter.rect_stroke(r, CornerRadius::ZERO, stroke(accent(painter), 1.0), StrokeKind::Inside);
    }
    // Pen path.
    pen_overlay(app, painter, zoom);
    // Eraser brush (vector edit mode).
    if let Some([x, y, r]) = ov.brush {
        let c = app.to_screen(x, y);
        let rs = (r * app.session.ds().map(|d| d.view.zoom).unwrap_or(1.0)) as f32;
        painter.circle_stroke(c, rs, stroke(Color32::WHITE, 3.0));
        painter.circle_stroke(c, rs, stroke(accent(painter), 1.5));
    }
    // Lasso (vector edit mode).
    if ov.lasso.len() > 1 {
        let mut pts: Vec<Pos2> = ov.lasso.iter().map(|[x, y]| app.to_screen(*x, *y)).collect();
        if let Some(first) = pts.first().copied() {
            pts.push(first);
        }
        painter.add(Shape::dashed_line(&pts, stroke(accent(painter), 1.0), 4.0, 3.0));
    }
    // Guides (under the rulers).
    crate::guides::draw(app, painter);
    // Rulers.
    if view.rulers {
        rulers(app, painter, t, &doc, &sel);
    }
    labels
}

/// Pixel grid at high zoom.
fn pixel_grid(app: &FigApp, painter: &egui::Painter) {
    let r = app.canvas.rect;
    let p0 = app.to_page(r.min);
    let p1 = app.to_page(r.max);
    let col = Color32::from_black_alpha(18);
    let mut x = p0.x.floor();
    while x <= p1.x {
        let s = app.to_screen(x, 0.0).x;
        painter.line_segment([pos2(s, r.min.y), pos2(s, r.max.y)], stroke(col, 1.0));
        x += 1.0;
    }
    let mut y = p0.y.floor();
    while y <= p1.y {
        let s = app.to_screen(0.0, y).y;
        painter.line_segment([pos2(r.min.x, s), pos2(r.max.x, s)], stroke(col, 1.0));
        y += 1.0;
    }
}

/// The layout grids of frame `c` (`n`).
fn layout_grids(app: &FigApp, painter: &egui::Painter, doc: &Document, c: NodeId, n: &figcraft_doc::Node) {
    let w = doc.world_transform(c);
    for g in n.grids.iter().filter(|g| g.visible) {
        let col = doc_c32(painter.ctx(), g.color);
        for r in figcraft_layout::grid_lines(g, n.width, n.height) {
            let a = w * Point::new(r.x0, r.y0);
            let b = w * Point::new(r.x1, r.y1);
            let (sa, sb) = (app.to_screen(a.x, a.y), app.to_screen(b.x, b.y));
            if g.kind == figcraft_doc::GridKind::Grid {
                painter.line_segment([sa, sb], stroke(col, 1.0));
            } else {
                painter.rect_filled(Rect::from_two_pos(sa, sb), CornerRadius::ZERO, col);
            }
        }
    }
}

fn text_edit_overlay(app: &FigApp, painter: &egui::Painter, t: &Tokens, doc: &Document, te: &figcraft_engine::TextEdit, zoom: f64) {
    let preedit = app.preedit_view();
    let edoc: &Document = preedit.map(|p| &*p.doc).unwrap_or(doc);
    let Some(text) = TextOnScreen::of(edoc, te.id) else { return };
    let layout = &text.layout;
    if let Some(q) = node_quad(app, edoc, te.id) {
        quad(painter, q, stroke(accent(painter), 1.0));
    }
    let (a, b) = (te.caret.min(te.anchor), te.caret.max(te.anchor));
    if a != b && preedit.is_none() {
        for q in text.selection(app, a, b) {
            painter.add(Shape::convex_polygon(q.to_vec(), accent(painter).gamma_multiply(0.3), Stroke::NONE));
        }
    }
    if let Some(p) = preedit {
        for r in layout.selection_rects(p.range.0, p.range.1) {
            // Underline just below the baseline, across the composed chars only.
            let Some(line) = layout.lines.iter().find(|l| (l.top - r.y0).abs() < 1e-6) else { continue };
            let x1 = layout
                .glyphs
                .get(line.glyph_start..line.glyph_end)
                .unwrap_or(&[])
                .iter()
                .filter(|g| g.char_end > g.char_start && g.char_start >= p.range.0 && g.char_end <= p.range.1)
                .map(|g| g.x + g.advance)
                .fold(r.x0, f64::max);
            let y = line.baseline + (line.top + line.height - line.baseline) * 0.35;
            painter.line_segment([text.at(app, r.x0, y), text.at(app, x1, y)], stroke(t.text, 1.0));
        }
    }
    let caret = preedit.map(|p| p.cursor).unwrap_or(te.caret);
    let blink = (painter.ctx().input(|i| i.time) * 2.0) as i64 % 2 == 0;
    if blink || a != b {
        painter.line_segment(text.caret(app, caret, zoom), stroke(t.text, 1.0));
    }
    painter.ctx().request_repaint_after(std::time::Duration::from_millis(500));
}

/// What the selection's overlay draws that only the document, the selection, the view and the
/// overlay colour decide: kept for the frames that show the same ([`crate::selection_cache`]).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct SelectionGeometry {
    /// The outline colour: the component colour when any selected layer is in one.
    pub(super) col: Color32,
    /// Every selected layer's outline on screen.
    pub(super) outlines: Arc<Vec<Shape>>,
    /// The transform box (page space) and its width and height.
    bbox: Option<[Point; 4]>,
    size: Option<(f64, f64)>,
    /// A selected layer is locked (no handles).
    locked: bool,
}

pub(super) fn selection_geometry(app: &FigApp, painter: &egui::Painter, doc: &Arc<Document>, sel: &[NodeId]) -> SelectionGeometry {
    let green = accent(painter);
    let mapping = [app.to_screen(0.0, 0.0), app.to_screen(1.0, 1.0)].map(crate::selection_cache::bits);
    let extra = [mapping[0][0], mapping[0][1], mapping[1][0], mapping[1][1], u64::from(u32::from_le_bytes(green.to_array()))];
    crate::selection_cache::cached(painter.ctx(), "selection overlay", doc, sel, &extra, || {
        let col = if sel.iter().any(|i| outline_colour(doc, *i, green) == COMPONENT) { COMPONENT } else { green };
        let outlines = sel.iter().flat_map(|id| outline_shapes(app, doc, *id, stroke(col, 1.0))).collect();
        SelectionGeometry {
            col,
            outlines: Arc::new(outlines),
            bbox: selection_box(doc, sel),
            size: selection_frame(doc, sel).map(|(_, w, h)| (w, h)),
            locked: sel.iter().any(|i| figcraft_engine::tools::locked_in(doc, *i)),
        }
    })
}

/// [`selection_geometry`] as the canvas shows it now, and how far to shift it (on screen, on the
/// page): while a gesture has only moved the selection, the geometry where it started (kept for
/// the whole gesture) shifted by the move ([`crate::selection_cache::moved`]).
fn shown_geometry(
    app: &FigApp,
    painter: &egui::Painter,
    ds: &figcraft_engine::DocState,
    doc: &Arc<Document>,
    sel: &[NodeId],
) -> (SelectionGeometry, egui::Vec2, figcraft_geom::Vec2) {
    match crate::selection_cache::moved(ds) {
        Some((base, (dx, dy))) => {
            let on_screen = app.to_screen(dx, dy) - app.to_screen(0.0, 0.0);
            (selection_geometry(app, painter, &base, sel), on_screen, figcraft_geom::Vec2::new(dx, dy))
        }
        None => (selection_geometry(app, painter, doc, sel), egui::Vec2::ZERO, figcraft_geom::Vec2::ZERO),
    }
}

/// The selection's outlines, transform box and handles; returns the size label's box.
fn selection_overlay(
    app: &FigApp,
    painter: &egui::Painter,
    ds: &figcraft_engine::DocState,
    doc: &Arc<Document>,
    sel: &[NodeId],
    zoom: f64,
) -> Option<Rect> {
    let mut size_label = None;
    let (geo, shift, page_shift) = shown_geometry(app, painter, ds, doc, sel);
    let col = geo.col;
    if shift == egui::Vec2::ZERO {
        painter.extend(geo.outlines.iter().cloned());
    } else {
        painter.extend(geo.outlines.iter().cloned().map(|mut s| {
            s.translate(shift);
            s
        }));
    }
    // Constraint guides belong to the normal selection overlay: not while the Shape Builder
    // reshapes by regions, nor while measuring to another layer (Alt), whose lines take over.
    if app.session.tool.tool != ToolId::ShapeBuilder && app.session.tool.overlay.measure_box.is_none() {
        constraint_lines(app, painter, ds, doc, sel);
    }
    // The shape builder reshapes by regions: no transform box.
    if app.session.tool.tool != ToolId::ShapeBuilder
        && let Some(b) = geo.bbox.map(|b| b.map(|p| p + page_shift))
        && let Some((w, h)) = geo.size
    {
        let is_line = sel.len() == 1 && matches!(sel.first().and_then(|i| doc.node(*i)).map(|n| &n.kind), Some(NodeKind::Line));
        // A selected connector's handles are its two ends (drag one onto another object).
        let connector = figcraft_engine::board::selected_connector_ends(doc, sel).map(|(_, ends)| ends);
        let mut label = if is_line { fmt(w) } else { format!("{} × {}", fmt(w), fmt(h)) };
        let scale = app.session.tool.overlay.scale;
        if let Some(hint) = scale {
            label = format!("{label} · {}x", fmt(hint.factor.abs()));
        }
        // A layer smaller than its handles on screen shows just its outline (they still answer
        // just outside it).
        let tiny = figcraft_engine::tools::box_sides_px(&b, zoom).1 < figcraft_engine::tools::SMALL_BOX_PX;
        // Measuring to another layer (Alt held) shows just the outlines: no handles or size.
        let measuring = app.session.tool.overlay.measure_box.is_some();
        // Locked layers (selected in the layers panel) can't be resized, nor anything in Dev
        // Mode: no handles either.
        // Crop mode draws its own marks on the box instead (`handles::crop_marks`).
        let cropping = app.ui.crop.is_some();
        let handles = if cropping || measuring || app.session.tool.dev_mode || connector.is_some() || geo.locked {
            0
        } else if is_line {
            2
        } else if tiny {
            0
        } else {
            4
        };
        size_label = transform_box(
            app,
            painter,
            b.map(|p| app.to_screen(p.x, p.y)),
            col,
            sel.len() > 1,
            handles,
            (!measuring && connector.is_none()).then_some(label),
        );
        // Its path handles, one for each portion that can be dragged.
        let paths = if app.session.tool.dev_mode { Vec::new() } else { figcraft_engine::board::selected_connector_handles(doc, sel, zoom) };
        for (_, at) in paths {
            let c = app.to_screen(at.x, at.y);
            painter.circle_filled(c, 3.5, col);
            painter.circle_stroke(c, 3.5, stroke(Color32::WHITE, 1.0));
        }
        for end in connector.into_iter().flatten().filter(|_| !app.session.tool.dev_mode) {
            let c = app.to_screen(end.x, end.y);
            painter.circle_filled(c, 5.0, Color32::WHITE);
            painter.circle_stroke(c, 5.0, stroke(col, 1.5));
        }
        if !measuring && !cropping {
            arc_overlay(app, painter, col);
            shape_overlay(app, painter, col);
        }
        if app.session.tool.tool == ToolId::Scale {
            // The point that stays put: the dragged pivot, else the anchor.
            let pivot = scale
                .map(|h| Point::new(h.pivot[0], h.pivot[1]))
                .or_else(|| figcraft_engine::tools::scale_anchor_point(doc, sel, app.session.tool.scale_anchor));
            if let Some(p) = pivot {
                let c = app.to_screen(p.x, p.y);
                painter.circle_filled(c, 4.0, Color32::WHITE);
                painter.circle_stroke(c, 4.0, stroke(col, 1.5));
                painter.circle_filled(c, 1.5, col);
            }
        }
        // The rotation origin while it shows (Alt+R): a ring with a cross through it.
        if let Some(o) = ds.origin_handle() {
            let c = app.to_screen(o.x, o.y);
            painter.circle_filled(c, 5.0, Color32::WHITE);
            painter.circle_stroke(c, 5.0, stroke(col, 1.5));
            for d in [vec2(1.0, 0.0), vec2(0.0, 1.0)] {
                painter.line_segment([c - d * 10.0, c - d * 5.0], stroke(col, 1.5));
                painter.line_segment([c + d * 5.0, c + d * 10.0], stroke(col, 1.5));
            }
            painter.circle_filled(c, 1.5, col);
        }
    }
    size_label
}

/// The connector tool's attach points on the object under the pointer; the one its end is at is a
/// larger see-through ring, so the end's cap shows inside it.
fn attach_points(app: &FigApp, painter: &egui::Painter, points: &[([f64; 2], bool)]) {
    let col = accent(painter);
    for ([x, y], on) in points {
        let c = app.to_screen(*x, *y);
        let (r, fill) = if *on { (7.0, col.gamma_multiply(0.25)) } else { (4.0, Color32::WHITE) };
        painter.circle_filled(c, r, fill);
        painter.circle_stroke(c, r, stroke(col, 1.5));
    }
}

fn pen_overlay(app: &FigApp, painter: &egui::Painter, zoom: f64) {
    let ov = &app.session.tool.overlay;
    if !ov.pen.is_empty() {
        let pts: Vec<Pos2> = ov.pen.iter().map(|[x, y]| app.to_screen(*x, *y)).collect();
        if app.session.tool.tool == ToolId::Pencil {
            // The stroke it will make, in its colour and weight (at least a point when zoomed out).
            let (color, weight) = figcraft_engine::tools::PENCIL_STROKE;
            painter.add(Shape::line(pts, stroke(doc_c32(painter.ctx(), color), ((weight * zoom) as f32).max(1.0))));
        } else {
            for [x0, y0, x1, y1] in &ov.pen_handles {
                bezier_handle(painter, app.to_screen(*x0, *y0), app.to_screen(*x1, *y1), 3.5, Color32::WHITE);
            }
            for p in &pts {
                painter.circle_filled(*p, 3.5, Color32::WHITE);
                painter.circle_stroke(*p, 3.5, stroke(accent(painter), 1.0));
            }
            if let Some([x0, y0, x1, y1]) = ov.rubber {
                let (a, b) = (app.to_screen(x0, y0), app.to_screen(x1, y1));
                // The segment a click would make: along the last anchor's out handle, if it has one.
                match ov.rubber_handle {
                    Some([hx, hy]) => {
                        let h = app.to_screen(hx, hy);
                        painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
                            [a, h, b, b],
                            false,
                            Color32::TRANSPARENT,
                            stroke(accent(painter), 1.0),
                        ));
                    }
                    None => {
                        painter.line_segment([a, b], stroke(accent(painter), 1.0));
                    }
                }
            }
        }
    } else if let Some([x0, y0, x1, y1]) = ov.rubber {
        // A selected connector's end being dragged: from its other end to the pointer (one being
        // drawn shows itself, previewed in the document).
        painter.line_segment([app.to_screen(x0, y0), app.to_screen(x1, y1)], stroke(accent(painter), 1.5));
    }
}

/// Mask outlines, frame outlines and slices, as the View menu shows.
fn view_outlines(app: &FigApp, painter: &egui::Painter, t: &Tokens, ds: &figcraft_engine::DocState, doc: &Document) {
    let (view, page) = (&ds.view, ds.page);
    for id in doc.subtree(page) {
        let Some(n) = doc.node(id).filter(|n| n.visible) else { continue };
        if view.mask_outlines && n.is_mask {
            draw_outline(app, painter, doc, id, stroke(accent(painter), 1.0));
        } else if view.frame_outlines && n.kind.is_frame_like() {
            draw_outline(app, painter, doc, id, stroke(t.text_secondary, 1.0));
        } else if view.show_slices
            && matches!(n.kind, NodeKind::Slice)
            && let Some(q) = node_quad(app, doc, id)
        {
            painter.extend(Shape::dashed_line(&[q[0], q[1], q[2], q[3], q[0]], stroke(t.text_secondary, 1.0), 4.0, 3.0));
        }
    }
}

/// A transform box (screen corners tl, tr, br, bl): its outline when `outline`, the first `handles`
/// corner handles and, when dimensions are shown, the size `label` (if any) under it (its box returned).
fn transform_box(
    app: &FigApp,
    painter: &egui::Painter,
    q: [Pos2; 4],
    col: Color32,
    outline: bool,
    handles: usize,
    label: Option<String>,
) -> Option<Rect> {
    if outline {
        quad(painter, q, stroke(col, 1.0));
    }
    for c in q.iter().take(handles) {
        let r = Rect::from_center_size(*c, vec2(8.0, 8.0));
        painter.rect_filled(r, CornerRadius::same(1), Color32::WHITE);
        painter.rect_stroke(r, CornerRadius::same(1), stroke(col, 1.0), StrokeKind::Inside);
    }
    if let Some(label) = label.filter(|_| app.session.prefs.show_dimensions) {
        let bottom = q.iter().map(|p| p.y).fold(f32::MIN, f32::max);
        let cx = q.iter().map(|p| p.x).sum::<f32>() / 4.0;
        return Some(pill(painter, pos2(cx, bottom + 9.0), Align2::CENTER_TOP, label, col));
    }
    None
}

/// Blue dashed guides showing how a selected layer is pinned within its parent frame, and a short
/// solid mark at the layer's centre when it is centred on an axis. A left constraint draws a
/// horizontal line from the layer's left-edge midpoint to the frame's left edge; right, top and
/// bottom likewise (the line is perpendicular to the edge it leaves). A centre constraint draws a
/// short bar through the layer's middle — a horizontal one, a vertical one, or both (a cross when
/// the layer is centred on both axes). Auto-layout frames ignore constraints, so no guides are
/// drawn there, except for a layer that ignores the auto layout (absolute position).
fn constraint_lines(app: &FigApp, painter: &egui::Painter, ds: &figcraft_engine::DocState, doc: &Arc<Document>, sel: &[NodeId]) {
    // Constraints bind a layer only to a frame it sits inside: one without auto layout, or one
    // whose auto layout the layer ignores (absolute position). A move changes neither.
    let unmoved = crate::selection_cache::unmoved(ds, doc);
    let bound = crate::selection_cache::cached(painter.ctx(), "constrained", &unmoved, sel, &[], || -> Vec<NodeId> {
        let binds = |n: &figcraft_doc::Node, p: &figcraft_doc::Node| p.kind.is_frame_like() && (p.layout.is_none() || n.child_layout.absolute);
        sel.iter()
            .copied()
            .filter(|i| unmoved.node(*i).is_some_and(|n| n.parent.and_then(|p| unmoved.node(p)).is_some_and(|p| binds(n, p))))
            .collect()
    });
    let s = stroke(accent(painter), 1.0);
    let line = |a: Pos2, b: Pos2| {
        painter.add(Shape::dashed_line(&[a, b], s, 4.0, 3.0));
    };
    // A short solid bar at the layer's centre: horizontal, vertical, or both (a cross).
    let mark = |c: Pos2, half: f32, h: bool, v: bool| {
        if h {
            painter.line_segment([pos2(c.x - half, c.y), pos2(c.x + half, c.y)], s);
        }
        if v {
            painter.line_segment([pos2(c.x, c.y - half), pos2(c.x, c.y + half)], s);
        }
    };
    for &id in &bound {
        let Some(n) = doc.node(id) else { continue };
        let Some(parent_id) = n.parent else { continue };
        let Some(parent) = doc.node(parent_id) else { continue };
        let nw = doc.world_transform(id);
        let pw = doc.world_transform(parent_id);
        let (nw_w, nw_h) = (n.width, n.height);
        let (pw_w, pw_h) = (parent.width, parent.height);
        let sp = |p: Point| app.to_screen(p.x, p.y);
        // The layer's edge midpoints and centre in page space.
        let n_left = sp(nw * Point::new(0.0, nw_h / 2.0));
        let n_right = sp(nw * Point::new(nw_w, nw_h / 2.0));
        let n_top = sp(nw * Point::new(nw_w / 2.0, 0.0));
        let n_bottom = sp(nw * Point::new(nw_w / 2.0, nw_h));
        let n_c = sp(nw * Point::new(nw_w / 2.0, nw_h / 2.0));
        // The frame's edges and centre in page space (axis-aligned approximation).
        let p_left = sp(pw * Point::new(0.0, 0.0)).x;
        let p_right = sp(pw * Point::new(pw_w, 0.0)).x;
        let p_top = sp(pw * Point::new(0.0, 0.0)).y;
        let p_bottom = sp(pw * Point::new(0.0, pw_h)).y;
        match n.constraints.horizontal {
            HConstraint::Left => line(n_left, pos2(p_left, n_left.y)),
            HConstraint::Right => line(n_right, pos2(p_right, n_right.y)),
            HConstraint::LeftRight => {
                line(n_left, pos2(p_left, n_left.y));
                line(n_right, pos2(p_right, n_right.y));
            }
            HConstraint::Center | HConstraint::Scale => {}
        }
        match n.constraints.vertical {
            VConstraint::Top => line(n_top, pos2(n_top.x, p_top)),
            VConstraint::Bottom => line(n_bottom, pos2(n_bottom.x, p_bottom)),
            VConstraint::TopBottom => {
                line(n_top, pos2(n_top.x, p_top));
                line(n_bottom, pos2(n_bottom.x, p_bottom));
            }
            VConstraint::Center | VConstraint::Scale => {}
        }
        // A centred axis marks the layer itself with a short bar through its middle (a cross when
        // both axes are centred), rather than a guide reaching the frame's centre.
        let hc = matches!(n.constraints.horizontal, HConstraint::Center);
        let vc = matches!(n.constraints.vertical, VConstraint::Center);
        if hc || vc {
            mark(n_c, 8.0, hc, vc);
        }
    }
}

/// A canvas readout: white `text` on a `col` pill placed at `at` by `align` (its box returned).
fn pill(painter: &egui::Painter, at: Pos2, align: Align2, text: String, col: Color32) -> Rect {
    let galley = painter.layout_no_wrap(text, semibold(11.0), Color32::WHITE);
    let r = align.anchor_size(at, galley.size() + vec2(8.0, 4.0));
    painter.rect_filled(r, CornerRadius::same(3), crate::theme::under_white(col));
    painter.galley(r.min + vec2(4.0, 2.0), galley, Color32::WHITE);
    r
}

/// The selected ellipse's arc handles (the hovered or dragged one larger, Start with a dot inside)
/// and, while one is dragged, its readout beside it.
fn arc_overlay(app: &FigApp, painter: &egui::Painter, col: Color32) {
    use figcraft_engine::arc_handles::ArcHandle;
    let Some(hs) = app.session.arc_handles() else { return };
    let ov = &app.session.tool.overlay;
    for h in [ArcHandle::Ratio, ArcHandle::Start, ArcHandle::Sweep] {
        let Some([x, y]) = hs.at(h) else { continue };
        let c = app.to_screen(x, y);
        let r = if ov.arc_hover == Some(h) { 5.0 } else { 4.0 };
        painter.circle_filled(c, r, Color32::WHITE);
        painter.circle_stroke(c, r, stroke(col, 1.0));
        if h == ArcHandle::Start {
            painter.circle_filled(c, 1.5, col);
        }
    }
    if let Some(hint) = &ov.arc {
        let c = app.to_screen(hint.at[0], hint.at[1]);
        pill(painter, c + vec2(12.0, 12.0), Align2::LEFT_TOP, hint.label.clone(), col);
    }
}

/// The selected shape's corner radius handles and a star's ratio handle (the hovered or dragged
/// one larger) and, while one is dragged, its value beside it.
fn shape_overlay(app: &FigApp, painter: &egui::Painter, col: Color32) {
    use figcraft_engine::shape_handles::ShapeHandle;
    let ov = &app.session.tool.overlay;
    for (h, [x, y]) in app.session.shape_handles() {
        let r = if ov.shape_hover == Some(h) { 4.5 } else { 3.5 };
        let c = app.to_screen(x, y);
        painter.circle_filled(c, r, Color32::WHITE);
        painter.circle_stroke(c, r, stroke(col, 1.0));
    }
    if let Some(hint) = &ov.shape {
        let c = app.to_screen(hint.at[0], hint.at[1]);
        let text = match hint.handle {
            ShapeHandle::Radius { .. } => format!("{} {}", tr("Radius"), hint.value),
            ShapeHandle::Ratio => format!("{} {}%", tr("Ratio"), hint.value),
        };
        pill(painter, c + vec2(12.0, 12.0), Align2::LEFT_TOP, text, col);
    }
}

/// Variable width tool: each width point on the path with a bar across the stroke to its two
/// width handles (the selected point filled), and a faint preview where a drag would add one.
fn width_overlay(app: &FigApp, painter: &egui::Painter, doc: &Document, ve: &figcraft_engine::vectoredit::VectorEdit, zoom: f64) {
    use figcraft_doc::WidthSide;
    use figcraft_engine::vectoredit::{WHit, width_handles, width_preview};
    let hover = app.session.tool.overlay.width_hover;
    let sp = |p: &[f64; 2]| app.to_screen(p[0], p[1]);
    let draw = |h: &figcraft_engine::vectoredit::WidthHandle, ghost: bool| {
        let (c, a, b) = (sp(&h.at), sp(&h.sides[0]), sp(&h.sides[1]));
        let col = if ghost { accent(painter).gamma_multiply(0.5) } else { accent(painter) };
        painter.line_segment([a, b], stroke(Color32::from_white_alpha(160), 3.0));
        painter.line_segment([a, b], stroke(col, 1.0));
        for (e, side) in [(a, WidthSide::Right), (b, WidthSide::Left)] {
            let grab = !ghost && hover == Some(WHit::Handle { index: h.index, side });
            let r = if grab { 4.5 } else { 3.5 };
            painter.circle_filled(e, r, Color32::WHITE);
            painter.circle_stroke(e, r, stroke(col, 1.0));
        }
        let on = !ghost && ve.width_point == Some(h.index);
        let hovered = !ghost && hover == Some(WHit::Point { index: h.index });
        let r = if hovered { 5.0 } else { 4.0 };
        painter.circle_filled(c, r, if on { accent(painter) } else { Color32::WHITE });
        painter.circle_stroke(c, r, stroke(col, 1.0));
    };
    for h in width_handles(doc, ve, zoom) {
        draw(&h, false);
    }
    if let Some(WHit::Path { t, .. }) = hover {
        for h in width_preview(doc, ve, t, zoom) {
            draw(&h, true);
        }
    }
}

/// A bezier handle from its anchor `v` to its end `e` (screen points): the line, and a dot of
/// radius `r` filled with `fill` (vector edit mode and the Pen draw them alike).
fn bezier_handle(painter: &egui::Painter, v: Pos2, e: Pos2, r: f32, fill: Color32) {
    painter.line_segment([v, e], stroke(accent(painter), 1.0));
    painter.circle_filled(e, r, fill);
    painter.circle_stroke(e, r, stroke(accent(painter), 1.0));
}

/// Vector edit mode: a layer's segments, handles and points (`points`) with its `hover`ed part, the
/// `add` point a press would put on the outline, and for the `active` layer the closed area under
/// the paint bucket and the pen's rubber band.
fn vector_overlay(
    app: &FigApp,
    painter: &egui::Painter,
    o: &figcraft_engine::vectoredit::EditOverlay,
    points: bool,
    hover: Option<figcraft_engine::vectoredit::VHit>,
    active: bool,
    add: Option<figcraft_engine::tools::VectorAdd>,
) {
    use figcraft_engine::vectoredit::VHit;
    let ov = &app.session.tool.overlay;
    let sp = |p: &[f64; 2]| app.to_screen(p[0], p[1]);
    // Paint bucket target: hatched like a fill to come.
    if active && !ov.face.is_empty() {
        let polys: Vec<Vec<Pos2>> = ov.face.iter().map(|poly| poly.iter().map(sp).collect()).collect();
        hatched(painter, &polys, accent(painter));
    }
    if active {
        region_hatch(app, painter);
    }
    let hover_seg = match hover {
        Some(VHit::Segment { seg, .. }) => Some(seg),
        _ => None,
    };
    for (i, seg) in o.segments.iter().enumerate() {
        let [a, b, c, d] = [sp(&seg[0]), sp(&seg[1]), sp(&seg[2]), sp(&seg[3])];
        let w = if o.selected_segments.contains(&i) {
            3.0
        } else if hover_seg == Some(i) {
            2.5
        } else {
            1.5
        };
        painter.add(egui::epaint::CubicBezierShape::from_points_stroke([a, b, c, d], false, Color32::TRANSPARENT, stroke(accent(painter), w)));
    }
    if !points {
        return;
    }
    for h in &o.handles {
        let hovered = hover == Some(VHit::Handle { seg: h.seg, start: h.start });
        let on = o.selected_handles.contains(&(h.seg, h.start));
        bezier_handle(painter, sp(&h.vertex), sp(&h.handle), if hovered { 4.5 } else { 3.5 }, if on { accent(painter) } else { Color32::WHITE });
    }
    for (i, v) in o.vertices.iter().enumerate() {
        let p = sp(v);
        let hovered = matches!(hover, Some(VHit::Vertex { index }) if index == i);
        let size = if hovered { 9.0 } else { 7.0 };
        let r = Rect::from_center_size(p, vec2(size, size));
        let on = o.selected_vertices.contains(&i);
        painter.rect_filled(r, CornerRadius::same(5), if on { accent(painter) } else { Color32::WHITE });
        painter.rect_stroke(r, CornerRadius::same(5), stroke(accent(painter), 1.0), StrokeKind::Inside);
        if o.anchor == Some(i) {
            painter.circle_stroke(p, 7.5, stroke(accent(painter), 1.0));
        }
    }
    // The point a press would add to the outline: the pen's ghost anchor where it is over a
    // segment, or a segment's midpoint under the Move tool (which grows and fills once the pointer
    // is on it, when a press adds it).
    if let Some(a) = add {
        let p = app.to_screen(a.at[0], a.at[1]);
        let col = accent(painter);
        if a.midpoint {
            let (r, fill) = if a.on { (5.5, col) } else { (3.5, Color32::from_white_alpha(190)) };
            painter.circle_filled(p, r, fill);
            painter.circle_stroke(p, r, stroke(if a.on { Color32::WHITE } else { col.gamma_multiply(0.7) }, 1.2));
        } else {
            let r = Rect::from_center_size(p, vec2(7.0, 7.0));
            painter.rect_filled(r, CornerRadius::same(5), Color32::from_white_alpha(160));
            painter.rect_stroke(r, CornerRadius::same(5), stroke(col.gamma_multiply(0.8), 1.0), StrokeKind::Inside);
        }
    }
    if let Some([x0, y0, x1, y1]) = ov.rubber.filter(|_| active) {
        painter.add(Shape::dashed_line(&[app.to_screen(x0, y0), app.to_screen(x1, y1)], stroke(accent(painter), 1.0), 4.0, 3.0));
    }
}

/// The overlay green for the page this painter draws on ([`crate::theme::canvas_accent`]).
pub(super) fn accent(p: &egui::Painter) -> Color32 {
    crate::theme::canvas_accent(p.ctx())
}
