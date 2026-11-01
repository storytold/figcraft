//! A server file joined with a comment link, against a scripted link: comments (new threads,
//! replies, resolving, reactions) are made and sync through the server; every other edit, the
//! editing tools and gestures are refused as with a view link, and a change made outside a command
//! is put back unless it only touches comment threads.

use figcraft_collab::message::ClientMsg;
use figcraft_doc::{Document, NodeKind};
use serde_json::json;

use crate::live::{Access, COMMENT_ONLY, Event, Role, tool_left};
use crate::live_tests::{info, push, relay, serve_as, session};
use crate::tools::ToolId;
use crate::{EngineError, Session};

fn refused(s: &mut Session, cmd: &str, params: serde_json::Value) {
    match s.execute(cmd, &params) {
        Err(EngineError::Disabled(_, reason)) => assert_eq!(reason, COMMENT_ONLY, "{cmd}"),
        other => panic!("{cmd}: expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_comment_link_lets_you_comment_and_nothing_else() {
    let (mut s, script) = session();
    s.execute("tool.set", &json!({"tool": "rectangle"})).unwrap();
    s.execute("collab.join", &json!({"link": "figcraft://join?host=a:1&file=f&token=t"})).unwrap();
    let mut doc = Document::new("Shared");
    let page = doc.first_page().unwrap();
    let card = doc.create(page, NodeKind::Rectangle, "Card", 0.0, 0.0, 10.0, 10.0).unwrap();
    let (mut server, id, welcome) = serve_as(&doc, Access::Comment);
    let mut me = info(Role::Guest, id);
    me.access = Access::Comment;
    push(&script, vec![Event::Welcome { info: me, peers: vec![] }, welcome]);
    s.live_poll();
    assert!(s.live().syncing());
    assert_eq!(s.active_read_only(), Some(COMMENT_ONLY));
    let summary = s.live_summary();
    assert_eq!((summary["access"].as_str(), summary["readOnly"].as_bool()), (Some("comment"), Some(true)));
    assert_eq!(s.tool.tool, ToolId::Move, "the editing tool was put away");
    // The comment tool is left, and commenting syncs through the server.
    s.execute("tool.set", &json!({"tool": "comment"})).unwrap();
    let thread = s.execute("comment.add", &json!({"x": 5, "y": 5, "text": "Bigger logo?"})).unwrap()["id"].as_u64().unwrap();
    s.execute("comment.reply", &json!({"id": thread, "text": "Or darker"})).unwrap();
    s.execute("comment.react", &json!({"id": thread, "emoji": "👍"})).unwrap();
    s.execute("comment.resolve", &json!({"id": thread})).unwrap();
    let answers = relay(&script, &mut server, id);
    let on_server = server.document().comment(thread).expect("the thread is on the server");
    assert!(on_server.resolved && on_server.replies.len() == 1 && !on_server.first.reactions.is_empty());
    push(&script, answers);
    s.live_poll();
    // Everything else is refused before anything is sent, as with a view link.
    let before = s.doc().unwrap().clone();
    refused(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5}));
    refused(&mut s, "node.set", json!({"ids": [card.0], "props": {"name": "Mine"}}));
    refused(&mut s, "edit.delete", json!({"ids": [card.0]}));
    refused(&mut s, "tool.set", json!({"tool": "text"}));
    assert!(matches!(s.begin_interaction("Move"), Err(EngineError::Disabled(..))));
    assert_eq!(*s.doc().unwrap(), before);
    assert!(script.lock().unwrap().msgs.iter().all(|m| !matches!(m, ClientMsg::Changes { .. })), "nothing sent");
    // A change made outside a command: put back, unless it only touches comment threads.
    s.ds_mut().unwrap().doc_mut().node_mut(card).unwrap().name = "Outside".into();
    s.live_sync();
    assert_eq!(s.doc().unwrap().node(card).unwrap().name, "Card");
    s.ds_mut().unwrap().doc_mut().comment_mut(thread).unwrap().resolved = false;
    s.live_sync();
    relay(&script, &mut server, id);
    assert!(!server.document().comment(thread).unwrap().resolved, "sent");
    // Your other tabs stay yours.
    s.active = 0;
    assert_eq!(s.active_read_only(), None);
    assert!(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).is_ok());
}

#[test]
fn only_a_comment_link_leaves_the_comment_tool() {
    for t in [ToolId::Move, ToolId::Hand, ToolId::Comment] {
        assert!(tool_left(COMMENT_ONLY, t), "{t:?}");
    }
    assert!(!tool_left(COMMENT_ONLY, ToolId::Rectangle));
    assert!(!tool_left(crate::live::VIEW_ONLY, ToolId::Comment) && tool_left(crate::live::VIEW_ONLY, ToolId::Hand));
    assert!(!tool_left(crate::live::PUBLISHING, ToolId::Comment));
}

#[test]
fn a_local_share_has_no_comment_code() {
    let (mut s, _) = session();
    let e = s.execute("collab.share", &json!({"via": "local", "access": "comment"})).unwrap_err().to_string();
    assert!(e.contains("can edit it"), "{e}");
    let e = s.execute("collab.share", &json!({"access": "owner"})).unwrap_err().to_string();
    assert!(e.contains("\"comment\""), "{e}");
}
