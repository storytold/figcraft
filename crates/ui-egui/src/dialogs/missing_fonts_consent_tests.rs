//! The Missing fonts window against a control client: input injected through the control channel
//! (`ui.input`) reaches the window like the user's own, and must not answer for the user whether
//! fonts may be downloaded (the setting every Craft app shares).

use figcraft_engine::Session;

use super::{Clicks, Shown, act};
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
