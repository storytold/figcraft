use super::*;
use crate::xml::path_data;
use figcraft_doc::BoolOp;
use figcraft_geom::Affine;

#[test]
fn a_rectangle_is_a_drawable_its_size_in_dp() {
    let (mut d, p) = doc();
    let r = rect(&mut d, p, "R", 10.0, 20.0, 30.0, 40.0, "FF0000");
    let out = export(&d, &[r], Compat::BestEffort);
    let els = elements(&out.xml);
    let v = &els[0];
    assert_eq!(v.tag, "vector");
    assert_eq!((v.get("width"), v.get("height")), (Some("30dp"), Some("40dp")));
    assert_eq!((v.get("viewportWidth"), v.get("viewportHeight")), (Some("30"), Some("40")));
    assert!(!out.xml.contains("xmlns:aapt"), "no gradient, no aapt namespace");
    let paths = tagged(&els, "path");
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0].get("fillColor"), Some("#FF0000"));
    assert_eq!(bounds(paths[0].get("pathData").unwrap()), (0.0, 0.0, 30.0, 40.0));
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
}

#[test]
fn colour_alpha_and_paint_opacity_go_into_aarrggbb() {
    let (mut d, p) = doc();
    let r = rect(&mut d, p, "R", 0.0, 0.0, 10.0, 10.0, "00FF0080");
    d.get_mut(r).unwrap().fills[0].opacity *= 0.5;
    let out = export(&d, &[r], Compat::BestEffort);
    // 0x80/255 × 0.5 ≈ 0.251 → 0x40.
    assert_eq!(paths_of(&out.xml)[0].get("fillColor"), Some("#4000FF00"));
}

#[test]
fn several_fills_are_paths_bottom_to_top_and_hidden_ones_are_left_out() {
    let (mut d, p) = doc();
    let r = rect(&mut d, p, "R", 0.0, 0.0, 10.0, 10.0, "FF0000");
    let mut hidden = Paint::solid(hex("0000FF"));
    hidden.visible = false;
    d.get_mut(r).unwrap().fills.extend([Paint::solid(hex("00FF00")), hidden]);
    let out = export(&d, &[r], Compat::BestEffort);
    let fills: Vec<_> = paths_of(&out.xml).iter().map(|e| e.get("fillColor").unwrap().to_string()).collect();
    assert_eq!(fills, ["#FF0000", "#00FF00"]);
}

#[test]
fn shapes_are_their_render_outlines_with_the_transform_baked_in() {
    let (mut d, p) = doc();
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 300.0, 300.0).unwrap();
    d.get_mut(f).unwrap().fills.clear();
    let kinds = [ellipse(), NodeKind::Star { count: 5, inner_ratio: 0.4 }, NodeKind::Polygon { count: 6 }, NodeKind::Rectangle];
    let mut ids = Vec::new();
    for (i, k) in kinds.into_iter().enumerate() {
        let id = d.create(f, k, "S", 20.0 + 60.0 * i as f64, 30.0, 50.0, 40.0).unwrap();
        let n = d.get_mut(id).unwrap();
        n.fills = vec![Paint::solid(hex("336699"))];
        n.corners.radii = [8.0, 4.0, 0.0, 12.0];
        // Rotated, flipped and sheared: none of it fits a <group>, all of it is in the path.
        n.transform *= Affine::new([0.9, 0.3, 0.4, -1.1, 0.0, 40.0]);
        ids.push(id);
    }
    let out = export(&d, &[f], Compat::Ignore);
    let paths = paths_of(&out.xml);
    assert_eq!(paths.len(), ids.len());
    let origin = Affine::translate((0.0, 0.0));
    for (id, el) in ids.iter().zip(&paths) {
        let n = d.node(*id).unwrap();
        let (mut outline, _) = figcraft_render::node_outline(&d, n).unwrap();
        outline.apply_affine(origin * d.world_transform(*id));
        assert_eq!(el.get("pathData").unwrap(), path_data(&outline));
    }
}

