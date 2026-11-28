//! Multi-seed mixed-version DST for `CatchUp::DeltaSnapshot`.

use figcraft_color::{Color, Paint};
use figcraft_doc::{Document, NodeId};
use figcraft_geom::Affine;

use crate::delta_snapshot::{decode_delta_snapshot, encode_delta_snapshot};
use crate::delta_snapshot_tests::{LEGACY_CLIENT_VERSION, assert_docs_json_eq, design_system_doc};
use crate::message::{CatchUp, ClientMsg, Hello, ServerMsg};
use crate::sim::Rng;
use crate::{Client, Limits, NoRecompute, Outgoing, Server};

fn deliver_outgoing(clients: &mut [(Client, Hello, bool)], outs: Vec<Outgoing>) {
    for out in outs {
        match out {
            Outgoing::All(m) => {
                for (c, _, _) in clients.iter_mut() {
                    if c.is_online() {
                        let _ = c.receive(m.clone(), &NoRecompute).unwrap();
                    }
                }
            }
            Outgoing::To(target, m) => {
                for (c, _, _) in clients.iter_mut() {
                    if c.is_online() && c.id() == target {
                        let _ = c.receive(m.clone(), &NoRecompute).unwrap();
                    }
                }
            }
            Outgoing::Others(skip, m) => {
                for (c, _, _) in clients.iter_mut() {
                    if c.is_online() && c.id() != skip {
                        let _ = c.receive(m.clone(), &NoRecompute).unwrap();
                    }
                }
            }
        }
    }
}

fn reconnect_dst_client(server: &mut Server, clients: &mut [(Client, Hello, bool)], idx: usize) -> (bool, bool) {
    let (c, base_h, delta_capable) = &mut clients[idx];
    let ClientMsg::Hello(mut rh) = c.reconnect(base_h.clone()) else { panic!("expected Hello") };
    if !*delta_capable {
        rh.client_version = base_h.client_version.clone();
    }
    let (id, outs) = server.join(rh).unwrap();
    let welcome = outs
        .into_iter()
        .find_map(|o| match o {
            Outgoing::To(t, m @ ServerMsg::Welcome { .. }) if t == id => Some(m),
            _ => None,
        })
        .unwrap();
    let is_delta_snap = matches!(welcome, ServerMsg::Welcome { catch_up: CatchUp::DeltaSnapshot(_), .. });
    let is_legacy_snap = matches!(welcome, ServerMsg::Welcome { catch_up: CatchUp::Snapshot(_), .. });
    let follow = c.receive(welcome, &NoRecompute).unwrap();
    let cid = c.id();
    for msg in follow {
        let rep = server.receive(cid, msg);
        deliver_outgoing(clients, rep);
    }
    (is_delta_snap, is_legacy_snap)
}

fn apply_random_dst_edit(edited: &mut Document, rng: &mut Rng, comp: NodeId, main_children: &[NodeId], seed: u64, step: usize) {
    let all_ids: Vec<NodeId> = edited.ids().filter(|id| !edited.node(*id).unwrap().is_page()).collect();
    match rng.below(4) {
        0 => {
            let target = *rng.pick(main_children).unwrap();
            if let Some(n) = edited.node_mut(target) {
                n.fills = vec![Paint::solid(Color::from_rgb8_const(rng.below(255) as u8, 80, 140))];
            }
        }
        1 => {
            let text_mirrors: Vec<NodeId> =
                all_ids.iter().copied().filter(|id| edited.node(*id).is_some_and(|n| n.source.is_some() && n.kind.text().is_some())).collect();
            if let Some(&target) = rng.pick(&text_mirrors)
                && let Some(n) = edited.node_mut(target)
            {
                if let Some(t) = n.kind.text_mut() {
                    t.characters = format!("S{seed}-T{step}");
                }
                if rng.chance(0.5) {
                    n.overrides.clear();
                } else {
                    n.overrides.insert("characters".to_string());
                }
            }
        }
        2 => {
            let mirrors: Vec<NodeId> = all_ids.iter().copied().filter(|id| edited.node(*id).is_some_and(|n| n.source.is_some())).collect();
            if let Some(&target) = rng.pick(&mirrors)
                && let Some(n) = edited.node_mut(target)
            {
                n.transform = Affine::translate((rng.below(400) as f64, rng.below(200) as f64));
                n.width = 20.0 + rng.below(120) as f64;
            }
        }
        _ => {
            if let Some(n) = edited.node_mut(comp) {
                n.name = format!("Card-v{step}");
            }
        }
    }
}

