//! Sharing with a join code in the window: the Share popover not sharing (online or the local
//! network, one Start button, the rest under Advanced), sharing online (the code large, copying,
//! what it lets people do, who is here, stop: the file stays on the server, not shared, and can
//! be shared again or removed), sharing on the local network, a server without
//! codes, joining with the one join field (the popover, the home screen, the Join dialog), its
//! errors, the server preference, and an idle popover asking for no frames.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use egui::{Event, Key, Shape};
use figcraft_engine::figcraft_collab::code::{self, Local};
use figcraft_engine::figcraft_collab::message::ClientMsg;
use figcraft_engine::live::{
    Access, DEFAULT_SERVER, Event as LiveEvent, HostOptions, LiveLink, NetReply, NetRequest, Peer, Presence, PublishOptions, Published, Role,
    SessionInfo, Shared as SharedLinks, SharedLink, Status,
};
use serde_json::json;

use crate::live_tests::{Harness, has_text, text_center, texts};

/// What the link was asked to do, and the events it hands out next (as the session started last).
#[derive(Default)]
struct Script {
    hosted: Vec<HostOptions>,
    published: Vec<PublishOptions>,
    joined: Vec<String>,
    left: usize,
    events: Vec<LiveEvent>,
    session: u64,
    copied: Vec<String>,
    /// Link ids the server was asked to revoke, how often it was asked for new links, the files
    /// it was asked to remove, and its answers on their way.
    revoked: Vec<String>,
    shares: usize,
    removed: Vec<String>,
    answers: Vec<LiveEvent>,
    /// Hosted sessions serve the browser viewer.
    viewer: bool,
    /// The browser's link: it only joins.
    browser: bool,
}

/// The address the scripted host gives for watching from a phone.
const VIEWER: &str = "http://192.168.1.20:47900/#view&join=figcraft://join?host=192.168.1.20:47900&file=f&token=ViewViewViewViewView00";

/// The links a server makes for others the `n`th time a file is shared, with their codes.
fn links(n: usize, codes: Option<(&str, &str)>) -> SharedLinks {
    let link = |what: &str, code: Option<&str>| SharedLink {
        id: format!("{what}{n}"),
        link: format!("figcraft://join?server=wss://share.example:443/&file=f1&token=Share{what}Share{what}0{n}"),
        code: code.map(str::to_string),
    };
    SharedLinks { edit: link("edit", codes.map(|c| c.0)), view: Some(link("view", codes.map(|c| c.1))), comment: None }
}

type Shared = Arc<Mutex<Script>>;

struct Link(Shared);

/// Where the scripted host listens, and its session's secret.
const HOST: Local = Local { ip: std::net::Ipv4Addr::new(192, 168, 1, 20), port: 47_900, secret: 0x123_4567_89AB };
/// A link without a code in it (an address a code can't carry).
const PLAIN: &str = "figcraft://join?host=[fd00::1]:47900&file=f&token=AbCdEfGhIjKlMnOpQrStUv";

fn info(role: Role, link: &str) -> SessionInfo {
    SessionInfo {
        role,
        me: 1,
        name: "Ada".into(),
        color: "#14AE5C".into(),
        file_name: "Untitled".into(),
        pages: vec![],
        link: link.into(),
        url: String::new(),
        address: "192.168.1.20:47900".into(),
        network: true,
        access: Access::Edit,
    }
}

impl LiveLink for Link {
    fn can_host(&self) -> bool {
        !self.0.lock().unwrap().browser
    }
    fn serves_viewer(&self) -> bool {
        self.0.lock().unwrap().viewer
    }
    fn viewer(&self, _: u64) -> Option<String> {
        self.serves_viewer().then(|| VIEWER.to_string())
    }
    fn host(&mut self, session: u64, opts: HostOptions) -> Result<SessionInfo, String> {
        let mut s = self.0.lock().unwrap();
        s.session = session;
        let link = if opts.port == Some(6) { PLAIN.to_string() } else { HOST.link() };
        s.hosted.push(opts);
        Ok(info(Role::Host, &link))
    }
    fn join(&mut self, session: u64, link: &str, _: &str) -> Result<(), String> {
        let mut s = self.0.lock().unwrap();
        s.session = session;
        s.joined.push(link.to_string());
        Ok(())
    }
    fn publish(&mut self, session: u64, opts: PublishOptions) -> Result<(), String> {
        let mut s = self.0.lock().unwrap();
        s.session = session;
        s.published.push(opts);
        Ok(())
    }
    fn share(&mut self, _: &str, _: &str, _: &str, revoke: &[String], create: bool) -> Result<Option<SharedLinks>, String> {
        let mut s = self.0.lock().unwrap();
        s.revoked.extend(revoke.iter().cloned());
        s.shares += usize::from(create);
        let n = s.shares;
        Ok(create.then(|| links(n, Some((&format!("NEWED1T0{n}"), &format!("NEWV1EW0{n}"))))))
    }
    fn hosting(
        &mut self,
        _: &str,
        file: &str,
        _: &str,
        req: Option<figcraft_engine::figcraft_collab::message::Admin>,
    ) -> Result<(figcraft_engine::live::HostInfo, Option<figcraft_engine::figcraft_collab::message::AdminReply>), String> {
        use figcraft_engine::figcraft_collab::message::{Admin, AdminReply};
        if matches!(req, Some(Admin::Remove { .. })) {
            self.0.lock().unwrap().removed.push(file.to_string());
        }
        Ok((Default::default(), Some(AdminReply::Done)))
    }
    fn background(&mut self, job: u64, req: NetRequest) -> bool {
        // Answered at the next poll, as the network would.
        let reply: NetReply = self.request(req);
        self.0.lock().unwrap().answers.push(LiveEvent::Net { job, reply: Box::new(reply) });
        true
    }
    fn leave(&mut self, _: u64) {
        self.0.lock().unwrap().left += 1;
    }
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

/// A window on a file, with the scripted link and a server of the test's own in Preferences.
fn harness() -> (Harness, Shared) {
    let (mut h, _) = Harness::new();
    let script = Shared::default();
    h.app.session.live.install(Box::new(Link(script.clone())));
    h.app.services = Box::new(Clipboard(script.clone()));
    h.app.session.execute("pref.publishServer", &json!({"url": "wss://share.example/"})).unwrap();
    h.frames(3);
    (h, script)
}

/// Open the Share popover.
fn open(h: &mut Harness) -> Vec<Shape> {
    let shapes = h.frames(1);
    h.click(text_center(&shapes, "Share"));
    h.frames(2)
}

fn click(h: &mut Harness, shapes: &[Shape], text: &str) -> Vec<Shape> {
    h.click(text_center(shapes, text));
    h.frames(2)
}

fn key(h: &mut Harness, key: Key) {
    for pressed in [true, false] {
        h.frame(vec![Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Default::default() }]);
    }
}

