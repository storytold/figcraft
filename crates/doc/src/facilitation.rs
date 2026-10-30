//! A board's facilitation state, shared by everyone on it: the countdown timer and the vote.
//!
//! The timer keeps when it ends (or, paused, how long is left), never the ticks: each client works
//! out the time left from its own clock. A vote's ballots are kept per voter, so two people voting
//! at once change different entries.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{BoardItem, Document, NodeId};

/// The longest a timer runs (24 hours, in milliseconds).
pub const TIMER_MAX_MS: u64 = 24 * 60 * 60 * 1000;
/// The most votes one person gets in a vote.
pub const VOTES_MAX: u32 = 99;

/// A board's countdown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum BoardTimer {
    /// Counting down to `ends` (milliseconds since the Unix epoch).
    Running { ends: u64 },
    /// Stopped with `left` milliseconds to go.
    Paused { left: u64 },
}

impl BoardTimer {
    /// Milliseconds left at `now` (ms since the Unix epoch); 0 once it has run out.
    pub fn left(&self, now: u64) -> u64 {
        match *self {
            BoardTimer::Running { ends } => ends.saturating_sub(now).min(TIMER_MAX_MS),
            BoardTimer::Paused { left } => left.min(TIMER_MAX_MS),
        }
    }
}

/// What a vote is on.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum VoteScope {
    /// Any object on the board.
    #[default]
    All,
    /// Sticky notes only.
    Stickies,
    /// The objects that were selected when it started.
    Only { ids: Vec<NodeId> },
}

impl VoteScope {
    /// The object a vote at layer `leaf` goes to: the nearest layer from it up that the vote is
    /// on (`All`: `whole`, the object a click there picks).
    pub fn target(&self, doc: &Document, leaf: NodeId, whole: NodeId) -> Option<NodeId> {
        if matches!(self, VoteScope::All) {
            return self.takes(doc, whole).then_some(whole);
        }
        std::iter::once(leaf).chain(doc.ancestors(leaf)).find(|id| self.takes(doc, *id))
    }

    /// Whether object `id` takes votes.
    pub fn takes(&self, doc: &Document, id: NodeId) -> bool {
        let Some(n) = doc.node(id) else { return false };
        if doc.pages.contains(&id) || n.parent.is_none() {
            return false;
        }
        match self {
            VoteScope::All => true,
            VoteScope::Stickies => matches!(n.board, Some(BoardItem::Sticky { .. })),
            VoteScope::Only { ids } => ids.contains(&id),
        }
    }
}

/// A vote: how many votes each person has, what they go on, and whether it has ended (then
/// everyone's votes show). The ballots are [`Document::votes`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Vote {
    pub per_person: u32,
    #[serde(default)]
    pub scope: VoteScope,
    #[serde(default)]
    pub ended: bool,
    /// Who started it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub by: String,
}

/// Every voter's ballot: the objects they put their votes on (once per vote).
pub type Ballots = BTreeMap<String, Vec<NodeId>>;

/// The most ended votes a board keeps the results of (the oldest goes first).
pub const VOTE_HISTORY_MAX: usize = 20;

/// An ended vote's results, kept in the file: the vote, everyone's ballots and the names the
/// voters went by when it ended (a live session's client ids mean nothing once it's over).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoteRecord {
    pub vote: Vote,
    #[serde(default)]
    pub ballots: Ballots,
    /// Voter key → name, for the keys that aren't names themselves.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub names: BTreeMap<String, String>,
    /// When it ended (milliseconds since the Unix epoch).
    #[serde(default)]
    pub ended: u64,
}

impl Vote {
    /// The votes of `ballot` that count: on objects that are still there, up to the cap.
    pub fn counted<'a>(&self, doc: &'a Document, ballot: &'a [NodeId]) -> impl Iterator<Item = NodeId> + 'a {
        ballot.iter().copied().filter(|id| doc.node(*id).is_some()).take(self.per_person.min(VOTES_MAX) as usize)
    }

    /// The votes each object got, most first (ties in layer id order).
    pub fn tally(&self, doc: &Document, ballots: &Ballots) -> Vec<(NodeId, u32)> {
        let mut counts: BTreeMap<NodeId, u32> = BTreeMap::new();
        for ballot in ballots.values() {
            for id in self.counted(doc, ballot) {
                *counts.entry(id).or_default() += 1;
            }
        }
        let mut out: Vec<(NodeId, u32)> = counts.into_iter().collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        out
    }
}
