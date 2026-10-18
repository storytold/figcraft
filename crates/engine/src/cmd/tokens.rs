//! Design tokens (DTCG 2025.10): sets and modifiers (themes, brands…) with their contexts, tokens
//! of every format type edited in them, the contexts a file and its frames use, token bindings on
//! layers, and import and export as DTCG token files or resolver documents.
//!
//! A file can have variables and tokens; Preferences ▸ Design tokens picks which one the interface
//! shows (`pref.designTokens`). Both keep working on the canvas.

use std::collections::{BTreeMap, BTreeSet};

use figcraft_doc::color::{Color, ColorStop, Gradient, Paint, PaintKind};
use figcraft_doc::tokens::{self as dt, OrderItem, Resolved, TokenType};
use figcraft_doc::*;
use serde_json::{Map, Value, json};

use super::lists::{BoundList, list_binding};
use super::{CommandSpec, ok};
use crate::params::*;
use crate::{EngineError, Result, Session, bad, spec};

/// Largest token file read.
const MAX_FILE: u64 = 32 << 20;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "token.list",
            "List design tokens",
            "{input? ({modifier: context}; default the file's contexts), node? (resolve in a layer's contexts)}  → {sets, modifiers, order, active, input, tokens: [{path, type, value, display, alias, description, deprecated, error}], errors}",
            false,
            list
        ),
        spec!("token.createSet", "Create token set", "{name?, description?}  → {name}", true, create_set),
        spec!("token.renameSet", "Rename token set", "{set, name?, description?}", true, rename_set),
        spec!("token.removeSet", "Delete token set", "{set}", true, remove_set),
        spec!(
            "token.createModifier",
            "Create token modifier",
            "{name?, contexts? ([names]; default two), description?}  a modifier is a theme, brand or other choice; one context of it applies at a time  → {name}",
            true,
            create_modifier
        ),
        spec!("token.renameModifier", "Rename token modifier", "{modifier, name?, description?}", true, rename_modifier),
        spec!("token.removeModifier", "Delete token modifier", "{modifier}", true, remove_modifier),
        spec!(
            "token.addContext",
            "Add modifier context",
            "{modifier, name?, copy? (context whose tokens it starts with; default the modifier's default; null: empty)}  → {name}",
            true,
            add_context
        ),
        spec!("token.renameContext", "Rename modifier context", "{modifier, context, name}", true, rename_context),
        spec!("token.removeContext", "Delete modifier context", "{modifier, context} (a modifier keeps at least one)", true, remove_context),
        spec!("token.setDefaultContext", "Set default context", "{modifier, context}", true, set_default_context),
        spec!("token.moveOrder", "Move in resolution order", "{set | modifier, to (index; later entries override earlier ones)}", true, move_order),
        spec!(
            "token.set",
            "Set design token",
            "{set | modifier+context, path (dot-separated: color.brand.primary), type? (color|dimension|number|fontFamily|fontWeight|duration|cubicBezier|strokeStyle|border|shadow|gradient|typography|transition; default the group's), value? (the DTCG value, an alias \"{other.token}\", a colour as hex, a dimension as 16 or \"1rem\"), description?, deprecated?}  creates or updates the token",
            true,
            set_token
        ),
        spec!("token.remove", "Delete design token", "{set | modifier+context, path (a token or group)}", true, remove_token),
        spec!(
            "token.rename",
            "Rename design token",
            "{set | modifier+context, path (a token or group), to (new path), everywhere? (default true: in every set and context that has it)}  aliases and layer bindings follow  → {renamed, aliases}",
            true,
            rename_token
        ),
        spec!(
            "token.setContext",
            "Set token context",
            "{modifier, context (null: the default, or for layers the inherited one), ids? (frames; none: the whole file)}",
            true,
            set_context
        ),
        spec!(
            "token.bind",
            "Bind design token",
            "{ids?, field (fills/<i>/color|strokes/<i>/color|fills/<i>/gradient|effects/<i>/<color|radius|offsetX|offsetY|spread>|width|height|cornerRadius|gap|padding…|opacity (number 0–1)|fontSize|fontFamily|fontWeight|lineHeight|letterSpacing|typography|shadow|border|strokeStyle), token (path; null unbinds)}",
            true,
            bind
        ),
        spec!(
            "token.import",
            "Import design tokens",
            "{tokens (a zip of token files (with a resolver document, or sets/<set>.json and modifiers/<modifier>/<context>.json), a DTCG token file or resolver document: an object, JSON text or a path) | data (the same as base64 bytes), set? (for a token file: the set it goes into; default its file name or \"tokens\"), modifier?+context? (put a token file in a context instead), replace? (a resolver document replaces this file's tokens instead of joining them)}  → {sets, modifiers, tokens, warnings}",
            true,
            import
        ),
        spec!(
            "token.export",
            "Export design tokens",
            "{format? (zip: a resolver document (tokens.resolver.json) pointing at a token file per set (sets/<set>.tokens.json) and per modifier context (modifiers/<modifier>/<context>.tokens.json), base64 in `data` unless written | resolver (default): one DTCG resolver document with every set and modifier inline | tokens: one DTCG token file), set? | modifier+context? (tokens: that source only), input? ({modifier: context}, tokens: the merged result for these contexts; default the file's), resolveAliases? (tokens: values instead of aliases), path? (write it there)}  → {text}, or {path} when written",
            false,
            export
        ),
        spec!("pref.designTokens", "Use design tokens", "{on? (true: the interface shows Design tokens instead of Variables)}", false, |s, p| {
            s.prefs.design_tokens = boolean(p, "on").unwrap_or(!s.prefs.design_tokens);
            Ok(json!({"on": s.prefs.design_tokens}))
        }),
    ]
}

// ----- Helpers -----

/// The token tree a command names: `set`, or `modifier` and `context`.
fn source_of(cmd: &str, doc: &Document, p: &Value) -> Result<TokenSource> {
    if let Some(s) = string(p, "set") {
        let set = doc.tokens.set(&s).ok_or_else(|| bad(cmd, format!("no token set {s}")))?;
        return Ok(TokenSource::Set(set.name.clone()));
    }
    let Some(m) = string(p, "modifier") else { return Err(bad(cmd, "`set`, or `modifier` and `context`, required")) };
    let m = doc.tokens.modifier(&m).ok_or_else(|| bad(cmd, format!("no modifier {m}")))?;
    let c = req_string(cmd, p, "context")?;
    let c = m.context(&c).ok_or_else(|| bad(cmd, format!("modifier {} has no context {c}", m.name)))?;
    Ok(TokenSource::Context { modifier: m.name.clone(), context: c.name.clone() })
}

