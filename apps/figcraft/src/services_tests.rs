//! The preferences are kept as plugins change, not only when the app quits: a plugin imported
//! (then removed) is still there (gone) when the app is killed and starts again.

use std::sync::{Arc, Mutex};

use figcraft_engine::Session;
use figcraft_ui_egui::FigApp;
use serde_json::json;

use crate::services::Desktop;

const ID: &str = "org.example.tiny";

fn root(name: &str) -> std::path::PathBuf {
    let r = std::env::temp_dir().join(format!("figcraft-prefs-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&r);
    std::fs::create_dir_all(&r).unwrap();
    r
}

/// A plugin folder (manifest and module) in `root`.
fn plugin(root: &std::path::Path) -> std::path::PathBuf {
    let dir = root.join("tiny");
    std::fs::create_dir_all(&dir).unwrap();
    let manifest = json!({"id": ID, "name": "Tiny tool", "version": "1.0.0", "main": "tiny.wasm", "permissions": {"access": "read"}});
    std::fs::write(dir.join("figcraft-plugin.json"), manifest.to_string()).unwrap();
    let wasm =
        wat::parse_str(r#"(module (memory (export "memory") 1) (func (export "figcraft_run") (param i32) (result i32) i32.const 0))"#).unwrap();
    std::fs::write(dir.join("tiny.wasm"), wasm).unwrap();
    dir
}

/// The app as it starts with its preferences in `dir`.
fn start(dir: &std::path::Path) -> FigApp {
    let svc = Desktop::load_from(Some(dir.to_path_buf()), Arc::new(Mutex::new(None)));
    let mut s = Session::new();
    if let Some(prefs) = svc.prefs.clone() {
        s.prefs = prefs;
    }
    FigApp::new(s, Box::new(svc))
}

/// Draw two frames (the first only sets the fonts up).
fn frames(fig: &mut FigApp, ctx: &egui::Context) {
    for _ in 0..2 {
        ctx.run_ui(egui::RawInput::default(), |ui| fig.ui(ui)).textures_delta.clear();
    }
}

#[test]
fn plugins_are_kept_when_the_app_is_killed() {
    let dir = root("killed");
    let folder = plugin(&dir);
    let ctx = egui::Context::default();
    let mut fig = start(&dir);
    assert!(fig.run("plugin.import", json!({"path": folder.to_string_lossy()})).is_some());
    frames(&mut fig, &ctx);
    // Killed: no quit, so nothing saves on the way out.
    drop(fig);
    let mut fig = start(&dir);
    assert_eq!(fig.session.prefs.plugins.iter().map(|p| p.manifest.id.as_str()).collect::<Vec<_>>(), [ID], "the plugin is still installed");
    assert!(fig.run("plugin.remove", json!({"id": ID})).is_some());
    frames(&mut fig, &ctx);
    drop(fig);
    assert!(start(&dir).session.prefs.plugins.is_empty(), "the removal was kept too");
    assert!(!dir.join("state.json.tmp").exists(), "no temporary file is left behind");
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn preferences_are_readable_by_their_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = root("private");
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    // An older world-readable file and a leftover temporary one from a crash.
    for name in ["state.json", "state.json.tmp"] {
        std::fs::write(dir.join(name), "{}").unwrap();
        std::fs::set_permissions(dir.join(name), std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    crate::services::write_prefs(&dir, &figcraft_engine::Prefs::default());
    assert_eq!(mode(&dir.join("state.json")), 0o600);
    let _ = std::fs::remove_dir_all(&dir);
}