/// Type into the field with the keyboard.
fn type_text(h: &mut Harness, text: &str) {
    h.frame(vec![Event::Text(text.into())]);
}

/// The file reached the server and the session is live, with these codes (if the server has any).
fn on_the_server(h: &mut Harness, script: &Shared, codes: Option<(&str, &str)>) -> Vec<Shape> {
    shared_on_the_server(h, script, links(0, codes))
}

/// [`on_the_server`] with the server's links for others as given.
fn shared_on_the_server(h: &mut Harness, script: &Shared, shared: SharedLinks) -> Vec<Shape> {
    let link = |token: &str| format!("figcraft://join?server=wss://share.example:443/&file=f1&token={token}");
    let published = Published {
        server: "wss://share.example:443/".into(),
        file: "f1".into(),
        name: "Untitled".into(),
        edit: link("EditEditEditEditEdit00"),
        view: link("ViewViewViewViewView00"),
        owner: Some("OwnerOwnerOwnerOwner00".into()),
        expires: None,
    };
    let welcome = LiveEvent::Welcome { info: info(Role::Guest, &published.edit), peers: vec![] };
    script.lock().unwrap().events.extend([LiveEvent::Published(published), LiveEvent::Shared(Ok(shared)), welcome]);
    h.frames(3)
}

/// The session's document arrives (the host's welcome): it opens in a tab of its own.
fn document_arrives(h: &mut Harness, script: &Shared) {
    use figcraft_engine::figcraft_collab::message::ServerMsg;
    use figcraft_engine::figcraft_collab::{Client, Limits, Outgoing, Server};
    let mut server = Server::new(h.app.session.doc().unwrap().clone(), "", Limits::default());
    let ClientMsg::Hello(me) = Client::hello("f", "", "Ada", "") else { panic!("no hello") };
    let (_, outs) = server.join(me).unwrap();
    let w = outs.into_iter().find_map(|o| if let Outgoing::To(_, w @ ServerMsg::Welcome { .. }) = o { Some(w) } else { None }).unwrap();
    script.lock().unwrap().events.push(LiveEvent::Sync(w));
    h.frames(3);
}

#[test]
fn not_sharing_the_popover_offers_one_way_to_start_and_one_to_join() {
    let (mut h, script) = harness();
    let shapes = open(&mut h);
    for t in ["Share this file", "Online", "Local network", "Start sharing", "Join with a code", "Join", "Advanced", "About this server", "Export…"]
    {
        assert!(has_text(&shapes, t), "the popover shows {t:?}");
    }
    assert!(has_text(&shapes, "People join from anywhere with a code. The file is kept on share.example while you share it."));
    // No addresses, keys or ports until Advanced is opened.
    for t in ["Server address", "Port", "This computer only", "Publish to another server…", "WebSocket URL", "Not in a live session"] {
        assert!(!has_text(&shapes, t), "not in the main flow: {t:?}");
    }
    let shapes = click(&mut h, &shapes, "Advanced");
    assert!(has_text(&shapes, "Publish to another server…") && !has_text(&shapes, "Port"));
    // The local network: what it means, and its own advanced part.
    let shapes = click(&mut h, &shapes, "Local network");
    assert!(has_text(&shapes, "People on your network join with a code. Nothing goes through a server."));
    assert!(has_text(&shapes, "This computer only") && has_text(&shapes, "Port") && !has_text(&shapes, "Publish to another server…"));
    assert!(!has_text(&shapes, "About this server"));
    assert_eq!(crate::live::inspect(&h.app)["local"], true);
    let shapes = click(&mut h, &shapes, "Advanced");
    assert!(!has_text(&shapes, "This computer only"), "closed again");
    // Nothing started, nothing asked of the network.
    let s = script.lock().unwrap();
    assert!(s.hosted.is_empty() && s.published.is_empty() && s.joined.is_empty());
    assert_eq!(h.app.session.live().status, Status::Off);
}

#[test]
fn the_server_key_shows_as_dots_while_shown_and_typed() {
    let (mut h, _) = harness();
    let shapes = open(&mut h);
    click(&mut h, &shapes, "Advanced");
    h.app.ui.live.share_key = Some(("wss://share.example/".to_string(), "s3cret-key".to_string()));
    let shapes = h.frames(2);
    let dots = "•".repeat(10);
    assert!(has_text(&shapes, &dots) && !has_text(&shapes, "s3cret-key"));
    // Editing it keeps it hidden.
    let shapes = click(&mut h, &shapes, &dots);
    assert!(!has_text(&shapes, "s3cret-key"));
}

