//! A prototype's Open link action carries a URL from the document and fires from whatever trigger
//! the prototype sets, including a delay, so the player hands the opener only a web link
//! ([`figcraft_engine::doc_urls`]). Without that, a saved document opens `javascript:` in the web
//! build's own origin (stored XSS) and `smb:`/`ms-…:` through the desktop's handlers. Every
//! openable link waits behind a notice that it leaves the prototype — to a website or to the
//! mail app (#1096). Skipping the notice opens at once only a link the viewer's own click, press,
//! drag or key fired, one per frame; a delay or the pointer passing over a layer never opens
//! anything unasked. Before, a mail link opened at once however it was fired, and with the notice
//! skipped any trigger opened any number of links: an unattended presentation could open links and
//! mail messages one after another.

use std::sync::{Arc, Mutex};

use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::Services;
use crate::present_tests::{Ui as Harness, id};

/// Records every URL the player asks the platform to open, and every save of the preferences.
#[derive(Default)]
struct Opened(Arc<Mutex<Vec<String>>>, Arc<Mutex<Vec<figcraft_engine::Prefs>>>);

impl Services for Opened {
    fn open_url(&mut self, url: &str) {
        if let Ok(mut v) = self.0.lock() {
            v.push(url.to_string());
        }
    }

    /// Recorded apart from [`Services::open_url`], so a test tells the two apart.
    fn open_url_here(&mut self, url: &str) {
        if let Ok(mut v) = self.0.lock() {
            v.push(format!("here {url}"));
        }
    }

    fn save_prefs(&mut self, prefs: &figcraft_engine::Prefs) {
        if let Ok(mut v) = self.1.lock() {
            v.push(prefs.clone());
        }
    }
}

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

/// Frame A presented by `cmd` (`proto.present` or `proto.preview`) with the notice skipped or
/// not, its `interaction` (a trigger and maybe `delayMs`) opening each of `urls`. Returns the
/// harness and what the player opened.
fn playing(cmd: &str, skip_notice: bool, interaction: Value, urls: &[&str]) -> (Harness, Opened) {
    let mut s = Session::new();
    s.prefs.link_notice_skipped = skip_notice;
    let f = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 400, "name": "A"})));
    let mut it = interaction;
    it["id"] = json!(f);
    it["actions"] = urls.iter().map(|u| json!({"type": "openUrl", "url": u})).collect();
    it["transition"] = json!({"type": "instant"});
    ex(&mut s, "proto.interaction.add", it);
    present(cmd, s, f)
}

/// `s` presented by `cmd` from frame `start`, with the opener and the preferences recorded.
fn present(cmd: &str, s: Session, start: figcraft_doc::NodeId) -> (Harness, Opened) {
    let seen = Opened::default();
    let mut u = Harness::new(s);
    u.app.services = Box::new(Opened(seen.0.clone(), seen.1.clone()));
    u.ctx.enable_accesskit();
    u.run(cmd, json!({"start": start}));
    u.frames(2, Vec::new);
    (u, seen)
}

/// A presented frame that opens `url` 100ms after the screen appears, with no click anywhere,
/// and with the notice skipped or not. Returns the harness and what the player opened.
fn presented(url: &str, skip_notice: bool) -> (Harness, Opened) {
    played("proto.present", url, skip_notice)
}

/// [`presented`], played by `cmd` (`proto.present` or `proto.preview`).
fn played(cmd: &str, url: &str, skip_notice: bool) -> (Harness, Opened) {
    let (mut u, seen) = playing(cmd, skip_notice, json!({"trigger": {"type": "afterDelay", "ms": 100}}), &[url]);
    // Well past the delay.
    u.frames(40, Vec::new);
    (u, seen)
}

/// A click on the presented frame, and a few frames for what it fires.
fn click_the_frame(u: &mut Harness) {
    let at = egui::pos2(700.0, 290.0);
    u.button(at, true);
    u.button(at, false);
    u.frames(3, Vec::new);
}

