//! Earlier votes' results kept in the board file (the reference app keeps each voting session's
//! results until they're deleted): recorded when a vote ends, at most [`VOTE_HISTORY_MAX`], listed
//! newest first, deleted one at a time, saved, carried over undo, and opened from older files.

use figcraft_doc::{NodeId, VOTE_HISTORY_MAX, Vote, VoteRecord, VoteScope};
use serde_json::{Value, json};

use crate::board_tests::{board, id};
use crate::{EngineError, Session};

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn sticky(s: &mut Session, x: f64, text: &str) -> NodeId {
    id(&ex(s, "board.sticky", json!({"x": x, "y": 0, "text": text})), "id")
}

/// A vote of `votes` (voter, object) pairs, ended.
fn vote(s: &mut Session, votes: &[(&str, NodeId)]) {
    ex(s, "board.vote", json!({"action": "start", "votes": 3}));
    for (who, on) in votes {
        ex(s, "board.vote", json!({"action": "cast", "id": on, "voter": who}));
    }
    ex(s, "board.vote", json!({"action": "end"}));
}

/// A listed session: its index, whether it's on the board, and its objects' votes (id, votes).
type Listed = (Value, bool, Vec<(u64, u64)>);

fn sessions(s: &mut Session) -> Vec<Listed> {
    let r = ex(s, "board.voteResults", json!({}));
    r["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| {
            let rows = x["results"].as_array().unwrap().iter().map(|r| (r["id"].as_u64().unwrap(), r["votes"].as_u64().unwrap())).collect();
            (x["index"].clone(), x["onBoard"].as_bool().unwrap(), rows)
        })
        .collect()
}

#[test]
fn ended_votes_are_kept_newest_first_and_the_last_twenty() {
    let mut s = board();
    let (a, b) = (sticky(&mut s, 0.0, "A"), sticky(&mut s, 300.0, "B"));
    vote(&mut s, &[("Ada", a), ("Bo", a)]);
    vote(&mut s, &[("Ada", b)]);
    assert_eq!(
        sessions(&mut s),
        vec![(json!(1), true, vec![(b.0, 1)]), (json!(0), false, vec![(a.0, 2)])],
        "the vote on the board first, then the one before"
    );
    let kept = &s.doc().unwrap().vote_history;
    assert!(kept.iter().all(|r| r.vote.ended && r.ended > 0), "ended, with when");
    // Clearing takes the results off the board; starting a vote leaves the kept ones too.
    ex(&mut s, "board.vote", json!({"action": "clear"}));
    assert_eq!(sessions(&mut s).iter().filter(|x| x.1).count(), 0, "none on the board");
    assert_eq!(sessions(&mut s).len(), 2, "still kept");
    ex(&mut s, "board.vote", json!({"action": "start"}));
    assert_eq!(ex(&mut s, "board.voteResults", json!({}))["sessions"].as_array().map(Vec::len), Some(2), "kept while a vote runs");
    ex(&mut s, "board.vote", json!({"action": "end"}));
    for _ in 0..VOTE_HISTORY_MAX + 2 {
        vote(&mut s, &[("Cy", a)]);
    }
    assert_eq!(s.doc().unwrap().vote_history.len(), VOTE_HISTORY_MAX, "the oldest go");
    assert!(s.doc().unwrap().vote_history.iter().all(|r| r.ballots.contains_key("Cy")), "the first two are gone");
}

