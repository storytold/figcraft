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
fn a_large_batch_of_new_comments_applies_in_linear_time() {
    let doc = Document::new("t");
    let changes: Vec<Change> = (1..=100_000).map(|i| put(&comment(i, "hi", &doc))).collect();
    let mut r = Replica::new(doc);
    let started = std::time::Instant::now();
    r.apply(&changes, Mode::Strict).unwrap();
    // A scan per put took minutes here; indexed it takes well under a second, even unoptimised.
    assert!(started.elapsed() < std::time::Duration::from_secs(20), "{:?}", started.elapsed());
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
    assert_eq!(crate::docprops::apply(&mut doc, &remove).unwrap(), false);
}
