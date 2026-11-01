use super::load::{ext, write};
use super::save::thumbnail;
use super::*;

/// Encode `ids` (or the page) in `format` at `scale`.
pub fn export_bytes(doc: &Document, ids: &[NodeId], format: ExportFormat, scale: f64) -> Result<Vec<u8>> {
    export_bytes_with(doc, ids, &ExportSetting { format, ..Default::default() }, scale)
}

/// The page area exporting `ids` with `setting` covers: what they paint (a clipping frame's own
/// area), or their boxes with "include bounding box" and for slices. Hidden layers paint nothing,
/// so they take no room next to visible ones (a hidden-only export keeps its own area).
pub fn export_area(doc: &Document, ids: &[NodeId], setting: &ExportSetting) -> Option<figcraft_geom::Rect> {
    let any_visible = ids.iter().any(|i| doc.node(*i).is_some_and(|n| n.visible));
    ids.iter()
        .filter_map(|i| {
            let node = doc.node(*i);
            let slice = node.is_some_and(|n| matches!(n.kind, NodeKind::Slice));
            if setting.include_bounding_box || slice {
                doc.world_bounds(*i)
            } else if any_visible && node.is_some_and(|n| !n.visible) {
                None
            } else {
                figcraft_render::export_bounds(doc, *i)
            }
        })
        .reduce(|a, b| a.union(b))
}

/// What an export of `ids` draws, bottom to top: the layers themselves, or (contents only off)
/// every top-level layer of their page, so whatever shows in the area is in. A slice paints nothing
/// itself: with contents only on, a slice inside a frame or group draws that frame or group, and a
/// slice anywhere else draws every top-level layer of its page.
pub fn drawn_layers(doc: &Document, ids: &[NodeId], setting: &ExportSetting) -> Vec<NodeId> {
    let Some(page) = ids.first().and_then(|i| doc.page_of(*i)).and_then(|p| doc.node(p)) else {
        return crate::cmd::edit::z_sorted(doc, ids);
    };
    if !setting.contents_only {
        return page.children.clone();
    }
    let mut own = Vec::with_capacity(ids.len());
    for &i in ids {
        if !doc.node(i).is_some_and(|n| matches!(n.kind, NodeKind::Slice)) {
            own.push(i);
            continue;
        }
        match slice_container(doc, i) {
            Some(c) => own.push(c),
            None => return page.children.clone(),
        }
    }
    own.sort_unstable();
    own.dedup();
    crate::cmd::edit::z_sorted(doc, &crate::cmd::edit::roots(doc, &own))
}

/// The frame or group a slice sits in: None on the page or in a section.
fn slice_container(doc: &Document, slice: NodeId) -> Option<NodeId> {
    let parent = doc.node(slice)?.parent?;
    doc.node(parent).is_some_and(|p| p.kind.is_frame_like() || matches!(p.kind, NodeKind::Group)).then_some(parent)
}

/// [`export_bytes`] with an export setting's format and options (its size is `scale`: see
/// [`export_scale`]): contents only, include bounding box, the SVG, PDF and Vector Drawable options.
pub fn export_bytes_with(doc: &Document, ids: &[NodeId], setting: &ExportSetting, scale: f64) -> Result<Vec<u8>> {
    export_file(doc, ids, setting, scale).map(|(bytes, _)| bytes)
}

/// What Copy as PNG puts on the clipboard: the layers together, at 2x (in the document's colour
/// profile; the clipboard wants sRGB).
pub fn copy_png(doc: &Document, ids: &[NodeId]) -> Option<figcraft_render::Pixmap> {
    let area = export_area(doc, ids, &ExportSetting::default())?;
    figcraft_render::render_area(doc, &crate::cmd::edit::z_sorted(doc, ids), area, 2.0)
}

