//! Variables: the local variables response (`GET /v1/files/:key/variables/local`, whose `meta`
//! holds `variableCollections` and `variables`), embedded next to the file JSON or passed on its
//! own, and the layers' `boundVariables` and `explicitVariableModes`. [`export`] writes
//! collections back in the same shape.

use std::collections::{BTreeMap, BTreeSet};

use figcraft_doc::color::PaintKind;
use figcraft_doc::{
    ALL_SCOPES, CODE_PLATFORMS, Collection, Mode, Node, NodeKind, Variable, VariableType, VariableValue, Variables, normalize_scopes,
};
use serde_json::{Map, Value, json};

use super::Ctx;
use super::props::{clip_str, color, list, text};

/// Most collections, variables and modes per collection read.
const MAX_COLLECTIONS: usize = 10_000;
const MAX_VARIABLES: usize = 100_000;
const MAX_MODES: usize = 1_000;
/// Longest string variable value kept (characters).
const MAX_STRING: usize = 10_000;

/// The object holding `variableCollections` and `variables`: a local variables response's `meta`,
/// or `v` itself.
pub fn source(v: &Value) -> Option<&Value> {
    [v.get("meta"), Some(v)].into_iter().flatten().find(|m| m.get("variableCollections").is_some_and(Value::is_object))
}

/// The variable ids the collections of a local variables response list in `variableIds`.
pub(super) fn listed_ids(v: &Value) -> BTreeSet<&str> {
    source(v)
        .into_iter()
        .flat_map(|src| objects(src, "variableCollections", MAX_COLLECTIONS))
        .flat_map(|(_, c)| list(c, "variableIds").iter().filter_map(Value::as_str).map(clip_str))
        .take(MAX_VARIABLES)
        .collect()
}

pub(super) fn kind_of(t: &str) -> Option<VariableType> {
    Some(match t {
        "COLOR" => VariableType::Color,
        "FLOAT" => VariableType::Number,
        "STRING" => VariableType::String,
        "BOOLEAN" => VariableType::Boolean,
        _ => return None,
    })
}

/// The id a `VariableAlias` (`{type: "VARIABLE_ALIAS", id}`) points at.
pub(super) fn alias(v: &Value) -> Option<&str> {
    (text(v, "type") == Some("VARIABLE_ALIAS")).then(|| text(v, "id")).flatten().filter(|s| !s.is_empty())
}

/// A raw variable value of type `kind` (or an alias).
pub(super) fn value(kind: VariableType, v: &Value) -> Option<VariableValue> {
    if let Some(a) = alias(v) {
        return Some(VariableValue::Alias(clip_str(a).to_string()));
    }
    Some(match kind {
        VariableType::Color => VariableValue::Color(color(v)?),
        VariableType::Number => VariableValue::Number(v.as_f64().filter(|x| x.is_finite())?.clamp(-1e9, 1e9)),
        VariableType::String => VariableValue::String(v.as_str()?.chars().take(MAX_STRING).collect()),
        VariableType::Boolean => VariableValue::Boolean(v.as_bool()?),
    })
}

