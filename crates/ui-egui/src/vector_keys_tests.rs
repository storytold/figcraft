//! Keys in vector edit mode that the window's key handling twists: Windows sends Shift+Delete
//! (delete the points and heal the path) as a cut command and no key at all, so the canvas must
//! heal the selected points rather than cut the layer.

use egui::{Event, Modifiers};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

fn id(v: serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn net(u: &Ui, path: NodeId) -> figcraft_doc::VectorNetwork {
    let doc = u.app.session.doc().unwrap();
    figcraft_engine::vectoredit::network(doc, path).unwrap().clone()
}

/// A three-point open path A-B-C in edit mode, B selected.
fn abc(u: &mut Ui) -> NodeId {
    let path = id(u.app.session.execute("vector.create", &json!({"points": [[100, 100], [200, 100], [300, 100]], "closed": false})).unwrap());
    u.app.session.execute("vector.edit.begin", &json!({"id": path})).unwrap();
    u.app.session.execute("vector.edit.select", &json!({"vertices": [1]})).unwrap();
    path
}

#[test]
fn shift_delete_in_vector_edit_mode_heals_the_point_instead_of_cutting_the_layer() {
    let mut u = Ui::new(Session::new());
    let path = abc(&mut u);
    // Windows: egui-winit turns Shift+Delete into a cut command, with no key event.
    u.frame_with(vec![Event::Cut], Modifiers::SHIFT);
    assert!(u.app.session.doc().unwrap().node(path).is_some(), "the layer stays");
    let n = net(&u, path);
    assert_eq!(n.vertices.len(), 2, "the point is gone: {n:?}");
    assert_eq!(n.segments.len(), 1, "A and C are joined");
    let (a, b) = (n.vertices.first().map(|v| v.x).unwrap_or_default(), n.vertices.get(1).map(|v| v.x).unwrap_or_default());
    assert!(b - a == 200.0 && n.vertices.iter().all(|v| v.y == 0.0), "it runs straight from A to C: {n:?}");
}

#[test]
fn a_cut_command_in_edit_mode_takes_the_selected_points_not_the_layer() {
    let mut u = Ui::new(Session::new());
    let path = abc(&mut u);
    u.frame_with(vec![Event::Cut], Modifiers::COMMAND);
    assert!(u.app.session.doc().unwrap().node(path).is_some(), "Ctrl+X on points keeps the layer");
    assert_eq!(net(&u, path).vertices.len(), 2, "it takes the selected point");
}

#[test]
fn a_cut_command_in_edit_mode_with_nothing_selected_keeps_the_layer() {
    let mut u = Ui::new(Session::new());
    let path = abc(&mut u);
    u.app.session.execute("vector.edit.select", &json!({"vertices": []})).unwrap();
    u.frame_with(vec![Event::Cut], Modifiers::SHIFT);
    assert!(u.app.session.doc().unwrap().node(path).is_some(), "nothing selected: nothing goes");
    assert_eq!(net(&u, path).vertices.len(), 3);
}

#[test]
fn a_cut_command_heals_points_selected_in_another_edited_layer() {
    let mut u = Ui::new(Session::new());
    let a = id(u.app.session.execute("vector.create", &json!({"points": [[100, 100], [200, 100], [300, 100]], "closed": false})).unwrap());
    let b = id(u.app.session.execute("vector.create", &json!({"points": [[100, 200], [200, 200], [300, 200]], "closed": false})).unwrap());
    figcraft_engine::vectoredit::begin_many(&mut u.app.session, &[a, b]).unwrap();
    // Points selected in the second layer, then the first made active: only `others` has a selection.
    u.app.session.execute("vector.edit.select", &json!({"id": b, "vertices": [1]})).unwrap();
    u.app.session.execute("vector.edit.select", &json!({"id": a, "add": true})).unwrap();
    u.frame_with(vec![Event::Cut], Modifiers::SHIFT);
    assert!(u.app.session.doc().unwrap().node(a).is_some() && u.app.session.doc().unwrap().node(b).is_some(), "both layers stay");
    assert_eq!(net(&u, a).vertices.len(), 3, "the active layer had nothing selected");
    assert_eq!(net(&u, b).vertices.len(), 2, "the other layer's point is gone");
}

#[test]
fn a_cut_command_outside_edit_mode_cuts_the_layer() {
    let mut s = Session::new();
    let path = id(s.execute("vector.create", &json!({"points": [[100, 100], [200, 100], [300, 100]], "closed": false})).unwrap());
    s.execute("select.set", &json!({"ids": [path]})).unwrap();
    let mut u = Ui::new(s);
    u.frame_with(vec![Event::Cut], Modifiers::SHIFT);
    assert!(u.app.session.doc().unwrap().node(path).is_none(), "outside edit mode the selected layer is cut");
}
