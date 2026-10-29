//! Sketch shared styles → FigCraft styles; Color Variables (swatches) → colour variables.

use figcraft_doc::color::PaintKind;
use figcraft_doc::{Document, Node, StyleKind};
use serde_json::{Value, json};

use super::fixture::*;
use super::import;

const BLACK: [f64; 4] = [0.0, 0.0, 0.0, 1.0];

fn open(doc_json: Value, layers: Vec<Value>) -> (Document, Vec<String>) {
    let z = sketch_zip("83.2", 139, doc_json, &[page("P", "Page", vec![artboard("A", "Board", 0.0, 0.0, 400.0, 400.0, layers)])], &[]);
    import(&z, "f").unwrap()
}

fn node<'d>(doc: &'d Document, name: &str) -> &'d Node {
    doc.nodes().find(|n| n.name == name).unwrap_or_else(|| panic!("no {name}"))
}

fn style<'d>(doc: &'d Document, id: &str) -> &'d figcraft_doc::Style {
    doc.styles.iter().find(|s| s.id == id).unwrap_or_else(|| panic!("no style {id}: {:?}", doc.styles.iter().map(|s| &s.id).collect::<Vec<_>>()))
}

fn card_style() -> Value {
    json!({"_class": "style", "fills": [solid_fill(0.0, 0.0, 1.0, 1.0)], "borders": [],
           "shadows": [{"_class": "shadow", "isEnabled": true, "blurRadius": 4, "offsetX": 0, "offsetY": 2, "spread": 0, "color": color(0.0, 0.0, 0.0, 0.3)}],
           "innerShadows": []})
}

fn with_shared(d: &mut Value, layer_styles: Vec<Value>, text_styles: Vec<Value>) {
    d["layerStyles"]["objects"] = Value::Array(layer_styles);
    d["layerTextStyles"]["objects"] = Value::Array(text_styles);
}

fn shared(id: &str, name: &str, value: Value) -> Value {
    json!({"_class": "sharedStyle", "do_objectID": id, "name": name, "value": value})
}

fn linked(mut l: Value, style: Value, shared_id: &str) -> Value {
    l["style"] = style;
    l["style"]["sharedObjectID"] = json!(shared_id);
    l
}

#[test]
fn layer_style_splits() {
    let mut d = document(&["P"]);
    with_shared(&mut d, vec![shared("CARD", "Card", card_style())], vec![]);
    let (doc, _) = open(d, vec![linked(rect("R", 0.0, 0.0, 50.0, 50.0), card_style(), "CARD")]);
    let n = node(&doc, "R");
    let fill = n.styles.fill.clone().expect("fill style linked");
    let effect = n.styles.effect.clone().expect("effect style linked");
    assert_eq!(style(&doc, &fill).name, "Card");
    assert!(matches!(style(&doc, &fill).kind, StyleKind::Paint { .. }));
    assert!(matches!(style(&doc, &effect).kind, StyleKind::Effect { .. }));
    assert_eq!(style(&doc, &effect).name, "Card");
}

#[test]
fn layer_style_borders_become_a_stroke_style() {
    let mut st = card_style();
    st["borders"] =
        json!([{"_class": "border", "isEnabled": true, "fillType": 0, "color": color(1.0, 0.0, 0.0, 1.0), "position": 1, "thickness": 2}]);
    let mut d = document(&["P"]);
    with_shared(&mut d, vec![shared("CARD", "Card", st.clone())], vec![]);
    let (doc, _) = open(d, vec![linked(rect("R", 0.0, 0.0, 50.0, 50.0), st, "CARD")]);
    let stroke = node(&doc, "R").styles.stroke.clone().expect("stroke style linked");
    assert!(style(&doc, &stroke).name.starts_with("Card"));
}

#[test]
fn unused_shared_styles_are_listed() {
    let mut d = document(&["P"]);
    with_shared(&mut d, vec![shared("CARD", "Card", card_style())], vec![]);
    let (doc, _) = open(d, vec![]);
    assert!(doc.styles.iter().any(|s| s.name == "Card"));
}

#[test]
fn text_style_links() {
    let enc = json!({"MSAttributedStringFontAttribute": {"_class": "fontDescriptor", "attributes": {"name": "Helvetica-Bold", "size": 32}},
                     "MSAttributedStringColorAttribute": color(1.0, 0.0, 0.0, 1.0),
                     "paragraphStyle": {"_class": "paragraphStyle", "alignment": 0, "maximumLineHeight": 40, "minimumLineHeight": 40}});
    let st = json!({"_class": "style", "textStyle": {"_class": "textStyle", "encodedAttributes": enc, "verticalAlignment": 0}});
    let mut d = document(&["P"]);
    with_shared(&mut d, vec![], vec![shared("H1", "Heading/H1", st.clone())]);
    let t = linked(text("Title", 0.0, 0.0, 200.0, 40.0, "Hello", &[(0, 5, "Helvetica-Bold", 32.0, [1.0, 0.0, 0.0, 1.0])]), st, "H1");
    let (doc, _) = open(d, vec![t]);
    let n = node(&doc, "Title");
    let sid = n.styles.text.clone().expect("text style linked");
    let s = style(&doc, &sid);
    assert_eq!(s.name, "Heading/H1");
    let StyleKind::Text { style: ts } = &s.kind else { panic!("not a text style") };
    assert_eq!((ts.font_family.as_str(), ts.font_style.as_str(), ts.font_size), ("Helvetica", "Bold", 32.0));
    assert!(n.fills.iter().any(|p| matches!(p.kind, PaintKind::Solid { color } if color.r == 1.0 && color.g == 0.0)));
}

