//! Structural equality, canonical vector keys, and single-source field mapping for compact
//! `.figcraft` mirror deltas.
//!
//! `Paint` ignores `color_alpha` in `PartialEq`, and IEEE-754 equates `-0.0` with `0.0`.
//! Float and paint fields therefore compare bit-for-bit or via serialized JSON so mirror
//! encoding never drops data that full `document.json` preserves.

use std::collections::{BTreeMap, BTreeSet};

use figcraft_doc::color::{BlendMode, Paint, PaintKind};
use figcraft_doc::geom::Affine;
use figcraft_doc::proto::{Interaction, Overflow, ScrollPosition};
use figcraft_doc::{
    Annotation, AutoLayout, BoardItem, BoolOp, ChildLayout, CodeLink, CollectionId, Constraints, Corners, DevResource, DevStatus, Effect,
    ExportSetting, Guide, LayoutGrid, LetterSpacing, LineHeight, MaskType, Measurement, ModeId, Node, NodeKind, Region, Segment, StrokeProps,
    StyleRefs, TextData, TextStyle, VariableId, VectorNetwork, Vertex,
};
use serde::Serialize;
use serde::de::MapAccess;
use serde::ser::SerializeMap;

/// Defines `MirrorPatch` plus mirror diff-serialization, streaming field deserialization, and
/// patch application from one field table so adding or editing a field keeps all paths in lockstep.
macro_rules! compact_mirror_fields {
    (
        boxed: [ $( $bkey:literal => $bfield:ident : $bty:ty => $beq:ident ),* $(,)? ],
        value: [ $( $vkey:literal => $vfield:ident : $vty:ty => $veq:ident ),* $(,)? ]
    ) => {
        #[derive(Default)]
        pub(super) struct MirrorPatch {
            pub(super) guides: Option<Vec<Guide>>,
            $( pub(super) $bfield: Option<Box<$bty>>, )*
            $( pub(super) $vfield: Option<$vty>, )*
        }

        pub(super) fn serialize_mirror_diff_fields<S: SerializeMap>(
            map: &mut S,
            src: &Node,
            node: &Node,
        ) -> Result<(), S::Error> {
            if !eq_vec_exact(&node.guides, &src.guides) {
                map.serialize_entry("guides", &node.guides)?;
            }
            $(
                if !$beq(&node.$bfield, &src.$bfield) {
                    map.serialize_entry($bkey, &node.$bfield)?;
                }
            )*
            $(
                if !$veq(&node.$vfield, &src.$vfield) {
                    map.serialize_entry($vkey, &node.$vfield)?;
                }
            )*
            Ok(())
        }

        pub(super) fn deserialize_patch_field<'de, A: MapAccess<'de>>(
            key: &str,
            map: &mut A,
            patch: &mut MirrorPatch,
        ) -> Result<bool, A::Error> {
            if key == "guides" {
                let list = map.next_value::<Vec<Guide>>()?;
                patch.guides.get_or_insert_with(Vec::new).extend(list);
                return Ok(true);
            }
            match key {
                $(
                    $bkey => {
                        patch.$bfield = Some(Box::new(map.next_value::<$bty>()?));
                        Ok(true)
                    }
                )*
                $(
                    $vkey => {
                        patch.$vfield = Some(map.next_value::<$vty>()?);
                        Ok(true)
                    }
                )*
                _ => Ok(false),
            }
        }

        pub(super) fn apply_mirror_patch(node: &mut Node, patch: MirrorPatch) {
            if let Some(guides) = patch.guides {
                node.guides = guides;
            }
            $(
                if let Some(v) = patch.$bfield {
                    node.$bfield = *v;
                }
            )*
            $(
                if let Some(v) = patch.$vfield {
                    node.$vfield = v;
                }
            )*
        }

        pub(super) fn apply_full_patch(node: &mut Node, patch: MirrorPatch) {
            apply_mirror_patch(node, patch);
        }
    };
}

