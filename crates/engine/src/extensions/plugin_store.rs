//! Plugins from bundle files, catalogs and GitHub releases, kept in the suite's plug-in store
//! (`craft_plugin::store`, RFC-0046): `plugin.install`, `plugin.update` and the `plugin.catalog.*`
//! commands. A figcraft-server's plugin directory is [`crate::plugin_directory`]; manifests under
//! development are `plugin.import`. See `docs/plugins.md`.
//!
//! Everything a catalog, GitHub or a bundle file says is untrusted. craft-plugin checks downloads
//! (GitHub's hosts over `https://` only, size caps, the SHA-256 a catalog pins or a release's
//! `SHA256SUMS` lists), then the bundle (its manifest, and its module compiled, never run), and
//! FigCraft checks the plugin was written for it ([`crate::plugins::APP`]). Installing needs the
//! user's say-so: without `confirm: true` the command only reads the plugin and describes it (what
//! the app's confirmation shows); nothing is written before. The bundle read for that description
//! is kept for the session, so the install that follows doesn't download it again.
//!
//! Downloads go through the session's link ([`NetRequest::Fetch`]), so the UI runs these commands
//! without waiting ([`crate::background`]); a session without a link that can fetch says so
//! ([`NO_HTTP`]). The store lives in the folder the app sets ([`Runtime::store_dir`]); without one
//! (the CLI, the MCP server, the browser, tests) an installed plugin lasts for the session.
//!
//! [`Runtime::store_dir`]: crate::plugins::Runtime::store_dir

use std::sync::Arc;

use craft_plugin::catalog::{Catalog, Package, Release};
use craft_plugin::{Fetch, Manifest, bundle, catalog, fetch, github, newer, sha256_hex};
use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::live::{NetReply, NetRequest};
use crate::params::{get, string};
use crate::plugins::{APP, InstalledPlugin, check_target, register};
use crate::{EngineError, Result, Session, bad, spec};

/// The store's folder in a settings folder (`Plug-ins`).
#[cfg(not(target_arch = "wasm32"))]
pub use craft_plugin::store::plugins_dir;

/// Why there's no downloading here.
pub const NO_HTTP: &str = "installing plugins from catalogs and GitHub needs the desktop app";
/// Most catalogs one person adds.
const MAX_CATALOGS: usize = 32;

/// Where a plugin in the store came from (craft-plugin's `store::Source`, which only native builds
/// have).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    /// A bundle file (its name, never its folder).
    File { name: String },
    /// Package `package` of the catalog at `catalog`.
    Catalog { catalog: String, package: String },
    /// Asset `asset` of release `tag` of GitHub repository `repo` (`owner/name`).
    Github { repo: String, tag: String, asset: String },
}

impl Origin {
    /// How the Plugins window and `plugin.list` show it.
    pub fn label(&self) -> String {
        match self {
            Origin::File { name } if name.is_empty() => "a bundle file".into(),
            Origin::File { name } => name.clone(),
            Origin::Catalog { catalog, package } => format!("{package} in {catalog}"),
            Origin::Github { repo, tag, .. } if tag.is_empty() => format!("github.com/{repo}"),
            Origin::Github { repo, tag, .. } => format!("github.com/{repo}@{tag}"),
        }
    }

