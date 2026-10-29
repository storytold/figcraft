//! Comments and version history behaviour suite, written from the public documentation of design
//! tools: each rule is a table row with what it implies and a one-line derivation. Where the
//! documentation is silent the row says so and pins today's behaviour.

// The tables read best as tuples.
#![allow(clippy::type_complexity)]

use figcraft_doc::{Comment, NodeId};
use serde_json::{Value, json};

use crate::Session;
use crate::keymap::{Context, Mods, Platform, lookup};
use crate::params::id_of;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn id(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn frame(s: &mut Session, x: f64, y: f64, w: f64, h: f64, parent: Option<NodeId>) -> NodeId {
    let mut p = json!({"x": x, "y": y, "width": w, "height": h});
    if let Some(parent) = parent {
        p["parent"] = json!(parent);
    }
    id(&ex(s, "frame.create", p))
}

fn rect(s: &mut Session, x: f64, y: f64, parent: Option<NodeId>) -> NodeId {
    let mut p = json!({"x": x, "y": y, "width": 40, "height": 40});
    if let Some(parent) = parent {
        p["parent"] = json!(parent);
    }
    id(&ex(s, "shape.rectangle", p))
}

/// A comment by `author` at a page point, pinned the way the Comment tool pins it.
fn add(s: &mut Session, x: f64, y: f64, text: &str, author: &str) -> u64 {
    ex(s, "comment.add", json!({"x": x, "y": y, "text": text, "author": author, "anchor": true}))["id"].as_u64().unwrap()
}

fn comment(s: &Session, c: u64) -> Comment {
    s.doc().unwrap().comment(c).unwrap().clone()
}

fn at(s: &Session, c: u64) -> (f64, f64) {
    let (x, y) = comment(s, c).position(s.doc().unwrap());
    ((x * 1000.0).round() / 1000.0, (y * 1000.0).round() / 1000.0)
}

fn ids(s: &mut Session, cmd: &str, p: Value) -> Vec<u64> {
    ex(s, cmd, p)["comments"].as_array().unwrap().iter().map(|c| c["id"].as_u64().unwrap()).collect()
}

fn versions(s: &mut Session) -> Vec<Value> {
    ex(s, "version.list", json!({})).as_array().unwrap().clone()
}

fn temp_path(name: &str) -> (std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("figcraft-history-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.figcraft")).to_string_lossy().to_string();
    (dir, path)
}

// ---------------------------------------------------------------------------------------------
// Comments.

/// Frame `f` (0,0 400×300) with rectangle `r` (20,20) and nested frame `n` (200,20 150×150);
/// hidden frame `h` (600,0 100×100); loose rectangle `p` (800,0).
struct Scene {
    f: NodeId,
    n: NodeId,
    r: NodeId,
    h: NodeId,
}

fn scene(s: &mut Session) -> Scene {
    let f = frame(s, 0.0, 0.0, 400.0, 300.0, None);
    let r = rect(s, 20.0, 20.0, Some(f));
    let n = frame(s, 200.0, 20.0, 150.0, 150.0, Some(f));
    let h = frame(s, 600.0, 0.0, 100.0, 100.0, None);
    ex(s, "object.toggleVisible", json!({"ids": [h], "visible": false}));
    rect(s, 800.0, 0.0, None);
    ex(s, "select.none", json!({}));
    Scene { f, n, r, h }
}

