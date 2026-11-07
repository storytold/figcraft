//! Dev Mode behaviour suite, written from the public documentation of handoff tools (in our own
//! words): the canvas inspects and never edits, what the menus and shortcuts may run there, the
//! list of what's ready for development, statuses (ready, changed, completed) and the focus view.

use crate::Session;
use crate::components::tests::{ex, idv};
use crate::devinspect::dev_mode_allows;
use crate::tools::{PointerEvent, PointerKind, ToolId};
use figcraft_doc::{DevStatus, NodeId};
use serde_json::{Value, json};

fn ev(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
    PointerEvent::at(kind, x, y, 1.0)
}

fn drag(s: &mut Session, from: (f64, f64), to: (f64, f64)) {
    s.pointer(&ev(PointerKind::Down, from.0, from.1)).unwrap();
    for k in 1..=4 {
        let t = f64::from(k) / 4.0;
        s.pointer(&ev(PointerKind::Move, from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t)).unwrap();
    }
    s.pointer(&ev(PointerKind::Up, to.0, to.1)).unwrap();
}

fn frame(s: &mut Session, x: f64, name: &str) -> NodeId {
    idv(&ex(s, "frame.create", json!({"x": x, "y": 0, "width": 200, "height": 200, "name": name})))
}

fn rect(s: &mut Session, parent: NodeId, x: f64) -> NodeId {
    idv(&ex(s, "shape.rectangle", json!({"x": x, "y": 20, "width": 40, "height": 40, "parent": parent})))
}

fn names(v: &Value) -> Vec<String> {
    v["items"].as_array().map(|a| a.iter().filter_map(|i| i["name"].as_str().map(String::from)).collect()).unwrap_or_default()
}

fn status(s: &mut Session, ids: &[NodeId], st: &str) {
    ex(s, "node.set", json!({"ids": ids, "props": {"devStatus": st}}));
}

#[test]
fn a_dev_mode_click_on_a_selected_layer_selects_just_it() {
    let mut s = Session::new();
    let a = idv(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 40, "height": 40})));
    let b = idv(&ex(&mut s, "shape.rectangle", json!({"x": 80, "y": 0, "width": 40, "height": 40})));
    s.tool.dev_mode = true;
    ex(&mut s, "select.set", json!({"ids": [a, b]}));
    // Dev Mode starts no move gesture, so the click narrows the selection at the press itself.
    s.pointer(&ev(PointerKind::Down, 20.0, 20.0)).unwrap();
    s.pointer(&ev(PointerKind::Up, 20.0, 20.0)).unwrap();
    assert_eq!(s.selection(), vec![a], "a click on one of several selected layers selects it");
}

#[test]
fn the_dev_mode_canvas_selects_and_pans_but_never_edits() {
    let mut s = Session::new();
    let f = frame(&mut s, 0.0, "Screen");
    let r = rect(&mut s, f, 20.0);
    let t = idv(&ex(&mut s, "text.create", json!({"text": "Hello", "x": 100, "y": 120, "parent": f})));
    s.tool.dev_mode = true;
    let before = s.doc().unwrap().clone();
    let steps = s.undo_labels(100).len();
    // A press selects; dragging it moves nothing.
    drag(&mut s, (40.0, 40.0), (140.0, 90.0));
    assert_eq!(s.selection(), vec![r]);
    // Nor do its resize handles.
    drag(&mut s, (60.0, 60.0), (120.0, 120.0));
    // Drawing tools only select there.
    for tool in [ToolId::Rectangle, ToolId::Frame, ToolId::Text, ToolId::Pen, ToolId::Pencil, ToolId::Scale] {
        s.set_tool(tool);
        drag(&mut s, (300.0, 300.0), (360.0, 360.0));
    }
    s.set_tool(ToolId::Move);
    // A double-click on text doesn't start typing; on a frame's layer it selects into it.
    ex(&mut s, "select.set", json!({"ids": [t]}));
    s.pointer(&ev(PointerKind::DoubleClick, 110.0, 128.0)).unwrap();
    assert!(s.ds().unwrap().text_edit.is_none(), "no text editing");
    ex(&mut s, "select.set", json!({"ids": [r]}));
    s.pointer(&ev(PointerKind::DoubleClick, 40.0, 40.0)).unwrap();
    assert!(s.ds().unwrap().vector_edit.is_none(), "no vector editing");
    assert!(*s.doc().unwrap() == before, "the design is untouched");
    assert_eq!(s.undo_labels(100).len(), steps, "nothing to undo");
    // Keys that edit with a tool (a selected guide's Delete) go to the commands instead.
    assert!(!s.tool_key("Delete", false, false, false).unwrap());
    // The hand still pans; comments still start.
    s.set_tool(ToolId::Hand);
    let cx = s.ds().unwrap().view.cx;
    s.pointer(&ev(PointerKind::Down, 500.0, 500.0)).unwrap();
    s.pointer(&PointerEvent { sx: 550.0, ..ev(PointerKind::Move, 500.0, 500.0) }).unwrap();
    s.pointer(&ev(PointerKind::Up, 500.0, 500.0)).unwrap();
    assert_ne!(s.ds().unwrap().view.cx, cx, "panned");
    s.set_tool(ToolId::Comment);
    s.pointer(&ev(PointerKind::Down, 500.0, 500.0)).unwrap();
    assert!(s.ui_requests.iter().any(|u| u.kind == "comment.new"));
    // Back in the design: the same drag moves.
    s.tool.dev_mode = false;
    s.set_tool(ToolId::Move);
    drag(&mut s, (40.0, 40.0), (140.0, 90.0));
    assert!(s.doc().unwrap().node(r).is_some_and(|n| n.transform.as_coeffs()[4] > 100.0), "moved");
}

