//! Document inspection as JSON, so agents can verify their work without screenshots.

use figcraft_doc::{Document, Node, NodeId, NodeKind, StrokeJoin, TextData, TextDecoration};
use serde_json::{Value, json};

use crate::Session;

/// A node and (to `depth`) its children, with the properties a designer would check.
pub fn node_json(doc: &Document, id: NodeId, depth: usize) -> Value {
    let Some(n) = doc.node(id) else { return Value::Null };
    let mut v = json!({
        "id": n.id,
        "name": n.name,
        "type": n.kind.type_name(),
    });
    let Some(o) = v.as_object_mut() else { return v };
    if !n.is_page() {
        base_props(doc, n, id, o);
        paint_props(n, o);
        layout_props(doc, n, o);
        dev_props(doc, n, o);
    }
    // Layer data written with `node.set {props: {pluginData}}` (pages too).
    if !n.plugin_data.is_empty() {
        o.insert("pluginData".into(), json!(n.plugin_data));
    }
    match &n.kind {
        NodeKind::Text(t) => text_props(t, o),
        _ => kind_props(doc, n, o),
    }
    if !n.children.is_empty() {
        if depth > 0 {
            o.insert("children".into(), Value::Array(n.children.iter().map(|c| node_json(doc, *c, depth - 1)).collect()));
        } else {
            o.insert("childCount".into(), json!(n.children.len()));
        }
    }
    v
}

fn base_props(doc: &Document, n: &Node, id: NodeId, o: &mut serde_json::Map<String, Value>) {
    let c = doc.frame_relative(id).as_coeffs();
    o.insert("x".into(), json!(round(c[4])));
    o.insert("y".into(), json!(round(c[5])));
    o.insert("width".into(), json!(round(n.width)));
    o.insert("height".into(), json!(round(n.height)));
    let rot = figcraft_geom::rotation_degrees(n.transform);
    if rot.abs() > 1e-6 {
        o.insert("rotation".into(), json!(round(rot)));
    }
    if let Some(b) = doc.world_bounds(id) {
        o.insert("absoluteBounds".into(), json!({"x": round(b.x0), "y": round(b.y0), "width": round(b.width()), "height": round(b.height())}));
    }
    if !n.visible {
        o.insert("visible".into(), json!(false));
    }
    if n.locked {
        o.insert("locked".into(), json!(true));
    }
    if n.opacity < 1.0 {
        o.insert("opacity".into(), json!(n.opacity));
    }
    if !matches!(n.blend, figcraft_color::BlendMode::Normal | figcraft_color::BlendMode::PassThrough) {
        o.insert("blend".into(), json!(n.blend));
    }
}

fn paint_props(n: &Node, o: &mut serde_json::Map<String, Value>) {
    if !n.fills.is_empty() {
        o.insert("fills".into(), paints(&n.fills));
    }
    if !n.strokes.is_empty() {
        o.insert("strokes".into(), paints(&n.strokes));
        o.insert("strokeWeight".into(), json!(n.stroke.weight));
        o.insert("strokeAlign".into(), json!(n.stroke.align));
        if n.stroke.sides.is_some() {
            o.insert("strokeSides".into(), json!(n.stroke.sides));
        }
        if !n.stroke.dashes.is_empty() {
            o.insert("dashes".into(), json!(n.stroke.dashes));
        }
        if n.stroke.join != StrokeJoin::Miter {
            o.insert("strokeJoin".into(), json!(n.stroke.join));
        } else if n.stroke.miter_limit != 4.0 {
            o.insert("miterAngle".into(), json!((n.stroke.miter_angle() * 100.0).round() / 100.0));
        }
        if let Some(p) = n.stroke.profile() {
            let points = crate::cmd::props::width_points_json(Some(p));
            o.insert("widthProfile".into(), json!({"preset": p.preset().map(|x| x.id()), "points": points}));
        }
        if let Some(b) = &n.stroke.brush {
            o.insert("brush".into(), camel(serde_json::to_value(b).unwrap_or(Value::Null)));
        }
    }
    if !n.effects.is_empty() {
        o.insert("effects".into(), camel(serde_json::to_value(&n.effects).unwrap_or(Value::Null)));
    }
    if n.corners.radii.iter().any(|r| *r > 0.0) {
        o.insert(
            "cornerRadius".into(),
            match n.corners.uniform() {
                Some(r) => json!(r),
                None => json!(n.corners.radii),
            },
        );
        if n.corners.smoothing > 0.0 {
            o.insert("cornerSmoothing".into(), json!(n.corners.smoothing));
        }
    }
}

