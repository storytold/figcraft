use super::{Access, Arc, HashMap, IpAddr, Ipv6Addr, Mutex, PoisonError, SocketAddr};

/// What a link lets a hello do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Admission {
    pub access: Access,
    /// The link's id, when the backend has several per file (revoking it disconnects whoever
    /// joined with it).
    pub link: Option<String>,
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

/// The loopback address of `addr`'s family.
pub(super) fn loopback(addr: SocketAddr) -> std::net::IpAddr {
    if addr.is_ipv6() { std::net::Ipv6Addr::LOCALHOST.into() } else { std::net::Ipv4Addr::LOCALHOST.into() }
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
pub(super) struct PerAddress(pub(super) Arc<Mutex<HashMap<Block, usize>>>);

/// One connection counted for a block until dropped.
pub(super) struct Counted {
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
    pub(super) fn enter_all(&self, ip: IpAddr, cap: usize, all: usize) -> Option<Vec<Counted>> {
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
