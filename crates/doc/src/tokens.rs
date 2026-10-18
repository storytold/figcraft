//! Design tokens in the Design Tokens Community Group format (DTCG 2025.10): token files (groups of
//! tokens with `$value`, `$type`, `$description`…, aliases as `{group.token}` or `$ref` JSON
//! pointers) and the resolver module's sets, modifiers (themes, brands: one context chosen per
//! modifier) and resolution order.
//!
//! A file keeps its tokens as the DTCG JSON they are written in, one tree per set and per modifier
//! context, so whatever a tool adds (`$extensions`, colour spaces we only convert) survives a
//! round trip. [`DesignTokens::resolve`] merges the trees in resolution order for an input (one
//! context per modifier) and resolves the aliases, as the resolver module describes.
//!
//! Files edited outside FigCraft are untrusted: every walk is bounded in depth and size, and a bad
//! token is reported in [`Resolved::errors`] instead of failing the rest.

use std::collections::{BTreeMap, BTreeSet};

use figcraft_color::Color;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// The resolver module version this reads and writes.
pub const RESOLVER_VERSION: &str = "2025.10";
/// How deep groups may nest (deeper ones are ignored).
pub const MAX_DEPTH: usize = 32;
/// How many tokens one resolution keeps at most.
pub const MAX_TOKENS: usize = 20_000;
/// How many aliases a value may chain through.
const MAX_ALIAS_HOPS: usize = 32;
/// Root font size `rem` dimensions are converted with.
pub const REM_PX: f64 = 16.0;

/// The token types of the format module.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TokenType {
    Color,
    Dimension,
    FontFamily,
    FontWeight,
    Duration,
    CubicBezier,
    Number,
    StrokeStyle,
    Border,
    Transition,
    Shadow,
    Gradient,
    Typography,
}

impl TokenType {
    /// Every type, simple ones first, in the order the editor offers them.
    pub const ALL: [TokenType; 13] = [
        TokenType::Color,
        TokenType::Dimension,
        TokenType::Number,
        TokenType::FontFamily,
        TokenType::FontWeight,
        TokenType::Duration,
        TokenType::CubicBezier,
        TokenType::StrokeStyle,
        TokenType::Border,
        TokenType::Shadow,
        TokenType::Gradient,
        TokenType::Typography,
        TokenType::Transition,
    ];

    /// The `$type` value.
    pub fn id(self) -> &'static str {
        match self {
            TokenType::Color => "color",
            TokenType::Dimension => "dimension",
            TokenType::FontFamily => "fontFamily",
            TokenType::FontWeight => "fontWeight",
            TokenType::Duration => "duration",
            TokenType::CubicBezier => "cubicBezier",
            TokenType::Number => "number",
            TokenType::StrokeStyle => "strokeStyle",
            TokenType::Border => "border",
            TokenType::Transition => "transition",
            TokenType::Shadow => "shadow",
            TokenType::Gradient => "gradient",
            TokenType::Typography => "typography",
        }
    }

    /// A type from its `$type` value (case-sensitive, as the format says).
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.id() == s)
    }

    /// The English label the editor shows.
    pub fn label(self) -> &'static str {
        match self {
            TokenType::Color => "Color",
            TokenType::Dimension => "Dimension",
            TokenType::FontFamily => "Font family",
            TokenType::FontWeight => "Font weight",
            TokenType::Duration => "Duration",
            TokenType::CubicBezier => "Cubic Bézier",
            TokenType::Number => "Number",
            TokenType::StrokeStyle => "Stroke style",
            TokenType::Border => "Border",
            TokenType::Transition => "Transition",
            TokenType::Shadow => "Shadow",
            TokenType::Gradient => "Gradient",
            TokenType::Typography => "Typography",
        }
    }

    /// Types whose value is an object or list of other values.
    pub fn is_composite(self) -> bool {
        matches!(
            self,
            TokenType::StrokeStyle | TokenType::Border | TokenType::Transition | TokenType::Shadow | TokenType::Gradient | TokenType::Typography
        )
    }

    /// The value a new token of this type starts with.
    pub fn default_value(self) -> Value {
        let px = |v: f64| json!({"value": v, "unit": "px"});
        match self {
            TokenType::Color => color_value(Color::BLACK),
            TokenType::Dimension => px(0.0),
            TokenType::FontFamily => json!("Inter"),
            TokenType::FontWeight => json!(400),
            TokenType::Duration => json!({"value": 200, "unit": "ms"}),
            TokenType::CubicBezier => json!([0.25, 0.1, 0.25, 1]),
            TokenType::Number => json!(0),
            TokenType::StrokeStyle => json!("solid"),
            TokenType::Border => json!({"color": color_value(Color::BLACK), "width": px(1.0), "style": "solid"}),
            TokenType::Transition => {
                json!({"duration": {"value": 200, "unit": "ms"}, "delay": {"value": 0, "unit": "ms"}, "timingFunction": [0.25, 0.1, 0.25, 1]})
            }
            TokenType::Shadow => json!({
                "color": color_value(Color::new(0.0, 0.0, 0.0, 0.25)),
                "offsetX": px(0.0), "offsetY": px(4.0), "blur": px(4.0), "spread": px(0.0)
            }),
            TokenType::Gradient => json!([
                {"color": color_value(Color::BLACK), "position": 0},
                {"color": color_value(Color::WHITE), "position": 1}
            ]),
            TokenType::Typography => json!({
                "fontFamily": "Inter", "fontSize": px(16.0), "fontWeight": 400,
                "letterSpacing": px(0.0), "lineHeight": 1.5
            }),
        }
    }
}

/// A named set of tokens (always part of the resolution when listed in the order).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TokenSet {
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// The DTCG token tree (an object of groups and tokens).
    pub tokens: Value,
}

/// One context of a modifier ("light", "dark", "brand-a"…) and the tokens it brings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TokenContext {
    pub name: String,
    pub tokens: Value,
}

/// A modifier: alternative contexts of which one is chosen per resolution (a theme, a brand, a
/// density…).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TokenModifier {
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub contexts: Vec<TokenContext>,
    /// The context used when none is chosen (a context name).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

impl TokenModifier {
    pub fn context(&self, name: &str) -> Option<&TokenContext> {
        self.contexts.iter().find(|c| c.name == name).or_else(|| self.contexts.iter().find(|c| c.name.eq_ignore_ascii_case(name)))
    }

    /// The context chosen when the input names none (or one it doesn't have).
    pub fn fallback(&self) -> Option<&TokenContext> {
        self.default.as_deref().and_then(|d| self.context(d)).or_else(|| self.contexts.first())
    }
}

/// One step of the resolution order: a set or a modifier, by name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "type", content = "name", rename_all = "camelCase")]
pub enum OrderItem {
    Set(String),
    Modifier(String),
}

/// Where a token tree lives: a set, or one context of a modifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Set(String),
    Context { modifier: String, context: String },
}

/// A file's design tokens.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DesignTokens {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub sets: Vec<TokenSet>,
    pub modifiers: Vec<TokenModifier>,
    /// The resolution order: later entries override earlier ones.
    pub order: Vec<OrderItem>,
    /// The context chosen for each modifier in this file (layers can choose others).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub active: BTreeMap<String, String>,
}

/// A token after resolution.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedToken {
    /// Dot-separated path (`color.brand.primary`).
    pub path: String,
    pub kind: Option<TokenType>,
    /// The value with every alias replaced (`Null` when it couldn't be resolved).
    pub value: Value,
    /// The value as written.
    pub raw: Value,
    /// The token the value is an alias of, when it is a whole-value alias.
    pub alias: Option<String>,
    pub description: String,
    pub deprecated: bool,
    /// Why the token couldn't be resolved.
    pub error: Option<String>,
}

/// The result of a resolution: tokens by path, and what went wrong.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Resolved {
    pub tokens: BTreeMap<String, ResolvedToken>,
    pub errors: Vec<String>,
}