fn layout_props(doc: &Document, n: &Node, o: &mut serde_json::Map<String, Value>) {
    if let Some(l) = &n.layout {
        o.insert("layout".into(), camel(serde_json::to_value(l).unwrap_or(Value::Null)));
    }
    let parent_al = n.parent.and_then(|p| doc.node(p)).is_some_and(|p| p.layout.is_some());
    if n.layout.is_some() || parent_al || matches!(n.kind, NodeKind::Text(_)) {
        o.insert("sizing".into(), json!({"horizontal": n.child_layout.sizing_h, "vertical": n.child_layout.sizing_v}));
    }
    let cl = &n.child_layout;
    if cl.absolute {
        o.insert("absolute".into(), json!(true));
    }
    for (k, v) in [("minWidth", cl.min_w), ("maxWidth", cl.max_w), ("minHeight", cl.min_h), ("maxHeight", cl.max_h)] {
        if let Some(v) = v {
            o.insert(k.into(), json!(v));
        }
    }
    if let Some(cell) = parent_al
        .then(|| n.parent.and_then(|p| figcraft_layout::grid_geometry(doc, p)))
        .flatten()
        .and_then(|g| g.cells.into_iter().find(|c| c.id == n.id))
    {
        o.insert(
            "gridCell".into(),
            json!({"column": cell.column, "row": cell.row, "columnSpan": cell.column_span, "rowSpan": cell.row_span,
                       "anchored": cl.grid_column.is_some() || cl.grid_row.is_some(), "alignH": cl.grid_align_h, "alignV": cl.grid_align_v}),
        );
    }
    if (!parent_al || cl.absolute) && n.parent.and_then(|p| doc.node(p)).is_some_and(|p| p.kind.is_frame_like()) {
        o.insert("constraints".into(), serde_json::to_value(n.constraints).unwrap_or(Value::Null));
    }
    if n.kind.is_frame_like() {
        o.insert("clip".into(), json!(n.clip));
    }
}

fn dev_props(doc: &Document, n: &Node, o: &mut serde_json::Map<String, Value>) {
    if !n.dev_status.is_none() {
        o.insert("devStatus".into(), json!(n.dev_status.id()));
    }
    if n.dev_changed {
        o.insert("devChanged".into(), json!(true));
    }
    if !n.dev_note.is_empty() {
        o.insert("devNote".into(), json!(n.dev_note));
    }
    if let NodeKind::Section { contents_hidden } = n.kind {
        o.insert("sectionContentsHidden".into(), json!(contents_hidden));
    }
    if n.is_mask {
        o.insert("isMask".into(), json!(true));
        o.insert("maskType".into(), json!(n.mask_type.id()));
    }
    if !n.grids.is_empty() {
        o.insert("grids".into(), camel(serde_json::to_value(&n.grids).unwrap_or(Value::Null)));
    }
    if !n.exports.is_empty() {
        o.insert("exports".into(), serde_json::to_value(&n.exports).unwrap_or(Value::Null));
    }
    if let Some(b) = &n.board {
        o.insert("board".into(), serde_json::to_value(b).unwrap_or(Value::Null));
    }
    if !n.annotations.is_empty() {
        o.insert("annotations".into(), serde_json::to_value(&n.annotations).unwrap_or(Value::Null));
    }
    if !n.code_links.is_empty() {
        o.insert("codeLinks".into(), camel(serde_json::to_value(&n.code_links).unwrap_or(Value::Null)));
    }
    if !n.measurements.is_empty() {
        o.insert("measurements".into(), serde_json::to_value(&n.measurements).unwrap_or(Value::Null));
    }
    let dev = crate::dev_resources::list_json(doc, n.id);
    if dev.as_array().is_some_and(|a| !a.is_empty()) {
        o.insert("devResources".into(), dev);
    }
    if !n.interactions.is_empty() {
        o.insert("interactions".into(), serde_json::to_value(&n.interactions).unwrap_or(Value::Null));
    }
    if !n.styles.is_empty() {
        o.insert("styles".into(), serde_json::to_value(&n.styles).unwrap_or(Value::Null));
    }
    if !n.bound.is_empty() {
        o.insert("boundVariables".into(), json!(n.bound));
    }
    if !n.overrides.is_empty() {
        o.insert("overrides".into(), json!(n.overrides));
    }
    if let Some(slot) = crate::components::slot_name(doc, n.id) {
        o.insert("slot".into(), json!(slot));
    }
    // An operand combining by an operation of its own.
    if let Some(op) = n.bool_op.filter(|_| crate::booleans::group_of(doc, n.id).is_some()) {
        o.insert("operandOperation".into(), json!(op));
    }
}

