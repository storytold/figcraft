//! Messages between clients and the session server, and their two encodings: a compact binary form
//! (a version byte, then postcard) and a JSON form for debugging and agents.

use std::io::{Read, Write};

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

/// The word a client puts in its [`Hello::client_version`] (or its managing connection's) when it
/// reads [`Access::Comment`]: older ones get a comment link as a view link.
pub const COMMENTS: &str = "comments";

/// The word a client puts in its [`Hello::client_version`] when it reads [`CatchUp::Packed`]: a
/// server compresses the document in a welcome only for clients that say so (older ones couldn't
/// read it).
pub const PACKED: &str = "packed";

/// The `client_version` this build announces: its version, then what it reads beyond the protocol.
pub fn client_version() -> String {
    format!("{} {SIGNALS} {HOSTING} {COMMENTS} {PACKED}", env!("CARGO_PKG_VERSION"))
}

/// Whether `client_version` announces `word`.
fn announces(client_version: &str, word: &str) -> bool {
    client_version.split_whitespace().any(|w| w == word)
}

/// A [`Hello::client_version`] that announces [`SIGNALS`].
pub fn reads_signals(client_version: &str) -> bool {
    announces(client_version, SIGNALS)
}

/// A [`Hello::client_version`] that announces [`HOSTING`].
pub fn reads_hosting(client_version: &str) -> bool {
    announces(client_version, HOSTING)
}

/// A [`Hello::client_version`] that announces [`COMMENTS`].
pub fn reads_comments(client_version: &str) -> bool {
    announces(client_version, COMMENTS)
}

/// A [`Hello::client_version`] that announces [`PACKED`].
pub fn reads_packed(client_version: &str) -> bool {
    announces(client_version, PACKED)
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

/// What a participant may do: edit, only view (a view link), or view and comment (a comment
/// link). A viewer gets everyone's changes and presence, and the server refuses every batch it
/// sends; a commenter's batches are refused unless they only change comment threads. A level
/// this build doesn't know reads as [`View`](Access::View), and a client that doesn't announce
/// [`COMMENTS`] is welcomed with a comment link as a viewer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Access {
    #[default]
    Edit,
    View,
    /// New levels go at the end: the binary form numbers them.
    Comment,
}

/// Reads what [`Access`]'s derived `Serialize` writes (a variant's number in the binary form, its
/// name in JSON); an unknown one reads as [`Access::View`].
impl<'de> Deserialize<'de> for Access {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::{EnumAccess, VariantAccess, Visitor};
        struct Level(Access);
        impl<'de> Deserialize<'de> for Level {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                struct Id;
                impl Visitor<'_> for Id {
                    type Value = Level;
                    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        f.write_str("an access level")
                    }
                    fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Level, E> {
                        Ok(Level(match v {
                            0 => Access::Edit,
                            2 => Access::Comment,
                            _ => Access::View,
                        }))
                    }
                    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Level, E> {
                        Ok(Level(Access::parse(v).unwrap_or(Access::View)))
                    }
                }
                d.deserialize_identifier(Id)
            }
        }
        struct Unit;
        impl<'de> Visitor<'de> for Unit {
            type Value = Access;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an access level")
            }
            fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> Result<Access, A::Error> {
                let (Level(a), v) = data.variant()?;
                v.unit_variant()?;
                Ok(a)
            }
        }
        d.deserialize_enum("Access", &["edit", "view", "comment"], Unit)
    }
}

impl Access {
    /// `"edit"`, `"view"` or `"comment"`.
    pub fn id(self) -> &'static str {
        match self {
            Access::Edit => "edit",
            Access::View => "view",
            Access::Comment => "comment",
        }
    }

    /// From [`id`](Self::id) (any case, surrounding spaces ignored).
    pub fn parse(s: &str) -> Option<Access> {
        match s.trim().to_ascii_lowercase().as_str() {
            "edit" => Some(Access::Edit),
            "view" => Some(Access::View),
            "comment" => Some(Access::Comment),
            _ => None,
        }
    }

    /// This access as a client can read it: a comment link only views for one that doesn't
    /// announce [`COMMENTS`] (`reads_comments` false).
    pub fn for_reader(self, reads_comments: bool) -> Access {
        if self == Access::Comment && !reads_comments { Access::View } else { self }
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
    /// The secret the server gave with this client id ([`ServerMsg::Welcome`]'s `secret`): only
    /// the client the id was given to can resume it (0: none).
    #[serde(default)]
    pub secret: u64,
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
    /// [`Signal::CHAT`], [`Signal::SPOTLIGHT`], (client to server) [`Signal::NAME`] or (server to
    /// client) [`Signal::READY`].
    pub kind: String,
    /// Chat: the message (empty: closed). Spotlight: `"on"`, or empty when it stopped. Name: the
    /// new name.
    pub text: String,
}

