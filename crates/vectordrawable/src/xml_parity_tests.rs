use super::*;
use crate::model::{Grad, GroupItem, GroupXf, NativeStroke, PathItem};
use figcraft_geom::{BezPath, Rect};

fn path(fill: Option<Fill>, cap: &'static str, join: &'static str, c: Color) -> Item {
    let mut data = BezPath::new();
    data.move_to((0.0, -0.0));
    data.line_to((1.23456, 2.0));
    data.quad_to((3.0, 4.0), (5.0, 6.0));
    data.curve_to((7.0, 8.0), (9.0, 10.0), (11.0, 12.0));
    data.close_path();
    Item {
        bbox: Rect::ZERO,
        kind: ItemKind::Path(PathItem {
            name: Some("<&>\"'\t\n\r\0\u{ffff}é𝄞&#9;".into()),
            data,
            fill,
            stroke: Some(NativeStroke { color: c, width: 2.34567, cap, join, miter: 3.0 }),
            even_odd: true,
        }),
    }
}

fn parity(w: f64, h: f64, items: &[Item]) {
    let actual = document(w, h, items.to_vec());
    assert_eq!(actual, legacy::document(w, h, items));
    let parsed = roxmltree::Document::parse(&actual).unwrap();
    assert_eq!(parsed.root_element().tag_name().name(), "vector");
}

#[test]
fn complete_documents_preserve_exact_bytes_and_app_colour_policy() {
    parity(0.0, f64::INFINITY, &[]);
    let colours = [
        Color::BLACK,
        Color::from_rgba8(26, 43, 60, 128),
        Color { r: f32::INFINITY, g: f32::NEG_INFINITY, b: f32::NAN, a: f32::INFINITY },
        Color { r: 0.49999, g: 0.50001, b: 2.0, a: -1.0 },
    ];
    for c in colours {
        let fills = [
            None,
            Some(Fill::Solid(c)),
            Some(Fill::Gradient(Grad { kind: GradKind::Linear { start: (0.0, 1.0), end: (3.0, 4.0) }, stops: vec![(0.0, c), (1.0, Color::WHITE)] })),
            Some(Fill::Gradient(Grad { kind: GradKind::Radial { center: (5.0, 6.0), radius: 7.0 }, stops: vec![(f64::NAN, c)] })),
            Some(Fill::Gradient(Grad { kind: GradKind::Sweep { center: (8.0, 9.0) }, stops: vec![] })),
        ];
        for fill in fills {
            for cap in ["butt", "round", "square"] {
                for join in ["miter", "round", "bevel"] {
                    let leaf = path(fill.clone(), cap, join, c);
                    parity(24.12345, 18.0, std::slice::from_ref(&leaf));
                    let group = Item {
                        bbox: Rect::ZERO,
                        kind: ItemKind::Group(GroupItem {
                            name: Some("group&\"".into()),
                            xf: Some(GroupXf { translate: (2.0, -3.0), rotation: 45.0, scale: (1.0, 2.0) }),
                            clip: Some(BezPath::new()),
                            children: vec![leaf],
                        }),
                    };
                    parity(24.0, 18.0, &[group]);
                }
            }
        }
    }
}

#[test]
fn source_depth_wrappers_do_not_lose_the_deepest_accepted_leaf() {
    let mut leaf = path(Some(Fill::Gradient(Grad { kind: GradKind::Sweep { center: (0.0, 0.0) }, stops: vec![] })), "butt", "miter", Color::BLACK);
    for _ in 0..XML_DEPTH - 2 {
        leaf = Item { bbox: Rect::ZERO, kind: ItemKind::Group(GroupItem { name: None, xf: None, clip: None, children: vec![leaf] }) };
    }
    let items = [leaf];
    let xml = document(1.0, 1.0, items.to_vec());
    assert_eq!(xml, legacy::document(1.0, 1.0, &items));
    assert!(xml.contains("aapt") && xml.contains("<path"));
}

#[test]
fn empty_groups_and_default_attributes_match() {
    for clip in [None, Some(BezPath::new())] {
        let group = Item {
            bbox: Rect::ZERO,
            kind: ItemKind::Group(GroupItem {
                name: None,
                xf: Some(GroupXf { translate: (0.0, -0.0), rotation: -0.0001, scale: (1.0, 1.0) }),
                clip,
                children: vec![],
            }),
        };
        parity(12.0, 12.0, &[group]);
    }
}

#[test]
fn projected_depth_fits_the_browser_sized_stack() {
    std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(|| {
            let mut leaf = path(Some(Fill::Solid(Color::BLACK)), "butt", "miter", Color::BLACK);
            for _ in 0..XML_DEPTH - 2 {
                leaf = Item { bbox: Rect::ZERO, kind: ItemKind::Group(GroupItem { name: None, xf: None, clip: None, children: vec![leaf] }) };
            }
            assert!(document(1.0, 1.0, vec![leaf]).contains("<path"));
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn mixed_siblings_nested_empty_and_clipped_groups_keep_closing_order() {
    let leaf = || path(Some(Fill::Solid(Color::BLACK)), "round", "bevel", Color::WHITE);
    let group = |name: &str, clip, children| Item {
        bbox: Rect::ZERO,
        kind: ItemKind::Group(GroupItem { name: Some(name.into()), xf: None, clip, children }),
    };
    let items = vec![
        leaf(),
        group(
            "outer",
            None,
            vec![
                group("empty", None, vec![]),
                group("clipped-empty", Some(BezPath::new()), vec![]),
                group("inner", Some(BezPath::new()), vec![leaf(), group("deeper", None, vec![leaf()]), leaf()]),
                leaf(),
            ],
        ),
        group("sibling", None, vec![leaf()]),
        leaf(),
    ];
    parity(48.0, 32.0, &items);
}

#[test]
fn over_depth_input_is_discarded_without_recursive_destruction() {
    std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(|| {
            let mut leaf = path(Some(Fill::Solid(Color::BLACK)), "butt", "miter", Color::BLACK);
            for _ in 0..XML_DEPTH * 3 {
                leaf = Item { bbox: Rect::ZERO, kind: ItemKind::Group(GroupItem { name: None, xf: None, clip: None, children: vec![leaf] }) };
            }
            let output = document(1.0, 1.0, vec![leaf]);
            assert_eq!(output.matches("<group").count(), XML_DEPTH);
            assert_eq!(output.matches("</group>").count(), XML_DEPTH - 1);
            assert!(!output.contains("<path"));
        })
        .unwrap()
        .join()
        .unwrap();
}
