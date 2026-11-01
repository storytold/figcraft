//! A batch keeps one undo snapshot, not one per step (#1099): it is still one undo step that
//! restores exactly what was there before, and a failure still rolls all of it back.

use serde_json::{Value, json};

use crate::{NodeId, Session};

fn id(value: &Value) -> NodeId {
    crate::params::id_of(&value["id"]).unwrap()
}

fn rectangles(s: &mut Session, n: usize) -> Vec<NodeId> {
    (0..n).map(|i| id(&s.execute("shape.rectangle", &json!({"x": i * 20, "y": 0, "width": 10, "height": 10})).unwrap())).collect()
}

fn rename(target: NodeId, name: &str) -> Value {
    json!({"command": "node.set", "params": {"id": target.0, "props": {"name": name}}})
}

fn undo_len(s: &Session) -> usize {
    s.ds().unwrap().history.undo.len()
}

#[test]
fn a_thousand_step_batch_keeps_one_snapshot_and_undoes_as_one_step() {
    let mut s = Session::new();
    let ids = rectangles(&mut s, 50);
    let (before, base) = (s.doc().unwrap().clone(), undo_len(&s));
    let mut most = 0;
    s.grouped("Many", |s| {
        for i in 0..1000 {
            let target = ids[i % ids.len()];
            s.execute("node.set", &json!({"id": target.0, "props": {"name": format!("Step {i}")}}))?;
            most = most.max(undo_len(s));
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(most, base + 1, "one snapshot held for the whole group, not one per step");
    assert_eq!(undo_len(&s), base + 1);
    assert_eq!(s.undo_labels(1), vec!["Many".to_string()]);
    let after = s.doc().unwrap().clone();
    assert!(s.undo().unwrap());
    assert_eq!(s.doc().unwrap(), &before, "undo puts back exactly what was there");
    assert!(s.redo().unwrap());
    assert_eq!(s.doc().unwrap(), &after);
}

#[test]
fn command_batch_and_its_continue_mode_are_one_step_each() {
    for mode in ["abort", "continue"] {
        let mut s = Session::new();
        let ids = rectangles(&mut s, 10);
        let (before, base) = (s.doc().unwrap().clone(), undo_len(&s));
        let mut steps: Vec<Value> = (0..1000).map(|i| rename(ids[i % ids.len()], &format!("{mode} {i}"))).collect();
        if mode == "continue" {
            steps.insert(500, json!({"command": "node.set", "params": {"id": 999_999, "props": {"name": "missing"}}}));
        }
        s.execute("command.batch", &json!({"onError": mode, "commands": steps})).unwrap();
        assert_eq!(undo_len(&s), base + 1, "{mode}");
        assert_eq!(s.doc().unwrap().node(ids[9]).unwrap().name, format!("{mode} 999"));
        assert!(s.undo().unwrap());
        assert_eq!(s.doc().unwrap(), &before, "{mode}");
    }
}

#[test]
fn a_failing_step_after_hundreds_rolls_back_everything() {
    let mut s = Session::new();
    let ids = rectangles(&mut s, 10);
    let (before, labels) = (s.doc().unwrap().clone(), s.undo_labels(100));
    let mut steps: Vec<Value> = (0..500).map(|i| rename(ids[i % ids.len()], &format!("R {i}"))).collect();
    steps.push(json!({"command": "shape.rectangle", "params": {"x": 0, "y": 0, "width": 5, "height": 5}}));
    steps.push(json!({"command": "no.such.command"}));
    assert!(s.execute("command.batch", &json!({"commands": steps})).is_err());
    assert_eq!(s.doc().unwrap(), &before);
    assert_eq!(s.undo_labels(100), labels);
    assert_eq!(s.undo_depth().1, 0);
}

#[test]
fn undo_inside_a_batch_undoes_the_batch_so_far() {
    let make = |x: i32| json!({"command": "shape.rectangle", "params": {"x": x, "y": 40, "width": 10, "height": 10}});
    let start = || {
        let mut s = Session::new();
        let first = rectangles(&mut s, 1)[0];
        (s, first)
    };
    // An edit and its undo: nothing changes.
    let (mut s, _) = start();
    let before = s.doc().unwrap().clone();
    s.execute("command.batch", &json!({"commands": [make(0), {"command": "edit.undo"}]})).unwrap();
    assert_eq!(s.doc().unwrap(), &before);
    // The batch's edits so far go back together; the next undo is the step before the batch.
    let (mut s, first) = start();
    s.execute("command.batch", &json!({"commands": [make(0), make(20), {"command": "edit.undo"}, {"command": "edit.undo"}]})).unwrap();
    assert!(!s.doc().unwrap().contains(first));
    assert!(s.undo().unwrap());
    assert!(s.doc().unwrap().contains(first));
    // Undo then redo inside: the edits come back and stay one step.
    let (mut s, _) = start();
    let r = s.execute("command.batch", &json!({"commands": [make(0), {"command": "edit.undo"}, {"command": "edit.redo"}, make(20)]})).unwrap();
    assert_eq!(r[2]["redone"], true);
    let count = |s: &Session| s.doc().unwrap().node(s.page().unwrap()).unwrap().children.len();
    assert_eq!(count(&s), 3);
    assert!(s.undo().unwrap());
    assert_eq!(count(&s), 1);
}