fn text_props(t: &TextData, o: &mut serde_json::Map<String, Value>) {
    o.insert("characters".into(), json!(t.characters));
    o.insert("fontFamily".into(), json!(t.style.font_family));
    o.insert("fontStyle".into(), json!(t.style.font_style));
    o.insert("fontSize".into(), json!(t.style.font_size));
    if !t.style.variations.is_empty() {
        o.insert("fontVariations".into(), json!(t.style.variations));
    }
    o.insert("lineHeight".into(), json!(t.style.line_height.label()));
    o.insert("letterSpacing".into(), json!(t.style.letter_spacing.label()));
    o.insert("textAlign".into(), json!(t.align_h));
    o.insert("textAutoResize".into(), json!(t.auto_resize));
    if t.max_lines > 0 {
        o.insert("maxLines".into(), json!(t.max_lines));
    }
    if let Some(tp) = &t.path {
        o.insert("textPath".into(), json!(tp));
    }
    if t.style.decoration != TextDecoration::None {
        o.insert("textDecoration".into(), json!(t.style.decoration));
    }
    if let Some(l) = t.style.decoration_look.as_deref() {
        o.insert("textDecorationStyle".into(), json!(l.style));
        o.insert("textDecorationThickness".into(), json!(l.thickness.label()));
        o.insert("textDecorationOffset".into(), json!(l.offset.label()));
        o.insert("textDecorationSkipInk".into(), json!(l.skip_ink));
        o.insert("textDecorationColor".into(), l.color.map_or(json!("auto"), |c| json!(format!("#{}", c.to_hex()))));
    }
    // Lists: one style for the layer, or each paragraph's type, nesting level and start.
    if t.paragraphs.is_empty() {
        if t.list != figcraft_doc::ListStyle::None {
            o.insert("listStyle".into(), json!(t.list));
        }
    } else {
        o.insert("lineTypes".into(), t.paragraphs.iter().map(|p| json!(p.list)).collect());
        o.insert("lineIndentations".into(), t.paragraphs.iter().map(|p| json!(p.indent)).collect());
        if t.paragraphs.iter().any(|p| p.start_offset > 0) {
            o.insert("listStartOffsets".into(), t.paragraphs.iter().map(|p| json!(p.start_offset)).collect());
        }
    }
    if t.list_spacing != 0.0 {
        o.insert("listSpacing".into(), json!(t.list_spacing));
    }
    for (k, on) in [("hangingPunctuation", t.hanging_punctuation), ("hangingList", t.hanging_list)] {
        if on {
            o.insert(k.into(), json!(true));
        }
    }
    if t.leading_trim != figcraft_doc::LeadingTrim::None {
        o.insert("leadingTrim".into(), json!(t.leading_trim));
    }
    if !t.runs.is_empty() {
        o.insert("styledRuns".into(), serde_json::to_value(&t.runs).unwrap_or(Value::Null));
    }
    // Chars linked to text styles: [start, end, style id] (the layer as a whole: `styles.text`).
    let linked: Vec<Value> = t.spans().into_iter().filter_map(|(a, b, s)| s.style_id.as_ref().map(|id| json!([a, b, id]))).collect();
    if !linked.is_empty() {
        o.insert("textStyleRanges".into(), Value::Array(linked));
    }
}

