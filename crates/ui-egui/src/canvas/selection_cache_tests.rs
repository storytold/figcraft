//! The selection overlay's geometry and the Design panel's values are kept for the frames that
//! show the same (#1571), made again on every input they show (an edit, the selection, zoom, pan,
//! the canvas's place, the theme's overlay green), and the same as when made afresh; what they
//! don't keep (the hover and drop-target highlight) shows on the next frame.

use std::sync::Arc;

use egui::{Color32, Context, LayerId, Painter, Rect, pos2, vec2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::{Value, json};

use super::overlays::{SelectionGeometry, overlays, selection_geometry};
use crate::theme::{Tokens, set_canvas_page};
use crate::{FigApp, NoServices};
use figcraft_engine::tools::{PointerEvent, PointerKind};

fn id(v: Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// Three rectangles (the middle one rounded, outlined along its corners) selected, and a fourth
/// that isn't, on a light page.
fn setup() -> (FigApp, Context, Vec<NodeId>) {
    let mut s = Session::new();
    let mut ids = Vec::new();
    for x in [0, 80, 160, 240] {
        ids.push(id(s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 50, "height": 50})).unwrap()));
    }
    let radii = json!({"topLeftRadius": 12, "topRightRadius": 12, "bottomRightRadius": 12, "bottomLeftRadius": 12});
    s.execute("node.set", &json!({"id": ids[1], "props": radii})).unwrap();
    s.execute("select.set", &json!({"ids": ids[..3]})).unwrap();
    let mut app = FigApp::new(s, Box::new(NoServices));
    app.canvas.rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
    let ctx = Context::default();
    set_canvas_page(&ctx, Color32::WHITE);
    (app, ctx, ids)
}

fn geometry(app: &FigApp, ctx: &Context) -> SelectionGeometry {
    let ds = app.session.ds().unwrap();
    selection_geometry(app, &Painter::new(ctx.clone(), LayerId::background(), Rect::EVERYTHING), &ds.doc.clone(), &ds.selection.clone())
}

/// The geometry made with nothing kept, on a page of colour `page`.
fn fresh(app: &FigApp, page: Color32) -> SelectionGeometry {
    let ctx = Context::default();
    set_canvas_page(&ctx, page);
    geometry(app, &ctx)
}

fn kept(a: &SelectionGeometry, b: &SelectionGeometry) -> bool {
    Arc::ptr_eq(&a.outlines, &b.outlines)
}

#[test]
fn an_unchanged_frame_reuses_the_selection_geometry() {
    let (app, ctx, _) = setup();
    let first = geometry(&app, &ctx);
    assert_eq!(first.outlines.len(), 3);
    assert!(kept(&first, &geometry(&app, &ctx)));
    assert_eq!(first, fresh(&app, Color32::WHITE));
}

#[test]
fn a_kept_value_does_not_keep_its_document_alive() {
    let (app, ctx, _) = setup();
    let doc = app.session.ds().unwrap().doc.clone();
    let holders = Arc::strong_count(&doc);
    geometry(&app, &ctx);
    assert_eq!(Arc::strong_count(&doc), holders);
}

type Change = Box<dyn Fn(&mut FigApp, &Context)>;