/// `collections` of `vars` (with their variables) as a local variables response, which
/// [`Ctx::variables`] reads back. Aliases to variables outside them are written as the value they
/// resolve to in the default modes.
pub fn export(vars: &Variables, collections: &[&Collection]) -> Value {
    let ids: BTreeSet<&str> = collections.iter().map(|c| c.id.as_str()).collect();
    let members: Vec<&Variable> = vars.variables.iter().filter(|v| ids.contains(v.collection.as_str())).collect();
    let exported: BTreeSet<&str> = members.iter().map(|v| v.id.as_str()).collect();
    let mut colls = Map::new();
    for c in collections {
        let modes: Vec<Value> = c.modes.iter().map(|m| json!({"modeId": m.id, "name": m.name})).collect();
        let var_ids: Vec<&str> = members.iter().filter(|v| v.collection == c.id).map(|v| v.id.as_str()).collect();
        colls.insert(
            c.id.clone(),
            json!({"id": c.id, "name": c.name, "key": c.id, "modes": modes, "defaultModeId": c.default_mode(), "remote": false,
                   "hiddenFromPublishing": false, "variableIds": var_ids}),
        );
    }
    let mut out = Map::new();
    for v in members {
        let mut values = Map::new();
        for (mode, val) in &v.values {
            let val = match val {
                VariableValue::Alias(a) if !exported.contains(a.as_str()) => match vars.resolve(a, &BTreeMap::new()) {
                    Some(r) => r,
                    None => continue,
                },
                other => other.clone(),
            };
            values.insert(mode.clone(), raw(&val));
        }
        let kind = match v.kind {
            VariableType::Color => "COLOR",
            VariableType::Number => "FLOAT",
            VariableType::String => "STRING",
            VariableType::Boolean => "BOOLEAN",
        };
        let scopes = v.scopes.clone().unwrap_or_else(|| vec![ALL_SCOPES.to_string()]);
        out.insert(
            v.id.clone(),
            json!({"id": v.id, "name": v.name, "key": v.id, "variableCollectionId": v.collection, "resolvedType": kind, "valuesByMode": values,
                   "remote": false, "description": v.description, "hiddenFromPublishing": v.hidden_from_publishing, "scopes": scopes,
                   "codeSyntax": v.code_syntax}),
        );
    }
    json!({"status": 200, "error": false, "meta": {"variableCollections": colls, "variables": out}})
}

/// A value as the variables JSON writes it (the inverse of [`value`]).
fn raw(v: &VariableValue) -> Value {
    match v {
        VariableValue::Color(c) => json!({"r": c.r, "g": c.g, "b": c.b, "a": c.a}),
        VariableValue::Number(x) => json!(x),
        VariableValue::String(s) => json!(s),
        VariableValue::Boolean(b) => json!(b),
        VariableValue::Alias(a) => json!({"type": "VARIABLE_ALIAS", "id": a}),
    }
}

fn objects<'v>(v: &'v Value, key: &str, max: usize) -> impl Iterator<Item = (&'v String, &'v Value)> {
    v.get(key).and_then(Value::as_object).into_iter().flat_map(Map::iter).take(max)
}