#[test]
fn sharing_online_shows_the_code_copies_it_and_stops() {
    let (mut h, script) = harness();
    let shapes = open(&mut h);
    let shapes = click(&mut h, &shapes, "Start sharing");
    {
        let s = script.lock().unwrap();
        let [p] = s.published.as_slice() else { panic!("published {}", s.published.len()) };
        assert!(p.server == "wss://share.example/" && p.key.is_none() && p.share, "the server in Preferences, shared once there");
    }
    assert!(has_text(&shapes, "Starting to share…") && has_text(&shapes, "Stop sharing"));
    assert!(!has_text(&shapes, "Start sharing"));
    let shapes = on_the_server(&mut h, &script, Some(("K7M2QX9TD", "4FHJ8R2WN")));
    for t in
        ["Live session", "Sharing online · nobody else here yet", "K7M-2QX-9TD", "Copy code", "Copy link", "Can edit", "Can view", "Stop sharing"]
    {
        assert!(has_text(&shapes, t), "sharing shows {t:?}");
    }
    assert!(has_text(&shapes, "Anyone with this code can edit the file.") && has_text(&shapes, "Here: Ada (you)"));
    assert!(!has_text(&shapes, "Can comment"), "this server made no comment link");
    // The code is the largest text in the popover.
    let size = |s: &str| texts(&shapes).into_iter().find(|(t, _, _)| t == s).map(|(_, _, size)| size.y).unwrap();
    assert!(size("K7M-2QX-9TD") > 1.5 * size("Copy code"));
    // No owner token dialog in this flow, and no link to read.
    assert_eq!(crate::live::inspect(&h.app)["ownerDialog"], false);
    assert!(!has_text(&shapes, "Edit link") && !has_text(&shapes, "Owner token"));
    assert_eq!(crate::live::inspect(&h.app)["code"], "K7M-2QX-9TD");
    // Copying.
    let shapes = click(&mut h, &shapes, "Copy code");
    let shapes = click(&mut h, &shapes, "Copy link");
    // The link is one made for others, not the owner's own.
    assert_eq!(script.lock().unwrap().copied, ["K7M-2QX-9TD", "figcraft://join?server=wss://share.example:443/&file=f1&token=ShareeditShareedit00"]);
    // Someone joins: they are listed.
    script.lock().unwrap().events.push(LiveEvent::PeerJoined(Peer { client_id: 2, name: "Hana".into(), color: "#9747FF".into() }));
    let shapes_now = h.frames(2);
    assert!(has_text(&shapes_now, "Sharing online · 1 person joined") && has_text(&shapes_now, "Here: Ada (you), Hana"));
    // What the code lets people do: the view code and link take the edit ones' place.
    let shapes = click(&mut h, &shapes, "Can view");
    assert!(has_text(&shapes, "4FH-J8R-2WN") && !has_text(&shapes, "K7M-2QX-9TD"));
    assert!(has_text(&shapes, "Anyone with this code can look at the file, not change it."));
    let shapes = click(&mut h, &shapes, "Copy link");
    assert!(script.lock().unwrap().copied.last().unwrap().ends_with("token=ShareviewShareview00"));
    let shapes = click(&mut h, &shapes, "Can edit");
    assert!(has_text(&shapes, "K7M-2QX-9TD"));
    // Stop: the others' links are revoked; the file stays on the server and you stay in it.
    click(&mut h, &shapes, "Stop sharing");
    let shapes = h.frames(3);
    assert_eq!(script.lock().unwrap().revoked, ["edit0", "view0"]);
    assert_eq!((h.app.session.live().status.clone(), script.lock().unwrap().left), (Status::Live, 0));
    for t in
        ["Not shared", "This file is on share.example, open only to you. Share it to let others in with a new code.", "Start sharing", "Advanced"]
    {
        assert!(has_text(&shapes, t), "not shared shows {t:?}");
    }
    assert!(
        !has_text(&shapes, "K7M-2QX-9TD")
            && !has_text(&shapes, "Stop sharing")
            && !has_text(&shapes, "Edit link")
            && !has_text(&shapes, "Local network")
    );
    assert_eq!(crate::live::inspect(&h.app)["share"]["shared"], false);
    // Share again: a new code for the same file, nothing published twice.
    click(&mut h, &shapes, "Start sharing");
    let shapes = h.frames(3);
    assert!(has_text(&shapes, "NEW-ED1-T01") && has_text(&shapes, "Stop sharing") && !has_text(&shapes, "K7M-2QX-9TD"));
    let s = script.lock().unwrap();
    assert_eq!((s.published.len(), s.shares), (1, 1));
}

#[test]
fn a_server_with_comment_links_offers_a_comment_code() {
    let (mut h, script) = harness();
    let shapes = open(&mut h);
    click(&mut h, &shapes, "Start sharing");
    let mut shared = links(0, Some(("K7M2QX9TD", "4FHJ8R2WN")));
    shared.comment = Some(SharedLink {
        id: "comment0".into(),
        link: "figcraft://join?server=wss://share.example:443/&file=f1&token=ShareCommentShareComm00".into(),
        code: Some("MNP4RT7WX".into()),
    });
    let shapes = shared_on_the_server(&mut h, &script, shared);
    for t in ["Can edit", "Can comment", "Can view", "K7M-2QX-9TD"] {
        assert!(has_text(&shapes, t), "sharing shows {t:?}");
    }
    // What the code lets people do: commenting, between editing and viewing.
    let x = |t: &str| text_center(&shapes, t).x;
    assert!(x("Can edit") < x("Can comment") && x("Can comment") < x("Can view"));
    let shapes = click(&mut h, &shapes, "Can comment");
    assert!(has_text(&shapes, "MNP-4RT-7WX") && !has_text(&shapes, "K7M-2QX-9TD"));
    assert!(has_text(&shapes, "Anyone with this code can look at the file and comment on it, not change it."));
    assert_eq!(crate::live::inspect(&h.app)["share"]["access"], "comment");
    let shapes = click(&mut h, &shapes, "Copy link");
    assert!(script.lock().unwrap().copied.last().unwrap().ends_with("token=ShareCommentShareComm00"));
    let shapes = click(&mut h, &shapes, "Can view");
    assert!(has_text(&shapes, "4FH-J8R-2WN") && has_text(&shapes, "Anyone with this code can look at the file, not change it."));
    // Stopping takes the comment link away with the others.
    click(&mut h, &shapes, "Stop sharing");
    h.frames(3);
    assert_eq!(script.lock().unwrap().revoked, ["edit0", "view0", "comment0"]);
}

