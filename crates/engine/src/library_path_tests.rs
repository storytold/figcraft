//! A library link is stored in the document, so it is followed only on this computer
//! ([`crate::library_server`]): a path on another server would make opening someone else's file
//! contact that server, and on Windows send the credentials of whoever opened it.

use crate::Session;

#[test]
fn a_library_is_opened_only_from_this_computer() {
    let mut s = Session::new();
    for path in [
        r"\\attacker.example\share\lib.figcraft",
        "//attacker.example/share/lib.figcraft",
        "file://attacker.example/share/lib.figcraft",
        r"\\?\UNC\attacker.example\share\lib.figcraft",
        r"\\.\pipe\lib",
        "https://attacker.example/lib.figcraft",
    ] {
        let e = s.fetch_library(path, false).expect_err(&format!("{path} must be refused"));
        assert!(e.to_string().contains("only from this computer"), "{path}: {e}");
    }
}

#[test]
fn a_local_library_path_is_still_followed() {
    let mut s = Session::new();
    // Refused for being absent, not for where it is: the gate let it through.
    let missing = std::env::temp_dir().join("figcraft-no-such-library.figcraft");
    let e = s.fetch_library(&missing.to_string_lossy(), false).expect_err("the file does not exist");
    assert!(!e.to_string().contains("only from this computer"), "a local path must not be refused as remote: {e}");
}

#[test]
fn the_library_cache_refuses_a_path_off_this_computer() {
    let mut cache = crate::library::Cache::default();
    for path in [r"\\attacker.example\share\lib.figcraft", "//attacker.example/share/lib.figcraft"] {
        let e = cache.load(path).expect_err(&format!("{path} must be refused"));
        assert!(e.to_string().contains("only from this computer"), "{path}: {e}");
    }
}
