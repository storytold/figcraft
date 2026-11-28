//! The command line: `figcraft-server` serves; `key`, `files`, `links` and `plugins` manage the
//! data directory. When a server is running on the directory they ask it (so revoking a link
//! disconnects whoever joined with it at once); otherwise they change the directory themselves.

use std::fmt::Write as _;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use figcraft_collab::message::{Access, Admin, AdminReply, FileRow, LinkInfo, NewLink, PluginEntry, PluginReply, PluginReq, ReportInfo};
use figcraft_collab_net::hub::Backend;
use figcraft_collab_net::{Endpoint, Invite, manage};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::disk;
use crate::store::{self, KEY, Store};

/// Where a running server says it listens.
pub const SERVER_JSON: &str = "server.json";

pub const USAGE: &str = "\
figcraft-server: FigCraft's collaboration server (files, share links, live sessions)

Usage:
  figcraft-server [serve] [--listen ADDR] [--data DIR] [--public-url URL] [--config FILE]
  figcraft-server key [--rotate]                 the address to publish to, with the server key
  figcraft-server check                          check the settings (after editing the --config file)
  figcraft-server files [list] [--sort size|age|idle]  the files kept (biggest, oldest or longest unopened first)
  figcraft-server files remove FILE              remove a file and its links for good
  figcraft-server links list FILE                a file's share links
  figcraft-server links create FILE --access edit|comment|view [--label TEXT]
  figcraft-server links revoke FILE LINK         revoke a link (whoever joined with it is disconnected)
  figcraft-server plugins [list]                 the plugin directory: every plugin and its versions
  figcraft-server plugins remove ID [--version V]  unpublish a plugin (every version, or one)
  figcraft-server reports                        what people reported about files
  figcraft-server block-ip IP                    refuse an address (an IPv6 address: its /64) and drop its connections
  figcraft-server unblock-ip IP                  accept an address again
  figcraft-server blocked                        the blocked addresses

Options (also keys of the --config TOML file):
  --listen ADDR       where to listen (default 127.0.0.1:47900; 0.0.0.0:47900 for every network)
  --data DIR          the data directory (default ./figcraft-data)
  --public-url URL    how links name the server behind a reverse proxy (wss://host/path/)
  --config FILE       settings: listen, data, public_url, max_files, max_file_mb, max_links,
                      max_connections, max_people, compact_after, compact_mb, max_log_mb,
                      max_plugins, max_plugin_mb, max_plugins_mb, team_files, web_url;
                      for a public server: open_publish,
                      operator_contact, terms_url, trusted_proxy, idle_days, max_total_mb,
                      max_files_per_ip, max_mb_per_ip, max_connections_per_ip,
                      connects_per_minute, publishes_per_hour, messages_per_second
  -h, --help          this text
  -V, --version       the version

Logs go to standard output (RUST_LOG=debug for more); they never contain keys, tokens or file contents.
GET /healthz on the same port answers \"ok\" (for uptime checks).
";

/// How `files list` orders the files.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    /// By id.
    #[default]
    Id,
    /// Biggest first.
    Size,
    /// Oldest first.
    Age,
    /// Longest unused first.
    Idle,
}

/// What to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Serve,
    Key { rotate: bool },
    Files { sort: Sort },
    RemoveFile { file: String },
    Links { file: String },
    CreateLink { file: String, access: Access, label: String },
    Revoke { file: String, link: String },
    Plugins,
    RemovePlugin { id: String, version: Option<String> },
    Reports,
    Check,
    Block { ip: String },
    Unblock { ip: String },
    Blocked,
    Help,
    Version,
}

/// A parsed command line.
#[derive(Clone, Debug, PartialEq)]
pub struct Cli {
    pub config: Config,
    pub command: Command,
}