#[test]
fn a_file_that_is_not_shared_can_be_removed_from_the_server_after_asking() {
    let (mut h, script) = harness();
    let shapes = open(&mut h);
    click(&mut h, &shapes, "Start sharing");
    let shapes = on_the_server(&mut h, &script, Some(("K7M2QX9TD", "4FHJ8R2WN")));
    click(&mut h, &shapes, "Stop sharing");
    let shapes = h.frames(3);
    // Removing is its own step under Advanced, and asks first.
    assert!(!has_text(&shapes, "Remove from server…"));
    let shapes = click(&mut h, &shapes, "Advanced");
    assert!(has_text(&shapes, "Remove from server…") && has_text(&shapes, "Leave session"));
    let shapes = click(&mut h, &shapes, "Remove from server…");
    assert!(has_text(&shapes, "Remove this file from the server? It can't be shared or opened from there again. Your copy stays open here."));
    assert!(script.lock().unwrap().removed.is_empty(), "nothing yet");
    let shapes = click(&mut h, &shapes, "Cancel");
    assert!(has_text(&shapes, "Remove from server…") && script.lock().unwrap().removed.is_empty());
    let shapes = click(&mut h, &shapes, "Remove from server…");
    click(&mut h, &shapes, "Remove");
    let shapes = h.frames(3);
    assert_eq!(script.lock().unwrap().removed, ["f1"]);
    assert_eq!(h.app.session.live().status, Status::Off);
    assert!(has_text(&shapes, "Share this file") && has_text(&shapes, "Removed from the server"));
}

#[test]
fn sharing_on_the_local_network_shows_its_longer_code() {
    let (mut h, script) = harness();
    let shapes = open(&mut h);
    let shapes = click(&mut h, &shapes, "Local network");
    let shapes = click(&mut h, &shapes, "Start sharing");
    let shown = code::grouped(&HOST.code());
    for t in ["Sharing on your network · nobody else here yet", shown.as_str(), "Copy code", "Copy link", "Stop sharing"] {
        assert!(has_text(&shapes, t), "sharing shows {t:?}");
    }
    assert!(has_text(&shapes, "Anyone on your network with this code can edit the file, while this app stays open."));
    assert!(!has_text(&shapes, "Can view"), "everyone edits on the local network");
    {
        let s = script.lock().unwrap();
        assert!(s.hosted[0].short && s.hosted[0].bind == figcraft_engine::live::Bind::Lan && s.hosted[0].port.is_none());
        assert!(s.published.is_empty(), "nothing goes to a server");
    }
    let shapes = click(&mut h, &shapes, "Copy code");
    let shapes = click(&mut h, &shapes, "Copy link");
    assert_eq!(script.lock().unwrap().copied, [shown, HOST.link()]);
    // Advanced: this computer only, on a port of your choosing.
    let shapes = click(&mut h, &shapes, "Stop sharing");
    let shapes = click(&mut h, &shapes, "Advanced");
    let shapes = click(&mut h, &shapes, "This computer only");
    h.app.ui.live.port = "6".into();
    click(&mut h, &shapes, "Start sharing");
    let s = script.lock().unwrap();
    assert!(s.hosted[1].bind == figcraft_engine::live::Bind::Local && s.hosted[1].port == Some(6));
}

#[test]
fn without_a_code_the_link_is_shared_instead() {
    // A server from before join codes.
    let (mut h, script) = harness();
    let shapes = open(&mut h);
    click(&mut h, &shapes, "Start sharing");
    let shapes = on_the_server(&mut h, &script, None);
    assert!(has_text(&shapes, "Sharing online · nobody else here yet") && has_text(&shapes, "Invite link") && has_text(&shapes, "Copy"));
    assert!(has_text(
        &shapes,
        "No join code here (the server is older than join codes, or this address can't be put in one): share the link instead."
    ));
    assert!(!has_text(&shapes, "Copy code"));
    let shapes = click(&mut h, &shapes, "Copy");
    assert!(script.lock().unwrap().copied[0].ends_with("token=ShareeditShareedit00"));
    // Stopping works without codes too (links are older than codes).
    click(&mut h, &shapes, "Stop sharing");
    let shapes = h.frames(3);
    assert!(has_text(&shapes, "Not shared") && script.lock().unwrap().revoked == ["edit0", "view0"]);
    // A host whose address a code can't carry.
    h.app.session.execute("collab.leave", &json!({})).unwrap();
    let shapes = h.frames(2);
    let shapes = click(&mut h, &shapes, "Local network");
    h.app.ui.live.port = "6".into();
    let shapes = click(&mut h, &shapes, "Start sharing");
    assert!(has_text(&shapes, "Invite link") && !has_text(&shapes, "Copy code"));
    click(&mut h, &shapes, "Copy");
    assert_eq!(script.lock().unwrap().copied.last().map(String::as_str), Some(PLAIN));
}

