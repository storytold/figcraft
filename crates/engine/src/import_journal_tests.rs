//! Imported file bytes already live in the document: the command log keeps no second copy.

use base64::Engine as _;
use serde_json::{Value, json};

use crate::Session;

const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="#3366cc"/></svg>"##;

fn payload() -> String {
    base64::engine::general_purpose::STANDARD.encode(SVG)
}

fn holds_payload(journal: &[(String, Value)]) -> bool {
    let payload = payload();
    journal.iter().any(|(_, params)| params.to_string().contains(&payload))
}

#[test]
fn byte_imports_keep_no_payload_in_the_journal() {
    let mut s = Session::new();
    let before = s.doc().unwrap().node_count();
    s.execute("file.import", &json!({"data": payload(), "name": "pasted.svg"})).unwrap();
    assert!(!holds_payload(&s.journal), "the journal must not retain the Base64 file");
    // Not journaling is no change to the edit itself: still one undo step.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().node_count(), before);
}

#[test]
fn batch_wrappers_do_not_retain_import_payloads() {
    let mut s = Session::new();
    let nested =
        json!({"command": "command.batch", "params": {"commands": [{"command": "file.import", "params": {"data": payload(), "name": "a.svg"}}]}});
    s.execute("command.batch", &json!({"commands": [nested]})).unwrap();
    assert!(!holds_payload(&s.journal));
}

#[test]
fn path_imports_and_ordinary_edits_are_still_journaled() {
    let dir = std::env::temp_dir().join(format!("figcraft-import-journal-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("kit.svg");
    std::fs::write(&path, SVG).unwrap();
    let mut s = Session::new();
    s.execute("file.import", &json!({"path": path.to_string_lossy()})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    let ids: Vec<_> = s.journal.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["file.import", "shape.rectangle"]);
    std::fs::remove_dir_all(&dir).ok();
}
