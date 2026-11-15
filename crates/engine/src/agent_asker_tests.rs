//! Work an agent asks for and that is done later (a batch of raw input waiting for its frame, a
//! window to open) is kept with who asked: the name and the stay it was admitted in. It is done
//! under the name only while that stay lasts: not once the agent left or went silent too long,
//! and not for another client that took the name since.

use std::time::Duration;

use serde_json::json;

use crate::Session;
use crate::agent::AGENT_IDLE;

/// `a` made a request: it is there.
fn with_a() -> Session {
    let mut s = Session::new();
    s.with_agent(Some("a"), |_| ());
    s
}

#[test]
fn an_agent_no_longer_asks_once_it_left() {
    let mut s = with_a();
    let asker = s.agent_asker(Some("a")).unwrap();
    assert!(s.agent_still_there(&asker), "calibration: a is there");

    s.agent_leave("a").unwrap();
    assert!(!s.agent_still_there(&asker), "a left");
    // Another client under the same name is not the one that asked.
    s.with_agent(Some("a"), |_| ());
    assert!(!s.agent_still_there(&asker), "the name was taken again: it is another stay");
    let again = s.agent_asker(Some("a")).unwrap();
    assert!(s.agent_still_there(&again), "whoever holds the name now asks for itself");
}

#[test]
fn an_agent_forgotten_for_its_silence_no_longer_asks() {
    let mut s = with_a();
    let asker = s.agent_asker(Some("a")).unwrap();
    assert_eq!(s.agents_gone_at(web_time::Instant::now() + AGENT_IDLE + Duration::from_secs(1)), ["a"]);
    assert!(!s.agent_still_there(&asker));
}

#[test]
fn a_name_that_is_not_there_asks_for_nothing_and_no_name_holds_nothing() {
    let mut s = Session::new();
    let absent = s.agent_asker(Some("nobody here")).unwrap();
    assert!(!s.agent_still_there(&absent), "a name that was not there when it asked");
    assert!(s.agent_asker(None).is_none(), "a request that names nobody");
    // A client that gave no name has no stay to lose: it is replayed as before.
    let unnamed = s.agent_asker(Some("")).unwrap();
    assert!(s.agent_still_there(&unnamed));
}

#[test]
fn the_same_agent_is_named_whatever_the_spaces_around_its_name() {
    let s = with_a();
    let asker = s.agent_asker(Some(" a ")).unwrap();
    assert!(asker.names("a") && !asker.names("b"));
    assert_eq!(asker.agent(), " a ", "the name is kept as the request gave it");
}

/// A request to the interface is handled after the call that made it: it carries the agent whose
/// call it was, so the interface handles it as that agent.
#[test]
fn a_request_to_the_interface_carries_the_agent_that_asked() {
    let mut s = Session::new();
    s.with_agent(Some("a"), |s| s.request_ui("window.new", json!({})));
    let asked = s.ui_requests.last().unwrap().agent.clone();
    assert_eq!(asked, s.agent_asker(Some("a")));
    assert!(asked.is_some_and(|a| s.agent_still_there(&a)));

    s.request_ui("window.new", json!({}));
    assert_eq!(s.ui_requests.last().unwrap().agent, None, "the user's request names nobody");
}

/// An agent working apart is told at once that the window it asks for would bring another
/// document in front; alone, its request is queued.
#[test]
fn window_new_is_refused_at_once_to_an_agent_working_apart() {
    let mut s = Session::new();
    s.with_agent(Some("a"), |s| s.execute("window.new", &json!({}))).unwrap();
    assert_eq!(s.ui_requests.len(), 1, "calibration: alone, a's request is queued");

    s.with_agent(Some("b"), |_| ());
    let r = s.with_agent(Some("a"), |s| s.execute("window.new", &json!({})));
    assert!(matches!(r, Err(crate::EngineError::Disabled(ref c, _)) if c == "window.new"), "{r:?}");
    assert_eq!(s.ui_requests.len(), 1, "nothing more was queued");
    // Moving a file that is open into a window of its own changes no document: still allowed.
    s.with_agent(Some("a"), |s| s.execute("window.new", &json!({"index": 0}))).unwrap();
}
