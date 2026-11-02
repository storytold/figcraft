//! The placement crosshair: the arrows with a shape or frame tool chosen show it in the middle of
//! the view and move it, and Enter places the object there ("Use the arrow keys to activate the
//! crosshair cursor. It will appear in the middle of the viewport indicating where the new object
//! will be placed." / "Use the arrow keys to move the crosshair around the canvas or board, then
//! press Return/Enter to place the object.", Accessibility at Figma, #1839).

use serde_json::json;

use crate::Session;

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