#[test]
fn every_input_the_geometry_shows_makes_it_again() {
    let (mut app, ctx, ids) = setup();
    let (a, b) = (ids[0], ids[1]);
    let changes: Vec<(&str, Change)> = vec![
        (
            "an edit",
            Box::new(move |app: &mut FigApp, _: &Context| {
                app.set_props(&[b], json!({"x": 90}));
            }),
        ),
        (
            "the selection",
            Box::new(move |app: &mut FigApp, _: &Context| {
                app.session.execute("select.set", &json!({"ids": [a, b]})).unwrap();
            }),
        ),
        ("zoom", Box::new(|app: &mut FigApp, _: &Context| app.session.ds_mut().unwrap().view.zoom *= 2.0)),
        ("pan", Box::new(|app: &mut FigApp, _: &Context| app.session.ds_mut().unwrap().view.cx += 30.0)),
        ("the canvas moving", Box::new(|app: &mut FigApp, _: &Context| app.canvas.rect = app.canvas.rect.translate(vec2(40.0, 0.0)))),
        ("the theme", Box::new(|_: &mut FigApp, ctx: &Context| set_canvas_page(ctx, Color32::BLACK))),
    ];
    let mut page = Color32::WHITE;
    for (what, change) in changes {
        let before = geometry(&app, &ctx);
        change(&mut app, &ctx);
        if what == "the theme" {
            page = Color32::BLACK;
            assert_ne!(crate::theme::canvas_accent(&ctx), before.col, "a dark page takes the other green");
        }
        let after = geometry(&app, &ctx);
        assert!(!kept(&before, &after), "{what}");
        assert_ne!(before, after, "{what}");
        assert_eq!(after, fresh(&app, page), "{what}: as if nothing were kept");
        assert!(kept(&after, &geometry(&app, &ctx)), "{what}: kept again");
    }
}

/// How many shapes one frame of the canvas overlays draws.
fn drawn(app: &FigApp, ctx: &Context) -> usize {
    let mut out = ctx.run_ui(Default::default(), |ui| {
        overlays(app, &ui.ctx().layer_painter(LayerId::background()), &Tokens::light());
    });
    out.textures_delta.clear();
    out.shapes.len()
}

#[test]
fn the_hover_and_drop_highlights_show_over_kept_geometry() {
    let (mut app, ctx, ids) = setup();
    let other = ids[3];
    let plain = drawn(&app, &ctx);
    let first = geometry(&app, &ctx);
    app.session.ds_mut().unwrap().hover = Some(other);
    assert_eq!(drawn(&app, &ctx), plain + 1, "the hovered layer's outline");
    assert!(kept(&first, &geometry(&app, &ctx)), "the hover isn't part of the selection's geometry");
    app.session.ds_mut().unwrap().hover = None;
    app.session.tool.overlay.drop_target = Some(other);
    assert_eq!(drawn(&app, &ctx), plain + 1, "the drop target's outline");
    app.session.tool.overlay.drop_target = None;
    assert_eq!(drawn(&app, &ctx), plain);
}

