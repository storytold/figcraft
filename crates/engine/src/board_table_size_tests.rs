//! Sizing a board table's columns and rows by their borders, and colouring cells: the documented
//! steps are to hover a border and drag it, and to pick a cell, or the table, and change its
//! background from the toolbar.

use figcraft_doc::{BoardItem, Document, NodeId};
use figcraft_geom::Point;
use serde_json::{Value, json};

use crate::Session;
use crate::board_table::active_cell;
use crate::tools::{PointerEvent, PointerKind};

/// A board with a 2 × 3 table at the origin: 480 wide, its columns 160, its rows at least 40.
fn table() -> (Session, NodeId, Value) {
    let mut s = Session::new();
    s.execute("file.newBoard", &json!({})).unwrap();
    let r = s.execute("board.table", &json!({"rows": 2, "columns": 3, "x": 0, "y": 0})).unwrap();
    (s, NodeId(r["id"].as_u64().unwrap()), r)
}

fn rows(d: &Document, t: NodeId) -> Vec<NodeId> {
    d.node(t).unwrap().children.iter().copied().filter(|c| !d.node(*c).unwrap().child_layout.absolute).collect()
}

/// The widths of the table's first row's cells, and the column lines' x positions.
fn columns(s: &Session, t: NodeId) -> (Vec<f64>, Vec<f64>) {
    let d = s.doc().unwrap();
    let row = rows(d, t)[0];
    let widths = d.node(row).unwrap().children.iter().map(|c| d.node(*c).unwrap().width).collect();
    let lines = d.node(t).unwrap().children.iter().filter(|c| d.node(**c).unwrap().child_layout.absolute);
    (widths, lines.map(|l| d.node(*l).unwrap().transform.translation().x + 0.5).collect())
}

fn edit(s: &mut Session, t: NodeId, action: &str, index: usize, size: f64) {
    s.execute("board.tableEdit", &json!({"id": t, "action": action, "index": index, "size": size})).unwrap();
}

fn drag(s: &mut Session, from: Point, to: Point) {
    let ev = |kind, p: Point| PointerEvent::at(kind, p.x, p.y, 1.0);
    s.pointer(&ev(PointerKind::Down, from)).unwrap();
    s.pointer(&ev(PointerKind::Move, from.midpoint(to))).unwrap();
    s.pointer(&ev(PointerKind::Move, to)).unwrap();
    s.pointer(&ev(PointerKind::Up, to)).unwrap();
}

fn click(s: &mut Session, p: Point) {
    let ev = |kind| PointerEvent::at(kind, p.x, p.y, 1.0);
    s.pointer(&ev(PointerKind::Down)).unwrap();
    s.pointer(&ev(PointerKind::Up)).unwrap();
}

#[test]
fn a_columns_border_drags_to_a_new_width_and_the_table_follows() {
    let (mut s, t, _) = table();
    drag(&mut s, Point::new(160.0, 20.0), Point::new(240.0, 20.0));
    assert_eq!(columns(&s, t), (vec![240.0, 160.0, 160.0], vec![240.0, 400.0]));
    assert_eq!(s.doc().unwrap().node(t).unwrap().width, 560.0);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(columns(&s, t).0, [160.0, 160.0, 160.0], "one undo step");
    assert_eq!(s.doc().unwrap().node(t).unwrap().width, 480.0);
}

#[test]
fn a_rows_border_drags_to_a_new_height() {
    let (mut s, t, _) = table();
    let h = |s: &Session| s.doc().unwrap().node(rows(s.doc().unwrap(), t)[0]).unwrap().height;
    let start = h(&s);
    drag(&mut s, Point::new(50.0, start), Point::new(50.0, start + 60.0));
    assert_eq!(h(&s), start + 60.0);
    // The last row has no line under it.
    assert_eq!(s.doc().unwrap().node(t).unwrap().height, 2.0 * start + 60.0 - 1.0, "the table hugs its rows");
    // It can't go below its text.
    edit(&mut s, t, "resizeRow", 0, 1.0);
    assert!(h(&s) >= 24.0);
}

/// A coloured cell reaches its row's line, whichever cell made the row tall.
#[test]
fn every_cell_of_a_row_is_as_tall_as_the_row() {
    let (mut s, t, made) = table();
    let long = "A long note that wraps over several lines in a column this narrow, and then some more";
    s.execute("node.set", &json!({"id": made["cells"][1][2], "props": {"characters": long}})).unwrap();
    let heights = |s: &Session, r: usize| {
        let d = s.doc().unwrap();
        d.node(rows(d, t)[r]).unwrap().children.iter().map(|c| d.node(*c).unwrap().height).collect::<Vec<_>>()
    };
    let tall = heights(&s, 1);
    assert!(tall[2] > 41.0, "{tall:?}");
    assert_eq!(tall, [tall[2]; 3]);
    // The shorter text again: they all come back.
    s.execute("node.set", &json!({"id": made["cells"][1][2], "props": {"characters": "Short"}})).unwrap();
    let short = heights(&s, 1);
    assert!(short[0] < tall[0] && short == [short[2]; 3], "{short:?}");
    edit(&mut s, t, "resizeRow", 0, 120.0);
    assert_eq!(heights(&s, 0), [119.0; 3], "its line is the row's last pixel");
}

