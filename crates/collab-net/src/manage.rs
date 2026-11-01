//! Requests to a server outside a live session, each on a connection of its own answered in
//! order: managing its files with the server's key ([`Manage`], [`request`]; the app publishes
//! through it, and the server's command line uses it to reach a running server), and using a
//! file's team library with one of the file's share links ([`OpenLibrary`], [`library`]), and its
//! plugin directory ([`OpenPlugins`], [`plugins`]: anyone browses, the key publishes).

use std::time::Duration;

use figcraft_collab::message::{
    self, Admin, AdminReply, ClientMsg, HostInfo, LibraryReply, LibraryReq, Manage, OpenHosting, OpenLibrary, OpenPlugins, PROTOCOL, PluginReply,
    PluginReq, ServerMsg,
};
use figcraft_engine::live::Shared;
use tungstenite::{Bytes, Message, WebSocket};

use crate::invite::{Endpoint, Invite, Lookup};
use crate::join::ask;
use crate::net::{Duplex, NetLimits, config, open};
pub(crate) use crate::requests::REFUSED;
use crate::requests::{code_requests, comment_request, hosting_error, link_requests, made_comment, made_links, shared};

type Ws = WebSocket<Duplex>;

/// Send `requests` to the server at `server` with its `key`; the replies, in order. Fails when the
/// server can't be reached, refuses the key or closes the connection (a request it can't do is a
/// [`AdminReply::Failed`] reply, not an error). Never echoes the key.
pub fn request(server: &Endpoint, key: &str, requests: Vec<Admin>, limits: &NetLimits, timeout: Duration) -> Result<Vec<AdminReply>, String> {
    let admin = |m: ServerMsg| match m {
        ServerMsg::Admin(r) => Some(*r),
        _ => None,
    };
    let manage = Manage { protocol: PROTOCOL, key: key.to_string(), client_version: message::client_version() };
    let mut ws = connect(server, limits, timeout)?;
    send(&mut ws, &ClientMsg::Manage(manage))?;
    match reply(&mut ws, limits, admin)? {
        AdminReply::Ready => {}
        other => return Err(format!("the server answered {other:?} instead of letting this connection manage files")),
    }
    let mut replies = Vec::with_capacity(requests.len());
    for r in requests {
        send(&mut ws, &ClientMsg::Admin(r))?;
        replies.push(reply(&mut ws, limits, admin)?);
    }
    close(ws);
    Ok(replies)
}

/// Ask the server at `server` about its hosting, with a file's owner token when `file` and `owner`
/// aren't empty; what it says, and its replies to `requests` (publishing without the key, managing
/// that file), in order. Fails when the server can't be reached or refuses (a wrong owner token, a
/// blocked address); a server older than open hosting can't read the request, and is reported as
/// taking files only with its key. Never echoes the token.
pub fn hosting(
    server: &Endpoint,
    file: &str,
    owner: &str,
    requests: Vec<Admin>,
    limits: &NetLimits,
    timeout: Duration,
) -> Result<(HostInfo, Vec<AdminReply>), String> {
    let admin = |m: ServerMsg| match m {
        ServerMsg::Admin(r) => Some(*r),
        _ => None,
    };
    let open = OpenHosting { protocol: PROTOCOL, client_version: message::client_version(), file: file.to_string(), owner: owner.to_string() };
    let mut ws = connect(server, limits, timeout)?;
    send(&mut ws, &ClientMsg::OpenHosting(open))?;
    let info = reply(&mut ws, limits, |m| match m {
        ServerMsg::Hosting(i) => Some(*i),
        _ => None,
    })
    .map_err(|e| hosting_error(server, e))?;
    let mut replies = Vec::with_capacity(requests.len());
    for r in requests {
        send(&mut ws, &ClientMsg::Admin(r))?;
        replies.push(reply(&mut ws, limits, admin)?);
    }
    close(ws);
    Ok((info, replies))
}

/// Ask `invite`'s server what `lookup` stands for: which file a join code opens, or the team link
/// of `invite.file`; the file's id and the link's token. Fails with the server's own words when it
/// knows no such code or keeps no such file on its team list (or the address tried too many: the
/// error starts with [`REFUSED`]); a server older than join codes (or team lists) can't read the
/// question ("couldn't be read").
pub fn resolve(invite: &Invite, lookup: &Lookup, limits: &NetLimits, timeout: Duration) -> Result<(String, String), String> {
    let ask = ask(invite, lookup);
    let mut ws = connect(&invite.server, limits, timeout)?;
    send(&mut ws, &ask)?;
    let answer = reply(&mut ws, limits, |m| match m {
        ServerMsg::Resolved { file, token } => Some((file, token)),
        _ => None,
    })?;
    close(ws);
    Ok(answer)
}

