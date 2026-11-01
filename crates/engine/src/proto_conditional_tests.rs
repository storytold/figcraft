//! Building conditionals and editing nested actions one at a time (`proto.action.*`,
//! `proto.conditional.*`), expressions given as text, and the presenter's hotspots.

use figcraft_doc::proto::{Action, ConditionalBlock, Expr, ExprOp};
use figcraft_doc::{NodeId, VariableValue};
use serde_json::{Value, json};

use crate::Session;
use crate::cmd::proto_actions::{action_at, default_condition};
use crate::params::id_of;
use crate::proto_run::hotspots;

/// Frames A and B, a button in A with a click → navigate to B, and variables `count` (number),
/// `open` (boolean) and `label` (string).
fn setup() -> (Session, NodeId, NodeId, NodeId) {
    let mut s = Session::new();
    let id = |s: &mut Session, c: &str, p: Value| id_of(&s.execute(c, &p).unwrap()["id"]).unwrap();
    let a = id(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 100, "name": "A"}));
    let b = id(&mut s, "frame.create", json!({"x": 200, "y": 0, "width": 100, "height": 100, "name": "B"}));
    let btn = id(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 20, "parent": a}));
    s.execute("variable.createCollection", &json!({"name": "State"})).unwrap();
    for (name, kind, value) in [("count", "number", json!(0)), ("open", "boolean", json!(false)), ("label", "string", json!("Hi"))] {
        s.execute("variable.create", &json!({"collection": "State", "name": name, "type": kind, "value": value})).unwrap();
    }
    s.execute("proto.interaction.add", &json!({"id": btn, "dest": b})).unwrap();
    (s, a, b, btn)
}

fn actions(s: &Session, id: NodeId) -> Vec<Action> {
    s.doc().unwrap().node(id).unwrap().interactions[0].actions.clone()
}

fn var(s: &Session, name: &str) -> String {
    s.doc().unwrap().variables.get(name).unwrap().id.clone()
}

fn blocks(a: &Action) -> &[ConditionalBlock] {
    match a {
        Action::Conditional { blocks } => blocks,
        other => panic!("not a conditional: {other:?}"),
    }
}

fn shown(s: &Session, e: &Expr) -> String {
    e.display(&s.doc().unwrap().variables)
}

#[test]
fn build_if_else_if_else_with_nested_actions() {
    let (mut s, _, b, btn) = setup();
    let ex = |s: &mut Session, c: &str, p: Value| s.execute(c, &p).unwrap();
    // Add a conditional after the navigate: an if on `count >= 3` (text), nothing inside yet.
    let r = ex(
        &mut s,
        "proto.action.add",
        json!({"id": btn, "index": 0, "action": {"type": "conditional", "blocks": [{"expression": "count >= 3", "actions": []}]}}),
    );
    assert_eq!(r["path"], json!([1]));
    let list = actions(&s, btn);
    assert_eq!(list.len(), 2);
    assert_eq!(shown(&s, blocks(&list[1])[0].condition.as_ref().unwrap()), "count >= 3");
    assert_eq!(blocks(&list[1])[0].condition.as_ref().unwrap().variables(), [var(&s, "count").as_str()], "stored by id");
    // Else if (default condition: the first boolean equal to its value), then an else.
    assert_eq!(ex(&mut s, "proto.conditional.addBlock", json!({"id": btn, "index": 0, "path": [1]}))["block"], 1);
    assert_eq!(ex(&mut s, "proto.conditional.addBlock", json!({"id": btn, "index": 0, "path": [1], "else": true}))["block"], 2);
    // Another else if goes before the else.
    assert_eq!(ex(&mut s, "proto.conditional.addBlock", json!({"id": btn, "index": 0, "path": [1], "expression": "label == \"Hi\""}))["block"], 2);
    let list = actions(&s, btn);
    let bl = blocks(&list[1]);
    assert_eq!(bl.len(), 4);
    assert_eq!(shown(&s, bl[1].condition.as_ref().unwrap()), "open == false");
    assert_eq!(shown(&s, bl[2].condition.as_ref().unwrap()), "label == \"Hi\"");
    assert!(bl[3].condition.is_none(), "the else stays last");
    actions_in_the_branches(&mut s, b, btn);
    reorder_replace_and_remove(&mut s, b, btn);
}

