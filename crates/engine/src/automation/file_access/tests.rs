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

/// How each spelling of a path on another computer or of a device is told apart from a local path
/// (#1094), on every system: only Windows checks them, but they are spelled the same everywhere.
#[test]
fn spellings_of_other_computers_and_devices_are_refused() {
    use super::refused_spelling;
    use craft_docpath::Refused::{Device, Network};
    let local = [PathBuf::from(r"C:\work")];
    // On `\\192.0.2.1\share`.
    let on_the_share = [
        r"\\192.0.2.1\share\x.png",
        "//192.0.2.1/share/x.png",
        r"\/192.0.2.1/share\x.png",
        r"\\?\UNC\192.0.2.1\share\x.png",
        r"\\?\unc\192.0.2.1\share\x.png",
        "//?/UNC/192.0.2.1/share/x.png",
        r"\\.\UNC\192.0.2.1\share\x.png",
    ];
    let elsewhere = [
        // A server without a share is contacted too.
        r"\\192.0.2.1",
        r"\\192.0.2.1\",
        r"\\SERVER.\share \x.png",
    ];
    let network = [&on_the_share[..], &elsewhere[..]].concat();
    let device = [
        r"\\.\pipe\figcraft",
        r"\\?\GLOBALROOT\Device\Mup\192.0.2.1\share\x.png",
        r"\\.\C:\work\x.png",
        r"\\.\COM1",
        r"\??\UNC\192.0.2.1\share\x.png",
        // A device name anywhere: each folder on the way is opened as the path is resolved.
        r"C:\work\COM1",
        r"C:\work\nul.txt",
        r"C:\work\Com1 .png",
        r"C:\work\lpt1.\x.png",
        r"C:\work\LPT³\x.png",
        r"C:\work\conout$",
        r"C:\work\aux:stream",
        // After a name `craft_docpath` refuses for another reason.
        r"C:\work\a:b\prn.png",
    ];
    let fine = [
        r"C:\work\x.png",
        r"\\?\C:\work\x.png",
        r"C:\work\console.png",
        r"C:\work\com10.png",
        r"C:\work\lpt.png",
        r"C:\work\nul-ish.txt",
        r"\\?\Volume{0b3b1c63-0000-0000-0000-100000000000}\x.png",
        r"\\.\Volume{0b3b1c63-0000-0000-0000-100000000000}\x.png",
        "/home/u/work/x.png",
        "x.png",
        r"..\x.png",
    ];
    for (paths, want) in [(&network[..], Some(Network)), (&device[..], Some(Device)), (&fine[..], None)] {
        for p in paths {
            assert_eq!(refused_spelling(p, &[]), want, "{p}");
            assert_eq!(refused_spelling(p, &local), want, "{p} with a local root");
        }
    }
    // A root on a share lets the paths on that share through, in any case and spelling, and only them:
    // every spelling above of a path on `\\192.0.2.1\share`, but not the server without a share
    // or another server.
    for root in [r"\\?\UNC\192.0.2.1\share", r"\\192.0.2.1\share\work"] {
        let on_share = [PathBuf::from(root)];
        for p in on_the_share {
            assert_eq!(refused_spelling(p, &on_share), None, "{p} with root {root}");
        }
        for p in elsewhere {
            assert_eq!(refused_spelling(p, &on_share), Some(Network), "{p} with root {root}");
        }
    }
    let shared = [PathBuf::from(r"\\?\UNC\Server\Share\work")];
    for p in [r"\\server\SHARE\work\x.png", r"\\?\UNC\SERVER\share\elsewhere\x.png", "//Server/Share/work/x.png"] {
        assert_eq!(refused_spelling(p, &shared), None, "{p}");
    }
    for p in [r"\\server\other\x.png", r"\\other\share\x.png", r"\\server", r"\\server.\share\x.png"] {
        assert_eq!(refused_spelling(p, &shared), Some(Network), "{p}");
    }
    assert_eq!(refused_spelling(r"\\server\share\work\COM1", &shared), Some(Device));
}