#[test]
fn the_design_panels_instance_facts_follow_edits_and_the_selection() {
    let mut s = Session::new();
    let f = id(s.execute("node.create", &json!({"type": "frame", "x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    id(s.execute("node.create", &json!({"type": "rectangle", "x": 10, "y": 10, "width": 20, "height": 20, "parent": f})).unwrap());
    let main = id(s.execute("component.create", &json!({"ids": [f]})).unwrap());
    let inst = id(s.execute("instance.create", &json!({"component": main, "x": 300, "y": 0})).unwrap());
    let child = s.doc().unwrap().node(inst).unwrap().children[0];
    let ctx = Context::default();
    let facts = |s: &Session| {
        let ds = s.ds().unwrap();
        crate::right::in_instance(&ctx, &ds.doc.clone(), &ds.selection.clone())
    };
    let facts_of = |s: &mut Session, ids: Value| {
        s.execute("select.set", &json!({"ids": ids})).unwrap();
        let f = facts(s);
        (f.any, f.nested)
    };
    assert_eq!(facts_of(&mut s, json!([main])), (false, false));
    assert_eq!(facts_of(&mut s, json!([inst])), (true, false));
    assert_eq!(facts_of(&mut s, json!([inst, child])), (true, true));
    assert_eq!(facts_of(&mut s, json!([inst])), (true, false));
    s.execute("component.detach", &json!({"ids": [inst]})).unwrap();
    assert!(s.doc().unwrap().node(inst).is_some(), "detaching keeps the layer");
    assert_eq!(s.ds().unwrap().selection, vec![inst], "and the selection");
    assert_eq!(facts(&s), crate::right::InInstance { any: false, nested: false }, "an edit");
}

#[test]
fn the_position_fields_follow_edits_and_the_selection() {
    let (mut app, ctx, ids) = setup();
    let x = |app: &FigApp| {
        let ds = app.session.ds().unwrap();
        crate::right::position_values(&ctx, &ds.doc.clone(), &ds.selection.clone()).x
    };
    assert_eq!(x(&app), None, "mixed");
    app.session.execute("select.set", &json!({"ids": [ids[1]]})).unwrap();
    assert_eq!(x(&app), Some(80.0), "the selection");
    app.set_props(&[ids[1]], json!({"x": 90}));
    assert_eq!(x(&app), Some(90.0), "an edit");
}

/// The visual box of every shape one frame of the canvas overlays draws, in drawing order.
fn boxes(app: &FigApp, ctx: &Context) -> Vec<Rect> {
    let mut out = ctx.run_ui(Default::default(), |ui| {
        overlays(app, &ui.ctx().layer_painter(LayerId::background()), &Tokens::light());
    });
    out.textures_delta.clear();
    out.shapes.iter().map(|s| s.shape.visual_bounding_rect()).collect()
}

/// A drag of the selection held at (`x`, `y`) on the page, from (10, 10); snapping is off (Cmd).
fn drag_to(app: &mut FigApp, x: f64, y: f64) {
    let ev = |kind, x, y| PointerEvent { cmd: true, ..PointerEvent::at(kind, x, y, 1.0) };
    if app.session.ds().unwrap().interaction.is_none() {
        app.session.pointer(&ev(PointerKind::Down, 10.0, 10.0)).unwrap();
    }
    app.session.pointer(&ev(PointerKind::Move, x, y)).unwrap();
}

#[test]
fn a_move_shifts_the_geometry_kept_from_where_it_started() {
    let (mut app, ctx, _) = setup();
    let start = app.session.ds().unwrap().doc.clone();
    drag_to(&mut app, 47.0, 71.0);
    assert_eq!(app.session.ds().unwrap().interaction.as_ref().and_then(|i| i.moved_by), Some((37.0, 61.0)));
    let moving = boxes(&app, &ctx);
    let first = selection_geometry(&app, &Painter::new(ctx.clone(), LayerId::background(), Rect::EVERYTHING), &start, &app.session.selection());
    drag_to(&mut app, 57.0, 91.0);
    let again = boxes(&app, &ctx);
    let kept_since = selection_geometry(&app, &Painter::new(ctx.clone(), LayerId::background(), Rect::EVERYTHING), &start, &app.session.selection());
    assert!(kept(&first, &kept_since), "the geometry where the move started is kept for it");
    // What shows is what the moved document shows when nothing is kept, to a hundredth of a point.
    let mut fresh = FigApp::new(Session::new(), Box::new(NoServices));
    fresh.canvas.rect = app.canvas.rect;
    {
        let (ds, now) = (fresh.session.ds_mut().unwrap(), app.session.ds().unwrap());
        (ds.doc, ds.selection, ds.view) = (now.doc.clone(), now.selection.clone(), now.view.clone());
    }
    let ctx2 = Context::default();
    set_canvas_page(&ctx2, Color32::WHITE);
    let want = boxes(&fresh, &ctx2);
    assert_eq!(again.len(), want.len());
    for (got, want) in again.iter().zip(&want) {
        assert!((got.min - want.min).length() < 0.01 && (got.max - want.max).length() < 0.01, "{got:?} != {want:?}");
    }
    assert_ne!(moving, again, "the second move shows further on");
}

#[test]
fn the_gesture_start_stands_in_only_while_the_selection_is_only_moved() {
    let (mut app, _, _) = setup();
    let start = app.session.ds().unwrap().doc.clone();
    assert!(crate::selection_cache::moved(app.session.ds().unwrap()).is_none(), "no gesture");
    drag_to(&mut app, 30.0, 10.0);
    let (base, d) = crate::selection_cache::moved(app.session.ds().unwrap()).expect("a plain move");
    assert!(Arc::ptr_eq(&base, &start) && d == (20.0, 0.0));
    // The selection changing under the gesture: the start no longer stands in.
    app.session.ds_mut().unwrap().selection.pop();
    assert!(crate::selection_cache::moved(app.session.ds().unwrap()).is_none());
}