fn actions_in_the_branches(s: &mut Session, b: NodeId, btn: NodeId) {
    let ex = |s: &mut Session, c: &str, p: Value| s.execute(c, &p).unwrap();
    // Actions inside the branches: a set variable with an expression in the if, a back in the else.
    let r = ex(
        s,
        "proto.action.add",
        json!({"id": btn, "index": 0, "path": [1, 0, 0], "action": {"type": "setVariable", "variable": "count", "expression": "count + 1"}}),
    );
    assert_eq!(r["path"], json!([1, 0, 0]));
    ex(s, "proto.action.add", json!({"id": btn, "index": 0, "path": [1, 3, 0], "action": {"type": "back"}}));
    ex(s, "proto.action.add", json!({"id": btn, "index": 0, "path": [1, 3, 1], "action": {"type": "navigate", "dest": b}}));
    let list = actions(s, btn);
    let Some(Action::SetVariable { variable, expr: Some(e), .. }) = action_at(&list, &[1, 0, 0]) else { panic!("{list:?}") };
    assert_eq!(variable, &var(s, "count"));
    assert_eq!(shown(s, e), "count + 1");
    assert_eq!(action_at(&list, &[1, 3, 0]), Some(&Action::Back));
}

fn reorder_replace_and_remove(s: &mut Session, b: NodeId, btn: NodeId) {
    let ex = |s: &mut Session, c: &str, p: Value| s.execute(c, &p).unwrap();
    // Reorder inside the else, replace one, change a condition, remove a block.
    assert_eq!(ex(s, "proto.action.move", json!({"id": btn, "index": 0, "path": [1, 3, 1], "to": 0}))["path"], json!([1, 3, 0]));
    assert!(matches!(action_at(&actions(s, btn), &[1, 3, 0]), Some(Action::Navigate { .. })));
    ex(s, "proto.action.set", json!({"id": btn, "index": 0, "path": [1, 3, 1], "action": {"type": "closeOverlay"}}));
    assert_eq!(action_at(&actions(s, btn), &[1, 3, 1]), Some(&Action::CloseOverlay));
    ex(s, "proto.conditional.setCondition", json!({"id": btn, "index": 0, "path": [1], "block": 1, "expression": "not open and count < 10"}));
    assert_eq!(shown(s, blocks(&actions(s, btn)[1])[1].condition.as_ref().unwrap()), "(not open) and (count < 10)");
    ex(s, "proto.conditional.removeBlock", json!({"id": btn, "index": 0, "path": [1], "block": 2}));
    assert_eq!(blocks(&actions(s, btn)[1]).len(), 3);
    ex(s, "proto.action.remove", json!({"id": btn, "index": 0, "path": [1, 2, 1]}));
    assert_eq!(blocks(&actions(s, btn)[1])[2].actions, [Action::Navigate { dest: b, preserve_scroll: false }]);
    // Every edit is one undo step.
    assert!(s.undo().unwrap());
    assert_eq!(blocks(&actions(s, btn)[1])[2].actions.len(), 2);
    // Removing the if leaves the else if as the if; removing that leaves only the else: the
    // conditional goes.
    let r = ex(s, "proto.conditional.removeBlock", json!({"id": btn, "index": 0, "path": [1], "block": 0}));
    assert_eq!(r["removedAction"], false);
    let r = ex(s, "proto.conditional.removeBlock", json!({"id": btn, "index": 0, "path": [1], "block": 0}));
    assert_eq!(r["removedAction"], true);
    assert_eq!(actions(s, btn), [Action::Navigate { dest: b, preserve_scroll: false }]);
}

