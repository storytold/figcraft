//! The placement crosshair: the arrows with a shape or frame tool chosen show it in the middle of
//! the view and move it, and Enter places the object there ("Use the arrow keys to activate the
//! crosshair cursor. It will appear in the middle of the viewport indicating where the new object
//! will be placed." / "Use the arrow keys to move the crosshair around the canvas or board, then
//! press Return/Enter to place the object.", Accessibility at Figma, #1839).

use serde_json::json;

use crate::Session;
use crate::tools::{PointerEvent, PointerKind};

#[test]
fn the_arrows_show_and_move_the_crosshair_and_enter_places_there() {
    let mut s = Session::new();
    s.execute("view.set", &json!({"zoom": 1.0, "cx": 300, "cy": 200})).unwrap();
    assert!(s.execute("tool.crosshair", &json!({"dx": 1})).is_err(), "a shape or frame tool first");
    s.execute("tool.rectangle", &json!({})).unwrap();
    // The first arrow shows the crosshair in the middle of the view, unmoved; the next move it by
    // the nudge amounts.
    let r = s.execute("tool.crosshair", &json!({"dx": 1, "dy": 0})).unwrap();
    assert_eq!((r["x"].as_f64(), r["y"].as_f64()), (Some(300.0), Some(200.0)));
    s.execute("tool.crosshair", &json!({"dx": 1, "dy": 0})).unwrap();
    let r = s.execute("tool.crosshair", &json!({"dx": 0, "dy": -1, "big": true})).unwrap();
    assert_eq!((r["x"].as_f64(), r["y"].as_f64()), (Some(301.0), Some(190.0)), "a nudge, then a big nudge");
    assert_eq!(s.tool_info()["crosshair"], json!([301.0, 190.0]));
    let v = &s.ds().unwrap().view;
    assert_eq!((v.cx, v.cy), (300.0, 200.0), "the view hasn't panned");
    // Enter: the rectangle is centred on the crosshair, which goes.
    let placed = s.execute("tool.place", &json!({})).unwrap();
    let id = crate::params::id_of(&placed["ids"][0]).unwrap();
    let b = s.doc().unwrap().world_bounds(id).unwrap();
    assert_eq!((b.center().x, b.center().y), (301.0, 190.0));
    assert_eq!(s.tool.place_at, None);
    assert!(s.tool_info()["crosshair"].is_null());
    // Escape takes a shown crosshair away before anything else, the tool staying; a tool change
    // takes it away too.
    s.execute("select.set", &json!({"ids": []})).unwrap();
    s.execute("tool.rectangle", &json!({})).unwrap();
    s.execute("tool.crosshair", &json!({})).unwrap();
    assert!(s.tool.place_at.is_some());
    s.escape().unwrap();
    assert_eq!((s.tool.place_at, s.tool.tool), (None, crate::ToolId::Rectangle));
    s.execute("tool.crosshair", &json!({})).unwrap();
    s.execute("tool.ellipse", &json!({})).unwrap();
    assert_eq!(s.tool.place_at, None);
}

