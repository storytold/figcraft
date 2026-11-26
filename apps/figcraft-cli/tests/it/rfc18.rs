//! RFC-0018 rule 2.7, in-process through `run`: help, versions, usage errors with exit 2 on stderr,
//! the files a command would replace, and the environment variables its flags declare.

use std::ffi::OsString;
use std::process::{Command, Stdio};

struct Outcome {
    code: i32,
    out: String,
    err: String,
}

fn invoke<I, T>(args: I) -> Outcome
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = figcraft_cli::run(args, &mut out, &mut err);
    Outcome { code, out: String::from_utf8_lossy(&out).into_owned(), err: String::from_utf8_lossy(&err).into_owned() }
}

fn workdir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("figcraft-cli-rfc18-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn usage_error(args: &[&str]) -> String {
    let got = invoke(args.iter().copied());
    assert_eq!(got.code, 2, "{args:?}: {}", got.err);
    assert_eq!(got.out, "", "{args:?} wrote to stdout");
    assert!(got.err.starts_with("error: "), "{args:?}: {}", got.err);
    got.err
}

const SUBCOMMANDS: [&[&str]; 10] = [
    &["info"],
    &["export"],
    &["convert"],
    &["render"],
    &["run"],
    &["commands"],
    &["mcp"],
    &["plugin", "publish"],
    &["connect", "publish"],
    &["perf"],
];

#[test]
fn every_subcommand_takes_help_and_the_version() {
    let top = invoke(["--version"]);
    assert_eq!((top.code, top.err.as_str()), (0, ""));
    let version = top.out.strip_prefix("figcraft-cli ").unwrap().trim_end().to_string();
    for sub in SUBCOMMANDS {
        for flag in ["--help", "-h"] {
            let got = invoke(sub.iter().copied().chain([flag]));
            assert_eq!((got.code, got.err.as_str()), (0, ""), "{sub:?} {flag}");
            assert!(got.out.contains(&format!("Usage: figcraft-cli {}", sub.join(" "))), "{sub:?}: {}", got.out);
        }
        for flag in ["--version", "-V"] {
            let got = invoke(sub.iter().copied().chain([flag]));
            assert_eq!((got.code, got.err.as_str()), (0, ""), "{sub:?} {flag}");
            assert_eq!(
                got.out,
                format!("{} {version}\n", ["figcraft-cli"].iter().chain(sub.iter()).copied().collect::<Vec<_>>().join("-")),
                "{sub:?}"
            );
        }
    }
    usage_error(&["version"]);
}

#[test]
fn no_arguments_and_unknown_flags_are_usage_errors() {
    let got = invoke(Vec::<OsString>::new());
    assert_eq!((got.code, got.out.as_str()), (2, ""));
    assert!(got.err.contains("Usage: figcraft-cli"), "{}", got.err);
    for sub in SUBCOMMANDS {
        let args: Vec<&str> = sub.iter().copied().chain(["--fromat"]).collect();
        assert!(usage_error(&args).contains("--fromat"), "{sub:?}");
    }
    usage_error(&["--fromat"]);
}

#[test]
fn a_flag_given_twice_and_bad_values_are_refused() {
    usage_error(&["info", "x.figcraft", "--json", "--json"]);
    usage_error(&["render", "x.figcraft", "-o", "a.png", "--scale", "0"]);
    usage_error(&["export", "x.figcraft", "-o", "out", "--format", "bmp"]);
    usage_error(&["mcp", "--attach", "nope"]);
    usage_error(&["run", "s.json", "--in-place"]);
}

#[test]
fn commands_lists_in_json_and_as_text() {
    let json = invoke(["commands", "--json", "--filter", "layer"]);
    assert_eq!((json.code, json.err.as_str()), (0, ""));
    assert!(serde_json::from_str::<serde_json::Value>(&json.out).unwrap().as_array().is_some_and(|l| !l.is_empty()));
    let text = invoke(["commands", "--filter", "layer"]);
    assert_eq!(text.code, 0);
    assert!(text.out.contains("  "), "{}", text.out);
}

