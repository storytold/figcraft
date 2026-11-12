//! Boards: templates (each placed as board objects in one undo step) and tables (made from auto
//! layout frames, edited by rows and columns, typed into with the text tools, synced live).

use figcraft_doc::{BoardItem, NodeId, NodeKind};
use serde_json::{Value, json};

use crate::board_tests::{board, id, node};
use crate::collab_live_tests::Room;
use crate::tools::{PointerEvent, PointerKind};
use crate::{EngineError, Session};

fn count(s: &Session, f: impl Fn(&BoardItem) -> bool) -> usize {
    s.doc().unwrap().nodes().filter(|n| n.board.as_ref().is_some_and(&f)).count()
}

fn ids(v: &Value, key: &str) -> Vec<NodeId> {
    v[key].as_array().unwrap().iter().map(|i| NodeId(i.as_u64().unwrap())).collect()
}

/// Rows of cell texts' characters.
fn texts(s: &Session, table: &Value) -> Vec<Vec<String>> {
    let chars = |v: &Value| match &node(s, NodeId(v.as_u64().unwrap())).kind {
        NodeKind::Text(td) => td.characters.clone(),
        _ => panic!("not text"),
    };
    table["cells"].as_array().unwrap().iter().map(|r| r.as_array().unwrap().iter().map(chars).collect()).collect()
}

/// A table's column lines (its laid-out-over children): their x and height.
fn lines(s: &Session, table: NodeId) -> Vec<(f64, f64)> {
    let doc = s.doc().unwrap();
    let t = doc.node(table).unwrap();
    t.children.iter().map(|c| doc.node(*c).unwrap()).filter(|n| n.child_layout.absolute).map(|n| (n.transform.translation().x, n.height)).collect()
}

#[test]
fn every_template_places_board_objects_in_one_undo_step() {
    let mut s = board();
    let list = s.execute("board.templates", &json!({})).unwrap();
    let names: Vec<&str> = list.as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap()).collect();
    assert_eq!(names, ["retrospective", "brainstorm", "weeklyPlanner", "flowChart", "kanban"]);
    for (i, name) in names.iter().enumerate() {
        let before = s.doc().unwrap().nodes().count();
        let r = s.execute("board.template", &json!({"name": name, "x": i as f64 * 2000.0, "y": 0})).unwrap();
        let made = ids(&r, "ids");
        assert!(!made.is_empty(), "{name}");
        assert_eq!(s.selection(), made, "{name}: what it placed is selected");
        let doc = s.doc().unwrap();
        // It sits where it was asked to, about its listed size.
        let b = doc.bounds_of(&made).unwrap();
        assert!((b.x0 - i as f64 * 2000.0).abs() < 1.0 && b.y0.abs() < 1.0, "{name} at {b:?}");
        let size = &list[i];
        assert!((b.width() - size["width"].as_f64().unwrap()).abs() < 2.0, "{name}: width {}", b.width());
        assert!(b.height() <= size["height"].as_f64().unwrap() + 2.0, "{name}: height {}", b.height());
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().nodes().count(), before, "{name}: one undo step");
        s.execute("edit.redo", &json!({})).unwrap();
    }
    // By name too, centred in the view without a place.
    let view = s.ds().unwrap().view.clone();
    let r = s.execute("board.template", &json!({"name": "Kanban board"})).unwrap();
    let b = s.doc().unwrap().bounds_of(&ids(&r, "ids")).unwrap();
    assert!((b.center().x - view.cx).abs() < 2.0, "{b:?} centred on {}", view.cx);
    assert!(s.execute("board.template", &json!({"name": "party"})).is_err());
    assert!(s.execute("board.template", &json!({})).is_err());
    let mut d = Session::new();
    d.execute("file.new", &json!({})).unwrap();
    assert!(matches!(d.execute("board.template", &json!({"name": "kanban"})), Err(EngineError::Disabled(..))), "boards only");
    assert!(matches!(d.execute("board.table", &json!({})), Err(EngineError::Disabled(..))), "boards only");
}

