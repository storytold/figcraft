//! Code connect: a main component (or component set) linked to the code component it is built as,
//! per language: an import path, the code component's name and a template mapping component
//! properties to its props. `inspect.code`, `inspect.componentCode` and Dev Mode then show that
//! snippet for its instances (with their values, also inside generated layer trees) instead of
//! generated code. Links are stored on the component or set (`Node::code_links`: saved, synced,
//! undoable) and travel as `figcraft.connect.json` kept in the code repository.
//!
//! Templates are text with placeholders and are never executed: `{prop}` (a property's name or its
//! parameter name) becomes the value as an expression in the language, escaped for it (React
//! keeps JSX's braces: `size={size}` → `size="Large"`, `{label}` → `{"Buy"}`), and
//! `{prop ? "a" : "b"}` picks one of two strings. Any other `{…}` stays as written.

use figcraft_doc::*;
use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::codegen::CodeLang;
use crate::component_codegen::{Component, Param, ParamKind, describe, instance_values, js_string, main_of, value_expr};
use crate::components::variant_set;
use crate::params::*;
use crate::{EngineError, Result, Session, bad, spec};

/// Longest template (chars).
pub const MAX_TEMPLATE: usize = 4_000;
/// Longest code component name (chars).
pub const MAX_NAME: usize = 100;
/// Longest import path (chars).
pub const MAX_IMPORT: usize = 300;
/// Most links a connect file holds.
pub const MAX_FILE_LINKS: usize = 5_000;
/// Largest connect file read (bytes).
const MAX_FILE: u64 = 4 * 1024 * 1024;
/// The connect file's format version.
const VERSION: u64 = 1;
/// Most warnings an import reports.
const MAX_WARNINGS: usize = 50;

/// The languages a component can be connected in.
pub fn language(s: &str) -> Option<CodeLang> {
    CodeLang::from_id(s).filter(|l| matches!(l, CodeLang::React | CodeLang::Leptos | CodeLang::SwiftUi | CodeLang::Compose))
}

/// The layer `id`'s links are stored on: its component set, else its component (an instance's
/// main). shortcut: links a component had before it joined a set stay on it unused; move them to
/// the set when combining variants if people hit it.
pub fn owner(doc: &Document, id: NodeId) -> Option<NodeId> {
    let comp = match &doc.node(id)?.kind {
        NodeKind::ComponentSet { .. } => return Some(id),
        NodeKind::Component { .. } => id,
        NodeKind::Instance { component, .. } => *component,
        _ => return None,
    };
    doc.node(comp).filter(|c| matches!(c.kind, NodeKind::Component { .. }))?;
    Some(variant_set(doc, comp).unwrap_or(comp))
}

/// The link layer `id` (a component, set or instance) has for `lang`.
pub fn link(doc: &Document, id: NodeId, lang: CodeLang) -> Option<&CodeLink> {
    doc.node(owner(doc, id)?)?.code_links.iter().find(|l| l.language == lang.id())
}

/// What a link's import brings in: `Card` for the code component `Card.Header`.
fn imported(link: &CodeLink) -> &str {
    link.name.split('.').next().unwrap_or_default()
}

/// The line importing `names` from `path`.
fn import_of(path: &str, names: &[&str], lang: CodeLang) -> String {
    match lang {
        _ if path.is_empty() => String::new(),
        CodeLang::React => format!("import {{ {} }} from {};", names.join(", "), js_string(path)),
        CodeLang::Leptos if names.len() == 1 => format!("use {path}::{};", names[0]),
        CodeLang::Leptos => format!("use {path}::{{{}}};", names.join(", ")),
        _ => format!("import {path}"),
    }
}

/// The import line of `link` ("" without an import path).
pub fn import_line(link: &CodeLink, lang: CodeLang) -> String {
    if link.import_path.is_empty() { String::new() } else { import_of(&link.import_path, &[imported(link)], lang) }
}

/// The import lines of `links` (the connected instances in a generated layer tree): one line per
/// import path, in path order, each naming what it brings in once, in name order. "" when none
/// has an import path.
pub fn import_lines(links: &[CodeLink], lang: CodeLang) -> String {
    let mut by_path: std::collections::BTreeMap<&str, std::collections::BTreeSet<&str>> = std::collections::BTreeMap::new();
    for l in links.iter().filter(|l| !l.import_path.is_empty()) {
        by_path.entry(&l.import_path).or_default().insert(imported(l));
    }
    let lines: Vec<String> = by_path.iter().map(|(path, names)| import_of(path, &names.iter().copied().collect::<Vec<_>>(), lang)).collect();
    lines.join("\n")
}