pub(super) fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.export";
    let whole_file = match string(p, "format") {
        Some(f) => f.to_ascii_lowercase(),
        None => string(p, "path").map(|pp| ext(&pp)).unwrap_or_default(),
    };
    match whole_file.as_str() {
        "fig" => return export_fig(s, p),
        "penpot" => return export_penpot(s, p),
        _ => {}
    }
    let doc = s.doc()?;
    let mut ids = targets(s, p);
    if ids.is_empty() {
        ids = doc.get(s.page()?)?.children.clone();
    }
    let mut setting = export_setting(cmd, ExportSetting::default(), p)?;
    if string(p, "format").is_none()
        && let Some(f) = string(p, "path").and_then(|pp| ExportFormat::from_ext(&ext(&pp)))
    {
        setting.format = f;
    }
    let scale = match size_param(cmd, p)? {
        Some(_) => export_scale(doc, &ids, &setting),
        None => num_or(p, "scale", 1.0).clamp(0.01, ExportSize::MAX_SCALE),
    };
    let format = setting.format;
    let (bytes, warnings) = export_file(doc, &ids, &setting, scale)?;
    let mut out = match string(p, "path") {
        Some(path) => {
            write(&path, &bytes)?;
            json!({"path": path, "bytes": bytes.len(), "format": format.ext()})
        }
        None => json!({"format": format.ext(), "data": base64::engine::general_purpose::STANDARD.encode(&bytes)}),
    };
    // Android Vector Drawables say what they changed or left out (an empty list when nothing).
    if format == ExportFormat::VectorDrawable {
        out["warnings"] = json!(warnings);
    }
    Ok(out)
}

/// The whole document as a `.fig` file (experimental: only round trips through our own importer
/// are verified), with the warnings about what was left out.
pub(super) fn export_fig(s: &mut Session, p: &Value) -> Result<Value> {
    let doc = s.doc()?;
    let (bytes, warnings) = fig_bytes(doc)?;
    match string(p, "path") {
        Some(path) => {
            write(&path, &bytes)?;
            Ok(json!({"path": path, "bytes": bytes.len(), "format": "fig", "warnings": warnings}))
        }
        None => Ok(json!({"format": "fig", "data": base64::engine::general_purpose::STANDARD.encode(&bytes), "warnings": warnings})),
    }
}

fn export_penpot(s: &mut Session, p: &Value) -> Result<Value> {
    let (bytes, warnings) = penpot_bytes(s.doc()?)?;
    match string(p, "path") {
        Some(path) => {
            write(&path, &bytes)?;
            Ok(json!({"path": path, "bytes": bytes.len(), "format": "penpot", "warnings": warnings}))
        }
        None => Ok(json!({"format": "penpot", "data": base64::engine::general_purpose::STANDARD.encode(&bytes), "warnings": warnings})),
    }
}

pub fn penpot_bytes(doc: &Document) -> Result<(Vec<u8>, Vec<String>)> {
    figcraft_format::penpot::export(doc).map_err(|e| EngineError::Other(e.to_string()))
}

/// `doc` as a `.fig` file (with a thumbnail) and the warnings about what was left out.
pub fn fig_bytes(doc: &Document) -> Result<(Vec<u8>, Vec<String>)> {
    figcraft_format::fig::export(doc, thumbnail(doc).as_deref()).map_err(|e| EngineError::Other(e.to_string()))
}

/// The scale that gives `ids` (their export area together) `setting`'s export size.
pub fn export_scale(doc: &Document, ids: &[NodeId], setting: &ExportSetting) -> f64 {
    // Size-based exports ask for bounds before encoding. Let encoding report the depth error
    // without entering a recursive bounds walk here; SVG/PDF do not use this raster scale.
    if setting.format.is_vector() && figcraft_render::check_export_depth(doc, ids).is_err() {
        return 1.0;
    }
    export_area(doc, ids, setting).map_or(1.0, |b| setting.size.scale_for(b.width(), b.height()))
}

