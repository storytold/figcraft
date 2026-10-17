//! Stroke-only vectors that import as the outline of their stroke keep the stroke's style and variables.

use super::proto_variable_tests::{BRAND, alias, variables_json};
use super::*;
use serde_json::json;

fn at(x: f64, y: f64) -> Value {
    json!([[1, 0, x], [0, 1, y]])
}

fn file(children: Value) -> Value {
    json!({"name": "Fixture", "schemaVersion": 0, "document": {"id": "0:0", "name": "Document", "type": "DOCUMENT", "children": [
        {"id": "0:1", "name": "Page", "type": "CANVAS", "children": children}
    ]}})
}

fn named<'a>(doc: &'a Document, name: &str) -> &'a Node {
    doc.nodes().find(|n| n.name == name).unwrap_or_else(|| panic!("no node {name}"))
}

/// A stroke-only icon vector as `geometry=paths` returns it: no fills, one visible stroke.
fn stroke_only_vector(id: &str, name: &str, stroke: Value) -> Value {
    json!({"id": id, "name": name, "type": "VECTOR", "relativeTransform": at(0.0, 0.0), "size": {"x": 10, "y": 10},
           "fills": [], "strokes": [stroke], "strokeWeight": 2,
           "fillGeometry": [],
           "strokeGeometry": [{"path": "M 0 0 L 10 0 L 10 2 L 0 2 Z", "windingRule": "NONZERO"}],
           "styles": {"stroke": "S:stroke"}})
}

#[test]
fn stroke_only_vector_shared_stroke_style_keeps_paints() {
    let stroke = json!({"type": "SOLID", "color": {"r": 1, "g": 0, "b": 0, "a": 1}});
    let mut v = file(json!([stroke_only_vector("1:1", "Icon", stroke)]));
    v["styles"] = json!({"S:stroke": {"key": "k", "name": "Brand/Stroke", "styleType": "FILL"}});
    let (doc, _) = import(&v, &no_images).unwrap();
    let icon = named(&doc, "Icon");
    assert!(matches!(icon.kind, NodeKind::Vector { .. }));
    let style = doc.style("S:stroke").expect("shared stroke style is created");
    match &style.kind {
        StyleKind::Paint { paints } => assert_eq!(paints.len(), 1, "shared stroke style must keep the stroke paint, got {paints:?}"),
        k => panic!("unexpected style kind {k:?}"),
    }
}

#[test]
fn stroke_colour_variable_on_outlined_vector_is_kept() {
    let stroke = json!({"type": "SOLID", "color": {"r": 1, "g": 0, "b": 0, "a": 1}, "boundVariables": {"color": alias(BRAND)}});
    let mut node = stroke_only_vector("1:1", "Icon", stroke);
    node.as_object_mut().unwrap().remove("styles");
    node["boundVariables"] = json!({"strokes": [alias(BRAND)]});
    let (doc, warnings) = import_with(&file(json!([node])), Some(&variables_json()), &no_images).unwrap();
    let icon = named(&doc, "Icon");
    // The stroke paint now paints the outline as its fill: its colour token must follow it.
    assert!(
        icon.bound.values().any(|v| v == BRAND),
        "colour variable bound on the outlined stroke was dropped; bound={:?} warnings={warnings:?}",
        icon.bound
    );
}
