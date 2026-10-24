//! Prototype player behaviour, written from the public documentation of prototype triggers,
//! actions, overlays, animations and scrolling. Each case states the rule, what it implies and a
//! one-line derivation; where the documentation is silent the case says so and pins what FigCraft
//! does. Gestures go through egui headless (the harness is `present_tests`'); the rest drives the
//! player directly. Model rules (drag directions, key combinations, easing, interactive
//! components, hotspots) are in the engine's `proto_behaviour_tests`.

use std::sync::Arc;

use egui::{Event, Key, Modifiers, PointerButton, Pos2, pos2, vec2};
use figcraft_doc::proto::{Action, Direction, Easing, OverlayPosition, Transition};
use figcraft_doc::{Document, NodeId, NodeKind};
use figcraft_engine::Session;
use serde_json::{Value, json};

use super::{Presenter, next_delay, raise_fixed, smart_interpolate};
use crate::present_tests::{Ui as Harness, id};

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn frame(s: &mut Session, name: &str, x: f64, w: f64, h: f64) -> NodeId {
    id(&ex(s, "frame.create", json!({"x": x, "y": 0, "width": w, "height": h, "name": name})))
}

fn rect(s: &mut Session, name: &str, parent: NodeId, [x, y, w, h]: [f64; 4]) -> NodeId {
    id(&ex(s, "shape.rectangle", json!({"x": x, "y": y, "width": w, "height": h, "parent": parent, "props": {"name": name}})))
}

fn on(s: &mut Session, layer: NodeId, trigger: Value, action: Value, transition: Value, ms: u32) {
    ex(s, "proto.interaction.add", json!({"id": layer, "trigger": trigger, "action": action, "transition": transition, "durationMs": ms}));
}

fn presenter(u: &Harness) -> Value {
    u.app.inspect()["presenter"].clone()
}

fn screen_name(u: &Harness) -> String {
    presenter(u)["name"].as_str().unwrap_or_default().to_string()
}

/// The centre of the presenter's hotspot on `layer`.
fn hotspot(u: &Harness, layer: NodeId) -> Pos2 {
    let hot = presenter(u)["hotspots"].as_array().cloned().unwrap_or_default();
    let h = hot.iter().find(|h| h["id"] == json!(layer)).unwrap_or_else(|| panic!("no hotspot on {layer}: {hot:?}"));
    let r: Vec<f32> = h["rect"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
    pos2((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0)
}

fn button(at: Pos2, pressed: bool, modifiers: Modifiers) -> Event {
    Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers }
}

/// What the pointer or the keyboard does over a layer.
#[derive(Clone, Copy, Debug)]
enum Gesture {
    Click,
    /// Press, move by (dx, dy) in five steps, release.
    Drag(f32, f32),
    /// Press and keep the button down.
    Hold,
    HoldRelease,
    Hover,
    /// Hover, then move off the layer.
    HoverLeave,
    Key(Key, Modifiers),
    /// Wait (seconds) with the pointer away.
    Wait(f64),
}

impl Gesture {
    fn play(self, u: &mut Harness, at: Pos2) {
        let none = Modifiers::NONE;
        let away = pos2(at.x - 40.0, at.y - 40.0);
        u.frame(vec![Event::PointerMoved(if matches!(self, Gesture::Wait(_)) { away } else { at })]);
        match self {
            Gesture::Click => {
                u.frame(vec![button(at, true, none)]);
                u.frame(vec![button(at, false, none)]);
            }
            Gesture::Drag(dx, dy) => {
                u.frame(vec![button(at, true, none)]);
                for k in 1..=5 {
                    u.frame(vec![Event::PointerMoved(at + vec2(dx, dy) * (k as f32 / 5.0))]);
                }
                u.frame(vec![button(at + vec2(dx, dy), false, none)]);
            }
            Gesture::Hold | Gesture::HoldRelease => {
                u.frame(vec![button(at, true, none)]);
                u.frames(2, Vec::new);
                if matches!(self, Gesture::HoldRelease) {
                    u.frame(vec![button(at, false, none)]);
                }
            }
            Gesture::Hover => u.frames(2, Vec::new),
            Gesture::HoverLeave => {
                u.frames(2, Vec::new);
                u.frame(vec![Event::PointerMoved(away)]);
            }
            Gesture::Key(key, modifiers) => {
                let k = |pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers };
                u.frame(vec![k(true), k(false)]);
            }
            Gesture::Wait(secs) => u.frames((secs * 60.0).round() as usize, Vec::new),
        }
        u.frames(2, Vec::new);
    }
}

