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
