//! The session server for one file: the single authority that orders, validates and rebroadcasts
//! batches. A pure state machine: the transport feeds it messages and delivers what it returns.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use figcraft_doc::{Document, NodeId};
use serde::{Deserialize, Serialize};

use crate::change::{Change, Entry, Prop, Target};
use crate::message::{
    Accepted, Access, CatchUp, ClientMsg, Hello, PROTOCOL, Peer, Presence, Processed, Resume, ServerMsg, Signal, reads_comments, reads_signals,
};
use crate::replica::{Mode, Replica, create_change};
use crate::{Error, Limits, docprops, same_secret};

/// A message for the transport to deliver.
#[derive(Clone, Debug, PartialEq)]
pub enum Outgoing {
    /// To one client.
    To(u32, ServerMsg),
    /// To every client.
    All(ServerMsg),
    /// To every client but one.
    Others(u32, ServerMsg),
}

/// Why a viewer's batch is refused.
pub const VIEW_ONLY: &str = "you can only view this file (your link is view-only)";

/// Why a commenter's batch that changes more than comment threads is refused.
pub const COMMENT_ONLY: &str = "you can only view and comment on this file (your link is a comment link)";

/// A new style, variable, collection or comment may not take an id in a space at or above this
/// that isn't its creator's. Client ids start above every space in the document after a restart,
/// so no member can use them up: at least this many always remain. Lower spaces stay open, for
/// entries imported from other files' sessions.
pub const NO_SESSION_SPACE: u32 = 1 << 31;

/// What changed a server's lasting state, in order: what a store appends to its log between
/// snapshots ([`Server::journaling`]) and replays over the last snapshot after a restart
/// ([`Server::replay`]). Presence and who is connected aren't in it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Journal {
    /// A client id was handed out.
    Issued { client: u32 },
    /// A batch was accepted.
    Accepted(Accepted),
    /// A client's batch was refused.
    Refused { client: u32, batch_id: u64 },
}

/// A session server for one file.
#[derive(Clone, Debug)]
pub struct Server {
    /// This session's identity (random): clients resume only into the session they were in.
    session: u64,
    state: Replica,
    seq: u64,
    token: String,
    limits: Limits,
    next_client: u32,
    peers: BTreeMap<u32, Peer>,
    presence: BTreeMap<u32, Presence>,
    acked: BTreeMap<u32, u64>,
    /// Deleted node ids: only these may be created again (by undo); new ids must come from the
    /// creating client's id space.
    deleted: BTreeSet<NodeId>,
    /// Removed document-level entries (styles, variables, collections, comments): these may be
    /// put back by anyone (undo); a new one must not have an id from another client's space.
    removed: BTreeSet<Entry>,
    /// Every client id handed out (a reconnecting client keeps its id) and what was done with
    /// its batches.
    issued: BTreeMap<u32, Processed>,
    /// The secret each client id resumes with (sent only in that client's welcome). Not saved:
    /// after a restart, the first to come back for an id that isn't connected sets it.
    secrets: BTreeMap<u32, u64>,
    /// The latest accepted batches, oldest first (a reconnecting client gets what it missed).
    log: VecDeque<Accepted>,
    /// Changes in `log`, and about how many bytes it holds ([`weight`]).
    log_changes: usize,
    log_bytes: usize,
    /// The connected clients that may not edit, and what they may do instead (view, or comment).
    limited: BTreeMap<u32, Access>,
    /// What changed the lasting state since it was last taken (`None`: not kept).
    journal: Option<Vec<Journal>>,
    /// The connected clients that read signals (cursor chat, spotlight).
    signals: BTreeSet<u32>,
    /// Who spotlights now (a newcomer is told).
    spotlight: Option<u32>,
    /// Saved history this server restored or replayed left image entries out (or may give a
    /// resuming client other bytes under a key it holds): every resuming client catches up with
    /// the snapshot, not the log.
    resync: bool,
}

/// The most client ids a session remembers for reconnecting (the oldest disconnected ones go
/// first; a forgotten client joins as new).
const MAX_REMEMBERED: usize = 65_536;