fn name_param(cmd: &str, p: &Value, key: &str) -> Result<Option<String>> {
    match string(p, key) {
        Some(n) if dt::valid_set_name(&n) => Ok(Some(n.trim().to_string())),
        Some(_) => Err(bad(cmd, format!("{key}: a name (not blank, at most 256 bytes)"))),
        None => Ok(None),
    }
}

fn taken(t: &DesignTokens, name: &str) -> bool {
    t.sets.iter().any(|s| s.name == name) || t.modifiers.iter().any(|m| m.name == name)
}

/// A resolver input from params (`input`: {modifier: context}), checked strictly.
fn input_param(cmd: &str, doc: &Document, p: &Value) -> Result<Option<BTreeMap<String, String>>> {
    match get(p, "input") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(o)) => doc.tokens.validate_input(o).map(Some).map_err(|e| bad(cmd, e)),
        Some(_) => Err(bad(cmd, "input: {modifier: context}")),
    }
}

fn token_json(t: &ResolvedToken) -> Value {
    json!({
        "path": t.path,
        "type": t.kind.map(TokenType::id),
        "value": t.value,
        "raw": t.raw,
        "display": dt::display(t.kind, &t.value),
        "alias": t.alias,
        "description": t.description,
        "deprecated": t.deprecated,
        "error": t.error,
    })
}

// ----- Structure -----

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "token.list";
    let doc = s.doc()?;
    let input = match (input_param(cmd, doc, p)?, opt_id(cmd, p, "node")?) {
        (Some(i), _) => i,
        (None, Some(n)) => {
            doc.get(n)?;
            doc.token_input_for(n)
        }
        (None, None) => doc.tokens.input_for(&BTreeMap::new()),
    };
    let r = doc.tokens.resolve(&input);
    let t = &doc.tokens;
    Ok(json!({
        "sets": t.sets.iter().map(|s| json!({"name": s.name, "description": s.description, "tokens": dt::flatten(&s.tokens).len()})).collect::<Vec<_>>(),
        "modifiers": t.modifiers.iter().map(|m| json!({
            "name": m.name, "description": m.description, "default": m.default,
            "contexts": m.contexts.iter().map(|c| json!({"name": c.name, "tokens": dt::flatten(&c.tokens).len()})).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "order": t.full_order(),
        "active": t.input_for(&BTreeMap::new()),
        "input": input,
        "tokens": r.tokens.values().map(token_json).collect::<Vec<_>>(),
        "errors": r.errors,
    }))
}

fn create_set(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "token.createSet";
    let name = name_param(cmd, p, "name")?;
    let doc = s.doc_mut()?;
    let name = match name {
        Some(n) if taken(&doc.tokens, &n) => return Err(bad(cmd, format!("{n} is taken"))),
        Some(n) => n,
        None => dt::unique_name(&doc.tokens, "Set"),
    };
    let description = string(p, "description").unwrap_or_default();
    doc.tokens.sets.push(TokenSet { name: name.clone(), description, tokens: json!({}) });
    doc.tokens.order = doc.tokens.full_order();
    Ok(json!({"name": name}))
}

fn rename_set(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "token.renameSet";
    let set = req_string(cmd, p, "set")?;
    let name = name_param(cmd, p, "name")?;
    let description = string(p, "description");
    let doc = s.doc_mut()?;
    let old = doc.tokens.set(&set).map(|x| x.name.clone()).ok_or_else(|| bad(cmd, format!("no token set {set}")))?;
    if let Some(n) = &name
        && *n != old
        && taken(&doc.tokens, n)
    {
        return Err(bad(cmd, format!("{n} is taken")));
    }
    let t = &mut doc.tokens;
    if let Some(x) = t.sets.iter_mut().find(|x| x.name == old) {
        if let Some(n) = &name {
            x.name = n.clone();
        }
        if let Some(d) = description {
            x.description = d;
        }
    }
    if let Some(n) = name {
        for item in &mut t.order {
            if *item == OrderItem::Set(old.clone()) {
                *item = OrderItem::Set(n.clone());
            }
        }
    }
    ok()
}

fn remove_set(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "token.removeSet";
    let set = req_string(cmd, p, "set")?;
    let doc = s.doc_mut()?;
    let name = doc.tokens.set(&set).map(|x| x.name.clone()).ok_or_else(|| bad(cmd, format!("no token set {set}")))?;
    doc.tokens.sets.retain(|x| x.name != name);
    doc.tokens.order.retain(|i| *i != OrderItem::Set(name.clone()));
    ok()
}

fn create_modifier(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "token.createModifier";
    let name = name_param(cmd, p, "name")?;
    let contexts: Vec<String> = match get(p, "contexts") {
        None | Some(Value::Null) => vec!["Context 1".into(), "Context 2".into()],
        Some(Value::Array(a)) => {
            let names: Vec<String> = a.iter().filter_map(Value::as_str).map(|x| x.trim().to_string()).collect();
            if names.len() != a.len() || names.is_empty() || names.iter().any(|n| !dt::valid_set_name(n)) {
                return Err(bad(cmd, "contexts: [names]"));
            }
            let unique: BTreeSet<&String> = names.iter().collect();
            if unique.len() != names.len() {
                return Err(bad(cmd, "contexts: names must differ"));
            }
            names
        }
        Some(_) => return Err(bad(cmd, "contexts: [names]")),
    };
    let doc = s.doc_mut()?;
    let name = match name {
        Some(n) if taken(&doc.tokens, &n) => return Err(bad(cmd, format!("{n} is taken"))),
        Some(n) => n,
        None => dt::unique_name(&doc.tokens, "Theme"),
    };
    let default = contexts.first().cloned();
    doc.tokens.modifiers.push(TokenModifier {
        name: name.clone(),
        description: string(p, "description").unwrap_or_default(),
        contexts: contexts.into_iter().map(|c| TokenContext { name: c, tokens: json!({}) }).collect(),
        default,
    });
    doc.tokens.order = doc.tokens.full_order();
    Ok(json!({"name": name}))
}

/// The modifier `p.modifier` names (its name).
fn modifier_name(cmd: &str, doc: &Document, p: &Value) -> Result<String> {
    let m = req_string(cmd, p, "modifier")?;
    doc.tokens.modifier(&m).map(|x| x.name.clone()).ok_or_else(|| bad(cmd, format!("no modifier {m}")))
}

fn modifier_mut<'a>(doc: &'a mut Document, name: &str) -> Option<&'a mut TokenModifier> {
    doc.tokens.modifiers.iter_mut().find(|m| m.name == name)
}

