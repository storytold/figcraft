//! Recovery copies: a run that ends without closing leaves copies of its unsaved documents that the
//! next start opens again (a running app's are left alone); a normal quit leaves nothing.

use figcraft_engine::Session;
use serde_json::json;

use crate::recovery::{Recovery, recover, recover_all};

fn root(name: &str) -> std::path::PathBuf {
    let r = std::env::temp_dir().join(format!("figcraft-recovery-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&r);
    r
}

fn edited() -> Session {
    let mut s = Session::new();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    s
}

#[test]
fn a_crashed_runs_unsaved_documents_come_back() {
    let root = root("crash");
    let s = edited();
    let mut r = Recovery::start(&root).unwrap();
    r.write(&s, true);
    // While it runs, another start leaves its copies alone.
    let mut other = Session::empty();
    assert_eq!(recover(&root, &mut other), 0);
    // It ends without closing (the lock goes with the process).
    drop(r);
    let mut next = Session::empty();
    assert_eq!(recover(&root, &mut next), 1);
    let ds = next.docs.first().unwrap();
    assert!(ds.is_dirty() && ds.path.is_none());
    assert_eq!(next.doc().unwrap().node(next.doc().unwrap().pages[0]).map(|p| p.children.len()), Some(1));
    assert!(std::fs::read_dir(&root).unwrap().next().is_some(), "the copy stays until a run has its own");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn copies_follow_saves_and_a_normal_quit_leaves_nothing() {
    let root = root("quit");
    let mut s = edited();
    let mut r = Recovery::start(&root).unwrap();
    r.write(&s, true);
    let dir = std::fs::read_dir(&root).unwrap().flatten().next().unwrap().path();
    let copies =
        |d: &std::path::Path| std::fs::read_dir(d).unwrap().flatten().filter(|f| f.path().extension().is_some_and(|e| e == "figcraft")).count();
    assert_eq!(copies(&dir), 1);
    // Saved: the copy goes.
    s.ds_mut().unwrap().mark_saved();
    r.write(&s, true);
    assert_eq!(copies(&dir), 0);
    // Changes pending while the interval runs: it asks to be woken for them.
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
    assert!(r.tick(&s, 0.0).is_none());
    assert!(r.tick(&s, 1.0).is_none(), "copied at 0 s: nothing new");
    s.execute("shape.rectangle", &json!({"x": 9, "y": 0, "width": 4, "height": 4})).unwrap();
    assert!(r.tick(&s, 1.0).is_some_and(|w| w > 0.0));
    r.finish();
    assert!(!dir.exists(), "a normal quit removes the copies");
    let _ = std::fs::remove_dir_all(&root);
}

/// A run that ended without closing, leaving a copy of `s`'s unsaved document: its folder (as
/// another process's).
fn crashed(s: &Session, root: &std::path::Path) -> std::path::PathBuf {
    let mut r = Recovery::start(root).unwrap();
    r.write(s, true);
    drop(r);
    let left = root.join("crashed");
    std::fs::rename(root.join(std::process::id().to_string()), &left).unwrap();
    left
}

fn copies_in(d: &std::path::Path) -> usize {
    std::fs::read_dir(d).map_or(0, |d| d.flatten().filter(|f| f.path().extension().is_some_and(|e| e == "figcraft")).count())
}

/// The next start: the copies come back, and this run's recovery keeps them until it covers them.
fn restart(root: &std::path::Path) -> (Session, Recovery) {
    let mut next = Session::empty();
    let recovered = recover_all(root, &mut next);
    assert_eq!(recovered.copies, 1);
    let mut r = Recovery::start(root).unwrap();
    r.adopt(recovered.waiting);
    (next, r)
}

#[test]
fn a_recovered_copy_stays_until_this_run_has_its_own_copy() {
    let root = root("handoff");
    let left = crashed(&edited(), &root);
    let (next, mut r) = restart(&root);
    assert_eq!(copies_in(&left), 1, "the only copy on disk until this run writes one");
    r.write(&next, true);
    r.tick(&next, 100.0);
    assert!(!left.exists(), "covered by this run's copy, the old one goes");
    assert_eq!(copies_in(&root.join(std::process::id().to_string())), 1);
    r.finish();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_recovered_copy_stays_while_its_replacement_cant_be_written() {
    let root = root("blocked");
    let left = crashed(&edited(), &root);
    let (next, mut r) = restart(&root);
    // Something is in the way of this run's copy: writing it fails.
    let mine = root.join(std::process::id().to_string()).join(format!("{}.figcraft", next.docs[0].uid));
    std::fs::create_dir_all(&mine).unwrap();
    r.write(&next, true);
    r.tick(&next, 100.0);
    assert_eq!(copies_in(&left), 1, "kept: the work is nowhere else on disk");
    // Once it can be written, the old copy goes.
    std::fs::remove_dir(&mine).unwrap();
    let mut next = next;
    next.execute("shape.rectangle", &json!({"x": 1, "y": 1, "width": 2, "height": 2})).unwrap();
    r.write(&next, true);
    r.tick(&next, 200.0);
    assert!(!left.exists());
    r.finish();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn without_a_recovery_of_its_own_the_copy_stays_for_the_next_start() {
    let root = root("nocopies");
    crashed(&edited(), &root);
    let mut next = Session::empty();
    let recovered = recover_all(&root, &mut next);
    assert_eq!(recovered.copies, 1);
    // This run couldn't take a folder of its own: it ends (or crashes) with nothing copied.
    drop(recovered);
    let mut after = Session::empty();
    assert_eq!(recover(&root, &mut after), 1, "the work comes back again");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_recovered_copy_goes_once_its_document_is_saved_or_closed() {
    for close in [false, true] {
        let root = root(if close { "closed" } else { "saved" });
        let left = crashed(&edited(), &root);
        let (mut next, mut r) = restart(&root);
        if close {
            next.docs.clear();
        } else {
            next.ds_mut().unwrap().mark_saved();
        }
        r.tick(&next, 0.0);
        assert!(!left.exists(), "close {close}");
        r.finish();
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[test]
fn a_normal_quit_removes_the_recovered_copies_it_took_over() {
    let root = root("quitafter");
    let left = crashed(&edited(), &root);
    let (_, r) = restart(&root);
    // Quitting asked to save or discard every document.
    r.finish();
    assert!(!left.exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_run_whose_folder_name_is_still_held_takes_another() {
    let root = root("pidreuse");
    // A crashed run with this process's id whose folder the start still holds (its copy waits).
    let held = root.join(std::process::id().to_string());
    std::fs::create_dir_all(&held).unwrap();
    let lock = std::fs::File::create(held.join("lock")).unwrap();
    lock.try_lock().unwrap();
    let mut r = Recovery::start(&root).expect("a folder of its own");
    let s = edited();
    r.write(&s, true);
    assert_eq!(copies_in(&held), 0);
    let dirs = std::fs::read_dir(&root).unwrap().flatten().filter(|e| e.path() != held).count();
    assert_eq!(dirs, 1);
    r.finish();
    drop(lock);
    let _ = std::fs::remove_dir_all(&root);
}

/// The rectangles in `doc`.
fn rectangles(doc: &figcraft_engine::doc::Document) -> usize {
    doc.nodes().filter(|n| matches!(n.kind, figcraft_engine::doc::NodeKind::Rectangle)).count()
}

/// The documents a crashed run's copies under `root` open as.
fn crash_and_recover(r: Recovery, root: &std::path::Path) -> Session {
    drop(r);
    let mut next = Session::empty();
    assert_eq!(recover(root, &mut next), 1);
    next
}

#[test]
fn copies_keep_the_version_history_and_branches() {
    // #185: a copy has what a save would write, the latest edits on a branch tab included.
    let root = root("history");
    let mut s = edited();
    s.execute("file.saveVersion", &json!({"name": "v1"})).unwrap();
    s.execute("branch.create", &json!({"name": "Explore"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 20, "y": 0, "width": 10, "height": 10})).unwrap();
    let mut r = Recovery::start(&root).unwrap();
    r.write(&s, true);
    let next = crash_and_recover(r, &root);
    let ds = next.docs.first().unwrap();
    assert_eq!(ds.versions.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(), ["v1"]);
    assert_eq!(ds.branches.iter().map(|b| (b.name.as_str(), rectangles(&b.head))).collect::<Vec<_>>(), [("Explore", 2)]);
    let _ = std::fs::remove_dir_all(&root);
}

/// The one copy under `root` (the run's folder), read back.
fn copy(root: &std::path::Path) -> figcraft_format::Loaded {
    let dir = std::fs::read_dir(root).unwrap().flatten().next().unwrap().path();
    let file = std::fs::read_dir(dir).unwrap().flatten().map(|f| f.path()).find(|f| f.extension().is_some_and(|e| e == "figcraft")).unwrap();
    figcraft_format::load_bytes_with_versions(&std::fs::read(file).unwrap()).unwrap()
}

#[test]
fn a_change_only_on_a_branch_tab_or_to_the_history_is_copied() {
    let root = root("history-only");
    let mut s = edited();
    s.execute("file.saveVersion", &json!({"name": "v1"})).unwrap();
    s.execute("branch.create", &json!({"name": "Explore"})).unwrap();
    for d in &mut s.docs {
        d.mark_saved();
    }
    let mut r = Recovery::start(&root).unwrap();
    // An edit on the branch tab only: the main file has no changes of its own yet.
    s.execute("shape.rectangle", &json!({"x": 20, "y": 0, "width": 10, "height": 10})).unwrap();
    r.write(&s, true);
    assert_eq!(copy(&root).branches.iter().map(|b| rectangles(&b.head)).collect::<Vec<_>>(), [2]);
    s.execute("shape.rectangle", &json!({"x": 40, "y": 0, "width": 10, "height": 10})).unwrap();
    r.write(&s, true);
    assert_eq!(copy(&root).branches.iter().map(|b| rectangles(&b.head)).collect::<Vec<_>>(), [3]);
    // A version renamed on the main file leaves its document as it was.
    s.execute("doc.switch", &json!({"index": 0})).unwrap();
    r.write(&s, true);
    let id = s.docs[0].versions[0].id.clone();
    s.execute("version.rename", &json!({"id": id, "name": "v1 (renamed)"})).unwrap();
    r.write(&s, true);
    assert_eq!(copy(&root).versions.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(), ["v1 (renamed)"]);
    r.finish();
    let _ = std::fs::remove_dir_all(&root);
}

fn slow_write(path: std::path::PathBuf) -> std::thread::JoinHandle<bool> {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(150));
        std::fs::write(path, b"stale").is_ok()
    })
}

#[test]
fn saving_or_closing_waits_for_a_slow_write_so_the_copy_does_not_come_back() {
    let root = root("slow-cleanup");
    let mut s = edited();
    let mut r = Recovery::start(&root).unwrap();
    let dir = std::fs::read_dir(&root).unwrap().flatten().next().unwrap().path();
    let uid = s.docs[0].uid;
    let copy = dir.join(format!("{uid}.figcraft"));
    r.hold(uid, 1, slow_write(copy.clone()));
    s.ds_mut().unwrap().mark_saved();
    r.write(&s, false);
    std::thread::sleep(std::time::Duration::from_millis(400));
    assert!(!copy.exists(), "a saved document's copy stays removed");
    // Closed: the same.
    let s = edited();
    let uid = s.docs[0].uid;
    let copy = dir.join(format!("{uid}.figcraft"));
    r.hold(uid, 1, slow_write(copy.clone()));
    r.write(&Session::empty(), false);
    std::thread::sleep(std::time::Duration::from_millis(400));
    assert!(!copy.exists(), "a closed document's copy stays removed");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_emergency_save_waits_for_the_write_in_flight_and_copies_the_newest_revision() {
    let root = root("slow-emergency");
    let s = edited();
    let mut r = Recovery::start(&root).unwrap();
    let dir = std::fs::read_dir(&root).unwrap().flatten().next().unwrap().path();
    let uid = s.docs[0].uid;
    let copy = dir.join(format!("{uid}.figcraft"));
    r.hold(uid, 0, slow_write(copy.clone()));
    r.write(&s, true);
    assert!(std::fs::read(&copy).is_ok_and(|b| b != b"stale"), "the newest revision replaced the slow write");
    let _ = std::fs::remove_dir_all(&root);
}

/// The files named `*.figcraft` in the run folder under `root`.
fn copy_count(root: &std::path::Path) -> usize {
    let Some(dir) = std::fs::read_dir(root).unwrap().flatten().next().map(|e| e.path()) else { return 0 };
    std::fs::read_dir(dir).unwrap().flatten().filter(|f| f.path().extension().is_some_and(|e| e == "figcraft")).count()
}

/// A folder at the path of `s`'s first document's copy in the run folder under `root`.
fn blocked_copy(root: &std::path::Path, s: &Session) -> std::path::PathBuf {
    let dir = std::fs::read_dir(root).unwrap().flatten().next().unwrap().path();
    let blocker = dir.join(format!("{}.figcraft", s.docs[0].uid));
    std::fs::create_dir(&blocker).unwrap();
    blocker
}

#[test]
fn a_copy_that_failed_is_tried_again_without_another_edit() {
    let root = root("retry");
    let s = edited();
    let mut r = Recovery::start(&root).unwrap();
    // A folder where the copy goes makes its write fail on every platform (renaming the run folder
    // away fails on Windows, where the run's lock file keeps it open).
    let blocker = blocked_copy(&root, &s);
    r.write(&s, true);
    std::fs::remove_dir(&blocker).unwrap();
    // Nothing was edited since: the same revision still has to be copied.
    r.write(&s, true);
    assert_eq!(copy_count(&root), 1, "the failed revision was copied again");
    r.finish();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_background_copy_that_failed_is_tried_again_without_another_edit() {
    let root = root("retry-bg");
    let s = edited();
    let mut r = Recovery::start(&root).unwrap();
    let blocker = blocked_copy(&root, &s);
    r.write(&s, false);
    // Let the failing write end before the way is clear again.
    std::thread::sleep(std::time::Duration::from_millis(300));
    std::fs::remove_dir(&blocker).unwrap();
    for _ in 0..50 {
        r.write(&s, false);
        if copy_count(&root) == 1 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert_eq!(copy_count(&root), 1, "the failed revision was copied again");
    r.finish();
    let _ = std::fs::remove_dir_all(&root);
}