impl Signal {
    /// The message typed next to someone's pointer.
    pub const CHAT: &str = "chat";
    /// Someone asks everyone to follow their view.
    pub const SPOTLIGHT: &str = "spotlight";
    /// From the server, about the client itself, right after its welcome: signals may be sent.
    pub const READY: &str = "ready";
    /// To the server: the client was renamed. The server keeps the new name and tells everyone,
    /// the client too, with its [`ServerMsg::PeerJoined`] (it isn't passed on as a signal).
    pub const NAME: &str = "name";
    /// Longest name shown to others (characters).
    pub const MAX_NAME: usize = 64;
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

    pub fn name(name: &str) -> Signal {
        Signal { kind: Signal::NAME.into(), text: name.into() }
    }

    /// A spotlight that is on.
    pub fn on(&self) -> bool {
        self.text == "on"
    }

    /// Untrusted input made safe to pass on or show: `None` for an unknown kind, a malformed
    /// spotlight, a blank name or anything oversized; a chat's control characters become spaces and
    /// it is cut to [`MAX_CHAT`](Self::MAX_CHAT) characters (an all-blank one is empty: closed); a
    /// name is trimmed, loses its control characters and is cut to [`MAX_NAME`](Self::MAX_NAME).
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
            Signal::NAME => {
                let name: String = self.text.trim().chars().filter(|c| !c.is_control()).take(Signal::MAX_NAME).collect();
                let name = name.trim_end();
                (!name.is_empty()).then(|| Signal::name(name))
            }
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
    /// A [`Snapshot`](CatchUp::Snapshot) compressed ([`pack`]), for a client whose hello announced
    /// [`PACKED`]: a document's changes shrink several times over. The client reads it with
    /// [`unpack`] before anything else sees the welcome.
    Packed(Vec<u8>),
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
    /// On an open hosting connection, or with the server key: the files shared from this server now
    /// (a link of theirs has a join code), when its operator turned on the team list:
    /// [`AdminReply::Team`]; they are joined with [`ResolveTeam`]. Servers older than this request
    /// can't read it.
    Team,
}

/// The most files an [`AdminReply::Team`] lists (the most recently changed ones).
pub const MAX_TEAM: usize = 500;

/// A file of a server's team list ([`AdminReply::Team`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TeamFile {
    pub file: FileInfo,
    /// What its team link lets people do.
    pub access: Access,
}

/// The most files one [`Admin::Owned`] asks about.
pub const MAX_OWNED: usize = 200;

/// Most other people a client keeps track of in one session (their names, presence and chat).
/// A server holds far fewer (256 by default, a larger `max_people` lowered to this): past it, someone new is
/// left out, so a hostile server can't grow a client's memory without bound.
pub const MAX_PEERS: usize = 4_096;

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

impl AdminReply {
    /// This reply as a managing client can read it ([`Access::for_reader`]).
    pub fn for_reader(mut self, reads_comments: bool) -> AdminReply {
        match &mut self {
            AdminReply::Links(links) => links.iter_mut().for_each(|l| l.access = l.access.for_reader(reads_comments)),
            AdminReply::Link(l) => l.link.access = l.link.access.for_reader(reads_comments),
            AdminReply::Team { files, .. } => files.iter_mut().for_each(|f| f.access = f.access.for_reader(reads_comments)),
            _ => {}
        }
        self
    }
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
    /// The answer to [`Admin::Team`]: where the server's browser build is served (`web`, empty when
    /// the operator didn't say) and the files shared now, most recently changed first.
    Team {
        web: String,
        files: Vec<TeamFile>,
    },
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

/// Asks a server for the link of a file on its team list (the first message on a connection,
/// instead of [`Hello`]). Answered with [`ServerMsg::Resolved`], then the connection closes; a
/// server without a team list, or a file that isn't shared now, is `Refused`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResolveTeam {
    pub protocol: u32,
    pub client_version: String,
    /// The file's id.
    pub file: String,
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

/// How many instances of each library component one file has detached, as its app reports it:
/// the file's name and the counts (by the component's id in the library), each file's own total.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Detached {
    pub file: String,
    pub components: Vec<(NodeId, u32)>,
}

