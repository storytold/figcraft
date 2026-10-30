//! The hub every host runs, for one file (a session the app hosts) or many (`figcraft-server`):
//! a WebSocket listener, a reader and a writer thread per connection, and one hub thread that owns
//! a collaboration core [`Server`] per open file and routes its messages.
//!
//! What differs between the two lives behind a [`Backend`]: which hellos it admits and with what
//! access, where a file that isn't open comes from, what is kept of the changes (a server appends
//! them to its log; the app keeps nothing), and the managing, team library and plugin directory
//! connections a server answers. The person hosting an app session takes part through the hub directly (no
//! socket): their presence and edits go in with [`Hosting::presence`] and [`Hosting::send`], what
//! others do comes out through their [`Inbox`].
//!
//! On top of the core the hub hands out colours and default names per file, throttles presence to
//! about 30 updates a second per person (latest wins), tells a newcomer its own name and colour (a
//! `PeerJoined` of itself, after its welcome) and where everyone already is, and disconnects
//! clients that send too much, too fast, or can't keep up. Whatever the servers say goes out only
//! after the backend has made what changed durable ([`Backend::flush`], once per burst of
//! messages), so a confirmed edit is never lost to a crash.
//!
//! Every connection is known by its client's address ([`ip_key`]; behind the configured reverse
//! proxy, the one its `X-Forwarded-For` names): the transport caps connections per address and the
//! backend may refuse addresses or limit what they do. A plain `GET /healthz` on the same port is
//! answered `200 ok` for uptime checks.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv6Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError, channel, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use figcraft_collab::message::{
    self, Access, Admin, AdminReply, ClientMsg, Hello, HostInfo, LibraryReply, LibraryReq, Manage, OpenHosting, OpenLibrary, OpenPlugins, PROTOCOL,
    PluginReply, PluginReq, Presence, Resolve, ServerMsg, reads_hosting,
};
use figcraft_collab::{Outgoing, Server};
use figcraft_engine::live::{Event, MAX_NAME, PEOPLE, sanitize};
use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::{CloseFrame, Role, WebSocket};
use tungstenite::{Bytes, Message};

use crate::inbox::Inbox;
use crate::join::{ENDED, RESTARTING};
use crate::net::{Bucket, Duplex, Meter, NetLimits, Readers, config, shutdown};
use crate::throttle::{PRESENCE_INTERVAL, Throttle, earliest};

mod backend;
mod conn;
mod hosting;
mod limits;
mod queue;

pub use backend::{Backend, Closing, Handled, OpenFile};
pub use conn::{SIGNAL_BURST, SIGNAL_RATE};
use conn::{encode, reason, refuse, refused};
pub use hosting::Hosting;
pub(crate) use hosting::{Local, clean_name};
pub use limits::{Admission, Block, ip_blocks, ip_key};
pub use queue::wide_block;
#[cfg(test)]
use queue::{BlockUse, Blocks};
use queue::{Charges, HubIn, InFlight, Line, Out, Push};

/// Most messages the hub handles before it sends what they produced (bounds the delay a burst
/// adds).
const BURST: usize = 256;

/// What a connection is for.
enum Kind {
    /// In `file`'s session as client `id`, through link `link`.
    Joined { file: String, id: u32, link: Option<String> },
    /// Managing the server's files; `comments`: its client reads comment links.
    Managing { comments: bool },
    /// Using `file`'s team library through link `link` of `access` (revoking it disconnects).
    Library { file: String, access: Access, link: Option<String> },
    /// Using the plugin directory; `publisher`: with the server's key.
    Plugins { publisher: bool },
    /// Publishing without the key, or managing the file `owns` with its owner token; `comments`:
    /// its client reads comment links.
    Hosting { owns: Option<String>, comments: bool },
}

/// A connection whose first message the hub is looking at.
struct Pending {
    out: Line,
    sock: TcpStream,
    ip: IpAddr,
    /// Told when it is taken in.
    admitted: SyncSender<()>,
}

/// A connection the hub took in.
struct Conn {
    out: Line,
    sock: TcpStream,
    kind: Kind,
    /// Its client's address ([`ip_key`]).
    ip: IpAddr,
}

