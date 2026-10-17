//! The presenter's scale options, device rotation and scroll momentum, driven headless through
//! egui (the harness is `present_tests`'), and the motion maths on its own.

use egui::{Event, Key, Pos2, pos2, vec2};
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::present::{PresentScale, fling_axis, rubber_band};
use crate::present_tests::{Ui, id, setup};

/// Run a fling for `secs` at `fps` frames a second; the offset where it ends and whether it rests.
fn fling_for(x: f64, v: f64, r: f64, decay: f64, bounce: bool, secs: f64, fps: f64) -> (f64, bool) {
    let (mut x, mut v, mut done) = (x, v, false);
    let dt = 1.0 / fps;
    for _ in 0..(secs * fps).round() as usize {
        (x, v, done) = fling_axis(x, v, r, dt, decay, bounce);
        if done {
            break;
        }
    }
    (x, done)
}

#[test]
fn flings_slow_down_the_same_at_any_frame_rate() {
    // It travels speed / decay, coming to rest.
    let (x, done) = fling_for(0.0, 1000.0, 10_000.0, 2.5, true, 5.0, 60.0);
    assert!(done && (x - 400.0).abs() < 3.0, "{x}");
    // 30 and 144 frames a second end up in the same place.
    let (a, _) = fling_for(100.0, 1500.0, 10_000.0, 2.5, true, 0.5, 30.0);
    let (b, _) = fling_for(100.0, 1500.0, 10_000.0, 2.5, true, 0.5, 144.0);
    assert!((a - b).abs() < 0.5, "{a} vs {b}");
    // Without bounce it stops at the end; with bounce it overshoots and springs back exactly.
    assert_eq!(fling_for(900.0, 3000.0, 1000.0, 8.0, false, 2.0, 60.0), (1000.0, true));
    let mut x = 900.0;
    let mut v = 3000.0;
    let mut furthest: f64 = 0.0;
    for _ in 0..240 {
        let (nx, nv, done) = fling_axis(x, v, 1000.0, 1.0 / 60.0, 2.5, true);
        (x, v) = (nx, nv);
        furthest = furthest.max(x);
        if done {
            break;
        }
    }
    assert!(furthest > 1000.0 && furthest < 1200.0, "{furthest}");
    assert_eq!((x, v), (1000.0, 0.0));
    // From past the start it comes back to 0.
    assert_eq!(fling_for(-80.0, 0.0, 1000.0, 2.5, true, 3.0, 60.0), (0.0, true));
    // Junk never panics and stays finite, in range.
    for (x, v, r, dt) in [
        (f64::NAN, 1.0, 10.0, 0.1),
        (1.0, f64::INFINITY, 10.0, 0.1),
        (1.0, 1.0, f64::NAN, 0.1),
        (1.0, 1.0, 10.0, f64::NAN),
        (5.0, 1e300, 10.0, 1e9),
        (5.0, 1.0, -3.0, 0.1),
    ] {
        let (nx, nv, _) = fling_axis(x, v, r, dt, 2.5, true);
        assert!(nx.is_finite() && nv.is_finite(), "{x} {v} {r} {dt} → {nx} {nv}");
    }
}

#[test]
fn rubber_band_stretches_less_the_further_it_goes() {
    assert_eq!(rubber_band(50.0, 100.0, 240.0), 50.0);
    let a = rubber_band(130.0, 100.0, 240.0);
    let b = rubber_band(400.0, 100.0, 240.0);
    let far = rubber_band(1e9, 100.0, 240.0);
    assert!(100.0 < a && a < 130.0 && a < b && b < far && far <= 340.0, "{a} {b} {far}");
    assert!(rubber_band(-30.0, 100.0, 240.0) < 0.0 && rubber_band(-30.0, 100.0, 240.0) > -30.0);
    // Nothing to scroll, or junk: no stretch.
    assert_eq!(rubber_band(-30.0, 0.0, 240.0), 0.0);
    assert_eq!(rubber_band(f64::NAN, 100.0, 240.0), 0.0);
    assert_eq!(rubber_band(130.0, 100.0, f64::NAN), 100.0);
}

