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

/// A live link that only records the sessions it was told to leave.
struct Leaves(Arc<Mutex<Vec<u64>>>);

impl figcraft_engine::live::LiveLink for Leaves {
    fn host(&mut self, _: u64, _: figcraft_engine::live::HostOptions) -> Result<figcraft_engine::live::SessionInfo, String> {
        Err("not in this test".into())
    }
    fn join(&mut self, _: u64, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn leave(&mut self, session: u64) {
        self.0.lock().unwrap().push(session);
    }
    fn presence(&mut self, _: u64, _: &figcraft_engine::live::Presence) {}
    fn send(&mut self, _: u64, _: figcraft_engine::figcraft_collab::message::ClientMsg) {}
    fn poll(&mut self) -> Vec<(u64, figcraft_engine::live::Event)> {
        Vec::new()
    }
}

#[test]
fn quitting_keeps_the_preferences_and_ends_the_live_sessions() {
    let dir = root("shutdown");
    let mut fig = start(&dir);
    let left = Arc::new(Mutex::new(Vec::new()));
    fig.session.live.install(Box::new(Leaves(left.clone())));
    let joined = fig.run("collab.join", json!({"link": "ws://127.0.0.1:9/?file=f1&token=EditSecretEditSecret00"})).unwrap();
    fig.session.prefs.user_name = "Ada".into();
    crate::services::shutdown(&mut fig, Some(&dir), None);
    assert_eq!(*left.lock().unwrap(), [joined["session"].as_u64().unwrap()], "the session was left");
    assert!(!fig.session.live.available(), "nothing is left to send on");
    assert_eq!(start(&dir).session.prefs.user_name, "Ada", "the preferences were kept");
    let _ = std::fs::remove_dir_all(&dir);
}

/// #1290: the Linux portal opens every dialog where it likes, so the app keeps the folder itself.
#[test]
fn the_next_dialog_starts_in_the_folder_of_the_last_pick() {
    use crate::services::{folder_to_remember, remembering, start_folder};
    use std::path::PathBuf;
    let dir = root("last-folder");
    let file = dir.join("Poster.figcraft");
    assert_eq!(folder_to_remember(&file, false), Some(dir.clone()), "a file: its folder");
    assert_eq!(folder_to_remember(&dir, true), Some(dir.clone()), "a folder pick: the folder itself");
    assert_eq!(folder_to_remember(std::path::Path::new("Poster.figcraft"), false), None, "a bare name has no folder");

    let last = Arc::new(Mutex::new(None::<PathBuf>));
    let mut pick = remembering(last.clone(), Box::pin(std::future::ready(Some(file.clone()))), false);
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    assert_eq!(pick.as_mut().poll(&mut cx), std::task::Poll::Ready(Some(file)));
    assert_eq!(*last.lock().unwrap(), Some(dir.clone()), "a pick is remembered");
    let mut cancelled = remembering(last.clone(), Box::pin(std::future::ready(None)), false);
    assert_eq!(cancelled.as_mut().poll(&mut cx), std::task::Poll::Ready(None));
    assert_eq!(*last.lock().unwrap(), Some(dir.clone()), "a cancelled dialog keeps the folder");

    assert_eq!(start_folder(Some(dir.clone())), Some(dir.clone()));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(start_folder(Some(dir)), None, "a folder deleted since: the system's own");
}
