//! Startup and system-theme changes through the real UI frame, before any settling frames.

use egui::{Context, RawInput, Theme};
use figcraft_engine::{Prefs, Session};
use serde_json::json;

use crate::{FigApp, NoServices, Services, theme::Tokens};

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

/// Services whose colour scheme answer stands in for the platform's (winit gives no theme on Linux
/// and the BSDs, where the freedesktop colour scheme takes over).
struct Scheme(Option<bool>);
impl Services for Scheme {
    fn system_dark(&self) -> Option<bool> {
        self.0
    }
}

#[test]
fn without_a_system_theme_the_colour_scheme_services_decide() {
    for (dark, expected) in [(Some(true), Tokens::dark()), (Some(false), Tokens::light())] {
        let ctx = Context::default();
        let mut app = FigApp::new(Session::empty(), Box::new(Scheme(dark)));
        app.session.execute("view.themeSystem", &json!({})).unwrap();
        frame(&ctx, &mut app, None);
        assert_eq!(Tokens::get(&ctx), expected);
        assert_eq!(app.ui.dark, expected.dark);
    }
}

#[test]
fn the_os_theme_wins_over_the_colour_scheme_services() {
    for (os, scheme) in [(Theme::Light, Some(true)), (Theme::Dark, Some(false))] {
        let ctx = Context::default();
        let mut app = FigApp::new(Session::empty(), Box::new(Scheme(scheme)));
        app.session.execute("view.themeSystem", &json!({})).unwrap();
        frame(&ctx, &mut app, Some(os));
        assert_eq!(Tokens::get(&ctx).dark, os == Theme::Dark);
    }
}

#[test]
fn manual_themes_ignore_the_colour_scheme_services() {
    let ctx = Context::default();
    let mut app = FigApp::new(Session::empty(), Box::new(Scheme(Some(false))));
    app.session.execute("view.themeDark", &json!({})).unwrap();
    frame(&ctx, &mut app, None);
    assert_eq!(Tokens::get(&ctx), Tokens::dark());
}
