use super::common::{
    bool_data, changed, prop_type, resolved_alike, scaled, sort_position, symbol_data, text_data, text_value, var_data, variant_pairs,
};
use super::{AUX, BTreeMap, Exporter, Fields, Kv, MAX_NODES, Node, NodeId, NodeKind, PropDef, Value, arr, b, e, fl, guid, o, s};

impl<'d> Exporter<'d> {
    /// A component property definition of `owner` (a component, or a set), `index`-th in its list.
    /// The default value is written twice: as `initialValue` and as `varValue`, which is what
    /// readers that know variables take.
    pub(super) fn prop_def(&mut self, owner: NodeId, name: &str, d: &PropDef, index: usize) -> Option<Kv<'static>> {
        let ty = prop_type(d)?;
        let id = *self.defs.get(&(owner, name.to_string(), ty))?;
        let (value, var) = match d {
            PropDef::Boolean { default } => (o(vec![("boolValue", b(*default))]), bool_data(*default)),
            PropDef::Text { default } => (o(vec![("textValue", text_value(default))]), text_data(default)),
            PropDef::InstanceSwap { default, preferred } => {
                if !preferred.is_empty() {
                    self.warn("preferred values of instance swap properties were left out");
                }
                let main = self.guid_of(*default)?;
                (o(vec![("guidValue", main.clone())]), symbol_data(main))
            }
            PropDef::Variant { .. } | PropDef::Slot { .. } => return None,
        };
        Some(o(vec![
            ("id", guid(AUX, id)),
            ("name", s(name)),
            ("initialValue", value),
            ("sortPosition", s(&sort_position(index))),
            ("type", e(ty)),
            ("preferredValues", o(Vec::new())),
            ("varValue", var),
        ]))
    }

    /// A component set's property definitions: its variant properties (from the variants' names,
    /// in the order they first appear, with their values in order) and the other properties its
    /// variants share; written on the set, as the format keeps them.
    pub(super) fn set_props(&mut self, set: &Node, f: &mut Fields) {
        let doc = self.doc;
        let variants: Vec<&Node> =
            set.children.iter().filter_map(|c| doc.node(*c)).filter(|c| matches!(c.kind, NodeKind::Component { .. })).collect();
        let mut order: Vec<(String, Vec<String>)> = Vec::new();
        for v in &variants {
            for (prop, value) in variant_pairs(&v.name) {
                let i = match order.iter().position(|(p, _)| *p == prop) {
                    Some(i) => i,
                    None => {
                        order.push((prop, Vec::new()));
                        order.len() - 1
                    }
                };
                if let Some((_, values)) = order.get_mut(i)
                    && !values.contains(&value)
                {
                    values.push(value);
                }
            }
        }
        let mut defs = Vec::new();
        for (prop, _) in &order {
            if let Some(&id) = self.defs.get(&(set.id, prop.clone(), "VARIANT")) {
                defs.push(o(vec![
                    ("id", guid(AUX, id)),
                    ("name", s(prop)),
                    ("initialValue", o(Vec::new())),
                    ("sortPosition", s(&sort_position(defs.len()))),
                    ("type", e("VARIANT")),
                    ("preferredValues", o(Vec::new())),
                    ("varValue", var_data(("textValue", s("")), "STRING", "STRING")),
                ]));
            }
        }
        // The format keeps one default per property of a set (a variant's copy holds none): the first
        // variant's, with a warning when another variant's differs.
        // A name used with two types is two properties (their definitions have their own ids).
        let mut seen: BTreeMap<(String, &'static str), PropDef> = BTreeMap::new();
        for v in &variants {
            let NodeKind::Component { props, prop_order, .. } = &v.kind else { continue };
            for (name, d) in figcraft_doc::ordered_props(props, prop_order) {
                let Some(ty) = prop_type(d) else { continue };
                match seen.get(&(name.clone(), ty)) {
                    Some(first) if first != d => self.warn("variants with different defaults for one property were written with the first variant's"),
                    Some(_) => {}
                    None => {
                        seen.insert((name.clone(), ty), d.clone());
                        let index = defs.len();
                        defs.extend(self.prop_def(set.id, name, d, index));
                    }
                }
            }
        }
        if !defs.is_empty() {
            f.push(("componentPropDefs", arr(defs)));
        }
        if !order.is_empty() {
            let orders = order.into_iter().map(|(p, values)| o(vec![("property", s(&p)), ("values", arr(values.iter().map(|x| s(x)).collect()))]));
            f.push(("stateGroupPropertyValueOrders", arr(orders.collect())));
        }
    }

    /// Which property controls a layer's `field`: the entry of its parameter consumption map and
    /// its component property reference (readers take one or the other), both naming the
    /// definition of the component the layer is in (a variant's own copy, in a set).
    pub(super) fn prop_ref(&mut self, n: &Node, field: &str, prop: &str) -> Option<(Kv<'static>, Kv<'static>)> {
        let (var_field, ty, resolved) = match field {
            "visible" => ("VISIBLE", "BOOL", "BOOLEAN"),
            "characters" => ("TEXT_DATA", "TEXT", "TEXT_DATA"),
            "mainComponent" => ("OVERRIDDEN_SYMBOL_ID", "INSTANCE_SWAP", "SYMBOL_ID"),
            _ => {
                self.warn("slot layers were written as plain layers");
                return None;
            }
        };
        let id = self.def_for(self.component_of(n)?, prop, ty)?;
        let data = o(vec![
            ("value", o(vec![("propRefValue", o(vec![("defId", guid(AUX, id))]))])),
            ("dataType", e("PROP_REF")),
            ("resolvedDataType", e(resolved)),
        ]);
        let entry = o(vec![("variableData", data), ("variableField", e(var_field))]);
        let reference = o(vec![("defID", guid(AUX, id)), ("componentPropNodeField", e(var_field))]);
        Some((entry, reference))
    }

    /// An instance: its main, the fields it and its layers override, the size, place and outline
    /// its layers get from that (derived data), its property values. `f`: its own fields so far.
    pub(super) fn instance(&mut self, n: &Node, main: NodeId, props: &BTreeMap<String, Value>, f: &mut Fields) {
        // Which properties control the instance itself (a nested instance in a main component) is
        // its own too, not an override of its main.
        const OWN: [&str; 9] =
            ["guid", "phase", "parentIndex", "type", "transform", "fillGeometry", "strokeGeometry", "parameterConsumptionMap", "componentPropRefs"];
        let doc = self.doc;
        let (Some(m), Some(symbol)) = (doc.node(main), self.guid_of(main)) else { return };
        // What the instance itself changes from its main (its size, its paints, …) is an override
        // addressed by the main's id: a reader takes everything else from the main. Where it sits
        // is its own.
        let mut root_theirs = Vec::new();
        self.layer(m, m.parent.and_then(|p| doc.node(p)), &mut root_theirs, true);
        root_theirs.retain(|(k, _)| !OWN.contains(k));
        // An instance is named after its component (the set, for a variant) until renamed.
        let set = m.parent.and_then(|p| doc.node(p)).filter(|p| matches!(p.kind, NodeKind::ComponentSet { .. }));
        let named = set.is_none_or(|s| s.name != n.name);
        let root_mine: Fields = f.iter().filter(|(k, _)| !OWN.contains(k) && (named || *k != "name")).cloned().collect();
        // Each layer below: its id path, the fields that differ from the main's layer, that layer's
        // fields, whether its size is its own (a nested instance, a size override) and the
        // component a nested instance was swapped for.
        struct Row {
            path: Kv<'static>,
            changed: Fields,
            theirs: Fields,
            sized: bool,
            swap: Option<Kv<'static>>,
        }
        let mut rows: Vec<Row> = Vec::new();
        let mut stack: Vec<(NodeId, NodeId)> = n.children.iter().rev().map(|c| (*c, n.id)).collect();
        let mut seen = 0usize;
        while let Some((id, parent)) = stack.pop() {
            let Some(d) = doc.node(id) else { continue };
            seen += 1;
            if seen > MAX_NODES {
                break;
            }
            stack.extend(d.children.iter().rev().map(|c| (*c, id)));
            let path = self.layer_path(d, n.id, main, 0);
            let guids: Option<Vec<Kv>> = path.and_then(|p| p.iter().map(|x| self.guid_of(*x)).collect());
            let (Some(guids), Some(src)) = (guids, d.source.and_then(|src| doc.node(src))) else {
                self.warn("instance layers that don't come from their main component were left out");
                continue;
            };
            let (mut mine, mut theirs) = (Vec::new(), Vec::new());
            self.layer(d, doc.node(parent), &mut mine, true);
            self.layer(src, src.parent.and_then(|p| doc.node(p)), &mut theirs, true);
            // A nested instance swapped for another component.
            let swap = match (&d.kind, &src.kind) {
                (NodeKind::Instance { component, .. }, NodeKind::Instance { component: was, .. }) if component == was => None,
                (NodeKind::Instance { component, .. }, _) => self.guid_of(*component),
                _ => None,
            };
            let sized = matches!(d.kind, NodeKind::Instance { .. }) || d.overrides.contains("size");
            rows.push(Row { path: o(vec![("guids", arr(guids))]), changed: changed(mine, &theirs), theirs, sized, swap });
        }
        // An instance scaled as a whole (the Scale tool) is `factor` times its main in every length:
        // sizes, places and stroke weights, its own and its layers'. A resized one keeps its weights.
        let field = |fields: &Fields, key: &str| fields.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone());
        let factor = Some(n.width / m.width).filter(|k| k.is_finite() && *k > 0.0 && (k - 1.0).abs() > 1e-6).filter(|k| {
            let grown = |mine: &Fields, theirs: &Fields| {
                let weight = field(theirs, "strokeWeight").and_then(|w| w.f64()).unwrap_or(0.0);
                (weight == 0.0 || field(mine, "strokeWeight").is_some_and(|w| scaled(&w, &fl(weight), *k, 0.0)))
                    && field(mine, "size").zip(field(theirs, "size")).is_some_and(|(a, b)| scaled(&a, &b, *k, 1.0))
                    && field(mine, "transform").zip(field(theirs, "transform")).is_none_or(|(a, b)| scaled(&a, &b, *k, 1.0))
            };
            (n.height / m.height - k).abs() <= 1e-6 * k.max(1.0)
                && grown(&changed(root_mine.clone(), &root_theirs), &root_theirs)
                && rows.iter().all(|r| grown(&r.changed, &r.theirs))
        });
        // What follows from the overrides goes with the derived data: a layer's size, place and
        // outline (a vector's network is the main's, drawn at the layer's size), the colours its
        // variables give here, and what the scale factor makes of the main's values.
        let follows = |k: &str, v: &Kv<'static>, theirs: &Fields| {
            ["size", "transform", "fillGeometry", "strokeGeometry", "vectorData"].contains(&k)
                || resolved_alike(k, v, theirs)
                || factor.is_some_and(|x| field(theirs, k).is_some_and(|t| scaled(v, &t, x, 0.0)))
        };
        let mut overrides = Vec::new();
        let mut derived = Vec::new();
        let mut of: Fields = vec![("guidPath", o(vec![("guids", arr(vec![symbol.clone()]))]))];
        of.extend(changed(root_mine, &root_theirs).into_iter().filter(|(k, v)| *k == "size" || !follows(k, v, &root_theirs)));
        if of.len() > 1 {
            overrides.push(o(of));
        }
        for r in rows {
            let (mut of, mut df): (Fields, Fields) = (vec![("guidPath", r.path.clone())], vec![("guidPath", r.path)]);
            for (k, v) in r.changed {
                let follows = follows(k, &v, &r.theirs);
                // A nested instance's size is its own to set, as is a size the layer overrides.
                if follows && r.sized && k == "size" {
                    of.push((k, v.clone()));
                }
                if follows { &mut df } else { &mut of }.push((k, v));
            }
            if df.len() > 1 {
                derived.push(o(df));
            }
            of.extend(r.swap.map(|g| ("overriddenSymbolID", g)));
            if of.len() > 1 {
                overrides.push(o(of));
            }
        }
        f.push(("symbolData", o(vec![("symbolID", symbol), ("symbolOverrides", arr(overrides)), ("uniformScaleFactor", fl(factor.unwrap_or(1.0)))])));
        if !derived.is_empty() {
            f.push(("derivedSymbolData", arr(derived)));
        }
        let mut assignments = Vec::new();
        for (name, v) in props {
            let (ty, value, var) = match v {
                Value::Bool(x) => ("BOOL", o(vec![("boolValue", b(*x))]), bool_data(*x)),
                Value::String(t) => ("TEXT", o(vec![("textValue", text_value(t))]), text_data(t)),
                Value::Number(c) => {
                    let Some(g) = c.as_u64().and_then(|c| self.guid_of(NodeId(c))) else { continue };
                    ("INSTANCE_SWAP", o(vec![("guidValue", g.clone())]), symbol_data(g))
                }
                _ => continue,
            };
            // Variant values have no definition of their own: the main they show says them. A
            // variant's instance names the variant's own copy of the definition.
            if let Some(id) = self.def_for(main, name, ty) {
                assignments.push(o(vec![("defID", guid(AUX, id)), ("value", value), ("varValue", var)]));
            }
        }
        if !assignments.is_empty() {
            f.push(("componentPropAssignments", arr(assignments)));
        }
        if n.children.iter().filter_map(|c| doc.node(*c)).any(|c| c.source.is_none()) {
            self.warn("instance layers that don't come from their main component were left out");
        }
    }

    /// The main component layer an instance or instance layer mirrors (through nested instances),
    /// or `id` itself.
    pub(super) fn main_layer(&self, id: NodeId) -> NodeId {
        let mut cur = id;
        for _ in 0..32 {
            let next = self.doc.node(cur).and_then(|n| match &n.kind {
                _ if n.source.is_some() => n.source,
                NodeKind::Instance { component, .. } => Some(*component),
                _ => None,
            });
            match next.filter(|s| self.doc.contains(*s)) {
                Some(s) => cur = s,
                None => break,
            }
        }
        cur
    }

    /// The id path of layer `d` of instance `inst` (of `main`): that of the main's layer it mirrors,
    /// or, inside a nested instance swapped for another component, the path of that instance and
    /// then the one inside its new main.
    fn layer_path(&self, d: &Node, inst: NodeId, main: NodeId, depth: usize) -> Option<Vec<NodeId>> {
        let src = d.source?;
        if let Some(p) = self.path_in(src, main, 0) {
            return Some(p);
        }
        if depth > 32 {
            return None;
        }
        let doc = self.doc;
        let nested = doc.ancestors(d.id).into_iter().take_while(|a| *a != inst).filter_map(|a| doc.node(a)).find_map(|a| match &a.kind {
            NodeKind::Instance { component, .. } => Some((a, *component)),
            _ => None,
        });
        let (outer, component) = nested?;
        let mut p = self.layer_path(outer, inst, main, depth + 1)?;
        p.extend(self.path_in(src, component, 0)?);
        Some(p)
    }

    /// The id path of layer `s` inside component `comp`: `s` itself, or through the nested
    /// instance it is expanded in, then on inside that instance's main.
    fn path_in(&self, s: NodeId, comp: NodeId, depth: usize) -> Option<Vec<NodeId>> {
        if depth > 32 {
            return None;
        }
        let doc = self.doc;
        let up = doc.ancestors(s);
        let below: Vec<NodeId> = up.iter().take_while(|a| **a != comp).copied().collect();
        if below.len() == up.len() {
            return None;
        }
        // The outermost nested instance between the component and the layer.
        match below.iter().rev().find_map(|a| match doc.node(*a).map(|n| &n.kind) {
            Some(NodeKind::Instance { component, .. }) => Some((*a, *component)),
            _ => None,
        }) {
            None => Some(vec![s]),
            Some((inst, main)) => {
                let mut p = vec![inst];
                p.extend(self.path_in(doc.node(s)?.source?, main, depth + 1)?);
                Some(p)
            }
        }
    }
}
