//! A board's timer and votes: the commands, what undo leaves alone, saving, live sessions, clicks
//! that vote, and junk parameters.

use figcraft_doc::{BoardTimer, NodeId, VoteScope};
use serde_json::{Value, json};

use crate::board_tests::{board, id};
use crate::cmd::facilitate::{clock, timer_after, timer_state};
use crate::collab_live_tests::Room;
use crate::tools::{PointerEvent, PointerKind};
use crate::{EngineError, Session};

const MIN: u64 = 60_000;

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn sticky(s: &mut Session, x: f64, text: &str) -> NodeId {
    id(&ex(s, "board.sticky", json!({"x": x, "y": 0, "text": text})), "id")
}

#[test]
fn the_timer_counts_from_its_end_time() {
    let now = 1_000_000;
    let none = json!({});
    let t = timer_after("t", None, "start", &json!({"minutes": 2}), now).unwrap();
    assert_eq!(t, Some(BoardTimer::Running { ends: now + 2 * MIN }));
    assert_eq!(timer_state(t, now + MIN), "running");
    assert_eq!(timer_state(t, now + 3 * MIN), "done", "run out: done until stopped");
    // Paused half way: the time left stays put however long the pause.
    let p = timer_after("t", t, "pause", &none, now + MIN).unwrap();
    assert_eq!(p, Some(BoardTimer::Paused { left: MIN }));
    assert_eq!(p.unwrap().left(now + 100 * MIN), MIN);
    // A minute more while paused, then on again.
    let p = timer_after("t", p, "add", &none, now + 5 * MIN).unwrap();
    assert_eq!(p, Some(BoardTimer::Paused { left: 2 * MIN }));
    let r = timer_after("t", p, "resume", &none, now + 10 * MIN).unwrap();
    assert_eq!(r, Some(BoardTimer::Running { ends: now + 12 * MIN }));
    // Adding to a timer that ran out starts from now.
    let r = timer_after("t", r, "add", &json!({"minutes": 0.5}), now + 20 * MIN).unwrap();
    assert_eq!(r, Some(BoardTimer::Running { ends: now + 20 * MIN + MIN / 2 }));
    assert_eq!(timer_after("t", r, "stop", &none, now).unwrap(), None);
    // What can't be done says so.
    assert!(timer_after("t", None, "pause", &none, now).is_err());
    assert!(timer_after("t", None, "add", &none, now).is_err());
    assert!(timer_after("t", r, "resume", &none, now).is_err());
    assert!(timer_after("t", Some(BoardTimer::Running { ends: now }), "pause", &none, now).is_err(), "nothing left to pause");
    for m in [json!(0), json!(-1), json!(1441), json!("NaN"), json!(1e308), json!("x"), json!([1])] {
        assert!(timer_after("t", None, "start", &json!({"minutes": m}), now).is_err(), "{m}");
    }
    // At most a day, also by adding.
    let long = timer_after("t", None, "start", &json!({"minutes": 1440}), now).unwrap();
    assert_eq!(timer_after("t", long, "add", &none, now).unwrap().unwrap().left(now), 1440 * MIN);
    assert!(timer_after("t", None, "start", &json!({"minutes": 1}), u64::MAX).is_ok(), "no overflow");
}

#[test]
fn clocks_read_minutes_and_seconds() {
    assert_eq!(clock(0), "0:00");
    assert_eq!(clock(1), "0:01", "a part of a second left still shows");
    assert_eq!(clock(5 * MIN), "5:00");
    assert_eq!(clock(65_000), "1:05");
    assert_eq!(clock(3_723_000), "1:02:03");
}