compact_mirror_fields! {
    boxed: [
        "stroke" => stroke: StrokeProps => eq_stroke,
        "child_layout" => child_layout: ChildLayout => eq_child_layout,
        "styles" => styles: StyleRefs => eq_plain,
    ],
    value: [
        "name" => name: String => eq_plain,
        "visible" => visible: bool => eq_plain,
        "locked" => locked: bool => eq_plain,
        "opacity" => opacity: f32 => eq_f32,
        "blend" => blend: BlendMode => eq_plain,
        "transform" => transform: Affine => eq_affine,
        "width" => width: f64 => eq_f64,
        "height" => height: f64 => eq_f64,
        "fills" => fills: Vec<Paint> => eq_paint_vec,
        "strokes" => strokes: Vec<Paint> => eq_paint_vec,
        "effects" => effects: Vec<Effect> => eq_vec_exact,
        "corners" => corners: Corners => eq_corners,
        "constraints" => constraints: Constraints => eq_plain,
        "layout" => layout: Option<AutoLayout> => eq_opt_exact,
        "clip" => clip: bool => eq_plain,
        "is_mask" => is_mask: bool => eq_plain,
        "mask_type" => mask_type: MaskType => eq_plain,
        "grids" => grids: Vec<LayoutGrid> => eq_vec_exact,
        "exports" => exports: Vec<ExportSetting> => eq_vec_exact,
        "interactions" => interactions: Vec<Interaction> => eq_vec_exact,
        "overflow" => overflow: Overflow => eq_plain,
        "scroll_position" => scroll_position: ScrollPosition => eq_scroll_position,
        "bound" => bound: BTreeMap<String, VariableId> => eq_plain,
        "var_modes" => var_modes: BTreeMap<CollectionId, ModeId> => eq_plain,
        "scale_factor" => scale_factor: f64 => eq_f64,
        "bool_op" => bool_op: Option<BoolOp> => eq_plain,
        "constrain_proportions" => constrain_proportions: bool => eq_plain,
        "exposed" => exposed: bool => eq_plain,
        "dev_status" => dev_status: DevStatus => eq_plain,
        "dev_changed" => dev_changed: bool => eq_plain,
        "dev_note" => dev_note: String => eq_plain,
        "overrides" => overrides: BTreeSet<String> => eq_plain,
        "prop_refs" => prop_refs: BTreeMap<String, String> => eq_plain,
        "plugin_data" => plugin_data: BTreeMap<String, String> => eq_plain,
        "annotations" => annotations: Vec<Annotation> => eq_vec_exact,
        "code_links" => code_links: Vec<CodeLink> => eq_plain,
        "dev_resources" => dev_resources: Vec<DevResource> => eq_plain,
        "measurements" => measurements: Vec<Measurement> => eq_vec_exact,
        "board" => board: Option<BoardItem> => eq_opt_exact,
    ]
}

/// Fails to compile if a new field is added to `figcraft_doc::Node` without updating
/// `compact_mirror_fields!`.
#[allow(dead_code)]
fn every_node_field_visited(n: &Node) {
    let Node {
        id: _,
        name: _,
        kind: _,
        parent: _,
        children: _,
        visible: _,
        locked: _,
        opacity: _,
        blend: _,
        transform: _,
        width: _,
        height: _,
        fills: _,
        strokes: _,
        stroke: _,
        effects: _,
        corners: _,
        constraints: _,
        layout: _,
        child_layout: _,
        clip: _,
        is_mask: _,
        mask_type: _,
        grids: _,
        guides: _,
        exports: _,
        interactions: _,
        overflow: _,
        scroll_position: _,
        styles: _,
        bound: _,
        var_modes: _,
        scale_factor: _,
        bool_op: _,
        constrain_proportions: _,
        source: _,
        exposed: _,
        dev_status: _,
        dev_changed: _,
        dev_note: _,
        overrides: _,
        prop_refs: _,
        plugin_data: _,
        annotations: _,
        code_links: _,
        dev_resources: _,
        measurements: _,
        board: _,
    } = n;
}

use sha2::Digest;

