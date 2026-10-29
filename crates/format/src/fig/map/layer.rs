use super::component::{Entry, Inst, OverrideRow, prop_ref};
use super::connector::connector_has_saved_label;
use super::layout::{layout_grid, tracks};
use super::paint::{effect, export_setting, has_bound_variables, paint, paint_bound, stroke};
use super::{
    Affine, BTreeMap, BTreeSet, Kv, MAX_COMMANDS, MAX_DEPTH, Mapper, Obj, Value, View, copy_num, copy_str, generated_record, guid, json, matrix,
    network, num, rows, size,
};

/// How the layers of a resized container follow it when the file gives no derived layout for
/// them: a frame's by their constraints, from its main's size to its own; a group's scale with it.
#[derive(Clone, Copy)]
pub(super) enum Fit {
    Frame { from: (f64, f64), to: (f64, f64) },
    Scale(f64, f64),
}

impl Fit {
    fn frame(from: Option<(f64, f64)>, to: Option<(f64, f64)>) -> Option<Self> {
        let (from, to) = (from?, to?);
        ((from.0 - to.0).abs() > 1e-9 || (from.1 - to.1).abs() > 1e-9).then_some(Fit::Frame { from, to })
    }

    fn scale(from: Option<(f64, f64)>, to: Option<(f64, f64)>) -> Option<Self> {
        let (from, to) = (from?, to?);
        let k = |a: f64, b: f64| if a > 1e-9 { b / a } else { 1.0 };
        let (kx, ky) = (k(from.0, to.0), k(from.1, to.1));
        ((kx - 1.0).abs() > 1e-9 || (ky - 1.0).abs() > 1e-9).then_some(Fit::Scale(kx, ky))
    }

    /// The size and transform (in the parent's coordinates) of a `w` × `h` layer at `t` with
    /// constraints `h_con`, `v_con`. Like the editor's constraint solver, a layer keeps its
    /// rotation and flip and stretches along its own axes.
    fn apply(self, w: f64, h: f64, t: Affine, h_con: Option<&str>, v_con: Option<&str>) -> ((f64, f64), Affine) {
        let [a, b, c, d, _, _] = t.as_coeffs();
        let lin = Affine::new([a, b, c, d, 0.0, 0.0]);
        let bb = figcraft_doc::geom::transform_rect_bbox(t, figcraft_doc::geom::Rect::new(0.0, 0.0, w, h));
        // Where the new box goes: its top-left (constraints) or its centre (a group's scale).
        let (kx, ky, x0, y0, centred) = match self {
            Fit::Frame { from, to } => {
                let axis = |con: Option<&str>, lo: f64, hi: f64, old: f64, new: f64| match con {
                    Some("MAX" | "FIXED_MAX") => (lo + new - old, hi + new - old),
                    Some("STRETCH") => (lo, hi + new - old),
                    Some("CENTER") => (lo + (new - old) / 2.0, hi + (new - old) / 2.0),
                    Some("SCALE") if old > 1e-9 => (lo * new / old, hi * new / old),
                    _ => (lo, hi),
                };
                let (x0, x1) = axis(h_con, bb.x0, bb.x1, from.0, to.0);
                let (y0, y1) = axis(v_con, bb.y0, bb.y1, from.1, to.1);
                let k = |new: f64, old: f64| if old > 1e-9 { (new / old).max(0.0) } else { 1.0 };
                (k(x1 - x0, bb.width()), k(y1 - y0, bb.height()), x0, y0, false)
            }
            Fit::Scale(kx, ky) => (kx, ky, bb.center().x * kx, bb.center().y * ky, true),
        };
        let along = |x: f64, y: f64, fallback: f64| {
            let len = x.hypot(y);
            let k = (kx * x).hypot(ky * y) / len;
            if len > 1e-12 && k.is_finite() { k } else { fallback }
        };
        let size = (w * along(a, b, kx), h * along(c, d, ky));
        let nb = figcraft_doc::geom::transform_rect_bbox(lin, figcraft_doc::geom::Rect::new(0.0, 0.0, size.0, size.1));
        let at = if centred { nb.center() } else { figcraft_doc::geom::Point::new(nb.x0, nb.y0) };
        (size, Affine::translate((x0 - at.x, y0 - at.y)) * lin)
    }
}

struct LayerChildren<'s, 'a> {
    id: &'s str,
    inst: Option<&'s Inst<'a>>,
    base: &'s Kv<'a>,
    child_space: Affine,
    view: &'s View<'a>,
    object: &'s mut Obj,
    ty: &'s str,
    rest_id: &'s str,
    connector_snapshot: bool,
    connector_terminal: bool,
    depth: usize,
    included: Option<&'s BTreeSet<String>>,
    slot_content: Option<String>,
}

impl<'a> Mapper<'a> {
    /// Layer `id` (inside instance `inst` when expanding one) and its subtree. `space`: from the
    /// parent's coordinates to the enclosing frame's (groups and booleans place their children in
    /// the frame's space, like the REST JSON). `parent`: the parent's view (grid tracks).
    pub(crate) fn layer(
        &mut self,
        id: &str,
        inst: Option<&Inst<'a>>,
        space: Affine,
        parent: Option<&View<'a>>,
        fit: Option<Fit>,
        depth: usize,
    ) -> Option<Value> {
        self.layer_filtered(id, inst, space, parent, fit, depth, None)
    }

