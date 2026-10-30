//! Joining a session: the client end of the transport, with reconnect.
//!
//! A supervisor thread connects, says hello, then writes (presence throttled to about 30 a
//! second, latest wins; a ping every keep-alive interval) while a reader thread takes in what the
//! host sends.
//! When the connection drops after having been live, it reconnects with growing pauses; a
//! refusal or the host ending the session is final.
//!
//! Publishing ([`Guest::publish`]) first uploads the document to a server over a managing
//! connection, then joins the new file with its edit link on the same thread.

use std::io;
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use figcraft_collab::message::{self, ClientMsg, HostInfo, Presence, ServerMsg};
use figcraft_doc::Document;
use figcraft_engine::live::{Event, Published, Status};
use tungstenite::Message;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::{CloseFrame, Role as WsRole, WebSocket};

use crate::inbox::Inbox;
use crate::invite::{Endpoint, Invite};
pub use crate::join::GuestConfig;
use crate::join::{Next, Outcome, Resuming, Seen, Taken, after, closed, hello, resolved, take, too_large, unresolved};
use crate::manage;
use crate::net::{Duplex, NetLimits, config, shutdown};
use crate::requests;
pub use crate::requests::Publish;
use crate::throttle::{PRESENCE_INTERVAL, Throttle, earliest};

/// A joined (or joining) session. Dropping it leaves.
pub struct Guest {
    cmd: Sender<Cmd>,
    thread: Option<JoinHandle<()>>,
}

enum Cmd {
    Fetch(u64, figcraft_assets::AssetRef),
    AssetDone(u64),
    Presence(Presence),
    /// For the server, in order (document changes).
    Send(ClientMsg),
    Leave,
    /// The reader of connection `epoch` got its welcome.
    Welcomed(u64),
    /// Connection `epoch` ended.
    Lost(u64, Outcome),
}

impl Guest {
    /// Start joining (returns at once; progress arrives in `inbox`).
    pub fn start(mut cfg: GuestConfig, inbox: Inbox) -> io::Result<Guest> {
        let (tx, rx) = channel();
        let me = tx.clone();
        let thread = std::thread::Builder::new().name("live-guest".into()).spawn(move || {
            // A join code: the server says which file it opens, then the join goes on as with a link.
            if let Some(code) = cfg.code.clone() {
                inbox.push(Event::Status(Status::Connecting));
                let asked =
                    manage::resolve(&cfg.invite.server, &code, &cfg.limits, cfg.connect_timeout).map_err(|e| match e.strip_prefix(manage::REFUSED) {
                        Some(why) => Outcome::Refused(why.to_string()),
                        None => Outcome::Retry(e),
                    });
                if let Err(o) = asked.and_then(|(file, token)| resolved(&mut cfg, file, token)) {
                    if let Next::Stop(Some(e)) = after(unresolved(&cfg, o), &cfg, false, &mut 0, &inbox) {
                        inbox.push(e);
                    }
                    return;
                }
            }
            supervise(&cfg, &inbox, &rx, &me);
        })?;
        Ok(Guest { cmd: tx, thread: Some(thread) })
    }

