//! Live sessions over the network for FigCraft (milestone M9, steps 2 and 4
//! of `docs/collab-protocol.md`): the WebSocket transport, the session host the desktop app embeds,
//! invite links, and presence relay. It carries the collaboration core's messages
//! ([`figcraft_collab::message`]) and runs its [`Server`](figcraft_collab::Server) as the host's
//! authority.
//!
//! The app installs a [`NetLink`] into the engine session (`session.live.install(…)`); the
//! engine's `collab.*` commands drive it and the UI reads the engine's live state. Network
//! threads wake the UI through a [`Waker`] only when something arrives; nothing polls.
//!
//! The browser build's [`NetLink`] joins sessions and publishes files to servers through the
//! browser's WebSocket API (`web-sys`); hosting a session needs the desktop app. Its connection
//! logic (`driver.rs`, `exchange.rs`) is target-independent and tested natively.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(any(target_arch = "wasm32", test))]
mod driver;
#[cfg(any(target_arch = "wasm32", test))]
mod exchange;
pub mod inbox;
pub mod invite;
mod join;
pub mod limits;
mod requests;
mod sessions;
pub mod throttle;

#[cfg(not(target_arch = "wasm32"))]
pub mod guest;
#[cfg(not(target_arch = "wasm32"))]
pub mod host;
#[cfg(not(target_arch = "wasm32"))]
pub mod hub;
#[cfg(not(target_arch = "wasm32"))]
mod link;
#[cfg(not(target_arch = "wasm32"))]
pub mod links;
#[cfg(not(target_arch = "wasm32"))]
pub mod manage;
#[cfg(not(target_arch = "wasm32"))]
pub mod net;
#[cfg(not(target_arch = "wasm32"))]
pub mod tls;
#[cfg(not(target_arch = "wasm32"))]
pub mod token;

#[cfg(target_arch = "wasm32")]
mod web;

pub use inbox::{Inbox, Waker};
pub use invite::{Endpoint, Invite};
pub use limits::NetLimits;
#[cfg(not(target_arch = "wasm32"))]
pub use link::{DEFAULT_PORT, NetLink, lan_ip};
#[cfg(target_arch = "wasm32")]
pub use web::NetLink;

#[cfg(test)]
mod client_cap_tests;
#[cfg(test)]
mod code_tests;
#[cfg(test)]
mod driver_tests;
#[cfg(test)]
mod exchange_tests;
#[cfg(test)]
mod invite_tests;
#[cfg(test)]
mod links_tests;
#[cfg(test)]
mod loopback_tests;
#[cfg(test)]
mod rename_tests;
#[cfg(test)]
mod session_tests;
#[cfg(test)]
mod sessions_tests;
#[cfg(test)]
mod tls_tests;
