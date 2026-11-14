//! Several named AI agents in one file: each works on its own page and selection, the window keeps
//! its own, an agent's undo is refused while another agent is there, and one agent alone works as
//! before.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;

use figcraft_doc::NodeId;
use serde_json::{Value, json};

use crate::Session;

fn id(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn rect(s: &mut Session, agent: Option<&str>) -> NodeId {
    id(&s.execute_as(agent, "shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 40})).unwrap())
}

/// A session with two pages, the window on the first with its rectangle selected; returns the two
/// pages and the rectangle.
fn two_pages() -> (Session, NodeId, NodeId, NodeId) {
    let mut s = Session::new();
    let first = s.page().unwrap();
    s.execute("page.add", &json!({})).unwrap();
    let second = s.page().unwrap();
    assert_ne!(first, second);
    s.execute("page.select", &json!({"id": first})).unwrap();
    let r = rect(&mut s, None);
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    (s, first, second, r)
}

fn agent_kinds(s: &Session) -> Vec<String> {
    s.events.poll(0, &["agents".to_string()], 100, Duration::ZERO).events.into_iter().map(|e| e.kind).collect()
}

#[test]
fn two_agents_work_on_two_pages_and_the_window_keeps_its_own() {
    let (mut s, first, second, r) = two_pages();
    s.agent_join("a").unwrap();
    s.agent_join("b").unwrap();
    let view = s.ds().unwrap().view.clone();

    s.execute_as(Some("b"), "page.select", &json!({"id": second})).unwrap();
    let rb = rect(&mut s, Some("b"));
    let ra = rect(&mut s, Some("a"));

    let ds = s.ds().unwrap();
    assert_eq!(ds.doc.page_of(rb), Some(second), "b drew on its page, which the window does not show");
    assert_eq!(ds.doc.page_of(ra), Some(first));
    assert_eq!((ds.page, ds.selection.clone()), (first, vec![r]), "the window's page and selection are the user's");
    assert_eq!(ds.view, view, "b's page change did not move the window's view");
    let list = s.agent_list();
    assert_eq!(list["agents"][0], json!({"agent": "a", "apart": true, "page": first, "selection": [ra]}));
    assert_eq!(list["agents"][1], json!({"agent": "b", "apart": true, "page": second, "selection": [rb]}));

    // A command on "the selection" takes the calling agent's.
    s.execute_as(Some("b"), "edit.delete", &json!({})).unwrap();
    let doc = &s.ds().unwrap().doc;
    assert!(!doc.contains(rb) && doc.contains(ra) && doc.contains(r));
}

#[test]
fn an_agents_undo_is_refused_while_another_agent_is_there_and_the_users_is_not() {
    let (mut s, first, second, r) = two_pages();
    s.agent_join("a").unwrap();
    s.agent_join("b").unwrap();
    s.execute_as(Some("b"), "page.select", &json!({"id": second})).unwrap();
    let rb = rect(&mut s, Some("b"));

    let e = s.execute_as(Some("a"), "edit.undo", &json!({})).unwrap_err().to_string();
    assert!(e.contains("history is shared"), "{e}");
    assert!(s.execute_as(Some("a"), "edit.redo", &json!({})).is_err());
    assert!(s.ds().unwrap().doc.contains(rb), "nothing was undone");

    assert_eq!(s.execute("edit.undo", &json!({})).unwrap(), json!({"undone": true}));
    let ds = s.ds().unwrap();
    assert!(!ds.doc.contains(rb));
    assert_eq!((ds.page, ds.selection.clone()), (first, vec![r]), "the user's undo of b's step stays on the window's page");

    // Alone again, an agent undoes as before.
    s.agent_leave("b").unwrap();
    let ra = rect(&mut s, Some("a"));
    assert_eq!(s.execute_as(Some("a"), "edit.undo", &json!({})).unwrap(), json!({"undone": true}));
    assert!(!s.ds().unwrap().doc.contains(ra));
}

#[test]
fn one_agent_alone_works_on_the_windows_page_and_selection() {
    let mut s = Session::new();
    let r = rect(&mut s, Some("solo"));
    assert_eq!(s.selection(), vec![r], "its new layer is the window's selection");
    assert_eq!(s.agent_list(), json!({"agents": [{"agent": "solo", "apart": false, "page": null, "selection": null}]}));
    assert_eq!(s.execute_as(Some("solo"), "edit.undo", &json!({})).unwrap(), json!({"undone": true}));
    assert!(agent_kinds(&s).is_empty(), "no agent event for one agent alone");
}

#[test]
fn agents_see_each_other_come_work_and_leave() {
    let (mut s, _, second, _) = two_pages();
    rect(&mut s, Some("a"));
    assert!(agent_kinds(&s).is_empty());
    s.execute_as(Some("b"), "page.select", &json!({"id": second})).unwrap();
    s.agent_leave("b").unwrap();
    assert_eq!(agent_kinds(&s), ["agent.joined", "agent.joined", "agent.selection", "agent.left"]);
    let events = s.events.poll(0, &["agents".to_string()], 100, Duration::ZERO).events;
    assert_eq!((&events[0].data, &events[1].data), (&json!({"agent": "a"}), &json!({"agent": "b"})));
    assert_eq!(events[2].data, json!({"agent": "b", "page": second, "selection": []}));
    assert!(s.agent_join(" ").is_err() && s.agent_leave("").is_err(), "an empty name is no agent");
}

#[test]
fn an_agent_silent_for_too_long_is_forgotten_and_comes_back_as_a_new_one() {
    use crate::agent::AGENT_IDLE;
    let (mut s, _, second, _) = two_pages();
    s.agent_join("a").unwrap();
    s.execute_as(Some("b"), "page.select", &json!({"id": second})).unwrap();
    let now = web_time::Instant::now();
    assert!(s.agents_gone_at(now).is_empty(), "both just made a request");
    assert_eq!(s.agent_list()["agents"].as_array().unwrap().len(), 2);

    // Their clients stopped without `agent.leave`: nothing else would ever take them away.
    assert_eq!(s.agents_gone_at(now + AGENT_IDLE + Duration::from_secs(1)), ["a", "b"]);
    assert_eq!(s.agent_list(), json!({"agents": []}));
    let events = s.events.poll(0, &["agents".to_string()], 100, Duration::ZERO).events;
    let left = events.last().unwrap();
    assert_eq!((left.kind.as_str(), &left.data), ("agent.left", &json!({"agent": "a", "reason": "idle"})));

    // Back, `b` is a new agent: its page and selection were forgotten with it.
    s.agent_join("b").unwrap();
    assert_eq!(s.agent_list(), json!({"agents": [{"agent": "b", "apart": true, "page": null, "selection": null}]}));
}

/// `a` joined and `b` works on the second page, the window on the first with its rectangle
/// selected; returns the two pages and the rectangle.
fn a_and_b_apart() -> (Session, NodeId, NodeId, NodeId) {
    let (mut s, first, second, r) = two_pages();
    s.agent_join("a").unwrap();
    s.execute_as(Some("b"), "page.select", &json!({"id": second})).unwrap();
    (s, first, second, r)
}

/// A bug that panics in the middle of `b`'s call; the MCP server catches it and keeps serving.
fn b_panics(s: &mut Session) {
    let unwound = catch_unwind(AssertUnwindSafe(|| s.with_agent::<()>(Some("b"), |_| panic!("a bug in b's command"))));
    assert!(unwound.is_err(), "the panic goes on to the caller");
}

#[test]
fn a_call_that_panics_gives_the_window_its_page_and_selection_back() {
    let (mut s, first, second, r) = a_and_b_apart();
    b_panics(&mut s);
    let ds = s.ds().unwrap();
    assert_eq!((ds.page, ds.selection.clone()), (first, vec![r]), "the window shows the user's page and selection, not b's");
    assert_eq!(s.agent_list()["agents"][1], json!({"agent": "b", "apart": true, "page": second, "selection": []}), "b keeps its own");
}

#[test]
fn after_a_call_that_panicked_the_next_call_is_its_own_callers() {
    let (mut s, first, _, _) = a_and_b_apart();
    b_panics(&mut s);
    let ra = rect(&mut s, Some("a"));
    assert_eq!(s.ds().unwrap().doc.page_of(ra), Some(first), "a's call ran as a, on a's page, not as b on b's");
    assert_eq!(s.agent_list()["agents"][0], json!({"agent": "a", "apart": true, "page": first, "selection": [ra]}));
    assert_eq!(s.execute("edit.undo", &json!({})).unwrap(), json!({"undone": true}), "the user's undo is never an agent's");
}

#[test]
fn the_first_agent_keeps_its_selection_when_a_second_one_comes() {
    let mut s = Session::new();
    let users = rect(&mut s, None);
    let agents = rect(&mut s, None);
    // Alone, `a` works on the window's selection.
    s.execute_as(Some("a"), "select.set", &json!({"ids": [agents]})).unwrap();
    s.agent_join("b").unwrap();
    // From then on the window's selection is the user's alone.
    s.execute("select.set", &json!({"ids": [users]})).unwrap();

    s.execute_as(Some("a"), "edit.delete", &json!({})).unwrap();
    let ds = s.ds().unwrap();
    assert!(!ds.doc.contains(agents), "a deleted what a had selected");
    assert!(ds.doc.contains(users), "what the user selected after b came is not a's selection");
    assert_eq!(ds.selection, vec![users], "the window's selection is still the user's");
}

#[test]
fn an_agent_on_the_windows_page_does_not_move_the_windows_view() {
    let (mut s, first, _, _) = two_pages();
    s.agent_join("a").unwrap();
    s.agent_join("b").unwrap();
    let view = s.ds().unwrap().view.clone();

    let panned = s.execute_as(Some("a"), "view.pan", &json!({"dx": 3, "dy": -2})).unwrap();
    assert_ne!((panned["cx"].as_f64(), panned["cy"].as_f64()), (Some(view.cx), Some(view.cy)), "the pan did move the view a worked with");
    let ds = s.ds().unwrap();
    assert_eq!(ds.page, first, "a stayed on the page the window shows");
    assert_eq!(ds.view, view, "the window's view is the user's: an agent working apart does not scroll it");
}
