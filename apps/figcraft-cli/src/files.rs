//! The file subcommands: info, export, convert, render and run.

use std::ffi::OsStr;
use std::path::Path;
use std::process::ExitCode;

use figcraft_doc::{Document, ExportFormat, ExportSetting, NodeId, NodeKind, PrintSettings, VdCompat};
use figcraft_engine::{BatchScope, BatchStep, Session};
use serde_json::{Value, json};

use crate::{CliError, CliResult};

pub fn load(path: &str) -> CliResult<Document> {
    let (doc, notes) = load_noted(path)?;
    if ext(path) == "penpot" {
        for note in notes {
            eprintln!("note: {note}");
        }
    }
    Ok(doc)
}

/// The document with the loader's notes (repairs of a damaged file, or import warnings).
fn load_noted(path: &str) -> CliResult<(Document, Vec<String>)> {
    figcraft_engine::cmd::file::load(path).map_err(|e| CliError::Failed(format!("can't open {path}: {e}")))
}

fn ext(path: &str) -> String {
    Path::new(path).extension().and_then(OsStr::to_str).unwrap_or("").to_ascii_lowercase()
}

fn image_format(e: &str) -> Option<ExportFormat> {
    match e {
        "png" | "jpg" | "jpeg" | "svg" | "webp" | "pdf" | "xml" => ExportFormat::from_ext(e),
        _ => None,
    }
}

/// Writes through [`figcraft_engine::file_access`] when automation roots are in force, so a
/// confined run can only write inside a write root (#833).
///
/// Unconfined it writes with `std::fs::write`, as it always did. `file_access::write` stages the
/// bytes and renames them into place, which **replaces** a symbolic link at `path` instead of
/// writing through it to its target -- a change of behaviour for anyone whose output path is a
/// link, and nothing to do with confinement. Inside a root the staged write is the right one:
/// `Roots::check` has already refused a link that leads out of the root.
fn write(path: &str, bytes: &[u8]) -> CliResult<()> {
    if let Some(dir) = Path::new(path).parent().filter(|d| !d.as_os_str().is_empty()) {
        figcraft_engine::file_access::create_dir_all(dir).map_err(|e| CliError::Failed(e.to_string()))?;
    }
    if figcraft_engine::file_access::current().is_some() {
        return figcraft_engine::file_access::write(path, bytes).map_err(|e| CliError::Failed(e.to_string()));
    }
    std::fs::write(path, bytes).map_err(|e| CliError::Failed(format!("{path}: {e}")))
}

fn page_named(doc: &Document, name: Option<&str>) -> CliResult<NodeId> {
    match name {
        None => doc.first_open_page().or_else(|| doc.first_page()).ok_or_else(|| CliError::Failed("the document has no pages".into())),
        Some(n) => doc
            .pages
            .iter()
            .copied()
            .find(|p| doc.node(*p).is_some_and(|pn| pn.name == n))
            .or_else(|| doc.pages.iter().copied().find(|p| doc.node(*p).is_some_and(|pn| pn.name.eq_ignore_ascii_case(n))))
            .ok_or_else(|| {
                let names: Vec<String> = doc.pages.iter().filter_map(|p| doc.node(*p).map(|n| n.name.clone())).collect();
                CliError::Failed(format!("no page named `{n}` (pages: {})", names.join(", ")))
            }),
    }
}

fn find_node(doc: &Document, page: NodeId, q: &str) -> CliResult<NodeId> {
    if let Ok(id) = q.parse::<NodeId>()
        && doc.contains(id)
    {
        return Ok(id);
    }
    doc.find_by_name(q, Some(page))
        .into_iter()
        .find(|i| doc.node(*i).is_some_and(|n| n.name == q))
        .or_else(|| doc.find_by_name(q, None).into_iter().find(|i| doc.node(*i).is_some_and(|n| n.name == q)))
        .or_else(|| doc.find_by_name(q, Some(page)).into_iter().next())
        .ok_or_else(|| CliError::Failed(format!("no layer with id or name `{q}`")))
}

