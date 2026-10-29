//! Comments from an AI agent (#1414): a command run as an agent (`Session::execute_as`, which the
//! control channel's and MCP's `engine.execute {agent}` use) signs comments with the agent's own
//! name and marks them as an agent's; the user's own comments stay as they were.

use serde_json::{Value, json};

use crate::Session;
use crate::cmd::comment::agent_author;
use crate::live::initials;

fn as_agent(s: &mut Session, agent: &str, cmd: &str, p: Value) -> Value {
    s.execute_as(Some(agent), cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn thread(s: &mut Session, agent: Option<&str>) -> Value {
    s.execute_as(agent, "comment.list", &json!({"filter": "all"})).unwrap()["comments"][0].clone()
}

#[test]
fn an_agents_comments_carry_its_name_and_the_agent_marker() {
    let mut s = Session::new();
    let c = s.execute("comment.add", &json!({"x": 10, "y": 10, "text": "@Claude make this bigger"})).unwrap()["id"].as_u64().unwrap();
    as_agent(&mut s, "Claude", "comment.reply", json!({"id": c, "text": "Done"}));
    let t = thread(&mut s, None);
    assert_eq!((t["author"].as_str(), t["agent"].as_bool()), (Some("You"), Some(false)), "the user's own comment is unchanged: {t}");
    let r = &t["replies"][0];
    assert_eq!((r["author"].as_str(), r["agent"].as_bool()), (Some("Claude (agent)"), Some(true)), "{r}");
    let doc = s.doc().unwrap();
    let m = &doc.comment(c).unwrap().replies[0];
    assert!(m.agent && m.author == "Claude (agent)");
    // The agent acts as itself again later: no `author` on any call.
    as_agent(&mut s, "Claude", "comment.react", json!({"id": c, "emoji": "👍"}));
    assert_eq!(s.doc().unwrap().comment(c).unwrap().first.reactions[0].users, ["Claude (agent)"]);
    assert_eq!(as_agent(&mut s, "Claude", "comment.people", json!({}))["you"], "Claude (agent)");
    assert_eq!(s.execute("comment.people", &json!({})).unwrap()["you"], "You", "the user's name never changes");
    assert_eq!(s.agent, None, "the agent acts only for its own call");
}

#[test]
fn an_agents_read_state_is_its_own() {
    let mut s = Session::new();
    let c = s.execute("comment.add", &json!({"x": 10, "y": 10, "text": "Look"})).unwrap()["id"].as_u64().unwrap();
    as_agent(&mut s, "Claude", "comment.reply", json!({"id": c, "text": "On it"}));
    assert_eq!(thread(&mut s, None)["unread"], true, "the agent's reply is unread for the user");
    as_agent(&mut s, "Claude", "comment.markRead", json!({}));
    assert_eq!(thread(&mut s, None)["unread"], true, "the agent reading it doesn't read it for the user");
    assert_eq!(thread(&mut s, Some("Claude"))["unread"], false);
}

#[test]
fn an_agent_without_a_name_is_still_not_the_user() {
    let mut s = Session::new();
    as_agent(&mut s, "", "comment.add", json!({"x": 0, "y": 0, "text": "Hi"}));
    let t = thread(&mut s, None);
    assert_eq!((t["author"].as_str(), t["agent"].as_bool()), (Some("AI agent"), Some(true)), "{t}");
    // An explicit `author` still wins, and the message is still an agent's.
    as_agent(&mut s, "Claude", "comment.reply", json!({"id": t["id"], "text": "x", "author": "Helper"}));
    let r = &thread(&mut s, None)["replies"][0];
    assert_eq!((r["author"].as_str(), r["agent"].as_bool()), (Some("Helper"), Some(true)), "{r}");
}

#[test]
fn agent_names_are_cleaned() {
    assert_eq!(agent_author("  Claude  "), "Claude (agent)");
    assert_eq!(agent_author("\u{7}\n "), "AI agent");
    assert_eq!(agent_author("a\u{0}b"), "ab (agent)");
    let long = "x".repeat(500);
    assert_eq!(agent_author(&long).chars().count(), 64 + " (agent)".len());
}

#[test]
fn an_agents_avatar_shows_its_names_initial() {
    assert_eq!(initials("Claude (agent)"), "C");
    assert_eq!(initials("AI agent"), "AA");
    assert_eq!(initials("Ada Lovelace"), "AL");
    assert_eq!(initials("(x) [y]"), "([", "no word starts with a letter: as before");
    assert_eq!(initials("  "), "?");
}
