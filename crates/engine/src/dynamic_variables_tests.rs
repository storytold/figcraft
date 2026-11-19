use serde_json::json;

use crate::Session;
use figcraft_doc::{VariableValue, Variables};

fn boolean(s: &Session, variable: &str, mode: &str) -> bool {
    let doc = s.doc().unwrap();
    let value = doc
        .variables
        .get(variable)
        .and_then(|v| doc.variables.collection_by_id(&v.collection).and_then(|c| c.find_mode(mode)).and_then(|m| v.values.get(&m.id)));
    match value {
        Some(VariableValue::Boolean(value)) => *value,
        other => panic!("expected boolean variable value, got {other:?}"),
    }
}

fn number(s: &Session, variable: &str, mode: &str) -> f64 {
    let doc = s.doc().unwrap();
    let value = doc
        .variables
        .get(variable)
        .and_then(|v| doc.variables.collection_by_id(&v.collection).and_then(|c| c.find_mode(mode)).and_then(|m| v.values.get(&m.id)));
    match value {
        Some(VariableValue::Number(n)) => *n,
        other => panic!("expected numeric variable value, got {other:?}"),
    }
}

fn text(s: &Session, variable: &str, mode: &str) -> String {
    let doc = s.doc().unwrap();
    let value = doc
        .variables
        .get(variable)
        .and_then(|v| doc.variables.collection_by_id(&v.collection).and_then(|c| c.find_mode(mode)).and_then(|m| v.values.get(&m.id)));
    match value {
        Some(VariableValue::String(value)) => value.clone(),
        other => panic!("expected text variable value, got {other:?}"),
    }
}

fn color(s: &Session, variable: &str, mode: &str) -> [u8; 4] {
    let doc = s.doc().unwrap();
    let value = doc
        .variables
        .get(variable)
        .and_then(|v| doc.variables.collection_by_id(&v.collection).and_then(|c| c.find_mode(mode)).and_then(|m| v.values.get(&m.id)));
    match value {
        Some(VariableValue::Color(c)) => c.to_rgba8(),
        other => panic!("expected color variable value, got {other:?}"),
    }
}

#[test]
fn expressions_update_synchronously_and_follow_each_mode() {
    let mut s = Session::new();
    let collection =
        s.execute("variable.createCollection", &json!({"name": "Theme", "modes": ["Light", "Dark"]})).unwrap()["id"].as_str().unwrap().to_string();
    s.execute("variable.create", &json!({"collection": collection, "name": "MenuOpen", "type": "boolean", "values": {"Light": true, "Dark": false}}))
        .unwrap();
    s.execute(
        "variable.create",
        &json!({"collection": collection, "name": "MenuHidden", "type": "boolean", "values": {"Light": false, "Dark": false}}),
    )
    .unwrap();
    s.execute("variable.setExpression", &json!({"variable": "MenuHidden", "expression": "if {MenuOpen} then false else true"})).unwrap();

    assert!(!boolean(&s, "MenuHidden", "Light"));
    assert!(boolean(&s, "MenuHidden", "Dark"));
    s.execute("variable.set", &json!({"variable": "MenuOpen", "mode": "Dark", "value": true})).unwrap();
    assert!(!boolean(&s, "MenuHidden", "Dark"), "the dependent output changes before variable.set returns");
    assert!(!boolean(&s, "MenuHidden", "Light"), "other modes keep their own computed result");
    s.execute("variable.set", &json!({"variable": "MenuOpen", "mode": "Light", "value": false})).unwrap();
    assert!(boolean(&s, "MenuHidden", "Light"));
}

