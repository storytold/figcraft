//! The team list (`team_files`) against a real server: off unless the operator turns it on; on,
//! it lists the files shared now (not the ones only published), hands out one team link per file
//! that joins with the list's access, and when the owner stops sharing the file leaves the list,
//! its team link stops working and whoever joined with it is disconnected. Turning the list off,
//! or changing its access, refuses the old team links at once.

use std::net::SocketAddr;

use figcraft_collab::message::{Access, Admin, AdminReply, ClientMsg, OpenHosting, PROTOCOL, ResolveTeam, ServerMsg, TeamFile};
use figcraft_engine::live::Status;
use serde_json::{Value, json};

use crate::config::Config;
use crate::store::team::{NO_TEAM, NOT_SHARED};
use crate::test_util::{Raw, TempDir, config, join, live, publish, pump, session};

fn team(dir: &std::path::Path, access: &str) -> Config {
    Config { open_publish: true, team_files: access.into(), web_url: "https://figcraft.example.com/app/".into(), ..config(dir) }
}

fn ex(s: &mut figcraft_engine::Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

/// One request on an open hosting connection that names no file (anyone's).
fn ask(addr: SocketAddr, req: Admin) -> AdminReply {
    let mut raw = Raw::connect(addr);
    raw.send(&ClientMsg::OpenHosting(OpenHosting { protocol: PROTOCOL, client_version: "test".into(), file: String::new(), owner: String::new() }));
    assert!(matches!(raw.next(), ServerMsg::Hosting(_)));
    raw.send(&ClientMsg::Admin(req));
    match raw.next() {
        ServerMsg::Admin(r) => *r,
        other => panic!("answered {other:?}"),
    }
}

fn listed(addr: SocketAddr) -> (String, Vec<TeamFile>) {
    match ask(addr, Admin::Team) {
        AdminReply::Team { web, files } => (web, files),
        other => panic!("not a team list: {other:?}"),
    }
}

/// The team link of `file`, as a join by the team list asks for it: its token.
fn team_link(addr: SocketAddr, file: &str) -> Result<String, String> {
    let mut raw = Raw::connect(addr);
    raw.send(&ClientMsg::ResolveTeam(ResolveTeam { protocol: PROTOCOL, client_version: "test".into(), file: file.into() }));
    match raw.next() {
        ServerMsg::Resolved { file: f, token } if f == file => Ok(token),
        ServerMsg::Refused { reason } => Err(reason),
        other => panic!("answered {other:?}"),
    }
}

/// The team link's entry in the file's links (as `figcraft-server links list` shows them).
fn team_entry(dir: &std::path::Path, file: &str) -> (Access, String) {
    let links: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("files").join(file).join("links.json")).unwrap()).unwrap();
    let l = links.as_array().unwrap().iter().find(|l| l["team"].is_string()).expect("a team link");
    (Access::parse(l["access"].as_str().unwrap()).unwrap(), l["label"].as_str().unwrap().to_string())
}

fn invite(addr: SocketAddr, file: &str, token: &str) -> String {
    format!("ws://{addr}/?file={file}&token={token}")
}

