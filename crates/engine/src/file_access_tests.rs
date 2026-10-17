//! Automation roots: how paths are resolved and compared, and that commands run confined can't
//! reach past them.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;

use super::{Access, Roots, allow, confined, current};
use crate::Session;

/// A fresh folder for one test: `<tmp>/figcraft-roots-<name>-<pid>`, holding `root/`, `root2/`
/// (a sibling whose name starts like the root's) and `outside/`.
struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str) -> Self {
        let d = std::env::temp_dir().join(format!("figcraft-roots-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for sub in ["root/sub", "root2", "outside"] {
            std::fs::create_dir_all(d.join(sub)).unwrap();
        }
        std::fs::write(d.join("root/in.txt"), b"in").unwrap();
        std::fs::write(d.join("outside/secret.txt"), b"secret").unwrap();
        Self(d)
    }

    fn p(&self, rel: &str) -> String {
        self.0.join(rel).to_string_lossy().into_owned()
    }

    /// The root as both read and write root.
    fn rw(&self) -> Roots {
        Roots::new(&[], &[self.p("root")]).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(unix)]
fn symlink_dir(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn symlink_dir(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_dir(target, link).is_ok()
}

#[cfg(unix)]
fn symlink_file(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn symlink_file(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_file(target, link).is_ok()
}

#[test]
fn inside_is_allowed_and_resolved_outside_is_refused() {
    let f = Fixture::new("inside");
    let r = f.rw();
    let got = r.check(Path::new(&f.p("root/in.txt")), Access::Read).unwrap();
    assert_eq!(got, std::fs::canonicalize(f.p("root/in.txt")).unwrap(), "the I/O uses the resolved path");
    assert!(r.check(Path::new(&f.p("root/new.png")), Access::Write).is_ok());
    let e = r.check(Path::new(&f.p("outside/secret.txt")), Access::Read).unwrap_err();
    assert!(e.starts_with("automation path rejected:") && e.contains("outside the folders that may be read"), "{e}");
    assert!(r.check(Path::new(&f.p("outside/new.png")), Access::Write).is_err());
    // The root itself is inside.
    assert!(r.check(Path::new(&f.p("root")), Access::Read).is_ok());
}

#[test]
fn a_sibling_whose_name_starts_like_the_root_is_outside() {
    let f = Fixture::new("prefix");
    let r = f.rw();
    for p in ["root2/x.png", "root2", "root.png"] {
        assert!(r.check(Path::new(&f.p(p)), Access::Write).is_err(), "{p}");
        assert!(r.check(Path::new(&f.p(p)), Access::Read).is_err(), "{p}");
    }
}

#[test]
fn traversal_is_resolved_before_comparing() {
    let f = Fixture::new("dots");
    let r = f.rw();
    assert!(r.check(Path::new(&f.p("root/sub/../../outside/secret.txt")), Access::Read).is_err());
    assert!(r.check(Path::new(&f.p("root/sub/../../outside/new.txt")), Access::Write).is_err());
    assert!(r.check(Path::new(&f.p("root/sub/../in.txt")), Access::Read).is_ok());
    assert!(r.check(Path::new(&f.p("root/./sub/../new.txt")), Access::Write).is_ok());
    // `..` below a folder that doesn't exist can't be resolved (on Windows it is taken lexically,
    // as the system does, and stays inside).
    let deep = f.p("root/missing/../../outside/x.txt");
    assert!(r.check(Path::new(&deep), Access::Write).is_err(), "{deep}");
}

#[test]
fn files_that_dont_exist_yet_resolve_through_their_nearest_folder() {
    let f = Fixture::new("missing");
    let r = f.rw();
    let got = r.check(Path::new(&f.p("root/a/b/c.png")), Access::Write).unwrap();
    assert_eq!(got, std::fs::canonicalize(f.p("root")).unwrap().join("a").join("b").join("c.png"));
    assert!(r.check(Path::new(&f.p("outside/a/b/c.png")), Access::Write).is_err());
}

#[test]
fn relative_paths_are_resolved_against_the_working_directory() {
    // `cargo test` runs in the crate's folder, which holds Cargo.toml.
    let cwd = std::env::current_dir().unwrap().to_string_lossy().into_owned();
    let here = Roots::new(std::slice::from_ref(&cwd), &[]).unwrap();
    assert_eq!(here.check(Path::new("Cargo.toml"), Access::Read).unwrap(), std::fs::canonicalize("Cargo.toml").unwrap());
    assert!(here.check(Path::new("src/../Cargo.toml"), Access::Read).is_ok());
    let f = Fixture::new("relative");
    assert!(f.rw().check(Path::new("Cargo.toml"), Access::Read).is_err());
    // Relative roots too.
    assert_eq!(Roots::new(&[".".into()], &[]).unwrap(), here);
}

#[test]
fn read_and_write_authority_are_separate() {
    let f = Fixture::new("modes");
    let read_only = Roots::new(&[f.p("root")], &[]).unwrap();
    assert!(read_only.check(Path::new(&f.p("root/in.txt")), Access::Read).is_ok());
    let e = read_only.check(Path::new(&f.p("root/out.png")), Access::Write).unwrap_err();
    assert!(e.contains("no folder may be written"), "{e}");
    // A write root may be read too; a read root elsewhere doesn't make it writable.
    let both = Roots::new(&[f.p("outside")], &[f.p("root")]).unwrap();
    assert!(both.check(Path::new(&f.p("root/in.txt")), Access::Read).is_ok());
    assert!(both.check(Path::new(&f.p("outside/secret.txt")), Access::Read).is_ok());
    assert!(both.check(Path::new(&f.p("outside/new.txt")), Access::Write).is_err());
    // Several roots of each kind.
    let many = Roots::new(&[f.p("root"), f.p("root2")], &[f.p("root2"), f.p("outside")]).unwrap();
    assert!(many.check(Path::new(&f.p("outside/new.txt")), Access::Write).is_ok());
    assert!(many.check(Path::new(&f.p("root/new.txt")), Access::Write).is_err());
}

#[test]
fn roots_must_be_existing_folders() {
    let f = Fixture::new("bad-roots");
    assert!(Roots::new(&[f.p("nope")], &[]).unwrap_err().contains("--automation-read-root"));
    assert!(Roots::new(&[], &[f.p("root/in.txt")]).unwrap_err().contains("not a folder"));
    assert!(Roots::new(&[], &[String::new()]).is_err());
    assert!(f.rw().check(Path::new(""), Access::Read).is_err());
}

#[test]
fn links_that_lead_out_of_a_root_are_outside() {
    let f = Fixture::new("links");
    let r = f.rw();
    if !symlink_dir(&f.0.join("outside"), &f.0.join("root/escape")) || !symlink_file(&f.0.join("outside/secret.txt"), &f.0.join("root/secret.txt")) {
        eprintln!("skipped: this system doesn't let tests make symbolic links");
        return;
    }
    assert!(r.check(Path::new(&f.p("root/escape/secret.txt")), Access::Read).is_err());
    assert!(r.check(Path::new(&f.p("root/escape/new.txt")), Access::Write).is_err());
    assert!(r.check(Path::new(&f.p("root/secret.txt")), Access::Read).is_err());
    assert!(r.check(Path::new(&f.p("root/secret.txt")), Access::Write).is_err());
    // A link that leads nowhere: writing through it would create its target.
    assert!(symlink_file(&f.0.join("outside/not-yet.txt"), &f.0.join("root/dangling.txt")));
    let e = r.check(Path::new(&f.p("root/dangling.txt")), Access::Write).unwrap_err();
    assert!(e.contains("leads nowhere"), "{e}");
    // A link inside the root to the root is fine.
    assert!(symlink_dir(&f.0.join("root/sub"), &f.0.join("root/alias")));
    assert!(r.check(Path::new(&f.p("root/alias/x.png")), Access::Write).is_ok());
}

#[cfg(windows)]
#[test]
fn windows_case_devices_and_streams() {
    let f = Fixture::new("windows");
    let r = f.rw();
    // Case is taken from the disk: another spelling of the root is the root.
    let upper = f.0.join("ROOT").join("IN.TXT");
    assert!(r.check(&upper, Access::Read).is_ok());
    for name in ["CON", "nul.txt", "Com1.png", "LPT9", "a.txt:stream", "conout$"] {
        assert!(r.check(&f.0.join("root").join(name), Access::Write).is_err(), "{name}");
    }
    assert!(r.check(&f.0.join("root").join("console.png"), Access::Write).is_ok());
    // The `\\?\` form of a path inside is inside; of a path outside, outside.
    let verbatim = std::fs::canonicalize(f.p("root")).unwrap().join("v.png");
    assert!(verbatim.to_string_lossy().starts_with(r"\\?\"));
    assert!(r.check(&verbatim, Access::Write).is_ok());
    assert!(r.check(&std::fs::canonicalize(f.p("outside")).unwrap().join("v.png"), Access::Write).is_err());
    // Messages don't show the prefix.
    let e = r.check(Path::new(&f.p("outside/x.png")), Access::Write).unwrap_err();
    assert!(!e.contains(r"\\?\"), "{e}");
}

#[test]
fn a_scope_confines_the_thread_and_ends_with_it() {
    let f = Fixture::new("scope");
    let roots = Arc::new(f.rw());
    assert!(current().is_none());
    confined(Some(&roots), || {
        assert!(allow(f.p("outside/secret.txt"), Access::Read).is_err());
        assert!(super::read(f.p("outside/secret.txt")).is_err());
        assert_eq!(super::read(f.p("root/in.txt")).unwrap(), b"in");
        // Not confined: the app's own files.
        assert_eq!(super::app_owned(|| super::read(f.p("outside/secret.txt")).unwrap()), b"secret");
        assert!(current().is_some(), "back in the scope after app_owned");
        // Another thread isn't confined by this one's scope.
        let p = f.p("outside/secret.txt");
        assert!(std::thread::spawn(move || current().is_none() && allow(p, Access::Read).is_ok()).join().unwrap());
    });
    assert!(current().is_none());
    assert!(allow(f.p("outside/secret.txt"), Access::Read).is_ok());
    // A panic inside the scope still ends it.
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| confined(Some(&roots), || panic!("boom"))));
    assert!(r.is_err());
    assert!(current().is_none());
    // `None` changes nothing.
    assert!(confined(None, || current().is_none()));
}

#[test]
fn commands_run_confined_read_and_write_only_inside() {
    let f = Fixture::new("commands");
    std::fs::create_dir_all(f.0.join("outside/readable")).unwrap();
    let roots = Arc::new(Roots::new(&[f.p("outside/readable")], &[f.p("root")]).unwrap());
    // Unconfined, the outside is writable: the bait files a confined command must not reach.
    let bait = f.p("outside/bait.figcraft");
    let mut s = Session::new();
    s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 20, "height": 10})).unwrap();
    s.execute("file.saveAs", &json!({"path": bait})).unwrap();
    std::fs::copy(&bait, f.0.join("outside/readable/ok.figcraft")).unwrap();
    let mut s = Session::new();
    s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 20, "height": 10})).unwrap();
    s.prefs.recent_files = vec![bait.clone(), f.p("root/in.txt")];
    confined(Some(&roots), || {
        let e = s.execute("file.export", &json!({"format": "png", "path": f.p("outside/x.png")})).unwrap_err().to_string();
        assert!(e.contains("automation path rejected"), "{e}");
        assert!(s.execute("file.export", &json!({"format": "png", "path": f.p("root/x.png")})).is_ok());
        assert!(s.execute("file.exportLayers", &json!({"dir": f.p("outside/layers")})).is_err());
        assert!(s.execute("file.open", &json!({"path": bait})).is_err());
        assert!(s.execute("file.import", &json!({"path": bait})).is_err());
        assert!(s.execute("file.placeImage", &json!({"path": f.p("outside/secret.txt")})).is_err());
        assert!(s.execute("file.open", &json!({"path": f.p("outside/readable/ok.figcraft")})).is_ok(), "read roots may be read");
        assert!(s.execute("file.save", &json!({})).is_err(), "but not written");
        assert!(s.execute("file.saveAs", &json!({"path": f.p("root/ok.figcraft")})).is_ok());
        // Recent files outside the roots aren't listed.
        let recent = s.execute("file.openRecent", &json!({})).unwrap().to_string();
        assert!(!recent.contains("bait") && recent.contains("in.txt"), "{recent}");
        assert!(s.execute("file.openRecent", &json!({"path": bait})).is_err());
    });
    assert!(f.0.join("root/x.png").is_file() && !f.0.join("outside/x.png").exists() && !f.0.join("outside/layers").exists());
}