/// One open file's session.
struct Room {
    server: Server,
    /// Client id → connection.
    ids: BTreeMap<u32, u64>,
    throttles: BTreeMap<u32, Throttle<Presence>>,
    /// Everyone's latest presence (for newcomers).
    latest: BTreeMap<u32, Presence>,
    /// Default names handed out ("Guest 1", "Guest 2"…).
    guests: u32,
    /// The person hosting (app sessions): their client id and inbox.
    local: Option<(u32, Inbox)>,
}

impl Room {
    fn new(server: Server) -> Self {
        Self { server, ids: BTreeMap::new(), throttles: BTreeMap::new(), latest: BTreeMap::new(), guests: 0, local: None }
    }

    /// Nobody is in it.
    fn empty(&self) -> bool {
        self.ids.is_empty() && self.local.is_none()
    }

    /// What listings say about it.
    fn info(&self) -> OpenFile {
        OpenFile { people: u32::try_from(self.server.peers().count()).unwrap_or(u32::MAX), seq: self.server.seq() }
    }
}

/// Why a client of another protocol version is refused.
fn unsupported(protocol: u32) -> String {
    format!("protocol {protocol} isn't supported (this server speaks {PROTOCOL})")
}

/// What someone in a file is told when the link they joined with is revoked (its owner stopped
/// sharing): the session's end for them.
pub const STOPPED: &str = "Sharing was stopped: the link you joined with no longer works. Your copy stays open";

/// What someone removed from a file is told: the session's end for them (they keep their copy;
/// nothing retries).
pub const REMOVED: &str = "You were removed from this session. Your copy stays open";

/// Why a removed person's address can't join the file again.
const BARRED: &str = "you were removed from this session";

/// The most addresses one file keeps barred.
const MAX_BARRED: usize = 256;

/// What everyone in a file is told when its changes couldn't be saved (they reconnect).
const UNSAVED: &str = "The server couldn't save the latest changes; reconnecting";

/// Owns the open files' servers and routes their messages.
struct Hub<B> {
    in_flight: InFlight,
    backend: B,
    rooms: HashMap<String, Room>,
    conns: HashMap<u64, Conn>,
    /// The file the person hosting is in (app sessions).
    local: Option<String>,
    /// What the servers said, sent once the backend made what changed durable.
    outbox: Vec<(String, Outgoing)>,
    /// Addresses ([`ip_key`]) removed from each open file: they can't join it again until its
    /// session closes (the file is closed or removed; the entry goes with it).
    barred: HashMap<String, HashSet<IpAddr>>,
}

