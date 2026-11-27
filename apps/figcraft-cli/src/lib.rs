//! `figcraft-cli`: FigCraft without a window. Inspect, export, convert and render documents, run
//! command scripts, list commands, serve MCP on stdio, publish plugins to a server's plugin
//! directory, publish code connect mappings to a design file and measure performance. `run` is the
//! whole program, so it can be tested in-process as well as through the binary. The conventions are
//! RFC-0018's: exit 0, 1 (a command failed) or 2 (a usage error), results on stdout,
//! `error: <message>` on stderr.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

/// Writes a line to `$io.out`. A closed pipe (`figcraft-cli commands | head`) is not worth an error:
/// the rest of the output is dropped.
macro_rules! outln {
    ($io:expr) => {
        drop(writeln!($io.out))
    };
    ($io:expr, $($arg:tt)*) => {
        drop(writeln!($io.out, $($arg)*))
    };
}

/// Writes a line to `$io.err`.
macro_rules! errln {
    ($io:expr, $($arg:tt)*) => {
        drop(writeln!($io.err, $($arg)*))
    };
}

mod args;
mod files;
mod perf;

use std::ffi::OsString;
use std::io::Write;

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

/// Why a command did not succeed. The variant is the exit code: a usage error is 2, a failure 1.
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// Bad arguments found after parsing (an output extension no format writes): exit code 2.
    #[error("{0}")]
    Usage(String),
    /// The operation failed: exit code 1.
    #[error("{0}")]
    Failed(String),
}

/// The two output streams a command writes to: results to `out`, diagnostics to `err`.
pub(crate) struct Io<'a> {
    pub(crate) out: &'a mut dyn Write,
    pub(crate) err: &'a mut dyn Write,
}

impl From<String> for CliError {
    fn from(s: String) -> Self {
        CliError::Failed(s)
    }
}

pub type CliResult<T> = Result<T, CliError>;

fn dispatch(cli: Cli, io: &mut Io) -> CliResult<i32> {
    check(&cli.command)?;
    if !cli.no_system_fonts {
        // Text lays out with the installed fonts, like the desktop app, so headless renders and
        // auto layout sizes match what users see.
        figcraft_text::scan_system_fonts_on_demand();
    } else {
        // Nothing scans them then, not even a `.sketch` import matching its fonts.
        figcraft_text::opt_out_of_system_fonts();
    }
    match cli.command {
        Command::Info(a) => confined(&a.roots, || files::info(&a.file, a.json, io)),
        Command::Export(a) => {
            let target =
                files::Target { print: a.print(), vd_compat: a.vd_compat, scale: a.scale.scale, format: a.format, page: a.page, node: a.node };
            confined(&a.roots, || files::export(&a.file, &a.out, &target, io))
        }
        Command::Convert(a) => confined(&a.roots, || files::convert(&a.input, &a.output, io)),
        Command::Render(a) => confined(&a.roots, || files::render(&a.file, &a.out, a.scale.scale, a.page.as_deref(), io)),
        Command::Run(a) => confined(&a.roots, || files::run_script(&a.script, a.input.as_deref(), a.out.as_deref(), io)),
        Command::Commands(a) => {
            commands(a.json, a.filter.as_deref(), io);
            Ok(0)
        }
        // `--attach` conflicts with the roots: the app runs the commands, only its own roots can confine them.
        Command::Mcp(a) => mcp(a.attach.as_deref(), a.timeout, roots_of(&a.roots)?, io),
        Command::Plugin { command: PluginCommand::Publish(a) } => publish_plugin(&a.folder, &a.server, a.key.as_deref(), io),
        Command::Connect { command: ConnectCommand::Publish(a) } => {
            confined(&a.roots, || files::connect_publish(&a.file, &a.mappings, a.dry_run, io))
        }
        Command::Perf(a) => {
            let sizes: Vec<usize> = a.layers.into_iter().map(|n| n.clamp(10, 2_000_000)).collect();
            let instances: Vec<usize> = a.instances.into_iter().filter(|n| *n > 0).map(|n| n.min(100_000)).collect();
            perf::run(&sizes, &instances, !cli.no_system_fonts, io)
        }
    }
}

fn commands(as_json: bool, filter: Option<&str>, io: &mut Io) {
    let f = filter.map(str::to_lowercase).unwrap_or_default();
    let list: Vec<&figcraft_engine::CommandSpec> = figcraft_engine::all_commands()
        .iter()
        .filter(|c| f.is_empty() || c.id.to_lowercase().contains(&f) || c.label.to_lowercase().contains(&f) || c.params.to_lowercase().contains(&f))
        .collect();
    if as_json {
        let v: Vec<Value> = list
            .iter()
            .map(|c| json!({"id": c.id, "label": c.label, "params": c.params, "menu": c.menu(), "shortcut": c.shortcut(), "undoable": c.history}))
            .collect();
        outln!(io, "{}", serde_json::to_string_pretty(&v).unwrap_or_default());
        return;
    }
    for c in list {
        let sc = c.shortcut().map(|s| format!(" ({s})")).unwrap_or_default();
        let undo = if c.history { "*" } else { "" };
        outln!(io, "{}{undo}  {}{sc}\n    {}", c.id, c.label, c.params);
    }
}

