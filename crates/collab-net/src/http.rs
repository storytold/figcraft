//! Downloading one file over HTTPS (fonts from a font library, #754): an HTTP/1.0 GET, so the
//! answer is the body up to the end of the connection, never chunked. Certificates are verified as
//! for `wss://` ([`crate::tls`]); redirects aren't followed; only a 200 is a file and a 404 is
//! none.

use std::io::{self, Read, Write};
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{ClientConnection, StreamOwned};

use crate::limits::NetLimits;
use crate::net::dial;

/// The most a response's status line and headers may take.
const MAX_HEAD: usize = 64 << 10;
/// How long a read may wait for the server.
const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a whole download may take.
const DEADLINE: Duration = Duration::from_secs(120);

/// `https://host[:port]/path`: the host and port to connect to, the name its certificate is for
/// and the path to ask for.
fn parse(url: &str) -> Result<(String, String, String), String> {
    let rest = url.strip_prefix("https://").ok_or_else(|| format!("{url} isn't an https:// address"))?;
    let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    if authority.is_empty() || authority.contains(['@', ' ', '?', '#']) || rest.contains(|c: char| c == ' ' || c.is_control()) {
        return Err(format!("{url} isn't an address FigCraft can download from"));
    }
    // `name:port`, `[v6]:port`, or no port.
    let (name, port) = match authority.rsplit_once(':') {
        Some((n, p)) if !n.contains(':') || n.ends_with(']') => (n, p.parse::<u16>().map_err(|_| format!("{url} has no valid port"))?),
        _ => (authority, 443),
    };
    let host = format!("{name}:{port}");
    // A `#fragment` is for the reader, never sent to the server.
    let path = path.split('#').next().unwrap_or_default();
    Ok((host, name.trim_matches(['[', ']']).to_string(), if path.is_empty() { "/".into() } else { path.into() }))
}

/// The body of a complete response `raw` to a GET: `None` for a 404. `clean` is whether the
/// server ended with TLS's goodbye; without it only a Content-Length shows the body is whole.
fn body(raw: &[u8], max: usize, clean: bool) -> Result<Option<Vec<u8>>, String> {
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut res = httparse::Response::new(&mut headers);
    let head = match res.parse(raw) {
        Ok(httparse::Status::Complete(n)) => n,
        Ok(httparse::Status::Partial) => return Err("the server's answer ended early".into()),
        Err(e) => return Err(format!("the server's answer can't be read ({e})")),
    };
    if head > MAX_HEAD {
        return Err(format!("the server's answer has more than {MAX_HEAD} bytes of headers"));
    }
    match res.code {
        Some(200) => {}
        Some(404) => return Ok(None),
        Some(code) => return Err(format!("the server answered {code}")),
        None => return Err("the server's answer has no status".into()),
    }
    // An HTTP/1.0 answer is the bytes as they are: chunked or compressed ones would be saved as junk.
    // Every occurrence of a header counts: a second one could undo what the first said.
    let header = |name: &'static str| {
        res.headers.iter().filter(move |h| h.name.eq_ignore_ascii_case(name)).map(|h| String::from_utf8_lossy(h.value).trim().to_string())
    };
    if let Some(te) = header("transfer-encoding").next() {
        return Err(format!("the server sent the file as {te}"));
    }
    if let Some(ce) = header("content-encoding").find(|ce| !ce.eq_ignore_ascii_case("identity")) {
        return Err(format!("the server sent the file as {ce}"));
    }
    let body = raw.get(head..).unwrap_or_default();
    let too_big = || format!("the file is larger than {max} bytes");
    let mut lengths = header("content-length");
    if let Some(v) = lengths.next() {
        if lengths.any(|other| other != v) {
            return Err("the server sent conflicting Content-Lengths".into());
        }
        let n: usize = v.parse().map_err(|_| "the server sent a bad Content-Length")?;
        if n > max {
            return Err(too_big());
        }
        if body.len() < n {
            return Err("the download ended early".into());
        }
        if body.len() > n {
            return Err("the server sent more than its Content-Length".into());
        }
    } else if !clean {
        return Err("the download ended early".into());
    }
    if body.len() > max {
        return Err(too_big());
    }
    Ok(Some(body.to_vec()))
}

/// Download `url` (`https://` only), at most `max` bytes: `None` when the server has no such file.
pub fn get(url: &str, max: usize, limits: &NetLimits, connect: Duration) -> Result<Option<Vec<u8>>, String> {
    let (host, name, path) = parse(url)?;
    let tcp = dial(&host, connect, limits)?;
    tcp.set_read_timeout(Some(READ_TIMEOUT)).map_err(|e| e.to_string())?;
    let server = ServerName::try_from(name.clone()).map_err(|_| format!("{name} isn't a name a certificate can be for"))?;
    let conn = ClientConnection::new(crate::tls::config()?, server).map_err(|e| e.to_string())?;
    let mut tls = StreamOwned::new(conn, tcp);
    let agent = concat!("FigCraft/", env!("CARGO_PKG_VERSION"));
    // `Host` names the port unless it's the default, and keeps an IPv6 address's brackets.
    let authority = host.strip_suffix(":443").unwrap_or(&host);
    let request =
        format!("GET {path} HTTP/1.0\r\nHost: {authority}\r\nUser-Agent: {agent}\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n");
    tls.write_all(request.as_bytes()).map_err(|e| format!("no secure connection to {host} ({e})"))?;
    let cap = max.saturating_add(MAX_HEAD);
    let mut raw = Vec::new();
    let mut buf = vec![0u8; 64 << 10];
    let start = std::time::Instant::now();
    let mut clean = true;
    loop {
        if start.elapsed() > DEADLINE {
            return Err(format!("the download from {host} took longer than {} s", DEADLINE.as_secs()));
        }
        match tls.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                raw.extend_from_slice(buf.get(..n).unwrap_or_default());
                if raw.len() > cap {
                    return Err(format!("the file is larger than {max} bytes"));
                }
            }
            // A server that closes without TLS's goodbye: only a Content-Length can tell a cut-off
            // answer from a whole one.
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                clean = false;
                break;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(format!("the download from {host} failed ({e})")),
        }
    }
    body(&raw, max, clean)
}

#[cfg(test)]
#[path = "http_tests.rs"]
mod tests;
