//! `vector.split`: a vector whose pieces don't touch becomes one sibling layer per piece,
//! the original replaced, z-order kept, one undo step.

use figcraft_doc::{NodeKind, Segment, VectorNetwork};
use figcraft_geom::BezPath;
use serde_json::json;

use crate::Session;
use crate::params::id_of;

fn id(v: &serde_json::Value) -> figcraft_doc::NodeId {
    id_of(v).or_else(|| id_of(&v["id"])).unwrap()
}

/// A filled square at (0,0) and a second, disjoint one at (150,0), as one vector layer.
fn two_parts(s: &mut Session) -> figcraft_doc::NodeId {
    let vid = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    let n = s.doc().unwrap().get(vid).unwrap().clone();
    let (path, _) = figcraft_render::node_outline(s.doc().unwrap(), &n).unwrap();
    let mut net = VectorNetwork::from_path(&path);
    net.regions[0].fills = Some(vec![figcraft_color::Paint::solid(figcraft_color::Color::BLACK)]);
    let mut second = VectorNetwork::from_path(&{
        let mut p = BezPath::new();
        p.move_to((150.0, 0.0));
        p.line_to((250.0, 0.0));
        p.line_to((250.0, 100.0));
        p.line_to((150.0, 100.0));
        p.close_path();
        p
    });
    second.regions[0].fills = Some(vec![figcraft_color::Paint::solid(figcraft_color::Color::BLACK)]);
    s.doc_mut().unwrap().get_mut(vid).unwrap().kind = NodeKind::Vector { network: net.clone() };
    {
        let doc = s.doc_mut().unwrap();
        let NodeKind::Vector { network } = &mut doc.get_mut(vid).unwrap().kind else { panic!("expected a vector fixture") };
        let (off, soff) = (network.vertices.len(), network.segments.len());
        network.vertices.extend(second.vertices);
        for sg in second.segments {
            network.segments.push(Segment { a: sg.a + off, b: sg.b + off, ..sg });
        }
        for r in second.regions {
            let mut r = r;
            for l in &mut r.loops {
                for i in l.iter_mut() {
                    *i += soff;
                }
            }
            network.regions.push(r);
        }
    }
    // A vector node's bounds come from its size, so the node spans both squares.
    let bb = {
        let doc = s.doc().unwrap();
        let NodeKind::Vector { network } = &doc.get(vid).unwrap().kind else { panic!("expected a vector fixture") };
        network.bounds().unwrap()
    };
    {
        let n = s.doc_mut().unwrap().get_mut(vid).unwrap();
        n.width = bb.width();
        n.height = bb.height();
    }
    vid
}

#[test]
fn split_separates_disjoint_parts_into_siblings() {
    let mut s = Session::new();
    let vid = two_parts(&mut s);
    let parent = s.doc().unwrap().get(vid).unwrap().parent;
    let idx = s.doc().unwrap().index_in_parent(vid);
    let r = s.execute("vector.split", &json!({"ids": [vid]})).unwrap();
    let parts: Vec<_> = r["ids"].as_array().unwrap().iter().map(id).collect();
    assert_eq!(parts.len(), 2, "two connected pieces become two layers");
    let doc = s.doc().unwrap();
    assert!(!doc.contains(vid), "the original layer is replaced");
    let (a, b) = (doc.world_bounds(parts[0]).unwrap(), doc.world_bounds(parts[1]).unwrap());
    assert_eq!((a.x0, a.x1), (0.0, 100.0));
    assert_eq!((b.x0, b.x1), (150.0, 250.0));
    // The parts sit where the original was, one above the other.
    for (k, &part) in parts.iter().enumerate() {
        let n = doc.node(part).unwrap();
        assert_eq!(n.parent, parent, "each part is a sibling in the same parent");
        assert_eq!(doc.index_in_parent(part), idx.map(|x| x + k), "z-order keeps the original's slot");
        assert_eq!(n.name, doc.node(parts[0]).unwrap().name, "the parts keep the source's name");
    }
    assert_eq!(s.selection(), parts, "the parts are selected");
}

#[test]
fn split_keeps_the_world_place_of_a_rotated_vector() {
    let mut s = Session::new();
    let vid = two_parts(&mut s);
    s.execute("transform.rotate", &json!({"ids": [vid], "degrees": 30.0})).unwrap();
    let before = s.doc().unwrap().world_bounds(vid).unwrap();
    let r = s.execute("vector.split", &json!({"ids": [vid]})).unwrap();
    let parts: Vec<_> = r["ids"].as_array().unwrap().iter().map(id).collect();
    let doc = s.doc().unwrap();
    let union = doc.world_bounds(parts[0]).unwrap().union(doc.world_bounds(parts[1]).unwrap());
    assert!((union.x0 - before.x0).abs() < 1e-6 && (union.y0 - before.y0).abs() < 1e-6, "the parts cover where the vector was");
    assert!((union.x1 - before.x1).abs() < 1e-6 && (union.y1 - before.y1).abs() < 1e-6);
}

#[test]
fn a_connected_vector_refuses_to_split() {
    let mut s = Session::new();
    let vid = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    let n = s.doc().unwrap().get(vid).unwrap().clone();
    let (path, _) = figcraft_render::node_outline(s.doc().unwrap(), &n).unwrap();
    s.doc_mut().unwrap().get_mut(vid).unwrap().kind = NodeKind::Vector { network: VectorNetwork::from_path(&path) };
    let err = s.execute("vector.split", &json!({"ids": [vid]})).unwrap_err();
    assert!(err.to_string().contains("connected"), "{err}");
    assert!(s.doc().unwrap().contains(vid), "a refused split leaves the layer");
}

#[test]
fn an_empty_network_refuses_to_split() {
    let mut s = Session::new();
    let vid = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    s.doc_mut().unwrap().get_mut(vid).unwrap().kind = NodeKind::Vector { network: VectorNetwork::default() };
    assert!(s.execute("vector.split", &json!({"ids": [vid]})).is_err());
    // A layer that isn't a vector at all has nothing to split either.
    let rect = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap());
    assert!(s.execute("vector.split", &json!({"ids": [rect]})).is_err());
}

#[test]
fn split_is_one_undo_step() {
    let mut s = Session::new();
    let vid = two_parts(&mut s);
    let depth_before = s.undo_depth().0;
    s.execute("vector.split", &json!({"ids": [vid]})).unwrap();
    assert_eq!(s.undo_depth(), (depth_before + 1, 0));
    assert!(s.undo().unwrap());
    let doc = s.doc().unwrap();
    assert!(doc.contains(vid), "undo brings the original back");
    let NodeKind::Vector { network } = &doc.get(vid).unwrap().kind else { panic!("expected a vector") };
    assert_eq!(network.vertices.len(), 8, "the two squares are one network again");
}

#[test]
fn split_refuses_unknown_params() {
    let mut s = Session::new();
    assert!(s.execute("vector.split", &json!({"bogus": 1})).is_err());
}