#[test]
fn timer_commands_drive_the_boards_timer() {
    let mut s = board();
    assert_eq!(ex(&mut s, "board.timerState", json!({}))["state"], "off");
    let r = ex(&mut s, "board.timer", json!({"action": "start", "minutes": 3}));
    assert_eq!(r["state"], "running");
    let left = r["left"].as_u64().unwrap();
    assert!(left > 3 * MIN - 5000 && left <= 3 * MIN, "{r}");
    assert!(r["ends"].as_u64().is_some());
    assert_eq!(ex(&mut s, "board.timer", json!({"action": "pause"}))["state"], "paused");
    let r = ex(&mut s, "board.timer", json!({"action": "add"}));
    assert!(r["left"].as_u64().unwrap() > 4 * MIN - 5000, "{r}");
    assert_eq!(ex(&mut s, "board.timer", json!({"action": "resume"}))["state"], "running");
    assert_eq!(ex(&mut s, "board.timer", json!({"action": "stop"}))["state"], "off");
    assert!(s.doc().unwrap().timer.is_none());
    // Not an undo step.
    assert_eq!(s.undo_depth().0, 0);
    assert!(matches!(s.execute("board.timer", &json!({"action": "pause"})), Err(EngineError::Disabled(..))));
    assert!(s.execute("board.timer", &json!({"action": "jump"})).is_err());
    // Boards only.
    let mut d = Session::new();
    d.execute("file.new", &json!({})).unwrap();
    for (cmd, p) in [
        ("board.timer", json!({"action": "start"})),
        ("board.timerState", json!({})),
        ("board.vote", json!({"action": "start"})),
        ("board.voteResults", json!({})),
    ] {
        assert!(matches!(d.execute(cmd, &p), Err(EngineError::Disabled(..))), "{cmd}");
    }
}

#[test]
fn votes_are_capped_hidden_until_the_end_and_ranked() {
    let mut s = board();
    let a = sticky(&mut s, 0.0, "Ship it");
    let b = sticky(&mut s, 300.0, "Wait");
    let c = sticky(&mut s, 600.0, "Rethink");
    let shape = id(&ex(&mut s, "board.shape", json!({"x": 0, "y": 400})), "id");
    let r = ex(&mut s, "board.vote", json!({"action": "start", "votes": 2, "on": "stickies"}));
    assert_eq!((r["state"].as_str(), r["left"].as_u64(), r["on"].as_str()), (Some("running"), Some(2), Some("stickies")));
    assert!(matches!(s.execute("board.vote", &json!({"action": "start"})), Err(EngineError::Disabled(..))), "one vote at a time");
    let cast = |s: &mut Session, who: &str, on: NodeId| s.execute("board.vote", &json!({"action": "cast", "id": on, "voter": who}));
    // Ada puts both her votes on one sticky; a third is refused, and so is a shape.
    cast(&mut s, "Ada", a).unwrap();
    let r = cast(&mut s, "Ada", a).unwrap();
    assert_eq!((r["left"].as_u64(), r["mine"].clone()), (Some(0), json!([{"id": a.0, "votes": 2}])));
    assert!(matches!(cast(&mut s, "Ada", b), Err(EngineError::Disabled(..))), "no votes left");
    assert!(matches!(cast(&mut s, "Bo", shape), Err(EngineError::Disabled(..))), "stickies only");
    assert!(matches!(cast(&mut s, "Bo", NodeId(99_999)), Err(EngineError::NoNode(_))));
    cast(&mut s, "Bo", b).unwrap();
    cast(&mut s, "Bo", c).unwrap();
    cast(&mut s, "Cy", b).unwrap();
    cast(&mut s, "Cy", c).unwrap();
    // Cy takes one back and puts it elsewhere.
    s.execute("board.vote", &json!({"action": "retract", "id": c, "voter": "Cy"})).unwrap();
    assert!(s.execute("board.vote", &json!({"action": "retract", "id": c, "voter": "Cy"})).is_err(), "no vote there any more");
    cast(&mut s, "Cy", a).unwrap();
    // While it runs, you see your own votes only.
    let r = ex(&mut s, "board.voteResults", json!({"voter": "Bo"}));
    assert!(r.get("results").is_none(), "{r}");
    assert_eq!(r["mine"], json!([{"id": b.0, "votes": 1}, {"id": c.0, "votes": 1}]));
    assert_eq!(r["voters"], 3);
    assert_eq!(s.undo_depth().0, 4, "votes are no undo steps (only the four objects)");
    ex(&mut s, "board.vote", json!({"action": "end"}));
    assert!(s.execute("board.vote", &json!({"action": "cast", "id": b, "voter": "Dee"})).is_err(), "ended");
    let r = ex(&mut s, "board.voteResults", json!({}));
    assert_eq!(r["state"], "ended");
    // Once it ends, everyone sees who voted for what.
    let voter = |name: &str, votes: u32| json!({"voter": name, "name": name, "votes": votes});
    assert_eq!(
        r["results"],
        json!([
            {"id": a.0, "name": "Ship it", "votes": 3, "rank": 1, "voters": [voter("Ada", 2), voter("Cy", 1)]},
            {"id": b.0, "name": "Wait", "votes": 2, "rank": 2, "voters": [voter("Bo", 1), voter("Cy", 1)]},
            {"id": c.0, "name": "Rethink", "votes": 1, "rank": 3, "voters": [voter("Bo", 1)]},
        ])
    );
    // A deleted object's votes no longer count.
    ex(&mut s, "edit.delete", json!({"ids": [a]}));
    assert_eq!(ex(&mut s, "board.voteResults", json!({}))["results"][0]["id"], b.0);
    ex(&mut s, "board.vote", json!({"action": "clear"}));
    assert_eq!(ex(&mut s, "board.voteResults", json!({}))["state"], "off");
    assert!(s.doc().unwrap().votes.is_empty());
}

