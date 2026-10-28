//! PDF print options through the export commands: one-off exports, layers' own export settings
//! (stored, edited, undone), the frames PDF, and bad values refused.

use base64::Engine as _;
use figcraft_doc::*;
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;

/// A session with a 200 × 100 frame (selected).
fn session() -> (Session, NodeId) {
    let mut s = Session::new();
    let r = s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 100, "name": "Poster"})).unwrap();
    let f = id_of(&r["id"]).unwrap();
    s.set_selection(vec![f]);
    (s, f)
}

fn pdf_text(r: &Value) -> String {
    let bytes = base64::engine::general_purpose::STANDARD.decode(r["data"].as_str().unwrap()).unwrap();
    assert!(bytes.starts_with(b"%PDF"));
    String::from_utf8_lossy(&bytes).into_owned()
}

#[test]
fn one_off_pdf_exports_take_print_options() {
    let (mut s, f) = session();
    let plain = pdf_text(&s.execute("file.export", &json!({"ids": [f], "format": "pdf"})).unwrap());
    assert!(plain.contains("/MediaBox[0 0 200 100]") && !plain.contains("/TrimBox"), "{plain}");
    // 0.125 in = 9 pt of bleed and crop marks.
    let r = s.execute("file.export", &json!({"ids": [f], "format": "pdf", "bleed": "0.125in", "cropMarks": true, "cmyk": true})).unwrap();
    let t = pdf_text(&r);
    assert!(t.contains("/TrimBox[33 33 233 133]") && t.contains("/BleedBox[24 24 242 142]"), "{t}");
    // Numbers are points; other formats ignore the options.
    let t = pdf_text(&s.execute("file.export", &json!({"ids": [f], "format": "pdf", "bleed": 9})).unwrap());
    assert!(t.contains("/TrimBox[9 9 209 109]"), "{t}");
    assert!(s.execute("file.export", &json!({"ids": [f], "format": "png", "bleed": "3mm"})).is_ok());
}

#[test]
fn bad_print_options_are_refused() {
    let (mut s, f) = session();
    for bad in [
        json!({"bleed": "3 furlongs"}),
        json!({"bleed": -1}),
        json!({"bleed": "3in"}),
        json!({"bleed": true}),
        json!({"cropMarks": "maybe"}),
        json!({"cmyk": [1]}),
    ] {
        let mut p = bad.clone();
        p["ids"] = json!([f]);
        p["format"] = json!("pdf");
        assert!(s.execute("file.export", &p).is_err(), "{bad}");
        assert!(s.execute("export.add", &json!({"ids": [f], "setting": bad})).is_err(), "{bad}");
    }
    assert!(s.doc().unwrap().node(f).unwrap().exports.is_empty(), "nothing added by a refused setting");
}

#[test]
fn export_settings_keep_print_options() {
    let (mut s, f) = session();
    s.execute("export.add", &json!({"ids": [f], "setting": {"format": "pdf", "bleed": "3mm", "cmyk": true}})).unwrap();
    let print = |s: &Session| s.doc().unwrap().node(f).unwrap().exports[0].print;
    assert!((print(&s).bleed - 72.0 / 25.4 * 3.0).abs() < 1e-9 && print(&s).cmyk && !print(&s).crop_marks);
    s.execute("export.update", &json!({"ids": [f], "index": 0, "cropMarks": true})).unwrap();
    assert!(print(&s).crop_marks && print(&s).cmyk, "the rest stays");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(!print(&s).crop_marks);
    // "Export layers" uses each setting's own options ...
    let r = s.execute("file.exportLayers", &json!({"ids": [f]})).unwrap();
    let t = pdf_text(&r["files"][0]);
    assert!(t.contains("/TrimBox[") && !t.contains("0 0 0 RG"), "a bleed, no marks: {t}");
    // ... unless options are passed for all of them.
    let r = s.execute("file.exportLayers", &json!({"ids": [f], "bleed": 0})).unwrap();
    assert!(!pdf_text(&r["files"][0]).contains("/TrimBox["));
    // Saved with the file and read back.
    let json = serde_json::to_value(&s.doc().unwrap().node(f).unwrap().exports).unwrap();
    assert_eq!(json[0]["print"]["cmyk"], json!(true));
}

