//! A clipping frame that takes in the whole pixmap cuts nothing off there: its children draw
//! without a layer, a mask or a copy back, and look the same.

use figcraft_color::{Color, Paint};
use figcraft_doc::{Document, NodeId, NodeKind};
use figcraft_geom::Affine;

use crate::draw::{NO_COVER_SHORTCUTS, clip_covers};
use crate::retained_tests::assert_same;
use crate::{Options, Pixmap, View, render_view_single};

fn white() -> Options {
    Options { background: Some(Color::WHITE), ..Default::default() }
}

/// A screen (clipping) > a list (clipping) > a card (rounded, clipping) > a picture (rounded,
/// clipping) > a shape; the document, its page and the shape.
fn card() -> (Document, NodeId, NodeId) {
    let fill = |d: &mut Document, id: NodeId, hex: &str| d.get_mut(id).unwrap().fills = vec![Paint::solid(Color::from_hex(hex).unwrap())];
    let mut d = Document::new("clips");
    let page = d.first_page().unwrap();
    let screen = d.create(page, NodeKind::Frame, "Screen", 470.0, 0.0, 390.0, 844.0).unwrap();
    fill(&mut d, screen, "F4F6F5");
    let list = d.create(screen, NodeKind::Frame, "List", 24.0, 261.0, 342.0, 312.0).unwrap();
    let card = d.create(list, NodeKind::Frame, "Card", 0.0, 108.0, 342.0, 96.0).unwrap();
    fill(&mut d, card, "FFFFFF");
    d.get_mut(card).unwrap().corners.radii = [20.0; 4];
    let picture = d.create(card, NodeKind::Frame, "Picture", 12.0, 12.0, 72.0, 72.0).unwrap();
    fill(&mut d, picture, "CFE3DA");
    d.get_mut(picture).unwrap().corners.radii = [14.0; 4];
    let shape = d.create(picture, NodeKind::Polygon { count: 5 }, "Shape", 0.0, 23.0, 72.0, 49.0).unwrap();
    fill(&mut d, shape, "7FAE9C");
    (d, page, shape)
}

/// A `width` × `height` view at `scale` with the shape in the middle.
fn close_up(d: &Document, shape: NodeId, scale: f64, width: u32, height: u32) -> View {
    let b = crate::draw::paint_bounds(d, shape).unwrap();
    let (cx, cy) = ((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0);
    View { origin: (cx - f64::from(width) / scale / 2.0, cy - f64::from(height) / scale / 2.0), scale, width, height }
}

/// The view rendered with and without the shortcut for clips that cut nothing.
fn both(d: &Document, page: NodeId, v: &View) -> (Pixmap, Pixmap) {
    let new = render_view_single(d, page, v, &white()).unwrap();
    NO_COVER_SHORTCUTS.with(|f| f.set(true));
    let old = render_view_single(d, page, v, &white());
    NO_COVER_SHORTCUTS.with(|f| f.set(false));
    (new, old.unwrap())
}

#[test]
fn views_inside_and_across_clipping_frames_look_the_same() {
    let (d, page, shape) = card();
    let views = [
        // Inside the card and its picture; across the picture's round corner; across the card's
        // edge and the list's; the whole screen; on fractional pixels.
        close_up(&d, shape, 38.66, 300, 200),
        View { origin: (504.0, 440.0), scale: 9.0, width: 300, height: 260 },
        View { origin: (480.0, 440.0), scale: 6.0, width: 400, height: 300 },
        View { origin: (460.0, -10.0), scale: 0.5, width: 220, height: 440 },
        View { origin: (500.37, 381.21), scale: 3.3, width: 300, height: 260 },
    ];
    for v in &views {
        let (new, old) = both(&d, page, v);
        assert_same(&new, &old, &format!("{:?} at {}", v.origin, v.scale));
    }
}

#[test]
fn a_rotated_frame_clips_as_it_did() {
    let mut d = Document::new("rotated");
    let page = d.first_page().unwrap();
    let f = d.create(page, NodeKind::Frame, "F", 100.0, 100.0, 300.0, 200.0).unwrap();
    d.get_mut(f).unwrap().fills = vec![Paint::solid(Color::from_hex("FFE0B0").unwrap())];
    d.get_mut(f).unwrap().transform = figcraft_geom::set_rotation_about_center(d.node(f).unwrap().transform, 300.0, 200.0, 25.0);
    let c = d.create(f, NodeKind::Rectangle, "C", -40.0, 40.0, 420.0, 60.0).unwrap();
    d.get_mut(c).unwrap().fills = vec![Paint::solid(Color::from_hex("E03030").unwrap())];
    // The frame's middle (inside it whole), and its corner (cut by two edges).
    for v in
        [View { origin: (220.0, 170.0), scale: 4.0, width: 200, height: 160 }, View { origin: (60.0, 60.0), scale: 2.0, width: 300, height: 300 }]
    {
        let (new, old) = both(&d, page, &v);
        assert_same(&new, &old, &format!("{:?}", v.origin));
    }
}

#[test]
fn a_pixmap_in_a_round_corner_is_not_taken_in() {
    let mut d = Document::new("corner");
    let page = d.first_page().unwrap();
    let f = d.create(page, NodeKind::Frame, "F", 0.0, 0.0, 200.0, 100.0).unwrap();
    d.get_mut(f).unwrap().corners.radii = [40.0, 0.0, 0.0, 0.0];
    let at = |d: &Document, x: f64, y: f64| clip_covers(d.node(f).unwrap(), Affine::translate((-x, -y)), (10, 10));
    assert!(at(&d, 100.0, 40.0), "the middle");
    assert!(at(&d, 190.0, 0.0), "a square corner, up to the edges");
    assert!(!at(&d, 195.0, 0.0), "across the right edge");
    assert!(!at(&d, 2.0, 2.0), "outside the arc of the round corner");
    assert!(at(&d, 20.0, 20.0), "inside the arc");
    assert!(!at(&d, 5.0, 20.0), "across the arc");
    // A smoothed corner turns further along its edges: its whole square counts as cut off.
    d.get_mut(f).unwrap().corners.smoothing = 0.6;
    assert!(!at(&d, 20.0, 20.0), "smoothed: the corner's square is left out");
    assert!(at(&d, 100.0, 40.0), "smoothed: the middle");
}

#[test]
fn a_view_inside_clipping_frames_holds_no_layer_for_them() {
    let (d, page, shape) = card();
    let v = close_up(&d, shape, 38.66, 600, 300);
    let held = || crate::layer_budget::within(crate::layer_budget::BUDGET, || render_view_single(&d, page, &v, &Options::default())).1;
    // The view lies inside the screen, the list, the card and the picture: none of their clips
    // cuts it. Each held a copy of the view while its children drew.
    assert_eq!(held(), 0, "no layer for a clip that cuts nothing");
    NO_COVER_SHORTCUTS.with(|f| f.set(true));
    let before = held();
    NO_COVER_SHORTCUTS.with(|f| f.set(false));
    assert!(before >= u64::from(v.width) * u64::from(v.height) * 4, "as it was: {before} bytes held");
}