/// Screens A (300×400, presented) and B; a 100×60 layer "Hot" at (50, 50) in A whose one
/// interaction (`trigger`, `transition`) navigates to B.
fn one_interaction(trigger: Value, transition: Value, ms: u32) -> (Harness, NodeId) {
    let mut s = Session::new();
    let a = frame(&mut s, "A", 0.0, 300.0, 400.0);
    let b = frame(&mut s, "B", 400.0, 300.0, 400.0);
    let hot = rect(&mut s, "Hot", a, [50.0, 50.0, 100.0, 60.0]);
    on(&mut s, hot, trigger, json!({"type": "navigate", "dest": b}), transition, ms);
    let mut u = Harness::new(s);
    u.run("proto.present", json!({"start": a}));
    (u, hot)
}

#[test]
fn triggers_fire_on_their_gesture() {
    let instant = json!({"type": "instant"});
    let push_left = json!({"type": "push", "direction": "left"});
    let t = |ty: &str| json!({"type": ty});
    let key = |k: &str| json!({"type": "keyPress", "key": k});
    let (none, shift) = (Modifiers::NONE, Modifiers::SHIFT);
    // (trigger, animation, gesture on the layer, screen after, derivation)
    let cases = [
        (t("onClick"), &instant, Gesture::Click, "B", "on click: press and release on the layer"),
        (t("onClick"), &instant, Gesture::Drag(40.0, 0.0), "A", "a drag is not a click"),
        (t("onDrag"), &instant, Gesture::Drag(40.0, 0.0), "B", "on drag: a drag that starts on the layer"),
        (t("onDrag"), &instant, Gesture::Click, "A", "a click is not a drag"),
        (t("onDrag"), &push_left, Gesture::Drag(-60.0, 0.0), "B", "on drag with push left: a drag to the left"),
        (t("onDrag"), &push_left, Gesture::Drag(60.0, 0.0), "A", "a drag against the animation's direction doesn't start it"),
        (t("mouseDown"), &instant, Gesture::Hold, "B", "mouse down: as the button goes down"),
        (t("mouseUp"), &instant, Gesture::Hold, "A", "mouse up: not before the button comes up"),
        (t("mouseUp"), &instant, Gesture::Click, "B", "mouse up: when it does"),
        (t("onPress"), &instant, Gesture::Hold, "B", "while pressing: shown while the button is down"),
        (t("onPress"), &instant, Gesture::HoldRelease, "A", "while pressing: reverts on release"),
        (t("onHover"), &instant, Gesture::Hover, "B", "while hovering: shown while the pointer is over the layer"),
        (t("onHover"), &instant, Gesture::HoverLeave, "A", "while hovering: reverts when the pointer leaves"),
        (t("mouseEnter"), &instant, Gesture::HoverLeave, "B", "mouse enter: fires on entering and doesn't revert"),
        (t("mouseLeave"), &instant, Gesture::Hover, "A", "mouse leave: not while the pointer is still over it"),
        (t("mouseLeave"), &instant, Gesture::HoverLeave, "B", "mouse leave: when it leaves"),
        (key("K"), &instant, Gesture::Key(Key::K, none), "B", "key: pressed anywhere on the screen"),
        (key("Shift+K"), &instant, Gesture::Key(Key::K, none), "A", "a combination needs its modifier"),
        (key("Shift+K"), &instant, Gesture::Key(Key::K, shift), "B", "the combination"),
        (json!({"type": "afterDelay", "ms": 500}), &instant, Gesture::Wait(0.3), "A", "after delay: not before the delay"),
        (json!({"type": "afterDelay", "ms": 500}), &instant, Gesture::Wait(0.6), "B", "after delay: timed from when the screen shows"),
    ];
    for (trigger, transition, gesture, want, why) in cases {
        let (mut u, hot) = one_interaction(trigger.clone(), transition.clone(), 300);
        let at = hotspot(&u, hot);
        gesture.play(&mut u, at);
        assert_eq!(screen_name(&u), want, "{trigger} {transition} {gesture:?}: {why}");
    }
}

