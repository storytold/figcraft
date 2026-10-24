//! Fonts a document names that this machine doesn't have (#752): text layers, their styled
//! ranges, text styles and the string variables bound to a font family. Text in such a family
//! is drawn in a fallback font, so the user is told which ones are missing. Once such a font is
//! there, [`refit`] lets auto-sizing text measure its box in it again.
//!
//! Free ones can be downloaded on request (#754) from the Google Fonts library on GitHub
//! ([`LIBRARY`]): nothing is sent until a command passes `confirm: true`. Each family's
//! `METADATA.pb` names its licence and font files; every file is downloaded before anything is
//! kept (the UI runs the command in the background, which runs it again from the start for each
//! answer), then each must be a font of the family asked for. They are saved with their licence
//! in the user's font folder ([`Session::fonts_dir`]) and drawn at once.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use figcraft_doc::{Document, NodeId, NodeKind, StyleKind, VariableValue, Variables};
use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::live::{NetReply, NetRequest};
use crate::params::{get, req_targets};
use crate::{EngineError, Result, Session, bad, spec};

/// Why fonts can't be downloaded here (the browser build, sessions without the network).
pub const NO_DOWNLOADS: &str = "downloading fonts needs the desktop app or the MCP server";
/// Where fonts are downloaded from: a copy of the google/fonts repository's layout (licence
/// folder, then the family's folder). A build can name another with `FIGCRAFT_FONT_LIBRARY`, or
/// leave downloads out with an empty one.
pub const LIBRARY: &str = match option_env!("FIGCRAFT_FONT_LIBRARY") {
    Some(s) => s,
    None => "https://raw.githubusercontent.com/google/fonts/main/",
};
/// The library's licence folders: the licence `METADATA.pb` names, and the file holding its text.
const LICENCES: [(&str, &str, &str); 3] = [("ofl", "OFL", "OFL.txt"), ("apache", "APACHE2", "LICENSE.txt"), ("ufl", "UFL", "UFL.txt")];
const MAX_METADATA: usize = 1 << 20;
const MAX_FILE: usize = 64 << 20;
/// All files of one download together.
const MAX_TOTAL: usize = 256 << 20;
const MAX_FILES: usize = 32;
const MAX_FAMILIES: usize = 32;
/// The folder downloaded fonts go in, inside the user's font folder.
pub const FOLDER: &str = "FigCraft";

/// This build can download fonts.
pub fn downloads_enabled() -> bool {
    usable(LIBRARY)
}

/// Fonts are fetched over HTTPS only, so a library at any other address can't be used.
fn usable(library: &str) -> bool {
    library.trim().strip_prefix("https://").is_some_and(|rest| !rest.is_empty())
}

/// A family the document names that no installed or built-in font provides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Missing {
    /// As the document spells it.
    pub family: String,
    /// Text layers that use it (0: only a text style or a variable names it).
    pub layers: usize,
}

/// The families `doc` names that the font database doesn't have, by name. On the desktop the
/// installed fonts are indexed in the background: ask once that's done
/// (`figcraft_text::installed_fonts_known`), or every installed family counts as missing.
pub fn missing(doc: &Document) -> Vec<Missing> {
    // Lower-case family → (spelling, layers).
    let mut used: BTreeMap<String, (String, usize)> = BTreeMap::new();
    let mut add = |family: &str, layer: bool| {
        let family = family.trim();
        if family.is_empty() {
            return;
        }
        let e = used.entry(family.to_lowercase()).or_insert_with(|| (family.to_string(), 0));
        e.1 += usize::from(layer);
    };
    for n in doc.nodes() {
        if let NodeKind::Text(t) = &n.kind {
            let mut families: Vec<&str> = t.spans().into_iter().map(|(_, _, st)| st.font_family.as_str()).collect();
            families.push(&t.style.font_family);
            families.sort_by_key(|f| f.trim().to_lowercase());
            families.dedup_by_key(|f| f.trim().to_lowercase());
            for f in families {
                add(f, true);
            }
        }
        if let Some(var) = n.bound.get("fontFamily") {
            for f in string_values(&doc.variables, var) {
                add(&f, false);
            }
        }
    }
    for st in &doc.styles {
        if let StyleKind::Text { style } = &st.kind {
            add(&style.font_family, false);
        }
        if let Some(var) = st.bound.get("fontFamily") {
            for f in string_values(&doc.variables, var) {
                add(&f, false);
            }
        }
    }
    used.into_values().filter(|(family, _)| !figcraft_text::has_family(family)).map(|(family, layers)| Missing { family, layers }).collect()
}