impl Resolved {
    pub fn get(&self, path: &str) -> Option<&ResolvedToken> {
        self.tokens.get(path).filter(|t| t.error.is_none())
    }
}

impl DesignTokens {
    pub fn is_empty(&self) -> bool {
        self.sets.is_empty() && self.modifiers.is_empty() && self.name.is_empty() && self.description.is_empty()
    }

    pub fn set(&self, name: &str) -> Option<&TokenSet> {
        self.sets.iter().find(|s| s.name == name)
    }

    pub fn modifier(&self, name: &str) -> Option<&TokenModifier> {
        self.modifiers.iter().find(|m| m.name == name).or_else(|| self.modifiers.iter().find(|m| m.name.eq_ignore_ascii_case(name)))
    }

    /// The token tree of `src`.
    pub fn tree(&self, src: &Source) -> Option<&Value> {
        match src {
            Source::Set(s) => self.set(s).map(|s| &s.tokens),
            Source::Context { modifier, context } => self.modifier(modifier).and_then(|m| m.context(context)).map(|c| &c.tokens),
        }
    }

    /// The token tree of `src`, to edit (made an object if it wasn't).
    pub fn tree_mut(&mut self, src: &Source) -> Option<&mut Value> {
        let tree = match src {
            Source::Set(s) => self.sets.iter_mut().find(|x| x.name == *s).map(|s| &mut s.tokens),
            Source::Context { modifier, context } => self
                .modifiers
                .iter_mut()
                .find(|m| m.name == *modifier)
                .and_then(|m| m.contexts.iter_mut().find(|c| c.name == *context))
                .map(|c| &mut c.tokens),
        }?;
        if !tree.is_object() {
            *tree = Value::Object(Map::new());
        }
        Some(tree)
    }

    /// Every token tree with where it lives, in resolution order (sets and modifiers missing from
    /// the order last).
    pub fn sources(&self) -> Vec<Source> {
        let mut out = Vec::new();
        let push = |item: &OrderItem, out: &mut Vec<Source>| match item {
            OrderItem::Set(s) if self.set(s).is_some() => out.push(Source::Set(s.clone())),
            OrderItem::Modifier(m) => {
                if let Some(m) = self.modifier(m) {
                    out.extend(m.contexts.iter().map(|c| Source::Context { modifier: m.name.clone(), context: c.name.clone() }));
                }
            }
            OrderItem::Set(_) => {}
        };
        for item in self.full_order() {
            push(&item, &mut out);
        }
        out
    }

    /// The resolution order with every set and modifier in it once (those the order misses
    /// after the rest, unknown names dropped).
    pub fn full_order(&self) -> Vec<OrderItem> {
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for item in &self.order {
            let known = match item {
                OrderItem::Set(s) => self.set(s).is_some(),
                OrderItem::Modifier(m) => self.modifier(m).is_some(),
            };
            if known && seen.insert(item.clone()) {
                out.push(item.clone());
            }
        }
        for s in &self.sets {
            let it = OrderItem::Set(s.name.clone());
            if seen.insert(it.clone()) {
                out.push(it);
            }
        }
        for m in &self.modifiers {
            let it = OrderItem::Modifier(m.name.clone());
            if seen.insert(it.clone()) {
                out.push(it);
            }
        }
        out
    }

    /// The input (modifier → context) for this file's choice overridden by `overrides`, with every
    /// modifier given a context it has (unknown modifiers dropped, unknown contexts replaced by the
    /// modifier's default).
    pub fn input_for(&self, overrides: &BTreeMap<String, String>) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for m in &self.modifiers {
            let pick = overrides.get(&m.name).or_else(|| self.active.get(&m.name)).and_then(|c| m.context(c)).or_else(|| m.fallback());
            if let Some(c) = pick {
                out.insert(m.name.clone(), c.name.clone());
            }
        }
        out
    }

    /// Check `input` as the resolver module asks: known modifiers, known contexts, and a context
    /// for each modifier without a default. Returns the complete input.
    pub fn validate_input(&self, input: &Map<String, Value>) -> Result<BTreeMap<String, String>, String> {
        let mut out = BTreeMap::new();
        for (k, v) in input {
            let m = self.modifier(k).ok_or_else(|| format!("no modifier {k}"))?;
            let name = v.as_str().ok_or_else(|| format!("{k}: a context name (a string)"))?;
            let c = m.context(name).ok_or_else(|| format!("modifier {} has no context {name}", m.name))?;
            out.insert(m.name.clone(), c.name.clone());
        }
        for m in &self.modifiers {
            if out.contains_key(&m.name) {
                continue;
            }
            let d = m.default.as_deref().and_then(|d| m.context(d)).ok_or_else(|| format!("modifier {} needs a context", m.name))?;
            out.insert(m.name.clone(), d.name.clone());
        }
        Ok(out)
    }

    /// The merged token tree for `input` (modifier → context; missing ones use their fallback).
    pub fn merged(&self, input: &BTreeMap<String, String>) -> Value {
        let mut out = Value::Object(Map::new());
        for item in self.full_order() {
            match item {
                OrderItem::Set(s) => {
                    if let Some(s) = self.set(&s) {
                        merge_into(&mut out, &s.tokens, 0);
                    }
                }
                OrderItem::Modifier(m) => {
                    let Some(m) = self.modifier(&m) else { continue };
                    let c = input.get(&m.name).and_then(|c| m.context(c)).or_else(|| m.fallback());
                    if let Some(c) = c {
                        merge_into(&mut out, &c.tokens, 0);
                    }
                }
            }
        }
        out
    }

    /// Resolve every token for `input`.
    pub fn resolve(&self, input: &BTreeMap<String, String>) -> Resolved {
        resolve_tree(&self.merged(input))
    }

    /// Every token path of every tree, with its type when known (for pickers and checks).
    pub fn all_paths(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for src in self.sources() {
            if let Some(t) = self.tree(&src) {
                for tok in flatten(t) {
                    out.insert(tok.path);
                }
            }
        }
        out
    }
}

/// A token as found in a tree (before aliases are resolved).
#[derive(Clone, Debug, PartialEq)]
pub struct RawToken {
    pub path: String,
    /// Its own `$type`, else the nearest group's.
    pub kind: Option<TokenType>,
    /// Whether the type was set on the token itself.
    pub own_kind: bool,
    pub value: Value,
    pub description: String,
    pub deprecated: bool,
    pub extensions: Option<Value>,
}

/// Whether `v` is a token (an object with `$value`).
pub fn is_token(v: &Value) -> bool {
    v.as_object().is_some_and(|o| o.contains_key("$value"))
}

/// Whether `name` can name a token or group: not empty, no leading `$`, no `{`, `}` or `.`.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('$') && !name.contains(['{', '}', '.']) && name.len() <= 256
}

/// Split a dot path into names (`$root` allowed as the last one).
pub fn split_path(path: &str) -> Option<Vec<String>> {
    let parts: Vec<String> = path.split('.').map(str::to_string).collect();
    let n = parts.len();
    let ok = n > 0 && n <= MAX_DEPTH && parts.iter().enumerate().all(|(i, p)| valid_name(p) || (i + 1 == n && p == "$root" && n > 1));
    ok.then_some(parts)
}

/// Every token of `tree` (depth-first, in the tree's key order), with group types inherited.
pub fn flatten(tree: &Value) -> Vec<RawToken> {
    let mut out = Vec::new();
    walk(tree, &mut Vec::new(), None, &mut out, 0);
    out
}