fn assert_dst_invariants(server: &Server, clients: &[(Client, Hello, bool)], seed: u64, step: usize) {
    let decoded = decode_delta_snapshot(&encode_delta_snapshot(server.replica()).unwrap(), 64 << 20, Limits::default().max_nodes).unwrap();
    assert_docs_json_eq(server.document(), decoded.document(), &format!("seed={seed} step={step} roundtrip"));
    for (i, (c, _, _)) in clients.iter().enumerate() {
        if c.is_online() {
            assert_docs_json_eq(server.document(), c.document(), &format!("seed={seed} step={step} client {i}"));
        }
    }
}

fn run_dst_seed(seed: u64) {
    let mut rng = Rng::new(seed);
    let (doc, comp, main_children) = design_system_doc(6);
    let limits = Limits { max_log_batches: 2, ..Limits::default() };
    let mut server = Server::new(doc, "dst-secret", limits);

    let join_client = |srv: &mut Server, name: &str, delta_capable: bool| -> (Client, Hello) {
        let ClientMsg::Hello(mut h) = Client::hello("dst", "dst-secret", name, "#2266EE") else { panic!() };
        if !delta_capable {
            h.client_version = LEGACY_CLIENT_VERSION.into();
        }
        let (id, outs) = srv.join(h.clone()).unwrap();
        let welcome = outs
            .into_iter()
            .find_map(|o| match o {
                Outgoing::To(t, m @ ServerMsg::Welcome { .. }) if t == id => Some(m),
                _ => None,
            })
            .unwrap();
        if delta_capable {
            assert!(matches!(welcome, ServerMsg::Welcome { catch_up: CatchUp::DeltaSnapshot(_), .. }));
        } else {
            assert!(matches!(welcome, ServerMsg::Welcome { catch_up: CatchUp::Snapshot(_), .. }));
        }
        (Client::join(welcome).unwrap(), h)
    };

    let (c0, h0) = join_client(&mut server, "Delta-A", true);
    let (c1, h1) = join_client(&mut server, "Delta-B", true);
    let (c2, h2) = join_client(&mut server, "Legacy-C", false);
    let mut clients = vec![(c0, h0, true), (c1, h1, true), (c2, h2, false)];
    let (mut delta_snap_rejoins, mut legacy_snap_rejoins) = (0usize, 0usize);

    for step in 0..30usize {
        if step == 15 {
            let saved = server.state().unwrap();
            server = Server::restore(saved, "dst-secret", limits).unwrap();
            for (c, _, _) in &mut clients {
                c.disconnect();
            }
            for idx in 0..clients.len() {
                let _ = reconnect_dst_client(&mut server, &mut clients, idx);
            }
            assert_dst_invariants(&server, &clients, seed, step);
            continue;
        }

        if step == 6 || step == 20 {
            let victim = if step == 6 { 1 } else { 2 };
            clients[victim].0.disconnect();
            server.leave(clients[victim].0.id());

            let mut off_doc = clients[victim].0.document().clone();
            if let Some(p) = off_doc.first_page()
                && let Some(n) = off_doc.node_mut(p)
            {
                n.name = format!("Offline-{seed}-{step}");
            }
            let _ = clients[victim].0.edit(&off_doc).unwrap();

            for sub in 0..4 {
                let mut edited = clients[0].0.document().clone();
                apply_random_dst_edit(&mut edited, &mut rng, comp, &main_children, seed, step * 10 + sub);
                if let Some(msg) = clients[0].0.edit(&edited).unwrap() {
                    let outs = server.receive(clients[0].0.id(), msg);
                    deliver_outgoing(&mut clients, outs);
                }
            }

            let (was_delta, was_legacy) = reconnect_dst_client(&mut server, &mut clients, victim);
            delta_snap_rejoins += usize::from(was_delta);
            legacy_snap_rejoins += usize::from(was_legacy);
            assert_dst_invariants(&server, &clients, seed, step);
            continue;
        }

        let actor_idx = rng.below(clients.len());
        let mut edited = clients[actor_idx].0.document().clone();
        apply_random_dst_edit(&mut edited, &mut rng, comp, &main_children, seed, step);
        if let Some(msg) = clients[actor_idx].0.edit(&edited).unwrap() {
            let outs = server.receive(clients[actor_idx].0.id(), msg);
            deliver_outgoing(&mut clients, outs);
        }
        assert_dst_invariants(&server, &clients, seed, step);
    }

    assert!(delta_snap_rejoins > 0 && legacy_snap_rejoins > 0, "seed={seed}: must exercise both snapshot rejoin paths");
}

#[test]
fn dst_mixed_delta_and_legacy_clients_converge_under_faults_and_restarts() {
    let seeds: Vec<u64> = match std::env::var("FIGCRAFT_DST_SEED").ok().and_then(|s| s.parse::<u64>().ok()) {
        Some(single) => vec![single],
        None => vec![1, 7, 42, 1337, 2026],
    };
    for seed in seeds {
        run_dst_seed(seed);
    }
}