fn children(doc: &Document, page: NodeId) -> Vec<NodeId> {
    doc.node(page).map(|p| p.children.clone()).unwrap_or_default()
}

/// Render a page's content (with the page background) at `scale`, shrinking to fit 16384 px.
pub fn render_page(doc: &Document, page: NodeId, scale: f64) -> CliResult<figcraft_render::Pixmap> {
    let b = doc.page_content_bounds(page).ok_or_else(|| CliError::Failed("the page is empty: nothing to render".into()))?;
    let side = b.width().max(b.height()).max(1e-6);
    let scale = scale.min(16_384.0 / side);
    let view = figcraft_render::View {
        origin: (b.x0, b.y0),
        scale,
        width: (b.width() * scale).ceil().max(1.0) as u32,
        height: (b.height() * scale).ceil().max(1.0) as u32,
    };
    figcraft_render::render_view(doc, page, &view, &Default::default()).ok_or_else(|| CliError::Failed("render failed".into()))
}

/// Encode a render as the format extension `e` names, tagged with `doc`'s colour profile.
fn encode(doc: &Document, pix: &figcraft_render::Pixmap, e: &str) -> CliResult<Vec<u8>> {
    let format = ExportFormat::from_ext(e).unwrap_or_default();
    figcraft_render::encode_export(pix, format, doc.color_profile).map_err(CliError::Failed)
}

/// Write `doc` to `out`, choosing the format by extension.
pub fn write_doc(doc: &Document, page: NodeId, out: &str) -> CliResult<()> {
    let e = ext(out);
    let bytes = match e.as_str() {
        "figcraft" => {
            let thumb = figcraft_engine::cmd::file::thumbnail(doc);
            figcraft_format::save_bytes(doc, thumb.as_deref()).map_err(|e| CliError::Failed(e.to_string()))?
        }
        "json" => {
            let v = figcraft_format::to_json(doc).map_err(|e| CliError::Failed(e.to_string()))?;
            serde_json::to_vec_pretty(&v).map_err(|e| CliError::Failed(e.to_string()))?
        }
        "svg" => figcraft_svg::try_export_svg(&doc.in_srgb(), &children(doc, page), &Default::default())
            .map_err(|e| CliError::Failed(e.to_string()))?
            .into_bytes(),
        "png" | "jpg" | "jpeg" | "webp" => encode(doc, &render_page(doc, page, 1.0)?, &e)?,
        "pdf" => page_pdf(doc, page, &PrintSettings::default())?,
        "fig" => fig_bytes(doc)?,
        "penpot" => penpot_bytes(doc)?,
        other => return Err(CliError::Usage(format!("unknown output format `.{other}` (figcraft, json, svg, png, jpg, webp, pdf, fig, penpot)"))),
    };
    write(out, &bytes)
}

/// `doc` as a `.fig` file (experimental), printing what was left out.
fn fig_bytes(doc: &Document) -> CliResult<Vec<u8>> {
    let (bytes, warnings) = figcraft_engine::cmd::file::fig_bytes(doc).map_err(|e| CliError::Failed(e.to_string()))?;
    for w in warnings {
        eprintln!("note: {w}");
    }
    Ok(bytes)
}

fn penpot_bytes(doc: &Document) -> CliResult<Vec<u8>> {
    let (bytes, warnings) = figcraft_engine::cmd::file::penpot_bytes(doc).map_err(|e| CliError::Failed(e.to_string()))?;
    for w in warnings {
        eprintln!("note: {w}");
    }
    Ok(bytes)
}

/// A page as PDF: its top-level frames one page each, or (without frames) everything on one page.
fn page_pdf(doc: &Document, page: NodeId, print: &PrintSettings) -> CliResult<Vec<u8>> {
    let r = if doc.top_frames(page).iter().any(|f| doc.node(*f).is_some_and(|n| n.visible)) {
        figcraft_engine::cmd::file::frames_pdf(doc, page, print).map(|(bytes, _)| bytes)
    } else {
        figcraft_engine::cmd::file::export_bytes_with(
            doc,
            &children(doc, page),
            &ExportSetting { format: ExportFormat::Pdf, print: *print, ..Default::default() },
            1.0,
        )
    };
    r.map_err(|e| CliError::Failed(e.to_string()))
}

