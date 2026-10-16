#![allow(clippy::unwrap_used)]

use egui::{Pos2, RawInput, Rect, Vec2, vec2};
use figcraft_engine::Session;
use figcraft_ui_egui::{FigApp, NoServices};
use serde_json::json;

fn frame(ctx: &egui::Context, app: &mut FigApp, size: Vec2) {
    let mut out = ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), ..Default::default() }, |ui| app.ui(ui));
    out.textures_delta.clear();
}

fn scene(selected: bool, rulers: bool) -> (egui::Context, FigApp) {
    let mut session = Session::new();
    session.execute("view.rulers", &json!({"on": rulers})).unwrap();
    session.execute("guide.add", &json!({"vertical": true, "offset": 100.0})).unwrap();
    if selected {
        session.execute("guide.select", &json!({"index": 0})).unwrap();
    }
    let ctx = egui::Context::default();
    let mut app = FigApp::new(session, Box::new(NoServices));
    frame(&ctx, &mut app, vec2(1440.0, 900.0));
    frame(&ctx, &mut app, vec2(1440.0, 900.0));
    (ctx, app)
}

#[test]
fn selected_guide_frame_completes_in_narrow_layout() {
    for (selected, rulers, width) in [(false, true, 200.0), (true, false, 200.0), (true, true, 800.0), (true, true, 200.0)] {
        let (ctx, mut app) = scene(selected, rulers);
        frame(&ctx, &mut app, vec2(width, 200.0));
    }
}
