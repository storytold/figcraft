//! Negotiated, bounded pieces of one collaboration message. Document operations still become
//! visible atomically, after the entire message has arrived and passed its SHA-256 check.

use sha2::{Digest, Sha256};

pub(crate) const PROTOCOL: &str = "figcraft.chunks.v1";
pub(crate) const CHUNK: usize = 1 << 20;
pub(crate) const HEADER: usize = 49;
const VERSION: u8 = 2;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Error {
    TooLarge,
    Invalid(String),
}

impl From<&str> for Error {
    fn from(reason: &str) -> Self {
        Self::Invalid(reason.to_string())
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge => f.write_str("the collaboration message exceeds the receiving limit"),
            Self::Invalid(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for Error {}

/// Legacy peers keep wire version 1. Only a negotiated connection sends version 2 pieces.
pub(crate) fn packets(bytes: &[u8], chunked: bool) -> Packets<'_> {
    let chunked = chunked && bytes.len() > CHUNK;
    let hash = if chunked { Sha256::digest(bytes).into() } else { [0; 32] };
    Packets { bytes, offset: 0, chunked, hash }
}

pub(crate) struct Packets<'a> {
    bytes: &'a [u8],
    offset: usize,
    chunked: bool,
    hash: [u8; 32],
}

impl Iterator for Packets<'_> {
    type Item = Vec<u8>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset >= self.bytes.len() {
            return None;
        }
        if !self.chunked {
            self.offset = self.bytes.len();
            return Some(self.bytes.to_vec());
        }
        let end = self.offset.saturating_add(CHUNK).min(self.bytes.len());
        let mut out = Vec::with_capacity(HEADER + end - self.offset);
        out.push(VERSION);
        out.extend_from_slice(&(self.bytes.len() as u64).to_le_bytes());
        out.extend_from_slice(&(self.offset as u64).to_le_bytes());
        out.extend_from_slice(&self.hash);
        out.extend_from_slice(self.bytes.get(self.offset..end)?);
        self.offset = end;
        Some(out)
    }
}

#[derive(Default)]
pub(crate) struct Receiver {
    bytes: Vec<u8>,
    total: Option<usize>,
    hash: [u8; 32],
    digest: Sha256,
}

