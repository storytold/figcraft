//! Where an AI agent attached over MCP works, for the app to show like a person in a live
//! session: its name and the layers it last edited or selected, until a short while after its last
//! command ([`Session::execute_as`]). Display only: it is no undo step, it is never saved, and it is
//! not presence a live session sends (the agent shows there only if it joins one itself).

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;
use std::time::Duration;

use figcraft_doc::{Document, NodeId};
use serde_json::{Value, json};
use web_time::Instant;

use crate::{DocState, Session, changed_nodes};

/// How long an agent's presence shows after its last command.
pub const LINGER: Duration = Duration::from_secs(8);
/// Most layers an agent's presence names.
pub const MAX_LAYERS: usize = 500;
/// How long a named agent may stay without a request before it is taken as gone. An agent is a
/// name, not a connection, and a client that stops (killed, out of budget) sends no `agent.leave`:
/// without this, it would stay there for good, and keep the others working apart.
pub const AGENT_IDLE: Duration = Duration::from_secs(30 * 60);

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

/// The page and selection one side works with, and the editing modes that hang on them: the
/// window's, or a named agent's own while several agents work in the session.
#[derive(Clone, Debug)]
struct Slot {
    page: NodeId,
    selection: Vec<NodeId>,
    text_edit: Option<crate::TextEdit>,
    vector_edit: Option<crate::vectoredit::VectorEdit>,
    rotation_origin: Option<crate::rotation_origin::RotationOrigin>,
    last_duplicate: Option<crate::cmd::edit::Duplicated>,
}

impl Slot {
    /// A new agent starts where the window is: its page and its selection, no editing mode.
    fn of(ds: &DocState) -> Self {
        Self { page: ds.page, selection: ds.selection.clone(), text_edit: None, vector_edit: None, rotation_origin: None, last_duplicate: None }
    }

    /// Exchange what this slot holds with what the document shows.
    fn swap(&mut self, ds: &mut DocState) {
        std::mem::swap(&mut self.page, &mut ds.page);
        std::mem::swap(&mut self.selection, &mut ds.selection);
        std::mem::swap(&mut self.text_edit, &mut ds.text_edit);
        std::mem::swap(&mut self.vector_edit, &mut ds.vector_edit);
        std::mem::swap(&mut self.rotation_origin, &mut ds.rotation_origin);
        std::mem::swap(&mut self.last_duplicate, &mut ds.last_duplicate);
    }
}

/// The named AI agents of a session. One agent alone works on the window's page and selection, as
/// the user does. From the moment a second one is there (or once it asked with `agent.join`), an
/// agent works apart: on its own page and selection in each file, which the window never shows.
#[derive(Debug, Default)]
pub struct Agents {
    /// Who is there, in the order they came ([`agent_name`](crate::cmd::comment::agent_name)).
    present: Vec<String>,
    /// Who works apart; an agent stays apart until it leaves.
    apart: BTreeSet<String>,
    /// The page and selection of an agent working apart, per (agent, `DocState::uid`).
    slots: BTreeMap<(String, u64), Slot>,
    /// When the last request of each one came, for [`AGENT_IDLE`].
    seen: BTreeMap<String, Instant>,
    /// The agent whose call is running ([`Session::with_agent`]).
    current: Option<String>,
}

/// What [`Session::enter_slot`] took from the document, for [`Session::leave_slot`] to put back.
struct Entered {
    uid: u64,
    /// The window's page and selection while the agent's are in the document.
    window: Slot,
    view: crate::ViewState,
    /// The document, and the agent's page and selection, when the call started.
    doc: Arc<Document>,
    was: (NodeId, Vec<NodeId>),
    /// The newest undo step was not one of this document state when the call started.
    no_step: bool,
}

impl Session {
    /// Run `f` for the AI agent named `agent` (nothing changes for `None` or an empty name). An
    /// agent that works apart has its own page and selection in the file in front while `f` runs,
    /// so commands that act on "the current page" or "the selection" act on the agent's; the
    /// window's page, selection and view are put back afterwards, whatever `f` did.
    pub fn with_agent<T>(&mut self, agent: Option<&str>, f: impl FnOnce(&mut Session) -> T) -> T {
        let name = agent.map(crate::cmd::comment::agent_name).filter(|n| !n.is_empty());
        let Some(name) = name.filter(|_| self.agents.current.is_none()) else { return f(self) };
        self.agent_seen(&name);
        let entered = self.enter_slot(&name);
        self.agents.current = Some(name.clone());
        let r = f(self);
        self.agents.current = None;
        if let Some(e) = entered {
            self.leave_slot(name, e);
        }
        r
    }

    /// Note that `name` is there. `agent.joined` events start with the second agent (its own and
    /// the first one's), so a single agent's event log reads as before.
    fn agent_seen(&mut self, name: &str) {
        let now = Instant::now();
        self.agents.seen.insert(name.to_string(), now);
        self.agents_gone_at(now);
        if self.agents.present.iter().any(|n| n == name) {
            return;
        }
        self.agents.present.push(name.to_string());
        let announce = match self.agents.present.len() {
            1 => Vec::new(),
            2 => self.agents.present.clone(),
            _ => vec![name.to_string()],
        };
        for agent in announce {
            self.record_event("agent.joined", json!({"agent": agent}));
        }
    }

