//! Home ▸ Online in the window: nothing is asked of a server until the section opens, then each
//! server once; the rows (name, where, shared or not, when opened); a server that can't be
//! reached; opening a file as its owner (its tab shows when it arrives); the row menu (stop and
//! start sharing, copy the code and the link, rename, remove with its confirmation, forget); a
//! file that is gone; the marks on tabs of files in a live session; toasts that name a file that
//! isn't in front; and an idle section asking for no frames.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use egui::{Event, Key, Shape, pos2};
use figcraft_engine::Prefs;
use figcraft_engine::figcraft_collab::message::{Admin, AdminReply, ClientMsg, FileInfo, OwnedFile, TeamFile};
use figcraft_engine::live::{
    Access, Event as LiveEvent, HostInfo, HostOptions, LiveLink, NetReply, NetRequest, Peer, Presence, PublishOptions, Role, SessionInfo,
    Shared as SharedLinks, SharedLink,
};
use serde_json::{Value, json};

use crate::live_tests::{Harness, has_text, text_center, texts};

const SERVER: &str = "wss://share.example/";
const OTHER: &str = "ws://127.0.0.1:47900/";
const OWNER: &str = "OwnerSecretOwnerSecret";
const EDIT_TOKEN: &str = "EditSecretEditSecret00";

/// What the scripted servers keep and were asked, and the events the link hands out next.
#[derive(Default)]
struct Script {
    hosted: usize,
    joined: Vec<String>,
    /// Hosting requests: the server and what was asked.
    asked: Vec<(String, Option<Admin>)>,
    /// Share requests: (file, owner, revoke, create).
    shares: Vec<(String, String, Vec<String>, bool)>,
    /// Ids of the files on the servers.
    files: Vec<String>,
    offline: bool,
    answers: Vec<LiveEvent>,
    events: Vec<LiveEvent>,
    session: u64,
    copied: Vec<String>,
    /// The team list the server keeps (its browser build's address and files), if any.
    team: Option<(String, Vec<TeamFile>)>,
}

type Shared = Arc<Mutex<Script>>;

struct Link(Shared);

fn info(role: Role, name: &str) -> SessionInfo {
    SessionInfo {
        role,
        me: 1,
        name: "Ada".into(),
        color: "#14AE5C".into(),
        file_name: name.into(),
        pages: vec![],
        link: "figcraft://join?host=192.168.1.20:47900&file=f&token=AbCdEfGhIjKlMnOpQrStUv".into(),
        url: String::new(),
        address: "192.168.1.20:47900".into(),
        network: true,
        access: Access::Edit,
    }
}

/// The links a server makes for others the `n`th time a file is shared.
fn links(file: &str, n: usize) -> SharedLinks {
    let link = |what: &str, code: String| SharedLink {
        id: format!("{what}{n}"),
        link: format!("figcraft://join?server={SERVER}&file={file}&token=Shared{what}Shared{what}0{n}"),
        code: Some(code),
    };
    SharedLinks { edit: link("edit", format!("NEWED1T0{n}")), view: Some(link("view", format!("NEWV1EW0{n}"))), comment: None }
}

