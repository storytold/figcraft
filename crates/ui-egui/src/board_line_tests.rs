//! The bar over a board's selected connector (#1596), drawn headless: its line type, start and end
//! points, dashed or solid line and weight, and any colour through the colour picker as well as
//! the board colours; the colour wheel only when the selection is all connectors.

use egui::{Event, Modifiers, PointerButton, Pos2, Rect, pos2};
use figcraft_doc::{NodeId, Routing, StrokeCap};
use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::layers_drag_tests::Ui;
use crate::live_tests::{flat, has_text, text_center};

fn id(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn click(u: &mut Ui, p: Pos2) {
    for pressed in [true, false] {
        u.frame(vec![Event::PointerMoved(p), Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
    }
    u.frame(vec![]);
}

/// The bar over the selection, when it shows.
fn bar(u: &Ui) -> Option<Rect> {
    let layer = egui::LayerId::new(egui::Order::Middle, egui::Id::unique("board_bar"));
    u.ctx.memory(|m| m.areas().visible_last_frame(&layer)).then(|| u.ctx.memory(|m| m.area_rect(layer.id))).flatten()
}

/// A board with two stickies joined by a selected elbow connector.
fn connector() -> (Ui, NodeId, NodeId) {
    let mut s = Session::new();
    s.execute("file.newBoard", &json!({})).unwrap();
    let a = s.execute("board.sticky", &json!({"x": 0, "y": 0})).unwrap()["id"].clone();
    let b = s.execute("board.sticky", &json!({"x": 400, "y": 300})).unwrap()["id"].clone();
    let c = id(&s.execute("board.connector", &json!({"from": a, "to": b, "style": "elbow"})).unwrap());
    let mut u = Ui::new(s);
    u.frame(vec![]);
    u.frame(vec![]);
    (u, c, NodeId(b.as_u64().unwrap()))
}

/// Open the menu at `x` from the bar's left edge and pick `item` in it.
fn pick(u: &mut Ui, x: f32, item: &str) {
    let r = bar(u).expect("the bar shows");
    click(u, pos2(r.min.x + x, r.center().y));
    let shapes = flat(&u.frame_output(vec![]).shapes);
    click(u, text_center(&shapes, item));
}

// The bar's parts from its left edge: 8 board colours from 8 (24 apart) and the colour wheel
// (to 224), Straight, Elbow and Curved from 232 (30 apart), the start point (62 wide) from 328, the
// end point from 392 and the line style from 462.
const START: f32 = 328.0 + 31.0;
const END: f32 = 392.0 + 31.0;
const STYLE: f32 = 462.0 + 20.0;
const WHEEL: f32 = 200.0 + 10.0;

#[test]
fn the_connector_bar_sets_its_ends_dashes_and_weight() {
    let (mut u, c, _) = connector();
    let stroke = |u: &Ui| u.app.session.doc().unwrap().node(c).unwrap().stroke.clone();
    assert_eq!((stroke(&u).cap_start, stroke(&u).cap_end), (StrokeCap::None, StrokeCap::TriangleArrow));
    pick(&mut u, START, "Circle");
    assert_eq!(stroke(&u).cap_start, StrokeCap::CircleFilled);
    pick(&mut u, END, "Diamond");
    assert_eq!(stroke(&u).cap_end, StrokeCap::DiamondFilled);
    pick(&mut u, STYLE, "Dash");
    assert!(!stroke(&u).dashes.is_empty(), "dashed");
    pick(&mut u, STYLE, "Stroke weight 4");
    assert_eq!(stroke(&u).weight, 4.0);
    pick(&mut u, STYLE, "Solid");
    assert!(stroke(&u).dashes.is_empty(), "solid again, the weight kept");
    assert_eq!(stroke(&u).weight, 4.0);
    // The line type cells still work, and each change is a step of its own.
    let r = bar(&u).unwrap();
    click(&mut u, pos2(r.min.x + 232.0 + 14.0, r.center().y));
    let routing = |u: &Ui| match &u.app.session.doc().unwrap().node(c).unwrap().board {
        Some(figcraft_doc::BoardItem::Connector(k)) => k.routing,
        _ => panic!("not a connector"),
    };
    assert_eq!(routing(&u), Routing::Straight);
    u.app.session.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(routing(&u), Routing::Elbow);
    assert_eq!(stroke(&u).weight, 4.0, "the weight is its own step");
}

#[test]
fn the_colour_wheel_opens_the_picker_for_connectors_only() {
    let (mut u, c, b) = connector();
    let r = bar(&u).unwrap();
    click(&mut u, pos2(r.min.x + WHEEL, r.center().y));
    assert_eq!(u.app.ui.picker, Some(crate::colorpicker::PickerTarget::Paint { fills: false, index: 0, ids: vec![c] }), "the line's colour");
    crate::colorpicker::close(&mut u.app);
    // With a sticky selected too, the bar has only the board colours: no wheel, and the line type
    // cells start where the wheel was.
    u.app.session.execute("select.set", &json!({"ids": [c, b]})).unwrap();
    u.frame(vec![]);
    u.frame(vec![]);
    let shapes = flat(&u.frame_output(vec![]).shapes);
    assert!(!has_text(&shapes, "Edit colour"));
    let r = bar(&u).unwrap();
    click(&mut u, pos2(r.min.x + WHEEL, r.center().y));
    assert!(u.app.ui.picker.is_none(), "no wheel to open");
}

/// The dots a selected connector's path handles are drawn as.
fn path_dots(u: &mut Ui) -> usize {
    let shapes = flat(&u.frame_output(vec![]).shapes);
    shapes.iter().filter(|s| matches!(s, egui::Shape::Circle(c) if c.radius == 3.5 && c.fill != egui::Color32::TRANSPARENT)).count()
}

/// A selected connector shows a path handle for each run, and dragging one on the canvas moves
/// that run. Dev Mode only inspects: no handles.
#[test]
fn a_selected_connector_shows_path_handles_to_drag() {
    let (mut u, c, _) = connector();
    let handles = |u: &Ui| figcraft_engine::board::selected_connector_handles(u.app.session.doc().unwrap(), &[c], 1.0);
    let n = handles(&u).len();
    assert!(n >= 2, "{n}");
    assert_eq!(path_dots(&mut u), n);
    let (_, at) = *handles(&u).last().unwrap();
    let from = u.app.to_screen(at.x, at.y);
    u.drag_to(from, from + egui::vec2(40.0, 30.0));
    let doc = u.app.session.doc().unwrap();
    let Some(figcraft_doc::BoardItem::Connector(k)) = &doc.node(c).unwrap().board else { panic!("no connector") };
    assert!(k.path.is_some(), "the drag reshaped it");
    u.app.ui.right_tab = crate::RightTab::Inspect;
    assert_eq!(path_dots(&mut u), 0);
}
