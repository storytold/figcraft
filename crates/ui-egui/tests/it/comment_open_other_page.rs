use egui::{Pos2, RawInput, Rect, vec2};
use figcraft_engine::Session;
use figcraft_ui_egui::{FigApp, NoServices, control};
use serde_json::json;

fn frame(ctx: &egui::Context, app: &mut FigApp, time: f64) {
    let mut out = ctx
        .run_ui(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))), time: Some(time), ..Default::default() }, |ui| {
            app.ui(ui)
        });
    out.textures_delta.clear();
}

#[test]
fn comment_open_other_page() {
    let mut session = Session::new();
    let first = session.page().unwrap();
    let second = session.execute("page.add", &json!({})).unwrap()["id"].as_u64().unwrap();
    session.execute("page.select", &json!({"id": second})).unwrap();
    let comment = session.execute("comment.add", &json!({"x": 50, "y": 50, "text": "Review this"})).unwrap()["id"].as_u64().unwrap();
    let ctx = egui::Context::default();
    let mut app = FigApp::new(session, Box::new(NoServices));
    for i in 1..=3 {
        frame(&ctx, &mut app, i as f64 / 60.0);
    }
    // Same page: stays open.
    control::handle(&mut app, &ctx, "ui.set", &json!({"commentOpen": comment})).unwrap();
    frame(&ctx, &mut app, 4.0 / 60.0);
    assert_eq!(app.inspect()["comments"]["open"], json!(comment));
    // null closes it.
    control::handle(&mut app, &ctx, "ui.set", &json!({"commentOpen": null})).unwrap();
    frame(&ctx, &mut app, 5.0 / 60.0);
    assert_eq!(app.inspect()["comments"]["open"], json!(null));
    // Another page: the page is revealed and the thread stays open across a frame.
    control::handle(&mut app, &ctx, "engine.execute", &json!({"command": "page.select", "params": {"id": first}})).unwrap();
    let reply = control::handle(&mut app, &ctx, "ui.set", &json!({"commentOpen": comment})).unwrap();
    assert_eq!(reply["comments"]["open"], json!(comment));
    frame(&ctx, &mut app, 6.0 / 60.0);
    assert_eq!(app.session.page().unwrap().0, second);
    assert_eq!(app.inspect()["comments"]["open"], json!(comment), "ui.set should keep the requested popover open across a frame");
}
