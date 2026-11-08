//! The desktop side of the Agent tab: runs a model in the background, sends its tool calls to the
//! UI thread as control requests, remembers the chat, and keeps its settings and API keys. The
//! general control channel stays off: Claude Code's MCP server reaches a run over a private socket
//! with a key.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use figcraft_agent::{Cancel, Event, Guarded, bridge, claude, openai};
use figcraft_mcp::{Backend, Server};
use figcraft_ui_egui::ControlRequest;
use figcraft_ui_egui::agent_panel::{AgentHost, AgentLine, AgentLineKind, AgentMode, AgentModel, AgentRun, AgentSettings, AgentUsage};
use serde_json::Value;

/// `figcraft --agent-mcp`: serve MCP to the Claude Code process an Agent tab run started.
pub const MCP_FLAG: &str = "--agent-mcp";

const SETTINGS_FILE: &str = "agent.json";
/// The log keeps this many lines.
const MAX_LINES: usize = 2000;
/// How long one tool call may take on the UI thread (a big batch).
const CALL_TIMEOUT: Duration = Duration::from_secs(120);

type Queue = Arc<Mutex<Vec<ControlRequest>>>;
type CtxSlot = Arc<Mutex<Option<egui::Context>>>;

/// Serve MCP on standard input and output; the process's exit code.
pub fn serve_mcp() -> i32 {
    let client = match bridge::Client::from_env() {
        Ok(client) => client,
        Err(why) => {
            eprintln!("FigCraft: {why}");
            return 2;
        }
    };
    let mut server = Server::new(Box::new(client));
    match server.serve(std::io::stdin().lock(), std::io::stdout().lock()) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("FigCraft: {e}");
            1
        }
    }
}

fn wake(ctx: &CtxSlot) {
    if let Some(c) = ctx.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref() {
        c.request_repaint();
    }
}

/// Tool calls to the UI thread, by the same queue the control channel uses.
struct UiBackend {
    queue: Queue,
    ctx: CtxSlot,
}

impl Backend for UiBackend {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        crate::control_server::call_ui(&self.queue, &self.ctx, method, params, CALL_TIMEOUT)
    }

    fn has_ui(&self) -> bool {
        true
    }

    fn describe(&self) -> String {
        "the FigCraft window".into()
    }
}

/// What a background run shares with the panel.
#[derive(Default)]
struct Shared {
    lines: Mutex<Vec<AgentLine>>,
    /// The models each program or endpoint offers.
    models: Mutex<BTreeMap<String, Vec<AgentModel>>>,
    running: AtomicBool,
    usage: Mutex<AgentUsage>,
    /// What the model remembers of the chat: Claude Code's session, an endpoint's messages.
    session: Mutex<Option<String>>,
    history: Mutex<Vec<Value>>,
}

fn locked<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Shared {
    fn push(&self, kind: AgentLineKind, text: String) {
        self.push_line(AgentLine { kind, text, detail: String::new() });
    }

    fn push_line(&self, line: AgentLine) {
        let mut lines = locked(&self.lines);
        if lines.len() >= MAX_LINES {
            lines.remove(0);
        }
        lines.push(line);
    }

    fn event(&self, event: Event) {
        let (kind, text) = match event {
            Event::Text(text) => (AgentLineKind::Text, text),
            Event::Tool(name, detail) => {
                self.push_line(AgentLine { kind: AgentLineKind::Tool, text: name, detail });
                return;
            }
            Event::ToolError(text) | Event::Error(text) => (AgentLineKind::Error, text),
            Event::Done => (AgentLineKind::Done, figcraft_ui_egui::i18n::tr("Done").to_string()),
            Event::Session(id) => {
                *locked(&self.session) = Some(id);
                return;
            }
            Event::Usage { input, output } => {
                let mut usage = locked(&self.usage);
                usage.input = usage.input.saturating_add(input);
                usage.output = usage.output.saturating_add(output);
                return;
            }
        };
        self.push(kind, text);
    }
}

