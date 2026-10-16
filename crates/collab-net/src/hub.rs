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

/// How long a closing connection waits for the peer to hang up.
const LINGER: Duration = Duration::from_secs(2);

/// Signals (cursor chat, spotlight) a client may send per second on average, and in a burst;
/// beyond that they are dropped.
pub const SIGNAL_RATE: f64 = 20.0;
pub const SIGNAL_BURST: f64 = 40.0;

/// Most messages the hub handles before it sends what they produced (bounds the delay a burst
/// adds).
const BURST: usize = 256;

/// What a link lets a hello do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Admission {
    pub access: Access,
    /// The link's id, when the backend has several per file (revoking it disconnects whoever
    /// joined with it).
    pub link: Option<String>,
}

/// An admin request's reply, and what the hub must do about connections it affects.
#[derive(Clone, Debug, PartialEq)]
pub struct Handled {
    pub reply: AdminReply,
    /// A link was revoked: `(file, link)`; whoever joined with it is disconnected.
    pub revoked: Option<(String, String)>,
    /// A file was removed: whoever is in it is disconnected and it is closed without saving.
    pub removed: Option<String>,
    /// An address was blocked ([`ip_key`]): its connections are dropped.
    pub blocked: Option<IpAddr>,
}

impl From<AdminReply> for Handled {
    fn from(reply: AdminReply) -> Self {
        Self { reply, revoked: None, removed: None, blocked: None }
    }
}

/// The address limits and blocks count a client by: its IPv4 address (an IPv4-mapped IPv6 address
/// as IPv4), or its IPv6 address's /64 (one subscriber's block: they have billions of addresses).
pub fn ip_key(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V6(v6) => {
            let [a, b, c, d, ..] = v6.segments();
            IpAddr::V6(Ipv6Addr::new(a, b, c, d, 0, 0, 0, 0))
        }
        v4 => v4,
    }
}

/// What everyone is told when a hub stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Closing {
    /// The session is over (the app stopped hosting): final.
    Ended,
    /// The server is restarting: clients reconnect until it is back.
    Restarting,
}

/// What the hub knows of an open file (for listings).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OpenFile {
    /// People connected to it.
    pub people: u32,
    /// Its latest sequence number.
    pub seq: u64,
}