#[test]
fn a_vote_on_the_selection_takes_only_those() {
    let mut s = board();
    let a = sticky(&mut s, 0.0, "A");
    let b = sticky(&mut s, 300.0, "B");
    ex(&mut s, "select.set", json!({"ids": [a]}));
    let r = ex(&mut s, "board.vote", json!({"action": "start", "on": "selection"}));
    assert_eq!(r["on"], json!({"selection": [a.0]}));
    assert_eq!(r["votesPerPerson"], 3);
    assert!(s.execute("board.vote", &json!({"action": "cast", "id": b})).is_err());
    ex(&mut s, "board.vote", json!({"action": "cast", "id": a}));
    // A finished vote may be replaced by a new one, which starts with no votes.
    ex(&mut s, "board.vote", json!({"action": "end"}));
    ex(&mut s, "board.vote", json!({"action": "start", "on": "all", "votes": 1}));
    assert!(s.doc().unwrap().votes.is_empty());
    assert_eq!(s.doc().unwrap().vote.as_ref().unwrap().scope, VoteScope::All);
    for bad in [json!({"votes": 0}), json!({"votes": 100}), json!({"votes": 1.5}), json!({"on": "pages"}), json!({"on": "selection", "ids": []})] {
        let mut p = bad.clone();
        p["action"] = json!("start");
        ex(&mut s, "board.vote", json!({"action": "clear"}));
        assert!(s.execute("board.vote", &p).is_err(), "{p}");
    }
}

#[test]
fn undo_leaves_the_timer_and_the_vote_alone() {
    let mut s = board();
    let a = sticky(&mut s, 0.0, "A");
    ex(&mut s, "board.timer", json!({"action": "start"}));
    ex(&mut s, "board.vote", json!({"action": "start"}));
    ex(&mut s, "board.vote", json!({"action": "cast", "id": a}));
    let b = sticky(&mut s, 300.0, "B");
    s.undo().unwrap();
    s.undo().unwrap();
    let doc = s.doc().unwrap();
    assert!(doc.node(a).is_none() && doc.node(b).is_none(), "the stickies are undone");
    assert!(doc.timer.is_some() && doc.vote.is_some() && doc.votes.len() == 1, "the timer and the vote stay");
    ex(&mut s, "board.timer", json!({"action": "stop"}));
    s.redo().unwrap();
    assert!(s.doc().unwrap().timer.is_none(), "redo doesn't bring a stopped timer back");
    assert!(s.doc().unwrap().node(a).is_some());
}