impl<B: Backend> Hub<B> {
    fn run(mut self, rx: &Receiver<HubIn>) {
        'run: loop {
            let due = earliest(self.rooms.values().flat_map(|r| r.throttles.values().map(Throttle::due)));
            let first = match due {
                Some(d) => match rx.recv_timeout(d.saturating_duration_since(Instant::now())) {
                    Ok(m) => Some(m),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => Some(HubIn::Stop),
                },
                None => Some(rx.recv().unwrap_or(HubIn::Stop)),
            };
            // A burst: handle what's waiting, then make it durable and answer.
            let mut next = first;
            let mut handled = 0;
            while let Some(m) = next.take() {
                if matches!(m, HubIn::Stop) {
                    break 'run;
                }
                self.handle(m);
                handled += 1;
                if handled < BURST {
                    next = rx.try_recv().ok();
                }
            }
            self.flush_due(Instant::now());
            self.send_out();
        }
        self.send_out();
        self.backend.stop(self.rooms.iter_mut().map(|(f, r)| (f.as_str(), &mut r.server)).collect());
        let (code, why) = match self.backend.closing() {
            Closing::Ended => (CloseCode::Away, ENDED),
            Closing::Restarting => (CloseCode::Restart, RESTARTING),
        };
        for c in self.conns.values() {
            if c.out.tx.try_send(Out::Close(code, why.into())).is_err() {
                shutdown(&c.sock);
            }
        }
    }

    fn handle(&mut self, m: HubIn) {
        match m {
            HubIn::Open { conn, ip, first, out, sock, bytes, admitted } => {
                self.in_flight.give(bytes);
                // The operator (with the server key) gets in from anywhere; everyone else is
                // checked (blocked addresses, connection rates).
                let operator = matches!(&first, ClientMsg::Manage(m) if self.backend.manage(&m.key).is_ok());
                if !operator && let Err(why) = self.backend.connect(ip) {
                    log::info!("event=refused ip={ip} reason={why:?}");
                    return refuse(&out, &why);
                }
                let c = Pending { out, sock, ip, admitted };
                match first {
                    ClientMsg::Hello(hello) => self.join(conn, hello, c),
                    ClientMsg::Manage(m) => self.manage(conn, &m, c),
                    ClientMsg::OpenLibrary(o) => self.open_library(conn, o, c),
                    ClientMsg::OpenPlugins(o) => self.open_plugins(conn, &o, c),
                    ClientMsg::OpenHosting(o) => self.open_hosting(conn, &o, c),
                    ClientMsg::Resolve(r) => self.resolve(&r, &c),
                    _ => refuse(&c.out, "say hello first"),
                }
            }
            HubIn::Msg { conn, msg, bytes } => {
                self.in_flight.give(bytes);
                let Some(c) = self.conns.get(&conn) else { return };
                let ip = c.ip;
                // The operator (with the server key) isn't held to the limits on addresses.
                if !matches!(c.kind, Kind::Managing { .. }) && !self.backend.message(ip, &msg) {
                    log::info!("event=refused ip={ip} reason=messages_per_address");
                    return self.kick(&[conn], "too many messages from your address: wait a minute and try again");
                }
                self.message(conn, ip, msg, bytes);
            }
            HubIn::Gone { conn } => self.gone(conn),
            HubIn::Local(p) | HubIn::LocalMsg(ClientMsg::Presence(p)) => {
                if let Some((file, id)) = self.local_id() {
                    self.presence(&file, id, p);
                }
            }
            HubIn::LocalMsg(msg) => {
                if let Some((file, id)) = self.local_id() {
                    self.client_message(&file, id, msg, None);
                }
            }
            HubIn::Remove(id) => {
                if let Some(file) = self.local.clone()
                    && let Err(why) = self.remove_peer(&file, id)
                {
                    log::info!("live session: couldn't remove client {id}: {why}");
                }
            }
            #[cfg(test)]
            HubIn::Cut => {
                for c in self.conns.values() {
                    shutdown(&c.sock);
                }
            }
            HubIn::Stop => {}
        }
    }

    /// A message on connection `conn` (from `ip`), by what the connection is for.
    fn message(&mut self, conn: u64, ip: IpAddr, msg: ClientMsg, bytes: usize) {
        match self.conns.get(&conn).map(|c| &c.kind) {
            Some(Kind::Joined { file, link, .. }) if matches!(msg, ClientMsg::Report { .. }) => {
                let (file, link) = (file.clone(), link.clone());
                let reason = if let ClientMsg::Report { reason } = &msg { reason.as_str() } else { "" };
                let error = self.backend.report(&file, link.as_deref(), ip, reason).err();
                self.answer(conn, &ServerMsg::Reported { error });
            }
            Some(Kind::Joined { file, id, .. }) => {
                let (file, id) = (file.clone(), *id);
                self.client_message(&file, id, msg, Some(bytes));
            }
            Some(Kind::Hosting { owns, .. }) => match msg {
                ClientMsg::Admin(Admin::RemovePeer { file, client }) => {
                    let reply = if owns.as_deref() == Some(file.as_str()) {
                        self.removed_reply(&file, client)
                    } else {
                        AdminReply::Failed { reason: "an owner token manages only its own file".into() }
                    };
                    self.answer(conn, &ServerMsg::Admin(Box::new(reply)));
                }
                ClientMsg::Admin(req) => {
                    let owns = owns.clone();
                    let rooms = &self.rooms;
                    let open = |file: &str| rooms.get(file).map(Room::info);
                    let handled = self.backend.hosted(ip, owns.as_deref(), req, &open);
                    self.handled(conn, handled);
                }
                ClientMsg::Ping => self.answer(conn, &ServerMsg::Pong),
                _ => self.kick(&[conn], "only publishing and managing your own files are spoken on this connection"),
            },
            Some(Kind::Managing { .. }) => match msg {
                ClientMsg::Admin(Admin::RemovePeer { file, client }) => {
                    let reply = self.removed_reply(&file, client);
                    self.answer(conn, &ServerMsg::Admin(Box::new(reply)));
                }
                ClientMsg::Admin(req) => self.admin(conn, ip, req),
                ClientMsg::Ping => self.answer(conn, &ServerMsg::Pong),
                _ => self.kick(&[conn], "only requests to manage files are spoken on this connection"),
            },
            Some(Kind::Library { file, access, .. }) => match msg {
                ClientMsg::Library(req) => {
                    let (file, access) = (file.clone(), *access);
                    let reply = self.backend.library(ip, &file, access, req);
                    self.answer(conn, &ServerMsg::Library(Box::new(reply)));
                }
                ClientMsg::Ping => self.answer(conn, &ServerMsg::Pong),
                _ => self.kick(&[conn], "only library requests are spoken on this connection"),
            },
            Some(Kind::Plugins { publisher }) => match msg {
                ClientMsg::Plugins(req) => {
                    let reply = self.backend.plugins(ip, *publisher, req);
                    self.answer(conn, &ServerMsg::Plugins(Box::new(reply)));
                }
                ClientMsg::Ping => self.answer(conn, &ServerMsg::Pong),
                _ => self.kick(&[conn], "only plugin directory requests are spoken on this connection"),
            },
            None => {}
        }
    }

    /// The hosting person's file and client id.
    fn local_id(&self) -> Option<(String, u32)> {
        let file = self.local.clone()?;
        let id = self.rooms.get(&file)?.local.as_ref()?.0;
        Some((file, id))
    }

    /// A message from client `id` of `file`.
    /// A message from client `id` of `file`; `bytes`: its size on the wire (a socket's).
    fn client_message(&mut self, file: &str, id: u32, msg: ClientMsg, bytes: Option<usize>) {
        if let ClientMsg::Presence(p) = msg {
            return self.presence(file, id, p);
        }
        // A batch the file has no room for is refused before the session takes it (the client
        // rolls it back with the reason).
        if let (ClientMsg::Changes { batch_id, .. }, Some(n)) = (&msg, bytes)
            && let Some(room) = self.rooms.get(file)
            && let Err(reason) = self.backend.room(file, &room.server, n)
        {
            let reject = ServerMsg::Reject { batch_id: *batch_id, reason, fixes: Vec::new() };
            return self.queue(file, vec![Outgoing::To(id, reject)]);
        }
        let Some(room) = self.rooms.get_mut(file) else { return };
        let outs = room.server.receive(id, msg);
        self.backend.changed(file, &mut room.server);
        self.queue(file, outs);
    }

    fn queue(&mut self, file: &str, outs: Vec<Outgoing>) {
        self.outbox.extend(outs.into_iter().map(|o| (file.to_string(), o)));
    }

    fn join(&mut self, conn: u64, mut hello: Hello, c: Pending) {
        let admission = match self.backend.admit(&hello.file, &hello.token) {
            Ok(a) => a,
            Err(why) => return refuse(&c.out, &why),
        };
        if self.barred.get(&hello.file).is_some_and(|b| b.contains(&c.ip)) {
            log::info!("event=refused ip={} reason=removed", c.ip);
            return refuse(&c.out, BARRED);
        }
        let hosting = reads_hosting(&hello.client_version);
        // The backend checked the secret: the sessions don't keep it.
        hello.token.clear();
        let file = hello.file.clone();
        if !self.rooms.contains_key(&file) {
            match self.backend.open(&file) {
                Ok(server) => {
                    self.rooms.insert(file.clone(), Room::new(server));
                }
                Err(why) => return refuse(&c.out, &why),
            }
        }
        let info = if hosting { self.backend.hosting(&file) } else { None };
        let Some(room) = self.rooms.get_mut(&file) else { return };
        // Colours: the first one nobody has.
        let taken: Vec<&str> = room.server.peers().map(|p| p.color.as_str()).collect();
        let color = PEOPLE.iter().find(|c| !taken.contains(c)).copied().unwrap_or(PEOPLE[taken.len() % PEOPLE.len()]);
        hello.color = color.to_string();
        hello.name = match clean_name(&hello.name) {
            n if n.is_empty() => {
                room.guests += 1;
                format!("Guest {}", room.guests)
            }
            n => n,
        };
        match room.server.join_as(hello, admission.access) {
            Ok((id, outs)) => {
                // A guest back after a drop keeps its id: its old connection, if the hub hasn't
                // noticed it's gone yet, is closed without the guest leaving.
                let old = room.ids.insert(id, conn).filter(|old| *old != conn);
                self.backend.changed(&file, &mut room.server);
                // The newcomer learns how it is shown, then where everyone is.
                let mut more = outs;
                if let Some(me) = room.server.peers().find(|p| p.client_id == id) {
                    more.push(Outgoing::To(id, ServerMsg::PeerJoined(me.clone())));
                }
                more.extend(room.latest.iter().map(|(&from, p)| Outgoing::To(id, ServerMsg::Presence { client_id: from, presence: p.clone() })));
                // A client that reads it hears about the server's hosting (operator, terms, expiry).
                more.extend(info.map(|i| Outgoing::To(id, ServerMsg::Hosting(Box::new(i)))));
                if let Some(c) = old.and_then(|old| self.conns.remove(&old)) {
                    shutdown(&c.sock);
                }
                let _ = c.admitted.try_send(());
                self.conns
                    .insert(conn, Conn { out: c.out, sock: c.sock, kind: Kind::Joined { file: file.clone(), id, link: admission.link }, ip: c.ip });
                self.queue(&file, more);
            }
            Err(ServerMsg::Refused { reason }) => {
                refuse(&c.out, &reason);
                self.close_if_empty(&file);
            }
            Err(_) => {
                refuse(&c.out, "refused");
                self.close_if_empty(&file);
            }
        }
    }

    /// Remove client `id` from `file`: they are told the session is over for them and
    /// disconnected, and their address can't join the file again while its session lasts. The
    /// person hosting can't be removed.
    fn remove_peer(&mut self, file: &str, id: u32) -> Result<(), String> {
        let room = self.rooms.get(file).ok_or("nobody is in this file")?;
        if room.local.as_ref().is_some_and(|(local, _)| *local == id) {
            return Err("the person hosting can't be removed".into());
        }
        let conn = *room.ids.get(&id).ok_or_else(|| format!("nobody with client id {id} is in this file"))?;
        let c = self.conns.get(&conn).ok_or_else(|| format!("nobody with client id {id} is in this file"))?;
        let barred = self.barred.entry(file.to_string()).or_default();
        if barred.len() < MAX_BARRED {
            barred.insert(c.ip);
        }
        log::info!("event=removed file={file} client={id} ip={}", c.ip);
        if c.out.tx.try_send(Out::Close(CloseCode::Away, REMOVED.into())).is_err() {
            shutdown(&c.sock);
        }
        self.gone(conn);
        Ok(())
    }

    /// [`remove_peer`](Self::remove_peer) as a managing connection's answer.
    fn removed_reply(&mut self, file: &str, id: u32) -> AdminReply {
        match self.remove_peer(file, id) {
            Ok(()) => AdminReply::Done,
            Err(reason) => AdminReply::Failed { reason },
        }
    }

    /// Take in connection `c` as `kind` and send it `first`.
    fn take_in(&mut self, conn: u64, c: Pending, kind: Kind, first: &ServerMsg) {
        let _ = c.admitted.try_send(());
        self.conns.insert(conn, Conn { out: c.out, sock: c.sock, kind, ip: c.ip });
        self.answer(conn, first);
    }

    fn manage(&mut self, conn: u64, m: &Manage, c: Pending) {
        if m.protocol != PROTOCOL {
            return refuse(&c.out, &unsupported(m.protocol));
        }
        if let Err(why) = self.backend.manage(&m.key) {
            log::info!("event=refused ip={} reason=server_key", c.ip);
            return refuse(&c.out, &why);
        }
        log::info!("live session: connection {conn} manages files");
        let comments = message::reads_comments(&m.client_version);
        self.take_in(conn, c, Kind::Managing { comments }, &ServerMsg::Admin(Box::new(AdminReply::Ready)));
    }

    fn open_library(&mut self, conn: u64, o: OpenLibrary, c: Pending) {
        if o.protocol != PROTOCOL {
            return refuse(&c.out, &unsupported(o.protocol));
        }
        let (access, link) = match self.backend.admit(&o.file, &o.token) {
            Ok(a) => (a.access.for_reader(message::reads_comments(&o.client_version)), a.link),
            Err(why) => return refuse(&c.out, &why),
        };
        self.take_in(conn, c, Kind::Library { file: o.file, access, link }, &ServerMsg::Library(Box::new(LibraryReply::Ready { access })));
    }

    /// A plugin directory connection: anyone may browse; a key, when given, must be the server's.
    fn open_plugins(&mut self, conn: u64, o: &OpenPlugins, c: Pending) {
        if o.protocol != PROTOCOL {
            return refuse(&c.out, &unsupported(o.protocol));
        }
        let publisher = !o.key.is_empty();
        if publisher && let Err(why) = self.backend.manage(&o.key) {
            return refuse(&c.out, &why);
        }
        self.take_in(conn, c, Kind::Plugins { publisher }, &ServerMsg::Plugins(Box::new(PluginReply::Ready { publisher })));
    }

    /// An open hosting connection: what the server says about its hosting, then publishing (when
    /// it is open to the public) or managing the file whose owner token it gave.
    fn open_hosting(&mut self, conn: u64, o: &OpenHosting, c: Pending) {
        if o.protocol != PROTOCOL {
            return refuse(&c.out, &unsupported(o.protocol));
        }
        let info = match self.backend.open_hosting(c.ip, &o.file, &o.owner) {
            Ok(i) => i,
            Err(why) => {
                log::info!("event=refused ip={} reason={why:?}", c.ip);
                return refuse(&c.out, &why);
            }
        };
        let owns = (!o.owner.is_empty()).then(|| o.file.clone());
        let comments = message::reads_comments(&o.client_version);
        self.take_in(conn, c, Kind::Hosting { owns, comments }, &ServerMsg::Hosting(Box::new(info)));
    }

    /// A join code: the file it opens and the token to join it with, then the connection closes
    /// (it was never taken in).
    fn resolve(&mut self, r: &Resolve, c: &Pending) {
        if r.protocol != PROTOCOL {
            return refuse(&c.out, &unsupported(r.protocol));
        }
        match self.backend.resolve(c.ip, &r.code) {
            Ok((file, token)) => {
                if let Some(b) = encode(&ServerMsg::Resolved { file, token }) {
                    let _ = c.out.tx.try_send(Out::Bytes(b, None));
                }
                let _ = c.out.tx.try_send(Out::Close(CloseCode::Normal, String::new()));
            }
            Err(why) => {
                log::info!("event=code_refused ip={} reason={why:?}", c.ip);
                refuse(&c.out, &why);
            }
        }
    }

    fn admin(&mut self, conn: u64, ip: IpAddr, req: Admin) {
        let rooms = &self.rooms;
        let open = |file: &str| rooms.get(file).map(Room::info);
        let handled = self.backend.admin(ip, req, &open);
        self.handled(conn, handled);
    }

    /// Answer a managing or hosting request (as its client reads it), and disconnect whoever it
    /// affects.
    fn handled(&mut self, conn: u64, handled: Handled) {
        let comments =
            self.conns.get(&conn).is_some_and(|c| matches!(c.kind, Kind::Managing { comments: true } | Kind::Hosting { comments: true, .. }));
        self.answer(conn, &ServerMsg::Admin(Box::new(handled.reply.for_reader(comments))));
        if let Some((file, link)) = handled.revoked {
            let with = |joined: bool| -> Vec<u64> {
                let uses = |c: &Conn| match &c.kind {
                    Kind::Joined { file: f, link: Some(l), .. } => joined && *f == file && *l == link,
                    Kind::Library { file: f, link: Some(l), .. } => !joined && *f == file && *l == link,
                    _ => false,
                };
                self.conns.iter().filter(|(_, c)| uses(c)).map(|(k, _)| *k).collect()
            };
            let (people, libraries) = (with(true), with(false));
            // People in the file are told the session is over for them, as when a host stops (they
            // keep their copy; nothing retries); a library request is refused.
            for conn in &people {
                if let Some(c) = self.conns.get(conn)
                    && c.out.tx.try_send(Out::Close(CloseCode::Away, STOPPED.into())).is_err()
                {
                    shutdown(&c.sock);
                }
                self.gone(*conn);
            }
            self.kick(&libraries, "the link you joined with was revoked");
        }
        if let Some(file) = handled.removed {
            let users: Vec<u64> = self
                .conns
                .iter()
                .filter(|(_, c)| matches!(&c.kind, Kind::Joined { file: f, .. } | Kind::Library { file: f, .. } if *f == file))
                .map(|(k, _)| *k)
                .collect();
            // Closed without saving: the file is gone.
            self.rooms.remove(&file);
            self.barred.remove(&file);
            self.kick(&users, "this file was removed from the server");
        }
        if let Some(ip) = handled.blocked {
            let users: Vec<u64> = self.conns.iter().filter(|(_, c)| c.ip == ip).map(|(k, _)| *k).collect();
            self.kick(&users, "this server no longer accepts connections from your address");
        }
    }

    /// Send one message to one connection now (managing connections' answers).
    fn answer(&mut self, conn: u64, msg: &ServerMsg) {
        if !self.conns.contains_key(&conn) {
            return;
        }
        let mut drop = Vec::new();
        if !encode(msg).is_some_and(|b| deliver(&self.conns, conn, &b, &mut Charges::default(), &mut drop)) {
            drop.push(conn);
        }
        self.drop_conns(drop);
    }

    /// Cut connections that can't keep up (or were shed) and let them go.
    fn drop_conns(&mut self, conns: Vec<u64>) {
        for conn in conns {
            if let Some(c) = self.conns.get(&conn) {
                shutdown(&c.sock);
            }
            self.gone(conn);
        }
    }

    /// Refuse connections with `why` and drop them at once (nothing more they send counts).
    fn kick(&mut self, conns: &[u64], why: &str) {
        for conn in conns {
            if let Some(c) = self.conns.get(conn)
                && !refused(&c.out, why)
            {
                shutdown(&c.sock);
            }
            self.gone(*conn);
        }
    }

    fn gone(&mut self, conn: u64) {
        let Some(c) = self.conns.remove(&conn) else { return };
        let Kind::Joined { file, id, .. } = c.kind else { return };
        let Some(room) = self.rooms.get_mut(&file) else { return };
        if room.ids.get(&id) == Some(&conn) {
            room.ids.remove(&id);
        }
        room.throttles.remove(&id);
        room.latest.remove(&id);
        let outs = room.server.leave(id);
        self.queue(&file, outs);
        self.close_if_empty(&file);
    }

    /// What changed in `file` couldn't be made durable: nobody is told it was confirmed. The file
    /// is closed without saving and everyone in it reconnects (its last durable state opens again
    /// and their apps send again what wasn't confirmed).
    fn unsaved(&mut self, file: &str) {
        log::error!("file {file}: changes couldn't be saved; reconnecting everyone in it");
        self.outbox.retain(|(f, _)| f != file);
        self.rooms.remove(file);
        let users: Vec<u64> =
            self.conns.iter().filter(|(_, c)| matches!(&c.kind, Kind::Joined { file: f, .. } if f == file)).map(|(k, _)| *k).collect();
        for conn in users {
            if let Some(c) = self.conns.remove(&conn)
                && c.out.tx.try_send(Out::Close(CloseCode::Restart, reason(UNSAVED))).is_err()
            {
                shutdown(&c.sock);
            }
        }
    }

    /// Nobody is in `file`: the backend saves it and may close it.
    fn close_if_empty(&mut self, file: &str) {
        let Some(room) = self.rooms.get_mut(file).filter(|r| r.empty()) else { return };
        if self.backend.idle(file, &mut room.server) {
            self.rooms.remove(file);
            self.barred.remove(file);
        }
    }

    fn presence(&mut self, file: &str, id: u32, p: Presence) {
        let Some(room) = self.rooms.get_mut(file) else { return };
        let p = sanitize(p);
        room.latest.insert(id, p.clone());
        if let Some(p) = room.throttles.entry(id).or_insert_with(|| Throttle::new(PRESENCE_INTERVAL)).offer(p, Instant::now()) {
            let outs = room.server.receive(id, ClientMsg::Presence(p));
            self.queue(file, outs);
        }
    }

    fn flush_due(&mut self, now: Instant) {
        let mut outs = Vec::new();
        for (file, room) in &mut self.rooms {
            let due: Vec<(u32, Presence)> = room.throttles.iter_mut().filter_map(|(id, t)| t.take_due(now).map(|p| (*id, p))).collect();
            for (id, p) in due {
                outs.extend(room.server.receive(id, ClientMsg::Presence(p)).into_iter().map(|o| (file.clone(), o)));
            }
        }
        self.outbox.extend(outs);
    }

    /// Make what changed durable, then send what the servers said; connections that can't keep up
    /// are dropped (which may say more: that they left).
    fn send_out(&mut self) {
        for file in self.backend.flush() {
            self.unsaved(&file);
        }
        while !self.outbox.is_empty() {
            let mut stuck = Vec::new();
            for (file, o) in std::mem::take(&mut self.outbox) {
                let Some(room) = self.rooms.get(&file) else { continue };
                let (to, msg): (Vec<u32>, ServerMsg) = match o {
                    Outgoing::To(id, m) => (vec![id], m),
                    Outgoing::All(m) => (room.server.peers().map(|p| p.client_id).collect(), m),
                    Outgoing::Others(x, m) => (room.server.peers().map(|p| p.client_id).filter(|id| *id != x).collect(), m),
                };
                let mut bytes = None;
                // One count against the server's budget, and each block's, for every copy of this
                // message; welcomes aren't counted against blocks.
                let mut held = Charges { skip_blocks: matches!(msg, ServerMsg::Welcome { .. }), ..Charges::default() };
                for id in to {
                    if let Some((local, inbox)) = room.local.as_ref().filter(|(l, _)| *l == id) {
                        to_local(inbox, *local, &msg);
                        continue;
                    }
                    let Some(&conn) = room.ids.get(&id) else { continue };
                    let Some(b) = bytes.get_or_insert_with(|| encode(&msg)).clone() else { continue };
                    if !deliver(&self.conns, conn, &b, &mut held, &mut stuck) {
                        stuck.push(conn);
                    }
                }
            }
            self.drop_conns(stuck);
        }
    }
}

