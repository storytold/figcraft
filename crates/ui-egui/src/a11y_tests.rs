//! Accessibility, driven through egui headless: the AccessKit tree names the custom-painted widgets
//! (tools, layer rows, number fields, tabs, the canvas), and keyboard navigation: F6 cycles the
//! regions, Tab moves within one, arrows and Enter pick a tool, the arrows walk the layer rows,
//! a menu opened from the keyboard takes it and Escape gives it back, Escape returns to the
//! canvas where shortcuts work again, and a pointer press ends keyboard mode.

use egui::accesskit::{Node, Role, Toggled};
use egui::{Event, Key, Modifiers};
use figcraft_doc::NodeId;
use figcraft_engine::{Session, ToolId};
use serde_json::json;

use crate::a11y::{Region, focused_name};
use crate::layers_drag_tests::Ui;

fn id(v: serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn press(u: &mut Ui, key: Key, modifiers: Modifiers) {
    u.frame(vec![
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers },
        Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers },
    ]);
}

fn f6(u: &mut Ui, back: bool) {
    press(u, Key::F6, if back { Modifiers::SHIFT } else { Modifiers::NONE });
}

fn name(u: &Ui) -> String {
    focused_name(&u.ctx)
}

fn region(u: &Ui) -> Region {
    u.app.ui.nav.region
}

/// Two rectangles, "Rect A" and "Rect B" (listed above A: it's on top), A selected.
fn setup() -> (Ui, NodeId, NodeId) {
    let mut s = Session::new();
    let a = id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 30})).unwrap());
    let b = id(s.execute("shape.rectangle", &json!({"x": 100, "y": 30, "width": 20, "height": 20})).unwrap());
    s.execute("object.rename", &json!({"ids": [a], "name": "Rect A"})).unwrap();
    s.execute("object.rename", &json!({"ids": [b], "name": "Rect B"})).unwrap();
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    let mut u = Ui::new(s);
    u.frame(vec![]);
    (u, a, b)
}

/// The nodes of the AccessKit tree a screen reader would get now.
fn tree(u: &mut Ui) -> Vec<Node> {
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    let out = u.frame_output(vec![]);
    out.platform_output.accesskit_update.expect("an AccessKit update").nodes.into_iter().map(|(_, n)| n).collect()
}

fn find<'a>(nodes: &'a [Node], role: Role, label: &str) -> &'a Node {
    nodes.iter().find(|n| n.role() == role && n.label() == Some(label)).unwrap_or_else(|| panic!("no {role:?} named {label:?}"))
}

#[test]
fn the_screen_reader_tree_names_tools_layers_fields_tabs_and_the_canvas() {
    let (mut u, ..) = setup();
    let nodes = tree(&mut u);
    // Regions are named containers.
    find(&nodes, Role::Toolbar, "Tools");
    find(&nodes, Role::Pane, "Layers");
    find(&nodes, Role::Pane, "Design");
    // Tools: toggle buttons with their key, the current one pressed.
    let mv = find(&nodes, Role::Button, "Move");
    assert_eq!(mv.toggled(), Some(Toggled::True));
    assert_eq!(mv.keyboard_shortcut(), Some("V"));
    assert_eq!(find(&nodes, Role::Button, "Rectangle").toggled(), Some(Toggled::False));
    assert_eq!(find(&nodes, Role::Button, "Shape tools").is_expanded(), Some(false));
    // Layer rows: tree items with their type, level and selection.
    let a = find(&nodes, Role::TreeItem, "Rect A");
    assert_eq!((a.level(), a.is_selected(), a.description()), (Some(1), Some(true), Some("Rectangle")));
    assert_eq!(find(&nodes, Role::TreeItem, "Rect B").is_selected(), Some(false));
    // Number fields: name and value.
    let w = find(&nodes, Role::SpinButton, "Width");
    assert_eq!((w.value(), w.numeric_value()), (Some("40"), Some(40.0)));
    assert_eq!(find(&nodes, Role::SpinButton, "Rotation").value(), Some("0°"));
    // Tabs and dropdowns.
    assert_eq!(find(&nodes, Role::Tab, "Design").is_selected(), Some(true));
    assert_eq!(find(&nodes, Role::Tab, "Prototype").is_selected(), Some(false));
    assert_eq!(find(&nodes, Role::ComboBox, "Zoom").is_expanded(), Some(false));
    // Icon buttons are named by their tooltip.
    find(&nodes, Role::Button, "Add new page");
    // The canvas describes the selection.
    let canvas = find(&nodes, Role::Group, "Canvas");
    assert_eq!(canvas.description(), Some("Rect A, Rectangle, 40 by 30, selected"));
}