#[test]
fn dev_mode_runs_its_own_edits_and_refuses_design_edits_from_menus_and_shortcuts() {
    for id in [
        "annotation.add",
        "annotation.edit",
        "dev.resolveChange",
        "dev.focus",
        "devResource.add",
        "codeConnect.set",
        "comment.add",
        "edit.undo",
        "edit.redo",
        "edit.copy",
        "edit.copyAsCss",
        "select.all",
        "view.zoomIn",
        "view.devMode",
        "file.export",
        "file.save",
        "tool.move",
        "tool.hand",
        "tool.comment",
        "inspect.code",
    ] {
        assert!(dev_mode_allows(id), "{id} runs in Dev Mode");
    }
    for id in [
        "edit.delete",
        "edit.paste",
        "edit.cut",
        "edit.duplicate",
        "transform.nudge",
        "object.group",
        "tool.rectangle",
        "tool.text",
        "tool.place",
        "file.import",
        "file.revert",
        "data.fill",
    ] {
        assert!(!dev_mode_allows(id), "{id} edits the design");
    }
}

#[test]
fn the_ready_list_shows_frames_and_sections_with_a_status_across_pages() {
    let mut s = Session::new();
    let a = frame(&mut s, 0.0, "Login");
    let b = frame(&mut s, 300.0, "Signup");
    let nested = idv(&ex(&mut s, "frame.create", json!({"x": 10, "y": 10, "width": 50, "height": 50, "parent": a, "name": "Card"})));
    status(&mut s, &[a], "readyForDev");
    status(&mut s, &[b], "completed");
    // A layer inside a frame takes no status of its own (its frame's covers it).
    assert!(s.execute("node.set", &json!({"id": nested, "props": {"devStatus": "readyForDev"}})).is_err());
    assert!(s.execute("node.set", &json!({"ids": [a, nested], "props": {"devStatus": "completed"}})).is_err(), "all or nothing");
    assert_eq!(s.doc().unwrap().node(a).map(|n| n.dev_status), Some(DevStatus::ReadyForDev));
    // Clearing one is always fine.
    ex(&mut s, "node.set", json!({"id": nested, "props": {"devStatus": null}}));
    let pg2 = idv(&ex(&mut s, "page.add", json!({"name": "Settings"})));
    ex(&mut s, "page.select", json!({"id": pg2}));
    let c = frame(&mut s, 0.0, "Profile");
    status(&mut s, &[c], "readyForDev");
    assert_eq!(names(&ex(&mut s, "inspect.readyForDev", json!({}))), vec!["Login", "Profile"]);
    assert_eq!(names(&ex(&mut s, "inspect.readyForDev", json!({"status": "completed"}))), vec!["Signup"]);
    assert_eq!(names(&ex(&mut s, "inspect.readyForDev", json!({"status": "any"}))), vec!["Login", "Signup", "Profile"]);
    // Statuses are undoable.
    s.undo().unwrap();
    assert_eq!(names(&ex(&mut s, "inspect.readyForDev", json!({}))), vec!["Login"]);
}

