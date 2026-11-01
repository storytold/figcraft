//! Prototype actions on variables (set variable with expressions, set variable mode, conditional
//! branches) and conditional actions through the interaction commands.

use figcraft_doc::proto::{Action, ConditionalBlock, Expr, ExprOp};
use figcraft_doc::{Document, NodeId, NodeKind, VariableType, VariableValue};
use serde_json::json;

use crate::Session;
use crate::params::id_of;
use crate::proto_run::{branch, literal, set_mode, set_variable};

fn var_id(doc: &Document, name: &str) -> String {
    doc.variables.get(name).map(|v| v.id.clone()).unwrap_or_default()
}

fn num(n: f64) -> Expr {
    Expr::Value(VariableValue::Number(n))
}

/// Two frames; a counter, a label and a colour in a collection with Light and Dark modes; a text
/// layer bound to the label and a rectangle bound to the colour.
fn setup() -> (Session, NodeId, NodeId, NodeId, NodeId) {
    let mut s = Session::new();
    let a = id_of(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 100, "height": 100, "name": "A"})).unwrap()["id"]).unwrap();
    let b = id_of(&s.execute("frame.create", &json!({"x": 200, "y": 0, "width": 100, "height": 100, "name": "B"})).unwrap()["id"]).unwrap();
    let t = id_of(&s.execute("text.create", &json!({"text": "x", "x": 10, "y": 10, "parent": a})).unwrap()["id"]).unwrap();
    let r = id_of(&s.execute("shape.rectangle", &json!({"x": 10, "y": 50, "width": 10, "height": 10, "parent": a})).unwrap()["id"]).unwrap();
    s.execute("variable.createCollection", &json!({"name": "Theme", "modes": ["Light", "Dark"]})).unwrap();
    for (name, kind, value) in [("count", "number", json!(0)), ("label", "string", json!("Zero")), ("bg", "color", json!("#FFFFFF"))] {
        s.execute("variable.create", &json!({"collection": "Theme", "name": name, "type": kind, "value": value})).unwrap();
    }
    s.execute("variable.set", &json!({"variable": "bg", "mode": "Dark", "value": "#000000"})).unwrap();
    s.execute("variable.bind", &json!({"ids": [t], "field": "characters", "variable": "label"})).unwrap();
    s.execute("variable.bind", &json!({"ids": [r], "field": "fill", "variable": "bg"})).unwrap();
    (s, a, b, t, r)
}

fn chars(doc: &Document, t: NodeId) -> String {
    match doc.node(t).map(|n| &n.kind) {
        Some(NodeKind::Text(td)) => td.characters.clone(),
        _ => String::new(),
    }
}

fn fill_hex(doc: &Document, r: NodeId) -> Option<String> {
    doc.node(r).and_then(|n| n.fills.first()).and_then(|p| p.solid_color()).map(|c| c.to_hex())
}

#[test]
fn literals_parse_by_the_variable_type() {
    assert_eq!(literal(VariableType::Number, &json!("4")), Some(VariableValue::Number(4.0)));
    assert_eq!(literal(VariableType::Boolean, &json!(true)), Some(VariableValue::Boolean(true)));
    assert_eq!(literal(VariableType::Boolean, &json!({})), None);
    assert_eq!(literal(VariableType::String, &json!(3)), Some(VariableValue::String("3".into())));
    assert!(matches!(literal(VariableType::Color, &json!("#FF0000")), Some(VariableValue::Color(_))));
    assert_eq!(literal(VariableType::Number, &json!("lots")), None);
}

