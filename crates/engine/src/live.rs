//! Live sessions (milestone M9, step 2 of `docs/collab-protocol.md`): people edit the same file
//! together and see each other's cursors, selections and viewports, and can follow one another.
//!
//! The engine holds what everyone sees ([`Live`]: status, peers and their presence, who you
//! follow) and the `collab.*` commands. The networking lives in a higher layer
//! (`figcraft-collab-net`), which the app installs as a [`LiveLink`]: the engine never opens a
//! socket, so nothing below the UI layer depends on networking. Without a link (the CLI, tests)
//! the commands say live sessions aren't available.
//!
//! Peers, presence and document changes are the wire types of the collaboration core
//! ([`figcraft_collab::message`]). Each participant runs a core [`Client`]: the host's session
//! document is the one it shared, a guest gets the host's file as a new tab from the welcome.
//! Every command, gesture or group on the session document is synced as one batch and is one step
//! of the user's own undo history ([`Session::record`]); undo and redo there take back only the
//! user's own steps ([`figcraft_collab::Client::undo`]). Batches from the host's server are applied
//! through the client and installed with [`Session::apply_remote`], which never touches the undo
//! history. After a dropped connection the client resumes with its id and catches up; what it
//! made meanwhile goes out then.
//!
//! A file can also live on a `figcraft-server`: `collab.publish` uploads the open document and
//! syncs that tab through the server, whose share links let others edit or only view. A view link
//! makes the synced document read-only here (every edit is refused before it is sent; the server
//! refuses it too).
//!
//! Sessions are per document ([`Lives`]): each open document has its own session or none, several
//! run at once (host one file, join another, publish a third), and they keep syncing while their
//! tab isn't in front. The commands act on the active document's session ([`Session::live`]);
//! closing a document ends its session only. One link carries them all, each under its
//! [`SessionId`].
//!
//! Sharing with a join code (`collab.share`): online, the open document is published to the
//! server in Preferences and the server names its links with short codes; on the local network
//! the app hosts and the code carries the host's address and the session's secret
//! ([`figcraft_collab::code`]). `collab.join` takes a code of either kind, or an invite link.
//!
//! Cursor chat (`collab.cursorChat`) and spotlights (`collab.spotlight`) travel as signals:
//! short-lived presence the host passes on to everyone, never stored. A spotlight makes everyone
//! follow the person who started it; moving your own view stops following, as with any follow.

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::Arc;

use figcraft_collab::Client;
use figcraft_collab::code::{self, Code};
pub use figcraft_collab::message::Signal;
use figcraft_collab::message::{Admin, AdminReply, ClientMsg, EditingText, LibraryReply, LibraryReq, PluginReply, PluginReq, ServerMsg};

pub use figcraft_collab::message::{Access, HostInfo, Resume, utc_date};
use figcraft_doc::{Document, NodeId};
use serde::Serialize;
use serde_json::{Value, json};

pub use figcraft_collab::message::{Peer, Presence};

use crate::cmd::CommandSpec;
use crate::collab::EngineRecompute;
use crate::params::{get, num, req_string, string};
use crate::tools::ToolId;
use crate::{DocState, EngineError, Result, Session, TextEdit, ViewState, bad, spec};

/// The server files are shared through online unless Preferences name another: the only place
/// its address is written.
///
/// A build can ship another one, or none: set `FIGCRAFT_DEFAULT_SERVER` when compiling to a
/// server's address (`wss://host/`), or to nothing (`FIGCRAFT_DEFAULT_SERVER=`) for a build whose
/// online sharing stays off until the user enters a server in Preferences.
pub const DEFAULT_SERVER: &str = match option_env!("FIGCRAFT_DEFAULT_SERVER") {
    Some(server) => server,
    None => "wss://collab.craftapps.dev/",
};

/// Most selected ids one presence update carries (the rest aren't shown to others).
pub const MAX_SELECTION: usize = 10_000;
/// Longest name shown to others (characters).
pub const MAX_NAME: usize = 64;

/// The colours people get in a session, in the order the host hands them out (`#RRGGBB`, white
/// text reads on each).
pub const PEOPLE: [&str; 8] = ["#0D99FF", "#9747FF", "#14AE5C", "#F24822", "#D9730D", "#E83E8C", "#00A3A3", "#665CE0"];

/// `#RRGGBB` → (r, g, b); `None` for anything else.
pub fn parse_color(s: &str) -> Option<(u8, u8, u8)> {
    let hex = s.strip_prefix('#').filter(|h| h.len() == 6 && h.bytes().all(|b| b.is_ascii_hexdigit()))?;
    let byte = |i: usize| hex.get(i..i + 2).and_then(|b| u8::from_str_radix(b, 16).ok());
    Some((byte(0)?, byte(2)?, byte(4)?))
}

/// Why a join failed when a message from the host is over this app's size limit (`{mb}`: the
/// limit in MB). The UI translates it.
pub const TOO_LARGE_TO_JOIN: &str = "This file is too large to join here (up to {mb} MB)";

/// The connection to a live session.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum Status {
    /// Not in a session.
    #[default]
    Off,
    /// Joining (connecting to the host, waiting for its welcome).
    Connecting,
    /// In the session.
    Live,
    /// The connection dropped; trying again (`attempt` from 1).
    Reconnecting { attempt: u32 },
    /// Couldn't join, or the connection was lost for good.
    Failed { reason: String },
}

impl Status {
    pub fn id(&self) -> &'static str {
        match self {
            Status::Off => "off",
            Status::Connecting => "connecting",
            Status::Live => "live",
            Status::Reconnecting { .. } => "reconnecting",
            Status::Failed { .. } => "failed",
        }
    }

    /// In a session or on the way into one.
    pub fn active(&self) -> bool {
        matches!(self, Status::Connecting | Status::Live | Status::Reconnecting { .. })
    }

    /// What the Share popover says about it.
    pub fn label(&self) -> String {
        match self {
            Status::Off => "Not in a live session".into(),
            Status::Connecting => "Joining…".into(),
            Status::Live => "Live".into(),
            Status::Reconnecting { attempt } => format!("Connection lost, reconnecting (attempt {attempt})…"),
            Status::Failed { reason } => reason.clone(),
        }
    }
}

/// Your part in the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    Host,
    Guest,
}

/// A page of the shared file.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PageInfo {
    pub id: u64,
    pub name: String,
}

/// The session you're in.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub role: Role,
    /// Your client id, name and colour as the others see them.
    pub me: u32,
    pub name: String,
    pub color: String,
    /// The shared file's name and pages (the host's).
    pub file_name: String,
    pub pages: Vec<PageInfo>,
    /// The invite link (`figcraft://join?host=…&file=…&token=…`) and the same as a plain
    /// WebSocket URL. Both carry the session's secret token: share them only with people you
    /// invite.
    pub link: String,
    pub url: String,
    /// `ip:port` the session listens on (host) or connects to (guest); a server's URL when it
    /// isn't plain `ws://` at `/`.
    pub address: String,
    /// Reachable from other machines (the host listens on a network address, not loopback).
    pub network: bool,
    /// What you may do: edit, or only view (a server's view link).
    pub access: Access,
}

/// A file you published to a server: where, and its first share links (they carry their tokens:
/// share them like passwords; the server keeps only hashes).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Published {
    /// The server's address.
    pub server: String,
    /// The file's id and name on the server.
    pub file: String,
    pub name: String,
    /// The link that lets people edit, and the one that lets them only view.
    pub edit: String,
    pub view: String,
    /// Published without the server's key: the owner token that revokes links, renames or removes
    /// the file (shown this once; the server keeps only its hash).
    pub owner: Option<String>,
    /// When the server deletes the file if nobody opens it (seconds since 1970).
    pub expires: Option<u64>,
}

/// How a file is shared with a join code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Via {
    /// Through a server: the file is kept there and the server hands out the codes.
    Online,
    /// From this app, to people on the same network.
    Local,
}

/// One way into a shared file: an invite link, and the join code that stands for it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SharedLink {
    /// The link's id on its server (what revokes it; empty for a session this app hosts).
    pub id: String,
    /// The invite link (it carries its secret).
    pub link: String,
    /// Its join code's symbols. `None`: there is none (a server from before join codes, a host
    /// without an IPv4 address): the link is shared instead.
    pub code: Option<String>,
}

/// What others join a shared file with: to edit it and, on a server, to only view it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Shared {
    pub edit: SharedLink,
    pub view: Option<SharedLink>,
}

/// A file shared with a join code (`collab.share`). Online, the file is its owner's on the
/// server: sharing gives others links (and codes) of their own, and stopping takes those away
/// while the owner stays in the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Share {
    pub via: Via,
    /// Online: the server's address as it was given (it may carry the server's key: never shown).
    pub server: String,
    /// What the code shown lets people do.
    pub access: Access,
    /// What others join with now. `None`: on its way, or (online) not shared at the moment: the
    /// file is on the server, open only to its owner.
    pub links: Option<Shared>,
}

impl Share {
    /// The way in to show: the one for [`access`](Self::access).
    fn shown(&self) -> Option<&SharedLink> {
        let links = self.links.as_ref()?;
        if self.access == Access::View { links.view.as_ref() } else { Some(&links.edit) }
    }

    /// The code to show, grouped (`K7M-2QX-9TD`).
    pub fn code(&self) -> Option<String> {
        self.shown()?.code.as_deref().map(code::grouped)
    }

    /// The invite link to give with (or in place of) the code.
    pub fn link(&self) -> Option<&str> {
        self.shown().map(|l| l.link.as_str())
    }
}

/// What someone typed to join, as what the link joins: an invite link as it is, a local network
/// code as the link it stands for, an online code with `server` to ask about it.
pub fn join_target(server: &str, typed: &str) -> std::result::Result<String, String> {
    let typed = typed.trim();
    if typed.contains("://") || typed.contains('?') {
        return Ok(typed.to_string());
    }
    match code::parse(typed)? {
        Code::Local(local) => Ok(local.link()),
        Code::Online(_) if server.trim().is_empty() => Err(format!("that is an online join code, and {ONLINE_OFF}")),
        Code::Online(code) => Ok(format!("figcraft://join?server={}&code={code}", server.trim())),
    }
}

/// Why nothing can be shared or joined online.
pub const ONLINE_OFF: &str = "online sharing is off: choose a server in Preferences (or share on the local network)";

/// A session's number on the link: the engine picks it (from 1) when it starts the session.
pub type SessionId = u64;
/// The [`SessionId`] of events that aren't about a session (answers to background requests).
pub const NO_SESSION: SessionId = 0;

/// What happened in a session since the last [`LiveLink::poll`].
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Status(Status),
    /// Joined, or joined again after a reconnect: who you are and who else is here.
    Welcome {
        info: SessionInfo,
        peers: Vec<Peer>,
    },
    PeerJoined(Peer),
    PeerLeft(u32),
    Presence(u32, Presence),
    /// Someone's cursor chat or spotlight, or (about you) the host's [`Signal::READY`].
    Signal(u32, Signal),
    /// For the document sync client: the welcome (with the whole document), or a batch of
    /// changes the server ordered, or the rejection of one of yours.
    Sync(ServerMsg),
    /// The host ended the session.
    Ended {
        reason: String,
    },
    /// The document is on the server (`collab.publish`); joining it follows.
    Published(Published),
    /// The published file is shared (`collab.share`): the links and codes others join with, or
    /// why the server made none.
    Shared(std::result::Result<Shared, String>),
    /// What the server says about its hosting (operator, terms, this file's expiry).
    Hosting(HostInfo),
    /// The server's answer to `collab.report`: kept, or why not.
    Reported(Option<String>),
    /// The answer to a team library request sent in the background
    /// ([`LiveLink::library_background`]) about the file behind share link `link`.
    Library {
        link: String,
        reply: std::result::Result<LibraryReply, String>,
    },
    /// The answer to request `job` sent in the background ([`LiveLink::background`]).
    Net {
        job: u64,
        reply: Box<NetReply>,
    },
}

