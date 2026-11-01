//! A branch's own version history: versions saved on a branch tab go into the branch (not the
//! main file's history), are viewed and restored there, are saved in the file with the branch and
//! stay with it when it is merged or archived; merging adds a version to the main file's history.
//! Also fills merging item by item through real commands.

use figcraft_doc::{NodeId, NodeKind};
use serde_json::{Value, json};

use crate::Session;
use crate::collab_tests::base;

fn ok(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn names(s: &mut Session) -> Vec<String> {
    ok(s, "version.list", json!({})).as_array().unwrap().iter().map(|v| v["name"].as_str().unwrap().to_string()).collect()
}

fn layer_name(s: &Session, id: NodeId) -> String {
    s.doc().unwrap().node(id).unwrap().name.clone()
}

/// The base file with one version saved on main and a branch "Explore" shown: (session, branch
/// id, a rectangle).
fn branched() -> (Session, String, NodeId) {
    let mut s = Session::with_document(base());
    let r = s.doc().unwrap().nodes().find(|n| matches!(n.kind, NodeKind::Rectangle)).unwrap().id;
    ok(&mut s, "file.saveVersion", json!({"name": "Main v1"}));
    let id = ok(&mut s, "branch.create", json!({"name": "Explore"}))["id"].as_str().unwrap().to_string();
    (s, id, r)
}

#[test]
fn versions_saved_on_a_branch_are_the_branchs_own() {
    let (mut s, _, r) = branched();
    let original = layer_name(&s, r);
    // A new branch's history starts with its creation: the main file's versions stay with it.
    assert_eq!(names(&mut s), ["Branch created"]);
    ok(&mut s, "node.set", json!({"id": r, "props": {"name": "Before"}}));
    let v = ok(&mut s, "file.saveVersion", json!({"name": "Branch v1"}))["id"].clone();
    ok(&mut s, "node.set", json!({"id": r, "props": {"name": "After"}}));
    assert_eq!(names(&mut s), ["Branch created", "Branch v1"]);
    // Viewing a branch version shows it read-only; going back shows the branch as it is.
    ok(&mut s, "version.preview", json!({"id": v}));
    assert_eq!(layer_name(&s, r), "Before");
    assert!(s.execute("node.set", &json!({"id": r, "props": {"name": "No"}})).is_err());
    ok(&mut s, "version.exitPreview", json!({}));
    assert_eq!(layer_name(&s, r), "After");
    // Restoring is one undo step on the branch.
    ok(&mut s, "version.restore", json!({"id": v}));
    assert_eq!(layer_name(&s, r), "Before");
    ok(&mut s, "edit.undo", json!({}));
    assert_eq!(layer_name(&s, r), "After");
    ok(&mut s, "edit.redo", json!({}));
    // Renaming works on the branch's history; main's history is untouched.
    ok(&mut s, "version.rename", json!({"id": v, "name": "Branch v1 (renamed)"}));
    ok(&mut s, "doc.switch", json!({"index": 0}));
    assert_eq!(names(&mut s), ["Main v1"]);
    assert_eq!(layer_name(&s, r), original, "restoring on the branch leaves main alone");
}

#[test]
fn branch_versions_save_with_the_file_and_survive_merging() {
    let dir = std::env::temp_dir().join(format!("figcraft-branch-versions-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("file.figcraft").to_string_lossy().to_string();
    let (mut s, id, r) = branched();
    ok(&mut s, "node.set", json!({"id": r, "props": {"name": "On the branch"}}));
    ok(&mut s, "file.saveVersion", json!({"name": "Branch v1"}));
    // An autosave on a branch tab goes into the branch's history too.
    ok(&mut s, "node.set", json!({"id": r, "props": {"name": "On the branch, later"}}));
    assert_eq!(ok(&mut s, "version.autosave", json!({}))["saved"], true);
    ok(&mut s, "doc.switch", json!({"index": 0}));
    ok(&mut s, "file.saveAs", json!({"path": path}));
    let mut t = Session::empty();
    ok(&mut t, "file.open", json!({"path": path}));
    assert_eq!(names(&mut t), ["Main v1"]);
    ok(&mut t, "branch.open", json!({"id": id}));
    assert_eq!(names(&mut t), ["Branch created", "Branch v1", "Autosave"]);
    // Merging adds a version to the main file's history; the branch keeps its own.
    ok(&mut t, "branch.merge", json!({}));
    assert_eq!(names(&mut t), ["Main v1", "Autosave", "Merged “Explore”"]);
    assert_eq!(layer_name(&t, r), "On the branch, later");
    ok(&mut t, "branch.open", json!({"id": id}));
    assert_eq!(names(&mut t), ["Branch created", "Branch v1", "Autosave"]);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn archiving_keeps_the_branch_history() {
    let (mut s, id, _) = branched();
    ok(&mut s, "file.saveVersion", json!({"name": "Branch v1"}));
    ok(&mut s, "branch.archive", json!({}));
    assert_eq!(names(&mut s), ["Main v1"]);
    ok(&mut s, "branch.open", json!({"id": id}));
    assert_eq!(names(&mut s), ["Branch created", "Branch v1"]);
}

#[test]
fn different_fills_of_a_layer_merge_without_a_conflict() {
    // Main gives a rectangle a second fill before branching; the branch recolours the first and
    // main the second: no conflict, both kept. Recolouring the same fill both ways conflicts.
    let mut s = Session::with_document(base());
    let r = s.doc().unwrap().nodes().find(|n| matches!(n.kind, NodeKind::Rectangle)).unwrap().id;
    ok(&mut s, "fill.add", json!({"ids": [r]}));
    let id = ok(&mut s, "branch.create", json!({}))["id"].as_str().unwrap().to_string();
    ok(&mut s, "fill.set", json!({"ids": [r], "index": 0, "color": "#FF0000"}));
    ok(&mut s, "doc.switch", json!({"index": 0}));
    ok(&mut s, "fill.set", json!({"ids": [r], "index": 1, "color": "#0000FF"}));
    assert_eq!(ok(&mut s, "branch.review", json!({"id": id}))["conflicts"], json!([]));
    ok(&mut s, "branch.merge", json!({"id": id}));
    let fills = serde_json::to_string(&s.doc().unwrap().node(r).unwrap().fills).unwrap();
    let mut t = Session::with_document(s.doc().unwrap().clone());
    ok(&mut t, "fill.set", json!({"ids": [r], "index": 0, "color": "#FF0000"}));
    ok(&mut t, "fill.set", json!({"ids": [r], "index": 1, "color": "#0000FF"}));
    assert_eq!(fills, serde_json::to_string(&t.doc().unwrap().node(r).unwrap().fills).unwrap(), "both recolours kept");

    let id = ok(&mut s, "branch.create", json!({}))["id"].as_str().unwrap().to_string();
    ok(&mut s, "fill.set", json!({"ids": [r], "index": 1, "color": "#00FF00"}));
    ok(&mut s, "doc.switch", json!({"index": 0}));
    ok(&mut s, "fill.set", json!({"ids": [r], "index": 1, "color": "#FFFF00"}));
    let conflicts = ok(&mut s, "branch.review", json!({"id": id}))["conflicts"].clone();
    assert_eq!(conflicts.as_array().unwrap().len(), 1, "{conflicts}");
    assert_eq!(conflicts[0]["key"], format!("{r}/fills"));
}