/// What a click on a frame whose click opens `url` opens, with the notice skipped.
fn opens_on_a_click(url: &str) -> Vec<String> {
    let (mut u, seen) = playing("proto.present", true, json!({"trigger": {"type": "onClick"}}), &[url]);
    click_the_frame(&mut u);
    opened(&seen)
}

fn opened(seen: &Opened) -> Vec<String> {
    seen.0.lock().map(|v| v.clone()).unwrap_or_default()
}

/// The URL the presenter's notice waits to open.
fn notice(u: &Harness) -> Value {
    u.app.inspect()["presenter"]["linkNotice"].clone()
}

/// The labels of the controls drawn now.
fn labels(u: &mut Harness) -> Vec<String> {
    let tree = u.frame_output(vec![]).platform_output.accesskit_update.unwrap().nodes;
    tree.into_iter().filter_map(|(_, n)| n.label().map(str::to_string)).collect()
}

/// Clicks the control labeled `label`.
fn click(u: &mut Harness, label: &str) {
    let tree = u.frame_output(vec![]).platform_output.accesskit_update.unwrap().nodes;
    let n = tree.into_iter().map(|(_, n)| n).find(|n| n.label() == Some(label)).unwrap_or_else(|| panic!("no {label:?}"));
    let b = n.bounds().unwrap();
    let at = egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32);
    u.button(at, true);
    u.button(at, false);
}

#[test]
fn a_prototype_opens_web_links() {
    // The viewer chose to skip the notice: their yes, kept, for the links they click.
    assert_eq!(opens_on_a_click("https://example.com/a"), ["https://example.com/a"]);
    assert_eq!(opens_on_a_click("mailto:a@b.c"), ["mailto:a@b.c"]);
    // With the notice on a mail link waits behind it like a website: nothing reaches the mail
    // app, or any opener, unasked (#1096).
    let (mut u, seen) = presented("mailto:a@b.c", false);
    assert!(opened(&seen).is_empty(), "a mail link fired without a click waits behind the notice");
    assert_eq!(notice(&u), "mailto:a@b.c");
    click(&mut u, "Open link");
    assert_eq!(opened(&seen), ["mailto:a@b.c"]);
    assert!(notice(&u).is_null());
}

/// Skipping the notice covers what the viewer does, not what the prototype does by itself: a link
/// on a delay still waits behind the notice, which then offers no "Don't show this again", as
/// that couldn't cover it (#1096).
#[test]
fn a_skipped_notice_still_shows_for_a_link_no_click_fired() {
    for url in ["https://example.com/a", "mailto:a@b.c"] {
        let (mut u, seen) = presented(url, true);
        assert!(opened(&seen).is_empty(), "{url}: a delay opened it with the notice skipped");
        assert_eq!(notice(&u), url);
        assert!(!labels(&mut u).iter().any(|l| l == "Don't show this again"), "{url}: the checkbox can't skip this notice");
        click(&mut u, "Open link");
        assert_eq!(opened(&seen), [url]);
    }
}

/// A clicked link the notice first shows for offers "Don't show this again" (#1096).
#[test]
fn the_notice_for_a_clicked_link_offers_to_skip_it() {
    let (mut u, _) = playing("proto.present", false, json!({"trigger": {"type": "onClick"}}), &["https://example.com/a"]);
    click_the_frame(&mut u);
    assert_eq!(notice(&u), "https://example.com/a");
    assert!(labels(&mut u).iter().any(|l| l == "Don't show this again"));
}

/// With the notice skipped one click opens one link, however many its interaction opens: the
/// next waits behind the notice and the rest are dropped (#1096).
#[test]
fn a_click_opens_one_link_with_the_notice_skipped() {
    let urls = ["https://example.com/1", "https://example.com/2", "mailto:a@b.c"];
    let (mut u, seen) = playing("proto.present", true, json!({"trigger": {"type": "onClick"}}), &urls);
    click_the_frame(&mut u);
    assert_eq!(opened(&seen), ["https://example.com/1"]);
    assert_eq!(notice(&u), "https://example.com/2");
}