#[test]
fn comments_pin_to_what_is_under_them() {
    let mut s = Session::new();
    let k = scene(&mut s);
    // (point, `anchor` param, pinned to, derivation)
    let rows: Vec<((f64, f64), Option<Value>, Option<NodeId>, &str)> = vec![
        ((900.0, 500.0), Some(json!(true)), None, "a comment on empty canvas stays at its spot on the canvas"),
        ((100.0, 200.0), Some(json!(true)), Some(k.f), "a comment dropped on a frame is pinned to it, so it moves with the frame"),
        ((250.0, 50.0), Some(json!(true)), Some(k.n), "on a frame inside a frame, the innermost frame takes the pin"),
        ((30.0, 30.0), Some(json!(true)), Some(k.f), "silent: over a layer inside a frame the frame takes the pin, not the layer (pinned)"),
        ((820.0, 20.0), Some(json!(true)), None, "silent: a loose layer on the canvas is not a frame, the pin stays on the canvas (pinned)"),
        ((650.0, 50.0), Some(json!(true)), None, "silent: a hidden frame takes no pin (pinned)"),
        ((30.0, 30.0), Some(json!(k.r)), Some(k.r), "a comment can be pinned to a given layer and follows that layer"),
        ((30.0, 30.0), Some(json!(k.h.0 + 10_000)), None, "a layer that doesn't exist pins nothing"),
        ((100.0, 200.0), None, None, "without an anchor the pin stays on the canvas"),
    ];
    for ((x, y), anchor, want, why) in rows {
        let mut p = json!({"x": x, "y": y, "text": "Here"});
        if let Some(a) = anchor {
            p["anchor"] = a;
        }
        let c = ex(&mut s, "comment.add", p)["id"].as_u64().unwrap();
        assert_eq!(comment(&s, c).anchor, want, "{why}");
        assert_eq!(at(&s, c), (x, y), "{why}: the pin sits where it was dropped");
    }
}

#[test]
fn pins_follow_their_layer() {
    let mut s = Session::new();
    let k = scene(&mut s);
    let on_f = add(&mut s, 100.0, 200.0, "On the screen", "You");
    let on_n = add(&mut s, 250.0, 50.0, "On the card", "You");
    let loose = add(&mut s, 900.0, 500.0, "On the canvas", "You");
    let page = s.page().unwrap();
    // (command, params, expected pins of on_f, on_n, loose, derivation)
    let rows: Vec<(&str, Value, [(f64, f64); 3], &str)> = vec![
        (
            "transform.move",
            json!({"ids": [k.f], "dx": 50, "dy": 10}),
            [(150.0, 210.0), (300.0, 60.0), (900.0, 500.0)],
            "moving a frame moves its comments, and those of the frames inside it",
        ),
        (
            "transform.resize",
            json!({"ids": [k.f], "width": 600, "anchor": "tl"}),
            [(150.0, 210.0), (300.0, 60.0), (900.0, 500.0)],
            "silent: resizing from the top-left keeps the pins at their offset from the frame's corner (pinned)",
        ),
        (
            "node.reparent",
            json!({"ids": [k.n], "parent": page}),
            [(150.0, 210.0), (300.0, 60.0), (900.0, 500.0)],
            "taking a frame out of its parent keeps its pins on it, in place",
        ),
        (
            "transform.move",
            json!({"ids": [k.n], "dx": 0, "dy": 100}),
            [(150.0, 210.0), (300.0, 160.0), (900.0, 500.0)],
            "the pin goes with its own frame, not with the one it used to be in",
        ),
        (
            "edit.delete",
            json!({"ids": [k.n]}),
            [(150.0, 210.0), (300.0, 160.0), (900.0, 500.0)],
            "deleting the layer leaves its comment where it was (the conversation isn't lost)",
        ),
        (
            "transform.move",
            json!({"ids": [k.f], "dx": -50, "dy": -10}),
            [(100.0, 200.0), (300.0, 160.0), (900.0, 500.0)],
            "a comment whose layer was deleted no longer moves with anything",
        ),
    ];
    for (cmd, p, want, why) in rows {
        ex(&mut s, cmd, p);
        assert_eq!([at(&s, on_f), at(&s, on_n), at(&s, loose)], want, "{why}");
    }
    assert!(s.undo().unwrap() && s.undo().unwrap());
    assert_eq!(comment(&s, on_n).anchor, Some(k.n), "undoing the delete pins the comment to the layer again");
}

