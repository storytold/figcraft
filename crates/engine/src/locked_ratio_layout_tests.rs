use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use figcraft_doc::NodeId;

fn id(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn artboard(s: &mut Session, y: f64, beside: f64) -> (NodeId, NodeId) {
    let frame = id(&s.execute("frame.create", &json!({"x": 0, "y": y, "width": 1728, "height": 400})).unwrap());
    s.execute("node.set", &json!({"id": frame, "props": {"layout": "horizontal", "gap": 60, "padding": [0, 30, 0, 30], "sizingV": "hug"}})).unwrap();
    let column = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": beside, "height": 300, "parent": frame})).unwrap());
    (frame, column)
}

fn size(s: &Session, n: NodeId) -> (f64, f64) {
    let n = s.doc().unwrap().node(n).unwrap();
    (n.width, n.height)
}

fn pasted_photo(locked: bool, cropped_image: bool) -> (f64, f64) {
    let mut s = Session::new();
    let (source, _) = artboard(&mut s, 0.0, 804.0);
    let photo = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 804, "height": 969.15, "parent": source})).unwrap());
    s.execute("node.set", &json!({"id": photo, "props": {"sizingH": "fill", "sizingV": "fixed", "constrainProportions": locked}})).unwrap();
    if cropped_image {
        s.doc_mut().unwrap().get_mut(photo).unwrap().fills = vec![figcraft_color::Paint::image("photo", figcraft_color::ImageScaleMode::Crop)];
    }
    assert_eq!(size(&s, photo), (804.0, 969.15), "the photo fills the 804 left beside the column");
    let (_, column) = artboard(&mut s, 2000.0, 516.0);
    s.set_selection(vec![photo]);
    s.execute("edit.copy", &json!({})).unwrap();
    s.set_selection(vec![column]);
    let pasted = s.execute("edit.paste", &json!({})).unwrap()["ids"].as_array().unwrap().iter().filter_map(id_of).collect::<Vec<_>>();
    size(&s, pasted[0])
}

#[test]
fn a_locked_ratio_layer_filling_a_row_keeps_its_proportions_when_pasted_into_a_wider_one() {
    let (w, h) = pasted_photo(true, false);
    assert_eq!(w, 1092.0, "it fills the 1092 left beside the narrower column");
    assert!((h / w - 969.15 / 804.0).abs() < 1e-9, "its height follows the width: {w} × {h}");
}

#[test]
fn a_pasted_cropped_image_filling_a_row_keeps_its_proportions_without_the_lock() {
    let (w, h) = pasted_photo(false, true);
    assert_eq!(w, 1092.0);
    assert!((h / w - 969.15 / 804.0).abs() < 1e-9, "the image isn't squashed: {w} × {h}");
}

#[test]
fn without_the_lock_a_layer_without_an_image_filling_a_row_keeps_its_fixed_height() {
    assert_eq!(pasted_photo(false, false), (1092.0, 969.15));
}

#[test]
fn a_locked_ratio_layer_filling_across_a_column_keeps_its_proportions() {
    let mut s = Session::new();
    let frame = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 600, "height": 400})).unwrap());
    s.execute("node.set", &json!({"id": frame, "props": {"layout": "vertical", "padding": 0, "sizingV": "hug"}})).unwrap();
    let photo = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 300, "height": 200, "parent": frame})).unwrap());
    s.execute("node.set", &json!({"id": photo, "props": {"constrainProportions": true, "sizingH": "fill", "sizingV": "fixed"}})).unwrap();
    assert_eq!(size(&s, photo), (600.0, 400.0));
    assert_eq!(size(&s, frame).1, 400.0, "the hugging column grows with it");
}