fn rename_modifier(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "token.renameModifier";
    let name = name_param(cmd, p, "name")?;
    let description = string(p, "description");
    let doc = s.doc_mut()?;
    let old = modifier_name(cmd, doc, p)?;
    if let Some(n) = &name
        && *n != old
        && taken(&doc.tokens, n)
    {
        return Err(bad(cmd, format!("{n} is taken")));
    }
    if let Some(m) = modifier_mut(doc, &old) {
        if let Some(n) = &name {
            m.name = n.clone();
        }
        if let Some(d) = description {
            m.description = d;
        }
    }
    let Some(n) = name.filter(|n| *n != old) else { return ok() };
    for item in &mut doc.tokens.order {
        if *item == OrderItem::Modifier(old.clone()) {
            *item = OrderItem::Modifier(n.clone());
        }
    }
    if let Some(c) = doc.tokens.active.remove(&old) {
        doc.tokens.active.insert(n.clone(), c);
    }
    // Layers that chose a context of it keep it.
    let ids: Vec<NodeId> = doc.nodes().filter(|x| x.token_contexts.contains_key(&old)).map(|x| x.id).collect();
    for id in ids {
        let tc = &mut doc.get_mut(id)?.token_contexts;
        if let Some(c) = tc.remove(&old) {
            tc.insert(n.clone(), c);
        }
    }
    ok()
}

fn remove_modifier(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "token.removeModifier";
    let doc = s.doc_mut()?;
    let name = modifier_name(cmd, doc, p)?;
    doc.tokens.modifiers.retain(|m| m.name != name);
    doc.tokens.order.retain(|i| *i != OrderItem::Modifier(name.clone()));
    doc.tokens.active.remove(&name);
    let ids: Vec<NodeId> = doc.nodes().filter(|x| x.token_contexts.contains_key(&name)).map(|x| x.id).collect();
    for id in ids {
        doc.get_mut(id)?.token_contexts.remove(&name);
    }
    ok()
}

fn add_context(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "token.addContext";
    let name = name_param(cmd, p, "name")?;
    let doc = s.doc_mut()?;
    let m = modifier_name(cmd, doc, p)?;
    let Some(modi) = doc.tokens.modifier(&m) else { return Err(bad(cmd, format!("no modifier {m}"))) };
    let copy = match get(p, "copy") {
        Some(Value::Null) => None,
        Some(Value::String(c)) => Some(modi.context(c).ok_or_else(|| bad(cmd, format!("modifier {m} has no context {c}")))?.tokens.clone()),
        Some(_) => return Err(bad(cmd, "copy: a context name, or null")),
        None => modi.fallback().map(|c| c.tokens.clone()),
    };
    let has = |n: &str| modi.contexts.iter().any(|c| c.name == n);
    let name = match name {
        Some(n) if has(&n) => return Err(bad(cmd, format!("{m} already has {n}"))),
        Some(n) => n,
        None => (1..10_000).map(|k| format!("Context {}", modi.contexts.len() + k)).find(|n| !has(n)).unwrap_or_else(|| "Context".into()),
    };
    if modi.contexts.len() >= 1000 {
        return Err(bad(cmd, "a modifier has at most 1000 contexts"));
    }
    if let Some(modi) = modifier_mut(doc, &m) {
        modi.contexts.push(TokenContext { name: name.clone(), tokens: copy.unwrap_or_else(|| json!({})) });
    }
    Ok(json!({"name": name}))
}

/// The context `p.context` of modifier `m` (its name).
fn context_name(cmd: &str, doc: &Document, m: &str, p: &Value) -> Result<String> {
    let c = req_string(cmd, p, "context")?;
    doc.tokens.modifier(m).and_then(|x| x.context(&c)).map(|x| x.name.clone()).ok_or_else(|| bad(cmd, format!("modifier {m} has no context {c}")))
}

fn rename_context(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "token.renameContext";
    let name = name_param(cmd, p, "name")?.ok_or_else(|| bad(cmd, "name required"))?;
    let doc = s.doc_mut()?;
    let m = modifier_name(cmd, doc, p)?;
    let old = context_name(cmd, doc, &m, p)?;
    if old == name {
        return ok();
    }
    if doc.tokens.modifier(&m).is_some_and(|x| x.contexts.iter().any(|c| c.name == name)) {
        return Err(bad(cmd, format!("{m} already has {name}")));
    }
    if let Some(modi) = modifier_mut(doc, &m) {
        if let Some(c) = modi.contexts.iter_mut().find(|c| c.name == old) {
            c.name = name.clone();
        }
        if modi.default.as_deref() == Some(old.as_str()) {
            modi.default = Some(name.clone());
        }
    }
    if doc.tokens.active.get(&m) == Some(&old) {
        doc.tokens.active.insert(m.clone(), name.clone());
    }
    let ids: Vec<NodeId> = doc.nodes().filter(|x| x.token_contexts.get(&m) == Some(&old)).map(|x| x.id).collect();
    for id in ids {
        doc.get_mut(id)?.token_contexts.insert(m.clone(), name.clone());
    }
    ok()
}

fn remove_context(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "token.removeContext";
    let doc = s.doc_mut()?;
    let m = modifier_name(cmd, doc, p)?;
    let c = context_name(cmd, doc, &m, p)?;
    if doc.tokens.modifier(&m).is_some_and(|x| x.contexts.len() <= 1) {
        return Err(bad(cmd, "a modifier keeps at least one context"));
    }
    if let Some(modi) = modifier_mut(doc, &m) {
        modi.contexts.retain(|x| x.name != c);
        if modi.default.as_deref() == Some(c.as_str()) {
            modi.default = modi.contexts.first().map(|x| x.name.clone());
        }
    }
    if doc.tokens.active.get(&m) == Some(&c) {
        doc.tokens.active.remove(&m);
    }
    let ids: Vec<NodeId> = doc.nodes().filter(|x| x.token_contexts.get(&m) == Some(&c)).map(|x| x.id).collect();
    for id in ids {
        doc.get_mut(id)?.token_contexts.remove(&m);
    }
    ok()
}

fn set_default_context(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "token.setDefaultContext";
    let doc = s.doc_mut()?;
    let m = modifier_name(cmd, doc, p)?;
    let c = context_name(cmd, doc, &m, p)?;
    if let Some(modi) = modifier_mut(doc, &m) {
        modi.default = Some(c);
    }
    ok()
}

