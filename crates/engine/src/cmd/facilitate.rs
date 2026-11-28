//! Running a session on a board: a countdown everyone sees and votes on objects.
//!
//! Both live in the document ([`figcraft_doc::facilitation`]) so they reach everyone in a live
//! session, and neither is an undo step: undoing an edit leaves the timer and the vote as they are.

use std::collections::BTreeMap;
use std::sync::Arc;

use figcraft_doc::{Ballots, BoardTimer, Document, NodeId, TIMER_MAX_MS, VOTE_HISTORY_MAX, VOTES_MAX, Vote, VoteRecord, VoteScope};
use figcraft_geom::{Point, Rect};
use serde_json::{Value, json};

use crate::board::{require, text_of};
use crate::cmd::CommandSpec;
use crate::params::{id, num, opt_index, req_string, req_targets, string};
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
            "{action: start|cast|retract|end|clear|delete, votes? (start: per person, 1–99, default 3), on? (start: all|stickies|selection; default all), ids? (start with on: selection; default the selection), id (cast, retract: the object), index (delete: which kept results, 0 the oldest), voter? (default you)} → the vote's state (as board.voteResults)  one vote at a time; others' votes show once it ends, and its results are kept in the file (the last 20 votes'); clear takes the results off the board, delete removes kept ones",
            false,
            vote_cmd
        ),
        spec!(
            "board.voteResults",
            "Voting results",
            "{voter?} → {state: off|running|ended, votesPerPerson, on, by, left (your votes left), mine [{id, votes}], voters, results? [{id, name, votes, rank, voters: [{voter, name (null: someone who left the live session), votes}]}] (once ended, most votes first, with who voted for each), sessions [{index (in the kept results, 0 the oldest; null for one not kept), onBoard, ended (ms since 1970), votesPerPerson, on, by, results}] (every ended vote's results, newest first)}",
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
        "end" => end_vote(s, CMD)?,
        "clear" => {
            let doc = s.doc()?;
            if doc.vote.is_some() || !doc.votes.is_empty() {
                let doc = s.doc_mut()?;
                doc.vote = None;
                doc.votes.clear();
            }
        }
        "delete" => delete_results(s, p, CMD)?,
        other => return Err(bad(CMD, format!("unknown action `{other}` (start|cast|retract|end|clear|delete)"))),
    }
    vote_json(s, &who)
}

/// `board.vote {action: end}`: everyone's votes show, and the results are kept in the file with
/// the names the voters go by now (the last [`VOTE_HISTORY_MAX`] votes').
fn end_vote(s: &mut Session, cmd: &str) -> Result<()> {
    running(cmd, s.doc()?)?;
    let names = s.doc()?.votes.keys().filter_map(|k| voter_name(s, k).filter(|n| n != k).map(|n| (k.clone(), n))).collect();
    let ended = now_ms();
    let doc = s.doc_mut()?;
    if let Some(v) = doc.vote.as_mut() {
        v.ended = true;
        doc.vote_history.push(VoteRecord { vote: v.clone(), ballots: doc.votes.clone(), names, ended });
        let extra = doc.vote_history.len().saturating_sub(VOTE_HISTORY_MAX);
        doc.vote_history.drain(..extra);
    }
    Ok(())
}

/// `board.vote {action: delete, index}`: kept results deleted from the file (`index` 0 the
/// oldest, as `board.voteResults` lists them); those on the board now are cleared from it too.
fn delete_results(s: &mut Session, p: &Value, cmd: &str) -> Result<()> {
    let doc = s.doc()?;
    if doc.vote_history.is_empty() {
        return Err(EngineError::Disabled(cmd.into(), "no results are kept".into()));
    }
    let last = doc.vote_history.len() - 1;
    let index = opt_index(cmd, p, "index")?.filter(|i| *i <= last).ok_or_else(|| bad(cmd, format!("`index` is from 0 to {last}")))?;
    let on_board = doc.vote_history.get(index).is_some_and(|r| on_board(doc, r));
    let doc = s.doc_mut()?;
    doc.vote_history.remove(index);
    if on_board {
        doc.vote = None;
        doc.votes.clear();
    }
    Ok(())
}

