//! The eyedropper with Cmd/Ctrl+Shift held makes a colour style or variable of what it picks (as
//! the reference app's eyedropper does): it asks the UI which (`dialog.createFromColor`) and leaves the
//! selection's fills alone; a plain pick still fills the selection.

use figcraft_doc::NodeId;
use serde_json::json;

use crate::Session;
use crate::tools::{PointerEvent, PointerKind, ToolId};

/// A white page with a red square at (0, 0) and a green one selected at (200, 0).
fn scene() -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("page.setBackground", &json!({"color": "#FFFFFF"})).unwrap();
    let rect = |s: &mut Session, x: f64, fill: &str| {
        let r = s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 100, "height": 100})).unwrap();
        let id = crate::params::id_of(&r["id"]).unwrap();
        s.execute("node.set", &json!({"id": id, "props": {"fills": [fill]}})).unwrap();
        id
    };
    rect(&mut s, 0.0, "#FF0000");
    let green = rect(&mut s, 200.0, "#00FF00");
    s.execute("select.set", &json!({"ids": [green]})).unwrap();
    s.ui_requests.clear();
    (s, green)
}

fn pick(s: &mut Session, cmd: bool, shift: bool) {
    s.execute("edit.pickColor", &json!({})).unwrap();
    s.pointer(&PointerEvent { cmd, shift, ..PointerEvent::at(PointerKind::Down, 50.0, 50.0, 1.0) }).unwrap();
}

fn fill(s: &Session, id: NodeId) -> Option<String> {
    s.doc().unwrap().node(id).unwrap().fills.first().and_then(|p| p.solid_color()).map(|c| c.to_hex())
}

#[test]
fn cmd_shift_asks_to_make_a_style_or_variable_of_the_pick() {
    let (mut s, green) = scene();
    let undo = s.undo_depth().0;
    pick(&mut s, true, true);
    let asks: Vec<_> = s.ui_requests.iter().filter(|r| r.kind == "dialog.createFromColor").collect();
    assert_eq!(asks.len(), 1, "{:?}", s.ui_requests);
    assert_eq!(asks[0].data["color"], "#FF0000", "the colour under the pointer");
    assert_eq!(fill(&s, green), Some("00FF00".into()), "the selection keeps its fill");
    assert_eq!(s.undo_depth().0, undo, "nothing to undo");
    assert_eq!(s.tool.tool, ToolId::Move, "picking ends the eyedropper");
    assert_eq!(s.recent_colors.first().map(|c| c.to_hex()), Some("FF0000".into()), "a recent colour as any pick");
}

#[test]
fn other_picks_fill_the_selection_as_before() {
    for (cmd, shift) in [(false, false), (true, false), (false, true)] {
        let (mut s, green) = scene();
        pick(&mut s, cmd, shift);
        assert!(!s.ui_requests.iter().any(|r| r.kind == "dialog.createFromColor"), "cmd {cmd} shift {shift}");
        assert_eq!(fill(&s, green), Some("FF0000".into()), "cmd {cmd} shift {shift}");
    }
    // The colour picker's eyedropper only samples, whatever is held.
    let (mut s, green) = scene();
    s.execute("edit.pickColor", &json!({"apply": false})).unwrap();
    s.pointer(&PointerEvent { cmd: true, shift: true, ..PointerEvent::at(PointerKind::Down, 50.0, 50.0, 1.0) }).unwrap();
    assert!(s.ui_requests.is_empty());
    assert_eq!(s.tool.picked.map(|c| c.to_hex()), Some("FF0000".into()));
    assert_eq!(fill(&s, green), Some("00FF00".into()));
}