/// What a session server needs to carry on after a restart: the document, the sequence number,
/// the client-id counter (so ids are never handed out twice), the clients that can reconnect,
/// the deleted ids and removed entries that may be created again and the log of recent batches.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionState {
    pub session: u64,
    pub snapshot: Vec<Change>,
    pub seq: u64,
    pub next_client: u32,
    pub issued: BTreeMap<u32, Processed>,
    pub deleted: BTreeSet<NodeId>,
    /// Absent in states saved before entries were tracked.
    #[serde(default)]
    pub removed: BTreeSet<Entry>,
    pub log: Vec<Accepted>,
}

impl Server {
    /// A session for `doc`. Joining requires `token` (empty: no token). Client ids start above every
    /// client id space already used in the document (by layers, styles, variables, collections,
    /// modes or comments), so ids from earlier sessions are never handed out again.
    pub fn new(doc: Document, token: impl Into<String>, limits: Limits) -> Server {
        let next_client = doc.highest_id_space().saturating_add(1);
        Server {
            session: new_session_id(),
            state: Replica::new(doc),
            seq: 0,
            token: token.into(),
            limits,
            next_client,
            peers: BTreeMap::new(),
            presence: BTreeMap::new(),
            acked: BTreeMap::new(),
            deleted: BTreeSet::new(),
            removed: BTreeSet::new(),
            issued: BTreeMap::new(),
            secrets: BTreeMap::new(),
            log: VecDeque::new(),
            log_changes: 0,
            log_bytes: 0,
            limited: BTreeMap::new(),
            journal: None,
            signals: BTreeSet::new(),
            spotlight: None,
            resync: false,
        }
    }

    /// The session's state, to save (see [`restore`](Self::restore)).
    pub fn state(&self) -> Result<SessionState, Error> {
        Ok(SessionState {
            session: self.session,
            snapshot: self.state.snapshot()?,
            seq: self.seq,
            next_client: self.next_client,
            issued: self.issued.clone(),
            deleted: self.deleted.clone(),
            removed: self.removed.clone(),
            log: self.log.iter().cloned().collect(),
        })
    }

    /// A session carrying on from a saved [`state`](Self::state) (nobody is connected yet; clients
    /// reconnect with their ids). A log that doesn't lead up to the sequence number is dropped
    /// (clients then catch up with a snapshot).
    pub fn restore(state: SessionState, token: impl Into<String>, limits: Limits) -> Result<Server, Error> {
        let (replica, left_out) = Replica::from_snapshot_leaving_out(&state.snapshot)?;
        let mut server = Server::new(replica.document().clone(), token, limits);
        server.state = replica;
        server.session = state.session;
        server.seq = state.seq;
        // A counter stuck at the top (saved while an out-of-range id in the document blocked
        // every join) carries on above the ids actually handed out.
        let saved = match state.next_client {
            u32::MAX => state.issued.keys().chain(state.log.iter().map(|a| &a.author)).max().map_or(0, |c| c.saturating_add(1)),
            n => n,
        };
        server.next_client = server.next_client.max(saved);
        server.issued = state.issued.into_iter().filter(|(id, _)| *id < server.next_client).collect();
        server.deleted = state.deleted;
        server.removed = state.removed;
        let contiguous = state.log.windows(2).all(|w| matches!(w, [a, b] if a.seq + 1 == b.seq));
        // History saved before the image checks may hold image entries they refuse. The snapshot
        // opens without them, and a saved log putting images may give a resuming client other
        // bytes under a key it holds, which this state can't check: either way resuming clients
        // catch up with the snapshot.
        let puts_images = state.log.iter().flat_map(|a| &a.changes).any(Change::is_image);
        server.resync = left_out > 0 || puts_images;
        if contiguous && state.log.last().is_none_or(|a| a.seq == state.seq) {
            for a in state.log {
                server.remember(a);
            }
        }
        Ok(server)
    }

