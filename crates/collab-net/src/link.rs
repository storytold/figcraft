//! [`NetLink`]: the engine's [`LiveLink`] over the network.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use figcraft_collab::code;
use figcraft_collab::message::{Access, Admin, AdminReply, ClientMsg, LibraryReply, LibraryReq, PluginReply, PluginReq};
use figcraft_engine::live::{
    Bind, Event, HostInfo, HostOptions, LiveLink, NO_SESSION, NetReply, NetRequest, PageInfo, Presence, PublishOptions, Published, Resume, Role,
    SessionId, SessionInfo, Shared,
};

use crate::guest::{Guest, GuestConfig, Publish, upload_file};
use crate::host::{Host, HostConfig};
use crate::inbox::{Inbox, Waker};
use crate::invite::{Endpoint, Invite, Lookup};
use crate::manage;
use crate::net::NetLimits;
use crate::requests;
use crate::sessions::Sessions;
use crate::token;

/// The port a hosted session tries first.
pub const DEFAULT_PORT: u16 = 47900;

/// How long a request to a server (outside a session) waits for its connection.
const CONNECT: Duration = Duration::from_secs(5);

enum Active {
    Host(Host),
    Guest(Guest),
}

impl Active {
    /// Stop hosting, or leave.
    fn end(self) {
        match self {
            Active::Host(mut h) => h.stop(),
            Active::Guest(mut g) => g.leave(),
        }
    }
}

/// Hosts and joins sessions, any number at once (one per open document).
pub struct NetLink {
    limits: NetLimits,
    sessions: Sessions<Active>,
    /// Tweaks for joining (tests shorten the reconnect pauses).
    guest: fn(GuestConfig) -> GuestConfig,
    /// Answers to requests sent in the background (outlives sessions).
    libraries: Inbox,
    /// Background library requests are out.
    fetching: Arc<AtomicBool>,
}

impl NetLink {
    /// `wake` is called (from network threads) whenever something arrives for the UI.
    pub fn new(wake: Waker) -> Self {
        let libraries = Inbox::new(wake.clone());
        Self { limits: NetLimits::default(), sessions: Sessions::new(wake), guest: |c| c, libraries, fetching: Arc::default() }
    }

    /// With other transport limits.
    pub fn with_limits(mut self, limits: NetLimits) -> Self {
        self.limits = limits;
        self
    }

    /// With joins configured by `f` (reconnect pauses, attempts).
    pub fn with_guest_config(mut self, f: fn(GuestConfig) -> GuestConfig) -> Self {
        self.guest = f;
        self
    }

    /// How this link joins `invite` (after asking its server about `lookup`, if there is one).
    fn guest_config(&self, invite: Invite, lookup: Option<Lookup>, name: &str) -> GuestConfig {
        let mut cfg = (self.guest)(GuestConfig { lookup, ..GuestConfig::new(invite, name.to_string()) });
        cfg.limits = self.limits;
        cfg
    }

    /// The running host of session `session`, if it is hosted here.
    pub fn host_handle(&self, session: SessionId) -> Option<&Host> {
        match self.sessions.get(session) {
            Some(Active::Host(h)) => Some(h),
            _ => None,
        }
    }

    /// Session `session` started (one that had the id ends).
    fn started(&mut self, session: SessionId, active: Active, inbox: Inbox) {
        if let Some(old) = self.sessions.insert(session, active, inbox) {
            old.end();
        }
    }
}

/// This machine's address on the local network: the interface the default route leaves by (no
/// packet is sent).
pub fn lan_ip() -> Option<IpAddr> {
    let sock = UdpSocket::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0))).ok()?;
    // Documentation and private ranges: only the route matters.
    for probe in ["192.0.2.1:9", "10.255.255.255:9"] {
        if sock.connect(probe).is_ok()
            && let Ok(a) = sock.local_addr()
            && !a.ip().is_loopback()
            && !a.ip().is_unspecified()
        {
            return Some(a.ip());
        }
    }
    None
}