/// A request to a server outside a live session: its plugin directory, a team library, or keeping
/// a file there.
#[derive(Clone, Debug)]
pub enum NetRequest {
    /// To the plugin directory of the server at `server` (`ws://host:port/`, with `?key=…` to
    /// publish).
    Plugins { server: String, req: PluginReq },
    /// About the team library of the file behind share link `link`.
    Library { link: String, req: LibraryReq },
    /// Keep a file on a server without joining it ([`LiveLink::upload`]).
    Upload(PublishOptions),
    /// Change who else gets into `file` on the server at `server` (its address, with `?key=…`, or
    /// the file's `owner` token): revoke the links with these ids, then, to `create`, make a new
    /// edit link and view link with their join codes ([`LiveLink::share`]).
    Share { server: String, file: String, owner: String, revoke: Vec<String>, create: bool },
    /// Ask the server at `server` about its hosting and, with a file's owner token, manage that
    /// file ([`LiveLink::hosting`]).
    Hosting { server: String, file: String, owner: String, req: Option<Admin> },
    /// Download the file at `url` (`https://` only), at most `max` bytes ([`LiveLink::fetch`]).
    Fetch { url: String, max: usize },
}

impl NetRequest {
    /// What a session without the network says.
    pub(crate) fn unavailable(&self) -> &'static str {
        match self {
            NetRequest::Plugins { .. } => crate::plugin_directory::NO_DIRECTORY,
            NetRequest::Library { .. } | NetRequest::Upload(_) => NO_TEAM_LIBRARIES,
            NetRequest::Hosting { .. } | NetRequest::Share { .. } => NO_HOSTING,
            NetRequest::Fetch { .. } => crate::fonts::NO_DOWNLOADS,
        }
    }

    /// How to answer that this request failed (and why).
    pub fn failure(&self) -> fn(String) -> NetReply {
        match self {
            NetRequest::Plugins { .. } => |why| NetReply::Plugins(Err(why)),
            NetRequest::Library { .. } => |why| NetReply::Library(Err(why)),
            NetRequest::Upload(_) => |why| NetReply::Upload(Err(why)),
            NetRequest::Hosting { .. } => |why| NetReply::Hosting(Err(why)),
            NetRequest::Share { .. } => |why| NetReply::Share(Err(why)),
            NetRequest::Fetch { .. } => |why| NetReply::Fetch(Err(why)),
        }
    }

    /// The answer to this request stands for `other` when a command runs again: the same kind of
    /// request to the same place (what it carries may differ: a usage report, a fresh timestamp).
    pub(crate) fn stands_for(&self, other: &NetRequest) -> bool {
        use std::mem::discriminant;
        match (self, other) {
            (NetRequest::Plugins { server: a, req: x }, NetRequest::Plugins { server: b, req: y }) => a == b && discriminant(x) == discriminant(y),
            (NetRequest::Library { link: a, req: x }, NetRequest::Library { link: b, req: y }) => a == b && discriminant(x) == discriminant(y),
            (NetRequest::Upload(a), NetRequest::Upload(b)) => a.server == b.server,
            (NetRequest::Hosting { server: a, file: f, req: x, .. }, NetRequest::Hosting { server: b, file: g, req: y, .. }) => {
                a == b && f == g && x.as_ref().map(discriminant) == y.as_ref().map(discriminant)
            }
            (NetRequest::Share { server: a, file: f, create: x, .. }, NetRequest::Share { server: b, file: g, create: y, .. }) => {
                a == b && f == g && x == y
            }
            (NetRequest::Fetch { url: a, .. }, NetRequest::Fetch { url: b, .. }) => a == b,
            _ => false,
        }
    }
}

/// The answer to a [`NetRequest`] (an error: the server couldn't be reached or refused).
#[derive(Clone, Debug, PartialEq)]
pub enum NetReply {
    Plugins(std::result::Result<PluginReply, String>),
    Library(std::result::Result<LibraryReply, String>),
    /// The new links, when some were asked for.
    Share(std::result::Result<Option<Shared>, String>),
    Upload(std::result::Result<Published, String>),
    /// What the server says about its hosting, and its answer to the request, if one was sent.
    Hosting(std::result::Result<(HostInfo, Option<AdminReply>), String>),
    /// The file's bytes (shared: a command run again in the background takes its earlier answers
    /// again), or `None` when the server has no such file.
    Fetch(std::result::Result<Option<Arc<Vec<u8>>>, String>),
}

/// What a session without the network says about servers' hosting.
pub const NO_HOSTING: &str = "publishing to a server needs the desktop app or the MCP server";

/// Which network interface a hosted session listens on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bind {
    /// This machine only (127.0.0.1): the default.
    Local,
    /// The machine's address on the local network (only when the user chooses to share there).
    Lan,
    /// A given interface address (e.g. a VPN's).
    Addr(IpAddr),
}

/// What [`LiveLink::host`] needs to start a session for the open file.
#[derive(Clone, Debug)]
pub struct HostOptions {
    /// `None`: the default port, or any free one when it's taken.
    pub port: Option<u16>,
    pub bind: Bind,
    /// Your name (empty: the host picks one).
    pub name: String,
    /// The shared file (the session's authoritative copy starts from it).
    pub doc: Arc<Document>,
    /// Shared with a join code (`collab.share`): the session's secret is short enough to type.
    pub short: bool,
}

/// What [`LiveLink::publish`] needs to put the open file on a server.
#[derive(Clone, Debug)]
pub struct PublishOptions {
    /// The server's address (`ws://host:port/`, `wss://host/path/`), which may carry its key
    /// (`?key=…`).
    pub server: String,
    /// The server's key, when the address doesn't carry it. Without one the file is published
    /// as anyone may, on a server open to the public (with an owner token).
    pub key: Option<String>,
    /// The file's name on the server.
    pub name: String,
    /// Your name in the session (empty: the server picks one).
    pub me: String,
    pub doc: Arc<Document>,
    /// Share the file once it is there: links and join codes for others ([`Event::Shared`];
    /// `collab.share`).
    pub share: bool,
}

/// The networking behind live sessions, installed by the app ([`Lives::install`]). It carries
/// any number of sessions at once, each under the [`SessionId`] it was started with. Calls return
/// at once; what happens later arrives through [`poll`](Self::poll).
pub trait LiveLink: Send {
    /// Sessions can be hosted here (the browser build can only join).
    fn can_host(&self) -> bool {
        true
    }
    /// Start hosting session `session` for `opts.doc`.
    fn host(&mut self, session: SessionId, opts: HostOptions) -> std::result::Result<SessionInfo, String>;
    /// Start joining the session behind an invite link, as session `session`. Errors are for
    /// links that can't be used; they never echo the link (it holds the token).
    fn join(&mut self, session: SessionId, link: &str, name: &str) -> std::result::Result<(), String>;
    /// [`join`](Self::join) as the client `resume` names, when the server still knows it (an app
    /// that starts over after a crash keeps its id, so what it made before is its own to send).
    /// A link that can't resume joins as someone new.
    fn join_resuming(&mut self, session: SessionId, link: &str, name: &str, _resume: Option<Resume>) -> std::result::Result<(), String> {
        self.join(session, link, name)
    }
    /// Start uploading `opts.doc` to a server as a new file, then join it with its edit link, as
    /// session `session`: [`Event::Published`] with its links, then the welcome. Errors are for
    /// addresses that can't be used; they never echo the key.
    fn publish(&mut self, _session: SessionId, _opts: PublishOptions) -> std::result::Result<(), String> {
        Err("publishing to a server needs the desktop app or the MCP server".into())
    }
    /// Keep `opts.doc` on a server as a new file without joining it: where, and its links
    /// (`library.publishToServer` with the server's key). Waits for the server. Errors never echo
    /// the key.
    fn upload(&mut self, _opts: PublishOptions) -> std::result::Result<Published, String> {
        Err(NO_TEAM_LIBRARIES.into())
    }
    /// One request about the team library of the file behind share link `link`; waits for the
    /// server's answer (a few seconds at most when it can't be reached). Errors never echo the
    /// link.
    fn library(&mut self, _link: &str, _req: LibraryReq) -> std::result::Result<LibraryReply, String> {
        Err(NO_TEAM_LIBRARIES.into())
    }
    /// Send library requests `(link, request)` in the background, in order; each answer arrives as
    /// [`Event::Library`]. Ignored while an earlier call's requests are still out.
    fn library_background(&mut self, _jobs: Vec<(String, LibraryReq)>) {}
    /// One request to the plugin directory of the server at `server` (`ws://host:port/`, with
    /// `?key=…` to publish); waits for the answer (a few seconds at most when it can't be
    /// reached). Errors never echo the key.
    fn plugins(&mut self, _server: &str, _req: PluginReq) -> std::result::Result<PluginReply, String> {
        Err(crate::plugin_directory::NO_DIRECTORY.into())
    }
    /// What the server at `server` says about its hosting, and with a file's owner token
    /// (`file` and `owner`) its answer to `req` about that file; waits for the server. Errors
    /// never echo the token.
    fn hosting(
        &mut self,
        _server: &str,
        _file: &str,
        _owner: &str,
        _req: Option<Admin>,
    ) -> std::result::Result<(HostInfo, Option<AdminReply>), String> {
        Err(NO_HOSTING.into())
    }
    /// See [`NetRequest::Share`]; waits for the server. Errors never echo the key or the token.
    fn share(&mut self, _server: &str, _file: &str, _owner: &str, _revoke: &[String], _create: bool) -> std::result::Result<Option<Shared>, String> {
        Err(NO_HOSTING.into())
    }
    /// Download the file at `url` (`https://` only), at most `max` bytes: `None` when the server
    /// has no such file. Waits for the server.
    fn fetch(&mut self, _url: &str, _max: usize) -> std::result::Result<Option<Vec<u8>>, String> {
        Err(crate::fonts::NO_DOWNLOADS.into())
    }
    /// Send `req` and wait for the answer (through the methods above).
    fn request(&mut self, req: NetRequest) -> NetReply {
        match req {
            NetRequest::Fetch { url, max } => NetReply::Fetch(self.fetch(&url, max).map(|b| b.map(Arc::new))),
            NetRequest::Share { server, file, owner, revoke, create } => NetReply::Share(self.share(&server, &file, &owner, &revoke, create)),
            NetRequest::Plugins { server, req } => NetReply::Plugins(self.plugins(&server, req)),
            NetRequest::Library { link, req } => NetReply::Library(self.library(&link, req)),
            NetRequest::Upload(o) => NetReply::Upload(self.upload(o)),
            NetRequest::Hosting { server, file, owner, req } => NetReply::Hosting(self.hosting(&server, &file, &owner, req)),
        }
    }
    /// Send `req` on a thread of its own and return at once: the answer arrives as [`Event::Net`]
    /// with `job` (a failure to send too). False when this link can't, and nothing was sent.
    fn background(&mut self, _job: u64, _req: NetRequest) -> bool {
        false
    }
    /// Remove client `client` from session `session`, which this link hosts: they are
    /// disconnected and can't join again from their address while it runs. Returns at once.
    fn remove(&mut self, _session: SessionId, _client: u32) -> std::result::Result<(), String> {
        Err("removing people needs the desktop app or the MCP server".into())
    }
    /// End session `session` (stop hosting or leave); the others go on. Nothing for a session
    /// that is over already.
    fn leave(&mut self, session: SessionId);
    /// Your latest presence in `session` (the link throttles and coalesces).
    fn presence(&mut self, session: SessionId, p: &Presence);
    /// A message for `session`'s server (document changes), sent in order.
    fn send(&mut self, session: SessionId, msg: ClientMsg);
    /// Events since the last call, each with its session ([`NO_SESSION`] for answers to
    /// background requests); never blocks.
    fn poll(&mut self) -> Vec<(SessionId, Event)>;
}

