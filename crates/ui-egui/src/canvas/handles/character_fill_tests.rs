//! The canvas crop handles on a text whose characters share an image fill the layer doesn't have
//! (#1856): they show and edit the characters' image, the one the Fill section and the colour
//! picker show, not the layer's own fill at that index.

use base64::Engine as _;
use figcraft_color::{Color, ImageScaleMode, Paint, PaintKind};
use figcraft_engine::Session;
use serde_json::json;

use super::crop_target;
use crate::colorpicker::PickerTarget;
use crate::layers_drag_tests::Ui;
use crate::present_tests::id;

#[test]
fn the_crop_handles_follow_the_characters_shared_image_in_crop_mode() {
    let mut s = Session::new();
    let t = id(&s.execute("text.create", &json!({"text": "Hello", "x": 0, "y": 0})).unwrap());
    s.execute("fill.set", &json!({"ids": [t.0], "color": "#00FF00"})).unwrap();
    let image = image::RgbaImage::from_pixel(40, 20, image::Rgba([200, 30, 30, 255]));
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let data = base64::engine::general_purpose::STANDARD.encode(png.into_inner());
    s.execute("image.place", &json!({"data": data, "target": t.0, "mode": "fill"})).unwrap();
    s.execute("image.crop", &json!({"ids": [t.0], "index": 1})).unwrap();
    // The cropped image goes to the characters; the layer keeps only its green.
    let cropped = s.doc().unwrap().node(t).unwrap().fills[1].clone();
    s.execute("text.style", &json!({"id": t.0, "start": 0, "end": 5, "textFills": [cropped]})).unwrap();
    s.doc_mut().unwrap().get_mut(t).unwrap().fills = vec![Paint::solid(Color::from_hex("#00FF00").unwrap())];
    let mut u = Ui::new(s);
    // The picker on the Fill section's row: the characters' image, already in Crop mode.
    u.app.ui.picker = Some(PickerTarget::Paint { fills: true, index: 0, ids: vec![t] });
    let (at, fills, index, img, _) = crop_target(&u.app).expect("the handles show the characters' image");
    assert_eq!((at, fills, index), (t, true, 0));
    assert_eq!(img.scale_mode, ImageScaleMode::Crop);
    assert!(
        matches!(u.app.session.doc().unwrap().node(t).unwrap().fills[0].kind, PaintKind::Solid { .. }),
        "the layer's own fill is not what they show"
    );
}
