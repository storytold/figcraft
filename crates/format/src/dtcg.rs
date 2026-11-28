//! Design tokens: a file's variables as a Design Tokens Community Group file (the DTCG format,
//! 2025.10), and such a file read back as variables (`variable.export {format: "dtcg"}`,
//! `variable.import`).
//!
//! Each collection is a top-level group and each variable a token in it, the `/` parts of its name
//! nested groups. Colours, numbers (px dimensions, or plain numbers where layers use them unitless)
//! and aliases (`{group.token}`) are DTCG values; strings and booleans, which DTCG has no type for,
//! the collection's modes and the variable's scopes are kept under `$extensions` ([`EXTENSION`]),
//! so the file reads back whole. Reading takes the 2025.10 shapes and the older string ones (`#rrggbb`, `16px`); composite
//! tokens (typography, shadows and the like) have no variable type and are skipped with a warning.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use figcraft_doc::color::{Color, ColorProfile};
use figcraft_doc::{Collection, Mode, Variable, VariableType, VariableValue, Variables};
use serde_json::{Map, Value, json};

/// The `$extensions` key FigCraft keeps modes and the types DTCG lacks under.
pub const EXTENSION: &str = "ai.storyteller.figcraft";
/// How deep groups are read.
const MAX_DEPTH: usize = 32;
/// How many warnings a read lists one by one.
const MAX_WARNINGS: usize = 50;
/// Pixels to a `rem`, as browsers default.
const REM_PX: f64 = 16.0;

/// `colls` of `vars` as a DTCG file. Each token's `$value` is its value in the mode named `mode`
/// (a collection without it, and every collection when `None`, in its default mode) and every
/// mode's value is listed under `$extensions`. Colours are in `profile`, the file's colour profile.
/// `unitless`: a number variable written as a plain number rather than a px dimension (opacity,
/// font weight). Colour calculations are written as the colour they show; the warnings name what
/// was left out.
pub fn export(
    vars: &Variables,
    colls: &[&Collection],
    profile: ColorProfile,
    mode: Option<&str>,
    unitless: &dyn Fn(&Variable) -> bool,
) -> (Value, Vec<String>) {
    let flattener = vars.flattener();
    let exported: HashSet<&str> = colls.iter().map(|c| c.id.as_str()).collect();
    let mut root = Map::new();
    let mut warnings = Vec::new();
    for c in colls {
        let Some(shown) = mode.and_then(|m| c.modes.iter().find(|x| x.name == m)).or(c.modes.first()) else { continue };
        let key = segment(&c.name);
        if root.contains_key(&key) {
            warnings.push(format!("collection `{}` was left out: another collection writes the same group", c.name));
            continue;
        }
        let mut group = Map::new();
        group.insert("$extensions".into(), json!({ EXTENSION: { "modes": c.modes.iter().map(|m| m.name.as_str()).collect::<Vec<_>>() } }));
        for v in vars.variables.iter().filter(|v| v.collection == c.id) {
            let value_in = |m: &Mode| -> Option<Value> {
                let flat = flattener.value(v, &m.id, v.value_or_first(&m.id)?)?;
                // A reference out of the file's tokens would dangle: it is written as what it shows.
                let flat = match flat {
                    VariableValue::Alias(t) if vars.get_by_id(&t).is_none_or(|t| !exported.contains(t.collection.as_str())) => {
                        vars.resolve(&v.id, &BTreeMap::from([(c.id.clone(), m.id.clone())]))?
                    }
                    other => other,
                };
                token_value(vars, &flat, unitless(v), profile)
            };
            let Some(value) = value_in(shown) else {
                warnings.push(format!("`{}` was left out: it has no value that resolves", v.name));
                continue;
            };
            let mut token = Map::new();
            let ty = match v.kind {
                VariableType::Color => Some("color"),
                VariableType::Number if unitless(v) => Some("number"),
                VariableType::Number => Some("dimension"),
                VariableType::String | VariableType::Boolean => None,
            };
            if let Some(ty) = ty {
                token.insert("$type".into(), json!(ty));
            }
            token.insert("$value".into(), value);
            if !v.description.is_empty() {
                token.insert("$description".into(), json!(v.description));
            }
            let mut ext = Map::new();
            if c.modes.len() > 1 {
                let modes: Map<String, Value> = c.modes.iter().filter_map(|m| Some((m.name.clone(), value_in(m)?))).collect();
                ext.insert("modes".into(), Value::Object(modes));
            }
            if let Some(scopes) = &v.scopes {
                ext.insert("scopes".into(), json!(scopes));
            }
            match v.kind {
                VariableType::String => _ = ext.insert("type".into(), json!("string")),
                VariableType::Boolean => _ = ext.insert("type".into(), json!("boolean")),
                VariableType::Color | VariableType::Number => {}
            }
            if !ext.is_empty() {
                token.insert("$extensions".into(), json!({ EXTENSION: ext }));
            }
            if !insert(&mut group, &token_path(&v.name), Value::Object(token)) {
                warnings.push(format!("`{}` was left out: another variable's name writes the same token or group", v.name));
            }
        }
        root.insert(key, Value::Object(group));
    }
    (Value::Object(root), warnings)
}