pub fn info(path: &str, as_json: bool) -> CliResult<ExitCode> {
    let (doc, notes) = load_noted(path)?;
    let pages: Vec<Value> = doc
        .pages
        .iter()
        .filter_map(|p| {
            let pn = doc.node(*p)?;
            let nodes = doc.subtree(*p).len().saturating_sub(1);
            let frames: Vec<Value> = pn
                .children
                .iter()
                .filter_map(|c| doc.node(*c))
                .filter(|n| n.kind.is_frame_like() || matches!(n.kind, NodeKind::Section { .. }))
                .map(|n| json!({"id": n.id, "name": n.name, "type": n.kind.type_name(), "width": n.width, "height": n.height}))
                .collect();
            Some(json!({"id": p, "name": pn.name, "nodes": nodes, "topLevel": pn.children.len(), "frames": frames}))
        })
        .collect();
    let missing = figcraft_engine::fonts::missing(&doc);
    let v = json!({
        "file": path,
        "name": doc.name,
        "nodes": doc.node_count(),
        "pages": pages,
        "styles": doc.styles.len(),
        "variables": doc.variables.variables.len(),
        "images": doc.images.len(),
        "notes": notes,
        "missingFonts": missing.iter().map(|m| json!({"family": m.family, "layers": m.layers})).collect::<Vec<_>>(),
    });
    if as_json {
        println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
        return Ok(ExitCode::SUCCESS);
    }
    for n in &notes {
        eprintln!("note: {n}");
    }
    println!(
        "{} — {} ({} nodes, {} styles, {} variables, {} images)",
        path,
        doc.name,
        doc.node_count(),
        doc.styles.len(),
        doc.variables.variables.len(),
        doc.images.len()
    );
    for p in &pages {
        println!("page {} \"{}\": {} nodes, {} top-level", p["id"], p["name"].as_str().unwrap_or(""), p["nodes"], p["topLevel"]);
        for f in p["frames"].as_array().into_iter().flatten() {
            println!("  {} {} \"{}\" {}×{}", f["id"], f["type"].as_str().unwrap_or(""), f["name"].as_str().unwrap_or(""), f["width"], f["height"]);
        }
    }
    for m in &missing {
        let layers = if m.layers == 1 { "text layer" } else { "text layers" };
        println!("missing font \"{}\": {} {layers} (drawn in a fallback font)", m.family, m.layers);
    }
    Ok(ExitCode::SUCCESS)
}

/// The file name a layer exports to: the app's rule ([`figcraft_engine::cmd::file::safe_file_name`],
/// so `/` becomes `_` too), without leading or trailing spaces and dots, at most 100 characters
/// and 200 bytes (file systems allow 255, and a number and the extension still fit).
fn sanitize(name: &str) -> String {
    let s = figcraft_engine::cmd::file::safe_file_name(name);
    let s = s.trim_matches(|c: char| c == '.' || c.is_whitespace());
    if s.is_empty() { "layer".into() } else { s.char_indices().take(100).take_while(|(i, c)| i + c.len_utf8() <= 200).map(|(_, c)| c).collect() }
}

/// What `export` exports and how.
pub struct Target {
    pub format: Option<String>,
    pub scale: f64,
    pub page: Option<String>,
    pub node: Option<String>,
    /// PDF only.
    pub print: PrintSettings,
    /// Android Vector Drawable (XML) only.
    pub vd_compat: VdCompat,
}

