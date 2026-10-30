//! Files on a server in the window: the Publish to server dialog (from the File menu and the
//! Share popover; its field takes the keyboard; Escape closes it; publishing leaves the tab
//! read-only until the file is there; a server open to the public takes it without a key and the
//! owner token shows once), the Share popover's address, access, a published file's links, where
//! it is hosted, when it expires and reporting it, and a view link's "View only" badge with only
//! the moving tools.

use egui::{Event, Key};
use figcraft_engine::figcraft_collab::message::{ClientMsg, ServerMsg};
use figcraft_engine::figcraft_collab::{Client, Limits, Outgoing, Server};
use figcraft_engine::live::{Access, COMMENT_ONLY, Event as LiveEvent, HostInfo, Published, Role, SessionInfo, Status, VIEW_ONLY};
use figcraft_engine::tools::ToolId;
use serde_json::json;

use crate::live_tests::{Harness, has_text, text_center};

fn key(h: &mut Harness, key: Key) {
    for pressed in [true, false] {
        h.frame(vec![Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Default::default() }]);
    }
}

/// Whether the widget with the given id currently has focus.
fn focused(h: &Harness, id: &str) -> bool {
    h.ctx.memory(|m| m.has_focus(egui::Id::unique(id)))
}

/// Joined through a server's link with `access`: the welcome and the document (a new tab).
pub(crate) fn joined(h: &mut Harness, access: Access) {
    h.app.session.execute("collab.join", &json!({"link": "figcraft://join?host=127.0.0.1:47900&file=f&token=AbCdEfGhIjKlMnOpQrStUv"})).unwrap();
    let mut server = Server::new(h.app.session.doc().unwrap().clone(), "", Limits::default());
    let ClientMsg::Hello(me) = Client::hello("f", "", "Ada", "") else { panic!("no hello") };
    let (id, outs) = server.join_as(me, access).unwrap();
    let w = outs.into_iter().find_map(|o| if let Outgoing::To(_, w @ ServerMsg::Welcome { .. }) = o { Some(w) } else { None }).unwrap();
    let info = SessionInfo {
        role: Role::Guest,
        me: id,
        name: "Ada".into(),
        color: "#14AE5C".into(),
        file_name: String::new(),
        pages: vec![],
        link: String::new(),
        url: String::new(),
        address: "127.0.0.1:47900".into(),
        network: false,
        access,
    };
    h.send(vec![LiveEvent::Welcome { info, peers: vec![] }, LiveEvent::Sync(w)]);
    h.frames(2);
    assert!(h.app.session.live_shared());
}

#[test]
fn the_publish_dialog() {
    let (mut h, _) = Harness::new();
    crate::menus::invoke(&mut h.app, "collab.publish");
    h.frames(2);
    assert_eq!(h.app.ui.live.publish.as_ref().map(|f| f.name.as_str()), Some("Untitled"), "the file's name to start with");
    assert!(focused(&h, "live_publish_server"), "the address field takes the keyboard");
    key(&mut h, Key::Escape);
    assert!(h.app.ui.live.publish.is_none(), "Escape closes it");
    // From the Share popover, then publish.
    let shapes = h.frames(2);
    h.click(text_center(&shapes, "Share"));
    let shapes = h.frames(2);
    h.click(text_center(&shapes, "Advanced"));
    let shapes = h.frames(2);
    assert!(has_text(&shapes, "Another server, or one that needs its key. The usual server is set in Preferences."));
    h.click(text_center(&shapes, "Publish to another server…"));
    let shapes = h.frames(2);
    assert!(has_text(
        &shapes,
        "Give the address of a server open to the public, or a server's address with its key (`figcraft-server key` prints it)."
    ));
    h.frame(vec![Event::Text("ws://127.0.0.1:47900/?key=KeyKeyKeyKeyKeyKeyKey0".into())]);
    let shapes = h.frames(1);
    assert!(has_text(&shapes, "Publishing with the server's key."));
    h.click(text_center(&shapes, "Publish"));
    let shapes = h.frames(2);
    assert!(h.app.ui.live.publish.is_none(), "it closes once publishing starts");
    assert_eq!(h.app.session.live().status, Status::Connecting);
    assert!(has_text(&shapes, "Publishing…"), "the toolbar says why the file can't be edited yet");
    assert_eq!(crate::live::inspect(&h.app)["readOnly"], figcraft_engine::live::PUBLISHING);
}

