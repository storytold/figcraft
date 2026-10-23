//! TLS for `wss://` addresses (native): rustls with ring, trusting Mozilla's root certificates
//! (compiled in) plus any in the PEM file `FIGCRAFT_CA_FILE` names (a private authority or a
//! self-signed server). Certificates are always verified.
//!
//! A connection's reader and writer threads share one TLS session ([`Tls`]), each holding it only
//! to decrypt or encrypt: the reader waits for bytes on its own socket handle without it, and a
//! sender takes the socket's write lock first (so records go out in the order they were made),
//! encrypts under the session's lock, then writes with only the socket's. Neither waits on the
//! network while holding the session, so a send stuck behind a full socket never stops the reader
//! from draining what arrives.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{CertificateError, ClientConfig, ClientConnection, RootCertStore};

/// The variable naming a PEM file of extra certificate authorities to trust.
pub const CA_FILE: &str = "FIGCRAFT_CA_FILE";

/// One connection's TLS session, shared by its reader and writer.
pub type Tls = Arc<Mutex<ClientConnection>>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Set up TLS on `stream`, connected to `host` (`name:port`), verifying its certificate.
pub fn handshake(stream: &TcpStream, host: &str) -> Result<Tls, String> {
    let name = host.rsplit_once(':').map_or(host, |(n, _)| n).trim_start_matches('[').trim_end_matches(']');
    let name = ServerName::try_from(name.to_string()).map_err(|_| format!("{name} isn't a name a certificate can be for"))?;
    let mut conn = ClientConnection::new(config()?, name).map_err(|e| e.to_string())?;
    // Writes are flushed to the socket at once, so the queue never grows.
    conn.set_buffer_limit(None);
    while conn.is_handshaking() {
        conn.complete_io(&mut &*stream).map_err(|e| explain(host, &e))?;
    }
    Ok(Arc::new(Mutex::new(conn)))
}

/// Why the TLS handshake with `host` failed, in words.
fn explain(host: &str, e: &io::Error) -> String {
    let why = |c: &CertificateError| match c {
        CertificateError::UnknownIssuer => format!(
            "isn't signed by an authority this app trusts (for a private or self-signed certificate, set {CA_FILE} to its authority's PEM file)"
        ),
        CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. } => "is for another name".into(),
        CertificateError::Expired | CertificateError::ExpiredContext { .. } => "has expired".into(),
        CertificateError::NotValidYet | CertificateError::NotValidYetContext { .. } => "isn't valid yet (is this computer's clock right?)".into(),
        CertificateError::Revoked => "has been revoked".into(),
        other => format!("isn't valid ({other:?})"),
    };
    match e.get_ref().and_then(|i| i.downcast_ref::<rustls::Error>()) {
        Some(rustls::Error::InvalidCertificate(c)) => format!("the certificate of {host} {}", why(c)),
        Some(other) => format!("no secure connection to {host} ({other}); is a TLS server answering there?"),
        None => format!("no secure connection to {host} ({e})"),
    }
}

/// The client settings, made once.
pub(crate) fn config() -> Result<Arc<ClientConfig>, String> {
    static CONFIG: OnceLock<Result<Arc<ClientConfig>, String>> = OnceLock::new();
    CONFIG.get_or_init(make_config).clone()
}

fn make_config() -> Result<Arc<ClientConfig>, String> {
    let mut roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    if let Some(path) = std::env::var_os(CA_FILE) {
        let shown = std::path::Path::new(&path).display().to_string();
        let certs: Vec<CertificateDer> = CertificateDer::pem_file_iter(&path)
            .and_then(Iterator::collect)
            .map_err(|e| format!("can't read the certificates in {CA_FILE} ({shown}): {e}"))?;
        if roots.add_parsable_certificates(certs).0 == 0 {
            return Err(format!("{CA_FILE} ({shown}) has no certificate this app can use"));
        }
    }
    #[cfg(test)]
    roots.add(CertificateDer::from(&include_bytes!("testdata/test-ca.der")[..])).map_err(|e| e.to_string())?;
    let config = ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

/// Read plaintext into `buf`: what the session has, else wait for bytes on `sock` (outside the
/// session's lock), take them in and try again. Anything the session must answer (alerts, key
/// updates) goes out through `out`.
pub fn read(tls: &Mutex<ClientConnection>, sock: &mut TcpStream, out: &Mutex<TcpStream>, buf: &mut [u8]) -> io::Result<usize> {
    loop {
        match lock(tls).reader().read(buf) {
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            done => return done,
        }
        // Wait for ciphertext without taking it off the socket, then let the session read only
        // what it takes: its plaintext buffer is empty here, so it never overflows (feeding it a
        // whole chunk could, when a record completes on top of plaintext not yet read).
        sock.peek(&mut [0u8; 1])?;
        let answer = {
            let mut c = lock(tls);
            // An empty read (the socket ended) tells the session so: the next read ends too.
            c.read_tls(sock)?;
            c.process_new_packets().map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            c.wants_write()
        };
        if answer {
            send(tls, out, &[])?;
        }
    }
}

/// Encrypt and send all of `buf`.
pub fn write(tls: &Mutex<ClientConnection>, out: &Mutex<TcpStream>, buf: &[u8]) -> io::Result<usize> {
    send(tls, out, buf)?;
    Ok(buf.len())
}

/// Encrypt `plain` and send it after whatever the session has waiting (see the module's docs for
/// the locking).
fn send(tls: &Mutex<ClientConnection>, out: &Mutex<TcpStream>, plain: &[u8]) -> io::Result<()> {
    let mut w = lock(out);
    let mut records = Vec::new();
    {
        let mut c = lock(tls);
        c.writer().write_all(plain)?;
        while c.wants_write() {
            c.write_tls(&mut records)?;
        }
    }
    w.write_all(&records)
}
