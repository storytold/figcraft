//! REST import of conditional, set-variable-mode and expression prototype actions. Every fixture is
//! synthetic, written by hand from the published REST schema (`Action`, `ConditionalBlock`,
//! `VariableData`, `Expression`).

use super::proto_variable_tests::{FLAG, LABEL, SPACE, THEME, alias, at, variables_json};
use super::tests::pointers;
use super::*;
use figcraft_doc::proto::{Action, ConditionalBlock, Expr, ExprOp, Transition, walk};
use figcraft_doc::{VariableValue, Variables};
use serde_json::json;

fn data(ty: &str, value: Value) -> Value {
    json!({"type": ty, "value": value})
}

fn var(id: &str) -> Value {
    data("VARIABLE_ALIAS", alias(id))
}

fn expr(function: &str, args: Vec<Value>) -> Value {
    json!({"type": "EXPRESSION", "resolvedType": "FLOAT", "value": {"expressionFunction": function, "expressionArguments": args}})
}

fn set(variable: &str, value: Value) -> Value {
    json!({"type": "SET_VARIABLE", "variableId": variable, "variableValue": value})
}

fn navigate(dest: &str) -> Value {
    json!({"type": "NODE", "destinationId": dest, "navigation": "NAVIGATE", "transition": {"type": "DISSOLVE", "duration": 200, "easing": {"type": "LINEAR"}}})
}

fn on(trigger: &str, actions: Vec<Value>) -> Value {
    json!({"trigger": {"type": trigger}, "actions": actions})
}

fn file(interactions: Vec<Value>) -> Value {
    json!({"name": "Logic", "document": {"id": "0:0", "type": "DOCUMENT", "children": [
        {"id": "0:1", "name": "Page", "type": "CANVAS", "children": [
            {"id": "1:1", "name": "Home", "type": "FRAME", "relativeTransform": at(0.0, 0.0), "size": {"x": 200, "y": 200}, "children": [
                {"id": "1:2", "name": "Button", "type": "RECTANGLE", "relativeTransform": at(10.0, 10.0), "size": {"x": 40, "y": 20},
                 "interactions": interactions}
            ]},
            {"id": "1:3", "name": "Details", "type": "FRAME", "relativeTransform": at(300.0, 0.0), "size": {"x": 200, "y": 200}}
        ]}
    ]}})
}

fn import_file(interactions: Vec<Value>) -> (Document, Vec<String>) {
    import_with(&file(interactions), Some(&variables_json()), &no_images).unwrap()
}

fn button(doc: &Document) -> &Node {
    doc.nodes().find(|n| n.name == "Button").unwrap()
}

fn has(warnings: &[String], s: &str) -> bool {
    warnings.iter().any(|w| w.contains(s))
}

fn num(x: f64) -> Expr {
    Expr::Value(VariableValue::Number(x))
}

