//! The Results popover steps through the votes kept in the file, newest first, and deletes the one
//! it shows; Results stays on the pill while any are kept, after they're cleared off the board.

use figcraft_engine::Session;
use serde_json::json;

use crate::export_preview_tests::{click, labelled};
use crate::layers_drag_tests::Ui;

/// A board where "Ship it" won a first vote and "Wait" a second one, both kept, then cleared off
/// the board.
fn two_votes() -> Ui {
    let mut s = Session::new();
    s.execute("file.newBoard", &json!({})).unwrap();
    let sticky =
        |s: &mut Session, x: i32, text: &str| crate::present_tests::id(&s.execute("board.sticky", &json!({"x": x, "y": 0, "text": text})).unwrap());
    let (a, b) = (sticky(&mut s, 0, "Ship it"), sticky(&mut s, 300, "Wait"));
    for on in [a, b] {
        s.execute("board.vote", &json!({"action": "start"})).unwrap();
        s.execute("board.vote", &json!({"action": "cast", "id": on, "voter": "Ada"})).unwrap();
        s.execute("board.vote", &json!({"action": "end"})).unwrap();
    }
    s.execute("board.vote", &json!({"action": "clear"})).unwrap();
    Ui::new(s)
}

fn texts(u: &mut Ui) -> Vec<String> {
    labelled(u).into_iter().map(|(t, _)| t).collect()
}

fn at(u: &mut Ui, text: &str) -> egui::Pos2 {
    let shown = labelled(u);
    shown.iter().find(|(t, _)| t == text).map(|x| x.1).unwrap_or_else(|| panic!("{text:?} in {shown:?}"))
}

#[test]
fn the_results_step_through_kept_votes_and_delete_one() {
    let mut u = two_votes();
    let results = at(&mut u, "Results");
    click(&mut u, results);
    let shown = texts(&mut u);
    assert!(shown.iter().any(|t| t == "1 of 2"), "{shown:?}");
    assert!(shown.iter().any(|t| t == "1. Wait"), "the latest first: {shown:?}");
    assert!(!shown.iter().any(|t| t == "Clear results"), "nothing on the board to clear");
    // The earlier vote (as the right arrow picks it).
    u.ctx.data_mut(|d| d.insert_temp(egui::Id::unique("board_vote_session"), 1usize));
    let shown = texts(&mut u);
    assert!(shown.iter().any(|t| t == "1. Ship it") && shown.iter().any(|t| t == "2 of 2"), "{shown:?}");
    let delete = at(&mut u, "Delete results");
    click(&mut u, delete);
    assert_eq!(u.app.session.doc().unwrap().vote_history.len(), 1, "the earlier vote is deleted");
    // It stays open, on what's left.
    let shown = texts(&mut u);
    assert!(shown.iter().any(|t| t == "1. Wait") && !shown.iter().any(|t| t == "1 of 2"), "one left: {shown:?}");
}
