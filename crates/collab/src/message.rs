//! Messages between clients and the session server, and their two encodings: a compact binary form
//! (a version byte, then postcard) and a JSON form for debugging and agents.

use figcraft_doc::NodeId;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::Error;
use crate::change::Change;

/// The protocol version a client announces in [`Hello`] (2: the welcome says what you may do, and
/// connections can manage a server or use a file's team library).
pub const PROTOCOL: u32 = 2;

/// The first byte of every binary message.
pub const WIRE_VERSION: u8 = 1;

/// The word a client puts in its [`Hello::client_version`] when it reads [`ServerMsg::Signal`]: a
/// server sends signals only to clients that say so (older ones couldn't read the message).
pub const SIGNALS: &str = "signals";

/// The word a client puts in its [`Hello::client_version`] when it reads [`ServerMsg::Hosting`] and
/// [`ServerMsg::Reported`]: a server sends them only to clients that say so.
pub const HOSTING: &str = "hosting";

/// The `client_version` this build announces: its version, then what it reads beyond the protocol.
pub fn client_version() -> String {
    format!("{} {SIGNALS} {HOSTING}", env!("CARGO_PKG_VERSION"))
}

/// A [`Hello::client_version`] that announces [`SIGNALS`].
pub fn reads_signals(client_version: &str) -> bool {
    client_version.split_whitespace().any(|w| w == SIGNALS)
}

/// A [`Hello::client_version`] that announces [`HOSTING`].
pub fn reads_hosting(client_version: &str) -> bool {
    client_version.split_whitespace().any(|w| w == HOSTING)
}

/// Join a file's session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hello {
    pub protocol: u32,
    /// The file's session id (from the invite).
    pub file: String,
    /// The session's secret (from the invite; empty when the session has none).
    pub token: String,
    /// The name shown on this person's cursor.
    pub name: String,
    /// Their colour (`#RRGGBB`).
    pub color: String,
    pub client_version: String,
    /// Coming back after a dropped connection: who this client was and the last sequence number
    /// it applied (`None`: a new participant).
    pub resume: Option<Resume>,
}

/// What a participant may do: edit, or only view (a view link). A viewer gets everyone's changes
/// and presence, and the server refuses every batch it sends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Access {
    #[default]
    Edit,
    View,
}

impl Access {
    /// `"edit"` or `"view"`.
    pub fn id(self) -> &'static str {
        match self {
            Access::Edit => "edit",
            Access::View => "view",
        }
    }

    /// From [`id`](Self::id) (any case, surrounding spaces ignored).
    pub fn parse(s: &str) -> Option<Access> {
        match s.trim().to_ascii_lowercase().as_str() {
            "edit" => Some(Access::Edit),
            "view" => Some(Access::View),
            _ => None,
        }
    }
}

/// What a reconnecting client tells the server in its [`Hello`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resume {
    /// The session it was in ([`ServerMsg::Welcome`]'s `session`).
    pub session: u64,
    /// The client id the server gave this client before.
    pub client_id: u32,
    /// The last sequence number it applied.
    pub seq: u64,
}

/// Someone in the session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Peer {
    pub client_id: u32,
    pub name: String,
    pub color: String,
}

/// Where someone is and what they look at; ephemeral (never stored), latest wins.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Presence {
    /// The page the pointer is on and its position in page coordinates.
    pub cursor: Option<(NodeId, f64, f64)>,
    pub selection: Option<Vec<NodeId>>,
    /// The page shown and the visible rectangle `(x, y, width, height)` in page coordinates.
    pub viewport: Option<(NodeId, f64, f64, f64, f64)>,
    /// The text layer being typed in, with the caret and selection.
    pub editing_text: Option<Box<EditingText>>,
}

/// Someone typing in a text layer: where their caret and selection are (in characters).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditingText {
    pub node: NodeId,
    pub caret: usize,
    /// The selected characters `(start, end)`, if any.
    pub selection: Option<(usize, usize)>,
}