/// Every command, given paths outside the roots in its usual path parameters, writes nothing
/// there and opens nothing from there: they all reach files through `file_access`.
#[test]
fn no_command_reaches_outside_the_roots() {
    let f = Fixture::new("sweep");
    let roots = Arc::new(f.rw());
    let outside = f.0.join("outside");
    // Bait the sweep could pick up: a document, an SVG, a PNG and variables JSON, each holding a
    // name that must never show up in a document or a result.
    let mut s = Session::new();
    s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 20, "height": 10, "name": "BAIT-LAYER"})).unwrap();
    s.execute("file.saveAs", &json!({"path": f.p("outside/bait.figcraft")})).unwrap();
    s.execute("file.export", &json!({"format": "png", "path": f.p("outside/bait.png")})).unwrap();
    std::fs::write(
        outside.join("bait.svg"),
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="9" height="9"><rect id="BAIT-LAYER" width="9" height="9"/></svg>"#,
    )
    .unwrap();
    let before: Vec<_> = std::fs::read_dir(&outside).unwrap().flatten().map(|e| e.file_name()).collect();
    // The bait is what an unconfined command finds.
    for (cmd, bait) in [("file.open", "bait.figcraft"), ("file.import", "bait.svg")] {
        let mut s = Session::new();
        s.execute(cmd, &json!({"path": f.p(&format!("outside/{bait}"))})).unwrap();
        assert!(crate::inspect::document(&s, 64, true).to_string().contains("BAIT-LAYER"), "{cmd} {bait}");
    }
    let mut s = Session::new();
    s.execute("file.placeImage", &json!({"path": f.p("outside/bait.png")})).unwrap();
    assert!(!s.doc().unwrap().images.is_empty());
    for bait in ["bait.figcraft", "bait.svg", "bait.png", "new.figcraft", "new.png"] {
        let path = f.p(&format!("outside/{bait}"));
        let params = json!({"path": path, "dir": f.p("outside"), "file": path, "variables": path, "folder": f.p("outside"), "confirm": true});
        for c in crate::all_commands() {
            let mut s = Session::new();
            let r = confined(Some(&roots), || s.execute(c.id, &params));
            let shown = format!("{r:?}");
            assert!(!shown.contains("BAIT-LAYER"), "{} read {bait}: {shown}", c.id);
            let docs = crate::inspect::document(&s, 64, true).to_string();
            assert!(!docs.contains("BAIT-LAYER"), "{} opened {bait}", c.id);
            assert!(s.docs.iter().all(|ds| ds.doc.images.is_empty()), "{} placed {bait}", c.id);
            let now: Vec<_> = std::fs::read_dir(&outside).unwrap().flatten().map(|e| e.file_name()).collect();
            assert_eq!(now, before, "{} wrote outside the roots with {bait}", c.id);
        }
    }
}