#[test]
fn threads_replies_and_resolving() {
    let mut s = Session::new();
    let c = add(&mut s, 0.0, 0.0, "Can the button be bigger?", "Ada");
    // (command, params, check, derivation)
    let rows: Vec<(&str, Value, fn(&Comment) -> bool, &str)> = vec![
        (
            "comment.reply",
            json!({"id": c, "text": "Sure", "author": "Sam"}),
            |c| c.replies.len() == 1,
            "a reply joins the thread under the first message",
        ),
        (
            "comment.reply",
            json!({"id": c, "text": "Done", "author": "Ada"}),
            |c| c.replies.iter().map(|m| m.text.as_str()).eq(["Sure", "Done"]),
            "replies stay in the order they were written",
        ),
        ("comment.resolve", json!({"id": c}), |c| c.resolved, "anyone in the file can resolve a thread, not just its author (it takes no author)"),
        ("comment.resolve", json!({"id": c, "resolved": false}), |c| !c.resolved, "a resolved thread can be reopened"),
        ("comment.resolve", json!({"id": c}), |c| c.resolved, "resolving again marks it resolved"),
        (
            "comment.reply",
            json!({"id": c, "text": "One more thing", "author": "Lin"}),
            |c| !c.resolved && c.replies.len() == 3,
            "silent: replying to a resolved thread reopens it (pinned)",
        ),
        ("comment.reply", json!({"id": c, "text": "   "}), |c| c.replies.len() == 3, "an empty reply is not sent"),
    ];
    for (cmd, p, check, why) in rows {
        let _ = s.execute(cmd, &p);
        assert!(check(&comment(&s, c)), "{why}");
    }
    assert_eq!(comment(&s, c).participants(), vec!["Ada", "Sam", "Lin"], "everyone who wrote is in the thread");
    // Resolving is one undo step.
    ex(&mut s, "comment.resolve", json!({"id": c}));
    assert!(s.undo().unwrap());
    assert!(!comment(&s, c).resolved);
}

#[test]
fn only_authors_edit_and_delete() {
    // (command, params, allowed, derivation)
    let rows: Vec<(&str, Value, bool, &str)> = vec![
        ("comment.edit", json!({"text": "Fixed typo", "author": "Ada"}), true, "you can edit your own comment"),
        ("comment.edit", json!({"text": "Hijack", "author": "Sam"}), false, "you can't edit someone else's comment"),
        ("comment.edit", json!({"reply": 0, "text": "Sure!", "author": "Sam"}), true, "replies are edited by whoever wrote them"),
        ("comment.edit", json!({"reply": 0, "text": "No", "author": "Ada"}), false, "the thread's author can't edit others' replies"),
        ("comment.delete", json!({"reply": 0, "author": "Ada"}), false, "silent without accounts: only a reply's author deletes it (pinned)"),
        ("comment.delete", json!({"author": "Sam"}), false, "silent without accounts: only the thread's author deletes the thread (pinned)"),
        ("comment.delete", json!({"reply": 0, "author": "Sam"}), true, "you can delete your own reply; the thread stays"),
    ];
    let mut s = Session::new();
    let c = add(&mut s, 0.0, 0.0, "Fix tpyo", "Ada");
    ex(&mut s, "comment.reply", json!({"id": c, "text": "Sure", "author": "Sam"}));
    for (cmd, mut p, allowed, why) in rows {
        p["id"] = json!(c);
        assert_eq!(s.execute(cmd, &p).is_ok(), allowed, "{why}");
    }
    let t = comment(&s, c);
    assert_eq!(t.first.text, "Fixed typo");
    assert!(t.first.edited > 0, "an edited comment says so");
    assert!(t.replies.is_empty());
    // Deleting the first message deletes the whole thread.
    ex(&mut s, "comment.reply", json!({"id": c, "text": "Hm", "author": "Sam"}));
    ex(&mut s, "comment.delete", json!({"id": c, "author": "Ada"}));
    assert!(s.doc().unwrap().comment(c).is_none(), "deleting the first comment deletes the thread with its replies");
    assert!(s.undo().unwrap(), "deleting is one undo step");
    assert_eq!(comment(&s, c).replies.len(), 1);
}