#[test]
fn if_else_with_expressions() {
    let (doc, warnings) = import_file(vec![on(
        "ON_CLICK",
        vec![
            set(SPACE, expr("ADDITION", vec![var(SPACE), data("FLOAT", json!(1))])),
            json!({"type": "CONDITIONAL", "conditionalBlocks": [
                {"condition": expr("GREATER_THAN", vec![var(SPACE), data("FLOAT", json!(10))]), "actions": [navigate("1:3")]},
                {"condition": expr("AND", vec![var(FLAG), expr("NOT", vec![data("BOOLEAN", json!(false))])]), "actions": []},
                {"actions": [set(FLAG, data("BOOLEAN", json!(false)))]},
                {"condition": var(FLAG), "actions": [{"type": "BACK"}]}
            ]}),
        ],
    )]);
    let details = doc.nodes().find(|n| n.name == "Details").unwrap().id;
    let its = &button(&doc).interactions;
    assert_eq!(its.len(), 1, "{warnings:?}");
    let it = &its[0];
    assert_eq!(
        it.actions[0],
        Action::SetVariable { variable: SPACE.into(), value: Value::Null, expr: Some(Expr::op(ExprOp::Add, vec![Expr::var(SPACE), num(1.0)])) }
    );
    let Action::Conditional { blocks } = &it.actions[1] else { panic!("{it:?}") };
    assert_eq!(blocks.len(), 3, "blocks after the else never run and are dropped");
    assert_eq!(
        blocks[0],
        ConditionalBlock {
            condition: Some(Expr::op(ExprOp::Greater, vec![Expr::var(SPACE), num(10.0)])),
            actions: vec![Action::Navigate { dest: details, preserve_scroll: false }],
        }
    );
    assert!(blocks[1].actions.is_empty(), "an empty branch is kept: it stops the else");
    assert_eq!(
        blocks[1].condition,
        Some(Expr::op(ExprOp::And, vec![Expr::var(FLAG), Expr::op(ExprOp::Not, vec![Expr::Value(VariableValue::Boolean(false))])]))
    );
    assert_eq!(blocks[2].condition, None);
    assert_eq!(blocks[2].actions, [Action::SetVariable { variable: FLAG.into(), value: json!(false), expr: None }]);
    // The interaction takes the transition of the navigation inside the branch.
    assert_eq!((it.transition, it.duration_ms), (Transition::Dissolve, 200));
    // The imported variables evaluate it: space/md is 8, so 8 + 1 = 9 is not > 10.
    let vars: &Variables = &doc.variables;
    let Some(Expr::Op { .. }) = blocks[0].condition.as_ref() else { panic!() };
    assert!(!blocks[0].condition.as_ref().unwrap().holds(vars, &Default::default()));
    assert!(blocks[1].condition.as_ref().unwrap().holds(vars, &Default::default()));
    assert!(warnings.iter().all(|w| !w.contains("conditional")), "{warnings:?}");
}

#[test]
fn set_variable_mode_and_variable_references() {
    let (doc, warnings) = import_file(vec![
        on("ON_HOVER", vec![json!({"type": "SET_VARIABLE_MODE", "variableCollectionId": THEME, "variableModeId": "1:1"})]),
        on("MOUSE_LEAVE", vec![json!({"type": "SET_VARIABLE_MODE", "variableCollectionId": THEME, "variableModeId": "9:9"})]),
        on("MOUSE_DOWN", vec![json!({"type": "SET_VARIABLE_MODE", "variableCollectionId": "VariableCollectionId:404"})]),
        on("MOUSE_ENTER", vec![set(LABEL, var(LABEL)), set(LABEL, expr("ADDITION", vec![data("STRING", json!("Size ")), var(SPACE)]))]),
        on("MOUSE_UP", vec![set(SPACE, data("STRING", json!("not a number")))]),
    ]);
    let its = &button(&doc).interactions;
    assert_eq!(its.len(), 2, "{its:#?}");
    assert_eq!(its[0].actions, [Action::SetVariableMode { collection: THEME.into(), mode: "1:1".into() }]);
    assert_eq!(its[1].actions[0], Action::SetVariable { variable: LABEL.into(), value: Value::Null, expr: Some(Expr::var(LABEL)) });
    let Action::SetVariable { expr: Some(e), .. } = &its[1].actions[1] else { panic!() };
    assert_eq!(e.eval(&doc.variables, &Default::default()), Some(VariableValue::String("Size 8".into())));
    for w in ["set-variable-mode actions", "values that don't fit"] {
        assert!(has(&warnings, w), "{w}: {warnings:?}");
    }
}

