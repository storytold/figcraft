//! The Motion inspector's keyframed colours and effects, and the clip window opened by the
//! Animations "+" menu: edits write keyframes at the playhead (not the static fill / effect), and
//! the evaluated node carries the interpolated value the canvas draws.

use figcraft_color::Color;
use figcraft_doc::NodeId;
use figcraft_doc::anim::motion::{self, AnimProperty, AnimValue, ClipKind};
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

fn setup() -> (Ui, NodeId) {
    let mut s = Session::new();
    let created = s.execute("node.create", &json!({"type": "rectangle", "x": 0, "y": 0, "width": 100, "height": 100})).unwrap();
    let layer = NodeId(created["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [layer]})).unwrap();
    let mut u = Ui::new(s);
    crate::motion::set_open(&mut u.app, true);
    for _ in 0..3 {
        u.frame(vec![]);
    }
    (u, layer)
}

fn track_len(u: &Ui, layer: NodeId, prop: AnimProperty) -> usize {
    u.app.session.doc().unwrap().node(layer).unwrap().motion.track(prop).map_or(0, |t| t.keys.len())
}

/// Fill edits in Motion mode key the colour at the playhead; the evaluated node interpolates it.
#[test]
fn fill_edits_write_keyframes_that_the_canvas_evaluates() {
    let (mut u, layer) = setup();
    let red = Color::from_rgb8_const(255, 0, 0);
    let blue = Color::from_rgb8_const(0, 0, 255);
    u.app.ui.motion.playhead_ms = 0.0;
    crate::motion::inspector::commit_anim_value(&mut u.app, layer, AnimProperty::FillColor, AnimValue::Color(red));
    u.app.ui.motion.playhead_ms = 1000.0;
    crate::motion::inspector::commit_anim_value(&mut u.app, layer, AnimProperty::FillColor, AnimValue::Color(blue));
    assert_eq!(track_len(&u, layer, AnimProperty::FillColor), 2, "two keyframes, not a static fill edit");

    let doc = u.app.session.doc().unwrap();
    let n = doc.node(layer).unwrap();
    let mid = motion::evaluate_node(n, 500.0).fill_color.expect("an evaluated fill colour");
    assert!((mid.r - 0.5).abs() < 0.02 && (mid.b - 0.5).abs() < 0.02, "half way: {mid:?}");
    let end = motion::evaluate_node(n, 1000.0).fill_color.expect("the end colour");
    assert!((end.b - 1.0).abs() < 0.02 && end.r.abs() < 0.02, "the second keyframe: {end:?}");
}

/// The first drop shadow's offset keys like the inspector's own fields, and the canvas sees it.
#[test]
fn shadow_edits_write_keyframes_for_the_first_drop_shadow() {
    let (mut u, layer) = setup();
    u.app.session.execute("effect.add", &json!({"ids": [layer], "type": "dropShadow"})).unwrap();
    u.app.ui.motion.playhead_ms = 0.0;
    crate::motion::inspector::commit_anim_value(&mut u.app, layer, AnimProperty::ShadowX, AnimValue::Scalar(0.0));
    u.app.ui.motion.playhead_ms = 1000.0;
    crate::motion::inspector::commit_anim_value(&mut u.app, layer, AnimProperty::ShadowX, AnimValue::Scalar(40.0));
    assert_eq!(track_len(&u, layer, AnimProperty::ShadowX), 2);

    let doc = u.app.session.doc().unwrap();
    let n = doc.node(layer).unwrap();
    let mid = motion::evaluate_node(n, 500.0).shadow_x.expect("an evaluated shadow x");
    assert!((mid - 20.0).abs() < 0.5, "half way: {mid}");
}

/// The "+" menu adds one clip of the chosen kind and opens its settings window, which stays open
/// across frames (the popover measures itself on the first one).
#[test]
fn the_plus_menu_adds_one_kind_and_opens_its_window() {
    let (mut u, layer) = setup();
    crate::motion::inspector::add_clip(&mut u.app, layer, ClipKind::Scale);
    let (clips, id) = {
        let doc = u.app.session.doc().unwrap();
        let clips = doc.node(layer).unwrap().motion.clips.clone();
        (clips.clone(), clips[0].id)
    };
    assert_eq!(clips.len(), 1, "one clip, not several");
    assert_eq!(clips[0].kind, ClipKind::Scale);
    assert_eq!(u.app.ui.motion.clip_popover, Some((layer, id)), "its settings open");
    for _ in 0..4 {
        u.frame(vec![]);
    }
    assert!(u.app.ui.motion.clip_pop.is_some(), "the window is kept between frames");
    assert_eq!(u.app.ui.motion.clip_popover, Some((layer, id)), "and stays open");
}

/// The preview is a picture, not an edit: the evaluated page is handed to the renderer while the
/// session keeps the document itself, at its own values, for input, commands and undo.
#[test]
fn the_motion_preview_never_replaces_the_document() {
    let (mut u, layer) = setup();
    {
        // Two position keyframes, written through the model (the inspector's own path is the
        // `motion.keyframe` command, which the other tests cover).
        let ds = u.app.session.ds_mut().unwrap();
        let doc = std::sync::Arc::make_mut(&mut ds.doc);
        let pos = doc.node_mut(layer).unwrap().motion.ensure_track(AnimProperty::Position);
        pos.set_key(0, AnimValue::Vec2(figcraft_geom::Vec2::new(0.0, 0.0)), None);
        pos.set_key(1000, AnimValue::Vec2(figcraft_geom::Vec2::new(100.0, 100.0)), None);
    }
    u.app.ui.motion.playhead_ms = 500.0;
    let held = u.app.session.ds().unwrap().doc.clone();
    let steps = u.app.session.undo_labels(100).len();
    let preview = u.app.preview_document().expect("a preview at 500 ms");
    let x = |doc: &figcraft_doc::Document| doc.node(layer).unwrap().transform.as_coeffs()[4];
    assert!((x(&preview) - 50.0).abs() < 1e-6, "the preview shows the page half way: {}", x(&preview));
    assert_eq!(x(&held), 0.0, "the document keeps its own position");
    assert!(std::sync::Arc::ptr_eq(&held, &u.app.session.ds().unwrap().doc), "the session's document is the same one");
    assert_eq!(u.app.session.undo_labels(100).len(), steps, "and a preview is no undo step");
}

/// A document that refuses edits — a version preview, say — drops a Motion edit instead of writing
/// it: the keyframe and clip commands never run, so nothing reaches the file or its undo.
#[test]
fn a_refusing_document_drops_the_inspectors_edits() {
    let (mut u, layer) = setup();
    u.app.ui.motion.playhead_ms = 500.0;
    let doc = u.app.session.doc().unwrap().clone();
    let before = serde_json::to_string(&doc).unwrap();
    let steps = u.app.session.undo_labels(100).len();
    u.app.session.ds_mut().unwrap().start_preview("v1", "Version 1", 0, std::sync::Arc::new(doc));
    let red = Color::from_rgb8_const(255, 0, 0);
    crate::motion::inspector::commit_anim_value(&mut u.app, layer, AnimProperty::FillColor, AnimValue::Color(red));
    crate::motion::inspector::commit_anim_value(&mut u.app, layer, AnimProperty::Opacity, AnimValue::Scalar(0.5));
    crate::motion::inspector::add_clip(&mut u.app, layer, ClipKind::Opacity);
    assert_eq!(track_len(&u, layer, AnimProperty::FillColor), 0, "no colour key");
    assert_eq!(track_len(&u, layer, AnimProperty::Opacity), 0, "no opacity key");
    assert!(u.app.session.doc().unwrap().node(layer).unwrap().motion.clips.is_empty(), "no clip");
    assert_eq!(serde_json::to_string(u.app.session.doc().unwrap()).unwrap(), before, "the document is what it was");
    assert_eq!(u.app.session.undo_labels(100).len(), steps, "and no undo step");
}
