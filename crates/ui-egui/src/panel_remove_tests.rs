//! Adding and removing fill, effect and layout grid rows in the Design panel, driven through egui headless:
//! adding a paint opens its colour picker beside the sidebar; outside presses dismiss it, while
//! open effect and grid settings stay with their items and go when their item goes.

use egui::{Event, Id, Pos2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::colorpicker::PickerTarget;
use crate::layers_drag_tests::Ui;
use crate::list_drag::grip_id;
use crate::live_tests::{flat, text_center};

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
            source: egui::MouseWheelSource::Unknown,
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

/// Read a temporary boolean open-settings flag keyed by `salt` and row index.
fn flag(u: &Ui, salt: &'static str, i: usize) -> bool {
    u.ctx.data(|d| d.get_temp::<bool>(Id::unique((salt, i)))).unwrap_or(false)
}

/// Removing a fill acts on the paint and dismisses the picker on the outside press.
#[test]
fn removing_a_fill_dismisses_the_picker() {
    let mut s = Session::new();
    let r = id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap());
    s.execute("fill.set", &json!({"ids": [r], "color": "#FF0000"})).unwrap();
    s.execute("fill.add", &json!({"ids": [r], "color": "#00FF00"})).unwrap();
    s.execute("fill.add", &json!({"ids": [r], "color": "#0000FF"})).unwrap();
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    let mut u = Ui::new(s);
    u.ctx.enable_accesskit();
    // Green's swatch, then red's Remove: the outside press dismisses the picker.
    let sw = u.ctx.read_response(Id::unique(("swatch", true, 1usize))).unwrap().rect.center();
    click(&mut u, sw);
    assert_eq!(u.app.ui.picker, Some(PickerTarget::Paint { fills: true, index: 1, ids: vec![r] }));
    let rm = remove_button(&mut u, "fills", 0);
    click(&mut u, rm);
    let fills: Vec<String> = u.app.session.doc().unwrap().node(r).unwrap().fills.iter().filter_map(|p| p.solid_color()).map(|c| c.to_hex()).collect();
    assert_eq!(fills, ["00FF00", "0000FF"]);
    assert_eq!(u.app.ui.picker, None);
    // Removing the surviving paint works with the picker dismissed.
    let rm = remove_button(&mut u, "fills", 0);
    click(&mut u, rm);
    assert_eq!(u.app.ui.picker, None);
}

/// Removing effects or grids moves open settings to the surviving row; the outside press
/// dismisses the colour picker. Removing the open item closes its settings.
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
    u.ctx.data_mut(|d| d.insert_temp(Id::unique(("fx_open", 2usize)), true));
    u.app.ui.picker = Some(PickerTarget::Effect { ids: vec![f], index: 2, secondary: false });
    let rm = remove_button(&mut u, "effects", 0);
    click(&mut u, rm);
    assert_eq!(u.app.session.doc().unwrap().node(f).unwrap().effects.len(), 2);
    assert!(flag(&u, "fx_open", 1) && !flag(&u, "fx_open", 2));
    assert_eq!(u.app.ui.picker, None);
    // Removing the open effect closes its settings and the picker.
    let rm = remove_button(&mut u, "effects", 1);
    click(&mut u, rm);
    assert!(!flag(&u, "fx_open", 0) && !flag(&u, "fx_open", 1));
    assert_eq!(u.app.ui.picker, None);
    // Grid 1's settings open; grid 0 goes.
    u.ctx.data_mut(|d| d.insert_temp(Id::unique(("grid_open", 1usize)), true));
    let rm = remove_button(&mut u, "grids", 0);
    click(&mut u, rm);
    assert_eq!(u.app.session.doc().unwrap().node(f).unwrap().grids.len(), 1);
    assert!(flag(&u, "grid_open", 0) && !flag(&u, "grid_open", 1));
}

/// The centre of the button labelled `label` (found in the AccessKit tree).
fn button(u: &mut Ui, label: &str) -> Pos2 {
    let out = u.frame_output(vec![]);
    let b = out.platform_output.accesskit_update.unwrap().nodes.into_iter().find(|(_, n)| n.label() == Some(label)).and_then(|(_, n)| n.bounds());
    let b = b.unwrap_or_else(|| panic!("no {label:?} button"));
    Pos2::new(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)
}

#[test]
fn adding_a_fill_or_stroke_opens_the_color_picker() {
    let mut s = Session::new();
    let r = id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap());
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    let mut u = Ui::new(s);
    u.ctx.enable_accesskit();
    assert_eq!(u.app.ui.picker, None);

    let add_fill = button(&mut u, "Add fill");
    click(&mut u, add_fill);
    assert_eq!(u.app.ui.picker, Some(PickerTarget::Paint { fills: true, index: 1, ids: vec![r] }));

    let add_stroke = button(&mut u, "Add stroke");
    click(&mut u, add_stroke);
    assert_eq!(u.app.ui.picker, Some(PickerTarget::Paint { fills: false, index: 0, ids: vec![r] }));
}

#[test]
fn an_empty_sections_header_adds_its_item_but_a_full_one_only_adds_from_the_plus() {
    let mut s = Session::new();
    let r = id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap());
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    let mut u = Ui::new(s);
    // Nothing is stroked yet: clicking anywhere on the Stroke header adds one.
    let shapes = flat(&u.frame_output(vec![]).shapes);
    click(&mut u, text_center(&shapes, "Stroke"));
    assert_eq!(u.app.session.doc().unwrap().node(r).unwrap().strokes.len(), 1);
    // It has a stroke now: the header is no longer a button of its own.
    u.app.ui.picker = None;
    u.frame(vec![]);
    let shapes = flat(&u.frame_output(vec![]).shapes);
    click(&mut u, text_center(&shapes, "Stroke"));
    assert_eq!(u.app.session.doc().unwrap().node(r).unwrap().strokes.len(), 1);
}

