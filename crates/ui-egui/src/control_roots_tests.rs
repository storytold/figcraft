//! An app started with automation roots: its frames and control requests read and write only
//! inside them, and captures come back as data without a path.

use std::path::PathBuf;
use std::sync::Arc;

use figcraft_engine::Session;
use figcraft_engine::file_access::Roots;
use serde_json::{Value, json};

use crate::{FigApp, NoServices};

struct Dirs(PathBuf);

impl Drop for Dirs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn confined_app(name: &str) -> (FigApp, Dirs) {
    let d = std::env::temp_dir().join(format!("figcraft-ui-roots-{name}-{}", std::process::id()));
    for sub in ["out", "outside"] {
        std::fs::create_dir_all(d.join(sub)).unwrap();
    }
    let mut s = Session::new();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 20, "height": 10})).unwrap();
    let mut app = FigApp::new(s, Box::new(NoServices));
    app.automation_roots = Some(Arc::new(Roots::new(&[], &[d.join("out").to_string_lossy().into_owned()]).unwrap()));
    (app, Dirs(d))
}

/// Control method `method` handled as the desktop app does: inside the frame guard.
fn handle(app: &mut FigApp, method: &str, p: Value) -> Result<Value, String> {
    let ctx = egui::Context::default();
    let mut res = Err("not run".to_string());
    crate::windows::guard(app, &ctx, |app| res = super::handle(app, &ctx, method, &p));
    res
}

#[test]
fn a_confined_app_writes_only_inside_its_write_root() {
    let (mut app, d) = confined_app("write");
    let p = |rel: &str| d.0.join(rel).to_string_lossy().into_owned();
    let e = handle(&mut app, "ui.render", json!({"path": p("outside/r.png")})).unwrap_err();
    assert!(e.contains("automation path rejected"), "{e}");
    let e =
        handle(&mut app, "engine.execute", json!({"command": "file.export", "params": {"format": "svg", "path": p("outside/r.svg")}})).unwrap_err();
    assert!(e.contains("automation path rejected"), "{e}");
    assert!(handle(&mut app, "ui.render", json!({"path": p("out/r.png")})).is_ok());
    // Without a path the PNG comes back as data.
    let r = handle(&mut app, "ui.render", json!({})).unwrap();
    assert!(r["data"].as_str().is_some_and(|d| !d.is_empty()), "{r}");
    assert_eq!(std::fs::read_dir(d.0.join("outside")).unwrap().count(), 0);
    assert!(d.0.join("out/r.png").is_file());
    // Outside the guard (the app's own work) nothing is confined.
    assert!(app.session.execute("file.export", &json!({"format": "svg", "path": p("outside/own.svg")})).is_ok());
}

#[test]
fn a_confined_app_saves_only_inside_its_write_root() {
    // #836: saving over the control channel, both as a new file and to the current one.
    let (mut app, d) = confined_app("save");
    let p = |rel: &str| d.0.join(rel).to_string_lossy().into_owned();
    for cmd in ["file.saveAs", "file.save"] {
        let e = handle(&mut app, "engine.execute", json!({"command": cmd, "params": {"path": p("outside/a.figcraft")}})).unwrap_err();
        assert!(e.contains("automation path rejected"), "{cmd}: {e}");
        let e = handle(&mut app, "engine.execute", json!({"command": cmd, "params": {"path": p("out/../outside/b.figcraft")}})).unwrap_err();
        assert!(e.contains("automation path rejected"), "{cmd}: {e}");
    }
    assert_eq!(std::fs::read_dir(d.0.join("outside")).unwrap().count(), 0, "nothing written outside the root");
    handle(&mut app, "engine.execute", json!({"command": "file.saveAs", "params": {"path": p("out/in.figcraft")}})).unwrap();
    assert!(d.0.join("out/in.figcraft").is_file());
    // Saving again goes to the same file, inside the root.
    handle(&mut app, "engine.execute", json!({"command": "file.save", "params": {}})).unwrap();
}

#[test]
fn window_captures_are_written_through_the_roots_or_returned() {
    let (app, d) = confined_app("capture");
    let img = egui::ColorImage::filled([3, 2], egui::Color32::RED);
    let roots = app.automation_roots.clone();
    let out = d.0.join("out/shot.png").to_string_lossy().into_owned();
    let outside = d.0.join("outside/shot.png").to_string_lossy().into_owned();
    figcraft_engine::file_access::confined(roots.as_ref(), || {
        assert!(super::shot_reply(&img, Some(&outside)).unwrap_err().contains("automation path rejected"));
        assert_eq!(super::shot_reply(&img, Some(&out)).unwrap()["width"], 3);
        let data = super::shot_reply(&img, None).unwrap();
        assert_eq!((data["format"].as_str(), data["height"].as_u64()), (Some("png"), Some(2)));
    });
    assert!(d.0.join("out/shot.png").is_file() && !d.0.join("outside/shot.png").exists());
}

#[test]
fn ui_input_rejects_non_finite_coordinates() {
    let (mut app, _d) = confined_app("input");
    for ev in [json!({"move": [1e300, 5]}), json!({"down": [5, 1e300]}), json!({"scroll": [1e300, 0]})] {
        let r = handle(&mut app, "ui.input", json!({"events": [ev]}));
        assert!(r.is_err(), "{ev}: {r:?}");
    }
    assert!(app.control_input.iter().all(|e| !matches!(e, egui::Event::PointerMoved(p) if !p.is_finite())));
}
