//! `figcraft-cli`: FigCraft without a window. Inspect, export, convert and render documents, run
//! command scripts, list commands, serve MCP on stdio, publish plugins to a server's plugin
//! directory, publish code connect mappings to a design file and measure performance.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod args;
mod files;
mod perf;

use std::io::Write;
use std::process::ExitCode;

use args::{Cli, Command, ConnectCommand, PluginCommand, RootArgs};
use serde_json::{Value, json};

/// The automation roots a file subcommand was given, if any. Omitting both leaves the run
/// unconfined, as before; giving either confines every file the subcommand touches (#833).
fn roots_of(r: &RootArgs) -> CliResult<Option<figcraft_engine::file_access::Roots>> {
    if r.automation_read_root.is_empty() && r.automation_write_root.is_empty() {
        return Ok(None);
    }
    Ok(Some(figcraft_engine::file_access::Roots::new(&r.automation_read_root, &r.automation_write_root).map_err(CliError::Failed)?))
}

/// Runs `f` confined to the roots `r` carries.
fn confined<R>(r: &RootArgs, f: impl FnOnce() -> CliResult<R>) -> CliResult<R> {
    let roots = roots_of(r)?.map(std::sync::Arc::new);
    figcraft_engine::file_access::confined(roots.as_ref(), f)
}

fn main() -> ExitCode {
    // `args_os`: a non-Unicode argument is clap's usage error, not a panic.
    let cli = match args::parse(std::env::args_os().collect()) {
        Ok(cli) => cli,
        Err(e) => {
            // Help and `--version` on stdout with 0, usage errors on stderr with 2.
            let _ = e.print();
            return ExitCode::from(u8::try_from(e.exit_code()).unwrap_or(2));
        }
    };
    match run(cli) {
        Ok(code) => code,
        Err(CliError::Usage(msg)) => {
            eprintln!("error: {msg}\n\nFor more information, try '--help'.");
            ExitCode::from(2)
        }
        Err(CliError::Failed(msg)) => {
            eprintln!("error: {msg}");
            ExitCode::from(1)
        }
    }
}

#[derive(Debug)]
pub enum CliError {
    /// Bad arguments found after parsing (an output extension no format writes): exit code 2.
    Usage(String),
    /// The operation failed: exit code 1.
    Failed(String),
}

impl From<String> for CliError {
    fn from(s: String) -> Self {
        CliError::Failed(s)
    }
}

pub type CliResult<T> = Result<T, CliError>;

fn run(cli: Cli) -> CliResult<ExitCode> {
    if !cli.no_system_fonts {
        // Text lays out with the installed fonts, like the desktop app, so headless renders and
        // auto layout sizes match what users see.
        figcraft_text::scan_system_fonts_on_demand();
    } else {
        // Nothing scans them then, not even a `.sketch` import matching its fonts.
        figcraft_text::opt_out_of_system_fonts();
    }
    match cli.command {
        Command::Info(a) => confined(&a.roots, || files::info(&a.file, a.json)),
        Command::Export(a) => {
            let target =
                files::Target { print: a.print(), vd_compat: a.vd_compat, scale: a.scale.scale, format: a.format, page: a.page, node: a.node };
            confined(&a.roots, || files::export(&a.file, &a.out, &target))
        }
        Command::Convert(a) => confined(&a.roots, || files::convert(&a.input, &a.output)),
        Command::Render(a) => confined(&a.roots, || files::render(&a.file, &a.out, a.scale.scale, a.page.as_deref())),
        Command::Run(a) => confined(&a.roots, || files::run_script(&a.script, a.input.as_deref(), a.out.as_deref())),
        Command::Commands(a) => {
            commands(a.json, a.filter.as_deref());
            Ok(ExitCode::SUCCESS)
        }
        // `--attach` conflicts with the roots: the app runs the commands, only its own roots can confine them.
        Command::Mcp(a) => mcp(a.attach.as_deref(), a.timeout, a.agent.as_deref(), roots_of(&a.roots)?),
        Command::Call(a) => call(&a),
        Command::Plugin { command: PluginCommand::Publish(a) } => publish_plugin(&a.folder, &a.server),
        Command::Connect { command: ConnectCommand::Publish(a) } => confined(&a.roots, || files::connect_publish(&a.file, &a.mappings, a.dry_run)),
        Command::Perf(a) => {
            let sizes: Vec<usize> = a.layers.into_iter().map(|n| n.clamp(10, 2_000_000)).collect();
            let instances: Vec<usize> = a.instances.into_iter().filter(|n| *n > 0).map(|n| n.min(100_000)).collect();
            perf::run(&sizes, &instances, !cli.no_system_fonts)
        }
    }
}

fn commands(as_json: bool, filter: Option<&str>) {
    let f = filter.map(str::to_lowercase).unwrap_or_default();
    let list: Vec<&figcraft_engine::CommandSpec> = figcraft_engine::all_commands()
        .iter()
        .filter(|c| f.is_empty() || c.id.to_lowercase().contains(&f) || c.label.to_lowercase().contains(&f) || c.params.to_lowercase().contains(&f))
        .collect();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    if as_json {
        let v: Vec<Value> = list
            .iter()
            .map(|c| json!({"id": c.id, "label": c.label, "params": c.params, "menu": c.menu(), "shortcut": c.shortcut(), "undoable": c.history}))
            .collect();
        let _ = writeln!(out, "{}", serde_json::to_string_pretty(&v).unwrap_or_default());
        return;
    }
    for c in list {
        let sc = c.shortcut().map(|s| format!(" ({s})")).unwrap_or_default();
        let undo = if c.history { "*" } else { "" };
        let _ = writeln!(out, "{}{undo}  {}{sc}\n    {}", c.id, c.label, c.params);
    }
}

