use crate::{Session, selection_reorder::ReorderLayout};
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
fn insertion_reflows_unequal_sizes_with_original_gaps_and_hierarchy() {
    for vertical in [false, true] {
        let mut s = Session::new();
        let mut ids = vec![];
        for (at, size) in [(0., 40.), (50., 60.), (120., 20.)] {
            ids.push(rect(
                &mut s,
                if vertical { 0. } else { at },
                if vertical { at } else { 0. },
                if vertical { 30. } else { size },
                if vertical { size } else { 30. },
                None,
            ));
        }
        let before = s.doc().unwrap().clone();
        // Explicit selection order must not determine visual order.
        s.execute("arrange.reorder", &json!({"ids":[ids[2],ids[0],ids[1]],"source":ids[0],"target":ids[2]})).unwrap();
        for (id, at) in [(ids[1], 0.), (ids[2], 70.), (ids[0], 100.)] {
            assert_eq!(xy(&s, id), if vertical { (0., at) } else { (at, 0.) });
            let old = before.node(id).unwrap();
            let new = s.doc().unwrap().node(id).unwrap();
            assert_eq!((new.width, new.height, new.parent), (old.width, old.height, old.parent));
        }
        assert_eq!(s.doc().unwrap().node(s.ds().unwrap().page).unwrap().children, before.node(s.ds().unwrap().page).unwrap().children);
        s.undo().unwrap();
        assert_eq!(s.doc().unwrap(), &before);
        s.redo().unwrap();
        assert_eq!(xy(&s, ids[0]), if vertical { (0., 100.) } else { (100., 0.) });
    }
}

#[test]
fn grids_insert_across_rows_and_swap_only_the_two_entries() {
    let mut s = Session::new();
    let mut ids = vec![];
    for row in 0..2 {
        for col in 0..3 {
            ids.push(rect(&mut s, col as f64 * 70., row as f64 * 60., 50., 40., None));
        }
    }
    let before = s.doc().unwrap().clone();
    s.execute("arrange.reorder", &json!({"ids":ids,"source":ids[0],"target":ids[4]})).unwrap();
    assert_eq!(xy(&s, ids[0]), (70., 60.));
    assert_eq!(xy(&s, ids[3]), (140., 0.));
    assert_eq!(xy(&s, ids[5]), (140., 60.));
    s.undo().unwrap();
    s.execute("arrange.reorder", &json!({"ids":ids,"source":ids[0],"target":ids[4],"swap":true})).unwrap();
    assert_eq!(xy(&s, ids[0]), (70., 60.));
    assert_eq!(xy(&s, ids[4]), (0., 0.));
    for id in [ids[1], ids[2], ids[3], ids[5]] {
        assert_eq!(s.doc().unwrap().node(id), before.node(id));
    }
}

#[test]
fn grid_reflow_keeps_gaps_with_different_sizes() {
    let mut s = Session::new();
    let a = rect(&mut s, 0., 0., 40., 30., None);
    let b = rect(&mut s, 50., 0., 60., 30., None);
    let c = rect(&mut s, 0., 50., 40., 50., None);
    let d = rect(&mut s, 50., 50., 60., 50., None);
    s.execute("arrange.reorder", &json!({"ids":[a,b,c,d],"source":a,"target":d})).unwrap();
    assert_eq!(xy(&s, b), (0., 0.));
    assert_eq!(xy(&s, c), (70., 0.));
    assert_eq!(xy(&s, d), (0., 70.));
    assert_eq!(xy(&s, a), (70., 70.));
}