/// A (presented) holding a 200×150 frame "Parent" at (50, 50) that goes to C on click, with a
/// 60×40 "Child" inside it at (20, 20) going to B on `child` (a trigger; None: no interaction),
/// optionally hidden, and optionally a "Cover" without interactions over the child. The screen a
/// click on the child's middle shows.
fn click_on_child(child: Option<Value>, hidden: bool, cover: bool) -> String {
    let mut s = Session::new();
    let a = frame(&mut s, "A", 0.0, 300.0, 400.0);
    let b = frame(&mut s, "B", 400.0, 300.0, 400.0);
    let c = frame(&mut s, "C", 800.0, 300.0, 400.0);
    let parent = id(&ex(&mut s, "frame.create", json!({"x": 50, "y": 50, "width": 200, "height": 150, "parent": a, "name": "Parent"})));
    let kid = rect(&mut s, "Child", parent, [20.0, 20.0, 60.0, 40.0]);
    on(&mut s, parent, json!({"type": "onClick"}), json!({"type": "navigate", "dest": c}), json!({"type": "instant"}), 0);
    if let Some(t) = child {
        on(&mut s, kid, t, json!({"type": "navigate", "dest": b}), json!({"type": "instant"}), 0);
    }
    if hidden {
        ex(&mut s, "node.set", json!({"id": kid, "props": {"visible": false}}));
    }
    if cover {
        rect(&mut s, "Cover", a, [60.0, 60.0, 100.0, 60.0]);
    }
    let mut u = Harness::new(s);
    u.run("proto.present", json!({"start": a}));
    // The child's middle: the parent's hotspot's top-left + (20 + 30, 20 + 20).
    let hot = presenter(&u)["hotspots"].as_array().cloned().unwrap_or_default();
    let p = hot.iter().find(|h| h["id"] == json!(parent)).unwrap();
    let at = pos2(p["rect"][0].as_f64().unwrap() as f32 + 50.0, p["rect"][1].as_f64().unwrap() as f32 + 40.0);
    Gesture::Click.play(&mut u, at);
    screen_name(&u)
}

#[test]
fn nested_hotspots_and_click_through() {
    // (child's trigger, child hidden, a cover on top, screen after a click on the child, derivation)
    let cases = [
        (Some(json!({"type": "onClick"})), false, false, "B", "the top-most layer with the trigger wins over its parent"),
        (Some(json!({"type": "keyPress", "key": "X"})), false, false, "C", "a layer without that trigger lets the click through to the parent's"),
        (None, false, false, "C", "a layer without interactions: the parent's"),
        (Some(json!({"type": "onClick"})), false, true, "B", "a layer without interactions over a hotspot doesn't block it (docs silent; pinned)"),
        (Some(json!({"type": "onClick"})), true, false, "C", "hidden layers aren't hotspots"),
    ];
    for (child, hidden, cover, want, why) in cases {
        assert_eq!(click_on_child(child.clone(), hidden, cover), want, "{child:?} hidden={hidden} cover={cover}: {why}");
    }
}

fn player(doc: Document, start: NodeId) -> Presenter {
    let mut p = Presenter::new(start);
    p.sync(&Arc::new(doc), 0.0);
    p
}

fn act(p: &mut Presenter, a: Action, now: f64) -> bool {
    let node = p.current;
    p.perform(&a, node, Transition::Instant, 0.0, Easing::Linear, None, now)
}

fn open(dest: NodeId, close_on_outside: bool) -> Action {
    Action::OpenOverlay { dest, position: OverlayPosition::Center, close_on_outside, background: None, offset: None, anchor: Default::default() }
}

/// Screens A and B and three overlay frames M1, M2, M3 (all top-level).
fn screens() -> (Document, [NodeId; 5]) {
    let mut s = Session::new();
    let ids = [("A", 0.0), ("B", 400.0), ("M1", 800.0), ("M2", 1000.0), ("M3", 1200.0)].map(|(n, x)| frame(&mut s, n, x, 150.0, 100.0));
    (s.doc().unwrap().clone(), ids)
}