/// The view a follow last applied: any other view means the user moved.
#[derive(Clone, Copy, Debug, PartialEq)]
struct FollowedView {
    page: NodeId,
    cx: f64,
    cy: f64,
    zoom: f64,
}

/// One live session (see the module docs): a document's, or a join on its way to its own tab.
#[derive(Default)]
pub struct Live {
    /// Its number on the link (0: no session, the state of a document that isn't in one).
    pub id: SessionId,
    pub status: Status,
    pub info: Option<SessionInfo>,
    /// Everyone else in the session, by client id.
    pub peers: BTreeMap<u32, Peer>,
    /// Their latest presence (only while they're in the session).
    pub presence: BTreeMap<u32, Presence>,
    /// The peer whose viewport you follow.
    pub follow: Option<u32>,
    followed: Option<FollowedView>,
    /// Your pointer over the canvas (page coordinates): the UI sets it, or `collab.cursor`.
    pub cursor: Option<(f64, f64)>,
    /// The presence last handed to the link.
    sent: Option<Presence>,
    /// The document sync client (from the welcome).
    client: Option<Client>,
    /// The open document the session syncs (its `DocState::uid`).
    pub doc_uid: Option<u64>,
    /// The document whose tab shows the session: `doc_uid`, or for a join the document it was
    /// started from, until the session's file opens in a tab of its own (`None`: none was open).
    pub tab: Option<u64>,
    /// The session document as last sent or installed: a different one has local edits to send.
    synced: Option<Arc<Document>>,
    /// Server batches waiting for a gesture or preview on the session document to end.
    deferred: Vec<ServerMsg>,
    /// Nesting of [`Session::grouped`] on the session document: its edits sync once, at the end.
    hold: u32,
    /// The address of the server this session's file was published to, or is yours on (empty
    /// otherwise; it may carry the server's key: never shown).
    pub server: String,
    /// The file you published to a server in this session, or one of yours you opened again
    /// (`collab.openFile`: without its owner token, which stays in the record), with its links.
    pub published: Option<Published>,
    /// The session was started by sharing with a join code (`collab.share`): how, and its codes.
    pub share: Option<Share>,
    /// What the server of the file you're in says about its hosting (operator, terms, expiry);
    /// a server that says it takes reports (`collab.report`).
    pub hosting: Option<HostInfo>,
    /// The host passes cursor chat and spotlights on (it said so after the welcome).
    pub signals: bool,
    /// Others' open cursor chat messages.
    pub chats: BTreeMap<u32, String>,
    /// Your cursor chat message (empty: none).
    pub chat: String,
    /// Who spotlights (you follow them until you move your view).
    pub spotlight: Option<u32>,
    /// You spotlight: everyone follows you.
    pub spotlighting: bool,
}

impl Live {
    pub fn role(&self) -> Option<Role> {
        self.info.as_ref().map(|i| i.role)
    }

    /// Stop following. False when you weren't following anyone.
    fn unfollow(&mut self) -> bool {
        self.followed = None;
        self.follow.take().is_some()
    }

    /// Back to no session, still shown on its tab (a failure's reason stays there).
    fn reset(&mut self) {
        *self = Live { id: self.id, tab: self.tab, ..Live::default() };
    }

    /// The document sync client (once the welcome arrived): peers, pending batches, the user's
    /// undo history.
    pub fn client(&self) -> Option<&Client> {
        self.client.as_ref()
    }

    /// Document changes are syncing (the welcome arrived).
    pub fn syncing(&self) -> bool {
        self.client.is_some()
    }

    /// What the file's tab says about its session: `Some(true)` others get in (a session this app
    /// hosts, one you joined, a file of yours shared with a code or links), `Some(false)` a file of
    /// yours on a server that only you get into now, `None` no session (or one still on its way).
    pub fn shared(&self) -> Option<bool> {
        let there = matches!(self.status, Status::Live | Status::Reconnecting { .. });
        there.then(|| self.share.as_ref().is_none_or(|s| s.links.is_some()))
    }

    /// The name of the session's file, once known.
    fn file_name(&self) -> Option<&str> {
        self.info.as_ref().map(|i| i.file_name.as_str()).or(self.published.as_ref().map(|p| p.name.as_str())).filter(|n| !n.is_empty())
    }
}

/// The app's live sessions, one per document at most, and what they share: the networking and
/// the invite-link registration.
#[derive(Default)]
pub struct Lives {
    link: Option<Box<dyn LiveLink>>,
    sessions: BTreeMap<SessionId, Live>,
    /// The last session id handed out.
    last: SessionId,
    /// What a document that isn't in a session shows.
    off: Live,
    /// Registers the app as the opener of `figcraft://` invite links (the desktop app on Windows
    /// installs one; see [`crate::invite_links`]).
    pub links: Option<Box<dyn crate::invite_links::LinkRegistrar>>,
    /// The recorded online files changed since the app last saved the preferences (it saves them
    /// at once: a crash must not lose an owner token).
    pub unsaved: bool,
    /// Servers that couldn't say what became of your files at the last `collab.files {refresh}`.
    pub servers: crate::online::Problems,
    /// Recovery copies of your online files waiting for their file to arrive from its server
    /// (`collab.recoverFile`).
    pub(crate) recovering: Vec<crate::online::Recovering>,
}

impl Lives {
    /// Install the networking (the app does this once at startup).
    pub fn install(&mut self, link: Box<dyn LiveLink>) {
        self.link = Some(link);
    }

    /// The installed networking, if any.
    pub(crate) fn link(&mut self) -> Option<&mut (dyn LiveLink + 'static)> {
        self.link.as_deref_mut()
    }

    /// A link is installed: sessions can be joined (and hosted, when [`can_host`](Self::can_host)).
    pub fn available(&self) -> bool {
        self.link.is_some()
    }

    /// Sessions can be hosted from here (a link is installed and it isn't the browser's).
    pub fn can_host(&self) -> bool {
        self.link.as_ref().is_some_and(|l| l.can_host())
    }

    /// Session `id`, while it runs or shows why it failed.
    pub fn get(&self, id: SessionId) -> Option<&Live> {
        self.sessions.get(&id)
    }

    /// A recovery copy started with session `id` still waits for its outcome (the app keeps the
    /// copy's file until it doesn't).
    pub fn recovering(&self, id: SessionId) -> bool {
        self.recovering.iter().any(|r| r.session == id)
    }

    /// Every session, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &Live> {
        self.sessions.values()
    }

    pub(crate) fn sessions_mut(&mut self) -> impl Iterator<Item = &mut Live> {
        self.sessions.values_mut()
    }

    /// The session the tab of document `tab` shows (`None`: no document is open): the one that
    /// syncs it, else the latest join started from it.
    fn shown(&self, tab: Option<u64>) -> Option<SessionId> {
        self.sessions.values().filter(|l| l.tab == tab).max_by_key(|l| (l.doc_uid.is_some(), l.id)).map(|l| l.id)
    }

    /// What the tab of document `tab` shows: its session, or no session.
    fn on(&self, tab: Option<u64>) -> &Live {
        self.shown(tab).and_then(|id| self.sessions.get(&id)).unwrap_or(&self.off)
    }

    /// The session of document `uid`: the one that syncs it, or will once it is published.
    fn doc(&self, uid: u64) -> Option<&Live> {
        self.sessions.values().find(|l| l.doc_uid == Some(uid))
    }

    fn doc_mut(&mut self, uid: u64) -> Option<&mut Live> {
        self.sessions.values_mut().find(|l| l.doc_uid == Some(uid))
    }

    /// Start a session on the link for command `cmd`: `start` gets the link and the new
    /// session's id, which comes back with what it returned.
    fn start<T>(&mut self, cmd: &str, start: impl FnOnce(&mut dyn LiveLink, SessionId) -> Result<T>) -> Result<(SessionId, T)> {
        let link = self.link.as_deref_mut().ok_or_else(|| unavailable(cmd))?;
        let id = self.last.saturating_add(1);
        let out = start(link, id)?;
        self.last = id;
        Ok((id, out))
    }

    /// Session `live` started on the link: it takes its tab over from what was there (a session
    /// that failed; for a document's own session, the one it was in). Joins on their way from
    /// that tab go on.
    fn open(&mut self, live: Live) {
        let own = live.doc_uid.is_some();
        let over: Vec<SessionId> =
            self.sessions.values().filter(|l| l.tab == live.tab && (!l.status.active() || (own && l.doc_uid.is_some()))).map(|l| l.id).collect();
        for id in over {
            self.end(id);
        }
        self.sessions.insert(live.id, live);
    }

    /// End session `id` (stop hosting or leave) and forget it.
    pub(crate) fn end(&mut self, id: SessionId) -> Option<Live> {
        self.leave(id);
        self.sessions.remove(&id)
    }

    /// Tell the link session `id` is over.
    fn leave(&mut self, id: SessionId) {
        if let Some(l) = self.link.as_mut() {
            l.leave(id);
        }
    }

    /// Messages for session `id`'s server, in order.
    fn send(&mut self, id: SessionId, msgs: impl IntoIterator<Item = ClientMsg>) {
        if let Some(l) = self.link.as_mut() {
            for m in msgs {
                l.send(id, m);
            }
        }
    }
}

impl Drop for Lives {
    fn drop(&mut self) {
        for id in std::mem::take(&mut self.sessions).into_keys() {
            self.leave(id);
        }
    }
}

/// The page a presence is on (its viewport's, else its cursor's).
pub fn presence_page(p: &Presence) -> Option<NodeId> {
    p.viewport.map(|v| v.0).or(p.cursor.map(|c| c.0))
}

/// Presence ready to send: non-finite numbers and empty viewports dropped, the selection capped.
pub fn sanitize(mut p: Presence) -> Presence {
    p.cursor = p.cursor.filter(|c| c.1.is_finite() && c.2.is_finite());
    p.viewport = p.viewport.filter(|v| v.1.is_finite() && v.2.is_finite() && v.3.is_finite() && v.4.is_finite() && v.3 > 0.0 && v.4 > 0.0);
    if let Some(s) = p.selection.as_mut() {
        s.truncate(MAX_SELECTION);
    }
    p
}

/// The name others see: the name from Preferences, or empty for the default ("You"), which the
/// host replaces.
pub fn display_name(prefs: &crate::Prefs) -> String {
    let n = prefs.user_name.trim();
    if n == crate::cmd::comment::DEFAULT_AUTHOR { String::new() } else { n.chars().filter(|c| !c.is_control()).take(MAX_NAME).collect() }
}

