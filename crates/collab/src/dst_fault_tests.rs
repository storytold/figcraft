//! Fault-injection simulation over many seeds: asymmetric partitions (some clients cut off while
//! the rest push the server past its log), connections dropped with batches in flight, offline
//! edits queued and sent on reconnect, one-way link stalls, and per-user undo and redo mixed with
//! edits to layers, styles, variables, comments, component instances and pages. Every seed must
//! catch up all three ways (missed batches from the log, a compressed snapshot, a plain snapshot
//! for an older app) and end converged, valid, with sequence numbers that never go back, deleted
//! ids that aren't live, and no undo that reverted someone else's write.
//!
//! One seed: `FIGCRAFT_DST_SEED=7 cargo test -p figcraft-collab dst_fault`.

use std::collections::BTreeMap;

use figcraft_color::{Color, Paint};
use figcraft_doc::{Collection, Comment, Document, Message, Mode, NodeId, NodeKind, TextData, Variable, VariableType, VariableValue};
use figcraft_geom::Affine;

use crate::delta_snapshot_tests::LEGACY_CLIENT_VERSION;
use crate::server::MAX_DELETED;
use crate::sim::{Rng, Sim};
use crate::sim_tests::{base, random_edit};
use crate::{Limits, NoRecompute};

const CLIENTS: usize = 4;
/// Reconnects with an older app that reads only plain snapshots.
const LEGACY: usize = 2;

/// The shared file: frames of rectangles, a component with two instances on another page, a
/// variable collection with two modes, and a comment.
fn fixture() -> Document {
    let mut d = base();
    let first = d.first_page().unwrap();
    let parts = d.add_page("Components");
    let comp = d
        .create(
            parts,
            NodeKind::Component { description: "Button".into(), props: BTreeMap::new(), prop_order: Vec::new() },
            "Button",
            0.0,
            0.0,
            120.0,
            40.0,
        )
        .unwrap();
    d.create(comp, NodeKind::Rectangle, "Fill", 0.0, 0.0, 120.0, 40.0).unwrap();
    d.create(comp, NodeKind::Text(TextData::new("Label".to_string()).into()), "Label", 8.0, 8.0, 100.0, 24.0).unwrap();
    for i in 0..2 {
        instantiate(&mut d, comp, first, i as f64 * 150.0).unwrap();
    }
    let cid = d.new_collection_id();
    d.variables.collections.push(Collection {
        id: cid.clone(),
        name: "Theme".into(),
        modes: vec![Mode { id: format!("{cid}/light"), name: "Light".into() }, Mode { id: format!("{cid}/dark"), name: "Dark".into() }],
        extends: None,
    });
    add_variable(&mut d, 4);
    let k = d.new_comment_id();
    d.comments.push(comment(k, first, "Check the spacing"));
    d.take_dirty();
    d
}

/// An instance of `comp` under `parent`: a root following the component and one mirror per child.
fn instantiate(doc: &mut Document, comp: NodeId, parent: NodeId, x: f64) -> Option<NodeId> {
    let main = doc.node(comp)?.clone();
    let inst = doc.alloc_id();
    let mut root = main.clone();
    root.id = inst;
    root.name = format!("{} instance", main.name);
    root.kind = NodeKind::Instance { component: comp, props: BTreeMap::new() };
    root.children.clear();
    root.parent = None;
    root.source = Some(comp);
    root.transform = Affine::translate((x, 300.0));
    doc.insert(root, parent, None).ok()?;
    for c in main.children {
        let mut mirror = doc.node(c)?.clone();
        mirror.id = doc.alloc_id();
        mirror.parent = None;
        mirror.children.clear();
        mirror.source = Some(c);
        mirror.overrides.clear();
        doc.insert(mirror, inst, None).ok()?;
    }
    Some(inst)
}

fn comment(id: u64, page: NodeId, text: &str) -> Comment {
    Comment { id, page, x: 10.0, y: 20.0, anchor: None, first: Message::new("DST", text, 0), resolved: false, replies: Vec::new(), read_by: None }
}