#[test]
fn templates_are_made_of_ordinary_board_objects() {
    let mut s = board();
    s.execute("board.template", &json!({"name": "retrospective", "x": 0, "y": 0})).unwrap();
    let doc = s.doc().unwrap();
    let sections: Vec<_> = doc.nodes().filter(|n| matches!(n.kind, NodeKind::Section { .. })).collect();
    assert_eq!(sections.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(), ["Went well", "To improve", "Next steps"]);
    for sec in &sections {
        let stickies = sec.children.iter().filter(|c| matches!(doc.node(**c).unwrap().board, Some(BoardItem::Sticky { .. }))).count();
        assert_eq!(stickies, 2, "{}", sec.name);
    }
    // No author on a template's stickies.
    assert!(doc.nodes().filter(|n| matches!(n.board, Some(BoardItem::Sticky { .. }))).all(|n| n.board.as_ref().unwrap().author().is_empty()));
    // The flow chart's connectors join its shapes, labelled at the decision.
    s.execute("board.template", &json!({"name": "flowChart", "x": 0, "y": 1000})).unwrap();
    let doc = s.doc().unwrap();
    let joined: Vec<_> = doc
        .nodes()
        .filter_map(|n| match &n.board {
            Some(BoardItem::Connector(c)) => Some((c.from.node().unwrap(), c.to.node().unwrap())),
            _ => None,
        })
        .collect();
    assert_eq!(joined.len(), 6);
    assert!(joined.iter().all(|(a, b)| matches!(doc.node(*a).unwrap().board, Some(BoardItem::Shape { .. }))
        && matches!(doc.node(*b).unwrap().board, Some(BoardItem::Shape { .. }))));
    assert_eq!(count(&s, |b| matches!(b, BoardItem::Label { .. })), 2);
    // The brainstorm's topic is joined to each of its four clusters.
    s.execute("board.template", &json!({"name": "brainstorm", "x": 3000, "y": 0})).unwrap();
    assert_eq!(count(&s, |b| matches!(b, BoardItem::Connector(_))), 10);
}

#[test]
fn a_table_is_auto_layout_frames_with_a_header_and_column_lines() {
    let mut s = board();
    let r = s.execute("board.table", &json!({"x": 0, "y": 0, "cells": [["Name", "Role"], ["Ada", "Lead"]], "rows": 3})).unwrap();
    let t = id(&r, "id");
    assert_eq!((r["rows"].as_u64(), r["columns"].as_u64()), (Some(3), Some(2)));
    assert_eq!(texts(&s, &r), [["Name", "Role"], ["Ada", "Lead"], ["", ""]]);
    assert_eq!(node(&s, t).board, Some(BoardItem::Table));
    assert_eq!(s.selection(), [t]);
    let table = node(&s, t);
    assert_eq!(table.width, 320.0);
    let rows: Vec<_> = table.children.iter().map(|c| node(&s, *c)).filter(|n| !n.child_layout.absolute).collect();
    assert_eq!(rows.len(), 3);
    assert!(rows[0].fills.len() == 1 && rows[1].fills.is_empty(), "the header row is tinted");
    assert!(rows[1].strokes.len() == 1 && rows[2].strokes.is_empty(), "a line under every row but the last");
    let cell = node(&s, rows[1].children[1]);
    assert_eq!((cell.width, cell.transform.translation().x), (160.0, 160.0), "cells share their row");
    assert_eq!(table.height, rows.iter().map(|r| r.height).sum::<f64>(), "the table hugs its rows");
    assert_eq!(lines(&s, t), [(159.5, table.height)], "the column line runs the table's height");
    // Caps and refusals.
    for bad in
        [json!({"rows": 51}), json!({"columns": 21}), json!({"rows": 0}), json!({"rows": 2.5}), json!({"cells": [["a", "b", "c"]], "columns": 2})]
    {
        assert!(s.execute("board.table", &bad).is_err(), "{bad}");
    }
    assert!(s.execute("board.table", &json!({"rows": 50, "columns": 20})).is_ok(), "the largest table");
    // It saves and reopens.
    let doc = s.doc().unwrap();
    let (back, _) = figcraft_format::from_json(&figcraft_format::to_json(doc).unwrap().to_string()).unwrap();
    assert_eq!(back.node(t).unwrap().board, Some(BoardItem::Table));
}