    /// Upload `job.doc` to the server as a new file, then join it with its edit link (returns at
    /// once: [`Event::Published`] with the file's links, then the session, arrive in `inbox`).
    /// `cfg` says how to join; its invite is replaced by the new file's.
    pub fn publish(job: Publish, cfg: GuestConfig, inbox: Inbox) -> io::Result<Guest> {
        let (tx, rx) = channel();
        let me = tx.clone();
        let thread = std::thread::Builder::new().name("live-publish".into()).spawn(move || {
            inbox.push(Event::Status(Status::Connecting));
            let (cfg, published, hosting) = match upload(&job, cfg) {
                Ok(done) => done,
                Err(e) => return inbox.push(Event::Status(Status::Failed { reason: format!("Couldn't publish to the server: {e}") })),
            };
            // Before anyone hears the file is there, so what others join with shows with it. The
            // file's first links stay its owner's (this app joins with one): others get their own,
            // which stopping takes away again.
            let shared = job.share.then(|| {
                let owner = published.owner.as_deref().unwrap_or_default();
                let made = manage::share(&job.server, job.key.as_deref(), &published.file, owner, &[], true, &cfg.limits, cfg.connect_timeout);
                made.and_then(|s| s.ok_or_else(|| "the server didn't make the links".to_string()))
            });
            if let Some(h) = hosting {
                inbox.push(Event::Hosting(h));
            }
            inbox.push(Event::Published(published));
            if let Some(shared) = shared {
                inbox.push(Event::Shared(shared));
            }
            // Left while uploading: the file stays on the server, nobody joins it. Anything else
            // that came meanwhile (presence) waits for the session.
            let mut waiting = Vec::new();
            loop {
                match rx.try_recv() {
                    Ok(Cmd::Leave) | Err(TryRecvError::Disconnected) => return,
                    Ok(c) => waiting.push(c),
                    Err(TryRecvError::Empty) => break,
                }
            }
            for c in waiting {
                let _ = me.send(c);
            }
            supervise(&cfg, &inbox, &rx, &me);
        })?;
        Ok(Guest { cmd: tx, thread: Some(thread) })
    }

    /// Your latest presence.
    pub fn presence(&self, p: Presence) {
        let _ = self.cmd.send(Cmd::Presence(p));
    }

    /// A message from the sync client (sent once the connection is welcomed; dropped when it
    /// breaks: the next welcome starts over).
    pub fn send(&self, msg: ClientMsg) {
        let _ = self.cmd.send(Cmd::Send(msg));
    }

    /// Leave (returns at once: a connection attempt in progress ends on its own).
    pub fn leave(&mut self) {
        let _ = self.cmd.send(Cmd::Leave);
    }

    /// The connection thread has ended.
    pub fn finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }
}

impl Drop for Guest {
    fn drop(&mut self) {
        self.leave();
        if let Some(t) = self.thread.take()
            && t.is_finished()
        {
            let _ = t.join();
        }
    }
}

/// A connected socket: reader and writer ends.
struct Conn {
    reader: WebSocket<Duplex>,
    writer: WebSocket<Duplex>,
    sock: TcpStream,
}

/// Keep `doc` on `server` as a new file named `name`, with the server's `key` (or, without one, as
/// anyone may on a server open to the public): its edit link, where it is with its links (and its
/// owner token without a key), and what the server says about its hosting (without a key).
pub(crate) fn upload_file(
    server: &Endpoint,
    key: Option<&str>,
    name: &str,
    doc: &Document,
    limits: &NetLimits,
    timeout: Duration,
) -> Result<(Invite, Published, Option<HostInfo>), String> {
    let publish = vec![requests::upload(name, doc)?];
    let (replies, hosting) = match key {
        Some(key) => (manage::request(server, key, publish, limits, timeout)?, None),
        None => {
            let (info, replies) = manage::hosting(server, "", "", publish, limits, timeout)?;
            (replies, Some(info))
        }
    };
    let (edit, published) = requests::uploaded(server, replies.into_iter().next())?;
    Ok((edit, published, hosting))
}

/// Upload the document; the join settings for the new file, its links and the server's hosting.
fn upload(job: &Publish, mut cfg: GuestConfig) -> Result<(GuestConfig, Published, Option<HostInfo>), String> {
    let (edit, published, hosting) = upload_file(&job.server, job.key.as_deref(), &job.name, &job.doc, &cfg.limits, cfg.connect_timeout)?;
    cfg.invite = edit;
    Ok((cfg, published, hosting))
}