#[test]
fn ready_frames_edited_after_marking_show_as_changed_until_resolved_or_completed() {
    let mut s = Session::new();
    let a = frame(&mut s, 0.0, "Login");
    let b = frame(&mut s, 300.0, "Signup");
    let r = rect(&mut s, a, 20.0);
    let outside = idv(&ex(&mut s, "shape.rectangle", json!({"x": 900, "y": 0, "width": 10, "height": 10})));
    status(&mut s, &[a, b], "readyForDev");
    let changed = |s: &mut Session| names(&ex(s, "inspect.readyForDev", json!({"status": "changed"})));
    assert!(changed(&mut s).is_empty(), "marking isn't a change");
    // Edits elsewhere don't count; an edit inside does.
    ex(&mut s, "node.set", json!({"id": outside, "props": {"width": 30}}));
    assert!(changed(&mut s).is_empty());
    ex(&mut s, "node.set", json!({"id": r, "props": {"width": 60}}));
    assert_eq!(changed(&mut s), vec!["Login"]);
    let items = ex(&mut s, "inspect.readyForDev", json!({}));
    assert_eq!(items["items"][0]["changed"], true, "the list says which changed: {items}");
    assert_eq!(items["items"][1]["changed"], false);
    // Still ready for dev (the status itself stays).
    assert_eq!(names(&items), vec!["Login", "Signup"]);
    // Resolving goes back to plain ready; so does marking it again, or completing it.
    ex(&mut s, "dev.resolveChange", json!({"ids": [a]}));
    assert!(changed(&mut s).is_empty());
    ex(&mut s, "node.set", json!({"id": r, "props": {"width": 70}}));
    status(&mut s, &[a], "completed");
    assert!(changed(&mut s).is_empty());
    let all = ex(&mut s, "inspect.readyForDev", json!({"status": "any"}));
    assert_eq!(all["items"][0]["changed"], false, "a completed frame isn't changed: {all}");
    // Completed frames don't turn changed when edited.
    ex(&mut s, "node.set", json!({"id": r, "props": {"width": 80}}));
    assert!(!s.doc().unwrap().node(a).unwrap().dev_changed);
}

#[test]
fn the_focus_view_steps_through_what_is_ready_and_skips_completed_frames() {
    let mut s = Session::new();
    let a = frame(&mut s, 0.0, "Login");
    let b = frame(&mut s, 300.0, "Signup");
    let c = frame(&mut s, 600.0, "Profile");
    let r = rect(&mut s, c, 620.0);
    status(&mut s, &[a, c], "readyForDev");
    status(&mut s, &[b], "completed");
    ex(&mut s, "node.set", json!({"id": r, "props": {"width": 60}}));
    let step = |s: &mut Session, d: &str| ex(s, "dev.focus", json!({"direction": d}));
    let v = ex(&mut s, "dev.focus", json!({"id": a}));
    assert_eq!((v["index"].clone(), v["count"].clone()), (json!(0), json!(2)));
    assert_eq!(step(&mut s, "next")["id"], json!(c), "completed ones are skipped, changed ones aren't");
    assert_eq!(step(&mut s, "previous")["id"], json!(a));
    // Any top-level frame opens in it, even one that's completed.
    assert_eq!(ex(&mut s, "dev.focus", json!({"id": b}))["index"], json!(null));
}

#[test]
fn a_drag_under_way_when_dev_mode_starts_moves_nothing() {
    let mut s = Session::new();
    let f = frame(&mut s, 0.0, "Screen");
    let r = rect(&mut s, f, 20.0);
    ex(&mut s, "select.set", json!({"ids": [r]}));
    let before = s.doc().unwrap().world_bounds(r);
    s.pointer(&ev(PointerKind::Down, 40.0, 40.0)).unwrap();
    s.pointer(&ev(PointerKind::Move, 90.0, 40.0)).unwrap();
    s.drop_gesture().unwrap();
    s.tool.dev_mode = true;
    s.pointer(&ev(PointerKind::Move, 120.0, 40.0)).unwrap();
    s.pointer(&ev(PointerKind::Up, 120.0, 40.0)).unwrap();
    assert_eq!(s.doc().unwrap().world_bounds(r), before, "the drag was dropped, not committed");
}