#[test]
fn table_edits_add_and_remove_rows_and_columns_keeping_text() {
    let mut s = board();
    let r = s.execute("board.table", &json!({"x": 0, "y": 0, "cells": [["a", "b"], ["c", "d"]]})).unwrap();
    let t = id(&r, "id");
    let edit = |s: &mut Session, p: Value| s.execute("board.tableEdit", &p);
    let r = edit(&mut s, json!({"id": t, "action": "addRow", "index": 1})).unwrap();
    assert_eq!(texts(&s, &r), [["a", "b"], ["", ""], ["c", "d"]]);
    let r = edit(&mut s, json!({"action": "addColumn", "index": 0})).unwrap();
    assert_eq!(texts(&s, &r), [["", "a", "b"], ["", "", ""], ["", "c", "d"]]);
    assert_eq!(node(&s, t).width, 480.0, "a new column as wide as the others");
    assert_eq!(lines(&s, t).len(), 2);
    let r = edit(&mut s, json!({"action": "removeRow", "index": 0})).unwrap();
    assert_eq!(texts(&s, &r), [["", "", ""], ["", "c", "d"]]);
    let first = node(&s, node(&s, t).children[0]);
    assert_eq!(first.fills.len(), 1, "the new first row is the header");
    let r = edit(&mut s, json!({"action": "removeColumn"})).unwrap();
    assert_eq!(texts(&s, &r), [["", ""], ["", "c"]]);
    assert_eq!(node(&s, t).width, 320.0);
    // From a cell inside it too.
    let cell_text = NodeId(r["cells"][1][1].as_u64().unwrap());
    let r = edit(&mut s, json!({"id": cell_text, "action": "addRow"})).unwrap();
    assert_eq!(r["rows"], 3);
    // Undo, one step each.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(node(&s, t).width, 480.0);
    // Refusals: the wrong action or index, the last row or column, beyond the caps, not a table.
    for p in [
        json!({"id": t, "action": "explode"}),
        json!({"id": t, "action": "addRow", "index": 9}),
        json!({"id": t, "action": "removeColumn", "index": 3}),
        json!({"id": t, "action": "addRow", "index": -1}),
        json!({"id": t}),
    ] {
        assert!(edit(&mut s, p.clone()).is_err(), "{p}");
    }
    let one = id(&s.execute("board.table", &json!({"rows": 1, "columns": 1})).unwrap(), "id");
    assert!(edit(&mut s, json!({"id": one, "action": "removeRow"})).is_err());
    assert!(edit(&mut s, json!({"id": one, "action": "removeColumn"})).is_err());
    let full = id(&s.execute("board.table", &json!({"rows": 50, "columns": 20})).unwrap(), "id");
    assert!(edit(&mut s, json!({"id": full, "action": "addRow"})).is_err());
    assert!(edit(&mut s, json!({"id": full, "action": "addColumn"})).is_err());
    let sticky = id(&s.execute("board.sticky", &json!({"x": 0, "y": 900})).unwrap(), "id");
    assert!(edit(&mut s, json!({"id": sticky, "action": "addRow"})).is_err());
}

#[test]
fn a_double_click_types_in_the_cell_and_its_row_grows_with_the_text() {
    // Measure with system fonts when the optional craft-fonts checkout is absent,
    // without depending on another test having scanned them first.
    figcraft_text::scan_system_fonts();
    let mut s = board();
    let r = s.execute("board.table", &json!({"x": 0, "y": 0})).unwrap();
    let t = id(&r, "id");
    let h0 = node(&s, t).height;
    s.execute("select.none", &json!({})).unwrap();
    // The middle cell (columns 160 wide, rows about 40 tall).
    let row_h = h0 / 3.0;
    s.pointer(&PointerEvent::at(PointerKind::DoubleClick, 240.0, row_h * 1.5, 1.0)).unwrap();
    let te = s.ds().unwrap().text_edit.clone().expect("typing in the cell");
    assert_eq!(te.id.0, r["cells"][1][1].as_u64().unwrap());
    crate::textedit::insert(
        &mut s,
        "A longer note
over
several lines",
    )
    .unwrap();
    crate::textedit::end(&mut s).unwrap();
    let h1 = node(&s, t).height;
    assert!(h1 > h0 + 10.0, "the row grew: {h0} → {h1}");
    assert!(lines(&s, t).iter().all(|(_, h)| (*h - h1).abs() < 1e-6), "column lines follow: {:?} vs {h1}", lines(&s, t));
    // A plain click picks the table whole.
    s.execute("select.none", &json!({})).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Down, 240.0, 10.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, 240.0, 10.0, 1.0)).unwrap();
    assert_eq!(s.selection(), [t]);
}

