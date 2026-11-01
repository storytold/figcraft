use super::component::{assigned, prop_guid};
use super::paint::color;
use super::variables::variable_value;
use super::{Affine, BTreeMap, BTreeSet, Kv, MAX_DEPTH, Mapper, Obj, Value, View, copy_str, guid, json, rows};

impl<'a> Mapper<'a> {
    /// The DOCUMENT with its pages.
    pub(super) fn document(&mut self) -> Value {
        let doc_id = self.nodes.iter().find(|(_, n)| n.get("type").and_then(Kv::str) == Some("DOCUMENT")).map(|(id, _)| id.clone());
        let pages: Vec<String> = match &doc_id {
            Some(d) => self.children(d),
            None => self.nodes.iter().filter(|(_, n)| n.get("type").and_then(Kv::str) == Some("CANVAS")).map(|(id, _)| id.clone()).collect(),
        };
        let mut internal = Vec::new();
        let mut visible = Vec::new();
        for p in pages {
            let Some(&n) = self.nodes.get(&p) else { continue };
            if n.get("type").and_then(Kv::str) != Some("CANVAS") {
                self.warn("document children other than pages were skipped");
                continue;
            }
            // The hidden page that holds styles, variables and library copies.
            if n.get("internalOnly").and_then(Kv::bool) == Some(true) {
                internal.push((p, n));
            } else {
                visible.push((p, n));
            }
        }
        // The library copies the pages use come first: in a file past the layer cap, the pages'
        // last layers are skipped rather than the mains every instance of them needs (#981).
        let visible_pages: Vec<String> = visible.iter().map(|(p, _)| p.clone()).collect();
        let required_mains = self.referenced_component_mains(&visible_pages);
        // The pages and their top-level layers, up to half the cap, stay for the pages.
        let top: usize = visible_pages.iter().map(|p| 1 + self.children(p).len()).sum();
        self.reserved = top.min(self.max_nodes / 2);
        for (id, canvas) in internal {
            if let Some(context) = self.library_context(&id, canvas, &required_mains) {
                self.library_contexts.push(context);
            }
        }
        self.reserved = 0;
        let out: Vec<Value> = visible.into_iter().map(|(p, n)| self.page(&p, n)).collect();
        if out.is_empty() {
            self.warn("the file has no pages");
        }
        let mut doc = json!({"id": doc_id.unwrap_or_else(|| "0:0".into()), "type": "DOCUMENT"});
        doc["children"] = Value::Array(out);
        doc
    }

    /// The ids of the components the visible pages use, directly or through the components they
    /// use in turn (nested instances, swap targets, overrides naming another main).
    fn referenced_component_mains(&mut self, visible_pages: &[String]) -> BTreeSet<String> {
        let mut mains = BTreeSet::new();
        let mut visited = BTreeSet::new();
        let mut pending: Vec<(String, usize)> = visible_pages.iter().flat_map(|page| self.children(page)).map(|id| (id, 1)).collect();
        while let Some((id, depth)) = pending.pop() {
            if depth > MAX_DEPTH {
                self.warn(format!("component references below {MAX_DEPTH} layer levels were skipped"));
                continue;
            }
            if !visited.insert(id.clone()) {
                continue;
            }
            if visited.len() > self.max_nodes {
                let max = self.max_nodes;
                self.warn(format!("the component reference scan reached the {max}-layer limit"));
                break;
            }
            let Some(node) = self.nodes.get(&id) else { continue };
            for target in self.component_main_targets(node) {
                if mains.insert(target.clone()) {
                    pending.push((target, 1));
                }
            }
            pending.extend(self.children(&id).into_iter().map(|child| (child, depth + 1)));
        }
        mains
    }

