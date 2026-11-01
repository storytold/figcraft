//! Comment polish: pins that follow their layers (and stay put when the layers go), messages you
//! edit and delete, reactions, @mentions, read state, filters and sorting, links that reveal a
//! thread, and files from before all this.

use figcraft_doc::{Comment, NodeId, mention_ranges};
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;

fn frame(s: &mut Session, x: f64, y: f64, name: &str) -> NodeId {
    let f = s.execute("node.create", &json!({"type": "frame", "x": x, "y": y, "width": 200, "height": 200, "name": name})).unwrap();
    id_of(&f["id"]).unwrap()
}

fn add(s: &mut Session, x: f64, y: f64, text: &str, author: &str) -> u64 {
    s.execute("comment.add", &json!({"x": x, "y": y, "text": text, "author": author, "anchor": true})).unwrap()["id"].as_u64().unwrap()
}

fn list(s: &mut Session, p: Value) -> Vec<u64> {
    s.execute("comment.list", &p).unwrap()["comments"].as_array().unwrap().iter().map(|c| c["id"].as_u64().unwrap()).collect()
}

fn comment(s: &Session, id: u64) -> Comment {
    s.doc().unwrap().comment(id).unwrap().clone()
}

#[test]
fn deleting_the_layer_keeps_the_pin_where_it_was() {
    let mut s = Session::new();
    let outer = frame(&mut s, 100.0, 100.0, "Screen");
    let inner =
        id_of(&s.execute("node.create", &json!({"type": "frame", "parent": outer, "x": 20, "y": 20, "width": 50, "height": 50})).unwrap()["id"])
            .unwrap();
    let c = add(&mut s, 130.0, 140.0, "Nested", "You");
    assert_eq!(comment(&s, c).anchor, Some(inner));
    // Deleting the inner frame pins the comment to the outer one, at the same spot.
    s.execute("edit.delete", &json!({"ids": [inner]})).unwrap();
    assert_eq!(comment(&s, c).anchor, Some(outer));
    assert_eq!(comment(&s, c).position(s.doc().unwrap()), (130.0, 140.0));
    // …which it then follows.
    s.execute("node.set", &json!({"id": outer, "props": {"x": 200}})).unwrap();
    assert_eq!(comment(&s, c).position(s.doc().unwrap()), (230.0, 140.0));
    // Deleting the outer frame leaves it on the page, where it last was.
    s.execute("edit.delete", &json!({"ids": [outer]})).unwrap();
    assert_eq!(comment(&s, c).anchor, None);
    assert_eq!(comment(&s, c).position(s.doc().unwrap()), (230.0, 140.0));
    // Undo puts the frame and the pin back together.
    assert!(s.undo().unwrap());
    assert_eq!(comment(&s, c).anchor, Some(outer));
}

#[test]
fn pins_follow_their_layer_to_another_page_and_can_be_moved() {
    let mut s = Session::new();
    let p1 = s.page().unwrap();
    let f = frame(&mut s, 0.0, 0.0, "Card");
    let g = frame(&mut s, 400.0, 0.0, "Other");
    let c = add(&mut s, 50.0, 60.0, "Here", "You");
    let p2 = id_of(&s.execute("page.add", &json!({"name": "Two"})).unwrap()["id"]).unwrap();
    s.execute("page.select", &json!({"id": p1})).unwrap();
    s.execute("object.moveToPage", &json!({"ids": [f], "page": p2})).unwrap();
    assert_eq!(comment(&s, c).page, p2);
    assert_eq!(comment(&s, c).anchor, Some(f));
    // Moving a pin onto another frame pins it there; `anchor: false` leaves it on the page.
    s.execute("object.moveToPage", &json!({"ids": [f], "page": p1})).unwrap();
    s.execute("comment.move", &json!({"id": c, "x": 450, "y": 20})).unwrap();
    assert_eq!(comment(&s, c).anchor, Some(g));
    assert_eq!(comment(&s, c).position(s.doc().unwrap()), (450.0, 20.0));
    s.execute("comment.move", &json!({"id": c, "x": 1000, "y": 20, "anchor": false})).unwrap();
    assert_eq!(comment(&s, c).anchor, None);
}