fn move_order(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "token.moveOrder";
    let to = req_index(cmd, p, "to")?;
    let doc = s.doc_mut()?;
    let item = if let Some(set) = string(p, "set") {
        OrderItem::Set(doc.tokens.set(&set).map(|x| x.name.clone()).ok_or_else(|| bad(cmd, format!("no token set {set}")))?)
    } else {
        OrderItem::Modifier(modifier_name(cmd, doc, p)?)
    };
    let mut order = doc.tokens.full_order();
    order.retain(|i| *i != item);
    order.insert(to.min(order.len()), item);
    doc.tokens.order = order;
    ok()
}

// ----- Tokens -----

/// A token value from params: DTCG JSON, with conveniences for hand-written values (a colour as hex,
/// a dimension as a number of pixels or "1rem", a duration as milliseconds).
fn value_param(kind: Option<TokenType>, v: &Value) -> Value {
    if dt::alias_of(v).is_some() {
        return v.clone();
    }
    match (kind, v) {
        (Some(TokenType::Color), Value::String(_)) => dt::color_of(v).map(dt::color_value).unwrap_or_else(|| v.clone()),
        (Some(TokenType::Dimension), Value::Number(_) | Value::String(_)) => match v.as_str().map(str::trim) {
            Some(t) if t.ends_with("rem") => t
                .trim_end_matches("rem")
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|x| x.is_finite())
                .map(|x| json!({"value": x, "unit": "rem"}))
                .unwrap_or_else(|| v.clone()),
            _ => dt::dimension_px(v).map(dt::px_value).unwrap_or_else(|| v.clone()),
        },
        (Some(TokenType::Duration), Value::Number(n)) => {
            n.as_f64().filter(|x| x.is_finite()).map(|x| json!({"value": x, "unit": "ms"})).unwrap_or_else(|| v.clone())
        }
        (Some(TokenType::Number | TokenType::FontWeight), Value::String(s)) => {
            s.trim().parse::<f64>().ok().filter(|x| x.is_finite()).map(|x| json!(x)).unwrap_or_else(|| v.clone())
        }
        _ => v.clone(),
    }
}

/// The type a token at `path` of `tree` gets from its groups.
fn group_type(tree: &Value, path: &str) -> Option<TokenType> {
    let parts = dt::split_path(path)?;
    let mut cur = tree;
    let mut kind = None;
    for p in parts.iter().take(parts.len().saturating_sub(1)) {
        if let Some(t) = cur.get("$type").and_then(Value::as_str).and_then(TokenType::parse) {
            kind = Some(t);
        }
        cur = cur.get(p)?;
    }
    cur.get("$type").and_then(Value::as_str).and_then(TokenType::parse).or(kind)
}

fn set_token(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "token.set";
    let path = req_string(cmd, p, "path")?;
    let kind = match string(p, "type") {
        Some(t) => Some(TokenType::parse(&t).ok_or_else(|| bad(cmd, format!("type {t}: not a DTCG type")))?),
        None => None,
    };
    let doc = s.doc_mut()?;
    let src = source_of(cmd, doc, p)?;
    let tree = doc.tokens.tree(&src).cloned().unwrap_or_else(|| json!({}));
    if dt::split_path(&path).is_none() {
        return Err(bad(cmd, format!("{path}: names are dot-separated, without {{, }} or a leading $")));
    }
    let old = dt::token_at(&tree, &path).cloned();
    // A token with a type of its own keeps it; a new one names one unless its group has one.
    let own = old.as_ref().and_then(|o| o.get("$type")).and_then(Value::as_str).and_then(TokenType::parse);
    let inherited = group_type(&tree, &path);
    let new_kind = kind.or(own);
    let effective = new_kind.or(inherited);
    let value = match get(p, "value") {
        Some(v) => value_param(effective, v),
        None => match &old {
            Some(o) if kind.is_none() || kind == own.or(inherited) => o.get("$value").cloned().unwrap_or(Value::Null),
            // A new type starts at that type's default.
            _ => effective.ok_or_else(|| bad(cmd, "type or value required for a new token"))?.default_value(),
        },
    };
    let Some(k) = effective else { return Err(bad(cmd, format!("{path}: a type is required (no group type)"))) };
    dt::check_value(k, &value).map_err(|e| bad(cmd, format!("{path}: {e}")))?;
    let mut tok = old.as_ref().and_then(Value::as_object).cloned().unwrap_or_default();
    // The type is written on the token unless the group gives it.
    match new_kind {
        Some(t) if Some(t) != inherited || own.is_some() => {
            tok.insert("$type".into(), json!(t.id()));
        }
        _ => {
            tok.remove("$type");
        }
    }
    tok.insert("$value".into(), value);
    if let Some(d) = string(p, "description") {
        if d.is_empty() {
            tok.remove("$description");
        } else {
            tok.insert("$description".into(), json!(d));
        }
    }
    match get(p, "deprecated") {
        Some(Value::Bool(false)) | Some(Value::Null) => {
            tok.remove("$deprecated");
        }
        Some(v @ (Value::Bool(true) | Value::String(_))) => {
            tok.insert("$deprecated".into(), v.clone());
        }
        _ => {}
    }
    // Keep `$type` and `$value` first, the way token files are written.
    let mut ordered = Map::new();
    for k in ["$type", "$value", "$description", "$deprecated"] {
        if let Some(v) = tok.remove(k) {
            ordered.insert(k.into(), v);
        }
    }
    ordered.extend(tok);
    let t = doc.tokens.tree_mut(&src).ok_or_else(|| bad(cmd, "no such source"))?;
    if dt::flatten(t).len() >= dt::MAX_TOKENS && old.is_none() {
        return Err(bad(cmd, format!("a set holds at most {} tokens", dt::MAX_TOKENS)));
    }
    dt::put_token(t, &path, Value::Object(ordered)).map_err(|e| bad(cmd, e))?;
    Ok(json!({"path": path, "type": k.id()}))
}

fn remove_token(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "token.remove";
    let path = req_string(cmd, p, "path")?;
    let doc = s.doc_mut()?;
    let src = source_of(cmd, doc, p)?;
    let t = doc.tokens.tree_mut(&src).ok_or_else(|| bad(cmd, "no such source"))?;
    dt::remove_at(t, &path).ok_or_else(|| bad(cmd, format!("no token or group {path}")))?;
    ok()
}

