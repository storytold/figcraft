//! The coarse image around the view ([`Surround`]): three views wide and tall at a quarter of the
//! pixels each way, kept while the view stays a third of a view inside it, moved (a shift and the
//! strips) when a pan goes further, patched after an edit, none in pixel preview; and the desktop
//! worker hands one over after each refined render, so a pan past the canvas image shows that
//! instead of the bare page colour.

use std::time::Duration;

use figcraft_render::{View, Work};
use serde_json::json;

use super::zoom_preview_tests::{document, job, view_at};
use crate::render_worker::{Job, PREVIEW_FACTOR, Surround, Worker, surround_view};

/// Whether `outer` shows all of the page `inner` does, give or take the snapping to whole pixels.
fn contains(outer: &View, inner: &View) -> bool {
    let (o, i) = (outer.page_rect(), inner.page_rect());
    let eps = 2.0 / outer.scale;
    o.x0 <= i.x0 + eps && o.y0 <= i.y0 + eps && o.x1 >= i.x1 - eps && o.y1 >= i.y1 - eps
}

#[test]
fn the_area_around_a_view_is_three_views_wide_at_a_quarter_of_the_pixels() {
    let v = view_at((400.0, 300.0), 2.0, 800, 600);
    let a = surround_view(&v);
    assert_eq!(a.scale, v.scale / PREVIEW_FACTOR);
    assert_eq!((a.width, a.height), (600, 450), "3 × 800 / 4 by 3 × 600 / 4");
    let (pa, pv) = (a.page_rect(), v.page_rect());
    assert!((pa.center().x - pv.center().x).abs() < 1e-9 && (pa.center().y - pv.center().y).abs() < 1e-9, "centred on the view");
    assert!((pa.width() - 3.0 * pv.width()).abs() < 1e-9 && (pa.height() - 3.0 * pv.height()).abs() < 1e-9);
}

#[test]
fn the_image_around_the_view_is_kept_for_small_pans_and_moved_for_far_ones() {
    let (s, rect) = document();
    let ds = s.ds().unwrap();
    let at = |c: (f64, f64)| job(&ds.doc, ds.page, ds.uid, view_at(c, 1.0, 400, 300));
    let mut around = Surround::default();
    let first = around.render(&at((250.0, 150.0))).expect("the first one");
    assert_eq!(first.work, Work::Full);
    assert!(first.coarse && contains(&first.view, &surround_view(&at((250.0, 150.0)).view)));
    assert!(around.render(&at((250.0, 150.0))).is_none(), "the same request: nothing new");
    // A quarter of a view: it still reaches more than a third of a view past every side.
    assert!(around.render(&at((350.0, 225.0))).is_none(), "kept for a small pan");
    // Most of a view: moved to centre on the view again, built on the kept image.
    let moved = around.render(&at((570.0, 150.0))).expect("moved for a far pan");
    assert!(matches!(moved.work, Work::Partial { .. }), "a shift and the strips: {:?}", moved.work);
    assert!(contains(&moved.view, &surround_view(&at((570.0, 150.0)).view)));
    // An edit redraws its area.
    let mut s = s;
    s.execute("node.set", &json!({"id": rect, "props": {"x": 140}})).unwrap();
    let ds = s.ds().unwrap();
    let edited = job(&ds.doc, ds.page, ds.uid, view_at((570.0, 150.0), 1.0, 400, 300));
    assert!(matches!(around.render(&edited).expect("after an edit").work, Work::Partial { .. }));
    // Soft proof marks the image after it renders: turning it on or off hands over a new one.
    assert!(around.render(&Job { proof: true, ..edited.clone() }).is_some(), "proof on");
    assert!(around.render(&Job { proof: true, ..edited.clone() }).is_none());
    assert!(around.render(&edited).is_some(), "proof off");
    // Another zoom starts over; pixel preview has none.
    let zoomed = job(&ds.doc, ds.page, ds.uid, view_at((570.0, 150.0), 2.0, 400, 300));
    assert_eq!(around.render(&zoomed).expect("another zoom").work, Work::Full);
    assert!(around.render(&Job { zoom_preview: false, ..job(&ds.doc, ds.page, ds.uid, view_at((0.0, 0.0), 8.0, 400, 300)) }).is_none());
}

#[test]
fn the_worker_hands_over_the_image_around_each_refined_view() {
    let (s, _) = document();
    let ds = s.ds().unwrap();
    let j = job(&ds.doc, ds.page, ds.uid, view_at((250.0, 150.0), 1.0, 400, 300));
    let mut w = Worker::new(egui::Context::default());
    w.submit(j.clone());
    let start = std::time::Instant::now();
    let around = loop {
        assert!(start.elapsed() < Duration::from_secs(60), "no image around the view came");
        if let Some(d) = w.surround() {
            break d;
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!((around.uid, around.page), (ds.uid, ds.page));
    assert!(contains(&around.view, &surround_view(&j.view)), "it reaches a view past every side");
    assert!(w.poll().is_some_and(|d| !d.coarse), "the refined image came through as before");
}