#[test]
fn tables_and_templates_sync_in_live_sessions() {
    let s = board();
    let mut room = Room::new(s.doc().unwrap().clone(), 2, 11);
    let t = id(&room.s(0).execute("board.table", &json!({"x": 0, "y": 0})).unwrap(), "id");
    room.s(1).execute("board.template", &json!({"name": "flowChart", "x": 0, "y": 600})).unwrap();
    room.settle();
    room.check();
    room.s(1).execute("board.tableEdit", &json!({"id": t, "action": "addColumn"})).unwrap();
    room.settle();
    room.check();
    let doc = room.server.document();
    assert_eq!(doc.node(t).unwrap().width, 640.0);
    assert_eq!(doc.nodes().filter(|n| matches!(n.board, Some(BoardItem::Connector(_)))).count(), 6);
}

#[test]
fn template_and_table_commands_survive_junk() {
    let junk = [
        json!({}),
        json!(null),
        json!([1]),
        json!({"name": 7, "x": "NaN", "y": 1e308, "rows": "x", "columns": -1, "cells": 5}),
        json!({"name": "kanban", "x": -1e308, "y": 1e308, "rows": 1e308, "columns": 0.5, "cells": [[1, 2]]}),
        json!({"name": "\u{0}", "cells": [["a"], "b"], "rows": 50, "columns": 20}),
        json!({"id": "x", "action": 5, "index": 1e308}),
        json!({"id": -1, "action": "addRow", "index": -1}),
        json!({"id": 99_999, "action": "removeColumn", "index": "NaN"}),
        json!({"id": 2, "action": "addColumn", "index": 0}),
    ];
    for cmd in ["board.template", "board.templates", "board.table", "board.tableEdit"] {
        for j in &junk {
            let mut s = board();
            let t = s.execute("board.table", &json!({"x": 0, "y": 0})).unwrap()["id"].clone();
            let _ = s.execute("board.template", &json!({"name": "brainstorm"}));
            let mut j = j.clone();
            if j.get("action").is_some_and(|a| a == "addColumn") {
                j["id"] = t;
            }
            if let Err(EngineError::Internal { cmd, msg }) = s.execute(cmd, &j) {
                panic!("{cmd} panicked on {j}: {msg}");
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Resizing a table (#2172): a height change goes into its rows, never into blank space.

fn drag(s: &mut Session, from: (f64, f64), to: (f64, f64)) {
    s.pointer(&PointerEvent::at(PointerKind::Down, from.0, from.1, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, (from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Move, to.0, to.1, 1.0)).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::Up, to.0, to.1, 1.0)).unwrap();
}

/// A selected 3 x 3 table at 0,0 (480 x 122: a line under all but the last row adds 1 to each), with `texts` in the first row.
fn table_3x3(texts: [&str; 3]) -> (Session, NodeId) {
    let mut s = board();
    let r = s.execute("board.table", &json!({"x": 0, "y": 0, "cells": [texts, ["", "", ""], ["", "", ""]]})).unwrap();
    (s, id(&r, "id"))
}

fn rows_of(s: &Session, t: NodeId) -> Vec<NodeId> {
    let doc = s.doc().unwrap();
    doc.node(t).unwrap().children.iter().copied().filter(|c| !node(s, *c).child_layout.absolute).collect()
}

fn row_heights(s: &Session, t: NodeId) -> Vec<f64> {
    rows_of(s, t).into_iter().map(|r| node(s, r).height).collect()
}

fn column_widths(s: &Session, t: NodeId) -> Vec<f64> {
    let r = rows_of(s, t)[0];
    node(s, r).children.iter().map(|c| node(s, *c).width).collect()
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.6
}

fn all_near(got: &[f64], want: &[f64]) -> bool {
    got.len() == want.len() && got.iter().zip(want).all(|(a, b)| near(*a, *b))
}

fn is_hug(s: &Session, t: NodeId) -> bool {
    node(s, t).child_layout.sizing_v == figcraft_doc::Sizing::Hug
}

#[test]
fn a_bottom_drag_grows_only_the_last_row() {
    // The programmatic route.
    let (mut s, t) = table_3x3(["Alpha", "", ""]);
    s.execute("node.set", &json!({"id": t, "props": {"height": 272}})).unwrap();
    assert!(all_near(&row_heights(&s, t), &[41.0, 41.0, 190.0]), "{:?}", row_heights(&s, t));
    let n = node(&s, t);
    assert!(near(n.width, 480.0) && near(n.height, 272.0), "{}x{}", n.width, n.height);
    assert!(is_hug(&s, t), "still hugging its rows");
    assert!(all_near(&column_widths(&s, t), &[160.0; 3]));
    assert!(lines(&s, t).iter().all(|(_, h)| near(*h, 272.0)), "{:?}", lines(&s, t));
    // The pointer route: the bottom edge dragged down by 150.
    let (mut s, t) = table_3x3(["Alpha", "", ""]);
    drag(&mut s, (240.0, 122.0), (240.0, 272.0));
    assert!(all_near(&row_heights(&s, t), &[41.0, 41.0, 190.0]), "{:?}", row_heights(&s, t));
    assert!(near(node(&s, t).height, 272.0) && near(node(&s, t).width, 480.0));
    assert!(is_hug(&s, t));
    assert!(all_near(&column_widths(&s, t), &[160.0; 3]));
    assert!(lines(&s, t).iter().all(|(_, h)| near(*h, 272.0)), "{:?}", lines(&s, t));
    // The text keeps its size.
    let row0 = rows_of(&s, t)[0];
    let cell = node(&s, node(&s, row0).children[0]);
    let NodeKind::Text(td) = &node(&s, cell.children[0]).kind else { panic!("text") };
    assert_eq!(td.style.font_size, 14.0);
}

#[test]
fn a_corner_drag_shares_the_change_over_every_row_and_column() {
    let (mut s, t) = table_3x3(["", "", ""]);
    drag(&mut s, (480.0, 122.0), (570.0, 182.0));
    let n = node(&s, t);
    assert!(near(n.width, 570.0) && near(n.height, 182.0), "{}x{}", n.width, n.height);
    assert!(all_near(&row_heights(&s, t), &[61.0, 61.0, 60.0]), "{:?}", row_heights(&s, t));
    assert!(all_near(&column_widths(&s, t), &[190.0; 3]), "{:?}", column_widths(&s, t));
    let xs: Vec<f64> = lines(&s, t).iter().map(|l| l.0).collect();
    assert!(all_near(&xs, &[189.5, 379.5]), "{xs:?}");
    assert!(is_hug(&s, t));
    // The same through `transform.resize`.
    let (mut s, t) = table_3x3(["", "", ""]);
    s.execute("transform.resize", &json!({"ids": [t], "width": 570, "height": 182})).unwrap();
    assert!(all_near(&row_heights(&s, t), &[61.0, 61.0, 60.0]), "{:?}", row_heights(&s, t));
    assert!(all_near(&column_widths(&s, t), &[190.0; 3]), "{:?}", column_widths(&s, t));
}

#[test]
fn a_side_drag_changes_no_row() {
    let (mut s, t) = table_3x3(["", "", ""]);
    s.execute("node.set", &json!({"id": t, "props": {"width": 680}})).unwrap();
    assert!(all_near(&row_heights(&s, t), &[41.0, 41.0, 40.0]), "{:?}", row_heights(&s, t));
    assert!(near(node(&s, t).height, 122.0) && near(node(&s, t).width, 680.0));
    assert!(is_hug(&s, t));
    // A pointer drag of the right edge.
    let (mut s, t) = table_3x3(["", "", ""]);
    drag(&mut s, (480.0, 60.0), (680.0, 60.0));
    assert!(all_near(&row_heights(&s, t), &[41.0, 41.0, 40.0]), "{:?}", row_heights(&s, t));
    assert!(near(node(&s, t).height, 122.0) && near(node(&s, t).width, 680.0));
    assert!(is_hug(&s, t));
}

#[test]
fn a_table_never_shrinks_below_its_cells() {
    let (mut s, t) = table_3x3(["Alpha", "Bravo longer text", "Charlie"]);
    drag(&mut s, (480.0, 122.0), (50.0, 32.0));
    let n = node(&s, t);
    assert!(near(n.width, 120.0), "width {}", n.width);
    assert!(all_near(&column_widths(&s, t), &[40.0; 3]), "{:?}", column_widths(&s, t));
    let rows = row_heights(&s, t);
    assert!(rows[0] > 40.0, "row 1 holds its wrapped text: {rows:?}");
    assert!(near(rows[1], 41.0) && near(rows[2], 40.0), "{rows:?}");
    assert!(near(n.height, rows.iter().sum::<f64>()), "{} vs {rows:?}", n.height);
    assert!(is_hug(&s, t));
}

#[test]
fn a_resized_table_still_grows_with_its_text_and_undoes_in_one_step() {
    // Text measures only once system fonts are scanned (without craft-fonts), so scan them here
    // without depending on another test having scanned them first.
    figcraft_text::scan_system_fonts();
    let (mut s, t) = table_3x3(["", "", ""]);
    drag(&mut s, (240.0, 122.0), (240.0, 272.0));
    assert!(near(node(&s, t).height, 272.0));
    // Typing in a last-row cell: the table grows once the text needs more than 190.
    s.execute("select.none", &json!({})).unwrap();
    s.pointer(&PointerEvent::at(PointerKind::DoubleClick, 80.0, 200.0, 1.0)).unwrap();
    assert!(s.ds().unwrap().text_edit.is_some(), "typing in the last row's cell");
    let long = (0..12).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
    crate::textedit::insert(&mut s, &long).unwrap();
    crate::textedit::end(&mut s).unwrap();
    assert!(node(&s, t).height > 272.0, "grew with its text: {}", node(&s, t).height);
    assert!(is_hug(&s, t));
    // Undo: the typing, then the resize, to the table as made.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(near(node(&s, t).height, 272.0), "{}", node(&s, t).height);
    s.execute("edit.undo", &json!({})).unwrap();
    let n = node(&s, t);
    assert!(near(n.width, 480.0) && near(n.height, 122.0), "{}x{}", n.width, n.height);
    assert!(all_near(&row_heights(&s, t), &[41.0, 41.0, 40.0]), "{:?}", row_heights(&s, t));
}

#[test]
fn table_resizes_sync_in_live_sessions() {
    let s = board();
    let mut room = Room::new(s.doc().unwrap().clone(), 2, 12);
    let t = id(&room.s(0).execute("board.table", &json!({"x": 0, "y": 0})).unwrap(), "id");
    room.settle();
    room.check();
    room.s(0).execute("node.set", &json!({"id": t, "props": {"height": 272}})).unwrap();
    room.settle();
    room.check();
    let doc = room.server.document();
    let heights: Vec<f64> =
        doc.node(t).unwrap().children.iter().map(|c| doc.node(*c).unwrap()).filter(|n| !n.child_layout.absolute).map(|n| n.height).collect();
    assert!(all_near(&heights, &[41.0, 41.0, 190.0]), "{heights:?}");
    assert!(near(doc.node(t).unwrap().height, 272.0));
    assert_eq!(doc.node(t).unwrap().child_layout.sizing_v, figcraft_doc::Sizing::Hug);
}

#[test]
fn resizing_a_table_survives_junk_and_saves_the_rows() {
    for h in [json!(1e308), json!(-5), json!("NaN"), json!(null), json!(0)] {
        let (mut s, t) = table_3x3(["", "", ""]);
        if let Err(EngineError::Internal { cmd, msg }) = s.execute("node.set", &json!({"id": t, "props": {"height": h}})) {
            panic!("{cmd} panicked on {h}: {msg}");
        }
        for p in [json!({"ids": [t], "width": h, "height": h}), json!({"ids": [t], "height": h}), json!({"ids": [t], "width": h})] {
            if let Err(EngineError::Internal { cmd, msg }) = s.execute("transform.resize", &p) {
                panic!("{cmd} panicked on {p}: {msg}");
            }
        }
        let n = node(&s, t);
        assert!(n.width.is_finite() && n.height.is_finite(), "{h}: {}x{}", n.width, n.height);
    }
    let (mut s, t) = table_3x3(["", "", ""]);
    s.execute("node.set", &json!({"id": t, "props": {"height": 272}})).unwrap();
    let doc = s.doc().unwrap();
    let (back, _) = figcraft_format::from_json(&figcraft_format::to_json(doc).unwrap().to_string()).unwrap();
    let heights: Vec<f64> =
        back.node(t).unwrap().children.iter().map(|c| back.node(*c).unwrap()).filter(|n| !n.child_layout.absolute).map(|n| n.height).collect();
    assert!(all_near(&heights, &[41.0, 41.0, 190.0]), "{heights:?}");
}
