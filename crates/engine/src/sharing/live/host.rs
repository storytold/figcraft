use super::types::{Bind, HostOptions, NetReply, NetRequest, ONLINE_OFF, PublishOptions, Role, SessionId, Share, Shared, SharedLink, Status, Via};
use super::{
    Access, Admin, AdminReply, ClientMsg, EngineError, HostInfo, IpAddr, Live, Result, Session, Value, bad, code, display_name, get, json,
    req_string, string,
};

/// What a server says about its hosting, made safe to show: no control characters, short, and a
/// terms address only when it is a web page (`http(s)://`).
pub(super) fn clean_hosting(mut info: HostInfo) -> HostInfo {
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

pub(super) fn unavailable(cmd: &str) -> EngineError {
    EngineError::Disabled(
        cmd.to_string(),
        "live sessions aren't available here (the desktop app and the MCP server host and join them; the web build joins)".into(),
    )
}

pub(super) fn host(s: &mut Session, p: &Value) -> Result<Value> {
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

pub(super) fn publish(s: &mut Session, p: &Value) -> Result<Value> {
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

/// Why a local network share has no view-only or comment code.
const LOCAL_EDITS: &str = "everyone who joins a file shared on the local network can edit it: share it online for a view-only or comment code";

/// Why a share has no comment code.
const NO_COMMENT_LINK: &str =
    "this share has no comment code (its server is older than comment links, or it was shared before them): stop sharing and share again";

/// The `access` parameter of `cmd`, if given.
fn access_param(cmd: &str, p: &Value) -> Result<Option<Access>> {
    string(p, "access").map(|a| Access::parse(&a).ok_or_else(|| bad(cmd, "`access` must be \"edit\", \"comment\" or \"view\""))).transpose()
}

pub(super) fn share(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "collab.share";
    let via = match string(p, "via").as_deref().map(str::trim) {
        None | Some("online") => Via::Online,
        Some("local") => Via::Local,
        Some(_) => return Err(bad(CMD, "`via` must be \"online\" or \"local\"")),
    };
    let access = access_param(CMD, p)?;
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
        if access.is_some_and(|a| a != Access::Edit) && shared == Via::Local {
            return Err(bad(CMD, LOCAL_EDITS));
        }
        if let Some(req) = share_request(s, Vec::new(), true).filter(|_| s.live().share.as_ref().is_some_and(|sh| sh.links.is_none())) {
            let links = share_answer(s, req)?.ok_or_else(|| EngineError::Other("the server made no links".into()))?;
            shared_now(s, Some(clean_shared(links)));
        }
        if let (Some(a), Some(sh)) = (access, s.live_mut().and_then(|l| l.share.as_mut())) {
            if sh.links.as_ref().is_some_and(|l| l.of(a).is_none()) {
                return Err(bad(CMD, NO_COMMENT_LINK));
            }
            sh.access = a;
        }
        return Ok(s.live_summary());
    }
    let access = access.unwrap_or_default();
    let mut server = String::new();
    let id = match via {
        Via::Local if access != Access::Edit => return Err(bad(CMD, LOCAL_EDITS)),
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
            Shared { edit: SharedLink { id: String::new(), link: i.link.clone(), code }, ..Shared::default() }
        });
        live.share = Some(Share { via, server, access, links });
    }
    Ok(s.live_summary())
}

/// What others join the open file's server file with changed: in its session and its record.
pub(super) fn shared_now(s: &mut Session, links: Option<Shared>) {
    if let Some((server, file)) = s.live().server_file() {
        s.online_shared(&server, &file, links);
    }
}

/// Links and codes as a server gave them, with only codes that are codes (nine symbols).
pub(crate) fn clean_shared(mut links: Shared) -> Shared {
    let clean = |l: &mut SharedLink| l.code = l.code.take().and_then(|c| code::normalize(&c).ok()).filter(|c| c.len() == code::ONLINE_LEN);
    clean(&mut links.edit);
    links.view.as_mut().map(clean);
    links.comment.as_mut().map(clean);
    links
}

/// The request that changes who else gets into the server file the open file's session is its
/// owner's session of (`None`: it isn't one, or the file isn't there yet).
pub(super) fn share_request(s: &Session, revoke: Vec<String>, create: bool) -> Option<NetRequest> {
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
pub(super) fn hosting_request(
    s: &mut Session,
    cmd: &str,
    server: &str,
    file: &str,
    owner: &str,
    req: Option<Admin>,
) -> Result<(HostInfo, Option<AdminReply>)> {
    let req = NetRequest::Hosting { server: server.to_string(), file: file.to_string(), owner: owner.to_string(), req };
    match s.net(req)?.0 {
        // The server's refusals say why themselves.
        NetReply::Hosting(Ok((_, Some(AdminReply::Failed { reason })))) | NetReply::Hosting(Err(reason)) => Err(EngineError::Other(reason)),
        NetReply::Hosting(Ok((info, reply))) => Ok((clean_hosting(info), reply)),
        _ => Err(EngineError::Other(format!("{cmd}: the server answered something else"))),
    }
}

pub(super) fn server_info(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "collab.serverInfo";
    let server = req_string(CMD, p, "server")?;
    let (info, _) = hosting_request(s, CMD, server.trim(), "", "", None)?;
    Ok(hosting_json(&info))
}

pub(super) fn manage_file(s: &mut Session, p: &Value) -> Result<Value> {
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
            let access = access_param(CMD, p)?.unwrap_or(Access::View);
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

pub(super) fn report(s: &mut Session, p: &Value) -> Result<Value> {
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
