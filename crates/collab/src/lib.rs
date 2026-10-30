//! Real-time collaboration core for FigCraft: the synced object/property model over
//! [`Document`], change sets, the session server's ordering and validation, the client's
//! optimistic state, and the wire messages. Pure library: no networking, no UI (the WebSocket
//! transport comes on top). The design is `docs/collab-protocol.md`.
//!
//! - A [`Replica`] is one copy of the synced state: the document plus each node's fractional
//!   position key ([`frac`]).
//! - [`diff`] turns an edit (the document before and after a command) into a batch of
//!   [`Change`]s at property granularity; [`Replica::apply`] applies a batch.
//! - A [`Server`] orders batches (one sequence number each), validates them and rebroadcasts them;
//!   a [`Client`] applies its own edits at once and reconciles when the server confirms, rejects or
//!   orders someone else's batch first. Conflicting writes to one property resolve last-writer-wins
//!   in server order, so every replica converges to the server's state.
//! - The client keeps the user's own undo history ([`Client::undo`], [`Client::redo`]): an undo
//!   writes back what the user's step replaced where nobody else wrote since. Refused batches
//!   leave the history with a [`Notice`]. Offline, batches queue; on reconnect the server sends
//!   what the client missed (or a snapshot) and the queue goes out ([`Client::reconnect`]).
//! - [`merge`] is the three-way merge behind branches: what a branch and its main file changed
//!   since the branch was made, their conflicts, and one merged into the other.
//! - [`sim`] runs a server and several clients over in-memory links with random delays, dropped
//!   connections and an undo checker (tests).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod change;
pub mod client;
pub mod code;
mod diff;
mod docprops;
pub mod frac;
mod history;
pub mod merge;
pub mod message;
pub mod replica;
pub mod server;
pub mod sim;

use std::collections::BTreeSet;

pub use change::Target;
pub use change::{Change, DocProp, Entry, Json, Prop};
pub use client::{Client, HISTORY_LIMIT};
pub use diff::{Diff, diff};
pub use figcraft_doc::{Document, NodeId};
pub use history::{BatchKind, Notice};
pub use message::{Accepted, Access, CatchUp, ClientMsg, EditingText, Hello, Peer, Presence, Processed, Resume, ServerMsg, Signal};
pub use replica::{Applied, Mode, Replica};
pub use server::{COMMENT_ONLY, Journal, Outgoing, Server, SessionState, VIEW_ONLY};

/// Compare secrets (tokens, keys, their hashes) without stopping at the first difference.
pub fn same_secret(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// What can go wrong syncing.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum Error {
    /// A change names a node that doesn't exist (or no longer does).
    #[error("no such node {0}")]
    Unknown(NodeId),
    /// A change or message is not valid.
    #[error("{0}")]
    Invalid(String),
    #[error("can't encode: {0}")]
    Encode(String),
    #[error("can't decode: {0}")]
    Decode(String),
    /// The other side broke the protocol (a message out of order or out of place).
    #[error("protocol error: {0}")]
    Protocol(String),
}

/// Size limits the server enforces (and decoders use).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Largest message, binary or JSON.
    pub max_message_bytes: usize,
    /// Most changes in one batch.
    pub max_changes: usize,
    /// Most nodes in a document.
    pub max_nodes: usize,
    /// Most people in a session.
    pub max_peers: usize,
    /// Longest display name or colour string (chars).
    pub max_name: usize,
    /// Most accepted batches the server keeps for clients that reconnect (older ones catch up
    /// with a snapshot).
    pub max_log_batches: usize,
    /// Most changes in those batches together.
    pub max_log_changes: usize,
    /// About how many bytes those batches may hold together ([`server::weight`]). 64 MiB by
    /// default, so a long busy session (on a desktop host too) keeps a bounded log.
    pub max_log_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_message_bytes: 64 << 20,
            max_changes: 500_000,
            max_nodes: 5_000_000,
            max_peers: 256,
            max_name: 64,
            max_log_batches: 10_000,
            max_log_changes: 1_000_000,
            max_log_bytes: 64 << 20,
        }
    }
}

/// Brings derived data up to date after changes (auto layout, instances following their mains,
/// variable values…). The engine implements it with its normalize pass; a client runs it over its
/// own batch when the server ordered someone else's batch first, and sends what it changes as a
/// follow-up batch (see [`Client::receive`]).
pub trait Recompute {
    /// `doc` is `before` with the nodes in `touched` changed; update what depends on them.
    fn recompute(&self, doc: &mut Document, before: &Document, touched: &BTreeSet<NodeId>);
}

/// Leaves derived data as the authors computed it.
pub struct NoRecompute;

impl Recompute for NoRecompute {
    fn recompute(&self, _: &mut Document, _: &Document, _: &BTreeSet<NodeId>) {}
}

#[cfg(test)]
mod board_sync_tests;
#[cfg(test)]
mod client_tests;
#[cfg(test)]
mod codec_tests;
#[cfg(test)]
mod comment_access_tests;
#[cfg(test)]
mod deleted_cap_tests;
#[cfg(test)]
mod diff_tests;
#[cfg(test)]
mod entry_ids_tests;
#[cfg(test)]
mod frac_tests;
#[cfg(test)]
mod image_tests;
#[cfg(test)]
mod journal_tests;
#[cfg(test)]
mod merge_list_tests;
#[cfg(test)]
mod merge_tests;
#[cfg(test)]
mod owned_tests;
#[cfg(test)]
mod reconnect_tests;
#[cfg(test)]
mod rename_tests;
#[cfg(test)]
mod server_tests;
#[cfg(test)]
mod signal_tests;
#[cfg(test)]
mod sim_tests;
#[cfg(test)]
mod undo_tests;