pub fn export(file: &str, out: &str, t: &Target) -> CliResult<ExitCode> {
    let doc = load(file)?;
    // `.fig`: the whole file.
    if t.format.as_deref().map_or(ext(out) == "fig", |f| f.eq_ignore_ascii_case("fig")) {
        write(out, &fig_bytes(&doc)?)?;
        println!("{out}");
        return Ok(ExitCode::SUCCESS);
    }
    if t.format.as_deref().map_or(ext(out) == "penpot", |f| f.eq_ignore_ascii_case("penpot")) {
        write(out, &penpot_bytes(&doc)?)?;
        println!("{out}");
        return Ok(ExitCode::SUCCESS);
    }
    let page = page_named(&doc, t.page.as_deref())?;
    let (scale, node, print) = (t.scale, t.node.as_deref(), &t.print);
    let fmt = match t.format.as_deref() {
        Some(f) => image_format(&f.to_ascii_lowercase())
            .ok_or_else(|| CliError::Usage(format!("unknown --format `{f}` (png|jpg|svg|webp|pdf|xml|fig|penpot)")))?,
        None => image_format(&ext(out)).unwrap_or(ExportFormat::Png),
    };
    let target = node.map(|q| find_node(&doc, page, q)).transpose()?;
    let single = image_format(&ext(out)).is_some();
    let bytes_of = |ids: &[NodeId]| {
        let setting = ExportSetting { format: fmt, print: *print, vd_compat: t.vd_compat, ..Default::default() };
        let (bytes, warnings) = figcraft_engine::cmd::file::export_file(&doc, ids, &setting, scale).map_err(|e| CliError::Failed(e.to_string()))?;
        // What an Android Vector Drawable changed or left out, like the .fig notes.
        for w in warnings {
            eprintln!("note: {w}");
        }
        Ok::<_, CliError>(bytes)
    };
    if single {
        let bytes = match target {
            Some(t) => bytes_of(&[t])?,
            None if fmt == ExportFormat::Pdf => page_pdf(&doc, page, print)?,
            None => bytes_of(&children(&doc, page))?,
        };
        write(out, &bytes)?;
        println!("{out}");
        return Ok(ExitCode::SUCCESS);
    }
    figcraft_engine::file_access::create_dir_all(out).map_err(|e| CliError::Failed(e.to_string()))?;
    let ids = match target {
        Some(t) => vec![t],
        None => {
            let kids = children(&doc, page);
            let frames: Vec<NodeId> = kids.iter().copied().filter(|i| doc.node(*i).is_some_and(|n| n.kind.is_frame_like())).collect();
            if frames.is_empty() { kids } else { frames }
        }
    };
    if ids.is_empty() {
        return Err(CliError::Failed("the page is empty: nothing to export".into()));
    }
    let mut used = std::collections::BTreeSet::new();
    let mut next_suffix: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for id in ids {
        let base = sanitize(doc.node(id).map(|n| n.name.as_str()).unwrap_or("layer"));
        let base_lower = base.to_lowercase();
        let next = next_suffix.entry(base_lower).or_insert(1);
        let name = loop {
            let k = *next;
            *next += 1;
            let candidate = if k == 1 { base.clone() } else { format!("{base} {k}") };
            if used.insert(candidate.to_lowercase()) {
                break candidate;
            }
        };
        let path = Path::new(out).join(format!("{name}.{}", fmt.ext())).to_string_lossy().into_owned();
        write(&path, &bytes_of(&[id])?)?;
        println!("{path}");
    }
    Ok(ExitCode::SUCCESS)
}

fn refuse_dropping_images(input: &str, out: &str, images_left_out: usize) -> CliResult<()> {
    if images_left_out > 0 && figcraft_engine::file_access::same_file(input, out) {
        return Err(CliError::Failed(format!(
            "{out} wasn't written: {images_left_out} of its images weren't loaded (together they pass what FigCraft reads from one file), and writing over it would delete them. Write to another path; {input} keeps them."
        )));
    }
    Ok(())
}