/// Ends a run in the panel however its thread ends.
struct RunEnd(Arc<Shared>, CtxSlot);

impl Drop for RunEnd {
    fn drop(&mut self) {
        self.0.running.store(false, Ordering::Relaxed);
        wake(&self.1);
    }
}

pub struct Host {
    dir: Option<PathBuf>,
    settings: AgentSettings,
    queue: Queue,
    ctx: CtxSlot,
    shared: Arc<Shared>,
    cancel: Cancel,
}

impl Host {
    pub fn new(dir: Option<PathBuf>, queue: Queue, ctx: CtxSlot) -> Self {
        let settings = dir.as_deref().and_then(|d| read_settings(&d.join(SETTINGS_FILE))).unwrap_or_default();
        Self { dir, settings, queue, ctx, shared: Arc::default(), cancel: Cancel::default() }
    }
}

/// The settings in `path`: any problem with the file leaves the defaults.
fn read_settings(path: &Path) -> Option<AgentSettings> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn run_endpoint(
    run: &AgentRun,
    history: &mut Vec<Value>,
    dir: Option<&Path>,
    queue: Queue,
    ctx: CtxSlot,
    on: &mut dyn FnMut(Event),
    cancel: &Cancel,
) {
    let settings = &run.settings;
    let url = settings.url.trim();
    let api_key = settings.keyed.iter().any(|k| k == url).then(|| secret::load(dir, url)).flatten();
    let config = openai::Config { url: url.to_string(), api_key: api_key.clone(), model: settings.model_name().to_string(), vision: settings.vision };
    let mut http = match openai::Http::new(url, api_key) {
        Ok(http) => http,
        Err(why) => return on(Event::Error(why)),
    };
    let mut backend = Guarded::new(UiBackend { queue, ctx }, run.document, config.model.clone(), cancel.clone());
    openai::run(&config, &run.message(), history, &mut backend, &mut http, on, cancel);
}

