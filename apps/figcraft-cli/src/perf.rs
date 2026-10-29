//! `figcraft-cli perf`: synthetic large documents and timings of the hot paths.
//!
//! The documents come from [`figcraft_engine::synthetic`] (screens with an auto layout header, a
//! card group, button instances and a grid of shapes and text). Every row is the mean of several
//! runs; use a release build for meaningful numbers (`cargo xtask bench`).

use std::process::ExitCode;
use std::time::Instant;

use figcraft_doc::geom::Point;
use figcraft_doc::{Document, NodeId, NodeKind};
use figcraft_engine::{PointerEvent, PointerKind, Session, ViewState};
use serde_json::json;

use crate::{CliError, CliResult};

/// Canvas size of the measured viewport (a typical laptop window).
const VIEW_W: u32 = 1440;
const VIEW_H: u32 = 900;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

/// Mean milliseconds of `n` runs of `f`.
fn mean(n: usize, mut f: impl FnMut(usize)) -> f64 {
    let t = Instant::now();
    for i in 0..n {
        f(i);
    }
    ms(t) / n.max(1) as f64
}

/// Set when a measured command fails (the numbers would be meaningless).
static FAILED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn exec(s: &mut Session, id: &str, p: serde_json::Value) {
    if let Err(e) = s.execute(id, &p) {
        eprintln!("{id}: {e}");
        FAILED.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

fn failed(e: impl std::fmt::Display) -> CliError {
    CliError::Failed(e.to_string())
}

/// The `k`-th layer of `kind` among the descendants of `root` (pre-order).
fn find(doc: &Document, root: NodeId, k: usize, pred: impl Fn(&figcraft_doc::Node) -> bool) -> Option<NodeId> {
    doc.subtree(root).into_iter().filter(|i| doc.node(*i).is_some_and(&pred)).nth(k)
}

/// What the canvas renders with: a preview, with levels of detail when zoomed out.
fn canvas() -> figcraft_render::Options {
    figcraft_render::Options { lod: true, ..Default::default() }
}

fn render(doc: &Document, page: NodeId, v: &ViewState, opts: &figcraft_render::Options) -> bool {
    let (ox, oy) = v.origin();
    let view = figcraft_render::View { origin: (ox, oy), scale: v.zoom, width: VIEW_W, height: VIEW_H };
    figcraft_render::render_view(doc, page, &view, opts).is_some()
}

/// Render `v` panned right by `dx` page units through the retained renderer, as the canvas does.
fn retained(r: &mut figcraft_render::Retained, doc: &std::sync::Arc<Document>, page: NodeId, v: &ViewState, dx: f64) -> bool {
    let (ox, oy) = v.origin();
    let view = figcraft_render::View { origin: (ox + dx, oy), scale: v.zoom, width: VIEW_W, height: VIEW_H };
    r.render(doc, page, &view, &canvas()).is_some()
}

struct Table {
    rows: Vec<(String, Vec<f64>)>,
}

impl Table {
    fn put(&mut self, col: usize, name: &str, v: f64) {
        let i = match self.rows.iter().position(|r| r.0 == name) {
            Some(i) => i,
            None => {
                self.rows.push((name.to_string(), Vec::new()));
                self.rows.len() - 1
            }
        };
        if let Some(r) = self.rows.get_mut(i) {
            r.1.resize(col + 1, f64::NAN);
            if let Some(c) = r.1.get_mut(col) {
                *c = v;
            }
        }
    }
}

/// Measure one document size into column `col`.
fn measure(table: &mut Table, col: usize, layers: usize) -> CliResult<()> {
    let t = Instant::now();
    let (doc, page) = figcraft_engine::synthetic::large_document(layers).map_err(failed)?;
    table.put(col, "build + normalize (like open)", ms(t));

    let t = Instant::now();
    let bytes = figcraft_format::save_bytes(&doc, None).map_err(failed)?;
    table.put(col, "save .figcraft", ms(t));
    let t = Instant::now();
    let (back, _) = figcraft_format::load_bytes(&bytes).map_err(failed)?;
    table.put(col, "load .figcraft", ms(t));
    drop(back);

    // Rendering a 1440×900 viewport as the canvas does: 100% on the first screen, and zoomed out to
    // fit the page (also exactly, as an export of the same area would).
    let bounds = doc.page_content_bounds(page).ok_or_else(|| failed("empty page"))?;
    let mut at100 = ViewState { vw: f64::from(VIEW_W), vh: f64::from(VIEW_H), ..Default::default() };
    at100.cx = f64::from(VIEW_W) / 2.0;
    at100.cy = f64::from(VIEW_H) / 2.0;
    table.put(col, "render 1440×900 @100%", mean(5, |_| _ = render(&doc, page, &at100, &canvas())));
    let mut fit = at100.clone();
    fit.fit(bounds, 1.0);
    table.put(col, "render 1440×900 zoomed to fit", mean(5, |_| _ = render(&doc, page, &fit, &canvas())));
    table.put(col, "render 1440×900 zoomed to fit, exact (no LOD)", mean(3, |_| _ = render(&doc, page, &fit, &Default::default())));

    // The canvas keeps its last image: a pan renders the uncovered strips, an edit the layers'
    // areas.
    let doc = std::sync::Arc::new(doc);
    let pans = 10;
    // Zoomed out, but with content right up to the edges (a fitted page has margins).
    let mut out = fit.clone();
    out.zoom *= 2.0;
    table.put(col, "render 1440×900 zoomed out (2× fit)", mean(5, |_| _ = render(&doc, page, &out, &canvas())));
    for (name, v) in [("pan 40 px @100% (retained)", &at100), ("pan 40 px zoomed out (retained)", &out)] {
        let mut r = figcraft_render::Retained::default();
        let _ = retained(&mut r, &doc, page, v, 0.0);
        table.put(col, name, mean(pans, |i| _ = retained(&mut r, &doc, page, v, if i % 2 == 0 { 40.0 / v.zoom } else { 0.0 })));
    }
    for (name, v) in [("move one layer, re-render @100% (retained)", &at100), ("move one layer, re-render zoomed out (retained)", &out)] {
        // A shape in the screen nearest the view's centre.
        let centre = Point::new(v.cx, v.cy);
        let screens = doc.node(page).map(|p| p.children.clone()).unwrap_or_default();
        let nearest =
            screens.into_iter().filter_map(|c| doc.world_bounds(c).map(|b| (c, (b.center() - centre).hypot2()))).min_by(|a, b| a.1.total_cmp(&b.1));
        let target = nearest.and_then(|(c, _)| doc.node(c)?.children.last().copied()).ok_or_else(|| failed("no screen to edit"))?;
        let moved: Vec<std::sync::Arc<Document>> = (0..2)
            .map(|k| {
                let mut d = (*doc).clone();
                if let Some(n) = d.node_mut(target) {
                    n.transform = figcraft_doc::geom::Affine::translate((f64::from(k) * 6.0, 0.0)) * n.transform;
                }
                std::sync::Arc::new(d)
            })
            .collect();
        let mut r = figcraft_render::Retained::default();
        let _ = retained(&mut r, &doc, page, v, 0.0);
        table.put(col, name, mean(10, |i| _ = moved.get(i % 2).map(|d| retained(&mut r, d, page, v, 0.0))));
    }
    let doc = std::sync::Arc::unwrap_or_clone(doc);

    // Hit testing (a hover under the pointer).
    let hits = 2000;
    let pt = |i: usize| {
        let (fx, fy) = ((i * 7919 % 1000) as f64 / 1000.0, (i * 104_729 % 1000) as f64 / 1000.0);
        Point::new(bounds.x0 + bounds.width() * fx, bounds.y0 + bounds.height() * fy)
    };
    {
        let result = figcraft_engine::booleans::result_outline(&doc, &[]);
        table.put(col, "hit test (µs each)", mean(hits, |i| _ = doc.hit_test(page, pt(i), 1.0, Some(&result))) * 1000.0);
    }
    measure_boolean_drag(table, col);

    let mut s = Session::with_document(doc);
    let doc = s.doc().map_err(failed)?.clone();
    let rect =
        find(&doc, page, layers / 2 / 50, |n| matches!(n.kind, NodeKind::Rectangle) && n.name == "Rectangle").ok_or_else(|| failed("no rect"))?;
    let title = find(&doc, page, 0, |n| n.name == "Title").ok_or_else(|| failed("no title"))?;
    let button = find(&doc, page, 0, |n| matches!(n.kind, NodeKind::Component { .. })).ok_or_else(|| failed("no component"))?;
    let button_bg = doc.node(button).and_then(|b| b.children.first().copied()).ok_or_else(|| failed("no button layers"))?;

    // Hover with the Move tool: hit test + select target, as the canvas does per pointer move.
    let zoom = at100.zoom;
    table.put(
        col,
        "hover pointer move (µs each)",
        mean(hits, |i| {
            let p = pt(i);
            let ev = PointerEvent {
                kind: PointerKind::Hover,
                x: p.x,
                y: p.y,
                sx: 0.0,
                sy: 0.0,
                shift: false,
                alt: false,
                cmd: false,
                ctrl: false,
                zoom,
                pressure: None,
            };
            let _ = s.pointer(&ev);
        }) * 1000.0,
    );

    // Single-layer edits through the engine (command + normalize + undo step).
    let colors = ["#FF0000", "#00FF00"];
    table.put(col, "node.set fill on one layer", mean(20, |i| exec(&mut s, "node.set", json!({"ids": [rect], "fill": colors[i % 2]}))));
    exec(&mut s, "select.set", json!({"ids": [rect]}));
    table.put(col, "transform.move one layer", mean(20, |i| exec(&mut s, "transform.move", json!({"dx": if i % 2 == 0 { 1 } else { -1 }, "dy": 0}))));
    table.put(
        col,
        "edit text in an auto layout frame",
        mean(20, |i| exec(&mut s, "node.set", json!({"ids": [title], "characters": format!("Title {i}")}))),
    );
    table.put(
        col,
        "edit main component (all instances follow)",
        mean(5, |i| exec(&mut s, "node.set", json!({"ids": [button_bg], "fill": colors[i % 2]}))),
    );
    table.put(col, "undo", mean(10, |_| _ = s.undo()));
    table.put(col, "redo", mean(10, |_| _ = s.redo()));

    // A drag: every pointer move re-applies the move to the gesture's snapshot.
    exec(&mut s, "select.set", json!({"ids": [rect]}));
    let _ = s.begin_interaction("Move");
    table.put(
        col,
        "drag preview frame (one layer)",
        mean(20, |i| {
            let _ = s.preview(|d, sel| {
                for id in sel.iter() {
                    if let Some(n) = d.node_mut(*id) {
                        n.transform = figcraft_doc::geom::Affine::translate((i as f64, 0.0)) * n.transform;
                    }
                }
                Ok(())
            });
        }),
    );
    let _ = s.commit_interaction();

    // Raw normalisation after a one-node edit (no command machinery).
    let before = s.doc().map_err(failed)?.clone();
    table.put(
        col,
        "normalize after a one-node edit",
        mean(50, |i| {
            let mut d = before.clone();
            if let Some(n) = d.node_mut(rect) {
                n.opacity = if i % 2 == 0 { 0.5 } else { 1.0 };
            }
            figcraft_engine::normalize(&mut d, &before);
        }),
    );

    table.put(col, "document.inspect (depth 8)", mean(3, |_| _ = figcraft_engine::inspect::document(&s, 8, false)));
    exec(&mut s, "select.none", json!({}));
    let t = Instant::now();
    exec(&mut s, "select.all", json!({}));
    exec(&mut s, "transform.move", json!({"dx": 10, "dy": 0}));
    table.put(col, "select all + move every screen", ms(t));
    Ok(())
}

/// Live boolean editing: one drag frame of an operand of a boolean group of 24 overlapping
/// circles, i.e. the group's result recomputed with the operand somewhere new (each frame a new
/// position, as in a real drag). The top operand recombines one step; the bottom one, all 23.
fn measure_boolean_drag(table: &mut Table, col: usize) {
    let mut d = Document::new("booleans");
    let Some(page) = d.first_page() else { return };
    let Ok(b) = d.create(page, NodeKind::BooleanOp { op: figcraft_doc::BoolOp::Union }, "Union", 0.0, 0.0, 0.0, 0.0) else { return };
    let kids: Vec<NodeId> = (0..24)
        .filter_map(|i| {
            let a = f64::from(i) * std::f64::consts::TAU / 24.0;
            d.create(
                b,
                NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 },
                "Circle",
                200.0 + 120.0 * a.cos(),
                200.0 + 120.0 * a.sin(),
                90.0,
                90.0,
            )
            .ok()
        })
        .collect();
    let _ = d.fit_to_children(b);
    let frame = |which: usize, k: usize| {
        let mut moved = d.clone();
        if let Some(n) = kids.get(which).and_then(|id| moved.node_mut(*id)) {
            // Distinct offsets per run and column, so no frame is served from the cache.
            n.transform = figcraft_doc::geom::Affine::translate((0.37 * (k + 1) as f64 + 1000.0 * col as f64, 0.11 * k as f64)) * n.transform;
        }
        moved.node(b).and_then(|n| figcraft_render::node_outline(&moved, n)).is_some()
    };
    table.put(col, "boolean of 24: drag frame, top operand", mean(20, |k| _ = frame(23, k)));
    table.put(col, "boolean of 24: drag frame, bottom operand", mean(5, |k| _ = frame(0, 100 + k)));
}

/// Time the hot paths for each document size in `sizes` (layers) and, with the synthetic design
/// system, for each instance count in `instances`; print the tables.
pub fn run(sizes: &[usize], instances: &[usize], system_fonts: bool) -> CliResult<ExitCode> {
    // Text renders with system fonts too, like the desktop app; scanned up front, outside the timings.
    if system_fonts {
        figcraft_text::scan_system_fonts();
    }
    let mut table = Table { rows: Vec::new() };
    for (col, n) in sizes.iter().enumerate() {
        eprintln!("measuring {n} layers…");
        measure(&mut table, col, *n)?;
    }
    let mut comps = Table { rows: Vec::new() };
    for (col, n) in instances.iter().enumerate() {
        eprintln!("measuring components with {n} instances…");
        measure_components(&mut comps, col, *n)?;
    }
    if FAILED.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(failed("a measured command failed"));
    }
    println!(
        "figcraft perf ({} {}; ms unless noted; mean of several runs)",
        std::env::consts::OS,
        if cfg!(debug_assertions) { "debug build" } else { "release build" }
    );
    print_table(&table, &sizes.iter().map(|n| format!("{n} layers")).collect::<Vec<_>>());
    if !instances.is_empty() {
        println!();
        print_table(&comps, &instances.iter().map(|n| format!("{n} inst.")).collect::<Vec<_>>());
    }
    Ok(ExitCode::SUCCESS)
}

