//! What one address may do how often: new connections, publishing without the key and messages
//! (only on a server open to the public), and reports, downloads and requests that change the
//! server's files (always, with the server key too). Each is a token
//! bucket per block an address counts in ([`ip_blocks`]: an IPv4 address, or an IPv6 /64 and its
//! /56 and /48 with 2 and 4 times the rate), so a tunnel's /48 doesn't get 65,536 /64s' worth. Publishing has a budget for
//! everyone besides ([`PUBLISHES_ALL`] times one address's). The table keeps at most
//! [`MAX_ADDRESSES`] addresses: when it is full the half used longest ago is forgotten, so a flood
//! of addresses costs bounded memory (and forgotten addresses start with full buckets).
//!
//! Join codes are short, so guessing them is held back three ways: an address may try
//! [`CODES_PER_MINUTE`] codes a minute, [`WRONG_CODES`] wrong ones in a row lock it out of codes for
//! [`CODE_LOCKOUT`] (links still join), and everyone together may get [`WRONG_CODES_PER_HOUR`]
//! wrong an hour (an attacker with many addresses is slowed to that; while it lasts, people join
//! with their links).

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use figcraft_collab_net::hub::{Block, ip_blocks};
use figcraft_collab_net::net::Bucket;

/// Most addresses remembered.
pub const MAX_ADDRESSES: usize = 10_000;

/// Reports one address may send an hour.
pub const REPORTS_PER_HOUR: f64 = 10.0;

/// Bytes one address may download a second (team libraries, plugins), and at once: an IPv6 /48
/// gets four times that, an eighth of everyone's budgets (32 MiB a second, 512 MiB at once).
pub const DOWNLOAD_RATE: f64 = (1u64 << 20) as f64;
pub const DOWNLOAD_BURST: f64 = (16u64 << 20) as f64;

/// Everyone together may publish without the key this many times what one address may: eight
/// times what the widest block (an IPv6 /48, four addresses' worth) may, so no block takes it all.
pub const PUBLISHES_ALL: f64 = 32.0;

/// Requests that change the server's files (join codes, links, renames, removals, blocks,
/// publishing with the key) one address may make a minute, with the server key or an owner token.
pub const ADMIN_PER_MINUTE: f64 = 60.0;

/// Join codes one address may try a minute (right or wrong).
pub const CODES_PER_MINUTE: f64 = 10.0;
/// Wrong codes in a row (within [`CODE_LOCKOUT`] of each other) that lock an address out of codes.
pub const WRONG_CODES: u32 = 10;
/// How long an address is locked out of join codes.
pub const CODE_LOCKOUT: Duration = Duration::from_secs(15 * 60);
/// Wrong codes everyone together may try an hour.
pub const WRONG_CODES_PER_HOUR: f64 = 1_000.0;

/// The rates (see the server's settings).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AbuseConfig {
    /// Open to the public: connections, publishing and messages are limited.
    pub open: bool,
    pub connects_per_minute: u32,
    pub publishes_per_hour: u32,
    pub messages_per_second: u32,
}

/// What an address does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Connect,
    Publish,
    Message,
    /// Presence (cursors, selections, viewports), cursor chat and pings: twice `messages_per_second`.
    Presence,
    Report,
    /// Downloading this many bytes.
    Download(usize),
    /// Trying a join code.
    Code,
    /// A request that changes the server's files ([`ADMIN_PER_MINUTE`]).
    Admin,
}

struct Address {
    connects: Bucket,
    publishes: Bucket,
    messages: Bucket,
    presence: Bucket,
    reports: Bucket,
    downloads: Bucket,
    codes: Bucket,
    admin: Bucket,
    /// Wrong join codes in a row, and when the last one was.
    wrong: u32,
    wrong_at: Instant,
    /// Locked out of join codes until then.
    locked: Option<Instant>,
    used: Instant,
}

/// The buckets of the addresses seen lately.
pub struct Abuse {
    cfg: AbuseConfig,
    table: HashMap<Block, Address>,
    /// Publishing without the key, everyone together.
    publishes: Bucket,
    /// Wrong join codes, everyone together.
    wrong_codes: Bucket,
}

/// `n` every `period` seconds on average, and all `n` at once at most.
fn bucket(n: f64, period: f64) -> Bucket {
    Bucket::new(n / period, n.max(1.0))
}

impl Abuse {
    pub fn new(cfg: AbuseConfig) -> Self {
        let publishes = bucket(f64::from(cfg.publishes_per_hour) * PUBLISHES_ALL, 3600.0);
        Self { cfg, table: HashMap::new(), publishes, wrong_codes: bucket(WRONG_CODES_PER_HOUR, 3600.0) }
    }