fn rename_token(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "token.rename";
    let path = req_string(cmd, p, "path")?;
    let to = req_string(cmd, p, "to")?;
    if dt::split_path(&to).is_none() {
        return Err(bad(cmd, format!("{to}: names are dot-separated, without {{, }} or a leading $")));
    }
    if to == path {
        return Ok(json!({"renamed": 0, "aliases": 0}));
    }
    if to.starts_with(&format!("{path}.")) {
        return Err(bad(cmd, "a group can't move into itself"));
    }
    let everywhere = boolean(p, "everywhere").unwrap_or(true);
    let doc = s.doc_mut()?;
    let src = source_of(cmd, doc, p)?;
    let parts = dt::split_path(&path).ok_or_else(|| bad(cmd, format!("no token or group {path}")))?;
    if doc.tokens.tree(&src).and_then(|t| dt::at_path(t, &parts)).is_none() {
        return Err(bad(cmd, format!("no token or group {path}")));
    }
    let srcs = if everywhere { doc.tokens.sources() } else { vec![src] };
    let to_parts = dt::split_path(&to).unwrap_or_default();
    // Check first: nothing moves when any tree has the new path already.
    for x in &srcs {
        if let Some(t) = doc.tokens.tree(x)
            && dt::at_path(t, &parts).is_some()
            && dt::at_path(t, &to_parts).is_some()
        {
            return Err(bad(cmd, format!("{to} is taken")));
        }
    }
    let mut renamed = 0;
    for x in &srcs {
        let Some(t) = doc.tokens.tree_mut(x) else { continue };
        if let Some(v) = dt::remove_at(t, &path) {
            // A group moves as a whole: put it back at its new place.
            put_at(t, &to_parts, v).map_err(|e| bad(cmd, e))?;
            renamed += 1;
        }
    }
    // Aliases everywhere follow.
    let mut aliases = 0;
    for x in doc.tokens.sources() {
        if let Some(t) = doc.tokens.tree_mut(&x) {
            aliases += dt::rename_aliases(t, &path, &to);
        }
    }
    // And layer bindings.
    let ids: Vec<NodeId> = doc.nodes().filter(|n| n.token_bound.values().any(|v| moved(v, &path).is_some())).map(|n| n.id).collect();
    for id in ids {
        let n = doc.get_mut(id)?;
        for v in n.token_bound.values_mut() {
            if let Some(rest) = moved(v, &path) {
                *v = format!("{to}{rest}");
            }
        }
    }
    Ok(json!({"renamed": renamed, "aliases": aliases}))
}

/// What follows `path` in `v` when `v` is `path` or below it.
fn moved<'a>(v: &'a str, path: &str) -> Option<&'a str> {
    let rest = v.strip_prefix(path)?;
    (rest.is_empty() || rest.starts_with('.')).then_some(rest)
}

/// Put any value (a token or a group) at `parts`, making groups on the way.
fn put_at(tree: &mut Value, parts: &[String], v: Value) -> std::result::Result<(), String> {
    let Some((last, parents)) = parts.split_last() else { return Err("empty path".into()) };
    let mut cur = tree;
    for p in parents {
        let o = cur.as_object_mut().ok_or("not inside a group")?;
        let slot = o.entry(p.clone()).or_insert_with(|| json!({}));
        if dt::is_token(slot) || !slot.is_object() {
            return Err(format!("{p} is a token, not a group"));
        }
        cur = slot;
    }
    cur.as_object_mut().ok_or("not inside a group")?.insert(last.clone(), v);
    Ok(())
}

// ----- Contexts and bindings -----

fn set_context(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "token.setContext";
    let ids = match get(p, "ids") {
        Some(Value::Array(_)) => req_targets(cmd, s, p)?,
        _ => Vec::new(),
    };
    let doc = s.doc_mut()?;
    let m = modifier_name(cmd, doc, p)?;
    let c = match get(p, "context") {
        Some(Value::Null) | None => None,
        Some(_) => Some(context_name(cmd, doc, &m, p)?),
    };
    if ids.is_empty() {
        match c {
            Some(c) => doc.tokens.active.insert(m, c),
            None => doc.tokens.active.remove(&m),
        };
        return ok();
    }
    for id in ids {
        let tc = &mut doc.get_mut(id)?.token_contexts;
        match &c {
            Some(c) => tc.insert(m.clone(), c.clone()),
            None => tc.remove(&m),
        };
    }
    ok()
}

/// The token types field `field` of `node` takes.
pub fn field_types(doc: &Document, node: NodeId, field: &str) -> &'static [TokenType] {
    use TokenType::*;
    let text = doc.node(node).is_some_and(|n| matches!(n.kind, NodeKind::Text(_)));
    match field {
        "typography" if text => &[Typography],
        "shadow" => &[Shadow],
        "border" => &[Border],
        "strokeStyle" => &[StrokeStyle],
        "opacity" => &[Number],
        "fontFamily" if text => &[FontFamily],
        "fontWeight" if text => &[FontWeight, Number],
        "lineHeight" if text => &[Number, Dimension],
        "fontSize" | "letterSpacing" | "paragraphSpacing" | "paragraphIndent" if text => &[Dimension, Number],
        "characters" if text => &[Number, Dimension, FontFamily],
        f => match list_binding(f) {
            Some((BoundList::Fills | BoundList::Strokes, _, "gradient")) => &[Gradient],
            Some((BoundList::Fills | BoundList::Strokes, _, _)) => &[Color],
            Some((BoundList::Effects, _, "color")) => &[Color],
            Some((BoundList::Effects | BoundList::Grids, _, _)) => &[Dimension, Number],
            None if super::variables::field_scope(doc, node, f).is_some() => &[Dimension, Number],
            None => &[],
        },
    }
}

fn bind(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "token.bind";
    let field = req_string(cmd, p, "field")?;
    let token = string(p, "token");
    let ids = req_targets(cmd, s, p)?;
    let doc = s.doc_mut()?;
    if let Some(t) = &token {
        let input = doc.tokens.input_for(&BTreeMap::new());
        let r = doc.tokens.resolve(&input);
        let kind = r.tokens.get(t).and_then(|x| x.kind).or_else(|| doc.tokens.all_paths().contains(t).then_some(None).flatten());
        if !doc.tokens.all_paths().contains(t) {
            return Err(bad(cmd, format!("no token {t}")));
        }
        for i in &ids {
            doc.get(*i)?;
            let takes = field_types(doc, *i, &field);
            if !kind.is_some_and(|k| takes.contains(&k)) {
                return Err(bad(cmd, format!("`{field}` doesn't take a {} token", kind.map_or("untyped", TokenType::id))));
            }
        }
    }
    for i in ids {
        let n = doc.get_mut(i)?;
        let Node { fills, strokes, token_bound, .. } = n;
        match &token {
            Some(t) => {
                if token_bound.get(&field) != Some(t) {
                    super::styles::rebind_paint_alpha(fills, strokes, &field, true);
                }
                token_bound.insert(field.clone(), t.clone());
            }
            None => {
                if token_bound.remove(&field).is_some() {
                    super::styles::rebind_paint_alpha(fills, strokes, &field, false);
                }
            }
        }
    }
    ok()
}