/// Whether kept results `r` are the ended vote on the board now.
fn on_board(doc: &Document, r: &VoteRecord) -> bool {
    doc.vote.as_ref().is_some_and(|v| v.ended && *v == r.vote) && doc.votes == r.ballots
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

/// Who put votes on `id` in `ballots`, and how many each, by voter (as the ballots key them).
pub fn voters_of(doc: &Document, vote: &Vote, ballots: &Ballots, id: NodeId) -> Vec<(String, u32)> {
    ballots
        .iter()
        .filter_map(|(who, ballot)| {
            let n = vote.counted(doc, ballot).filter(|v| *v == id).count();
            (n > 0).then(|| (who.clone(), u32::try_from(n).unwrap_or(u32::MAX)))
        })
        .collect()
}

/// One object in an ended vote's results.
#[derive(Clone, Debug, PartialEq)]
pub struct ResultRow {
    /// Ties share a rank ("1, 2, 2, 4").
    pub rank: usize,
    pub id: NodeId,
    /// The sticky's text or the layer's name ([`object_name`]).
    pub name: String,
    pub votes: u32,
    /// Who voted for it: their key, the name they go by (`None`: someone who left a live session
    /// before the vote ended) and their votes on it.
    pub voters: Vec<(String, Option<String>, u32)>,
}

/// An ended vote's results: kept in the file (`index` in [`Document::vote_history`]), on the
/// board now (`on_board`: its badges show), or both.
#[derive(Clone, Debug, PartialEq)]
pub struct VoteSession {
    pub index: Option<usize>,
    pub on_board: bool,
    /// When it ended (ms since 1970; 0 when not known).
    pub ended: u64,
    pub vote: Vote,
    pub rows: Vec<ResultRow>,
}

/// `ballots`' results under `vote`, most votes first; voters named from `names` (as they were
/// when it ended), else as the session knows them now ([`voter_name`]).
fn result_rows(s: &Session, doc: &Document, vote: &Vote, ballots: &Ballots, names: &BTreeMap<String, String>) -> Vec<ResultRow> {
    let name = |key: &str| names.get(key).cloned().or_else(|| voter_name(s, key));
    ranked(doc, vote, ballots)
        .into_iter()
        .map(|(rank, id, votes)| {
            let voters = voters_of(doc, vote, ballots, id).into_iter().map(|(key, n)| (key.clone(), name(&key), n)).collect();
            ResultRow { rank, id, name: object_name(doc, id), votes, voters }
        })
        .collect()
}

/// Every ended vote's results, newest first: those the file keeps, and the ended vote on the
/// board when the file doesn't keep it (one that ended before results were kept).
pub fn sessions(s: &Session) -> Result<Vec<VoteSession>> {
    let doc = s.doc()?;
    let mut out: Vec<VoteSession> = doc
        .vote_history
        .iter()
        .enumerate()
        .map(|(i, r)| VoteSession {
            index: Some(i),
            on_board: on_board(doc, r),
            ended: r.ended,
            vote: r.vote.clone(),
            rows: result_rows(s, doc, &r.vote, &r.ballots, &r.names),
        })
        .collect();
    if let Some(vote) = doc.vote.as_ref().filter(|v| v.ended)
        && !out.iter().any(|x| x.on_board)
    {
        let rows = result_rows(s, doc, vote, &doc.votes, &BTreeMap::new());
        out.push(VoteSession { index: None, on_board: true, ended: 0, vote: vote.clone(), rows });
    }
    out.reverse();
    Ok(out)
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

fn scope_json(scope: &VoteScope) -> Value {
    match scope {
        VoteScope::All => json!("all"),
        VoteScope::Stickies => json!("stickies"),
        VoteScope::Only { ids } => json!({"selection": ids}),
    }
}

fn rows_json(rows: &[ResultRow]) -> Value {
    let voters = |r: &ResultRow| r.voters.iter().map(|(key, name, n)| json!({"voter": key, "name": name, "votes": n})).collect::<Vec<_>>();
    json!(rows.iter().map(|r| json!({"id": r.id, "name": r.name, "votes": r.votes, "rank": r.rank, "voters": voters(r)})).collect::<Vec<_>>())
}

fn vote_json(s: &Session, who: &str) -> Result<Value> {
    let sessions = sessions(s)?;
    let kept: Vec<Value> = sessions
        .iter()
        .map(|x| {
            json!({"index": x.index, "onBoard": x.on_board, "ended": x.ended, "votesPerPerson": x.vote.per_person,
                   "on": scope_json(&x.vote.scope), "by": x.vote.by, "results": rows_json(&x.rows)})
        })
        .collect();
    let doc = s.doc()?;
    let Some(vote) = &doc.vote else { return Ok(json!({"state": "off", "sessions": kept})) };
    let on = scope_json(&vote.scope);
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
        "sessions": kept,
    });
    // Who voted for what shows once the vote ends; until then each person sees only their own.
    if let Some(on_board) = sessions.iter().find(|x| x.on_board).filter(|_| vote.ended) {
        out["results"] = rows_json(&on_board.rows);
    }
    Ok(out)
}

/// The objects with votes in `ballots`, most first, each with its rank (ties share one: "1, 2,
/// 2, 4").
pub fn ranked(doc: &Document, vote: &Vote, ballots: &Ballots) -> Vec<(usize, NodeId, u32)> {
    let mut last = None;
    let mut rank = 0;
    vote.tally(doc, ballots)
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

/// Undo and redo bring back a document as it was, but the timer, the vote and the kept results
/// aren't undoable: keep them as they are now (`from`).
pub(crate) fn keep(from: &Document, to: &mut Arc<Document>) {
    if from.timer != to.timer || from.vote != to.vote || from.votes != to.votes || from.vote_history != to.vote_history {
        let d = Arc::make_mut(to);
        d.timer = from.timer;
        d.vote = from.vote.clone();
        d.votes = from.votes.clone();
        d.vote_history = from.vote_history.clone();
    }
}
