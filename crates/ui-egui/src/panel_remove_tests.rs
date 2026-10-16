//! Removing fill, effect and layout grid rows in the Design panel, driven through egui headless: an
//! open colour picker and open settings stay with their items, and go when their item goes.

use egui::{Event, Id, Pos2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::colorpicker::PickerTarget;
use crate::layers_drag_tests::Ui;
use crate::list_drag::grip_id;

fn id(v: serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn click(u: &mut Ui, p: Pos2) {
    u.button(p, true);
    u.button(p, false);
    u.frame(vec![Event::PointerMoved(Pos2::new(1430.0, 890.0))]);
}

/// The Remove button in item `index`'s row of list `list` (found in the AccessKit tree).
fn remove_button(u: &mut Ui, list: &str, index: usize) -> Pos2 {
    // Let the layout settle (settings just opened).
    for _ in 0..5 {
        u.frame(vec![]);
    }
    // Scroll the panel until the row is in view.
    for _ in 0..20 {
        let row = u.ctx.read_response(grip_id(list, index)).unwrap().rect;
        if row.max.y < 860.0 {
            break;
        }
        let wheel = Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -100.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
        };
        u.frame(vec![Event::PointerMoved(Pos2::new(1300.0, 500.0)), wheel]);
        // Let the scroll settle.
        for _ in 0..30 {
            u.frame(vec![]);
        }
    }
    let out = u.frame_output(vec![]);
    let row = u.ctx.read_response(grip_id(list, index)).unwrap().rect;
    let all: Vec<Pos2> = out
        .platform_output
        .accesskit_update
        .unwrap()
        .nodes
        .into_iter()
        .filter(|(_, n)| n.label() == Some("Remove"))
        .filter_map(|(_, n)| n.bounds())
        .map(|b| Pos2::new(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32))
        .collect();
    all.iter()
        .copied()
        .find(|p| p.y > row.min.y && p.y < row.min.y + 32.0)
        .unwrap_or_else(|| panic!("no Remove button in {list} row {index} {row:?} {all:?}"))
}

fn flag(u: &Ui, salt: &'static str, i: usize) -> bool {
    u.ctx.data(|d| d.get_temp::<bool>(Id::new((salt, i)))).unwrap_or(false)
}

#[test]
fn removing_a_fill_keeps_the_picker_on_its_paint() {
    let mut s = Session::new();
    let r = id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap());
    s.execute("fill.set", &json!({"ids": [r], "color": "#FF0000"})).unwrap();
    s.execute("fill.add", &json!({"ids": [r], "color": "#00FF00"})).unwrap();
    s.execute("fill.add", &json!({"ids": [r], "color": "#0000FF"})).unwrap();
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    let mut u = Ui::new(s);
    u.ctx.enable_accesskit();
    // Green's swatch, then red's Remove: the picker follows green to index 0.
    let sw = u.ctx.read_response(Id::new(("swatch", true, 1usize))).unwrap().rect.center();
    click(&mut u, sw);
    assert_eq!(u.app.ui.picker, Some(PickerTarget::Paint { fills: true, index: 1, ids: vec![r] }));
    let rm = remove_button(&mut u, "fills", 0);
    click(&mut u, rm);
    let fills: Vec<String> = u.app.session.doc().unwrap().node(r).unwrap().fills.iter().filter_map(|p| p.solid_color()).map(|c| c.to_hex()).collect();
    assert_eq!(fills, ["00FF00", "0000FF"]);
    assert_eq!(u.app.ui.picker, Some(PickerTarget::Paint { fills: true, index: 0, ids: vec![r] }));
    // Removing the paint being edited closes the picker.
    let rm = remove_button(&mut u, "fills", 0);
    click(&mut u, rm);
    assert_eq!(u.app.ui.picker, None);
}

#[test]
fn removing_effects_and_grids_keeps_open_settings_with_their_rows() {
    let mut s = Session::new();
    let f = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    for _ in 0..3 {
        s.execute("effect.add", &json!({"ids": [f], "type": "dropShadow"})).unwrap();
    }
    s.execute("grid.add", &json!({"ids": [f], "grid": {"kind": "columns"}})).unwrap();
    s.execute("grid.add", &json!({"ids": [f], "grid": {"kind": "rows"}})).unwrap();
    s.execute("select.set", &json!({"ids": [f]})).unwrap();
    let mut u = Ui::new(s);
    u.ctx.enable_accesskit();
    // Effect 2's settings open and its colour in the picker; effect 0 goes.
    u.ctx.data_mut(|d| d.insert_temp(Id::new(("fx_open", 2usize)), true));
    u.app.ui.picker = Some(PickerTarget::Effect { ids: vec![f], index: 2, secondary: false });
    let rm = remove_button(&mut u, "effects", 0);
    click(&mut u, rm);
    assert_eq!(u.app.session.doc().unwrap().node(f).unwrap().effects.len(), 2);
    assert!(flag(&u, "fx_open", 1) && !flag(&u, "fx_open", 2));
    assert_eq!(u.app.ui.picker, Some(PickerTarget::Effect { ids: vec![f], index: 1, secondary: false }));
    // Removing the open effect closes its settings and the picker.
    let rm = remove_button(&mut u, "effects", 1);
    click(&mut u, rm);
    assert!(!flag(&u, "fx_open", 0) && !flag(&u, "fx_open", 1));
    assert_eq!(u.app.ui.picker, None);
    // Grid 1's settings open; grid 0 goes.
    u.ctx.data_mut(|d| d.insert_temp(Id::new(("grid_open", 1usize)), true));
    let rm = remove_button(&mut u, "grids", 0);
    click(&mut u, rm);
    assert_eq!(u.app.session.doc().unwrap().node(f).unwrap().grids.len(), 1);
    assert!(flag(&u, "grid_open", 0) && !flag(&u, "grid_open", 1));
}
