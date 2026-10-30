use super::*;

fn pending(styles: &[&str]) -> FontDb {
    let mut db = FontDb::default();
    let entries = styles
        .iter()
        .map(|s| Pending { path: format!("no-such-dir/{s}.ttf").into(), index: 0, style: (*s).into(), postscript: None, local: None })
        .collect();
    db.pending.insert("cjk ui".into(), ("CJK UI".into(), entries));
    db
}

fn left(db: &FontDb) -> Vec<String> {
    db.pending.get("cjk ui").map(|(_, v)| v.iter().map(|e| e.style.clone()).collect()).unwrap_or_default()
}

#[test]
fn find_one_reads_only_the_nearest_style() {
    // The files don't exist: what matters is which entry is tried, and that the rest stay pending.
    let mut db = pending(&["Bold", "Light", "Regular", "Italic"]);
    assert!(db.find_one("CJK UI", "Regular").is_none());
    assert_eq!(left(&db), ["Bold", "Light", "Italic"]);
    assert!(db.has_family("cjk ui"), "the family is still offered");
    // No exact name: the nearest weight, upright before italic.
    let mut db = pending(&["W6", "Italic", "W3", "Bold"]);
    db.find_one("CJK UI", "Regular");
    assert_eq!(left(&db), ["W6", "Italic", "Bold"]);
    // `find` reads the whole family.
    db.find("CJK UI", "Regular");
    assert!(left(&db).is_empty() && !db.has_family("cjk ui"));
    assert_eq!(pending_weight("W3"), 300);
    assert_eq!(pending_weight("SemiBold"), 600);
    assert_eq!(pending_weight("W12"), 400);
}

#[test]
fn system_fonts_are_scanned_once() {
    // Racing callers (the desktop's background scan, on-demand lookups, tests) share one walk.
    let threads: Vec<_> = (0..4).map(|_| std::thread::spawn(scan_system_fonts)).collect();
    for t in threads {
        t.join().unwrap();
    }
    scan_system_fonts_on_demand();
    let _ = db();
    scan_system_fonts();
    let walked = !system_fonts_opted_out();
    assert_eq!(SCANS.load(Ordering::Relaxed), usize::from(walked));
    assert_eq!(system_fonts_scanned(), walked);
}

#[test]
fn the_scan_reads_names_like_a_whole_file_read() {
    // Every installed font file (none on CI): the partial read finds the same faces and names.
    let mut files = Vec::new();
    for dir in system_font_dirs() {
        let mut stack = vec![dir];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.is_file()
                    && p.extension().is_some_and(|x| ["ttf", "otf", "ttc", "otc"].contains(&x.to_ascii_lowercase().to_str().unwrap_or("")))
                {
                    files.push(p);
                }
            }
        }
    }
    for p in files {
        let Ok(whole) = std::fs::read(&p) else { continue };
        assert_eq!(faces_in(&index_bytes(&p, whole.len()).unwrap()), faces_in(&whole), "{}", p.display());
    }
}

fn indexed_test_font(bytes: &[u8], case: &str) -> Vec<u8> {
    struct TempFile(std::path::PathBuf);
    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let file = TempFile(std::env::temp_dir().join(format!("figcraft-font-index-{case}-{}.ttc", std::process::id())));
    std::fs::write(&file.0, bytes).unwrap();
    index_bytes(&file.0, bytes.len()).unwrap()
}

