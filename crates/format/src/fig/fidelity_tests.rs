//! The fidelity harness: the pixel and box metrics keep the properties the scorecards rely on, a
//! built-in corpus of `.fig` documents survives an export and import with its structure intact,
//! and (ignored, local only) a folder of real `.fig` files is measured the same way.

use figcraft_doc::Document;
use figcraft_doc::geom::Rect;

use super::fidelity::{MAX_CHANNEL_SSD, PixelDiffError, StructuralReport, bbox_linf, compare_documents, compare_rgba, diff_count, ssd};
use super::{export, import, map};

/// Deterministic pseudo-random bytes.
fn bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut r = super::tests::Lcg(seed);
    (0..len).map(|_| r.next() as u8).collect()
}

#[test]
fn ssd_is_zero_exactly_when_the_buffers_are_equal() {
    for seed in 0..64 {
        let a = bytes(seed, 4 * 37);
        assert_eq!(ssd(&a, &a), Some(0));
        let mut b = a.clone();
        let i = (seed as usize * 13) % b.len();
        b[i] = b[i].wrapping_add(1 + seed as u8 % 255);
        assert!(ssd(&a, &b).unwrap() > 0, "one channel changed");
    }
}

#[test]
fn ssd_and_diff_count_are_symmetric_and_bounded() {
    for seed in 0..64 {
        let (a, b) = (bytes(seed, 4 * 29), bytes(seed + 1000, 4 * 29));
        assert_eq!(ssd(&a, &b), ssd(&b, &a));
        assert!(ssd(&a, &b).unwrap() <= a.len() as u64 * MAX_CHANNEL_SSD);
        for tol in [0, 1, 16, 254, 255] {
            assert_eq!(diff_count(&a, &b, tol), diff_count(&b, &a, tol));
            assert!(diff_count(&a, &b, tol).unwrap() <= a.len() as u64);
        }
        assert_eq!(diff_count(&a, &b, 255), Some(0), "no channel differs by more than 255");
    }
    // The bound is reached: black against white everywhere.
    let (black, white) = (vec![0u8; 40], vec![255u8; 40]);
    assert_eq!(ssd(&black, &white), Some(40 * MAX_CHANNEL_SSD));
}

#[test]
fn buffers_of_different_lengths_are_never_reported_equal() {
    let a = vec![7u8; 16];
    assert_eq!(ssd(&a, &a[..12]), None);
    assert_eq!(diff_count(&a, &a[..12], 0), None);
    assert!(matches!(compare_rgba(&a, &a[..12], 2, 2, 0), Err(PixelDiffError::WrongLength { .. })));
    assert!(matches!(compare_rgba(&a, &a, 3, 2, 0), Err(PixelDiffError::WrongLength { .. })));
    assert_eq!(compare_rgba(&[], &[], u32::MAX, u32::MAX, 0).err(), Some(PixelDiffError::TooLarge));
}

#[test]
fn bbox_linf_is_symmetric_and_zero_only_for_equal_boxes() {
    let mut r = super::tests::Lcg(5);
    let mut rect = || {
        let mut v = || (r.next() % 2000) as f64 / 4.0 - 250.0;
        let (x, y) = (v(), v());
        Rect::new(x, y, x + v().abs(), y + v().abs())
    };
    for _ in 0..200 {
        let (a, b) = (rect(), rect());
        assert_eq!(bbox_linf(a, b), bbox_linf(b, a));
        assert_eq!(bbox_linf(a, a), 0.0);
        assert_eq!(bbox_linf(a, b) == 0.0, a == b);
    }
    let a = Rect::new(0.0, 0.0, 10.0, 10.0);
    assert_eq!(bbox_linf(a, Rect::new(0.0, 0.0, 10.0, 13.5)), 3.5, "the largest edge difference");
    assert_eq!(bbox_linf(a, Rect::new(f64::NAN, 0.0, 10.0, 10.0)), f64::INFINITY, "a broken box never matches");
}

#[test]
fn identical_images_score_perfectly_and_different_ones_do_not() {
    let (w, h) = (19, 11);
    let a = bytes(3, w * h * 4);
    let same = compare_rgba(&a, &a, w as u32, h as u32, 0).unwrap();
    assert_eq!((same.ssd, same.diff_count, same.rmse, same.ssim), (0, 0, 0.0, 1.0));
    assert_eq!(same.psnr_db, f64::INFINITY);
    assert_eq!(same.to_json()["psnrDb"], "inf", "JSON can't hold infinity");
    let b = bytes(4, w * h * 4);
    let other = compare_rgba(&a, &b, w as u32, h as u32, 0).unwrap();
    assert!(other.ssd > 0 && other.psnr_db.is_finite() && other.diff_pct > 0.0);
    assert!((0.0..1.0).contains(&other.ssim), "{}", other.ssim);
    // A small shift scores between the two.
    let nudged: Vec<u8> = a.iter().map(|c| c.saturating_add(3)).collect();
    let near = compare_rgba(&a, &nudged, w as u32, h as u32, 2).unwrap();
    assert!(near.ssim > other.ssim && near.ssim < 1.0 && near.psnr_db > other.psnr_db);
    assert!(compare_rgba(&a, &nudged, w as u32, h as u32, 3).unwrap().diff_count < near.diff_count, "tol excludes the shift");
}

