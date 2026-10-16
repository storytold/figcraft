//! A duplicate's fill is its own: recolouring it, with the picker that was open on the original,
//! leaves the original as it was (#586).

use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::colorpicker::PickerTarget;
use crate::layers_drag_tests::Ui;

fn id(v: &serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().or_else(|| v["ids"][0].as_u64()).unwrap())
}

fn fill(u: &Ui, id: NodeId) -> String {
    u.app.session.doc().unwrap().node(id).unwrap().fills[0].solid_color().unwrap().to_css_hex()
}

/// A red rectangle or pen path, selected, with the colour picker of its fill open.
fn red(vector: bool) -> (Ui, NodeId) {
    let mut s = Session::new();
    let a = if vector {
        id(&s.execute("vector.create", &json!({"points": [[0, 0], [60, 0], [30, 50]], "closed": true})).unwrap())
    } else {
        id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 40})).unwrap())
    };
    s.execute("fill.set", &json!({"ids": [a], "color": "#FF0000"})).unwrap();
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    let mut u = Ui::new(s);
    u.app.ui.picker = Some(PickerTarget::Paint { fills: true, index: 0, ids: vec![a] });
    u.frame(vec![]);
    (u, a)
}

/// Duplicate the selection the way `how` names, moved clear of the original; the copy's id.
fn duplicate(u: &mut Ui, how: &str) -> NodeId {
    let copy = match how {
        "duplicate" => id(&u.app.session.execute("edit.duplicate", &json!({"dx": 100})).unwrap()),
        _ => {
            u.app.session.execute("edit.copy", &json!({})).unwrap();
            id(&u.app.session.execute("edit.paste", &json!({"x": 100, "y": 0})).unwrap())
        }
    };
    u.frame(vec![]);
    u.frame(vec![]);
    copy
}

#[test]
fn recolouring_a_duplicate_leaves_the_original() {
    for vector in [false, true] {
        for how in ["duplicate", "paste"] {
            let (mut u, a) = red(vector);
            let b = duplicate(&mut u, how);
            assert_ne!(a, b);
            assert_eq!(u.app.session.selection(), vec![b], "{how} selects the copy");
            // The picker opened for the original doesn't follow it onto the copy's edits.
            let picker = u.app.ui.picker.clone();
            assert!(
                !matches!(&picker, Some(PickerTarget::Paint { ids, .. }) if ids.contains(&a)),
                "vector {vector}, {how}: the picker still edits the original: {picker:?}"
            );
            // Recolour the selection as the fill row does.
            u.app.ui.picker = Some(PickerTarget::Paint { fills: true, index: 0, ids: u.app.session.selection() });
            u.app.session.execute("fill.set", &json!({"ids": u.app.session.selection(), "color": "#0000FF"})).unwrap();
            u.frame(vec![]);
            assert_eq!(fill(&u, b), "#0000FF", "vector {vector}, {how}");
            assert_eq!(fill(&u, a), "#FF0000", "vector {vector}, {how}: the original keeps its fill");
        }
    }
}
