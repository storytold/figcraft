//! `.sketch` import: detection, the archive, the version cut and pages, on fixtures built in code.

use serde_json::json;

use super::fixture::*;
use super::*;

fn page_names(doc: &Document) -> Vec<String> {
    doc.pages.iter().map(|p| doc.node(*p).unwrap().name.clone()).collect()
}

#[test]
fn detects_sketch_and_not_others() {
    let z = sketch_zip("50.2", 103, document(&["P1"]), &[page("P1", "Page 1", vec![])], &[]);
    assert!(is_sketch(&z));
    assert!(!is_sketch(b"fig-kiwi...."));
    assert!(!is_sketch(b"{}"));
    assert!(!is_sketch(b""));
    // A zip that isn't a Sketch file.
    let mut other = Vec::new();
    {
        use std::io::Write;
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut other));
        w.start_file("canvas.fig", zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(b"x").unwrap();
        w.finish().unwrap();
    }
    assert!(!is_sketch(&other));
}

#[test]
fn pages_become_pages_in_document_order() {
    let z = sketch_zip("50.2", 103, document(&["B", "A"]), &[page("A", "Alpha", vec![]), page("B", "Beta", vec![])], &[]);
    let (doc, _) = import(&z, "f").unwrap();
    assert_eq!(page_names(&doc), ["Beta", "Alpha"]);
}

#[test]
fn older_than_48_is_refused() {
    let z = sketch_zip("47.1", 95, document(&["P"]), &[page("P", "P", vec![])], &[]);
    let e = import(&z, "f").unwrap_err().to_string();
    assert!(e.contains(TOO_OLD), "{e}");
    let mut sqlite = b"SQLite format 3\0".to_vec();
    sqlite.resize(512, 0);
    assert!(is_sketch(&sqlite));
    assert!(import(&sqlite, "f").unwrap_err().to_string().contains(TOO_OLD));
}

#[test]
fn archived_text_without_meta_version_is_refused() {
    let text = json!({
        "_class": "text", "do_objectID": "T", "name": "T",
        "frame": {"_class": "rect", "x": 0, "y": 0, "width": 10, "height": 10},
        "attributedString": {"_class": "MSAttributedString", "archivedAttributedString": {"_archive": "YnBsaXN0MDA="}},
    });
    let z = raw_zip(&document(&["P"]), &json!({"app": "com.bohemiancoding.sketch3"}), &[page("P", "P", vec![text])], &[]);
    let e = import(&z, "f").unwrap_err().to_string();
    assert!(e.contains(TOO_OLD), "{e}");
}

#[test]
fn newer_than_tested_imports_with_one_warning() {
    let z = sketch_zip("2099.1", NEWEST_TESTED_VERSION + 1, document(&["P"]), &[page("P", "P", vec![])], &[]);
    let (_, warnings) = import(&z, "f").unwrap();
    assert_eq!(warnings.iter().filter(|w| w.contains("newer Sketch")).count(), 1, "{warnings:?}");
}

#[test]
fn junk_never_panics() {
    let z = sketch_zip("50.2", 103, document(&["P"]), &[page("P", "P", vec![rect("R", 0.0, 0.0, 10.0, 10.0)])], &[]);
    for cut in (0..z.len()).step_by(7) {
        let _ = import(&z[..cut], "f");
    }
    let mut seed = 0x2545_f491_u32;
    for _ in 0..200 {
        let mut b = z.clone();
        for _ in 0..8 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let i = seed as usize % b.len();
            b[i] = (seed >> 8) as u8;
        }
        let _ = import(&b, "f");
    }
    for doc in [json!([]), json!(null), json!({"pages": 5}), json!({"pages": [{"_ref": 7}, {"_ref": "pages/../../x"}]})] {
        let _ = import(&sketch_zip("50.2", 103, doc, &[json!(5), json!({"do_objectID": "Q", "layers": "x"})], &[]), "f");
    }
    let _ = import(&raw_zip(&document(&["P"]), &json!("x"), &[], &[]), "f");
}