fn hosting(expires: Option<u64>) -> HostInfo {
    HostInfo {
        open: true,
        operator: "abuse@example.com".into(),
        terms: "https://example.com/terms".into(),
        idle_secs: 30 * 86_400,
        max_file_mb: 64,
        expires,
    }
}

#[test]
fn a_server_open_to_the_public_takes_a_file_without_a_key() {
    let (mut h, _) = Harness::new();
    crate::menus::invoke(&mut h.app, "collab.publish");
    h.frames(2);
    h.frame(vec![Event::Text("wss://figcraft.example.com/".into())]);
    let shapes = h.frames(1);
    assert!(!crate::live::inspect(&h.app)["publishDialog"].is_null());
    // Without a key and before the server said it is open, Publish waits.
    h.click(text_center(&shapes, "Publish"));
    assert!(h.app.ui.live.publish.is_some(), "nothing is sent yet");
    // The server's answer (collab.serverInfo).
    h.app.ui.live.publish.as_mut().unwrap().info = Some((
        "wss://figcraft.example.com/".into(),
        json!({"open": true, "operator": "abuse@example.com", "terms": "https://example.com/terms", "idleDays": 30.0}),
    ));
    let shapes = h.frames(2);
    assert!(has_text(&shapes, "This server is open to the public: no key needed."));
    assert!(has_text(&shapes, "Hosted by abuse@example.com") && has_text(&shapes, "Terms"));
    assert!(has_text(&shapes, "Files nobody opens for 30 days are deleted."));
    assert!(has_text(&shapes, "Files aren't end-to-end encrypted: the server's operator can read them."));
    h.click(text_center(&shapes, "Publish"));
    h.frames(2);
    assert!(h.app.ui.live.publish.is_none());
    assert_eq!(h.app.session.live().status, Status::Connecting);
    // On the server: the owner token shows once, with the links and the expiry.
    let owner = "OwnerOwnerOwnerOwner00";
    h.send(vec![
        LiveEvent::Hosting(hosting(None)),
        LiveEvent::Published(Published {
            server: "wss://figcraft.example.com/".into(),
            file: "f".into(),
            name: "Poster".into(),
            edit: "figcraft://join?server=wss://figcraft.example.com/&file=f&token=EditEditEditEditEdit00".into(),
            view: "figcraft://join?server=wss://figcraft.example.com/&file=f&token=ViewViewViewViewView00".into(),
            owner: Some(owner.into()),
            expires: Some(1_800_000_000),
        }),
    ]);
    let shapes = h.frames(3);
    assert_eq!(crate::live::inspect(&h.app)["ownerDialog"], true);
    assert!(has_text(&shapes, "File published") && has_text(&shapes, "Owner token") && has_text(&shapes, owner));
    assert!(has_text(
        &shapes,
        "This computer remembers the owner token, so the file can be managed from the home screen's Online list. Copy it to manage the file from somewhere else: it is a password for this file and won't be shown again."
    ));
    assert!(has_text(&shapes, "The server deletes this file if nobody opens it before 2027-01-15 08:00 UTC."));
    h.click(text_center(&shapes, "Done"));
    h.frames(2);
    assert_eq!(crate::live::inspect(&h.app)["ownerDialog"], false, "shown once");
}

