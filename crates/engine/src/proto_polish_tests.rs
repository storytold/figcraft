//! Variable mode lookups in prototype expressions through the interaction commands: typed as text
//! or given as JSON by name, stored by id, checked, and evaluated where the interaction runs (the
//! current mode of a collection follows set variable mode actions; a lookup reads its own mode).
//! Overlays opened through the commands close on a click outside unless told not to, as in the
//! interaction editor.

use figcraft_doc::proto::{Action, Expr, ExprOp};
use figcraft_doc::{NodeId, VariableValue};
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::proto_run::{branch, set_mode, set_variable};

/// Frames A and B, a button in A (click → navigate to B), and a collection Theme (Light, Dark)
/// with a text variable `label` ("Day" in Light, "Night" in Dark).
fn setup() -> (Session, NodeId, NodeId) {
    let mut s = Session::new();
    let id = |s: &mut Session, c: &str, p: Value| id_of(&s.execute(c, &p).unwrap()["id"]).unwrap();
    let a = id(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 100, "name": "A"}));
    let b = id(&mut s, "frame.create", json!({"x": 200, "y": 0, "width": 100, "height": 100, "name": "B"}));
    let btn = id(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 20, "parent": a}));
    s.execute("variable.createCollection", &json!({"name": "Theme", "modes": ["Light", "Dark"]})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "label", "type": "string", "value": "Day"})).unwrap();
    s.execute("variable.set", &json!({"variable": "label", "mode": "Dark", "value": "Night"})).unwrap();
    s.execute("proto.interaction.add", &json!({"id": btn, "dest": b})).unwrap();
    (s, a, btn)
}

/// (collection id, Light id, Dark id, label id).
fn ids(s: &Session) -> (String, String, String, String) {
    let vars = &s.doc().unwrap().variables;
    let c = vars.collection("Theme").unwrap();
    (c.id.clone(), c.modes[0].id.clone(), c.modes[1].id.clone(), vars.get("label").unwrap().id.clone())
}

fn text(s: &str) -> Expr {
    Expr::Value(VariableValue::String(s.into()))
}

#[test]
fn conditions_on_a_collections_mode_follow_set_variable_mode() {
    let (mut s, a, btn) = setup();
    let (theme, light, dark, label) = ids(&s);
    // If Theme is in Dark, set label to its Light value; else switch Theme to Dark.
    s.execute(
        "proto.action.add",
        &json!({"id": btn, "index": 0, "action": {"type": "conditional", "blocks": [
            {"expression": "mode(Theme) == \"Dark\"", "actions": [{"type": "setVariable", "variable": "label", "expression": "label[Light] + \"!\""}]},
            {"actions": [{"type": "setVariableMode", "collection": "Theme", "mode": "Dark"}]},
        ]}}),
    )
    .unwrap();
    let doc = s.doc().unwrap().clone();
    let Some(Action::Conditional { blocks }) = doc.node(btn).unwrap().interactions[0].actions.get(1).cloned() else { panic!() };
    assert_eq!(blocks[0].condition, Some(Expr::op(ExprOp::Equals, vec![Expr::Mode { collection: theme.clone() }, text("Dark")])), "stored by id");
    let Some(Action::SetVariable { expr: Some(e), .. }) = blocks[0].actions.first() else { panic!() };
    assert_eq!(e, &Expr::op(ExprOp::Add, vec![Expr::InMode { variable: label.clone(), mode: light.clone() }, text("!")]));
    // In Light the else runs; once the prototype switched Theme to Dark, the if.
    let mut doc = doc;
    assert!(matches!(branch(&doc, btn, &blocks).as_slice(), [Action::SetVariableMode { .. }]));
    assert!(set_mode(&mut doc, a, "Theme", "Dark"));
    assert_eq!(doc.modes_for(btn).get(&theme), Some(&dark));
    let run = branch(&doc, btn, &blocks);
    let [Action::SetVariable { variable, value, expr }] = run.as_slice() else { panic!("{run:?}") };
    // The lookup reads Light's value although the button is in Dark.
    assert!(set_variable(&mut doc, btn, variable, value, expr.as_ref()));
    assert_eq!(doc.variables.resolve(&label, &doc.modes_for(btn)), Some(VariableValue::String("Day!".into())));
}