/// `e` with the `size` ("2x", "512w", "512h" or `{by, value}`), `suffix` and `format` that `p` sets.
pub(in crate::cmd) fn export_setting(cmd: &str, mut e: ExportSetting, p: &Value) -> Result<ExportSetting> {
    if let Some(size) = size_param(cmd, p)? {
        e.size = size;
    }
    if let Some(s) = string(p, "suffix") {
        e.suffix = s.chars().filter(|c| !c.is_control()).take(200).collect();
    }
    if let Some(f) = string(p, "format") {
        e.format = ExportFormat::from_ext(&f).ok_or_else(|| bad(cmd, "format: png|jpg|svg|pdf|webp|xml"))?;
    }
    if let Some(c) = vd_compat_param(cmd, p)? {
        e.vd_compat = c;
    }
    e.print = print_param(cmd, p, e.print)?;
    for (key, slot) in [
        ("contentsOnly", &mut e.contents_only),
        ("includeBoundingBox", &mut e.include_bounding_box),
        ("svgOutlineText", &mut e.svg_outline_text),
        ("svgIdAttribute", &mut e.svg_id_attribute),
        ("svgSimplifyStroke", &mut e.svg_simplify_stroke),
    ] {
        match get(p, key) {
            None | Some(Value::Null) => {}
            Some(v) => *slot = bool_of(v).ok_or_else(|| bad(cmd, format!("{key}: true or false")))?,
        }
    }
    Ok(e)
}

/// Whether `p` sets any PDF print option.
fn has_print_param(p: &Value) -> bool {
    ["bleed", "cropMarks", "bleedMarks", "cmyk"].iter().any(|k| get(p, k).is_some_and(|v| !v.is_null()))
}

/// `base` with the PDF print options `p` sets: `bleed` (points, or a length like "3mm" or
/// "0.125in"), `cropMarks`, `bleedMarks`, `cmyk`.
pub(in crate::cmd) fn print_param(cmd: &str, p: &Value, mut base: PrintSettings) -> Result<PrintSettings> {
    match get(p, "bleed") {
        None | Some(Value::Null) => {}
        Some(v) => {
            let pt = match v {
                Value::String(s) => PrintSettings::parse_length(s),
                v => v.as_f64().filter(|n| n.is_finite() && *n >= 0.0),
            };
            base.bleed = pt
                .filter(|b| *b <= PrintSettings::MAX_BLEED)
                .ok_or_else(|| bad(cmd, "bleed: points, or a length like \"3mm\" or \"0.125in\" (up to 144 pt, 2 in)"))?;
        }
    }
    for (key, slot) in [("cropMarks", &mut base.crop_marks), ("bleedMarks", &mut base.bleed_marks), ("cmyk", &mut base.cmyk)] {
        match get(p, key) {
            None | Some(Value::Null) => {}
            Some(v) => *slot = bool_of(v).ok_or_else(|| bad(cmd, format!("{key}: true or false")))?,
        }
    }
    Ok(base)
}

/// An export size passed as `size`: "2x", "512w", "512h", a number (scale) or `{by, value}`.
fn size_param(cmd: &str, p: &Value) -> Result<Option<ExportSize>> {
    let size = match get(p, "size") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(s)) => ExportSize::parse(s),
        Some(Value::Number(n)) => n.as_f64().and_then(|value| ExportSize::Scale { value }.checked()),
        Some(v) => serde_json::from_value::<ExportSize>(v.clone()).ok().and_then(ExportSize::checked),
    };
    size.map(Some).ok_or_else(|| bad(cmd, "size: 2x, 512w or 512h (or {by: scale|width|height, value})"))
}

/// Most files one `file.exportLayers` writes.
const MAX_EXPORT_FILES: usize = 1000;

/// Most bytes the files of one zip export add up to: the files, the archive and its base64 copy
/// are all in memory at once (in the browser too).
const MAX_ZIP_BYTES: usize = 256 << 20;

/// Keep `bytes` as zip member `name`, unless the members would pass `limit` bytes together.
pub(super) fn add_member<'a>(members: &mut Vec<(&'a str, Vec<u8>)>, name: &'a str, bytes: Vec<u8>, limit: usize) -> Result<()> {
    let total = members.iter().map(|(_, b)| b.len()).sum::<usize>().saturating_add(bytes.len());
    if total > limit {
        let mb = limit >> 20;
        return Err(bad("file.exportLayers", format!("the files would pass {mb} MB as one zip: export fewer layers at a time, or to a folder")));
    }
    members.push((name, bytes));
    Ok(())
}

