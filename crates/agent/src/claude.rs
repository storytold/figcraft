//! The Claude Code CLI as a backend: one `claude -p` process per message, resuming the chat's session,
//! with its own tools off and FigCraft's MCP server as its only tools. It uses the person's own Claude
//! Code login, and keeps the chat in Claude Code's own history as any session of it.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{Value, json};

use crate::{Cancel, Event, Model, TOOLS, bridge, clip, system_prompt, tool_detail};

const MCP_SERVER: &str = "figcraft";
/// One line of the CLI's output: a tool result may hold a whole picture.
const MAX_LINE: u64 = 64 * 1024 * 1024;
const MAX_TEXT: usize = 8 * 1024;
const MAX_STDERR: u64 = 16 * 1024;
const POLL: Duration = Duration::from_millis(100);
/// How long the CLI may take to say which models it has.
const MODELS_TIMEOUT: Duration = Duration::from_secs(30);
/// The answer to this request names the models. It calls no model and uses no tokens.
const MODELS_REQUEST: &str = "figcraft-models";

/// One run's process.
#[derive(Clone, Debug)]
pub struct Config {
    /// The `claude` program.
    pub program: PathBuf,
    /// Empty: Claude Code's default model.
    pub model: String,
    /// The program that serves MCP for this run (the app, with `mcp_args`).
    pub mcp_command: PathBuf,
    pub mcp_args: Vec<String>,
    /// [`bridge::ENV`]'s `port:key`, inherited by the MCP server through the CLI.
    pub bridge: String,
    /// The session of the chat so far, to continue it.
    pub resume: Option<String>,
}

/// Where a `claude` program is. An app opened from the Dock or a file manager doesn't get the shell's
/// `PATH`, so the usual install folders are tried too.
pub fn detect() -> Option<PathBuf> {
    let name = if cfg!(windows) { "claude.exe" } else { "claude" };
    let path = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default();
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    let usual = home.iter().flat_map(|h| [h.join(".local/bin"), h.join(".claude/local"), h.join(".npm-global/bin"), h.join("bin")]);
    path.into_iter().chain(usual).chain(["/opt/homebrew/bin".into(), "/usr/local/bin".into()]).map(|d| d.join(name)).find(|p| p.is_file())
}

/// The names of the tools the CLI may call: everything else is denied in print mode.
fn allowed_tools() -> String {
    TOOLS.iter().map(|t| format!("mcp__{MCP_SERVER}__{t}")).collect::<Vec<_>>().join(",")
}

/// The MCP configuration, given on the command line: it holds no secret, the key goes by environment.
fn mcp_config(config: &Config) -> String {
    json!({"mcpServers": {MCP_SERVER: {"command": config.mcp_command, "args": config.mcp_args}}}).to_string()
}

/// The CLI's arguments. The prompt goes through standard input, so it can't be read as a flag.
pub fn args(config: &Config) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        "--output-format",
        "stream-json",
        "--verbose",
        // Only this app's instructions: no project's or person's settings, hooks or CLAUDE.md.
        "--setting-sources",
        "",
        // Only FigCraft's server, and no shell, file or web tools.
        "--strict-mcp-config",
        "--tools",
        "",
        "--allowedTools",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    args.push(allowed_tools());
    // In place of Claude Code's own prompt, which is about writing code.
    args.extend(["--mcp-config".to_string(), mcp_config(config), "--system-prompt".to_string(), system_prompt()]);
    if !config.model.trim().is_empty() {
        args.extend(["--model".to_string(), config.model.trim().to_string()]);
    }
    if let Some(session) = &config.resume {
        args.extend(["--resume".to_string(), session.clone()]);
    }
    args
}

/// The models in the CLI's answer to [`MODELS_REQUEST`], when `line` is that answer.
fn parse_models(line: &str) -> Option<Result<Vec<Model>, String>> {
    let v: Value = serde_json::from_str(line).ok()?;
    let response = v.get("response").filter(|_| v.get("type").and_then(Value::as_str) == Some("control_response"))?;
    if response.get("request_id").and_then(Value::as_str) != Some(MODELS_REQUEST) {
        return None;
    }
    if response.get("subtype").and_then(Value::as_str) != Some("success") {
        let why = response.get("error").and_then(Value::as_str).unwrap_or("it refused");
        return Some(Err(format!("Claude Code didn't list its models: {}", clip(why, MAX_TEXT))));
    }
    let text = |m: &Value, key: &str| m.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let models = response.pointer("/response/models").and_then(Value::as_array).into_iter().flatten();
    let models: Vec<Model> = models
        .filter(|m| !text(m, "value").is_empty())
        .map(|m| {
            let label = Some(text(m, "displayName")).filter(|l| !l.is_empty()).unwrap_or_else(|| text(m, "value"));
            Model { value: text(m, "value"), label, detail: text(m, "description") }
        })
        .collect();
    Some(Ok(models))
}

