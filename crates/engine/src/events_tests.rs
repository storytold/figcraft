//! The live event log: sequence, topics, cursors, waiting, the retained window, and the events
//! comment commands record.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::events::{self, EventLog, MAX_EVENTS};
use crate::{EngineError, Session};

fn comments() -> Vec<String> {
    vec!["comments".into()]
}

#[test]
fn events_keep_sequence_and_filter_topics() {
    let log = EventLog::default();
    assert_eq!(log.latest(), 0);
    log.push("comment.added", "Doc", None, json!({"commentId": 1}));
    log.push("document.changed", "Doc", None, json!({}));
    log.push("comment.resolved", "Doc", None, json!({"commentId": 1}));
    assert_eq!(log.latest(), 3);

    let r = log.poll(0, &comments(), 10, Duration::ZERO);
    assert_eq!(r.events.len(), 2);
    assert_eq!(r.cursor, 3);
    assert!(!r.reset && r.dropped == 0);
    assert_eq!(r.events[0].kind, "comment.added");

    let r = log.poll(2, &comments(), 10, Duration::ZERO);
    assert_eq!(r.events.len(), 1);
    assert_eq!(r.events[0].seq, 3);

    let r = log.poll(3, &comments(), 10, Duration::ZERO);
    assert!(r.events.is_empty() && r.cursor == 3 && !r.reset);

    // A cursor ahead of the log means the app restarted.
    let r = log.poll(4, &comments(), 10, Duration::ZERO);
    assert!(r.reset && r.events.is_empty() && r.cursor == 3);

    let r = log.poll(0, &["document".into()], 10, Duration::ZERO);
    assert_eq!(r.events.len(), 1);
    assert_eq!(r.events[0].kind, "document.changed");
}

#[test]
fn a_poll_wakes_for_a_push() {
    let log = Arc::new(EventLog::default());
    let writer = log.clone();
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        writer.push("comment.added", "Doc", None, json!({"commentId": 7}));
    });
    let start = Instant::now();
    let r = log.poll(0, &comments(), 10, Duration::from_secs(2));
    t.join().unwrap();
    assert_eq!(r.events.len(), 1);
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn the_retained_window_reports_what_it_missed() {
    let log = EventLog::default();
    for i in 0..MAX_EVENTS + 5 {
        log.push("comment.added", "Doc", None, json!({"commentId": i}));
    }
    let r = log.poll(0, &comments(), 10, Duration::ZERO);
    assert_eq!(r.events.len(), 10);
    let r = log.poll(1, &comments(), 1000, Duration::ZERO);
    assert_eq!(r.events.len(), 1000);
    assert_eq!(r.dropped, 4);
    assert_eq!(r.events[0].seq, 6);
}

#[test]
fn comment_commands_record_events() {
    let mut s = Session::new();
    let add = s.execute("comment.add", &json!({"x": 10, "y": 20, "text": "Fix", "author": "You"})).unwrap();
    let id = add["id"].as_u64().unwrap();
    let r = s.events.poll(0, &comments(), 10, Duration::ZERO);
    assert_eq!(r.events.len(), 1);
    assert_eq!(r.events[0].kind, "comment.added");
    assert_eq!(r.events[0].data["commentId"], id);
    assert_eq!(r.events[0].document, "Untitled");

    s.execute("comment.reply", &json!({"id": id, "text": "Agreed", "author": "Sam"})).unwrap();
    s.execute("comment.resolve", &json!({"id": id})).unwrap();
    let r = s.events.poll(1, &comments(), 10, Duration::ZERO);
    assert_eq!(r.events.len(), 2);
    assert_eq!(r.events[0].kind, "comment.replied");
    assert_eq!(r.events[0].data["reply"], 0, "the reply's index, from the command's result");
    assert_eq!(r.events[1].kind, "comment.resolved");
    assert_eq!(r.events[1].data["commentId"], id);

    // Queries and read state are not feedback changes.
    let before = s.events.latest();
    s.execute("comment.markRead", &json!({"id": id})).unwrap();
    s.execute("comment.list", &json!({})).unwrap();
    s.execute("comment.prompt", &json!({})).unwrap();
    assert_eq!(s.events.latest(), before);

    s.execute("comment.delete", &json!({"id": id})).unwrap();
    let r = s.events.poll(before, &comments(), 10, Duration::ZERO);
    assert_eq!(r.events.len(), 1);
    assert_eq!(r.events[0].kind, "comment.deleted");
}

#[test]
fn a_failed_batch_reports_no_comment_events() {
    let mut s = Session::new();
    let before = s.events.latest();
    let r = s.execute(
        "command.batch",
        &json!({"commands": [
            {"command": "comment.add", "params": {"x": 10, "y": 10, "text": "Rolled back"}},
            {"command": "node.set", "params": {"ids": [999_999], "props": {"name": "x"}}}
        ]}),
    );
    assert!(r.is_err());
    assert_eq!(s.events.latest(), before);
    assert!(s.doc().unwrap().comments.is_empty());

    let r = s.execute(
        "command.batch",
        &json!({"commands": [
            {"command": "comment.add", "params": {"x": 10, "y": 10, "text": "Stays"}},
            {"command": "node.create", "params": {"type": "rectangle", "width": 10, "height": 10}}
        ]}),
    );
    assert!(r.is_ok());
    let r = s.events.poll(before, &comments(), 10, Duration::ZERO);
    assert_eq!(r.events.len(), 1);
    assert_eq!(r.events[0].kind, "comment.added");
}

