use super::{
    Access, Admin, AdminReply, Arc, ClientMsg, Code, Document, HostInfo, IpAddr, LibraryReply, LibraryReq, NO_TEAM_LIBRARIES, Peer, PluginReply,
    PluginReq, Presence, Resume, Serialize, ServerMsg, Signal, code,
};

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

pub use figcraft_collab::message::MAX_PEERS;

/// Longest name shown to others (characters).
pub const MAX_NAME: usize = Signal::MAX_NAME;

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
    /// What you may do: edit, only view (a server's view link), or view and comment (its comment
    /// link).
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

/// What others join a shared file with: to edit it and, on a server, to only view it or to view
/// and comment on it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Shared {
    pub edit: SharedLink,
    pub view: Option<SharedLink>,
    /// `None`: a server older than comment links, or a share made before them.
    pub comment: Option<SharedLink>,
}

impl Shared {
    /// The way in that gives `access`, if there is one.
    pub fn of(&self, access: Access) -> Option<&SharedLink> {
        match access {
            Access::Edit => Some(&self.edit),
            Access::View => self.view.as_ref(),
            Access::Comment => self.comment.as_ref(),
        }
    }

    /// Every way in (what stopping revokes).
    pub fn all(&self) -> impl Iterator<Item = &SharedLink> {
        std::iter::once(&self.edit).chain(&self.view).chain(&self.comment)
    }
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
        self.links.as_ref()?.of(self.access)
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
}

impl NetRequest {
    /// What a session without the network says.
    pub(crate) fn unavailable(&self) -> &'static str {
        match self {
            NetRequest::Plugins { .. } => crate::plugin_directory::NO_DIRECTORY,
            NetRequest::Library { .. } | NetRequest::Upload(_) => NO_TEAM_LIBRARIES,
            NetRequest::Hosting { .. } | NetRequest::Share { .. } => NO_HOSTING,
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
    /// Send `req` and wait for the answer (through the methods above).
    fn request(&mut self, req: NetRequest) -> NetReply {
        match req {
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
