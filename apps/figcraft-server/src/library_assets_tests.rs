//! Styles and variables in a team library's analytics, end to end over loopback: a file that uses
//! them reports how many of its layers do, the library's owner sees them per style and variable
//! with their names and files, and a file that stops using them stops counting. Reports are cleaned
//! and capped, and the usage list keeps its encoding.

use std::time::{Duration, Instant};

use figcraft_collab::message::{Access, AssetUsage, ClientMsg, LibraryReply, LibraryReq, MAX_ASSET_USAGE, OpenLibrary, PROTOCOL, ServerMsg};
use figcraft_collab_net::Invite;
use figcraft_doc::NodeId;
use figcraft_engine::Session;
use serde_json::{Value, json};

use crate::test_util::{Raw, TempDir, config, publish, session};

fn ex(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn id(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// `a`'s analytics once `done` holds for them; `b` reports again each time, as a check asked for
/// while the last one still runs is dropped.
fn usage_when(a: &mut Session, b: &mut Session, done: impl Fn(&Value) -> bool) -> Value {
    let start = Instant::now();
    loop {
        b.refresh_server_libraries();
        let u = ex(a, "library.usage", json!({}));
        if done(&u) {
            return u;
        }
        assert!(start.elapsed() < Duration::from_secs(30), "the reports never arrived: {u}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn rect(s: &mut Session, x: i32) -> NodeId {
    id(&ex(s, "shape.rectangle", json!({"x": x, "y": 0, "width": 40, "height": 40})))
}

#[test]
fn styles_and_variables_are_counted_per_layer_and_file() {
    let dir = TempDir::new("library-assets");
    let cfg = config(dir.path());
    let mut server = crate::start(&cfg).unwrap();
    let addr = server.addr();
    // Ada's library: a colour style, a colour variable, and a Card that uses the style.
    let mut a = session("Ada");
    let r = rect(&mut a, 0);
    let style = ex(&mut a, "style.create", json!({"type": "paint", "name": "Brand", "from": r}));
    let brand = style["id"].as_str().unwrap().to_string();
    let collection = ex(&mut a, "variable.createCollection", json!({"name": "Tokens"}));
    ex(&mut a, "variable.create", json!({"name": "Accent", "type": "color", "collection": collection["id"], "value": "#0D99FF"}));
    let card = id(&ex(&mut a, "frame.create", json!({"x": 0, "y": 100, "width": 100, "height": 40, "name": "Card"})));
    ex(&mut a, "style.apply", json!({"ids": [card], "style": brand}));
    ex(&mut a, "component.create", json!({"ids": [card]}));
    let p = publish(&mut a, dir.path(), addr);
    ex(&mut a, "library.publishToServer", json!({}));

    // Bo uses the style on two layers and the variable on one, and places a Card: it belongs to
    // the component, so it doesn't count for the style.
    let mut b = session("Bo");
    let lid = ex(&mut b, "library.add", json!({"path": p.view}))["id"].as_str().unwrap().to_string();
    let (one, two, three) = (rect(&mut b, 0), rect(&mut b, 100), rect(&mut b, 200));
    ex(&mut b, "library.applyStyle", json!({"library": lid, "style": "Brand", "ids": [one, two]}));
    ex(&mut b, "library.bindVariable", json!({"library": lid, "variable": "Accent", "ids": [three], "field": "fill"}));
    ex(&mut b, "library.place", json!({"library": lid, "component": "Card", "x": 300, "y": 0}));

    let u = usage_when(&mut a, &mut b, |u| u["styles"][0]["layers"] == 2 && u["variables"][0]["layers"] == 1 && u["instances"] == 1);
    assert_eq!(u["assetsKnown"], true, "{u}");
    let s = &u["styles"][0];
    assert_eq!((s["name"].clone(), s["type"].clone(), s["files"].clone()), (json!("Brand"), json!("paint"), json!(1)), "{u}");
    let v = &u["variables"][0];
    assert_eq!((v["name"].clone(), v["collection"].clone(), v["files"].clone()), (json!("Accent"), json!("Tokens"), json!(1)), "{u}");
    assert_eq!(u["instances"], 1, "components count as before: {u}");

    // Bo takes the style off one layer and the variable off its only one.
    ex(&mut b, "style.detach", json!({"ids": [two], "which": "fill"}));
    ex(&mut b, "variable.bind", json!({"ids": [three], "field": "fill"}));
    let u = usage_when(&mut a, &mut b, |u| u["styles"][0]["layers"] == 1 && u["variables"].as_array().is_some_and(Vec::is_empty));
    assert_eq!(u["styles"].as_array().unwrap().len(), 1, "{u}");
    server.stop();
}

/// Reports from the wire: names cleaned, counts capped and long ids dropped, one count per id; the
/// reports survive the server's restart, and a server's list has only files that use something.
#[test]
fn asset_reports_are_capped_and_kept() {
    let dir = TempDir::new("library-assets-wire");
    let cfg = config(dir.path());
    let mut server = crate::start(&cfg).unwrap();
    let addr = server.addr();
    let mut a = session("Ada");
    let r = rect(&mut a, 0);
    ex(&mut a, "style.create", json!({"type": "paint", "name": "Brand", "from": r}));
    let p = publish(&mut a, dir.path(), addr);
    ex(&mut a, "library.publishToServer", json!({}));
    let view = Invite::parse(&p.view).unwrap();
    let open = |addr| {
        let mut raw = Raw::connect(addr);
        raw.send(&ClientMsg::OpenLibrary(OpenLibrary {
            protocol: PROTOCOL,
            file: view.file.clone(),
            token: view.token.clone(),
            client_version: String::new(),
        }));
        assert_eq!(answer(&mut raw), LibraryReply::Ready { access: Access::View });
        raw
    };
    let mut raw = open(addr);
    let flood = (0..5_000).map(|i| (format!("s{i}"), 1)).collect();
    let noted = |raw: &mut Raw, a| {
        raw.send(&ClientMsg::Library(LibraryReq::ReportAssets(a)));
        assert_eq!(answer(raw), LibraryReply::Noted);
    };
    noted(&mut raw, AssetUsage { file: "Spam".into(), styles: flood, variables: Vec::new() });
    let odd = vec![("x".repeat(65), 7), ("a".into(), 4), ("a".into(), 9), ("b".into(), 0)];
    noted(&mut raw, AssetUsage { file: "\u{7}Shop\n".into(), styles: Vec::new(), variables: odd });
    noted(&mut raw, AssetUsage { file: "Nothing".into(), ..AssetUsage::default() });
    raw.send(&ClientMsg::Library(LibraryReq::Assets));
    let LibraryReply::Assets(files) = answer(&mut raw) else { panic!("no reports") };
    assert_eq!(files.len(), 2, "a file that uses nothing isn't listed: {files:?}");
    assert_eq!(files.iter().find(|f| f.file == "Spam").unwrap().styles.len(), MAX_ASSET_USAGE);
    assert_eq!(files.iter().find(|f| f.file == "Shop").unwrap().variables, [("a".to_string(), 4)]);
    // Older apps ask for the usage list, where nobody reported using the library.
    raw.send(&ClientMsg::Library(LibraryReq::Usage));
    assert_eq!(answer(&mut raw), LibraryReply::Usage(Vec::new()));
    server.stop();

    let mut server = crate::start(&cfg).unwrap();
    let mut raw = open(server.addr());
    raw.send(&ClientMsg::Library(LibraryReq::Assets));
    let LibraryReply::Assets(kept) = answer(&mut raw) else { panic!("no reports") };
    assert_eq!(kept, files);
    server.stop();
}

fn answer(raw: &mut Raw) -> LibraryReply {
    match raw.next() {
        ServerMsg::Library(r) => *r,
        other => panic!("not a library answer: {other:?}"),
    }
}