#[test]
fn the_timer_and_votes_save_and_old_files_open() {
    let mut s = board();
    let a = sticky(&mut s, 0.0, "A");
    ex(&mut s, "board.timer", json!({"action": "start"}));
    ex(&mut s, "board.timer", json!({"action": "pause"}));
    ex(&mut s, "board.vote", json!({"action": "start", "on": "stickies"}));
    ex(&mut s, "board.vote", json!({"action": "cast", "id": a}));
    let doc = s.doc().unwrap().clone();
    let bytes = figcraft_format::save_bytes(&doc, None).unwrap();
    let (back, _) = figcraft_format::load_bytes(&bytes).unwrap();
    assert_eq!((back.timer, &back.vote, &back.votes), (doc.timer, &doc.vote, &doc.votes));
    // A file from before: none of the fields.
    let mut v = figcraft_format::to_json(&doc).unwrap();
    for k in ["timer", "vote", "votes"] {
        assert!(v.as_object_mut().unwrap().remove(k).is_some(), "{k} is saved");
    }
    let (old, _) = figcraft_format::from_json(&v.to_string()).unwrap();
    assert!(old.timer.is_none() && old.vote.is_none() && old.votes.is_empty());
    // Design files don't write them.
    let plain = figcraft_format::to_json(&figcraft_doc::Document::new("D")).unwrap();
    assert!(plain.get("timer").is_none() && plain.get("vote").is_none() && plain.get("votes").is_none());
}

#[test]
fn everyone_in_a_live_session_shares_the_timer_and_the_vote() {
    let mut s = board();
    let a = sticky(&mut s, 0.0, "A");
    let b = sticky(&mut s, 300.0, "B");
    ex(&mut s, "board.timer", json!({"action": "start", "minutes": 10}));
    let mut room = Room::new(s.doc().unwrap().clone(), 2, 11);
    assert!(room.s(1).doc().unwrap().timer.is_some(), "joiners get the running timer");
    let t0 = room.s(0).doc().unwrap().timer;
    assert_eq!(room.s(1).doc().unwrap().timer, t0);
    ex(room.s(1), "board.timer", json!({"action": "pause"}));
    ex(room.s(0), "board.vote", json!({"action": "start", "votes": 2}));
    room.settle();
    room.check();
    assert!(matches!(room.s(0).doc().unwrap().timer, Some(BoardTimer::Paused { .. })), "a pause reaches everyone");
    // Both vote at once: each ballot is its own entry, so neither is lost.
    ex(room.s(0), "board.vote", json!({"action": "cast", "id": a}));
    ex(room.s(1), "board.vote", json!({"action": "cast", "id": a}));
    ex(room.s(1), "board.vote", json!({"action": "cast", "id": b}));
    room.settle();
    room.check();
    assert_eq!(room.server.document().votes.len(), 2, "two voters, apart though both are called the same");
    assert_eq!(ex(room.s(0), "board.voteResults", json!({}))["mine"], json!([{"id": a.0, "votes": 1}]));
    // Who voted shows by the names people go by in the session, kept with the results when it
    // ends (this room delivers no join messages, so peer 2 is added as one would add them).
    let me = room.s(0).live_mut().and_then(|l| l.info.as_ref()).map(|i| i.name.clone()).expect("in the session");
    let peer2 = figcraft_collab::Peer { client_id: 2, name: "Grace".into(), color: "#00AA88".into() };
    room.s(0).live_mut().expect("in the session").peers.insert(2, peer2);
    ex(room.s(0), "board.vote", json!({"action": "end"}));
    room.settle();
    room.check();
    let grace = json!({"voter": "peer 2", "name": "Grace", "votes": 1});
    let want = json!([
        {"id": a.0, "name": "A", "votes": 2, "rank": 1, "voters": [{"voter": "peer 1", "name": me, "votes": 1}, grace]},
        {"id": b.0, "name": "B", "votes": 1, "rank": 2, "voters": [grace]},
    ]);
    for i in 0..2 {
        assert_eq!(ex(room.s(i), "board.voteResults", json!({}))["results"], want, "everyone sees them ({i})");
    }
    // Someone who has left keeps the name the results were kept with; their votes still count.
    room.s(0).live_mut().expect("in the session").peers.remove(&2);
    assert_eq!(ex(room.s(0), "board.voteResults", json!({}))["results"], want);
    assert_eq!(room.server.document().vote_history.len(), 1, "the results are kept for everyone");
    // Undo in a session takes back your own edits only, not the vote.
    room.s(0).undo().unwrap();
    room.settle();
    room.check();
    assert!(room.server.document().vote.is_some());
}

