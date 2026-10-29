//! Where an AI agent attached over MCP works, for the app to show like a person in a live
//! session: its name and the layers it last edited or selected, until a short while after its last
//! command ([`Session::execute_as`]). Display only: it is no undo step, it is never saved, and it is
//! not presence a live session sends (the agent shows there only if it joins one itself).

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use figcraft_doc::{Document, NodeId};
use web_time::Instant;

use crate::{Session, changed_nodes};

/// How long an agent's presence shows after its last command.
pub const LINGER: Duration = Duration::from_secs(8);
/// Most layers an agent's presence names.
pub const MAX_LAYERS: usize = 500;

/// An attached AI agent at work.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentPresence {
    /// As its comments are signed ("Claude (agent)").
    pub name: String,
    /// The file it last worked in (`DocState::uid`).
    pub doc: u64,
    /// The layers its last edit changed (the innermost), or that it last selected.
    pub layers: Vec<NodeId>,
    /// When its last command ran.
    pub at: Instant,
}

impl AgentPresence {
    /// How much longer it shows at `now` (zero: gone).
    pub fn left(&self, now: Instant) -> Duration {
        LINGER.saturating_sub(now.saturating_duration_since(self.at))
    }
}

/// The document and selection a command started from: (file, document, selection).
pub(crate) type Before = (u64, Arc<Document>, Vec<NodeId>);

/// The layers a command worked on, going from `before` to `after`: the innermost layers it edited
/// (a parent changes with its children's list), else the selection it changed to; `None` when it
/// did neither.
fn touched(before: &Document, after: &Document, sel_before: &[NodeId], sel_after: &[NodeId]) -> Option<Vec<NodeId>> {
    let changed: Vec<NodeId> = if std::ptr::eq(before, after) { Vec::new() } else { changed_nodes(after, before) };
    let edited: Vec<&figcraft_doc::Node> = changed.iter().filter_map(|i| after.node(*i)).filter(|n| !n.is_page()).collect();
    if !edited.is_empty() {
        let parents: HashSet<NodeId> = edited.iter().filter_map(|n| n.parent).collect();
        return Some(edited.iter().map(|n| n.id).filter(|i| !parents.contains(i)).take(MAX_LAYERS).collect());
    }
    (sel_before != sel_after).then(|| sel_after.iter().copied().take(MAX_LAYERS).collect())
}

impl Session {
    /// The document and selection before a command, for [`Session::note_agent`].
    pub(crate) fn agent_before(&self) -> Option<Before> {
        self.ds().ok().map(|d| (d.uid, d.doc.clone(), d.selection.clone()))
    }

    /// The running command is the agent's: it is at work, on what it just edited or selected
    /// (else on what it worked on before in the same file).
    pub(crate) fn note_agent(&mut self, before: Option<Before>) {
        let (Some(name), Ok(ds)) = (self.agent.clone(), self.ds()) else { return };
        let uid = ds.uid;
        let now = before.filter(|b| b.0 == uid).and_then(|(_, doc, sel)| touched(&doc, &ds.doc, &sel, &ds.selection));
        let kept = self.agent_presence.take().filter(|p| p.doc == uid && p.name == name).map(|p| p.layers);
        let layers = now.or(kept).unwrap_or_default();
        self.agent_presence = Some(AgentPresence { name, doc: uid, layers, at: Instant::now() });
    }

    /// The agent at work at `now`, if one ran a command less than [`LINGER`] before.
    pub fn agent_presence_at(&self, now: Instant) -> Option<&AgentPresence> {
        self.agent_presence.as_ref().filter(|p| !p.left(now).is_zero())
    }
}