    /// How long `ip` is still locked out of join codes (in any block it is in), if it is.
    pub fn code_locked(&self, ip: IpAddr, now: Instant) -> Option<Duration> {
        let until = ip_blocks(ip).into_iter().filter_map(|(k, _)| self.table.get(&k)?.locked).max()?;
        Some(until.saturating_duration_since(now)).filter(|d| !d.is_zero())
    }

    /// Everyone together has wrong codes left to try (see [`WRONG_CODES_PER_HOUR`]).
    pub fn codes_open(&mut self, now: Instant) -> bool {
        let open = self.wrong_codes.take(now);
        if open {
            self.wrong_codes.give(1.0);
        }
        open
    }

    /// `ip` tried a join code that was right (`ok`) or wrong: a right one doesn't forgive the wrong
    /// ones before it (they only decay with time); enough wrong ones lock the address out.
    pub fn code_tried(&mut self, ip: IpAddr, ok: bool, now: Instant) {
        if !ok {
            self.wrong_codes.take(now);
        }
        for (k, times) in ip_blocks(ip) {
            let a = self.address(k, f64::from(times), now);
            if now.saturating_duration_since(a.wrong_at) > CODE_LOCKOUT {
                a.wrong = 0;
            }
            if !ok {
                a.wrong += 1;
                a.wrong_at = now;
                if a.wrong >= WRONG_CODES.saturating_mul(times) {
                    a.wrong = 0;
                    a.locked = Some(now + CODE_LOCKOUT);
                }
            }
        }
    }

    /// Whether `ip` may do `what` now (counting it, in every block it is in, when it may).
    pub fn allow(&mut self, ip: IpAddr, what: Action, now: Instant) -> bool {
        if !self.cfg.open && matches!(what, Action::Connect | Action::Publish | Action::Message | Action::Presence) {
            return true;
        }
        let keys: Vec<(Block, f64)> = ip_blocks(ip).into_iter().map(|(k, times)| (k, f64::from(times))).collect();
        // Every block must have room before any is charged.
        if !keys.iter().all(|&(k, times)| self.room(k, times, what, now)) {
            return false;
        }
        if what == Action::Publish && !self.publishes.take(now) {
            return false;
        }
        for &(k, times) in &keys {
            let (b, n) = Self::pick(self.address(k, times, now), what);
            b.take_n(now, n);
        }
        true
    }

    /// The address's buckets, made when it is new (forgetting the oldest half when full).
    fn address(&mut self, ip: Block, times: f64, now: Instant) -> &mut Address {
        if !self.table.contains_key(&ip) && self.table.len() >= MAX_ADDRESSES {
            self.forget_oldest_half();
        }
        let c = self.cfg;
        let a = self.table.entry(ip).or_insert_with(|| Address {
            connects: bucket(f64::from(c.connects_per_minute) * times, 60.0),
            publishes: bucket(f64::from(c.publishes_per_hour) * times, 3600.0),
            // A burst of four seconds' worth (a big paste, a quick drag).
            messages: Bucket::new(f64::from(c.messages_per_second) * times, f64::from(c.messages_per_second) * times * 4.0),
            presence: Bucket::new(f64::from(c.messages_per_second) * times * 2.0, f64::from(c.messages_per_second) * times * 8.0),
            reports: bucket(REPORTS_PER_HOUR * times, 3600.0),
            downloads: Bucket::new(DOWNLOAD_RATE * times, DOWNLOAD_BURST * times),
            codes: bucket(CODES_PER_MINUTE * times, 60.0),
            admin: bucket(ADMIN_PER_MINUTE * times, 60.0),
            wrong: 0,
            wrong_at: now,
            locked: None,
            used: now,
        });
        a.used = now;
        a
    }

    fn pick(a: &mut Address, what: Action) -> (&mut Bucket, f64) {
        match what {
            Action::Connect => (&mut a.connects, 1.0),
            Action::Publish => (&mut a.publishes, 1.0),
            Action::Message => (&mut a.messages, 1.0),
            Action::Presence => (&mut a.presence, 1.0),
            Action::Report => (&mut a.reports, 1.0),
            Action::Download(n) => (&mut a.downloads, n as f64),
            Action::Code => (&mut a.codes, 1.0),
            Action::Admin => (&mut a.admin, 1.0),
        }
    }

    /// The block has `what`'s tokens now (nothing is spent).
    fn room(&mut self, ip: Block, times: f64, what: Action, now: Instant) -> bool {
        let (b, n) = Self::pick(self.address(ip, times, now), what);
        if b.take_n(now, n) {
            b.give(n);
            return true;
        }
        false
    }

    /// Addresses remembered.
    pub fn len(&self) -> usize {
        self.table.len()
    }

    /// None remembered.
    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    fn forget_oldest_half(&mut self) {
        let mut used: Vec<Instant> = self.table.values().map(|a| a.used).collect();
        used.sort_unstable();
        let Some(&cut) = used.get(used.len() / 2) else { return };
        self.table.retain(|_, a| a.used > cut);
    }
}