/// Where a hub's files come from and what it keeps of them (see the module docs). The hub calls it
/// from its own thread only.
pub trait Backend: Send + 'static {
    /// A hello for `file` with `token`: what it may do, or why it may not join (the reason is
    /// shown to the person joining; never echo the token).
    fn admit(&mut self, file: &str, token: &str) -> Result<Admission, String>;

    /// The session for a file that isn't open (after [`admit`](Self::admit) let someone in).
    fn open(&mut self, file: &str) -> Result<Server, String>;

    /// The hub handled a message for an open file: keep what changed (its server's journal).
    fn changed(&mut self, _file: &str, _server: &mut Server) {}

    /// Make what [`changed`](Self::changed) kept durable; the hub sends the servers' answers
    /// after this. The files whose changes couldn't be made durable: nobody hears of them, the
    /// files are closed (the backend forgets their open state) and everyone in them reconnects.
    fn flush(&mut self) -> Vec<String> {
        Vec::new()
    }

    /// Whether `file` has room for a batch of about `bytes` (its size on the wire): why not, and
    /// the batch is refused.
    fn room(&mut self, _file: &str, _server: &Server, _bytes: usize) -> Result<(), String> {
        Ok(())
    }

    /// Everyone left `file`: save it. True closes it (the next hello opens it again).
    fn idle(&mut self, _file: &str, _server: &mut Server) -> bool {
        false
    }

    /// The hub is stopping: save the open files.
    fn stop(&mut self, _open: Vec<(&str, &mut Server)>) {}

    /// A managing connection's key: whether it may manage files.
    fn manage(&mut self, _key: &str) -> Result<(), String> {
        Err("this session is hosted by an app: it has no files to manage".into())
    }

    /// A request on a managing connection; `open` tells about a file that is open.
    fn admin(&mut self, _req: Admin, _open: &dyn Fn(&str) -> Option<OpenFile>) -> Handled {
        AdminReply::Failed { reason: "nothing to manage here".into() }.into()
    }

    /// A request on a library connection to `file`, opened with a link of `access` (checked by
    /// [`admit`](Self::admit)).
    fn library(&mut self, _ip: IpAddr, _file: &str, _access: Access, _req: LibraryReq) -> LibraryReply {
        LibraryReply::Failed { reason: "this session is hosted by an app: team libraries live on a figcraft-server".into() }
    }

    /// A request on a plugin directory connection; `publisher`: it was opened with the right key
    /// (checked by [`manage`](Self::manage)).
    fn plugins(&mut self, _ip: IpAddr, _publisher: bool, _req: PluginReq) -> PluginReply {
        PluginReply::Failed { reason: "this session is hosted by an app: plugin directories live on a figcraft-server".into() }
    }

    /// A new connection from `ip` ([`ip_key`]): whether it may go on (why not is shown to the
    /// client).
    fn connect(&mut self, _ip: IpAddr) -> Result<(), String> {
        Ok(())
    }

    /// A message from a connection from `ip`: false disconnects it (that address sends too much).
    fn message(&mut self, _ip: IpAddr, _msg: &ClientMsg) -> bool {
        true
    }

    /// What to tell a client that reads it about the server's hosting, after its welcome to
    /// `file` (`None`: nothing).
    fn hosting(&mut self, _file: &str) -> Option<HostInfo> {
        None
    }

    /// An open hosting connection from `ip`, with a file's owner token when `file` isn't empty:
    /// what the server says about its hosting, or why the connection is refused.
    fn open_hosting(&mut self, _ip: IpAddr, _file: &str, _owner: &str) -> Result<HostInfo, String> {
        Err("this session is hosted by an app: it doesn't keep files for anyone".into())
    }

    /// A request on an open hosting connection from `ip`; `owns`: the file whose owner token it
    /// was opened with.
    fn hosted(&mut self, _ip: IpAddr, _owns: Option<&str>, _req: Admin, _open: &dyn Fn(&str) -> Option<OpenFile>) -> Handled {
        AdminReply::Failed { reason: "nothing to manage here".into() }.into()
    }

    /// Someone in `file` (joined with `link`, from `ip`) reports it to the operator.
    fn report(&mut self, _file: &str, _link: Option<&str>, _ip: IpAddr, _reason: &str) -> Result<(), String> {
        Err("this session is hosted by an app: tell the person hosting it".into())
    }

    /// A join code typed by someone at `ip`: the file it opens and that link's token, or why not
    /// (shown to them; wrong codes are counted against the address).
    fn resolve(&mut self, _ip: IpAddr, _code: &str) -> Result<(String, String), String> {
        Err("this session is hosted by an app: join it with its own code or invite link".into())
    }

    /// What everyone is told when the hub stops.
    fn closing(&self) -> Closing;
}

/// A message for one connection's writer: bytes (with what they count against, released when
/// they are sent or dropped), or the close.
enum Out {
    Bytes(Bytes, Option<Charge>),
    Close(CloseCode, String),
}

/// One message's bytes held for one or more connections (a broadcast shares one buffer), counted
/// once against a shared counter (the server's, or a block's) until the last copy is sent or
/// dropped.
struct Held {
    n: usize,
    counter: Arc<AtomicUsize>,
}

impl Held {
    fn new(n: usize, counter: &Arc<AtomicUsize>) -> Arc<Held> {
        counter.fetch_add(n, Ordering::Relaxed);
        Arc::new(Held { n, counter: counter.clone() })
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        self.counter.fetch_sub(self.n, Ordering::Relaxed);
    }
}

/// What one message counts against, made by its first recipient and shared by the others: the
/// server's budget once, and each recipient block's once (twenty people behind one office's
/// address cost their block one copy of a broadcast, not twenty).
#[derive(Default)]
struct Charges {
    all: Option<Arc<Held>>,
    blocks: Vec<(usize, Arc<Held>)>,
    /// Count it against blocks at all: a welcome isn't (it is bounded by the file's size and each
    /// connection's cap, and a whole office joining at once must fit).
    skip_blocks: bool,
}

/// Bytes waiting in a connection's queue, counted against its cap until dropped (and, through
/// [`Held`], once against the server's budget and once against its block's).
struct Charge {
    n: usize,
    mine: Arc<AtomicUsize>,
    _all: Arc<Held>,
    _block: Option<Arc<Held>>,
}

impl Drop for Charge {
    fn drop(&mut self) {
        self.mine.fetch_sub(self.n, Ordering::Relaxed);
    }
}

