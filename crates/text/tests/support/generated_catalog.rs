//! Compile and execute the generated web catalog independently of Cargo and optional font assets.

use super::{catalog_source, craft_fonts, pack_font, tests::Fixture};
use std::fmt::Write as _;

/// Generated Rust must preserve each face's inputs, skip rejected faces, and initialize only once.
#[test]
fn generated_web_catalog_compiles_and_initializes() {
    let fixture = Fixture::new();
    let mut manifest = String::new();
    let mut expected = String::from("static EXPECTED: &[(&str, &str, &[u8], usize, Option<&[u8]>)] = &[\n");
    for (index, (family, style, bytes, accepted)) in [
        ("Inter", "SemiBold", b"first synthetic font".as_slice(), true),
        ("Noto Sans Arabic", "Regular", b"second synthetic font with different bytes".as_slice(), true),
        ("Noto Sans Georgian", "Regular", b"rejected synthetic font".as_slice(), false),
    ]
    .into_iter()
    .enumerate()
    {
        let relative = format!("fonts/face-{index}.ttf");
        std::fs::write(fixture.0.join(&relative), bytes).unwrap();
        let source = fixture.0.join(&relative).canonicalize().unwrap();
        let (packed, _) = pack_font(&source, &fixture.0).unwrap();
        writeln!(manifest, "{family} | {style} | {relative} | Latn").unwrap();
        let decoded = if accepted { format!("Some(include_bytes!({:?}).as_slice())", source) } else { "None".into() };
        writeln!(expected, "({family:?}, {style:?}, include_bytes!({packed:?}), {}, {decoded}),", bytes.len()).unwrap();
    }
    expected.push_str("];\n");
    std::fs::write(fixture.0.join("fonts/manifest.txt"), manifest).unwrap();
    let entries = craft_fonts(&fixture.0, "fonts/manifest.txt", true, false, &[], &[], Some(&fixture.0)).unwrap().entries;
    for populated in [true, false] {
        let catalog = catalog_source(if populated { &entries } else { "" }, true);
        compile_and_run(&fixture.0, &format!("const POPULATED: bool = {populated};\n{expected}{catalog}{HARNESS}"));
    }
}

/// Compile a std-only contract harness, then execute its assertions without discovering rlibs.
fn compile_and_run(dir: &std::path::Path, source: &str) {
    let input = dir.join("generated_catalog.rs");
    let executable = dir.join(format!("generated_catalog{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&input, source).unwrap();
    let compiler = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let built = std::process::Command::new(compiler).arg("--edition=2024").arg(&input).arg("-o").arg(&executable).output().unwrap();
    assert!(built.status.success(), "generated catalog did not compile:\n{}", String::from_utf8_lossy(&built.stderr));
    let ran = std::process::Command::new(&executable).output().unwrap();
    assert!(ran.status.success(), "generated catalog failed:\n{}", String::from_utf8_lossy(&ran.stderr));
}

// This stub tests code-generation wiring; the real decoder has separate byte-for-byte tests.
// OnceLock deliberately preserves the real type's interior mutability, catching invalid static
// promotions (E0492) that a plain byte-slice stub would allow.
const HARNESS: &str = r#"
pub struct CraftFont { pub family: &'static str, pub style: &'static str, pub bytes: &'static [u8] }
static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
mod embedded {
    pub struct PackedFont {
        bytes: &'static [u8], len: usize,
        decoded: std::sync::OnceLock<Option<&'static [u8]>>,
    }
    impl PackedFont {
        /// Mirror the production constructor's const/static contract.
        pub const fn new(bytes: &'static [u8], len: usize) -> Self {
            Self { bytes, len, decoded: std::sync::OnceLock::new() }
        }
        /// Validate all generated inputs and return a distinct marker or a rejected face.
        pub fn data(&'static self, family: &str, style: &str) -> Option<&'static [u8]> {
            crate::CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            *self.decoded.get_or_init(|| {
                let expected = crate::EXPECTED.iter().find(|row| row.0 == family && row.1 == style).unwrap();
                assert_eq!(self.bytes, expected.2);
                assert_eq!(self.len, expected.3);
                expected.4
            })
        }
    }
}
/// Observe the generated public catalog twice; initialization must happen once, on first access.
fn main() {
    use std::sync::atomic::Ordering::SeqCst;
    assert_eq!(CALLS.load(SeqCst), 0);
    let expected: Vec<_> = EXPECTED.iter().filter(|row| POPULATED && row.4.is_some()).collect();
    for _ in 0..2 {
        assert_eq!(CRAFT_FONTS.len(), expected.len());
        for (font, row) in CRAFT_FONTS.iter().zip(&expected) {
            assert_eq!((font.family, font.style, font.bytes), (row.0, row.1, row.4.unwrap()));
        }
    }
    assert_eq!(CALLS.load(SeqCst), if POPULATED { EXPECTED.len() } else { 0 });
}
"#;
