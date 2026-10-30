//! The command line, as clap types (craft-contrib RFC-0012). Values are parsed into their types
//! here, so a bad one is a usage error naming its flag (exit 2) before any work starts, and the
//! handlers never see it. A flag a subcommand doesn't take is a usage error, and so is one given
//! twice, except the automation roots, which repeat. A value starting with `-` is written
//! `--filter=-x`; after `--` every argument is a positional.

use std::ffi::OsString;
use std::net::{Ipv4Addr, SocketAddr};

use clap::{ArgAction, Args, CommandFactory, FromArgMatches, Parser, Subcommand};
use figcraft_doc::{PrintSettings, VdCompat};

/// FigCraft without a window
///
/// Files: .figcraft (native), .penpot (Penpot 2.x), .json (FigCraft or REST API file JSON), .fig
/// (best effort; `info` lists what wasn't imported), .sketch (Sketch 48 or newer, best effort),
/// .svg, .png/.jpg/.webp/.gif/.bmp (open as an image layer).
#[derive(Parser)]
#[command(name = "figcraft-cli", bin_name = "figcraft-cli", version, arg_required_else_help = true, after_help = "See docs/cli.md and docs/mcp.md.")]
pub struct Cli {
    /// Lay text out with the fonts built into FigCraft only, for renders that match on every
    /// machine (or set FIGCRAFT_NO_SYSTEM_FONTS=1). Otherwise installed fonts are indexed the
    /// first time text is laid out, as in the app
    #[arg(long, global = true)]
    pub no_system_fonts: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    Info(InfoArgs),
    Export(ExportArgs),
    Convert(ConvertArgs),
    Render(RenderArgs),
    Run(RunArgs),
    Commands(CommandsArgs),
    Mcp(McpArgs),
    /// Publish plugins to a server's plugin directory
    Plugin {
        #[command(subcommand)]
        command: PluginCommand,
    },
    /// Publish a code repository's component mappings (code connect) to a design file
    Connect {
        #[command(subcommand)]
        command: ConnectCommand,
    },
    Perf(PerfArgs),
}

/// The folders a file subcommand may touch. Without either flag it reads and writes as the user
/// who started it; with one, reads are allowed only inside a read or write root and writes only
/// inside a write root, so a read root alone denies writing altogether.
#[derive(Args)]
pub struct RootArgs {
    /// Confine file access: a folder that may be read (repeatable; it must exist)
    #[arg(long, value_name = "DIR")]
    pub automation_read_root: Vec<String>,
    /// Confine file access: a folder that may be read and written (repeatable; it must exist)
    #[arg(long, value_name = "DIR")]
    pub automation_write_root: Vec<String>,
}

/// Pages, node counts, top-level frames, style/variable/image counts
#[derive(Args)]
pub struct InfoArgs {
    /// The document to read
    #[arg(value_name = "FILE")]
    pub file: String,
    /// Print JSON
    #[arg(long)]
    pub json: bool,
    #[command(flatten)]
    pub roots: RootArgs,
}

/// Export frames, a layer or the whole file
///
/// <OUT> is a folder: one file per top-level frame of the page. An <OUT> with an image extension,
/// or --node, writes one file. `fig` (or an <OUT> ending in .fig) writes the whole file as .fig,
/// experimental: other apps may not open every file. `penpot` (or an <OUT> ending in .penpot)
/// writes the whole file as a Penpot 2.x archive. The format comes from --format, else <OUT>'s
/// extension, else png.
#[derive(Args)]
pub struct ExportArgs {
    /// The document to read
    #[arg(value_name = "FILE")]
    pub file: String,
    /// The folder or file to write
    #[arg(short = 'o', long = "out", value_name = "OUT")]
    pub out: String,
    /// png, jpg, svg, webp, pdf, xml (Android Vector Drawable), fig or penpot
    #[arg(long, value_name = "FORMAT", value_parser = format)]
    pub format: Option<String>,
    #[command(flatten)]
    pub scale: ScaleArg,
    /// The page to export (default the first)
    #[arg(long, value_name = "NAME")]
    pub page: Option<String>,
    /// The layer to export, by id (12 or 0:12) or name
    #[arg(long, value_name = "ID|NAME")]
    pub node: Option<String>,
    /// PDF: grow each page by this bleed, in points or a length like 3mm or 0.125in (up to 2in)
    #[arg(long, value_name = "LENGTH", value_parser = bleed, default_value = "0")]
    pub bleed: f64,
    /// PDF: draw crop marks outside the bleed
    #[arg(long)]
    pub crop_marks: bool,
    /// PDF: draw bleed marks outside the bleed
    #[arg(long)]
    pub bleed_marks: bool,
    /// PDF: write CMYK colour
    #[arg(long)]
    pub cmyk: bool,
    /// Android Vector Drawable: what the format can't draw is approximated (bestEffort), left out
    /// (exact) or dropped without notes (ignore)
    #[arg(long, value_name = "bestEffort|exact|ignore", value_parser = vd_compat, default_value = "bestEffort")]
    pub vd_compat: VdCompat,
    #[command(flatten)]
    pub roots: RootArgs,
}