#[test]
fn unsupported_conditions_skip_the_whole_conditional() {
    let lookup = json!({"type": "EXPRESSION", "value": {"expressionFunction": "VAR_MODE_LOOKUP", "expressionArguments": [var(SPACE)]}});
    let (doc, warnings) = import_file(vec![
        on(
            "ON_PRESS",
            vec![
                json!({"type": "CONDITIONAL", "conditionalBlocks": [{"condition": lookup, "actions": [navigate("1:3")]}, {"actions": [{"type": "BACK"}]}]}),
            ],
        ),
        on("ON_DRAG", vec![set(SPACE, expr("ADDITION", vec![var("VariableID:404"), data("FLOAT", json!(1))]))]),
        on("ON_CLICK", vec![json!({"type": "CONDITIONAL", "conditionalBlocks": [{"condition": var(FLAG), "actions": [navigate("gone")]}]})]),
        on("AFTER_TIMEOUT", vec![set(SPACE, expr("ADDITION", vec![]))]),
    ]);
    assert!(button(&doc).interactions.is_empty(), "{:#?}", button(&doc).interactions);
    for w in ["variable mode lookups", "conditions that can't be evaluated", "variables that aren't in the variables JSON", "without a destination"] {
        assert!(has(&warnings, w), "{w}: {warnings:?}");
    }
}

#[test]
fn variable_mode_lookups_read_a_variable_in_one_mode() {
    let lookup = |v: &str, mode: Value| json!({"type": "EXPRESSION", "value": {"expressionFunction": "VAR_MODE_LOOKUP", "expressionArguments": [var(v), mode]}});
    let (doc, warnings) = import_file(vec![
        // The label as it reads in Light, whatever mode the button is in.
        on("ON_CLICK", vec![set(LABEL, lookup(LABEL, data("STRING", json!("1:0"))))]),
        on(
            "ON_HOVER",
            vec![json!({"type": "CONDITIONAL", "conditionalBlocks": [
                {"condition": expr("EQUALS", vec![lookup(SPACE, json!({"value": "1:0"})), data("FLOAT", json!(8))]), "actions": [navigate("1:3")]}
            ]})],
        ),
        // Not a mode of the variable's collection, or not a variable: skipped.
        on("ON_PRESS", vec![set(LABEL, lookup(LABEL, data("STRING", json!("9:9"))))]),
        on(
            "ON_DRAG",
            vec![set(
                LABEL,
                json!({"type": "EXPRESSION", "value": {"expressionFunction": "VAR_MODE_LOOKUP", "expressionArguments": [data("STRING", json!("1:0")), var(LABEL)]}}),
            )],
        ),
    ]);
    let its = &button(&doc).interactions;
    assert_eq!(its.len(), 2, "{its:#?}");
    let Action::SetVariable { expr: Some(e), .. } = &its[0].actions[0] else { panic!("{its:#?}") };
    assert_eq!(e, &Expr::InMode { variable: LABEL.into(), mode: "1:0".into() });
    // Dark is the default mode: the lookup still reads Light's value.
    assert_eq!(e.eval(&doc.variables, &Default::default()), Some(VariableValue::String("Hello".into())));
    assert_eq!(e.display(&doc.variables), "copy/label[Light]");
    let Action::Conditional { blocks } = &its[1].actions[0] else { panic!() };
    assert!(blocks[0].condition.as_ref().unwrap().holds(&doc.variables, &Default::default()));
    assert!(has(&warnings, "variable mode lookups"), "{warnings:?}");
}

#[test]
fn nested_conditionals_and_depth_limit() {
    // if FLAG { if SPACE == 8 { navigate } else { back } }
    let inner = json!({"type": "CONDITIONAL", "conditionalBlocks": [
        {"condition": expr("EQUALS", vec![var(SPACE), data("FLOAT", json!(8))]), "actions": [navigate("1:3")]},
        {"actions": [{"type": "BACK"}]}
    ]});
    let outer = json!({"type": "CONDITIONAL", "conditionalBlocks": [{"condition": var(FLAG), "actions": [inner]}]});
    let (doc, _) = import_file(vec![on("ON_CLICK", vec![outer])]);
    let it = &button(&doc).interactions[0];
    assert_eq!(walk(&it.actions).count(), 4);
    let mut deep = json!({"type": "BACK"});
    for _ in 0..40 {
        deep = json!({"type": "CONDITIONAL", "conditionalBlocks": [{"condition": var(FLAG), "actions": [deep]}, {"actions": []}]});
    }
    let (doc, warnings) = import_file(vec![on("ON_CLICK", vec![deep])]);
    assert!(has(&warnings, "nested too deeply"), "{warnings:?}");
    assert!(button(&doc).interactions.iter().all(|it| walk(&it.actions).count() < 40));
}

