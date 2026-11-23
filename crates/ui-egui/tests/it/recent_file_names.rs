//! Recent file rows keep their full accessible name.

use egui::{Context, Id, Pos2, RawInput, Rect, accesskit::Role, vec2};
use figcraft_engine::Session;
use figcraft_ui_egui::{FigApp, NoServices};

/// Render the home screen in `view` with `path` (and a single-space control) as recents and return
/// the row's (label, shortcut, focused name).
fn row(view: &str, path: &str, prefix: &str, exclude: &str) -> (String, Option<String>, String) {
    let mut session = Session::empty();
    session.prefs.recent_files = vec![path.into(), "Q3 report.figcraft".into()];
    session.prefs.getting_started_dismissed = true;
    session.prefs.home_views.insert("recents".into(), view.into());
    let mut app = FigApp::new(session, Box::new(NoServices));
    app.ui.home = true;
    let ctx = Context::default();
    ctx.enable_accesskit();
    let mut found = None;
    for frame in 0..4 {
        if frame >= 2 {
            ctx.memory_mut(|m| m.request_focus(Id::unique(("home_recent_file", path))));
        }
        let mut out = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))),
                time: Some(frame as f64 / 60.0),
                ..Default::default()
            },
            |ui| app.ui(ui),
        );
        out.textures_delta.clear();
        if frame == 3 {
            let nodes = out.platform_output.accesskit_update.unwrap().nodes;
            let mut matches = nodes.iter().map(|(_, n)| n).filter(|n| {
                n.role() == Role::Button && n.label().is_some_and(|s| s.starts_with(prefix) && (exclude.is_empty() || !s.starts_with(exclude)))
            });
            let n = matches.next().expect("file row");
            assert!(matches.next().is_none(), "{view}: more than one row matches {prefix:?}");
            found = Some((
                n.label().unwrap_or_default().to_string(),
                n.keyboard_shortcut().map(str::to_string),
                figcraft_ui_egui::a11y::focused_name(&ctx),
            ));
        }
    }
    found.expect("rendered")
}

#[test]
fn recent_file_names_preserve_spaces() {
    for view in ["grid", "list"] {
        for (path, prefix, full) in [("Q3  report.figcraft", "Q3", "Q3  report"), ("Notes  A.figcraft", "Notes", "Notes  A")] {
            // The single-space control "Q3 report" also starts with "Q3"; it is not the row under test.
            let (label, shortcut, focused) = row(view, path, prefix, "Q3 report");
            assert!(label.starts_with(full), "{view}: label {label:?}");
            assert_eq!(shortcut, None, "{view}: {path}");
            assert!(focused.starts_with(full), "{view}: focused {focused:?}");
        }
        let (label, shortcut, _) = row(view, "Q3  report.figcraft", "Q3 report", "");
        assert!(label.starts_with("Q3 report"), "{view}: control {label:?}");
        assert_eq!(shortcut, None);
    }
}

#[path = "../support/native_raster.rs"]
mod native_raster;

/// Native Home output for the default-off capability and a connected provider. The optional
/// image is rasterised from the same egui meshes and font textures inspected by this test.
#[test]
fn provider_recents_render_separately_from_device_history() {
    let directory = std::env::var_os("CRAFT_PROVIDER_UI_FIXTURES").map(std::path::PathBuf::from);
    if let Some(path) = &directory {
        std::fs::create_dir_all(path).unwrap();
    }
    for enabled in [false, true] {
        let mut session = Session::empty();
        session.prefs.recent_files = vec!["Device design.figcraft".into()];
        session.prefs.getting_started_dismissed = true;
        session.prefs.home_views.insert("recents".into(), "list".into());
        let mut app = FigApp::new(session, Box::new(NoServices));
        app.ui.home = true;
        if enabled {
            let host = &app.file_providers;
            host.set_session("fixture-session-0123456789abcdef".into()).unwrap();
            let connection = host.connect("fixture".into(), "External storage".into(), "fixture-account".into()).unwrap();
            host.set_resources(
                &connection,
                vec![figcraft_ui_egui::file_providers::Resource {
                    id: "provider-design".into(),
                    name: "Provider design.figcraft".into(),
                    updated_at: String::new(),
                }],
            )
            .unwrap();
        }
        let ctx = Context::default();
        let mut textures = std::collections::HashMap::new();
        let mut output = None;
        for frame in 0..6 {
            let mut out = ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 1000.0))),
                    max_texture_side: Some(8192),
                    time: Some(frame as f64 / 60.0),
                    ..Default::default()
                },
                |ui| app.ui(ui),
            );
            native_raster::remember(&mut textures, &mut out);
            output = Some(out);
        }
        let out = output.unwrap();
        fn texts(shape: &egui::Shape, found: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => found.push(text.galley.text().to_string()),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| texts(shape, found)),
                _ => {}
            }
        }
        let mut found = Vec::new();
        for shape in &out.shapes {
            texts(&shape.shape, &mut found);
        }
        assert!(found.iter().any(|text| text == "Device design"), "{found:?}");
        assert_eq!(found.iter().any(|text| text == "Provider design.figcraft"), enabled, "{found:?}");
        assert_eq!(found.iter().any(|text| text == "Recent files — External storage"), enabled, "{found:?}");
        assert_eq!(found.iter().any(|text| text == "Recent files on this device"), enabled, "{found:?}");
        if let Some(path) = &directory {
            let state = if enabled { "on" } else { "off" };
            native_raster::raster(&ctx, &textures, out).save(path.join(format!("figcraft-provider-recents-{state}.png"))).unwrap();
        }
    }
}
