//! `.sketch` files through the file commands. The importer's own tests (fixtures built in code)
//! are in `figcraft_format::sketch`.

use std::io::Write;

use base64::Engine;
use serde_json::{Value, json};

use figcraft_doc::NodeKind;

use crate::Session;

/// A minimal `.sketch` zip with one page per `(id, name)`, saved by `app_version`.
fn sketch(app_version: &str, pages: &[(&str, &str)]) -> Vec<u8> {
    sketch_with(app_version, pages, &[])
}

/// [`sketch`] with `layers` on the first page.
fn sketch_with(app_version: &str, pages: &[(&str, &str)], layers: &[Value]) -> Vec<u8> {
    sketch_with_doc(app_version, pages, layers, json!({}))
}

/// [`sketch_with`] with the fields of `extra` added to `document.json` (shared styles…).
fn sketch_with_doc(app_version: &str, pages: &[(&str, &str)], layers: &[Value], extra: Value) -> Vec<u8> {
    let refs: Vec<Value> = pages.iter().map(|(id, _)| json!({"_class": "MSJSONFileReference", "_ref": format!("pages/{id}")})).collect();
    let mut document = json!({"_class": "document", "pages": refs});
    for (k, v) in extra.as_object().into_iter().flatten() {
        document[k] = v.clone();
    }
    let mut out = Vec::new();
    {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("document.json", o).unwrap();
        z.write_all(document.to_string().as_bytes()).unwrap();
        z.start_file("meta.json", o).unwrap();
        // The format version that Sketch writes (103 for 50; 110 and up from 52).
        let version = if app_version.split('.').next().and_then(|v| v.parse::<u32>().ok()).is_some_and(|v| v >= 52) { 125 } else { 103 };
        z.write_all(json!({"app": "com.bohemiancoding.sketch3", "appVersion": app_version, "version": version}).to_string().as_bytes()).unwrap();
        for (i, (id, name)) in pages.iter().enumerate() {
            let ls = if i == 0 { layers.to_vec() } else { Vec::new() };
            z.start_file(format!("pages/{id}.json"), o).unwrap();
            z.write_all(json!({"_class": "page", "do_objectID": id, "name": name, "layers": ls}).to_string().as_bytes()).unwrap();
        }
        z.finish().unwrap();
    }
    out
}

fn temp(name: &str, bytes: &[u8]) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("figcraft-sketch-test-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    (dir, path)
}

fn page_names(s: &Session) -> Vec<String> {
    let doc = s.doc().unwrap();
    doc.pages.iter().map(|p| doc.node(*p).unwrap().name.clone()).collect()
}

