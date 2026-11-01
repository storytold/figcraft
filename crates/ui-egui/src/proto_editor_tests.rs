//! The interaction editor's conditionals and the presenter's hotspot hints, driven headless
//! through egui (the harness is `present_tests`').

use egui::{Pos2, Rect, pos2};
use figcraft_doc::proto::{Action, Expr, ExprOp};
use figcraft_doc::{NodeId, VariableValue};
use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::present_tests::{Ui, id};
use crate::proto_editor::{Rhs, Simple, Slot, VarPart, VarSlot, pick_variable, simple};

/// Frames A (400×300) and B, a button in A that navigates to B on click, and variables `count`
/// (number), `open` and `done` (booleans), `label` (text).
fn setup() -> (Session, NodeId, NodeId, NodeId) {
    let mut s = Session::new();
    let ex = |s: &mut Session, c: &str, p: Value| s.execute(c, &p).unwrap();
    let a = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 400, "height": 300, "name": "A"})));
    let b = id(&ex(&mut s, "frame.create", json!({"x": 500, "y": 0, "width": 400, "height": 300, "name": "B"})));
    let btn = id(&ex(&mut s, "shape.rectangle", json!({"x": 20, "y": 20, "width": 120, "height": 40, "parent": a})));
    ex(&mut s, "variable.createCollection", json!({"name": "State"}));
    for (name, kind, value) in
        [("count", "number", json!(0)), ("open", "boolean", json!(false)), ("done", "boolean", json!(true)), ("label", "string", json!("Hi"))]
    {
        ex(&mut s, "variable.create", json!({"collection": "State", "name": name, "type": kind, "value": value}));
    }
    ex(&mut s, "proto.interaction.add", json!({"id": btn, "dest": b}));
    (s, a, b, btn)
}

fn presenter(u: &Ui) -> Value {
    u.app.inspect()["presenter"].clone()
}

fn rect_of(v: &Value) -> Rect {
    let n = |i: usize| v[i].as_f64().unwrap() as f32;
    Rect::from_min_max(pos2(n(0), n(1)), pos2(n(2), n(3)))
}

fn click(u: &mut Ui, p: Pos2) {
    u.button(p, true);
    u.button(p, false);
}

#[test]
fn a_click_where_nothing_reacts_flashes_the_hotspots() {
    let (s, a, b, btn) = setup();
    let mut u = Ui::new(s);
    u.run("proto.present", json!({"start": a}));
    let p = presenter(&u);
    assert_eq!(p["hotspotHints"], json!(true), "on by default");
    let hot = p["hotspots"].as_array().cloned().unwrap_or_default();
    assert_eq!(hot.len(), 1, "{p}");
    assert_eq!(hot[0]["id"], json!(btn));
    let r = rect_of(&hot[0]["rect"]);
    // Below the button, inside A: nothing reacts there.
    let empty = pos2(r.center().x, r.max.y + r.height() * 2.0);
    click(&mut u, empty);
    assert_eq!(presenter(&u)["hotspotFlash"], json!(true));
    assert_eq!(presenter(&u)["current"], json!(a));
    u.frames(40, Vec::new);
    assert_eq!(presenter(&u)["hotspotFlash"], json!(false), "faded out after half a second");
    // The hotspot itself reacts instead.
    click(&mut u, r.center());
    assert_eq!(presenter(&u)["current"], json!(b));
    assert_eq!(presenter(&u)["hotspotFlash"], json!(false));
    // Turned off, nothing flashes; the option carries over.
    u.run("proto.presentOptions", json!({"hotspotHints": false}));
    assert_eq!(presenter(&u)["hotspotHints"], json!(false));
    click(&mut u, empty);
    assert_eq!(presenter(&u)["hotspotFlash"], json!(false));
    assert_eq!(u.app.inspect()["presentOptions"]["hotspotHints"], json!(false));
}

fn condition(u: &Ui, btn: NodeId, block: usize) -> String {
    let doc = u.app.session.doc().unwrap();
    let Some(Action::Conditional { blocks }) = doc.node(btn).unwrap().interactions[0].actions.get(1) else { panic!() };
    blocks[block].condition.as_ref().map(|c| c.display(&doc.variables)).unwrap_or_default()
}