/// A number variable in the first collection, `value` in every mode.
fn add_variable(doc: &mut Document, value: u32) {
    let Some(c) = doc.variables.collections.first() else { return };
    let (cid, values) = (c.id.clone(), c.modes.iter().map(|m| (m.id.clone(), VariableValue::Number(f64::from(value)))).collect());
    let id = doc.new_variable_id();
    doc.variables.variables.push(Variable {
        id,
        name: format!("space/{value}"),
        collection: cid,
        kind: VariableType::Number,
        values,
        description: String::new(),
        scopes: None,
        hidden_from_publishing: false,
        extends: None,
        overrides: Default::default(),
        code_syntax: Default::default(),
    });
}

fn of_kind(doc: &Document, f: impl Fn(&NodeKind) -> bool) -> Vec<NodeId> {
    doc.nodes().filter(|n| f(&n.kind)).map(|n| n.id).collect()
}

/// One random edit: layers, styles and pages ([`random_edit`]) half the time, else variables,
/// comments, instances (overrides, new instances), main components or page names.
fn fault_edit(doc: &mut Document, rng: &mut Rng) {
    match rng.below(12) {
        0..=5 => random_edit(doc, rng, &NoRecompute),
        6 => match rng.below(4) {
            0 => add_variable(doc, rng.below(100) as u32),
            1 => {
                let n = doc.variables.variables.len();
                if let Some(v) = doc.variables.variables.get_mut(rng.below(n)) {
                    let mode = v.values.keys().nth(rng.below(v.values.len())).cloned();
                    if let Some(m) = mode {
                        v.values.insert(m, VariableValue::Number(rng.below(64) as f64));
                    }
                }
            }
            2 => {
                let n = doc.variables.variables.len();
                if let Some(v) = doc.variables.variables.get_mut(rng.below(n)) {
                    v.name = format!("renamed/{}", rng.below(100));
                }
            }
            _ => {
                if doc.variables.variables.len() > 1 {
                    let i = rng.below(doc.variables.variables.len());
                    doc.variables.variables.remove(i);
                }
            }
        },
        7 => {
            let n = doc.comments.len();
            match rng.below(4) {
                0 => {
                    let page = doc.pages[rng.below(doc.pages.len())];
                    let k = doc.new_comment_id();
                    doc.comments.push(comment(k, page, &format!("Note {}", rng.below(100))));
                }
                1 => {
                    if let Some(c) = doc.comments.get_mut(rng.below(n)) {
                        c.resolved = !c.resolved;
                    }
                }
                2 => {
                    if let Some(c) = doc.comments.get_mut(rng.below(n)) {
                        c.replies.push(Message::new("DST", format!("Reply {}", rng.below(100)), 0));
                    }
                }
                _ => {
                    if n > 0 {
                        doc.comments.remove(rng.below(n));
                    }
                }
            }
        }
        8 => {
            // An override on an instance's mirror, or the instance moved or renamed.
            let mirrors: Vec<NodeId> =
                doc.nodes().filter(|n| n.source.is_some() && !matches!(n.kind, NodeKind::Instance { .. })).map(|n| n.id).collect();
            let instances = of_kind(doc, |k| matches!(k, NodeKind::Instance { .. }));
            if rng.chance(0.5)
                && let Some(n) = rng.pick(&mirrors).copied().and_then(|m| doc.node_mut(m))
            {
                n.fills = vec![Paint::solid(Color::from_rgb8_const(0, rng.below(256) as u8, 200))];
                n.overrides.insert("fills".to_string());
            } else if let Some(n) = rng.pick(&instances).copied().and_then(|m| doc.node_mut(m)) {
                n.transform = Affine::translate((rng.below(600) as f64, 300.0));
                n.name = format!("Button {}", rng.below(50));
            }
        }
        9 => {
            let comps = of_kind(doc, |k| matches!(k, NodeKind::Component { .. }));
            if let (Some(&comp), Some(&page)) = (rng.pick(&comps), doc.pages.first()) {
                let x = rng.below(800) as f64;
                let _ = instantiate(doc, comp, page, x);
            }
        }
        10 => {
            // The main component changes: what instances follow.
            let comps = of_kind(doc, |k| matches!(k, NodeKind::Component { .. }));
            let parts: Vec<NodeId> = comps.iter().filter_map(|c| doc.node(*c)).flat_map(|n| n.children.clone()).collect();
            if let Some(n) = rng.pick(&parts).copied().and_then(|p| doc.node_mut(p)) {
                n.fills = vec![Paint::solid(Color::from_rgb8_const(rng.below(256) as u8, 120, 0))];
                n.opacity = rng.below(11) as f32 / 10.0;
            }
        }
        _ => {
            let pages = doc.pages.clone();
            if let Some(n) = rng.pick(&pages).copied().and_then(|p| doc.node_mut(p)) {
                n.name = format!("Page {}", rng.below(100));
            } else {
                doc.add_page("Extra");
            }
        }
    }
}

