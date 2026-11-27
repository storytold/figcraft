//! The files open when the desktop app quit open again at its next start.

use std::path::{Path, PathBuf};

use serde_json::json;

use crate::Session;

/// A fresh folder for a test's files, removed when the test ends, a failing one too.
struct Folder(PathBuf);

impl std::ops::Deref for Folder {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn folder(name: &str) -> Folder {
    let dir = std::env::temp_dir().join(format!("figcraft-reopen-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    Folder(dir)
}

fn saved(s: &mut Session, dir: &Path, name: &str) -> String {
    let path = dir.join(format!("{name}.figcraft")).to_string_lossy().into_owned();
    s.execute("file.new", &json!({"name": name})).unwrap();
    s.execute("file.save", &json!({"path": path})).unwrap();
    path
}

fn paths(s: &Session) -> Vec<Option<String>> {
    s.docs.iter().map(|d| d.path.clone()).collect()
}

/// The next start: the preferences the last run kept, nothing open yet.
fn next_start(last: &Session) -> Session {
    let mut s = Session::empty();
    s.prefs = last.prefs.clone();
    s
}

#[test]
fn the_files_open_at_quit_reopen_in_their_order_with_the_same_one_in_front() {
    let dir = folder("order");
    let mut s = Session::empty();
    let a = saved(&mut s, &dir, "A");
    s.execute("file.new", &json!({"name": "Untitled"})).unwrap();
    let b = saved(&mut s, &dir, "B");
    let c = saved(&mut s, &dir, "C");
    s.execute("doc.switch", &json!({"index": 2})).unwrap();
    s.remember_open_files();
    assert_eq!(s.prefs.open_files, vec![a.clone(), b.clone(), c.clone()], "the new file without a path is left out");
    assert_eq!(s.prefs.front_file.as_deref(), Some(b.as_str()));

    let mut t = next_start(&s);
    assert!(t.reopen_files().is_empty());
    assert_eq!(paths(&t), vec![Some(a), Some(b.clone()), Some(c)]);
    assert_eq!(t.docs[t.active].path.as_deref(), Some(b.as_str()), "B is in front again");
}

#[test]
fn a_file_gone_since_is_skipped_and_one_that_wont_open_says_why() {
    let dir = folder("gone");
    let mut s = Session::empty();
    let kept = saved(&mut s, &dir, "Kept");
    let gone = saved(&mut s, &dir, "Gone");
    let broken = saved(&mut s, &dir, "Broken");
    s.remember_open_files();
    std::fs::remove_file(&gone).unwrap();
    std::fs::write(&broken, b"not a figcraft file").unwrap();

    let mut t = next_start(&s);
    let failed = t.reopen_files();
    assert_eq!(failed.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>(), vec![broken]);
    assert_eq!(paths(&t), vec![Some(kept)]);
}

#[test]
fn a_file_already_open_at_start_keeps_its_one_tab() {
    let dir = folder("twice");
    let mut s = Session::empty();
    let a = saved(&mut s, &dir, "A");
    let b = saved(&mut s, &dir, "B");
    s.execute("doc.switch", &json!({"index": 0})).unwrap();
    s.remember_open_files();

    // B was named on the command line (or came back as a recovered copy).
    let mut t = next_start(&s);
    t.execute("file.open", &json!({"path": b})).unwrap();
    assert!(t.reopen_files().is_empty());
    assert_eq!(paths(&t), vec![Some(b), Some(a.clone())]);
    assert_eq!(t.docs[t.active].path.as_deref(), Some(a.as_str()));
}

#[test]
fn quitting_with_nothing_open_leaves_nothing_to_reopen() {
    let dir = folder("none");
    let mut s = Session::empty();
    saved(&mut s, &dir, "A");
    s.remember_open_files();
    s.execute("file.close", &json!({"index": 0})).unwrap();
    s.remember_open_files();
    assert!(s.prefs.open_files.is_empty());
    assert_eq!(s.prefs.front_file, None);
    let saved = serde_json::to_value(&s.prefs).unwrap();
    assert!(saved.get("open_files").is_none() && saved.get("front_file").is_none(), "not written when empty");
}
