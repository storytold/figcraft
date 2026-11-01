//! `comment.list` reports the names each message @mentions, by the rule the thread view highlights
//! them with: resolved against the file's people where it can be, and whether the name is you.
//! Its `mentions` param lists only the threads where a message mentions a name.

use serde_json::{Value, json};

use crate::Session;

fn mentions(s: &mut Session, p: Value) -> Vec<Value> {
    let l = s.execute("comment.list", &p).unwrap();
    let c = &l["comments"][0];
    [&c["mentions"], &c["replies"][0]["mentions"]].into_iter().cloned().collect()
}

#[test]
fn a_message_lists_the_names_it_mentions_and_not_an_email_address() {
    let mut s = Session::new();
    let id = s.execute("comment.add", &json!({"x": 0, "y": 0, "text": "Hi", "author": "Ada Lovelace"})).unwrap()["id"].as_u64().unwrap();
    let text = "@ada lovelace and @You: please check, or ask @Bob (mail a@b.com). Thanks @Ada Lovelace";
    s.execute("comment.reply", &json!({"id": id, "text": text, "author": "Sam"})).unwrap();
    let [first, reply] = <[Value; 2]>::try_from(mentions(&mut s, json!({}))).unwrap();
    assert_eq!(first, json!([]));
    assert_eq!(
        reply,
        json!([
            {"name": "Ada Lovelace", "known": true, "you": false},
            {"name": "You", "known": true, "you": true},
            {"name": "Bob", "known": false, "you": false},
        ]),
        "two people the file knows (once each, as the file spells them), one name it doesn't, no email"
    );
    // `author` says whose list it is: then Ada is "you".
    let reply = mentions(&mut s, json!({"author": "Ada Lovelace"})).remove(1);
    assert_eq!(reply[0], json!({"name": "Ada Lovelace", "known": true, "you": true}));
}

#[test]
fn the_mention_rule_matches_the_highlighting_rule() {
    let people = ["Sam", "Sam Lee"];
    let found = figcraft_doc::mentions("@Sam Lee, @sam, x@Sam, @Zoe!", &people);
    let names: Vec<_> = found.iter().map(|(r, who)| (r.clone(), *who)).collect();
    assert_eq!(names, vec![(0..8, Some("Sam Lee")), (10..14, Some("Sam")), (23..27, None)]);
    assert_eq!(figcraft_doc::mention_ranges("@Sam Lee, @sam, x@Sam, @Zoe!", &people), vec![0..8, 10..14]);
    assert!(figcraft_doc::mentions("@ and a@b.com", &people).is_empty());
}

/// The ids of the threads `comment.list` returns for `p`.
fn listed(s: &mut Session, p: Value) -> Vec<u64> {
    let l = s.execute("comment.list", &p).unwrap();
    l["comments"].as_array().unwrap().iter().map(|c| c["id"].as_u64().unwrap()).collect()
}

fn add(s: &mut Session, text: &str, author: &str) -> u64 {
    s.execute("comment.add", &json!({"x": 0, "y": 0, "text": text, "author": author})).unwrap()["id"].as_u64().unwrap()
}

#[test]
fn the_mentions_param_lists_only_the_threads_that_mention_a_name() {
    let mut s = Session::new();
    let claude = add(&mut s, "@Claude make this bigger", "Sam");
    let in_reply = add(&mut s, "Spacing looks off", "Sam");
    s.execute("comment.reply", &json!({"id": in_reply, "text": "agreed, @claude: tighten it", "author": "Ada Lovelace"})).unwrap();
    let other = add(&mut s, "@Ada Lovelace and @Zoe Smith, mail claude@example.com", "Sam");
    add(&mut s, "No mentions here", "Sam");
    let mut both = listed(&mut s, json!({"mentions": "CLAUDE"}));
    both.sort_unstable();
    assert_eq!(both, vec![claude, in_reply], "any case, in a reply too, never an email address");
    assert_eq!(listed(&mut s, json!({"mentions": " @Claude "})), listed(&mut s, json!({"mentions": "claude"})), "a leading @ is fine");
    assert_eq!(listed(&mut s, json!({"mentions": "ada lovelace"})), vec![other]);
    assert!(listed(&mut s, json!({"mentions": "Ada"})).is_empty(), "@Ada Lovelace names Ada Lovelace, not Ada");
    assert_eq!(listed(&mut s, json!({"mentions": "Zoe Smith"})), vec![other], "a name nobody in the file has, several words");
    assert_eq!(listed(&mut s, json!({"mentions": null})).len(), 4);
    // Combined with the other filters: resolved threads drop out.
    s.execute("comment.resolve", &json!({"id": claude})).unwrap();
    assert_eq!(listed(&mut s, json!({"mentions": "Claude", "filter": "open"})), vec![in_reply]);
    for junk in [json!(""), json!("@"), json!(5), json!(["Claude"])] {
        assert!(s.execute("comment.list", &json!({"mentions": junk})).is_err(), "{junk}");
    }
}