    /// [`Self::layer`], keeping only the layers in `included` (all when `None`) below the
    /// top-level one: the cached library definitions of an internal canvas.
    pub(super) fn layer_filtered(
        &mut self,
        id: &str,
        inst: Option<&Inst<'a>>,
        space: Affine,
        parent: Option<&View<'a>>,
        fit: Option<Fit>,
        depth: usize,
        included: Option<&BTreeSet<String>>,
    ) -> Option<Value> {
        if included.is_some_and(|ids| !ids.contains(id)) {
            return None;
        }
        if depth > MAX_DEPTH {
            self.warn(format!("layers nested deeper than {MAX_DEPTH} levels were skipped"));
            return None;
        }
        if self.count >= self.budget() {
            let max = self.max_nodes;
            self.warn(format!("the file has more than {max} layers; the rest were skipped"));
            return None;
        }
        let base = *self.nodes.get(id)?;
        let path = inst.map(|i| [i.path.as_slice(), &[id.to_string()]].concat());
        let entries: &[OverrideRow<'a>] = match (inst, &path) {
            (Some(i), Some(p)) => i.overrides.get(p).map(Vec::as_slice).unwrap_or_default(),
            _ => &[],
        };
        let mut view = View::of(base).with(entries.iter().map(|e| e.row));
        // A layer of a resized instance the file gives no derived layout for follows its
        // constraints from the main's geometry, then takes the size and place overridden on it
        // (auto layout places the others, as in the editor).
        let derived = entries.iter().any(|e| e.entry == Entry::Derived && (e.row.get("size").is_some() || e.row.get("transform").is_some()));
        let stacked = parent.and_then(|p| p.str("stackMode")).is_some_and(|m| matches!(m, "HORIZONTAL" | "VERTICAL" | "GRID"))
            && view.str("stackPositioning") != Some("ABSOLUTE");
        let main = View::of(base);
        let set = |k: &str| entries.iter().rev().filter(|e| e.entry != Entry::Stale).find_map(|e| e.row.get(k));
        if let (Some(f), false, false, Some((w, h))) = (fit, derived, stacked, main.size()) {
            let t = main.transform().unwrap_or_default();
            let (fw, ft) = f.apply(w, h, t, view.str("horizontalConstraint"), view.str("verticalConstraint"));
            view.fitted = Some((set("size").and_then(size).unwrap_or(fw), set("transform").and_then(matrix).unwrap_or(ft)));
        } else if entries.iter().any(|e| e.entry == Entry::Stale && (e.row.get("size").is_some() || e.row.get("transform").is_some()))
            && let Some(s) = set("size").and_then(size).or_else(|| main.size())
        {
            view.fitted = Some((s, set("transform").and_then(matrix).or_else(|| main.transform()).unwrap_or_default()));
        }
        let rest_id = match (inst, &path) {
            (Some(i), Some(p)) => format!("I{};{}", i.root, p.join(";")),
            _ => id.to_string(),
        };
        let own = view.str("type").unwrap_or("");
        let connector_snapshot = own == "CONNECTOR" && self.connector_snapshot_supported(&view);
        // Only a connector with a saved label needs the frame wrapper; an unlabelled one stays a single vector.
        let connector_terminal =
            connector_snapshot && (!connector_has_saved_label(&view) || depth >= MAX_DEPTH || self.count.saturating_add(1) >= self.budget());
        // A supported connector snapshot has its own note below instead of the generic stand-in one.
        let stand = stand_in(own).filter(|_| !connector_snapshot);
        let ty = Self::mapped_type(own, &view, connector_snapshot, connector_terminal, stand)?;
        if let Some((_, what)) = stand.filter(|_| own != "TEXT_PATH") {
            self.warn(format!("{} layers import as {what}", own.to_ascii_lowercase().replace('_', " ")));
        }
        if connector_snapshot {
            self.warn("connector layers import as static frame/vector snapshots with labels when present; live endpoint bindings are not retained");
        }
        self.count += 1;
        if own == "SYMBOL" && inst.is_none() {
            self.mapped_mains.insert(id.to_string());
        }
        let mut o = Obj::new();
        o.insert("id".into(), json!(rest_id));
        // Private provenance for the .fig importer; ordinary REST import ignores it.
        o.insert(
            crate::rest::FIG_SOURCE_FIELD.into(),
            json!({
                "origin": id,
                "expanded": inst.is_some(),
                "wireFrame": own == "FRAME",
                "wireInstance": own == "INSTANCE",
                "slotContent": inst.is_some_and(|i| i.slot_content),
                // A size the instance gives this layer itself (kept when the instance relays out).
                "sizeOverride": entries.iter().any(|e| e.entry == Entry::Override && e.row.get("size").is_some()),
            }),
        );
        o.insert("type".into(), json!(ty));
        copy_str(&mut o, &view, "name", "name");
        self.common(&mut o, &view, ty, space, parent);
        match ty {
            "TEXT" => {
                self.text(&mut o, &view);
                if own == "TEXT_PATH" {
                    self.text_path(&mut o, &view);
                }
            }
            "VECTOR" | "STAR" | "REGULAR_POLYGON" => self.geometry(&mut o, &view),
            "ELLIPSE" => {
                if let Some(a) = view.get("arcData") {
                    let arc: Obj =
                        ["startingAngle", "endingAngle", "innerRadius"].iter().filter_map(|k| Some((k.to_string(), json!(num(a, k)?)))).collect();
                    o.insert("arcData".into(), Value::Object(arc));
                }
            }
            "BOOLEAN_OPERATION" => {
                if let Some(op) = view.str("booleanOperation") {
                    o.insert("booleanOperation".into(), json!(if op == "XOR" { "EXCLUDE" } else { op }));
                }
            }
            _ => {}
        }
        if own == "CONNECTOR" && connector_snapshot {
            if connector_terminal {
                self.connector_snapshot_flatten(&mut o, &view, &rest_id, space);
            } else {
                o.insert("fills".into(), json!([]));
                o.insert("strokes".into(), json!([]));
                for key in ["styles", "strokeWeight", "strokeAlign", "strokeJoin", "strokeDashes", "strokeCap"] {
                    o.remove(key);
                }
                o.insert("clipsContent".into(), json!(false));
            }
        } else if own == "CONNECTOR" {
            self.connector(&mut o, &view, space);
        }
        if ty == "COMPONENT" || ty == "COMPONENT_SET" {
            self.component(&mut o, &view, &rest_id);
        }
        self.prop_refs(&mut o, &view, inst);
        let slot_content = if own == "FRAME" {
            prop_ref(&view, "SLOT_CONTENT_ID").and_then(|def| inst.and_then(|i| i.slot_contents.get(&self.prop_root(&def)).cloned()))
        } else {
            None
        };
        if slot_content.is_some()
            && let Some(source) = o.get_mut(crate::rest::FIG_SOURCE_FIELD).and_then(Value::as_object_mut)
        {
            source.insert("slotOverride".into(), json!(true));
        }
        let child_space = if matches!(ty, "GROUP" | "BOOLEAN_OPERATION") { space * view.transform().unwrap_or_default() } else { Affine::IDENTITY };
        let mut kids = self.map_layer_children(LayerChildren {
            id,
            inst,
            base,
            child_space,
            view: &view,
            object: &mut o,
            ty,
            rest_id: &rest_id,
            connector_snapshot,
            connector_terminal,
            depth,
            included,
            slot_content,
        });
        if stand.is_some_and(|s| s.0 == "FRAME") {
            if let Some(snapshot) = self.board_snapshot(&mut o, &view, own, &rest_id, depth) {
                kids.splice(0..0, snapshot);
            } else {
                kids.splice(0..0, self.shape_with_text(&mut o, &view, own, &rest_id));
                kids.extend(self.inner_text(&o, &view, &rest_id, own == "STICKY"));
            }
        }
        if !kids.is_empty() {
            o.insert("children".into(), Value::Array(kids));
        }
        Some(Value::Object(o))
    }

