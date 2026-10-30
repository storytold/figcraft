//! Running a session on a board: a countdown everyone sees and votes on objects.
//!
//! Both live in the document ([`figcraft_doc::facilitation`]) so they reach everyone in a live
//! session, and neither is an undo step: undoing an edit leaves the timer and the vote as they are.

use std::sync::Arc;

use figcraft_doc::{BoardTimer, Document, NodeId, TIMER_MAX_MS, VOTES_MAX, Vote, VoteScope};
use figcraft_geom::{Point, Rect};
use serde_json::{Value, json};

use crate::board::{require, text_of};
use crate::cmd::CommandSpec;
use crate::params::{id, num, req_string, req_targets, string};
use crate::tools::PointerEvent;
use crate::{EngineError, Result, Session, bad, spec};

/// Minutes a timer starts with, and adds, when none are given.
const START_MINUTES: f64 = 5.0;
const ADD_MINUTES: f64 = 1.0;
/// Votes each person gets when none are given.
const VOTES: u32 = 3;
/// A vote badge's radius on screen (logical pixels).
pub const BADGE_PX: f64 = 11.0;
/// The longest a voter name is kept.
const MAX_VOTER: usize = 64;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "board.timer",
            "Timer",
            "{action: start|pause|resume|add|stop, minutes? (start: default 5; add: default 1; up to 1440)} → the timer's state (as board.timerState)  one countdown per board, the same for everyone in a live session",
            false,
            timer_cmd
        ),
        spec!(
            "board.timerState",
            "Timer state",
            "{} → {state: off|running|paused|done, left (ms), text (m:ss), ends? (ms since 1970, while running)}",
            false,
            |s, _| {
                require(s, "board.timerState")?;
                Ok(timer_json(s.doc()?.timer, now_ms()))
            }
        ),
        spec!(
            "board.vote",
            "Voting session",
            "{action: start|cast|retract|end|clear, votes? (start: per person, 1–99, default 3), on? (start: all|stickies|selection; default all), ids? (start with on: selection; default the selection), id (cast, retract: the object), voter? (default you)} → the vote's state (as board.voteResults)  one vote at a time; others' votes show once it ends",
            false,
            vote_cmd
        ),
        spec!(
            "board.voteResults",
            "Voting results",
            "{voter?} → {state: off|running|ended, votesPerPerson, on, by, left (your votes left), mine [{id, votes}], voters, results? [{id, name, votes, rank, voters: [{voter, name (null: someone who left the live session), votes}]}] (once ended, most votes first, with who voted for each)}",
            false,
            |s, p| {
                require(s, "board.voteResults")?;
                let who = voter(s, p);
                vote_json(s, &who)
            }
        ),
    ]
}

