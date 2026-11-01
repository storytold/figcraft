use super::common::{export_setting, layout_grid, outlined, prop_type, variant_pairs};
use super::{
    AUX, AutoLayout, BoolOp, Bound, ContentAlign, CrossAlign, Exporter, Fields, HConstraint, Kv, LAYERS, LayoutMode, MAX_DEPTH, MainAlign, MaskType,
    Node, NodeId, NodeKind, PropDef, STASH, Sizing, TextData, TextStyle, VConstraint, WindingRule, arr, b, blend_name, color, e, fl, grid_sizing,
    guid, int, matrix, o, position, root_fields, s, u, xy,
};

impl<'d> Exporter<'d> {
    /// Write layer `id` (child `index` of the layer with GUID `parent_guid`) and its subtree.
    pub(super) fn emit(&mut self, id: NodeId, parent_guid: Kv<'static>, index: usize, parent: Option<&Node>, depth: usize) {
        let doc = self.doc;
        let (Some(&local), Some(n)) = (self.ids.get(&id), doc.node(id)) else { return };
        if depth > MAX_DEPTH {
            self.warn("layers nested deeper than 256 levels were left out");
            return;
        }
        let mut f: Fields = vec![
            ("guid", guid(LAYERS, local)),
            ("phase", e("CREATED")),
            ("parentIndex", o(vec![("guid", parent_guid), ("position", s(&position(index)))])),
        ];
        let children = if let NodeKind::Page { background, prototype, .. } = &n.kind {
            f.extend([("type", e("CANVAS")), ("name", s(&n.name))]);
            f.extend(root_fields());
            f.extend([("backgroundColor", color(*background)), ("backgroundOpacity", fl(background.a.into())), ("backgroundEnabled", b(true))]);
            if !n.guides.is_empty() {
                self.warn("ruler guides were left out");
            }
            if *prototype != Default::default() {
                self.warn("prototype device settings were left out");
            }
            self.modes(n, &mut f);
            true
        } else {
            self.layer(n, parent, &mut f, false)
        };
        self.nodes.push(o(f));
        if children {
            for (i, c) in n.children.iter().enumerate() {
                self.emit(*c, guid(LAYERS, local), i, Some(n), depth + 1);
            }
        }
    }