/// Read the arguments (after the program name).
pub fn parse(args: &[String]) -> Result<Cli, String> {
    // The config file first: flags override it.
    let mut config = match args.iter().position(|a| a == "--config") {
        Some(i) => Config::load(Path::new(args.get(i + 1).ok_or("--config needs a file")?))?,
        None => Config::default(),
    };
    let mut words = Vec::new();
    let mut access = None;
    let mut label = String::new();
    let mut rotate = false;
    let mut version = None;
    let mut sort = Sort::Id;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| it.next().cloned().ok_or_else(|| format!("{name} needs a value"));
        match a.as_str() {
            "-h" | "--help" => return Ok(Cli { config, command: Command::Help }),
            // After `plugins remove ID`: the version to unpublish.
            "--version" if words.first().is_some_and(|w| w == "plugins") => version = Some(value("--version")?),
            "-V" | "--version" => return Ok(Cli { config, command: Command::Version }),
            "--config" => {
                value("--config")?;
            }
            "--listen" => config.listen = value("--listen")?,
            "--data" => config.data = PathBuf::from(value("--data")?),
            "--public-url" => config.public_url = Some(value("--public-url")?),
            "--access" => access = Some(Access::parse(&value("--access")?).ok_or("--access must be edit, comment or view")?),
            "--label" => label = value("--label")?,
            "--rotate" => rotate = true,
            "--sort" => {
                sort = match value("--sort")?.as_str() {
                    "size" => Sort::Size,
                    "age" => Sort::Age,
                    "idle" => Sort::Idle,
                    _ => return Err("--sort must be size, age or idle".into()),
                }
            }
            s if s.starts_with('-') => return Err(format!("unknown option {s}")),
            s => words.push(s.to_string()),
        }
    }
    let w: Vec<&str> = words.iter().map(String::as_str).collect();
    let command = match w.as_slice() {
        [] | ["serve"] => Command::Serve,
        ["key"] => Command::Key { rotate },
        ["files"] | ["files", "list"] => Command::Files { sort },
        ["files", "remove", file] => Command::RemoveFile { file: file.to_string() },
        ["links", "list", file] => Command::Links { file: file.to_string() },
        ["links", "create", file] => {
            Command::CreateLink { file: file.to_string(), access: access.ok_or("links create needs --access edit, comment or view")?, label }
        }
        ["links", "revoke", file, link] => Command::Revoke { file: file.to_string(), link: link.to_string() },
        ["plugins"] | ["plugins", "list"] => Command::Plugins,
        ["plugins", "remove", id] => Command::RemovePlugin { id: id.to_string(), version },
        ["reports"] => Command::Reports,
        ["check"] => Command::Check,
        ["block-ip", ip] => Command::Block { ip: ip.to_string() },
        ["unblock-ip", ip] => Command::Unblock { ip: ip.to_string() },
        ["blocked"] => Command::Blocked,
        _ => return Err(format!("unknown command `{}`", words.join(" "))),
    };
    config.check()?;
    Ok(Cli { config, command })
}

/// What a running server writes into its data directory.
#[derive(Debug, Serialize, Deserialize)]
pub struct Running {
    pub listen: SocketAddr,
    pub pid: u32,
}

/// Note where the server listens (for the command line).
pub fn write_running(data: &Path, listen: SocketAddr) -> Result<(), String> {
    let text = serde_json::to_vec_pretty(&Running { listen, pid: std::process::id() }).map_err(|e| e.to_string())?;
    disk::write_atomic(&data.join(SERVER_JSON), &text).map_err(|e| format!("can't write {SERVER_JSON}: {e}"))
}

/// `addr` as something this machine can connect to (the loopback address for "every interface").
fn reachable(addr: SocketAddr) -> SocketAddr {
    match addr.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => SocketAddr::from(([127, 0, 0, 1], addr.port())),
        IpAddr::V6(ip) if ip.is_unspecified() => SocketAddr::from((std::net::Ipv6Addr::LOCALHOST, addr.port())),
        _ => addr,
    }
}