/// Presence beyond [`Presence`]: someone's cursor chat, or their spotlight. Ephemeral like presence
/// (never in the document or the log), latest per kind wins. Kinds a side doesn't know are
/// ignored: a server drops them, a client skips them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signal {
    /// [`Signal::CHAT`], [`Signal::SPOTLIGHT`] or (server to client) [`Signal::READY`].
    pub kind: String,
    /// Chat: the message (empty: closed). Spotlight: `"on"`, or empty when it stopped.
    pub text: String,
}

impl Signal {
    /// The message typed next to someone's pointer.
    pub const CHAT: &str = "chat";
    /// Someone asks everyone to follow their view.
    pub const SPOTLIGHT: &str = "spotlight";
    /// From the server, about the client itself, right after its welcome: signals may be sent.
    pub const READY: &str = "ready";
    /// Longest chat message (characters).
    pub const MAX_CHAT: usize = 100;
    /// Longest signal on the wire before it is cleaned (bytes of kind and text together).
    pub const MAX_BYTES: usize = 1024;

    pub fn chat(text: &str) -> Signal {
        Signal { kind: Signal::CHAT.into(), text: text.into() }
    }

    pub fn spotlight(on: bool) -> Signal {
        Signal { kind: Signal::SPOTLIGHT.into(), text: if on { "on".into() } else { String::new() } }
    }

    pub fn ready() -> Signal {
        Signal { kind: Signal::READY.into(), text: String::new() }
    }

    /// A spotlight that is on.
    pub fn on(&self) -> bool {
        self.text == "on"
    }

    /// Untrusted input made safe to pass on or show: `None` for an unknown kind, a malformed
    /// spotlight or anything oversized; a chat's control characters become spaces and it is cut to
    /// [`MAX_CHAT`](Self::MAX_CHAT) characters (an all-blank one is empty: closed).
    pub fn clean(self) -> Option<Signal> {
        if self.kind.len() + self.text.len() > Signal::MAX_BYTES {
            return None;
        }
        match self.kind.as_str() {
            Signal::CHAT => {
                let text: String = self.text.chars().map(|c| if c.is_control() { ' ' } else { c }).take(Signal::MAX_CHAT).collect();
                Some(Signal::chat(if text.trim().is_empty() { "" } else { &text }))
            }
            Signal::SPOTLIGHT if self.text.is_empty() || self.on() => Some(self),
            Signal::READY if self.text.is_empty() => Some(self),
            _ => None,
        }
    }
}

/// A batch the server accepted: its sequence number, who sent it and their batch id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Accepted {
    pub seq: u64,
    pub author: u32,
    pub batch_id: u64,
    pub changes: Vec<Change>,
}

/// What the server did with one client's batches: the last batch id it processed (accepted or
/// refused; 0: none) and the ids of the latest refused ones (at most [`MAX_REFUSED`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Processed {
    pub last_batch: u64,
    pub refused: Vec<u64>,
}

/// The most refused batch ids [`Processed`] remembers per client.
pub const MAX_REFUSED: usize = 1024;

impl Processed {
    /// Note batch `batch_id` as processed (`refused`: not accepted).
    pub fn note(&mut self, batch_id: u64, refused: bool) {
        self.last_batch = self.last_batch.max(batch_id);
        if refused {
            self.refused.push(batch_id);
            let over = self.refused.len().saturating_sub(MAX_REFUSED);
            self.refused.drain(..over);
        }
    }
}

/// How a [`ServerMsg::Welcome`] brings a client up to date.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CatchUp {
    /// The whole document ([`Replica::snapshot`](crate::Replica::snapshot)): a new participant,
    /// or one whose missed batches are no longer in the server's log.
    Snapshot(Vec<Change>),
    /// The batches a reconnecting client missed, in order.
    Delta(Vec<Accepted>),
}

/// Opens a connection that manages a server's files (instead of [`Hello`]): the server's admin
/// key. Answered with [`AdminReply::Ready`] or `Refused`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manage {
    pub protocol: u32,
    pub key: String,
    pub client_version: String,
}

