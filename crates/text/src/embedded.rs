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
    /// Describe an embedded stream without decoding or allocating until it is first used.
    pub(crate) const fn new(bytes: &'static [u8], unpacked_len: usize) -> Self {
        Self { bytes, unpacked_len, decoded: OnceLock::new() }
    }

    /// Restore static font bytes once, or report the unavailable face once and cache the failure.
    pub(crate) fn data(&'static self, family: &str, style: &str) -> Option<&'static [u8]> {
        self.decoded
            .get_or_init(|| {
                let bytes = unpack(self.bytes, self.unpacked_len);
                if bytes.is_none() {
                    log::warn!("embedded font {family} {style} could not be decoded");
                }
                bytes
            })
            .as_deref()
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

    /// Binary font data survives compression without losing any byte values.
    #[test]
    fn compressed_bytes_round_trip_exactly() {
        let original = b"synthetic font bytes\0\xff\x80\0synthetic font bytes";
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        encoder.write_all(original).unwrap();
        let packed = encoder.finish().unwrap();
        assert_eq!(unpack(&packed, original.len()).unwrap(), original);
    }

    /// Invalid checksums, unfinished streams and appended data never become usable font bytes.
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

    /// The declared size is both an exact output length and a bounded allocation request.
    #[test]
    fn declared_size_is_exact_and_bounded() {
        for len in [0, 3, 5, MAX_UNPACKED_FONT_BYTES + 1, usize::MAX] {
            assert!(unpack(PACKED, len).is_none(), "invalid unpacked size {len}");
        }
        assert_eq!(unpack(PACKED, 4).unwrap(), b"font");
    }

    /// Repeated requests reuse the same decoded allocation, including a cached decode failure.
    #[test]
    fn decoded_bytes_and_failures_are_cached() {
        static FONT: PackedFont = PackedFont::new(PACKED, 4);
        static BAD: PackedFont = PackedFont::new(PACKED, 3);
        let first = FONT.data("Synthetic", "Regular").unwrap();
        assert_eq!(first, b"font");
        assert!(std::ptr::eq(first, FONT.data("Synthetic", "Regular").unwrap()));
        assert!(BAD.data("Synthetic", "Invalid").is_none());
        assert!(BAD.decoded.get().is_some_and(Option::is_none));
    }

    /// An omitted face names its family and style in a single warning, even after repeated use.
    #[test]
    fn unavailable_font_is_reported_once() {
        struct Warnings(std::sync::Mutex<Vec<String>>);
        impl log::Log for Warnings {
            /// Capture only warnings emitted by the decoder.
            fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
                metadata.level() == log::Level::Warn
            }

            /// Retain messages so the test can count warnings for its own face.
            fn log(&self, record: &log::Record<'_>) {
                if self.enabled(record.metadata()) {
                    self.0.lock().unwrap().push(record.args().to_string());
                }
            }

            /// Messages are already stored synchronously in memory.
            fn flush(&self) {}
        }
        static WARNINGS: Warnings = Warnings(std::sync::Mutex::new(Vec::new()));
        static BAD: PackedFont = PackedFont::new(b"broken stream", 4);
        log::set_logger(&WARNINGS).unwrap();
        log::set_max_level(log::LevelFilter::Warn);
        assert!(BAD.data("Unavailable Test Face", "SemiBold").is_none());
        assert!(BAD.data("Unavailable Test Face", "SemiBold").is_none());
        let messages = WARNINGS.0.lock().unwrap();
        let expected = "embedded font Unavailable Test Face SemiBold could not be decoded";
        assert_eq!(messages.iter().filter(|message| message.as_str() == expected).count(), 1);
    }
}