    fn map_layer_children(&mut self, context: LayerChildren<'_, 'a>) -> Vec<Value> {
        let LayerChildren {
            id,
            inst,
            base,
            child_space,
            view,
            object: o,
            ty,
            rest_id,
            connector_snapshot,
            connector_terminal,
            depth,
            included,
            slot_content,
        } = context;
        let mut kids = if connector_snapshot && !connector_terminal { self.connector_snapshot_children(view, rest_id) } else { Vec::new() };
        if connector_terminal {
            // The saved vector is flattened into the connector node when no child slot remains.
        } else if ty == "INSTANCE" {
            if let Some((sub, sym)) = self.instance(o, view, id, inst, rest_id) {
                let main = self.nodes.get(&sym).map(|m| View::of(m));
                let fit = Fit::frame(main.as_ref().and_then(View::size), view.size());
                // An instance that records no auto layout of its own has its main's.
                let view = match main {
                    Some(m) if view.get("stackMode").is_none() && m.get("stackMode").is_some() => {
                        let layered = View { fitted: view.fitted, ..m.with(view.layers.iter().copied()) };
                        self.auto_layout(o, &layered);
                        layered
                    }
                    _ => view.clone(),
                };
                for c in self.children(&sym) {
                    // The main's branch can live outside the cached context being mapped.
                    kids.extend(self.layer_filtered(&c, Some(&sub), child_space, Some(&view), fit, depth + 1, None));
                }
            }
        } else if let Some(content) = slot_content {
            let mut content_inst = inst.cloned();
            if let Some(content_inst) = &mut content_inst {
                content_inst.slot_content = true;
            }
            for c in self.children(&content) {
                kids.extend(self.layer_filtered(&c, content_inst.as_ref(), child_space, Some(view), None, depth + 1, included));
            }
        } else {
            // Inside an instance a layer's children are its main's: they follow it from the main's
            // size to its own.
            let fit = match (inst, ty) {
                (None, _) => None,
                (Some(_), "GROUP" | "BOOLEAN_OPERATION") => Fit::scale(View::of(base).size(), view.size()),
                (Some(_), _) => Fit::frame(View::of(base).size(), view.size()),
            };
            for c in self.children(id) {
                kids.extend(self.layer_filtered(&c, inst, child_space, Some(view), fit, depth + 1, included));
            }
        }
        kids
    }

