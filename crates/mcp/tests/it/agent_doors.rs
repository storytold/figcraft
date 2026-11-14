//! Every method `Headless` answers, against every rule that holds for an AI agent at work with
//! another one present. One function per rule says, for each method, the request that tries the
//! rule through that door, or why the door cannot reach it; the tests walk their own lists of
//! methods. A method added without being placed in a rule makes the rule's function fail.
//!
//! The requests name the agent themselves (`agent`, `agentKey`), as the MCP server does.

use figcraft_mcp::{Backend, Headless};
use serde_json::{Value, json};

/// Every method of the control protocol, whether `Headless` answers it or not (a window answers
/// the others: the rules say why they do not reach it from here).
pub(crate) const METHODS: &[&str] = &[
    "engine.execute",
    "engine.commands",
    "document.inspect",
    "node.get",
    "ui.inspect",
    "ui.pointer",
    "ui.key",
    "ui.type",
    "ui.render",
    "ui.screenshot",
    "ui.input",
    "ui.set",
    "ui.menu.invoke",
    "window.new",
    "window.list",
    "window.focus",
    "events.poll",
    "agent.join",
    "agent.leave",
    "agent.list",
    "app.quit",
];

/// `method` as the MCP server sends it for the agent `name`, holding that name with `key-<name>`.
fn as_agent(h: &mut Headless, method: &str, mut p: Value, name: &str) -> Result<Value, String> {
    p["agent"] = json!(name);
    p["agentKey"] = json!(format!("key-{name}"));
    h.call(method, p)
}

/// `method` as the user (no agent).
fn as_user(h: &mut Headless, method: &str, p: Value) -> Result<Value, String> {
    h.call(method, p)
}

fn exec(command: &str, params: Value) -> Value {
    json!({"command": command, "params": params})
}