#[test]
fn opened_sketch_files_have_their_pages_and_no_path() {
    let (dir, path) = temp("design.sketch", &sketch("50.2", &[("A", "Screens"), ("B", "Symbols")]));
    let mut s = Session::new();
    let r = s.execute("file.open", &json!({"path": path.to_string_lossy()})).unwrap();
    assert!(r.get("summary").is_some());
    assert_eq!(page_names(&s), ["Screens", "Symbols"]);
    // Never saved over: the document has no path, so Save asks for one.
    assert!(s.docs[s.active].path.is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sketch_import_appends_pages() {
    let (dir, path) = temp("more.sketch", &sketch("83.2", &[("A", "Imported")]));
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let before = page_names(&s).len();
    s.execute("file.import", &json!({"path": path.to_string_lossy()})).unwrap();
    let names = page_names(&s);
    assert_eq!(names.len(), before + 1);
    assert_eq!(names.last().unwrap(), "Imported");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sketch_data_opens_with_a_summary() {
    let data = base64::engine::general_purpose::STANDARD.encode(sketch("50.2", &[("A", "Page")]));
    let mut s = Session::new();
    let r = s.execute("file.open", &json!({"data": data, "name": "x.sketch"})).unwrap();
    assert!(r.get("summary").is_some());
    assert_eq!(page_names(&s), ["Page"]);
}

#[test]
fn old_and_damaged_sketch_files_fail_cleanly() {
    let mut s = Session::new();
    let (dir, path) = temp("old.sketch", &sketch("47.1", &[("A", "P")]));
    let e = s.execute("file.open", &json!({"path": path.to_string_lossy()})).unwrap_err().to_string();
    assert!(e.contains("older than 48"), "{e}");
    let mut broken = sketch("50.2", &[("A", "P")]);
    broken.truncate(broken.len() / 2);
    std::fs::write(&path, &broken).unwrap();
    assert!(s.execute("file.open", &json!({"path": path.to_string_lossy()})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

fn shape(class: &str, name: &str, frame: [f64; 4], extra: Value) -> Value {
    let mut l = json!({"_class": class, "do_objectID": name, "name": name, "isVisible": true, "resizingConstraint": 63,
        "frame": {"x": frame[0], "y": frame[1], "width": frame[2], "height": frame[3]}, "style": {"fills": []}});
    for (k, v) in extra.as_object().unwrap() {
        l[k] = v.clone();
    }
    l
}

fn fill(grey: f64) -> Value {
    json!({"style": {"fills": [{"isEnabled": true, "fillType": 0, "color": {"red": grey, "green": grey, "blue": grey, "alpha": 1}}]}})
}

/// Sketch draws a mask's own fill under what it clips; the clipped layers show inside it.
#[test]
fn sketch_masks_show_their_content() {
    let oval = shape("oval", "Ring", [0.0, 0.0, 60.0, 60.0], json!({"booleanOperation": -1, "points": []}));
    let mut mask = shape("shapeGroup", "Circle", [0.0, 0.0, 60.0, 60.0], fill(0.8));
    mask["hasClippingMask"] = json!(true);
    mask["layers"] = json!([oval]);
    let pts = ["{0, 0}", "{1, 0}", "{1, 1}", "{0, 1}"].map(|p| json!({"point": p, "curveFrom": p, "curveTo": p, "curveMode": 1}));
    let body = shape("shapePath", "Square", [0.0, 0.0, 20.0, 20.0], json!({"points": pts, "isClosed": true, "booleanOperation": -1}));
    let mut content = shape("shapeGroup", "Body", [20.0, 20.0, 20.0, 20.0], fill(0.2));
    content["layers"] = json!([body]);
    let icon = shape("group", "Icon", [0.0, 0.0, 60.0, 60.0], json!({"layers": [mask, content]}));
    let data = base64::engine::general_purpose::STANDARD.encode(sketch_with("50.2", &[("A", "Page")], &[icon]));
    let mut s = Session::new();
    s.execute("file.open", &json!({"data": data, "name": "x.sketch"})).unwrap();
    let doc = s.doc().unwrap();
    let icon = doc.nodes().find(|n| n.name == "Icon").unwrap().id;
    let pix = figcraft_render::render_node(doc, icon, 1.0).unwrap();
    let px = |x: u32, y: u32| pix.pixel(x, y).unwrap();
    // The square in the middle, dark grey.
    let c = px(30, 30);
    assert!(c.alpha() > 200 && c.red() < 100, "content: {c:?}");
    // The mask's own fill around it, light grey; outside the circle nothing.
    let m = px(30, 8);
    assert!(m.alpha() > 200 && m.red() > 150, "mask fill: {m:?}");
    assert!(px(2, 2).alpha() < 50);
}

/// Family names guessed from PostScript names find the installed family whatever its spaces.
#[test]
fn sketch_font_families_match_installed_ones() {
    let installed = ["PingFang SC".to_string(), "Helvetica Neue".to_string(), "IBM Plex Sans".to_string()];
    let m = |f: &str| crate::cmd::file::installed_family(f, &installed);
    assert_eq!(m("Ping Fang SC").as_deref(), Some("PingFang SC"));
    assert_eq!(m("Helvetica Neue"), None, "already right");
    assert_eq!(m("IBMPlex Sans").as_deref(), Some("IBM Plex Sans"));
    assert_eq!(m("Comic Sans"), None);
}

/// Editing a master changes its instances, and a resized instance's layout isn't an override.
#[test]
fn sketch_instances_stay_live() {
    let tag = shape("rectangle", "Tag", [80.0, 0.0, 20.0, 40.0], json!({"resizingConstraint": 60, "points": []}));
    let master = shape("symbolMaster", "Bar", [0.0, 0.0, 100.0, 40.0], json!({"symbolID": "S", "layers": [tag]}));
    let inst = shape("symbolInstance", "Wide", [0.0, 100.0, 200.0, 40.0], json!({"symbolID": "S", "overrideValues": []}));
    let data = base64::engine::general_purpose::STANDARD.encode(sketch_with("64", &[("A", "Page")], &[master, inst]));
    let mut s = Session::new();
    s.execute("file.open", &json!({"data": data, "name": "x.sketch"})).unwrap();
    let find = |s: &Session, inst: bool| {
        let doc = s.doc().unwrap();
        let root = doc
            .nodes()
            .find(|n| if inst { matches!(n.kind, NodeKind::Instance { .. }) } else { matches!(n.kind, NodeKind::Component { .. }) })
            .unwrap()
            .id;
        doc.subtree(root).into_iter().find(|id| doc.node(*id).is_some_and(|n| n.name == "Tag")).unwrap()
    };
    let (main_tag, inst_tag) = (find(&s, false), find(&s, true));
    assert!(s.doc().unwrap().node(inst_tag).unwrap().overrides.is_empty(), "{:?}", s.doc().unwrap().node(inst_tag).unwrap().overrides);
    s.execute("node.set", &json!({"id": main_tag, "props": {"fill": "#ff0000"}})).unwrap();
    let doc = s.doc().unwrap();
    let fill = &doc.node(inst_tag).unwrap().fills;
    assert!(fill.iter().any(|p| matches!(p.kind, figcraft_doc::color::PaintKind::Solid { color } if color.r > 0.99 && color.g < 0.01)), "{fill:?}");
    let b = doc.world_bounds(inst_tag).unwrap();
    assert!((b.x0 - 180.0).abs() < 1e-6, "{b:?}");
}

/// Local only (`FIGCRAFT_SKETCH_SAMPLES` names a folder of `.sketch` files): each opens (Sketch 48
/// and newer) with its instances linked and finite geometry, or is refused as too old.
#[test]
fn local_sketch_samples_open() {
    let Ok(dir) = std::env::var("FIGCRAFT_SKETCH_SAMPLES") else { return };
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok().map(|e| e.path())).collect();
    files.retain(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("sketch")));
    files.sort();
    assert!(!files.is_empty(), "no .sketch files in {dir}");
    for path in files {
        let started = std::time::Instant::now();
        let mut s = Session::new();
        match s.execute("file.open", &json!({"path": path.to_string_lossy()})) {
            Err(e) => {
                let e = e.to_string();
                assert!(e.contains("older than 48"), "{}: {e}", path.display());
                eprintln!("{}: refused as too old", path.display());
            }
            Ok(r) => {
                let doc = s.doc().unwrap();
                let (mut layers, mut instances, mut unlinked) = (0, 0, 0);
                for n in doc.nodes() {
                    layers += 1;
                    assert!(
                        n.width.is_finite() && n.height.is_finite() && n.transform.as_coeffs().iter().all(|c| c.is_finite()),
                        "{}: {}",
                        path.display(),
                        n.name
                    );
                    if let NodeKind::Instance { component, .. } = n.kind {
                        instances += 1;
                        if !doc.contains(component) {
                            unlinked += 1;
                        }
                    }
                }
                assert_eq!(unlinked, 0, "{}", path.display());
                let notes = r.get("warnings").and_then(|w| w.as_array()).map_or(0, Vec::len);
                eprintln!("{}: {layers} layers, {instances} instances, {notes} notes, {:?}", path.display(), started.elapsed());
            }
        }
    }
}

/// Local only (`FIGCRAFT_SKETCH_SAMPLES` and `FIGCRAFT_SKETCHTOOL`, the path of Sketch's
/// `sketchtool`): every artboard of each sample (Sketch 48 and newer) drawn by FigCraft is compared
/// with Sketch's own export of it. Prints the share of pixels that differ per artboard (a channel
/// off by more than 40, on white); fails past 10% (a broken artboard, not antialiasing or fonts).
#[test]
fn local_sketch_samples_match_sketch() {
    let (Ok(dir), Ok(tool)) = (std::env::var("FIGCRAFT_SKETCH_SAMPLES"), std::env::var("FIGCRAFT_SKETCHTOOL")) else { return };
    // Text draws in the fonts Sketch uses only once the system fonts are known.
    figcraft_text::fontdb::scan_system_fonts();
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok().map(|e| e.path())).collect();
    files.retain(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("sketch")));
    files.sort();
    let mut worst: Vec<(f64, String)> = Vec::new();
    for path in files {
        let mut s = Session::new();
        if s.execute("file.open", &json!({"path": path.to_string_lossy()})).is_err() {
            continue;
        }
        let out = std::env::temp_dir().join(format!("figcraft-sketch-ref-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        let status = std::process::Command::new(&tool)
            .args(["export", "artboards"])
            .arg(&path)
            .arg(format!("--output={}", out.display()))
            .args(["--formats=png", "--scales=1", "--use-id-for-name=YES", "--include-symbols=NO"])
            .output()
            .unwrap();
        assert!(status.status.success(), "sketchtool failed on {}", path.display());
        // Artboards by page and order, from the file itself.
        let archive = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
        let mut archive = archive;
        let mut read = |name: &str| -> Value {
            let mut s = String::new();
            std::io::Read::read_to_string(&mut archive.by_name(name).unwrap(), &mut s).unwrap();
            serde_json::from_str(&s).unwrap()
        };
        let document = read("document.json");
        let doc = s.doc().unwrap();
        let refs: Vec<String> = document["pages"].as_array().unwrap().iter().map(|r| r["_ref"].as_str().unwrap().to_string()).collect();
        for (page_ref, page_id) in refs.iter().zip(doc.pages.iter()) {
            let page = read(&format!("{page_ref}.json"));
            let boards: Vec<String> = page["layers"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|l| l["_class"] == "artboard")
                .map(|l| l["do_objectID"].as_str().unwrap().to_string())
                .collect();
            let frames: Vec<figcraft_doc::NodeId> =
                doc.node(*page_id).unwrap().children.iter().copied().filter(|c| matches!(doc.node(*c).unwrap().kind, NodeKind::Frame)).collect();
            assert_eq!(boards.len(), frames.len(), "{}: artboards of {page_ref}", path.display());
            for (id, frame) in boards.iter().zip(frames) {
                let Ok(reference) = image::open(out.join(format!("{id}.png"))) else { continue };
                let reference = reference.to_rgba8();
                let ours = figcraft_render::render_node(doc, frame, 1.0).unwrap();
                let (w, h) = (reference.width().min(ours.width()), reference.height().min(ours.height()));
                let ours_rgba = figcraft_render::to_rgba8(&ours);
                // Both on white, straight alpha.
                let on_white = |c: &[u8], i: usize| {
                    let a = f64::from(c[3]) / 255.0;
                    f64::from(c[i]) * a + 255.0 * (1.0 - a)
                };
                let mut bad = 0u64;
                for y in 0..h {
                    for x in 0..w {
                        let r = reference.get_pixel(x, y).0;
                        let at = ((y * ours.width() + x) * 4) as usize;
                        let Some(o) = ours_rgba.get(at..at + 4) else { continue };
                        if (0..3).any(|i| (on_white(&r, i) - on_white(o, i)).abs() > 40.0) {
                            bad += 1;
                        }
                    }
                }
                let share = bad as f64 * 100.0 / (f64::from(w) * f64::from(h)).max(1.0);
                let name = doc.node(frame).unwrap().name.clone();
                eprintln!("{share:6.2}%  {}  {name}", path.file_name().unwrap().to_string_lossy());
                worst.push((share, format!("{}: {name}", path.display())));
            }
        }
        let _ = std::fs::remove_dir_all(&out);
    }
    worst.sort_by(|a, b| b.0.total_cmp(&a.0));
    assert!(worst.first().is_none_or(|w| w.0 < 10.0), "{worst:?}");
}

/// Sketch draws Apple's fonts with the tracking their `trak` table gives at each size (the
/// platform does it on its own); the imported text carries it as letter spacing (macOS only).
#[test]
fn sketch_text_keeps_the_platform_tracking() {
    if !std::path::Path::new("/System/Library/Fonts/SFNS.ttf").exists() {
        return;
    }
    figcraft_text::fontdb::scan_system_fonts();
    let font = json!({"name": "SFUIText-Regular", "size": 13});
    let run = json!({"location": 0, "length": 2, "attributes": {"MSAttributedStringFontAttribute": {"attributes": font}}});
    let t = shape("text", "Label", [0.0, 0.0, 100.0, 20.0], json!({"attributedString": {"string": "Hi", "attributes": [run]}, "textBehaviour": 0}));
    let data = base64::engine::general_purpose::STANDARD.encode(sketch_with("64", &[("A", "Page")], &[t]));
    let mut s = Session::new();
    s.execute("file.open", &json!({"data": data, "name": "x.sketch"})).unwrap();
    let doc = s.doc().unwrap();
    let NodeKind::Text(td) = &doc.nodes().find(|n| n.name == "Label").unwrap().kind else { panic!("not text") };
    let ls = td.style.letter_spacing.resolve(13.0);
    assert!((ls - (-12.0 * 13.0 / 2048.0)).abs() < 1e-6, "{ls}");
    // A named style's optical size follows the size there: the text carries it.
    let f = figcraft_text::fontdb::find_face(&td.style.font_family, &td.style.font_style).unwrap();
    let named = f.axis("opsz").is_some() && f.instance(&td.style.font_style).is_some();
    assert_eq!(td.style.variations.get("opsz").copied(), named.then_some(13.0), "{:?}", td.style);
}

/// A text an override changed is measured again, and the stack around it re-flows: a hugging chip
/// grows to its new label.
#[test]
fn sketch_overridden_text_reflows_its_stack() {
    let font = json!({"name": "Helvetica", "size": 12});
    let run = json!({"location": 0, "length": 2, "attributes": {"MSAttributedStringFontAttribute": {"attributes": font}}});
    let label = shape(
        "text",
        "Label",
        [20.0, 4.0, 14.0, 16.0],
        json!({"attributedString": {"string": "Go", "attributes": [run]}, "horizontalSizing": 1, "verticalSizing": 1}),
    );
    let dot = shape("oval", "Dot", [8.0, 9.0, 6.0, 6.0], json!({"points": []}));
    let layout = json!({"_class": "MSImmutableFlexGroupLayout", "flexDirection": 0, "justifyContent": 0, "alignItems": 1, "allGuttersGap": 6, "stackingOrder": 0});
    let master = shape(
        "symbolMaster",
        "Chip",
        [0.0, 0.0, 44.0, 24.0],
        json!({"symbolID": "S", "groupBehavior": 1, "groupLayout": layout, "horizontalSizing": 1, "verticalSizing": 1,
            "leftPadding": 8, "rightPadding": 10, "topPadding": 4, "bottomPadding": 4, "layers": [label, dot]}),
    );
    let ov = json!([{"_class": "overrideValue", "overrideName": "Label_stringValue", "value": "A much longer label here"}]);
    let inst = shape(
        "symbolInstance",
        "Long",
        [0.0, 100.0, 160.0, 24.0],
        json!({"symbolID": "S", "overrideValues": ov, "horizontalSizing": 1, "verticalSizing": 1}),
    );
    let data = base64::engine::general_purpose::STANDARD.encode(sketch_with("2026.3.1", &[("A", "Page")], &[master, inst]));
    let mut s = Session::new();
    s.execute("file.open", &json!({"data": data, "name": "x.sketch"})).unwrap();
    let doc = s.doc().unwrap();
    let inst = doc.nodes().find(|n| n.name == "Long").unwrap().id;
    let label = doc.subtree(inst).into_iter().find(|id| doc.node(*id).is_some_and(|n| n.name == "Label")).unwrap();
    let dot = doc.subtree(inst).into_iter().find(|id| doc.node(*id).is_some_and(|n| n.name == "Dot")).unwrap();
    let (i, l, d) = (doc.world_bounds(inst).unwrap(), doc.world_bounds(label).unwrap(), doc.world_bounds(dot).unwrap());
    assert!(l.x1 - l.x0 > 60.0, "the label was measured: {l:?}");
    assert!((d.x0 - i.x0 - 8.0).abs() < 0.5, "the dot after the padding: {d:?} in {i:?}");
    assert!((l.x0 - d.x1 - 6.0).abs() < 0.5, "the label after the gap: {l:?}");
    assert!((i.x1 - l.x1 - 10.0).abs() < 0.5, "the chip hugs: {i:?} {l:?}");
}

/// An instance sized apart from its master lays its master's stack out at its own size: a wide
/// button keeps its label and icon together in the middle.
#[test]
fn sketch_resized_stack_instances_reflow() {
    let a = shape("rectangle", "A", [22.0, 12.0, 40.0, 20.0], json!({"points": []}));
    let b = shape("rectangle", "B", [70.0, 12.0, 20.0, 20.0], json!({"points": []}));
    let layout = json!({"_class": "MSImmutableFlexGroupLayout", "flexDirection": 0, "justifyContent": 1, "alignItems": 1, "allGuttersGap": 8, "stackingOrder": 0});
    let master = shape(
        "symbolMaster",
        "Button",
        [0.0, 0.0, 112.0, 44.0],
        json!({"symbolID": "S", "groupBehavior": 1, "groupLayout": layout, "horizontalSizing": 1, "verticalSizing": 1,
            "leftPadding": 22, "rightPadding": 22, "topPadding": 12, "bottomPadding": 12, "layers": [b, a]}),
    );
    let inst = shape(
        "symbolInstance",
        "Wide",
        [0.0, 100.0, 342.0, 44.0],
        json!({"symbolID": "S", "overrideValues": [], "horizontalSizing": 0, "verticalSizing": 1}),
    );
    let data = base64::engine::general_purpose::STANDARD.encode(sketch_with("2026.3.1", &[("A", "Page")], &[master, inst]));
    let mut s = Session::new();
    s.execute("file.open", &json!({"data": data, "name": "x.sketch"})).unwrap();
    let doc = s.doc().unwrap();
    let inst = doc.nodes().find(|n| n.name == "Wide").unwrap().id;
    let at = |name: &str| doc.world_bounds(doc.subtree(inst).into_iter().find(|id| doc.node(*id).is_some_and(|n| n.name == name)).unwrap()).unwrap();
    let (a, b) = (at("A"), at("B"));
    // 40 + 8 + 20 = 68 wide, centred in 342: from 137.
    assert!((a.x0 - 137.0).abs() < 0.5, "{a:?}");
    assert!((b.x0 - a.x1 - 8.0).abs() < 0.5, "{a:?} {b:?}");
    assert!((doc.world_bounds(inst).unwrap().x1 - 342.0).abs() < 1e-6);
}

/// Sketch's natural line: the font's ascent and descent each rounded to whole pixels, without its
/// line gap, the baseline at the rounded ascent; the classic Mac fonts get a fifth more, all of it
/// above the text.
#[test]
fn sketch_natural_line_rule() {
    use crate::cmd::file::sketch_line;
    // Helvetica Neue Medium (975 + 217 of 1000, a gap of 29 left out) at 17 px: 16.575 → 17 and
    // 3.689 → 4, a line of 21 with the baseline at 17. FigCraft puts the baseline at
    // (21 − 20.264) / 2 + 16.575 = 16.943, so the text moves down 0.057.
    let (line, down) = sketch_line(975.0, 217.0, 1000.0, "Helvetica Neue", 17.0);
    assert!((line - 21.0).abs() < 1e-9 && (down - 0.057).abs() < 1e-9, "{line} {down}");
    // Helvetica at 100 px: 77 + 23 and a fifth more, 120, all of it above: the baseline at 97
    // where FigCraft centres it at 87.
    let (line, down) = sketch_line(1577.0, 471.0, 2048.0, "helvetica", 100.0);
    assert!((line - 120.0).abs() < 1e-9 && (down - 10.0).abs() < 0.05, "{line} {down}");
    assert_eq!(sketch_line(0.0, 0.0, 0.0, "X", 12.0), (0.0, 0.0), "no metrics, no change");
    assert_eq!(sketch_line(900.0, 200.0, 1000.0, "X", f64::NAN), (0.0, 0.0));
}

/// Text without a fixed line height gets Sketch's natural line as a share of its size, in the
/// layer and in shared text styles.
#[test]
fn sketch_text_gets_sketchs_natural_line() {
    if !std::path::Path::new("/System/Library/Fonts/HelveticaNeue.ttc").exists() {
        return;
    }
    figcraft_text::fontdb::scan_system_fonts();
    let font = json!({"name": "HelveticaNeue", "size": 100});
    let run = json!({"location": 0, "length": 2, "attributes": {"MSAttributedStringFontAttribute": {"attributes": font}}});
    let t = shape("text", "Label", [0.0, 0.0, 200.0, 120.0], json!({"attributedString": {"string": "Hi", "attributes": [run]}, "textBehaviour": 0}));
    let data = base64::engine::general_purpose::STANDARD.encode(sketch_with("64", &[("A", "Page")], &[t]));
    let mut s = Session::new();
    s.execute("file.open", &json!({"data": data, "name": "x.sketch"})).unwrap();
    let doc = s.doc().unwrap();
    let NodeKind::Text(td) = &doc.nodes().find(|n| n.name == "Label").unwrap().kind else { panic!("not text") };
    // 95.2 → 95 and 21.3 → 21: 116 px, as a share of the size.
    let figcraft_doc::LineHeight::Percent(p) = td.style.line_height else { panic!("{:?}", td.style.line_height) };
    assert!((p - 116.0).abs() < 1e-9, "{p}");
}

/// A text an auto layout places isn't moved to Sketch's baseline (the next layout pass would take
/// the move back): laying the stack out again leaves it where the import put it.
#[test]
fn sketch_text_in_a_stack_stays_put() {
    if !std::path::Path::new("/System/Library/Fonts/Helvetica.ttc").exists() {
        return;
    }
    figcraft_text::fontdb::scan_system_fonts();
    let font = json!({"name": "Helvetica", "size": 40});
    let run = json!({"location": 0, "length": 2, "attributes": {"MSAttributedStringFontAttribute": {"attributes": font}}});
    let label = shape(
        "text",
        "Label",
        [10.0, 10.0, 50.0, 48.0],
        json!({"attributedString": {"string": "Hi", "attributes": [run]}, "horizontalSizing": 1, "verticalSizing": 1}),
    );
    let layout = json!({"_class": "MSImmutableFlexGroupLayout", "flexDirection": 1, "justifyContent": 0, "alignItems": 0, "allGuttersGap": 0, "stackingOrder": 0});
    let stack = shape(
        "group",
        "Stack",
        [0.0, 0.0, 200.0, 68.0],
        json!({"groupBehavior": 1, "groupLayout": layout, "leftPadding": 10, "topPadding": 10, "rightPadding": 10, "bottomPadding": 10, "layers": [label]}),
    );
    let data = base64::engine::general_purpose::STANDARD.encode(sketch_with("2026.3.1", &[("A", "Page")], &[stack]));
    let mut s = Session::new();
    s.execute("file.open", &json!({"data": data, "name": "x.sketch"})).unwrap();
    let mut doc = s.doc().unwrap().clone();
    let stack = doc.nodes().find(|n| n.name == "Stack").unwrap().id;
    let label = doc.nodes().find(|n| n.name == "Label").unwrap().id;
    let before = doc.world_bounds(label).unwrap();
    figcraft_layout::layout_frame(&mut doc, stack, &crate::measure::TextMeasure);
    let after = doc.world_bounds(label).unwrap();
    assert!((before.y0 - after.y0).abs() < 1e-6, "{before:?} → {after:?}");
}

/// Shared text styles get what their linked layers get on import: the installed family's name and
/// the platform's tracking (a layer and its style stay alike).
#[test]
fn sketch_shared_text_styles_match_their_layers() {
    if !std::path::Path::new("/System/Library/Fonts/SFNS.ttf").exists() {
        return;
    }
    figcraft_text::fontdb::scan_system_fonts();
    let attrs = json!({"MSAttributedStringFontAttribute": {"attributes": {"name": "SFUIText-Regular", "size": 13}}});
    let style = json!({"_class": "style", "textStyle": {"_class": "textStyle", "encodedAttributes": attrs.clone()}});
    let shared = json!({"_class": "sharedStyle", "do_objectID": "TS", "name": "Body", "value": style.clone()});
    let run = json!({"location": 0, "length": 2, "attributes": attrs});
    let mut linked = style;
    linked["sharedObjectID"] = json!("TS");
    let t = shape(
        "text",
        "Label",
        [0.0, 0.0, 100.0, 20.0],
        json!({"attributedString": {"string": "Hi", "attributes": [run]}, "textBehaviour": 0, "style": linked}),
    );
    let extra = json!({"layerTextStyles": {"_class": "sharedTextStyleContainer", "objects": [shared]}});
    let data = base64::engine::general_purpose::STANDARD.encode(sketch_with_doc("64", &[("A", "Page")], &[t], extra));
    let mut s = Session::new();
    s.execute("file.open", &json!({"data": data, "name": "x.sketch"})).unwrap();
    let doc = s.doc().unwrap();
    let NodeKind::Text(td) = &doc.nodes().find(|n| n.name == "Label").unwrap().kind else { panic!("not text") };
    let st = doc
        .styles
        .iter()
        .find_map(|st| match &st.kind {
            figcraft_doc::StyleKind::Text { style } if st.name == "Body" => Some(style.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(st.font_family, td.style.font_family);
    assert_eq!(st.letter_spacing.resolve(13.0), td.style.letter_spacing.resolve(13.0), "{st:?}");
}

/// An auto-width text an override made longer grows from its aligned edge, as in Sketch's renders:
/// a left-aligned one keeps its left edge, a centred one its centre, a right-aligned one its right
/// edge.
#[test]
fn sketch_overridden_text_grows_from_its_aligned_edge() {
    let font = json!({"name": "Helvetica", "size": 12});
    let mut layers = Vec::new();
    for (i, align) in [0, 2, 1].into_iter().enumerate() {
        let run = json!({"location": 0, "length": 2, "attributes": {"MSAttributedStringFontAttribute": {"attributes": font},
            "paragraphStyle": {"_class": "paragraphStyle", "alignment": align}}});
        let label = shape(
            &format!("L{i}"),
            &format!("L{i}"),
            [70.0, 4.0, 14.0, 16.0],
            json!({"_class": "text", "attributedString": {"string": "Go", "attributes": [run]}, "horizontalSizing": 1, "verticalSizing": 1}),
        );
        let sid = format!("S{i}");
        layers.push(shape("symbolMaster", &format!("M{i}"), [0.0, 100.0 * i as f64, 154.0, 24.0], json!({"symbolID": sid, "layers": [label]})));
        let ov = json!([{"_class": "overrideValue", "overrideName": format!("L{i}_stringValue"), "value": "A much longer label"}]);
        layers.push(shape(
            "symbolInstance",
            &format!("I{i}"),
            [300.0, 100.0 * i as f64, 154.0, 24.0],
            json!({"symbolID": sid, "overrideValues": ov}),
        ));
    }
    let data = base64::engine::general_purpose::STANDARD.encode(sketch_with("2026.3.1", &[("A", "Page")], &layers));
    let mut s = Session::new();
    s.execute("file.open", &json!({"data": data, "name": "x.sketch"})).unwrap();
    let doc = s.doc().unwrap();
    let bounds = |i: usize| {
        let inst = doc.nodes().find(|n| n.name == format!("I{i}")).unwrap();
        let label = doc.subtree(inst.id).into_iter().find(|id| doc.node(*id).is_some_and(|n| n.name == format!("L{i}"))).unwrap();
        let (b, l) = (doc.world_bounds(inst.id).unwrap(), doc.world_bounds(label).unwrap());
        assert!(l.x1 - l.x0 > 40.0, "the label was measured: {l:?}");
        (l.x0 - b.x0, l.x1 - b.x0)
    };
    let (left, centre, right) = (bounds(0), bounds(1), bounds(2));
    assert!((left.0 - 70.0).abs() < 0.5, "{left:?}");
    assert!(((centre.0 + centre.1) / 2.0 - 77.0).abs() < 0.5, "{centre:?}");
    assert!((right.1 - 84.0).abs() < 0.5, "{right:?}");
}