/// A request on a managing connection; each gets one [`AdminReply`], in order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Admin {
    /// Keep a document as a new file: its name and its snapshot
    /// ([`Replica::snapshot`](crate::Replica::snapshot)). The reply has a new edit link and a new
    /// view link for it.
    Publish { name: String, snapshot: Vec<Change> },
    /// The files kept.
    Files,
    /// A file's links (without their tokens).
    Links { file: String },
    /// A new link to a file.
    CreateLink { file: String, access: Access, label: String },
    /// Revoke a link: it no longer joins, and whoever joined with it is disconnected.
    Revoke { file: String, link: String },
    /// Remove a file and its links (whoever is in it is disconnected).
    Remove { file: String },
    /// Rename a file on the server (its listing; the document inside keeps its own name).
    Rename { file: String, name: String },
    /// The files kept, with what the operator moderates by ([`AdminReply::Listing`]).
    Listing,
    /// The reports people sent about files.
    Reports,
    /// Refuse connections from an address (an IPv6 address: its /64), and disconnect them.
    Block { ip: String },
    /// Accept an address again.
    Unblock { ip: String },
    /// The blocked addresses.
    Blocked,
    /// A join code for the link of `file` whose token this is (the token proves the link and lets
    /// the server give it to whoever types the code): [`AdminReply::Code`]. A link has one code:
    /// asking again replaces it. Servers older than join codes can't read this request.
    Code { file: String, token: String },
    /// On an open hosting connection (no file named in its opening): what became of the files
    /// these `(file id, owner token)` pairs own, at most [`MAX_OWNED`] of them:
    /// [`AdminReply::Owned`]. Servers older than this request can't read it.
    Owned { files: Vec<(String, String)> },
    /// Remove client `client` from `file`'s session: they are disconnected and their address
    /// can't join the file again while its session lasts. With the server key, or on an open
    /// hosting connection with `file`'s owner token. Servers older than this request can't read it.
    RemovePeer { file: String, client: u32 },
}

/// The most files one [`Admin::Owned`] asks about.
pub const MAX_OWNED: usize = 200;

/// A file its owner asked about ([`Admin::Owned`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedFile {
    /// Its name on the server.
    pub name: String,
    /// The ids of its share links (the owner's own and the ones made for others).
    pub links: Vec<String>,
    /// When it is deleted if nobody opens it (seconds since 1970).
    pub expires: Option<u64>,
}

/// A file a server keeps.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileInfo {
    /// Its id (in links: `file=<id>`).
    pub id: String,
    pub name: String,
    /// Seconds since 1970 (UTC).
    pub created: u64,
    pub updated: u64,
    /// The sequence number of its last accepted batch.
    pub seq: u64,
    /// Bytes on disk.
    pub bytes: u64,
    /// People in it now.
    pub people: u32,
}

/// A share link (the server keeps only a hash of its token).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LinkInfo {
    /// Its id (to revoke it by; not a secret).
    pub id: String,
    pub file: String,
    pub access: Access,
    pub label: String,
    /// Seconds since 1970 (UTC).
    pub created: u64,
}

/// A link just made, with its token: shown this once.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NewLink {
    pub link: LinkInfo,
    pub token: String,
}

/// The answer to an [`Admin`] request (the first one on a connection answers [`Manage`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AdminReply {
    /// The key is right: requests may follow.
    Ready,
    Published {
        file: FileInfo,
        edit: NewLink,
        view: NewLink,
    },
    Files(Vec<FileInfo>),
    Links(Vec<LinkInfo>),
    Link(NewLink),
    /// Done (a revoke or a removal).
    Done,
    /// The request couldn't be done (the connection stays open).
    Failed {
        reason: String,
    },
    /// A file published on an open hosting connection: its first two links and its owner token
    /// (tokens shown once), and when it is deleted if nobody opens it (seconds since 1970).
    Hosted {
        file: FileInfo,
        edit: NewLink,
        view: NewLink,
        owner: String,
        expires: Option<u64>,
    },
    Listing(Vec<FileRow>),
    Reports(Vec<ReportInfo>),
    Blocked(Vec<String>),
    /// A link's new join code (shown this once; the server keeps only a hash).
    Code {
        code: String,
    },
    /// The answer to [`Admin::Owned`], one per pair asked about, in order. `None`: no file of that
    /// id is managed by that token here (removed, expired, or never there).
    Owned(Vec<Option<OwnedFile>>),
}