    fn describe(&self) -> Value {
        match self {
            Origin::File { name } => json!({"type": "file", "name": name}),
            Origin::Catalog { catalog, package } => json!({"type": "catalog", "catalog": catalog, "package": package}),
            Origin::Github { repo, tag, asset } => json!({"type": "github", "repo": repo, "tag": tag, "asset": asset}),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn to_source(&self) -> craft_plugin::store::Source {
        use craft_plugin::store::Source;
        match self.clone() {
            Origin::File { name } => Source::File { path: name },
            Origin::Catalog { catalog, package } => Source::Catalog { catalog, package },
            Origin::Github { repo, tag, asset } => Source::Github { repo, tag, asset },
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn from_source(source: &craft_plugin::store::Source) -> Origin {
        use craft_plugin::store::Source;
        match source.clone() {
            Source::File { path } => Origin::File { name: path },
            Source::Catalog { catalog, package } => Origin::Catalog { catalog, package },
            Source::Github { repo, tag, asset } => Origin::Github { repo, tag, asset },
        }
    }
}

/// A bundle read for an install confirmation (see the module docs).
#[derive(Clone, Debug)]
pub struct Pending {
    pub sha256: String,
    pub bytes: Arc<Vec<u8>>,
    pub origin: Origin,
}

/// One plugin a catalog offers this FigCraft: its newest release that runs here.
#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    /// The catalog's URL.
    pub catalog: String,
    pub package: Package,
    pub release: Release,
}

/// The catalogs as they were listed last: what they offer, and the ones that couldn't be read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CatalogListing {
    pub offers: Vec<Offer>,
    /// (catalog, why)
    pub errors: Vec<(String, String)>,
}

/// [`Fetch`] over the session's link. A request sent in the background stops the command
/// ([`EngineError::Waiting`]); it runs again when the answer is in.
struct NetFetch<'s> {
    s: &'s mut Session,
    waiting: bool,
}

impl Fetch for NetFetch<'_> {
    fn get(&mut self, url: &str, max: usize) -> std::result::Result<Option<Vec<u8>>, String> {
        match self.s.net(NetRequest::Fetch { url: url.to_string(), max }) {
            Ok((NetReply::Fetch(r), _)) => r,
            Ok(_) => Err("the link answered something else".into()),
            Err(EngineError::Waiting) => {
                self.waiting = true;
                Err("waiting for the download".into())
            }
            Err(e) => Err(e.to_string()),
        }
    }
}

/// Run `f` with the session's link as its HTTPS client: its error as a sentence, or
/// [`EngineError::Waiting`] while a request is out.
fn fetching<T>(s: &mut Session, f: impl FnOnce(&mut dyn Fetch) -> std::result::Result<T, String>) -> Result<std::result::Result<T, String>> {
    let mut net = NetFetch { s, waiting: false };
    let r = f(&mut net);
    if net.waiting {
        return Err(EngineError::Waiting);
    }
    Ok(r)
}

/// A `file://` URL (a local catalog, or a file it names) may be read only where this session may
/// read files (automation roots apply); `https://` URLs pass.
fn local_allowed(url: &str) -> std::result::Result<(), String> {
    if !url.trim_start().to_ascii_lowercase().starts_with("file:") {
        return Ok(());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let path = catalog::file_url_path(url.trim())?;
        crate::file_access::allow(&path, crate::file_access::Access::Read).map(|_| ()).map_err(|e| e.to_string())
    }
    #[cfg(target_arch = "wasm32")]
    Err("there are no files in the browser: use an https:// catalog".into())
}

/// Read the catalog at `url`.
fn read_catalog(s: &mut Session, url: &str) -> Result<std::result::Result<(Catalog, Vec<String>), String>> {
    if let Err(why) = local_allowed(url) {
        return Ok(Err(why));
    }
    fetching(s, |f| fetch::fetch_catalog(f, url))
}

