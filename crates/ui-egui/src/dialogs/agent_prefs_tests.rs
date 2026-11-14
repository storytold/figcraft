//! Preferences ▸ AI agents against a control client: input injected through the control channel
//! (`ui.input`) reaches the Preferences window like the user's own, and must not let a client
//! widen its folder, grant itself control or be promised a shutdown that does not happen.

use egui::{Event, Id};
use figcraft_engine::Session;

use crate::layers_drag_tests::Ui;
use crate::{FigApp, NoServices};

fn app() -> FigApp {
    let mut app = FigApp::new(Session::new(), Box::new(NoServices));
    app.session.prefs.automation_root = "/agents".into();
    app.session.prefs.agent_control = false;
    app.session.prefs_unsaved = false;
    app
}

#[test]
fn injected_input_cannot_move_the_folder_for_agents() {
    let mut app = app();
    // The frame applies a batch a control client sent with `ui.input`.
    app.control_synthetic = true;
    super::set_automation_root(&mut app, "/");
    assert_eq!(app.session.prefs.automation_root, "/agents", "a control client widened its own folder");
    assert!(!app.session.prefs_unsaved);
    // The user's own frame still sets it.
    app.control_synthetic = false;
    super::set_automation_root(&mut app, "/home/ada/agents");
    assert_eq!(app.session.prefs.automation_root, "/home/ada/agents");
}

#[test]
fn injected_input_opens_no_folder_picker() {
    let mut app = app();
    app.control_synthetic = true;
    super::browse_automation_root(&mut app, "Folder for agents");
    assert!(app.pending_dialog.is_none(), "a control client opened the picker that sets its own folder");
}

#[test]
fn injected_input_cannot_answer_the_agent_control_question() {
    let mut app = app();
    app.ui.agent_control_ask = Some(true);
    app.control_synthetic = true;
    super::answer_agent_control(&mut app, true, true);
    assert!(!app.session.prefs.agent_control, "a control client granted itself control");
    assert_eq!(app.ui.agent_control_ask, Some(true), "the question stays for the user to answer");
    // The user's own answer is kept.
    app.control_synthetic = false;
    super::answer_agent_control(&mut app, true, true);
    assert!(app.session.prefs.agent_control);
    assert_eq!(app.ui.agent_control_ask, None);
}

#[test]
fn the_switch_asks_nothing_when_it_cannot_change_anything() {
    // A launch that named its control port keeps its server until exit: asking "Stop AI agents?"
    // would promise a shutdown that does not happen.
    let mut app = app();
    app.agent_control_fixed = true;
    super::ask_agent_control(&mut app, false);
    assert_eq!(app.ui.agent_control_ask, None, "the question promises what the launch options forbid");
    // Injected input opens no question either.
    app.agent_control_fixed = false;
    app.control_synthetic = true;
    super::ask_agent_control(&mut app, true);
    assert_eq!(app.ui.agent_control_ask, None, "a control client opened the question that lets it in");
    app.control_synthetic = false;
    super::ask_agent_control(&mut app, true);
    assert_eq!(app.ui.agent_control_ask, Some(true));
}

/// One frame that applies `events` as a batch a control client sent with `ui.input`
/// (`windows::take_input` stands for the desktop app's input hook).
fn injected(u: &mut Ui, events: Vec<Event>) {
    u.app.push_control_input(events);
    let events = crate::windows::take_input(&mut u.app, egui::ViewportId::ROOT);
    assert!(!events.is_empty(), "the batch reaches the frame");
    u.frame(events);
}

fn pointer(at: egui::Pos2, pressed: bool) -> Vec<Event> {
    vec![Event::PointerMoved(at), Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }]
}

#[test]
fn text_a_client_typed_in_the_folder_field_is_not_kept_by_the_users_next_click() {
    let mut u = Ui::new(Session::new());
    u.app.session.prefs.automation_root = "/agents".into();
    u.app.ui.preferences = true;
    for _ in 0..3 {
        u.frame(vec![]);
    }
    let field = Id::unique("pref_automation_root");
    let at = u.ctx.read_response(field.with("click")).expect("the folder field is shown").rect.center();
    let draft = |u: &Ui| u.ctx.data(|d| d.get_temp::<String>(field.with("editing")));

    // The client clicks the field and types the root of the disk: nothing is kept yet, its text
    // waits in the field.
    injected(&mut u, pointer(at, true));
    injected(&mut u, pointer(at, false));
    injected(&mut u, vec![Event::Text("/".into())]);
    let typed = draft(&u);
    assert!(typed.as_deref().is_some_and(|t| t != "/agents"), "the client's text waits in the field: {typed:?}");
    assert_eq!(u.app.session.prefs.automation_root, "/agents");
    assert!(!u.app.control_synthetic, "the frames that follow are the user's own");

    // The user clicks somewhere else, as they would to go on working: the field ends its edit.
    let away = egui::pos2(at.x - 300.0, at.y);
    u.button(away, true);
    u.button(away, false);
    u.frame(vec![]);
    assert_eq!(draft(&u), None, "the click away ended the edit");
    assert_eq!(u.app.session.prefs.automation_root, "/agents", "the user's click kept what a control client typed as its folder");
}

