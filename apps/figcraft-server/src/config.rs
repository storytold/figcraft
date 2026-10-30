//! The server's settings: defaults, then a TOML file (`--config server.toml`), then flags.

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

use figcraft_collab_net::{Endpoint, NetLimits};
use serde::Deserialize;

use crate::abuse::AbuseConfig;
use crate::directory::DirectoryConfig;
use crate::store::{HostingConfig, StoreConfig};

/// The port the server listens on unless told otherwise (the app's sessions try it first too).
pub const DEFAULT_PORT: u16 = figcraft_collab_net::DEFAULT_PORT;

/// Everything the server can be told (the TOML file's keys are these names).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Where to listen: `127.0.0.1:47900` (this machine only) unless told otherwise.
    pub listen: String,
    /// The data directory (files, links, the server key).
    pub data: PathBuf,
    /// How share links name this server when it isn't reached at `listen` (a reverse proxy:
    /// `wss://figcraft.example.com/`).
    pub public_url: Option<String>,
    /// Most files kept.
    pub max_files: usize,
    /// Largest file (its snapshot), in MiB (a welcome carries the whole file, and messages are at
    /// most 64 MiB). Unset: 64, or 16 when open to the public ([`Config::file_mb`]).
    pub max_file_mb: Option<u64>,
    /// Most share links per file.
    pub max_links: usize,
    /// Most open connections.
    pub max_connections: usize,
    /// Most people in one file at once.
    pub max_people: usize,
    /// Compact a file's log into a new snapshot after this many entries…
    pub compact_after: usize,
    /// …or once the log is this large (MiB).
    pub compact_mb: u64,
    /// Most MiB of recent batches a file in use keeps in memory for apps that reconnect (one
    /// further behind catches up with the whole file).
    pub max_log_mb: u64,
    /// Most plugin versions the plugin directory keeps (0 turns it off).
    pub max_plugins: usize,
    /// Largest plugin bundle, in MiB.
    pub max_plugin_mb: u64,
    /// Most MiB of plugin bundles kept.
    pub max_plugins_mb: u64,
    /// Anyone may publish a file without the server key (and gets an owner token for it): a
    /// server open to the public. The limits below then apply per address.
    pub open_publish: bool,
    /// How to reach the operator (an email address or a web page) and the terms of use (a web
    /// address): the app shows them where people publish.
    pub operator_contact: String,
    pub terms_url: String,
    /// The reverse proxy in front of the server (`127.0.0.1` for Caddy or nginx on the same
    /// machine): only its `X-Forwarded-For` is believed.
    pub trusted_proxy: Option<String>,
    /// Files published without the key are deleted after this many days without anyone opening
    /// them (0: kept; fractions allowed).
    pub idle_days: f64,
    /// Publishing without the key stops while the files take this many MiB in all.
    pub max_total_mb: u64,
    /// Per address, when open: most files published without the key, and their MiB.
    pub max_files_per_ip: usize,
    pub max_mb_per_ip: u64,
    /// Per address, when open: most connections at once, new connections a minute, files
    /// published an hour and messages a second.
    pub max_connections_per_ip: usize,
    pub connects_per_minute: u32,
    pub publishes_per_hour: u32,
    pub messages_per_second: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: format!("127.0.0.1:{DEFAULT_PORT}"),
            data: PathBuf::from("figcraft-data"),
            public_url: None,
            max_files: 1000,
            max_file_mb: None,
            max_links: 100,
            max_connections: 512,
            max_people: 256,
            compact_after: 1000,
            compact_mb: 16,
            max_log_mb: 64,
            max_plugins: 500,
            max_plugin_mb: 16,
            max_plugins_mb: 1024,
            open_publish: false,
            operator_contact: String::new(),
            terms_url: String::new(),
            trusted_proxy: None,
            idle_days: 30.0,
            max_total_mb: 10_240,
            max_files_per_ip: 20,
            max_mb_per_ip: 256,
            max_connections_per_ip: 32,
            connects_per_minute: 30,
            publishes_per_hour: 10,
            messages_per_second: 200,
        }
    }
}

impl Config {
    /// Read a TOML file's settings (unknown keys are errors).
    pub fn load(path: &Path) -> Result<Config, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
        toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The listen address.
    pub fn addr(&self) -> Result<SocketAddr, String> {
        self.listen.trim().parse().map_err(|_| format!("`listen` must be an address with a port, like 0.0.0.0:{DEFAULT_PORT}"))
    }

    /// The public URL, when set: a `ws://` or `wss://` address without a query.
    pub fn public(&self) -> Result<Option<Endpoint>, String> {
        let Some(url) = self.public_url.as_deref().map(str::trim).filter(|u| !u.is_empty()) else { return Ok(None) };
        let (e, query) = Endpoint::parse(url).map_err(|e| format!("`public_url`: {e}"))?;
        if !query.is_empty() {
            return Err("`public_url` can't have a query (no keys or tokens in it)".into());
        }
        Ok(Some(e))
    }