/// Write bound token values into layers (`only`: these layers; `None`: every bound one), each in
/// its own contexts.
pub fn apply_token_bindings(doc: &mut Document, only: Option<&BTreeSet<NodeId>>) {
    if doc.tokens.sets.is_empty() && doc.tokens.modifiers.is_empty() {
        return;
    }
    let bound: Vec<NodeId> = match only {
        Some(ids) => ids.iter().copied().filter(|i| doc.node(*i).is_some_and(|n| !n.token_bound.is_empty())).collect(),
        None => doc.nodes().filter(|n| !n.token_bound.is_empty()).map(|n| n.id).collect(),
    };
    let mut cache: BTreeMap<BTreeMap<String, String>, Resolved> = BTreeMap::new();
    for id in bound {
        let input = doc.token_input_for(id);
        if !cache.contains_key(&input) {
            let r = doc.tokens.resolve(&input);
            cache.insert(input.clone(), r);
        }
        let Some(res) = cache.get(&input) else { continue };
        apply_node(doc, id, res);
    }
}

/// A border token's parts: colour, width and dash pattern.
type Border = (Option<Color>, Option<f64>, Option<Vec<f64>>);

fn apply_node(doc: &mut Document, id: NodeId, res: &Resolved) {
    use TokenType as T;
    let Some(n) = doc.node(id) else { return };
    let binds = n.token_bound.clone();
    let mut sets = Map::new();
    let mut paint_sets: Vec<(bool, usize, Option<usize>, Color)> = Vec::new();
    let mut effect_sets: Vec<(usize, String, VariableValue)> = Vec::new();
    let mut grid_sets: Vec<(usize, String, f64)> = Vec::new();
    let mut gradients: Vec<(bool, usize, Vec<ColorStop>)> = Vec::new();
    let mut shadows: Option<Vec<Effect>> = None;
    let mut border: Option<Border> = None;
    let mut dashes: Option<Vec<f64>> = None;
    for (field, path) in &binds {
        let Some(tok) = res.get(path) else { continue };
        let (Some(kind), v) = (tok.kind, &tok.value) else { continue };
        if !field_types(doc, id, field).contains(&kind) {
            continue;
        }
        let num = || match kind {
            T::Dimension => dt::dimension_px(v),
            T::Number | T::FontWeight => v.as_f64().or_else(|| dt::font_weight_of(v)),
            _ => None,
        };
        match field.as_str() {
            "typography" => {
                if let Some(f) = v.get("fontFamily").and_then(dt::font_family_of) {
                    sets.insert("fontFamily".into(), json!(f));
                }
                if let Some(x) = v.get("fontSize").and_then(dt::dimension_px) {
                    sets.insert("fontSize".into(), json!(x));
                }
                if let Some(w) = v.get("fontWeight").and_then(dt::font_weight_of) {
                    sets.insert("fontWeight".into(), json!(w));
                }
                if let Some(x) = v.get("letterSpacing").and_then(dt::dimension_px) {
                    sets.insert("letterSpacing".into(), json!(x));
                }
                if let Some(x) = v.get("lineHeight").and_then(Value::as_f64).filter(|x| x.is_finite()) {
                    sets.insert("lineHeight".into(), json!(format!("{}%", trim_num(x * 100.0))));
                }
            }
            "shadow" => shadows = Some(shadow_effects(v)),
            "border" => {
                border = Some((
                    v.get("color").and_then(dt::color_of),
                    v.get("width").and_then(dt::dimension_px),
                    v.get("style").map(|s| dash_pattern(s, v.get("width").and_then(dt::dimension_px).unwrap_or(1.0))),
                ));
            }
            "strokeStyle" => {
                let w = doc.node(id).map_or(1.0, |n| n.stroke.weight.max(0.5));
                dashes = Some(dash_pattern(v, w));
            }
            "opacity" => {
                if let Some(x) = v.as_f64().filter(|x| x.is_finite()) {
                    // Numbers above 1 read as percentages.
                    sets.insert("opacity".into(), json!(if x > 1.0 { x / 100.0 } else { x }.clamp(0.0, 1.0)));
                }
            }
            "fontFamily" => {
                if let Some(f) = dt::font_family_of(v) {
                    sets.insert("fontFamily".into(), json!(f));
                }
            }
            "lineHeight" => match kind {
                T::Number => {
                    if let Some(x) = v.as_f64().filter(|x| x.is_finite()) {
                        sets.insert("lineHeight".into(), json!(format!("{}%", trim_num(x * 100.0))));
                    }
                }
                _ => {
                    if let Some(x) = num() {
                        sets.insert("lineHeight".into(), json!(x));
                    }
                }
            },
            "characters" => {
                sets.insert("characters".into(), json!(dt::display(Some(kind), v)));
            }
            f => match list_binding(f) {
                Some((list @ (BoundList::Fills | BoundList::Strokes), i, prop)) => {
                    let fills = list == BoundList::Fills;
                    if prop == "gradient" {
                        let stops: Vec<ColorStop> = v
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|s| {
                                let c = s.get("color").and_then(dt::color_of)?;
                                let p = s.get("position").and_then(Value::as_f64).filter(|x| x.is_finite())?.clamp(0.0, 1.0);
                                Some(ColorStop { position: p as f32, color: c })
                            })
                            .collect();
                        if stops.len() >= 2 {
                            gradients.push((fills, i, stops));
                        }
                    } else if let Some(c) = dt::color_of(v) {
                        let stop = if prop == "color" { None } else { super::lists::stop_binding(prop) };
                        if prop == "color" || stop.is_some() {
                            paint_sets.push((fills, i, stop, c));
                        }
                    }
                }
                Some((BoundList::Effects, i, prop)) => {
                    let val = match kind {
                        T::Color => dt::color_of(v).map(VariableValue::Color),
                        _ => num().map(VariableValue::Number),
                    };
                    if let Some(val) = val {
                        effect_sets.push((i, prop.to_string(), val));
                    }
                }
                Some((BoundList::Grids, i, prop)) => {
                    if let Some(x) = num() {
                        grid_sets.push((i, prop.to_string(), x));
                    }
                }
                None => {
                    if let Some(x) = num() {
                        sets.insert(f.to_string(), json!(x));
                    }
                }
            },
        }
    }
    if !sets.is_empty() {
        let old = doc.node_arc(id).cloned();
        let was_dirty = doc.dirty().contains(&id);
        let _ = super::props::apply("tokens", doc, id, &sets);
        if let Some(old) = old {
            // A value a token gives a layer of an instance is no override.
            if doc.node(id).is_some_and(|n| n.overrides != old.overrides)
                && let Ok(n) = doc.get_mut(id)
            {
                n.overrides = old.overrides.clone();
            }
            doc.keep_if_unchanged(old, was_dirty);
        }
    }
    if !paint_sets.is_empty() {
        super::styles::apply_paint_bindings(doc, id, &paint_sets);
    }
    if !effect_sets.is_empty() {
        super::styles::apply_effect_bindings(doc, id, &effect_sets);
    }
    if !grid_sets.is_empty() {
        super::lists::apply_grid_bindings(doc, id, &grid_sets);
    }
    let Some(n) = doc.node(id) else { return };
    let (mut fills, mut strokes, mut effects, mut stroke) = (n.fills.clone(), n.strokes.clone(), n.effects.clone(), n.stroke.clone());
    for (is_fill, i, stops) in gradients {
        let list = if is_fill { &mut fills } else { &mut strokes };
        let len = list.len();
        match list.get_mut(i) {
            Some(p) => match &mut p.kind {
                PaintKind::Gradient(g) => g.stops = stops,
                k => {
                    let mut g = Gradient::default_for(Default::default(), Color::BLACK);
                    g.stops = stops;
                    *k = PaintKind::Gradient(g);
                }
            },
            None if i == len => {
                let mut g = Gradient::default_for(Default::default(), Color::BLACK);
                g.stops = stops;
                list.push(Paint::gradient(g));
            }
            None => {}
        }
    }
    if let Some(sh) = shadows {
        let rest = effects.iter().filter(|e| !matches!(e.kind, EffectKind::DropShadow(_) | EffectKind::InnerShadow(_))).cloned();
        effects = sh.into_iter().chain(rest).collect();
    }
    if let Some((c, w, d)) = border {
        if let Some(c) = c {
            super::styles::set_paint_color(&mut strokes, 0, None, c);
        }
        if let Some(w) = w {
            stroke.weight = w.clamp(0.0, 10_000.0);
        }
        if let Some(d) = d {
            stroke.dashes = d;
        }
    }
    if let Some(d) = dashes {
        stroke.dashes = d;
    }
    if (n.fills != fills || n.strokes != strokes || n.effects != effects || n.stroke != stroke)
        && let Ok(n) = doc.get_mut(id)
    {
        n.fills = fills;
        n.strokes = strokes;
        n.effects = effects;
        n.stroke = stroke;
    }
}

