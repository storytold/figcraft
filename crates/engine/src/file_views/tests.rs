//! Files open again where they were left.

use serde_json::{Value, json};

use super::*;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

/// A test's folder, removed when the test ends, a failing one too.
struct Dir(std::path::PathBuf);

impl std::ops::Deref for Dir {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn dir(name: &str) -> Dir {
    let d = std::env::temp_dir().join(format!("figcraft-file-views-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    Dir(d)
}

/// A saved file at `path` with two pages, each holding a rectangle, the first page shown.
fn two_pages(s: &mut Session, path: &str) -> (NodeId, NodeId) {
    ex(s, "file.new", json!({}));
    ex(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 100}));
    ex(s, "page.add", json!({"name": "Second"}));
    let pages = s.doc().unwrap().pages.clone();
    ex(s, "page.select", json!({"id": pages[1].0}));
    ex(s, "shape.rectangle", json!({"x": 2000, "y": 2000, "width": 50, "height": 50}));
    ex(s, "page.select", json!({"id": pages[0].0}));
    ex(s, "file.saveAs", json!({"path": path}));
    (pages[0], pages[1])
}

fn view(s: &Session) -> (NodeId, f64, f64, f64) {
    let ds = s.ds().unwrap();
    (ds.page, ds.view.cx, ds.view.cy, ds.view.zoom)
}

fn look(s: &mut Session, cx: f64, cy: f64, zoom: f64) {
    let v = &mut s.ds_mut().unwrap().view;
    (v.cx, v.cy, v.zoom) = (cx, cy, zoom);
}

/// Closed and opened again, a file shows the page it showed, at its view, and its other page at
/// the view that page was left at; looking around leaves the file unchanged.
#[test]
fn a_file_opens_again_where_it_was_left() {
    let d = dir("left");
    let path = d.join("two.figcraft").to_string_lossy().into_owned();
    let mut s = Session::new();
    let (first, second) = two_pages(&mut s, &path);
    let saved = std::fs::read(&path).unwrap();
    look(&mut s, 40.0, 60.0, 2.5);
    ex(&mut s, "page.select", json!({"id": second.0}));
    look(&mut s, 2010.0, 2020.0, 0.5);
    assert!(!s.ds().unwrap().is_dirty(), "looking around is no change to the file");
    ex(&mut s, "file.close", json!({}));
    assert_eq!(std::fs::read(&path).unwrap(), saved, "nothing written to the file");
    assert!(s.prefs_unsaved, "kept in the preferences");
    ex(&mut s, "file.open", json!({"path": path}));
    assert_eq!(view(&s), (second, 2010.0, 2020.0, 0.5));
    assert!(!s.ds().unwrap().fit_when_sized, "the canvas keeps the view instead of fitting");
    ex(&mut s, "page.select", json!({"id": first.0}));
    assert_eq!(view(&s), (first, 40.0, 60.0, 2.5));
    // Through the preferences file, as the next start reads them.
    let prefs: crate::Prefs = serde_json::from_value(serde_json::to_value(&s.prefs).unwrap()).unwrap();
    let mut next = Session::new();
    next.prefs = prefs;
    ex(&mut next, "file.open", json!({"path": path}));
    assert_eq!(view(&next), (second, 2010.0, 2020.0, 0.5));
}

/// Quitting keeps every open file's view; a file never saved keeps none.
#[test]
fn quitting_keeps_each_open_files_view() {
    let d = dir("quit");
    let (a, b) = (d.join("a.figcraft").to_string_lossy().into_owned(), d.join("b.figcraft").to_string_lossy().into_owned());
    let mut s = Session::new();
    let (a_page, _) = two_pages(&mut s, &a);
    look(&mut s, 1.0, 2.0, 3.0);
    let (b_page, _) = two_pages(&mut s, &b);
    look(&mut s, 4.0, 5.0, 0.25);
    ex(&mut s, "file.new", json!({}));
    s.remember_views();
    let kept: Vec<&str> = s.prefs.file_views.iter().map(|v| v.path.as_str()).collect();
    assert_eq!(kept.len(), 2, "{kept:?}");
    let mut next = Session::new();
    next.prefs = s.prefs.clone();
    ex(&mut next, "file.open", json!({"path": a}));
    assert_eq!(view(&next), (a_page, 1.0, 2.0, 3.0));
    ex(&mut next, "file.open", json!({"path": b}));
    assert_eq!(view(&next), (b_page, 4.0, 5.0, 0.25));
}

/// A kept page the file no longer has, or a view the canvas can't show, is left out: the file
/// opens fitted as before.
#[test]
fn a_view_that_no_longer_fits_the_file_is_left_out() {
    let d = dir("stale");
    let path = d.join("one.figcraft").to_string_lossy().into_owned();
    let mut s = Session::new();
    let (first, _) = two_pages(&mut s, &path);
    ex(&mut s, "file.close", json!({}));
    s.prefs.file_views[0].page = NodeId(999_999);
    s.prefs.file_views[0].pages = vec![PageView { page: first, cx: f64::NAN, cy: 0.0, zoom: 1.0 }];
    ex(&mut s, "file.open", json!({"path": path}));
    let ds = s.ds().unwrap();
    assert_eq!(ds.page, first);
    assert!(ds.fit_when_sized, "fitted as a file never left");
    assert!(ds.view.cx.is_finite());
}

/// The views kept are bounded, the most recently left first.
#[test]
fn the_most_recently_left_files_are_kept() {
    let mut s = Session::new();
    for k in 0..MAX_FILE_VIEWS + 5 {
        ex(&mut s, "file.new", json!({}));
        s.ds_mut().unwrap().path = Some(format!("/nowhere/{k}.figcraft"));
        ex(&mut s, "file.close", json!({"force": true}));
    }
    assert_eq!(s.prefs.file_views.len(), MAX_FILE_VIEWS);
    assert_eq!(s.prefs.file_views[0].path, format!("/nowhere/{}.figcraft", MAX_FILE_VIEWS + 4));
}