/// One file of a layer export: the layer, its setting and the file name.
pub struct ExportJob {
    pub id: NodeId,
    pub setting: ExportSetting,
    pub file: String,
}

/// The files exporting `ids` with their export settings makes, with unique names (repeats get
/// " 2", " 3"…). Layers without settings make none.
pub fn export_jobs(doc: &Document, ids: &[NodeId]) -> Vec<ExportJob> {
    jobs_for(ids.iter().filter_map(|i| Some((*i, doc.node(*i)?.exports.clone()))), doc)
}

/// The files exporting each layer with the settings beside it makes (see [`export_jobs`]).
pub fn jobs_for(items: impl IntoIterator<Item = (NodeId, Vec<ExportSetting>)>, doc: &Document) -> Vec<ExportJob> {
    let mut used = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for (id, settings) in items {
        let Some(n) = doc.node(id) else { continue };
        let base = export_path(&n.name);
        for e in settings {
            if out.len() >= MAX_EXPORT_FILES {
                return out;
            }
            let stem = format!("{base}{}", safe_file_name(&e.file_suffix()));
            let ext = e.format.ext();
            let file =
                (1..).map(|k| if k == 1 { format!("{stem}.{ext}") } else { format!("{stem} {k}.{ext}") }).find(|f| used.insert(f.to_lowercase()));
            if let Some(file) = file {
                out.push(ExportJob { id, setting: e, file });
            }
        }
    }
    out
}

/// The file path a layer name exports to, without extension: each `/` makes a folder; `.` and
/// `..` segments are dropped, so it stays inside the export folder.
fn export_path(name: &str) -> String {
    let parts: Vec<String> =
        name.split('/').map(|seg| safe_file_name(seg).trim().trim_end_matches('.').to_string()).filter(|seg| !seg.is_empty()).take(32).collect();
    if parts.is_empty() { "Untitled".into() } else { parts.join("/") }
}

/// Characters no file name may contain (path separators and ones file systems reject).
pub(crate) const FORBIDDEN_FILE_NAME_CHARS: &str = "/\\:*?\"<>|";

/// `name` without path separators and characters file systems reject (`/ \ : * ? " < > |` and
/// control characters become `_`); everything else is kept, up to the first 200 characters.
/// `figcraft-cli export` names its files by this rule too (and keeps at most 100 characters before
/// the number and extension it adds).
pub fn safe_file_name(name: &str) -> String {
    name.chars().take(200).map(|c| if c.is_control() || FORBIDDEN_FILE_NAME_CHARS.contains(c) { '_' } else { c }).collect()
}

/// Layers on `page` that have export settings (document order).
pub fn exportable_layers(doc: &Document, page: NodeId) -> Vec<NodeId> {
    doc.subtree(page).into_iter().skip(1).filter(|i| doc.node(*i).is_some_and(|n| !n.exports.is_empty())).collect()
}

