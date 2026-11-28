//! An in-memory session for tests: one [`Server`], several [`Client`]s and a link each way per
//! client that delivers in order (like a WebSocket) after a random delay, so messages on different
//! links interleave differently for every seed. Every message goes through the binary or the JSON
//! encoding on its way. Clients can drop their connection and come back, and every undo or redo
//! the server accepts is checked against what others wrote before it ([`Sim::violations`]).

use std::collections::VecDeque;

use figcraft_doc::Document;

use crate::change::{Change, Target};
use crate::history::Touches;
use crate::message::{self, Accepted, ClientMsg, Hello, ServerMsg};
use crate::{BatchKind, Client, Error, Limits, Outgoing, Recompute, Replica, Server};

/// A small deterministic random number generator (SplitMix64) for simulations and fuzzing.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `0..n` (0 when `n` is 0).
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next_u64() % n as u64) as usize }
    }

    /// True with probability `p`.
    pub fn chance(&mut self, p: f64) -> bool {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64 <= p
    }

    /// A random element of `items`.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        items.get(self.below(items.len()))
    }
}

/// A message on the wire.
#[derive(Clone, Debug)]
enum Wire {
    Binary(Vec<u8>),
    Json(String),
}

/// One direction of one connection: in order, each message due at a time.
#[derive(Clone, Debug, Default)]
struct Link {
    queue: VecDeque<(u64, Wire)>,
    last_due: u64,
}

/// One accepted batch, as the undo checker remembers it.
struct Logged {
    author: u32,
    touches: Touches,
}

/// A simulated session.
pub struct Sim {
    pub server: Server,
    clients: Vec<Client>,
    /// Each client's id at the server and its first hello (to reconnect with).
    ids: Vec<u32>,
    hellos: Vec<Hello>,
    online: Vec<bool>,
    up: Vec<Link>,
    down: Vec<Link>,
    rng: Rng,
    now: u64,
    max_delay: u64,
    limits: Limits,
    /// The server's state after each sequence number (index: the sequence number).
    states: Vec<Replica>,
    /// What each accepted batch wrote (index: sequence number − 1).
    log: Vec<Logged>,
    /// Per client: the batches it only got as part of a snapshot (`(after, upto]`).
    gaps: Vec<Vec<(u64, u64)>>,
    /// Messages delivered so far.
    pub delivered: usize,
    /// Batches the server rejected.
    pub rejected: usize,
    /// Follow-up batches clients sent to repair derived data.
    pub repairs: usize,
    /// Undo and redo batches the server accepted.
    pub undos: usize,
    /// Reconnects that caught up with a snapshot (the missed batches were no longer logged).
    pub snapshots: usize,
    /// Undo or redo batches that reverted a write by someone else the client had already seen.
    pub violations: Vec<String>,
}

impl Sim {
    /// A session for `doc` with `n` clients joined; message delays are random in `1..=max_delay`.
    pub fn new(doc: Document, n: usize, seed: u64, max_delay: u64) -> Result<Sim, Error> {
        Sim::with_limits(doc, n, seed, max_delay, Limits::default())
    }

    /// [`new`](Self::new) with the server's limits (e.g. a short log of batches).
    pub fn with_limits(doc: Document, n: usize, seed: u64, max_delay: u64, limits: Limits) -> Result<Sim, Error> {
        let server = Server::new(doc, "secret", limits);
        let mut sim = Sim {
            states: vec![server.replica().clone()],
            server,
            clients: Vec::new(),
            ids: Vec::new(),
            hellos: Vec::new(),
            online: Vec::new(),
            up: Vec::new(),
            down: Vec::new(),
            rng: Rng::new(seed),
            now: 0,
            max_delay: max_delay.max(1),
            limits,
            log: Vec::new(),
            gaps: Vec::new(),
            delivered: 0,
            rejected: 0,
            repairs: 0,
            undos: 0,
            snapshots: 0,
            violations: Vec::new(),
        };
        for i in 0..n {
            let ClientMsg::Hello(hello) = Client::hello("file", "secret", &format!("Person {}", i + 1), "#3366FF") else {
                return Err(Error::Protocol("hello".into()));
            };
            let (id, out) = sim.server.join(hello.clone()).map_err(|m| Error::Protocol(format!("{m:?}")))?;
            let mut welcome = None;
            let mut rest = Vec::new();
            for o in out {
                match o {
                    Outgoing::To(to, m @ ServerMsg::Welcome { .. }) if to == id => welcome = Some(m),
                    o => rest.push(o),
                }
            }
            let welcome = welcome.ok_or_else(|| Error::Protocol("no welcome".into()))?;
            // The welcome travels encoded like everything else.
            let welcome: ServerMsg = message::decode(&message::encode(&welcome)?, limits.max_message_bytes)?;
            sim.clients.push(Client::join(welcome)?);
            sim.ids.push(id);
            sim.hellos.push(hello);
            sim.online.push(true);
            sim.gaps.push(Vec::new());
            sim.up.push(Link::default());
            sim.down.push(Link::default());
            sim.route(rest)?;
        }
        Ok(sim)
    }

