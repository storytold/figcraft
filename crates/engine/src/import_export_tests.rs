//! REST import with prototypes and variables through the file commands, and layer export
//! settings (rows, sizes, "Export N layers"). Fixtures are synthetic.

use base64::Engine as _;
use figcraft_doc::proto::{Action, Trigger};
use figcraft_doc::*;
use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;

const THEME: &str = "VariableCollectionId:1:1";
const BRAND: &str = "VariableID:1:2";

fn variables() -> Value {
    json!({"status": 200, "error": false, "meta": {
        "variableCollections": {THEME: {"id": THEME, "name": "Theme", "defaultModeId": "1:0",
            "modes": [{"modeId": "1:0", "name": "Light"}, {"modeId": "1:1", "name": "Dark"}]}},
        "variables": {BRAND: {"id": BRAND, "name": "brand", "variableCollectionId": THEME, "resolvedType": "COLOR",
            "valuesByMode": {"1:0": {"r": 1, "g": 0, "b": 0, "a": 1}, "1:1": {"r": 0, "g": 0, "b": 1, "a": 1}}, "scopes": ["ALL_SCOPES"]}}
    }})
}

/// Two screens: a button on the first navigates to the second; its fill is bound to `brand`.
fn file() -> Value {
    let at = |x: f64| json!([[1, 0, x], [0, 1, 0]]);
    let red = json!({"r": 1, "g": 0, "b": 0, "a": 1});
    let action = json!({"type": "NODE", "destinationId": "1:3", "navigation": "NAVIGATE",
                        "transition": {"type": "DISSOLVE", "duration": 200, "easing": {"type": "EASE_OUT"}}});
    let button = json!({"id": "1:2", "name": "Button", "type": "RECTANGLE", "relativeTransform": at(10.0), "size": {"x": 40, "y": 20},
        "fills": [{"type": "SOLID", "color": red, "boundVariables": {"color": {"type": "VARIABLE_ALIAS", "id": BRAND}}}],
        "interactions": [{"trigger": {"type": "ON_CLICK"}, "actions": [action]}]});
    let screens = json!([
        {"id": "1:1", "name": "Home", "type": "FRAME", "relativeTransform": at(0.0), "size": {"x": 100, "y": 100}, "children": [button]},
        {"id": "1:3", "name": "Details", "type": "FRAME", "relativeTransform": at(200.0), "size": {"x": 100, "y": 100},
         "fills": [{"type": "SOLID", "color": red}]}
    ]);
    json!({"name": "Proto", "document": {"id": "0:0", "type": "DOCUMENT", "children": [
        {"id": "0:1", "name": "Screens", "type": "CANVAS", "backgroundColor": {"r": 1, "g": 1, "b": 1, "a": 1},
         "flowStartingPoints": [{"nodeId": "1:1", "name": "Flow"}], "children": screens}
    ]}})
}

fn named(s: &Session, name: &str) -> Node {
    s.doc().unwrap().nodes().find(|n| n.name == name).cloned().unwrap_or_else(|| panic!("no {name}"))
}

