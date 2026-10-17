//! `figcraft-cli`: FigCraft without a window. Inspect, export, convert and render documents, run
//! command scripts, list commands, serve MCP on stdio, publish plugins to a server's plugin
//! directory and measure performance.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod args;
mod files;
mod perf;

use std::io::Write;
use std::process::ExitCode;

use args::Args;
use serde_json::{Value, json};

pub const USAGE: &str = "figcraft-cli — FigCraft without a window

USAGE:
  figcraft-cli info <file> [--json]
  figcraft-cli export <file> -o <out> [--format png|jpg|svg|webp|pdf|fig] [--scale N] [--page NAME] [--node ID|NAME]
                      [--bleed LENGTH] [--crop-marks] [--bleed-marks] [--cmyk]
  figcraft-cli convert <in> <out>
  figcraft-cli render <file> -o <out.png> [--scale N] [--page NAME]
  figcraft-cli run <script.json|script.jsonl> [--in <file>] [--out <file>]
  figcraft-cli commands [--json] [--filter TEXT]
  figcraft-cli mcp [--attach PORT] [--automation-read-root DIR]... [--automation-write-root DIR]...
  figcraft-cli plugin publish <folder> --server ws://host:port/[?key=KEY]
  figcraft-cli perf [--layers N[,N...]] [--instances N[,N...]]
  figcraft-cli help | --help | --version

Every subcommand takes --no-system-fonts (or set FIGCRAFT_NO_SYSTEM_FONTS=1): text uses only the
fonts built into FigCraft, for renders that match on every machine. Otherwise installed fonts are
indexed the first time text is laid out, as in the app.

