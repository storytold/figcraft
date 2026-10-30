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
        Command::Mcp(a) => mcp(a.attach.as_deref(), roots_of(&a.roots)?),
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

fn mcp(attach: Option<&str>, roots: Option<figcraft_engine::file_access::Roots>) -> CliResult<ExitCode> {
    let backend: Box<dyn figcraft_mcp::Backend> = match attach {
        Some(addr) => {
            let b = figcraft_mcp::Attached::connect(addr)
                .map_err(|e| format!("can't reach the FigCraft app at {addr}: {e} (start it with `figcraft --control PORT`)"))?;
            Box::new(b)
        }
        None => Box::new(figcraft_mcp::Headless::new()),
    };
    eprintln!("figcraft MCP server ({}) on stdio", backend.describe());
    if let Some(r) = &roots {
        eprintln!("file access confined to {}", r.to_json());
    }
    let mut server = figcraft_mcp::Server::new(backend).with_roots(roots);
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    server.serve(stdin.lock(), stdout.lock()).map_err(|e| format!("stdio: {e}"))?;
    Ok(ExitCode::SUCCESS)
}
