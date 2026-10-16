//! Layers panel behaviour suite (the gestures), written from the public documentation of design
//! tools and driven through egui headless: clicking, Shift ranges and Cmd/Ctrl toggles, expanding
//! and collapsing, sweeping across the eyes and padlocks, renaming several layers, and the pages
//! list (dragging pages, dividers). Rows the documentation is silent on say so and pin today's
//! behaviour. What the gestures do to the document is in the engine's `panels_behaviour_tests`.

use egui::{Event, Id, Modifiers, Pos2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

fn id(v: serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

const SHIFT: Modifiers = Modifiers { alt: false, ctrl: false, shift: true, mac_cmd: false, command: false };
const CMD: Modifiers = Modifiers { alt: false, ctrl: true, shift: false, mac_cmd: false, command: true };
const ALT: Modifiers = Modifiers { alt: true, ctrl: false, shift: false, mac_cmd: false, command: false };

/// A click at `p` with `m` held, then a pause (so the next click is no double click).
fn click(u: &mut Ui, p: Pos2, m: Modifiers) {
    let button = |pressed| Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: m };
    u.frame_with(vec![Event::PointerMoved(p)], m);
    u.frame_with(vec![button(true)], m);
    u.frame_with(vec![button(false)], m);
    for _ in 0..30 {
        u.frame(vec![]);
    }
}

fn rect(s: &mut Session, name: &str, x: f64, parent: Option<NodeId>) -> NodeId {
    let mut p = json!({"x": x, "y": 0, "width": 20, "height": 20, "props": {"name": name}});
    if let Some(parent) = parent {
        p["parent"] = json!(parent);
    }
    id(s.execute("shape.rectangle", &p).unwrap())
}

/// Four rectangles `a`…`d` on the page (`d` on top: the panel's first row).
fn four() -> (Ui, [NodeId; 4]) {
    let mut s = Session::new();
    let ids = ["a", "b", "c", "d"].map(|n| rect(&mut s, n, 0.0, None));
    s.execute("select.none", &json!({})).unwrap();
    (Ui::new(s), ids)
}

fn sorted(mut v: Vec<NodeId>) -> Vec<NodeId> {
    v.sort();
    v
}

#[test]
fn clicking_rows_selects_by_the_documented_rules() {
    let (mut u, [a, b, c, d]) = four();
    let rows = [a, b, c, d].map(|i| u.row_at(i));
    let [ra, rb, rc, rd] = rows;
    // (row, modifiers, selection after, derivation)
    let steps: [(Pos2, Modifiers, Vec<NodeId>, &str); 5] = [
        (rc, Modifiers::NONE, vec![c], "a click on a layer's row selects it (on the canvas too)"),
        (ra, SHIFT, vec![a, b, c], "Shift-click selects every row between the first and the last"),
        (rd, CMD, vec![a, b, c, d], "Cmd/Ctrl-click adds a row that isn't next to the others"),
        (rb, CMD, vec![a, c, d], "silent: Cmd/Ctrl-click on a selected row takes it out (pinned)"),
        (rb, Modifiers::NONE, vec![b], "a plain click selects only that row"),
    ];
    for (p, m, after, why) in steps {
        click(&mut u, p, m);
        assert_eq!(sorted(u.app.session.selection()), sorted(after), "{why}");
    }
}

#[test]
fn expanding_collapsing_and_revealing() {
    let mut s = Session::new();
    let f = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 200, "name": "F"})).unwrap());
    let inner = rect(&mut s, "inner", 10.0, Some(f));
    let g = id(s.execute("object.group", &json!({"ids": [inner]})).unwrap());
    let h = id(s.execute("frame.create", &json!({"x": 300, "y": 0, "width": 100, "height": 100, "name": "H"})).unwrap());
    rect(&mut s, "h1", 10.0, Some(h));
    s.execute("select.none", &json!({})).unwrap();
    let mut u = Ui::new(s);
    u.app.ui.expanded.clear();
    u.frame(vec![]);
    // Alt-click on a disclosure arrow opens everything inside (silent, pinned); again closes it.
    let row = u.row_at(f);
    // The arrow sits at the row's left, after the panel's inset.
    let arrow = Pos2::new(74.0, row.y);
    click(&mut u, arrow, ALT);
    assert!(u.app.ui.expanded.contains(&f) && u.app.ui.expanded.contains(&g), "silent: Alt-click on an arrow expands every layer inside (pinned)");
    click(&mut u, arrow, ALT);
    assert!(!u.app.ui.expanded.contains(&f) && !u.app.ui.expanded.contains(&g), "silent: Alt-click again collapses them all (pinned)");
    // Selecting on the canvas opens the rows down to the layer (silent, pinned).
    u.app.run("select.set", json!({"ids": [inner]}));
    u.frame(vec![]);
    assert!(
        u.app.ui.expanded.contains(&f) && u.app.ui.expanded.contains(&g),
        "silent: a layer selected on the canvas is revealed in the panel (pinned)"
    );
    // Collapse layers: everything closes except the way to the selection.
    u.app.ui.expanded.insert(h);
    u.app.run("layers.collapseAll", json!({}));
    u.frame(vec![]);
    assert!(!u.app.ui.expanded.contains(&h), "Collapse layers closes the expanded layers");
    assert!(u.app.ui.expanded.contains(&f) && u.app.ui.expanded.contains(&g), "the selected layer's rows stay open");
    u.app.run("select.none", json!({}));
    u.app.run("layers.collapseAll", json!({}));
    u.frame(vec![]);
    assert!(u.app.ui.expanded.is_empty(), "with nothing selected every layer collapses");
}