fn connect(cfg: &GuestConfig) -> Result<Conn, String> {
    let (reader, wr, sock) = crate::net::connect_websocket(&cfg.invite.server, cfg.connect_timeout, &cfg.limits)?;
    let writer = WebSocket::from_raw_socket(wr, WsRole::Client, Some(config(cfg.limits.max_client_message)));
    reader.get_ref().set_read_timeout(Some(cfg.limits.idle_timeout)).map_err(|e| e.to_string())?;
    Ok(Conn { reader, writer, sock })
}

fn supervise(cfg: &GuestConfig, inbox: &Inbox, rx: &Receiver<Cmd>, me: &Sender<Cmd>) {
    let resume: Arc<Resuming> = Arc::new(std::sync::Mutex::new(cfg.resume));
    let mut attempt = 0u32;
    let mut was_live = false;
    let mut pending: Option<Presence> = None;
    let mut epoch = 0u64;
    inbox.push(Event::Status(Status::Connecting));
    loop {
        epoch += 1;
        let outcome = match connect(cfg) {
            Ok(c) => session(c, cfg, inbox, rx, me, epoch, &resume, &mut pending, &mut was_live, &mut attempt),
            Err(e) => Outcome::Retry(e),
        };
        match after(outcome, cfg, was_live, &mut attempt, inbox) {
            Next::Stop(e) => {
                if let Some(e) = e {
                    inbox.push(e);
                }
                return;
            }
            Next::Reconnect(pause) => {
                let until = Instant::now() + pause;
                loop {
                    match rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
                        Ok(Cmd::Leave) | Err(RecvTimeoutError::Disconnected) => return,
                        Ok(Cmd::Presence(p)) => pending = Some(p),
                        Ok(Cmd::Fetch(_, asset)) => figcraft_assets::cache::failed(&asset, "the image's session is reconnecting"),
                        // From the old connection's client: the next welcome starts over.
                        Ok(_) => {}
                        Err(RecvTimeoutError::Timeout) => break,
                    }
                }
            }
        }
    }
}

