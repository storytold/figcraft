//! The pixel grid, drawn headless: from 400% its lines are one screen pixel wide and a grey that
//! shows on a white frame and on the dark canvas alike, as in Figma.

use egui::{Color32, RawInput, Shape, ViewportId, ViewportInfo};
use figcraft_engine::Session;
use serde_json::json;

use crate::canvas::overlays::PIXEL_GRID;
use crate::layers_drag_tests::Ui;
use crate::live_tests::flat;

/// Each grid line drawn in a frame on a screen of `native` pixels per point (the last of a few,
/// for a new scale to take effect): its width and the coordinate it runs along, its x if
/// vertical and its y if horizontal.
fn grid_lines(u: &mut Ui, native: f32) -> Vec<(f32, f32)> {
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
            Shape::LineSegment { points: [a, b], stroke } if stroke.color == PIXEL_GRID => Some((stroke.width, if a.x == b.x { a.x } else { a.y })),
            _ => None,
        })
        .collect()
}

/// Whether every line in `lines` sits on the middle of a row or column of pixels of a screen of
/// `native` pixels per point, so it covers that one pixel and no half of two.
fn on_pixel_centres(lines: &[(f32, f32)], native: f32) -> bool {
    lines.iter().all(|(_, at)| ((at * native - 0.5) - (at * native - 0.5).round()).abs() < 1e-3)
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
    let view = &mut u.app.session.ds_mut().unwrap().view;
    view.zoom = 8.0;
    // A view between whole points, so the page's pixels fall between the screen's.
    (view.cx, view.cy) = (50.03, 50.07);
    let lines = grid_lines(&mut u, 1.0);
    assert!(!lines.is_empty(), "the grid draws at 800%");
    assert!(lines.iter().all(|(w, _)| *w == 1.0), "one screen pixel: {lines:?}");
    assert!(on_pixel_centres(&lines, 1.0), "on the screen's pixels: {lines:?}");
    // Measured in Figma (2026-11-24): a grid line is 242 on white and 78 on a grey of 50.
    assert_eq!((over(PIXEL_GRID, 255).round(), over(PIXEL_GRID, 50).round()), (242.0, 78.0));
    // On a 2x screen the lines stay one of its pixels wide.
    let lines = grid_lines(&mut u, 2.0);
    assert!(!lines.is_empty() && lines.iter().all(|(w, _)| *w == 0.5), "{lines:?}");
    assert!(on_pixel_centres(&lines, 2.0), "on the 2x screen's pixels: {lines:?}");
    u.app.session.ds_mut().unwrap().view.zoom = 3.0;
    assert!(grid_lines(&mut u, 2.0).is_empty(), "no grid below 400%");
}
