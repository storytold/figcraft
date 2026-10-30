//! Canvas fixes, driven through egui headless: View ▸ Frame outlines and View ▸ Slices draw (off
//! by default), the render worker counts a job as in flight until it is done, and the Pen shows
//! the handles of the path it draws.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use egui::{Event, Modifiers, PointerButton, Pos2, Shape};
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;
use crate::live_tests::flat;
use crate::present_tests::id;

/// Whether `shapes` hold a closed outline through the corners `q`.
fn outlined(shapes: &[Shape], q: [Pos2; 4]) -> bool {
    shapes.iter().any(|s| matches!(s, Shape::Path(p) if p.closed && p.points.len() == 4 && p.points.iter().zip(q).all(|(a, b)| a.distance(b) < 0.5)))
}

/// How many line segments lie along the top edge from `a` to `b` (a dashed outline's dashes).
fn dashes_along(shapes: &[Shape], a: Pos2, b: Pos2) -> usize {
    let on = |p: Pos2| (p.y - a.y).abs() < 0.5 && p.x >= a.x - 0.5 && p.x <= b.x + 0.5;
    shapes.iter().filter(|s| matches!(s, Shape::LineSegment { points: [p, q], .. } if on(*p) && on(*q))).count()
}

#[test]
fn a_press_and_release_in_one_frame_draw_from_the_press() {
    // A quick drag (or a slow frame) puts both in one frame, where the hover position is the end:
    // the press must still use its own position, or the shape starts where the pointer let go.
    let mut u = Ui::new(Session::new());
    u.app.run("tool.set", json!({"tool": "rectangle"}));
    u.frame(vec![]);
    let press = u.app.to_screen(100.0, 100.0);
    let release = u.app.to_screen(260.0, 220.0);
    u.frame(vec![
        Event::PointerMoved(release),
        Event::PointerButton { pos: press, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
        Event::PointerButton { pos: release, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE },
    ]);
    for _ in 0..3 {
        u.frame(vec![]);
    }
    let doc = u.app.session.doc().unwrap();
    let drawn = u.app.session.selection().first().copied().expect("the press drew a shape");
    let b = doc.world_bounds(drawn).expect("the shape has bounds");
    assert!((b.x0 - 100.0).abs() < 1.0 && (b.y0 - 100.0).abs() < 1.0, "drawn from the press, not the release: {b:?}");
}

#[test]
fn frame_outlines_and_slices_draw_when_shown() {
    let mut s = Session::new();
    let f = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 120})).unwrap());
    let inner = id(&s.execute("frame.create", &json!({"x": 20, "y": 20, "width": 80, "height": 60, "parent": f})).unwrap());
    let slice = id(&s.execute("node.create", &json!({"type": "slice", "x": 300, "y": 0, "width": 100, "height": 50})).unwrap());
    s.execute("select.set", &json!({"ids": []})).unwrap();
    let mut u = Ui::new(s);
    let quad = |u: &Ui, i| crate::canvas::node_quad(&u.app, u.app.session.doc().unwrap(), i).unwrap();
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let sq = quad(&u, slice);
    assert!(!outlined(&shapes, quad(&u, inner)), "off by default");
    assert_eq!(dashes_along(&shapes, sq[0], sq[1]), 0, "slices hidden by default");
    u.app.run("view.frameOutlines", json!({}));
    u.app.run("view.slices", json!({}));
    let shapes = flat(&u.frame_output(vec![]).shapes);
    for frame in [f, inner] {
        assert!(outlined(&shapes, quad(&u, frame)), "every frame is outlined");
    }
    assert!(dashes_along(&shapes, sq[0], sq[1]) > 3, "the slice shows as a dashed rectangle");
    u.app.run("view.frameOutlines", json!({"on": false}));
    u.app.run("view.slices", json!({"on": false}));
    let shapes = flat(&u.frame_output(vec![]).shapes);
    assert!(!outlined(&shapes, quad(&u, inner)) && dashes_along(&shapes, sq[0], sq[1]) == 0, "both turn off again");
}

