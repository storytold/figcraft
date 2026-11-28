//! Text on a path: points land on the path turned with it, flipped runs the other way, glyphs
//! past an open path's end hide and a closed path wraps.

use figcraft_doc::{TextAutoResize, TextData, TextPath};
use figcraft_geom::{BezPath, Point};

use crate::OnPath;

fn tp(d: &str, start: f64, flip: bool) -> TextPath {
    TextPath { d: d.into(), start, flip }
}

fn near(a: Point, b: (f64, f64)) -> bool {
    (a.x - b.0).abs() < 1e-6 && (a.y - b.1).abs() < 1e-6
}

#[test]
fn points_follow_the_path() {
    // Halfway along a horizontal line, baseline at y = 10.
    let on = OnPath::new(&tp("M0 0 L100 0", 0.5, false), 0.0, 10.0).unwrap();
    let f = on.frame(0.0, 10.0).unwrap();
    assert!(near(f * Point::new(0.0, 10.0), (50.0, 0.0)));
    assert!(near(f * Point::new(0.0, 0.0), (50.0, -10.0)), "above the baseline is above the path");
    // Down a vertical line, x runs along y.
    let on = OnPath::new(&tp("M0 0 L0 100", 0.0, false), 0.0, 0.0).unwrap();
    let f = on.frame(10.0, 0.0).unwrap();
    assert!(near(f * Point::new(10.0, 0.0), (0.0, 10.0)));
    assert!(near(f * Point::new(11.0, 0.0), (0.0, 11.0)));
    // Flipped: from the other end, on the other side.
    let on = OnPath::new(&tp("M0 0 L100 0", 0.0, true), 0.0, 10.0).unwrap();
    let f = on.frame(0.0, 10.0).unwrap();
    assert!(near(f * Point::new(0.0, 10.0), (100.0, 0.0)));
    assert!(near(f * Point::new(0.0, 0.0), (100.0, 10.0)));
    // Centred text starts half its width earlier.
    let on = OnPath::new(&tp("M0 0 L100 0", 0.5, false), 20.0, 0.0).unwrap();
    assert!(near(on.frame(0.0, 0.0).unwrap() * Point::new(0.0, 0.0), (30.0, 0.0)));
}

#[test]
fn open_paths_end_and_closed_ones_wrap() {
    let open = OnPath::new(&tp("M0 0 L100 0", 0.0, false), 0.0, 0.0).unwrap();
    assert!(open.frame(150.0, 0.0).is_none() && open.frame(-1.0, 0.0).is_none());
    let closed = OnPath::new(&tp("M0 0 L100 0 L100 100 L0 100 Z", 0.0, false), 0.0, 0.0).unwrap();
    assert!(near(closed.frame(410.0, 0.0).unwrap() * Point::new(410.0, 0.0), (10.0, 0.0)), "wrapped past the start");
    assert!(OnPath::new(&tp("not a path", 0.0, false), 0.0, 0.0).is_none());
    assert!(OnPath::new(&tp("", f64::NAN, false), 0.0, 0.0).is_none());
}

#[test]
fn laid_out_glyphs_sit_on_the_path() {
    if crate::db().write().unwrap_or_else(std::sync::PoisonError::into_inner).default_face().is_none() {
        eprintln!("no fonts; skipping");
        return;
    }
    let mut path = BezPath::new();
    path.move_to((0.0, 0.0));
    path.line_to((0.0, 1000.0));
    let mut t = TextData::new("Hello");
    t.style.font_size = 20.0;
    t.path = Some(Box::new(TextPath::new(&path)));
    let l = crate::layout(&t, None, None);
    // Down the vertical line: every glyph is turned 90° and lies along x = 0.
    let ink = l.ink_bounds().unwrap();
    assert!(ink.width() < 60.0 && ink.height() > 40.0, "{ink:?}");
    assert!(l.glyphs.iter().all(|g| (g.warp.as_coeffs()[1] - 1.0).abs() < 1e-6));
    // Clicking near the third glyph puts the caret by it.
    let g = &l.glyphs[2];
    let mid = g.warp * Point::new(g.x + g.advance * 0.25, g.y - 5.0);
    assert_eq!(l.hit(mid.x, mid.y), g.char_start);
    // A text node on a path keeps its box.
    let mut n = figcraft_doc::Node::new(figcraft_doc::NodeId(1), figcraft_doc::NodeKind::Text(t.into()), "t", 10.0, 1000.0);
    if let figcraft_doc::NodeKind::Text(t) = &mut n.kind {
        t.auto_resize = TextAutoResize::WidthAndHeight;
    }
    assert!(crate::auto_size(&n).is_none());
}
