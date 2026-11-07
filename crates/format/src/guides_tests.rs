//! Ruler guides in `.figcraft` files: a reopened page keeps its guides, and files saved with a
//! page's guides written twice open again (#1476).

use figcraft_doc::{Document, Guide, NodeKind};

use super::*;

/// A guide: vertical (at an x offset) or horizontal (at a y offset).
fn guide(vertical: bool, offset: f64) -> Guide {
    Guide { vertical, offset }
}

/// Page and frame guides survive saving, reopening, saving the reopened file and opening it again.
#[test]
fn page_and_frame_guides_survive_save_and_reopen() {
    let mut d = Document::new("ruler");
    let page = d.first_page().unwrap();
    let frame = d.create(page, NodeKind::Frame, "F", 0.0, 0.0, 100.0, 100.0).unwrap();
    d.node_mut(page).unwrap().guides_mut().extend([guide(true, 120.0), guide(false, -30.5)]);
    d.node_mut(frame).unwrap().guides_mut().push(guide(true, 12.0));
    let (once, issues) = load_bytes(&save_bytes(&d, None).unwrap()).unwrap();
    assert!(issues.is_empty(), "{issues:?}");
    assert_eq!(once.node(page).unwrap().guides(), d.node(page).unwrap().guides());
    assert_eq!(once.node(frame).unwrap().guides(), d.node(frame).unwrap().guides());
    // Saving the reopened file and opening it again (where the file stopped opening).
    let text = serde_json::to_string(&to_json(&once).unwrap()).unwrap();
    assert_eq!(text.matches("\"guides\"").count(), 2, "one list on the page, one on the frame: {text}");
    let (twice, _) = load_bytes(&save_bytes(&once, None).unwrap()).unwrap();
    assert_eq!(twice.node(page).unwrap().guides(), d.node(page).unwrap().guides());
}

/// A `document.json` with a page's `guides` key twice (as saving a reopened page wrote it) opens
/// with both lists.
#[test]
fn a_file_with_a_page_guides_list_written_twice_opens() {
    let mut d = Document::new("twice");
    let page = d.first_page().unwrap();
    d.node_mut(page).unwrap().guides_mut().push(guide(false, 7.0));
    let text = serde_json::to_string(&to_json(&d).unwrap()).unwrap();
    // What saving a reopened page wrote: the guides added since, then the ones the file had.
    let broken = text.replacen("\"guides\":[", "\"guides\":[{\"vertical\":true,\"offset\":5.0}],\"guides\":[", 1);
    assert_eq!(broken.matches("\"guides\"").count(), 2);
    let (back, _) = from_json(&broken).unwrap();
    assert_eq!(back.node(page).unwrap().guides(), &[guide(true, 5.0), guide(false, 7.0)]);
}
