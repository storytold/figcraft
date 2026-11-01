//! The command line: parsing (flags over the config file), and managing a data directory both
//! on its own and through the server running on it.

use figcraft_collab::Replica;
use figcraft_collab::message::Access;
use figcraft_doc::Document;

use crate::cli::{self, Cli, Command, Sort, date, parse};
use crate::config::Config;
use crate::store::Store;
use crate::test_util::{TempDir, config};

fn args(s: &[&str]) -> Vec<String> {
    s.iter().map(ToString::to_string).collect()
}

fn run(dir: &std::path::Path, command: Command) -> Result<String, String> {
    cli::run(&Cli { config: config(dir), command })
}

/// A data directory with one published file: its id.
fn with_file(dir: &std::path::Path) -> String {
    let mut store = Store::open(dir, config(dir).store()).unwrap();
    let snapshot = Replica::new(Document::new("Board")).snapshot().unwrap();
    store.publish("Board", &snapshot).unwrap().0.id
}

#[test]
fn parsing() {
    let c = parse(&[]).unwrap();
    assert_eq!(c.command, Command::Serve);
    assert_eq!(c.config, Config::default());
    let c = parse(&args(&["serve", "--listen", "0.0.0.0:47900", "--data", "d", "--public-url", "wss://f.example.com/live/"])).unwrap();
    assert_eq!((c.config.listen.as_str(), c.config.data.to_str()), ("0.0.0.0:47900", Some("d")));
    assert_eq!(
        parse(&args(&["links", "create", "f1", "--access", "view", "--label", "Client"])).unwrap().command,
        Command::CreateLink { file: "f1".into(), access: Access::View, label: "Client".into() }
    );
    assert_eq!(parse(&args(&["files"])).unwrap().command, Command::Files { sort: Sort::Id });
    assert_eq!(parse(&args(&["key", "--rotate"])).unwrap().command, Command::Key { rotate: true });
    assert_eq!(parse(&args(&["-h"])).unwrap().command, Command::Help);
    for bad in
        [&["--nope"][..], &["links", "create", "f1"], &["--listen", "nowhere"], &["--listen"], &["frobnicate"], &["--public-url", "ws://h/?key=k"]]
    {
        assert!(parse(&args(bad)).is_err(), "{bad:?}");
    }
    // A config file, with flags over it; unknown keys are errors.
    let dir = TempDir::new("cli-config");
    let file = dir.path().join("server.toml");
    std::fs::write(&file, "listen = \"0.0.0.0:47901\"\nmax_files = 7\ncompact_after = 50\n").unwrap();
    let c = parse(&args(&["--config", file.to_str().unwrap(), "--listen", "127.0.0.1:1"])).unwrap();
    assert_eq!((c.config.listen.as_str(), c.config.max_files, c.config.compact_after), ("127.0.0.1:1", 7, 50));
    std::fs::write(&file, "max_files = 0\n").unwrap();
    assert!(parse(&args(&["--config", file.to_str().unwrap()])).unwrap_err().contains("max_files"));
    std::fs::write(&file, "colour = \"blue\"\n").unwrap();
    assert!(parse(&args(&["--config", file.to_str().unwrap()])).is_err());
}

#[test]
fn managing_a_directory_on_its_own() {
    let dir = TempDir::new("cli-offline");
    let file = with_file(dir.path());
    let list = run(dir.path(), Command::Files { sort: Sort::Id }).unwrap();
    assert!(list.contains(&file) && list.contains("Board"), "{list}");
    let made = run(dir.path(), Command::CreateLink { file: file.clone(), access: Access::View, label: "Client".into() }).unwrap();
    assert!(made.contains("New view link") && made.contains(&format!("file={file}&token=")), "{made}");
    let links = run(dir.path(), Command::Links { file: file.clone() }).unwrap();
    assert_eq!(links.lines().count(), 4, "a header and three links: {links}");
    let id = links.lines().find(|l| l.contains("Client")).and_then(|l| l.split_whitespace().next()).unwrap().to_string();
    run(dir.path(), Command::Revoke { file: file.clone(), link: id.clone() }).unwrap();
    assert!(run(dir.path(), Command::Revoke { file: file.clone(), link: id }).unwrap_err().contains("no such link"));
    // The key: printed as the address to publish to; replaced on request.
    let key = run(dir.path(), Command::Key { rotate: false }).unwrap();
    assert!(key.contains("ws://127.0.0.1:") && key.contains("?key="), "{key}");
    let rotated = run(dir.path(), Command::Key { rotate: true }).unwrap();
    assert_ne!(key.lines().nth(1), rotated.lines().nth(2));
    run(dir.path(), Command::RemoveFile { file: file.clone() }).unwrap();
    assert!(run(dir.path(), Command::Files { sort: Sort::Id }).unwrap().contains("No files yet"));
    assert!(run(dir.path(), Command::Links { file }).is_err());
}

#[test]
fn managing_through_the_running_server() {
    let dir = TempDir::new("cli-online");
    let file = with_file(dir.path());
    let mut server = crate::start(&config(dir.path())).unwrap();
    // The directory is the server's now: commands ask it.
    let made = run(dir.path(), Command::CreateLink { file: file.clone(), access: Access::Edit, label: "Team".into() }).unwrap();
    assert!(made.contains(&format!("host={}", server.addr())), "links name the running server: {made}");
    let links = run(dir.path(), Command::Links { file: file.clone() }).unwrap();
    assert!(links.contains("Team"), "{links}");
    let id = links.lines().find(|l| l.contains("Team")).and_then(|l| l.split_whitespace().next()).unwrap().to_string();
    assert!(run(dir.path(), Command::Revoke { file: file.clone(), link: id }).unwrap().contains("Revoked"));
    assert!(run(dir.path(), Command::Files { sort: Sort::Id }).unwrap().contains("Board"));
    assert!(run(dir.path(), Command::Key { rotate: true }).unwrap_err().contains("stop the server"));
    assert!(run(dir.path(), Command::Key { rotate: false }).unwrap().contains(&format!("ws://{}/?key=", server.addr())));
    server.stop();
    // Stopped: the directory is free again.
    assert!(run(dir.path(), Command::Files { sort: Sort::Id }).unwrap().contains("Board"));
}

#[test]
fn dates() {
    assert_eq!(date(0), "1970-01-01 00:00 UTC");
    assert_eq!(date(951_782_400), "2000-02-29 00:00 UTC");
    assert_eq!(date(1_791_986_580), "2026-10-14 14:03 UTC");
}
