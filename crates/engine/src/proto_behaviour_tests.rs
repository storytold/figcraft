//! Prototyping behaviour, written from the public documentation of prototype triggers, actions,
//! animations, easing and interactive components. Each case states the rule, what it implies and
//! a one-line derivation; where the documentation is silent the case says so and pins what
//! FigCraft does. The player's side (gestures, overlays, timers, scrolling, smart animate) is
//! `ui-egui`'s `present_behaviour_tests`.

use figcraft_doc::proto::{Action, Direction, Easing, KeyMods, Transition, Trigger};
use figcraft_doc::{Document, NodeId};
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::proto_run::{change_to_target, hotspots, play_interactions};

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn idv(v: &Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

#[test]
fn on_drag_starts_only_a_drag_its_animation_goes() {
    let push = |direction| Transition::Push { direction };
    // (animation, drag dx, dy (y down), starts it, derivation)
    let cases = [
        (push(Direction::Left), -40.0, 5.0, true, "push left: the screen is dragged to the left"),
        (push(Direction::Left), 40.0, 0.0, false, "push left: a drag to the right goes the other way"),
        (push(Direction::Left), -10.0, 30.0, false, "mostly downwards is not its way (the drag can scroll instead)"),
        (push(Direction::Right), 25.0, -3.0, true, "push right: a drag to the right"),
        (Transition::MoveIn { direction: Direction::Top }, 0.0, -25.0, true, "move in upwards: a drag up"),
        (Transition::MoveOut { direction: Direction::Top }, 0.0, 25.0, false, "move out upwards: not a drag down"),
        (Transition::SlideOut { direction: Direction::Bottom }, 3.0, 25.0, true, "slide out downwards: a drag down"),
        (Transition::SlideIn { direction: Direction::Right }, 20.0, -20.0, true, "exactly diagonal counts for both axes (docs silent; pinned)"),
        (Transition::SmartAnimate, 0.0, 30.0, true, "smart animate has no direction: any drag"),
        (Transition::Dissolve, -30.0, 0.0, true, "dissolve: any drag"),
        (Transition::Instant, 7.0, 0.0, true, "instant: any drag"),
    ];
    for (tr, dx, dy, want, why) in cases {
        assert_eq!(tr.accepts_drag(dx, dy), want, "{tr:?} dragged ({dx}, {dy}): {why}");
    }
}

#[test]
fn key_triggers_fire_for_their_key_or_combination() {
    let none = KeyMods::default();
    let shift = KeyMods { shift: true, ..none };
    let ctrl_shift = KeyMods { ctrl: true, shift: true, ..none };
    let alt_cmd = KeyMods { alt: true, cmd: true, ..none };
    // (the trigger's key, key pressed, modifiers held, fires, derivation)
    let cases = [
        ("K", "K", none, true, "the key alone"),
        ("k", "K", none, true, "letters in either case"),
        ("K", "K", shift, false, "a combination is a trigger of its own: K doesn't fire for Shift+K"),
        ("Shift+K", "K", shift, true, "the combination recorded"),
        ("Shift+K", "K", none, false, "not without its modifier"),
        ("Ctrl+Shift+K", "K", ctrl_shift, true, "two modifiers"),
        ("Ctrl+Shift+K", "K", shift, false, "every modifier of the combination must be held"),
        ("Command+Option+K", "K", alt_cmd, true, "other spellings of the modifiers (ClawDesign accepts them)"),
        ("Space", "Space", none, true, "named keys"),
        ("space", "Space", none, true, "names in any case"),
        ("Return", "Enter", none, true, "common other names of keys"),
        ("←", "ArrowLeft", none, true, "arrow glyphs"),
        ("Shift+Space", "Space", shift, true, "a named key in a combination"),
        ("J", "K", none, false, "another key"),
    ];
    for (want, key, mods, fires, why) in cases {
        let t = Trigger::KeyPress { key: want.into() };
        assert_eq!(t.fires_on_key(key, mods), fires, "{want:?} vs {mods:?}+{key}: {why}");
    }
    assert!(!Trigger::OnClick.fires_on_key("K", none), "only key triggers take keys");
    // Combinations are stored in one order and read back exactly.
    for bits in 0..16u8 {
        let m = KeyMods { ctrl: bits & 1 != 0, alt: bits & 2 != 0, shift: bits & 4 != 0, cmd: bits & 8 != 0 };
        assert_eq!(KeyMods::split(&m.combo("K")), (m, "K"));
    }
    assert_eq!(ctrl_shift.combo("K"), "Ctrl+Shift+K");
    // A plus key keeps its plus; junk is a key that never comes.
    assert_eq!(KeyMods::split("Ctrl++"), (KeyMods { ctrl: true, ..none }, "+"));
    assert_eq!(KeyMods::split("+"), (none, "+"));
    assert_eq!(KeyMods::split("Hyper+K"), (none, "Hyper+K"));
}

#[test]
fn easing_curves_and_springs() {
    let customs = [Easing::CUSTOM_BEZIER, Easing::CUSTOM_SPRING];
    for e in Easing::ALL.iter().chain(customs.iter()).copied() {
        assert!(e.eval(0.0).abs() < 1e-9 && (e.eval(1.0) - 1.0).abs() < 1e-9, "{e:?}: every easing starts at 0 and ends at 1");
    }
    // (easing, t, eased below/above t, derivation)
    assert_eq!(Easing::Linear.eval(0.3), 0.3, "linear: constant speed");
    assert!(Easing::EaseIn.eval(0.25) < 0.25, "ease in starts slowly");
    assert!(Easing::EaseOut.eval(0.25) > 0.25, "ease out starts fast and slows down");
    assert!((Easing::EaseInOut.eval(0.5) - 0.5).abs() < 1e-9, "ease in and out: symmetric about the middle");
    let curve = |e: Easing| (0..=100).map(move |i| e.eval(f64::from(i) / 100.0));
    assert!(curve(Easing::EaseInBack).any(|v| v < -0.05), "ease in back pulls back before it goes");
    assert!(curve(Easing::EaseOutBack).any(|v| v > 1.05), "ease out back overshoots, then settles");
    // Springs are physical: mass 1 with stiffness / damping 100/15 (gentle), 300/20 (quick),
    // 600/15 (bouncy), 80/20 (slow). Overshoot of an under-damped spring: e^(-πζ/√(1-ζ²)).
    // (preset, ζ = c / 2√(km), overshoot expected, settle seconds)
    let springs = [
        (Easing::Gentle, 0.75, 0.028, 0.92),
        (Easing::Quick, 0.577, 0.108, 0.69),
        (Easing::Bouncy, 0.306, 0.36, 0.92),
        (Easing::Slow, 1.118, 0.0, 1.66),
    ];
    for (e, zeta, overshoot, secs) in springs {
        let (m, k, c) = e.spring().unwrap();
        assert!((c / (2.0 * (k * m).sqrt()) - zeta).abs() < 1e-3, "{e:?}: damping ratio {zeta}");
        let peak = curve(e).fold(0.0, f64::max) - 1.0;
        assert!((peak.max(0.0) - overshoot).abs() < 0.01, "{e:?}: overshoot {peak} ≈ {overshoot}");
        // A spring plays until it settles, whatever duration is set.
        assert!((e.seconds(300) - secs).abs() < 0.01, "{e:?}: {} s ≈ {secs} s", e.seconds(300));
        assert_eq!(e.seconds(300), e.seconds(5000), "{e:?}: the duration field doesn't apply");
        assert!((e.eval(0.98) - 1.0).abs() < 0.005, "{e:?}: at rest when it ends (nothing jumps)");
    }
    // Curves play for the duration set.
    assert_eq!(Easing::EaseOut.seconds(300), 0.3);
    assert_eq!(Easing::CUSTOM_BEZIER.seconds(1200), 1.2);
    // Junk springs stay finite and bounded.
    for e in [
        Easing::CustomSpring { mass: 0.0, stiffness: -1.0, damping: f64::NAN },
        Easing::CustomSpring { mass: 1e300, stiffness: 1e-300, damping: 0.0 },
        Easing::CustomSpring { mass: 1.0, stiffness: 100.0, damping: 1e9 },
    ] {
        let s = e.seconds(300);
        assert!((0.05..=10.0).contains(&s), "{e:?}: {s}");
        assert!(curve(e).all(f64::is_finite), "{e:?}");
    }
}

#[test]
fn back_and_reverting_play_the_animation_the_other_way() {
    // (animation going forward, played back, derivation)
    let cases = [
        (
            Transition::MoveIn { direction: Direction::Left },
            Transition::MoveOut { direction: Direction::Right },
            "what moved in leaves the way it came",
        ),
        (Transition::MoveOut { direction: Direction::Top }, Transition::MoveIn { direction: Direction::Bottom }, "what moved out comes back"),
        (Transition::Push { direction: Direction::Left }, Transition::Push { direction: Direction::Right }, "a push left pushes back right"),
        (Transition::SlideIn { direction: Direction::Top }, Transition::SlideOut { direction: Direction::Bottom }, "slide in up, slide out down"),
        (Transition::SlideOut { direction: Direction::Left }, Transition::SlideIn { direction: Direction::Right }, "slide out left, slide in right"),
        (Transition::Dissolve, Transition::Dissolve, "a dissolve has no direction"),
        (Transition::SmartAnimate, Transition::SmartAnimate, "smart animate tweens back"),
        (Transition::Instant, Transition::Instant, "instant stays instant"),
    ];
    for (fwd, back, why) in cases {
        assert_eq!(fwd.reversed(), back, "{fwd:?}: {why}");
        assert_eq!(back.reversed(), fwd, "{back:?}: reversing twice is the original");
    }
}

/// A button component set (`State=Default`, `State=Pressed`) whose variants each hold an icon
/// instance; Default's root goes to Pressed on click and on press, Pressed's back on click. A
/// screen holds an instance of the set. Returns the session, the screen, the button instance, the nested icon instance in it
/// and the Pressed variant.
fn interactive_button() -> (Session, NodeId, NodeId, NodeId, NodeId) {
    let mut s = Session::new();
    let icon_frame = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 300, "width": 10, "height": 10, "name": "Icon"})));
    let icon = idv(&ex(&mut s, "component.create", json!({"ids": [icon_frame]})));
    let mut variants = Vec::new();
    for (k, state) in ["Default", "Pressed"].iter().enumerate() {
        let f =
            idv(&ex(&mut s, "frame.create", json!({"x": k as f64 * 150.0, "y": 0, "width": 100, "height": 40, "name": format!("State={state}")})));
        ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 40, "parent": f, "props": {"name": "Bg"}}));
        ex(&mut s, "instance.create", json!({"component": icon, "x": 4, "y": 4, "parent": f}));
        variants.push(idv(&ex(&mut s, "component.create", json!({"ids": [f]}))));
    }
    let set = idv(&ex(&mut s, "component.combineVariants", json!({"ids": variants.clone()})));
    let (default, pressed) = (variants[0], variants[1]);
    for trigger in ["onClick", "onPress"] {
        ex(&mut s, "proto.interaction.add", json!({"id": default, "trigger": {"type": trigger}, "action": {"type": "changeTo", "dest": pressed}}));
    }
    ex(&mut s, "proto.interaction.add", json!({"id": pressed, "trigger": {"type": "onClick"}, "action": {"type": "changeTo", "dest": default}}));
    let screen = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 500, "width": 300, "height": 300, "name": "Screen"})));
    let button = idv(&ex(&mut s, "instance.create", json!({"component": set, "x": 20, "y": 20, "parent": screen})));
    let nested = s
        .doc()
        .unwrap()
        .subtree(button)
        .into_iter()
        .skip(1)
        .find(|i| matches!(s.doc().unwrap().node(*i).map(|n| &n.kind), Some(figcraft_doc::NodeKind::Instance { .. })));
    (s, screen, button, nested.unwrap(), pressed)
}