    /// Keep a [`Journal`] of what changes the lasting state (`on`), or stop (dropping what wasn't
    /// taken).
    pub fn journaling(&mut self, on: bool) {
        self.journal = on.then(|| self.journal.take().unwrap_or_default());
    }

    /// What changed the lasting state since the last call, oldest first.
    pub fn take_journal(&mut self) -> Vec<Journal> {
        self.journal.as_mut().map(std::mem::take).unwrap_or_default()
    }

    fn note(&mut self, j: Journal) {
        if let Some(v) = self.journal.as_mut() {
            v.push(j);
        }
    }

    /// Apply a journal entry over a restored state (after a restart). What the state already has
    /// (a batch at or below its sequence number, a client id it knows, a refusal it noted)
    /// changes nothing; a batch that doesn't follow the sequence number, or doesn't apply, is an
    /// error (the rest of that log can't be trusted). Replayed entries aren't journaled again.
    pub fn replay(&mut self, j: Journal) -> Result<(), Error> {
        match j {
            Journal::Issued { client } => {
                if let std::collections::btree_map::Entry::Vacant(v) = self.issued.entry(client) {
                    v.insert(Processed::default());
                    self.next_client = self.next_client.max(client.saturating_add(1));
                    self.forget_oldest();
                }
                Ok(())
            }
            Journal::Accepted(acc) if acc.seq <= self.seq => Ok(()),
            Journal::Accepted(mut acc) => {
                if acc.seq != self.seq + 1 {
                    return Err(Error::Protocol(format!("batch {} after {}", acc.seq, self.seq)));
                }
                // A log written before the image checks may hold an image entry they refuse: the
                // batch replays without it, so the edits after it aren't dropped with the rest of
                // the log, and resuming clients catch up with the snapshot.
                if acc.changes.iter().any(Change::is_image) && self.state.clone().apply_kept(&mut acc.changes, Change::is_image)? > 0 {
                    self.resync = true;
                }
                self.accept(acc.author, self.seq, &acc.changes)?;
                self.issued.entry(acc.author).or_default().note(acc.batch_id, false);
                self.next_client = self.next_client.max(acc.author.saturating_add(1));
                self.remember(acc);
                Ok(())
            }
            Journal::Refused { client, batch_id } => {
                if let Some(p) = self.issued.get_mut(&client).filter(|p| batch_id > p.last_batch) {
                    p.note(batch_id, true);
                }
                Ok(())
            }
        }
    }

    /// Beyond [`MAX_REMEMBERED`] client ids, forget the oldest one that isn't connected.
    fn forget_oldest(&mut self) {
        if self.issued.len() > MAX_REMEMBERED
            && let Some(old) = self.issued.keys().copied().find(|k| !self.peers.contains_key(k))
        {
            self.issued.remove(&old);
            self.secrets.remove(&old);
        }
    }

    /// The authoritative document.
    pub fn document(&self) -> &Document {
        self.state.document()
    }

    /// The authoritative state (document and positions).
    pub fn replica(&self) -> &Replica {
        &self.state
    }

    /// The sequence number of the last accepted batch (0: none yet).
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Who is in the session.
    pub fn peers(&self) -> impl Iterator<Item = &Peer> {
        self.peers.values()
    }

    /// The highest sequence number client `id` reported applying.
    pub fn acked(&self, id: u32) -> Option<u64> {
        self.acked.get(&id).copied()
    }

    /// A new connection says hello: on success its client id and the messages to send (its
    /// `Welcome`, everyone else's `PeerJoined`); otherwise the `Refused` to send back. A client
    /// coming back ([`Hello::resume`]) keeps its id (taking over from its old connection if that
    /// is still open: the transport closes the old one) and gets the batches it missed, or a
    /// snapshot when they are no longer in the log; an id this session never gave out joins as
    /// new.
    pub fn join(&mut self, hello: Hello) -> Result<(u32, Vec<Outgoing>), ServerMsg> {
        self.join_as(hello, Access::Edit)
    }