/// Publish the plugin folder `folder` to the plugin directory at `server` (the key from the address
/// or `FIGCRAFT_SERVER_KEY`); never prints the key.
fn publish_plugin(folder: &str, server: &str, key: Option<&str>, io: &mut Io) -> CliResult<i32> {
    let address = server.split('?').next().unwrap_or_default().to_string();
    let server = match key.map(str::trim) {
        Some(key) if !server.contains("key=") && !key.is_empty() => format!("{address}?key={key}"),
        _ => server.to_string(),
    };
    let mut h = figcraft_mcp::Headless::new();
    use figcraft_mcp::Backend as _;
    let r = h.call("engine.execute", json!({"command": "plugin.directory.publish", "params": {"path": folder, "server": server}}))?;
    let e = &r["published"];
    outln!(
        io,
        "Published {} {} ({}) to {address}: {} bytes, SHA-256 {}",
        e["name"].as_str().unwrap_or_default(),
        e["version"].as_str().unwrap_or_default(),
        e["id"].as_str().unwrap_or_default(),
        e["bytes"],
        e["sha256"].as_str().unwrap_or_default()
    );
    Ok(0)
}

fn mcp(attach: Option<&str>, timeout: Option<u64>, roots: Option<figcraft_engine::file_access::Roots>, io: &mut Io) -> CliResult<i32> {
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
    errln!(io, "figcraft MCP server ({}) on stdio", backend.describe());
    if let Some(r) = &roots {
        errln!(io, "file access confined to {}", r.to_json());
    }
    let mut server = figcraft_mcp::Server::new(backend).with_roots(roots);
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    server.serve(stdin.lock(), stdout.lock()).map_err(|e| format!("stdio: {e}"))?;
    Ok(0)
}

/// What the arguments can't say alone: an output that would replace its input needs `--in-place`.
fn check(command: &Command) -> CliResult<()> {
    let refuse = |flag: &str, path: &str, input: &str, allowed: bool| -> CliResult<()> {
        if !same_file(path, input) {
            return Ok(());
        }
        if allowed {
            return Ok(());
        }
        Err(CliError::Usage(format!("{flag} {path} is the input, which it would replace: choose another file, or pass --in-place")))
    };
    match command {
        Command::Convert(a) => refuse("OUT", &a.output, &a.input, a.in_place)?,
        Command::Run(a) => {
            if let (Some(out), Some(input)) = (&a.out, &a.input) {
                refuse("--out", out, input, a.in_place)?;
            }
        }
        Command::Export(a) => refuse("--out", &a.out, &a.file, false)?,
        Command::Render(a) => refuse("--out", &a.out, &a.file, false)?,
        _ => {}
    }
    if let Command::Convert(a) = command
        && a.in_place
        && !same_file(&a.output, &a.input)
    {
        return Err(CliError::Usage("--in-place is for an OUT that is the IN, and this is another file".into()));
    }
    if let Command::Run(a) = command
        && a.in_place
        && !matches!((&a.out, &a.input), (Some(out), Some(input)) if same_file(out, input))
    {
        return Err(CliError::Usage("--in-place is for an --out that is the --in file, and this is another file".into()));
    }
    Ok(())
}

/// Whether two paths name the same existing file, through links and `.` and `..`.
fn same_file(a: &str, b: &str) -> bool {
    matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(a), Ok(b)) if a == b)
}

/// Run the CLI on its arguments (without the program name), writing results to `out` and
/// diagnostics to `err`. Returns the process exit code: 0 on success, 1 on failure, 2 for a usage
/// error. Help and the version go to `out`, exit 0. `mcp` speaks MCP on the process's own stdin
/// and stdout.
pub fn run<I, T>(args: I, out: &mut dyn Write, err: &mut dyn Write) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let argv: Vec<OsString> = std::iter::once(OsString::from("figcraft-cli")).chain(args.into_iter().map(Into::into)).collect();
    let cli = match args::parse(argv) {
        Ok(cli) => cli,
        Err(e) => {
            // Help and `--version` on stdout with 0, usage errors on stderr with 2.
            let to: &mut dyn Write = if e.use_stderr() { err } else { out };
            let _ = write!(to, "{}", e.render());
            return e.exit_code();
        }
    };
    let mut io = Io { out, err };
    // Paths and imported/plugin errors may carry terminal controls; keep diagnostics printable.
    match dispatch(cli, &mut io) {
        Ok(code) => code,
        Err(e @ CliError::Usage(_)) => {
            errln!(io, "error: {}\n\nFor more information, try '--help'.", files::plain(&e.to_string()));
            2
        }
        Err(e @ CliError::Failed(_)) => {
            errln!(io, "error: {}", files::plain(&e.to_string()));
            1
        }
    }
}
