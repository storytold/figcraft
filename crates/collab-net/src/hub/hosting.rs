use super::backend::Backend;
use super::conn::{Shared, accept};
use super::limits::{PerAddress, loopback};
use super::queue::{Blocks, HubIn, InFlight};
use super::{
    Arc, AtomicBool, AtomicUsize, ClientMsg, Duration, HashMap, Hub, Inbox, Instant, JoinHandle, MAX_NAME, NetLimits, Ordering, Presence, Readers,
    Room, Sender, Server, SocketAddr, TcpListener, TcpStream, channel, io,
};

/// The person hosting an app session: in the file's session without a socket.
pub(crate) struct Local {
    pub file: String,
    pub server: Server,
    /// Their client id (they joined `server` already).
    pub id: u32,
    pub inbox: Inbox,
}

pub(super) fn spawn(name: &str, f: impl FnOnce() + Send + 'static) -> io::Result<JoinHandle<()>> {
    std::thread::Builder::new().name(name.to_string()).spawn(f)
}

/// A name others may see: trimmed, no control characters, at most [`MAX_NAME`] characters.
pub(crate) fn clean_name(s: &str) -> String {
    s.trim().chars().filter(|c| !c.is_control()).take(MAX_NAME).collect()
}

/// A running hub: its listener, connections and hub thread. Dropping it stops it.
pub struct Hosting {
    addr: SocketAddr,
    hub: Sender<HubIn>,
    stop: Arc<AtomicBool>,
    open: Arc<AtomicUsize>,
    /// Bytes queued for every connection together, and in flight to the hub.
    queued: Arc<AtomicUsize>,
    in_flight: InFlight,
    blocks: Blocks,
    reading: Arc<AtomicUsize>,
    hub_thread: Option<JoinHandle<()>>,
    accept_thread: Option<JoinHandle<()>>,
}

impl Hosting {
    /// Serve `backend`'s files on `listener`.
    pub fn serve<B: Backend>(listener: TcpListener, backend: B, limits: NetLimits) -> io::Result<Hosting> {
        Self::start(listener, backend, limits, None)
    }

    pub(crate) fn start<B: Backend>(listener: TcpListener, backend: B, limits: NetLimits, local: Option<Local>) -> io::Result<Hosting> {
        let addr = listener.local_addr()?;
        let in_flight = InFlight(Arc::default());
        let mut hub = Hub {
            backend,
            rooms: HashMap::new(),
            conns: HashMap::new(),
            local: None,
            outbox: Vec::new(),
            in_flight: in_flight.clone(),
            barred: HashMap::new(),
        };
        if let Some(l) = local {
            let mut room = Room::new(l.server);
            room.local = Some((l.id, l.inbox));
            hub.rooms.insert(l.file.clone(), room);
            hub.local = Some(l.file);
        }
        let (hub_tx, hub_rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let open = Arc::new(AtomicUsize::new(0));
        let hub_thread = spawn("live-hub", move || hub.run(&hub_rx))?;
        let shared = Shared {
            per_address: PerAddress::default(),
            queued: Arc::default(),
            in_flight: in_flight.clone(),
            blocks: Blocks::default(),
            reading: Arc::default(),
            readers: Readers::default(),
        };
        let queued = shared.queued.clone();
        let blocks = shared.blocks.clone();
        let reading = shared.reading.clone();
        let accept_thread = {
            let (hub_tx, stop, open) = (hub_tx.clone(), stop.clone(), open.clone());
            spawn("live-accept", move || accept(&listener, &hub_tx, &stop, &open, limits, shared))
        };
        let accept_thread = match accept_thread {
            Ok(t) => t,
            Err(e) => {
                let _ = hub_tx.send(HubIn::Stop);
                let _ = hub_thread.join();
                return Err(e);
            }
        };
        log::info!("live session: listening on {addr}");
        Ok(Hosting {
            addr,
            hub: hub_tx,
            stop,
            open,
            queued,
            in_flight,
            blocks,
            reading,
            hub_thread: Some(hub_thread),
            accept_thread: Some(accept_thread),
        })
    }

    /// Bytes waiting to be sent to clients, all connections together.
    pub fn queued_bytes(&self) -> usize {
        self.queued.load(Ordering::Relaxed)
    }

    /// Blocks of addresses the server keeps counters for (those of open connections, and any
    /// whose last connection just ended, until the next connection prunes them).
    pub fn blocks_remembered(&self) -> usize {
        self.blocks.len()
    }

    /// Bytes being read into messages not whole yet, all connections together.
    pub fn reading_bytes(&self) -> usize {
        self.reading.load(Ordering::Relaxed)
    }

    /// Bytes of messages read and not handled by the hub yet.
    pub fn in_flight_bytes(&self) -> usize {
        self.in_flight.0.load(Ordering::Relaxed)
    }

    /// Where it listens.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Open connections (joined or not).
    pub fn connections(&self) -> usize {
        self.open.load(Ordering::Relaxed)
    }

    /// The hosting person's presence (app sessions).
    pub(crate) fn presence(&self, p: Presence) {
        let _ = self.hub.send(HubIn::Local(p));
    }

    /// A message from the hosting person's sync client (their edits).
    pub(crate) fn send(&self, msg: ClientMsg) {
        let _ = self.hub.send(HubIn::LocalMsg(msg));
    }

    /// The hosting person removes client `id` from their file (app sessions; see
    /// [`HubIn::Remove`]).
    pub(crate) fn remove(&self, id: u32) {
        let _ = self.hub.send(HubIn::Remove(id));
    }

    /// Cut every connection as a network failure would (no close frame).
    #[cfg(test)]
    pub(crate) fn cut_connections(&self) {
        let _ = self.hub.send(HubIn::Cut);
    }

    /// Stop: the backend saves what's open, everyone is told and disconnected, the port is
    /// released.
    pub fn stop(&mut self) {
        if self.stop.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = self.hub.send(HubIn::Stop);
        if let Some(t) = self.hub_thread.take() {
            let _ = t.join();
        }
        // The accept loop checks the flag after each connection: wake it with one.
        let wake = if self.addr.ip().is_unspecified() { SocketAddr::new(loopback(self.addr), self.addr.port()) } else { self.addr };
        let woke = TcpStream::connect_timeout(&wake, Duration::from_secs(1)).is_ok();
        if let Some(t) = self.accept_thread.take() {
            if woke {
                let _ = t.join();
            } else {
                log::warn!("live session: couldn't wake the accept loop; it ends with the next connection");
            }
        }
        // Readers end once their writers have closed the sockets.
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.open.load(Ordering::Relaxed) > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        log::info!("live session: stopped listening on {}", self.addr);
    }
}

impl Drop for Hosting {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Counts an open connection until dropped.
pub(super) struct Open(pub(super) Arc<AtomicUsize>);

impl Drop for Open {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}