Files: .figcraft (native), .json (FigCraft or REST API file JSON), .fig (best effort; `info` lists what
wasn't imported), .svg, .png/.jpg/.webp/.gif/.bmp (open as an image layer). `export` writes one file
per top-level frame into the <out> directory, or one file when <out> has an image extension or --node
is given (`fig`, or an <out> ending in .fig: the whole file as .fig, experimental; other apps may
not open every file); PDFs take print options (a bleed like 3mm or 0.125in,
crop and bleed marks, CMYK colour). `run` executes a JSON array (or JSON lines) of
{\"command\": id, \"params\": {...}} in a headless session and prints one JSON result line per step.
`plugin publish` sends a plugin folder (figcraft-plugin.json, its module, an optional assets/ folder)
to a figcraft-server's plugin directory as a new version; the server's key comes from the address or
the FIGCRAFT_SERVER_KEY environment variable (which keeps it out of the shell history).
`mcp --automation-read-root DIR` / `--automation-write-root DIR` (each may be repeated; the folders
must exist) confine the files the agent's commands touch: reads only inside a read or write root,
writes only inside a write root, everything else refused. A running app confines itself: start it
with `figcraft --control PORT` and the same flags (`--attach` refuses them).
See docs/cli.md and docs/mcp.md.";

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match run(&argv) {
        Ok(code) => code,
        Err(CliError::Usage(msg)) => {
            eprintln!("error: {msg}\n\n{USAGE}");
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
    /// Bad arguments: usage is printed, exit code 2.
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

fn run(argv: &[String]) -> CliResult<ExitCode> {
    let system_fonts = !argv.iter().any(|a| a == "--no-system-fonts");
    let argv: Vec<String> = argv.iter().filter(|a| *a != "--no-system-fonts").cloned().collect();
    if system_fonts {
        // Text lays out with the installed fonts, like the desktop app, so headless renders and
        // auto layout sizes match what users see.
        figcraft_text::scan_system_fonts_on_demand();
    }
    let Some((cmd, rest)) = argv.split_first() else { return Err(CliError::Usage("missing subcommand".into())) };
    match cmd.as_str() {
        "help" | "--help" | "-h" => {
            println!("{USAGE}");
            Ok(ExitCode::SUCCESS)
        }
        "--version" | "-V" | "version" => {
            println!("figcraft-cli {}", env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }
        "info" => {
            let a = Args::parse(rest, &[], &["json"])?;
            let file = a.one_positional("info <file>")?;
            files::info(&file, a.flag("json"))
        }
        "export" => {
            let a = Args::parse(rest, &["format", "scale", "page", "node", "o", "out", "bleed"], &["crop-marks", "bleed-marks", "cmyk"])?;
            let file = a.one_positional("export <file> -o <out>")?;
            let out = a.value("o").or_else(|| a.value("out")).ok_or_else(|| CliError::Usage("export needs -o <out>".into()))?;
            let target =
                files::Target { format: a.value("format"), scale: a.scale()?, page: a.value("page"), node: a.value("node"), print: a.print()? };
            files::export(&file, &out, &target)
        }
        "convert" => {
            let a = Args::parse(rest, &[], &[])?;
            match a.positional.as_slice() {
                [i, o] => files::convert(i, o),
                _ => Err(CliError::Usage("convert <in> <out>".into())),
            }
        }
        "render" => {
            let a = Args::parse(rest, &["o", "out", "scale", "page"], &[])?;
            let file = a.one_positional("render <file> -o <out.png>")?;
            let out = a.value("o").or_else(|| a.value("out")).ok_or_else(|| CliError::Usage("render needs -o <out.png>".into()))?;
            files::render(&file, &out, a.scale()?, a.value("page").as_deref())
        }
        "run" => {
            let a = Args::parse(rest, &["in", "out"], &[])?;
            let script = a.one_positional("run <script.json|.jsonl>")?;
            files::run_script(&script, a.value("in").as_deref(), a.value("out").as_deref())
        }
        "commands" => {
            let a = Args::parse(rest, &["filter"], &["json"])?;
            if !a.positional.is_empty() {
                return Err(CliError::Usage("commands takes no positional arguments".into()));
            }
            commands(a.flag("json"), a.value("filter").as_deref());
            Ok(ExitCode::SUCCESS)
        }
        "mcp" => {
            let a = Args::parse(rest, &["attach", "automation-read-root", "automation-write-root"], &[])?;
            if !a.positional.is_empty() {
                return Err(CliError::Usage("mcp takes no positional arguments".into()));
            }
            let (read, write) = (a.values("automation-read-root"), a.values("automation-write-root"));
            let roots = match (read.is_empty() && write.is_empty(), a.value("attach")) {
                (true, _) => None,
                // The app runs the commands: only its own roots can confine them.
                (false, Some(_)) => {
                    return Err(CliError::Usage(
                        "--attach can't take automation roots: start the app with them instead (`figcraft --control PORT --automation-read-root DIR ...`)"
                            .into(),
                    ));
                }
                (false, None) => Some(figcraft_engine::file_access::Roots::new(read, write).map_err(CliError::Failed)?),
            };
            mcp(a.value("attach").as_deref(), roots)
        }
        "plugin" => {
            let a = Args::parse(rest, &["server"], &[])?;
            match a.positional.as_slice() {
                [verb, folder] if verb == "publish" => {
                    let server = a.value("server").ok_or_else(|| CliError::Usage("plugin publish needs --server ws://host:port/".into()))?;
                    publish_plugin(folder, &server)
                }
                _ => Err(CliError::Usage("expected: plugin publish <folder> --server ws://host:port/".into())),
            }
        }
        "perf" => {
            let a = Args::parse(rest, &["layers", "instances"], &[])?;
            let sizes: Vec<usize> = a.usizes("layers", &[1000, 10_000])?.into_iter().map(|n| n.clamp(10, 2_000_000)).collect();
            let instances: Vec<usize> = a.usizes("instances", &[2000])?.into_iter().filter(|n| *n > 0).map(|n| n.min(100_000)).collect();
            perf::run(&sizes, &instances, system_fonts)
        }
        other => Err(CliError::Usage(format!("unknown subcommand `{other}`"))),
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
        Some(p) => {
            let addr = if p.contains(':') {
                p.to_string()
            } else {
                format!("127.0.0.1:{}", p.parse::<u16>().map_err(|_| CliError::Usage(format!("--attach needs a port number, got `{p}`")))?)
            };
            let b = figcraft_mcp::Attached::connect(&addr)
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