#[test]
fn mentions_and_reactions() {
    let mut s = Session::new();
    let a = add(&mut s, 0.0, 0.0, "Looks good", "Sam");
    let b = add(&mut s, 50.0, 0.0, "@Ada can you check the spacing?", "Sam");
    let c = add(&mut s, 90.0, 0.0, "Note to self", "Lin");
    // (who, threads that are theirs, derivation)
    let rows: Vec<(&str, Vec<u64>, &str)> = vec![
        ("Ada", vec![b], "a thread that @mentions you is one of yours"),
        ("Sam", vec![b, a], "threads you started are yours"),
        ("Lin", vec![c], "and nobody else's threads are"),
        ("Nobody", vec![], "a mention is text: naming someone needs no account"),
    ];
    for (who, want, why) in rows {
        assert_eq!(ids(&mut s, "comment.list", json!({"filter": "mine", "author": who})), want, "{why}");
    }
    // Reactions: (author, emoji, users after, derivation)
    let rows: Vec<(&str, &str, Vec<&str>, &str)> = vec![
        ("Ada", "👍", vec!["Ada"], "anyone can react to a comment with an emoji"),
        ("Lin", "👍", vec!["Ada", "Lin"], "the same emoji from someone else adds them to it"),
        ("Ada", "👍", vec!["Lin"], "reacting again with the same emoji takes yours back"),
        ("Lin", "👍", vec![], "the last one gone removes the reaction"),
    ];
    for (who, emoji, users, why) in rows {
        ex(&mut s, "comment.react", json!({"id": a, "emoji": emoji, "author": who}));
        let r = comment(&s, a).first.reactions;
        assert_eq!(r.first().map(|r| r.users.clone()).unwrap_or_default(), users, "{why}");
    }
}

#[test]
fn comment_shortcuts() {
    let m = |shift, alt, cmd, ctrl| Mods { shift, alt, cmd, ctrl };
    // (platform, key, modifiers, command, derivation)
    let rows: Vec<(Platform, &str, Mods, &str, &str)> = vec![
        (Platform::Mac, "C", m(false, false, false, false), "tool.comment", "C picks the Comment tool"),
        (Platform::Other, "C", m(false, false, false, false), "tool.comment", "C picks the Comment tool on Windows and Linux too"),
        (Platform::Mac, "C", m(true, false, false, false), "view.comments", "Shift+C shows and hides comments on the canvas"),
        (Platform::Mac, "S", m(false, true, true, false), "file.saveVersion", "Alt+Cmd+S saves to version history"),
        (Platform::Other, "S", m(false, true, false, true), "file.saveVersion", "Ctrl+Alt+S saves to version history"),
    ];
    for (p, key, mods, want, why) in rows {
        assert_eq!(lookup(p, Context::Canvas, key, mods).map(|b| b.id), Some(want), "{why}");
    }
    // A click with the Comment tool starts a new comment there; Shift+C's flag toggles.
    let mut s = Session::new();
    ex(&mut s, "tool.comment", json!({}));
    let down = crate::tools::PointerEvent::at(crate::tools::PointerKind::Down, 120.0, 80.0, 1.0);
    s.pointer(&down).unwrap();
    let req = s.ui_requests.iter().rev().find(|r| r.kind == "comment.new").expect("a new comment is asked for");
    assert_eq!((req.data["x"].as_f64(), req.data["y"].as_f64()), (Some(120.0), Some(80.0)));
    assert!(s.ds().unwrap().view.comments, "silent: comments show on the canvas by default (pinned)");
    ex(&mut s, "view.comments", json!({}));
    assert!(!s.ds().unwrap().view.comments, "the toggle hides them");
    ex(&mut s, "view.comments", json!({}));
    assert!(s.ds().unwrap().view.comments, "and shows them again");
}

/// Threads on two pages: `a` (oldest) and `b` on page 1, `c` on page 2 (newest); `b` resolved.
fn threads(s: &mut Session) -> (u64, u64, u64, NodeId, NodeId) {
    let p1 = s.page().unwrap();
    let a = add(s, 0.0, 0.0, "Spacing", "Sam");
    let b = add(s, 900.0, 900.0, "Colour", "Ada");
    let p2 = id(&ex(s, "page.add", json!({"name": "Two"})));
    ex(s, "page.select", json!({"id": p2}));
    let c = add(s, 10.0, 10.0, "Copy on page two", "Sam");
    ex(s, "page.select", json!({"id": p1}));
    // Distinct times: the newest activity is listed first.
    for (i, t) in [(a, 100), (b, 200), (c, 300)] {
        s.doc_mut().unwrap().comment_mut(i).unwrap().first.created = t;
    }
    ex(s, "comment.resolve", json!({"id": b}));
    (a, b, c, p1, p2)
}

