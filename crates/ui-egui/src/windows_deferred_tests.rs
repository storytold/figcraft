//! What the window does later for a control client: the keyboard a frame of injected input leaves
//! on the unsaved-changes question belongs to that window alone, and a window a confined client
//! asked for (`window.new`) is opened under the folders that client was given.

use std::collections::HashSet;
use std::sync::Arc;

use egui::accesskit::{NodeId, Role};
use egui::{Event, RawInput, ViewportId};
use figcraft_engine::file_access::{self, Roots};
use figcraft_engine::{Session, UiRequest};
use serde_json::json;

use crate::{FigApp, NoServices};

fn second_window() -> ViewportId {
    ViewportId::from_hash_of("second window")
}

fn key_tap(key: egui::Key) -> Vec<Event> {
    [true, false].map(|pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Default::default() }).to_vec()
}

/// A window over a file with unsaved changes, the About window open (one of the states in which the
/// app's own Tab handler stands aside, so egui moves the keyboard with Tab) and the unsaved-changes
/// question on screen, AccessKit on.
fn blocked_close_question() -> (egui::Context, FigApp) {
    let mut s = Session::new();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    let mut app = FigApp::new(s, Box::new(NoServices));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    app.ui.about = true;
    app.ui.confirm_close = true;
    for _ in 0..3 {
        first_window_frame(&ctx, &mut app, Vec::new(), false, &mut HashSet::new());
    }
    assert!(app.ui.about, "the About window stays open");
    assert!(app.ui.confirm_close, "the question shows over it");
    (ctx, app)
}

/// One frame of the first window with `events` (a control client's batch when `injected`, else the
/// user's own); whether the keyboard is on the question's "Don't Save" button afterwards.
/// AccessKit updates carry only the nodes that changed: `dont_save` remembers the button's id.
fn first_window_frame(ctx: &egui::Context, app: &mut FigApp, events: Vec<Event>, injected: bool, dont_save: &mut HashSet<NodeId>) -> bool {
    let events = if injected {
        assert!(app.push_control_input(events, None), "the batch is queued");
        crate::windows::take_input(app, ViewportId::ROOT)
    } else {
        events
    };
    let mut out = ctx.run_ui(RawInput { events, ..Default::default() }, |ui| {
        crate::windows::on_close_request(app, ui.ctx());
        app.ui(ui);
    });
    out.textures_delta.clear();
    let update = out.platform_output.accesskit_update.expect("an AccessKit update");
    for (id, node) in &update.nodes {
        if node.role() == Role::Button && node.label() == Some("Don't Save") {
            dont_save.insert(*id);
        }
    }
    dont_save.contains(&update.focus)
}

/// One frame of the second window: no events of its own, but the app draws its dialogs in it too.
fn second_window_frame(ctx: &egui::Context, app: &mut FigApp) {
    ctx.run_ui(crate::windows_tests::raw_input_in(second_window()), |ui| app.ui(ui)).textures_delta.clear();
}

/// How many of the user's own Tab put the keyboard on "Don't Save". Panics when none ever does.
fn tabs_to_dont_save() -> usize {
    let (ctx, mut app) = blocked_close_question();
    let mut seen = HashSet::new();
    let Some(k) = (1..=400).find(|_| first_window_frame(&ctx, &mut app, key_tap(egui::Key::Tab), false, &mut seen)) else {
        panic!("the user's own Tab never reaches Don't Save in 400 frames: the test cannot show the defect");
    };
    k
}

/// The keyboard a control client's Tab left on "Don't Save" is let go by the first window's next
/// frame, even when a frame of another window comes first. Without a fix the mark that asks for
/// it is one flag for the whole app, the second window's frame takes it, and the first window
/// keeps a keyboard on the button the client chose.
///
/// Calibration: with the user's own Tab instead of the client's, the second window's frame leaves
/// the keyboard on "Don't Save" (the first window's own focus is not touched by it). Without
/// that, a keyboard that is gone proves nothing, and the test fails there.
#[test]
fn a_frame_of_another_window_does_not_take_back_the_keyboard_left_on_dont_save() {
    let k = tabs_to_dont_save();

    let (ctx, mut app) = blocked_close_question();
    let mut seen = HashSet::new();
    for _ in 0..k {
        first_window_frame(&ctx, &mut app, key_tap(egui::Key::Tab), false, &mut seen);
    }
    second_window_frame(&ctx, &mut app);
    let kept = first_window_frame(&ctx, &mut app, Vec::new(), false, &mut seen);
    assert!(kept, "calibration: the user's own keyboard stays on Don't Save across a frame of the second window");

    let (ctx, mut app) = blocked_close_question();
    let mut seen = HashSet::new();
    for _ in 0..k {
        first_window_frame(&ctx, &mut app, key_tap(egui::Key::Tab), true, &mut seen);
    }
    second_window_frame(&ctx, &mut app);
    assert!(!app.control_synthetic, "the frames that follow are the user's own");
    let left = first_window_frame(&ctx, &mut app, Vec::new(), false, &mut seen);
    assert!(!left, "a control client's Tab left the keyboard on Don't Save: a frame of another window took back the mark");
}