/// Asserts the interaction editor shows conditional blocks and nested actions, restarts a
/// condition when a variable of another type is picked, ignores a right-hand-side variable pick
/// for a complex condition, and one undo restores the previous condition.
#[test]
fn the_interaction_editor_shows_conditionals_and_picks_their_variables() {
    let (mut s, _, b, btn) = setup();
    s.execute(
        "proto.action.add",
        &json!({"id": btn, "index": 0, "action": {"type": "conditional", "blocks": [
            {"expression": "count >= 3", "actions": [{"type": "navigate", "dest": b}, {"type": "setVariable", "variable": "count", "expression": "count + 1"}]},
            {"expression": "open and count < 2", "actions": []},
            {"actions": [{"type": "back"}]},
        ]}}),
    )
    .unwrap();
    s.execute("select.set", &json!({"ids": [btn]})).unwrap();
    let mut u = Ui::new(s);
    u.app.ui.right_tab = crate::RightTab::Prototype;
    crate::proto_panel::open_editor(&u.ctx, Some((btn, 0)));
    // The conditional open, then its nested set variable, then the else-if as an expression.
    let open = egui::Id::unique(("proto_open_action", btn, 0usize));
    for path in [vec![1usize], vec![1, 0, 1], vec![1, 2, 0]] {
        u.ctx.data_mut(|d| d.insert_temp(open, path));
        u.frames(3, Vec::new);
        assert_eq!(crate::proto_panel::editor(&u.ctx), Some((btn, 0)), "the editor stays open");
    }
    let vars = u.app.session.doc().unwrap().variables.clone();
    let var = |n: &str| vars.get(n).unwrap().id.clone();
    let at = |path: Vec<usize>, part| VarSlot { slot: Slot { node: btn, k: 0, path }, part };
    // Another variable of another type: the condition starts over from its value.
    pick_variable(&mut u.app, &at(vec![1], VarPart::Lhs(0)), &var("open"));
    assert_eq!(condition(&u, btn, 0), "open == false");
    // Compared with another variable.
    pick_variable(&mut u.app, &at(vec![1], VarPart::Rhs(0)), &var("done"));
    assert_eq!(condition(&u, btn, 0), "open == done");
    // Same type: the rest stays.
    pick_variable(&mut u.app, &at(vec![1], VarPart::Lhs(0)), &var("done"));
    assert_eq!(condition(&u, btn, 0), "done == done");
    // A complex condition can't take a right-hand variable (it's edited as an expression).
    pick_variable(&mut u.app, &at(vec![1], VarPart::Rhs(1)), &var("done"));
    assert_eq!(condition(&u, btn, 1), "open and (count < 2)");
    // A set variable keeps its expression for a variable of the same type, else starts over.
    pick_variable(&mut u.app, &at(vec![1, 0, 1], VarPart::SetVariable), &var("label"));
    let doc = u.app.session.doc().unwrap();
    let list = &doc.node(btn).unwrap().interactions[0].actions;
    assert_eq!(
        figcraft_engine::cmd::proto_actions::action_at(list, &[1, 0, 1]),
        Some(&Action::SetVariable { variable: var("label"), value: json!("Hi"), expr: None })
    );
    // Undo steps back through the picks.
    assert!(u.app.session.undo().unwrap());
    u.frames(3, Vec::new);
    assert_eq!(condition(&u, btn, 0), "done == done");
}

#[test]
fn the_builder_takes_variable_operator_value_conditions() {
    let lit = |v| Simple { var: "a".into(), op: ExprOp::Less, rhs: Rhs::Value(v) };
    for s in [
        lit(VariableValue::Number(3.0)),
        Simple { var: "a".into(), op: ExprOp::Or, rhs: Rhs::Variable("b".into()) },
        Simple { var: "a".into(), op: ExprOp::Not, rhs: Rhs::Nothing },
    ] {
        assert_eq!(simple(&s.expr()), Some(s.clone()), "{s:?}");
    }
    // Anything else is edited as an expression.
    let n = |x| Expr::Value(VariableValue::Number(x));
    for e in [
        Expr::var("a"),
        Expr::op(ExprOp::Add, vec![Expr::var("a"), n(1.0)]),
        Expr::op(ExprOp::Equals, vec![n(1.0), Expr::var("a")]),
        Expr::op(ExprOp::Equals, vec![Expr::var("a"), Expr::op(ExprOp::Negate, vec![n(1.0)])]),
        Expr::op(ExprOp::Equals, vec![Expr::var("a"), n(1.0), n(1.0)]),
        Expr::op(ExprOp::Not, vec![Expr::op(ExprOp::Not, vec![Expr::var("a")])]),
    ] {
        assert_eq!(simple(&e), None, "{e:?}");
    }
}

