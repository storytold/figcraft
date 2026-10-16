//! The client side: the confirmed server state, this client's unconfirmed batches on top of it,
//! the optimistic view the user edits, the user's own undo history, and the queue of batches made
//! while the connection is down.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use figcraft_doc::{Document, NodeId};

use crate::change::{Change, Prop};
use crate::history::{BACK, BatchKind, FWD, Notice, Step, Touches};
use crate::message::{Accepted, Access, CatchUp, ClientMsg, Hello, PROTOCOL, Peer, Presence, Processed, Resume, ServerMsg};
use crate::replica::{Mode, Replica};
use crate::{Diff, Error, NoRecompute, Recompute, diff};

/// The most undo steps a client keeps by default.
pub const HISTORY_LIMIT: usize = 1000;

/// A batch not yet confirmed or rejected: sent, or waiting to be sent.
#[derive(Clone, Debug)]
struct Pending {
    batch_id: u64,
    /// The server state it was made on (kept when it is sent again after a reconnect).
    base_seq: u64,
    changes: Vec<Change>,
    kind: BatchKind,
    /// The undo step this batch applies (its edit, undo or redo).
    step: Option<u64>,
    /// Someone else's batch was ordered before this one: its derived data may be stale.
    rebased: bool,
    /// Sent on the current connection. Batches go out in order; one waits (with every batch
    /// after it) while the client is offline, and an undo or redo waits until every earlier batch
    /// is confirmed or refused: it is then made again from the confirmed state (what it shows
    /// meanwhile is a preview), so it never rests on an edit the server refused or on stale
    /// values.
    sent: bool,
}

/// One participant's sync state.
#[derive(Clone, Debug)]
pub struct Client {
    /// The session joined (see [`ServerMsg::Welcome`]).
    session: u64,
    id: u32,
    /// The server's state as of `seq`.
    confirmed: Replica,
    seq: u64,
    pending: VecDeque<Pending>,
    /// `confirmed` with the pending batches applied: what the user sees and edits.
    view: Replica,
    next_batch: u64,
    /// Ids below this were handed out; rebuilding the view never hands them out again.
    next_id: NodeId,
    peers: BTreeMap<u32, Peer>,
    presence: BTreeMap<u32, Presence>,
    /// Connected (batches go out); otherwise they wait for [`reconnect`](Self::reconnect).
    online: bool,
    undo: Vec<Step>,
    redo: Vec<Step>,
    next_step: u64,
    history_limit: usize,
    notices: Vec<Notice>,
    /// What the server lets this client do (a viewer's edits are refused).
    access: Access,
}

/// The step with id `id` in either stack.
fn find_step<'a>(undo: &'a mut [Step], redo: &'a mut [Step], id: u64) -> Option<&'a mut Step> {
    undo.iter_mut().chain(redo.iter_mut()).find(|s| s.id == id)
}

impl Client {
    /// The first message of a connection.
    pub fn hello(file: &str, token: &str, name: &str, color: &str) -> ClientMsg {
        ClientMsg::Hello(Hello {
            protocol: PROTOCOL,
            file: file.to_string(),
            token: token.to_string(),
            name: name.to_string(),
            color: color.to_string(),
            client_version: crate::message::client_version(),
            resume: None,
        })
    }

    /// Start from the server's `Welcome` (which must carry a snapshot).
    pub fn join(welcome: ServerMsg) -> Result<Client, Error> {
        let ServerMsg::Welcome { session, client_id, seq, catch_up: CatchUp::Snapshot(snapshot), processed, peers, access } = welcome else {
            return Err(Error::Protocol("expected a welcome with the document".into()));
        };
        let mut confirmed = Replica::from_snapshot(&snapshot)?;
        confirmed.document_mut().set_client_id(client_id);
        let next_id = confirmed.document().next_id();
        Ok(Client {
            session,
            id: client_id,
            view: confirmed.clone(),
            confirmed,
            seq,
            pending: VecDeque::new(),
            // After the batches the server already processed for this id (a client that starts
            // over under the id it had, e.g. after its app crashed: lower ids would be ignored).
            next_batch: processed.last_batch.saturating_add(1),
            next_id,
            peers: peers.into_iter().map(|p| (p.client_id, p)).collect(),
            presence: BTreeMap::new(),
            online: true,
            undo: Vec::new(),
            redo: Vec::new(),
            next_step: 1,
            history_limit: HISTORY_LIMIT,
            notices: Vec::new(),
            access,
        })
    }