#[test]
fn junk_in_conditional_actions_never_panics() {
    let base = file(vec![
        on(
            "ON_CLICK",
            vec![json!({"type": "CONDITIONAL", "conditionalBlocks": [
                {"condition": expr("LESS_THAN_OR_EQUAL", vec![var(SPACE), expr("MULTIPLICATION", vec![data("FLOAT", json!(2)), var(SPACE)])]),
                 "actions": [navigate("1:3"), set(LABEL, expr("ADDITION", vec![var(LABEL), data("STRING", json!("!"))]))]},
                {"actions": [{"type": "SET_VARIABLE_MODE", "variableCollectionId": THEME, "variableModeId": "1:0"}]}
            ]})],
        ),
        on("ON_HOVER", vec![set(FLAG, expr("OR", vec![var(FLAG), expr("NEGATE", vec![var(FLAG)])]))]),
        on(
            "ON_PRESS",
            vec![set(
                LABEL,
                json!({"type": "EXPRESSION", "value": {"expressionFunction": "VAR_MODE_LOOKUP", "expressionArguments": [var(LABEL), data("STRING", json!("1:1"))]}}),
            )],
        ),
    ]);
    let mut combined = base.clone();
    combined["meta"] = variables_json()["meta"].clone();
    let junk = [json!(null), json!(-1), json!(1e308), json!("x"), json!([]), json!({}), json!(true), alias(FLAG)];
    let mut ptrs = Vec::new();
    pointers(&combined["document"], "/document".into(), &mut ptrs);
    for p in &ptrs {
        for j in &junk {
            let mut v = combined.clone();
            if let Some(slot) = v.pointer_mut(p) {
                *slot = j.clone();
            }
            if let Ok((doc, _)) = import(&v, &no_images) {
                assert!(doc.validate().is_empty(), "{p} = {j}");
                for n in doc.nodes() {
                    for it in &n.interactions {
                        assert!(walk(&it.actions).filter_map(Action::dest).all(|d| doc.contains(d)), "{p} = {j}");
                        for a in walk(&it.actions) {
                            assert!(a.variables().iter().all(|v| doc.variables.get(v).is_some()), "{p} = {j}");
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn stored_variable_ids_keep_imported_mode_lookup_identity() {
    for collection in [false, true] {
        let mut vars = variables_json();
        if collection {
            let mut shadow = vars["meta"]["variableCollections"][THEME].clone();
            shadow["id"] = json!("VariableCollectionId:0:0");
            shadow["name"] = json!(THEME);
            shadow["modes"] = json!([{"modeId": "other", "name": "Other"}]);
            vars["meta"]["variableCollections"]["VariableCollectionId:0:0"] = shadow;
        } else {
            let mut shadow = vars["meta"]["variables"][SPACE].clone();
            shadow["id"] = json!("VariableID:0:0");
            shadow["name"] = json!(LABEL);
            vars["meta"]["variables"]["VariableID:0:0"] = shadow;
        }
        let lookup = expr("VAR_MODE_LOOKUP", vec![var(LABEL), data("STRING", json!("1:0"))]);
        let (doc, warnings) = import_with(&file(vec![on("ON_CLICK", vec![set(LABEL, lookup)])]), Some(&vars), &no_images).unwrap();
        let actions = &button(&doc).interactions[0].actions;
        assert_eq!(
            actions[0],
            Action::SetVariable {
                variable: LABEL.into(),
                value: Value::Null,
                expr: Some(Expr::InMode { variable: LABEL.into(), mode: "1:0".into() }),
            },
            "{warnings:?}"
        );
    }
}
