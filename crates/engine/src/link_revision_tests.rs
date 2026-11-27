use crate::{Session, inspect};
use serde_json::json;
use std::time::Duration;

#[test]
fn link_events_observe_unsaved_properties_and_undo_redo() {
    let mut s = Session::new();
    let id = s.execute("shape.rectangle", &json!({"x":1,"y":2,"width":30,"height":40})).unwrap()["id"].as_u64().unwrap();
    let cursor = s.events.latest();
    s.execute("node.set", &json!({"ids":[id],"props":{"name":"Unsaved","opacity":0.5}})).unwrap();
    let events = s.events.poll(cursor, &["document".into()], 10, Duration::ZERO);
    assert_eq!(events.events.len(), 1);
    let event = &events.events[0];
    assert_eq!(event.kind, "document.changed");
    assert_eq!(event.data["documentId"], s.ds().unwrap().uid);
    assert_eq!(event.data["revision"], s.ds().unwrap().revision);
    assert!(event.data["ids"].as_array().unwrap().contains(&json!(id)));
    assert!(s.ds().unwrap().path.is_none());
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(s.events.poll(events.cursor, &["document".into()], 10, Duration::ZERO).events.len(), 2);
}

#[test]
fn link_conditional_write_rejects_stale_document_or_revision_without_mutation() {
    let mut s = Session::new();
    let id = s.execute("shape.rectangle", &json!({"x":0,"y":0,"width":10,"height":10})).unwrap()["id"].as_u64().unwrap();
    let (uid, revision) = (s.ds().unwrap().uid, s.ds().unwrap().revision);
    s.execute_as_if_revision(None, "node.set", &json!({"ids":[id],"props":{"name":"First"}}), uid, revision).unwrap();
    let doc = s.ds().unwrap().doc.clone();
    let undo = s.undo_depth();
    let cursor = s.events.latest();
    let current = s.ds().unwrap().revision;
    assert!(s.execute_as_if_revision(None, "node.set", &json!({"ids":[id],"props":{"name":"Lost"}}), uid, revision).is_err());
    assert!(s.execute_as_if_revision(None, "node.set", &json!({"ids":[id],"props":{"name":"Wrong file"}}), uid + 1, current).is_err());
    assert_eq!(s.ds().unwrap().doc, doc);
    assert_eq!(s.undo_depth(), undo);
    assert_eq!(s.events.latest(), cursor);
    assert_eq!(s.ds().unwrap().revision, current);
    assert_eq!(inspect::document(&s, 2, false)["revision"], current);
}

#[test]
fn link_failed_batch_emits_no_document_change() {
    let mut s = Session::new();
    let cursor = s.events.latest();
    assert!(s.execute("command.batch",&json!({"commands":[{"command":"shape.rectangle","params":{"x":0,"y":0,"width":10,"height":10}},{"command":"node.set","params":{"ids":[999999],"props":{"opacity":"bad"}}}]})).is_err());
    assert!(s.events.poll(cursor, &["document".into()], 10, Duration::ZERO).events.is_empty());
}

#[test]
fn link_inspection_does_not_emit_change() {
    let mut s = Session::new();
    let cursor = s.events.latest();
    s.execute("doc.list", &json!({})).unwrap();
    assert!(s.events.poll(cursor, &["document".into()], 10, Duration::ZERO).events.is_empty());
}