#[test]
fn only_authors_edit_and_delete_their_messages() {
    let mut s = Session::new();
    let c = add(&mut s, 0.0, 0.0, "First", "You");
    s.execute("comment.reply", &json!({"id": c, "text": "Second", "author": "Sam"})).unwrap();
    // Sam's reply isn't yours to edit or delete, nor is your thread Sam's to delete.
    assert!(s.execute("comment.edit", &json!({"id": c, "reply": 0, "text": "Nope"})).is_err());
    assert!(s.execute("comment.delete", &json!({"id": c, "reply": 0})).is_err());
    assert!(s.execute("comment.delete", &json!({"id": c, "author": "Sam"})).is_err());
    assert!(s.execute("comment.edit", &json!({"id": c, "reply": -1, "text": "x"})).is_err());
    assert!(s.execute("comment.edit", &json!({"id": c, "reply": 5, "text": "x"})).is_err());
    s.execute("comment.edit", &json!({"id": c, "text": "  First, edited  "})).unwrap();
    let m = comment(&s, c).first;
    assert_eq!(m.text, "First, edited");
    assert!(m.edited > 0);
    s.execute("comment.edit", &json!({"id": c, "reply": 0, "text": "Second!", "author": "Sam"})).unwrap();
    assert_eq!(comment(&s, c).replies[0].text, "Second!");
    s.execute("comment.delete", &json!({"id": c, "reply": 0, "author": "Sam"})).unwrap();
    assert!(comment(&s, c).replies.is_empty());
    // Your name from Preferences signs and owns your comments.
    s.execute("pref.userName", &json!({"name": "  Ada  "})).unwrap();
    let d = s.execute("comment.add", &json!({"x": 0, "y": 0, "text": "Mine"})).unwrap()["id"].as_u64().unwrap();
    assert_eq!(comment(&s, d).first.author, "Ada");
    assert!(s.execute("comment.delete", &json!({"id": c})).is_err());
    s.execute("comment.delete", &json!({"id": d})).unwrap();
    s.execute("pref.userName", &json!({"name": " "})).unwrap();
    assert_eq!(s.prefs.user_name, "You");
}

#[test]
fn reactions_toggle_per_person() {
    let mut s = Session::new();
    let c = add(&mut s, 0.0, 0.0, "Ship it?", "You");
    s.execute("comment.reply", &json!({"id": c, "text": "Yes", "author": "Sam"})).unwrap();
    assert_eq!(s.execute("comment.react", &json!({"id": c, "emoji": "👍"})).unwrap()["on"], true);
    s.execute("comment.react", &json!({"id": c, "emoji": "👍", "author": "Sam"})).unwrap();
    s.execute("comment.react", &json!({"id": c, "emoji": "🎉", "reply": 0})).unwrap();
    let m = comment(&s, c);
    assert_eq!(m.first.reactions[0].users, vec!["You", "Sam"]);
    assert_eq!(m.replies[0].reactions[0].emoji, "🎉");
    // Toggling off removes the person; the last one removes the reaction. `on` forces a state.
    assert_eq!(s.execute("comment.react", &json!({"id": c, "emoji": "👍"})).unwrap()["on"], false);
    s.execute("comment.react", &json!({"id": c, "emoji": "👍", "author": "Sam", "on": false})).unwrap();
    s.execute("comment.react", &json!({"id": c, "emoji": "👍", "author": "Sam", "on": false})).unwrap();
    assert!(comment(&s, c).first.reactions.is_empty());
    for bad in [json!(""), json!("two words"), json!("x".repeat(40)), json!(null)] {
        assert!(s.execute("comment.react", &json!({"id": c, "emoji": bad})).is_err());
    }
    // Reactions are one undo step each.
    assert!(s.undo().unwrap());
    assert_eq!(comment(&s, c).first.reactions[0].users, vec!["Sam"]);
}