/// Streaming SHA-256 fingerprint of a finite non-empty `VectorNetwork` without buffering its JSON.
pub(super) fn vector_fingerprint(net: &VectorNetwork) -> Option<[u8; 32]> {
    struct HashWriter(sha2::Sha256);
    impl std::io::Write for HashWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.update(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    if net.vertices.is_empty() || !is_finite_network(net) {
        return None;
    }
    let mut w = HashWriter(sha2::Sha256::new());
    serde_json::to_writer(&mut w, net).ok()?;
    Some(w.0.finalize().into())
}

/// Whether every float in `net` is finite (non-finite networks stay inline so `FiniteValue`
/// sanitizes them directly).
pub(super) fn is_finite_network(net: &VectorNetwork) -> bool {
    let verts_ok = net.vertices.iter().all(|v| v.x.is_finite() && v.y.is_finite() && v.radius.is_finite());
    let segs_ok = net.segments.iter().all(|s| s.ta.0.is_finite() && s.ta.1.is_finite() && s.tb.0.is_finite() && s.tb.1.is_finite());
    let regs_ok = net.regions.iter().all(is_finite_region);
    verts_ok && segs_ok && regs_ok
}

fn is_finite_region(r: &Region) -> bool {
    let space_ok =
        r.paint_space.is_none_or(|ps| ps.width.is_finite() && ps.height.is_finite() && ps.transform.as_coeffs().iter().all(|c| c.is_finite()));
    let fills_ok = r.fills.as_ref().is_none_or(|fs| fs.iter().all(is_finite_paint));
    space_ok && fills_ok
}

fn is_finite_paint(p: &Paint) -> bool {
    if !p.opacity.is_finite() || !p.color_alpha.is_finite() {
        return false;
    }
    match &p.kind {
        PaintKind::Solid { color } => color.r.is_finite() && color.g.is_finite() && color.b.is_finite() && color.a.is_finite(),
        _ => serde_json::to_vec(&p.kind).is_ok(),
    }
}

fn est_paint_bytes(p: &Paint) -> usize {
    let extra = match &p.kind {
        PaintKind::Gradient(g) => g.stops.len().saturating_mul(32),
        PaintKind::Image(img) => img.image.len(),
        PaintKind::Solid { .. } | PaintKind::Pattern(_) => 0,
    };
    std::mem::size_of::<Paint>().saturating_add(extra)
}

fn est_paint_slice_bytes(paints: &[Paint]) -> usize {
    paints.iter().fold(0usize, |acc, p| acc.saturating_add(est_paint_bytes(p)))
}

/// Conservative heap byte estimate for one `VectorNetwork`.
pub(super) fn est_vector_bytes(net: &VectorNetwork) -> usize {
    let loops: usize = net.regions.iter().map(|r| r.loops.iter().map(|l| l.len().saturating_mul(8)).sum::<usize>()).sum();
    let region_fills: usize = net.regions.iter().filter_map(|r| r.fills.as_deref()).map(est_paint_slice_bytes).fold(0usize, usize::saturating_add);
    net.vertices
        .len()
        .saturating_mul(std::mem::size_of::<Vertex>())
        .saturating_add(net.segments.len().saturating_mul(std::mem::size_of::<Segment>()))
        .saturating_add(net.regions.len().saturating_mul(std::mem::size_of::<Region>()))
        .saturating_add(loops)
        .saturating_add(region_fills)
}

/// Conservative heap byte estimate for one materialized `Node`.
pub(super) fn est_node_bytes(node: &Node) -> usize {
    let kind_extra = match &node.kind {
        NodeKind::Vector { network } => est_vector_bytes(network),
        NodeKind::Text(t) => t
            .characters
            .len()
            .saturating_add(t.runs.len().saturating_mul(std::mem::size_of::<figcraft_doc::StyleRun>()))
            .saturating_add(t.paragraphs.len().saturating_mul(16)),
        NodeKind::Component { description, props, prop_order } => {
            description.len().saturating_add(props.len().saturating_mul(64)).saturating_add(prop_order.len().saturating_mul(24))
        }
        NodeKind::Instance { props, .. } => props.len().saturating_mul(64),
        _ => 0,
    };
    let bound_bytes: usize = node.bound.iter().map(|(k, v)| k.len().saturating_add(v.len()).saturating_add(64)).sum();
    let var_modes_bytes: usize = node.var_modes.iter().map(|(k, v)| k.len().saturating_add(v.len()).saturating_add(64)).sum();
    let plugin_bytes: usize = node.plugin_data.iter().map(|(k, v)| k.len().saturating_add(v.len()).saturating_add(48)).sum();
    let prop_refs_bytes: usize = node.prop_refs.iter().map(|(k, v)| k.len().saturating_add(v.len()).saturating_add(48)).sum();
    let ann_bytes: usize = node
        .annotations
        .iter()
        .map(|a| a.label.len().saturating_add(a.properties.iter().map(String::len).sum::<usize>()).saturating_add(std::mem::size_of::<Annotation>()))
        .sum();
    let links_bytes: usize = node
        .code_links
        .iter()
        .map(|c| {
            c.language
                .len()
                .saturating_add(c.import_path.len())
                .saturating_add(c.name.len())
                .saturating_add(c.template.len())
                .saturating_add(c.source.len())
                .saturating_add(c.notes.len())
                .saturating_add(std::mem::size_of::<CodeLink>())
        })
        .sum();
    let resources_bytes: usize =
        node.dev_resources.iter().map(|r| r.url.len().saturating_add(r.name.len()).saturating_add(std::mem::size_of::<DevResource>())).sum();
    std::mem::size_of::<Node>()
        .saturating_add(node.name.len())
        .saturating_add(node.dev_note.len())
        .saturating_add(node.children.len().saturating_mul(8))
        .saturating_add(est_paint_slice_bytes(&node.fills))
        .saturating_add(est_paint_slice_bytes(&node.strokes))
        .saturating_add(node.effects.len().saturating_mul(std::mem::size_of::<Effect>()))
        .saturating_add(node.overrides.iter().map(|s| s.len().saturating_add(32)).sum::<usize>())
        .saturating_add(bound_bytes)
        .saturating_add(var_modes_bytes)
        .saturating_add(plugin_bytes)
        .saturating_add(prop_refs_bytes)
        .saturating_add(ann_bytes)
        .saturating_add(links_bytes)
        .saturating_add(resources_bytes)
        .saturating_add(kind_extra)
}

#[inline]
fn eq_plain<T: PartialEq>(a: &T, b: &T) -> bool {
    a == b
}

#[inline]
fn eq_f64(a: &f64, b: &f64) -> bool {
    a.to_bits() == b.to_bits()
}

#[inline]
fn eq_f32(a: &f32, b: &f32) -> bool {
    a.to_bits() == b.to_bits()
}

#[inline]
fn eq_affine(a: &Affine, b: &Affine) -> bool {
    a.as_coeffs().iter().zip(b.as_coeffs()).all(|(x, y)| x.to_bits() == y.to_bits())
}

fn eq_opt_f64(a: Option<f64>, b: Option<f64>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => eq_f64(&x, &y),
        _ => false,
    }
}

