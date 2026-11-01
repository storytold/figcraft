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