/// A folder of its own for one test, made new.
fn test_dir(tag: &str) -> std::path::PathBuf {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("figcraft-deferred-window-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A saved file with one rectangle in `dir`, made by a session of its own.
fn saved_file(dir: &std::path::Path, name: &str) -> String {
    let path = dir.join(name).to_string_lossy().into_owned();
    let mut s = Session::new();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    s.execute("file.saveAs", &json!({"path": path})).unwrap();
    path
}

/// The files `app` has open after a `window.new` for `path` was queued with `roots`, and handled
/// by the code that opens windows (`windows::requests`), the way a frame does it.
fn open_after_request(path: &str, roots: Option<Arc<Roots>>) -> usize {
    let mut app = FigApp::new(Session::new(), Box::new(NoServices));
    let before = app.session.docs.len();
    app.window_requests.push(UiRequest { kind: "window.new".into(), data: json!({ "path": path }), roots });
    crate::windows::requests(&mut app, &egui::Context::default());
    app.session.docs.len() - before
}

/// A `window.new` a confined client asked for is opened later, in a frame, when the roots the client
/// was given no longer apply. The window opens under them anyway: a file outside them is not read
/// then (the file a permitted document links to is read by the same code, under the same roots).
/// A library linked from a document is not mounted here; the test stands on the file the request
/// opens, which goes through the same check.
///
/// Calibration: the same request with roots that cover the file opens it, and one with no roots
/// (the user's own) opens it too, so a file that stays shut is the roots' doing.
#[test]
fn a_window_asked_for_by_a_confined_client_opens_under_its_roots() {
    let allowed = test_dir("allowed");
    let outside = test_dir("outside");
    let inside_file = saved_file(&allowed, "inside.figcraft");
    let outside_file = saved_file(&outside, "outside.figcraft");
    let folder = allowed.to_string_lossy().into_owned();
    let roots = Arc::new(Roots::new(std::slice::from_ref(&folder), std::slice::from_ref(&folder)).unwrap());

    assert_eq!(open_after_request(&inside_file, Some(roots.clone())), 1, "calibration: a file inside the roots opens");
    assert_eq!(open_after_request(&outside_file, None), 1, "calibration: with no roots (the user's own request) any file opens");
    let read = open_after_request(&outside_file, Some(roots));
    std::fs::remove_dir_all(&allowed).ok();
    std::fs::remove_dir_all(&outside).ok();
    assert_eq!(read, 0, "the window opened a file outside the folders of the client that asked for it");
}

/// The roots a request carries are the ones its client had when it asked, and the window code
/// runs it under exactly those: asked under roots, the queued request holds them; asked with none,
/// it holds none.
#[test]
fn a_window_request_carries_the_roots_it_was_made_under() {
    let dir = test_dir("carried");
    let file = saved_file(&dir, "a.figcraft");
    let folder = dir.to_string_lossy().into_owned();
    let roots = Arc::new(Roots::new(std::slice::from_ref(&folder), std::slice::from_ref(&folder)).unwrap());

    let queued = |under: Option<&Arc<Roots>>| {
        let mut app = FigApp::new(Session::new(), Box::new(NoServices));
        file_access::confined(under, || app.run("window.new", json!({"path": file})));
        app.window_requests.iter().find(|r| r.kind == "window.new").map(|r| r.roots.clone())
    };
    let confined = queued(Some(&roots));
    let free = queued(None);
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(confined, Some(Some(roots)), "calibration: the request was queued, and holds the roots it was made under");
    assert_eq!(free, Some(None), "a request made with no roots holds none");
}