#[test]
fn f6_cycles_the_regions_and_lands_on_the_active_tool_and_the_selected_row() {
    let (mut u, ..) = setup();
    assert_eq!(region(&u), Region::Canvas);
    assert!(!u.app.ui.nav.keyboard);
    f6(&mut u, false);
    assert_eq!(region(&u), Region::Properties);
    assert!(u.app.ui.nav.keyboard, "keyboard mode: the focus ring shows");
    assert!(!name(&u).is_empty(), "a control in the properties panel has the keyboard");
    f6(&mut u, false);
    assert_eq!((region(&u), name(&u)), (Region::Toolbar, "Move".to_string()));
    f6(&mut u, false);
    assert_eq!((region(&u), name(&u)), (Region::Layers, "Rect A".to_string()));
    f6(&mut u, false);
    assert_eq!(region(&u), Region::Canvas);
    assert_eq!(name(&u), "Canvas");
    // Shift+F6 goes back.
    f6(&mut u, true);
    assert_eq!(region(&u), Region::Layers);
    // The commands do the same (menus, the control channel, MCP).
    u.app.run("view.previousRegion", json!({}));
    u.frame(vec![]);
    assert_eq!(region(&u), Region::Toolbar);
    // Hidden panels are skipped.
    u.app.run("view.toggleUi", json!({"on": true}));
    u.frame(vec![]);
    f6(&mut u, false);
    assert_eq!(region(&u), Region::Canvas);
    f6(&mut u, false);
    assert_eq!(region(&u), Region::Canvas, "only the canvas is shown");
}

#[test]
fn tab_moves_between_the_properties_controls_and_wraps_without_leaving_the_panel() {
    let (mut u, a, _) = setup();
    f6(&mut u, false);
    let first = name(&u);
    let mut seen = vec![first.clone()];
    for _ in 0..200 {
        press(&mut u, Key::Tab, Modifiers::NONE);
        assert_eq!(region(&u), Region::Properties);
        if name(&u) == first {
            break;
        }
        seen.push(name(&u));
    }
    assert_eq!(name(&u), first, "Tab wraps round to the panel's first control");
    for want in ["Width", "Height", "Rotation", "Opacity"] {
        assert!(seen.iter().any(|n| n == want), "{want} not reached by Tab: {seen:?}");
    }
    // Shift+Tab steps back.
    press(&mut u, Key::Tab, Modifiers::SHIFT);
    assert_eq!(Some(&name(&u)), seen.last());
    // Tab moved the keyboard, not the selection.
    assert_eq!(u.app.session.selection(), vec![a]);
}

#[test]
fn enter_edits_a_focused_field_up_steps_it_and_tab_commits_and_moves_on() {
    let (mut u, a, _) = setup();
    f6(&mut u, false);
    for _ in 0..100 {
        if name(&u) == "Width" {
            break;
        }
        press(&mut u, Key::Tab, Modifiers::NONE);
    }
    assert_eq!(name(&u), "Width");
    press(&mut u, Key::Enter, Modifiers::NONE);
    u.frame(vec![]);
    // Typing replaces the value; Up steps it and the field keeps the keyboard (egui's own arrow
    // focus moves are off on the panels).
    u.frame(vec![Event::Text("64".into())]);
    press(&mut u, Key::ArrowUp, Modifiers::NONE);
    u.frame(vec![]);
    assert_eq!(u.app.session.doc().unwrap().node(a).unwrap().width, 65.0);
    assert!(crate::a11y::field_focused(&u.ctx), "still editing");
    press(&mut u, Key::Tab, Modifiers::NONE);
    u.frame(vec![]);
    assert_eq!(u.app.session.doc().unwrap().node(a).unwrap().width, 65.0, "the edited width is kept");
    assert_eq!(name(&u), "Height");
    assert_eq!(region(&u), Region::Properties);
}

#[test]
fn arrows_and_enter_pick_a_toolbar_tool() {
    let (mut u, ..) = setup();
    f6(&mut u, true);
    f6(&mut u, true);
    assert_eq!((region(&u), name(&u)), (Region::Toolbar, "Move".to_string()));
    // Move, its flyout button, Frame, its flyout button, Rectangle.
    for _ in 0..4 {
        press(&mut u, Key::ArrowRight, Modifiers::NONE);
    }
    assert_eq!(name(&u), "Rectangle");
    press(&mut u, Key::Enter, Modifiers::NONE);
    assert_eq!(u.app.session.tool.tool, ToolId::Rectangle);
    // Left wraps past the first control to the last; Home goes back to the first.
    press(&mut u, Key::Home, Modifiers::NONE);
    assert_eq!(name(&u), "Move");
    press(&mut u, Key::ArrowLeft, Modifiers::NONE);
    assert_ne!(name(&u), "Move");
    assert_eq!(region(&u), Region::Toolbar);
}

