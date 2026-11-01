//! The eyedropper of a colour picker, driven through egui headless: the colour it samples goes to
//! the paint the picker edits (a stroke, an effect), not to the layer's fill (vectorcraft#3342).

use egui::accesskit::Role;
use egui::{Event, Pos2, pos2};
use figcraft_doc::NodeId;
use figcraft_engine::{Session, ToolId};
use serde_json::json;

use crate::colorpicker::PickerTarget;
use crate::layers_drag_tests::Ui;

fn rect(s: &mut Session, x: f64, props: serde_json::Value) -> NodeId {
    let v = s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 60, "height": 60, "props": props})).unwrap();
    NodeId(v["id"].as_u64().unwrap())
}

/// The middle of the picker's eyedropper button.
fn eyedropper_button(u: &mut Ui) -> Pos2 {
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    let out = u.frame_output(vec![]);
    let nodes = out.platform_output.accesskit_update.expect("an AccessKit update").nodes;
    let n = nodes.iter().map(|(_, n)| n).find(|n| n.role() == Role::Button && n.label() == Some("Pick color (I)")).expect("the eyedropper button");
    let b = n.bounds().expect("bounds");
    pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)
}

fn click(u: &mut Ui, p: Pos2) {
    u.button(p, true);
    u.button(p, false);
    u.frame(vec![Event::PointerMoved(p)]);
    u.frame(vec![]);
}

#[test]
fn the_pickers_eyedropper_applies_the_colour_to_the_paint_it_edits() {
    for effect in [false, true] {
        // A red layer to sample, and a green one with a blue stroke (and a shadow) to edit.
        let mut s = Session::new();
        s.execute("page.setBackground", &json!({"color": "#FFFFFF"})).unwrap();
        let red = rect(&mut s, 0.0, json!({"fill": "#FF0000"}));
        let b = rect(&mut s, 200.0, json!({"fill": "#00FF00", "stroke": "#0000FF", "strokeWeight": 2}));
        s.execute("select.set", &json!({"ids": [b]})).unwrap();
        let target = if effect {
            s.execute("effect.add", &json!({"ids": [b], "type": "dropShadow"})).unwrap();
            PickerTarget::Effect { ids: vec![b], index: 0, secondary: false }
        } else {
            PickerTarget::Paint { fills: false, index: 0, ids: vec![b] }
        };
        let mut u = Ui::new(s);
        u.app.ui.picker = Some(target.clone());
        let at = eyedropper_button(&mut u);
        click(&mut u, at);
        assert_eq!(u.app.session.tool.tool, ToolId::Eyedropper, "the button picks up the eyedropper");
        let undo = u.app.session.undo_depth().0;
        let on_red = u.app.to_screen(30.0, 30.0);
        click(&mut u, on_red);
        let doc = u.app.session.doc().unwrap();
        let n = doc.node(b).unwrap();
        let hex = |p: Option<figcraft_color::Color>| p.map(|c| c.to_hex()).unwrap_or_default();
        let picked = if effect { hex(n.effects[0].color(false)) } else { hex(n.strokes[0].solid_color()) };
        assert_eq!(picked, "FF0000", "the picker's paint takes the colour (effect: {effect})");
        assert_eq!(hex(n.fills[0].solid_color()), "00FF00", "the fill is left alone (effect: {effect})");
        assert!(doc.node(red).is_some());
        assert_eq!(u.app.session.tool.tool, ToolId::Move, "picking ends the eyedropper");
        assert_eq!(u.app.ui.picker, Some(target), "the picker stays open on its paint");
        assert_eq!(u.app.session.undo_depth().0, undo + 1, "one undo step");
    }
}

#[test]
fn closing_the_picker_drops_its_eyedropper() {
    // The colour sampled after the picker closed has no paint to go to: it must not wait for the
    // next picker, and the eyedropper gives way to the Move tool.
    let mut s = Session::new();
    let b = rect(&mut s, 200.0, json!({"fill": "#00FF00", "stroke": "#0000FF"}));
    s.execute("select.set", &json!({"ids": [b]})).unwrap();
    let mut u = Ui::new(s);
    u.app.ui.picker = Some(PickerTarget::Paint { fills: false, index: 0, ids: vec![b] });
    let at = eyedropper_button(&mut u);
    click(&mut u, at);
    assert_eq!(u.app.session.tool.tool, ToolId::Eyedropper);
    u.app.ui.picker = None;
    u.app.session.tool.picked = figcraft_color::Color::from_hex("#FF0000");
    u.frame(vec![]);
    u.frame(vec![]);
    assert_eq!(u.app.session.tool.tool, ToolId::Move);
    assert_eq!(u.app.session.tool.picked, None, "a colour sampled for a closed picker is dropped");
}