#[test]
fn inherited_values_are_synchronized_before_formulas_and_bindings_update() {
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "Theme", "modes": ["Light", "Dark"]})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "Base", "type": "number", "values": {"Light": 2, "Dark": 5}})).unwrap();
    let extended = s.execute("variable.extendCollection", &json!({"collection": "Theme"})).unwrap();
    let child_collection = extended["id"].as_str().unwrap();
    let child_base = {
        let doc = s.doc().unwrap();
        doc.variables.variables.iter().find(|variable| variable.collection == child_collection && variable.name == "Base").unwrap().id.clone()
    };
    s.execute("variable.create", &json!({"collection": child_collection, "name": "Computed", "type": "number", "values": {"Light": 0, "Dark": 0}}))
        .unwrap();
    s.execute("variable.setExpression", &json!({"variable": "Computed", "expression": format!("{{{child_base}}} * 2")})).unwrap();
    let rect = crate::params::id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 1, "height": 1})).unwrap()["id"]).unwrap();
    s.execute("variable.bind", &json!({"ids": [rect], "field": "width", "variable": "Computed"})).unwrap();

    s.execute("variable.set", &json!({"variable": "Base", "mode": "Light", "value": 3})).unwrap();

    assert_eq!(number(&s, "Computed", "Light"), 6.0);
    assert_eq!(number(&s, "Computed", "Dark"), 10.0);
    assert_eq!(s.doc().unwrap().node(rect).unwrap().width, 6.0, "bound layers use the updated formula before normalization");
}

#[test]
fn inherited_expression_overrides_survive_recomputation() {
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "Theme", "modes": ["Light", "Dark"]})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "Base", "type": "number", "values": {"Light": 2, "Dark": 5}})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "Computed", "type": "number", "values": {"Light": 0, "Dark": 0}})).unwrap();
    s.execute("variable.setExpression", &json!({"variable": "Computed", "expression": "{Base} * 2"})).unwrap();
    let extended = s.execute("variable.extendCollection", &json!({"collection": "Theme"})).unwrap();
    let child_collection = extended["id"].as_str().unwrap();
    let child_computed = {
        let doc = s.doc().unwrap();
        doc.variables.variables.iter().find(|variable| variable.collection == child_collection && variable.name == "Computed").unwrap().id.clone()
    };
    s.execute("variable.set", &json!({"variable": child_computed, "mode": "Light", "value": 99})).unwrap();

    s.execute("variable.set", &json!({"variable": "Base", "mode": "Light", "value": 3})).unwrap();

    assert_eq!(number(&s, &child_computed, "Light"), 99.0, "an overridden value stays user-authored");
    assert_eq!(number(&s, &child_computed, "Dark"), 10.0, "non-overridden modes continue to follow the parent formula");
}

#[test]
fn dependents_wait_for_dynamic_targets_kept_in_inherited_alias_overrides() {
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "Theme", "modes": ["Light", "Dark"]})).unwrap();
    let input = s.execute("variable.create", &json!({"collection": "Theme", "name": "Input", "type": "number", "value": 2})).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "A", "type": "number", "value": 0})).unwrap();
    s.execute("variable.setExpression", &json!({"variable": "A", "expression": format!("{{{input}}} + 1")})).unwrap();
    let extended = s.execute("variable.extendCollection", &json!({"collection": "Theme"})).unwrap();
    let child_collection = extended["id"].as_str().unwrap();
    let (child_input, child_a) = {
        let doc = s.doc().unwrap();
        let id = |name: &str| {
            doc.variables.variables.iter().find(|variable| variable.collection == child_collection && variable.name == name).unwrap().id.clone()
        };
        (id("Input"), id("A"))
    };
    s.execute("variable.create", &json!({"collection": child_collection, "name": "B", "type": "number", "value": 0})).unwrap();
    let child_b = {
        let doc = s.doc().unwrap();
        doc.variables.variables.iter().find(|variable| variable.collection == child_collection && variable.name == "B").unwrap().id.clone()
    };
    s.execute("variable.setExpression", &json!({"variable": child_b, "expression": format!("{{{child_a}}} * 2")})).unwrap();
    s.execute("variable.create", &json!({"collection": child_collection, "name": "Z", "type": "number", "value": 0})).unwrap();
    let child_z = {
        let doc = s.doc().unwrap();
        doc.variables.variables.iter().find(|variable| variable.collection == child_collection && variable.name == "Z").unwrap().id.clone()
    };
    s.execute("variable.setExpression", &json!({"variable": child_z, "expression": format!("{{{child_input}}} * 3")})).unwrap();
    s.execute("variable.set", &json!({"variable": child_a, "mode": "Light", "value": {"alias": child_z}})).unwrap();

    s.execute("variable.set", &json!({"variable": input, "mode": "Light", "value": 4})).unwrap();

    assert_eq!(number(&s, &child_z, "Light"), 12.0);
    assert_eq!(number(&s, &child_b, "Light"), 24.0, "the dependent reads the updated value through the retained alias");
}