/// Publish the plugin folder `folder` to the plugin directory at `server` (the key from the address
/// or `FIGCRAFT_SERVER_KEY`); never prints the key.
fn publish_plugin(folder: &str, server: &str) -> CliResult<ExitCode> {
    let address = server.split('?').next().unwrap_or_default().to_string();
    let server = match std::env::var("FIGCRAFT_SERVER_KEY") {
        Ok(key) if !server.contains("key=") && !key.trim().is_empty() => format!("{address}?key={}", key.trim()),
        _ => server.to_string(),
    };
    let mut h = figcraft_mcp::Headless::new();
    use figcraft_mcp::Backend as _;
    let r = h.call("engine.execute", json!({"command": "plugin.directory.publish", "params": {"path": folder, "server": server}}))?;
    let e = &r["published"];
    println!(
        "Published {} {} ({}) to {address}: {} bytes, SHA-256 {}",
        e["name"].as_str().unwrap_or_default(),
        e["version"].as_str().unwrap_or_default(),
        e["id"].as_str().unwrap_or_default(),
        e["bytes"],
        e["sha256"].as_str().unwrap_or_default()
    );
    Ok(ExitCode::SUCCESS)
}

/// `--attach auto`: the address of the app that lets AI agents in, from the state file it keeps
/// in its settings folder while it listens.
fn running_app() -> CliResult<String> {
    use figcraft_engine::discovery;
    let dirs = discovery::search_dirs();
    let Some((port, _)) = dirs.iter().find_map(|dir| discovery::read(dir)) else {
        return Err(no_running_app(&dirs).into());
    };
    Ok(std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, port)).to_string())
}

/// Why `--attach auto` found no app: the folders searched for its state file, and what to do.
fn no_running_app(dirs: &[std::path::PathBuf]) -> String {
    use figcraft_engine::discovery::{CONFIG_DIR_VAR, STATE_FILE};
    let searched = if dirs.is_empty() {
        format!("{CONFIG_DIR_VAR} is set but empty, so no folder was searched")
    } else {
        let dirs: Vec<String> = dirs.iter().map(|d| d.display().to_string()).collect();
        format!("no readable {STATE_FILE} in {}", dirs.join(" or in "))
    };
    format!(
        "no FigCraft app lets AI agents in ({searched}): turn on Preferences ▸ Allow AI agents to control FigCraft (or start it with `figcraft --control PORT`); if the app keeps its settings in another folder, name it with {CONFIG_DIR_VAR}"
    )
}

/// A connection to the running app `attach` names (`auto`: the one that lets AI agents in),
/// which may take `timeout` seconds over each request.
fn attached(attach: &str, timeout: Option<u64>) -> CliResult<figcraft_mcp::Attached> {
    let addr = if attach == args::ATTACH_AUTO { running_app()? } else { attach.to_owned() };
    let no_app = |e: std::io::Error| format!("can't reach the FigCraft app at {addr}: {e} (start it with `figcraft --control PORT`)");
    let app = figcraft_mcp::Attached::connect(&addr).map_err(no_app)?;
    Ok(match timeout {
        Some(secs) => app.with_timeout(std::time::Duration::from_secs(secs)).map_err(no_app)?,
        None => app,
    })
}

/// The most text `call` reads as parameters from standard input.
const MAX_CALL_PARAMS: u64 = 16 << 20;

/// `call`: one request to the running app, named for `--agent`; its result on standard output.
fn call(a: &args::CallArgs) -> CliResult<ExitCode> {
    use figcraft_mcp::Backend;
    use std::io::Read;
    let text = if a.params == "-" {
        let mut text = String::new();
        std::io::stdin().lock().take(MAX_CALL_PARAMS).read_to_string(&mut text).map_err(|e| format!("stdin: {e}"))?;
        text
    } else {
        a.params.clone()
    };
    let mut params: Value = serde_json::from_str(&text).map_err(|e| CliError::Usage(format!("the parameters are not JSON: {e}")))?;
    if !params.is_object() {
        return Err(CliError::Usage("the parameters must be a JSON object, like {\"command\":\"select.all\"}".into()));
    }
    if let Some(agent) = a.agent.as_deref() {
        figcraft_mcp::name_agent(&a.method, &mut params, agent);
    }
    let result = attached(&a.attach, a.timeout)?.call(&a.method, params)?;
    writeln!(std::io::stdout().lock(), "{result}").map_err(|e| format!("stdout: {e}"))?;
    Ok(ExitCode::SUCCESS)
}

fn mcp(attach: Option<&str>, timeout: Option<u64>, agent: Option<&str>, roots: Option<figcraft_engine::file_access::Roots>) -> CliResult<ExitCode> {
    let backend: Box<dyn figcraft_mcp::Backend> = match attach {
        Some(app) => Box::new(attached(app, timeout)?),
        None => Box::new(figcraft_mcp::Headless::new()),
    };
    eprintln!("figcraft MCP server ({}) on stdio", backend.describe());
    if let Some(r) = &roots {
        eprintln!("file access confined to {}", r.to_json());
    }
    let mut server = figcraft_mcp::Server::new(backend).with_roots(roots).with_agent(agent.unwrap_or_default());
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    server.serve(stdin.lock(), stdout.lock()).map_err(|e| format!("stdio: {e}"))?;
    Ok(ExitCode::SUCCESS)
}