fn run_claude(run: &AgentRun, resume: Option<String>, queue: Queue, ctx: CtxSlot, on: &mut dyn FnMut(Event), cancel: &Cancel) {
    let started = (|| -> Result<_, String> {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("can't open a local socket: {e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let key = figcraft_collab_net::token::secret()?;
        let exe = std::env::current_exe().map_err(|e| format!("can't find FigCraft's own program: {e}"))?;
        Ok((listener, port, key, exe))
    })();
    let (listener, port, key, exe) = match started {
        Ok(started) => started,
        Err(why) => return on(Event::Error(why)),
    };
    let config = claude::Config {
        program: PathBuf::from(run.settings.claude_path.trim()),
        model: run.settings.model_name().to_string(),
        mcp_command: exe,
        mcp_args: vec![MCP_FLAG.to_string()],
        bridge: format!("{port}:{key}"),
        resume,
    };
    let mut backend = Guarded::new(UiBackend { queue, ctx }, run.document, "Claude Code", cancel.clone());
    std::thread::scope(|scope| {
        let serving = scope.spawn(|| {
            if let Err(e) = bridge::serve(&listener, &key, &mut backend, cancel) {
                log::warn!("agent socket: {e}");
            }
        });
        claude::run(&config, &run.message(), on, cancel);
        // The process is gone: its socket closes with the run.
        cancel.store(true, Ordering::Relaxed);
        let _ = serving.join();
    });
}

impl AgentHost for Host {
    fn settings(&self) -> AgentSettings {
        self.settings.clone()
    }

    fn save_settings(&mut self, settings: &AgentSettings) {
        self.settings = settings.clone();
        let Some(dir) = &self.dir else { return };
        let tmp = dir.join(format!("{SETTINGS_FILE}.tmp"));
        let written = serde_json::to_vec(settings)
            .map_err(std::io::Error::other)
            .and_then(|bytes| {
                std::fs::create_dir_all(dir)?;
                crate::services::write_private(&tmp, &bytes)
            })
            .and_then(|()| std::fs::rename(&tmp, dir.join(SETTINGS_FILE)));
        if let Err(e) = written {
            log::warn!("agent settings: couldn't save them in {}: {e}", dir.display());
        }
    }

    fn set_key(&mut self, url: &str, key: &str) -> Result<(), String> {
        secret::save(self.dir.as_deref(), url, key)
    }

    fn start(&mut self, run: AgentRun) -> Result<(), String> {
        if self.shared.running.swap(true, Ordering::Relaxed) {
            return Err("a run is going on".to_string());
        }
        self.cancel = Cancel::default();
        self.shared.push(AgentLineKind::Prompt, run.prompt.clone());
        let (shared, cancel, queue, ctx, dir) = (self.shared.clone(), self.cancel.clone(), self.queue.clone(), self.ctx.clone(), self.dir.clone());
        let spawned = std::thread::Builder::new().name("figcraft-agent".into()).spawn(move || {
            let _end = RunEnd(shared.clone(), ctx.clone());
            let wake_ctx = ctx.clone();
            let mut on = |event: Event| {
                shared.event(event);
                wake(&wake_ctx);
            };
            match run.settings.mode {
                AgentMode::ClaudeCode => {
                    let resume = locked(&shared.session).clone();
                    run_claude(&run, resume, queue, ctx, &mut on, &cancel);
                }
                AgentMode::Endpoint => {
                    let mut history = std::mem::take(&mut *locked(&shared.history));
                    run_endpoint(&run, &mut history, dir.as_deref(), queue, ctx, &mut on, &cancel);
                    *locked(&shared.history) = history;
                }
            }
        });
        spawned.map(|_| ()).map_err(|e| {
            self.shared.running.store(false, Ordering::Relaxed);
            e.to_string()
        })
    }

    fn stop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    fn running(&self) -> bool {
        self.shared.running.load(Ordering::Relaxed)
    }

    fn lines(&self) -> Vec<AgentLine> {
        locked(&self.shared.lines).clone()
    }

    fn usage(&self) -> AgentUsage {
        *locked(&self.shared.usage)
    }

    fn clear(&mut self) {
        locked(&self.shared.lines).clear();
        locked(&self.shared.history).clear();
        *locked(&self.shared.session) = None;
        *locked(&self.shared.usage) = AgentUsage::default();
    }

    fn load_models(&mut self, settings: &AgentSettings) {
        let (shared, ctx, dir, settings) = (self.shared.clone(), self.ctx.clone(), self.dir.clone(), settings.clone());
        let spawned = std::thread::Builder::new().name("figcraft-agent-models".into()).spawn(move || {
            let source = settings.source().to_string();
            let listed = match settings.mode {
                AgentMode::ClaudeCode => claude::list_models(Path::new(&source)),
                AgentMode::Endpoint => {
                    let key = settings.keyed.contains(&source).then(|| secret::load(dir.as_deref(), &source)).flatten();
                    openai::Http::new(&source, key)
                        .and_then(|http| http.models())
                        .map(|ids| ids.into_iter().map(|id| figcraft_agent::Model { label: id.clone(), value: id, detail: String::new() }).collect())
                }
            };
            match listed {
                Ok(models) => {
                    let models = models.into_iter().map(|m| AgentModel { value: m.value, label: m.label, detail: m.detail }).collect();
                    locked(&shared.models).insert(source, models);
                }
                Err(why) => shared.push(AgentLineKind::Error, why),
            }
            wake(&ctx);
        });
        if let Err(e) = spawned {
            self.shared.push(AgentLineKind::Error, e.to_string());
        }
    }

    fn models(&self, settings: &AgentSettings) -> Vec<AgentModel> {
        locked(&self.shared.models).get(settings.source()).cloned().unwrap_or_default()
    }

    fn detect_claude(&self) -> Option<String> {
        claude::detect().map(|p| p.to_string_lossy().into_owned())
    }
}

/// API keys: the system's keychain on macOS and Windows. Elsewhere a file only its owner can read,
/// beside the preferences: the Linux and BSD secret stores need a session service many setups lack.
mod secret {
    use std::path::Path;

    #[cfg(any(target_os = "macos", windows))]
    const SERVICE: &str = "FigCraft Agent";

    #[cfg(any(target_os = "macos", windows))]
    pub fn save(_: Option<&Path>, url: &str, key: &str) -> Result<(), String> {
        keyring::Entry::new(SERVICE, url).and_then(|entry| entry.set_password(key)).map_err(|e| format!("couldn't save the key: {e}"))
    }

    #[cfg(any(target_os = "macos", windows))]
    pub fn load(_: Option<&Path>, url: &str) -> Option<String> {
        keyring::Entry::new(SERVICE, url).and_then(|entry| entry.get_password()).map_err(|e| log::warn!("agent: couldn't read the key: {e}")).ok()
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    const FILE: &str = "agent-keys.json";

    #[cfg(not(any(target_os = "macos", windows)))]
    pub fn save(dir: Option<&Path>, url: &str, key: &str) -> Result<(), String> {
        let dir = dir.ok_or("there is no settings folder to keep the key in")?;
        let mut keys: serde_json::Map<String, serde_json::Value> =
            std::fs::read_to_string(dir.join(FILE)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default();
        keys.insert(url.to_string(), key.into());
        let tmp = dir.join(format!("{FILE}.tmp"));
        std::fs::create_dir_all(dir)
            .and_then(|()| crate::services::write_private(&tmp, serde_json::Value::Object(keys).to_string().as_bytes()))
            .and_then(|()| std::fs::rename(&tmp, dir.join(FILE)))
            .map_err(|e| format!("couldn't save the key: {e}"))
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    pub fn load(dir: Option<&Path>, url: &str) -> Option<String> {
        let text = std::fs::read_to_string(dir?.join(FILE)).ok()?;
        serde_json::from_str::<serde_json::Value>(&text).ok()?.get(url)?.as_str().map(str::to_string)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn host(dir: Option<PathBuf>) -> Host {
        Host::new(dir, Queue::default(), CtxSlot::default())
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("figcraft-agent-host-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// Answers the queue as the UI thread does, until told to stop.
    fn answer_queue(queue: Queue, done: Arc<AtomicBool>, seen: Arc<Mutex<Vec<Value>>>) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                let taken: Vec<ControlRequest> = queue.lock().unwrap().drain(..).collect();
                for request in taken {
                    assert!(request.state.start());
                    seen.lock().unwrap().push(json!({"method": request.method, "params": request.params}));
                    let _ = request.reply.send(figcraft_ui_egui::ControlReply { id: request.id, result: Ok(json!({"ok": true})) });
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        })
    }

    fn wait_for_end(host: &Host) {
        let since = std::time::Instant::now();
        while host.running() && since.elapsed() < Duration::from_secs(20) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!host.running(), "the run ended");
    }

    #[test]
    fn settings_survive_a_restart_and_a_broken_file_leaves_the_defaults() {
        let dir = temp("settings");
        let mut h = host(Some(dir.clone()));
        let saved = AgentSettings { url: "http://localhost:1/v1".into(), model: "m".into(), ..Default::default() };
        h.save_settings(&saved);
        assert_eq!(host(Some(dir.clone())).settings(), saved);
        std::fs::write(dir.join(SETTINGS_FILE), "{broken").unwrap();
        assert_eq!(host(Some(dir.clone())).settings(), AgentSettings::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_chat_remembers_and_counts_until_a_new_chat() {
        let mut h = host(None);
        h.shared.event(Event::Session("s-1".into()));
        h.shared.event(Event::Usage { input: 10, output: 2 });
        h.shared.event(Event::Usage { input: 5, output: 1 });
        h.shared.event(Event::Text("hello".into()));
        locked(&h.shared.history).push(json!({"role": "system"}));
        assert_eq!(h.usage(), AgentUsage { input: 15, output: 3 });
        assert_eq!(h.lines().len(), 1, "sessions and counts aren't chat lines");
        assert_eq!(locked(&h.shared.session).as_deref(), Some("s-1"));
        h.clear();
        assert_eq!((h.usage(), h.lines().len()), (AgentUsage::default(), 0));
        assert!(locked(&h.shared.session).is_none() && locked(&h.shared.history).is_empty(), "the model forgets too");
    }

    #[test]
    fn an_endpoint_that_cant_be_used_ends_the_run_with_a_reason() {
        let mut h = host(None);
        let settings = AgentSettings { mode: AgentMode::Endpoint, url: "http://example.com/v1".into(), model: "m".into(), ..Default::default() };
        h.start(AgentRun { settings, prompt: "x".into(), context: String::new(), document: None }).unwrap();
        wait_for_end(&h);
        let lines = h.lines();
        assert_eq!(lines[0].kind, AgentLineKind::Prompt);
        assert!(matches!(lines.last(), Some(AgentLine { kind: AgentLineKind::Error, text, .. }) if text.contains("http")), "{lines:?}");
    }

    /// The Claude Code path without the model: a stand-in CLI hands over the socket's address, this
    /// test speaks to it as the MCP child does, and the calls come out of the queue guarded.
    #[cfg(unix)]
    #[test]
    fn a_claude_run_reaches_the_ui_only_through_the_guard() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp("claude");
        std::fs::create_dir_all(&dir).unwrap();
        let program = dir.join("claude");
        std::fs::write(&program, "#!/bin/sh\nprintf '%s' \"$FIGCRAFT_AGENT_BRIDGE\" > \"$(dirname \"$0\")/bridge\"\nwhile [ ! -e \"$(dirname \"$0\")/go\" ]; do sleep 0.05; done\necho '{\"type\":\"result\",\"is_error\":false,\"result\":\"ok\"}'\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut h = host(None);
        let (done, seen) = (Arc::new(AtomicBool::new(false)), Arc::new(Mutex::new(Vec::new())));
        let ui = answer_queue(h.queue.clone(), done.clone(), seen.clone());
        let settings = AgentSettings { claude_path: program.to_string_lossy().into_owned(), ..Default::default() };
        h.start(AgentRun { settings, prompt: "x".into(), context: String::new(), document: Some(7) }).unwrap();
        let since = std::time::Instant::now();
        let address = loop {
            if let Ok(text) = std::fs::read_to_string(dir.join("bridge"))
                && text.contains(':')
            {
                break text;
            }
            assert!(since.elapsed() < Duration::from_secs(20), "the CLI got the bridge's address");
            std::thread::sleep(Duration::from_millis(10));
        };
        let (port, key) = address.split_once(':').unwrap();
        let mut client = bridge::Client::connect(port.parse().unwrap(), key).unwrap();
        assert!(client.call("engine.execute", json!({"command": "shape.rectangle", "params": {}})).is_ok());
        assert!(client.call("engine.execute", json!({"command": "file.open", "params": {"path": "/etc/passwd"}})).is_err());
        assert!(client.call("app.quit", json!({})).is_err());
        std::fs::write(dir.join("go"), "").unwrap();
        wait_for_end(&h);
        done.store(true, Ordering::Relaxed);
        ui.join().unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "only the allowed call reached the UI: {seen:?}");
        assert_eq!(seen[0]["params"]["agentScope"], json!(true));
        assert_eq!(seen[0]["params"]["document"], json!(7));
        assert_eq!(h.lines().last().map(|l| l.kind), Some(AgentLineKind::Done));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