/// A file with what its operator moderates by.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileRow {
    pub info: FileInfo,
    /// When someone last opened it (seconds since 1970; 0: never since it was published).
    pub opened: u64,
    /// When it is deleted if nobody opens it (files published without the server key).
    pub expires: Option<u64>,
    /// Published without the server key (it has an owner token), from this address.
    pub open: bool,
    pub ip: String,
    /// Reports kept about it.
    pub reports: u32,
}

/// A report someone in a file sent its server's operator.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReportInfo {
    /// Seconds since 1970.
    pub at: u64,
    pub file: String,
    /// The file's name then.
    pub name: String,
    /// The link the reporter joined with, and their address.
    pub link: String,
    pub ip: String,
    pub reason: String,
}

/// Opens a connection to a server's open hosting (instead of [`Hello`]): anyone may ask about the
/// server and, when it is open to the public, publish a file without the server key; with a file's
/// owner token (`file` and `owner` set) the connection also manages that file. Answered with
/// [`ServerMsg::Hosting`] or `Refused`; requests are [`Admin`] messages, each answered with one
/// [`AdminReply`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpenHosting {
    pub protocol: u32,
    pub client_version: String,
    pub file: String,
    pub owner: String,
}

/// Asks a server which file a join code opens (the first message on a connection, instead of
/// [`Hello`]). Answered with [`ServerMsg::Resolved`], then the connection closes; a wrong code, too
/// many wrong ones from an address, or a server that hosts no files is `Refused`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Resolve {
    pub protocol: u32,
    pub client_version: String,
    /// The code's symbols ([`crate::code::normalize`]d).
    pub code: String,
}

/// What a server says about its hosting: whether anyone may publish, who runs it and its terms,
/// how long files published without the key are kept unopened, and the largest file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostInfo {
    /// Anyone may publish without the server key.
    pub open: bool,
    /// How to reach the operator (an email address or a web page), and the terms of use (a URL).
    pub operator: String,
    pub terms: String,
    /// Files published without the key are deleted after this many seconds unopened (0: kept).
    pub idle_secs: u64,
    pub max_file_mb: u64,
    /// This file's expiry (seconds since 1970), when it is one that expires.
    pub expires: Option<u64>,
}

/// Opens a connection that uses a file's team library (instead of [`Hello`]), with one of the
/// file's share links. Answered with [`LibraryReply::Ready`] or `Refused`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpenLibrary {
    pub protocol: u32,
    pub file: String,
    pub token: String,
    pub client_version: String,
}

/// How one file uses a library, as its app reports it: the file's name and its instances of each
/// library component (by the component's id in the library).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub file: String,
    pub components: Vec<(NodeId, u32)>,
}

/// A file's last usage report, with when it came (seconds since 1970, UTC).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileUsage {
    pub usage: Usage,
    pub updated: u64,
}

/// A request on a library connection; each gets one [`LibraryReply`], in order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LibraryReq {
    /// The file's library, unless the server's version is `have`; `usage` reports how the asking
    /// file uses it.
    Fetch { have: Option<u64>, usage: Option<Usage> },
    /// Keep `archive` (a `.figcraft` archive holding the published library) as the file's library.
    /// Needs an edit link; the library must be the one kept (same id) at a newer version.
    Publish { archive: Vec<u8> },
    /// The usage reports of the files that use the library.
    Usage,
}

/// The answer to a [`LibraryReq`] (the first one on a connection answers [`OpenLibrary`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LibraryReply {
    /// The link is right: requests may follow, with this access.
    Ready {
        access: Access,
    },
    /// The library (a `.figcraft` archive holding it) at `version`.
    Library {
        version: u64,
        archive: Vec<u8>,
    },
    /// The server's version is the one asked about.
    Unchanged {
        version: u64,
    },
    /// The file has no library on the server.
    Empty,
    Published {
        version: u64,
    },
    Usage(Vec<FileUsage>),
    /// The request couldn't be done (the connection stays open).
    Failed {
        reason: String,
    },
}