/// Every string value of variable `id` in any mode, following aliases (at most 32 deep).
fn string_values(vars: &Variables, id: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut todo = vec![(id.to_string(), 0)];
    while let Some((id, depth)) = todo.pop() {
        let Some(var) = vars.get(&id) else { continue };
        for v in var.values.values() {
            match v {
                VariableValue::String(s) => out.push(s.clone()),
                VariableValue::Alias(a) if depth < 32 => todo.push((a.clone(), depth + 1)),
                _ => {}
            }
        }
    }
    out
}

/// Auto-sizing text layers measure their boxes again: a box measured while its family was missing
/// (in a fallback font) follows the real face once that's installed or loaded. Only layers under
/// `under` (every layer: `None`) that use one of `families` (any family: `None`). Returns the
/// layers whose size changed.
pub fn refit(doc: &mut Document, under: Option<&[NodeId]>, families: Option<&[String]>) -> Vec<NodeId> {
    let want: Option<BTreeSet<String>> = families.map(|f| f.iter().map(|x| x.trim().to_lowercase()).collect());
    let scope: Option<BTreeSet<NodeId>> = under.map(|ids| ids.iter().flat_map(|i| doc.subtree(*i)).collect());
    let ids: Vec<NodeId> = doc
        .nodes()
        .filter(|n| scope.as_ref().is_none_or(|s| s.contains(&n.id)))
        .filter(|n| {
            let NodeKind::Text(t) = &n.kind else { return false };
            let uses = |w: &BTreeSet<String>| {
                w.contains(&t.style.font_family.trim().to_lowercase())
                    || t.spans().iter().any(|(_, _, st)| w.contains(&st.font_family.trim().to_lowercase()))
            };
            want.as_ref().is_none_or(uses)
                && figcraft_text::auto_size(n).is_some_and(|(w, h)| (w - n.width).abs() > 1e-6 || (h - n.height).abs() > 1e-6)
        })
        .map(|n| n.id)
        .collect();
    if !ids.is_empty() {
        let before = doc.clone();
        doc.mark_dirty(ids.iter().copied());
        crate::normalize(doc, &before);
    }
    ids
}

/// [`refit`] in the open document, if there is one.
pub(crate) fn refit_open(s: &mut Session, under: Option<&[NodeId]>, families: Option<&[String]>) -> Vec<NodeId> {
    match s.doc_mut() {
        Ok(doc) => refit(doc, under, families),
        Err(_) => Vec::new(),
    }
}

/// `text.refit`: the layers `ids` names (an `ids` that isn't a list of layers is refused), else the
/// whole document.
fn refit_text(s: &mut Session, p: &Value) -> Result<Value> {
    let under = match get(p, "ids") {
        None | Some(Value::Null) => None,
        Some(_) => Some(req_targets("text.refit", s, p)?),
    };
    s.doc()?;
    Ok(json!({"refitted": refit_open(s, under.as_deref(), None)}))
}

fn missing_fonts(s: &mut Session, _: &Value) -> Result<Value> {
    let families: Vec<Value> = missing(s.doc()?).into_iter().map(|m| json!({"family": m.family, "layers": m.layers})).collect();
    s.request_ui("dialog.missingFonts", json!({}));
    Ok(json!({"families": families}))
}

/// A family's entry in the library.
#[derive(Debug, PartialEq, Eq)]
struct Metadata {
    name: String,
    license: String,
    files: Vec<String>,
}

/// The family's folder name in the library: its name in lower case, letters and digits only.
fn folder_name(family: &str) -> String {
    family.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect()
}

/// The parts of a `METADATA.pb` (protobuf text format) used here: the family's `name` and
/// `license` (top-level fields) and each font's `filename`.
fn parse_metadata(text: &str) -> Option<Metadata> {
    let value = |line: &str, key: &str| -> Option<String> {
        let v = line.trim().strip_prefix(key)?.trim().strip_prefix('"')?.strip_suffix('"')?;
        (!v.contains(['\\', '"'])).then(|| v.to_string())
    };
    let (mut name, mut license, mut files) = (None, None, Vec::new());
    for line in text.lines() {
        let top = !line.starts_with([' ', '\t']);
        if top && name.is_none() {
            name = value(line, "name:");
        }
        if top && license.is_none() {
            license = value(line, "license:");
        }
        // A file name that can't be read refuses the entry: skipping it would install part of the family.
        if line.trim().starts_with("filename:") {
            files.push(value(line, "filename:")?);
        }
    }
    Some(Metadata { name: name?, license: license?, files })
}

