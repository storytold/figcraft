//! Copying a layer's style with Cmd+Alt+C / Ctrl+Alt+C and pasting it onto other layers with
//! Cmd+Alt+V / Ctrl+Alt+V: fills, strokes, corners, effects, opacity and blend mode.

use egui::{Event, Key, Modifiers};
use figcraft_doc::{NodeId, NodeKind};
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

fn id(v: serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// The copy and paste keys of the platform the tests run on.
fn chord() -> Modifiers {
    if cfg!(target_os = "macos") {
        Modifiers { alt: true, command: true, ..Default::default() }
    } else {
        Modifiers { alt: true, ctrl: true, command: true, ..Default::default() }
    }
}

/// Cmd/Ctrl+Alt+V: with Alt held the desktop shell delivers its press as for any other key.
fn press_v(u: &mut Ui) {
    let m = chord();
    u.frame_with(vec![Event::Key { key: Key::V, physical_key: Some(Key::V), pressed: true, repeat: false, modifiers: m }], m);
}

/// Two rectangles, the first with a full style and selected.
fn scene() -> (Ui, NodeId, NodeId) {
    let mut s = Session::new();
    let styled = id(s.execute("node.create", &json!({"type": "rectangle", "x": 0, "y": 0, "width": 40, "height": 20})).unwrap());
    let plain = id(s.execute("node.create", &json!({"type": "rectangle", "x": 100, "y": 0, "width": 40, "height": 20})).unwrap());
    s.execute(
        "node.set",
        &json!({"id": styled, "props": {
            "fills": [{"type": "solid", "color": "#ff0000"}],
            "strokes": [{"type": "solid", "color": "#0000ff"}],
            "strokeWeight": 3,
            "strokeAlign": "outside",
            "strokeSides": [1, 2, 3, 4],
            "startCap": "round",
            "endCap": "square",
            "strokeJoin": "bevel",
            "miterLimit": 7,
            "dashes": [4, 2],
            "widthProfile": [[0, 0.5], [1, 1]],
            "brush": "taper",
            "cornerRadius": 6,
            "cornerSmoothing": 0.6,
            "effects": [{"type": "dropShadow", "offset": [2, 4], "radius": 5, "spread": 1}],
            "opacity": 0.5,
            "blend": "multiply",
        }}),
    )
    .unwrap();
    s.execute("select.set", &json!({"ids": [styled]})).unwrap();
    (Ui::new(s), styled, plain)
}

#[test]
fn the_copy_chord_arrives_as_a_copy_event_and_takes_properties_not_layers() {
    let (mut u, _styled, _plain) = scene();
    // egui-winit sends Cmd/Ctrl+C as a copy event whatever Alt does: the key never reaches the
    // keymap, and Alt is what tells Copy properties from Copy.
    u.frame_with(vec![Event::Copy], chord());
    assert!(u.app.session.clipboard.props.is_some(), "Copy properties ran from the copy event");
    assert!(u.app.session.clipboard.nodes.is_empty(), "the layers were not copied");
    assert!(u.app.ui.toast.is_some(), "the copy shows a confirmation");
}

#[test]
fn the_style_keys_copy_and_paste_every_style_property() {
    let (mut u, styled, plain) = scene();
    u.frame_with(vec![Event::Copy], chord());
    u.app.session.execute("select.set", &json!({"ids": [plain]})).unwrap();
    press_v(&mut u);
    let doc = u.app.session.doc().unwrap();
    let (a, b) = (doc.node(styled).unwrap(), doc.node(plain).unwrap());
    assert_eq!(b.fills, a.fills, "fills");
    assert_eq!(b.stroke, a.stroke, "strokes: paints, weight, align, sides, caps, join, dashes, width profile, brush");
    assert_eq!(b.corners, a.corners, "corner radii and smoothing");
    assert_eq!(b.effects, a.effects, "effects");
    assert_eq!(b.opacity, a.opacity, "layer opacity");
    assert_eq!(b.blend, a.blend, "blend mode");
}

#[test]
fn a_vectors_anchor_radii_are_not_part_of_the_copied_style() {
    // Radii set on anchors in edit mode live in the network's vertices, not in the layer's corners:
    // copying such a vector's style and pasting it back must leave every anchor's radius as it was.
    let mut s = Session::new();
    let v = id(s.execute("node.create", &json!({"type": "rectangle", "x": 0, "y": 0, "width": 40, "height": 40})).unwrap());
    s.execute("vector.edit.begin", &json!({"id": v})).unwrap();
    s.execute("vector.edit.cornerRadius", &json!({"radius": 6, "vertices": [0, 1]})).unwrap();
    s.execute("vector.edit.end", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [v]})).unwrap();
    let radii = |s: &Session| match &s.doc().unwrap().node(v).unwrap().kind {
        NodeKind::Vector { network } => network.vertices.iter().map(|p| p.radius).collect::<Vec<_>>(),
        _ => panic!("the layer was converted to a vector"),
    };
    let before = radii(&s);
    assert_eq!(&before[..2], &[6.0, 6.0], "two anchors are rounded");
    let mut u = Ui::new(s);
    u.frame_with(vec![Event::Copy], chord());
    press_v(&mut u);
    assert_eq!(radii(&u.app.session), before, "the paste left every anchor's radius alone");
}

#[test]
fn pasting_a_style_leaves_the_geometry_alone() {
    let (mut u, _styled, plain) = scene();
    u.frame_with(vec![Event::Copy], chord());
    u.app.session.execute("select.set", &json!({"ids": [plain]})).unwrap();
    press_v(&mut u);
    let b = u.app.session.doc().unwrap().world_bounds(plain).unwrap();
    assert_eq!((b.x0, b.y0, b.width(), b.height()), (100.0, 0.0, 40.0, 20.0), "the layer keeps its place and size");
}