/// A key press and release, as one batch of events (copied from windows_tests.rs, where it is private).
fn key_tap(key: egui::Key) -> Vec<Event> {
    [true, false].map(|pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Default::default() }).to_vec()
}

/// A window with Preferences open over a document and "Allow AI agents?" asked, AccessKit on.
/// Preferences is one of the states in which the app's own Tab handler stands aside (`blocked` in
/// a11y.rs), so egui moves the keyboard with Tab: the only state in which a client's Tab can reach
/// the question's buttons.
fn asking_to_allow() -> Ui {
    let mut u = Ui::new(Session::new());
    u.app.session.prefs.agent_control = false;
    u.app.session.prefs_unsaved = false;
    u.app.ui.preferences = true;
    u.app.ui.agent_control_ask = Some(true);
    u.ctx.enable_accesskit();
    u.frame(vec![]);
    assert!(u.app.has_files(), "a document is open");
    assert!(u.app.ui.preferences, "Preferences stays open");
    assert_eq!(u.app.ui.agent_control_ask, Some(true), "the question shows");
    u
}

/// One frame of `events` (a control client's batch when `injected`, else the user's own);
/// whether the keyboard is on the question's Allow button afterwards. AccessKit updates carry only
/// the nodes that changed: `allow` remembers the button's id.
fn frame_on_allow(u: &mut Ui, events: Vec<Event>, injected: bool, allow: &mut std::collections::HashSet<egui::accesskit::NodeId>) -> bool {
    let events = if injected {
        assert!(u.app.push_control_input(events), "the batch is queued");
        crate::windows::take_input(&mut u.app, egui::ViewportId::ROOT)
    } else {
        events
    };
    let out = u.frame_output(events);
    let update = out.platform_output.accesskit_update.expect("an AccessKit update");
    for (id, node) in &update.nodes {
        if node.role() == egui::accesskit::Role::Button && node.label() == Some("Allow") {
            allow.insert(*id);
        }
    }
    allow.contains(&update.focus)
}

/// Calibration on a window of its own: how many of the user's own Tab it takes to put the keyboard
/// on Allow. Red, not green, when no Tab ever gets there.
fn tabs_to_allow() -> usize {
    let mut u = asking_to_allow();
    let mut allow = std::collections::HashSet::new();
    let Some(k) = (1..=400).find(|_| frame_on_allow(&mut u, key_tap(egui::Key::Tab), false, &mut allow)) else {
        panic!("the user's own Tab never reaches Allow in 400 frames: the test cannot show the defect");
    };
    k
}

/// A control client's Tab can move the keyboard to Allow, but the user's own Enter must not
/// answer "Allow AI agents?" for the user. Without a fix, the Enter presses the button the client
/// left the keyboard on: the question answers Allow, `answer_agent_control` takes it (the frame is
/// the user's own) and `prefs.agent_control` (an in-memory preference) turns true. The test first
/// counts, on a window of its own, the user's Tab that reach Allow (and fails if none does):
/// without that count the number of injected Tab would be a guess, and a test that never reaches
/// the button would be green for nothing.
#[test]
fn injected_tab_cannot_leave_allow_for_the_users_enter_on_the_agent_question() {
    let k = tabs_to_allow();
    let mut u = asking_to_allow();
    let mut allow = std::collections::HashSet::new();
    for _ in 0..k {
        frame_on_allow(&mut u, key_tap(egui::Key::Tab), true, &mut allow);
    }
    assert!(!u.app.control_synthetic, "the frames that follow are the user's own");
    assert_eq!(u.app.ui.agent_control_ask, Some(true), "the question is still shown before the user's Enter");
    frame_on_allow(&mut u, key_tap(egui::Key::Enter), false, &mut allow);
    u.frame(vec![]);
    assert_eq!(
        u.app.ui.agent_control_ask,
        Some(true),
        "the user's own Enter answered the agent question: a control client had left the keyboard on Allow"
    );
    assert!(!u.app.session.prefs.agent_control, "the user's own Enter answered the agent question: a control client had left the keyboard on Allow");
}