#[test]
fn nested_groups_rotations_and_flips_are_baked() {
    let (mut d, p) = doc();
    let g = d.create(p, NodeKind::Group, "G", 0.0, 0.0, 100.0, 100.0).unwrap();
    let r = rect(&mut d, g, "R", 10.0, 10.0, 40.0, 20.0, "000000");
    d.get_mut(g).unwrap().transform = Affine::translate((50.0, 50.0)) * Affine::rotate(0.5) * Affine::scale_non_uniform(-1.0, 2.0);
    let area = figcraft_render::export_bounds(&d, g).unwrap();
    let out = export(&d, &[g], Compat::BestEffort);
    let data = paths_of(&out.xml)[0].get("pathData").unwrap().to_string();
    let n = d.node(r).unwrap();
    let (mut outline, _) = figcraft_render::node_outline(&d, n).unwrap();
    outline.apply_affine(Affine::translate((-area.x0, -area.y0)) * d.world_transform(r));
    assert_eq!(data, path_data(&outline));
    assert!(!out.xml.contains("<group"), "no group needed:\n{}", out.xml);
}

#[test]
fn a_boolean_is_its_result_path() {
    let (mut d, p) = doc();
    let a = rect(&mut d, p, "A", 0.0, 0.0, 60.0, 60.0, "000000");
    let b = d.create(p, ellipse(), "B", 30.0, 30.0, 50.0, 50.0).unwrap();
    let bo = d.create(p, NodeKind::BooleanOp { op: BoolOp::Subtract }, "Bool", 0.0, 0.0, 1.0, 1.0).unwrap();
    d.reparent(a, bo, None).unwrap();
    d.reparent(b, bo, None).unwrap();
    d.fit_to_children(bo).unwrap();
    d.get_mut(bo).unwrap().fills = vec![Paint::solid(hex("CC2244"))];
    let out = export(&d, &[bo], Compat::Exact);
    let paths = paths_of(&out.xml);
    assert_eq!(paths.len(), 1, "operands aren't drawn on their own:\n{}", out.xml);
    assert_eq!(paths[0].get("fillColor"), Some("#CC2244"));
    assert_eq!(paths[0].get("fillType"), Some("evenOdd"), "exact keeps the boolean's even-odd rule");
}

#[test]
fn a_clipping_frame_is_a_group_with_its_clip_path_first() {
    let (mut d, p) = doc();
    let f = rect(&mut d, p, "F", 0.0, 0.0, 100.0, 100.0, "FFFFFF");
    d.get_mut(f).unwrap().kind = NodeKind::Frame;
    d.get_mut(f).unwrap().clip = true;
    d.get_mut(f).unwrap().corners.radii = [10.0; 4];
    rect(&mut d, f, "Over", 50.0, 50.0, 100.0, 100.0, "FF0000");
    let out = export(&d, &[f], Compat::BestEffort);
    let els = elements(&out.xml);
    let tags: Vec<&str> = els.iter().map(|e| e.tag.as_str()).collect();
    assert_eq!(tags, ["vector", "path", "group", "clip-path", "path"], "{}", out.xml);
    let clip = tagged(&els, "clip-path")[0].get("pathData").unwrap();
    assert_eq!(bounds(clip), (0.0, 0.0, 100.0, 100.0));
    assert!(clip.contains('C'), "rounded corners clip too: {clip}");
}

#[test]
fn an_unclipped_frame_draws_its_children_without_a_group() {
    let (mut d, p) = doc();
    let f = d.create(p, NodeKind::Frame, "F", 0.0, 0.0, 100.0, 100.0).unwrap();
    d.get_mut(f).unwrap().clip = false;
    d.get_mut(f).unwrap().fills.clear();
    rect(&mut d, f, "A", 10.0, 10.0, 10.0, 10.0, "FF0000");
    let out = export(&d, &[f], Compat::BestEffort);
    assert!(!out.xml.contains("<group"), "{}", out.xml);
}