/// Ada shares a file online: the app with the file live, and the file's id.
fn share(addr: SocketAddr, name: &str) -> (figcraft_engine::Session, String) {
    let mut a = session("Ada");
    ex(&mut a, "pref.publishServer", json!({"url": format!("ws://{addr}/")}));
    ex(&mut a, "file.rename", json!({"name": name}));
    ex(&mut a, "shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 30}));
    ex(&mut a, "collab.share", json!({}));
    pump(&mut [&mut a], "the share", |s| live(s[0]) && s[0].live().share.as_ref().is_some_and(|sh| sh.links.is_some()));
    let file = a.prefs.online_files[0].file.clone();
    (a, file)
}

fn layers(s: &figcraft_engine::Session) -> usize {
    s.doc().map_or(0, |d| d.node_count())
}

fn refused(s: &figcraft_engine::Session) -> bool {
    matches!(&s.live().status, Status::Failed { reason } if reason.contains("isn't valid"))
}

#[test]
fn there_is_no_team_list_unless_the_operator_turns_it_on() {
    let dir = TempDir::new("team-off");
    let server = crate::start(&Config { open_publish: true, ..config(dir.path()) }).unwrap();
    let (_a, file) = share(server.addr(), "Homepage");
    assert_eq!(ask(server.addr(), Admin::Team), AdminReply::Failed { reason: NO_TEAM.into() });
    assert_eq!(team_link(server.addr(), &file), Err(NO_TEAM.into()));
    for bad in ["owner", "edit view"] {
        assert!(team(dir.path(), bad).check().is_err(), "team_files = {bad:?} is refused");
    }
    for bad in ["ftp://x/", "https://x/app/?k=1", "https://x/app/#a", "https://x y/"] {
        assert!(Config { web_url: bad.into(), ..config(dir.path()) }.check().is_err(), "web_url = {bad:?} is refused");
    }
    assert!(team(dir.path(), " View ").check().is_ok());
}

#[test]
fn the_team_list_has_the_files_shared_now_and_their_team_link_joins() {
    let dir = TempDir::new("team-list");
    let server = crate::start(&team(dir.path(), "edit")).unwrap();
    let addr = server.addr();
    let (mut a, file) = share(addr, "Homepage");
    // Published with the key but not shared: not on the list.
    let mut c = session("Cy");
    let unshared = publish(&mut c, dir.path(), addr).file;
    let (web, files) = listed(addr);
    assert_eq!(web, "https://figcraft.example.com/app/");
    let ids: Vec<(&str, &str, Access)> = files.iter().map(|f| (f.file.id.as_str(), f.file.name.as_str(), f.access)).collect();
    assert_eq!(ids, vec![(file.as_str(), "Homepage", Access::Edit)]);
    assert!(team_link(addr, &unshared).is_err_and(|e| e == NOT_SHARED));
    assert!(team_link(addr, "nonesuch").is_err_and(|e| e == NOT_SHARED));
    // The team link: made once, the same for everyone after.
    let token = team_link(addr, &file).unwrap();
    assert_eq!(team_entry(dir.path(), &file), (Access::Edit, "Team (edit)".to_string()));
    assert_eq!(team_link(addr, &file).unwrap(), token);
    // Bob joins with it and edits; Ada sees the change.
    let mut b = join(&invite(addr, &file, &token), "Bob");
    pump(&mut [&mut a, &mut b], "Bob in", |s| live(s[1]) && s[0].live().peers.len() == 1);
    let before = layers(&a);
    ex(&mut b, "shape.ellipse", json!({"x": 80, "y": 10, "width": 30, "height": 30}));
    pump(&mut [&mut a, &mut b], "Bob's ellipse at Ada's", |s| layers(s[0]) == before + 1);
    // The team link isn't a share of its own: the file isn't listed twice, and the owner's
    // record still has just the links Ada made.
    assert_eq!(listed(addr).1.len(), 1);
}

#[test]
fn stopping_sharing_takes_the_file_off_the_list_and_ends_the_team_link() {
    let dir = TempDir::new("team-unshare");
    let server = crate::start(&team(dir.path(), "edit")).unwrap();
    let addr = server.addr();
    let (mut a, file) = share(addr, "Homepage");
    let token = team_link(addr, &file).unwrap();
    let mut b = join(&invite(addr, &file, &token), "Bob");
    pump(&mut [&mut a, &mut b], "Bob in", |s| live(s[1]));
    assert_eq!(ex(&mut a, "collab.manageFile", json!({"file": file, "action": "unshare"}))["shared"], false);
    // Bob is out, as people who joined with Ada's own links are.
    pump(&mut [&mut a, &mut b], "Bob disconnected", |s| matches!(s[1].live().status, Status::Failed { .. } | Status::Off));
    assert!(listed(addr).1.is_empty());
    assert!(team_link(addr, &file).is_err_and(|e| e == NOT_SHARED));
    let mut again = join(&invite(addr, &file, &token), "Bob");
    pump(&mut [&mut again], "the old team link refused", |s| refused(s[0]));
    // Sharing again lists it again, with a new team link.
    assert_eq!(ex(&mut a, "collab.manageFile", json!({"file": file, "action": "share"}))["shared"], true);
    assert_eq!(listed(addr).1.len(), 1);
    assert_ne!(team_link(addr, &file).unwrap(), token);
}

#[test]
fn turning_the_list_off_or_changing_its_access_refuses_old_team_links() {
    let dir = TempDir::new("team-config");
    let mut server = crate::start(&team(dir.path(), "edit")).unwrap();
    let addr = server.addr();
    let (a, file) = share(addr, "Homepage");
    let edit = team_link(addr, &file).unwrap();
    drop(a);
    server.stop();
    // View only now: the edit team link no longer joins; the new one is view only.
    let server = crate::start(&team(dir.path(), "view")).unwrap();
    let addr = server.addr();
    let mut b = join(&invite(addr, &file, &edit), "Bob");
    pump(&mut [&mut b], "the edit team link refused", |s| refused(s[0]));
    let view = team_link(addr, &file).unwrap();
    assert_eq!(team_entry(dir.path(), &file).0, Access::View);
    let mut b = join(&invite(addr, &file, &view), "Bob");
    pump(&mut [&mut b], "Bob in, view only", |s| live(s[0]) && s[0].live_summary()["access"] == "view");
    drop(b);
    drop(server);
    // Off: the list is gone and its links with it, also from the file's links.
    let server = crate::start(&Config { open_publish: true, ..config(dir.path()) }).unwrap();
    let mut b = join(&invite(server.addr(), &file, &view), "Bob");
    pump(&mut [&mut b], "the team link refused", |s| refused(s[0]));
    drop(b);
    drop(server);
    assert!(!std::fs::read_to_string(dir.path().join("files").join(&file).join("links.json")).unwrap().contains(r#""team""#));
    // On again with the old access: the old links stay refused; the next join gets a new one.
    let server = crate::start(&team(dir.path(), "view")).unwrap();
    let mut b = join(&invite(server.addr(), &file, &view), "Bob");
    pump(&mut [&mut b], "the old view team link still refused", |s| refused(s[0]));
    assert_ne!(team_link(server.addr(), &file).unwrap(), view);
}

#[test]
fn a_team_link_never_handed_out_again_stays_refused_after_the_setting_goes_back() {
    let dir = TempDir::new("team-back");
    let mut server = crate::start(&team(dir.path(), "edit")).unwrap();
    let (a, file) = share(server.addr(), "Homepage");
    let edit = team_link(server.addr(), &file).unwrap();
    drop(a);
    server.stop();
    // View only for a while, and nobody asks for the view team link…
    drop(crate::start(&team(dir.path(), "view")).unwrap());
    // …then edit again: the edit team link of before doesn't come back.
    let server = crate::start(&team(dir.path(), "edit")).unwrap();
    let mut b = join(&invite(server.addr(), &file, &edit), "Bob");
    pump(&mut [&mut b], "the old edit team link refused", |s| refused(s[0]));
    assert_ne!(team_link(server.addr(), &file).unwrap(), edit);
}

#[test]
fn a_team_list_link_joins_without_a_token_while_the_file_is_shared() {
    let dir = TempDir::new("team-join");
    let server = crate::start(&team(dir.path(), "edit")).unwrap();
    let addr = server.addr();
    let (mut a, file) = share(addr, "Homepage");
    // What the team list gives people to click or paste: no token in it.
    let link = format!("figcraft://join?host={addr}&file={file}&team=1");
    let mut b = join(&link, "Bob");
    pump(&mut [&mut a, &mut b], "Bob in by the team list", |s| live(s[1]) && s[0].live().peers.len() == 1);
    assert_eq!(b.live_summary()["access"], "edit");
    let failed = |s: &figcraft_engine::Session, why: &str| matches!(&s.live().status, Status::Failed { reason } if reason.contains(why));
    let mut c = join(&format!("figcraft://join?host={addr}&file=0123456789abcdef&team=1"), "Cy");
    pump(&mut [&mut c], "a file that isn't shared refused", |s| failed(s[0], "isn't shared now"));
    // A server without a team list says so.
    let dir = TempDir::new("team-join-off");
    let off = crate::start(&Config { open_publish: true, ..config(dir.path()) }).unwrap();
    let mut d = join(&format!("figcraft://join?host={}&file={file}&team=1", off.addr()), "Di");
    pump(&mut [&mut d], "no team list", |s| failed(s[0], "keeps no team list"));
}

#[test]
fn the_apps_online_files_list_the_team_list_and_open_its_files() {
    let dir = TempDir::new("team-app");
    let server = crate::start(&team(dir.path(), "edit")).unwrap();
    let addr = server.addr();
    let (mut a, file) = share(addr, "Homepage");
    // Bob has the same server in Preferences and nothing of his own online.
    let mut b = session("Bob");
    ex(&mut b, "pref.publishServer", json!({"url": format!("ws://{addr}/")}));
    assert_eq!(ex(&mut b, "collab.files", json!({}))["team"], Value::Null, "nothing asked without refresh");
    let list = ex(&mut b, "collab.files", json!({"refresh": true}));
    let t = &list["team"];
    assert_eq!(
        (&t["state"], &t["web"], &t["files"][0]["file"], &t["files"][0]["name"]),
        (&json!("listed"), &json!("https://figcraft.example.com/app/"), &json!(file), &json!("Homepage"))
    );
    let link = t["files"][0]["link"].as_str().unwrap().to_string();
    assert_eq!(link, format!("figcraft://join?server=ws://{addr}/&file={file}&team=1"));
    assert_eq!(
        t["files"][0]["webLink"],
        json!(format!(
            "https://figcraft.example.com/app/#join=figcraft%3A%2F%2Fjoin%3Fserver%3Dws%3A%2F%2F{}%2F%26file%3D{file}%26team%3D1",
            addr.to_string().replace(':', "%3A")
        ))
    );
    assert!(!list.to_string().contains("token"), "no team link token in the result: {list}");
    ex(&mut b, "collab.join", json!({"link": link}));
    pump(&mut [&mut a, &mut b], "Bob in from the list", |s| live(s[1]) && s[0].live().peers.len() == 1);
    // Another server in Preferences: its list isn't this one.
    ex(&mut b, "pref.publishServer", json!({"url": "wss://elsewhere.example/"}));
    assert_eq!(ex(&mut b, "collab.files", json!({}))["team"], Value::Null);
    // A server without a team list says so.
    let dir = TempDir::new("team-app-off");
    let off = crate::start(&Config { open_publish: true, ..config(dir.path()) }).unwrap();
    ex(&mut b, "pref.publishServer", json!({"url": format!("ws://{}/", off.addr())}));
    assert_eq!(ex(&mut b, "collab.files", json!({"refresh": true}))["team"]["state"], "none");
}