#[test]
fn filtering_and_searching_comments() {
    let mut s = Session::new();
    let (a, b, c, _, p2) = threads(&mut s);
    // (params, listed, derivation)
    let rows: Vec<(Value, Vec<u64>, &str)> = vec![
        (json!({}), vec![b, a], "the list shows the current page, newest activity first"),
        (json!({"page": "all"}), vec![c, b, a], "it can show every page"),
        (json!({"page": p2}), vec![c], "or another page"),
        (json!({"filter": "open"}), vec![a], "resolved threads can be hidden"),
        (json!({"filter": "resolved"}), vec![b], "or shown on their own"),
        (json!({"page": "all", "sort": "page"}), vec![b, a, c], "sorting by page groups threads in page order"),
        (json!({"page": "all", "query": "colour"}), vec![b], "search finds a thread by its text, in any case"),
        (json!({"page": "all", "query": "sam"}), vec![c, a], "and by who wrote it"),
    ];
    for (p, want, why) in rows {
        assert_eq!(ids(&mut s, "comment.list", p), want, "{why}");
    }
}

#[test]
fn stepping_between_comments() {
    let mut s = Session::new();
    let (a, b, c, p1, p2) = threads(&mut s);
    let opened = |s: &mut Session, p: Value| ex(s, "comment.next", p)["id"].as_u64().unwrap();
    // (params, opened, page shown after, derivation)
    let rows: Vec<(Value, u64, NodeId, &str)> = vec![
        (json!({}), b, p1, "stepping starts at the newest thread on the page"),
        (json!({"id": b}), a, p1, "next goes on down the list"),
        (json!({"id": a}), a, p1, "silent: past the last thread it stays on the last (pinned)"),
        (json!({"id": a, "dir": -1}), b, p1, "previous goes back up the list"),
        (json!({"id": b, "filter": "open"}), a, p1, "the list's filter decides what is stepped through"),
        (json!({"page": "all"}), c, p2, "across all pages the view goes to the thread's page"),
    ];
    for (p, want, page, why) in rows {
        assert_eq!(opened(&mut s, p), want, "{why}");
        assert_eq!(s.page().unwrap(), page, "{why}");
    }
    ex(&mut s, "comment.markRead", json!({"author": "Sam"}));
    assert!(s.execute("comment.next", &json!({"filter": "unread", "author": "Sam"})).is_err(), "nothing to step through says so");
}