/// A name part as a DTCG name, which can't hold `.`, `{` or `}` or start with `$`.
fn segment(name: &str) -> String {
    let s: String = name.trim().chars().map(|c| if matches!(c, '.' | '{' | '}') { '_' } else { c }).collect();
    match s.strip_prefix('$') {
        Some(rest) => format!("_{rest}"),
        None if s.is_empty() => "_".into(),
        None => s,
    }
}

/// A variable's token path in its collection's group: its name's `/` parts.
fn token_path(name: &str) -> Vec<String> {
    let parts: Vec<String> = name.split('/').map(str::trim).filter(|p| !p.is_empty()).map(segment).collect();
    if parts.is_empty() { vec!["_".into()] } else { parts }
}

/// Put `token` at `path` in `group`; false when a token or group is in the way.
fn insert(group: &mut Map<String, Value>, path: &[String], token: Value) -> bool {
    let Some((leaf, groups)) = path.split_last() else { return false };
    let mut at = group;
    for g in groups {
        match at.entry(g.clone()).or_insert_with(|| Value::Object(Map::new())) {
            Value::Object(o) if !o.contains_key("$value") => at = o,
            _ => return false,
        }
    }
    if at.contains_key(leaf) {
        return false;
    }
    at.insert(leaf.clone(), token);
    true
}

/// A variable value as a token's `$value` (`None`: a colour calculation, which is flattened first).
fn token_value(vars: &Variables, val: &VariableValue, unitless: bool, profile: ColorProfile) -> Option<Value> {
    Some(match val {
        VariableValue::Color(c) => color_value(*c, profile),
        VariableValue::Number(x) if !x.is_finite() => return None,
        VariableValue::Number(x) if unitless => json!(x),
        VariableValue::Number(x) => json!({"value": x, "unit": "px"}),
        VariableValue::String(s) => json!(s),
        VariableValue::Boolean(b) => json!(b),
        VariableValue::Alias(t) => {
            let t = vars.get_by_id(t)?;
            let c = vars.collection_by_id(&t.collection)?;
            let path: Vec<String> = std::iter::once(segment(&c.name)).chain(token_path(&t.name)).collect();
            json!(format!("{{{}}}", path.join(".")))
        }
        VariableValue::ColorAlpha { .. } | VariableValue::ColorMix { .. } => return None,
    })
}

/// A colour as DTCG writes one: its colour space and components, its alpha when not opaque, and
/// in sRGB its hex.
fn color_value(c: Color, profile: ColorProfile) -> Value {
    let c = c.sanitized();
    // Unrounded, so a colour reads back exactly.
    let round = f64::from;
    let space = if profile == ColorProfile::DisplayP3 { "display-p3" } else { "srgb" };
    let mut o = json!({"colorSpace": space, "components": [round(c.r), round(c.g), round(c.b)]});
    if c.a < 1.0 {
        o["alpha"] = json!(round(c.a));
    }
    if profile == ColorProfile::Srgb {
        o["hex"] = json!(format!("#{}", c.to_hex().to_ascii_lowercase()));
    }
    o
}

/// Whether `v` holds design tokens (an object with a `$value` somewhere) rather than the variables
/// JSON.
pub fn is_tokens(v: &Value) -> bool {
    fn has(v: &Value, depth: usize) -> bool {
        depth <= MAX_DEPTH && v.as_object().is_some_and(|o| o.contains_key("$value") || o.values().any(|c| has(c, depth + 1)))
    }
    has(v, 0)
}

/// One token read: where it is and what it holds.
struct Token<'a> {
    coll: usize,
    name: String,
    ty: Option<&'a str>,
    value: &'a Value,
    description: Option<&'a str>,
    ext: Option<&'a Map<String, Value>>,
}