/// Asserts invalid typed conditions keep their draft and show errors without saving, a valid
/// one clears the error and saves, and Enter does not leak to shortcuts.
#[test]
fn typed_conditions_are_checked_and_errors_show_until_fixed() {
    let (mut s, _, b, btn) = setup();
    s.execute(
        "proto.action.add",
        &json!({"id": btn, "index": 0, "action": {"type": "conditional", "blocks": [
            {"expression": "count >= 3", "actions": [{"type": "navigate", "dest": b}]},
            {"expression": "open and count < 2", "actions": []},
        ]}}),
    )
    .unwrap();
    s.execute("select.set", &json!({"ids": [btn]})).unwrap();
    let mut u = Ui::new(s);
    u.app.ui.right_tab = crate::RightTab::Prototype;
    crate::proto_panel::open_editor(&u.ctx, Some((btn, 0)));
    u.ctx.data_mut(|d| d.insert_temp(egui::Id::unique(("proto_open_action", btn, 0usize)), vec![1usize]));
    u.frames(3, Vec::new);
    // The else if is complex: it shows as an expression field.
    let field = Slot { node: btn, k: 0, path: vec![1] }.id("proto_cond_expr").with(1usize);
    let rect = u.ctx.read_response(field.with("click")).expect("the expression field shows").rect;
    let type_in = |u: &mut Ui, text: &str| {
        // One click starts editing with the keyboard in the field.
        click(u, rect.center());
        let key = |k, command| egui::Event::Key {
            key: k,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers { command, ctrl: command, ..Default::default() },
        };
        u.frame(vec![key(egui::Key::A, true)]);
        u.frame(vec![egui::Event::Text(text.into())]);
        u.frame(vec![key(egui::Key::Enter, false)]);
        u.frames(2, Vec::new);
    };
    type_in(&mut u, "count +");
    assert_eq!(condition(&u, btn, 1), "open and (count < 2)", "nothing saved");
    let draft: Option<(String, String, String)> = u.ctx.data(|d| d.get_temp(field.with("draft")));
    assert_eq!(draft.map(|d| (d.1, d.2)), Some(("count +".to_string(), "Something is missing at the end".to_string())));
    type_in(&mut u, "count + 1");
    let draft: Option<(String, String, String)> = u.ctx.data(|d| d.get_temp(field.with("draft")));
    assert_eq!(draft.map(|d| d.2), Some("A condition must be true or false, not a number".to_string()));
    type_in(&mut u, "count == 1 or not open");
    assert_eq!(condition(&u, btn, 1), "(count == 1) or (not open)");
    assert_eq!(u.ctx.data(|d| d.get_temp::<(String, String, String)>(field.with("draft"))), None, "the error is gone");
    // Enter ended the field's editing without reaching the shortcuts (no vector editing of the
    // selected rectangle).
    assert!(matches!(u.app.session.doc().unwrap().node(btn).unwrap().kind, figcraft_doc::NodeKind::Rectangle));
}

#[test]
fn the_presenter_inspects_the_layer_under_the_pointer() {
    let (s, a, _, btn) = setup();
    let mut u = Ui::new(s);
    u.run("proto.present", json!({"start": a}));
    assert_eq!(presenter(&u)["inspect"], json!(false), "off by default");
    let r = rect_of(&presenter(&u)["hotspots"][0]["rect"]);
    u.run("proto.presentOptions", json!({"inspect": true}));
    u.frames(2, Vec::new);
    // The panel takes room: the prototype moves; hover the button where it is now.
    let r2 = rect_of(&presenter(&u)["hotspots"][0]["rect"]);
    assert!(r2.center().x < r.center().x, "{r:?} → {r2:?}");
    u.frame(vec![egui::Event::PointerMoved(r2.center())]);
    u.frames(1, Vec::new);
    assert_eq!(presenter(&u)["inspected"], json!(btn));
    // Elsewhere in the frame: the frame itself; outside it, the last one stays.
    u.frame(vec![egui::Event::PointerMoved(pos2(r2.center().x, r2.max.y + 30.0))]);
    assert_eq!(presenter(&u)["inspected"], json!(a));
    u.frame(vec![egui::Event::PointerMoved(pos2(5.0, 400.0))]);
    assert_eq!(presenter(&u)["inspected"], json!(a));
    assert_eq!(u.app.inspect()["presentOptions"]["inspect"], json!(true));
}

