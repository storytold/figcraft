//! Crash and compaction simulation over many seeds, on the real data directory: several clients
//! edit (and undo) one file while the store logs, flushes and compacts it, connections drop, and
//! the server crashes (the [`Store`] and session dropped with nothing more written) before a
//! flush, in the middle of one (its records cut short on disk) or after one but before anyone
//! heard back. Everyone leaving compacts and parks the file. As the hub does, nobody hears of a
//! change before its flush.
//!
//! After every restart the file must come back as exactly the state the server had at some
//! sequence number, no earlier than the last durable point (a clean flush or a compaction) and
//! no earlier than anything a client heard; valid; and taking appends again. Clients resume with
//! their ids, resend what the server lost, and end with exactly its state.
//!
//! One seed: `FIGCRAFT_DST_SEED=7 cargo test -p figcraft-server dst_crash`.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use figcraft_collab::message::{Access, ClientMsg, ServerMsg};
use figcraft_collab::sim::Rng;
use figcraft_collab::{Client, NoRecompute, Notice, Outgoing, Replica, Server};
use figcraft_collab_net::hub::Backend;
use figcraft_doc::{Document, NodeId, NodeKind, Style, StyleKind};

use crate::disk::{self, LOG};
use crate::store::{FILES, Store, StoreConfig};
use crate::test_util::{TempDir, config};

const CLIENTS: usize = 3;

fn fixture() -> Document {
    let mut d = Document::new("Crash DST");
    let page = d.first_page().unwrap();
    for i in 0..3 {
        let f = d.create(page, NodeKind::Frame, &format!("F{i}"), i as f64 * 200.0, 0.0, 150.0, 100.0).unwrap();
        for j in 0..2 {
            d.create(f, NodeKind::Rectangle, &format!("R{i}{j}"), 0.0, j as f64 * 20.0, 10.0, 10.0).unwrap();
        }
    }
    d.repair();
    d.take_dirty();
    d
}

/// One random edit: layers created, moved, resized, renamed, deleted; pages and styles.
fn random_edit(doc: &mut Document, rng: &mut Rng) {
    let layers: Vec<NodeId> = doc.nodes().filter(|n| !n.is_page()).map(|n| n.id).collect();
    let containers: Vec<NodeId> = doc.nodes().filter(|n| n.kind.is_container()).map(|n| n.id).collect();
    match rng.below(9) {
        0 | 1 => {
            if let Some(&p) = rng.pick(&containers) {
                let _ = doc.create(p, NodeKind::Rectangle, "New", rng.below(300) as f64, rng.below(300) as f64, 8.0, 8.0);
            }
        }
        2 | 3 => {
            if let Some(n) = rng.pick(&layers).copied().and_then(|x| doc.node_mut(x)) {
                n.width = 1.0 + rng.below(400) as f64;
                n.opacity = rng.below(11) as f32 / 10.0;
            }
        }
        4 => {
            if let Some(n) = rng.pick(&layers).copied().and_then(|x| doc.node_mut(x)) {
                n.name = format!("Layer {}", rng.below(1000));
            }
        }
        5 => {
            if let Some(&x) = rng.pick(&layers).filter(|_| rng.chance(0.5)) {
                let _ = doc.remove(x);
            }
        }
        6 => {
            if let (Some(&x), Some(&p)) = (rng.pick(&layers), rng.pick(&containers)) {
                let _ = doc.place(x, p, 0);
            }
        }
        7 => {
            let pages = doc.pages.clone();
            match rng.pick(&pages).copied().and_then(|p| doc.node_mut(p)) {
                Some(n) if rng.chance(0.7) => n.name = format!("Page {}", rng.below(100)),
                _ => {
                    doc.add_page("More");
                }
            }
        }
        _ => {
            let id = doc.new_style_id();
            let name = format!("Style {}", rng.below(100));
            doc.styles.push(Style { id, name, description: String::new(), kind: StyleKind::Paint { paints: vec![] }, bound: Default::default() });
        }
    }
}

