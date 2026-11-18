//! Pasting spreadsheet cells: on a board they make a table (centred in the view, or with Paste here
//! its top-left at the point); in a design file, or text that isn't a grid of cells, a text layer as
//! before.

use figcraft_doc::{BoardItem, Document, NodeKind};
use figcraft_engine::Session;
use serde_json::json;

use crate::{FigApp, NoServices};

const CELLS: &str = "Name\tRole\nAda\tLead\nGrace\tReview\n";

fn app(board: bool) -> FigApp {
    let mut s = Session::new();
    if board {
        s.execute("file.newBoard", &json!({})).unwrap();
    }
    FigApp::new(s, Box::new(NoServices))
}

fn tables(doc: &Document) -> Vec<figcraft_doc::NodeId> {
    doc.ids().filter(|i| doc.node(*i).is_some_and(|n| n.board == Some(BoardItem::Table))).collect()
}

fn texts(doc: &Document) -> usize {
    doc.ids().filter(|i| doc.node(*i).is_some_and(|n| matches!(n.kind, NodeKind::Text(_)))).count()
}

#[test]
fn spreadsheet_cells_paste_as_a_table_on_a_board() {
    let mut app = app(true);
    crate::menus::paste(&mut app, Some(CELLS), None);
    let doc = app.session.doc().unwrap();
    let [table] = tables(doc)[..] else { panic!("one table") };
    assert_eq!(app.session.selection(), vec![table], "selected, as a new table is");
    let rows = doc.node(table).unwrap().children.iter().filter(|r| doc.node(**r).is_some_and(|n| !n.child_layout.absolute)).count();
    assert_eq!(rows, 3);
    let view = app.session.ds().unwrap().view.clone();
    let b = doc.world_bounds(table).unwrap();
    assert!((b.center().x - view.cx).abs() < 1.0, "centred in the view: {b:?}");
    // Paste here: its top-left at the point.
    crate::menus::paste(&mut app, Some(CELLS), Some((500.0, 700.0)));
    let doc = app.session.doc().unwrap();
    let placed = tables(doc).into_iter().find(|t| *t != table).expect("a second table");
    let b = doc.world_bounds(placed).unwrap();
    assert_eq!((b.x0, b.y0), (500.0, 700.0));
    assert!(app.ui.toast.is_none(), "nothing left out, nothing to say");
}

#[test]
fn a_larger_paste_keeps_the_largest_table_and_says_so() {
    let mut app = app(true);
    let row = vec!["x"; 25].join("\t");
    crate::menus::paste(&mut app, Some(&vec![row.as_str(); 60].join("\n")), None);
    assert_eq!(tables(app.session.doc().unwrap()).len(), 1);
    assert!(app.ui.toast.as_ref().is_some_and(|(m, _)| m.contains("50 rows and 20 columns")), "the rest wasn't pasted");
}

#[test]
fn other_pastes_stay_text() {
    // Cells pasted into a design file.
    let mut design = app(false);
    let before = texts(design.session.doc().unwrap());
    crate::menus::paste(&mut design, Some(CELLS), None);
    assert!(tables(design.session.doc().unwrap()).is_empty());
    assert_eq!(texts(design.session.doc().unwrap()), before + 1, "a text layer");
    // Lines without tabs on a board.
    let mut board = app(true);
    let before = texts(board.session.doc().unwrap());
    crate::menus::paste(&mut board, Some("first idea\nsecond idea"), None);
    assert!(tables(board.session.doc().unwrap()).is_empty());
    assert_eq!(texts(board.session.doc().unwrap()), before + 1);
}
