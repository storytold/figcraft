//! An ended vote's results say who voted for each object (as the reference board app shows how
//! each participant voted once a vote ends); while it runs, nobody's votes but your own show.

use figcraft_engine::Session;
use serde_json::json;

use crate::export_preview_tests::{click, labelled};
use crate::layers_drag_tests::Ui;

/// A board with two stickies voted on by Ada (twice on the first) and Bo (once on each).
fn voted() -> Ui {
    let mut s = Session::new();
    s.execute("file.newBoard", &json!({})).unwrap();
    let sticky =
        |s: &mut Session, x: i32, text: &str| crate::present_tests::id(&s.execute("board.sticky", &json!({"x": x, "y": 0, "text": text})).unwrap());
    let (a, b) = (sticky(&mut s, 0, "Ship it"), sticky(&mut s, 300, "Wait"));
    s.execute("board.vote", &json!({"action": "start", "votes": 2})).unwrap();
    for (who, on) in [("Ada", a), ("Ada", a), ("Bo", a), ("Bo", b)] {
        s.execute("board.vote", &json!({"action": "cast", "id": on, "voter": who})).unwrap();
    }
    Ui::new(s)
}

#[test]
fn the_results_say_who_voted_for_each_object() {
    let mut u = voted();
    let running = labelled(&mut u);
    assert!(!running.iter().any(|(t, _)| t.contains("Ada")), "nobody's names while it runs");
    u.app.run("board.vote", json!({"action": "end"}));
    let shown = labelled(&mut u);
    let results = shown.iter().find(|(t, _)| t == "Results").map(|r| r.1).unwrap_or_else(|| panic!("a Results button in {shown:?}"));
    click(&mut u, results);
    let texts: Vec<String> = labelled(&mut u).into_iter().map(|(t, _)| t).collect();
    let row = |name: &str| texts.iter().position(|t| t.ends_with(name)).unwrap_or_else(|| panic!("a row for {name} in {texts:?}"));
    let (first, second) = (row("Ship it"), row("Wait"));
    assert!(first < second, "most votes first");
    assert!(texts[first..second].iter().any(|t| t == "Ada ×2, Bo"), "under the first: {texts:?}");
    assert!(texts[second..].iter().any(|t| t == "Bo"), "under the second: {texts:?}");
    // Clear takes the results off the board and keeps them in the file.
    let shown = labelled(&mut u);
    let clear = shown.iter().find(|(t, _)| t == "Clear results").map(|r| r.1).unwrap_or_else(|| panic!("Clear results in {shown:?}"));
    click(&mut u, clear);
    let doc = u.app.session.doc().unwrap();
    assert!(doc.vote.is_none() && doc.vote_history.len() == 1, "cleared off the board, kept in the file");
}