#[test]
fn set_variable_evaluates_expressions_where_the_layer_is() {
    let (s, a, _, t, _) = setup();
    let mut doc = s.doc().unwrap().clone();
    let (count, label) = (var_id(&doc, "count"), var_id(&doc, "label"));
    // count = count + 1, twice.
    let inc = Expr::op(ExprOp::Add, vec![Expr::var(&count), num(1.0)]);
    for _ in 0..2 {
        assert!(set_variable(&mut doc, a, &count, &json!(null), Some(&inc)));
    }
    assert_eq!(doc.variables.resolve(&count, &Default::default()), Some(VariableValue::Number(2.0)));
    // label = "Count: " + count reaches the bound text.
    let text = Expr::op(ExprOp::Add, vec![Expr::Value(VariableValue::String("Count: ".into())), Expr::var(&count)]);
    assert!(set_variable(&mut doc, t, &label, &json!(null), Some(&text)));
    assert_eq!(chars(&doc, t), "Count: 2");
    // A number result fits a string variable as text; a boolean doesn't fit a number.
    assert!(set_variable(&mut doc, t, &label, &json!(null), Some(&Expr::var(&count))));
    assert_eq!(chars(&doc, t), "2");
    let truth = Expr::op(ExprOp::Equals, vec![Expr::var(&count), num(2.0)]);
    assert!(!set_variable(&mut doc, a, &count, &json!(null), Some(&truth)));
    // Errors (division by zero, missing variables) leave the value alone.
    let div0 = Expr::op(ExprOp::Divide, vec![Expr::var(&count), num(0.0)]);
    assert!(!set_variable(&mut doc, a, &count, &json!(null), Some(&div0)));
    assert!(!set_variable(&mut doc, a, &count, &json!(null), Some(&Expr::var("nope"))));
    assert!(!set_variable(&mut doc, a, "nope", &json!(1), None));
    assert_eq!(doc.variables.resolve(&count, &Default::default()), Some(VariableValue::Number(2.0)));
    // A plain value still works.
    assert!(set_variable(&mut doc, a, &label, &json!("Done"), None));
    assert_eq!(chars(&doc, t), "Done");
}

#[test]
fn set_mode_switches_the_prototype_and_keeps_nested_choices() {
    let (mut s, a, b, _, r) = setup();
    // B chose Light explicitly; a group inside A chose Light too (nested: it wins).
    s.execute("variable.setMode", &json!({"ids": [b], "collection": "Theme", "mode": "Light"})).unwrap();
    let g = id_of(&s.execute("frame.create", &json!({"x": 40, "y": 40, "width": 20, "height": 20, "parent": a})).unwrap()["id"]).unwrap();
    let r2 = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5, "parent": g})).unwrap()["id"]).unwrap();
    s.execute("variable.bind", &json!({"ids": [r2], "field": "fill", "variable": "bg"})).unwrap();
    s.execute("variable.setMode", &json!({"ids": [g], "collection": "Theme", "mode": "Light"})).unwrap();
    let mut doc = s.doc().unwrap().clone();
    let theme = doc.variables.collections[0].clone();
    let dark = theme.modes.iter().find(|m| m.name == "Dark").map(|m| m.id.clone()).unwrap();
    assert_eq!(fill_hex(&doc, r).as_deref(), Some("FFFFFF"));
    assert!(set_mode(&mut doc, a, &theme.id, &dark));
    assert_eq!(fill_hex(&doc, r).as_deref(), Some("000000"), "bound layers follow the new mode");
    assert_eq!(fill_hex(&doc, r2).as_deref(), Some("FFFFFF"), "a mode chosen inside the frame still wins");
    let page = doc.page_of(a).unwrap();
    assert_eq!(doc.node(page).unwrap().var_modes.get(&theme.id), Some(&dark));
    assert_eq!(doc.node(b).unwrap().var_modes.get(&theme.id), Some(&dark), "top-level frames that chose a mode switch too");
    // Collections and modes by name; unknown ones change nothing.
    assert!(set_mode(&mut doc, b, "Theme", "Light"));
    assert_eq!(fill_hex(&doc, r).as_deref(), Some("FFFFFF"), "modes by name");
    assert!(!set_mode(&mut doc, a, "Theme", "nope"));
    assert!(!set_mode(&mut doc, a, "Nope", &dark));
}