/// Why a message couldn't be queued.
#[derive(Debug, PartialEq, Eq)]
enum Push {
    /// The connection's or its block's queue is full: it can't keep up.
    Full,
    /// The server's budget for queued bytes is used up.
    Everyone,
    /// The connection is gone.
    Gone,
}

/// One connection's way out: its writer's queue, capped in messages and in bytes (its own, its
/// block of addresses', and the server's for everyone), so a client that doesn't read can't make
/// the server hold more than that for it.
#[derive(Clone)]
struct Line {
    tx: SyncSender<Out>,
    queued: Arc<AtomicUsize>,
    block: Arc<AtomicUsize>,
    all: Arc<AtomicUsize>,
    max: usize,
    max_block: usize,
    max_all: usize,
}

impl Line {
    /// Queue `b`. One message always fits an empty queue (a welcome as large as the largest file
    /// gets out). `charges` is what the message counts against, shared by its recipients;
    /// `over_budget` lets it past the server's budget (the hub just dropped the biggest consumer
    /// to make room).
    fn push(&self, b: &Bytes, charges: &mut Charges, over_budget: bool) -> Result<(), Push> {
        let n = b.len();
        let mine = self.queued.load(Ordering::Relaxed);
        if mine > 0 && mine.saturating_add(n) > self.max {
            return Err(Push::Full);
        }
        let key = Arc::as_ptr(&self.block) as usize;
        let block = if charges.skip_blocks {
            None
        } else if let Some((_, h)) = charges.blocks.iter().find(|(k, _)| *k == key) {
            Some(h.clone())
        } else {
            let used = self.block.load(Ordering::Relaxed);
            if used > 0 && used.saturating_add(n) > self.max_block {
                return Err(Push::Full);
            }
            let h = Held::new(n, &self.block);
            charges.blocks.push((key, h.clone()));
            Some(h)
        };
        let all = match &charges.all {
            Some(h) => h.clone(),
            None => {
                if !over_budget && self.all.load(Ordering::Relaxed).saturating_add(n) > self.max_all {
                    return Err(Push::Everyone);
                }
                charges.all.insert(Held::new(n, &self.all)).clone()
            }
        };
        self.queued.fetch_add(n, Ordering::Relaxed);
        let charge = Charge { n, mine: self.queued.clone(), _all: all, _block: block };
        self.tx.try_send(Out::Bytes(b.clone(), Some(charge))).map_err(|e| match e {
            TrySendError::Full(_) => Push::Full,
            TrySendError::Disconnected(_) => Push::Gone,
        })
    }
}

/// What one block of addresses (an IPv4 address, an IPv6 /48: [`wide_block`]) has queued and is
/// having read, shared by its connections.
#[derive(Clone, Default)]
struct BlockUse {
    queued: Arc<AtomicUsize>,
    reading: Arc<AtomicUsize>,
}

/// The blocks with open connections; a block leaves when its last connection ends.
#[derive(Clone, Default)]
struct Blocks(Arc<Mutex<HashMap<IpAddr, BlockUse>>>);

impl Blocks {
    /// Block `key`'s counters; blocks nobody uses any more (only the table holds their counters,
    /// whose connections' queues and readers are all gone) are forgotten first, so the table
    /// never holds more than the blocks of open connections.
    fn get(&self, key: IpAddr) -> BlockUse {
        let mut t = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        t.retain(|_, u| Arc::strong_count(&u.queued) > 1 || Arc::strong_count(&u.reading) > 1);
        t.entry(key).or_default().clone()
    }

    /// Blocks remembered.
    fn len(&self) -> usize {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).len()
    }
}

/// The widest block an address counts in for the server's shared budgets: an IPv4 address, or an
/// IPv6 address's /48.
pub fn wide_block(key: IpAddr) -> IpAddr {
    ip_blocks(key).last().map_or(key, |(b, _)| b.0)
}

/// Messages handed to the hub thread and not handled yet, in bytes: past the budget a reader
/// drops its connection instead of piling more on a busy hub.
#[derive(Clone)]
struct InFlight(Arc<AtomicUsize>);

impl InFlight {
    fn take(&self, n: usize, max: usize) -> bool {
        if self.0.fetch_add(n, Ordering::Relaxed).saturating_add(n) > max {
            self.0.fetch_sub(n, Ordering::Relaxed);
            return false;
        }
        true
    }

    fn give(&self, n: usize) {
        self.0.fetch_sub(n, Ordering::Relaxed);
    }
}