#[test]
fn escape_returns_to_the_canvas_where_shortcuts_and_tab_work_again() {
    let (mut u, a, b) = setup();
    f6(&mut u, true);
    f6(&mut u, true);
    assert_eq!(region(&u), Region::Toolbar);
    // A shortcut key while a toolbar button has the keyboard does nothing.
    press(&mut u, Key::R, Modifiers::NONE);
    assert_eq!(u.app.session.tool.tool, ToolId::Move);
    press(&mut u, Key::Escape, Modifiers::NONE);
    assert_eq!((region(&u), name(&u)), (Region::Canvas, "Canvas".to_string()));
    assert_eq!(u.app.session.selection(), vec![a], "leaving the toolbar doesn't deselect");
    // Tab on the canvas selects the next layer and keeps the keyboard there.
    press(&mut u, Key::Tab, Modifiers::NONE);
    assert_eq!(u.app.session.selection(), vec![b]);
    assert_eq!(name(&u), "Canvas");
    press(&mut u, Key::R, Modifiers::NONE);
    assert_eq!(u.app.session.tool.tool, ToolId::Rectangle);
}

#[test]
fn tab_on_the_canvas_selects_the_next_layer_without_keyboard_mode() {
    let (mut u, _, b) = setup();
    press(&mut u, Key::Tab, Modifiers::NONE);
    u.frame(vec![]);
    assert_eq!(u.app.session.selection(), vec![b]);
    assert!(u.ctx.memory(|m| m.focused()).is_none(), "Tab doesn't wander into the tab bar");
}

#[test]
fn arrows_walk_the_layer_rows_and_the_keyboard_follows_the_selection() {
    let (mut u, a, b) = setup();
    f6(&mut u, true);
    assert_eq!((region(&u), name(&u)), (Region::Layers, "Rect A".to_string()));
    // B is listed above A.
    press(&mut u, Key::ArrowUp, Modifiers::NONE);
    u.frame(vec![]);
    u.frame(vec![]);
    assert_eq!(u.app.session.selection(), vec![b]);
    assert_eq!(name(&u), "Rect B");
    press(&mut u, Key::ArrowDown, Modifiers::SHIFT);
    u.frame(vec![]);
    let mut sel = u.app.session.selection();
    sel.sort();
    assert_eq!(sel, vec![a, b], "Shift extends");
    // Nothing moved on the canvas.
    assert_eq!(u.app.session.doc().unwrap().node(a).unwrap().transform.translation().x, 0.0);
}

#[test]
fn a_menu_opened_from_the_keyboard_takes_it_and_escape_gives_it_back() {
    let (mut u, ..) = setup();
    f6(&mut u, false);
    for _ in 0..100 {
        if name(&u) == "Zoom" {
            break;
        }
        press(&mut u, Key::Tab, Modifiers::NONE);
    }
    assert_eq!(name(&u), "Zoom");
    press(&mut u, Key::Enter, Modifiers::NONE);
    u.frame(vec![]);
    u.frame(vec![]);
    assert_eq!(name(&u), "Zoom in", "the menu's first item has the keyboard");
    press(&mut u, Key::Escape, Modifiers::NONE);
    u.frame(vec![]);
    assert_eq!(name(&u), "Zoom");
    assert_eq!(region(&u), Region::Properties, "closing the menu stays in the panel");
}

#[test]
fn a_pointer_press_ends_keyboard_mode() {
    let (mut u, ..) = setup();
    f6(&mut u, false);
    assert!(u.app.ui.nav.keyboard);
    let p = u.app.canvas.rect.center();
    u.button(p, true);
    u.button(p, false);
    assert!(!u.app.ui.nav.keyboard);
    assert_eq!(region(&u), Region::Canvas);
}

#[test]
fn focus_region_from_the_control_channel() {
    let (mut u, ..) = setup();
    let ctx = u.ctx.clone();
    crate::control::handle(&mut u.app, &ctx, "ui.set", &json!({"focusRegion": "layers"})).unwrap();
    u.frame(vec![]);
    let v = crate::control::handle(&mut u.app, &ctx, "ui.inspect", &json!({})).unwrap();
    assert_eq!(v["focus"]["region"], "layers");
    assert_eq!(v["focus"]["keyboard"], true);
    assert_eq!(v["focus"]["control"], "Rect A");
    assert!(crate::control::handle(&mut u.app, &ctx, "ui.set", &json!({"focusRegion": "nowhere"})).is_err());
}

#[test]
fn reduce_motion_turns_egui_animations_off() {
    let (mut u, ..) = setup();
    assert!(u.ctx.global_style().animation_time > 0.0);
    u.app.run("pref.reduceMotion", json!({"on": true}));
    u.frame(vec![]);
    assert_eq!(u.ctx.global_style().animation_time, 0.0);
    assert_eq!(u.app.inspect()["reduceMotion"], true);
}