    fn mapped_type<'s>(
        own: &'s str,
        view: &View<'a>,
        connector_snapshot: bool,
        connector_terminal: bool,
        stand: Option<(&'static str, &'static str)>,
    ) -> Option<&'s str> {
        match own {
            "ROUNDED_RECTANGLE" => Some("RECTANGLE"),
            "SYMBOL" => Some("COMPONENT"),
            "FRAME" if view.flag("isStateGroup") == Some(true) => Some("COMPONENT_SET"),
            "FRAME" if view.flag("resizeToFit") == Some(true) => Some("GROUP"),
            "VARIABLE" | "VARIABLE_SET" => None,
            "CONNECTOR" if connector_snapshot && connector_terminal => Some("VECTOR"),
            "CONNECTOR" if connector_snapshot => Some("FRAME"),
            t => Some(stand.map_or(t, |s| s.0)),
        }
    }

    /// FigJam notes and shapes keep their paints and text on generated template children,
    /// not on the outer node. Keep the saved layout as ordinary editable layers (including a
    /// note's author), without applying the background's opacity or transform to the wrapper.
    /// When no generated body survives, retain usable outer text alongside successful children.
    fn board_snapshot(&mut self, o: &mut Obj, v: &View<'a>, own: &str, rest_id: &str, depth: usize) -> Option<Vec<Value>> {
        if !matches!(own, "STICKY" | "SHAPE_WITH_TEXT") || v.get("nodeGenerationData").is_none() {
            return None;
        }
        if v.get("derivedImmutableFrameData").and_then(|d| num(d, "version")) != Some(1.0) {
            self.warn("note or shape generated layout is missing or unsupported; using its outer frame");
            return None;
        }
        let mut children = Vec::new();
        let mut has_body = false;
        for local_id in 0..if own == "STICKY" { 3 } else { 2 } {
            let style = generated_record(v, "nodeGenerationData", local_id);
            let bounds = generated_record(v, "derivedImmutableFrameData", local_id);
            let (Some(style), Some(bounds)) = (style, bounds) else {
                if style.is_some() || bounds.is_some() {
                    self.warn("note or shape generated child is missing its style or bounds; child skipped");
                }
                continue;
            };
            let child = View::of(style).with([bounds]);
            let chars = own_text(&child);
            if local_id != 0 && chars.is_none() {
                continue;
            }
            if valid_size(child.get("size")).is_none() || child.transform().is_none() {
                self.warn("note or shape generated child has invalid bounds; child skipped");
                continue;
            }
            if self.count >= self.budget() || depth >= MAX_DEPTH {
                self.warn("note or shape generated children exceed the layer or nesting limit; remaining children skipped");
                break;
            }
            let mut layer = Obj::new();
            let ty = if local_id == 0 { "RECTANGLE" } else { "TEXT" };
            layer.insert("id".into(), json!(format!("{rest_id};generated{local_id}")));
            layer.insert("type".into(), json!(ty));
            layer.insert("name".into(), json!(chars.map_or_else(|| "Background".into(), |c| c.chars().take(40).collect::<String>())));
            self.common(&mut layer, &child, ty, Affine::IDENTITY, None);
            if local_id == 0 {
                // Network decoding is only enabled for vectors; choose that type before
                // geometry sees a saved network, then restore the rectangle if it is unusable.
                if child.get("vectorData").is_some() {
                    layer.insert("type".into(), json!("VECTOR"));
                }
                self.geometry(&mut layer, &child);
                let outlined = |layer: &Obj| ["fillGeometry", "strokeGeometry", "vectorNetworkRef"].iter().any(|key| layer.contains_key(*key));
                if own == "SHAPE_WITH_TEXT" && !outlined(&layer) {
                    self.geometry(&mut layer, v);
                }
                if outlined(&layer) {
                    layer.insert("type".into(), json!("VECTOR"));
                } else {
                    layer.insert("type".into(), json!("RECTANGLE"));
                    if own == "SHAPE_WITH_TEXT" && !matches!(v.str("shapeWithTextType"), None | Some("SQUARE" | "ROUNDED_RECTANGLE")) {
                        self.warn("shape generated outline is missing or damaged; using its rectangle");
                    }
                }
                o.insert("fills".into(), json!([]));
                o.insert("strokes".into(), json!([]));
            } else {
                self.text(&mut layer, &child);
                if let Some(Value::Object(style)) = layer.get_mut("style") {
                    style.insert("textAutoResize".into(), json!("NONE"));
                }
            }
            self.count += 1;
            has_body |= local_id == 1;
            children.push(Value::Object(layer));
        }
        if depth < MAX_DEPTH && !has_body {
            children.extend(self.inner_text(o, v, rest_id, own == "STICKY"));
        }
        Some(children)
    }

    /// A non-rectangular shape-with-text's shape, as a vector under its frame (which gives up its
    /// paints to it).
    fn shape_with_text(&mut self, o: &mut Obj, v: &View<'a>, own: &str, rest_id: &str) -> Option<Value> {
        let shape = v.str("shapeWithTextType").unwrap_or("SQUARE");
        let outlined = !v.list("fillGeometry").is_empty() || !v.list("strokeGeometry").is_empty();
        if own != "SHAPE_WITH_TEXT" || matches!(shape, "SQUARE" | "ROUNDED_RECTANGLE") || !outlined || self.count >= self.budget() {
            return None;
        }
        let mut s = Obj::new();
        s.insert("id".into(), json!(format!("{rest_id};shape")));
        s.insert("type".into(), json!("VECTOR"));
        s.insert("name".into(), json!(shape.to_ascii_lowercase().replace('_', " ")));
        for k in ["size", "strokeWeight", "strokeAlign", "strokeJoin", "strokeDashes"] {
            if let Some(x) = o.get(k) {
                s.insert(k.into(), x.clone());
            }
        }
        for k in ["fills", "strokes"] {
            s.insert(k.into(), o.insert(k.into(), json!([])).unwrap_or_else(|| json!([])));
        }
        s.insert("relativeTransform".into(), rows(Affine::IDENTITY));
        self.geometry(&mut s, v);
        if !s.contains_key("fillGeometry") && !s.contains_key("strokeGeometry") {
            // Damaged outlines: the frame keeps its paints and box.
            for k in ["fills", "strokes"] {
                o.insert(k.into(), s.remove(k).unwrap_or_else(|| json!([])));
            }
            return None;
        }
        self.count += 1;
        Some(Value::Object(s))
    }

    /// The text a stand-in frame holds (a sticky note's, a shape's, a table cell's) as a text
    /// layer filling it, inset a little; centred unless it's a note.
    fn inner_text(&mut self, o: &Obj, v: &View<'a>, rest_id: &str, note: bool) -> Option<Value> {
        let chars = own_text(v)?;
        if self.count >= self.budget() {
            return None;
        }
        self.count += 1;
        let size = |k: &str| o.get("size").and_then(|s| s.get(k)).and_then(Value::as_f64).unwrap_or(0.0);
        let (w, h) = (size("x"), size("y"));
        let pad = (w.min(h) / 4.0).clamp(0.0, 16.0);
        let mut t = Obj::new();
        t.insert("id".into(), json!(format!("{rest_id};text")));
        t.insert("type".into(), json!("TEXT"));
        t.insert("name".into(), json!(chars.chars().take(40).collect::<String>()));
        t.insert("size".into(), json!({"x": (w - 2.0 * pad).max(1.0), "y": (h - 2.0 * pad).max(1.0)}));
        t.insert("relativeTransform".into(), rows(Affine::translate((pad, pad))));
        self.text(&mut t, v);
        if let Some(Value::Object(st)) = t.get_mut("style") {
            let (hor, ver) = if note { ("LEFT", "TOP") } else { ("CENTER", "CENTER") };
            st.entry("textAlignHorizontal").or_insert_with(|| json!(hor));
            st.entry("textAlignVertical").or_insert_with(|| json!(ver));
            st.insert("textAutoResize".into(), json!("NONE"));
        }
        Some(Value::Object(t))
    }

    /// Fields every layer type shares.
    pub(super) fn common(&mut self, o: &mut Obj, v: &View<'a>, ty: &str, space: Affine, parent: Option<&View<'a>>) {
        for (from, to) in [("visible", "visible"), ("locked", "locked"), ("mask", "isMask")] {
            if let Some(b) = v.flag(from) {
                o.insert(to.into(), json!(b));
            }
        }
        if let Some(m) = v.str("maskType") {
            o.insert("maskType".into(), json!(if m == "OUTLINE" { "VECTOR" } else { m }));
        }
        copy_num(o, v, "opacity", "opacity");
        if let Some(b) = v.str("blendMode").filter(|b| *b != "UNSET") {
            o.insert("blendMode".into(), json!(b));
        }
        if let Some((x, y)) = v.size() {
            o.insert("size".into(), json!({"x": x, "y": y}));
        }
        let t = v.transform().unwrap_or_default();
        o.insert("relativeTransform".into(), rows(space * t));
        // Paints, from the shared style when one is linked (the layer's own copy can be stale),
        // unless the layer's own paints are bound to variables: those bindings must not be lost.
        let mut styles = Obj::new();
        let own = |k: &str| Some(v.list(k)).filter(|l| l.iter().any(paint_bound));
        let fill_style = self.style(v, "styleIdForFill", "inheritFillStyleID", "fill", &mut styles);
        let fills = own("fillPaints")
            .unwrap_or_else(|| fill_style.map_or_else(|| v.list("fillPaints"), |s| s.get("fillPaints").map(Kv::list).unwrap_or_default()));
        o.insert("fills".into(), Value::Array(fills.iter().filter_map(|p| paint(p, &mut |w| self.warn(w))).collect()));
        let stroke_style = self.style(v, "styleIdForStrokeFill", "inheritFillStyleIDForStroke", "stroke", &mut styles);
        let strokes = own("strokePaints")
            .unwrap_or_else(|| stroke_style.map_or_else(|| v.list("strokePaints"), |s| s.get("fillPaints").map(Kv::list).unwrap_or_default()));
        o.insert("strokes".into(), Value::Array(strokes.iter().filter_map(|p| paint(p, &mut |w| self.warn(w))).collect()));
        let effect_style = self.style(v, "styleIdForEffect", "inheritEffectStyleID", "effect", &mut styles);
        let effects = effect_style.map_or_else(|| v.list("effects"), |s| s.get("effects").map(Kv::list).unwrap_or_default());
        if !effects.is_empty() {
            o.insert("effects".into(), Value::Array(effects.iter().filter_map(effect).collect()));
        }
        let grid_style = self.style(v, "styleIdForGrid", "inheritGridStyleID", "grid", &mut styles);
        let grids = grid_style.map_or_else(|| v.list("layoutGrids"), |s| s.get("layoutGrids").map(Kv::list).unwrap_or_default());
        if !grids.is_empty() {
            o.insert("layoutGrids".into(), Value::Array(grids.iter().map(layout_grid).collect()));
        }
        if ty == "TEXT" {
            self.style(v, "styleIdForText", "inheritTextStyleID", "text", &mut styles);
        }
        if !styles.is_empty() {
            o.insert("styles".into(), Value::Object(styles));
        }
        stroke(o, v);
        copy_num(o, v, "cornerRadius", "cornerRadius");
        copy_num(o, v, "cornerSmoothing", "cornerSmoothing");
        if v.flag("rectangleCornerRadiiIndependent") == Some(true) {
            let r = |k: &str| v.num(k).unwrap_or(0.0);
            o.insert(
                "rectangleCornerRadii".into(),
                json!([
                    r("rectangleTopLeftCornerRadius"),
                    r("rectangleTopRightCornerRadius"),
                    r("rectangleBottomRightCornerRadius"),
                    r("rectangleBottomLeftCornerRadius")
                ]),
            );
        }
        let constraint = |k: &str, min: &'static str, max: &'static str, both: &'static str| match v.str(k) {
            Some("CENTER") => "CENTER",
            Some("MAX" | "FIXED_MAX") => max,
            Some("STRETCH") => both,
            Some("SCALE") => "SCALE",
            _ => min,
        };
        o.insert(
            "constraints".into(),
            json!({
                "horizontal": constraint("horizontalConstraint", "LEFT", "RIGHT", "LEFT_RIGHT"),
                "vertical": constraint("verticalConstraint", "TOP", "BOTTOM", "TOP_BOTTOM"),
            }),
        );
        if matches!(ty, "FRAME" | "COMPONENT" | "COMPONENT_SET" | "INSTANCE" | "SECTION") {
            o.insert("clipsContent".into(), json!(!v.flag("frameMaskDisabled").unwrap_or(false)));
            self.auto_layout(o, v);
        }
        child_layout(o, v, parent);
        let exports: Vec<Value> = v.list("exportSettings").iter().map(export_setting).collect();
        if !exports.is_empty() {
            o.insert("exportSettings".into(), Value::Array(exports));
        }
        if let Some(b) = v.flag("sectionContentsHidden") {
            o.insert("sectionContentsHidden".into(), json!(b));
        }
        self.prototype(o, v);
        self.variable_bindings(o, v);
    }

    /// `fillGeometry` / `strokeGeometry` as SVG path data, and star/polygon parameters.
    pub(super) fn geometry(&mut self, o: &mut Obj, v: &View<'a>) {
        for key in ["fillGeometry", "strokeGeometry"] {
            let mut out = Vec::new();
            for p in v.list(key) {
                let Some(blob) = num(p, "commandsBlob").and_then(|i| self.blobs.get(i as usize)).copied() else { continue };
                let Some(d) = svg_path(blob) else {
                    self.warn("damaged vector geometry was skipped");
                    continue;
                };
                if d.is_empty() {
                    continue;
                }
                let winding = if p.get("windingRule").and_then(Kv::str) == Some("ODD") { "EVENODD" } else { "NONZERO" };
                out.push(json!({"path": d, "windingRule": winding}));
            }
            if !out.is_empty() {
                o.insert(key.into(), Value::Array(out));
            }
        }
        let vector = o.get("type").and_then(Value::as_str) == Some("VECTOR");
        if vector && !self.network(o, v) && !o.contains_key("fillGeometry") && !o.contains_key("strokeGeometry") && v.str("type") != Some("CONNECTOR")
        {
            self.warn("vectors without geometry are empty");
        }
        if !vector {
            copy_num(o, v, "count", "pointCount");
            copy_num(o, v, "starInnerScale", "innerRadius");
        }
    }

    /// The editable network of `vectorData` as `vectorNetwork` (the REST importer's extension),
    /// scaled to the layer's size, with per-point corner radii, handle mirroring and end caps.
    /// False when there's none or it's damaged (the outlines stand in).
    fn network(&mut self, o: &mut Obj, v: &View<'a>) -> bool {
        let Some(vd) = v.get("vectorData") else { return false };
        let Some(blob_id) = num(vd, "vectorNetworkBlob").map(|i| i as usize) else { return false };
        let Some(blob) = self.blobs.get(blob_id).copied() else { return false };
        let Some(net) = network::decode(blob) else {
            self.warn("damaged vector networks import as outlines");
            return false;
        };
        let styles: BTreeMap<u64, &Kv> =
            vd.get("styleOverrideTable").map(Kv::list).unwrap_or_default().iter().filter_map(|s| Some((num(s, "styleID")? as u64, s))).collect();
        let mut has_uninterpreted = false;
        let mut has_missing = false;
        for segment in &net.segments {
            if segment.style == 0 {
                continue;
            }
            if styles.contains_key(&u64::from(segment.style)) {
                has_uninterpreted = true;
            } else {
                has_missing = true;
            }
        }
        if has_uninterpreted {
            self.warn("style settings of some vector segments aren't supported; those segments use the layer's stroke");
        }
        if has_missing {
            self.warn("some vector segments refer to style settings missing from the file; they use the layer's stroke");
        }
        // An open path shows its fill, which regions can't hold: the outline keeps the look.
        let fills_shown =
            o.get("fills").and_then(Value::as_array).is_some_and(|f| f.iter().any(|p| p.get("visible").and_then(Value::as_bool) != Some(false)));
        if net.segments.is_empty() || (net.regions.is_empty() && fills_shown && o.contains_key("fillGeometry")) {
            return false;
        }
        let style = |id: u32| styles.get(&u64::from(id)).copied().filter(|_| id != 0);
        let scale = |k: &str| {
            let size = v.size().map(|s| if k == "x" { s.0 } else { s.1 });
            let norm = vd.get("normalizedSize").and_then(|s| num(s, k));
            size.zip(norm).filter(|(s, n)| s.is_finite() && *n > 1e-9).map_or(1.0, |(s, n)| s / n)
        };
        let (sx, sy) = (scale("x"), scale("y"));
        let vertices: Vec<Value> = net
            .vertices
            .iter()
            .map(|p| {
                let mut out = json!({"x": p.x * sx, "y": p.y * sy});
                let st = style(p.style);
                if let Some(r) = st.and_then(|s| num(s, "cornerRadius")).filter(|r| *r > 0.0) {
                    out["radius"] = json!(r);
                }
                match st.and_then(|s| s.get("handleMirroring")).and_then(Kv::str) {
                    Some("ANGLE") => out["mirror"] = json!("angle"),
                    Some("ANGLE_AND_LENGTH") => out["mirror"] = json!("angleAndLength"),
                    Some("NONE") => out["mirror"] = json!("none"),
                    _ => {}
                }
                if let Some(join @ ("MITER" | "BEVEL" | "ROUND")) = st.and_then(|s| s.get("strokeJoin")).and_then(Kv::str) {
                    out["join"] = json!(join.to_ascii_lowercase());
                }
                out
            })
            .collect();
        let segments: Vec<Value> =
            net.segments.iter().map(|s| json!({"a": s.a, "b": s.b, "ta": [s.ta.0 * sx, s.ta.1 * sy], "tb": [s.tb.0 * sx, s.tb.1 * sy]})).collect();
        let regions: Vec<Value> = net
            .regions
            .iter()
            .map(|r| {
                let mut region = json!({"loops": r.loops, "winding": if r.non_zero { "nonZero" } else { "evenOdd" }});
                if r.style != 0 {
                    match style(r.style) {
                        Some(s) => {
                            let fills = s.get("fillPaints").and_then(|own_fills| {
                                if own_fills.list().iter().any(paint_bound) {
                                    return Some(own_fills);
                                }
                                let linked_style = self
                                    .style_link(&View::of(s), "styleIdForFill", "inheritFillStyleID")
                                    .and_then(|id| self.nodes.get(&id).copied())
                                    .filter(|style| style.get("styleType").and_then(Kv::str) == Some("FILL"));
                                match linked_style {
                                    Some(style) => style.get("fillPaints"),
                                    None => Some(own_fills),
                                }
                            });
                            if let Some(fills) = fills {
                                if let Kv::Array(source_paints) = fills {
                                    let paint_cost = source_paints.iter().fold(source_paints.len(), |cost, paint| {
                                        cost.saturating_add(paint.get("stops").map(Kv::list).unwrap_or_default().len())
                                    });
                                    let oversized_stops =
                                        source_paints.iter().any(|paint| paint.get("stops").map(Kv::list).unwrap_or_default().len() > 256);
                                    if source_paints.len() > 256 || oversized_stops || self.region_paint_elements.saturating_add(paint_cost) > 200_000
                                    {
                                        self.warn("vector region paint data exceeded import limits and was skipped");
                                        return region;
                                    }
                                    self.region_paint_elements += paint_cost;
                                    let paints: Vec<Value> = source_paints.iter().filter_map(|p| paint(p, &mut |w| self.warn(w))).collect();
                                    if paints.len() != source_paints.len() {
                                        self.warn("invalid vector region paints were skipped");
                                    }
                                    if paints.iter().any(has_bound_variables) {
                                        self.warn("variable-bound vector region paints keep resolved appearance; live bindings were left out");
                                    }
                                    if s.get("styleIdForFill").is_some() {
                                        self.warn("linked vector region fill styles keep resolved appearance; live style links were left out");
                                    }
                                    region["_figRegionFills"] = Value::Array(paints);
                                } else {
                                    self.warn("invalid vector region fill paint list; layer fill used");
                                }
                            }
                        }
                        None => self.warn("vector region references a missing fill style; layer fill used"),
                    }
                }
                region
            })
            .collect();
        // An absent layer join is the source default.
        let layer_join = v.str("strokeJoin").unwrap_or("MITER");
        let point_join = |p: &network::Vertex| style(p.style).and_then(|s| s.get("strokeJoin"));
        let known = |j: &str| matches!(j, "MITER" | "BEVEL" | "ROUND");
        let unresolved_style = net.vertices.iter().any(|p| p.style != 0 && style(p.style).is_none());
        let unknown_join = net.vertices.iter().any(|p| point_join(p).is_some_and(|j| !j.str().is_some_and(known)));
        let unsupported_join = net.vertices.iter().any(|p| point_join(p).and_then(Kv::str).is_some_and(|j| known(j) && j != layer_join));
        if unresolved_style {
            self.warn("some vector points refer to point settings missing from the file; they use the layer's joins and plain corners");
        }
        if unknown_join {
            self.warn("vector points with unknown stroke joins use the layer join");
        }
        if unsupported_join && (o.contains_key("strokeDashes") || o.contains_key("strokeWidthProfile")) {
            self.warn("point stroke joins were preserved; dashed or variable-width strokes render with the layer join");
        }
        // The stroke starts at the first end point and stops at the last (as the editor draws it).
        let mut degree = vec![0usize; net.vertices.len()];
        for s in &net.segments {
            for i in [s.a, s.b] {
                if let Some(d) = degree.get_mut(i) {
                    *d += 1;
                }
            }
        }
        let ends: Vec<usize> = (0..degree.len()).filter(|i| degree.get(*i) == Some(&1)).collect();
        let cap =
            |i: Option<&usize>| i.and_then(|i| net.vertices.get(*i)).and_then(|p| style(p.style)).and_then(|s| s.get("strokeCap")).and_then(Kv::str);
        let (first, last) = (cap(ends.first()), cap(ends.last()));
        if first.is_some() || last.is_some() {
            let node = v.str("strokeCap").unwrap_or("NONE");
            o.insert("strokeCapStart".into(), json!(cap_name(first.unwrap_or(node))));
            o.insert("strokeCapEnd".into(), json!(cap_name(last.unwrap_or(node))));
        }
        // Every instance of a main shows the same network: it is written once and shared.
        let mut entry = json!({});
        entry["vertices"] = Value::Array(vertices);
        entry["segments"] = Value::Array(segments);
        entry["regions"] = Value::Array(regions.clone());
        if regions.iter().any(|r| r.get("_figRegionFills").is_some()) {
            o.insert("vectorNetwork".into(), entry);
            return true;
        }
        let key = (blob_id, sx.to_bits(), sy.to_bits(), std::ptr::from_ref(vd) as usize);
        let id = match self.network_ids.get(&key) {
            Some(id) => *id,
            None => {
                self.networks.push(crate::rest::network(&entry));
                self.network_ids.insert(key, self.networks.len() - 1);
                self.networks.len() - 1
            }
        };
        o.insert("vectorNetworkRef".into(), json!(id));
        true
    }
}

