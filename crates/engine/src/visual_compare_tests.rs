//! `inspect.compare` on captures made from the design itself, then changed the way a build can be
//! wrong, so the expected answer is known.

use base64::Engine as _;
use serde_json::{Value, json};

use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s
}

fn make(s: &mut Session, params: Value) -> u64 {
    s.execute("node.create", &params).unwrap()["id"].as_u64().unwrap()
}

/// A white 400 × 300 frame with a blue box.
fn card(s: &mut Session) -> (u64, u64) {
    let f = make(s, json!({"type": "frame", "name": "Card", "width": 400, "height": 300, "props": {"fill": "#FFFFFF"}}));
    let b = make(
        s,
        json!({"type": "rectangle", "parent": f, "name": "Box", "x": 100, "y": 80, "width": 120, "height": 60, "props": {"fill": "#2040C0"}}),
    );
    (f, b)
}

/// The frame as a PNG, as a browser capture would be, at `scale`.
fn capture(s: &mut Session, id: u64, scale: f64) -> String {
    let v = s.execute("file.export", &json!({"ids": [id], "format": "png", "scale": scale})).unwrap();
    v["data"].as_str().unwrap().to_string()
}

fn compare(s: &mut Session, id: u64, data: &str) -> Value {
    s.execute("inspect.compare", &json!({"id": id, "data": data})).unwrap()
}

fn differs<'a>(v: &'a Value, name: &str) -> Option<&'a Value> {
    v["differences"].as_array().unwrap().iter().find(|d| d["name"] == name)
}

#[test]
fn a_faithful_capture_has_no_differences() {
    let mut s = session();
    let (f, _) = card(&mut s);
    let data = capture(&mut s, f, 1.0);
    let v = compare(&mut s, f, &data);
    assert_eq!(v["total"], 0, "{v}");
    assert_eq!(v["changedPercent"], 0.0);
    assert_eq!(v["viewport"], json!({"width": 400.0, "height": 300.0}));
    assert_eq!(v["capture"]["sameSize"], true);
}

#[test]
fn a_retina_capture_is_the_same_design() {
    let mut s = session();
    let (f, _) = card(&mut s);
    let data = capture(&mut s, f, 2.0);
    let v = compare(&mut s, f, &data);
    assert_eq!(v["capture"]["density"], 2.0);
    assert_eq!(v["total"], 0, "{v}");
}

#[test]
fn a_wrong_colour_names_the_layer_and_both_colours() {
    let mut s = session();
    let (f, b) = card(&mut s);
    s.execute("node.set", &json!({"ids": [b], "props": {"fill": "#C02020"}})).unwrap();
    let data = capture(&mut s, f, 1.0);
    s.execute("node.set", &json!({"ids": [b], "props": {"fill": "#2040C0"}})).unwrap();
    let v = compare(&mut s, f, &data);
    let d = differs(&v, "Box").unwrap_or_else(|| panic!("{v}"));
    assert_eq!(d["kind"], "color");
    assert_eq!((d["expected"].as_str(), d["actual"].as_str()), (Some("#2040C0"), Some("#C02020")));
    assert!(differs(&v, "Card").is_none(), "reported once, on the deepest layer: {v}");
}

#[test]
fn a_moved_layer_says_how_far() {
    let mut s = session();
    let (f, b) = card(&mut s);
    s.execute("node.set", &json!({"ids": [b], "props": {"x": 108, "y": 76}})).unwrap();
    let data = capture(&mut s, f, 1.0);
    s.execute("node.set", &json!({"ids": [b], "props": {"x": 100, "y": 80}})).unwrap();
    let v = compare(&mut s, f, &data);
    let d = differs(&v, "Box").unwrap_or_else(|| panic!("{v}"));
    assert_eq!(d["kind"], "position");
    assert_eq!(d["offset"], json!({"x": 8.0, "y": -4.0}));
}

#[test]
fn a_capture_of_another_size_says_so() {
    let mut s = session();
    let (f, _) = card(&mut s);
    let wider = make(&mut s, json!({"type": "frame", "name": "Wide", "width": 400, "height": 500, "props": {"fill": "#FFFFFF"}}));
    let data = capture(&mut s, wider, 1.0);
    let v = compare(&mut s, f, &data);
    assert_eq!(v["capture"]["sameSize"], false);
    assert_eq!(v["capture"]["cssHeight"], 500.0);
}

