//! The Assets panel showing a file that names a team library on an unknown server doesn't
//! contact it (#1097): the library waits until the user clicks Check for updates. That a library
//! not allowed is never asked is the engine's `library_beacon_tests`; here, what allows it.

use figcraft_doc::LibraryLink;
use figcraft_engine::Session;

use crate::layers_drag_tests::Ui;
use crate::live_tests::{flat, has_text, text_center};

const LINK: &str = "ws://tracker.example:9/?file=f&token=per-file-secret";

#[test]
fn the_assets_panel_checks_an_unknown_library_server_only_when_asked() {
    let mut s = Session::new();
    s.doc_mut().unwrap().libraries_mut().enabled.push(LibraryLink { id: "lib".into(), name: "Kit".into(), path: LINK.into(), version: 1 });
    let mut u = Ui::new(s);
    u.app.ui.left_tab = crate::LeftTab::Assets;
    u.app.ui.libraries = true;
    for _ in 0..4 {
        u.frame(vec![]);
        u.advance_clock(60.0);
    }
    assert!(!u.app.session.library_allowed(LINK), "showing the panel allows nothing");
    let shapes = flat(&u.frame_output(vec![]).shapes);
    assert!(has_text(&shapes, "Not checked yet"));
    let at = text_center(&shapes, "Check for updates");
    u.button(at, true);
    u.button(at, false);
    u.frame(vec![]);
    assert!(u.app.session.library_allowed(LINK), "allowed once the user checked");
}
