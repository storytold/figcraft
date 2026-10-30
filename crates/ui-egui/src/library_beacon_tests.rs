//! The Assets panel showing a file that names a team library on an unknown server doesn't
//! contact it (#1097): the library waits until the user clicks Check for updates.

use std::sync::{Arc, Mutex};

use figcraft_doc::LibraryLink;
use figcraft_engine::Session;
use figcraft_engine::figcraft_collab::message::{ClientMsg, LibraryReq};
use figcraft_engine::live::{Event, HostOptions, LiveLink, Presence, SessionInfo};

use crate::layers_drag_tests::Ui;
use crate::live_tests::{flat, text_center};

const LINK: &str = "ws://tracker.example:9/?file=f&token=per-file-secret";

/// The team library links the app asked about.
struct Link(Arc<Mutex<Vec<String>>>);

impl LiveLink for Link {
    fn host(&mut self, _: u64, _: HostOptions) -> Result<SessionInfo, String> {
        Err("no".into())
    }
    fn join(&mut self, _: u64, _: &str, _: &str) -> Result<(), String> {
        Err("no".into())
    }
    fn leave(&mut self, _: u64) {}
    fn presence(&mut self, _: u64, _: &Presence) {}
    fn send(&mut self, _: u64, _: ClientMsg) {}
    fn poll(&mut self) -> Vec<(u64, Event)> {
        Vec::new()
    }
    fn library_background(&mut self, jobs: Vec<(String, LibraryReq)>) {
        self.0.lock().unwrap().extend(jobs.into_iter().map(|(l, _)| l));
    }
}

#[test]
fn the_assets_panel_contacts_no_unknown_library_server_until_asked() {
    let mut s = Session::new();
    s.doc_mut().unwrap().libraries_mut().enabled.push(LibraryLink { id: "lib".into(), name: "Kit".into(), path: LINK.into(), version: 1 });
    let asked = Arc::new(Mutex::new(Vec::new()));
    s.live.install(Box::new(Link(asked.clone())));
    let mut u = Ui::new(s);
    u.app.ui.left_tab = crate::LeftTab::Assets;
    u.app.ui.libraries = true;
    for _ in 0..4 {
        u.frame(vec![]);
        u.advance_clock(60.0);
    }
    assert!(asked.lock().unwrap().is_empty(), "no connection attempt");
    let shapes = flat(&u.frame_output(vec![]).shapes);
    let at = text_center(&shapes, "Check for updates");
    u.button(at, true);
    u.button(at, false);
    u.frame(vec![]);
    assert_eq!(*asked.lock().unwrap(), vec![LINK.to_string()], "asked once the user checked");
}