    /// This client's id (its node ids are `id << 32 | counter`).
    pub fn id(&self) -> u32 {
        self.id
    }

    /// What the server lets this client do (from the latest welcome).
    pub fn access(&self) -> Access {
        self.access
    }

    /// The document as this user sees it (server state plus their unconfirmed edits). Edit a copy
    /// (it allocates ids in this client's space) and hand it to [`edit`](Self::edit).
    pub fn document(&self) -> &Document {
        self.view.document()
    }

    /// The optimistic state (document and positions).
    pub fn view(&self) -> &Replica {
        &self.view
    }

    /// The last confirmed server state.
    pub fn confirmed(&self) -> &Replica {
        &self.confirmed
    }

    /// The sequence number of the last server batch applied.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Batches not yet confirmed or rejected (sent or waiting).
    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    /// What this client's unconfirmed batch `batch_id` is for.
    pub fn batch_kind(&self, batch_id: u64) -> Option<BatchKind> {
        self.pending.iter().find(|p| p.batch_id == batch_id).map(|p| p.kind)
    }

    /// Connected: batches go out as they are made.
    pub fn is_online(&self) -> bool {
        self.online
    }

    /// The other people in the session.
    pub fn peers(&self) -> impl Iterator<Item = &Peer> {
        self.peers.values()
    }

    /// The last presence each peer sent.
    pub fn presence_of(&self, id: u32) -> Option<&Presence> {
        self.presence.get(&id)
    }

    /// What happened that the user should hear about since the last call (rejected edits…).
    pub fn take_notices(&mut self) -> Vec<Notice> {
        std::mem::take(&mut self.notices)
    }

    /// The most undo steps kept (at least one).
    pub fn set_history_limit(&mut self, limit: usize) {
        self.history_limit = limit.max(1);
        self.trim_history();
    }

    /// Undo steps, oldest first, by label.
    pub fn undo_labels(&self) -> impl DoubleEndedIterator<Item = &str> {
        self.undo.iter().map(|s| s.label.as_str())
    }

    /// Redo steps, the next one last, by label.
    pub fn redo_labels(&self) -> impl DoubleEndedIterator<Item = &str> {
        self.redo.iter().map(|s| s.label.as_str())
    }

    /// The user edited the document (`doc` is [`document`](Self::document) after a command): show
    /// it at once and return the batch to send, if anything synced changed and it can go now
    /// (otherwise it is queued: see [`Pending::sent`]). The edit is one undo step.
    pub fn edit(&mut self, doc: &Document) -> Result<Option<ClientMsg>, Error> {
        self.edit_with(doc, Some(""))
    }

    /// [`edit`](Self::edit) with the undo step's label (`None`: not undoable, e.g. a change no
    /// command made; the redo steps stay).
    pub fn edit_with(&mut self, doc: &Document, undo_label: Option<&str>) -> Result<Option<ClientMsg>, Error> {
        let d = diff(&self.view, doc)?;
        if d.changes.is_empty() {
            return Ok(None);
        }
        let step = match undo_label {
            Some(label) => {
                let id = self.next_step;
                self.next_step += 1;
                Some(Step::new(id, label, self.next_batch, &self.view, &d.replica, &d.changes)?)
            }
            None => None,
        };
        self.view = d.replica;
        self.keep_ids();
        let step_id = step.as_ref().map(|s| s.id);
        if let Some(s) = step {
            self.undo.push(s);
            self.redo.clear();
            self.trim_history();
        }
        self.queue(d.changes, BatchKind::Edit, step_id);
        // Everything queued before this batch is waiting (or it would have gone already), so at
        // most this edit goes now, and no undo becomes ready to go.
        Ok(self.flush(&NoRecompute).pop())
    }

