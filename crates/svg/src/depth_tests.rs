use super::*;
use figcraft_color::{Color, Paint};

fn nested(depth: usize) -> (Document, NodeId, NodeId) {
    let mut doc = Document::new("export depth");
    let page = doc.first_page().unwrap();
    let root = doc.create(page, NodeKind::Frame, "root", 0.0, 0.0, 32.0, 32.0).unwrap();
    let mut parent = root;
    for _ in 1..depth {
        parent = doc.create(parent, NodeKind::Group, "group", 0.0, 0.0, 32.0, 32.0).unwrap();
    }
    let leaf = doc.create(parent, NodeKind::Rectangle, "deep-leaf", 3.0, 5.0, 13.0, 17.0).unwrap();
    doc.get_mut(leaf).unwrap().fills = vec![Paint::solid(Color::from_hex("FF0000").unwrap())];
    (doc, root, leaf)
}

#[test]
fn export_depth_boundary_keeps_the_leaf() {
    let opts = SvgOptions { include_ids: true, ..Default::default() };
    for depth in [255, 256] {
        let (doc, root, _) = nested(depth);
        let svg = export_svg(&doc, &[root], &opts).unwrap();
        assert!(svg.contains("id=\"deep-leaf\""), "the leaf at depth {depth} was omitted");
        assert!(svg.contains("#FF0000"));
    }
}

#[test]
fn export_depth_overflow_is_refused_and_an_inner_root_still_exports() {
    let (doc, root, leaf) = nested(257);
    let opts = SvgOptions { include_ids: true, ..Default::default() };
    assert!(export_svg(&doc, &[root], &opts).is_none(), "an incomplete SVG must not report success");
    assert_eq!(try_export_svg(&doc, &[root], &opts), Err(SvgError::Depth(ExportDepthError { root, node: leaf })));
    let inner = doc.node(root).unwrap().children[0];
    assert!(export_svg(&doc, &[inner], &opts).unwrap().contains("id=\"deep-leaf\""));
}

#[test]
fn export_depth_checks_all_roots_and_explicit_bounds() {
    let (mut doc, root, leaf) = nested(257);
    let shallow = doc.create(doc.first_page().unwrap(), NodeKind::Rectangle, "shallow", 0.0, 0.0, 8.0, 8.0).unwrap();
    let opts = SvgOptions { bounds: Some(Rect::new(0.0, 0.0, 32.0, 32.0)), ..Default::default() };
    assert!(try_export_svg(&doc, &[shallow], &opts).is_ok());
    assert_eq!(try_export_svg(&doc, &[shallow, root], &opts), Err(SvgError::Depth(ExportDepthError { root, node: leaf })));
}

#[test]
fn export_depth_ignores_hidden_subtrees_and_checks_masks_and_raster_fallbacks() {
    let (mut doc, root, _) = nested(257);
    let inner = doc.node(root).unwrap().children[0];
    doc.get_mut(inner).unwrap().visible = false;
    assert!(try_export_svg(&doc, &[root], &SvgOptions::default()).is_ok());
    doc.get_mut(inner).unwrap().visible = true;
    doc.get_mut(inner).unwrap().is_mask = true;
    assert!(matches!(try_export_svg(&doc, &[root], &SvgOptions::default()), Err(SvgError::Depth(_))));
    doc.get_mut(inner).unwrap().is_mask = false;
    doc.get_mut(root).unwrap().effects = vec![figcraft_doc::Effect { kind: EffectKind::LayerBlur(figcraft_doc::Blur::uniform(1.0)), visible: true }];
    assert!(matches!(try_export_svg(&doc, &[root], &SvgOptions::default()), Err(SvgError::Depth(_))));
}
