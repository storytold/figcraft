//! Asset handoff: the files a design's images and vectors become in a code project
//! (`file.exportAssets`, the MCP tool `export_assets`). An image is written as the bytes the file
//! holds, never re-encoded; a vector goes through the SVG export and a layer with export settings
//! through those settings. Names come from the content, so the same asset always has the same file
//! and a second export writes nothing. A result says of every file what it is (`source`), so an
//! agent never takes a render for an original.

use std::collections::BTreeMap;
use std::sync::Arc;

use figcraft_color::{Paint, PaintKind};
use figcraft_doc::{Document, ExportFormat, NodeId};
use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::cmd::file::{export_file, export_scale};
use crate::devinspect::{Asset, AssetKind, MAX_ASSETS, assets};
use crate::file_access;
use crate::params::{get, targets};
use crate::{Result, Session, bad, spec};

/// How much of the content hash names a file: 64 bits, far past a project's assets.
const NAME_HEX: usize = 16;
/// The largest file a repeated export reads back to compare, beyond the images a document holds.
const MAX_COMPARE: u64 = 1 << 31;

/// A file to hand over.
struct File {
    /// `<content hash>.<extension>`.
    name: String,
    hash: String,
    bytes: Arc<Vec<u8>>,
    /// `original`: the file's own bytes. `vector`: written from its shapes. `rendered`: pixels.
    source: &'static str,
    format: String,
    size: Option<(u32, u32)>,
    /// The layers that use it.
    layers: Vec<Value>,
    /// What a layer does to the image that isn't in its bytes.
    adjustments: Vec<Value>,
}

/// The extension encoded image `bytes` go by, from their signature.
fn image_kind(b: &[u8]) -> Option<&'static str> {
    match b {
        [0x89, b'P', b'N', b'G', ..] => Some("png"),
        [0xFF, 0xD8, ..] => Some("jpg"),
        [b'G', b'I', b'F', ..] => Some("gif"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("webp"),
        [b'I', b'I', 42, 0, ..] | [b'M', b'M', 0, 42, ..] => Some("tiff"),
        [b'B', b'M', ..] => Some("bmp"),
        _ => None,
    }
}

/// What paint `p` of a layer changes about its image, beyond the original's bytes.
fn adjustments(p: &Paint) -> Option<Value> {
    let PaintKind::Image(i) = &p.kind else { return None };
    let mut changed = serde_json::Map::new();
    if i.filters != figcraft_color::ImageFilters::default() {
        changed.insert("filters".into(), serde_json::to_value(i.filters).unwrap_or(Value::Null));
    }
    if let Some(crop) = i.crop {
        changed.insert("crop".into(), json!(crop));
    }
    if i.rotation != 0 {
        changed.insert("rotation".into(), json!(i.rotation * 90));
    }
    if p.opacity < 1.0 {
        changed.insert("opacity".into(), json!(p.opacity));
    }
    (!changed.is_empty()).then_some(Value::Object(changed))
}

/// Adds a file, or the layer to the file it is already.
fn add(files: &mut BTreeMap<String, File>, mut file: File) {
    match files.get_mut(&file.name) {
        Some(known) => {
            known.layers.append(&mut file.layers);
            known.adjustments.append(&mut file.adjustments);
        }
        None => {
            files.insert(file.name.clone(), file);
        }
    }
}

/// The image paints of layer `id`: their original bytes, or why one can't be handed over.
fn image_files(doc: &Document, id: NodeId, files: &mut BTreeMap<String, File>, skipped: &mut Vec<Value>) {
    let Some(n) = doc.node(id) else { return };
    for p in n.fills.iter().chain(&n.strokes).filter(|p| p.visible) {
        let PaintKind::Image(paint) = &p.kind else { continue };
        let refuse = |why: &str| json!({"id": id, "name": n.name, "image": paint.image, "why": why});
        let Some(data) = doc.images.get(&paint.image).filter(|d| !d.bytes.is_empty()) else {
            skipped.push(refuse("the file doesn't hold this image"));
            continue;
        };
        let Some(ext) = image_kind(&data.bytes) else {
            skipped.push(refuse("not an image format a project can use as it is"));
            continue;
        };
        let hash = figcraft_format::image_hash(&data.bytes);
        let name = format!("{}.{ext}", hash.get(..NAME_HEX).unwrap_or(&hash));
        let adjusted = adjustments(p).map(|changes| json!({"id": id, "changes": changes}));
        let file = File {
            name,
            hash,
            bytes: data.bytes.clone(),
            source: "original",
            format: ext.to_string(),
            size: Some((data.width, data.height)),
            layers: vec![json!({"id": id, "name": n.name})],
            adjustments: adjusted.into_iter().collect(),
        };
        add(files, file);
    }
}