/// Asserts the join field accepts a typed code or invite link, joins on Enter, clears after
/// joining, shows failure/retry, rejects an invalid code with an error, and accepts local
/// codes and links.
#[test]
fn the_join_field_takes_a_code_or_a_link_and_enter_joins() {
    let (mut h, script) = harness();
    let shapes = open(&mut h);
    // Click into the field (its hint), type a code the way people type it, press Enter.
    click(&mut h, &shapes, "Code or invite link");
    assert!(h.ctx.memory(|m| m.has_focus(egui::Id::unique("live_join_code"))));
    type_text(&mut h, " k7m 2qx-9td ");
    assert_eq!(crate::live::inspect(&h.app)["joinField"], " k7m 2qx-9td ");
    key(&mut h, Key::Enter);
    assert_eq!(script.lock().unwrap().joined, ["figcraft://join?server=wss://share.example/&code=K7M2QX9TD"]);
    assert!(h.app.ui.live.code.is_empty(), "the field is cleared once the join started");
    let shapes = h.frames(2);
    assert!(has_text(&shapes, "Joining…") && has_text(&shapes, "Leave session"));
    // The server doesn't know the code: the popover says so and offers the field again.
    script.lock().unwrap().events.push(LiveEvent::Status(Status::Failed { reason: "No session with that code".into() }));
    let shapes = h.frames(3);
    assert!(has_text(&shapes, "No session with that code") && has_text(&shapes, "Join with a code") && has_text(&shapes, "Start sharing"));
    // Something that can't be a code: it says why and stays to be fixed (the Join button).
    click(&mut h, &shapes, "Code or invite link");
    type_text(&mut h, "K7M-2QX");
    let shapes = h.frames(1);
    click(&mut h, &shapes, "Join");
    assert!(h.app.ui.last_error.as_deref().is_some_and(|e| e.contains("9 characters")), "{:?}", h.app.ui.last_error);
    assert_eq!(h.app.ui.live.code, "K7M-2QX");
    assert_eq!(script.lock().unwrap().joined.len(), 1);
    // A local network code and a pasted link go through the same field.
    for (typed, want) in [(code::grouped(&HOST.code()), HOST.link()), (PLAIN.to_string(), PLAIN.to_string())] {
        h.app.ui.live.code = typed;
        let shapes = h.frames(1);
        click(&mut h, &shapes, "Join");
        assert_eq!(script.lock().unwrap().joined.last(), Some(&want));
        h.app.session.execute("collab.leave", &json!({})).unwrap();
        h.frames(2);
    }
}

#[test]
fn the_home_screen_joins_with_a_code_and_opens_the_file() {
    let (mut h, script) = harness();
    h.app.ui.home = true;
    let shapes = h.frames(2);
    assert!(has_text(&shapes, "Join with a code") && has_text(&shapes, "Code or invite link") && has_text(&shapes, "Join"));
    click(&mut h, &shapes, "Code or invite link");
    type_text(&mut h, "K7M-2QX-9TD");
    key(&mut h, Key::Enter);
    assert_eq!(script.lock().unwrap().joined, ["figcraft://join?server=wss://share.example/&code=K7M2QX9TD"]);
    let shapes = h.frames(2);
    assert!(has_text(&shapes, "Joining live session…") && h.app.ui.home, "the Join dialog shows the join");
    // It failed: the dialog says why, and the home screen stays.
    script.lock().unwrap().events.push(LiveEvent::Status(Status::Failed { reason: "That code has expired or its sharing was stopped".into() }));
    let shapes = h.frames(3);
    assert!(has_text(&shapes, "That code has expired or its sharing was stopped") && h.app.ui.home);
    // Again, and it works: the shared file's tab comes to the front.
    h.app.ui.live.code = "K7M-2QX-9TD".into();
    let shapes = h.frames(1);
    click(&mut h, &shapes, "Join");
    assert_eq!(script.lock().unwrap().joined.len(), 2);
    script.lock().unwrap().events.push(LiveEvent::Welcome { info: info(Role::Guest, PLAIN), peers: vec![] });
    h.frames(2);
    document_arrives(&mut h, &script);
    assert!(!h.app.ui.home, "the joined file shows");
    assert_eq!(h.app.session.docs.len(), 2);
}

/// Asserts the join dialog uses the same shared join field, focuses it, and shows the join once
/// it starts.
#[test]
fn the_join_dialog_is_the_same_field() {
    let (mut h, script) = harness();
    crate::menus::invoke(&mut h.app, "collab.join");
    let shapes = h.frames(2);
    assert!(has_text(&shapes, "Type the join code you were given, or paste an invite link.") && has_text(&shapes, "Code or invite link"));
    assert!(h.ctx.memory(|m| m.has_focus(egui::Id::unique("live_join_link"))), "the field has the keyboard");
    type_text(&mut h, "4fh j8r 2wn");
    key(&mut h, Key::Enter);
    h.frames(1);
    assert_eq!(script.lock().unwrap().joined, ["figcraft://join?server=wss://share.example/&code=4FHJ8R2WN"]);
    assert!(h.app.ui.live.join.is_none(), "its field goes once the join started");
    assert!(has_text(&h.frames(1), "Joining live session…"), "and it shows the join");
}

#[test]
fn the_server_preference_resets_and_turns_online_sharing_off() {
    let (mut h, script) = harness();
    h.app.ui.preferences = true;
    let shapes = h.frames(2);
    assert!(has_text(&shapes, "Server for online sharing") && has_text(&shapes, "Reset"));
    click(&mut h, &shapes, "Reset");
    assert_eq!(h.app.session.prefs.publish_server, DEFAULT_SERVER);
    // Off: Preferences say what that means, and the popover offers the local network only.
    h.app.session.execute("pref.publishServer", &json!({"url": ""})).unwrap();
    let shapes = h.frames(2);
    assert!(has_text(&shapes, "Online sharing is off: files are shared on the local network only."));
    h.app.ui.preferences = false;
    h.frames(2);
    let shapes = open(&mut h);
    assert!(has_text(&shapes, "Sharing on the local network (online sharing is off: no server is set in Preferences)."));
    assert!(!has_text(&shapes, "Online") && has_text(&shapes, "Start sharing"));
    click(&mut h, &shapes, "Start sharing");
    let s = script.lock().unwrap();
    assert!(s.published.is_empty() && s.hosted.len() == 1, "shared on the local network");
}

#[test]
fn a_key_typed_for_one_server_is_not_sent_to_another() {
    // The Server key field remembers the server it was typed for: it goes out only to that
    // server. Changing the server in Preferences must not send the old server's key to the new
    // one. (The key is set as the field editor would, bound to the server it was typed for.)
    let (mut h, script) = harness();
    h.app.ui.live.share_key = Some(("wss://share.example/".to_string(), "KeyKeyKeyKeyKeyKeyKey0".to_string()));
    h.frames(2);
    // Change the server in Preferences to another one, then share: the old key must not go to it.
    h.app.session.execute("pref.publishServer", &json!({"url": "wss://other.example/"})).unwrap();
    h.frames(2);
    let shapes = open(&mut h);
    click(&mut h, &shapes, "Start sharing");
    assert_eq!(script.lock().unwrap().published.last().unwrap().key, None, "the old server's key is not sent to the new server");
}