    /// The clients.
    pub fn clients(&self) -> &[Client] {
        &self.clients
    }

    /// The clients, to read their notices or change settings.
    pub fn clients_mut(&mut self) -> &mut [Client] {
        &mut self.clients
    }

    /// Client `i`'s document (to edit a copy of).
    pub fn document(&self, i: usize) -> Option<&Document> {
        self.clients.get(i).map(Client::document)
    }

    /// The random number generator (drivers use it to pick edits).
    pub fn rng(&mut self) -> &mut Rng {
        &mut self.rng
    }

    /// Whether client `i` is connected.
    pub fn is_online(&self, i: usize) -> bool {
        self.online.get(i).copied().unwrap_or(false)
    }

    fn client(&mut self, i: usize) -> Result<&mut Client, Error> {
        self.clients.get_mut(i).ok_or_else(|| Error::Invalid(format!("no client {i}")))
    }

    fn wire<T: serde::Serialize>(&mut self, msg: &T) -> Result<Wire, Error> {
        // Mostly binary; now and then the JSON form, so both encodings are exercised.
        if self.rng.chance(0.15) { message::encode_json(msg).map(Wire::Json) } else { message::encode(msg).map(Wire::Binary) }
    }

    fn unwire<T: serde::de::DeserializeOwned>(&self, w: &Wire) -> Result<T, Error> {
        match w {
            Wire::Binary(b) => message::decode(b, self.limits.max_message_bytes),
            Wire::Json(t) => message::decode_json(t, self.limits.max_message_bytes),
        }
    }

    fn send(&mut self, up: bool, i: usize, w: Wire) {
        let delay = 1 + self.rng.next_u64() % self.max_delay;
        let now = self.now;
        let link = if up { self.up.get_mut(i) } else { self.down.get_mut(i) };
        if let Some(link) = link {
            let due = (now + delay).max(link.last_due);
            link.last_due = due;
            link.queue.push_back((due, w));
        }
    }

    /// Send client `i`'s messages to the server.
    fn send_up(&mut self, i: usize, msgs: Vec<ClientMsg>) -> Result<(), Error> {
        for m in msgs {
            let w = self.wire(&m)?;
            self.send(true, i, w);
        }
        Ok(())
    }

    fn route(&mut self, out: Vec<Outgoing>) -> Result<(), Error> {
        for o in out {
            let (msg, targets): (ServerMsg, Vec<usize>) = match o {
                Outgoing::To(id, m) => (m, self.ids.iter().position(|c| *c == id).into_iter().collect()),
                Outgoing::All(m) => (m, (0..self.clients.len()).collect()),
                Outgoing::Others(id, m) => (m, (0..self.clients.len()).filter(|i| self.ids.get(*i) != Some(&id)).collect()),
            };
            let targets: Vec<usize> = targets.into_iter().filter(|t| self.is_online(*t)).collect();
            for t in targets {
                let w = self.wire(&msg)?;
                self.send(false, t, w);
            }
        }
        Ok(())
    }

    /// Client `i` edited its document into `doc`: it applies it at once and sends the batch
    /// (offline: queues it). Returns whether a batch went out.
    pub fn edit(&mut self, i: usize, doc: &Document) -> Result<bool, Error> {
        let msg = self.client(i)?.edit(doc)?;
        let sent = msg.is_some();
        self.send_up(i, msg.into_iter().collect())?;
        Ok(sent)
    }

    /// Client `i` undoes its last step. Returns whether batches went out.
    pub fn undo(&mut self, i: usize, recompute: &dyn Recompute) -> Result<bool, Error> {
        let msgs = self.client(i)?.undo(recompute)?;
        let sent = !msgs.is_empty();
        self.send_up(i, msgs)?;
        Ok(sent)
    }

    /// Client `i` redoes its last undone step. Returns whether batches went out.
    pub fn redo(&mut self, i: usize, recompute: &dyn Recompute) -> Result<bool, Error> {
        let msgs = self.client(i)?.redo(recompute)?;
        let sent = !msgs.is_empty();
        self.send_up(i, msgs)?;
        Ok(sent)
    }

    /// Client `i`'s connection drops: whatever is in flight either way is lost.
    pub fn disconnect(&mut self, i: usize) -> Result<(), Error> {
        if !self.is_online(i) {
            return Ok(());
        }
        for link in [self.up.get_mut(i), self.down.get_mut(i)].into_iter().flatten() {
            link.queue.clear();
        }
        if let Some(o) = self.online.get_mut(i) {
            *o = false;
        }
        self.client(i)?.disconnect();
        let id = self.ids.get(i).copied().unwrap_or(0);
        let out = self.server.leave(id);
        self.route(out)
    }