/// Layer `id`'s code with `link`: the import line and the snippet (an instance's values, else the
/// defaults). `None` when `id` isn't a component, set or instance.
pub fn connected_code(doc: &Document, id: NodeId, lang: CodeLang, link: &CodeLink) -> Option<String> {
    let c = main_of(doc, id).and_then(|m| describe(doc, m))?;
    let values = instance_values(doc, id, &c).unwrap_or_else(|| c.params.iter().map(|p| (p.prop.clone(), p.default.clone())).collect());
    let snippet = render(doc, &c, &values, link, lang, "");
    let import = import_line(link, lang);
    Some(if import.is_empty() { snippet } else { format!("{import}\n\n{snippet}") })
}

// ------------------------------------------------------------------ templates

/// A piece of a template (`inner`: the text between the braces, written back for names that
/// aren't properties).
#[derive(Debug, PartialEq)]
enum Piece<'a> {
    Text(&'a str),
    Value { name: &'a str, inner: &'a str },
    Pick { name: &'a str, yes: &'a str, no: &'a str, inner: &'a str },
}

/// Whether `s` reads as a property name rather than code.
fn is_name(s: &str) -> bool {
    !s.is_empty() && s.chars().count() <= MAX_NAME && s.chars().all(|c| c.is_alphanumeric() || " _-./".contains(c))
}

/// `"text"…` → (text, what follows the closing quote).
fn quoted(s: &str) -> Option<(&str, &str)> {
    s.trim_start().strip_prefix('"')?.split_once('"')
}

/// One `{…}`: a placeholder, `None` for code kept as written, an error for a malformed pick.
fn placeholder(inner: &str) -> std::result::Result<Option<Piece<'_>>, String> {
    let s = inner.trim();
    let Some((name, picks)) = s.split_once('?') else { return Ok(is_name(s).then_some(Piece::Value { name: s, inner })) };
    // A code ternary without string choices stays as written.
    if !picks.contains('"') {
        return Ok(None);
    }
    let name = name.trim();
    let malformed = || format!("write {{{name} ? \"a\" : \"b\"}}");
    let (yes, rest) = quoted(picks).ok_or_else(malformed)?;
    let rest = rest.trim_start().strip_prefix(':').ok_or_else(malformed)?;
    let (no, rest) = quoted(rest).ok_or_else(malformed)?;
    if !rest.trim().is_empty() || !is_name(name) {
        return Err(malformed());
    }
    Ok(Some(Piece::Pick { name, yes, no, inner }))
}

/// Template `t` as pieces.
fn parse(t: &str) -> std::result::Result<Vec<Piece<'_>>, String> {
    let mut out = Vec::new();
    let mut rest = t;
    while let Some((before, after)) = rest.split_once('{') {
        if !before.is_empty() {
            out.push(Piece::Text(before));
        }
        let piece = match after.split_once('}') {
            Some((inner, tail)) if !inner.contains('{') => placeholder(inner)?.map(|p| (p, tail)),
            _ => None,
        };
        match piece {
            Some((p, tail)) => {
                out.push(p);
                rest = tail;
            }
            // A literal brace (JSX's own, an object, code): the rest is scanned again.
            None => {
                out.push(Piece::Text("{"));
                rest = after;
            }
        }
    }
    if !rest.is_empty() {
        out.push(Piece::Text(rest));
    }
    Ok(out)
}

/// The parameter of `c` a placeholder names (its property name or its name in code), with its
/// value.
fn find<'a>(c: &'a Component, values: &'a [(String, Value)], name: &str) -> Option<(&'a Param, &'a Value)> {
    c.params.iter().zip(values).find(|(p, _)| p.prop == name || p.ident == name).map(|(p, (_, v))| (p, v))
}

/// Whether a value picks the first string of `{prop ? "a" : "b"}`.
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty() && !["false", "off", "no", "0", "none"].iter().any(|f| s.eq_ignore_ascii_case(f)),
        _ => true,
    }
}

/// A parameter's value as an expression: an instance swap to a connected component calls it by
/// its code name.
fn expr(doc: &Document, c: &Component, p: &Param, v: &Value, lang: CodeLang) -> String {
    let swapped = (p.kind == ParamKind::InstanceSwap).then(|| id_of(v)).flatten().and_then(|i| link(doc, i, lang));
    match (swapped, lang) {
        (Some(l), CodeLang::React) => format!("<{} />", l.name),
        (Some(l), CodeLang::Leptos) => format!("|| view! {{ <{} /> }}", l.name),
        (Some(l), CodeLang::SwiftUi) => format!("{}()", l.name),
        (Some(l), _) => format!("{{ {}() }}", l.name),
        (None, _) => value_expr(doc, c, p, v, lang),
    }
}

