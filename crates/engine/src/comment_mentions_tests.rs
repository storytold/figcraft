//! `comment.list` reports the names each message @mentions, by the rule the thread view highlights
//! them with: resolved against the file's people where it can be, and whether the name is you.

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
