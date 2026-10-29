use super::{DocState, Document, EngineError, LineHeight, NodeId, NodeKind, Result, StyleKind, TextStyle, Value, bad, get, json};

/// The recovery copy at `path` as a document with unsaved changes and no path.
pub(crate) fn recovered(path: &str) -> Result<DocState> {
    let loaded = read(path).and_then(|b| figcraft_format::load_bytes_with_versions(&b).map_err(|e| EngineError::Other(e.to_string())))?;
    Ok(DocState::new(loaded.doc, None).with_versions(loaded.versions).with_branches(loaded.branches).unsaved())
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn read(path: &str) -> Result<Vec<u8>> {
    crate::file_access::read(path)
}

#[cfg(target_arch = "wasm32")]
pub(super) fn read(path: &str) -> Result<Vec<u8>> {
    Err(EngineError::Other(format!("{path}: no file system in the browser")))
}

/// The bytes of file `path`, refused when it's larger than `max`.
pub(crate) fn read_capped(path: &str, max: u64) -> Result<Vec<u8>> {
    crate::file_access::read_capped(path, max)
}

/// Write `bytes` to `path` through a uniquely named, exclusively created staging file and one
/// atomic rename (see [`figcraft_format::write_file_atomic`]): a failed or interrupted write never
/// truncates `path`, and no file or symlink at a staging name is ever written through.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn write(path: &str, bytes: &[u8]) -> Result<()> {
    crate::file_access::write(path, bytes)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn write(path: &str, _bytes: &[u8]) -> Result<()> {
    Err(EngineError::Other(format!("{path}: no file system in the browser")))
}

pub(super) fn ext(path: &str) -> String {
    std::path::Path::new(path).extension().and_then(std::ffi::OsStr::to_str).unwrap_or("").to_ascii_lowercase()
}

pub(super) fn file_stem(path: &str) -> String {
    std::path::Path::new(path).file_stem().and_then(std::ffi::OsStr::to_str).unwrap_or("Untitled").to_string()
}

/// The counts the import report shows for a freshly loaded document.
pub(super) fn import_summary(doc: &Document) -> Value {
    json!({"name": doc.name, "pages": doc.pages.len(), "layers": doc.nodes().filter(|n| !n.is_page()).count(), "images": doc.images.len()})
}

/// Load a document from any supported file (REST API file JSON images come from an `images`
/// folder next to the file).
pub fn load(path: &str) -> Result<(Document, Vec<String>)> {
    load_path(path, None).map(|(l, _)| (l.doc, l.issues))
}

pub fn load_bytes(bytes: &[u8], name_hint: &str) -> Result<(Document, Vec<String>)> {
    load_with(bytes, name_hint, None, &figcraft_format::rest::no_images).map(|(l, _)| (l.doc, l.issues))
}

/// [`load`] with the version history, also telling whether the file was REST API file JSON. A REST
/// import reads its variables from `variables`, else from a variables JSON next to the file.
pub fn load_path(path: &str, variables: Option<&Value>) -> Result<(figcraft_format::Loaded, bool)> {
    let bytes = read(path)?;
    let sibling = if variables.is_none() { sibling_variables(path) } else { None };
    load_with(&bytes, path, variables.or(sibling.as_ref()), &rest_images(path))
}

/// Load from bytes. Returns the document with its notes (repairs of a native file, or import
/// warnings) and version history, and whether it was imported from REST API file JSON or a `.fig`
/// file (such a file is never saved over). `variables`: the local variables JSON for a REST API file JSON.
pub(super) fn load_with(
    bytes: &[u8],
    name_hint: &str,
    variables: Option<&Value>,
    images: figcraft_format::rest::ImageResolver,
) -> Result<(figcraft_format::Loaded, bool)> {
    match ext(name_hint).as_str() {
        "svg" => {
            let mut doc = Document::new(file_stem(name_hint));
            let page = doc.first_page().ok_or(EngineError::NoDocument)?;
            let notes = figcraft_svg::import_svg_with_notes(&mut doc, bytes, page, 0.0, 0.0, &file_stem(name_hint))
                .map_err(|e| EngineError::Other(e.to_string()))?
                .notes;
            Ok((figcraft_format::Loaded { issues: notes, ..doc.into() }, false))
        }
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff" => {
            let mut doc = Document::new(file_stem(name_hint));
            let page = doc.first_page().ok_or(EngineError::NoDocument)?;
            let (hash, w, h) = figcraft_format::add_image(&mut doc, bytes.to_vec()).map_err(|e| EngineError::Other(e.to_string()))?;
            let id = doc.create(page, NodeKind::Rectangle, &file_stem(name_hint), 0.0, 0.0, f64::from(w), f64::from(h))?;
            doc.get_mut(id)?.fills = vec![figcraft_color::Paint::image(hash, figcraft_color::ImageScaleMode::Fill)];
            Ok((doc.into(), false))
        }
        _ => {
            if let Some(r) = load_fig(bytes, &file_stem(name_hint)) {
                return Ok((r?, true));
            }
            if let Some(r) = load_sketch(bytes, &file_stem(name_hint)) {
                return Ok((r?, true));
            }
            if let Some(r) = figcraft_format::rest::try_import_with(bytes, variables, images) {
                let (mut doc, warnings) = r.map_err(|e| EngineError::Other(e.to_string()))?;
                link_instances(&mut doc);
                return Ok((figcraft_format::Loaded { issues: warnings, ..doc.into() }, true));
            }
            let mut loaded = figcraft_format::load_bytes_with_versions(bytes).map_err(|e| EngineError::Other(e.to_string()))?;
            // Older files saved an untouched default title even after the user chose a filename.
            if loaded.doc.name.is_empty() || loaded.doc.name == "Untitled" {
                loaded.doc.name = file_stem(name_hint);
            }
            Ok((loaded, false))
        }
    }
}

