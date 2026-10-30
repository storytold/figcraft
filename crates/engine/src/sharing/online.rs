//! Your online files (milestone M9): the files you put on a server stay yours after their tab
//! closes or the app quits.
//!
//! Publishing or sharing a file online without the server's key makes you its owner: the server
//! hands out an owner token once. The app records each such file with the preferences
//! ([`Prefs::online_files`](crate::Prefs)), never in a document: where it is, its name, the owner
//! token, your own links to it and what others join with while it is shared. The token is a
//! password: it is stored on this computer only, isn't logged, and no command result or summary
//! carries it (they say `hasOwnerToken: true`); `Debug` leaves the secrets out.
//!
//! `collab.files` lists them (asking each server once what became of them, when told to),
//! `collab.openFile` joins one again as its owner, `collab.forgetFile` drops one from the list
//! (the file stays on its server) and `collab.manageFile` shares, unshares, renames or removes
//! one with the recorded token.

use std::collections::BTreeMap;

use figcraft_collab::Change;
use figcraft_collab::code;
use figcraft_collab::message::{Admin, AdminReply, OwnedFile, Resume};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::live::{Access, Live, NetReply, NetRequest, Published, Share, Shared, SharedLink, Status, Via, clean_shared};
use crate::params::{req_string, string};
use crate::{DocState, Result, Session, bad, spec};

/// Most files remembered: recording another drops the one opened longest ago.
pub const MAX_ONLINE_FILES: usize = 200;

/// What a file's server said about it when it was last asked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FileState {
    /// Not asked yet in this run, or the server couldn't say.
    #[default]
    Unknown,
    /// On the server.
    Online,
    /// The server no longer has it for this owner (removed or expired).
    Gone,
}

/// A file of yours on a server (see the module docs). Most recently opened first in the list.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OnlineFile {
    /// The server's address (`wss://host/`), never with a key.
    pub server: String,
    /// The file's id and name there.
    pub file: String,
    pub name: String,
    /// The owner token: a password. Never logged or put in a command's result.
    pub owner: String,
    /// Your own links to the file (they carry their secrets).
    pub edit: String,
    pub view: String,
    /// When it was put on the server and when it was last opened here (seconds since 1970).
    pub created: u64,
    pub opened: u64,
    /// What others join with while it is shared (`None`: open only to you).
    pub shared: Option<Shared>,
    /// What the server said last (this run only).
    #[serde(skip)]
    pub state: FileState,
    /// When the server deletes it unless someone opens it (this run only).
    #[serde(skip)]
    pub expires: Option<u64>,
}

impl std::fmt::Debug for OnlineFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OnlineFile")
            .field("server", &self.server)
            .field("file", &self.file)
            .field("name", &self.name)
            .field("shared", &self.shared.is_some())
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

/// Why a server's files couldn't be asked about at the last refresh.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerProblem {
    /// It couldn't be reached, or refused (the reason).
    Offline(String),
    /// It is older than this request: what this computer remembers is shown as it is.
    Unsupported,
}

/// Servers whose files couldn't be asked about at the last refresh, by address.
pub type Problems = BTreeMap<String, ServerProblem>;

/// A recovery copy of one of your online files (`collab.recoverFile`): the file is being joined
/// again as session `session`, and the copy waits to be compared with what arrives.
pub struct Recovering {
    pub session: u64,
    /// The copy, as the unsaved document it opens as when it can't be merged.
    copy: Box<DocState>,
    /// Who the app was in the file when the copy was written, and the last change of the server's
    /// it had then: the copy is that state with the app's own unconfirmed changes on top.
    at: Resume,
}

/// A name as shown: no control characters, at most 256 characters.
fn clean_name(s: &str) -> String {
    s.trim().chars().filter(|c| !c.is_control()).take(256).collect()
}

/// A server address without its query, as recorded.
pub(crate) fn plain_server(server: &str) -> String {
    server.split('?').next().unwrap_or_default().trim().to_string()
}