enum HubIn {
    /// A connection's first message (`Hello`, `Manage`, `OpenLibrary`, `OpenPlugins`,
    /// `OpenHosting` or `Resolve`) and its client's address ([`ip_key`]).
    Open {
        conn: u64,
        ip: IpAddr,
        first: ClientMsg,
        out: Line,
        sock: TcpStream,
        /// Its size (in flight until handled).
        bytes: usize,
    },
    Msg {
        conn: u64,
        msg: ClientMsg,
        bytes: usize,
    },
    Gone {
        conn: u64,
    },
    Local(Presence),
    LocalMsg(ClientMsg),
    /// The person hosting removes client `id` from their file (app sessions).
    Remove(u32),
    /// Cut every connection without a word (tests: a network failure).
    #[cfg(test)]
    Cut,
    Stop,
}

/// The person hosting an app session: in the file's session without a socket.
pub(crate) struct Local {
    pub file: String,
    pub server: Server,
    /// Their client id (they joined `server` already).
    pub id: u32,
    pub inbox: Inbox,
}

fn spawn(name: &str, f: impl FnOnce() + Send + 'static) -> io::Result<JoinHandle<()>> {
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

/// The loopback address of `addr`'s family.
fn loopback(addr: SocketAddr) -> std::net::IpAddr {
    if addr.is_ipv6() { std::net::Ipv6Addr::LOCALHOST.into() } else { std::net::Ipv4Addr::LOCALHOST.into() }
}

/// Counts an open connection until dropped.
struct Open(Arc<AtomicUsize>);

impl Drop for Open {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// A block of addresses: its first address and its prefix length (2001:db8:: as a /64 and as a
/// /48 are different blocks).
pub type Block = (IpAddr, u8);

/// The blocks a client's address ([`ip_key`]) counts in, with how many times its own limits each
/// gets: the address itself (an IPv4 /32 or an IPv6 /64, ×1) and, for IPv6, its /56 (×2) and /48
/// (×4), so a tunnel's free /48 doesn't multiply the limits 65,536 times.
pub fn ip_blocks(key: IpAddr) -> Vec<(Block, u32)> {
    let IpAddr::V6(v6) = key else { return vec![((key, 32), 1)] };
    let [a, b, c, d, ..] = v6.segments();
    vec![
        ((IpAddr::V6(Ipv6Addr::new(a, b, c, d, 0, 0, 0, 0)), 64), 1),
        ((IpAddr::V6(Ipv6Addr::new(a, b, c, d & 0xff00, 0, 0, 0, 0)), 56), 2),
        ((IpAddr::V6(Ipv6Addr::new(a, b, c, 0, 0, 0, 0, 0)), 48), 4),
    ]
}

/// Open connections per client address ([`ip_key`], and its wider IPv6 blocks); an address
/// leaves when its last one closes, so the table never holds more than the open connections.
#[derive(Clone, Default)]
struct PerAddress(Arc<Mutex<HashMap<Block, usize>>>);

/// One connection counted for a block until dropped.
struct Counted {
    table: PerAddress,
    ip: Block,
}

impl PerAddress {
    /// Count a connection from block `ip`, unless it has `cap` already (0: no cap).
    fn enter(&self, ip: Block, cap: usize) -> Option<Counted> {
        let mut t = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let n = t.entry(ip).or_insert(0);
        if cap > 0 && *n >= cap {
            if *n == 0 {
                t.remove(&ip);
            }
            return None;
        }
        *n += 1;
        Some(Counted { table: self.clone(), ip })
    }

    /// Count a connection from `ip` in each of its blocks, unless one of them is full. A wider
    /// block's cap is its multiple of `cap`, but at most a sixteenth (/56) or an eighth (/48) of
    /// `all` connections, so no block takes every connection.
    fn enter_all(&self, ip: IpAddr, cap: usize, all: usize) -> Option<Vec<Counted>> {
        let mut held = Vec::new();
        for ((addr, prefix), times) in ip_blocks(ip) {
            let block_cap = match prefix {
                56 => cap.saturating_mul(times as usize).min(all / 16).max(1),
                48 => cap.saturating_mul(times as usize).min(all / 8).max(1),
                _ => cap,
            };
            held.push(self.enter((addr, prefix), if cap == 0 { 0 } else { block_cap })?);
        }
        Some(held)
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        let mut t = self.table.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(n) = t.get_mut(&self.ip) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                t.remove(&self.ip);
            }
        }
    }
}