/// A file name the library may give: a plain `.ttf` or `.otf` name, never a path.
fn plain_font_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !name.is_empty()
        && name.len() <= 255
        && !name.starts_with('.')
        && !name.contains(['/', '\\', ':', '\0'])
        && !windows_device(name)
        && (lower.ends_with(".ttf") || lower.ends_with(".otf"))
}

/// A name Windows keeps for a device (`CON`, `NUL.ttf`, `com1.otf`…), whatever follows the first dot.
fn windows_device(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or_default().trim_end().to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.ends_with(|c: char| c.is_ascii_digit() && c != '0'))
}

/// `name` as a URL path segment (variable fonts are named like `Inter[opsz,wght].ttf`).
fn url_segment(name: &str) -> String {
    name.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => char::from(b).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// One download: `None` when the library has no such file.
fn fetch(s: &mut Session, url: String, max: usize) -> Result<Option<Arc<Vec<u8>>>> {
    match s.net(NetRequest::Fetch { url, max })?.0 {
        NetReply::Fetch(Ok(Some(b))) if b.len() > max => Err(EngineError::Other(format!("a file is larger than {max} bytes"))),
        NetReply::Fetch(r) => r.map_err(EngineError::Other),
        _ => Err(EngineError::Other("the server answered something else".into())),
    }
}

/// A family's files, downloaded and not yet checked.
struct Download {
    family: String,
    /// The family's name as the library spells it: its folder on disk, whatever spelling was asked.
    name: String,
    license: String,
    files: Vec<(String, Arc<Vec<u8>>)>,
    licence_file: (String, Arc<Vec<u8>>),
}

/// Download `family` from the library: `Ok(Err(why))` when it can't be had from there. `total`
/// counts the bytes of the whole command.
fn download(s: &mut Session, family: &str, total: &mut usize) -> Result<std::result::Result<Download, String>> {
    let folder = folder_name(family);
    let not_there = || Ok(Err("it isn't in the Google Fonts library".to_string()));
    if folder.is_empty() {
        return not_there();
    }
    let too_big = || Ok(Err(format!("the download is larger than {} MiB", MAX_TOTAL >> 20)));
    for (licence_folder, licence, licence_name) in LICENCES {
        let base = format!("{}/{licence_folder}/{folder}/", LIBRARY.trim().trim_end_matches('/'));
        let Some(meta) = fetch(s, format!("{base}METADATA.pb"), MAX_METADATA)? else { continue };
        *total += meta.len();
        if *total > MAX_TOTAL {
            return too_big();
        }
        let Some(meta) = std::str::from_utf8(&meta).ok().and_then(parse_metadata) else {
            return Ok(Err("the library's entry for it can't be read".into()));
        };
        // Another family in a folder of the same name.
        if !meta.name.eq_ignore_ascii_case(family.trim()) {
            continue;
        }
        if meta.license != licence {
            return Ok(Err(format!("its licence ({}) isn't one FigCraft downloads", meta.license)));
        }
        if meta.files.is_empty() || meta.files.len() > MAX_FILES || !meta.files.iter().all(|f| plain_font_file(f)) {
            return Ok(Err("the library's entry for it lists no font files FigCraft can use".into()));
        }
        let mut files = Vec::new();
        for name in meta.files {
            let Some(bytes) = fetch(s, format!("{base}{}", url_segment(&name)), MAX_FILE)? else {
                return Ok(Err(format!("{name} is missing from the library")));
            };
            *total += bytes.len();
            if *total > MAX_TOTAL {
                return too_big();
            }
            files.push((name, bytes));
        }
        let Some(text) = fetch(s, format!("{base}{licence_name}"), MAX_METADATA)? else {
            return Ok(Err("its licence text is missing from the library".into()));
        };
        *total += text.len();
        if *total > MAX_TOTAL {
            return too_big();
        }
        return Ok(Ok(Download {
            family: family.trim().to_string(),
            name: meta.name,
            license: meta.license,
            files,
            licence_file: (licence_name.into(), text),
        }));
    }
    not_there()
}

/// Every font file of `d` holds faces of its family (as the font database names them, so text in
/// that family finds them).
fn check(d: &Download) -> std::result::Result<(), String> {
    for (name, bytes) in &d.files {
        let faces = figcraft_text::FontDb::default().add_bytes(bytes.to_vec());
        if faces.is_empty() {
            return Err(format!("{name} isn't a font FigCraft can read"));
        }
        if let Some(other) = faces.iter().find(|f| !f.family.eq_ignore_ascii_case(&d.family)) {
            return Err(format!("{name} is a font of {}", other.family));
        }
    }
    Ok(())
}

/// The family's folder name on disk: letters, digits, spaces, `-` and `_`.
fn safe_folder(family: &str) -> String {
    let name: String = family.chars().map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') { c } else { '_' }).take(100).collect();
    // Windows drops trailing spaces and refuses device names.
    let name = name.trim_end();
    if windows_device(name) { format!("_{name}") } else { name.to_string() }
}

/// Put right what a save that stopped half-way left in `root`'s FigCraft folder: a partial folder
/// goes (the font scan would index its files), and an earlier download whose replacement never
/// landed goes back in place. The app runs it before scanning fonts; every save runs it too.
// shortcut: two processes saving into one folder at once can undo each other's half-done save (it
// then fails; no family is lost). A lock file would fix it if the MCP server and the app share one.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::disallowed_methods)]
pub fn tidy(root: &std::path::Path) {
    tidy_with(root, &|from, to| std::fs::rename(from, to));
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::disallowed_methods)]
fn tidy_with(root: &std::path::Path, rename: &dyn Fn(&std::path::Path, &std::path::Path) -> std::io::Result<()>) {
    use std::fs;
    let parent = root.join(FOLDER);
    for e in fs::read_dir(&parent).into_iter().flatten().flatten() {
        if !e.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let file = e.file_name().to_string_lossy().into_owned();
        // Only the hidden `.<family>.partial` folders a save makes; a user's `Backup.partial` stays.
        if file.starts_with('.') && file.ends_with(".partial") {
            let _ = fs::remove_dir_all(e.path());
        } else if let Some(family) = file.strip_prefix('.').and_then(|f| f.strip_suffix(".previous")) {
            let home = parent.join(family);
            if home.exists() {
                let _ = fs::remove_dir_all(e.path());
            } else {
                let _ = rename(&e.path(), &home);
            }
        }
    }
}