#[test]
fn numeric_arithmetic_comparisons_and_conditionals_recompute_without_assignment_prefix() {
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "Theme"})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "CornerRadius", "type": "number", "value": 1})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "StrokeThickness", "type": "number", "value": 0})).unwrap();
    assert!(s.execute("variable.setExpression", &json!({"variable": "StrokeThickness", "expression": "CornerRadius * 2"})).is_err());
    s.execute("variable.setExpression", &json!({"variable": "StrokeThickness", "expression": "if {CornerRadius} == 1 then 20 else 10"})).unwrap();
    assert_eq!(number(&s, "StrokeThickness", "Mode 1"), 20.0);

    s.execute("variable.set", &json!({"variable": "CornerRadius", "value": 2})).unwrap();
    assert_eq!(number(&s, "StrokeThickness", "Mode 1"), 10.0, "a comparison in a condition recalculates synchronously");

    s.execute("variable.setExpression", &json!({"variable": "StrokeThickness", "expression": "{CornerRadius} * 2"})).unwrap();
    assert_eq!(number(&s, "StrokeThickness", "Mode 1"), 4.0, "an expression is entered without an assignment prefix");
}

#[test]
fn clearing_a_dynamic_expression_accepts_explicit_null_and_leaves_the_last_value_static() {
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "Theme"})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "Base", "type": "number", "value": 2})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "Computed", "type": "number", "value": 7})).unwrap();
    s.execute("variable.setExpression", &json!({"variable": "Computed", "expression": "{Base} * 3"})).unwrap();
    assert_eq!(number(&s, "Computed", "Mode 1"), 6.0);

    s.execute("variable.setExpression", &json!({"variable": "Computed", "expression": null})).unwrap();

    assert!(s.doc().unwrap().variables.get("Computed").unwrap().expression.is_none());
    assert_eq!(number(&s, "Computed", "Mode 1"), 6.0, "clearing leaves the last evaluated value as the variable's static value");
    assert!(s.execute("variable.setExpression", &json!({"variable": "Computed"})).is_err(), "omitting expression is still invalid");
}

#[test]
fn dynamic_expressions_support_boolean_value_selection_and_recompute() {
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "Theme", "modes": ["Light", "Dark"]})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "Eligible", "type": "boolean", "values": {"Light": true, "Dark": false}}))
        .unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "Result", "type": "string", "value": "Unset"})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "NumberResult", "type": "number", "value": 99})).unwrap();
    s.execute("variable.setExpression", &json!({"variable": "Result", "expression": "{Eligible} and \"Success\" or \"Fail\""})).unwrap();
    s.execute("variable.setExpression", &json!({"variable": "NumberResult", "expression": "{Eligible} and 0 or 10"})).unwrap();

    assert_eq!(text(&s, "Result", "Light"), "Success");
    assert_eq!(text(&s, "Result", "Dark"), "Fail");
    assert_eq!(number(&s, "NumberResult", "Light"), 0.0, "zero is a selected value, not a falsy fallback");
    assert_eq!(number(&s, "NumberResult", "Dark"), 10.0);
    s.execute("variable.set", &json!({"variable": "Eligible", "mode": "Dark", "value": true})).unwrap();
    assert_eq!(text(&s, "Result", "Dark"), "Success", "the value-selection expression recalculates synchronously");
    assert_eq!(number(&s, "NumberResult", "Dark"), 0.0);
    s.execute("variable.set", &json!({"variable": "Eligible", "mode": "Light", "value": false})).unwrap();
    assert_eq!(text(&s, "Result", "Light"), "Fail");
    s.execute("variable.setExpression", &json!({"variable": "Result", "expression": "{Eligible} and \"\" or \"fallback\""})).unwrap();
    assert_eq!(text(&s, "Result", "Dark"), "", "empty text is also selected normally");
    assert_eq!(text(&s, "Result", "Light"), "fallback");
}