/// The links on the way to a path are followed by their spelling before anything they lead to is
/// opened (#1094). The links are made up here, so this runs where tests may not make links.
#[test]
fn links_on_the_way_are_followed_by_their_spelling_first() {
    use super::{Entry, MAX_LINKS, link_refused};
    use std::cell::RefCell;
    use std::collections::HashMap;
    let base = std::env::temp_dir().join("figcraft-links");
    let at = |rel: &str| base.join(rel);
    let mut links: HashMap<PathBuf, Option<PathBuf>> = HashMap::from([
        // Relative to the link's folder, to a device name.
        (at("root/dev"), Some(PathBuf::from("../outside/nul"))),
        // To another link.
        (at("root/hop"), Some(at("root/dev"))),
        // Within the root.
        (at("root/alias"), Some(PathBuf::from("sub"))),
        (at("root/loop"), Some(at("root/loop"))),
        (at("root/unreadable"), None),
        // Below a name that isn't there: never looked at.
        (at("missing/dev"), Some(PathBuf::from("nul"))),
    ]);
    if cfg!(windows) {
        links.insert(at("root/remote"), Some(PathBuf::from(r"\\192.0.2.1\share")));
        links.insert(at("root/far"), Some(at("root/remote/work")));
    }
    // Chains of `n` links, `root/<n>-1` to `root/<n>-2` and so on, the last one to `root/sub`.
    for n in [MAX_LINKS - 1, MAX_LINKS, MAX_LINKS + 1] {
        for i in 1..=n {
            let next = if i == n { at("root/sub") } else { at(&format!("root/{n}-{}", i + 1)) };
            links.insert(at(&format!("root/{n}-{i}")), Some(next));
        }
    }
    let plain = [at("root"), at("root/sub"), at("outside")];
    // What can't be looked at (no permission, say) may be a link, so it isn't taken for nothing.
    let unreadable = at("root/locked");
    let looked = RefCell::new(Vec::new());
    let look = |p: &Path| {
        looked.borrow_mut().push(p.to_path_buf());
        match links.get(p) {
            Some(target) => Entry::Link(target.clone()),
            None if base.starts_with(p) || plain.iter().any(|q| q == p) => Entry::Plain,
            None if p == unreadable => Entry::Unreadable,
            None => Entry::Missing,
        }
    };
    let refused = |rel: &str, roots: &[PathBuf]| link_refused(&at(rel), roots, MAX_LINKS, &look);
    for rel in ["root/dev/x.png", "root/hop/x.png", "root/loop/x.png", "root/unreadable/x.png", "root/locked/x.png", "root/locked"] {
        assert!(refused(rel, &[]), "{rel}");
    }
    for rel in ["root/alias/x.png", "root/sub/x.png", "root/x.png", "missing/dev/x.png"] {
        assert!(!refused(rel, &[]), "{rel}");
    }
    // As many links as are followed lead on; one more is refused.
    assert!(!refused(&format!("root/{}-1/x.png", MAX_LINKS - 1), &[]));
    assert!(!refused(&format!("root/{MAX_LINKS}-1/x.png"), &[]));
    assert!(refused(&format!("root/{}-1/x.png", MAX_LINKS + 1), &[]));
    // A refused target isn't looked at, nor is anything below what isn't there.
    let looked = looked.take();
    assert!(!looked.contains(&at("outside/nul")) && !looked.contains(&at("missing/dev")), "{looked:?}");
    if cfg!(windows) {
        for rel in ["root/remote/x.png", "root/far/x.png"] {
            assert!(refused(rel, &[]), "{rel}");
            // Unless a root is on that share.
            assert!(!refused(rel, &[PathBuf::from(r"\\?\UNC\192.0.2.1\share")]), "{rel}");
        }
    }
}

/// A path that can't be looked at is refused rather than taken for one with nothing there (#1094):
/// only a failure that says nothing is there lets the walk stop, and a path that can't even be
/// made absolute isn't left to the checks after it.
#[test]
fn what_cannot_be_looked_at_is_refused() {
    use super::{Unresolved, nothing_there, refused_unresolved};
    use std::io::{Error, ErrorKind};
    for kind in [ErrorKind::NotFound, ErrorKind::NotADirectory, ErrorKind::InvalidFilename] {
        assert!(nothing_there(&Error::from(kind)), "{kind:?}");
    }
    for kind in [ErrorKind::PermissionDenied, ErrorKind::ResourceBusy, ErrorKind::Interrupted, ErrorKind::Other] {
        assert!(!nothing_there(&Error::from(kind)), "{kind:?}");
    }
    assert_eq!(refused_unresolved(Path::new(""), &[]), Some(Unresolved::Linked));
}

