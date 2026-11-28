//! Collaboration over real commands: every edit round-trips through its change batch, and several
//! people editing the same file at once all end up with the server's document.

use figcraft_collab::sim::{Rng, Sim};
use figcraft_collab::{Mode, NoRecompute, Recompute, Replica, diff};
use figcraft_doc::{Document, NodeId, NodeKind};
use serde_json::{Value, json};

use crate::Session;
use crate::collab::EngineRecompute;

/// A starting file: a frame with a few layers and some text.
pub(crate) fn base() -> Document {
    let mut s = Session::new();
    let f = s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 400, "height": 300})).unwrap()["id"].clone();
    for i in 0..3 {
        s.execute("shape.rectangle", &json!({"x": 20 + i * 60, "y": 20, "width": 50, "height": 40, "parent": f})).unwrap();
    }
    s.execute("text.create", &json!({"text": "Hello", "x": 20, "y": 100, "parent": f})).unwrap();
    s.execute("shape.ellipse", &json!({"x": 500, "y": 0, "width": 80, "height": 80})).unwrap();
    // A component with an instance (instances follow edits of their main) and variables to bind.
    let card = s.execute("frame.create", &json!({"x": 0, "y": 400, "width": 120, "height": 80})).unwrap()["id"].clone();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 410, "width": 30, "height": 30, "parent": card})).unwrap();
    let main = s.execute("component.create", &json!({"ids": [card]})).unwrap()["id"].clone();
    s.execute("instance.create", &json!({"component": main, "x": 200, "y": 400})).unwrap();
    s.execute("variable.createCollection", &json!({"name": "Tokens"})).unwrap();
    for v in 0..4 {
        s.execute("variable.create", &json!({"collection": "Tokens", "name": format!("v{v}"), "type": "color", "value": "#3366FF"})).unwrap();
    }
    s.doc().unwrap().clone()
}