fn eq_corners(a: &Corners, b: &Corners) -> bool {
    a.radii.iter().zip(&b.radii).all(|(x, y)| eq_f64(x, y)) && eq_f64(&a.smoothing, &b.smoothing)
}

fn eq_child_layout(a: &ChildLayout, b: &ChildLayout) -> bool {
    a.sizing_h == b.sizing_h
        && a.sizing_v == b.sizing_v
        && a.absolute == b.absolute
        && eq_opt_f64(a.min_w, b.min_w)
        && eq_opt_f64(a.max_w, b.max_w)
        && eq_opt_f64(a.min_h, b.min_h)
        && eq_opt_f64(a.max_h, b.max_h)
        && a.grid_column == b.grid_column
        && a.grid_row == b.grid_row
        && a.grid_column_span == b.grid_column_span
        && a.grid_row_span == b.grid_row_span
}

fn eq_scroll_position(a: &ScrollPosition, b: &ScrollPosition) -> bool {
    a == b
}

fn eq_stroke(a: &StrokeProps, b: &StrokeProps) -> bool {
    if !eq_f64(&a.weight, &b.weight)
        || a.align != b.align
        || a.cap_start != b.cap_start
        || a.cap_end != b.cap_end
        || a.join != b.join
        || !eq_f64(&a.miter_limit, &b.miter_limit)
        || a.dash_cap != b.dash_cap
        || a.dashes.len() != b.dashes.len()
        || !a.dashes.iter().zip(&b.dashes).all(|(x, y)| eq_f64(x, y))
    {
        return false;
    }
    let sides_ok = match (a.sides, b.sides) {
        (None, None) => true,
        (Some(x), Some(y)) => x.iter().zip(&y).all(|(l, r)| eq_f64(l, r)),
        _ => false,
    };
    sides_ok && eq_opt_exact(&a.width_profile, &b.width_profile) && eq_opt_exact(&a.brush, &b.brush)
}