/// Initials for an avatar ("Ada Lovelace" → "AL"; "?" for none).
pub fn initials(name: &str) -> String {
    let mut out: String = name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect();
    if out.is_empty() {
        out.push('?');
    }
    out.to_uppercase()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

/// What others are told about your text editing: the layer, caret and selected characters.
fn editing_of(t: &TextEdit) -> EditingText {
    let (a, b) = (t.anchor.min(t.caret), t.anchor.max(t.caret));
    EditingText { node: t.id, caret: t.caret, selection: (a != b).then_some((a, b)) }
}

/// The note shown while others type in the text layer you look at: "Hana is editing this text",
/// "Hana and Ada are…", "Hana and 2 others are…" (`None`: nobody).
pub fn editing_hint(names: &[&str]) -> Option<String> {
    let verb = "editing this text";
    Some(match names {
        [] => return None,
        [one] => format!("{one} is {verb}"),
        [a, b] => format!("{a} and {b} are {verb}"),
        [first, rest @ ..] => format!("{first} and {} others are {verb}", rest.len()),
    })
}

/// Why team libraries aren't available (no network link, or the browser build).
pub const NO_TEAM_LIBRARIES: &str = "team libraries on a server need the desktop app or the MCP server";

/// Why an edit of a server file you joined with a view link is refused.
pub const VIEW_ONLY: &str = "View only: the link you joined with lets you see this file, not change it";
/// Why an edit of a file on its way to a server is refused.
pub const PUBLISHING: &str = "The file is being published: you can edit it again once it is on the server";

/// A gesture, version preview or live text preview shows a document that isn't the edit to
/// sync yet.
fn busy(ds: &DocState) -> bool {
    ds.interaction.is_some() || ds.preview.is_some() || ds.text_preview.is_some()
}

impl Session {
    /// The active document (`None`: no document is open).
    fn tab(&self) -> Option<u64> {
        self.ds().ok().map(|d| d.uid)
    }

    /// The active document's live session (status off when it isn't in one). Everything the
    /// window shows and the `collab.*` commands act on is this one.
    pub fn live(&self) -> &Live {
        self.live.on(self.tab())
    }

    /// The active document's live session, when it has one (running, on its way or failed).
    pub fn live_mut(&mut self) -> Option<&mut Live> {
        let id = self.live.shown(self.tab())?;
        self.live.sessions.get_mut(&id)
    }

    /// Document `uid`'s live session (status off when it isn't in one).
    pub fn live_of(&self, uid: u64) -> &Live {
        self.live.on(Some(uid))
    }

    /// The active document is its session's synced document: page and layer ids in others'
    /// presence refer to it.
    pub fn live_shared(&self) -> bool {
        self.tab().is_some_and(|uid| self.live_doc(uid))
    }

    /// Install a document that came from the session (others' edits): the selection, page and view
    /// stay where they can, and the undo history isn't touched.
    pub fn apply_remote(&mut self, uid: u64, doc: Arc<Document>) {
        let Some(ds) = self.docs.iter_mut().find(|d| d.uid == uid) else { return };
        ds.doc = doc;
        if !ds.doc.pages.contains(&ds.page) {
            ds.page = ds.doc.first_page().unwrap_or(ds.page);
        }
        if ds.text_edit.as_ref().is_some_and(|t| !ds.doc.contains(t.id)) {
            ds.text_edit = None;
        }
        if ds.vector_edit.as_ref().is_some_and(|v| !ds.doc.contains(v.id)) {
            ds.vector_edit = None;
        }
        ds.clean_selection();
        ds.revision += 1;
    }

    /// Your presence in the active document's session now: its page, selection, viewport and
    /// edited text, and the pointer.
    pub fn live_presence(&self) -> Presence {
        self.presence_in(self.live())
    }

    /// Your presence in `live` (nothing while another document than its own is in front).
    fn presence_in(&self, live: &Live) -> Presence {
        let Some(ds) = self.ds().ok().filter(|ds| live.tab == Some(ds.uid)) else { return Presence::default() };
        let (x0, y0) = ds.view.origin();
        let z = ds.view.zoom.max(1e-9);
        sanitize(Presence {
            cursor: live.cursor.map(|(x, y)| (ds.page, x, y)),
            selection: Some(ds.selection.clone()),
            viewport: Some((ds.page, x0, y0, ds.view.vw / z, ds.view.vh / z)),
            editing_text: ds.text_edit.as_ref().map(|t| Box::new(editing_of(t))),
        })
    }

    /// The others typing in text layer `id` of the active document, in its session.
    pub fn live_text_editors(&self, id: NodeId) -> Vec<&Peer> {
        if !self.live_shared() {
            return Vec::new();
        }
        let live = self.live();
        live.presence.iter().filter(|(_, p)| p.editing_text.as_ref().is_some_and(|e| e.node == id)).filter_map(|(c, _)| live.peers.get(c)).collect()
    }

    /// You started typing in text layer `id`: say so when someone else is typing in it too (both
    /// edits sync when they finish; the one finished last wins).
    pub(crate) fn live_text_notice(&mut self, id: NodeId) {
        let names: Vec<&str> = self.live_text_editors(id).iter().map(|p| p.name.as_str()).collect();
        if let Some(hint) = editing_hint(&names) {
            self.message = Some(format!("{hint} too: the version finished last is kept"));
        }
    }

    /// Take in what happened in every session (the UI calls it every frame; the network wakes the
    /// UI when something arrives). Returns whether anything changed.
    pub fn live_poll(&mut self) -> bool {
        let Some(link) = self.live.link.as_mut() else { return false };
        let events = link.poll();
        if events.is_empty() {
            return false;
        }
        for (id, e) in events {
            self.live_event(id, e);
        }
        self.live_closed();
        for id in self.live.sessions.keys().copied().collect::<Vec<_>>() {
            self.live_deferred(id);
            self.live_follow_apply(id);
        }
        self.online_recoveries();
        true
    }

    fn live_event(&mut self, id: SessionId, e: Event) {
        let e = match e {
            Event::Library { link, reply } => return self.library_answer(&link, reply),
            Event::Net { job, reply } => return self.background_answer(job, *reply),
            e => e,
        };
        // A session left meanwhile: what was still on its way is dropped.
        let front = self.tab();
        let Some(live) = self.live.sessions.get_mut(&id) else { return };
        // What happens in a file that isn't in front says which file (by its tab's name).
        let open = self.docs.iter().find(|d| Some(d.uid) == live.doc_uid).map(|d| d.doc.name.as_str());
        let behind = open.or(live.file_name()).filter(|_| live.tab != front).map(|n| format!(" {n}")).unwrap_or_default();
        match e {
            Event::Status(Status::Failed { reason }) => self.live_fail(id, reason),
            Event::Status(Status::Off) => {
                self.live.end(id);
            }
            Event::Status(st) => {
                // Who's here comes again with the next welcome; edits wait for it.
                if let Status::Reconnecting { .. } = st {
                    live.peers.clear();
                    live.presence.clear();
                    live.sent = None;
                    // Until the host says it reads them again (a spotlight of yours goes out
                    // again then).
                    live.signals = false;
                    live.chats.clear();
                    live.chat.clear();
                    live.spotlight = None;
                    if let Some(c) = live.client.as_mut() {
                        c.disconnect();
                    }
                }
                live.status = st;
            }
            Event::Welcome { mut info, peers } => {
                // The shared file's name and pages come with the document (the sync welcome).
                if let Some(old) = live.info.as_ref().filter(|_| info.file_name.is_empty()) {
                    info.file_name = old.file_name.clone();
                    info.pages = old.pages.clone();
                }
                live.status = Status::Live;
                live.info = Some(info);
                live.peers = peers.into_iter().map(|p| (p.client_id, p)).collect();
                live.presence.retain(|id, _| live.peers.contains_key(id));
                if live.follow.is_some_and(|f| !live.peers.contains_key(&f)) {
                    live.unfollow();
                }
                // Others need your presence again.
                live.sent = None;
            }
            Event::PeerJoined(p) => {
                self.message = Some(format!("{} joined{behind}", p.name));
                live.peers.insert(p.client_id, p);
            }
            Event::PeerLeft(peer) => {
                live.presence.remove(&peer);
                live.chats.remove(&peer);
                if live.spotlight == Some(peer) {
                    live.spotlight = None;
                }
                if let Some(p) = live.peers.remove(&peer) {
                    self.message = Some(format!("{} left{behind}", p.name));
                }
                if live.follow == Some(peer) {
                    live.unfollow();
                }
            }
            Event::Presence(peer, p) => {
                if live.peers.contains_key(&peer) {
                    live.presence.insert(peer, p);
                }
            }
            Event::Signal(peer, sig) => self.live_signal_in(id, peer, sig),
            Event::Sync(welcome @ ServerMsg::Welcome { .. }) if live.client.is_none() => self.live_welcome(id, welcome),
            // Batches, and the welcome back after a dropped connection: the client catches up in
            // order.
            Event::Sync(msg) => live.deferred.push(msg),
            Event::Ended { reason } => {
                self.message = Some(if behind.is_empty() { reason } else { format!("{}: {reason}", behind.trim_start()) });
                self.live.end(id);
            }
            Event::Published(p) => {
                // Shared with a code: the popover shows it, nothing more to say.
                if live.share.is_none() {
                    self.message = Some(format!("Published {} to the server", p.name));
                }
                let server = live.server.clone();
                live.published = Some(p.clone());
                self.online_published(&server, &p);
            }
            Event::Shared(Ok(links)) => {
                let links = clean_shared(links);
                match live.server_file() {
                    Some((server, file)) => self.online_shared(&server, &file, Some(links)),
                    None => {
                        if let Some(share) = live.share.as_mut() {
                            share.links = Some(links);
                        }
                    }
                }
            }
            Event::Shared(Err(why)) => self.message = Some(format!("The file is on the server, but couldn't be shared: {why}")),
            Event::Hosting(info) => live.hosting = Some(clean_hosting(info)),
            Event::Reported(error) => {
                self.message =
                    Some(error.map_or_else(|| "Your report was sent to the server's operator".into(), |e| format!("The report wasn't sent: {e}")));
            }
            // Answered above.
            Event::Library { .. } | Event::Net { .. } => {}
        }
    }

    /// Someone's cursor chat or spotlight in session `id` (checked again: the host is untrusted
    /// too), or the host saying it reads them.
    fn live_signal_in(&mut self, id: SessionId, peer: u32, sig: Signal) {
        let Some(sig) = sig.clean() else { return };
        let Some(live) = self.live.sessions.get_mut(&id) else { return };
        let name = live.peers.get(&peer).map(|p| p.name.clone());
        match (sig.kind.as_str(), name) {
            (Signal::READY, _) if live.info.as_ref().is_some_and(|i| i.me == peer) => {
                live.signals = true;
                if live.spotlighting {
                    self.live.send(id, [ClientMsg::Signal(Signal::spotlight(true))]);
                }
            }
            (Signal::CHAT, Some(_)) if sig.text.is_empty() => {
                live.chats.remove(&peer);
            }
            (Signal::CHAT, Some(_)) => {
                live.chats.insert(peer, sig.text);
            }
            (Signal::SPOTLIGHT, Some(name)) if sig.on() => {
                // Theirs takes over from yours or anyone else's.
                live.spotlight = Some(peer);
                live.spotlighting = false;
                live.follow = Some(peer);
                live.followed = None;
                self.message = Some(format!("{name} is spotlighting: you follow their view (move it to stop)"));
            }
            (Signal::SPOTLIGHT, Some(name)) if live.spotlight == Some(peer) => {
                live.spotlight = None;
                if live.follow == Some(peer) {
                    live.unfollow();
                }
                self.message = Some(format!("{name} stopped spotlighting"));
            }
            _ => {}
        }
    }

    /// Why cursor chat or a spotlight can't go out now, if it can't.
    fn signal_blocker(&self) -> Option<&'static str> {
        let live = self.live();
        if live.status != Status::Live {
            Some("cursor chat and spotlight need a live session")
        } else if !live.signals {
            Some("the session's host runs an older version without cursor chat and spotlight")
        } else {
            None
        }
    }

    /// Show `text` next to your pointer for everyone (empty closes it); returns it as sent
    /// (control characters out, at most [`Signal::MAX_CHAT`] characters).
    pub fn live_chat(&mut self, text: &str) -> Result<String> {
        let sig = Signal::chat(text).clean().unwrap_or_default();
        if sig.text.is_empty() && self.live().chat.is_empty() {
            return Ok(String::new());
        }
        if let Some(why) = self.signal_blocker() {
            return Err(EngineError::Disabled("collab.cursorChat".into(), why.into()));
        }
        let Some(live) = self.live_mut() else { return Ok(String::new()) };
        if sig.text != live.chat {
            live.chat.clone_from(&sig.text);
            let id = live.id;
            self.live.send(id, [ClientMsg::Signal(sig.clone())]);
        }
        Ok(sig.text)
    }

    /// Start (`on`) or stop asking everyone to follow your view.
    pub fn live_spotlight(&mut self, on: bool) -> Result<()> {
        if on == self.live().spotlighting {
            return Ok(());
        }
        if on && let Some(why) = self.signal_blocker() {
            return Err(EngineError::Disabled("collab.spotlight".into(), why.into()));
        }
        let Some(live) = self.live_mut() else { return Ok(()) };
        if on {
            live.unfollow();
            live.spotlight = None;
        }
        live.spotlighting = on;
        let id = live.id;
        self.live.send(id, [ClientMsg::Signal(Signal::spotlight(on))]);
        Ok(())
    }

    /// Session `id`'s welcome: start the sync client; the host keeps editing the document it
    /// shared, a guest opens the host's file as a new tab (or gets it again in that tab after a
    /// reconnect).
    fn live_welcome(&mut self, id: SessionId, welcome: ServerMsg) {
        let client = match Client::join(welcome) {
            Ok(c) => c,
            Err(e) => return self.live_fail(id, format!("Couldn't open the shared file: {e}")),
        };
        let Some(live) = self.live.sessions.get(&id) else { return };
        let existing = live.doc_uid.filter(|u| self.docs.iter().any(|d| d.uid == *u));
        let (uid, synced) = match (live.role(), existing) {
            (Some(Role::Host), Some(uid)) => {
                if let Some(ds) = self.docs.iter_mut().find(|d| d.uid == uid) {
                    let was_dirty = ds.is_dirty();
                    // Undo is per user from now on: earlier whole-document steps would revert
                    // others' edits.
                    ds.history.undo.clear();
                    ds.history.redo.clear();
                    ds.doc_mut().set_client_id(client.id());
                    if !was_dirty {
                        ds.mark_saved();
                    }
                }
                // Edits made since hosting started go out with the first sync.
                (uid, None)
            }
            (Some(Role::Guest), Some(uid)) => {
                // Joining again in this tab, or the file you published arriving from the server.
                let doc = Arc::new(client.document().clone());
                let was_dirty = self.docs.iter().find(|d| d.uid == uid).is_some_and(DocState::is_dirty);
                self.apply_remote(uid, doc.clone());
                if let Some(ds) = self.docs.iter_mut().find(|d| d.uid == uid) {
                    // Undo is per user from now on: earlier whole-document steps would revert
                    // others' edits.
                    ds.history.undo.clear();
                    ds.history.redo.clear();
                    if !was_dirty {
                        ds.mark_saved();
                    }
                }
                (uid, Some(doc))
            }
            (Some(Role::Guest), None) => {
                // The session's file gets a tab of its own, in front.
                let ds = DocState::new(client.document().clone(), None);
                self.message = Some(format!("Joined {}", ds.doc.name));
                let opened = (ds.uid, Some(ds.doc.clone()));
                self.active = self.docs.len();
                self.docs.push(ds);
                opened
            }
            _ => return,
        };
        // A view link: the editing tools are off.
        if client.access() == Access::View && self.tab() == Some(uid) && !matches!(self.tool.tool, ToolId::Move | ToolId::Hand) {
            self.set_tool(ToolId::Move);
        }
        let Some(live) = self.live.sessions.get_mut(&id) else { return };
        let doc = client.document();
        if let Some(info) = live.info.as_mut() {
            info.file_name = doc.name.clone();
            info.pages = doc.pages.iter().map(|p| PageInfo { id: p.0, name: doc.node(*p).map(|n| n.name.clone()).unwrap_or_default() }).collect();
        }
        live.doc_uid = Some(uid);
        live.tab = Some(uid);
        live.synced = synced;
        live.client = Some(client);
        live.deferred.clear();
    }

    /// Session `id` is over with a message: it couldn't be joined, the connection was lost for
    /// good or sync broke. Its tab shows why.
    fn live_fail(&mut self, id: SessionId, reason: String) {
        log::warn!("live session: {reason}");
        self.live.leave(id);
        if let Some(live) = self.live.sessions.get_mut(&id) {
            live.reset();
            live.status = Status::Failed { reason: reason.clone() };
        }
        self.message = Some(reason);
    }

    /// Why document `uid` can't be edited now, if it can't: you may only view it in its session,
    /// or it is on its way to a server.
    pub fn live_read_only(&self, uid: u64) -> Option<&'static str> {
        let live = self.live.doc(uid)?;
        match &live.client {
            Some(c) if c.access() == Access::View => Some(VIEW_ONLY),
            Some(_) => None,
            None if live.status.active() && live.role() != Some(Role::Host) => Some(PUBLISHING),
            None => None,
        }
    }

    /// The active document can't be edited now (see [`live_read_only`](Self::live_read_only)).
    pub fn active_read_only(&self) -> Option<&'static str> {
        self.tab().and_then(|uid| self.live_read_only(uid))
    }

    /// Whether document `uid` is a live session's synced document: its edits sync and its undo
    /// history is the user's own.
    pub fn live_doc(&self, uid: u64) -> bool {
        self.live.doc(uid).is_some_and(Live::syncing)
    }

    /// Sync the edits of session document `uid` as one batch: one step of the user's undo history
    /// called `label`, or not undoable (`None`: also what changed it outside a command). Nothing
    /// while a gesture or preview shows, or inside a group (it syncs when the group ends).
    pub(crate) fn live_edit(&mut self, uid: u64, label: Option<&str>) {
        let Some(ds) = self.docs.iter().find(|d| d.uid == uid) else { return };
        let Some(live) = self.live.doc(uid) else { return };
        if live.hold > 0 || busy(ds) || live.synced.as_ref().is_some_and(|s| Arc::ptr_eq(s, &ds.doc)) {
            return;
        }
        let (id, doc, synced) = (live.id, ds.doc.clone(), live.synced.clone());
        if self.live_read_only(uid).is_some() {
            // Nothing goes out: what changed outside a command is put back.
            if let Some(s) = synced {
                self.apply_remote(uid, s);
            }
            return;
        }
        let Some(live) = self.live.sessions.get_mut(&id) else { return };
        let Some(client) = live.client.as_mut() else { return };
        match client.edit_with(&doc, label) {
            Ok(msg) => {
                live.synced = Some(doc);
                self.live.send(id, msg);
            }
            Err(e) => self.live_fail(id, format!("Couldn't sync an edit: {e}")),
        }
    }

    /// A group of commands on document `uid` starts (`on`) or ends: in between its edits don't
    /// sync on their own.
    pub(crate) fn live_hold(&mut self, uid: u64, on: bool) {
        if let Some(live) = self.live.doc_mut(uid).filter(|l| l.syncing()) {
            live.hold = if on { live.hold.saturating_add(1) } else { live.hold.saturating_sub(1) };
        }
    }

    /// Undo (`back`) or redo the user's own last step on the active document, in its session:
    /// what it wrote goes back where nobody else wrote since. Returns whether there was a step to
    /// take back.
    pub(crate) fn live_travel(&mut self, back: bool) -> bool {
        let Some(uid) = self.tab() else { return false };
        // Edits not synced yet go first.
        self.live_edit(uid, None);
        let Some(live) = self.live.doc_mut(uid) else { return false };
        let Some(client) = live.client.as_mut() else { return false };
        let (undo, redo) = client.history_len();
        let msgs = match if back { client.undo(&EngineRecompute) } else { client.redo(&EngineRecompute) } {
            Ok(m) => m,
            Err(e) => {
                log::warn!("live session: {} failed: {e}", if back { "undo" } else { "redo" });
                return false;
            }
        };
        let (u, r) = client.history_len();
        let did = if back { r > redo } else { u > undo };
        let doc = Arc::new(client.document().clone());
        if did {
            live.synced = Some(doc.clone());
        }
        let id = live.id;
        self.live.send(id, msgs);
        if did {
            self.apply_remote(uid, doc);
        }
        did
    }

    /// Apply the server's batches that waited in session `id` (none while a gesture or preview
    /// shows on its document).
    fn live_deferred(&mut self, id: SessionId) {
        let Some(live) = self.live.sessions.get(&id).filter(|l| !l.deferred.is_empty() && l.syncing()) else { return };
        let Some(uid) = live.doc_uid else { return };
        if self.docs.iter().find(|d| d.uid == uid).is_none_or(busy) {
            return;
        }
        // Your own edits first: the client replays them over what others did.
        self.live_edit(uid, None);
        let Some(live) = self.live.sessions.get_mut(&id) else { return };
        let Some(client) = live.client.as_mut() else { return };
        let mut out = Vec::new();
        for m in std::mem::take(&mut live.deferred) {
            match client.receive(m, &EngineRecompute) {
                Ok(more) => out.extend(more),
                Err(e) => return self.live_fail(id, format!("The live session got out of step ({e}); join again")),
            }
        }
        let doc = Arc::new(client.document().clone());
        // Edits the server refused were rolled back: say why (the UI's toast).
        let notices: Vec<String> = client.take_notices().iter().map(figcraft_collab::Notice::message).collect();
        live.synced = Some(doc.clone());
        if !notices.is_empty() {
            self.message = Some(notices.join("; "));
        }
        self.live.send(id, out);
        self.apply_remote(uid, doc);
    }

    /// Show the peer followed in session `id`: their viewport (and their page, in the session's
    /// document) on the session's tab.
    fn live_follow_apply(&mut self, id: SessionId) {
        let Some(live) = self.live.sessions.get_mut(&id) else { return };
        let Some(peer) = live.follow else { return };
        if !live.peers.contains_key(&peer) {
            live.unfollow();
            return;
        }
        let Some((page, x, y, w, h)) = live.presence.get(&peer).and_then(|p| p.viewport) else { return };
        let Some(ds) = self.docs.iter_mut().find(|d| Some(d.uid) == live.tab) else { return };
        if live.syncing() && page != ds.page && ds.doc.pages.contains(&page) {
            ds.page = page;
            ds.selection.clear();
            ds.text_edit = None;
        }
        if !(w > 0.0 && h > 0.0 && x.is_finite() && y.is_finite()) {
            return;
        }
        ds.view.cx = x + w / 2.0;
        ds.view.cy = y + h / 2.0;
        let zoom = (ds.view.vw / w).min(ds.view.vh / h);
        if zoom.is_finite() {
            ds.view.zoom = zoom.clamp(ViewState::MIN_ZOOM, ViewState::MAX_ZOOM);
        }
        ds.revision += 1;
        live.followed = Some(FollowedView { page: ds.page, cx: ds.view.cx, cy: ds.view.cy, zoom: ds.view.zoom });
    }

    /// Sessions whose tab was closed end (hosts stop hosting, guests leave); the others go on. A
    /// join still on its way from a closed tab shows on the one in front.
    fn live_closed(&mut self) {
        let (docs, tab) = (&self.docs, self.tab());
        let mut gone = Vec::new();
        for l in self.live.sessions.values_mut().filter(|l| l.tab != tab && !docs.iter().any(|d| Some(d.uid) == l.tab)) {
            if l.doc_uid.is_none() && l.status.active() {
                l.tab = tab;
            } else {
                gone.push((l.id, l.status == Status::Live));
            }
        }
        for (id, was_live) in gone {
            self.live.end(id);
            if was_live {
                self.message = Some("The live session's file was closed, so you left the session".into());
            }
        }
    }

    /// Send your edits and presence in every session when they changed (headless sessions call
    /// it after each command). A view you moved yourself stops following.
    pub fn live_sync(&mut self) {
        self.live_sync_in(|_| true);
    }

    /// [`live_sync`](Self::live_sync) for the sessions whose document (`None`: a join on its way)
    /// `mine` picks: a window calls it every frame it draws, after input, for the documents it
    /// holds.
    pub fn live_sync_in(&mut self, mine: impl Fn(Option<u64>) -> bool) {
        // A recovery copy whose session was left has its outcome too.
        if !self.live.recovering.is_empty() {
            self.online_recoveries();
        }
        if self.live.sessions.is_empty() {
            return;
        }
        self.live_closed();
        let ids: Vec<SessionId> = self.live.sessions.values().filter(|l| l.status == Status::Live && mine(l.doc_uid)).map(|l| l.id).collect();
        for id in ids {
            self.live_deferred(id);
            if let Some(uid) = self.live.get(id).and_then(|l| l.doc_uid) {
                self.live_edit(uid, None);
            }
            self.live_present(id);
        }
    }

    /// Hand your presence in session `id` to the link when it changed.
    fn live_present(&mut self, id: SessionId) {
        let Some(live) = self.live.get(id).filter(|l| l.status == Status::Live) else { return };
        let moved = live.followed.is_some_and(|f| {
            self.docs
                .iter()
                .find(|d| Some(d.uid) == live.tab)
                .is_some_and(|ds| ds.page != f.page || !close(ds.view.cx, f.cx) || !close(ds.view.cy, f.cy) || !close(ds.view.zoom, f.zoom))
        });
        let p = self.presence_in(live);
        let changed = live.sent.as_ref() != Some(&p);
        let Some(live) = self.live.sessions.get_mut(&id) else { return };
        if moved {
            live.unfollow();
        }
        if changed {
            if let Some(link) = self.live.link.as_mut() {
                link.presence(id, &p);
            }
            live.sent = Some(p);
        }
    }

    /// Stop following in the active document's session (Escape, `collab.follow {}`). False when
    /// you weren't following anyone.
    pub fn live_unfollow(&mut self) -> bool {
        self.live_mut().is_some_and(Live::unfollow)
    }

    /// Follow `peer`'s viewport in the active document's session (shown at once when known).
    pub fn live_follow(&mut self, peer: u32) -> Result<()> {
        let Some(live) = self.live_mut().filter(|l| l.peers.contains_key(&peer)) else {
            return Err(bad("collab.follow", format!("nobody with client id {peer} is in the session")));
        };
        live.follow = Some(peer);
        live.followed = None;
        let id = live.id;
        self.live_follow_apply(id);
        Ok(())
    }

    /// The active document's session as JSON (`collab.peers`), with every session in brief.
    pub fn live_summary(&self) -> Value {
        self.summary(self.live())
    }

    /// The index of the open document session `l` syncs.
    fn live_index(&self, l: &Live) -> Option<usize> {
        l.doc_uid.and_then(|u| self.docs.iter().position(|d| d.uid == u))
    }

    fn summary(&self, l: &Live) -> Value {
        let info = l.info.as_ref();
        let reason = |l: &Live| match &l.status {
            Status::Failed { reason } => Some(reason.clone()),
            _ => None,
        };
        let synced = self.live_index(l).and_then(|i| self.docs.get(i));
        // Who types in which layer of the session document.
        let editing = |e: &EditingText| {
            let layer = synced.and_then(|d| d.doc.node(e.node)).map(|n| n.name.as_str());
            json!({"node": e.node, "layer": layer, "caret": e.caret, "selection": e.selection})
        };
        let peers: Vec<Value> = l
            .peers
            .values()
            .map(|p| {
                let presence = l.presence.get(&p.client_id);
                let typing = presence.and_then(|pr| pr.editing_text.as_deref()).map(editing);
                let chat = l.chats.get(&p.client_id);
                json!({"id": p.client_id, "name": p.name, "color": p.color, "editing": typing, "chat": chat, "presence": presence})
            })
            .collect();
        let mine = synced.and_then(|d| d.text_edit.as_ref()).map(|t| editing(&editing_of(t)));
        let sessions: Vec<Value> = self
            .live
            .iter()
            .map(|o| {
                let info = o.info.as_ref();
                json!({
                    "session": o.id,
                    "document": self.live_index(o),
                    "status": o.status.id(),
                    "reason": reason(o),
                    "role": o.role(),
                    "fileName": info.map(|i| i.file_name.as_str()),
                    "link": info.map(|i| i.link.as_str()),
                    "peers": o.peers.len(),
                })
            })
            .collect();
        json!({
            "available": self.live.available(),
            "canHost": self.live.can_host(),
            "session": (l.id != NO_SESSION).then_some(l.id),
            "status": l.status.id(),
            "reason": reason(l),
            "attempt": match l.status { Status::Reconnecting { attempt } => Some(attempt), _ => None },
            "role": info.map(|i| i.role),
            "me": info.map(|i| i.me),
            "name": info.map(|i| i.name.as_str()),
            "color": info.map(|i| i.color.as_str()),
            "fileName": info.map(|i| i.file_name.as_str()),
            "pages": info.map(|i| &i.pages),
            "link": info.map(|i| i.link.as_str()),
            "url": info.map(|i| i.url.as_str()),
            "address": info.map(|i| i.address.as_str()),
            "network": info.map(|i| i.network),
            "access": l.client.as_ref().map(Client::access).or(info.map(|i| i.access)),
            "readOnly": l.doc_uid.is_some_and(|u| self.live_read_only(u).is_some()),
            "published": l.published,
            "code": l.share.as_ref().and_then(Share::code),
            "share": l.share.as_ref().map(|sh| {
                let (edit, view) = (sh.links.as_ref().map(|s| &s.edit), sh.links.as_ref().and_then(|s| s.view.as_ref()));
                let code = |l: Option<&SharedLink>| l.and_then(|l| l.code.as_deref()).map(code::grouped);
                json!({
                    "via": sh.via,
                    "access": sh.access.id(),
                    "shared": sh.links.is_some(),
                    "editCode": code(edit),
                    "viewCode": code(view),
                    "editLink": edit.map(|l| l.link.as_str()),
                    "viewLink": view.map(|l| l.link.as_str()),
                })
            }),
            "hosting": l.hosting.as_ref().map(hosting_json),
            "follow": l.follow,
            "signals": l.signals,
            "chat": l.chat,
            "spotlight": l.spotlight,
            "spotlighting": l.spotlighting,
            "syncing": l.syncing(),
            "document": self.live_index(l),
            "editing": mine,
            "peers": peers,
            "sessions": sessions,
        })
    }
}