#[test]
fn mentions_and_people() {
    let people = ["Sam", "Sam Lee", "Ada"];
    let t = "Hi @sam lee and @Ada, not me@Ada or @Adam";
    let r = mention_ranges(t, &people);
    assert_eq!(r.iter().map(|r| &t[r.clone()]).collect::<Vec<_>>(), vec!["@sam lee", "@Ada"]);
    assert!(mention_ranges("@", &people).is_empty());
    assert!(mention_ranges("é@Sam", &people).is_empty());
    assert_eq!(mention_ranges("(@Sam)", &people).len(), 1);
    let mut s = Session::new();
    let c = add(&mut s, 0.0, 0.0, "Ask @Ada", "Sam");
    s.execute("comment.react", &json!({"id": c, "emoji": "👀", "author": "Lin"})).unwrap();
    let p = s.execute("comment.people", &json!({})).unwrap();
    // Ada was only @mentioned, and is listed too (#1415).
    assert_eq!(p["people"], json!(["Ada", "Lin", "Sam"]));
    assert_eq!(p["you"], "You");
    // A thread you're mentioned in is yours.
    assert!(comment(&s, c).involves("Ada"));
    assert!(!comment(&s, c).involves("You"));
}

#[test]
fn read_state_follows_new_messages() {
    let mut s = Session::new();
    let c = add(&mut s, 0.0, 0.0, "From Sam", "Sam");
    assert!(comment(&s, c).is_unread("You"));
    assert!(!comment(&s, c).is_unread("Sam"));
    assert_eq!(list(&mut s, json!({"filter": "unread"})), vec![c]);
    let before = s.ds().unwrap().history.undo.len();
    assert_eq!(s.execute("comment.markRead", &json!({"id": c})).unwrap()["changed"], 1);
    assert!(list(&mut s, json!({"filter": "unread"})).is_empty());
    // Read state isn't an undo step, and marking what's already read changes nothing.
    assert_eq!(s.ds().unwrap().history.undo.len(), before);
    let doc = s.ds().unwrap().doc.clone();
    s.execute("comment.markRead", &json!({})).unwrap();
    assert!(std::sync::Arc::ptr_eq(&doc, &s.ds().unwrap().doc));
    // A reply makes it unread for everyone else; mark unread puts it back.
    s.execute("comment.reply", &json!({"id": c, "text": "More", "author": "Sam"})).unwrap();
    assert!(comment(&s, c).is_unread("You"));
    s.execute("comment.markRead", &json!({})).unwrap();
    s.execute("comment.markRead", &json!({"link": format!("figcraft://comment/{c}"), "read": false})).unwrap();
    assert!(comment(&s, c).is_unread("You"));
    assert!(!comment(&s, c).is_unread("Sam"));
    assert!(s.execute("comment.markRead", &json!({"link": "nope"})).is_err());
}

#[test]
fn files_from_before_load_and_count_as_read() {
    let old = json!({"id": 3, "page": 1, "x": 5.0, "y": 6.0, "anchor": null, "author": "Sam", "text": "Old", "created": 10,
        "resolved": false, "replies": [{"author": "Lin", "text": "Reply", "created": 11}]});
    let mut c: Comment = serde_json::from_value(old).unwrap();
    assert_eq!(c.first.author, "Sam");
    assert_eq!(c.replies[0].text, "Reply");
    assert!(!c.is_unread("You"));
    // The first message's fields stay on the comment itself; defaults are left out.
    let v = serde_json::to_value(&c).unwrap();
    assert_eq!(v["text"], "Old");
    assert!(v.get("first").is_none() && v.get("readBy").is_none() && v.get("read_by").is_none() && v.get("edited").is_none());
    // Marking an old thread unread keeps everyone else's read.
    assert!(c.set_read("Sam", false));
    assert!(c.is_unread("Sam"));
    assert!(!c.is_unread("Lin"));
    let back: Comment = serde_json::from_value(serde_json::to_value(&c).unwrap()).unwrap();
    assert_eq!(back, c);
}