#[test]
fn bad_edits_are_refused_and_change_nothing() {
    let (mut s, _, b, btn) = setup();
    s.execute(
        "proto.action.add",
        &json!({"id": btn, "index": 0, "action": {"type": "conditional", "blocks": [{"expression": "open", "actions": [{"type": "back"}]}]}}),
    )
    .unwrap();
    let before = actions(&s, btn);
    let err = |s: &mut Session, c: &str, p: Value| s.execute(c, &p).unwrap_err().to_string();
    // Conditions give true or false; set variable expressions fit the variable.
    assert!(
        err(&mut s, "proto.conditional.setCondition", json!({"id": btn, "index": 0, "path": [1], "block": 0, "expression": "count + 1"}))
            .contains("must be true or false")
    );
    assert!(
        err(&mut s, "proto.conditional.setCondition", json!({"id": btn, "index": 0, "path": [1], "block": 0, "expression": "count >"}))
            .contains("missing at the end")
    );
    assert!(
        err(&mut s, "proto.conditional.setCondition", json!({"id": btn, "index": 0, "path": [1], "block": 0, "expression": "nope == 1"}))
            .contains("No variable `nope`")
    );
    let bad_cond = Expr::op(ExprOp::Add, vec![Expr::var("count"), Expr::Value(VariableValue::Number(1.0))]);
    assert!(
        err(&mut s, "proto.conditional.setCondition", json!({"id": btn, "index": 0, "path": [1], "block": 0, "condition": bad_cond}))
            .contains("true or false")
    );
    assert!(
        err(
            &mut s,
            "proto.action.add",
            json!({"id": btn, "index": 0, "action": {"type": "setVariable", "variable": "count", "expression": "\"text\""}})
        )
        .contains("holds a number")
    );
    assert!(
        err(&mut s, "proto.action.add", json!({"id": btn, "index": 0, "action": {"type": "setVariable", "variable": "gone", "value": 1}}))
            .contains("no variable")
    );
    assert!(err(&mut s, "proto.action.add", json!({"id": btn, "index": 0, "action": {"type": "navigate", "dest": 99999}})).contains("doesn't exist"));
    // Paths must lead through conditionals to an action.
    for path in [json!([]), json!([0, 0]), json!([0, 0, 0]), json!([1, 5, 0]), json!([9]), json!([-1]), json!([1.5]), json!("1"), json!([1, 0, 9])] {
        assert!(s.execute("proto.action.set", &json!({"id": btn, "index": 0, "path": path, "action": {"type": "back"}})).is_err(), "{path}");
        assert!(s.execute("proto.action.remove", &json!({"id": btn, "index": 0, "path": path})).is_err(), "{path}");
    }
    assert!(s.execute("proto.action.add", &json!({"id": btn, "index": 0, "path": [1, 0, 2], "action": {"type": "back"}})).is_err(), "past the end");
    assert!(s.execute("proto.action.move", &json!({"id": btn, "index": 0, "path": [0], "to": 5})).is_err());
    assert!(s.execute("proto.conditional.addBlock", &json!({"id": btn, "index": 0, "path": [0]})).is_err(), "not a conditional");
    assert!(s.execute("proto.conditional.addBlock", &json!({"id": btn, "index": 0, "path": [1], "else": true, "expression": "open"})).is_err());
    assert!(s.execute("proto.conditional.addBlock", &json!({"id": btn, "index": 0, "path": [1], "else": [1]})).is_err());
    assert!(s.execute("proto.conditional.removeBlock", &json!({"id": btn, "index": 0, "path": [1], "block": 4})).is_err());
    assert!(s.execute("proto.action.set", &json!({"id": btn, "index": 3, "path": [0], "action": {"type": "back"}})).is_err(), "no interaction 3");
    assert_eq!(actions(&s, btn), before, "nothing changed");
    // An else only once.
    s.execute("proto.conditional.addBlock", &json!({"id": btn, "index": 0, "path": [1], "else": true})).unwrap();
    assert!(s.execute("proto.conditional.addBlock", &json!({"id": btn, "index": 0, "path": [1], "else": true})).is_err());
    // An interaction keeps one action.
    s.execute("proto.action.remove", &json!({"id": btn, "index": 0, "path": [1]})).unwrap();
    assert!(s.execute("proto.action.remove", &json!({"id": btn, "index": 0, "path": [0]})).unwrap_err().to_string().contains("needs an action"));
    assert_eq!(actions(&s, btn), [Action::Navigate { dest: b, preserve_scroll: false }]);
}

#[test]
fn expressions_as_text_through_the_interaction_commands() {
    let (mut s, _, _, btn) = setup();
    s.execute(
        "proto.interaction.set",
        &json!({"id": btn, "index": 0, "actions": [
            {"type": "setVariable", "variable": "label", "expression": "\"Count: \" + count"},
            {"type": "conditional", "blocks": [{"expression": "count == 0", "actions": [{"type": "back"}]}, {"actions": []}]},
        ]}),
    )
    .unwrap();
    let list = actions(&s, btn);
    let Action::SetVariable { expr: Some(e), .. } = &list[0] else { panic!() };
    assert_eq!(shown(&s, e), "\"Count: \" + count");
    assert_eq!(shown(&s, blocks(&list[1])[0].condition.as_ref().unwrap()), "count == 0");
    assert!(
        s.execute("proto.interaction.add", &json!({"id": btn, "action": {"type": "setVariable", "variable": "open", "expression": "count"}}))
            .is_err()
    );
}

#[test]
fn several_layers_edit_together() {
    let (mut s, a, b, btn) = setup();
    let other = id_of(&s.execute("shape.rectangle", &json!({"x": 60, "y": 10, "width": 20, "height": 20, "parent": a})).unwrap()["id"]).unwrap();
    s.execute("proto.interaction.add", &json!({"id": other, "dest": b})).unwrap();
    s.execute(
        "proto.action.add",
        &json!({"ids": [btn, other], "index": 0, "action": {"type": "conditional", "blocks": [{"expression": "open", "actions": []}]}}),
    )
    .unwrap();
    s.execute("proto.conditional.addBlock", &json!({"ids": [btn, other], "index": 0, "path": [1], "else": true})).unwrap();
    assert_eq!(actions(&s, btn), actions(&s, other));
    assert_eq!(blocks(&actions(&s, other)[1]).len(), 2);
}