    /// [`join`](Self::join) with what the client may do: every batch of a viewer is refused, and
    /// every batch of a commenter that changes more than comment threads. A client that can't
    /// read comment links joins with one as a viewer.
    pub fn join_as(&mut self, hello: Hello, access: Access) -> Result<(u32, Vec<Outgoing>), ServerMsg> {
        let access = access.for_reader(reads_comments(&hello.client_version));
        let refuse = |reason: &str| ServerMsg::Refused { reason: reason.to_string() };
        if hello.protocol != PROTOCOL {
            return Err(refuse(&format!("protocol {} isn't supported (this server speaks {PROTOCOL})", hello.protocol)));
        }
        if !same_secret(&hello.token, &self.token) {
            return Err(refuse("wrong session token"));
        }
        let resume = hello.resume.filter(|r| r.session == self.session && self.issued.contains_key(&r.client_id) && self.may_resume(r));
        if self.peers.len() >= self.limits.max_peers && resume.is_none_or(|r| !self.peers.contains_key(&r.client_id)) {
            return Err(refuse("the session is full"));
        }
        if hello.name.chars().count() > self.limits.max_name || hello.color.chars().count() > self.limits.max_name {
            return Err(refuse("name or colour too long"));
        }
        if resume.is_none() && self.next_client == u32::MAX {
            return Err(refuse("no client ids left in this session"));
        }
        // The log holds every batch after `floor`.
        let floor = self.log.front().map_or(self.seq, |a| a.seq.saturating_sub(1));
        let catch_up = match resume {
            Some(r) if !self.resync && r.seq <= self.seq && r.seq >= floor => {
                CatchUp::Delta(self.log.iter().filter(|a| a.seq > r.seq).cloned().collect())
            }
            _ => CatchUp::Snapshot(self.state.snapshot().map_err(|e| refuse(&e.to_string()))?),
        };
        let (id, secret) = match resume {
            Some(r) => (r.client_id, *self.secrets.entry(r.client_id).or_insert(r.secret)),
            None => {
                let id = self.next_client;
                self.next_client += 1;
                self.issued.insert(id, Processed::default());
                let secret = new_session_id().max(1);
                self.secrets.insert(id, secret);
                self.forget_oldest();
                self.note(Journal::Issued { client: id });
                (id, secret)
            }
        };
        let processed = self.issued.get(&id).cloned().unwrap_or_default();
        self.presence.remove(&id);
        let signals = reads_signals(&hello.client_version);
        let peer = Peer { client_id: id, name: hello.name, color: hello.color };
        let others = self.peers.values().filter(|p| p.client_id != id).cloned().collect();
        let welcome = ServerMsg::Welcome { session: self.session, client_id: id, seq: self.seq, catch_up, processed, peers: others, access, secret };
        self.peers.insert(id, peer.clone());
        if access == Access::Edit {
            self.limited.remove(&id);
        } else {
            self.limited.insert(id, access);
        }
        let mut outs = vec![Outgoing::To(id, welcome), Outgoing::Others(id, ServerMsg::PeerJoined(peer))];
        if signals {
            self.signals.insert(id);
            outs.push(Outgoing::To(id, ServerMsg::Signal { client_id: id, signal: Signal::ready() }));
            if let Some(p) = self.spotlight.filter(|p| *p != id) {
                outs.push(Outgoing::To(id, ServerMsg::Signal { client_id: p, signal: Signal::spotlight(true) }));
            }
        } else {
            self.signals.remove(&id);
        }
        Ok((id, outs))
    }

    /// Whether `r` may take its client id back: it carries the secret that id was given with, or
    /// (after a restart, which forgets secrets) a secret for an id nobody is connected as.
    fn may_resume(&self, r: &Resume) -> bool {
        r.secret != 0
            && match self.secrets.get(&r.client_id) {
                Some(s) => *s == r.secret,
                None => !self.peers.contains_key(&r.client_id),
            }
    }

    /// Client `id`'s connection closed.
    pub fn leave(&mut self, id: u32) -> Vec<Outgoing> {
        self.presence.remove(&id);
        self.acked.remove(&id);
        self.limited.remove(&id);
        self.signals.remove(&id);
        if self.spotlight == Some(id) {
            self.spotlight = None;
        }
        match self.peers.remove(&id) {
            Some(_) => vec![Outgoing::Others(id, ServerMsg::PeerLeft { client_id: id })],
            None => Vec::new(),
        }
    }

