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

use std::collections::{BTreeMap, BTreeSet};
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

mod events;
mod host;
mod join;
mod sync;
mod types;

use host::unavailable;
pub use host::{MAX_REPORT, hosting_json};
pub(crate) use host::{clean_shared, share_answer};
pub(crate) use join::start_join;
pub use join::{can_remove_people, specs};
pub use types::{
    Bind, DEFAULT_SERVER, Event, HostOptions, LiveLink, MAX_NAME, MAX_PEERS, MAX_SELECTION, NO_HOSTING, NO_SESSION, NetReply, NetRequest, ONLINE_OFF,
    PEOPLE, PageInfo, PublishOptions, Published, Role, SessionId, SessionInfo, Share, Shared, SharedLink, Status, TOO_LARGE_TO_JOIN, Via,
    join_target, parse_color,
};

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
    /// The branches of the session's file made or last updated from main in this session: every
    /// layer of their base was in it (see [`crate::branches`]).
    branch_bases: BTreeSet<String>,
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
    /// The recorded online files or your team libraries changed since the app last saved the
    /// preferences (it saves them at once: a crash must not lose an owner token).
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

/// Initials for an avatar ("Ada Lovelace" → "AL"; "Claude (agent)" → "C": words that start with a
/// letter or digit, when there are any; "?" for none).
pub fn initials(name: &str) -> String {
    let firsts = |alnum: bool| -> String {
        name.split_whitespace().filter_map(|w| w.chars().next()).filter(|c| !alnum || c.is_alphanumeric()).take(2).collect()
    };
    let mut out = firsts(true);
    if out.is_empty() {
        out = firsts(false);
    }
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
/// Why an edit other than to comment threads of a server file you joined with a comment link is
/// refused.
pub const COMMENT_ONLY: &str = "Can comment: the link you joined with lets you see this file and comment on it, not change it";

/// Why a session document can't be edited with `access` (`None`: it can).
pub(crate) fn limited_by(access: Access) -> Option<&'static str> {
    match access {
        Access::Edit => None,
        Access::View => Some(VIEW_ONLY),
        Access::Comment => Some(COMMENT_ONLY),
    }
}

/// Whether `tool` is one a file you can't edit now leaves you (`why`: what
/// [`Session::live_read_only`] says): moving around and, with a comment link, commenting.
pub fn tool_left(why: &str, tool: ToolId) -> bool {
    matches!(tool, ToolId::Move | ToolId::Hand) || (why == COMMENT_ONLY && tool == ToolId::Comment)
}
/// Why an edit of a file on its way to a server is refused.
pub const PUBLISHING: &str = "The file is being published: you can edit it again once it is on the server";

/// A gesture, version preview or live text preview shows a document that isn't the edit to
/// sync yet.
fn busy(ds: &DocState) -> bool {
    ds.interaction.is_some() || ds.preview.is_some() || ds.text_preview.is_some()
}