    /// The largest file in MiB: as set, else 64, or 16 on a server open to the public (strangers'
    /// files stay small unless the operator says otherwise).
    pub fn file_mb(&self) -> u64 {
        self.max_file_mb.unwrap_or(if self.open_publish { 16 } else { 64 })
    }

    /// What an operator should hear about these settings at startup (a public server without an
    /// abuse contact, or one whose clients would all count as its proxy).
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        let client_max = NetLimits::default().max_server_message >> 20;
        if usize::try_from(self.file_mb()).is_ok_and(|mb| mb >= client_max) {
            out.push(format!(
                "max_file_mb is {}: the app opens files of up to {} MiB, so a file that large can't be opened",
                self.file_mb(),
                client_max - 1
            ));
        }
        if self.people() < self.max_people {
            out.push(format!(
                "max_people is {}: the app keeps track of at most {} people in a file, so {} are let in",
                self.max_people,
                self.people(),
                self.people()
            ));
        }
        if !self.open_publish {
            return out;
        }
        if self.file_mb() > 64 {
            out.push(format!(
                "open_publish is on with max_file_mb = {}: anyone may send messages that large, and the memory set aside for reading and queueing them grows to eight times it; keep it at 64 or less on a public server",
                self.file_mb()
            ));
        }
        if self.operator_contact.trim().is_empty() {
            out.push("open_publish is on but operator_contact is empty: people can't reach you about abuse; set it (and terms_url)".into());
        }
        let behind_proxy = self.public().ok().flatten().is_some_and(|e| e.secure) || self.addr().is_ok_and(|a| a.ip().is_loopback());
        if behind_proxy && self.trusted_proxy.as_deref().is_none_or(|p| p.trim().is_empty()) {
            out.push(
                "open_publish is on without trusted_proxy: behind a reverse proxy every client counts as the proxy's address, so one person's limits stop everyone; set trusted_proxy = \"127.0.0.1\"".into(),
            );
        }
        if self.idle_days == 0.0 {
            out.push("open_publish is on with idle_days = 0: files published without the key are never deleted".into());
        }
        out
    }

    /// Most people in one file at once: `max_people`, lowered to what the app keeps track of
    /// ([`figcraft_collab::message::MAX_PEERS`]).
    pub fn people(&self) -> usize {
        self.max_people.min(figcraft_collab::message::MAX_PEERS)
    }

    /// The trusted proxy's address, when set.
    pub fn proxy(&self) -> Result<Option<IpAddr>, String> {
        let Some(p) = self.trusted_proxy.as_deref().map(str::trim).filter(|p| !p.is_empty()) else { return Ok(None) };
        p.parse().map(Some).map_err(|_| "`trusted_proxy` must be an IP address, like 127.0.0.1".to_string())
    }

    /// Check everything (before starting).
    pub fn check(&self) -> Result<(), String> {
        self.addr()?;
        self.public()?;
        self.proxy()?;
        if !self.idle_days.is_finite() || !(0.0..=3650.0).contains(&self.idle_days) {
            return Err("`idle_days` must be from 0 to 3650".into());
        }
        let terms = self.terms_url.trim();
        if !(terms.is_empty() || terms.starts_with("https://") || terms.starts_with("http://")) || terms.contains(char::is_whitespace) {
            return Err("`terms_url` must be a web address (https://…)".into());
        }
        if self.operator_contact.chars().count() > 120 || self.operator_contact.chars().any(char::is_control) || terms.len() > 500 {
            return Err("`operator_contact` is at most 120 characters and `terms_url` at most 500, on one line".into());
        }
        for (name, v) in [
            ("max_total_mb", self.max_total_mb),
            ("max_files_per_ip", self.max_files_per_ip as u64),
            ("max_mb_per_ip", self.max_mb_per_ip),
            ("max_connections_per_ip", self.max_connections_per_ip as u64),
            ("connects_per_minute", u64::from(self.connects_per_minute)),
            ("publishes_per_hour", u64::from(self.publishes_per_hour)),
            ("messages_per_second", u64::from(self.messages_per_second)),
        ] {
            if v == 0 {
                return Err(format!("`{name}` must be at least 1"));
            }
        }
        for (name, v) in [
            ("max_files", self.max_files as u64),
            ("max_file_mb", self.file_mb()),
            ("max_links", self.max_links as u64),
            ("max_connections", self.max_connections as u64),
            ("max_people", self.max_people as u64),
            ("compact_after", self.compact_after as u64),
            ("compact_mb", self.compact_mb),
            ("max_log_mb", self.max_log_mb),
            ("max_plugin_mb", self.max_plugin_mb),
            ("max_plugins_mb", self.max_plugins_mb),
        ] {
            if v == 0 {
                return Err(format!("`{name}` must be at least 1"));
            }
        }
        if self.file_mb() > 4096 || self.compact_mb > 4096 || self.max_log_mb > 4096 {
            return Err("sizes are in MiB and at most 4096".into());
        }
        // A bundle travels in one message (at most 64 MiB) next to its entry.
        if self.max_plugin_mb > 32 {
            return Err("`max_plugin_mb` is at most 32".into());
        }
        if self.max_plugins > 10_000 || self.max_plugins_mb > 1 << 20 {
            return Err("`max_plugins` is at most 10000 and `max_plugins_mb` at most 1048576".into());
        }
        Ok(())
    }

    /// The store's part.
    pub fn store(&self) -> StoreConfig {
        let mib = |n: u64| n.saturating_mul(1 << 20);
        StoreConfig {
            hosting: HostingConfig {
                open: self.open_publish,
                operator: self.operator_contact.trim().to_string(),
                terms: self.terms_url.trim().to_string(),
                // Whole seconds: `check` keeps the days finite and at most 3650.
                idle_secs: (self.idle_days * 86_400.0).round() as u64,
                max_file_mb: self.file_mb(),
                max_total_bytes: mib(self.max_total_mb),
                max_files_per_ip: self.max_files_per_ip,
                max_bytes_per_ip: mib(self.max_mb_per_ip),
                abuse: AbuseConfig {
                    open: self.open_publish,
                    connects_per_minute: self.connects_per_minute,
                    publishes_per_hour: self.publishes_per_hour,
                    messages_per_second: self.messages_per_second,
                },
            },
            max_files: self.max_files,
            max_file_bytes: mib(self.file_mb()),
            max_links: self.max_links,
            compact_after: self.compact_after,
            compact_bytes: mib(self.compact_mb),
            core: self.core(),
            directory: DirectoryConfig {
                max_plugins: self.max_plugins,
                max_plugin_bytes: mib(self.max_plugin_mb),
                max_total_bytes: mib(self.max_plugins_mb),
            },
            // At least 1 GiB, and room for two of the largest files (a compaction writes one beside
            // the old one).
            min_free_bytes: mib(self.file_mb().saturating_mul(2)).max(1 << 30),
        }
    }

    /// Largest message a client may send: a whole file (or plugin bundle) and a margin. Open to the
    /// public this is also the cap (strangers' files stay small); a private server never goes below
    /// the core's 64 MiB, and a `max_file_mb` above it raises the cap with it.
    fn open_message(&self) -> usize {
        usize::try_from(self.file_mb().max(self.max_plugin_mb).saturating_add(1).saturating_mul(1 << 20)).unwrap_or(usize::MAX)
    }

    /// The collaboration core's part: the batches kept for reconnecting clients (in memory, and
    /// in the file's snapshot) at most `max_log_mb`; when open, messages as large as a file at
    /// most, and those batches also at most a quarter of a file's bytes (256 KiB at least).
    fn core(&self) -> figcraft_collab::Limits {
        let bytes = |mb: u64| usize::try_from(mb.saturating_mul(1 << 20)).unwrap_or(usize::MAX);
        let base = figcraft_collab::Limits { max_peers: self.people(), max_log_bytes: bytes(self.max_log_mb), ..figcraft_collab::Limits::default() };
        if !self.open_publish {
            return figcraft_collab::Limits { max_message_bytes: base.max_message_bytes.max(self.open_message()), ..base };
        }
        let max = self.open_message();
        let log = (bytes(self.file_mb()) / 4).max(256 << 10).min(base.max_log_bytes);
        figcraft_collab::Limits { max_message_bytes: max, max_log_bytes: log, ..base }
    }

    /// The transport's part. A private server keeps the defaults; open to the public, connections
    /// per address are capped, messages are as large as a file at most, the bytes queued for a
    /// client and for everyone and those waiting for the hub are bounded, one-shot requests leave
    /// an eighth of the connections to sessions and are dropped after 20 idle seconds.
    pub fn net(&self) -> NetLimits {
        let base = NetLimits { max_connections: self.max_connections, trusted_proxy: self.proxy().ok().flatten(), ..NetLimits::default() };
        if !self.open_publish {
            let max = base.max_client_message.max(self.open_message());
            return NetLimits { max_client_message: max, max_queued_bytes: base.max_queued_bytes.max(max.saturating_add(1 << 20)), ..base };
        }
        let max = self.open_message();
        // Shared budgets: one block of addresses (an IPv4 address, an IPv6 /48) gets at most an
        // eighth of each, and its biggest consumers are dropped first. Each still fits one whole
        // message, or a file as large as `max_file_mb` could never arrive.
        let one = max.saturating_add(1 << 20);
        let (queued, reading) = ((256usize << 20).max(one.saturating_mul(8)), (192usize << 20).max(one.saturating_mul(8)));
        NetLimits {
            max_per_address: self.max_connections_per_ip,
            max_client_message: max,
            max_queued_bytes: one,
            max_queued_block: queued / 8,
            max_queued_total: queued,
            max_reading_block: one.min(reading / 8),
            max_reading_total: reading,
            max_in_flight: (128usize << 20).max(one),
            reserve_for_sessions: self.max_connections / 8,
            request_idle: std::time::Duration::from_secs(20),
            ..base
        }
    }
}