fn eq_json<T: Serialize + ?Sized>(a: &T, b: &T) -> bool {
    matches!((serde_json::to_vec(a), serde_json::to_vec(b)), (Ok(x), Ok(y)) if x == y)
}

fn eq_exact<T: Serialize + PartialEq>(a: &T, b: &T) -> bool {
    a == b && eq_json(a, b)
}

fn eq_vec_exact<T: Serialize + PartialEq>(a: &[T], b: &[T]) -> bool {
    (a.is_empty() && b.is_empty()) || (a == b && eq_json(a, b))
}

fn eq_opt_exact<T: Serialize + PartialEq>(a: &Option<T>, b: &Option<T>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => x == y && eq_json(x, y),
        _ => false,
    }
}

fn eq_paint(a: &Paint, b: &Paint) -> bool {
    if a.visible != b.visible || a.blend != b.blend || !eq_f32(&a.opacity, &b.opacity) || !eq_f32(&a.color_alpha, &b.color_alpha) {
        return false;
    }
    match (&a.kind, &b.kind) {
        (PaintKind::Solid { color: ca }, PaintKind::Solid { color: cb }) => {
            eq_f32(&ca.r, &cb.r) && eq_f32(&ca.g, &cb.g) && eq_f32(&ca.b, &cb.b) && eq_f32(&ca.a, &cb.a)
        }
        _ => a.kind == b.kind && eq_json(&a.kind, &b.kind),
    }
}

fn eq_paint_vec(a: &[Paint], b: &[Paint]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| eq_paint(x, y))
}

fn eq_vertex(a: &Vertex, b: &Vertex) -> bool {
    eq_f64(&a.x, &b.x) && eq_f64(&a.y, &b.y) && eq_f64(&a.radius, &b.radius) && a.mirror == b.mirror && a.cap == b.cap && a.join == b.join
}

fn eq_segment(a: &Segment, b: &Segment) -> bool {
    a.a == b.a && a.b == b.b && eq_f64(&a.ta.0, &b.ta.0) && eq_f64(&a.ta.1, &b.ta.1) && eq_f64(&a.tb.0, &b.tb.0) && eq_f64(&a.tb.1, &b.tb.1)
}

fn eq_region(a: &Region, b: &Region) -> bool {
    let fills_ok = match (&a.fills, &b.fills) {
        (None, None) => true,
        (Some(fa), Some(fb)) => eq_paint_vec(fa, fb),
        _ => false,
    };
    let space_ok = match (a.paint_space, b.paint_space) {
        (None, None) => true,
        (Some(pa), Some(pb)) => eq_f64(&pa.width, &pb.width) && eq_f64(&pa.height, &pb.height) && eq_affine(&pa.transform, &pb.transform),
        _ => false,
    };
    a.winding == b.winding && a.loops == b.loops && fills_ok && space_ok
}