#[test]
fn comments_survive_saving_and_stay_put_on_copy_paste() {
    let mut s = Session::new();
    let k = scene(&mut s);
    let c = add(&mut s, 100.0, 200.0, "Pinned", "Ada");
    ex(&mut s, "comment.reply", json!({"id": c, "text": "Agreed @Ada", "author": "Sam"}));
    ex(&mut s, "comment.react", json!({"id": c, "emoji": "🎉", "author": "Lin"}));
    ex(&mut s, "comment.resolve", json!({"id": c}));
    let loose = add(&mut s, 900.0, 900.0, "Loose", "Ada");
    let before = s.doc().unwrap().comments.clone();

    // (command, params, derivation): none of these copies or moves a comment.
    let rows: Vec<(&str, Value, &str)> = vec![
        ("edit.copy", json!({"ids": [k.f]}), "copying a frame copies its layers"),
        ("edit.paste", json!({"x": 1000, "y": 0}), "silent: a pasted frame comes without the original's comments (pinned)"),
        ("edit.duplicate", json!({"ids": [k.f]}), "silent: a duplicated frame comes without them too (pinned)"),
        ("page.duplicate", json!({}), "silent: a duplicated page comes without its comments (pinned)"),
    ];
    for (cmd, p, why) in rows {
        ex(&mut s, cmd, p);
        assert_eq!(s.doc().unwrap().comments, before, "{why}");
    }

    let (dir, path) = temp_path("comments");
    ex(&mut s, "file.save", json!({"path": path}));
    let mut t = Session::empty();
    ex(&mut t, "file.open", json!({"path": path}));
    assert_eq!(t.doc().unwrap().comments, before, "threads, replies, reactions, resolved state and pins are saved in the file");
    assert_eq!(comment(&t, c).anchor, Some(k.f));
    // A pin reopened still follows its frame.
    ex(&mut t, "transform.move", json!({"ids": [k.f], "dx": 10, "dy": 0}));
    assert_eq!((at(&t, c), at(&t, loose)), ((110.0, 200.0), (900.0, 900.0)));
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------------------------
// Version history.

#[test]
fn autosaves_and_named_versions() {
    let mut s = Session::new();
    // (command, params, versions after as (name, auto), derivation)
    let rows: Vec<(&str, Value, Vec<(&str, bool)>, &str)> = vec![
        ("version.autosave", json!({}), vec![], "no checkpoint while nothing has changed"),
        ("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}), vec![], "editing alone saves no version"),
        ("version.autosave", json!({}), vec![("Autosave", true)], "after changes the file is checkpointed automatically"),
        ("version.autosave", json!({}), vec![("Autosave", true)], "no new checkpoint without new changes"),
        (
            "file.saveVersion",
            json!({"name": "Hand-off", "description": "for review"}),
            vec![("Autosave", true), ("Hand-off", false)],
            "a version can be saved by hand with a title and a description",
        ),
        (
            "file.saveVersion",
            json!({}),
            vec![("Autosave", true), ("Hand-off", false), ("Version 3", false)],
            "silent: a version saved without a title is numbered (pinned)",
        ),
        (
            "version.rename",
            json!({"index": 0, "name": "Before the redesign"}),
            vec![("Before the redesign", false), ("Hand-off", false), ("Version 3", false)],
            "giving an autosave a title makes it a named version",
        ),
    ];
    for (cmd, p, want, why) in rows {
        ex(&mut s, cmd, p);
        let got: Vec<(String, bool)> = versions(&mut s).iter().map(|v| (v["name"].as_str().unwrap().to_string(), v["auto"] == true)).collect();
        let want: Vec<(String, bool)> = want.into_iter().map(|(n, a)| (n.to_string(), a)).collect();
        assert_eq!(got, want, "{why}");
    }
    assert_eq!(versions(&mut s)[1]["description"], "for review");
    // A named autosave is kept when autosaves make room for new ones.
    for i in 0..(crate::cmd::file::MAX_AUTOSAVES + 2) {
        ex(&mut s, "shape.rectangle", json!({"x": i * 20, "y": 50, "width": 10, "height": 10}));
        ex(&mut s, "version.autosave", json!({}));
    }
    assert_eq!(versions(&mut s)[0]["name"], "Before the redesign", "versions you named are never dropped to make room");
    // The history lists versions in the order they were saved.
    let times: Vec<u64> = versions(&mut s).iter().map(|v| v["time"].as_u64().unwrap()).collect();
    assert!(times.windows(2).all(|w| w[0] <= w[1]), "oldest first in the list (the panel shows it newest first)");
}

#[test]
fn restoring_a_version() {
    let mut s = Session::new();
    ex(&mut s, "file.rename", json!({"name": "Shop"}));
    let a = rect(&mut s, 0.0, 0.0, None);
    let v1 = ex(&mut s, "file.saveVersion", json!({"name": "One box"}));
    let b = rect(&mut s, 100.0, 0.0, None);
    let c =
        ex(&mut s, "comment.add", json!({"x": 120, "y": 20, "text": "Is this box needed?", "author": "Sam", "anchor": b}))["id"].as_u64().unwrap();
    assert_eq!(comment(&s, c).anchor, Some(b));
    ex(&mut s, "comment.reply", json!({"id": c, "text": "Yes", "author": "Ada"}));
    ex(&mut s, "file.rename", json!({"name": "Shop v2"}));
    let n = versions(&mut s).len();

    ex(&mut s, "version.restore", json!({"id": v1["id"]}));
    let doc = s.doc().unwrap();
    // (holds, derivation)
    let rows: Vec<(bool, &str)> = vec![
        (doc.contains(a) && !doc.contains(b), "restoring puts the design back as it was in that version"),
        (doc.name == "Shop v2", "silent: the file keeps its current name (pinned)"),
        (doc.comment(c).is_some_and(|c| c.replies.len() == 1), "comments are kept apart from the design's history: restoring leaves them"),
        (doc.comment(c).is_some_and(|c| c.anchor.is_none()), "a comment on a layer the version doesn't have stays on the canvas"),
    ];
    for (holds, why) in rows {
        assert!(holds, "{why}");
    }
    assert_eq!(at(&s, c), (120.0, 20.0), "where it was");
    let l = versions(&mut s);
    assert_eq!(l.len(), n + 1, "restoring never loses work: the state before it is kept in the history first");
    let kept = l.last().unwrap();
    assert_eq!(kept["auto"], true);
    ex(&mut s, "version.preview", json!({"id": kept["id"]}));
    assert!(s.doc().unwrap().contains(b), "the kept checkpoint holds what was there before restoring");
    ex(&mut s, "version.exitPreview", json!({}));
    // Restoring again straight away doesn't checkpoint the restored state a second time.
    ex(&mut s, "version.restore", json!({"id": v1["id"]}));
    assert_eq!(versions(&mut s).len(), n + 1);
    // Restoring is one undo step (twice here).
    assert!(s.undo().unwrap() && s.undo().unwrap());
    assert!(s.doc().unwrap().contains(b));
}

#[test]
fn duplicating_and_viewing_versions() {
    let mut s = Session::new();
    ex(&mut s, "file.rename", json!({"name": "App"}));
    let a = rect(&mut s, 0.0, 0.0, None);
    let v = ex(&mut s, "file.saveVersion", json!({"name": "Start"}));
    let b = rect(&mut s, 100.0, 0.0, None);
    // Viewing a version shows it read-only.
    ex(&mut s, "version.preview", json!({"id": v["id"]}));
    assert!(!s.doc().unwrap().contains(b), "viewing a version shows the file as it was then");
    assert!(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).is_err(), "a version being viewed can't be edited");
    ex(&mut s, "version.exitPreview", json!({}));
    assert!(s.doc().unwrap().contains(b), "going back shows the current file, unchanged");
    // Duplicating a version opens it as a new file.
    let d = ex(&mut s, "version.duplicate", json!({"id": v["id"]}));
    assert_eq!(s.active, d["index"].as_u64().unwrap() as usize);
    let doc = s.doc().unwrap();
    assert!(doc.contains(a) && !doc.contains(b), "a version can be duplicated into a new file");
    assert!(s.ds().unwrap().versions.is_empty(), "the new file starts its own history");
    assert!(s.ds().unwrap().path.is_none());
    // So does duplicating the whole file.
    ex(&mut s, "doc.switch", json!({"index": 0}));
    ex(&mut s, "file.duplicate", json!({}));
    assert!(s.ds().unwrap().versions.is_empty(), "silent: a duplicated file starts its own history (pinned)");
}

#[test]
fn versions_survive_saving() {
    let mut s = Session::new();
    rect(&mut s, 0.0, 0.0, None);
    ex(&mut s, "version.autosave", json!({}));
    rect(&mut s, 50.0, 0.0, None);
    ex(&mut s, "file.saveVersion", json!({"name": "Named", "description": "two boxes"}));
    rect(&mut s, 100.0, 0.0, None);
    ex(&mut s, "version.autosave", json!({}));
    ex(&mut s, "version.rename", json!({"index": 2, "name": "Three"}));
    let before = versions(&mut s);
    let (dir, path) = temp_path("versions");
    ex(&mut s, "file.save", json!({"path": path}));
    let mut t = Session::empty();
    ex(&mut t, "file.open", json!({"path": path}));
    assert_eq!(versions(&mut t), before, "names, descriptions, times, order and the autosave flag are saved in the file");
    // Save as keeps the history too.
    let (dir2, path2) = temp_path("versions-as");
    ex(&mut t, "file.saveAs", json!({"path": path2}));
    let mut u = Session::empty();
    ex(&mut u, "file.open", json!({"path": path2}));
    assert_eq!(versions(&mut u), before);
    ex(&mut u, "version.restore", json!({"index": 0}));
    assert_eq!(u.doc().unwrap().node_count(), t.doc().unwrap().node_count() - 2, "a version read from the file restores");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&dir2);
}