/// The pointer passing over a layer is not the viewer asking for a link: with the notice skipped,
/// a mouse enter's link still waits behind it (#1096).
#[test]
fn a_link_the_pointer_fires_by_passing_waits_with_the_notice_skipped() {
    let (mut u, seen) = playing("proto.present", true, json!({"trigger": {"type": "mouseEnter"}}), &["https://example.com/hover"]);
    u.frame(vec![egui::Event::PointerMoved(egui::pos2(700.0, 290.0))]);
    u.frames(3, Vec::new);
    assert!(opened(&seen).is_empty(), "passing over the layer opened it");
    assert_eq!(notice(&u), "https://example.com/hover");
}

/// A presentation whose prototype queues several mail links on delays opens none of them by
/// itself: the first waits behind the notice and the rest are dropped with it (#1096).
#[test]
fn delayed_opens_do_not_pile_up_unattended() {
    let mut s = Session::new();
    s.prefs.link_notice_skipped = true;
    let f = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 400, "name": "A"})));
    for ms in [100u32, 200, 300] {
        let layer = id(&ex(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 30, "parent": f})));
        ex(
            &mut s,
            "proto.interaction.add",
            json!({"id": layer, "trigger": {"type": "afterDelay", "ms": ms}, "action": {"type": "openUrl", "url": format!("mailto:victim@example.com?subject=spam{ms}")}}),
        );
    }
    let (mut u, seen) = present("proto.present", s, f);
    u.frames(40, Vec::new); // no click, no key: the presentation alone
    assert!(opened(&seen).is_empty(), "a presentation by itself opens nothing, the notice skipped or not");
    assert_eq!(notice(&u), "mailto:victim@example.com?subject=spam100", "the first waits, the rest are dropped");
    click(&mut u, "Open link");
    assert_eq!(opened(&seen), ["mailto:victim@example.com?subject=spam100"]);
}

/// A mail link the viewer clicks is still leaving the prototype: it waits behind the notice like
/// every other link (#1096). Before, a mail link opened straight away, trigger or no trigger.
#[test]
fn a_clicked_mail_link_waits_behind_the_notice() {
    let (mut u, seen) = playing("proto.present", false, json!({"trigger": {"type": "onClick"}}), &["mailto:a@b.c?subject=hello"]);
    click_the_frame(&mut u);
    assert!(opened(&seen).is_empty(), "the notice comes first, whatever fired the link");
    assert_eq!(notice(&u), "mailto:a@b.c?subject=hello");
    click(&mut u, "Open link");
    assert_eq!(opened(&seen), ["mailto:a@b.c?subject=hello"]);
}

/// A mouse-up whose actions run on a delay fires once the click is long over; its link waits
/// behind the notice like a timed one does, the notice skipped or not (#1096).
#[test]
fn a_link_a_click_queued_to_fire_later_waits_behind_the_notice() {
    for skip in [false, true] {
        let (mut u, seen) = playing("proto.present", skip, json!({"trigger": {"type": "mouseUp"}, "delayMs": 150}), &["https://example.com/late"]);
        click_the_frame(&mut u);
        u.frames(20, Vec::new); // well past the 150 ms the click queued
        assert!(opened(&seen).is_empty(), "skip {skip}: the actions fire after the click is over: not a gesture");
        assert_eq!(notice(&u), "https://example.com/late");
    }
}

/// A link a key trigger fires waits behind the notice too, and opens at once with it skipped: a
/// key is the viewer's own doing (#1096).
#[test]
fn a_link_fired_by_a_key_waits_behind_the_notice() {
    for skip in [false, true] {
        let (mut u, seen) = playing("proto.present", skip, json!({"trigger": {"type": "keyPress", "key": "K"}}), &["https://example.com/keyed"]);
        u.key(egui::pos2(700.0, 450.0), egui::Key::K);
        u.frames(3, Vec::new);
        if skip {
            assert_eq!(opened(&seen), ["https://example.com/keyed"]);
        } else {
            assert!(opened(&seen).is_empty(), "a key is not a click");
            assert_eq!(notice(&u), "https://example.com/keyed");
        }
    }
}

