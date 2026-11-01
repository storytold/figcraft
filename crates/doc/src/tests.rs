use super::*;
use figcraft_geom::Point;

fn page(doc: &Document) -> NodeId {
    doc.first_page().unwrap()
}

#[test]
fn new_document_has_one_page() {
    let d = Document::new("x");
    assert_eq!(d.pages.len(), 1);
    assert_eq!(d.node(page(&d)).unwrap().name, "Page 1");
    assert!(d.validate().is_empty());
}

#[test]
fn create_and_world_transform() {
    let mut d = Document::new("x");
    let p = page(&d);
    let f = d.create(p, NodeKind::Frame, "Frame 1", 100.0, 50.0, 400.0, 300.0).unwrap();
    let r = d.create(f, NodeKind::Rectangle, "Rect", 10.0, 20.0, 50.0, 40.0).unwrap();
    let b = d.world_bounds(r).unwrap();
    assert_eq!((b.x0, b.y0, b.x1, b.y1), (110.0, 70.0, 160.0, 110.0));
    assert!(d.validate().is_empty());
    assert_eq!(d.top_level_frame(r), Some(f));
}

#[test]
fn reparent_keeps_world_position_and_rejects_cycles() {
    let mut d = Document::new("x");
    let p = page(&d);
    let f = d.create(p, NodeKind::Frame, "F", 100.0, 100.0, 400.0, 300.0).unwrap();
    let r = d.create(p, NodeKind::Rectangle, "R", 150.0, 150.0, 10.0, 10.0).unwrap();
    d.reparent(r, f, None).unwrap();
    assert_eq!(d.node(r).unwrap().parent, Some(f));
    let b = d.world_bounds(r).unwrap();
    assert_eq!((b.x0, b.y0), (150.0, 150.0));
    assert!(matches!(d.reparent(f, f, None), Err(DocError::Cycle(..))));
    let g = d.create(f, NodeKind::Group, "G", 0.0, 0.0, 1.0, 1.0).unwrap();
    assert!(matches!(d.reparent(f, g, None), Err(DocError::Cycle(..))));
    let r2 = d.create(p, NodeKind::Rectangle, "R2", 0.0, 0.0, 1.0, 1.0).unwrap();
    assert!(matches!(d.reparent(f, r2, None), Err(DocError::NotContainer(..))));
    assert!(d.validate().is_empty());
}

#[test]
fn reorder_within_parent() {
    let mut d = Document::new("x");
    let p = page(&d);
    let a = d.create(p, NodeKind::Rectangle, "A", 0.0, 0.0, 1.0, 1.0).unwrap();
    let b = d.create(p, NodeKind::Rectangle, "B", 0.0, 0.0, 1.0, 1.0).unwrap();
    let c = d.create(p, NodeKind::Rectangle, "C", 0.0, 0.0, 1.0, 1.0).unwrap();
    d.reparent(a, p, Some(3)).unwrap();
    assert_eq!(d.node(p).unwrap().children, vec![b, c, a]);
    d.reorder(a, 0).unwrap();
    assert_eq!(d.node(p).unwrap().children, vec![a, b, c]);
}

#[test]
fn remove_subtree_and_last_page_guard() {
    let mut d = Document::new("x");
    let p = page(&d);
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 10.0, 10.0).unwrap();
    let r = d.create(f, NodeKind::Rectangle, "R", 0.0, 0.0, 1.0, 1.0).unwrap();
    d.remove(f).unwrap();
    assert!(!d.contains(r));
    assert!(d.remove(p).is_err());
    let p2 = d.add_page("Page 2");
    d.remove(p).unwrap();
    assert_eq!(d.pages, vec![p2]);
}

#[test]
fn group_fits_children() {
    let mut d = Document::new("x");
    let p = page(&d);
    let g = d.create(p, NodeKind::Group, "Group 1", 0.0, 0.0, 0.0, 0.0).unwrap();
    d.create(g, NodeKind::Rectangle, "A", 10.0, 10.0, 20.0, 20.0).unwrap();
    let b = d.create(g, NodeKind::Rectangle, "B", 50.0, 40.0, 10.0, 10.0).unwrap();
    d.fit_to_children(g).unwrap();
    let gb = d.world_bounds(g).unwrap();
    assert_eq!((gb.x0, gb.y0, gb.x1, gb.y1), (10.0, 10.0, 60.0, 50.0));
    let bb = d.world_bounds(b).unwrap();
    assert_eq!((bb.x0, bb.y0), (50.0, 40.0));
}

#[test]
fn duplicate_remaps_ids_and_interactions() {
    let mut d = Document::new("x");
    let p = page(&d);
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 100.0, 100.0).unwrap();
    let a = d.create(f, NodeKind::Rectangle, "A", 0.0, 0.0, 10.0, 10.0).unwrap();
    let b = d.create(f, NodeKind::Rectangle, "B", 20.0, 0.0, 10.0, 10.0).unwrap();
    d.get_mut(a).unwrap().interactions.push(proto::Interaction {
        trigger: proto::Trigger::OnClick,
        actions: vec![proto::Action::Navigate { dest: b, preserve_scroll: false }],
        transition: proto::Transition::Instant,
        duration_ms: 300,
        easing: proto::Easing::EaseOut,
        delay_ms: 0,
    });
    let f2 = d.duplicate_into(f, p, None).unwrap();
    let kids = d.node(f2).unwrap().children.clone();
    assert_eq!(kids.len(), 2);
    assert!(!kids.contains(&a));
    let a2 = d.node(kids[0]).unwrap();
    let proto::Action::Navigate { dest, .. } = a2.interactions[0].actions[0] else { panic!() };
    assert_eq!(dest, kids[1]);
    assert!(d.validate().is_empty());
}

