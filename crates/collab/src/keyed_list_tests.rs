//! Keyed document lists (comments, styles, variables, collections) applied in large batches: a
//! batch of new entries is placed through an index, not a scan of the growing list per entry, and
//! puts, removes and reorders in one batch still leave the list a scan would.

use figcraft_doc::{Comment, Document, Message};

use crate::change::{Change, DocProp, Entry, Json};
use crate::{Mode, Replica};

fn comment(id: u64, text: &str, doc: &Document) -> Comment {
    let page = doc.first_page().unwrap();
    Comment { id, page, x: 1.0, y: 2.0, anchor: None, first: Message::new("A", text, 0), resolved: false, replies: Vec::new(), read_by: None }
}

fn put(c: &Comment) -> Change {
    Change::Put { entry: Entry::Comment(c.id), value: Json::of(c).unwrap() }
}

#[test]
fn a_large_batch_of_new_comments_lands_whole_and_in_order() {
    let doc = Document::new("t");
    let changes: Vec<Change> = (1..=100_000).map(|i| put(&comment(i, "hi", &doc))).collect();
    let mut r = Replica::new(doc);
    // A scan per put took minutes here; indexed it takes well under a second, even unoptimised.
    r.apply(&changes, Mode::Strict).unwrap();
    assert_eq!(r.document().comments.len(), 100_000);
    assert!(r.document().comments.iter().zip(1..).all(|(c, i)| c.id == i));
}

#[test]
fn puts_removes_and_reorders_in_one_batch_match_one_by_one() {
    let doc = Document::new("t");
    let c = |i, t| comment(i, t, &doc);
    let batch = vec![
        put(&c(1, "a")),
        put(&c(2, "b")),
        put(&c(3, "c")),
        // Replaced in place.
        put(&c(2, "B")),
        Change::Remove { entry: Entry::Comment(1) },
        // Absent: nothing changes.
        Change::Remove { entry: Entry::Comment(9) },
        put(&c(4, "d")),
        put(&c(6, "f")),
        // Two removals in a row: the second goes through the index the first kept current.
        Change::Remove { entry: Entry::Comment(2) },
        Change::Remove { entry: Entry::Comment(6) },
        put(&c(2, "B")),
        put(&c(3, "C")),
        Change::SetDoc { prop: DocProp::CommentOrder, value: Json::of(&[4u64, 3, 2]).unwrap() },
        put(&c(2, "BB")),
        put(&c(1, "again")),
        Change::SetDoc { prop: DocProp::Comments, value: Json::of(&vec![c(7, "x"), c(8, "y")]).unwrap() },
        put(&c(8, "Y")),
        put(&c(5, "e")),
    ];
    let mut together = Replica::new(doc.clone());
    together.apply(&batch, Mode::Strict).unwrap();
    let mut one_by_one = Replica::new(doc.clone());
    for ch in &batch {
        one_by_one.apply(std::slice::from_ref(ch), Mode::Strict).unwrap();
    }
    assert_eq!(together.document().comments, one_by_one.document().comments);
    let got: Vec<(u64, &str)> = together.document().comments.iter().map(|c| (c.id, c.first.text.as_str())).collect();
    assert_eq!(got, vec![(7, "x"), (8, "Y"), (5, "e")]);
}

#[test]
fn removing_an_absent_image_does_not_fail() {
    let mut doc = Document::new("t");
    let remove = Change::Remove { entry: Entry::Image("missing".into()) };
    assert!(!crate::docprops::apply(&mut doc, &remove).unwrap());
}

#[test]
fn removals_in_one_batch_keep_the_index_current() {
    let doc = Document::new("t");
    let c = |i, t| comment(i, t, &doc);
    let batch = vec![
        put(&c(1, "a")),
        put(&c(2, "b")),
        put(&c(3, "c")),
        put(&c(4, "d")),
        put(&c(5, "e")),
        Change::Remove { entry: Entry::Comment(2) },
        Change::Remove { entry: Entry::Comment(9) },
        Change::Remove { entry: Entry::Comment(4) },
        // Replaced where the shifted index says they now are.
        put(&c(5, "E")),
        put(&c(3, "C")),
        put(&c(1, "A")),
        Change::Remove { entry: Entry::Comment(1) },
        put(&c(2, "B")),
        put(&c(5, "EE")),
    ];
    let mut together = Replica::new(doc.clone());
    together.apply(&batch, Mode::Strict).unwrap();
    let mut one_by_one = Replica::new(doc.clone());
    for ch in &batch {
        one_by_one.apply(std::slice::from_ref(ch), Mode::Strict).unwrap();
    }
    assert_eq!(together.document().comments, one_by_one.document().comments);
    let got: Vec<(u64, &str)> = together.document().comments.iter().map(|c| (c.id, c.first.text.as_str())).collect();
    assert_eq!(got, vec![(3, "C"), (5, "EE"), (2, "B")]);
}

#[test]
fn duplicate_keys_from_a_whole_list_match_one_by_one() {
    let doc = Document::new("t");
    let c = |i, t| comment(i, t, &doc);
    let batch = vec![
        Change::SetDoc { prop: DocProp::Comments, value: Json::of(&vec![c(1, "a"), c(2, "b"), c(2, "b2"), c(3, "c"), c(4, "d")]).unwrap() },
        // The first entry with the key is the one replaced, as a scan finds it.
        put(&c(2, "B")),
        put(&c(4, "D")),
        // Both entries with the key go: the index is built again for what follows.
        Change::Remove { entry: Entry::Comment(2) },
        put(&c(3, "C")),
        Change::Remove { entry: Entry::Comment(1) },
        put(&c(2, "again")),
        put(&c(4, "DD")),
    ];
    let mut together = Replica::new(doc.clone());
    together.apply(&batch, Mode::Lenient).unwrap();
    let mut one_by_one = Replica::new(doc.clone());
    for ch in &batch {
        one_by_one.apply(std::slice::from_ref(ch), Mode::Lenient).unwrap();
    }
    assert_eq!(together.document().comments, one_by_one.document().comments);
    let got: Vec<(u64, &str)> = together.document().comments.iter().map(|c| (c.id, c.first.text.as_str())).collect();
    assert_eq!(got, vec![(3, "C"), (4, "DD"), (2, "again")]);
}