#[test]
fn the_crosshair_keeps_its_page_and_goes_with_a_pointer_placement_but_not_the_same_tool_again() {
    let mut s = Session::new();
    s.execute("view.set", &json!({"zoom": 1.0, "cx": 300, "cy": 200})).unwrap();
    s.execute("tool.rectangle", &json!({})).unwrap();
    s.execute("tool.crosshair", &json!({"dx": 1})).unwrap();
    s.execute("tool.crosshair", &json!({"dx": 1})).unwrap();
    // The tool already chosen named again keeps the crosshair: `tool.place {tool}` lands on it.
    let placed = s.execute("tool.place", &json!({"tool": "rectangle"})).unwrap();
    let id = crate::params::id_of(&placed["ids"][0]).unwrap();
    assert_eq!(s.doc().unwrap().world_bounds(id).unwrap().center().x, 301.0);
    // A click places too (the tool kept by the preference) and takes the crosshair with it.
    s.prefs.keep_tool = true;
    s.execute("select.set", &json!({"ids": []})).unwrap();
    s.execute("tool.rectangle", &json!({})).unwrap();
    s.execute("tool.crosshair", &json!({})).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Down, 600.0, 600.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, 600.0, 600.0, 1.0)).unwrap();
    assert_eq!((s.tool.tool, s.tool.place_at), (crate::ToolId::Rectangle, None), "placed by the pointer: the crosshair goes, the tool stays");
    // Another page in front: the crosshair shown on the first isn't there, Escape has nothing to
    // take, and the next arrow starts over in the middle of this page's view.
    s.execute("select.set", &json!({"ids": []})).unwrap();
    s.execute("tool.crosshair", &json!({"dx": 5})).unwrap();
    s.execute("page.add", &json!({})).unwrap();
    s.execute("page.select", &json!({"index": 1})).unwrap();
    s.execute("tool.rectangle", &json!({})).unwrap();
    assert_eq!(s.crosshair(), None);
    assert!(s.tool_info()["crosshair"].is_null());
    s.escape().unwrap();
    assert_eq!(s.tool.tool, crate::ToolId::Move, "Escape went on to the tool");
    s.execute("tool.rectangle", &json!({})).unwrap();
    let r = s.execute("tool.crosshair", &json!({"dx": 1})).unwrap();
    let v = &s.ds().unwrap().view;
    assert_eq!((r["x"].as_f64(), r["y"].as_f64()), (Some(v.cx), Some(v.cy)), "shown afresh in the middle of this page's view");
}

#[test]
fn the_view_pans_to_keep_the_crosshair_in_sight() {
    // "Use the arrow keys to move the crosshair around the canvas or board": nudged past an edge
    // of the view, the view follows it by the overshoot; inside it, the view stays put.
    let mut s = Session::new();
    s.execute("view.set", &json!({"zoom": 10.0, "cx": 300, "cy": 200})).unwrap();
    s.execute("tool.rectangle", &json!({})).unwrap();
    s.execute("tool.crosshair", &json!({})).unwrap();
    for _ in 0..6 {
        s.execute("tool.crosshair", &json!({"dx": 1, "big": true})).unwrap();
    }
    let v = s.ds().unwrap().view.clone();
    assert_eq!((s.tool.place_at.map(|p| p.x), v.cx, v.cy), (Some(360.0), 300.0, 200.0), "at the edge of the 120 px wide view: no pan yet");
    s.execute("tool.crosshair", &json!({"dx": 1, "big": true})).unwrap();
    let v = s.ds().unwrap().view.clone();
    assert_eq!((v.cx, v.cy), (310.0, 200.0), "ten past the edge: the view moved ten");
    for _ in 0..5 {
        s.execute("tool.crosshair", &json!({"dy": -1, "big": true})).unwrap();
    }
    let v = s.ds().unwrap().view.clone();
    assert_eq!((v.cx, v.cy), (310.0, 190.0), "the view is 80 px tall: 40 up is its edge, 50 moves it ten");
}

#[test]
fn the_board_shapes_tool_places_on_the_crosshair_too() {
    // "...move the crosshair around the canvas or board, then press Return/Enter to place the
    // object": the board Shapes tool takes the crosshair like the shape tools, its shape centred
    // on it (the tool centres its own 160 px shape on a press).
    let mut s = crate::board_tests::board();
    s.execute("view.set", &json!({"zoom": 1.0, "cx": 300, "cy": 200})).unwrap();
    s.execute("tool.set", &json!({"tool": "shape"})).unwrap();
    s.execute("tool.crosshair", &json!({})).unwrap();
    s.execute("tool.crosshair", &json!({"dx": 2})).unwrap();
    let placed = s.execute("tool.place", &json!({})).unwrap();
    let id = crate::params::id_of(&placed["ids"][0]).unwrap();
    let b = s.doc().unwrap().world_bounds(id).unwrap();
    assert_eq!((b.center().x, b.center().y, b.width(), b.height()), (302.0, 200.0, 160.0, 160.0));
    assert_eq!(s.tool.place_at, None);
}
