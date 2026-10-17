//! `edit.copy` / `edit.cut` skip pages (they aren't layers and can't be pasted), and refuse a
//! selection of only pages under their own command id.

use serde_json::json;

use crate::{EngineError, Session};

#[test]
fn copy_and_cut_ignore_pages_so_paste_keeps_the_document_valid() {
    for cmd in ["edit.copy", "edit.cut"] {
        let mut s = Session::new();
        let page = s.page().unwrap();
        s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        match s.execute(cmd, &json!({"ids": [page.0]})) {
            Err(EngineError::Disabled(id, _)) => assert_eq!(id, cmd),
            r => panic!("{cmd} on a page: {r:?}"),
        }
        let _ = s.execute("edit.paste", &json!({}));
        let doc = s.doc().unwrap();
        assert!(doc.validate().is_empty(), "{cmd}: {:?}", doc.validate());
        assert!(doc.node(page).is_some(), "{cmd} removed the page");
    }
}

#[test]
fn cut_of_a_page_and_a_layer_moves_only_the_layer() {
    let mut s = Session::new();
    let page = s.page().unwrap();
    let r = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    let rect = crate::params::id_of(&r["id"]).unwrap();
    s.execute("edit.cut", &json!({"ids": [page.0, rect.0]})).unwrap();
    assert!(s.doc().unwrap().node(page).is_some());
    assert!(s.doc().unwrap().node(rect).is_none());
    s.execute("edit.paste", &json!({})).unwrap();
    assert!(s.doc().unwrap().validate().is_empty());
}
