//! `design.audit` and `design.rules`: the checks and the report they write.

use figcraft_doc::NodeId;
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap()
}

fn id(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn rules(report: &Value) -> Vec<String> {
    report["findings"].as_array().unwrap().iter().filter_map(|f| f["rule"].as_str().map(str::to_string)).collect()
}

fn has(report: &Value, rule: &str) -> bool {
    rules(report).iter().any(|r| r == rule)
}

#[test]
fn reports_spacing_radius_copy_and_small_type() {
    let mut s = Session::new();
    let f = id(&ex(
        &mut s,
        "node.create",
        json!({"type": "frame", "x": 0, "y": 0, "width": 320, "height": 480,
               "props": {"layout": "vertical", "gap": 13, "padding": 11, "cornerRadius": 7}}),
    ));
    ex(&mut s, "text.create", json!({"text": "Welcome back, Sam!", "x": 0, "y": 0, "parent": f, "props": {"fontSize": 10}}));
    let r = ex(&mut s, "design.audit", json!({"scope": "page"}));
    for rule in ["spacing.off-grid", "radius.off-scale", "copy.exclamation", "type.too-small"] {
        assert!(has(&r, rule), "{rule} missing from {r}");
    }
    let spacing = r["findings"].as_array().unwrap().iter().find(|f| f["rule"] == "spacing.off-grid").unwrap();
    assert_eq!(spacing["suggestion"]["gap"], 12.0, "{r}");
    assert_eq!(spacing["suggestion"]["padding"], json!([12.0, 12.0, 12.0, 12.0]), "{r}");
    assert_eq!(spacing["fixable"], true);
}

#[test]
fn reports_gradient_text_glow_and_low_contrast() {
    let mut s = Session::new();
    let t = id(&ex(&mut s, "text.create", json!({"text": "Ship it", "x": 0, "y": 0})));
    ex(&mut s, "node.set", json!({"id": t, "fills": [{"type": "linearGradient", "stops": ["#ff0000", "#0000ff"]}]}));
    let box_id = id(&ex(&mut s, "node.create", json!({"type": "frame", "x": 0, "y": 200, "width": 200, "height": 100})));
    ex(
        &mut s,
        "node.set",
        json!({"id": box_id, "props": {"effects": [{"type": "dropShadow", "color": "#00000040", "offset": [0, 0], "radius": 24}]}}),
    );
    let white = id(&ex(&mut s, "text.create", json!({"text": "Invisible", "x": 0, "y": 400})));
    ex(&mut s, "node.set", json!({"id": white, "props": {"fill": "#FFFFFF"}}));
    let r = ex(&mut s, "design.audit", json!({"scope": "page"}));
    for rule in ["color.gradient-text", "color.glow", "color.contrast"] {
        assert!(has(&r, rule), "{rule} missing from {r}");
    }
}

#[test]
fn reports_a_gradient_on_a_shape() {
    let mut s = Session::new();
    let rect = id(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 50})));
    ex(&mut s, "node.set", json!({"id": rect, "fills": [{"type": "linearGradient", "stops": ["#ff0000", "#0000ff"]}]}));
    let r = ex(&mut s, "design.audit", json!({"scope": "page"}));
    assert!(has(&r, "color.gradient"), "{r}");
    assert!(!has(&r, "color.gradient-text"), "{r}");
}

#[test]
fn counts_type_sizes_and_families_across_the_scope() {
    let mut s = Session::new();
    for size in [12, 13, 14, 15, 16, 17, 18, 19] {
        ex(&mut s, "text.create", json!({"text": "Text", "x": 0, "y": size as f64 * 20.0, "props": {"fontSize": size}}));
    }
    let r = ex(&mut s, "design.audit", json!({"scope": "page"}));
    assert!(has(&r, "type.sizes"), "{r}");
    let f = r["findings"].as_array().unwrap().iter().find(|f| f["rule"] == "type.sizes").unwrap();
    assert!(f["message"].as_str().unwrap().contains("8 font sizes"), "{r}");
}