/// One connection: say hello, then write until it ends.
fn session(
    c: Conn,
    cfg: &GuestConfig,
    inbox: &Inbox,
    rx: &Receiver<Cmd>,
    me: &Sender<Cmd>,
    epoch: u64,
    resume: &Arc<Resuming>,
    pending: &mut Option<Presence>,
    was_live: &mut bool,
    attempt: &mut u32,
) -> Outcome {
    let Conn { reader, mut writer, sock } = c;
    let requests: Requests = Arc::default();
    let reader = {
        let ctx = ReaderCtx { inbox: inbox.clone(), tx: me.clone(), epoch, cfg: cfg.clone(), resume: resume.clone(), requests: requests.clone() };
        match std::thread::Builder::new().name("live-guest-read".into()).spawn(move || read_loop(reader, &ctx)) {
            Ok(t) => t,
            Err(e) => {
                shutdown(&sock);
                return Outcome::Retry(e.to_string());
            }
        }
    };
    // A ping every keep-alive interval, whatever else is sent: presence gets no answer, and the
    // reader counts only what the host sends.
    let mut last_ping = Instant::now();
    let hello = hello(cfg, resume);
    let mut welcomed = false;
    let mut throttle = Throttle::new(PRESENCE_INTERVAL);
    let mut asset_queue = std::collections::VecDeque::new();
    let scope = session_scope(cfg);
    let outcome = if send(&mut writer, &hello, &requests) {
        loop {
            let ping_at = last_ping + cfg.limits.keepalive;
            let wake = earliest([throttle.due(), Some(ping_at)]).unwrap_or(ping_at);
            let mut out = None;
            let mut now_send = Vec::new();
            match rx.recv_timeout(wake.saturating_duration_since(Instant::now())) {
                Ok(Cmd::Fetch(e, asset)) if e == epoch && writer.get_ref().assets => {
                    if figcraft_assets::cache::load(&asset).is_ok() {
                        continue;
                    }
                    if !requests.lock().unwrap_or_else(std::sync::PoisonError::into_inner).contains(&asset) && !asset_queue.contains(&asset) {
                        if asset_queue.len() < 512 {
                            asset_queue.push_back(asset);
                        } else {
                            figcraft_assets::cache::failed(&asset, "the image queue is full; try again in a moment");
                        }
                    }
                }
                Ok(Cmd::AssetDone(e)) if e == epoch => {}
                Ok(Cmd::Fetch(_, asset)) => figcraft_assets::cache::failed(&asset, "the image's connection changed or does not support downloads"),
                Ok(Cmd::AssetDone(_)) => {}
                Ok(Cmd::Send(m)) if welcomed => now_send.push(m),
                // Made for the previous connection (the sync client sends again what the server
                // didn't get once it has caught up with this connection's welcome).
                Ok(Cmd::Send(_)) => {}
                Ok(Cmd::Presence(p)) if welcomed => out = throttle.offer(p, Instant::now()),
                Ok(Cmd::Presence(p)) => *pending = Some(p),
                Ok(Cmd::Welcomed(e)) if e == epoch => {
                    welcomed = true;
                    *was_live = true;
                    *attempt = 0;
                    out = pending.take().and_then(|p| throttle.offer(p, Instant::now()));
                }
                Ok(Cmd::Lost(e, o)) if e == epoch => break o,
                Ok(Cmd::Leave) | Err(RecvTimeoutError::Disconnected) => {
                    let _ = writer.close(Some(CloseFrame { code: CloseCode::Normal, reason: "".into() }));
                    break Outcome::Left;
                }
                // From an earlier connection.
                Ok(Cmd::Welcomed(_) | Cmd::Lost(..)) => {}
                Err(RecvTimeoutError::Timeout) => {}
            }
            if !now_send.iter().all(|m| send_edit(&mut writer, m, &requests, inbox, &scope)) {
                break Outcome::Retry("the connection dropped".into());
            }
            while requests.lock().unwrap_or_else(std::sync::PoisonError::into_inner).len() < 2 && welcomed {
                let Some(asset) = asset_queue.pop_front() else { break };
                if figcraft_assets::cache::load(&asset).is_ok() {
                    continue;
                }
                requests.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(asset.clone());
                if !send(&mut writer, &ClientMsg::FetchAsset(asset), &requests) {
                    break;
                }
            }
            let now = Instant::now();
            let out = out.or_else(|| throttle.take_due(now));
            let ok = out.is_none_or(|p| send(&mut writer, &ClientMsg::Presence(p), &requests));
            let ok = ok
                && (now < last_ping + cfg.limits.keepalive || {
                    last_ping = now;
                    send(&mut writer, &ClientMsg::Ping, &requests)
                });
            if !ok {
                break Outcome::Retry("the connection dropped".into());
            }
        }
    } else {
        Outcome::Retry("the connection dropped".into())
    };
    shutdown(&sock);
    let _ = reader.join();
    for asset in asset_queue {
        figcraft_assets::cache::failed(&asset, "the image's session disconnected");
    }
    outcome
}

/// Send one message; false when the connection is gone.
type Requests = Arc<std::sync::Mutex<std::collections::BTreeSet<figcraft_assets::AssetRef>>>;

fn send_edit(ws: &mut WebSocket<Duplex>, msg: &ClientMsg, requests: &Requests, inbox: &Inbox, scope: &str) -> bool {
    match send_result(ws, msg, requests, Some(scope)) {
        Ok(()) => true,
        Err(SendError::Unavailable(reason)) => {
            if let ClientMsg::Changes { batch_id, .. } = msg {
                inbox.push(Event::Sync(ServerMsg::Reject { batch_id: *batch_id, reason, fixes: Vec::new() }));
                true
            } else {
                false
            }
        }
        Err(SendError::Disconnected) => false,
    }
}