#[test]
fn a_vector_mask_clips_the_layers_above_it_and_isnt_drawn() {
    let (mut d, p) = doc();
    let g = d.create(p, NodeKind::Group, "G", 0.0, 0.0, 100.0, 100.0).unwrap();
    let m = d.create(g, ellipse(), "Mask", 10.0, 10.0, 80.0, 80.0).unwrap();
    {
        let n = d.get_mut(m).unwrap();
        n.is_mask = true;
        n.mask_type = figcraft_doc::MaskType::Vector;
        n.fills = vec![Paint::solid(hex("00FF00"))];
    }
    rect(&mut d, g, "Content", 0.0, 0.0, 100.0, 100.0, "2255FF");
    let out = export(&d, &[g], Compat::Exact);
    let els = elements(&out.xml);
    let tags: Vec<&str> = els.iter().map(|e| e.tag.as_str()).collect();
    assert_eq!(tags, ["vector", "group", "clip-path", "path"], "{}", out.xml);
    assert!(!out.xml.contains("#00FF00"), "the mask itself doesn't paint");
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
}

#[test]
fn text_is_glyph_outlines_one_path_per_styled_run() {
    use figcraft_doc::{StyleRun, TextData};
    figcraft_text::scan_system_fonts();
    let (mut d, p) = doc();
    let mut text = TextData::new("ab");
    text.style.font_size = 24.0;
    let mut style = text.style.clone();
    style.fills = Some(vec![Paint::solid(hex("00FF00"))]);
    text.runs.push(StyleRun { start: 1, end: 2, style });
    text.normalize();
    let t = d.create(p, NodeKind::Text(text), "Text", 0.0, 0.0, 100.0, 40.0).unwrap();
    d.get_mut(t).unwrap().fills = vec![Paint::solid(Color::BLACK)];
    let out = export(&d, &[t], Compat::BestEffort);
    let fills: Vec<_> = paths_of(&out.xml).iter().map(|e| e.get("fillColor").unwrap().to_string()).collect();
    assert_eq!(fills, ["#000000", "#00FF00"], "{}", out.xml);
}

#[test]
fn nothing_to_export_is_an_error() {
    let (mut d, p) = doc();
    assert_eq!(try_export(&d, &[], &VdOptions::default()), Err(crate::VdError::Empty));
    let r = rect(&mut d, p, "R", 0.0, 0.0, 10.0, 10.0, "000000");
    let flat = VdOptions { bounds: Some(figcraft_geom::Rect::new(5.0, 5.0, 5.0, 20.0)), ..Default::default() };
    assert_eq!(try_export(&d, &[r], &flat), Err(crate::VdError::Empty));
}

#[test]
fn an_area_shifts_the_viewport_origin() {
    let (mut d, p) = doc();
    let r = rect(&mut d, p, "R", 10.0, 10.0, 10.0, 10.0, "000000");
    let opts = VdOptions { bounds: Some(figcraft_geom::Rect::new(0.0, 0.0, 50.0, 25.0)), ..Default::default() };
    let out = export_with(&d, &[r], opts);
    let els = elements(&out.xml);
    assert_eq!(els[0].get("width"), Some("50dp"));
    assert_eq!(bounds(tagged(&els, "path")[0].get("pathData").unwrap()), (10.0, 10.0, 20.0, 20.0));
}

#[test]
fn depth_past_the_limit_is_refused_and_at_the_limit_kept() {
    use figcraft_render::{ExportDepthError, MAX_EXPORT_DEPTH};
    let nested = |depth: usize| {
        let (mut d, p) = doc();
        let root = d.create(p, NodeKind::Frame, "root", 0.0, 0.0, 32.0, 32.0).unwrap();
        let mut parent = root;
        for _ in 1..depth {
            parent = d.create(parent, NodeKind::Group, "group", 0.0, 0.0, 32.0, 32.0).unwrap();
        }
        let leaf = rect(&mut d, parent, "deep-leaf", 3.0, 5.0, 13.0, 17.0, "FF0000");
        (d, root, leaf)
    };
    let (d, root, _) = nested(MAX_EXPORT_DEPTH);
    assert!(export(&d, &[root], Compat::BestEffort).xml.contains("#FF0000"));
    let (d, root, leaf) = nested(MAX_EXPORT_DEPTH + 1);
    assert_eq!(try_export(&d, &[root], &VdOptions::default()), Err(crate::VdError::Depth(ExportDepthError { root, node: leaf })));
}
