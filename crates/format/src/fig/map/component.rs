use super::layer::own_text;
use super::text::char_styles;
use super::{BTreeMap, BTreeSet, Kv, MAX_DEPTH, Mapper, Obj, Rc, Value, View, guid, json};

/// An override row names a layer of the main by its current GUID or by the `overrideKey` the
/// layer had when the row was written: the key becomes the current GUID, unless the layer is
/// current already or the key is ambiguous.
fn normalize_override_segment(path: &mut [String], index: usize, scope: &OverrideScope) {
    let Some(segment) = path.get(index) else { return };
    if scope.current.contains(segment) {
        return;
    }
    if let Some(Some(current)) = scope.aliases.get(segment) {
        path[index] = current.clone();
    }
}

/// Overrides and derived layout by id path.
type Overrides<'a> = BTreeMap<Vec<String>, Vec<OverrideRow<'a>>>;

/// Where an override entry comes from.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Entry {
    Override,
    Derived,
    /// Derived layout of a nested instance for a size it no longer has: its size and place no
    /// longer hold, the rest (resolved colours, text) still does.
    Stale,
}

/// One override or derived-layout row addressed to a layer.
#[derive(Clone)]
pub(super) struct OverrideRow<'a> {
    pub(super) row: &'a Kv<'a>,
    pub(super) entry: Entry,
    /// How many mains are being expanded around the instance that wrote the row: the rows of
    /// the enclosing (outer) instances win over those of the inner ones.
    source_depth: usize,
    /// How many levels below the writing instance the row restates (`overrideLevel`; 0 for the
    /// instance's own rows).
    level: usize,
    /// Position among the rows of the instance that wrote it; later rows win.
    ordinal: usize,
}

/// The nested level an override row restates: its `overrideLevel`, capped; 0 when it has none.
fn override_level(row: &Kv) -> usize {
    row.get("overrideLevel").and_then(Kv::f64).filter(|l| l.is_finite() && *l > 0.0).map_or(0, |l| l.min(MAX_DEPTH as f64) as usize)
}

/// The layers of a main by current GUID, and the old `overrideKey` of each that has one
/// (`None` when two layers claim the same key).
pub(super) struct OverrideScope {
    current: BTreeSet<String>,
    aliases: BTreeMap<String, Option<String>>,
}

/// Values by key that nested instances add to their outer instances' (property values, slot
/// contents): an outer instance's value wins. Each instance keeps only what it adds and shares the
/// rest, instead of copying every outer value (#1077); lookups walk at most the nesting depth.
pub(super) struct Chain<V> {
    own: BTreeMap<String, V>,
    outer: Option<Rc<Chain<V>>>,
}

impl<V> Chain<V> {
    /// `outer`'s values, then `own`'s for keys `outer` lacks (shared as it is when `own` adds nothing).
    fn over(outer: Option<&Rc<Chain<V>>>, own: BTreeMap<String, V>) -> Rc<Self> {
        match outer {
            Some(o) if own.is_empty() => Rc::clone(o),
            _ => Rc::new(Self { own, outer: outer.cloned() }),
        }
    }

    pub(super) fn get(&self, key: &str) -> Option<&V> {
        self.outer.as_ref().and_then(|o| o.get(key)).or_else(|| self.own.get(key))
    }
}

/// Where an instance's expanded layers are: the overrides and derived layout of the outermost
/// instance (by id path), the path of the instance being expanded, and the property values.
#[derive(Clone)]
pub(super) struct Inst<'a> {
    pub(super) root: String,
    pub(super) path: Vec<String>,
    pub(super) overrides: Rc<Overrides<'a>>,
    props: Rc<Chain<&'a Kv<'a>>>,
    /// Slot property GUID → private slot-content node GUID.
    pub(super) slot_contents: Rc<Chain<String>>,
    /// Layers imported from a slot's independent content subtree.
    pub(super) slot_content: bool,
    /// Mains being expanded (an instance inside its own main stops there).
    symbols: Vec<String>,
}