#[test]
fn sizes_are_kept_to_what_a_column_may_be_and_junk_is_refused() {
    let (mut s, t, _) = table();
    edit(&mut s, t, "resizeColumn", 0, 1.0);
    assert_eq!(columns(&s, t).0[0], 40.0);
    edit(&mut s, t, "resizeColumn", 0, 1e9);
    assert_eq!(columns(&s, t).0[0], 4000.0);
    for p in [
        json!({"id": t, "action": "resizeColumn", "index": 9, "size": 100}),
        json!({"id": t, "action": "resizeRow", "index": 9, "size": 100}),
        json!({"id": t, "action": "resizeColumn", "size": 100}),
        json!({"id": t, "action": "resizeRow", "index": 0}),
        json!({"id": t, "action": "resizeColumn", "index": 0, "size": "wide"}),
    ] {
        assert!(s.execute("board.tableEdit", &p).is_err(), "{p}");
    }
}

#[test]
fn sized_columns_keep_their_widths_through_edits_and_new_rows_match() {
    let (mut s, t, _) = table();
    edit(&mut s, t, "resizeColumn", 0, 300.0);
    s.execute("board.tableEdit", &json!({"id": t, "action": "addRow"})).unwrap();
    let d = s.doc().unwrap();
    let last = *rows(d, t).last().unwrap();
    let w: Vec<f64> = d.node(last).unwrap().children.iter().map(|c| d.node(*c).unwrap().width).collect();
    assert_eq!(w, [300.0, 160.0, 160.0], "a new row lines up");
    s.execute("board.tableEdit", &json!({"id": t, "action": "addColumn", "index": 1})).unwrap();
    assert_eq!(columns(&s, t).0.len(), 4);
    assert_eq!(columns(&s, t).0[0], 300.0);
    s.execute("board.tableEdit", &json!({"id": t, "action": "removeColumn", "index": 1})).unwrap();
    assert_eq!(columns(&s, t), (vec![300.0, 160.0, 160.0], vec![300.0, 460.0]));
}

#[test]
fn colours_go_on_the_table_or_the_cell_picked_in_it() {
    let (mut s, t, made) = table();
    let fill = |s: &Session, c: NodeId| s.doc().unwrap().node(c).unwrap().fills.len();
    let cell = |d: &Document, r: usize, c: usize| d.node(rows(d, t)[r]).unwrap().children[c];
    let d = s.doc().unwrap();
    let (a, b, first) = (cell(d, 1, 1), cell(d, 1, 2), cell(d, 0, 0));
    // The table is selected: a click picks the cell, and the table stays selected.
    assert_eq!(s.selection(), [t]);
    let y = s.doc().unwrap().node(rows(s.doc().unwrap(), t)[0]).unwrap().height + 10.0;
    click(&mut s, Point::new(250.0, y));
    assert_eq!((s.selection().to_vec(), active_cell(&s)), (vec![t], Some(a)));
    s.execute("board.setColor", &json!({"ids": [a], "color": "blue"})).unwrap();
    assert_eq!((fill(&s, a), fill(&s, b)), (1, 0));
    // A cell's text stands for it, as `board.table` answers with the texts.
    let text = made["cells"][1][2].as_u64().unwrap();
    s.execute("board.setColor", &json!({"ids": [text], "color": "green"})).unwrap();
    assert_eq!(fill(&s, b), 1);
    // The table colours all its cells.
    s.execute("board.setColor", &json!({"ids": [t], "color": "red"})).unwrap();
    assert_eq!(fill(&s, first), 1);
    // A click on the empty board ends it.
    click(&mut s, Point::new(900.0, 900.0));
    assert_eq!(active_cell(&s), None);
}

#[test]
fn a_selected_table_still_drags_and_a_drag_picks_no_cell() {
    let (mut s, t, _) = table();
    drag(&mut s, Point::new(250.0, 60.0), Point::new(300.0, 160.0));
    let n = s.doc().unwrap().node(t).unwrap();
    assert_eq!((n.transform.translation().x, n.transform.translation().y), (50.0, 100.0));
    assert_eq!(n.board, Some(BoardItem::Table));
    assert!(active_cell(&s).is_none());
}

#[test]
fn sized_tables_save_and_reopen() {
    let (mut s, t, _) = table();
    edit(&mut s, t, "resizeColumn", 1, 300.0);
    let bytes = figcraft_format::save_bytes(s.doc().unwrap(), None).unwrap();
    let (open, _) = figcraft_format::load_bytes(&bytes).unwrap();
    let row = rows(&open, t)[0];
    let w: Vec<f64> = open.node(row).unwrap().children.iter().map(|c| open.node(*c).unwrap().width).collect();
    assert_eq!(w, [160.0, 300.0, 160.0]);
}