#[test]
fn conditional_branches_follow_variables() {
    let (s, a, b, _, _) = setup();
    let doc = s.doc().unwrap().clone();
    let count = var_id(&doc, "count");
    let blocks = vec![
        ConditionalBlock {
            condition: Some(Expr::op(ExprOp::Greater, vec![Expr::var(&count), num(5.0)])),
            actions: vec![Action::Navigate { dest: b, preserve_scroll: false }],
        },
        ConditionalBlock { condition: Some(Expr::op(ExprOp::Equals, vec![Expr::var(&count), num(0.0)])), actions: vec![Action::Back] },
        ConditionalBlock { condition: None, actions: vec![Action::CloseOverlay] },
    ];
    assert_eq!(branch(&doc, a, &blocks), [Action::Back]);
    let mut d2 = doc.clone();
    assert!(set_variable(&mut d2, a, &count, &json!(9), None));
    assert_eq!(branch(&d2, a, &blocks), [Action::Navigate { dest: b, preserve_scroll: false }]);
    assert!(set_variable(&mut d2, a, &count, &json!(3), None));
    assert_eq!(branch(&d2, a, &blocks), [Action::CloseOverlay], "else");
    // Without an else, nothing runs; a condition that errors is false.
    let no_else = &blocks[..1];
    assert!(branch(&d2, a, no_else).is_empty());
    let broken = [ConditionalBlock { condition: Some(Expr::var("nope")), actions: vec![Action::Back] }];
    assert!(branch(&d2, a, &broken).is_empty());
}

#[test]
fn interaction_commands_take_conditional_actions() {
    let (mut s, a, b, _, _) = setup();
    let count = var_id(s.doc().unwrap(), "count");
    let conditional = json!({"type": "conditional", "blocks": [
        {"condition": {"op": {"op": "greaterOrEqual", "args": [{"value": {"type": "alias", "value": count}}, {"value": {"type": "number", "value": 2}}]}},
         "actions": [{"type": "navigate", "dest": b}]},
        {"actions": [{"type": "setVariable", "variable": count, "value": null,
                      "expr": {"op": {"op": "add", "args": [{"value": {"type": "alias", "value": count}}, {"value": {"type": "number", "value": 1}}]}}}]}
    ]});
    s.execute("proto.interaction.add", &json!({"ids": [a], "trigger": {"type": "onClick"}, "actions": [conditional, {"type": "setVariableMode", "collection": "Theme", "mode": "Dark"}]}))
        .unwrap();
    let it = s.doc().unwrap().node(a).unwrap().interactions[0].clone();
    let Action::Conditional { blocks } = &it.actions[0] else { panic!("{it:?}") };
    assert_eq!(blocks.len(), 2);
    assert!(matches!(&blocks[1].actions[0], Action::SetVariable { expr: Some(_), .. }));
    // Collections and modes named by name are stored by id.
    let theme = s.doc().unwrap().variables.collections[0].clone();
    let dark = theme.modes.iter().find(|m| m.name == "Dark").map(|m| m.id.clone()).unwrap();
    assert_eq!(it.actions[1], Action::SetVariableMode { collection: theme.id, mode: dark });
    // Destinations inside branches must exist.
    let bad = json!({"type": "conditional", "blocks": [{"actions": [{"type": "navigate", "dest": 99_999}]}]});
    assert!(s.execute("proto.interaction.add", &json!({"ids": [a], "action": bad})).is_err());
    // Saved files keep them.
    let saved = crate::cmd::file::file_bytes(s.ds().unwrap()).unwrap();
    let (back, _) = figcraft_format::load_bytes(&saved).unwrap();
    assert_eq!(back.node(a).unwrap().interactions, s.doc().unwrap().node(a).unwrap().interactions, "saved and loaded");
}

