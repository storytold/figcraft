//! The "+" handles beside a board's selected sticky or shape (#1597), drawn headless: four, only
//! for one sticky or shape with the Move tool, clear of the bar above it; clicking one makes the
//! next shape on that side joined by a connector and starts typing in it; one undo takes both away.

use egui::{Event, Id, LayerId, Modifiers, Order, PointerButton, Pos2, Rect, pos2};
use figcraft_doc::{BoardItem, NodeId};
use figcraft_engine::Session;
use figcraft_engine::tools::ToolId;
use serde_json::{Value, json};

use crate::board_connect::REACH;
use crate::layers_drag_tests::Ui;

fn id(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn board_with_shape() -> (Ui, NodeId) {
    let mut s = Session::new();
    s.execute("file.newBoard", &json!({})).unwrap();
    let a = id(&s.execute("board.shape", &json!({"shape": "roundedRectangle", "x": 0, "y": 0, "width": 160, "height": 100})).unwrap());
    let mut u = Ui::new(s);
    u.frame(vec![]);
    // The shape in the middle of the canvas at 100%.
    let view = &mut u.app.session.ds_mut().unwrap().view;
    (view.zoom, view.cx, view.cy) = (1.0, 80.0, 50.0);
    u.frame(vec![]);
    (u, a)
}

/// Where the "+" for a side shows, if it does.
fn handle(u: &Ui, side: &str) -> Option<Rect> {
    let layer = LayerId::new(Order::Middle, Id::unique(("board_connect", side)));
    u.ctx.memory(|m| m.areas().visible_last_frame(&layer)).then(|| u.ctx.memory(|m| m.area_rect(layer.id))).flatten()
}

/// Where the bar above the selection shows, if it does.
fn bar(u: &Ui) -> Option<Rect> {
    let layer = LayerId::new(Order::Middle, Id::unique("board_bar"));
    u.ctx.memory(|m| m.areas().visible_last_frame(&layer)).then(|| u.ctx.memory(|m| m.area_rect(layer.id))).flatten()
}

fn click(u: &mut Ui, p: Pos2) {
    for pressed in [true, false] {
        u.frame(vec![Event::PointerMoved(p), Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
    }
    u.frame(vec![]);
}

fn screen_box(u: &Ui, n: NodeId) -> Rect {
    let b = u.app.session.doc().unwrap().world_bounds(n).unwrap();
    Rect::from_two_pos(u.app.to_screen(b.x0, b.y0), u.app.to_screen(b.x1, b.y1))
}

#[test]
fn four_handles_show_for_one_shape_and_clear_of_the_bar() {
    let (mut u, a) = board_with_shape();
    u.app.session.execute("select.set", &json!({"ids": []})).unwrap();
    u.frame(vec![]);
    u.frame(vec![]);
    assert!(handle(&u, "right").is_none(), "nothing selected");
    u.app.session.execute("select.set", &json!({"ids": [a]})).unwrap();
    u.frame(vec![]);
    u.frame(vec![]);
    let r = screen_box(&u, a);
    for (side, want) in [
        ("above", pos2(r.center().x, r.min.y - REACH)),
        ("right", pos2(r.max.x + REACH, r.center().y)),
        ("below", pos2(r.center().x, r.max.y + REACH)),
        ("left", pos2(r.min.x - REACH, r.center().y)),
    ] {
        let h = handle(&u, side).unwrap_or_else(|| panic!("{side}: no handle"));
        assert!((h.center() - want).length() < 1.0, "{side}: {h:?}");
    }
    let bar = u.ctx.memory(|m| m.area_rect(Id::unique("board_bar"))).expect("the bar shows");
    assert!(!bar.intersects(handle(&u, "above").unwrap()), "the bar leaves the handle above free");
    // Another tool, or two layers selected: none.
    u.app.session.set_tool(ToolId::Hand);
    u.frame(vec![]);
    u.frame(vec![]);
    assert!(handle(&u, "right").is_none(), "another tool");
    u.app.session.set_tool(ToolId::Move);
    let b = id(&u.app.session.execute("board.sticky", &json!({"x": 600, "y": 0})).unwrap());
    u.app.session.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    u.frame(vec![]);
    u.frame(vec![]);
    assert!(handle(&u, "right").is_none(), "two layers");
}

#[test]
fn clicking_a_handle_adds_the_next_shape_joined_and_typing() {
    let (mut u, a) = board_with_shape();
    u.app.session.execute("select.set", &json!({"ids": [a]})).unwrap();
    u.frame(vec![]);
    u.frame(vec![]);
    let before = u.app.session.doc().unwrap().ids().count();
    let at = handle(&u, "below").expect("a handle below").center();
    click(&mut u, at);
    let doc = u.app.session.doc().unwrap();
    let made: Vec<NodeId> = doc.ids().filter(|i| doc.node(*i).is_some_and(|n| n.board.is_some())).collect();
    let next = made.iter().copied().find(|i| *i != a && matches!(doc.node(*i).unwrap().board, Some(BoardItem::Shape { .. }))).expect("a new shape");
    let line = made.iter().copied().find(|i| matches!(doc.node(*i).unwrap().board, Some(BoardItem::Connector(_)))).expect("a connector");
    assert!(screen_box(&u, next).min.y > screen_box(&u, a).max.y, "below it");
    assert!(matches!(&doc.node(line).unwrap().board, Some(BoardItem::Connector(c)) if c.from.node() == Some(a) && c.to.node() == Some(next)));
    assert!(u.app.session.ds().unwrap().text_edit.is_some(), "typing in the new shape");
    figcraft_engine::textedit::end(&mut u.app.session).unwrap();
    u.app.session.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(u.app.session.doc().unwrap().ids().count(), before, "undo takes the shape and its connector away");
}

/// While the stamp tool is over an object, or a stamp is dragged over one, that object is outlined:
/// the one the stamp will stick to (the same rule `board.stamp` attaches by). Nothing off it, and
/// nothing once the stamp is let go.
#[test]
fn the_object_a_stamp_will_stick_to_is_highlighted() {
    let (mut u, a) = board_with_shape();
    let target = |u: &Ui| u.app.session.tool.overlay.stamp_target;
    u.app.run("tool.stamp", json!({"stamp": "heart"}));
    let centre = screen_box(&u, a).center();
    let off = centre + egui::vec2(400.0, 0.0);
    u.frame(vec![Event::PointerMoved(centre)]);
    assert_eq!(target(&u), Some(a), "the stamp tool over the shape");
    u.frame(vec![Event::PointerMoved(off)]);
    assert_eq!(target(&u), None, "over the empty board");
    // Placed where the outline was: it sticks to that shape.
    click(&mut u, centre);
    let stamp = u.app.session.ds().unwrap().selection[0];
    let on = |u: &Ui| match &u.app.session.doc().unwrap().node(stamp).unwrap().board {
        Some(BoardItem::Stamp { on, .. }) => *on,
        other => panic!("not a stamp: {other:?}"),
    };
    assert_eq!(on(&u), Some(a));
    // Dragged with the Move tool: the shape outlined while the stamp is over it, nothing off it.
    u.app.run("tool.set", json!({"tool": "move"}));
    u.drag_to(centre, centre + egui::vec2(20.0, 10.0));
    assert_eq!(target(&u), Some(a), "a stamp dragged over the shape");
    // Let go there, it sticks to the shape and stays on the page, not in the shape's layout.
    let there = centre + egui::vec2(20.0, 10.0);
    u.button(there, false);
    let page = u.app.session.ds().unwrap().page;
    assert_eq!(u.app.session.doc().unwrap().node(stamp).unwrap().parent, Some(page), "a shape takes no layers in");
    assert_eq!((on(&u), target(&u)), (Some(a), None));
    u.drag_to(there, there + egui::vec2(10.0, 0.0));
    u.frame(vec![Event::PointerMoved(off)]);
    assert_eq!(target(&u), None, "dragged off it");
    u.button(off, false);
    assert_eq!((target(&u), on(&u)), (None, None), "let go off the shape: on nothing, and no outline");
}

/// Dragging a "+" carries the preview ("Click to select and create the next shape in that
/// direction, or drag the preview to any location on your board", Build faster with quick create;
/// #1919): let go on empty board and the next shape is made there, centred on the pointer, joined
/// by a connector, with typing started; one undo takes both away.
#[test]
fn dragging_a_handle_drops_the_next_shape_where_it_is_let_go() {
    let (mut u, a) = board_with_shape();
    u.app.session.execute("select.set", &json!({"ids": [a]})).unwrap();
    u.frame(vec![]);
    u.frame(vec![]);
    let before = u.app.session.doc().unwrap().ids().count();
    let from = handle(&u, "below").expect("a handle below").center();
    let to = from + egui::vec2(260.0, 180.0);
    u.drag_to(from, to);
    u.button(to, false);
    u.frame(vec![]);
    let doc = u.app.session.doc().unwrap();
    let next = doc.ids().find(|i| *i != a && matches!(doc.node(*i).unwrap().board, Some(BoardItem::Shape { .. }))).expect("a new shape");
    let line = doc.ids().find(|i| matches!(doc.node(*i).unwrap().board, Some(BoardItem::Connector(_)))).expect("a connector");
    let centre = screen_box(&u, next).center();
    assert!((centre - to).length() < 1.0, "centred where it was let go: {centre:?}, not {to:?}");
    assert!(matches!(&doc.node(line).unwrap().board, Some(BoardItem::Connector(c)) if c.from.node() == Some(a) && c.to.node() == Some(next)));
    assert!(u.app.session.ds().unwrap().text_edit.is_some(), "typing in the new shape");
    figcraft_engine::textedit::end(&mut u.app.session).unwrap();
    u.app.session.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(u.app.session.doc().unwrap().ids().count(), before, "undo takes the shape and its connector away");
}

/// A "+" dragged onto another sticky or shape joins the two with a connector instead of making a
/// new shape (#1919).
#[test]
fn dropping_a_handle_on_another_shape_joins_them() {
    let (mut u, a) = board_with_shape();
    let b = id(&u.app.session.execute("board.shape", &json!({"shape": "ellipse", "x": 400, "y": 300, "width": 160, "height": 100})).unwrap());
    u.app.session.execute("select.set", &json!({"ids": [a]})).unwrap();
    u.frame(vec![]);
    u.frame(vec![]);
    let shapes = |u: &Ui| {
        let doc = u.app.session.doc().unwrap();
        doc.ids().filter(|i| matches!(doc.node(*i).unwrap().board, Some(BoardItem::Shape { .. }))).count()
    };
    assert_eq!(shapes(&u), 2);
    let from = handle(&u, "right").expect("a handle right").center();
    let to = screen_box(&u, b).center();
    u.drag_to(from, to);
    u.button(to, false);
    u.frame(vec![]);
    assert_eq!(shapes(&u), 2, "no new shape");
    let doc = u.app.session.doc().unwrap();
    let line = doc.ids().find(|i| matches!(doc.node(*i).unwrap().board, Some(BoardItem::Connector(_)))).expect("a connector");
    assert!(matches!(&doc.node(line).unwrap().board, Some(BoardItem::Connector(c)) if c.from.node() == Some(a) && c.to.node() == Some(b)));
}

/// A sticky or shape inside a section or frame takes the drop too (review): the connector joins it.
#[test]
fn dropping_a_handle_on_a_shape_inside_a_frame_joins_it() {
    let (mut u, a) = board_with_shape();
    let f = id(&u.app.session.execute("frame.create", &json!({"x": 300, "y": 200, "width": 500, "height": 400})).unwrap());
    let b = id(&u
        .app
        .session
        .execute("board.shape", &json!({"shape": "ellipse", "x": 100, "y": 100, "width": 160, "height": 100, "parent": f}))
        .unwrap());
    u.app.session.execute("select.set", &json!({"ids": [a]})).unwrap();
    u.frame(vec![]);
    u.frame(vec![]);
    let from = handle(&u, "right").expect("a handle right").center();
    let to = screen_box(&u, b).center();
    u.drag_to(from, to);
    u.button(to, false);
    u.frame(vec![]);
    let doc = u.app.session.doc().unwrap();
    let line = doc.ids().find(|i| matches!(doc.node(*i).unwrap().board, Some(BoardItem::Connector(_)))).expect("a connector");
    assert!(
        matches!(&doc.node(line).unwrap().board, Some(BoardItem::Connector(c)) if c.from.node() == Some(a) && c.to.node() == Some(b)),
        "joined to the shape in the frame, not the frame"
    );
}

/// A handle let go off the canvas, over a panel, makes nothing (review).
#[test]
fn letting_a_handle_go_over_a_panel_makes_nothing() {
    let (mut u, a) = board_with_shape();
    u.app.session.execute("select.set", &json!({"ids": [a]})).unwrap();
    u.frame(vec![]);
    u.frame(vec![]);
    let before = u.app.session.doc().unwrap().ids().count();
    let from = handle(&u, "left").expect("a handle left").center();
    // The left sidebar.
    let to = Pos2::new(u.app.canvas.rect.min.x - 60.0, from.y);
    u.drag_to(from, to);
    u.button(to, false);
    u.frame(vec![]);
    assert_eq!(u.app.session.doc().unwrap().ids().count(), before, "nothing made");
}

/// A handle let go over the floating toolbar at the bottom of the canvas makes nothing either
/// (review): the toolbar lies inside the canvas rectangle.
#[test]
fn letting_a_handle_go_over_the_toolbar_makes_nothing() {
    let (mut u, a) = board_with_shape();
    u.app.session.execute("select.set", &json!({"ids": [a]})).unwrap();
    u.frame(vec![]);
    u.frame(vec![]);
    let before = u.app.session.doc().unwrap().ids().count();
    let from = handle(&u, "below").expect("a handle below").center();
    let to = crate::toolbar::pill_rect(u.app.canvas.rect, crate::toolbar::bar_width(&u.app)).center();
    u.drag_to(from, to);
    u.button(to, false);
    u.frame(vec![]);
    assert_eq!(u.app.session.doc().unwrap().ids().count(), before, "nothing made behind the toolbar");
}

/// The selection moving to another shape under a drag ends the drag (review): letting go later,
/// even with the first shape selected again, makes nothing.
#[test]
fn a_drag_ends_when_the_selection_moves_to_another_shape() {
    let (mut u, a) = board_with_shape();
    let b = id(&u.app.session.execute("board.shape", &json!({"shape": "ellipse", "x": 600, "y": 0, "width": 160, "height": 100})).unwrap());
    u.app.session.execute("select.set", &json!({"ids": [a]})).unwrap();
    u.frame(vec![]);
    u.frame(vec![]);
    let before = u.app.session.doc().unwrap().ids().count();
    let from = handle(&u, "below").expect("a handle below").center();
    let to = from + egui::vec2(0.0, 160.0);
    u.drag_to(from, to);
    u.app.session.execute("select.set", &json!({"ids": [b]})).unwrap();
    u.frame(vec![Event::PointerMoved(to)]);
    u.app.session.execute("select.set", &json!({"ids": [a]})).unwrap();
    u.frame(vec![Event::PointerMoved(to)]);
    u.button(to, false);
    u.frame(vec![]);
    assert_eq!(u.app.session.doc().unwrap().ids().count(), before, "the drag was over");
}

/// The bar above the selection stays away while a handle is dragged, as for any drag (review):
/// letting go where it was makes the next one there, not behind it, and the bar is back after.
#[test]
fn the_bar_stays_away_while_a_handle_is_dragged() {
    let (mut u, a) = board_with_shape();
    u.app.session.execute("select.set", &json!({"ids": [a]})).unwrap();
    u.frame(vec![]);
    u.frame(vec![]);
    let r = bar(&u).expect("the bar shows");
    let from = handle(&u, "above").expect("a handle above").center();
    let to = r.center();
    u.drag_to(from, to);
    assert!(bar(&u).is_none(), "no bar while a handle is dragged");
    u.button(to, false);
    u.frame(vec![]);
    let doc = u.app.session.doc().unwrap();
    let next = doc.ids().find(|i| *i != a && matches!(doc.node(*i).unwrap().board, Some(BoardItem::Shape { .. }))).expect("a new shape");
    let line = doc.ids().find(|i| matches!(doc.node(*i).unwrap().board, Some(BoardItem::Connector(_)))).expect("a connector");
    let centre = screen_box(&u, next).center();
    assert!((centre - to).length() < 1.0, "centred where the bar was: {centre:?}, not {to:?}");
    assert!(matches!(&doc.node(line).unwrap().board, Some(BoardItem::Connector(c)) if c.from.node() == Some(a) && c.to.node() == Some(next)));
    // Typing in the new one keeps the bar away, as before; done typing, it is back.
    figcraft_engine::textedit::end(&mut u.app.session).unwrap();
    u.frame(vec![]);
    u.frame(vec![]);
    assert!(bar(&u).is_some(), "the bar is back for the new selection");
}
