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
        Command::Mcp(a) => {
            if a.status || a.install.is_some() {
                return mcp_harness(a.status, a.install.as_deref(), a.port);
            }
            mcp(a.attach.as_deref(), a.timeout, roots_of(&a.roots)?)
        }
        Command::Plugin { command } => match command {
            PluginCommand::New(a) => plugin_new(&a),
            PluginCommand::Build(a) => plugin_build(&a.folder),
            PluginCommand::Run(a) => confined(&a.roots, || plugin_run(&a)),
            PluginCommand::Publish(a) => publish_plugin(&a.folder, &a.server),
        },
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

fn mcp(attach: Option<&str>, timeout: Option<u64>, roots: Option<figcraft_engine::file_access::Roots>) -> CliResult<ExitCode> {
    let backend: Box<dyn figcraft_mcp::Backend> = match attach {
        Some(addr) => {
            let no_app = |e: std::io::Error| format!("can't reach the FigCraft app at {addr}: {e} (start it with `figcraft --control PORT`)");
            let b = figcraft_mcp::Attached::connect(addr).map_err(no_app)?;
            match timeout {
                Some(secs) => Box::new(b.with_timeout(std::time::Duration::from_secs(secs)).map_err(no_app)?),
                None => Box::new(b),
            }
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

fn mcp_harness(status: bool, install_target: Option<&str>, port: u16) -> CliResult<ExitCode> {
    use figcraft_engine::harness::{HarnessKind, detect_cli_binary, inspect_all, install};
    let cli = detect_cli_binary();
    let opt_port = (port != 0).then_some(port);
    if let Some(target) = install_target {
        let targets: Vec<HarnessKind> = if target.eq_ignore_ascii_case("all") {
            HarnessKind::ALL.to_vec()
        } else {
            let kind = HarnessKind::parse(target).ok_or_else(|| {
                CliError::Usage(format!("unknown harness `{target}` (expected antigravity, claude-code, claude-desktop, codex or all)"))
            })?;
            vec![kind]
        };
        for kind in targets {
            let res = install(kind, &cli, opt_port).map_err(CliError::Failed)?;
            println!("Configured {} ({}) -> {}", kind.label(), kind.id(), res.config_path.display());
        }
    }
    if status || install_target.is_none() {
        let statuses = inspect_all(&cli, opt_port);
        println!("FigCraft CLI binary: {cli}");
        println!("Control port:        {}", if port == 0 { "headless".to_string() } else { port.to_string() });
        println!();
        for h in statuses {
            let state = if h.configured { "configured" } else { "not configured" };
            println!("  {:16} [{:14}] {}", h.label, state, h.config_path.display());
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn plugin_new(a: &args::PluginNewArgs) -> CliResult<ExitCode> {
    let mut s = figcraft_engine::Session::new();
    let name =
        a.name.clone().unwrap_or_else(|| std::path::Path::new(&a.folder).file_name().and_then(|n| n.to_str()).unwrap_or("My Plugin").to_string());
    let mut params = json!({
        "dir": a.folder,
        "name": name,
        "template": a.template,
    });
    if let Some(id) = &a.id {
        params["id"] = json!(id);
    }
    let r = s.execute("plugin.create", &params).map_err(|e| CliError::Failed(e.to_string()))?;
    println!(
        "Created plugin {} ({}) in {}",
        r["name"].as_str().unwrap_or_default(),
        r["id"].as_str().unwrap_or_default(),
        r["dir"].as_str().unwrap_or_default()
    );
    Ok(ExitCode::SUCCESS)
}

fn plugin_build(folder: &str) -> CliResult<ExitCode> {
    let dir = std::path::Path::new(folder);
    let manifest_path =
        if dir.file_name().is_some_and(|n| n == "figcraft-plugin.json") { dir.to_path_buf() } else { dir.join("figcraft-plugin.json") };
    let root = manifest_path.parent().unwrap_or(dir);
    let raw = std::fs::read_to_string(&manifest_path).map_err(|e| CliError::Failed(format!("{}: {e}", manifest_path.display())))?;
    let manifest: Value = serde_json::from_str(&raw).map_err(|e| CliError::Failed(format!("{}: {e}", manifest_path.display())))?;
    let main_wasm = manifest["main"].as_str().unwrap_or("plugin.wasm");
    let cargo = cargo_binary();
    let status = std::process::Command::new(&cargo)
        .arg("build")
        .arg("--release")
        .arg("--target")
        .arg("wasm32-unknown-unknown")
        .current_dir(root)
        .status()
        .map_err(|e| CliError::Failed(format!("failed to run cargo: {e}")))?;
    if !status.success() {
        return Err(CliError::Failed("cargo build --release --target wasm32-unknown-unknown failed".into()));
    }
    let wasm_out = root.join(main_wasm);
    // Copy the built .wasm from target/wasm32-unknown-unknown/release/*.wasm to the manifest's `main` path.
    let rel = root.join("target").join("wasm32-unknown-unknown").join("release");
    if let Ok(entries) = std::fs::read_dir(&rel) {
        for entry in entries.flatten() {
            if entry.path().extension().is_some_and(|e| e == "wasm") {
                let _ = std::fs::copy(entry.path(), &wasm_out);
                break;
            }
        }
    }
    let mut s = figcraft_engine::Session::new();
    let imported = s.execute("plugin.import", &json!({"path": manifest_path.to_string_lossy()})).map_err(|e| CliError::Failed(e.to_string()))?;
    println!("Built {} ({}) -> {}", imported["name"].as_str().unwrap_or_default(), imported["id"].as_str().unwrap_or_default(), wasm_out.display());
    Ok(ExitCode::SUCCESS)
}

fn plugin_run(a: &args::PluginRunArgs) -> CliResult<ExitCode> {
    let params_val: Value = serde_json::from_str(&a.params).map_err(|e| CliError::Usage(format!("--params is not valid JSON: {e}")))?;
    let mut s = match &a.input {
        Some(path) => figcraft_engine::Session::with_document(files::load(path)?),
        None => figcraft_engine::Session::new(),
    };
    let imported = s.execute("plugin.import", &json!({"path": a.folder})).map_err(|e| CliError::Failed(e.to_string()))?;
    let id = imported["id"].as_str().unwrap_or_default().to_string();
    let mut run_params = json!({"id": id, "params": params_val});
    if let Some(cmd) = &a.command {
        run_params["command"] = json!(cmd);
    }
    let out = s.execute("plugin.run", &run_params).map_err(|e| CliError::Failed(e.to_string()))?;
    println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
    if let Some(out_path) = &a.out {
        s.execute("file.saveAs", &json!({"path": out_path})).map_err(|e| CliError::Failed(e.to_string()))?;
    }
    Ok(ExitCode::SUCCESS)
}

fn cargo_binary() -> std::path::PathBuf {
    use std::path::PathBuf;
    if let Some(c) = std::env::var_os("CARGO").filter(|v| !v.is_empty()).map(PathBuf::from)
        && c.is_file()
    {
        return c;
    }
    let name = if cfg!(windows) { "cargo.exe" } else { "cargo" };
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    if let Some(h) = home {
        let candidate = h.join(".cargo").join("bin").join(name);
        if candidate.is_file() {
            return candidate;
        }
    }
    PathBuf::from(name)
}
