//! Align and distribute the selected points in vector edit mode: the Design panel's alignment row
//! and the Alt+letter keys act on the points.

use egui::accesskit::Node;
use egui::{Event, Key, Modifiers, Pos2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

/// A four-point open path in edit mode, the first two points selected.
fn two_selected() -> (Ui, NodeId) {
    let mut s = Session::new();
    let r = s.execute("vector.create", &json!({"points": [[330, 480], [480, 430], [560, 560], [380, 620]], "closed": false})).unwrap();
    let path = NodeId(r["id"].as_u64().unwrap());
    s.execute("vector.edit.begin", &json!({"id": path})).unwrap();
    s.execute("vector.edit.select", &json!({"vertices": [0, 1]})).unwrap();
    let mut u = Ui::new(s);
    u.frame(vec![]);
    (u, path)
}

/// Page x of every point of `path`.
fn xs(u: &Ui, path: NodeId) -> Vec<f64> {
    let doc = u.app.session.doc().unwrap();
    let w = doc.world_transform(path);
    figcraft_engine::vectoredit::network(doc, path).unwrap().vertices.iter().map(|v| (w * v.point()).x).collect()
}

fn tree(u: &mut Ui) -> Vec<Node> {
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    let out = u.frame_output(vec![]);
    out.platform_output.accesskit_update.expect("an AccessKit update").nodes.into_iter().map(|(_, n)| n).collect()
}

fn button<'a>(nodes: &'a [Node], label: &str) -> Option<&'a Node> {
    nodes.iter().find(|n| n.label().is_some_and(|l| l.starts_with(label)))
}

#[test]
fn the_alignment_row_shows_for_points_and_align_left_moves_them() {
    let (mut u, path) = two_selected();
    let nodes = tree(&mut u);
    let left = button(&nodes, "Align left").expect("the alignment row shows in edit mode with points selected");
    assert!(!left.is_disabled(), "enabled whenever points are selected");
    assert!(button(&nodes, "Distribute and tidy").is_none(), "two points have nothing to distribute");
    let b = left.bounds().expect("bounds");
    let p = Pos2::new(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32);
    u.button(p, true);
    u.button(p, false);
    u.frame(vec![]);
    let x = xs(&u, path);
    assert!((x[0] - 330.0).abs() < 1e-6 && (x[1] - 330.0).abs() < 1e-6, "{x:?}");
    assert!((x[2] - 560.0).abs() < 1e-6, "unselected points stay: {x:?}");
}

#[test]
fn the_distribute_menu_shows_from_three_points() {
    let (mut u, _) = two_selected();
    u.app.session.execute("vector.edit.select", &json!({"vertices": [0, 1, 2]})).unwrap();
    let nodes = tree(&mut u);
    assert!(button(&nodes, "Distribute and tidy").is_some());
}

/// Two layers edited at once, points selected only in the one that isn't active: the row still
/// shows, since the commands act on every edited layer's selected points.
#[test]
fn the_alignment_row_shows_when_only_another_edited_layer_has_points_selected() {
    let mut s = Session::new();
    let mut make =
        |pts: serde_json::Value| NodeId(s.execute("vector.create", &json!({"points": pts, "closed": false})).unwrap()["id"].as_u64().unwrap());
    let a = make(json!([[330, 480], [480, 430], [560, 560]]));
    let b = make(json!([[700, 480], [800, 430], [900, 560]]));
    s.execute("vector.edit.begin", &json!({"ids": [a, b]})).unwrap();
    s.execute("vector.edit.select", &json!({"id": a, "vertices": [0, 1, 2]})).unwrap();
    s.execute("vector.edit.select", &json!({"id": b, "vertices": [], "add": true})).unwrap();
    let mut u = Ui::new(s);
    u.frame(vec![]);
    let nodes = tree(&mut u);
    assert!(button(&nodes, "Align left").is_some(), "the row shows for the other layer's points");
    assert!(button(&nodes, "Distribute and tidy").is_some(), "three points there: distribute too");
}

#[test]
fn alt_d_in_edit_mode_aligns_the_points_right() {
    let (mut u, path) = two_selected();
    let m = Modifiers::ALT;
    u.frame_with(
        vec![
            Event::Key { key: Key::D, physical_key: Some(Key::D), pressed: true, repeat: false, modifiers: m },
            Event::Key { key: Key::D, physical_key: Some(Key::D), pressed: false, repeat: false, modifiers: m },
        ],
        m,
    );
    let x = xs(&u, path);
    assert!((x[0] - 480.0).abs() < 1e-6 && (x[1] - 480.0).abs() < 1e-6, "{x:?}");
}