pub(super) fn exact_vector_network_eq(a: &VectorNetwork, b: &VectorNetwork) -> bool {
    a.vertices.len() == b.vertices.len()
        && a.segments.len() == b.segments.len()
        && a.regions.len() == b.regions.len()
        && a.vertices.iter().zip(&b.vertices).all(|(x, y)| eq_vertex(x, y))
        && a.segments.iter().zip(&b.segments).all(|(x, y)| eq_segment(x, y))
        && a.regions.iter().zip(&b.regions).all(|(x, y)| eq_region(x, y))
}

fn eq_line_height(a: LineHeight, b: LineHeight) -> bool {
    match (a, b) {
        (LineHeight::Auto, LineHeight::Auto) => true,
        (LineHeight::Pixels(x), LineHeight::Pixels(y)) | (LineHeight::Percent(x), LineHeight::Percent(y)) => eq_f64(&x, &y),
        _ => false,
    }
}

fn eq_letter_spacing(a: LetterSpacing, b: LetterSpacing) -> bool {
    match (a, b) {
        (LetterSpacing::Pixels(x), LetterSpacing::Pixels(y)) | (LetterSpacing::Percent(x), LetterSpacing::Percent(y)) => eq_f64(&x, &y),
        _ => false,
    }
}

fn eq_text_style(a: &TextStyle, b: &TextStyle) -> bool {
    if a.font_family != b.font_family
        || a.font_style != b.font_style
        || !eq_f64(&a.font_size, &b.font_size)
        || !eq_line_height(a.line_height, b.line_height)
        || !eq_letter_spacing(a.letter_spacing, b.letter_spacing)
        || a.decoration != b.decoration
        || a.case != b.case
        || a.position != b.position
        || a.link != b.link
        || a.style_id != b.style_id
        || a.variations.len() != b.variations.len()
        || !a.variations.iter().zip(&b.variations).all(|((ka, va), (kb, vb))| ka == kb && eq_f64(va, vb))
    {
        return false;
    }
    let fills_ok = match (&a.fills, &b.fills) {
        (None, None) => true,
        (Some(fa), Some(fb)) => eq_paint_vec(fa, fb),
        _ => false,
    };
    fills_ok && eq_opt_exact(&a.decoration_look, &b.decoration_look)
}

fn eq_text_data(a: &TextData, b: &TextData) -> bool {
    a.characters == b.characters
        && eq_text_style(&a.style, &b.style)
        && a.auto_resize == b.auto_resize
        && a.align_h == b.align_h
        && a.align_v == b.align_v
        && eq_f64(&a.paragraph_spacing, &b.paragraph_spacing)
        && eq_f64(&a.paragraph_indent, &b.paragraph_indent)
        && a.list == b.list
        && a.paragraphs == b.paragraphs
        && eq_f64(&a.list_spacing, &b.list_spacing)
        && a.hanging_punctuation == b.hanging_punctuation
        && a.hanging_list == b.hanging_list
        && a.leading_trim == b.leading_trim
        && a.max_lines == b.max_lines
        && a.wrap_style == b.wrap_style
        && a.features == b.features
        && eq_vec_exact(&a.runs, &b.runs)
        && eq_opt_exact(&a.path, &b.path)
        && eq_opt_exact(&a.saved_breaks, &b.saved_breaks)
}

pub(super) fn exact_kind_eq(a: &NodeKind, b: &NodeKind) -> bool {
    match (a, b) {
        (NodeKind::Frame, NodeKind::Frame)
        | (NodeKind::Group, NodeKind::Group)
        | (NodeKind::Rectangle, NodeKind::Rectangle)
        | (NodeKind::Line, NodeKind::Line)
        | (NodeKind::Slice, NodeKind::Slice) => true,
        (NodeKind::Vector { network: na }, NodeKind::Vector { network: nb }) => exact_vector_network_eq(na, nb),
        (NodeKind::Text(ta), NodeKind::Text(tb)) => eq_text_data(ta, tb),
        _ => eq_exact(a, b),
    }
}
