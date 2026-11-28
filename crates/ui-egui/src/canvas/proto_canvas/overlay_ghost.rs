//! The ghost of a manually placed overlay: while the interaction editor has an Open overlay action
//! with Manual position open, the overlay frame shows on the canvas where it will open (its offset
//! from the top-left of the layer that opens it), and dragging the ghost moves it. The drag is one
//! undo step, and the action's X and Y fields follow it live.

use std::sync::Arc;

use egui::{Color32, CornerRadius, Id, Pos2, Rect, Stroke, StrokeKind, pos2};
use figcraft_doc::proto::{Action, MAX_ACTION_DEPTH, OverlayPosition};
use figcraft_doc::{Document, NodeId};
use serde_json::json;

use crate::FigApp;
use crate::proto_editor::{Slot, run_at};

/// The overlay thumbnail's longest side (pixels).
const GHOST_PX: f32 = 1024.0;
/// The undo step a drag makes (the label of the command it runs).
const LABEL: &str = "Edit prototype action";

/// The Manual overlay action being edited.
struct Manual {
    slot: Slot,
    action: Action,
    dest: NodeId,
    /// The trigger layer's top-left (page units).
    origin: [f64; 2],
    /// The overlay's top-left from `origin`; without one, right below the trigger layer.
    offset: [f64; 2],
}

/// A ghost being dragged: the action at the press, the page point grabbed and the last offset set.
#[derive(Clone)]
struct Drag {
    slot: Slot,
    action: Action,
    grab: [f64; 2],
    start: [f64; 2],
    last: [f64; 2],
}

fn drag_id() -> Id {
    Id::unique("proto_overlay_ghost_drag")
}

/// The first Open overlay action with Manual position in `actions` (conditional blocks too); its
/// path is left in `path`.
fn find_manual(actions: &[Action], path: &mut Vec<usize>, depth: usize) -> Option<Action> {
    for (i, a) in actions.iter().enumerate() {
        path.push(i);
        if matches!(a, Action::OpenOverlay { position: OverlayPosition::Manual, .. }) {
            return Some(a.clone());
        }
        if let Action::Conditional { blocks } = a
            && depth < MAX_ACTION_DEPTH
            && let Some(found) = blocks.iter().enumerate().find_map(|(b, block)| {
                let mut inner = vec![b];
                let found = find_manual(&block.actions, &mut inner, depth + 1)?;
                path.extend(inner);
                Some(found)
            })
        {
            return Some(found);
        }
        path.pop();
    }
    None
}

/// The Manual overlay of interaction `k` of `node`, if it has one.
fn manual(doc: &Document, node: NodeId, k: usize) -> Option<Manual> {
    let it = doc.node(node)?.interactions.get(k)?;
    let mut path = Vec::new();
    let action = find_manual(&it.actions, &mut path, 0)?;
    let Action::OpenOverlay { dest, offset, .. } = &action else { return None };
    let trigger = doc.world_bounds(node)?;
    let (dest, offset) = (*dest, offset.unwrap_or([0.0, trigger.height()]));
    Some(Manual { slot: Slot { node, k, path }, action, dest, origin: [trigger.x0, trigger.y0], offset })
}

/// The Manual overlay the interaction editor has open on the shown page.
fn editing(app: &FigApp, ctx: &egui::Context, doc: &Document, page: NodeId) -> Option<Manual> {
    let (node, k) = crate::proto_panel::editor(ctx)?;
    if !super::active(app) || doc.page_of(node) != Some(page) {
        return None;
    }
    manual(doc, node, k)
}

/// Screen rect of page rect (`x`, `y`, `w`, `h`).
fn screen(app: &FigApp, [x, y]: [f64; 2], w: f64, h: f64) -> Rect {
    Rect::from_min_max(app.to_screen(x, y), app.to_screen(x + w, y + h))
}

/// Where the overlay frame opens, on screen.
fn ghost_rect(app: &FigApp, doc: &Document, m: &Manual) -> Option<Rect> {
    let b = doc.world_bounds(m.dest)?;
    Some(screen(app, [m.origin[0] + m.offset[0], m.origin[1] + m.offset[1]], b.width(), b.height()))
}

/// The overlay frame's thumbnail and the page area it shows (effects reach past the frame), kept
/// while the frame's layers stay the same: dragging edits only the trigger layer.
#[derive(Clone)]
struct Thumb {
    doc: Arc<Document>,
    dest: NodeId,
    tex: Option<egui::TextureHandle>,
}