/// Longest request line an uptime check may have.
const MAX_CHECK_LINE: usize = 2048;

/// An uptime check: a plain `GET` of a path ending in `/healthz` is answered `200 ok` and closed.
/// True when it was one (the connection is done). Anything else is only peeked at, so a WebSocket
/// handshake goes on untouched.
fn health_check(stream: &mut TcpStream, until: Instant) -> bool {
    let mut buf = [0u8; MAX_CHECK_LINE];
    let line = loop {
        let n = match stream.peek(&mut buf) {
            Ok(0) | Err(_) => return false,
            Ok(n) => n,
        };
        let seen = buf.get(..n).unwrap_or_default();
        if !seen.starts_with(b"GET ") && !b"GET ".starts_with(seen) {
            return false;
        }
        if let Some(end) = seen.windows(2).position(|w| w == b"\r\n") {
            break seen.get(..end).unwrap_or_default().to_vec();
        }
        if n == buf.len() || Instant::now() >= until {
            return false;
        }
        // The rest of the line is on its way (it nearly always arrives in one piece).
        std::thread::sleep(Duration::from_millis(10));
    };
    let path = line.split(|b| *b == b' ').nth(1).unwrap_or_default();
    let path = path.split(|b| *b == b'?').next().unwrap_or_default();
    if !path.ends_with(b"/healthz") {
        return false;
    }
    // Read the request (its headers end with a blank line), so closing doesn't reset it.
    let mut req = Vec::new();
    let mut chunk = [0u8; 1024];
    while !req.windows(4).any(|w| w == b"\r\n\r\n") && req.len() < 16 << 10 {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => req.extend_from_slice(chunk.get(..n).unwrap_or_default()),
        }
    }
    let _ = stream
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 3\r\nCache-Control: no-store\r\nConnection: close\r\n\r\nok\n");
    let _ = stream.shutdown(std::net::Shutdown::Write);
    true
}

/// The client a trusted proxy names: the last address of its last `X-Forwarded-For` (the one the
/// proxy added; any before it are whatever the client sent).
fn forwarded_for(req: &Request) -> Option<IpAddr> {
    let last = req.headers().get_all("x-forwarded-for").iter().next_back()?.to_str().ok()?;
    let entry = last.rsplit(',').next()?.trim();
    entry.parse::<IpAddr>().ok().or_else(|| entry.parse::<SocketAddr>().ok().map(|a| a.ip()))
}

/// What every connection shares: the count per address, the bytes queued for all of them and the
/// bytes in flight to the hub.
#[derive(Clone)]
struct Shared {
    per_address: PerAddress,
    queued: Arc<AtomicUsize>,
    in_flight: InFlight,
    blocks: Blocks,
    /// Bytes being read into messages, all connections together, and who is reading them.
    reading: Arc<AtomicUsize>,
    readers: Readers,
}

fn accept(listener: &TcpListener, hub: &Sender<HubIn>, stop: &AtomicBool, open: &Arc<AtomicUsize>, limits: NetLimits, shared: Shared) {
    let mut next = 0u64;
    for stream in listener.incoming() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                // Out of sockets or similar: don't spin.
                log::warn!("live session: accept failed: {e}");
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
        };
        if open.load(Ordering::Relaxed) >= limits.max_connections {
            log::info!("live session: too many connections; refusing one");
            continue;
        }
        open.fetch_add(1, Ordering::Relaxed);
        let guard = Open(open.clone());
        next += 1;
        let (conn, hub, shared) = (next, hub.clone(), shared.clone());
        if let Err(e) = spawn("live-conn", move || serve(stream, conn, &hub, limits, guard, &shared)) {
            log::warn!("live session: can't start a connection thread: {e}");
        }
    }
}

/// A close reason that fits a WebSocket close frame (123 bytes).
fn reason(s: &str) -> String {
    let mut end = s.len().min(123);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s.get(..end).unwrap_or_default().to_string()
}

fn encode(msg: &ServerMsg) -> Option<Bytes> {
    match message::encode(msg) {
        Ok(b) => Some(Bytes::from(b)),
        Err(e) => {
            log::warn!("live session: can't encode a message: {e}");
            None
        }
    }
}