#[test]
fn the_frames_pdf_takes_print_options() {
    let (mut s, _) = session();
    s.execute("frame.create", &json!({"x": 300, "y": 0, "width": 50, "height": 50, "name": "Card"})).unwrap();
    let r = s.execute("file.exportFramesPdf", &json!({"bleed": "3mm", "bleedMarks": true})).unwrap();
    assert_eq!(r["pages"], json!(2));
    let t = pdf_text(&r);
    assert_eq!(t.matches("/TrimBox[").count(), 2, "{t}");
    assert_eq!(t.matches("/BleedBox[").count(), 2, "{t}");
}

/// File > Print: the plan lays each frame on a paper sheet; the command only opens the dialog.
#[test]
fn print_plan_lays_frames_on_paper_and_the_command_opens_the_dialog() {
    use crate::cmd::file::{print_plan, printable_frames};
    use figcraft_pdf::{Fit, FitOrientation, FitScale};
    let (mut s, f) = session();
    let page = s.page().unwrap();
    let doc = s.doc().unwrap();
    assert_eq!(printable_frames(doc, page, None), vec![f]);
    assert!(printable_frames(doc, page, Some(&[])).is_empty());
    let a4 = Fit { paper: (595.28, 841.89), orientation: FitOrientation::Auto, scale: FitScale::Shrink, margin: 0.0 };
    let plan = print_plan(doc, &[f], Some(a4), false).unwrap();
    assert!(plan.pdf.starts_with(b"%PDF"));
    // 200 x 100 is landscape: the A4 sheet turns, the frame keeps its size and is centred.
    let sheet = &plan.sheets[0];
    assert_eq!(sheet.sheet, (841.89, 595.28));
    assert_eq!(sheet.scale, 1.0);
    assert!((sheet.offset.0 - (841.89 - 200.0) / 2.0).abs() < 1e-9);
    // Without a paper the sheet is the frame.
    let own = print_plan(doc, &[f], None, false).unwrap();
    assert_eq!(own.sheets[0].sheet, (200.0, 100.0));
    assert!(print_plan(doc, &[], None, false).is_err());
    // The command opens the dialog and prints nothing.
    let r = s.execute("file.print", &json!({})).unwrap();
    assert_eq!(r["frames"], 1);
    assert!(s.ui_requests.iter().any(|u| u.kind == "dialog.print"));
}

/// A frame with one hyperlinked word of text (`None` when this machine has no font to lay it out).
fn linked_doc() -> Option<(figcraft_doc::Document, figcraft_doc::NodeId)> {
    use figcraft_doc::{NodeKind, StyleRun, TextData};
    let mut d = figcraft_doc::Document::new("Links");
    let page = d.first_page()?;
    let f = d.create(page, NodeKind::Frame, "F", 0.0, 0.0, 300.0, 100.0).ok()?;
    let mut td = TextData::new("Read the docs here");
    td.style.font_size = 20.0;
    let mut linked = td.style.clone();
    linked.link = Some("https://example.com/figcraft".into());
    td.runs = vec![StyleRun { start: 9, end: 13, style: linked }];
    let t = d.create(f, NodeKind::Text(td), "T", 10.0, 10.0, 280.0, 40.0).ok()?;
    d.get_mut(t).ok()?.fills = vec![figcraft_color::Paint::solid(figcraft_color::Color::BLACK)];
    let (layout, _) = figcraft_text::layout_node(d.node(t)?)?;
    (!layout.glyphs.is_empty()).then_some((d, f))
}

/// Whether the PDF bytes carry any link: an annotation array, a link annotation or a URI action.
fn has_links(pdf: &[u8]) -> bool {
    let s = String::from_utf8_lossy(pdf);
    ["/Annots", "/Link", "/URI"].iter().any(|k| s.contains(k))
}

/// File > Print never writes link annotations, whatever the paper; exporting to PDF keeps them.
#[test]
fn the_print_pdf_has_no_links_but_the_export_keeps_them() {
    use crate::cmd::file::{frames_pdf, print_plan};
    use figcraft_pdf::{Fit, FitOrientation, FitScale};
    let Some((d, f)) = linked_doc() else { return };
    let page = d.first_page().unwrap();
    assert!(has_links(&frames_pdf(&d, page, &PrintSettings::default()).unwrap().0), "the exported PDF keeps its links");
    let a4 = Fit { paper: (595.28, 841.89), orientation: FitOrientation::Auto, scale: FitScale::Shrink, margin: 18.0 };
    for fit in [None, Some(a4)] {
        for gray in [false, true] {
            let plan = print_plan(&d, &[f], fit, gray).unwrap();
            assert!(!has_links(&plan.pdf), "fit {fit:?}, gray {gray}");
        }
    }
}