/// The files the assets in and under `ids` make, and what can't be handed over.
fn collect(doc: &Document, listed: &[Asset]) -> (BTreeMap<String, File>, Vec<Value>) {
    let mut files = BTreeMap::new();
    let mut skipped = Vec::new();
    for asset in listed {
        // A layer's export settings say how it renders; its images stay available as they are.
        image_files(doc, asset.id, &mut files, &mut skipped);
        if asset.kind == AssetKind::Image {
            continue;
        }
        let name = doc.node(asset.id).map(|n| n.name.clone()).unwrap_or_default();
        for setting in &asset.settings {
            let scale = export_scale(doc, &[asset.id], setting);
            let bytes = match export_file(doc, &[asset.id], setting, scale) {
                Ok((bytes, _)) => bytes,
                Err(e) => {
                    skipped.push(json!({"id": asset.id, "name": name, "why": e.to_string()}));
                    continue;
                }
            };
            let hash = figcraft_format::image_hash(&bytes);
            let vector = matches!(setting.format, ExportFormat::Svg);
            let file = File {
                name: format!("{}.{}", hash.get(..NAME_HEX).unwrap_or(&hash), setting.format.ext()),
                hash,
                bytes: Arc::new(bytes),
                source: if vector { "vector" } else { "rendered" },
                format: setting.format.ext().to_string(),
                size: None,
                layers: vec![json!({"id": asset.id, "name": name})],
                adjustments: vec![],
            };
            add(&mut files, file);
        }
    }
    (files, skipped)
}

/// Whether `path` already holds exactly `file`.
fn already_there(path: &std::path::Path, file: &File) -> bool {
    file_access::metadata(path).is_ok_and(|m| m.len() == file.bytes.len() as u64)
        && file_access::read_capped(path, MAX_COMPARE).is_ok_and(|bytes| bytes == *file.bytes)
}

fn export_assets(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.exportAssets";
    let dir = match get(p, "dir") {
        None | Some(Value::Null) => None,
        Some(Value::String(d)) if !d.trim().is_empty() => Some(d.clone()),
        Some(_) => return Err(bad(cmd, "dir: a folder")),
    };
    let doc = s.doc()?;
    let mut ids = targets(s, p);
    if ids.is_empty() {
        ids = vec![s.page()?];
    }
    let found = assets(doc, &ids);
    let (files, skipped) = collect(doc, &found);
    let mut listed = Vec::with_capacity(files.len());
    let (mut written, mut reused) = (0usize, 0usize);
    for file in files.values() {
        let mut entry = json!({
            "file": file.name,
            "hash": file.hash,
            "source": file.source,
            "format": file.format,
            "bytes": file.bytes.len(),
            "layers": file.layers,
        });
        if let Some((w, h)) = file.size {
            entry["width"] = json!(w);
            entry["height"] = json!(h);
        }
        if !file.adjustments.is_empty() {
            entry["adjustments"] = json!(file.adjustments);
        }
        if let Some(d) = &dir {
            let path = std::path::Path::new(d).join(&file.name);
            if already_there(&path, file) {
                reused += 1;
            } else {
                file_access::create_dir_all(d)?;
                file_access::write(&path, &file.bytes)?;
                written += 1;
            }
            entry["path"] = json!(path.to_string_lossy());
        }
        listed.push(entry);
    }
    let mut out = json!({"assets": listed, "skipped": skipped});
    if found.len() >= MAX_ASSETS {
        out["truncated"] = json!({"maxAssets": MAX_ASSETS, "hint": "call again with the ids of the parts you need"});
    }
    if let Some(d) = &dir {
        out["dir"] = json!(d);
        out["written"] = json!(written);
        out["reused"] = json!(reused);
    }
    Ok(out)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![spec!(
        "file.exportAssets",
        "Export assets",
        "{ids? (default the selection, else the page), dir? (write them there; without it they are only listed)}  the images and vectors in and under the layers as files named by their content: images as the file's original bytes, vectors as SVG, layers with export settings by those settings. A file already there isn't written again  → {assets: [{file, hash, source: original|vector|rendered, format, bytes, width?, height?, layers, adjustments?, path?}], skipped: [{id, name, why}], dir?, written?, reused?, truncated?}",
        false,
        export_assets
    )]
}