/// Queue `Refused {reason}` and a close for one connection (small: not counted against its caps).
fn refuse(out: &Line, why: &str) {
    if let Some(b) = encode(&ServerMsg::Refused { reason: why.to_string() }) {
        let _ = out.tx.try_send(Out::Bytes(b, None));
    }
    let _ = out.tx.try_send(Out::Close(CloseCode::Policy, reason(why)));
}

/// What the server tells a client when it is too busy to take more from it.
const BUSY: &str = "the server is busy: try again in a moment";

/// Read the next binary message (skipping pings) and its size, or why the connection should end.
fn next_message(ws: &mut WebSocket<Duplex>, max: usize) -> Result<Option<(ClientMsg, usize)>, String> {
    loop {
        return match ws.read() {
            Ok(Message::Binary(b)) => {
                // Whole now: it counts as in flight from here, no longer as being read.
                ws.get_mut().read_done();
                message::decode::<ClientMsg>(&b, max).map(|m| Some((m, b.len()))).map_err(|_| "that message couldn't be read".to_string())
            }
            Ok(Message::Text(_)) => Err("only binary messages are spoken here".into()),
            Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => continue,
            Ok(Message::Close(_)) => Ok(None),
            Err(tungstenite::Error::Capacity(_)) => Err(too_large(max)),
            // The reading budget's refusal (see `Meter`).
            Err(tungstenite::Error::Io(e)) if e.kind() == io::ErrorKind::Other => Err(e.to_string()),
            Err(_) => Ok(None),
        };
    }
}

/// Why a message over `max` bytes is refused.
fn too_large(max: usize) -> String {
    match max >> 20 {
        0 => format!("message too large: this server takes at most {} KiB at once here", max >> 10),
        mib => format!("message too large: this server takes at most {mib} MiB at once (a change that big can't be kept here)"),
    }
}

/// One connection's reader (the writer runs on its own thread).
fn serve(mut stream: TcpStream, conn: u64, hub: &Sender<HubIn>, limits: NetLimits, open: Open, shared: &Shared) {
    let _ = stream.set_nodelay(true);
    if stream.set_read_timeout(Some(limits.hello_timeout)).is_err() || stream.set_write_timeout(Some(limits.write_timeout)).is_err() {
        return;
    }
    if health_check(&mut stream, Instant::now() + limits.hello_timeout) {
        return;
    }
    let Ok(peer) = stream.peer_addr().map(|a| a.ip().to_canonical()) else { return };
    let proxied = limits.trusted_proxy.is_some_and(|p| p.to_canonical() == peer);
    let Ok((rd, wr, sock)) = Duplex::split(stream, None) else { return };
    let cfg = config(limits.max_client_message);
    let mut forwarded = None;
    // The handshake callback's type is tungstenite's (its error is an HTTP response).
    #[allow(clippy::result_large_err)]
    let read_client = |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
        if proxied {
            forwarded = forwarded_for(req);
        }
        Ok(resp)
    };
    let mut ws = match tungstenite::accept_hdr_with_config(rd, read_client, Some(config(limits.max_hello))) {
        Ok(ws) => ws,
        Err(e) => {
            log::debug!("live session: handshake failed: {e}");
            shutdown(&sock);
            return;
        }
    };
    if proxied && forwarded.is_none() {
        log::debug!("live session: connection {conn} came through the proxy without X-Forwarded-For");
    }
    let ip = ip_key(forwarded.unwrap_or(peer));
    let counted = shared.per_address.enter_all(ip, limits.max_per_address, limits.max_connections);
    let wide = wide_block(ip);
    let block = shared.blocks.get(wide);
    ws.get_mut().meter(Meter::new(
        conn,
        &sock,
        &shared.readers,
        block.reading.clone(),
        shared.reading.clone(),
        limits.max_reading_block,
        limits.max_reading_total,
    ));
    let (tx, rx) = sync_channel::<Out>(limits.queue.max(4));
    let out = Line {
        tx,
        queued: Arc::default(),
        block: block.queued.clone(),
        all: shared.queued.clone(),
        max: limits.max_queued_bytes,
        max_block: limits.max_queued_block,
        max_all: limits.max_queued_total,
    };
    let writer =
        match sock.try_clone().map(|s| spawn("live-write", move || write_loop(WebSocket::from_raw_socket(wr, Role::Server, Some(cfg)), &rx, &s))) {
            Ok(Ok(t)) => t,
            _ => {
                shutdown(&sock);
                return;
            }
        };
    let end = |why: Option<&str>| {
        if let Some(why) = why {
            refuse(&out, why);
        } else {
            let _ = out.tx.try_send(Out::Close(CloseCode::Normal, String::new()));
        }
    };
    if counted.is_none() {
        log::info!("event=refused ip={ip} reason=connections_per_address");
        end(Some("too many connections from your address: close some and try again"));
    } else {
        match next_message(&mut ws, limits.max_hello) {
            Ok(Some((
                first @ (ClientMsg::Hello(_)
                | ClientMsg::Manage(_)
                | ClientMsg::OpenLibrary(_)
                | ClientMsg::OpenPlugins(_)
                | ClientMsg::OpenHosting(_)
                | ClientMsg::Resolve(_)),
                bytes,
            ))) => {
                // Sessions and the operator may use the last connections; one-shot requests
                // (hosting, libraries, browsing plugins) leave them free.
                let session = matches!(first, ClientMsg::Hello(_) | ClientMsg::Manage(_));
                let max = match &first {
                    ClientMsg::OpenPlugins(o) if o.key.is_empty() => limits.max_request,
                    _ => limits.max_client_message,
                };
                let idle = if session { limits.idle_timeout } else { limits.request_idle };
                let reserved = !session && open.0.load(Ordering::Relaxed) > limits.max_connections.saturating_sub(limits.reserve_for_sessions);
                // (In flight is only taken when the connection goes on: the hub gives it back.)
                if reserved || !shared.in_flight.take(bytes, limits.max_in_flight) {
                    end(Some(BUSY));
                } else {
                    ws.set_config(|c| *c = config(max));
                    let opened = sock.try_clone().is_ok_and(|s| hub.send(HubIn::Open { conn, ip, first, out: out.clone(), sock: s, bytes }).is_ok());
                    if opened && ws.get_ref().set_read_timeout(Some(idle)).is_ok() {
                        read_loop(&mut ws, conn, hub, limits, max, &shared.in_flight, &end);
                        let _ = hub.send(HubIn::Gone { conn });
                    }
                }
            }
            Ok(Some(_)) => end(Some("say hello first")),
            Ok(None) => {}
            Err(why) => end(Some(&why)),
        }
    }
    drop(counted);
    // The block's counters go with the last handle on them (the table forgets it then).
    drop(block);
    linger(&mut ws);
    drop(out);
    // The writer closes the socket once it has sent what's queued (a refusal, a close).
    let _ = writer.join();
    shutdown(&sock);
}