#[test]
fn junk_conditional_actions_never_panic() {
    let (mut s, a, b, _, _) = setup();
    let junk = [json!(null), json!(-1), json!(1e308), json!("x"), json!([]), json!({}), json!(true)];
    let shapes = |j: &serde_json::Value| {
        [
            json!({"type": "conditional", "blocks": j}),
            json!({"type": "conditional", "blocks": [{"condition": j, "actions": [{"type": "navigate", "dest": b}]}]}),
            json!({"type": "conditional", "blocks": [{"condition": {"op": {"op": "add", "args": [j, j]}}, "actions": j}]}),
            json!({"type": "setVariable", "variable": "count", "value": j, "expr": j}),
            json!({"type": "setVariable", "variable": "count", "value": null, "expr": {"op": {"op": "divide", "args": [{"value": {"type": "number", "value": 1}}, j]}}}),
            json!({"type": "setVariableMode", "collection": j, "mode": j}),
        ]
    };
    for j in &junk {
        for action in shapes(j) {
            if s.execute("proto.interaction.add", &json!({"ids": [a], "action": action})).is_ok() {
                // Whatever was accepted runs without panicking.
                let mut doc = s.doc().unwrap().clone();
                for it in doc.node(a).unwrap().interactions.clone() {
                    for act in figcraft_doc::proto::walk(&it.actions) {
                        match act {
                            Action::SetVariable { variable, value, expr } => {
                                let _ = set_variable(&mut doc, a, variable, value, expr.as_ref());
                            }
                            Action::SetVariableMode { collection, mode } => {
                                let _ = set_mode(&mut doc, a, collection, mode);
                            }
                            Action::Conditional { blocks } => {
                                let _ = branch(&doc, a, blocks);
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn stored_variable_ids_set_only_the_prototype_action_target() {
    let (mut s, frame, _, _, _) = setup();
    let shadow = var_id(s.doc().unwrap(), "count");
    let target = s.execute("variable.create", &json!({"collection": "Theme", "name": "target", "type": "number", "value": 5})).unwrap();
    let target = target["id"].as_str().unwrap().to_string();
    s.execute("variable.update", &json!({"variable": shadow, "name": target})).unwrap();
    let mut doc = s.doc().unwrap().clone();
    assert!(set_variable(&mut doc, frame, &target, &json!(9), None));
    assert_eq!(doc.variables.resolve(&target, &Default::default()), Some(VariableValue::Number(9.0)));
    assert_eq!(doc.variables.resolve(&shadow, &Default::default()), Some(VariableValue::Number(0.0)));
    doc.variables.variables.retain(|v| v.id != target);
    let before = doc.clone();
    assert!(!set_variable(&mut doc, frame, &target, &json!(11), None));
    assert_eq!(doc, before);
}

#[test]
fn stored_variable_ids_survive_unrelated_interaction_edits() {
    let (mut s, frame, _, _, _) = setup();
    let target = s.execute("variable.create", &json!({"collection": "Theme", "name": "target", "type": "number", "value": 5})).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    s.execute(
        "proto.interaction.add",
        &json!({"id": frame, "actions": [
            {"type": "back"},
            {"type": "setVariable", "variable": "target", "expression": "target + 1"},
            {"type": "setVariableMode", "collection": "Theme", "mode": "Dark"}
        ]}),
    )
    .unwrap();
    let theme = s.doc().unwrap().variables.collections[0].id.clone();
    s.execute("variable.createCollection", &json!({"name": "shadow", "modes": ["Light", "Dark"]})).unwrap();
    s.execute("variable.renameCollection", &json!({"collection": "shadow", "name": theme})).unwrap();
    // Put the namesake before the referenced collection, as variable lookup already does.
    s.doc_mut().unwrap().variables.collections.swap(0, 1);
    s.execute("variable.update", &json!({"variable": "count", "name": target})).unwrap();
    let actions = s.doc().unwrap().node(frame).unwrap().interactions[0].actions.clone();
    for patch in [json!({"durationMs": 450}), json!({"action": {"type": "closeOverlay"}})] {
        let mut p = json!({"id": frame, "index": 0});
        p.as_object_mut().unwrap().extend(patch.as_object().unwrap().clone());
        s.execute("proto.interaction.set", &p).unwrap();
        let it = &s.doc().unwrap().node(frame).unwrap().interactions[0];
        assert_eq!(it.duration_ms, 450);
        assert_eq!(it.actions[1..], actions[1..], "only the supplied fields change");
    }
}

#[test]
fn stored_variable_ids_from_text_are_not_resolved_twice() {
    let (mut s, frame, _, _, _) = setup();
    let target = s.execute("variable.create", &json!({"collection": "Theme", "name": "target", "type": "number", "value": 5})).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    s.execute("variable.set", &json!({"variable": "count", "value": 100})).unwrap();
    s.execute("variable.update", &json!({"variable": "count", "name": target})).unwrap();
    let action = json!({"type": "setVariable", "variable": "target", "expression": "target + 1"});
    s.execute("proto.interaction.add", &json!({"id": frame, "action": action})).unwrap();
    for (cmd, patch) in [
        ("proto.interaction.add", json!({"action": action})),
        ("proto.interaction.set", json!({"action": action})),
        ("proto.interaction.set", json!({"interaction": {"actions": [action]}})),
        ("proto.action.add", json!({"path": [0], "action": action})),
        ("proto.action.set", json!({"path": [0], "action": action})),
    ] {
        let mut p = json!({"id": frame});
        if cmd != "proto.interaction.add" {
            p["index"] = json!(0);
        }
        p.as_object_mut().unwrap().extend(patch.as_object().unwrap().clone());
        let r = s.execute(cmd, &p).unwrap();
        let index = if cmd == "proto.interaction.add" { r["index"].as_u64().unwrap() as usize } else { 0 };
        let Action::SetVariable { variable, expr: Some(expr), .. } = &s.doc().unwrap().node(frame).unwrap().interactions[index].actions[0] else {
            panic!("{cmd}: no set variable expression")
        };
        assert_eq!(variable, &target, "{cmd}");
        assert_eq!(expr.eval(&s.doc().unwrap().variables, &Default::default()), Some(VariableValue::Number(6.0)), "{cmd}");
    }
}

#[test]
fn stored_variable_ids_resolve_only_incoming_action_and_expression_names() {
    let (mut s, frame, _, _, _) = setup();
    let target = s.execute("variable.create", &json!({"collection": "Theme", "name": "target", "type": "number", "value": 5})).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    s.execute("variable.set", &json!({"variable": "count", "value": 100})).unwrap();
    s.execute("variable.update", &json!({"variable": "count", "name": target})).unwrap();
    let value = json!({"type": "setVariable", "variable": "target"});
    s.execute(
        "proto.interaction.add",
        &json!({"id": frame, "actions": [
            {"type": "setVariable", "variable": "target", "value": null, "expr": Expr::var(&target)},
            {"type": "conditional", "blocks": [{"expression": "target == 5", "actions": [
                {"type": "setVariable", "variable": "label", "value": value}
            ]}]}
        ]}),
    )
    .unwrap();
    let doc = s.doc().unwrap();
    let actions = &doc.node(frame).unwrap().interactions[0].actions;
    let Action::SetVariable { variable, expr: Some(expr), .. } = &actions[0] else { panic!("no set variable") };
    assert_eq!(variable, &target);
    // A JSON expression is command input: it accepts names, but an exact id comes first, so the
    // variable renamed to target's id doesn't take its place (#1613).
    assert_eq!(expr.eval(&doc.variables, &Default::default()), Some(VariableValue::Number(5.0)));
    let Action::Conditional { blocks } = &actions[1] else { panic!("no conditional") };
    assert_eq!(blocks[0].condition.as_ref().unwrap().eval(&doc.variables, &Default::default()), Some(VariableValue::Boolean(true)));
    let Action::SetVariable { value: stored, .. } = &blocks[0].actions[0] else { panic!("no nested set variable") };
    assert_eq!(stored, &value, "a literal value is not an action to canonicalize");
}

#[test]
fn stored_variable_ids_switch_only_the_prototype_collection() {
    let (s, frame, _, _, r) = setup();
    let mut doc = s.doc().unwrap().clone();
    let theme = doc.variables.collections[0].clone();
    let mut shadow = theme.clone();
    shadow.id = "shadow".into();
    shadow.name = theme.id.clone();
    doc.variables.collections.insert(0, shadow);
    let dark = theme.find_mode("Dark").unwrap().id.clone();
    assert!(!crate::proto_run::set_mode_by_id(&mut doc, frame, &theme.id, "Dark"), "a stored mode is an id, not a name");
    assert!(crate::proto_run::set_mode_by_id(&mut doc, frame, &theme.id, &dark));
    assert_eq!(fill_hex(&doc, r).as_deref(), Some("000000"));
    let page = doc.page_of(frame).unwrap();
    assert!(!doc.node(page).unwrap().var_modes.contains_key("shadow"));
    // The name-accepting wrapper takes an exact id before a name (#1613).
    assert!(set_mode(&mut doc, frame, &theme.id, "Light"));
    assert!(!doc.node(page).unwrap().var_modes.contains_key("shadow"));
    assert!(set_mode(&mut doc, frame, "shadow", "Light"), "the other collection by its id");
    assert!(doc.node(page).unwrap().var_modes.contains_key("shadow"));
    doc.variables.collections.retain(|c| c.id != theme.id);
    let before = doc.clone();
    assert!(!crate::proto_run::set_mode_by_id(&mut doc, frame, &theme.id, &dark));
    assert_eq!(doc, before);
}
