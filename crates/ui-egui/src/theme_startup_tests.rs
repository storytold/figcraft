//! Startup and system-theme changes through the real UI frame, before any settling frames.

use egui::{Context, RawInput, Theme};
use figcraft_engine::{Prefs, Session};
use serde_json::json;

use crate::{FigApp, NoServices, theme::Tokens};

fn frame(ctx: &Context, app: &mut FigApp, system: Option<Theme>) {
    let mut output = ctx.run_ui(RawInput { system_theme: system, ..Default::default() }, |ui| app.ui(ui));
    output.textures_delta.clear();
}

#[test]
fn fresh_app_uses_system_dark_theme_on_its_first_frame() {
    let ctx = Context::default();
    let mut app = FigApp::new(Session::empty(), Box::new(NoServices));
    frame(&ctx, &mut app, Some(Theme::Dark));
    assert!(app.ui.dark, "a first run must use the OS dark theme before drawing");
    assert_eq!(Tokens::get(&ctx), Tokens::dark());
    assert_eq!(app.session.prefs.theme, "system", "following the OS must remain a preference, not a saved snapshot");
}

#[test]
fn first_run_and_preferences_without_a_theme_follow_system_light() {
    for prefs in [Prefs::default(), serde_json::from_value(json!({"nudge_small": 2})).unwrap()] {
        let ctx = Context::default();
        let mut session = Session::empty();
        session.prefs = prefs;
        let mut app = FigApp::new(session, Box::new(NoServices));
        frame(&ctx, &mut app, Some(Theme::Light));
        assert_eq!(app.session.prefs.theme, "system");
        assert_eq!(Tokens::get(&ctx), Tokens::light());
    }
}

#[test]
fn saved_manual_themes_survive_startup_and_os_changes() {
    for (saved, expected) in [("light", Tokens::light()), ("dark", Tokens::dark())] {
        let ctx = Context::default();
        let mut session = Session::empty();
        session.prefs = serde_json::from_value(json!({"theme": saved})).unwrap();
        let mut app = FigApp::new(session, Box::new(NoServices));
        for system in [Theme::Dark, Theme::Light, Theme::Dark] {
            frame(&ctx, &mut app, Some(system));
            assert_eq!(Tokens::get(&ctx), expected);
            assert_eq!(app.session.prefs.theme, saved);
        }
    }
}

#[test]
fn returning_to_system_reads_the_os_not_the_previously_drawn_palette() {
    for (manual, os) in [("view.themeLight", Theme::Dark), ("view.themeDark", Theme::Light)] {
        let ctx = Context::default();
        let mut app = FigApp::new(Session::empty(), Box::new(NoServices));
        app.session.execute(manual, &json!({})).unwrap();
        frame(&ctx, &mut app, Some(os));
        app.session.execute("view.themeSystem", &json!({})).unwrap();
        frame(&ctx, &mut app, Some(os));
        assert_eq!(app.ui.dark, os == Theme::Dark);
        assert_eq!(Tokens::get(&ctx).dark, os == Theme::Dark);
    }
}

#[test]
fn system_theme_changes_apply_in_the_same_frame() {
    let ctx = Context::default();
    let mut app = FigApp::new(Session::empty(), Box::new(NoServices));
    app.session.execute("view.themeSystem", &json!({})).unwrap();
    for os in [Theme::Dark, Theme::Light, Theme::Dark] {
        frame(&ctx, &mut app, Some(os));
        assert_eq!(Tokens::get(&ctx).dark, os == Theme::Dark);
        assert_eq!(app.session.prefs.theme, "system");
    }
}

#[test]
fn unavailable_system_theme_uses_the_existing_light_fallback() {
    let ctx = Context::default();
    let mut app = FigApp::new(Session::empty(), Box::new(NoServices));
    app.session.execute("view.themeSystem", &json!({})).unwrap();
    frame(&ctx, &mut app, None);
    assert_eq!(Tokens::get(&ctx), Tokens::light());
}

fn rel_luminance(c: egui::Color32) -> f64 {
    let srgb = |b: u8| {
        let v = b as f64 / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * srgb(c.r()) + 0.7152 * srgb(c.g()) + 0.0722 * srgb(c.b())
}

fn contrast_ratio(c1: egui::Color32, c2: egui::Color32) -> f64 {
    let l1 = rel_luminance(c1);
    let l2 = rel_luminance(c2);
    let (bright, dark) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
    (bright + 0.05) / (dark + 0.05)
}

#[test]
fn light_theme_menus_follow_light_palette() {
    let light = Tokens::light();
    assert!(!light.dark);
    assert_eq!(light.menu, egui::Color32::from_rgb(0xFF, 0xFF, 0xFF));
    assert_eq!(light.menu_text, egui::Color32::from_rgb(0x1E, 0x1E, 0x1E));
    assert_eq!(light.menu_hover, egui::Color32::from_rgb(0xF5, 0xF5, 0xF5));
    assert_eq!(light.menu_dim, egui::Color32::from_rgb(0x6B, 0x6B, 0x6B));
    assert_ne!(light.menu, Tokens::dark().menu);
    assert_ne!(light.menu_text, Tokens::dark().menu_text);

    // WCAG AA contrast verification (at least 4.5:1 for normal text).
    let text_on_menu = contrast_ratio(light.menu_text, light.menu);
    let text_on_hover = contrast_ratio(light.menu_text, light.menu_hover);
    let dim_on_menu = contrast_ratio(light.menu_dim, light.menu);
    let dim_on_hover = contrast_ratio(light.menu_dim, light.menu_hover);

    assert!(text_on_menu >= 10.0, "menu_text on menu contrast {text_on_menu:.2}:1");
    assert!(text_on_hover >= 10.0, "menu_text on hover contrast {text_on_hover:.2}:1");
    assert!(dim_on_menu >= 4.5, "menu_dim on menu contrast {dim_on_menu:.2}:1 (WCAG AA requires >= 4.5)");
    assert!(dim_on_hover >= 4.5, "menu_dim on hover contrast {dim_on_hover:.2}:1 (WCAG AA requires >= 4.5)");
}
