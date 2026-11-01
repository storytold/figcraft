//! Plugins from a server's plugin directory (`figcraft-server`): listing it, installing and updating
//! plugins from it, and publishing and unpublishing them with the server's key. See
//! `docs/plugins.md` and `docs/server.md`.
//!
//! Everything a server sends is untrusted. An install needs the user's say-so (`confirm: true`,
//! which the app passes only from its confirmation step; nothing here opens a dialog), and the
//! download must match the SHA-256 the directory lists before anything else is read: then the
//! bundle is decoded within a size cap, its manifest and module are checked like an imported
//! plugin's (the module compiled, never run) and it runs in the same sandbox with its manifest's
//! permissions. Nothing here reaches the network except these commands (the UI runs them without
//! waiting: [`crate::background`]).

use std::cmp::Ordering;

pub use figcraft_collab::message::PluginEntry;
use figcraft_collab::message::{PluginReply, PluginReq};
pub use figcraft_plugin::bundle;
pub use figcraft_plugin::bundle::newer as newer_version;
use figcraft_plugin::bundle::{Asset, Bundle, MAX_ASSETS, check_asset_name, compare_versions, newer, sha256_hex};
use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::live::{NetReply, NetRequest};
use crate::params::{get, string};
use crate::plugins::{InstalledPlugin, MANIFEST_FILE, Manifest, installed, register};
use crate::{EngineError, Result, Session, bad, spec};

/// Why there's no plugin directory here (the browser build, sessions without the network).
pub const NO_DIRECTORY: &str = "plugin directories on a server need the desktop app, figcraft-cli or the MCP server";
/// Largest bundle downloaded or published (a server's own cap is at most this).
pub const MAX_BUNDLE: usize = 32 << 20;
/// The module's file name in an installed plugin's folder, and the folder its assets go in.
const MODULE: &str = "module.wasm";
const ASSETS: &str = "assets";

/// A directory as it was listed last.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Listing {
    /// Its address (`ws://host:port/`, without a key).
    pub server: String,
    pub entries: Vec<PluginEntry>,
}

/// A server address without its query (the key), as it's shown and remembered.
pub fn address(server: &str) -> String {
    server.trim().split('?').next().unwrap_or_default().to_string()
}

/// The installed version of plugin `id`, if it is installed.
pub fn installed_version<'a>(s: &'a Session, id: &str) -> Option<&'a str> {
    s.prefs.plugins.iter().find(|p| p.manifest.id == id).map(|p| p.manifest.version.as_str())
}

/// Entry `e` with whether it's installed (its version) and whether that's older.
fn describe(s: &Session, e: &PluginEntry) -> Value {
    let mut v = serde_json::to_value(e).unwrap_or_else(|_| json!({"id": e.id}));
    let have = installed_version(s, &e.id);
    v["installed"] = json!(have);
    v["update"] = json!(have.is_some_and(|h| newer(&e.version, h)));
    v
}

/// `server`, or the directory browsed last.
fn server(s: &Session, cmd: &str, p: &Value) -> Result<String> {
    string(p, "server")
        .filter(|v| !v.trim().is_empty())
        .or_else(|| s.prefs.plugin_directory.clone())
        .ok_or_else(|| bad(cmd, "`server` required: the server's address, like ws://host:47900/"))
}

/// One request to the directory at `server`; a refusal is an error.
fn request(s: &mut Session, server: &str, req: PluginReq) -> Result<PluginReply> {
    match s.net(NetRequest::Plugins { server: server.to_string(), req })?.0 {
        NetReply::Plugins(Ok(PluginReply::Failed { reason }) | Err(reason)) => Err(EngineError::Other(reason)),
        NetReply::Plugins(Ok(r)) => Ok(r),
        _ => Err(EngineError::Other("the server answered something else".into())),
    }
}

fn unexpected(cmd: &str) -> EngineError {
    EngineError::Other(format!("{cmd}: the server answered something else"))
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "plugin.directory.list";
    let server = server(s, cmd, p)?;
    let query = string(p, "query").unwrap_or_default();
    let PluginReply::List(entries) = request(s, &server, PluginReq::List { query })? else { return Err(unexpected(cmd)) };
    let server = address(&server);
    let plugins: Vec<Value> = entries.iter().map(|e| describe(s, e)).collect();
    s.prefs.plugin_directory = Some(server.clone());
    s.plugins.directory = Some(Listing { server: server.clone(), entries });
    Ok(json!({"server": server, "plugins": plugins}))
}

