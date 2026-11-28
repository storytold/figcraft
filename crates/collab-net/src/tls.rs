//! TLS for `wss://` addresses (native): rustls with ring, trusting the public root certificates of
//! webpki-roots (compiled in), the authorities this computer trusts (its certificate store, unless
//! `FIGCRAFT_NO_SYSTEM_ROOTS` is set) and any in the PEM file `FIGCRAFT_CA_FILE` names (a private
//! authority or a self-signed server). Certificates are always verified.
//!
//! A connection's reader and writer threads share one TLS session ([`Tls`]), each holding it only
//! to decrypt or encrypt: the reader waits for bytes on its own socket handle without it, and a
//! sender takes the socket's write lock first (so records go out in the order they were made),
//! encrypts under the session's lock, then writes with only the socket's. Neither waits on the
//! network while holding the session, so a send stuck behind a full socket never stops the reader
//! from draining what arrives. The session is a client's or, for an app hosting on the network
//! with its own certificate ([`crate::cert`]), a server's ([`accept`]).
//!
//! A `wss://` address with a `cert=` pin ([`handshake_pinned`]) trusts no authority: the server
//! must present exactly the certificate whose SHA-256 the invite names, and prove it holds its key.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Instant;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig, ClientConnection, Connection, DigitallySignedStruct, RootCertStore, ServerConfig, ServerConnection,
    SignatureScheme,
};

use crate::cert::{Fingerprint, fingerprint};

/// The variable naming a PEM file of extra certificate authorities to trust.
pub const CA_FILE: &str = "FIGCRAFT_CA_FILE";

/// The variable that, set, leaves the operating system's certificate store out.
pub const NO_SYSTEM_ROOTS: &str = "FIGCRAFT_NO_SYSTEM_ROOTS";

/// One connection's TLS session (a client's or a server's), shared by its reader and writer.
pub type Tls = Arc<Mutex<Connection>>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The certificate name in `host` (`name:port`, `[v6]:port`).
fn server_name(host: &str) -> Result<ServerName<'static>, String> {
    let name = host.rsplit_once(':').map_or(host, |(n, _)| n).trim_start_matches('[').trim_end_matches(']');
    ServerName::try_from(name.to_string()).map_err(|_| format!("{name} isn't a name a certificate can be for"))
}

/// Set up TLS on `stream`, connected to `host` (`name:port`), verifying its certificate.
pub fn handshake(stream: &TcpStream, host: &str) -> Result<Tls, String> {
    let conn = ClientConnection::new(config()?, server_name(host)?).map_err(|e| e.to_string())?;
    client_handshake(stream, conn).map_err(|e| explain(host, &e))
}

/// Set up TLS on `stream`, connected to `host`, accepting only the certificate whose SHA-256 is
/// `expected` (an invite's `cert=`): no authority is asked, and any other certificate (someone in
/// between) is refused before anything is sent.
pub fn handshake_pinned(stream: &TcpStream, host: &str, expected: &Fingerprint) -> Result<Tls, String> {
    let conn = ClientConnection::new(pinned_config(expected)?, server_name(host)?).map_err(|e| e.to_string())?;
    client_handshake(stream, conn).map_err(|e| match rustls_error(&e) {
        Some(rustls::Error::InvalidCertificate(CertificateError::ApplicationVerificationFailure)) => format!(
            "the certificate of {host} isn't the one the invite names: someone may be in between, or the session restarted (ask for a new invite link)"
        ),
        _ => explain(host, &e),
    })
}

fn client_handshake(stream: &TcpStream, mut conn: ClientConnection) -> io::Result<Tls> {
    // Writes are flushed to the socket at once, so the queue never grows.
    conn.set_buffer_limit(None);
    while conn.is_handshaking() {
        conn.complete_io(&mut &*stream)?;
    }
    Ok(Arc::new(Mutex::new(Connection::Client(conn))))
}