/// `link`'s template with `values` (one per parameter of `c`), every line prefixed by `pad`.
pub(crate) fn render(doc: &Document, c: &Component, values: &[(String, Value)], link: &CodeLink, lang: CodeLang, pad: &str) -> String {
    // A template that skipped `check` (a hand-edited file) and is far too long stays as written:
    // parsing rescans after each literal brace.
    let checkable = link.template.len() <= MAX_TEMPLATE * 4;
    let pieces = checkable.then(|| parse(&link.template).ok()).flatten().unwrap_or_else(|| vec![Piece::Text(&link.template)]);
    let mut out = String::new();
    for piece in pieces {
        match piece {
            Piece::Text(t) => out.push_str(t),
            Piece::Value { name, inner } => match find(c, values, name) {
                Some((p, v)) => {
                    let e = expr(doc, c, p, v, lang);
                    // JSX and `view!`: a plain string reads best as an attribute value, anything
                    // else is an expression in braces (JSX attribute strings have no escapes).
                    let markup = matches!(lang, CodeLang::React | CodeLang::Leptos);
                    if !markup || (out.ends_with('=') && e.starts_with('"') && !e.contains('\\')) {
                        out.push_str(&e);
                    } else {
                        out.push_str(&format!("{{{e}}}"));
                    }
                }
                None => out.push_str(&format!("{{{inner}}}")),
            },
            Piece::Pick { name, yes, no, inner } => match find(c, values, name) {
                Some((_, v)) => out.push_str(if truthy(v) { yes } else { no }),
                None => out.push_str(&format!("{{{inner}}}")),
            },
        }
    }
    out.lines().map(|l| if l.is_empty() { "\n".to_string() } else { format!("{pad}{l}\n") }).collect()
}

/// A starting template: the code component called with every property (but slots) passed.
fn default_template(c: &Component, lang: CodeLang, name: &str) -> String {
    let params: Vec<&str> = c.params.iter().filter(|p| p.kind != ParamKind::Slot).map(|p| p.ident.as_str()).collect();
    match lang {
        CodeLang::React => format!("<{name}{} />", params.iter().map(|p| format!(" {p}={{{p}}}")).collect::<String>()),
        CodeLang::Leptos => {
            format!("<{name}{} />", params.iter().map(|p| format!(" {}={{{p}}}", crate::component_codegen::rust_ident(p))).collect::<String>())
        }
        CodeLang::SwiftUi => format!("{name}({})", params.iter().map(|p| format!("{p}: {{{p}}}")).collect::<Vec<_>>().join(", ")),
        _ => format!("{name}({})", params.iter().map(|p| format!("{p} = {{{p}}}")).collect::<Vec<_>>().join(", ")),
    }
}

/// Checks a link: its name, import path and template. Returns the placeholder names that aren't
/// properties of `c` (kept as written).
pub fn check(c: &Component, link: &CodeLink, lang: CodeLang) -> std::result::Result<Vec<String>, String> {
    let name = &link.name;
    let mut chars = name.chars();
    if name.chars().count() > MAX_NAME
        || !chars.next().is_some_and(|f| f.is_ascii_alphabetic() || f == '_' || f == '$')
        || !chars.all(|ch| ch.is_ascii_alphanumeric() || "_$.".contains(ch))
    {
        return Err(format!("name: the code component's name (letters, digits, _ $ and ., up to {MAX_NAME})"));
    }
    let path = &link.import_path;
    let path_ok = match lang {
        CodeLang::React => !path.chars().any(|ch| ch.is_control() || ch == '"' || ch == '\\'),
        CodeLang::Leptos => path.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == ':'),
        _ => path.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '.'),
    };
    if path.chars().count() > MAX_IMPORT || !path_ok {
        return Err(match lang {
            CodeLang::React => format!("importPath: a module path without quotes, up to {MAX_IMPORT} characters"),
            CodeLang::Leptos => format!("importPath: a Rust module path (letters, digits, _ and ::), up to {MAX_IMPORT} characters"),
            _ => format!("importPath: a module or package name (letters, digits, _ and .), up to {MAX_IMPORT} characters"),
        });
    }
    let t = &link.template;
    if t.trim().is_empty() || t.chars().count() > MAX_TEMPLATE || t.chars().any(|ch| ch.is_control() && !"\n\t\r".contains(ch)) {
        return Err(format!("template: the snippet, up to {MAX_TEMPLATE} characters"));
    }
    let pieces = parse(t).map_err(|e| format!("template: {e}"))?;
    let values: Vec<(String, Value)> = c.params.iter().map(|p| (p.prop.clone(), Value::Null)).collect();
    let mut unknown: Vec<String> = Vec::new();
    for p in pieces {
        if let Piece::Value { name, .. } | Piece::Pick { name, .. } = p
            && find(c, &values, name).is_none()
            && !unknown.iter().any(|u| u == name)
        {
            unknown.push(name.to_string());
        }
    }
    Ok(unknown)
}