/// What a server says about its hosting, made safe to show: no control characters, short, and a
/// terms address only when it is a web page (`http(s)://`).
fn clean_hosting(mut info: HostInfo) -> HostInfo {
    let clean = |s: &str, n: usize| s.trim().chars().filter(|c| !c.is_control()).take(n).collect::<String>();
    info.operator = clean(&info.operator, 120);
    info.terms = clean(&info.terms, 500);
    let web = info.terms.starts_with("https://") || info.terms.starts_with("http://");
    if !web || info.terms.contains(char::is_whitespace) {
        info.terms.clear();
    }
    info
}

/// A server's hosting as JSON (`collab.peers`, `collab.serverInfo`).
pub fn hosting_json(h: &HostInfo) -> Value {
    json!({
        "open": h.open,
        "operator": h.operator,
        "terms": h.terms,
        "idleDays": (h.idle_secs > 0).then(|| (h.idle_secs as f64 / 86_400.0 * 100.0).round() / 100.0),
        "maxFileMb": h.max_file_mb,
        "expires": h.expires,
    })
}

fn unavailable(cmd: &str) -> EngineError {
    EngineError::Disabled(
        cmd.to_string(),
        "live sessions aren't available here (the desktop app and the MCP server host and join them; the web build joins)".into(),
    )
}

