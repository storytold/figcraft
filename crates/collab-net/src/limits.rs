//! Transport limits, shared by the native transport and the browser client.

use std::net::IpAddr;
use std::time::Duration;

/// Transport limits (the collaboration core has its own: [`figcraft_collab::Limits`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NetLimits {
    /// Sockets a host keeps open at once, joined or not.
    pub max_connections: usize,
    /// Largest message a client may send once it has joined (a batch can carry images), and
    /// before: its hello.
    pub max_client_message: usize,
    pub max_hello: usize,
    /// Largest message a client accepts from the host (a welcome carries the whole document): 1 GiB,
    /// room for any file up to 1023 MiB and well past the servers' defaults, while a hostile server
    /// (any link can name one) can't make the app or a browser tab buffer more than that.
    pub max_server_message: usize,
    /// Messages a client may send per second on average, and in a burst; beyond that it is
    /// disconnected.
    pub rate: f64,
    pub burst: f64,
    /// A new connection must finish the WebSocket handshake and say hello within this.
    pub hello_timeout: Duration,
    /// A connection with no traffic for this long is dead.
    pub idle_timeout: Duration,
    /// How long a one-shot request connection (hosting, a library, a plugin directory — and, most
    /// importantly, publishing a file) waits for the server's reply before giving up. Publishing
    /// uploads a whole file and a large one on a slow link takes the server a while to receive and
    /// answer, so this is longer than `idle_timeout` (a large project on a slow connection can
    /// easily need several minutes).
    pub request_reply_timeout: Duration,
    /// A quiet client pings this often (keeps it under `idle_timeout`).
    pub keepalive: Duration,
    /// Messages queued for one client before it counts as stuck and is disconnected.
    pub queue: usize,
    /// How long a write may block.
    pub write_timeout: Duration,
    /// Most connections from one address at once (an IPv6 address counts by its /64; 0: no cap).
    pub max_per_address: usize,
    /// The reverse proxy in front of a host, whose `X-Forwarded-For` names the client: only
    /// connections from this address are believed (the header is anyone's to forge).
    pub trusted_proxy: Option<IpAddr>,
    /// Bytes queued for one client before it counts as stuck (one message always fits an empty
    /// queue), for one block of addresses (an IPv4 address, an IPv6 /48), and for every client
    /// together (a broadcast counted once; past it the biggest consumer is dropped).
    pub max_queued_bytes: usize,
    pub max_queued_block: usize,
    pub max_queued_total: usize,
    /// Bytes being read into messages (not whole yet) for one block, and for every connection:
    /// past them the biggest readers are dropped.
    pub max_reading_block: usize,
    pub max_reading_total: usize,
    /// Bytes of messages handed to the hub and not handled yet, all connections together: past it
    /// a connection is dropped as busy instead of piling more on.
    pub max_in_flight: usize,
    /// Largest message on a connection that only asks small things (browsing a plugin directory).
    pub max_request: usize,
    /// Connections only sessions (`Hello`) and the operator may take: one-shot requests leave them.
    pub reserve_for_sessions: usize,
    /// A one-shot request connection (hosting, a library, a plugin directory) with no traffic for
    /// this long is dropped.
    pub request_idle: Duration,
}

impl Default for NetLimits {
    fn default() -> Self {
        Self {
            max_connections: 64,
            max_client_message: figcraft_collab::Limits::default().max_message_bytes,
            max_hello: 64 << 10,
            max_server_message: 1 << 30,
            rate: 100.0,
            burst: 400.0,
            hello_timeout: Duration::from_secs(10),
            idle_timeout: Duration::from_secs(45),
            request_reply_timeout: Duration::from_secs(300),
            keepalive: Duration::from_secs(15),
            queue: 1024,
            write_timeout: Duration::from_secs(10),
            max_per_address: 0,
            trusted_proxy: None,
            max_queued_bytes: 160 << 20,
            max_queued_block: usize::MAX,
            max_queued_total: usize::MAX,
            max_reading_block: usize::MAX,
            max_reading_total: usize::MAX,
            max_in_flight: usize::MAX,
            max_request: 64 << 10,
            reserve_for_sessions: 0,
            request_idle: Duration::from_secs(45),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_reply_wait_is_longer_than_idle() {
        let d = NetLimits::default();
        // Publishing uploads a whole file; a slow link must not cut it off at the session idle
        // timeout. Keep the reply wait comfortably above it.
        assert!(d.request_reply_timeout > d.idle_timeout);
    }
}
