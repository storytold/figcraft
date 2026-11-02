//! `design.audit`, `design.fix` and `design.rules`: the checks, the repairs and the report.

use figcraft_doc::{EffectKind, NodeId, NodeKind};
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

fn rule_nodes(report: &Value, rule: &str) -> Vec<u64> {
    report["findings"].as_array().unwrap().iter().filter(|f| f["rule"] == rule).filter_map(|f| f["node"].as_u64()).collect()
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
    assert!(all["nodes"].as_u64().unwrap() >= 2, "the report counts the layers it walked: {all}");
}

#[test]
fn a_light_border_is_a_hairline() {
    let mut s = Session::new();
    let light = id(&ex(
        &mut s,
        "node.create",
        json!({
            "type": "frame", "x": 0, "y": 0, "width": 200, "height": 60,
            "props": {"stroke": "#E0E0E0", "strokeWeight": 1}
        }),
    ));
    let strong = id(&ex(
        &mut s,
        "node.create",
        json!({
            "type": "frame", "x": 0, "y": 100, "width": 200, "height": 60,
            "props": {"stroke": "#333333", "strokeWeight": 1}
        }),
    ));
    // An inside stroke on a dark panel sits on the panel, not on the page behind it.
    let dark_panel = id(&ex(
        &mut s,
        "node.create",
        json!({
            "type": "frame", "x": 0, "y": 200, "width": 200, "height": 60,
            "props": {"fill": "#1E1E1E", "stroke": "#2A2A2A", "strokeWeight": 1, "strokeAlign": "inside"}
        }),
    ));
    let r = ex(&mut s, "design.audit", json!({"scope": "page", "rules": ["color.border-opacity"]}));
    let flagged: Vec<u64> = r["findings"].as_array().unwrap().iter().filter_map(|f| f["node"].as_u64()).collect();
    assert!(!flagged.contains(&light.0), "{r}");
    assert!(flagged.contains(&strong.0), "{r}");
    assert!(!flagged.contains(&dark_panel.0), "a dark border on a dark panel is not a strong hairline: {r}");
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

fn spaced_frame(s: &mut Session) -> NodeId {
    id(&ex(
        s,
        "node.create",
        json!({"type": "frame", "x": 0, "y": 0, "width": 200, "height": 100,
               "props": {"layout": "vertical", "gap": 13, "padding": 11, "cornerRadius": 7}}),
    ))
}

#[test]
fn fix_snaps_spacing_and_radius_as_one_undo_step() {
    let mut s = Session::new();
    let f = spaced_frame(&mut s);
    let r = ex(&mut s, "design.fix", json!({"scope": "page"}));
    assert_eq!(r["dryRun"], false, "{r}");
    assert!(r["changed"].as_u64().unwrap() >= 1, "{r}");
    assert!(r["byRule"]["spacing.off-grid"].as_u64().unwrap() >= 1, "{r}");
    assert!(r["byRule"]["radius.off-scale"].as_u64().unwrap() >= 1, "{r}");
    {
        let n = s.doc().unwrap().node(f).unwrap();
        let al = n.layout.as_ref().unwrap();
        assert_eq!((al.gap, al.padding[0], al.padding[1]), (12.0, 12.0, 12.0));
        assert_eq!(n.corners.uniform(), Some(8.0));
    }
    let after = ex(&mut s, "design.audit", json!({"scope": "page"}));
    assert!(!has(&after, "spacing.off-grid"), "{after}");
    assert!(!has(&after, "radius.off-scale"), "{after}");
    // The whole repair is one undo step.
    ex(&mut s, "edit.undo", json!({}));
    let n = s.doc().unwrap().node(f).unwrap();
    assert_eq!(n.layout.as_ref().unwrap().gap, 13.0);
    assert_eq!(n.corners.uniform(), Some(7.0));
}

#[test]
fn dry_run_reports_without_changing_anything() {
    let mut s = Session::new();
    let f = spaced_frame(&mut s);
    let r = ex(&mut s, "design.fix", json!({"scope": "page", "dryRun": true}));
    assert_eq!(r["dryRun"], true, "{r}");
    assert!(r["changed"].as_u64().unwrap() >= 1, "{r}");
    assert_eq!(s.doc().unwrap().node(f).unwrap().layout.as_ref().unwrap().gap, 13.0);
}

#[test]
fn fix_filters_by_rule() {
    let mut s = Session::new();
    let f = spaced_frame(&mut s);
    ex(&mut s, "design.fix", json!({"scope": "page", "rules": ["radius.off-scale"]}));
    let n = s.doc().unwrap().node(f).unwrap();
    assert_eq!(n.layout.as_ref().unwrap().gap, 13.0, "spacing was not asked for");
    assert_eq!(n.corners.uniform(), Some(8.0));
}

#[test]
fn fix_caps_heading_weight_and_removes_tracking() {
    let mut s = Session::new();
    let heading = id(&ex(&mut s, "text.create", json!({"text": "Title", "x": 0, "y": 0, "props": {"fontSize": 28, "fontStyle": "Bold"}})));
    let small = id(&ex(
        &mut s,
        "text.create",
        json!({"text": "Label", "x": 0, "y": 60,
               "props": {"fontSize": 16, "letterSpacing": {"unit": "pixels", "value": -1.5}}}),
    ));
    let r = ex(&mut s, "design.fix", json!({"scope": "page"}));
    assert!(r["changed"].as_u64().unwrap() >= 2, "{r}");
    let doc = s.doc().unwrap();
    let NodeKind::Text(h) = &doc.node(heading).unwrap().kind else { panic!() };
    assert_eq!(h.style.weight(), 600);
    let NodeKind::Text(l) = &doc.node(small).unwrap().kind else { panic!() };
    assert_eq!(l.style.letter_spacing.resolve(16.0), 0.0);
    let after = ex(&mut s, "design.audit", json!({"scope": "page"}));
    assert!(!has(&after, "type.weight"), "{after}");
    assert!(!has(&after, "type.negative-tracking"), "{after}");
}

#[test]
fn fix_removes_glows_and_neutralises_coloured_shadows() {
    let mut s = Session::new();
    let f = id(&ex(&mut s, "node.create", json!({"type": "frame", "x": 0, "y": 0, "width": 200, "height": 100})));
    ex(
        &mut s,
        "node.set",
        json!({"id": f, "props": {"effects": [
            {"type": "dropShadow", "color": "#00000040", "offset": [0, 0], "radius": 24},
            {"type": "dropShadow", "color": "#7048E880", "offset": [0, 4], "radius": 12}
        ]}}),
    );
    let r = ex(&mut s, "design.fix", json!({"scope": "page"}));
    assert!(r["changed"].as_u64().unwrap() >= 1, "{r}");
    let effects = &s.doc().unwrap().node(f).unwrap().effects;
    assert_eq!(effects.len(), 1, "the glow is gone: {effects:?}");
    let EffectKind::DropShadow(shadow) = &effects[0].kind else { panic!() };
    assert_eq!((shadow.color.r, shadow.color.g, shadow.color.b), (0.0, 0.0, 0.0));
    let after = ex(&mut s, "design.audit", json!({"scope": "page"}));
    assert!(!has(&after, "color.glow"), "{after}");
    assert!(!has(&after, "color.colored-shadow"), "{after}");
}

#[test]
fn fix_leaves_content_and_gradients_alone() {
    let mut s = Session::new();
    let t = id(&ex(&mut s, "text.create", json!({"text": "Ship it 🚀!", "x": 0, "y": 0, "props": {"fontSize": 16}})));
    ex(&mut s, "node.set", json!({"id": t, "fills": [{"type": "linearGradient", "stops": ["#ff0000", "#0000ff"]}]}));
    ex(&mut s, "design.fix", json!({"scope": "page"}));
    let doc = s.doc().unwrap();
    let NodeKind::Text(td) = &doc.node(t).unwrap().kind else { panic!() };
    assert_eq!(td.characters, "Ship it 🚀!");
    assert!(matches!(&td.style.fills.as_deref().unwrap_or(&doc.node(t).unwrap().fills)[0].kind, figcraft_color::PaintKind::Gradient(_)));
}

#[test]
fn fix_darkens_low_contrast_text() {
    let mut s = Session::new();
    let f = id(&ex(&mut s, "node.create", json!({"type": "frame", "x": 0, "y": 0, "width": 200, "height": 100})));
    let t = id(&ex(&mut s, "text.create", json!({"text": "Invisible", "x": 0, "y": 0, "parent": f, "props": {"fontSize": 16, "fill": "#FFFFFF"}})));
    assert!(has(&ex(&mut s, "design.audit", json!({"scope": "page"})), "color.contrast"));
    let r = ex(&mut s, "design.fix", json!({"scope": "page", "rules": ["color.contrast"]}));
    assert!(r["changed"].as_u64().unwrap() >= 1, "{r}");
    {
        let doc = s.doc().unwrap();
        let n = doc.node(t).unwrap();
        let NodeKind::Text(td) = &n.kind else { panic!() };
        let paints = td.style.fills.as_deref().unwrap_or(&n.fills);
        let color = paints.iter().rev().find_map(|p| p.solid_color()).unwrap();
        assert!(color.contrast(figcraft_color::Color::WHITE) >= 4.5, "{color:?}");
    }
    assert!(!has(&ex(&mut s, "design.audit", json!({"scope": "page"})), "color.contrast"));
}

#[test]
fn reports_text_that_overflows_its_box() {
    let mut s = Session::new();
    let t = id(&ex(
        &mut s,
        "node.create",
        json!({"type": "text", "text": "A long headline that needs several lines in a narrow box",
               "x": 0, "y": 0, "width": 120, "height": 40,
               "props": {"fontSize": 24, "textAutoResize": "none"}}),
    ));
    let r = ex(&mut s, "design.audit", json!({"scope": "page"}));
    let f = r["findings"].as_array().unwrap().iter().find(|f| f["rule"] == "layout.text-overflow").unwrap_or_else(|| panic!("{r}"));
    assert_eq!(f["node"], t.0, "{r}");
}

#[test]
fn a_sibling_background_is_the_text_backdrop() {
    let mut s = Session::new();
    let f = id(&ex(&mut s, "node.create", json!({"type": "frame", "x": 0, "y": 0, "width": 300, "height": 200, "props": {"fill": "#F4F1EA"}})));
    ex(&mut s, "node.create", json!({"type": "rectangle", "parent": f, "x": 0, "y": 0, "width": 120, "height": 200, "props": {"fill": "#1E1E1E"}}));
    let white =
        id(&ex(&mut s, "text.create", json!({"text": "White on dark", "x": 20, "y": 20, "parent": f, "props": {"fontSize": 16, "fill": "#FFFFFF"}})));
    let dark =
        id(&ex(&mut s, "text.create", json!({"text": "Dark on dark", "x": 20, "y": 80, "parent": f, "props": {"fontSize": 16, "fill": "#111111"}})));
    let r = ex(&mut s, "design.audit", json!({"scope": "page", "rules": ["color.contrast"]}));
    let flagged = rule_nodes(&r, "color.contrast");
    assert!(!flagged.contains(&white.0), "white on the dark sibling is legible: {r}");
    assert!(flagged.contains(&dark.0), "dark on the dark sibling is not: {r}");
}

#[test]
fn fix_uses_the_sibling_background_for_contrast() {
    let mut s = Session::new();
    let f = id(&ex(&mut s, "node.create", json!({"type": "frame", "x": 0, "y": 0, "width": 300, "height": 200, "props": {"fill": "#F4F1EA"}})));
    ex(&mut s, "node.create", json!({"type": "rectangle", "parent": f, "x": 0, "y": 0, "width": 120, "height": 200, "props": {"fill": "#1E1E1E"}}));
    let white =
        id(&ex(&mut s, "text.create", json!({"text": "White on dark", "x": 20, "y": 20, "parent": f, "props": {"fontSize": 16, "fill": "#FFFFFF"}})));
    let dark =
        id(&ex(&mut s, "text.create", json!({"text": "Dark on dark", "x": 20, "y": 80, "parent": f, "props": {"fontSize": 16, "fill": "#111111"}})));
    ex(&mut s, "design.fix", json!({"scope": "page", "rules": ["color.contrast"]}));
    let doc = s.doc().unwrap();
    let fill = |id| {
        let n = doc.node(id).unwrap();
        let NodeKind::Text(t) = &n.kind else { panic!() };
        t.style.fills.as_deref().unwrap_or(&n.fills).iter().rev().find_map(|p| p.solid_color()).unwrap()
    };
    let w = fill(white);
    assert_eq!((w.r, w.g, w.b), (1.0, 1.0, 1.0), "the legible white text is left alone");
    let d = fill(dark);
    assert!(d.contrast(figcraft_color::Color::from_rgb8_const(0x1E, 0x1E, 0x1E)) >= 4.5, "the dark text is lightened: {d:?}");
}

#[test]
fn fix_junk_is_an_error_not_a_panic() {
    let mut s = Session::new();
    assert!(s.execute("design.fix", &json!({"scope": "everything"})).is_err());
    assert!(s.execute("design.fix", &json!({"rules": ["no.such.rule"]})).is_err());
    assert!(s.execute("design.fix", &json!({})).is_ok());
}
