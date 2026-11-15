//! What an agent asked the window to do later is done for that agent only. The raw input it
//! queued (`ui.input`) is not played once it left, nor for another client that took its name; the
//! window it asked for (`window.new`) is not opened once it left, and is opened under the rules
//! the agent works under, not as nobody.

use egui::ViewportId;
use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::control::handle;
use crate::layers_drag_tests::Ui;

fn call(u: &mut Ui, method: &str, params: Value) -> Result<Value, String> {
    let ctx = u.ctx.clone();
    handle(&mut u.app, &ctx, method, &params)
}

/// `p` made for agent `who`, with the key it holds its name with.
fn as_who(mut p: Value, who: &str) -> Value {
    p["agent"] = json!(who);
    p["agentKey"] = json!(format!("key {who}"));
    p
}

fn run(u: &mut Ui, who: &str, command: &str, params: Value) -> Result<Value, String> {
    call(u, "engine.execute", as_who(json!({"command": command, "params": params}), who))
}

fn batch() -> Value {
    json!({"events": [{"move": [5.0, 5.0]}]})
}

/// `a` and `b` are there with their keys.
fn two_agents() -> Ui {
    let mut u = Ui::new(Session::new());
    for name in ["a", "b"] {
        call(&mut u, "agent.join", as_who(json!({}), name)).unwrap();
    }
    u
}

/// How many files are open.
fn files(u: &Ui) -> usize {
    u.app.session.docs.len()
}

// ---------------------------------------------------------------------------------------------
// Raw input waiting for its frame
// ---------------------------------------------------------------------------------------------

#[test]
fn the_input_an_agent_queued_is_dropped_when_it_leaves() {
    let mut u = two_agents();
    call(&mut u, "ui.input", as_who(batch(), "a")).unwrap();
    assert!(!u.app.control_input.is_empty(), "calibration: a's batch waits for its frame");

    call(&mut u, "agent.leave", as_who(json!({}), "a")).unwrap();
    assert!(u.app.control_input.is_empty(), "a left, and its batch still waits for a frame");
    // Nothing of a's keeps b's input out.
    call(&mut u, "ui.input", as_who(batch(), "b")).unwrap();
}

#[test]
fn a_batch_is_not_played_for_another_client_that_took_the_name() {
    let mut u = two_agents();
    call(&mut u, "ui.input", as_who(batch(), "a")).unwrap();
    // `a` is forgotten without its `agent.leave` reaching the window (as when it stays silent too
    // long), and another client takes the name with a key of its own.
    u.app.session.agent_leave("a").unwrap();
    call(&mut u, "agent.join", json!({"agent": "a", "agentKey": "another client's"})).unwrap();

    let events = crate::windows::take_input(&mut u.app, ViewportId::ROOT);
    assert!(events.is_empty(), "the batch of the one that left is played under the name another client holds");
    assert!(u.app.applied_agent.is_none());
}

#[test]
fn an_agent_does_not_leave_while_the_frame_plays_its_input() {
    let mut u = two_agents();
    call(&mut u, "ui.input", as_who(batch(), "a")).unwrap();
    let events = crate::windows::take_input(&mut u.app, ViewportId::ROOT);
    assert!(!events.is_empty(), "calibration: a's batch is in the frame");

    let r = call(&mut u, "agent.leave", as_who(json!({}), "a"));
    assert!(r.as_ref().is_err_and(|e| e.contains("being applied")), "{r:?}");
    // The frame over, it leaves.
    u.frame(events);
    call(&mut u, "agent.leave", as_who(json!({}), "a")).unwrap();
}

// ---------------------------------------------------------------------------------------------
// A window asked for with the `window.new` command, opened by a later frame
// ---------------------------------------------------------------------------------------------

#[test]
fn a_window_is_not_opened_for_an_agent_that_left() {
    let mut u = Ui::new(Session::new());
    // Alone, `a` may ask for a window with a new file: the request waits for a frame.
    run(&mut u, "a", "window.new", json!({})).unwrap();
    let before = files(&u);

    call(&mut u, "agent.leave", as_who(json!({}), "a")).unwrap();
    u.frame(vec![]);
    assert_eq!(files(&u), before, "a file was opened for an agent that is no longer there");
}

#[test]
fn a_window_asked_for_alone_is_not_opened_once_the_agent_works_apart() {
    let mut u = Ui::new(Session::new());
    run(&mut u, "a", "window.new", json!({})).unwrap();
    let before = (files(&u), u.app.session.active);

    // A second agent comes before the frame opens the window: `a` works apart from now on, and an
    // agent working apart does not change the document in front.
    call(&mut u, "agent.join", as_who(json!({}), "b")).unwrap();
    u.frame(vec![]);
    assert_eq!((files(&u), u.app.session.active), before, "the window opened a file past the rule its agent works under");
}

#[test]
fn an_agent_working_apart_is_told_at_once_that_its_window_is_refused() {
    let mut u = two_agents();
    let before = files(&u);

    let r = run(&mut u, "a", "window.new", json!({}));
    u.frame(vec![]);
    assert_eq!(files(&u), before, "a file came in front for an agent working apart");
    assert!(r.is_err(), "the agent was told its window would open: {r:?}");
}