/// A random command with parameters that make sense for `doc` (often valid, sometimes not:
/// cycles, deleted targets…).
pub(crate) fn random_command(doc: &Document, rng: &mut Rng, with_undo: bool) -> (&'static str, Value) {
    let store = doc.libraries.store;
    let all: Vec<&figcraft_doc::Node> = doc.nodes().filter(|n| store.is_none_or(|st| doc.page_of(n.id) != Some(st))).collect();
    let ids = |f: &dyn Fn(&figcraft_doc::Node) -> bool| -> Vec<NodeId> { all.iter().filter(|n| f(n)).map(|n| n.id).collect() };
    let layers = ids(&|n| !n.is_page());
    let containers = ids(&|n| n.kind.is_container() && !matches!(n.kind, NodeKind::Instance { .. }));
    let frames = ids(&|n| matches!(n.kind, NodeKind::Frame));
    let components = ids(&|n| matches!(n.kind, NodeKind::Component { .. }));
    let texts = ids(&|n| matches!(n.kind, NodeKind::Text(_)));
    let one = |v: &[NodeId], rng: &mut Rng| rng.pick(v).copied().map_or(json!(0), |i| json!(i));
    let xy = |rng: &mut Rng| (rng.below(800) as f64 - 100.0, rng.below(600) as f64 - 100.0);
    let color = |rng: &mut Rng| format!("#{:06X}", rng.next_u64() & 0xFF_FFFF);
    let (x, y) = xy(rng);
    match rng.below(if with_undo { 30 } else { 28 }) {
        0 | 1 => {
            ("shape.rectangle", json!({"x": x, "y": y, "width": 10 + rng.below(90), "height": 10 + rng.below(90), "parent": one(&containers, rng)}))
        }
        2 => ("frame.create", json!({"x": x, "y": y, "width": 100 + rng.below(200), "height": 100 + rng.below(200)})),
        3 => ("text.create", json!({"text": format!("Text {}", rng.below(100)), "x": x, "y": y, "parent": one(&frames, rng)})),
        4 | 5 => ("transform.move", json!({"ids": [one(&layers, rng)], "dx": rng.below(41) as f64 - 20.0, "dy": rng.below(41) as f64 - 20.0})),
        6 => ("transform.resize", json!({"ids": [one(&layers, rng)], "width": 5 + rng.below(200), "height": 5 + rng.below(200)})),
        7 | 8 => ("node.reparent", json!({"ids": [one(&layers, rng)], "parent": one(&containers, rng)})),
        9 => ("node.reorder", json!({"id": one(&layers, rng), "index": rng.below(4)})),
        10 => ("edit.delete", json!({"ids": [one(&layers, rng)]})),
        11 => ("edit.duplicate", json!({"ids": [one(&layers, rng)]})),
        12 => ("object.group", json!({"ids": [one(&layers, rng), one(&layers, rng)]})),
        13 => ("object.ungroup", json!({"ids": [one(&layers, rng)]})),
        14 => ("layout.add", json!({"ids": [one(&frames, rng)]})),
        15 => ("node.set", json!({"id": one(&frames, rng), "props": {"padding": rng.below(30), "itemSpacing": rng.below(20)}})),
        16 => ("component.create", json!({"ids": [one(&layers, rng)]})),
        17 => ("instance.create", json!({"component": one(&components, rng), "x": x, "y": y})),
        18 => ("fill.set", json!({"ids": [one(&layers, rng)], "color": color(rng)})),
        19 => ("style.create", json!({"type": "paint", "name": format!("Color {}", rng.below(5)), "from": one(&layers, rng)})),
        20 => ("style.apply", json!({"ids": [one(&layers, rng)], "style": format!("Color {}", rng.below(5))})),
        21 => ("text.set", json!({"id": one(&texts, rng), "characters": format!("Typed {}", rng.below(1000))})),
        22 => match rng.below(3) {
            0 => ("page.add", json!({"name": format!("Page {}", rng.below(9))})),
            // Not the first page: deleting it would take most of the file with it.
            1 => ("page.delete", json!({"id": one(doc.pages.get(1..).unwrap_or_default(), rng)})),
            _ => ("page.move", json!({"id": one(&doc.pages, rng), "index": rng.below(3)})),
        },
        23 => ("object.moveToPage", json!({"ids": [one(&layers, rng)], "page": one(&doc.pages, rng)})),
        24 => ("comment.add", json!({"x": x, "y": y, "text": "Looks good", "page": one(&doc.pages, rng)})),
        25 => match rng.below(3) {
            0 => ("variable.createCollection", json!({"name": "Tokens"})),
            1 => ("variable.create", json!({"collection": "Tokens", "name": format!("v{}", rng.below(4)), "type": "color", "value": color(rng)})),
            _ => ("variable.bind", json!({"ids": [one(&layers, rng)], "field": "fill", "variable": format!("v{}", rng.below(4))})),
        },
        26 => ("effect.add", json!({"ids": [one(&layers, rng)], "type": "dropShadow"})),
        27 => (
            "node.set",
            json!({"id": one(&layers, rng), "props": {"name": format!("Layer {}", rng.below(50)), "opacity": rng.below(11) as f64 / 10.0}}),
        ),
        28 => ("edit.undo", json!({})),
        _ => ("edit.redo", json!({})),
    }
}