fn hello(i: usize) -> figcraft_collab::Hello {
    let ClientMsg::Hello(h) = Client::hello("f", "", &format!("Person {i}"), "#3366FF") else { panic!("not a hello") };
    h
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Crash {
    /// Before the pending records were written.
    BeforeFlush,
    /// While they were written: the log ends inside them.
    Torn,
    /// After a clean flush, before anyone heard back.
    BeforeAcks,
}

/// One file in a data directory, its live session and its clients.
struct Rig {
    seed: u64,
    rng: Rng,
    dir: TempDir,
    cfg: StoreConfig,
    store: Option<Store>,
    server: Server,
    file: String,
    clients: Vec<Client>,
    online: Vec<bool>,
    /// What the server sent, to deliver once it is flushed: (client index, message).
    outbox: Vec<(usize, ServerMsg)>,
    /// The server's state after each sequence number, on the timeline that survived.
    history: BTreeMap<u64, Replica>,
    /// The last state on disk for sure (a clean flush or a compaction).
    durable: (u64, Replica),
    compactions: u64,
    /// Compactions over every store opened.
    compacted: u64,
    crashes: Vec<Crash>,
    /// Torn crashes whose log ended inside a record (reported at the next open).
    damaged: usize,
    idles: usize,
}

impl Rig {
    fn new(seed: u64) -> Rig {
        let dir = TempDir::new(&format!("dst-crash-{seed}"));
        let cfg = StoreConfig { compact_after: 3 + (seed % 6) as usize, ..config(dir.path()).store() };
        let mut store = Store::open(dir.path(), cfg.clone()).unwrap();
        let (info, _, _) = store.publish("DST", &Replica::new(fixture()).snapshot().unwrap()).unwrap();
        let mut server = store.open(&info.id).unwrap();
        let start = server.replica().clone();
        let mut clients = Vec::new();
        let mut outs = Vec::new();
        for i in 0..CLIENTS {
            let (id, out) = server.join_as(hello(i), Access::Edit).unwrap();
            store.changed(&info.id, &mut server);
            assert!(store.flush().is_empty());
            let mut welcome = None;
            for o in out {
                match o {
                    Outgoing::To(to, m @ ServerMsg::Welcome { .. }) if to == id => welcome = Some(m),
                    o => outs.push(o),
                }
            }
            clients.push(Client::join(welcome.unwrap()).unwrap());
        }
        let compactions = store.compactions();
        let mut rig = Rig {
            seed,
            rng: Rng::new(seed ^ 0xD57),
            dir,
            cfg,
            store: Some(store),
            server,
            file: info.id,
            clients,
            online: vec![true; CLIENTS],
            outbox: Vec::new(),
            history: BTreeMap::from([(0, start.clone())]),
            durable: (0, start),
            compactions,
            compacted: 0,
            crashes: Vec::new(),
            damaged: 0,
            idles: 0,
        };
        rig.route(outs);
        rig.deliver();
        rig
    }

    fn store(&mut self) -> &mut Store {
        self.store.as_mut().unwrap()
    }

    fn log_path(&self) -> PathBuf {
        self.dir.path().join(FILES).join(&self.file).join(LOG)
    }

    fn log_len(&self) -> u64 {
        fs::metadata(self.log_path()).map_or(0, |m| m.len())
    }

    /// Queue what the server sent for the clients connected now.
    fn route(&mut self, out: Vec<Outgoing>) {
        for o in out {
            let ids: Vec<u32> = self.clients.iter().map(Client::id).collect();
            let (msg, targets): (ServerMsg, Vec<usize>) = match o {
                Outgoing::To(id, m) => (m, ids.iter().position(|c| *c == id).into_iter().collect()),
                Outgoing::All(m) => (m, (0..CLIENTS).collect()),
                Outgoing::Others(id, m) => (m, (0..CLIENTS).filter(|i| ids[*i] != id).collect()),
            };
            for t in targets.into_iter().filter(|t| self.online[*t]) {
                self.outbox.push((t, msg.clone()));
            }
        }
    }

    /// The store logs what the server just did (and may compact); the timeline grows.
    fn logged(&mut self, out: Vec<Outgoing>) {
        let file = self.file.clone();
        let store = self.store.as_mut().unwrap();
        store.changed(&file, &mut self.server);
        let compactions = store.compactions();
        let seq = self.server.seq();
        self.history.entry(seq).or_insert_with(|| self.server.replica().clone());
        if compactions != self.compactions {
            self.compacted += compactions - self.compactions;
            self.compactions = compactions;
            self.durable = (seq, self.server.replica().clone());
        }
        self.route(out);
    }

    fn send(&mut self, i: usize, msg: ClientMsg) {
        let id = self.clients[i].id();
        let out = self.server.receive(id, msg);
        self.logged(out);
    }

    /// A clean flush: what is logged is durable.
    fn flush(&mut self) {
        let file = self.file.clone();
        let failed = self.store().flush();
        assert!(!failed.contains(&file), "seed {}: the log couldn't be written", self.seed);
        self.durable = (self.server.seq(), self.server.replica().clone());
    }

    /// Flush, then let the clients hear everything, until nothing more is said.
    fn deliver(&mut self) {
        for _ in 0..10_000 {
            if self.outbox.is_empty() {
                return;
            }
            self.flush();
            for (i, msg) in std::mem::take(&mut self.outbox) {
                if !self.online[i] {
                    continue;
                }
                let replies = self.clients[i].receive(msg, &NoRecompute).unwrap();
                for r in replies {
                    self.send(i, r);
                }
            }
        }
        panic!("seed {}: the session never went quiet", self.seed);
    }

    fn edit(&mut self, i: usize) {
        let mut doc = self.clients[i].document().clone();
        let mut rng = Rng::new(self.rng.next_u64());
        random_edit(&mut doc, &mut rng);
        // Offline (or before the welcome) it is queued.
        if let Some(m) = self.clients[i].edit(&doc).unwrap() {
            self.send(i, m);
        }
    }

    fn undo(&mut self, i: usize) {
        let msgs = if self.rng.chance(0.6) { self.clients[i].undo(&NoRecompute) } else { self.clients[i].redo(&NoRecompute) };
        for m in msgs.unwrap() {
            self.send(i, m);
        }
    }

    /// The connection drops: what was on its way to the client is lost.
    fn disconnect(&mut self, i: usize) {
        if !self.online[i] {
            return;
        }
        self.online[i] = false;
        self.outbox.retain(|(t, _)| *t != i);
        self.clients[i].disconnect();
        let out = self.server.leave(self.clients[i].id());
        self.logged(out);
    }

    fn reconnect(&mut self, i: usize) {
        if self.online[i] {
            return;
        }
        let ClientMsg::Hello(h) = self.clients[i].reconnect(hello(i)) else { panic!("not a hello") };
        let (id, out) = self.server.join_as(h, Access::Edit).unwrap();
        assert_eq!(id, self.clients[i].id(), "seed {}: client {i} came back as someone else", self.seed);
        self.online[i] = true;
        self.logged(out);
    }

    /// The server dies; the clients' connections with it. Then it starts again from the disk.
    fn crash(&mut self, mut how: Crash) {
        let seed = self.seed;
        let mut torn_at = None;
        match how {
            Crash::BeforeFlush => {}
            Crash::BeforeAcks => self.flush(),
            Crash::Torn => {
                let before = self.log_len();
                let file = self.file.clone();
                assert!(!self.store().flush().contains(&file));
                let after = self.log_len();
                if after > before {
                    // Cut inside the new records: a cut on a record boundary is moved on to the
                    // next byte that isn't one.
                    let bytes = fs::read(self.log_path()).unwrap();
                    let mut at = before + self.rng.below((after - before) as usize) as u64;
                    while at + 1 < after && disk::read_log(&bytes[..at as usize]).unwrap().damaged.is_none() {
                        at += 1;
                    }
                    torn_at = Some(at);
                } else {
                    // Nothing was pending: that was a clean flush.
                    self.durable = (self.server.seq(), self.server.replica().clone());
                    how = Crash::BeforeAcks;
                }
            }
        }
        // Nothing more is written: no flush, no compaction, no goodbye.
        drop(self.store.take());
        self.outbox.clear();
        for i in 0..CLIENTS {
            self.online[i] = false;
            self.clients[i].disconnect();
        }
        let mut damaged = false;
        if let Some(at) = torn_at {
            let bytes = fs::read(self.log_path()).unwrap();
            let cut = &bytes[..at as usize];
            damaged = disk::read_log(cut).unwrap().damaged.is_some();
            fs::write(self.log_path(), cut).unwrap();
        }
        self.crashes.push(how);
        self.damaged += usize::from(damaged);

        let mut store = Store::open(self.dir.path(), self.cfg.clone()).unwrap();
        self.server = store.open(&self.file).unwrap();
        let seq = self.server.seq();
        let what = format!("seed {seed}: after a crash {how:?} (durable {}, reopened {seq})", self.durable.0);
        if damaged {
            assert!(store.problems().iter().any(|p| p.contains(&self.file)), "{what}: the cut-short log wasn't reported");
        }
        if how == Crash::Torn {
            assert!(seq >= self.durable.0, "{what}: lost flushed edits");
        } else {
            assert_eq!(seq, self.durable.0, "{what}");
            assert!(self.server.replica() == &self.durable.1, "{what}: not the state on disk");
        }
        let Some(expected) = self.history.get(&seq) else { panic!("{what}: a sequence number the server never had") };
        assert!(self.server.replica() == expected, "{what}: not the state the server had then");
        assert!(self.server.document().validate().is_empty(), "{what}: {:?}", self.server.document().validate());
        // Nobody heard of anything that didn't survive, and what they heard is what came back.
        for (i, c) in self.clients.iter().enumerate() {
            assert!(c.seq() <= seq, "{what}: client {i} heard of seq {}", c.seq());
            assert!(c.confirmed() == &self.history[&c.seq()], "{what}: client {i}'s confirmed state isn't the server's at {}", c.seq());
        }
        // What came after the surviving sequence number is gone: new batches take those numbers.
        self.history.retain(|s, _| *s <= seq);
        self.durable = (seq, self.server.replica().clone());
        self.compactions = store.compactions();
        self.store = Some(store);
        for i in 0..CLIENTS {
            if self.rng.chance(0.8) {
                self.reconnect(i);
            }
        }
    }

    /// Everyone leaves: the file is compacted and parked, and opened again on the next join.
    fn everyone_leaves(&mut self) {
        for i in 0..CLIENTS {
            self.disconnect(i);
        }
        self.deliver();
        self.flush();
        let before = self.server.replica().clone();
        let file = self.file.clone();
        let mut server = std::mem::replace(&mut self.server, Server::new(Document::new(""), "", self.cfg.core));
        let store = self.store.as_mut().unwrap();
        store.idle(&file, &mut server);
        self.server = store.open(&file).unwrap();
        self.compacted += store.compactions() - self.compactions;
        self.compactions = store.compactions();
        assert!(self.server.replica() == &before, "seed {}: the file came back changed after everyone left", self.seed);
        self.durable = (self.server.seq(), before);
        self.idles += 1;
        for i in 0..CLIENTS {
            self.reconnect(i);
        }
    }

    /// Everyone back and up to date, then edits until the server holds batches not yet flushed.
    fn pending_edits(&mut self) {
        for i in 0..CLIENTS {
            self.reconnect(i);
        }
        self.deliver();
        let start = self.server.seq();
        for _ in 0..1_000 {
            if self.server.seq() > start {
                return;
            }
            let i = self.rng.below(CLIENTS);
            self.edit(i);
        }
        panic!("seed {}: no edit was accepted", self.seed);
    }

    fn step(&mut self) {
        let i = self.rng.below(CLIENTS);
        match self.rng.below(24) {
            0..=8 => self.edit(i),
            9 | 10 => {
                if self.online[i] {
                    self.undo(i);
                }
            }
            11..=13 => self.deliver(),
            14 | 15 => self.disconnect(i),
            16 | 17 => self.reconnect(i),
            18 => {
                let how = [Crash::BeforeFlush, Crash::Torn, Crash::BeforeAcks][self.rng.below(3)];
                self.crash(how);
            }
            19 => self.everyone_leaves(),
            _ => self.edit(i),
        }
    }

    /// Everyone back, everything said: each client holds exactly the server's state.
    fn settle(&mut self, when: &str) {
        for i in 0..CLIENTS {
            self.reconnect(i);
        }
        self.deliver();
        let seed = self.seed;
        let server = self.server.replica();
        for (i, c) in self.clients.iter_mut().enumerate() {
            assert_eq!((c.pending(), c.seq()), (0, self.server.seq()), "seed {seed} {when}: client {i}");
            assert!(c.view() == server && c.confirmed() == server, "seed {seed} {when}: client {i} differs from the server");
            let lost: Vec<Notice> = c.take_notices().into_iter().filter(|n| matches!(n, Notice::Lost { .. })).collect();
            assert!(lost.is_empty(), "seed {seed} {when}: client {i} lost its queue: {lost:?}");
        }
        assert!(self.server.document().validate().is_empty(), "seed {seed} {when}");
    }
}

fn run_seed(seed: u64) -> Rig {
    let mut rig = Rig::new(seed);
    for round in 0..3 {
        for _ in 0..25 {
            rig.step();
        }
        // Every round tears a flush of batches nobody heard of yet.
        rig.pending_edits();
        rig.crash(Crash::Torn);
        rig.settle(&format!("round {round}"));
    }
    // Everyone leaves (compacted, parked), then a crash: the compacted file is read from disk.
    rig.everyone_leaves();
    rig.crash(Crash::BeforeFlush);
    rig.settle("after a crash once everyone left");
    // Accepted and flushed, nobody heard: the clients' resends are recognised as done.
    rig.pending_edits();
    rig.crash(Crash::BeforeAcks);
    rig.settle("after a crash before the acks");
    // The last word: a crash right after everyone agreed loses nothing.
    rig.crash(Crash::BeforeFlush);
    rig.settle("after the last crash");
    assert!(rig.damaged > 0, "seed {seed}: no torn log ended inside a record");
    assert!(rig.crashes.contains(&Crash::Torn), "seed {seed}: no flush was torn");
    rig
}

#[test]
fn dst_crashes_compactions_and_torn_logs_lose_nothing_acknowledged() {
    let seeds: Vec<u64> = match std::env::var("FIGCRAFT_DST_SEED").ok().and_then(|s| s.parse().ok()) {
        Some(one) => vec![one],
        None => (0..24).collect(),
    };
    // Seeds are independent (a data directory each) and mostly wait on the disk: run them side
    // by side. Each stays deterministic.
    let rigs: Vec<Rig> = std::thread::scope(|s| {
        let runs: Vec<_> = seeds.into_iter().map(|seed| s.spawn(move || run_seed(seed))).collect();
        runs.into_iter().map(|r| r.join().unwrap()).collect()
    });
    let (mut kinds, mut damaged, mut idles, mut compactions) = ([0usize; 3], 0, 0, 0);
    for rig in &rigs {
        for c in &rig.crashes {
            kinds[*c as usize] += 1;
        }
        damaged += rig.damaged;
        idles += rig.idles;
        compactions += rig.compacted;
    }
    assert!(kinds.iter().all(|k| *k > 0), "every kind of crash happens: {kinds:?}");
    assert!(damaged > 0, "some torn log ends inside a record");
    assert!(idles > 0 && compactions > 0, "files are compacted ({compactions}) and parked ({idles})");
}