impl LiveLink for Link {
    fn host(&mut self, session: u64, _: HostOptions) -> Result<SessionInfo, String> {
        let mut s = self.0.lock().unwrap();
        s.session = session;
        s.hosted += 1;
        Ok(info(Role::Host, "Untitled"))
    }
    fn join(&mut self, session: u64, link: &str, _: &str) -> Result<(), String> {
        let mut s = self.0.lock().unwrap();
        s.session = session;
        s.joined.push(link.to_string());
        Ok(())
    }
    fn publish(&mut self, _: u64, _: PublishOptions) -> Result<(), String> {
        Err("not in this test".into())
    }
    fn hosting(&mut self, server: &str, _: &str, _: &str, req: Option<Admin>) -> Result<(HostInfo, Option<AdminReply>), String> {
        let mut s = self.0.lock().unwrap();
        s.asked.push((server.to_string(), req.clone()));
        if s.offline {
            return Err("no answer from the server (timed out)".into());
        }
        let reply = match req {
            Some(Admin::Owned { files }) => {
                let known = |(file, _): &(String, String)| {
                    s.files.contains(file).then(|| OwnedFile {
                        name: format!("File {file}"),
                        links: vec!["edit0".into(), "view0".into()],
                        expires: None,
                    })
                };
                AdminReply::Owned(files.iter().map(known).collect())
            }
            Some(Admin::Remove { file }) => {
                s.files.retain(|f| *f != file);
                AdminReply::Done
            }
            Some(Admin::Team) => match s.team.clone() {
                Some((web, files)) => AdminReply::Team { web, files },
                None => AdminReply::Failed { reason: "this server keeps no team list".into() },
            },
            _ => AdminReply::Done,
        };
        Ok((HostInfo::default(), Some(reply)))
    }
    fn share(&mut self, _: &str, file: &str, owner: &str, revoke: &[String], create: bool) -> Result<Option<SharedLinks>, String> {
        let mut s = self.0.lock().unwrap();
        s.shares.push((file.into(), owner.into(), revoke.to_vec(), create));
        let n = s.shares.iter().filter(|a| a.3).count();
        Ok(create.then(|| links(file, n)))
    }
    fn background(&mut self, job: u64, req: NetRequest) -> bool {
        // Answered at the next poll, as the network would.
        let reply: NetReply = self.request(req);
        self.0.lock().unwrap().answers.push(LiveEvent::Net { job, reply: Box::new(reply) });
        true
    }
    fn leave(&mut self, _: u64) {}
    fn presence(&mut self, _: u64, _: &Presence) {}
    fn send(&mut self, _: u64, _: ClientMsg) {}
    fn poll(&mut self) -> Vec<(u64, LiveEvent)> {
        let mut s = self.0.lock().unwrap();
        let session = s.session;
        let answers: Vec<(u64, LiveEvent)> = std::mem::take(&mut s.answers).into_iter().map(|e| (0, e)).collect();
        answers.into_iter().chain(std::mem::take(&mut s.events).into_iter().map(|e| (session, e))).collect()
    }
}

/// The clipboard, kept where the test can read it.
struct Clipboard(Shared);

impl crate::Services for Clipboard {
    fn clipboard_set_text(&mut self, s: &str) {
        self.0.lock().unwrap().copied.push(s.to_string());
    }
}

fn edit_link(server: &str, file: &str) -> String {
    format!("figcraft://join?server={server}&file={file}&token={EDIT_TOKEN}")
}

/// A record as the preferences file holds it: `File <id>`, shared with a code or not.
fn record(server: &str, file: &str, shared: bool) -> Value {
    let mut r = json!({"server": server, "file": file, "name": format!("File {file}"), "owner": OWNER, "edit": edit_link(server, file), "created": 1_791_800_000u64, "opened": 1_791_800_000u64});
    if shared {
        r["shared"] = serde_json::to_value(links(file, 0)).unwrap();
    }
    r
}

/// A window on the home screen (Recents), with these records loaded from the preferences and all
/// of them on their servers.
fn harness(records: Vec<Value>) -> (Harness, Shared) {
    let (mut h, _) = Harness::new();
    let script = Shared::default();
    script.lock().unwrap().files = records.iter().filter_map(|r| r["file"].as_str().map(str::to_string)).collect();
    h.app.session.live.install(Box::new(Link(script.clone())));
    h.app.services = Box::new(Clipboard(script.clone()));
    h.app.session.prefs = serde_json::from_value::<Prefs>(json!({"online_files": records})).unwrap();
    h.app.ui.home = true;
    h.frames(3);
    (h, script)
}

/// Open the Online section from the sidebar; what it shows once the servers answered.
fn open(h: &mut Harness) -> Vec<Shape> {
    let shapes = h.frames(1);
    h.click(text_center(&shapes, "Online"));
    h.frames(4)
}

fn click(h: &mut Harness, shapes: &[Shape], text: &str) -> Vec<Shape> {
    h.click(text_center(shapes, text));
    h.frames(4)
}

/// Open the menu of the row of the file called `name`.
fn menu(h: &mut Harness, name: &str) -> Vec<Shape> {
    let shapes = h.frames(1);
    // The button at the row's end (the list is 720 wide from x = 280).
    h.click(pos2(280.0 + 720.0 - 18.0, text_center(&shapes, name).y));
    h.frames(3)
}

fn toast(h: &Harness) -> String {
    h.app.ui.toast.as_ref().map(|t| t.0.clone()).unwrap_or_default()
}

fn owned_requests(script: &Shared) -> Vec<(String, usize)> {
    let s = script.lock().unwrap();
    s.asked
        .iter()
        .filter_map(|(server, req)| if let Some(Admin::Owned { files }) = req { Some((server.clone(), files.len())) } else { None })
        .collect()
}