#[test]
fn overlays_stack_swap_close_and_go_with_navigation() {
    let (doc, [a, b, m1, m2, m3]) = screens();
    let mut p = player(doc, a);
    let shown = |p: &Presenter| p.overlays.iter().map(|o| o.id).collect::<Vec<_>>();
    // (action, overlays after, screen after, derivation)
    let steps = [
        (open(m1, true), vec![m1], a, "open overlay: over the screen"),
        (open(m2, true), vec![m1, m2], a, "an overlay opened from an overlay stacks on it"),
        (Action::SwapOverlay { dest: m3 }, vec![m1, m3], a, "swap overlay replaces the top overlay"),
        (Action::Back, vec![m1], a, "back closes the top overlay first"),
        (Action::CloseOverlay, vec![], a, "close overlay closes it"),
        (open(m1, true), vec![m1], a, "(again)"),
        (
            Action::Navigate { dest: b, preserve_scroll: false },
            vec![],
            b,
            "navigating from an overlay replaces the screen under it and closes the overlays",
        ),
        (Action::Back, vec![], a, "back returns to the screen before"),
    ];
    for (k, (action, overlays, screen, why)) in steps.into_iter().enumerate() {
        assert!(act(&mut p, action.clone(), k as f64), "{action:?}: {why}");
        assert_eq!((shown(&p), p.current), (overlays, screen), "{action:?}: {why}");
    }
    assert!(!act(&mut p, Action::Back, 10.0), "nothing left to go back to");
    assert!(!act(&mut p, Action::CloseOverlay, 10.0), "no overlay to close");
}

#[test]
fn a_click_outside_an_overlay_closes_it_unless_turned_off() {
    // (close on click outside, overlays left after a click outside it, derivation)
    for (close, left, why) in [(true, 0, "closes on a click outside (the default)"), (false, 1, "turned off: stays open")] {
        let mut s = Session::new();
        let a = frame(&mut s, "A", 0.0, 300.0, 400.0);
        let menu = frame(&mut s, "Menu", 400.0, 100.0, 100.0);
        let hot = rect(&mut s, "Hot", a, [10.0, 10.0, 60.0, 40.0]);
        on(
            &mut s,
            hot,
            json!({"type": "onClick"}),
            json!({"type": "openOverlay", "dest": menu, "closeOnOutside": close}),
            json!({"type": "instant"}),
            0,
        );
        let mut u = Harness::new(s);
        u.run("proto.present", json!({"start": a}));
        let at = hotspot(&u, hot);
        Gesture::Click.play(&mut u, at);
        assert_eq!(presenter(&u)["overlays"], json!([menu]));
        // The overlay is centred on A; its top-left corner area is outside it.
        Gesture::Click.play(&mut u, at);
        assert_eq!(presenter(&u)["overlays"].as_array().map(Vec::len), Some(left), "{why}");
    }
}

#[test]
fn an_overlays_after_delay_counts_from_when_it_opens() {
    let mut s = Session::new();
    let a = frame(&mut s, "A", 0.0, 300.0, 400.0);
    let toast = frame(&mut s, "Toast", 400.0, 200.0, 50.0);
    on(&mut s, toast, json!({"type": "afterDelay", "ms": 800}), json!({"type": "closeOverlay"}), json!({"type": "instant"}), 0);
    let mut p = player(s.doc().unwrap().clone(), a);
    let due = |p: &Presenter, now: f64| next_delay(p, p.doc.as_ref().unwrap(), toast, now).map(|(k, d)| (k, (d * 1000.0).round() / 1000.0));
    // Opened 5 s after A arrived: the overlay is the screen shown, so its timer starts then.
    assert!(act(&mut p, open(toast, true), 5.0));
    let (key, d) = due(&p, 5.0).unwrap();
    assert_eq!(d, 0.8, "due 0.8 s after it opens, not at once");
    p.delays_fired.insert(key);
    assert!(act(&mut p, Action::CloseOverlay, 5.8));
    // Each time it opens its timer starts again.
    assert!(act(&mut p, open(toast, true), 10.0));
    assert_eq!(due(&p, 10.5).map(|x| x.1), Some(0.3));
}

