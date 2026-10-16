//! Session restore: a saved session survives the preferences file (and junk in it never costs the
//! other preferences, nor a panic); `session.restore` opens the tabs again in their order with
//! their page and view and the one in front, skips files that are gone and says which, joins
//! your online files again through their records (a scripted link) and puts each in its place
//! when it arrives; windows keep their own tabs; the last run's session on offer is restored
//! once; the preference toggles; junk parameters are refused or ignored.

use std::sync::{Arc, Mutex};

use figcraft_collab::message::{Admin, AdminReply, ClientMsg, HostInfo};
use serde_json::{Value, json};

use crate::live::{Access, Event, HostOptions, LiveLink, Presence, PublishOptions, Role, SessionInfo, Shared, Status};
use crate::restore::{Arrival, MAX_TABS, MAX_WINDOWS, SavedSession, SavedTab, SavedWindow};
use crate::{Prefs, Session};

const SERVER: &str = "wss://share.example/";

/// A folder of saved files named `names`, and their paths.
fn files(test: &str, names: &[&str]) -> (std::path::PathBuf, Vec<String>) {
    let dir = std::env::temp_dir().join(format!("figcraft-restore-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::empty();
    let paths = names
        .iter()
        .map(|n| {
            let path = dir.join(format!("{n}.figcraft")).to_string_lossy().into_owned();
            s.execute("file.new", &json!({"name": n})).unwrap();
            s.execute("page.add", &json!({"name": "Second"})).unwrap();
            s.execute("page.select", &json!({"index": 0})).unwrap();
            s.execute("file.saveAs", &json!({"path": path})).unwrap();
            path
        })
        .collect();
    (dir, paths)
}

fn tab(path: &str) -> SavedTab {
    SavedTab { path: Some(path.into()), ..SavedTab::default() }
}

fn window(tabs: Vec<SavedTab>, front: usize) -> SavedWindow {
    SavedWindow { tabs, front, ..SavedWindow::default() }
}

fn names(s: &Session) -> Vec<String> {
    s.docs.iter().map(|d| d.doc.name.clone()).collect()
}

/// A session with `saved` kept in its preferences, as a new run has it.
fn run_with(saved: SavedSession) -> Session {
    let mut s = Session::empty();
    s.prefs.session = saved;
    s.prefs = serde_json::from_value(serde_json::to_value(&s.prefs).unwrap()).unwrap();
    s
}

#[test]
fn a_saved_session_survives_the_preferences_file() {
    let saved = SavedSession {
        windows: vec![
            SavedWindow {
                rect: Some([10.0, 20.0, 1200.0, 800.0]),
                maximized: true,
                fullscreen: false,
                monitor: Some([0.0, 0.0, 1920.0, 1080.0]),
                tabs: vec![
                    SavedTab { path: Some("C:/work/a.figcraft".into()), page: 2, view: Some([10.5, -4.0, 2.0]), ..SavedTab::default() },
                    SavedTab { server: Some(SERVER.into()), file: Some("f1".into()), ..SavedTab::default() },
                ],
                front: 1,
                left_tab: "assets".into(),
                right_tab: "inspect".into(),
                left_collapsed: true,
                panel_widths: Some([300.0, 280.0]),
            },
            window(vec![tab("/home/me/b.figcraft")], 0),
        ],
    };
    let s = run_with(saved.clone());
    assert_eq!(s.prefs.session, saved);
    assert_eq!(s.prefs.session.tabs(), 3);
    assert!(s.prefs.restore_session, "on unless turned off");
    // No token is ever in it.
    assert!(!serde_json::to_string(&s.prefs.session).unwrap().contains("owner"));
}

#[test]
fn junk_in_a_saved_session_never_panics_or_costs_the_other_preferences() {
    let deep = (0..200).fold(json!(1), |v, _| json!([v]));
    let junk = [
        json!(null),
        json!(7),
        json!("windows"),
        json!([]),
        json!({"windows": 3}),
        json!({"windows": {"0": {}}}),
        json!({"windows": [null, 1, "w", [], {"tabs": "none"}, {"tabs": [null, 2, {}, {"path": 5}, {"path": ""}, {"file": "f"}, {"server": "http://x/", "file": "f"}]}]}),
        json!({"windows": [{"rect": [0, 0, 0, 0], "tabs": [{"path": "a"}]}, {"rect": [1e30, 0, 800, 600]}, {"rect": ["a", null, {}, []]}, {"rect": [0, 0, 1e9, 1e9]}, {"rect": [0, 0, 800]}]}),
        json!({"windows": [{"front": -4, "maximized": "yes", "fullscreen": 1, "leftTab": 9, "rightTab": "x".repeat(5000), "panelWidths": [1e300, -1e300], "tabs": [{"path": "a", "page": -1, "view": [0, 0]}, {"path": "b", "page": 1e99, "view": [1e308, -1e308, 0]}]}]}),
        json!({"windows": [{"tabs": [{"path": "a\u{0}b"}, {"path": "p".repeat(100_000)}, {"server": "wss://x/", "file": "bad id"}, {"server": "wss://x/", "file": "i".repeat(65)}]}]}),
        json!({"windows": deep}),
    ];
    for j in &junk {
        let read = SavedSession::read(j);
        assert!(read.windows.len() <= MAX_WINDOWS);
        for w in &read.windows {
            assert!(w.tabs.is_empty() || w.front < w.tabs.len(), "{j}");
            assert!(w.rect.is_none_or(|r| r.iter().all(|n| n.is_finite()) && r[2] >= 100.0 && r[3] >= 100.0 && r[2] <= 16_384.0), "{j}");
            for t in &w.tabs {
                assert!(t.path.is_some() || (t.server.is_some() && t.file.is_some()), "{j}");
                assert!(t.view.is_none_or(|v| v.iter().all(|n| n.is_finite()) && v[2] > 0.0), "{j}");
            }
        }
        // The other preferences load whatever the session is.
        let prefs: Prefs = serde_json::from_value(json!({"theme": "dark", "session": j, "restore_session": false})).unwrap();
        assert_eq!((prefs.theme.as_str(), prefs.restore_session, &prefs.session), ("dark", false, &read));
    }
    // Zero-sized and absurd rectangles are dropped, their windows kept.
    let read = SavedSession::read(&junk[7]);
    assert_eq!(read.windows.len(), 5);
    assert!(read.windows.iter().all(|w| w.rect.is_none()));
    // Unusable tabs are skipped; the tab in front keeps its place among the ones kept.
    let read = SavedSession::read(&json!({"windows": [{"front": 3, "tabs": [{"path": "a"}, {}, {"path": "a"}, {"path": "b"}, {"path": "c"}]}]}));
    assert_eq!((read.windows[0].tabs.len(), read.windows[0].front), (3, 1));
    // Huge lists are cut to the limits.
    let many: Vec<Value> = (0..600).map(|i| json!({"path": format!("f{i}")})).collect();
    let windows: Vec<Value> = (0..200).map(|_| json!({"tabs": many})).collect();
    let read = SavedSession::read(&json!({"windows": windows}));
    assert_eq!((read.windows.len(), read.windows[0].tabs.len()), (MAX_WINDOWS, MAX_TABS));
    // A preferences file cut off anywhere is an error to read, never a panic.
    let whole = serde_json::to_string(&json!({"prefs": run_with(read).prefs})).unwrap();
    for cut in (0..whole.len().min(4000)).step_by(7).filter(|c| whole.is_char_boundary(*c)) {
        let _ = serde_json::from_str::<Value>(&whole[..cut]).map(|v| serde_json::from_value::<Prefs>(v["prefs"].clone()));
    }
}

#[test]
fn tabs_come_back_in_order_with_their_page_view_and_the_one_in_front() {
    let (dir, paths) = files("order", &["A", "B", "C"]);
    let saved = SavedSession {
        windows: vec![window(vec![tab(&paths[2]), SavedTab { page: 1, view: Some([40.0, -30.0, 3.0]), ..tab(&paths[0]) }, tab(&paths[1])], 1)],
    };
    let mut s = run_with(saved);
    let r = s.execute("session.restore", &json!({})).unwrap();
    assert_eq!(r, json!({"windows": 1, "opened": 3, "joining": 0, "missing": []}));
    assert_eq!(names(&s), ["C", "A", "B"]);
    let front = s.ds().unwrap();
    assert_eq!(front.doc.name, "A", "the tab in front");
    assert_eq!(front.doc.pages.iter().position(|p| *p == front.page), Some(1), "its page");
    assert_eq!((front.view.cx, front.view.cy, front.view.zoom), (40.0, -30.0, 3.0), "its view");
    assert!(!front.is_dirty());
    // What the interface places: the window with its documents and the one in front.
    let (placed, missing) = s.restore.placing.take().unwrap();
    assert_eq!((placed.len(), missing), (1, 0));
    assert_eq!(placed[0].docs, s.docs.iter().map(|d| d.uid).collect::<Vec<_>>());
    assert_eq!(placed[0].front, Some(s.docs[1].uid));
    assert!(s.ui_requests.iter().any(|u| u.kind == "session.restore"));
    // The open tabs as a saved session again.
    let tabs: Vec<SavedTab> = s.docs.iter().filter_map(|d| s.saved_tab(d)).collect();
    assert_eq!(tabs.iter().map(|t| t.path.clone().unwrap()).collect::<Vec<_>>(), [paths[2].clone(), paths[0].clone(), paths[1].clone()]);
    assert_eq!((tabs[1].page, tabs[1].view), (1, Some([40.0, -30.0, 3.0])));
    // Restoring again opens nothing twice; files open already are put in order.
    s.execute("doc.switch", &json!({"index": 2})).unwrap();
    let again = json!({"windows": [{"tabs": [{"path": paths[1]}, {"path": paths[0]}], "front": 0}]});
    let r = s.execute("session.restore", &json!({"session": again})).unwrap();
    assert_eq!((&r["opened"], names(&s)), (&json!(0), vec!["B".to_string(), "A".into(), "C".into()]));
    assert_eq!(s.ds().unwrap().doc.name, "B");
    // A document that was never saved isn't a tab of a saved session.
    s.execute("file.new", &json!({})).unwrap();
    assert_eq!(s.saved_tab(s.ds().unwrap()), None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn files_that_are_gone_are_skipped_and_named() {
    let (dir, paths) = files("missing", &["A", "B", "C"]);
    std::fs::remove_file(&paths[0]).unwrap();
    std::fs::write(&paths[2], b"not a file of ours").unwrap();
    let saved = SavedSession { windows: vec![window(paths.iter().map(|p| tab(p)).collect(), 0)] };
    let mut s = run_with(saved);
    let r = s.execute("session.restore", &json!({})).unwrap();
    assert_eq!(r, json!({"windows": 1, "opened": 1, "joining": 0, "missing": ["A.figcraft", "C.figcraft"]}));
    assert_eq!(names(&s), ["B"]);
    assert_eq!(s.restore.placing.as_ref().map(|p| p.1), Some(2), "the interface says how many");
    // The tab in front was gone: the first one that opened shows.
    assert_eq!(s.ds().unwrap().doc.name, "B");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn windows_keep_their_own_tabs() {
    let (dir, paths) = files("windows", &["A", "B", "C"]);
    let saved = SavedSession {
        windows: vec![
            SavedWindow { rect: Some([0.0, 0.0, 1000.0, 700.0]), ..window(vec![tab(&paths[0])], 0) },
            // (A file listed in two windows is a tab of the first.)
            SavedWindow {
                rect: Some([200.0, 100.0, 900.0, 600.0]),
                maximized: true,
                ..window(vec![tab(&paths[1]), tab(&paths[2]), tab(&paths[0])], 1)
            },
        ],
    };
    let mut s = run_with(saved.clone());
    let r = s.execute("session.restore", &json!({})).unwrap();
    assert_eq!((&r["windows"], &r["opened"]), (&json!(2), &json!(3)));
    let (placed, _) = s.restore.placing.take().unwrap();
    let uid = |n: &str| s.docs.iter().find(|d| d.doc.name == n).unwrap().uid;
    assert_eq!((placed[0].docs.clone(), placed[0].front), (vec![uid("A")], Some(uid("A"))));
    assert_eq!((placed[1].docs.clone(), placed[1].front), (vec![uid("B"), uid("C")], Some(uid("C"))));
    assert_eq!(placed[1].saved, saved.windows[1]);
    assert_eq!(s.ds().unwrap().doc.name, "A", "the main window's tab in front is the session's");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_last_runs_session_on_offer_is_restored_once() {
    let (dir, paths) = files("offer", &["A", "B"]);
    // This run started without restoring: its own session is what the preferences hold now.
    let mut s = run_with(SavedSession { windows: vec![window(vec![tab(&paths[1])], 0)] });
    s.restore.last = Some(SavedSession { windows: vec![window(vec![tab(&paths[0])], 0)] });
    s.execute("session.restore", &json!({})).unwrap();
    assert_eq!(names(&s), ["A"]);
    assert!(s.restore.last.is_none());
    s.execute("session.restore", &json!({})).unwrap();
    assert_eq!(names(&s), ["B", "A"], "then the one kept with the preferences");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_preference_toggles_and_is_kept() {
    let mut s = Session::empty();
    assert_eq!(s.execute("pref.restoreSession", &json!({"on": false})).unwrap(), json!({"on": false}));
    let prefs: Prefs = serde_json::from_value(serde_json::to_value(&s.prefs).unwrap()).unwrap();
    assert!(!prefs.restore_session);
    assert_eq!(s.execute("pref.restoreSession", &json!({})).unwrap(), json!({"on": true}));
}

#[test]
fn junk_parameters_are_refused_or_ignored() {
    let (dir, paths) = files("sweep", &["A"]);
    let mut s = run_with(SavedSession { windows: vec![window(vec![tab(&paths[0])], 0)] });
    for j in [json!(7), json!("all"), json!([1, 2]), json!(true)] {
        assert!(s.execute("session.restore", &json!({"session": j})).unwrap_err().to_string().contains("saved session"));
        assert!(s.execute("pref.restoreSession", &json!({"on": j})).is_ok());
    }
    assert!(s.docs.is_empty() && s.restore.placing.is_none());
    for j in [json!({}), json!({"windows": 5}), json!({"windows": [{"tabs": [{"path": 3}, {"path": "no/such/file.figcraft"}]}]})] {
        let r = s.execute("session.restore", &json!({"session": j})).unwrap();
        assert_eq!(r["opened"], 0, "{j}");
    }
    assert!(s.docs.is_empty());
    // Nothing saved: nothing to do, and nothing asked of the interface.
    let mut empty = Session::empty();
    assert_eq!(empty.execute("session.restore", &json!(null)).unwrap(), json!({"windows": 0, "opened": 0, "joining": 0, "missing": []}));
    assert!(empty.ui_requests.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

/// What the scripted server was asked to join and the events its link hands out next.
#[derive(Default)]
struct Script {
    joined: Vec<(u64, String)>,
    events: Vec<(u64, Event)>,
}

struct Link(Arc<Mutex<Script>>);

impl LiveLink for Link {
    fn host(&mut self, _: u64, _: HostOptions) -> Result<SessionInfo, String> {
        Err("not in this test".into())
    }
    fn join(&mut self, session: u64, link: &str, _: &str) -> Result<(), String> {
        self.0.lock().unwrap().joined.push((session, link.to_string()));
        Ok(())
    }
    fn publish(&mut self, _: u64, _: PublishOptions) -> Result<(), String> {
        Err("not in this test".into())
    }
    fn hosting(&mut self, _: &str, _: &str, _: &str, _: Option<Admin>) -> Result<(HostInfo, Option<AdminReply>), String> {
        Err("not in this test".into())
    }
    fn share(&mut self, _: &str, _: &str, _: &str, _: &[String], _: bool) -> Result<Option<Shared>, String> {
        Err("not in this test".into())
    }
    fn leave(&mut self, _: u64) {}
    fn presence(&mut self, _: u64, _: &Presence) {}
    fn send(&mut self, _: u64, _: ClientMsg) {}
    fn poll(&mut self) -> Vec<(u64, Event)> {
        std::mem::take(&mut self.0.lock().unwrap().events)
    }
}

fn edit_link(file: &str) -> String {
    format!("figcraft://join?server={SERVER}&file={file}&token=EditSecretEditSecret00")
}

/// The record of online file `file`, as the preferences file holds it.
fn record(file: &str) -> Value {
    json!({"server": SERVER, "file": file, "name": format!("Online {file}"), "owner": "OwnerSecretOwnerSecret", "edit": edit_link(file), "view": "", "created": 1, "opened": 1})
}

/// The file of session `session` arrives from the scripted server.
fn arrive(script: &Arc<Mutex<Script>>, session: u64, file: &str) {
    let info = SessionInfo {
        role: Role::Guest,
        me: 1,
        name: "Me".into(),
        color: "#0D99FF".into(),
        file_name: format!("Online {file}"),
        pages: vec![],
        link: edit_link(file),
        url: String::new(),
        address: SERVER.into(),
        network: true,
        access: Access::Edit,
    };
    let (_, _, sync) = crate::live_tests::serve(&Session::new().doc().unwrap().clone());
    script.lock().unwrap().events.extend([(session, Event::Welcome { info, peers: vec![] }), (session, sync)]);
}

#[test]
fn online_files_are_joined_again_and_take_their_place_when_they_arrive() {
    let (dir, paths) = files("online", &["A", "B"]);
    let online =
        |file: &str| SavedTab { server: Some(SERVER.into()), file: Some(file.into()), page: 0, view: Some([5.0, 6.0, 1.5]), ..SavedTab::default() };
    let saved =
        SavedSession { windows: vec![window(vec![tab(&paths[0]), online("f1"), tab(&paths[1])], 1), window(vec![online("f2"), online("gone")], 0)] };
    let mut s = run_with(saved);
    s.prefs.online_files = serde_json::from_value::<Prefs>(json!({"online_files": [record("f1"), record("f2")]})).unwrap().online_files;
    let script = Arc::new(Mutex::new(Script::default()));
    s.live.install(Box::new(Link(script.clone())));
    let r = s.execute("session.restore", &json!({})).unwrap();
    // The one no longer recorded on this computer counts as gone.
    assert_eq!(r, json!({"windows": 2, "opened": 2, "joining": 2, "missing": ["gone"]}));
    assert!(!r.to_string().contains("Secret"));
    let joined = script.lock().unwrap().joined.clone();
    assert_eq!(joined.iter().map(|j| j.1.clone()).collect::<Vec<_>>(), [edit_link("f1"), edit_link("f2")], "as their owner, with the recorded links");
    assert_eq!(names(&s), ["A", "B"]);
    // Until they arrive they are still tabs of their windows.
    let waiting: Vec<(usize, Option<String>)> = s.restore.joining().map(|(k, t)| (k, t.file.clone())).collect();
    assert_eq!(waiting, [(0, Some("f1".into())), (1, Some("f2".into()))]);
    assert!(s.restore_arrivals().is_empty());

    // The first arrives: between A and B, in front of its window, with its view.
    arrive(&script, joined[0].0, "f1");
    s.live_poll();
    let arrived = s.restore_arrivals();
    let uid = s.docs.iter().find(|d| d.path.is_none()).unwrap().uid;
    assert_eq!(arrived, [Arrival { uid, window: 0, front: true }]);
    assert_eq!(s.docs.iter().map(|d| d.path.is_some()).collect::<Vec<_>>(), [true, false, true]);
    let ds = s.docs.iter().find(|d| d.uid == uid).unwrap();
    assert_eq!((ds.view.cx, ds.view.cy, ds.view.zoom), (5.0, 6.0, 1.5));
    assert_eq!(s.saved_tab(ds).map(|t| (t.server, t.file)), Some((Some(SERVER.into()), Some("f1".into()))), "a tab of the next saved session");
    assert_eq!(s.restore.joining().count(), 1);

    // The other's server can't be reached: it is skipped, and the message says so.
    script.lock().unwrap().events.push((joined[1].0, Event::Status(Status::Failed { reason: "no answer from the server (timed out)".into() })));
    s.live_poll();
    assert!(s.restore_arrivals().is_empty());
    assert_eq!(s.restore.joining().count(), 0);
    assert_eq!(s.message.as_deref(), Some("f2 couldn't be reopened from its server: no answer from the server (timed out)"));
    assert_eq!(s.docs.len(), 3);
    let _ = std::fs::remove_dir_all(dir);
}

/// Two online tabs of one window keep their saved order however they arrive: together in one
/// poll, or the later one first.
#[test]
fn two_online_tabs_of_one_window_keep_their_saved_order() {
    let online =
        |file: &str| SavedTab { server: Some(SERVER.into()), file: Some(file.into()), ..SavedTab::default() };
    let order = |s: &Session| {
        s.docs
            .iter()
            .map(|d| s.saved_tab(d).and_then(|t| t.path.or(t.file)).unwrap_or_default())
            .collect::<Vec<_>>()
    };
    let setup = || {
        let (dir, paths) = files("online-order", &["A"]);
        let saved = SavedSession { windows: vec![window(vec![tab(&paths[0]), online("f1"), online("f2")], 0)] };
        let mut s = run_with(saved);
        s.prefs.online_files =
            serde_json::from_value::<Prefs>(json!({"online_files": [record("f1"), record("f2")]})).unwrap().online_files;
        let script = Arc::new(Mutex::new(Script::default()));
        s.live.install(Box::new(Link(script.clone())));
        let r = s.execute("session.restore", &json!({})).unwrap();
        assert_eq!(r, json!({"windows": 1, "opened": 1, "joining": 2, "missing": []}));
        let joined = script.lock().unwrap().joined.clone();
        assert_eq!(joined.len(), 2);
        (dir, paths, s, script, joined)
    };

    // Both ready in the same poll: each lands after the tab before it.
    let (dir, paths, mut s, script, joined) = setup();
    arrive(&script, joined[0].0, "f1");
    arrive(&script, joined[1].0, "f2");
    s.live_poll();
    let arrived = s.restore_arrivals();
    assert_eq!(arrived.len(), 2);
    assert!(arrived.iter().all(|a| a.window == 0));
    assert_eq!(order(&s), [paths[0].clone(), "f1".into(), "f2".into()]);
    assert_eq!(s.restore.joining().count(), 0);
    let _ = std::fs::remove_dir_all(dir);

    // The later one first, each in its own poll: the earlier one still lands before it.
    let (dir, paths, mut s, script, joined) = setup();
    arrive(&script, joined[1].0, "f2");
    s.live_poll();
    assert_eq!(s.restore_arrivals().len(), 1);
    arrive(&script, joined[0].0, "f1");
    s.live_poll();
    assert_eq!(s.restore_arrivals().len(), 1);
    assert_eq!(order(&s), [paths[0].clone(), "f1".into(), "f2".into()]);
    assert_eq!(s.restore.joining().count(), 0);
    let _ = std::fs::remove_dir_all(dir);
}