fn thumbnail(ctx: &egui::Context, doc: &Arc<Document>, dest: NodeId) -> Option<egui::TextureHandle> {
    let id = Id::unique("proto_overlay_ghost_thumb");
    let kept: Option<Thumb> = ctx.data(|d| d.get_temp(id));
    let tex = match kept {
        Some(t) if t.dest == dest && (Arc::ptr_eq(&t.doc, doc) || crate::assets_panel::same_layers(&t.doc, doc, dest)) => t.tex,
        _ => crate::assets_panel::thumbnail(ctx, doc, dest, GHOST_PX),
    };
    ctx.data_mut(|d| d.insert_temp(id, Thumb { doc: doc.clone(), dest, tex: tex.clone() }));
    tex
}

/// Draw the ghost of the Manual overlay being edited: the frame, dimmed, outlined in the accent.
pub(super) fn draw(app: &FigApp, painter: &egui::Painter, doc: &Arc<Document>, page: NodeId) {
    let ctx = painter.ctx();
    let Some(m) = editing(app, ctx, doc, page) else { return };
    let Some(r) = ghost_rect(app, doc, &m) else { return };
    let accent = crate::theme::canvas_accent(ctx);
    let art = figcraft_render::export_bounds(doc, m.dest).zip(doc.world_bounds(m.dest));
    match (thumbnail(ctx, doc, m.dest), art) {
        (Some(tex), Some((e, b))) => {
            let at = [m.origin[0] + m.offset[0] + e.x0 - b.x0, m.origin[1] + m.offset[1] + e.y0 - b.y0];
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            painter.image(tex.id(), screen(app, at, e.width(), e.height()), uv, Color32::WHITE.gamma_multiply(0.6));
        }
        _ => {
            painter.rect_filled(r, CornerRadius::ZERO, accent.gamma_multiply(0.12));
        }
    }
    painter.rect_stroke(r, CornerRadius::ZERO, Stroke::new(1.5, accent), StrokeKind::Outside);
}

/// Move the dragged ghost so the grabbed point is under `p`.
fn drag_to(app: &mut FigApp, ctx: &egui::Context, mut d: Drag, p: Pos2) {
    let at = app.to_page(p);
    let o = [(d.start[0] + at.x - d.grab[0]).round(), (d.start[1] + at.y - d.grab[1]).round()].map(|v| v.clamp(-100_000.0, 100_000.0));
    if o != d.last {
        let mut action = d.action.clone();
        if let Action::OpenOverlay { offset, .. } = &mut action {
            *offset = Some(o);
        }
        run_at(app, &d.slot, "proto.action.set", json!({ "action": action }));
        d.last = o;
    }
    ctx.data_mut(|m| m.insert_temp(drag_id(), d));
}

/// End a drag under way, as one undo step.
pub(super) fn end(app: &mut FigApp, ctx: &egui::Context) {
    if ctx.data(|m| m.get_temp::<Drag>(drag_id())).is_some() {
        ctx.data_mut(|m| m.remove::<Drag>(drag_id()));
        let _ = app.session.commit_interaction();
    }
}

/// Canvas input for the ghost: a press on it starts a drag, which follows the pointer until the
/// button is released. Returns true while it has the gesture.
pub(super) fn input(app: &mut FigApp, ctx: &egui::Context, pos: Option<Pos2>, hovered: bool) -> bool {
    let (pressed, down) = ctx.input(|i| (i.pointer.button_pressed(egui::PointerButton::Primary), i.pointer.primary_down()));
    if let Some(d) = ctx.data(|m| m.get_temp::<Drag>(drag_id())) {
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        ctx.request_repaint();
        match pos {
            Some(p) if down => drag_to(app, ctx, d, p),
            _ => end(app, ctx),
        }
        return true;
    }
    let Some(p) = pos.filter(|_| hovered) else { return false };
    let Ok(ds) = app.session.ds() else { return false };
    let (doc, page) = (ds.doc.clone(), ds.page);
    let Some(m) = editing(app, ctx, &doc, page).filter(|m| ghost_rect(app, &doc, m).is_some_and(|r| r.contains(p))) else { return false };
    ctx.set_cursor_icon(egui::CursorIcon::Grab);
    if !pressed || app.session.begin_interaction(LABEL).is_err() {
        return false;
    }
    let at = app.to_page(p);
    let d = Drag { slot: m.slot, action: m.action, grab: [at.x, at.y], start: m.offset, last: m.offset };
    ctx.data_mut(|m| m.insert_temp(drag_id(), d));
    true
}

#[cfg(test)]
mod tests {
    use egui::{Event, PointerButton, vec2};
    use figcraft_doc::NodeId;
    use figcraft_doc::proto::{Action, OverlayAnchor, OverlayPosition};
    use figcraft_engine::Session;
    use serde_json::json;

    use super::{find_manual, ghost_rect, manual};
    use crate::RightTab;
    use crate::present_tests::{Ui, id};

    fn overlay(dest: NodeId, position: OverlayPosition, offset: Option<[f64; 2]>) -> Action {
        Action::OpenOverlay { dest, position, close_on_outside: true, background: None, offset, anchor: OverlayAnchor::Screen }
    }