/// Print `table` as Markdown with one column per `heads`.
fn print_table(table: &Table, heads: &[String]) {
    let w = table.rows.iter().map(|r| r.0.chars().count()).max().unwrap_or(10);
    let mut head = format!("| {:<w$} |", "operation");
    let mut rule = format!("|{}|", "-".repeat(w + 2));
    for h in heads {
        head.push_str(&format!(" {h:>10} |"));
        rule.push_str(&format!("{}:|", "-".repeat(11)));
    }
    println!("{head}");
    println!("{rule}");
    for (name, vals) in &table.rows {
        let mut line = format!("| {name:<w$} |");
        for v in vals {
            line.push_str(&format!(" {:>10} |", if v.is_finite() { format!("{v:.2}") } else { "-".into() }));
        }
        println!("{line}");
    }
}

/// The first top-level instance in `inst` of component set `set` with a layer at name path
/// `path`: (instance, layer).
fn instance_with(doc: &Document, inst: &[NodeId], set: NodeId, path: &[&str]) -> Option<(NodeId, NodeId)> {
    use figcraft_engine::components::variant_set;
    inst.iter().copied().find_map(|i| {
        let NodeKind::Instance { component, .. } = &doc.node(i)?.kind else { return None };
        if variant_set(doc, *component) != Some(set) {
            return None;
        }
        figcraft_engine::synthetic_ds::find_path(doc, i, path).map(|l| (i, l))
    })
}

