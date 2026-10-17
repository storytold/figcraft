//! Preferences ▸ AI agents, the desktop side: the control server MCP clients reach the app through,
//! and FigCraft's entry in the clients' settings (Claude Code, Claude Desktop, Cursor).
//!
//! A client runs `<this app> mcp --attach PORT` (main.rs), the MCP server attached to this window.
//! Claude Code is set up through its own CLI (`claude mcp add`), never by editing `~/.claude.json`,
//! which a running Claude Code rewrites; Claude Desktop and Cursor keep their servers in a JSON file
//! that only gains or loses FigCraft's entry, everything else as it was.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use figcraft_ui_egui::ControlRequest;
use figcraft_ui_egui::agents::{AgentHost, Client, ClientState, ServerStatus};
use serde_json::{Map, Value, json};

use crate::control_server::ControlServer;

type Queue = Arc<Mutex<Vec<ControlRequest>>>;
type CtxSlot = Arc<Mutex<Option<egui::Context>>>;

/// The server's name in the clients' settings.
pub const SERVER_NAME: &str = "figcraft";
/// How long `claude mcp …` may take before the install gives up.
const CLI_TIMEOUT: Duration = Duration::from_secs(20);
/// Ports [`DesktopAgents::free_port`] tries.
const PORT_SEARCH: u16 = 50;

pub struct DesktopAgents {
    queue: Queue,
    ctx: CtxSlot,
    server: Option<ControlServer>,
    error: Option<(u16, String)>,
    /// Where the clients' settings and tools live (the home folder; tests use a scratch one).
    home: Option<PathBuf>,
    /// This app's executable, which the clients run as `… mcp --attach PORT`.
    exe: PathBuf,
}

impl DesktopAgents {
    pub fn new(queue: Queue, ctx: CtxSlot) -> Self {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("figcraft"));
        Self { queue, ctx, server: None, error: None, home, exe }
    }

    #[cfg(test)]
    fn with_home(home: &Path, exe: &Path) -> Self {
        Self { queue: Arc::default(), ctx: Arc::default(), server: None, error: None, home: Some(home.to_path_buf()), exe: exe.to_path_buf() }
    }

    /// The command a client runs to reach this app on `port`.
    fn entry(&self, port: u16) -> Value {
        json!({"command": self.exe.to_string_lossy(), "args": ["mcp", "--attach", port.to_string()]})
    }

    fn json_config(&self, id: &str) -> Option<PathBuf> {
        let home = self.home.as_deref()?;
        match id {
            "claude-desktop" => Some(claude_desktop_dir(home)?.join("claude_desktop_config.json")),
            "cursor" => Some(home.join(".cursor").join("mcp.json")),
            _ => None,
        }
    }

    fn claude_code(&self, port: u16) -> Client {
        let bin = self.home.as_deref().and_then(find_claude);
        let installed = self.home.as_deref().and_then(|h| read_json(&h.join(".claude.json")).ok()).and_then(|v| entry_port(&v));
        let state = match (installed, &bin) {
            (Some(p), _) => ClientState::Installed { same_port: p == port },
            (None, Some(_)) => ClientState::Available,
            (None, None) => ClientState::Missing,
        };
        let detail = bin.map(|b| tilde(self.home.as_deref(), &b)).unwrap_or_default();
        Client { id: "claude-code", name: "Claude Code", state, detail }
    }

    fn json_client(&self, id: &'static str, name: &'static str, port: u16) -> Client {
        let Some(path) = self.json_config(id) else {
            return Client { id, name, state: ClientState::Missing, detail: String::new() };
        };
        // The client's folder exists once it's installed and has run.
        let present = path.parent().is_some_and(Path::is_dir);
        let state = match read_json(&path).ok().and_then(|v| entry_port(&v)) {
            Some(p) => ClientState::Installed { same_port: p == port },
            None if present => ClientState::Available,
            None => ClientState::Missing,
        };
        Client { id, name, state, detail: tilde(self.home.as_deref(), &path) }
    }

    fn claude_cli(&self, args: &[&str]) -> Result<String, String> {
        let bin = self.home.as_deref().and_then(find_claude).ok_or("Claude Code's `claude` command wasn't found")?;
        run(Command::new(bin).args(args))
    }
}

