//! Ids of styles, variables, collections, modes and comments: unchanged outside a live session,
//! from the client's own space during one.

use crate::ids::{COLLECTION_ID_PREFIX, id_number, id_space, mode_number, space_of};
use crate::{Collection, Comment, Document, Message, Mode, NodeId, NodeKind, Style, StyleKind};

const C3: u64 = 3 << 32;

fn style(id: &str) -> Style {
    Style {
        id: id.into(),
        name: format!("Style {id}"),
        description: String::new(),
        kind: StyleKind::Paint { paints: Vec::new() },
        bound: Default::default(),
    }
}

fn comment(id: u64, page: NodeId) -> Comment {
    Comment { id, page, x: 0.0, y: 0.0, anchor: None, first: Message::new("A", "hi", 0), resolved: false, replies: Vec::new(), read_by: None }
}

fn collection(id: &str, modes: &[&str]) -> Collection {
    Collection { id: id.into(), name: id.into(), modes: modes.iter().map(|m| Mode { id: (*m).into(), name: (*m).into() }).collect(), extends: None }
}

#[test]
fn outside_a_session_ids_are_made_as_before() {
    let mut d = Document::new("a");
    let page = d.first_page().unwrap();
    assert_eq!(d.new_style_id(), "S:1");
    d.styles.push(style("S:1"));
    d.styles.push(style("S:3"));
    // The first free number from the count up.
    assert_eq!(d.new_style_id(), "S:4");
    assert_eq!(d.new_variable_id(), "VariableID:1");
    assert_eq!(d.new_collection_id(), "VariableCollectionId:1");
    d.variables.collections.push(collection("VariableCollectionId:7", &["VariableCollectionId:7/mode1", "VariableCollectionId:7/mode3"]));
    assert_eq!(d.new_collection_id(), "VariableCollectionId:8");
    assert_eq!(d.new_mode_id("VariableCollectionId:7").as_deref(), Some("VariableCollectionId:7/mode4"));
    assert_eq!(d.new_mode_id("nope"), None);
    assert_eq!(d.new_comment_id(), 1);
    d.comments.push(comment(5, page));
    assert_eq!(d.new_comment_id(), 6);
    // Nothing allocated from the layer counter.
    assert_eq!(d.next_id(), NodeId(2));
}

#[test]
fn in_a_session_ids_come_from_the_clients_space_and_never_repeat() {
    let mut d = Document::new("a");
    let page = d.first_page().unwrap();
    d.styles.push(style("S:1"));
    d.set_client_id(3);
    let s1 = d.new_style_id();
    assert_eq!(id_number(&s1, "S:").map(space_of), Some(3), "{s1}");
    d.styles.push(style(&s1));
    let s2 = d.new_style_id();
    assert_ne!(s1, s2);
    // Deleted ids aren't made again.
    d.styles.retain(|s| s.id != s1);
    let s3 = d.new_style_id();
    assert!(s3 != s1 && s3 != s2);
    let v = d.new_variable_id();
    let c = d.new_collection_id();
    assert_eq!(id_space(&v, "VariableID:"), 3);
    assert_eq!(id_space(&c, COLLECTION_ID_PREFIX), 3);
    d.variables.collections.push(collection(&c, &[&format!("{c}/mode1")]));
    let m = d.new_mode_id(&c).unwrap();
    assert_eq!(mode_number(&m).map(space_of), Some(3), "{m}");
    let k = d.new_comment_id();
    assert_eq!(space_of(k), 3);
    d.comments.push(comment(k, page));
    assert!(d.new_comment_id() > k);
    // A layer made next doesn't reuse any of those numbers either.
    let r = d.create(page, NodeKind::Rectangle, "R", 0.0, 0.0, 1.0, 1.0).unwrap();
    assert!(r.0 > k && space_of(r.0) == 3);
}

#[test]
fn ids_of_the_space_already_in_the_document_stay_taken() {
    // A document rebuilt from a snapshot: its allocator hasn't seen this client's styles.
    let mut d = Document::new("a");
    d.styles.push(style(&format!("S:{}", C3 | 40)));
    d.comments.push(comment(C3 | 41, d.first_page().unwrap()));
    d.set_client_id(3);
    assert_eq!(d.new_style_id(), format!("S:{}", C3 | 41));
    assert_eq!(d.new_comment_id(), C3 | 42);
    // Someone else's ids don't move this client's numbers.
    let mut e = Document::new("b");
    e.styles.push(style(&format!("S:{}", (5u64 << 32) | 9)));
    e.set_client_id(3);
    assert_eq!(e.new_style_id(), format!("S:{}", C3 | 1));
}

#[test]
fn the_highest_space_counts_every_kind_of_id() {
    let mut d = Document::new("a");
    assert_eq!(d.highest_id_space(), 0);
    d.styles.push(style(&format!("S:{}", (4u64 << 32) | 1)));
    assert_eq!(d.highest_id_space(), 4);
    d.variables.collections.push(collection("lib:Tokens", &[&format!("lib:Tokens/mode{}", (9u64 << 32) | 2)]));
    assert_eq!(d.highest_id_space(), 9);
    d.comments.push(comment(11u64 << 32, d.first_page().unwrap()));
    assert_eq!(d.highest_id_space(), 11);
    // Numbers past every client's space (they saturate to `u32::MAX`) are no session's.
    d.styles.push(style(&format!("S:{}", u64::MAX)));
    d.comments.push(comment(u64::from(u32::MAX) << 32, d.first_page().unwrap()));
    assert_eq!(d.highest_id_space(), 11);
    // Other forms are space 0.
    assert_eq!(id_space("lib:S:99999999999", "S:"), 0);
    assert_eq!(id_space("S:+5", "S:"), 0);
    assert_eq!(id_space("S:", "S:"), 0);
    assert_eq!(id_space("S:99999999999999999999999", "S:"), 0, "too big for a number");
    assert_eq!(mode_number("1:0"), None);
}

#[test]
fn nodeid_rejects_plus_sign() {
    assert!("+12".parse::<crate::NodeId>().is_err());
    assert!("0:+12".parse::<crate::NodeId>().is_err());
    assert_eq!("12".parse::<crate::NodeId>(), Ok(crate::NodeId(12)));
    assert_eq!("0:12".parse::<crate::NodeId>(), Ok(crate::NodeId(12)));
}