#[test]
fn nothing_is_asked_until_the_section_opens_then_each_server_once() {
    let (mut h, script) = harness(vec![record(SERVER, "a", true), record(OTHER, "b", false), record(SERVER, "c", false)]);
    let shapes = h.frames(2);
    assert!(has_text(&shapes, "Recents") && has_text(&shapes, "Online") && !has_text(&shapes, "Online files"));
    assert!(script.lock().unwrap().asked.is_empty(), "the home screen alone asks nothing");
    let shapes = open(&mut h);
    assert!(has_text(&shapes, "Online files") && !has_text(&shapes, "Open files"));
    assert_eq!(owned_requests(&script), [(SERVER.to_string(), 2), (OTHER.to_string(), 1)]);
    for t in ["File a", "File b", "File c", "share.example · Shared · Opened 2026-10-12", "127.0.0.1:47900 · Not shared · Opened 2026-10-12"] {
        assert!(has_text(&shapes, t), "the list shows {t:?}: {:?}", texts(&shapes).iter().map(|t| t.0.clone()).collect::<Vec<_>>());
    }
    let inspect = h.app.inspect();
    assert_eq!(
        (&inspect["online"]["open"], &inspect["online"]["files"][0]["state"], &inspect["online"]["files"][0]["hasOwnerToken"]),
        (&json!(true), &json!("online"), &json!(true))
    );
    for secret in [OWNER, EDIT_TOKEN] {
        assert!(!inspect.to_string().contains(secret), "ui.inspect carries a secret");
    }
    // Staying there asks nothing more, and an idle section asks for no frames.
    assert!(h.goes_quiet());
    let (_, delay) = h.frame(vec![]);
    assert!(delay >= Duration::from_secs(1));
    assert_eq!(script.lock().unwrap().asked.len(), 2);
    // Leaving and coming back asks again, once.
    let shapes = h.frames(1);
    let shapes = click(&mut h, &shapes, "Recents");
    assert!(has_text(&shapes, "Open files"));
    open(&mut h);
    assert_eq!(script.lock().unwrap().asked.len(), 4);
}

#[test]
fn an_empty_list_says_how_files_get_there_and_asks_no_server() {
    let (mut h, script) = harness(vec![]);
    let shapes = open(&mut h);
    assert!(has_text(&shapes, "Nothing online yet. Use Share on a file to put it online and it is listed here."));
    assert!(script.lock().unwrap().asked.is_empty());
}

#[test]
fn a_server_that_cant_be_reached_says_so_and_the_list_still_shows() {
    let (mut h, script) = harness(vec![record(SERVER, "a", true)]);
    script.lock().unwrap().offline = true;
    let shapes = open(&mut h);
    assert!(has_text(&shapes, "Couldn't reach share.example: showing what this computer remembers."));
    assert!(has_text(&shapes, "File a") && has_text(&shapes, "Try again"));
    assert_eq!(h.app.inspect()["online"]["files"][0]["state"], "unknown");
    // Back: Try again asks once more and the note goes.
    script.lock().unwrap().offline = false;
    let shapes = click(&mut h, &shapes, "Try again");
    assert!(!has_text(&shapes, "Try again") && has_text(&shapes, "File a"));
    assert_eq!(owned_requests(&script).len(), 2);
}

#[test]
fn a_row_opens_its_file_as_owner_and_its_tab_shows_when_it_arrives() {
    let (mut h, script) = harness(vec![record(SERVER, "a", true)]);
    let shapes = open(&mut h);
    let shapes = click(&mut h, &shapes, "File a");
    assert_eq!(script.lock().unwrap().joined, [edit_link(SERVER, "a")]);
    assert!(has_text(&shapes, "Joining…") && h.app.ui.home, "still on the home screen while it joins");
    // The owner's welcome and the document.
    use figcraft_engine::figcraft_collab::message::ServerMsg;
    use figcraft_engine::figcraft_collab::{Client, Limits, Outgoing, Server};
    let mut server = Server::new(h.app.session.doc().unwrap().clone(), "", Limits::default());
    let ClientMsg::Hello(me) = Client::hello("f", "", "Ada", "") else { panic!("no hello") };
    let (_, outs) = server.join(me).unwrap();
    let w = outs.into_iter().find_map(|o| if let Outgoing::To(_, w @ ServerMsg::Welcome { .. }) = o { Some(w) } else { None }).unwrap();
    script.lock().unwrap().events.extend([LiveEvent::Welcome { info: info(Role::Guest, "File a"), peers: vec![] }, LiveEvent::Sync(w)]);
    h.frames(3);
    assert!(!h.app.ui.home && h.app.session.active == 1, "its tab is in front");
    assert_eq!(h.app.inspect()["tabs"], json!([{"name": "Untitled", "live": null}, {"name": "Untitled", "live": "shared"}]));
    // Clicking the row again brings that tab back instead of joining twice.
    h.app.ui.home = true;
    let shapes = h.frames(2);
    click(&mut h, &shapes, "File a");
    assert!(!h.app.ui.home && script.lock().unwrap().joined.len() == 1);
}