    /// A message from client `from` (which has joined).
    pub fn receive(&mut self, from: u32, msg: ClientMsg) -> Vec<Outgoing> {
        if !self.peers.contains_key(&from) {
            return vec![Outgoing::To(from, ServerMsg::Refused { reason: "say hello first".into() })];
        }
        match msg {
            ClientMsg::Hello(_) => vec![Outgoing::To(from, ServerMsg::Refused { reason: "already joined".into() })],
            // A batch already processed (a copy sent again around a reconnect) is ignored: the
            // client learned what happened to it from the catch-up.
            ClientMsg::Changes { batch_id, .. } if self.issued.get(&from).is_some_and(|p| batch_id <= p.last_batch) => Vec::new(),
            ClientMsg::Changes { base_seq, batch_id, changes } => {
                let result = match self.limited.get(&from) {
                    Some(Access::View) => Err(Error::Invalid(VIEW_ONLY.into())),
                    Some(Access::Comment) if !changes.iter().all(Change::is_comment) => Err(Error::Invalid(COMMENT_ONLY.into())),
                    _ => self.accept(from, base_seq, &changes),
                };
                if let Some(p) = self.issued.get_mut(&from) {
                    p.note(batch_id, result.is_err());
                }
                match result {
                    Ok(()) => {
                        let acc = Accepted { seq: self.seq, author: from, batch_id, changes };
                        self.remember(acc.clone());
                        self.note(Journal::Accepted(acc.clone()));
                        vec![Outgoing::All(ServerMsg::Changes(acc))]
                    }
                    Err(e) => {
                        self.note(Journal::Refused { client: from, batch_id });
                        let fixes = self.fixes(&changes).unwrap_or_default();
                        vec![Outgoing::To(from, ServerMsg::Reject { batch_id, reason: reason(&e), fixes })]
                    }
                }
            }
            ClientMsg::Presence(p) => {
                self.presence.insert(from, p.clone());
                vec![Outgoing::Others(from, ServerMsg::Presence { client_id: from, presence: p })]
            }
            // Checked and cleaned, then passed on to those who read signals; never stored but the
            // current spotlight.
            ClientMsg::Signal(sig) => {
                let Some(sig) = sig.clean().filter(|s| s.kind != Signal::READY) else { return Vec::new() };
                if sig.kind == Signal::NAME {
                    return self.rename(from, sig.text);
                }
                if sig.kind == Signal::SPOTLIGHT {
                    if sig.on() {
                        self.spotlight = Some(from);
                    } else if self.spotlight == Some(from) {
                        self.spotlight = None;
                    }
                }
                let to = self.signals.iter().filter(|id| **id != from);
                to.map(|id| Outgoing::To(*id, ServerMsg::Signal { client_id: from, signal: sig.clone() })).collect()
            }
            ClientMsg::Ack { seq } => {
                self.acked.insert(from, seq.min(self.seq));
                Vec::new()
            }
            ClientMsg::Ping => vec![Outgoing::To(from, ServerMsg::Pong)],
            ClientMsg::Manage(_)
            | ClientMsg::Admin(_)
            | ClientMsg::OpenLibrary(_)
            | ClientMsg::Library(_)
            | ClientMsg::OpenPlugins(_)
            | ClientMsg::Plugins(_)
            | ClientMsg::OpenHosting(_)
            | ClientMsg::Resolve(_)
            | ClientMsg::Report { .. } => {
                vec![Outgoing::To(from, ServerMsg::Refused { reason: "managing files and libraries needs a connection of its own".into() })]
            }
        }
    }