#[test]
fn selection_scope_ignores_other_layers() {
    let mut s = Session::new();
    let bad = id(&ex(
        &mut s,
        "node.create",
        json!({"type": "frame", "x": 0, "y": 0, "width": 200, "height": 100, "props": {"layout": "vertical", "gap": 13}}),
    ));
    let good = id(&ex(
        &mut s,
        "node.create",
        json!({"type": "frame", "x": 300, "y": 0, "width": 200, "height": 100, "props": {"layout": "vertical", "gap": 8}}),
    ));
    s.set_selection(vec![good]);
    let r = ex(&mut s, "design.audit", json!({"scope": "selection"}));
    assert!(!has(&r, "spacing.off-grid"), "{r}");
    s.set_selection(vec![bad]);
    let r = ex(&mut s, "design.audit", json!({"scope": "selection"}));
    assert!(has(&r, "spacing.off-grid"), "{r}");
}

#[test]
fn a_nested_selection_is_audited_once() {
    let mut s = Session::new();
    let f = id(&ex(&mut s, "node.create", json!({"type": "frame", "x": 0, "y": 0, "width": 200, "height": 100})));
    let child = id(&ex(&mut s, "node.create", json!({"type": "rectangle", "parent": f, "x": 0, "y": 0, "width": 20, "height": 20})));
    s.set_selection(vec![f, child]);
    let r = ex(&mut s, "design.audit", json!({"scope": "selection"}));
    // The rectangle is only in the report through the frame's subtree.
    let rect_findings = r["findings"].as_array().unwrap().iter().filter(|f| f["path"].as_str().unwrap().contains("Rectangle")).count();
    assert!(rect_findings <= 1, "{r}");
}

#[test]
fn filters_by_rule_and_severity() {
    let mut s = Session::new();
    ex(&mut s, "node.create", json!({"type": "frame", "x": 0, "y": 0, "width": 200, "height": 100, "props": {"layout": "vertical", "gap": 13}}));
    let only = ex(&mut s, "design.audit", json!({"scope": "page", "rules": ["spacing.off-grid"]}));
    assert_eq!(rules(&only), vec!["spacing.off-grid".to_string()], "{only}");
    let high = ex(&mut s, "design.audit", json!({"scope": "page", "minSeverity": "high"}));
    assert!(rules(&high).is_empty(), "{high}");
    assert_eq!(high["score"], 100.0, "{high}");
    let all = ex(&mut s, "design.audit", json!({"scope": "page"}));
    assert!(all["score"].as_f64().unwrap() < 100.0, "{all}");
}

#[test]
fn reports_emoji_and_default_names() {
    let mut s = Session::new();
    let f = id(&ex(&mut s, "node.create", json!({"type": "frame", "x": 0, "y": 0, "width": 200, "height": 100})));
    ex(&mut s, "text.create", json!({"text": "Ship it 🚀", "x": 0, "y": 0, "parent": f, "props": {"fontSize": 16}}));
    let r = ex(&mut s, "design.audit", json!({"scope": "page"}));
    assert!(has(&r, "icon.emoji"), "{r}");
    assert!(has(&r, "naming.auto"), "{r}");
}

#[test]
fn lists_the_rules() {
    let mut s = Session::new();
    let r = ex(&mut s, "design.rules", json!({}));
    let rules = r["rules"].as_array().unwrap();
    assert!(rules.len() >= 15, "{r}");
    let spacing = rules.iter().find(|r| r["id"] == "spacing.off-grid").unwrap();
    assert_eq!(spacing["severity"], "medium");
    assert_eq!(spacing["fixable"], true);
}

#[test]
fn junk_params_are_errors_not_panics() {
    let mut s = Session::new();
    for junk in [
        json!({"scope": {"x": 1}}),
        json!({"scope": "everything"}),
        json!({"rules": [1]}),
        json!({"rules": "spacing.off-grid"}),
        json!({"rules": ["no.such.rule"]}),
        json!({"minSeverity": "loud"}),
    ] {
        assert!(s.execute("design.audit", &junk).is_err(), "{junk} was accepted");
    }
    assert!(s.execute("design.audit", &json!({})).is_ok());
    // Nulls mean "the default", not junk.
    assert!(s.execute("design.audit", &json!({"scope": "document", "rules": null, "minSeverity": null})).is_ok());
}
