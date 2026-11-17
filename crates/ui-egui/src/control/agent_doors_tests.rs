//! Every method of the control protocol, against every rule that holds for an agent (`agent` and
//! `agentKey` on the request): admission, its own page and selection, undo, gestures, the document
//! in front and raw input. For each rule one function says how a method reaches it (the request
//! that tries the action through that door) or why it cannot (`None`, with the reason). The
//! `match` has no default arm that passes: a new method fails here until it is classified.
//! `agent_doors_table_tests.rs` checks the classification against the engine's table of doors.

use std::collections::HashMap;

use egui::{Event, Pos2, ViewportId};
use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::control::handle;
use crate::layers_drag_tests::Ui;

/// Every method the window's control channel handles (`handle_method`, and the two requests that
/// are queued for a frame instead).
pub(super) const METHODS: &[&str] = &[
    "engine.execute",
    "engine.commands",
    "document.inspect",
    "node.get",
    "agent.join",
    "agent.leave",
    "agent.list",
    "window.new",
    "window.list",
    "window.focus",
    "ui.inspect",
    "ui.pointer",
    "ui.key",
    "ui.input",
    "ui.type",
    "ui.menu.invoke",
    "ui.set",
    "ui.render",
    "ui.screenshot",
    "events.poll",
    "app.quit",
];

const NEW_METHOD: &str = "new method: say how it reaches this rule";

// ---------------------------------------------------------------------------------------------
// Aids
// ---------------------------------------------------------------------------------------------

fn call(u: &mut Ui, method: &str, params: Value) -> Result<Value, String> {
    let ctx = u.ctx.clone();
    handle(&mut u.app, &ctx, method, &params)
}

/// `p` made for agent `who` (its key is `key <who>`), or the user's own when there is none.
fn as_who(mut p: Value, who: Option<&str>) -> Value {
    if let Some(who) = who {
        p["agent"] = json!(who);
        p["agentKey"] = json!(format!("key {who}"));
    }
    p
}

fn run(u: &mut Ui, who: Option<&str>, command: &str, params: Value) -> Result<Value, String> {
    call(u, "engine.execute", as_who(json!({"command": command, "params": params}), who))
}

fn new_id(v: &Value) -> u64 {
    v["id"].as_u64().unwrap()
}

fn ensure(ok: bool, what: impl Into<String>) -> Result<(), String> {
    if ok { Ok(()) } else { Err(what.into()) }
}

fn refused(r: &Result<Value, String>, word: &str) -> bool {
    r.as_ref().is_err_and(|e| e.contains(word))
}

/// The layers of the page the inspection is in depth on: `currentPage` is the caller's page, the
/// only one the reply details.
fn ids_of(doc: &Value) -> Vec<u64> {
    let current = &doc["currentPage"];
    let page = doc["pages"].as_array().and_then(|pages| pages.iter().find(|p| &p["id"] == current));
    page.and_then(|p| p["children"].as_array()).map_or_else(Vec::new, |a| a.iter().filter_map(|c| c["id"].as_u64()).collect())
}

/// The layers of the page `who` sees (the window's page when `who` is the user).
fn layers(u: &mut Ui, who: Option<&str>) -> Vec<u64> {
    let doc = call(u, "document.inspect", as_who(json!({}), who)).unwrap();
    ids_of(&doc)
}

fn selection(u: &Ui) -> Vec<u64> {
    u.app.session.selection().iter().map(|n| n.0).collect()
}

fn x_of(u: &mut Ui, id: u64) -> f64 {
    call(u, "node.get", json!({"id": id})).unwrap()["x"].as_f64().unwrap()
}

fn front(u: &Ui) -> u64 {
    u.app.session.docs[u.app.session.active].uid
}

/// The batch the control client sent with `ui.input` meets its frame, as the desktop app's input
/// hook hands it over.
fn frame_injected(u: &mut Ui) {
    let events = crate::windows::take_input(&mut u.app, ViewportId::ROOT);
    u.frame(events);
}