#[test]
fn clicks_vote_while_a_vote_runs() {
    let mut s = board();
    let a = sticky(&mut s, 0.0, "A");
    ex(&mut s, "select.none", json!({}));
    let click = |s: &mut Session, x: f64, y: f64, alt: bool| {
        let mut down = PointerEvent::at(PointerKind::Down, x, y, 1.0);
        down.alt = alt;
        s.pointer(&down).unwrap();
        s.pointer(&PointerEvent::at(PointerKind::Up, x, y, 1.0)).unwrap();
    };
    // No vote: a click selects.
    click(&mut s, 100.0, 100.0, false);
    assert_eq!(s.selection(), vec![a]);
    ex(&mut s, "select.none", json!({}));
    ex(&mut s, "board.vote", json!({"action": "start"}));
    click(&mut s, 100.0, 100.0, false);
    click(&mut s, 50.0, 150.0, false);
    assert!(s.selection().is_empty(), "votes, not selection");
    assert_eq!(crate::cmd::facilitate::mine(s.doc().unwrap(), "You"), vec![(a, 2)]);
    // Alt takes one back; so does a click on your badge (the sticky's top-right corner).
    click(&mut s, 100.0, 100.0, true);
    click(&mut s, 199.0, 1.0, false);
    assert!(s.doc().unwrap().votes.is_empty());
    // Empty canvas is the tool's as ever.
    click(&mut s, 900.0, 900.0, false);
    assert!(s.doc().unwrap().votes.is_empty());
}

#[test]
fn facilitation_commands_survive_junk() {
    let junk = [
        json!({}),
        json!(null),
        json!([1]),
        json!({"action": 5, "minutes": "NaN", "votes": -1, "on": [1], "id": "x", "voter": {"a": 1}}),
        json!({"action": "start", "minutes": 1e308, "votes": 1e308, "on": "selection", "ids": ["x", -1]}),
        json!({"action": "add", "minutes": -1e308}),
        json!({"action": "cast", "id": 99_999, "voter": "\u{0}\u{7}"}),
        json!({"action": "retract", "id": -1, "voter": "x".repeat(10_000)}),
        json!({"action": "end"}),
        json!({"action": "clear", "ids": 7}),
        json!({"action": "resume", "minutes": [1]}),
        json!({"action": "delete"}),
        json!({"action": "delete", "index": -1}),
        json!({"action": "delete", "index": 1e308}),
        json!({"action": "delete", "index": "0"}),
        json!({"action": "delete", "index": 0}),
    ];
    for cmd in ["board.timer", "board.timerState", "board.vote", "board.voteResults"] {
        for j in &junk {
            let mut s = board();
            let a = sticky(&mut s, 0.0, "A");
            ex(&mut s, "board.timer", json!({"action": "start"}));
            ex(&mut s, "board.vote", json!({"action": "start"}));
            ex(&mut s, "board.vote", json!({"action": "cast", "id": a}));
            if let Err(EngineError::Internal { cmd, msg }) = s.execute(cmd, j) {
                panic!("{cmd} panicked on {j}: {msg}");
            }
            // A voter name is kept short.
            assert!(s.doc().unwrap().votes.keys().all(|k| k.chars().count() <= 64));
        }
    }
}

#[test]
fn ties_share_a_rank() {
    let mut s = board();
    let ids: Vec<NodeId> = (0..4).map(|i| sticky(&mut s, f64::from(i) * 300.0, "x")).collect();
    ex(&mut s, "board.vote", json!({"action": "start", "votes": 2}));
    for (who, on) in [("a", 0), ("a", 1), ("b", 1), ("b", 0), ("c", 2), ("c", 3), ("d", 2)] {
        ex(&mut s, "board.vote", json!({"action": "cast", "id": ids[on], "voter": who}));
    }
    ex(&mut s, "board.vote", json!({"action": "end"}));
    let r = ex(&mut s, "board.voteResults", json!({}));
    let ranks: Vec<u64> = r["results"].as_array().unwrap().iter().map(|x| x["rank"].as_u64().unwrap()).collect();
    assert_eq!(ranks, vec![1, 1, 1, 4], "{r}");
}