#[test]
fn flat_and_empty_images_have_a_defined_ssim() {
    let white = vec![255u8; 16 * 16 * 4];
    let black: Vec<u8> = (0..16 * 16).flat_map(|_| [0, 0, 0, 255]).collect();
    assert_eq!(compare_rgba(&white, &white, 16, 16, 0).unwrap().ssim, 1.0);
    let flat = compare_rgba(&white, &black, 16, 16, 0).unwrap().ssim;
    assert!(flat.is_finite() && (0.0..0.01).contains(&flat), "{flat}");
    let empty = compare_rgba(&[], &[], 0, 0, 0).unwrap();
    assert_eq!((empty.ssd, empty.ssim, empty.diff_pct), (0, 1.0, 0.0));
    // Transparent pixels read as white, whatever colour they keep.
    let clear: Vec<u8> = (0..16 * 16).flat_map(|_| [10, 200, 30, 0]).collect();
    assert_eq!(compare_rgba(&white, &clear, 16, 16, 0).unwrap().ssim, 1.0);
}

#[test]
fn reports_print_the_same_way_every_time() {
    let (a, b) = (bytes(1, 8 * 8 * 4), bytes(2, 8 * 8 * 4));
    let r = compare_rgba(&a, &b, 8, 8, 4).unwrap();
    assert_eq!(r.to_json().to_string(), compare_rgba(&a, &b, 8, 8, 4).unwrap().to_json().to_string());
    assert!(r.to_markdown().starts_with("| Size |"));
    let (doc, _) = import(&super::tests::fixture(), "Fixture").unwrap();
    let rows = vec![("x|y".to_string(), compare_documents(&doc, &doc))];
    let table = StructuralReport::markdown_table(&rows);
    assert_eq!(table, StructuralReport::markdown_table(&rows));
    assert!(table.contains("x\\|y"), "a name can't break the table: {table}");
}

#[test]
fn a_document_compared_with_itself_is_exact_and_a_moved_layer_is_not() {
    let (doc, _) = import(&super::tests::fixture(), "Fixture").unwrap();
    let same = compare_documents(&doc, &doc);
    assert!(same.is_exact(0.0), "{:?}", same.to_json());
    assert!(same.matched_layers > 0);
    let mut moved = doc.clone();
    let layer = moved.nodes().find(|n| !n.is_page() && n.parent.is_some_and(|p| doc.pages.contains(&p))).unwrap().id;
    moved.get_mut(layer).unwrap().transform = moved.node(layer).unwrap().transform.then_translate((3.0, -1.0).into());
    let report = compare_documents(&doc, &moved);
    assert_eq!(report.bbox_max_linf, 3.0, "{:?}", report.to_json());
    assert!(report.bbox_mean_linf > 0.0 && !report.is_exact(1e-6));
    assert_eq!(report.worst[0].linf, 3.0);
    let mut renamed = doc.clone();
    let text = renamed.nodes().find(|n| matches!(n.kind, figcraft_doc::NodeKind::Text(_))).unwrap().id;
    if let figcraft_doc::NodeKind::Text(t) = &mut renamed.get_mut(text).unwrap().kind {
        t.characters.push('!');
    }
    assert!(compare_documents(&doc, &renamed).text_match_rate() < 1.0);
}

/// A corpus document as first imported, with its import warnings.
struct Sample {
    name: &'static str,
    doc: Document,
    warnings: Vec<String>,
}

/// Each feature the import and export carry: components, instances, auto layout, styles,
/// variables and text runs (the fixture); vectors with regions, sticky notes, shapes with text,
/// connectors routed here (with and without a label) and a table with its cells (the tail); a
/// connector with its saved snapshot; and generated documents with effects, gradients, images,
/// vector networks, grids and nested auto layout.
fn corpus() -> Vec<Sample> {
    let fig = |name, bytes: Vec<u8>| {
        let (doc, warnings) = import(&bytes, name).unwrap();
        Sample { name, doc, warnings }
    };
    let (json, mut warnings, networks) =
        map::to_rest(&super::connector_label_tests::synthetic_connector(1.0, true, 8.5), "Snapshot", crate::rest::MAX_NODES);
    let (snapshot, more) = crate::rest::import_with_networks(&json, &crate::rest::no_images, networks, crate::rest::MAX_NODES).unwrap();
    warnings.extend(more);
    let mut samples = vec![
        fig("components, auto layout, text", super::tests::fixture()),
        fig("vectors, connectors, table", super::tests::tail::tail_canvas()),
        Sample { name: "connector snapshot", doc: snapshot, warnings },
        fig("labelled connector, no snapshot", labelled_connector()),
    ];
    for (seed, name) in [(1, "generated 1"), (2, "generated 2"), (3, "generated 3")] {
        samples.push(Sample { name, doc: super::export_tests::generate(seed), warnings: Vec::new() });
    }
    samples
}