fn send(ws: &mut WebSocket<Duplex>, msg: &ClientMsg, requests: &Requests) -> bool {
    send_result(ws, msg, requests, None).is_ok()
}

enum SendError {
    Disconnected,
    Unavailable(String),
}

fn session_scope(cfg: &GuestConfig) -> String {
    figcraft_assets::file_scope(&cfg.invite.server.url(), &cfg.invite.file)
}

fn send_result(ws: &mut WebSocket<Duplex>, msg: &ClientMsg, requests: &Requests, scope: Option<&str>) -> Result<(), SendError> {
    if ws.get_ref().assets
        && let Some(scope) = scope
    {
        crate::assets_wire::remember_uploads(msg, scope).map_err(SendError::Unavailable)?;
    }
    let missing = crate::assets_wire::missing(msg).map_err(SendError::Unavailable)?;
    // Undo can reference an image the viewport never needed. Request it directly before
    // waiting: queueing it to this same writer thread would deadlock the upload.
    for asset in missing {
        if !ws.get_ref().assets {
            return Err(SendError::Unavailable("this server cannot supply a deferred image; reconnect with an updated server".into()));
        }
        let asset_error =
            |reason| if figcraft_assets::cache::is_unavailable(&asset) { SendError::Unavailable(reason) } else { SendError::Disconnected };
        figcraft_assets::cache::enqueue(&asset).map_err(asset_error)?;
        if requests.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(asset.clone()) {
            let bytes = crate::assets_wire::client(&ClientMsg::FetchAsset(asset.clone()), true).map_err(SendError::Unavailable)?;
            crate::net::send_binary(ws, &bytes).map_err(|_| SendError::Disconnected)?;
        }
        figcraft_assets::cache::require(&asset).map_err(asset_error)?;
    }
    let bytes = crate::assets_wire::client(msg, ws.get_ref().assets).map_err(SendError::Unavailable)?;
    crate::net::send_binary(ws, &bytes).map_err(|_| SendError::Disconnected)
}

struct ReaderCtx {
    requests: Requests,
    inbox: Inbox,
    tx: Sender<Cmd>,
    epoch: u64,
    cfg: GuestConfig,
    resume: Arc<Resuming>,
}

