//! Lengths in rem: `node.set` takes "2rem" and "24px" for size, spacing, radius and text fields;
//! layers keep pixels and remember the rem; Preferences' size of 1rem moves them.

use serde_json::{Value, json};

use crate::Session;
use crate::params::id_of;
use crate::units::{Length, parse_length};
use figcraft_doc::{NodeId, NodeKind};

fn id(v: Value) -> NodeId {
    id_of(&v["id"]).unwrap()
}

fn run(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

#[test]
fn lengths_parse_in_px_and_rem() {
    assert_eq!(parse_length("2rem"), Some(Length::Rem(2.0)));
    assert_eq!(parse_length(" 1.5 rem"), Some(Length::Rem(1.5)));
    assert_eq!(parse_length("24px"), Some(Length::Px(24.0)));
    assert_eq!(parse_length("24"), Some(Length::Px(24.0)));
    for junk in ["", "rem", "NaNrem", "1e400rem", "auto", "2em"] {
        assert_eq!(parse_length(junk), None, "{junk}");
    }
}

#[test]
fn fields_set_in_rem_keep_it_until_set_in_pixels() {
    let mut s = Session::new();
    let f = id(run(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 100})));
    run(
        &mut s,
        "node.set",
        json!({"ids": [f], "props": {"width": "10rem", "cornerRadius": "0.5rem", "layoutMode": "vertical", "gap": "1.5rem", "paddingHorizontal": "2rem"}}),
    );
    let n = s.doc().unwrap().node(f).unwrap().clone();
    assert_eq!(n.width, 160.0);
    assert_eq!(n.corners.radii, [8.0; 4]);
    assert_eq!(n.layout.as_ref().map(|l| (l.gap, l.padding[1], l.padding[3])), Some((24.0, 32.0, 32.0)));
    assert_eq!(n.units.get("width"), Some(&10.0));
    assert_eq!(n.units.get("paddingHorizontal"), Some(&2.0));
    // Pixels (a number or "px") make the field pixels again.
    run(&mut s, "node.set", json!({"ids": [f], "props": {"width": 200, "gap": "12px"}}));
    let n = s.doc().unwrap().node(f).unwrap().clone();
    assert_eq!((n.width, n.units.get("width"), n.units.get("gap")), (200.0, None, None));
    assert_eq!(n.units.get("cornerRadius"), Some(&0.5));
    // Text sizes too; undo takes the rem back with the value.
    let t = id(run(&mut s, "text.create", json!({"text": "Hi"})));
    run(&mut s, "node.set", json!({"ids": [t], "props": {"fontSize": "2rem"}}));
    let size = |s: &Session| match &s.doc().unwrap().node(t).unwrap().kind {
        NodeKind::Text(td) => td.style.font_size,
        _ => 0.0,
    };
    assert_eq!(size(&s), 32.0);
    run(&mut s, "edit.undo", json!({}));
    assert!(s.doc().unwrap().node(t).unwrap().units.is_empty());
    // Other fields don't take rem.
    assert!(s.execute("node.set", &json!({"ids": [f], "props": {"x": "2rem"}})).is_err());
}

#[test]
fn the_size_of_1rem_moves_lengths_set_in_rem() {
    let mut s = Session::new();
    let f = id(run(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 100})));
    let g = id(run(&mut s, "frame.create", json!({"x": 200, "y": 0, "width": 100, "height": 100})));
    run(&mut s, "node.set", json!({"ids": [f, g], "props": {"width": "4rem"}}));
    // g was resized in pixels since: it no longer follows its rem.
    run(&mut s, "node.set", json!({"ids": [g], "props": {"height": 50}}));
    run(&mut s, "pref.remSize", json!({"px": 20}));
    assert_eq!(s.doc().unwrap().node(f).unwrap().width, 80.0);
    assert_eq!(s.doc().unwrap().node(g).unwrap().width, 80.0, "its width is still in rem");
    run(&mut s, "node.set", json!({"ids": [f], "props": {"height": "1rem"}}));
    assert_eq!(s.doc().unwrap().node(f).unwrap().height, 20.0, "new lengths use the new size");
    // Bounds and junk.
    assert_eq!(run(&mut s, "pref.remSize", json!({"px": 0}))["px"], 1.0);
    assert!(s.execute("pref.remSize", &json!({"px": "big"})).is_err());
}
