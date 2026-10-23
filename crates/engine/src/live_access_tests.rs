//! Files on a server, against a scripted link: `collab.publish` makes the open tab the server
//! file's synced document (read-only until it is there, its undo history per person after), and
//! a view link makes the synced document read-only (commands, gestures, editing tools and changes
//! made outside a command are all refused or put back; others' edits still arrive; other tabs stay
//! editable). Leaving keeps the file open as your copy.

use std::sync::{Arc, Mutex};

use figcraft_collab::message::{ClientMsg, ServerMsg};
use figcraft_collab::{Client, NoRecompute, Outgoing};
use serde_json::json;

use crate::live::{Access, Event, PUBLISHING, Published, Role, Status, VIEW_ONLY};
use crate::live_tests::{Script, info, push, relay, serve_as, session};
use crate::tools::ToolId;
use crate::{EngineError, Session};

fn refused(s: &mut Session, cmd: &str, params: serde_json::Value, why: &str) {
    match s.execute(cmd, &params) {
        Err(EngineError::Disabled(_, reason)) => assert_eq!(reason, why, "{cmd}"),
        other => panic!("{cmd}: expected a refusal, got {other:?}"),
    }
}

#[test]
fn publishing_makes_the_open_tab_the_server_file() {
    let (mut s, script) = session();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
    assert!(s.undo_depth().0 > 0);
    let e = s.execute("collab.publish", &json!({"server": "nope"})).unwrap_err();
    assert!(e.to_string().contains("not a server address"), "{e}");
    let r = s.execute("collab.publish", &json!({"server": " ws://127.0.0.1:47900/?key=k ", "name": " Team\n"})).unwrap();
    assert_eq!(r["status"], "connecting");
    assert_eq!(r["readOnly"], true);
    {
        let sc = script.lock().unwrap();
        assert_eq!(sc.published.len(), 1);
        assert_eq!(sc.published[0].server, "ws://127.0.0.1:47900/?key=k");
        assert_eq!(sc.published[0].name, "Team");
        assert_eq!(sc.published[0].doc.node_count(), s.doc().unwrap().node_count());
    }
    // Until it is on the server nothing changes it (and the name defaults to the file's).
    refused(&mut s, "shape.ellipse", json!({"x": 0, "y": 0, "width": 5, "height": 5}), PUBLISHING);
    refused(&mut s, "tool.set", json!({"tool": "rectangle"}), PUBLISHING);
    assert!(s.execute("tool.set", &json!({"tool": "hand"})).is_ok(), "moving around is fine");
    assert!(s.begin_interaction("Move").is_err());
    // The server's welcome: this tab is the synced document, undo is per person from now on.
    let mut doc = s.doc().unwrap().clone();
    doc.name = "Team".into();
    let (mut server, id, welcome) = serve_as(&doc, Access::Edit);
    let published = Published {
        server: "127.0.0.1:47900".into(),
        file: "f".into(),
        name: "Team".into(),
        edit: "e".into(),
        view: "v".into(),
        owner: None,
        expires: None,
    };
    push(&script, vec![Event::Published(published.clone()), Event::Welcome { info: info(Role::Guest, id), peers: vec![] }, welcome]);
    s.live_poll();
    assert!(s.live().syncing());
    assert_eq!(s.docs.len(), 1, "no new tab");
    assert_eq!(s.doc().unwrap().name, "Team");
    assert_eq!(s.undo_depth(), (0, 0));
    assert_eq!(s.active_read_only(), None);
    assert_eq!(s.live().published, Some(published));
    assert_eq!(s.message.as_deref(), Some("Published Team to the server"));
    assert_eq!(s.live_summary()["published"]["name"], "Team");
    // Edits sync through the server.
    let rect = s.execute("shape.rectangle", &json!({"x": 100, "y": 0, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    let answers = relay(&script, &mut server, id);
    assert!(server.document().contains(figcraft_doc::NodeId(rect)));
    push(&script, answers);
    s.live_poll();
    assert_eq!(s.undo_depth(), (1, 0));
    // Leaving keeps the file open, yours to edit.
    s.execute("collab.leave", &json!({})).unwrap();
    assert_eq!(s.docs.len(), 1);
    assert!(s.doc().unwrap().contains(figcraft_doc::NodeId(rect)));
    assert_eq!(s.message.as_deref(), Some("You left the session; your copy stays open here"));
    assert!(s.execute("shape.ellipse", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).is_ok());
}

#[test]
fn a_view_link_makes_the_file_read_only() {
    let (mut s, script) = session();
    s.execute("tool.set", &json!({"tool": "rectangle"})).unwrap();
    s.execute("collab.join", &json!({"link": "figcraft://join?host=a:1&file=f&token=t"})).unwrap();
    let mut doc = figcraft_doc::Document::new("Shared");
    let page = doc.first_page().unwrap();
    let card = doc.create(page, figcraft_doc::NodeKind::Rectangle, "Card", 0.0, 0.0, 10.0, 10.0).unwrap();
    let (mut server, id, welcome) = serve_as(&doc, Access::View);
    let mut me = info(Role::Guest, id);
    me.access = Access::View;
    push(&script, vec![Event::Welcome { info: me, peers: vec![] }, welcome]);
    s.live_poll();
    assert!(s.live().syncing());
    assert_eq!(s.active_read_only(), Some(VIEW_ONLY));
    let summary = s.live_summary();
    assert_eq!((summary["access"].as_str(), summary["readOnly"].as_bool()), (Some("view"), Some(true)));
    assert_eq!(s.tool.tool, ToolId::Move, "the editing tool was put away");
    // Commands, tools and gestures are refused; nothing changes and nothing is sent.
    let before = s.doc().unwrap().clone();
    refused(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5}), VIEW_ONLY);
    refused(&mut s, "node.set", json!({"ids": [card.0], "props": {"name": "Mine"}}), VIEW_ONLY);
    refused(&mut s, "edit.delete", json!({"ids": [card.0]}), VIEW_ONLY);
    refused(&mut s, "tool.set", json!({"tool": "text"}), VIEW_ONLY);
    assert!(matches!(s.begin_interaction("Move"), Err(EngineError::Disabled(..))));
    assert_eq!(*s.doc().unwrap(), before);
    assert!(s.execute("select.set", &json!({"ids": [card.0]})).is_ok(), "selecting is fine");
    // A change made outside a command is put back, never sent.
    s.ds_mut().unwrap().doc_mut().node_mut(card).unwrap().name = "Outside".into();
    s.live_sync();
    assert_eq!(s.doc().unwrap().node(card).unwrap().name, "Card");
    assert!(script.lock().unwrap().msgs.iter().all(|m| !matches!(m, ClientMsg::Changes { .. })));
    // Someone who may edit renames the card: it arrives.
    let ClientMsg::Hello(h) = Client::hello("f", "", "Bo", "") else { panic!() };
    let (bo_id, outs) = server.join(h).unwrap();
    let Some(Outgoing::To(_, w)) = outs.into_iter().next() else { panic!() };
    let mut bo = Client::join(w).unwrap();
    let mut d = bo.document().clone();
    d.node_mut(card).unwrap().name = "Bo's card".into();
    let m = bo.edit(&d).unwrap().unwrap();
    for o in server.receive(bo_id, m) {
        if let Outgoing::All(m @ ServerMsg::Changes(_)) = o {
            bo.receive(m.clone(), &NoRecompute).unwrap();
            push(&script, vec![Event::Sync(m)]);
        }
    }
    s.live_poll();
    assert_eq!(s.doc().unwrap().node(card).unwrap().name, "Bo's card");
    // Your other tabs stay yours.
    s.active = 0;
    assert_eq!(s.active_read_only(), None);
    assert!(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).is_ok());
}