/// Milliseconds since the Unix epoch (0 if the clock is before it).
pub fn now_ms() -> u64 {
    web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// `left` milliseconds as a clock: "4:05", "1:02:03" (whole seconds, rounded up).
pub fn clock(left: u64) -> String {
    let s = left.div_ceil(1000);
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// The timer's state at `now`: off, running, paused or done (run out, until someone stops it).
pub fn timer_state(t: Option<BoardTimer>, now: u64) -> &'static str {
    match t {
        None => "off",
        Some(BoardTimer::Paused { .. }) => "paused",
        Some(t) if t.left(now) == 0 => "done",
        Some(_) => "running",
    }
}

fn timer_json(t: Option<BoardTimer>, now: u64) -> Value {
    let left = t.map_or(0, |t| t.left(now));
    let ends = match t {
        Some(BoardTimer::Running { ends }) => Some(ends),
        _ => None,
    };
    json!({"state": timer_state(t, now), "left": left, "text": clock(left), "ends": ends})
}

/// `minutes` (or `default`) as milliseconds: more than 0, at most a day.
fn duration(cmd: &str, p: &Value, default: f64) -> Result<u64> {
    let m = match p.get("minutes").filter(|v| !v.is_null()) {
        Some(_) => num(p, "minutes").ok_or_else(|| bad(cmd, "`minutes` must be a number"))?,
        None => default,
    };
    let ms = (m * 60_000.0).round();
    if !(1000.0..=TIMER_MAX_MS as f64).contains(&ms) {
        return Err(bad(cmd, "`minutes` must be from 1 second to 1440 minutes"));
    }
    Ok(ms as u64)
}

/// The timer after `action` at `now` (`Ok(None)`: stopped).
pub fn timer_after(cmd: &str, t: Option<BoardTimer>, action: &str, p: &Value, now: u64) -> Result<Option<BoardTimer>> {
    let off = || EngineError::Disabled(cmd.into(), "there is no timer: start one".into());
    Ok(match action {
        "start" => Some(BoardTimer::Running { ends: now.saturating_add(duration(cmd, p, START_MINUTES)?) }),
        "stop" => None,
        "pause" => match t {
            Some(r @ BoardTimer::Running { .. }) if r.left(now) > 0 => Some(BoardTimer::Paused { left: r.left(now) }),
            _ => return Err(EngineError::Disabled(cmd.into(), "the timer isn't running".into())),
        },
        "resume" => match t {
            Some(BoardTimer::Paused { left }) => Some(BoardTimer::Running { ends: now.saturating_add(left) }),
            _ => return Err(EngineError::Disabled(cmd.into(), "the timer isn't paused".into())),
        },
        "add" => {
            let more = duration(cmd, p, ADD_MINUTES)?;
            let left = t.ok_or_else(off)?.left(now).saturating_add(more).min(TIMER_MAX_MS);
            match t {
                Some(BoardTimer::Paused { .. }) => Some(BoardTimer::Paused { left }),
                _ => Some(BoardTimer::Running { ends: now.saturating_add(left) }),
            }
        }
        other => return Err(bad(cmd, format!("unknown action `{other}` (start|pause|resume|add|stop)"))),
    })
}

fn timer_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    require(s, "board.timer")?;
    let action = req_string("board.timer", p, "action")?;
    let now = now_ms();
    let t = timer_after("board.timer", s.doc()?.timer, &action, p, now)?;
    if s.doc()?.timer != t {
        s.doc_mut()?.timer = t;
    }
    Ok(timer_json(t, now))
}

/// Who votes: `voter` from the params, else you as the live session knows you (your client id
/// there, so two people of the same name vote apart), else your name.
pub fn voter(s: &Session, p: &Value) -> String {
    if let Some(v) = string(p, "voter").map(|v| v.trim().chars().filter(|c| !c.is_control()).take(MAX_VOTER).collect::<String>())
        && !v.is_empty()
    {
        return v;
    }
    let live = s.ds().ok().filter(|ds| s.live_doc(ds.uid)).and_then(|ds| s.live_of(ds.uid).info.as_ref());
    match live {
        Some(info) => format!("peer {}", info.me),
        None => crate::cmd::comment::user_name(s),
    }
}

fn running<'a>(cmd: &str, doc: &'a Document) -> Result<&'a Vote> {
    match &doc.vote {
        Some(v) if !v.ended => Ok(v),
        _ => Err(EngineError::Disabled(cmd.into(), "no vote is running".into())),
    }
}

fn vote_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "board.vote";
    require(s, CMD)?;
    let action = req_string(CMD, p, "action")?;
    let who = voter(s, p);
    match action.as_str() {
        "start" => start_vote(s, p, CMD)?,
        "cast" | "retract" => change_ballot(s, p, CMD, &who, action == "cast")?,
        "end" => {
            running(CMD, s.doc()?)?;
            if let Some(v) = s.doc_mut()?.vote.as_mut() {
                v.ended = true;
            }
        }
        "clear" => {
            let doc = s.doc()?;
            if doc.vote.is_some() || !doc.votes.is_empty() {
                let doc = s.doc_mut()?;
                doc.vote = None;
                doc.votes.clear();
            }
        }
        other => return Err(bad(CMD, format!("unknown action `{other}` (start|cast|retract|end|clear)"))),
    }
    vote_json(s, &who)
}

/// `board.vote {action: start}`: a new vote (votes per person, on what), clearing the last results.
fn start_vote(s: &mut Session, p: &Value, cmd: &str) -> Result<()> {
    if s.doc()?.vote.as_ref().is_some_and(|v| !v.ended) {
        return Err(EngineError::Disabled(cmd.into(), "a vote is already running: end it first".into()));
    }
    let per = match p.get("votes").filter(|v| !v.is_null()) {
        Some(_) => num(p, "votes")
            .filter(|n| n.fract() == 0.0 && (1.0..=f64::from(VOTES_MAX)).contains(n))
            .ok_or_else(|| bad(cmd, "`votes` must be a whole number from 1 to 99"))?,
        None => f64::from(VOTES),
    } as u32;
    let scope = match string(p, "on").as_deref().unwrap_or("all") {
        "all" => VoteScope::All,
        "stickies" => VoteScope::Stickies,
        "selection" => {
            let doc = s.doc()?;
            let ids: Vec<NodeId> = req_targets(cmd, s, p)?.into_iter().filter(|i| VoteScope::All.takes(doc, *i)).collect();
            if ids.is_empty() {
                return Err(bad(cmd, "pick the objects to vote on (not pages)"));
            }
            VoteScope::Only { ids }
        }
        other => return Err(bad(cmd, format!("unknown `on` `{other}` (all|stickies|selection)"))),
    };
    let by = crate::board::author(s);
    let doc = s.doc_mut()?;
    doc.votes.clear();
    doc.vote = Some(Vote { per_person: per, scope, ended: false, by });
    Ok(())
}

