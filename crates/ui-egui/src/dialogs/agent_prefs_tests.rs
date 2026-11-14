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