#[test]
fn dynamic_color_constructors_and_lerp_recompute_from_color_variables() {
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "Theme"})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "Base", "type": "color", "value": "#00FF00"})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "Tint", "type": "color", "value": "#000000"})).unwrap();

    assert!(s.execute("variable.setExpression", &json!({"variable": "Tint", "expression": "#FFFFFF"})).is_err());
    s.execute("variable.setExpression", &json!({"variable": "Tint", "expression": "lerp ( hsl ( 1, 100, 100 ), hex ( #000000 ), 0.15 )"})).unwrap();
    assert_eq!(color(&s, "Tint", "Mode 1"), [217, 217, 217, 255]);

    s.execute("variable.setExpression", &json!({"variable": "Tint", "expression": "lerp(hsl(0, 100, 50), {Base}, 0.5)"})).unwrap();
    assert_eq!(color(&s, "Tint", "Mode 1"), [128, 128, 0, 255]);
    s.execute("variable.set", &json!({"variable": "Base", "value": "#0000FF"})).unwrap();
    assert_eq!(color(&s, "Tint", "Mode 1"), [128, 0, 128, 255]);
}

#[test]
fn operator_aliases_and_boolean_unary_minus_evaluate() {
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "Theme"})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "A", "type": "boolean", "value": true})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "B", "type": "boolean", "value": false})).unwrap();
    for (name, expression) in
        [("OrSymbol", "{A} || {B}"), ("AndSymbol", "{A} && {B}"), ("NotSymbol", "!{B}"), ("EqualsSymbol", "{A} = true"), ("MinusBoolean", "-{B}")]
    {
        s.execute("variable.create", &json!({"collection": "Theme", "name": name, "type": "boolean", "value": false})).unwrap();
        s.execute("variable.setExpression", &json!({"variable": name, "expression": expression})).unwrap();
    }

    assert!(boolean(&s, "OrSymbol", "Mode 1"));
    assert!(!boolean(&s, "AndSymbol", "Mode 1"));
    assert!(boolean(&s, "NotSymbol", "Mode 1"));
    assert!(boolean(&s, "EqualsSymbol", "Mode 1"));
    assert!(boolean(&s, "MinusBoolean", "Mode 1"));
}

#[test]
fn hsl_and_lerp_bounds_are_applied_and_color_equality_ignores_alpha() {
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "Theme"})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "Tint", "type": "color", "value": "#000000"})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "HalfAlpha", "type": "color", "value": "#FF000080"})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "Opaque", "type": "color", "value": "#FF0000FF"})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "SameColor", "type": "boolean", "value": false})).unwrap();

    s.execute("variable.setExpression", &json!({"variable": "Tint", "expression": "hsl(720, 150, 50)"})).unwrap();
    assert_eq!(color(&s, "Tint", "Mode 1"), [255, 0, 0, 255], "hue wraps and saturation clamps to 100");
    s.execute("variable.setExpression", &json!({"variable": "Tint", "expression": "hsl(0, 100, 150)"})).unwrap();
    assert_eq!(color(&s, "Tint", "Mode 1"), [255, 255, 255, 255], "lightness clamps to 100");
    s.execute("variable.setExpression", &json!({"variable": "Tint", "expression": "hsl(0, 100, -50)"})).unwrap();
    assert_eq!(color(&s, "Tint", "Mode 1"), [0, 0, 0, 255], "lightness clamps to 0");
    s.execute("variable.setExpression", &json!({"variable": "Tint", "expression": "lerp(hex(#000000), hex(#FFFFFF), -1)"})).unwrap();
    assert_eq!(color(&s, "Tint", "Mode 1"), [0, 0, 0, 255], "lerp clamps values below 0");
    s.execute("variable.setExpression", &json!({"variable": "Tint", "expression": "lerp(hex(#000000), hex(#FFFFFF), 2)"})).unwrap();
    assert_eq!(color(&s, "Tint", "Mode 1"), [255, 255, 255, 255], "lerp clamps values above 1");
    s.execute("variable.setExpression", &json!({"variable": "SameColor", "expression": "{HalfAlpha} == {Opaque}"})).unwrap();
    assert!(boolean(&s, "SameColor", "Mode 1"), "color equality compares visible RGB, not alpha");
}