#[test]
fn a_published_file_shows_its_links() {
    let (mut h, _) = Harness::new();
    joined(&mut h, Access::Edit);
    h.app.session.live_mut().unwrap().published = Some(Published {
        server: "127.0.0.1:47900".into(),
        file: "f".into(),
        name: "Team".into(),
        edit: "figcraft://join?host=127.0.0.1:47900&file=f&token=EditEditEditEditEdit00".into(),
        view: "figcraft://join?host=127.0.0.1:47900&file=f&token=ViewViewViewViewView00".into(),
        owner: None,
        expires: None,
    });
    h.send(vec![LiveEvent::Hosting(hosting(Some(1_800_000_000)))]);
    let shapes = h.frames(2);
    h.click(text_center(&shapes, "Share"));
    let shapes = h.frames(2);
    assert!(has_text(&shapes, "Published · nobody else here yet"));
    assert!(has_text(&shapes, "127.0.0.1:47900 · Can edit"));
    assert!(has_text(&shapes, "Edit link") && has_text(&shapes, "View link"));
    assert!(has_text(&shapes, "Leave session"));
    assert!(!has_text(&shapes, "Owner token"), "the owner token showed once, not here");
    // Where it is hosted, when it expires, and reporting it to the operator.
    assert!(has_text(&shapes, "Hosted by abuse@example.com") && has_text(&shapes, "Terms"));
    assert!(has_text(&shapes, "The server deletes this file if nobody opens it before 2027-01-15 08:00 UTC."));
    h.click(text_center(&shapes, "Report this file…"));
    let shapes = h.frames(2);
    assert!(has_text(&shapes, "What is wrong with this file? The server's operator gets your report."));
    h.app.ui.live.report = Some("Spam".into());
    let shapes = h.frames(2);
    h.click(text_center(&shapes, "Send report"));
    h.frames(2);
    assert!(h.app.ui.live.report.is_none(), "sent");
}

#[test]
fn a_view_link_shows_the_badge_and_only_the_moving_tools() {
    let (mut h, _) = Harness::new();
    h.app.session.execute("tool.set", &json!({"tool": "rectangle"})).unwrap();
    joined(&mut h, Access::View);
    let shapes = h.frames(2);
    assert!(has_text(&shapes, "View only"), "the badge");
    assert_eq!(h.app.session.tool.tool, ToolId::Move);
    assert_eq!(crate::live::inspect(&h.app)["readOnly"], VIEW_ONLY);
    // A tool's shortcut changes nothing, and says why.
    key(&mut h, Key::R);
    assert_eq!(h.app.session.tool.tool, ToolId::Move);
    assert!(h.app.ui.last_error.as_deref().is_some_and(|e| e.contains("View only")));
    // The Share popover says what the link allows.
    let shapes = h.frames(1);
    h.click(text_center(&shapes, "Share"));
    let shapes = h.frames(2);
    assert!(has_text(&shapes, "127.0.0.1:47900 · View only"));
    assert!(has_text(&shapes, "View only: you see every change as it happens but can't edit. Leave to keep a copy you can edit."));
    // Leaving: the copy is yours, the full toolbar is back.
    h.click(text_center(&shapes, "Leave session"));
    let shapes = h.frames(2);
    assert!(!has_text(&shapes, "View only"));
    assert!(h.app.session.execute("tool.set", &json!({"tool": "rectangle"})).is_ok());
}

#[test]
fn a_comment_link_shows_its_badge_and_leaves_the_comment_tool() {
    let (mut h, _) = Harness::new();
    h.app.session.execute("tool.set", &json!({"tool": "rectangle"})).unwrap();
    joined(&mut h, Access::Comment);
    let shapes = h.frames(2);
    assert!(has_text(&shapes, "Can comment"), "the badge");
    assert_eq!(h.app.session.tool.tool, ToolId::Move);
    assert_eq!(crate::live::inspect(&h.app)["readOnly"], COMMENT_ONLY);
    // The comment tool is in the pill and its shortcut works; an editing tool's says why not.
    h.app.session.execute("tool.set", &json!({"tool": "comment"})).unwrap();
    h.frames(2);
    assert_eq!(h.app.session.tool.tool, ToolId::Comment, "commenting stays");
    key(&mut h, Key::R);
    assert_eq!(h.app.session.tool.tool, ToolId::Comment);
    assert!(h.app.ui.last_error.as_deref().is_some_and(|e| e.contains("Can comment")));
    // The Share popover says what the link allows.
    let shapes = h.frames(1);
    h.click(text_center(&shapes, "Share"));
    let shapes = h.frames(2);
    assert!(has_text(&shapes, "127.0.0.1:47900 · Can comment"));
    assert!(has_text(&shapes, "Can comment: you see every change as it happens and can comment, but can't edit. Leave to keep a copy you can edit."));
}