    /// Client `from` is called `name` from now on (cleaned): everyone, itself too, hears it as its
    /// `PeerJoined`, and newcomers' welcomes list it so.
    fn rename(&mut self, from: u32, name: String) -> Vec<Outgoing> {
        let max = self.limits.max_name;
        let Some(peer) = self.peers.get_mut(&from).filter(|p| p.name != name && name.chars().count() <= max) else { return Vec::new() };
        peer.name = name;
        vec![Outgoing::All(ServerMsg::PeerJoined(peer.clone()))]
    }

    /// Keep an accepted batch in the log (dropping the oldest beyond the limits).
    fn remember(&mut self, acc: Accepted) {
        self.log_changes += acc.changes.len();
        self.log_bytes = self.log_bytes.saturating_add(weight(&acc.changes));
        self.log.push_back(acc);
        while !self.log.is_empty()
            && (self.log.len() > self.limits.max_log_batches
                || self.log_changes > self.limits.max_log_changes
                || self.log_bytes > self.limits.max_log_bytes)
        {
            if let Some(old) = self.log.pop_front() {
                self.log_changes = self.log_changes.saturating_sub(old.changes.len());
                self.log_bytes = self.log_bytes.saturating_sub(weight(&old.changes));
            }
        }
    }

    /// About how many bytes the batches kept for reconnecting clients hold.
    pub fn log_bytes(&self) -> usize {
        self.log_bytes
    }

    /// Validate and apply a batch; on success it has the next sequence number.
    fn accept(&mut self, from: u32, base_seq: u64, changes: &[Change]) -> Result<(), Error> {
        if changes.len() > self.limits.max_changes {
            return Err(Error::Invalid(format!("{} changes in one batch (at most {})", changes.len(), self.limits.max_changes)));
        }
        if base_seq > self.seq {
            return Err(Error::Protocol(format!("based on sequence {base_seq}, but the server is at {}", self.seq)));
        }
        for c in changes {
            if let Change::Create { id, .. } = c
                && !self.state.document().contains(*id)
                && !self.deleted.contains(id)
                && id.0 >> 32 != u64::from(from)
            {
                return Err(Error::Invalid(format!("new node {id} isn't in client {from}'s id space")));
            }
        }
        let mut next = self.state.clone();
        let applied = next.apply(changes, Mode::Strict)?;
        if next.document().node_count() > self.limits.max_nodes {
            return Err(Error::Invalid(format!("more than {} layers", self.limits.max_nodes)));
        }
        let entries = self.check_entries(from, changes, next.document())?;
        for c in changes {
            if let Change::Create { id, .. } = c {
                self.deleted.remove(id);
            }
        }
        self.deleted.extend(applied.deleted);
        for (entry, now) in entries {
            if now {
                self.removed.remove(&entry);
            } else {
                self.removed.insert(entry);
            }
        }
        self.state = next;
        self.seq += 1;
        Ok(())
    }