/// Keep `d`'s files and licence in `root/FigCraft/<family>/` (replacing an earlier download); the
/// folder. Written to `.<family>.partial` first; an earlier download is moved to
/// `.<family>.previous` and only deleted once the new folder is in place, so a failure or a stop at
/// any point keeps the earlier fonts.
fn save(root: &std::path::Path, d: &Download) -> std::result::Result<String, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        #[allow(clippy::disallowed_methods)]
        save_with(root, d, &|from, to| std::fs::rename(from, to))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (root, d);
        Err(NO_DOWNLOADS.into())
    }
}

/// [`save`] with the folder rename passed in, so tests can make it fail.
// The fonts folder is the app's choice (set by the desktop app, never by a command), so automation
// roots don't apply to it.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::disallowed_methods)]
fn save_with(
    root: &std::path::Path,
    d: &Download,
    rename: &dyn Fn(&std::path::Path, &std::path::Path) -> std::io::Result<()>,
) -> std::result::Result<String, String> {
    use std::fs;
    let parent = root.join(FOLDER);
    let name = safe_folder(&d.name);
    let folder = parent.join(&name);
    let partial = parent.join(format!(".{name}.partial"));
    let previous = parent.join(format!(".{name}.previous"));
    tidy_with(root, rename);
    let write = || -> std::io::Result<()> {
        fs::create_dir_all(&partial)?;
        for (file, bytes) in d.files.iter().chain(std::iter::once(&d.licence_file)) {
            fs::write(partial.join(file), bytes.as_slice())?;
        }
        let had = folder.exists();
        if had {
            rename(&folder, &previous)?;
        }
        if let Err(e) = rename(&partial, &folder) {
            if had {
                let _ = rename(&previous, &folder);
            }
            return Err(e);
        }
        if had {
            let _ = fs::remove_dir_all(&previous);
        }
        Ok(())
    };
    if let Err(e) = write() {
        let _ = fs::remove_dir_all(&partial);
        return Err(format!("it couldn't be saved in {}: {e}", parent.display()));
    }
    Ok(folder.to_string_lossy().into_owned())
}

