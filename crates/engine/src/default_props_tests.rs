//! Edit ▸ Set default properties (#2028): new rectangles, ellipses, polygons and stars drawn with
//! the tools take the properties of the layer it was set from, and new text takes a text layer's;
//! it no longer only copied them for Paste properties.

use figcraft_color::Color;
use figcraft_doc::{Node, NodeId, NodeKind};
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::tools::{PointerEvent, PointerKind, ToolId};

/// Run `cmd`, failing the test on an error.
fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

/// The layer `id` as it is now.
fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().node(id).unwrap().clone()
}

/// Draw with `tool` from `a` to `b` (a click when they're equal); the new layer.
fn draw(s: &mut Session, tool: ToolId, a: (f64, f64), b: (f64, f64)) -> NodeId {
    s.set_tool(tool);
    s.pointer(&PointerEvent::at(PointerKind::Down, a.0, a.1, 1.0)).unwrap();
    if a != b {
        s.pointer(&PointerEvent::at(PointerKind::Move, (a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0, 1.0)).unwrap();
        s.pointer(&PointerEvent::at(PointerKind::Move, b.0, b.1, 1.0)).unwrap();
    }
    s.pointer(&PointerEvent::at(PointerKind::Up, b.0, b.1, 1.0)).unwrap();
    *s.selection().first().unwrap()
}

/// The first solid fill's colour of layer `n`.
fn fill(n: &Node) -> Option<Color> {
    n.fills.first().and_then(|p| p.solid_color())
}

/// A red, half-transparent rectangle with a 4 pt blue stroke and 12 pt corners.
fn styled(s: &mut Session) -> NodeId {
    let id = id_of(&ex(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"]).unwrap();
    ex(s, "node.set", json!({"id": id, "props": {"fill": "#FF0000", "stroke": "#0000FF", "strokeWeight": 4, "opacity": 0.5, "cornerRadius": 12}}));
    id
}

/// Rectangles, ellipses, polygons and stars drawn or clicked with their tools take the shape
/// defaults (corners only where a shape has them, its own drawn size); frames, lines and layers made
/// by commands keep their own look; undo takes a drawn shape away whole.
#[test]
fn new_shapes_drawn_with_the_tools_take_the_default_properties() {
    let mut s = Session::new();
    let source = styled(&mut s);
    let set = ex(&mut s, "edit.setDefaultProperties", json!({"id": source}));
    assert_eq!(set["for"], "shapes");
    let red = Some(Color::from_hex("#FF0000").unwrap());
    // Dragged and clicked (a click places a 100×100 shape) alike.
    for (tool, a, b) in
        [(ToolId::Rectangle, (200.0, 0.0), (300.0, 80.0)), (ToolId::Ellipse, (400.0, 0.0), (400.0, 0.0)), (ToolId::Star, (600.0, 0.0), (660.0, 60.0))]
    {
        let id = draw(&mut s, tool, a, b);
        let n = node(&s, id);
        assert_eq!(fill(&n), red, "{tool:?}");
        assert_eq!((n.stroke.weight, n.opacity), (4.0, 0.5), "{tool:?}");
        assert_eq!(n.strokes.first().and_then(|p| p.solid_color()), Some(Color::from_hex("#0000FF").unwrap()), "{tool:?}");
    }
    let rect = draw(&mut s, ToolId::Rectangle, (0.0, 200.0), (80.0, 260.0));
    assert_eq!(node(&s, rect).corners.radii, [12.0; 4], "corners go to layers with corners");
    assert_eq!((node(&s, rect).width, node(&s, rect).height), (80.0, 60.0), "the drawn size, not the source's");
    // Frames and lines keep their own look; a shape made by a command too.
    let frame = draw(&mut s, ToolId::Frame, (0.0, 400.0), (100.0, 500.0));
    assert_ne!(fill(&node(&s, frame)), red);
    let line = draw(&mut s, ToolId::Line, (0.0, 600.0), (100.0, 600.0));
    assert_ne!(node(&s, line).stroke.weight, 4.0);
    let made = id_of(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 700, "width": 10, "height": 10}))["id"]).unwrap();
    assert_ne!(fill(&node(&s, made)), red);
    // Undo takes a drawn shape away whole, defaults included.
    let drawn = draw(&mut s, ToolId::Polygon, (800.0, 0.0), (860.0, 60.0));
    ex(&mut s, "edit.undo", json!({}));
    assert!(s.doc().unwrap().node(drawn).is_none());
}

/// Setting defaults no longer overwrites what Copy properties copied.
#[test]
fn setting_defaults_leaves_the_copied_properties_alone() {
    let mut s = Session::new();
    let copied = id_of(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"]).unwrap();
    ex(&mut s, "node.set", json!({"id": copied, "props": {"fill": "#00FF00"}}));
    ex(&mut s, "edit.copyProperties", json!({"id": copied}));
    let source = styled(&mut s);
    ex(&mut s, "edit.setDefaultProperties", json!({"id": source}));
    let target = id_of(&ex(&mut s, "shape.ellipse", json!({"x": 0, "y": 100, "width": 50, "height": 50}))["id"]).unwrap();
    ex(&mut s, "edit.pasteProperties", json!({"ids": [target]}));
    assert_eq!(fill(&node(&s, target)), Some(Color::from_hex("#00FF00").unwrap()), "Paste properties pastes what was copied");
}

/// A text layer sets the defaults of new text (its font and fills, laid out at that size), and the
/// shapes' defaults stay as they were.
#[test]
fn new_text_takes_a_text_layers_defaults_and_shapes_keep_theirs() {
    let mut s = Session::new();
    let shape = styled(&mut s);
    ex(&mut s, "edit.setDefaultProperties", json!({"id": shape}));
    let text = id_of(&ex(&mut s, "text.create", json!({"text": "Title", "x": 0, "y": 200}))["id"]).unwrap();
    ex(&mut s, "node.set", json!({"id": text, "props": {"fontSize": 32, "fill": "#008000"}}));
    let set = ex(&mut s, "edit.setDefaultProperties", json!({"id": text}));
    assert_eq!(set["for"], "text");
    let id = draw(&mut s, ToolId::Text, (300.0, 300.0), (300.0, 300.0));
    let n = node(&s, id);
    let NodeKind::Text(t) = &n.kind else { panic!("a text layer") };
    assert_eq!(t.style.font_size, 32.0);
    assert_eq!(fill(&n), Some(Color::from_hex("#008000").unwrap()));
    assert!(n.height > 30.0, "laid out at the default size: {}", n.height);
    // The shapes' defaults are still the rectangle's.
    let rect = draw(&mut s, ToolId::Rectangle, (0.0, 500.0), (80.0, 560.0));
    assert_eq!(fill(&node(&s, rect)), Some(Color::from_hex("#FF0000").unwrap()));
}