    /// The mains a layer names: an instance's own, the ones its overrides swap in, the instance-swap
    /// defaults and values of component properties.
    fn component_main_targets(&self, node: &Kv<'a>) -> Vec<String> {
        let mut targets: Vec<String> = node
            .get("componentPropDefs")
            .map(Kv::list)
            .unwrap_or_default()
            .iter()
            .filter_map(|definition| {
                self.is_instance_swap(definition.get("id")?)?;
                definition.get("initialValue")?.get("guidValue").and_then(guid)
            })
            .collect();
        if node.get("type").and_then(Kv::str) != Some("INSTANCE") {
            return targets;
        }
        // The instance and the override rows below it (rows carry the same fields, nested).
        let mut pending: Vec<(&Kv<'a>, usize)> = vec![(node, 0)];
        let mut rows = 0usize;
        while let Some((row, depth)) = pending.pop() {
            rows += 1;
            if depth > MAX_DEPTH || rows > self.max_nodes {
                break;
            }
            let data = row.get("symbolData");
            targets.extend(row.get("overriddenSymbolID").and_then(guid));
            targets.extend(data.and_then(|d| d.get("symbolID")).and_then(guid));
            for assignment in row.get("componentPropAssignments").map(Kv::list).unwrap_or_default() {
                if assignment.get("defID").is_some_and(|def| self.is_instance_swap(def).is_some()) {
                    targets.extend(assigned(assignment).and_then(prop_guid));
                }
            }
            let overrides = data.and_then(|d| d.get("symbolOverrides")).map(Kv::list).unwrap_or_default();
            let derived = row.get("derivedSymbolData").map(Kv::list).unwrap_or_default();
            pending.extend(overrides.iter().chain(derived).map(|row| (row, depth + 1)));
        }
        targets
    }

    /// The branches of the internal canvas `id` that hold the `required_mains` (with the frames
    /// they sit in), as a frame of its own: the cached definitions of components that come from a
    /// library. `None` when it holds none.
    fn library_context(&mut self, id: &str, canvas: &'a Kv<'a>, required_mains: &BTreeSet<String>) -> Option<Value> {
        let mut included = BTreeSet::new();
        let mut parents: BTreeMap<String, String> = BTreeMap::new();
        // The canvas itself is never a layer of the walk, whatever the parent links say.
        let mut visited = BTreeSet::from([id.to_string()]);
        // Each entry: layer, depth, inside a required definition.
        let mut pending: Vec<(String, usize, bool)> = self.children(id).into_iter().map(|child| (child, 1, false)).collect();
        while let Some((node_id, depth, in_definition)) = pending.pop() {
            if depth > MAX_DEPTH {
                self.warn(format!("internal library branches deeper than {MAX_DEPTH} levels were skipped"));
                continue;
            }
            if !visited.insert(node_id.clone()) {
                continue;
            }
            self.library_scan_count += 1;
            if self.library_scan_count > self.max_nodes {
                let max = self.max_nodes;
                self.warn(format!("the internal library scan reached the {max}-layer limit; the remaining branches were skipped"));
                break;
            }
            let Some(node) = self.nodes.get(&node_id) else { continue };
            let kind = node.get("type").and_then(Kv::str);
            let definition = (kind == Some("SYMBOL") || (kind == Some("FRAME") && node.get("isStateGroup").and_then(Kv::bool) == Some(true)))
                && required_mains.contains(&node_id);
            let in_definition = in_definition || definition;
            if in_definition {
                included.insert(node_id.clone());
            }
            if definition {
                // The frames around it keep the path its layers are addressed by.
                let mut ancestor = parents.get(&node_id);
                for _ in 0..=MAX_DEPTH {
                    let Some(parent) = ancestor else { break };
                    if !included.insert(parent.clone()) {
                        break;
                    }
                    ancestor = parents.get(parent);
                }
            }
            for child in self.children(&node_id).into_iter().rev() {
                // A layer reached again keeps the parent it was first reached by, so the parents
                // lead back to the canvas.
                if visited.contains(&child) {
                    continue;
                }
                parents.entry(child.clone()).or_insert_with(|| node_id.clone());
                pending.push((child, depth + 1, in_definition));
            }
        }
        if included.is_empty() {
            return None;
        }
        let mut context = Obj::new();
        context.insert("id".into(), json!(id));
        context.insert("type".into(), json!("FRAME"));
        copy_str(&mut context, &View::of(canvas), "name", "name");
        context.insert("size".into(), json!({"x": 0.0, "y": 0.0}));
        context.insert("relativeTransform".into(), rows(Affine::IDENTITY));
        context.insert(crate::rest::FIG_SOURCE_FIELD.into(), json!({"origin": id, "expanded": false, "wireFrame": true, "wireInstance": false}));
        self.variable_bindings(&mut context, &View::of(canvas));
        let mut children = Vec::new();
        for child in self.children(id) {
            children.extend(self.layer_filtered(&child, None, Affine::IDENTITY, None, None, 0, Some(&included)));
        }
        if !children.is_empty() {
            context.insert("children".into(), Value::Array(children));
        }
        Some(Value::Object(context))
    }

    fn page(&mut self, id: &str, n: &'a Kv<'a>) -> Value {
        let v = View::of(n);
        let mut o = Obj::new();
        o.insert("id".into(), json!(id));
        o.insert("type".into(), json!("CANVAS"));
        copy_str(&mut o, &v, "name", "name");
        if let Some(c) = v.get("backgroundColor") {
            o.insert("backgroundColor".into(), color(c));
        }
        if let Some(c) = v.get("prototypeBackgroundColor") {
            o.insert("prototypeBackgrounds".into(), json!([color(c)]));
        }
        self.variable_bindings(&mut o, &v);
        let mut flows: Vec<(&str, Value)> = Vec::new();
        let mut kids = Vec::new();
        for c in self.children(id) {
            if let Some(sp) = self.nodes.get(&c).and_then(|k| k.get("prototypeStartingPoint")) {
                let name = sp.get("name").and_then(Kv::str).filter(|s| !s.is_empty()).unwrap_or("Flow");
                flows.push((sp.get("position").and_then(Kv::str).unwrap_or(""), json!({"nodeId": c, "name": name})));
            }
            kids.extend(self.layer(&c, None, Affine::IDENTITY, None, None, 0));
        }
        flows.sort_by(|a, b| a.0.cmp(b.0));
        if !flows.is_empty() {
            o.insert("flowStartingPoints".into(), Value::Array(flows.into_iter().map(|f| f.1).collect()));
        }
        o.insert("children".into(), Value::Array(kids));
        Value::Object(o)
    }

    /// The shared styles the file defines, in their order: each with the sample layer that holds
    /// its values (`styleDefinitions`, our extension of the file JSON; the REST importer reads it).
    pub(super) fn style_definitions(&mut self) -> Vec<Value> {
        let mut found: Vec<(&'a str, String, &'a Kv<'a>)> = Vec::new();
        for (id, n) in &self.nodes {
            let live = n.get("isSoftDeleted").and_then(Kv::bool) != Some(true) && n.get("isSoftDeletedStyle").and_then(Kv::bool) != Some(true);
            if live && n.get("styleType").and_then(Kv::str).is_some_and(|t| matches!(t, "FILL" | "TEXT" | "EFFECT" | "GRID")) {
                found.push((n.get("sortPosition").and_then(Kv::str).unwrap_or(""), id.clone(), n));
            }
        }
        found.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
        let mut out = Vec::new();
        for (_, id, n) in found {
            let Some(node) = self.layer(&id, None, Affine::IDENTITY, None, None, 0) else { break };
            let name = n.get("name").and_then(Kv::str).unwrap_or("Style");
            let desc = n.get("styleDescription").or_else(|| n.get("description")).and_then(Kv::str).unwrap_or("");
            let ty = n.get("styleType").and_then(Kv::str).unwrap_or("");
            out.push(json!({"id": id, "name": name, "description": desc, "styleType": ty, "node": node}));
        }
        out
    }

    /// Collections and variables, as the local variables response's `meta`, with their ids in the
    /// file's order (`collectionOrder` and `variableOrder`, our extensions).
    pub(super) fn variables(&self) -> Option<Value> {
        let of_type = |t: &'static str, key: &'static str| {
            let live = move |n: &Kv| n.get("type").and_then(Kv::str) == Some(t) && n.get("isSoftDeleted").and_then(Kv::bool) != Some(true);
            let mut list: Vec<(&str, &String, &'a Kv<'a>)> = self
                .nodes
                .iter()
                .filter(|(_, n)| live(n))
                .map(|(id, n)| (n.get(key).and_then(|p| p.get("position").or(Some(p))).and_then(Kv::str).unwrap_or(""), id, *n))
                .collect();
            list.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
            list.into_iter().map(|(_, id, n)| (id, n))
        };
        let mut order = Vec::new();
        let mut collections = Obj::new();
        for (id, n) in of_type("VARIABLE_SET", "parentIndex") {
            order.push(json!(id));
            let mut modes: Vec<(&str, Value)> = n
                .get("variableSetModes")
                .map(Kv::list)
                .unwrap_or_default()
                .iter()
                .filter_map(|m| {
                    let mode = json!({"modeId": m.get("id").and_then(guid)?, "name": m.get("name").and_then(Kv::str).unwrap_or("Mode")});
                    Some((m.get("sortPosition").and_then(Kv::str).unwrap_or(""), mode))
                })
                .collect();
            modes.sort_by(|a, b| a.0.cmp(b.0));
            let default = modes.first().map_or(Value::Null, |m| m.1["modeId"].clone());
            let name = n.get("name").and_then(Kv::str).unwrap_or("Collection");
            let modes: Vec<Value> = modes.into_iter().map(|m| m.1).collect();
            collections.insert(id.clone(), json!({"id": id, "name": name, "modes": modes, "defaultModeId": default}));
        }
        let mut variable_order = Vec::new();
        let mut variables = Obj::new();
        for (id, n) in of_type("VARIABLE", "sortPosition") {
            let Some(set) = n.get("variableSetID").and_then(|s| self.resource_id(s)).filter(|s| collections.contains_key(s)) else { continue };
            let ty = n.get("variableResolvedType").and_then(Kv::str).unwrap_or("");
            if !matches!(ty, "BOOLEAN" | "FLOAT" | "STRING" | "COLOR") {
                continue;
            }
            let mut values = Obj::new();
            for e in n.get("variableDataValues").and_then(|m| m.get("entries")).map(Kv::list).unwrap_or_default() {
                if let (Some(mode), Some(val)) = (e.get("modeID").and_then(guid), e.get("variableData").and_then(variable_value)) {
                    values.insert(mode, val);
                }
            }
            let name = n.get("name").and_then(Kv::str).unwrap_or("Variable");
            let desc = n.get("description").and_then(Kv::str).unwrap_or("");
            let mut var =
                json!({"id": id, "name": name, "variableCollectionId": set, "resolvedType": ty, "valuesByMode": values, "description": desc});
            // The properties pickers offer the variable for, under the REST names.
            if let Some(scopes) = n.get("variableScopes") {
                var["scopes"] = scopes.list().iter().filter_map(Kv::str).map(|s| if s == "STROKE" { "STROKE_COLOR" } else { s }).collect();
            }
            variable_order.push(json!(id));
            variables.insert(id.clone(), var);
        }
        (!collections.is_empty())
            .then(|| json!({"variableCollections": collections, "variables": variables, "collectionOrder": order, "variableOrder": variable_order}))
    }
}