/// Change who else gets into `file`, as whoever manages it (with the server's `key`, else with the
/// file's `owner` token): revoke the links with the ids in `revoke` (whoever joined with them is
/// disconnected, and their join codes stop working; one already gone is fine), then, to `create`,
/// make a new edit link, a new view link and a new comment link and ask for their join codes: what
/// others join with. A server older than join codes makes the links without codes, and one older
/// than comment links makes no comment link.
pub fn share(
    server: &Endpoint,
    key: Option<&str>,
    file: &str,
    owner: &str,
    revoke: &[String],
    create: bool,
    limits: &NetLimits,
    timeout: Duration,
) -> Result<Option<Shared>, String> {
    let ask = |requests: Vec<Admin>| match key {
        Some(key) => request(server, key, requests, limits, timeout),
        None => hosting(server, file, owner, requests, limits, timeout).map(|(_, replies)| replies),
    };
    let Some(mut links) = made_links(ask(link_requests(file, revoke, create))?, create)? else { return Ok(None) };
    // The comment link on a connection of its own: a server from before comment links can't read
    // the request and closes it (the file is shared without one then).
    let comment = ask(comment_request(file)).map_err(|e| log::info!("live session: no comment link from {}: {e}", server.display()));
    links.comment = made_comment(comment.unwrap_or_default());
    // Their codes, on a connection of its own: a server from before join codes can't read the
    // request and closes it (the links are shared without codes then).
    let codes =
        ask(code_requests(file, &links)).map_err(|e| log::info!("live session: no join codes from {}: {e}", server.display())).unwrap_or_default();
    Ok(Some(shared(server, file, links, codes)))
}

/// Send one library request about the file behind share link `link`; the reply. Fails when the
/// link isn't one, the server can't be reached or refuses the link (a request it can't do is a
/// [`LibraryReply::Failed`] reply). Never echoes the link.
pub fn library(link: &str, req: LibraryReq, limits: &NetLimits, timeout: Duration) -> Result<LibraryReply, String> {
    let lib = |m: ServerMsg| match m {
        ServerMsg::Library(r) => Some(*r),
        _ => None,
    };
    let invite = Invite::parse(link)?;
    let open = OpenLibrary { protocol: PROTOCOL, file: invite.file, token: invite.token, client_version: message::client_version() };
    let mut ws = connect(&invite.server, limits, timeout)?;
    send(&mut ws, &ClientMsg::OpenLibrary(open))?;
    match reply(&mut ws, limits, lib)? {
        LibraryReply::Ready { .. } => {}
        other => return Err(format!("the server answered {other:?} instead of opening the library")),
    }
    send(&mut ws, &ClientMsg::Library(req))?;
    let r = reply(&mut ws, limits, lib)?;
    close(ws);
    Ok(r)
}

/// Send one request to the plugin directory of the server at `server`, with its `key` (empty: only
/// browsing); the reply. Fails when the server can't be reached, refuses the key or has no
/// directory (an older server); a request it can't do is a [`PluginReply::Failed`] reply. Never
/// echoes the key.
pub fn plugins(server: &Endpoint, key: &str, req: PluginReq, limits: &NetLimits, timeout: Duration) -> Result<PluginReply, String> {
    let pick = |m: ServerMsg| match m {
        ServerMsg::Plugins(r) => Some(*r),
        _ => None,
    };
    let open = OpenPlugins { protocol: PROTOCOL, key: key.to_string(), client_version: env!("CARGO_PKG_VERSION").to_string() };
    let mut ws = connect(server, limits, timeout)?;
    send(&mut ws, &ClientMsg::OpenPlugins(open))?;
    match reply(&mut ws, limits, pick) {
        Ok(PluginReply::Ready { .. }) => {}
        // An older server can't read the request at all.
        Err(e) if e.contains("couldn't be read") => return Err(format!("{} has no plugin directory (update the server)", server.display())),
        Err(e) => return Err(e),
        Ok(other) => return Err(format!("the server answered {other:?} instead of opening its plugin directory")),
    }
    send(&mut ws, &ClientMsg::Plugins(req))?;
    let r = reply(&mut ws, limits, pick)?;
    close(ws);
    Ok(r)
}