/// Two addresses name the same server (case, a closing slash and the scheme's own port aside).
pub fn same_server(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        let s = plain_server(s).to_ascii_lowercase();
        let s = s.trim_end_matches('/');
        let port = if s.starts_with("wss://") { ":443" } else { ":80" };
        let start = s.find("://").map_or(0, |i| i + 3);
        let end = s[start..].find('/').map_or(s.len(), |i| start + i);
        match s[start..end].strip_suffix(port) {
            Some(host) => format!("{}{}{}", &s[..start], host, &s[end..]),
            None => s.to_string(),
        }
    };
    norm(a) == norm(b)
}

/// The host a server address names (`wss://share.example/` is `share.example`).
pub fn server_host(server: &str) -> &str {
    let plain = server.split('?').next().unwrap_or_default();
    plain.split_once("://").map_or(plain, |(_, rest)| rest).trim_end_matches('/')
}

fn secret(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && !s.contains(|c: char| c.is_control() || c.is_whitespace())
}

fn link(s: &str) -> bool {
    secret(s, 2048) && s.contains("://")
}

impl OnlineFile {
    /// The record made safe to use, or `None` when it can't be one (anything may be in a
    /// preferences file).
    fn clean(mut self) -> Option<OnlineFile> {
        self.server = plain_server(&self.server);
        let server = (self.server.starts_with("ws://") || self.server.starts_with("wss://")) && secret(&self.server, 512);
        let id = (1..=64).contains(&self.file.len()) && self.file.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if !(server && id && secret(&self.owner, 512) && link(&self.edit)) {
            return None;
        }
        if !link(&self.view) {
            self.view.clear();
        }
        self.name = clean_name(&self.name);
        let usable = |l: &SharedLink| link(&l.link) && l.id.len() <= 64 && !l.id.contains(char::is_control);
        self.shared = self.shared.take().filter(|s| usable(&s.edit)).map(|mut s| {
            s.view = s.view.take().filter(usable);
            s.comment = s.comment.take().filter(usable);
            clean_shared(s)
        });
        Some(self)
    }

    fn is(&self, server: &str, file: &str) -> bool {
        self.file == file && same_server(&self.server, server)
    }
}

/// The usable records of `list`, one per file, at most [`MAX_ONLINE_FILES`] (the first ones: the
/// list is most recently opened first).
fn clean(list: impl IntoIterator<Item = OnlineFile>) -> Vec<OnlineFile> {
    let mut out: Vec<OnlineFile> = Vec::new();
    for f in list.into_iter().filter_map(OnlineFile::clean) {
        if out.len() < MAX_ONLINE_FILES && !out.iter().any(|o| o.is(&f.server, &f.file)) {
            out.push(f);
        }
    }
    out
}

/// Reads the list from the preferences: whatever isn't a usable record is skipped, so junk there
/// never costs the other preferences.
pub(crate) fn read<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Vec<OnlineFile>, D::Error> {
    let v = Value::deserialize(d).unwrap_or(Value::Null);
    let entries = v.as_array().map(Vec::as_slice).unwrap_or_default();
    Ok(clean(entries.iter().take(4 * MAX_ONLINE_FILES).filter_map(|e| serde_json::from_value(e.clone()).ok())))
}