#[test]
fn junk_params_never_panic() {
    let (mut s, _, _, btn) = setup();
    s.execute(
        "proto.action.add",
        &json!({"id": btn, "index": 0, "action": {"type": "conditional", "blocks": [{"expression": "open", "actions": []}]}}),
    )
    .unwrap();
    let cmds = [
        "proto.action.add",
        "proto.action.set",
        "proto.action.remove",
        "proto.action.move",
        "proto.conditional.addBlock",
        "proto.conditional.setCondition",
        "proto.conditional.removeBlock",
    ];
    let deep: Vec<usize> = vec![1; 401];
    let junk = [
        json!({"id": btn, "index": 0, "path": deep}),
        json!({"id": btn, "index": 1e300, "path": [1, 0, 0], "to": -5, "block": "x"}),
        json!({"id": btn, "index": 0, "path": [1, 0, 0], "action": {"type": "conditional", "blocks": "no"}}),
        json!({"id": btn, "index": 0, "path": [1], "block": 0, "expression": "((((((((((((((((((((((((((((((((((((((((1"}),
        json!({"id": btn, "index": 0, "path": [1], "block": 0, "expression": 5}),
        json!({"id": btn, "index": 0, "path": [1], "block": 0, "condition": {"op": {"op": "and", "args": []}}}),
        json!({"id": btn, "index": 0, "path": [1, 0, 0], "action": {"type": "setVariable", "variable": "count", "expression": "count + 🙂"}}),
        json!({"id": btn, "index": 0, "path": [1, 0, 0], "action": {"type": "setVariable", "variable": "count", "value": 1, "expression": null}}),
        json!({"id": btn, "index": 0, "path": [null], "to": null}),
    ];
    for c in cmds {
        for j in &junk {
            if let Err(crate::EngineError::Internal { cmd, msg }) = s.execute(c, j) {
                panic!("{cmd} panicked on {j}: {msg}");
            }
        }
    }
    // Deeply nested actions in a param are refused, not followed.
    let mut a = json!({"type": "back"});
    for _ in 0..100 {
        a = json!({"type": "conditional", "blocks": [{"expression": "open", "actions": [a]}]});
    }
    assert!(s.execute("proto.action.add", &json!({"id": btn, "index": 0, "action": a})).is_err());
}

#[test]
fn default_condition_uses_a_boolean_first() {
    let (s, ..) = setup();
    let vars = &s.doc().unwrap().variables;
    assert_eq!(default_condition(vars).display(vars), "open == false");
    let mut only_text = vars.clone();
    only_text.variables.retain(|v| v.name == "label");
    assert_eq!(default_condition(&only_text).display(&only_text), "label == \"Hi\"");
    let none = figcraft_doc::Variables::default();
    assert_eq!(default_condition(&none), Expr::Value(VariableValue::Boolean(true)));
}

#[test]
fn hotspots_are_the_visible_layers_with_interactions_clipped_by_their_frames() {
    let (mut s, a, b, btn) = setup();
    let ex = |s: &mut Session, c: &str, p: Value| id_of(&s.execute(c, &p).unwrap()["id"]);
    // A layer half outside A (A clips), one hidden, one without interactions.
    let half = ex(&mut s, "shape.rectangle", json!({"x": 80, "y": 50, "width": 40, "height": 20, "parent": a})).unwrap();
    let hidden = ex(&mut s, "shape.rectangle", json!({"x": 10, "y": 60, "width": 20, "height": 20, "parent": a})).unwrap();
    ex(&mut s, "shape.rectangle", json!({"x": 40, "y": 60, "width": 20, "height": 20, "parent": a}));
    for id in [half, hidden] {
        s.execute("proto.interaction.add", &json!({"id": id, "dest": b})).unwrap();
    }
    s.execute("node.set", &json!({"id": hidden, "props": {"visible": false}})).unwrap();
    s.execute("node.set", &json!({"id": a, "props": {"clip": true}})).unwrap();
    let doc = s.doc().unwrap();
    let hs = hotspots(doc, a);
    assert_eq!(hs.iter().map(|h| h.0).collect::<Vec<_>>(), [btn, half], "bottom-most first, visible only");
    let r = hs[1].1;
    assert_eq!((r.x0, r.y0, r.x1, r.y1), (80.0, 50.0, 100.0, 70.0), "clipped by A");
    assert!(hotspots(doc, b).is_empty());
    assert!(hotspots(doc, NodeId(987_654)).is_empty());
}