#[test]
fn recent_files_outside_the_roots_are_never_opened_removed_or_named() {
    let f = Fixture::new("recents");
    let roots = Arc::new(f.rw());
    let (bait, gone, ok) = (f.p("outside/bait.figcraft"), f.p("outside/gone.figcraft"), f.p("root/ok.figcraft"));
    let mut s = Session::new();
    s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 20, "height": 10})).unwrap();
    s.execute("file.saveAs", &json!({"path": bait})).unwrap();
    s.execute("file.saveAs", &json!({"path": ok})).unwrap();
    let mut s = Session::new();
    let recents = vec![bait.clone(), gone.clone(), ok.clone()];
    s.prefs.recent_files = recents.clone();
    let quiet = |r: &crate::Result<serde_json::Value>| {
        let shown = format!("{r:?}");
        assert!(!shown.contains("bait") && !shown.contains("gone"), "an outside recent file was named: {shown}");
    };
    confined(Some(&roots), || {
        let listing = s.execute("file.openRecent", &json!({}));
        quiet(&listing);
        assert_eq!(listing.unwrap()["files"].as_array().unwrap().len(), 1);
        // `index` counts the files listed: 0 is the first one inside the roots.
        let opened = s.execute("file.openRecent", &json!({"index": 0}));
        quiet(&opened);
        assert!(opened.is_ok(), "{opened:?}");
        assert_eq!(s.ds().unwrap().path.as_deref(), Some(ok.as_str()));
        for i in [1, 2] {
            let r = s.execute("file.openRecent", &json!({"index": i}));
            quiet(&r);
            assert!(r.is_err());
        }
        // A path outside is refused like any outside path, and never dropped from the list.
        let e = s.execute("file.openRecent", &json!({"path": gone})).unwrap_err().to_string();
        assert!(e.contains("is outside the folders that may be read") && !e.contains("no longer exists"), "{e}");
        quiet(&s.execute("file.openRecent", &json!({"remove": bait})));
        assert!(s.prefs.recent_files.contains(&bait) && s.prefs.recent_files.contains(&gone));
        // Clear clears what is listed.
        quiet(&s.execute("file.openRecent", &json!({"clear": true})));
    });
    assert_eq!(s.prefs.recent_files, vec![bait, gone]);
    // Unconfined, a recent file that is gone is still dropped.
    s.execute("file.openRecent", &json!({"index": 1})).unwrap_err();
    assert_eq!(s.prefs.recent_files.len(), 1);
}