fn fill_hex(n: &Node) -> Option<String> {
    n.fills.first().and_then(|p| p.solid_color()).map(|c| c.to_hex())
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("figcraft-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn open_reads_variables_from_param_or_next_to_the_file() {
    let dir = temp_dir("restvars");
    let path = dir.join("proto.json");
    std::fs::write(&path, serde_json::to_vec(&file()).unwrap()).unwrap();
    let path = path.to_string_lossy().to_string();

    // Passed as an object.
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": path, "variables": variables()})).unwrap();
    let button = named(&s, "Button");
    assert_eq!(button.bound.get("fill").map(String::as_str), Some(BRAND));
    let details = named(&s, "Details").id;
    assert_eq!(button.interactions[0].actions, [Action::Navigate { dest: details, preserve_scroll: false }]);
    assert_eq!(button.interactions[0].trigger, Trigger::OnClick);
    // The binding is live: switching the mode recolours the button.
    s.execute("variable.setMode", &json!({"collection": "Theme", "mode": "Dark"})).unwrap();
    assert_eq!(fill_hex(&named(&s, "Button")).as_deref(), Some("0000FF"));

    // Found next to the file.
    let vars_path = dir.join("proto.variables.json");
    std::fs::write(&vars_path, serde_json::to_vec(&variables()).unwrap()).unwrap();
    let mut s = Session::new();
    let r = s.execute("file.open", &json!({"path": path})).unwrap();
    assert_eq!(s.doc().unwrap().variables.variables.len(), 1, "{r}");
    assert!(r["warnings"].as_array().is_some_and(|w| w.iter().all(|w| !w.as_str().unwrap_or("").contains("variable"))), "{r}");

    // As a path or JSON text; anything else is refused.
    let mut s = Session::new();
    s.execute("file.import", &json!({"path": path, "variables": vars_path.to_string_lossy()})).unwrap();
    assert_eq!(s.doc().unwrap().variables.collections.len(), 1);
    let mut s = Session::new();
    s.execute("file.import", &json!({"path": path, "variables": variables().to_string()})).unwrap();
    assert_eq!(s.doc().unwrap().variables.variables.len(), 1);
    assert!(Session::new().execute("file.open", &json!({"path": path, "variables": {"x": 1}})).is_err());
    assert!(Session::new().execute("file.open", &json!({"path": path, "variables": 5})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Every document, the active tab and the recent files, compared before and after a refused command.
fn state(s: &Session) -> (String, usize, Vec<String>) {
    (format!("{:?}", s.docs), s.active, s.prefs.recent_files.clone())
}

#[test]
fn an_explicit_variables_path_is_capped_like_the_sibling_file() {
    let dir = temp_dir("restvars-cap");
    let path = dir.join("proto.json");
    std::fs::write(&path, serde_json::to_vec(&file()).unwrap()).unwrap();
    let path = path.to_string_lossy().to_string();
    let vars = dir.join("theme.json");
    std::fs::write(&vars, serde_json::to_vec(&variables()).unwrap()).unwrap();
    let vars = vars.to_string_lossy().to_string();

    // A small file opens and imports as before.
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": path, "variables": vars})).unwrap();
    assert_eq!(named(&s, "Button").bound.get("fill").map(String::as_str), Some(BRAND));

    // One byte over the limit (sparse, so nothing is written), a folder, and a device are refused
    // before the document changes, by the same check the sibling file goes through.
    let big = dir.join("big.json");
    std::fs::File::create(&big).unwrap().set_len(256 * 1024 * 1024 + 1).unwrap();
    let mut refused = vec![big.to_string_lossy().to_string(), dir.to_string_lossy().to_string()];
    if cfg!(unix) {
        refused.push("/dev/null".to_string());
    }
    s.prefs.recent_files = vec![path.clone()];
    s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 20, "height": 10})).unwrap();
    for bad in &refused {
        for cmd in ["file.open", "file.import"] {
            let before = state(&s);
            let e = s.execute(cmd, &json!({"path": path, "variables": bad})).unwrap_err().to_string();
            assert!(
                ["more than the 256 MB allowed here", "is a folder, not a file", "isn't a regular file"].iter().any(|w| e.contains(w)),
                "{cmd} {bad}: {e}"
            );
            assert_eq!(state(&s), before, "{cmd} {bad}");
        }
    }

    // Access roots still apply: inside them the file reads, outside it is rejected.
    let inside = temp_dir("restvars-cap-root");
    std::fs::copy(&vars, inside.join("theme.json")).unwrap();
    std::fs::copy(&path, inside.join("proto.json")).unwrap();
    let roots = std::sync::Arc::new(crate::file_access::Roots::new(&[inside.to_string_lossy().to_string()], &[]).unwrap());
    let inside_vars = inside.join("theme.json").to_string_lossy().to_string();
    let inside_path = inside.join("proto.json").to_string_lossy().to_string();
    crate::file_access::confined(Some(&roots), || {
        s.execute("file.import", &json!({"path": inside_path, "variables": inside_vars})).unwrap();
        let before = state(&s);
        let e = s.execute("file.import", &json!({"path": inside_path, "variables": vars})).unwrap_err().to_string();
        assert!(e.contains("automation path rejected"), "{e}");
        assert_eq!(state(&s), before);
    });
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&inside);
}