#[test]
fn division_by_zero_and_mismatched_conditional_branches_do_not_change_values() {
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "Theme"})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "Numerator", "type": "number", "value": 8})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "Denominator", "type": "number", "value": 0})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "Quotient", "type": "number", "value": 17})).unwrap();
    s.execute("variable.setExpression", &json!({"variable": "Quotient", "expression": "{Numerator} / {Denominator}"})).unwrap();
    assert_eq!(number(&s, "Quotient", "Mode 1"), 17.0, "an expression without a result leaves the stored value unchanged");

    s.execute("variable.set", &json!({"variable": "Denominator", "value": 2})).unwrap();
    assert_eq!(number(&s, "Quotient", "Mode 1"), 4.0);
    s.execute("variable.set", &json!({"variable": "Denominator", "value": 0})).unwrap();
    assert_eq!(number(&s, "Quotient", "Mode 1"), 4.0, "later division by zero also preserves the last computed value");

    s.execute("variable.create", &json!({"collection": "Theme", "name": "Flag", "type": "boolean", "value": false})).unwrap();
    assert!(
        s.execute("variable.setExpression", &json!({"variable": "Flag", "expression": "if true then true else 0"})).is_err(),
        "conditional branches with different types are rejected"
    );
    assert!(s.doc().unwrap().variables.get("Flag").unwrap().expression.is_none());
}

#[test]
fn text_comparisons_and_number_text_addition_evaluate() {
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "Theme"})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "Earlier", "type": "string", "value": "Alpha"})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "Later", "type": "string", "value": "Beta"})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "IsEarlier", "type": "boolean", "value": false})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "IsLater", "type": "boolean", "value": false})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "Count", "type": "number", "value": 12})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "Label", "type": "string", "value": ""})).unwrap();

    s.execute("variable.setExpression", &json!({"variable": "IsEarlier", "expression": "{Earlier} < {Later}"})).unwrap();
    s.execute("variable.setExpression", &json!({"variable": "IsLater", "expression": "{Later} > {Earlier}"})).unwrap();
    s.execute("variable.setExpression", &json!({"variable": "Label", "expression": "{Count} + {Earlier}"})).unwrap();
    assert!(boolean(&s, "IsEarlier", "Mode 1"));
    assert!(boolean(&s, "IsLater", "Mode 1"));
    assert_eq!(text(&s, "Label", "Mode 1"), "12Alpha");
}

#[test]
fn deleting_a_referenced_variable_clears_the_formula_and_keeps_its_value() {
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "Theme"})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "Source", "type": "number", "value": 4})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "Computed", "type": "number", "value": 0})).unwrap();
    s.execute("variable.setExpression", &json!({"variable": "Computed", "expression": "{Source} * 2"})).unwrap();
    assert_eq!(number(&s, "Computed", "Mode 1"), 8.0);

    s.execute("variable.delete", &json!({"variable": "Source"})).unwrap();

    let computed = s.doc().unwrap().variables.get("Computed").unwrap();
    assert!(computed.expression.is_none(), "deleting an input detaches formulas that directly reference it");
    assert_eq!(number(&s, "Computed", "Mode 1"), 8.0, "the computed value becomes static rather than dangling");
}

#[test]
fn references_in_other_collections_use_that_collections_default_mode() {
    let mut s = Session::new();
    let output_collection =
        s.execute("variable.createCollection", &json!({"name": "Output", "modes": ["Light", "Dark"]})).unwrap()["id"].as_str().unwrap().to_string();
    let input_collection =
        s.execute("variable.createCollection", &json!({"name": "Input", "modes": ["First", "Second"]})).unwrap()["id"].as_str().unwrap().to_string();
    s.execute(
        "variable.create",
        &json!({"collection": input_collection, "name": "Foreign", "type": "boolean", "values": {"First": false, "Second": true}}),
    )
    .unwrap();
    s.execute(
        "variable.create",
        &json!({"collection": output_collection, "name": "Result", "type": "boolean", "values": {"Light": true, "Dark": true}}),
    )
    .unwrap();
    s.execute("variable.setExpression", &json!({"variable": "Result", "expression": "{Foreign}"})).unwrap();

    assert!(!boolean(&s, "Result", "Light"));
    assert!(!boolean(&s, "Result", "Dark"), "the foreign collection resolves in its own first/default mode");
}