impl Ctx<'_> {
    /// Add the collections and variables of a local variables response (or its `meta`); nothing
    /// when `v` isn't one. The `variables` map is read next to `variableCollections`, or from
    /// `meta` or the top level when it sits on the other side.
    pub(super) fn variables(&mut self, v: &Value) {
        let Some(src) = source(v) else { return };
        for (key, c) in objects(src, "variableCollections", MAX_COLLECTIONS) {
            self.collection(key, c);
        }
        let holder = [Some(src), v.get("meta"), Some(v)].into_iter().flatten().find(|m| m.get("variables").is_some_and(Value::is_object));
        let mut found: Vec<Variable> = Vec::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for (key, v) in holder.into_iter().flat_map(|h| objects(h, "variables", MAX_VARIABLES)) {
            if let Some(var) = self.variable(key, v)
                && seen.insert(var.id.clone())
            {
                found.push(var);
            }
        }
        self.check_aliases(&mut found);
        for v in &found {
            self.var_kinds.insert(v.id.clone(), v.kind);
        }
        // Objects have no order: `collectionOrder` and `variableOrder` (written by the `.fig`
        // import) list the ids in theirs. Ids they leave out keep their place after the listed.
        let rank = |key: &str| -> BTreeMap<&str, usize> { list(src, key).iter().filter_map(Value::as_str).zip(0..).collect() };
        let (collections, variables) = (rank("collectionOrder"), rank("variableOrder"));
        if !variables.is_empty() {
            found.sort_by_key(|v| variables.get(v.id.as_str()).copied().unwrap_or(usize::MAX));
        }
        if !collections.is_empty() {
            self.doc.variables.collections.sort_by_key(|c| collections.get(c.id.as_str()).copied().unwrap_or(usize::MAX));
        }
        self.doc.variables.variables.extend(found);
    }

    /// The modes a layer or page sets explicitly (`explicitVariableModes`): collection → mode.
    pub(super) fn modes(&mut self, j: &Value) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for (coll, mode) in j.get("explicitVariableModes").and_then(Value::as_object).into_iter().flatten() {
            match mode.as_str() {
                Some(m) if self.var_modes.get(coll.as_str()).is_some_and(|ms| ms.iter().any(|x| x == m)) => {
                    out.insert(coll.clone(), m.to_string());
                }
                _ => self.warn("variable modes of collections that aren't in the variables JSON were skipped"),
            }
        }
        out
    }

    fn collection(&mut self, key: &str, c: &Value) {
        let id = clip_str(text(c, "id").unwrap_or(key)).to_string();
        if id.is_empty() || self.var_modes.contains_key(&id) {
            return;
        }
        let mut modes: Vec<Mode> = Vec::new();
        for m in list(c, "modes").iter().take(MAX_MODES) {
            let Some(mid) = text(m, "modeId").filter(|s| !s.is_empty()).map(clip_str) else { continue };
            if !modes.iter().any(|x| x.id == mid) {
                let name =
                    text(m, "name").filter(|s| !s.trim().is_empty()).map_or_else(|| format!("Mode {}", modes.len() + 1), |s| clip_str(s).to_string());
                modes.push(Mode { id: mid.to_string(), name });
            }
        }
        // Our default mode is the first one.
        if let Some(i) = text(c, "defaultModeId").and_then(|d| modes.iter().position(|m| m.id == d)) {
            let m = modes.remove(i);
            modes.insert(0, m);
        }
        if modes.is_empty() {
            self.warn("variable collections without modes were skipped");
            return;
        }
        let name = text(c, "name").filter(|s| !s.trim().is_empty()).map_or("Collection", clip_str).to_string();
        self.var_modes.insert(id.clone(), modes.iter().map(|m| m.id.clone()).collect());
        self.doc.variables.collections.push(Collection { id, name, modes, extends: None });
    }

    fn variable(&mut self, key: &str, v: &Value) -> Option<Variable> {
        let id = clip_str(text(v, "id").unwrap_or(key)).to_string();
        if id.is_empty() || self.var_kinds.contains_key(&id) {
            return None;
        }
        let Some(kind) = text(v, "resolvedType").and_then(kind_of) else {
            self.warn("variables of unknown types were skipped");
            return None;
        };
        let Some((collection, mode_ids)) = text(v, "variableCollectionId").and_then(|c| self.var_modes.get_key_value(c)) else {
            self.warn("variables whose collection isn't in the variables JSON were skipped");
            return None;
        };
        let collection = collection.clone();
        let mut values = BTreeMap::new();
        let mut unreadable = false;
        for (mode, raw) in objects(v, "valuesByMode", MAX_MODES).filter(|(m, _)| mode_ids.contains(m)) {
            match value(kind, raw) {
                Some(val) => {
                    values.insert(mode.clone(), val);
                }
                None => unreadable = true,
            }
        }
        if unreadable {
            self.warn("variable values that don't match their type (or composed colours) were skipped");
        }
        if values.is_empty() {
            self.warn("variables without readable values were skipped");
            return None;
        }
        let scopes = v.get("scopes").and_then(Value::as_array).map(|a| normalize_scopes(kind, a.iter().filter_map(Value::as_str)));
        Some(Variable {
            id,
            name: text(v, "name").filter(|s| !s.trim().is_empty()).map_or("Variable", clip_str).to_string(),
            collection,
            kind,
            values,
            description: text(v, "description").map(|s| s.chars().take(MAX_STRING).collect()).unwrap_or_default(),
            scopes: scopes.flatten(),
            hidden_from_publishing: v.get("hiddenFromPublishing").and_then(Value::as_bool).unwrap_or(false),
            extends: None,
            overrides: Default::default(),
            code_syntax: objects(v, "codeSyntax", 16)
                .filter(|(k, _)| CODE_PLATFORMS.contains(&k.as_str()))
                .filter_map(|(k, x)| Some((k.clone(), clip_str(x.as_str().filter(|x| !x.trim().is_empty())?).to_string())))
                .collect(),
        })
    }

    /// Drop aliases to variables that aren't there (library variables) or have another type, fill
    /// modes without a value from the default mode, and drop variables left without values.
    fn check_aliases(&mut self, found: &mut Vec<Variable>) {
        let mut dropped = false;
        // Removing a variable can break aliases to it: repeat a few rounds (a dangling alias left
        // after that only resolves to nothing).
        for _ in 0..8 {
            let kinds: BTreeMap<String, VariableType> =
                found.iter().map(|v| (v.id.clone(), v.kind)).chain(self.var_kinds.iter().map(|(k, t)| (k.clone(), *t))).collect();
            let bad: BTreeSet<String> = found
                .iter()
                .filter(|v| v.values.values().any(|x| matches!(x, VariableValue::Alias(t) if kinds.get(t) != Some(&v.kind))))
                .map(|v| v.id.clone())
                .collect();
            if bad.is_empty() {
                break;
            }
            dropped = true;
            for v in found.iter_mut().filter(|v| bad.contains(&v.id)) {
                v.values.retain(|_, x| !matches!(x, VariableValue::Alias(t) if kinds.get(t) != Some(&v.kind)));
            }
            found.retain(|v| !v.values.is_empty());
        }
        if dropped {
            self.warn("variable aliases to variables that aren't in the variables JSON (or of another type) were skipped");
        }
        for v in found.iter_mut() {
            let Some(modes) = self.var_modes.get(&v.collection) else { continue };
            let fallback = modes.first().and_then(|m| v.values.get(m)).or_else(|| v.values.values().next()).cloned();
            if let Some(f) = fallback {
                for m in modes {
                    v.values.entry(m.clone()).or_insert_with(|| f.clone());
                }
            }
        }
    }

    /// The layer's variable bindings (`boundVariables`, paint and text style bindings) and
    /// explicitly set modes, in the field names `variable.bind` uses.
    pub(super) fn bindings<'j>(&mut self, n: &mut Node, j: &'j Value) {
        let mut found: Vec<(String, &'j str)> = Vec::new();
        let mut skipped: BTreeSet<&str> = BTreeSet::new();
        if let Some(b) = j.get("boundVariables").and_then(Value::as_object) {
            for (k, v) in b {
                if !node_binding(k, v, n, &mut found) {
                    skipped.insert(k.as_str());
                }
            }
        }
        // Paints carry their colour binding themselves: the first paint's is the layer's `fill` (or
        // `stroke`) when it is its only paint, else `fills/<i>/color`; gradient stops carry theirs.
        for (key, field) in [("fills", "fill"), ("strokes", "stroke")] {
            let paints = if key == "fills" { &n.fills } else { &n.strokes };
            // Paints that didn't import shift the ones after them: indices only mean something when
            // every paint came across.
            let aligned = paints.len() == list(j, key).len();
            for (i, p) in list(j, key).iter().enumerate() {
                let ours = paints.get(i).filter(|_| aligned);
                let stops = match ours.map(|o| &o.kind) {
                    Some(PaintKind::Gradient(g)) => g.stops.len(),
                    _ => 0,
                };
                for (s, stop) in list(p, "gradientStops").iter().enumerate().take(stops) {
                    if let Some(var) = stop.get("boundVariables").and_then(|b| b.get("color")).and_then(alias) {
                        found.push((format!("{key}/{i}/stops/{s}/color"), var));
                    }
                }
                let Some(var) = p.get("boundVariables").and_then(|b| b.get("color")).and_then(alias) else { continue };
                if i == 0 && single_solid(n, field) {
                    found.retain(|(f, _)| f != field);
                    found.push((field.to_string(), var));
                } else if ours.is_some_and(|o| o.solid_color().is_some()) {
                    found.push((format!("{key}/{i}/color"), var));
                } else {
                    skipped.insert(key);
                }
            }
        }
        // Effects and layout grids carry theirs per field.
        const EFFECT: [(&str, &str); 6] = [
            ("color", "color"),
            ("radius", "radius"),
            ("spread", "spread"),
            ("offsetX", "offsetX"),
            ("offsetY", "offsetY"),
            ("startRadius", "startRadius"),
        ];
        const GRID: [(&str, &str); 4] = [("sectionSize", "size"), ("count", "count"), ("gutterSize", "gutter"), ("offset", "offset")];
        for (key, prefix, fields, len) in
            [("effects", "effects", &EFFECT[..], n.effects.len()), ("layoutGrids", "layoutGrids", &GRID[..], n.grids.len())]
        {
            if list(j, key).len() != len {
                continue;
            }
            for (i, item) in list(j, key).iter().enumerate() {
                for (k, v) in item.get("boundVariables").and_then(Value::as_object).into_iter().flatten() {
                    match (fields.iter().find(|(from, _)| from == k), alias(v)) {
                        (Some((_, to)), Some(var)) => found.push((format!("{prefix}/{i}/{to}"), var)),
                        (Some(_), None) => {}
                        (None, _) => {
                            skipped.insert(key);
                        }
                    }
                }
            }
        }
        if matches!(n.kind, NodeKind::Text(_)) {
            for (k, v) in j.get("style").and_then(|s| s.get("boundVariables")).and_then(Value::as_object).into_iter().flatten() {
                match (text_field(k), alias(v)) {
                    (Some(f), Some(var)) if !found.iter().any(|(x, _)| x == f) => found.push((f.to_string(), var)),
                    (Some(_), _) => {}
                    (None, _) => {
                        skipped.insert(k.as_str());
                    }
                }
            }
        }
        for k in skipped {
            self.warn(format!("variables bound to `{}` are not supported and were skipped", clip_str(k)));
        }
        for (field, var) in collapse(found) {
            if !applies(n, &field) {
                self.warn("variable bindings to properties the layer doesn't have were skipped");
                continue;
            }
            match self.var_kinds.get(var) {
                Some(kind) if *kind == field_type(&field) => {
                    n.bound.insert(field, var.to_string());
                }
                Some(_) => self.warn("variable bindings whose variable type doesn't fit the property were skipped"),
                None if self.var_kinds.is_empty() => self.warn(
                    "variable bindings were skipped: the file JSON has no variable definitions (pass the local variables JSON as `variables`, or embed its `meta` next to the file JSON)",
                ),
                None => self.warn("variable bindings to variables that aren't in the variables JSON (library variables) were skipped"),
            }
        }
        n.var_modes.extend(self.modes(j));
    }
}

