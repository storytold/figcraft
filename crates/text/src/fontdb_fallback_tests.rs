//! The fallback fonts text draws missing characters with: each family reads the one face nearest
//! the run's style (Noto Sans CJK is seven files of 16–20 MB, read whole until now), families
//! naming the faces of one file share its bytes, and the faces are worked out once per style until
//! fonts change, not for every text shaped.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::{Face, FontBytes, FontDb, LocalFont, Pending};

/// A synthetic TrueType font: printable ASCII as outline-less glyphs 500 of 1000 units wide.
fn synthetic_font(family: &str) -> Vec<u8> {
    let be16 = |v: &mut Vec<u8>, x: u16| v.extend_from_slice(&x.to_be_bytes());
    let glyphs: u16 = 1 + (0x7E - 0x20 + 1);
    let mut head = Vec::new();
    for x in [0x0001_0000u32, 0x0001_0000, 0, 0x5F0F_3CF5] {
        head.extend_from_slice(&x.to_be_bytes());
    }
    be16(&mut head, 0); // flags
    be16(&mut head, 1000); // unitsPerEm
    head.extend_from_slice(&[0; 16]); // created, modified
    for x in [0u16, (-200i16) as u16, 500, 800, 0, 8, 2, 0, 0] {
        be16(&mut head, x); // bbox, macStyle, lowestRecPPEM, direction hint, short loca, glyph format
    }
    let mut hhea = 0x0001_0000u32.to_be_bytes().to_vec();
    for x in [800u16, (-200i16) as u16, 0, 500, 0, 0, 500, 1, 0, 0, 0, 0, 0, 0, 0, glyphs] {
        be16(&mut hhea, x);
    }
    let mut maxp = 0x0001_0000u32.to_be_bytes().to_vec();
    be16(&mut maxp, glyphs);
    for x in [0u16, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0] {
        be16(&mut maxp, x);
    }
    let mut hmtx = Vec::new();
    for _ in 0..glyphs {
        be16(&mut hmtx, 500);
        be16(&mut hmtx, 0);
    }
    let loca = vec![0; 2 * (usize::from(glyphs) + 1)];
    let glyf = vec![0; 4];
    // cmap: one Windows Unicode BMP subtable, format 4, 0x20..=0x7E → glyphs 1.. plus the end segment.
    let mut cmap = Vec::new();
    for x in [0u16, 1, 3, 1, 0, 12] {
        be16(&mut cmap, x);
    }
    for x in [4u16, 32, 0, 4, 4, 1, 0, 0x7E, 0xFFFF, 0, 0x20, 0xFFFF, (1i16 - 0x20) as u16, 1, 0, 0] {
        be16(&mut cmap, x);
    }
    let strings = [(1u16, family.to_string()), (2, "Regular".into()), (6, family.replace(' ', "") + "-Regular")];
    let mut name = Vec::new();
    be16(&mut name, 0);
    be16(&mut name, strings.len() as u16);
    be16(&mut name, 6 + 12 * strings.len() as u16);
    let mut storage = Vec::new();
    for (id, s) in &strings {
        let utf16: Vec<u8> = s.encode_utf16().flat_map(u16::to_be_bytes).collect();
        for x in [3u16, 1, 0x409, *id, utf16.len() as u16, storage.len() as u16] {
            be16(&mut name, x);
        }
        storage.extend(utf16);
    }
    name.extend(storage);
    let mut post = 0x0003_0000u32.to_be_bytes().to_vec();
    post.extend_from_slice(&[0; 28]);
    let tables: [(&[u8; 4], Vec<u8>); 9] = [
        (b"cmap", cmap),
        (b"glyf", glyf),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", loca),
        (b"maxp", maxp),
        (b"name", name),
        (b"post", post),
    ];
    let mut font = 0x0001_0000u32.to_be_bytes().to_vec();
    for x in [tables.len() as u16, 128, 3, 16] {
        be16(&mut font, x);
    }
    let mut offset = 12 + 16 * tables.len();
    for (tag, data) in &tables {
        font.extend_from_slice(*tag);
        font.extend_from_slice(&0u32.to_be_bytes());
        font.extend_from_slice(&(offset as u32).to_be_bytes());
        font.extend_from_slice(&(data.len() as u32).to_be_bytes());
        offset += data.len().next_multiple_of(4);
    }
    for (_, data) in &tables {
        font.extend_from_slice(data);
        font.resize(font.len().next_multiple_of(4), 0);
    }
    font
}

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A folder of synthetic font files, removed afterwards.
struct Fonts(PathBuf);

