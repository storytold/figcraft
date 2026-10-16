//! The session host the app embeds: the shared [hub](crate::hub) serving one file, the one the
//! app shares, with the session's single token. The person hosting is in it without a socket:
//! their presence goes in with [`Host::presence`], their edits with [`Host::send`], what others do
//! comes out through their [`Inbox`]. When the host stops, the session ends for everyone.

use std::io;
use std::net::{SocketAddr, TcpListener};

use figcraft_collab::message::{Access, ClientMsg, Hello, PROTOCOL, Peer, Presence, ServerMsg};
use figcraft_collab::{Outgoing, Server, same_secret};
use figcraft_doc::Document;
use figcraft_engine::live::{Event, PEOPLE};

pub use crate::hub::REMOVED;
use crate::hub::{Admission, Backend, Closing, Hosting, Local, clean_name, to_local};
use crate::inbox::Inbox;
pub use crate::join::ENDED;
use crate::net::NetLimits;

/// What a hosted session needs.
pub struct HostConfig {
    /// The address to listen on (port 0: any free port).
    pub addr: SocketAddr,
    /// When `addr`'s port is taken, take any free one instead.
    pub any_port_if_taken: bool,
    /// The file's session id and secret (both in the invite).
    pub file: String,
    pub token: String,
    /// The shared document (the session's authoritative copy starts from it).
    pub doc: Document,
    /// The host's own name (empty: "Host").
    pub name: String,
    pub limits: NetLimits,
    pub core: figcraft_collab::Limits,
}

/// A running session host. Dropping it stops it.
pub struct Host {
    hosting: Hosting,
    me: Peer,
}

/// The app's one file and its token: everyone with the invite edits.
struct OneFile {
    file: String,
    token: String,
}

impl Backend for OneFile {
    fn admit(&mut self, file: &str, token: &str) -> Result<Admission, String> {
        if file != self.file {
            return Err("this invite is for another file or an earlier session".into());
        }
        if !same_secret(token, &self.token) {
            return Err("wrong session token".into());
        }
        Ok(Admission { access: Access::Edit, link: None })
    }

    fn open(&mut self, _: &str) -> Result<Server, String> {
        Err("this invite is for another file or an earlier session".into())
    }

    fn closing(&self) -> Closing {
        Closing::Ended
    }
}

impl Host {
    /// Listen and start the hub; the host joins its own session as the first person.
    pub fn start(cfg: HostConfig, inbox: Inbox) -> io::Result<Host> {
        let listener = match TcpListener::bind(cfg.addr) {
            Ok(l) => l,
            Err(e) if e.kind() == io::ErrorKind::AddrInUse && cfg.any_port_if_taken => TcpListener::bind(SocketAddr::new(cfg.addr.ip(), 0))?,
            Err(e) => return Err(e),
        };
        // The hub checks the token; the session itself keeps none.
        let mut server = Server::new(cfg.doc, "", cfg.core);
        let name = match clean_name(&cfg.name) {
            n if n.is_empty() => "Host".to_string(),
            n => n,
        };
        let hello = Hello {
            protocol: PROTOCOL,
            file: cfg.file.clone(),
            token: String::new(),
            name,
            color: PEOPLE[0].to_string(),
            client_version: figcraft_collab::message::client_version(),
            resume: None,
        };
        let (me, outs) = server.join(hello).map_err(|r| io::Error::other(format!("{r:?}")))?;
        // The host's window syncs through its own client too: its welcome carries the document.
        for o in outs {
            match o {
                Outgoing::To(id, w @ ServerMsg::Welcome { .. }) if id == me => inbox.push(Event::Sync(w)),
                Outgoing::To(id, ref m @ ServerMsg::Signal { .. }) if id == me => to_local(&inbox, me, m),
                _ => {}
            }
        }
        let me = server.peers().find(|p| p.client_id == me).cloned().ok_or_else(|| io::Error::other("the host didn't join its own session"))?;
        let backend = OneFile { file: cfg.file.clone(), token: cfg.token };
        let local = Local { file: cfg.file, server, id: me.client_id, inbox };
        let hosting = Hosting::start(listener, backend, cfg.limits, Some(local))?;
        Ok(Host { hosting, me })
    }

    /// Where it listens.
    pub fn addr(&self) -> SocketAddr {
        self.hosting.addr()
    }

    /// The host as the others see them.
    pub fn me(&self) -> &Peer {
        &self.me
    }

    /// Open connections (joined or not).
    pub fn connections(&self) -> usize {
        self.hosting.connections()
    }

    /// The host's own presence.
    pub fn presence(&self, p: Presence) {
        self.hosting.presence(p);
    }

    /// A message from the host's own sync client (its edits).
    pub fn send(&self, msg: ClientMsg) {
        self.hosting.send(msg);
    }

    /// Remove client `id` from the session: they are disconnected and can't join again from
    /// their address while it runs. The host can't remove themselves.
    pub fn remove(&self, id: u32) {
        self.hosting.remove(id);
    }

    /// Cut every connection as a network failure would (no close frame).
    #[cfg(test)]
    pub(crate) fn cut_connections(&self) {
        self.hosting.cut_connections();
    }

    /// End the session: everyone is told and disconnected, the port is released.
    pub fn stop(&mut self) {
        self.hosting.stop();
    }
}