/// The text a layer that isn't a text layer carries (a note's, a shape's), unless blank.
pub(super) fn own_text<'a>(v: &View<'a>) -> Option<&'a str> {
    v.get("textData").and_then(|t| t.get("characters")).and_then(Kv::str).filter(|c| !c.trim().is_empty())
}

pub(super) fn valid_size(value: Option<&Kv<'_>>) -> Option<(f64, f64)> {
    let x = num(value?, "x")?;
    let y = num(value?, "y")?;
    (x.is_finite() && y.is_finite() && x >= 0.0 && y >= 0.0).then_some((x, y))
}

/// The REST name of a stroke cap.
pub(super) fn cap_name(cap: &str) -> &str {
    match cap {
        "ARROW_LINES" => "LINE_ARROW",
        "ARROW_EQUILATERAL" => "TRIANGLE_ARROW",
        c => c,
    }
}

/// The layer kind (and how the warning names it) that a layer the editor has no counterpart for
/// imports as: frames for notes, shapes, tables, slides and the like (their text as a text layer),
/// vectors for connectors and marker strokes, plain text for text on a path.
fn stand_in(ty: &str) -> Option<(&'static str, &'static str)> {
    Some(match ty {
        "STICKY"
        | "SHAPE_WITH_TEXT"
        | "CODE_BLOCK"
        | "TABLE"
        | "TABLE_CELL"
        | "STAMP"
        | "MEDIA"
        | "WIDGET"
        | "SLIDE"
        | "SLIDE_GRID"
        | "SLIDE_ROW"
        | "INTERACTIVE_SLIDE_ELEMENT"
        | "MODULE" => ("FRAME", "frames with their text"),
        "CONNECTOR" | "HIGHLIGHT" | "WASHI_TAPE" => ("VECTOR", "vectors"),
        "TEXT_PATH" => ("TEXT", "text off its path"),
        _ => return None,
    })
}

