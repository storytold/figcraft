use super::*;
use crate::tests::inflated;
use figcraft_color::{Color, Paint};
use figcraft_doc::NodeKind;

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
    for depth in [255, 256] {
        let (doc, root, _) = nested(depth);
        let pdf = export_pdf(&doc, &[root], &PdfOptions::default()).unwrap();
        assert!(inflated(&pdf).contains("1 0 0 rg"), "the red leaf at depth {depth} was omitted");
    }
}

#[test]
fn export_depth_overflow_is_refused_by_every_public_entry_point() {
    let (doc, root, leaf) = nested(257);
    let opts = PdfOptions::default();
    let error = || PdfError::Depth(figcraft_render::ExportDepthError { root, node: leaf });
    assert_eq!(export_pdf(&doc, &[root], &opts), Err(error()));
    assert_eq!(export_nodes_pdf(&doc, &[root]), Err(error()));
    assert_eq!(export_groups(&doc, &[vec![root]], &opts), Err(error()));
    let inner = doc.node(root).unwrap().children[0];
    assert!(inflated(&export_pdf(&doc, &[inner], &opts).unwrap()).contains("1 0 0 rg"));
}

#[test]
fn export_depth_checks_every_page_and_explicit_area() {
    let (mut doc, root, leaf) = nested(257);
    let shallow = doc.create(doc.first_page().unwrap(), NodeKind::Rectangle, "shallow", 0.0, 0.0, 8.0, 8.0).unwrap();
    let opts = PdfOptions { area: Some(Rect::new(0.0, 0.0, 32.0, 32.0)), ..Default::default() };
    assert!(export_pdf(&doc, &[shallow], &opts).is_ok());
    assert_eq!(
        export_groups(&doc, &[vec![shallow], vec![root]], &opts),
        Err(PdfError::Depth(figcraft_render::ExportDepthError { root, node: leaf }))
    );
}

#[test]
fn export_depth_ignores_hidden_subtrees_and_checks_masks_and_raster_fallbacks() {
    let (mut doc, root, _) = nested(257);
    let inner = doc.node(root).unwrap().children[0];
    doc.get_mut(inner).unwrap().visible = false;
    assert!(export_pdf(&doc, &[root], &PdfOptions::default()).is_ok());
    doc.get_mut(inner).unwrap().visible = true;
    doc.get_mut(inner).unwrap().is_mask = true;
    assert!(matches!(export_pdf(&doc, &[root], &PdfOptions::default()), Err(PdfError::Depth(_))));
    doc.get_mut(inner).unwrap().is_mask = false;
    doc.get_mut(root).unwrap().effects =
        vec![figcraft_doc::Effect { kind: figcraft_doc::EffectKind::LayerBlur(figcraft_doc::Blur::uniform(1.0)), visible: true }];
    assert!(matches!(export_pdf(&doc, &[root], &PdfOptions::default()), Err(PdfError::Depth(_))));
}
