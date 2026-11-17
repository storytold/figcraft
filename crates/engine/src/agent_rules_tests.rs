//! The rules for AI agents hold on the session itself, whichever way a call reaches it: an agent
//! working apart acts on its own page and selection and the window keeps its own; a gesture
//! belongs to whoever began it; the document in front stays in front. Each rule is tried through
//! the calls an interface makes (`with_agent` around a pointer, a key, the interaction API) and
//! through every command of the registry.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use figcraft_doc::NodeId;
use serde_json::{Value, json};

use crate::agent::Door;
use crate::file_access::{Roots, confined};
use crate::tools::{PointerEvent, PointerKind};
use crate::{EngineError, Session};

fn id(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn rect_at(s: &mut Session, agent: Option<&str>, x: f64, y: f64) -> NodeId {
    id(&s.with_agent(agent, |s| s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": 40, "height": 40}))).unwrap())
}

fn ev(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
    PointerEvent::at(kind, x, y, 1.0)
}

/// One pointer event sent for `agent` (the user for `None`).
fn pointer_as(s: &mut Session, agent: Option<&str>, kind: PointerKind, x: f64, y: f64) -> crate::Result<()> {
    s.with_agent(agent, |s| s.pointer(&ev(kind, x, y)))
}

/// `a` and `b` joined and working apart; the window is on the first page with its rectangle (at
/// 0, 0) selected, `b` is on the second page. Returns the session, the two pages and the rectangle.
fn a_and_b_apart() -> (Session, NodeId, NodeId, NodeId) {
    let mut s = Session::new();
    let first = s.page().unwrap();
    s.execute("page.add", &json!({})).unwrap();
    let second = s.page().unwrap();
    assert_ne!(first, second);
    s.execute("page.select", &json!({"id": first})).unwrap();
    let r = rect_at(&mut s, None, 0.0, 0.0);
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    s.agent_join("a").unwrap();
    s.agent_join("b").unwrap();
    s.with_agent(Some("b"), |s| s.execute("page.select", &json!({"id": second}))).unwrap();
    // Calibration: both agents work apart, so the window's page and selection are the user's.
    let list = s.agent_list();
    assert_eq!((list["agents"][0]["apart"].clone(), list["agents"][1]["apart"].clone()), (json!(true), json!(true)));
    assert_eq!((s.ds().unwrap().page, s.ds().unwrap().selection.clone()), (first, vec![r]));
    (s, first, second, r)
}

/// Whether `r` is the refusal of an edit or gesture that is not the caller's.
fn refused_gesture<T: std::fmt::Debug>(r: &crate::Result<T>) -> bool {
    matches!(r, Err(EngineError::Disabled(_, why)) if why.contains("gesture"))
}

/// The window drags the rectangle: its button is down and it has moved twice.
fn window_drag(s: &mut Session) {
    pointer_as(s, None, PointerKind::Down, 20.0, 20.0).unwrap();
    pointer_as(s, None, PointerKind::Move, 30.0, 30.0).unwrap();
    pointer_as(s, None, PointerKind::Move, 40.0, 40.0).unwrap();
    assert!(s.tool.busy(), "calibration: the window's drag is under way");
}

/// Move `node` to `x` in the open interaction.
fn move_preview(s: &mut Session, node: NodeId, x: f64) -> crate::Result<()> {
    let props = serde_json::Map::from_iter([("x".to_string(), json!(x))]);
    s.preview(|d, _| crate::cmd::props::apply("node.set", d, node, &props))
}

fn interaction_open(s: &Session) -> bool {
    s.ds().unwrap().interaction.is_some()
}

// ---- Placement: the agent's own page and selection ----

#[test]
fn a_pointer_click_by_an_agent_acts_on_its_page_and_selection_and_not_the_windows() {
    let (mut s, first, second, r) = a_and_b_apart();
    s.with_agent(Some("a"), |s| s.execute("page.select", &json!({"id": second}))).unwrap();
    let ra = rect_at(&mut s, Some("a"), 200.0, 200.0);
    // Calibration: the rectangle is on a's page, which is not the window's.
    assert_eq!(s.doc().unwrap().page_of(ra), Some(second));
    assert_eq!(s.ds().unwrap().page, first);

    pointer_as(&mut s, Some("a"), PointerKind::Down, 220.0, 220.0).unwrap();
    pointer_as(&mut s, Some("a"), PointerKind::Up, 220.0, 220.0).unwrap();
    assert_eq!(s.agent_list()["agents"][0]["selection"], json!([ra]), "a's click picked a's rectangle, on a's page");
    let ds = s.ds().unwrap();
    assert_eq!((ds.page, ds.selection.clone()), (first, vec![r]), "the window shows the user's page and selection again");

    // A click on nothing empties a's selection, not the window's.
    pointer_as(&mut s, Some("a"), PointerKind::Down, 900.0, 900.0).unwrap();
    pointer_as(&mut s, Some("a"), PointerKind::Up, 900.0, 900.0).unwrap();
    assert_eq!(s.agent_list()["agents"][0]["selection"], json!([]));
    assert_eq!(s.ds().unwrap().selection, vec![r]);
}

#[test]
fn escape_by_an_agent_cancels_its_own_gesture_and_leaves_the_windows_selection() {
    let (mut s, first, _, r) = a_and_b_apart();
    let ra = rect_at(&mut s, Some("a"), 200.0, 200.0);
    pointer_as(&mut s, Some("a"), PointerKind::Down, 220.0, 220.0).unwrap();
    pointer_as(&mut s, Some("a"), PointerKind::Move, 230.0, 230.0).unwrap();
    pointer_as(&mut s, Some("a"), PointerKind::Move, 240.0, 240.0).unwrap();
    assert!(s.tool.busy(), "calibration: a's drag is under way");
    assert_eq!(s.agent_list()["agents"][0]["selection"], json!([ra]), "calibration: a holds its own selection");

    s.with_agent(Some("a"), |s| s.escape()).unwrap();
    assert!(!s.tool.busy() && !interaction_open(&s), "the drag is cancelled");
    let ds = s.ds().unwrap();
    assert_eq!((ds.page, ds.selection.clone()), (first, vec![r]), "the window's page and selection are the user's");
    assert_eq!(s.doc().unwrap().world_bounds(ra).unwrap().x0, 200.0, "the rectangle is back where it was");
}

// ---- Signature: with_agent signs what the agent writes ----

#[test]
fn what_an_agent_writes_inside_with_agent_is_signed_by_the_agent() {
    let mut s = Session::new();
    let page = s.page().unwrap();
    let comment = json!({"x": 1, "y": 1, "text": "hello", "page": page});
    s.execute("comment.add", &comment).unwrap();
    // Calibration: the user's own comment is not an agent's.
    assert!(!s.doc().unwrap().comments[0].first.agent);

    s.with_agent(Some("Claude"), |s| s.execute("comment.add", &comment)).unwrap();
    let first = &s.doc().unwrap().comments[1].first;
    assert_eq!(first.author, "Claude (agent)");
    assert!(first.agent, "the comment is marked as an agent's");
    assert!(s.agent.is_none(), "the signature does not outlive the call");
    s.execute("comment.add", &comment).unwrap();
    assert!(!s.doc().unwrap().comments[2].first.agent, "the next call is the user's again");
}

// ---- Gesture: it belongs to whoever began it ----

#[test]
fn an_agent_cannot_act_in_a_gesture_of_the_window() {
    let (mut s, ..) = a_and_b_apart();
    window_drag(&mut s);
    let doc = Arc::clone(&s.ds().unwrap().doc);

    for kind in [PointerKind::Move, PointerKind::Up, PointerKind::Down] {
        let r = pointer_as(&mut s, Some("a"), kind, 50.0, 50.0);
        assert!(refused_gesture(&r), "{kind:?}: {r:?}");
    }
    let r = s.with_agent(Some("a"), |s| s.tool_key("Enter", false, false, false));
    assert!(refused_gesture(&r), "{r:?}");
    let r = s.with_agent(Some("a"), |s| s.escape());
    assert!(refused_gesture(&r), "{r:?}");
    assert!(s.tool.busy(), "the window's drag is still under way");
    assert!(Arc::ptr_eq(&doc, &s.ds().unwrap().doc), "the refused calls left the document as it was");

    // The window's own drag goes on and ends as it would have.
    pointer_as(&mut s, None, PointerKind::Move, 50.0, 50.0).unwrap();
    pointer_as(&mut s, None, PointerKind::Up, 50.0, 50.0).unwrap();
    assert!(!s.tool.busy());
}

#[test]
fn a_gesture_an_agent_began_is_its_own_and_no_other_agent_continues_or_ends_it() {
    let (mut s, ..) = a_and_b_apart();
    pointer_as(&mut s, Some("a"), PointerKind::Down, 20.0, 20.0).unwrap();
    pointer_as(&mut s, Some("a"), PointerKind::Move, 30.0, 30.0).unwrap();
    pointer_as(&mut s, Some("a"), PointerKind::Move, 40.0, 40.0).unwrap();
    // Calibration: a's drag is under way, and b is there to be refused.
    assert!(s.tool.busy());
    assert_eq!(s.agent_list()["agents"].as_array().unwrap().len(), 2);
    let doc = Arc::clone(&s.ds().unwrap().doc);

    for kind in [PointerKind::Move, PointerKind::Up] {
        let r = pointer_as(&mut s, Some("b"), kind, 60.0, 60.0);
        assert!(refused_gesture(&r), "{kind:?}: {r:?}");
    }
    let r = s.with_agent(Some("b"), |s| s.escape());
    assert!(refused_gesture(&r), "{r:?}");
    assert!(s.tool.busy(), "b did not end a's drag");
    assert!(Arc::ptr_eq(&doc, &s.ds().unwrap().doc));

    pointer_as(&mut s, Some("a"), PointerKind::Move, 50.0, 50.0).unwrap();
    pointer_as(&mut s, Some("a"), PointerKind::Up, 50.0, 50.0).unwrap();
    assert!(!s.tool.busy(), "a finished its own drag");
}

#[test]
fn the_user_is_never_refused_during_a_gesture_of_an_agent() {
    let (mut s, ..) = a_and_b_apart();
    pointer_as(&mut s, Some("a"), PointerKind::Down, 20.0, 20.0).unwrap();
    pointer_as(&mut s, Some("a"), PointerKind::Move, 30.0, 30.0).unwrap();
    pointer_as(&mut s, Some("a"), PointerKind::Move, 40.0, 40.0).unwrap();
    assert!(s.tool.busy(), "calibration: a's drag is under way");
    // The refusal is for agents: the same event from a second agent is refused.
    assert!(refused_gesture(&pointer_as(&mut s, Some("b"), PointerKind::Move, 45.0, 45.0)), "calibration");

    pointer_as(&mut s, None, PointerKind::Move, 50.0, 50.0).unwrap();
    s.with_agent(None, |s| s.escape()).unwrap();
    assert!(!s.tool.busy(), "the user's Escape ended a's drag");
}

#[test]
fn the_interaction_api_refuses_an_agent_in_the_windows_interaction() {
    let (mut s, _, _, r) = a_and_b_apart();
    s.begin_interaction("Move").unwrap();
    move_preview(&mut s, r, 20.0).unwrap();
    assert!(interaction_open(&s), "calibration: the window's interaction is open");
    let (doc, depth) = (Arc::clone(&s.ds().unwrap().doc), s.undo_depth());

    let r1 = s.with_agent(Some("a"), |s| s.begin_interaction("Other"));
    assert!(refused_gesture(&r1), "begin: {r1:?}");
    let r2 = s.with_agent(Some("a"), |s| move_preview(s, r, 90.0));
    assert!(refused_gesture(&r2), "preview: {r2:?}");
    let r3 = s.with_agent(Some("a"), |s| s.commit_interaction());
    assert!(refused_gesture(&r3), "commit: {r3:?}");
    let r4 = s.with_agent(Some("a"), |s| s.cancel_interaction());
    assert!(refused_gesture(&r4), "cancel: {r4:?}");
    assert!(interaction_open(&s));
    assert!(Arc::ptr_eq(&doc, &s.ds().unwrap().doc), "nothing the agent tried changed the document");
    assert_eq!(s.undo_depth(), depth);

    s.commit_interaction().unwrap();
    assert_eq!(s.undo_depth(), (depth.0 + 1, 0), "the window's gesture ends as one step of its own");
}

#[test]
fn an_interaction_an_agent_began_is_its_own() {
    let (mut s, _, _, r) = a_and_b_apart();
    s.with_agent(Some("a"), |s| s.begin_interaction("Move")).unwrap();
    s.with_agent(Some("a"), |s| move_preview(s, r, 20.0)).unwrap();
    // Calibration: the interaction is open, and b is there to be refused.
    assert!(interaction_open(&s));
    let (doc, depth) = (Arc::clone(&s.ds().unwrap().doc), s.undo_depth());

    let r1 = s.with_agent(Some("b"), |s| s.begin_interaction("Other"));
    assert!(refused_gesture(&r1), "begin: {r1:?}");
    let r2 = s.with_agent(Some("b"), |s| move_preview(s, r, 90.0));
    assert!(refused_gesture(&r2), "preview: {r2:?}");
    let r3 = s.with_agent(Some("b"), |s| s.commit_interaction());
    assert!(refused_gesture(&r3), "commit: {r3:?}");
    let r4 = s.with_agent(Some("b"), |s| s.cancel_interaction());
    assert!(refused_gesture(&r4), "cancel: {r4:?}");
    assert!(interaction_open(&s));
    assert!(Arc::ptr_eq(&doc, &s.ds().unwrap().doc));

    s.with_agent(Some("a"), |s| move_preview(s, r, 30.0)).unwrap();
    s.with_agent(Some("a"), |s| s.commit_interaction()).unwrap();
    assert_eq!(s.undo_depth(), (depth.0 + 1, 0), "a ended its own gesture as one step");
}

#[test]
fn the_user_uses_the_interaction_api_during_an_agents_interaction() {
    let (mut s, _, _, r) = a_and_b_apart();
    s.with_agent(Some("a"), |s| s.begin_interaction("Move")).unwrap();
    assert!(interaction_open(&s), "calibration: a's interaction is open");
    assert!(refused_gesture(&s.with_agent(Some("b"), |s| s.cancel_interaction())), "calibration: b is refused");

    s.with_agent(None, |s| move_preview(s, r, 20.0)).unwrap();
    s.with_agent(None, |s| s.cancel_interaction()).unwrap();
    assert!(!interaction_open(&s), "the user's cancel ended a's interaction");
}

/// Editing text opens a gesture that belongs to the agent whose call opened it. When that agent
/// leaves, nothing it held stays: the other agent can edit again.
#[test]
fn after_an_agent_leaves_another_edits_where_its_text_editor_was_open() {
    let mut s = Session::new();
    s.agent_join("b").unwrap();
    let t = id(&s.with_agent(Some("a"), |s| s.execute("node.create", &json!({"type": "text", "text": "Hi"}))).unwrap());
    s.with_agent(Some("a"), |s| s.execute("text.edit.begin", &json!({"id": t.0}))).unwrap();
    let rect = json!({"x": 0, "y": 0, "width": 10, "height": 10});
    // Calibration: the editor is open and holds b back.
    assert!(interaction_open(&s));
    let r = s.with_agent(Some("b"), |s| s.execute("shape.rectangle", &rect));
    assert!(refused_gesture(&r), "{r:?}");

    s.agent_leave("a").unwrap();
    s.with_agent(Some("b"), |s| s.execute("shape.rectangle", &rect)).unwrap();
}

#[test]
fn after_an_agent_leaves_another_acts_where_its_drag_was_under_way() {
    let (mut s, ..) = a_and_b_apart();
    pointer_as(&mut s, Some("a"), PointerKind::Down, 20.0, 20.0).unwrap();
    pointer_as(&mut s, Some("a"), PointerKind::Move, 30.0, 30.0).unwrap();
    pointer_as(&mut s, Some("a"), PointerKind::Move, 40.0, 40.0).unwrap();
    // Calibration: the drag is a's, and it holds b back.
    assert!(s.tool.busy());
    assert!(refused_gesture(&pointer_as(&mut s, Some("b"), PointerKind::Move, 60.0, 60.0)));

    s.agent_leave("a").unwrap();
    assert!(!s.tool.busy(), "the drag an agent held is let go when it leaves");
    pointer_as(&mut s, Some("b"), PointerKind::Down, 20.0, 20.0).unwrap();
    pointer_as(&mut s, Some("b"), PointerKind::Up, 20.0, 20.0).unwrap();
}

// ---- Document in front: the agent never changes it ----

/// Two documents, the first in front with its rectangle selected, `a` and `b` working apart.
fn two_documents_apart() -> (Session, u64) {
    let (mut s, ..) = a_and_b_apart();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("doc.switch", &json!({"index": 0})).unwrap();
    let uid = s.docs[0].uid;
    assert_eq!((s.docs.len(), s.active), (2, 0), "calibration: two tabs, the first in front");
    (s, uid)
}

#[test]
fn whatever_a_call_of_an_agent_apart_does_the_window_keeps_its_document() {
    let (mut s, uid) = two_documents_apart();
    s.with_agent(Some("a"), |s| s.active = 1);
    assert_eq!(s.docs[s.active].uid, uid, "the call moved the tab in front, and it is back");
}

#[test]
fn a_call_that_panics_after_changing_the_tab_gives_the_window_its_document_back() {
    let (mut s, uid) = two_documents_apart();
    let unwound = catch_unwind(AssertUnwindSafe(|| {
        s.with_agent::<()>(Some("a"), |s| {
            s.active = 1;
            panic!("a bug after the switch");
        })
    }));
    assert!(unwound.is_err(), "calibration: the panic goes on to the caller");
    assert_eq!(s.docs[s.active].uid, uid);
}

#[test]
fn the_user_changes_the_document_in_front_as_before() {
    let (mut s, uid) = two_documents_apart();
    s.with_agent(None, |s| s.active = 1);
    assert_ne!(s.docs[s.active].uid, uid, "nobody's call is not put back");
}

// ---- Registry sweep ----

/// A directory unique to one test, removed when dropped (also after a failed assertion).
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(test: &str) -> Self {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("figcraft-agent-rules-{}-{stamp}-{test}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn file(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().to_string()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What the window shows: its document, page and selection.
fn window(s: &Session) -> (u64, NodeId, Vec<NodeId>) {
    let ds = s.ds().unwrap();
    (ds.uid, ds.page, ds.selection.clone())
}

/// A session for the sweep: two tabs (the first in front, with a selected rectangle), two pages in
/// it, and `a` and `b` at work apart. Returns it with the rectangle and the second page.
fn sweep_session() -> (Session, NodeId, NodeId) {
    let (mut s, _, second, r) = a_and_b_apart();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("doc.switch", &json!({"index": 0})).unwrap();
    assert_eq!((s.docs.len(), s.active), (2, 0));
    (s, r, second)
}

/// The names a command takes, from the ones the sweep has values for.
fn params_for(command: &str, r: NodeId, second: NodeId, file: &str, index: usize) -> Value {
    let all = json!({
        "ids": [r], "id": r, "index": index, "path": file, "name": "x", "text": "t", "x": 0, "y": 0,
        "width": 10, "height": 10, "template": "dashboard", "page": second, "document": index,
    });
    let (Some(names), Some(o)) = (crate::cmd::param_names(command), all.as_object()) else { return all };
    Value::Object(
        o.iter().filter(|(k, _)| matches!(k.as_str(), "ids" | "id") || names.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect(),
    )
}

/// No command, with whatever it is given, takes the document in front away from the window: the
/// same document stays in front, no tab is added unless the call said why it failed, and the
/// window keeps its page and selection. Run confined to a folder of the test, so a file or a
/// network address a command is given reaches nothing else.
#[test]
fn no_command_of_the_registry_run_by_an_agent_changes_what_the_window_shows() {
    let scratch = Scratch::new("sweep");
    // Another file to open and import: the sweep must be able to bring a tab in.
    let other = scratch.file("other.figcraft");
    let mut elsewhere = Session::new();
    elsewhere.execute("file.saveAs", &json!({"path": other})).unwrap();
    let roots = Arc::new(Roots::new(&[], &[scratch.0.to_string_lossy().to_string()]).unwrap());

    // Calibration: the same command as the user brings another tab in front, so a change of the
    // window's document would show in `window`, and agents work apart in the sweep's session.
    let (mut s, r, second) = sweep_session();
    let before = window(&s);
    confined(Some(&roots), || s.execute("doc.switch", &json!({"index": 1}))).unwrap();
    assert_ne!(window(&s).0, before.0, "the user's doc.switch changes the document in front");
    let list = sweep_session().0.agent_list();
    assert_eq!((list["agents"].as_array().unwrap().len(), list["agents"][0]["apart"].clone()), (2, json!(true)));
    assert!(crate::all_commands().len() > 100, "the registry is what the sweep walks");
    let _ = (r, second);

    // Every command is run, and every gap is listed in the one failure at the end.
    let mut gaps: Vec<String> = Vec::new();
    for spec in crate::all_commands() {
        // Walks the font folders: one walk per call is slow, and it opens nothing.
        if spec.id == "text.rescanFonts" {
            continue;
        }
        for index in [1, 0] {
            for taken in [false, true] {
                let (mut s, r, second) = sweep_session();
                let before = window(&s);
                let params = if taken { params_for(spec.id, r, second, &other, index) } else { json!({}) };
                let result = confined(Some(&roots), || s.with_agent(Some("a"), |s| s.execute(spec.id, &params)));
                let mut gap = |name: &str| gaps.push(format!("{} with {params}: {name} ({result:?})", spec.id));
                let (uid, page, selection) = window(&s);
                if uid != before.0 {
                    gap("another document is in front");
                }
                if !s.docs.iter().any(|d| d.uid == before.0) {
                    gap("the document in front is gone");
                }
                if s.docs.len() > 2 && result.is_ok() {
                    gap("a tab was added without an error");
                }
                // A page or a layer an agent removed leaves the window: its page falls back to the
                // first one and its selection loses what no longer exists or is no longer on that page.
                let Some(doc) = s.ds().ok().map(|d| d.doc.clone()) else { continue };
                let expected_page = Some(before.1).filter(|p| doc.pages.contains(p)).or_else(|| doc.first_page());
                if Some(page) != expected_page {
                    gap(&format!("the window's page is {page:?}, expected {expected_page:?}"));
                }
                let expected_selection: Vec<NodeId> =
                    before.2.iter().copied().filter(|n| doc.contains(*n) && doc.page_of(*n) == expected_page).collect();
                if selection != expected_selection {
                    gap(&format!("the window's selection is {selection:?}, expected {expected_selection:?}"));
                }
            }
        }
    }
    assert!(gaps.is_empty(), "{} gaps in the window's document, page or selection:\n{}", gaps.len(), gaps.join("\n"));
}

// ---- A gesture taken over, and released by whoever leaves ----

#[test]
fn the_user_pressing_during_an_agents_drag_takes_the_gesture_over() {
    let (mut s, ..) = a_and_b_apart();
    pointer_as(&mut s, Some("a"), PointerKind::Down, 20.0, 20.0).unwrap();
    pointer_as(&mut s, Some("a"), PointerKind::Move, 30.0, 30.0).unwrap();
    pointer_as(&mut s, Some("a"), PointerKind::Move, 40.0, 40.0).unwrap();
    assert!(s.tool.busy(), "calibration: a's drag is under way");
    assert_eq!(s.tool.held_by.as_deref(), Some("a"), "calibration: a holds it");

    pointer_as(&mut s, None, PointerKind::Down, 60.0, 60.0).unwrap();
    assert_eq!(s.tool.held_by, None, "the gesture is the user's from the press on");
}

#[test]
fn an_agent_that_leaves_does_not_end_the_interaction_of_another() {
    let mut s = Session::new();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    s.agent_join("a").unwrap();
    s.agent_join("b").unwrap();
    s.with_agent(Some("b"), |s| s.begin_interaction("Move")).unwrap();
    assert!(interaction_open(&s), "calibration: b's interaction is open");

    s.agent_leave("a").unwrap();
    assert!(interaction_open(&s), "a held nothing: b's interaction is intact");
    // It is still b's own: b ends it, and another agent is refused.
    assert!(refused_gesture(&s.with_agent(Some("a"), |s| s.cancel_interaction())), "calibration: it is b's");
    s.with_agent(Some("b"), |s| s.commit_interaction()).unwrap();
    assert!(!interaction_open(&s));
}

/// The user who presses on the layer an agent drags goes on in the agent's interaction: that
/// interaction is the user's too from then on. Left under the agent's name, it would be committed
/// when the agent leaves, and the user's drag would have nothing under it.
#[test]
fn the_interaction_goes_to_the_user_who_takes_an_agents_drag_over() {
    let (mut s, ..) = a_and_b_apart();
    pointer_as(&mut s, Some("a"), PointerKind::Down, 20.0, 20.0).unwrap();
    pointer_as(&mut s, Some("a"), PointerKind::Move, 30.0, 30.0).unwrap();
    pointer_as(&mut s, Some("a"), PointerKind::Move, 40.0, 40.0).unwrap();
    let owner = |s: &Session| s.ds().unwrap().interaction.as_ref().map(|it| it.agent.clone());
    // Calibration: a's drag is under way in an interaction of its own; the rectangle is at 20, 20.
    assert!(s.tool.busy());
    assert_eq!(owner(&s), Some(Some("a".to_string())));

    pointer_as(&mut s, None, PointerKind::Down, 40.0, 40.0).unwrap();
    assert_eq!(s.tool.held_by, None, "the gesture is the user's from the press on");
    assert_eq!(owner(&s), Some(None), "and so is the interaction it works in");

    // `a` leaves during the user's drag: nothing of the user's is ended with it.
    s.agent_leave("a").unwrap();
    assert!(interaction_open(&s), "the user's drag still has its interaction");
    pointer_as(&mut s, None, PointerKind::Move, 50.0, 50.0).unwrap();
    pointer_as(&mut s, None, PointerKind::Up, 50.0, 50.0).unwrap();
    assert!(!interaction_open(&s), "the user ended it");
}

/// A tool gesture that opens no interaction (a marquee, a pencil stroke, a pan) is its holder's
/// too: another agent begins no interaction during it. It could not go on with one (its preview
/// and its commit are refused), and the holder's next event would be refused in turn.
#[test]
fn an_agent_begins_no_interaction_during_another_agents_tool_gesture() {
    let (mut s, ..) = a_and_b_apart();
    pointer_as(&mut s, Some("a"), PointerKind::Down, 200.0, 200.0).unwrap();
    pointer_as(&mut s, Some("a"), PointerKind::Move, 220.0, 220.0).unwrap();
    // Calibration: a drags a marquee off any layer, which opens no interaction.
    assert!(s.tool.busy() && !interaction_open(&s));
    assert_eq!(s.tool.held_by.as_deref(), Some("a"));

    let r = s.with_agent(Some("b"), |s| s.begin_interaction("Other"));
    assert!(refused_gesture(&r), "begin: {r:?}");
    assert!(!interaction_open(&s), "b opened nothing");
    // a goes on with its gesture and ends it.
    pointer_as(&mut s, Some("a"), PointerKind::Move, 240.0, 240.0).unwrap();
    pointer_as(&mut s, Some("a"), PointerKind::Up, 240.0, 240.0).unwrap();
    assert!(!s.tool.busy());
}

// ---- Going back in the history: never another agent's step ----

/// The pen's double-click takes its first press back by going back in the history. The press is
/// its presser's only while nobody else has edited the document since: b's step forgets a's
/// press, so a's double-click takes nothing back.
#[test]
fn the_pens_double_click_of_an_agent_does_not_take_back_another_agents_step() {
    let mut s = Session::new();
    s.agent_join("a").unwrap();
    s.agent_join("b").unwrap();
    s.set_tool(crate::tools::ToolId::Pen);
    let click = |s: &mut Session, x: f64, y: f64| {
        pointer_as(s, Some("a"), PointerKind::Down, x, y).unwrap();
        pointer_as(s, Some("a"), PointerKind::Up, x, y).unwrap();
    };
    // a draws a path, then keeps the pen in edit mode (Escape), as `pen_vector_edit_tests` does.
    click(&mut s, 100.0, 100.0);
    click(&mut s, 200.0, 100.0);
    s.with_agent(Some("a"), |s| s.escape()).unwrap();
    // The first press of a double-click off the path begins something new...
    click(&mut s, 150.0, 300.0);
    // ...and b draws a rectangle before the second press.
    let rect = rect_at(&mut s, Some("b"), 400.0, 400.0);
    // Calibration: b's rectangle is in the document, a step made after a's press.
    assert!(s.doc().unwrap().node(rect).is_some());

    click(&mut s, 150.0, 300.0);
    pointer_as(&mut s, Some("a"), PointerKind::DoubleClick, 150.0, 300.0).unwrap();
    assert!(s.doc().unwrap().node(rect).is_some(), "a's double-click took b's rectangle back");
}

/// Who is still there when the double-click comes says nothing about the steps made since the
/// press: b drew after a's press and left, and its rectangle is no part of that press.
#[test]
fn the_pens_double_click_does_not_take_back_the_step_of_an_agent_that_left() {
    let mut s = Session::new();
    s.agent_join("a").unwrap();
    s.agent_join("b").unwrap();
    s.set_tool(crate::tools::ToolId::Pen);
    let click = |s: &mut Session, x: f64, y: f64| {
        pointer_as(s, Some("a"), PointerKind::Down, x, y).unwrap();
        pointer_as(s, Some("a"), PointerKind::Up, x, y).unwrap();
    };
    click(&mut s, 100.0, 100.0);
    click(&mut s, 200.0, 100.0);
    s.with_agent(Some("a"), |s| s.escape()).unwrap();
    // The first press of a's double-click, then b draws a rectangle and leaves.
    click(&mut s, 150.0, 300.0);
    let rect = rect_at(&mut s, Some("b"), 400.0, 400.0);
    // Calibration: b's rectangle is in the document, a step made after a's press.
    assert!(s.doc().unwrap().node(rect).is_some());
    s.agent_leave("b").unwrap();

    click(&mut s, 150.0, 300.0);
    pointer_as(&mut s, Some("a"), PointerKind::DoubleClick, 150.0, 300.0).unwrap();
    assert!(s.doc().unwrap().node(rect).is_some(), "a's double-click took back the rectangle of an agent that left");
}

/// The same for the user, whose undo is never refused: an agent's step made between the user's
/// two presses is not the user's press.
#[test]
fn the_users_pen_double_click_does_not_take_back_an_agents_step() {
    let mut s = Session::new();
    s.agent_join("b").unwrap();
    s.set_tool(crate::tools::ToolId::Pen);
    let click = |s: &mut Session, x: f64, y: f64| {
        pointer_as(s, None, PointerKind::Down, x, y).unwrap();
        pointer_as(s, None, PointerKind::Up, x, y).unwrap();
    };
    click(&mut s, 100.0, 100.0);
    click(&mut s, 200.0, 100.0);
    s.with_agent(None, |s| s.escape()).unwrap();
    // The first press of the user's double-click, then b draws a rectangle.
    click(&mut s, 150.0, 300.0);
    let rect = rect_at(&mut s, Some("b"), 400.0, 400.0);
    // Calibration: b's rectangle is in the document, a step made after the user's press.
    assert!(s.doc().unwrap().node(rect).is_some());

    click(&mut s, 150.0, 300.0);
    pointer_as(&mut s, None, PointerKind::DoubleClick, 150.0, 300.0).unwrap();
    assert!(s.doc().unwrap().node(rect).is_some(), "the user's double-click took the agent's rectangle back");
}

/// A press is taken back by whoever made it: once the agent that pressed has left, a double-click
/// there (the user's here) takes nothing back in its name.
#[test]
fn the_pens_double_click_takes_nothing_back_for_the_press_of_an_agent_that_left() {
    let mut s = Session::new();
    s.set_tool(crate::tools::ToolId::Pen);
    let click = |s: &mut Session, x: f64, y: f64| {
        pointer_as(s, Some("a"), PointerKind::Down, x, y).unwrap();
        pointer_as(s, Some("a"), PointerKind::Up, x, y).unwrap();
    };
    // a, the only agent, works on what the window shows.
    click(&mut s, 100.0, 100.0);
    click(&mut s, 200.0, 100.0);
    s.with_agent(Some("a"), |s| s.escape()).unwrap();
    let before = s.undo_depth().0;
    click(&mut s, 150.0, 300.0);
    let depth = s.undo_depth().0;
    assert!(depth > before, "calibration: a's press began something new, a step of its own");
    s.agent_leave("a").unwrap();

    pointer_as(&mut s, None, PointerKind::DoubleClick, 150.0, 300.0).unwrap();
    assert_eq!(s.undo_depth().0, depth, "the user's double-click took back the press of an agent that left");
}

// ---- The agent that worked alone keeps what it had when it starts working apart ----

/// One agent alone works on what the window shows. When a second one comes it works apart, and
/// the text it was editing is still its own: it goes on typing in it, and ends it.
#[test]
fn the_agent_that_worked_alone_goes_on_typing_in_its_text_when_a_second_one_comes() {
    let mut s = Session::new();
    let t = id(&s.with_agent(Some("a"), |s| s.execute("node.create", &json!({"type": "text", "text": "Hi"}))).unwrap());
    s.with_agent(Some("a"), |s| s.execute("text.edit.begin", &json!({"id": t.0}))).unwrap();
    // Calibration: alone, a edits on what the window shows.
    assert!(s.ds().unwrap().text_edit.is_some());

    s.with_agent(Some("b"), |_| ());
    assert!(s.with_agent(Some("a"), |s| s.ds().unwrap().text_edit.is_some()), "a lost the editor it opened");
    s.with_agent(Some("a"), |s| s.execute("text.edit.insert", &json!({"text": "Hello"}))).unwrap();
    s.with_agent(Some("a"), |s| s.execute("text.edit.end", &json!({}))).unwrap();
    assert!(!interaction_open(&s), "a ended its own editing");
}

/// The same for the other editing modes (the points of a vector, here), whether a second agent
/// comes or the agent asks to work apart while alone.
#[test]
fn the_agent_that_worked_alone_keeps_its_editing_mode_when_it_starts_working_apart() {
    let editing_points = || {
        let mut s = Session::new();
        let r = rect_at(&mut s, Some("a"), 0.0, 0.0);
        s.with_agent(Some("a"), |s| s.execute("vector.edit.begin", &json!({"id": r}))).unwrap();
        assert!(s.ds().unwrap().vector_edit.is_some(), "calibration: alone, a edits the points of its rectangle");
        s
    };
    let mut s = editing_points();
    s.with_agent(Some("b"), |_| ());
    assert!(s.with_agent(Some("a"), |s| s.ds().unwrap().vector_edit.is_some()), "a lost its editing mode when b came");

    let mut s = editing_points();
    s.agent_join("a").unwrap();
    assert!(s.with_agent(Some("a"), |s| s.ds().unwrap().vector_edit.is_some()), "a lost its editing mode when it joined");
}

// ---- The event log: nobody's name ----

#[test]
fn events_poll_is_answered_for_nobody() {
    assert_eq!(crate::agent::door("events.poll"), Some(Door::Events));
    assert!(crate::agent::DOORS.iter().filter(|(_, d)| *d == Door::Events).all(|(m, _)| *m == "events.poll"));

    let mut s = Session::new();
    s.agent_admit(Some("held"), Some("key")).unwrap();
    // The name belongs to another key, so a request for it is refused by any door that admits...
    let other = json!({"agent": "held", "agentKey": "another"});
    assert!(s.agent_admit_request("node.get", &other).is_err(), "calibration: the name is held");
    // ...and the event log asks nobody.
    assert_eq!(s.agent_admit_request("events.poll", &other).unwrap(), Door::Events);

    let acting = s.agent_begin(Some("held"), Door::Events);
    assert!(s.agent.is_none() && s.actor().is_none(), "nobody acts for it");
    assert_eq!(s.agent_list()["agents"], json!([]), "and nobody is there for it");
    s.agent_end(acting);
}