fn refuse_dropping_images_from_script(input: Option<(&str, usize)>, active: &figcraft_engine::DocState, out: &str) -> CliResult<()> {
    if let Some((input, images_left_out)) = input {
        refuse_dropping_images(input, out, images_left_out)?;
    }
    if let Some(images_left_out) = active.images_lost_saving_to(out) {
        refuse_dropping_images(out, out, images_left_out)?;
    }
    Ok(())
}

pub fn convert(input: &str, out: &str) -> CliResult<ExitCode> {
    let (loaded, _) = figcraft_engine::cmd::file::load_path(input, None).map_err(|e| CliError::Failed(format!("can't open {input}: {e}")))?;
    refuse_dropping_images(input, out, loaded.images_past_budget)?;
    if ext(input) == "penpot" {
        for note in &loaded.issues {
            eprintln!("note: {note}");
        }
    }
    let doc = loaded.doc;
    let page = doc
        .current_page
        .filter(|p| doc.pages.contains(p))
        .or_else(|| doc.first_page())
        .ok_or_else(|| CliError::Failed("the document has no pages".into()))?;
    write_doc(&doc, page, out)?;
    println!("{out}");
    Ok(ExitCode::SUCCESS)
}

pub fn render(file: &str, out: &str, scale: f64, page: Option<&str>) -> CliResult<ExitCode> {
    let doc = load(file)?;
    let page = page_named(&doc, page)?;
    let e = ext(out);
    if !matches!(e.as_str(), "png" | "jpg" | "jpeg" | "webp") {
        return Err(CliError::Usage(format!("render writes .png, .jpg or .webp, not `{out}`")));
    }
    let pix = render_page(&doc, page, scale)?;
    write(out, &encode(&doc, &pix, &e)?)?;
    println!("{out} ({}×{})", pix.width(), pix.height());
    Ok(ExitCode::SUCCESS)
}

/// Parse a script: a JSON array of steps, `{"commands": [...]}`, or JSON lines.
pub fn parse_script(text: &str) -> Result<Vec<BatchStep>, String> {
    let steps: Vec<Value> = match serde_json::from_str::<Value>(text) {
        Ok(Value::Array(a)) => a,
        Ok(Value::Object(o)) if o.contains_key("commands") => {
            o.get("commands").and_then(Value::as_array).cloned().ok_or("`commands` must be an array")?
        }
        Ok(v @ Value::Object(_)) => vec![v],
        Ok(_) => return Err("a script is an array of {\"command\", \"params\"} objects".into()),
        Err(_) => text
            .lines()
            .enumerate()
            .filter(|(_, l)| !l.trim().is_empty() && !l.trim_start().starts_with("//"))
            .map(|(i, l)| serde_json::from_str::<Value>(l).map_err(|e| format!("line {}: {e}", i + 1)))
            .collect::<Result<_, _>>()?,
    };
    steps
        .into_iter()
        .enumerate()
        .map(|(i, s)| {
            let cmd = s.get("command").and_then(Value::as_str).ok_or_else(|| format!("step {i}: `command` (a string) is required"))?;
            figcraft_engine::batch::refuse_unknown_step_keys(i, &s).map_err(|e| e.to_string())?;
            let params = match s.get("params") {
                None | Some(Value::Null) => json!({}),
                Some(p @ Value::Object(_)) => p.clone(),
                Some(_) => return Err(format!("step {i} ({cmd}): `params` must be an object")),
            };
            let name = figcraft_engine::batch::step_name(i, &s).map_err(|e| e.to_string())?;
            Ok(BatchStep { name, ..BatchStep::new(cmd, params) })
        })
        .collect()
}