    /// Undo the user's last step that still changes something: write back the values it replaced
    /// where nobody else wrote since (their later writes win), delete what it created (unless
    /// others put layers inside) and bring back what it deleted. Returns the batches to send now
    /// (the undo, then any derived data `recompute` brings up to date as a follow-up); none when
    /// there is nothing to undo, or when they have to wait (offline, or the step's own edit is
    /// still unconfirmed).
    pub fn undo(&mut self, recompute: &dyn Recompute) -> Result<Vec<ClientMsg>, Error> {
        self.travel(BACK, recompute)
    }

    /// Redo the last undone step (the inverse of [`undo`](Self::undo)).
    pub fn redo(&mut self, recompute: &dyn Recompute) -> Result<Vec<ClientMsg>, Error> {
        self.travel(FWD, recompute)
    }

    /// How many undo and redo steps there are.
    pub fn history_len(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }

    /// Whether [`undo`](Self::undo) has a step to try.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether [`redo`](Self::redo) has a step to try.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    fn travel(&mut self, side: usize, recompute: &dyn Recompute) -> Result<Vec<ClientMsg>, Error> {
        loop {
            let popped = if side == BACK { self.undo.pop() } else { self.redo.pop() };
            let Some(mut step) = popped else { return Ok(Vec::new()) };
            let changes = match step.toward(side, &self.view) {
                Ok(c) => c,
                Err(e) => {
                    // Put it back: nothing was done.
                    if side == BACK {
                        self.undo.push(step)
                    } else {
                        self.redo.push(step)
                    }
                    return Err(e);
                }
            };
            if changes.is_empty() {
                // Everything it changed was changed again by others (or is gone): skip it.
                continue;
            }
            let before = self.view.clone();
            let applied = self.view.apply(&changes, Mode::Lenient)?;
            self.keep_ids();
            step.batch = self.next_batch;
            let kind = if side == BACK { BatchKind::Undo } else { BatchKind::Redo };
            self.queue(changes, kind, Some(step.id));
            // The derived data that follows, as its own batch (possibly empty: it is made again
            // with the undo when that goes out).
            let follow = self.recomputed(&self.view, before.document(), &applied.touched, recompute)?;
            if !follow.changes.is_empty() {
                self.view = follow.replica;
                self.keep_ids();
            }
            self.queue(follow.changes, BatchKind::FollowUp, Some(step.id));
            if side == BACK {
                self.redo.push(step)
            } else {
                self.undo.push(step)
            }
            return Ok(self.flush(recompute));
        }
    }

    fn trim_history(&mut self) {
        let over = self.undo.len().saturating_sub(self.history_limit);
        self.undo.drain(..over);
    }

    /// Queue `changes` as a new batch (already shown in the view).
    fn queue(&mut self, changes: Vec<Change>, kind: BatchKind, step: Option<u64>) {
        let batch_id = self.next_batch;
        // `next_batch` starts from the welcome's `last_batch`, the host's word: saturate.
        self.next_batch = self.next_batch.saturating_add(1);
        self.pending.push_back(Pending { batch_id, base_seq: self.seq, changes, kind, step, rebased: false, sent: false });
    }

    /// The waiting batches that can go now, in order (see [`Pending::sent`]).
    fn flush(&mut self, recompute: &dyn Recompute) -> Vec<ClientMsg> {
        let mut out = Vec::new();
        if !self.online {
            return out;
        }
        let mut i = 0;
        while let Some(p) = self.pending.get(i) {
            if p.sent {
                i += 1;
                continue;
            }
            if matches!(p.kind, BatchKind::Undo | BatchKind::Redo) {
                if i > 0 {
                    break;
                }
                if !self.remake_front(recompute) {
                    // Nothing of it is left to do: it's gone; look at what came after it.
                    continue;
                }
            }
            let Some(p) = self.pending.get_mut(i) else { break };
            if p.changes.is_empty() {
                // A follow-up with nothing to bring up to date.
                self.pending.remove(i);
                continue;
            }
            p.sent = true;
            out.push(ClientMsg::Changes { base_seq: p.base_seq, batch_id: p.batch_id, changes: p.changes.clone() });
            i += 1;
        }
        out
    }

    /// The undo or redo at the front of the queue goes out now: make it again from the confirmed
    /// state (others' writes since it was previewed are left alone, nodes that no longer fit are
    /// skipped), with its follow-up. False when nothing of it is left (it and its step are
    /// dropped).
    fn remake_front(&mut self, recompute: &dyn Recompute) -> bool {
        let Some((batch, step_id, side)) = self.pending.front().map(|p| (p.batch_id, p.step, if p.kind == BatchKind::Undo { BACK } else { FWD }))
        else {
            return true;
        };
        let Some(step) = step_id.and_then(|id| find_step(&mut self.undo, &mut self.redo, id)).filter(|s| s.batch == batch) else {
            // Its step left the history (the size limit): send it as previewed.
            return true;
        };
        let Ok(changes) = step.toward(side, &self.confirmed) else { return true };
        if changes.is_empty() {
            let id = step.id;
            self.pending.pop_front();
            self.undo.retain(|s| s.id != id);
            self.redo.retain(|s| s.id != id);
            self.pending.retain(|q| q.step != Some(id) || q.sent);
            self.rebuild();
            return false;
        }
        step.send(&changes);
        let mut after = self.confirmed.clone();
        let follow = after
            .apply(&changes, Mode::Lenient)
            .and_then(|a| self.recomputed(&after, self.confirmed.document(), &a.touched, recompute))
            .map(|d| d.changes)
            .unwrap_or_default();
        let seq = self.seq;
        if let Some(p) = self.pending.front_mut() {
            p.changes = changes;
            p.base_seq = seq;
        }
        if let Some(f) = self.pending.get_mut(1).filter(|q| q.kind == BatchKind::FollowUp && q.step == step_id && !q.sent) {
            f.changes = follow;
            f.base_seq = seq;
        }
        self.rebuild();
        true
    }

    /// Keep allocating in this client's space, after every id already handed out.
    fn keep_ids(&mut self) {
        let doc = self.view.document_mut();
        if doc.client_id() != self.id {
            doc.set_client_id(self.id);
        }
        if doc.next_id() < self.next_id {
            doc.reserve_ids(NodeId(self.next_id.0.saturating_sub(1)));
        }
        self.next_id = doc.next_id();
    }

    /// The connection dropped: keep editing; batches wait for [`reconnect`](Self::reconnect).
    pub fn disconnect(&mut self) {
        self.online = false;
    }

    /// The first message of a new connection after [`disconnect`](Self::disconnect): `hello`
    /// (as for the first join) with this client's session, id and last sequence number. The
    /// server's `Welcome` then goes to [`receive`](Self::receive), which returns the batches to
    /// send.
    pub fn reconnect(&mut self, hello: Hello) -> ClientMsg {
        self.online = false;
        ClientMsg::Hello(Hello { resume: Some(self.resume()), ..hello })
    }

    /// What a reconnect resumes with: this client's session, id and last sequence number.
    pub fn resume(&self) -> Resume {
        Resume { session: self.session, client_id: self.id, seq: self.seq }
    }

    /// Handle a message from the server. Returns messages to send: batches that were waiting for
    /// this one, a follow-up batch when this client's confirmed batch was ordered after someone
    /// else's and `recompute` changes derived data over it, or after a reconnect's `Welcome`, the
    /// queued batches. Errors mean the connection is unusable (rejoin).
    pub fn receive(&mut self, msg: ServerMsg, recompute: &dyn Recompute) -> Result<Vec<ClientMsg>, Error> {
        match msg {
            ServerMsg::Changes(acc) => {
                self.accepted(acc, recompute)?;
                Ok(self.flush(recompute))
            }
            ServerMsg::Reject { batch_id, reason, fixes } => {
                let mine = self.pop_sent(batch_id, "rejection")?;
                // The confirmed copy already is the server's state (every batch ordered before
                // this rejection arrived first); the fixes change nothing there unless it drifted.
                let _ = self.confirmed.apply(&fixes, Mode::Lenient);
                self.refused(&mine, reason);
                self.rebuild();
                Ok(self.flush(recompute))
            }
            ServerMsg::Presence { client_id, presence } => {
                self.presence.insert(client_id, presence);
                Ok(Vec::new())
            }
            ServerMsg::PeerJoined(p) => {
                self.peers.insert(p.client_id, p);
                Ok(Vec::new())
            }
            ServerMsg::PeerLeft { client_id } => {
                self.peers.remove(&client_id);
                self.presence.remove(&client_id);
                Ok(Vec::new())
            }
            // Cursor chat, spotlights and what a server says about its hosting are the app's (the
            // transport hands them over).
            ServerMsg::Pong | ServerMsg::Signal { .. } | ServerMsg::Hosting(_) | ServerMsg::Reported { .. } | ServerMsg::Resolved { .. } => {
                Ok(Vec::new())
            }
            ServerMsg::Welcome { .. } if self.online => Err(Error::Protocol("already joined".into())),
            ServerMsg::Welcome { session, client_id, seq, catch_up, processed, peers, access } => {
                if session != self.session || client_id != self.id {
                    return self.restart(ServerMsg::Welcome { session, client_id, seq, catch_up, processed, peers, access });
                }
                self.access = access;
                self.rejoin(seq, catch_up, &processed, peers, recompute)
            }
            ServerMsg::Refused { reason } => Err(Error::Protocol(reason)),
            ServerMsg::Admin(_) | ServerMsg::Library(_) | ServerMsg::Plugins(_) => {
                Err(Error::Protocol("an answer meant for a managing, library or plugin directory connection".into()))
            }
        }
    }

    /// The oldest pending batch, which the server just confirmed or rejected as `batch_id`.
    fn pop_sent(&mut self, batch_id: u64, what: &str) -> Result<Pending, Error> {
        match self.pending.front() {
            Some(p) if p.sent && p.batch_id == batch_id => {
                self.pending.pop_front().ok_or_else(|| Error::Protocol(format!("{what} of an unknown batch")))
            }
            Some(p) => Err(Error::Protocol(format!("{what} of batch {batch_id}, expected {}", p.batch_id))),
            None => Err(Error::Protocol(format!("{what} of an unknown batch"))),
        }
    }

    /// A batch the server accepted (in order). Follow-ups it causes are queued.
    fn accepted(&mut self, acc: Accepted, recompute: &dyn Recompute) -> Result<(), Error> {
        // The host's sequence number is untrusted: at `u64::MAX` it can't step past it.
        let Some(next) = self.seq.checked_add(1) else {
            return Err(Error::Protocol(format!("sequence {} after {}", acc.seq, self.seq)));
        };
        if acc.seq != next {
            return Err(Error::Protocol(format!("sequence {} after {}", acc.seq, self.seq)));
        }
        if acc.author != self.id {
            self.confirmed.apply(&acc.changes, Mode::Strict)?;
            self.seq = acc.seq;
            // Ordered after the steps the server confirmed already: those leave its targets alone.
            let touches = Touches::of(&acc.changes);
            for s in self.undo.iter_mut().chain(self.redo.iter_mut()) {
                s.others_wrote(&touches);
            }
            for p in &mut self.pending {
                p.rebased = true;
            }
            self.rebuild();
            return Ok(());
        }
        let mine = self.pop_sent(acc.batch_id, "confirmation")?;
        let before = self.confirmed.clone();
        let applied = self.confirmed.apply(&acc.changes, Mode::Strict)?;
        self.seq = acc.seq;
        if mine.kind != BatchKind::FollowUp
            && let Some(s) = mine.step.and_then(|id| find_step(&mut self.undo, &mut self.redo, id))
        {
            // Later batches of the step haven't gone out yet (an undo waits for what came before).
            s.settle(mine.kind);
            s.refresh(&before, &self.confirmed, &acc.changes, mine.kind)?;
        }
        if mine.rebased {
            self.repair(&before, &applied.touched, recompute)?;
        }
        Ok(())
    }

    /// Batch `p` was refused: its step leaves the undo history, with the batches of that step
    /// waiting behind it (an undo of the refused edit), and the user hears why.
    fn refused(&mut self, p: &Pending, reason: String) {
        if p.kind == BatchKind::FollowUp {
            return;
        }
        if let Some(id) = p.step {
            self.undo.retain(|s| s.id != id);
            self.redo.retain(|s| s.id != id);
            // Those never went out: an undo waits for its edit.
            self.pending.retain(|q| q.step != Some(id) || q.batch_id < p.batch_id || q.sent);
        }
        self.notices.push(Notice::Rejected { batch_id: p.batch_id, kind: p.kind, reason });
    }

    /// Back after a dropped connection: catch up (missed batches, or a snapshot when the server
    /// no longer has them), settle the batches the server processed meanwhile, and return the
    /// ones to send.
    fn rejoin(
        &mut self,
        seq: u64,
        catch_up: CatchUp,
        processed: &Processed,
        peers: Vec<Peer>,
        recompute: &dyn Recompute,
    ) -> Result<Vec<ClientMsg>, Error> {
        let last_batch = processed.last_batch;
        // An undo or redo lost in flight is made again before it goes out, knowing what others did
        // meanwhile: its step conflicts with what they wrote while it was in flight, and with what
        // comes now, as if it had never been sent.
        for p in self.pending.iter().filter(|p| p.sent && p.batch_id > last_batch && matches!(p.kind, BatchKind::Undo | BatchKind::Redo)) {
            if let Some(s) = p.step.and_then(|id| find_step(&mut self.undo, &mut self.redo, id)) {
                s.lost();
            }
        }
        self.peers = peers.into_iter().map(|p| (p.client_id, p)).collect();
        self.presence.clear();
        let offline = || "it no longer applied when the connection came back".to_string();
        match catch_up {
            CatchUp::Delta(batches) => {
                for acc in batches {
                    if acc.author == self.id {
                        // Batches the server processed before this one and didn't log were refused.
                        while let Some(p) = self.pending.front().filter(|p| p.batch_id < acc.batch_id).cloned() {
                            self.pending.pop_front();
                            self.refused(&p, offline());
                        }
                    }
                    self.accepted(acc, recompute)?;
                }
            }
            CatchUp::Snapshot(snapshot) => {
                let mut confirmed = Replica::from_snapshot(&snapshot)?;
                confirmed.document_mut().set_client_id(self.id);
                self.confirmed = confirmed;
                self.seq = seq;
                // What the server accepted meanwhile is in the snapshot. Which of the batches since
                // were others' isn't known, so the steps check every value they wrote below.
                while let Some(p) = self.pending.front().filter(|p| p.batch_id <= last_batch).cloned() {
                    self.pending.pop_front();
                    if processed.refused.contains(&p.batch_id) {
                        self.refused(&p, offline());
                    } else if p.kind != BatchKind::FollowUp
                        && let Some(s) = p.step.and_then(|id| find_step(&mut self.undo, &mut self.redo, id))
                    {
                        s.settle(p.kind);
                    }
                }
                for s in self.undo.iter_mut().chain(self.redo.iter_mut()).filter(|s| s.confirmed) {
                    s.verify(&self.confirmed)?;
                }
                for p in &mut self.pending {
                    p.rebased = true;
                }
            }
        }
        if self.seq != seq {
            return Err(Error::Protocol(format!("caught up to sequence {}, the server is at {seq}", self.seq)));
        }
        // Processed by the server but neither logged nor confirmed: refused.
        while let Some(p) = self.pending.front().filter(|p| p.batch_id <= last_batch).cloned() {
            self.pending.pop_front();
            self.refused(&p, offline());
        }
        // The server never saw the rest: they go again, in order.
        for p in &mut self.pending {
            p.sent = false;
        }
        self.online = true;
        self.rebuild();
        Ok(self.flush(recompute))
    }

    /// A session that doesn't know this client (it restarted without its saved state) welcomed
    /// it as new: start over from its document; unconfirmed work and the undo history are gone.
    fn restart(&mut self, welcome: ServerMsg) -> Result<Vec<ClientMsg>, Error> {
        let lost = self.pending.iter().filter(|p| p.kind != BatchKind::FollowUp).count();
        let notices = std::mem::take(&mut self.notices);
        let limit = self.history_limit;
        *self = Client::join(welcome)?;
        self.notices = notices;
        self.history_limit = limit;
        if lost > 0 {
            self.notices.push(Notice::Lost { batches: lost });
        }
        Ok(Vec::new())
    }

    /// The view is the confirmed state with the pending batches replayed on top (changes that no
    /// longer apply are skipped; the server will reject their batch). A step whose batch is
    /// replayed learns the values that batch now replaces (not from an undo or redo that waits
    /// to go out: that is made again from the step when it goes).
    fn rebuild(&mut self) {
        let mut v = self.confirmed.clone();
        for p in &self.pending {
            let learns = p.kind == BatchKind::Edit || (p.sent && p.kind != BatchKind::FollowUp);
            let step = p.step.filter(|_| learns).and_then(|id| find_step(&mut self.undo, &mut self.redo, id));
            let pre = step.as_ref().map(|_| v.clone());
            // Lenient application never fails.
            let _ = v.apply(&p.changes, Mode::Lenient);
            if let (Some(s), Some(pre)) = (step, pre) {
                let _ = s.refresh(&pre, &v, &p.changes, p.kind);
            }
        }
        self.view = v;
        self.keep_ids();
    }

    /// `base`'s document with derived data recomputed over `touched` (`before`: the state before
    /// the edit that touched them), as a diff from `base`.
    fn recomputed(&self, base: &Replica, before: &Document, touched: &BTreeSet<NodeId>, recompute: &dyn Recompute) -> Result<Diff, Error> {
        let mut work = base.document().clone();
        // Ids the recompute makes must not collide with those of pending batches.
        work.reserve_ids(NodeId(self.next_id.0.saturating_sub(1)));
        recompute.recompute(&mut work, before, touched);
        diff(base, &work)
    }

    /// This client's batch landed after someone else's: recompute derived data over what it
    /// touched and queue what changes (minus properties a later pending batch writes anyway).
    fn repair(&mut self, before: &Replica, touched: &BTreeSet<NodeId>, recompute: &dyn Recompute) -> Result<(), Error> {
        let fix = self.recomputed(&self.confirmed, before.document(), touched, recompute)?;
        let later: BTreeSet<(NodeId, Prop)> = self
            .pending
            .iter()
            .flat_map(|p| &p.changes)
            .filter_map(|c| match c {
                Change::Set { id, prop, .. } => Some((*id, *prop)),
                _ => None,
            })
            .collect();
        let changes: Vec<Change> =
            fix.changes.into_iter().filter(|c| !matches!(c, Change::Set { id, prop, .. } if later.contains(&(*id, *prop)))).collect();
        if changes.is_empty() {
            return Ok(());
        }
        let _ = self.view.apply(&changes, Mode::Lenient);
        self.keep_ids();
        self.queue(changes, BatchKind::FollowUp, None);
        Ok(())
    }
}