#[test]
fn scale_options_match_the_command_and_cycle() {
    for s in figcraft_engine::cmd::proto::PRESENT_SCALES {
        assert_eq!(PresentScale::parse(s).map(PresentScale::id), Some(s));
    }
    assert_eq!(PresentScale::parse("huge"), None);
    // Without a device Z goes fit → fit width → fill → 100% → fit; on one, fit width is skipped.
    let mut s = PresentScale::Fit;
    let mut seen = vec![];
    for _ in 0..4 {
        s = s.next(false);
        seen.push(s);
    }
    assert_eq!(seen, [PresentScale::FitWidth, PresentScale::Fill, PresentScale::Actual, PresentScale::Fit]);
    assert_eq!(PresentScale::Fit.next(true), PresentScale::Fill);
    assert_eq!(PresentScale::FitWidth.next(true), PresentScale::Fit, "not offered on a device: back to the first");
    assert!(!PresentScale::options(true).contains(&PresentScale::FitWidth));
}

#[test]
fn presentation_tab_keeps_the_current_screen_when_returning_to_the_editor() {
    let (mut u, a, b, _) = setup();
    u.run("proto.present", json!({"start": a}));
    u.key(pos2(720.0, 470.0), Key::ArrowRight);
    assert_eq!(presenter(&u)["current"], json!(b));

    let design_tab = pos2(100.0, 20.0);
    u.button(design_tab, true);
    u.button(design_tab, false);
    assert!(u.app.ui.present.is_none());
    assert_eq!(u.app.ui.paused_present.as_ref().map(|p| p.current), Some(b));

    let name_len = u.app.session.doc().unwrap().name.chars().count() as f32;
    let design_w = (12.0 * 0.6 * name_len + 64.0).clamp(120.0, 220.0);
    let preview_tab = pos2(8.0 + 44.0 + design_w + 40.0, 20.0);
    u.button(preview_tab, true);
    u.button(preview_tab, false);
    assert!(u.app.ui.paused_present.is_none());
    assert_eq!(presenter(&u)["current"], json!(b));
}

/// No device: a big screen (2000×3000) and a small one, presenting the big one.
fn big() -> (Ui, NodeId) {
    let mut s = Session::new();
    let big = id(&s.execute("node.create", &json!({"type": "frame", "name": "Big", "x": 0, "y": 0, "width": 2000, "height": 3000})).unwrap());
    s.execute("node.create", &json!({"type": "frame", "name": "Small", "x": 2100, "y": 0, "width": 300, "height": 200})).unwrap();
    let mut u = Ui::new(s);
    u.run("proto.present", json!({"start": big}));
    (u, big)
}

fn presenter(u: &Ui) -> Value {
    u.app.inspect()["presenter"].clone()
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap()
}