impl<'a> Mapper<'a> {
    /// The layers of main `main` by current GUID and by `overrideKey`. An instance nested in the
    /// main is not entered: its layers belong to another main.
    fn override_scope(&mut self, main: &str) -> Rc<OverrideScope> {
        if let Some(scope) = self.override_scopes.get(main) {
            return Rc::clone(scope);
        }
        let mut current = BTreeSet::new();
        let mut aliases: BTreeMap<String, Option<String>> = BTreeMap::new();
        let mut visited = BTreeSet::new();
        let mut pending: Vec<(String, usize)> = self.children(main).into_iter().map(|id| (id, 1)).collect();
        let mut complete = true;
        while let Some((id, depth)) = pending.pop() {
            if depth > MAX_DEPTH || visited.len() >= self.max_nodes {
                complete = false;
                break;
            }
            if !visited.insert(id.clone()) {
                continue;
            }
            let Some(node) = self.nodes.get(&id) else { continue };
            current.insert(id.clone());
            if let Some(alias) = node.get("overrideKey").and_then(guid) {
                let target = aliases.entry(alias).or_insert_with(|| Some(id.clone()));
                if target.as_deref().is_some_and(|target| target != id) {
                    *target = None;
                }
            }
            if node.get("type").and_then(Kv::str) != Some("INSTANCE") {
                pending.extend(self.children(&id).into_iter().map(|child| (child, depth + 1)));
            }
        }
        // A scope cut short could name the wrong layer for a key: use none of the keys.
        if !complete {
            aliases.clear();
        }
        let scope = Rc::new(OverrideScope { current, aliases });
        let cost = scope.current.len() + scope.aliases.len();
        if cost <= self.max_nodes.saturating_sub(self.override_scope_entries) {
            self.override_scope_entries += cost;
            self.override_scopes.insert(main.to_string(), Rc::clone(&scope));
        }
        scope
    }

    pub(super) fn is_instance_swap(&self, def: &Kv) -> Option<()> {
        matches!(self.prop_def(&guid(def)?), Some((_, "INSTANCE_SWAP"))).then_some(())
    }

    /// The definition a copy of it was made from, followed to the original: the values an instance
    /// sets are kept under it, since an assignment and the layers that read it may name different
    /// copies of one definition.
    pub(super) fn prop_root(&self, def: &str) -> String {
        let mut def = def;
        for _ in 0..=MAX_DEPTH {
            let Some(parent) = self.prop_parents.get(def) else { break };
            def = parent;
        }
        def.to_string()
    }

    /// The definition `def` names: its own, or the set's for a variant's copy of it.
    fn prop_def(&self, def: &str) -> Option<&(String, &'a str)> {
        let mut def = def;
        for _ in 0..=MAX_DEPTH {
            if let Some(found) = self.prop_defs.get(def) {
                return Some(found);
            }
            def = self.prop_parents.get(def)?;
        }
        None
    }

    /// Component property definitions and description.
    pub(super) fn component(&mut self, o: &mut Obj, v: &View<'a>, id: &str) {
        let mut defs = Obj::new();
        for d in v.list("componentPropDefs") {
            if d.get("isDeleted").and_then(Kv::bool) == Some(true) {
                continue;
            }
            let Some((name, ty)) = d.get("id").and_then(guid).and_then(|i| self.prop_defs.get(&i).cloned()) else { continue };
            let init = d.get("initialValue");
            let def = match ty {
                "BOOL" => json!({"type": "BOOLEAN", "defaultValue": init.and_then(|i| i.get("boolValue")).and_then(Kv::bool).unwrap_or(true)}),
                "TEXT" => json!({"type": "TEXT", "defaultValue": init.and_then(prop_text).unwrap_or("")}),
                "INSTANCE_SWAP" => match init.and_then(|i| i.get("guidValue")).and_then(guid) {
                    Some(g) => {
                        if self.nodes.contains_key(&g) {
                            self.referenced_mains.insert(g.clone());
                        }
                        json!({"type": "INSTANCE_SWAP", "defaultValue": g})
                    }
                    None => continue,
                },
                "VARIANT" => json!({"type": "VARIANT"}),
                "SLOT" => {
                    self.warn_non_default_slot_config(d);
                    self.warn_dropped_slot_preferred_values(d);
                    json!({"type": "SLOT"})
                }
                _ => {
                    self.warn(format!("{} component properties are not supported and were skipped", ty.to_ascii_lowercase()));
                    continue;
                }
            };
            defs.insert(name, def);
        }
        if !defs.is_empty() {
            o.insert("componentPropertyDefinitions".into(), Value::Object(defs));
        }
        if let Some(d) = v.str("symbolDescription").or_else(|| v.str("description")).filter(|d| !d.is_empty()) {
            let into = if o.get("type").and_then(Value::as_str) == Some("COMPONENT_SET") { &mut self.sets } else { &mut self.components };
            into.insert(id.to_string(), json!({"description": d}));
        }
    }