#[test]
fn sweeping_across_eyes_and_padlocks_sets_several_layers() {
    let (mut u, [a, b, c, d]) = four();
    // The eye and padlock are at the row's right end (the left panel spans 56…296).
    let (eye, padlock) = (272.0, 250.0);
    let (rd, rb) = (u.row_at(d), u.row_at(b));
    let steps = u.app.session.undo_depth().0;
    let from = Pos2::new(eye, rd.y);
    u.frame(vec![Event::PointerMoved(from)]);
    u.drag_to(from, Pos2::new(eye, rb.y));
    u.button(Pos2::new(eye, rb.y), false);
    let doc = u.app.session.doc().unwrap();
    let hidden: Vec<bool> = [a, b, c, d].iter().map(|i| !doc.node(*i).unwrap().visible).collect();
    assert_eq!(hidden, [false, true, true, true], "dragging across the eyes hides every layer passed");
    assert_eq!(u.app.session.undo_depth().0, steps + 1, "silent: one undo step for the sweep (pinned)");
    for _ in 0..30 {
        u.frame(vec![]);
    }
    let from = Pos2::new(padlock, rd.y);
    u.frame(vec![Event::PointerMoved(from)]);
    u.drag_to(from, Pos2::new(padlock, rb.y));
    u.button(Pos2::new(padlock, rb.y), false);
    let doc = u.app.session.doc().unwrap();
    let locked: Vec<bool> = [a, b, c, d].iter().map(|i| doc.node(*i).unwrap().locked).collect();
    assert_eq!(locked, [false, true, true, true], "dragging across the padlocks locks every layer passed");
    // A plain click on an eye still toggles one layer.
    for _ in 0..30 {
        u.frame(vec![]);
    }
    let ra = u.row_at(a);
    click(&mut u, Pos2::new(eye, ra.y), Modifiers::NONE);
    assert!(!u.app.session.doc().unwrap().node(a).unwrap().visible, "a click on the eye hides that layer");
}