    /// Put the agent's page and selection into the file in front, if it works apart.
    fn enter_slot(&mut self, name: &str) -> Option<Entered> {
        if !self.agents.apart.contains(name) {
            if !self.agents.present.iter().any(|n| n != name) {
                return None;
            }
            self.agents.apart.insert(name.to_string());
        }
        let ds = self.docs.get_mut(self.active)?;
        let mut slot = self.agents.slots.remove(&(name.to_string(), ds.uid)).unwrap_or_else(|| Slot::of(ds));
        if !ds.doc.pages.contains(&slot.page) {
            // Its page was deleted meanwhile: it is back where the window is.
            slot = Slot { selection: Vec::new(), ..Slot::of(ds) };
        }
        let doc = ds.doc.clone();
        let no_step = !ds.history.undo.last().is_some_and(|e| Arc::ptr_eq(&e.doc, &doc));
        let (uid, view) = (ds.uid, ds.view.clone());
        slot.swap(ds);
        ds.clean_selection();
        let was = (ds.page, ds.selection.clone());
        Some(Entered { uid, window: slot, view, doc, was, no_step })
    }

    /// Keep the agent's page and selection and put the window's back.
    fn leave_slot(&mut self, name: String, e: Entered) {
        // The call closed the file: the agent has nothing to keep in it.
        let Some(ds) = self.docs.iter_mut().find(|d| d.uid == e.uid) else { return };
        let mut slot = e.window;
        slot.swap(ds);
        if slot.page != ds.page {
            // The agent is on another page: what it did to the view (a page change fits it) is
            // not what the window shows.
            ds.view = e.view;
        }
        // The user's undo of the agent's step comes back to what the window showed, not to the
        // agent's page.
        let (page, selection) = (ds.page, ds.selection.clone());
        if e.no_step
            && let Some(step) = ds.history.undo.last_mut().filter(|s| Arc::ptr_eq(&s.doc, &e.doc))
        {
            step.page = page;
            step.selection = selection;
        }
        if !ds.doc.pages.contains(&ds.page)
            && let Some(first) = ds.doc.first_page()
        {
            ds.page = first;
        }
        ds.clean_selection();
        let now = (slot.page, slot.selection.clone());
        self.agents.slots.insert((name.clone(), e.uid), slot);
        if now != e.was {
            self.record_event("agent.selection", json!({"agent": name, "page": now.0, "selection": now.1}));
        }
    }

    /// Undo and redo put the whole document back, so with other agents at work they would take
    /// away another agent's edits: an agent's are refused then. The user's never are.
    pub(crate) fn refuse_shared_undo(&self, id: &str) -> crate::Result<()> {
        let others = |a: &String| self.agents.present.iter().any(|n| n != a);
        if matches!(id, "edit.undo" | "edit.redo") && self.agents.current.as_ref().is_some_and(others) {
            let why = "the history is shared by everyone in the file: with other agents at work, correct with a new edit instead";
            return Err(crate::EngineError::Disabled(id.to_string(), why.into()));
        }
        Ok(())
    }

    /// `agent.join`: `name` is there and works apart from now on, even alone.
    pub fn agent_join(&mut self, name: &str) -> crate::Result<Value> {
        let name = crate::cmd::comment::agent_name(name);
        if name.is_empty() {
            return Err(crate::bad("agent.join", "`agent` (a name) is required"));
        }
        self.agent_seen(&name);
        self.agents.apart.insert(name.clone());
        Ok(json!({"agent": name, "agents": self.agent_list()["agents"]}))
    }

    /// `agent.leave`: forget `name`, its pages and its selections.
    pub fn agent_leave(&mut self, name: &str) -> crate::Result<Value> {
        let name = crate::cmd::comment::agent_name(name);
        if name.is_empty() {
            return Err(crate::bad("agent.leave", "`agent` (a name) is required"));
        }
        self.agent_forget(&name, None);
        Ok(json!({"left": name}))
    }

    /// Forget `name`, its pages and its selections. `reason` is why, when it did not ask to leave.
    fn agent_forget(&mut self, name: &str, reason: Option<&str>) {
        let several = self.agents.present.len() > 1;
        let was_there = self.agents.present.iter().any(|n| n == name);
        self.agents.present.retain(|n| n != name);
        self.agents.apart.remove(name);
        self.agents.slots.retain(|(n, _), _| n != name);
        self.agents.seen.remove(name);
        if was_there && several {
            let data = match reason {
                Some(reason) => json!({"agent": name, "reason": reason}),
                None => json!({"agent": name}),
            };
            self.record_event("agent.left", data);
        }
    }

    /// Forget the agents whose last request is [`AGENT_IDLE`] old at `now`, as `agent.leave` does
    /// (`agent.left` then says `reason: "idle"`); returns their names. One that comes back is a new
    /// agent: it starts where the window is. The agent whose call is running is never one of them.
    pub(crate) fn agents_gone_at(&mut self, now: Instant) -> Vec<String> {
        let agents = &self.agents;
        let gone: Vec<String> = agents
            .seen
            .iter()
            .filter(|(n, at)| now.saturating_duration_since(**at) >= AGENT_IDLE && agents.current.as_ref() != Some(*n))
            .map(|(n, _)| n.clone())
            .collect();
        for name in &gone {
            self.agent_forget(name, Some("idle"));
        }
        gone
    }

    /// `agent.list`: who is there, and for those working apart their page and selection in the
    /// file in front. Agents silent for [`AGENT_IDLE`] are forgotten first.
    pub fn agent_list(&mut self) -> Value {
        self.agents_gone_at(Instant::now());
        let uid = self.ds().ok().map(|d| d.uid);
        let agents: Vec<Value> = self
            .agents
            .present
            .iter()
            .map(|n| {
                let slot = uid.and_then(|u| self.agents.slots.get(&(n.clone(), u)));
                json!({
                    "agent": n,
                    "apart": self.agents.apart.contains(n),
                    "page": slot.map(|s| s.page),
                    "selection": slot.map(|s| s.selection.clone()),
                })
            })
            .collect();
        json!({"agents": agents})
    }
}
