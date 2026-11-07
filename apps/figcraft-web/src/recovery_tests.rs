//! Recovery copies in the browser: what a page kept when it stopped opens again, unsaved, on the
//! next page.

use figcraft_engine::Session;
use figcraft_ui_egui::browser_stopped;
use serde_json::json;

use crate::recovery::{Unsaved, restore, store};

fn edited(name: &str) -> Session {
    let mut s = Session::new();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    s.execute("file.rename", &json!({"name": name})).unwrap();
    s
}

/// The page's copies, stored as the panic hook stores them.
fn stored(u: &Unsaved) -> Option<String> {
    let mut out = None;
    let kept = store(u.files(), |v| {
        out = Some(v.to_string());
        true
    });
    assert_eq!(kept, u.count());
    out
}

#[test]
fn a_stopped_pages_unsaved_files_open_again_unsaved() {
    let s = edited("Poster");
    let mut u = Unsaved::default();
    u.follow(&s);
    assert_eq!(u.count(), 1);
    let mut next = Session::new();
    let before = next.docs.len();
    assert_eq!(restore(&mut next, stored(&u)), 1);
    assert_eq!(next.docs.len(), before + 1);
    let d = next.ds().unwrap();
    assert_eq!(d.doc.name, "Poster");
    assert!(d.is_dirty(), "a recovered file has unsaved changes");
    assert!(d.path.is_none(), "saving it asks where");
}

#[test]
fn the_latest_edit_is_kept_and_a_saved_file_is_not() {
    let mut s = edited("First");
    let mut u = Unsaved::default();
    u.follow(&s);
    u.follow(&s);
    assert_eq!(u.count(), 1, "following again without a change keeps the one copy");
    s.execute("file.rename", &json!({"name": "Later"})).unwrap();
    u.follow(&s);
    let mut next = Session::new();
    assert_eq!(restore(&mut next, stored(&u)), 1);
    assert_eq!(next.ds().unwrap().doc.name, "Later");
    s.execute("file.markSaved", &json!({})).unwrap();
    u.follow(&s);
    assert_eq!(u.count(), 0);
    assert!(u.files().is_empty());
}

#[test]
fn what_the_storage_has_no_room_for_is_left_out() {
    let files = vec!["a".repeat(10), "b".repeat(100), "c".repeat(10)];
    let mut kept = String::new();
    let n = store(files, |v| {
        let fits = v.len() < 60;
        if fits {
            kept = v.to_string();
        }
        fits
    });
    assert_eq!(n, 2);
    assert_eq!(kept, json!(["a".repeat(10), "c".repeat(10)]).to_string());
    assert_eq!(store(vec!["x".into()], |_| false), 0);
}

#[test]
fn junk_in_the_storage_opens_nothing() {
    let mut s = Session::new();
    let before = s.docs.len();
    for junk in [None, Some("not json"), Some("{}"), Some(r#"["!!"]"#), Some(r#"["aGVsbG8="]"#), Some("[1, null]")] {
        assert_eq!(restore(&mut s, junk.map(String::from)), 0, "{junk:?}");
    }
    assert_eq!(s.docs.len(), before);
}

#[test]
fn the_stopped_page_says_what_was_kept() {
    let says = |unsaved, kept, what: &str| {
        let text = browser_stopped(unsaved, kept);
        assert!(text.starts_with("FigCraft stopped because of an internal error. ") && text.contains(what), "{text}");
    };
    says(0, 0, "start again");
    says(2, 2, "get your unsaved files back");
    says(2, 0, "too large for the browser to keep");
    says(3, 1, "the browser had room for");
}

#[test]
fn a_copy_taken_while_a_version_is_previewed_is_of_the_file() {
    let mut s = edited("Poster");
    s.execute("file.saveVersion", &json!({"name": "v1"})).unwrap();
    s.execute("file.rename", &json!({"name": "Later"})).unwrap();
    // Version v1 (named Poster) shows in place of the file.
    s.execute("version.preview", &json!({"index": 0})).unwrap();
    assert_eq!(s.doc().unwrap().name, "Poster", "the preview shows");
    let mut u = Unsaved::default();
    u.follow(&s);
    let mut next = Session::new();
    assert_eq!(restore(&mut next, stored(&u)), 1);
    let d = next.ds().unwrap();
    assert_eq!(d.doc.name, "Later", "the file comes back, not the version shown");
    assert_eq!(d.versions.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(), ["v1"]);
}
