//! `call` and `--attach auto`: which app is reached, and what standard input may carry.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::disallowed_methods)]

use std::net::TcpListener;

use super::*;

fn dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("figcraft-cli-call-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// The first folder searched holds the state file of an app that was killed: nothing listens on
/// its port. The app that listens wrote its own in the second folder, and it is the one reached.
#[test]
fn a_state_file_left_by_a_killed_app_does_not_hide_the_app_in_the_next_folder() {
    use figcraft_engine::discovery;
    let (stale, live) = (dir("stale"), dir("live"));
    let app = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    // Bound while `app` is, so it is another port; nothing listens on it once this line ends.
    let gone = TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
    discovery::write(&stale, gone).unwrap();
    discovery::write(&live, app.local_addr().unwrap().port()).unwrap();

    let found = auto_app(&[stale.clone(), live.clone()]);
    let _ = std::fs::remove_dir_all(&stale);
    let _ = std::fs::remove_dir_all(&live);
    let found = found.unwrap_or_else(|e| panic!("the app in the second folder was not reached: {e:?}"));
    assert_eq!(found.addr(), app.local_addr().unwrap().to_string());
}

/// Parameters longer than the limit are refused. Cut at the limit, what is left can be a whole
/// JSON object, and it would be sent to the app as if it were all the caller wrote.
#[test]
fn parameters_over_the_limit_are_refused_not_cut() {
    let head = r#"{"command":"select.all"}"#;
    let fits = format!("{head}{}", " ".repeat(MAX_CALL_PARAMS as usize - head.len()));
    assert_eq!(read_params(fits.as_bytes()).unwrap().len() as u64, MAX_CALL_PARAMS, "a text of the limit's size is read whole");
    let over = format!("{fits}{head}");
    let refused = read_params(over.as_bytes());
    assert!(
        matches!(&refused, Err(CliError::Usage(why)) if why.contains("too large")),
        "a text over the limit is refused, not cut: {:?}",
        refused.map(|text| text.len())
    );
}