#[test]
fn branches_and_the_version_history() {
    let mut s = Session::with_document(crate::collab_tests::base());
    ex(&mut s, "file.saveVersion", json!({"name": "Before branching"}));
    let b = ex(&mut s, "branch.create", json!({"name": "Explore"}));
    let branch = b["id"].as_str().unwrap().to_string();
    // On the branch tab.
    // A branch keeps its own version history: a version saved on its tab is listed there only.
    ex(&mut s, "file.saveVersion", json!({"name": "On the branch"}));
    assert_eq!(
        versions(&mut s).iter().map(|v| v["name"].clone()).collect::<Vec<_>>(),
        [json!("Branch created"), json!("On the branch")],
        "a branch's own history"
    );
    rect(&mut s, 900.0, 900.0, None);
    ex(&mut s, "doc.switch", json!({"index": 0}));
    let made_on_main = rect(&mut s, -500.0, 0.0, None);
    let n = versions(&mut s).len();
    ex(&mut s, "branch.merge", json!({"id": branch}));
    let l = versions(&mut s);
    // (holds, derivation)
    let rows: Vec<(bool, &str)> = vec![
        (l.len() == n + 2, "merging a branch adds to the main file's history: what was there before, then the merge"),
        (l.get(n).is_some_and(|v| v["auto"] == true), "the main file as it was before the merge is kept"),
        (l.get(n + 1).is_some_and(|v| v["name"] == "Merged “Explore”" && v["auto"] != true), "the merge is a named version, after the branch"),
    ];
    for (holds, why) in rows {
        assert!(holds, "{why}: {l:?}");
    }
    let before_merge = l[n]["id"].clone();
    ex(&mut s, "version.preview", json!({"id": before_merge}));
    assert!(s.doc().unwrap().contains(made_on_main) && s.doc().unwrap().node_count() > 0);
    ex(&mut s, "version.exitPreview", json!({}));
    // The file can go back to before the merge by restoring that version.
    let count = s.doc().unwrap().node_count();
    ex(&mut s, "version.restore", json!({"id": before_merge}));
    assert_eq!(s.doc().unwrap().node_count(), count - 1, "restoring the version before the merge takes the branch's work out");
}

