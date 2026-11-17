//! The main menu and its submenus stay inside the window (#2217): a submenu that would run past
//! the bottom moves up to fit, and one taller than the window scrolls.

use egui::{Event, Id, MouseWheelUnit, Rect, TouchPhase, pos2, vec2};
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;
use crate::live_tests::{flat, text_center};
use crate::menus::{HOVER_DELAY, main_menu_id};
use crate::present_tests::id;

/// A window `h` tall with the main menu open and the Object submenu showing.
fn object_menu_open(s: Session, h: f32) -> Ui {
    let mut u = Ui::new(s);
    u.screen = vec2(1280.0, h);
    u.app.ui.main_menu = true;
    u.frame(vec![Event::PointerMoved(pos2(1000.0, 10.0))]);
    u.frame(vec![]);
    let m = u.ctx.memory(|m| m.area_rect(main_menu_id())).expect("main menu drawn");
    // Frame margin, the Actions row and the space under it, then three rows down to Object.
    let y = m.min.y + 8.0 + 30.0 + 6.0 + 28.0 * 3.0 + 14.0;
    u.frame(vec![Event::PointerMoved(pos2(m.min.x + 60.0, y))]);
    for _ in 0..((HOVER_DELAY + 0.3) * 60.0) as usize {
        u.frame(vec![]);
    }
    u
}

fn object_panel(u: &Ui) -> Rect {
    u.ctx.memory(|m| m.area_rect(Id::unique(("menu", "Object")))).expect("the Object submenu is drawn")
}

#[test]
fn the_object_submenu_stays_inside_a_short_window() {
    let u = object_menu_open(Session::new(), 600.0);
    let panel = object_panel(&u);
    let window = Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 600.0));
    assert!(window.contains_rect(panel), "the Object submenu {panel:?} runs outside the window {window:?}");
}

#[test]
fn a_menu_taller_than_the_window_scrolls_to_its_last_command() {
    let mut s = Session::new();
    let a = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 40, "height": 30})).unwrap());
    let b = id(&s.execute("shape.rectangle", &json!({"x": 100, "y": 30, "width": 20, "height": 20})).unwrap());
    s.execute("select.set", &json!({"ids": [b]})).unwrap();
    let mut u = object_menu_open(s, 420.0);
    let panel = object_panel(&u);
    assert!(Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 420.0)).contains_rect(panel), "{panel:?} outside the window");
    // Turn the wheel over the panel until the end of the list is in view.
    for _ in 0..4 {
        let wheel = Event::MouseWheel {
            unit: MouseWheelUnit::Point,
            source: egui::MouseWheelSource::Unknown,
            delta: vec2(0.0, -1500.0),
            phase: TouchPhase::Move,
            modifiers: Default::default(),
        };
        u.frame(vec![Event::PointerMoved(panel.center()), wheel]);
        for _ in 0..20 {
            u.frame(vec![]);
        }
    }
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let at = text_center(&shapes, "Move to page");
    assert!(panel.contains(at), "the last commands are out of reach: at {at:?} outside {panel:?}");
    // And a command above it, reached by scrolling back up, runs on click.
    let wheel = Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        source: egui::MouseWheelSource::Unknown,
        delta: vec2(0.0, 1.0e5),
        phase: TouchPhase::Move,
        modifiers: Default::default(),
    };
    u.frame(vec![Event::PointerMoved(panel.center()), wheel]);
    for _ in 0..20 {
        u.frame(vec![]);
    }
    // Scroll down in small steps until "Send to back" shows inside the panel, then click it.
    let mut hit = None;
    for _ in 0..40 {
        let shapes = flat(&u.frame_output(vec![]).shapes);
        let at = text_center(&shapes, "Send to back");
        if panel.contains(at) {
            hit = Some(at);
            break;
        }
        let wheel = Event::MouseWheel {
            unit: MouseWheelUnit::Point,
            source: egui::MouseWheelSource::Unknown,
            delta: vec2(0.0, -120.0),
            phase: TouchPhase::Move,
            modifiers: Default::default(),
        };
        u.frame(vec![Event::PointerMoved(panel.center()), wheel]);
        for _ in 0..10 {
            u.frame(vec![]);
        }
    }
    let at = hit.expect("Send to back scrolls into view");
    crate::component_swap_default_tests::click(&mut u, at);
    let doc = u.app.session.doc().unwrap();
    let page = u.app.session.page().unwrap();
    let kids = &doc.node(page).unwrap().children;
    assert_eq!((kids.first().copied(), kids.last().copied()), (Some(b), Some(a)), "Send to back ran");
}