#[test]
fn import_appends_flows_interactions_and_variables() {
    let mut s = Session::new();
    let data = base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(&file()).unwrap());
    s.execute("file.import", &json!({"data": data, "name": "proto.json", "variables": variables()})).unwrap();
    // Twice: ids stay unique, variables aren't duplicated.
    s.execute("file.import", &json!({"data": data, "name": "proto.json", "variables": variables()})).unwrap();
    let doc = s.doc().unwrap();
    assert_eq!(doc.variables.variables.len(), 1);
    assert_eq!(doc.pages.len(), 3);
    for page in &doc.pages[1..] {
        let NodeKind::Page { flows, .. } = &doc.node(*page).unwrap().kind else { panic!() };
        assert_eq!(flows.len(), 1);
        assert_eq!(doc.page_of(flows[0].start), Some(*page), "flows point into their own page");
        let home = doc.node(flows[0].start).unwrap();
        let button = doc.node(home.children[0]).unwrap();
        let dest = button.interactions[0].actions[0].dest().unwrap();
        assert_eq!(doc.page_of(dest), Some(*page));
        assert_eq!(doc.node(dest).unwrap().name, "Details");
        assert_eq!(button.bound.get("fill").map(String::as_str), Some(BRAND));
    }
}

// ---- export settings ----

fn rect(s: &mut Session, name: &str, w: f64, h: f64) -> NodeId {
    let id = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": w, "height": h})).unwrap()["id"]).unwrap();
    s.execute("node.set", &json!({"ids": [id], "props": {"name": name}})).unwrap();
    id
}

fn exports(s: &Session, id: NodeId) -> Vec<ExportSetting> {
    s.doc().unwrap().node(id).unwrap().exports.clone()
}

/// Width of a PNG from its header.
fn png_width(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[16], b[17], b[18], b[19]])
}

#[test]
fn export_rows_add_step_up_and_update() {
    let mut s = Session::new();
    let r = rect(&mut s, "Icon", 32.0, 16.0);
    for _ in 0..3 {
        s.execute("export.add", &json!({"ids": [r]})).unwrap();
    }
    let rows = exports(&s, r);
    let sizes: Vec<String> = rows.iter().map(|e| e.size.label()).collect();
    assert_eq!(sizes, ["1x", "2x", "3x"]);
    assert_eq!(rows.iter().map(|e| e.suffix.as_str()).collect::<Vec<_>>(), ["", "@2x", "@3x"]);

    s.execute("export.update", &json!({"ids": [r], "index": 1, "size": "512w", "format": "webp", "suffix": "-wide"})).unwrap();
    s.execute("export.update", &json!({"ids": [r], "index": 2, "size": {"by": "height", "value": 64}, "format": "pdf"})).unwrap();
    let rows = exports(&s, r);
    assert_eq!(
        rows[1],
        ExportSetting { size: ExportSize::Width { value: 512.0 }, suffix: "-wide".into(), format: ExportFormat::Webp, ..Default::default() }
    );
    assert_eq!((rows[2].size, rows[2].format), (ExportSize::Height { value: 64.0 }, ExportFormat::Pdf));
    for bad in [
        json!({"size": "0x"}),
        json!({"size": "abc"}),
        json!({"size": "1e9w"}),
        json!({"format": "gif"}),
        json!({"size": {"by": "scale", "value": -1}}),
    ] {
        let mut p = bad.clone();
        p["ids"] = json!([r]);
        p["index"] = json!(0);
        assert!(s.execute("export.update", &p).is_err(), "{bad}");
    }
    assert_eq!(exports(&s, r), rows, "refused edits change nothing");
    // A given setting is used as is (sizes as text too).
    s.execute("export.add", &json!({"ids": [r], "setting": {"size": "0.5x", "format": "jpg"}})).unwrap();
    assert_eq!(exports(&s, r)[3].size, ExportSize::Scale { value: 0.5 });
    assert!(s.execute("export.add", &json!({"ids": [r], "setting": {"size": "huge"}})).is_err());
}