impl LiveLink for NetLink {
    fn host(&mut self, session: SessionId, o: HostOptions) -> Result<SessionInfo, String> {
        let ip = match o.bind {
            Bind::Local => IpAddr::V4(Ipv4Addr::LOCALHOST),
            Bind::Lan => lan_ip().ok_or("this machine has no local network address (connect to a network, or share on this machine only)")?,
            Bind::Addr(ip) => ip,
        };
        // A session shared with a join code: its secret is short enough to be typed with the
        // address (see `figcraft_collab::code`); the code and the invite link name the same session.
        let (file, token) = if o.short {
            (code::LOCAL_FILE.to_string(), code::Local::new(Ipv4Addr::UNSPECIFIED, 0, token::random()?).token())
        } else {
            (token::file_id()?, token::secret()?)
        };
        let inbox = self.sessions.inbox();
        let doc = (*o.doc).clone();
        let pages = doc.pages.iter().map(|p| PageInfo { id: p.0, name: doc.node(*p).map(|n| n.name.clone()).unwrap_or_default() }).collect();
        let file_name = doc.name.clone();
        let cfg = HostConfig {
            addr: SocketAddr::new(ip, o.port.unwrap_or(DEFAULT_PORT)),
            any_port_if_taken: o.port.is_none(),
            file: file.clone(),
            token: token.clone(),
            doc,
            name: o.name,
            limits: self.limits,
            core: figcraft_collab::Limits::default(),
        };
        let host = Host::start(cfg, inbox.clone()).map_err(|e| format!("can't listen on {ip}: {e}"))?;
        let invite = Invite::new(host.addr().to_string(), file, token);
        let info = SessionInfo {
            role: Role::Host,
            me: host.me().client_id,
            name: host.me().name.clone(),
            color: host.me().color.clone(),
            file_name,
            pages,
            link: invite.link(),
            url: invite.url(),
            address: host.addr().to_string(),
            network: !ip.is_loopback(),
            access: Access::Edit,
        };
        self.started(session, Active::Host(host), inbox);
        Ok(info)
    }

    fn join(&mut self, session: SessionId, link: &str, name: &str) -> Result<(), String> {
        self.join_resuming(session, link, name, None)
    }

    fn join_resuming(&mut self, session: SessionId, link: &str, name: &str, resume: Option<Resume>) -> Result<(), String> {
        let (invite, lookup) = Invite::parse_join(link)?;
        let cfg = GuestConfig { resume, ..self.guest_config(invite, lookup, name) };
        let inbox = self.sessions.inbox();
        let guest = Guest::start(cfg, inbox.clone()).map_err(|e| format!("can't start the connection: {e}"))?;
        self.started(session, Active::Guest(guest), inbox);
        Ok(())
    }

    fn publish(&mut self, session: SessionId, o: PublishOptions) -> Result<(), String> {
        let (server, query) = Endpoint::parse(&o.server)?;
        // Without a key: as anyone may, when the server is open to the public (it says if not).
        let key = requests::server_key(o.key, query);
        // Until the upload is done the invite names only the server.
        let cfg = self.guest_config(Invite { server: server.clone(), file: String::new(), token: String::new() }, None, &o.me);
        let inbox = self.sessions.inbox();
        let job = Publish { server, key, name: o.name, doc: o.doc, share: o.share };
        let guest = Guest::publish(job, cfg, inbox.clone()).map_err(|e| format!("can't start the connection: {e}"))?;
        self.started(session, Active::Guest(guest), inbox);
        Ok(())
    }

    fn upload(&mut self, o: PublishOptions) -> Result<Published, String> {
        upload(o, &self.limits)
    }

    fn library(&mut self, link: &str, req: LibraryReq) -> Result<LibraryReply, String> {
        manage::library(link, req, &self.limits, CONNECT)
    }

    fn plugins(&mut self, server: &str, req: PluginReq) -> Result<PluginReply, String> {
        plugins(server, req, &self.limits)
    }

    fn hosting(&mut self, server: &str, file: &str, owner: &str, req: Option<Admin>) -> Result<(HostInfo, Option<AdminReply>), String> {
        hosting(server, file, owner, req, &self.limits)
    }

    fn share(&mut self, server: &str, file: &str, owner: &str, revoke: &[String], create: bool) -> Result<Option<Shared>, String> {
        share(server, file, owner, revoke, create, &self.limits)
    }

    fn background(&mut self, job: u64, req: NetRequest) -> bool {
        let (inbox, limits) = (self.libraries.clone(), self.limits);
        let (failed, failure) = (inbox.clone(), req.failure());
        let spawned = std::thread::Builder::new().name("server-request".into()).spawn(move || {
            let reply = match req {
                NetRequest::Plugins { server, req } => NetReply::Plugins(plugins(&server, req, &limits)),
                NetRequest::Library { link, req } => NetReply::Library(manage::library(&link, req, &limits, CONNECT)),
                NetRequest::Upload(o) => NetReply::Upload(upload(o, &limits)),
                NetRequest::Hosting { server, file, owner, req } => NetReply::Hosting(hosting(&server, &file, &owner, req, &limits)),
                NetRequest::Share { server, file, owner, revoke, create } => NetReply::Share(share(&server, &file, &owner, &revoke, create, &limits)),
            };
            inbox.push(Event::Net { job, reply: Box::new(reply) });
        });
        if let Err(e) = spawned {
            failed.push(Event::Net { job, reply: Box::new(failure(format!("can't start a thread for the request: {e}"))) });
        }
        true
    }