/// `board.vote {action: cast|retract, id}`: one vote of `who`'s placed on (`cast`) or taken back
/// from an object, within the running vote's scope and votes per person.
fn change_ballot(s: &mut Session, p: &Value, cmd: &str, who: &str, cast: bool) -> Result<()> {
    let target = id(p, "id").ok_or_else(|| bad(cmd, "`id` must be the object to vote on"))?;
    let doc = s.doc()?;
    let vote = running(cmd, doc)?;
    let mut ballot: Vec<NodeId> = vote.counted(doc, doc.votes.get(who).map_or(&[], Vec::as_slice)).collect();
    if cast {
        if !vote.scope.takes(doc, target) {
            return Err(match doc.node(target) {
                None => EngineError::NoNode(target),
                Some(_) => EngineError::Disabled(cmd.into(), "this vote isn't on that object".into()),
            });
        }
        if ballot.len() >= vote.per_person as usize {
            return Err(EngineError::Disabled(cmd.into(), "you have no votes left: remove one first".into()));
        }
        ballot.push(target);
    } else {
        let i =
            ballot.iter().rposition(|i| *i == target).ok_or_else(|| EngineError::Disabled(cmd.into(), "you have no vote on that object".into()))?;
        ballot.remove(i);
    }
    let votes = &mut s.doc_mut()?.votes;
    if ballot.is_empty() {
        votes.remove(who);
    } else {
        votes.insert(who.to_string(), ballot);
    }
    Ok(())
}

/// `who`'s votes per object, in the order first voted on.
pub fn mine(doc: &Document, who: &str) -> Vec<(NodeId, u32)> {
    let (Some(vote), Some(ballot)) = (&doc.vote, doc.votes.get(who)) else { return Vec::new() };
    let mut out: Vec<(NodeId, u32)> = Vec::new();
    for id in vote.counted(doc, ballot) {
        match out.iter_mut().find(|(i, _)| *i == id) {
            Some((_, n)) => *n += 1,
            None => out.push((id, 1)),
        }
    }
    out
}

/// What an object is called in the results: its text (a sticky's note), else its name.
pub fn object_name(doc: &Document, id: NodeId) -> String {
    let text = text_of(doc, id).and_then(|t| doc.node(t)).and_then(|n| n.kind.text()).map(|t| t.characters.trim().to_string());
    let name = text.filter(|t| !t.is_empty()).or_else(|| doc.node(id).map(|n| n.name.clone())).unwrap_or_default();
    let line = name.lines().next().unwrap_or_default();
    let short: String = line.chars().take(60).collect();
    if short.len() < name.len() { format!("{short}…") } else { short }
}

/// Who put votes on `id`, and how many each, by voter (as the ballots key them).
pub fn voters_of(doc: &Document, vote: &Vote, id: NodeId) -> Vec<(String, u32)> {
    doc.votes
        .iter()
        .filter_map(|(who, ballot)| {
            let n = vote.counted(doc, ballot).filter(|v| *v == id).count();
            (n > 0).then(|| (who.clone(), u32::try_from(n).unwrap_or(u32::MAX)))
        })
        .collect()
}

/// The name voter `key` goes by: in a live session a client id's ("peer 3") is the person's
/// name there (yours or another's); `None` for one who isn't in the session any more. Any other
/// key is a name already (yours offline, or a `voter` given to `board.vote`).
pub fn voter_name(s: &Session, key: &str) -> Option<String> {
    let Some(client) = key.strip_prefix("peer ").and_then(|n| n.parse::<u32>().ok()) else { return Some(key.to_string()) };
    let live = s.ds().ok().filter(|ds| s.live_doc(ds.uid)).map(|ds| s.live_of(ds.uid))?;
    match &live.info {
        Some(info) if info.me == client => Some(info.name.clone()),
        _ => live.peers.get(&client).map(|p| p.name.clone()),
    }
}

