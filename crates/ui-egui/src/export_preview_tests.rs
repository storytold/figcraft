//! The Export section's Preview row: a slice paints nothing itself, so its preview must show the
//! page content under it — the same thing exporting the slice writes — and not a blank
//! transparency checkerboard next to a correct export. And its setting rows: a size only for
//! images, and advanced options named as the reference app names them.

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

/// The preview follows the slice's first export setting: with Ignore overlapping layers (contents
/// only) on, a slice in a frame shows only the frame's content, as its export does.
#[test]
fn a_slice_in_a_frame_previews_only_the_frame() {
    let mut s = Session::new();
    let new = |s: &mut Session, p: Value| NodeId(s.execute("node.create", &p).unwrap()["id"].as_u64().unwrap());
    let f = new(&mut s, json!({"type": "frame", "x": 0, "y": 0, "width": 200, "height": 100, "props": {"fills": []}}));
    let slice = new(&mut s, json!({"type": "slice", "x": 0, "y": 0, "width": 200, "height": 100, "parent": f}));
    new(&mut s, json!({"type": "rectangle", "x": 0, "y": 0, "width": 200, "height": 100}));
    let pix = thumbnail_pixmap(s.doc().unwrap(), slice, 200.0).expect("the slice previews");
    assert_eq!([pix.width(), pix.height()], [200, 100]);
    assert!(!paints(&pix), "the rectangle on the page above the empty frame is left out");
    s.execute("export.add", &json!({"ids": [slice], "setting": {"size": "1x", "contentsOnly": false}})).unwrap();
    let pix = thumbnail_pixmap(s.doc().unwrap(), slice, 200.0).expect("the slice previews");
    assert!(paints(&pix), "with the option off, the rectangle shows");
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
    let size_menu = |u: &crate::layers_drag_tests::Ui| u.ctx.read_response(egui::Id::unique(("export_size", 0)).with("menu")).is_some();
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
pub(crate) fn labelled(u: &mut crate::layers_drag_tests::Ui) -> Vec<(String, egui::Pos2)> {
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

pub(crate) fn click(u: &mut crate::layers_drag_tests::Ui, p: egui::Pos2) {
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

/// The advanced options: Ignore overlapping layers (the `contentsOnly` setting), and Include
/// bounding box for text layers only. A setting that already has the bounding box on keeps it, and
/// the option stays so it can be turned off.
#[test]
fn advanced_options_offer_the_bounding_box_for_text_layers() {
    let mut s = Session::new();
    let r = crate::present_tests::id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 80})).unwrap());
    let t = crate::present_tests::id(&s.execute("text.create", &json!({"text": "Hi", "x": 200, "y": 0})).unwrap());
    s.execute("export.add", &json!({"ids": [r]})).unwrap();
    s.execute("export.add", &json!({"ids": [t]})).unwrap();
    let mut u = crate::layers_drag_tests::Ui::new(s);
    let options = |u: &mut crate::layers_drag_tests::Ui, id: NodeId| {
        u.app.run("select.set", json!({"ids": [id]}));
        u.frame(vec![]);
        egui::Popup::open_id(&u.ctx, egui::Id::unique(("export_more", 0)));
        u.frame(vec![]);
        crate::import_report_tests::shown_texts(u.frame_output(vec![]).shapes)
    };
    let shown = options(&mut u, r);
    assert!(shown.iter().any(|s| s == "Ignore overlapping layers"), "the reference app's name: {shown:?}");
    assert!(!shown.iter().any(|s| s == "Contents only"));
    assert!(!shown.iter().any(|s| s == "Include bounding box"), "a rectangle has no bounding box option");
    let shown = options(&mut u, t);
    assert!(shown.iter().any(|s| s == "Include bounding box"), "a text layer has it: {shown:?}");
    u.app.run("export.update", json!({"ids": [r], "index": 0, "includeBoundingBox": true}));
    let shown = options(&mut u, r);
    assert!(shown.iter().any(|s| s == "Include bounding box"), "a rectangle that has it on shows it: {shown:?}");
    assert!(u.app.session.doc().unwrap().node(r).unwrap().exports[0].include_bounding_box, "the setting is kept");
}
