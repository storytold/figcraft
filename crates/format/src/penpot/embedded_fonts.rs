//! Embedded font detection without decoding or retaining unsupported binaries.

use super::{BudgetReader, InflateBudget, MAX_ENTRIES, warn};
use std::io::{Cursor, Read};
use zip::ZipArchive;

/// Identify unsupported fonts from signatures without decoding their payloads.
pub(super) fn report(z: &mut ZipArchive<Cursor<&[u8]>>, warnings: &mut Vec<String>, budget: &mut InflateBudget) {
    let names: Vec<String> = z
        .file_names()
        .filter(|n| n.starts_with("objects/") || n.starts_with("fonts/") || n.starts_with("assets/fonts/"))
        .take(MAX_ENTRIES)
        .map(str::to_owned)
        .collect();
    let mut count = 0;
    for name in names {
        let Ok(member) = z.by_name(&name) else { continue };
        let mut header = [0; 4];
        if (BudgetReader { inner: member, budget }).read_exact(&mut header).is_ok()
            && (matches!(&header, b"OTTO" | b"true" | b"ttcf" | b"wOFF" | b"wOF2") || header == [0, 1, 0, 0])
        {
            count += 1;
        }
    }
    if count > 0 {
        warn(
            warnings,
            format!(
                "Penpot contains {count} embedded font file(s); the current model does not retain font binaries. Install the licensed fonts locally; original family references are preserved"
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::{ZipWriter, write::SimpleFileOptions};

    #[test]
    fn uuid_fonts_are_reported_without_decoding_their_payload() {
        let mut z = ZipWriter::new(Cursor::new(Vec::new()));
        z.start_file("objects/aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa", SimpleFileOptions::default()).unwrap();
        z.write_all(b"wOF2").unwrap();
        z.write_all(&vec![0; 100_000]).unwrap();
        let bytes = z.finish().unwrap().into_inner();
        let mut z = ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();
        let mut budget = InflateBudget::new();
        let before = budget.remaining;
        let mut warnings = Vec::new();
        report(&mut z, &mut warnings, &mut budget);
        assert_eq!(before - budget.remaining, 4);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("does not retain font binaries"));
    }
}
