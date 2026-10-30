//! The WebSocket plumbing both ends share: one connection read by one thread and written by
//! another, limits, and rate limiting.
//!
//! Each connection has a reader thread blocked in `read` and a writer thread blocked on its
//! queue, so an idle session costs no CPU. Both sides of a connection are tungstenite state
//! machines over the same socket; every write goes through one lock and writes whole frames, so
//! frames from the two never interleave.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

pub use crate::limits::NetLimits;

use tungstenite::protocol::WebSocketConfig;

use crate::invite::Endpoint;
use crate::tls::{self, Tls};

/// tungstenite's settings for messages up to `max` bytes.
pub fn config(max: usize) -> WebSocketConfig {
    WebSocketConfig::default().max_message_size(Some(max)).max_frame_size(Some(max))
}

/// One side (read or write) of a connection: reads from its own handle, writes whole buffers
/// under the connection's write lock; through the connection's TLS session when it has one.
pub struct Duplex {
    read: TcpStream,
    write: Arc<Mutex<TcpStream>>,
    tls: Option<Tls>,
    /// What the bytes read count against (a server's reader), if anything.
    meter: Option<Meter>,
    /// When reading must be over, however slowly the bytes come ([`Duplex::read_until`]).
    deadline: Option<Instant>,
}

impl Duplex {
    /// Two handles on `stream` (for the reader and the writer) and one to shut it down with.
    pub fn split(stream: TcpStream, tls: Option<Tls>) -> io::Result<(Duplex, Duplex, TcpStream)> {
        let write = Arc::new(Mutex::new(stream.try_clone()?));
        let a = Duplex { read: stream.try_clone()?, write: write.clone(), tls: tls.clone(), meter: None, deadline: None };
        let b = Duplex { read: stream.try_clone()?, write, tls, meter: None, deadline: None };
        Ok((a, b, stream))
    }

    /// Count what this handle reads against `meter`.
    pub fn meter(&mut self, meter: Meter) {
        self.meter = Some(meter);
    }

    /// Reads on this handle fail once `deadline` passes, even when each one gets a few bytes in
    /// time (a message trickled in can't outlast it). `None`: only the read timeout applies.
    pub fn read_until(&mut self, deadline: Option<Instant>) {
        self.deadline = deadline;
    }

    /// What was read so far became a message (handed on): it no longer counts as being read.
    pub fn read_done(&mut self) {
        if let Some(m) = self.meter.as_mut() {
            m.release();
        }
    }
}

/// Every reader that counts what it reads, with what it holds half-read and its socket: when
/// everyone's reading budget runs out, the reader holding the most is cut off.
#[derive(Clone, Default)]
pub struct Readers(Arc<Mutex<HashMap<u64, Reader>>>);

/// One registered reader: what it holds half-read, its block's count, whether it was cut off, and
/// its socket.
struct Reader {
    mine: Arc<AtomicUsize>,
    block: Arc<AtomicUsize>,
    shed: Arc<AtomicBool>,
    sock: TcpStream,
}

impl Readers {
    /// Cut off the reader holding the most half-read bytes, unless that is `me` (then `false`: the
    /// caller is the biggest). Its bytes are given back to the budget at once, it can't read any
    /// more, and its socket is shut (where that wakes a blocked read, it ends now; elsewhere at its
    /// read timeout).
    fn shed_biggest(&self, me: u64, all: &AtomicUsize) -> bool {
        let t = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let Some((&id, r)) = t.iter().filter(|(_, r)| !r.shed.load(Ordering::Relaxed)).max_by_key(|(_, r)| r.mine.load(Ordering::Relaxed)) else {
            return false;
        };
        if id == me || r.mine.load(Ordering::Relaxed) == 0 {
            return false;
        }
        log::info!("event=shed conn={id} reason=reading_bytes");
        r.shed.store(true, Ordering::Relaxed);
        let held = r.mine.swap(0, Ordering::Relaxed);
        r.block.fetch_sub(held, Ordering::Relaxed);
        all.fetch_sub(held, Ordering::Relaxed);
        let _ = r.sock.shutdown(Shutdown::Both);
        true
    }

    /// Readers registered.
    pub fn len(&self) -> usize {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).len()
    }

    /// None registered.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Bytes being read into messages (not whole yet), per connection, per block of addresses and
/// for every connection together. A read that takes its block over its share fails. One that takes
/// everyone over the budget cuts off the readers holding the most until it fits (their bytes are
/// given back at once, they can't read any more and their sockets are shut) or, when this reader is
/// the biggest, fails: the biggest readers go, not the next one.
pub struct Meter {
    id: u64,
    mine: Arc<AtomicUsize>,
    shed: Arc<AtomicBool>,
    block: Arc<AtomicUsize>,
    all: Arc<AtomicUsize>,
    max_block: usize,
    max_all: usize,
    readers: Readers,
}

/// What a reader cut off for using too much of the budget is told.
const BUSY_READING: &str = "the server is busy reading from others: try again in a moment";