#[test]
fn previews_are_absolute_and_returning_to_start_is_a_noop() {
    let mut s = Session::new();
    let ids: Vec<_> = (0..3).map(|i| rect(&mut s, i as f64 * 70., 0., 50., 40., None)).collect();
    s.execute("select.set", &json!({"ids":ids})).unwrap();
    let before = s.doc().unwrap().clone();
    let history = s.undo_depth().0;
    s.begin_interaction("Reorder selection").unwrap();
    for target in [ids[2], ids[1], ids[0]] {
        s.execute("arrange.reorder", &json!({"ids":ids,"source":ids[0],"target":target,"fromGesture":true})).unwrap();
    }
    s.commit_interaction().unwrap();
    assert_eq!(s.doc().unwrap(), &before);
    assert_eq!(s.undo_depth().0, history);
    s.begin_interaction("Reorder selection").unwrap();
    for target in [ids[1], ids[2]] {
        s.execute("arrange.reorder", &json!({"ids":ids,"source":ids[0],"target":target,"fromGesture":true})).unwrap();
    }
    s.commit_interaction().unwrap();
    assert_eq!(s.undo_depth().0, history + 1);
    assert_eq!(s.selection(), ids);
    s.undo().unwrap();
    assert_eq!(s.doc().unwrap(), &before);
}

#[test]
fn invalid_selections_and_targets_do_not_edit_the_document() {
    let mut s = Session::new();
    let a = rect(&mut s, 0., 0., 50., 40., None);
    let b = rect(&mut s, 70., 0., 50., 40., None);
    let c = rect(&mut s, 160., 0., 50., 40., None);
    for args in [
        json!({"ids":[a,b,c],"source":a,"target":b}), // Mixed gaps.
        json!({"ids":[a,b],"source":c,"target":b}),
        json!({"ids":[a,b],"source":a,"target":999999}),
        json!({"ids":[a,a,b],"source":a,"target":b}),
        json!({"ids":[a,b],"source":[],"target":b}),
    ] {
        let before = s.doc().unwrap().clone();
        assert!(s.execute("arrange.reorder", &args).is_err());
        assert_eq!(s.doc().unwrap(), &before);
    }
    for props in [json!({"locked":true}), json!({"visible":false})] {
        s.execute("node.set", &json!({"id":b,"props":props})).unwrap();
        assert!(ReorderLayout::new(s.doc().unwrap(), &[a, b]).is_none());
        s.execute("node.set", &json!({"id":b,"props":{"locked":false,"visible":true}})).unwrap();
    }
    let d = rect(&mut s, 0., 70., 50., 40., None);
    assert!(ReorderLayout::new(s.doc().unwrap(), &[a, b, d]).is_none(), "an incomplete grid has no slots");
    let frame =
        NodeId(s.execute("frame.create", &json!({"width":300,"height":100,"props":{"layout":"horizontal"}})).unwrap()["id"].as_u64().unwrap());
    let x = rect(&mut s, 0., 0., 50., 40., Some(frame));
    let y = rect(&mut s, 70., 0., 50., 40., Some(frame));
    assert!(ReorderLayout::new(s.doc().unwrap(), &[x, y]).is_none());
    assert!(ReorderLayout::new(s.doc().unwrap(), &[a, x]).is_none());
}

#[test]
fn reordering_children_of_a_rotated_parent_uses_page_coordinates() {
    let mut s = Session::new();
    let frame = NodeId(
        s.execute("frame.create", &json!({"x":200,"y":200,"width":300,"height":100,"props":{"rotation":90}})).unwrap()["id"].as_u64().unwrap(),
    );
    let a = rect(&mut s, 0., 0., 50., 40., Some(frame));
    let b = rect(&mut s, 70., 0., 50., 40., Some(frame));
    let (pa, pb) = (xy(&s, a), xy(&s, b));
    s.execute("arrange.reorder", &json!({"ids":[a,b],"source":a,"target":b})).unwrap();
    let (qa, qb) = (xy(&s, a), xy(&s, b));
    assert!((qa.0 - pb.0).abs() < 1e-7 && (qa.1 - pb.1).abs() < 1e-7);
    assert!((qb.0 - pa.0).abs() < 1e-7 && (qb.1 - pa.1).abs() < 1e-7);
}
