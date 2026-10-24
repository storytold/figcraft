//! Local libraries (the data model is in [`figcraft_doc::library`]).
//!
//! - **Publish** ([`publish`]): the chosen components, component sets, styles and variable
//!   collections, and every asset, style and variable they use, are copied into a snapshot. Each
//!   asset keeps the library version it last changed in, so files using the library update only
//!   what changed. Variables hidden from publishing stay out unless something published uses them.
//! - **Read** ([`Cache`]): library files are read read-only (only their library entries, with size
//!   limits) and again whenever they change on disk. The browser has no file system: there every
//!   read is a clear error.
//! - **Use** ([`import`], [`import_styles`], [`import_variables`]): assets are copied into the
//!   document's library store as read-only main components, and into its styles and variables as
//!   read-only linked copies; [`check_read_only`] refuses other edits of them.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use figcraft_doc::proto::Action;
use figcraft_doc::*;
use serde_json::Value;

use crate::components::inside_instance;
use crate::params::id_of;
use crate::{EngineError, Result};

/// Largest library file read.
pub const MAX_LIBRARY_FILE: u64 = 1024 * 1024 * 1024;
/// Most library files a session remembers.
const MAX_CACHED: usize = 64;
/// Most assets one import or publish walks (with what they use).
const MAX_ASSETS: usize = 100_000;
/// Why library edits are refused.
pub const READ_ONLY: &str = "library components, styles and variables can't be edited in this file: edit them in their library file and publish it";
#[cfg(target_arch = "wasm32")]
const NO_FILES: &str = "libraries need the desktop app: the browser has no file system";

fn disabled(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::Disabled(cmd.to_string(), msg.into())
}

fn is_asset(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Component { .. } | NodeKind::ComponentSet { .. })
}

/// Names starting with `.` or `_` keep an asset out of a library by default.
pub fn hidden_name(name: &str) -> bool {
    name.starts_with('.') || name.starts_with('_')
}

/// `name` with or without the mark that keeps an asset out of a library by default: hiding puts
/// a `.` before it, showing takes every leading `.` and `_` away (a name of nothing else keeps
/// one letter's worth: "Component").
pub fn with_hidden_name(name: &str, hidden: bool) -> String {
    match (hidden, hidden_name(name)) {
        (true, false) => format!(".{name}"),
        (false, true) => {
            let shown = name.trim_start_matches(['.', '_']).trim_start();
            if shown.is_empty() { "Component".to_string() } else { shown.to_string() }
        }
        _ => name.to_string(),
    }
}

/// Where the Assets panel files asset `id`: under its page, the frames, sections and groups
/// holding it, then the folders its name makes with slashes (`Buttons/Primary` is `Primary` in
/// `Buttons`). Returns the folders, outermost first, and the name shown on its tile.
pub fn asset_folder(doc: &Document, id: NodeId) -> (Vec<String>, String) {
    let Some(n) = doc.node(id) else { return (Vec::new(), String::new()) };
    let mut folders: Vec<String> = doc.ancestors(id).iter().rev().filter_map(|a| doc.node(*a)).map(|a| a.name.trim().to_string()).collect();
    let mut parts: Vec<String> = n.name.split('/').map(str::trim).filter(|p| !p.is_empty()).map(str::to_string).collect();
    let label = parts.pop().unwrap_or_else(|| n.name.clone());
    folders.extend(parts);
    (folders, label)
}

/// The asset a layer belongs to: its outermost component or component set (itself included).
pub fn asset_root(doc: &Document, id: NodeId) -> Option<NodeId> {
    let mut chain = vec![id];
    chain.extend(doc.ancestors(id));
    chain.into_iter().rfind(|c| doc.node(*c).is_some_and(is_asset))
}