#[test]
fn instances_play_their_variants_interactions() {
    let (mut s, screen, button, nested, pressed) = interactive_button();
    let triggers = |d: &Document, id| d.node(id).unwrap().interactions.iter().map(|it| it.trigger.clone()).collect::<Vec<_>>();
    // The file keeps the interactions on the main component; the player's copy plays them on
    // every instance (interactive components).
    let mut play = s.doc().unwrap().clone();
    assert!(triggers(&play, button).is_empty());
    play_interactions(&mut play);
    assert_eq!(triggers(&play, button), [Trigger::OnClick, Trigger::OnPress], "an instance plays its variant's interactions");
    assert_eq!(hotspots(&play, screen).first().map(|h| h.0), Some(button), "so it is a hotspot on the screen");
    // An interaction on the instance with the same trigger wins over the main's.
    ex(&mut s, "proto.interaction.add", json!({"id": button, "trigger": {"type": "onClick"}, "action": {"type": "back"}}));
    let mut play = s.doc().unwrap().clone();
    play_interactions(&mut play);
    let its = &play.node(button).unwrap().interactions;
    assert_eq!(its.len(), 2);
    assert_eq!((&its[0].trigger, &its[0].actions[..]), (&Trigger::OnClick, &[Action::Back][..]), "the instance's own click");
    assert_eq!(its[1].trigger, Trigger::OnPress, "the main's press still plays");
    // "Change to" on a layer inside the button changes the button, not the icon instance nested
    // in it (the variant belongs to the button's set).
    let doc = s.doc().unwrap();
    assert_eq!(change_to_target(doc, nested, pressed), Some(button));
    assert_eq!(change_to_target(doc, button, pressed), Some(button));
    assert_eq!(crate::components::owning_instance(doc, nested), Some(nested), "(the innermost instance would have been the icon)");
    // Outside any instance there is nothing to change.
    assert_eq!(change_to_target(doc, screen, pressed), None);
    // Shown as Pressed it plays Pressed's interactions, also when it holds copies of Default's
    // (instances placed by earlier versions copied the main's interactions).
    let default_its = s.doc().unwrap().node(default_of(&s, pressed)).unwrap().interactions.clone();
    ex(&mut s, "proto.interaction.clear", json!({"ids": [button]}));
    ex(&mut s, "instance.setProperties", json!({"ids": [button], "properties": {"State": "Pressed"}}));
    let mut play = s.doc().unwrap().clone();
    play.get_mut(button).unwrap().interactions = default_its;
    play_interactions(&mut play);
    let its = &play.node(button).unwrap().interactions;
    assert_eq!(its.len(), 1);
    assert!(matches!(its[0].actions[..], [Action::ChangeTo { dest }] if dest != pressed), "back to Default: {its:?}");
}