#[test]
fn people_lists_the_names_only_mentioned_too() {
    let mut s = Session::new();
    let id = add(&mut s, "@ada lovelace, @Zoe and @zoe: see me@example.com", "Sam");
    s.execute("comment.reply", &json!({"id": id, "text": "On it", "author": "Ada Lovelace"})).unwrap();
    s.execute("comment.react", &json!({"id": id, "emoji": "👍", "author": "Lin"})).unwrap();
    let p = s.execute("comment.people", &json!({})).unwrap();
    // Ada once, spelt as the file spells her; Zoe once in any case; no email address.
    assert_eq!(p["people"], json!(["Ada Lovelace", "Lin", "Sam", "Zoe"]));
    assert_eq!(p["you"], "You");
}

#[test]
fn people_who_write_in_two_cases_are_listed_once() {
    let mut s = Session::new();
    let id = add(&mut s, "Hi", "Ada");
    s.execute("comment.reply", &json!({"id": id, "text": "Hi @ADA", "author": "ada"})).unwrap();
    let p = s.execute("comment.people", &json!({})).unwrap();
    assert_eq!(p["people"].as_array().map(Vec::len), Some(1), "{}", p["people"]);
}

#[test]
fn next_and_prompt_carry_the_mentions_too() {
    let mut s = Session::new();
    let first = add(&mut s, "@Claude make this bigger", "Sam");
    let second = add(&mut s, "Spacing looks off", "Sam");
    s.execute("comment.reply", &json!({"id": second, "text": "@Claude and @Bob, tighten it", "author": "Ada"})).unwrap();
    let plain = add(&mut s, "No mentions here", "Sam");
    let claude = json!({"name": "Claude", "known": false, "you": false});
    // comment.next returns the thread as comment.list lists it, mentions and all.
    let list = s.execute("comment.list", &json!({})).unwrap();
    let ids: Vec<u64> = list["comments"].as_array().unwrap().iter().map(|c| c["id"].as_u64().unwrap()).collect();
    let next = s.execute("comment.next", &json!({"id": ids[0]})).unwrap();
    let listed = list["comments"].as_array().unwrap().iter().find(|c| c["id"] == next["id"]).unwrap();
    assert_eq!(next["mentions"], listed["mentions"]);
    assert_eq!(next["replies"], listed["replies"]);
    // Stepping through the threads that mention Claude skips the others.
    let n = s.execute("comment.next", &json!({"mentions": "claude"})).unwrap();
    let m = s.execute("comment.next", &json!({"mentions": "claude", "id": n["id"]})).unwrap();
    let mut both = vec![n["id"].as_u64().unwrap(), m["id"].as_u64().unwrap()];
    both.sort_unstable();
    assert_eq!(both, vec![first, second]);
    assert!(s.execute("comment.next", &json!({"mentions": 5})).is_err());
    // comment.prompt: each thread's mentions, the first message's and the replies' together.
    let prompt = s.execute("comment.prompt", &json!({})).unwrap();
    let mentions = |id: u64| prompt["comments"].as_array().unwrap().iter().find(|c| c["id"] == id).unwrap()["mentions"].clone();
    assert_eq!(mentions(first), json!([claude]));
    assert_eq!(mentions(second), json!([claude, {"name": "Bob", "known": false, "you": false}]));
    assert_eq!(mentions(plain), json!([]));
}

#[test]
fn a_prompt_lists_a_bounded_number_of_short_mentions() {
    let mut s = Session::new();
    let many: String = (0..100).map(|i| format!("@n{i} ")).collect();
    let id = add(&mut s, &format!("{many} @{}", "x".repeat(500)), "Sam");
    let prompt = s.execute("comment.prompt", &json!({"ids": [id]})).unwrap();
    let mentions = prompt["comments"][0]["mentions"].as_array().unwrap().clone();
    assert_eq!(mentions.len(), 32);
    let id = add(&mut s, &format!("@{}", "y".repeat(500)), "Sam");
    let long = s.execute("comment.prompt", &json!({"ids": [id]})).unwrap();
    assert!(long["comments"][0]["mentions"][0]["name"].as_str().unwrap().chars().count() <= 81);
}
