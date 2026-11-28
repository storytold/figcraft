use serde_json::json;

use crate::plugins_tests::{OnRefusal, install, manifest, script};
use crate::{EngineError, Session};

fn id(value: &serde_json::Value) -> crate::NodeId {
    crate::params::id_of(&value["id"]).unwrap()
}

#[test]
fn failed_group_closes_documents_opened_inside_it() {
    let mut s = Session::new();
    let original = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap());
    let before_journal = s.journal.clone();
    let before_history = s.ds().unwrap().history.undo.len();

    let result = s.execute(
        "command.batch",
        &json!({
            "label": "Open then fail",
            "commands": [
                {"command": "file.new", "params": {"name": "Temporary"}},
                {"command": "shape.rectangle", "params": {"x": 20, "y": 20, "width": 10, "height": 10}},
                {"command": "node.set", "params": {"id": 999999, "props": {"name": "must fail"}}}
            ]
        }),
    );

    assert!(result.is_err());
    assert_eq!(s.docs.len(), 1);
    assert_eq!(s.active, 0);
    assert!(s.doc().unwrap().contains(original));
    assert_eq!(s.ds().unwrap().history.undo.len(), before_history);
    assert_eq!(s.journal, before_journal);
}

#[test]
fn failed_group_reopens_documents_closed_inside_it() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"name": "Second"})).unwrap();
    let uids: Vec<_> = s.docs.iter().map(|ds| ds.uid).collect();
    assert_eq!((s.docs.len(), s.active), (2, 1));

    let result = s.execute(
        "command.batch",
        &json!({
            "commands": [
                {"command": "file.close", "params": {"index": 1, "force": true}},
                {"command": "node.set", "params": {"id": 999999, "props": {"name": "must fail"}}}
            ]
        }),
    );

    assert!(result.is_err());
    assert_eq!(s.docs.len(), 2);
    assert_eq!(s.active, 1);
    assert_eq!(s.docs.iter().map(|ds| ds.uid).collect::<Vec<_>>(), uids);
    assert!(s.ds().is_ok());
    assert!(s.closed_in_group.is_empty());
}

#[test]
fn failed_group_restores_each_tabs_history_after_undo_close_and_open() {
    let mut s = Session::new();
    let mut originals = Vec::new();
    for (index, name) in ["First", "Second"].into_iter().enumerate() {
        if index > 0 {
            s.execute("file.new", &json!({"name": name})).unwrap();
        }
        let created = s
            .execute(
                "command.batch",
                &json!({
                    "label": format!("{name} shape"),
                    "commands": [{"command": "shape.rectangle", "params": {
                        "x": index * 20, "y": 0, "width": 10, "height": 10, "props": {"name": name}
                    }}]
                }),
            )
            .unwrap();
        let original = id(&created[0]);
        s.execute(
            "command.batch",
            &json!({
                "label": format!("{name} name"),
                "commands": [{"command": "node.set", "params": {"id": original, "props": {"name": format!("{name} renamed")}}}]
            }),
        )
        .unwrap();
        assert_eq!(s.execute("edit.undo", &json!({})).unwrap(), json!({"undone": true}));
        assert_eq!(s.undo_depth(), (1, 1));
        originals.push(original);
    }
    let uids: Vec<_> = s.docs.iter().map(|ds| ds.uid).collect();
    let before_docs: Vec<_> = s.docs.iter().map(|ds| ds.doc.clone()).collect();
    let before_labels: Vec<_> = s
        .docs
        .iter()
        .map(|ds| {
            (ds.history.undo.iter().map(|h| h.label.clone()).collect::<Vec<_>>(), ds.history.redo.iter().map(|h| h.label.clone()).collect::<Vec<_>>())
        })
        .collect();
    assert_ne!(before_labels[0], before_labels[1]);
    let active = s.active;

    let result = s.execute(
        "command.batch",
        &json!({
            "commands": [
                {"command": "edit.undo"},
                {"command": "doc.nextTab"},
                {"command": "edit.undo"},
                {"command": "file.close", "params": {"index": 0, "force": true}},
                {"command": "file.new", "params": {"name": "Temporary"}},
                {"command": "no.such.command"}
            ]
        }),
    );

    assert!(result.is_err());
    assert_eq!(s.docs.iter().map(|ds| ds.uid).collect::<Vec<_>>(), uids);
    assert_eq!(s.active, active);
    assert!(s.closed_in_group.is_empty());
    for (index, ds) in s.docs.iter().enumerate() {
        assert_eq!(ds.doc, before_docs[index]);
        assert_eq!(ds.history.undo.iter().map(|h| h.label.clone()).collect::<Vec<_>>(), before_labels[index].0);
        assert_eq!(ds.history.redo.iter().map(|h| h.label.clone()).collect::<Vec<_>>(), before_labels[index].1);
    }
    // Travel through each restored tab's real history, including the pre-existing redo step.
    for _ in 0..2 {
        let index = s.active;
        let original = originals[index];
        assert_eq!(s.execute("edit.undo", &json!({})).unwrap(), json!({"undone": true}));
        assert!(!s.doc().unwrap().contains(original));
        assert_eq!(s.execute("edit.redo", &json!({})).unwrap(), json!({"redone": true}));
        assert_eq!(s.doc().unwrap(), before_docs[index].as_ref());
        assert_eq!(s.execute("edit.redo", &json!({})).unwrap(), json!({"redone": true}));
        let name = &before_docs[index].node(original).unwrap().name;
        assert_eq!(s.doc().unwrap().node(original).unwrap().name, format!("{name} renamed"));
        assert_eq!(s.execute("edit.undo", &json!({})).unwrap(), json!({"undone": true}));
        assert_eq!(s.doc().unwrap(), before_docs[index].as_ref());
        s.execute("doc.nextTab", &json!({})).unwrap();
    }
}