/// How a layer sizes and sits in an auto layout parent.
fn child_layout(o: &mut Obj, v: &View, parent: Option<&View>) {
    copy_num(o, v, "stackChildPrimaryGrow", "layoutGrow");
    if v.str("stackChildAlignSelf") == Some("STRETCH") {
        o.insert("layoutAlign".into(), json!("STRETCH"));
    }
    if v.str("stackPositioning") == Some("ABSOLUTE") {
        o.insert("layoutPositioning".into(), json!("ABSOLUTE"));
    }
    for (k, w, h) in [("minSize", "minWidth", "minHeight"), ("maxSize", "maxWidth", "maxHeight")] {
        if let Some(val) = v.get(k).and_then(|s| s.get("value")) {
            for (axis, to) in [("x", w), ("y", h)] {
                if let Some(x) = num(val, axis).filter(|x| *x > 0.0 && *x < 1e9) {
                    o.insert(to.into(), json!(x));
                }
            }
        }
    }
    if let Some(p) = parent.filter(|p| p.str("stackMode") == Some("GRID")) {
        for (anchor, list, to) in [("gridColumnAnchor", "gridColumns", "gridColumnAnchorIndex"), ("gridRowAnchor", "gridRows", "gridRowAnchorIndex")]
        {
            if let Some(i) = v.get(anchor).and_then(guid).and_then(|g| tracks(p, list).iter().position(|t| *t == g)) {
                o.insert(to.into(), json!(i));
            }
        }
        copy_num(o, v, "gridColumnSpan", "gridColumnSpan");
        copy_num(o, v, "gridRowSpan", "gridRowSpan");
    }
}

/// SVG path data of a geometry blob: commands `0` close, `1` move, `2` line, `3` quadratic and
/// `4` cubic, each followed by its points as little-endian `f32` pairs. `None` when damaged.
pub(super) fn svg_path(b: &[u8]) -> Option<String> {
    let mut out = String::new();
    let mut i = 0usize;
    let mut n = 0usize;
    while i < b.len() {
        n += 1;
        if n > MAX_COMMANDS {
            return None;
        }
        let (letter, points) = match *b.get(i)? {
            0 => ('Z', 0),
            1 => ('M', 1),
            2 => ('L', 1),
            3 => ('Q', 2),
            4 => ('C', 3),
            _ => return None,
        };
        i += 1;
        if !out.is_empty() {
            out.push(' ');
        }
        out.push(letter);
        for _ in 0..points * 2 {
            let bytes: [u8; 4] = b.get(i..i + 4)?.try_into().ok()?;
            let x = f32::from_le_bytes(bytes);
            if !x.is_finite() {
                return None;
            }
            out.push_str(&format!(" {x}"));
            i += 4;
        }
    }
    Some(out)
}