/// A job a few hundred milliseconds of rendering big.
fn big_job(doc: &Arc<figcraft_doc::Document>, page: figcraft_doc::NodeId, scale: f64) -> crate::render_worker::Job {
    crate::render_worker::Job {
        doc: doc.clone(),
        page,
        view: figcraft_render::View { origin: (0.0, 0.0), scale, width: 2400, height: 2400 },
        outline: false,
        outline_extras: Default::default(),
        outlined: vec![],
        skip: vec![],
        solo: None,
        proof: false,
        dots: None,
        uid: 0,
        revision: 0,
        zoom_preview: false,
        refine: false,
        gpu: false,
        at: web_time::Instant::now(),
    }
}

#[test]
fn a_job_queued_behind_a_finished_one_stays_in_flight() {
    let ctx = egui::Context::default();
    let wakes = Arc::new(AtomicUsize::new(0));
    let w = wakes.clone();
    ctx.set_request_repaint_callback(move |_| {
        w.fetch_add(1, Ordering::SeqCst);
    });
    let mut s = Session::new();
    for k in 0..40 {
        s.execute("shape.ellipse", &json!({"x": k * 50, "y": k * 50, "width": 400, "height": 400})).unwrap();
    }
    let ds = s.ds().unwrap();
    let mut worker = crate::render_worker::Worker::new(ctx.clone());
    worker.submit(big_job(&ds.doc, ds.page, 1.0));
    let start = Instant::now();
    while wakes.load(Ordering::SeqCst) == 0 {
        assert!(start.elapsed() < Duration::from_secs(60), "the first render never finished");
        std::thread::sleep(Duration::from_millis(2));
    }
    // As in a canvas frame: a new job goes out, then the finished one is picked up.
    worker.submit(big_job(&ds.doc, ds.page, 0.9));
    assert!(worker.poll().is_some(), "the first image");
    // Not in flight only once the second is done: its image is then waiting.
    assert!(worker.in_flight() || worker.poll().is_some(), "the second job counts until its image is there");
}

#[test]
fn the_pen_draws_the_handles_of_the_point_being_dragged_and_a_curved_band() {
    use egui::{Event, PointerButton};
    let mut u = Ui::new(Session::new());
    u.app.run("tool.set", json!({"tool": "pen"}));
    let at = |u: &Ui, x: f64, y: f64| u.app.to_screen(x, y);
    let press = |p: Pos2, pressed: bool| {
        vec![Event::PointerMoved(p), Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Default::default() }]
    };
    let accent = egui::epaint::ColorMode::Solid(crate::theme::ACCENT);
    // A handle: the line from its anchor and the dot at its end.
    let handle = |shapes: &[Shape], a: Pos2, e: Pos2| {
        let near = |p: Pos2, q: Pos2| p.distance(q) < 0.5;
        let line = shapes.iter().any(|s| matches!(s, Shape::LineSegment { points: [p, q], .. } if near(*p, a) && near(*q, e)));
        let dot = shapes.iter().any(|s| matches!(s, Shape::Circle(c) if near(c.center, e) && c.fill == egui::Color32::WHITE));
        line && dot
    };
    let a = at(&u, 100.0, 100.0);
    u.frame(press(a, true));
    for k in 1..=4 {
        u.frame(vec![Event::PointerMoved(at(&u, 100.0 + 10.0 * f64::from(k), 100.0))]);
    }
    let shapes = flat(&u.frame_output(vec![]).shapes);
    assert!(handle(&shapes, a, at(&u, 140.0, 100.0)), "the out handle while dragging");
    assert!(handle(&shapes, a, at(&u, 60.0, 100.0)), "and the in handle opposite");
    u.frame(press(at(&u, 140.0, 100.0), false));
    // After release, towards the next point: the band curves along the out handle.
    let shapes = flat(&u.frame_output(vec![Event::PointerMoved(at(&u, 200.0, 200.0))]).shapes);
    assert!(handle(&shapes, a, at(&u, 140.0, 100.0)), "the handles stay");
    let band = shapes.iter().any(|s| matches!(s, Shape::CubicBezier(c) if c.points[0].distance(a) < 0.5 && c.points[1].distance(at(&u, 140.0, 100.0)) < 0.5 && c.stroke.color == accent));
    assert!(band, "a curve leaving along the out handle");
}