fn saved_file(s: &mut Session) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("figcraft-live-revert-{}-{}.figcraft", std::process::id(), s.ds().unwrap().uid));
    s.execute("file.saveVersion", &json!({"name": "Saved"})).unwrap();
    s.execute("file.save", &json!({"path": path})).unwrap();
    s.execute("file.rename", &json!({"name": "Unsaved"})).unwrap();
    path
}

fn revert_refused_unchanged(s: &mut Session, script: &Arc<Mutex<Script>>) {
    let uid = s.ds().unwrap().uid;
    let session = s.live().id;
    let before = format!("{:?}", s.docs);
    let live = s.live_summary();
    let journal = s.journal.clone();
    let last = s.last_command.clone();
    let message = s.message.clone();
    let ui = s.ui_requests.clone();
    let active = s.active;
    let script_before = {
        let sc = script.lock().unwrap();
        (sc.msgs.len(), sc.sent.len(), sc.left)
    };
    let result = s.execute("file.revert", &json!({}));
    assert!(
        matches!(result, Err(EngineError::Disabled(ref cmd, _)) if cmd == "file.revert"),
        "expected refusal, got {result:?}; document UID {uid} -> {}, live session {session} -> {}",
        s.ds().unwrap().uid,
        s.live().id
    );
    assert_eq!(format!("{:?}", s.docs), before, "all document state, including previews and branch tabs, stays intact");
    assert_eq!(s.live_summary(), live);
    assert_eq!((s.active, &s.journal, &s.last_command, &s.message, &s.ui_requests), (active, &journal, &last, &message, &ui));
    let sc = script.lock().unwrap();
    assert_eq!((sc.msgs.len(), sc.sent.len(), sc.left), script_before, "no sync, presence or leave sent");
}