#[test]
fn filters_sort_search_and_frames() {
    let mut s = Session::new();
    let p1 = s.page().unwrap();
    let home = frame(&mut s, 0.0, 0.0, "Home");
    let a = add(&mut s, 10.0, 10.0, "Logo too big", "Sam");
    let b = add(&mut s, 500.0, 500.0, "Loose note @You", "Lin");
    let p2 = id_of(&s.execute("page.add", &json!({"name": "Two"})).unwrap()["id"]).unwrap();
    let c = add(&mut s, 0.0, 0.0, "Second page", "You");
    s.execute("page.select", &json!({"id": p1})).unwrap();
    s.execute("comment.resolve", &json!({"id": a})).unwrap();
    // Make `a` the most recently active thread.
    if let Some(m) = s.doc_mut().unwrap().comment_mut(a) {
        m.first.created = 9_999_999_999;
    }
    assert_eq!(list(&mut s, json!({})), vec![a, b]);
    // By date: latest activity first. By page: the first page's threads (latest first), then the next.
    let mut by_date = vec![a];
    by_date.extend(newest_first(&s, &[b, c]));
    assert_eq!(list(&mut s, json!({"page": "all"})), by_date);
    assert_eq!(list(&mut s, json!({"page": "all", "sort": "page"})), vec![a, b, c]);
    assert_eq!(list(&mut s, json!({"page": "all", "filter": "open"})).len(), 2);
    assert_eq!(list(&mut s, json!({"page": "all", "filter": "resolved"})), vec![a]);
    // Yours: threads you wrote in or are mentioned in.
    let mine = list(&mut s, json!({"page": "all", "filter": "mine"}));
    assert!(mine.contains(&b) && mine.contains(&c) && !mine.contains(&a));
    assert_eq!(list(&mut s, json!({"page": "all", "query": "LOGO"})), vec![a]);
    // The frame's name matches too, and each thread names its frame and page.
    assert_eq!(list(&mut s, json!({"page": "all", "query": "home"})), vec![a]);
    assert_eq!(list(&mut s, json!({"frame": home})), vec![a]);
    let all = s.execute("comment.list", &json!({"page": "all"})).unwrap();
    let ja = all["comments"].as_array().unwrap().iter().find(|j| j["id"] == a).unwrap().clone();
    assert_eq!(ja["frameName"], "Home");
    assert_eq!(ja["pageName"], "Page 1");
    assert_eq!(ja["link"], format!("figcraft://comment/{a}"));
    assert_eq!(list(&mut s, json!({"page": p2})), vec![c]);
    assert!(s.execute("comment.list", &json!({"filter": "weird"})).is_err());
    assert!(s.execute("comment.list", &json!({"sort": "weird"})).is_err());
}

/// `ids` newest first, as the date sort orders them.
fn newest_first(s: &Session, ids: &[u64]) -> Vec<u64> {
    let doc = s.doc().unwrap();
    let mut v = ids.to_vec();
    v.sort_by_key(|i| {
        let c = doc.comment(*i).unwrap();
        (std::cmp::Reverse(c.last_activity()), std::cmp::Reverse(c.id))
    });
    v
}