/// Map one `boundVariables` entry onto our field names; false when the property isn't bindable.
fn node_binding<'v>(k: &str, v: &'v Value, n: &Node, out: &mut Vec<(String, &'v str)>) -> bool {
    let mut push = |f: &str, a: Option<&'v str>| {
        if let Some(a) = a {
            out.push((f.to_string(), a));
        }
    };
    match k {
        "size" => {
            push("width", v.get("x").and_then(alias));
            push("height", v.get("y").and_then(alias));
        }
        "itemSpacing" => push("gap", alias(v)),
        "counterAxisSpacing" => push("crossGap", alias(v)),
        "characters" | "visible" | "opacity" | "strokeWeight" | "strokeTopWeight" | "strokeRightWeight" | "strokeBottomWeight"
        | "strokeLeftWeight" | "paddingLeft" | "paddingRight" | "paddingTop" | "paddingBottom" | "topLeftRadius" | "topRightRadius"
        | "bottomLeftRadius" | "bottomRightRadius" | "minWidth" | "maxWidth" | "minHeight" | "maxHeight" => push(k, alias(v)),
        "rectangleCornerRadii" => {
            for (key, f) in [
                ("RECTANGLE_TOP_LEFT_CORNER_RADIUS", "topLeftRadius"),
                ("RECTANGLE_TOP_RIGHT_CORNER_RADIUS", "topRightRadius"),
                ("RECTANGLE_BOTTOM_LEFT_CORNER_RADIUS", "bottomLeftRadius"),
                ("RECTANGLE_BOTTOM_RIGHT_CORNER_RADIUS", "bottomRightRadius"),
            ] {
                push(f, v.get(key).and_then(alias));
            }
        }
        "fills" | "strokes" => {
            let field = if k == "fills" { "fill" } else { "stroke" };
            match v.as_array().and_then(|a| a.first()).and_then(alias) {
                Some(a) if single_solid(n, field) => push(field, Some(a)),
                Some(_) => return false,
                None => {}
            }
        }
        _ => match text_field(k) {
            // Text properties bind per character range: one variable for the whole text only.
            Some(f) => match same_alias(v) {
                Some(a) => push(f, Some(a)),
                None => return v.as_array().is_some_and(Vec::is_empty),
            },
            None => return false,
        },
    }
    true
}