    /// Client `i` connects again: it says hello with its id and last sequence number; the
    /// server's welcome reaches it over the new link.
    pub fn reconnect(&mut self, i: usize) -> Result<(), Error> {
        if self.is_online(i) {
            return Ok(());
        }
        let hello = self.hellos.get(i).cloned().ok_or_else(|| Error::Invalid(format!("no client {i}")))?;
        let ClientMsg::Hello(hello) = self.client(i)?.reconnect(hello) else { return Err(Error::Protocol("hello".into())) };
        // The hello travels encoded too.
        let hello: ClientMsg = message::decode(&message::encode(&ClientMsg::Hello(hello))?, self.limits.max_message_bytes)?;
        let ClientMsg::Hello(hello) = hello else { return Err(Error::Protocol("hello".into())) };
        let seen = self.clients.get(i).map_or(0, Client::seq);
        let (id, out) = self.server.join(hello).map_err(|m| Error::Protocol(format!("{m:?}")))?;
        if out.iter().any(|o| {
            matches!(o, Outgoing::To(to, ServerMsg::Welcome { catch_up: crate::CatchUp::Snapshot(_) | crate::CatchUp::DeltaSnapshot(_), .. }) if *to == id)
        }) {
            self.snapshots += 1;
            if let Some(g) = self.gaps.get_mut(i) {
                g.push((seen, self.server.seq()));
            }
        }
        if let Some(c) = self.ids.get_mut(i) {
            *c = id;
        }
        if let Some(o) = self.online.get_mut(i) {
            *o = true;
        }
        self.route(out)
    }

    /// Deliver the next due message (the earliest; ties go to the lowest link). False when none.
    pub fn step(&mut self, recompute: &dyn Recompute) -> Result<bool, Error> {
        let next = self
            .up
            .iter()
            .enumerate()
            .filter_map(|(i, l)| l.queue.front().map(|(due, _)| (*due, 0u8, i)))
            .chain(self.down.iter().enumerate().filter_map(|(i, l)| l.queue.front().map(|(due, _)| (*due, 1u8, i))))
            .min();
        let Some((due, dir, i)) = next else { return Ok(false) };
        self.now = self.now.max(due);
        self.delivered += 1;
        if dir == 0 {
            let Some((_, w)) = self.up.get_mut(i).and_then(|l| l.queue.pop_front()) else { return Ok(false) };
            let msg: ClientMsg = self.unwire(&w)?;
            let travel = match &msg {
                ClientMsg::Changes { base_seq, batch_id, .. } => self
                    .clients
                    .get(i)
                    .and_then(|c| c.batch_kind(*batch_id))
                    .filter(|k| matches!(k, BatchKind::Undo | BatchKind::Redo))
                    .map(|_| *base_seq),
                _ => None,
            };
            let id = self.ids.get(i).copied().unwrap_or(0);
            let out = self.server.receive(id, msg);
            for o in &out {
                if let Outgoing::All(ServerMsg::Changes(acc)) = o {
                    self.record(acc, travel);
                }
            }
            self.route(out)?;
        } else {
            let Some((_, w)) = self.down.get_mut(i).and_then(|l| l.queue.pop_front()) else { return Ok(false) };
            let msg: ServerMsg = self.unwire(&w)?;
            self.rejected += usize::from(matches!(msg, ServerMsg::Reject { .. }));
            let replies = match self.clients.get_mut(i) {
                Some(c) => c.receive(msg, recompute)?,
                None => Vec::new(),
            };
            self.repairs += replies.len();
            self.send_up(i, replies)?;
        }
        Ok(true)
    }

    /// The server accepted `acc`; `travel`: it is an undo or redo made on top of that sequence
    /// number, to check first.
    fn record(&mut self, acc: &Accepted, travel: Option<u64>) {
        if let Some(base) = travel {
            self.undos += 1;
            self.check(acc, base);
        }
        self.log.push(Logged { author: acc.author, touches: Touches::of(&acc.changes) });
        self.states.push(self.server.replica().clone());
    }

    /// The sequence number and author of the last batch for which `hit` holds.
    fn last_writer(&self, hit: impl Fn(&Touches) -> bool) -> Option<(u64, u32)> {
        self.log.iter().enumerate().rev().find(|(_, l)| hit(&l.touches)).map(|(i, l)| (i as u64 + 1, l.author))
    }