#[test]
fn links_open_and_zoom_to_a_thread() {
    let mut s = Session::new();
    let p1 = s.page().unwrap();
    frame(&mut s, 1000.0, 1000.0, "Far");
    let c = add(&mut s, 1050.0, 1060.0, "Look here", "Sam");
    let link = s.execute("comment.link", &json!({"id": c})).unwrap()["link"].as_str().unwrap().to_string();
    s.execute("page.add", &json!({"name": "Elsewhere"})).unwrap();
    assert_ne!(s.page().unwrap(), p1);
    s.ui_requests.clear();
    let r = s.execute("comment.open", &json!({"link": link})).unwrap();
    assert_eq!(r["id"], c);
    assert_eq!(r["unread"], false);
    assert_eq!(s.page().unwrap(), p1);
    let v = &s.ds().unwrap().view;
    assert!(v.zoom <= 1.0 && (v.cx - 1100.0).abs() < 1.0 && (v.cy - 1100.0).abs() < 1.0);
    assert!(s.ui_requests.iter().any(|u| u.kind == "comment.open" && u.data["id"] == c));
    // Centre keeps the zoom.
    s.execute("view.set", &json!({"zoom": 3, "cx": 0, "cy": 0})).unwrap();
    s.execute("comment.open", &json!({"id": c, "view": "center"})).unwrap();
    let v = &s.ds().unwrap().view;
    assert_eq!((v.zoom, v.cx, v.cy), (3.0, 1050.0, 1060.0));
    assert!(s.execute("comment.open", &json!({"id": c, "view": "nowhere"})).is_err());
    assert!(s.execute("comment.open", &json!({"link": "figcraft://comment/99"})).is_err());
    // Zooming to a cluster of pins at least doubles the zoom.
    let d = add(&mut s, 1052.0, 1061.0, "Near", "Sam");
    s.execute("view.set", &json!({"zoom": 0.1})).unwrap();
    let z = s.execute("comment.zoom", &json!({"ids": [c, d]})).unwrap()["zoom"].as_f64().unwrap();
    assert!(z >= 0.2);
    assert!(s.execute("comment.zoom", &json!({"ids": [12345]})).is_err());
}

#[test]
fn next_steps_through_the_list_across_pages() {
    let mut s = Session::new();
    let a = add(&mut s, 0.0, 0.0, "One", "Sam");
    s.execute("page.add", &json!({"name": "Two"})).unwrap();
    let b = add(&mut s, 0.0, 0.0, "Two", "Sam");
    // Page order: `a` (first page), then `b`.
    let step = |s: &mut Session, from: Option<u64>, dir: i32| {
        s.execute("comment.next", &json!({"id": from, "dir": dir, "page": "all", "sort": "page"})).unwrap()["id"].as_u64().unwrap()
    };
    assert_eq!(step(&mut s, None, 1), a);
    let first_page = s.doc().unwrap().pages[0];
    assert_eq!(s.page().unwrap(), first_page);
    assert_eq!(step(&mut s, Some(a), 1), b);
    assert_ne!(s.page().unwrap(), first_page);
    // The ends stay put.
    assert_eq!(step(&mut s, Some(b), 1), b);
    assert_eq!(step(&mut s, Some(b), -1), a);
    assert_eq!(step(&mut s, Some(a), -1), a);
    assert_eq!(step(&mut s, None, -1), b);
    // Stepping reads the threads.
    assert!(!comment(&s, a).is_unread("You") && !comment(&s, b).is_unread("You"));
    assert!(s.execute("comment.next", &json!({"page": "all", "filter": "resolved"})).is_err());
}

#[test]
fn prompt_briefs_a_thread_for_an_agent() {
    let mut s = Session::new();
    let f = frame(&mut s, 100.0, 100.0, "Button");
    let c = add(&mut s, 150.0, 160.0, "Make the padding 12", "You");
    s.execute("comment.reply", &json!({"id": c, "text": "Agreed", "author": "Sam"})).unwrap();
    let r = s.execute("comment.prompt", &json!({})).unwrap();
    assert_eq!(r["count"], 1);
    assert_eq!(r["truncated"], false);
    let text = r["text"].as_str().unwrap();
    assert!(text.contains("# FigCraft feedback"));
    assert!(text.contains("Make the padding 12"));
    assert!(text.contains("Button"));
    assert!(text.contains(&format!("node {f}")));
    assert!(text.contains("Replies:") && text.contains("Sam: Agreed"));
    assert!(text.contains(&format!("figcraft://comment/{c}")));
    assert!(text.contains("comment.reply") && text.contains("comment.resolve"));
    let list = r["comments"].as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["id"], c);
    assert_eq!(list[0]["anchor"].as_u64(), Some(f.0));
}