fn synthetic_font_collection(first: u32) -> Vec<u8> {
    let mut bytes = vec![0; 112];
    bytes[..4].copy_from_slice(b"ttcf");
    bytes[4..8].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    bytes[8..12].copy_from_slice(&2u32.to_be_bytes());
    bytes[12..16].copy_from_slice(&first.to_be_bytes());
    bytes[16..20].copy_from_slice(&32u32.to_be_bytes());
    // The second face has a name table and an outline table. Only names should be read.
    bytes[32..36].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    bytes[36..38].copy_from_slice(&2u16.to_be_bytes());
    bytes[44..48].copy_from_slice(b"name");
    bytes[52..56].copy_from_slice(&80u32.to_be_bytes());
    bytes[56..60].copy_from_slice(&8u32.to_be_bytes());
    bytes[60..64].copy_from_slice(b"glyf");
    bytes[68..72].copy_from_slice(&96u32.to_be_bytes());
    bytes[72..76].copy_from_slice(&4u32.to_be_bytes());
    bytes[80..88].copy_from_slice(b"fontname");
    bytes[96..100].copy_from_slice(b"skip");
    bytes
}

#[test]
fn index_bytes_skips_collection_offsets_near_u32_max() {
    for offset in [u32::MAX - 11, u32::MAX - 1, u32::MAX] {
        let bytes = synthetic_font_collection(offset);
        let indexed = indexed_test_font(&bytes, "offset");
        assert_eq!(&indexed[32..76], &bytes[32..76]);
        assert_eq!(&indexed[80..88], b"fontname");
        assert_eq!(&indexed[96..100], &[0; 4], "outline bytes stay unread");
    }
}

#[test]
fn index_bytes_skips_a_truncated_directory_and_reads_the_next_face() {
    let mut bytes = synthetic_font_collection(100);
    bytes[100..104].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    bytes[104..106].copy_from_slice(&2u16.to_be_bytes());
    let indexed = indexed_test_font(&bytes, "directory");
    assert_eq!(&indexed[100..112], &bytes[100..112], "the header fits but its directory does not");
    assert_eq!(&indexed[32..76], &bytes[32..76]);
    assert_eq!(&indexed[80..88], b"fontname");
    assert_eq!(&indexed[96..100], &[0; 4], "outline bytes stay unread");
}

#[test]
fn macos_font_assets_are_found_in_the_asset_store() {
    // A store laid out like macOS's: fonts downloaded on demand sit in `<id>.asset/AssetData`
    // folders under `com_apple_MobileAsset_Font*`; other assets are never walked. Untested on a Mac.
    struct TempDir(std::path::PathBuf);
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let tmp = TempDir(std::env::temp_dir().join(format!("figcraft-font-assets-{}", std::process::id())));
    let store = tmp.0.join("AssetsV2");
    let fonts = store.join("com_apple_MobileAsset_Font7");
    let data = |kind: &std::path::Path| kind.join("0f1e2d.asset").join("AssetData");
    for dir in [data(&fonts), data(&store.join("com_apple_MobileAsset_VoiceServices"))] {
        std::fs::create_dir_all(dir).unwrap();
    }
    // A file under a font-like name is listed but walks to nothing.
    std::fs::write(store.join("com_apple_MobileAsset_Font7.plist"), b"not a folder").unwrap();
    let found = font_asset_dirs(&store);
    assert!(found.contains(&fonts) && !found.iter().any(|d| d.ends_with("com_apple_MobileAsset_VoiceServices")), "{found:?}");
    assert!(font_asset_dirs(&tmp.0.join("missing")).is_empty());
    // With an installed font to copy in (none on CI), the scan finds it only in the font folder.
    let installed = system_font_dirs()
        .into_iter()
        .find_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x == "ttf")));
    let Some(font) = installed else { return };
    for kind in ["com_apple_MobileAsset_Font7", "com_apple_MobileAsset_VoiceServices"] {
        std::fs::copy(&font, data(&store.join(kind)).join("Font.ttf")).unwrap();
    }
    let mut pending = BTreeMap::new();
    for dir in found {
        scan_dir(&dir, 0, &mut pending);
    }
    let paths: Vec<_> = pending.values().flat_map(|(_, p)| p.iter().map(|p| p.path.clone())).collect();
    assert!(!paths.is_empty() && paths.iter().all(|p| p.starts_with(&fonts)), "{paths:?}");
}