#[test]
fn outside_paths_are_refused_alike_whatever_exists_there() {
    let f = Fixture::new("oracle");
    let r = f.rw();
    let refusal = |p: &str| r.check(Path::new(p), Access::Write).unwrap_err();
    let standard = |p: &str| format!("automation path rejected: {} is outside the folders that may be written:", Path::new(p).display());
    let mut outside = vec![f.p("outside/secret.txt"), f.p("outside/none.txt"), f.p("outside/missing/../x.txt"), f.p("nowhere/deeper/x.txt")];
    if symlink_file(&f.0.join("outside/not-yet.txt"), &f.0.join("outside/dangling.txt")) {
        outside.push(f.p("outside/dangling.txt"));
    }
    for p in &outside {
        let e = refusal(p);
        assert!(e.starts_with(&standard(p)), "{p}: {e}");
    }
    // Inside a root the reason is given.
    if symlink_file(&f.0.join("outside/not-yet.txt"), &f.0.join("root/dangling.txt")) {
        assert!(refusal(&f.p("root/dangling.txt")).contains("leads nowhere"));
    }
}

#[test]
fn only_resolved_paths_count_as_the_apps_own_folder() {
    let f = Fixture::new("own");
    let install = f.0.join("root");
    assert!(super::within_app_folder(&install.join("in.txt"), &install));
    assert!(!super::within_app_folder(&install.join("..").join("outside").join("secret.txt"), &install), "`..` leaves it");
    assert!(!super::within_app_folder(&f.0.join("root2"), &install), "a folder named like it is another");
    assert!(!super::within_app_folder(&install.join("missing.wasm"), &install));
    assert!(!super::within_app_folder(&install.join("in.txt"), &f.0.join("no-such-folder")));
}