fn download_fonts(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "text.downloadFonts";
    if !downloads_enabled() {
        return Err(EngineError::Disabled(cmd.into(), "this build of FigCraft leaves font downloads out".into()));
    }
    if get(p, "confirm") != Some(&Value::Bool(true)) {
        return Err(bad(
            cmd,
            "downloading sends the font names to GitHub (the Google Fonts library) and saves the fonts on this computer: pass `confirm: true`",
        ));
    }
    if s.live.link().is_none() {
        return Err(EngineError::Other(NO_DOWNLOADS.into()));
    }
    let asked: Vec<String> = match get(p, "families") {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(|f| f.trim().to_string()).filter(|f| !f.is_empty()).collect(),
        Some(_) => return Err(bad(cmd, "`families` is a list of font family names")),
        None => missing(s.doc()?).into_iter().map(|m| m.family).collect(),
    };
    let (mut families, mut failed, mut seen): (Vec<String>, Vec<Value>, Vec<String>) = (Vec::new(), Vec::new(), Vec::new());
    for f in asked {
        if seen.iter().any(|g| g.eq_ignore_ascii_case(&f)) {
            continue;
        }
        seen.push(f.clone());
        if figcraft_text::has_family(&f) {
            failed.push(json!({"family": f, "reason": "it's already installed"}));
        } else {
            families.push(f);
        }
    }
    let (mut downloads, mut total) = (Vec::new(), 0);
    for family in families.split_off(families.len().min(MAX_FAMILIES)) {
        failed.push(json!({"family": family, "reason": format!("at most {MAX_FAMILIES} families are downloaded at a time")}));
    }
    // Everything is downloaded before anything is kept: a run stopped to wait for an answer
    // changes nothing.
    for family in families {
        match download(s, &family, &mut total) {
            Ok(Ok(d)) => downloads.push(d),
            Ok(Err(why)) => failed.push(json!({"family": family, "reason": why})),
            Err(EngineError::Waiting) => return Err(EngineError::Waiting),
            Err(e) => failed.push(json!({"family": family, "reason": e.to_string()})),
        }
    }
    let (mut installed, mut got) = (Vec::new(), Vec::new());
    for d in downloads {
        let kept = check(&d).and_then(|()| s.fonts_dir.as_deref().map(|root| save(root, &d)).transpose());
        let path = match kept {
            Ok(path) => path,
            Err(why) => {
                failed.push(json!({"family": d.family, "reason": why}));
                continue;
            }
        };
        for (_, bytes) in &d.files {
            figcraft_text::load_font_bytes(bytes.to_vec());
        }
        let files: Vec<&str> = d.files.iter().map(|(n, _)| n.as_str()).collect();
        installed.push(json!({"family": d.family, "license": d.license, "files": files, "path": path}));
        got.push(d.family);
    }
    let refitted = if got.is_empty() { Vec::new() } else { refit_open(s, None, Some(&got)) };
    Ok(json!({"installed": installed, "failed": failed, "refitted": refitted}))
}

/// The font commands: `text.missingFonts`, `text.refit` and `text.downloadFonts`.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "text.missingFonts",
            "Missing fonts",
            "{}  fonts the open document names that aren't installed (text layers, styled ranges, text styles, font family variables); the app shows them → {families: [{family, layers}]}",
            false,
            missing_fonts
        ),
        spec!(
            "text.refit",
            "Fit text to fonts",
            "{ids? (these layers and the text in them; default: the whole document)}  auto-sizing text measures its box again, e.g. a box measured in a fallback font while its family was missing → {refitted: [ids whose size changed]}",
            true,
            refit_text
        ),
        spec!(
            "text.downloadFonts",
            "Download missing fonts",
            "{families? (default: the open document's missing fonts), confirm: true (required: sends the font names to GitHub)}  downloads free fonts from the Google Fonts library (OFL, Apache 2.0 or UFL) into the user's font folder, with their licences, and draws text in them at once; the open document's auto-sizing text in them fits them (text.refit) → {installed: [{family, license, files, path}], failed: [{family, reason}], refitted: [ids]}",
            true,
            download_fonts
        ),
    ]
}

#[cfg(test)]
#[path = "fonts_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "fonts_download_tests.rs"]
mod download_tests;