/// Warnings, listed one by one up to [`MAX_WARNINGS`], then counted.
#[derive(Default)]
struct Warnings {
    list: Vec<String>,
    more: usize,
    /// Tokens skipped by their type, counted.
    types: BTreeMap<String, usize>,
}

impl Warnings {
    fn push(&mut self, w: String) {
        if self.list.len() < MAX_WARNINGS {
            self.list.push(w);
        } else {
            self.more += 1;
        }
    }

    fn finish(mut self) -> Vec<String> {
        for (ty, n) in std::mem::take(&mut self.types) {
            self.list.push(format!("{n} `{ty}` token(s) skipped: variables have no such type"));
        }
        if self.more > 0 {
            self.list.push(format!("and {} more", self.more));
        }
        self.list
    }
}

/// A DTCG file as variables (for `variable.import`'s merge, as the variables JSON reads): each
/// top-level group a collection (tokens at the top level go in one named "Tokens"), each token a
/// variable named by its path under that group. A collection takes the modes its `$extensions`
/// list, else one, named `default_mode(collection name)` (the mode an existing collection of that
/// name has first) or "Mode 1". `profile`: the file's colour profile, which colours are converted
/// into. `None` when `v` is not an object.
pub fn import(v: &Value, profile: ColorProfile, default_mode: &dyn Fn(&str) -> Option<String>) -> Option<(Variables, Vec<String>)> {
    let root = v.as_object()?;
    let mut vars = Variables::default();
    let mut read = Reader::default();
    let mut w = Warnings::default();
    let add_collection = |vars: &mut Variables, name: &str, group: Option<&Map<String, Value>>| {
        let listed: Vec<&str> = group
            .and_then(|g| extension(g)?.get("modes")?.as_array())
            .map(|m| m.iter().filter_map(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).collect())
            .unwrap_or_default();
        let mut seen = BTreeSet::new();
        let mut names: Vec<String> = listed.into_iter().filter(|n| seen.insert(*n)).map(str::to_string).collect();
        if names.is_empty() {
            names.push(default_mode(name).unwrap_or_else(|| "Mode 1".into()));
        }
        let id = format!("c{}", vars.collections.len());
        let modes = names.into_iter().enumerate().map(|(i, name)| Mode { id: format!("{id}:{i}"), name }).collect();
        vars.collections.push(Collection { id, name: name.to_string(), modes, extends: None });
        vars.collections.len() - 1
    };
    let mut loose = None;
    for (key, child) in root.iter().filter(|(k, _)| !k.starts_with('$')) {
        let Some(obj) = child.as_object() else { continue };
        if obj.contains_key("$value") {
            let coll = *loose.get_or_insert_with(|| add_collection(&mut vars, "Tokens", None));
            read.add(key.clone(), token(coll, key.clone(), obj, root.get("$type").and_then(Value::as_str)));
        } else {
            let coll = add_collection(&mut vars, key, Some(obj));
            read.walk(obj, coll, key, &[], obj.get("$type").and_then(Value::as_str), 1);
        }
    }
    let Reader { tokens, refs } = read;
    // Each token's variable type: from its own or its group's `$type`, else from its value, and an
    // untyped alias from what it names.
    let mut kinds: Vec<Option<VariableType>> = tokens.iter().map(|t| kind(t, &mut w)).collect();
    for i in 0..tokens.len() {
        // Along the chain of aliases to the first typed token (a loop or a long chain gives none).
        let mut at = i;
        for _ in 0..MAX_DEPTH {
            let Some(&next) = reference(tokens[at].value).and_then(|r| refs.get(r)) else { break };
            if kinds[i].is_some() || kinds[next].is_some() {
                kinds[i] = kinds[i].or(kinds[next]);
                break;
            }
            at = next;
        }
    }
    let ids: Vec<String> = (0..tokens.len()).map(|i| format!("v{i}")).collect();
    let conversion = ColorProfile::Srgb.conversion_to(profile);
    let p3 = ColorProfile::DisplayP3.conversion_to(profile);
    let mut rems = false;
    for (i, t) in tokens.iter().enumerate() {
        let Some(kind) = kinds[i] else {
            // A type variables lack is counted by its type already.
            if t.ty.is_none() {
                w.push(format!("`{}` was skipped: its type isn't known (an alias to no token, or a value of no type)", t.name));
            }
            continue;
        };
        let Some(coll) = vars.collections.get(t.coll) else { continue };
        let modes_ext = t.ext.and_then(|e| e.get("modes")).and_then(Value::as_object);
        let mut values = BTreeMap::new();
        for m in &coll.modes {
            let raw = modes_ext.and_then(|o| o.get(&m.name)).unwrap_or(t.value);
            let parsed = match reference(raw) {
                Some(r) => match refs.get(r) {
                    Some(target) if kinds[*target] == Some(kind) => Some(VariableValue::Alias(ids[*target].clone())),
                    Some(_) => {
                        w.push(format!("`{}`: its alias {{{r}}} names a token of another type", t.name));
                        continue;
                    }
                    None => None,
                },
                None => match kind {
                    VariableType::Color => color(raw, &conversion, &p3).map(VariableValue::Color),
                    VariableType::Number => number(raw, &mut rems).map(VariableValue::Number),
                    VariableType::String => text(raw).map(VariableValue::String),
                    VariableType::Boolean => raw.as_bool().map(VariableValue::Boolean),
                },
            };
            match parsed {
                Some(val) => _ = values.insert(m.id.clone(), val),
                None => w.push(format!("`{}`: its value in {} couldn't be read", t.name, m.name)),
            }
        }
        if values.is_empty() {
            continue;
        }
        vars.variables.push(Variable {
            id: ids[i].clone(),
            name: t.name.clone(),
            collection: coll.id.clone(),
            kind,
            values,
            description: t.description.unwrap_or_default().to_string(),
            scopes: t
                .ext
                .and_then(|e| e.get("scopes")?.as_array())
                .and_then(|a| figcraft_doc::normalize_scopes(kind, a.iter().filter_map(Value::as_str))),
            hidden_from_publishing: false,
            extends: None,
            overrides: BTreeSet::new(),
            code_syntax: BTreeMap::new(),
        });
    }
    if rems {
        w.push(format!("rem dimensions were read at {REM_PX} px to the rem"));
    }
    if vars.variables.is_empty() {
        w.push("no tokens of a variable type (color, dimension, number, font family or weight, duration, string, boolean) found".into());
    }
    Some((vars, w.finish()))
}

