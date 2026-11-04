//! Sessions you joined by a link or a code, remembered on this computer so they can be joined again
//! from the home screen without the link (#1882). Each record holds its link, which carries the
//! link's secret: like the owner tokens in [`crate::online`] it stays in the preferences file, and
//! never goes in a log or a command's result.
//!
//! Whether a session's server can be reached is asked of the server itself (`/healthz`). When it
//! can't be, whether this computer has a network at all tells "the server is down" from "you're
//! offline", from the local routing table: nothing is sent to anyone else to find out.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::live::{NetReply, NetRequest, Role, SessionInfo};
use crate::params::{req_string, string};
use crate::{Result, Session, bad, spec};

/// How many sessions are remembered; the ones opened longest ago go first.
pub const MAX_JOINED_FILES: usize = 100;

const MAX_NAME: usize = 200;
const MAX_LINK: usize = 2048;

/// A session you joined (see the module docs). Most recently opened first in the list.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct JoinedFile {
    /// Where it is, as shown: `host:port`.
    pub address: String,
    /// The file's id there.
    pub file: String,
    pub name: String,
    /// The invite link you joined with: a secret. Never logged or put in a command's result.
    pub link: String,
    /// `edit`, `comment` or `view`, as the session said.
    pub access: String,
    /// When it was last opened here (seconds since 1970).
    pub opened: u64,
}

impl fmt::Debug for JoinedFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JoinedFile").field("address", &self.address).field("file", &self.file).field("name", &self.name).finish_non_exhaustive()
    }
}

impl JoinedFile {
    /// The record if it can be used: it has a place and a link, within bounds.
    fn clean(mut self) -> Option<JoinedFile> {
        self.name = self.name.chars().filter(|c| !c.is_control()).take(MAX_NAME).collect();
        let usable = !self.address.is_empty() && !self.file.is_empty() && !self.link.is_empty() && self.link.len() <= MAX_LINK;
        usable.then_some(self)
    }

    /// The name of this record's cached preview: the same session always gets the same one, and it
    /// is safe in a path whatever the record says.
    pub fn preview_key(&self) -> String {
        preview_key(&self.address, &self.file)
    }
}

/// The name of the cached preview of `file` at `address` (FNV-1a in hex).
pub fn preview_key(address: &str, file: &str) -> String {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for b in address.bytes().chain([0]).chain(file.bytes()) {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Reads the list from the preferences: whatever isn't a usable record is skipped, so junk there
/// never costs the other preferences.
pub(crate) fn read<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Vec<JoinedFile>, D::Error> {
    let v = Value::deserialize(d).unwrap_or(Value::Null);
    let entries = v.as_array().map(Vec::as_slice).unwrap_or_default();
    let mut out: Vec<JoinedFile> = Vec::new();
    for f in
        entries.iter().take(4 * MAX_JOINED_FILES).filter_map(|e| serde_json::from_value::<JoinedFile>(e.clone()).ok()).filter_map(JoinedFile::clean)
    {
        if out.len() < MAX_JOINED_FILES && !out.iter().any(|o| o.address == f.address && o.file == f.file) {
            out.push(f);
        }
    }
    Ok(out)
}

/// The value of `key` in the query of `link`.
fn query_value<'a>(link: &'a str, key: &str) -> Option<&'a str> {
    link.split_once('?')?.1.split('&').find_map(|kv| kv.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| v))
}