    /// The fields of layer `n` (whose parent is `parent`). `nested`: for an instance's override
    /// (no type, no instance or component data). Returns whether its children are written as
    /// layers (an instance's come from its main).
    pub(super) fn layer(&mut self, n: &Node, parent: Option<&Node>, f: &mut Fields, nested: bool) -> bool {
        let doc = self.doc;
        if let Some((outline, original)) = outlined(n) {
            // The format's variable width is the same on both sides of the path: a stroke wider on
            // one side is written as the area it paints, with the stroke itself kept as plugin data
            // (which the import puts back).
            self.warn("strokes wider on one side of their path were written as filled outlines (the stroke itself is kept for FigCraft)");
            let children = self.layer(&outline, parent, f, nested);
            f.push(("pluginData", arr(vec![o(vec![("pluginID", s(STASH.0)), ("value", s(&original)), ("key", s(STASH.1))])])));
            return children;
        }
        let main = match &n.kind {
            NodeKind::Instance { component, .. } => {
                let found = self.ids.contains_key(component) && doc.node(*component).is_some_and(|c| matches!(c.kind, NodeKind::Component { .. }));
                if !found && !nested {
                    self.warn("instances of components that aren't in the file were written as frames");
                }
                found.then_some(*component)
            }
            _ => None,
        };
        let ty = match &n.kind {
            NodeKind::Page { .. } | NodeKind::Frame | NodeKind::Group | NodeKind::ComponentSet { .. } => "FRAME",
            NodeKind::Instance { .. } if main.is_none() => "FRAME",
            NodeKind::Section { .. } => "SECTION",
            NodeKind::Rectangle => "ROUNDED_RECTANGLE",
            NodeKind::Ellipse { .. } => "ELLIPSE",
            NodeKind::Polygon { .. } => "REGULAR_POLYGON",
            NodeKind::Star { .. } => "STAR",
            NodeKind::Line => "LINE",
            NodeKind::Vector { .. } => "VECTOR",
            NodeKind::Text(_) => "TEXT",
            NodeKind::BooleanOp { .. } => "BOOLEAN_OPERATION",
            NodeKind::Component { .. } => "SYMBOL",
            NodeKind::Instance { .. } => "INSTANCE",
            NodeKind::Slice => "SLICE",
        };
        if !nested {
            f.push(("type", e(ty)));
        }
        f.push(("name", s(&n.name)));
        self.common(n, parent, f);
        match &n.kind {
            NodeKind::Group => f.push(("resizeToFit", b(true))),
            NodeKind::Section { contents_hidden } => f.push(("sectionContentsHidden", b(*contents_hidden))),
            NodeKind::Ellipse { start, sweep, inner } => {
                f.push(("arcData", o(vec![("startingAngle", fl(*start)), ("endingAngle", fl(start + sweep)), ("innerRadius", fl(*inner))])))
            }
            NodeKind::Polygon { count } => f.push(("count", u(*count))),
            NodeKind::Star { count, inner_ratio } => f.extend([("count", u(*count)), ("starInnerScale", fl(*inner_ratio))]),
            NodeKind::Vector { network } => self.vector(n, network, f),
            NodeKind::Text(td) => self.text(td, &n.fills, f),
            NodeKind::BooleanOp { op } => {
                let name = match op {
                    BoolOp::Union => "UNION",
                    BoolOp::Subtract => "SUBTRACT",
                    BoolOp::Intersect => "INTERSECT",
                    BoolOp::Exclude => "XOR",
                };
                f.push(("booleanOperation", e(name)));
                if n.children.iter().filter_map(|c| doc.node(*c)).any(|c| c.bool_op.is_some()) {
                    self.warn("boolean groups whose layers combine in different ways use the group's operation");
                }
            }
            NodeKind::Component { description, props, prop_order } if !nested => {
                if let Some(set) = self.set_of(n.id) {
                    // A variant: its set holds the definitions; the variant has its own copy of
                    // each (pointing at the set's), and its values of the variant properties.
                    let own: Vec<Kv> = props
                        .iter()
                        .filter_map(|(name, d)| {
                            let ty = prop_type(d)?;
                            let id = *self.child_defs.get(&(n.id, name.clone(), ty))?;
                            let parent = *self.defs.get(&(set, name.clone(), ty))?;
                            Some(o(vec![("id", guid(AUX, id)), ("parentPropDefId", guid(AUX, parent))]))
                        })
                        .collect();
                    if !own.is_empty() {
                        f.push(("componentPropDefs", arr(own)));
                    }
                    let specs: Vec<Kv> = variant_pairs(&n.name)
                        .into_iter()
                        .filter_map(|(prop, value)| {
                            let id = *self.defs.get(&(set, prop, "VARIANT"))?;
                            Some(o(vec![("propDefId", guid(AUX, id)), ("value", s(&value))]))
                        })
                        .collect();
                    if !specs.is_empty() {
                        f.push(("variantPropSpecs", arr(specs)));
                    }
                } else {
                    let defs: Vec<Kv> = figcraft_doc::ordered_props(props, prop_order)
                        .into_iter()
                        .enumerate()
                        .filter_map(|(i, (name, d))| self.prop_def(n.id, name, d, i))
                        .collect();
                    if !defs.is_empty() {
                        f.push(("componentPropDefs", arr(defs)));
                    }
                }
                if props.values().any(|d| matches!(d, PropDef::Slot { .. })) {
                    self.warn("slot properties were left out");
                }
                if !description.is_empty() {
                    f.push(("symbolDescription", s(description)));
                }
            }
            NodeKind::ComponentSet { description } if !nested => {
                f.push(("isStateGroup", b(true)));
                self.set_props(n, f);
                if !description.is_empty() {
                    f.push(("symbolDescription", s(description)));
                }
            }
            NodeKind::Instance { props, .. } if !nested => {
                if let Some(main) = main {
                    self.instance(n, main, props, f);
                }
            }
            _ => {}
        }
        let boxy = n.kind.is_frame_like() || matches!(n.kind, NodeKind::Rectangle | NodeKind::Section { .. });
        if (boxy || matches!(n.kind, NodeKind::Ellipse { .. } | NodeKind::Polygon { .. } | NodeKind::Star { .. }))
            && let Some(p) = figcraft_doc::geometry::shape_path(n)
        {
            self.geometry("fillGeometry", &p, WindingRule::NonZero, f);
        }
        self.stroke_geometry(n, boxy, f);
        main.is_none()
    }