/// Sequence numbers seen so far: the server's and each client's never go back.
struct Watch {
    seed: u64,
    server: u64,
    clients: Vec<u64>,
}

impl Watch {
    fn check(&mut self, sim: &Sim) {
        let seed = self.seed;
        assert!(sim.server.seq() >= self.server, "seed {seed}: the server went back from {} to {}", self.server, sim.server.seq());
        self.server = sim.server.seq();
        for (i, c) in sim.clients().iter().enumerate() {
            assert!(c.seq() >= self.clients[i], "seed {seed}: client {i} went back from {} to {}", self.clients[i], c.seq());
            assert!(c.seq() <= self.server, "seed {seed}: client {i} at {} is ahead of the server at {}", c.seq(), self.server);
            self.clients[i] = c.seq();
        }
    }
}

/// Deliver up to `n` messages, checking sequence numbers after each.
fn pump(sim: &mut Sim, w: &mut Watch, n: usize) {
    for _ in 0..n {
        if !sim.step(&NoRecompute).unwrap() {
            break;
        }
        w.check(sim);
    }
}

/// Deliver everything in flight.
fn quiesce(sim: &mut Sim, w: &mut Watch) {
    pump(sim, w, 1_000_000);
    sim.settle(&NoRecompute, 0).unwrap();
}

fn edit(sim: &mut Sim, i: usize) -> bool {
    let mut doc = sim.document(i).unwrap().clone();
    let mut rng = Rng::new(sim.rng().next_u64());
    fault_edit(&mut doc, &mut rng);
    sim.edit(i, &doc).unwrap()
}

/// An edit that always makes a batch (queued, offline): the first page renamed.
fn sure_edit(sim: &mut Sim, i: usize, tag: &str) {
    let mut doc = sim.document(i).unwrap().clone();
    let page = doc.pages[0];
    doc.node_mut(page).unwrap().name = format!("{tag} by {i}");
    let queued = sim.clients()[i].pending();
    sim.edit(i, &doc).unwrap();
    assert!(sim.clients()[i].pending() > queued, "{tag}: no batch");
}

/// Client `i` edits, undoes, redoes, drops or regains its connection, or its link stalls one way.
fn act(sim: &mut Sim, i: usize, faults: bool) {
    match sim.rng().below(12) {
        0 | 1 => {
            sim.undo(i, &NoRecompute).unwrap();
        }
        2 => {
            sim.redo(i, &NoRecompute).unwrap();
        }
        3 if faults => {
            if sim.is_online(i) {
                sim.disconnect(i).unwrap();
            } else {
                sim.reconnect(i).unwrap();
            }
        }
        4 if faults => {
            let (up, ticks) = (sim.rng().chance(0.5), 1 + sim.rng().below(40) as u64);
            sim.stall(i, up, ticks);
        }
        _ => {
            edit(sim, i);
        }
    }
}