/// The one variable every entry of a per-range list (or a single alias) names.
fn same_alias(v: &Value) -> Option<&str> {
    match v.as_array() {
        Some(a) => {
            let first = alias(a.first()?)?;
            a.iter().all(|x| alias(x) == Some(first)).then_some(first)
        }
        None => alias(v),
    }
}

/// Text properties that bind by name.
fn text_field(k: &str) -> Option<&'static str> {
    Some(match k {
        "fontFamily" => "fontFamily",
        "fontSize" => "fontSize",
        "fontStyle" => "fontStyle",
        "fontWeight" => "fontWeight",
        "letterSpacing" => "letterSpacing",
        "lineHeight" => "lineHeight",
        "paragraphSpacing" => "paragraphSpacing",
        "paragraphIndent" => "paragraphIndent",
        _ => return None,
    })
}

/// A colour binding replaces the paints with one solid colour: only for layers with one solid paint.
fn single_solid(n: &Node, field: &str) -> bool {
    let paints = if field == "fill" { &n.fills } else { &n.strokes };
    matches!(paints.as_slice(), [p] if p.solid_color().is_some())
}

/// Paired sides bound to one variable become the combined field the panels show.
fn collapse(mut found: Vec<(String, &str)>) -> Vec<(String, &str)> {
    for (parts, into) in [
        (&["topLeftRadius", "topRightRadius", "bottomLeftRadius", "bottomRightRadius"][..], "cornerRadius"),
        (&["paddingLeft", "paddingRight"][..], "paddingHorizontal"),
        (&["paddingTop", "paddingBottom"][..], "paddingVertical"),
    ] {
        let vars: Vec<Option<&str>> = parts.iter().map(|p| found.iter().find(|(f, _)| f == p).map(|(_, v)| *v)).collect();
        if let Some(Some(var)) = vars.first().copied()
            && vars.iter().all(|v| *v == Some(var))
        {
            found.retain(|(f, _)| !parts.contains(&f.as_str()));
            found.push((into.to_string(), var));
        }
    }
    found
}

/// Whether the layer has the bound property (binding a gap to a layer without auto layout would
/// give it one when the value is applied).
fn applies(n: &Node, field: &str) -> bool {
    match field {
        "gap" | "crossGap" | "paddingLeft" | "paddingRight" | "paddingTop" | "paddingBottom" | "paddingHorizontal" | "paddingVertical" => {
            n.layout.is_some()
        }
        "cornerRadius" | "topLeftRadius" | "topRightRadius" | "bottomLeftRadius" | "bottomRightRadius" => n.kind.has_corner_radius(),
        f if f == "characters" || text_field(f).is_some() => matches!(n.kind, NodeKind::Text(_)),
        _ => true,
    }
}

/// The variable type a bound field takes.
fn field_type(field: &str) -> VariableType {
    match field {
        "fill" | "stroke" => VariableType::Color,
        f if f.ends_with("/color") => VariableType::Color,
        "characters" | "fontFamily" | "fontStyle" => VariableType::String,
        "visible" => VariableType::Boolean,
        _ => VariableType::Number,
    }
}