/// The models this `claude` program offers the person's login, from its startup handshake.
pub fn list_models(program: &Path) -> Result<Vec<Model>, String> {
    let mut command = Command::new(program);
    command
        .args([
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--no-session-persistence",
            "--strict-mcp-config",
            "--tools",
            "",
        ])
        .env_remove("CLAUDECODE")
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(path) = search_path(program) {
        command.env("PATH", path);
    }
    let mut child = command.spawn().map_err(|e| format!("can't start {}: {e}", program.display()))?;
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("can't talk to Claude Code".into());
    };
    let request = json!({"type": "control_request", "request_id": MODELS_REQUEST, "request": {"subtype": "initialize"}});
    let written = writeln!(stdin, "{request}");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = Vec::new();
        loop {
            line.clear();
            match (&mut reader).take(MAX_LINE).read_until(b'\n', &mut line) {
                Ok(n) if n > 0 => {
                    if let Some(answer) = parse_models(&String::from_utf8_lossy(&line)) {
                        let _ = tx.send(answer);
                        return;
                    }
                }
                _ => return,
            }
        }
    });
    let answer = written.map_err(|e| e.to_string()).and_then(|()| {
        rx.recv_timeout(MODELS_TIMEOUT).map_err(|_| "Claude Code didn't list its models: check that it runs and is logged in".to_string())?
    });
    let _ = child.kill();
    let _ = child.wait();
    answer
}

