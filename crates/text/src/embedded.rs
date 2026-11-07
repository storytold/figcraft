//! Lossless decoding of web fonts, shared by the document renderer and UI through static bytes.

use std::sync::OnceLock;

/// Matches the font-file limit: corrupted metadata must not request an unbounded allocation.
const MAX_UNPACKED_FONT_BYTES: usize = 256 * 1024 * 1024;

/// One embedded zlib stream, decoded at most once; invalid streams stay unavailable.
pub(crate) struct PackedFont {
    bytes: &'static [u8],
    unpacked_len: usize,
    decoded: OnceLock<Option<Vec<u8>>>,
}

impl PackedFont {
    pub(crate) const fn new(bytes: &'static [u8], unpacked_len: usize) -> Self {
        Self { bytes, unpacked_len, decoded: OnceLock::new() }
    }

    pub(crate) fn data(&'static self) -> Option<&'static [u8]> {
        self.decoded.get_or_init(|| unpack(self.bytes, self.unpacked_len)).as_deref()
    }
}

/// Decode exactly the declared size, refusing malformed, incomplete or trailing data.
pub(crate) fn unpack(bytes: &[u8], unpacked_len: usize) -> Option<Vec<u8>> {
    if unpacked_len == 0 || unpacked_len > MAX_UNPACKED_FONT_BYTES {
        return None;
    }
    // A fixed output buffer prevents even a valid compressed stream expanding past its budget.
    let mut decoded = Vec::new();
    decoded.try_reserve_exact(unpacked_len).ok()?;
    decoded.resize(unpacked_len, 0);
    let mut decoder = flate2::Decompress::new(true);
    let status = decoder.decompress(bytes, &mut decoded, flate2::FlushDecompress::Finish).ok()?;
    (status == flate2::Status::StreamEnd && decoder.total_out() == unpacked_len as u64 && decoder.total_in() == bytes.len() as u64).then_some(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    // A zlib stream of the synthetic bytes "font"; no font asset is needed by these tests.
    const PACKED: &[u8] = &[120, 218, 75, 203, 207, 43, 1, 0, 4, 57, 1, 184];

    #[test]
    fn compressed_bytes_round_trip_exactly() {
        let original = b"synthetic font bytes\0\xff\x80\0synthetic font bytes";
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        encoder.write_all(original).unwrap();
        let packed = encoder.finish().unwrap();
        assert_eq!(unpack(&packed, original.len()).unwrap(), original);
    }

    #[test]
    fn corrupt_truncated_and_trailing_streams_are_refused() {
        assert!(unpack(b"not a zlib stream", 4).is_none());
        for end in 0..PACKED.len() {
            assert!(unpack(&PACKED[..end], 4).is_none(), "truncated at byte {end}");
        }
        let mut corrupt = PACKED.to_vec();
        corrupt[11] ^= 1;
        assert!(unpack(&corrupt, 4).is_none(), "checksum mismatch");
        let mut trailing = PACKED.to_vec();
        trailing.push(0);
        assert!(unpack(&trailing, 4).is_none(), "trailing bytes");
    }

    #[test]
    fn declared_size_is_exact_and_bounded() {
        for len in [0, 3, 5, MAX_UNPACKED_FONT_BYTES + 1, usize::MAX] {
            assert!(unpack(PACKED, len).is_none(), "invalid unpacked size {len}");
        }
        assert_eq!(unpack(PACKED, 4).unwrap(), b"font");
    }

    #[test]
    fn decoded_bytes_and_failures_are_cached() {
        static FONT: PackedFont = PackedFont::new(PACKED, 4);
        static BAD: PackedFont = PackedFont::new(PACKED, 3);
        let first = FONT.data().unwrap();
        assert_eq!(first, b"font");
        assert!(std::ptr::eq(first, FONT.data().unwrap()));
        assert!(BAD.data().is_none());
        assert!(BAD.decoded.get().is_some_and(Option::is_none));
    }
}
