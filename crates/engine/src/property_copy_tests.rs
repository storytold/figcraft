//! Copying one fill, stroke or effect row (`fill.copy`, `stroke.copy`, `effect.copy`) and pasting
//! it onto other layers (`fill.paste`, `stroke.paste`, `effect.paste`): the Design panel's row
//! menus.

use figcraft_color::{Color, Paint};
use figcraft_doc::{Effect, EffectKind, NodeId};
use serde_json::{Value, json};

use crate::Session;

fn id(v: Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn rect(s: &mut Session) -> NodeId {
    id(ex(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})))
}

fn node(s: &Session, id: NodeId) -> figcraft_doc::Node {
    s.doc().unwrap().node(id).unwrap().clone()
}

#[test]
fn a_copied_fill_replaces_a_fill_row_or_adds_one() {
    let mut s = Session::new();
    let a = rect(&mut s);
    let b = rect(&mut s);
    let red = Paint::solid(Color::from_hex("#FF0000").unwrap());
    ex(&mut s, "fill.add", json!({"ids": [a], "paint": red}));
    let copied = ex(&mut s, "fill.copy", json!({"id": a, "index": 1}));
    assert_eq!(copied["type"], "solid", "the copy reports the paint it took");
    // Onto b's fill row: it replaces that paint.
    ex(&mut s, "fill.paste", json!({"ids": [b], "index": 0}));
    assert_eq!(node(&s, b).fills, vec![red.clone()]);
    // With no row: it adds one on top.
    ex(&mut s, "fill.paste", json!({"ids": [b]}));
    assert_eq!(node(&s, b).fills, [red.clone(), red]);
    // Each paste is one undo step.
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(node(&s, b).fills.len(), 1);
}

#[test]
fn a_copied_stroke_pastes_as_a_stroke_only() {
    let mut s = Session::new();
    let a = rect(&mut s);
    let b = rect(&mut s);
    let blue = Paint::solid(Color::from_hex("#0000FF").unwrap());
    ex(&mut s, "stroke.add", json!({"ids": [a], "paint": blue}));
    ex(&mut s, "node.set", json!({"ids": [a], "props": {"strokeWeight": 3.0, "strokeAlign": "outside"}}));
    ex(&mut s, "stroke.copy", json!({"id": a, "index": 0}));
    ex(&mut s, "stroke.add", json!({"ids": [b]}));
    ex(&mut s, "stroke.paste", json!({"ids": [b], "index": 0}));
    assert_eq!(node(&s, b).strokes, [blue]);
    assert_eq!(node(&s, b).stroke, node(&s, a).stroke, "the weight and alignment come with the paint");
    assert_eq!((node(&s, b).stroke.weight, node(&s, b).stroke.align), (3.0, figcraft_doc::StrokeAlign::Outside));
    // A copied fill doesn't paste as a stroke, and the target keeps its stroke.
    ex(&mut s, "fill.copy", json!({"id": a}));
    assert!(s.execute("stroke.paste", &json!({"ids": [b]})).is_err());
    assert_eq!(node(&s, b).strokes.len(), 1);
    // A layer with no stroke has nothing to copy.
    assert!(s.execute("stroke.copy", &json!({"id": b, "index": 1})).is_err());
}

#[test]
fn a_copied_effect_replaces_an_effect_row_or_adds_one() {
    let mut s = Session::new();
    let a = rect(&mut s);
    let b = rect(&mut s);
    ex(&mut s, "effect.add", json!({"ids": [a], "effect": {"type": "dropShadow", "radius": 7, "offset": [2, 3]}}));
    ex(&mut s, "effect.copy", json!({"id": a, "index": 0}));
    let e = node(&s, a).effects[0].clone();
    // Onto b's effect row: it replaces that effect.
    ex(&mut s, "effect.add", json!({"ids": [b], "type": "layerBlur"}));
    ex(&mut s, "effect.paste", json!({"ids": [b], "index": 0}));
    assert_eq!(node(&s, b).effects, vec![e.clone()]);
    // With no row: it adds one on top.
    ex(&mut s, "effect.paste", json!({"ids": [b]}));
    assert_eq!(node(&s, b).effects, [e.clone(), e]);
    ex(&mut s, "edit.undo", json!({}));
    assert_eq!(node(&s, b).effects.len(), 1);
    let Effect { kind: EffectKind::DropShadow(shadow), visible } = node(&s, b).effects[0].clone() else {
        panic!("the pasted effect is a drop shadow")
    };
    assert_eq!((shadow.radius, shadow.offset, visible), (7.0, (2.0, 3.0), true));
}