#[test]
fn hit_test_respects_z_and_clip() {
    let mut d = Document::new("x");
    let p = page(&d);
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 100.0, 100.0).unwrap();
    let a = d.create(f, NodeKind::Rectangle, "A", 10.0, 10.0, 50.0, 50.0).unwrap();
    let b = d.create(f, NodeKind::Rectangle, "B", 30.0, 30.0, 50.0, 50.0).unwrap();
    let outside = d.create(f, NodeKind::Rectangle, "Out", 90.0, 90.0, 50.0, 50.0).unwrap();
    assert_eq!(d.hit_test(p, Point::new(40.0, 40.0), 0.0, None), Some(b));
    assert_eq!(d.hit_test(p, Point::new(15.0, 15.0), 0.0, None), Some(a));
    assert_eq!(d.hit_test(p, Point::new(95.0, 95.0), 0.0, None), Some(outside));
    // Clipped part of `outside` isn't hit.
    assert_eq!(d.hit_test(p, Point::new(120.0, 120.0), 0.0, None), None);
    assert_eq!(d.hit_test(p, Point::new(5.0, 95.0), 0.0, None), Some(f));
}

#[test]
fn region_paints_participate_in_colors_images_and_pattern_remapping() {
    let mut d = Document::new("region resources");
    let p = page(&d);
    let group = d.create(p, NodeKind::Group, "Group", 0.0, 0.0, 40.0, 20.0).unwrap();
    let source = d.create(group, NodeKind::Rectangle, "Source", 0.0, 0.0, 4.0, 4.0).unwrap();
    let mut network = VectorNetwork::from_points(&[Point::new(0.0, 0.0), Point::new(20.0, 0.0), Point::new(20.0, 20.0), Point::new(0.0, 20.0)], true);
    network.regions[0].fills = Some(vec![
        figcraft_color::Paint::solid(figcraft_color::Color::from_hex("FF0000").unwrap()),
        figcraft_color::Paint::image("region-image", figcraft_color::ImageScaleMode::Fill),
        figcraft_color::Paint::pattern(source.0),
    ]);
    let vector = d.create(group, NodeKind::Vector { network }, "Vector", 0.0, 0.0, 20.0, 20.0).unwrap();
    assert!(used_images(&d).contains("region-image"));
    let dup = d.duplicate_into(group, p, None).unwrap();
    let copied_children = d.node(dup).unwrap().children.clone();
    let copied_source = copied_children[0];
    let copied_vector = d.node(copied_children[1]).unwrap();
    let NodeKind::Vector { network } = &copied_vector.kind else { panic!() };
    let figcraft_color::PaintKind::Pattern(pattern) = &network.regions[0].fills.as_ref().unwrap()[2].kind else { panic!() };
    assert_eq!(pattern.source, copied_source.0);

    let red = figcraft_color::Color::from_hex("FF0000").unwrap();
    d.map_colors(|c| if c == red { figcraft_color::Color::BLACK } else { c });
    let NodeKind::Vector { network } = &d.node(vector).unwrap().kind else { panic!() };
    assert_eq!(network.regions[0].fills.as_ref().unwrap()[0].solid_color(), Some(figcraft_color::Color::BLACK));
}

#[test]
fn click_target_rules() {
    let mut d = Document::new("x");
    let p = page(&d);
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 200.0, 200.0).unwrap();
    let g = d.create(f, NodeKind::Group, "G", 0.0, 0.0, 0.0, 0.0).unwrap();
    let r = d.create(g, NodeKind::Rectangle, "R", 10.0, 10.0, 20.0, 20.0).unwrap();
    d.fit_to_children(g).unwrap();
    let leaf = d.hit_test(p, Point::new(15.0, 15.0), 0.0, None).unwrap();
    assert_eq!(leaf, r);
    // Child of a top-level frame: the group.
    assert_eq!(geometry::select_target(&d, leaf, &[], false), g);
    // With the group selected, a click enters it.
    assert_eq!(geometry::select_target(&d, leaf, &[r], false), r);
    // Deep select.
    assert_eq!(geometry::select_target(&d, leaf, &[], true), r);
    // Top-level group selects as a whole.
    let g2 = d.create(p, NodeKind::Group, "G2", 0.0, 0.0, 0.0, 0.0).unwrap();
    let r2 = d.create(g2, NodeKind::Rectangle, "R2", 300.0, 0.0, 20.0, 20.0).unwrap();
    d.fit_to_children(g2).unwrap();
    assert_eq!(geometry::select_target(&d, r2, &[], false), g2);
}

#[test]
fn text_runs_restyle_and_replace() {
    let mut t = TextData::new("Hello world");
    t.restyle(0, 5, |s| s.font_style = "Bold".into());
    assert_eq!(t.runs.len(), 1);
    assert_eq!(t.style_at(0).font_style, "Bold");
    assert_eq!(t.style_at(6).font_style, "Regular");
    t.replace_range(5, 5, "!!");
    assert_eq!(t.characters, "Hello!! world");
    // Typed text at the end of a run takes its style.
    assert_eq!(t.style_at(6).font_style, "Bold");
    assert_eq!(t.spans().len(), 2);
    t.restyle(0, 100, |s| s.font_size = 20.0);
    assert!(t.spans().iter().all(|(_, _, s)| s.font_size == 20.0));
    t.restyle(0, 13, |s| s.font_style = "Regular".into());
    assert!(t.is_uniform());
    t.replace_range(0, 99, "");
    assert_eq!(t.characters, "");
}

/// A run that reaches past the text (a file can carry one: nothing clamps `runs` on load)
/// must not overflow the arithmetic that shifts the runs along, and must not leave the
/// text unwritable.
#[test]
fn text_runs_past_the_text_never_overflow() {
    let style = || TextStyle { font_style: "Bold".into(), ..Default::default() };
    for end in [usize::MAX, usize::MAX - 1, 1_000_000] {
        let mut t = TextData::new("hi");
        t.runs = vec![StyleRun { start: 0, end, style: style() }];
        // Typing at the caret, and replacing a selection: both shift `end`.
        t.replace_range(0, 0, "x");
        assert_eq!(t.characters, "xhi", "end {end}");
        t.replace_range(0, 3, "yo");
        assert_eq!(t.characters, "yo", "end {end}");
        // The text is still writable and the runs are back inside it.
        t.replace_range(0, 0, "!");
        assert_eq!(t.characters, "!yo", "end {end}");
        assert!(t.runs.iter().all(|r| r.start < r.end && r.end <= t.char_count()), "end {end}: {:?}", t.runs);
    }
}