/// The tokens read so far, and the dotted path a reference names each by.
#[derive(Default)]
struct Reader<'a> {
    tokens: Vec<Token<'a>>,
    refs: HashMap<String, usize>,
}

impl<'a> Reader<'a> {
    /// Read the tokens of `group` (collection `coll`, at reference path `at` and name `name`,
    /// `ty` the type it hands down).
    fn walk(&mut self, group: &'a Map<String, Value>, coll: usize, at: &str, name: &[&'a str], ty: Option<&'a str>, depth: usize) {
        for (key, child) in group {
            let Some(obj) = child.as_object() else { continue };
            let path = format!("{at}.{key}");
            if obj.contains_key("$value") {
                // A group's own token (`$root`) is named by the group, and also reached by its path.
                let own = key == "$root";
                if (own && name.is_empty()) || (key.starts_with('$') && !own) {
                    continue;
                }
                let full: Vec<&str> = if own { name.to_vec() } else { name.iter().copied().chain([key.as_str()]).collect() };
                if own {
                    self.refs.insert(at.to_string(), self.tokens.len());
                }
                self.add(path, token(coll, full.join("/"), obj, ty));
            } else if !key.starts_with('$') && depth < MAX_DEPTH {
                let inner: Vec<&str> = name.iter().copied().chain([key.as_str()]).collect();
                self.walk(obj, coll, &path, &inner, obj.get("$type").and_then(Value::as_str).or(ty), depth + 1);
            }
        }
    }

    fn add(&mut self, path: String, t: Token<'a>) {
        self.refs.insert(path, self.tokens.len());
        self.tokens.push(t);
    }
}

fn token<'a>(coll: usize, name: String, obj: &'a Map<String, Value>, inherited: Option<&'a str>) -> Token<'a> {
    Token {
        coll,
        name,
        ty: obj.get("$type").and_then(Value::as_str).or(inherited),
        value: obj.get("$value").unwrap_or(&Value::Null),
        description: obj.get("$description").and_then(Value::as_str),
        ext: extension(obj),
    }
}

/// FigCraft's `$extensions` entry on a token or group.
fn extension(obj: &Map<String, Value>) -> Option<&Map<String, Value>> {
    obj.get("$extensions")?.get(EXTENSION)?.as_object()
}