/// A WebSocket to `server` that waits for answers up to the request-reply timeout (a large upload
/// takes the server a while to keep).
fn connect(server: &Endpoint, limits: &NetLimits, timeout: Duration) -> Result<Ws, String> {
    let (stream, _, _) = open(server, timeout, limits)?;
    let (ws, _) = tungstenite::client::client_with_config(server.url(), stream, Some(config(limits.max_server_message)))
        .map_err(|e| format!("{} doesn't answer as a FigCraft server: {e}", server.display()))?;
    // A publish uploads a whole file; a large project on a slow link can take several minutes to
    // receive and answer. The wait uses `request_reply_timeout`, not the session `idle_timeout`.
    ws.get_ref().set_read_timeout(Some(limits.request_reply_timeout)).map_err(|e| e.to_string())?;
    Ok(ws)
}

/// How long `close` waits for the server's close.
const CLOSE_DRAIN: Duration = Duration::from_secs(2);

/// Close politely: read until the server's close arrives (or the socket ends), so it isn't lost,
/// but no longer than [`CLOSE_DRAIN`] (a server that keeps pinging, or goes quiet, would pin the
/// loop).
fn close(mut ws: Ws) {
    let _ = ws.close(None);
    ws.get_mut().read_until(Some(std::time::Instant::now() + CLOSE_DRAIN));
    while ws.read().is_ok() {}
}

fn send(ws: &mut Ws, msg: &ClientMsg) -> Result<(), String> {
    let bytes = message::encode(msg).map_err(|e| e.to_string())?;
    ws.send(Message::Binary(Bytes::from(bytes))).map_err(|e| format!("the connection to the server dropped ({e})"))
}

/// The next answer `pick` takes, or why there is none.
fn reply<T>(ws: &mut Ws, limits: &NetLimits, pick: impl Fn(ServerMsg) -> Option<T>) -> Result<T, String> {
    loop {
        return match ws.read() {
            Ok(Message::Binary(b)) => match message::decode::<ServerMsg>(&b, limits.max_server_message) {
                Ok(ServerMsg::Refused { reason }) => Err(format!("{REFUSED}{reason}")),
                Ok(ServerMsg::Pong) => continue,
                Ok(m) => pick(m).ok_or_else(|| "the server sent something other than an answer".into()),
                Err(e) => Err(format!("the server sent a message this version can't read ({e})")),
            },
            Ok(Message::Close(f)) => Err(match f.map(|f| f.reason.to_string()).filter(|r| !r.is_empty()) {
                Some(r) => format!("the server closed the connection: {r}"),
                None => "the server closed the connection".into(),
            }),
            Ok(Message::Text(_)) => Err("the server sent a text message".into()),
            Ok(_) => continue,
            Err(e) => Err(format!("no answer from the server ({e})")),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether `close` returns within 3 s against a server that never answers the close and does
    /// `serve` instead, with reads waiting up to `idle`.
    fn closes_in_time(idle: Duration, serve: impl FnOnce(WebSocket<std::net::TcpStream>) + Send + 'static) -> bool {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            if let Ok(ws) = tungstenite::accept(s) {
                serve(ws);
            }
        });
        let limits = NetLimits { idle_timeout: idle, ..NetLimits::default() };
        let server = Endpoint { host: addr.to_string(), path: "/".into(), secure: false };
        let ws = connect(&server, &limits, Duration::from_secs(2)).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            close(ws);
            let _ = tx.send(());
        });
        rx.recv_timeout(Duration::from_secs(3)).is_ok()
    }

    /// A server that keeps pinging and never answers the close must not pin `close` forever.
    #[test]
    fn close_returns_against_a_server_that_keeps_pinging() {
        let pinging = closes_in_time(Duration::from_millis(300), |mut ws| {
            while ws.send(Message::Ping(Bytes::new())).is_ok() {
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        assert!(pinging, "close() kept draining a pinging server");
    }

    /// Nor one that goes quiet, however long the idle timeout.
    #[test]
    fn close_returns_against_a_silent_server() {
        let silent = closes_in_time(Duration::from_secs(10), |ws| {
            std::thread::sleep(Duration::from_secs(5));
            drop(ws);
        });
        assert!(silent, "close() waited out the idle timeout");
    }
}