#[test]
fn hidden_layers_arent_blamed() {
    let mut s = session();
    let (f, b) = card(&mut s);
    let data = capture(&mut s, f, 1.0);
    s.execute("node.set", &json!({"ids": [b], "props": {"visible": false}})).unwrap();
    let v = compare(&mut s, f, &data);
    assert!(differs(&v, "Box").is_none());
    assert!(differs(&v, "Card").is_some(), "the frame shows what it didn't expect: {v}");
}

#[test]
fn a_path_is_read_only_inside_the_automation_roots() {
    let mut s = session();
    let (f, _) = card(&mut s);
    let base = std::env::temp_dir().join(format!("figcraft-compare-{}", std::process::id()));
    let (inside, outside) = (base.join("inside"), base.join("outside"));
    std::fs::create_dir_all(&inside).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let bytes = base64::engine::general_purpose::STANDARD.decode(capture(&mut s, f, 1.0)).unwrap();
    for dir in [&inside, &outside] {
        std::fs::write(dir.join("shot.png"), &bytes).unwrap();
    }
    let roots = std::sync::Arc::new(crate::file_access::Roots::new(&[inside.to_string_lossy().to_string()], &[]).unwrap());
    crate::file_access::confined(Some(&roots), || {
        assert_eq!(s.execute("inspect.compare", &json!({"id": f, "path": inside.join("shot.png")})).unwrap()["total"], 0);
        let e = s.execute("inspect.compare", &json!({"id": f, "path": outside.join("shot.png")})).unwrap_err().to_string();
        assert!(e.contains("automation path rejected"), "{e}");
    });
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn junk_is_refused_without_a_panic() {
    let mut s = session();
    let (f, _) = card(&mut s);
    for p in [
        json!({"id": f}),
        json!({"id": f, "data": "not base64!"}),
        json!({"id": f, "data": "aGVsbG8="}),
        json!({"id": 999_999, "data": capture(&mut s, f, 1.0)}),
    ] {
        assert!(s.execute("inspect.compare", &p).is_err(), "{p}");
    }
    let data = capture(&mut s, f, 1.0);
    // Out of range numbers are clamped; a value that isn't a number is a mistake.
    assert!(s.execute("inspect.compare", &json!({"id": f, "data": data, "tolerance": 1000, "density": -1})).is_ok());
    for p in [json!({"id": f, "data": data, "tolerance": "x"}), json!({"id": f, "data": data, "density": "2x"}), json!({"id": f, "data": 7})] {
        assert!(s.execute("inspect.compare", &p).is_err(), "{p}");
    }
}

#[test]
fn a_malformed_target_is_refused_rather_than_read_as_the_selection() {
    let mut s = session();
    let (f, _) = card(&mut s);
    let data = capture(&mut s, f, 1.0);
    s.execute("select.set", &json!({"ids": [f]})).unwrap();
    for target in [json!({"id": "not-an-id"}), json!({"ids": [f, "nope"]})] {
        let mut p = target.clone();
        p["data"] = json!(data);
        assert!(s.execute("inspect.compare", &p).is_err(), "{target}");
    }
    // With no target, the selection is the frame.
    assert_eq!(s.execute("inspect.compare", &json!({"data": data})).unwrap()["target"]["id"], f);
}

#[test]
fn a_capture_and_a_path_together_are_refused() {
    let mut s = session();
    let (f, _) = card(&mut s);
    let data = capture(&mut s, f, 1.0);
    assert!(s.execute("inspect.compare", &json!({"id": f, "data": data, "path": "shot.png"})).is_err());
}

#[test]
fn many_differences_are_listed_up_to_the_cap_and_counted() {
    let mut s = session();
    let f = make(&mut s, json!({"type": "frame", "name": "Grid", "width": 180, "height": 120, "props": {"fill": "#FFFFFF"}}));
    let boxes: Vec<u64> = (0..150)
        .map(|k| {
            let (x, y) = ((k % 15) * 12, (k / 15) * 12);
            make(&mut s, json!({"type": "rectangle", "parent": f, "x": x, "y": y, "width": 8, "height": 8, "props": {"fill": "#2040C0"}}))
        })
        .collect();
    for b in &boxes {
        s.execute("node.set", &json!({"ids": [b], "props": {"fill": "#C02020"}})).unwrap();
    }
    let data = capture(&mut s, f, 1.0);
    for b in &boxes {
        s.execute("node.set", &json!({"ids": [b], "props": {"fill": "#2040C0"}})).unwrap();
    }
    let v = compare(&mut s, f, &data);
    let listed = v["differences"].as_array().unwrap();
    assert_eq!(v["total"], 150, "every changed box is counted: {}", v["total"]);
    assert!(listed.len() < 150, "the list is capped below the count");
    assert!(listed.iter().all(|d| d["kind"] == "color"), "{v}");
}

#[test]
fn one_frame_is_compared_at_a_time() {
    let mut s = session();
    let (f, b) = card(&mut s);
    let data = capture(&mut s, f, 1.0);
    let e = s.execute("inspect.compare", &json!({"ids": [f, b], "data": data})).unwrap_err().to_string();
    assert!(e.contains("one frame"), "{e}");
}

#[test]
fn only_a_frame_component_or_instance_is_compared() {
    let mut s = session();
    let (f, b) = card(&mut s);
    let data = capture(&mut s, f, 1.0);
    let e = s.execute("inspect.compare", &json!({"id": b, "data": data})).unwrap_err().to_string();
    assert!(e.contains("frame"), "a rectangle named outright: {e}");
    s.execute("select.set", &json!({"ids": [b]})).unwrap();
    let e = s.execute("inspect.compare", &json!({"data": data})).unwrap_err().to_string();
    assert!(e.contains("frame"), "a rectangle selected: {e}");
    let c = s.execute("component.create", &json!({"ids": [f]})).unwrap()["id"].as_u64().unwrap();
    let i = s.execute("instance.create", &json!({"component": c, "x": 800, "y": 0})).unwrap()["id"].as_u64().unwrap();
    for id in [c, i] {
        assert!(s.execute("inspect.compare", &json!({"id": id, "data": data})).is_ok(), "{id}");
    }
}

#[test]
fn a_capture_is_not_kept_in_the_journal() {
    let mut s = session();
    let (f, _) = card(&mut s);
    let data = capture(&mut s, f, 1.0);
    compare(&mut s, f, &data);
    assert!(!s.journal.iter().any(|(_, p)| p.to_string().contains(&data)), "the log must not hold the capture");
}

#[test]
fn only_the_frames_own_area_is_drawn_for_the_comparison() {
    let mut s = session();
    let (f, _) = card(&mut s);
    s.execute("node.set", &json!({"ids": [f], "props": {"clipsContent": false}})).unwrap();
    let data = capture(&mut s, f, 1.0);
    // A layer past the frame's left edge is drawn, but it isn't part of the page.
    make(&mut s, json!({"type": "rectangle", "parent": f, "x": -160, "y": 20, "width": 100, "height": 100, "props": {"fill": "#00AA00"}}));
    let v = compare(&mut s, f, &data);
    assert_eq!(v["total"], 0, "{v}");
}

#[test]
fn a_shape_with_see_through_corners_leaves_the_layer_under_it_to_blame() {
    let mut s = session();
    let (f, b) = card(&mut s);
    make(&mut s, json!({"type": "ellipse", "parent": f, "name": "Dot", "x": 100, "y": 80, "width": 120, "height": 60, "props": {"fill": "#FFFFFF"}}));
    s.execute("node.set", &json!({"ids": [b], "props": {"fill": "#C02020"}})).unwrap();
    let data = capture(&mut s, f, 1.0);
    s.execute("node.set", &json!({"ids": [b], "props": {"fill": "#2040C0"}})).unwrap();
    let v = compare(&mut s, f, &data);
    assert!(differs(&v, "Box").is_some(), "the box shows through the ellipse's corners: {v}");
}
