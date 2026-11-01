//! The command line: `figcraft [--control <port>] [--automation-read-root <dir>]…
//! [--automation-write-root <dir>]… [files…] [figcraft://join?…]`. An invite link
//! (what the system passes when a `figcraft://` link is opened) joins that session once the window
//! is up; everything else that isn't a flag is a file to open, as a path or a `file://` URL (the
//! Linux desktop entry passes URLs, `%U`, so that it can also hand over invite links). Any other
//! `--flag` is unknown and stops the app at startup (#836): a mistyped root flag must never leave
//! the app running unconfined. After a bare `--`, every argument is a file.

/// What the app was started with.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Args {
    /// `--control <port>` (or `--control=<port>`): the JSON-lines control server.
    pub control: Option<u16>,
    pub files: Vec<String>,
    /// The last invite link given.
    pub join: Option<String>,
    /// `--automation-read-root <dir>` / `--automation-write-root <dir>` (each repeatable; `=` works
    /// too): the folders the app's file access is confined to. A flag without a value gives `""`,
    /// which is refused at startup.
    pub read_roots: Vec<String>,
    pub write_roots: Vec<String>,
    /// `--flags` the app doesn't know, as given.
    pub unknown: Vec<String>,
}

/// An invite link rather than a file (`figcraft:` links, in any case).
pub fn is_invite(arg: &str) -> bool {
    arg.get(..9).is_some_and(|s| s.eq_ignore_ascii_case("figcraft:"))
}

/// The path a local `file://` URL names (`%XX` escapes decoded); `None` for anything else.
pub fn file_url_path(arg: &str) -> Option<String> {
    let rest = arg.get(..7).filter(|s| s.eq_ignore_ascii_case("file://")).and(arg.get(7..))?;
    let path = rest.strip_prefix("localhost").unwrap_or(rest);
    if !path.starts_with('/') {
        return None;
    }
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b);
            i += 1;
        }
    }
    let path = String::from_utf8(out).ok()?;
    // `file:///C:/…` on Windows.
    let drive = matches!(path.as_bytes().get(1..3), Some([l, b':']) if l.is_ascii_alphabetic());
    Some(if cfg!(windows) && drive { path.get(1..).unwrap_or_default().to_string() } else { path })
}

/// Read the arguments after the program name.
pub fn parse(args: &[String]) -> Args {
    let mut out = Args::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--" {
            out.files.extend(it.by_ref().cloned());
            break;
        }
        let (flag, inline) = a.split_once('=').map_or((a.as_str(), None), |(f, v)| (f, Some(v.to_string())));
        let roots = match flag {
            "--automation-read-root" => Some(&mut out.read_roots),
            "--automation-write-root" => Some(&mut out.write_roots),
            _ => None,
        };
        if let Some(roots) = roots {
            roots.push(inline.or_else(|| it.next().cloned()).unwrap_or_default());
        } else if flag == "--control" {
            out.control = inline.or_else(|| it.next().cloned()).and_then(|p| p.parse().ok());
        } else if is_invite(a) {
            out.join = Some(a.clone());
        } else if let Some(path) = file_url_path(a) {
            out.files.push(path);
        } else if a.starts_with("--") {
            out.unknown.push(a.clone());
        } else {
            out.files.push(a.clone());
        }
    }
    out
}