impl AgentHost for DesktopAgents {
    fn status(&self) -> ServerStatus {
        ServerStatus {
            listening: self.server.as_ref().map(ControlServer::port),
            connections: self.server.as_ref().map_or(0, ControlServer::connections),
            error: self.error.as_ref().map(|(_, e)| e.clone()),
            error_port: self.error.as_ref().map(|(p, _)| *p),
        }
    }

    fn start(&mut self, port: u16) -> Result<(), String> {
        if self.server.as_ref().is_some_and(|s| s.port() == port) {
            return Ok(());
        }
        // The new port first: when it's taken, the agents connected on the old one stay connected.
        match ControlServer::start(port, self.queue.clone(), self.ctx.clone()) {
            Ok(s) => {
                if let Some(old) = self.server.replace(s) {
                    old.stop();
                }
                self.error = None;
                Ok(())
            }
            Err(e) => {
                log::error!("control server: can't listen on 127.0.0.1:{port}: {e}");
                self.error = Some((port, e.to_string()));
                Err(e.to_string())
            }
        }
    }

    fn stop(&mut self) {
        if let Some(s) = self.server.take() {
            s.stop();
        }
        self.error = None;
    }

    fn disconnect(&mut self) {
        if let Some(s) = &self.server {
            s.disconnect();
        }
    }

    fn free_port(&self, from: u16) -> Option<u16> {
        (from..from.saturating_add(PORT_SEARCH)).find(|p| std::net::TcpListener::bind(("127.0.0.1", *p)).is_ok())
    }

    fn clients(&mut self, port: u16) -> Vec<Client> {
        vec![self.claude_code(port), self.json_client("claude-desktop", "Claude Desktop", port), self.json_client("cursor", "Cursor", port)]
    }

    fn install(&mut self, id: &str, port: u16) -> Result<String, String> {
        match id {
            "claude-code" => {
                // A FigCraft entry with another port goes first (`add` refuses an existing name).
                let _ = self.claude_cli(&["mcp", "remove", SERVER_NAME, "-s", "user"]);
                let exe = self.exe.to_string_lossy().into_owned();
                let port = port.to_string();
                self.claude_cli(&["mcp", "add", SERVER_NAME, "-s", "user", "--", &exe, "mcp", "--attach", &port])?;
                Ok("FigCraft is installed in Claude Code: start a new Claude Code session to use it".into())
            }
            "claude-desktop" | "cursor" => {
                let path = self.json_config(id).ok_or("the settings folder wasn't found")?;
                let current = match read_json(&path) {
                    Ok(v) => Some(v),
                    Err(ReadError::Missing) => None,
                    Err(ReadError::Invalid(e)) => return Err(format!("{} isn't valid JSON ({e}): fix or remove it first", path.display())),
                };
                write_json(&path, &with_entry(current, self.entry(port)))?;
                Ok(if id == "cursor" {
                    "FigCraft is installed in Cursor: it shows under Settings ▸ MCP".into()
                } else {
                    "FigCraft is installed in Claude Desktop: quit and reopen Claude Desktop to load it".into()
                })
            }
            _ => Err(format!("unknown client `{id}`")),
        }
    }

    fn uninstall(&mut self, id: &str) -> Result<(), String> {
        match id {
            "claude-code" => self.claude_cli(&["mcp", "remove", SERVER_NAME, "-s", "user"]).map(|_| ()),
            "claude-desktop" | "cursor" => {
                let path = self.json_config(id).ok_or("the settings folder wasn't found")?;
                match read_json(&path) {
                    Ok(v) => write_json(&path, &without_entry(v)),
                    Err(ReadError::Missing) => Ok(()),
                    Err(ReadError::Invalid(e)) => Err(format!("{} isn't valid JSON ({e})", path.display())),
                }
            }
            _ => Err(format!("unknown client `{id}`")),
        }
    }

    fn config_json(&self, port: u16) -> String {
        serde_json::to_string_pretty(&json!({"mcpServers": {SERVER_NAME: self.entry(port)}})).unwrap_or_default()
    }
}

