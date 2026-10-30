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
