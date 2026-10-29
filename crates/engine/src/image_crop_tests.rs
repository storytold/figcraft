//! `image.crop` (Object ▸ Crop image, the right-click menu and the image fill row): the layer's
//! top visible image fill (or the one `index` names) switches to Crop with the picture left where
//! its old mode drew it, as one undo step, and the canvas is asked to enter crop mode. A layer
//! without an image fill, or several layers, are refused with a reason and nothing changes.

use base64::Engine as _;
use figcraft_color::{ImageScaleMode, Paint, PaintKind};
use figcraft_doc::NodeId;
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

/// A 200×100 synthetic image layer (every pixel differs), resized to 200×150 so Fill and Fit
/// place the picture differently.
fn image_layer(s: &mut Session) -> NodeId {
    let image = image::RgbaImage::from_fn(200, 100, |x, y| image::Rgba([x as u8, y as u8, 120, 255]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    let data = base64::engine::general_purpose::STANDARD.encode(bytes.into_inner());
    let id = id_of(&ex(s, "file.import", json!({"data": data, "name": "crop.png", "x": 0, "y": 0}))["id"]).unwrap();
    ex(s, "node.set", json!({"ids": [id], "props": {"height": 150}}));
    id
}

fn fills(s: &Session, id: NodeId) -> Vec<Paint> {
    s.doc().unwrap().node(id).unwrap().fills.clone()
}

fn set_mode(s: &mut Session, id: NodeId, mode: ImageScaleMode) {
    let mut f = fills(s, id);
    let PaintKind::Image(img) = &mut f[0].kind else { panic!("image fill") };
    img.scale_mode = mode;
    ex(s, "node.set", json!({"ids": [id], "props": {"fills": f}}));
}

fn pixels(s: &Session, id: NodeId) -> Vec<u8> {
    figcraft_render::render_node(s.doc().unwrap(), id, 1.0).unwrap().data().to_vec()
}

fn undo_steps(s: &Session) -> usize {
    s.ds().unwrap().history.undo.len()
}

#[test]
fn crop_image_switches_to_crop_keeping_the_picture_as_one_undo_step() {
    for mode in [ImageScaleMode::Fill, ImageScaleMode::Fit, ImageScaleMode::Crop] {
        let mut s = Session::new();
        let layer = image_layer(&mut s);
        set_mode(&mut s, layer, mode);
        let (before, doc_before, steps) = (pixels(&s, layer), s.doc().unwrap().clone(), undo_steps(&s));
        s.ui_requests.clear();
        let r = ex(&mut s, "image.crop", json!({}));
        assert_eq!(r, json!({"id": layer, "index": 0}), "{mode:?}");
        let PaintKind::Image(img) = &fills(&s, layer)[0].kind else { panic!("image fill") };
        assert_eq!(img.scale_mode, ImageScaleMode::Crop, "{mode:?}");
        assert_eq!(pixels(&s, layer), before, "{mode:?}: the picture stays where it was");
        let asked: Vec<_> = s.ui_requests.iter().filter(|u| u.kind == "image.crop").map(|u| u.data.clone()).collect();
        assert_eq!(asked, vec![json!({"id": layer, "index": 0})], "{mode:?}: the canvas enters crop mode");
        if mode == ImageScaleMode::Crop {
            assert_eq!(undo_steps(&s), steps, "already cropped: nothing to undo");
            continue;
        }
        assert_eq!(undo_steps(&s), steps + 1, "{mode:?}: one undo step");
        ex(&mut s, "edit.undo", json!({}));
        assert_eq!(s.doc().unwrap(), &doc_before, "{mode:?}: undo restores the fill");
    }
}

#[test]
fn crop_image_takes_the_top_visible_image_fill_or_the_one_named() {
    let mut s = Session::new();
    let layer = image_layer(&mut s);
    let mut f = fills(&s, layer);
    let image = f[0].clone();
    let mut hidden = image.clone();
    hidden.visible = false;
    // Bottom to top: image, image, solid, hidden image.
    f = vec![image.clone(), image, Paint::solid(figcraft_color::Color::BLACK), hidden];
    ex(&mut s, "node.set", json!({"ids": [layer], "props": {"fills": f}}));
    assert_eq!(ex(&mut s, "image.crop", json!({"ids": [layer]}))["index"], 1, "the top visible image fill");
    let modes =
        |s: &Session| fills(s, layer).iter().map(|p| if let PaintKind::Image(i) = &p.kind { Some(i.scale_mode) } else { None }).collect::<Vec<_>>();
    assert_eq!(modes(&s), [Some(ImageScaleMode::Fill), Some(ImageScaleMode::Crop), None, Some(ImageScaleMode::Fill)]);
    assert_eq!(ex(&mut s, "image.crop", json!({"ids": [layer], "index": 0}))["index"], 0, "an image fill by index");
    assert_eq!(modes(&s)[0], Some(ImageScaleMode::Crop));
    let err = s.execute("image.crop", &json!({"ids": [layer], "index": 2})).unwrap_err().to_string();
    assert!(err.contains("not an image"), "{err}");
    let err = s.execute("image.crop", &json!({"ids": [layer], "index": 9})).unwrap_err().to_string();
    assert!(err.contains("no fill at that index"), "{err}");
}

#[test]
fn crop_image_is_refused_without_one_image_layer() {
    let mut s = Session::new();
    let rect = id_of(&ex(&mut s, "shape.rectangle", json!({"x": 300, "y": 0, "width": 50, "height": 50}))["id"]).unwrap();
    let layer = image_layer(&mut s);
    let doc = s.doc().unwrap().clone();
    s.ui_requests.clear();
    for (ids, why) in [(json!([rect]), "no image fill"), (json!([rect, layer]), "select one layer")] {
        let err = s.execute("image.crop", &json!({"ids": ids})).unwrap_err();
        assert!(matches!(&err, crate::EngineError::Disabled(..)) && err.to_string().contains(why), "{err}");
    }
    assert_eq!(s.doc().unwrap(), &doc, "nothing changes");
    assert!(s.ui_requests.is_empty(), "no crop mode");
}
