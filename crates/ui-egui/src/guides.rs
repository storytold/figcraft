//! Ruler guides on the canvas: drag out of a ruler to add one, drag a guide to move it (Alt: a copy),
//! drag it back onto its ruler to remove it; a click selects one (Delete, arrows and the context
//! menu act on it). A guide dropped on a top-level frame belongs to that frame: it's kept in the
//! frame's coordinates, moves with it and spans only the frame (along the frame's own axes when
//! it's rotated). Guides show while the rulers do; layers snap to them. Alt while dragging measures
//! from the guide to the layers in the frame it crosses.

use egui::{Color32, CornerRadius, Id, Pos2, Rect, Stroke, pos2, vec2};
use figcraft_doc::{Document, NodeId};
use figcraft_engine::guides;
use figcraft_geom::Point;
use serde_json::json;

use crate::FigApp;
use crate::theme::semibold;

/// Ruler thickness (matches the canvas rulers).
const RULER: f32 = 20.0;
const GUIDE: Color32 = Color32::from_rgb(0xF2, 0x48, 0x22);

#[derive(Clone, Copy, Debug)]
enum GuideDrag {
    /// Dragging a new guide out of a ruler.
    New { vertical: bool, frame: Option<NodeId> },
    /// Moving guide `index` of `owner` (a page or frame); `copy` (Alt at the press) drops a new one.
    Move { owner: NodeId, index: usize, vertical: bool, copy: bool, frame: Option<NodeId> },
}

impl GuideDrag {
    /// The frame Alt measures in: the one selected when the drag began, else the guide's own.
    fn frame(self) -> Option<NodeId> {
        match self {
            GuideDrag::New { frame, .. } | GuideDrag::Move { frame, .. } => frame,
        }
    }
}

/// A guide where it lands on the page.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Placed {
    /// The page or frame it belongs to.
    owner: NodeId,
    index: usize,
    vertical: bool,
    /// Its offset in the owner's coordinates (what its label shows; page coordinates for a page).
    offset: f64,
    /// A frame guide's page segment, along the frame's own axis across it; None: a page guide,
    /// across the whole canvas.
    line: Option<[Point; 2]>,
}

fn drag_id() -> Id {
    Id::new("guide_drag")
}

/// The dragged guide as it would land.
fn preview_id() -> Id {
    Id::new("guide_preview")
}

/// The page's guides and those of its top-level frames (frames last, so they're found first).
fn placed_guides(app: &FigApp) -> Vec<Placed> {
    let Ok(ds) = app.session.ds() else { return vec![] };
    let doc = &ds.doc;
    let mut out: Vec<Placed> = doc
        .node(ds.page)
        .map(|n| n.guides())
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(index, g)| Placed { owner: ds.page, index, vertical: g.vertical, offset: g.offset, line: None })
        .collect();
    for f in doc.top_frames(ds.page) {
        let Some(n) = doc.node(f).filter(|n| n.visible) else { continue };
        out.extend(n.guides.iter().enumerate().filter_map(|(index, g)| place(doc, f, index, g.vertical, g.offset)));
    }
    out
}

/// A placed guide as a page line (vertical, page offset), when it runs along the page's axes.
fn page_line(app: &FigApp, g: &Placed) -> Option<(bool, f64)> {
    match g.line {
        None => Some((g.vertical, g.offset)),
        Some(_) => guides::line_in(app.session.doc().ok()?.world_transform(g.owner), g.vertical, g.offset),
    }
}

/// The top-level frame of the selection, if any.
fn selected_frame(app: &FigApp) -> Option<NodeId> {
    let ds = app.session.ds().ok()?;
    ds.selection.first().and_then(|i| ds.doc.top_level_frame(*i))
}

/// Where guide `index` of frame `owner` at `offset` (frame coordinates) lands on the page.
fn place(doc: &Document, owner: NodeId, index: usize, vertical: bool, offset: f64) -> Option<Placed> {
    Some(Placed { owner, index, vertical, offset, line: Some(guides::segment(doc, owner, vertical, offset)?) })
}

