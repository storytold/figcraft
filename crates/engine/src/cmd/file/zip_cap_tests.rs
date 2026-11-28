use super::add_member;

#[test]
fn zip_members_stop_at_the_limit_and_keep_what_fit() {
    let mut members = Vec::new();
    add_member(&mut members, "a.png", vec![0; 6], 10).unwrap();
    add_member(&mut members, "b.png", vec![0; 4], 10).unwrap();
    let err = add_member(&mut members, "c.png", vec![0; 1], 10).unwrap_err().to_string();
    assert!(err.contains("as one zip"), "{err}");
    assert_eq!(members.iter().map(|(n, _)| *n).collect::<Vec<_>>(), ["a.png", "b.png"]);
}
