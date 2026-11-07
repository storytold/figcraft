//! The keyboard alone, driven through egui headless: F2 renames the selected layer (from the
//! canvas, a clicked row or a row with the keyboard, which gets it back afterwards), Shift+F10
//! (or the Menu key) opens the context menu at the focused row or below the canvas selection and the menu takes the
//! keyboard, and F8 draws a selection box the arrows move and size and Enter selects with.

use egui::{Event, Key, Modifiers};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
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

fn settle(u: &mut Ui) {
    for _ in 0..3 {
        u.frame(vec![]);
    }
}

fn name(u: &Ui, id: NodeId) -> String {
    u.app.session.doc().unwrap().node(id).unwrap().name.clone()
}

fn renaming(u: &Ui) -> Option<NodeId> {
    u.app.ui.rename.map(|r| r.id)
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

/// F6 to the layers panel, on the selected layer's row.
fn to_layers(u: &mut Ui) {
    for _ in 0..4 {
        press(u, Key::F6, Modifiers::NONE);
        if u.app.ui.nav.region == Region::Layers {
            return;
        }
    }
    panic!("F6 never reached the layers panel");
}

#[test]
fn f2_renames_from_the_canvas_and_from_a_row_with_the_keyboard() {
    let (mut u, a, _) = setup();
    press(&mut u, Key::F2, Modifiers::NONE);
    assert_eq!(renaming(&u), Some(a), "F2 on the canvas renames the selected layer");
    u.frame(vec![]);
    u.frame(vec![Event::Text("Alpha".into())]);
    press(&mut u, Key::Enter, Modifiers::NONE);
    assert_eq!(name(&u, a), "Alpha");
    assert_eq!(renaming(&u), None);
    // From a row with the keyboard (the canvas shortcuts are off there): the row gets it back.
    to_layers(&mut u);
    assert_eq!(focused_name(&u.ctx), "Alpha");
    press(&mut u, Key::F2, Modifiers::NONE);
    assert_eq!(renaming(&u), Some(a));
    u.frame(vec![]);
    u.frame(vec![Event::Text("One".into())]);
    press(&mut u, Key::Enter, Modifiers::NONE);
    settle(&mut u);
    assert_eq!(name(&u, a), "One");
    assert_eq!((u.app.ui.nav.region, focused_name(&u.ctx)), (Region::Layers, "One".to_string()), "the row has the keyboard again");
    // Escape cancels a rename and stays on the row (it doesn't jump to the canvas).
    press(&mut u, Key::F2, Modifiers::NONE);
    u.frame(vec![]);
    u.frame(vec![Event::Text("Dropped".into())]);
    press(&mut u, Key::Escape, Modifiers::NONE);
    settle(&mut u);
    assert_eq!(name(&u, a), "One");
    assert_eq!((u.app.ui.nav.region, focused_name(&u.ctx)), (Region::Layers, "One".to_string()));
}

#[test]
fn shift_f10_opens_the_context_menu_at_the_row_or_the_canvas_selection() {
    let (mut u, a, _) = setup();
    to_layers(&mut u);
    press(&mut u, Key::F10, Modifiers::SHIFT);
    settle(&mut u);
    let (pos, ids) = u.app.ui.context_menu.clone().expect("the menu opens");
    assert_eq!(ids, vec![a]);
    assert!(pos.x < u.app.canvas.rect.min.x, "at the row in the layers panel: {pos:?}");
    let item = focused_name(&u.ctx);
    assert!(!item.is_empty() && item != "Rect A", "the menu has the keyboard: {item:?}");
    press(&mut u, Key::Escape, Modifiers::NONE);
    settle(&mut u);
    assert!(u.app.ui.context_menu.is_none());
    assert_eq!(focused_name(&u.ctx), "Rect A", "closing gives the keyboard back to the row");
    // On the canvas: below the selection, and the menu takes the keyboard.
    press(&mut u, Key::Escape, Modifiers::NONE);
    assert_eq!(u.app.ui.nav.region, Region::Canvas);
    press(&mut u, Key::F10, Modifiers::SHIFT);
    settle(&mut u);
    let (pos, ids) = u.app.ui.context_menu.clone().expect("the menu opens on the canvas");
    assert_eq!(ids, vec![a]);
    let bottom = u.app.to_screen(0.0, 30.0).y;
    assert!(u.app.canvas.rect.contains(pos) && pos.y > bottom, "below the selection: {pos:?}");
    assert_ne!(focused_name(&u.ctx), "Canvas");
    press(&mut u, Key::Escape, Modifiers::NONE);
    settle(&mut u);
    assert!(u.app.ui.context_menu.is_none());
    assert_eq!(focused_name(&u.ctx), "Canvas");
    // After a click on a row (no keyboard mode), at that row too.
    let p = u.row_at(a);
    u.button(p, true);
    u.button(p, false);
    press(&mut u, Key::F10, Modifiers::SHIFT);
    settle(&mut u);
    let (pos, _) = u.app.ui.context_menu.clone().expect("the menu opens");
    assert!(pos.x < u.app.canvas.rect.min.x && (pos.y - p.y).abs() < 40.0, "{pos:?} near {p:?}");
}

/// The keyboard's Menu key (between the right Alt and Ctrl on many PC keyboards) opens the same
/// menu as Shift+F10, as it does in other apps.
#[test]
fn the_menu_key_opens_the_context_menu_too() {
    let (mut u, a, _) = setup();
    // On a layer row: at the row.
    to_layers(&mut u);
    press(&mut u, Key::Menu, Modifiers::NONE);
    settle(&mut u);
    let (pos, ids) = u.app.ui.context_menu.clone().expect("the menu opens at the row");
    assert_eq!(ids, vec![a]);
    assert!(pos.x < u.app.canvas.rect.min.x, "at the row in the layers panel: {pos:?}");
    press(&mut u, Key::Escape, Modifiers::NONE);
    settle(&mut u);
    assert!(u.app.ui.context_menu.is_none());
    // On the canvas: below the selection, and the menu takes the keyboard.
    press(&mut u, Key::Escape, Modifiers::NONE);
    assert_eq!(u.app.ui.nav.region, Region::Canvas);
    press(&mut u, Key::Menu, Modifiers::NONE);
    settle(&mut u);
    let (pos, ids) = u.app.ui.context_menu.clone().expect("the menu opens on the canvas");
    assert_eq!(ids, vec![a]);
    let bottom = u.app.to_screen(0.0, 30.0).y;
    assert!(u.app.canvas.rect.contains(pos) && pos.y > bottom, "below the selection: {pos:?}");
    assert_ne!(focused_name(&u.ctx), "Canvas", "the menu has the keyboard");
}

#[test]
fn f8_draws_a_selection_box_the_arrows_move_and_size() {
    let (mut u, a, b) = setup();
    press(&mut u, Key::F8, Modifiers::NONE);
    let [x0, y0, x1, y1] = u.app.ui.key_box.expect("a box around the selection");
    assert!(x0 < 0.0 && y0 < 0.0 && x1 > 40.0 && y1 > 30.0 && x1 < 100.0, "{x0} {y0} {x1} {y1}");
    // Screen readers hear the box and the keys.
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    let out = u.frame_output(vec![]);
    let nodes = out.platform_output.accesskit_update.expect("an AccessKit update").nodes;
    let canvas = nodes.iter().find(|(_, n)| n.label() == Some("Canvas")).map(|(_, n)| n.description().unwrap_or_default().to_string());
    assert!(canvas.as_deref().is_some_and(|d| d.starts_with("Selection box")), "{canvas:?}");
    // Shift+arrows size it from its far corner until it reaches B.
    for _ in 0..200 {
        if u.app.ui.key_box.is_some_and(|b| b[2] > 125.0) {
            break;
        }
        press(&mut u, Key::ArrowRight, Modifiers::SHIFT);
    }
    for _ in 0..200 {
        if u.app.ui.key_box.is_some_and(|b| b[3] > 55.0) {
            break;
        }
        press(&mut u, Key::ArrowDown, Modifiers::SHIFT);
    }
    assert_eq!(u.app.ui.key_box.map(|b| [b[0], b[1]]), Some([x0, y0]), "the start stays");
    assert_eq!(u.app.session.selection(), vec![a], "nothing selected before Enter");
    press(&mut u, Key::Enter, Modifiers::NONE);
    let mut sel = u.app.session.selection();
    sel.sort();
    assert_eq!(sel, {
        let mut v = vec![a, b];
        v.sort();
        v
    });
    assert!(u.app.ui.key_box.is_none());
    // Plain arrows move the whole box; Escape drops it and keeps the selection.
    press(&mut u, Key::F8, Modifiers::NONE);
    let before = u.app.ui.key_box.unwrap();
    press(&mut u, Key::ArrowRight, Modifiers::NONE);
    let after = u.app.ui.key_box.unwrap();
    assert!(after[0] > before[0] && (after[0] - before[0] - (after[2] - before[2])).abs() < 1e-9);
    assert_eq!(u.app.doc_x(a), 0.0, "the arrows don't nudge layers while the box shows");
    press(&mut u, Key::Escape, Modifiers::NONE);
    assert!(u.app.ui.key_box.is_none());
    assert_eq!(u.app.session.selection().len(), 2);
    // With nothing selected it starts in the middle of the view.
    u.app.run("select.none", json!({}));
    press(&mut u, Key::F8, Modifiers::NONE);
    let [x0, y0, x1, y1] = u.app.ui.key_box.unwrap();
    let c = u.app.to_page(u.app.canvas.rect.center());
    assert!(((x0 + x1) / 2.0 - c.x).abs() < 1e-6 && ((y0 + y1) / 2.0 - c.y).abs() < 1e-6);
    // A press on the canvas drops it.
    let p = u.app.canvas.rect.center();
    u.button(p, true);
    u.button(p, false);
    assert!(u.app.ui.key_box.is_none());
}

impl crate::FigApp {
    fn doc_x(&self, id: NodeId) -> f64 {
        self.session.doc().unwrap().node(id).unwrap().transform.translation().x
    }
}

/// Pick select (#1836): Tab turns the selection box into a pink outline on the selected layer (or
/// the page's top layer); Tab walks siblings, Enter and Shift+Enter go down and up the tree, Space
/// selects, and Esc or a click on the canvas leaves it.
#[test]
fn tab_switches_the_selection_box_to_pick_select() {
    let (mut u, a, b) = setup();
    // A frame with a child, on top of A and B in the layers list.
    let f = NodeId(u.app.run("frame.create", json!({"x": 200, "y": 0, "width": 80, "height": 80})).unwrap()["id"].as_u64().unwrap());
    let c = NodeId(u.app.run("shape.rectangle", json!({"x": 210, "y": 10, "width": 20, "height": 20, "parent": f})).unwrap()["id"].as_u64().unwrap());
    u.app.run("select.set", json!({"ids": [a]}));
    press(&mut u, Key::F8, Modifiers::NONE);
    press(&mut u, Key::Tab, Modifiers::NONE);
    assert!(u.app.ui.key_box.is_none(), "the box becomes the pick cursor");
    assert_eq!(u.app.picked(), Some(a), "on the selected layer");
    assert_eq!(u.app.session.selection(), vec![a], "nothing selected yet");
    // Tab walks the siblings as the Layers panel lists them (the frame on top, then B, then A),
    // wrapping; Shift+Tab goes the other way.
    press(&mut u, Key::Tab, Modifiers::NONE);
    assert_eq!(u.app.picked(), Some(f), "below A comes the top of the list");
    press(&mut u, Key::Tab, Modifiers::SHIFT);
    assert_eq!(u.app.picked(), Some(a));
    press(&mut u, Key::Tab, Modifiers::SHIFT);
    assert_eq!(u.app.picked(), Some(b));
    // Enter goes into a frame's top child, Shift+Enter back out; the page is never picked.
    press(&mut u, Key::Tab, Modifiers::SHIFT);
    assert_eq!(u.app.picked(), Some(f));
    press(&mut u, Key::Enter, Modifiers::NONE);
    assert_eq!(u.app.picked(), Some(c));
    press(&mut u, Key::Enter, Modifiers::SHIFT);
    press(&mut u, Key::Enter, Modifiers::SHIFT);
    assert_eq!(u.app.picked(), Some(f));
    assert_eq!(u.app.session.selection(), vec![a], "walking selects nothing");
    // Space selects the picked layer and ends pick select.
    press(&mut u, Key::Space, Modifiers::NONE);
    assert_eq!(u.app.session.selection(), vec![f]);
    assert!(u.app.picked().is_none());
    // The chord again while the box shows switches too; Escape ends it, the selection kept.
    press(&mut u, Key::F8, Modifiers::NONE);
    press(&mut u, Key::F8, Modifiers::NONE);
    assert!(u.app.ui.key_box.is_none());
    assert_eq!(u.app.picked(), Some(f));
    press(&mut u, Key::Escape, Modifiers::NONE);
    assert!(u.app.picked().is_none());
    assert_eq!(u.app.session.selection(), vec![f]);
    // Nothing selected: the pick starts on the page's top layer; a press on the canvas ends it.
    u.app.run("select.none", json!({}));
    press(&mut u, Key::F8, Modifiers::NONE);
    press(&mut u, Key::Tab, Modifiers::NONE);
    assert_eq!(u.app.picked(), Some(f));
    let p = u.app.canvas.rect.center();
    u.button(p, true);
    u.button(p, false);
    assert!(u.app.picked().is_none());
    // Picked in one document, the cursor stays behind when another comes to the front, even where
    // that one has a layer with the same id (each document numbers its layers from 1): Space
    // there selects nothing, and the stale pick is dropped.
    u.app.run("select.set", json!({"ids": [a]}));
    press(&mut u, Key::F8, Modifiers::NONE);
    press(&mut u, Key::Tab, Modifiers::NONE);
    assert_eq!(u.app.picked(), Some(a));
    // On another page of the same document the cursor isn't there either: Space selects nothing.
    let first_page = u.app.session.page().unwrap();
    u.app.run("page.add", json!({}));
    u.app.run("page.select", json!({"index": 1}));
    assert_ne!(u.app.session.page().unwrap(), first_page);
    assert_eq!(u.app.picked(), None);
    press(&mut u, Key::Space, Modifiers::NONE);
    assert!(u.app.session.selection().is_empty(), "nothing selected on the other page");
    assert!(u.app.ui.pick.is_none());
    u.app.run("page.select", json!({"index": 0}));
    u.app.run("select.set", json!({"ids": [a]}));
    press(&mut u, Key::F8, Modifiers::NONE);
    press(&mut u, Key::Tab, Modifiers::NONE);
    assert_eq!(u.app.picked(), Some(a));
    u.app.run("file.new", json!({}));
    for _ in 0..3 {
        u.app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}));
    }
    u.app.run("select.none", json!({}));
    assert!(u.app.session.doc().unwrap().node(a).is_some(), "the new document has a layer with the picked id");
    assert_eq!(u.app.picked(), None);
    press(&mut u, Key::Space, Modifiers::NONE);
    assert!(u.app.session.selection().is_empty(), "nothing selected in the new document");
    assert!(u.app.ui.pick.is_none());
}

/// A press on a prototype connection handle is a press on the canvas too: it ends pick select
/// like any other canvas press, rather than leaving the pink outline up.
#[test]
fn a_prototype_handle_press_ends_pick_select() {
    let (mut u, a, _) = setup();
    u.app.run("select.set", json!({"ids": [a]}));
    press(&mut u, Key::F8, Modifiers::NONE);
    press(&mut u, Key::Tab, Modifiers::NONE);
    assert_eq!(u.app.picked(), Some(a));
    u.app.ui.right_tab = crate::RightTab::Prototype;
    u.frame(vec![]);
    assert_eq!(u.app.picked(), Some(a), "still picking in the Prototype tab");
    let b = u.app.session.doc().unwrap().world_bounds(a).unwrap();
    let handle = u.app.to_screen(b.x1, (b.y0 + b.y1) / 2.0) + egui::vec2(11.0, 0.0);
    u.button(handle, true);
    u.button(handle, false);
    assert!(u.app.ui.pick.is_none(), "the handle press ends pick select");
}