#[test]
fn prompt_filters_and_explicit_ids() {
    let mut s = Session::new();
    let a = add(&mut s, 0.0, 0.0, "First", "You");
    let b = add(&mut s, 10.0, 10.0, "Second", "You");
    s.execute("comment.resolve", &json!({"id": b})).unwrap();
    let open = s.execute("comment.prompt", &json!({})).unwrap();
    assert_eq!(open["count"], 1);
    assert_eq!(open["comments"][0]["id"], a);
    let all = s.execute("comment.prompt", &json!({"includeResolved": true})).unwrap();
    assert_eq!(all["count"], 2);
    let resolved = s.execute("comment.prompt", &json!({"filter": "resolved"})).unwrap();
    assert_eq!(resolved["count"], 1);
    assert_eq!(resolved["comments"][0]["id"], b);
    let one = s.execute("comment.prompt", &json!({"ids": [b]})).unwrap();
    assert_eq!(one["count"], 1);
    assert!(one["text"].as_str().unwrap().contains("Second"));
    assert!(s.execute("comment.prompt", &json!({"ids": []})).is_err());
    assert!(s.execute("comment.prompt", &json!({"ids": [999]})).is_err());
    assert!(s.execute("comment.prompt", &json!({"filter": "weird"})).is_err());
    assert!(s.execute("comment.prompt", &json!({"sort": "weird"})).is_err());
    assert!(s.execute("comment.prompt", &json!({"page": 999})).is_err());
}

#[test]
fn prompt_reads_every_page_by_default() {
    let mut s = Session::new();
    let p1 = s.page().unwrap();
    let a = add(&mut s, 10.0, 10.0, "One", "You");
    let p2 = id_of(&s.execute("page.add", &json!({"name": "Two"})).unwrap()["id"]).unwrap();
    s.execute("page.select", &json!({"id": p2})).unwrap();
    let b = add(&mut s, 10.0, 10.0, "Two", "You");
    let all = s.execute("comment.prompt", &json!({})).unwrap();
    assert_eq!(all["count"], 2);
    let one = s.execute("comment.prompt", &json!({"page": p1.0})).unwrap();
    assert_eq!(one["count"], 1);
    assert_eq!(one["comments"][0]["id"], a);
    let two = s.execute("comment.prompt", &json!({"page": p2.0})).unwrap();
    assert_eq!(two["comments"][0]["id"], b);
}

#[test]
fn prompt_is_bounded_and_empty_is_safe() {
    let mut s = Session::new();
    let empty = s.execute("comment.prompt", &json!({})).unwrap();
    assert_eq!(empty["count"], 0);
    assert!(empty["text"].as_str().unwrap().contains("No comments match"));
    for i in 0..201 {
        add(&mut s, f64::from(i), 0.0, "Fix", "You");
    }
    let r = s.execute("comment.prompt", &json!({})).unwrap();
    assert_eq!(r["count"], 201);
    assert_eq!(r["truncated"], true);
    assert!(r["text"].as_str().unwrap().contains("Truncated: 1 more"));
    let briefs = r["comments"].as_array().unwrap();
    assert_eq!(briefs.len(), 200, "bounded metadata, not every thread");
    assert!(briefs[0]["replies"].as_u64().is_some(), "a reply count, not the replies");
}