/// Time one command (ms); undone afterwards when `undo`.
fn timed(s: &mut Session, id: &str, p: serde_json::Value, undo: bool) -> f64 {
    let t = Instant::now();
    exec(s, id, p);
    let v = ms(t);
    if undo {
        let _ = s.undo();
    }
    v
}

/// Components at scale: the synthetic design system (about 300 components, variables, styles,
/// nesting four deep) used by `instances` instances with overrides at every level, in one file and
/// as a library in another, into column `col`.
fn measure_components(table: &mut Table, col: usize, instances: usize) -> CliResult<()> {
    use figcraft_engine::synthetic_ds::{DsSpec, design_system_document, design_system_with_instances, find_path, place_instances};
    let spec = DsSpec::FULL;
    let t = Instant::now();
    let (doc, ds, inst) = design_system_with_instances(&spec, instances).map_err(failed)?;
    table.put(col, "build design system + instances (like open)", ms(t));
    table.put(col, "layers in the file", doc.node_count() as f64);
    let glyph = doc
        .node(ds.icons.first().copied().ok_or_else(|| failed("no icons"))?)
        .and_then(|n| n.children.first().copied())
        .ok_or_else(|| failed("no glyph"))?;
    let icon0 = ds.icons.first().copied().ok_or_else(|| failed("no icons"))?;
    let uses = doc.nodes().filter(|n| matches!(&n.kind, NodeKind::Instance { component, .. } if *component == icon0)).count();
    table.put(col, "instances of the edited icon main (nested included)", uses as f64);
    // Layers mirroring the icon's glyph at any depth (copies of copies included).
    let mirrors = |doc: &Document, of: NodeId| {
        doc.nodes()
            .filter(|n| {
                let mut cur = n.source;
                for _ in 0..32 {
                    match cur {
                        Some(x) if x == of => return true,
                        Some(x) => cur = doc.node(x).and_then(|m| m.source),
                        None => return false,
                    }
                }
                false
            })
            .count()
    };
    table.put(col, "layers following the edited glyph (all depths)", mirrors(&doc, glyph) as f64);
    let mut s = Session::with_document(doc);
    let page = s.doc().map_err(failed)?.page_of(inst.first().copied().ok_or_else(|| failed("no instances"))?).ok_or_else(|| failed("no page"))?;
    if let Ok(d) = s.ds_mut() {
        d.page = page;
    }
    let colors = ["#FF0000", "#00AA00"];
    exec(&mut s, "variable.bind", json!({"ids": [glyph], "field": "fill", "variable": null}));
    table.put(
        col,
        "edit deep main: icon colour (all copies follow)",
        mean(5, |i| exec(&mut s, "node.set", json!({"ids": [glyph], "props": {"fill": colors[i % 2]}}))),
    );
    table.put(
        col,
        "edit deep main: icon glyph size",
        mean(5, |i| exec(&mut s, "node.set", json!({"ids": [glyph], "props": {"width": 14 + (i % 2) * 2}}))),
    );
    table.put(
        col,
        "edit deep main: add a layer to the icon",
        mean(3, |_| {
            exec(&mut s, "shape.rectangle", json!({"x": 1, "y": 1, "width": 2, "height": 2, "parent": icon0}));
            let _ = s.undo();
        }),
    );
    // Every button variant's label: used by buttons, cards' footers and navs' items.
    let doc = s.doc().map_err(failed)?.clone();
    let labels: Vec<u64> =
        doc.node(ds.button).map(|set| set.children.iter().filter_map(|v| find_path(&doc, *v, &["Label"])).map(|l| l.0).collect()).unwrap_or_default();
    let label_uses: usize = labels.iter().map(|l| mirrors(&doc, NodeId(*l))).sum();
    table.put(col, "layers following the 72 button labels (all depths)", label_uses as f64);
    table.put(
        col,
        "edit 72 button mains: label size (all copies follow)",
        mean(3, |i| exec(&mut s, "node.set", json!({"ids": labels, "props": {"fontSize": 13 + (i % 2) * 2}}))),
    );
    let doc = s.doc().map_err(failed)?.clone();
    // Small workloads may not contain every instance type; the rows needing a missing one stay empty.
    if let Some((card, _)) = instance_with(&doc, &inst, ds.card, &["Footer button", "Icon", "Glyph"]) {
        table.put(
            col,
            "switch a card's variant (nested overrides kept)",
            mean(5, |i| {
                exec(&mut s, "instance.setProperties", json!({"ids": [card], "properties": {"Size": if i % 2 == 0 { "Large" } else { "Small" }}}))
            }),
        );
    } else {
        table.put(col, "switch a card's variant (nested overrides kept)", f64::NAN);
    }
    let nav = instance_with(&doc, &inst, ds.nav, &["Profile"]).map(|(nav, _)| nav);
    if let Some(nav) = nav {
        table.put(col, "reset all changes on a nav", timed(&mut s, "component.resetInstance", json!({"ids": [nav]}), true));
        table.put(col, "detach a nav", timed(&mut s, "component.detach", json!({"ids": [nav]}), true));
    } else {
        table.put(col, "reset all changes on a nav", f64::NAN);
        table.put(col, "detach a nav", f64::NAN);
    }
    if let Some((button, bglyph)) = instance_with(&doc, &inst, ds.button, &["Icon", "Glyph"]) {
        table.put(col, "push a button's changes to its main", timed(&mut s, "component.pushChanges", json!({"id": button}), true));
        // A button glyph the drop really lands on: screens can overlap (text sizes differ without
        // fonts), so ask the hit test the swap uses.
        let now = s.doc().map_err(failed)?.clone();
        let page = s.page().map_err(failed)?;
        let drop_at = std::iter::once(bglyph)
            .chain(inst.iter().filter_map(|i| {
                let NodeKind::Instance { component, .. } = &now.node(*i)?.kind else { return None };
                (figcraft_engine::components::variant_set(&now, *component) == Some(ds.button)).then(|| find_path(&now, *i, &["Icon", "Glyph"]))?
            }))
            .filter_map(|g| now.world_bounds(g))
            .find(|b| figcraft_engine::drop::swap_target(&now, page, b.center(), None, None).is_some());
        let other = ds.icons.get(5).copied().unwrap_or(icon0);
        table.put(
            col,
            "Alt-drop swap onto an instance",
            match drop_at {
                Some(b) => timed(&mut s, "instance.swapAt", json!({"component": other, "x": b.center().x, "y": b.center().y}), true),
                None => f64::NAN,
            },
        );
    } else {
        table.put(col, "push a button's changes to its main", f64::NAN);
        table.put(col, "Alt-drop swap onto an instance", f64::NAN);
    }
    if let Some(nav) = nav {
        exec(&mut s, "edit.copy", json!({"ids": [nav]}));
        table.put(col, "copy + paste a nav", timed(&mut s, "edit.paste", json!({}), true));
    } else {
        table.put(col, "copy + paste a nav", f64::NAN);
    }
    table.put(col, "undo (deep main edit)", mean(10, |_| _ = s.undo()));
    table.put(col, "redo", mean(10, |_| _ = s.redo()));

    // As a library: publish it, use it from another file, publish a change, accept the update.
    // One directory per run, so concurrent runs in a process don't delete each other's files.
    static RUNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let run = RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("figcraft-perf-{}-{run}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(failed)?;
    let path = dir.join("design-system.figcraft").to_string_lossy().into_owned();
    let (lib_doc, lib_ds) = design_system_document(&spec).map_err(failed)?;
    let mut lib_s = Session::with_document(lib_doc);
    table.put(col, "publish the library", timed(&mut lib_s, "library.publish", json!({}), false));
    exec(&mut lib_s, "file.saveAs", json!({"path": path}));
    let lib = figcraft_engine::library::Cache::default().load(&path).map_err(failed)?;
    let mut doc = Document::new("Consumer");
    let m = &lib.manifest;
    doc.libraries_mut().enabled.push(figcraft_doc::LibraryLink { id: m.id.clone(), name: m.name.clone(), path: path.clone(), version: m.version });
    let map = figcraft_engine::library::import(&mut doc, &lib, &lib_ds.assets(), false).map_err(failed)?;
    let mut local = lib_ds.remap(&map);
    local.theme = doc.libraries.collections.iter().find(|r| r.key == lib_ds.theme).map(|r| r.local.clone()).unwrap_or_default();
    figcraft_engine::normalize(&mut doc, &Document::new(""));
    place_instances(&mut doc, &local, instances).map_err(failed)?;
    figcraft_engine::normalize(&mut doc, &Document::new(""));
    let mut consumer = Session::with_document(doc);
    let lglyph = lib_s.doc().map_err(failed)?.node(icon0).and_then(|n| n.children.first().copied()).ok_or_else(|| failed("no glyph"))?;
    let label = lib_s
        .doc()
        .map_err(failed)?
        .node(lib_ds.button)
        .and_then(|n| n.children.first().copied())
        .and_then(|v| find_path(lib_s.doc().ok()?, v, &["Label"]))
        .ok_or_else(|| failed("no label"))?;
    exec(&mut lib_s, "node.set", json!({"ids": [lglyph], "props": {"cornerRadius": 4}}));
    exec(&mut lib_s, "node.set", json!({"ids": [label], "props": {"fontSize": 16}}));
    exec(&mut lib_s, "library.publish", json!({}));
    exec(&mut lib_s, "file.save", json!({}));
    table.put(col, "check library updates", timed(&mut consumer, "library.updates", json!({}), false));
    table.put(col, "accept library updates (overrides kept)", timed(&mut consumer, "library.acceptUpdates", json!({}), false));
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn measures_components_at_a_small_scale() {
        let mut t = super::Table { rows: Vec::new() };
        super::measure_components(&mut t, 0, 30).unwrap();
        assert!(!super::FAILED.load(std::sync::atomic::Ordering::Relaxed));
        assert!(t.rows.len() > 15);
        assert!(t.rows.iter().all(|r| r.1.len() == 1 && r.1[0].is_finite()), "{:?}", t.rows);
    }

    #[test]
    fn measures_components_with_a_single_instance() {
        let mut t = super::Table { rows: Vec::new() };
        super::measure_components(&mut t, 0, 1).unwrap();
        assert!(t.rows.iter().all(|r| r.1.len() == 1), "{:?}", t.rows);
        assert!(t.rows.iter().any(|r| r.0.starts_with("build design system") && r.1[0].is_finite()), "{:?}", t.rows);
    }

    #[test]
    fn measures_a_small_document() {
        let mut t = super::Table { rows: Vec::new() };
        super::measure(&mut t, 0, 300).unwrap();
        assert!(t.rows.len() > 10);
        assert!(t.rows.iter().all(|r| r.1.len() == 1 && r.1[0].is_finite()), "{:?}", t.rows);
    }
}
