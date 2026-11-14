//! `window.new` under automation roots (a control client's folders): the path is checked when the
//! command runs, so a refused path is never queued, and the queued path is the one checked. The
//! user's own `window.new` (no roots) stays unconfined.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;

use crate::Session;
use crate::file_access::{Access, Roots, allowed, confined, current};

/// A fresh folder for one test: `<tmp>/figcraft-window-new-<name>-<pid>`, with `root/` (the
/// folder the roots allow) and `outside/` (not allowed).
struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str) -> Self {
        let d = std::env::temp_dir().join(format!("figcraft-window-new-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for sub in ["root", "outside"] {
            std::fs::create_dir_all(d.join(sub)).unwrap();
        }
        std::fs::write(d.join("root/in.figcraft"), b"in").unwrap();
        std::fs::write(d.join("outside/out.figcraft"), b"out").unwrap();
        Self(d)
    }

    fn p(&self, rel: &str) -> String {
        self.0.join(rel).to_string_lossy().into_owned()
    }

    fn roots(&self) -> Arc<Roots> {
        Arc::new(Roots::new(&[], &[self.p("root")]).unwrap())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `window.new` as a control client runs it: under the folders.
fn client_new(s: &mut Session, roots: &Arc<Roots>, path: &str) -> crate::Result<serde_json::Value> {
    confined(Some(roots), || s.execute("window.new", &json!({"path": path})))
}

/// The paths of the `window.new` requests queued so far.
fn queued(s: &Session) -> Vec<String> {
    s.ui_requests.iter().filter(|r| r.kind == "window.new").filter_map(|r| r.data["path"].as_str().map(str::to_string)).collect()
}

#[test]
fn a_client_window_new_outside_the_roots_is_refused_and_not_queued() {
    let f = Fixture::new("refused");
    let roots = f.roots();
    // The calibration: the folders are active for this command, and the file lies outside them.
    confined(Some(&roots), || {
        assert!(current().is_some(), "the roots are active");
        assert!(!allowed(f.p("outside/out.figcraft"), Access::Read), "the file is outside the roots");
    });
    let mut s = Session::new();
    assert!(client_new(&mut s, &roots, &f.p("outside/out.figcraft")).is_err(), "a client's window.new outside the folders is refused");
    assert!(queued(&s).is_empty(), "nothing is queued for the window");
}

#[test]
fn a_client_window_new_inside_the_roots_is_queued_with_the_checked_path() {
    let f = Fixture::new("inside");
    let roots = f.roots();
    let mut s = Session::new();
    client_new(&mut s, &roots, &f.p("root/in.figcraft")).unwrap();
    let checked = std::fs::canonicalize(f.p("root/in.figcraft")).unwrap().to_string_lossy().into_owned();
    assert_eq!(queued(&s), vec![checked]);
}

#[cfg(unix)]
#[test]
fn a_link_inside_the_roots_to_a_file_outside_is_refused_and_not_queued() {
    let f = Fixture::new("link");
    std::os::unix::fs::symlink(f.p("outside/out.figcraft"), f.0.join("root/link.figcraft")).unwrap();
    let roots = f.roots();
    let mut s = Session::new();
    assert!(client_new(&mut s, &roots, &f.p("root/link.figcraft")).is_err(), "the link leads outside the folders");
    assert!(queued(&s).is_empty());
}

#[test]
fn the_users_window_new_outside_the_roots_is_queued() {
    let f = Fixture::new("user");
    let mut s = Session::new();
    s.execute("window.new", &json!({"path": f.p("outside/out.figcraft")})).unwrap();
    assert_eq!(queued(&s), vec![f.p("outside/out.figcraft")]);
}