#[test]
fn the_row_menu_stops_and_starts_sharing_and_copies_the_code_and_link() {
    let (mut h, script) = harness(vec![record(SERVER, "a", true)]);
    open(&mut h);
    let shapes = menu(&mut h, "File a");
    for t in ["Open", "Stop sharing", "Copy code", "Copy link", "Rename on the server…", "Remove from server…", "Forget on this computer"] {
        assert!(has_text(&shapes, t), "the menu has {t:?}");
    }
    click(&mut h, &shapes, "Copy code");
    assert_eq!(script.lock().unwrap().copied, ["NEW-ED1-T00"]);
    assert_eq!(toast(&h), "Join code copied");
    let shapes = menu(&mut h, "File a");
    click(&mut h, &shapes, "Copy link");
    assert_eq!(script.lock().unwrap().copied.last(), Some(&links("a", 0).edit.link));
    // Stop: the others' links are revoked with the recorded token; the row says so.
    let shapes = menu(&mut h, "File a");
    let shapes = click(&mut h, &shapes, "Stop sharing");
    assert_eq!(script.lock().unwrap().shares, [("a".to_string(), OWNER.to_string(), vec!["edit0".to_string(), "view0".to_string()], false)]);
    assert!(has_text(&shapes, "share.example · Not shared · Opened 2026-10-12"));
    assert_eq!(toast(&h), "Sharing stopped: the file stays on the server, open only to you");
    // Not shared: nothing to copy; Start sharing makes a new code.
    let shapes = menu(&mut h, "File a");
    assert!(has_text(&shapes, "Start sharing") && !has_text(&shapes, "Copy code") && !has_text(&shapes, "Copy link"));
    let shapes = click(&mut h, &shapes, "Start sharing");
    assert!(has_text(&shapes, "share.example · Shared · Opened 2026-10-12"));
    let shapes = menu(&mut h, "File a");
    click(&mut h, &shapes, "Copy code");
    assert_eq!(script.lock().unwrap().copied.last().map(String::as_str), Some("NEW-ED1-T01"));
}

#[test]
fn remove_from_server_asks_first_and_forget_only_drops_the_row() {
    let (mut h, script) = harness(vec![record(SERVER, "a", true), record(SERVER, "b", false)]);
    open(&mut h);
    let shapes = menu(&mut h, "File a");
    let shapes = click(&mut h, &shapes, "Remove from server…");
    assert!(has_text(&shapes, "Remove File a from share.example?"));
    assert!(has_text(
        &shapes,
        "The file is deleted from the server: nobody can open or join it from there again, and this can't be undone. Copies that are open stay with whoever has them."
    ));
    assert_eq!(h.app.inspect()["online"]["confirmRemove"], "File a");
    // Cancel: nothing happened.
    let shapes = click(&mut h, &shapes, "Cancel");
    assert!(!has_text(&shapes, "Remove File a from share.example?") && script.lock().unwrap().files.len() == 2);
    // Escape closes it too.
    let shapes = menu(&mut h, "File a");
    click(&mut h, &shapes, "Remove from server…");
    for pressed in [true, false] {
        h.frame(vec![Event::Key { key: Key::Escape, physical_key: None, pressed, repeat: false, modifiers: Default::default() }]);
    }
    assert!(h.app.inspect()["online"]["confirmRemove"].is_null());
    // Remove: off the server and out of the list.
    let shapes = menu(&mut h, "File a");
    let shapes = click(&mut h, &shapes, "Remove from server…");
    let shapes = click(&mut h, &shapes, "Remove");
    assert_eq!(script.lock().unwrap().files, ["b"]);
    assert!(!has_text(&shapes, "File a") && has_text(&shapes, "File b"));
    assert_eq!(toast(&h), "File a was removed from the server");
    // Forget: the row goes, the server isn't asked, and the toast says the file stays.
    let before = script.lock().unwrap().asked.len();
    let shapes = menu(&mut h, "File b");
    let shapes = click(&mut h, &shapes, "Forget on this computer");
    assert!(!has_text(&shapes, "File b") && h.app.session.prefs.online_files.is_empty());
    assert_eq!(script.lock().unwrap().asked.len(), before);
    assert_eq!(script.lock().unwrap().files, ["b"], "the file stays on the server");
    assert_eq!(toast(&h), "File b is forgotten on this computer; the file stays on share.example");
}

