//! A save confirmed some time after its bytes were taken (a page hosting the browser build stores
//! them, then answers): the file is saved as of those bytes, never as of edits made meanwhile.

use serde_json::json;

use crate::Session;
use crate::cmd::file::FileSnapshot;

fn rect(s: &mut Session, x: f64) {
    s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 10, "height": 10})).unwrap();
}

#[test]
fn a_confirmed_save_with_no_edits_since_clears_the_unsaved_changes() {
    let mut s = Session::new();
    rect(&mut s, 0.0);
    let snapshot = FileSnapshot::of(s.ds().unwrap());
    assert!(s.ds().unwrap().is_dirty());
    assert!(snapshot.delivered(&mut s));
    assert!(!s.ds().unwrap().is_dirty());
}

#[test]
fn edits_made_while_the_save_was_on_its_way_stay_unsaved() {
    let mut s = Session::new();
    rect(&mut s, 0.0);
    let snapshot = FileSnapshot::of(s.ds().unwrap());
    rect(&mut s, 20.0);
    assert!(snapshot.delivered(&mut s));
    assert!(s.ds().unwrap().is_dirty(), "the second rectangle never reached the host");
}

#[test]
fn a_confirmation_for_a_tab_saved_or_closed_since_changes_nothing() {
    let mut s = Session::new();
    rect(&mut s, 0.0);
    let snapshot = FileSnapshot::of(s.ds().unwrap());
    s.execute("file.markSaved", &json!({})).unwrap();
    rect(&mut s, 20.0);
    assert!(!snapshot.delivered(&mut s), "another save happened since");
    assert!(s.ds().unwrap().is_dirty());

    let snapshot = FileSnapshot::of(s.ds().unwrap());
    s.execute("file.close", &json!({"force": true})).unwrap();
    assert!(!snapshot.delivered(&mut s), "the tab is gone");
}
