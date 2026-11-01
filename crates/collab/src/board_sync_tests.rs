//! Boards sync: the file kind reaches joiners in their snapshot, and a layer's board metadata (a
//! connector's ends, a sticky's author) syncs as one property, through both encodings.

use figcraft_doc::{BoardItem, Connector, Document, End, FileKind, NodeKind, Routing, Side};

use crate::change::{Change, DocProp, Prop};
use crate::replica::{Mode, Replica};
use crate::{diff, message};

#[test]
fn a_boards_kind_and_its_objects_metadata_sync() {
    let mut doc = Document::new("Board");
    doc.kind = FileKind::Board;
    let page = doc.first_page().unwrap();
    let a = doc.create(page, NodeKind::Frame, "Sticky", 0.0, 0.0, 200.0, 200.0).unwrap();
    doc.node_mut(a).unwrap().board = Some(BoardItem::Sticky { author: "Ada".into() });
    let host = Replica::new(doc);
    // A joiner's copy, built from the snapshot, is a board with the sticky's metadata.
    let snap = host.snapshot().unwrap();
    assert!(snap.iter().any(|c| matches!(c, Change::SetDoc { prop: DocProp::Kind, .. })));
    let joiner = Replica::from_snapshot(&snap).unwrap();
    assert_eq!(joiner.document().kind, FileKind::Board);
    assert_eq!(joiner.document().node(a).unwrap().board, host.document().node(a).unwrap().board);
    // Re-attaching a connector's end is one property write, the same after the wire.
    let mut edited = host.document().clone();
    let c = edited.create(page, NodeKind::Vector { network: Default::default() }, "Connector", 0.0, 0.0, 1.0, 1.0).unwrap();
    edited.node_mut(c).unwrap().board = Some(BoardItem::Connector(Connector {
        from: End::Node { node: a, side: Side::Auto, at: None },
        to: End::Point { x: 5.0, y: 5.0 },
        routing: Routing::Elbow,
        path: None,
    }));
    let mut r = host.clone();
    r.apply(&diff(&host, &edited).unwrap().changes, Mode::Strict).unwrap();
    let mut moved = r.document().clone();
    moved.node_mut(c).unwrap().board = Some(BoardItem::Connector(Connector {
        from: End::Node { node: a, side: Side::Top, at: None },
        to: End::Point { x: 9.0, y: 1.0 },
        routing: Routing::Straight,
        path: None,
    }));
    let d = diff(&r, &moved).unwrap();
    assert!(matches!(d.changes.as_slice(), [Change::Set { prop: Prop::Board, .. }]), "{:?}", d.changes);
    let wire: Vec<Change> = message::decode(&message::encode(&d.changes).unwrap(), usize::MAX).unwrap();
    r.apply(&wire, Mode::Strict).unwrap();
    assert_eq!(r.document(), &moved);
}