fn walk(node: &Value, path: &mut Vec<String>, group_type: Option<TokenType>, out: &mut Vec<RawToken>, depth: usize) {
    let Some(obj) = node.as_object() else { return };
    if depth > MAX_DEPTH || out.len() >= MAX_TOKENS {
        return;
    }
    let own = obj.get("$type").and_then(Value::as_str).and_then(TokenType::parse);
    if obj.contains_key("$value") {
        if path.is_empty() {
            return;
        }
        let deprecated = match obj.get("$deprecated") {
            Some(Value::Bool(b)) => *b,
            Some(Value::String(_)) => true,
            _ => false,
        };
        out.push(RawToken {
            path: path.join("."),
            kind: own.or(group_type),
            own_kind: own.is_some(),
            value: obj.get("$value").cloned().unwrap_or(Value::Null),
            description: obj.get("$description").and_then(Value::as_str).unwrap_or_default().to_string(),
            deprecated,
            extensions: obj.get("$extensions").cloned(),
        });
        return;
    }
    let inherited = own.or(group_type);
    for (k, v) in obj {
        if k.starts_with('$') && k != "$root" {
            continue;
        }
        path.push(k.clone());
        walk(v, path, inherited, out, depth + 1);
        path.pop();
    }
}

/// Merge `src` into `dst`: groups merge key by key, a token replaces whatever was at its path.
pub fn merge_into(dst: &mut Value, src: &Value, depth: usize) {
    let Some(s) = src.as_object() else { return };
    if !dst.is_object() || is_token(dst) || is_token(src) || depth > MAX_DEPTH {
        *dst = src.clone();
        return;
    }
    let Some(d) = dst.as_object_mut() else { return };
    for (k, v) in s {
        match d.get_mut(k) {
            Some(slot) if v.is_object() && !is_token(v) && slot.is_object() && !is_token(slot) => merge_into(slot, v, depth + 1),
            _ => {
                d.insert(k.clone(), v.clone());
            }
        }
    }
}

/// Apply groups' `$extends` (a `{group}` reference or a `#/group` pointer): the group is the
/// referenced group's content with its own merged over it. Cycles and references to tokens are
/// left alone.
fn apply_extends(tree: &mut Value) {
    // Paths of groups that extend another, outermost first.
    fn find(node: &Value, path: &mut Vec<String>, out: &mut Vec<(Vec<String>, String)>, depth: usize) {
        let Some(o) = node.as_object() else { return };
        if depth > MAX_DEPTH || is_token(node) {
            return;
        }
        if let Some(r) = o.get("$extends").and_then(Value::as_str) {
            out.push((path.clone(), r.to_string()));
        }
        for (k, v) in o {
            if !k.starts_with('$') {
                path.push(k.clone());
                find(v, path, out, depth + 1);
                path.pop();
            }
        }
    }
    let mut groups = Vec::new();
    find(tree, &mut Vec::new(), &mut groups, 0);
    // A few rounds settle chains (a extends b extends c).
    for _ in 0..4 {
        for (path, target) in &groups {
            let Some(tpath) = reference_path(target) else { continue };
            if tpath.starts_with(path.as_slice()) || path.starts_with(tpath.as_slice()) {
                continue;
            }
            let Some(base) = at_path(tree, &tpath).filter(|b| b.is_object() && !is_token(b)).cloned() else { continue };
            let Some(slot) = at_path_mut(tree, path) else { continue };
            let mut merged = base;
            merge_into(&mut merged, slot, 0);
            if let Some(o) = merged.as_object_mut() {
                o.remove("$extends");
            }
            // Keep the reference so the next rounds can follow chains that settle later.
            if let Some(o) = merged.as_object_mut() {
                o.insert("$extends".into(), Value::String(target.clone()));
            }
            *slot = merged;
        }
    }
    for (path, _) in &groups {
        if let Some(o) = at_path_mut(tree, path).and_then(Value::as_object_mut) {
            o.remove("$extends");
        }
    }
}

/// The path a `{a.b}` reference or `#/a/b` pointer names.
fn reference_path(r: &str) -> Option<Vec<String>> {
    if let Some(inner) = r.strip_prefix('{').and_then(|x| x.strip_suffix('}')) {
        return split_path(inner);
    }
    let p = r.strip_prefix('#')?;
    let parts: Vec<String> = p.split('/').skip(1).map(|s| s.replace("~1", "/").replace("~0", "~")).collect();
    (!parts.is_empty() && parts.len() <= MAX_DEPTH * 2).then_some(parts)
}