#[test]
fn presenter_scales_by_its_options_and_pans_what_overflows() {
    let (mut u, big) = big();
    let at = pos2(720.0, 470.0);
    // Fit: the tab strip leaves 1376×756 inside the presenter's padding; nothing pans.
    let p = presenter(&u);
    assert_eq!(p["scale"], "fit");
    assert!((num(&p["zoom"]) - 756.0 / 3000.0).abs() < 1e-3, "{p}");
    assert_eq!(p["panRange"], json!([0.0, 0.0]));
    // Z: fit width (the whole 1440 width; the rest of the height pans).
    u.key(at, Key::Z);
    let p = presenter(&u);
    assert_eq!(p["scale"], "fitWidth");
    assert!((num(&p["zoom"]) - 0.72).abs() < 1e-3, "{p}");
    assert_eq!(p["panRange"][0], 0.0);
    assert!((num(&p["panRange"][1]) - (2160.0 - 820.0)).abs() < 1.0, "{p}");
    // Z: fill (here the width decides too), then 100%: both ways overflow, centred across at the top.
    u.key(at, Key::Z);
    assert_eq!(presenter(&u)["scale"], "fill");
    u.key(at, Key::Z);
    let p = presenter(&u);
    assert_eq!((p["scale"].as_str(), num(&p["zoom"])), (Some("actual"), 1.0));
    assert_eq!(p["panRange"], json!([624.0, 2244.0]));
    assert_eq!(p["pan"], json!([312.0, 0.0]));
    // The wheel pans down; a drag pans with the pointer.
    let wheel = move || {
        vec![
            Event::PointerMoved(at),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, -80.0),
                phase: egui::TouchPhase::Move,
                modifiers: Default::default(),
            },
        ]
    };
    u.frames(4, wheel);
    u.frames(60, Vec::new);
    let y = num(&presenter(&u)["pan"][1]);
    assert!(y > 300.0, "{y}");
    u.button(at, true);
    for k in 1..=5 {
        u.frame(vec![Event::PointerMoved(at - vec2(0.0, 20.0 * k as f32))]);
    }
    u.button(at - vec2(0.0, 100.0), false);
    assert!((num(&presenter(&u)["pan"][1]) - (y + 100.0)).abs() < 1.0, "{}", presenter(&u));
    // The command sets the options too; they carry over to the next presentation.
    u.run("proto.presentOptions", json!({"scale": "fit"}));
    assert_eq!(presenter(&u)["panRange"], json!([0.0, 0.0]));
    u.run("proto.presentOptions", json!({"scale": "fill"}));
    u.key(at, Key::Escape);
    assert!(u.app.ui.present.is_none());
    assert_eq!(u.app.inspect()["presentOptions"]["scale"], "fill");
    u.run("proto.present", json!({"start": big}));
    assert_eq!(presenter(&u)["scale"], "fill");
    // R restarts.
    u.key(at, Key::ArrowRight);
    assert_eq!(presenter(&u)["name"], "Small");
    u.key(at, Key::R);
    assert_eq!(presenter(&u)["name"], "Big");
}

#[test]
fn devices_turn_and_their_frame_can_hide() {
    let (mut u, a, ..) = setup();
    u.run("proto.settings", json!({"rotation": "landscape"}));
    u.run("proto.preview", json!({"start": a}));
    let p = u.preview();
    assert_eq!((p["device"].as_str(), &p["landscape"]), (Some("iPhone 16"), &json!(true)));
    // The 393-wide screen shows 393 of the frame's height on the 852×393 screen; it scrolls the rest.
    assert_eq!(p["viewport"], json!([393.0, 393.0]));
    assert_eq!(p["scrollRange"], json!([0.0, 1607.0]));
    u.run("proto.closePreview", json!({}));
    // Full window, upright: hiding the device's body leaves more room for the screen.
    u.run("proto.settings", json!({"rotation": "portrait"}));
    u.run("proto.present", json!({"start": a}));
    let with_body = num(&presenter(&u)["zoom"]);
    u.run("proto.presentOptions", json!({"deviceFrame": false}));
    let bare = num(&presenter(&u)["zoom"]);
    assert!(bare > with_body && (bare - 756.0 / 852.0).abs() < 1e-3, "{with_body} {bare}");
    // A device offers no fit width: Z goes fit → fill → 100%.
    u.key(pos2(720.0, 470.0), Key::Z);
    assert_eq!(presenter(&u)["scale"], "fill");
}

/// Drag the pane's screen by `dy` per frame for `n` frames, then let go.
fn flick(u: &mut Ui, from: Pos2, dy: f32, n: usize) {
    u.frame(vec![Event::PointerMoved(from)]);
    u.button(from, true);
    for k in 1..=n {
        u.frame(vec![Event::PointerMoved(from + vec2(0.0, dy * k as f32))]);
    }
    u.button(from + vec2(0.0, dy * n as f32), false);
}