#[test]
fn back_plays_the_way_in_backwards_and_restart_starts_over() {
    let (doc, [a, b, c, ..]) = screens();
    let mut p = player(doc, a);
    let go = |p: &mut Presenter, dest, tr, now| {
        let node = p.current;
        p.perform(&Action::Navigate { dest, preserve_scroll: false }, node, tr, 0.3, Easing::Linear, None, now)
    };
    assert!(go(&mut p, b, Transition::Push { direction: Direction::Left }, 1.0));
    assert!(go(&mut p, c, Transition::MoveIn { direction: Direction::Top }, 2.0));
    assert_eq!(p.history.iter().map(|v| v.frame).collect::<Vec<_>>(), [a, b], "history: the screens left, in order");
    // (screen after Back, animation it plays, derivation)
    for (screen, tr, why) in [
        (b, Transition::MoveOut { direction: Direction::Bottom }, "back from C: C moves out the way it came"),
        (a, Transition::Push { direction: Direction::Right }, "back from B: pushed back to the right"),
    ] {
        assert!(act(&mut p, Action::Back, 3.0));
        assert_eq!((p.current, p.anim.as_ref().map(|x| x.transition)), (screen, Some(tr)), "{why}");
    }
    go(&mut p, b, Transition::Instant, 4.0);
    p.restart(a, 5.0);
    assert!(p.history.is_empty() && p.current == a, "restart: the starting screen, no history");
}

#[test]
fn springs_play_until_they_settle() {
    let mut s = Session::new();
    let a = frame(&mut s, "A", 0.0, 300.0, 400.0);
    let b = frame(&mut s, "B", 400.0, 300.0, 400.0);
    let hot = rect(&mut s, "Hot", a, [10.0, 10.0, 60.0, 40.0]);
    // (easing, how long the animation plays (s), derivation)
    let cases = [
        (json!("easeOut"), 0.3, "a curve plays for the duration set"),
        (json!("bouncy"), Easing::Bouncy.seconds(300), "a spring plays until it settles, whatever the duration"),
        (json!({"customSpring": {"mass": 1, "stiffness": 100, "damping": 20}}), 0.92, "custom springs too (critically damped: ~9.2 / √(k/m) s)"),
    ];
    for (k, (easing, secs, why)) in cases.iter().enumerate() {
        ex(&mut s, "proto.interaction.add", json!({"id": hot, "dest": b, "transition": {"type": "dissolve"}, "durationMs": 300, "easing": easing}));
        let mut p = player(s.doc().unwrap().clone(), a);
        assert!(p.fire(hot, k, None, 1.0));
        let dur = p.anim.as_ref().map(|x| x.dur).unwrap_or_default();
        assert!((dur - secs).abs() < 0.02, "{easing}: {dur} s: {why}");
    }
}

#[test]
fn scroll_to_brings_the_layer_to_the_top_less_its_offset() {
    let mut s = Session::new();
    let a = frame(&mut s, "A", 0.0, 300.0, 600.0);
    ex(&mut s, "node.set", json!({"id": a, "props": {"overflow": "vertical"}}));
    rect(&mut s, "Long", a, [0.0, 0.0, 300.0, 2000.0]);
    let item = rect(&mut s, "Item", a, [0.0, 1200.0, 300.0, 40.0]);
    let doc = s.doc().unwrap().clone();
    // (offset, scroll after, derivation): A scrolls 2000 − 600 = 1400.
    let cases = [
        ([0.0, 0.0], 1200.0, "no offset: the layer at the top"),
        ([0.0, 60.0], 1140.0, "60 down from the top (under a 60 high fixed header)"),
        ([0.0, -50.0], 1250.0, "a negative offset scrolls past it"),
        ([0.0, 5000.0], 0.0, "never before the start"),
        ([0.0, -5000.0], 1400.0, "nor past the end"),
        ([f64::NAN, f64::INFINITY], 1200.0, "junk offsets count as none"),
    ];
    for (offset, want, why) in cases {
        let mut p = player(doc.clone(), a);
        assert!(act(&mut p, Action::ScrollTo { dest: item, offset }, 1.0));
        assert_eq!(p.scroll.get(&a).copied(), Some((0.0, want)), "{offset:?}: {why}");
    }
}