    /// Fields every layer kind shares.
    fn common(&mut self, n: &Node, parent: Option<&Node>, f: &mut Fields) {
        f.extend([
            ("visible", b(n.visible)),
            ("locked", b(n.locked)),
            ("opacity", fl(n.opacity.into())),
            ("blendMode", e(blend_name(n.blend))),
            ("size", xy(n.width, n.height)),
            ("transform", matrix(n.transform)),
        ]);
        if n.is_mask {
            let ty = match n.mask_type {
                MaskType::Alpha => "ALPHA",
                MaskType::Vector => "OUTLINE",
                MaskType::Luminance => "LUMINANCE",
            };
            f.extend([("mask", b(true)), ("maskType", e(ty))]);
        }
        let bound = self.bound(n);
        // Text whose own style sets fills paints with them (the layer's fills only show through
        // runs); a variable bound to the layer's fill colours the layer's.
        let fills = match &n.kind {
            NodeKind::Text(TextData { style: TextStyle { fills: Some(own), .. }, .. }) => self.paints(own, true, &Bound::default()),
            _ => self.paints(&n.fills, true, &bound),
        };
        let strokes = self.paints(&n.strokes, false, &bound);
        f.extend([("fillPaints", fills), ("strokePaints", strokes)]);
        self.stroke(n, f);
        if !n.effects.is_empty() {
            let effects = n.effects.iter().enumerate().map(|(i, x)| self.effect(x, i, &bound)).collect();
            f.push(("effects", arr(effects)));
        }
        if n.kind.has_corner_radius() {
            let r = n.corners.radii;
            f.push(("cornerRadius", fl(r[0])));
            let differ = r.iter().any(|x| *x != r[0]);
            if n.kind.has_independent_corners() && (differ || r[0] > 0.0) {
                f.extend(differ.then(|| ("rectangleCornerRadiiIndependent", b(true))));
                f.extend([
                    ("rectangleTopLeftCornerRadius", fl(r[0])),
                    ("rectangleTopRightCornerRadius", fl(r[1])),
                    ("rectangleBottomRightCornerRadius", fl(r[2])),
                    ("rectangleBottomLeftCornerRadius", fl(r[3])),
                ]);
            }
            if n.corners.smoothing > 0.0 {
                f.push(("cornerSmoothing", fl(n.corners.smoothing)));
            }
        }
        let h = match n.constraints.horizontal {
            HConstraint::Left => "MIN",
            HConstraint::Right => "MAX",
            HConstraint::LeftRight => "STRETCH",
            HConstraint::Center => "CENTER",
            HConstraint::Scale => "SCALE",
        };
        let v = match n.constraints.vertical {
            VConstraint::Top => "MIN",
            VConstraint::Bottom => "MAX",
            VConstraint::TopBottom => "STRETCH",
            VConstraint::Center => "CENTER",
            VConstraint::Scale => "SCALE",
        };
        f.extend([("horizontalConstraint", e(h)), ("verticalConstraint", e(v))]);
        if n.kind.is_frame_like() || matches!(n.kind, NodeKind::Section { .. }) {
            f.push(("frameMaskDisabled", b(!n.clip)));
        }
        if n.kind.is_frame_like() {
            if let Some(l) = &n.layout {
                self.auto_layout(n, l, f);
            }
            if !n.grids.is_empty() {
                f.push(("layoutGrids", arr(n.grids.iter().enumerate().map(|(i, g)| layout_grid(g, i, &bound)).collect())));
            }
        }
        self.child_layout(n, parent, f);
        if !n.exports.is_empty() {
            if n.exports.iter().any(|x| x.format == figcraft_doc::ExportFormat::Webp) {
                self.warn("WebP export settings were written as PNG");
            }
            if n.exports.iter().any(|x| x.format == figcraft_doc::ExportFormat::VectorDrawable) {
                self.warn("Android Vector Drawable (XML) export settings were written as SVG");
            }
            f.push(("exportSettings", arr(n.exports.iter().map(export_setting).collect())));
        }
        self.prototype(n, f);
        let (entries, refs): (Vec<Kv>, Vec<Kv>) = n.prop_refs.iter().filter_map(|(field, prop)| self.prop_ref(n, field, prop)).unzip();
        if !entries.is_empty() {
            f.push(("parameterConsumptionMap", o(vec![("entries", arr(entries))])));
            f.push(("componentPropRefs", arr(refs)));
        }
        let text = matches!(n.kind, NodeKind::Text(_));
        let links = [
            ("styleIdForFill", &n.styles.fill, true),
            ("styleIdForStrokeFill", &n.styles.stroke, true),
            ("styleIdForEffect", &n.styles.effect, true),
            ("styleIdForGrid", &n.styles.grid, n.kind.is_frame_like()),
            ("styleIdForText", &n.styles.text, text),
        ];
        for (key, id, fits) in links {
            let Some(id) = id else { continue };
            match self.style_ids.get(id.as_str()).filter(|_| fits) {
                Some(local) => f.push((key, o(vec![("guid", guid(LAYERS, *local))]))),
                None => self.warn("links to shared styles that aren't in the document were left out (their layers keep the values)"),
            }
        }
        if let NodeKind::Text(td) = &n.kind
            && n.styles.text.is_none()
            && std::iter::once(&td.style).chain(td.runs.iter().map(|r| &r.style)).any(|st| st.style_id.is_some())
        {
            self.warn("text styles applied to part of a text were written as their values");
        }
        if !bound.entries.is_empty() {
            f.push(("variableConsumptionMap", o(vec![("entries", arr(bound.entries))])));
        }
        self.modes(n, f);
        if !n.guides.is_empty() {
            self.warn("ruler guides were left out");
        }
        if !n.annotations.is_empty()
            || !n.dev_resources.is_empty()
            || !n.code_links.is_empty()
            || !n.measurements.is_empty()
            || !n.dev_status.is_none()
        {
            self.warn("Dev Mode annotations, statuses, resources, measurements and code links were left out");
        }
        if !n.plugin_data.is_empty() {
            self.warn("plugin data was left out");
        }
    }