#[test]
fn prompt_stops_inside_one_huge_thread_without_splitting_a_character() {
    let mut s = Session::new();
    let id = add(&mut s, 10.0, 20.0, "Start", "You");
    let body = "é".repeat(4096);
    for _ in 0..40 {
        s.execute("comment.reply", &json!({"id": id, "text": body, "author": "Sam"})).unwrap();
    }
    let r = s.execute("comment.prompt", &json!({})).unwrap();
    assert_eq!(r["count"], 1);
    assert_eq!(r["truncated"], true);
    let text = r["text"].as_str().unwrap();
    assert!(text.len() <= 256 * 1024, "{} bytes", text.len());
    assert!(text.contains("Truncated") && text.contains("last thread is too long"), "{text}");
    let briefs = r["comments"].as_array().unwrap();
    assert_eq!(briefs.len(), 1);
    assert!(briefs[0]["replies"].as_u64() == Some(40), "a reply count, not the replies");
    assert!(briefs[0]["preview"].as_str().is_some_and(|p| p.chars().count() <= 161));
    assert!(serde_json::to_string(&briefs).unwrap().len() < 2_000, "the metadata stays bounded");
}

#[test]
fn prompt_truncates_a_huge_document_name_even_with_no_comments() {
    let mut s = Session::new();
    s.doc_mut().unwrap().name = "x".repeat(300_000);
    let empty = s.execute("comment.prompt", &json!({})).unwrap();
    assert_eq!(empty["count"], 0);
    assert_eq!(empty["truncated"], true);
    let text = empty["text"].as_str().unwrap();
    assert!(text.len() <= 256 * 1024, "{} bytes", text.len());
    assert!(text.contains("Document name or path is truncated"), "{text}");

    add(&mut s, 10.0, 20.0, "Fix", "You");
    let r = s.execute("comment.prompt", &json!({})).unwrap();
    assert_eq!(r["count"], 1);
    assert_eq!(r["truncated"], true);
    let text = r["text"].as_str().unwrap();
    assert!(text.len() <= 256 * 1024, "{} bytes", text.len());
    assert!(text.contains("Document name or path is truncated"), "{text}");
}

#[test]
fn prompt_keeps_control_characters_out_of_single_line_fields() {
    let mut s = Session::new();
    s.doc_mut().unwrap().name = "Doc\nName".into();
    let page = s.page().unwrap();
    s.doc_mut().unwrap().node_mut(page).unwrap().name = "Pa\nge".into();
    frame(&mut s, 100.0, 100.0, "Fra\nme");
    let id = add(&mut s, 150.0, 160.0, "Fix", "Bad\nAuthor");
    s.execute("comment.reply", &json!({"id": id, "text": "Reply", "author": "Re\nplier"})).unwrap();
    let r = s.execute("comment.prompt", &json!({})).unwrap();
    let text = r["text"].as_str().unwrap();
    for (bad, good) in
        [("Doc\nName", "Doc Name"), ("Pa\nge", "Pa ge"), ("Fra\nme", "Fra me"), ("Bad\nAuthor", "Bad Author"), ("Re\nplier", "Re plier")]
    {
        assert!(!text.contains(bad), "{bad:?} in {text}");
        assert!(text.contains(good), "{good:?} in {text}");
    }
    assert!(text.contains("untrusted feedback data"), "{text}");
}

#[test]
fn prompt_normalizes_carriage_returns_in_messages() {
    let mut s = Session::new();
    let id = add(&mut s, 10.0, 20.0, "First\r\nSecond\rThird", "You");
    s.execute("comment.reply", &json!({"id": id, "text": "A\rB\r\nC", "author": "Sam"})).unwrap();
    let r = s.execute("comment.prompt", &json!({})).unwrap();
    let text = r["text"].as_str().unwrap();
    assert!(!text.contains('\r'), "{text:?}");
    let quoted: Vec<&str> = text.lines().filter(|l| l.starts_with("> ")).collect();
    assert!(quoted.contains(&"> First") && quoted.contains(&"> Second") && quoted.contains(&"> Third"), "{text}");
    assert!(text.contains("- Sam: A B"), "{text}");
}