    /// Preferred slot values are source component keys, but this mapper has no proven way to map
    /// those keys to file GUIDs. They are dropped instead of guessing native NodeIds.
    fn warn_dropped_slot_preferred_values(&mut self, def: &Kv<'a>) {
        let entries = def.get("preferredValues").and_then(|v| v.get("instanceSwapValues")).map(Kv::list).unwrap_or_default();
        if !entries.is_empty() {
            self.warn("preferred slot component keys cannot be resolved to file nodes and were skipped");
        }
    }

    fn warn_non_default_slot_config(&mut self, def: &Kv<'a>) {
        let Some(config) = def.get("slotPropConfig") else { return };
        let non_default = [
            ("stretchChildOnInsert", Some(false)),
            ("displayByDefault", Some(false)),
            ("minChildren", None),
            ("maxChildren", None),
            ("allowPreferredValuesOnly", Some(false)),
        ]
        .into_iter()
        .any(|(field, bool_default)| {
            let Some(value) = config.get(field) else { return false };
            if let Some(default) = bool_default { value.bool() != Some(default) } else { value.f64() != Some(0.0) }
        });
        if non_default {
            self.warn("non-default slot settings are not represented and were skipped");
        }
    }

    /// A layer's component property references, and inside an instance the values they take.
    pub(super) fn prop_refs(&mut self, o: &mut Obj, v: &View<'a>, inst: Option<&Inst<'a>>) {
        let mut refs = Obj::new();
        for e in v.get("parameterConsumptionMap").and_then(|m| m.get("entries")).map(Kv::list).unwrap_or_default() {
            let field = match e.get("variableField").and_then(Kv::str) {
                Some("VISIBLE") => "visible",
                Some("TEXT_DATA") => "characters",
                Some("OVERRIDDEN_SYMBOL_ID") => "mainComponent",
                Some("SLOT_CONTENT_ID") => "slot",
                _ => continue,
            };
            let Some(def) =
                e.get("variableData").and_then(|d| d.get("value")).and_then(|x| x.get("propRefValue")).and_then(|p| p.get("defId")).and_then(guid)
            else {
                continue;
            };
            // Expanded frames must follow their main frame's source link. A direct slot reference
            // makes the engine resolve ownership against the instance, where the property is not defined.
            if !(field == "slot" && inst.is_some())
                && let Some((name, _)) = self.prop_def(&def)
            {
                refs.insert(field.into(), json!(name));
            }
            let Some(val) = inst.and_then(|i| i.props.get(&self.prop_root(&def))) else { continue };
            match field {
                "visible" => {
                    if let Some(b) = val.get("boolValue").and_then(Kv::bool) {
                        o.insert("visible".into(), json!(b));
                    }
                }
                "characters" => {
                    if let Some(t) = prop_text(val) {
                        o.insert("characters".into(), json!(t));
                        // Run styles count the chars they were made for: for other chars only
                        // the value's own apply.
                        if own_text(v) != Some(t) {
                            o.remove("characterStyleOverrides");
                            o.remove("styleOverrideTable");
                            o.remove("_figLineStarts");
                            if let Some(tv) = prop_text_data(val) {
                                let size = o.get("style").and_then(|s| s.get("fontSize")).and_then(Value::as_f64);
                                char_styles(o, tv, size, &mut |w| self.warn(w));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if !refs.is_empty() {
            o.insert("componentPropertyReferences".into(), Value::Object(refs));
        }
    }

    /// An instance's main and properties; the context its expanded layers are built in.
    pub(super) fn instance(&mut self, o: &mut Obj, v: &View<'a>, id: &str, outer: Option<&Inst<'a>>, rest_id: &str) -> Option<(Inst<'a>, String)> {
        let sd = v.get("symbolData");
        let mut sym = v.get("overriddenSymbolID").and_then(guid).or_else(|| sd.and_then(|s| s.get("symbolID")).and_then(guid));
        // An instance swap property value picks the main.
        if let Some(i) = outer
            && let Some(def) = prop_ref(v, "OVERRIDDEN_SYMBOL_ID")
            && let Some(g) = i.props.get(&self.prop_root(&def)).and_then(|p| prop_guid(p))
        {
            sym = Some(g);
        }
        let Some(sym) = sym else {
            self.warn("instances without a main component were imported as empty frames");
            return None;
        };
        o.insert("componentId".into(), json!(sym));
        let assignments = v.list("componentPropAssignments");
        let mut props = Obj::new();
        for a in assignments {
            let Some((name, ty)) = a.get("defID").and_then(guid).and_then(|d| self.prop_def(&d).cloned()) else { continue };
            let val = assigned(a);
            let p = match ty {
                "BOOL" => val.and_then(|x| x.get("boolValue")).and_then(Kv::bool).map(|b| json!({"type": "BOOLEAN", "value": b})),
                "TEXT" => val.and_then(prop_text).map(|t| json!({"type": "TEXT", "value": t})),
                "INSTANCE_SWAP" => val.and_then(prop_guid).map(|g| json!({"type": "INSTANCE_SWAP", "value": g})),
                _ => None,
            };
            if let Some(p) = p {
                props.insert(name, p);
            }
        }
        if !props.is_empty() {
            o.insert("componentProperties".into(), Value::Object(props));
        }
        if !self.nodes.contains_key(&sym) {
            self.warn("instances of components that aren't in the file show no layers");
            return None;
        }
        if !self.referenced_mains.contains(&sym) {
            self.referenced_mains.insert(sym.clone());
        }
        if outer.is_some_and(|i| i.symbols.contains(&sym)) {
            return None;
        }
        // Overrides by id path below this instance (the instance itself is addressed by its main's id).
        let own_path: Vec<String> = outer.map(|i| [i.path.as_slice(), &[id.to_string()]].concat()).unwrap_or_default();
        let scope = self.override_scope(&sym);
        let mut overrides: Overrides<'a> = BTreeMap::new();
        if let Some(inherited) = outer {
            // Only rows addressed below this instance can apply inside it (every path looked up in
            // it starts with `own_path`); they sort together, so the others aren't even visited. A
            // nested instance used to copy all of its outer instance's rows (#1077).
            let below = inherited.overrides.range(own_path.clone()..).take_while(|(path, _)| path.starts_with(&own_path));
            for (path, rows) in below.filter(|(path, _)| path.len() > own_path.len()) {
                let mut path = path.clone();
                // Rows an outer instance wrote for this instance's layers may use their old keys.
                normalize_override_segment(&mut path, own_path.len(), &scope);
                overrides.entry(path).or_default().extend(rows.iter().cloned());
            }
        }
        // A nested instance's derived layout is for the size it has in its main: resized here, the
        // sizes and places in it no longer hold and its layers follow their constraints instead.
        let differs = |a: f64, b: f64| (a - b).abs() > 1e-6 * a.abs().max(b.abs()).max(1.0);
        let resized = outer.is_some()
            && match (self.nodes.get(id).and_then(|b| View::of(b).size()), v.size()) {
                (Some(a), Some(b)) => differs(a.0, b.0) || differs(a.1, b.1),
                (a, b) => a.is_some() != b.is_some(),
            };
        let mut local: Overrides<'a> = BTreeMap::new();
        let lists = [sd.and_then(|s| s.get("symbolOverrides")).map(Kv::list).unwrap_or_default(), v.list("derivedSymbolData")];
        let source_depth = outer.map_or(0, |i| i.symbols.len());
        let mut ordinal = 0;
        for (layer, list) in lists.into_iter().enumerate() {
            for ov in list {
                let row_ordinal = ordinal;
                ordinal += 1;
                let mut rel: Vec<String> =
                    ov.get("guidPath").and_then(|g| g.get("guids")).map(Kv::list).unwrap_or_default().iter().filter_map(guid).collect();
                if rel.is_empty() || rel == [sym.clone()] {
                    continue;
                }
                normalize_override_segment(&mut rel, 0, &scope);
                // Explicit overrides, then derived layout (stale once the instance is resized).
                let entry = match (layer, resized) {
                    (0, _) => Entry::Override,
                    (_, false) => Entry::Derived,
                    (_, true) => Entry::Stale,
                };
                local.entry([own_path.as_slice(), &rel].concat()).or_default().push(OverrideRow {
                    row: ov,
                    entry,
                    source_depth,
                    level: override_level(ov),
                    ordinal: row_ordinal,
                });
            }
        }
        for (path, rows) in local {
            overrides.entry(path).or_default().extend(rows);
        }
        for rows in overrides.values_mut() {
            // Deeper rows apply first and outer rows win: a row restating a nested level counts at
            // that level's depth, wherever it sits on the wire. Equal depths keep the outer source
            // last, then wire order.
            rows.sort_by_key(|row| (std::cmp::Reverse(row.source_depth + row.level), std::cmp::Reverse(row.source_depth), row.ordinal));
        }
        // This instance's own property values and slot contents; the outer instances' win.
        let mut all_props: BTreeMap<String, &'a Kv<'a>> = BTreeMap::new();
        let mut slot_contents: BTreeMap<String, String> = BTreeMap::new();
        for a in assignments {
            if let (Some(d), Some(val)) = (a.get("defID").and_then(guid), assigned(a)) {
                all_props.entry(self.prop_root(&d)).or_insert(val);
            }
            let Some(def) = a.get("defID").and_then(guid) else { continue };
            if self.prop_def(&def).is_none_or(|(_, ty)| *ty != "SLOT") {
                continue;
            }
            let var_value = a.get("varValue");
            if var_value.and_then(|v| v.get("dataType")).and_then(Kv::str) != Some("SLOT_CONTENT_ID") {
                self.warn("malformed slot content assignment was skipped");
                continue;
            }
            let target = var_value.and_then(|v| v.get("value")).and_then(|v| v.get("slotContentIdValue")).and_then(|v| v.get("guid")).and_then(guid);
            let Some(target) = target else {
                self.warn("malformed slot content assignment was skipped");
                continue;
            };
            if target == "4294967295:4294967295" {
                self.warn("slot content assignment uses the empty GUID sentinel and was skipped");
                continue;
            }
            if self.nodes.get(&target).is_none_or(|n| n.get("isSlotContent").and_then(Kv::bool) != Some(true)) {
                self.warn("slot content assignment target is missing or is not slot content");
                continue;
            }
            slot_contents.entry(self.prop_root(&def)).or_insert(target);
        }
        let mut symbols = outer.map(|i| i.symbols.clone()).unwrap_or_default();
        symbols.push(sym.clone());
        let root = outer.map_or_else(|| rest_id.to_string(), |i| i.root.clone());
        Some((
            Inst {
                root,
                path: own_path,
                overrides: Rc::new(overrides),
                props: Chain::over(outer.map(|i| &i.props), all_props),
                slot_contents: Chain::over(outer.map(|i| &i.slot_contents), slot_contents),
                slot_content: false,
                symbols,
            },
            sym,
        ))
    }
}

/// The text of a component property value.
fn prop_text<'k>(v: &'k Kv) -> Option<&'k str> {
    prop_text_data(v).and_then(|t| t.get("characters")).and_then(Kv::str)
}

/// The text data of a component property value: `textValue`, or `textDataValue` in the variable form.
fn prop_text_data<'k, 's>(v: &'k Kv<'s>) -> Option<&'k Kv<'s>> {
    v.get("textValue").or_else(|| v.get("textDataValue"))
}

/// The main an instance swap value names: `guidValue`, or `symbolIdValue` in the variable form.
pub(super) fn prop_guid(v: &Kv) -> Option<String> {
    v.get("guidValue").and_then(guid).or_else(|| v.get("symbolIdValue")?.get("guid").and_then(guid))
}

/// The value a component property assignment sets: `value`, or the variable form `varValue` holds
/// when `value` is empty, as current files write many booleans and instance swaps.
pub(super) fn assigned<'k, 's>(assignment: &'k Kv<'s>) -> Option<&'k Kv<'s>> {
    assignment.get("value").filter(|v| !matches!(v, Kv::Object(fields) if fields.is_empty())).or_else(|| assignment.get("varValue")?.get("value"))
}

/// The property a layer's `field` is bound to (`parameterConsumptionMap`).
pub(super) fn prop_ref(v: &View, field: &str) -> Option<String> {
    v.get("parameterConsumptionMap")?.get("entries")?.list().iter().find_map(|e| {
        if e.get("variableField").and_then(Kv::str) != Some(field) {
            return None;
        }
        e.get("variableData")?.get("value")?.get("propRefValue")?.get("defId").and_then(guid)
    })
}
