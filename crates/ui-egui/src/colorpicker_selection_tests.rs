//! The colour picker of a layer's paint or effect closes when that layer is no longer the
//! selection, so it never keeps editing a layer that was deselected (#396).

use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::colorpicker::PickerTarget;
use crate::layers_drag_tests::Ui;

fn rect(s: &mut Session, x: f64) -> NodeId {
    let v = s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 20, "height": 20})).unwrap();
    NodeId(v["id"].as_u64().unwrap())
}

fn two() -> (Ui, NodeId, NodeId) {
    let mut s = Session::new();
    let a = rect(&mut s, 0.0);
    let b = rect(&mut s, 40.0);
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    (Ui::new(s), a, b)
}

#[test]
fn picker_closes_when_its_layer_is_deselected() {
    for effect in [false, true] {
        let (mut u, a, _) = two();
        let target = if effect {
            u.app.run("effect.add", json!({"ids": [a], "type": "dropShadow"}));
            PickerTarget::Effect { ids: vec![a], index: 0, secondary: false }
        } else {
            PickerTarget::Paint { fills: true, index: 0, ids: vec![a] }
        };
        u.app.ui.picker = Some(target.clone());
        u.frame(vec![]);
        u.frame(vec![]);
        assert_eq!(u.app.ui.picker, Some(target.clone()), "the picker opens while its layer is selected");
        u.app.run("select.none", json!({}));
        u.frame(vec![]);
        u.frame(vec![]);
        assert!(u.app.ui.picker.is_none(), "deselecting closes {target:?}");
    }
}

#[test]
fn picker_closes_when_another_layer_is_selected() {
    let (mut u, a, b) = two();
    u.app.ui.picker = Some(PickerTarget::Paint { fills: true, index: 0, ids: vec![a] });
    u.frame(vec![]);
    u.app.run("select.set", json!({"ids": [b]}));
    u.frame(vec![]);
    u.frame(vec![]);
    assert!(u.app.ui.picker.is_none(), "selecting another layer closes the picker: {:?}", u.app.ui.picker);
}