#[test]
fn a_link_to_a_website_waits_for_the_notice() {
    let (mut u, seen) = presented("https://example.com/a", false);
    assert!(opened(&seen).is_empty(), "the link waits for the viewer");
    assert_eq!(notice(&u), "https://example.com/a");
    click(&mut u, "Open link");
    assert_eq!(opened(&seen), ["https://example.com/a"]);
    assert!(notice(&u).is_null());
    assert!(!u.app.session.prefs.link_notice_skipped && seen.1.lock().unwrap().is_empty(), "the notice shows next time");
}

#[test]
fn the_preview_pane_shows_the_notice_too() {
    let (mut u, seen) = played("proto.preview", "https://example.com/a", false);
    assert!(opened(&seen).is_empty());
    assert_eq!(u.app.inspect()["preview"]["linkNotice"], "https://example.com/a");
    click(&mut u, "Open link");
    assert_eq!(opened(&seen), ["https://example.com/a"]);
}

#[test]
fn cancel_or_escape_drops_the_link() {
    let (mut u, seen) = presented("https://example.com/a", false);
    click(&mut u, "Cancel");
    assert!(notice(&u).is_null() && opened(&seen).is_empty());
    let (mut u, seen) = presented("https://example.com/a", false);
    u.key(egui::pos2(700.0, 450.0), egui::Key::Escape);
    assert!(notice(&u).is_null() && opened(&seen).is_empty());
    assert!(u.app.ui.present.is_some(), "Escape closes the notice, not the presentation");
}

#[test]
fn the_prototype_takes_no_keys_while_the_notice_shows() {
    let mut s = Session::new();
    let a = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 400, "name": "A"})));
    let b = id(&ex(&mut s, "frame.create", json!({"x": 400, "y": 0, "width": 300, "height": 400, "name": "B"})));
    let layer = id(&ex(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 30, "parent": a})));
    let link = json!({"type": "openUrl", "url": "https://example.com/a"});
    ex(&mut s, "proto.interaction.add", json!({"id": layer, "trigger": {"type": "afterDelay", "ms": 100}, "action": link}));
    let to_b = json!({"type": "navigate", "dest": b});
    ex(&mut s, "proto.interaction.add", json!({"id": a, "trigger": {"type": "keyPress", "key": "Escape"}, "action": to_b}));
    let mut u = Harness::new(s);
    u.app.services = Box::new(Opened::default());
    u.run("proto.present", json!({"start": a}));
    u.frames(40, Vec::new);
    assert_eq!(notice(&u), "https://example.com/a");
    u.key(egui::pos2(700.0, 450.0), egui::Key::Escape);
    assert!(notice(&u).is_null(), "Escape cancels the notice");
    assert_eq!(u.app.inspect()["presenter"]["current"], json!(a), "the prototype's Escape trigger did not run");
}

#[test]
fn the_presenter_keys_wait_while_the_notice_shows() {
    let (mut u, _) = presented("https://example.com/a", false);
    assert_eq!(notice(&u), "https://example.com/a");
    let press = |key, modifiers| {
        let k = |pressed| egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers };
        vec![k(true), k(false)]
    };
    let out = u.frame_output(press(egui::Key::F, egui::Modifiers::NONE));
    let cmds = out.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.commands.clone()).unwrap_or_default();
    assert!(!cmds.iter().any(|c| matches!(c, egui::ViewportCommand::Fullscreen(_))), "F: {cmds:?}");
    u.frame(press(egui::Key::C, egui::Modifiers::NONE));
    assert_eq!(u.app.inspect()["presenter"]["comments"], false, "C");
    u.frame(press(egui::Key::C, egui::Modifiers::SHIFT));
    assert_eq!(u.app.inspect()["presenter"]["comments"], false, "Shift+C");
    assert_eq!(notice(&u), "https://example.com/a");
}