/// Answer a client's TLS handshake on `stream` with `config`, by `deadline` however slowly its
/// bytes come.
pub fn accept(stream: &TcpStream, config: Arc<ServerConfig>, deadline: Instant) -> io::Result<Tls> {
    let mut conn = ServerConnection::new(config).map_err(io::Error::other)?;
    conn.set_buffer_limit(None);
    while conn.is_handshaking() {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        stream.set_read_timeout(Some(left))?;
        conn.complete_io(&mut &*stream)?;
    }
    Ok(Arc::new(Mutex::new(Connection::Server(conn))))
}

fn rustls_error(e: &io::Error) -> Option<&rustls::Error> {
    e.get_ref().and_then(|i| i.downcast_ref::<rustls::Error>())
}

/// Why the TLS handshake with `host` failed, in words.
fn explain(host: &str, e: &io::Error) -> String {
    let why = |c: &CertificateError| match c {
        CertificateError::UnknownIssuer => format!(
            "isn't signed by an authority this app trusts (for a private or self-signed certificate, add its authority to this computer's trusted certificates, or set {CA_FILE} to its authority's PEM file)"
        ),
        CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. } => "is for another name".into(),
        CertificateError::Expired | CertificateError::ExpiredContext { .. } => "has expired".into(),
        CertificateError::NotValidYet | CertificateError::NotValidYetContext { .. } => "isn't valid yet (is this computer's clock right?)".into(),
        CertificateError::Revoked => "has been revoked".into(),
        other => format!("isn't valid ({other:?})"),
    };
    match rustls_error(e) {
        Some(rustls::Error::InvalidCertificate(c)) => format!("the certificate of {host} {}", why(c)),
        Some(other) => format!("no secure connection to {host} ({other}); is a TLS server answering there?"),
        None => format!("no secure connection to {host} ({e})"),
    }
}

/// The client settings, made once.
fn config() -> Result<Arc<ClientConfig>, String> {
    static CONFIG: OnceLock<Result<Arc<ClientConfig>, String>> = OnceLock::new();
    CONFIG.get_or_init(make_config).clone()
}

fn make_config() -> Result<Arc<ClientConfig>, String> {
    let mut roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    // A private authority an administrator installed (a company's, a proxy's) is trusted as the
    // computer's other apps trust it (#2342). An unreadable store adds nothing.
    if std::env::var_os(NO_SYSTEM_ROOTS).is_none() {
        let system = rustls_native_certs::load_native_certs();
        for e in &system.errors {
            log::warn!("system certificate store: {e}");
        }
        roots.add_parsable_certificates(system.certs);
    }
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
    let config = ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

/// The ring provider every connection uses.
fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Client settings trusting only the certificate `expected` is the SHA-256 of.
fn pinned_config(expected: &Fingerprint) -> Result<Arc<ClientConfig>, String> {
    let provider = provider();
    let verifier = Pinned { expected: *expected, algorithms: provider.signature_verification_algorithms };
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    Ok(Arc::new(config))
}

/// Accepts the one certificate an invite pins, and only a handshake signed with its key.
#[derive(Debug)]
struct Pinned {
    expected: Fingerprint,
    algorithms: WebPkiSupportedAlgorithms,
}

/// `a == b`, in a time that doesn't depend on where they differ.
fn same(a: &Fingerprint, b: &Fingerprint) -> bool {
    a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if same(&fingerprint(end_entity.as_ref()), &self.expected) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(CertificateError::ApplicationVerificationFailure))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// Read plaintext into `buf`: what the session has, else wait for bytes on `sock` (outside the
/// session's lock), take them in and try again. Anything the session must answer (alerts, key
/// updates) goes out through `out`.
pub fn read(tls: &Mutex<Connection>, sock: &mut TcpStream, out: &Mutex<TcpStream>, buf: &mut [u8]) -> io::Result<usize> {
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
pub fn write(tls: &Mutex<Connection>, out: &Mutex<TcpStream>, buf: &[u8]) -> io::Result<usize> {
    send(tls, out, buf)?;
    Ok(buf.len())
}

/// Encrypt `plain` and send it after whatever the session has waiting (see the module's docs for
/// the locking).
fn send(tls: &Mutex<Connection>, out: &Mutex<TcpStream>, plain: &[u8]) -> io::Result<()> {
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