fn vote_json(s: &Session, who: &str) -> Result<Value> {
    let doc = s.doc()?;
    let Some(vote) = &doc.vote else { return Ok(json!({"state": "off"})) };
    let on = match &vote.scope {
        VoteScope::All => json!("all"),
        VoteScope::Stickies => json!("stickies"),
        VoteScope::Only { ids } => json!({"selection": ids}),
    };
    let mine = mine(doc, who);
    let used: u32 = mine.iter().map(|m| m.1).sum();
    let voters = doc.votes.values().filter(|b| vote.counted(doc, b).next().is_some()).count();
    let mut out = json!({
        "state": if vote.ended { "ended" } else { "running" },
        "votesPerPerson": vote.per_person,
        "on": on,
        "by": vote.by,
        "left": vote.per_person.saturating_sub(used),
        "mine": mine.iter().map(|(id, n)| json!({"id": id, "votes": n})).collect::<Vec<_>>(),
        "voters": voters,
    });
    // Who voted for what shows once the vote ends; until then each person sees only their own.
    if vote.ended {
        let voters = |id| {
            voters_of(doc, vote, id).into_iter().map(|(key, n)| json!({"voter": key, "name": voter_name(s, &key), "votes": n})).collect::<Vec<_>>()
        };
        let results: Vec<Value> = ranked(doc, vote)
            .into_iter()
            .map(|(rank, id, n)| json!({"id": id, "name": object_name(doc, id), "votes": n, "rank": rank, "voters": voters(id)}))
            .collect();
        out["results"] = json!(results);
    }
    Ok(out)
}

/// The objects with votes, most first, each with its rank (ties share one: "1, 2, 2, 4").
pub fn ranked(doc: &Document, vote: &Vote) -> Vec<(usize, NodeId, u32)> {
    let mut last = None;
    let mut rank = 0;
    vote.tally(doc, &doc.votes)
        .into_iter()
        .enumerate()
        .map(|(i, (id, n))| {
            if last != Some(n) {
                rank = i + 1;
                last = Some(n);
            }
            (rank, id, n)
        })
        .collect()
}

/// Where object `b`'s vote badge sits (its top-right corner) and its radius, in page units at
/// `zoom` screen pixels per unit.
pub fn badge(b: Rect, zoom: f64) -> (Point, f64) {
    (Point::new(b.x1, b.y0), BADGE_PX / zoom.max(1e-6))
}

impl Session {
    /// A press with the Move tool while a vote runs: on one of your vote badges, or with Alt, it
    /// takes that vote back; on an object the vote is on, it puts a vote there. False when the
    /// press is for the tool (no vote running, or nothing to vote on there).
    pub(crate) fn vote_down(&mut self, ev: &PointerEvent) -> Result<bool> {
        let doc = self.doc()?;
        let Some(vote) = doc.vote.as_ref().filter(|v| !v.ended) else { return Ok(false) };
        let who = voter(self, &Value::Null);
        let p = ev.p();
        let on_badge = mine(doc, &who).into_iter().map(|(id, _)| id).find(|id| {
            doc.world_bounds(*id).is_some_and(|b| {
                let (c, r) = badge(b, ev.zoom);
                (p - c).hypot() <= r
            })
        });
        let hit = || {
            let leaf = self.hit(doc, self.page().ok()?, p, 2.0 / ev.zoom.max(1e-6))?;
            vote.scope.target(doc, leaf, figcraft_doc::geometry::select_target(doc, leaf, &[], false))
        };
        let (action, target) = match on_badge {
            Some(id) => ("retract", id),
            None => match hit() {
                Some(id) => (if ev.alt { "retract" } else { "cast" }, id),
                None => return Ok(false),
            },
        };
        self.execute("board.vote", &json!({"action": action, "id": target}))?;
        Ok(true)
    }
}

/// Undo and redo bring back a document as it was, but the timer and the vote aren't undoable:
/// keep them as they are now (`from`).
pub(crate) fn keep(from: &Document, to: &mut Arc<Document>) {
    if from.timer != to.timer || from.vote != to.vote || from.votes != to.votes {
        let d = Arc::make_mut(to);
        d.timer = from.timer;
        d.vote = from.vote.clone();
        d.votes = from.votes.clone();
    }
}