#[test]
fn the_prototype_takes_no_controller_buttons_while_the_notice_shows() {
    let mut s = Session::new();
    let a = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 400, "name": "A"})));
    let b = id(&ex(&mut s, "frame.create", json!({"x": 400, "y": 0, "width": 300, "height": 400, "name": "B"})));
    let layer = id(&ex(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 30, "parent": a})));
    let link = json!({"type": "openUrl", "url": "https://example.com/a"});
    ex(&mut s, "proto.interaction.add", json!({"id": layer, "trigger": {"type": "afterDelay", "ms": 100}, "action": link}));
    let to_b = json!({"type": "navigate", "dest": b});
    ex(&mut s, "proto.interaction.add", json!({"id": a, "trigger": {"type": "keyPress", "key": "Gamepad A"}, "action": to_b}));
    let mut u = Harness::new(s);
    u.app.services = Box::new(Opened::default());
    u.ctx.enable_accesskit();
    u.run("proto.present", json!({"start": a}));
    u.frames(40, Vec::new);
    assert_eq!(notice(&u), "https://example.com/a");
    let ctx = u.ctx.clone();
    crate::control::handle(&mut u.app, &ctx, "ui.input", &json!({"events": [{"gamepad": "A"}]})).unwrap();
    u.frames(2, Vec::new);
    assert_eq!(u.app.inspect()["presenter"]["current"], json!(a), "the Gamepad A trigger did not run");
    assert_eq!(notice(&u), "https://example.com/a");
    click(&mut u, "Cancel");
    assert!(notice(&u).is_null());
    crate::control::handle(&mut u.app, &ctx, "ui.input", &json!({"events": [{"gamepad": "A"}]})).unwrap();
    u.frames(2, Vec::new);
    assert_eq!(u.app.inspect()["presenter"]["current"], json!(b), "the button plays once the notice closes");
}

#[test]
fn dont_show_this_again_is_saved_with_the_preferences() {
    let (mut u, seen) = playing("proto.present", false, json!({"trigger": {"type": "onClick"}}), &["https://example.com/a"]);
    click_the_frame(&mut u);
    click(&mut u, "Don't show this again");
    click(&mut u, "Open link");
    assert_eq!(opened(&seen), ["https://example.com/a"]);
    assert!(u.app.session.prefs.link_notice_skipped);
    assert!(seen.1.lock().unwrap().last().is_some_and(|p| p.link_notice_skipped), "the choice is saved");
    assert_eq!(u.run("proto.linkNotice", json!({})), json!({"shown": true}));
    assert!(!u.app.session.prefs.link_notice_skipped);
}

/// Skipping the notice with the command (the Actions palette, the control channel, MCP) is saved
/// at once as well, not only when the app quits, and a new session started from the saved
/// preferences skips it.
#[test]
fn skipping_the_notice_by_command_is_saved_at_once() {
    let (mut u, seen) = playing("proto.present", false, json!({"trigger": {"type": "onClick"}}), &["https://example.com/a"]);
    let before = seen.1.lock().unwrap().len();
    assert_eq!(u.run("proto.linkNotice", json!({"show": false})), json!({"shown": false}));
    u.frames(1, Vec::new);
    let saves = seen.1.lock().unwrap().clone();
    assert_eq!(saves.len(), before + 1, "saved once");
    let saved = saves.last().unwrap();
    assert!(saved.link_notice_skipped, "the choice is saved");
    // As the app keeps them: JSON, read back at the next start.
    let mut next = Session::new();
    next.prefs = serde_json::from_value(serde_json::to_value(saved).unwrap()).unwrap();
    assert!(next.prefs.link_notice_skipped, "a new session reads it back");
    u.frames(1, Vec::new);
    assert_eq!(seen.1.lock().unwrap().len(), before + 1, "and not saved again every frame");
}