#[test]
fn renaming_several_layers_opens_the_rename_dialog() {
    let (mut u, [a, b, c, _d]) = four();
    // Cmd/Ctrl+R (F2, the context menu's Rename) with several layers selected.
    u.app.run("select.set", json!({"ids": [c, a, b]}));
    u.app.run("object.rename", json!({}));
    u.frame(vec![]);
    let dialog = u.app.ui.batch_rename.clone().expect("the Rename layers dialog opens for several layers");
    assert_eq!(dialog.ids, [a, b, c], "silent: the layers are numbered in stacking order, bottom first (pinned)");
    assert_eq!(u.app.ui.rename, None, "no single row is renamed in place");
    // Match empty: Rename to renames the whole name; Number up counts up.
    let mut d = dialog.clone();
    d.to = "Icon $n".into();
    u.app.ui.batch_rename = Some(d.clone());
    u.frame(vec![]);
    u.app.run("object.batchRename", crate::dialogs::batch_rename_params(&d));
    let doc = u.app.session.doc().unwrap();
    let names: Vec<&str> = [a, b, c].iter().map(|i| doc.node(*i).unwrap().name.as_str()).collect();
    assert_eq!(names, ["Icon 1", "Icon 2", "Icon 3"], "Rename to with Number up");
    // With a match, only the matched part changes.
    d.find = "Icon".into();
    d.to = "Logo".into();
    u.app.run("object.batchRename", crate::dialogs::batch_rename_params(&d));
    assert_eq!(u.app.session.doc().unwrap().node(a).unwrap().name, "Logo 1", "a match replaces only that part");
    // Escape closes the dialog; one layer still renames in its row.
    u.frame(vec![Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
    assert!(u.app.ui.batch_rename.is_none(), "Escape closes the dialog");
    u.app.run("select.set", json!({"ids": [a]}));
    u.app.run("object.rename", json!({}));
    u.frame(vec![]);
    assert_eq!(u.app.ui.rename.map(|r| r.id), Some(a), "a single layer renames in its row");
    assert!(u.app.ui.batch_rename.is_none());
}

/// The middle of page `page`'s row in the pages list, found by clicking down the list from page
/// `back` (each click opens the page under it); `back` is open again afterwards.
fn page_row(u: &mut Ui, page: NodeId, back: NodeId) -> Pos2 {
    u.app.session.execute("page.select", &json!({"id": back})).unwrap();
    u.frame(vec![]);
    for y in (90..400).step_by(4) {
        let p = Pos2::new(150.0, y as f32);
        u.button(p, true);
        u.button(p, false);
        u.app.ui.rename_page = None;
        if u.app.session.page().unwrap() == page {
            u.app.session.execute("page.select", &json!({"id": back})).unwrap();
            for _ in 0..30 {
                u.frame(vec![]);
            }
            // Rows are 32 high: aim at the middle.
            return p + egui::vec2(0.0, 12.0);
        }
    }
    panic!("no row for page {page:?}");
}

#[test]
fn pages_drag_to_reorder_and_dividers_open_nothing() {
    let mut s = Session::new();
    let p1 = s.page().unwrap();
    let p2 = id(s.execute("page.add", &json!({})).unwrap());
    let p3 = id(s.execute("page.add", &json!({})).unwrap());
    s.execute("page.select", &json!({"id": p1})).unwrap();
    let mut u = Ui::new(s);
    let (r1, r3) = (page_row(&mut u, p1, p2), page_row(&mut u, p3, p2));
    u.drag_to(r3, r1 - egui::vec2(0.0, 10.0));
    u.button(r1 - egui::vec2(0.0, 10.0), false);
    assert_eq!(u.app.session.doc().unwrap().pages, [p3, p1, p2], "a page dragged up the list moves there");
    // A divider between two pages: a line that opens nothing.
    let div = id(u.app.session.execute("page.add", &json!({"name": "-"})).unwrap());
    u.app.session.execute("page.move", &json!({"id": div, "index": 1})).unwrap();
    u.app.session.execute("page.select", &json!({"id": p1})).unwrap();
    for _ in 0..30 {
        u.frame(vec![]);
    }
    let (top, below) = (page_row(&mut u, p3, p2), page_row(&mut u, p1, p2));
    u.app.session.execute("page.select", &json!({"id": p1})).unwrap();
    u.frame(vec![]);
    let mid = Pos2::new(top.x, (top.y + below.y) / 2.0);
    click(&mut u, mid, Modifiers::NONE);
    assert_eq!(u.app.session.page().unwrap(), p1, "clicking a divider opens nothing");
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
}

/// The page opened by a click on each spot from just under the pages header down to `to`.
fn pages_down_to(u: &mut Ui, to: f32) -> Vec<NodeId> {
    let mut opened = Vec::new();
    for y in (190..to as i32).step_by(8) {
        click(u, Pos2::new(150.0, y as f32), Modifiers::NONE);
        opened.push(u.app.session.page().unwrap());
    }
    opened
}

#[test]
fn a_long_pages_list_scrolls_and_leaves_room_for_the_layers() {
    let mut s = Session::new();
    let first = s.page().unwrap();
    let rect = id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap());
    for _ in 0..59 {
        s.execute("page.add", &json!({})).unwrap();
    }
    let pages = s.doc().unwrap().pages.clone();
    let last = *pages.last().unwrap();
    s.execute("page.select", &json!({"id": first})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let mut u = Ui::new(s);
    // The layers list stays on screen under 60 pages.
    let layer = u.row_at(rect);
    // The wheel over the pages list scrolls it to its last page.
    let wheel = Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -400.0),
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
    };
    for _ in 0..10 {
        u.frame(vec![Event::PointerMoved(Pos2::new(150.0, 200.0)), wheel.clone()]);
    }
    for _ in 0..30 {
        u.frame(vec![]);
    }
    let opened = pages_down_to(&mut u, layer.y - 60.0);
    assert!(opened.contains(&last), "the last page can be reached");
    let top = pages.iter().position(|p| Some(p) == opened.first()).unwrap();
    assert!(top > 30, "the list scrolled past its first pages (top row: page {top})");
}