fn now() -> u64 {
    web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl Session {
    /// You're in the session `info` says, with its file called `name`: remember it. A file you
    /// published yourself is in [`crate::online`] already.
    pub(crate) fn joined_record(&mut self, info: &SessionInfo, name: &str, published: bool) {
        if published || info.role != Role::Guest {
            return;
        }
        let access = serde_json::to_value(info.access).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
        let record = JoinedFile {
            address: info.address.clone(),
            file: query_value(&info.link, "file").unwrap_or_default().to_string(),
            name: name.to_string(),
            link: info.link.clone(),
            access,
            opened: now(),
        };
        let Some(record) = record.clean() else { return };
        self.prefs.joined_files.retain(|f| f.address != record.address || f.file != record.file);
        self.prefs.joined_files.insert(0, record);
        self.prefs.joined_files.truncate(MAX_JOINED_FILES);
        self.live.unsaved = true;
    }

    /// Ask each remembered session's server whether it can be reached.
    fn joined_refresh(&mut self) -> Result<()> {
        let mut servers: BTreeMap<String, String> = BTreeMap::new();
        for f in &self.prefs.joined_files {
            servers.entry(f.address.clone()).or_insert_with(|| f.link.clone());
        }
        for (address, link) in servers {
            let (reply, _) = self.net(NetRequest::Probe { link })?;
            if let NetReply::Probe(Some(reach)) = reply {
                self.live.reach.insert(address, reach);
            }
        }
        Ok(())
    }

    fn joined_json(&self) -> Value {
        let files: Vec<Value> = self
            .prefs
            .joined_files
            .iter()
            .map(|f| {
                let reach = self.live.reach.get(&f.address).map(|r| r.id());
                json!({"address": f.address, "file": f.file, "name": f.name, "access": f.access, "opened": f.opened, "reach": reach})
            })
            .collect();
        json!({"files": files})
    }

    /// Where the record `cmd`'s `file` (and `address`) name is in the list.
    fn joined_named(&self, cmd: &str, p: &Value) -> Result<usize> {
        let file = req_string(cmd, p, "file")?;
        let address = string(p, "address");
        let mut hits = self.prefs.joined_files.iter().enumerate().filter(|(_, f)| f.file == file && address.as_ref().is_none_or(|a| *a == f.address));
        match (hits.next(), hits.next()) {
            (Some((i, _)), None) => Ok(i),
            (Some(_), Some(_)) => Err(bad(cmd, "sessions on several servers have that id: say which with `address`")),
            _ => Err(bad(cmd, "no joined session with that id is remembered on this computer (`collab.joinedFiles` lists them)")),
        }
    }
}

fn joined_files(s: &mut Session, p: &Value) -> Result<Value> {
    match p.get("refresh") {
        None => {}
        Some(v) if v.as_bool() == Some(false) => {}
        Some(v) if v.as_bool() == Some(true) => s.joined_refresh()?,
        Some(_) => return Err(bad("collab.joinedFiles", "`refresh` must be true or false")),
    }
    Ok(s.joined_json())
}

fn open_joined(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "collab.openJoined";
    let i = s.joined_named(CMD, p)?;
    let Some(f) = s.prefs.joined_files.get(i).cloned() else { return Err(bad(CMD, "no such session")) };
    // In it already: its tab comes to the front.
    let open = s.live.iter().find(|l| l.status.active() && l.info.as_ref().is_some_and(|n| n.link == f.link)).and_then(|l| l.doc_uid);
    if let Some(index) = open.and_then(|uid| s.docs.iter().position(|d| d.uid == uid)) {
        s.active = index;
        return Ok(json!({"alreadyOpen": true, "document": index}));
    }
    let session = crate::live::start_join(s, CMD, &f.link, None, None, None)?;
    Ok(json!({"session": session, "status": "connecting"}))
}

fn forget_joined(s: &mut Session, p: &Value) -> Result<Value> {
    let i = s.joined_named("collab.forgetJoined", p)?;
    let f = s.prefs.joined_files.remove(i);
    s.live.unsaved = true;
    Ok(json!({"forgotten": true, "file": f.file, "address": f.address}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "collab.joinedFiles",
            "Sessions you joined",
            "{refresh? (true: ask each session's server whether it can be reached; default false: no network)} → {files: [{address, file, name, access, opened, reach: online, serverDown, noNetwork or null (not asked)}]}: the sessions you joined on this computer, most recently opened first, without their links",
            false,
            joined_files
        ),
        spec!(
            "collab.openJoined",
            "Join session again",
            "{file, address? (when sessions on several servers have that id)} → {session, status} or {alreadyOpen, document}: joins a session from `collab.joinedFiles` with the link it was joined with",
            false,
            open_joined
        ),
        spec!(
            "collab.forgetJoined",
            "Forget joined session",
            "{file, address?} → {forgotten, file, address}: takes it off this computer's list. The file stays on its server, and its link still works",
            false,
            forget_joined
        ),
    ]
}