/// Guide `index` of `owner` (a page or frame) through page point `p`, its offset rounded to whole
/// pixels with `snap` (else to hundredths).
fn through(app: &FigApp, owner: NodeId, index: usize, vertical: bool, p: Point, snap: bool) -> Option<Placed> {
    let ds = app.session.ds().ok()?;
    let v = guides::offset_at(&ds.doc, owner, vertical, p)?;
    let offset = if snap { v.round() } else { (v * 100.0).round() / 100.0 };
    if owner == ds.page { Some(Placed { owner, index, vertical, offset, line: None }) } else { place(&ds.doc, owner, index, vertical, offset) }
}

/// A guide dragged out of a ruler (`vertical`: the left one) and dropped at page point `p`: on the
/// top-level frame under it, in the frame's coordinates and along its axis nearer the ruler's, or
/// on the page.
fn drop_target(app: &FigApp, p: Point, vertical: bool, snap: bool) -> Option<Placed> {
    let ds = app.session.ds().ok()?;
    match guides::drop_owner(&ds.doc, ds.page, p) {
        f if f != ds.page => through(app, f, usize::MAX, guides::frame_axis(&ds.doc, f, vertical), p, snap),
        page => through(app, page, usize::MAX, vertical, p, snap),
    }
}

/// A guide command's params: frame guides name their frame, page guides need no owner.
fn owned(owner: NodeId, page: Option<NodeId>, mut params: serde_json::Value) -> serde_json::Value {
    if Some(owner) != page
        && let Some(o) = params.as_object_mut()
    {
        o.insert("frame".into(), json!(owner));
    }
    params
}

fn rulers_on(app: &FigApp) -> bool {
    app.session.ds().is_ok_and(|d| d.view.rulers)
}

/// The guide's line on screen: a page guide across the canvas `r`, a frame guide across its frame.
fn segment(app: &FigApp, g: &Placed, r: Rect) -> [Pos2; 2] {
    match g.line {
        Some([a, b]) => [app.to_screen(a.x, a.y), app.to_screen(b.x, b.y)],
        None if g.vertical => {
            let x = app.to_screen(g.offset, 0.0).x;
            [pos2(x, r.min.y), pos2(x, r.max.y)]
        }
        None => {
            let y = app.to_screen(0.0, g.offset).y;
            [pos2(r.min.x, y), pos2(r.max.x, y)]
        }
    }
}

