//! The Design panel's Tidy up row follows the engine's rule (measured in Figma's web app on
//! 2026-11-12): offered for three or more layers forming a row, a column or a full grid, greyed out otherwise.

use egui::Context;
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::{FigApp, NoServices};

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap()["id"].as_u64().unwrap())
}

fn available(layers: &[(f64, f64, f64, f64)]) -> bool {
    let mut s = Session::new();
    let ids: Vec<NodeId> = layers.iter().map(|(x, y, w, h)| rect(&mut s, *x, *y, *w, *h)).collect();
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    let app = FigApp::new(s, Box::new(NoServices));
    let ctx = Context::default();
    let doc = app.session.ds().unwrap().doc.clone();
    crate::right::tidy_available(&ctx, &doc, &ids)
}

#[test]
fn tidy_up_is_off_in_the_panel_for_overlapping_layers() {
    assert!(available(&[(0., 0., 50., 50.), (80., 0., 50., 50.), (160., 0., 50., 50.)]), "three squares in a row");
    assert!(!available(&[(100., 460., 50., 30.), (140., 430., 80., 60.), (130., 400., 30., 90.)]), "the overlapping triple");
    assert!(!available(&[(0., 0., 50., 50.), (87., 0., 50., 50.)]), "two squares");
}