/// Opens a connection to a server's plugin directory (instead of [`Hello`]). Anyone who can reach
/// the server may list and fetch plugins; with the server's key (`key` not empty) the connection
/// may also publish and unpublish them. Answered with [`PluginReply::Ready`] or `Refused`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpenPlugins {
    pub protocol: u32,
    /// The server's key, or empty to only browse.
    pub key: String,
    pub client_version: String,
}

/// A plugin a directory keeps: what its manifest says (the author as they typed it) and the bundle
/// that holds it (its size and SHA-256, which a download must match).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PluginEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    /// The plugin API it was written for.
    pub api: u32,
    /// The permissions it asks for: `read` or `write` access to the document, and the commands
    /// it may call (empty: any its access allows).
    pub access: String,
    pub commands: Vec<String>,
    /// It draws a widget.
    pub widget: bool,
    /// The bundle's size in bytes and its SHA-256 (hex).
    pub bytes: u64,
    pub sha256: String,
    /// When it was published (seconds since 1970, UTC).
    pub published: u64,
    /// In listings: every version the directory keeps of it, newest first.
    pub versions: Vec<String>,
}

/// A request on a plugin directory connection; each gets one [`PluginReply`], in order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PluginReq {
    /// The latest version of each plugin whose id, name, description or author contains `query`
    /// (any case; empty: all).
    List { query: String },
    /// A plugin's bundle: version `version`, or its latest.
    Fetch { id: String, version: Option<String> },
    /// Keep a bundle (`figcraft_plugin::bundle`) as a new version of its plugin. Needs the key.
    Publish { bundle: Vec<u8> },
    /// Remove version `version` of a plugin, or all of them. Needs the key.
    Unpublish { id: String, version: Option<String> },
}

/// The answer to a [`PluginReq`] (the first one on a connection answers [`OpenPlugins`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PluginReply {
    /// Requests may follow; `publisher`: the key was given, so publishing is allowed.
    Ready {
        publisher: bool,
    },
    List(Vec<PluginEntry>),
    /// A plugin's entry and its bundle's bytes (their SHA-256 is the entry's).
    Bundle {
        entry: PluginEntry,
        bundle: Vec<u8>,
    },
    Published(PluginEntry),
    /// Unpublished: the versions removed.
    Removed(Vec<String>),
    /// The request couldn't be done (the connection stays open).
    Failed {
        reason: String,
    },
}

/// Client → server.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClientMsg {
    /// Join (the first message on a connection).
    Hello(Hello),
    /// One user action's changes, made on top of server state `base_seq`.
    Changes {
        base_seq: u64,
        batch_id: u64,
        changes: Vec<Change>,
    },
    Presence(Presence),
    /// The highest sequence number applied.
    Ack {
        seq: u64,
    },
    Ping,
    /// Manage the server's files (the first message on a connection, instead of `Hello`).
    Manage(Manage),
    /// A request on a managing connection.
    Admin(Admin),
    /// Use a file's team library (the first message on a connection, instead of `Hello`).
    OpenLibrary(OpenLibrary),
    /// A request on a library connection.
    Library(LibraryReq),
    /// Your cursor chat or spotlight (only to a server that sent [`Signal::READY`]).
    Signal(Signal),
    /// Use the server's plugin directory (the first message on a connection, instead of `Hello`).
    OpenPlugins(OpenPlugins),
    /// A request on a plugin directory connection.
    Plugins(PluginReq),
    /// Ask about the server's hosting, publish without the key or manage a file with its owner
    /// token (the first message on a connection, instead of `Hello`).
    OpenHosting(OpenHosting),
    /// Report the file you are in to the server's operator (only to a server that sent
    /// [`ServerMsg::Hosting`]): why, in a few words.
    Report {
        reason: String,
    },
    /// Ask which file a join code opens (the first message on a connection, instead of `Hello`).
    Resolve(Resolve),
}