/// Queue `b` for connection `conn`: false when it must be dropped (it can't keep up, or it is
/// gone). When the server's budget for queued bytes is used up, the connection holding the most
/// queued bytes is shed (added to `shed`) to make room, unless that is `conn` itself: whoever
/// hogs the budget goes, not the next client that happens to need a little of it.
fn deliver(conns: &HashMap<u64, Conn>, conn: u64, b: &Bytes, held: &mut Charges, shed: &mut Vec<u64>) -> bool {
    let Some(c) = conns.get(&conn) else { return true };
    match c.out.push(b, held, false) {
        Ok(()) => true,
        Err(Push::Everyone) => {
            let biggest = conns.iter().filter(|(k, _)| !shed.contains(k)).max_by_key(|(_, c)| c.out.queued.load(Ordering::Relaxed));
            match biggest {
                Some((&big, bc)) if big != conn && bc.out.queued.load(Ordering::Relaxed) > 0 => {
                    log::info!("event=shed conn={big} ip={} reason=queued_bytes", bc.ip);
                    shed.push(big);
                    c.out.push(b, held, true).is_ok()
                }
                _ => {
                    log::info!("live session: connection {conn} holds the most queued bytes; disconnecting it");
                    false
                }
            }
        }
        Err(Push::Full) => {
            log::info!("live session: connection {conn} can't keep up; disconnecting it");
            false
        }
        Err(Push::Gone) => false,
    }
}

/// What the hosting person's window should know.
pub(crate) fn to_local(inbox: &Inbox, local: u32, msg: &ServerMsg) {
    match msg {
        ServerMsg::PeerJoined(p) if p.client_id != local => inbox.push(Event::PeerJoined(p.clone())),
        ServerMsg::PeerLeft { client_id } => inbox.push(Event::PeerLeft(*client_id)),
        ServerMsg::Presence { client_id, presence } => inbox.presence(*client_id, presence.clone()),
        ServerMsg::Changes { .. } | ServerMsg::Reject { .. } => inbox.push(Event::Sync(msg.clone())),
        ServerMsg::Signal { client_id, signal } => inbox.push(Event::Signal(*client_id, signal.clone())),
        _ => {}
    }
}

#[cfg(test)]
mod queue_tests;

#[cfg(test)]
mod barred_tests;

#[cfg(test)]
mod preauth_tests;
