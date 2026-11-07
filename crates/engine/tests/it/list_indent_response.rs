//! `text.listIndent` / `text.listOutdent` report `changed` only when the document changed.

use figcraft_engine::{
    Session,
    doc::{ListStyle, NodeId, NodeKind},
};
use serde_json::json;

fn editing_list(indent: u8) -> (Session, NodeId) {
    let mut s = Session::new();
    let value = s.execute("node.create", &json!({"type":"text", "props":{"characters":"a"}})).unwrap();
    let id = NodeId(value["id"].as_u64().unwrap());
    s.execute("text.listStyle", &json!({"ids":[id.0], "style":"bulleted"})).unwrap();
    s.execute("node.set", &json!({"id":id.0, "props":{"indentation":indent}})).unwrap();
    s.execute("text.edit.begin", &json!({"id":id.0, "caret":0})).unwrap();
    (s, id)
}

fn list_state(s: &Session, id: NodeId) -> (ListStyle, u8) {
    let NodeKind::Text(t) = &s.doc().unwrap().node(id).unwrap().kind else { panic!("not text") };
    let p = t.para_list(0);
    (p.list, p.indent)
}

#[test]
fn indent_at_cap_reports_false() {
    let (mut s, id) = editing_list(8);
    let before = s.doc().unwrap().clone();
    let result = s.execute("text.listIndent", &json!({})).unwrap();
    assert_eq!(list_state(&s, id), (ListStyle::Bulleted, 8));
    assert_eq!(before, *s.doc().unwrap());
    assert_eq!(result["changed"], json!(false));
}

#[test]
fn indent_from_zero_reports_true() {
    let (mut s, id) = editing_list(0);
    let result = s.execute("text.listIndent", &json!({})).unwrap();
    assert_eq!(list_state(&s, id), (ListStyle::Bulleted, 1));
    assert_eq!(result["changed"], json!(true));
}

#[test]
fn outdent_from_zero_reports_true() {
    let (mut s, id) = editing_list(0);
    let result = s.execute("text.listOutdent", &json!({})).unwrap();
    assert_eq!(list_state(&s, id).0, ListStyle::None);
    assert_eq!(result["changed"], json!(true));
}

#[test]
fn indent_plain_text_reports_false() {
    let mut s = Session::new();
    let value = s.execute("node.create", &json!({"type":"text", "props":{"characters":"a"}})).unwrap();
    let id = NodeId(value["id"].as_u64().unwrap());
    s.execute("text.edit.begin", &json!({"id":id.0, "caret":0})).unwrap();
    let before = s.doc().unwrap().clone();
    let result = s.execute("text.listIndent", &json!({})).unwrap();
    assert_eq!(before, *s.doc().unwrap());
    assert_eq!(result["changed"], json!(false));
}

#[test]
fn tab_keys_still_nest_and_unnest() {
    let (mut s, id) = editing_list(0);
    s.execute("text.edit.key", &json!({"key":"Tab"})).unwrap();
    assert_eq!(list_state(&s, id), (ListStyle::Bulleted, 1));
    s.execute("text.edit.key", &json!({"key":"Tab", "shift":true})).unwrap();
    assert_eq!(list_state(&s, id), (ListStyle::Bulleted, 0));
}