/// A file using most of what the importer reads (Sketch 2025 frames and stacks, tints, symbols
/// with overrides, text, paths, gradients, masks, fades, grids) with each of its values in turn
/// replaced by a value of another type: every import ends without a panic.
#[test]
fn junk_values_never_panic() {
    let text_layer = text("T", 0.0, 0.0, 80.0, 20.0, "Go", &[(0, 2, "Helvetica", 14.0, [0.0, 0.0, 0.0, 1.0])]);
    let dot = styled(rect("Dot", 0.0, 0.0, 10.0, 10.0), "fills", json!([solid_fill(0.0, 0.0, 0.0, 1.0)]));
    let mut tint = solid_fill(1.0, 0.0, 0.0, 1.0);
    tint["layeringType"] = json!(1);
    let mut icon = master("MI", "SI", "Icon", 24.0, 24.0, vec![dot.clone()]);
    icon["groupBehavior"] = json!(2);
    icon["style"]["fills"] = json!([tint]);
    let mut chip = stack(
        master("MC", "SC", "Chip", 60.0, 24.0, vec![text_layer.clone(), instance("N", "SI", 0.0, 0.0, 24.0, 24.0, &[])]),
        0,
        1,
        1,
        6.0,
        [8.0, 4.0, 10.0, 4.0],
    );
    chip["style"]["corners"] = json!({"_class": "MSImmutableStyleCorners", "radii": [12], "style": 2, "smoothing": 0.6});
    let over = [
        ("T_stringValue", json!("Longer")),
        ("N_fillColor", color(0.0, 1.0, 0.0, 1.0)),
        ("T_textColor", color(0.0, 0.0, 1.0, 1.0)),
        ("N_isVisible", json!("0")),
    ];
    let inst = instance("I", "SC", 0.0, 100.0, 200.0, 24.0, &over);
    let grad = json!({"_class": "fill", "isEnabled": true, "fillType": 1, "color": color(0.0, 0.0, 0.0, 1.0),
        "gradient": {"_class": "gradient", "gradientType": 0, "from": "{0, 0}", "to": "{1, 1}", "stops": [
            {"_class": "gradientStop", "position": 0, "color": color(0.0, 0.0, 0.0, 1.0)}, {"_class": "gradientStop", "position": 1, "color": color(1.0, 1.0, 1.0, 0.0)}]}});
    let mut faded = styled(rect("Faded", 0.0, 0.0, 30.0, 30.0), "fills", json!([grad.clone()]));
    faded["style"]["contextSettings"] =
        json!({"_class": "graphicsContextSettings", "opacity": 1, "isProgressive": true, "gradient": grad["gradient"].clone()});
    let mask =
        with(with(styled(rect("Mask", 0.0, 0.0, 30.0, 30.0), "fills", json!([grad])), "hasClippingMask", json!(true)), "clippingMaskMode", json!(1));
    let path = shape_path("P", 0.0, 0.0, 20.0, 20.0, &[(0.0, 0.0), (1.0, 0.0), (0.5, 1.0)], true);
    let mut g = group("G", 0.0, 0.0, 100.0, 100.0, vec![mask, faded, path, dot, text_layer]);
    g["style"] = json!({"_class": "style", "fills": [tint_fill()]});
    let mut frame = stack(frame_group("F", 0.0, 0.0, 300.0, 300.0, vec![g, inst]), 1, 0, 3, 8.0, [0.0; 4]);
    frame["layout"] =
        json!({"_class": "layoutGrid", "isEnabled": true, "columnWidth": 20, "gutterWidth": 10, "numberOfColumns": 4, "drawVertical": true});
    let pages = [page("P", "Page", vec![frame]), page("S", "Symbols", vec![icon, chip])];
    // Every value's path, with each junk value in turn (a deterministic sample keeps it quick).
    fn paths(v: &serde_json::Value, at: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
        out.push(at.clone());
        match v {
            serde_json::Value::Object(m) => {
                for (k, c) in m {
                    at.push(k.clone());
                    paths(c, at, out);
                    at.pop();
                }
            }
            serde_json::Value::Array(a) => {
                for (i, c) in a.iter().enumerate() {
                    at.push(i.to_string());
                    paths(c, at, out);
                    at.pop();
                }
            }
            _ => {}
        }
    }
    let junk = [json!(null), json!(5), json!(-1e300), json!("x"), json!(true), json!([]), json!({}), json!([1, "a"])];
    for (pi, p) in pages.iter().enumerate() {
        let mut all = Vec::new();
        paths(p, &mut Vec::new(), &mut all);
        for (n, at) in all.iter().enumerate().skip(1) {
            let mut changed = pages.clone();
            let mut v = &mut changed[pi];
            for k in at {
                v = match v {
                    serde_json::Value::Object(m) => m.get_mut(k).unwrap(),
                    serde_json::Value::Array(a) => &mut a[k.parse::<usize>().unwrap()],
                    _ => panic!("a path only runs through objects and arrays"),
                };
            }
            *v = junk[n % junk.len()].clone();
            let _ = import(&sketch_zip("2026.3.1", 196, document(&["P", "S"]), &changed, &[]), "f");
        }
    }
}

fn tint_fill() -> serde_json::Value {
    let mut f = solid_fill(0.0, 0.0, 1.0, 1.0);
    f["layeringType"] = json!(1);
    f
}

#[test]
fn deep_nesting_does_not_fail_the_file() {
    let mut deep = rect("R", 0.0, 0.0, 1.0, 1.0);
    for i in 0..300 {
        deep = group(&format!("G{i}"), 0.0, 0.0, 1.0, 1.0, vec![deep]);
    }
    let z = sketch_zip("50.2", 103, document(&["A", "B"]), &[page("A", "Deep", vec![deep]), page("B", "Normal", vec![])], &[]);
    let (doc, warnings) = import(&z, "f").unwrap();
    assert!(page_names(&doc).contains(&"Normal".to_string()));
    assert!(!warnings.is_empty());
}

#[test]
fn members_over_budget_are_skipped() {
    let z = sketch_zip("50.2", 103, document(&["A", "B"]), &[page("A", "Big", vec![]), page("B", "Small", vec![])], &[]);
    let (doc, warnings) = import_within(&z, "f", 200).unwrap();
    // Each page JSON is larger than 200 bytes: both are skipped, with a warning; nothing panics.
    assert!(warnings.iter().any(|w| w.contains("too large")), "{warnings:?}");
    assert!(!page_names(&doc).contains(&"Big".to_string()));
}