#[test]
fn pasting_design_layers_keeps_a_copied_row() {
    let mut s = Session::new();
    let a = rect(&mut s);
    ex(&mut s, "fill.copy", json!({"id": a}));
    // Layers pasted from another app replace the copied layers, not the copied fill row.
    let mut other = Session::new();
    other.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    let exported = other.execute("file.export", &json!({"format": "fig"})).unwrap();
    ex(&mut s, "edit.pasteDesignLayers", json!({"data": exported["data"]}));
    let b = rect(&mut s);
    ex(&mut s, "fill.paste", json!({"ids": [b], "index": 0}));
}

#[test]
fn a_copied_image_fill_brings_its_image_to_another_file() {
    let mut s = Session::new();
    let a = rect(&mut s);
    ex(&mut s, "fill.add", json!({"ids": [a], "paint": {"type": "image", "image": "img-key"}}));
    let data = figcraft_doc::ImageData { bytes: std::sync::Arc::new(vec![1, 2, 3]), width: 2, height: 2 };
    s.doc_mut().unwrap().images.insert("img-key".into(), data.clone());
    ex(&mut s, "fill.copy", json!({"id": a, "index": 1}));
    // Another file has no image under that key: the copy brings the data with it.
    ex(&mut s, "file.new", json!({}));
    let b = rect(&mut s);
    assert!(!s.doc().unwrap().images.contains_key("img-key"));
    ex(&mut s, "fill.paste", json!({"ids": [b], "index": 0}));
    assert_eq!(s.doc().unwrap().images.get("img-key"), Some(&data));
    assert!(matches!(node(&s, b).fills[0].kind, figcraft_color::PaintKind::Image(_)));
}

#[test]
fn a_copied_image_key_taken_in_another_file_gets_the_copied_data() {
    let mut s = Session::new();
    let a = rect(&mut s);
    let copied = figcraft_doc::ImageData { bytes: std::sync::Arc::new(vec![1, 2, 3]), width: 2, height: 2 };
    let taken = figcraft_doc::ImageData { bytes: std::sync::Arc::new(vec![9, 9, 9]), width: 3, height: 3 };
    ex(&mut s, "fill.add", json!({"ids": [a], "paint": {"type": "image", "image": "img-key"}}));
    s.doc_mut().unwrap().images.insert("img-key".into(), copied.clone());
    ex(&mut s, "fill.copy", json!({"id": a, "index": 1}));
    // The other file already uses that key for different data: the pasted paint takes a content
    // key for the copied data instead, and the destination's image stays as it is.
    ex(&mut s, "file.new", json!({}));
    let b = rect(&mut s);
    s.doc_mut().unwrap().images.insert("img-key".into(), taken.clone());
    ex(&mut s, "fill.paste", json!({"ids": [b], "index": 0}));
    let doc = s.doc().unwrap();
    let fill = &doc.node(b).unwrap().fills[0];
    let figcraft_color::PaintKind::Image(img) = &fill.kind else { panic!("the pasted fill is an image") };
    assert_ne!(img.image, "img-key");
    assert_eq!(doc.images.get(&img.image), Some(&copied));
    assert_eq!(doc.images.get("img-key"), Some(&taken), "the destination's image stays");
}

