//! The Join dialog follows a join from the click until its file is ready: joining (with Cancel),
//! then the file in a tab of its own; or why it failed, with Try again (#1381).

use egui::{Event, Key};
use figcraft_engine::live::{Event as LiveEvent, Status};
use serde_json::json;

use crate::live_tests::{Harness, has_text, text_center};

const LINK: &str = "figcraft://join?host=127.0.0.1:47900&file=f&token=t";

/// Join from the Join dialog with the invite link: the session it started.
fn join(h: &mut Harness) -> u64 {
    crate::live::open_invite(&mut h.app, LINK);
    // A new window measures itself first.
    let shapes = h.frames(3);
    h.click(text_center(&shapes, "Join"));
    let shapes = h.frames(1);
    assert!(has_text(&shapes, "Joining live session…") && has_text(&shapes, "Cancel"), "the dialog shows the join at once");
    let (id, typed) = h.app.ui.live.joining.clone().unwrap();
    assert_eq!(typed, LINK);
    id
}

fn joining_shown(h: &mut Harness) -> bool {
    has_text(&h.frames(2), "Joining live session…")
}

#[test]
fn joining_shows_until_the_file_is_ready() {
    let (mut h, _) = Harness::new();
    let id = join(&mut h);
    // One join at a time: the home screen's field waits.
    h.app.ui.home = true;
    h.app.ui.live.code = LINK.into();
    let shapes = h.frames(2);
    h.click(text_center(&shapes, "Join"));
    assert_eq!(h.app.ui.live.joining.as_ref().map(|j| j.0), Some(id), "no second join");
    assert_eq!(h.app.session.live.iter().count(), 1);
    // Welcomed, but the file isn't here yet: still joining.
    h.welcome();
    assert!(joining_shown(&mut h));
    h.sync();
    assert!(!joining_shown(&mut h) && h.app.ui.live.joining.is_none(), "it goes once the file is ready");
    assert!(!h.app.ui.home, "and the joined file shows");
    assert_eq!(h.app.session.docs.len(), 2);
}

#[test]
fn a_failed_join_says_why_and_tries_again() {
    let (mut h, _) = Harness::new();
    let first = join(&mut h);
    h.send(vec![LiveEvent::Status(Status::Failed { reason: "Couldn't reach 127.0.0.1:47900".into() })]);
    let shapes = h.frames(2);
    assert!(has_text(&shapes, "Couldn't reach 127.0.0.1:47900") && has_text(&shapes, "Try again"));
    assert!(!has_text(&shapes, "Joining live session…"));
    h.click(text_center(&shapes, "Try again"));
    let second = h.app.ui.live.joining.as_ref().map(|j| j.0).unwrap();
    assert_ne!(second, first, "a new join with the same link");
    assert!(h.app.session.live.get(first).is_none(), "the failed one is gone");
    assert_eq!(h.app.session.live.get(second).map(|l| l.status.clone()), Some(Status::Connecting));
    assert!(joining_shown(&mut h));
    h.welcome();
    h.sync();
    assert!(h.app.ui.live.joining.is_none() && h.app.session.live.get(second).is_some_and(|l| l.syncing()));
}

#[test]
fn cancel_and_escape_call_the_join_off() {
    let (mut h, _) = Harness::new();
    let id = join(&mut h);
    let shapes = h.frames(1);
    h.click(text_center(&shapes, "Cancel"));
    assert!(h.app.ui.live.joining.is_none() && h.app.session.live.get(id).is_none(), "the join ended");
    assert_eq!(h.app.session.live().status, Status::Off);
    assert!(!joining_shown(&mut h));
    // Escape is Cancel, and so is a failed join's Cancel (it is forgotten).
    let id = join(&mut h);
    for pressed in [true, false] {
        h.frame(vec![Event::Key { key: Key::Escape, physical_key: None, pressed, repeat: false, modifiers: Default::default() }]);
    }
    assert!(h.app.ui.live.joining.is_none() && h.app.session.live.get(id).is_none());
    let id = join(&mut h);
    h.send(vec![LiveEvent::Status(Status::Failed { reason: "No session with that code".into() })]);
    let shapes = h.frames(2);
    h.click(text_center(&shapes, "Cancel"));
    assert!(h.app.ui.live.joining.is_none() && h.app.session.live.get(id).is_none());
    assert!(!has_text(&h.frames(2), "Try again"));
}

#[test]
fn leave_takes_a_session_id() {
    let (mut h, _) = Harness::new();
    let id = h.app.session.execute("collab.join", &json!({"link": LINK})).unwrap()["session"].as_u64().unwrap();
    assert!(h.app.session.execute("collab.leave", &json!({"session": "one"})).is_err());
    assert_eq!(h.app.session.execute("collab.leave", &json!({"session": id + 7})).unwrap()["ended"], false, "an unknown one is no error");
    assert_eq!(h.app.session.execute("collab.leave", &json!({"session": id})).unwrap()["ended"], true);
    assert!(h.app.session.live.get(id).is_none());
}