#[test]
fn rename_on_the_server_takes_a_name() {
    let (mut h, script) = harness(vec![record(SERVER, "a", false)]);
    open(&mut h);
    let shapes = menu(&mut h, "File a");
    let shapes = click(&mut h, &shapes, "Rename on the server…");
    assert!(has_text(&shapes, "Rename on the server") && has_text(&shapes, "Rename"));
    // The field has the keyboard: select everything in it and type over it.
    for pressed in [true, false] {
        h.frame(vec![Event::Key { key: Key::A, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::COMMAND }]);
    }
    h.frame(vec![Event::Text("Landing".into())]);
    let shapes = h.frames(2);
    let shapes = click(&mut h, &shapes, "Rename");
    assert!(script.lock().unwrap().asked.iter().any(|(_, r)| matches!(r, Some(Admin::Rename { file, name }) if file == "a" && name == "Landing")));
    assert!(has_text(&shapes, "Landing") && h.app.inspect()["online"]["rename"].is_null());
}

#[test]
fn a_file_that_is_gone_says_so_and_can_only_be_cleared() {
    let (mut h, script) = harness(vec![record(SERVER, "a", true), record(SERVER, "b", false)]);
    script.lock().unwrap().files = vec!["b".into()];
    let shapes = open(&mut h);
    assert!(has_text(&shapes, "share.example · No longer on the server"));
    // A click doesn't try to open it.
    click(&mut h, &shapes, "File a");
    assert!(script.lock().unwrap().joined.is_empty());
    let shapes = menu(&mut h, "File a");
    assert!(has_text(&shapes, "Clear from this list") && !has_text(&shapes, "Open") && !has_text(&shapes, "Remove from server…"));
    let shapes = click(&mut h, &shapes, "Clear from this list");
    assert!(!has_text(&shapes, "File a") && has_text(&shapes, "File b"));
    assert_eq!(toast(&h), "File a cleared from the list");
}

#[test]
fn tabs_mark_files_in_a_live_session_and_toasts_name_a_file_that_isnt_in_front() {
    let (mut h, script) = harness(vec![]);
    h.app.ui.home = false;
    h.app.session.execute("file.rename", &json!({"name": "Homepage"})).unwrap();
    assert_eq!(h.app.inspect()["tabs"], json!([{"name": "Homepage", "live": null}]));
    // Hosting on the local network: others get in.
    h.app.session.execute("collab.share", &json!({"via": "local"})).unwrap();
    h.frames(2);
    assert_eq!(script.lock().unwrap().hosted, 1);
    assert_eq!(crate::tabs::live_mark(&h.app, h.app.session.docs[0].uid), Some(true));
    // A file of yours on a server that nobody else gets into now: marked apart.
    if let Some(sh) = h.app.session.live_mut().and_then(|l| l.share.as_mut()) {
        sh.links = None;
    }
    h.frames(1);
    assert_eq!(h.app.inspect()["tabs"], json!([{"name": "Homepage", "live": "private"}]));
    h.app.session.execute("collab.leave", &json!({})).unwrap();
    h.app.session.execute("collab.share", &json!({"via": "local"})).unwrap();
    // Someone joins while the file is in front, then while another is.
    let peer = |id, name: &str| Peer { client_id: id, name: name.into(), color: "#9747FF".into() };
    script.lock().unwrap().events.push(LiveEvent::PeerJoined(peer(2, "Hana")));
    h.frames(2);
    assert_eq!(toast(&h), "Hana joined");
    h.app.session.execute("file.new", &json!({})).unwrap();
    script.lock().unwrap().events.push(LiveEvent::PeerJoined(peer(3, "Ada")));
    h.frames(2);
    assert_eq!(toast(&h), "Ada joined Homepage");
    assert_eq!(h.app.inspect()["tabs"], json!([{"name": "Homepage", "live": "shared"}, {"name": "Untitled", "live": null}]));
    script.lock().unwrap().events.push(LiveEvent::PeerLeft(2));
    h.frames(2);
    assert_eq!(toast(&h), "Hana left Homepage");
}