/// Online clients in `actors` edit until the server is `by` batches further (refused and empty
/// edits don't count), delivering everything after each edit when `settle`.
fn advance(sim: &mut Sim, w: &mut Watch, actors: &[usize], by: u64, settle: bool) {
    let target = sim.server.seq() + by;
    let mut tries = 0;
    while sim.server.seq() < target {
        tries += 1;
        assert!(tries < 2_000, "seed {}: the server stuck at {} (target {target})", w.seed, sim.server.seq());
        let i = actors[sim.rng().below(actors.len())];
        edit(sim, i);
        if settle {
            quiesce(sim, w);
        } else {
            let n = sim.rng().below(4);
            pump(sim, w, n);
        }
    }
}

/// Drop client `i`'s connection with its last edit possibly still in flight, then edit (and maybe
/// undo) offline: the queue must hold work for the reconnect.
fn cut_off(sim: &mut Sim, w: &mut Watch, i: usize, tag: &str) {
    edit(sim, i);
    let n = sim.rng().below(3);
    pump(sim, w, n);
    sim.disconnect(i).unwrap();
    sure_edit(sim, i, tag);
    for _ in 0..sim.rng().below(4) {
        edit(sim, i);
    }
    if sim.rng().chance(0.5) {
        sim.undo(i, &NoRecompute).unwrap();
        if sim.rng().chance(0.5) {
            sim.redo(i, &NoRecompute).unwrap();
        }
    }
}

/// What must hold whenever nothing is in flight and everyone is back.
fn check_quiet(sim: &Sim, seed: u64, when: &str) {
    if let Err(e) = sim.converged() {
        panic!("seed {seed} {when}: {e}");
    }
    let doc = sim.server.document();
    assert!(doc.validate().is_empty(), "seed {seed} {when}: {:?}", doc.validate());
    for (i, c) in sim.clients().iter().enumerate() {
        assert!(c.document().validate().is_empty(), "seed {seed} {when}: client {i}: {:?}", c.document().validate());
    }
    assert!(sim.violations.is_empty(), "seed {seed} {when}: {:?}", sim.violations);
    let deleted = sim.server.state().unwrap().deleted;
    assert!(deleted.len() <= MAX_DELETED, "seed {seed} {when}: {} deleted ids remembered", deleted.len());
    let live: Vec<_> = deleted.iter().filter(|id| doc.contains(**id)).collect();
    assert!(live.is_empty(), "seed {seed} {when}: ids remembered as deleted are in the document: {live:?}");
}

struct Outcome {
    undos: usize,
    log_catch_ups: usize,
    delta_snapshots: usize,
    plain_snapshots: usize,
}