fn host(s: &mut Session, p: &Value) -> Result<Value> {
    start_host(s, "collab.host", p, Bind::Local, false)?;
    Ok(s.live_summary())
}

/// Start hosting the open file for command `cmd` (on `bind` unless `p` says where; `short`: with
/// a secret a join code can carry): the new session.
fn start_host(s: &mut Session, cmd: &str, p: &Value, bind: Bind, short: bool) -> Result<SessionId> {
    let port = match get(p, "port") {
        None => None,
        Some(v) => Some(v.as_u64().and_then(|n| u16::try_from(n).ok()).ok_or_else(|| bad(cmd, "`port` must be a whole number from 0 to 65535"))?),
    };
    let bind = match string(p, "bind").as_deref().map(str::trim) {
        None => bind,
        Some("local") => Bind::Local,
        Some("lan") => Bind::Lan,
        Some(other) => other.parse::<IpAddr>().map(Bind::Addr).map_err(|_| bad(cmd, "`bind` must be \"local\", \"lan\" or an IP address"))?,
    };
    let (doc, uid) = s.ds().map(|ds| (ds.doc.clone(), ds.uid))?;
    let opts = HostOptions { port, bind, name: display_name(&s.prefs), doc, short };
    let (id, info) = s.live.start(cmd, |l, id| l.host(id, opts).map_err(|e| EngineError::Other(format!("couldn't start the live session: {e}"))))?;
    s.live.open(Live { id, status: Status::Live, info: Some(info), doc_uid: Some(uid), tab: Some(uid), ..Live::default() });
    Ok(id)
}

fn publish(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "collab.publish";
    let server = req_string(CMD, p, "server")?;
    start_publish(s, CMD, &server, p, false)?;
    Ok(s.live_summary())
}

/// Start publishing the open file to `server` for command `cmd` (`share`: with links and join
/// codes for others too): the new session.
fn start_publish(s: &mut Session, cmd: &str, server: &str, p: &Value, share: bool) -> Result<SessionId> {
    let key = string(p, "key").map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
    let (doc, uid) = s.ds().map(|ds| (ds.doc.clone(), ds.uid))?;
    let name = string(p, "name").map(|n| n.trim().chars().filter(|c| !c.is_control()).take(256).collect::<String>()).filter(|n| !n.is_empty());
    let me = display_name(&s.prefs);
    let opts = PublishOptions { server: server.trim().to_string(), key, name: name.unwrap_or_else(|| doc.name.clone()), me, doc, share };
    let (id, ()) = s.live.start(cmd, |l, id| l.publish(id, opts).map_err(|e| bad(cmd, e)))?;
    // This tab becomes the server file's synced document once it is there.
    s.live.open(Live { id, status: Status::Connecting, doc_uid: Some(uid), tab: Some(uid), server: server.trim().to_string(), ..Live::default() });
    Ok(id)
}

/// Why a local network share has no view-only code.
const LOCAL_EDITS: &str = "everyone who joins a file shared on the local network can edit it: share it online for a view-only code";