#[test]
fn deleting_a_prototype_interaction_removes_the_interaction_not_the_component() {
    let (s, _a, b, btn) = setup();
    let mut u = Ui::new(s);
    u.app.ui.right_tab = crate::RightTab::Prototype;
    u.run("select.set", json!({"ids": [btn]}));
    crate::proto_panel::open_editor(&u.ctx, Some((btn, 0)));
    u.frames(1, Vec::new);

    assert!(u.app.session.doc().unwrap().node(btn).is_some());
    assert_eq!(u.app.session.doc().unwrap().node(btn).unwrap().interactions.len(), 1);
    assert_eq!(crate::proto_panel::editor(&u.ctx), Some((btn, 0)));

    // Pressing Delete when an interaction is selected should delete the interaction, NOT the component.
    u.key(pos2(100.0, 100.0), egui::Key::Delete);

    assert!(u.app.session.doc().unwrap().node(btn).is_some(), "Bug #767 regression: component was deleted when deleting prototype interaction!");
    assert_eq!(u.app.session.doc().unwrap().node(btn).unwrap().interactions.len(), 0, "the interaction should be removed");
    assert_eq!(crate::proto_panel::editor(&u.ctx), None);

    // Re-add interaction and test Backspace as well.
    u.run("proto.interaction.add", json!({"id": btn, "dest": b}));
    assert_eq!(u.app.session.doc().unwrap().node(btn).unwrap().interactions.len(), 1);
    crate::proto_panel::open_editor(&u.ctx, Some((btn, 0)));
    u.frames(1, Vec::new);
    u.key(pos2(100.0, 100.0), egui::Key::Backspace);
    assert!(u.app.session.doc().unwrap().node(btn).is_some());
    assert_eq!(u.app.session.doc().unwrap().node(btn).unwrap().interactions.len(), 0);
    assert_eq!(crate::proto_panel::editor(&u.ctx), None);

    // When NO interaction is active, Delete deletes the component as normal.
    u.key(pos2(100.0, 100.0), egui::Key::Delete);
    assert!(u.app.session.doc().unwrap().node(btn).is_none(), "when no interaction is selected, Delete removes the layer");
}

/// Edit ▸ Delete from the menu bar does the same as the Delete key (#767).
#[test]
fn edit_delete_from_the_menu_removes_the_open_interaction_not_the_layer() {
    let (s, _a, _b, btn) = setup();
    let mut u = Ui::new(s);
    u.app.ui.right_tab = crate::RightTab::Prototype;
    u.run("select.set", json!({"ids": [btn]}));
    crate::proto_panel::open_editor(&u.ctx, Some((btn, 0)));
    u.frames(1, Vec::new);
    crate::menus::invoke(&mut u.app, "edit.delete");
    let doc = u.app.session.doc().unwrap();
    assert!(doc.node(btn).is_some_and(|n| n.interactions.is_empty()), "the interaction goes, the layer stays");
    assert_eq!(crate::proto_panel::editor(&u.ctx), None);
    crate::menus::invoke(&mut u.app, "edit.delete");
    assert!(u.app.session.doc().unwrap().node(btn).is_none(), "with no interaction open, Edit ▸ Delete deletes the layer");
}

#[test]
fn escape_closes_prototype_editor_without_deselecting_layer() {
    let (s, a, _b, btn) = setup();
    let mut u = Ui::new(s);
    u.app.ui.right_tab = crate::RightTab::Prototype;
    u.run("select.set", json!({"ids": [btn]}));
    crate::proto_panel::open_editor(&u.ctx, Some((btn, 0)));
    u.frames(1, Vec::new);

    assert_eq!(crate::proto_panel::editor(&u.ctx), Some((btn, 0)));
    assert_eq!(u.app.session.selection(), vec![btn]);

    // First Escape closes the interaction editor; layer remains selected.
    u.key(pos2(100.0, 100.0), egui::Key::Escape);
    assert_eq!(crate::proto_panel::editor(&u.ctx), None);
    assert_eq!(u.app.session.selection(), vec![btn], "layer remains selected");

    // Second Escape steps out to parent frame.
    u.key(pos2(100.0, 100.0), egui::Key::Escape);
    assert_eq!(u.app.session.selection(), vec![a], "steps out to parent frame");

    // Third Escape deselects all.
    u.key(pos2(100.0, 100.0), egui::Key::Escape);
    assert!(u.app.session.selection().is_empty(), "selection cleared on third escape");
}

/// After a click in the Pages list, Delete that removes an interaction ends the list's hold on the
/// keyboard: with the layer deselected afterwards, Delete doesn't take the page (#859).
#[test]
fn deleting_an_interaction_ends_the_pages_list_focus() {
    let (s, _a, _b, btn) = setup();
    let mut u = Ui::new(s);
    let first = u.app.session.page().unwrap();
    u.run("page.add", json!({}));
    u.run("page.select", json!({"id": first}));
    // As after a click on the page's row (layers_keys_tests clicks the real row).
    u.app.ui.pages_focus = u.app.session.ds().ok().map(|d| d.uid);
    u.app.ui.right_tab = crate::RightTab::Prototype;
    u.run("select.set", json!({"ids": [btn]}));
    crate::proto_panel::open_editor(&u.ctx, Some((btn, 0)));
    u.frames(1, Vec::new);
    u.key(pos2(100.0, 100.0), egui::Key::Delete);
    assert_eq!(u.app.session.doc().unwrap().node(btn).unwrap().interactions.len(), 0);
    assert!(u.app.ui.pages_focus.is_none());
    u.run("select.none", json!({}));
    u.key(pos2(100.0, 100.0), egui::Key::Delete);
    assert_eq!(u.app.session.doc().unwrap().pages.len(), 2, "the page stays");
}