fn kind_props(doc: &Document, n: &Node, o: &mut serde_json::Map<String, Value>) {
    match &n.kind {
        NodeKind::Ellipse { start, sweep, inner } if (*sweep).abs() < std::f64::consts::TAU - 1e-9 || *inner > 0.0 => {
            o.insert("arc".into(), json!({"start": start.to_degrees(), "sweep": sweep.to_degrees(), "inner": inner}));
        }
        NodeKind::Polygon { count } => {
            o.insert("pointCount".into(), json!(count));
        }
        NodeKind::Star { count, inner_ratio } => {
            o.insert("pointCount".into(), json!(count));
            o.insert("innerRadius".into(), json!(inner_ratio));
        }
        NodeKind::BooleanOp { op } => {
            o.insert("booleanOperation".into(), json!(op));
        }
        NodeKind::Instance { component, props } => {
            o.insert("mainComponent".into(), json!(component));
            o.insert("mainComponentName".into(), json!(doc.node(*component).map(|c| c.name.clone())));
            if !props.is_empty() {
                o.insert("componentProperties".into(), json!(props));
            }
        }
        NodeKind::Component { description, props } => {
            if !description.is_empty() {
                o.insert("description".into(), json!(description));
            }
            if !props.is_empty() {
                o.insert("componentPropertyDefinitions".into(), serde_json::to_value(props).unwrap_or(Value::Null));
            }
        }
        NodeKind::Vector { network } => {
            o.insert("vertices".into(), json!(network.vertices.len()));
            o.insert("segments".into(), json!(network.segments.len()));
        }
        NodeKind::Page { background, flows, .. } => {
            o.insert("background".into(), json!(background.to_css_hex()));
            if !flows.is_empty() {
                o.insert("flows".into(), serde_json::to_value(flows).unwrap_or(Value::Null));
            }
        }
        _ => {}
    }
}

fn round(v: f64) -> f64 {
    figcraft_geom::round_to(v, 2)
}

fn paints(p: &[figcraft_color::Paint]) -> Value {
    Value::Array(
        p.iter()
            .map(|x| match &x.kind {
                figcraft_color::PaintKind::Solid { color } => {
                    let mut v = json!({"type": "solid", "color": color.to_css_hex()});
                    if x.opacity < 1.0 {
                        v["opacity"] = json!(x.opacity);
                    }
                    if x.blend != figcraft_color::BlendMode::Normal {
                        v["blend"] = json!(x.blend);
                    }
                    if !x.visible {
                        v["visible"] = json!(false);
                    }
                    v
                }
                _ => serde_json::to_value(x).unwrap_or(Value::Null),
            })
            .collect(),
    )
}

/// The whole document (current page in depth, other pages summarised).
pub fn document(s: &Session, depth: usize, all_pages: bool) -> Value {
    let Ok(ds) = s.ds() else { return json!({"documents": 0}) };
    let doc = &ds.doc;
    let pages: Vec<Value> =
        doc.pages.iter().map(|p| if all_pages || *p == ds.page { node_json(doc, *p, depth) } else { node_json(doc, *p, 0) }).collect();
    json!({
        "id": ds.uid,
        "name": doc.name,
        "kind": doc.kind.id(),
        "path": ds.path,
        "dirty": ds.is_dirty(),
        "currentPage": ds.page,
        "selection": ds.selection,
        "pages": pages,
        "styles": doc.styles.iter().map(|st| json!({"id": st.id, "name": st.name, "type": st.kind.type_name()})).collect::<Vec<_>>(),
        "variables": doc.variables.variables.len(),
        "images": doc.images.len(),
        "colorProfile": doc.color_profile.id(),
        "textEdit": ds.text_edit,
        "history": {"undo": s.undo_labels(20), "redo": s.undo_depth().1},
    })
}

/// snake_case object keys → camelCase (inspect output matches the keys `node.set` accepts).
pub fn camel(v: Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.into_iter()
                .map(|(k, v)| {
                    let mut out = String::new();
                    let mut up = false;
                    for c in k.chars() {
                        if c == '_' {
                            up = true;
                        } else if up {
                            out.extend(c.to_uppercase());
                            up = false;
                        } else {
                            out.push(c);
                        }
                    }
                    (out, camel(v))
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.into_iter().map(camel).collect()),
        other => other,
    }
}