#[test]
fn three_variable_chain_and_diamond_recompute_in_dependency_order() {
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "Theme"})).unwrap();
    for (name, value) in [("A", 2.0), ("B", 0.0), ("C", 0.0), ("D", 0.0), ("E", 0.0)] {
        s.execute("variable.create", &json!({"collection": "Theme", "name": name, "type": "number", "value": value})).unwrap();
    }
    s.execute("variable.setExpression", &json!({"variable": "B", "expression": "{A} * 2"})).unwrap();
    s.execute("variable.setExpression", &json!({"variable": "C", "expression": "{A} + 3"})).unwrap();
    s.execute("variable.setExpression", &json!({"variable": "D", "expression": "{B} + {C}"})).unwrap();
    s.execute("variable.setExpression", &json!({"variable": "E", "expression": "{D} + 1"})).unwrap();
    assert_eq!(number(&s, "B", "Mode 1"), 4.0);
    assert_eq!(number(&s, "C", "Mode 1"), 5.0);
    assert_eq!(number(&s, "D", "Mode 1"), 9.0, "D depends on both sides of the A -> B/C -> D diamond");
    assert_eq!(number(&s, "E", "Mode 1"), 10.0, "E extends the chain beyond the diamond");

    s.execute("variable.set", &json!({"variable": "A", "value": 4})).unwrap();
    assert_eq!(number(&s, "B", "Mode 1"), 8.0);
    assert_eq!(number(&s, "C", "Mode 1"), 7.0);
    assert_eq!(number(&s, "D", "Mode 1"), 15.0);
    assert_eq!(number(&s, "E", "Mode 1"), 16.0);
}

#[test]
fn dynamic_color_opacity_is_mode_specific_and_survives_recalculation() {
    let mut s = Session::new();
    let collection =
        s.execute("variable.createCollection", &json!({"name": "Theme", "modes": ["Light", "Dark"]})).unwrap()["id"].as_str().unwrap().to_string();
    s.execute(
        "variable.create",
        &json!({"collection": collection, "name": "Source", "type": "color", "values": {"Light": "#FF000080", "Dark": "#00FF0080"}}),
    )
    .unwrap();
    s.execute("variable.create", &json!({"collection": collection, "name": "Computed", "type": "color", "value": "#000000"})).unwrap();
    s.execute("variable.create", &json!({"collection": collection, "name": "PassedThrough", "type": "color", "value": "#000000"})).unwrap();
    s.execute("variable.create", &json!({"collection": collection, "name": "Mixed", "type": "color", "value": "#000000"})).unwrap();
    s.execute("variable.setExpression", &json!({"variable": "Computed", "expression": "{Source}"})).unwrap();
    s.execute("variable.setExpression", &json!({"variable": "PassedThrough", "expression": "{Computed}"})).unwrap();
    s.execute("variable.setExpression", &json!({"variable": "Mixed", "expression": "lerp({Source}, hex(#0000FF80), 0.5)"})).unwrap();
    assert_eq!(color(&s, "Computed", "Light"), [255, 0, 0, 255]);
    assert_eq!(color(&s, "Computed", "Dark"), [0, 255, 0, 255]);
    assert_eq!(color(&s, "PassedThrough", "Dark")[3], 255, "a computed color's opacity is not an expression input");
    assert_eq!(color(&s, "Mixed", "Dark")[3], 255, "lerp ignores both endpoint alphas");

    s.execute("variable.setExpressionOpacity", &json!({"variable": "Computed", "mode": "Dark", "opacity": 25})).unwrap();
    assert_eq!(color(&s, "Computed", "Dark"), [0, 255, 0, 64]);
    assert_eq!(color(&s, "Computed", "Light"), [255, 0, 0, 255], "another mode keeps its own expression opacity");
    assert_eq!(color(&s, "PassedThrough", "Dark")[3], 255, "downstream expressions ignore this variable's opacity override");
    let doc_json = serde_json::to_value(s.doc().unwrap()).unwrap();
    let restored: figcraft_doc::Document = serde_json::from_value(doc_json).unwrap();
    let dark_mode = restored.variables.collection("Theme").unwrap().find_mode("Dark").unwrap().id.clone();
    assert_eq!(
        restored.variables.get("Computed").unwrap().expression_opacity.get(&dark_mode),
        Some(&0.25),
        "the override is saved with the document"
    );

    s.execute("variable.set", &json!({"variable": "Source", "mode": "Dark", "value": "#0000FF80"})).unwrap();
    assert_eq!(color(&s, "Computed", "Dark"), [0, 0, 255, 64], "source changes keep the explicit opacity");
    s.execute("variable.setExpressionOpacity", &json!({"variable": "Computed", "mode": "Dark", "opacity": null})).unwrap();
    assert_eq!(color(&s, "Computed", "Dark"), [0, 0, 255, 255], "clearing the override restores full opacity");
    assert!(s.execute("variable.setExpressionOpacity", &json!({"variable": "Computed", "mode": "Dark", "opacity": 101})).is_err());
}

