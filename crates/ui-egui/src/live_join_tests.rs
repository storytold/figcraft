//! The Join dialog's keyboard (its field takes the keys when it opens; Escape closes it without
//! reaching the canvas), opening the app with an invite link, and the browser's Share popover
//! (it joins but can't host).

use egui::{Event, Key};
use figcraft_engine::live::{Event as LiveEvent, HostOptions, LiveLink, Presence, SessionInfo, Status};
use serde_json::json;

use crate::live_tests::{Harness, has_text, text_center};

fn escape(h: &mut Harness) {
    for pressed in [true, false] {
        h.frame(vec![Event::Key { key: Key::Escape, physical_key: None, pressed, repeat: false, modifiers: Default::default() }]);
    }
}

fn link_focused(h: &Harness) -> bool {
    h.ctx.memory(|m| m.has_focus(egui::Id::new("live_join_link")))
}

#[test]
fn escape_closes_the_join_dialog_and_keeps_the_selection() {
    let (mut h, rect) = Harness::new();
    h.app.session.execute("select.set", &json!({"ids": [rect.0]})).unwrap();
    crate::menus::invoke(&mut h.app, "collab.join");
    h.frames(2);
    assert!(h.app.ui.live.join.is_some());
    assert!(link_focused(&h), "the link field takes the keyboard when the dialog opens");
    // Escape while typing in the field.
    escape(&mut h);
    assert!(h.app.ui.live.join.is_none(), "Escape closes the dialog");
    assert_eq!(h.app.session.ds().unwrap().selection, vec![rect], "and doesn't deselect on the canvas");
    // Escape with the field not focused (after clicking the dialog's text).
    crate::menus::invoke(&mut h.app, "collab.join");
    let shapes = h.frames(2);
    h.click(text_center(&shapes, "Type the join code you were given, or paste an invite link."));
    h.frames(1);
    assert!(!link_focused(&h));
    escape(&mut h);
    assert!(h.app.ui.live.join.is_none());
    assert_eq!(h.app.session.ds().unwrap().selection, vec![rect]);
    // With the dialog closed, Escape deselects as usual.
    h.click(egui::pos2(700.0, 500.0));
    h.app.session.execute("select.set", &json!({"ids": [rect.0]})).unwrap();
    escape(&mut h);
    assert!(h.app.session.ds().unwrap().selection.is_empty());
}

#[test]
fn opening_the_app_with_an_invite_link_asks_before_joining() {
    let (mut h, _) = Harness::new();
    let link = "figcraft://join?host=127.0.0.1:47900&file=abc&token=AbCdEfGhIjKlMnOpQrStUv";
    crate::live::open_invite(&mut h.app, &format!(" {link} "));
    // The system hands the app whatever `figcraft://` link was followed, which may come from a
    // page or a message, so the link is shown and confirmed rather than acted on.
    assert_eq!(h.app.ui.live.join.as_deref(), Some(link), "the dialog opens with the link filled in");
    assert_eq!(h.app.session.live().status, Status::Off, "nothing is joined until the person confirms");
}

/// The browser's link: joins, can't host; joining fails here to show the dialog path.
struct Browser;

impl LiveLink for Browser {
    fn can_host(&self) -> bool {
        false
    }
    fn host(&mut self, _: u64, _: HostOptions) -> Result<SessionInfo, String> {
        Err("hosting a live session needs the desktop app".into())
    }
    fn join(&mut self, _: u64, _: &str, _: &str) -> Result<(), String> {
        Err("that's not a live session invite".into())
    }
    fn leave(&mut self, _: u64) {}
    fn presence(&mut self, _: u64, _: &Presence) {}
    fn send(&mut self, _: u64, _: figcraft_engine::figcraft_collab::message::ClientMsg) {}
    fn poll(&mut self) -> Vec<(u64, LiveEvent)> {
        Vec::new()
    }
}

#[test]
fn a_link_that_cant_be_used_opens_the_dialog_with_it() {
    let (mut h, _) = Harness::new();
    h.app.session.live.install(Box::new(Browser));
    crate::live::open_invite(&mut h.app, "figcraft://nope");
    assert_eq!(h.app.ui.live.join.as_deref(), Some("figcraft://nope"), "the link stays in the field to fix");
    // The link is not tried until the person presses Join, so its reason shows then, not now.
    assert!(h.app.ui.last_error.is_none());
    assert_eq!(h.app.session.live().status, Status::Off);
}

#[test]
fn the_browser_joins_but_does_not_host() {
    let (mut h, _) = Harness::new();
    h.app.session.live.install(Box::new(Browser));
    assert!(!h.app.session.live.can_host());
    assert_eq!(h.app.session.execute("collab.peers", &json!({})).unwrap()["canHost"], false);
    let shapes = h.frames(3);
    h.click(text_center(&shapes, "Share"));
    let shapes = h.frames(2);
    assert!(has_text(&shapes, "Sharing a file needs the desktop app; here you can join one."));
    assert!(!has_text(&shapes, "Start sharing") && !has_text(&shapes, "Local network"));
    // The join field is right there, with what the browser can reach.
    assert!(has_text(&shapes, "Join with a code") && has_text(&shapes, "Join"));
    assert!(has_text(
        &shapes,
        "In the browser you can join sessions the desktop app hosts and files on a FigCraft server. A page opened over https reaches other computers only through wss:// links (a server behind TLS)."
    ));
}