#[test]
fn a_copied_image_whose_hash_key_is_also_taken_gets_a_free_key() {
    let mut s = Session::new();
    let a = rect(&mut s);
    let bytes = vec![1, 2, 3];
    let copied = figcraft_doc::ImageData { bytes: std::sync::Arc::new(bytes.clone()), width: 2, height: 2 };
    let taken = figcraft_doc::ImageData { bytes: std::sync::Arc::new(vec![9, 9, 9]), width: 3, height: 3 };
    // Images are usually keyed by their content hash: here the copied key is that hash, and the
    // other file already uses it for other data, so the replacement key is taken as well.
    let hash = figcraft_format::image_hash(&bytes);
    ex(&mut s, "fill.add", json!({"ids": [a], "paint": {"type": "image", "image": hash}}));
    s.doc_mut().unwrap().images.insert(hash.clone(), copied.clone());
    ex(&mut s, "fill.copy", json!({"id": a, "index": 1}));
    ex(&mut s, "file.new", json!({}));
    let b = rect(&mut s);
    s.doc_mut().unwrap().images.insert(hash.clone(), taken.clone());
    ex(&mut s, "fill.paste", json!({"ids": [b], "index": 0}));
    let doc = s.doc().unwrap();
    let figcraft_color::PaintKind::Image(img) = &doc.node(b).unwrap().fills[0].kind else { panic!("the pasted fill is an image") };
    assert_ne!(img.image, hash, "the taken hash is not reused");
    assert_eq!(doc.images.get(&img.image), Some(&copied), "the pasted paint points at the copied data");
    assert_eq!(doc.images.get(&hash), Some(&taken), "the destination's image stays");
    let key = img.image.clone();
    // An image key names an archive member, and only letters and digits are read back: the
    // numbered key must bring the copied image through a save and reopen.
    assert!(key.bytes().all(|b| b.is_ascii_alphanumeric()), "the key is archive-valid: {key}");
    let dir = std::env::temp_dir().join(format!("figcraft-image-key-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("pasted.figcraft").to_string_lossy().to_string();
    ex(&mut s, "file.save", json!({"path": path}));
    let mut t = Session::empty();
    ex(&mut t, "file.open", json!({"path": path}));
    let saved = t.doc().unwrap();
    let figcraft_color::PaintKind::Image(img) = &saved.node(b).unwrap().fills[0].kind else { panic!("the pasted fill is an image") };
    assert_eq!(img.image, key, "the reloaded paint keeps its key");
    // A reopened image's size is read from its pixels on demand, so only its bytes are compared.
    assert_eq!(saved.images.get(&key).map(|i| i.bytes.as_slice()), Some(copied.bytes.as_slice()), "the copied image survives the reload");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_copied_pattern_does_not_paste_into_another_file() {
    let mut s = Session::new();
    let source = rect(&mut s);
    let patterned = rect(&mut s);
    ex(&mut s, "fill.pattern", json!({"ids": [patterned], "source": source}));
    ex(&mut s, "fill.copy", json!({"id": patterned}));
    // Its node id means nothing in another file, where it may name an unrelated layer.
    ex(&mut s, "file.new", json!({}));
    let b = rect(&mut s);
    assert!(s.execute("fill.paste", &json!({"ids": [b], "index": 0})).is_err(), "a pattern stays in its file");
}

#[test]
fn a_copied_pattern_cannot_paste_into_a_layer_it_draws() {
    let mut s = Session::new();
    let outer = id(ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 100})));
    let inner = id(ex(&mut s, "node.create", json!({"type": "rectangle", "x": 10, "y": 10, "width": 20, "height": 20, "parent": outer})));
    // A pattern taken from the frame's child, on a layer beside the frame.
    let source = rect(&mut s);
    ex(&mut s, "fill.pattern", json!({"ids": [source], "source": inner}));
    ex(&mut s, "fill.copy", json!({"id": source}));
    // Onto the frame that holds the pattern's source it would draw itself, as `fill.pattern` refuses.
    assert!(s.execute("fill.paste", &json!({"ids": [outer]})).is_err());
    // Onto a layer that doesn't hold the pattern's source it still pastes.
    let other = rect(&mut s);
    ex(&mut s, "fill.paste", json!({"ids": [other], "index": 0}));
}