#[test]
fn failed_plugin_run_restores_each_document_without_restoring_unrelated_state() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"name": "Second"})).unwrap();
    let before_history: Vec<_> = s.docs.iter().map(|ds| ds.history.undo.len()).collect();
    let calls = [
        ("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})),
        ("doc.nextTab", json!({})),
        ("shape.rectangle", json!({"x": 20, "y": 0, "width": 10, "height": 10})),
        ("node.set", json!({"id": 999999, "props": {"name": "must fail"}})),
    ];
    install(&mut s, manifest("org.test.rollback", "write", &[]), &script(&calls, OnRefusal::Fail, ""));
    s.message = Some("keep this".into());
    let before_journal = s.journal.clone();
    let before_message = s.message.clone();
    // The new file is the one in front when the plugin starts.
    let before_active = s.active;

    let result = s.execute("plugin.run", &json!({"id": "org.test.rollback"}));

    assert!(matches!(result, Err(EngineError::Other(_))));
    assert_eq!(s.docs.iter().map(|ds| ds.doc.node(ds.page).map_or(0, |page| page.children.len())).collect::<Vec<_>>(), vec![0, 0]);
    assert_eq!(s.active, before_active, "the tab in front is put back");
    assert_eq!(s.docs.iter().map(|ds| ds.history.undo.len()).collect::<Vec<_>>(), before_history);
    assert_eq!(s.journal, before_journal);
    assert_eq!(s.message, before_message);
}

#[test]
fn failed_group_keeps_the_undo_steps_it_undid() {
    let mut s = Session::new();
    let original = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap());
    let before_doc = s.doc().unwrap().clone();
    assert_eq!(s.undo_depth(), (1, 0));

    let result = s.execute(
        "command.batch",
        &json!({
            "commands": [
                {"command": "edit.undo"},
                {"command": "no.such.command"}
            ]
        }),
    );

    assert!(result.is_err());
    assert_eq!(s.doc().unwrap(), &before_doc);
    assert_eq!(s.undo_depth(), (1, 0), "the undo step survives the failed batch");
    assert_eq!(s.execute("edit.undo", &json!({})).unwrap(), json!({"undone": true}));
    assert!(!s.doc().unwrap().contains(original));
    assert_eq!(s.execute("edit.redo", &json!({})).unwrap(), json!({"redone": true}));
    assert!(s.doc().unwrap().contains(original));
}