#[test]
fn locally_changed_style_keeps_link() {
    let mut local = card_style();
    local["fills"] = json!([solid_fill(0.0, 1.0, 0.0, 1.0)]);
    let mut d = document(&["P"]);
    with_shared(&mut d, vec![shared("CARD", "Card", card_style())], vec![]);
    let (doc, _) = open(d, vec![linked(rect("R", 0.0, 0.0, 50.0, 50.0), local, "CARD")]);
    let n = node(&doc, "R");
    assert!(n.styles.fill.is_some());
    assert!(matches!(n.fills[0].kind, PaintKind::Solid { color } if color.g == 1.0 && color.b == 0.0));
}

#[test]
fn foreign_styles_too() {
    let mut d = document(&["P"]);
    d["foreignLayerStyles"] = json!([{"_class": "MSImmutableForeignLayerStyle", "sourceLibraryName": "Kit", "localSharedStyle": shared("FCARD", "Kit card", card_style())}]);
    let (doc, _) = open(d, vec![linked(rect("R", 0.0, 0.0, 50.0, 50.0), card_style(), "FCARD")]);
    assert_eq!(style(&doc, node(&doc, "R").styles.fill.as_deref().unwrap()).name, "Kit card");
}

#[test]
fn swatches_are_color_variables() {
    let mut d = document(&["P"]);
    d["sharedSwatches"] = json!({"_class": "swatchContainer", "objects": [{"_class": "swatch", "do_objectID": "SW1", "name": "Brand/Blue", "value": color(0.06, 0.38, 1.0, 1.0)}]});
    d["foreignSwatches"] = json!([{"_class": "MSImmutableForeignSwatch", "sourceLibraryName": "Kit", "localSwatch": {"_class": "swatch", "do_objectID": "SW2", "name": "Kit/Red", "value": color(1.0, 0.0, 0.0, 1.0)}}]);
    let mut fill = solid_fill(0.06, 0.38, 1.0, 1.0);
    fill["color"]["swatchID"] = json!("SW1");
    let mut fill2 = solid_fill(1.0, 0.0, 0.0, 1.0);
    fill2["color"]["swatchID"] = json!("SW2");
    let (doc, _) = open(
        d,
        vec![styled(rect("R", 0.0, 0.0, 50.0, 50.0), "fills", json!([fill])), styled(rect("S", 0.0, 0.0, 50.0, 50.0), "fills", json!([fill2]))],
    );
    let coll = doc.variables.collections.iter().find(|c| c.name == "Color Variables").expect("collection");
    let var = doc.variables.variables.iter().find(|v| v.name == "Brand/Blue").expect("variable");
    assert_eq!(var.collection, coll.id);
    assert!(doc.variables.variables.iter().any(|v| v.name == "Kit/Red"));
    assert_eq!(node(&doc, "R").bound.get("fill"), Some(&var.id));
    assert!(node(&doc, "S").bound.contains_key("fill"));
}

#[test]
fn unknown_shared_ids_are_ignored() {
    let (doc, _) = open(document(&["P"]), vec![linked(rect("R", 0.0, 0.0, 50.0, 50.0), card_style(), "NOPE")]);
    let n = node(&doc, "R");
    assert!(n.styles.fill.is_none());
    let mut f = solid_fill(0.0, 0.0, 0.0, 1.0);
    f["color"]["swatchID"] = json!("NOPE");
    let (doc, _) =
        open(document(&["P"]), vec![styled(text("T", 0.0, 0.0, 10.0, 10.0, "x", &[(0, 1, "Helvetica", 9.0, BLACK)]), "fills", json!([f]))]);
    assert!(node(&doc, "T").bound.is_empty());
}

/// A bitmap's fills hold its own image under the style's fills, so they no longer match a shared
/// layer style: the fill isn't linked (its border and effects still are).
#[test]
fn bitmaps_keep_their_image_out_of_shared_fills() {
    let png = {
        let mut out = Vec::new();
        image::RgbaImage::new(2, 2).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    };
    let mut d = document(&["P"]);
    with_shared(&mut d, vec![shared("CARD", "Card", card_style())], vec![]);
    let mut bitmap = linked(rect("Photo", 0.0, 0.0, 50.0, 50.0), card_style(), "CARD");
    bitmap["_class"] = json!("bitmap");
    bitmap["image"] = json!({"_class": "MSJSONFileReference", "_ref": "images/a.png"});
    let z = sketch_zip(
        "83.2",
        139,
        d,
        &[page("P", "Page", vec![artboard("A", "Board", 0.0, 0.0, 400.0, 400.0, vec![bitmap])])],
        &[("images/a.png", &png)],
    );
    let (doc, _) = import(&z, "f").unwrap();
    let n = node(&doc, "Photo");
    assert!(matches!(n.fills[0].kind, PaintKind::Image(_)));
    assert!(n.styles.fill.is_none(), "{:?}", n.styles.fill);
    assert!(n.styles.effect.is_some());
}