/// Components the layers of `root` use: mains of instances, instance swap defaults and values.
fn used_components(doc: &Document, root: NodeId) -> BTreeSet<NodeId> {
    let mut out = BTreeSet::new();
    for id in doc.subtree(root) {
        let Some(n) = doc.node(id) else { continue };
        match &n.kind {
            NodeKind::Instance { component, props } => {
                out.insert(*component);
                for k in swap_props(doc, *component) {
                    out.extend(props.get(&k).and_then(id_of));
                }
            }
            NodeKind::Component { props, .. } => {
                for d in props.values() {
                    match d {
                        PropDef::InstanceSwap { default, preferred } => {
                            out.insert(*default);
                            out.extend(preferred.iter().copied());
                        }
                        PropDef::Slot { preferred } => out.extend(preferred.iter().copied()),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// The instance swap properties of component `c`.
pub(crate) fn swap_props(doc: &Document, c: NodeId) -> Vec<String> {
    match doc.node(c).map(|n| &n.kind) {
        Some(NodeKind::Component { props, .. }) => {
            props.iter().filter(|(_, d)| matches!(d, PropDef::InstanceSwap { .. })).map(|(k, _)| k.clone()).collect()
        }
        _ => Vec::new(),
    }
}

/// `roots` (asset roots) and every asset they use, transitively, in discovery order.
pub fn closure(doc: &Document, roots: &[NodeId]) -> Vec<NodeId> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    let mut stack: Vec<NodeId> = roots.iter().rev().copied().collect();
    while let Some(r) = stack.pop() {
        if out.len() >= MAX_ASSETS || !seen.insert(r) {
            continue;
        }
        out.push(r);
        stack.extend(used_components(doc, r).into_iter().filter_map(|c| asset_root(doc, c)).filter(|a| !seen.contains(a)));
    }
    out
}

/// The style ids a layer links to (fill, stroke, effect, grid, text, and text styles on
/// character ranges).
pub(crate) fn style_links(n: &Node) -> impl Iterator<Item = &String> {
    let chars =
        n.kind.text().into_iter().flat_map(|t| std::iter::once(&t.style).chain(t.runs.iter().map(|r| &r.style))).filter_map(|s| s.style_id.as_ref());
    [&n.styles.fill, &n.styles.stroke, &n.styles.effect, &n.styles.grid, &n.styles.text].into_iter().flatten().chain(chars)
}

/// Style ids the layers under `roots` link to.
fn linked_styles(doc: &Document, roots: &[NodeId]) -> BTreeSet<String> {
    roots.iter().flat_map(|r| doc.subtree(*r)).filter_map(|i| doc.node(i)).flat_map(|n| style_links(n).cloned()).collect()
}

/// Local components and component sets, sets as one entry (never library copies or layers of
/// instances), in id order.
pub fn local_components(doc: &Document) -> Vec<NodeId> {
    let in_set = |n: &Node| n.parent.and_then(|p| doc.node(p)).is_some_and(|p| matches!(p.kind, NodeKind::ComponentSet { .. }));
    doc.nodes()
        .filter(|n| matches!(n.kind, NodeKind::Component { .. }) && !in_set(n) || matches!(n.kind, NodeKind::ComponentSet { .. }))
        .filter(|n| !doc.is_remote(n.id) && !inside_instance(doc, n.id))
        .map(|n| n.id)
        .collect()
}

/// Whether `style` was copied from a library.
pub fn is_remote_style(doc: &Document, style: &str) -> bool {
    doc.libraries.style_import(style).is_some()
}

/// Local styles (not library copies).
pub fn local_styles(doc: &Document) -> impl Iterator<Item = &Style> {
    doc.styles.iter().filter(|s| !is_remote_style(doc, &s.id))
}

/// Whether variable `id` was copied from a library.
pub fn is_remote_variable(doc: &Document, id: &str) -> bool {
    doc.libraries.variable_import(id).is_some()
}

/// Whether variable collection `id` was copied from a library.
pub fn is_remote_collection(doc: &Document, id: &str) -> bool {
    doc.libraries.collection_import(id).is_some()
}

/// Local variable collections (not library copies).
pub fn local_collections(doc: &Document) -> impl Iterator<Item = &Collection> {
    doc.variables.collections.iter().filter(|c| !is_remote_collection(doc, &c.id))
}

/// What a publish offers: asset roots, style ids and variable collection ids.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    pub components: Vec<NodeId>,
    pub styles: Vec<String>,
    pub collections: Vec<CollectionId>,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.components.is_empty() && self.styles.is_empty() && self.collections.is_empty()
    }
}

/// What a file can publish: its top-level local components and sets, its local styles and its
/// local variable collections.
pub fn publishable(doc: &Document) -> Selection {
    Selection {
        components: local_components(doc).into_iter().filter(|c| asset_root(doc, *c) == Some(*c)).collect(),
        styles: local_styles(doc).map(|s| s.id.clone()).collect(),
        collections: local_collections(doc).map(|c| c.id.clone()).collect(),
    }
}

/// What a publish offers when nothing is chosen: the assets offered last time (by `prev`, the
/// version published last) plus new ones, never assets named with a leading `.` or `_`. The name
/// decides both ways: adding the prefix takes a published asset out, removing it brings the asset
/// back.
pub fn default_selection(doc: &Document, prev: Option<&LibraryManifest>) -> Selection {
    let all = publishable(doc);
    // `last`: whether it was offered last time, and under which name.
    let pick = |last: Option<(bool, &str)>, name: Option<&str>| {
        name.is_some_and(|n| !hidden_name(n))
            && match last {
                Some((offered, was)) if !hidden_name(was) => offered,
                _ => true,
            }
    };
    Selection {
        components: all
            .components
            .into_iter()
            .filter(|c| pick(prev.and_then(|m| m.component(*c)).map(|a| (a.published, a.name.as_str())), doc.node(*c).map(|n| n.name.as_str())))
            .collect(),
        styles: all
            .styles
            .into_iter()
            .filter(|s| pick(prev.and_then(|m| m.style(s)).map(|x| (x.published, x.name.as_str())), doc.style(s).map(|x| x.name.as_str())))
            .collect(),
        collections: all
            .collections
            .into_iter()
            .filter(|c| {
                pick(prev.and_then(|m| m.collection(c)).map(|x| (x.published, x.name.as_str())), doc.variables.collection(c).map(|x| x.name.as_str()))
            })
            .collect(),
    }
}

/// `seeds` (variable ids) and every variable they alias, transitively, by id.
fn alias_closure(vars: &Variables, seeds: impl IntoIterator<Item = VariableId>) -> BTreeSet<VariableId> {
    let mut out = BTreeSet::new();
    let mut stack: Vec<VariableId> = seeds.into_iter().collect();
    while let Some(cur) = stack.pop() {
        let Some(v) = vars.get(&cur) else { continue };
        if out.len() >= MAX_ASSETS || !out.insert(v.id.clone()) {
            continue;
        }
        for val in v.values.values() {
            if let VariableValue::Alias(a) = val {
                stack.push(a.clone());
            }
        }
    }
    out
}

/// The variables a layer's prototype actions set or read (in expressions and conditions).
fn action_variables(n: &Node) -> impl Iterator<Item = &str> {
    n.interactions.iter().flat_map(|it| &it.actions).flat_map(Action::variables)
}

/// Variables the layers under `roots` bind, and the collections they choose modes of.
fn used_variables(doc: &Document, roots: &[NodeId]) -> (BTreeSet<VariableId>, BTreeSet<CollectionId>) {
    let (mut vars, mut colls) = (BTreeSet::new(), BTreeSet::new());
    for n in roots.iter().flat_map(|r| doc.subtree(*r)).filter_map(|i| doc.node(i)) {
        vars.extend(n.bound.values().cloned());
        colls.extend(n.var_modes.keys().cloned());
    }
    (vars, colls)
}

/// Whether asset `root` is the same in two snapshots (its position on the page aside).
fn same_asset(a: &Document, b: &Document, root: NodeId) -> bool {
    let (sa, sb) = (a.subtree(root), b.subtree(root));
    sa == sb
        && sa.iter().all(|i| match (a.node(*i), b.node(*i)) {
            (Some(x), Some(y)) if *i == root => Node { transform: y.transform, parent: y.parent, ..x.clone() } == *y,
            (Some(x), Some(y)) => x == y,
            _ => false,
        })
}

/// A new, unique library id.
fn new_library_id(doc: &Document, seed: u64) -> String {
    let now = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let h = figcraft_format::image_hash(format!("{}|{now}|{seed}|{}", doc.name, doc.node_count()).as_bytes());
    h.get(..16).map(str::to_string).unwrap_or(h)
}

/// Publish `sel` (asset roots, styles and variable collections) of `doc` as the library version
/// after `prev` (the file's last published version, here or on its server; `None`: the first).
/// `seed` keeps a first publish's new id unique. Refused when there is nothing to publish or
/// nothing changed.
pub fn publish(doc: &Document, prev: Option<&PublishedLibrary>, sel: &Selection, description: Option<String>, seed: u64) -> Result<PublishedLibrary> {
    let cmd = "library.publish";
    if sel.is_empty() {
        return Err(disabled(cmd, "nothing to publish: the file has no components, styles or variables to offer"));
    }
    let (components, styles) = (&sel.components, &sel.styles);
    let units = closure(doc, components);
    // The snapshot keeps the file's ids, on a page whose id none of them uses.
    let mut snap = Document::new(doc.name.clone());
    let subtrees: Vec<(NodeId, Vec<NodeId>)> = units.iter().map(|u| (*u, doc.subtree(*u))).collect();
    if let Some(max) = subtrees.iter().flat_map(|(_, s)| s.iter()).max() {
        snap.reserve_ids(*max);
    }
    let page = snap.add_page("Library");
    for p in snap.pages.clone() {
        if p != page {
            snap.remove(p)?;
        }
    }
    snap.current_page = Some(page);
    for (u, sub) in &subtrees {
        let mut nodes: Vec<Node> = sub.iter().filter_map(|i| doc.node(*i).cloned()).collect();
        if let Some(r) = nodes.first_mut() {
            r.transform = doc.world_transform(*u);
        }
        let ids: BTreeMap<NodeId, NodeId> = nodes.iter().map(|n| (n.id, n.id)).collect();
        snap.paste_nodes(&nodes, &ids, *u, page, None)?;
    }
    let mut keep: BTreeSet<String> = linked_styles(doc, &units);
    keep.extend(styles.iter().cloned());
    snap.styles = doc.styles.iter().filter(|s| keep.contains(&s.id)).cloned().collect();
    // Variables: the published collections' (but those hidden from publishing), those the assets
    // bind, and what all of them alias.
    let offered_colls: BTreeSet<&str> = sel.collections.iter().map(String::as_str).collect();
    let offered_vars: BTreeSet<&str> = doc
        .variables
        .variables
        .iter()
        .filter(|v| offered_colls.contains(v.collection.as_str()) && !v.hidden_from_publishing)
        .map(|v| v.id.as_str())
        .collect();
    let (bound, mode_colls) = used_variables(doc, &units);
    let var_ids = alias_closure(&doc.variables, offered_vars.iter().map(ToString::to_string).chain(bound));
    snap.variables.variables = doc.variables.variables.iter().filter(|v| var_ids.contains(&v.id)).cloned().collect();
    let coll_ids: BTreeSet<&str> = snap
        .variables
        .variables
        .iter()
        .map(|v| v.collection.as_str())
        .chain(offered_colls.iter().copied())
        .chain(mode_colls.iter().map(String::as_str))
        .collect();
    snap.variables.collections = doc.variables.collections.iter().filter(|c| coll_ids.contains(c.id.as_str())).cloned().collect();
    for h in used_images(&snap) {
        if let Some(img) = doc.images.get(&h) {
            snap.images.insert(h, img.clone());
        }
    }
    snap.take_dirty();
    // Versions: an asset or style keeps the version it last changed in.
    let version = prev.map_or(1, |p| p.manifest.version.saturating_add(1));
    let offered: BTreeSet<NodeId> = components.iter().copied().collect();
    let components = units
        .iter()
        .filter_map(|u| {
            let n = snap.node(*u)?;
            let kept = prev.and_then(|p| p.manifest.component(*u).filter(|_| same_asset(&p.doc, &snap, *u))).map(|a| a.version);
            Some(LibraryAsset {
                id: *u,
                name: n.name.clone(),
                set: matches!(n.kind, NodeKind::ComponentSet { .. }),
                version: kept.unwrap_or(version),
                published: offered.contains(u),
            })
        })
        .collect();
    let styles = snap
        .styles
        .iter()
        .map(|s| {
            let kept = prev.and_then(|p| p.manifest.style(&s.id).filter(|_| p.doc.styles.iter().any(|x| x == s))).map(|x| x.version);
            LibraryStyle {
                id: s.id.clone(),
                name: s.name.clone(),
                kind: s.kind.type_name().into(),
                version: kept.unwrap_or(version),
                published: styles.contains(&s.id),
            }
        })
        .collect();
    let old_colls: HashMap<&str, &Collection> =
        prev.map(|p| p.doc.variables.collections.iter().map(|c| (c.id.as_str(), c)).collect()).unwrap_or_default();
    let collections = snap
        .variables
        .collections
        .iter()
        .map(|c| {
            let kept = prev.and_then(|p| p.manifest.collection(&c.id)).filter(|_| old_colls.get(c.id.as_str()) == Some(&c)).map(|x| x.version);
            LibraryCollection {
                id: c.id.clone(),
                name: c.name.clone(),
                version: kept.unwrap_or(version),
                published: offered_colls.contains(c.id.as_str()),
            }
        })
        .collect();
    let old_vars: HashMap<&str, &Variable> = prev.map(|p| p.doc.variables.variables.iter().map(|v| (v.id.as_str(), v)).collect()).unwrap_or_default();
    let variables = snap
        .variables
        .variables
        .iter()
        .map(|v| {
            let kept = prev.and_then(|p| p.manifest.variable(&v.id)).filter(|_| old_vars.get(v.id.as_str()) == Some(&v)).map(|x| x.version);
            LibraryVariable {
                id: v.id.clone(),
                name: v.name.clone(),
                collection: v.collection.clone(),
                kind: v.kind.id().into(),
                version: kept.unwrap_or(version),
                published: offered_vars.contains(v.id.as_str()),
            }
        })
        .collect();
    let manifest = LibraryManifest {
        id: prev.map(|p| p.manifest.id.clone()).unwrap_or_else(|| new_library_id(doc, seed)),
        name: doc.name.clone(),
        version,
        description: description.or_else(|| prev.map(|p| p.manifest.description.clone())).unwrap_or_default(),
        time: crate::cmd::comment::now_secs(),
        components,
        styles,
        collections,
        variables,
    };
    if let Some(p) = prev
        && (LibraryManifest { version: p.manifest.version, time: p.manifest.time, ..manifest.clone() }) == p.manifest
    {
        return Err(disabled(cmd, format!("no changes to publish since version {}", p.manifest.version)));
    }
    Ok(PublishedLibrary { manifest, doc: Arc::new(snap) })
}

/// When a library file was last read.
#[derive(Clone, Debug, PartialEq)]
struct Stamp {
    len: u64,
    modified: Option<std::time::SystemTime>,
}

#[derive(Clone, Debug)]
struct Entry {
    stamp: Option<Stamp>,
    /// When the file was read: a stamp only proves the file unchanged once its modification time
    /// is safely older than that, since file systems keep the time in coarse ticks.
    read_at: Option<std::time::SystemTime>,
    result: std::result::Result<Arc<PublishedLibrary>, String>,
}

/// How far a file's modification time must lie before its read for the stamp to be trusted.
const RACY_STAMP: std::time::Duration = std::time::Duration::from_secs(2);

impl Entry {
    /// A file stamped in the future (clock skew, unpacked archives) never settles: it is re-read on
    /// every load until the clock passes it.
    fn still_valid(&self, now: &Stamp) -> bool {
        let settled = match (now.modified, self.read_at) {
            (Some(modified), Some(read_at)) => modified.checked_add(RACY_STAMP).is_some_and(|t| t < read_at),
            _ => false,
        };
        settled && self.stamp.as_ref() == Some(now)
    }
}

/// Library files read by a session, by path (re-read when they change on disk).
#[derive(Clone, Debug, Default)]
pub struct Cache {
    entries: HashMap<String, Entry>,
}

impl Cache {
    /// The published library in the file at `path`.
    pub fn load(&mut self, path: &str) -> Result<Arc<PublishedLibrary>> {
        self.load_limited(path, MAX_LIBRARY_FILE)
    }

    /// [`load`](Self::load), refusing files larger than `max_bytes`. A server library link gives
    /// what was last fetched ([`Session::load_library`](crate::Session::load_library) fetches).
    pub fn load_limited(&mut self, path: &str, max_bytes: u64) -> Result<Arc<PublishedLibrary>> {
        if crate::library_server::is_server_link(path) {
            return match self.cached(path) {
                Some(r) => r.cloned().map_err(|e| EngineError::Other(e.to_string())),
                None => Err(EngineError::Other(crate::library_server::NOT_FETCHED.into())),
            };
        }
        // The path comes from a document: a network share would be contacted (and on Windows get
        // the opener's credentials) by the stat below.
        if !craft_docpath::is_local(path) {
            return Err(EngineError::Other(format!("{path}: a library is opened only from this computer")));
        }
        let stamp = stat(path);
        // Checked before the cache: the limit is the caller's.
        if let Ok(st) = &stamp
            && st.len > max_bytes
        {
            return Err(EngineError::Other(format!("{path} is too large to use as a library (the limit is {} MB)", max_bytes / 1024 / 1024)));
        }
        if let (Some(e), Ok(st)) = (self.entries.get(path), &stamp)
            && e.still_valid(st)
        {
            return e.result.clone().map_err(EngineError::Other);
        }
        let read_at = now();
        let result = match &stamp {
            Ok(_) => read_file(path).map(Arc::new),
            Err(e) => Err(e.clone()),
        };
        if self.entries.len() >= MAX_CACHED && !self.entries.contains_key(path) {
            self.entries.clear();
        }
        self.entries.insert(path.to_string(), Entry { stamp: stamp.ok(), read_at, result: result.clone() });
        result.map_err(EngineError::Other)
    }

    /// What the last read of `path` gave (no file access).
    pub fn cached(&self, path: &str) -> Option<std::result::Result<&Arc<PublishedLibrary>, &str>> {
        self.entries.get(path).map(|e| e.result.as_ref().map_err(String::as_str))
    }

    /// Remember `result` for `path` (a library fetched from a server).
    pub fn insert(&mut self, path: &str, result: std::result::Result<Arc<PublishedLibrary>, String>) {
        if self.entries.len() >= MAX_CACHED && !self.entries.contains_key(path) {
            self.entries.clear();
        }
        self.entries.insert(path.to_string(), Entry { stamp: None, read_at: None, result });
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn now() -> Option<std::time::SystemTime> {
    Some(std::time::SystemTime::now())
}

#[cfg(target_arch = "wasm32")]
fn now() -> Option<std::time::SystemTime> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
fn stat(path: &str) -> std::result::Result<Stamp, String> {
    let m = crate::file_access::metadata(path).map_err(|e| e.to_string())?;
    if !m.is_file() {
        return Err(format!("{path} is not a file"));
    }
    Ok(Stamp { len: m.len(), modified: m.modified().ok() })
}

#[cfg(target_arch = "wasm32")]
fn stat(_path: &str) -> std::result::Result<Stamp, String> {
    Err(NO_FILES.into())
}

/// A library read from a file or a server, with the modes chosen inside its instances marked when
/// it was published before they were overrides ([`crate::components::mark_layer_modes`]).
pub(crate) fn settled(mut lib: PublishedLibrary) -> PublishedLibrary {
    crate::components::mark_layer_modes(Arc::make_mut(&mut lib.doc));
    lib
}

#[cfg(not(target_arch = "wasm32"))]
fn read_file(path: &str) -> std::result::Result<PublishedLibrary, String> {
    let f = crate::file_access::open(path).map_err(|e| e.to_string())?;
    match figcraft_format::read_library(std::io::BufReader::new(f)) {
        Ok(Some(lib)) => Ok(settled(lib)),
        Ok(None) => Err(format!("{path} has no published library: open it and publish it first (Libraries ▸ Publish library…)")),
        Err(e) => Err(format!("{path}: {e}")),
    }
}

#[cfg(target_arch = "wasm32")]
fn read_file(_path: &str) -> std::result::Result<PublishedLibrary, String> {
    Err(NO_FILES.into())
}

/// Turn `path` into an absolute path (library links outlive the working directory).
pub fn absolute_path(path: &str) -> String {
    #[cfg(not(target_arch = "wasm32"))]
    if let Ok(p) = std::path::absolute(path) {
        return p.to_string_lossy().into_owned();
    }
    path.to_string()
}

/// The enabled library `key` (its id, name or path).
pub fn find_link<'a>(doc: &'a Document, key: &str) -> Option<&'a LibraryLink> {
    let l = &doc.libraries.enabled;
    l.iter().find(|l| l.id == key).or_else(|| l.iter().find(|l| l.name == key || l.path == key))
}

/// Copy images `hashes` of `src` that `doc` lacks.
fn copy_images(doc: &mut Document, src: &Document, hashes: &BTreeSet<String>) {
    for h in hashes {
        if !doc.images.contains_key(h)
            && let Some(img) = src.images.get(h)
        {
            doc.images.insert(h.clone(), img.clone());
        }
    }
}

/// The record of library `lid`'s `key` among `records` whose copy `exists`.
fn record_of(records: &[ImportedRecord], lid: &str, key: &str, exists: impl Fn(&str) -> bool) -> Option<ImportedRecord> {
    records.iter().find(|r| r.library == lid && r.key == key && exists(&r.local)).cloned()
}

/// Add or replace the record of `rec`'s library asset.
fn set_record(records: &mut Vec<ImportedRecord>, rec: ImportedRecord) {
    match records.iter_mut().find(|r| r.library == rec.library && r.key == rec.key) {
        Some(r) => *r = rec,
        None => records.push(rec),
    }
}

/// A local id for a copy of library `lid`'s `key`: `lid:key`, numbered when `taken`.
fn local_key(lid: &str, key: &str, taken: impl Fn(&str) -> bool) -> String {
    let base = format!("{lid}:{key}");
    (1..10_000).map(|k| if k == 1 { base.clone() } else { format!("{base}#{k}") }).find(|l| !taken(l)).unwrap_or(base)
}

/// Copy library styles `keys` into `doc` as linked remote styles (with `update`, copies older than
/// the library's are refreshed). The variables a written copy binds are imported too and its
/// bindings point at their copies (never at this file's variables of the same ids). Returns library
/// style id → local id.
pub fn import_styles(doc: &mut Document, lib: &PublishedLibrary, keys: &BTreeSet<String>, update: bool) -> BTreeMap<String, String> {
    let lid = lib.manifest.id.clone();
    let mut out = BTreeMap::new();
    let mut hashes = BTreeSet::new();
    // Local ids of the copies written now, with the library style they copy.
    let mut written: Vec<(String, &Style)> = Vec::new();
    for key in keys {
        let Some(style) = lib.doc.styles.iter().find(|s| s.id == *key) else { continue };
        let version = lib.manifest.style(key).map_or(lib.manifest.version, |s| s.version);
        let local = match record_of(&doc.libraries.styles, &lid, key, |l| doc.styles.iter().any(|s| s.id == l)) {
            Some(r) => {
                if update
                    && version > r.version
                    && let Some(slot) = doc.styles.iter_mut().find(|s| s.id == r.local)
                {
                    *slot = Style { id: r.local.clone(), ..style.clone() };
                    set_record(&mut doc.libraries_mut().styles, ImportedStyle { version, ..r.clone() });
                    written.push((r.local.clone(), style));
                }
                r.local
            }
            None => {
                let local = local_key(&lid, key, |l| doc.styles.iter().any(|s| s.id == l));
                doc.styles.push(Style { id: local.clone(), ..style.clone() });
                set_record(&mut doc.libraries_mut().styles, ImportedStyle { library: lid.clone(), key: key.clone(), local: local.clone(), version });
                written.push((local.clone(), style));
                local
            }
        };
        if let StyleKind::Paint { paints } = &style.kind {
            paint_images(paints, &mut hashes);
        }
        out.insert(key.clone(), local);
    }
    let bound: BTreeSet<VariableId> = written.iter().flat_map(|(_, s)| s.bound.values().cloned()).collect();
    if !bound.is_empty() {
        let vars = import_variables(doc, lib, &bound, &BTreeSet::new(), update);
        for (local, _) in &written {
            if let Some(slot) = doc.styles.iter_mut().find(|s| s.id == *local) {
                // Bindings to variables the library lacks are dropped (the published value stays).
                slot.bound =
                    std::mem::take(&mut slot.bound).into_iter().filter_map(|(f, v)| vars.variables.get(&v).map(|l| (f, l.clone()))).collect();
            }
        }
    }
    copy_images(doc, &lib.doc, &hashes);
    out
}

/// Library variable and collection ids → the ids of their local copies.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VariableMap {
    pub variables: BTreeMap<VariableId, VariableId>,
    pub collections: BTreeMap<CollectionId, CollectionId>,
}

/// Copy library variables `keys` (with every variable they alias) and collections `collections`
/// (with those of the variables) into `doc` as linked, read-only copies. Copies the document
/// already has are kept, or with `update` refreshed in place (same ids, so bindings, aliases and
/// explicit modes follow) when the library has a newer version.
pub fn import_variables(
    doc: &mut Document,
    lib: &PublishedLibrary,
    keys: &BTreeSet<VariableId>,
    collections: &BTreeSet<CollectionId>,
    update: bool,
) -> VariableMap {
    let (lid, m, src) = (lib.manifest.id.as_str(), &lib.manifest, &lib.doc.variables);
    let vars = alias_closure(src, keys.iter().cloned());
    let colls: BTreeSet<&CollectionId> = collections.iter().chain(vars.iter().filter_map(|v| src.get(v)).map(|v| &v.collection)).collect();
    let mut out = VariableMap::default();
    // Collections first: the variables name theirs.
    for key in colls {
        let Some(c) = src.collections.iter().find(|c| c.id == *key) else { continue };
        let version = m.collection(key).map_or(m.version, |x| x.version);
        let exists = |d: &Document, l: &str| d.variables.collections.iter().any(|x| x.id == l);
        let local = match record_of(&doc.libraries.collections, lid, key, |l| exists(doc, l)) {
            Some(r) => {
                if update
                    && version > r.version
                    && let Some(slot) = doc.variables.collections.iter_mut().find(|x| x.id == r.local)
                {
                    *slot = Collection { id: r.local.clone(), extends: None, ..c.clone() };
                    set_record(&mut doc.libraries_mut().collections, ImportedRecord { version, ..r.clone() });
                }
                r.local
            }
            None => {
                let local = local_key(lid, key, |l| exists(doc, l));
                doc.variables.collections.push(Collection { id: local.clone(), extends: None, ..c.clone() });
                let rec = ImportedRecord { library: lid.to_string(), key: key.clone(), local: local.clone(), version };
                set_record(&mut doc.libraries_mut().collections, rec);
                local
            }
        };
        out.collections.insert(key.clone(), local);
    }
    // Local ids first (the copies alias each other), then the values.
    let mut write: Vec<(&Variable, VariableId, u64, bool)> = Vec::new();
    for key in &vars {
        let Some(v) = src.variables.iter().find(|x| x.id == *key).filter(|v| out.collections.contains_key(&v.collection)) else { continue };
        let version = m.variable(key).map_or(m.version, |x| x.version);
        let local = match record_of(&doc.libraries.variables, lid, key, |l| doc.variables.variables.iter().any(|x| x.id == l)) {
            Some(r) => {
                if update && version > r.version {
                    write.push((v, r.local.clone(), version, false));
                }
                r.local
            }
            None => {
                let taken = |l: &str| doc.variables.variables.iter().any(|x| x.id == l) || out.variables.values().any(|x| x == l);
                let local = local_key(lid, key, taken);
                write.push((v, local.clone(), version, true));
                local
            }
        };
        out.variables.insert(key.clone(), local);
    }
    for (v, local, version, new) in write {
        let Some(collection) = out.collections.get(&v.collection).cloned() else { continue };
        let mut values = BTreeMap::new();
        for (mode, val) in &v.values {
            let val = match val {
                // An alias to a variable the library doesn't have resolves to nothing: dropped
                // (never pointed at a variable of this file).
                VariableValue::Alias(a) => match src.get(a).and_then(|t| out.variables.get(&t.id)) {
                    Some(l) => VariableValue::Alias(l.clone()),
                    None => continue,
                },
                other => other.clone(),
            };
            values.insert(mode.clone(), val);
        }
        let copy = Variable { id: local.clone(), collection, values, extends: None, overrides: Default::default(), ..v.clone() };
        match doc.variables.variables.iter_mut().find(|x| x.id == local).filter(|_| !new) {
            Some(slot) => *slot = copy,
            None => doc.variables.variables.push(copy),
        }
        set_record(&mut doc.libraries_mut().variables, ImportedRecord { library: lid.to_string(), key: v.id.clone(), local, version });
    }
    out
}

/// Point a layer copied from `src` at its new ids. [`Document::paste_nodes`] maps ids, children,
/// parents, instance mains and prototype destinations; this maps the rest: mirrored layers,
/// instance swap properties and (with `styles` and `vars`) style links, variable bindings and
/// explicit modes; references the maps don't know are dropped (bindings with their values kept).
fn localize(n: &mut Node, src: &Document, map: &BTreeMap<NodeId, NodeId>, styles: Option<&BTreeMap<String, String>>, vars: Option<&VariableMap>) {
    n.source = n.source.and_then(|s| map.get(&s).copied());
    let swaps = match &n.kind {
        NodeKind::Instance { component, .. } if !map.contains_key(component) => {
            // Its main isn't there: keep the layers as a plain frame.
            n.kind = NodeKind::Frame;
            Vec::new()
        }
        NodeKind::Instance { component, .. } => swap_props(src, *component),
        _ => Vec::new(),
    };
    match &mut n.kind {
        NodeKind::Instance { props, .. } => {
            for k in swaps {
                match props.get(&k).and_then(id_of).and_then(|c| map.get(&c)) {
                    Some(l) => {
                        props.insert(k, Value::from(l.0));
                    }
                    None => {
                        props.remove(&k);
                    }
                }
            }
        }
        NodeKind::Component { props, .. } => {
            props.retain(|_, d| !matches!(d, PropDef::InstanceSwap { default, .. } if !map.contains_key(default)));
            for d in props.values_mut() {
                match d {
                    PropDef::InstanceSwap { default, preferred } => {
                        if let Some(l) = map.get(default) {
                            *default = *l;
                        }
                        *preferred = preferred.iter().filter_map(|c| map.get(c).copied()).collect();
                    }
                    PropDef::Slot { preferred } => *preferred = preferred.iter().filter_map(|c| map.get(c).copied()).collect(),
                    _ => {}
                }
            }
        }
        _ => {}
    }
    if let Some(styles) = styles {
        for r in [&mut n.styles.fill, &mut n.styles.stroke, &mut n.styles.effect, &mut n.styles.grid, &mut n.styles.text] {
            *r = r.take().and_then(|s| styles.get(&s).cloned());
        }
        if let NodeKind::Text(t) = &mut n.kind {
            t.map_text_styles(|s| styles.get(s).cloned());
        }
    }
    if let Some(vars) = vars {
        n.bound = std::mem::take(&mut n.bound).into_iter().filter_map(|(f, v)| vars.variables.get(&v).map(|l| (f, l.clone()))).collect();
        n.var_modes = std::mem::take(&mut n.var_modes).into_iter().filter_map(|(c, m)| vars.collections.get(&c).map(|l| (l.clone(), m))).collect();
    }
    n.plugin_data.remove("figcraft.thumbnail");
    for it in &mut n.interactions {
        // Variable actions stay with the library's file.
        figcraft_doc::proto::retain_deep(&mut it.actions, &mut |a| {
            !matches!(a, Action::SetVariable { .. } | Action::SetVariableMode { .. } | Action::Conditional { .. })
                && a.dest().is_none_or(|d| map.contains_key(&d))
        });
    }
    n.interactions.retain(|it| !it.actions.is_empty());
}

/// Copy library assets `keys` (any layer of an asset names it), with every asset and style they
/// use, into `doc`'s library store. Copies the document already has are kept, or with `update`
/// re-imported in place (same layer ids, so instances keep their overrides) when the library has
/// a newer version. Returns library id → local id for every layer of those assets.
pub fn import(doc: &mut Document, lib: &PublishedLibrary, keys: &[NodeId], update: bool) -> Result<BTreeMap<NodeId, NodeId>> {
    let src = lib.doc.as_ref();
    let lid = lib.manifest.id.clone();
    let roots: Vec<NodeId> = keys.iter().filter_map(|k| asset_root(src, *k)).collect();
    if roots.is_empty() {
        return Err(EngineError::Other("no such component in the library".into()));
    }
    let units = closure(src, &roots);
    let store = doc.ensure_library_store();
    let records: BTreeMap<NodeId, ImportedComponent> =
        doc.libraries.components.iter().filter(|r| r.library == lid && doc.contains(r.root)).map(|r| (r.key, r.clone())).collect();
    let version_of = |u: NodeId| lib.manifest.component(u).map_or(lib.manifest.version, |a| a.version);
    let write: Vec<(NodeId, Vec<NodeId>)> =
        units.iter().filter(|u| records.get(u).is_none_or(|r| update && version_of(**u) > r.version)).map(|u| (*u, src.subtree(*u))).collect();
    // Kept copies keep their ids; re-imported ones reuse them where the library layer remains.
    let mut map: BTreeMap<NodeId, NodeId> = BTreeMap::new();
    for u in &units {
        if let Some(r) = records.get(u)
            && !write.iter().any(|(w, _)| w == u)
        {
            map.extend(r.ids.iter().map(|(a, b)| (*a, *b)));
        }
    }
    let mut used: BTreeSet<NodeId> = map.values().copied().collect();
    for (u, sub) in &write {
        let old = records.get(u);
        let old_layers: BTreeSet<NodeId> = old.map(|r| doc.subtree(r.root).into_iter().collect()).unwrap_or_default();
        for id in sub {
            let reuse = old.and_then(|r| r.ids.get(id)).copied().filter(|l| (old_layers.contains(l) || !doc.contains(*l)) && !used.contains(l));
            let local = reuse.unwrap_or_else(|| doc.alloc_id());
            used.insert(local);
            map.insert(*id, local);
        }
    }
    let roots_written: Vec<NodeId> = write.iter().map(|(u, _)| *u).collect();
    let styles = import_styles(doc, lib, &linked_styles(src, &roots_written), update);
    let (bound, mode_colls) = used_variables(src, &roots_written);
    let vars = import_variables(doc, lib, &bound, &mode_colls, update);
    for (u, _) in &write {
        if let Some(r) = records.get(u)
            && doc.contains(r.root)
        {
            doc.remove(r.root)?;
        }
    }
    let mut hashes = BTreeSet::new();
    for (u, sub) in &write {
        let mut nodes = Vec::with_capacity(sub.len());
        for id in sub {
            let Some(mut n) = src.node(*id).cloned() else { continue };
            localize(&mut n, src, &map, Some(&styles), Some(&vars));
            node_images(&n, &mut hashes);
            nodes.push(n);
        }
        doc.paste_nodes(&nodes, &map, *u, store, None)?;
    }
    copy_images(doc, src, &hashes);
    for (u, sub) in &write {
        let Some(root) = map.get(u).copied() else { continue };
        let ids = sub.iter().filter_map(|i| map.get(i).map(|l| (*i, *l))).collect();
        let rec = ImportedComponent { library: lid.clone(), key: *u, root, version: version_of(*u), ids };
        let libs = doc.libraries_mut();
        match libs.components.iter_mut().find(|r| r.library == lid && r.key == *u) {
            Some(r) => *r = rec,
            None => libs.components.push(rec),
        }
    }
    Ok(map)
}

/// A document's copies of one library's assets (the records), e.g. those with pending updates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pending {
    pub components: Vec<ImportedComponent>,
    pub styles: Vec<ImportedRecord>,
    pub collections: Vec<ImportedRecord>,
    pub variables: Vec<ImportedRecord>,
}

impl Pending {
    pub fn count(&self) -> usize {
        self.components.len() + self.styles.len() + self.collections.len() + self.variables.len()
    }

    pub fn is_empty(&self) -> bool {
        self.count() == 0
    }
}

/// The records of library `lid` among `records` whose copy `exists` and whose library `version` is
/// newer.
fn stale(records: &[ImportedRecord], lid: &str, exists: impl Fn(&str) -> bool, version: impl Fn(&str) -> Option<u64>) -> Vec<ImportedRecord> {
    records.iter().filter(|r| r.library == lid && exists(&r.local) && version(&r.key).is_some_and(|v| v > r.version)).cloned().collect()
}

/// Pending updates of `doc`'s copies of library `lib`. Variables a library collection gained count
/// too when one of `doc`'s collections extends it (they have no copy yet: `local` is empty).
pub fn pending(doc: &Document, lib: &PublishedLibrary) -> Pending {
    let (m, libs, vars) = (&lib.manifest, &doc.libraries, &doc.variables);
    let mut variables = stale(&libs.variables, &m.id, |l| vars.variables.iter().any(|v| v.id == l), |k| m.variable(k).map(|x| x.version));
    let extended: BTreeSet<&str> = vars
        .collections
        .iter()
        .filter_map(|c| libs.collection_import(c.extends.as_deref()?))
        .filter(|r| r.library == m.id)
        .map(|r| r.key.as_str())
        .collect();
    let copied = |key: &str| libs.variables.iter().any(|r| r.library == m.id && r.key == key && vars.get(&r.local).is_some());
    variables.extend(published_variables(lib).filter(|v| extended.contains(v.collection.as_str()) && !copied(&v.id)).map(|v| ImportedRecord {
        library: m.id.clone(),
        key: v.id.clone(),
        local: String::new(),
        version: 0,
    }));
    Pending {
        components: libs
            .components
            .iter()
            .filter(|r| r.library == m.id && doc.contains(r.root) && m.component(r.key).is_some_and(|a| a.version > r.version))
            .cloned()
            .collect(),
        styles: stale(&libs.styles, &m.id, |l| doc.styles.iter().any(|s| s.id == l), |k| m.style(k).map(|x| x.version)),
        collections: stale(&libs.collections, &m.id, |l| vars.collections.iter().any(|c| c.id == l), |k| m.collection(k).map(|x| x.version)),
        variables,
    }
}

/// Pull library `lib`'s updates of the copies `doc` has (instances keep their overrides, bound
/// layers re-resolve). Returns what was updated.
pub fn accept_updates(doc: &mut Document, lib: &PublishedLibrary) -> Result<Pending> {
    let p = pending(doc, lib);
    if !p.components.is_empty() {
        let keys: Vec<NodeId> = p.components.iter().map(|r| r.key).collect();
        import(doc, lib, &keys, true)?;
    }
    if !p.styles.is_empty() {
        import_styles(doc, lib, &p.styles.iter().map(|r| r.key.clone()).collect(), true);
    }
    if !p.variables.is_empty() || !p.collections.is_empty() {
        let keys = |list: &[ImportedRecord]| list.iter().map(|r| r.key.clone()).collect::<BTreeSet<_>>();
        import_variables(doc, lib, &keys(&p.variables), &keys(&p.collections), true);
    }
    crate::cmd::variables::sync_extensions(doc);
    let m = &lib.manifest;
    if let Some(l) = doc.libraries_mut().enabled.iter_mut().find(|l| l.id == m.id)
        && (l.version != m.version || l.name != m.name)
    {
        l.version = m.version;
        l.name = m.name.clone();
    }
    prune(doc, &m.id)?;
    Ok(p)
}

/// Drop copies of `library`'s components, styles, variables and collections that nothing in the
/// document uses any more.
pub fn prune(doc: &mut Document, library: &str) -> Result<()> {
    for _ in 0..MAX_ASSETS {
        // Which copies instances outside them use.
        let mut used: BTreeSet<NodeId> = BTreeSet::new();
        for n in doc.nodes() {
            if let NodeKind::Instance { component, .. } = &n.kind
                && let Some(root) = asset_root(doc, *component)
                && !doc.is_ancestor(root, n.id)
            {
                used.insert(root);
            }
        }
        let unused: Vec<ImportedComponent> =
            doc.libraries.components.iter().filter(|r| r.library == library && !used.contains(&r.root)).cloned().collect();
        if unused.is_empty() {
            break;
        }
        for r in &unused {
            if doc.contains(r.root) && doc.is_remote(r.root) {
                doc.remove(r.root)?;
            }
        }
        doc.libraries_mut().components.retain(|r| !unused.contains(r));
    }
    let linked: BTreeSet<String> = doc.nodes().flat_map(|n| style_links(n).cloned()).collect();
    let unused: Vec<String> =
        doc.libraries.styles.iter().filter(|r| r.library == library && !linked.contains(&r.local)).map(|r| r.local.clone()).collect();
    if !unused.is_empty() {
        doc.styles.retain(|s| !unused.contains(&s.id));
        doc.libraries_mut().styles.retain(|r| !unused.contains(&r.local));
    }
    // Variables layers bind or set, local variables alias or extend (with what those alias).
    let seeds = doc
        .nodes()
        .flat_map(|n| n.bound.values().map(String::as_str).chain(action_variables(n)))
        .chain(doc.variables.variables.iter().filter_map(|v| v.extends.as_deref()))
        .map(str::to_string)
        .chain(doc.variables.variables.iter().filter(|v| !is_remote_variable(doc, &v.id)).flat_map(|v| v.values.values()).filter_map(|x| match x {
            VariableValue::Alias(a) => Some(a.clone()),
            _ => None,
        }))
        .collect::<Vec<_>>();
    let used = alias_closure(&doc.variables, seeds);
    let unused: BTreeSet<String> =
        doc.libraries.variables.iter().filter(|r| r.library == library && !used.contains(&r.local)).map(|r| r.local.clone()).collect();
    if !unused.is_empty() {
        doc.variables.variables.retain(|v| !unused.contains(&v.id));
        doc.libraries_mut().variables.retain(|r| !unused.contains(&r.local));
    }
    // Collections the remaining variables are in, or layers (and their prototype actions) choose a
    // mode of.
    let action_modes = doc.nodes().flat_map(|n| &n.interactions).flat_map(|it| figcraft_doc::proto::walk(&it.actions)).filter_map(|a| match a {
        Action::SetVariableMode { collection, .. } => Some(collection),
        _ => None,
    });
    let used: BTreeSet<String> = doc
        .variables
        .variables
        .iter()
        .map(|v| &v.collection)
        .chain(doc.nodes().flat_map(|n| n.var_modes.keys()))
        .chain(doc.variables.collections.iter().filter_map(|c| c.extends.as_ref()))
        .chain(action_modes)
        .cloned()
        .collect();
    let unused: BTreeSet<String> =
        doc.libraries.collections.iter().filter(|r| r.library == library && !used.contains(&r.local)).map(|r| r.local.clone()).collect();
    if !unused.is_empty() {
        doc.variables.collections.retain(|c| !unused.contains(&c.id));
        doc.libraries_mut().collections.retain(|r| !unused.contains(&r.local));
    }
    Ok(())
}

/// `theirs` records of copies `exists` in this document that `mine` doesn't list yet.
fn new_records(mine: &[ImportedRecord], theirs: &[ImportedRecord], exists: impl Fn(&str) -> bool) -> Vec<ImportedRecord> {
    theirs.iter().filter(|s| exists(&s.local) && !mine.iter().any(|x| x.library == s.library && x.key == s.key)).cloned().collect()
}

/// When pages of `other` are appended to `doc` (`map`: their ids → the pasted ids): the library
/// copies they use come along (shared with the copies `doc` has of the same assets), with their
/// records and library links, and the pasted layers point at them. Styles and variables are
/// appended by id beforehand: this records the copies among them.
pub fn merge_store(doc: &mut Document, other: &Document, map: &BTreeMap<NodeId, NodeId>) -> Result<()> {
    if other.libraries.is_empty() {
        return Ok(());
    }
    if other.library_store().is_some() {
        merge_components(doc, other, map, None)?;
    }
    let styles = new_records(&doc.libraries.styles, &other.libraries.styles, |l| doc.styles.iter().any(|s| s.id == l));
    let colls = new_records(&doc.libraries.collections, &other.libraries.collections, |l| doc.variables.collections.iter().any(|c| c.id == l));
    let vars = new_records(&doc.libraries.variables, &other.libraries.variables, |l| doc.variables.variables.iter().any(|v| v.id == l));
    let libs = doc.libraries_mut();
    libs.styles.extend(styles);
    libs.collections.extend(colls);
    libs.variables.extend(vars);
    for l in &other.libraries.enabled {
        if libs.link(&l.id).is_none() {
            libs.enabled.push(l.clone());
        }
    }
    Ok(())
}

/// The component copies of [`merge_store`] (only those rooted at `only` when given). Returns
/// `other`'s ids of their layers → `doc`'s.
fn merge_components(
    doc: &mut Document,
    other: &Document,
    map: &BTreeMap<NodeId, NodeId>,
    only: Option<&BTreeSet<NodeId>>,
) -> Result<BTreeMap<NodeId, NodeId>> {
    let mut remote: BTreeMap<NodeId, NodeId> = BTreeMap::new();
    let mut fresh = Vec::new();
    for r in other.libraries.components.iter().filter(|r| other.contains(r.root) && only.is_none_or(|o| o.contains(&r.root))) {
        match doc.libraries.components.iter().find(|m| m.library == r.library && m.key == r.key && doc.contains(m.root)) {
            Some(mine) => remote.extend(r.ids.iter().filter_map(|(lib, theirs)| mine.ids.get(lib).map(|l| (*theirs, *l)))),
            None => fresh.push(r.clone()),
        }
    }
    for r in &fresh {
        for id in other.subtree(r.root) {
            remote.insert(id, doc.alloc_id());
        }
    }
    if !fresh.is_empty() {
        let store = doc.ensure_library_store();
        for r in &fresh {
            let mut nodes: Vec<Node> = other.subtree(r.root).into_iter().filter_map(|i| other.node(i).cloned()).collect();
            for n in &mut nodes {
                localize(n, other, &remote, None, None);
            }
            let Some(root) = remote.get(&r.root).copied() else { continue };
            doc.paste_nodes(&nodes, &remote, r.root, store, None)?;
            let ids = r.ids.iter().filter_map(|(lib, theirs)| remote.get(theirs).map(|l| (*lib, *l))).collect();
            doc.libraries_mut().components.push(ImportedComponent { root, ids, ..r.clone() });
        }
    }
    // The pasted layers still name `other`'s copies: point them at `doc`'s.
    for (theirs, id) in map {
        let Some(n) = other.node(*theirs) else { continue };
        let main = match &n.kind {
            NodeKind::Instance { component, .. } => remote.get(component).copied(),
            _ => None,
        };
        let source = n.source.and_then(|s| remote.get(&s).copied());
        if (main.is_none() && source.is_none()) || !doc.contains(*id) {
            continue;
        }
        let n = doc.get_mut(*id)?;
        if let (NodeKind::Instance { component, .. }, Some(m)) = (&mut n.kind, main) {
            *component = m;
        }
        if source.is_some() {
            n.source = source;
        }
    }
    Ok(remote)
}

/// When layers copied from another document `src` are pasted into `doc` (`map`: their ids → the
/// pasted ids), point the instances among them at mains of `doc`: library copies are shared with
/// (or brought along as) `doc`'s copies of the same library assets; components `src` publishes as
/// a library are used from that library (enabled when `src` is saved at `src_path`); other mains
/// stay behind, so those instances show a missing main (restorable). Links into `src` that can't
/// be followed never point at an unrelated layer of `doc`.
pub fn adopt_pasted(doc: &mut Document, src: &Document, src_path: Option<&str>, map: &BTreeMap<NodeId, NodeId>) -> Result<()> {
    // The mains the pasted instances show and offer as instance swap values (`src`'s ids).
    let mut mains = BTreeSet::new();
    for theirs in map.keys() {
        let Some(NodeKind::Instance { component, props }) = src.node(*theirs).map(|n| &n.kind) else { continue };
        mains.insert(*component);
        for k in swap_props(src, *component) {
            mains.extend(props.get(&k).and_then(id_of));
        }
    }
    let roots: Vec<NodeId> = mains.iter().filter_map(|m| asset_root(src, *m)).collect::<BTreeSet<_>>().into_iter().collect();
    let mut remote: BTreeMap<NodeId, NodeId> = BTreeMap::new();
    // Library copies: of this file's own library they are its own layers; of other libraries they
    // join (or come along as) this file's copies.
    let own = doc.published.as_ref().map(|p| p.manifest.id.clone());
    let lib_roots: Vec<NodeId> = roots.iter().copied().filter(|r| src.is_remote(*r)).collect();
    let mut foreign = BTreeSet::new();
    for root in closure(src, &lib_roots) {
        let Some(r) = src.libraries.import_of(root) else { continue };
        if own.as_deref() == Some(r.library.as_str()) {
            remote.extend(r.ids.iter().filter(|(lib, _)| doc.contains(**lib)).map(|(lib, theirs)| (*theirs, *lib)));
        } else {
            foreign.insert(root);
            if doc.libraries.link(&r.library).is_none()
                && let Some(l) = src.libraries.link(&r.library)
            {
                doc.libraries_mut().enabled.push(l.clone());
            }
        }
    }
    if !foreign.is_empty() {
        remote.extend(merge_components(doc, src, &BTreeMap::new(), Some(&foreign))?);
    }
    // Components `src` publishes: from its library.
    if let Some(lib) = src.published.as_ref().filter(|l| own.as_deref() != Some(l.manifest.id.as_str())) {
        let keys: Vec<NodeId> =
            roots.iter().copied().filter(|r| !src.is_remote(*r) && lib.manifest.component(*r).is_some() && lib.doc.contains(*r)).collect();
        if !keys.is_empty() {
            remote.extend(import(doc, lib, &keys, false)?);
            let m = &lib.manifest;
            if doc.libraries.link(&m.id).is_none()
                && let Some(path) = src_path
            {
                doc.libraries_mut().enabled.push(LibraryLink { id: m.id.clone(), name: m.name.clone(), path: path.to_string(), version: m.version });
            }
        }
    }
    // Point the pasted layers at those mains; what stays behind gets ids of its own.
    let mut behind: BTreeMap<NodeId, NodeId> = BTreeMap::new();
    let mut follow = |doc: &mut Document, id: NodeId| -> NodeId {
        if let Some(m) = map.get(&id).or_else(|| remote.get(&id)) {
            return *m;
        }
        *behind.entry(id).or_insert_with(|| doc.alloc_id())
    };
    for (theirs, mine) in map {
        let Some(n) = src.node(*theirs) else { continue };
        let source = n.source.map(|s| follow(doc, s));
        let kind = match &n.kind {
            NodeKind::Instance { component, props } => {
                let swaps = swap_props(src, *component);
                let mut props = props.clone();
                for k in swaps {
                    match props.get(&k).and_then(id_of).and_then(|c| map.get(&c).or_else(|| remote.get(&c)).copied()) {
                        Some(l) => {
                            props.insert(k, Value::from(l.0));
                        }
                        None => {
                            props.remove(&k);
                        }
                    }
                }
                Some(NodeKind::Instance { component: follow(doc, *component), props })
            }
            _ => None,
        };
        let Ok(m) = doc.get_mut(*mine) else { continue };
        m.source = source;
        if let Some(k) = kind {
            m.kind = k;
        }
    }
    Ok(())
}

/// Refuse edits of library copies, except by library commands (and imports of other files,
/// which bring their copies along).
pub fn check_read_only(cmd: &str, before: &Document, after: &Document) -> std::result::Result<(), String> {
    // Pastes from other files bring the library copies their instances use.
    if cmd.starts_with("library.") || cmd == "file.import" || cmd.starts_with("edit.paste") {
        return Ok(());
    }
    if !variables_unchanged(before, after) {
        return Err(READ_ONLY.into());
    }
    let changed_remote = |id| {
        if !before.is_remote(id) && !after.is_remote(id) {
            return false;
        }
        match (before.node_arc(id), after.node_arc(id)) {
            (Some(old), Some(new)) => !Arc::ptr_eq(old, new),
            _ => true,
        }
    };
    if after.dirty().iter().any(|id| changed_remote(*id)) {
        return Err(READ_ONLY.into());
    }
    Ok(())
}

/// Whether the library variables and collections copied into `before` are the same in `after`,
/// and no other variable joined one of those collections. A document whose library data was
/// replaced (undo, redo, a restored version) brings its own copies.
fn variables_unchanged(before: &Document, after: &Document) -> bool {
    let libs = &before.libraries;
    if (libs.variables.is_empty() && libs.collections.is_empty()) || !Arc::ptr_eq(libs, &after.libraries) || before.variables == after.variables {
        return true;
    }
    let (b, a) = (&before.variables, &after.variables);
    fn by_id<T>(list: &[T], id: impl Fn(&T) -> &str) -> HashMap<&str, &T> {
        list.iter().map(|x| (id(x), x)).collect()
    }
    let (bv, av) = (by_id(&b.variables, |x| &x.id), by_id(&a.variables, |x| &x.id));
    let (bc, ac) = (by_id(&b.collections, |x| &x.id), by_id(&a.collections, |x| &x.id));
    libs.variables.iter().all(|r| bv.get(r.local.as_str()) == av.get(r.local.as_str()))
        && libs.collections.iter().all(|r| bc.get(r.local.as_str()) == ac.get(r.local.as_str()))
        && a.variables.iter().all(|v| libs.collection_import(&v.collection).is_none() || after.libraries.variable_import(&v.id).is_some())
}

/// The library a remote layer was copied from (its id), if any.
pub fn library_of(doc: &Document, id: NodeId) -> Option<&str> {
    if !doc.is_remote(id) {
        return None;
    }
    let root = asset_root(doc, id)?;
    doc.libraries.import_of(root).map(|r| r.library.as_str())
}

/// The display name of library `id` in `doc` (its link's name, else the id).
pub fn library_name<'a>(doc: &'a Document, id: &'a str) -> &'a str {
    doc.libraries.link(id).map_or(id, |l| l.name.as_str())
}

/// The enabled libraries of `doc` that `cache` has read (and that still hold the linked library).
pub fn read_libraries<'a>(cache: &'a Cache, doc: &'a Document) -> impl Iterator<Item = (&'a LibraryLink, &'a Arc<PublishedLibrary>)> {
    doc.libraries.enabled.iter().filter_map(|l| match cache.cached(&l.path) {
        Some(Ok(lib)) if lib.manifest.id == l.id => Some((l, lib)),
        _ => None,
    })
}

/// The variable collections library `lib` offers.
pub fn published_collections(lib: &PublishedLibrary) -> impl Iterator<Item = &Collection> {
    let m = &lib.manifest;
    lib.doc.variables.collections.iter().filter(|c| m.collection(&c.id).is_some_and(|x| x.published))
}

/// The variables library `lib` offers (not those hidden from publishing).
pub fn published_variables(lib: &PublishedLibrary) -> impl Iterator<Item = &Variable> {
    let offered: BTreeSet<&str> = lib.manifest.variables.iter().filter(|v| v.published).map(|v| v.id.as_str()).collect();
    lib.doc.variables.variables.iter().filter(move |v| offered.contains(v.id.as_str()))
}

#[cfg(test)]
#[path = "library_tests.rs"]
mod tests;