fn run_seed(seed: u64) -> Outcome {
    let retention = 4 + (seed % 3) as usize;
    let limits = Limits { max_log_batches: retention, ..Limits::default() };
    let mut sim = Sim::with_limits(fixture(), CLIENTS, seed, 1 + seed % 12, limits).unwrap();
    sim.set_version(LEGACY, LEGACY_CLIENT_VERSION);
    let mut w = Watch { seed, server: 0, clients: vec![0; CLIENTS] };

    // Everyone works at once for a while.
    for _ in 0..20 {
        let i = sim.rng().below(CLIENTS);
        act(&mut sim, i, false);
        let n = sim.rng().below(4);
        pump(&mut sim, &mut w, n);
    }
    quiesce(&mut sim, &mut w);
    check_quiet(&sim, seed, "after warm-up");

    // A short partition: client 1 misses a batch or two, still in the log.
    let before = sim.log_catch_ups;
    cut_off(&mut sim, &mut w, 1, "short");
    quiesce(&mut sim, &mut w);
    advance(&mut sim, &mut w, &[0, 3], 1, true);
    assert!(sim.clients()[1].pending() > 0, "seed {seed}: client 1 queued nothing offline");
    sim.reconnect(1).unwrap();
    assert!(sim.log_catch_ups > before, "seed {seed}: the short absence wasn't caught up from the log");
    quiesce(&mut sim, &mut w);
    check_quiet(&sim, seed, "after the short partition");

    // An asymmetric partition: the older app and a current one are cut off while the other two
    // push the server well past what its log keeps, one of them stalling on the way.
    let (plain, delta) = (sim.plain_snapshots, sim.delta_snapshots);
    cut_off(&mut sim, &mut w, LEGACY, "long");
    cut_off(&mut sim, &mut w, 3, "long");
    let ticks = 5 + sim.rng().below(30) as u64;
    let up = sim.rng().chance(0.5);
    sim.stall(0, up, ticks);
    let past = retention as u64 + 2 + sim.rng().below(3) as u64;
    advance(&mut sim, &mut w, &[0, 1], past, false);
    for _ in 0..10 {
        let i = sim.rng().below(2);
        act(&mut sim, i, false);
        let n = sim.rng().below(3);
        pump(&mut sim, &mut w, n);
    }
    for i in [LEGACY, 3] {
        assert!(sim.clients()[i].pending() > 0, "seed {seed}: client {i} queued nothing offline");
    }
    // They come back while the others' batches are still in flight.
    sim.reconnect(LEGACY).unwrap();
    sim.reconnect(3).unwrap();
    assert!(sim.plain_snapshots > plain, "seed {seed}: the older app didn't catch up with a plain snapshot");
    assert!(sim.delta_snapshots > delta, "seed {seed}: the current app didn't catch up with a compressed snapshot");
    quiesce(&mut sim, &mut w);
    check_quiet(&sim, seed, "after the long partition");

    // Everyone at once with every fault: drops, reconnects, stalls, undo and redo.
    for _ in 0..60 {
        let i = sim.rng().below(CLIENTS);
        act(&mut sim, i, true);
        let n = sim.rng().below(4);
        pump(&mut sim, &mut w, n);
    }
    for i in 0..CLIENTS {
        sim.reconnect(i).unwrap();
    }
    quiesce(&mut sim, &mut w);
    check_quiet(&sim, seed, "at the end");
    assert!(sim.undos > 0, "seed {seed}: no undo or redo was accepted");
    Outcome { undos: sim.undos, log_catch_ups: sim.log_catch_ups, delta_snapshots: sim.delta_snapshots, plain_snapshots: sim.plain_snapshots }
}

#[test]
fn the_fixture_and_every_kind_of_edit_stay_valid() {
    let doc = fixture();
    assert!(doc.validate().is_empty(), "{:?}", doc.validate());
    assert_eq!(of_kind(&doc, |k| matches!(k, NodeKind::Instance { .. })).len(), 2);
    let mut rng = Rng::new(3);
    let mut d = doc.clone();
    for step in 0..500 {
        fault_edit(&mut d, &mut rng);
        assert!(d.validate().is_empty(), "step {step}: {:?}", d.validate());
    }
    assert!(d.variables.variables.len() + d.comments.len() > 0);
}

#[test]
fn dst_fault_injection_converges_on_every_seed() {
    let seeds: Vec<u64> = match std::env::var("FIGCRAFT_DST_SEED").ok().and_then(|s| s.parse().ok()) {
        Some(one) => vec![one],
        None => (0..32).collect(),
    };
    let mut total = Outcome { undos: 0, log_catch_ups: 0, delta_snapshots: 0, plain_snapshots: 0 };
    for seed in seeds {
        let o = run_seed(seed);
        total.undos += o.undos;
        total.log_catch_ups += o.log_catch_ups;
        total.delta_snapshots += o.delta_snapshots;
        total.plain_snapshots += o.plain_snapshots;
    }
    assert!(total.undos > 0 && total.log_catch_ups > 0 && total.delta_snapshots > 0 && total.plain_snapshots > 0);
}
