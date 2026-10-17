use crate::{Session, selection_spacing::selection_spacing};
use figcraft_doc::NodeId;
use serde_json::json;

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64, parent: Option<NodeId>) -> NodeId {
    NodeId(s.execute("shape.rectangle", &json!({"x":x,"y":y,"width":w,"height":h,"parent":parent})).unwrap()["id"].as_u64().unwrap())
}
fn xy(s: &Session, id: NodeId) -> (f64, f64) {
    let b = s.doc().unwrap().world_bounds(id).unwrap();
    (b.x0, b.y0)
}

#[test]
fn rows_columns_and_grids_keep_track_contents_and_sizes() {
    let mut s = Session::new();
    // Unequal widths, left-aligned columns. The column widths are 50 and 80; gaps 30 and 20.
    let a = rect(&mut s, 100., 100., 50., 40., None);
    let b = rect(&mut s, 180., 100., 80., 40., None);
    let c = rect(&mut s, 100., 160., 30., 60., None);
    let d = rect(&mut s, 180., 160., 40., 60., None);
    let ids = [a, b, c, d];
    let layout = selection_spacing(s.doc().unwrap(), &ids);
    assert_eq!(layout.horizontal.unwrap().value, Some(30.));
    assert_eq!(layout.vertical.unwrap().value, Some(20.));
    s.execute("arrange.spacing", &json!({"ids":ids,"horizontal":12,"vertical":25})).unwrap();
    assert_eq!(xy(&s, a), (100., 100.));
    assert_eq!(xy(&s, b), (162., 100.));
    assert_eq!(xy(&s, c), (100., 165.));
    assert_eq!(xy(&s, d), (162., 165.));

    assert_eq!(s.doc().unwrap().node(d).unwrap().width, 40.);
    s.undo().unwrap();
    assert_eq!(xy(&s, d), (180., 160.));
    s.redo().unwrap();
    assert_eq!(xy(&s, d), (162., 165.));
    s.begin_interaction("Spacing").unwrap();
    s.execute("arrange.spacing", &json!({"ids":ids,"horizontal":22,"fromGesture":true})).unwrap();
    s.execute("arrange.spacing", &json!({"ids":ids,"vertical":35,"fromGesture":true})).unwrap();
    assert_eq!(xy(&s, d), (172., 175.), "the unrequested axis keeps the earlier edit");
    s.commit_interaction().unwrap();
}

#[test]
fn mixed_gaps_normalize_and_fractional_negative_gaps_are_supported() {
    let mut s = Session::new();
    let a = rect(&mut s, 0., 0., 50., 40., None);
    let b = rect(&mut s, 70., 0., 50., 40., None);
    let c = rect(&mut s, 160., 0., 50., 40., None);
    assert_eq!(selection_spacing(s.doc().unwrap(), &[a, b, c]).horizontal.unwrap().value, None);
    s.execute("arrange.spacing", &json!({"ids":[a,b,c],"horizontal":-5.5})).unwrap();
    assert_eq!(xy(&s, b), (44.5, 0.));
    assert_eq!(xy(&s, c), (89., 0.));
    assert_eq!(selection_spacing(s.doc().unwrap(), &[a, b, c]).horizontal.unwrap().value, Some(-5.5));
}

#[test]
fn a_scrub_uses_the_original_tracks_after_crossing_and_makes_one_undo_step() {
    let mut s = Session::new();
    let a = rect(&mut s, 0., 0., 50., 40., None);
    let b = rect(&mut s, 70., 0., 50., 40., None);
    let c = rect(&mut s, 140., 0., 50., 40., None);
    let before = s.undo_depth().0;
    s.begin_interaction("Spacing").unwrap();
    for v in [-100., 0., 30.] {
        s.execute("arrange.spacing", &json!({"ids":[a,b,c],"horizontal":v,"fromGesture":true})).unwrap();
    }
    assert_eq!(xy(&s, b), (80., 0.));
    assert_eq!(xy(&s, c), (160., 0.));
    s.commit_interaction().unwrap();
    assert_eq!(s.undo_depth().0, before + 1);
    s.undo().unwrap();
    assert_eq!(xy(&s, c), (140., 0.));
}

#[test]
fn unsafe_selections_and_invalid_axes_are_rejected_without_changes() {
    let mut s = Session::new();
    let a = rect(&mut s, 0., 0., 50., 40., None);
    let b = rect(&mut s, 70., 0., 50., 40., None);
    for props in [json!({"locked":true}), json!({"visible":false})] {
        s.execute("node.set", &json!({"id":b,"props":props})).unwrap();
        assert!(s.execute("arrange.spacing", &json!({"ids":[a,b],"horizontal":5})).is_err());
        s.execute("node.set", &json!({"id":b,"props":{"locked":false,"visible":true}})).unwrap();
    }
    for args in [
        json!({"ids":[a,b],"vertical":5}),
        json!({"ids":[a,b],"horizontal":5,"vertical":5}),
        json!({"ids":[a,b],"horizontal":"bad"}),
        json!({"ids":[a,b],"horizontal":1e300}),
    ] {
        assert!(s.execute("arrange.spacing", &args).is_err());
        assert_eq!(xy(&s, b), (70., 0.));
    }
    let f = NodeId(
        s.execute("frame.create", &json!({"x":0,"y":0,"width":300,"height":100,"props":{"layout":"horizontal"}})).unwrap()["id"].as_u64().unwrap(),
    );
    let x = rect(&mut s, 0., 0., 50., 40., Some(f));
    let y = rect(&mut s, 70., 0., 50., 40., Some(f));
    assert!(selection_spacing(s.doc().unwrap(), &[x, y]).horizontal.is_none());
    assert!(selection_spacing(s.doc().unwrap(), &[a, x]).horizontal.is_none());
}

#[test]
fn rotated_parents_use_page_space_and_cancellation_stays_with_its_document() {
    let mut s = Session::new();
    let f = NodeId(
        s.execute("frame.create", &json!({"x":100,"y":100,"width":300,"height":200,"props":{"rotation":90}})).unwrap()["id"].as_u64().unwrap(),
    );
    let a = rect(&mut s, 0., 0., 50., 40., Some(f));
    let b = rect(&mut s, 70., 0., 50., 40., Some(f));
    let before = xy(&s, b);
    let uid = s.ds().unwrap().uid;
    s.begin_interaction("Spacing").unwrap();
    s.execute("arrange.spacing", &json!({"ids":[a,b],"vertical":40,"fromGesture":true})).unwrap();
    assert!(((xy(&s, b).1 - xy(&s, a).1).abs() - 90.0).abs() < 1e-7, "50-high rotated rectangles must have a 40-point gap");
    s.execute("file.new", &json!({})).unwrap();
    s.begin_interaction("Unrelated").unwrap();
    crate::selection_spacing::cancel_gesture(&mut s, uid);
    assert!(s.ds().unwrap().interaction.is_some());
    s.active = 0;
    assert_eq!(xy(&s, b), before);
    assert!(s.ds().unwrap().interaction.is_none());
}