/// Agents `a` and `b` are there (both apart), and `b` drew a rectangle at (0, 0), 10 x 10: the
/// history holds an edit of `b`'s, and the window's page holds something to hit.
fn two_agents() -> Headless {
    let mut h = Headless::new();
    for name in ["a", "b"] {
        as_agent(&mut h, "agent.join", json!({"agent": name}), name).unwrap();
    }
    as_agent(&mut h, "engine.execute", exec("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})), "b").unwrap();
    h
}

/// Only `a` is there, and it drew a rectangle: nobody else's history to protect.
fn alone() -> Headless {
    let mut h = Headless::new();
    as_agent(&mut h, "agent.join", json!({"agent": "a"}), "a").unwrap();
    as_agent(&mut h, "engine.execute", exec("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})), "a").unwrap();
    h
}

/// Layers on the window's first page.
fn drawn(h: &mut Headless) -> usize {
    let doc = as_user(h, "document.inspect", json!({})).unwrap();
    doc["pages"][0]["children"].as_array().map_or(0, Vec::len)
}

/// Steps in the history, as the window sees it.
fn undo_steps(h: &mut Headless) -> usize {
    let doc = as_user(h, "document.inspect", json!({})).unwrap();
    doc["history"]["undo"].as_array().map_or(0, Vec::len)
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or(f64::NAN)
}

// ---------------------------------------------------------------------------------------------
// Rule 1: a name held with another key is refused by every door that is not about membership.
// ---------------------------------------------------------------------------------------------

/// The request that tries to act under a held name through `method`.
pub(crate) fn admitted_through(method: &str) -> Option<Value> {
    match method {
        "engine.execute" => Some(exec("select.all", json!({}))),
        "engine.commands" | "document.inspect" | "ui.inspect" | "ui.render" | "ui.screenshot" | "app.quit" => Some(json!({})),
        "node.get" => Some(json!({"id": 1})),
        "ui.pointer" => Some(json!({"kind": "hover", "x": 0, "y": 0})),
        "ui.key" => Some(json!({"key": "A", "cmd": true})),
        "ui.type" => Some(json!({"text": "x"})),
        // Headless has no window: these are answered by the window only.
        "ui.input" | "ui.set" | "ui.menu.invoke" | "window.new" | "window.list" | "window.focus" => None,
        // The event log is the same for every client and its reply names nobody: nothing is admitted.
        "events.poll" => None,
        // Membership: each is admitted by its own rule (a join under a held name is refused as
        // such, a list is for anyone), and runs under nobody's name.
        "agent.join" | "agent.leave" | "agent.list" => None,
        _ => panic!("new method: say how it reaches this rule: {method}"),
    }
}

/// The event log is read by anyone, whatever name or key the request gives: it is answered, not
/// refused, under a name held with another key. Calibration: the same request with a held name
/// and another key is refused for a method that does admit.
#[test]
fn the_event_log_is_answered_for_nobody() {
    let mut h = two_agents();
    let mut p = json!({"after": 0});
    p["agent"] = json!("a");
    p["agentKey"] = json!("not a's");
    let r = h.call("events.poll", p);
    assert!(r.is_ok(), "events.poll was refused for a name: {r:?}");
    let mut p = json!({});
    p["agent"] = json!("a");
    p["agentKey"] = json!("not a's");
    let refused = h.call("document.inspect", p);
    assert!(matches!(&refused, Err(e) if e.contains("held by another client")), "calibration: {refused:?}");
}

/// Calibration: with the right key the request is not refused for its name. Then, with another
/// key, every door refuses it, whatever it would have done.
#[test]
fn a_held_name_with_another_key_is_refused_by_every_door() {
    for method in METHODS {
        let Some(p) = admitted_through(method) else { continue };
        let mut h = two_agents();
        let own = as_agent(&mut h, method, p.clone(), "a");
        assert!(!matches!(&own, Err(e) if e.contains("held by another client")), "{method}: the right key is refused: {own:?}");
        let mut other = p;
        other["agent"] = json!("a");
        other["agentKey"] = json!("not a's");
        let r = h.call(method, other);
        assert!(matches!(&r, Err(e) if e.contains("held by another client")), "{method}: a held name with another key got through: {r:?}");
    }
}

/// A request that gives no key under a held name is refused the same way.
#[test]
fn a_held_name_without_a_key_is_refused_by_every_door() {
    for method in METHODS {
        let Some(mut p) = admitted_through(method) else { continue };
        let mut h = two_agents();
        p["agent"] = json!("a");
        let r = h.call(method, p);
        assert!(matches!(&r, Err(e) if e.contains("held by another client")), "{method}: a held name without its key got through: {r:?}");
    }
}

// ---------------------------------------------------------------------------------------------
// Rule 2: through every door that is the agent's own, it acts and reads on ITS page and
// selection, and the window finds its own again afterwards.
// ---------------------------------------------------------------------------------------------

/// The scene of the placement tests: `a` is on a page of its own ("Pa") with a rectangle and a
/// text layer, `b` drew on the window's page, and the window has nothing selected.
struct Scene {
    h: Headless,
    rect: Value,
    text: Value,
    page: Value,
    window_page: Value,
}

fn scene() -> Scene {
    let mut h = two_agents();
    let window_page = as_user(&mut h, "document.inspect", json!({})).unwrap()["currentPage"].clone();
    let page = as_agent(&mut h, "engine.execute", exec("page.add", json!({"name": "Pa"})), "a").unwrap()["id"].clone();
    let rect =
        as_agent(&mut h, "engine.execute", exec("shape.rectangle", json!({"x": 200, "y": 200, "width": 50, "height": 30})), "a").unwrap()["id"]
            .clone();
    let text = as_agent(&mut h, "engine.execute", exec("node.create", json!({"type": "text", "x": 200, "y": 300, "text": "Hi"})), "a").unwrap()["id"]
        .clone();
    Scene { h, rect, text, page, window_page }
}

/// Calibration for every placement test: the scene shows `a` and the window apart. `a` is on its
/// own page with its own selection, the window is on its first page with nothing selected.
/// Without that, a door that acts on the window's page and selection would look right.
fn calibrate_scene(s: &mut Scene) {
    let seen_by_a = as_agent(&mut s.h, "document.inspect", json!({}), "a").unwrap();
    assert_eq!(seen_by_a["currentPage"], s.page, "a is on its own page");
    assert_ne!(s.page, s.window_page, "the pages differ");
    let window = as_user(&mut s.h, "document.inspect", json!({})).unwrap();
    assert_eq!(window["currentPage"], s.window_page, "the window stays on its page");
    assert_eq!(window["selection"], json!([]), "the window has nothing selected");
}

/// What `a` selects before the request.
#[derive(Clone, Copy)]
pub(crate) enum Select {
    Rect,
    Text,
    /// The text layer, with its editor open (`text.edit.begin`).
    TextEditing,
}

type Check = fn(&mut Scene, Result<Value, String>);

/// One attempt: what `a` selects, the request and what must be true after it.
pub(crate) struct Placed {
    select: Select,
    request: Value,
    check: Check,
}

fn x_of(s: &mut Scene, id: &Value) -> f64 {
    num(&s.h.call("node.get", json!({"id": id})).unwrap()["x"])
}

/// The requests that act or read through `method`, and what shows they landed on `a`'s page and
/// selection.
pub(crate) fn placed_through(method: &str) -> Option<Vec<Placed>> {
    let key = |k: &str| json!({"key": k});
    match method {
        "engine.execute" => Some(vec![Placed {
            select: Select::Rect,
            request: exec("transform.nudge", json!({"dx": 3, "dy": 0})),
            check: |s, r| {
                r.unwrap();
                let rect = s.rect.clone();
                assert_eq!(x_of(s, &rect), 203.0, "the nudge moved a's selection");
            },
        }]),
        "ui.pointer" => Some(vec![Placed {
            select: Select::Rect,
            request: json!({"kind": "hover", "x": 0, "y": 0}),
            check: |s, r| assert_eq!(r.unwrap()["selection"], json!([s.rect]), "the reply is a's selection, not the window's"),
        }]),
        "ui.key" => Some(vec![
            Placed {
                select: Select::Rect,
                request: key("ArrowRight"),
                check: |s, r| {
                    r.unwrap();
                    let rect = s.rect.clone();
                    assert_eq!(x_of(s, &rect), 201.0, "the arrow nudged a's selection (with nothing selected in the window it would pan)");
                },
            },
            Placed {
                select: Select::Rect,
                request: key("Delete"),
                check: |s, r| {
                    r.unwrap();
                    assert!(s.h.call("node.get", json!({"id": s.rect})).is_err(), "Delete removed a's selection");
                    assert!(s.h.call("node.get", json!({"id": s.text})).is_ok(), "and only that");
                },
            },
            Placed {
                select: Select::Text,
                request: key("Enter"),
                check: |s, r| {
                    r.unwrap();
                    let a = as_agent(&mut s.h, "document.inspect", json!({}), "a").unwrap();
                    assert!(!a["textEdit"].is_null(), "Enter opened the editor of the layer a selected: {a}");
                    let window = as_user(&mut s.h, "document.inspect", json!({})).unwrap();
                    assert!(window["textEdit"].is_null(), "the window has no editor open");
                },
            },
        ]),
        "ui.type" => Some(vec![Placed {
            select: Select::TextEditing,
            request: json!({"text": "Z"}),
            check: |s, r| {
                r.unwrap();
                let n = s.h.call("node.get", json!({"id": s.text})).unwrap();
                assert!(n["characters"].as_str().is_some_and(|c| c.contains('Z')), "typed into the editor a opened: {n}");
            },
        }]),
        "document.inspect" => Some(vec![Placed {
            select: Select::Rect,
            request: json!({}),
            check: |s, r| {
                let r = r.unwrap();
                assert_eq!(
                    (r["currentPage"].clone(), r["selection"].clone()),
                    (s.page.clone(), json!([s.rect])),
                    "a reads its own page and selection"
                );
            },
        }]),
        "node.get" => Some(vec![Placed {
            select: Select::Rect,
            request: json!({"id": "<rect>"}),
            check: |s, r| assert_eq!(r.unwrap()["id"], s.rect, "a reads the layer on its page"),
        }]),
        "ui.render" => Some(vec![Placed {
            select: Select::Rect,
            request: json!({}),
            check: |s, r| {
                let r = r.unwrap();
                // The window's page holds a 10 x 10 rectangle, a's page a 50 x 30 one and a text.
                assert!(num(&r["width"]) >= 50.0, "the render is a's page, not the window's: {}", r["width"]);
                // Calibration: the window's own render of its page is narrower, so the width tells the pages apart.
                let window = s.h.call("ui.render", json!({})).unwrap();
                assert!(num(&window["width"]) < 50.0, "the window's page is the small one: {}", window["width"]);
            },
        }]),
        // Reads that name no page or selection of an agent, or that the window alone answers.
        "engine.commands" => None, // reads: the registry, the same for everyone
        "ui.inspect" => None,      // reads the window's own settings: the window door
        "ui.screenshot" => None,   // the window's picture: the window door
        "ui.input" | "ui.set" | "ui.menu.invoke" | "window.new" | "window.list" | "window.focus" => None, // Headless has no window
        "events.poll" => None,     // the event log is the same for every client: no page or selection
        "app.quit" => None,        // the window door: nothing of an agent's page or selection
        "agent.join" | "agent.leave" | "agent.list" => None, // membership: run under nobody's name
        _ => panic!("new method: say how it reaches this rule: {method}"),
    }
}

fn prepare(s: &mut Scene, select: Select) {
    let (ids, editing) = match select {
        Select::Rect => (json!([s.rect]), false),
        Select::Text => (json!([s.text]), false),
        Select::TextEditing => (json!([s.text]), true),
    };
    as_agent(&mut s.h, "engine.execute", exec("select.set", json!({"ids": ids})), "a").unwrap();
    if editing {
        as_agent(&mut s.h, "engine.execute", exec("text.edit.begin", json!({"id": s.text})), "a").unwrap();
    }
}

#[test]
fn an_agents_own_doors_act_and_read_on_its_page_and_selection() {
    for method in METHODS {
        let Some(list) = placed_through(method) else { continue };
        for attempt in list {
            let mut s = scene();
            calibrate_scene(&mut s);
            prepare(&mut s, attempt.select);
            let mut request = attempt.request.clone();
            if request["id"] == "<rect>" {
                request["id"] = s.rect.clone();
            }
            let r = as_agent(&mut s.h, method, request.clone(), "a");
            (attempt.check)(&mut s, r);
            let window = as_user(&mut s.h, "document.inspect", json!({})).unwrap();
            assert_eq!(
                (window["currentPage"].clone(), window["selection"].clone()),
                (s.window_page.clone(), json!([])),
                "{method} {request}: the window found its own page and selection again"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Rule 3: undo and redo of an agent are refused, with another agent there, by every door that
// reaches them.
// ---------------------------------------------------------------------------------------------

/// The requests that try an undo or a redo through `method`.
pub(crate) fn undo_through(method: &str) -> Option<Vec<Value>> {
    match method {
        "engine.execute" => Some(vec![
            exec("edit.undo", json!({})),
            exec("edit.redo", json!({})),
            exec("command.batch", json!({"commands": [{"command": "edit.undo", "params": {}}]})),
            exec("command.batch", json!({"commands": [{"command": "edit.redo", "params": {}}]})),
        ]),
        "ui.key" => Some(vec![json!({"key": "Z", "cmd": true}), json!({"key": "Z", "cmd": true, "shift": true})]),
        // Headless has no window: a menu line or raw input reaches undo only there.
        "ui.input" | "ui.menu.invoke" => None,
        "engine.commands" | "document.inspect" | "node.get" | "ui.inspect" | "ui.render" | "ui.screenshot" | "events.poll" => None, // reads: cannot reach undo
        "ui.pointer" | "ui.type" => None, // a pointer event and typed text hold no shortcut
        "ui.set" | "window.new" | "window.list" | "window.focus" | "app.quit" => None, // the window's own settings: no history
        "agent.join" | "agent.leave" | "agent.list" => None, // membership: no history
        _ => panic!("new method: say how it reaches this rule: {method}"),
    }
}

/// Calibration: with nobody else there, the same requests take `a`'s own steps back (a redo after
/// an undo). It is the document that shows it: a batch is an undo step itself, so an undo inside
/// one leaves the history as deep as before. Then, with `b` there, each is refused and `b`'s
/// rectangle and the history stay.
#[test]
fn an_agents_undo_and_redo_are_refused_by_every_door_while_another_is_there() {
    for method in METHODS {
        let Some(list) = undo_through(method) else { continue };
        for request in list {
            // A redo is the command by its id, or its key: the undo key with Shift.
            let redo = request.to_string().contains("edit.redo") || request["shift"] == true;
            let mut lone = alone();
            if redo {
                as_user(&mut lone, "engine.execute", exec("edit.undo", json!({}))).unwrap();
            }
            let shown = drawn(&mut lone);
            as_agent(&mut lone, method, request.clone(), "a").unwrap_or_else(|e| panic!("{method} {request}: refused with nobody else there: {e}"));
            assert_ne!(drawn(&mut lone), shown, "{method} {request}: the document did not move when alone");

            let mut h = two_agents();
            if redo {
                // The user takes `b`'s rectangle back, so there is something to redo.
                as_user(&mut h, "engine.execute", exec("edit.undo", json!({}))).unwrap();
            }
            let (shown, steps) = (drawn(&mut h), undo_steps(&mut h));
            let r = as_agent(&mut h, method, request.clone(), "a");
            assert!(matches!(&r, Err(e) if e.contains("history is shared")), "{method} {request}: got {r:?}");
            assert_eq!((drawn(&mut h), undo_steps(&mut h)), (shown, steps), "{method} {request}: the history is intact");
        }
    }
}

/// The user's undo is never refused, with agents at work (nor by key).
#[test]
fn the_users_undo_works_while_agents_are_there() {
    for method in METHODS {
        let Some(list) = undo_through(method) else { continue };
        for request in list.into_iter().filter(|r| !r.to_string().contains("edit.redo") && !r.get("shift").is_some_and(|s| s.as_bool() == Some(true)))
        {
            let mut h = two_agents();
            assert_eq!(drawn(&mut h), 1, "b's rectangle is in the history");
            as_user(&mut h, method, request.clone()).unwrap_or_else(|e| panic!("{method} {request}: the user's undo was refused: {e}"));
            assert_eq!(drawn(&mut h), 0, "{method} {request}: the user's undo took the rectangle away, as it may");
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Rule 4: during a gesture that is not its own, an agent neither acts, nor continues it, nor
// ends it.
// ---------------------------------------------------------------------------------------------

/// The requests that try to act, continue or end a gesture through `method`.
pub(crate) fn gesture_through(method: &str) -> Option<Vec<Value>> {
    match method {
        "engine.execute" => Some(vec![exec("shape.rectangle", json!({"x": 300, "y": 300, "width": 5, "height": 5}))]),
        "ui.pointer" => Some(vec![
            json!({"kind": "move", "x": 60, "y": 60}),
            json!({"kind": "up", "x": 60, "y": 60}),
            json!({"kind": "click", "x": 400, "y": 400}),
            json!({"kind": "down", "x": 400, "y": 400}),
        ]),
        // A tool letter ends the gesture's tool, Escape cancels the gesture.
        "ui.key" => Some(vec![json!({"key": "R"}), json!({"key": "Escape"})]),
        "ui.type" => Some(vec![json!({"text": "Z"})]),
        // Headless has no window: raw input and the menu reach a gesture only there.
        "ui.input" | "ui.menu.invoke" => None,
        "engine.commands" | "document.inspect" | "node.get" | "ui.inspect" | "ui.render" | "ui.screenshot" | "events.poll" => None, // reads: no gesture to touch
        "ui.set" | "window.new" | "window.list" | "window.focus" | "app.quit" => None, // the window's own settings, no document edit
        "agent.join" | "agent.leave" | "agent.list" => None,                           // membership: no edit
        _ => panic!("new method: say how it reaches this rule: {method}"),
    }
}

/// `a` has a text editor open, then the user (move tool) presses on `b`'s rectangle and holds.
/// With `held` false the user does not press: no gesture is open.
fn gesture_scene(held: bool) -> Headless {
    let mut h = two_agents();
    let text = as_agent(&mut h, "engine.execute", exec("node.create", json!({"type": "text", "x": 200, "y": 300, "text": "Hi"})), "a").unwrap()["id"]
        .clone();
    as_agent(&mut h, "engine.execute", exec("text.edit.begin", json!({"id": text})), "a").unwrap();
    as_user(&mut h, "engine.execute", exec("tool.set", json!({"tool": "move"}))).unwrap();
    if held {
        as_user(&mut h, "ui.pointer", json!({"kind": "down", "x": 5, "y": 5})).unwrap();
    }
    h
}

/// Calibration: without the user's press, the same request is made for `a`. With it, it is
/// refused and nothing is recorded.
#[test]
fn an_agent_cannot_act_in_or_end_a_gesture_of_the_window() {
    for method in METHODS {
        let Some(list) = gesture_through(method) else { continue };
        for request in list {
            let mut free = gesture_scene(false);
            as_agent(&mut free, method, request.clone(), "a").unwrap_or_else(|e| panic!("{method} {request}: refused with no gesture open: {e}"));

            let mut h = gesture_scene(true);
            let steps = undo_steps(&mut h);
            let r = as_agent(&mut h, method, request.clone(), "a");
            assert!(matches!(&r, Err(e) if e.contains("gesture")), "{method} {request}: got {r:?}");
            assert_eq!(undo_steps(&mut h), steps, "{method} {request}: nothing was recorded");
        }
    }
}

/// The gesture `b` holds is not `a`'s either: its move and release are refused too.
#[test]
fn an_agent_cannot_continue_a_gesture_of_another_agent() {
    let hold = |h: &mut Headless| {
        as_agent(h, "engine.execute", exec("tool.set", json!({"tool": "move"})), "b").unwrap();
        as_agent(h, "ui.pointer", json!({"kind": "down", "x": 5, "y": 5}), "b").unwrap();
    };
    // Calibration: with `b` not holding anything, `a`'s move is made.
    let mut free = two_agents();
    as_agent(&mut free, "ui.pointer", json!({"kind": "move", "x": 60, "y": 60}), "a").unwrap();

    let mut h = two_agents();
    hold(&mut h);
    for kind in ["move", "up"] {
        let r = as_agent(&mut h, "ui.pointer", json!({"kind": kind, "x": 60, "y": 60}), "a");
        assert!(matches!(&r, Err(e) if e.contains("gesture")), "{kind}: {r:?}");
    }
    // `b`'s own release goes through.
    as_agent(&mut h, "ui.pointer", json!({"kind": "up", "x": 5, "y": 5}), "b").unwrap();
}

/// The gesture `a` opens with a press is its own: its move and release go through, `b`'s do not,
/// and the user is never refused.
#[test]
fn the_gesture_an_agent_opens_is_its_own() {
    let mut h = two_agents();
    as_agent(&mut h, "engine.execute", exec("shape.rectangle", json!({"x": 100, "y": 100, "width": 40, "height": 40})), "a").unwrap();
    as_agent(&mut h, "engine.execute", exec("tool.set", json!({"tool": "move"})), "a").unwrap();
    as_agent(&mut h, "ui.pointer", json!({"kind": "down", "x": 110, "y": 110}), "a").unwrap();
    // Calibration: the press opened a gesture, so someone else's move is refused.
    let r = as_agent(&mut h, "ui.pointer", json!({"kind": "move", "x": 130, "y": 130}), "b");
    assert!(matches!(&r, Err(e) if e.contains("gesture")), "b's move: {r:?}");
    as_user(&mut h, "ui.pointer", json!({"kind": "hover", "x": 500, "y": 500})).unwrap();
    as_agent(&mut h, "ui.pointer", json!({"kind": "move", "x": 120, "y": 120}), "a").unwrap();
    as_agent(&mut h, "ui.pointer", json!({"kind": "up", "x": 120, "y": 120}), "a").unwrap();
    // Released: `b` may act again.
    as_agent(&mut h, "ui.pointer", json!({"kind": "move", "x": 130, "y": 130}), "b").unwrap();
}

/// When `a` leaves with a text editor open, `b` can edit again. Calibration: while `a` is there
/// with its editor open, `b`'s edit is refused.
#[test]
fn an_agent_that_leaves_holds_nothing() {
    let mut h = two_agents();
    let text = as_agent(&mut h, "engine.execute", exec("node.create", json!({"type": "text", "x": 200, "y": 300, "text": "Hi"})), "a").unwrap()["id"]
        .clone();
    as_agent(&mut h, "engine.execute", exec("text.edit.begin", json!({"id": text})), "a").unwrap();
    let edit = exec("shape.rectangle", json!({"x": 300, "y": 300, "width": 5, "height": 5}));
    let r = as_agent(&mut h, "engine.execute", edit.clone(), "b");
    assert!(matches!(&r, Err(e) if e.contains("gesture")), "a's editor is a gesture of its own: {r:?}");
    as_agent(&mut h, "agent.leave", json!({"agent": "a"}), "a").unwrap();
    as_agent(&mut h, "engine.execute", edit, "b").unwrap_or_else(|e| panic!("b is still held back by a that left: {e}"));
}

// ---------------------------------------------------------------------------------------------
// Rule 5: the document in front is not changed by an agent apart, by any door; and `document`
// acts in the other file while the window stays on its own.
// ---------------------------------------------------------------------------------------------

/// The requests that try to change the document in front through `method`.
pub(crate) fn front_through(method: &str) -> Option<Vec<Value>> {
    match method {
        "engine.execute" => Some(vec![exec("doc.switch", json!({"index": 1})), exec("file.new", json!({})), exec("doc.nextTab", json!({}))]),
        "ui.key" => Some(vec![json!({"key": "Tab", "ctrl": true})]),
        // Headless has no window: a menu line or a click on a tab reaches it only there.
        "ui.input" | "ui.menu.invoke" => None,
        "engine.commands" | "document.inspect" | "node.get" | "ui.inspect" | "ui.render" | "ui.screenshot" | "events.poll" => None, // reads: cannot change it
        "ui.pointer" | "ui.type" => None, // a pointer event and typed text do not switch files
        "ui.set" | "window.new" | "window.list" | "window.focus" | "app.quit" => None, // the window's own affair, not an agent's door
        "agent.join" | "agent.leave" | "agent.list" => None, // membership: no document
        _ => panic!("new method: say how it reaches this rule: {method}"),
    }
}

/// Two files, the first in front. With `with_b`, `a` and `b` have joined, so each works apart.
/// Without it nobody has joined: `a` is a named agent that is alone, and free to switch.
fn two_files(with_b: bool) -> Headless {
    let mut h = Headless::new();
    as_user(&mut h, "engine.execute", exec("file.new", json!({}))).unwrap();
    as_user(&mut h, "engine.execute", exec("doc.switch", json!({"index": 0}))).unwrap();
    if with_b {
        for name in ["a", "b"] {
            as_agent(&mut h, "agent.join", json!({"agent": name}), name).unwrap();
        }
    }
    h
}

fn front(h: &mut Headless) -> Value {
    as_user(h, "document.inspect", json!({})).unwrap()["id"].clone()
}

/// Calibration: with `a` alone each request changes the file in front. With `b` there, none does.
#[test]
fn an_agent_apart_does_not_change_the_document_in_front_by_any_door() {
    for method in METHODS {
        let Some(list) = front_through(method) else { continue };
        for request in list {
            let mut lone = two_files(false);
            let was = front(&mut lone);
            as_agent(&mut lone, method, request.clone(), "a").unwrap_or_else(|e| panic!("{method} {request}: refused with nobody else there: {e}"));
            assert_ne!(front(&mut lone), was, "{method} {request}: a lone agent changes the front");

            let mut h = two_files(true);
            let was = front(&mut h);
            let r = as_agent(&mut h, method, request.clone(), "a");
            assert!(r.is_err(), "{method} {request}: an agent apart changed the front: {r:?}");
            assert_eq!(front(&mut h), was, "{method} {request}: the window keeps its file");
        }
    }
}

/// `document` makes the call act in that file, and the window keeps its own in front.
/// Calibration: without `document` the rectangle lands in the file in front.
#[test]
fn the_document_parameter_acts_in_the_other_file_and_the_front_stays() {
    let rect = exec("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}));
    // Ids of documents belong to the session that opened them: each part reads them in its own.
    let other_of = |h: &mut Headless| -> (Value, Value) {
        let ids: Vec<Value> =
            as_user(h, "engine.execute", exec("doc.list", json!({}))).unwrap().as_array().unwrap().iter().map(|d| d["id"].clone()).collect();
        assert_eq!(ids.len(), 2);
        let shown = front(h);
        let other = ids.iter().find(|i| **i != shown).unwrap().clone();
        (shown, other)
    };

    let mut h = two_files(true);
    as_agent(&mut h, "engine.execute", rect.clone(), "a").unwrap();
    assert_eq!(drawn(&mut h), 1, "without `document` the rectangle is in the front file");

    let mut h = two_files(true);
    let (shown, other) = other_of(&mut h);
    let mut p = rect;
    p["document"] = other.clone();
    as_agent(&mut h, "engine.execute", p, "a").unwrap();
    assert_eq!(front(&mut h), shown, "the window stays on its file");
    assert_eq!(drawn(&mut h), 0, "the front file is untouched");
    let there = as_user(&mut h, "document.inspect", json!({"document": other})).unwrap();
    assert_eq!(there["pages"][0]["children"].as_array().map(Vec::len), Some(1), "the rectangle is in the other file");
}

// ---------------------------------------------------------------------------------------------
// Rule 6: raw input (`ui.input`) is replayed into a frame of the window: two agents' batches do
// not mix there. `Headless` has no frame, so no door of this crate reaches the rule.
// ---------------------------------------------------------------------------------------------

/// The request that sends raw input through `method`: none here.
pub(crate) fn raw_input_through(method: &str) -> Option<Value> {
    match method {
        // Only the window answers `ui.input`; Headless refuses it as an unknown method.
        "ui.input" => None,
        "engine.execute" | "engine.commands" | "document.inspect" | "node.get" | "ui.inspect" | "ui.pointer" | "ui.key" | "ui.type" | "ui.render"
        | "ui.screenshot" => None, // events are made by the session, not queued for a frame
        "ui.set" | "ui.menu.invoke" | "window.new" | "window.list" | "window.focus" | "events.poll" | "app.quit" => None, // no input queue behind them
        "agent.join" | "agent.leave" | "agent.list" => None,                                                              // membership: no input
        _ => panic!("new method: say how it reaches this rule: {method}"),
    }
}

/// `Headless` does not answer raw input, so the rule is the window's alone. Calibration: a known
/// method gets past the same admission.
#[test]
fn headless_has_no_raw_input_to_mix() {
    let mut h = two_agents();
    as_agent(&mut h, "ui.inspect", json!({}), "a").unwrap();
    let r = as_agent(&mut h, "ui.input", json!({"events": []}), "a");
    assert!(matches!(&r, Err(e) if e.contains("unknown method")), "{r:?}");
    for method in METHODS {
        assert!(raw_input_through(method).is_none(), "{method}: Headless queues no raw input");
    }
}

#[test]
fn the_png_an_agent_asks_for_is_admitted_and_placed_like_a_request() {
    let mut s = scene();
    calibrate_scene(&mut s);
    // A name held with another key is refused (rule 1), as `ui.render` is.
    let r = s.h.render_png_as("a", "not-a's-key", &[], 1.0);
    assert!(r.is_err_and(|e| e.contains("held by another client")), "a render under a held name with another key was made");
    // Without ids, the agent apart renders its own page (rule 2), not the window's.
    let own = s.h.render_png_as("a", "key-a", &[], 1.0).unwrap();
    // Calibration: the window's own render of its page is another picture, so the bytes tell the pages apart.
    let window = s.h.render_png(&[], 1.0).unwrap();
    assert!(!own.is_empty() && !window.is_empty(), "both renders made a picture");
    assert_ne!(own, window, "the render is a's page, not the window's");
}