pub fn at_path<'a>(tree: &'a Value, path: &[String]) -> Option<&'a Value> {
    let mut cur = tree;
    for p in path {
        cur = match cur {
            Value::Object(o) => o.get(p)?,
            Value::Array(a) => a.get(p.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

pub fn at_path_mut<'a>(tree: &'a mut Value, path: &[String]) -> Option<&'a mut Value> {
    let mut cur = tree;
    for p in path {
        cur = match cur {
            Value::Object(o) => o.get_mut(p)?,
            Value::Array(a) => a.get_mut(p.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

/// The token path `v` is a whole-value alias of (`"{color.brand}"`).
pub fn alias_of(v: &Value) -> Option<String> {
    let s = v.as_str()?.trim();
    let inner = s.strip_prefix('{')?.strip_suffix('}')?;
    split_path(inner).map(|_| inner.to_string())
}

/// Resolve every token of a merged tree (groups' `$extends` applied first).
pub fn resolve_tree(tree: &Value) -> Resolved {
    let mut extended = tree.clone();
    apply_extends(&mut extended);
    let tree = &extended;
    let raw = flatten(tree);
    let by_path: BTreeMap<String, RawToken> = raw.iter().map(|t| (t.path.clone(), t.clone())).collect();
    let mut out = Resolved::default();
    for t in &raw {
        let mut visiting = BTreeSet::new();
        visiting.insert(t.path.clone());
        let alias = alias_of(&t.value);
        // The type: its own, else the alias target's, else the group's.
        let kind = if t.own_kind { t.kind } else { alias.as_deref().and_then(|a| type_through(&by_path, a, 0)).or(t.kind) };
        let res = resolve_value(&t.value, tree, &by_path, &mut visiting, 0);
        let mut error = None;
        let value = match res {
            Ok(v) => v,
            Err(e) => {
                error = Some(e);
                Value::Null
            }
        };
        if error.is_none()
            && let (Some(a), Some(k)) = (alias.as_deref(), kind)
            && let Some(tk) = type_through(&by_path, a, 0)
            && tk != k
        {
            error = Some(format!("{} is a {} alias of {a}, a {}", t.path, k.id(), tk.id()));
        }
        if error.is_none() && kind.is_none() {
            error = Some(format!("{} has no $type", t.path));
        }
        if let Some(e) = &error {
            out.errors.push(e.clone());
        }
        out.tokens.insert(
            t.path.clone(),
            ResolvedToken {
                path: t.path.clone(),
                kind,
                value,
                raw: t.value.clone(),
                alias,
                description: t.description.clone(),
                deprecated: t.deprecated,
                error,
            },
        );
    }
    out
}

/// The type of token `path`, following aliases.
fn type_through(by_path: &BTreeMap<String, RawToken>, path: &str, hops: usize) -> Option<TokenType> {
    let t = by_path.get(path)?;
    if t.own_kind || hops >= MAX_ALIAS_HOPS {
        return t.kind;
    }
    match alias_of(&t.value) {
        Some(a) => type_through(by_path, &a, hops + 1).or(t.kind),
        None => t.kind,
    }
}

fn resolve_value(
    v: &Value,
    tree: &Value,
    by_path: &BTreeMap<String, RawToken>,
    visiting: &mut BTreeSet<String>,
    depth: usize,
) -> Result<Value, String> {
    if depth > MAX_ALIAS_HOPS * 2 {
        return Err("aliases nest too deep".into());
    }
    if let Some(a) = alias_of(v) {
        let target = by_path.get(&a).ok_or_else(|| format!("{{{a}}} names no token"))?;
        if !visiting.insert(a.clone()) {
            return Err(format!("circular alias through {a}"));
        }
        let r = resolve_value(&target.value, tree, by_path, visiting, depth + 1);
        visiting.remove(&a);
        return r;
    }
    match v {
        Value::Object(o) if o.len() == 1 && o.get("$ref").is_some_and(Value::is_string) => {
            let r = o.get("$ref").and_then(Value::as_str).unwrap_or_default();
            if !r.starts_with('#') {
                return Err(format!("{r}: only same-document references resolve here"));
            }
            let path = reference_path(r).ok_or_else(|| format!("{r}: not a JSON pointer"))?;
            let key = format!("$ref:{r}");
            if !visiting.insert(key.clone()) {
                return Err(format!("circular reference through {r}"));
            }
            let target = at_path(tree, &path).ok_or_else(|| format!("{r} points at nothing"))?.clone();
            // A pointer at a token means its value.
            let target = if is_token(&target) { target.get("$value").cloned().unwrap_or(Value::Null) } else { target };
            let res = resolve_value(&target, tree, by_path, visiting, depth + 1);
            visiting.remove(&key);
            res
        }
        Value::Object(o) => {
            let mut m = Map::new();
            for (k, x) in o {
                m.insert(k.clone(), resolve_value(x, tree, by_path, visiting, depth + 1)?);
            }
            Ok(Value::Object(m))
        }
        Value::Array(a) => a.iter().map(|x| resolve_value(x, tree, by_path, visiting, depth + 1)).collect::<Result<Vec<_>, _>>().map(Value::Array),
        _ => Ok(v.clone()),
    }
}

// ----- Editing token trees -----

/// The token at `path` in `tree`.
pub fn token_at<'a>(tree: &'a Value, path: &str) -> Option<&'a Value> {
    at_path(tree, &split_path(path)?).filter(|v| is_token(v))
}

/// Put a token at `path` (groups made on the way; a token in the way of a group is an error).
pub fn put_token(tree: &mut Value, path: &str, token: Value) -> Result<(), String> {
    let parts = split_path(path).ok_or_else(|| format!("{path}: names are dot-separated, without {{, }} or a leading $"))?;
    if !tree.is_object() {
        *tree = Value::Object(Map::new());
    }
    let mut cur = tree;
    let n = parts.len();
    for (i, p) in parts.iter().enumerate() {
        let last = i + 1 == n;
        let Some(obj) = cur.as_object_mut() else { return Err(format!("{path}: not inside a group")) };
        if last {
            if obj.get(p).is_some_and(|v| v.is_object() && !is_token(v) && v.as_object().is_some_and(|o| o.keys().any(|k| !k.starts_with('$')))) {
                return Err(format!("{path} is a group"));
            }
            obj.insert(p.clone(), token);
            return Ok(());
        }
        let slot = obj.entry(p.clone()).or_insert_with(|| Value::Object(Map::new()));
        if is_token(slot) || !slot.is_object() {
            return Err(format!("{} is a token, not a group", parts.get(..=i).map(|x| x.join(".")).unwrap_or_default()));
        }
        cur = slot;
    }
    Ok(())
}

/// Remove the token or group at `path`; empty groups left behind go too. The removed value.
pub fn remove_at(tree: &mut Value, path: &str) -> Option<Value> {
    let parts = split_path(path)?;
    let (last, parents) = parts.split_last()?;
    let parent = at_path_mut(tree, parents)?.as_object_mut()?;
    let out = parent.remove(last)?;
    // Prune empty groups upwards.
    let mut cut = parents.to_vec();
    while let Some(name) = cut.pop() {
        let empty = at_path(tree, &[cut.clone(), vec![name.clone()]].concat())
            .and_then(Value::as_object)
            .is_some_and(|o| o.keys().all(|k| k.starts_with('$')) && !o.contains_key("$value"));
        if !empty {
            break;
        }
        if let Some(o) = at_path_mut(tree, &cut).and_then(Value::as_object_mut) {
            o.remove(&name);
        }
    }
    Some(out)
}

/// Rewrite aliases to `from` (or below it, for a group) to point at `to` in every value of `tree`.
pub fn rename_aliases(tree: &mut Value, from: &str, to: &str) -> usize {
    fn fix(v: &mut Value, from: &str, to: &str, n: &mut usize, depth: usize) {
        if depth > MAX_DEPTH * 2 {
            return;
        }
        match v {
            Value::String(s) => {
                if let Some(a) = alias_of(&Value::String(s.clone()))
                    && let Some(rest) = a.strip_prefix(from)
                    && (rest.is_empty() || rest.starts_with('.'))
                {
                    *s = format!("{{{to}{rest}}}");
                    *n += 1;
                }
            }
            Value::Array(a) => a.iter_mut().for_each(|x| fix(x, from, to, n, depth + 1)),
            Value::Object(o) => o.values_mut().for_each(|x| fix(x, from, to, n, depth + 1)),
            _ => {}
        }
    }
    let mut n = 0;
    fix(tree, from, to, &mut n, 0);
    n
}

/// Build a token object.
pub fn make_token(kind: Option<TokenType>, value: Value, description: &str) -> Value {
    let mut o = Map::new();
    if let Some(k) = kind {
        o.insert("$type".into(), Value::String(k.id().into()));
    }
    o.insert("$value".into(), value);
    if !description.is_empty() {
        o.insert("$description".into(), Value::String(description.into()));
    }
    Value::Object(o)
}

// ----- Values -----

/// A colour as a DTCG colour value (sRGB, with its hex).
pub fn color_value(c: Color) -> Value {
    let c = c.sanitized();
    let r = |x: f32| (f64::from(x) * 10_000.0).round() / 10_000.0;
    let mut o = Map::new();
    o.insert("colorSpace".into(), json!("srgb"));
    o.insert("components".into(), json!([r(c.r), r(c.g), r(c.b)]));
    if c.a < 1.0 {
        o.insert("alpha".into(), json!(r(c.a)));
    }
    o.insert("hex".into(), json!(c.with_alpha(1.0).to_css_hex().to_lowercase()));
    Value::Object(o)
}

/// A colour from a DTCG colour value (any colour space of the colour module, converted to sRGB;
/// a `#rrggbb` string as older files write it too).
pub fn color_of(v: &Value) -> Option<Color> {
    if let Some(s) = v.as_str() {
        return Color::from_hex(s).or_else(|| Color::parse_css(s));
    }
    let o = v.as_object()?;
    let alpha = match o.get("alpha") {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(1.0),
        _ => 1.0,
    };
    let comps: Vec<f64> =
        o.get("components").and_then(Value::as_array).map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect()).unwrap_or_default();
    let space = o.get("colorSpace").and_then(Value::as_str).unwrap_or("srgb");
    let rgb = match (comps.as_slice(), space) {
        ([a, b, c], s) => convert(s, [*a, *b, *c]),
        _ => None,
    };
    let rgb = rgb.or_else(|| {
        let c = Color::from_hex(o.get("hex")?.as_str()?)?;
        Some([f64::from(c.r), f64::from(c.g), f64::from(c.b)])
    })?;
    if !rgb.iter().all(|x| x.is_finite()) || !alpha.is_finite() {
        return None;
    }
    let f = |x: f64| x.clamp(0.0, 1.0) as f32;
    Some(Color::new(f(rgb[0]), f(rgb[1]), f(rgb[2]), f(alpha)))
}

type M3 = [[f64; 3]; 3];

fn mul(m: &M3, v: [f64; 3]) -> [f64; 3] {
    let row = |r: &[f64; 3]| r[0] * v[0] + r[1] * v[1] + r[2] * v[2];
    [row(&m[0]), row(&m[1]), row(&m[2])]
}

const XYZ_TO_SRGB: M3 = [
    [3.240_969_941_904_522_6, -1.537_383_177_570_094, -0.498_610_760_293_003_4],
    [-0.969_243_636_280_879_6, 1.875_967_501_507_720_2, 0.041_555_057_407_175_59],
    [0.055_630_079_696_993_66, -0.203_976_958_888_976_52, 1.056_971_514_242_878_6],
];
const P3_TO_XYZ: M3 = [
    [0.486_570_948_648_216_2, 0.265_667_693_169_093_06, 0.198_217_285_234_362_5],
    [0.228_974_564_069_748_8, 0.691_738_521_836_506_4, 0.079_286_914_093_745],
    [0.0, 0.045_113_381_858_902_64, 1.043_944_368_900_976],
];
const A98_TO_XYZ: M3 = [
    [0.576_669_042_910_130_5, 0.185_558_237_906_546_3, 0.188_228_646_234_994_7],
    [0.297_344_975_250_536_05, 0.627_363_566_255_466_1, 0.075_291_458_493_997_88],
    [0.027_031_361_386_412_34, 0.070_688_852_535_827_23, 0.991_337_536_837_638_8],
];
const REC2020_TO_XYZ: M3 = [
    [0.636_958_048_301_291_4, 0.144_616_903_586_208_32, 0.168_880_975_164_172_1],
    [0.262_700_212_011_267_1, 0.677_998_071_518_870_8, 0.059_301_716_469_861_96],
    [0.0, 0.028_072_693_049_087_428, 1.060_985_057_710_791],
];
const PROPHOTO_TO_XYZ50: M3 = [
    [0.797_760_489_672_302_7, 0.135_185_837_175_740_31, 0.031_349_349_581_524_8],
    [0.288_071_128_229_293_4, 0.711_843_217_810_101_4, 0.000_085_653_960_605_259_02],
    [0.0, 0.0, 0.825_104_602_510_460_1],
];
const D50_TO_D65: M3 = [
    [0.955_473_452_704_218_2, -0.023_098_536_874_261_423, 0.063_259_308_661_021_7],
    [-0.028_369_706_963_208_136, 1.009_995_458_005_822_6, 0.021_041_398_966_943_008],
    [0.012_314_001_688_319_899, -0.020_507_696_433_477_912, 1.330_365_936_608_075_3],
];
const D50_WHITE: [f64; 3] = [0.3457 / 0.3585, 1.0, (1.0 - 0.3457 - 0.3585) / 0.3585];

fn srgb_encode(x: f64) -> f64 {
    let a = x.abs();
    let e = if a <= 0.003_130_8 { 12.92 * a } else { 1.055 * a.powf(1.0 / 2.4) - 0.055 };
    e.copysign(x)
}

fn srgb_decode(x: f64) -> f64 {
    let a = x.abs();
    let d = if a <= 0.040_45 { a / 12.92 } else { ((a + 0.055) / 1.055).powf(2.4) };
    d.copysign(x)
}

fn lin_to_srgb(l: [f64; 3]) -> [f64; 3] {
    l.map(srgb_encode)
}

fn xyz65_to_srgb(xyz: [f64; 3]) -> [f64; 3] {
    lin_to_srgb(mul(&XYZ_TO_SRGB, xyz))
}

fn lab_to_xyz50(l: f64, a: f64, b: f64) -> [f64; 3] {
    let kappa = 24_389.0 / 27.0;
    let eps = 216.0 / 24_389.0;
    let f1 = (l + 16.0) / 116.0;
    let f0 = a / 500.0 + f1;
    let f2 = f1 - b / 200.0;
    let x = if f0.powi(3) > eps { f0.powi(3) } else { (116.0 * f0 - 16.0) / kappa };
    let y = if l > kappa * eps { f1.powi(3) } else { l / kappa };
    let z = if f2.powi(3) > eps { f2.powi(3) } else { (116.0 * f2 - 16.0) / kappa };
    [x * D50_WHITE[0], y * D50_WHITE[1], z * D50_WHITE[2]]
}

fn oklab_to_lin(l: f64, a: f64, b: f64) -> [f64; 3] {
    let l_ = l + 0.396_337_777_4 * a + 0.215_803_757_3 * b;
    let m_ = l - 0.105_561_345_8 * a - 0.063_854_172_8 * b;
    let s_ = l - 0.089_484_177_5 * a - 1.291_485_548 * b;
    let (l3, m3, s3) = (l_.powi(3), m_.powi(3), s_.powi(3));
    [
        4.076_741_662_1 * l3 - 3.307_711_591_3 * m3 + 0.230_969_929_2 * s3,
        -1.268_438_004_6 * l3 + 2.609_757_401_1 * m3 - 0.341_319_396_5 * s3,
        -0.004_196_086_3 * l3 - 0.703_418_614_7 * m3 + 1.707_614_701 * s3,
    ]
}

fn polar(c: f64, h: f64) -> (f64, f64) {
    let r = h.to_radians();
    (c * r.cos(), c * r.sin())
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> [f64; 3] {
    let (s, l) = (s / 100.0, l / 100.0);
    let f = |n: f64| {
        let k = (n + h.rem_euclid(360.0) / 30.0) % 12.0;
        let a = s * l.min(1.0 - l);
        l - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
    };
    [f(0.0), f(8.0), f(4.0)]
}

/// `[r, g, b]` in sRGB from three components in colour space `space`.
fn convert(space: &str, [a, b, c]: [f64; 3]) -> Option<[f64; 3]> {
    Some(match space {
        "srgb" => [a, b, c],
        "srgb-linear" => lin_to_srgb([a, b, c]),
        "hsl" => hsl_to_rgb(a, b, c),
        "hwb" => {
            let (w, bl) = (b / 100.0, c / 100.0);
            if w + bl >= 1.0 {
                let g = w / (w + bl);
                [g, g, g]
            } else {
                hsl_to_rgb(a, 100.0, 50.0).map(|x| x * (1.0 - w - bl) + w)
            }
        }
        "display-p3" => xyz65_to_srgb(mul(&P3_TO_XYZ, [a, b, c].map(srgb_decode))),
        "a98-rgb" => xyz65_to_srgb(mul(&A98_TO_XYZ, [a, b, c].map(|x| x.abs().powf(563.0 / 256.0).copysign(x)))),
        "prophoto-rgb" => {
            let d = |x: f64| {
                let ax = x.abs();
                (if ax <= 16.0 / 512.0 { ax / 16.0 } else { ax.powf(1.8) }).copysign(x)
            };
            xyz65_to_srgb(mul(&D50_TO_D65, mul(&PROPHOTO_TO_XYZ50, [a, b, c].map(d))))
        }
        "rec2020" => {
            let (al, be) = (1.099_296_826_809_44, 0.018_053_968_510_807);
            let d = |x: f64| {
                let ax = x.abs();
                (if ax < be * 4.5 { ax / 4.5 } else { ((ax + al - 1.0) / al).powf(1.0 / 0.45) }).copysign(x)
            };
            xyz65_to_srgb(mul(&REC2020_TO_XYZ, [a, b, c].map(d)))
        }
        "xyz-d65" | "xyz" => xyz65_to_srgb([a, b, c]),
        "xyz-d50" => xyz65_to_srgb(mul(&D50_TO_D65, [a, b, c])),
        "lab" => xyz65_to_srgb(mul(&D50_TO_D65, lab_to_xyz50(a, b, c))),
        "lch" => {
            let (x, y) = polar(b, c);
            xyz65_to_srgb(mul(&D50_TO_D65, lab_to_xyz50(a, x, y)))
        }
        "oklab" => lin_to_srgb(oklab_to_lin(a, b, c)),
        "oklch" => {
            let (x, y) = polar(b, c);
            lin_to_srgb(oklab_to_lin(a, x, y))
        }
        _ => return None,
    })
}

/// A dimension in pixels (`rem` at [`REM_PX`]; a bare number or a `"12px"` string as older files
/// write them).
pub fn dimension_px(v: &Value) -> Option<f64> {
    let px = match v {
        Value::Object(o) => {
            let n = o.get("value")?.as_f64()?;
            match o.get("unit").and_then(Value::as_str).unwrap_or("px") {
                "px" => n,
                "rem" | "em" => n * REM_PX,
                _ => return None,
            }
        }
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => {
            let s = s.trim();
            if let Some(n) = s.strip_suffix("rem") {
                n.trim().parse::<f64>().ok()? * REM_PX
            } else {
                s.strip_suffix("px").unwrap_or(s).trim().parse().ok()?
            }
        }
        _ => return None,
    };
    px.is_finite().then_some(px)
}

/// A dimension value in pixels.
pub fn px_value(px: f64) -> Value {
    json!({"value": (px * 1000.0).round() / 1000.0, "unit": "px"})
}

/// A duration in milliseconds.
pub fn duration_ms(v: &Value) -> Option<f64> {
    let o = v.as_object()?;
    let n = o.get("value")?.as_f64()?;
    let ms = match o.get("unit").and_then(Value::as_str)? {
        "ms" => n,
        "s" => n * 1000.0,
        _ => return None,
    };
    ms.is_finite().then_some(ms)
}

/// The font weight keywords and their numbers.
pub const FONT_WEIGHTS: [(&str, u16); 17] = [
    ("thin", 100),
    ("hairline", 100),
    ("extra-light", 200),
    ("ultra-light", 200),
    ("light", 300),
    ("normal", 400),
    ("regular", 400),
    ("book", 400),
    ("medium", 500),
    ("semi-bold", 600),
    ("demi-bold", 600),
    ("bold", 700),
    ("extra-bold", 800),
    ("ultra-bold", 800),
    ("black", 900),
    ("heavy", 900),
    ("extra-black", 950),
];

/// A font weight (1–1000) from a number or keyword.
pub fn font_weight_of(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64().filter(|w| (1.0..=1000.0).contains(w)),
        Value::String(s) if s == "ultra-black" => Some(950.0),
        Value::String(s) => FONT_WEIGHTS.iter().find(|(k, _)| k == s).map(|(_, w)| f64::from(*w)),
        _ => None,
    }
}

/// The preferred family of a font family value.
pub fn font_family_of(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()).filter(|s| !s.trim().is_empty()),
        Value::Array(a) => a.iter().find_map(Value::as_str).map(str::to_string),
        _ => None,
    }
}