#[test]
fn fixed_layers_stay_above_the_content_that_scrolls() {
    let mut s = Session::new();
    let a = frame(&mut s, "A", 0.0, 300.0, 600.0);
    ex(&mut s, "node.set", json!({"id": a, "props": {"overflow": "vertical"}}));
    let header = rect(&mut s, "Header", a, [0.0, 0.0, 300.0, 60.0]);
    let feed = rect(&mut s, "Feed", a, [0.0, 0.0, 300.0, 2000.0]);
    let b = frame(&mut s, "B", 400.0, 300.0, 600.0);
    for layer in [header, feed] {
        on(&mut s, layer, json!({"type": "onClick"}), json!({"type": "navigate", "dest": b}), json!({"type": "instant"}), 0);
    }
    ex(&mut s, "node.set", json!({"id": header, "props": {"scrollPosition": "fixed"}}));
    let mut p = player(s.doc().unwrap().clone(), a);
    let shown = p.display().unwrap();
    // The header was below the feed in the layer list; fixed, it draws (and takes clicks) on top.
    assert_eq!(shown.node(a).unwrap().children, [feed, header], "fixed layers draw above scrolling ones");
    assert_eq!(figcraft_engine::proto_run::hotspots(&shown, a).last().map(|h| h.0), Some(header), "the top-most hotspot");
    assert_eq!(p.design.as_ref().unwrap().node(a).unwrap().children, [header, feed], "the file's layer order doesn't change");
    // Nothing fixed: nothing moves.
    let mut d = s.doc().unwrap().clone();
    d.get_mut(header).unwrap().scroll_position = Default::default();
    raise_fixed(&mut d);
    assert_eq!(d.node(a).unwrap().children, [header, feed]);
}

#[test]
fn smart_animate_tweens_matched_layers_and_overshoots_with_the_easing() {
    let mut s = Session::new();
    let a = frame(&mut s, "A", 0.0, 300.0, 400.0);
    let b = frame(&mut s, "B", 400.0, 300.0, 400.0);
    rect(&mut s, "Box", a, [0.0, 0.0, 20.0, 20.0]);
    let to = rect(&mut s, "Box", b, [100.0, 0.0, 20.0, 20.0]);
    let gone = rect(&mut s, "Dot", b, [0.0, 100.0, 10.0, 10.0]);
    ex(&mut s, "node.set", json!({"id": gone, "props": {"visible": false}}));
    let doc = s.doc().unwrap().clone();
    // (eased progress, x of Box in B, derivation): matched by name and hierarchy, from x 0 to 100.
    let cases = [
        (0.0, 0.0, "starts where the layer was"),
        (0.5, 50.0, "half way"),
        (1.1, 110.0, "a spring or back easing past 1 carries the layer past its end"),
        (-0.1, -10.0, "and before its start (ease in back)"),
        (1.0, 100.0, "ends exactly where it is in the destination"),
    ];
    for (t, x, why) in cases {
        let mid = smart_interpolate(&doc, a, &doc, b, t);
        let bx = mid.world_bounds(to).unwrap().x0 - mid.world_bounds(b).unwrap().x0;
        assert!((bx - x).abs() < 1e-6, "t={t}: {bx} ≠ {x}: {why}");
        // Visibility doesn't tween (docs silent; pinned: the destination's).
        assert!(!mid.node(gone).unwrap().visible);
    }
}