impl Receiver {
    /// No allocation follows an untrusted size declaration. Storage grows only with bytes
    /// actually received, inside `max`; partial messages belong to this connection only.
    pub(crate) fn push(&mut self, bytes: &[u8], negotiated: bool, max: usize) -> Result<Option<Vec<u8>>, Error> {
        if bytes.first() != Some(&VERSION) {
            if self.total.is_some() {
                return Err("a collaboration transfer was interrupted by another message".into());
            }
            if bytes.len() > max {
                return Err(Error::TooLarge);
            }
            return Ok(Some(bytes.to_vec()));
        }
        if !negotiated {
            return Err("chunked collaboration was not negotiated".into());
        }
        if bytes.len() <= HEADER || bytes.len() > HEADER + CHUNK {
            return Err("invalid collaboration chunk length".into());
        }
        let number = |range: std::ops::Range<usize>| -> Result<u64, Error> {
            let array: [u8; 8] = bytes.get(range).and_then(|v| v.try_into().ok()).ok_or("invalid chunk header")?;
            Ok(u64::from_le_bytes(array))
        };
        let total = usize::try_from(number(1..9)?).map_err(|_| Error::TooLarge)?;
        if total > max {
            return Err(Error::TooLarge);
        }
        let offset = usize::try_from(number(9..17)?).map_err(|_| Error::from("chunk offset does not fit this platform"))?;
        let hash: [u8; 32] = bytes.get(17..HEADER).and_then(|v| v.try_into().ok()).ok_or("invalid chunk checksum")?;
        let payload = bytes.get(HEADER..).ok_or("invalid chunk payload")?;
        if total == 0 || offset != self.bytes.len() || payload.len() > total.saturating_sub(offset) {
            return Err("collaboration chunk is out of order".into());
        }
        if self.total.is_some_and(|n| n != total) || (self.total.is_some() && self.hash != hash) {
            return Err("collaboration transfer metadata changed".into());
        }
        self.total = Some(total);
        self.hash = hash;
        if self.bytes.capacity() - self.bytes.len() < payload.len() {
            // Retain geometric growth without requesting capacity beyond the declared total.
            let grow = self.bytes.len().max(payload.len()).min(total - self.bytes.len());
            self.bytes.try_reserve_exact(grow).map_err(|_| "not enough memory for the collaboration transfer")?;
        }
        self.bytes.extend_from_slice(payload);
        self.digest.update(payload);
        if self.bytes.len() != total {
            return Ok(None);
        }
        let digest: [u8; 32] = std::mem::take(&mut self.digest).finalize().into();
        if digest != self.hash {
            return Err("collaboration transfer checksum mismatch".into());
        }
        self.total = None;
        Ok(Some(std::mem::take(&mut self.bytes)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reassembly_growth_stays_within_the_total_and_reuses_spare_capacity() {
        let source = vec![37; CHUNK * 8 + 1];
        let mut rx = Receiver::default();
        let mut allocations = 0;
        for packet in packets(&source, true) {
            let (len, capacity) = (rx.bytes.len(), rx.bytes.capacity());
            let payload_len = packet.len() - HEADER;
            let complete = rx.push(&packet, true, source.len()).unwrap();
            let current_capacity = complete.as_ref().map_or(rx.bytes.capacity(), Vec::capacity);
            assert!(current_capacity <= source.len());
            if capacity - len >= payload_len {
                assert_eq!(current_capacity, capacity, "reuse space already allocated");
            }
            allocations += usize::from(current_capacity != capacity);
            if let Some(bytes) = complete {
                assert_eq!(bytes, source);
            }
        }
        assert!(allocations <= 5, "bounded geometric growth, not a reallocation per chunk");
    }

    #[test]
    fn large_message_is_atomic_and_every_piece_is_bounded() {
        let source = vec![37; CHUNK * 2 + 31];
        let chunks: Vec<_> = packets(&source, true).collect();
        assert_eq!(chunks.len(), 3);
        let mut rx = Receiver::default();
        for (i, p) in chunks.iter().enumerate() {
            assert!(p.len() <= CHUNK + HEADER);
            let received = rx.push(p, true, source.len()).unwrap();
            if i == chunks.len() - 1 {
                assert_eq!(received, Some(source.clone()));
            } else {
                assert_eq!(received, None, "no partial document can reach the engine");
            }
        }
        assert!(rx.bytes.is_empty());
    }

    #[test]
    fn legacy_peer_and_small_messages_keep_the_original_wire_format() {
        for (bytes, enabled) in [(vec![1, 2, 3], true), (vec![1; CHUNK + 1], false)] {
            let p: Vec<_> = packets(&bytes, enabled).collect();
            assert_eq!(p, vec![bytes.clone()]);
            assert_eq!(Receiver::default().push(&p[0], false, bytes.len()).unwrap(), Some(bytes));
        }
    }

    #[test]
    fn receive_budget_errors_are_distinct_from_invalid_frames() {
        let source = vec![1; CHUNK + 1];
        let p: Vec<_> = packets(&source, true).collect();
        let mut rx = Receiver::default();
        assert_eq!(rx.push(&p[0], true, CHUNK), Err(Error::TooLarge));
        assert!(rx.bytes.is_empty());
        assert_eq!(Receiver::default().push(&source, false, CHUNK), Err(Error::TooLarge));
        assert!(matches!(Receiver::default().push(&p[1], true, source.len()), Err(Error::Invalid(_))));
        let mut zero = p[0].clone();
        zero[1..9].copy_from_slice(&0u64.to_le_bytes());
        assert!(matches!(Receiver::default().push(&zero, true, source.len()), Err(Error::Invalid(_))));
        let mut short = p[0].clone();
        short[1..9].copy_from_slice(&1u64.to_le_bytes());
        assert!(matches!(Receiver::default().push(&short, true, source.len()), Err(Error::Invalid(_))));
        let mut huge = p[0].clone();
        huge[1..9].copy_from_slice(&u64::MAX.to_le_bytes());
        assert_eq!(Receiver::default().push(&huge, true, CHUNK), Err(Error::TooLarge));
    }

    #[test]
    fn hostile_headers_are_refused_before_allocating() {
        let source = vec![4; CHUNK + 1];
        let p: Vec<_> = packets(&source, true).collect();
        let mut rx = Receiver::default();
        assert!(rx.push(&p[0], true, CHUNK).is_err());
        assert!(rx.bytes.is_empty());
        assert!(Receiver::default().push(&p[0], false, source.len()).is_err());
        assert!(Receiver::default().push(&p[1], true, source.len()).is_err());
    }

    #[test]
    fn missing_duplicate_corrupt_and_interleaved_pieces_do_not_commit() {
        let source = vec![4; CHUNK + 1];
        let p: Vec<_> = packets(&source, true).collect();
        let mut rx = Receiver::default();
        assert!(rx.push(&p[0], true, source.len()).unwrap().is_none());
        assert!(rx.push(&p[0], true, source.len()).is_err());
        let mut rx = Receiver::default();
        rx.push(&p[0], true, source.len()).unwrap();
        assert!(rx.push(&[1, 4], true, source.len()).is_err());
        let mut rx = Receiver::default();
        rx.push(&p[0], true, source.len()).unwrap();
        let mut corrupted = p[1].clone();
        *corrupted.last_mut().unwrap() ^= 1;
        assert!(rx.push(&corrupted, true, source.len()).is_err());
    }

    #[test]
    fn a_new_connection_does_not_inherit_an_incomplete_transfer() {
        let source = vec![4; CHUNK + 1];
        let p: Vec<_> = packets(&source, true).collect();
        let mut rx = Receiver::default();
        rx.push(&p[0], true, source.len()).unwrap();
        drop(rx);
        let mut rx = Receiver::default();
        assert!(rx.push(&p[1], true, source.len()).is_err());
        assert!(rx.push(&p[0], true, source.len()).unwrap().is_none());
        assert_eq!(rx.push(&p[1], true, source.len()).unwrap(), Some(source));
    }
}
