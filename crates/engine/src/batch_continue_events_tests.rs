//! A batch that goes on after a failed step (`onError: "continue"`) reports the live events of the
//! steps that stayed, not of the ones rolled back.

use std::time::Duration;

use serde_json::json;

use crate::Session;

#[test]
fn a_continued_batch_reports_only_the_events_of_steps_that_stayed() {
    let mut s = Session::new();
    let before = s.events.latest();
    let r = s
        .execute(
            "command.batch",
            &json!({"onError": "continue", "commands": [
                {"command": "comment.add", "params": {"x": 10, "y": 10, "text": "Stays"}},
                {"command": "command.batch", "params": {"commands": [
                    {"command": "comment.add", "params": {"x": 20, "y": 20, "text": "Rolled back"}},
                    {"command": "node.set", "params": {"ids": [999_999], "props": {"name": "x"}}}
                ]}},
                {"command": "comment.add", "params": {"x": 30, "y": 30, "text": "Stays too"}}
            ]}),
        )
        .unwrap();
    let ok: Vec<bool> = r.as_array().unwrap().iter().map(|o| o["ok"].as_bool().unwrap()).collect();
    assert_eq!(ok, [true, false, true]);
    let texts: Vec<String> = s.doc().unwrap().comments.iter().map(|c| c.first.text.clone()).collect();
    assert_eq!(texts, ["Stays", "Stays too"]);
    let events = s.events.poll(before, &["comments".into()], 10, Duration::ZERO).events;
    assert_eq!(events.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(), ["comment.added", "comment.added"]);
}
