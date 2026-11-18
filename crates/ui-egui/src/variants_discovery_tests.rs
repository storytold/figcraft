//! Combine as variants and Add variant where people look for them (#903), driven through egui
//! headless: the Design panel offers Combine for two or more main components and Add variant for
//! a lone one, and the canvas / layers context menu lists Combine only when it applies.

use egui::Pos2;
use figcraft_doc::{NodeId, NodeKind};
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;
use crate::live_tests::{flat, has_text, text_center};
use crate::present_tests::id;

const COMBINE: &str = "Combine as variants";

/// A session with lone main components named A, B and C side by side, and a frame. Returns
/// (session, [a, b, c], frame).
fn doc() -> (Session, [NodeId; 3], NodeId) {
    let mut s = Session::new();
    let mut comps = Vec::new();
    for (k, name) in ["A", "B", "C"].iter().enumerate() {
        let f = id(&s.execute("frame.create", &json!({"x": k * 160, "y": 0, "width": 120, "height": 40, "name": name})).unwrap());
        comps.push(id(&s.execute("component.create", &json!({"ids": [f]})).unwrap()));
    }
    let frame = id(&s.execute("frame.create", &json!({"x": 0, "y": 200, "width": 60, "height": 60})).unwrap());
    (s, [comps[0], comps[1], comps[2]], frame)
}

fn select(u: &mut Ui, ids: &[NodeId]) {
    u.app.session.execute("select.set", &json!({"ids": ids})).unwrap();
    u.frame(vec![]);
}

/// The text painted this frame.
fn shows(u: &mut Ui, s: &str) -> Option<Pos2> {
    let shapes = flat(&u.frame_output(vec![]).shapes);
    has_text(&shapes, s).then(|| text_center(&shapes, s))
}

fn click(u: &mut Ui, p: Pos2) {
    u.button(p, true);
    u.button(p, false);
}

fn set_of(u: &Ui, c: NodeId) -> Option<NodeId> {
    let doc = u.app.session.doc().unwrap();
    doc.node(c).and_then(|n| n.parent).filter(|p| doc.node(*p).is_some_and(|n| matches!(n.kind, NodeKind::ComponentSet { .. })))
}

fn undo_len(u: &Ui) -> usize {
    u.app.session.ds().unwrap().history.undo.len()
}

#[test]
fn the_design_panel_combines_two_selected_components() {
    let (s, [a, b, _], _) = doc();
    let mut u = Ui::new(s);
    select(&mut u, &[a, b]);
    let at = shows(&mut u, COMBINE).expect("the Design panel offers Combine as variants");
    let undo = undo_len(&u);
    click(&mut u, at);
    let set = set_of(&u, a).expect("combined into a set");
    assert_eq!(set_of(&u, b), Some(set));
    assert_eq!(u.app.session.selection(), vec![set]);
    assert_eq!(undo_len(&u), undo + 1);
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
    assert!(shows(&mut u, COMBINE).is_none(), "a set selected: nothing to combine");
}

#[test]
fn the_design_panel_offers_combine_only_when_it_applies() {
    // The cases are the engine's (`can_combine_variants`); this pins the panel to it.
    let (s, [a, ..], frame) = doc();
    let mut u = Ui::new(s);
    select(&mut u, &[a, frame]);
    assert!(shows(&mut u, COMBINE).is_none(), "a component and a frame");
}

#[test]
fn the_design_panel_adds_a_variant_to_a_lone_component() {
    let (s, [a, ..], _) = doc();
    let mut u = Ui::new(s);
    select(&mut u, &[a]);
    let at = shows(&mut u, "Add variant").expect("a lone main component offers Add variant");
    click(&mut u, at);
    let set = set_of(&u, a).expect("the component became a set");
    assert_eq!(u.app.session.doc().unwrap().node(set).unwrap().children.len(), 2);
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
}

#[test]
fn the_context_menu_lists_combine_for_two_selected_components() {
    let (s, [a, b, _], _) = doc();
    let mut u = Ui::new(s);
    // One component: no Combine, Create component as before.
    select(&mut u, &[a]);
    u.app.ui.context_menu = Some((Pos2::new(700.0, 300.0), vec![a]));
    assert!(shows(&mut u, COMBINE).is_none());
    assert!(shows(&mut u, "Create component").is_some());
    u.app.ui.context_menu = None;
    // Two components, right-clicked on one layer row (the menu gets only that row): judged on
    // the selection, which is what the command combines.
    select(&mut u, &[a, b]);
    u.app.ui.context_menu = Some((Pos2::new(700.0, 300.0), vec![a]));
    u.frame(vec![]);
    let at = shows(&mut u, COMBINE).expect("the context menu offers Combine as variants");
    click(&mut u, at);
    let set = set_of(&u, a).expect("combined into a set");
    assert_eq!(set_of(&u, b), Some(set));
    assert!(u.app.ui.context_menu.is_none(), "the menu closes");
}