#[test]
fn interactive_components_change_in_place() {
    let mut s = Session::new();
    let mut variants = Vec::new();
    for (k, state) in ["Off", "On"].iter().enumerate() {
        let f =
            id(&ex(&mut s, "frame.create", json!({"x": k as f64 * 150.0, "y": 600, "width": 60, "height": 30, "name": format!("State={state}")})));
        rect(&mut s, "Knob", f, [if k == 0 { 0.0 } else { 30.0 }, 0.0, 30.0, 30.0]);
        variants.push(id(&ex(&mut s, "component.create", json!({"ids": [f]}))));
    }
    let set = id(&ex(&mut s, "component.combineVariants", json!({"ids": variants.clone()})));
    let (off, on_) = (variants[0], variants[1]);
    on(&mut s, off, json!({"type": "onClick"}), json!({"type": "changeTo", "dest": on_}), json!({"type": "smartAnimate"}), 200);
    on(&mut s, on_, json!({"type": "onClick"}), json!({"type": "changeTo", "dest": off}), json!({"type": "smartAnimate"}), 200);
    let screen = frame(&mut s, "Screen", 0.0, 300.0, 400.0);
    let toggle = id(&ex(&mut s, "instance.create", json!({"component": set, "x": 20, "y": 20, "parent": screen})));
    let mut p = player(s.doc().unwrap().clone(), screen);
    let shows = |p: &Presenter| match p.doc.as_ref().and_then(|d| d.node(toggle)).map(|n| &n.kind) {
        Some(NodeKind::Instance { component, .. }) => Some(*component),
        _ => None,
    };
    // The instance plays its variant's click (it has none of its own) and changes in place, with
    // the variant's animation; the next click plays the other variant's.
    for (k, want, why) in [(1.0, on_, "Off's click: changes to On"), (2.0, off, "On's click: back to Off")] {
        assert!(p.fire(toggle, 0, None, k), "{why}");
        assert_eq!((shows(&p), p.current), (Some(want), screen), "{why}");
        assert_eq!(p.anim.as_ref().map(|x| x.transition), Some(Transition::SmartAnimate), "{why}");
    }
}

/// A hugging auto layout frame (in `parent`, else on the page) holding text `chars`: (frame, text).
fn hugging_label(s: &mut Session, parent: Option<NodeId>, name: &str, x: f64, chars: &str) -> (NodeId, NodeId) {
    let f = id(&ex(s, "frame.create", json!({"x": x, "y": 600, "width": 60, "height": 30, "name": name, "parent": parent})));
    let t = id(&ex(s, "text.create", json!({"text": chars, "x": 0, "y": 0, "parent": f})));
    ex(s, "node.set", json!({"ids": [f], "props": {"layout": "horizontal", "padding": 8, "sizingH": "hug", "sizingV": "hug"}}));
    (f, t)
}

fn size(d: &Document, n: NodeId) -> (f64, f64) {
    d.node(n).map(|n| (n.width, n.height)).unwrap()
}

#[test]
fn change_to_lays_out_the_instance_around_its_own_content() {
    // Change to shows the other variant in place: the instance keeps its overrides (a longer
    // label), so it is laid out around them, as switching the variant on the canvas does.
    figcraft_text::scan_system_fonts();
    let mut s = Session::new();
    let variants: Vec<NodeId> = ["Default", "Hover"]
        .iter()
        .enumerate()
        .map(|(k, state)| {
            let (f, _) = hugging_label(&mut s, None, &format!("State={state}"), k as f64 * 150.0, "Button");
            id(&ex(&mut s, "component.create", json!({"ids": [f]})))
        })
        .collect();
    let set = id(&ex(&mut s, "component.combineVariants", json!({"ids": variants.clone()})));
    on(&mut s, variants[0], json!({"type": "onClick"}), json!({"type": "changeTo", "dest": variants[1]}), json!({"type": "instant"}), 0);
    let screen = frame(&mut s, "Screen", 0.0, 400.0, 300.0);
    let button = id(&ex(&mut s, "instance.create", json!({"component": set, "x": 20, "y": 20, "parent": screen})));
    let label = s.doc().unwrap().node(button).unwrap().children[0];
    ex(&mut s, "text.set", json!({"id": label, "characters": "A much longer label"}));
    let (main, canvas, canvas_label) = {
        let d = s.doc().unwrap();
        (size(d, variants[1]), size(d, button), size(d, label))
    };
    // Without any font nothing is measured and both sizes are the main's.
    if figcraft_text::layout_node(s.doc().unwrap().node(label).unwrap()).is_some_and(|(l, _)| !l.glyphs.is_empty()) {
        assert!(canvas.0 > main.0 + 20.0, "on the canvas the instance hugs its longer label");
    }
    let mut p = player(s.doc().unwrap().clone(), screen);
    assert!(p.fire(button, 0, None, 1.0));
    let d = p.doc.as_ref().unwrap();
    assert!(matches!(d.node(button).unwrap().kind, NodeKind::Instance { component, .. } if component == variants[1]));
    assert_eq!((size(d, button), size(d, label)), (canvas, canvas_label), "the Hover variant, laid out around the instance's label");
}