// ---------------------------------------------------------------------------------------------
// Live sessions.

#[test]
fn comments_and_restores_in_a_live_session() {
    use crate::collab_live_tests::{Room, frame_and_rect};
    let mut room = Room::new(crate::collab_tests::base(), 2, 7);
    let (f, _) = frame_and_rect(room.server.document());
    let c = ex(room.s(0), "comment.add", json!({"x": 50, "y": 50, "text": "Thoughts?", "author": "Ada", "anchor": true}))["id"].as_u64().unwrap();
    room.settle();
    // (person, command, params, derivation)
    let rows: Vec<(usize, &str, Value, &str)> = vec![
        (1, "comment.reply", json!({"id": c, "text": "Bigger", "author": "Sam"}), "a reply from someone else reaches everyone"),
        (1, "comment.react", json!({"id": c, "emoji": "👍", "author": "Sam"}), "so does a reaction"),
        (0, "comment.resolve", json!({"id": c}), "and resolving"),
    ];
    for (who, cmd, p, why) in rows {
        ex(room.s(who), cmd, p);
        room.settle();
        room.check();
        let mine = room.s(0).doc().unwrap().comment(c).cloned();
        assert_eq!(mine.as_ref(), room.s(1).doc().unwrap().comment(c), "{why}");
    }
    assert_eq!(room.s(1).doc().unwrap().comment(c).unwrap().anchor, Some(f), "the pin is on the same frame for everyone");
    // A version is the saving person's own (versions aren't shared in a session, by design for now).
    let v = ex(room.s(0), "file.saveVersion", json!({"name": "Shared?"}));
    assert!(versions(room.s(1)).is_empty(), "silent here: versions stay with whoever saved them (shared history needs accounts)");
    ex(room.s(1), "transform.move", json!({"ids": [f], "dx": 40, "dy": 0}));
    let d = ex(room.s(1), "comment.add", json!({"x": 900, "y": 900, "text": "Later", "author": "Sam"}))["id"].as_u64().unwrap();
    room.settle();
    // Restoring in a session changes the design for everyone and keeps everyone's comments.
    ex(room.s(0), "version.restore", json!({"id": v["id"]}));
    room.settle();
    room.check();
    let doc = room.s(1).doc().unwrap();
    let x = doc.world_bounds(f).unwrap().x0;
    assert_eq!(x, 0.0, "the restore reached everyone");
    assert!(doc.comment(c).is_some() && doc.comment(d).is_some(), "comments made since the version are kept");
}