/// While drawing, the Pencil shows the stroke it will make, in its colour and weight (#256): in a
/// dark theme the theme's text colour was white, so over a white shape nothing showed until the
/// stroke was let go.
#[test]
fn the_pencil_previews_its_stroke_in_its_colour_and_weight() {
    let mut u = Ui::new(Session::new());
    u.app.run("tool.set", json!({"tool": "pencil"}));
    let (color, weight) = figcraft_engine::tools::PENCIL_STROKE;
    let preview = |shapes: &[Shape]| {
        shapes.iter().find_map(|s| match s {
            Shape::Path(p) if !p.closed && p.points.len() >= 3 && p.stroke.color == egui::epaint::ColorMode::Solid(crate::theme::c32(color)) => {
                Some(p.stroke.width)
            }
            _ => None,
        })
    };
    let at = |u: &Ui, x: f64, y: f64| u.app.to_screen(x, y);
    let start = at(&u, 10.0, 10.0);
    u.button(start, true);
    for i in 1..=6 {
        let p = at(&u, 10.0 + 12.0 * f64::from(i), 10.0 + 6.0 * f64::from(i % 2));
        u.frame(vec![egui::Event::PointerMoved(p)]);
    }
    let zoom = u.app.session.ds().unwrap().view.zoom;
    let w = preview(&flat(&u.frame_output(vec![]).shapes)).expect("a preview while drawing");
    assert!((w - (weight * zoom) as f32).abs() < 0.01, "{w} for {weight} at {zoom}");
    // Zoomed far out it stays visible.
    u.app.session.ds_mut().unwrap().view.zoom = 0.05;
    let p = at(&u, 120.0, 40.0);
    let w = preview(&flat(&u.frame_output(vec![egui::Event::PointerMoved(p)]).shapes)).expect("still previewing");
    assert!((w - 1.0).abs() < 0.01, "at least a point: {w}");
}

#[test]
fn a_selected_layer_shows_its_constraint_guides_in_its_frame() {
    // A child pinned left (the default) in a frame without auto layout: a dashed guide runs from its
    // left-edge midpoint to the frame's left edge. An auto layout frame places its children itself,
    // so only a child that ignores the auto layout (absolute) shows one there.
    let mut s = Session::new();
    let f = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 120})).unwrap());
    let c = id(&s.execute("node.create", &json!({"type": "rectangle", "x": 80, "y": 40, "width": 40, "height": 40, "parent": f})).unwrap());
    s.execute("select.set", &json!({"ids": [c]})).unwrap();
    let mut u = Ui::new(s);
    let guide = |u: &mut Ui| {
        let shapes = flat(&u.frame_output(vec![]).shapes);
        let doc = u.app.session.doc().unwrap();
        let (fb, cb) = (doc.world_bounds(f).unwrap(), doc.world_bounds(c).unwrap());
        let y = cb.center().y;
        dashes_along(&shapes, u.app.to_screen(fb.x0, y), u.app.to_screen(cb.x0, y))
    };
    assert!(guide(&mut u) > 0, "a left constraint draws a dashed guide to the frame's left edge");
    u.app.run("node.set", json!({"id": f, "props": {"layout": "horizontal", "padding": 40}}));
    assert_eq!(guide(&mut u), 0, "auto layout places the child: no constraint guide");
    u.app.run("node.set", json!({"id": c, "props": {"absolute": true}}));
    assert!(guide(&mut u) > 0, "an absolute child is pinned by its constraints again");
}