    /// The variable modes layer (or page) `n` sets for what is in it.
    fn modes(&mut self, n: &Node, f: &mut Fields) {
        let mut entries = Vec::new();
        for (coll, mode) in &n.var_modes {
            match (self.set_ids.get(coll.as_str()), self.mode_ids.get(&(coll.as_str(), mode.as_str()))) {
                (Some(set), Some(mode)) => {
                    entries.push(o(vec![("variableSetID", o(vec![("guid", guid(LAYERS, *set))])), ("variableModeID", guid(AUX, *mode))]));
                }
                _ => self.warn("variable modes of collections that aren't in the document were left out"),
            }
        }
        if !entries.is_empty() {
            f.push(("variableModeBySetMap", o(vec![("entries", arr(entries))])));
        }
    }

    fn auto_layout(&mut self, n: &Node, l: &AutoLayout, f: &mut Fields) {
        let grid = l.mode == LayoutMode::Grid;
        let mode = match l.mode {
            LayoutMode::Horizontal => "HORIZONTAL",
            LayoutMode::Vertical => "VERTICAL",
            LayoutMode::Grid => "GRID",
        };
        let [top, right, bottom, left] = l.padding;
        let main = match l.main_align {
            MainAlign::Min => "MIN",
            MainAlign::Center => "CENTER",
            MainAlign::Max => "MAX",
            MainAlign::SpaceBetween => "SPACE_BETWEEN",
            MainAlign::SpaceAround => "SPACE_AROUND",
            MainAlign::SpaceEvenly => "SPACE_EVENLY_CSS",
        };
        let cross = match l.cross_align {
            CrossAlign::Min => "MIN",
            CrossAlign::Center => "CENTER",
            CrossAlign::Max => "MAX",
            CrossAlign::Baseline => "BASELINE",
        };
        // The main axis is the width for horizontal and grid layouts.
        let (main_hug, cross_hug) = {
            let (h, v) = (n.child_layout.sizing_h == Sizing::Hug, n.child_layout.sizing_v == Sizing::Hug);
            if l.mode == LayoutMode::Vertical { (v, h) } else { (h, v) }
        };
        let size = |hug: bool| e(if hug { "RESIZE_TO_FIT" } else { "FIXED" });
        f.extend([
            ("stackMode", e(mode)),
            ("stackWrap", e(if l.wrap { "WRAP" } else { "NO_WRAP" })),
            ("stackHorizontalPadding", fl(left)),
            ("stackVerticalPadding", fl(top)),
            ("stackPaddingRight", fl(right)),
            ("stackPaddingBottom", fl(bottom)),
            ("stackPrimaryAlignItems", e(main)),
            ("stackCounterAlignItems", e(cross)),
            ("stackPrimarySizing", size(main_hug)),
            ("stackCounterSizing", size(cross_hug)),
            ("bordersTakeSpace", b(l.strokes_included)),
            ("stackReverseZIndex", b(l.reverse_z)),
        ]);
        if grid {
            self.grid_layout(n, l, f);
        } else {
            let spread = l.content_align == ContentAlign::SpaceBetween;
            f.extend([
                ("stackSpacing", fl(l.gap)),
                ("stackCounterSpacing", fl(l.cross_gap)),
                ("stackCounterAlignContent", e(if spread { "SPACE_BETWEEN" } else { "AUTO" })),
            ]);
        }
    }