fn read_loop(
    ws: &mut WebSocket<Duplex>,
    conn: u64,
    hub: &Sender<HubIn>,
    limits: NetLimits,
    max: usize,
    in_flight: &InFlight,
    end: &dyn Fn(Option<&str>),
) {
    let mut bucket = Bucket::new(limits.rate, limits.burst);
    let mut signals = Bucket::new(SIGNAL_RATE, SIGNAL_BURST);
    loop {
        match next_message(ws, max) {
            Ok(Some((msg, bytes))) => {
                let now = Instant::now();
                if !bucket.take(now) {
                    log::info!("live session: connection {conn} sends too much; disconnecting it");
                    return end(Some("too many messages"));
                }
                // Typing sends a chat signal per keystroke; more than this is dropped.
                if matches!(msg, ClientMsg::Signal(_)) && !signals.take(now) {
                    log::debug!("live session: connection {conn} sends signals too fast; dropping one");
                    continue;
                }
                if !in_flight.take(bytes, limits.max_in_flight) {
                    log::info!("live session: the hub is behind; dropping connection {conn}");
                    return end(Some(BUSY));
                }
                if hub.send(HubIn::Msg { conn, msg, bytes }).is_err() {
                    return;
                }
            }
            Ok(None) => return end(None),
            Err(why) => return end(Some(&why)),
        }
    }
}

fn write_loop(mut ws: WebSocket<Duplex>, rx: &Receiver<Out>, sock: &TcpStream) {
    let mut close = Some((CloseCode::Normal, String::new()));
    while let Ok(o) = rx.recv() {
        match o {
            Out::Bytes(b, _charge) => {
                if ws.send(Message::Binary(b)).is_err() {
                    close = None;
                    break;
                }
            }
            Out::Close(code, why) => {
                close = Some((code, why));
                break;
            }
        }
    }
    if let Some((code, why)) = close {
        // Close politely: the close frame, then no more writing; the reader drains what the peer
        // still sends until it hangs up (closing on unread data would reset the connection and
        // could lose the close frame), or until the linger runs out.
        let _ = ws.close(Some(CloseFrame { code, reason: why.into() }));
        let _ = sock.shutdown(std::net::Shutdown::Write);
        let until = Instant::now() + LINGER;
        while let Ok(_) | Err(RecvTimeoutError::Timeout) = rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
            if Instant::now() >= until {
                break;
            }
        }
    }
    shutdown(sock);
}