#[test]
fn a_prototype_cannot_open_a_script_or_handler_url() {
    for url in [
        // In the web build the opener is `window.open`, which runs this in the app's own origin.
        "javascript:alert(document.cookie)",
        "JavaScript:alert(1)",
        "data:text/html,<script>alert(1)</script>",
        // Reaches the file system; a UNC host also sends Windows credentials.
        "file:///etc/passwd",
        "smb://attacker.example/share/x",
        // Handed to an application handler by the desktop.
        "ms-msdt:/id PCWDiagnostic",
        "figcraft://join?host=127.0.0.1:47900&file=f&token=t",
    ] {
        assert!(opens_on_a_click(url).is_empty(), "{url:?} must never reach the opener");
    }
}

#[test]
fn the_prototype_takes_no_clicks_while_the_notice_shows() {
    let mut s = Session::new();
    let a = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 400, "name": "A"})));
    let b = id(&ex(&mut s, "frame.create", json!({"x": 400, "y": 0, "width": 300, "height": 400, "name": "B"})));
    let layer = id(&ex(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 30, "parent": a})));
    let link = json!({"type": "openUrl", "url": "https://example.com/a"});
    ex(&mut s, "proto.interaction.add", json!({"id": layer, "trigger": {"type": "afterDelay", "ms": 100}, "action": link}));
    let to_b = json!({"type": "navigate", "dest": b});
    ex(&mut s, "proto.interaction.add", json!({"id": a, "trigger": {"type": "onClick"}, "action": to_b, "transition": {"type": "instant"}}));
    let mut u = Harness::new(s);
    u.app.services = Box::new(Opened::default());
    u.run("proto.present", json!({"start": a}));
    u.frames(40, Vec::new);
    assert_eq!(notice(&u), "https://example.com/a");
    // On the frame, above the notice.
    let at = egui::pos2(700.0, 290.0);
    u.button(at, true);
    u.button(at, false);
    u.frames(5, Vec::new);
    assert_eq!(u.app.inspect()["presenter"]["current"], json!(a), "the click behind the notice did not navigate");
    assert_eq!(notice(&u), "https://example.com/a", "the notice still waits");
    let shapes = crate::live_tests::flat(&u.frame_output(vec![]).shapes);
    crate::live_tests::text_center(&shapes, "Open link");
    u.key(egui::pos2(700.0, 450.0), egui::Key::Escape);
    assert!(notice(&u).is_null());
    u.button(at, true);
    u.button(at, false);
    u.frames(5, Vec::new);
    assert_eq!(u.app.inspect()["presenter"]["current"], json!(b), "with the notice gone the same click navigates");
}

/// While a notice shows nothing else opens, even a link the viewer fired with the notice skipped:
/// a press that opened one link and put the next behind the notice then releases over a
/// mouse-up link, which is dropped with the notice up (#1096).
#[test]
fn nothing_opens_while_a_notice_shows_with_the_notice_skipped() {
    let mut s = Session::new();
    s.prefs.link_notice_skipped = true;
    let f = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 400, "name": "A"})));
    let open = |u: &str| json!({"type": "openUrl", "url": u});
    ex(
        &mut s,
        "proto.interaction.add",
        json!({"id": f, "trigger": {"type": "mouseDown"}, "actions": [open("https://example.com/1"), open("https://example.com/2")], "transition": {"type": "instant"}}),
    );
    ex(
        &mut s,
        "proto.interaction.add",
        json!({"id": f, "trigger": {"type": "mouseUp"}, "actions": [open("https://example.com/3")], "transition": {"type": "instant"}}),
    );
    let (mut u, seen) = present("proto.present", s, f);
    let at = egui::pos2(700.0, 290.0);
    u.button(at, true);
    u.frames(2, Vec::new);
    assert_eq!(opened(&seen), ["https://example.com/1"]);
    assert_eq!(notice(&u), "https://example.com/2");
    u.button(at, false);
    u.frames(3, Vec::new);
    assert_eq!(opened(&seen), ["https://example.com/1"], "the release opened a link with the notice up");
    assert_eq!(notice(&u), "https://example.com/2");
}

