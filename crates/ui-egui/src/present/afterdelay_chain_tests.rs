//! A chain of variant After delay steps (Change to) plays to the end, each step's delay counted
//! from when its variant appeared (#2352). Driven through egui headless on the harness's clock.

use figcraft_doc::{NodeId, NodeKind};
use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::present_tests::{Ui as Harness, id};

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

/// Variants A to D of one set, an instance on A in a screen, each step an After delay of `ms`
/// changing to the next variant. (harness presenting the screen, the instance, the variants)
fn chain(ms: u32) -> (Harness, NodeId, Vec<NodeId>) {
    let mut s = Session::new();
    let variants: Vec<NodeId> = ["A", "B", "C", "D"]
        .iter()
        .enumerate()
        .map(|(k, state)| {
            let f = id(&ex(
                &mut s,
                "frame.create",
                json!({"x": k as f64 * 150.0, "y": 600, "width": 100, "height": 60, "name": format!("State={state}")}),
            ));
            id(&ex(&mut s, "component.create", json!({"ids": [f]})))
        })
        .collect();
    ex(&mut s, "component.combineVariants", json!({"ids": variants.clone()}));
    for pair in variants.windows(2) {
        let it = json!({"id": pair[0], "trigger": {"type": "afterDelay", "ms": ms}, "action": {"type": "changeTo", "dest": pair[1]}, "transition": {"type": "instant"}, "durationMs": 0});
        ex(&mut s, "proto.interaction.add", it);
    }
    let set = s.doc().unwrap().node(variants[0]).unwrap().parent.unwrap();
    let screen = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 400, "height": 300, "name": "Screen"})));
    let inst = id(&ex(&mut s, "instance.create", json!({"component": set, "x": 20, "y": 20, "parent": screen})));
    ex(&mut s, "instance.swap", json!({"ids": [inst], "component": variants[0]}));
    let mut u = Harness::new(s);
    u.run("proto.present", json!({"start": screen}));
    (u, inst, variants)
}

/// The variant (index) the instance shows in the presenter.
fn shown(u: &Harness, inst: NodeId, variants: &[NodeId]) -> Option<usize> {
    let p = crate::tabs::shown_or_paused(&u.app)?;
    let d = p.doc.as_ref()?;
    match d.node(inst)?.kind {
        NodeKind::Instance { component, .. } => variants.iter().position(|v| *v == component),
        _ => None,
    }
}

#[test]
fn a_chain_of_variant_after_delays_plays_to_the_end() {
    let (mut u, inst, variants) = chain(0);
    u.frames(30, Vec::new);
    assert_eq!(shown(&u, inst, &variants), Some(3), "A -> B -> C -> D");
}

#[test]
fn each_step_waits_its_own_delay_from_when_its_variant_appeared() {
    let (mut u, inst, variants) = chain(500);
    // Step until the instance shows B, then note the time.
    let mut guard = 0;
    while shown(&u, inst, &variants) != Some(1) {
        u.frame(Vec::new());
        guard += 1;
        assert!(guard < 120, "A never changed to B");
    }
    let appeared = u.time;
    while u.time < appeared + 0.4 {
        u.frame(Vec::new());
    }
    assert_eq!(shown(&u, inst, &variants), Some(1), "400 ms after B appeared: still B");
    while u.time < appeared + 0.6 {
        u.frame(Vec::new());
    }
    assert_eq!(shown(&u, inst, &variants), Some(2), "600 ms after B appeared: C");
}

#[test]
fn a_single_after_delay_still_fires_once() {
    let mut s = Session::new();
    let a = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 400, "name": "A"})));
    let b = id(&ex(&mut s, "frame.create", json!({"x": 400, "y": 0, "width": 300, "height": 400, "name": "B"})));
    let it = json!({"id": a, "trigger": {"type": "afterDelay", "ms": 100}, "action": {"type": "navigate", "dest": b}, "transition": {"type": "instant"}, "durationMs": 0});
    ex(&mut s, "proto.interaction.add", it);
    let mut u = Harness::new(s);
    u.run("proto.present", json!({"start": a}));
    u.frames(3, Vec::new);
    let name = |u: &Harness| u.app.inspect()["presenter"]["name"].as_str().unwrap_or_default().to_string();
    assert_eq!(name(&u), "A", "not before the delay");
    u.frames(30, Vec::new);
    assert_eq!(name(&u), "B", "once the delay is over");
    u.frames(60, Vec::new);
    assert_eq!(name(&u), "B", "and not again");
}
