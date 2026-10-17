use super::*;

fn pending(styles: &[&str]) -> FontDb {
    let mut db = FontDb::default();
    let entries = styles.iter().map(|s| Pending { path: format!("no-such-dir/{s}.ttf").into(), index: 0, style: (*s).into() }).collect();
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
        #[cfg(target_os = "linux")]
        {
            let dir = std::path::Path::new(&dir);
            index_configured_files(vec![dir.join("pipe.ttf"), dir.join("linked.otf"), dir.join("broken.ttf")], &mut pending);
            assert!(pending.is_empty(), "configured files must skip special targets too");
        }
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

#[cfg(target_os = "linux")]
struct FontDirectory(std::path::PathBuf);
#[cfg(target_os = "linux")]
impl FontDirectory {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let next = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("figcraft-configured-fonts-{}-{next}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
#[cfg(target_os = "linux")]
impl Drop for FontDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn configured_files_preserve_priority_aliases_and_the_existing_depth_policy() {
    let Some(font) = CRAFT_FONTS.iter().find(|font| !faces_in(font.bytes).is_empty()) else {
        eprintln!("skipping real-font indexing fixture: optional CRAFT_FONTS input is empty");
        return;
    };
    let directory = FontDirectory::new();
    let conventional = directory.0.join("a");
    std::fs::create_dir(&conventional).unwrap();
    let first = conventional.join("font.ttf");
    std::fs::write(&first, font.bytes).unwrap();
    let mut deep = directory.0.join("b");
    for _ in 0..9 {
        deep = deep.join("nested");
    }
    std::fs::create_dir_all(&deep).unwrap();
    let second = deep.join("font.ttf");
    std::fs::write(&second, font.bytes).unwrap();
    let broken = directory.0.join("broken.otf");
    std::os::unix::fs::symlink("missing.ttf", &broken).unwrap();
    let alias = directory.0.join("linked.otf");
    std::os::unix::fs::symlink(&second, &alias).unwrap();
    let mut found = BTreeMap::new();
    scan_dir(&conventional, 0, &mut found);
    let mut bounded = BTreeMap::new();
    scan_dir(&directory.0.join("b"), 0, &mut bounded);
    assert!(bounded.is_empty(), "conventional directory depth policy is unchanged");
    index_configured_files(vec![first.clone(), second.clone(), broken, alias.clone()], &mut found);
    let mut expected = BTreeMap::<String, usize>::new();
    for (_, family, _) in faces_in(font.bytes) {
        *expected.entry(family.to_lowercase()).or_default() += 1;
    }
    for (family, faces) in expected {
        let paths: Vec<_> = found[&family].1.iter().map(|face| face.path.clone()).collect();
        let expected: Vec<_> = [&first, &second, &alias].into_iter().flat_map(|path| std::iter::repeat_n(path.clone(), faces)).collect();
        assert_eq!(paths, expected, "existing fonts stay first; exact duplicates vanish but aliases remain");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn the_opt_out_prevents_both_system_scan_entry_points() {
    const VERIFIED: &str = "FIGCRAFT_OPTOUT_VERIFIED";
    if std::env::var_os("FIGCRAFT_TEST_OPTOUT_CHILD").is_some() {
        let mut local = FontDb::default();
        local.scan_system();
        assert!(!local.system_scanned && local.pending.is_empty());
        scan_system_fonts();
        assert_eq!(SCANS.load(Ordering::Relaxed), 0);
        assert!(!system_fonts_scanned());
        println!("\n{VERIFIED}");
        return;
    }
    let module = module_path!().split_once("::").unwrap().1;
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", &format!("{module}::the_opt_out_prevents_both_system_scan_entry_points"), "--nocapture"]);
    command.env("FIGCRAFT_TEST_OPTOUT_CHILD", "1").env("FIGCRAFT_NO_SYSTEM_FONTS", "1");
    // The production query supervisor also bounds and reaps this isolated test child.
    let output = fontconfig::test_command(command).unwrap();
    assert!(String::from_utf8(output).unwrap().lines().any(|line| line == VERIFIED), "the exact test must actually run");
}

#[cfg(target_os = "linux")]
#[test]
fn configured_files_keep_the_large_font_indexing_limit() {
    let Some(font) = CRAFT_FONTS.iter().find(|font| !faces_in(font.bytes).is_empty()) else {
        eprintln!("skipping real-font indexing fixture: optional CRAFT_FONTS input is empty");
        return;
    };
    let directory = FontDirectory::new();
    let path = directory.0.join("large.ttf");
    std::fs::write(&path, font.bytes).unwrap();
    // Pad an existing open font sparsely: this tests the indexing limit without needing a
    // proprietary colour-emoji fixture or reading a whole large font from disk.
    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_len(65 * 1024 * 1024).unwrap();
    let expected = faces_in(font.bytes);
    let mut found = BTreeMap::new();
    index_configured_files(vec![path.clone()], &mut found);
    for (index, family, style) in expected {
        let entries = &found.get(&family.to_lowercase()).unwrap().1;
        assert!(entries.iter().any(|entry| entry.path == path && entry.index == index && entry.style == style));
    }
    // The extraction must also retain the current upper bound, without reading this file.
    file.set_len(256 * 1024 * 1024 + 1).unwrap();
    let mut oversized = BTreeMap::new();
    index_configured_files(vec![path], &mut oversized);
    assert!(oversized.is_empty());
}
