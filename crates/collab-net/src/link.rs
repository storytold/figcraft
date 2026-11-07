//! [`NetLink`]: the engine's [`LiveLink`] over the network.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use figcraft_collab::code;
use figcraft_collab::message::{Access, Admin, AdminReply, ClientMsg, LibraryReply, LibraryReq, PluginReply, PluginReq};
use figcraft_engine::live::{
    Bind, Event, HostInfo, HostOptions, LiveLink, NO_SESSION, NetReply, NetRequest, PageInfo, Presence, PublishOptions, Published, Resume, Role,
    SessionId, SessionInfo, Shared, WayIn,
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

/// A new secret for a hosted session: one a join code can carry (`short`: 44 bits, see
/// `figcraft_collab::code`), else a full one.
fn session_secret(short: bool) -> Result<String, String> {
    match short {
        true => Ok(code::Local::new(Ipv4Addr::UNSPECIFIED, 0, token::random()?).token()),
        false => token::secret(),
    }
}

/// A view-only secret for a session whose edit secret is `edit`: of the same kind, never the same.
fn view_secret(short: bool, edit: &str) -> Result<String, String> {
    for _ in 0..3 {
        let view = session_secret(short)?;
        if view != edit {
            return Ok(view);
        }
    }
    Err("couldn't make a view-only secret".into())
}

/// This machine's address on the local network: the interface the default route leaves by (no
/// packet is sent).
pub fn lan_ip() -> Option<IpAddr> {
    // Documentation and private ranges: only the route matters.
    route_ip(Ipv4Addr::UNSPECIFIED.into(), &["192.0.2.1:9", "10.255.255.255:9"])
}

/// The address of `any`'s family that the route to the first of `probes` that has one leaves by
/// (no packet is sent): never a loopback, unspecified or IPv6 link-local one, which others can't
/// reach this machine at.
fn route_ip(any: IpAddr, probes: &[&str]) -> Option<IpAddr> {
    let sock = UdpSocket::bind(SocketAddr::new(any, 0)).ok()?;
    probes.iter().find_map(|probe| {
        sock.connect(probe).ok()?;
        let ip = sock.local_addr().ok()?.ip();
        let link_local = matches!(ip, IpAddr::V6(v6) if v6.is_unicast_link_local());
        (!ip.is_loopback() && !ip.is_unspecified() && !link_local).then_some(ip)
    })
}

/// Where others reach a session listening on `addr`: `addr` itself, or for a listener on every
/// interface (`0.0.0.0`, `::`) this machine's local network address of the same family (its
/// loopback without one), on the same port. Links name it; the listener keeps every interface.
pub(crate) fn reachable(addr: SocketAddr) -> SocketAddr {
    let ip = match addr.ip() {
        ip if !ip.is_unspecified() => ip,
        IpAddr::V4(_) => lan_ip().unwrap_or(Ipv4Addr::LOCALHOST.into()),
        // Documentation and unique local ranges, as for IPv4.
        IpAddr::V6(_) => route_ip(Ipv6Addr::UNSPECIFIED.into(), &["[2001:db8::1]:9", "[fd00::1]:9"]).unwrap_or(Ipv6Addr::LOCALHOST.into()),
    };
    SocketAddr::new(ip, addr.port())
}

impl LiveLink for NetLink {
    /// Host the open file on `o.bind`: a join code's short secret when `o.short`, and a second,
    /// view-only secret with its invite link when `o.view`.
    fn host(&mut self, session: SessionId, o: HostOptions) -> Result<SessionInfo, String> {
        let ip = match o.bind {
            Bind::Local => IpAddr::V4(Ipv4Addr::LOCALHOST),
            Bind::Lan => lan_ip().ok_or("this machine has no local network address (connect to a network, or share on this machine only)")?,
            Bind::Addr(ip) => ip,
        };
        // A session shared with a join code: its secret is short enough to be typed with the
        // address (see `figcraft_collab::code`); the code and the invite link name the same session.
        let file = if o.short { code::LOCAL_FILE.to_string() } else { token::file_id()? };
        let token = session_secret(o.short)?;
        // A way in for looking only: a second secret of the same kind, never the edit one.
        let view_token = o.view.then(|| view_secret(o.short, &token)).transpose()?;
        let inbox = self.sessions.inbox();
        let doc = (*o.doc).clone();
        let pages = doc.pages.iter().map(|p| PageInfo { id: p.0, name: doc.node(*p).map(|n| n.name.clone()).unwrap_or_default() }).collect();
        let file_name = doc.name.clone();
        let cfg = HostConfig {
            addr: SocketAddr::new(ip, o.port.unwrap_or(DEFAULT_PORT)),
            any_port_if_taken: o.port.is_none(),
            file: file.clone(),
            token: token.clone(),
            view_token: view_token.clone(),
            doc,
            name: o.name,
            limits: self.limits,
            core: figcraft_collab::Limits::default(),
        };
        let host = Host::start(cfg, inbox.clone()).map_err(|e| format!("can't listen on {ip}: {e}"))?;
        // Listening on every interface (`0.0.0.0`, `::`), the links name an address others reach.
        let at = reachable(host.addr()).to_string();
        let view = view_token.map(|v| Invite::new(at.clone(), file.clone(), v)).map(|v| WayIn { link: v.link(), url: v.url() });
        let invite = Invite::new(at.clone(), file, token);
        let info = SessionInfo {
            role: Role::Host,
            me: host.me().client_id,
            name: host.me().name.clone(),
            color: host.me().color.clone(),
            file_name,
            pages,
            link: invite.link(),
            url: invite.url(),
            address: at,
            network: !ip.is_loopback(),
            access: Access::Edit,
            view,
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

    fn retry(&mut self, session: SessionId) {
        if let Some(Active::Guest(g)) = self.sessions.get(session) {
            g.retry();
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