/// Get the bundle `p` names (`path`, `data`, `repo`, or `catalog` + `id`), checked as far as its
/// source allows; a catalog's release is checked against its manifest too.
fn obtain(s: &mut Session, cmd: &str, p: &Value) -> Result<(Arc<Vec<u8>>, Origin)> {
    let failed = |e: String| EngineError::Other(format!("{cmd}: {e}"));
    if let Some(path) = string(p, "path") {
        return read_bundle_file(cmd, &path);
    }
    if let Some(data) = string(p, "data") {
        use base64::Engine as _;
        if data.len() > craft_plugin::MAX_BUNDLE_BYTES / 3 * 4 + 4 {
            return Err(bad(cmd, format!("the bundle is larger than {} MB", craft_plugin::MAX_BUNDLE_BYTES >> 20)));
        }
        let bytes = base64::engine::general_purpose::STANDARD.decode(data.trim()).map_err(|e| bad(cmd, format!("`data`: {e}")))?;
        let name: String = string(p, "name").unwrap_or_default().chars().filter(|c| !c.is_control()).take(200).collect();
        return Ok((Arc::new(bytes), Origin::File { name }));
    }
    if let Some(spec) = string(p, "repo") {
        let repo = github::parse_repo_spec(&spec).map_err(|e| bad(cmd, e))?;
        let plan = fetching(s, |f| fetch::resolve_repo(f, &repo))?.map_err(failed)?;
        let bytes = fetching(s, |f| fetch::download_repo(f, &plan))?.map_err(failed)?;
        let origin = Origin::Github { repo: plan.repo.slug(), tag: plan.repo.tag.clone().unwrap_or_default(), asset: plan.asset };
        return Ok((Arc::new(bytes), origin));
    }
    if let Some(url) = string(p, "catalog") {
        let id = string(p, "id").ok_or_else(|| bad(cmd, "`id` required with `catalog` (a package's id; see plugin.catalog.list)"))?;
        let (cat, _) = read_catalog(s, &url)?.map_err(failed)?;
        let package = cat.package(&id).ok_or_else(|| failed(format!("{url} has no plugin `{id}`")))?.clone();
        let release = match string(p, "version") {
            Some(v) => package.release(&v).ok_or_else(|| failed(format!("{url} has no version {v} of {id}")))?,
            None => package.latest_for(&APP).ok_or_else(|| failed(format!("{url} has no release of {id} that runs in this FigCraft")))?,
        }
        .clone();
        release.target(&package.kind, &APP).map_err(|e| failed(format!("{} can't run here: {e}", package.name)))?;
        if let Err(why) = local_allowed(&release.url) {
            return Err(failed(why));
        }
        let bytes = fetching(s, |f| fetch::download_release(f, &url, &package, &release))?.map_err(failed)?;
        let (_, manifest) = bundle::open(&bytes).map_err(failed)?;
        package.check_manifest(&release, &manifest).map_err(failed)?;
        return Ok((Arc::new(bytes), Origin::Catalog { catalog: url, package: package.id }));
    }
    Err(bad(cmd, "pass `path` (a .craftplugin file), `data` (one as base64), `repo` (owner/repo or owner/repo@tag), or `catalog` and `id`"))
}

#[cfg(not(target_arch = "wasm32"))]
fn read_bundle_file(cmd: &str, path: &str) -> Result<(Arc<Vec<u8>>, Origin)> {
    let file = std::path::PathBuf::from(crate::library::absolute_path(path.trim()));
    if crate::file_access::is_dir(&file) || file.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")) {
        return Err(bad(cmd, "that's a plugin under development: import its manifest with plugin.import"));
    }
    let bytes = crate::file_access::read_capped(&file, craft_plugin::MAX_BUNDLE_BYTES as u64)?;
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok((Arc::new(bytes), Origin::File { name }))
}

#[cfg(target_arch = "wasm32")]
fn read_bundle_file(cmd: &str, _path: &str) -> Result<(Arc<Vec<u8>>, Origin)> {
    Err(bad(cmd, "there are no files in the browser: pass the bundle as `data` (base64)"))
}