#[cfg(unix)]
#[test]
fn the_scan_skips_named_pipes() {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    const INPUT: &str = "FIGCRAFT_TEST_PIPE_FONT_DIR";
    if let Some(dir) = std::env::var_os(INPUT) {
        let mut pending = BTreeMap::new();
        scan_dir(std::path::Path::new(&dir), 0, &mut pending);
        assert!(pending.is_empty());
        std::fs::write(std::path::Path::new(&dir).join("complete"), b"scanned").unwrap();
        return;
    }

    struct TempDir(std::path::PathBuf);
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let path = std::env::temp_dir().join(format!("figcraft-font-pipe-{}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let dir = TempDir(path);
    // POSIX mkfifo is only a test fixture helper, never a runtime font dependency.
    let status = match Command::new("mkfifo").arg(dir.0.join("pipe.ttf")).status() {
        Ok(status) => status,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("skipping named-pipe fixture: POSIX mkfifo is unavailable");
            return;
        }
        Err(error) => panic!("cannot create named-pipe fixture: {error}"),
    };
    assert!(status.success(), "mkfifo failed");
    std::os::unix::fs::symlink("pipe.ttf", dir.0.join("linked.otf")).unwrap();
    std::os::unix::fs::symlink("missing.ttf", dir.0.join("broken.ttf")).unwrap();

    // A regression must fail without leaving a blocked test thread behind. Run only this
    // test in a child, then kill and reap it if opening the pipe blocks the scanner.
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let module = module_path!().split_once("::").unwrap().1;
    let mut child = Child(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &format!("{module}::the_scan_skips_named_pipes"), "--nocapture"])
            .env(INPUT, &dir.0)
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success(), "font-scan child failed: {status}");
            break;
        }
        assert!(Instant::now() < deadline, "font scan blocked on a named pipe");
        // std::process has no portable timed wait; a bounded poll keeps the regression finite.
        std::thread::sleep(Duration::from_millis(10));
    }
    // An exact-filter typo otherwise makes libtest report success after running zero tests.
    assert_eq!(std::fs::read(dir.0.join("complete")).unwrap(), b"scanned");
}

#[test]
fn fallback_faces_follow_the_runs_style() {
    // A bold run's missing characters come from each fallback family's face nearest Bold, not its
    // Regular, so Hangul or kana in a bold Latin run come out bold too. Same families, same order.
    scan_system_fonts();
    let mut w = db().write().unwrap();
    let regular = w.fallbacks();
    let bold = w.fallbacks_for("Bold");
    let families = |v: &[Arc<Face>]| v.iter().map(|f| f.family.clone()).collect::<Vec<_>>();
    assert_eq!(families(&regular), families(&bold));
    for (r, b) in regular.iter().zip(&bold) {
        assert!(b.weight >= r.weight, "{}: {} is lighter than {}", b.family, b.style, r.style);
    }
}