#[test]
fn an_open_popover_asks_for_no_frames() {
    let (mut h, script) = harness();
    let shapes = open(&mut h);
    assert!(h.goes_quiet(), "not sharing");
    click(&mut h, &shapes, "Start sharing");
    on_the_server(&mut h, &script, Some(("K7M2QX9TD", "4FHJ8R2WN")));
    assert!(h.goes_quiet(), "sharing, with the code showing");
    let (shapes, delay) = h.frame(vec![]);
    assert!(has_text(&shapes, "K7M-2QX-9TD") && delay >= Duration::from_secs(1));
}

#[test]
fn view_on_another_device_shares_locally_and_shows_the_address_as_a_qr_code() {
    let (mut h, script) = harness();
    // A build without the browser viewer offers it greyed out.
    let shapes = open(&mut h);
    assert!(has_text(&shapes, "View on another device"));
    let shapes = click(&mut h, &shapes, "View on another device");
    assert!(script.lock().unwrap().hosted.is_empty(), "nothing is shared");
    assert!(has_text(&shapes, "Start sharing"));

    script.lock().unwrap().viewer = true;
    // (A frame for the button to take clicks again.)
    let shapes = h.frames(2);
    // It asks first: the file goes, unencrypted, to the whole network.
    let shapes = click(&mut h, &shapes, "View on another device");
    assert!(has_text(&shapes, "Share on this network?") && has_text(&shapes, DEVICE_CONFIRM));
    assert!(!has_text(&shapes, "Start sharing"), "one question, one button to share");
    assert!(script.lock().unwrap().hosted.is_empty(), "nothing is shared before it is confirmed");
    let shapes = click(&mut h, &shapes, "Share on this network");
    {
        let s = script.lock().unwrap();
        let [hosted] = s.hosted.as_slice() else { panic!("hosted {}", s.hosted.len()) };
        assert!(hosted.short && hosted.bind == figcraft_engine::live::Bind::Lan, "on the local network, where the phone is");
        assert!(s.published.is_empty(), "nothing goes to a server");
    }
    for t in ["Shared on this network", "Copy address", "Copy code", "Stop sharing"] {
        assert!(has_text(&shapes, t), "sharing shows {t:?}");
    }
    assert!(has_text(
        &shapes,
        "Scan it with a phone on the same network, or open the address in a browser there: it plays the prototype and follows your edits, view only, while this app stays open. The connection isn't encrypted: anyone on this network who sees the code or the address can watch."
    ));
    // Stop sharing once, red, right under the address.
    assert_eq!(texts(&shapes).iter().filter(|(s, _, _)| s == "Stop sharing").count(), 1);
    let danger = crate::theme::Tokens::get(&h.ctx).danger;
    assert!(shapes.iter().any(|s| matches!(s, Shape::Rect(r) if r.fill == danger)), "a red Stop sharing");
    assert!(text_center(&shapes, "Stop sharing").y > text_center(&shapes, "Copy address").y);
    let here = texts(&shapes).into_iter().find(|(s, _, _)| s.starts_with("Here: ")).map(|(_, p, _)| p.y).unwrap();
    assert!(text_center(&shapes, "Stop sharing").y < here, "above who is here");
    let cells = crate::qr::cells(VIEWER).unwrap();
    let dark = cells.iter().flatten().filter(|c| **c).count();
    let squares = shapes.iter().filter(|s| matches!(s, Shape::Rect(r) if r.fill == egui::Color32::BLACK)).count();
    assert!(squares > 0 && squares <= dark, "the QR code is drawn ({squares} dark runs for {dark} dark cells)");
    assert_eq!(crate::live::inspect(&h.app)["viewer"], VIEWER);
    let shapes = click(&mut h, &shapes, "Copy address");
    assert_eq!(script.lock().unwrap().copied, [VIEWER]);
    // It folds away; the share goes on, still red at the bottom.
    let shapes = click(&mut h, &shapes, "View on another device");
    assert!(!has_text(&shapes, "Copy address") && has_text(&shapes, "Stop sharing"));
    assert!(shapes.iter().any(|s| matches!(s, Shape::Rect(r) if r.fill == danger)));
    let shapes = click(&mut h, &shapes, "Stop sharing");
    // A share started the usual way has it too, folded.
    let shapes = click(&mut h, &shapes, "Local network");
    let shapes = click(&mut h, &shapes, "Start sharing");
    assert!(has_text(&shapes, "View on another device") && !has_text(&shapes, "Copy address"));
}

/// What View on another device asks first.
const DEVICE_CONFIRM: &str = "Anyone on this network who sees the QR code or the address can watch the prototype while this app stays open. The connection isn't encrypted: use networks you trust.";

#[test]
fn view_on_another_device_can_be_cancelled_or_stop_asking() {
    let (mut h, script) = harness();
    script.lock().unwrap().viewer = true;
    let shapes = open(&mut h);
    let shapes = click(&mut h, &shapes, "View on another device");
    // Cancel: nothing shared, the button is back.
    let shapes = click(&mut h, &shapes, "Cancel");
    assert!(script.lock().unwrap().hosted.is_empty());
    assert!(!has_text(&shapes, DEVICE_CONFIRM) && has_text(&shapes, "View on another device"));
    assert!(has_text(&shapes, "Start sharing"), "back with the question gone");
    // Don't show again: shared, and the preference is off.
    let shapes = click(&mut h, &shapes, "View on another device");
    let shapes = click(&mut h, &shapes, "Don't show again");
    let shapes = click(&mut h, &shapes, "Share on this network");
    assert_eq!(script.lock().unwrap().hosted.len(), 1);
    assert!(!h.app.session.prefs.confirm_view_on_device);
    // Next time it shares at once.
    let shapes = click(&mut h, &shapes, "Stop sharing");
    let shapes = click(&mut h, &shapes, "Local network");
    let shapes = click(&mut h, &shapes, "View on another device");
    assert!(!has_text(&shapes, DEVICE_CONFIRM));
    assert_eq!(script.lock().unwrap().hosted.len(), 2);
    // Without ticking it, it asks again next time.
    h.app.run("pref.confirmViewOnDevice", json!({"on": true}));
    let shapes = click(&mut h, &shapes, "Stop sharing");
    let shapes = click(&mut h, &shapes, "View on another device");
    let shapes = click(&mut h, &shapes, "Share on this network");
    assert!(h.app.session.prefs.confirm_view_on_device);
    let shapes = click(&mut h, &shapes, "Stop sharing");
    let shapes = click(&mut h, &shapes, "View on another device");
    assert!(has_text(&shapes, DEVICE_CONFIRM));
}

