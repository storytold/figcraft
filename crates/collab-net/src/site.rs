//! The page a session host serves to browsers on the same port as its WebSocket: the browser
//! build (`apps/figcraft-web`), so a phone or another computer on the network opens
//! `http://<host>:<port>/?view&join=<view link>` and watches the shared file (Share ▸ View on
//! another device). The files are compiled into the app ([`Site`]); nothing is read from disk.
//!
//! A plain `GET` without `Upgrade: websocket` is answered from the site and closed; everything
//! else (the WebSocket handshake) goes on untouched, since the request is only peeked at until it
//! is known to be one for the site. Only the site's own paths are served, gzipped (every browser
//! takes gzip; Brotli only over https), and the page loads nothing from anywhere else.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

/// Longest request head (request line and headers) a page request may have.
const MAX_HEAD: usize = 16 << 10;

/// One file of the site.
#[derive(Clone, Copy, Debug)]
pub struct SiteFile {
    /// Its path from the site's root, without a leading `/` (`index.html`, `figcraft-web-….js`).
    pub path: &'static str,
    /// Its `Content-Type`.
    pub content_type: &'static str,
    /// Its bytes, gzipped.
    pub gzip: &'static [u8],
}

/// The files a host serves to browsers (the browser build, compiled into the desktop app).
#[derive(Clone, Debug, Default)]
pub struct Site {
    files: Vec<SiteFile>,
}

impl Site {
    /// A site of `files`; `index.html` among them is the page at `/`.
    pub fn new(files: Vec<SiteFile>) -> Self {
        Self { files }
    }

    /// It has a page to open.
    pub fn has_page(&self) -> bool {
        self.file("index.html").is_some()
    }

    /// The file at `path` (`/` and `/index.html` are the page).
    pub fn file(&self, path: &str) -> Option<&SiteFile> {
        let path = path.trim_start_matches('/');
        let path = if path.is_empty() { "index.html" } else { path };
        self.files.iter().find(|f| f.path == path)
    }
}

/// The `Content-Type` of a site file named `path`.
pub fn content_type(path: &str) -> &'static str {
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "wasm" => "application/wasm",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    }
}

/// The address a phone opens to watch the session that `link` (a view-only invite link) joins,
/// hosted at `addr`: the page, which joins the link once it has loaded (`?view&join=…`; the link
/// goes last and keeps its own `&`s).
pub fn viewer_url(addr: SocketAddr, link: &str) -> String {
    format!("http://{addr}/?view&join={link}")
}

/// A request for the site, as far as it matters here.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PageRequest {
    /// The path, without the query.
    pub path: String,
    /// It asks for a WebSocket (`Upgrade: websocket`): not the site's.
    pub upgrade: bool,
    /// It takes gzip (`Accept-Encoding`).
    pub gzip: bool,
}

/// Read a whole request head (`GET` line and headers, without the blank line). `None`: not a
/// `GET`, or not one this reads.
pub(crate) fn parse_head(head: &[u8]) -> Option<PageRequest> {
    let head = std::str::from_utf8(head).ok()?;
    let mut lines = head.split("\r\n");
    let mut line = lines.next()?.split(' ');
    if line.next()? != "GET" {
        return None;
    }
    let target = line.next()?;
    let path = target.split(['?', '#']).next().unwrap_or_default().to_string();
    let (mut upgrade, mut gzip) = (false, false);
    for header in lines {
        let Some((name, value)) = header.split_once(':') else { continue };
        let (name, value) = (name.trim(), value.trim().to_ascii_lowercase());
        if name.eq_ignore_ascii_case("upgrade") && value.split(',').any(|v| v.trim() == "websocket") {
            upgrade = true;
        }
        if name.eq_ignore_ascii_case("accept-encoding") && value.split(',').any(|v| v.split(';').next().is_some_and(|e| e.trim() == "gzip")) {
            gzip = true;
        }
    }
    Some(PageRequest { path, upgrade, gzip })
}

/// The response to `req` from `site`: its head and body.
pub(crate) fn respond(site: &Site, req: &PageRequest) -> (Vec<u8>, &'static [u8]) {
    let Some(file) = site.file(&req.path) else {
        return (plain("404 Not Found", "not found\n"), &[]);
    };
    if !req.gzip {
        return (plain("406 Not Acceptable", "this page is sent gzipped: open it in a web browser\n"), &[]);
    }
    // The page carries no hash in its name (and its address the session's secret); the rest do.
    let cache = if file.path == "index.html" { "no-cache" } else { "public, max-age=31536000, immutable" };
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nCache-Control: {cache}\r\nVary: Accept-Encoding\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
        file.content_type,
        file.gzip.len(),
    );
    (head.into_bytes(), file.gzip)
}

/// A short plain-text response (`status` like `404 Not Found`) with its body in the head.
fn plain(status: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

/// A request for the site on `stream`: answered and closed (true; the connection is done), or
/// left untouched for the WebSocket handshake (false). The head is waited for until `until`.
pub(crate) fn serve(stream: &mut TcpStream, until: Instant, site: &Site) -> bool {
    let mut buf = vec![0u8; MAX_HEAD];
    let (req, len) = loop {
        let n = match stream.peek(&mut buf) {
            Ok(0) | Err(_) => return false,
            Ok(n) => n,
        };
        let seen = buf.get(..n).unwrap_or_default();
        if !seen.starts_with(b"GET ") && !b"GET ".starts_with(seen) {
            return false;
        }
        if let Some(end) = seen.windows(4).position(|w| w == b"\r\n\r\n") {
            let Some(req) = parse_head(seen.get(..end).unwrap_or_default()) else { return false };
            break (req, end + 4);
        }
        if n == buf.len() || Instant::now() >= until {
            return false;
        }
        // The rest of the head is on its way (it nearly always arrives in one piece).
        std::thread::sleep(Duration::from_millis(10));
    };
    if req.upgrade {
        return false;
    }
    // Take the head off the socket (a GET has no body), so closing doesn't reset the response.
    let mut head = vec![0u8; len];
    if stream.read_exact(&mut head).is_err() {
        return true;
    }
    let (head, body) = respond(site, &req);
    if let Err(e) = write_response(stream, &head, body) {
        log::debug!("live session: the page didn't go out: {e}");
    }
    let _ = stream.shutdown(std::net::Shutdown::Write);
    true
}

fn write_response(stream: &mut TcpStream, head: &[u8], body: &[u8]) -> io::Result<()> {
    stream.write_all(head)?;
    stream.write_all(body)?;
    stream.flush()
}
