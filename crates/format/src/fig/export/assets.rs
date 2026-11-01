use super::common::{layout_grid, node_field, prop_type, resolved_type, scope_name, variant_pairs};
use super::{
    AUX, Action, Affine, Bound, Color, Exporter, Fields, Kv, LAYERS, MAX_NODES, Node, NodeId, NodeKind, Overlay, OverlayPosition, Paint, StyleKind,
    TextData, VariableValue, arr, b, color, e, fl, guid, matrix, o, position, root_fields, s, u, xy,
};

impl<'d> Exporter<'d> {
    /// GUIDs for every layer (pages first, then depth-first), component property definitions,
    /// flow starting points and overlay settings, before anything refers to them.
    pub(super) fn assign(&mut self) {
        let doc = self.doc;
        let mut stack: Vec<NodeId> = doc.pages.iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            let Some(n) = doc.node(id) else { continue };
            if self.ids.len() >= MAX_NODES {
                self.warn("the document has more layers than a .fig file is read with; the rest were left out");
                break;
            }
            let local = u32::try_from(self.ids.len() + 1).unwrap_or(u32::MAX);
            self.ids.insert(id, local);
            // An instance's layers aren't written (its main says them), so nothing may refer to them.
            let mirrored = matches!(&n.kind, NodeKind::Instance { component, .. } if doc.node(*component).is_some_and(|c| matches!(c.kind, NodeKind::Component { .. })));
            if !mirrored {
                stack.extend(n.children.iter().rev());
            }
            match &n.kind {
                NodeKind::Component { props, .. } => {
                    let set = self.set_of(id);
                    let owner = set.unwrap_or(id);
                    for (name, d) in props {
                        if let Some(ty) = prop_type(d) {
                            let key = (owner, name.clone(), ty);
                            if !self.defs.contains_key(&key) {
                                let def = self.next_aux();
                                self.defs.insert(key, def);
                            }
                            if set.is_some() {
                                let child = self.next_aux();
                                self.child_defs.insert((id, name.clone(), ty), child);
                            }
                        }
                    }
                    if let Some(set) = set {
                        for (prop, _) in variant_pairs(&n.name) {
                            let key = (set, prop, "VARIANT");
                            if !self.defs.contains_key(&key) {
                                let def = self.next_aux();
                                self.defs.insert(key, def);
                            }
                        }
                    }
                }
                NodeKind::Page { flows, .. } => {
                    for (i, f) in flows.iter().enumerate() {
                        self.starts.entry(f.start).or_insert((i, f.name.clone()));
                    }
                }
                _ => {}
            }
            for a in n.interactions.iter().flat_map(|it| figcraft_doc::proto::walk(&it.actions)) {
                if let Action::OpenOverlay { dest, position, close_on_outside, background, anchor, .. } = a {
                    let position = if !anchor.is_screen() && *position != OverlayPosition::Manual { OverlayPosition::Manual } else { *position };
                    let o = Overlay { position, close_on_outside: *close_on_outside, background: *background };
                    match self.overlays.get(dest) {
                        Some(old) if (old.position, old.close_on_outside, old.background) != (o.position, o.close_on_outside, o.background) => {
                            self.warn("overlays opened with different settings keep the first one's");
                        }
                        Some(_) => {}
                        None => {
                            self.overlays.insert(*dest, o);
                        }
                    }
                }
            }
        }
        // The page that holds styles and variables takes the next id, then their nodes.
        let mut next = u32::try_from(self.ids.len() + 1).unwrap_or(u32::MAX);
        let mut node = || {
            next = next.saturating_add(1);
            next
        };
        for st in &doc.styles {
            self.style_ids.entry(st.id.as_str()).or_insert_with(&mut node);
        }
        for c in &doc.variables.collections {
            self.set_ids.entry(c.id.as_str()).or_insert_with(&mut node);
        }
        for v in &doc.variables.variables {
            if self.set_ids.contains_key(v.collection.as_str()) {
                self.var_ids.entry(v.id.as_str()).or_insert_with(&mut node);
            }
        }
        for c in &doc.variables.collections {
            for m in &c.modes {
                if !self.mode_ids.contains_key(&(c.id.as_str(), m.id.as_str())) {
                    let id = self.next_aux();
                    self.mode_ids.insert((c.id.as_str(), m.id.as_str()), id);
                }
            }
        }
    }

    /// The hidden page after the `pages` real ones that holds the shared styles, the variable
    /// collections and the variables, each a node of its own.
    pub(super) fn assets(&mut self, pages: usize) {
        let doc = self.doc;
        if self.style_ids.is_empty() && self.set_ids.is_empty() {
            return;
        }
        let page = u32::try_from(self.ids.len() + 1).unwrap_or(u32::MAX);
        let mut f: Fields = vec![
            ("guid", guid(LAYERS, page)),
            ("phase", e("CREATED")),
            ("parentIndex", o(vec![("guid", guid(0, 0)), ("position", s(&position(pages)))])),
            ("type", e("CANVAS")),
            ("name", s("Internal Only Canvas")),
        ];
        f.extend(root_fields().into_iter().map(|(k, v)| if k == "visible" { (k, b(false)) } else { (k, v) }));
        f.push(("internalOnly", b(true)));
        self.nodes.push(o(f));
        let mut index = 0usize;
        let mut head = |local: u32, ty: &'static str, name: &str| -> Fields {
            index += 1;
            vec![
                ("guid", guid(LAYERS, local)),
                ("phase", e("CREATED")),
                ("parentIndex", o(vec![("guid", guid(LAYERS, page)), ("position", s(&position(index)))])),
                ("type", e(ty)),
                ("name", s(name)),
                ("sortPosition", s(&position(index))),
            ]
        };
        for st in &doc.styles {
            let Some(&local) = self.style_ids.get(st.id.as_str()) else { continue };
            // A sample layer that shows the style: a square for paints and effects, a frame for
            // grids, two letters for text.
            let (ty, kind) = match &st.kind {
                StyleKind::Paint { .. } => ("ROUNDED_RECTANGLE", "FILL"),
                StyleKind::Effect { .. } => ("ROUNDED_RECTANGLE", "EFFECT"),
                StyleKind::Grid { .. } => ("FRAME", "GRID"),
                StyleKind::Text { .. } => ("TEXT", "TEXT"),
            };
            let mut f = head(local, ty, &st.name);
            f.extend([
                ("styleType", e(kind)),
                ("visible", b(true)),
                ("opacity", fl(1.0)),
                ("size", xy(100.0, 100.0)),
                ("transform", matrix(Affine::IDENTITY)),
                ("strokeWeight", fl(0.0)),
                ("strokeAlign", e("INSIDE")),
                ("strokeJoin", e("MITER")),
            ]);
            if !st.description.is_empty() {
                f.push(("description", s(&st.description)));
            }
            // The style's variable bindings ride on its sample layer, as a layer's do.
            let none = if st.bound.is_empty() {
                Bound::default()
            } else {
                let mut sample = Node::new(NodeId(0), NodeKind::Rectangle, "", 0.0, 0.0);
                if let StyleKind::Text { style } = &st.kind {
                    sample.kind = NodeKind::Text(TextData { style: style.as_text_style(), ..TextData::new("Ag") });
                }
                sample.bound.clone_from(&st.bound);
                self.bound(&sample)
            };
            let entries = none.entries.clone();
            if !entries.is_empty() {
                f.push(("variableConsumptionMap", o(vec![("entries", arr(entries))])));
            }
            match &st.kind {
                StyleKind::Paint { paints } => f.push(("fillPaints", self.paints(paints, true, &none))),
                StyleKind::Effect { effects } => {
                    let list = effects.iter().enumerate().map(|(i, x)| self.effect(x, i, &none)).collect();
                    f.extend([("fillPaints", arr(Vec::new())), ("effects", arr(list))]);
                }
                StyleKind::Grid { grids } => {
                    let list = grids.iter().enumerate().map(|(i, g)| layout_grid(g, i, &none)).collect();
                    f.extend([("fillPaints", arr(Vec::new())), ("layoutGrids", arr(list))]);
                }
                StyleKind::Text { style } => {
                    let td = TextData { style: style.as_text_style(), ..TextData::new("Ag") };
                    let fills = [Paint::solid(Color::BLACK)];
                    f.push(("fillPaints", self.paints(style.fills.as_deref().unwrap_or(&fills), true, &none)));
                    self.text(&td, &fills, &mut f);
                }
            }
            self.nodes.push(o(f));
        }
        for c in &doc.variables.collections {
            let Some(&local) = self.set_ids.get(c.id.as_str()) else { continue };
            let mut f = head(local, "VARIABLE_SET", &c.name);
            let modes = c.modes.iter().enumerate().filter_map(|(i, m)| {
                let id = self.mode_ids.get(&(c.id.as_str(), m.id.as_str()))?;
                Some(o(vec![("id", guid(AUX, *id)), ("name", s(&m.name)), ("sortPosition", s(&position(i)))]))
            });
            f.push(("variableSetModes", arr(modes.collect())));
            self.nodes.push(o(f));
            if c.extends.is_some() {
                self.warn("extended variable collections were written as collections of their own");
            }
        }
        for v in &doc.variables.variables {
            let ids = (self.var_ids.get(v.id.as_str()), self.set_ids.get(v.collection.as_str()));
            let (Some(&local), Some(&set), Some(c)) = (ids.0, ids.1, doc.variables.collections.iter().find(|c| c.id == v.collection)) else {
                self.warn("variables whose collection isn't in the document were left out");
                continue;
            };
            let resolved = resolved_type(v.kind);
            let mut values = Vec::new();
            for m in &c.modes {
                let (Some(mode), Some(val)) = (self.mode_ids.get(&(c.id.as_str(), m.id.as_str())), v.values.get(&m.id)) else { continue };
                // The Figma binary has no encoding FigCraft writes for calculations: the colour
                // they show in this mode goes out as a plain colour value.
                let Some(val) = doc.variables.flattened(v, &m.id, val) else {
                    self.warn(crate::CALCULATIONS_LEFT_OUT);
                    continue;
                };
                let (ty, value) = match &val {
                    VariableValue::Color(x) => ("COLOR", ("colorValue", color(*x))),
                    VariableValue::Number(x) => ("FLOAT", ("floatValue", fl(*x))),
                    // NULs can't be stored in a string.
                    VariableValue::String(x) => ("STRING", ("textValue", s(&x.replace('\0', "")))),
                    VariableValue::Boolean(x) => ("BOOLEAN", ("boolValue", b(*x))),
                    VariableValue::Alias(a) => match self.var_ids.get(a.as_str()) {
                        Some(t) => ("ALIAS", ("alias", o(vec![("guid", guid(LAYERS, *t))]))),
                        None => {
                            self.warn("variable aliases to variables that aren't in the document were left out");
                            continue;
                        }
                    },
                    // `flattened` leaves none: this format has no shape for a calculation.
                    VariableValue::ColorAlpha { .. } | VariableValue::ColorMix { .. } => continue,
                };
                let data = o(vec![("value", o(vec![value])), ("dataType", e(ty)), ("resolvedDataType", e(resolved))]);
                values.push(o(vec![("modeID", guid(AUX, *mode)), ("variableData", data)]));
            }
            let mut f = head(local, "VARIABLE", &v.name);
            f.extend([
                ("variableSetID", o(vec![("guid", guid(LAYERS, set))])),
                ("variableResolvedType", e(resolved)),
                ("variableDataValues", o(vec![("entries", arr(values))])),
            ]);
            if !v.description.is_empty() {
                f.push(("description", s(&v.description)));
            }
            if let Some(scopes) = &v.scopes {
                let known: Vec<Kv> = scopes.iter().filter_map(|sc| scope_name(sc)).map(e).collect();
                if known.len() < scopes.len() {
                    self.warn("variable scopes the format has no name for were left out");
                }
                f.push(("variableScopes", arr(known)));
            }
            if v.hidden_from_publishing || !v.code_syntax.is_empty() || v.extends.is_some() {
                self.warn("variables' publishing, code syntax and inheritance settings were left out");
            }
            self.nodes.push(o(f));
        }
    }

    /// A reference to variable `var` (a stored id: no name stands in for it, #1613) as variable data.
    fn alias(&mut self, var: &str) -> Option<Kv<'static>> {
        let found = self.doc.variables.get_by_id(var).and_then(|v| Some((v.kind, *self.var_ids.get(v.id.as_str())?)));
        let Some((kind, local)) = found else {
            self.warn("bindings to variables that aren't in the document were written as their values");
            return None;
        };
        Some(o(vec![
            ("value", o(vec![("alias", o(vec![("guid", guid(LAYERS, local))]))])),
            ("dataType", e("ALIAS")),
            ("resolvedDataType", e(resolved_type(kind))),
        ]))
    }

    /// Layer `n`'s variable bindings, sorted into the fields that carry them. What the format has
    /// no field for stays as the value the layer already shows, with a warning.
    pub(super) fn bound(&mut self, n: &Node) -> Bound {
        const BAKED: &str = "variables bound to properties the format has no field for were written as their values";
        let mut out = Bound::default();
        for (key, var) in &n.bound {
            let parts: Vec<&str> = key.split('/').collect();
            let index = |i: &str| i.parse::<usize>().ok().filter(|_| i.bytes().all(|c| c.is_ascii_digit()));
            let fields: &[&'static str] = match (parts.as_slice(), key.as_str()) {
                (_, "fill" | "fills" | "fills/0" | "stroke") => {
                    if let Some(a) = self.alias(var) {
                        out.paints.insert((key != "stroke", 0), a);
                    }
                    continue;
                }
                ([list @ ("fills" | "strokes"), i, "color"], _) => {
                    if let (Some(i), Some(a)) = (index(i), self.alias(var)) {
                        out.paints.insert((*list == "fills", i), a);
                    }
                    continue;
                }
                ([list @ ("fills" | "strokes"), i, "stops", j, "color"], _) => {
                    if let (Some(i), Some(j), Some(a)) = (index(i), index(j), self.alias(var)) {
                        out.stops.insert((*list == "fills", i, j), a);
                    }
                    continue;
                }
                ([list @ ("effects" | "layoutGrids"), i, field], _) => {
                    let names: &[&'static str] = if *list == "effects" {
                        &["color", "radius", "spread", "offsetX", "offsetY", "startRadius"]
                    } else {
                        &["size", "count", "gutter", "offset"]
                    };
                    match (index(i), names.iter().find(|f| *f == field)) {
                        (Some(i), Some(name)) => {
                            if let Some(a) = self.alias(var) {
                                if *list == "effects" { &mut out.effects } else { &mut out.grids }.insert((i, *name), a);
                            }
                        }
                        _ => self.warn(BAKED),
                    }
                    continue;
                }
                (_, "width") => &["WIDTH"],
                (_, "height") => &["HEIGHT"],
                (_, "gap") => &["STACK_SPACING"],
                (_, "crossGap") => &["STACK_COUNTER_SPACING"],
                (_, "paddingLeft") => &["STACK_PADDING_LEFT"],
                (_, "paddingTop") => &["STACK_PADDING_TOP"],
                (_, "paddingRight") => &["STACK_PADDING_RIGHT"],
                (_, "paddingBottom") => &["STACK_PADDING_BOTTOM"],
                (_, "paddingHorizontal") => &["STACK_PADDING_LEFT", "STACK_PADDING_RIGHT"],
                (_, "paddingVertical") => &["STACK_PADDING_TOP", "STACK_PADDING_BOTTOM"],
                (_, "padding") => &["STACK_PADDING_LEFT", "STACK_PADDING_TOP", "STACK_PADDING_RIGHT", "STACK_PADDING_BOTTOM"],
                (_, "cornerRadius") => &[
                    "CORNER_RADIUS",
                    "RECTANGLE_TOP_LEFT_CORNER_RADIUS",
                    "RECTANGLE_TOP_RIGHT_CORNER_RADIUS",
                    "RECTANGLE_BOTTOM_LEFT_CORNER_RADIUS",
                    "RECTANGLE_BOTTOM_RIGHT_CORNER_RADIUS",
                ],
                (_, "topLeftRadius") => &["RECTANGLE_TOP_LEFT_CORNER_RADIUS"],
                (_, "topRightRadius") => &["RECTANGLE_TOP_RIGHT_CORNER_RADIUS"],
                (_, "bottomLeftRadius") => &["RECTANGLE_BOTTOM_LEFT_CORNER_RADIUS"],
                (_, "bottomRightRadius") => &["RECTANGLE_BOTTOM_RIGHT_CORNER_RADIUS"],
                (_, "opacity") => &["OPACITY"],
                (_, "visible") => &["VISIBLE"],
                (_, "characters") => &["TEXT_DATA"],
                (_, "strokeWeight") => &["STROKE_WEIGHT"],
                (_, "strokeTopWeight") => &["BORDER_TOP_WEIGHT"],
                (_, "strokeRightWeight") => &["BORDER_RIGHT_WEIGHT"],
                (_, "strokeBottomWeight") => &["BORDER_BOTTOM_WEIGHT"],
                (_, "strokeLeftWeight") => &["BORDER_LEFT_WEIGHT"],
                (_, "minWidth") => &["MIN_WIDTH"],
                (_, "maxWidth") => &["MAX_WIDTH"],
                (_, "minHeight") => &["MIN_HEIGHT"],
                (_, "maxHeight") => &["MAX_HEIGHT"],
                (_, "fontSize") => &["FONT_SIZE"],
                (_, "fontFamily") => &["FONT_FAMILY"],
                (_, "fontStyle") => &["FONT_STYLE"],
                (_, "letterSpacing") => &["LETTER_SPACING"],
                (_, "lineHeight") => &["LINE_HEIGHT"],
                (_, "paragraphSpacing") => &["PARAGRAPH_SPACING"],
                (_, "paragraphIndent") => &["PARAGRAPH_INDENT"],
                _ => {
                    self.warn(BAKED);
                    continue;
                }
            };
            // Each entry names the bound property and the number of the layer field that holds it.
            for field in fields {
                if let Some(a) = self.alias(var) {
                    let mut entry: Fields = node_field(field).into_iter().map(|id| ("nodeField", u(id))).collect();
                    entry.extend([("variableData", a), ("variableField", e(field))]);
                    out.entries.push(o(entry));
                }
            }
        }
        out
    }
}
