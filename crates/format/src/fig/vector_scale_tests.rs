//! Per-point radii follow the scale of their vector-data row, not the resized layer's box.

use figcraft_doc::{Document, Node, NodeKind, VectorNetwork};

use super::kiwi::Kv;
use super::network::{self, Network, Region, Segment, Vertex};
use super::tests::{arr, at, canvas, e, f, g, node, o, schema, u, xy};

fn vector_data(radius: f64) -> Kv<'static> {
    o(vec![
        ("vectorNetworkBlob", u(0)),
        ("normalizedSize", xy(100.0, 100.0)),
        (
            "styleOverrideTable",
            arr(vec![o(vec![("styleID", u(1)), ("cornerRadius", f(radius))]), o(vec![("styleID", u(2)), ("cornerRadius", f(0.0))])]),
        ),
    ])
}

fn source() -> Vec<Kv<'static>> {
    vec![
        node(0, None, "DOCUMENT", "D", vec![]),
        node(1, Some((0, "!")), "CANVAS", "P", vec![]),
        node(5, Some((1, "!")), "SYMBOL", "Main", vec![("size", xy(100.0, 100.0))]),
        node(
            6,
            Some((5, "!")),
            "VECTOR",
            "Triangle",
            vec![
                ("size", xy(100.0, 100.0)),
                ("vectorData", vector_data(8.0)),
                ("horizontalConstraint", e("SCALE")),
                ("verticalConstraint", e("SCALE")),
            ],
        ),
    ]
}

fn instance(id: u64, parent: u64, main: u64, name: &str, size: f64, factor: Option<f64>, rows: Vec<Kv<'static>>) -> Kv<'static> {
    let mut symbol = vec![("symbolID", g(main)), ("symbolOverrides", arr(rows))];
    if let Some(factor) = factor {
        symbol.push(("uniformScaleFactor", f(factor)));
    }
    node(id, Some((parent, "!")), "INSTANCE", name, vec![("size", xy(size, size)), ("transform", at(200.0, 0.0)), ("symbolData", o(symbol))])
}

fn row(fields: Vec<(&'static str, Kv<'static>)>) -> Kv<'static> {
    let mut values = vec![("guidPath", o(vec![("guids", arr(vec![g(6)]))]))];
    values.extend(fields);
    o(values)
}

fn import(nodes: Vec<Kv<'static>>) -> Document {
    let network = Network {
        vertices: vec![Vertex { style: 1, x: 0.0, y: 0.0 }, Vertex { style: 1, x: 100.0, y: 0.0 }, Vertex { style: 2, x: 0.0, y: 100.0 }],
        segments: (0..3).map(|a| Segment { style: 0, a, b: (a + 1) % 3, ta: (0.0, 0.0), tb: (0.0, 0.0) }).collect(),
        regions: vec![Region { style: 0, non_zero: false, loops: vec![vec![0, 1, 2]] }],
    };
    let message = o(vec![("nodeChanges", arr(nodes)), ("blobs", arr(vec![o(vec![("bytes", Kv::Bytes(network::encode(&network)))])]))]);
    super::import(&canvas(&schema(), &message), "Scaled vectors").unwrap().0
}

fn child<'a>(doc: &'a Document, name: &str) -> &'a Node {
    let parent = doc.nodes().find(|node| node.name == name).unwrap();
    doc.node(parent.children[0]).unwrap()
}

fn network(node: &Node) -> &VectorNetwork {
    let NodeKind::Vector { network } = &node.kind else { panic!("not a vector") };
    network
}

fn assert_vector(node: &Node, size: f64, radius: f64) {
    assert_eq!((node.width, node.height), (size, size));
    let vector = network(node);
    assert_eq!((vector.vertices[1].x, vector.vertices[2].y), (size, size));
    assert_eq!(vector.vertices.iter().map(|point| point.radius).collect::<Vec<_>>(), [radius, radius, 0.0]);
}

#[test]
fn vector_corner_radii_follow_uniform_scale_without_changing_the_main() {
    for (factor, size, radius) in [(None, 100.0, 8.0), (Some(1.0), 100.0, 8.0), (Some(0.5), 50.0, 4.0), (Some(2.0), 200.0, 16.0)] {
        let mut nodes = source();
        nodes.push(instance(9, 1, 5, "Copy", size, factor, vec![]));
        let doc = import(nodes);
        assert_vector(child(&doc, "Main"), 100.0, 8.0);
        assert_vector(child(&doc, "Copy"), size, radius);
    }
}

#[test]
fn vector_corner_radii_from_override_and_derived_rows_are_not_scaled_twice() {
    for derived in [false, true] {
        let mut nodes = source();
        let rows = vec![row(vec![("vectorData", vector_data(3.0))])];
        let mut copy = instance(9, 1, 5, "Copy", 50.0, Some(0.5), if derived { vec![] } else { rows.clone() });
        if derived {
            let Kv::Object(fields) = &mut copy else { panic!() };
            fields.push(("derivedSymbolData", arr(rows)));
        }
        nodes.push(copy);
        let doc = import(nodes);
        assert_vector(child(&doc, "Copy"), 50.0, 3.0);
    }
}

#[test]
fn vector_corner_radii_still_scale_when_only_the_size_is_derived() {
    let mut nodes = source();
    let mut copy = instance(9, 1, 5, "Copy", 50.0, Some(0.5), vec![]);
    let Kv::Object(fields) = &mut copy else { panic!() };
    fields.push(("derivedSymbolData", arr(vec![row(vec![("size", xy(50.0, 50.0))])])));
    nodes.push(copy);
    let doc = import(nodes);
    assert_vector(child(&doc, "Copy"), 50.0, 4.0);
}

#[test]
fn nested_vector_corner_radii_take_the_scale_of_their_own_row() {
    for own_row in [false, true] {
        let mut nodes = source();
        nodes.push(node(20, Some((1, "!")), "SYMBOL", "Wrapper", vec![("size", xy(50.0, 50.0))]));
        let rows = if own_row { vec![row(vec![("vectorData", vector_data(6.0))])] } else { vec![] };
        nodes.push(instance(21, 20, 5, "Inner", 50.0, Some(0.5), rows));
        nodes.push(instance(30, 1, 20, "Outer", 25.0, Some(0.5), vec![]));
        let doc = import(nodes);
        let inner = child(&doc, "Outer");
        assert_vector(doc.node(inner.children[0]).unwrap(), 25.0, if own_row { 3.0 } else { 2.0 });
    }
}

#[test]
fn equally_sized_vectors_with_different_scale_factors_do_not_share_radii() {
    for scaled_first in [false, true] {
        let mut nodes = source();
        // Equal sort positions are ordered by GUID, so vary IDs to exercise both cache orders.
        nodes.push(instance(if scaled_first { 8 } else { 9 }, 1, 5, "Scaled", 50.0, Some(0.5), vec![]));
        nodes.push(instance(if scaled_first { 9 } else { 8 }, 1, 5, "Resized", 50.0, None, vec![]));
        let doc = import(nodes);
        assert_vector(child(&doc, "Scaled"), 50.0, 4.0);
        assert_vector(child(&doc, "Resized"), 50.0, 8.0);
    }
}
