use figcraft_geom::svg_path;

#[test]
fn compact_arc_flags_are_not_part_of_the_next_number() {
    let compact = svg_path("M0 0A5 5 0 11999999999 0").expect("compact flags parse");
    let spaced = svg_path("M0 0A5 5 0 1 1 999999999 0").expect("spaced flags parse");
    assert_eq!(compact, spaced);
}

#[test]
fn a_real_oversized_arc_coordinate_is_still_refused() {
    assert!(svg_path("M0 0A5 5 0 112000000000 0").is_err());
    assert!(svg_path("M0 0A5 5 0 1 1 2000000000 0").is_err());
}