    /// A button in a frame opening frame `b` as a Manual overlay 10, 20 from the button's corner,
    /// with the interaction editor open on it.
    fn scene() -> (Ui, NodeId, NodeId) {
        let mut s = Session::new();
        let mut ex = |c: &str, p: serde_json::Value| s.execute(c, &p).unwrap();
        ex("frame.create", json!({"x": 0, "y": 0, "width": 400, "height": 300}));
        let btn = id(&ex("shape.rectangle", json!({"x": 40, "y": 30, "width": 80, "height": 30})));
        let b = id(&ex("frame.create", json!({"x": 600, "y": 0, "width": 160, "height": 100})));
        ex("proto.interaction.add", json!({"ids": [btn], "dest": b}));
        ex("proto.action.set", json!({"ids": [btn], "index": 0, "path": [0], "action": overlay(b, OverlayPosition::Manual, Some([10.0, 20.0]))}));
        ex("select.set", json!({"ids": [btn]}));
        let mut u = Ui::new(s);
        u.app.ui.right_tab = RightTab::Prototype;
        u.run("view.zoomFit", json!({}));
        crate::proto_panel::open_editor(&u.ctx, Some((btn, 0)));
        u.frames(2, Vec::new);
        (u, btn, b)
    }

    fn offset(u: &Ui, btn: NodeId) -> Option<[f64; 2]> {
        let doc = u.app.session.doc().unwrap();
        match doc.node(btn).unwrap().interactions[0].actions.first() {
            Some(Action::OpenOverlay { offset, .. }) => *offset,
            _ => None,
        }
    }

    #[test]
    fn dragging_the_ghost_moves_the_overlay_by_the_dragged_amount_in_one_undo_step() {
        let (mut u, btn, _) = scene();
        let doc = u.app.session.doc().unwrap().clone();
        let r = ghost_rect(&u.app, &doc, &manual(&doc, btn, 0).unwrap()).unwrap();
        let (from, to) = (r.center(), r.center() + vec2(60.0, 45.0));
        let moved = [u.app.to_page(to).x - u.app.to_page(from).x, u.app.to_page(to).y - u.app.to_page(from).y];
        let steps = u.app.session.undo_labels(100).len();
        u.button(from, true);
        for k in 1..=3 {
            u.frame(vec![Event::PointerMoved(from + vec2(20.0, 15.0) * k as f32)]);
        }
        assert_eq!(offset(&u, btn), Some([10.0 + moved[0].round(), 20.0 + moved[1].round()]), "the X/Y follow while dragging");
        u.frame(vec![Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Default::default() }]);
        u.frames(2, Vec::new);
        assert_eq!(offset(&u, btn), Some([10.0 + moved[0].round(), 20.0 + moved[1].round()]));
        assert_eq!(u.app.session.undo_labels(100).len(), steps + 1, "one undo step");
        u.app.session.undo().unwrap();
        assert_eq!(offset(&u, btn), Some([10.0, 20.0]));
    }

    #[test]
    fn without_an_offset_the_ghost_sits_right_below_the_trigger() {
        let mut s = Session::new();
        let btn = id(&s.execute("shape.rectangle", &json!({"x": 5, "y": 7, "width": 50, "height": 20})).unwrap());
        let b = id(&s.execute("frame.create", &json!({"x": 300, "y": 0, "width": 100, "height": 80})).unwrap());
        s.execute("proto.interaction.add", &json!({"ids": [btn], "dest": b})).unwrap();
        s.execute("proto.action.set", &json!({"ids": [btn], "index": 0, "path": [0], "action": overlay(b, OverlayPosition::Manual, None)})).unwrap();
        let m = manual(&s.doc().unwrap().clone(), btn, 0).unwrap();
        assert_eq!((m.origin, m.offset, m.dest), ([5.0, 7.0], [0.0, 20.0], b));
    }

    #[test]
    fn only_a_manual_overlay_has_a_ghost_and_nested_ones_are_found() {
        let d = NodeId(9);
        let centred = overlay(d, OverlayPosition::Center, None);
        let mut path = Vec::new();
        assert!(find_manual(std::slice::from_ref(&centred), &mut path, 0).is_none());
        assert!(path.is_empty());
        let nested = Action::Conditional {
            blocks: vec![
                figcraft_doc::proto::ConditionalBlock { condition: None, actions: vec![centred.clone()] },
                figcraft_doc::proto::ConditionalBlock { condition: None, actions: vec![Action::Back, overlay(d, OverlayPosition::Manual, None)] },
            ],
        };
        let found = find_manual(&[centred, nested], &mut path, 0);
        assert!(matches!(found, Some(Action::OpenOverlay { position: OverlayPosition::Manual, .. })));
        assert_eq!(path, vec![1, 1, 1]);
    }
}
