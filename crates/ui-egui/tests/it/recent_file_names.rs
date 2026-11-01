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