impl ExportArgs {
    /// The PDF print options.
    pub fn print(&self) -> PrintSettings {
        PrintSettings { bleed: self.bleed, crop_marks: self.crop_marks, bleed_marks: self.bleed_marks, cmyk: self.cmyk }
    }
}

/// `--scale`, shared by `export` and `render`.
#[derive(Args)]
pub struct ScaleArg {
    /// Raster scale, in (0, 64]: 2 or 2x
    // allow_negative_numbers makes `--scale -3` an out-of-range value rather than an unexpected argument.
    #[arg(long, value_name = "N", value_parser = scale, default_value = "1", allow_negative_numbers = true)]
    pub scale: f64,
}

/// Convert by extension (.figcraft, .json, .svg, .png, .jpg, .webp, .pdf, .fig, .penpot)
#[derive(Args)]
pub struct ConvertArgs {
    /// The document to read
    #[arg(value_name = "IN")]
    pub input: String,
    /// The file to write; its extension picks the format
    #[arg(value_name = "OUT")]
    pub output: String,
    #[command(flatten)]
    pub roots: RootArgs,
}

/// Render the whole page with its background as png, jpg or webp
#[derive(Args)]
pub struct RenderArgs {
    /// The document to read
    #[arg(value_name = "FILE")]
    pub file: String,
    /// The image to write
    #[arg(short = 'o', long = "out", value_name = "OUT.png")]
    pub out: String,
    #[command(flatten)]
    pub scale: ScaleArg,
    /// The page to render (default the first)
    #[arg(long, value_name = "NAME")]
    pub page: Option<String>,
    #[command(flatten)]
    pub roots: RootArgs,
}

/// Run a script of commands in a headless session, one JSON result line per step
///
/// The script is a JSON array (or JSON lines) of {"command": id, "params": {...}}, run as one
/// all-or-nothing undo step.
#[derive(Args)]
pub struct RunArgs {
    /// The script to run
    #[arg(value_name = "SCRIPT.json|SCRIPT.jsonl")]
    pub script: String,
    /// Open this file first instead of a new document
    #[arg(long = "in", value_name = "FILE")]
    pub input: Option<String>,
    /// Save the result here, by extension
    #[arg(long, value_name = "FILE")]
    pub out: Option<String>,
    #[command(flatten)]
    pub roots: RootArgs,
}

/// List every command with its label, shortcut, undoable flag (*) and parameters
#[derive(Args)]
pub struct CommandsArgs {
    /// Print JSON
    #[arg(long)]
    pub json: bool,
    /// Only the commands whose id, label or parameters contain this text
    #[arg(long, value_name = "TEXT")]
    pub filter: Option<String>,
}

/// Serve MCP on stdio: headless, or attached to a running app
///
/// The automation roots confine the headless session's file access. A server started with
/// --attach is confined by the app it attaches to: start that app with `figcraft --control PORT`
/// and the same roots.
#[derive(Args)]
pub struct McpArgs {
    /// Attach to the app started with `figcraft --control PORT`; a bare port means 127.0.0.1:PORT.
    /// Refuses the automation roots
    #[arg(long, value_name = "PORT", value_parser = attach, conflicts_with_all = ["automation_read_root", "automation_write_root"])]
    pub attach: Option<String>,
    #[command(flatten)]
    pub roots: RootArgs,
}

#[derive(Subcommand)]
pub enum PluginCommand {
    Publish(PublishArgs),
}