/// A short text for a resolved value of type `kind` (the editor's value column).
pub fn display(kind: Option<TokenType>, v: &Value) -> String {
    let dim = |v: &Value| match v {
        Value::Object(o) => match (o.get("value").and_then(Value::as_f64), o.get("unit").and_then(Value::as_str)) {
            (Some(n), Some(u)) => format!("{}{u}", crate::text::trim_num(n)),
            _ => compact(v),
        },
        _ => compact(v),
    };
    match kind {
        Some(TokenType::Color) => {
            color_of(v).map(|c| if c.a < 1.0 { c.to_hex() } else { c.with_alpha(1.0).to_css_hex() }).unwrap_or_else(|| compact(v))
        }
        Some(TokenType::Dimension | TokenType::Duration) => dim(v),
        Some(TokenType::FontFamily) => match v {
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "),
            _ => font_family_of(v).unwrap_or_else(|| compact(v)),
        },
        Some(TokenType::Number | TokenType::FontWeight) => match v {
            Value::Number(n) => n.as_f64().map(crate::text::trim_num).unwrap_or_default(),
            _ => compact(v),
        },
        Some(TokenType::CubicBezier) => match v.as_array() {
            Some(a) => format!("({})", a.iter().map(|x| x.as_f64().map(crate::text::trim_num).unwrap_or_default()).collect::<Vec<_>>().join(", ")),
            None => compact(v),
        },
        Some(TokenType::Typography) => {
            let fam = v.get("fontFamily").and_then(font_family_of).unwrap_or_default();
            let size = v.get("fontSize").map(dim).unwrap_or_default();
            let w = v.get("fontWeight").and_then(font_weight_of).map(crate::text::trim_num).unwrap_or_default();
            format!("{fam} {size} · {w}").trim().to_string()
        }
        Some(TokenType::Border) => {
            let c = v.get("color").and_then(color_of).map(|c| c.with_alpha(1.0).to_css_hex()).unwrap_or_default();
            let w = v.get("width").map(dim).unwrap_or_default();
            let s = v.get("style").and_then(Value::as_str).unwrap_or("custom");
            format!("{w} {s} {c}")
        }
        Some(TokenType::Shadow) => {
            let one = |s: &Value| {
                let c = s.get("color").and_then(color_of).map(|c| c.to_hex()).unwrap_or_default();
                let p: Vec<String> = ["offsetX", "offsetY", "blur", "spread"].iter().map(|k| s.get(*k).map(dim).unwrap_or_default()).collect();
                format!("{}{} {c}", if s.get("inset").and_then(Value::as_bool) == Some(true) { "inset " } else { "" }, p.join(" "))
            };
            match v {
                Value::Array(a) => a.iter().map(one).collect::<Vec<_>>().join(", "),
                _ => one(v),
            }
        }
        Some(TokenType::Gradient) => match v.as_array() {
            Some(a) => a
                .iter()
                .map(|s| {
                    let c = s.get("color").and_then(color_of).map(|c| c.with_alpha(1.0).to_css_hex()).unwrap_or_default();
                    let p = s.get("position").and_then(Value::as_f64).map(|p| format!(" {}%", crate::text::trim_num(p * 100.0))).unwrap_or_default();
                    format!("{c}{p}")
                })
                .collect::<Vec<_>>()
                .join(", "),
            None => compact(v),
        },
        Some(TokenType::Transition) => {
            let d = v.get("duration").map(dim).unwrap_or_default();
            let de = v.get("delay").map(dim).unwrap_or_default();
            format!("{d} · {de}")
        }
        Some(TokenType::StrokeStyle) => match v {
            Value::String(s) => s.clone(),
            _ => tr_dash(v),
        },
        None => compact(v),
    }
}