#[test]
fn preferences_can_ask_again_where_a_phone_can_watch() {
    let (mut h, script) = harness();
    h.app.ui.preferences = true;
    assert!(!has_text(&h.frames(2), "Ask before viewing on another device"), "not without the browser viewer");
    script.lock().unwrap().viewer = true;
    h.app.session.prefs.confirm_view_on_device = false;
    let shapes = h.frames(2);
    assert!(has_text(&shapes, "Ask before viewing on another device"));
    click(&mut h, &shapes, "Ask before viewing on another device");
    assert!(h.app.session.prefs.confirm_view_on_device);
}

#[test]
fn a_qr_code_holds_the_address() {
    let cells = crate::qr::cells(VIEWER).unwrap();
    // Version 7 or so: square, with the three finder patterns' dark corners.
    let n = cells.len();
    assert!((21..=57).contains(&n) && cells.iter().all(|r| r.len() == n), "{n}");
    for (x, y) in [(0, 0), (n - 1, 0), (0, n - 1)] {
        assert!(cells[y][x], "finder corner at {x},{y}");
    }
    assert!(crate::qr::cells(&"x".repeat(8000)).is_none(), "too long for any QR code");
}

#[test]
fn the_device_viewer_presents_the_shared_file_and_says_when_it_ended() {
    let (mut h, script) = harness();
    // The shared file has a frame to present.
    h.app.session.execute("frame.create", &json!({"x": 0, "y": 0, "width": 390, "height": 844})).unwrap();
    crate::viewer::open(&mut h.app, Some(VIEWER.split_once("join=").unwrap().1));
    let shapes = h.frames(2);
    assert_eq!(script.lock().unwrap().joined.len(), 1, "it joins the address's link");
    assert!(has_text(&shapes, "Opening the shared file…"));
    assert!(!has_text(&shapes, "Share"), "no editor");
    script.lock().unwrap().events.push(LiveEvent::Welcome { info: SessionInfo { access: Access::View, ..info(Role::Guest, PLAIN) }, peers: vec![] });
    h.frames(2);
    document_arrives(&mut h, &script);
    let shapes = h.frames(2);
    assert!(h.app.ui.present.is_some(), "the prototype plays");
    assert_eq!(h.app.ui.present_view, crate::viewer::present_view(), "sized for the phone, no device drawn around it");
    assert!(has_text(&shapes, "Restart"));
    for t in ["Close", "Options", "Inspect", "Comments"] {
        assert!(!has_text(&shapes, t), "nothing to go back to or change: {t:?}");
    }
    // Escape has nothing to close to.
    key(&mut h, Key::Escape);
    assert!(h.app.ui.present.is_some());
    // The host stops: the copy shown isn't the shared file any more.
    script.lock().unwrap().events.push(LiveEvent::Ended { reason: "The host ended the session".into() });
    let shapes = h.frames(3);
    assert!(h.app.ui.present.is_none());
    assert!(has_text(&shapes, "The session ended. Share ▸ View on another device on the computer gives a new address."));
}

#[test]
fn a_device_viewer_without_a_link_or_frames_says_why() {
    let (mut h, script) = harness();
    crate::viewer::open(&mut h.app, None);
    let shapes = h.frames(2);
    assert!(has_text(
        &shapes,
        "Couldn't open the shared file.\n\nThis address doesn't name a shared file. Open the address Share ▸ View on another device shows."
    ));
    assert!(script.lock().unwrap().joined.is_empty());
    // A file with nothing to present.
    let (mut h, script) = harness();
    crate::viewer::open(&mut h.app, Some("figcraft://join?host=192.168.1.20:47900&file=f&token=V"));
    script.lock().unwrap().events.push(LiveEvent::Welcome { info: info(Role::Guest, PLAIN), peers: vec![] });
    h.frames(2);
    document_arrives(&mut h, &script);
    let shapes = h.frames(2);
    assert!(h.app.ui.present.is_none());
    assert!(has_text(&shapes, "Nothing to present yet: add a frame to the file on the computer sharing it."));
}

#[test]
fn the_viewer_stage_follows_the_session() {
    use crate::viewer::{Stage, stage};
    assert_eq!(stage(&Status::Off, false, false), Stage::Opening);
    assert_eq!(stage(&Status::Connecting, false, false), Stage::Opening);
    assert_eq!(stage(&Status::Live, false, true), Stage::Opening, "live, the file on its way");
    assert_eq!(stage(&Status::Live, true, true), Stage::Present);
    assert_eq!(stage(&Status::Reconnecting { attempt: 2 }, true, true), Stage::Reconnecting);
    assert_eq!(stage(&Status::Failed { reason: "gone".into() }, false, false), Stage::Failed("gone".into()));
    assert_eq!(stage(&Status::Off, false, true), Stage::Ended);
}

/// A file of two screens: a button on the first navigates to the second.
fn two_screens(h: &mut Harness) -> (u64, u64, u64) {
    let id = |v: serde_json::Value| v["id"].as_u64().unwrap();
    let s = &mut h.app.session;
    let a = id(s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 390, "height": 844})).unwrap());
    let b = id(s.execute("frame.create", &json!({"x": 500, "y": 0, "width": 390, "height": 844})).unwrap());
    let button = id(s.execute("shape.rectangle", &json!({"x": 40, "y": 40, "width": 300, "height": 60, "parent": a})).unwrap());
    s.execute("proto.interaction.add", &json!({"id": button, "dest": b})).unwrap();
    s.execute("proto.flow.add", &json!({"start": a})).unwrap();
    (a, b, button)
}