#[test]
fn a_tool_flyout_opened_from_the_keyboard_picks_a_tool() {
    let (mut u, ..) = setup();
    f6(&mut u, true);
    f6(&mut u, true);
    // Move, its flyout button, Frame, its flyout button, Rectangle, the shape tools' button.
    for _ in 0..5 {
        press(&mut u, Key::ArrowRight, Modifiers::NONE);
    }
    assert_eq!(name(&u), "Shape tools");
    press(&mut u, Key::Enter, Modifiers::NONE);
    u.frame(vec![]);
    u.frame(vec![]);
    assert_eq!(u.app.ui.tool_flyout, Some("shape"));
    assert_eq!(name(&u), "Rectangle", "the flyout's first tool has the keyboard");
    // egui moves the focus between a menu's items at the end of the frame.
    press(&mut u, Key::ArrowDown, Modifiers::NONE);
    u.frame(vec![]);
    assert_eq!(name(&u), "Line");
    press(&mut u, Key::Enter, Modifiers::NONE);
    u.frame(vec![]);
    u.frame(vec![]);
    assert_eq!(u.app.session.tool.tool, ToolId::Line);
    assert_eq!(u.app.ui.tool_flyout, None);
    assert_eq!((region(&u), name(&u)), (Region::Toolbar, "Shape tools".to_string()), "back on the flyout's button");
    // Escape closes an open flyout and stays in the toolbar.
    press(&mut u, Key::Enter, Modifiers::NONE);
    u.frame(vec![]);
    u.frame(vec![]);
    press(&mut u, Key::Escape, Modifiers::NONE);
    u.frame(vec![]);
    assert_eq!(u.app.ui.tool_flyout, None);
    assert_eq!((region(&u), name(&u)), (Region::Toolbar, "Shape tools".to_string()));
}

/// Every keyboard and naming path with the screen reader tree on: the node writers and the shared
/// state run under egui's context lock, and a nested context call there would deadlock (debug
/// builds turn it into a panic after 10 s, so this test fails instead of hanging).
#[test]
fn every_accessibility_path_runs_with_the_tree_on_without_nested_context_locks() {
    let (mut u, ..) = setup();
    u.ctx.enable_accesskit();
    u.app.run("pref.reduceMotion", json!({"on": true}));
    for _ in 0..5 {
        f6(&mut u, false);
        let out = u.frame_output(vec![]);
        assert!(out.platform_output.accesskit_update.is_some());
    }
    // Properties: Tab, a field edit, the zoom menu from the keyboard and back.
    while region(&u) != Region::Properties {
        f6(&mut u, false);
    }
    for _ in 0..100 {
        if name(&u) == "Zoom" {
            break;
        }
        press(&mut u, Key::Tab, Modifiers::NONE);
    }
    press(&mut u, Key::Enter, Modifiers::NONE);
    u.frame(vec![]);
    u.frame(vec![]);
    assert_eq!(name(&u), "Zoom in");
    press(&mut u, Key::ArrowDown, Modifiers::NONE);
    press(&mut u, Key::Escape, Modifiers::NONE);
    u.frame(vec![]);
    // Toolbar: a flyout; layers: the arrows; the canvas; a pointer press.
    while region(&u) != Region::Toolbar {
        f6(&mut u, false);
    }
    press(&mut u, Key::End, Modifiers::NONE);
    press(&mut u, Key::Home, Modifiers::NONE);
    while region(&u) != Region::Layers {
        f6(&mut u, false);
    }
    press(&mut u, Key::ArrowUp, Modifiers::NONE);
    u.frame(vec![]);
    press(&mut u, Key::Escape, Modifiers::NONE);
    assert_eq!(region(&u), Region::Canvas);
    let p = u.app.canvas.rect.center();
    u.button(p, true);
    u.button(p, false);
    let out = u.frame_output(vec![]);
    assert!(out.platform_output.accesskit_update.is_some());
}

#[test]
fn the_align_buttons_are_off_for_a_layer_alone_on_the_page() {
    let (mut u, a, b) = setup();
    let align_left = |nodes: &[Node]| {
        nodes.iter().find(|n| n.label().is_some_and(|l| l.starts_with("Align left"))).map(Node::is_disabled).expect("the Align left button")
    };
    assert!(align_left(&tree(&mut u)), "Rect A alone on the page: nothing to align to");
    u.app.run("select.set", json!({"ids": [a, b]}));
    assert!(!align_left(&tree(&mut u)), "two layers align to each other");
    let f = id(u.app.session.execute("frame.create", &json!({"x": 300, "y": 0, "width": 100, "height": 100})).unwrap());
    let inner = id(u.app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 10, "height": 10, "parent": f})).unwrap());
    u.app.run("select.set", json!({"ids": [inner]}));
    assert!(!align_left(&tree(&mut u)), "one in a frame aligns to it");
}