#[test]
fn revert_refuses_live_documents_without_losing_access_or_sync() {
    for state in ["editing", "viewing", "publishing", "reconnecting", "hosting"] {
        let (mut s, script) = session();
        let path = saved_file(&mut s);
        let uid = s.ds().unwrap().uid;
        if state == "hosting" {
            s.execute("collab.host", &json!({})).unwrap();
        } else {
            s.execute("collab.publish", &json!({"server": "ws://127.0.0.1:47900/"})).unwrap();
        }
        let access = if state == "viewing" { Access::View } else { Access::Edit };
        let (mut server, me, welcome) = serve_as(s.doc().unwrap(), access);
        if !matches!(state, "publishing" | "hosting") {
            let mut i = info(Role::Guest, me);
            i.access = access;
            push(&script, vec![Event::Welcome { info: i, peers: vec![] }, welcome.clone()]);
            s.live_poll();
        }
        if state == "reconnecting" {
            push(&script, vec![Event::Status(Status::Reconnecting { attempt: 1 })]);
            s.live_poll();
        }
        let read_only = match state {
            "viewing" => Some(VIEW_ONLY),
            "publishing" => Some(PUBLISHING),
            _ => None,
        };
        assert_eq!(s.active_read_only(), read_only, "{state}");
        revert_refused_unchanged(&mut s, &script);
        assert_eq!(s.live().doc_uid, Some(uid), "{state}");
        assert_eq!(s.active_read_only(), read_only, "{state}");
        if let Some(why) = read_only {
            refused(&mut s, "file.rename", json!({"name": "Refused"}), why);
        }
        if matches!(state, "publishing" | "hosting") {
            if state == "publishing" {
                push(&script, vec![Event::Welcome { info: info(Role::Guest, me), peers: vec![] }]);
            }
            push(&script, vec![welcome]);
            s.live_poll();
            assert_eq!(s.live().doc_uid, Some(uid), "the welcome still finds its document");
        }
        if access == Access::Edit && state != "reconnecting" {
            s.execute("file.rename", &json!({"name": "Still syncing"})).unwrap();
            let answers = relay(&script, &mut server, me);
            assert_eq!(server.document().name, "Still syncing", "{state}");
            push(&script, answers);
            s.live_poll();
        }
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn revert_refusal_preserves_previews_and_branch_tabs() {
    let (mut s, script) = session();
    let text = s.execute("text.create", &json!({"text": "Preview"})).unwrap()["id"].clone();
    let path = saved_file(&mut s);
    s.execute("branch.create", &json!({"name": "Working"})).unwrap();
    s.execute("doc.switch", &json!({"index": 0})).unwrap();
    s.execute("collab.host", &json!({})).unwrap();
    let (_, _, welcome) = serve_as(s.doc().unwrap(), Access::Edit);
    push(&script, vec![welcome]);
    s.live_poll();
    let main_uid = s.ds().unwrap().uid;
    // A branch has no local save path; refusal keeps its live parent associated.
    s.execute("doc.switch", &json!({"index": 1})).unwrap();
    assert!(s.ds().unwrap().path.is_none());
    revert_refused_unchanged(&mut s, &script);
    assert_eq!(s.live_of(main_uid).doc_uid, Some(main_uid));
    s.execute("doc.switch", &json!({"index": 0})).unwrap();
    s.execute("version.preview", &json!({"index": 0})).unwrap();
    assert!(s.ds().unwrap().preview.is_some());
    // A gesture on a branch has not yet been copied back into its main file.
    s.docs[1].doc_mut().name = "Branch edits".into();
    revert_refused_unchanged(&mut s, &script);
    s.execute("version.exitPreview", &json!({})).unwrap();
    s.execute("text.preview", &json!({"command": "text.style", "params": {"ids": [text], "fontSize": 32}})).unwrap();
    assert!(s.ds().unwrap().text_preview.is_some());
    revert_refused_unchanged(&mut s, &script);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn revert_offline_file_works_while_another_file_is_live_and_after_leaving() {
    let (mut s, script) = session();
    let path = saved_file(&mut s);
    let uid = s.ds().unwrap().uid;
    s.execute("collab.host", &json!({})).unwrap();
    s.execute("file.new", &json!({})).unwrap();
    let offline_path = saved_file(&mut s);
    s.execute("file.revert", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().name, offline_path.file_stem().unwrap().to_string_lossy());
    assert!(!s.ds().unwrap().is_dirty());
    assert_eq!(s.ds().unwrap().versions.len(), 1);
    assert_eq!(s.live_of(uid).doc_uid, Some(uid));
    assert_eq!(script.lock().unwrap().left, 0);
    s.execute("doc.switch", &json!({"index": 0})).unwrap();
    s.execute("collab.leave", &json!({})).unwrap();
    s.execute("file.revert", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().name, path.file_stem().unwrap().to_string_lossy());
    assert!(!s.ds().unwrap().is_dirty());
    assert_eq!(s.ds().unwrap().versions.len(), 1);
    assert_eq!(s.live().doc_uid, None);
    assert_eq!(script.lock().unwrap().left, 1);
    std::fs::remove_file(path).unwrap();
    std::fs::remove_file(offline_path).unwrap();
}
