//! Boards in the UI: the toolbar's templates picker (pictures, a template or a table placed with
//! a click) and the table's part of the bar over the selection.

use egui::{Event, Modifiers, PointerButton, Pos2, pos2};
use figcraft_doc::{BoardItem, NodeId};
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;
use crate::live_tests::{flat, has_text, text_center};

fn click(u: &mut Ui, p: Pos2) {
    for pressed in [true, false] {
        u.frame(vec![Event::PointerMoved(p), Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
    }
    u.frame(vec![]);
}

fn board() -> Ui {
    let mut s = Session::new();
    s.execute("file.newBoard", &json!({})).unwrap();
    let mut u = Ui::new(s);
    u.frame(vec![]);
    u
}

/// The templates button: left of the Design panel's button at the board toolbar's right end.
fn templates_button(u: &Ui) -> Pos2 {
    let p = crate::toolbar::pill_rect(u.app.canvas.rect, crate::toolbar::bar_width(&u.app));
    pos2(p.max.x - 8.0 - 32.0 - 8.0 - 16.0, p.center().y)
}

fn count(u: &Ui, f: impl Fn(&BoardItem) -> bool) -> usize {
    u.app.session.doc().unwrap().nodes().filter(|n| n.board.as_ref().is_some_and(&f)).count()
}

#[test]
fn the_templates_picker_places_a_template_or_a_table() {
    let mut u = board();
    let at = templates_button(&u);
    click(&mut u, at);
    assert_eq!(u.app.ui.tool_flyout, Some("board_templates"));
    let shapes = flat(&u.frame_output(vec![]).shapes);
    for name in ["Retrospective", "Brainstorm", "Weekly planner", "Flow chart", "Kanban board", "Table"] {
        assert!(has_text(&shapes, name), "{name} in the picker");
    }
    let pictures = shapes.iter().filter(|s| matches!(s, egui::Shape::Mesh(m) if m.texture_id != egui::TextureId::default())).count();
    assert!(pictures >= 5, "each template has its picture ({pictures})");
    click(&mut u, text_center(&shapes, "Flow chart"));
    assert_eq!(u.app.ui.tool_flyout, None, "a pick closes the picker");
    assert_eq!(count(&u, |b| matches!(b, BoardItem::Connector(_))), 6);
    assert!(!u.app.session.selection().is_empty(), "what it placed is selected");
    // Again, for a table.
    click(&mut u, at);
    let shapes = flat(&u.frame_output(vec![]).shapes);
    click(&mut u, text_center(&shapes, "Table"));
    assert_eq!(count(&u, |b| *b == BoardItem::Table), 1);
    // Escape closes it.
    click(&mut u, at);
    u.frame(vec![Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
    assert_eq!(u.app.ui.tool_flyout, None);
}

#[test]
fn the_selection_bar_adds_and_removes_a_tables_rows_and_columns() {
    let mut u = board();
    let t = NodeId(u.app.session.execute("board.table", &json!({"x": 0, "y": 0, "rows": 2, "columns": 2})).unwrap()["id"].as_u64().unwrap());
    u.frame(vec![]);
    u.frame(vec![]);
    let size = |u: &Ui| {
        let doc = u.app.session.doc().unwrap();
        let n = doc.node(t).unwrap();
        let rows: Vec<_> = n.children.iter().filter(|c| !doc.node(**c).unwrap().child_layout.absolute).collect();
        (rows.len(), doc.node(*rows[0]).unwrap().children.len())
    };
    // Add row, remove row, add column, remove column: 30 apart, 8 in.
    for (i, want) in [(0, (3, 2)), (1, (2, 2)), (2, (2, 3)), (3, (2, 2))] {
        // The bar follows the table's middle as it widens.
        let r = u.ctx.memory(|m| m.area_rect(egui::Id::unique("board_bar"))).expect("the bar shows for a table");
        click(&mut u, pos2(r.min.x + 8.0 + i as f32 * 30.0 + 14.0, r.center().y));
        assert_eq!(size(&u), want, "cell {i}");
    }
}