#[test]
fn a_failed_command_exits_1_and_a_closed_stdout_is_not_an_error() {
    let got = invoke(["info", "missing.figcraft"]);
    assert_eq!((got.code, got.out.as_str()), (1, ""));
    assert!(got.err.starts_with("error: "), "{}", got.err);
    struct Closed;
    impl std::io::Write for Closed {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut err = Vec::new();
    assert_eq!(figcraft_cli::run(["commands"], &mut Closed, &mut err), 0);
    assert!(err.is_empty());
}

#[test]
fn an_output_that_would_replace_an_input_is_refused_without_in_place() {
    let dir = workdir("replace");
    let a = dir.join("a.figcraft");
    let a = a.to_str().unwrap();
    let first = invoke(["convert", "-", "-"]);
    assert_eq!(first.code, 1, "{}", first.err); // a missing input is a failure, not a usage error
    // Make a real document: a script into a new file.
    let script = dir.join("s.json");
    std::fs::write(&script, r#"[{"command": "frame.create", "params": {"x": 0, "y": 0, "width": 10, "height": 10, "name": "F"}}]"#).unwrap();
    let made = invoke(["run", script.to_str().unwrap(), "--out", a]);
    assert_eq!(made.code, 0, "{}", made.err);
    let before = std::fs::read(a).unwrap();
    let dotted = dir.join(".").join("a.figcraft");
    for args in [
        vec!["convert", a, a],
        vec!["convert", a, dotted.to_str().unwrap()],
        vec!["run", script.to_str().unwrap(), "--in", a, "--out", a],
        vec!["export", a, "-o", a],
        vec!["render", a, "-o", a],
    ] {
        let message = usage_error(&args);
        assert!(message.contains("would replace"), "{args:?}: {message}");
    }
    assert_eq!(std::fs::read(a).unwrap(), before, "a refused command changes nothing");
    let other = dir.join("b.figcraft");
    let other = other.to_str().unwrap();
    assert!(usage_error(&["convert", a, other, "--in-place"]).contains("--in-place"));
    assert!(usage_error(&["run", script.to_str().unwrap(), "--in", a, "--out", other, "--in-place"]).contains("--in-place"));
    let got = invoke(["run", script.to_str().unwrap(), "--in", a, "--out", a, "--in-place"]);
    assert_eq!(got.code, 0, "{}", got.err);
    let got = invoke(["convert", a, a, "--in-place"]);
    assert_eq!(got.code, 0, "{}", got.err);
    std::fs::remove_dir_all(dir).unwrap();
}

fn binary() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_figcraft-cli"));
    command.env_remove("FIGCRAFT_NO_SYSTEM_FONTS").env_remove("FIGCRAFT_SERVER_KEY");
    command
}

#[test]
fn the_variables_are_declared_on_their_flags() {
    // The font variable takes the usual boolean spellings, and an empty one is unset.
    for (value, code) in [("1", 0), ("true", 0), ("0", 0), ("", 0), ("maybe", 2)] {
        let output =
            binary().args(["commands", "--filter", "layer.add"]).env("FIGCRAFT_NO_SYSTEM_FONTS", value).stdin(Stdio::null()).output().unwrap();
        assert_eq!(output.status.code(), Some(code), "{value:?}: {}", String::from_utf8_lossy(&output.stderr));
    }
    // The server key's value is never printed in help; an unreachable server reports itself, not the key.
    let secret = "do-not-print-this-key-0123456789";
    let help = binary().args(["plugin", "publish", "--help"]).env("FIGCRAFT_SERVER_KEY", secret).output().unwrap();
    let help = String::from_utf8_lossy(&help.stdout).into_owned();
    assert!(help.contains("FIGCRAFT_SERVER_KEY") && !help.contains(secret), "{help}");
    let dir = workdir("key");
    let failed =
        binary().args(["plugin", "publish"]).arg(&dir).args(["--server", "ws://127.0.0.1:9/"]).env("FIGCRAFT_SERVER_KEY", secret).output().unwrap();
    assert_eq!(failed.status.code(), Some(1));
    assert!(!String::from_utf8_lossy(&failed.stderr).contains(secret), "{}", String::from_utf8_lossy(&failed.stderr));
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn a_file_name_that_is_not_valid_unicode_is_a_usage_error() {
    use std::os::unix::ffi::OsStringExt;
    let got = invoke([OsString::from("info"), OsString::from_vec(b"caf\xe9.figcraft".to_vec())]);
    assert_eq!((got.code, got.out.as_str()), (2, ""));
    assert!(got.err.starts_with("error: "), "{}", got.err);
}

#[test]
fn perf_prints_its_tables_on_stdout_and_its_progress_on_stderr() {
    let got = invoke(["perf", "--no-system-fonts", "--layers", "10", "--instances", "5"]);
    assert_eq!(got.code, 0, "{}", got.err);
    assert!(got.out.starts_with("figcraft perf ("), "{}", got.out);
    assert!(got.out.contains("| operation") && got.out.contains("10 layers") && got.out.contains("5 inst."), "{}", got.out);
    assert!(got.err.contains("measuring 10 layers") && got.err.contains("measuring components with 5 instances"), "{}", got.err);
    // `--instances 0` leaves the components table out.
    let got = invoke(["perf", "--no-system-fonts", "--layers", "10", "--instances", "0"]);
    assert_eq!(got.code, 0, "{}", got.err);
    assert!(!got.out.contains("inst."), "{}", got.out);
}

#[test]
fn a_server_key_flag_and_an_unreachable_app_fail_with_exit_1_and_a_message() {
    let dir = workdir("publish");
    let missing = dir.join("no-such-plugin");
    let got = invoke(["plugin", "publish", missing.to_str().unwrap(), "--server", "ws://127.0.0.1:1/", "--key", "secret-key"]);
    assert_eq!(got.code, 1, "{}", got.err);
    assert!(got.err.starts_with("error: "), "{}", got.err);
    assert!(!got.err.contains("secret-key") && got.out.is_empty(), "{} {}", got.err, got.out);
    let got = invoke(["mcp", "--attach", "127.0.0.1:1"]);
    assert_eq!(got.code, 1, "{}", got.err);
    assert!(got.err.contains("can't reach the FigCraft app at 127.0.0.1:1"), "{}", got.err);
    let _ = std::fs::remove_dir_all(dir);
}