/// `apply(diff(a, b), a) == b` for every edit of a long random sequence of real commands (undo and
/// redo included), and snapshots rebuild the same state.
#[test]
fn every_command_edit_round_trips_through_its_changes() {
    for seed in 0..6 {
        let mut rng = Rng::new(seed);
        let mut s = Session::with_document(base());
        let mut replica = Replica::new(s.doc().unwrap().clone());
        let mut edits = 0;
        for _ in 0..150 {
            let (cmd, params) = random_command(s.doc().unwrap(), &mut rng, true);
            if s.execute(cmd, &params).is_err() {
                continue;
            }
            let doc = s.doc().unwrap();
            let d = diff(&replica, doc).unwrap();
            let mut applied = replica.clone();
            applied.apply(&d.changes, Mode::Strict).unwrap_or_else(|e| panic!("seed {seed}, {cmd} {params}: {e}\n{:#?}", d.changes));
            assert_eq!(applied.document(), doc, "seed {seed}: {cmd} {params}");
            assert_eq!(applied, d.replica, "seed {seed}: {cmd} positions");
            edits += usize::from(!d.changes.is_empty());
            replica = d.replica;
        }
        assert!(edits > 50, "seed {seed}: only {edits} edits");
        let back = Replica::from_snapshot(replica.snapshot().unwrap()).unwrap();
        assert_eq!(back, replica, "seed {seed}: snapshot");
    }
}

/// Run `clients` people editing with real commands for `rounds` edits; messages are delayed at
/// random per link. Returns the settled session.
fn session(seed: u64, clients: usize, rounds: usize, recompute: &dyn Recompute) -> Sim {
    let mut sim = Sim::new(base(), clients, seed, 10).unwrap();
    for _ in 0..rounds {
        let i = sim.rng().below(clients);
        let doc = sim.document(i).unwrap().clone();
        let mut s = Session::with_document(doc);
        let mut rng = Rng::new(sim.rng().next_u64());
        let (cmd, params) = random_command(s.doc().unwrap(), &mut rng, false);
        if s.execute(cmd, &params).is_ok() {
            sim.edit(i, s.doc().unwrap()).unwrap_or_else(|e| panic!("seed {seed}: {cmd} {params}: {e}"));
        }
        for _ in 0..sim.rng().below(4) {
            sim.step(recompute).unwrap();
        }
    }
    sim.settle(recompute, 1_000_000).unwrap();
    sim
}

/// The convergence fuzz test: after quiescence every client's document equals the server's.
#[test]
fn people_editing_at_once_converge() {
    let (mut rejected, mut repairs) = (0, 0);
    for seed in 0..5 {
        let sim = session(seed, 3, 60, &EngineRecompute);
        if let Err(e) = sim.converged() {
            panic!("seed {seed}: {e}");
        }
        rejected += sim.rejected;
        repairs += sim.repairs;
        assert!(sim.server.seq() > 20, "seed {seed}: only {} batches accepted", sim.server.seq());
        assert!(sim.server.document().validate().is_empty(), "seed {seed}: {:?}", sim.server.document().validate());
    }
    // Conflicts happened: batches refused (edits to deleted layers, cycles) and derived data
    // repaired after concurrent edits.
    assert!(rejected > 0 && repairs > 0, "{rejected} rejected, {repairs} repairs");
    // Without the recompute hook they converge too (derived data may then be stale).
    let sim = session(99, 4, 40, &NoRecompute);
    assert_eq!(sim.converged(), Ok(()));
}

#[test]
#[ignore = "longer run for development: cargo test -p figcraft-engine collab -- --ignored"]
fn many_sessions_converge() {
    for seed in 100..160 {
        let sim = session(seed, 4, 150, &EngineRecompute);
        if let Err(e) = sim.converged() {
            panic!("seed {seed}: {e}");
        }
    }
}

#[test]
fn collab_diff_shows_the_last_edit() {
    let mut s = Session::with_document(base());
    assert!(s.execute("collab.diff", &json!({})).is_err(), "nothing to undo yet");
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    let r = s.execute("collab.diff", &json!({})).unwrap();
    assert_eq!(r["count"], 1);
    assert!(r["changes"][0]["create"]["kind"]["type"] == "rectangle", "{r}");
    assert!(r["bytes"].as_u64().unwrap() > 0);
    let saved = s.execute("collab.diff", &json!({"against": "saved"})).unwrap();
    assert!(saved["count"].as_u64().unwrap() >= 1);
    assert!(s.execute("collab.diff", &json!({"against": "nope"})).is_err());
}