#[test]
fn a_main_component_with_a_cropped_image_pastes_as_an_instance_that_keeps_its_proportions() {
    let mut s = Session::new();
    let (source, _) = artboard(&mut s, 0.0, 804.0);
    let photo = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 804, "height": 969.15, "parent": source})).unwrap());
    let main = id(&s.execute("component.create", &json!({"ids": [photo]})).unwrap());
    s.execute("node.set", &json!({"id": main, "props": {"sizingH": "fill", "sizingV": "fixed", "constrainProportions": false}})).unwrap();
    s.doc_mut().unwrap().get_mut(main).unwrap().fills = vec![figcraft_color::Paint::image("photo", figcraft_color::ImageScaleMode::Crop)];
    let (_, column) = artboard(&mut s, 2000.0, 516.0);
    s.set_selection(vec![main]);
    s.execute("edit.copy", &json!({})).unwrap();
    s.set_selection(vec![column]);
    let pasted = s.execute("edit.paste", &json!({})).unwrap()["ids"].as_array().unwrap().iter().filter_map(id_of).collect::<Vec<_>>();
    let doc = s.doc().unwrap();
    let instance = doc.node(pasted[0]).unwrap();
    assert!(matches!(instance.kind, figcraft_doc::NodeKind::Instance { .. }), "a main copied in its file pastes as an instance");
    assert!(instance.constrain_proportions, "the instance's ratio is locked like any other pasted cropped image");
}

#[test]
fn a_locked_ratio_layer_in_a_wrapping_row_keeps_its_fixed_height() {
    let mut s = Session::new();
    let row = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 300})).unwrap());
    s.execute("node.set", &json!({"id": row, "props": {"layout": "horizontal", "wrap": true, "padding": 0, "gap": 0, "crossGap": 0}})).unwrap();
    let photo = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 50, "parent": row})).unwrap());
    let next = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 150, "height": 40, "parent": row})).unwrap());
    s.execute("node.set", &json!({"id": photo, "props": {"constrainProportions": true, "sizingH": "fill", "sizingV": "fixed", "minWidth": 100}}))
        .unwrap();
    assert_eq!(size(&s, photo), (200.0, 50.0), "in a wrapping row the fixed side stays fixed");
    let next_y = s.doc().unwrap().world_bounds(next).unwrap().y0;
    assert_eq!(next_y, 50.0, "the next line starts below the unchanged first one");
}

#[test]
fn a_rotated_locked_ratio_layer_filling_a_row_is_laid_out_as_without_the_lock() {
    let filled = |lock: bool| {
        let mut s = Session::new();
        let row = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 300})).unwrap());
        s.execute("node.set", &json!({"id": row, "props": {"layout": "horizontal", "padding": 0, "gap": 0}})).unwrap();
        let photo = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 50, "parent": row})).unwrap());
        s.execute("node.set", &json!({"id": photo, "props": {"rotation": 90, "constrainProportions": lock}})).unwrap();
        s.execute("node.set", &json!({"id": photo, "props": {"sizingH": "fill", "sizingV": "fixed"}})).unwrap();
        size(&s, photo)
    };
    assert_eq!(filled(true), filled(false), "the ratio of a turned layer's bounding box isn't the ratio of its own size");
}

#[test]
fn a_locked_ratio_layer_in_a_row_collapsed_to_nothing_comes_back_visible() {
    let mut s = Session::new();
    let row = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 300})).unwrap());
    s.execute("node.set", &json!({"id": row, "props": {"layout": "horizontal", "padding": 0, "gap": 0}})).unwrap();
    let photo = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 50, "parent": row})).unwrap());
    s.execute("node.set", &json!({"id": photo, "props": {"constrainProportions": true, "sizingH": "fill", "sizingV": "fixed"}})).unwrap();
    s.execute("node.set", &json!({"id": row, "props": {"width": 0}})).unwrap();
    s.execute("node.set", &json!({"id": row, "props": {"width": 200}})).unwrap();
    let (w, h) = size(&s, photo);
    assert_eq!(w, 200.0);
    assert!(h > 0.0, "the fixed side isn't left at zero: {w} × {h}");
}