fn share(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "collab.share";
    let via = match string(p, "via").as_deref().map(str::trim) {
        None | Some("online") => Via::Online,
        Some("local") => Via::Local,
        Some(_) => return Err(bad(CMD, "`via` must be \"online\" or \"local\"")),
    };
    let access = match string(p, "access") {
        None => None,
        Some(a) => Some(Access::parse(&a).ok_or_else(|| bad(CMD, "`access` must be \"edit\" or \"view\""))?),
    };
    // A share of this file is there already: only which code is shown changes, or (the file is
    // on the server, not shared at the moment) it gets new links and codes. Nothing is published
    // twice.
    if let Some(shared) = s.live().share.as_ref().filter(|_| s.live().status.active()).map(|sh| sh.via) {
        if get(p, "via").is_some() && shared != via {
            return Err(bad(
                CMD,
                "this file is shared the other way already: stop sharing it first (collab.stop), or leave its session (collab.leave)",
            ));
        }
        if access == Some(Access::View) && shared == Via::Local {
            return Err(bad(CMD, LOCAL_EDITS));
        }
        if let Some(req) = share_request(s, Vec::new(), true).filter(|_| s.live().share.as_ref().is_some_and(|sh| sh.links.is_none())) {
            let links = share_answer(s, req)?.ok_or_else(|| EngineError::Other("the server made no links".into()))?;
            shared_now(s, Some(clean_shared(links)));
        }
        if let (Some(a), Some(sh)) = (access, s.live_mut().and_then(|l| l.share.as_mut())) {
            sh.access = a;
        }
        return Ok(s.live_summary());
    }
    let access = access.unwrap_or_default();
    let mut server = String::new();
    let id = match via {
        Via::Local if access == Access::View => return Err(bad(CMD, LOCAL_EDITS)),
        Via::Local => start_host(s, CMD, p, Bind::Lan, true)?,
        Via::Online => {
            server = string(p, "server").unwrap_or_else(|| s.prefs.publish_server.clone()).trim().to_string();
            if server.is_empty() {
                return Err(EngineError::Disabled(CMD.into(), ONLINE_OFF.into()));
            }
            start_publish(s, CMD, &server, p, true)?
        }
    };
    if let Some(live) = s.live.sessions.get_mut(&id) {
        // On the local network the session's own link is what others join with, and the code says
        // what the link says: where the session is and its secret. (Online they arrive with the file.)
        let links = live.info.as_ref().filter(|_| via == Via::Local).map(|i| {
            let code = code::Local::from_link(&i.link).map(|l| l.code());
            Shared { edit: SharedLink { id: String::new(), link: i.link.clone(), code }, view: None }
        });
        live.share = Some(Share { via, server, access, links });
    }
    Ok(s.live_summary())
}

/// What others join the open file's server file with changed: in its session and its record.
fn shared_now(s: &mut Session, links: Option<Shared>) {
    if let Some((server, file)) = s.live().server_file() {
        s.online_shared(&server, &file, links);
    }
}

/// Links and codes as a server gave them, with only codes that are codes (nine symbols).
pub(crate) fn clean_shared(mut links: Shared) -> Shared {
    let clean = |l: &mut SharedLink| l.code = l.code.take().and_then(|c| code::normalize(&c).ok()).filter(|c| c.len() == code::ONLINE_LEN);
    clean(&mut links.edit);
    links.view.as_mut().map(clean);
    links
}

/// The request that changes who else gets into the server file the open file's session is its
/// owner's session of (`None`: it isn't one, or the file isn't there yet).
fn share_request(s: &Session, revoke: Vec<String>, create: bool) -> Option<NetRequest> {
    let live = s.live();
    let (share, published) = (live.share.as_ref().filter(|s| s.via == Via::Online)?, live.published.as_ref()?);
    if live.status != Status::Live {
        return None;
    }
    // The token this session got when it published the file, else the recorded one.
    let owner = published.owner.clone().or_else(|| s.online_owner(&share.server, &published.file)).unwrap_or_default();
    Some(NetRequest::Share { server: share.server.clone(), file: published.file.clone(), owner, revoke, create })
}

/// Send `req` ([`NetRequest::Share`]): the new links, if it made any.
pub(crate) fn share_answer(s: &mut Session, req: NetRequest) -> Result<Option<Shared>> {
    match s.net(req)?.0 {
        NetReply::Share(Ok(links)) => Ok(links),
        NetReply::Share(Err(why)) => Err(EngineError::Other(why)),
        _ => Err(EngineError::Other("the server answered something else".into())),
    }
}

/// What the server at `server` says about its hosting, and its answer to `req` about `file` with
/// its owner token (`collab.serverInfo`, `collab.manageFile`).
fn hosting_request(s: &mut Session, cmd: &str, server: &str, file: &str, owner: &str, req: Option<Admin>) -> Result<(HostInfo, Option<AdminReply>)> {
    let req = NetRequest::Hosting { server: server.to_string(), file: file.to_string(), owner: owner.to_string(), req };
    match s.net(req)?.0 {
        // The server's refusals say why themselves.
        NetReply::Hosting(Ok((_, Some(AdminReply::Failed { reason })))) | NetReply::Hosting(Err(reason)) => Err(EngineError::Other(reason)),
        NetReply::Hosting(Ok((info, reply))) => Ok((clean_hosting(info), reply)),
        _ => Err(EngineError::Other(format!("{cmd}: the server answered something else"))),
    }
}

fn server_info(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "collab.serverInfo";
    let server = req_string(CMD, p, "server")?;
    let (info, _) = hosting_request(s, CMD, server.trim(), "", "", None)?;
    Ok(hosting_json(&info))
}

fn manage_file(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "collab.manageFile";
    let file = req_string(CMD, p, "file")?.trim().to_string();
    let (server, owner) = crate::online::manager(s, CMD, p, &file)?;
    let action = req_string(CMD, p, "action")?;
    let mut renamed = None;
    let req = match action.trim() {
        "share" => return crate::online::set_shared(s, CMD, &server, &file, &owner, true),
        "unshare" => return crate::online::set_shared(s, CMD, &server, &file, &owner, false),
        "links" => Admin::Links { file: file.clone() },
        "createLink" => {
            let access = match string(p, "access").as_deref().map(str::trim) {
                None | Some("view") => Access::View,
                Some("edit") => Access::Edit,
                Some(_) => return Err(bad(CMD, "`access` must be \"edit\" or \"view\"")),
            };
            Admin::CreateLink { file: file.clone(), access, label: string(p, "label").unwrap_or_default() }
        }
        "revoke" => Admin::Revoke { file: file.clone(), link: req_string(CMD, p, "link")?.trim().to_string() },
        "rename" => {
            let name = req_string(CMD, p, "name")?;
            renamed = Some(name.clone());
            Admin::Rename { file: file.clone(), name }
        }
        "remove" => Admin::Remove { file: file.clone() },
        _ => return Err(bad(CMD, "`action` must be links, createLink, revoke, share, unshare, rename or remove")),
    };
    let changes = matches!(req, Admin::Rename { .. } | Admin::Remove { .. });
    let reply = hosting_request(s, CMD, &server, &file, &owner, Some(req))?.1;
    // The record follows what the server did.
    if changes && reply == Some(AdminReply::Done) {
        crate::online::managed(s, &server, &file, renamed.as_deref());
        if renamed.is_none() {
            crate::online::removed(s, &server, &file);
        }
    }
    match reply {
        Some(AdminReply::Links(links)) => Ok(json!({"links": links.iter().map(|l| json!({
            "id": l.id, "access": l.access.id(), "label": l.label, "created": l.created
        })).collect::<Vec<_>>()})),
        // The link layer gives the new link whole in place of its token.
        Some(AdminReply::Link(l)) => Ok(json!({"id": l.link.id, "access": l.link.access.id(), "link": l.token})),
        Some(AdminReply::Done) => Ok(json!({"done": true})),
        _ => Err(bad(CMD, "the server answered something else")),
    }
}

fn report(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "collab.report";
    let reason: String = req_string(CMD, p, "reason")?.trim().chars().map(|c| if c.is_control() { ' ' } else { c }).take(MAX_REPORT).collect();
    if reason.is_empty() {
        return Err(bad(CMD, "say in a few words what is wrong with the file (`reason`)"));
    }
    let live = s.live();
    if live.status != Status::Live || live.role() != Some(Role::Guest) {
        return Err(bad(CMD, "join a file on a FigCraft server first: reports go to that server's operator"));
    }
    if live.hosting.is_none() {
        return Err(bad(CMD, "this server doesn't take reports (it is older than this app); contact its operator another way"));
    }
    let id = live.id;
    s.live.send(id, [ClientMsg::Report { reason }]);
    Ok(json!({"sent": true}))
}

/// Longest report sent (characters).
pub const MAX_REPORT: usize = 500;

fn join(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "collab.join";
    let typed = match string(p, "code") {
        Some(code) => code,
        None => req_string(CMD, p, "link").map_err(|_| bad(CMD, "give the join `code` you were told, or the invite `link`"))?,
    };
    let link = join_target(&s.prefs.publish_server, &typed).map_err(|e| bad(CMD, e))?;
    let id = start_join(s, CMD, &link, None, None, None)?;
    Ok(s.live.get(id).map_or(Value::Null, |l| s.summary(l)))
}

/// Start joining the session behind `link` for command `cmd`: the new session. `published` and
/// `share`: the file is yours on a server (`collab.openFile`), with what others join it with;
/// `resume`: as the client you were there.
pub(crate) fn start_join(
    s: &mut Session,
    cmd: &str,
    link: &str,
    published: Option<Published>,
    share: Option<Share>,
    resume: Option<Resume>,
) -> Result<SessionId> {
    let (name, tab) = (display_name(&s.prefs), s.tab());
    let (id, ()) = s.live.start(cmd, |l, id| l.join_resuming(id, link, &name, resume).map_err(|e| bad(cmd, e)))?;
    let server = share.as_ref().map(|s| s.server.clone()).unwrap_or_default();
    // The session's file opens in a tab of its own; until then the join shows on this one.
    s.live.open(Live { id, status: Status::Connecting, tab, server, published, share, ..Live::default() });
    Ok(id)
}

fn leave(s: &mut Session, _: &Value) -> Result<Value> {
    let ended = s.live.shown(s.tab()).and_then(|id| s.live.end(id));
    if ended.as_ref().is_some_and(|l| l.syncing() && l.role() == Some(Role::Guest)) {
        s.message = Some("You left the session; your copy stays open here".into());
    }
    Ok(json!({"ended": ended.is_some_and(|l| l.status.active())}))
}

fn stop(s: &mut Session, p: &Value) -> Result<Value> {
    // A file shared online stays its owner's on the server: stopping takes the others' links (and
    // their codes) away, which disconnects whoever joined with them, and the owner stays in the
    // file. Anything else (a session this app hosts, one it joined, a file not shared now) ends.
    let shared = s.live().share.as_ref().and_then(|sh| sh.links.as_ref());
    let ids: Vec<String> = shared.into_iter().flat_map(|l| [Some(&l.edit), l.view.as_ref()]).flatten().map(|l| l.id.clone()).collect();
    let Some(req) = share_request(s, ids, false).filter(|_| shared.is_some()) else { return leave(s, p) };
    share_answer(s, req).map_err(|e| EngineError::Other(format!("couldn't stop sharing (the others can still get in): {e}")))?;
    shared_now(s, None);
    s.message = Some("Sharing stopped: the file stays on the server, open only to you".into());
    Ok(json!({"ended": false, "shared": false}))
}