    /// A grid layout's track gaps, order and sizes.
    fn grid_layout(&mut self, n: &Node, l: &AutoLayout, f: &mut Fields) {
        // Instance layers share their main layer's track ids, so unchanged tracks and the cells
        // their layers sit in still match.
        let key = self.main_layer(n.id);
        let (want_c, want_r) = (l.grid_columns.min(1000) as usize, l.grid_rows.min(1000) as usize);
        let (cols, rows) = match self.tracks.get(&key) {
            Some((c, r)) if (c.len(), r.len()) == (want_c, want_r) => (c.clone(), r.clone()),
            _ => ((0..want_c).map(|_| self.next_aux()).collect(), (0..want_r).map(|_| self.next_aux()).collect()),
        };
        let map = |ids: &[u32]| {
            o(vec![("entries", arr(ids.iter().enumerate().map(|(i, t)| o(vec![("id", guid(AUX, *t)), ("position", s(&position(i)))])).collect()))])
        };
        f.extend([("gridColumnGap", fl(l.gap)), ("gridRowGap", fl(l.cross_gap)), ("gridColumns", map(&cols)), ("gridRows", map(&rows))]);
        let mut dropped = false;
        let column_sizing = grid_sizing(&l.column_tracks, &cols, &mut dropped);
        let row_sizing = grid_sizing(&l.row_tracks, &rows, &mut dropped);
        if dropped {
            self.warn("grid tracks whose size the format can't hold were written as equal tracks");
        }
        for (name, sizing) in [("gridColumnsSizing", column_sizing), ("gridRowsSizing", row_sizing)] {
            if let Some(v) = sizing {
                f.push((name, v));
            }
        }
        self.tracks.insert(key, (cols, rows));
    }

    /// How a layer sizes and sits in its auto layout parent. Min/max sizes are the layer's own
    /// and are written whatever its parent is.
    fn child_layout(&mut self, n: &Node, parent: Option<&Node>, f: &mut Fields) {
        let cl = &n.child_layout;
        // An axis without a minimum is 0; one without a maximum is infinite (0 would be a maximum
        // of 0, which collapses the layer in readers that apply it).
        for (key, w, h, none) in [("minSize", cl.min_w, cl.min_h, 0.0), ("maxSize", cl.max_w, cl.max_h, f32::INFINITY)] {
            if w.is_some() || h.is_some() {
                let axis = |x: Option<f64>| x.map_or(Kv::Float(none), fl);
                f.push((key, o(vec![("value", o(vec![("x", axis(w)), ("y", axis(h))]))])));
            }
        }
        let Some(pl) = parent.and_then(|p| p.layout.as_ref()) else { return };
        let (fill_h, fill_v) = (cl.sizing_h == Sizing::Fill, cl.sizing_v == Sizing::Fill);
        let (grow, stretch) = if pl.mode == LayoutMode::Vertical { (fill_v, fill_h) } else { (fill_h, fill_v) };
        if grow {
            f.push(("stackChildPrimaryGrow", fl(1.0)));
        }
        if stretch {
            f.push(("stackChildAlignSelf", e("STRETCH")));
        }
        if cl.absolute {
            f.push(("stackPositioning", e("ABSOLUTE")));
        }
        if pl.mode == LayoutMode::Grid
            && let Some((cols, rows)) = parent.and_then(|p| self.tracks.get(&self.main_layer(p.id)))
        {
            let anchor = |ids: &[u32], i: Option<u32>| i.and_then(|i| ids.get(i as usize)).map(|t| guid(AUX, *t));
            let (c, r) = (anchor(cols, cl.grid_column), anchor(rows, cl.grid_row));
            f.extend(c.map(|g| ("gridColumnAnchor", g)));
            f.extend(r.map(|g| ("gridRowAnchor", g)));
            if cl.grid_column_span > 0 {
                f.push(("gridColumnSpan", int(cl.grid_column_span.into())));
            }
            if cl.grid_row_span > 0 {
                f.push(("gridRowSpan", int(cl.grid_row_span.into())));
            }
        }
    }
}