/// Download, check and install a plugin (see the module docs).
fn install(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "plugin.directory.install";
    let id = string(p, "id").ok_or_else(|| bad(cmd, "`id` required (see plugin.directory.list)"))?;
    if get(p, "confirm") != Some(&Value::Bool(true)) {
        return Err(bad(
            cmd,
            "installing a plugin runs its code on this machine: check the permissions it asks for (plugin.directory.list) and pass `confirm: true`",
        ));
    }
    let server = server(s, cmd, p)?;
    let version = string(p, "version");
    let PluginReply::Bundle { entry, bundle: bytes } = request(s, &server, PluginReq::Fetch { id: id.clone(), version: version.clone() })? else {
        return Err(unexpected(cmd));
    };
    let fail = |why: &str| EngineError::Other(format!("{id} wasn't installed: {why}"));
    if entry.id != id || version.as_deref().is_some_and(|v| compare_versions(&entry.version, v) != Some(Ordering::Equal)) {
        return Err(fail("the server sent another plugin than the one asked for"));
    }
    if bytes.len() > MAX_BUNDLE {
        return Err(fail("the download is too large"));
    }
    let sha = sha256_hex(&bytes);
    if sha != entry.sha256 {
        return Err(fail("the download doesn't match the SHA-256 the directory lists"));
    }
    if string(p, "sha256").is_some_and(|want| !want.trim().eq_ignore_ascii_case(&sha)) {
        return Err(fail("the download isn't the version that was shown (its SHA-256 differs)"));
    }
    let bundle = Bundle::decode(&bytes, MAX_BUNDLE).map_err(|e| fail(&e))?;
    let mut manifest = bundle.check().map_err(|e| fail(&e))?;
    if manifest.id != entry.id || manifest.version != entry.version {
        return Err(fail("its manifest doesn't match the directory's entry"));
    }
    let source = address(&server);
    let path = save(s, &mut manifest, &bundle).map_err(|e| fail(&e))?;
    let out = json!({
        "installed": manifest.id,
        "name": manifest.name,
        "version": manifest.version,
        "permissions": manifest.permissions,
        "sha256": sha,
        "path": path,
        "source": source,
    });
    let replaced = register(s, InstalledPlugin { manifest, path, source: Some(source) }, bundle.wasm);
    let mut out = out;
    out["replaced"] = json!(replaced);
    Ok(out)
}

/// Keep an installed plugin's files in its own folder under the install folder (named by the hash
/// of its id, so no id reaches the file system), its module renamed; the manifest's path. Without
/// an install folder (the browser, the CLI and tests) the plugin lasts for the session.
// The install folder is the app's own (set by the desktop app, never by a command), so automation
// roots don't apply to it.
#[allow(clippy::disallowed_methods)]
fn save(s: &Session, m: &mut Manifest, b: &Bundle) -> std::result::Result<Option<String>, String> {
    let Some(root) = s.plugins.install_dir.clone() else { return Ok(None) };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::fs;
        let hash = sha256_hex(m.id.as_bytes());
        let folder = root.join(hash.get(..32).unwrap_or(&hash));
        let partial = folder.with_extension("partial");
        let _ = fs::remove_dir_all(&partial);
        m.main = MODULE.into();
        let write = || -> std::io::Result<()> {
            fs::create_dir_all(&partial)?;
            fs::write(partial.join(MODULE), &b.wasm)?;
            if !b.assets.is_empty() {
                fs::create_dir_all(partial.join(ASSETS))?;
                for a in &b.assets {
                    // Checked plain names (`Bundle::check`): never a path.
                    fs::write(partial.join(ASSETS).join(&a.name), &a.data)?;
                }
            }
            fs::write(partial.join(MANIFEST_FILE), serde_json::to_vec_pretty(&*m).map_err(std::io::Error::other)?)
        };
        let done = write().and_then(|()| {
            if folder.exists() {
                fs::remove_dir_all(&folder)?;
            }
            fs::rename(&partial, &folder)
        });
        if let Err(e) = done {
            let _ = fs::remove_dir_all(&partial);
            return Err(format!("couldn't write it to {}: {e}", root.display()));
        }
        Ok(Some(folder.join(MANIFEST_FILE).to_string_lossy().into_owned()))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (root, m, b);
        Ok(None)
    }
}