    /// An undo or redo by `acc.author`, made when the client had seen the server up to `base`,
    /// may change a value (or delete a node) only if its author wrote (created) it last, or if
    /// the last writer's batch came after `base` (the client couldn't know: concurrent edits are
    /// ordered last-writer-wins like any other). Batches the client only saw inside a snapshot
    /// are exempt: it compares values then, and can't tell an equal value someone else wrote.
    fn check(&mut self, acc: &Accepted, base: u64) {
        let Some(state) = self.states.last() else { return };
        let gaps = self.ids.iter().position(|id| *id == acc.author).and_then(|i| self.gaps.get(i)).cloned().unwrap_or_default();
        let allowed = |w: Option<(u64, u32)>| {
            w.is_some_and(|(seq, author)| author == acc.author || seq > base || gaps.iter().any(|(a, b)| *a < seq && seq <= *b))
        };
        let mut found = Vec::new();
        // Deletions are checked where they happen in the batch (moves before them take layers out).
        let mut work = state.clone();
        for c in &acc.changes {
            match c {
                Change::Delete { id } => {
                    let doc = work.document();
                    for d in doc.subtree(*id) {
                        // Fine if the author created it, or it (or the branch holding it inside the
                        // deleted node) was created or moved in after `base`.
                        let mut path = vec![d];
                        let mut cur = d;
                        while let Some(p) = doc.node(cur).and_then(|x| x.parent).filter(|p| cur != *id && *p != *id) {
                            path.push(p);
                            cur = p;
                        }
                        let ok = path.iter().any(|n| {
                            let moved = self.last_writer(|t| t.hits(&Target::Node(*n, crate::Prop::Parent)));
                            allowed(self.last_writer(|t| t.creates(*n))) || moved.is_some_and(|(seq, _)| seq > base)
                        });
                        if !ok {
                            found.push(format!("seq {}: client {} deleted {d} (created by someone else, before the undo)", acc.seq, acc.author));
                        }
                    }
                }
                Change::Create { .. } => {}
                other => {
                    let written = match other {
                        Change::Set { value, .. } | Change::SetDoc { value, .. } | Change::Put { value, .. } => Some(value),
                        _ => None,
                    };
                    if let Some(t) = Target::of(other)
                        && state.value(&t).ok().flatten().as_ref() != written
                    {
                        let writer = self.last_writer(|w| w.hits(&t));
                        if !allowed(writer) {
                            found.push(format!("seq {}: client {} wrote {t:?}, written last by {writer:?}", acc.seq, acc.author));
                        }
                    }
                }
            }
            let _ = work.apply(std::slice::from_ref(c), crate::Mode::Lenient);
        }
        self.violations.extend(found);
    }

    /// Deliver until no message is in flight (at most `max_steps`). Returns the steps taken.
    pub fn settle(&mut self, recompute: &dyn Recompute, max_steps: usize) -> Result<usize, Error> {
        let mut n = 0;
        while n < max_steps && self.step(recompute)? {
            n += 1;
        }
        if self.up.iter().chain(&self.down).any(|l| !l.queue.is_empty()) {
            return Err(Error::Protocol(format!("still busy after {max_steps} steps")));
        }
        Ok(n)
    }

    /// Whether every client is connected, caught up with the server and holds exactly its state;
    /// the first difference otherwise.
    pub fn converged(&self) -> Result<(), String> {
        let server: &Replica = self.server.replica();
        for (i, c) in self.clients.iter().enumerate() {
            if !self.is_online(i) {
                return Err(format!("client {} is offline", c.id()));
            }
            if c.pending() > 0 || c.seq() != self.server.seq() {
                return Err(format!("client {} at seq {} with {} pending; server at {}", c.id(), c.seq(), c.pending(), self.server.seq()));
            }
            if c.view() != server || c.confirmed() != server {
                return Err(format!("client {}: {}", c.id(), describe(server, c.view())));
            }
        }
        Ok(())
    }
}

/// The first difference between two replicas, for test failures.
pub fn describe(a: &Replica, b: &Replica) -> String {
    let (da, db) = (a.document(), b.document());
    if let Some(&id) = db.changed_ids(da, usize::MAX).unwrap_or_default().first() {
        return match (da.node(id), db.node(id)) {
            (Some(x), Some(y)) => format!("node {id} differs:\n  {x:?}\n  {y:?}"),
            (x, y) => format!("node {id}: {} vs {}", x.is_some(), y.is_some()),
        };
    }
    if da.pages != db.pages {
        return format!("pages {:?} vs {:?}", da.pages, db.pages);
    }
    if a.positions() != b.positions() {
        let k = a.positions().iter().find(|(k, v)| b.positions().get(*k) != Some(*v)).map(|(k, _)| *k);
        return format!("positions differ (first at {k:?})");
    }
    if da != db {
        return "document-level data differs".into();
    }
    "equal".into()
}