#[test]
fn failed_group_keeps_history_after_undo_and_redo() {
    let mut s = Session::new();
    let first = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap());
    let second = id(&s.execute("shape.rectangle", &json!({"x": 20, "y": 0, "width": 10, "height": 10})).unwrap());
    assert!(s.undo().unwrap());
    let before_doc = s.doc().unwrap().clone();
    let before_labels = s.undo_labels(10);
    assert_eq!(s.undo_depth(), (1, 1));

    let result = s.execute(
        "command.batch",
        &json!({
            "commands": [
                {"command": "edit.undo"},
                {"command": "edit.redo"},
                {"command": "no.such.command"}
            ]
        }),
    );

    assert!(result.is_err());
    assert_eq!(s.doc().unwrap(), &before_doc);
    assert_eq!(s.undo_depth(), (1, 1));
    assert_eq!(s.undo_labels(10), before_labels);
    assert!(s.undo().unwrap());
    assert!(!s.doc().unwrap().contains(first));
    assert!(s.redo().unwrap());
    assert!(s.doc().unwrap().contains(first));
    assert!(!s.doc().unwrap().contains(second));
    assert!(s.redo().unwrap());
    assert!(s.doc().unwrap().contains(second));
}

#[test]
fn failed_outer_group_keeps_the_undo_steps_a_nested_batch_undid() {
    let mut s = Session::new();
    let original = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap());
    let before_doc = s.doc().unwrap().clone();

    let result = s.execute(
        "command.batch",
        &json!({
            "commands": [
                {"command": "command.batch", "params": {"commands": [{"command": "edit.undo"}]}},
                {"command": "no.such.command"}
            ]
        }),
    );

    assert!(result.is_err());
    assert_eq!(s.doc().unwrap(), &before_doc);
    assert_eq!(s.undo_depth(), (1, 0));
    assert!(s.undo().unwrap());
    assert!(!s.doc().unwrap().contains(original));
    assert!(s.redo().unwrap());
    assert!(s.doc().unwrap().contains(original));
}

#[test]
fn failed_group_restores_the_small_history_limit() {
    let mut s = Session::new();
    s.ds_mut().unwrap().history.limit = 1;
    let first = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap());
    let before_doc = s.doc().unwrap().clone();

    let result = s.execute(
        "command.batch",
        &json!({
            "commands": [
                {"command": "edit.undo"},
                {"command": "shape.rectangle", "params": {"x": 20, "y": 0, "width": 10, "height": 10}},
                {"command": "no.such.command"}
            ]
        }),
    );

    assert!(result.is_err());
    assert_eq!(s.doc().unwrap(), &before_doc);
    assert_eq!(s.undo_depth(), (1, 0));
    assert_eq!(s.ds().unwrap().history.limit, 1);
    let second = id(&s.execute("shape.rectangle", &json!({"x": 40, "y": 0, "width": 10, "height": 10})).unwrap());
    assert_eq!(s.undo_depth(), (1, 0), "new edits still respect the limit");
    assert!(s.undo().unwrap());
    assert!(s.doc().unwrap().contains(first));
    assert!(!s.doc().unwrap().contains(second));
    assert!(!s.undo().unwrap(), "the oldest step was dropped");
}

#[test]
fn failed_group_keeps_the_redo_steps() {
    let mut s = Session::new();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    assert!(s.undo().unwrap());
    assert_eq!(s.undo_depth().1, 1);

    let result = s.execute(
        "command.batch",
        &json!({
            "commands": [
                {"command": "shape.rectangle", "params": {"x": 20, "y": 20, "width": 10, "height": 10}},
                {"command": "node.set", "params": {"id": 999999, "props": {"name": "must fail"}}}
            ]
        }),
    );

    assert!(result.is_err());
    assert_eq!(s.undo_depth().1, 1, "the redo step survives the failed batch");
    assert!(s.redo().unwrap());
}