/// Delete the folder of a plugin installed from a directory (only one inside the install folder).
#[allow(clippy::disallowed_methods)]
pub(crate) fn delete_installed(s: &Session, p: &InstalledPlugin) {
    #[cfg(not(target_arch = "wasm32"))]
    if let (Some(_), Some(path), Some(root)) = (&p.source, &p.path, &s.plugins.install_dir)
        && let Some(folder) = std::path::Path::new(path).parent()
        && folder.parent() == Some(root.as_path())
        && let Err(e) = std::fs::remove_dir_all(folder)
    {
        log::warn!("plugins: couldn't delete {}: {e}", folder.display());
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (s, p);
}

/// The bundle of the plugin folder (or manifest) at `path`: its manifest as written, its module and
/// the files in its `assets` folder.
#[cfg(not(target_arch = "wasm32"))]
fn bundle_from_disk(cmd: &str, path: &str) -> Result<Bundle> {
    use crate::plugins::{module_file, read_file};
    let mut file = std::path::PathBuf::from(crate::library::absolute_path(path.trim()));
    if crate::file_access::is_dir(&file) {
        file.push(MANIFEST_FILE);
    }
    let text =
        String::from_utf8(read_file(&file, figcraft_plugin::manifest::MAX_MANIFEST_BYTES)?).map_err(|_| bad(cmd, "the manifest isn't UTF-8 text"))?;
    let m = Manifest::parse(&text).map_err(|e| bad(cmd, e))?;
    let wasm = module_file(&file, &m)?;
    let mut assets = Vec::new();
    let dir = file.parent().map(|d| d.join(ASSETS));
    if let Some(entries) = dir.as_ref().and_then(|d| crate::file_access::read_dir(d).ok()) {
        let mut names: Vec<(String, std::path::PathBuf)> = entries
            .into_iter()
            .filter(|p| crate::file_access::metadata(p).is_ok_and(|m| m.is_file()))
            .map(|p| (p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), p))
            .collect();
        names.sort();
        if names.len() > MAX_ASSETS {
            return Err(bad(cmd, format!("a plugin's assets folder holds at most {MAX_ASSETS} files")));
        }
        for (name, path) in names {
            check_asset_name(&name).map_err(|e| bad(cmd, e))?;
            assets.push(Asset { name, data: read_file(&path, MAX_BUNDLE)? });
        }
    }
    Ok(Bundle { manifest: text, wasm, assets })
}

#[cfg(target_arch = "wasm32")]
fn bundle_from_disk(cmd: &str, _path: &str) -> Result<Bundle> {
    Err(bad(cmd, "there are no files in the browser: publish an installed plugin with `plugin`"))
}

/// Publish a plugin folder (or an installed plugin) as a new version (needs the server's key).
fn publish(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "plugin.directory.publish";
    let server = string(p, "server").ok_or_else(|| bad(cmd, "`server` required: ws://host:port/?key=… (`figcraft-server key` prints it)"))?;
    let bundle = match (string(p, "path"), string(p, "plugin")) {
        (Some(path), _) => bundle_from_disk(cmd, &path)?,
        (None, Some(id)) => {
            let ip = installed(s, cmd, &id)?.clone();
            match &ip.path {
                Some(path) => bundle_from_disk(cmd, path)?,
                None => {
                    let wasm = crate::plugins::module_of(s, &ip)?;
                    let manifest = serde_json::to_string(&ip.manifest).map_err(|e| EngineError::Other(e.to_string()))?;
                    Bundle { manifest, wasm: wasm.to_vec(), assets: Vec::new() }
                }
            }
        }
        (None, None) => return Err(bad(cmd, "pass `path` (a plugin folder or its figcraft-plugin.json) or `plugin` (an installed plugin's id)")),
    };
    // Checked here too, for a clear message before anything is sent.
    bundle.check().map_err(|e| bad(cmd, e))?;
    let bytes = bundle.encode();
    if bytes.len() > MAX_BUNDLE {
        return Err(bad(cmd, format!("the plugin is larger than {} MiB", MAX_BUNDLE >> 20)));
    }
    let PluginReply::Published(entry) = request(s, &server, PluginReq::Publish { bundle: bytes })? else { return Err(unexpected(cmd)) };
    Ok(json!({"published": entry, "server": address(&server)}))
}

fn unpublish(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "plugin.directory.unpublish";
    let id = string(p, "id").ok_or_else(|| bad(cmd, "`id` required"))?;
    let server = string(p, "server").ok_or_else(|| bad(cmd, "`server` required: ws://host:port/?key=…"))?;
    let PluginReply::Removed(versions) = request(s, &server, PluginReq::Unpublish { id: id.clone(), version: string(p, "version") })? else {
        return Err(unexpected(cmd));
    };
    Ok(json!({"id": id, "removed": versions}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "plugin.directory.list",
            "Browse plugin directory",
            "{server? (ws://host:port/; default: the one browsed last), query?} → {server, plugins: [{id, name, version, description, author, api, access, commands, widget, bytes, sha256, published, versions, installed, update}]}",
            false,
            list
        ),
        spec!(
            "plugin.directory.install",
            "Install plugin from directory",
            "{id, version? (default: the latest), confirm: true (required: installing runs the plugin's code), server?, sha256? (the one shown)} downloads, checks the SHA-256 and installs or updates it → {installed, name, version, permissions, sha256, path, source, replaced}",
            false,
            install
        ),
        spec!(
            "plugin.directory.publish",
            "Publish plugin to directory",
            "{path (a plugin folder or its figcraft-plugin.json) | plugin (an installed plugin's id), server (ws://host:port/?key=…)} → {published: entry, server}",
            false,
            publish
        ),
        spec!(
            "plugin.directory.unpublish",
            "Unpublish plugin from directory",
            "{id, version? (default: every version), server (ws://host:port/?key=…)} → {id, removed: [versions]}",
            false,
            unpublish
        ),
    ]
}
