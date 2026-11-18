//! The team list of the server in Preferences (milestone M9): the files shared from it now, which
//! everyone who reaches the server may open, when its operator turned the list on (`team_files`).
//! `collab.files {refresh: true}` asks for it beside your own files, and Home ▸ Online shows it.
//!
//! A file of the list opens by its team list link, `figcraft://join?server=…&file=…&team=1`: the
//! server hands out the file's team link while the file is shared, so the link carries no secret
//! and can go anywhere people who reach the server read (a ticket, a chat). With the address the
//! server gives for its browser build, the same file opens in a browser: `<web>#join=<link>`.

use figcraft_collab::message::{Access, Admin, AdminReply, MAX_TEAM};
use serde_json::{Value, json};

use crate::live::{DEFAULT_SERVER, NetReply, NetRequest};
use crate::online::{plain_server, same_server, server_host};
use crate::{Result, Session};

/// A file of a team list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TeamFile {
    /// Its id on the server, and its name there.
    pub file: String,
    pub name: String,
    /// When it last changed (seconds since 1970) and how many people are in it now.
    pub updated: u64,
    pub people: u32,
    /// What its team link lets people do.
    pub access: Access,
}

/// What a server said about its team list at the last refresh (this run only).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TeamList {
    /// The files shared now, most recently changed first, and where the server's browser build is
    /// (empty: it didn't say).
    Files { web: String, files: Vec<TeamFile> },
    /// The server keeps no team list (or is older than team lists).
    None,
    /// It couldn't be reached, or refused (why).
    Offline(String),
}

/// The team list of the server at `server`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Team {
    pub server: String,
    pub list: TeamList,
}

/// The link that opens `file` of the team list on `server` (no secret in it).
pub fn team_link(server: &str, file: &str) -> String {
    format!("figcraft://join?server={}&file={file}&team=1", plain_server(server))
}

/// The same file in the server's browser build at `web`: the link rides in the address's fragment,
/// which the browser never sends to the web server.
pub fn web_link(web: &str, server: &str, file: &str) -> String {
    let escaped: String = team_link(server, file)
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => char::from(b).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!("{web}#join={escaped}")
}

/// A file id as servers make them (and nothing a link could be bent with).
fn valid_id(s: &str) -> bool {
    (1..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// A browser build's address as the server gave it, when it is one.
fn valid_web(s: &str) -> bool {
    (s.starts_with("https://") || s.starts_with("http://"))
        && s.len() <= 500
        && !s.contains(|c: char| c.is_whitespace() || c.is_control() || c == '#')
}

/// The team list in a server's answer, made safe to show (anything may arrive).
fn listed(web: String, files: Vec<figcraft_collab::message::TeamFile>) -> TeamList {
    let files = files
        .into_iter()
        .filter(|f| valid_id(&f.file.id))
        .take(MAX_TEAM)
        .map(|f| TeamFile {
            name: f.file.name.trim().chars().filter(|c| !c.is_control()).take(256).collect(),
            file: f.file.id,
            updated: f.file.updated,
            people: f.file.people,
            access: f.access,
        })
        .collect();
    TeamList::Files { web: if valid_web(&web) { web } else { String::new() }, files }
}

impl Session {
    /// Ask the server in Preferences for its team list (none asked when online sharing is off,
    /// without the network, or of the public server FigCraft ships with: it keeps none).
    pub(crate) fn team_refresh(&mut self) -> Result<()> {
        let server = plain_server(&self.prefs.publish_server);
        let public = option_env!("FIGCRAFT_DEFAULT_SERVER").is_none() && same_server(&server, DEFAULT_SERVER);
        if server.is_empty() || public || self.live.link().is_none() || crate::file_access::network_refused().is_some() {
            self.live.team = None;
            return Ok(());
        }
        let req = NetRequest::Hosting { server: server.clone(), file: String::new(), owner: String::new(), req: Some(Admin::Team) };
        let list = match self.net(req)?.0 {
            NetReply::Hosting(Ok((_, Some(AdminReply::Team { web, files })))) => listed(web, files),
            // No team list there; a server older than team lists can't read the question.
            NetReply::Hosting(Ok((_, Some(AdminReply::Failed { .. })))) => TeamList::None,
            NetReply::Hosting(Err(why)) if why.contains("couldn't be read") => TeamList::None,
            NetReply::Hosting(Err(why)) => TeamList::Offline(why),
            _ => TeamList::None,
        };
        self.live.team = Some(Team { server, list });
        Ok(())
    }

    /// The team list of the server in Preferences, as asked last: `None` when it wasn't asked (or
    /// Preferences name another server now).
    pub fn team(&self) -> Option<&Team> {
        self.live.team.as_ref().filter(|t| same_server(&t.server, &self.prefs.publish_server))
    }

    /// The team list as `collab.files` gives it.
    pub(crate) fn team_json(&self) -> Value {
        let Some(t) = self.team() else { return Value::Null };
        let (state, reason, web, files) = match &t.list {
            TeamList::Files { web, files } => ("listed", None, Some(web.as_str()).filter(|w| !w.is_empty()), files.as_slice()),
            TeamList::None => ("none", None, None, &[][..]),
            TeamList::Offline(why) => ("offline", Some(why.as_str()), None, &[][..]),
        };
        let files: Vec<Value> = files
            .iter()
            .map(|f| {
                json!({
                    "file": f.file,
                    "name": f.name,
                    "updated": f.updated,
                    "people": f.people,
                    "access": f.access.id(),
                    "link": team_link(&t.server, &f.file),
                    "webLink": web.map(|w| web_link(w, &t.server, &f.file)),
                })
            })
            .collect();
        json!({"server": t.server, "host": server_host(&t.server), "state": state, "reason": reason, "web": web, "files": files})
    }
}
