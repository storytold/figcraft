//! Comments in the UI, driven headless through egui (the harness is `present_tests`'): crowded
//! pins cluster and zoom apart, your pins drag onto another frame, Shift+C hides the pins, the
//! arrows step through threads, `@` offers names, Escape closes, the presenter shows the presented
//! frame's comments; and the helpers on their own.

use egui::{Event, Key, Modifiers, Pos2, pos2, vec2};
use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::comments::{cluster, insert_mention, mention_matches, mention_query, start_draft};
use crate::present_tests::{Ui, id};

fn add(s: &mut Session, x: f64, y: f64, text: &str, author: &str) -> u64 {
    s.execute("comment.add", &json!({"x": x, "y": y, "text": text, "author": author, "anchor": true})).unwrap()["id"].as_u64().unwrap()
}

/// The middle of the pin whose spot is page point (`x`, `y`).
fn pin_at(u: &Ui, x: f64, y: f64) -> Pos2 {
    u.app.to_screen(x, y) + vec2(14.0, -14.0)
}

fn click(u: &mut Ui, p: Pos2) {
    u.button(p, true);
    u.button(p, false);
    u.frame(vec![]);
}

/// Click the pin whose spot is page point (`x`, `y`).
fn click_pin(u: &mut Ui, x: f64, y: f64) {
    let p = pin_at(u, x, y);
    click(u, p);
}

fn key(u: &mut Ui, key: Key, modifiers: Modifiers) {
    let k = |pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers };
    u.frame(vec![k(true), k(false)]);
    u.frame(vec![]);
}

fn comments(u: &Ui) -> Value {
    u.app.inspect()["comments"].clone()
}

#[test]
fn switching_from_comment_to_move_closes_the_unposted_composer() {
    let mut u = Ui::new(Session::new());
    u.app.run("tool.set", json!({"tool": "comment"}));
    start_draft(&mut u.app, 100.0, 80.0);
    assert!(comments(&u)["draft"].is_array());

    u.app.run("tool.set", json!({"tool": "move"}));
    u.frame(vec![]);
    assert!(comments(&u)["draft"].is_null());
}

#[test]
fn control_channel_tool_switch_closes_the_unposted_composer() {
    let mut u = Ui::new(Session::new());
    u.app.run("tool.set", json!({"tool": "comment"}));
    start_draft(&mut u.app, 100.0, 80.0);
    assert!(comments(&u)["draft"].is_array());

    let invalid = json!({"command": "tool.set", "params": {"tool": "not-a-tool"}});
    assert!(crate::control::handle(&mut u.app, &egui::Context::default(), "engine.execute", &invalid).is_err());
    assert!(comments(&u)["draft"].is_array());

    crate::control::handle(&mut u.app, &egui::Context::default(), "engine.execute", &json!({"command": "tool.set", "params": {"tool": "move"}}))
        .unwrap();
    u.frame(vec![]);
    assert!(comments(&u)["draft"].is_null());
}

#[test]
fn mention_helpers() {
    // The `@query` before the cursor.
    assert_eq!(mention_query("Hi @Sa", 6), Some((3, 6)));
    assert_eq!(mention_query("@", 1), Some((0, 1)));
    assert_eq!(mention_query("Hi @Sam Lee", 11), Some((3, 11)));
    assert_eq!(mention_query("Hi @Sam ", 8), None, "a space ends it");
    assert_eq!(mention_query("mail@x", 6), None, "inside a word");
    assert_eq!(mention_query("@Sa\nx", 5), None, "on another line");
    assert_eq!(mention_query("é @Sa", 4), Some((3, 5)), "byte range, char cursor");
    assert_eq!(mention_query("Hi @Sa", 99), Some((3, 6)), "a cursor past the end");
    let people: Vec<String> = ["Ada Lovelace", "Sam", "Sam Lee", "You"].iter().map(|s| s.to_string()).collect();
    assert_eq!(mention_matches(&people, "sa", "You"), vec!["Sam", "Sam Lee"]);
    assert_eq!(mention_matches(&people, "love", "You"), vec!["Ada Lovelace"], "a later word");
    assert_eq!(mention_matches(&people, "", "You").len(), 3, "everyone but you");
    // Inserting replaces the query and leaves the cursor after a space.
    let mut t = String::from("Hi @Sa, ok");
    let at = insert_mention(&mut t, (3, 6), "Sam Lee");
    assert_eq!(t, "Hi @Sam Lee , ok");
    assert_eq!(at, 12);
    let mut t = String::from("é");
    assert_eq!(insert_mention(&mut t, (1, 2), "X"), 1, "not on a char boundary: unchanged");
    assert_eq!(insert_mention(&mut t, (5, 9), "X"), 1, "out of range: unchanged");
    assert_eq!(t, "é");
}

