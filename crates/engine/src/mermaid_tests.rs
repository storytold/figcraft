//! Pasting Mermaid flowcharts onto a board.

use serde_json::json;

use crate::Session;
use crate::mermaid::{Direction, Shape, looks_like, parse};

const LOGIN: &str = "flowchart TD
    %% a comment
    A([Start]) --> B[Open the app]
    B --> C{Logged in?}
    C -->|yes| D[(Users)]
    C -- no --> E[/Sign in/]
    E -.-> B
    D ==> F((End)) & G[[Audit]]
    style A fill:#f9f
";

#[test]
fn flowcharts_are_read_with_their_shapes_links_and_labels() {
    assert!(looks_like(LOGIN));
    assert!(!looks_like("sequenceDiagram\n A->>B: hi"));
    let c = parse(LOGIN).unwrap();
    assert_eq!(c.direction, Direction::Down);
    let shape = |id: &str| c.nodes.iter().find(|n| n.id == id).map(|n| (n.text.as_str(), n.shape));
    assert_eq!(shape("A"), Some(("Start", Shape::Pill)));
    assert_eq!(shape("C"), Some(("Logged in?", Shape::Decision)));
    assert_eq!(shape("D"), Some(("Users", Shape::Database)));
    assert_eq!(shape("E"), Some(("Sign in", Shape::Data)));
    assert_eq!(shape("F"), Some(("End", Shape::Circle)));
    assert_eq!(shape("G"), Some(("Audit", Shape::Subroutine)));
    let edge = |a: &str, b: &str| c.edges.iter().find(|e| e.from == a && e.to == b).cloned();
    assert_eq!(edge("C", "D").map(|e| e.label), Some("yes".into()));
    assert_eq!(edge("C", "E").map(|e| e.label), Some("no".into()));
    assert!(edge("E", "B").is_some_and(|e| e.dashed));
    assert!(edge("D", "F").is_some_and(|e| e.thick) && edge("D", "G").is_some(), "& links to both");
    assert_eq!(c.edges.len(), 7);
    // One line, left to right, a chain.
    let c = parse("graph LR; a-->b-->c---d").unwrap();
    assert_eq!(c.direction, Direction::Right);
    assert_eq!(c.edges.len(), 3);
    assert!(!c.edges[2].arrow);
}

#[test]
fn junk_never_breaks_the_reader() {
    for t in [
        "",
        "graph",
        "graph TD\n",
        "flowchart TD\n -->",
        "graph TD\nA[unclosed --> B",
        "graph TD\nA -->|unclosed B",
        "graph TD\nA -- never ends",
        "graph TD\n[[[[]]]]",
        "graph TD\nA{{{{",
        "graph TD\nÄ --> 東京[\"quoted\"] --> 🙂",
        "graph TD\nA & & B --> C",
        "graph TD\nA --o B --x C <--> D",
    ] {
        let _ = parse(t);
    }
    // Too big to paste.
    let big: String = std::iter::once("graph TD".to_string()).chain((0..600).map(|i| format!("n{i} --> n{}", i + 1))).collect::<Vec<_>>().join("\n");
    assert!(parse(&big).is_none());
}

#[test]
fn a_pasted_flowchart_becomes_shapes_and_connectors_in_layers() {
    let mut s = Session::new();
    s.execute("file.newBoard", &json!({})).unwrap();
    let r = s.execute("board.pasteMermaid", &json!({"text": LOGIN, "x": 0, "y": 0})).unwrap();
    let ids: Vec<figcraft_doc::NodeId> = r["ids"].as_array().unwrap().iter().map(|v| crate::params::id_of(v).unwrap()).collect();
    assert_eq!(ids.len(), 7);
    assert_eq!(r["connectors"].as_array().unwrap().len(), 7);
    let doc = s.doc().unwrap();
    // Every shape has its words; the decision is a diamond and the database a cylinder.
    for i in &ids {
        assert!(figcraft_doc::shape_text::text_of(doc, *i).is_some());
    }
    assert!(ids.iter().any(|i| matches!(doc.node(*i).unwrap().kind, figcraft_doc::NodeKind::Polygon { count: 4 })));
    assert!(ids.iter().any(|i| crate::board_shapes::preset_of(doc.node(*i).unwrap()).is_some_and(|p| p.id == "cylinder")));
    // Top to bottom: each node below the ones leading to it.
    let y = |i: usize| doc.world_bounds(ids[i]).unwrap().center().y;
    assert!(y(0) < y(1) && y(1) < y(2) && y(2) < y(3), "{:?}", (y(0), y(1), y(2), y(3)));
    // The labelled links carry their labels; one undo step takes it all back.
    let labels = doc
        .nodes()
        .filter(|n| n.text_host.is_some_and(|h| matches!(doc.node(h).map(|x| &x.kind), Some(figcraft_doc::NodeKind::Connector { .. }))))
        .count();
    assert_eq!(labels, 2);
    s.undo().unwrap();
    assert!(!s.doc().unwrap().contains(ids[0]));
    assert!(s.execute("board.pasteMermaid", &json!({"text": "hello"})).is_err());
}