/// Gives `None` when `bytes` are not a `.sketch` file. Then other readers can try.
fn load_sketch(bytes: &[u8], name: &str) -> Option<Result<figcraft_format::Loaded>> {
    if !figcraft_format::sketch::is_sketch(bytes) {
        return None;
    }
    // The import and what follows read the installed fonts: the system's must be known first (the
    // desktop app scans them in the background; this waits for that scan).
    figcraft_text::fontdb::scan_system_fonts();
    let fonts: figcraft_format::sketch::FontLookup = Box::new(figcraft_text::fontdb::postscript_face);
    let r = figcraft_format::sketch::try_import_with_fonts(bytes, name, Some(fonts))?;
    Some(r.map_err(|e| EngineError::Other(e.to_string())).map(|(mut doc, warnings)| {
        match_font_families(&mut doc);
        platform_tracking(&mut doc);
        sketch_line_heights(&mut doc);
        link_instances(&mut doc);
        reflow_overridden_text(&mut doc);
        figcraft_format::Loaded { issues: warnings, ..doc.into() }
    }))
}

/// Instances of a stack sized apart from their main are laid out at their size, and texts in
/// instances whose string an override changed are measured again with the auto layout around them
/// re-flowing (Sketch saves layouts measured with the master's string). The rest keeps the layout
/// Sketch saved.
fn reflow_overridden_text(doc: &mut Document) {
    let changed: Vec<NodeId> = doc
        .nodes()
        .filter(|n| match (&n.kind, n.source.and_then(|s| doc.node(s)).map(|s| &s.kind)) {
            (NodeKind::Text(t), Some(NodeKind::Text(main))) => t.characters != main.characters,
            _ => false,
        })
        .map(|n| n.id)
        .collect();
    let m = crate::measure::TextMeasure;
    // Instances of a stack at another size than their main: the stack at their size, innermost first.
    let mut resized: Vec<(usize, NodeId)> = doc
        .nodes()
        .filter(|n| n.layout.is_some())
        .filter(|n| match &n.kind {
            NodeKind::Instance { component, .. } => {
                doc.node(*component).is_some_and(|c| (c.width - n.width).abs() > 1e-6 || (c.height - n.height).abs() > 1e-6)
            }
            _ => false,
        })
        .map(|n| (doc.ancestors(n.id).len(), n.id))
        .collect();
    resized.sort_by_key(|r| std::cmp::Reverse(r.0));
    for (_, id) in resized {
        figcraft_layout::layout_frame(doc, id, &m);
    }
    for id in changed {
        if let Some((w, h)) = doc.node(id).and_then(figcraft_text::auto_size) {
            // Sketch grows auto-width text from its aligned edge: a centred one stays centred, a
            // right-aligned one keeps its right edge.
            let shift = doc.node(id).map_or(0.0, |n| match &n.kind {
                NodeKind::Text(t) if t.auto_resize == figcraft_doc::TextAutoResize::WidthAndHeight => match t.align_h {
                    figcraft_doc::TextAlignH::Center => (n.width - w) / 2.0,
                    figcraft_doc::TextAlignH::Right => n.width - w,
                    _ => 0.0,
                },
                _ => 0.0,
            });
            figcraft_layout::set_size(doc, id, w, h);
            if shift.is_finite()
                && shift != 0.0
                && let Some(n) = doc.node_mut(id)
            {
                n.transform *= figcraft_doc::geom::Affine::translate((shift, 0.0));
            }
        }
        figcraft_layout::layout_up(doc, id, &m);
    }
}