/// A file's last report of its detaches, with when it came (seconds since 1970, UTC).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDetached {
    pub detached: Detached,
    pub updated: u64,
}

/// What happened to one library component in one week (the week starting `week`, seconds since
/// 1970 UTC, a Monday at 00:00), over every file that reported it: instances inserted (placed,
/// pasted or duplicated) and detached, counted when the server first heard of them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeekActions {
    pub week: u64,
    pub component: NodeId,
    pub insertions: u32,
    pub detachments: u32,
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
    // Variants below are newer: a server from before them can't read them (it refuses the
    // request), and the ones above keep their encoding.
    /// How the asking file has detached the library's components (answered with
    /// [`LibraryReply::Noted`]).
    ReportDetached(Detached),
    /// The detach reports of the files that use the library.
    Detached,
    /// How many instances of each library component the asking file has inserted (a
    /// [`Detached`]-shaped report of its totals; answered with [`LibraryReply::Noted`]).
    ReportInserted(Detached),
    /// The library's components' insertions and detachments week by week (answered with
    /// [`LibraryReply::Actions`]).
    Actions,
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
    // Newer replies, only ever sent to the newer requests above.
    /// A report was kept.
    Noted,
    Detached(Vec<FileDetached>),
    Actions(Vec<WeekActions>),
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
    /// Ask for the link of a file on the server's team list (the first message on a connection,
    /// instead of `Hello`).
    ResolveTeam(ResolveTeam),
}

/// Server → client.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServerMsg {
    /// Joined: the session's identity (a server that restarted without its saved state is a new
    /// session), this connection's client id, the current sequence number, the document (or the
    /// batches missed since a reconnecting client's last sequence number), what the server did
    /// with this client's batches so far, who else is here, what this client may do and the
    /// secret it resumes its id with ([`Resume::secret`]; only this client is told).
    Welcome {
        session: u64,
        client_id: u32,
        seq: u64,
        catch_up: CatchUp,
        processed: Processed,
        peers: Vec<Peer>,
        access: Access,
        #[serde(default)]
        secret: u64,
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
    /// The answer to [`ClientMsg::Resolve`] and [`ClientMsg::ResolveTeam`]: the file and its link's
    /// token (what a `Hello` joins with).
    Resolved {
        file: String,
        token: String,
    },
}

/// (days since 1970-01-01, seconds-of-day, nanos) → (proleptic Gregorian year, the same wall-clock
/// instant moved into chrono's range by whole 400-year cycles). The calendar repeats every 400
/// years (146_097 days), so month, day and time of day come from chrono and only the year is
/// carried, which keeps the result exact for any `i64` day count. This is not hand-rolled calendar
/// math: chrono still does the date, the cycle count just shifts the year.
fn utc_parts(days: i64, sod: u32, nanos: u32) -> (i64, chrono::NaiveDateTime) {
    use chrono::Datelike;
    let (cycles, rest) = (days.div_euclid(146_097), days.rem_euclid(146_097));
    // `rest + 719_163` (1970-01-01 is CE day 719_163) is always a valid CE day, and `sod` < 86_400.
    let date = i32::try_from(rest + 719_163).ok().and_then(chrono::NaiveDate::from_num_days_from_ce_opt);
    let time = chrono::NaiveTime::from_num_seconds_from_midnight_opt(sod.min(86_399), nanos.min(999_999_999));
    match (date, time) {
        (Some(d), Some(t)) => {
            let y = i128::from(d.year()) + i128::from(cycles) * 400;
            (i64::try_from(y).unwrap_or(if y < 0 { i64::MIN } else { i64::MAX }), d.and_time(t))
        }
        _ => (1970, chrono::NaiveDateTime::default()), // unreachable
    }
}

/// `YYYY-MM-DD HH:MM UTC` for seconds since 1970 (how servers and apps show the times in these
/// messages). Years past chrono's own range (262143) keep converting: the instant is moved into
/// range by whole 400-year cycles and the real year restored, so every `u64` reads as the date the
/// old hand-rolled conversion produced (e.g. `u64::MAX` → `584554051223-11-09 07:00 UTC`).
pub fn utc_date(secs: u64) -> String {
    // The day count of any `u64` seconds fits in `i64` (`u64::MAX / 86_400` ≈ 2.1e14), and the
    // seconds-of-day is < 86_400, so the conversion is always exact (no epoch fallback). Splitting
    // with `u64` arithmetic (as the old code did) avoids the `i64` seconds overflow near `u64::MAX`.
    let (days, sod) = (i64::try_from(secs / 86_400).unwrap_or(i64::MAX), (secs % 86_400) as u32);
    let (y, t) = utc_parts(days, sod, 0);
    // `{y:04}` rather than `%Y`, which writes a `+` before years past 9999 (the old code did not).
    format!("{y:04}-{}", t.format("%m-%d %H:%M UTC"))
}