/// A file of the scripted server's team list: `people` in it now.
fn team_file(id: &str, name: &str, people: u32, access: Access) -> TeamFile {
    TeamFile {
        file: FileInfo { id: id.into(), name: name.into(), created: 1_791_800_000, updated: 1_791_800_000, seq: 3, bytes: 900, people },
        access,
    }
}

/// A window whose Preferences name the scripted server, which keeps `team` as its team list.
fn team_harness(team: Option<(String, Vec<TeamFile>)>) -> (Harness, Shared) {
    let (mut h, script) = harness(vec![]);
    script.lock().unwrap().team = team;
    h.app.session.prefs.publish_server = SERVER.into();
    (h, script)
}

#[test]
fn the_team_list_of_the_preferences_server_shows_and_a_row_joins_its_file() {
    let web = "https://share.example/app/";
    let files = vec![team_file("t1", "Homepage", 2, Access::Edit), team_file("t2", "Pricing", 0, Access::Edit)];
    let (mut h, script) = team_harness(Some((web.into(), files)));
    let shapes = open(&mut h);
    let asked: Vec<(String, Option<Admin>)> = script.lock().unwrap().asked.clone();
    assert_eq!(asked, [(SERVER.to_string(), Some(Admin::Team))], "only the team list of the Preferences server is asked");
    for t in [
        "Nothing online yet. Use Share on a file to put it online and it is listed here.",
        "Shared by your team on share.example",
        "Everyone who reaches this server can open these files while they are shared.",
        "Homepage",
        "2 in it now · Changed 2026-10-12",
        "Pricing",
        "Changed 2026-10-12",
    ] {
        assert!(has_text(&shapes, t), "the section shows {t:?}: {:?}", texts(&shapes).iter().map(|t| t.0.clone()).collect::<Vec<_>>());
    }
    let inspect = h.app.inspect();
    assert_eq!((&inspect["online"]["team"]["state"], &inspect["online"]["team"]["files"][1]["name"]), (&json!("listed"), &json!("Pricing")));
    // The row's menu copies the links that open the file: for the app, and for the browser build.
    let shapes = menu(&mut h, "Pricing");
    click(&mut h, &shapes, "Copy link");
    let shapes = menu(&mut h, "Pricing");
    click(&mut h, &shapes, "Copy browser link");
    let link = format!("figcraft://join?server={SERVER}&file=t2&team=1");
    let browser = format!("{web}#join=figcraft%3A%2F%2Fjoin%3Fserver%3Dwss%3A%2F%2Fshare.example%2F%26file%3Dt2%26team%3D1");
    assert_eq!(script.lock().unwrap().copied, [link, browser]);
    // A click joins the file by its team list link.
    let shapes = h.frames(1);
    click(&mut h, &shapes, "Homepage");
    assert_eq!(script.lock().unwrap().joined, [format!("figcraft://join?server={SERVER}&file=t1&team=1")]);
}

#[test]
fn a_server_without_a_team_list_shows_none_and_the_shipped_public_server_isnt_asked() {
    let (mut h, script) = team_harness(None);
    let shapes = open(&mut h);
    assert!(!has_text(&shapes, "Shared by your team on share.example"));
    assert_eq!(h.app.inspect()["online"]["team"]["state"], "none");
    assert_eq!(script.lock().unwrap().asked.len(), 1);
    // Nobody sharing now, and a server that can't be reached, say so.
    let (mut h, _) = team_harness(Some((String::new(), vec![])));
    assert!(has_text(&open(&mut h), "Nobody shares a file there now."));
    let (mut h, script) = team_harness(None);
    script.lock().unwrap().offline = true;
    assert!(has_text(&open(&mut h), "Couldn't reach share.example for its team list."));
    // The public server the app ships with keeps no team list: it isn't asked.
    let (mut h, script) = harness(vec![]);
    h.app.session.prefs.publish_server = figcraft_engine::live::DEFAULT_SERVER.into();
    open(&mut h);
    assert!(script.lock().unwrap().asked.is_empty());
}