/// Font families a `.sketch` import guessed from PostScript names ("Ping Fang SC" from
/// "PingFangSC-Semibold") take the installed family's name when they differ only in spaces and
/// case.
fn match_font_families(doc: &mut Document) {
    let installed = figcraft_text::fontdb::family_names();
    let mut cache: std::collections::BTreeMap<String, Option<String>> = std::collections::BTreeMap::new();
    let mut fix = |style: &mut figcraft_doc::TextStyle| {
        let found = cache.entry(style.font_family.clone()).or_insert_with(|| installed_family(&style.font_family, &installed));
        if let Some(f) = found {
            style.font_family = f.clone();
        }
    };
    let ids: Vec<NodeId> = doc.nodes().filter(|n| matches!(n.kind, NodeKind::Text(_))).map(|n| n.id).collect();
    for id in ids {
        if let Some(NodeKind::Text(td)) = doc.node_mut(id).map(|n| &mut n.kind) {
            fix(&mut td.style);
            for r in &mut td.runs {
                fix(&mut r.style);
            }
        }
    }
    // Shared text styles too, so they stay like the layers linked to them.
    for st in &mut doc.styles {
        if let StyleKind::Text { style } = &mut st.kind {
            fix(style);
        }
    }
}

/// Sketch draws text with the tracking a font's `trak` table gives at its size (Apple's text engine
/// adds it; FigCraft's, like Figma's, doesn't): the imported text keeps it as letter spacing, so
/// lines measure and break as they did there. A named style's optical size follows the text size
/// there too (FigCraft keeps the instance's), so the imported text carries it as an `opsz` value.
fn platform_tracking(doc: &mut Document) {
    let mut cache: std::collections::BTreeMap<(String, String), Option<std::sync::Arc<figcraft_text::fontdb::Face>>> =
        std::collections::BTreeMap::new();
    let mut face = |style: &figcraft_doc::TextStyle| {
        cache
            .entry((style.font_family.clone(), style.font_style.clone()))
            .or_insert_with(|| {
                figcraft_text::fontdb::has_family(&style.font_family)
                    .then(|| figcraft_text::fontdb::find_face(&style.font_family, &style.font_style))
                    .flatten()
            })
            .clone()
    };
    let mut track = |style: &mut figcraft_doc::TextStyle| {
        let Some(f) = face(style) else { return };
        let t = f.tracking(style.font_size);
        if t != 0.0 && t.is_finite() {
            let px = style.letter_spacing.resolve(style.font_size);
            style.letter_spacing = figcraft_doc::LetterSpacing::Pixels(px + t);
        }
        if f.axis("opsz").is_some() && f.instance(&style.font_style).is_some() && style.font_size.is_finite() {
            style.variations.entry("opsz".into()).or_insert(style.font_size);
        }
    };
    let ids: Vec<NodeId> = doc.nodes().filter(|n| matches!(n.kind, NodeKind::Text(_))).map(|n| n.id).collect();
    for id in ids {
        if let Some(NodeKind::Text(td)) = doc.node_mut(id).map(|n| &mut n.kind) {
            track(&mut td.style);
            for r in &mut td.runs {
                track(&mut r.style);
            }
        }
    }
    for st in &mut doc.styles {
        if let StyleKind::Text { style } = &mut st.kind {
            track(style);
        }
    }
}

/// Families Sketch (Apple's text system) gives a fifth more than their ascent and descent as their
/// natural line: classic Mac fonts, found on `sketchtool` renders. Other fonts show none of this.
const TALL_LINE_FAMILIES: [&str; 11] =
    ["Academy Engraved LET", "Courier", "Helvetica", "Hoefler Text", "Palatino", "Sathu", "Silom", "Skia", "Times", "Webdings", "Zapf Dingbats"];