fn follow(s: &mut Session, p: &Value) -> Result<Value> {
    match get(p, "client") {
        None => {
            s.live_unfollow();
        }
        Some(v) => {
            let id = v.as_u64().and_then(|n| u32::try_from(n).ok()).ok_or_else(|| bad("collab.follow", "`client` must be a client id"))?;
            s.live_follow(id)?;
        }
    }
    Ok(json!({"follow": s.live().follow}))
}

/// Remove someone from the open file's session: in a session this app hosts, the hub drops them;
/// in a file of yours on a server, the server does (with the file's owner token). They are told
/// the session is over for them, keep their copy, and can't join again from their address while
/// the session (or the server) runs.
fn remove_person(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "collab.removePerson";
    let client =
        get(p, "client").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).ok_or_else(|| bad(CMD, "`client` must be a client id"))?;
    let live = s.live();
    if live.status != Status::Live {
        return Err(bad(CMD, "the open file isn't in a live session"));
    }
    let name =
        live.peers.get(&client).map(|p| p.name.clone()).ok_or_else(|| bad(CMD, format!("nobody with client id {client} is in the session")))?;
    let (id, role, server_file) = (live.id, live.role(), live.server_file());
    match (role, server_file) {
        (Some(Role::Host), _) => {
            let link = s.live.link.as_deref_mut().ok_or_else(|| unavailable(CMD))?;
            link.remove(id, client).map_err(EngineError::Other)?;
        }
        (Some(Role::Guest), Some((server, file))) => {
            let owner =
                s.live().published.as_ref().and_then(|p| p.owner.clone()).or_else(|| s.online_owner(&server, &file)).filter(|o| !o.is_empty());
            let owner = owner.ok_or_else(|| bad(CMD, "only the file's owner can remove people from it"))?;
            let req = Some(Admin::RemovePeer { file: file.clone(), client });
            match hosting_request(s, CMD, &server, &file, &owner, req) {
                Ok((_, Some(AdminReply::Done))) => {}
                Ok(_) => return Err(EngineError::Other(format!("couldn't remove {name}: the server answered something else"))),
                // A server older than this request can't read it.
                Err(e) => return Err(EngineError::Other(format!("couldn't remove {name}: {e} (the server may be older than removing people)"))),
            }
        }
        _ => return Err(bad(CMD, "only the person hosting the session, or the file's owner, can remove people")),
    }
    s.message = Some(format!("{name} was removed from the session"));
    Ok(json!({"removed": client, "name": name}))
}

/// You may remove others from the open file's session: you host it, or it is your file on a
/// server.
pub fn can_remove_people(s: &Session) -> bool {
    let live = s.live();
    live.status == Status::Live
        && match live.role() {
            Some(Role::Host) => true,
            Some(Role::Guest) => live.server_file().is_some_and(|(server, file)| {
                live.published.as_ref().and_then(|p| p.owner.as_deref()).is_some_and(|o| !o.is_empty())
                    || s.online_owner(&server, &file).is_some_and(|o| !o.is_empty())
            }),
            None => false,
        }
}

fn cursor(s: &mut Session, p: &Value) -> Result<Value> {
    let at = match (num(p, "x"), num(p, "y")) {
        (Some(x), Some(y)) => Some((x, y)),
        (None, None) => None,
        _ => return Err(bad("collab.cursor", "give both `x` and `y` (page coordinates), or neither to hide the cursor")),
    };
    if let Some(l) = s.live_mut() {
        l.cursor = at;
    }
    s.live_sync();
    Ok(json!({"cursor": s.live().cursor.map(|(x, y)| [x, y])}))
}

fn cursor_chat(s: &mut Session, p: &Value) -> Result<Value> {
    let Some(v) = get(p, "text") else {
        // The app's chat box (programmatic calls never open dialogs).
        let open = s.signal_blocker().is_none();
        if open {
            s.request_ui("dialog.cursorChat", json!({}));
        }
        return Ok(json!({"open": open, "chat": s.live().chat}));
    };
    let text = v.as_str().ok_or_else(|| bad("collab.cursorChat", "`text` must be a string (empty closes the chat)"))?;
    Ok(json!({"chat": s.live_chat(text)?}))
}

fn spotlight(s: &mut Session, p: &Value) -> Result<Value> {
    let on = match get(p, "on") {
        None => !s.live().spotlighting,
        Some(v) => v.as_bool().ok_or_else(|| bad("collab.spotlight", "`on` must be true or false"))?,
    };
    s.live_spotlight(on)?;
    Ok(json!({"spotlighting": s.live().spotlighting}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "collab.host",
            "Start live session",
            "{port? (default 47900, else any free port), bind?: \"local\" (this machine, default) | \"lan\" (the local network) | an IP address} → the open file's session: {session, link, url (invites: they carry the secret token), address, …}; sessions are per file, so other files' sessions go on (a session this file was in ends)",
            false,
            host
        ),
        spec!(
            "collab.share",
            "Share with a join code",
            "{via?: \"online\" (default: the open file is kept on the server in Preferences, `pref.publishServer`, and the server hands out the codes) | \"local\" (this app hosts it for people on the same network; no server), access?: \"edit\" (default) | \"view\" (online only: which code `code` is), server? (online: another server's address, with ?key=… when it needs its key), name? (online: the file's name there), port?, bind?: \"lan\" (default) | \"local\" (this machine only) | an IP address (local)} → the open file's session as `collab.peers` gives it: `code` is what others type to join (`collab.join {code}`, the Join field), `link` an invite link. Local: at once, the code carries the host's IPv4 address, port and secret (K7M2Q-…, 20 symbols). Online: status \"connecting\" first; once live `code` is the server's (K7M-2QX-9TD) and `share` is {via, access, shared, editCode, viewCode, editLink, viewLink}: links made for others (the file's own links in `published` stay yours); `code` stays null when the server is older than join codes (give `share.editLink` / `share.viewLink` instead). Called again while shared: only switches which code `code` is (`access`). Called after `collab.stop` on a file that is still on the server: new links and codes for that same file (nothing is published twice). `collab.stop` stops sharing",
            false,
            share
        ),
        spec!(
            "collab.stop",
            "Stop live session",
            "{} → {ended}: the open file's session ends (a session this app hosts stops for everyone; a joined one is left). A file shared online with `collab.share` is different: its sharing stops → {ended: false, shared: false}: the links and codes others got are revoked on the server, whoever joined with them is disconnected at once (they keep their copy), and you stay in the file, which stays on the server, open only to you (`collab.share` shares it again with new codes; `collab.leave` leaves it there; `collab.manageFile {action: \"remove\"}` with its owner token takes it off the server)",
            false,
            stop
        ),
        spec!(
            "collab.join",
            "Join live session",
            "{code: a join code (K7M-2QX-9TD from a server, asked at the server in Preferences; a longer one for the local network; spaces, dashes and case don't matter, O reads as 0 and I or L as 1) | link: figcraft://join?host=…&file=…&token=…, figcraft://join?server=ws(s)://…&file=…&token=… or ws(s)://host:port/?file=…&token=… (a session an app hosts, or a server's share link; a code here works too)} → the new session: {session, status: \"connecting\", …}; its file opens in a tab of its own (other files' sessions go on) and `collab.peers` tells when it's live (`sessions` lists it until then, and if it fails)",
            false,
            join
        ),
        spec!(
            "collab.leave",
            "Leave live session",
            "{} (the open file's session; the synced file stays open as a local copy) → {ended}",
            false,
            leave
        ),
        spec!(
            "collab.publish",
            "Publish to server",
            "{server: ws://host:port/?key=… or wss://host/path/?key=… (the server's address with its key; `figcraft-server key` prints it; without a key on a server open to the public), key? (when the address has none), name? (default: the file's name)} → status \"connecting\": the open file is uploaded as a new file and this tab syncs through the server; `collab.peers` then has `published: {server, file, name, edit, view, owner (published without a key: the owner token, shown once), expires (seconds since 1970, or null)}` and `hosting` (the server's operator, terms and idle expiry)",
            false,
            publish
        ),
        spec!(
            "collab.serverInfo",
            "Server info",
            "{server: ws://host:port/ or wss://host/path/} → {open (anyone may publish without the key), operator, terms (a web address), idleDays (files published without the key are deleted after this many days unopened; null: kept), maxFileMb, expires}",
            false,
            server_info
        ),
        spec!(
            "collab.manageFile",
            "Manage published file",
            "{file (its id), server?, owner? (the server's address and the owner token you got when you published the file without the key; each defaults to what is recorded for one of your online files, see `collab.files`), action: links | createLink {access?: view (default) | edit, label?} | revoke {link (a link's id)} | share (others get new links and join codes) | unshare (the links and codes others got are revoked; the file stays yours on the server) | rename {name} | remove (the file is deleted from the server and leaves your online files)} → {links: [{id, access, label, created}]} | {id, access, link (a new share link, shown once)} | {shared, code, viewCode, editLink, viewLink} | {done}",
            false,
            manage_file
        ),
        spec!(
            "collab.report",
            "Report file to server operator",
            "{reason (a few words, at most 500 characters)} → {sent}: the file you joined on a server is reported to its operator; the session's message says when the server kept it (servers older than this app don't take reports)",
            false,
            report
        ),
        spec!(
            "collab.peers",
            "People in this file",
            "{} → the open file's session (each open file has its own or none): {available, canHost (false in the browser: it can only join), session (its id; null: this file isn't in a session), status: off|connecting|live|reconnecting|failed, reason?, role, me, name, color, fileName, pages, link, url, address, network, access: edit|view (view: a server's view link; edits are refused), readOnly (the open file can't be edited now: view only, or being published), published (a file you published: {server, file, name, edit, view}), code (the join code to give others, when the file was shared with `collab.share`; null otherwise or while it is on its way), share ({via: online|local, access, shared (false: a file of yours on a server that nobody else gets into now), editCode, viewCode, editLink, viewLink (what others join with; null when not shared)} or null), follow, syncing, document (index of the synced open document), editing (the text layer you type in: {node, layer, caret, selection}), peers: [{id, name, color, editing (theirs, same shape, or null), presence: {cursor: [page, x, y], selection, viewport: [page, x, y, w, h], editing_text}}], sessions: [{session, document (index of its open document; null for a join on its way), status, reason, role, fileName, link, peers (how many others)}] (every session of the app)}",
            false,
            |s, _| Ok(s.live_summary())
        ),
        spec!("collab.follow", "Follow", "{client? (a peer's id; omit to stop following)} → {follow}", false, follow),
        spec!(
            "collab.removePerson",
            "Remove from session",
            "{client (a peer's id)} → {removed, name}: they are disconnected (they keep their copy) and can't join again from their address while the session runs; only the person hosting, or the owner of a file on a server, can do it",
            false,
            remove_person
        ),
        spec!(
            "collab.cursor",
            "Show cursor at",
            "{x?, y? (page coordinates on the current page; omit both to hide)} → {cursor} (agents; in the app the pointer drives it): your pointer in the open file's session (null when the file isn't in one)",
            false,
            cursor
        ),
        spec!(
            "collab.cursorChat",
            "Cursor chat",
            "{text? (shown next to your cursor for everyone, at most 100 characters; empty closes it; omit to open the chat box in the app)} → {chat} (or {open}); others see it fade after 5 s without changes",
            false,
            cursor_chat
        ),
        spec!(
            "collab.spotlight",
            "Spotlight me",
            "{on? (default: toggle)} → {spotlighting}: everyone in the session follows your view and page until they move theirs",
            false,
            spotlight
        ),
    ]
}