pub(super) fn export_layers(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.exportLayers";
    let doc = s.doc()?;
    let assets = boolean(p, "assets").unwrap_or(false);
    let mut ids = targets(s, p);
    if ids.is_empty() {
        ids = if assets { vec![s.page()?] } else { exportable_layers(doc, s.page()?) };
    }
    let jobs = if assets { crate::devinspect::asset_jobs(doc, &ids) } else { export_jobs(doc, &ids) };
    if jobs.is_empty() {
        return Err(EngineError::Disabled(cmd.into(), "no export settings (add one with export.add)".into()));
    }
    let dir = string(p, "dir");
    // The browser saves one download at a time: several files go as one zip (#969).
    let zip = dir.is_none() && boolean(p, "zip").unwrap_or(false);
    let mut members = Vec::new();
    // Print options passed here stand in for every PDF setting's own.
    let print = has_print_param(p).then(|| print_param(cmd, p, PrintSettings::default())).transpose()?;
    // And a compatibility mode for every Android Vector Drawable.
    let compat = vd_compat_param(cmd, p)?;
    let mut files = Vec::with_capacity(jobs.len());
    for job in &jobs {
        let setting =
            ExportSetting { print: print.unwrap_or(job.setting.print), vd_compat: compat.unwrap_or(job.setting.vd_compat), ..job.setting.clone() };
        let scale = export_scale(doc, &[job.id], &setting);
        let (bytes, warnings) = export_file(doc, &[job.id], &setting, scale)?;
        let mut f = json!({"name": job.file, "id": job.id, "bytes": bytes.len()});
        if setting.format == ExportFormat::VectorDrawable {
            f["warnings"] = json!(warnings);
        }
        match &dir {
            Some(d) => {
                let path = std::path::Path::new(d).join(&job.file);
                // A layer name with slashes exports into folders.
                if let Some(parent) = path.parent() {
                    let _ = crate::file_access::create_dir_all(parent);
                }
                let path = path.to_string_lossy().to_string();
                write(&path, &bytes)?;
                f["path"] = json!(path);
            }
            None if zip => add_member(&mut members, job.file.as_str(), bytes, MAX_ZIP_BYTES)?,
            None => f["data"] = json!(base64::engine::general_purpose::STANDARD.encode(&bytes)),
        }
        files.push(f);
    }
    let with_settings: std::collections::BTreeSet<NodeId> = jobs.iter().map(|j| j.id).collect();
    let mut r = json!({"files": files, "skipped": ids.len().saturating_sub(with_settings.len())});
    if zip {
        let archive = figcraft_format::zip_files(members.iter().map(|(n, b)| (*n, b.as_slice()))).map_err(|e| EngineError::Other(e.to_string()))?;
        r["data"] = json!(base64::engine::general_purpose::STANDARD.encode(&archive));
    }
    Ok(r)
}

/// A PDF with one page per group of layers (an sRGB document), titled after the file; `area`
/// sizes a single page to that page area instead of the layers' bounds.
pub(super) fn pdf_bytes(doc: &Document, groups: &[Vec<NodeId>], print: &PrintSettings, area: Option<figcraft_geom::Rect>) -> Result<Vec<u8>> {
    let opts = figcraft_pdf::PdfOptions { title: Some(doc.name.clone()), print: *print, area, ..Default::default() };
    figcraft_pdf::export_groups(doc, groups, &opts).map_err(|e| EngineError::Other(e.to_string()))
}

/// Every top-level frame of `page` (reading order) as one PDF page each, and the page count.
pub fn frames_pdf(doc: &Document, page: NodeId, print: &PrintSettings) -> Result<(Vec<u8>, usize)> {
    let mut frames = doc.top_frames(page);
    frames.retain(|f| doc.node(*f).is_some_and(|n| n.visible));
    if frames.is_empty() {
        return Err(EngineError::Disabled("file.exportFramesPdf".into(), "the page has no visible frames".into()));
    }
    let groups: Vec<Vec<NodeId>> = frames.iter().map(|f| vec![*f]).collect();
    Ok((pdf_bytes(&doc.in_srgb(), &groups, print, None)?, frames.len()))
}

/// Every top-level frame of a page (reading order), one PDF page each.
pub(super) fn export_frames_pdf(s: &mut Session, p: &Value) -> Result<Value> {
    let doc = s.doc()?;
    let page = match id(p, "page") {
        Some(pg) if doc.node(pg).is_some_and(Node::is_page) => pg,
        Some(_) => return Err(bad("file.exportFramesPdf", "`page` is not a page")),
        None => s.page()?,
    };
    let print = print_param("file.exportFramesPdf", p, PrintSettings::default())?;
    let (bytes, pages) = frames_pdf(doc, page, &print)?;
    match string(p, "path") {
        Some(path) => {
            write(&path, &bytes)?;
            Ok(json!({"path": path, "bytes": bytes.len(), "pages": pages}))
        }
        None => Ok(json!({"format": "pdf", "pages": pages, "data": base64::engine::general_purpose::STANDARD.encode(&bytes)})),
    }
}
