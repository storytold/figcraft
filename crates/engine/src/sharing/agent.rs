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
/// Most bytes of the key an agent holds its name with ([`Session::agent_admit`]).
pub const MAX_AGENT_KEY: usize = 256;

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
    /// The key each name is held with, for the names a client took with one
    /// ([`Session::agent_admit`]). Never listed, never recorded in an event.
    keys: BTreeMap<String, String>,
    /// The agent whose call is running ([`Session::with_agent`]).
    current: Option<String>,
}

/// The key a control request gives for the agent it names (`agentKey`, next to `agent`).
pub fn key_of(p: &Value) -> Option<&str> {
    p.get("agentKey").and_then(Value::as_str)
}

/// Whether two keys are the same, in a time that does not tell where they differ.
fn same_key(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
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
        // A panic in `f` ends the call like any other end, then goes on to the caller's guard:
        // the next call is its own caller's, and the window has its page and selection back.
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(self)));
        self.agents.current = None;
        if let Some(e) = entered {
            self.leave_slot(name, e);
        }
        match r {
            Ok(r) => r,
            Err(payload) => std::panic::resume_unwind(payload),
        }
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
        // Whoever worked alone until now did so on the window's page and selection. Those are its
        // own from this moment: at its next call the window may show what the user chose since.
        let alone: Vec<String> = self.agents.present.iter().filter(|n| *n != name && !self.agents.apart.contains(*n)).cloned().collect();
        for first in alone {
            for ds in &self.docs {
                self.agents.slots.insert((first.clone(), ds.uid), Slot::of(ds));
            }
            self.agents.apart.insert(first);
        }
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
        // What the agent did to the view (a page change fits it, a pan scrolls it) is not what the
        // window shows, on whichever page the agent is.
        ds.view = e.view;
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

    /// A request from a control channel names `agent` and gives `key` (its `agentKey`): is it that
    /// agent's? The first key given under a name holds it, until the agent is forgotten
    /// (`agent.leave`, or [`AGENT_IDLE`] without a request). Meanwhile a request under that name
    /// with another key, or with none, is refused. An agent is a name, not a connection: without
    /// the key, any client of the channel could work under another one's name, or make it leave
    /// and lose its pages and selections. A name no key holds is anyone's, as before agents had
    /// keys. The channel asks this before it runs anything for the agent; the session's own calls
    /// ([`Session::execute_as`], [`Session::with_agent`], [`Session::agent_join`]…) are not asked.
    pub fn agent_admit(&mut self, agent: Option<&str>, key: Option<&str>) -> crate::Result<()> {
        let name = agent.map(crate::cmd::comment::agent_name).unwrap_or_default();
        if name.is_empty() {
            return Ok(());
        }
        if key.is_some_and(|k| k.is_empty() || k.len() > MAX_AGENT_KEY) {
            return Err(crate::EngineError::Other(format!("`agentKey` is 1 to {MAX_AGENT_KEY} bytes")));
        }
        let now = Instant::now();
        // An agent that stopped without leaving no longer holds its name.
        self.agents_gone_at(now);
        let own = self.agents.keys.get(&name).map(|held| key.is_some_and(|k| same_key(held, k)));
        match (own, key) {
            (Some(true), _) | (None, None) => Ok(()),
            (Some(false), _) => Err(crate::EngineError::Other(format!(
                "the name `{name}` is held by another client: only a request with that agent's `agentKey` is made under it (work under another name)"
            ))),
            (None, Some(key)) => {
                self.agents.keys.insert(name.clone(), key.to_string());
                // The time it may stay silent runs from now, even if this request gets no further.
                self.agents.seen.entry(name).or_insert(now);
                Ok(())
            }
        }
    }

    /// [`Session::agent_admit`] for `agent.join`: the agent takes its name there, so it gives a
    /// key. Without one, the name it joined under would be anyone's.
    pub fn agent_admit_join(&mut self, agent: &str, key: Option<&str>) -> crate::Result<()> {
        if key.is_none() {
            let why = "`agentKey` is required: the key this agent holds its name with (any text only its client knows)";
            return Err(crate::bad("agent.join", why));
        }
        self.agent_admit(Some(agent), key)
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
        // Its name is free again: whoever comes next under it takes it with its own key.
        self.agents.keys.remove(name);
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