#[test]
fn dynamic_expression_command_documents_syntax_for_discovery() {
    let command =
        crate::all_commands().iter().find(|command| command.id == "variable.setExpression").expect("dynamic expression command is registered");
    for syntax in [
        "expression only",
        "if condition then value else value",
        "condition and value or fallback",
        "empty strings and 0 are selected normally",
        "group boolean logic with parentheses",
        "wrap every variable reference in braces",
        "hsl(h,s,l)",
        "hue wraps by 360 degrees",
        "s/l clamp to 0..100 percent",
        "hex(#RRGGBB)",
        "lerp({Brand}, hex(#111111), 0.2)",
        "Division by zero leaves the current value unchanged",
        "Deleting a referenced variable clears the formula and keeps its last value",
    ] {
        assert!(command.params.contains(syntax), "command discovery docs should contain `{syntax}`");
    }
    assert!(command.params.contains("Expressions ignore color alpha"));
    let opacity_command = crate::all_commands()
        .iter()
        .find(|command| command.id == "variable.setExpressionOpacity")
        .expect("dynamic color opacity command is registered");
    assert!(opacity_command.params.contains("opacity (0..100 percent"));
    assert!(opacity_command.params.contains("null removes the override"));
}

#[test]
fn renaming_a_formula_input_preserves_its_reference_and_recalculation() {
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "Theme"})).unwrap();
    let input_id = s.execute("variable.create", &json!({"collection": "Theme", "name": "Count", "type": "number", "value": 4})).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "UnderLimit", "type": "boolean", "value": true})).unwrap();
    s.execute("variable.setExpression", &json!({"variable": "UnderLimit", "expression": "{Count} < 5"})).unwrap();

    s.execute("variable.update", &json!({"variable": input_id, "name": "Limit"})).unwrap();
    let doc = s.doc().unwrap();
    let formula = doc.variables.get("UnderLimit").unwrap().expression.as_ref().unwrap();
    assert_eq!(formula.variables(), vec![input_id.as_str()]);
    let displayed = formula.display_dynamic(&doc.variables);
    assert_eq!(displayed, "{Limit} < 5");
    assert_eq!(figcraft_doc::proto::Expr::parse_dynamic(&displayed, &doc.variables).as_ref(), Ok(formula));

    s.execute("variable.set", &json!({"variable": "Limit", "value": 5})).unwrap();
    assert!(!boolean(&s, "UnderLimit", "Mode 1"), "the renamed input continues to drive the formula");
}

#[test]
fn dependent_formulas_run_in_order_and_reject_cycles_or_unsupported_operators() {
    let mut s = Session::new();
    s.execute("variable.createCollection", &json!({"name": "Theme"})).unwrap();
    for (name, value) in [("A", true), ("B", false), ("C", false)] {
        s.execute("variable.create", &json!({"collection": "Theme", "name": name, "type": "boolean", "value": value})).unwrap();
    }
    s.execute("variable.create", &json!({"collection": "Theme", "name": "Count", "type": "number", "value": 0})).unwrap();
    s.execute("variable.setExpression", &json!({"variable": "B", "expression": "not {A}"})).unwrap();
    s.execute("variable.setExpression", &json!({"variable": "C", "expression": "{B} and true"})).unwrap();
    s.execute("variable.set", &json!({"variable": "A", "value": false})).unwrap();
    assert!(boolean(&s, "B", "Mode 1"));
    assert!(boolean(&s, "C", "Mode 1"), "the dependent formula sees B's newly computed value");

    assert!(s.execute("variable.setExpression", &json!({"variable": "A", "expression": "not {C}"})).is_err());
    assert!(s.execute("variable.setExpression", &json!({"variable": "C", "expression": "{A} + 1"})).is_err());
    assert!(s.execute("variable.setExpression", &json!({"variable": "Count", "expression": "{A}"})).is_err());
    assert!(s.doc().unwrap().variables.get("A").unwrap().expression.is_none());
    let encoded = serde_json::to_value(&s.doc().unwrap().variables).unwrap();
    let decoded: Variables = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded.get("B").unwrap().expression, s.doc().unwrap().variables.get("B").unwrap().expression);
}