fn tr_dash(v: &Value) -> String {
    let dashes = v
        .get("dashArray")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(dimension_px).map(crate::text::trim_num).collect::<Vec<_>>().join(" "));
    format!("dashed {}", dashes.unwrap_or_default()).trim().to_string()
}

/// JSON on one line, cut to a readable length (at a char boundary).
fn compact(v: &Value) -> String {
    let s = match v {
        Value::String(s) => s.clone(),
        _ => v.to_string(),
    };
    match s.char_indices().nth(80) {
        Some((i, _)) => format!("{}…", s.get(..i).unwrap_or_default()),
        None => s,
    }
}

/// Check that `v` is a valid value of type `kind` (aliases anywhere are accepted; they're checked
/// when resolved). The reason when it isn't.
pub fn check_value(kind: TokenType, v: &Value) -> Result<(), String> {
    if alias_of(v).is_some() || v.get("$ref").is_some() {
        return Ok(());
    }
    let sub = |o: &Map<String, Value>, k: &str, t: TokenType| match o.get(k) {
        Some(x) => check_value(t, x).map_err(|e| format!("{k}: {e}")),
        None => Err(format!("{k} is missing")),
    };
    match kind {
        TokenType::Color => color_of(v).map(|_| ()).ok_or_else(|| "a colour: {colorSpace, components, alpha?, hex?}".into()),
        TokenType::Dimension => match v.as_object() {
            Some(o)
                if o.get("value").and_then(Value::as_f64).is_some_and(f64::is_finite)
                    && matches!(o.get("unit").and_then(Value::as_str), Some("px" | "rem")) =>
            {
                Ok(())
            }
            _ => Err("a dimension: {value, unit: px|rem}".into()),
        },
        TokenType::Duration => duration_ms(v).map(|_| ()).ok_or_else(|| "a duration: {value, unit: ms|s}".into()),
        TokenType::Number => v.as_f64().filter(|x| x.is_finite()).map(|_| ()).ok_or_else(|| "a number".into()),
        TokenType::FontFamily => match v {
            Value::String(s) if !s.is_empty() => Ok(()),
            Value::Array(a) if !a.is_empty() && a.iter().all(Value::is_string) => Ok(()),
            _ => Err("a font family name, or a list of names".into()),
        },
        TokenType::FontWeight => font_weight_of(v).map(|_| ()).ok_or_else(|| "a font weight: 1–1000 or a keyword (bold, regular…)".into()),
        TokenType::CubicBezier => match v.as_array().map(Vec::as_slice) {
            Some([a, b, c, d]) => {
                let n: Vec<Option<f64>> = [a, b, c, d].iter().map(|x| x.as_f64()).collect();
                let in01 = |x: Option<f64>| x.is_some_and(|x| (0.0..=1.0).contains(&x));
                let fin = |x: Option<f64>| x.is_some_and(f64::is_finite);
                match n.as_slice() {
                    [p1x, p1y, p2x, p2y] if in01(*p1x) && in01(*p2x) && fin(*p1y) && fin(*p2y) => Ok(()),
                    _ => Err("a cubic Bézier: [P1x, P1y, P2x, P2y], x within 0–1".into()),
                }
            }
            _ => Err("a cubic Bézier: [P1x, P1y, P2x, P2y]".into()),
        },
        TokenType::StrokeStyle => match v {
            Value::String(s) if ["solid", "dashed", "dotted", "double", "groove", "ridge", "outset", "inset"].contains(&s.as_str()) => Ok(()),
            Value::Object(o)
                if o.get("dashArray").is_some_and(Value::is_array)
                    && matches!(o.get("lineCap").and_then(Value::as_str), Some("round" | "butt" | "square")) =>
            {
                Ok(())
            }
            _ => Err("a stroke style: solid|dashed|dotted|double|groove|ridge|outset|inset, or {dashArray, lineCap}".into()),
        },
        TokenType::Border => {
            let o = v.as_object().ok_or("a border: {color, width, style}")?;
            sub(o, "color", TokenType::Color)?;
            sub(o, "width", TokenType::Dimension)?;
            sub(o, "style", TokenType::StrokeStyle)
        }
        TokenType::Transition => {
            let o = v.as_object().ok_or("a transition: {duration, delay, timingFunction}")?;
            sub(o, "duration", TokenType::Duration)?;
            sub(o, "delay", TokenType::Duration)?;
            sub(o, "timingFunction", TokenType::CubicBezier)
        }
        TokenType::Shadow => {
            let one = |s: &Value| -> Result<(), String> {
                if alias_of(s).is_some() {
                    return Ok(());
                }
                let o = s.as_object().ok_or("a shadow: {color, offsetX, offsetY, blur, spread, inset?}")?;
                sub(o, "color", TokenType::Color)?;
                for k in ["offsetX", "offsetY", "blur", "spread"] {
                    sub(o, k, TokenType::Dimension)?;
                }
                match o.get("inset") {
                    None | Some(Value::Bool(_)) => Ok(()),
                    _ => Err("inset: true or false".into()),
                }
            };
            match v {
                Value::Array(a) => a.iter().try_for_each(one),
                _ => one(v),
            }
        }
        TokenType::Gradient => {
            let a = v.as_array().ok_or("a gradient: [{color, position}]")?;
            a.iter().try_for_each(|s| {
                if alias_of(s).is_some() {
                    return Ok(());
                }
                let o = s.as_object().ok_or("a gradient stop: {color, position}")?;
                sub(o, "color", TokenType::Color)?;
                sub(o, "position", TokenType::Number)
            })
        }
        TokenType::Typography => {
            let o = v.as_object().ok_or("typography: {fontFamily, fontSize, fontWeight, letterSpacing, lineHeight}")?;
            sub(o, "fontFamily", TokenType::FontFamily)?;
            sub(o, "fontSize", TokenType::Dimension)?;
            sub(o, "fontWeight", TokenType::FontWeight)?;
            sub(o, "letterSpacing", TokenType::Dimension)?;
            sub(o, "lineHeight", TokenType::Number)
        }
    }
}

