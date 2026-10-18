//! The Export section's Preview row: a slice paints nothing itself, so its preview must show the
//! page content under it — the same thing exporting the slice writes — and not a blank
//! transparency checkerboard next to a correct export. And its setting rows: a size only for
//! images.

use figcraft_doc::{Document, NodeId};
use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::assets_panel::thumbnail_pixmap;

/// Run `commands`, returning the document and the id each one made.
fn doc_after(commands: &[(&str, Value)]) -> (Document, Vec<NodeId>) {
    let mut s = Session::new();
    let ids = commands.iter().map(|(cmd, params)| NodeId(s.execute(cmd, params).unwrap()["id"].as_u64().unwrap())).collect();
    (s.doc().unwrap().clone(), ids)
}

/// Whether the pixmap paints anything (any pixel not fully transparent).
fn paints(pix: &figcraft_render::Pixmap) -> bool {
    figcraft_render::to_rgba8(pix).as_chunks::<4>().0.iter().any(|p| p.get(3).copied().unwrap_or(0) > 0)
}

#[test]
fn a_slice_previews_the_content_under_it() {
    let (doc, ids) = doc_after(&[
        ("shape.rectangle", json!({"x": 0, "y": 0, "width": 200, "height": 100})),
        ("node.create", json!({"type": "slice", "x": 0, "y": 0, "width": 200, "height": 100})),
    ]);
    let slice = ids[1];
    let pix = thumbnail_pixmap(&doc, slice, 200.0).expect("the slice previews");
    assert_eq!([pix.width(), pix.height()], [200, 100], "the slice's area, not an empty node");
    assert!(paints(&pix), "the rectangle under the slice shows");
}

#[test]
fn a_slice_with_nothing_under_it_is_blank() {
    let (doc, ids) = doc_after(&[("node.create", json!({"type": "slice", "x": 0, "y": 0, "width": 200, "height": 100}))]);
    let pix = thumbnail_pixmap(&doc, ids[0], 200.0).expect("the preview still has the slice's area");
    assert_eq!([pix.width(), pix.height()], [200, 100]);
    assert!(!paints(&pix), "nothing under it stays transparent");
}

#[test]
fn other_layers_preview_themselves() {
    let (doc, ids) = doc_after(&[
        ("shape.rectangle", json!({"x": 0, "y": 0, "width": 200, "height": 100})),
        ("node.create", json!({"type": "slice", "x": 0, "y": 0, "width": 200, "height": 100})),
    ]);
    let pix = thumbnail_pixmap(&doc, ids[0], 200.0).expect("the rectangle previews");
    assert_eq!([pix.width(), pix.height()], [200, 100], "the layer's own area");
    assert!(paints(&pix));
}

/// SVG and PDF are vector: their export rows offer no size, which their export ignores (#256).
#[test]
fn export_rows_offer_a_size_for_images_only() {
    let mut s = Session::new();
    let r = crate::present_tests::id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 80})).unwrap());
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    s.execute("export.add", &json!({"ids": [r]})).unwrap();
    let mut u = crate::layers_drag_tests::Ui::new(s);
    let size_menu = |u: &crate::layers_drag_tests::Ui| u.ctx.read_response(egui::Id::new(("export_size", 0)).with("menu")).is_some();
    u.frame(vec![]);
    assert!(size_menu(&u), "PNG has sizes");
    for format in ["svg", "pdf", "xml"] {
        u.app.run("export.update", json!({"ids": [r], "index": 0, "format": format}));
        u.frame(vec![]);
        u.frame(vec![]);
        assert!(!size_menu(&u), "{format} is vector: no size");
    }
    for format in ["jpg", "webp"] {
        u.app.run("export.update", json!({"ids": [r], "index": 0, "format": format}));
        u.frame(vec![]);
        u.frame(vec![]);
        assert!(size_menu(&u), "{format} has sizes again");
    }
}

/// The text painted this frame, with the centre of each piece.
fn labelled(u: &mut crate::layers_drag_tests::Ui) -> Vec<(String, egui::Pos2)> {
    fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Pos2)>) {
        match shape {
            egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.pos + t.galley.rect.center().to_vec2())),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    u.frame(vec![]);
    let frame = u.frame_output(vec![]);
    let mut out = Vec::new();
    for c in &frame.shapes {
        walk(&c.shape, &mut out);
    }
    out
}

fn click(u: &mut crate::layers_drag_tests::Ui, p: egui::Pos2) {
    u.frame(vec![egui::Event::PointerMoved(p)]);
    u.button(p, true);
    u.button(p, false);
    u.frame(vec![]);
}

/// Android Vector Drawable is in the format dropdown as "XML", and choosing it sets the format.
#[test]
fn the_format_dropdown_offers_android_vector_drawable() {
    assert!(figcraft_doc::ExportFormat::ALL.contains(&figcraft_doc::ExportFormat::VectorDrawable));
    let mut s = Session::new();
    let r = crate::present_tests::id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 80})).unwrap());
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    s.execute("export.add", &json!({"ids": [r]})).unwrap();
    let mut u = crate::layers_drag_tests::Ui::new(s);
    u.frame(vec![]);
    let nodes = labelled(&mut u);
    let field = nodes.iter().find(|(l, _)| l == "PNG").map(|n| n.1).unwrap_or_else(|| panic!("no PNG dropdown in {nodes:?}"));
    click(&mut u, field);
    let nodes = labelled(&mut u);
    let xml = nodes.iter().find(|(l, _)| l == "XML").map(|n| n.1).unwrap_or_else(|| panic!("no XML row in {nodes:?}"));
    click(&mut u, xml);
    let doc = u.app.session.doc().unwrap();
    assert_eq!(doc.node(r).unwrap().exports[0].format, figcraft_doc::ExportFormat::VectorDrawable);
}

/// The File > Export dialog has an XML preset.
#[test]
fn the_export_dialog_has_an_xml_preset() {
    let mut s = Session::new();
    let r = crate::present_tests::id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 80})).unwrap());
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    let mut u = crate::layers_drag_tests::Ui::new(s);
    u.frame(vec![]);
    crate::menus::invoke(&mut u.app, "file.export");
    let nodes = labelled(&mut u);
    assert!(nodes.iter().any(|(l, _)| l == "XML"), "no XML preset in {nodes:?}");
}