/// How share links and the publish address name the server: `public_url`, else the listen
/// address (this machine's network address for "every interface").
pub fn public_endpoint(cfg: &Config, listen: SocketAddr) -> Result<Endpoint, String> {
    if let Some(e) = cfg.public()? {
        return Ok(e);
    }
    let ip = if listen.ip().is_unspecified() { figcraft_collab_net::lan_ip().unwrap_or(reachable(listen).ip()) } else { listen.ip() };
    Ok(Endpoint::plain(SocketAddr::new(ip, listen.port()).to_string()))
}

/// Run a command other than serving; what to print.
pub fn run(cli: &Cli) -> Result<String, String> {
    let cfg = &cli.config;
    match &cli.command {
        Command::Help => Ok(USAGE.to_string()),
        Command::Version => Ok(format!("figcraft-server {}\n", env!("CARGO_PKG_VERSION"))),
        Command::Serve => Err("serving runs in the foreground: run figcraft-server without a command".into()),
        Command::Key { rotate } => key(cfg, *rotate),
        Command::Files { sort } => match admin(cfg, Admin::Listing)? {
            AdminReply::Listing(files) => Ok(files_table(files, *sort, store::now())),
            other => unexpected(other),
        },
        // `parse` checked them.
        Command::Check => {
            Ok(format!("The settings are fine{}.\n", if cfg.open_publish { ": open to the public (anyone may publish without the key)" } else { "" }))
        }
        Command::Reports => match admin(cfg, Admin::Reports)? {
            AdminReply::Reports(reports) => Ok(reports_table(&reports)),
            other => unexpected(other),
        },
        Command::Block { ip } => match admin(cfg, Admin::Block { ip: ip.clone() })? {
            AdminReply::Done => Ok(format!("Blocked {ip}: its connections were dropped and new ones are refused.\n")),
            other => unexpected(other),
        },
        Command::Unblock { ip } => match admin(cfg, Admin::Unblock { ip: ip.clone() })? {
            AdminReply::Done => Ok(format!("Unblocked {ip}.\n")),
            other => unexpected(other),
        },
        Command::Blocked => match admin(cfg, Admin::Blocked)? {
            AdminReply::Blocked(list) if list.is_empty() => Ok("No addresses are blocked.\n".into()),
            AdminReply::Blocked(list) => Ok(list.iter().map(|ip| format!("{ip}\n")).collect()),
            other => unexpected(other),
        },
        Command::RemoveFile { file } => match admin(cfg, Admin::Remove { file: file.clone() })? {
            AdminReply::Done => Ok(format!("Removed file {file} and its links.\n")),
            other => unexpected(other),
        },
        Command::Links { file } => match admin(cfg, Admin::Links { file: file.clone() })? {
            AdminReply::Links(links) => Ok(links_table(&links)),
            other => unexpected(other),
        },
        Command::CreateLink { file, access, label } => {
            match admin(cfg, Admin::CreateLink { file: file.clone(), access: *access, label: label.clone() })? {
                AdminReply::Link(l) => Ok(new_link(cfg, &l)?),
                other => unexpected(other),
            }
        }
        Command::Revoke { file, link } => match admin(cfg, Admin::Revoke { file: file.clone(), link: link.clone() })? {
            AdminReply::Done => Ok(format!("Revoked link {link}: it no longer joins, and whoever was in the file with it was disconnected.\n")),
            other => unexpected(other),
        },
        Command::Plugins => match directory(cfg, PluginReq::List { query: String::new() })? {
            PluginReply::List(entries) => Ok(plugins_table(&entries)),
            other => unexpected_plugins(other),
        },
        Command::RemovePlugin { id, version } => match directory(cfg, PluginReq::Unpublish { id: id.clone(), version: version.clone() })? {
            PluginReply::Removed(versions) => Ok(format!("Unpublished {id} {}.\n", versions.join(", "))),
            other => unexpected_plugins(other),
        },
    }
}

fn unexpected_plugins(reply: PluginReply) -> Result<String, String> {
    match reply {
        PluginReply::Failed { reason } => Err(reason),
        _ => Err("the server answered something else".into()),
    }
}

