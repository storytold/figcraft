//! A panic in a grouped closure, outside any command, ends the group like a failure (#1085): the
//! documents, selection, undo and redo, history limit and journal are as they were, and the panic
//! goes on to the caller.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use serde_json::json;

use crate::{HistoryEntry, NodeId, Session};

fn rect(s: &mut Session, x: i64) -> NodeId {
    s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().map(NodeId).unwrap()
}

fn labels(entries: &[HistoryEntry]) -> Vec<String> {
    entries.iter().map(|e| e.label.clone()).collect()
}

#[test]
fn a_grouped_closure_that_panics_leaves_the_session_as_before() {
    let mut s = Session::new();
    let first = rect(&mut s, 0);
    let second = rect(&mut s, 20);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [first]})).unwrap();
    let ds = s.ds().unwrap();
    let (doc, selection, page) = (ds.doc.clone(), ds.selection.clone(), ds.page);
    let (undo, redo, limit) = (labels(&ds.history.undo), labels(&ds.history.redo), ds.history.limit);
    assert_eq!(redo.len(), 1);
    let journal = s.journal.clone();
    let depth = s.undo_depth();

    let unwound = catch_unwind(AssertUnwindSafe(|| {
        s.grouped("Edit then panic", |s| -> crate::Result<()> {
            s.execute("node.set", &json!({"id": first, "props": {"name": "Renamed"}}))?;
            rect(s, 40);
            s.execute("select.set", &json!({"ids": []}))?;
            panic!("a bug in the closure");
        })
    }));

    let payload = unwound.unwrap_err();
    assert_eq!(payload.downcast_ref::<&str>(), Some(&"a bug in the closure"), "the original panic goes on to the caller");
    let ds = s.ds().unwrap();
    assert!(Arc::ptr_eq(&ds.doc, &doc));
    assert_eq!(*ds.doc, *doc);
    assert_eq!(ds.selection, selection);
    assert_eq!(ds.page, page);
    assert_eq!(labels(&ds.history.undo), undo);
    assert_eq!(labels(&ds.history.redo), redo);
    assert_eq!(ds.history.limit, limit);
    assert_eq!(s.journal, journal);
    assert_eq!(s.undo_depth(), depth);
    assert_eq!(s.group_depth, 0);
    assert!(s.closed_in_group.is_empty());

    // The redo step survived, and the next edit pushes one step of its own.
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(s.doc().unwrap().contains(second));
    rect(&mut s, 60);
    assert_eq!(s.undo_depth(), (depth.0 + 2, 0));
}

#[test]
fn a_grouped_closure_that_panics_releases_the_live_hold_so_later_edits_sync() {
    use crate::live_tests::{push, relay, serve, session};

    let (mut s, script) = session();
    let first = rect(&mut s, 0);
    s.execute("collab.host", &json!({})).unwrap();
    let (mut server, id, welcome) = serve(s.doc().unwrap());
    push(&script, vec![welcome]);
    s.live_poll();
    assert!(s.live_doc(s.ds().unwrap().uid));
    let before = s.doc().unwrap().clone();
    script.lock().unwrap().msgs.clear();

    let unwound = catch_unwind(AssertUnwindSafe(|| {
        s.grouped("Rename then panic", |s| -> crate::Result<()> {
            s.execute("node.set", &json!({"id": first, "props": {"name": "Rolled back"}}))?;
            s.live_sync();
            assert!(script.lock().unwrap().msgs.is_empty(), "the edit waits for the group to end");
            panic!("a bug in the live closure");
        })
    }));

    assert!(unwound.is_err());
    assert_eq!(s.doc().unwrap(), &before);
    s.live_sync();
    assert!(script.lock().unwrap().msgs.is_empty(), "the rolled-back edit never goes out");

    s.execute("node.set", &json!({"id": first, "props": {"name": "Sent afterwards"}})).unwrap();
    s.live_sync();
    assert_eq!(script.lock().unwrap().msgs.len(), 1, "the later edit goes out");
    let replies = relay(&script, &mut server, id);
    push(&script, replies);
    s.live_poll();
    assert_eq!(server.document().node(first).unwrap().name, "Sent afterwards");
    assert_eq!(s.doc().unwrap(), server.document());
}

#[test]
fn a_panic_in_an_inner_group_rolls_back_through_the_outer_one() {
    let mut s = Session::new();
    let unwound = catch_unwind(AssertUnwindSafe(|| {
        s.grouped("outer", |s| {
            s.execute("comment.add", &json!({"x": 1, "y": 2, "text": "Rolled back"}))?;
            s.grouped("inner", |s| -> crate::Result<()> {
                s.execute("comment.add", &json!({"x": 3, "y": 4, "text": "Also rolled back"}))?;
                panic!("a bug in the inner closure");
            })
        })
    }));

    assert!(unwound.is_err());
    assert!(s.doc().unwrap().comments.is_empty());
    assert!(s.pending_events.is_empty());
    assert_eq!(s.events.latest(), 0);
    assert_eq!(s.group_depth, 0);
    assert_eq!(s.undo_depth(), (0, 0));

    // The session still groups normally afterwards.
    s.grouped("again", |s| s.execute("comment.add", &json!({"x": 5, "y": 6, "text": "Kept"})).map(|_| ())).unwrap();
    assert_eq!(s.doc().unwrap().comments.len(), 1);
    assert_eq!(s.undo_depth(), (1, 0));
}

#[test]
fn a_group_that_closes_and_opens_tabs_then_panics_brings_the_tabs_back() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"name": "Second"})).unwrap();
    let uids: Vec<_> = s.docs.iter().map(|ds| ds.uid).collect();
    let active = s.active;

    let unwound = catch_unwind(AssertUnwindSafe(|| {
        s.grouped("Close, open, panic", |s| -> crate::Result<()> {
            s.execute("file.close", &json!({"index": 1, "force": true}))?;
            s.execute("file.new", &json!({"name": "Temporary"}))?;
            panic!("a bug after switching tabs");
        })
    }));

    assert!(unwound.is_err());
    assert_eq!(s.docs.iter().map(|ds| ds.uid).collect::<Vec<_>>(), uids);
    assert_eq!(s.active, active);
    assert!(s.closed_in_group.is_empty());
    assert!(s.docs.iter().all(|ds| ds.history.limit != usize::MAX));
}