/// The guide within a few points of `pos` (frame guides and later ones first).
fn guide_at(app: &FigApp, pos: Pos2) -> Option<Placed> {
    let r = app.canvas.rect;
    placed_guides(app).into_iter().rev().find(|g| {
        let [a, b] = segment(app, g, r);
        let ab = b - a;
        let t = if ab.length_sq() > 1e-6 { ((pos - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0) } else { 0.0 };
        (a + ab * t).distance(pos) <= 3.0
    })
}

fn cursor(vertical: bool) -> egui::CursorIcon {
    if vertical { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::ResizeVertical }
}

/// The resize cursor for dragging guide `g` across itself, whichever way it runs on screen.
fn cursor_across(app: &FigApp, g: &Placed, r: Rect) -> egui::CursorIcon {
    let [a, b] = segment(app, g, r);
    let d = b - a;
    // The line's direction in eighths of a turn, rounded (screen y points down): 0 runs across,
    // 2 runs down.
    let eighths = (d.y.atan2(d.x).to_degrees().rem_euclid(180.0) + 22.5) / 45.0;
    match eighths as u8 % 4 {
        0 => egui::CursorIcon::ResizeVertical,
        1 => egui::CursorIcon::ResizeNeSw,
        2 => egui::CursorIcon::ResizeHorizontal,
        _ => egui::CursorIcon::ResizeNwSe,
    }
}

/// Pointer handling; returns true when the event belongs to a ruler or guide.
pub fn input(app: &mut FigApp, ctx: &egui::Context, pos: Pos2) -> bool {
    if !rulers_on(app) {
        return false;
    }
    let r = app.canvas.rect;
    let (pressed, released, alt) = ctx.input(|i| {
        (i.pointer.button_pressed(egui::PointerButton::Primary), i.pointer.button_released(egui::PointerButton::Primary), i.modifiers.alt)
    });
    if let Some(drag) = ctx.data(|d| d.get_temp::<GuideDrag>(drag_id())) {
        let vertical = match drag {
            GuideDrag::New { vertical, .. } | GuideDrag::Move { vertical, .. } => vertical,
        };
        let p = app.to_page(pos);
        let snap = app.session.ds().map(|d| d.view.snap_pixel).unwrap_or(true);
        let at = match drag {
            GuideDrag::New { vertical, .. } => drop_target(app, p, vertical, snap),
            GuideDrag::Move { owner, index, vertical, .. } => through(app, owner, index, vertical, p, snap),
        };
        // Alt: distances from the guide to the layers in the frame it crosses.
        let frame = drag.frame().or_else(|| at.map(|g| g.owner).filter(|o| app.session.ds().is_ok_and(|d| d.page != *o)));
        app.session.tool.overlay.guides = match (alt && !released, frame, at.and_then(|g| page_line(app, &g))) {
            (true, Some(f), Some((v, off))) => app.session.doc().map(|d| figcraft_engine::distance::from_guide(d, f, v, off)).unwrap_or_default(),
            _ => Vec::new(),
        };
        ctx.data_mut(|d| match at {
            Some(g) => {
                d.insert_temp(preview_id(), g);
            }
            None => d.remove::<Placed>(preview_id()),
        });
        ctx.set_cursor_icon(at.map_or(cursor(vertical), |g| cursor_across(app, &g, r)));
        if released {
            ctx.data_mut(|d| {
                d.remove::<GuideDrag>(drag_id());
                d.remove::<Placed>(preview_id());
            });
            // Released over its own ruler (the left one for a guide running down the screen): no
            // guide (a moved one is removed).
            let down = at.map_or(vertical, |g| {
                let [a, b] = segment(app, &g, r);
                (b.y - a.y).abs() >= (b.x - a.x).abs()
            });
            let on_ruler = if down { pos.x < r.min.x + RULER } else { pos.y < r.min.y + RULER };
            let page = app.session.ds().map(|d| d.page).ok();
            let before = app.session.doc().ok().and_then(|d| match drag {
                GuideDrag::Move { owner, index, .. } => d.node(owner).and_then(|n| n.guides().get(index)).map(|g| g.offset),
                GuideDrag::New { .. } => None,
            });
            match (drag, on_ruler, at) {
                (GuideDrag::New { .. } | GuideDrag::Move { copy: true, .. }, false, Some(g)) if Some(g.offset) != before => {
                    app.run("guide.add", owned(g.owner, page, json!({"vertical": g.vertical, "offset": g.offset})));
                }
                (GuideDrag::Move { owner, index, copy: false, .. }, true, _) => {
                    app.run("guide.remove", owned(owner, page, json!({"index": index})));
                }
                // A click without a move leaves it where it is (selected).
                (GuideDrag::Move { owner, index, copy: false, .. }, false, Some(g)) if Some(g.offset) != before => {
                    app.run("guide.move", owned(owner, page, json!({"index": index, "offset": g.offset})));
                }
                _ => {}
            }
        }
        ctx.request_repaint();
        return true;
    }
    let in_top = pos.y < r.min.y + RULER && pos.x > r.min.x + RULER;
    let in_left = pos.x < r.min.x + RULER && pos.y > r.min.y + RULER;
    if in_top || in_left {
        ctx.set_cursor_icon(cursor(in_left));
        if pressed {
            let frame = selected_frame(app);
            ctx.data_mut(|d| d.insert_temp(drag_id(), GuideDrag::New { vertical: in_left, frame }));
        }
        return true;
    }
    if pos.x < r.min.x + RULER || pos.y < r.min.y + RULER {
        return true;
    }
    // Existing guides move with the Move tool (not while editing text or a vector).
    let idle =
        app.session.tool.tool == figcraft_engine::ToolId::Move && app.session.ds().is_ok_and(|d| d.text_edit.is_none() && d.vector_edit.is_none());
    if idle && let Some(g) = guide_at(app, pos) {
        ctx.set_cursor_icon(cursor_across(app, &g, r));
        let page = app.session.ds().map(|d| d.page).ok();
        // The context menu opens on the release, like the canvas's.
        let secondary = ctx.input(|i| i.pointer.button_clicked(egui::PointerButton::Secondary));
        if pressed || secondary {
            // The frame selected before the click is the one Alt measures in.
            let frame = selected_frame(app).or((Some(g.owner) != page).then_some(g.owner));
            app.run("guide.select", owned(g.owner, page, json!({"index": g.index})));
            if secondary {
                app.ui.context_menu = Some((pos, Vec::new()));
            } else {
                ctx.data_mut(|d| {
                    d.insert_temp(drag_id(), GuideDrag::Move { owner: g.owner, index: g.index, vertical: g.vertical, copy: alt, frame })
                });
            }
        }
        return true;
    }
    false
}

/// Ends a guide drag whose button is up by the end of the frame without [`input`] taking the
/// release (released off the canvas, or over something on it): the gesture is dropped, so a later
/// click doesn't add or move a guide.
pub fn end_released(ctx: &egui::Context) {
    if ctx.input(|i| i.pointer.primary_down()) {
        return;
    }
    ctx.data_mut(|d| {
        d.remove::<GuideDrag>(drag_id());
        d.remove::<Placed>(preview_id());
    });
}

/// Draw the page's and its frames' guides (under the rulers) and the one being dragged with its
/// position.
pub fn draw(app: &FigApp, painter: &egui::Painter) {
    if !rulers_on(app) {
        return;
    }
    let r = app.canvas.rect;
    let ctx = painter.ctx();
    let drag = ctx.data(|d| d.get_temp::<GuideDrag>(drag_id()));
    let moving = match drag {
        Some(GuideDrag::Move { owner, index, copy: false, .. }) => Some((owner, index)),
        _ => None,
    };
    let hover = if drag.is_none() { ctx.input(|i| i.pointer.hover_pos()).filter(|p| r.contains(*p)).and_then(|p| guide_at(app, p)) } else { None };
    let selected = app.session.ds().ok().and_then(|d| d.selected_guide());
    for g in placed_guides(app) {
        if Some((g.owner, g.index)) == moving {
            continue;
        }
        let hovered = hover == Some(g) || selected == Some((g.owner, g.index));
        painter.line_segment(segment(app, &g, r), Stroke::new(if hovered { 2.0 } else { 1.0 }, GUIDE));
        if hovered {
            label(app, painter, r, &g);
        }
    }
    if let Some(g) = ctx.data(|d| d.get_temp::<Placed>(preview_id())) {
        painter.line_segment(segment(app, &g, r), Stroke::new(1.0, GUIDE));
        label(app, painter, r, &g);
    }
}

/// The guide's offset (in its page's or frame's coordinates), in a tag next to its ruler or at the
/// start of a frame guide.
fn label(app: &FigApp, painter: &egui::Painter, r: Rect, g: &Placed) {
    let galley = painter.layout_no_wrap(crate::canvas::fmt(g.offset), semibold(10.0), Color32::WHITE);
    let size = galley.size() + vec2(8.0, 4.0);
    let [a, _] = segment(app, g, r);
    // A canvas narrower or shorter than the ruler leaves `min > max`; keep the tag at the ruler edge then.
    let (lo_x, lo_y) = (r.min.x + RULER, r.min.y + RULER);
    let at = pos2(a.x.clamp(lo_x, r.max.x.max(lo_x)) + 4.0, a.y.clamp(lo_y, r.max.y.max(lo_y)) + 4.0);
    let tag = Rect::from_min_size(at, size);
    painter.rect_filled(tag, CornerRadius::same(3), GUIDE);
    painter.galley(tag.min + vec2(4.0, 2.0), galley, Color32::WHITE);
}