#[test]
fn clusters_group_nearby_points() {
    let pts = [pos2(0.0, 0.0), pos2(10.0, 0.0), pos2(100.0, 0.0), pos2(5.0, 12.0), pos2(300.0, 300.0)];
    assert_eq!(cluster(&pts, 24.0), vec![vec![0, 1, 3], vec![2], vec![4]]);
    assert!(cluster(&[], 24.0).is_empty());
    assert_eq!(cluster(&pts, 0.0).len(), 5);
}

#[test]
fn crowded_pins_cluster_and_a_click_zooms_them_apart() {
    let mut s = Session::new();
    s.execute("node.create", &json!({"type": "frame", "x": 0, "y": 0, "width": 400, "height": 400})).unwrap();
    for (i, x) in [100.0, 104.0, 108.0].into_iter().enumerate() {
        add(&mut s, x, 100.0, &format!("Note {i}"), "Sam");
    }
    let mut u = Ui::new(s);
    u.run("view.set", json!({"zoom": 1, "cx": 100, "cy": 100}));
    // Three pins a few points apart: one cluster.
    let zoom = u.app.session.ds().unwrap().view.zoom;
    let spot = pin_at(&u, 104.0, 100.0);
    click(&mut u, spot);
    let after = u.app.session.ds().unwrap().view.zoom;
    assert!(after >= zoom * 2.0, "{zoom} -> {after}");
    assert_eq!(comments(&u)["open"], Value::Null, "a cluster click opens no thread");
    // Zoomed in, they stand apart: a click opens one.
    u.run("view.set", json!({"zoom": 8, "cx": 104, "cy": 100}));
    click_pin(&mut u, 108.0, 100.0);
    assert_eq!(comments(&u)["open"], 3);
}

#[test]
fn your_pins_drag_onto_another_frame() {
    let mut s = Session::new();
    s.execute("node.create", &json!({"type": "frame", "x": 0, "y": 0, "width": 200, "height": 200})).unwrap();
    let b = id(&s.execute("node.create", &json!({"type": "frame", "x": 300, "y": 0, "width": 200, "height": 200})).unwrap());
    let mine = add(&mut s, 50.0, 50.0, "Mine", "You");
    let theirs = add(&mut s, 50.0, 150.0, "Theirs", "Sam");
    let mut u = Ui::new(s);
    u.run("view.set", json!({"zoom": 1, "cx": 250, "cy": 100}));
    let drag = |u: &mut Ui, from: Pos2, to: Pos2| {
        u.button(from, true);
        for k in 1..=4 {
            let p = from + (to - from) * (k as f32 / 4.0);
            u.frame(vec![Event::PointerMoved(p)]);
        }
        u.button(to, false);
        u.frame(vec![]);
    };
    let (from, to) = (pin_at(&u, 50.0, 50.0), pin_at(&u, 350.0, 60.0));
    drag(&mut u, from, to);
    let c = u.app.session.doc().unwrap().comment(mine).unwrap().clone();
    assert_eq!(c.anchor, Some(b));
    let (x, y) = c.position(u.app.session.doc().unwrap());
    assert!((x - 350.0).abs() < 0.01 && (y - 60.0).abs() < 0.01, "{x}, {y}");
    // Someone else's pin doesn't move: the press opens it instead.
    let (from, to) = (pin_at(&u, 50.0, 150.0), pin_at(&u, 350.0, 160.0));
    drag(&mut u, from, to);
    assert_eq!(u.app.session.doc().unwrap().comment(theirs).unwrap().position(u.app.session.doc().unwrap()), (50.0, 150.0));
    assert_eq!(comments(&u)["open"], theirs);
}

#[test]
fn shift_c_hides_the_pins_and_escape_closes_a_thread() {
    let mut s = Session::new();
    let c = add(&mut s, 100.0, 100.0, "Hello", "Sam");
    let mut u = Ui::new(s);
    u.run("view.set", json!({"zoom": 1, "cx": 100, "cy": 100}));
    assert!(u.app.session.ds().unwrap().view.comments);
    key(&mut u, Key::C, Modifiers::SHIFT);
    assert!(!u.app.session.ds().unwrap().view.comments);
    // Hidden pins don't take clicks.
    click_pin(&mut u, 100.0, 100.0);
    assert_eq!(comments(&u)["open"], Value::Null);
    key(&mut u, Key::C, Modifiers::SHIFT);
    click_pin(&mut u, 100.0, 100.0);
    assert_eq!(comments(&u)["open"], c);
    // Opening it read it.
    assert!(!u.app.session.doc().unwrap().comment(c).unwrap().is_unread("You"));
    key(&mut u, Key::Escape, Modifiers::NONE);
    assert_eq!(comments(&u)["open"], Value::Null);
}