    fn library_background(&mut self, jobs: Vec<(String, LibraryReq)>) {
        if jobs.is_empty() || self.fetching.swap(true, Ordering::SeqCst) {
            return;
        }
        let (inbox, fetching, limits) = (self.libraries.clone(), self.fetching.clone(), self.limits);
        let spawned = std::thread::Builder::new().name("library-fetch".into()).spawn(move || {
            for (link, req) in jobs {
                // A report's answer says nothing about the library (and an older server refuses it).
                let report = matches!(req, LibraryReq::ReportDetached(_) | LibraryReq::ReportInserted(_) | LibraryReq::ReportAssets(_));
                let reply = manage::library(&link, req, &limits, CONNECT);
                if !report {
                    inbox.push(Event::Library { link, reply });
                }
            }
            fetching.store(false, Ordering::SeqCst);
        });
        if let Err(e) = spawned {
            log::warn!("team libraries: can't start a thread: {e}");
            self.fetching.store(false, Ordering::SeqCst);
        }
    }

    fn remove(&mut self, session: SessionId, client: u32) -> Result<(), String> {
        match self.sessions.get(session) {
            Some(Active::Host(h)) if h.me().client_id == client => Err("you can't remove yourself: stop the session instead".into()),
            Some(Active::Host(h)) => {
                h.remove(client);
                Ok(())
            }
            Some(Active::Guest(_)) => Err("only the person hosting can remove people from this session".into()),
            None => Err("this session is over".into()),
        }
    }

    fn leave(&mut self, session: SessionId) {
        if let Some(active) = self.sessions.remove(session) {
            active.end();
        }
    }

    fn presence(&mut self, session: SessionId, p: &Presence) {
        match self.sessions.get(session) {
            Some(Active::Host(h)) => h.presence(p.clone()),
            Some(Active::Guest(g)) => g.presence(p.clone()),
            None => {}
        }
    }

    fn send(&mut self, session: SessionId, msg: ClientMsg) {
        match self.sessions.get(session) {
            Some(Active::Host(h)) => h.send(msg),
            Some(Active::Guest(g)) => g.send(msg),
            None => {}
        }
    }

    fn poll(&mut self) -> Vec<(SessionId, Event)> {
        let mut out: Vec<(SessionId, Event)> = self.libraries.drain().into_iter().map(|e| (NO_SESSION, e)).collect();
        out.extend(self.sessions.poll());
        out
    }
}

/// Keep `o.doc` on a server as a new file (see [`LiveLink::upload`]).
fn upload(o: PublishOptions, limits: &NetLimits) -> Result<Published, String> {
    let (server, query) = Endpoint::parse(&o.server)?;
    let key = requests::server_key(o.key, query)
        .ok_or("the server address needs the server's key (ws://host:port/?key=…; `figcraft-server key` prints it)")?;
    upload_file(&server, Some(&key), &o.name, &o.doc, limits, CONNECT).map(|(_, published, _)| published)
}

/// What the server at `server` says about its hosting, and its answer to `req` about `file` with
/// its owner token (see [`LiveLink::hosting`]). A new link comes back whole, in place of its token.
fn hosting(server: &str, file: &str, owner: &str, req: Option<Admin>, limits: &NetLimits) -> Result<(HostInfo, Option<AdminReply>), String> {
    let (endpoint, _) = Endpoint::parse(server)?;
    let (info, replies) = manage::hosting(&endpoint, file, owner, req.into_iter().collect(), limits, CONNECT)?;
    Ok((info, requests::hosted(&endpoint, replies.into_iter().next())))
}

/// See [`LiveLink::share`]: as the server's operator when the address carries its key, else as
/// the file's owner.
fn share(server: &str, file: &str, owner: &str, revoke: &[String], create: bool, limits: &NetLimits) -> Result<Option<Shared>, String> {
    let (endpoint, query) = Endpoint::parse(server)?;
    let key = requests::server_key(None, query);
    manage::share(&endpoint, key.as_deref(), file, owner, revoke, create, limits, CONNECT)
}

/// One request to the plugin directory at `server` (its key in the address's query, if any).
fn plugins(server: &str, req: PluginReq, limits: &NetLimits) -> Result<PluginReply, String> {
    let (server, query) = Endpoint::parse(server)?;
    let key = query.into_iter().find(|(k, _)| k == "key").map(|(_, v)| v).unwrap_or_default();
    manage::plugins(&server, key.trim(), req, limits, CONNECT)
}

impl Drop for NetLink {
    fn drop(&mut self) {
        for id in self.sessions.ids() {
            self.leave(id);
        }
    }
}
