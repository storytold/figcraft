//! Autosave checkpoints: only for unsaved changes since the last version, capped (oldest
//! autosaves go first, user versions stay), kept in the file with their flag.

use serde_json::{Value, json};

use crate::Session;

fn rect(s: &mut Session, x: f64) {
    s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 10, "height": 10})).unwrap();
}

fn autosave(s: &mut Session) -> bool {
    s.execute("version.autosave", &json!({})).unwrap()["saved"].as_bool().unwrap()
}

fn list(s: &mut Session) -> Vec<Value> {
    s.execute("version.list", &json!({})).unwrap().as_array().unwrap().clone()
}

#[test]
fn autosaves_follow_unsaved_changes_only() {
    let mut s = Session::new();
    assert!(!autosave(&mut s), "nothing changed");
    rect(&mut s, 0.0);
    assert!(autosave(&mut s));
    let l = list(&mut s);
    assert_eq!((l[0]["name"].as_str(), l[0]["auto"].as_bool()), (Some("Autosave"), Some(true)));
    assert!(!autosave(&mut s), "no change since the checkpoint");
    // A version the user saves is a checkpoint too.
    rect(&mut s, 20.0);
    s.execute("file.saveVersion", &json!({"name": "Mine"})).unwrap();
    assert!(!autosave(&mut s));
    assert!(list(&mut s)[1].get("auto").is_none());
    // Not while a version is shown.
    rect(&mut s, 40.0);
    s.execute("version.preview", &json!({"index": 0})).unwrap();
    assert!(!autosave(&mut s));
    s.execute("version.exitPreview", &json!({})).unwrap();
    assert!(autosave(&mut s));
}

#[test]
fn oldest_autosaves_make_room_and_user_versions_stay() {
    let mut s = Session::new();
    rect(&mut s, 0.0);
    s.execute("file.saveVersion", &json!({"name": "Keep me"})).unwrap();
    for i in 0..(crate::cmd::file::MAX_AUTOSAVES + 5) {
        rect(&mut s, i as f64 * 20.0);
        assert!(autosave(&mut s));
    }
    let l = list(&mut s);
    assert_eq!(l.iter().filter(|v| v["auto"] == true).count(), crate::cmd::file::MAX_AUTOSAVES);
    assert_eq!(l[0]["name"], "Keep me");
}

#[test]
fn autosaves_keep_their_flag_in_the_file() {
    let dir = std::env::temp_dir().join(format!("figcraft-autosave-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("auto.figcraft").to_string_lossy().to_string();
    let mut s = Session::new();
    rect(&mut s, 0.0);
    assert!(autosave(&mut s));
    s.execute("file.saveVersion", &json!({"name": "Named"})).unwrap();
    s.execute("file.save", &json!({"path": path})).unwrap();
    let mut t = Session::empty();
    t.execute("file.open", &json!({"path": path})).unwrap();
    let l = list(&mut t);
    assert_eq!(l.len(), 2);
    assert_eq!(l[0]["auto"], true);
    assert!(l[1].get("auto").is_none());
    // Just opened: nothing to autosave.
    assert!(!autosave(&mut t));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn files_that_save_themselves_still_get_checkpoints() {
    let dir = std::env::temp_dir().join(format!("figcraft-autosave-on-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("auto.figcraft").to_string_lossy().to_string();
    let mut s = Session::new();
    s.execute("pref.autoSave", &json!({"on": true})).unwrap();
    rect(&mut s, 0.0);
    s.execute("file.save", &json!({"path": path})).unwrap();
    assert!(autosave(&mut s), "saved, but changed since the last version");
    assert!(!autosave(&mut s), "no change since the checkpoint");
    // Just opened: nothing to checkpoint.
    let mut t = Session::empty();
    t.execute("pref.autoSave", &json!({"on": true})).unwrap();
    t.execute("file.open", &json!({"path": path})).unwrap();
    assert!(!autosave(&mut t));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_encoded_elsewhere_is_saved_as_of_its_snapshot() {
    use crate::cmd::file::FileSnapshot;
    let dir = std::env::temp_dir().join(format!("figcraft-snapshot-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("snap.figcraft").to_string_lossy().to_string();
    let mut s = Session::new();
    s.execute("file.save", &json!({"path": path})).unwrap();
    rect(&mut s, 0.0);
    let file = FileSnapshot::of(s.ds().unwrap());
    let bytes = file.encode().unwrap();
    rect(&mut s, 50.0);
    assert!(file.save(&mut s, &bytes).unwrap());
    assert!(s.ds().unwrap().is_dirty(), "the edit since the snapshot is still unsaved");
    let mut t = Session::empty();
    t.execute("file.open", &json!({"path": path})).unwrap();
    assert_eq!(t.ds().unwrap().doc.page_content_bounds(t.ds().unwrap().page).unwrap().x1, 10.0, "the file is the snapshot");
    // Saved meanwhile: the older bytes never overwrite the newer file.
    let file = FileSnapshot::of(s.ds().unwrap());
    let bytes = file.encode().unwrap();
    rect(&mut s, 100.0);
    s.execute("file.save", &json!({})).unwrap();
    assert!(!file.save(&mut s, &bytes).unwrap());
    assert!(!s.ds().unwrap().is_dirty());
    let mut t = Session::empty();
    t.execute("file.open", &json!({"path": path})).unwrap();
    assert_eq!(t.ds().unwrap().doc.page_content_bounds(t.ds().unwrap().page).unwrap().x1, 110.0);
    let _ = std::fs::remove_dir_all(&dir);
}
