//! The interface size, driven through egui headless on screens of different system scales: 100 %
//! leaves the system's scale alone, another size is relative to it on every screen, and the
//! Preferences drop-down tells a screen reader its name and value.
//!
//! The test window's size is fixed in points, so it grows with the zoom here; the limit for a small
//! window is tested on its own in `ui_scale::tests`, and the sizes below stay inside it.

use egui::accesskit::Role;
use egui::{RawInput, ViewportId, ViewportInfo};
use figcraft_engine::Session;
use serde_json::json;

use crate::layers_drag_tests::Ui;

/// One frame on a screen whose system scale is `native` (a real window reports it every frame).
fn frame_at(u: &mut Ui, native: f32) -> egui::FullOutput {
    let mut raw = RawInput::default();
    raw.viewports.insert(ViewportId::ROOT, ViewportInfo { native_pixels_per_point: Some(native), ..Default::default() });
    u.frame_raw(raw)
}

/// Frames on that screen, enough for a new zoom to take effect.
fn settle(u: &mut Ui, native: f32) {
    for _ in 0..4 {
        frame_at(u, native);
    }
}

const SCREENS: [f32; 5] = [1.0, 1.25, 1.5, 2.0, 3.0];

#[test]
fn by_default_the_interface_is_the_systems_own_size() {
    for native in SCREENS {
        let mut u = Ui::new(Session::new());
        settle(&mut u, native);
        assert_eq!(u.ctx.zoom_factor(), 1.0, "{native}");
        assert_eq!(u.ctx.pixels_per_point(), native, "{native}");
    }
}

#[test]
fn a_size_is_relative_to_the_systems_scale_on_every_screen_and_auto_is_100() {
    for native in SCREENS {
        let mut u = Ui::new(Session::new());
        for size in [0.75_f64, 0.9, 1.25, 1.5] {
            assert!(u.app.run("pref.uiScale", json!({"scale": size})).is_some());
            settle(&mut u, native);
            let drawn = u.ctx.pixels_per_point();
            assert!((drawn - size as f32 * native).abs() < 1e-4, "{size} on a {native}× screen drew at {drawn}");
            assert!((u.ctx.zoom_factor() - size as f32).abs() < 1e-5, "{size} on {native}");
            assert!(!u.app.ui.ui_scale.limited, "{size} on {native}");
        }
        assert!(u.app.run("pref.uiScale", json!({"scale": "auto"})).is_some());
        settle(&mut u, native);
        assert_eq!((u.ctx.zoom_factor(), u.ctx.pixels_per_point()), (1.0, native), "{native}");
    }
}

#[test]
fn the_interface_size_menu_has_a_name_a_value_and_a_state_for_a_screen_reader() {
    let mut u = Ui::new(Session::new());
    u.app.ui.preferences = true;
    u.ctx.enable_accesskit();
    let combo = |u: &mut Ui| {
        settle(u, 2.0);
        let out = frame_at(u, 2.0);
        let nodes: Vec<_> = out.platform_output.accesskit_update.expect("an AccessKit update").nodes.into_iter().map(|(_, n)| n).collect();
        let n = nodes.iter().find(|n| n.role() == Role::ComboBox && n.label() == Some("Interface size")).expect("an Interface size menu");
        (n.value().map(str::to_owned), n.is_expanded())
    };
    // 100 % is the system's own size, whatever it is (2× here).
    assert_eq!(combo(&mut u), (Some("100%".to_owned()), Some(false)));
    u.app.run("pref.uiScale", json!({"scale": 1.25}));
    assert_eq!(combo(&mut u), (Some("125%".to_owned()), Some(false)));
}