#[test]
fn export_layers_uses_each_layers_settings() {
    let mut s = Session::new();
    let a = rect(&mut s, "Icon", 32.0, 16.0);
    let b = rect(&mut s, "Logo/Dark", 10.0, 10.0);
    let c = rect(&mut s, "Plain", 10.0, 10.0);
    s.execute("export.add", &json!({"ids": [a]})).unwrap();
    s.execute("export.add", &json!({"ids": [a]})).unwrap();
    s.execute("export.add", &json!({"ids": [a], "setting": {"size": "512w", "format": "png"}})).unwrap();
    s.execute("export.add", &json!({"ids": [b], "setting": {"format": "svg"}})).unwrap();

    let r = s.execute("file.exportLayers", &json!({"ids": [a, b, c]})).unwrap();
    let files = r["files"].as_array().unwrap();
    let names: Vec<&str> = files.iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Icon.png", "Icon@2x.png", "Icon 2.png", "Logo/Dark.svg"], "unique names; a slash makes a folder");
    assert_eq!(r["skipped"], 1);
    let data = |i: usize| base64::engine::general_purpose::STANDARD.decode(files[i]["data"].as_str().unwrap()).unwrap();
    assert_eq!(png_width(&data(0)), 32);
    assert_eq!(png_width(&data(1)), 64);
    assert_eq!(png_width(&data(2)), 512, "512w scales to that width");
    assert!(String::from_utf8_lossy(&data(3)).contains("<svg"));

    // Without ids or a selection: every layer on the page with settings; into a folder.
    s.set_selection(Vec::new());
    let dir = temp_dir("exportlayers");
    let r = s.execute("file.exportLayers", &json!({"dir": dir.to_string_lossy()})).unwrap();
    assert_eq!(r["files"].as_array().map(Vec::len), Some(4));
    assert!(dir.join("Icon@2x.png").is_file() && dir.join("Logo").join("Dark.svg").is_file());
    let _ = std::fs::remove_dir_all(&dir);

    // Nothing to export.
    assert!(s.execute("file.exportLayers", &json!({"ids": [c]})).is_err());
    // `file.export` takes the same sizes.
    let r = s.execute("file.export", &json!({"ids": [a], "size": "128w"})).unwrap();
    assert_eq!(png_width(&base64::engine::general_purpose::STANDARD.decode(r["data"].as_str().unwrap()).unwrap()), 128);
    assert!(s.execute("file.export", &json!({"ids": [a], "size": "-2x"})).is_err());
}

#[test]
fn export_sizes_parse_and_label() {
    for (text, want) in [
        ("2x", ExportSize::Scale { value: 2.0 }),
        ("@0.5X", ExportSize::Scale { value: 0.5 }),
        (" 3 ", ExportSize::Scale { value: 3.0 }),
        ("512w", ExportSize::Width { value: 512.0 }),
        ("100 h", ExportSize::Height { value: 100.0 }),
    ] {
        assert_eq!(ExportSize::parse(text), Some(want), "{text}");
        assert_eq!(ExportSize::parse(&want.label()), Some(want));
    }
    for bad in ["", "x", "0x", "-1x", "NaNx", "infx", "65x", "20000w", "2q", "2xx"] {
        assert_eq!(ExportSize::parse(bad), None, "{bad}");
    }
    let e = |size, suffix: &str, format| ExportSetting { size, suffix: suffix.into(), format, ..Default::default() };
    assert_eq!(e(ExportSize::Scale { value: 2.0 }, "", ExportFormat::Png).file_suffix(), "@2x");
    assert_eq!(e(ExportSize::Scale { value: 2.0 }, "", ExportFormat::Svg).file_suffix(), "");
    assert_eq!(e(ExportSize::Scale { value: 2.0 }, "-big", ExportFormat::Png).file_suffix(), "-big");
    assert_eq!(e(ExportSize::Width { value: 64.0 }, "", ExportFormat::Png).file_suffix(), "");
}

#[test]
fn export_size_label_parses_back_exactly() {
    for want in [
        ExportSize::Scale { value: 0.125 },
        ExportSize::Scale { value: 0.004 },
        ExportSize::Scale { value: 0.001 },
        ExportSize::Scale { value: 63.99 },
        ExportSize::Scale { value: 1.0 / 3.0 },
        ExportSize::Width { value: 512.125 },
        ExportSize::Height { value: 0.005 },
    ] {
        assert_eq!(ExportSize::parse(&want.label()), Some(want), "{}", want.label());
    }
    assert_eq!(ExportSize::Scale { value: 0.125 }.label(), "0.125x");
    assert_eq!(ExportSize::Scale { value: 2.0 }.label(), "2x");
    assert_eq!(ExportSize::Width { value: 512.0 }.label(), "512w");
}