/// On Windows a path is classified as Windows opens it (#1094): with `..` taken by name before
/// anything is opened (so a link followed by `..` is never gone through), and with names that
/// aren't Unicode (a lone surrogate) still read for the parts that are spelled in ASCII.
#[cfg(windows)]
#[test]
fn paths_are_classified_as_windows_opens_them() {
    use super::{Unresolved, refused_unresolved};
    use craft_docpath::Refused::{Device, Network};
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    assert_eq!(std::path::absolute(r"C:\work\remote\..\x.png").unwrap(), PathBuf::from(r"C:\work\x.png"));
    // `before`, a lone surrogate, then `after`.
    let odd = |before: &str, after: &str| {
        let wide: Vec<u16> = before.encode_utf16().chain([0xD800]).chain(after.encode_utf16()).collect();
        PathBuf::from(OsString::from_wide(&wide))
    };
    assert_eq!(refused_unresolved(&odd(r"\\192.0.2.1\share\", ".png"), &[]), Some(Unresolved::Spelled(Network)));
    assert_eq!(refused_unresolved(&odd(r"C:\figcraft-no-such-folder\", r"\nul"), &[]), Some(Unresolved::Spelled(Device)));
    assert_eq!(refused_unresolved(&odd(r"C:\figcraft-no-such-folder\", ".png"), &[]), None);
    // A NUL can't be passed to Windows at all, so the path can't be made absolute.
    assert_eq!(refused_unresolved(Path::new("C:\\work\\a\0b.png"), &[]), Some(Unresolved::Linked));
}

/// `..` after a link is taken by name, by the check and by Windows when it opens the path, so
/// `root\j\..\in.txt` is `root\in.txt` and the link `j` is never gone through (#1094). Shown with
/// a junction, a link any account can make: `j` leads out of the root to `outside`, and beside
/// `outside` is another `in.txt`, which is what `..` would reach through the link.
#[cfg(windows)]
#[test]
fn a_parent_after_a_link_is_taken_by_name() {
    use super::{Entry, on_disk};
    let f = Fixture::new("parent");
    let r = f.rw();
    std::fs::write(f.0.join("in.txt"), b"beside").unwrap();
    let made = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(f.0.join(r"root\j")).arg(f.0.join("outside")).output();
    assert!(made.is_ok_and(|o| o.status.success()), "a junction can be made");
    // The junction is a link to the check, and what is through it is outside.
    assert!(matches!(on_disk(&f.0.join(r"root\j")), Entry::Link(Some(_))));
    assert!(r.check(Path::new(&f.p(r"root\j\secret.txt")), Access::Read).is_err());
    let through = f.p(r"root\j\..\in.txt");
    assert_eq!(r.check(Path::new(&through), Access::Read), r.check(Path::new(&f.p(r"root\in.txt")), Access::Read));
    assert!(r.check(Path::new(&through), Access::Read).is_ok());
    assert_eq!(std::fs::read(&through).unwrap(), b"in", "Windows opens the file in the root");
}

/// A path on another computer and a device path are refused before anything is opened: resolving
/// them made Windows contact the server, which may send it the signed-in user's credentials
/// (#1094). Each is refused with a message that only the check before resolving gives: had the
/// path been resolved, 192.0.2.1 (an address for documentation, where nothing answers) would have
/// been contacted and the message would be the one for a path that can't be resolved.
#[cfg(windows)]
#[test]
fn other_computers_and_devices_are_refused_before_anything_is_opened() {
    let f = Fixture::new("network");
    let r = f.rw();
    let read_only = Roots::new(&[f.p("root")], &[]).unwrap();
    let refused = |roots: &Roots, p: &str, access: Access| roots.check(Path::new(p), access).expect_err(p);
    for p in
        [r"\\192.0.2.1\share\x.png", "//192.0.2.1/share/x.png", r"\\?\UNC\192.0.2.1\share\x.png", r"\\.\UNC\192.0.2.1\share\x.png", r"\\192.0.2.1"]
    {
        for access in [Access::Read, Access::Write] {
            let e = refused(&r, p, access);
            assert!(e.contains("is on another computer, and no folder that may be"), "{p}: {e}");
        }
        let e = refused(&read_only, p, Access::Write);
        assert!(e.ends_with("is on another computer, and no folder that may be written is on its share"), "{p}: {e}");
    }
    for p in [r"\\.\pipe\figcraft-roots", r"\\?\GLOBALROOT\Device\Mup\192.0.2.1\share\x.png", &f.p(r"root\COM1\x.png"), "nul"] {
        for access in [Access::Read, Access::Write] {
            let e = refused(&r, p, access);
            assert!(e.ends_with("names a device, not a file"), "{p}: {e}");
        }
    }
    // Local paths, in either spelling, are checked as before.
    let inside = std::fs::canonicalize(f.p("root")).unwrap().join("in.txt");
    assert!(r.check(&inside, Access::Read).is_ok());
    assert!(r.check(Path::new(&f.p(r"root\in.txt")), Access::Read).is_ok());
}

/// A link in a root to another computer is refused before it is followed (#1094), also through
/// another link, and with the message any path outside gets: it doesn't tell where the link
/// leads. Had the link been followed, the message would be the one for a path that can't be
/// resolved. This needs real links, which Windows lets only some accounts make: elsewhere it
/// says so and checks nothing (`links_on_the_way_are_followed_by_their_spelling_first` covers the
/// same walk with made-up links on every system).
#[cfg(windows)]
#[test]
fn links_to_other_computers_are_refused_before_they_are_followed() {
    let f = Fixture::new("network-links");
    let r = f.rw();
    if !symlink_dir(Path::new(r"\\192.0.2.1\share"), &f.0.join("root/remote")) || !symlink_dir(&f.0.join("root/remote/work"), &f.0.join("root/hop")) {
        eprintln!("skipped: this system doesn't let tests make symbolic links");
        return;
    }
    // Relative to the link's folder.
    assert!(symlink_dir(Path::new(r"..\remote"), &f.0.join("root/sub/up")));
    assert!(symlink_file(Path::new(r"\\.\pipe\figcraft-roots"), &f.0.join("root/pipe")));
    let standard = |p: &str| format!("automation path rejected: {p} is outside the folders that may be");
    for p in [f.p("root/remote/x.png"), f.p("root/hop/x.png"), f.p("root/sub/up/x.png"), f.p("root/pipe")] {
        for access in [Access::Read, Access::Write] {
            let e = r.check(Path::new(&p), access).expect_err(&p);
            assert!(e.starts_with(&standard(&p)), "{p}: {e}");
        }
    }
    // `..` after the link is taken by name, so the link isn't gone through.
    assert!(r.check(Path::new(&f.p(r"root\remote\..\in.txt")), Access::Read).is_ok());
    // A link to a folder in the root still leads there.
    assert!(symlink_dir(&f.0.join("root/sub"), &f.0.join("root/alias")));
    assert!(r.check(Path::new(&f.p("root/alias/x.png")), Access::Write).is_ok());
}

#[test]
fn a_scope_confines_the_thread_and_ends_with_it() {
    let f = Fixture::new("scope");
    let roots = Arc::new(f.rw());
    assert!(current().is_none());
    confined(Some(&roots), || {
        assert!(allow(f.p("outside/secret.txt"), Access::Read).is_err());
        assert!(super::read_capped(f.p("outside/secret.txt"), 64).is_err());
        assert_eq!(super::read_capped(f.p("root/in.txt"), 64).unwrap(), b"in");
        // Not confined: the app's own files.
        assert_eq!(super::app_owned(|| super::read_capped(f.p("outside/secret.txt"), 64).unwrap()), b"secret");
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
        // Reached through a link in the root, it is outside all the same.
        if symlink_dir(&f.0.join("outside"), &f.0.join("root/escape")) {
            outside.push(f.p("root/escape/dangling.txt"));
        }
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
fn a_root_given_through_a_link_tells_why_paths_in_it_are_refused() {
    let f = Fixture::new("linked-root");
    if !symlink_dir(&f.0.join("root"), &f.0.join("via")) || !symlink_file(&f.0.join("outside/not-yet.txt"), &f.0.join("root/dangling.txt")) {
        eprintln!("skipped: this system doesn't let tests make symbolic links");
        return;
    }
    // Like `/tmp/work` on macOS, where `/tmp` leads to `/private/tmp`.
    let r = Roots::new(&[], &[f.p("via")]).unwrap();
    let e = r.check(Path::new(&f.p("via/dangling.txt")), Access::Write).unwrap_err();
    assert!(e.contains("leads nowhere"), "{e}");
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

#[test]
fn sizes_read_as_a_person_says_them() {
    assert_eq!((super::size(300), super::size((12 << 20) - 5), super::size(13 << 29)), ("300 bytes".into(), "12 MB".into(), "6.5 GB".into()));
}