/// What one line of `stream-json` output reports.
pub fn parse_line(line: &str) -> Vec<Event> {
    let Ok(v) = serde_json::from_str::<Value>(line) else { return Vec::new() };
    let content = || v.pointer("/message/content").and_then(Value::as_array).into_iter().flatten();
    match v.get("type").and_then(Value::as_str) {
        Some("system") if v.get("subtype").and_then(Value::as_str) == Some("init") => {
            let server = v
                .get("mcp_servers")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find(|s| s.get("name").and_then(Value::as_str) == Some(MCP_SERVER));
            let mut events: Vec<Event> = v.get("session_id").and_then(Value::as_str).map(|id| Event::Session(id.to_string())).into_iter().collect();
            match server.and_then(|s| s.get("status")).and_then(Value::as_str) {
                Some("connected") | Some("pending") | None => {}
                Some(status) => events.push(Event::Error(format!("FigCraft's tools didn't connect to Claude Code ({})", clip(status, 80)))),
            }
            events
        }
        Some("assistant") => content()
            .filter_map(|c| match c.get("type").and_then(Value::as_str) {
                Some("text") => {
                    c.get("text").and_then(Value::as_str).map(str::trim).filter(|t| !t.is_empty()).map(|t| Event::Text(clip(t, MAX_TEXT)))
                }
                Some("tool_use") => {
                    let name = c.get("name").and_then(Value::as_str).unwrap_or_default();
                    let name = name.strip_prefix(&format!("mcp__{MCP_SERVER}__")).unwrap_or(name);
                    Some(Event::Tool(name.to_string(), tool_detail(name, c.get("input").unwrap_or(&Value::Null))))
                }
                _ => None,
            })
            .collect(),
        Some("user") => content()
            .filter(|c| c.get("type").and_then(Value::as_str) == Some("tool_result") && c.get("is_error").and_then(Value::as_bool) == Some(true))
            .map(|c| {
                let text = c.get("content").map_or_else(String::new, |t| {
                    t.as_str().map_or_else(
                        || t.as_array().into_iter().flatten().filter_map(|p| p.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"),
                        str::to_string,
                    )
                });
                Event::ToolError(clip(&text, MAX_TEXT))
            })
            .collect(),
        Some("result") if v.get("is_error").and_then(Value::as_bool) == Some(true) => {
            let why = v
                .get("result")
                .and_then(Value::as_str)
                .or_else(|| v.get("subtype").and_then(Value::as_str))
                .unwrap_or("Claude Code reported an error");
            usage(&v).into_iter().chain([Event::Error(clip(why, MAX_TEXT))]).collect()
        }
        Some("result") => usage(&v).into_iter().chain([Event::Done]).collect(),
        _ => Vec::new(),
    }
}

/// The tokens a `result` line reports. Input read from or written to the prompt cache is input too.
fn usage(result: &Value) -> Option<Event> {
    let usage = result.get("usage")?;
    let count = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
    let input = count("input_tokens").saturating_add(count("cache_creation_input_tokens")).saturating_add(count("cache_read_input_tokens"));
    Some(Event::Usage { input, output: count("output_tokens") })
}

/// `PATH` with the program's folder first: a launcher script may need what's beside it.
fn search_path(program: &Path) -> Option<std::ffi::OsString> {
    let mut dirs: Vec<PathBuf> = program.parent().map(Path::to_path_buf).into_iter().collect();
    dirs.extend(std::env::var_os("PATH").iter().flat_map(std::env::split_paths));
    std::env::join_paths(dirs).ok()
}

/// Run `prompt` to the end, reporting through `on`. Stopping kills the process.
pub fn run(config: &Config, prompt: &str, on: &mut dyn FnMut(Event), cancel: &Cancel) {
    if let Err(why) = drive(config, prompt, on, cancel) {
        on(Event::Error(why));
    }
}

fn drive(config: &Config, prompt: &str, on: &mut dyn FnMut(Event), cancel: &Cancel) -> Result<(), String> {
    let mut command = Command::new(&config.program);
    command
        .args(args(config))
        .env(bridge::ENV, &config.bridge)
        // Started from inside a Claude Code session, the CLI would refuse to nest.
        .env_remove("CLAUDECODE")
        // Away from any project, so that a project's settings and instructions don't apply.
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(path) = search_path(&config.program) {
        command.env("PATH", path);
    }
    let mut child = command.spawn().map_err(|e| format!("can't start {}: {e}", config.program.display()))?;
    let (Some(mut stdin), Some(stdout), Some(stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("can't talk to Claude Code".into());
    };
    let prompt = prompt.to_string();
    std::thread::spawn(move || {
        // A process that ended early is reported by its exit status.
        let _ = stdin.write_all(prompt.as_bytes());
    });
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.take(MAX_STDERR).read_to_string(&mut text);
        text
    });
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = Vec::new();
        loop {
            line.clear();
            match (&mut reader).take(MAX_LINE).read_until(b'\n', &mut line) {
                Ok(n) if n > 0 && tx.send(String::from_utf8_lossy(&line).into_owned()).is_ok() => {}
                _ => break,
            }
        }
    });
    let mut finished = false;
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("stopped".into());
        }
        match rx.recv_timeout(POLL) {
            Ok(line) => {
                for event in parse_line(&line) {
                    finished |= matches!(event, Event::Done | Event::Error(_));
                    on(event);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let stderr = errors.join().unwrap_or_default();
    if !finished {
        let why = stderr.trim();
        return Err(if why.is_empty() { format!("Claude Code ended without an answer ({status})") } else { clip(why, MAX_TEXT) });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(program: &Path) -> Config {
        Config {
            program: program.to_path_buf(),
            model: "haiku".into(),
            mcp_command: "/app/figcraft".into(),
            mcp_args: vec!["--agent-mcp".into()],
            bridge: "1:k".into(),
            resume: None,
        }
    }

    #[test]
    fn a_chat_resumes_its_session() {
        let mut c = config(Path::new("claude"));
        assert!(!args(&c).contains(&"--resume".to_string()));
        c.resume = Some("s-1".into());
        let a = args(&c);
        assert_eq!(a.iter().position(|x| x == "--resume").map(|i| a[i + 1].as_str()), Some("s-1"));
        assert!(!a.contains(&"--no-session-persistence".to_string()), "a later message has to find the session");
    }

    #[test]
    fn sessions_and_tokens_come_from_the_stream() {
        assert_eq!(parse_line(r#"{"type":"system","subtype":"init","session_id":"s-1"}"#), [Event::Session("s-1".into())]);
        let done = r#"{"type":"result","is_error":false,"usage":{"input_tokens":3,"cache_creation_input_tokens":100,"cache_read_input_tokens":20,"output_tokens":7}}"#;
        assert_eq!(parse_line(done), [Event::Usage { input: 123, output: 7 }, Event::Done]);
        let failed = r#"{"type":"result","is_error":true,"result":"limit","usage":{"input_tokens":1,"output_tokens":2}}"#;
        assert_eq!(parse_line(failed), [Event::Usage { input: 1, output: 2 }, Event::Error("limit".into())]);
    }

    #[test]
    fn the_models_come_from_the_handshake_answer() {
        let answer = |models: &str| {
            format!(
                r#"{{"type":"control_response","response":{{"subtype":"success","request_id":"{MODELS_REQUEST}","response":{{"models":{models}}}}}}}"#
            )
        };
        let listed = parse_models(&answer(
            r#"[{"value":"haiku","displayName":"Haiku 5.5","description":"Fastest"},{"value":"x"},{"displayName":"no value"},3]"#,
        ));
        assert_eq!(
            listed,
            Some(Ok(vec![
                Model { value: "haiku".into(), label: "Haiku 5.5".into(), detail: "Fastest".into() },
                Model { value: "x".into(), label: "x".into(), detail: String::new() },
            ]))
        );
        assert_eq!(parse_models(&answer("3")), Some(Ok(Vec::new())));
        assert_eq!(parse_models(r#"{"type":"system"}"#), None, "other lines come first");
        assert_eq!(parse_models("not json"), None);
        let other = r#"{"type":"control_response","response":{"subtype":"success","request_id":"other"}}"#;
        assert_eq!(parse_models(other), None);
        let refused = format!(r#"{{"type":"control_response","response":{{"subtype":"error","request_id":"{MODELS_REQUEST}","error":"nope"}}}}"#);
        assert!(matches!(parse_models(&refused), Some(Err(e)) if e.contains("nope")));
    }

    #[test]
    fn a_missing_program_lists_no_models_and_says_why() {
        assert!(matches!(list_models(Path::new("/no/such/claude")), Err(e) if e.contains("can't start")));
    }

    #[cfg(unix)]
    #[test]
    fn a_program_is_asked_for_its_models_and_ended() {
        let body = format!(
            r#"read -r request; case "$request" in *initialize*) ;; *) exit 9 ;; esac
echo '{{"type":"system","subtype":"hook_started"}}'
echo '{{"type":"control_response","response":{{"subtype":"success","request_id":"{MODELS_REQUEST}","response":{{"models":[{{"value":"sonnet","displayName":"Sonnet"}}]}}}}}}'
exec sleep 30"#
        );
        let fake = Fake::new("models", &body);
        let since = std::time::Instant::now();
        let models = list_models(&fake.program()).unwrap();
        assert_eq!(models[0].value, "sonnet");
        assert!(since.elapsed() < Duration::from_secs(10), "the program is ended, not waited for");
    }

    #[test]
    fn the_cli_only_gets_figcrafts_tools() {
        let a = args(&config(Path::new("claude")));
        let after = |flag: &str| a.iter().position(|x| x == flag).map(|i| a[i + 1].as_str());
        assert_eq!(after("--tools"), Some(""));
        assert!(a.contains(&"--strict-mcp-config".to_string()));
        assert_eq!(after("--setting-sources"), Some(""), "the person's coding rules stay out of the design chat");
        assert!(after("--system-prompt").unwrap().contains("like Figma"));
        assert_eq!(after("--model"), Some("haiku"));
        let allowed = after("--allowedTools").unwrap();
        assert!(allowed.split(',').all(|t| t.starts_with("mcp__figcraft__")));
        assert!(!allowed.contains("save_file") && !allowed.contains("open_file") && !allowed.contains("type_text"));
        let mcp: Value = serde_json::from_str(after("--mcp-config").unwrap()).unwrap();
        assert_eq!(mcp["mcpServers"]["figcraft"]["args"], json!(["--agent-mcp"]));
        assert!(!a.iter().any(|x| x.contains("1:k")), "the key isn't on the command line");
    }

    #[test]
    fn no_model_means_the_default() {
        let mut c = config(Path::new("claude"));
        c.model = "  ".into();
        assert!(!args(&c).contains(&"--model".to_string()));
    }

    #[test]
    fn stream_lines_become_events() {
        let assistant = r#"{"type":"assistant","message":{"content":[{"type":"text","text":" Hi "},{"type":"tool_use","name":"mcp__figcraft__batch","input":{"label":"Card"}}]}}"#;
        assert_eq!(parse_line(assistant), [Event::Text("Hi".into()), Event::Tool("batch".into(), "Card".into())]);
        let failed = r#"{"type":"user","message":{"content":[{"type":"tool_result","is_error":true,"content":[{"type":"text","text":"refused"}]}]}}"#;
        assert_eq!(parse_line(failed), [Event::ToolError("refused".into())]);
        assert_eq!(parse_line(r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"fine"}]}}"#), []);
        assert_eq!(parse_line(r#"{"type":"result","subtype":"success","is_error":false,"result":"ok"}"#), [Event::Done]);
        assert_eq!(parse_line(r#"{"type":"result","is_error":true,"result":"limit"}"#), [Event::Error("limit".into())]);
        let failed = r#"{"type":"system","subtype":"init","mcp_servers":[{"name":"figcraft","status":"failed"}]}"#;
        assert!(matches!(&parse_line(failed)[0], Event::Error(e) if e.contains("failed")));
        let fine = r#"{"type":"system","subtype":"init","mcp_servers":[{"name":"figcraft","status":"connected"}]}"#;
        assert!(parse_line(fine).is_empty());
    }

    #[test]
    fn garbage_lines_are_ignored() {
        for line in [
            "",
            "not json",
            "3",
            "{}",
            r#"{"type":"assistant"}"#,
            r#"{"type":"assistant","message":{"content":"x"}}"#,
            r#"{"type":"user","message":{"content":[1]}}"#,
        ] {
            assert!(parse_line(line).is_empty(), "{line}");
        }
    }

    #[test]
    fn a_missing_program_is_an_error_not_a_crash() {
        let mut events = Vec::new();
        run(&config(Path::new("/no/such/claude")), "x", &mut |e| events.push(e), &Cancel::default());
        assert!(matches!(&events[0], Event::Error(e) if e.contains("can't start")));
    }

    /// A stand-in `claude` script, in a folder of its own that's removed with this.
    #[cfg(unix)]
    struct Fake(PathBuf);

    #[cfg(unix)]
    impl Fake {
        fn new(name: &str, body: &str) -> Self {
            use std::os::unix::fs::PermissionsExt;
            let dir = std::env::temp_dir().join(format!("figcraft-agent-{name}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("claude");
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            Self(dir)
        }
        fn program(&self) -> PathBuf {
            self.0.join("claude")
        }
    }

    #[cfg(unix)]
    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_prompt_goes_in_and_the_stream_comes_out() {
        let fake = Fake::new(
            "stream",
            r#"read -r p; [ "$FIGCRAFT_AGENT_BRIDGE" = "1:k" ] || exit 9
printf '{"type":"assistant","message":{"content":[{"type":"text","text":"%s"}]}}\n' "$p"
echo '{"type":"result","is_error":false,"result":"ok"}'"#,
        );
        let mut events = Vec::new();
        run(&config(&fake.program()), "--make a card\n", &mut |e| events.push(e), &Cancel::default());
        assert_eq!(events, [Event::Text("--make a card".into()), Event::Done]);
    }

    #[cfg(unix)]
    #[test]
    fn a_failing_process_says_why() {
        let fake = Fake::new("fails", "echo 'not logged in' >&2; exit 1");
        let mut events = Vec::new();
        run(&config(&fake.program()), "x", &mut |e| events.push(e), &Cancel::default());
        assert_eq!(events, [Event::Error("not logged in".into())]);
    }

    #[cfg(unix)]
    #[test]
    fn stopping_kills_the_process() {
        let fake = Fake::new("stop", "exec sleep 30");
        let cancel = Cancel::default();
        cancel.store(true, Ordering::Relaxed);
        let since = std::time::Instant::now();
        let mut events = Vec::new();
        run(&config(&fake.program()), "x", &mut |e| events.push(e), &cancel);
        assert_eq!(events, [Event::Error("stopped".into())]);
        assert!(since.elapsed() < Duration::from_secs(10));
    }
}
