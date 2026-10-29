use super::host::{host, hosting_request, manage_file, publish, report, server_info, share, share_answer, share_request, shared_now, unavailable};
use super::types::{Published, Role, SessionId, Share, Status, join_target};
use super::{
    Admin, AdminReply, CommandSpec, EngineError, Live, Result, Resume, Session, Value, bad, display_name, get, json, num, req_string, spec, string,
};

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
            "{via?: \"online\" (default: the open file is kept on the server in Preferences, `pref.publishServer`, and the server hands out the codes) | \"local\" (this app hosts it for people on the same network; no server), access?: \"edit\" (default) | \"view\" (online only: which code `code` is), server? (online: another server's address, with ?key=… when it needs its key), key? (online: the server's key, when the address has none), name? (online: the file's name there), port?, bind?: \"lan\" (default) | \"local\" (this machine only) | an IP address (local)} → the open file's session as `collab.peers` gives it: `code` is what others type to join (`collab.join {code}`, the Join field), `link` an invite link. Local: at once, the code carries the host's IPv4 address, port and secret (K7M2Q-…, 20 symbols). Online: status \"connecting\" first; once live `code` is the server's (K7M-2QX-9TD) and `share` is {via, access, shared, editCode, viewCode, editLink, viewLink}: links made for others (the file's own links in `published` stay yours); `code` stays null when the server is older than join codes (give `share.editLink` / `share.viewLink` instead). Called again while shared: only switches which code `code` is (`access`). Called after `collab.stop` on a file that is still on the server: new links and codes for that same file (nothing is published twice). `collab.stop` stops sharing",
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
            "{code | link (a join code: K7M-2QX-9TD from a server, asked at the server in Preferences; a longer one for the local network; spaces, dashes and case don't matter, O reads as 0 and I or L as 1; or an invite link: figcraft://join?host=…&file=…&token=…, figcraft://join?server=ws(s)://…&file=…&token=… or ws(s)://host:port/?file=…&token=…, from a session an app hosts or a server's share link; either key takes either)} → the new session: {session, status: \"connecting\", …}; its file opens in a tab of its own (other files' sessions go on) and `collab.peers` tells when it's live (`sessions` lists it until then, and if it fails)",
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
            "{file (its id), server?, owner? (the server's address and the owner token you got when you published the file without the key; each defaults to what is recorded for one of your online files, see `collab.files`), action: links | createLink | revoke | share (others get new links and join codes) | unshare (the links and codes others got are revoked; the file stays yours on the server) | rename | remove (the file is deleted from the server and leaves your online files), access? (createLink: view (default) | edit), label? (createLink), link? (revoke: a link's id), name? (rename)} → {links: [{id, access, label, created}]} | {id, access, link (a new share link, shown once)} | {shared, code, viewCode, editLink, viewLink} | {done}",
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