/// The other variant of `variant`'s set.
fn default_of(s: &Session, variant: NodeId) -> NodeId {
    let doc = s.doc().unwrap();
    let set = crate::components::variant_set(doc, variant).unwrap();
    doc.node(set).unwrap().children.iter().copied().find(|v| *v != variant).unwrap()
}

#[test]
fn hotspots_priority_and_click_through() {
    let mut s = Session::new();
    let a = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 300, "name": "A"})));
    let b = idv(&ex(&mut s, "frame.create", json!({"x": 400, "y": 0, "width": 300, "height": 300, "name": "B"})));
    let card = idv(&ex(&mut s, "frame.create", json!({"x": 20, "y": 20, "width": 200, "height": 100, "parent": a, "name": "Card"})));
    ex(&mut s, "node.set", json!({"id": card, "props": {"clip": true}}));
    let rect = |s: &mut Session, name: &str, parent, x: f64, y: f64| {
        idv(&ex(s, "shape.rectangle", json!({"x": x, "y": y, "width": 60, "height": 40, "parent": parent, "props": {"name": name}})))
    };
    let button = rect(&mut s, "Button", card, 30.0, 30.0);
    let edge = rect(&mut s, "Edge", card, 190.0, 30.0);
    let away = rect(&mut s, "Away", card, 260.0, 30.0);
    let hidden = rect(&mut s, "Hidden", a, 30.0, 200.0);
    let cover = rect(&mut s, "Cover", a, 30.0, 30.0);
    for id in [card, button, edge, away, hidden] {
        ex(&mut s, "proto.interaction.add", json!({"id": id, "dest": b}));
    }
    ex(&mut s, "node.set", json!({"id": hidden, "props": {"visible": false}}));
    let doc = s.doc().unwrap();
    let hot = hotspots(doc, a);
    let ids: Vec<NodeId> = hot.iter().map(|h| h.0).collect();
    // Bottom-most first: the player tries them top-most first, so the button (inside the card)
    // wins a click over the card, and the card takes clicks on the rest of it.
    assert_eq!(ids, [card, button, edge], "nested hotspots, the deepest on top");
    // Layers without interactions are not hotspots: the cover over the button doesn't block it
    // (docs silent on covered hotspots; pinned: clicks reach the hotspot under it).
    assert!(!ids.contains(&cover));
    // Hidden layers aren't hotspots; a clipping frame cuts what's outside it.
    assert!(!ids.contains(&hidden), "hidden");
    assert!(!ids.contains(&away), "clipped away entirely");
    let e = hot[2].1;
    assert_eq!((e.x0, e.x1), (210.0, 220.0), "the edge layer is clipped to the card (20 + 200)");
}

#[test]
fn scroll_to_keeps_an_offset() {
    let mut s = Session::new();
    let a = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 900, "name": "A"})));
    let item = idv(&ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 700, "width": 300, "height": 40, "parent": a})));
    let add = |s: &mut Session, action: Value| s.execute("proto.interaction.add", &json!({"id": a, "action": action}));
    add(&mut s, json!({"type": "scrollTo", "dest": item, "offset": [0, 60]})).unwrap();
    add(&mut s, json!({"type": "scrollTo", "dest": item})).unwrap();
    let its = &s.doc().unwrap().node(a).unwrap().interactions;
    assert_eq!(its[0].actions, [Action::ScrollTo { dest: item, offset: [0.0, 60.0] }]);
    assert_eq!(its[1].actions, [Action::ScrollTo { dest: item, offset: [0.0, 0.0] }], "no offset: the layer goes to the top");
    // Files only carry an offset that was set.
    assert!(serde_json::to_value(&its[1].actions[0]).unwrap().get("offset").is_none());
    // Junk is refused, nothing changes.
    assert!(add(&mut s, json!({"type": "scrollTo", "dest": item, "offset": "top"})).is_err());
    assert_eq!(s.doc().unwrap().node(a).unwrap().interactions.len(), 2);
}