#[test]
fn a_page_dropped_below_the_pages_list_stays_where_it_was() {
    let mut s = Session::new();
    let first = s.page().unwrap();
    let rect = id(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap());
    let second = id(s.execute("page.add", &json!({})).unwrap());
    for _ in 0..58 {
        s.execute("page.add", &json!({})).unwrap();
    }
    let pages = s.doc().unwrap().pages.clone();
    s.execute("page.select", &json!({"id": first})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let mut u = Ui::new(s);
    let (row, layer) = (page_row(&mut u, second, first), u.row_at(rect));
    // Released over the layers, where pages scrolled out of view would be: no page row is there.
    u.drag_to(row, layer);
    u.button(layer, false);
    assert_eq!(u.app.session.doc().unwrap().pages, pages, "the pages keep their order");
}

/// A double click at `p`, with the screen reader tree checked on every frame: the platform adapter
/// (UI Automation on Windows) aborts the app when the focused node isn't in the tree.
fn double_click_checking_focus(u: &mut Ui, p: Pos2) {
    for _ in 0..30 {
        u.frame(vec![Event::PointerMoved(p)]);
    }
    let presses = [true, false, true, false].into_iter().map(Some);
    for (k, pressed) in presses.chain(std::iter::repeat_n(None, 30)).enumerate() {
        let events =
            pressed.map(|pressed| Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        let out = u.frame_output(events.into_iter().collect());
        let tree = out.platform_output.accesskit_update.expect("an AccessKit update");
        assert!(tree.nodes.iter().any(|(i, _)| *i == tree.focus), "frame {k}: the focused node {:?} isn't in the tree", tree.focus);
    }
}

#[test]
fn double_clicking_a_page_renames_it_with_the_focus_in_the_screen_reader_tree() {
    let mut s = Session::new();
    let p1 = s.page().unwrap();
    let p2 = id(s.execute("page.add", &json!({})).unwrap());
    let mut u = Ui::new(s);
    let (r1, r2) = (page_row(&mut u, p1, p2), page_row(&mut u, p2, p1));
    u.ctx.enable_accesskit();
    // Another page: the first click opens it, the second renames it.
    double_click_checking_focus(&mut u, r2);
    assert_eq!(u.app.session.page().unwrap(), p2);
    assert_eq!(u.app.ui.rename_page.as_ref().map(|r| r.0), Some(p2), "a double click renames the page");
    assert!(u.ctx.memory(|m| m.has_focus(Id::new(("pagename", p2)).with("te"))), "with the keyboard in its name");
    // The page that's open.
    u.app.ui.rename_page = None;
    u.ctx.memory_mut(|m| m.surrender_focus(Id::new(("pagename", p2)).with("te")));
    u.app.session.execute("page.select", &json!({"id": p1})).unwrap();
    for _ in 0..30 {
        u.frame(vec![]);
    }
    double_click_checking_focus(&mut u, r1);
    assert_eq!(u.app.ui.rename_page.as_ref().map(|r| r.0), Some(p1));
    assert!(u.app.ui.last_error.is_none(), "{:?}", u.app.ui.last_error);
}