#[test]
fn json_lookups_by_name_are_stored_by_id_and_checked() {
    let (mut s, _, btn) = setup();
    let (theme, light, _, label) = ids(&s);
    let r = s.execute(
        "proto.action.add",
        &json!({"id": btn, "index": 0, "action": {"type": "conditional", "blocks": [
            {"condition": {"op": {"op": "notEqual", "args": [{"inMode": {"variable": "label", "mode": "Light"}}, {"mode": {"collection": "Theme"}}]}}, "actions": []},
        ]}}),
    );
    assert!(r.is_ok(), "{r:?}");
    let doc = s.doc().unwrap();
    let Some(Action::Conditional { blocks }) = doc.node(btn).unwrap().interactions[0].actions.get(1) else { panic!() };
    assert_eq!(
        blocks[0].condition,
        Some(Expr::op(ExprOp::NotEqual, vec![Expr::InMode { variable: label, mode: light }, Expr::Mode { collection: theme }]))
    );
    assert_eq!(blocks[0].condition.as_ref().unwrap().display(&doc.variables), "label[Light] != mode(Theme)");
    // Unknown modes and collections, and a mode used as a number, fail with what's wrong.
    for (expression, why) in [
        ("label[Sepia] == \"x\"", "Theme has no mode `Sepia`"),
        ("mode(Colors) == \"x\"", "No variable collection `Colors`"),
        ("mode(Theme) > 1", "`>` compares two numbers or two texts"),
    ] {
        let err =
            s.execute("proto.conditional.addBlock", &json!({"id": btn, "index": 0, "path": [1], "expression": expression})).unwrap_err().to_string();
        assert!(err.contains(why), "{expression}: {err}");
    }
    let err = s
        .execute("proto.action.add", &json!({"id": btn, "index": 0, "action": {"type": "setVariable", "variable": "label", "value": null, "expr": {"inMode": {"variable": "label", "mode": "Sepia"}}}}))
        .unwrap_err()
        .to_string();
    assert!(err.contains("Theme has no mode `Sepia`"), "{err}");
}

#[test]
fn prototype_inputs_are_text_layers_only_and_default_on() {
    let (mut s, a, _) = setup();
    let id = |s: &mut Session, c: &str, p: Value| id_of(&s.execute(c, &p).unwrap()["id"]).unwrap();
    let t = id(&mut s, "text.create", json!({"text": "Name", "x": 10, "y": 40, "parent": a}));
    let input = |s: &Session, id: NodeId| s.doc().unwrap().node(id).unwrap().kind.text().unwrap().input;
    assert!(!input(&s, t), "off by default");
    s.execute("proto.input.set", &json!({"ids": [t]})).unwrap();
    assert!(input(&s, t));
    s.execute("proto.input.set", &json!({"ids": [t], "enabled": false})).unwrap();
    assert!(!input(&s, t));
    // The selection is the default target.
    s.execute("select.set", &json!({"ids": [t]})).unwrap();
    s.execute("proto.input.set", &json!({})).unwrap();
    assert!(input(&s, t));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(!input(&s, t), "one undo step");
    // Text only.
    let r = id(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10, "parent": a}));
    let err = s.execute("proto.input.set", &json!({"ids": [r]})).unwrap_err().to_string();
    assert!(err.contains("text layers"), "{err}");
}

#[test]
fn overlays_from_commands_close_on_a_click_outside_by_default() {
    let (mut s, _, btn) = setup();
    let menu = id_of(&s.execute("frame.create", &json!({"x": 400, "y": 0, "width": 80, "height": 60, "name": "Menu"})).unwrap()["id"]).unwrap();
    for (extra, closes) in [(json!({}), true), (json!({"closeOnOutside": false}), false), (json!({"close_on_outside": false}), false)] {
        let mut action = json!({"type": "openOverlay", "dest": menu});
        if let (Some(o), Value::Object(x)) = (action.as_object_mut(), extra) {
            o.extend(x);
        }
        s.execute("proto.action.set", &json!({"id": btn, "index": 0, "path": [0], "action": action})).unwrap();
        let doc = s.doc().unwrap();
        let Some(Action::OpenOverlay { close_on_outside, .. }) = doc.node(btn).unwrap().interactions[0].actions.first() else { panic!() };
        assert_eq!(*close_on_outside, closes);
    }
}
