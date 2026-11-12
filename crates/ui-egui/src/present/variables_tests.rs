//! The player runs variable actions: expressions, conditional branches and variable modes.

use std::sync::Arc;

use figcraft_doc::proto::{Action, ConditionalBlock, Expr, ExprOp, Interaction, Trigger};
use figcraft_doc::{Document, NodeId, VariableValue};
use figcraft_engine::Session;
use figcraft_engine::params::id_of;
use serde_json::json;

use super::Presenter;

/// Screens A and B; a button on A; a number `count` (0) and a colour `bg` (white in Light, black in
/// Dark) bound to A's fill.
fn setup() -> (Document, NodeId, NodeId, NodeId) {
    let mut s = Session::new();
    let mut make = |cmd: &str, p: serde_json::Value| id_of(&s.execute(cmd, &p).unwrap()["id"]).unwrap();
    let a = make("frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 100, "name": "A"}));
    let b = make("frame.create", json!({"x": 200, "y": 0, "width": 100, "height": 100, "name": "B"}));
    let button = make("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20, "parent": a}));
    s.execute("variable.createCollection", &json!({"name": "Theme", "modes": ["Light", "Dark"]})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "count", "type": "number", "value": 0})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "bg", "type": "color", "value": "#FFFFFF"})).unwrap();
    s.execute("variable.set", &json!({"variable": "bg", "mode": "Dark", "value": "#000000"})).unwrap();
    s.execute("variable.bind", &json!({"ids": [a], "field": "fill", "variable": "bg"})).unwrap();
    (s.doc().unwrap().clone(), a, b, button)
}

fn on_click(doc: &mut Document, node: NodeId, actions: Vec<Action>) {
    let it =
        Interaction { trigger: Trigger::OnClick, actions, transition: Default::default(), duration_ms: 0, easing: Default::default(), delay_ms: 0 };
    doc.get_mut(node).unwrap().interactions.push(it);
}

fn count_of(p: &Presenter) -> Option<VariableValue> {
    let d = p.doc.as_ref()?;
    let id = d.variables.get("count")?.id.clone();
    d.variables.resolve(&id, &Default::default())
}

#[test]
fn counter_then_conditional_navigation() {
    let (mut doc, a, b, button) = setup();
    let count = doc.variables.get("count").unwrap().id.clone();
    on_click(
        &mut doc,
        button,
        vec![
            Action::SetVariable {
                variable: count.clone(),
                value: json!(null),
                expr: Some(Expr::op(ExprOp::Add, vec![Expr::var(&count), Expr::Value(VariableValue::Number(1.0))])),
            },
            // Evaluated after the increment.
            Action::Conditional {
                blocks: vec![ConditionalBlock {
                    condition: Some(Expr::op(ExprOp::GreaterOrEqual, vec![Expr::var(&count), Expr::Value(VariableValue::Number(2.0))])),
                    actions: vec![Action::Navigate { dest: b, preserve_scroll: false }],
                }],
            },
        ],
    );
    let mut p = Presenter::new(a);
    p.sync(&Arc::new(doc), 0.0);
    assert!(p.fire(button, 0, None, 1.0));
    assert_eq!(p.current, a, "count is 1: no navigation yet");
    assert_eq!(count_of(&p), Some(VariableValue::Number(1.0)));
    assert!(p.fire(button, 0, None, 2.0));
    assert_eq!(p.current, b, "count is 2: the branch navigates");
    assert_eq!(count_of(&p), Some(VariableValue::Number(2.0)));
    // Restarting starts the variables over.
    p.restart(a, 3.0);
    assert_eq!(count_of(&p), Some(VariableValue::Number(0.0)));
}

#[test]
fn set_variable_mode_recolours_bound_layers() {
    let (mut doc, a, _, button) = setup();
    let theme = doc.variables.collection("Theme").unwrap().id.clone();
    // Actions store mode ids (commands and imports turn names into them).
    let dark = doc.variables.collection("Theme").unwrap().find_mode("Dark").unwrap().id.clone();
    on_click(&mut doc, button, vec![Action::SetVariableMode { collection: theme.clone(), mode: dark }]);
    let hex = |p: &Presenter| p.doc.as_ref().and_then(|d| d.node(a)?.fills.first()?.solid_color()).map(|c| c.to_hex());
    let mut p = Presenter::new(a);
    p.sync(&Arc::new(doc), 0.0);
    assert_eq!(hex(&p).as_deref(), Some("FFFFFF"));
    assert!(p.fire(button, 0, None, 1.0));
    assert_eq!(hex(&p).as_deref(), Some("000000"));
    // An unknown mode does nothing.
    let mut d2 = p.doc.clone().unwrap();
    on_click(&mut d2, button, vec![Action::SetVariableMode { collection: theme, mode: "Sepia".into() }]);
    let mut p2 = Presenter::new(a);
    p2.sync(&Arc::new(d2), 0.0);
    assert!(!p2.fire(button, 1, None, 1.0));
}

#[test]
fn deeply_nested_conditionals_stop() {
    let (mut doc, a, b, button) = setup();
    let mut act = Action::Navigate { dest: b, preserve_scroll: false };
    for _ in 0..(figcraft_doc::proto::MAX_ACTION_DEPTH * 3) {
        act = Action::Conditional { blocks: vec![ConditionalBlock { condition: None, actions: vec![act] }] };
    }
    on_click(&mut doc, button, vec![act]);
    let mut p = Presenter::new(a);
    p.sync(&Arc::new(doc), 0.0);
    assert!(!p.fire(button, 0, None, 1.0));
    assert_eq!(p.current, a);
}

#[test]
fn stored_variable_ids_keep_player_collection_identity() {
    for missing in [false, true] {
        let (mut doc, a, _, button) = setup();
        let theme = doc.variables.collections[0].clone();
        let mut shadow = theme.clone();
        shadow.id = "shadow".into();
        shadow.name = theme.id.clone();
        doc.variables.collections.insert(0, shadow);
        if missing {
            doc.variables.collections.retain(|c| c.id != theme.id);
        }
        let dark = theme.find_mode("Dark").unwrap().id.clone();
        on_click(&mut doc, button, vec![Action::SetVariableMode { collection: theme.id.clone(), mode: dark }]);
        let mut p = Presenter::new(a);
        p.sync(&Arc::new(doc), 0.0);
        assert_eq!(p.fire(button, 0, None, 1.0), !missing);
        let played = p.doc.as_ref().unwrap();
        let modes = &played.node(played.page_of(a).unwrap()).unwrap().var_modes;
        assert!(!modes.contains_key("shadow"));
        if !missing {
            let fill = played.node(a).unwrap().fills[0].solid_color().unwrap();
            assert_eq!(fill.to_hex(), "000000");
        }
    }
}