/// A screen holding a hugging chip whose text is bound to string variable `label` of collection
/// "Copy" (modes A "Short" and B "A much longer label"), the chip's click running `action`:
/// (session, screen, chip, text).
fn bound_chip(action: Value) -> (Session, NodeId, NodeId, NodeId) {
    figcraft_text::scan_system_fonts();
    let mut s = Session::new();
    ex(&mut s, "variable.createCollection", json!({"name": "Copy", "modes": ["A", "B"]}));
    ex(
        &mut s,
        "variable.create",
        json!({"collection": "Copy", "name": "label", "type": "string", "values": {"A": "Short", "B": "A much longer label"}}),
    );
    let screen = frame(&mut s, "Screen", 0.0, 400.0, 300.0);
    let (chip, text) = hugging_label(&mut s, Some(screen), "Chip", 20.0, "Short");
    ex(&mut s, "variable.bind", json!({"ids": [text], "field": "characters", "variable": "label"}));
    on(&mut s, chip, json!({"type": "onClick"}), action, json!({"type": "instant"}), 0);
    (s, screen, chip, text)
}

#[test]
fn set_variable_and_set_mode_lay_out_the_text_they_change() {
    // A text bound to a string variable shows what an interaction sets, measured: the hugging
    // chip around it grows with it, to the size the canvas gives the same text.
    let cases = [
        (
            json!({"type": "setVariable", "variable": "label", "value": "A much longer label"}),
            "variable.set",
            json!({"variable": "label", "value": "A much longer label"}),
        ),
        (json!({"type": "setVariableMode", "collection": "Copy", "mode": "B"}), "variable.setMode", json!({"collection": "Copy", "mode": "B"})),
    ];
    for (action, canvas_cmd, canvas_params) in cases {
        let (mut s, screen, chip, text) = bound_chip(action.clone());
        let mut p = player(s.doc().unwrap().clone(), screen);
        assert!(p.fire(chip, 0, None, 1.0), "{action}");
        let page = s.doc().unwrap().page_of(screen).unwrap();
        let mut params = canvas_params;
        if canvas_cmd == "variable.setMode" {
            params["ids"] = json!([page]);
        }
        ex(&mut s, canvas_cmd, params);
        let (d, canvas) = (p.doc.as_ref().unwrap(), s.doc().unwrap());
        assert_eq!((size(d, chip), size(d, text)), (size(canvas, chip), size(canvas, text)), "{action}: the chip grows with its text");
    }
}

#[test]
fn a_drag_started_transition_follows_the_pointer() {
    let (mut u, hot) = one_interaction(json!({"type": "onDrag"}), json!({"type": "push", "direction": "left"}), 400);
    let at = hotspot(&u, hot);
    let progress = |u: &Harness| {
        let a = u.app.ui.present.as_ref().and_then(|p| p.anim.as_ref())?;
        Some(((u.time - a.start) / a.dur * 100.0).round() / 100.0)
    };
    u.frame(vec![Event::PointerMoved(at)]);
    u.frame(vec![button(at, true, Modifiers::NONE)]);
    // 30 points to the left over a 300 wide screen: a tenth of the way, for as long as it's held.
    for k in 1..=3 {
        u.frame(vec![Event::PointerMoved(at - vec2(10.0 * k as f32, 0.0))]);
    }
    assert_eq!(screen_name(&u), "B");
    assert_eq!(progress(&u), Some(0.1));
    u.frames(5, Vec::new);
    assert_eq!(progress(&u), Some(0.1), "held still: it waits");
    u.frame(vec![Event::PointerMoved(at - vec2(150.0, 0.0))]);
    assert_eq!(progress(&u), Some(0.5), "half the screen: half way");
    // Let go, it plays on from there to the end.
    u.frame(vec![button(at - vec2(150.0, 0.0), false, Modifiers::NONE)]);
    u.frames(30, Vec::new);
    assert_eq!(progress(&u), None, "finished");
}