/// A shadow token's shadows as effects (the token's first shadow on top).
fn shadow_effects(v: &Value) -> Vec<Effect> {
    let one = |s: &Value| -> Option<Effect> {
        let px = |k: &str| s.get(k).and_then(dt::dimension_px).unwrap_or(0.0).clamp(-10_000.0, 10_000.0);
        let sh = Shadow {
            color: s.get("color").and_then(dt::color_of)?,
            offset: (px("offsetX"), px("offsetY")),
            radius: px("blur").max(0.0),
            spread: px("spread"),
            ..Default::default()
        };
        let kind = if s.get("inset").and_then(Value::as_bool) == Some(true) { EffectKind::InnerShadow(sh) } else { EffectKind::DropShadow(sh) };
        Some(Effect { kind, visible: true })
    };
    match v {
        Value::Array(a) => a.iter().take(64).filter_map(one).collect(),
        _ => one(v).into_iter().collect(),
    }
}

/// The dash pattern for a stroke style at stroke weight `w` (empty: solid).
fn dash_pattern(v: &Value, w: f64) -> Vec<f64> {
    let w = if w.is_finite() { w.clamp(0.5, 1000.0) } else { 1.0 };
    match v.as_str() {
        Some("dashed") => vec![w * 3.0, w * 2.0],
        Some("dotted") => vec![w, w],
        Some(_) => Vec::new(),
        None => v
            .get("dashArray")
            .and_then(Value::as_array)
            .map(|a| a.iter().take(64).filter_map(dt::dimension_px).map(|x| x.clamp(0.0, 10_000.0)).collect())
            .unwrap_or_default(),
    }
}

// ----- Import and export -----

/// The JSON a `tokens` param holds (object, text or path), with the path it came from.
fn tokens_param(cmd: &str, p: &Value) -> Result<(Input, Option<String>)> {
    let parse = |text: &[u8]| serde_json::from_slice::<Value>(text).map(Input::Json).map_err(|e| bad(cmd, format!("tokens: {e}")));
    let bytes = |b: Vec<u8>| if figcraft_format::tokens_bundle::is_zip(&b) { Ok(Input::Zip(b)) } else { parse(&b) };
    match (get(p, "tokens"), get(p, "data")) {
        (Some(v @ Value::Object(_)), _) => Ok((Input::Json(v.clone()), None)),
        (Some(Value::String(s)), _) if s.trim_start().starts_with('{') => Ok((parse(s.as_bytes())?, None)),
        (Some(Value::String(path)), _) => Ok((bytes(super::file::read_capped(path, MAX_FILE)?)?, Some(path.clone()))),
        (None | Some(Value::Null), Some(Value::String(b64))) => {
            use base64::Engine as _;
            if b64.len() as u64 > MAX_FILE * 4 / 3 + 4 {
                return Err(bad(cmd, "data: too large"));
            }
            let b = base64::engine::general_purpose::STANDARD.decode(b64.trim()).map_err(|e| bad(cmd, format!("data: {e}")))?;
            Ok((bytes(b)?, None))
        }
        _ => Err(bad(cmd, "tokens: a token zip, DTCG token file or resolver document (an object, JSON text or a path), or data (base64)")),
    }
}