/// `a` and `b` are there with their keys, and `b` drew a rectangle: the history holds an edit of
/// `b`'s, which an undo of `a` would take away.
fn two_agents() -> Ui {
    let mut u = Ui::new(Session::new());
    for name in ["a", "b"] {
        call(&mut u, "agent.join", json!({"agent": name, "agentKey": format!("key {name}")})).unwrap();
    }
    run(&mut u, Some("b"), "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    u
}

/// Fails when the scenario cannot show the defect: both agents are there and `b`'s rectangle is
/// the only step of the history.
fn calibrate_two(u: &mut Ui) {
    let list = call(u, "agent.list", json!({})).unwrap();
    assert_eq!(list["agents"].as_array().map(Vec::len), Some(2), "a and b are both present: {list}");
    assert_eq!(layers(u, None).len(), 1, "b's rectangle is in the document");
    assert_eq!(u.app.session.undo_depth(), (1, 0), "b's rectangle is the only step");
}

// ---------------------------------------------------------------------------------------------
// Scenes
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Kind {
    /// Rectangles on the page.
    Rect,
    /// Text layers.
    Text,
    /// Text layers, the text of `a` open in an editor `a` began.
    Editing,
}

/// The layers of the scene: the user's, `a`'s and `b`'s.
pub(super) struct Scene {
    window: u64,
    agent: u64,
    other: u64,
}

/// `a` and `b` work apart. The user has a layer selected on the window's page; `a` is on a page of
/// its own with a layer selected there; `b` drew on the window's page. Nothing `a` does through an
/// `Own` door may touch the window's page or selection.
fn scene(kind: Kind) -> (Ui, Scene) {
    let mut u = Ui::new(Session::new());
    for name in ["a", "b"] {
        call(&mut u, "agent.join", json!({"agent": name, "agentKey": format!("key {name}")})).unwrap();
    }
    let text = kind != Kind::Rect;
    let make = |u: &mut Ui, who: Option<&str>, x: f64| -> u64 {
        let r = if text {
            run(u, who, "text.create", json!({"text": "t", "x": x, "y": 0}))
        } else {
            run(u, who, "shape.rectangle", json!({"x": x, "y": 0, "width": 40, "height": 40}))
        };
        new_id(&r.unwrap())
    };
    let window = make(&mut u, None, 200.0);
    run(&mut u, Some("a"), "page.add", json!({"name": "A page"})).unwrap();
    let agent = make(&mut u, Some("a"), 0.0);
    let other = make(&mut u, Some("b"), 100.0);
    if kind == Kind::Editing {
        run(&mut u, Some("a"), "text.edit.begin", json!({"id": agent})).unwrap();
    }
    // Calibration: the agent's page and selection are not the window's.
    assert_eq!(selection(&u), vec![window], "the window keeps its own selection");
    assert_eq!(layers(&mut u, None), vec![window, other], "the window's page holds the user's layer and b's");
    assert_eq!(layers(&mut u, Some("a")), vec![agent], "a is on a page of its own");
    (u, Scene { window, agent, other })
}

// ---------------------------------------------------------------------------------------------
// Rule 1: admission
// ---------------------------------------------------------------------------------------------

/// A request made under a name another client holds (the key given is not the holder's).
pub(super) fn admission_through(method: &str) -> Option<Value> {
    match method {
        "engine.execute" => Some(json!({"command": "shape.rectangle", "params": {"x": 0, "y": 0, "width": 5, "height": 5}})),
        "engine.commands" => Some(json!({})),
        "document.inspect" => Some(json!({})),
        "node.get" => Some(json!({"id": 1})),
        "window.new" => Some(json!({})),
        "window.list" => Some(json!({})),
        "window.focus" => Some(json!({"id": 1})),
        "ui.inspect" => Some(json!({})),
        "ui.pointer" => Some(json!({"kind": "click", "x": 5, "y": 5})),
        "ui.key" => Some(json!({"key": "Z", "cmd": true})),
        "ui.input" => Some(json!({"events": [{"key": "Z", "cmd": true}]})),
        "ui.type" => Some(json!({"text": "x"})),
        "ui.menu.invoke" => Some(json!({"id": "edit.undo"})),
        "ui.set" => Some(json!({"rightTab": "design"})),
        "ui.render" => Some(json!({})),
        "app.quit" => Some(json!({})),
        // Membership: says who is there, admitted by its own rule (a join gives its key, a leave needs the holder's).
        "agent.join" | "agent.leave" | "agent.list" => None,
        // Queued for a frame, not run by `handle`: the window admits a capture when it takes it (the capture test
        // below). `events.poll` is answered on the transport thread from the shared log and admits nobody.
        "ui.screenshot" | "events.poll" => None,
        _ => panic!("{NEW_METHOD}"),
    }
}

#[test]
fn a_name_held_with_another_key_refuses_every_method_but_membership() {
    for method in METHODS {
        let Some(p) = admission_through(method) else { continue };
        // Calibration: with the holder's key the request is not refused for its name.
        let mut u = two_agents();
        calibrate_two(&mut u);
        let held = call(&mut u, method, as_who(p.clone(), Some("a")));
        assert!(!refused(&held, "held by another client"), "{method}: the holder's own key was refused: {held:?}");

        let mut u = two_agents();
        calibrate_two(&mut u);
        let mut stolen = p;
        stolen["agent"] = json!("a");
        stolen["agentKey"] = json!("not a's");
        let r = call(&mut u, method, stolen);
        assert!(refused(&r, "held by another client"), "{method}: {r:?}");
        assert_eq!(layers(&mut u, None).len(), 1, "{method}: nothing was drawn");
        assert_eq!(u.app.session.undo_depth(), (1, 0), "{method}: the history is untouched");
    }
}

// ---------------------------------------------------------------------------------------------
// Rule 2: the agent's own page and selection
// ---------------------------------------------------------------------------------------------

/// What a probe checks once its request is answered: the window, the scene, and the door's reply.
type ProbeCheck = fn(&mut Ui, &Scene, Result<Value, String>) -> Result<(), String>;

pub(super) struct Probe {
    what: &'static str,
    kind: Kind,
    params: fn(&Scene) -> Value,
    check: ProbeCheck,
}

/// What `a` does through an `Own` door, and what must then be true of `a`'s page and selection.
pub(super) fn emplacement_through(method: &str) -> Option<Vec<Probe>> {
    match method {
        "engine.execute" => Some(vec![Probe {
            what: "a nudge moves a's selection, not the window's",
            kind: Kind::Rect,
            params: |_| json!({"command": "transform.nudge", "params": {"dx": 1, "dy": 0}}),
            check: |u, s, r| {
                r?;
                ensure(x_of(u, s.agent) > 0.0, "a's layer did not move")?;
                ensure(x_of(u, s.window) == 200.0, "the window's layer moved")
            },
        }]),
        "document.inspect" => Some(vec![Probe {
            what: "reads a's page",
            kind: Kind::Rect,
            params: |_| json!({}),
            check: |_, s, r| ensure(ids_of(&r?) == vec![s.agent], "the window's page was read"),
        }]),
        "node.get" => Some(vec![Probe {
            what: "reads a's layer",
            kind: Kind::Rect,
            params: |s| json!({"id": s.agent}),
            check: |_, s, r| ensure(new_id(&r?) == s.agent, "another node was read"),
        }]),
        "ui.pointer" => Some(vec![Probe {
            what: "the reply is a's selection",
            kind: Kind::Rect,
            params: |_| json!({"kind": "click", "x": 20, "y": 20}),
            check: |_, s, r| {
                let r = r?;
                ensure(r["selection"] == json!([s.agent]), format!("the reply is not a's selection: {r}"))
            },
        }]),
        "ui.key" => Some(vec![
            Probe {
                what: "Enter opens the editor of the layer a selected",
                kind: Kind::Text,
                params: |_| json!({"key": "Enter"}),
                check: |u, s, r| {
                    r?;
                    // The editor is a's own: out of a's call, `text_edit` is the window's.
                    let editing = u.app.session.with_agent(Some("a"), |s| s.ds().ok().and_then(|d| d.text_edit.as_ref().map(|t| t.id.0)));
                    ensure(editing == Some(s.agent), format!("the editor opened on {editing:?}, not on a's layer"))
                },
            },
            Probe {
                what: "an arrow moves a's selection",
                kind: Kind::Rect,
                params: |_| json!({"key": "ArrowRight"}),
                check: |u, s, r| {
                    r?;
                    ensure(x_of(u, s.agent) > 0.0, "a's layer did not move")?;
                    ensure(x_of(u, s.window) == 200.0, "the window's layer moved")
                },
            },
            Probe {
                what: "Delete removes a's selection",
                kind: Kind::Rect,
                params: |_| json!({"key": "Delete"}),
                check: |u, s, r| {
                    r?;
                    ensure(layers(u, Some("a")).is_empty(), "a's layer is still there")?;
                    ensure(layers(u, None).contains(&s.window), "the window's layer went")
                },
            },
        ]),
        "ui.type" => Some(vec![Probe {
            what: "writes in the editor a opened, and is not refused",
            kind: Kind::Editing,
            params: |_| json!({"text": "X"}),
            check: |u, s, r| {
                r?;
                let node = call(u, "node.get", json!({"id": s.agent}))?;
                ensure(node.to_string().contains('X'), format!("the text is not in a's layer: {node}"))
            },
        }]),
        "ui.menu.invoke" => Some(vec![Probe {
            what: "a menu command clears a's selection, not the window's",
            kind: Kind::Rect,
            params: |_| json!({"id": "select.none"}),
            check: |u, s, r| {
                r?;
                ensure(selection(u) == vec![s.window], "the window's selection was cleared")
            },
        }]),
        "ui.render" => Some(vec![Probe {
            what: "renders a's page (the reply carries no page: only that it is made)",
            kind: Kind::Rect,
            params: |_| json!({}),
            check: |_, _, r| r.map(|_| ()),
        }]),
        // Not an `Own` door: runs for the window (settings, windows, captures, raw input) or says who is there.
        "engine.commands" | "ui.inspect" | "ui.set" | "ui.input" | "window.new" | "window.list" | "window.focus" | "app.quit" => None,
        "agent.join" | "agent.leave" | "agent.list" => None,
        "ui.screenshot" | "events.poll" => None,
        _ => panic!("{NEW_METHOD}"),
    }
}

#[test]
fn an_agent_acts_and_reads_on_its_own_page_and_selection() {
    for method in METHODS {
        for probe in emplacement_through(method).unwrap_or_default() {
            let (mut u, s) = scene(probe.kind);
            let p = as_who((probe.params)(&s), Some("a"));
            let r = call(&mut u, method, p);
            if let Err(e) = (probe.check)(&mut u, &s, r) {
                panic!("{method}: {}: {e}", probe.what);
            }
            assert_eq!(selection(&u), vec![s.window], "{method}: {}: the window keeps its selection", probe.what);
            assert!(layers(&mut u, None).contains(&s.other), "{method}: {}: the window's page is the window's", probe.what);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Rule 3: undo and redo
// ---------------------------------------------------------------------------------------------

/// How a request of the rules reaches the interface.
pub(super) enum Via {
    /// A request of the method itself.
    Call(Value),
    /// A request of `ui.input` carrying keys, applied by the frame that follows.
    Keys(Value),
    /// A real click on each row of the main menu (AccessKit finds the row), through `ui.input`.
    /// The menu is open, as a click on its button leaves it.
    Menu(Vec<&'static str>),
    /// A real click on what carries each label (a tab), through `ui.input`.
    Mouse(Vec<&'static str>),
    /// A request built with the uid of the file behind the tab in front.
    With(fn(u64) -> Value),
}

/// Rows of the Edit menu, found by AccessKit as the question's "Allow" button is.
#[derive(Default)]
struct Seen(HashMap<egui::accesskit::NodeId, (String, egui::accesskit::Rect)>);

fn frame_seen(u: &mut Ui, seen: &mut Seen, events: Vec<Event>) {
    let out = u.frame_output(events);
    let Some(update) = out.platform_output.accesskit_update else { return };
    for (id, node) in &update.nodes {
        if let (Some(label), Some(bounds)) = (node.label(), node.bounds()) {
            seen.0.insert(*id, (label.to_string(), bounds));
        }
    }
}

/// The topmost node labelled `label`.
fn centre_of(seen: &Seen, label: &str) -> Option<Pos2> {
    let mut found: Vec<_> = seen.0.values().filter(|(l, _)| l == label).map(|(_, b)| b).collect();
    found.sort_by(|p, q| p.y0.total_cmp(&q.y0).then(p.x0.total_cmp(&q.x0)));
    found.first().map(|b| Pos2::new(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32))
}

/// A press or release at `at`: the user's own, or injected by `who` through `ui.input`.
fn press(u: &mut Ui, seen: &mut Seen, who: Option<&str>, at: Pos2, pressed: bool) {
    let button = egui::PointerButton::Primary;
    let Some(who) = who else {
        let events = vec![Event::PointerMoved(at), Event::PointerButton { pos: at, button, pressed, modifiers: Default::default() }];
        return frame_seen(u, seen, events);
    };
    let key = if pressed { "down" } else { "up" };
    // A refusal at the request is fine: the batch is then empty and the frame applies nothing.
    let mut event = json!({});
    event[key] = json!([at.x, at.y]);
    let _ = call(u, "ui.input", as_who(json!({"events": [event]}), Some(who)));
    let events = crate::windows::take_input(&mut u.app, ViewportId::ROOT);
    frame_seen(u, seen, events);
}

/// A click on each label in turn: the menu opens, then its row is clicked.
fn click_path(u: &mut Ui, who: Option<&str>, labels: &[&str]) {
    u.ctx.enable_accesskit();
    let mut seen = Seen::default();
    for label in labels {
        let mut at = None;
        for _ in 0..30 {
            frame_seen(u, &mut seen, vec![]);
            at = centre_of(&seen, label);
            if at.is_some() {
                break;
            }
        }
        let at = at.unwrap_or_else(|| panic!("no `{label}` on screen: the click cannot reach it"));
        for pressed in [true, false] {
            press(u, &mut seen, who, at, pressed);
        }
        for _ in 0..3 {
            frame_seen(u, &mut seen, vec![]);
        }
    }
}

fn attempt(u: &mut Ui, method: &str, via: &Via, who: Option<&str>, second: u64) -> Result<Value, String> {
    match via {
        Via::Call(p) => call(u, method, as_who(p.clone(), who)),
        Via::Keys(p) => {
            let r = call(u, method, as_who(p.clone(), who));
            frame_injected(u);
            r
        }
        Via::Menu(rows) => {
            u.app.ui.main_menu = true;
            click_path(u, who, rows);
            Ok(json!({}))
        }
        Via::Mouse(labels) => {
            click_path(u, who, labels);
            Ok(json!({}))
        }
        Via::With(f) => call(u, method, as_who(f(second), who)),
    }
}

/// The undo (or the redo) an agent tries through the door.
pub(super) fn undo_through(method: &str, redo: bool) -> Option<Vec<Via>> {
    let (command, key, label) = if redo {
        ("edit.redo", json!({"key": "Z", "cmd": true, "shift": true}), "Redo")
    } else {
        ("edit.undo", json!({"key": "Z", "cmd": true}), "Undo")
    };
    match method {
        "engine.execute" => Some(vec![
            Via::Call(json!({"command": command, "params": {}})),
            Via::Call(json!({"command": "command.batch", "params": {"commands": [{"command": command, "params": {}}]}})),
        ]),
        "ui.key" => Some(vec![Via::Call(key.clone())]),
        "ui.menu.invoke" => Some(vec![Via::Call(json!({"id": command}))]),
        "ui.input" => Some(vec![Via::Keys(json!({"events": [key]})), Via::Menu(vec!["Edit", label])]),
        // Reads: cannot reach undo.
        "document.inspect" | "node.get" | "ui.render" | "engine.commands" | "ui.inspect" | "window.list" => None,
        // Settings, windows and the app: no history step is made or taken back.
        "ui.set" | "window.new" | "window.focus" | "app.quit" => None,
        // The pointer and typing edit through the tools and the text editor: neither runs a command by id.
        "ui.pointer" | "ui.type" => None,
        "agent.join" | "agent.leave" | "agent.list" => None,
        "ui.screenshot" | "events.poll" => None,
        _ => panic!("{NEW_METHOD}"),
    }
}

/// With `b` there and its rectangle drawn: the history to leave alone (`redo`: after the user's
/// undo, so that the rectangle waits in the redo stack).
fn undo_scene(redo: bool) -> Ui {
    let mut u = two_agents();
    if redo {
        call(&mut u, "ui.key", json!({"key": "Z", "cmd": true})).unwrap();
        assert_eq!(layers(&mut u, None).len(), 0, "the user's undo took b's rectangle away");
    }
    let depth = if redo { (0, 1) } else { (1, 0) };
    assert_eq!(u.app.session.undo_depth(), depth, "the history is as the scenario says");
    u
}

#[test]
fn an_agents_undo_and_redo_are_refused_by_every_door() {
    for redo in [false, true] {
        for method in METHODS {
            for (n, via) in undo_through(method, redo).unwrap_or_default().iter().enumerate() {
                let what = format!("{method} #{n} ({})", if redo { "redo" } else { "undo" });
                let start = if redo { (0, 1) } else { (1, 0) };
                // What the step changes: the history and the document. A batch is a step of its own, so an
                // undo inside one takes a step and adds one back (the depth stays), but the document moves.
                let seen = |u: &mut Ui| (u.app.session.undo_depth(), layers(u, None).len());
                // Calibration: the user takes the same step through the same door.
                let mut u = undo_scene(redo);
                let before = seen(&mut u);
                attempt(&mut u, method, via, None, 0).unwrap_or_else(|e| panic!("{what}: the user's own was refused: {e}"));
                assert_ne!(seen(&mut u), before, "{what}: the user's own step did not reach the history: the scenario cannot show the defect");
                // The agent's.
                let mut u = undo_scene(redo);
                let r = attempt(&mut u, method, via, Some("a"), 0);
                if matches!(via, Via::Call(_)) {
                    assert!(refused(&r, "shared by everyone"), "{what}: a's step was not refused: {r:?}");
                } else if r.is_err() {
                    assert!(refused(&r, "shared by everyone"), "{what}: {r:?}");
                }
                assert_eq!(u.app.session.undo_depth(), start, "{what}: the history is intact");
                assert_eq!(layers(&mut u, None).len(), usize::from(!redo), "{what}: b's rectangle is as it was");
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Rule 4: gestures
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub(super) enum Holder {
    /// The window holds an interaction open (a drag of the user's).
    Window,
    /// `b` pressed the pointer on its layer.
    BTool,
    /// `b` has a text editor open.
    BText,
}

pub(super) struct Attempt {
    what: &'static str,
    holder: Holder,
    params: Value,
}

/// What `a` tries while a gesture that is not its own is open.
pub(super) fn gesture_through(method: &str) -> Option<Vec<Attempt>> {
    let at = |what, holder, params| Attempt { what, holder, params };
    let edit = json!({"command": "shape.rectangle", "params": {"x": 300, "y": 300, "width": 5, "height": 5}});
    match method {
        "engine.execute" => Some(vec![
            at("an edit during the window's gesture", Holder::Window, edit.clone()),
            at("an edit during b's pointer gesture", Holder::BTool, edit.clone()),
            at("an edit during b's text editing", Holder::BText, edit),
        ]),
        "ui.pointer" => Some(
            [Holder::Window, Holder::BTool]
                .into_iter()
                .flat_map(|h| {
                    [
                        at("a move", h, json!({"kind": "move", "x": 30, "y": 30})),
                        at("a release", h, json!({"kind": "up", "x": 30, "y": 30})),
                        at("a click", h, json!({"kind": "click", "x": 20, "y": 20})),
                    ]
                })
                .collect(),
        ),
        "ui.key" => Some(
            [Holder::Window, Holder::BTool]
                .into_iter()
                .flat_map(|h| [at("Escape", h, json!({"key": "Escape"})), at("a tool key", h, json!({"key": "R"}))])
                .collect(),
        ),
        "ui.type" => Some(vec![at("typing in b's editor", Holder::BText, json!({"text": "Z"}))]),
        // Reads, settings, windows and raw input (the frame that applies it runs under the engine's rules): no gesture to join or end.
        "document.inspect" | "node.get" | "ui.render" | "engine.commands" | "ui.inspect" | "window.list" => None,
        "ui.set" | "ui.input" | "ui.menu.invoke" | "window.new" | "window.focus" | "app.quit" => None,
        "agent.join" | "agent.leave" | "agent.list" => None,
        "ui.screenshot" | "events.poll" => None,
        _ => panic!("{NEW_METHOD}"),
    }
}

/// `a` and `b` apart over a page with a layer each (the user's at 200, `a`'s at 0, `b`'s at 100)
/// and a text of `b`'s; the gesture of `holder` is open.
fn gesture_scene(holder: Option<Holder>) -> (Ui, u64) {
    let mut u = Ui::new(Session::new());
    for name in ["a", "b"] {
        call(&mut u, "agent.join", json!({"agent": name, "agentKey": format!("key {name}")})).unwrap();
    }
    let rect = |x: f64| json!({"x": x, "y": 0, "width": 40, "height": 40});
    run(&mut u, None, "shape.rectangle", rect(200.0)).unwrap();
    run(&mut u, Some("a"), "shape.rectangle", rect(0.0)).unwrap();
    run(&mut u, Some("b"), "shape.rectangle", rect(100.0)).unwrap();
    let text = run(&mut u, Some("b"), "text.create", json!({"text": "t", "x": 300, "y": 100})).unwrap();
    run(&mut u, None, "tool.move", json!({})).unwrap();
    let tb = new_id(&text);
    match holder {
        Some(Holder::Window) => u.app.session.begin_interaction("Move").unwrap(),
        Some(Holder::BTool) => {
            call(&mut u, "ui.pointer", as_who(json!({"kind": "down", "x": 120, "y": 20}), Some("b"))).unwrap();
        }
        Some(Holder::BText) => {
            run(&mut u, Some("b"), "text.edit.begin", json!({"id": tb})).unwrap();
        }
        None => {}
    }
    (u, tb)
}

/// Whether `holder`'s gesture is still open, and still `b`'s to go on with (`b` is not refused).
fn gesture_goes_on(u: &mut Ui, holder: Holder) -> Result<(), String> {
    match holder {
        Holder::Window => ensure(u.app.session.ds().is_ok_and(|d| d.interaction.is_some()), "the window's interaction closed"),
        Holder::BTool => {
            call(u, "ui.pointer", as_who(json!({"kind": "move", "x": 121, "y": 20}), Some("b")))?;
            Ok(())
        }
        Holder::BText => {
            // The editor is b's own: out of b's call, `text_edit` is the window's.
            let open = u.app.session.with_agent(Some("b"), |s| s.ds().is_ok_and(|d| d.text_edit.is_some()));
            ensure(open, "b's editor closed")?;
            call(u, "ui.type", as_who(json!({"text": "q"}), Some("b")))?;
            Ok(())
        }
    }
}

#[test]
fn a_cannot_act_in_or_end_a_gesture_that_is_not_its_own() {
    for method in METHODS {
        for t in gesture_through(method).unwrap_or_default() {
            let what = format!("{method}: {} ({:?})", t.what, t.holder);
            let (mut u, _) = gesture_scene(Some(t.holder));
            // Calibration: the gesture is open and its holder is not refused in it.
            gesture_goes_on(&mut u, t.holder).unwrap_or_else(|e| panic!("{what}: the scenario cannot show the defect: {e}"));
            let tool = u.app.session.tool.tool.id();
            let r = call(&mut u, method, as_who(t.params.clone(), Some("a")));
            assert!(refused(&r, "gesture"), "{what}: a was not refused: {r:?}");
            assert_eq!(u.app.session.tool.tool.id(), tool, "{what}: the tool is as it was");
            gesture_goes_on(&mut u, t.holder).unwrap_or_else(|e| panic!("{what}: a's attempt broke the gesture: {e}"));
            if let Holder::BTool = t.holder {
                let up = call(&mut u, "ui.pointer", as_who(json!({"kind": "up", "x": 121, "y": 20}), Some("b")));
                assert!(up.is_ok(), "{what}: b ends its own gesture: {up:?}");
            }
        }
    }
}

#[test]
fn a_command_of_the_user_is_never_refused_by_a_gesture() {
    for holder in [Holder::Window, Holder::BTool, Holder::BText] {
        let (mut u, _) = gesture_scene(Some(holder));
        gesture_goes_on(&mut u, holder).unwrap_or_else(|e| panic!("{holder:?}: the scenario cannot show the defect: {e}"));
        let r = run(&mut u, None, "shape.rectangle", json!({"x": 300, "y": 300, "width": 5, "height": 5}));
        assert!(r.is_ok(), "{holder:?}: the user's command was refused: {r:?}");
    }
}

#[test]
fn the_gesture_a_opens_with_a_pointer_press_is_its_own() {
    let (mut u, _) = gesture_scene(None);
    let pointer = |u: &mut Ui, who: &str, kind: &str| call(u, "ui.pointer", as_who(json!({"kind": kind, "x": 20, "y": 20}), Some(who)));
    pointer(&mut u, "a", "down").unwrap();
    // Calibration: b is refused only because the press is a's (b's own press, on its own, is not).
    let b = pointer(&mut u, "b", "move");
    assert!(refused(&b, "gesture"), "b went on with a's gesture: {b:?}");
    let b = pointer(&mut u, "b", "up");
    assert!(refused(&b, "gesture"), "b ended a's gesture: {b:?}");
    assert!(pointer(&mut u, "a", "move").is_ok(), "a goes on with its own gesture");
    assert!(pointer(&mut u, "a", "up").is_ok(), "a ends its own gesture");
    let (mut v, _) = gesture_scene(None);
    pointer(&mut v, "b", "down").unwrap();
    assert!(pointer(&mut v, "b", "move").is_ok(), "calibration: the holder of a press is not refused in it");
}

#[test]
fn after_a_leaves_with_an_editor_open_b_can_edit_again() {
    let (mut u, _) = gesture_scene(None);
    let ta = new_id(&run(&mut u, Some("a"), "text.create", json!({"text": "t", "x": 400, "y": 100})).unwrap());
    run(&mut u, Some("a"), "text.edit.begin", json!({"id": ta})).unwrap();
    let edit = json!({"x": 300, "y": 300, "width": 5, "height": 5});
    // Calibration: while a's editor is open, b's edit is refused.
    let r = run(&mut u, Some("b"), "shape.rectangle", edit.clone());
    assert!(refused(&r, "gesture"), "a's editor does not hold b off: {r:?}");
    call(&mut u, "agent.leave", json!({"agent": "a", "agentKey": "key a"})).unwrap();
    let r = run(&mut u, Some("b"), "shape.rectangle", edit);
    assert!(r.is_ok(), "an agent that left still holds b off: {r:?}");
}

// ---------------------------------------------------------------------------------------------
// Rule 5: the document in front
// ---------------------------------------------------------------------------------------------

/// What an agent apart tries to change the document in front.
pub(super) fn document_front_through(method: &str) -> Option<Vec<Via>> {
    match method {
        "engine.execute" => Some(vec![
            Via::Call(json!({"command": "doc.switch", "params": {"index": 1}})),
            Via::Call(json!({"command": "file.new", "params": {}})),
            Via::Call(json!({"command": "file.open", "params": {"path": "/nonexistent/other.figcraft"}})),
            // Asked for with the command, the window and its new file are opened by a later frame:
            // as the agent that asked, not as nobody.
            Via::Call(json!({"command": "window.new", "params": {}})),
            // The parameter `document` acts in the other file: the window stays on its own.
            Via::With(|uid| json!({"command": "shape.rectangle", "params": {"x": 0, "y": 0, "width": 9, "height": 9}, "document": uid})),
        ]),
        "document.inspect" | "node.get" | "ui.render" => Some(vec![Via::With(|uid| json!({"document": uid, "id": 1}))]),
        "ui.key" => Some(vec![Via::Call(json!({"key": "Tab", "cmd": true}))]),
        "ui.menu.invoke" => Some(vec![Via::Call(json!({"id": "doc.nextTab"}))]),
        "ui.input" => Some(vec![Via::Keys(json!({"events": [{"key": "Tab", "cmd": true}]})), Via::Mouse(vec!["Second"])]),
        // The pointer and typing act inside the document; they have no way to name another one.
        "ui.pointer" | "ui.type" => None,
        // Settings, windows and the app: they do not change which tab is in front for the agent.
        "engine.commands" | "ui.inspect" | "ui.set" | "window.new" | "window.list" | "window.focus" | "app.quit" => None,
        "agent.join" | "agent.leave" | "agent.list" => None,
        "ui.screenshot" | "events.poll" => None,
        _ => panic!("{NEW_METHOD}"),
    }
}

/// Two files open, the first in front, `a` and `b` there.
fn two_documents() -> (Ui, u64, u64) {
    let mut u = Ui::new(Session::new());
    for name in ["a", "b"] {
        call(&mut u, "agent.join", json!({"agent": name, "agentKey": format!("key {name}")})).unwrap();
    }
    run(&mut u, None, "file.new", json!({"name": "Second"})).unwrap();
    run(&mut u, None, "doc.switch", json!({"index": 0})).unwrap();
    u.frame(vec![]);
    let (first, second) = (u.app.session.docs[0].uid, u.app.session.docs[1].uid);
    assert_eq!(u.app.session.docs.len(), 2, "two files are open");
    assert_eq!(front(&u), first, "the first file is in front");
    (u, first, second)
}

#[test]
fn an_agent_apart_changes_the_document_in_front_by_no_door() {
    for method in METHODS {
        for (n, via) in document_front_through(method).unwrap_or_default().iter().enumerate() {
            let what = format!("{method} #{n}");
            // Calibration: the user changes the file in front by the same door (the doors that
            // name no switch are the `document` ones, which never do).
            // A file that does not exist cannot be opened by anyone: it is no calibration either.
            let opens_nothing = matches!(via, Via::Call(p) if p["command"] == "file.open");
            let switches = !matches!(via, Via::With(_)) && !opens_nothing;
            if switches {
                let (mut u, first, second) = two_documents();
                let _ = attempt(&mut u, method, via, None, second);
                u.frame(vec![]);
                assert_ne!(front(&u), first, "{what}: the user's own does not change the file in front: the scenario cannot show the defect");
            }
            let (mut u, first, second) = two_documents();
            let r = attempt(&mut u, method, via, Some("a"), second);
            u.frame(vec![]);
            assert_eq!(front(&u), first, "{what}: the file in front changed ({r:?})");
            assert_eq!(u.app.session.docs.len(), 2, "{what}: no tab was added");
            if let (Via::With(_), "engine.execute") = (via, *method) {
                assert!(r.is_ok(), "{what}: the edit in the other file was refused: {r:?}");
                let there = call(&mut u, "document.inspect", json!({"document": second})).unwrap();
                assert_eq!(ids_of(&there).len(), 1, "{what}: the edit landed in the other file");
                assert!(layers(&mut u, None).is_empty(), "{what}: the window's file was edited");
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Rule 6: raw input
// ---------------------------------------------------------------------------------------------

/// A batch of raw events for a frame.
pub(super) fn raw_input_through(method: &str) -> Option<Value> {
    match method {
        "ui.input" => Some(json!({"events": [{"move": [10.0, 10.0]}]})),
        // Not raw input: the request runs by itself (or is queued for a capture, which is not a batch of events).
        "engine.execute" | "engine.commands" | "document.inspect" | "node.get" | "ui.pointer" | "ui.key" | "ui.type" | "ui.menu.invoke"
        | "ui.render" => None,
        "ui.inspect" | "ui.set" | "window.new" | "window.list" | "window.focus" | "app.quit" => None,
        "agent.join" | "agent.leave" | "agent.list" => None,
        "ui.screenshot" | "events.poll" => None,
        _ => panic!("{NEW_METHOD}"),
    }
}

#[test]
fn two_agents_do_not_mix_their_raw_input() {
    for method in METHODS {
        let Some(batch) = raw_input_through(method) else { continue };
        let mut u = two_agents();
        calibrate_two(&mut u);
        call(&mut u, method, as_who(batch.clone(), Some("a"))).unwrap();
        let waiting = u.app.control_input.len();
        assert!(waiting > 0, "a's batch waits for its frame");
        // Calibration: the same agent adds to its own batch.
        call(&mut u, method, as_who(batch.clone(), Some("a"))).unwrap();
        let waiting = u.app.control_input.len();
        assert!(waiting > 0, "a's second batch waits too");
        let r = call(&mut u, method, as_who(batch.clone(), Some("b")));
        assert!(refused(&r, "waiting for its frame"), "{method}: b's batch was mixed with a's: {r:?}");
        assert_eq!(u.app.control_input.len(), waiting, "{method}: a's batch is as it was");
        // Once the frame has taken a's batch, b is let in.
        frame_injected(&mut u);
        let r = call(&mut u, method, as_who(batch, Some("b")));
        assert!(r.is_ok(), "{method}: b is refused with nothing waiting: {r:?}");
    }
}

// ---------------------------------------------------------------------------------------------
// Rule 1 for the request `handle` never sees: a capture is admitted when its window takes it
// ---------------------------------------------------------------------------------------------

/// Queue a `ui.screenshot` request as `who` and let the window's frame route it.
fn queue_capture(u: &mut Ui, who: Value) -> std::sync::mpsc::Receiver<crate::control::ControlReply> {
    let (tx, rx) = std::sync::mpsc::channel();
    let req = crate::control::ControlRequest { id: json!(1), method: "ui.screenshot".into(), params: who, reply: tx, state: Default::default() };
    u.app.control_queue.push(req);
    u.frame(vec![]);
    rx
}

#[test]
fn a_capture_is_admitted_for_its_name_by_the_window_that_takes_it() {
    // Calibration: with the holder's key the capture is admitted and waits for its readback.
    let mut u = two_agents();
    calibrate_two(&mut u);
    let held = queue_capture(&mut u, json!({"agent": "a", "agentKey": "key a"}));
    assert_eq!(u.app.capture.shots.len(), 1, "the holder's own capture was not admitted");
    if let Ok(reply) = held.try_recv() {
        assert!(!refused(&reply.result, "held by another client"), "the holder's own key was refused: {:?}", reply.result);
    }

    // A name held with another key is refused before anything is reserved for the capture.
    let mut u = two_agents();
    calibrate_two(&mut u);
    let stolen = queue_capture(&mut u, json!({"agent": "a", "agentKey": "not a's"}));
    assert!(u.app.capture.shots.is_empty(), "a capture under a name held by another key was admitted");
    let reply = stolen.try_recv().expect("the refusal is answered at once");
    assert!(refused(&reply.result, "held by another client"), "{:?}", reply.result);
}

/// A request of another client, handled in the frame that applies `a`'s batch, is that client's:
/// its comment is signed by `b`, or by nobody when it gave no name, never by `a`.
#[test]
fn a_request_drained_in_another_agents_frame_runs_as_its_own_client() {
    let mut u = two_agents();
    let page = u.app.session.page().unwrap();
    call(&mut u, "ui.input", as_who(json!({"events": [{"move": [5.0, 5.0]}]}), Some("a"))).unwrap();
    let events = crate::windows::take_input(&mut u.app, ViewportId::ROOT);
    assert!(!events.is_empty(), "a's batch is in the frame, which then acts as a");
    assert_eq!(u.app.applied_agent.as_ref().map(|a| a.agent()), Some("a"));
    let comment = |text: &str| json!({"command": "comment.add", "params": {"x": 1, "y": 1, "text": text, "page": page}});
    let mut replies = Vec::new();
    for (text, who) in [("from b", Some("b")), ("from nobody", None)] {
        let (tx, rx) = std::sync::mpsc::channel();
        let params = as_who(comment(text), who);
        u.app.control_queue.push(crate::control::ControlRequest {
            id: json!(1),
            method: "engine.execute".into(),
            params,
            reply: tx,
            state: Default::default(),
        });
        replies.push(rx);
    }
    let input = egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, u.screen)), ..Default::default() };
    let ctx = u.ctx.clone();
    let app = &mut u.app;
    ctx.run_ui(input, |ui| crate::windows::guard(app, &ctx, |app| app.ui(ui))).textures_delta.clear();
    for rx in replies {
        let reply = rx.try_recv().expect("the request is answered in that frame");
        assert!(reply.result.is_ok(), "{:?}", reply.result);
    }
    let doc = u.app.session.doc().unwrap();
    let first = |text: &str| doc.comments.iter().map(|c| &c.first).find(|f| f.text == text).map(|f| (f.author.clone(), f.agent));
    assert_eq!(first("from b"), Some(("b (agent)".to_string(), true)), "b's request is signed by b");
    assert!(first("from nobody").is_some_and(|(_, agent)| !agent), "a request without a name is not a's");
    assert!(u.app.session.agent.is_none(), "the frame's identity does not outlive it");
}

/// `a`'s batch is taken by its window (so the window is the one replaying input), and the frame has
/// not run yet: what runs now, outside that frame, is not a's.
fn a_batch_taken() -> (Ui, Vec<Event>) {
    let mut u = two_agents();
    calibrate_two(&mut u);
    call(&mut u, "ui.input", as_who(json!({"events": [{"move": [5.0, 5.0]}]}), Some("a"))).unwrap();
    let events = crate::windows::take_input(&mut u.app, ViewportId::ROOT);
    assert!(!events.is_empty(), "a's batch is taken");
    assert_eq!(u.app.applied_frame, Some(ViewportId::ROOT));
    (u, events)
}

/// The background work runs under the window's guard while `applied_frame` names that window: it is
/// not the agent, so a rule that refuses an agent leaves it alone.
#[test]
fn the_background_work_is_not_the_agent_whose_batch_waits_for_its_frame() {
    let (mut u, _events) = a_batch_taken();
    // Calibration: the same undo by `a` is refused while `b` is there.
    let r = run(&mut u, Some("a"), "edit.undo", json!({}));
    assert!(r.is_err(), "a's undo is refused in this scene: {r:?}");
    assert_eq!(u.app.session.undo_depth(), (1, 0), "b's rectangle is still the only step");
    let ctx = u.ctx.clone();
    let mut done = None;
    crate::windows::guard(&mut u.app, &ctx, |app| done = Some(app.session.execute("edit.undo", &json!({}))));
    let done = done.expect("the guard ran the work");
    assert!(done.is_ok(), "the background work was taken for a: {done:?}");
    assert_eq!(u.app.session.undo_depth(), (0, 1), "the undo happened");
}

/// A request handled outside the frame (a nested guard) acts for its own client, and nothing of it
/// outlives the frame of `a`: the user's next command is the user's.
#[test]
fn an_identity_does_not_leak_from_a_request_handled_outside_the_frame() {
    let (mut u, events) = a_batch_taken();
    let ctx = u.ctx.clone();
    let page = u.app.session.page().unwrap();
    let params = as_who(json!({"command": "comment.add", "params": {"x": 1, "y": 1, "text": "from b", "page": page}}), Some("b"));
    crate::windows::guard(&mut u.app, &ctx, |app| {
        crate::windows::guard(app, &ctx, |app| {
            let _ = handle(app, &ctx, "engine.execute", &params);
        });
    });
    assert!(u.app.session.agent.is_none(), "no identity is left set by the guards");
    let doc = u.app.session.doc().unwrap();
    let author = doc.comments.iter().map(|c| &c.first).find(|f| f.text == "from b").map(|f| (f.author.clone(), f.agent));
    assert_eq!(author, Some(("b (agent)".to_string(), true)), "b's request is signed by b");
    // The frame of a runs and ends; then the user's own command is the user's.
    let input = egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, u.screen)), ..Default::default() };
    let app = &mut u.app;
    ctx.run_ui(input, |ui| crate::windows::guard(app, &ctx, |app| app.ui(ui))).textures_delta.clear();
    assert!(u.app.session.agent.is_none(), "the frame's identity does not outlive it");
    let r = u.app.session.execute("edit.undo", &json!({}));
    assert!(r.is_ok(), "a command of the user was taken for an agent's: {r:?}");
}
