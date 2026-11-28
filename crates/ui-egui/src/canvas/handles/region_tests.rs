//! The canvas paint handles with a region of a vector selected in edit mode (#1875): they read
//! and write the paint the colour picker shows, the region's own, not the layer's.

use figcraft_color::{Color, Gradient, GradientKind, Paint, PaintKind};
use serde_json::json;

use super::{gradient_target, set_gradient};
use crate::colorpicker::PickerTarget;
use crate::layers_drag_tests::Ui;

#[test]
fn the_gradient_handles_edit_a_selected_regions_own_paint() {
    let mut s = figcraft_engine::Session::new();
    let id = crate::region_fill_rows_tests::two_region_vector(&mut s);
    s.execute("vector.edit.begin", &json!({"id": id.0})).unwrap();
    s.execute("vector.edit.select", &json!({"regions": [1]})).unwrap();
    // The region's own paint: a gradient at half opacity; the layer keeps its solid grey.
    let mut paint = Paint::gradient(Gradient::default_for(GradientKind::Linear, Color::BLACK));
    paint.opacity = 0.5;
    s.execute("fill.set", &json!({"ids": [id.0], "paint": paint})).unwrap();
    let mut u = Ui::new(s);
    u.app.ui.picker = Some(PickerTarget::Paint { fills: true, index: 0, ids: vec![id] });
    let (at, fills, index, g) = gradient_target(&u.app).expect("the handles show the region's gradient");
    assert_eq!((at, fills, index), (id, true, 0));
    let mut moved = g.clone();
    moved.from = (0.1, 0.2);
    set_gradient(&mut u.app, id, true, 0, moved);
    let doc = u.app.session.doc().unwrap();
    let n = doc.node(id).unwrap();
    let figcraft_doc::NodeKind::Vector { network } = &n.kind else { panic!("not a vector") };
    let region = network.regions[1].fills.clone().unwrap();
    assert!(matches!(&region[0].kind, PaintKind::Gradient(x) if x.from == (0.1, 0.2)), "{region:?}");
    assert_eq!(region[0].opacity, 0.5, "the region's paint keeps its own opacity");
    assert!(matches!(n.fills[0].kind, PaintKind::Solid { .. }), "the layer's fill is untouched: {:?}", n.fills);
}