/// Send a plugin folder to a figcraft-server's plugin directory as a new version
///
/// The folder holds figcraft-plugin.json, its module and an optional assets/ folder. The server's
/// key comes from the address or the FIGCRAFT_SERVER_KEY environment variable (which keeps it out
/// of the shell history); it is never printed.
#[derive(Args)]
pub struct PublishArgs {
    /// The plugin folder
    #[arg(value_name = "FOLDER")]
    pub folder: String,
    /// The server, ws://host:port/ (optionally ?key=KEY)
    #[arg(long, value_name = "URL")]
    pub server: String,
}

#[derive(Subcommand)]
pub enum ConnectCommand {
    Publish(ConnectPublishArgs),
}

/// Link a design file's components to their code as a figcraft.connect.json says, and save it
///
/// The mappings are what Dev Mode's Export code connections writes, kept in the code repository:
/// each links a component (or component set) of that name to its code component for a language.
/// Entries that don't fit (an unknown component, a bad template) are reported and skipped.
#[derive(Args)]
pub struct ConnectPublishArgs {
    /// The design file (.figcraft), saved in place
    #[arg(value_name = "FILE")]
    pub file: String,
    /// The mappings
    #[arg(long, value_name = "FILE", default_value = "figcraft.connect.json")]
    pub mappings: String,
    /// Report what would be linked without saving the file
    #[arg(long)]
    pub dry_run: bool,
    #[command(flatten)]
    pub roots: RootArgs,
}

/// Measure the hot paths on synthetic documents and print Markdown tables
#[derive(Args)]
pub struct PerfArgs {
    /// Document sizes in layers, each clamped to 10..=2,000,000
    #[arg(long, value_name = "N[,N...]", value_parser = whole_number, value_delimiter = ',', action = ArgAction::Set, default_value = "1000,10000")]
    pub layers: Vec<usize>,
    /// Component instance counts, capped to 100,000; 0 skips the components table
    #[arg(long, value_name = "N[,N...]", value_parser = whole_number, value_delimiter = ',', action = ArgAction::Set, default_value = "2000")]
    pub instances: Vec<usize>,
}

/// Why a value was refused: clap prints it after the flag and its value (exit 2).
#[derive(Debug, thiserror::Error)]
enum ValueError {
    #[error("expected a number in (0, 64], like 2 or 2x")]
    Scale,
    #[error("expected points or a length like 3mm or 0.125in, up to 2in")]
    Bleed,
    #[error("expected bestEffort, exact or ignore")]
    VdCompat,
    #[error("expected png, jpg, svg, webp, pdf, xml, fig or penpot")]
    Format,
    #[error("expected whole numbers, like 1000,10_000")]
    WholeNumber,
    #[error("expected a port number or host:port")]
    Attach,
}

fn scale(s: &str) -> Result<f64, ValueError> {
    s.trim_end_matches('x').parse::<f64>().ok().filter(|v| v.is_finite() && *v > 0.0 && *v <= 64.0).ok_or(ValueError::Scale)
}

fn bleed(s: &str) -> Result<f64, ValueError> {
    PrintSettings::parse_length(s).filter(|b| *b <= PrintSettings::MAX_BLEED).ok_or(ValueError::Bleed)
}

fn vd_compat(s: &str) -> Result<VdCompat, ValueError> {
    VdCompat::from_key(s).ok_or(ValueError::VdCompat)
}

/// `--format`, any case; kept as text for the exporter, which also reads it from the extension.
fn format(s: &str) -> Result<String, ValueError> {
    let f = s.to_ascii_lowercase();
    match f.as_str() {
        "png" | "jpg" | "jpeg" | "svg" | "webp" | "pdf" | "xml" | "fig" | "penpot" => Ok(f),
        _ => Err(ValueError::Format),
    }
}

fn whole_number(s: &str) -> Result<usize, ValueError> {
    s.trim().replace('_', "").parse().map_err(|_| ValueError::WholeNumber)
}

/// `mcp --attach`: `host:port` as given, or a bare port on 127.0.0.1.
fn attach(s: &str) -> Result<String, ValueError> {
    if s.contains(':') {
        return Ok(s.to_owned());
    }
    let port: u16 = s.parse().map_err(|_| ValueError::Attach)?;
    Ok(SocketAddr::from((Ipv4Addr::LOCALHOST, port)).to_string())
}