#[test]
fn every_platform_has_a_symbol_font_in_the_fallbacks() {
    // Symbols such as ⌖ (U+2316) and ⤒ (U+2912) are in no text face (#1156): macOS has Apple
    // Symbols, Windows Segoe UI Symbol, and Linux the Noto symbol and math fonts (fontconfig names
    // "Noto Sans Symbols 2"; some packages name it "Noto Sans Symbols2").
    for family in ["Apple Symbols", "Segoe UI Symbol", "Noto Sans Symbols", "Noto Sans Symbols 2", "Noto Sans Symbols2", "Noto Sans Math"] {
        assert!(FALLBACK_FAMILIES.contains(&family), "{family} is a fallback");
    }
    // Text faces come first: a symbol font only draws what they lack.
    let at = |f: &str| FALLBACK_FAMILIES.iter().position(|x| *x == f).unwrap();
    assert!(at("Noto Sans") < at("Noto Sans Symbols") && at("DejaVu Sans") < at("Noto Sans Math"));
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn user_font_dirs_follow_xdg_data_home() {
    use std::ffi::OsString;
    use std::path::PathBuf;
    let dirs = |home: Option<&str>, data: Option<&str>| unix_user_font_dirs(home.map(OsString::from), data.map(OsString::from));
    let paths = |items: &[&str]| items.iter().map(PathBuf::from).collect::<Vec<_>>();
    assert_eq!(dirs(Some("/home/user"), Some("/custom user data")), paths(&["/home/user/.fonts", "/custom user data/fonts"]));
    for data in [None, Some(""), Some("relative/data")] {
        assert_eq!(dirs(Some("/home/user"), data), paths(&["/home/user/.fonts", "/home/user/.local/share/fonts"]));
    }
    assert_eq!(dirs(None, Some("/custom-data")), paths(&["/custom-data/fonts"]));
    assert_eq!(dirs(None, None), paths(&[]));
    assert_eq!(dirs(None, Some("relative")), paths(&[]));
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn user_font_dirs_preserve_non_utf8_paths() {
    use std::ffi::OsString;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let dirs = unix_user_font_dirs(Some(OsString::from_vec(b"/home/\xff".to_vec())), Some(OsString::from_vec(b"/data/\xfe".to_vec())));
    assert_eq!(dirs[0].as_os_str().as_bytes(), b"/home/\xff/.fonts");
    assert_eq!(dirs[1].as_os_str().as_bytes(), b"/data/\xfe/fonts");
}

#[test]
fn the_scan_reads_font_files_without_an_extension() {
    // #298: Adobe Fonts keeps the fonts it activates in hidden files named by number. Any installed
    // font stands in for one (none on CI); a file that isn't a font is left alone.
    let Some(font) = system_font_dirs().into_iter().find_map(|d| {
        std::fs::read_dir(d)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ttf") || x.eq_ignore_ascii_case("otf")))
    }) else {
        eprintln!("skipping: no installed font to stand in for an activated one");
        return;
    };
    let dir = std::env::temp_dir().join(format!("figcraft-font-bare-{}", std::process::id()));
    std::fs::create_dir_all(dir.join(".r")).unwrap();
    std::fs::copy(&font, dir.join(".r/.12345")).unwrap();
    std::fs::write(dir.join(".r/entitlements"), b"<xml/>").unwrap();
    let mut found = BTreeMap::new();
    scan_dir(&dir, 0, &mut found);
    let _ = std::fs::remove_dir_all(&dir);
    let families: Vec<&String> = found.values().map(|(f, _)| f).collect();
    assert_eq!(found.values().map(|(_, v)| v.len()).sum::<usize>(), faces_in(&std::fs::read(&font).unwrap()).len(), "{families:?}");
    assert!(!families.is_empty(), "{} is read as a font", font.display());
    assert!(found.values().flat_map(|(_, v)| v).all(|p| p.path.ends_with(".r/.12345")));
    #[cfg(target_os = "macos")]
    assert!(system_font_dirs().iter().any(|d| d.ends_with("Adobe/CoreSync/plugins/livetype")), "Adobe Fonts' folder is scanned");
}

/// Fonts downloaded into the suite's font store (by any Craft app) are scanned with the system's.
#[test]
fn the_suite_font_store_is_scanned() {
    let store = craft_font_catalog::font_store_dir(&craft_font_catalog::FontEnv::from_process());
    assert!(store.is_none_or(|dir| system_font_dirs().contains(&dir)));
}

/// A rescan offers new families and the styles installed since, and never a file twice.
#[test]
fn a_rescan_offers_fonts_installed_since() {
    let entry = |s: &str| Pending { path: format!("no-such-dir/{s}.ttf").into(), index: 0, style: s.into(), postscript: None, local: None };
    let mut db = pending(&["Regular"]);
    let found = |fams: &[(&str, &[&str])]| -> BTreeMap<String, (String, Vec<Pending>)> {
        fams.iter().map(|(f, styles)| (f.to_lowercase(), ((*f).to_string(), styles.iter().map(|s| entry(s)).collect()))).collect()
    };
    // A new family is added; a pending one takes the walk's files (a new style); both changed.
    let r = db.merge_rescan(found(&[("CJK UI", &["Regular", "Bold"]), ("Brand Sans", &["Regular"])]));
    assert_eq!(r.added, ["Brand Sans"]);
    assert_eq!(r.changed, ["Brand Sans", "CJK UI"]);
    assert!(db.has_family("brand sans"));
    assert_eq!(left(&db), ["Regular", "Bold"]);
    // A loaded family keeps its faces and gets only the files it hasn't read, once.
    db.families.insert("brand sans".into(), ("Brand Sans".into(), Vec::new()));
    db.pending.remove("brand sans");
    db.loaded_files.insert(("no-such-dir/Regular.ttf".into(), 0));
    let same = db.merge_rescan(found(&[("Brand Sans", &["Regular"])]));
    assert_eq!(same, Rescan::default());
    assert!(!db.pending.contains_key("brand sans"));
    let bold = db.merge_rescan(found(&[("Brand Sans", &["Regular", "Black"])]));
    assert_eq!((bold.added.len(), bold.changed), (0, vec!["Brand Sans".to_string()]));
    let waiting = |db: &FontDb| db.pending.get("brand sans").map(|(_, v)| v.iter().map(|e| e.style.clone()).collect::<Vec<_>>()).unwrap_or_default();
    assert_eq!(waiting(&db), ["Black"]);
    assert_eq!(db.merge_rescan(found(&[("Brand Sans", &["Regular", "Black"])])), Rescan::default());
    assert_eq!(waiting(&db), ["Black"]);
}

/// #1246: the first scan offers the installed styles an embedded family lacks (release builds embed
/// Poppins Regular, Italic and Bold; an installed SemiBold and Medium went unseen). Embedded styles win.
#[test]
fn the_scan_offers_installed_styles_an_embedded_family_lacks() {
    // Any embedded or installed font stands in for the embedded face (none without craft-fonts on CI).
    let stand_in = CRAFT_FONTS.first().map(|f| f.bytes.to_vec()).or_else(|| {
        let mut installed = BTreeMap::new();
        for dir in system_font_dirs() {
            scan_dir(&dir, 0, &mut installed);
        }
        installed.values().flat_map(|(_, v)| v).find_map(|p| std::fs::read(&p.path).ok())
    });
    let Some(mut face) = stand_in.and_then(|b| Face::from_bytes(FontBytes::Shared(Arc::new(b)), 0, Some("Brand Sans"), Some("Regular"))) else {
        eprintln!("skipping: no font to stand in for an embedded one");
        return;
    };
    // A variable stand-in's named instances aren't this family's styles.
    face.instances.clear();
    let mut db = FontDb::default();
    db.add(face);
    let entry = |s: &str| Pending { path: format!("no-such-dir/{s}.ttf").into(), index: 0, style: s.into(), postscript: None, local: None };
    let found =
        |family: &str, styles: &[&str]| BTreeMap::from([(family.to_lowercase(), (family.to_string(), styles.iter().map(|s| entry(s)).collect()))]);
    db.merge_scan(found("Brand Sans", &["Regular"]));
    assert!(!db.pending.contains_key("brand sans"), "an embedded style isn't read from disk again");
    db.merge_scan(found("Brand Sans", &["Regular", "SemiBold", "Medium"]));
    let waiting: Vec<String> = db.pending.get("brand sans").map(|(_, v)| v.iter().map(|e| e.style.clone()).collect()).unwrap_or_default();
    assert_eq!(waiting, ["SemiBold", "Medium"]);
    assert_eq!(db.families(), ["Brand Sans"]);
    db.merge_scan(found("CJK UI", &["Regular", "Bold"]));
    assert_eq!(left(&db), ["Regular", "Bold"]);
}

/// Font file bytes add their faces once; junk adds nothing.
#[test]
fn font_bytes_load_into_the_db() {
    assert!(load_font_bytes(b"not a font".to_vec()).is_empty());
    assert!(load_font_bytes(Vec::new()).is_empty());
    // With the embedded fonts (absent on CI builds without craft-fonts): a file's family is added,
    // and loading the same file again returns it without adding its faces twice.
    if let Some(f) = CRAFT_FONTS.first() {
        let families = load_font_bytes(f.bytes.to_vec());
        assert_eq!(families.len(), 1);
        assert!(families.iter().all(|f| has_family(f)));
        let faces = db().read().unwrap().face_count();
        assert_eq!(load_font_bytes(f.bytes.to_vec()), families);
        assert_eq!(db().read().unwrap().face_count(), faces);
    }
}

/// Apple's system font answers to the names designs and CSS use for it (SF Pro, SF UI Text,
/// system-ui, …), unless a font of that name is installed.
#[test]
fn apple_system_font_names_find_the_system_font() {
    let mut db = FontDb::default();
    db.pending.insert(
        "system font".into(),
        (
            "System Font".into(),
            vec![Pending { path: "no-such-dir/SFNS.ttf".into(), index: 0, style: "Regular".into(), postscript: None, local: None }],
        ),
    );
    for name in ["SF Pro", "SF Pro Text", "SF Pro Display", "SF UI Text", "SF UI Display", ".SF NS Text", "San Francisco"] {
        assert!(db.has_family(name), "{name}");
    }
    for name in ["system-ui", "-apple-system", "BlinkMacSystemFont", ".AppleSystemUIFont"] {
        assert_eq!(db.key(name), "system font", "{name}");
    }
    assert!(!db.has_family("SF Mono"));
    assert!(!db.has_family("ui-monospace"));
    db.pending.insert("sf pro".into(), ("SF Pro".into(), Vec::new()));
    assert_eq!(db.key("SF Pro"), "sf pro", "an installed SF Pro is itself");
    assert_eq!(db.key("SF UI Text"), "system font");
}

#[test]
fn hidden_apple_families_take_public_names() {
    assert_eq!(public_family(".SF Compact").as_deref(), Some("SF Compact"));
    assert_eq!(public_family(".SF Compact Rounded").as_deref(), Some("SF Compact Rounded"));
    assert_eq!(public_family(".SF NS Rounded").as_deref(), Some("SF Pro Rounded"));
    assert_eq!(public_family(".Keyboard"), None);
    assert_eq!(public_family("Inter").as_deref(), Some("Inter"));
}

/// A named style keeps its own optical size whatever the text size (existing documents render as
/// before); a custom `opsz` value (what a `.sketch` import sets) wins. Needs a variable font with an
/// optical size axis and named instances: macOS's or Windows' system font.
#[test]
fn named_styles_keep_their_optical_size() {
    let Some(bytes) = ["/System/Library/Fonts/SFNS.ttf", "C:/Windows/Fonts/SegUIVar.ttf"].iter().find_map(|p| std::fs::read(p).ok()) else {
        return;
    };
    let face = Face::from_bytes(FontBytes::Shared(std::sync::Arc::new(bytes)), 0, None, None).unwrap();
    let opsz = face.axes.iter().position(|a| a.tag == "opsz").unwrap();
    let inst = face.instances.first().unwrap();
    let mut style = figcraft_doc::TextStyle { font_style: inst.name.clone(), ..Default::default() };
    for size in [1.0, 12.0, 40.0, 200.0] {
        style.font_size = size;
        assert_eq!(face.axis_values(&style)[opsz], inst.coords[opsz], "{} at {size}", inst.name);
    }
    style.variations.insert("opsz".into(), 1e9);
    assert_eq!(face.axis_values(&style)[opsz], face.axes[opsz].max);
}

fn synthetic_trak() -> Vec<u8> {
    let mut bytes = vec![0; 46];
    bytes[..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    bytes[6..8].copy_from_slice(&12u16.to_be_bytes());
    bytes[12..14].copy_from_slice(&1u16.to_be_bytes());
    bytes[14..16].copy_from_slice(&3u16.to_be_bytes());
    bytes[16..20].copy_from_slice(&28u32.to_be_bytes());
    // The normal track (0.0), with three sizes and their signed tracking values.
    bytes[24..26].copy_from_slice(&256u16.to_be_bytes());
    bytes[26..28].copy_from_slice(&40u16.to_be_bytes());
    for (i, size) in [10i32, 20, 40].into_iter().enumerate() {
        bytes[28 + i * 4..32 + i * 4].copy_from_slice(&(size * 65536).to_be_bytes());
    }
    for (i, value) in [-8i16, 0, 16].into_iter().enumerate() {
        bytes[40 + i * 2..42 + i * 2].copy_from_slice(&value.to_be_bytes());
    }
    bytes
}

#[test]
fn normal_track_reads_a_well_formed_table() {
    assert_eq!(normal_track(&synthetic_trak()), [(10.0, -8.0), (20.0, 0.0), (40.0, 16.0)]);
}

#[test]
fn normal_track_ignores_size_table_offsets_near_u32_max() {
    let mut bytes = synthetic_trak();
    // On 32-bit targets even the first size's end can overflow. On 64-bit these are out of bounds.
    for offset in [u32::MAX - 7, u32::MAX - 3, u32::MAX - 1, u32::MAX] {
        bytes[16..20].copy_from_slice(&offset.to_be_bytes());
        assert!(normal_track(&bytes).is_empty(), "size table at {offset}");
    }
}

#[test]
fn normal_track_never_returns_a_partial_track_from_a_damaged_table() {
    let mut bytes = synthetic_trak();
    bytes.truncate(44); // Two values fit, but the third is missing.
    assert!(normal_track(&bytes).is_empty());
    let mut bytes = synthetic_trak();
    bytes[26..28].copy_from_slice(&20u16.to_be_bytes());
    bytes.truncate(36); // All values fit, but only two sizes do.
    assert!(normal_track(&bytes).is_empty());
}

/// The tracking a font's `trak` table gives at a size (Apple's fonts carry one; the platform text
/// engine applies it on its own), in pixels, interpolated between the sizes the table lists.
#[test]
fn trak_tracking_by_size() {
    let Ok(bytes) = std::fs::read("/System/Library/Fonts/SFNS.ttf") else { return };
    let face = Face::from_bytes(FontBytes::Shared(std::sync::Arc::new(bytes)), 0, None, None).unwrap();
    // Track 0 of SF: −12 units at 13 pt, 0 at 12 pt, −40 at 16 pt (2048 units per em).
    assert!((face.tracking(13.0) - (-12.0 * 13.0 / 2048.0)).abs() < 1e-9, "{}", face.tracking(13.0));
    assert!(face.tracking(12.0).abs() < 1e-9);
    assert!((face.tracking(12.5) - (-6.0 * 12.5 / 2048.0)).abs() < 1e-9);
    // Past the table's ends: its last values.
    assert!(face.tracking(500.0).is_finite());
    let plain = crate::fontdb::db().write().unwrap().default_face();
    if let Some(f) = plain.filter(|f| f.family != "System Font") {
        let _ = f.tracking(12.0);
    }
}

/// A face is found by its PostScript name (ignoring case), loaded or only scanned: the family and
/// style it is offered under.
#[test]
fn faces_by_postscript_name() {
    let mut db = FontDb::default();
    db.pending.insert(
        "brand sans".into(),
        (
            "Brand Sans".into(),
            vec![Pending {
                path: "no-such-dir/b.ttf".into(),
                index: 0,
                style: "Bold".into(),
                postscript: Some("BrandSans-Bold".into()),
                local: None,
            }],
        ),
    );
    assert_eq!(db.postscript_face("brandsans-bold"), Some(("Brand Sans".to_string(), "Bold".to_string())));
    assert_eq!(db.postscript_face("BrandSans-Light"), None);
    if let Some(f) = CRAFT_FONTS.first()
        && let Some(face) = Face::from_bytes(FontBytes::Static(f.bytes), 0, Some(f.family), Some(f.style))
        && let Some(ps) = face.postscript.clone()
    {
        db.add(face);
        assert_eq!(db.postscript_face(&ps), Some((f.family.to_string(), f.style.to_string())));
    }
    assert!(postscript_face("").is_none());
}

#[test]
fn every_platform_has_a_thai_font_in_the_fallbacks() {
    // Thai typed into a Latin layer (new text starts in Inter) is drawn by the first fallback that
    // has it: Noto Sans Thai (craft-fonts, Linux), Leelawadee UI or Tahoma (Windows), Thonburi
    // (macOS). They come after the Latin faces, which go on drawing Latin text.
    let at = |family: &str| FALLBACK_FAMILIES.iter().position(|f| *f == family);
    let latin = at("Segoe UI").unwrap();
    for thai in ["Noto Sans Thai", "Leelawadee UI", "Tahoma", "Thonburi"] {
        assert!(at(thai).is_some_and(|i| i > latin), "{thai}");
    }
}

// The paths are Unix absolute paths (font managers are a macOS thing): on Windows they aren't absolute.
#[cfg(unix)]
#[test]
fn font_manager_files_outside_the_font_folders_are_indexed_once() {
    let folders = [std::path::PathBuf::from("/Library/Fonts"), std::path::PathBuf::from("/Users/a/Library/Fonts")];
    let library = std::path::PathBuf::from("/Users/a/Fonts.rightfontlibrary/fonts/I/Inter/InterVariable.ttf");
    let files = vec![
        std::path::PathBuf::from("/Library/Fonts/Arial.ttf"),
        std::path::PathBuf::from("/Users/a/Library/Fonts/Sub/Brand.otf"),
        library.clone(),
        library.clone(),
        std::path::PathBuf::from("relative/Font.ttf"),
        std::path::PathBuf::from("/Library/FontsElsewhere/Mono.ttf"),
    ];
    let outside: Vec<_> = files_outside_dirs(files, &folders).into_iter().collect();
    assert_eq!(outside, [std::path::PathBuf::from("/Library/FontsElsewhere/Mono.ttf"), library]);
}

#[test]
fn a_font_file_outside_the_font_folders_is_indexed_from_its_path() {
    struct TempDir(std::path::PathBuf);
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let installed = system_font_dirs()
        .into_iter()
        .find_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x == "ttf")));
    let Some(font) = installed else { return };
    let tmp = TempDir(std::env::temp_dir().join(format!("figcraft-font-manager-{}", std::process::id())));
    std::fs::create_dir_all(&tmp.0).unwrap();
    let copy = tmp.0.join("Activated.ttf");
    std::fs::copy(&font, &copy).unwrap();
    let mut pending = BTreeMap::new();
    index_font_file(&copy, &std::fs::metadata(&copy).unwrap(), &mut pending);
    let paths: Vec<_> = pending.values().flat_map(|(_, p)| p.iter().map(|p| p.path.clone())).collect();
    assert!(!paths.is_empty() && paths.iter().all(|p| *p == copy), "{paths:?}");
    let mut none = BTreeMap::new();
    index_font_file(&tmp.0, &std::fs::metadata(&tmp.0).unwrap(), &mut none);
    assert!(none.is_empty());
}

#[cfg(target_os = "macos")]
#[test]
fn core_text_lists_the_installed_font_files() {
    let files = font_manager_files();
    assert!(files.iter().any(|f| f.starts_with("/System/Library/Fonts")), "{} files", files.len());
    assert!(files.iter().all(|f| f.is_absolute()));
}