/// Sketch's natural line for a font of `ascent` and `descent` in `upem` units at `size` px: its
/// height in px and how far down the text moves so its baseline sits where Sketch puts it. Sketch
/// rounds the ascent and descent to whole pixels each and leaves the font's line gap out (a fifth
/// more for [`TALL_LINE_FAMILIES`]), with the baseline above the rounded descent; FigCraft centres
/// the font's own ascent and descent in the line. `(0, 0)` without metrics.
pub(crate) fn sketch_line(ascent: f64, descent: f64, upem: f64, family: &str, size: f64) -> (f64, f64) {
    let k = size / upem;
    let (a, d) = (ascent * k, descent * k);
    if !(a.is_finite() && d.is_finite()) || a + d <= 0.0 {
        return (0.0, 0.0);
    }
    let natural = a.round() + d.round();
    let line = if TALL_LINE_FAMILIES.iter().any(|f| f.eq_ignore_ascii_case(family)) { (natural * 1.2).round() } else { natural };
    let sketch_baseline = line - d.round();
    let figcraft_baseline = (line - (a + d)) / 2.0 + a;
    (line, sketch_baseline - figcraft_baseline)
}

/// Text without a fixed line height takes the line Sketch drew it with (FigCraft's natural line,
/// like Figma's, counts the font's line gap and centres the font in it; Sketch's doesn't), as a
/// share of its size so it still follows the size, and moves to Sketch's baseline. Shared text
/// styles take the line too.
fn sketch_line_heights(doc: &mut Document) {
    // Per family and style: the face's ascent, descent and units per em.
    type Metrics = Option<(f64, f64, f64)>;
    let mut cache: std::collections::BTreeMap<(String, String), Metrics> = std::collections::BTreeMap::new();
    let mut set = |style: &mut TextStyle| -> f64 {
        if style.line_height != LineHeight::Auto || !style.font_size.is_finite() || style.font_size <= 0.0 {
            return 0.0;
        }
        let m = *cache.entry((style.font_family.clone(), style.font_style.clone())).or_insert_with(|| {
            figcraft_text::fontdb::has_family(&style.font_family)
                .then(|| figcraft_text::fontdb::find_face(&style.font_family, &style.font_style))
                .flatten()
                .map(|f| (f.ascent, f.descent, f.upem))
        });
        let Some((a, d, upem)) = m else { return 0.0 };
        let (line, down) = sketch_line(a, d, upem, &style.font_family, style.font_size);
        if line > 0.0 {
            style.line_height = LineHeight::Percent(line / style.font_size * 100.0);
        }
        down
    };
    let ids: Vec<NodeId> = doc.nodes().filter(|n| matches!(n.kind, NodeKind::Text(_))).map(|n| n.id).collect();
    for id in ids {
        // An auto layout places its texts itself (its next pass would take a move back).
        let laid_out =
            doc.node(id).is_some_and(|n| !n.child_layout.absolute && n.parent.and_then(|p| doc.node(p)).is_some_and(|p| p.layout.is_some()));
        let Some(n) = doc.node_mut(id) else { continue };
        let NodeKind::Text(td) = &mut n.kind else { continue };
        let down = set(&mut td.style);
        for r in &mut td.runs {
            set(&mut r.style);
        }
        // Down in the layer's own frame (a rotated text moves along its own axis).
        if !laid_out && down.is_finite() && down != 0.0 {
            n.transform *= figcraft_doc::geom::Affine::translate((0.0, down));
        }
    }
    for st in &mut doc.styles {
        if let StyleKind::Text { style } = &mut st.kind {
            set(style);
        }
    }
}

/// The installed family `family` means when it isn't installed under that exact name: the one
/// whose name is the same without spaces, ignoring case.
pub(crate) fn installed_family(family: &str, installed: &[String]) -> Option<String> {
    let key = |s: &str| s.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect::<String>();
    if installed.iter().any(|f| f.eq_ignore_ascii_case(family)) {
        return None;
    }
    let want = key(family);
    installed.iter().find(|f| key(f) == want).cloned()
}

