//! Mirror field deltas and structural equality checks.
//!
//! `Paint` ignores `color_alpha` in `PartialEq`, and IEEE-754 equates `-0.0` with `0.0`.
//! Float and paint fields therefore compare bit-for-bit or via serialized JSON so mirror
//! encoding never drops data that legacy snapshots preserve.

use std::collections::BTreeMap;

use figcraft_doc::color::{BlendMode, Paint, PaintKind};
use figcraft_doc::geom::Affine;
use figcraft_doc::proto::{Interaction, Overflow, ScrollPosition};
use figcraft_doc::{
    Annotation, AutoLayout, BoardItem, BoolOp, ChildLayout, CodeLink, CollectionId, Constraints, Corners, DevResource, DevStatus, Effect,
    ExportSetting, Guide, LayoutGrid, MaskType, Measurement, ModeId, Node, NodeKind, Region, Segment, StrokeProps, StyleRefs, VariableId,
    VectorNetwork, Vertex,
};
use serde::{Deserialize, Serialize};

use super::encode::VectorInterner;
use crate::Error;

/// Defines `FieldDelta`, mirror diffing and delta application from one field table so adding
/// or editing a field keeps all three in lockstep.
///
/// New variants must be appended and gated behind a version or capability bump because older
/// clients reject unknown variants during deserialization.
macro_rules! delta_fields {
    (
        boxed: [ $( $bvar:ident ( $bty:ty ) => $bfield:ident via $beq:ident ),* $(,)? ],
        value: [ $( $vvar:ident ( $vty:ty ) => $vfield:ident via $veq:ident ),* $(,)? ]
    ) => {
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub(super) enum FieldDelta {
            Kind(Box<NodeKind>),
            VectorRef(u32),
            $( $bvar(Box<$bty>), )*
            $( $vvar($vty), )*
        }

        fn diff_table_fields(src: &Node, node: &Node, out: &mut Vec<FieldDelta>) {
            $(
                if !$beq(&node.$bfield, &src.$bfield) {
                    out.push(FieldDelta::$bvar(Box::new(node.$bfield.clone())));
                }
            )*
            $(
                if !$veq(&node.$vfield, &src.$vfield) {
                    out.push(FieldDelta::$vvar(node.$vfield.clone()));
                }
            )*
        }

        pub(super) fn apply_field_delta(node: &mut Node, delta: FieldDelta, vectors: &[VectorNetwork]) -> Result<(), Error> {
            match delta {
                FieldDelta::Kind(v) => node.kind = *v,
                FieldDelta::VectorRef(idx) => {
                    let net = usize::try_from(idx)
                        .ok()
                        .and_then(|i| vectors.get(i))
                        .ok_or_else(|| Error::Decode(format!("invalid vector ref {idx}")))?;
                    node.kind = NodeKind::Vector { network: net.clone() };
                }
                $( FieldDelta::$bvar(v) => node.$bfield = *v, )*
                $( FieldDelta::$vvar(v) => node.$vfield = v, )*
            }
            Ok(())
        }
    };
}

delta_fields! {
    boxed: [
        Stroke(StrokeProps) => stroke via eq_exact,
        ChildLayout(ChildLayout) => child_layout via eq_exact,
        Styles(StyleRefs) => styles via eq_plain,
    ],
    value: [
        Name(String) => name via eq_plain,
        Visible(bool) => visible via eq_plain,
        Locked(bool) => locked via eq_plain,
        Opacity(f32) => opacity via eq_f32,
        Blend(BlendMode) => blend via eq_plain,
        Transform(Affine) => transform via eq_affine,
        Width(f64) => width via eq_f64,
        Height(f64) => height via eq_f64,
        Fills(Vec<Paint>) => fills via eq_paint_vec,
        Strokes(Vec<Paint>) => strokes via eq_paint_vec,
        Effects(Vec<Effect>) => effects via eq_vec_exact,
        Corners(Corners) => corners via eq_exact,
        Constraints(Constraints) => constraints via eq_plain,
        Layout(Option<AutoLayout>) => layout via eq_opt_exact,
        Clip(bool) => clip via eq_plain,
        IsMask(bool) => is_mask via eq_plain,
        MaskType(MaskType) => mask_type via eq_plain,
        Grids(Vec<LayoutGrid>) => grids via eq_vec_exact,
        Guides(Vec<Guide>) => guides via eq_vec_exact,
        Exports(Vec<ExportSetting>) => exports via eq_vec_exact,
        Interactions(Vec<Interaction>) => interactions via eq_vec_exact,
        Overflow(Overflow) => overflow via eq_plain,
        ScrollPosition(ScrollPosition) => scroll_position via eq_exact,
        Bound(BTreeMap<String, VariableId>) => bound via eq_plain,
        VarModes(BTreeMap<CollectionId, ModeId>) => var_modes via eq_plain,
        ScaleFactor(f64) => scale_factor via eq_f64,
        BoolOp(Option<BoolOp>) => bool_op via eq_plain,
        ConstrainProportions(bool) => constrain_proportions via eq_plain,
        Exposed(bool) => exposed via eq_plain,
        DevStatus(DevStatus) => dev_status via eq_plain,
        DevChanged(bool) => dev_changed via eq_plain,
        DevNote(String) => dev_note via eq_plain,
        PropRefs(BTreeMap<String, String>) => prop_refs via eq_plain,
        PluginData(BTreeMap<String, String>) => plugin_data via eq_plain,
        Annotations(Vec<Annotation>) => annotations via eq_vec_exact,
        CodeLinks(Vec<CodeLink>) => code_links via eq_plain,
        DevResources(Vec<DevResource>) => dev_resources via eq_plain,
        Measurements(Vec<Measurement>) => measurements via eq_vec_exact,
        Board(Option<BoardItem>) => board via eq_opt_exact,
    ]
}

/// Fails to compile if a new field is added to `figcraft_doc::Node` without updating `delta_fields!`.
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

pub(super) fn diff_mirror_fields(src: &Node, node: &Node, interner: &mut VectorInterner) -> Result<Vec<FieldDelta>, Error> {
    let mut deltas = Vec::new();
    if !exact_kind_eq(&node.kind, &src.kind) {
        match &node.kind {
            NodeKind::Vector { network } => deltas.push(FieldDelta::VectorRef(interner.intern(network)?)),
            other => deltas.push(FieldDelta::Kind(Box::new(other.clone()))),
        }
    }
    diff_table_fields(src, node, &mut deltas);
    Ok(deltas)
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

fn exact_kind_eq(a: &NodeKind, b: &NodeKind) -> bool {
    match (a, b) {
        (NodeKind::Frame, NodeKind::Frame)
        | (NodeKind::Group, NodeKind::Group)
        | (NodeKind::Rectangle, NodeKind::Rectangle)
        | (NodeKind::Line, NodeKind::Line) => true,
        (NodeKind::Vector { network: na }, NodeKind::Vector { network: nb }) => exact_vector_network_eq(na, nb),
        _ => eq_exact(a, b),
    }
}
