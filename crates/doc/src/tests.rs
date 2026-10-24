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
        regions: vec![Region { loops: vec![vec![0, 9]], winding: WindingRule::EvenOdd }],
    };
    bad.sanitize();
    assert!(bad.segments.is_empty());
    let _ = bad.fill_path();
}

#[test]
fn variables_resolve_modes_and_aliases() {
    let mut v = Variables::default();
    v.collections.push(Collection {
        id: "c1".into(),
        name: "Theme".into(),
        modes: vec![Mode { id: "light".into(), name: "Light".into() }, Mode { id: "dark".into(), name: "Dark".into() }],
        extends: None,
    });
    let mut vals = std::collections::BTreeMap::new();
    vals.insert("light".to_string(), VariableValue::Color(Color::WHITE));
    vals.insert("dark".to_string(), VariableValue::Color(Color::BLACK));
    v.variables.push(Variable {
        id: "bg".into(),
        name: "bg".into(),
        collection: "c1".into(),
        kind: VariableType::Color,
        values: vals,
        description: String::new(),
        scopes: None,
        hidden_from_publishing: false,
        extends: None,
        overrides: Default::default(),
        code_syntax: Default::default(),
    });
    let mut alias = std::collections::BTreeMap::new();
    alias.insert("light".to_string(), VariableValue::Alias("bg".into()));
    v.variables.push(Variable {
        id: "surface".into(),
        name: "surface".into(),
        collection: "c1".into(),
        kind: VariableType::Color,
        values: alias,
        description: String::new(),
        scopes: None,
        hidden_from_publishing: false,
        extends: None,
        overrides: Default::default(),
        code_syntax: Default::default(),
    });
    let mut modes = std::collections::BTreeMap::new();
    assert_eq!(v.resolve("surface", &modes), Some(VariableValue::Color(Color::WHITE)));
    modes.insert("c1".to_string(), "dark".to_string());
    assert_eq!(v.resolve("bg", &modes), Some(VariableValue::Color(Color::BLACK)));
    // Cycle.
    let mut cyc = std::collections::BTreeMap::new();
    cyc.insert("light".to_string(), VariableValue::Alias("loop".into()));
    v.variables.push(Variable {
        id: "loop".into(),
        name: "loop".into(),
        collection: "c1".into(),
        kind: VariableType::Color,
        values: cyc,
        description: String::new(),
        scopes: None,
        hidden_from_publishing: false,
        extends: None,
        overrides: Default::default(),
        code_syntax: Default::default(),
    });
    assert_eq!(v.resolve("loop", &std::collections::BTreeMap::new()), None);
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
            regions: vec![Region { loops: vec![vec![0, 1, 2, 3], vec![4, 5, 6, 7]], winding: rule }],
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