impl Fonts {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("fig-fallback-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    /// Write a font file named `file`.
    fn file(&self, file: &str) -> PathBuf {
        let path = self.0.join(file);
        std::fs::write(&path, synthetic_font("Synthetic")).unwrap();
        path
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Offer `family`'s faces as the font scan would: by style, from their files.
fn scanned(db: &mut FontDb, family: &str, faces: &[(&str, &Path)]) {
    let entries =
        faces.iter().map(|(style, path)| Pending { path: path.to_path_buf(), index: 0, style: (*style).into(), postscript: None, local: None });
    db.merge_scan(BTreeMap::from([(family.to_lowercase(), (family.to_string(), entries.collect()))]));
}

/// The styles of `family` not read yet.
fn waiting(db: &FontDb, family: &str) -> Vec<String> {
    db.pending.get(&family.to_lowercase()).map(|(_, v)| v.iter().map(|e| e.style.clone()).collect()).unwrap_or_default()
}

fn named(faces: &[Arc<Face>]) -> Vec<(String, String)> {
    faces.iter().map(|f| (f.family.clone(), f.style.clone())).collect()
}

const CJK: &str = "Noto Sans CJK SC";

#[test]
fn a_fallback_family_reads_only_the_face_nearest_each_style() {
    let fonts = Fonts::new();
    let styles = ["Thin", "Light", "DemiLight", "Regular", "Medium", "Bold", "Black"];
    let files: Vec<PathBuf> = styles.iter().map(|s| fonts.file(&format!("{s}.ttf"))).collect();
    let faces: Vec<(&str, &Path)> = styles.iter().copied().zip(files.iter().map(PathBuf::as_path)).collect();
    let mut db = FontDb::default();
    scanned(&mut db, CJK, &faces);
    let regular = db.fallbacks();
    assert_eq!(named(&regular), [(CJK.to_string(), "Regular".to_string())]);
    assert_eq!(waiting(&db, CJK), ["Thin", "Light", "DemiLight", "Medium", "Bold", "Black"], "one file read, not seven");
    // A bold run reads the bold face, once.
    let bold = db.fallbacks_for("Bold");
    assert_eq!(named(&bold), [(CJK.to_string(), "Bold".to_string())]);
    db.fallbacks_for("Bold");
    assert_eq!(waiting(&db, CJK), ["Thin", "Light", "DemiLight", "Medium", "Black"]);
    // A style no nearer than a face read already reads nothing: SemiBold is as far from Medium.
    db.fallbacks_for("SemiBold");
    assert_eq!(waiting(&db, CJK).len(), 5);
    // Used by name, the family reads the rest.
    assert!(db.find(CJK, "Light").is_some());
    assert!(waiting(&db, CJK).is_empty());
}

#[test]
fn fallback_faces_are_worked_out_once_until_fonts_change() {
    let fonts = Fonts::new();
    let regular = fonts.file("Regular.ttf");
    let mut db = FontDb::default();
    scanned(&mut db, CJK, &[("Regular", regular.as_path())]);
    let first = db.fallbacks();
    let generation = db.generation;
    // Every text shaped asks again: the same faces, with nothing looked up or read.
    for _ in 0..3 {
        let again = db.fallbacks();
        assert!(first.len() == again.len() && first.iter().zip(&again).all(|(a, b)| Arc::ptr_eq(a, b)));
    }
    assert_eq!(db.generation, generation, "nothing read or looked up again");
    // A fallback family installed since joins in its place.
    scanned(&mut db, "Noto Sans Arabic", &[("Regular", fonts.file("Arabic.ttf").as_path())]);
    let families: Vec<String> = db.fallbacks().iter().map(|f| f.family.clone()).collect();
    assert_eq!(families, [CJK, "Noto Sans Arabic"]);
}

#[test]
fn families_naming_the_faces_of_one_file_share_its_bytes() {
    // Noto Sans CJK SC and KR are faces of the same collections.
    let fonts = Fonts::new();
    let file = fonts.file("NotoSansCJK-Regular.ttc");
    let mut db = FontDb::default();
    scanned(&mut db, CJK, &[("Regular", file.as_path())]);
    scanned(&mut db, "Noto Sans CJK KR", &[("Regular", file.as_path())]);
    let faces = db.fallbacks();
    let shared = |a: &Arc<Face>, b: &Arc<Face>| matches!((&a.bytes, &b.bytes), (FontBytes::Shared(a), FontBytes::Shared(b)) if Arc::ptr_eq(a, b));
    let [sc, kr] = faces.as_slice() else { panic!("{:?}", named(&faces)) };
    assert_eq!((sc.family.as_str(), kr.family.as_str()), (CJK, "Noto Sans CJK KR"));
    assert!(shared(sc, kr), "the file is read once");
    // A file that changed length since is read again.
    let mut grown = std::fs::read(&file).unwrap();
    grown.extend_from_slice(&[0; 4]);
    std::fs::write(&file, grown).unwrap();
    scanned(&mut db, "Noto Sans KR", &[("Regular", file.as_path())]);
    let faces = db.fallbacks();
    let kr2 = faces.iter().find(|f| f.family == "Noto Sans KR").unwrap();
    assert!(!shared(sc, kr2));
}

#[test]
fn a_face_that_cant_be_read_gives_way_to_the_next_nearest() {
    let fonts = Fonts::new();
    let missing = fonts.0.join("Regular.ttf");
    let medium = fonts.file("Medium.ttf");
    let mut db = FontDb::default();
    let bold = fonts.file("Bold.ttf");
    scanned(&mut db, "DejaVu Sans", &[("Regular", missing.as_path()), ("Bold", bold.as_path()), ("Medium", medium.as_path())]);
    assert_eq!(named(&db.fallbacks()), [("DejaVu Sans".to_string(), "Medium".to_string())]);
    assert_eq!(waiting(&db, "DejaVu Sans"), ["Bold"]);
}

#[test]
fn a_run_reads_the_face_that_slants_as_it_does() {
    // DejaVu Sans names its upright face Book and its slanted ones Oblique: a regular run reads
    // Book, not Oblique, and a bold italic one Bold Oblique, not Bold.
    let fonts = Fonts::new();
    let styles = ["Bold", "Bold Oblique", "ExtraLight", "Oblique", "Book"];
    let files: Vec<PathBuf> = styles.iter().map(|s| fonts.file(&format!("DejaVuSans-{s}.ttf"))).collect();
    let faces: Vec<(&str, &Path)> = styles.iter().copied().zip(files.iter().map(PathBuf::as_path)).collect();
    let mut db = FontDb::default();
    scanned(&mut db, "DejaVu Sans", &faces);
    assert_eq!(named(&db.fallbacks()), [("DejaVu Sans".to_string(), "Book".to_string())]);
    db.fallbacks_for("Bold Italic");
    assert_eq!(waiting(&db, "DejaVu Sans"), ["Bold", "ExtraLight", "Oblique"]);
}

#[test]
fn a_browser_fallback_font_is_asked_for_once_per_style() {
    let local =
        |id: usize, style: &str| LocalFont { id, family: "Noto Sans Thai".into(), style: style.into(), postscript: format!("NotoSansThai-{style}") };
    let mut db = FontDb::default();
    db.add_local(vec![local(1, "Bold"), local(2, "Regular")]);
    // Nothing draws until the bytes come; the regular face is asked for, not the bold one.
    assert!(db.fallbacks().is_empty());
    assert_eq!(std::mem::take(&mut db.wanted), [2]);
    let generation = db.generation;
    db.fallbacks();
    db.fallbacks();
    assert!(db.wanted.is_empty(), "asked for once");
    assert_eq!(db.generation, generation);
    // A bold run asks for the bold face.
    db.fallbacks_for("Bold");
    assert_eq!(std::mem::take(&mut db.wanted), [1]);
    assert_eq!(db.load_local(2, synthetic_font("Noto Sans Thai")).as_deref(), Some("Noto Sans Thai"));
    assert_eq!(named(&db.fallbacks()), [("Noto Sans Thai".to_string(), "Regular".to_string())]);
    assert!(db.wanted.is_empty());
}