/// The dotted path a `{reference}` value names.
fn reference(v: &Value) -> Option<&str> {
    let s = v.as_str()?.trim().strip_prefix('{')?.strip_suffix('}')?;
    (!s.is_empty() && !s.contains(['{', '}'])).then_some(s)
}

/// A token's variable type (`None`: an untyped alias, settled from its target, or a type
/// variables lack, counted in the warnings).
fn kind(t: &Token, w: &mut Warnings) -> Option<VariableType> {
    match t.ext.and_then(|e| e.get("type")).and_then(Value::as_str) {
        Some("string") => return Some(VariableType::String),
        Some("boolean") => return Some(VariableType::Boolean),
        _ => {}
    }
    match t.ty {
        Some("color") => Some(VariableType::Color),
        Some("dimension" | "number" | "fontWeight" | "duration") => Some(VariableType::Number),
        Some("fontFamily") => Some(VariableType::String),
        Some(other) => {
            *w.types.entry(other.chars().take(40).collect()).or_default() += 1;
            None
        }
        None if reference(t.value).is_some() => None,
        None => match t.value {
            Value::Bool(_) => Some(VariableType::Boolean),
            Value::Number(_) => Some(VariableType::Number),
            Value::String(_) => Some(VariableType::String),
            _ => None,
        },
    }
}

/// A colour value: the 2025.10 object (an sRGB or Display P3 one by its components, another space
/// by its hex) or a `#hex` string, converted into the file's profile.
fn color(v: &Value, from_srgb: &figcraft_doc::color::ProfileConversion, from_p3: &figcraft_doc::color::ProfileConversion) -> Option<Color> {
    let (c, p3) = match v {
        Value::String(s) if s.trim_start().starts_with('#') => (Color::from_hex(s)?, false),
        Value::Object(o) => {
            let alpha = o.get("alpha").and_then(Value::as_f64).unwrap_or(1.0) as f32;
            let parts: Option<Vec<f32>> = o
                .get("components")
                .and_then(Value::as_array)?
                .iter()
                .map(|c| c.as_f64().or_else(|| (c == "none").then_some(0.0)).map(|x| x as f32))
                .collect();
            match (o.get("colorSpace").and_then(Value::as_str), parts.as_deref()) {
                (Some(space @ ("srgb" | "display-p3")), Some([r, g, b])) => (Color::new(*r, *g, *b, alpha), space == "display-p3"),
                _ => (Color { a: alpha, ..Color::from_hex(o.get("hex").and_then(Value::as_str)?)? }, false),
            }
        }
        _ => return None,
    };
    Some(if p3 { from_p3.apply(c) } else { from_srgb.apply(c) }.sanitized())
}

/// A number value: a number, a dimension or duration (`{value, unit}`, or the older `"16px"`) in
/// px (a rem at [`REM_PX`], noted in `rems`) or ms, or a font weight's name.
fn number(v: &Value, rems: &mut bool) -> Option<f64> {
    let (x, unit) = match v {
        Value::Number(n) => (n.as_f64()?, ""),
        Value::Object(o) => (o.get("value")?.as_f64()?, o.get("unit").and_then(Value::as_str).unwrap_or("")),
        Value::String(s) => {
            let s = s.trim();
            if let Some(w) = weight(s) {
                return Some(w);
            }
            let split = s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len());
            (s[..split].trim().parse().ok()?, &s[split..])
        }
        _ => return None,
    };
    let x = match unit {
        "" | "px" | "ms" => x,
        "rem" => {
            *rems = true;
            x * REM_PX
        }
        "s" => x * 1000.0,
        _ => return None,
    };
    x.is_finite().then_some(x)
}

/// A font weight's name as its number (DTCG's list).
fn weight(name: &str) -> Option<f64> {
    Some(match name.to_ascii_lowercase().as_str() {
        "thin" | "hairline" => 100.0,
        "extra-light" | "ultra-light" => 200.0,
        "light" => 300.0,
        "normal" | "regular" | "book" => 400.0,
        "medium" => 500.0,
        "semi-bold" | "demi-bold" => 600.0,
        "bold" => 700.0,
        "extra-bold" | "ultra-bold" => 800.0,
        "black" | "heavy" => 900.0,
        "extra-black" | "ultra-black" => 950.0,
        _ => return None,
    })
}

/// A string value; a font family list gives its first family.
fn text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Array(a) => a.first()?.as_str().map(str::to_string),
        _ => None,
    }
}