/// Read and drop what the peer still sends after the close, until it hangs up (at most
/// [`LINGER`]). Raw bytes, not frames: after a frame over the size limit the WebSocket's reader
/// is mid-frame and mustn't be asked for more.
fn linger(ws: &mut WebSocket<Duplex>) {
    let until = Instant::now() + LINGER;
    let mut buf = [0u8; 4096];
    while Instant::now() < until && ws.get_ref().set_read_timeout(Some(Duration::from_millis(250))).is_ok() {
        match ws.get_mut().read(&mut buf) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => {}
            Err(_) => break,
        }
    }
}

/// What a connection is for.
enum Kind {
    /// In `file`'s session as client `id`, through link `link`.
    Joined { file: String, id: u32, link: Option<String> },
    /// Managing the server's files.
    Managing,
    /// Using `file`'s team library through link `link` of `access` (revoking it disconnects).
    Library { file: String, access: Access, link: Option<String> },
    /// Using the plugin directory; `publisher`: with the server's key.
    Plugins { publisher: bool },
    /// Publishing without the key, or managing the file `owns` with its owner token.
    Hosting { owns: Option<String> },
}

/// A connection whose first message the hub is looking at.
struct Pending {
    out: Line,
    sock: TcpStream,
    ip: IpAddr,
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
            HubIn::Open { conn, ip, first, out, sock, bytes } => {
                self.in_flight.give(bytes);
                // The operator (with the server key) gets in from anywhere; everyone else is
                // checked (blocked addresses, connection rates).
                let operator = matches!(&first, ClientMsg::Manage(m) if self.backend.manage(&m.key).is_ok());
                if !operator && let Err(why) = self.backend.connect(ip) {
                    log::info!("event=refused ip={ip} reason={why:?}");
                    return refuse(&out, &why);
                }
                let c = Pending { out, sock, ip };
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
                if !matches!(c.kind, Kind::Managing) && !self.backend.message(ip, &msg) {
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
            Some(Kind::Hosting { owns }) => match msg {
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
            Some(Kind::Managing) => match msg {
                ClientMsg::Admin(Admin::RemovePeer { file, client }) => {
                    let reply = self.removed_reply(&file, client);
                    self.answer(conn, &ServerMsg::Admin(Box::new(reply)));
                }
                ClientMsg::Admin(req) => self.admin(conn, req),
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
        let _ = c.out.tx.try_send(Out::Close(CloseCode::Away, REMOVED.into()));
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
        self.take_in(conn, c, Kind::Managing, &ServerMsg::Admin(Box::new(AdminReply::Ready)));
    }

    fn open_library(&mut self, conn: u64, o: OpenLibrary, c: Pending) {
        if o.protocol != PROTOCOL {
            return refuse(&c.out, &unsupported(o.protocol));
        }
        let (access, link) = match self.backend.admit(&o.file, &o.token) {
            Ok(a) => (a.access, a.link),
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
        self.take_in(conn, c, Kind::Hosting { owns }, &ServerMsg::Hosting(Box::new(info)));
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

    fn admin(&mut self, conn: u64, req: Admin) {
        let rooms = &self.rooms;
        let open = |file: &str| rooms.get(file).map(Room::info);
        let handled = self.backend.admin(req, &open);
        self.handled(conn, handled);
    }

    /// Answer a managing or hosting request, and disconnect whoever it affects.
    fn handled(&mut self, conn: u64, handled: Handled) {
        self.answer(conn, &ServerMsg::Admin(Box::new(handled.reply)));
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
                if let Some(c) = self.conns.get(conn) {
                    let _ = c.out.tx.try_send(Out::Close(CloseCode::Away, STOPPED.into()));
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
            if let Some(c) = self.conns.get(conn) {
                refuse(&c.out, why);
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
#[path = "queue_tests.rs"]
mod queue_tests;

#[cfg(test)]
#[path = "barred_tests.rs"]
mod barred_tests;