/// The binary form of `msg`: [`WIRE_VERSION`], then postcard.
pub fn encode<T: Serialize>(msg: &T) -> Result<Vec<u8>, Error> {
    encode_into(msg, Vec::new())
}

/// Append the binary form of `msg` to an owned prefix, without a second payload-sized buffer.
pub fn encode_into<T: Serialize>(msg: &T, mut prefix: Vec<u8>) -> Result<Vec<u8>, Error> {
    prefix.push(WIRE_VERSION);
    postcard::to_extend(msg, prefix).map_err(|e| Error::Encode(e.to_string()))
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
    from_postcard(body)
}

/// One postcard value that fills `body` (no trailing bytes).
fn from_postcard<T: DeserializeOwned>(body: &[u8]) -> Result<T, Error> {
    let (msg, rest) = postcard::take_from_bytes::<T>(body).map_err(|e| Error::Decode(e.to_string()))?;
    if !rest.is_empty() {
        return Err(Error::Decode(format!("{} bytes after the message", rest.len())));
    }
    Ok(msg)
}

/// Compress the document in a welcome ([`CatchUp::Packed`]), for a client that announced
/// [`PACKED`]. Anything else, a catch-up by the missed batches too, is left as it is. Deflate at
/// its fastest level: the hub packs on its own thread, and a document's changes still shrink
/// about eightfold.
pub fn pack(msg: &mut ServerMsg) -> Result<(), Error> {
    if let ServerMsg::Welcome { catch_up, .. } = msg
        && let CatchUp::Snapshot(changes) = catch_up
    {
        let raw = postcard::to_allocvec(changes).map_err(|e| Error::Encode(e.to_string()))?;
        let mut z = flate2::write::DeflateEncoder::new(Vec::with_capacity(raw.len() / 4), flate2::Compression::fast());
        z.write_all(&raw).map_err(|e| Error::Encode(e.to_string()))?;
        *catch_up = CatchUp::Packed(z.finish().map_err(|e| Error::Encode(e.to_string()))?);
    }
    Ok(())
}

/// Undo [`pack`]: a welcome's packed document back as its changes, refused once it would grow
/// past `max_bytes` (untrusted input: the limit an unpacked welcome would have had to fit).
/// Anything else is left as it is.
pub fn unpack(msg: &mut ServerMsg, max_bytes: usize) -> Result<(), Error> {
    if let ServerMsg::Welcome { catch_up, .. } = msg
        && let CatchUp::Packed(packed) = catch_up
    {
        let mut raw = Vec::new();
        let limit = u64::try_from(max_bytes).unwrap_or(u64::MAX).saturating_add(1);
        flate2::read::DeflateDecoder::new(packed.as_slice())
            .take(limit)
            .read_to_end(&mut raw)
            .map_err(|e| Error::Decode(format!("packed document: {e}")))?;
        if raw.len() > max_bytes {
            return Err(Error::TooLarge(max_bytes));
        }
        *catch_up = CatchUp::Snapshot(from_postcard(&raw)?);
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `utc_date` keeps the exact `YYYY-MM-DD HH:MM UTC` output the hand-rolled civil-from-days
    /// code produced (leap days and all).
    #[test]
    fn utc_date_formats_seconds_since_1970() {
        assert_eq!(utc_date(0), "1970-01-01 00:00 UTC");
        assert_eq!(utc_date(1_000_000), "1970-01-12 13:46 UTC");
        assert_eq!(utc_date(951_782_400), "2000-02-29 00:00 UTC");
        assert_eq!(utc_date(1_709_164_800), "2024-02-29 00:00 UTC");
        assert_eq!(utc_date(1_791_208_980), "2026-10-05 14:03 UTC");
        // A year past 9999 keeps the plain zero-padded year (no `+`), as the old `{y:04}` did.
        assert_eq!(utc_date(253_402_300_800), "10000-01-01 00:00 UTC");
        // Past chrono's own year range (262143) the instant is moved into range by whole 400-year
        // cycles and the real year restored, so it keeps the old hand-rolled output instead of
        // falling back to the epoch.
        assert_eq!(utc_date(u64::MAX), "584554051223-11-09 07:00 UTC");
    }
}