/// Whether one of the catalogs added offers GitHub repository `slug` (`owner/name`); catalogs
/// that can't be read count as not offering it.
fn in_catalogs(s: &mut Session, slug: &str) -> Result<bool> {
    let Some((owner, name)) = slug.split_once('/') else { return Ok(false) };
    for url in s.prefs.plugin_catalogs.clone() {
        if let Ok((cat, _)) = read_catalog(s, &url)?
            && cat.packages.iter().any(|p| p.repo_ref().is_some_and(|r| r.same_repo(owner, name)))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Keep `bytes` in the store, when the session has one; whether it did.
#[cfg(not(target_arch = "wasm32"))]
fn keep(s: &Session, bytes: &[u8], origin: &Origin) -> std::result::Result<bool, String> {
    let Some(dir) = &s.plugins.store_dir else { return Ok(false) };
    craft_plugin::store::Store::new(dir.clone()).install(bytes, origin.to_source()).map(|_| true)
}

#[cfg(target_arch = "wasm32")]
fn keep(_: &Session, _: &[u8], _: &Origin) -> std::result::Result<bool, String> {
    Ok(false)
}

/// Remove plugin `id` from the store in `dir`, if it is there.
pub(crate) fn forget(dir: Option<&std::path::Path>, id: &str) {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(dir) = dir
        && let Err(e) = craft_plugin::store::Store::new(dir).remove(id)
    {
        log::warn!("plugins: {e}");
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (dir, id);
}

/// List the plugins in the store among the installed ones (the app calls this at start, after
/// setting [`crate::plugins::Runtime::store_dir`]): how many. A plugin with the id of one already
/// listed, one this FigCraft can't run and a damaged folder are left out, with a warning in the log.
pub fn load_store(s: &mut Session) -> usize {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(dir) = s.plugins.store_dir.clone() else { return 0 };
        let (installed, problems) = craft_plugin::store::Store::new(dir).list();
        for p in problems {
            log::warn!("plugins: {}: {}", p.dir.display(), p.error);
        }
        let mut n = 0;
        for i in installed {
            if s.prefs.plugins.iter().any(|p| p.manifest.id == i.manifest.id) {
                continue;
            }
            if let Err(why) = check_target(&i.manifest) {
                log::warn!("plugins: {why}");
                continue;
            }
            let label = Origin::from_source(&i.record.source).label();
            s.plugins.modules.insert(i.manifest.id.clone(), Arc::new(i.bundle.wasm));
            s.prefs.plugins.push(InstalledPlugin { manifest: i.manifest, path: None, source: Some(label), store: true });
            n += 1;
        }
        n
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = s;
        0
    }
}

/// What a confirmation shows of a plugin about to be installed.
fn preview(s: &Session, m: &Manifest, sha256: &str, origin: &Origin, bytes: usize) -> Value {
    let installed = s.prefs.plugins.iter().find(|p| p.manifest.id == m.id);
    json!({
        "id": m.id,
        "name": m.name,
        "version": m.version,
        "author": m.author,
        "description": m.description,
        "license": m.license,
        "kind": m.kind,
        "api": m.api,
        "widget": m.widget,
        "permissions": m.permissions,
        "commands": m.commands.iter().map(|c| &c.label).collect::<Vec<_>>(),
        "sha256": sha256,
        "bytes": bytes,
        "source": origin.describe(),
        "from": origin.label(),
        "installedVersion": installed.map(|p| &p.manifest.version),
        "permissionsChanged": installed.is_some_and(|p| craft_plugin::permissions::changed(&p.manifest, m)),
    })
}

/// `plugin.install`, and `plugin.update` once it knows where the new version comes from.
fn install_from(s: &mut Session, cmd: &str, p: &Value) -> Result<Value> {
    let confirm = get(p, "confirm") == Some(&Value::Bool(true));
    let shown = string(p, "sha256").map(|h| h.trim().to_ascii_lowercase());
    // The bundle the confirmation showed, when this is its install.
    let kept = s.plugins.pending.clone().filter(|k| confirm && shown.as_deref() == Some(k.sha256.as_str()) && same_request(&k.origin, p));
    let (bytes, origin) = match kept {
        Some(k) => (k.bytes, k.origin),
        None => obtain(s, cmd, p)?,
    };
    let fail = |why: String| EngineError::Other(format!("{cmd}: {why}"));
    let (bundle, manifest) = bundle::open(&bytes).map_err(fail)?;
    check_target(&manifest).map_err(fail)?;
    let sha = sha256_hex(&bytes);
    if shown.as_ref().is_some_and(|want| *want != sha) {
        return Err(fail(format!("{} isn't the version that was shown (its SHA-256 differs)", manifest.name)));
    }
    let mut out = preview(s, &manifest, &sha, &origin, bytes.len());
    if !confirm {
        // A plugin from a repository may be in no catalog the user reads: the confirmation says so.
        out["inCatalog"] = match &origin {
            Origin::Github { repo, .. } => json!(in_catalogs(s, repo)?),
            Origin::Catalog { .. } => json!(true),
            Origin::File { .. } => Value::Null,
        };
        out["installed"] = json!(false);
        out["confirm"] =
            json!("installing runs the plugin's code on this machine: check what it asks for, then pass `confirm: true` and this `sha256`");
        s.plugins.pending = Some(Pending { sha256: sha, bytes, origin });
        return Ok(out);
    }
    let stored = keep(s, &bytes, &origin).map_err(fail)?;
    s.plugins.pending = None;
    let entry = InstalledPlugin { manifest, path: None, source: Some(origin.label()), store: stored };
    out["replaced"] = json!(register(s, entry, bundle.wasm));
    out["installed"] = json!(true);
    out["store"] = json!(stored);
    Ok(out)
}

/// Whether `p` asks for the bundle that `origin` describes (a kept bundle answers only the same
/// request).
fn same_request(origin: &Origin, p: &Value) -> bool {
    match origin {
        Origin::File { .. } => false,
        Origin::Catalog { catalog, package } => {
            string(p, "catalog").as_deref() == Some(catalog.as_str()) && string(p, "id").is_some_and(|id| id.eq_ignore_ascii_case(package))
        }
        Origin::Github { repo, .. } => {
            string(p, "repo").and_then(|r| github::parse_repo_spec(&r).ok()).is_some_and(|r| r.slug().eq_ignore_ascii_case(repo))
        }
    }
}

fn install(s: &mut Session, p: &Value) -> Result<Value> {
    install_from(s, "plugin.install", p)
}

/// Where plugin `id` came from, if it is in the store.
#[cfg(not(target_arch = "wasm32"))]
fn stored(s: &Session, id: &str) -> std::result::Result<Option<(Origin, String)>, String> {
    let Some(dir) = &s.plugins.store_dir else { return Ok(None) };
    Ok(craft_plugin::store::Store::new(dir.clone()).load(id)?.map(|i| (Origin::from_source(&i.record.source), i.record.sha256)))
}

#[cfg(target_arch = "wasm32")]
fn stored(_: &Session, _: &str) -> std::result::Result<Option<(Origin, String)>, String> {
    Ok(None)
}

/// `plugin.install`'s parameters for the latest release of where `origin` points (`None`: a
/// bundle file, which has no updates).
fn latest_request(origin: &Origin) -> Option<Value> {
    match origin {
        Origin::File { .. } => None,
        Origin::Catalog { catalog, package } => Some(json!({"catalog": catalog, "id": package})),
        Origin::Github { repo, .. } => Some(json!({"repo": repo})),
    }
}

/// The update of a plugin from a catalog or a repository, if there is one: what it would install.
fn check_update(s: &mut Session, id: &str, origin: &Origin, sha256: &str, version: &str) -> Result<Option<Value>> {
    let failed = |e: String| EngineError::Other(format!("plugin.update: {id}: {e}"));
    match origin {
        Origin::File { .. } => Ok(None),
        Origin::Catalog { catalog, package } => {
            let (cat, _) = read_catalog(s, catalog)?.map_err(failed)?;
            let Some(release) = cat.package(package).and_then(|p| p.latest_for(&APP)) else { return Ok(None) };
            Ok(newer(&release.version, version).then(|| json!({"version": release.version, "sha256": release.sha256})))
        }
        Origin::Github { repo, .. } => {
            let r = github::parse_repo_spec(repo).map_err(failed)?;
            let plan = fetching(s, |f| fetch::resolve_repo(f, &r))?.map_err(failed)?;
            Ok((!plan.sha256.eq_ignore_ascii_case(sha256)).then(|| json!({"tag": plan.repo.tag, "sha256": plan.sha256})))
        }
    }
}

/// Without `id`: the plugins in the store with a newer release where they came from. With `id`:
/// that plugin's newer release, described for a confirmation, or installed with `confirm: true`.
fn update(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "plugin.update";
    let fail = |why: String| EngineError::Other(format!("{cmd}: {why}"));
    if let Some(id) = string(p, "id") {
        let installed = crate::plugins::installed(s, cmd, &id)?.clone();
        let Some((origin, sha)) = stored(s, &id).map_err(fail)? else {
            return Err(bad(
                cmd,
                format!("{} isn't from a catalog or a GitHub repository (one from a plugin directory updates there)", installed.manifest.name),
            ));
        };
        let mut req = latest_request(&origin)
            .ok_or_else(|| bad(cmd, format!("{} was installed from a file: install the new file", installed.manifest.name)))?;
        for k in ["confirm", "sha256"] {
            if let Some(v) = get(p, k) {
                req[k] = v.clone();
            }
        }
        let out = install_from(s, cmd, &req)?;
        if out["sha256"].as_str().is_some_and(|new| new.eq_ignore_ascii_case(&sha)) && out["installed"] != json!(true) {
            return Ok(json!({"id": id, "update": false, "version": installed.manifest.version}));
        }
        let mut out = out;
        out["update"] = json!(true);
        return Ok(out);
    }
    let mut updates = Vec::new();
    let ids: Vec<(String, String, String)> =
        s.prefs.plugins.iter().filter(|x| x.store).map(|x| (x.manifest.id.clone(), x.manifest.name.clone(), x.manifest.version.clone())).collect();
    let mut errors = Vec::new();
    for (id, name, version) in ids {
        let Some((origin, sha)) = stored(s, &id).map_err(fail)? else { continue };
        match check_update(s, &id, &origin, &sha, &version) {
            Ok(Some(mut u)) => {
                u["id"] = json!(id);
                u["name"] = json!(name);
                u["installedVersion"] = json!(version);
                u["from"] = json!(origin.label());
                updates.push(u);
            }
            Ok(None) => {}
            Err(EngineError::Waiting) => return Err(EngineError::Waiting),
            Err(e) => errors.push(json!({"id": id, "error": e.to_string()})),
        }
    }
    Ok(json!({"updates": updates, "errors": errors}))
}

/// Read the catalogs `p` names (`catalog`, else every one added) and keep what they offer for the
/// Plugins window.
fn catalog_list(s: &mut Session, p: &Value) -> Result<Value> {
    let urls = match string(p, "catalog") {
        Some(url) => vec![url],
        None => s.prefs.plugin_catalogs.clone(),
    };
    let query = string(p, "query").unwrap_or_default().trim().to_lowercase();
    let mut listing = CatalogListing::default();
    let mut warnings = Vec::new();
    for url in &urls {
        match read_catalog(s, url)? {
            Ok((cat, w)) => {
                warnings.extend(w.into_iter().map(|w| json!({"catalog": url, "warning": w})));
                for package in cat.packages {
                    let Some(release) = package.latest_for(&APP).cloned() else { continue };
                    let hay = [&package.id, &package.name, &package.author, &package.description];
                    if query.is_empty() || hay.iter().any(|f| f.to_lowercase().contains(&query)) {
                        listing.offers.push(Offer { catalog: url.clone(), package, release });
                    }
                }
            }
            Err(why) => listing.errors.push((url.clone(), why)),
        }
    }
    let plugins: Vec<Value> = listing
        .offers
        .iter()
        .map(|o| {
            let have = crate::plugin_directory::installed_version(s, &o.package.id);
            json!({
                "catalog": o.catalog,
                "id": o.package.id,
                "name": o.package.name,
                "author": o.package.author,
                "description": o.package.description,
                "license": o.package.license,
                "repo": o.package.repo,
                "kind": o.package.kind,
                "version": o.release.version,
                "sha256": o.release.sha256,
                "bytes": o.release.size,
                "installed": have,
                "update": have.is_some_and(|h| newer(&o.release.version, h)),
            })
        })
        .collect();
    let errors: Vec<Value> = listing.errors.iter().map(|(c, e)| json!({"catalog": c, "error": e})).collect();
    s.plugins.catalogs = Some(listing);
    Ok(json!({"catalogs": urls, "plugins": plugins, "errors": errors, "warnings": warnings}))
}

/// Add a catalog (read first, so a wrong address is said at once).
fn catalog_add(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "plugin.catalog.add";
    let url = string(p, "url")
        .map(|u| u.trim().to_string())
        .ok_or_else(|| bad(cmd, "`url` required: a catalog's index.json (https:// on GitHub, or file://)"))?;
    catalog::check_url(&url, None).map_err(|e| bad(cmd, e))?;
    let (cat, warnings) = read_catalog(s, &url)?.map_err(|e| EngineError::Other(format!("{cmd}: {e}")))?;
    let list = &mut s.prefs.plugin_catalogs;
    let added = !list.contains(&url);
    if added {
        if list.len() >= MAX_CATALOGS {
            return Err(bad(cmd, format!("at most {MAX_CATALOGS} catalogs: remove one first")));
        }
        list.push(url.clone());
        s.plugins.unsaved = true;
    }
    Ok(json!({"catalog": url, "added": added, "packages": cat.packages.len(), "warnings": warnings, "catalogs": s.prefs.plugin_catalogs}))
}

fn catalog_remove(s: &mut Session, p: &Value) -> Result<Value> {
    let url = string(p, "url").map(|u| u.trim().to_string()).ok_or_else(|| bad("plugin.catalog.remove", "`url` required"))?;
    let before = s.prefs.plugin_catalogs.len();
    s.prefs.plugin_catalogs.retain(|c| *c != url);
    let removed = s.prefs.plugin_catalogs.len() != before;
    if removed {
        s.plugins.unsaved = true;
        if let Some(l) = s.plugins.catalogs.as_mut() {
            l.offers.retain(|o| o.catalog != url);
            l.errors.retain(|(c, _)| *c != url);
        }
    }
    Ok(json!({"removed": removed, "catalogs": s.prefs.plugin_catalogs}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "plugin.install",
            "Install plugin",
            "{path (a .craftplugin file) | data (one as base64), name? | repo (owner/repo or owner/repo@tag: its GitHub release) | catalog (index.json URL), id, version? (default: the newest that runs here), confirm: true (required to install: installing runs the plugin's code), sha256? (the one shown)} without confirm: reads and checks the plugin and describes it, writing nothing → {id, name, version, kind, permissions, sha256, source, from, inCatalog, installedVersion, permissionsChanged, installed: false}; with confirm: installs it into the plug-in store (this session only without one) → {…, installed: true, store, replaced}",
            false,
            install
        ),
        spec!(
            "plugin.update",
            "Update plugins",
            "{id?, confirm?, sha256?} without id: the plugins from catalogs and GitHub with a newer release → {updates: [{id, name, installedVersion, version | tag, sha256, from}], errors}; with id: like plugin.install for its newest release (confirm: true installs it) → {…, update}",
            false,
            update
        ),
        spec!(
            "plugin.catalog.list",
            "Browse plugin catalogs",
            "{catalog? (an index.json URL; default: every catalog added), query?} → {catalogs, plugins: [{catalog, id, name, author, description, license, repo, kind, version, sha256, bytes, installed, update}], errors, warnings}",
            false,
            catalog_list
        ),
        spec!(
            "plugin.catalog.add",
            "Add plugin catalog",
            "{url (index.json: https:// on GitHub's hosts, or file://)} reads it, then remembers it → {catalog, added, packages, warnings, catalogs}",
            false,
            catalog_add
        ),
        spec!("plugin.catalog.remove", "Remove plugin catalog", "{url} → {removed, catalogs}", false, catalog_remove),
    ]
}
