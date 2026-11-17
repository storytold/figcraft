use super::*;

/// Two open documents; the second one (active) carries an unsaved rectangle.
fn session_with_unsaved_work() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"name": "Work"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 40})).unwrap();
    assert!(s.docs[1].is_dirty(), "setup: the second document must carry unsaved changes");
    s
}

/// A temporary directory unique to one test, removed when dropped (also after a failed assertion).
struct ScratchDir(std::path::PathBuf);

impl ScratchDir {
    fn new(test: &str) -> Self {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        Self(std::env::temp_dir().join(format!("figcraft-keep-unsaved-{}-{stamp}-{test}", std::process::id())))
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_caller_who_may_not_discard_cannot_close_unsaved_work_by_force() {
    let mut s = session_with_unsaved_work();
    s.keep_unsaved = true;
    let refused = s.execute("file.close", &json!({"index": 1, "force": true}));
    assert!(refused.is_err(), "a caller dropped the user's unsaved changes by force");
    assert_eq!(s.docs.len(), 2, "the refused close still removed the document");
    assert!(s.docs[1].is_dirty(), "the refused close lost the unsaved changes");
    s.keep_unsaved = false;
    s.execute("file.close", &json!({"index": 1, "force": true})).unwrap();
    assert_eq!(s.docs.len(), 1, "a caller allowed to discard can still close by force");
}

#[test]
fn nor_as_a_step_of_a_batch() {
    let mut s = session_with_unsaved_work();
    s.keep_unsaved = true;
    let steps = json!({"commands": [{"command": "file.close", "params": {"index": 1, "force": true}}]});
    let refused = s.execute("command.batch", &steps);
    assert!(refused.is_err(), "a batch step dropped the user's unsaved changes by force");
    assert_eq!(s.docs.len(), 2, "the batch closed the document with unsaved changes");
    assert!(s.docs[1].is_dirty(), "the batch lost the unsaved changes");
}

#[test]
fn nor_revert_it_to_the_saved_file() {
    let scratch = ScratchDir::new("nor_revert_it_to_the_saved_file");
    let dir = scratch.0.clone();
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("kept.figcraft").to_string_lossy().to_string();
    let mut s = Session::new();
    s.execute("file.saveAs", &json!({"path": path})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 40})).unwrap();
    assert!(s.docs[0].is_dirty(), "setup: the saved document must be modified again");
    s.keep_unsaved = true;
    let refused = s.execute("file.revert", &json!({}));
    assert!(refused.is_err(), "a caller threw away the user's unsaved changes by reverting");
    assert!(s.docs[0].is_dirty(), "the refused revert still dropped the unsaved changes");
    s.keep_unsaved = false;
    s.execute("file.revert", &json!({})).unwrap();
    assert!(!s.docs[0].is_dirty(), "a caller allowed to discard can still revert");
}

#[test]
fn mark_saved_then_close_force_under_keep_unsaved_still_refuses() {
    let mut s = session_with_unsaved_work();
    s.keep_unsaved = true;
    assert!(s.docs[1].is_dirty(), "setup: the document must be modified before markSaved");
    let marked = s.execute("file.markSaved", &json!({}));
    assert!(marked.is_err(), "a caller marked the user's unsaved changes as saved");
    let refused = s.execute("file.close", &json!({"index": 1, "force": true}));
    assert!(refused.is_err(), "markSaved opened the way to a forced close of unsaved work");
    assert_eq!(s.docs.len(), 2, "the document with unsaved changes was closed");
    assert!(s.docs[1].is_dirty(), "the unsaved changes were lost");
}

#[test]
fn mark_saved_then_revert_under_keep_unsaved_still_refuses() {
    let mut s = session_with_unsaved_work();
    s.keep_unsaved = true;
    assert!(s.docs[1].is_dirty(), "setup: the document must be modified before markSaved");
    let marked = s.execute("file.markSaved", &json!({}));
    assert!(marked.is_err(), "a caller marked the user's unsaved changes as saved");
    let refused = s.execute("file.revert", &json!({}));
    assert!(refused.is_err(), "markSaved opened the way to reverting unsaved work");
    assert!(s.docs[1].is_dirty(), "the unsaved changes were lost");
}

// Guard, not proof of a defect: outside the restriction markSaved keeps working.
#[test]
fn mark_saved_outside_keep_unsaved_still_marks_saved() {
    let mut s = session_with_unsaved_work();
    s.keep_unsaved = false;
    s.execute("file.markSaved", &json!({})).unwrap();
    assert!(!s.docs[1].is_dirty(), "markSaved no longer marks the document saved");
}

// Guard, not proof of a defect: with nothing unsaved there is nothing to keep,
// so the restriction must not get in the way of closing.
#[test]
fn a_document_without_unsaved_changes_still_closes() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"name": "Clean"})).unwrap();
    assert!(!s.docs[1].is_dirty(), "setup: the new document must be unmodified");
    s.keep_unsaved = true;
    s.execute("file.close", &json!({"index": 1, "force": true})).unwrap();
    assert_eq!(s.docs.len(), 1, "an unmodified document did not close");
}
