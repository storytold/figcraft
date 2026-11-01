use super::limits::ip_blocks;
use super::{
    Arc, AtomicUsize, Bytes, ClientMsg, CloseCode, HashMap, IpAddr, Mutex, Ordering, PoisonError, Presence, SyncSender, TcpStream, TrySendError,
};

/// A message for one connection's writer: bytes (with what they count against, released when
/// they are sent or dropped), or the close.
pub(crate) enum Out {
    Bytes(Bytes, Option<Charge>),
    Close(CloseCode, String),
}

/// One message's bytes held for one or more connections (a broadcast shares one buffer), counted
/// once against a shared counter (the server's, or a block's) until the last copy is sent or
/// dropped.
pub(super) struct Held {
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
pub(crate) struct Charges {
    pub(super) all: Option<Arc<Held>>,
    pub(super) blocks: Vec<(usize, Arc<Held>)>,
    /// Count it against blocks at all: a welcome isn't (it is bounded by the file's size and each
    /// connection's cap, and a whole office joining at once must fit).
    pub(crate) skip_blocks: bool,
}

/// Bytes waiting in a connection's queue, counted against its cap until dropped (and, through
/// [`Held`], once against the server's budget and once against its block's).
pub(super) struct Charge {
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
pub(crate) enum Push {
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
pub(crate) struct Line {
    pub(crate) tx: SyncSender<Out>,
    pub(crate) queued: Arc<AtomicUsize>,
    pub(crate) block: Arc<AtomicUsize>,
    pub(crate) all: Arc<AtomicUsize>,
    pub(crate) max: usize,
    pub(crate) max_block: usize,
    pub(crate) max_all: usize,
}

impl Line {
    /// Queue `b`. One message always fits an empty queue (a welcome as large as the largest file
    /// gets out). `charges` is what the message counts against, shared by its recipients;
    /// `over_budget` lets it past the server's budget (the hub just dropped the biggest consumer
    /// to make room).
    pub(crate) fn push(&self, b: &Bytes, charges: &mut Charges, over_budget: bool) -> Result<(), Push> {
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
pub(crate) struct BlockUse {
    pub(super) queued: Arc<AtomicUsize>,
    pub(super) reading: Arc<AtomicUsize>,
}

/// The blocks with open connections; a block leaves when its last connection ends.
#[derive(Clone, Default)]
pub(crate) struct Blocks(pub(crate) Arc<Mutex<HashMap<IpAddr, BlockUse>>>);

impl Blocks {
    /// Block `key`'s counters; blocks nobody uses any more (only the table holds their counters,
    /// whose connections' queues and readers are all gone) are forgotten first, so the table
    /// never holds more than the blocks of open connections.
    pub(crate) fn get(&self, key: IpAddr) -> BlockUse {
        let mut t = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        t.retain(|_, u| Arc::strong_count(&u.queued) > 1 || Arc::strong_count(&u.reading) > 1);
        t.entry(key).or_default().clone()
    }

    /// Blocks remembered.
    pub(crate) fn len(&self) -> usize {
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
pub(crate) struct InFlight(pub(crate) Arc<AtomicUsize>);

impl InFlight {
    pub(super) fn take(&self, n: usize, max: usize) -> bool {
        if self.0.fetch_add(n, Ordering::Relaxed).saturating_add(n) > max {
            self.0.fetch_sub(n, Ordering::Relaxed);
            return false;
        }
        true
    }

    pub(super) fn give(&self, n: usize) {
        self.0.fetch_sub(n, Ordering::Relaxed);
    }
}

pub(super) enum HubIn {
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
        /// Told when the hub takes the connection in (dropped unsent when it doesn't).
        admitted: SyncSender<()>,
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