/// Every window size, system scale and interface size, drawn the way a real window is: the window is
/// a fixed number of physical pixels, and its size in points is that divided by the scale drawn now
/// (egui-winit does the same each frame), so a bigger size shrinks the window in points.
///
/// What a fixed size promises: whatever it draws at above 100 %, the window still holds the smallest
/// layout the interface is made for (`ui_scale::FIT_POINTS`), and the panels leave room for a canvas.
#[test]
fn every_window_size_system_scale_and_interface_size_lays_out_inside_the_window() {
    use crate::ui_scale::{FIT_POINTS, SIZES};
    use crate::{FigApp, NoServices};
    use egui::{Pos2, Rect, vec2};

    // Window sizes in points: the main window's minimum, common laptop and desktop screens up to 5K,
    // and a short wide strip.
    let windows: [(f32, f32); 10] = [
        (800.0, 500.0),
        (1024.0, 640.0),
        (1280.0, 720.0),
        (1366.0, 768.0),
        (1440.0, 900.0),
        (1920.0, 1080.0),
        (2560.0, 1440.0),
        (3840.0, 2160.0),
        (5120.0, 2880.0),
        (2560.0, 520.0),
    ];

    for native in [1.0_f32, 1.25, 1.5, 1.75, 2.0, 3.0] {
        for (w, h) in windows {
            let px = vec2(w * native, h * native);
            for want in SIZES {
                let case = format!("{w}x{h} pt at {native}x, {want}");
                let mut app = FigApp::new(Session::new(), Box::new(NoServices));
                assert!(app.run("pref.uiScale", json!({"scale": want})).is_some(), "{case}");
                let ctx = egui::Context::default();
                let mut time = 0.0;
                let mut seen = Vec::new();
                for _ in 0..8 {
                    time += 1.0 / 60.0;
                    let ppp = ctx.pixels_per_point();
                    let mut raw = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, px / ppp)), time: Some(time), ..Default::default() };
                    raw.viewports.insert(ViewportId::ROOT, ViewportInfo { native_pixels_per_point: Some(native), ..Default::default() });
                    let mut out = ctx.run_ui(raw, |ui| app.ui(ui));
                    out.textures_delta.clear();
                    seen.push((ctx.pixels_per_point(), ctx.content_rect().size()));
                }
                // It settles within a few frames and then stays put: no flicker between two sizes.
                let (ppp, content) = seen[seen.len() - 1];
                assert_eq!(seen[seen.len() - 2].0, ppp, "{case}: still changing");
                assert!(ppp.is_finite() && ppp > 0.0 && content.x.is_finite() && content.y.is_finite(), "{case}: {ppp} {content:?}");
                let plan = app.ui.ui_scale;
                assert!((ppp - plan.drawn).abs() < 1e-3, "{case}: egui draws at {ppp}, the plan said {}", plan.drawn);
                // The size drawn, relative to the system's scale.
                let zoom = ppp / native;
                let want = want as f32;
                assert_eq!(plan.limited, zoom < want - 0.005, "{case}: limited flag");
                // Above 100 % the window holds the smallest layout; at or below it there is nothing more to give up.
                if zoom > 1.001 {
                    assert!(content.x >= FIT_POINTS.0 - 0.5 && content.y >= FIT_POINTS.1 - 0.5, "{case}: window is {content:?} points at {ppp}");
                }
                // It never draws bigger than asked, and only a window too small for it draws smaller.
                assert!(zoom <= want + 1e-3, "{case}");
                if !plan.limited {
                    assert!((zoom - want).abs() < 1e-3, "{case}");
                }
                let panels = &app.inspect()["panels"];
                let side = panels["left"].as_f64().unwrap_or(0.0) + panels["right"].as_f64().unwrap_or(0.0);
                assert!(side < f64::from(content.x) - 100.0, "{case}: the panels ({side} pt) leave no canvas in {content:?}");
            }
        }
    }
}

/// The interface size is reachable and changeable without a pointer: Tab reaches the menu, Enter
/// opens it, the arrows and Tab move through its entries, Enter picks one and closes the menu, and
/// Escape closes it alone, without changing anything and without closing Preferences (a second
/// Escape does).
#[test]
fn the_interface_size_menu_works_from_the_keyboard() {
    use crate::a11y::focused_name;
    use egui::{Event, Key, Modifiers, Popup};

    // A key press, then two quiet frames for the focus (and the name a screen reader reads) to settle.
    let press_mod = |u: &mut Ui, key: Key, modifiers: Modifiers| {
        u.frame_with(
            vec![
                Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers },
                Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers },
            ],
            modifiers,
        );
        u.frame(vec![]);
        u.frame(vec![]);
    };
    let press = |u: &mut Ui, key: Key| press_mod(u, key, Modifiers::NONE);
    let open = |u: &mut Ui| {
        // The Preferences dialog is not modal for the keyboard: Tab crosses the main window first.
        let mut tabs = 0;
        while focused_name(&u.ctx) != "Interface size" {
            assert!(tabs < 140, "Tab never reaches the Interface size menu (focus: {:?})", focused_name(&u.ctx));
            press(u, Key::Tab);
            tabs += 1;
        }
        press(u, Key::Enter);
        assert!(Popup::is_any_open(&u.ctx), "Enter opens the menu");
    };
    let chosen = |u: &Ui| u.app.inspect()["uiScale"]["choice"].clone();

    let mut u = Ui::new(Session::new());
    u.app.ui.preferences = true;
    u.ctx.enable_accesskit();
    settle(&mut u, 1.0);
    assert_eq!(chosen(&u), json!(1.0));

    // Escape closes the menu and nothing else: the size is unchanged and Preferences stays open.
    open(&mut u);
    press(&mut u, Key::Escape);
    u.frame(vec![]);
    assert!(!Popup::is_any_open(&u.ctx));
    assert!(u.app.ui.preferences, "the first Escape closes the menu only");
    assert_eq!(chosen(&u), json!(1.0));

    // The arrows enter the menu on its first size and go down through the sizes (75, 90, 100, 110,
    // 125); Enter picks the one that has the keyboard and closes the menu.
    open(&mut u);
    press(&mut u, Key::ArrowDown);
    assert_eq!(focused_name(&u.ctx), "75%");
    for _ in 0..4 {
        press(&mut u, Key::ArrowDown);
    }
    assert_eq!(focused_name(&u.ctx), "125%");
    for expected in ["150%", "175%", "200%", "200%"] {
        press(&mut u, Key::Tab);
        assert_eq!(focused_name(&u.ctx), expected, "Tab stays in the size menu");
    }
    for expected in ["175%", "150%", "125%"] {
        press_mod(&mut u, Key::Tab, Modifiers::SHIFT);
        assert_eq!(focused_name(&u.ctx), expected, "Shift+Tab stays in the size menu");
    }
    press(&mut u, Key::Enter);
    assert_eq!(chosen(&u), json!(1.25));
    assert!(!Popup::is_any_open(&u.ctx), "picking a size closes the menu");
    assert!(u.app.ui.preferences, "picking a size leaves Preferences open");
    // With no menu open, Escape closes Preferences as before.
    press(&mut u, Key::Escape);
    assert!(!u.app.ui.preferences);
}