/// The prototype takes no input while a notice shows, including the release of a press that
/// began before it: a mouse-up that navigates doesn't run behind the notice (#1096).
#[test]
fn a_press_that_raised_a_notice_doesnt_release_behind_it() {
    let mut s = Session::new();
    s.prefs.link_notice_skipped = true;
    let a = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 400, "name": "A"})));
    let b = id(&ex(&mut s, "frame.create", json!({"x": 400, "y": 0, "width": 300, "height": 400, "name": "B"})));
    let open = |u: &str| json!({"type": "openUrl", "url": u});
    ex(
        &mut s,
        "proto.interaction.add",
        json!({"id": a, "trigger": {"type": "mouseDown"}, "actions": [open("https://example.com/1"), open("https://example.com/2")], "transition": {"type": "instant"}}),
    );
    ex(
        &mut s,
        "proto.interaction.add",
        json!({"id": a, "trigger": {"type": "mouseUp"}, "action": {"type": "navigate", "dest": b}, "transition": {"type": "instant"}}),
    );
    let (mut u, _) = present("proto.present", s, a);
    let at = egui::pos2(700.0, 290.0);
    u.button(at, true);
    u.frames(2, Vec::new);
    assert_eq!(notice(&u), "https://example.com/2");
    u.button(at, false);
    u.frames(3, Vec::new);
    assert_eq!(u.app.inspect()["presenter"]["current"], json!(a), "the release navigated behind the notice");
}

/// An Open link action with "Open in new tab" off opens in place of the prototype (the web build's
/// own tab), through the notice or with it skipped; on, the default, it opens a new tab.
#[test]
fn a_link_can_open_in_place() {
    for skip in [true, false] {
        let mut s = Session::new();
        s.prefs.link_notice_skipped = skip;
        let f = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 400, "name": "A"})));
        let action = json!({"type": "openUrl", "url": "https://example.com/a", "openInNewTab": false});
        ex(&mut s, "proto.interaction.add", json!({"id": f, "trigger": {"type": "onClick"}, "actions": [action], "transition": {"type": "instant"}}));
        let (mut u, seen) = present("proto.present", s, f);
        click_the_frame(&mut u);
        if !skip {
            click(&mut u, "Open link");
        }
        assert_eq!(opened(&seen), ["here https://example.com/a"], "notice skipped: {skip}");
    }
    assert_eq!(opens_on_a_click("https://example.com/b"), ["https://example.com/b"]);
}

/// The interaction editor's "Open in new tab" box turns the setting off and back on, keeping the
/// link.
#[test]
fn the_editor_turns_open_in_new_tab_off_and_on() {
    let mut s = Session::new();
    let f = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 400, "name": "A"})));
    let action = json!({"type": "openUrl", "url": "https://example.com/a"});
    ex(&mut s, "proto.interaction.add", json!({"id": f, "trigger": {"type": "onClick"}, "actions": [action]}));
    ex(&mut s, "select.set", json!({"ids": [f]}));
    let mut u = Harness::new(s);
    u.ctx.enable_accesskit();
    u.app.ui.right_tab = crate::RightTab::Prototype;
    crate::proto_panel::open_editor(&u.ctx, Some((f, 0)));
    u.frames(3, Vec::new);
    for new_tab in [false, true] {
        click(&mut u, "Open in new tab");
        u.frames(2, Vec::new);
        let doc = u.app.session.doc().unwrap();
        let url = "https://example.com/a".to_string();
        assert_eq!(doc.node(f).unwrap().interactions[0].actions, [figcraft_doc::proto::Action::OpenUrl { url, new_tab }]);
    }
}