// ----- Resolver documents -----

/// Escape a name for a JSON pointer.
fn pointer_name(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}

/// This file's tokens as a DTCG resolver document (sets and modifier contexts inline).
pub fn to_resolver(t: &DesignTokens, name: &str) -> Value {
    resolver_with(t, name, &|_| None)
}

/// A resolver document whose sources are `$ref`s to separate token files: `file` names the file
/// of each set and context (relative to the resolver document); `None` keeps it inline.
pub fn resolver_with(t: &DesignTokens, name: &str, file: &dyn Fn(&Source) -> Option<String>) -> Value {
    let source = |src: Source, tree: &Value| match file(&src) {
        Some(f) => json!([{ "$ref": f }]),
        None => json!([tree_or_empty(tree)]),
    };
    let mut root = Map::new();
    let name = if t.name.is_empty() { name } else { &t.name };
    if !name.is_empty() {
        root.insert("name".into(), json!(name));
    }
    root.insert("version".into(), json!(RESOLVER_VERSION));
    if !t.description.is_empty() {
        root.insert("description".into(), json!(t.description));
    }
    let mut sets = Map::new();
    for s in &t.sets {
        let mut o = Map::new();
        if !s.description.is_empty() {
            o.insert("description".into(), json!(s.description));
        }
        o.insert("sources".into(), source(Source::Set(s.name.clone()), &s.tokens));
        sets.insert(s.name.clone(), Value::Object(o));
    }
    if !sets.is_empty() {
        root.insert("sets".into(), Value::Object(sets));
    }
    let mut mods = Map::new();
    for m in &t.modifiers {
        let mut o = Map::new();
        if !m.description.is_empty() {
            o.insert("description".into(), json!(m.description));
        }
        let ctx: Map<String, Value> = m
            .contexts
            .iter()
            .map(|c| (c.name.clone(), source(Source::Context { modifier: m.name.clone(), context: c.name.clone() }, &c.tokens)))
            .collect();
        o.insert("contexts".into(), Value::Object(ctx));
        if let Some(d) = m.default.as_deref().and_then(|d| m.context(d)) {
            o.insert("default".into(), json!(d.name));
        }
        mods.insert(m.name.clone(), Value::Object(o));
    }
    if !mods.is_empty() {
        root.insert("modifiers".into(), Value::Object(mods));
    }
    let order: Vec<Value> = t
        .full_order()
        .iter()
        .map(|i| match i {
            OrderItem::Set(s) => json!({"$ref": format!("#/sets/{}", pointer_name(s))}),
            OrderItem::Modifier(m) => json!({"$ref": format!("#/modifiers/{}", pointer_name(m))}),
        })
        .collect();
    root.insert("resolutionOrder".into(), Value::Array(order));
    Value::Object(root)
}

fn tree_or_empty(v: &Value) -> Value {
    if v.is_object() { v.clone() } else { Value::Object(Map::new()) }
}

/// Whether a JSON document is a resolver document (rather than a token file).
pub fn is_resolver(v: &Value) -> bool {
    v.as_object()
        .is_some_and(|o| o.contains_key("resolutionOrder") || (o.contains_key("version") && (o.contains_key("sets") || o.contains_key("modifiers"))))
}