fn renders(h: &FigApp) -> u64 {
    h.ui.present.as_ref().unwrap().inspect()["renders"].as_u64().unwrap()
}

use crate::FigApp;

#[test]
fn the_device_viewer_draws_the_next_screen_ahead_and_never_twice() {
    let (mut h, script) = harness();
    let (a, b, button) = two_screens(&mut h);
    crate::viewer::open(&mut h.app, Some("figcraft://join?host=192.168.1.20:47900&file=f&token=V"));
    script.lock().unwrap().events.push(LiveEvent::Welcome { info: SessionInfo { access: Access::View, ..info(Role::Guest, PLAIN) }, peers: vec![] });
    h.frames(2);
    document_arrives(&mut h, &script);
    h.frames(2);
    let pres = h.app.ui.present.as_ref().unwrap();
    assert_eq!(pres.inspect()["current"], json!(a));
    assert_eq!(pres.max_px, Some(2.0), "a phone draws at most 2 pixels a point");
    assert_eq!(renders(&h.app), 1, "the first screen, at once");
    // Nothing is drawn ahead while the screen is being used.
    h.frames(30);
    assert_eq!(renders(&h.app), 1, "not before a pause");
    // After a pause the screen the button leads to is drawn, smaller.
    h.skip(1.5);
    h.frames(3);
    assert_eq!(renders(&h.app), 2, "drawn ahead");
    h.frames(5);
    assert_eq!(renders(&h.app), 2, "once");
    // The tap shows it at once, from the drawing made ahead.
    let place = h.app.ui.present.as_ref().unwrap().shown_place().unwrap();
    let r = h.app.session.doc().unwrap().world_bounds(figcraft_doc::NodeId(button)).unwrap();
    h.click(place.rect(r).center());
    h.frames(5);
    assert_eq!(h.app.ui.present.as_ref().unwrap().inspect()["current"], json!(b));
    assert_eq!(renders(&h.app), 2, "shown without drawing");
    // At the next pause it sharpens, once.
    h.skip(1.5);
    h.frames(5);
    assert_eq!(renders(&h.app), 3, "drawn sharp");
    h.skip(1.5);
    h.frames(5);
    assert_eq!(renders(&h.app), 3, "nothing drawn twice");
}

#[test]
fn a_frame_draws_the_same_until_what_it_shows_changes() {
    use crate::present::same_picture;
    let mut s = figcraft_engine::Session::new();
    let a = s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    let b = s.execute("frame.create", &json!({"x": 200, "y": 0, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    let before = s.doc().unwrap().clone();
    let (a, b) = (figcraft_doc::NodeId(a), figcraft_doc::NodeId(b));
    assert!(same_picture(&before, &before.clone(), a));
    s.execute("shape.rectangle", &json!({"x": 220, "y": 20, "width": 10, "height": 10, "parent": b.0})).unwrap();
    let after = s.doc().unwrap().clone();
    assert!(same_picture(&before, &after, a), "another frame changed");
    assert!(!same_picture(&before, &after, b), "a layer was added to it");
    s.execute("node.set", &json!({"id": a.0, "props": {"opacity": 0.5}})).unwrap();
    assert!(!same_picture(&after, s.doc().unwrap(), a), "the frame itself changed");
    // A variable mode set on the page (or a section around the frame) changes what it shows.
    s.execute("variable.createCollection", &json!({"name": "Theme", "modes": ["Light", "Dark"]})).unwrap();
    s.execute("variable.create", &json!({"collection": "Theme", "name": "bg", "type": "color", "values": {"Light": "#FFFFFF", "Dark": "#000000"}}))
        .unwrap();
    let light = s.doc().unwrap().clone();
    let page = s.ds().unwrap().page;
    s.execute("variable.setMode", &json!({"ids": [page.0], "collection": "Theme", "mode": "Dark"})).unwrap();
    assert!(!same_picture(&light, s.doc().unwrap(), a), "the page's mode changed");
}

/// The Actions palette rows for `q`, by key.
fn palette_keys(app: &FigApp, q: &str) -> Vec<String> {
    crate::menus::palette_sections(app, q, crate::menus::PaletteTab::All).into_iter().flat_map(|(_, rows)| rows).map(|r| r.key).collect()
}

#[test]
fn view_on_another_device_is_offered_only_where_it_can_work() {
    const KEY: &str = "command:collab.viewOnDevice";
    // A desktop build with the browser viewer: in the popover and the Actions palette.
    let (mut h, script) = harness();
    script.lock().unwrap().viewer = true;
    assert!(has_text(&open(&mut h), "View on another device"));
    assert!(palette_keys(&h.app, "another device").iter().any(|k| k == KEY));
    // Without it: greyed out in the popover (it says why), not in the palette.
    let (mut h, _) = harness();
    assert!(has_text(&open(&mut h), "View on another device"));
    assert!(!palette_keys(&h.app, "another device").iter().any(|k| k == KEY));
    // A recent action that can't run here isn't offered either.
    h.app.session.prefs.recent_actions = vec![KEY.into()];
    assert!(!palette_keys(&h.app, "").iter().any(|k| k == KEY));
    // The browser build hosts nothing: neither shows it, also where it shares online.
    let (mut h, script) = harness();
    script.lock().unwrap().browser = true;
    let shapes = open(&mut h);
    assert!(has_text(&shapes, "Join with a code"));
    assert!(!has_text(&shapes, "View on another device"));
    assert!(!palette_keys(&h.app, "another device").iter().any(|k| k == KEY));
    let (mut h, script) = harness();
    script.lock().unwrap().browser = true;
    h.app.session.prefs.publish_server = "wss://figcraft.example/".into();
    let shapes = open(&mut h);
    assert!(has_text(&shapes, "Start sharing"), "it shares online");
    assert!(!has_text(&shapes, "View on another device"));
}
