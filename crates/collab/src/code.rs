//! Join codes: the short text someone types to join a shared file, in place of an invite link.
//!
//! Both kinds use Crockford's base32 alphabet (digits and capitals without `I`, `L`, `O`, `U`), so
//! a code survives being read aloud or copied by hand: reading one ignores case, spaces and dashes
//! and takes `O` as `0` and `I` or `L` as `1`.
//!
//! - An **online** code ([`ONLINE_LEN`] symbols, 45 random bits, shown `K7M-2QX-9TD`) is a name a
//!   server gave one share link: the server is asked which file it opens.
//! - A **local network** code ([`LOCAL_LEN`] symbols, shown in fives) has no server to ask, so it
//!   carries what a joiner needs ([`Local`]): the host's IPv4 address, its port, the session's
//!   44-bit secret and 8 check bits that catch typing mistakes before anything connects.

use std::net::Ipv4Addr;

/// Crockford's base32 symbols, by value.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Symbols in an online code.
pub const ONLINE_LEN: usize = 9;
/// Symbols in a local network code.
pub const LOCAL_LEN: usize = 20;
/// Bits of a local session's secret.
const SECRET_BITS: u32 = 44;
/// Longest text read as a code (anything longer isn't one).
const MAX_TYPED: usize = 128;

/// The file id every local session uses (its port tells sessions apart).
pub const LOCAL_FILE: &str = "local";
const TOKEN_PREFIX: &str = "local-";

/// The value of a typed symbol: its own, or the digit it is mistaken for.
fn value(c: char) -> Option<u8> {
    let c = match c.to_ascii_uppercase() {
        'O' => '0',
        'I' | 'L' => '1',
        c => c,
    };
    ALPHABET.iter().position(|a| char::from(*a) == c).and_then(|i| u8::try_from(i).ok())
}

/// The symbol for the low five bits of `v`.
fn symbol(v: u8) -> char {
    char::from(ALPHABET.get(usize::from(v & 31)).copied().unwrap_or(b'0'))
}

/// The lowest `n` symbols of `v`, most significant first.
fn symbols(v: u128, n: usize) -> String {
    (0..n).rev().map(|i| symbol((v >> (5 * i)).to_le_bytes()[0])).collect()
}

/// What someone typed, as a code's symbols: capitals, without spaces and dashes, look-alikes
/// read as the digits they stand for. Errors say what to fix (they never echo the text).
pub fn normalize(text: &str) -> Result<String, String> {
    if text.len() > MAX_TYPED {
        return Err("that is too long to be a join code".into());
    }
    let typed = text.chars().filter(|c| !c.is_whitespace() && !matches!(c, '-' | '–' | '—' | '_' | '.'));
    let code = typed
        .map(|c| value(c).map(symbol))
        .collect::<Option<String>>()
        .ok_or("a join code has only letters and digits (and no U): check it and try again")?;
    if code.is_empty() {
        return Err("type the join code you were given".into());
    }
    Ok(code)
}

/// A code as it is shown: in groups, with dashes (`K7M-2QX-9TD`; local codes in fives).
pub fn grouped(code: &str) -> String {
    let n = if code.len() == ONLINE_LEN { 3 } else { 5 };
    let chars: Vec<char> = code.chars().collect();
    chars.chunks(n).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join("-")
}

/// An online code from 45 random bits (the rest of `random` is ignored).
pub fn online(random: u64) -> String {
    symbols(u128::from(random), ONLINE_LEN)
}

/// What a local network code carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Local {
    pub ip: Ipv4Addr,
    pub port: u16,
    /// The session's secret ([`SECRET_BITS`] bits).
    pub secret: u64,
}

impl Local {
    /// For a session at `ip:port` with a secret made from `random` (its low 44 bits).
    pub fn new(ip: Ipv4Addr, port: u16, random: u64) -> Self {
        Self { ip, port, secret: random & ((1 << SECRET_BITS) - 1) }
    }

    /// Address, port and secret as one number (92 bits).
    fn bits(&self) -> u128 {
        (u128::from(u32::from(self.ip)) << 60) | (u128::from(self.port) << SECRET_BITS) | u128::from(self.secret)
    }

    /// The code's symbols (show them [`grouped`]).
    pub fn code(&self) -> String {
        let bits = self.bits();
        symbols((bits << 8) | u128::from(check(bits)), LOCAL_LEN)
    }

    /// Read a [`normalize`]d local code.
    fn read(code: &str) -> Result<Local, String> {
        const TYPO: &str = "that local network code has a typing mistake: check it and try again";
        let v = code.chars().try_fold(0u128, |v, c| value(c).map(|s| (v << 5) | u128::from(s))).ok_or(TYPO)?;
        let bits = v >> 8;
        let local = Local {
            ip: Ipv4Addr::from(u32::try_from(bits >> 60).map_err(|_| TYPO)?),
            port: u16::try_from((bits >> SECRET_BITS) & 0xFFFF).map_err(|_| TYPO)?,
            secret: u64::try_from(bits & ((1 << SECRET_BITS) - 1)).map_err(|_| TYPO)?,
        };
        if u128::from(check(bits)) != v & 0xFF || local.port == 0 || local.ip.is_unspecified() {
            return Err(TYPO.into());
        }
        Ok(local)
    }

    /// The session's token: what its host checks a hello against.
    pub fn token(&self) -> String {
        format!("{TOKEN_PREFIX}{:011x}", self.secret)
    }

    /// The session's invite link (what a joiner's app connects with).
    pub fn link(&self) -> String {
        format!("figcraft://join?host={}:{}&file={LOCAL_FILE}&token={}", self.ip, self.port, self.token())
    }

    /// The session behind a local session's invite link (`None` for any other link: only IPv4
    /// hosts with a code-sized secret have a code).
    pub fn from_link(link: &str) -> Option<Local> {
        let query = link.strip_prefix("figcraft://join?")?;
        let get = |key: &str| query.split('&').find_map(|kv| kv.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| v));
        let addr: std::net::SocketAddrV4 = get("host")?.parse().ok()?;
        let hex = get("token")?.strip_prefix(TOKEN_PREFIX).filter(|h| h.len() == 11 && h.bytes().all(|b| b.is_ascii_hexdigit()))?;
        let local = Local { ip: *addr.ip(), port: addr.port(), secret: u64::from_str_radix(hex, 16).ok()? };
        (get("file") == Some(LOCAL_FILE) && local.port != 0).then_some(local)
    }
}

/// Eight check bits over a local code's 92 bits (FNV-1a, folded): not a secret, only a typo catcher.
fn check(bits: u128) -> u8 {
    let h = bits.to_be_bytes().iter().fold(0x811C_9DC5_u32, |h, b| (h ^ u32::from(*b)).wrapping_mul(0x0100_0193));
    h.to_be_bytes().iter().fold(0, |a, b| a ^ b)
}

/// A code someone typed, by kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Code {
    /// A server's code (its symbols): the server says which file it opens.
    Online(String),
    Local(Local),
}

/// Read what someone typed as a join code of either kind.
pub fn parse(text: &str) -> Result<Code, String> {
    let code = normalize(text)?;
    match code.len() {
        ONLINE_LEN => Ok(Code::Online(code)),
        LOCAL_LEN => Local::read(&code).map(Code::Local),
        n => Err(format!("a join code has {ONLINE_LEN} characters ({LOCAL_LEN} on the local network); that one has {n}")),
    }
}

#[cfg(test)]
#[path = "code_tests.rs"]
mod tests;