#[test]
fn scrolling_keeps_its_momentum_and_springs_back_from_the_ends() {
    let (mut u, a, ..) = setup();
    u.run("proto.preview", json!({"start": a}));
    let c = u.pane().center();
    // A quick flick up carries on after letting go, slowing down to rest.
    flick(&mut u, c + vec2(0.0, 100.0), -25.0, 6);
    let released = u.scroll_y();
    assert!(u.preview()["flinging"] == json!(true), "{}", u.preview());
    u.frames(20, Vec::new);
    let coasted = u.scroll_y();
    assert!(coasted > released + 20.0, "{released} → {coasted}");
    u.frames(400, Vec::new);
    assert_eq!(u.preview()["flinging"], json!(false));
    let rest = u.scroll_y();
    assert!(rest >= coasted && rest <= 1148.0, "{rest}");
    // A press catches a fling.
    flick(&mut u, c + vec2(0.0, 100.0), 25.0, 6);
    u.frames(2, Vec::new);
    u.button(c, true);
    let caught = u.scroll_y();
    u.frames(10, Vec::new);
    assert_eq!(u.scroll_y(), caught);
    u.button(c, false);
    // Dragged past the top it stretches while held, and springs back to it when let go.
    crate::control::handle(&mut u.app, &u.ctx, "ui.set", &json!({"presentScroll": [0, 0]})).unwrap();
    u.button(c, true);
    for k in 1..=10 {
        u.frame(vec![Event::PointerMoved(c + vec2(0.0, 12.0 * k as f32))]);
    }
    let stretched = u.scroll_y();
    assert!(stretched < 0.0, "{stretched}");
    for _ in 0..10 {
        u.frame(vec![Event::PointerMoved(c + vec2(0.0, 120.0))]);
    }
    u.button(c + vec2(0.0, 120.0), false);
    u.frames(90, Vec::new);
    assert_eq!(u.scroll_y(), 0.0);
    assert_eq!(u.preview()["flinging"], json!(false));
}

#[test]
fn the_wheel_coasts_after_it_stops() {
    let (mut u, a, ..) = setup();
    u.run("proto.preview", json!({"start": a}));
    let c = u.pane().center();
    let wheel = move || {
        vec![
            Event::PointerMoved(c),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, -40.0),
                phase: egui::TouchPhase::Move,
                modifiers: Default::default(),
            },
        ]
    };
    u.frames(8, wheel);
    let mut coasted = false;
    for _ in 0..60 {
        u.frame(vec![Event::PointerMoved(c)]);
        coasted |= u.preview()["flinging"] == json!(true);
    }
    assert!(coasted, "the wheel's motion carried on");
    u.frames(200, Vec::new);
    assert_eq!(u.preview()["flinging"], json!(false), "and came to rest");
}

#[test]
fn design_panel_picks_variable_modes_for_frames() {
    let (mut u, a, _, card) = setup();
    let ex = |u: &mut Ui, c: &str, p: Value| u.run(c, p);
    ex(&mut u, "variable.createCollection", json!({"name": "Theme", "modes": ["Light", "Dark"]}));
    ex(&mut u, "variable.createCollection", json!({"name": "A collection with a very long name indeed", "modes": ["One", "Two"]}));
    ex(&mut u, "variable.createCollection", json!({"name": "Single", "modes": ["Only"]}));
    // A frame selected (Auto), then with a mode of its own, then mixed with another frame; a
    // rectangle has no modes section. Each draws without trouble.
    for (sel, mode) in [(vec![a], None), (vec![a], Some("Dark")), (vec![a, card], None)] {
        if let Some(m) = mode {
            ex(&mut u, "variable.setMode", json!({"ids": [a], "collection": "Theme", "mode": m}));
        }
        ex(&mut u, "select.set", json!({"ids": sel}));
        u.frames(2, Vec::new);
    }
    assert_eq!(u.app.session.doc().unwrap().node(a).unwrap().var_modes.len(), 1);
}