/// Gives `None` when `bytes` are not a `.fig` file. Then other readers can try.
pub(crate) fn load_fig(bytes: &[u8], name: &str) -> Option<Result<figcraft_format::Loaded>> {
    let r = figcraft_format::fig::try_import(bytes, name)?;
    Some(r.map_err(|e| EngineError::Other(e.to_string())).map(|(mut doc, warnings)| {
        link_instances(&mut doc);
        figcraft_format::Loaded { issues: warnings, ..doc.into() }
    }))
}

/// Link imported instances to their mains (layer sources, overrides found by comparing values),
/// innermost first so outer instances settle last.
fn link_instances(doc: &mut Document) {
    crate::cmd::styles::unfold_imported_bound_alpha(doc);
    crate::style_vars::after_load(doc);
    crate::cmd::styles::apply_paint_bindings_on_load(doc);
    let mut insts: Vec<(usize, NodeId)> =
        doc.nodes().filter(|n| matches!(n.kind, NodeKind::Instance { .. })).map(|n| (doc.ancestors(n.id).len(), n.id)).collect();
    insts.sort_by_key(|a| std::cmp::Reverse(a.0));
    for (_, i) in insts {
        crate::components::rebase_instance(doc, i);
    }
}

/// Largest image file read for a REST API file JSON import.
#[cfg(not(target_arch = "wasm32"))]
const MAX_IMAGE_FILE: u64 = figcraft_doc::ImageData::MAX_BYTES;

/// Image bytes for a REST API file JSON's `imageRef`: `images/<imageRef>[.png|.jpg|…]` next to
/// the file.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn rest_images(path: &str) -> impl Fn(&str) -> Option<Vec<u8>> {
    let dir = std::path::Path::new(path).parent().map(|d| d.join("images"));
    move |r: &str| {
        // Image refs are content hashes; anything else never becomes a path.
        if r.is_empty() || r.len() > 128 || !r.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
            return None;
        }
        let dir = dir.as_ref()?;
        ["", ".png", ".jpg", ".jpeg", ".gif", ".webp"]
            .iter()
            .find_map(|e| crate::file_access::read_capped(dir.join(format!("{r}{e}")), MAX_IMAGE_FILE).ok())
    }
}

#[cfg(target_arch = "wasm32")]
pub(super) fn rest_images(_path: &str) -> impl Fn(&str) -> Option<Vec<u8>> {
    figcraft_format::rest::no_images
}

/// Largest variables JSON read from disk.
#[cfg(not(target_arch = "wasm32"))]
const MAX_VARIABLES_FILE: u64 = 256 * 1024 * 1024;

/// A local variables JSON next to a REST API file JSON: `<name>.variables.json`, else
/// `variables.json`.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn sibling_variables(path: &str) -> Option<Value> {
    if ext(path) != "json" {
        return None;
    }
    let file = std::path::Path::new(path);
    let dir = file.parent()?;
    [format!("{}.variables.json", file_stem(path)), "variables.json".to_string()].iter().map(|n| dir.join(n)).filter(|c| c != file).find_map(|c| {
        let v: Value = serde_json::from_slice(&crate::file_access::read_capped(c, MAX_VARIABLES_FILE).ok()?).ok()?;
        figcraft_format::rest::is_variables_json(&v).then_some(v)
    })
}

#[cfg(target_arch = "wasm32")]
pub(super) fn sibling_variables(_path: &str) -> Option<Value> {
    None
}

/// The local variables JSON a command passes as `variables`: an object, JSON text or a file path.
pub(crate) fn variables_param(cmd: &str, p: &Value) -> Result<Option<Value>> {
    let parse = |text: &[u8]| serde_json::from_slice::<Value>(text).map_err(|e| bad(cmd, format!("variables: {e}")));
    let v = match get(p, "variables") {
        None | Some(Value::Null) => return Ok(None),
        Some(v @ Value::Object(_)) => v.clone(),
        Some(Value::String(s)) if s.trim_start().starts_with('{') => parse(s.as_bytes())?,
        Some(Value::String(path)) => parse(&read(path)?)?,
        Some(_) => return Err(bad(cmd, "variables: the local variables JSON (an object, JSON text or a path)")),
    };
    if !figcraft_format::rest::is_variables_json(&v) {
        return Err(bad(cmd, "variables: expected the local variables JSON (`meta.variableCollections` and `meta.variables`)"));
    }
    Ok(Some(v))
}