/// Claude Desktop's settings folder on this platform.
fn claude_desktop_dir(home: &Path) -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        Some(home.join("Library/Application Support/Claude"))
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("Claude"))
    } else {
        Some(std::env::var_os("XDG_CONFIG_HOME").map_or_else(|| home.join(".config"), PathBuf::from).join("Claude"))
    }
}

/// Claude Code's `claude` command: on the PATH, or where its installers put it (an app started
/// from the Finder or the Dock has a short PATH).
fn find_claude(home: &Path) -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) { &["claude.exe", "claude.cmd"] } else { &["claude"] };
    let path_dirs = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default();
    let usual = [
        home.join(".local/bin"),
        home.join(".claude/local"),
        home.join(".npm-global/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ];
    path_dirs.iter().chain(usual.iter()).flat_map(|d| names.iter().map(move |n| d.join(n))).find(|p| p.is_file())
}

/// `~/…` for paths in the home folder, as people read them.
fn tilde(home: Option<&Path>, p: &Path) -> String {
    match home.and_then(|h| p.strip_prefix(h).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => p.display().to_string(),
    }
}

#[derive(Debug)]
pub enum ReadError {
    Missing,
    Invalid(String),
}

/// Settings files bigger than this aren't settings.
const MAX_SETTINGS: u64 = 16 * 1024 * 1024;

fn read_json(path: &Path) -> Result<Value, ReadError> {
    let meta = std::fs::metadata(path).map_err(|_| ReadError::Missing)?;
    if meta.len() > MAX_SETTINGS {
        return Err(ReadError::Invalid("too large".into()));
    }
    let text = std::fs::read_to_string(path).map_err(|e| ReadError::Invalid(e.to_string()))?;
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(&text).map_err(|e| ReadError::Invalid(e.to_string()))
}

/// Write `v` next to `path` and move it into place, so a crash never leaves half a file.
fn write_json(path: &Path, v: &Value) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension("json.figcraft-tmp");
    let text = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, text + "\n").map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })
}

/// `settings` with FigCraft's server entry set, every other key kept.
pub fn with_entry(settings: Option<Value>, entry: Value) -> Value {
    let mut root = match settings {
        Some(Value::Object(o)) => o,
        _ => Map::new(),
    };
    let servers = root.entry("mcpServers").or_insert_with(|| json!({}));
    if !servers.is_object() {
        *servers = json!({});
    }
    if let Some(s) = servers.as_object_mut() {
        s.insert(SERVER_NAME.into(), entry);
    }
    Value::Object(root)
}

/// `settings` without FigCraft's server entry, every other key kept.
pub fn without_entry(mut settings: Value) -> Value {
    if let Some(s) = settings.get_mut("mcpServers").and_then(Value::as_object_mut) {
        s.remove(SERVER_NAME);
    }
    settings
}

/// The port FigCraft's entry in `settings` attaches to, if it has one.
pub fn entry_port(settings: &Value) -> Option<u16> {
    let args = settings.get("mcpServers")?.get(SERVER_NAME)?.get("args")?.as_array()?;
    let i = args.iter().position(|a| a == "--attach")?;
    let port = args.get(i + 1)?;
    port.as_str().and_then(|p| p.rsplit(':').next()?.parse().ok()).or_else(|| port.as_u64().and_then(|p| u16::try_from(p).ok()))
}

/// Run a command, its output as the error when it fails, giving up after [`CLI_TIMEOUT`].
fn run(cmd: &mut Command) -> Result<String, String> {
    let mut child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > CLI_TIMEOUT => {
                let _ = child.kill();
                return Err(format!("no answer after {} s", CLI_TIMEOUT.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(e) => return Err(e.to_string()),
        }
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let text = |b: &[u8]| String::from_utf8_lossy(b).trim().to_string();
    if out.status.success() {
        Ok(text(&out.stdout))
    } else {
        Err(Some(text(&out.stderr)).filter(|s| !s.is_empty()).unwrap_or_else(|| text(&out.stdout)))
    }
}

#[cfg(test)]
#[path = "agents_tests.rs"]
mod tests;
