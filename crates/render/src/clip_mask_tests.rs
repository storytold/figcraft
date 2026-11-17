//! The mask a clipping frame draws its children through: none when the clip has no area.

use crate::draw::clip_mask;

fn line(from: (f32, f32), to: (f32, f32)) -> tiny_skia::Path {
    let mut b = tiny_skia::PathBuilder::new();
    b.move_to(from.0, from.1);
    b.line_to(to.0, to.1);
    b.finish().unwrap()
}

#[test]
fn a_clip_with_no_area_gives_no_mask_and_one_with_area_gives_one() {
    assert!(clip_mask(&line((0.0, 5.0), (10.0, 5.0)), 16, 16).is_none(), "a horizontal line lets nothing through");
    assert!(clip_mask(&line((5.0, 0.0), (5.0, 10.0)), 16, 16).is_none(), "nor does a vertical one");
    let square = tiny_skia::PathBuilder::from_rect(tiny_skia::Rect::from_xywh(2.0, 2.0, 8.0, 8.0).unwrap());
    let mask = clip_mask(&square, 16, 16).unwrap();
    assert!(mask.data().contains(&255), "a square covers its pixels");
}