fn now() -> u64 {
    web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl Live {
    /// The server file this session is in as its owner or publisher: `(server, file id)`.
    pub(crate) fn server_file(&self) -> Option<(String, String)> {
        let p = self.published.as_ref()?;
        let server = self.share.as_ref().filter(|s| s.via == Via::Online).map_or(self.server.as_str(), |s| s.server.as_str());
        Some((server.to_string(), p.file.clone()))
    }
}

impl Session {
    /// Where the recorded file `file` on `server` is in the list.
    fn online_index(&self, server: &str, file: &str) -> Option<usize> {
        self.prefs.online_files.iter().position(|f| f.is(server, file))
    }

    /// The recorded file command `cmd` names with `file` (and `server`, when several servers have
    /// a file of that id).
    fn online_named(&self, cmd: &str, file: &str, server: Option<&str>) -> Result<usize> {
        let mut hits = self.prefs.online_files.iter().enumerate().filter(|(_, f)| f.file == file && server.is_none_or(|s| same_server(&f.server, s)));
        match (hits.next(), hits.next()) {
            (Some((i, _)), None) => Ok(i),
            (Some(_), Some(_)) => Err(bad(cmd, "files on several servers have that id: say which with `server`")),
            _ => Err(bad(cmd, "no online file with that id is recorded on this computer (`collab.files` lists them)")),
        }
    }

    /// The owner token recorded for `file` on `server`.
    pub(crate) fn online_owner(&self, server: &str, file: &str) -> Option<String> {
        self.online_index(server, file).and_then(|i| self.prefs.online_files.get(i)).map(|f| f.owner.clone())
    }

    /// The list changed: the app saves the preferences at once (a crash must not lose a token).
    fn online_changed(&mut self) {
        self.prefs.online_files.truncate(MAX_ONLINE_FILES);
        self.live.unsaved = true;
    }

    /// A file reached `server` (`collab.publish`, `collab.share`): remember it when it came with
    /// an owner token (one published with the server's key is the operator's to manage).
    pub(crate) fn online_published(&mut self, server: &str, p: &Published) {
        let Some(owner) = p.owner.clone() else { return };
        let t = now();
        let record = OnlineFile {
            server: plain_server(server),
            file: p.file.clone(),
            name: p.name.clone(),
            owner,
            edit: p.edit.clone(),
            view: p.view.clone(),
            created: t,
            opened: t,
            state: FileState::Online,
            expires: p.expires,
            ..OnlineFile::default()
        };
        let Some(record) = record.clean() else { return };
        self.prefs.online_files.retain(|f| !f.is(&record.server, &record.file));
        self.prefs.online_files.insert(0, record);
        self.online_changed();
    }

    /// What others join `file` on `server` with changed (`None`: nobody else gets in now): in the
    /// record, and in the sessions open on the file.
    pub(crate) fn online_shared(&mut self, server: &str, file: &str, links: Option<Shared>) {
        for l in self.live.sessions_mut().filter(|l| l.server_file().is_some_and(|(s, f)| f == file && same_server(&s, server))) {
            if let Some(sh) = l.share.as_mut() {
                sh.links.clone_from(&links);
            }
        }
        if let Some(f) = self.online_index(server, file).and_then(|i| self.prefs.online_files.get_mut(i)).filter(|f| f.shared != links) {
            f.shared = links;
            self.online_changed();
        }
    }

    /// The recorded file document `uid` is open on as its owner and syncs with (connected or
    /// trying again): `(server, file id)` and who the app is there with the last change of the
    /// server's it has. A copy of the document made now is that state plus the app's own changes
    /// the server hasn't confirmed: what recovering it after a crash takes (`collab.recoverFile`).
    pub fn online_of(&self, uid: u64) -> Option<(String, String, Resume)> {
        let live = self.live_of(uid);
        let there = matches!(live.status, Status::Live | Status::Reconnecting { .. }) && live.doc_uid == Some(uid);
        let at = live.client().filter(|_| there)?.resume();
        let (server, file) = live.server_file()?;
        self.online_index(&server, &file).map(|_| (plain_server(&server), file, at))
    }

    /// Settle the recovery copies whose file arrived from its server, or won't (see
    /// [`recover_file`]). Called when something happened in a session.
    pub(crate) fn online_recoveries(&mut self) {
        for r in std::mem::take(&mut self.live.recovering) {
            match self.live.get(r.session).map(|l| (l.syncing(), l.status.active())) {
                Some((true, _)) => self.online_merge(r),
                // Still on its way.
                Some((false, true)) => self.live.recovering.push(r),
                // It couldn't be joined, or was left: the copy is what there is.
                _ => {
                    let name = r.copy.doc.name.clone();
                    self.docs.push(*r.copy);
                    self.active = self.docs.len() - 1;
                    self.message = Some(format!("{name} couldn't be opened from its server: your recovered copy is open, with its unsaved changes"));
                }
            }
        }
    }

    /// The file of recovery copy `r` arrived: nothing to do when the server has everything the
    /// copy has; the copy's changes go on top as one edit when they are provably only the app's
    /// own unconfirmed ones (the same client, and nothing changed on the server since the copy);
    /// otherwise the copy opens beside the file.
    fn online_merge(&mut self, r: Recovering) {
        let Some(live) = self.live.get(r.session) else { return };
        let (Some(client), Some(uid)) = (live.client(), live.doc_uid) else { return };
        let name = r.copy.doc.name.clone();
        let me = client.resume();
        let ahead = figcraft_collab::diff(client.view(), &r.copy.doc).map(|d| d.changes);
        if ahead.as_ref().is_ok_and(Vec::is_empty) {
            return;
        }
        // New layers must be this client's to create (the server refuses anyone else's ids).
        let own = |changes: &[Change]| changes.iter().all(|c| !matches!(c, Change::Create { id, .. } if id.0 >> 32 != u64::from(me.client_id)));
        if me == r.at && client.access() == Access::Edit && ahead.as_deref().is_ok_and(own) {
            let mut doc = (*r.copy.doc).clone();
            doc.set_client_id(me.client_id);
            self.apply_remote(uid, std::sync::Arc::new(doc));
            self.live_edit(uid, Some("Recovered changes"));
            self.message = Some(format!("Recovered your changes to {name} that hadn't reached the server"));
            return;
        }
        let mut copy = *r.copy;
        std::sync::Arc::make_mut(&mut copy.doc).name = format!("{name} (recovered copy)");
        self.docs.push(copy);
        self.message =
            Some(format!("{name} changed on the server before your last changes reached it: they are open beside it as “{name} (recovered copy)”"));
    }

    /// The session open on recorded file `f`, if one is.
    fn online_session(&self, f: &OnlineFile) -> Option<&Live> {
        self.live.iter().find(|l| l.status.active() && l.server_file().is_some_and(|(s, id)| f.is(&s, &id)))
    }

    /// Ask every server once what became of the files recorded on it.
    fn online_refresh(&mut self) -> Result<()> {
        let mut servers: Vec<String> = Vec::new();
        for f in &self.prefs.online_files {
            if !servers.iter().any(|s| same_server(s, &f.server)) {
                servers.push(f.server.clone());
            }
        }
        for server in servers {
            let asked: Vec<(String, String)> =
                self.prefs.online_files.iter().filter(|f| same_server(&f.server, &server)).map(|f| (f.file.clone(), f.owner.clone())).collect();
            let req =
                NetRequest::Hosting { server: server.clone(), file: String::new(), owner: String::new(), req: Some(Admin::Owned { files: asked }) };
            // What was asked when the answer is an earlier one (a command run in the background).
            let (reply, earlier) = self.net(req.clone())?;
            let asked = match earlier.unwrap_or(req) {
                NetRequest::Hosting { req: Some(Admin::Owned { files }), .. } => files,
                _ => Vec::new(),
            };
            let problem = match reply {
                NetReply::Hosting(Ok((_, Some(AdminReply::Owned(answers))))) if answers.len() == asked.len() => {
                    for ((file, _), answer) in asked.iter().zip(answers) {
                        self.online_answer(&server, file, answer);
                    }
                    None
                }
                // A server from before this request can't read it.
                NetReply::Hosting(Err(why)) if why.contains("couldn't be read") => Some(ServerProblem::Unsupported),
                NetReply::Hosting(Err(why)) | NetReply::Hosting(Ok((_, Some(AdminReply::Failed { reason: why })))) => {
                    Some(ServerProblem::Offline(why))
                }
                _ => Some(ServerProblem::Offline("the server answered something else".into())),
            };
            self.live.servers.retain(|s, _| !same_server(s, &server));
            if let Some(p) = problem {
                self.live.servers.insert(server, p);
            }
        }
        Ok(())
    }

    /// What `server` said about recorded file `file`: there (with its name, links and expiry), or
    /// gone.
    fn online_answer(&mut self, server: &str, file: &str, answer: Option<OwnedFile>) {
        let Some(i) = self.online_index(server, file) else { return };
        let Some(f) = self.prefs.online_files.get_mut(i) else { return };
        let before = (f.name.clone(), f.shared.is_some());
        // Links revoked from elsewhere no longer let anyone in.
        let live = |f: &OnlineFile, o: &OwnedFile| f.shared.as_ref().is_some_and(|s| s.all().all(|l| o.links.contains(&l.id)));
        let shared = match answer {
            Some(o) => {
                f.state = FileState::Online;
                f.expires = o.expires;
                let name = clean_name(&o.name);
                if !name.is_empty() {
                    f.name = name;
                }
                live(f, &o)
            }
            None => {
                f.state = FileState::Gone;
                f.expires = None;
                false
            }
        };
        if before.1 && !shared {
            self.online_shared(server, file, None);
        } else if self.prefs.online_files.get(i).is_some_and(|f| f.name != before.0) {
            self.online_changed();
        }
    }

    /// The list as `collab.files` gives it: no token, no link of yours.
    fn online_json(&self) -> Value {
        let files: Vec<Value> = self
            .prefs
            .online_files
            .iter()
            .map(|f| {
                let session = self.online_session(f);
                let code = |l: Option<&SharedLink>| l.and_then(|l| l.code.as_deref()).map(code::grouped);
                json!({
                    "server": f.server,
                    "host": server_host(&f.server),
                    "file": f.file,
                    "name": f.name,
                    "created": f.created,
                    "opened": f.opened,
                    "shared": f.shared.is_some(),
                    "code": code(f.shared.as_ref().map(|s| &s.edit)),
                    "viewCode": code(f.shared.as_ref().and_then(|s| s.view.as_ref())),
                    "commentCode": code(f.shared.as_ref().and_then(|s| s.comment.as_ref())),
                    "state": f.state,
                    "expires": f.expires,
                    "session": session.map(|l| l.id),
                    "document": session.and_then(|l| l.doc_uid).and_then(|u| self.docs.iter().position(|d| d.uid == u)),
                    "hasOwnerToken": true,
                })
            })
            .collect();
        let servers: Vec<Value> = self
            .live
            .servers
            .iter()
            .map(|(server, p)| match p {
                ServerProblem::Offline(why) => json!({"server": server, "host": server_host(server), "state": "offline", "reason": why}),
                ServerProblem::Unsupported => json!({"server": server, "host": server_host(server), "state": "unsupported", "reason": Value::Null}),
            })
            .collect();
        json!({"files": files, "problems": servers, "max": MAX_ONLINE_FILES})
    }
}

fn files(s: &mut Session, p: &Value) -> Result<Value> {
    match p.get("refresh") {
        None => {}
        Some(v) if v.as_bool() == Some(false) => {}
        Some(v) if v.as_bool() == Some(true) => s.online_refresh()?,
        Some(_) => return Err(bad("collab.files", "`refresh` must be true or false")),
    }
    Ok(s.online_json())
}

/// The recorded file `cmd`'s `file` (and `server`) name.
fn named(s: &Session, cmd: &str, p: &Value) -> Result<usize> {
    let file = req_string(cmd, p, "file")?;
    s.online_named(cmd, file.trim(), string(p, "server").as_deref().map(str::trim))
}

fn open_file(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "collab.openFile";
    let i = named(s, CMD, p)?;
    let Some(f) = s.prefs.online_files.get(i).cloned() else { return Err(bad(CMD, "no such file")) };
    if f.state == FileState::Gone {
        return Err(bad(CMD, "this file is no longer on its server (it was removed or expired): `collab.forgetFile` clears it from the list"));
    }
    let brief = |session: u64, status: &str, document: Option<usize>, open: bool| json!({"session": session, "status": status, "document": document, "alreadyOpen": open, "file": f.file, "server": f.server, "name": f.name});
    // Open already: its tab comes to the front.
    if let Some((id, status, uid)) = s.online_session(&f).map(|l| (l.id, l.status.id(), l.doc_uid)) {
        let document = uid.and_then(|u| s.docs.iter().position(|d| d.uid == u));
        if let Some(d) = document {
            s.active = d;
        }
        return Ok(brief(id, status, document, true));
    }
    let id = join_recorded(s, CMD, i, None)?;
    Ok(brief(id, "connecting", None, false))
}

/// Join recorded file `i` as its owner for command `cmd` (`resume`: as the client the app was
/// there): the new session. The file leads the list.
fn join_recorded(s: &mut Session, cmd: &str, i: usize, resume: Option<Resume>) -> Result<u64> {
    let Some(f) = s.prefs.online_files.get(i).cloned() else { return Err(bad(cmd, "no such file")) };
    // Yours on the server: its sharing is this session's to start and stop. The token stays in
    // the record.
    let published = Published {
        server: f.server.clone(),
        file: f.file.clone(),
        name: f.name.clone(),
        edit: f.edit.clone(),
        view: f.view.clone(),
        owner: None,
        expires: f.expires,
    };
    let share = Share { via: Via::Online, server: f.server.clone(), access: Access::Edit, links: f.shared.clone() };
    let id = crate::live::start_join(s, cmd, &f.edit, Some(published), Some(share), resume)?;
    // Most recently opened first.
    let mut record = s.prefs.online_files.remove(i);
    record.opened = now();
    s.prefs.online_files.insert(0, record);
    s.online_changed();
    Ok(id)
}

/// `collab.recoverFile`: a recovery copy of one of your online files, written while the app was
/// the client `session`/`client` in it with the server's changes up to `seq`. The file is joined
/// again as its owner (as that client, when the server still knows it); the copy is kept until
/// what arrives is compared with it ([`Session::online_recoveries`]), so nothing in it is lost.
fn recover_file(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "collab.recoverFile";
    let path = req_string(CMD, p, "path")?;
    let i = named(s, CMD, p)?;
    let num = |key: &str| p.get(key).and_then(Value::as_u64).ok_or_else(|| bad(CMD, format!("`{key}` must be a whole number")));
    let client_id = u32::try_from(num("client")?).map_err(|_| bad(CMD, "`client` must be a client id"))?;
    let secret = p.get("secret").and_then(Value::as_u64).unwrap_or(0);
    let at = Resume { session: num("session")?, client_id, seq: num("seq")?, secret };
    if s.prefs.online_files.get(i).is_some_and(|f| s.online_session(f).is_some()) {
        return Err(bad(CMD, "this file is open already"));
    }
    let copy = Box::new(crate::cmd::file::recovered(&path)?);
    // The whole document is asked for (no change is "after" this number): the app has no state
    // of its own to catch up from.
    let session = join_recorded(s, CMD, i, Some(Resume { seq: u64::MAX, ..at }))?;
    s.live.recovering.push(Recovering { session, copy, at });
    Ok(json!({"session": session, "status": "connecting"}))
}

fn forget_file(s: &mut Session, p: &Value) -> Result<Value> {
    let i = named(s, "collab.forgetFile", p)?;
    let f = s.prefs.online_files.remove(i);
    s.online_changed();
    Ok(json!({"forgotten": true, "file": f.file, "server": f.server, "onServer": f.state != FileState::Gone}))
}

/// Who manages `file` for `collab.manageFile`: the server and owner token given, else the
/// recorded ones.
pub(crate) fn manager(s: &Session, cmd: &str, p: &Value, file: &str) -> Result<(String, String)> {
    let given = |key: &str| string(p, key).map(|v| v.trim().to_string());
    match (given("server"), given("owner")) {
        (Some(server), Some(owner)) => Ok((server, owner)),
        (server, owner) => {
            let missing = if server.is_none() { "`server`" } else { "`owner`" };
            let i = s.online_named(cmd, file, server.as_deref()).map_err(|e| match e {
                crate::EngineError::BadParams { msg, .. } => bad(cmd, format!("{missing} is missing, and {msg}")),
                e => e,
            })?;
            let f = s.prefs.online_files.get(i).ok_or_else(|| bad(cmd, "no such file"))?;
            Ok((server.unwrap_or_else(|| f.server.clone()), owner.unwrap_or_else(|| f.owner.clone())))
        }
    }
}

/// `collab.manageFile` renamed or removed `file` on `server`: the record follows.
pub(crate) fn managed(s: &mut Session, server: &str, file: &str, renamed: Option<&str>) {
    let Some(i) = s.online_index(server, file) else { return };
    match renamed {
        Some(name) => {
            if let Some(f) = s.prefs.online_files.get_mut(i) {
                f.name = clean_name(name);
            }
        }
        None => {
            s.prefs.online_files.remove(i);
        }
    }
    s.online_changed();
}

/// `file` was removed from `server`: the sessions open on it end here (their copies stay open),
/// before the server's goodbye reads as a failure.
pub(crate) fn removed(s: &mut Session, server: &str, file: &str) {
    let open: Vec<u64> =
        s.live.iter().filter(|l| l.server_file().is_some_and(|(sv, f)| f == file && same_server(&sv, server))).map(|l| l.id).collect();
    for id in open {
        s.live.end(id);
    }
}

/// `collab.manageFile {action: "share" | "unshare"}`: give others new links and codes for `file`,
/// or take away the ones they have. The file itself stays where it is, its owner's.
pub(crate) fn set_shared(s: &mut Session, cmd: &str, server: &str, file: &str, owner: &str, on: bool) -> Result<Value> {
    let shared = s.online_index(server, file).and_then(|i| s.prefs.online_files.get(i)).and_then(|f| f.shared.clone());
    let result = |links: Option<&Shared>| {
        let (edit, view, comment) = (links.map(|l| &l.edit), links.and_then(|l| l.view.as_ref()), links.and_then(|l| l.comment.as_ref()));
        let code = |l: Option<&SharedLink>| l.and_then(|l| l.code.as_deref()).map(code::grouped);
        json!({
            "shared": links.is_some(),
            "code": code(edit),
            "viewCode": code(view),
            "commentCode": code(comment),
            "editLink": edit.map(|l| l.link.as_str()),
            "viewLink": view.map(|l| l.link.as_str()),
            "commentLink": comment.map(|l| l.link.as_str()),
        })
    };
    // As asked already: nothing to tell the server.
    if on == shared.is_some() && (on || s.online_index(server, file).is_some()) {
        return Ok(result(shared.as_ref()));
    }
    if !on && shared.is_none() {
        return Err(bad(cmd, "the links others got aren't recorded on this computer: revoke them by id (action: links, then revoke)"));
    }
    let revoke: Vec<String> = shared.iter().flat_map(Shared::all).map(|l| l.id.clone()).collect();
    let req = NetRequest::Share { server: server.to_string(), file: file.to_string(), owner: owner.to_string(), revoke, create: on };
    let links = crate::live::share_answer(s, req)?.map(clean_shared);
    if on && links.is_none() {
        return Err(crate::EngineError::Other("the server made no links".into()));
    }
    s.online_shared(server, file, links.clone());
    Ok(result(links.as_ref()))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "collab.files",
            "Your online files",
            "{refresh? (true: ask each server once what became of its files; default false: no network)} → {files: [{server, host, file (its id), name, created, opened (seconds since 1970), shared (others get in with a code now), code, viewCode, commentCode (the join codes, while shared; commentCode null from a server older than comment links), state: unknown | online | gone (the server no longer has it: removed or expired), expires, session, document (the session and open document it is in now, or null), hasOwnerToken: true}], problems: [{server, host, state: offline | unsupported (a server older than this request), reason}], max}: the files you published or shared without a server key, most recently opened first, as recorded on this computer with the preferences. Owner tokens and your own links are never in the result",
            false,
            files
        ),
        spec!(
            "collab.openFile",
            "Open online file",
            "{file (its id, from `collab.files`), server? (when files on several servers have that id)} → {session, status: \"connecting\", document, alreadyOpen, file, server, name}: joins the file as its owner in a tab of its own (`collab.peers` tells when it's live; `collab.share` and `collab.stop` there start and stop its sharing); a file that is open already comes to the front instead (alreadyOpen: true)",
            false,
            open_file
        ),
        spec!(
            "collab.forgetFile",
            "Forget online file",
            "{file, server?} → {forgotten, file, server, onServer}: drops the file from the list on this computer only. The file stays on its server until it expires, and without its owner token it can't be managed from here again (`collab.manageFile {action: \"remove\"}` takes it off the server instead)",
            false,
            forget_file
        ),
        spec!(
            "collab.recoverFile",
            "Recover online file",
            "{path (a recovery copy of one of your online files), file, server?, session, client, seq, secret? (who the app was in the file when the copy was written, and the server's last change it had: from the note beside the copy)} → {session, status: \"connecting\"}: joins the file again as its owner and keeps the copy until the file arrives. Then: the server has everything in the copy → nothing more; the copy has changes of yours the server never confirmed and nothing else changed there → they are applied on top as one edit (\"Recovered changes\", synced to everyone); anything else changed meanwhile → the copy opens beside the file as “<name> (recovered copy)”; the file can't be joined → the copy opens as an unsaved document. The session's message says which",
            false,
            recover_file
        ),
    ]
}