#[test]
fn deleting_kept_results() {
    let mut s = board();
    let (a, b) = (sticky(&mut s, 0.0, "A"), sticky(&mut s, 300.0, "B"));
    assert!(matches!(s.execute("board.vote", &json!({"action": "delete", "index": 0})), Err(EngineError::Disabled(..))), "none kept");
    vote(&mut s, &[("Ada", a)]);
    vote(&mut s, &[("Bo", b)]);
    assert!(s.execute("board.vote", &json!({"action": "delete", "index": 2})).is_err(), "no third");
    assert!(s.execute("board.vote", &json!({"action": "delete"})).is_err(), "which one");
    assert!(s.execute("board.vote", &json!({"action": "undelete", "index": 0})).is_err(), "no such action");
    // An earlier one: the vote on the board stays.
    ex(&mut s, "board.vote", json!({"action": "delete", "index": 0}));
    assert_eq!(sessions(&mut s), vec![(json!(0), true, vec![(b.0, 1)])]);
    assert!(s.doc().unwrap().vote.is_some());
    // The one on the board: it leaves the board too.
    ex(&mut s, "board.vote", json!({"action": "delete", "index": 0}));
    let doc = s.doc().unwrap();
    assert!(doc.vote_history.is_empty() && doc.vote.is_none() && doc.votes.is_empty());
    assert_eq!(ex(&mut s, "board.voteResults", json!({})), json!({"state": "off", "sessions": []}));
}

#[test]
fn kept_results_save_and_older_files_open() {
    let mut s = board();
    let a = sticky(&mut s, 0.0, "A");
    vote(&mut s, &[("Ada", a)]);
    let doc = s.doc().unwrap().clone();
    let bytes = figcraft_format::save_bytes(&doc, None).unwrap();
    let (back, _) = figcraft_format::load_bytes(&bytes).unwrap();
    assert_eq!(back.vote_history, doc.vote_history);
    // A file from before results were kept: none listed but the ended vote on its board, which
    // shows and clears as before (it can't be deleted: it isn't kept).
    let mut v = figcraft_format::to_json(&doc).unwrap();
    assert!(v.as_object_mut().unwrap().remove("vote_history").is_some(), "saved as vote_history");
    let (old, _) = figcraft_format::from_json(&v.to_string()).unwrap();
    assert!(old.vote_history.is_empty() && old.vote.is_some());
    let mut s = board();
    s.ds_mut().unwrap().doc = std::sync::Arc::new(old);
    assert_eq!(sessions(&mut s), vec![(Value::Null, true, vec![(a.0, 1)])]);
    ex(&mut s, "board.vote", json!({"action": "clear"}));
    assert!(sessions(&mut s).is_empty());
    // Design files don't write it.
    assert!(figcraft_format::to_json(&figcraft_doc::Document::new("D")).unwrap().get("vote_history").is_none());
}

#[test]
fn undo_leaves_kept_results_alone() {
    let mut s = board();
    let a = sticky(&mut s, 0.0, "A");
    vote(&mut s, &[("Ada", a)]);
    let b = sticky(&mut s, 300.0, "B");
    vote(&mut s, &[("Bo", b)]);
    s.undo().unwrap();
    assert!(s.doc().unwrap().node(b).is_none(), "the sticky is undone");
    assert_eq!(s.doc().unwrap().vote_history.len(), 2, "the results ended since stay");
    ex(&mut s, "board.vote", json!({"action": "delete", "index": 0}));
    s.redo().unwrap();
    assert_eq!(s.doc().unwrap().vote_history.len(), 1, "redo doesn't bring deleted results back");
}

#[test]
fn voters_go_by_the_names_kept_with_the_results() {
    let mut s = board();
    let a = sticky(&mut s, 0.0, "A");
    let record = |names: &[(&str, &str)]| VoteRecord {
        vote: Vote { per_person: 3, scope: VoteScope::All, ended: true, by: String::new() },
        ballots: [("peer 7".to_string(), vec![a]), ("peer 9".to_string(), vec![a])].into(),
        names: names.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect(),
        ended: 1,
    };
    s.ds_mut().unwrap().doc = {
        let mut d = s.doc().unwrap().clone();
        d.vote_history = vec![record(&[("peer 7", "Grace")])];
        std::sync::Arc::new(d)
    };
    let r = ex(&mut s, "board.voteResults", json!({}));
    assert_eq!(
        r["sessions"][0]["results"][0]["voters"],
        json!([{"voter": "peer 7", "name": "Grace", "votes": 1}, {"voter": "peer 9", "name": null, "votes": 1}]),
        "a name kept, and one nobody knew"
    );
}