#[test]
fn arrows_step_through_the_threads_with_the_comment_tool() {
    let mut s = Session::new();
    let a = add(&mut s, 0.0, 0.0, "First", "Sam");
    let b = add(&mut s, 900.0, 900.0, "Second", "Sam");
    if let Some(c) = s.doc_mut().unwrap().comment_mut(a) {
        c.first.created += 10;
    }
    let mut u = Ui::new(s);
    u.run("tool.comment", json!({}));
    // Newest first: `a`, then `b`; the view follows.
    key(&mut u, Key::ArrowDown, Modifiers::NONE);
    assert_eq!(comments(&u)["open"], a);
    key(&mut u, Key::ArrowDown, Modifiers::NONE);
    assert_eq!(comments(&u)["open"], b);
    let v = &u.app.session.ds().unwrap().view;
    assert_eq!((v.cx, v.cy), (900.0, 900.0));
    key(&mut u, Key::ArrowUp, Modifiers::NONE);
    assert_eq!(comments(&u)["open"], a);
    // Typing a reply keeps the arrows in the field.
    u.frame(vec![Event::Text("x".into())]);
    key(&mut u, Key::ArrowDown, Modifiers::NONE);
    assert_eq!(comments(&u)["open"], a);
}

#[test]
fn typing_at_offers_names_and_enter_inserts_one() {
    let mut s = Session::new();
    let c = add(&mut s, 100.0, 100.0, "Who can check?", "Sam Lee");
    s.execute("comment.reply", &json!({"id": c, "text": "Me", "author": "Ada"})).unwrap();
    let mut u = Ui::new(s);
    u.run("view.set", json!({"zoom": 1, "cx": 100, "cy": 100}));
    u.run("comment.open", json!({"id": c, "view": "keep"}));
    u.frames(2, Vec::new);
    u.frame(vec![Event::Text("Thanks @sa".into())]);
    u.frame(vec![]);
    assert_eq!(comments(&u)["mentions"], json!(["Sam Lee"]));
    key(&mut u, Key::Enter, Modifiers::NONE);
    assert_eq!(u.app.ui.comments.reply, "Thanks @Sam Lee ");
    assert_eq!(comments(&u)["mentions"], json!([]));
    // Enter now posts the reply, the mention included.
    u.frame(vec![Event::Text("!".into())]);
    key(&mut u, Key::Enter, Modifiers::NONE);
    let doc = u.app.session.doc().unwrap();
    assert_eq!(doc.comment(c).unwrap().replies.last().unwrap().text, "Thanks @Sam Lee !");
    assert!(doc.comment(c).unwrap().involves("Sam Lee"));
    assert!(u.app.ui.comments.reply.is_empty());
}

#[test]
fn the_presenter_shows_the_frames_comments() {
    let mut s = Session::new();
    let f = id(&s.execute("node.create", &json!({"type": "frame", "x": 0, "y": 0, "width": 400, "height": 300})).unwrap());
    let other = id(&s.execute("node.create", &json!({"type": "frame", "x": 600, "y": 0, "width": 400, "height": 300})).unwrap());
    let c = add(&mut s, 100.0, 100.0, "On the frame", "Sam");
    add(&mut s, 700.0, 100.0, "Elsewhere", "Sam");
    let mut u = Ui::new(s);
    u.run("proto.present", json!({"start": f}));
    assert!(u.app.ui.present.is_some());
    // Off by default; Shift+C shows them.
    assert_eq!(u.app.inspect()["presenter"]["comments"], false);
    key(&mut u, Key::C, Modifiers::SHIFT);
    assert_eq!(u.app.inspect()["presenter"]["comments"], true);
    // The frame is presented at 100% (Fit never scales up), centred under the bar.
    let screen = egui::Rect::from_min_size(pos2(0.0, 80.0), vec2(1440.0, 820.0));
    let frame_min = screen.center() - vec2(200.0, 150.0);
    let pin = frame_min + vec2(100.0, 100.0) + vec2(14.0, -14.0);
    click(&mut u, pin);
    assert_eq!(comments(&u)["open"], c);
    assert!(u.app.ui.present.is_some(), "the click didn't reach the prototype or close it");
    // Escape closes the thread first, then the presenter.
    key(&mut u, Key::Escape, Modifiers::NONE);
    assert_eq!(comments(&u)["open"], Value::Null);
    assert!(u.app.ui.present.is_some());
    let _ = other;
}
