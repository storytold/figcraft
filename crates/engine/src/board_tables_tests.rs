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