#[test]
fn every_comment_mutation_records_its_kind() {
    let mut s = Session::new();
    let add = s.execute("comment.add", &json!({"x": 10, "y": 20, "text": "Fix", "author": "You"})).unwrap();
    let id = add["id"].as_u64().unwrap();
    s.execute("comment.reply", &json!({"id": id, "text": "On it", "author": "Sam"})).unwrap();
    s.execute("comment.edit", &json!({"id": id, "text": "Fix the padding", "author": "You"})).unwrap();
    s.execute("comment.react", &json!({"id": id, "emoji": "👍", "author": "Sam"})).unwrap();
    s.execute("comment.move", &json!({"id": id, "x": 30, "y": 40})).unwrap();
    s.execute("comment.resolve", &json!({"id": id})).unwrap();
    s.execute("comment.delete", &json!({"id": id})).unwrap();
    let r = s.events.poll(0, &comments(), 20, Duration::ZERO);
    let kinds: Vec<&str> = r.events.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(
        kinds,
        ["comment.added", "comment.replied", "comment.edited", "comment.reacted", "comment.moved", "comment.resolved", "comment.deleted"]
    );
    assert!(r.events.iter().all(|e| e.data["commentId"] == id), "{:?}", r.events);
    assert_eq!(r.events[1].data["reply"], 0, "the reply index");
}

#[test]
fn nested_groups_commit_events_only_when_the_outer_group_succeeds() {
    let mut s = Session::new();
    let r: crate::Result<()> = s.grouped("outer", |s| {
        s.grouped("inner", |s| s.execute("comment.add", &json!({"x": 1, "y": 2, "text": "Rolled back"})).map(|_| ()))?;
        Err(EngineError::Other("roll back the outer group".into()))
    });
    assert!(r.is_err());
    assert_eq!(s.events.latest(), 0);
    assert!(s.doc().unwrap().comments.is_empty());

    let r = s.grouped("outer", |s| {
        s.grouped("inner", |s| s.execute("comment.add", &json!({"x": 1, "y": 2, "text": "One"})).map(|_| ()))?;
        s.execute("comment.add", &json!({"x": 3, "y": 4, "text": "Two"})).map(|_| ())
    });
    assert!(r.is_ok());
    assert_eq!(s.doc().unwrap().comments.len(), 2);
    let r = s.events.poll(0, &comments(), 10, Duration::ZERO);
    assert_eq!(r.events.len(), 2);
    assert!(r.events.iter().all(|e| e.kind == "comment.added"));
}

#[test]
fn ordinary_edits_do_not_emit_comment_events() {
    let mut s = Session::new();
    let id = s.execute("node.create", &json!({"type": "rectangle", "width": 10, "height": 10})).unwrap()["id"].clone();
    s.execute("node.set", &json!({"ids": [id], "props": {"name": "Renamed"}})).unwrap();
    let r = s.events.poll(0, &comments(), 10, Duration::ZERO);
    assert!(r.events.is_empty());
    assert_eq!(s.events.latest(), 0);
}

#[test]
fn topics_parse_and_reject_unknown_names() {
    assert_eq!(events::topics(None).unwrap(), vec!["comments".to_string()]);
    assert_eq!(events::topics(Some(&json!([]))).unwrap(), vec!["all".to_string()]);
    assert_eq!(events::topics(Some(&json!(["comments", "document"]))).unwrap(), vec!["comments".to_string(), "document".to_string()]);
    assert!(events::topics(Some(&json!(["nope"]))).is_err());
    assert!(events::topics(Some(&json!("comments"))).is_err());
}

#[test]
fn every_waiter_is_woken_by_a_push() {
    let log = Arc::new(EventLog::default());
    let readers: Vec<_> = (0..2)
        .map(|_| {
            let l = log.clone();
            std::thread::spawn(move || l.poll(0, &comments(), 10, Duration::from_secs(2)))
        })
        .collect();
    std::thread::sleep(Duration::from_millis(50));
    log.push("comment.added", "Doc", None, json!({"commentId": 9}));
    for reader in readers {
        let r = reader.join().unwrap();
        assert_eq!(r.events.len(), 1);
        assert_eq!(r.events[0].data["commentId"], 9);
    }
}

#[test]
fn restoring_after_an_internal_error_keeps_the_outer_group_context() {
    let mut s = Session::new();
    s.grouped("outer", |s| {
        s.execute("comment.add", &json!({"x": 1, "y": 2, "text": "Before"})).map(|_| ())?;
        let depth = s.group_depth;
        let pending = s.pending_events.len();
        assert_eq!((depth, pending), (1, 1));
        // A failing inner command appends its own event; the restore must keep the group's
        // earlier event and remove only what the failed command added.
        s.execute("comment.add", &json!({"x": 3, "y": 4, "text": "During"})).map(|_| ())?;
        assert_eq!(s.pending_events.len(), 2);
        s.restore_after_internal_error(depth, pending);
        assert_eq!((s.group_depth, s.pending_events.len()), (depth, pending));
        Ok(())
    })
    .unwrap();
    // The outer group's success still commits its buffered event.
    let r = s.events.poll(0, &comments(), 10, Duration::ZERO);
    assert_eq!(r.events.len(), 1);
    assert_eq!(r.events[0].kind, "comment.added");
}