/// Server → client.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServerMsg {
    /// Joined: the session's identity (a server that restarted without its saved state is a new
    /// session), this connection's client id, the current sequence number, the document (or the
    /// batches missed since a reconnecting client's last sequence number), what the server did
    /// with this client's batches so far, who else is here and what this client may do.
    Welcome {
        session: u64,
        client_id: u32,
        seq: u64,
        catch_up: CatchUp,
        processed: Processed,
        peers: Vec<Peer>,
        access: Access,
    },
    /// Accepted changes, in order, to everyone (the author's own come back as confirmation).
    Changes(Accepted),
    /// A batch was refused (nothing of it applies): why, and the current values of what it
    /// touched.
    Reject {
        batch_id: u64,
        reason: String,
        fixes: Vec<Change>,
    },
    /// Someone's presence.
    Presence {
        client_id: u32,
        presence: Presence,
    },
    PeerJoined(Peer),
    PeerLeft {
        client_id: u32,
    },
    /// The connection is refused (bad protocol version or token, session full, a message out of
    /// place).
    Refused {
        reason: String,
    },
    Pong,
    /// The answer to a managing connection's request.
    Admin(Box<AdminReply>),
    /// The answer to a library connection's request.
    Library(Box<LibraryReply>),
    /// Someone's cursor chat or spotlight; sent only to clients whose hello announced
    /// [`SIGNALS`].
    Signal {
        client_id: u32,
        signal: Signal,
    },
    /// The answer to a plugin directory connection's request.
    Plugins(Box<PluginReply>),
    /// The server's hosting: the answer to [`OpenHosting`], and after a welcome to clients whose
    /// hello announced [`HOSTING`] (with the file's expiry).
    Hosting(Box<HostInfo>),
    /// The answer to a [`ClientMsg::Report`]: kept, or why not.
    Reported {
        error: Option<String>,
    },
    /// The answer to [`ClientMsg::Resolve`]: the file the code opens and its link's token (what a
    /// `Hello` joins with).
    Resolved {
        file: String,
        token: String,
    },
}

/// `YYYY-MM-DD HH:MM UTC` for seconds since 1970 (how servers and apps show the times in these
/// messages).
pub fn utc_date(secs: u64) -> String {
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let (h, m) = ((secs % 86_400) / 3600, (secs % 3600) / 60);
    // Days to a civil date (proleptic Gregorian).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(mo <= 2);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02} UTC")
}

/// The binary form of `msg`: [`WIRE_VERSION`], then postcard.
pub fn encode<T: Serialize>(msg: &T) -> Result<Vec<u8>, Error> {
    let body = postcard::to_allocvec(msg).map_err(|e| Error::Encode(e.to_string()))?;
    let mut out = Vec::with_capacity(body.len() + 1);
    out.push(WIRE_VERSION);
    out.extend(body);
    Ok(out)
}

/// Read a binary message of at most `max_bytes` (untrusted input: never panics, allocates at most
/// about the message's size, and refuses trailing bytes).
pub fn decode<T: DeserializeOwned>(bytes: &[u8], max_bytes: usize) -> Result<T, Error> {
    if bytes.len() > max_bytes {
        return Err(Error::Decode(format!("message of {} bytes is over the {max_bytes}-byte limit", bytes.len())));
    }
    let Some((&version, body)) = bytes.split_first() else { return Err(Error::Decode("empty message".into())) };
    if version != WIRE_VERSION {
        return Err(Error::Decode(format!("wire version {version}, expected {WIRE_VERSION}")));
    }
    let (msg, rest) = postcard::take_from_bytes::<T>(body).map_err(|e| Error::Decode(e.to_string()))?;
    if !rest.is_empty() {
        return Err(Error::Decode(format!("{} bytes after the message", rest.len())));
    }
    Ok(msg)
}

/// The JSON form of `msg` (`{"<type>": {…}}`).
pub fn encode_json<T: Serialize>(msg: &T) -> Result<String, Error> {
    serde_json::to_string(msg).map_err(|e| Error::Encode(e.to_string()))
}

/// Read a JSON message of at most `max_bytes`.
pub fn decode_json<T: DeserializeOwned>(text: &str, max_bytes: usize) -> Result<T, Error> {
    if text.len() > max_bytes {
        return Err(Error::Decode(format!("message of {} bytes is over the {max_bytes}-byte limit", text.len())));
    }
    serde_json::from_str(text).map_err(|e| Error::Decode(e.to_string()))
}
