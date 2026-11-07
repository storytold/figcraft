//! Paste of layers copied from another design app: the clipboard's design data (a `.fig` canvas in
//! its HTML) read through the `.fig` import and pasted as Paste pastes our own layers.

use base64::Engine as _;
use serde_json::json;

use super::{NodeId, Session, Value, params};
use crate::cmd::edit::{CLIP_CLOSE, CLIP_OPEN, design_clipboard_buffer};

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

fn id(v: &Value) -> NodeId {
    params::id_of(&v["id"]).unwrap()
}

fn copied_file() -> Vec<u8> {
    let mut s = Session::new();
    let frame = id(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 100, "name": "Card"})).unwrap());
    s.execute("node.create", &json!({"type": "rectangle", "x": 10, "y": 10, "width": 40, "height": 20, "parent": frame, "name": "Badge"})).unwrap();
    s.execute("node.create", &json!({"type": "ellipse", "x": 300, "y": 0, "width": 50, "height": 50, "name": "Dot"})).unwrap();
    let r = s.execute("file.export", &json!({"format": "fig"})).unwrap();
    B64.decode(r["data"].as_str().unwrap()).unwrap()
}

fn clipboard_html(buffer: &[u8]) -> String {
    let mut wrapped = B64.encode(buffer);
    wrapped.insert(8, '\n');
    format!(r#"<meta charset="utf-8"><span data-buffer="<!--{CLIP_OPEN}{wrapped}{CLIP_CLOSE}-->"></span>"#)
}

fn names(s: &Session, ids: &[NodeId]) -> Vec<String> {
    let doc = s.doc().unwrap();
    ids.iter().map(|i| doc.node(*i).unwrap().name.clone()).collect()
}

#[test]
fn buffer_is_found_in_the_html() {
    let file = copied_file();
    assert_eq!(design_clipboard_buffer(&clipboard_html(&file)), Some(file));
    assert_eq!(design_clipboard_buffer("<p>plain</p>"), None);
    assert_eq!(design_clipboard_buffer(&format!("{CLIP_OPEN}not base64!{CLIP_CLOSE}")), None);
    assert_eq!(design_clipboard_buffer(&format!("{CLIP_OPEN}{}{CLIP_CLOSE}", B64.encode(b"hello"))), None);
    assert_eq!(design_clipboard_buffer(&format!("{CLIP_OPEN}ZmlnLQ")), None);
    // Web text that only mentions the markers holds no design data.
    assert_eq!(design_clipboard_buffer(&format!("<p>see {CLIP_OPEN} and {CLIP_CLOSE} in the docs</p>")), None);
}

#[test]
fn oversized_design_data_is_refused_before_decoding() {
    // Just past the cap (256 MB of data, as base64), made of valid base64 characters.
    let big = "A".repeat((256 * 1024 * 1024usize).div_ceil(3) * 4 + 4);
    assert_eq!(design_clipboard_buffer(&format!("{CLIP_OPEN}{big}{CLIP_CLOSE}")), None);
    let mut s = Session::new();
    let e = s.execute("edit.pasteDesignLayers", &json!({ "data": big })).unwrap_err();
    assert!(e.to_string().contains("256 MB"), "{e}");
    assert!(s.clipboard.nodes.is_empty());
}

#[test]
fn layers_paste_into_the_page_and_undo() {
    let mut s = Session::new();
    let page = s.page().unwrap();
    let r = s.execute("edit.pasteDesignLayers", &json!({"html": clipboard_html(&copied_file())})).unwrap();
    let ids: Vec<NodeId> = r["ids"].as_array().unwrap().iter().map(|v| params::id_of(v).unwrap()).collect();
    assert_eq!(names(&s, &ids), ["Card", "Dot"]);
    assert_eq!(s.selection(), ids);
    let doc = s.doc().unwrap();
    assert!(ids.iter().all(|i| doc.node(*i).unwrap().parent == Some(page)));
    let card = doc.node(ids[0]).unwrap();
    assert_eq!(names(&s, &card.children), ["Badge"]);
    assert_eq!((card.width, card.height), (200.0, 100.0));
    assert!(s.undo().unwrap());
    assert!(ids.iter().all(|i| !s.doc().unwrap().contains(*i)));
}

#[test]
fn a_second_paste_repeats_the_copied_layers() {
    let mut s = Session::new();
    // `data` may be wrapped and unpadded, like the HTML's base64.
    let mut data = B64.encode(copied_file()).trim_end_matches('=').to_string();
    data.insert(8, '\n');
    s.execute("edit.pasteDesignLayers", &json!({ "data": data })).unwrap();
    let r = s.execute("edit.paste", &json!({})).unwrap();
    let ids: Vec<NodeId> = r["ids"].as_array().unwrap().iter().map(|v| params::id_of(v).unwrap()).collect();
    assert_eq!(names(&s, &ids), ["Card", "Dot"]);
}

#[test]
fn pastes_into_the_selected_frame() {
    let mut s = Session::new();
    let host = id(&s.execute("frame.create", &json!({"x": 1000, "y": 0, "width": 800, "height": 600})).unwrap());
    s.execute("select.set", &json!({"ids": [host]})).unwrap();
    let r = s.execute("edit.pasteDesignLayers", &json!({"html": clipboard_html(&copied_file())})).unwrap();
    let first = params::id_of(&r["ids"][0]).unwrap();
    assert_eq!(s.doc().unwrap().node(first).unwrap().parent, Some(host));
}

#[test]
fn other_content_is_refused_without_changes() {
    let mut s = Session::new();
    let before = s.doc().unwrap().nodes().count();
    assert!(s.execute("edit.pasteDesignLayers", &json!({})).is_err());
    assert!(s.execute("edit.pasteDesignLayers", &json!({"html": "<b>hi</b>"})).is_err());
    assert!(s.execute("edit.pasteDesignLayers", &json!({"data": B64.encode(b"PK not a zip")})).is_err());
    assert!(s.execute("edit.pasteDesignLayers", &json!({"data": B64.encode(b"fig-kiwi\x00\x00\x00\x00junk")})).is_err());
    assert_eq!(s.doc().unwrap().nodes().count(), before);
    assert!(s.clipboard.nodes.is_empty());
}
