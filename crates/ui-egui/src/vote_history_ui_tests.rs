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

/// Where the text `text` is drawn.
fn rect_of(u: &mut Ui, text: &str) -> egui::Rect {
    fn walk(shape: &egui::Shape, text: &str) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Text(t) if t.galley.text() == text => Some(t.galley.rect.translate(t.pos.to_vec2())),
            egui::Shape::Vec(v) => v.iter().find_map(|s| walk(s, text)),
            _ => None,
        }
    }
    u.frame(vec![]);
    let frame = u.frame_output(vec![]);
    frame.shapes.iter().find_map(|c| walk(&c.shape, text)).unwrap_or_else(|| panic!("no {text:?}"))
}

/// Clicks the stepper's left (later vote) or right (earlier vote) arrow, either side of "n of m".
fn step(u: &mut Ui, now: &str, right: bool) {
    let r = rect_of(u, now);
    // The 24-pixel arrows sit next to the count.
    let x = if right { r.max.x + 14.0 } else { r.min.x - 14.0 };
    click(u, egui::pos2(x, r.center().y));
}

fn has(u: &mut Ui, text: &str) -> bool {
    texts(u).iter().any(|t| t == text)
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
    // At either end that arrow is dimmed and does nothing; the other steps.
    step(&mut u, "1 of 2", false);
    assert!(has(&mut u, "1 of 2"), "no later vote than the latest");
    step(&mut u, "1 of 2", true);
    assert!(has(&mut u, "2 of 2") && has(&mut u, "1. Ship it"), "the earlier vote");
    step(&mut u, "2 of 2", true);
    assert!(has(&mut u, "2 of 2"), "no earlier vote than the first");
    step(&mut u, "2 of 2", false);
    assert!(has(&mut u, "1 of 2") && has(&mut u, "1. Wait"), "back to the latest");
    step(&mut u, "1 of 2", true);
    // A row selects its object.
    let row = at(&mut u, "1. Ship it");
    click(&mut u, row);
    let earlier = figcraft_engine::cmd::facilitate::sessions(&u.app.session).unwrap()[1].rows[0].id;
    assert_eq!(u.app.session.selection(), [earlier]);
    let delete = at(&mut u, "Delete results");
    click(&mut u, delete);
    assert_eq!(u.app.session.doc().unwrap().vote_history.len(), 1, "the earlier vote is deleted");
    // It stays open, on what's left.
    let shown = texts(&mut u);
    assert!(shown.iter().any(|t| t == "1. Wait") && !shown.iter().any(|t| t == "1 of 2"), "one left: {shown:?}");
    let delete = at(&mut u, "Delete results");
    click(&mut u, delete);
    assert!(u.app.session.doc().unwrap().vote_history.is_empty() && !has(&mut u, "Results"), "none left, and no Results to open");
}