    /// The document-level entries (styles, variables, collections, comments) a batch adds or
    /// removes, each with whether it is there afterwards. A new one is refused when its id lies in
    /// the space of another client this session gave an id to, or in a space at or above
    /// [`NO_SESSION_SPACE`] (unless it was removed before: undo puts entries back); ids from
    /// before the session (space 0, earlier sessions, other files) are fine.
    fn check_entries(&self, from: u32, changes: &[Change], after: &Document) -> Result<Vec<(Entry, bool)>, Error> {
        let before = self.state.document();
        let mut touched: BTreeSet<Entry> = BTreeSet::new();
        for c in changes {
            match c {
                Change::Put { entry, .. } | Change::Remove { entry } if entry.id_space().is_some() => {
                    touched.insert(entry.clone());
                }
                // A whole list replaced (the fallback for lists whose ids aren't unique).
                Change::SetDoc { prop, .. } => {
                    if let Some(Ok(family)) = prop.list() {
                        touched.extend(docprops::entries(before, family));
                        touched.extend(docprops::entries(after, family));
                    }
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        for entry in touched {
            let (was, now) = (docprops::has(before, &entry), docprops::has(after, &entry));
            if was == now {
                continue;
            }
            if now
                && !self.removed.contains(&entry)
                && let Some(space) = entry.id_space().filter(|s| *s != from && (self.issued.contains_key(s) || *s >= NO_SESSION_SPACE))
            {
                return Err(Error::Invalid(format!("new {} isn't in client {from}'s id space (it is client {space}'s)", entry.describe())));
            }
            out.push((entry, now));
        }
        Ok(out)
    }

    /// The current values of what a refused batch touched, so a client can roll its optimistic
    /// copy back: nodes that don't exist are deleted, deleted subtrees come back whole, edited
    /// properties get their current values, document-level data likewise.
    fn fixes(&self, changes: &[Change]) -> Result<Vec<Change>, Error> {
        let doc = self.state.document();
        let mut props: BTreeMap<NodeId, BTreeSet<Prop>> = BTreeMap::new();
        let mut gone: BTreeSet<NodeId> = BTreeSet::new();
        let (mut asked, mut sent) = (BTreeSet::new(), BTreeSet::new());
        let mut out = Vec::new();
        for c in changes {
            match c {
                Change::Create { id, .. } => {
                    props.entry(*id).or_default().extend(Prop::FIELDS.iter().copied().chain([Prop::Parent, Prop::Position]));
                }
                Change::Set { id, prop, .. } => {
                    props.entry(*id).or_default().insert(*prop);
                }
                Change::Delete { id } => {
                    gone.insert(*id);
                }
                // The same target has the same fix: look each one up once, and send each fix once.
                other => {
                    if Target::of(other).is_some_and(|t| asked.insert(t))
                        && let Some(fix) = docprops::current(doc, other)?
                        && Target::of(&fix).is_some_and(|t| sent.insert(t))
                    {
                        out.push(fix);
                    }
                }
            }
        }
        let mut nodes = Vec::new();
        for (id, ps) in props {
            if !doc.contains(id) {
                nodes.push(Change::Delete { id });
                continue;
            }
            for p in ps {
                let t = Target::Node(id, p);
                nodes.extend(t.write(self.state.value(&t)?.as_ref()));
            }
        }
        // Each deleted node's subtree, pre-order, every node once: a restored node's whole subtree
        // went out with it, so the walk stops there (deleting a deep chain stays linear).
        let mut restored = BTreeSet::new();
        for id in gone {
            if !doc.contains(id) {
                nodes.push(Change::Delete { id });
                continue;
            }
            let mut stack = vec![id];
            while let Some(d) = stack.pop() {
                let Some(n) = doc.node(d) else { continue };
                if restored.insert(d) {
                    nodes.push(create_change(n, self.state.key(d))?);
                    stack.extend(n.children.iter().rev());
                }
            }
        }
        nodes.extend(out);
        Ok(nodes)
    }
}

/// A fresh session identity: random per server (std's hasher keys are seeded from the OS).
fn new_session_id() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u8(0);
    h.finish()
}

/// Why a batch was refused, for the person who made it.
fn reason(e: &Error) -> String {
    match e {
        Error::Unknown(_) => "a layer it changes was deleted by someone else".into(),
        e => e.to_string(),
    }
}

/// About how many bytes `changes` take in memory: their values' text and a little per change.
pub fn weight(changes: &[Change]) -> usize {
    fn json(v: &serde_json::Value) -> usize {
        match v {
            serde_json::Value::String(s) => s.len() + 2,
            serde_json::Value::Array(a) => a.iter().map(json).fold(2, usize::saturating_add),
            serde_json::Value::Object(o) => o.iter().map(|(k, v)| k.len() + 3 + json(v)).fold(2, usize::saturating_add),
            _ => 8,
        }
    }
    changes
        .iter()
        .map(|c| match c {
            Change::Create { kind, position, props, .. } => {
                props.iter().map(|(_, v)| 16 + json(&v.0)).fold(64 + json(&kind.0) + position.len(), usize::saturating_add)
            }
            Change::Set { value, .. } | Change::SetDoc { value, .. } => 32 + json(&value.0),
            Change::Put { value, .. } => 64 + json(&value.0),
            Change::Delete { .. } => 16,
            Change::Remove { .. } => 64,
        })
        .fold(0, usize::saturating_add)
}
