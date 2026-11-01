//! Widgets' hover styles: while the pointer is over a widget layer that has one, the canvas shows
//! it as a live preview (no undo step, nothing saved or synced); moving away, leaving the canvas or
//! any command puts the layer back, and a click on a hovered part acts on the document itself.

use serde_json::{Value, json};

use crate::components::tests::{ex, idv, node};
use crate::plugins_tests::{install, undo_len};
use crate::tools::{PointerEvent, PointerKind};
use crate::widgets_tests::{W, widget_manifest};
use crate::{Session, textpreview};
use figcraft_doc::NodeId;
use figcraft_geom::Point;

const BLUE: &str = "0D99FF";
const DARK: &str = "0B7FD6";

/// A widget whose button darkens and whose label turns white while hovered; the widget fades a
/// little. A click on the button counts.
fn button() -> Value {
    json!({"root": {"type": "frame", "name": "Hover", "layout": "horizontal", "padding": 10, "fill": "#FFFFFF", "hover": {"opacity": 0.9}, "children": [
        {"type": "rectangle", "name": "Button", "target": "go", "width": 40, "height": 20, "fill": format!("#{BLUE}"), "hover": {"fill": format!("#{DARK}"), "stroke": "#000000"}},
        {"type": "text", "name": "Label", "text": "Go", "color": "#111111", "hover": {"color": "#FFFFFF"}}]}})
}

fn session() -> (Session, NodeId) {
    let mut s = Session::new();
    let mut w = W::default();
    let body = w.render(&button());
    install(&mut s, widget_manifest("org.test.hover", "read"), &w.build(&body));
    let id = idv(&ex(&mut s, "widget.insert", json!({"plugin": "org.test.hover", "x": 50, "y": 60})));
    ex(&mut s, "select.set", json!({"ids": []}));
    (s, id)
}

fn fill(s: &Session, id: NodeId) -> String {
    node(s, id).fills.first().and_then(|p| p.solid_color()).map(|c| c.to_hex()).unwrap_or_default()
}

fn hover(s: &mut Session, p: Point) {
    s.pointer(&PointerEvent::at(PointerKind::Hover, p.x, p.y, 1.0)).unwrap();
}

fn hover_over(s: &mut Session, id: NodeId) {
    let p = centre(s, id);
    hover(s, p);
}

fn centre(s: &Session, id: NodeId) -> Point {
    s.doc().unwrap().world_bounds(id).unwrap().center()
}

#[test]
fn hovering_a_widget_shows_its_hover_styles_without_an_edit() {
    let (mut s, w) = session();
    let kids = node(&s, w).children.clone();
    let (btn, label) = (kids[0], kids[1]);
    let undo = undo_len(&s);
    hover_over(&mut s, btn);
    assert_eq!(fill(&s, btn), DARK, "the hovered button");
    assert!(!node(&s, btn).strokes.is_empty());
    assert!((node(&s, w).opacity - 0.9).abs() < 1e-6, "the widget the pointer is in");
    assert_eq!(fill(&s, label), "111111", "the label isn't under the pointer");
    let itself = s.ds().unwrap().document_itself().clone();
    assert_eq!(itself.node(btn).unwrap().fills[0].solid_color().unwrap().to_hex(), BLUE, "the document itself is unchanged");
    assert_eq!(undo_len(&s), undo, "no undo step");
    // Over the label: the label's style, not the button's.
    hover_over(&mut s, label);
    assert_eq!((fill(&s, btn), fill(&s, label)), (BLUE.into(), "FFFFFF".into()));
    // Away from the widget, and leaving the canvas: as it was.
    hover(&mut s, Point::new(-400.0, -400.0));
    assert_eq!((fill(&s, btn), fill(&s, label)), (BLUE.into(), "111111".into()));
    assert!((node(&s, w).opacity - 1.0).abs() < 1e-6);
    hover_over(&mut s, btn);
    s.pointer_left();
    assert_eq!(fill(&s, btn), BLUE);
    assert!(!textpreview::active(&s));
}

#[test]
fn a_click_on_a_hovered_part_edits_the_document_itself() {
    let (mut s, w) = session();
    let btn = node(&s, w).children[0];
    let at = centre(&s, btn);
    hover(&mut s, at);
    assert_eq!(fill(&s, btn), DARK);
    let undo = undo_len(&s);
    s.pointer(&PointerEvent::at(PointerKind::Down, at.x, at.y, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, at.x, at.y, 1.0)).unwrap();
    assert_eq!(undo_len(&s), undo + 1, "the click is one undo step");
    // The widget was redrawn from the document itself: no hover style was kept in it.
    let btn = node(&s, w).children[0];
    assert_eq!(fill(&s, btn), BLUE);
    ex(&mut s, "edit.undo", json!({}));
    let btn = node(&s, w).children[0];
    assert_eq!(fill(&s, btn), BLUE, "undo brings back the document, not a hovered one");
    // Hovering again shows it again.
    hover(&mut s, at);
    assert_eq!(fill(&s, btn), DARK);
}

#[test]
fn widget_hover_is_a_command_any_other_command_ends() {
    let (mut s, w) = session();
    let btn = node(&s, w).children[0];
    assert_eq!(ex(&mut s, "widget.hover", json!({"ids": [btn]}))["hovering"], true);
    assert_eq!(fill(&s, btn), DARK);
    ex(&mut s, "select.set", json!({"ids": [w]}));
    assert_eq!(fill(&s, btn), BLUE, "another command puts the document back first");
    // Layers without a hover style, and layers outside widgets, show nothing.
    let r = idv(&ex(&mut s, "shape.rectangle", json!({"x": 400, "y": 0, "width": 10, "height": 10})));
    assert_eq!(ex(&mut s, "widget.hover", json!({"ids": [r]}))["hovering"], false);
    assert_eq!(ex(&mut s, "widget.hover", json!({}))["hovering"], false);
    // A hover style damaged in a file is ignored.
    let doc = s.ds_mut().unwrap().doc_mut();
    doc.get_mut(btn).unwrap().plugin_data.insert("figcraft.widget.hover".into(), "{\"fill\": \"not a colour\"}".into());
    assert_eq!(ex(&mut s, "widget.hover", json!({"ids": [btn]}))["hovering"], false);
}

#[test]
fn a_widget_doesnt_replace_another_live_preview() {
    let (mut s, w) = session();
    let btn = node(&s, w).children[0];
    let other = idv(&ex(&mut s, "shape.rectangle", json!({"x": 400, "y": 0, "width": 10, "height": 10})));
    ex(&mut s, "text.preview", json!({"command": "node.set", "params": {"ids": [other], "props": {"opacity": 0.5}}}));
    hover_over(&mut s, btn);
    assert!((node(&s, other).opacity - 0.5).abs() < 1e-6, "the preview showing stays");
    assert_eq!(fill(&s, btn), BLUE);
}