fn read_loop(mut ws: WebSocket<Duplex>, ctx: &ReaderCtx) {
    let inbox = ctx.inbox.clone();
    let cache_waker: Arc<dyn Fn() + Send + Sync> = Arc::new(move || inbox.push(Event::AssetsChanged));
    figcraft_assets::cache::watch(&cache_waker);
    let mut seen = Seen::default();
    let tx = ctx.tx.clone();
    let epoch = ctx.epoch;
    let loader: Arc<figcraft_assets::cache::Loader> = Arc::new(move |asset| tx.send(Cmd::Fetch(epoch, asset)).is_ok());
    let scope = session_scope(&ctx.cfg);
    figcraft_assets::cache::register_scope(&scope, &loader);
    let outcome = loop {
        match ws.read() {
            Ok(Message::Binary(b)) => match message::decode::<ServerMsg>(&b, ctx.cfg.limits.max_server_message) {
                Ok(mut m) => {
                    if let ServerMsg::Asset { mut asset, bytes, error } = m {
                        asset.scope = scope.clone();
                        if !ws.get_ref().assets || !ctx.requests.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&asset) {
                            break Outcome::Refused("the host sent an unrequested image".into());
                        }
                        match bytes {
                            Some(bytes) if error.is_none() => match figcraft_assets::cache::store(&asset, &bytes.0) {
                                Ok(()) => ctx.inbox.push(Event::AssetsChanged),
                                Err(reason) => {
                                    figcraft_assets::cache::failed(&asset, &reason);
                                    break Outcome::Refused(reason);
                                }
                            },
                            _ => figcraft_assets::cache::unavailable(&asset, error.as_deref().unwrap_or("the host has no bytes for this image")),
                        }
                        let _ = ctx.tx.send(Cmd::AssetDone(ctx.epoch));
                        continue;
                    }
                    if ws.get_ref().assets {
                        if let Err(reason) = crate::assets_wire::scope(&mut m, &scope) {
                            break Outcome::Refused(reason);
                        }
                        let assets = match crate::assets_wire::manifests(&m) {
                            Ok(assets) => assets,
                            Err(reason) => break Outcome::Refused(reason),
                        };
                        if assets.iter().any(|asset| !asset.valid()) || assets.len() > 20_000 {
                            break Outcome::Refused("the host's image manifest exceeds the receiving budget".into());
                        }
                        for asset in assets {
                            figcraft_assets::cache::register(&asset, &loader);
                        }
                    } else if let Err(reason) = crate::assets_wire::validate_legacy(&m) {
                        break Outcome::Refused(reason);
                    }
                    let welcomed = || {
                        let _ = ctx.tx.send(Cmd::Welcomed(ctx.epoch));
                    };
                    match take(m, &ctx.cfg, &ctx.inbox, &mut seen, &ctx.resume, &welcomed) {
                        Taken::Continue | Taken::Welcomed => {}
                        Taken::End(o) => break o,
                    }
                }
                Err(e) => break Outcome::Refused(format!("the host sent a message this version can't read ({e})")),
            },
            Ok(Message::Close(f)) => break f.map_or_else(|| closed(0, ""), |f| closed(f.code.into(), &f.reason)),
            Ok(Message::Text(_)) => break Outcome::Refused("the host sent a text message".into()),
            Ok(_) => {}
            Err(tungstenite::Error::Capacity(_)) => break too_large(&ctx.cfg.limits),
            Err(e) => break Outcome::Retry(e.to_string()),
        }
    };
    for asset in std::mem::take(&mut *ctx.requests.lock().unwrap_or_else(std::sync::PoisonError::into_inner)) {
        figcraft_assets::cache::failed(&asset, "the image's session disconnected");
    }
    let _ = ctx.tx.send(Cmd::Lost(ctx.epoch, outcome));
}

#[cfg(test)]
mod asset_request_tests {
    use super::*;

    #[test]
    fn late_asset_requests_fail_promptly_after_an_epoch_change_and_during_reconnect() {
        let (mut host, _host_probe, invite) = crate::loopback_tests::start();
        let mut probe = crate::loopback_tests::Probe::new();
        let mut cfg = GuestConfig::new(invite, "Late image requester".into());
        cfg.backoff = Duration::from_secs(5);
        let mut guest = Guest::start(cfg, probe.inbox.clone()).unwrap();
        probe.wait("welcome", |events| events.iter().any(|e| matches!(e, Event::Welcome { .. })));

        let check = |epoch, expected: &str| {
            let seed = crate::token::file_id().unwrap();
            let mut asset = figcraft_assets::AssetRef::of(seed.as_bytes());
            asset.scope = asset.sha256.clone();
            let tx = guest.cmd.clone();
            let loader: Arc<figcraft_assets::cache::Loader> = Arc::new(move |asset| tx.send(Cmd::Fetch(epoch, asset)).is_ok());
            figcraft_assets::cache::register(&asset, &loader);
            let start = Instant::now();
            let error = figcraft_assets::cache::require(&asset).unwrap_err();
            assert!(error.contains(expected), "{error}");
            assert!(start.elapsed() < Duration::from_secs(2), "a discarded request waited for the cache timeout");
        };
        // A callback retained from an older reader must release cache waiters.
        check(0, "connection changed");
        guest.cmd.send(Cmd::Lost(1, Outcome::Retry("synthetic disconnect".into()))).unwrap();
        probe.wait("reconnect pause", |events| events.iter().any(|e| matches!(e, Event::Status(Status::Reconnecting { .. }))));
        check(1, "reconnecting");
        guest.leave();
        host.stop();
    }
}