/// What an import reads: one JSON document, or a zip of token files.
enum Input {
    Json(Value),
    Zip(Vec<u8>),
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "token.import";
    let (input, path) = tokens_param(cmd, p)?;
    let replace = boolean(p, "replace").unwrap_or(false);
    let set_name = name_param(cmd, p, "set")?;
    let doc = s.doc_mut()?;
    let mut warnings = Vec::new();
    let (added_sets, added_mods);
    // A zip, or a resolver document: whole sets and modifiers.
    let whole = match &input {
        Input::Zip(b) => Some(figcraft_format::tokens_bundle::read(b).map_err(|e| bad(cmd, e.to_string()))?),
        Input::Json(v) if dt::is_resolver(v) => {
            let dir = path.as_deref().and_then(|x| std::path::Path::new(x).parent()).map(std::path::Path::to_path_buf);
            let load = |file: &str| -> Option<Value> {
                let dir = dir.as_ref()?;
                // Only files next to the document or below it.
                if file.contains("..") || std::path::Path::new(file).is_absolute() {
                    return None;
                }
                let bytes = super::file::read_capped(&dir.join(file).to_string_lossy(), MAX_FILE).ok()?;
                serde_json::from_slice(&bytes).ok()
            };
            Some(dt::from_resolver(v, &load).map_err(|e| bad(cmd, e))?)
        }
        Input::Json(_) => None,
    };
    let v = match input {
        Input::Json(v) => v,
        Input::Zip(_) => Value::Null,
    };
    if let Some((t, w)) = whole {
        warnings.extend(w);
        added_sets = t.sets.len();
        added_mods = t.modifiers.len();
        if replace || doc.tokens.is_empty() {
            let active = std::mem::take(&mut doc.tokens.active);
            doc.tokens = t;
            doc.tokens.active = active;
        } else {
            join(&mut doc.tokens, t);
        }
    } else {
        if !v.is_object() {
            return Err(bad(cmd, "tokens: a JSON object"));
        }
        match (string(p, "modifier"), set_name) {
            (Some(_), _) => {
                let src = source_of(cmd, doc, p)?;
                if let Some(t) = doc.tokens.tree_mut(&src) {
                    dt::merge_into(t, &v, 0);
                }
                (added_sets, added_mods) = (0, 0);
            }
            (None, name) => {
                let stem = path.as_deref().and_then(|x| std::path::Path::new(x).file_name()).map(|f| f.to_string_lossy().to_string());
                let stem = stem.as_deref().map(|f| f.trim_end_matches(".json").trim_end_matches(".tokens").to_string());
                let name = name.or(stem).filter(|n| dt::valid_set_name(n)).unwrap_or_else(|| "tokens".into());
                match doc.tokens.sets.iter_mut().find(|x| x.name == name) {
                    Some(set) => {
                        dt::merge_into(&mut set.tokens, &v, 0);
                        added_sets = 0;
                    }
                    None => {
                        doc.tokens.sets.push(TokenSet { name: name.clone(), description: String::new(), tokens: v.clone() });
                        doc.tokens.order = doc.tokens.full_order();
                        added_sets = 1;
                    }
                }
                added_mods = 0;
            }
        }
    }
    let input = doc.tokens.input_for(&BTreeMap::new());
    let r = doc.tokens.resolve(&input);
    warnings.extend(r.errors.iter().take(100).cloned());
    if r.tokens.is_empty() {
        warnings.push("no tokens found".into());
    }
    Ok(json!({"sets": added_sets, "modifiers": added_mods, "tokens": r.tokens.len(), "warnings": warnings}))
}

/// Join imported tokens into this file's: sets and contexts of the same name merge (the import's
/// tokens win), new ones are added in the import's order.
fn join(into: &mut DesignTokens, t: DesignTokens) {
    for s in t.sets {
        match into.sets.iter_mut().find(|x| x.name == s.name) {
            Some(x) => dt::merge_into(&mut x.tokens, &s.tokens, 0),
            None if !into.modifiers.iter().any(|m| m.name == s.name) => into.sets.push(s),
            None => {
                let name = dt::unique_name(into, &s.name);
                into.sets.push(TokenSet { name, ..s });
            }
        }
    }
    for m in t.modifiers {
        match into.modifiers.iter_mut().find(|x| x.name == m.name) {
            Some(x) => {
                for c in m.contexts {
                    match x.contexts.iter_mut().find(|y| y.name == c.name) {
                        Some(y) => dt::merge_into(&mut y.tokens, &c.tokens, 0),
                        None => x.contexts.push(c),
                    }
                }
            }
            None if !into.sets.iter().any(|s| s.name == m.name) => into.modifiers.push(m),
            None => {
                let name = dt::unique_name(into, &m.name);
                into.modifiers.push(TokenModifier { name, ..m });
            }
        }
    }
    let mut order = into.full_order();
    for item in t.order {
        if !order.contains(&item) {
            order.push(item);
        }
    }
    into.order = order;
    into.order = into.full_order();
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "token.export";
    let doc = s.doc()?;
    let format = string(p, "format").unwrap_or_else(|| "resolver".into());
    if format == "zip" {
        let bytes = figcraft_format::tokens_bundle::write(&doc.tokens, &doc.name).map_err(|e| EngineError::Other(e.to_string()))?;
        return match string(p, "path") {
            Some(path) => {
                super::file::write(&path, &bytes)?;
                Ok(json!({"path": path}))
            }
            None => {
                use base64::Engine as _;
                Ok(json!({"data": base64::engine::general_purpose::STANDARD.encode(&bytes)}))
            }
        };
    }
    let value = match format.as_str() {
        "resolver" => dt::to_resolver(&doc.tokens, &doc.name),
        "tokens" => {
            let tree = if get(p, "set").is_some() || get(p, "modifier").is_some() {
                let src = source_of(cmd, doc, p)?;
                doc.tokens.tree(&src).cloned().unwrap_or_else(|| json!({}))
            } else {
                let input = input_param(cmd, doc, p)?.unwrap_or_else(|| doc.tokens.input_for(&BTreeMap::new()));
                doc.tokens.merged(&input)
            };
            if boolean(p, "resolveAliases").unwrap_or(false) { resolved_tree(&tree) } else { tree }
        }
        _ => return Err(bad(cmd, "format: zip|resolver|tokens")),
    };
    let text = serde_json::to_string_pretty(&value).map_err(|e| EngineError::Other(e.to_string()))?;
    match string(p, "path") {
        Some(path) => {
            super::file::write(&path, text.as_bytes())?;
            Ok(json!({"path": path}))
        }
        None => Ok(json!({"text": text})),
    }
}

/// `tree` with every token's value replaced by its resolved one (and its type written on it).
fn resolved_tree(tree: &Value) -> Value {
    let r = dt::resolve_tree(tree);
    let mut out = tree.clone();
    for t in r.tokens.values().filter(|t| t.error.is_none()) {
        let Some(parts) = dt::split_path(&t.path) else { continue };
        if let Some(o) = dt::at_path_mut(&mut out, &parts).and_then(Value::as_object_mut) {
            o.insert("$value".into(), t.value.clone());
            if let Some(k) = t.kind {
                o.insert("$type".into(), json!(k.id()));
            }
        }
    }
    out
}