#[test]
fn text_case() {
    assert_eq!(TextCase::Title.apply("hello big world"), "Hello Big World");
    assert_eq!(style_weight("Semi Bold Italic"), 600);
    assert_eq!(style_weight("Black"), 900);
    assert_eq!(style_weight("Regular"), 400);
}

#[test]
fn vector_network_paths() {
    let net = VectorNetwork::from_points(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0)], true);
    use figcraft_geom::Shape;
    assert!((net.fill_path().area().abs() - 50.0).abs() < 1e-9);
    let open = VectorNetwork::from_points(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0)], false);
    assert!(open.fill_path().elements().is_empty());
    assert_eq!(open.stroke_path().segments().count(), 2);
    // Round-trip a bezier path.
    let rr = figcraft_geom::rounded_rect(40.0, 20.0, [5.0; 4], 0.0);
    let n2 = VectorNetwork::from_path(&rr);
    assert!((n2.fill_path().area() - rr.area()).abs() < 1e-6);
    let mut bad = VectorNetwork {
        vertices: vec![Vertex::new(0.0, 0.0)],
        segments: vec![Segment::line(0, 5)],
        regions: vec![Region { loops: vec![vec![0, 9]], winding: WindingRule::EvenOdd, fills: None, ..Default::default() }],
    };
    bad.sanitize();
    assert!(bad.segments.is_empty());
    let _ = bad.fill_path();
}

#[test]
fn variables_resolve_modes_and_aliases() {
    let mut v = vars_with_colours(&[("bg", VariableValue::Color(Color::WHITE), VariableValue::Color(Color::BLACK))]);
    add(&mut v, "surface", VariableValue::Alias("bg".into()));
    let mut modes = std::collections::BTreeMap::new();
    assert_eq!(v.resolve("surface", &modes), Some(VariableValue::Color(Color::WHITE)));
    modes.insert("c1".to_string(), "dark".to_string());
    assert_eq!(v.resolve("bg", &modes), Some(VariableValue::Color(Color::BLACK)));
    // Cycle.
    add(&mut v, "loop", VariableValue::Alias("loop".into()));
    assert_eq!(v.resolve("loop", &std::collections::BTreeMap::new()), None);
}

#[test]
fn variables_resolve_stored_ids_never_by_name() {
    let mut v = Variables::default();
    v.collections.push(Collection {
        id: "c1".into(),
        name: "Theme".into(),
        modes: vec![Mode { id: "m".into(), name: "Mode 1".into() }],
        extends: None,
    });
    let var = |id: &str, name: &str, value: VariableValue| colour_var(id, name, &[("m", value)]);
    let red = Color::from_hex("#FF0000").unwrap();
    let blue = Color::from_hex("#0D99FF").unwrap();
    // Named like the next variable's id, and like an id that no longer exists.
    v.variables.push(var("VariableID:1", "VariableID:2", VariableValue::Color(red)));
    v.variables.push(var("VariableID:2", "surface", VariableValue::Color(blue)));
    v.variables.push(var("VariableID:3", "VariableID:9", VariableValue::Color(red)));
    v.variables.push(var("VariableID:4", "alias", VariableValue::Alias("VariableID:9".into())));
    let modes = std::collections::BTreeMap::new();
    assert_eq!(v.resolve("VariableID:2", &modes), Some(VariableValue::Color(blue)));
    // A deleted id stays unresolved instead of picking up the variable named like it.
    assert_eq!(v.resolve("VariableID:9", &modes), None);
    assert_eq!(v.resolve("VariableID:4", &modes), None);
    // What a person types: an id wins over a name, and a name still finds its variable (#1613).
    assert_eq!(v.get("VariableID:2").map(|x| x.name.as_str()), Some("surface"));
    assert_eq!(v.get("surface").map(|x| x.id.as_str()), Some("VariableID:2"));
    assert!(v.get_by_id("surface").is_none());
}

#[test]
fn collections_and_modes_resolve_stored_ids_never_by_name() {
    let mut v = Variables::default();
    let coll = |id: &str, name: &str, modes: Vec<Mode>| Collection { id: id.into(), name: name.into(), modes, extends: None };
    let mode = |id: &str, name: &str| Mode { id: id.into(), name: name.into() };
    v.collections.push(coll("c1", "c2", vec![mode("m1", "m2")]));
    v.collections.push(coll("c2", "Theme", vec![mode("m2", "Dark"), mode("m3", "m9")]));
    assert_eq!(v.collection("c2").map(|c| c.name.as_str()), Some("Theme"), "an id wins over a name");
    assert_eq!(v.collection("Theme").map(|c| c.id.as_str()), Some("c2"));
    assert!(v.collection_by_id("Theme").is_none() && v.collection_by_id("c9").is_none());
    let theme = v.collection_by_id("c2").unwrap();
    assert_eq!(theme.mode_by_id("m2").map(|m| m.name.as_str()), Some("Dark"));
    assert!(theme.mode_by_id("m9").is_none(), "a deleted mode id isn't the mode named like it");
    assert_eq!(theme.find_mode("Dark").map(|m| m.id.as_str()), Some("m2"));
}

#[test]
fn serde_roundtrip_and_repair() {
    let mut d = Document::new("x");
    let p = page(&d);
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 100.0, 100.0).unwrap();
    d.get_mut(f).unwrap().layout = Some(AutoLayout::default());
    d.create(f, NodeKind::Text(TextData::new("Hi")), "Hi", 0.0, 0.0, 10.0, 10.0).unwrap();
    d.create(f, NodeKind::Star { count: 5, inner_ratio: 0.4 }, "S", 0.0, 0.0, 10.0, 10.0).unwrap();
    let j = serde_json::to_string(&d).unwrap();
    let back: Document = serde_json::from_str(&j).unwrap();
    assert_eq!(back, d);
    // Corrupt: dangling child, orphan.
    let mut v: serde_json::Value = serde_json::from_str(&j).unwrap();
    v["nodes"][f.to_string()]["children"].as_array_mut().unwrap().push(serde_json::json!(999));
    let mut bad: Document = serde_json::from_value(v).unwrap();
    assert!(!bad.validate().is_empty());
    bad.repair();
    assert!(bad.validate().is_empty(), "{:?}", bad.validate());
}

