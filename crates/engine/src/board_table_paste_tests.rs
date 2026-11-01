//! Spreadsheet cells as a board table's cells (`board_table::spreadsheet_cells`): rows on lines,
//! cells between tabs, quoted cells holding tabs, line breaks and quotes; other text isn't a table.

use serde_json::json;

use crate::Session;
use crate::board_table::{MAX_COLUMNS, MAX_ROWS, spreadsheet_cells};

fn cells(text: &str) -> Vec<Vec<String>> {
    let (cells, more) = spreadsheet_cells(text).expect("a table");
    assert!(!more, "nothing left out");
    cells
}

fn rows(r: &[&[&str]]) -> Vec<Vec<String>> {
    r.iter().map(|row| row.iter().map(|c| (*c).to_string()).collect()).collect()
}

#[test]
fn spreadsheet_text_makes_rows_of_cells() {
    assert_eq!(cells("Name\tRole\nAda\tLead\n"), rows(&[&["Name", "Role"], &["Ada", "Lead"]]), "a copy ends with a line break");
    assert_eq!(cells("Name\tRole\r\nAda\tLead\r\n"), rows(&[&["Name", "Role"], &["Ada", "Lead"]]), "Windows line breaks");
    assert_eq!(cells("a\t\tc\n\tb\t"), rows(&[&["a", "", "c"], &["", "b", ""]]), "empty cells keep their place");
    assert_eq!(cells("a\tb\nc"), rows(&[&["a", "b"], &["c"]]), "a short row stays short (the table fills it)");
    assert_eq!(cells("Title\nx\ty"), rows(&[&["Title"], &["x", "y"]]), "one row of two cells is enough");
}

#[test]
fn quoted_cells_hold_tabs_line_breaks_and_quotes() {
    let text = "\"a\tb\"\t\"line 1\nline 2\"\t\"say \"\"hi\"\"\"\n\"\"\tplain \"quote\"";
    assert_eq!(cells(text), rows(&[&["a\tb", "line 1\nline 2", "say \"hi\""], &["", "plain \"quote\""]]));
}

#[test]
fn other_text_is_not_a_table() {
    for text in ["", "just text", "one\ntwo\nthree", "a, b, c\n1, 2, 3", "\n\n"] {
        assert_eq!(spreadsheet_cells(text), None, "{text:?}");
    }
}

#[test]
fn a_table_keeps_at_most_its_largest_size() {
    let row = vec!["x"; MAX_COLUMNS + 5].join("\t");
    let text = vec![row.as_str(); MAX_ROWS + 10].join("\n");
    let (cells, more) = spreadsheet_cells(&text).expect("a table");
    assert!(more, "the rest is left out, and said so");
    assert_eq!(cells.len(), MAX_ROWS);
    assert!(cells.iter().all(|r| r.len() == MAX_COLUMNS));
}

#[test]
fn pasted_cells_make_a_board_table() {
    let mut s = Session::new();
    s.execute("file.newBoard", &json!({})).unwrap();
    let (cells, _) = spreadsheet_cells("Name\tRole\tTeam\nAda\tLead\n").unwrap();
    let r = s.execute("board.table", &json!({"cells": cells, "x": 0, "y": 0})).unwrap();
    assert_eq!((r["rows"].as_u64(), r["columns"].as_u64()), (Some(2), Some(3)));
    let doc = s.doc().unwrap();
    let text = |row: usize, col: usize| {
        let id = crate::params::id_of(&r["cells"][row][col]).unwrap();
        doc.node(id).and_then(|n| n.kind.text()).map(|t| t.characters.clone()).unwrap()
    };
    assert_eq!((text(0, 0), text(0, 2), text(1, 1), text(1, 2)), ("Name".into(), "Team".into(), "Lead".into(), String::new()));
}

#[test]
fn junk_never_panics() {
    let parts = ["\t", "\n", "\r", "\r\n", "\"", "\"\"", "a", "é", "漢", " "];
    let mut seed = 7u64;
    for _ in 0..2000 {
        let mut text = String::new();
        for _ in 0..(seed % 24) {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            text.push_str(parts[(seed >> 33) as usize % parts.len()]);
        }
        if let Some((cells, _)) = spreadsheet_cells(&text) {
            assert!(cells.len() <= MAX_ROWS && cells.iter().all(|r| r.len() <= MAX_COLUMNS));
            assert!(cells.iter().any(|r| r.len() > 1), "{text:?}");
        }
    }
}