#[test]
fn adding_a_fill_to_mixed_fills_opens_no_picker() {
    let mut s = Session::new();
    let a = id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap());
    let b = id(s.execute("shape.rectangle", &json!({"x": 60, "y": 0, "width": 50, "height": 50})).unwrap());
    s.execute("fill.add", &json!({"ids": [b]})).unwrap();
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let mut u = Ui::new(s);
    u.ctx.enable_accesskit();

    let add_fill = button(&mut u, "Add fill");
    click(&mut u, add_fill);
    let doc = u.app.session.doc().unwrap();
    assert_eq!((doc.node(a).unwrap().fills.len(), doc.node(b).unwrap().fills.len()), (2, 3));
    assert_eq!(u.app.ui.picker, None);
}

/// The shared popover's closing reaches the colour picker: a press on the canvas closes it.
#[test]
fn a_press_on_the_canvas_closes_the_colour_picker() {
    let mut s = Session::new();
    let r = id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap());
    s.execute("fill.add", &json!({"ids": [r], "color": "#FF0000"})).unwrap();
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    let mut u = Ui::new(s);
    u.ctx.enable_accesskit();
    let sw = u.ctx.read_response(Id::unique(("swatch", true, 0usize))).unwrap().rect.center();
    click(&mut u, sw);
    assert!(u.app.ui.picker.is_some(), "the swatch opens the picker");
    click(&mut u, Pos2::new(320.0, 300.0));
    assert_eq!(u.app.ui.picker, None, "a press on the canvas closes the picker");
}

/// Pressing the swatch that opened the picker again closes it: its press is held out of the
/// popover's outside-press closing, so the swatch's own toggle gets the click.
#[test]
fn the_swatch_that_opened_the_colour_picker_closes_it_again() {
    let mut s = Session::new();
    let r = id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap());
    s.execute("fill.add", &json!({"ids": [r], "color": "#FF0000"})).unwrap();
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    let mut u = Ui::new(s);
    u.ctx.enable_accesskit();
    let sw = u.ctx.read_response(Id::unique(("swatch", true, 0usize))).unwrap().rect.center();
    click(&mut u, sw);
    assert!(u.app.ui.picker.is_some(), "the swatch opens the picker");
    click(&mut u, sw);
    assert_eq!(u.app.ui.picker, None, "the same swatch closes it again");
}

/// Blank space in the sidebar is outside the picker too, including after adding a paint.
#[test]
fn added_paints_open_beside_the_sidebar_and_close_on_a_sidebar_press() {
    for (label, fills) in [("Add fill", true), ("Add stroke", false)] {
        let mut s = Session::new();
        let r = id(s.execute("shape.rectangle", &json!({"width": 50, "height": 50})).unwrap());
        s.execute("select.set", &json!({"ids": [r]})).unwrap();
        let mut u = Ui::new(s);
        u.ctx.enable_accesskit();
        let add = button(&mut u, label);
        click(&mut u, add);
        for _ in 0..3 {
            u.frame(vec![]);
        }
        assert!(matches!(u.app.ui.picker, Some(PickerTarget::Paint { fills: f, .. }) if f == fills));
        let pop = u.app.ui.picker_popover.as_ref().unwrap();
        let sidebar_left = u.app.ui.right_panel_rect.unwrap().min.x;
        assert!((pop.rect.max.x - (sidebar_left - 8.0)).abs() < 1.0, "{:?}", pop.rect);
        assert!(pop.rect.min.y <= add.y, "the picker aligns with its opener or lifts to fit");
        assert!(pop.rect.max.y <= 900.0 - 8.0);
        u.button(Pos2::new(1420.0, 850.0), true);
        assert!(u.app.ui.picker.is_none(), "a sidebar press dismisses the picker before release");
    }
}

#[test]
fn page_colour_picker_closes_on_an_outside_press_and_reopens_at_its_control() {
    let mut u = Ui::new(Session::new());
    let button = egui::Rect::from_min_size(Pos2::new(1230.0, 220.0), egui::vec2(16.0, 16.0));
    u.app.toggle_picker(PickerTarget::PageBackground, Some(button));
    for _ in 0..3 {
        u.frame(vec![]);
    }
    let old = u.app.ui.picker_popover.take().unwrap().at(Pos2::new(300.0, 100.0));
    u.app.ui.picker_popover = Some(old);
    u.frame(vec![]);
    u.button(Pos2::new(1420.0, 850.0), true);
    assert!(u.app.ui.picker.is_none());
    u.button(Pos2::new(1420.0, 850.0), false);
    u.app.toggle_picker(PickerTarget::PageBackground, Some(button));
    for _ in 0..3 {
        u.frame(vec![]);
    }
    let rect = u.app.ui.picker_popover.as_ref().unwrap().rect;
    assert!((rect.max.x - (u.app.ui.right_panel_rect.unwrap().min.x - 8.0)).abs() < 1.0);
    assert!((rect.min.y - u.app.ui.picker_button.unwrap().min.y).abs() < 1.0);
}