#[test]
fn repair_drops_cycles_and_repeated_children() {
    let mut d = Document::new("x");
    let p = page(&d);
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 100.0, 100.0).unwrap();
    let g = d.create(f, NodeKind::Frame, "G", 0.0, 0.0, 10.0, 10.0).unwrap();
    let r = d.create(p, NodeKind::Rectangle, "R", 0.0, 0.0, 10.0, 10.0).unwrap();
    let mut v = serde_json::to_value(&d).unwrap();
    // F and G each other's parent and child (G listing F twice), and R listed twice by the page.
    v["nodes"][f.to_string()]["parent"] = serde_json::json!(g);
    v["nodes"][g.to_string()]["children"] = serde_json::json!([f, f]);
    v["nodes"][p.to_string()]["children"].as_array_mut().unwrap().push(serde_json::json!(r));
    let mut bad: Document = serde_json::from_value(v).unwrap();
    let issues = bad.validate();
    for want in ["listed twice", "not under a page"] {
        assert!(issues.iter().any(|i| i.contains(want)), "{want}: {issues:?}");
    }
    bad.repair();
    assert!(bad.validate().is_empty(), "{:?}", bad.validate());
    assert!(bad.node(f).is_none() && bad.node(g).is_none());
    assert_eq!(bad.node(p).unwrap().children, vec![r]);
    assert_eq!(bad.subtree(p), vec![p, r]);
}

#[test]
fn validate_caps_the_issue_list() {
    let mut d = Document::new("x");
    let p = page(&d);
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 100.0, 100.0).unwrap();
    let g = d.create(f, NodeKind::Frame, "G", 0.0, 0.0, 10.0, 10.0).unwrap();
    let mut v = serde_json::to_value(&d).unwrap();
    v["nodes"][f.to_string()]["children"] = serde_json::json!(vec![g; 5000]);
    let bad: Document = serde_json::from_value(v).unwrap();
    let issues = bad.validate();
    assert!(!issues.is_empty());
    assert!(issues.len() <= 100, "{}", issues.len());
}