pub fn run_script(script: &str, input: Option<&str>, out: Option<&str>) -> CliResult<ExitCode> {
    let bytes = figcraft_engine::file_access::read(script).map_err(|e| CliError::Failed(e.to_string()))?;
    let text = String::from_utf8(bytes).map_err(|_| CliError::Failed(format!("{script}: not UTF-8")))?;
    let steps = parse_script(&text).map_err(|e| CliError::Failed(format!("{script}: {e}")))?;
    let (mut s, input_images_left_out) = match input {
        Some(path) => {
            let mut s = Session::empty();
            s.execute("file.open", &json!({"path": path})).map_err(|e| CliError::Failed(format!("can't open {path}: {e}")))?;
            let left_out = s.ds().map_err(|e| CliError::Failed(e.to_string()))?.images_lost_saving_to(path).unwrap_or(0);
            (s, left_out)
        }
        None => (Session::new(), 0),
    };
    // One undo step and all-or-nothing, the way the same commands behave through `command.batch`
    // (MCP, the control channel, plugins): `{"$step": n}` and `{"$ref": "name"}` resolve against the
    // steps before it, and a step that fails takes the whole script back instead of leaving the
    // earlier steps applied. The result lines are held back until the script is through, so none of
    // them reports work that was rolled back.
    let mut scope = BatchScope::new(&steps).map_err(|e| CliError::Failed(format!("{script}: {e}")))?;
    let mut failed_at = 0;
    let outcome = s.grouped(&format!("run {script}"), |s| {
        for (i, step) in steps.iter().enumerate() {
            failed_at = i;
            s.batch_step(&mut scope, &step.command, &step.params)?;
            s.ui_requests.clear();
        }
        Ok(())
    });
    if let Err(e) = outcome {
        let cmd = steps.get(failed_at).map_or("", |step| step.command.as_str());
        let i = failed_at;
        println!("{}", json!({"step": i, "command": cmd, "ok": false, "error": e.to_string(), "rolledBack": true}));
        eprintln!("error: step {i} ({cmd}) failed: {e}");
        eprintln!("the script's document changes were rolled back");
        return Ok(ExitCode::from(1));
    }
    for (i, (step, v)) in steps.iter().zip(scope.results()).enumerate() {
        println!("{}", json!({"step": i, "command": step.command, "ok": true, "result": v}));
    }
    if let Some(out) = out {
        let ds = s.ds().map_err(|e| CliError::Failed(e.to_string()))?;
        refuse_dropping_images_from_script(input.map(|input| (input, input_images_left_out)), ds, out)?;
        if ext(out) == "figcraft" {
            // Through the engine, so the version history is kept.
            write(out, &figcraft_engine::cmd::file::file_bytes(ds).map_err(|e| CliError::Failed(e.to_string()))?)?;
        } else {
            write_doc(&ds.doc, ds.page, out)?;
        }
        eprintln!("wrote {out}");
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_formats() {
        assert_eq!(parse_script(r#"[{"command":"a"},{"command":"b","params":{"x":1}}]"#).unwrap().len(), 2);
        assert_eq!(parse_script(r#"{"commands":[{"command":"a"}]}"#).unwrap().len(), 1);
        assert_eq!(parse_script("{\"command\":\"a\"}\n\n// note\n{\"command\":\"b\"}\n").unwrap().len(), 2);
        assert!(parse_script("[{\"params\":{}}]").is_err());
        assert!(parse_script("[{\"command\":\"a\",\"params\":3}]").is_err());
        assert!(parse_script("not json\n").is_err());
        assert!(parse_script("42").is_err());
        // `args` for `params` would run the step without them (#1413).
        let e = parse_script(r#"[{"command":"a"},{"command":"b","args":{"x":1}}]"#).unwrap_err().to_string();
        assert!(e.contains("step 1: unknown key `args`"), "{e}");
    }

    #[test]
    fn sanitizes_names() {
        assert_eq!(sanitize("Home / Hero"), "Home _ Hero");
        assert_eq!(sanitize("../.."), "_");
        assert_eq!(sanitize(""), "layer");
        // The same characters the app's export keeps (#1154): only what file systems refuse goes.
        assert_eq!(sanitize("Homepage - desktop (live + verbeteringen)"), "Homepage - desktop (live + verbeteringen)");
        assert_eq!(sanitize("Q&A, 50% #1 [v2] {x} ~é"), "Q&A, 50% #1 [v2] {x} ~é");
        assert_eq!(sanitize("a\\b:c*d?e\"f<g>h|i\tj"), "a_b_c_d_e_f_g_h_i_j");
        assert_eq!(sanitize(" .hidden. "), "hidden");
        assert_eq!(sanitize(" . .name. . "), "name", "spaces and dots at the edges, mixed");
        assert_eq!(sanitize(&"x".repeat(300)).chars().count(), 100);
        assert_eq!(sanitize(&"\u{1F600}".repeat(100)), "\u{1F600}".repeat(50), "at most 200 bytes, whole characters");
    }

    #[test]
    fn default_page_skips_leading_divider() {
        let mut doc = Document::new("divider");
        let page = doc.first_page().unwrap();
        doc.create(page, figcraft_doc::NodeKind::Frame, "First", 0.0, 0.0, 40.0, 30.0).unwrap();
        let divider = doc.add_page("- Divider");
        doc.pages.retain(|p| *p != divider);
        doc.pages.insert(0, divider);
        assert!(doc.is_divider(divider));
        assert_eq!(page_named(&doc, None).unwrap(), page);
        assert_eq!(page_named(&doc, Some("- Divider")).unwrap(), divider);
        let pm = render_page(&doc, page_named(&doc, None).unwrap(), 1.0).unwrap();
        assert_eq!((pm.width(), pm.height()), (40, 30));
        assert!(render_page(&doc, divider, 1.0).is_err());
    }

    #[test]
    fn export_depth_errors_preserve_conversion_destinations() {
        let mut doc = Document::new("export depth");
        let page = doc.first_page().unwrap();
        let root = doc.create(page, figcraft_doc::NodeKind::Frame, "root", 0.0, 0.0, 32.0, 32.0).unwrap();
        let mut parent = root;
        for _ in 1..257 {
            parent = doc.create(parent, figcraft_doc::NodeKind::Group, "group", 0.0, 0.0, 32.0, 32.0).unwrap();
        }
        doc.create(parent, figcraft_doc::NodeKind::Rectangle, "leaf", 0.0, 0.0, 8.0, 8.0).unwrap();
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("figcraft-cli-export-depth-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        for ext in ["svg", "pdf"] {
            let path = dir.join(format!("previous.{ext}"));
            std::fs::write(&path, b"previous export").unwrap();
            let CliError::Failed(error) = write_doc(&doc, page, path.to_str().unwrap()).unwrap_err() else {
                panic!("depth overflow must be an operation failure");
            };
            assert!(error.contains("maximum export depth of 256"), "{ext}: {error}");
            assert!(error.contains("select an inner layer"), "{ext}: {error}");
            assert_eq!(std::fs::read(&path).unwrap(), b"previous export");
        }
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("figcraft-cli-{tag}-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        dir
    }

    /// A script resolves `{"$step": n}` like the same commands through `command.batch`. Each step
    /// used to run on its own, so the reference reached `parent` unresolved, `node.create` read it
    /// as "no parent" and the layer landed on the page — with every step reporting success.
    #[test]
    fn a_script_puts_a_layer_in_a_layer_an_earlier_step_made() {
        let dir = temp_dir("run-step");
        let script = dir.join("script.json");
        std::fs::write(
            &script,
            r#"[
                {"command": "node.create", "params": {"type": "frame", "name": "Card", "x": 0, "y": 0, "width": 200, "height": 100}},
                {"command": "node.create", "params": {"type": "rectangle", "name": "child", "parent": {"$step": 0}, "x": 10, "y": 10, "width": 20, "height": 20}}
            ]"#,
        )
        .unwrap();
        let out = dir.join("out.figcraft");
        run_script(script.to_str().unwrap(), None, Some(out.to_str().unwrap())).unwrap();

        let mut s = Session::empty();
        s.execute("file.open", &json!({"path": out.to_str().unwrap()})).unwrap();
        let doc = s.doc().unwrap();
        let page = s.page().unwrap();
        let top = doc.node(page).unwrap().children.clone();
        assert_eq!(top.len(), 1, "only the frame is top level, not the rectangle too");
        assert_eq!(doc.node(top[0]).unwrap().name, "Card");
        let inner = doc.node(top[0]).unwrap().children.clone();
        assert_eq!(inner.len(), 1, "the rectangle went inside the frame");
        assert_eq!(doc.node(inner[0]).unwrap().name, "child");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A script names its steps with `as` and reads paths into their results, like `command.batch`.
    #[test]
    fn a_script_refers_to_named_steps_and_paths() {
        let dir = temp_dir("run-ref");
        let script = dir.join("script.json");
        std::fs::write(
            &script,
            r#"[
                {"command": "node.create", "as": "card", "params": {"type": "frame", "name": "Card", "x": 0, "y": 0, "width": 200, "height": 100}},
                {"command": "edit.duplicate", "as": "copy", "params": {"ids": [{"$ref": "card"}]}},
                {"command": "node.set", "params": {"ids": [{"$ref": "copy", "field": "ids/0"}], "props": {"name": "Copy"}}}
            ]"#,
        )
        .unwrap();
        let out = dir.join("out.figcraft");
        run_script(script.to_str().unwrap(), None, Some(out.to_str().unwrap())).unwrap();

        let mut s = Session::empty();
        s.execute("file.open", &json!({"path": out.to_str().unwrap()})).unwrap();
        let doc = s.doc().unwrap();
        let top = doc.node(s.page().unwrap()).unwrap().children.clone();
        let names: Vec<_> = top.iter().map(|id| doc.node(*id).unwrap().name.clone()).collect();
        assert_eq!(names, ["Card", "Copy"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Two steps with one name fail before anything runs.
    #[test]
    fn a_script_with_a_name_twice_runs_nothing() {
        let dir = temp_dir("run-dup-name");
        let script = dir.join("script.json");
        std::fs::write(&script, r#"[{"command": "select.none", "as": "a"}, {"command": "select.none", "as": "a"}]"#).unwrap();
        let Err(CliError::Failed(e)) = run_script(script.to_str().unwrap(), None, None) else { panic!("a name twice must fail") };
        assert!(e.contains("step 1: `as` \"a\" is already step 0's name"), "{e}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A step that fails takes the whole script back: nothing is written, and the failure says so.
    #[test]
    fn a_failed_step_leaves_no_output() {
        let dir = temp_dir("run-rollback");
        let script = dir.join("script.json");
        std::fs::write(
            &script,
            r#"[
                {"command": "node.create", "params": {"type": "frame", "name": "Card", "x": 0, "y": 0, "width": 200, "height": 100}},
                {"command": "node.set", "params": {"ids": [99999], "props": {"name": "nope"}}}
            ]"#,
        )
        .unwrap();
        let out = dir.join("out.figcraft");
        // The failure is the script's, not the call's: the CLI reports it and exits non-zero.
        run_script(script.to_str().unwrap(), None, Some(out.to_str().unwrap())).unwrap();
        assert!(!out.exists(), "a script that failed writes nothing");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_script_output_over_its_input_is_refused_whichever_tab_the_script_left_active() {
        let dir = temp_dir("script-images-left-out");
        let input = dir.join("big.figcraft");
        std::fs::write(&input, b"kept").unwrap();
        let link = dir.join("link.figcraft");
        std::fs::hard_link(&input, &link).unwrap();
        let (input, link, other) = (input.to_str().unwrap(), link.to_str().unwrap(), dir.join("other.figcraft"));
        let duplicate = Session::new();
        let active = duplicate.ds().unwrap();

        for out in [input, link] {
            let refused = refuse_dropping_images_from_script(Some((input, 3)), active, out);
            assert!(matches!(&refused, Err(CliError::Failed(m)) if m.contains("3 of its images")), "{out}");
        }
        assert!(refuse_dropping_images_from_script(Some((input, 3)), active, other.to_str().unwrap()).is_ok());
        assert!(refuse_dropping_images_from_script(Some((input, 0)), active, input).is_ok());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
