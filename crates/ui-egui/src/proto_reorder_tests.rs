//! Dragging actions in the interaction editor, driven headless through egui: a row dropped
//! elsewhere in its list moves the action in one undo step (top level and inside a conditional
//! block), the open action stays open where it went, and Escape cancels the drag without closing
//! the editor.

use egui::{Event, Pos2, vec2};
use figcraft_doc::NodeId;
use figcraft_doc::proto::Action;
use figcraft_engine::Session;
use serde_json::json;

use crate::list_drag::grip_id;
use crate::present_tests::{Ui, id};
use crate::proto_editor::list_key;

/// A button on frame A whose click runs: navigate to B, a conditional (if count >= 3: navigate to
/// B, add one to count, back), back. The interaction editor is open on it with the conditional open.
fn setup() -> (Ui, NodeId) {
    let mut s = Session::new();
    let a = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 400, "height": 300, "name": "A"})).unwrap());
    let b = id(&s.execute("frame.create", &json!({"x": 500, "y": 0, "width": 400, "height": 300, "name": "B"})).unwrap());
    let btn = id(&s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 120, "height": 40, "parent": a})).unwrap());
    s.execute("variable.createCollection", &json!({"name": "State"})).unwrap();
    s.execute("variable.create", &json!({"collection": "State", "name": "count", "type": "number", "value": 0})).unwrap();
    s.execute("proto.interaction.add", &json!({"id": btn, "dest": b})).unwrap();
    s.execute(
        "proto.action.add",
        &json!({"id": btn, "index": 0, "action": {"type": "conditional", "blocks": [
            {"expression": "count >= 3", "actions": [
                {"type": "navigate", "dest": b},
                {"type": "setVariable", "variable": "count", "expression": "count + 1"},
                {"type": "back"},
            ]},
        ]}}),
    )
    .unwrap();
    s.execute("proto.action.add", &json!({"id": btn, "index": 0, "action": {"type": "back"}})).unwrap();
    s.execute("select.set", &json!({"ids": [btn]})).unwrap();
    let mut u = Ui::new(s);
    u.app.ui.right_tab = crate::RightTab::Prototype;
    crate::proto_panel::open_editor(&u.ctx, Some((btn, 0)));
    u.ctx.data_mut(|d| d.insert_temp(open_id(btn), vec![1usize]));
    u.frames(3, Vec::new);
    (u, btn)
}

/// Stable id for a prototype node's open action path in the reorder tests.
fn open_id(btn: NodeId) -> egui::Id {
    egui::Id::unique(("proto_open_action", btn, 0usize))
}

/// The kinds of the actions at `prefix` (the interaction's, or a block's).
fn kinds(u: &Ui, btn: NodeId, prefix: &[usize]) -> Vec<&'static str> {
    let doc = u.app.session.doc().unwrap();
    let actions = &doc.node(btn).unwrap().interactions[0].actions;
    let list = match prefix {
        [] => actions.as_slice(),
        [a, b] => match &actions[*a] {
            Action::Conditional { blocks } => blocks[*b].actions.as_slice(),
            other => panic!("{other:?}"),
        },
        _ => panic!(),
    };
    list.iter().map(Action::label).collect()
}

/// The middle of the grip of action `index` of the list at `prefix` (as laid out last frame).
fn grip(u: &Ui, btn: NodeId, prefix: &[usize], index: usize) -> Pos2 {
    u.ctx.read_response(grip_id(list_key(btn, 0, prefix), index)).expect("the row has a grip").rect.center()
}

/// Press at `from`, move to `to` in small steps, and leave the button down.
fn drag(u: &mut Ui, from: Pos2, to: Pos2) {
    u.button(from, true);
    for k in 1..=10 {
        u.frame(vec![Event::PointerMoved(from + (to - from) * (k as f32 / 10.0))]);
    }
}

fn undo_len(u: &Ui) -> usize {
    u.app.session.ds().unwrap().history.undo.len()
}

#[test]
fn dragging_an_action_row_reorders_the_actions_in_one_step() {
    let (mut u, btn) = setup();
    assert_eq!(kinds(&u, btn, &[]), ["Navigate to", "Conditional", "Back"]);
    let (first, last) = (grip(&u, btn, &[], 0), grip(&u, btn, &[], 2));
    assert!(first.y < last.y, "listed in order");
    let undo = undo_len(&u);
    // Back above the first action.
    let to = first - vec2(0.0, 12.0);
    drag(&mut u, last, to);
    assert_eq!(kinds(&u, btn, &[]), ["Navigate to", "Conditional", "Back"], "nothing moves before the drop");
    u.button(to, false);
    assert_eq!(kinds(&u, btn, &[]), ["Back", "Navigate to", "Conditional"]);
    assert_eq!(undo_len(&u), undo + 1);
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
    // The conditional stays open where it went.
    assert_eq!(u.ctx.data(|d| d.get_temp::<Vec<usize>>(open_id(btn))), Some(vec![2]));
    // Dropped where it is: nothing changes.
    u.frames(2, Vec::new);
    let g = grip(&u, btn, &[], 1);
    drag(&mut u, g, g + vec2(0.0, 6.0));
    u.button(g + vec2(0.0, 6.0), false);
    assert_eq!(undo_len(&u), undo + 1);
    // Undo puts the order back.
    assert!(u.app.session.undo().unwrap());
    assert_eq!(kinds(&u, btn, &[]), ["Navigate to", "Conditional", "Back"]);
}

#[test]
fn actions_inside_a_conditional_block_drag_within_their_block() {
    let (mut u, btn) = setup();
    let block = [1usize, 0];
    assert_eq!(kinds(&u, btn, &block), ["Navigate to", "Set variable", "Back"]);
    // Open the set variable action inside the block.
    u.ctx.data_mut(|d| d.insert_temp(open_id(btn), vec![1usize, 0, 1]));
    u.frames(3, Vec::new);
    let (first, last) = (grip(&u, btn, &block, 0), grip(&u, btn, &block, 2));
    let undo = undo_len(&u);
    // The first action below the last one.
    let to = last + vec2(0.0, 12.0);
    drag(&mut u, first, to);
    u.button(to, false);
    assert_eq!(kinds(&u, btn, &block), ["Set variable", "Back", "Navigate to"]);
    assert_eq!(kinds(&u, btn, &[]), ["Navigate to", "Conditional", "Back"], "the outer list stays");
    assert_eq!(undo_len(&u), undo + 1);
    assert_eq!(u.ctx.data(|d| d.get_temp::<Vec<usize>>(open_id(btn))), Some(vec![1, 0, 0]), "the open action follows");
}

#[test]
fn escape_cancels_an_action_drag_and_keeps_the_editor_open() {
    let (mut u, btn) = setup();
    let (first, last) = (grip(&u, btn, &[], 0), grip(&u, btn, &[], 2));
    drag(&mut u, last, first);
    u.key(first, egui::Key::Escape);
    u.button(first, false);
    assert_eq!(kinds(&u, btn, &[]), ["Navigate to", "Conditional", "Back"]);
    assert_eq!(crate::proto_panel::editor(&u.ctx), Some((btn, 0)), "the editor stays open");
}