/// Parse the arguments, the program name first. clap reads them as `OsString`s, so a non-Unicode
/// one is a usage error rather than a panic. A bare `--help` or `-h` before `--` is help even after
/// a mistake on the same line (`mcp --attach nope --help`, `export -o --help`), which clap would
/// report first.
pub fn parse(args: Vec<OsString>) -> Result<Cli, clap::Error> {
    let mut cmd = Cli::command();
    match cmd.try_get_matches_from_mut(&args) {
        Ok(matches) => Cli::from_arg_matches(&matches),
        Err(e) => match args.iter().skip(1).take_while(|a| *a != "--").find(|a| *a == "--help" || *a == "-h") {
            Some(flag) => {
                let help: Vec<OsString> = subcommand_path(&cmd, &args).into_iter().chain([flag.clone()]).collect();
                Err(cmd.try_get_matches_from_mut(help).err().unwrap_or(e))
            }
            None => Err(e),
        },
    }
}

/// The program name and the subcommands `args` names (`figcraft-cli plugin publish`): at each
/// level, the first argument that isn't a flag, while it names a subcommand.
fn subcommand_path(cmd: &clap::Command, args: &[OsString]) -> Vec<OsString> {
    let mut path: Vec<OsString> = args.first().cloned().into_iter().collect();
    let mut at = cmd;
    let mut rest = args.iter().skip(1).filter(|a| !a.to_string_lossy().starts_with('-'));
    while let Some(sub) = rest.next().and_then(|a| a.to_str()).and_then(|name| at.find_subcommand(name)) {
        path.push(sub.get_name().into());
        at = sub;
    }
    path
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;
    use clap::error::ErrorKind;

    use super::{Cli, Command, ConnectCommand, PluginCommand, parse};

    fn try_parse(args: &[&str]) -> Result<Cli, clap::Error> {
        parse(std::iter::once("figcraft-cli").chain(args.iter().copied()).map(Into::into).collect())
    }

    fn ok(args: &[&str]) -> Command {
        match try_parse(args) {
            Ok(cli) => cli.command,
            Err(e) => panic!("{args:?}: {e}"),
        }
    }

    fn kind(args: &[&str]) -> ErrorKind {
        match try_parse(args) {
            Ok(_) => panic!("{args:?} parsed"),
            Err(e) => e.kind(),
        }
    }

    #[test]
    fn the_definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn info_and_the_roots() {
        let Command::Info(a) = ok(&["info", "a.figcraft", "--json", "--automation-read-root", "in", "--automation-read-root=more"]) else {
            panic!("info")
        };
        assert_eq!(a.file, "a.figcraft");
        assert!(a.json);
        assert_eq!(a.roots.automation_read_root, ["in", "more"]);
        assert!(a.roots.automation_write_root.is_empty());
        let Command::Info(a) = ok(&["info", "--json", "--", "--drawing.figcraft"]) else { panic!("info") };
        assert_eq!(a.file, "--drawing.figcraft");
    }

    #[test]
    fn export_takes_every_documented_option() {
        let Command::Export(a) = ok(&[
            "export",
            "app.figcraft",
            "-o",
            "poster.pdf",
            "--format=PDF",
            "--scale",
            "2x",
            "--page",
            "Screens",
            "--node",
            "Poster",
            "--bleed",
            "3mm",
            "--crop-marks",
            "--bleed-marks",
            "--cmyk",
            "--vd-compat",
            "exact",
        ]) else {
            panic!("export")
        };
        assert_eq!((a.file.as_str(), a.out.as_str(), a.format.as_deref()), ("app.figcraft", "poster.pdf", Some("pdf")));
        assert_eq!((a.scale.scale, a.page.as_deref(), a.node.as_deref()), (2.0, Some("Screens"), Some("Poster")));
        let print = a.print();
        assert!((print.bleed - 3.0 * 72.0 / 25.4).abs() < 1e-9, "{}", print.bleed);
        assert!(print.crop_marks && print.bleed_marks && print.cmyk);
        assert_eq!(a.vd_compat, figcraft_doc::VdCompat::Exact);
        // The defaults, and `--out` for `-o`.
        let Command::Export(a) = ok(&["export", "app.figcraft", "--out=exports/"]) else { panic!("export") };
        assert_eq!((a.out.as_str(), a.format.as_deref(), a.scale.scale, a.print().bleed), ("exports/", None, 1.0, 0.0));
        assert_eq!(a.vd_compat, figcraft_doc::VdCompat::default());
    }

    #[test]
    fn convert_render_run_commands_perf() {
        let Command::Convert(a) = ok(&["convert", "in.svg", "out.figcraft"]) else { panic!("convert") };
        assert_eq!((a.input.as_str(), a.output.as_str()), ("in.svg", "out.figcraft"));
        let Command::Render(a) = ok(&["render", "doc.figcraft", "--out", "doc.png", "--scale=0.5", "--page", "P"]) else { panic!("render") };
        assert_eq!((a.out.as_str(), a.scale.scale, a.page.as_deref()), ("doc.png", 0.5, Some("P")));
        let Command::Run(a) = ok(&["run", "s.jsonl", "--in", "a.figcraft", "--out", "b.figcraft", "--automation-write-root", "."]) else {
            panic!("run")
        };
        assert_eq!((a.script.as_str(), a.input.as_deref(), a.out.as_deref()), ("s.jsonl", Some("a.figcraft"), Some("b.figcraft")));
        assert_eq!(a.roots.automation_write_root, ["."]);
        let Command::Commands(a) = ok(&["commands", "--json", "--filter=--help"]) else { panic!("commands") };
        assert!(a.json);
        assert_eq!(a.filter.as_deref(), Some("--help"));
        let Command::Perf(a) = ok(&["perf"]) else { panic!("perf") };
        assert_eq!((a.layers, a.instances), (vec![1000, 10_000], vec![2000]));
        let Command::Perf(a) = ok(&["perf", "--layers", "1000,10_000, 50000", "--instances=0"]) else { panic!("perf") };
        assert_eq!((a.layers, a.instances), (vec![1000, 10_000, 50_000], vec![0]));
    }

    #[test]
    fn mcp_and_plugin_publish() {
        let Command::Mcp(a) = ok(&["mcp"]) else { panic!("mcp") };
        assert_eq!(a.attach, None);
        let Command::Mcp(a) = ok(&["mcp", "--attach", "7979"]) else { panic!("mcp") };
        assert_eq!(a.attach.as_deref(), Some("127.0.0.1:7979"));
        let Command::Mcp(a) = ok(&["mcp", "--attach=localhost:7979"]) else { panic!("mcp") };
        assert_eq!(a.attach.as_deref(), Some("localhost:7979"));
        let Command::Mcp(a) = ok(&["mcp", "--automation-read-root", "in", "--automation-write-root", "out"]) else { panic!("mcp") };
        assert_eq!((a.roots.automation_read_root, a.roots.automation_write_root), (vec!["in".to_owned()], vec!["out".to_owned()]));
        let Command::Plugin { command: PluginCommand::Publish(a) } = ok(&["plugin", "publish", "dist/counter", "--server", "ws://h:47900/"]) else {
            panic!("plugin")
        };
        assert_eq!((a.folder.as_str(), a.server.as_str()), ("dist/counter", "ws://h:47900/"));
        let Command::Connect { command: ConnectCommand::Publish(a) } = ok(&["connect", "publish", "ui.figcraft"]) else { panic!("connect") };
        assert_eq!((a.file.as_str(), a.mappings.as_str(), a.dry_run), ("ui.figcraft", "figcraft.connect.json", false));
        let Command::Connect { command: ConnectCommand::Publish(a) } =
            ok(&["connect", "publish", "ui.figcraft", "--mappings", "code/map.json", "--dry-run"])
        else {
            panic!("connect")
        };
        assert_eq!((a.mappings.as_str(), a.dry_run), ("code/map.json", true));
    }

    #[test]
    fn the_font_flag_goes_before_or_after_the_subcommand() {
        for args in [&["--no-system-fonts", "commands"][..], &["commands", "--no-system-fonts"]] {
            assert!(try_parse(args).is_ok_and(|cli| cli.no_system_fonts), "{args:?}");
        }
        assert!(try_parse(&["commands"]).is_ok_and(|cli| !cli.no_system_fonts));
    }

    #[test]
    fn bad_arguments_are_usage_errors() {
        use ErrorKind::{
            ArgumentConflict, DisplayHelpOnMissingArgumentOrSubcommand, InvalidSubcommand, InvalidValue, MissingRequiredArgument, TooManyValues,
            UnknownArgument, ValueValidation,
        };
        let cases: &[(&[&str], ErrorKind)] = &[
            (&[], DisplayHelpOnMissingArgumentOrSubcommand),
            (&["bogus"], InvalidSubcommand),
            (&["version"], InvalidSubcommand),
            (&["export", "x.figcraft"], MissingRequiredArgument),
            (&["info", "--nope", "a"], UnknownArgument),
            (&["info", "a", "b"], UnknownArgument),
            (&["info", "--json=1", "a"], TooManyValues),
            (&["render", "a", "-o"], InvalidValue),
            (&["export", "a", "-o", "b", "--scale", "0"], ValueValidation),
            (&["export", "a", "-o", "b", "--scale", "-3"], ValueValidation),
            (&["export", "a", "-o", "b", "--scale", "65"], ValueValidation),
            (&["export", "a", "-o", "b", "--bleed", "lots"], ValueValidation),
            (&["export", "a", "-o", "b", "--bleed", "3in"], ValueValidation),
            (&["export", "a", "-o", "b", "--vd-compat", "sloppy"], ValueValidation),
            (&["export", "a", "-o", "b", "--format", "bmp"], ValueValidation),
            (&["export", "a", "-o", "b", "--page", "A", "--page", "B"], ArgumentConflict),
            (&["perf", "--shapes", "lots"], UnknownArgument),
            (&["perf", "--layers", "10,x"], ValueValidation),
            (&["perf", "--layers", "10", "--layers", "20"], ArgumentConflict),
            (&["mcp", "--attach", "notaport"], ValueValidation),
            (&["mcp", "--attach", "7979", "--automation-read-root", "."], ArgumentConflict),
            (&["mcp", "--automation-write-root"], InvalidValue),
            (&["mcp", "stray"], UnknownArgument),
            (&["convert", "a"], MissingRequiredArgument),
            (&["plugin", "publish", "dist"], MissingRequiredArgument),
            (&["commands", "--filter", "--json"], InvalidValue),
        ];
        for (args, want) in cases {
            assert_eq!(kind(args), *want, "{args:?}");
            assert_eq!(try_parse(args).err().map(|e| e.exit_code()), Some(2), "{args:?}");
        }
        assert_eq!(try_parse(&["plugin"]).err().map(|e| e.exit_code()), Some(2));
    }

    #[test]
    fn help_and_version_exit_0() {
        assert_eq!(kind(&["--version"]), ErrorKind::DisplayVersion);
        assert_eq!(kind(&["-V"]), ErrorKind::DisplayVersion);
        for args in [
            &["--help"][..],
            &["-h"],
            &["help"],
            &["help", "export"],
            &["export", "x.figcraft", "-o", "out", "--help"],
            &["export", "-o", "--help"],
            &["mcp", "--attach", "notaport", "--help"],
            &["mcp", "--bogus", "-h"],
            &["--no-system-fonts", "plugin", "publish", "--bogus", "--help"],
        ] {
            let e = try_parse(args).err();
            assert_eq!(e.as_ref().map(clap::Error::kind), Some(ErrorKind::DisplayHelp), "{args:?}");
            assert_eq!(e.map(|e| e.exit_code()), Some(0), "{args:?}");
        }
        // The help is the subcommand's, even after a mistake.
        let help = try_parse(&["plugin", "publish", "--bogus", "--help"]).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(help.contains("Usage: figcraft-cli plugin publish"), "{help}");
        // After `--`, `-h` is a file name.
        let Command::Info(a) = ok(&["info", "--", "-h"]) else { panic!("info") };
        assert_eq!(a.file, "-h");
    }

    /// Every subcommand's help renders and documents each of its options.
    #[test]
    fn every_subcommand_has_help() {
        let mut cli = Cli::command();
        cli.build();
        let mut todo = vec![cli];
        while let Some(mut cmd) = todo.pop() {
            let help = cmd.render_long_help().to_string();
            assert!(help.contains("Usage: figcraft-cli"), "{}: {help}", cmd.get_name());
            for arg in cmd.get_arguments().filter(|a| !matches!(a.get_long(), Some("help" | "version"))) {
                assert!(arg.get_help().is_some(), "{} {}", cmd.get_name(), arg.get_id());
            }
            todo.extend(cmd.get_subcommands().filter(|s| s.get_name() != "help").cloned());
        }
    }
}