/// Do one plugin directory request with the server key: through the running server, or on the
/// data directory.
fn directory(cfg: &Config, req: PluginReq) -> Result<PluginReply, String> {
    match Store::try_open(&cfg.data, cfg.store())? {
        Some(mut store) => Ok(store.directory().handle(true, req)),
        None => {
            let addr = running(&cfg.data)?;
            let key = std::fs::read_to_string(cfg.data.join(KEY)).map_err(|e| format!("can't read the server key: {e}"))?;
            manage::plugins(&Endpoint::plain(addr.to_string()), key.trim(), req, &cfg.net(), Duration::from_secs(5))
        }
    }
}

fn plugins_table(entries: &[PluginEntry]) -> String {
    if entries.is_empty() {
        return "No plugins yet (publish one with `figcraft-cli plugin publish FOLDER --server ws://host:port/?key=KEY`).\n".into();
    }
    let mut out =
        String::from("ID                                VERSIONS            NAME                  AUTHOR            ACCESS  SIZE       SHA-256\n");
    for e in entries {
        let _ = writeln!(
            out,
            "{:<32}  {:<18}  {:<20}  {:<16}  {:<6}  {:<9}  {}",
            elide(&e.id, 32),
            elide(&e.versions.join(","), 18),
            elide(&e.name, 20),
            elide(&e.author, 16),
            e.access,
            size(e.bytes),
            e.sha256.get(..16).unwrap_or_default()
        );
    }
    out
}

fn unexpected(reply: AdminReply) -> Result<String, String> {
    match reply {
        AdminReply::Failed { reason } => Err(reason),
        other => Err(format!("the server answered {other:?}")),
    }
}

/// The running server's address, when one holds the data directory.
fn running(data: &Path) -> Result<SocketAddr, String> {
    let bytes = disk::read_capped(&data.join(SERVER_JSON), 1 << 16).map_err(|e| format!("a server is using {} but {e}", data.display()))?;
    let r: Running = serde_json::from_slice(&bytes).map_err(|e| format!("{SERVER_JSON} is damaged ({e})"))?;
    Ok(reachable(r.listen))
}

/// Do one request: through the running server, or on the data directory.
fn admin(cfg: &Config, req: Admin) -> Result<AdminReply, String> {
    match Store::try_open(&cfg.data, cfg.store())? {
        // On this machine, one request per command: never over the address's budget.
        Some(mut store) => Ok(store.admin(std::net::Ipv4Addr::LOCALHOST.into(), req, &|_| None).reply),
        None => {
            let addr = running(&cfg.data)?;
            let key = std::fs::read_to_string(cfg.data.join(KEY)).map_err(|e| format!("can't read the server key: {e}"))?;
            let replies = manage::request(&Endpoint::plain(addr.to_string()), key.trim(), vec![req], &cfg.net(), Duration::from_secs(5))?;
            replies.into_iter().next().ok_or_else(|| "the server didn't answer".to_string())
        }
    }
}

/// The address to publish to, with the key.
fn key(cfg: &Config, rotate: bool) -> Result<String, String> {
    let mut store = Store::try_open(&cfg.data, cfg.store())?;
    let listen = match &store {
        Some(_) => cfg.addr()?,
        None => running(&cfg.data)?,
    };
    let key = match (store.as_mut(), rotate) {
        (Some(s), true) => s.rotate_key()?,
        (Some(s), false) => s.key().to_string(),
        (None, true) => return Err("stop the server to replace its key".into()),
        (None, false) => std::fs::read_to_string(cfg.data.join(KEY)).map_err(|e| format!("can't read the server key: {e}"))?.trim().to_string(),
    };
    let public = public_endpoint(cfg, listen)?;
    let mut out = String::new();
    if rotate {
        out.push_str("A new server key: the old one no longer works.\n");
    }
    let _ = writeln!(out, "Publish to this address (in FigCraft: File > Publish to server…):");
    let _ = writeln!(out, "  {}?key={key}", public.url());
    let _ = writeln!(out, "Whoever has it can publish files and manage their links: share it only with people you trust.");
    if cfg.open_publish {
        let _ = writeln!(out, "Open to the public: anyone can publish to {} without the key (each file gets an owner token).", public.url());
    }
    Ok(out)
}