/// Read a resolver document. `load` reads an external file a `$ref` names (relative to the
/// document); `None` when it can't, which becomes a warning. Returns the tokens and warnings.
pub fn from_resolver(v: &Value, load: &dyn Fn(&str) -> Option<Value>) -> Result<(DesignTokens, Vec<String>), String> {
    let o = v.as_object().ok_or("a resolver document is a JSON object")?;
    let mut warn = Vec::new();
    match o.get("version").and_then(Value::as_str) {
        Some(RESOLVER_VERSION) => {}
        Some(other) => warn.push(format!("version {other}: read as {RESOLVER_VERSION}")),
        None => warn.push(format!("no version: read as {RESOLVER_VERSION}")),
    }
    let mut out = DesignTokens {
        name: o.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
        description: o.get("description").and_then(Value::as_str).unwrap_or_default().to_string(),
        ..Default::default()
    };
    let sets = o.get("sets").and_then(Value::as_object);
    // A set's tokens: its sources merged.
    let set_tree = |sources: Option<&Value>, warn: &mut Vec<String>, what: &str| -> Value {
        let mut tree = Value::Object(Map::new());
        for src in sources.and_then(Value::as_array).into_iter().flatten().take(1000) {
            if let Some(t) = source_tree(src, v, load, warn, what, 0) {
                merge_into(&mut tree, &t, 0);
            }
        }
        tree
    };
    if let Some(sets) = sets {
        for (name, s) in sets.iter().take(1000) {
            if !valid_set_name(name) {
                warn.push(format!("set {name}: not a usable name, skipped"));
                continue;
            }
            out.sets.push(TokenSet {
                name: name.clone(),
                description: s.get("description").and_then(Value::as_str).unwrap_or_default().to_string(),
                tokens: set_tree(s.get("sources"), &mut warn, &format!("set {name}")),
            });
        }
    }
    let read_modifier = |name: &str, m: &Value, warn: &mut Vec<String>| -> Option<TokenModifier> {
        let ctx = m.get("contexts").or_else(|| m.get("context")).and_then(Value::as_object);
        let Some(ctx) = ctx.filter(|c| !c.is_empty()) else {
            warn.push(format!("modifier {name} has no contexts, skipped"));
            return None;
        };
        if ctx.len() == 1 {
            warn.push(format!("modifier {name} has a single context"));
        }
        let contexts = ctx
            .iter()
            .take(1000)
            .map(|(c, sources)| TokenContext { name: c.clone(), tokens: set_tree(Some(sources), warn, &format!("modifier {name} context {c}")) })
            .collect::<Vec<_>>();
        let default = m.get("default").and_then(Value::as_str).map(str::to_string);
        if let Some(d) = &default
            && !contexts.iter().any(|c| c.name == *d)
        {
            warn.push(format!("modifier {name}: default {d} is not one of its contexts"));
        }
        Some(TokenModifier {
            name: name.to_string(),
            description: m.get("description").and_then(Value::as_str).unwrap_or_default().to_string(),
            contexts,
            default: default.filter(|d| ctx.contains_key(d)),
        })
    };
    if let Some(mods) = o.get("modifiers").and_then(Value::as_object) {
        for (name, m) in mods.iter().take(1000) {
            if !valid_set_name(name) {
                warn.push(format!("modifier {name}: not a usable name, skipped"));
                continue;
            }
            if let Some(m) = read_modifier(name, m, &mut warn) {
                out.modifiers.push(m);
            }
        }
    }
    for item in o.get("resolutionOrder").and_then(Value::as_array).into_iter().flatten().take(1000) {
        if let Some(r) = item.get("$ref").and_then(Value::as_str) {
            if let Some(s) = r.strip_prefix("#/sets/") {
                let s = s.replace("~1", "/").replace("~0", "~");
                if out.set(&s).is_some() {
                    out.order.push(OrderItem::Set(s));
                } else {
                    warn.push(format!("resolutionOrder: {r} names no set"));
                }
            } else if let Some(m) = r.strip_prefix("#/modifiers/") {
                let m = m.replace("~1", "/").replace("~0", "~");
                if out.modifier(&m).is_some() {
                    out.order.push(OrderItem::Modifier(m));
                } else {
                    warn.push(format!("resolutionOrder: {r} names no modifier"));
                }
            } else {
                // An external file straight in the order: a set of its own.
                let name = unique_name(&out, r.rsplit('/').next().unwrap_or("set").trim_end_matches(".json").trim_end_matches(".tokens"));
                let tokens = set_tree(Some(&json!([item])), &mut warn, &format!("resolutionOrder {r}"));
                out.sets.push(TokenSet { name: name.clone(), description: String::new(), tokens });
                out.order.push(OrderItem::Set(name));
            }
            continue;
        }
        // Inline sets and modifiers.
        let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
        if !valid_set_name(name) {
            warn.push("resolutionOrder: an inline entry without a usable name, skipped".into());
            continue;
        }
        let name = unique_name(&out, name);
        match item.get("type").and_then(Value::as_str) {
            Some("modifier") => {
                if let Some(m) = read_modifier(&name, item, &mut warn) {
                    out.modifiers.push(m);
                    out.order.push(OrderItem::Modifier(name));
                }
            }
            Some("set") | None => {
                let tokens = set_tree(item.get("sources"), &mut warn, &format!("set {name}"));
                out.sets.push(TokenSet {
                    name: name.clone(),
                    description: item.get("description").and_then(Value::as_str).unwrap_or_default().to_string(),
                    tokens,
                });
                out.order.push(OrderItem::Set(name));
            }
            Some(t) => warn.push(format!("resolutionOrder: {name} has unknown type {t}, skipped")),
        }
    }
    Ok((out, warn))
}

/// Names of sets and modifiers: not empty, not too long, no control characters.
pub fn valid_set_name(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= 256 && !name.chars().any(char::is_control)
}

/// `base`, or `base 2`, `base 3`… so no set or modifier has it yet.
pub fn unique_name(t: &DesignTokens, base: &str) -> String {
    let base = if valid_set_name(base) { base.trim() } else { "Set" };
    let taken = |n: &str| t.sets.iter().any(|s| s.name == n) || t.modifiers.iter().any(|m| m.name == n);
    (1..10_000).map(|k| if k == 1 { base.to_string() } else { format!("{base} {k}") }).find(|n| !taken(n)).unwrap_or_else(|| base.to_string())
}

/// The token tree a source (an inline tree or a `$ref`) brings.
fn source_tree(src: &Value, doc: &Value, load: &dyn Fn(&str) -> Option<Value>, warn: &mut Vec<String>, what: &str, depth: usize) -> Option<Value> {
    if depth > 8 {
        warn.push(format!("{what}: references nest too deep"));
        return None;
    }
    let o = src.as_object()?;
    let Some(r) = o.get("$ref").and_then(Value::as_str) else { return Some(src.clone()) };
    let target = if let Some(ptr) = r.strip_prefix('#') {
        if ptr.starts_with("/modifiers") || ptr.starts_with("/resolutionOrder") {
            warn.push(format!("{what}: {r} can't be referenced here"));
            return None;
        }
        let path = reference_path(r)?;
        let t = at_path(doc, &path).cloned();
        // A whole set: its sources merged.
        match t {
            Some(set) if set.get("sources").is_some() => {
                let mut tree = Value::Object(Map::new());
                for s in set.get("sources").and_then(Value::as_array).into_iter().flatten() {
                    if let Some(t) = source_tree(s, doc, load, warn, what, depth + 1) {
                        merge_into(&mut tree, &t, 0);
                    }
                }
                Some(tree)
            }
            other => other,
        }
    } else {
        let (file, ptr) = r.split_once('#').unwrap_or((r, ""));
        let loaded = load(file);
        if loaded.is_none() {
            warn.push(format!("{what}: couldn't read {file}"));
        }
        loaded.and_then(|v| if ptr.is_empty() { Some(v) } else { at_path(&v, &reference_path(&format!("#{ptr}"))?).cloned() })
    };
    let mut t = target?;
    // Sibling keys override what the reference brings (shallowly).
    if let Some(to) = t.as_object_mut() {
        for (k, v) in o {
            if k != "$ref" {
                to.insert(k.clone(), v.clone());
            }
        }
    }
    Some(t)
}