// ------------------------------------------------------------------ commands

/// A string parameter (`None` when missing or null).
fn text(cmd: &str, p: &Value, key: &str) -> Result<Option<String>> {
    match get(p, key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(bad(cmd, format!("{key}: a string"))),
    }
}

/// The `language` parameter (default React).
fn lang_param(cmd: &str, p: &Value) -> Result<CodeLang> {
    match text(cmd, p, "language")? {
        Some(l) => language(&l).ok_or_else(|| bad(cmd, "language: react|leptos|swiftui|compose")),
        None => Ok(CodeLang::React),
    }
}

/// The target of a command: (the layer, the component or set its links are on, its code).
fn target(cmd: &str, s: &Session, p: &Value) -> Result<(NodeId, NodeId, Component)> {
    let id = targets(s, p).first().copied().ok_or_else(|| bad(cmd, "select a component or instance"))?;
    let doc = s.doc()?;
    let not = || bad(cmd, "not a component, set or instance");
    let owner = owner(doc, id).ok_or_else(not)?;
    let c = main_of(doc, owner).and_then(|m| describe(doc, m)).ok_or_else(not)?;
    Ok((id, owner, c))
}

/// Store `link` on `owner`, replacing its link for the same language.
fn store(doc: &mut Document, owner: NodeId, link: CodeLink) -> Result<()> {
    let n = doc.get_mut(owner)?;
    match n.code_links.iter_mut().find(|l| l.language == link.language) {
        Some(l) => *l = link,
        None => n.code_links.push(link),
    }
    Ok(())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "codeConnect.set",
            "Connect to code",
            "{id? (component, set or instance; default the selection), language?: react|leptos|swiftui|compose (default react), importPath?, name? (default the generated name), template? (default a call passing every property)}  links the component (its set, for a variant) to its code component; fields left out keep their current value. Placeholders: {property} (its name or parameter name) and {property ? \"a\" : \"b\"}; other {…} stay as written  → {component, language, code (the snippet for the layer), unknown (placeholder names that aren't properties)}",
            true,
            |s, p| {
                let cmd = "codeConnect.set";
                let lang = lang_param(cmd, p)?;
                let (id, owner, c) = target(cmd, s, p)?;
                let doc = s.doc()?;
                let old = link(doc, owner, lang).cloned().unwrap_or_default();
                let name =
                    text(cmd, p, "name")?.map(|n| n.trim().to_string()).unwrap_or(if old.name.is_empty() { c.type_name.clone() } else { old.name });
                let template = match text(cmd, p, "template")? {
                    Some(t) => t,
                    None if old.template.is_empty() => default_template(&c, lang, &name),
                    None => old.template,
                };
                let import_path = text(cmd, p, "importPath")?.map_or(old.import_path, |i| i.trim().to_string());
                let link = CodeLink { language: lang.id().into(), import_path, name, template };
                let unknown = check(&c, &link, lang).map_err(|e| bad(cmd, e))?;
                let code = connected_code(doc, id, lang, &link);
                store(s.doc_mut()?, owner, link)?;
                Ok(json!({"component": owner, "language": lang.id(), "code": code, "unknown": unknown}))
            }
        ),
        spec!(
            "codeConnect.remove",
            "Disconnect from code",
            "{id? (component, set or instance; default the selection), language? (omit: every language)}  → {component, removed}",
            true,
            |s, p| {
                let cmd = "codeConnect.remove";
                let lang = match get(p, "language") {
                    None | Some(Value::Null) => None,
                    Some(_) => Some(lang_param(cmd, p)?),
                };
                let (_, owner, _) = target(cmd, s, p)?;
                let n = s.doc_mut()?.get_mut(owner)?;
                let before = n.code_links.len();
                n.code_links.retain(|l| lang.is_some_and(|k| l.language != k.id()));
                let removed = before - n.code_links.len();
                if removed == 0 {
                    return Err(bad(cmd, "the component isn't connected to code"));
                }
                Ok(json!({"component": owner, "removed": removed}))
            }
        ),
        spec!(
            "codeConnect.export",
            "Export code connections",
            "{path? (write it there, e.g. figcraft.connect.json in the code repository)}  every component's links as {version: 1, links: [{component (its name), language, importPath, name, template}]}, sorted  → {text, links} or {path, links}",
            false,
            |s, p| {
                let cmd = "codeConnect.export";
                let path = text(cmd, p, "path")?;
                let doc = s.doc()?;
                let mut links: Vec<Value> = doc
                    .nodes()
                    .filter(|n| owner(doc, n.id) == Some(n.id))
                    .flat_map(|n| {
                        n.code_links.iter().map(|l| {
                            json!({"component": n.name, "language": l.language, "importPath": l.import_path, "name": l.name, "template": l.template})
                        })
                    })
                    .collect();
                let key =
                    |v: &Value| (v["component"].as_str().unwrap_or_default().to_string(), v["language"].as_str().unwrap_or_default().to_string());
                links.sort_by_key(key);
                let count = links.len();
                let text =
                    serde_json::to_string_pretty(&json!({"version": VERSION, "links": links})).map_err(|e| EngineError::Other(e.to_string()))? + "\n";
                match path {
                    Some(path) => {
                        crate::cmd::file::write(&path, text.as_bytes())?;
                        Ok(json!({"path": path, "links": count}))
                    }
                    None => Ok(json!({"text": text, "links": count})),
                }
            }
        ),
        spec!(
            "codeConnect.import",
            "Import code connections",
            "{json (the connect JSON as codeConnect.export writes it: an object or JSON text) | path (a figcraft.connect.json)}  links each entry to every component or set of that name (replacing its link for the language); invalid entries and unknown components are skipped with a warning  → {linked, warnings}",
            true,
            import
        ),
    ]
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "codeConnect.import";
    let parse_json = |b: &[u8]| serde_json::from_slice::<Value>(b).map_err(|e| bad(cmd, format!("not JSON: {e}")));
    let v = match (get(p, "json"), text(cmd, p, "path")?) {
        (Some(v @ Value::Object(_)), _) => v.clone(),
        (Some(Value::String(t)), _) => parse_json(t.as_bytes())?,
        (Some(_), _) => return Err(bad(cmd, "json: the connect JSON (an object or JSON text)")),
        (None, Some(path)) => parse_json(&crate::cmd::file::read_capped(&path, MAX_FILE)?)?,
        (None, None) => return Err(bad(cmd, "pass `json` or `path`")),
    };
    if v.get("version").is_some_and(|x| x.as_u64() != Some(VERSION)) {
        return Err(bad(cmd, format!("version: only {VERSION} is supported")));
    }
    let entries = v.get("links").and_then(Value::as_array).ok_or_else(|| bad(cmd, "expected {version: 1, links: [...]}"))?;
    if entries.len() > MAX_FILE_LINKS {
        return Err(bad(cmd, format!("at most {MAX_FILE_LINKS} links")));
    }
    let doc = s.doc()?;
    // Components and sets by name (variants are linked through their set).
    let owners: Vec<(String, NodeId)> = doc.nodes().filter(|n| owner(doc, n.id) == Some(n.id)).map(|n| (n.name.clone(), n.id)).collect();
    let mut warnings: Vec<String> = Vec::new();
    let mut apply: Vec<(NodeId, CodeLink)> = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        let field = |k: &str| e.get(k).and_then(Value::as_str).map(str::trim);
        let (Some(component), Some(lang), Some(name), Some(template)) =
            (field("component"), field("language").and_then(language), field("name"), e.get("template").and_then(Value::as_str))
        else {
            warnings.push(format!("link {}: needs component, language (react|leptos|swiftui|compose), name and template strings", i + 1));
            continue;
        };
        let link = CodeLink {
            language: lang.id().into(),
            import_path: field("importPath").unwrap_or_default().to_string(),
            name: name.into(),
            template: template.into(),
        };
        let matches: Vec<NodeId> = owners.iter().filter(|(n, _)| n == component).map(|(_, id)| *id).collect();
        if matches.is_empty() {
            warnings.push(format!("link {}: no component named {component:?}", i + 1));
        }
        for id in matches {
            let Some(c) = main_of(doc, id).and_then(|m| describe(doc, m)) else { continue };
            match check(&c, &link, lang) {
                Ok(_) => apply.push((id, link.clone())),
                Err(err) => warnings.push(format!("link {} ({component}): {err}", i + 1)),
            }
        }
    }
    let linked = apply.len();
    let doc = s.doc_mut()?;
    for (id, link) in apply {
        store(doc, id, link)?;
    }
    warnings.truncate(MAX_WARNINGS);
    Ok(json!({"linked": linked, "warnings": warnings}))
}
