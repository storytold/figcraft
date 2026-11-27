//! Pattern paints: the layer each one tiles is linked once every layer is imported, wherever it
//! comes in the file. Fixtures are synthetic, written from the published schema.

use serde_json::json;

use super::*;
use figcraft_doc::color::{PatternAlign, PatternTile};

fn file(children: Value) -> Value {
    json!({"name": "Fixture", "schemaVersion": 0, "document": {"id": "0:0", "name": "Document", "type": "DOCUMENT",
        "children": [{"id": "0:1", "name": "Page", "type": "CANVAS", "children": children}]}})
}

fn layer(id: &str, name: &str, fills: Value) -> Value {
    json!({"id": id, "name": name, "type": "RECTANGLE", "relativeTransform": [[1, 0, 0], [0, 1, 0]], "size": {"x": 100, "y": 100}, "fills": fills})
}

fn node<'a>(doc: &'a Document, name: &str) -> &'a Node {
    doc.nodes().find(|n| n.name == name).unwrap_or_else(|| panic!("no layer {name}"))
}

fn pattern(n: &Node) -> &figcraft_doc::color::PatternPaint {
    match n.fills.first().map(|p| &p.kind) {
        Some(PaintKind::Pattern(p)) => p,
        other => panic!("{} has no pattern fill: {other:?}", n.name),
    }
}

#[test]
fn a_pattern_fill_tiles_its_source_layer_with_its_settings() {
    // The filled layer comes before the tile in the file: the link is made at the end.
    let fill = json!({"type": "PATTERN", "opacity": 0.07, "blendMode": "LIGHTEN", "sourceNodeId": "1:9",
        "tileType": "HORIZONTAL_HEXAGONAL", "scalingFactor": 0.5, "spacing": {"x": 4, "y": 2}, "horizontalAlignment": "CENTER"});
    let mut cta = layer("1:1", "CTA", json!([fill]));
    cta["strokes"] = json!([{"type": "PATTERN", "sourceNodeId": "1:9"}]);
    let v = file(json!([cta, layer("1:9", "Grid tile", json!([]))]));
    let (doc, warnings) = import(&v, &no_images).unwrap();
    assert!(!warnings.iter().any(|w| w.contains("PATTERN") || w.contains("pattern")), "{warnings:?}");
    let cta = node(&doc, "CTA");
    let p = pattern(cta);
    assert_eq!(NodeId(p.source), node(&doc, "Grid tile").id);
    assert_eq!((p.tile, p.align), (PatternTile::HorizontalHex, PatternAlign::Center));
    assert_eq!((p.scale, p.spacing), (0.5, (4.0, 2.0)));
    assert!(matches!(&cta.strokes[0].kind, PaintKind::Pattern(s) if s.source == p.source), "a pattern stroke too");
    let paint = &cta.fills[0];
    assert!((paint.opacity - 0.07).abs() < 1e-6 && paint.blend == figcraft_doc::color::BlendMode::Lighten);
}

#[test]
fn a_pattern_without_its_source_layer_is_skipped_with_a_note() {
    let bare = json!({"type": "PATTERN"});
    let gone = json!({"type": "PATTERN", "sourceNodeId": "7:7"});
    let v = file(json!([
        layer("1:1", "No source", json!([bare])),
        layer("1:2", "Gone", json!([gone, {"type": "SOLID", "color": {"r": 1, "g": 0, "b": 0, "a": 1}}]))
    ]));
    let (doc, warnings) = import(&v, &no_images).unwrap();
    assert!(node(&doc, "No source").fills.is_empty());
    let gone = node(&doc, "Gone");
    assert_eq!(gone.fills.len(), 1, "the solid fill stays");
    assert!(matches!(gone.fills[0].kind, PaintKind::Solid { .. }));
    assert!(warnings.iter().any(|w| w.contains("without a source layer")), "{warnings:?}");
    assert!(warnings.iter().any(|w| w.contains("tile a layer that wasn't imported")), "{warnings:?}");
}

#[test]
fn out_of_range_settings_are_clamped() {
    let fill = json!({"type": "PATTERN", "sourceNodeId": "1:9", "scalingFactor": 1e9, "spacing": {"x": -5, "y": 1e9}});
    let v = file(json!([layer("1:1", "Filled", json!([fill])), layer("1:9", "Tile", json!([]))]));
    let (doc, _) = import(&v, &no_images).unwrap();
    let p = pattern(node(&doc, "Filled"));
    assert_eq!(p.scale, figcraft_doc::color::PatternPaint::SCALE_RANGE.1);
    assert_eq!(p.spacing, (0.0, figcraft_doc::color::PatternPaint::MAX_SPACING));
}