/// Two boxes joined by a connector with a label and no saved snapshot (its label gets a pill).
fn labelled_connector() -> Vec<u8> {
    use super::tests::{arr, at, canvas, e, f, g, node, o, schema, solid, xy};
    let box_at = |id, name: &str, x| node(id, Some((1, "*")), "RECTANGLE", name, vec![("size", xy(100.0, 100.0)), ("transform", at(x, 0.0))]);
    let side = |id, x| o(vec![("endpointNodeID", g(id)), ("relativePosition", xy(x, 0.5))]);
    let message = o(vec![(
        "nodeChanges",
        arr(vec![
            node(0, None, "DOCUMENT", "Document", vec![]),
            node(1, Some((0, "!")), "CANVAS", "Page", vec![]),
            box_at(2, "A", 0.0),
            box_at(3, "B", 300.0),
            node(
                4,
                Some((1, "+")),
                "CONNECTOR",
                "Says",
                vec![
                    ("strokePaints", arr(vec![solid(0.0, 0.0, 0.0)])),
                    ("connectorStart", side(2, 1.0)),
                    ("connectorEnd", side(3, 0.0)),
                    ("connectorLineStyle", e("STRAIGHT")),
                    ("textData", o(vec![("characters", super::tests::s("yes"))])),
                    ("fontSize", f(20.0)),
                ],
            ),
        ]),
    )]);
    canvas(&schema(), &message)
}

/// Export `doc` as `.fig` and import it again.
fn round_trip(doc: &Document) -> (Document, Vec<String>) {
    let (bytes, _) = export(doc, None).unwrap();
    import(&bytes, &doc.name).unwrap()
}

/// Every corpus document survives a `.fig` round trip with its structure intact: every layer
/// matched, in place to a thousandth of a pixel, with its text, paints and opacity; and the import
/// of the written file warns about nothing the first import didn't (a fresh warning is a
/// regression in the export or the import).
#[test]
fn the_built_in_corpus_round_trips_with_exact_structure() {
    let mut rows = Vec::new();
    for sample in corpus() {
        let (back, warnings) = round_trip(&sample.doc);
        let report = compare_documents(&sample.doc, &back).with_warnings(warnings.len());
        let fresh: Vec<&String> = warnings.iter().filter(|w| !sample.warnings.iter().any(|seen| warning_kind(seen) == warning_kind(w))).collect();
        assert!(fresh.is_empty(), "{}: new warnings after the round trip {fresh:?}", sample.name);
        assert!(report.is_exact(1e-3), "{}: {}", sample.name, report.to_json());
        rows.push((sample.name.to_string(), report));
    }
    println!("{}", StructuralReport::markdown_table(&rows));
}

/// A warning without its count, so `x (×3)` and `x (×2)` are the same kind.
fn warning_kind(w: &str) -> &str {
    w.rsplit_once(" (×").map_or(w, |(base, _)| base)
}

/// Measure every `.fig` file in `FIGCRAFT_FIG_CORPUS` (a local folder; the files stay out of the
/// repository): import it, export and import it again, and print each file's structural report as
/// JSON lines and the whole run as a Markdown scorecard. Run with
/// `FIGCRAFT_FIG_CORPUS=<dir> cargo test -p figcraft-format real_fig_corpus -- --ignored --nocapture`.
#[test]
#[ignore = "reads a local folder of .fig files named by FIGCRAFT_FIG_CORPUS"]
fn real_fig_corpus() {
    let Some(dir) = std::env::var_os("FIGCRAFT_FIG_CORPUS") else {
        println!("FIGCRAFT_FIG_CORPUS is not set; nothing to measure");
        return;
    };
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("fig")))
        .collect();
    paths.sort();
    let mut rows = Vec::new();
    for path in paths {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let bytes = std::fs::read(&path).unwrap();
        let (doc, first) = match import(&bytes, &name) {
            Ok(r) => r,
            Err(e) => {
                println!("{}", serde_json::json!({"file": name, "error": e.to_string()}));
                continue;
            }
        };
        let back = export(&doc, None).and_then(|(written, _)| import(&written, &name));
        let (back, warnings) = match back {
            Ok(r) => r,
            Err(e) => {
                println!("{}", serde_json::json!({"file": name, "importWarnings": first.len(), "error": e.to_string()}));
                continue;
            }
        };
        let report = compare_documents(&doc, &back).with_warnings(first.len());
        let mut line = report.to_json();
        line["file"] = serde_json::json!(name);
        line["roundTripWarnings"] = serde_json::json!(warnings.len());
        println!("{line}");
        rows.push((name, report));
    }
    println!("\n{}", StructuralReport::markdown_table(&rows));
}