impl Meter {
    /// Connection `id`'s meter (registered with `readers`, cut off through `sock`).
    pub fn new(
        id: u64,
        sock: &TcpStream,
        readers: &Readers,
        block: Arc<AtomicUsize>,
        all: Arc<AtomicUsize>,
        max_block: usize,
        max_all: usize,
    ) -> Self {
        let (mine, shed) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicBool::new(false)));
        if let Ok(s) = sock.try_clone() {
            let r = Reader { mine: mine.clone(), block: block.clone(), shed: shed.clone(), sock: s };
            readers.0.lock().unwrap_or_else(PoisonError::into_inner).insert(id, r);
        }
        Self { id, mine, shed, block, all, max_block, max_all, readers: readers.clone() }
    }

    fn add(&mut self, n: usize) -> io::Result<()> {
        if self.shed.load(Ordering::Relaxed) {
            return Err(io::Error::other(BUSY_READING));
        }
        self.mine.fetch_add(n, Ordering::Relaxed);
        let block = self.block.fetch_add(n, Ordering::Relaxed).saturating_add(n);
        let mut all = self.all.fetch_add(n, Ordering::Relaxed).saturating_add(n);
        if block > self.max_block {
            return Err(io::Error::other(BUSY_READING));
        }
        // Over everyone's budget: the biggest readers are cut off until it fits, or this one goes
        // when it is the biggest.
        while all > self.max_all {
            if !self.readers.shed_biggest(self.id, &self.all) {
                return Err(io::Error::other(BUSY_READING));
            }
            all = self.all.load(Ordering::Relaxed);
        }
        Ok(())
    }

    fn release(&mut self) {
        let mine = self.mine.swap(0, Ordering::Relaxed);
        self.block.fetch_sub(mine, Ordering::Relaxed);
        self.all.fetch_sub(mine, Ordering::Relaxed);
    }
}

impl Drop for Meter {
    fn drop(&mut self) {
        self.release();
        self.readers.0.lock().unwrap_or_else(PoisonError::into_inner).remove(&self.id);
    }
}

impl Duplex {
    /// How long a read on this handle may block (socket options are per handle: set them on the
    /// one that reads).
    pub fn set_read_timeout(&self, d: Option<Duration>) -> io::Result<()> {
        self.read.set_read_timeout(d)
    }
}

impl Read for Duplex {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = loop {
            if let Some(d) = self.deadline {
                let left = d.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(io::ErrorKind::TimedOut.into());
                }
                self.read.set_read_timeout(Some(left))?;
            }
            let read = match &self.tls {
                Some(t) => tls::read(t, &mut self.read, &self.write, buf),
                None => self.read.read(buf),
            };
            match read {
                // The socket's timeout can run out a moment before the deadline (it counts whole
                // milliseconds): wait out the rest, so a read times out only once it has passed.
                Err(e) if self.deadline.is_some() && matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
                read => break read?,
            }
        };
        if let Some(m) = self.meter.as_mut() {
            m.add(n)?;
        }
        Ok(n)
    }
}

impl Write for Duplex {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Some(t) = &self.tls {
            return tls::write(t, &self.write, buf);
        }
        self.write.lock().unwrap_or_else(PoisonError::into_inner).write_all(buf)?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.write.lock().unwrap_or_else(PoisonError::into_inner).flush()
    }
}

/// Close the socket both ways (wakes a reader blocked on it).
pub fn shutdown(s: &TcpStream) {
    let _ = s.shutdown(Shutdown::Both);
}

/// Connect to `server` (through TLS for `wss://`): the reader's and the writer's handles, and the
/// socket to shut it down with.
pub fn open(server: &Endpoint, timeout: Duration, limits: &NetLimits) -> Result<(Duplex, Duplex, TcpStream), String> {
    let stream = dial(&server.host, timeout, limits)?;
    let tls = if server.secure { Some(tls::handshake(&stream, &server.host)?) } else { None };
    Duplex::split(stream, tls).map_err(|e| e.to_string())
}

/// Connect to `host` (`name:port`), trying each of its addresses within `timeout`, with Nagle off
/// and reads and writes bounded by `limits`' hello and write timeouts.
fn dial(host: &str, timeout: Duration, limits: &NetLimits) -> Result<TcpStream, String> {
    use std::net::ToSocketAddrs;
    let addrs = host.to_socket_addrs().map_err(|e| format!("can't find {host}: {e}"))?;
    let mut last = format!("can't find {host}");
    for a in addrs {
        match TcpStream::connect_timeout(&a, timeout) {
            Ok(s) => {
                let _ = s.set_nodelay(true);
                s.set_read_timeout(Some(limits.hello_timeout)).map_err(|e| e.to_string())?;
                s.set_write_timeout(Some(limits.write_timeout)).map_err(|e| e.to_string())?;
                return Ok(s);
            }
            Err(e) => last = format!("no answer from {a}: {e}"),
        }
    }
    Err(last)
}

/// A token bucket: `rate` per second on average, `burst` at once.
#[derive(Debug)]
pub struct Bucket {
    rate: f64,
    burst: f64,
    tokens: f64,
    at: Instant,
}

impl Bucket {
    pub fn new(rate: f64, burst: f64) -> Self {
        Self { rate, burst, tokens: burst, at: Instant::now() }
    }

    /// Spend one token at `now`; false when there are none left.
    pub fn take(&mut self, now: Instant) -> bool {
        self.take_n(now, 1.0)
    }

    /// Spend `n` tokens at `now`; false (spending none) when there aren't that many.
    pub fn take_n(&mut self, now: Instant, n: f64) -> bool {
        let dt = now.saturating_duration_since(self.at).as_secs_f64();
        self.at = now;
        self.tokens = (self.tokens + dt * self.rate).min(self.burst);
        if self.tokens < n {
            return false;
        }
        self.tokens -= n;
        true
    }

    /// Like [`take_n`](Self::take_n), but more than the burst may be taken from a full bucket,
    /// which it leaves in debt: nothing more until the rate has paid it back. Something larger
    /// than the burst (a big download) is then never refused for good, and the average stays at
    /// `rate`.
    pub fn take_spread(&mut self, now: Instant, n: f64) -> bool {
        if n <= self.burst {
            return self.take_n(now, n);
        }
        if !self.take_n(now, self.burst) {
            return false;
        }
        self.tokens -= n - self.burst;
        true
    }

    /// Give back `n` tokens (up to the burst).
    pub fn give(&mut self, n: f64) {
        self.tokens = (self.tokens + n).min(self.burst);
    }
}
