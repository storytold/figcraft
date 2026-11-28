//! A session's own certificate (native): a fresh Ed25519 key and a self-signed X.509 v3
//! certificate for it, made when an app starts hosting on the network and gone when it stops.
//! Nobody vouches for it; the invite link carries its SHA-256 instead (`cert=`), and a guest
//! connects only to a host presenting exactly that certificate ([`crate::tls::handshake_pinned`]).
//!
//! The certificate is written out as DER by hand (a few fixed structures), so hosting needs no
//! certificate library: its validity runs from 2000 to 9999-12-31 (RFC 5280's "no expiry"), as
//! the pin, not a date, is what's checked.

use std::sync::Arc;

use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{Ed25519KeyPair, KeyPair};
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

/// The SHA-256 of a certificate's DER: what an invite pins.
pub type Fingerprint = [u8; 32];

/// A session's certificate, its private key and its fingerprint.
pub struct SessionCert {
    pub cert: CertificateDer<'static>,
    pub key: PrivateKeyDer<'static>,
    pub sha256: Fingerprint,
}

/// The SHA-256 of `der`.
pub fn fingerprint(der: &[u8]) -> Fingerprint {
    let mut out = [0u8; 32];
    out.copy_from_slice(ring::digest::digest(&ring::digest::SHA256, der).as_ref());
    out
}

/// `id-Ed25519` (1.3.101.112), as an AlgorithmIdentifier: no parameters (RFC 8410).
const ED25519: &[u8] = &[0x30, 0x05, 0x06, 0x03, 0x2B, 0x65, 0x70];

/// A DER element: `tag`, its length, `body`.
fn der(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    match u8::try_from(body.len()) {
        Ok(n) if n < 0x80 => out.push(n),
        _ => {
            // Long form: the length's bytes without leading zeros, after their count.
            let bytes = body.len().to_be_bytes();
            let len: Vec<u8> = bytes.iter().copied().skip_while(|b| *b == 0).collect();
            out.push(0x80 | u8::try_from(len.len()).unwrap_or(0));
            out.extend_from_slice(&len);
        }
    }
    out.extend_from_slice(body);
    out
}

fn seq(parts: &[&[u8]]) -> Vec<u8> {
    der(0x30, &parts.concat())
}

/// A BIT STRING of whole bytes.
fn bits(b: &[u8]) -> Vec<u8> {
    der(0x03, &[&[0u8][..], b].concat())
}

/// `CN=<name>`.
fn name(cn: &str) -> Vec<u8> {
    let attr = seq(&[&der(0x06, &[0x55, 0x04, 0x03]), &der(0x0C, cn.as_bytes())]);
    seq(&[&der(0x31, &attr)])
}

/// The to-be-signed part of a certificate for `public` (32 bytes) with `serial`.
fn tbs(serial: &[u8; 16], public: &[u8]) -> Vec<u8> {
    let version = der(0xA0, &der(0x02, &[2]));
    let subject = name("FigCraft live session");
    let validity = seq(&[&der(0x17, b"000101000000Z"), &der(0x18, b"99991231235959Z")]);
    let spki = seq(&[ED25519, &bits(public)]);
    // basicConstraints (2.5.29.19), critical: not an authority.
    let basic = seq(&[&der(0x06, &[0x55, 0x1D, 0x13]), &der(0x01, &[0xFF]), &der(0x04, &seq(&[]))]);
    let extensions = der(0xA3, &seq(&[&basic]));
    seq(&[&version, &der(0x02, serial), ED25519, &subject, &validity, &subject, &spki, &extensions])
}

/// A fresh key and a self-signed certificate for it.
pub fn generate() -> Result<SessionCert, String> {
    let rng = SystemRandom::new();
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).map_err(|_| "can't make a key for the session's certificate")?;
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).map_err(|_| "can't read the session's new key")?;
    let mut serial = [0u8; 16];
    rng.fill(&mut serial).map_err(|_| "no randomness for the session's certificate")?;
    // A positive INTEGER with no leading zero byte.
    serial[0] = (serial[0] & 0x7F) | 0x01;
    let tbs = tbs(&serial, pair.public_key().as_ref());
    let signature = pair.sign(&tbs);
    let cert = seq(&[&tbs, ED25519, &bits(signature.as_ref())]);
    let sha256 = fingerprint(&cert);
    Ok(SessionCert { cert: CertificateDer::from(cert), key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(pkcs8.as_ref().to_vec())), sha256 })
}

/// The server settings presenting `cert`.
pub fn server_config(cert: &SessionCert) -> Result<Arc<ServerConfig>, String> {
    let cfg = ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(vec![cert.cert.clone()], cert.key.clone_key())
        .map_err(|e| format!("the session's certificate isn't usable: {e}"))?;
    Ok(Arc::new(cfg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_lengths_use_the_fewest_bytes() {
        assert_eq!(der(0x04, &[0; 0x7F])[..2], [0x04, 0x7F]);
        assert_eq!(der(0x04, &[0; 0x80])[..3], [0x04, 0x81, 0x80]);
        assert_eq!(der(0x04, &[0; 0x1234])[..4], [0x04, 0x82, 0x12, 0x34]);
    }

    #[test]
    fn each_certificate_is_new_and_its_fingerprint_is_its_hash() {
        let (a, b) = (generate().unwrap(), generate().unwrap());
        assert_ne!(a.sha256, b.sha256);
        assert_eq!(a.sha256, fingerprint(a.cert.as_ref()));
        // rustls (webpki) parses it and accepts the key as its own.
        server_config(&a).unwrap();
    }
}