#[test]
fn auto_layout_loads_files_without_grid_tracks() {
    // Saved before grid tracks, cells and spans existed.
    let al: AutoLayout = serde_json::from_str(r#"{"mode":"grid","gap":4,"grid_columns":3,"grid_rows":2}"#).unwrap();
    assert_eq!((al.grid_columns, al.column_tracks.len(), al.row_tracks.len()), (3, 0, 0));
    let cl: ChildLayout = serde_json::from_str(r#"{"sizing_h":"fill","absolute":true,"min_w":10}"#).unwrap();
    assert_eq!((cl.grid_column, cl.grid_row, cl.column_span(), cl.row_span()), (None, None, 1, 1));
    // Tracks round-trip; a track missing its value reads as the default kind's value.
    let t: GridTrack = serde_json::from_str(r#"{"sizing":"fixed","value":120}"#).unwrap();
    assert_eq!(t, GridTrack::fixed(120.0));
    assert_eq!(serde_json::from_str::<GridTrack>("{}").unwrap(), GridTrack::fill(1.0));
}

#[test]
fn node_id_parse() {
    assert_eq!("12".parse::<NodeId>().unwrap(), NodeId(12));
    assert_eq!("0:12".parse::<NodeId>().unwrap(), NodeId(12));
    assert!("x".parse::<NodeId>().is_err());
}

#[test]
fn next_name_increments() {
    let mut d = Document::new("x");
    let p = page(&d);
    d.create(p, NodeKind::Frame, "Frame 1", 0.0, 0.0, 1.0, 1.0).unwrap();
    d.create(p, NodeKind::Frame, "Frame 7", 0.0, 0.0, 1.0, 1.0).unwrap();
    assert_eq!(d.next_name("Frame"), "Frame 8");
    assert_eq!(d.next_name("Ellipse"), "Ellipse 1");
}

#[test]
fn next_name_never_overflows() {
    let mut d = Document::new("x");
    let p = page(&d);
    d.create(p, NodeKind::Frame, "Frame 18446744073709551615", 0.0, 0.0, 1.0, 1.0).unwrap();
    assert_eq!(d.next_name("Frame"), "Frame 1", "the counter can't step past u64::MAX");
    d.create(p, NodeKind::Frame, "Frame 1", 0.0, 0.0, 1.0, 1.0).unwrap();
    assert_eq!(d.next_name("Frame"), "Frame 2");
}

#[test]
fn custom_easings() {
    use crate::proto::Easing;
    // cubic-bezier(0.25, 0.1, 0.25, 1) is CSS `ease`: 0.8024 at half time.
    let ease = Easing::CustomBezier { x1: 0.25, y1: 0.1, x2: 0.25, y2: 1.0 };
    assert!((ease.eval(0.5) - 0.8024).abs() < 1e-3, "{}", ease.eval(0.5));
    let linear = Easing::CustomBezier { x1: 0.0, y1: 0.0, x2: 1.0, y2: 1.0 };
    for t in [0.0, 0.1, 0.37, 0.9, 1.0] {
        assert!((linear.eval(t) - t).abs() < 1e-6);
    }
    // Springs start at 0, end at 1; an under-damped one overshoots.
    let bouncy = Easing::CustomSpring { mass: 1.0, stiffness: 300.0, damping: 6.0 };
    assert!(bouncy.eval(0.0).abs() < 1e-9);
    assert_eq!(bouncy.eval(1.0), 1.0);
    assert!((0..100).map(|i| bouncy.eval(f64::from(i) / 100.0)).any(|v| v > 1.01), "overshoots");
    let stiff = Easing::CustomSpring { mass: 1.0, stiffness: 100.0, damping: 40.0 };
    assert!((0..100).map(|i| stiff.eval(f64::from(i) / 100.0)).all(|v| (-1e-9..=1.0 + 1e-9).contains(&v)), "over-damped never overshoots");
    assert!(Easing::CUSTOM_SPRING.is_spring());
    // Junk values stay finite.
    for e in [
        Easing::CustomBezier { x1: f64::NAN, y1: f64::INFINITY, x2: -5.0, y2: 1e300 },
        Easing::CustomSpring { mass: 0.0, stiffness: -1.0, damping: f64::NAN },
        Easing::CustomSpring { mass: 1e300, stiffness: 1e-300, damping: 0.0 },
    ] {
        for i in 0..=20 {
            assert!(e.eval(f64::from(i) / 20.0).is_finite());
        }
    }
    // Serde: presets stay strings, custom curves are objects.
    assert_eq!(serde_json::to_value(Easing::Gentle).unwrap(), serde_json::json!("gentle"));
    let v = serde_json::to_value(Easing::CUSTOM_BEZIER).unwrap();
    assert_eq!(v["customBezier"]["x1"], serde_json::json!(0.42));
    let back: Easing = serde_json::from_value(serde_json::json!({"customSpring": {"mass": 2, "stiffness": 50, "damping": 5}})).unwrap();
    assert_eq!(back, Easing::CustomSpring { mass: 2.0, stiffness: 50.0, damping: 5.0 });
    assert_eq!(serde_json::from_value::<Easing>(serde_json::json!("easeInOut")).unwrap(), Easing::EaseInOut);
}

#[test]
fn text_style_updates_merge_equal_runs() {
    let def = TextStyle { font_size: 14.0, ..Default::default() };
    let mut t = TextData::new("abcd");
    t.link_text_style(0, 2, &def, "S");
    t.link_text_style(2, 4, &def, "S");
    t.restyle(2, 4, |s| s.font_size = 24.0);
    assert_eq!(t.runs.len(), 2);
    t.update_text_style("S", &TextStyle { font_size: 24.0, ..def.clone() });
    let spans = t.spans();
    assert_eq!(spans.len(), 1);
    assert_eq!((spans[0].0, spans[0].1), (0, 4));
    assert_eq!(spans[0].2.font_size, 24.0);
    assert_eq!(spans[0].2.style_id.as_deref(), Some("S"));

    // Remapping two links to one id merges the runs.
    let mut t = TextData::new("ab");
    t.link_text_style(0, 1, &def, "A");
    t.link_text_style(1, 2, &def, "B");
    t.map_text_styles(|_| Some("C".into()));
    assert_eq!(t.spans().len(), 1);
    assert_eq!(t.style_at(1).style_id.as_deref(), Some("C"));

    // Different values stay separate.
    let mut t = TextData::new("ab");
    t.link_text_style(0, 1, &def, "A");
    t.link_text_style(1, 2, &TextStyle { font_size: 30.0, ..Default::default() }, "B");
    t.map_text_styles(|_| Some("C".into()));
    let spans = t.spans();
    assert_eq!(spans.len(), 2);
    assert_eq!((spans[0].2.font_size, spans[1].2.font_size), (14.0, 30.0));

    // Unlinking a run equal to the base drops it.
    let mut t = TextData::new("ab");
    let base = t.style.clone();
    t.link_text_style(0, 1, &base, "S");
    t.map_text_styles(|_| None);
    assert!(t.runs.is_empty());
}

#[test]
fn text_style_empty_range_links_and_unlinks_all() {
    let def = TextStyle { font_style: "Bold".into(), ..Default::default() };
    let mut t = TextData::new("abcd");
    t.link_text_style(2, 2, &def, "S");
    assert!((0..4).all(|i| t.style_at(i).style_id.as_deref() == Some("S") && t.style_at(i).font_style == "Bold"));
    t.unlink_text_style(2, 2);
    assert!((0..4).all(|i| t.style_at(i).style_id.is_none() && t.style_at(i).font_style == "Bold"));
    // Partial ranges stay partial.
    t.link_text_style(0, 4, &def, "S");
    t.unlink_text_style(1, 3);
    assert_eq!(t.style_at(0).style_id.as_deref(), Some("S"));
    assert!(t.style_at(1).style_id.is_none() && t.style_at(2).style_id.is_none());
    assert_eq!(t.style_at(3).style_id.as_deref(), Some("S"));
    // Empty text changes the base style.
    let mut e = TextData::new("");
    e.link_text_style(0, 0, &def, "S");
    assert_eq!(e.style.style_id.as_deref(), Some("S"));
    e.unlink_text_style(0, 0);
    assert!(e.style.style_id.is_none());
}

#[test]
fn stale_next_id_never_overwrites_existing_nodes() {
    let mut d = Document::new("x");
    let p = page(&d);
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 100.0, 100.0).unwrap();
    let a = d.create(f, NodeKind::Rectangle, "A", 0.0, 0.0, 10.0, 10.0).unwrap();
    let mut v = serde_json::to_value(&d).unwrap();
    v["next_id"] = serde_json::json!(1);
    let mut bad: Document = serde_json::from_value(v).unwrap();
    let before: Vec<(NodeId, String, Option<NodeId>)> =
        [p, f, a].iter().map(|i| (*i, bad.node(*i).unwrap().name.clone(), bad.node(*i).unwrap().parent)).collect();
    let n = bad.node_count();
    let copy = bad.duplicate_into(f, p, None).unwrap();
    assert!(![p, f, a].contains(&copy));
    assert_eq!(bad.node_count(), n + 2);
    let copies = bad.duplicate_all_into(&[f], p).unwrap();
    assert_eq!(bad.node_count(), n + 4);
    assert!(copies.iter().all(|c| ![p, f, a, copy].contains(c)));
    for (i, name, parent) in &before {
        let node = bad.node(*i).unwrap();
        assert_eq!((&node.name, node.parent), (name, *parent));
    }
    assert!(bad.validate().is_empty(), "{:?}", bad.validate());

    // A paste whose id map points at an existing node fails and changes nothing.
    let nodes: Vec<Node> = bad.subtree(f).into_iter().filter_map(|i| bad.node(i).cloned()).collect();
    let mut map: BTreeMap<NodeId, NodeId> = nodes.iter().map(|n| (n.id, bad.alloc_id())).collect();
    map.insert(a, p);
    let snapshot = bad.clone();
    assert!(bad.paste_nodes(&nodes, &map, f, p, None).is_err());
    assert_eq!(bad, snapshot);
    assert_eq!(bad.node(p).unwrap().name, snapshot.node(p).unwrap().name);
    assert!(bad.validate().is_empty(), "{:?}", bad.validate());

    // So does one that maps two nodes to the same new id.
    let mut map: BTreeMap<NodeId, NodeId> = nodes.iter().map(|n| (n.id, bad.alloc_id())).collect();
    map.insert(a, map[&f]);
    assert!(bad.paste_nodes(&nodes, &map, f, p, None).is_err());
    assert_eq!(bad, snapshot);
}

#[test]
fn hit_test_even_odd_vector_hole_misses() {
    let ring = |rule| {
        let sq = |o: f64, s: f64, base: usize| {
            (
                [(o, o), (o + s, o), (o + s, o + s), (o, o + s)].map(|(x, y)| Vertex::new(x, y)),
                [Segment::line(base, base + 1), Segment::line(base + 1, base + 2), Segment::line(base + 2, base + 3), Segment::line(base + 3, base)],
            )
        };
        let (v1, s1) = sq(0.0, 200.0, 0);
        let (v2, s2) = sq(60.0, 80.0, 4);
        VectorNetwork {
            vertices: v1.into_iter().chain(v2).collect(),
            segments: s1.into_iter().chain(s2).collect(),
            regions: vec![Region { loops: vec![vec![0, 1, 2, 3], vec![4, 5, 6, 7]], winding: rule, ..Default::default() }],
        }
    };
    let mut d = Document::new("x");
    let p = page(&d);
    let eo = d.create(p, NodeKind::Vector { network: ring(WindingRule::EvenOdd) }, "V", 0.0, 0.0, 200.0, 200.0).unwrap();
    assert_eq!(d.hit_test(p, Point::new(20.0, 100.0), 0.0, None), Some(eo));
    assert_eq!(d.hit_test(p, Point::new(100.0, 100.0), 0.0, None), None, "the hole is not filled");
    let mut d = Document::new("x");
    let p = page(&d);
    let nz = d.create(p, NodeKind::Vector { network: ring(WindingRule::NonZero) }, "V", 0.0, 0.0, 200.0, 200.0).unwrap();
    assert_eq!(d.hit_test(p, Point::new(100.0, 100.0), 0.0, None), Some(nz));
}

#[test]
fn is_ancestor_holds_for_very_deep_chains() {
    let mut d = Document::new("x");
    let root = d.create(page(&d), NodeKind::Frame, "root", 0.0, 0.0, 1.0, 1.0).unwrap();
    let mut leaf = root;
    for _ in 0..10_005 {
        leaf = d.create(leaf, NodeKind::Frame, "n", 0.0, 0.0, 1.0, 1.0).unwrap();
    }
    assert!(d.is_ancestor(root, leaf));
    assert!(matches!(d.reparent(root, leaf, None), Err(DocError::Cycle(..))));
}

#[test]
fn render_bounds_stop_where_drawing_stops_on_very_deep_chains() {
    // A crafted file nests groups far deeper than anything draws (#1062). The leaf counts while
    // it's within the renderer's depth and is left out past it, as the renderer leaves it out.
    let chain = |depth: usize| {
        let mut d = Document::new("x");
        let mut parent = page(&d);
        for _ in 0..depth {
            parent = d.create(parent, NodeKind::Group, "G", 0.0, 0.0, 10.0, 10.0).unwrap();
        }
        d.create(parent, NodeKind::Rectangle, "R", 100.0, 100.0, 10.0, 10.0).unwrap();
        let top = d.node(page(&d)).unwrap().children[0];
        d.world_render_bounds(top).unwrap()
    };
    assert_eq!(chain(MAX_RENDER_DEPTH).x1, 110.0);
    assert_eq!(chain(20_000).x1, 10.0);
}

#[test]
fn text_insert_at_run_boundary_keeps_runs_disjoint() {
    let mut t = TextData::new("0123456789");
    t.restyle(0, 5, |s| s.font_size = 20.0);
    t.restyle(5, 10, |s| s.font_size = 30.0);
    t.replace_range(5, 5, "xx");
    let runs: Vec<(usize, usize)> = t.runs.iter().map(|r| (r.start, r.end)).collect();
    assert_eq!(runs, vec![(0, 7), (7, 12)]);
}

#[test]
fn many_small_vector_subpaths_build_and_stroke_without_rescans() {
    // Many closed subpaths whose last point repeats the start (#F-332), a long open chain (#F-334)
    // and a big rounded polygon (#F-333): each used to scan the whole network per step.
    let mut p = figcraft_geom::kurbo::BezPath::new();
    for i in 0..3_000 {
        let x = f64::from(i) * 10.0;
        p.move_to((x, 0.0));
        p.line_to((x + 5.0, 0.0));
        p.line_to((x + 5.0, 5.0));
        p.line_to((x, 0.0));
        p.close_path();
    }
    let net = VectorNetwork::from_path(&p);
    assert_eq!(net.vertices.len(), 3 * 3_000);
    assert_eq!(net.regions[0].loops.len(), 3_000);
    assert!(net.segments.iter().all(|s| s.a < net.vertices.len() && s.b < net.vertices.len()));
    assert_eq!(net.stroke_runs().len(), 3_000);

    let pts: Vec<Point> = (0..20_000).map(|i| Point::new(f64::from(i), f64::from(i % 7))).collect();
    let chain = VectorNetwork::from_points(&pts, false);
    let runs = chain.stroke_runs();
    assert_eq!(runs.len(), 1);
    assert!(!runs[0].closed);

    let ring: Vec<Point> = (0..4_000)
        .map(|i| {
            let t = f64::from(i) / 4_000.0 * std::f64::consts::TAU;
            Point::new(1000.0 * t.cos(), 1000.0 * t.sin() * if i % 2 == 0 { 1.0 } else { 0.9 })
        })
        .collect();
    let mut poly = VectorNetwork::from_points(&ring, true);
    for v in &mut poly.vertices {
        v.radius = 0.5;
    }
    let rounded = poly.rounded().unwrap();
    assert!(rounded.regions[0].loops[0].len() > poly.segments.len(), "arcs are walked between the sides");
    assert_eq!(rounded.regions[0].loops[0].len(), rounded.segments.len());
}

#[test]
fn two_segment_closed_loop_rounded_corners() {
    let mut v0 = Vertex::new(0.0, 0.0);
    v0.radius = 2.0;
    let mut v1 = Vertex::new(10.0, 0.0);
    v1.radius = 2.0;
    let net = VectorNetwork {
        vertices: vec![v0, v1],
        segments: vec![Segment { a: 0, b: 1, ta: (0.0, 0.0), tb: (0.0, 0.0) }, Segment { a: 1, b: 0, ta: (0.0, 5.0), tb: (0.0, -5.0) }],
        regions: vec![Region { loops: vec![vec![0, 1]], winding: WindingRule::NonZero, ..Default::default() }],
    };
    let r = net.rounded().unwrap();
    assert_eq!(r.regions[0].loops[0].len(), 4, "both arcs are in the loop");
    let path = net.fill_path();
    assert!(!path.elements().is_empty());
}

#[test]
fn stored_variable_ids_resolve_only_their_exact_collection() {
    let mut vars = crate::proto_expr_tests::vars();
    // The wrong collection's modes force the old resolver's first-value fallback (dark).
    let mut shadow = vars.collections[0].clone();
    shadow.id = "shadow".into();
    shadow.name = "c".into();
    shadow.modes = vec![Mode { id: "other".into(), name: "Other".into() }];
    vars.collections.insert(0, shadow);
    assert_eq!(vars.resolve("n", &Default::default()), Some(VariableValue::Number(3.0)));
    vars.collections.retain(|c| c.id != "c");
    assert_eq!(vars.resolve("n", &Default::default()), None);
}

// ——— colour calculations ———

/// A colour variable of collection `c1` with `values` by mode id.
fn colour_var(id: &str, name: &str, values: &[(&str, VariableValue)]) -> Variable {
    Variable {
        id: id.into(),
        name: name.into(),
        collection: "c1".into(),
        kind: VariableType::Color,
        values: values.iter().map(|(m, v)| (m.to_string(), v.clone())).collect(),
        description: String::new(),
        scopes: None,
        hidden_from_publishing: false,
        extends: None,
        overrides: Default::default(),
        code_syntax: Default::default(),
    }
}

/// `c1` with Light/Dark (Light is the default mode) and colour variables `("id", light, dark)`.
fn vars_with_colours(cols: &[(&str, VariableValue, VariableValue)]) -> Variables {
    let mut v = Variables::default();
    v.collections.push(Collection {
        id: "c1".into(),
        name: "C".into(),
        modes: vec![Mode { id: "light".into(), name: "Light".into() }, Mode { id: "dark".into(), name: "Dark".into() }],
        extends: None,
    });
    for (id, light, dark) in cols {
        v.variables.push(colour_var(id, id, &[("light", light.clone()), ("dark", dark.clone())]));
    }
    v
}

/// Adds `id` with a value in the Light mode only.
fn add(v: &mut Variables, id: &str, light: VariableValue) {
    v.variables.push(colour_var(id, id, &[("light", light)]));
}

fn fade(source: &str, opacity: f32) -> VariableValue {
    VariableValue::ColorAlpha { variable: source.into(), opacity }
}
fn mix(from: ColorOperand, to: ColorOperand, t: f32) -> VariableValue {
    VariableValue::ColorMix { from, to, t }
}
fn lit(hex: &str) -> ColorOperand {
    ColorOperand::Literal { literal: Color::from_hex_or_name(hex).unwrap() }
}
fn src(id: &str) -> ColorOperand {
    ColorOperand::Variable { variable: id.into() }
}
fn red() -> VariableValue {
    VariableValue::Color(Color::from_rgb8_const(255, 0, 0))
}
fn resolved(v: &Variables, id: &str, mode: Option<&str>) -> Color {
    let modes: std::collections::BTreeMap<String, String> =
        mode.map(|m| std::collections::BTreeMap::from([("c1".to_string(), m.to_string())])).unwrap_or_default();
    match v.resolve(id, &modes) {
        Some(VariableValue::Color(c)) => c,
        other => panic!("{id} did not resolve to a colour: {other:?}"),
    }
}

#[test]
fn an_alpha_calculation_replaces_alpha_and_follows_its_source() {
    let mut v = vars_with_colours(&[("base", red(), VariableValue::Color(Color::from_rgb8_const(0, 0, 255)))]);
    add(&mut v, "fade", fade("base", 0.5));
    assert_eq!(resolved(&v, "fade", None).to_rgba8(), [255, 0, 0, 128], "default mode: red at half alpha (replace)");
    assert_eq!(resolved(&v, "fade", Some("dark")).to_rgba8(), [0, 0, 255, 128], "the source's value in the chosen mode is what fades");
}

#[test]
fn an_alpha_calculation_replaces_even_a_faded_sources_alpha() {
    // Replace (not scale): a fade of a source that already has alpha lands on exactly the requested
    // opacity, so re-editing the source's alpha cannot compound with the fade's.
    let quarter = VariableValue::Color(Color::new(1.0, 0.0, 0.0, 0.25));
    let mut v = vars_with_colours(&[("base", quarter.clone(), quarter)]);
    add(&mut v, "fade", fade("base", 0.5));
    assert_eq!(resolved(&v, "fade", None).a, 0.5, "not 0.25 * 0.5");
}

#[test]
fn an_alpha_calculation_reads_through_an_alias() {
    let mut v = vars_with_colours(&[("base", red(), red())]);
    add(&mut v, "mirror", VariableValue::Alias("base".into()));
    add(&mut v, "faded", fade("mirror", 0.5));
    assert_eq!(resolved(&v, "faded", None).to_rgba8(), [255, 0, 0, 128]);
}

#[test]
fn a_mix_blends_stored_components_with_exact_edges() {
    let mut v = vars_with_colours(&[("base", red(), VariableValue::Color(Color::from_rgb8_const(0, 0, 255)))]);
    add(&mut v, "t0", mix(src("base"), lit("#FFFFFF"), 0.0));
    add(&mut v, "t1", mix(src("base"), lit("#FFFFFF"), 1.0));
    add(&mut v, "grey", mix(lit("#000000"), lit("#FFFFFF"), 0.4));
    add(&mut v, "afade", mix(lit("#FF0000"), lit("#FF000000"), 0.25));
    assert_eq!(resolved(&v, "t0", None).to_rgba8(), [255, 0, 0, 255], "t=0 is exactly the first operand");
    assert_eq!(resolved(&v, "t1", None).to_rgba8(), [255, 255, 255, 255], "t=1 is exactly the second");
    assert_eq!(resolved(&v, "grey", None).to_rgba8(), [102, 102, 102, 255], "0.4 of white rounds to 102 (0x66)");
    assert_eq!(resolved(&v, "afade", None).to_rgba8(), [255, 0, 0, 191], "alpha blends like the channels (0.75 -> 0xBF)");
    assert_eq!(resolved(&v, "t0", Some("dark")).to_rgba8(), [0, 0, 255, 255], "the variable operand follows the mode");
    // Exact in the stored components too, not only after 8-bit rounding.
    let faint = ColorOperand::Literal { literal: Color::new(1e-8, 0.0, 0.0, 1.0) };
    add(&mut v, "to_faint", mix(lit("#FF0000"), faint.clone(), 1.0));
    add(&mut v, "from_faint", mix(faint, lit("#FF0000"), 0.0));
    assert_eq!(resolved(&v, "to_faint", None).r, 1e-8, "t=1 is the second operand to the last bit");
    assert_eq!(resolved(&v, "from_faint", None).r, 1e-8, "t=0 is the first operand to the last bit");
}

#[test]
fn calculations_compose_and_a_crafted_cycle_terminates() {
    let mut v = vars_with_colours(&[("base", red(), red())]);
    add(&mut v, "fade", fade("base", 0.5));
    add(&mut v, "toast", mix(src("fade"), lit("#FFFFFF"), 0.0));
    assert_eq!(resolved(&v, "toast", None).to_rgba8(), [255, 0, 0, 128], "a calculation may read a calculation");
    // A hand-crafted cycle (commands refuse these) must unwind, never spin.
    add(&mut v, "loop", mix(src("loop2"), lit("#FFFFFF"), 0.5));
    add(&mut v, "loop2", fade("loop", 0.5));
    assert_eq!(v.resolve("loop", &std::collections::BTreeMap::new()), None);
}

#[test]
fn calculation_resolution_is_memoised_and_bounded() {
    // A doubling tree: each level reads the one below twice. Memoised per resolve, that is
    // one pass over the levels; unmemoised it is 2^23 walks (the time bound catches a regression).
    let mut v = vars_with_colours(&[("leaf", red(), red())]);
    let mut prev = "leaf".to_string();
    for i in 0..23 {
        let id = format!("f{i}");
        add(&mut v, &id, mix(src(&prev), src(&prev), 0.5));
        prev = id;
    }
    let started = std::time::Instant::now();
    assert_eq!(resolved(&v, &prev, None).to_rgba8(), [255, 0, 0, 255], "the tree reads the leaf");
    assert!(started.elapsed().as_millis() < 200, "memoised: {:?}", started.elapsed());
    // A diamond: two fades of one base under one mix (shared subtrees each resolved once).
    let quarter = VariableValue::Color(Color::new(1.0, 0.0, 0.0, 0.25));
    let mut d = vars_with_colours(&[("base", quarter.clone(), quarter)]);
    add(&mut d, "fa", fade("base", 0.25));
    add(&mut d, "fb", fade("base", 0.75));
    add(&mut d, "dia", mix(src("fa"), src("fb"), 0.5));
    assert_eq!(resolved(&d, "dia", None).a, 0.5, "0.25 + (0.75 - 0.25) * 0.5");
    // One resolve reads at most 32 variables, however they are chained.
    let mut long = vars_with_colours(&[("v0", red(), red())]);
    for i in 1..=40 {
        add(&mut long, &format!("v{i}"), fade(&format!("v{}", i - 1), 0.99));
    }
    assert_eq!(resolved(&long, "v31", None).to_rgba8(), [255, 0, 0, 252], "32 variables still resolve");
    assert_eq!(long.resolve("v32", &std::collections::BTreeMap::new()), None, "a 33rd reads nothing");
}

#[test]
fn the_colour_walker_visits_calculation_literals() {
    // Document::map_colors (profile conversion) must reach the colours stored inside a mix:
    // leaving them behind would blend converted operands with unconverted ones.
    let mut v = vars_with_colours(&[("base", red(), red())]);
    add(&mut v, "mx", mix(lit("#804020"), src("base"), 0.5));
    let mut doc = Document::new("x");
    doc.variables = v;
    doc.map_colors(|c| c.with_alpha(0.5));
    let mx = doc.variables.variables.iter().find(|x| x.id == "mx").unwrap();
    let VariableValue::ColorMix { from, .. } = &mx.values["light"] else { panic!("still the mix") };
    let ColorOperand::Literal { literal } = from else { panic!("literal operand") };
    assert_eq!(literal.a, 0.5, "the mix's stored literal was converted like every other colour");
}
