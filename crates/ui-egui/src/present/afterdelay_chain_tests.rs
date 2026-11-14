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

/// A hover that changes the variant and reverts leaves the variant shown again on its own After
/// delay deadline (counted from the screen's arrival), not from the hover.
#[test]
fn a_hover_variant_that_reverts_keeps_the_after_delay_deadline() {
    let mut s = Session::new();
    let variants: Vec<NodeId> = ["A", "Hover", "Next"]
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
    let instant = json!({"type": "instant"});
    let delay = json!({"id": variants[0], "trigger": {"type": "afterDelay", "ms": 1000}, "action": {"type": "changeTo", "dest": variants[2]}, "transition": instant, "durationMs": 0});
    ex(&mut s, "proto.interaction.add", delay);
    let hover = json!({"id": variants[0], "trigger": {"type": "onHover"}, "action": {"type": "changeTo", "dest": variants[1]}, "transition": instant, "durationMs": 0});
    ex(&mut s, "proto.interaction.add", hover);
    let set = s.doc().unwrap().node(variants[0]).unwrap().parent.unwrap();
    let screen = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 400, "height": 300, "name": "Screen"})));
    let inst = id(&ex(&mut s, "instance.create", json!({"component": set, "x": 20, "y": 20, "parent": screen})));
    ex(&mut s, "instance.swap", json!({"ids": [inst], "component": variants[0]}));
    let mut u = Harness::new(s);
    u.run("proto.present", json!({"start": screen}));
    u.frame(Vec::new());
    let start = u.time;
    let hot = u.app.inspect()["presenter"]["hotspots"].as_array().cloned().unwrap_or_default();
    let h = hot.iter().find(|h| h["id"] == json!(inst)).unwrap_or_else(|| panic!("no hotspot on the instance: {hot:?}"));
    let r: Vec<f32> = h["rect"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
    let (on, away) = (egui::pos2((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0), egui::pos2(r[0] - 40.0, r[1] - 40.0));
    while u.time < start + 0.8 {
        u.frame(vec![egui::Event::PointerMoved(away)]);
    }
    u.frame(vec![egui::Event::PointerMoved(on)]);
    u.frame(Vec::new());
    assert_eq!(shown(&u, inst, &variants), Some(1), "while hovering: the hover variant");
    while u.time < start + 0.9 {
        u.frame(vec![egui::Event::PointerMoved(on)]);
    }
    u.frame(vec![egui::Event::PointerMoved(away)]);
    u.frame(Vec::new());
    assert_eq!(shown(&u, inst, &variants), Some(0), "the hover left: A again");
    while u.time < start + 1.3 {
        u.frame(vec![egui::Event::PointerMoved(away)]);
    }
    assert_eq!(shown(&u, inst, &variants), Some(2), "A's 1,000 ms delay from the arrival fired, not 1,000 ms after the hover");
}

/// A delay that already ran on a variant stays run after a hover changes the variant and reverts:
/// the keys the temporary change cleared come back with the revert.
#[test]
fn a_hover_variant_that_reverts_does_not_rerun_a_fired_delay() {
    let mut s = Session::new();
    ex(&mut s, "variable.createCollection", json!({"name": "Run", "modes": ["Default"]}));
    ex(&mut s, "variable.create", json!({"collection": "Run", "name": "count", "type": "number", "value": 0}));
    let count = s.doc().unwrap().variables.get("count").map(|v| v.id.clone()).unwrap();
    let variants: Vec<NodeId> = ["A", "Hover"]
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
    let instant = json!({"type": "instant"});
    // A's delay leaves A on screen: it counts its runs (count = count + 1).
    let inc = json!({"op": {"op": "add", "args": [{"value": {"type": "alias", "value": count}}, {"value": {"type": "number", "value": 1}}]}});
    let delay = json!({"id": variants[0], "trigger": {"type": "afterDelay", "ms": 100}, "action": {"type": "setVariable", "variable": count, "value": null, "expr": inc}, "transition": instant, "durationMs": 0});
    ex(&mut s, "proto.interaction.add", delay);
    let hover = json!({"id": variants[0], "trigger": {"type": "onHover"}, "action": {"type": "changeTo", "dest": variants[1]}, "transition": instant, "durationMs": 0});
    ex(&mut s, "proto.interaction.add", hover);
    let set = s.doc().unwrap().node(variants[0]).unwrap().parent.unwrap();
    let screen = id(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 400, "height": 300, "name": "Screen"})));
    let inst = id(&ex(&mut s, "instance.create", json!({"component": set, "x": 20, "y": 20, "parent": screen})));
    ex(&mut s, "instance.swap", json!({"ids": [inst], "component": variants[0]}));
    let mut u = Harness::new(s);
    u.run("proto.present", json!({"start": screen}));
    u.frame(Vec::new());
    let start = u.time;
    let hot = u.app.inspect()["presenter"]["hotspots"].as_array().cloned().unwrap_or_default();
    let h = hot.iter().find(|h| h["id"] == json!(inst)).unwrap_or_else(|| panic!("no hotspot on the instance: {hot:?}"));
    let r: Vec<f32> = h["rect"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
    let (on, away) = (egui::pos2((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0), egui::pos2(r[0] - 40.0, r[1] - 40.0));
    while u.time < start + 0.3 {
        u.frame(vec![egui::Event::PointerMoved(away)]);
    }
    let fired: Vec<(NodeId, usize)> = crate::tabs::shown_or_paused(&u.app).map(|p| p.delays_fired.iter().copied().collect()).unwrap_or_default();
    assert!(!fired.is_empty(), "A's delay ran before the hover");
    u.frame(vec![egui::Event::PointerMoved(on)]);
    u.frame(Vec::new());
    assert_eq!(shown(&u, inst, &variants), Some(1), "while hovering: the hover variant");
    u.frame(vec![egui::Event::PointerMoved(away)]);
    u.frame(Vec::new());
    assert_eq!(shown(&u, inst, &variants), Some(0), "the hover left: A again");
    u.frames(5, Vec::new);
    let p = crate::tabs::shown_or_paused(&u.app).expect("presenting");
    assert!(fired.iter().all(|k| p.delays_fired.contains(k)), "the delay that ran before the hover is still marked as run: {:?}", p.delays_fired);
    let runs = p.design.as_ref().and_then(|d| d.variables.resolve(&count, &Default::default()));
    assert_eq!(runs, Some(figcraft_doc::VariableValue::Number(1.0)), "A's delay ran once, not again after the hover");
}

/// An edit in the editor while presenting brings back the editor's variants, so the swap times of
/// the variants changed at run time no longer delay them.
#[test]
fn an_edit_while_presenting_drops_the_swap_times() {
    let (mut u, inst, variants) = chain(500);
    let start = u.time;
    let mut guard = 0;
    while shown(&u, inst, &variants) != Some(1) {
        u.frame(Vec::new());
        guard += 1;
        assert!(guard < 120, "A never changed to B");
    }
    // An edit: the presenter shows the editor's document again, the instance on A.
    u.run("page.rename", json!({"name": "Renamed"}));
    u.frame(Vec::new());
    let edited = u.time;
    // A's 500 ms delay counts from the screen's arrival, long past: it fires at once, not 500 ms
    // after the run-time change to B.
    while u.time < edited + 0.2 {
        u.frame(Vec::new());
    }
    assert!(edited < start + 0.9, "the edit came before B's own delay was due");
    assert_eq!(shown(&u, inst, &variants), Some(1), "A changed to B again right after the edit");
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
