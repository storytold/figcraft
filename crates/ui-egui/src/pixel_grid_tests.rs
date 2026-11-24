//! The pixel grid, drawn headless: from 400% its lines are one screen pixel wide and a grey that
//! shows on a white frame and on the dark canvas alike, as in Figma.

use egui::{Color32, RawInput, Shape, ViewportId, ViewportInfo};
use figcraft_engine::Session;
use serde_json::json;

use crate::canvas::overlays::PIXEL_GRID;
use crate::layers_drag_tests::Ui;
use crate::live_tests::flat;

/// The width of each grid line drawn in a frame on a screen of `native` pixels per point (the
/// last of a few, for a new scale to take effect).
fn grid_lines(u: &mut Ui, native: f32) -> Vec<f32> {
    let mut out = None;
    for _ in 0..4 {
        let mut raw = RawInput::default();
        raw.viewports.insert(ViewportId::ROOT, ViewportInfo { native_pixels_per_point: Some(native), ..Default::default() });
        out = Some(u.frame_raw(raw));
    }
    let shapes = flat(&out.map(|o| o.shapes).unwrap_or_default());
    shapes
        .iter()
        .filter_map(|s| match s {
            Shape::LineSegment { stroke, .. } if stroke.color == PIXEL_GRID => Some(stroke.width),
            _ => None,
        })
        .collect()
}

/// `c` drawn over a grey of `bg`, as the canvas blends it.
fn over(c: Color32, bg: u8) -> f32 {
    let [r, _, _, a] = c.to_srgba_unmultiplied();
    let a = f32::from(a) / 255.0;
    f32::from(bg) * (1.0 - a) + f32::from(r) * a
}

#[test]
fn the_pixel_grid_shows_on_white_and_on_dark_one_screen_pixel_wide() {
    let mut s = Session::new();
    s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap();
    let mut u = Ui::new(s);
    u.app.session.ds_mut().unwrap().view.zoom = 8.0;
    let widths = grid_lines(&mut u, 1.0);
    assert!(!widths.is_empty(), "the grid draws at 800%");
    assert!(widths.iter().all(|w| *w == 1.0), "one screen pixel: {widths:?}");
    // Measured in Figma (2026-11-24): a grid line is 242 on white and 78 on a grey of 50.
    assert_eq!((over(PIXEL_GRID, 255).round(), over(PIXEL_GRID, 50).round()), (242.0, 78.0));
    // On a 2x screen the lines stay one of its pixels wide.
    let widths = grid_lines(&mut u, 2.0);
    assert!(!widths.is_empty() && widths.iter().all(|w| *w == 0.5), "{widths:?}");
    u.app.session.ds_mut().unwrap().view.zoom = 3.0;
    assert!(grid_lines(&mut u, 2.0).is_empty(), "no grid below 400%");
}
