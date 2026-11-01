//! Arrow keys in the layers panel once a row has been clicked: up/down through the rows (Shift
//! extends), left/right to collapse, expand, and go to the parent or first child; a press on the
//! canvas gives arrows back to nudging. Delete after a click in the Pages list deletes the page.

use egui::{Event, Key, Modifiers, Pos2, pos2, vec2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

fn id(v: serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn key(u: &mut Ui, k: Key, shift: bool) {
    let modifiers = Modifiers { shift, ..Default::default() };
    u.frame(vec![
        Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers },
        Event::Key { key: k, physical_key: None, pressed: false, repeat: false, modifiers },
    ]);
}

fn sel(u: &Ui) -> Vec<NodeId> {
    u.app.session.selection()
}

/// Rows top to bottom: `c` (rectangle), `frame` (collapsed, holding `child`), `a` (rectangle).
fn setup() -> (Ui, NodeId, NodeId, NodeId, NodeId) {
    let mut s = Session::new();
    let a = id(s.execute("node.create", &json!({"type": "rectangle", "x": 0, "y": 0, "width": 10, "height": 10})).unwrap());
    let frame = id(s.execute("node.create", &json!({"type": "frame", "x": 50, "y": 0, "width": 100, "height": 100})).unwrap());
    let child = id(s.execute("node.create", &json!({"type": "ellipse", "parent": frame, "x": 10, "y": 10, "width": 10, "height": 10})).unwrap());
    let c = id(s.execute("node.create", &json!({"type": "rectangle", "x": 200, "y": 0, "width": 10, "height": 10})).unwrap());
    s.execute("select.none", &json!({})).unwrap();
    let mut u = Ui::new(s);
    u.app.ui.expanded.clear();
    u.frame(vec![]);
    (u, a, frame, child, c)
}

fn click_row(u: &mut Ui, id: NodeId) {
    let p = u.row_at(id);
    u.button(p, true);
    u.button(p, false);
    assert_eq!(sel(u), vec![id]);
}

#[test]
fn up_and_down_walk_the_rows_and_shift_extends() {
    let (mut u, a, frame, _, c) = setup();
    click_row(&mut u, c);
    key(&mut u, Key::ArrowDown, false);
    assert_eq!(sel(&u), vec![frame]);
    key(&mut u, Key::ArrowDown, false);
    assert_eq!(sel(&u), vec![a]);
    // The last row stays put.
    key(&mut u, Key::ArrowDown, false);
    assert_eq!(sel(&u), vec![a]);
    key(&mut u, Key::ArrowUp, true);
    key(&mut u, Key::ArrowUp, true);
    let mut got = sel(&u);
    got.sort();
    let mut want = vec![a, frame, c];
    want.sort();
    assert_eq!(got, want, "Shift+Up from the bottom row selects up to the top");
    // Back down with Shift shrinks the range towards the anchor.
    key(&mut u, Key::ArrowDown, true);
    let mut got = sel(&u);
    got.sort();
    let mut want = vec![a, frame];
    want.sort();
    assert_eq!(got, want);
    // Nothing moved on the canvas.
    let doc = u.app.session.doc().unwrap();
    assert_eq!(doc.node(c).unwrap().transform.translation().x, 200.0);
}

#[test]
fn left_and_right_open_close_and_move_between_levels() {
    let (mut u, _, frame, child, _) = setup();
    click_row(&mut u, frame);
    key(&mut u, Key::ArrowRight, false);
    assert!(u.app.ui.expanded.contains(&frame), "Right opens a closed row");
    assert_eq!(sel(&u), vec![frame]);
    key(&mut u, Key::ArrowRight, false);
    assert_eq!(sel(&u), vec![child], "Right on an open row goes to its first child");
    key(&mut u, Key::ArrowLeft, false);
    assert_eq!(sel(&u), vec![frame], "Left on a leaf goes to its parent");
    key(&mut u, Key::ArrowLeft, false);
    assert!(!u.app.ui.expanded.contains(&frame), "Left closes an open row");
    assert_eq!(sel(&u), vec![frame]);
}

#[test]
fn a_canvas_press_gives_arrows_back_to_nudging() {
    let (mut u, _, _, _, c) = setup();
    click_row(&mut u, c);
    assert!(u.app.ui.layers_focus);
    // Press on empty canvas (deselects), then select c from the canvas side and nudge it.
    let p = pos2(900.0, 700.0);
    u.button(p, true);
    u.button(p, false);
    assert!(!u.app.ui.layers_focus);
    u.app.run("select.set", json!({"ids": [c]}));
    key(&mut u, Key::ArrowRight, false);
    let doc = u.app.session.doc().unwrap();
    assert_eq!(doc.node(c).unwrap().transform.translation().x, 201.0);
}

/// The middle of `page`'s row in the Pages list, found by clicking down the list until it opens;
/// then back to `back`.
fn page_row(u: &mut Ui, page: NodeId, back: NodeId) -> Pos2 {
    for y in (90..400).step_by(4) {
        let p = pos2(150.0, y as f32);
        u.button(p, true);
        u.button(p, false);
        if u.app.session.page().unwrap() == page {
            u.app.session.execute("page.select", &json!({"id": back})).unwrap();
            u.frame(vec![]);
            // Rows are 32 high: aim at the middle, past the hit tolerance at its edge.
            return p + vec2(0.0, 12.0);
        }
    }
    panic!("no row for page {page:?}");
}

fn pages(u: &Ui) -> usize {
    u.app.session.doc().unwrap().pages.len()
}

#[test]
fn delete_after_clicking_a_page_deletes_it_without_asking() {
    // #859: Delete on a page picked in the Pages list said "nothing is selected".
    let (mut u, ..) = setup();
    let first = u.app.session.page().unwrap();
    let second = id(u.app.session.execute("page.add", &json!({})).unwrap());
    u.app.session.execute("page.select", &json!({"id": first})).unwrap();
    u.frame(vec![]);
    let row = page_row(&mut u, second, first);
    u.button(row, true);
    u.button(row, false);
    assert_eq!(u.app.session.page().unwrap(), second);
    key(&mut u, Key::Delete, false);
    // Gone at once, no confirmation: a toast says so and Undo brings it back.
    assert_eq!(pages(&u), 1);
    assert!(!u.app.session.doc().unwrap().pages.contains(&second));
    assert_eq!(u.app.ui.toast.as_ref().map(|t| t.0.as_str()), Some("Page deleted"));
    u.app.run("edit.undo", json!({}));
    assert_eq!(pages(&u), 2);
    // Backspace (the Mac's delete key) does the same.
    let row = page_row(&mut u, second, first);
    u.button(row, true);
    u.button(row, false);
    key(&mut u, Key::Backspace, false);
    assert_eq!(pages(&u), 1);
    // The last page stays.
    key(&mut u, Key::Delete, false);
    assert_eq!(pages(&u), 1);
}

#[test]
fn selected_layers_or_a_canvas_press_take_delete_back_from_the_pages_list() {
    let (mut u, a, ..) = setup();
    let first = u.app.session.page().unwrap();
    let second = id(u.app.session.execute("page.add", &json!({})).unwrap());
    u.frame(vec![]);
    let row = page_row(&mut u, first, second);
    u.button(row, true);
    u.button(row, false);
    assert_eq!(u.app.session.page().unwrap(), first);
    assert!(u.app.ui.pages_focus.is_some());
    // Layers selected after the click (Select all, say): Delete deletes them, not the page.
    u.app.run("select.set", json!({"ids": [a]}));
    key(&mut u, Key::Delete, false);
    assert_eq!(pages(&u), 2);
    assert!(u.app.session.doc().unwrap().node(a).is_none());
    // Nothing is selected now, but the list gave up the keyboard: a second Delete keeps the page.
    assert!(u.app.ui.pages_focus.is_none());
    key(&mut u, Key::Delete, false);
    assert_eq!(pages(&u), 2, "a second Delete keeps the page");
    // A press on the canvas gives Delete back to it.
    u.button(row, true);
    u.button(row, false);
    assert!(u.app.ui.pages_focus.is_some());
    let p = pos2(900.0, 700.0);
    u.button(p, true);
    u.button(p, false);
    assert!(u.app.ui.pages_focus.is_none());
    u.app.run("select.none", json!({}));
    key(&mut u, Key::Delete, false);
    assert_eq!(pages(&u), 2, "Delete with nothing selected on the canvas deletes no page");
}

#[test]
fn delete_deletes_no_page_once_the_pages_list_is_off_screen_or_another_file_is_open() {
    let (mut u, ..) = setup();
    let first = u.app.session.page().unwrap();
    let second = id(u.app.session.execute("page.add", &json!({})).unwrap());
    u.app.session.execute("page.select", &json!({"id": first})).unwrap();
    u.frame(vec![]);
    let row = page_row(&mut u, second, first);
    let click = |u: &mut Ui| {
        u.button(row, true);
        u.button(row, false);
        assert!(u.app.ui.pages_focus.is_some());
    };
    // The Assets tab, a collapsed Pages list, a collapsed left panel.
    let hide: [fn(&mut Ui, bool); 3] = [
        |u, on| u.app.ui.left_tab = if on { crate::LeftTab::Assets } else { crate::LeftTab::File },
        |u, on| u.app.ui.pages_open = !on,
        |u, on| u.app.ui.left_collapsed = on,
    ];
    for (k, hide) in hide.iter().enumerate() {
        click(&mut u);
        hide(&mut u, true);
        u.frame(vec![]);
        key(&mut u, Key::Delete, false);
        assert_eq!(pages(&u), 2, "case {k}: the list is off screen");
        hide(&mut u, false);
        u.frame(vec![]);
    }
    // Another file open: its pages are safe.
    click(&mut u);
    u.app.run("file.new", json!({}));
    u.app.run("page.add", json!({}));
    u.frame(vec![]);
    assert_eq!(pages(&u), 2);
    key(&mut u, Key::Delete, false);
    assert_eq!(pages(&u), 2, "the other file keeps its pages");
}