fn new_link(cfg: &Config, l: &NewLink) -> Result<String, String> {
    let listen = match running(&cfg.data) {
        Ok(a) if store::lock(&cfg.data)?.is_none() => a,
        _ => cfg.addr()?,
    };
    let invite = Invite { server: public_endpoint(cfg, listen)?, file: l.link.file.clone(), token: l.token.clone() };
    let mut out = String::new();
    let _ = writeln!(out, "New {} link {} for file {}:", l.link.access.id(), l.link.id, l.link.file);
    let _ = writeln!(out, "  {}", invite.link());
    let _ = writeln!(out, "  {}", invite.url());
    let _ = writeln!(out, "This is the only time the link is shown (the server keeps only a hash of its token).");
    Ok(out)
}

/// When a listed file was last used (made, changed or opened).
fn used(f: &FileRow) -> u64 {
    f.info.created.max(f.info.updated).max(f.opened)
}

fn files_table(mut files: Vec<FileRow>, sort: Sort, now: u64) -> String {
    if files.is_empty() {
        return "No files yet (publish one from FigCraft: File > Publish to server…).\n".into();
    }
    match sort {
        Sort::Id => {}
        Sort::Size => files.sort_by_key(|f| std::cmp::Reverse(f.info.bytes)),
        Sort::Age => files.sort_by_key(|f| f.info.created),
        Sort::Idle => files.sort_by_key(used),
    }
    let mut out = String::from("ID                NAME                            PEOPLE  SIZE       CREATED     IDLE  EXPIRES     REPORTS  BY\n");
    for f in &files {
        let idle = format!("{}d", now.saturating_sub(used(f)) / 86_400);
        let expires = f.expires.map_or_else(|| "never".to_string(), day);
        let by = if f.open { f.ip.as_str() } else { "key" };
        let _ = writeln!(
            out,
            "{:<16}  {:<30}  {:>6}  {:<9}  {:<10}  {:>4}  {:<10}  {:>7}  {}",
            f.info.id,
            elide(&f.info.name, 30),
            f.info.people,
            size(f.info.bytes),
            day(f.info.created),
            idle,
            expires,
            f.reports,
            by
        );
    }
    out
}

fn reports_table(reports: &[ReportInfo]) -> String {
    if reports.is_empty() {
        return "No reports.\n".into();
    }
    let mut out = String::new();
    for r in reports {
        let _ = writeln!(out, "{}  file {} ({})  from {}  link {}", date(r.at), r.file, elide(&r.name, 40), r.ip, r.link);
        let _ = writeln!(out, "    {}", r.reason);
    }
    out
}

/// `YYYY-MM-DD` for seconds since 1970.
fn day(secs: u64) -> String {
    date(secs).chars().take(10).collect()
}

fn links_table(links: &[LinkInfo]) -> String {
    if links.is_empty() {
        return "No links (make one with `figcraft-server links create FILE --access view`).\n".into();
    }
    let mut out = String::from("LINK              ACCESS  CREATED               LABEL\n");
    for l in links {
        let _ = writeln!(out, "{:<16}  {:<6}  {:<20}  {}", l.id, l.access.id(), date(l.created), l.label);
    }
    out
}

fn elide(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { format!("{}…", s.chars().take(n - 1).collect::<String>()) }
}

fn size(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 20 => format!("{:.1} MiB", b as f64 / f64::from(1 << 20)),
        b if b >= 1 << 10 => format!("{:.1} KiB", b as f64 / 1024.0),
        b => format!("{b} B"),
    }
}

/// `YYYY-MM-DD HH:MM UTC` for seconds since 1970.
pub use figcraft_collab::message::utc_date as date;
