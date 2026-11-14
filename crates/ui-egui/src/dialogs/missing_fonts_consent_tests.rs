//! The Missing fonts window against a control client: input injected through the control channel
//! (`ui.input`) reaches the window like the user's own, and must not answer for the user whether
//! fonts may be downloaded (the setting every Craft app shares).

use figcraft_engine::Session;

use super::{ALLOW_TRIGGERED, Clicks, Shown, act};
use crate::{FigApp, NoServices};

/// The family the window asks about, while its question is open.
fn asked(app: &FigApp) -> Option<&str> {
    app.ui.missing_fonts.as_ref().and_then(|m| m.consent.as_deref())
}

#[test]
fn injected_input_cannot_answer_the_question_about_downloading_fonts() {
    let mut app = FigApp::new(Session::new(), Box::new(NoServices));
    app.ui.missing_fonts = Some(Shown { uid: 1, families: Vec::new(), indexing: false, consent: Some("Some Family".into()) });
    // The frame applies a batch a control client sent with `ui.input`. "Not now" is the answer
    // tried here: it changes nothing on this computer, where "Allow and download" would turn
    // downloading on. Both answers are taken by the same lines.
    app.control_synthetic = true;
    act(&mut app, Clicks { consent: Some(false), ..Default::default() });
    assert_eq!(asked(&app), Some("Some Family"), "a control client answered the question about downloading fonts");
    // The user's own answer is taken.
    app.control_synthetic = false;
    act(&mut app, Clicks { consent: Some(false), ..Default::default() });
    assert_eq!(asked(&app), None);
}

/// A window with a file, the About window open (one of the states in which the app's own Tab
/// handler stands aside, `blocked` in a11y.rs, so egui moves the keyboard with Tab) and the
/// Missing fonts question on screen for that file. Like `blocked_question` in windows_tests.rs.
fn blocked_consent() -> (egui::Context, FigApp) {
    let mut s = Session::new();
    s.execute("shape.rectangle", &serde_json::json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    let uid = s.ds().unwrap().uid;
    let mut app = FigApp::new(s, Box::new(NoServices));
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    app.ui.about = true;
    app.ui.missing_fonts = Some(Shown { uid, families: Vec::new(), indexing: false, consent: Some("Some Family".into()) });
    frame_with(&ctx, &mut app, Vec::new(), false, &mut None);
    assert!(app.has_files(), "a file is open");
    assert!(app.ui.about, "the About window stays open");
    assert_eq!(asked(&app), Some("Some Family"), "the question shows");
    (ctx, app)
}

/// One frame of `events` (a control client's batch when `injected`, else the user's own), as
/// `frame_with` in windows_tests.rs; records in `allow` the id of the "Allow and download" button
/// and returns whether the keyboard is on it afterwards.
fn frame_with(ctx: &egui::Context, app: &mut FigApp, events: Vec<egui::Event>, injected: bool, allow: &mut Option<egui::accesskit::NodeId>) -> bool {
    let events = if injected {
        assert!(app.push_control_input(events), "the batch is queued");
        crate::windows::take_input(app, egui::ViewportId::ROOT)
    } else {
        events
    };
    let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| app.ui(ui));
    out.textures_delta.clear();
    let update = out.platform_output.accesskit_update.expect("an AccessKit update");
    for (id, node) in &update.nodes {
        if node.role() == egui::accesskit::Role::Button && node.label() == Some("Allow and download") {
            *allow = Some(*id);
        }
    }
    *allow == Some(update.focus)
}

/// A Tab key press and release, as `key_tap` in windows_tests.rs.
fn tab() -> Vec<egui::Event> {
    [true, false]
        .map(|pressed| egui::Event::Key { key: egui::Key::Tab, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE })
        .to_vec()
}

/// An Enter key press and release, the user's own.
fn enter() -> Vec<egui::Event> {
    [true, false]
        .map(|pressed| egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE })
        .to_vec()
}

/// Calibration on a window of its own: how many of the user's own Tab it takes to put the
/// keyboard on "Allow and download". Panics when no Tab ever gets there.
fn tabs_to_allow_and_download() -> usize {
    let (ctx, mut app) = blocked_consent();
    let mut allow = None;
    let Some(k) = (1..=400).find(|_| frame_with(&ctx, &mut app, tab(), false, &mut allow)) else {
        panic!("the user's own Tab never reaches Allow and download in 400 frames: the test cannot show the defect");
    };
    k
}

/// A control client's Tab can leave the keyboard on "Allow and download" and the
/// user's own Enter, the frame after, is taken by `act` as the answer Allow: it takes the family,
/// calls `allow_downloads()` (which writes in the font folder the Craft apps share) and the
/// question is gone, though the user never answered. The window must drop a
/// focus a client moved onto the question's buttons. The defect is only reachable while the app's
/// own Tab handler stands aside (the About window is open). The calibration comes first so the
/// test is never green for nothing: it counts, on a window of its own, the user's Tab that reach
/// the button (and panics if none does), then has a client inject that many on a fresh one.
#[test]
fn injected_tab_cannot_leave_allow_and_download_for_the_users_enter() {
    let k = tabs_to_allow_and_download();
    let (ctx, mut app) = blocked_consent();
    let mut allow = None;
    for _ in 0..k {
        frame_with(&ctx, &mut app, tab(), true, &mut allow);
    }
    assert!(!app.control_synthetic, "the frame that follows is the user's own");
    assert_eq!(asked(&app), Some("Some Family"), "the question is still on screen before the user's Enter");
    // The user's own Enter. Taken by "Allow and download" it would run the allow action (here the
    // test seam `turn_downloads_on`, which notes the call and touches neither the shared font
    // folder nor the network; outside tests it is `allow_downloads()`).
    ALLOW_TRIGGERED.with(|n| n.set(0));
    frame_with(&ctx, &mut app, enter(), false, &mut allow);
    frame_with(&ctx, &mut app, Vec::new(), false, &mut allow);
    let triggered = ALLOW_TRIGGERED.with(|n| n.get());
    assert_eq!(triggered, 0, "the user's own Enter ran Allow and download: a control client had left the keyboard on it");
    assert_eq!(asked(&app), Some("Some Family"), "the question is still on screen");
}
