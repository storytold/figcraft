//! Component code: a component (or component set) becomes a React, Leptos, SwiftUI or Jetpack
//! Compose component whose properties are its parameters (a variant property → a union or enum, boolean →
//! a flag showing the layers bound to it, text → a string, instance swap → a view parameter, slot →
//! optional content), and an instance becomes a call passing the values it sets. Variants of a set
//! share one definition (the set's default variant); in React and Leptos their differences become CSS
//! rules on `data-*` attributes. Returned by `inspect.componentCode` and used by `inspect.code`. A component
//! connected to code (see [`crate::code_connect`]) shows its real snippet instead.

use figcraft_doc::*;
use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::codegen::{CodeLang, Ctx, Markup, class_name, compose, indent, literal, swift};
use crate::components::{prop_default_value, slot_edited, slots_of, variant_options, variant_props, variant_set};
use crate::params::*;
use crate::{bad, spec};

/// What a component property becomes in code.
#[derive(Clone, Debug, PartialEq)]
pub enum ParamKind {
    Boolean,
    Text,
    InstanceSwap,
    /// A variant property with its values.
    Variant(Vec<String>),
    Slot,
}

impl ParamKind {
    pub fn id(&self) -> &'static str {
        match self {
            ParamKind::Boolean => "boolean",
            ParamKind::Text => "text",
            ParamKind::InstanceSwap => "instanceSwap",
            ParamKind::Variant(_) => "variant",
            ParamKind::Slot => "slot",
        }
    }
}

/// A component property as a parameter.
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    /// The property's name.
    pub prop: String,
    /// Its name in code: camelCase, unique, never a keyword.
    pub ident: String,
    pub kind: ParamKind,
    /// The default: a bool, a string, a component id (instance swap) or null (slot).
    pub default: Value,
    /// Variant properties: the enum's name in SwiftUI (PascalCase, unique, never a type the
    /// generated code uses); Compose prefixes it with the component's name. Empty otherwise.
    pub enum_name: String,
}

/// A component as code: the main whose layers are drawn (a set's default variant), its set, its
/// name in code and its parameters (variant properties first).
#[derive(Clone, Debug, PartialEq)]
pub struct Component {
    pub comp: NodeId,
    pub set: Option<NodeId>,
    pub type_name: String,
    pub params: Vec<Param>,
}

pub fn specs() -> Vec<CommandSpec> {
    vec![spec!(
        "inspect.componentCode",
        "Component code",
        "{id? (component, set or instance; default the selection), language?: react|leptos|swiftui|compose (default react)} → {component, name, set, properties: [{name, param, type, default, value? (instances)}], definition, usage? (instances; the connected snippet when the component is connected to code), connected: {importPath, name, template, import, code} | null}",
        false,
        |s, p| {
            let cmd = "inspect.componentCode";
            let id = targets(s, p).first().copied().ok_or_else(|| bad(cmd, "select a component or instance"))?;
            let lang = match string(p, "language").map(|l| CodeLang::from_id(&l)) {
                Some(Some(l @ (CodeLang::React | CodeLang::Leptos | CodeLang::SwiftUi | CodeLang::Compose))) => l,
                Some(_) => return Err(bad(cmd, "language: react|leptos|swiftui|compose")),
                None => CodeLang::React,
            };
            let doc = s.doc()?;
            let c = main_of(doc, id).and_then(|m| describe(doc, m)).ok_or_else(|| bad(cmd, "not a component, set or instance"))?;
            let values = instance_values(doc, id, &c);
            let properties: Vec<Value> = c
                .params
                .iter()
                .map(|p| {
                    let mut v = json!({"name": p.prop, "param": p.ident, "type": p.kind.id(), "default": p.default});
                    if let Some(vals) = &values {
                        v["value"] = vals.iter().find(|(name, _)| *name == p.prop).map_or(Value::Null, |(_, x)| x.clone());
                    }
                    v
                })
                .collect();
            let mut out = json!({
                "component": c.comp, "name": c.type_name, "set": c.set, "properties": properties,
                "definition": definition(doc, &c, lang), "language": lang.id(),
            });
            if values.is_some() {
                out["usage"] = json!(usage(doc, id, lang, 0));
            }
            out["connected"] = crate::code_connect::link(doc, id, lang).map_or(Value::Null, |l| {
                json!({"importPath": l.import_path, "name": l.name, "template": l.template, "import": crate::code_connect::import_line(l, lang),
                    "code": crate::code_connect::connected_code(doc, id, lang, l)})
            });
            Ok(out)
        }
    )]
}

/// Code for any layer in React, Leptos, SwiftUI or Compose: an instance's call and its component's
/// definition, a component's (or set's) definition, or the layer tree (React and Leptos wrap it
/// in a function component).
pub fn code(doc: &Document, id: NodeId, lang: CodeLang) -> String {
    let Some(n) = doc.node(id) else { return String::new() };
    if let Some(code) = crate::code_connect::link(doc, id, lang).and_then(|l| crate::code_connect::connected_code(doc, id, lang, l)) {
        return code;
    }
    let comp = main_of(doc, id).and_then(|m| describe(doc, m));
    match (&n.kind, comp) {
        (NodeKind::Instance { .. }, Some(c)) => format!("{}\n{}", usage(doc, id, lang, 0).unwrap_or_default(), definition(doc, &c, lang)),
        (NodeKind::Component { .. } | NodeKind::ComponentSet { .. }, Some(c)) => definition(doc, &c, lang),
        _ => match lang {
            CodeLang::React | CodeLang::Leptos => {
                let c = Component { comp: id, set: None, type_name: component_type_name(&n.name), params: Vec::new() };
                if lang == CodeLang::Leptos { leptos(doc, &c) } else { react(doc, &c) }
            }
            CodeLang::SwiftUi => {
                let mut out = String::new();
                swift(&Ctx::new(doc, id), id, 0, &mut out, &[]);
                out
            }
            _ => {
                let mut out = String::new();
                compose(&Ctx::new(doc, id), id, 0, &mut out);
                out
            }
        },
    }
}

fn is_component(doc: &Document, id: NodeId) -> bool {
    doc.node(id).is_some_and(|n| matches!(n.kind, NodeKind::Component { .. }))
}

/// The main component a layer's code is defined by: a component itself, an instance's main, and
/// for anything in a set the set's default (top-left) variant.
pub fn main_of(doc: &Document, id: NodeId) -> Option<NodeId> {
    let n = doc.node(id)?;
    let main = match &n.kind {
        NodeKind::Component { .. } => id,
        NodeKind::ComponentSet { .. } => return crate::components::default_variant(doc, id),
        NodeKind::Instance { component, .. } => *component,
        _ => return None,
    };
    if !is_component(doc, main) {
        return None;
    }
    match variant_set(doc, main) {
        Some(set) => crate::components::default_variant(doc, set),
        None => Some(main),
    }
}

/// `comp` (a component) as code.
pub fn describe(doc: &Document, comp: NodeId) -> Option<Component> {
    let n = doc.node(comp)?;
    let NodeKind::Component { props, .. } = &n.kind else { return None };
    let set = variant_set(doc, comp);
    let type_name = component_type_name(&set.and_then(|s| doc.node(s)).unwrap_or(n).name);
    let mut used: Vec<String> = Vec::new();
    let mut enums: Vec<String> = vec![type_name.clone()];
    let mut params: Vec<Param> = Vec::new();
    let mut add = |prop: String, kind: ParamKind, default: Value| {
        let enum_name = if matches!(kind, ParamKind::Variant(_)) { enum_ident(&prop, &mut enums) } else { String::new() };
        params.push(Param { ident: ident(&prop, &mut used), prop, kind, default, enum_name });
    };
    let mut seen: Vec<String> = Vec::new();
    if let Some(set) = set {
        let own = variant_props(&n.name);
        for (k, vals) in variant_options(doc, set) {
            let default = own.get(&k).or_else(|| vals.first()).cloned().unwrap_or_default();
            seen.push(k.clone());
            add(k, ParamKind::Variant(vals), Value::String(default));
        }
    }
    for (k, d) in props {
        if seen.contains(k) {
            continue;
        }
        let kind = match d {
            PropDef::Boolean { .. } => ParamKind::Boolean,
            PropDef::Text { .. } => ParamKind::Text,
            PropDef::InstanceSwap { .. } => ParamKind::InstanceSwap,
            PropDef::Variant { options, .. } => ParamKind::Variant(options.clone()),
            PropDef::Slot { .. } => ParamKind::Slot,
        };
        add(k.clone(), kind, prop_default_value(d));
    }
    Some(Component { comp, set, type_name, params })
}

// ------------------------------------------------------------------ names

/// ASCII words of a name ("Show icon?" → ["Show", "icon"]).
fn words(s: &str) -> Vec<&str> {
    s.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).take(12).collect()
}

fn capitalized(w: &str) -> String {
    let mut c = w.chars();
    c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
}

/// A type name: "Button / Primary" → `ButtonPrimary`.
pub fn type_name(name: &str) -> String {
    let t: String = words(name).into_iter().map(capitalized).collect::<String>().chars().take(60).collect();
    match t.chars().next() {
        None => "Component".into(),
        Some(c) if c.is_ascii_digit() => format!("Component{t}"),
        Some(_) => t,
    }
}

/// Words no generated parameter may be called (keywords of TypeScript, Swift and Kotlin, and
/// names the generated code uses).
const RESERVED: &[&str] = &[
    "as",
    "body",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "default",
    "defer",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "extension",
    "false",
    "for",
    "fun",
    "func",
    "function",
    "guard",
    "if",
    "import",
    "in",
    "init",
    "interface",
    "is",
    "key",
    "let",
    "modifier",
    "new",
    "nil",
    "null",
    "object",
    "operator",
    "package",
    "private",
    "protocol",
    "public",
    "ref",
    "repeat",
    "return",
    "self",
    "static",
    "struct",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typealias",
    "typeof",
    "val",
    "var",
    "void",
    "when",
    "where",
    "while",
    "with",
    "yield",
];

/// Types the generated SwiftUI and Compose code names: a variant enum may not shadow them.
const TYPES: &[&str] = &[
    "Alignment",
    "AnyView",
    "Arrangement",
    "Bool",
    "Boolean",
    "Box",
    "CaseIterable",
    "CircleShape",
    "Color",
    "Column",
    "Composable",
    "Edge",
    "EdgeInsets",
    "Ellipse",
    "EmptyView",
    "Font",
    "FontWeight",
    "Group",
    "HStack",
    "Image",
    "Modifier",
    "RectangleShape",
    "RoundedCornerShape",
    "RoundedRectangle",
    "Row",
    "Shape",
    "String",
    "Text",
    "Unit",
    "VStack",
    "View",
    "ZStack",
];

/// A component's name in code: [`type_name`], suffixed when it would shadow one of [`TYPES`]
/// ("Text" → `TextComponent`).
pub fn component_type_name(name: &str) -> String {
    let t = type_name(name);
    if TYPES.contains(&t.as_str()) { format!("{t}Component") } else { t }
}

/// A variant enum's name: "Size" → `Size`, unique in `used`, never one of [`TYPES`].
fn enum_ident(prop: &str, used: &mut Vec<String>) -> String {
    let t = type_name(prop);
    let base = if t == "Component" || TYPES.contains(&t.as_str()) { format!("{t}Variant") } else { t };
    let out = (1..).map(|k| if k == 1 { base.clone() } else { format!("{base}{k}") }).find(|n| !used.contains(n)).unwrap_or(base);
    used.push(out.clone());
    out
}

/// A parameter name: "Show icon" → `showIcon`, unique in `used`.
fn ident(name: &str, used: &mut Vec<String>) -> String {
    let mut base = String::new();
    for (i, w) in words(name).into_iter().enumerate() {
        base.push_str(&if i == 0 { w.to_ascii_lowercase() } else { capitalized(w) });
    }
    let mut base: String = base.chars().take(40).collect();
    if base.is_empty() {
        base = "prop".into();
    } else if base.starts_with(|c: char| c.is_ascii_digit()) {
        base = format!("p{base}");
    }
    if RESERVED.contains(&base.as_str()) {
        base.push_str("Value");
    }
    let out = (1..).map(|k| if k == 1 { base.clone() } else { format!("{base}{k}") }).find(|n| !used.contains(n)).unwrap_or(base);
    used.push(out.clone());
    out
}

/// Rust's keywords, and `children` (Leptos passes a component's children by that name).
const RUST_RESERVED: &[&str] = &[
    "abstract", "as", "async", "await", "become", "box", "break", "children", "const", "continue", "crate", "do", "dyn", "else", "enum", "extern",
    "false", "final", "fn", "for", "gen", "if", "impl", "in", "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub",
    "ref", "return", "self", "static", "struct", "super", "trait", "true", "try", "type", "typeof", "unsafe", "unsized", "use", "virtual", "where",
    "while", "yield",
];

/// A parameter's Rust name: its camelCase name in snake_case (`showIcon` → `show_icon`), with a
/// trailing `_` when Rust reserves it (`type_`; camelCase names have no `_`, so it stays unique).
pub(crate) fn rust_ident(ident: &str) -> String {
    let mut out = String::new();
    for (i, ch) in ident.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    if RUST_RESERVED.contains(&out.as_str()) {
        out.push('_');
    }
    out
}

/// A Rust type name: `Self` is taken.
fn rust_type(t: &str) -> String {
    if t == "Self" { "SelfComponent".into() } else { t.to_string() }
}

/// A Leptos variant enum's name: the component's name and the property's, never the props struct
/// `#[component]` makes (`ButtonProps`).
fn leptos_enum(c: &Component, p: &Param) -> String {
    let t = rust_type(&c.type_name);
    let name = format!("{t}{}", p.enum_name);
    if name == format!("{t}Props") { format!("{name}Variant") } else { name }
}

/// Rust enum variants for variant values: [`cases`] in PascalCase, `Self` taken.
fn rust_cases(values: &[String]) -> Vec<String> {
    cases(values, false).into_iter().map(|c| if c == "Self" { "SelfValue".into() } else { c }).collect()
}

/// Enum case names for variant values, unique: Swift's lowerCamel (`.small`, `` .`default` ``),
/// Kotlin's PascalCase (`Small`).
fn cases(values: &[String], swift: bool) -> Vec<String> {
    let mut used = Vec::new();
    values
        .iter()
        .map(|v| {
            if swift {
                let id = ident(v, &mut used);
                // Keyword cases stay readable in backticks rather than suffixed.
                match id.strip_suffix("Value").filter(|b| RESERVED.contains(b)) {
                    Some(b) => format!("`{b}`"),
                    None => id,
                }
            } else {
                let t = type_name(v);
                let t = if t == "Component" && !v.is_empty() && words(v).is_empty() { "Value".to_string() } else { t };
                let out = (1..).map(|k| if k == 1 { t.clone() } else { format!("{t}{k}") }).find(|n| !used.contains(n)).unwrap_or(t);
                used.push(out.clone());
                out
            }
        })
        .collect()
}

/// A Rust string literal.
pub(crate) fn rust_string(s: &str) -> String {
    format!("{:?}", s.chars().take(10_000).collect::<String>())
}

/// A JavaScript string literal.
pub(crate) fn js_string(s: &str) -> String {
    let s: String = s.chars().take(10_000).collect();
    serde_json::to_string(&s).unwrap_or_else(|_| "\"\"".into())
}

/// The type name of a component (a variant's set) by id.
pub(crate) fn component_type(doc: &Document, comp: NodeId) -> Option<String> {
    let n = doc.node(comp).filter(|n| matches!(n.kind, NodeKind::Component { .. } | NodeKind::ComponentSet { .. }))?;
    Some(component_type_name(&variant_set(doc, comp).and_then(|s| doc.node(s)).unwrap_or(n).name))
}

/// A Swift enum or Kotlin enum class name for a variant parameter.
fn enum_name(c: &Component, p: &Param, swift: bool) -> String {
    if swift { p.enum_name.clone() } else { format!("{}{}", c.type_name, p.enum_name) }
}

/// `value` of a parameter as an expression in `lang`.
pub(crate) fn value_expr(doc: &Document, c: &Component, p: &Param, value: &Value, lang: CodeLang) -> String {
    match (&p.kind, lang) {
        (ParamKind::Boolean, _) => value.as_bool().unwrap_or(false).to_string(),
        (ParamKind::Text, CodeLang::React) => js_string(value.as_str().unwrap_or("")),
        (ParamKind::Text, CodeLang::Leptos) => rust_string(value.as_str().unwrap_or("")),
        (ParamKind::Variant(opts), CodeLang::Leptos) => {
            let i = opts.iter().position(|o| Some(o.as_str()) == value.as_str()).unwrap_or(0);
            format!("{}::{}", leptos_enum(c, p), rust_cases(opts).get(i).cloned().unwrap_or_else(|| "Unknown".into()))
        }
        (ParamKind::Text, l) => literal(value.as_str().unwrap_or(""), l == CodeLang::Compose),
        (ParamKind::Variant(opts), l) => {
            let v = value.as_str().unwrap_or("");
            if l == CodeLang::React {
                return js_string(v);
            }
            let swift = l == CodeLang::SwiftUi;
            let i = opts.iter().position(|o| o == v).unwrap_or(0);
            let case = cases(opts, swift).get(i).cloned().unwrap_or_else(|| "unknown".into());
            if swift { format!(".{case}") } else { format!("{}.{case}", enum_name(c, p, false)) }
        }
        (ParamKind::InstanceSwap, l) => {
            let t = id_of(value).and_then(|i| component_type(doc, i));
            match (t, l) {
                (Some(t), CodeLang::React) => format!("<{t} />"),
                (Some(t), CodeLang::Leptos) => format!("|| view! {{ <{} /> }}", rust_type(&t)),
                (None, CodeLang::Leptos) => "|| ()".into(),
                (Some(t), CodeLang::SwiftUi) => format!("AnyView({t}())"),
                (Some(t), _) => format!("{{ {t}() }}"),
                (None, CodeLang::React) => "null".into(),
                (None, CodeLang::SwiftUi) => "AnyView(EmptyView())".into(),
                (None, _) => "{}".into(),
            }
        }
        (ParamKind::Slot, _) => "null".into(),
    }
}

// ------------------------------------------------------------------ instances

/// The values an instance sets for `c`'s parameters, in order (`None` for other layers): variant
/// values from the variant it shows, slots as whether they hold its own content.
pub(crate) fn instance_values(doc: &Document, id: NodeId, c: &Component) -> Option<Vec<(String, Value)>> {
    let NodeKind::Instance { component, props } = &doc.node(id)?.kind else { return None };
    let variant = doc.node(*component).map(|m| variant_props(&m.name)).unwrap_or_default();
    let slots = slots_of(doc, id);
    Some(
        c.params
            .iter()
            .map(|p| {
                let v = match &p.kind {
                    ParamKind::Variant(_) if c.set.is_some() => variant.get(&p.prop).map(|v| json!(v)),
                    ParamKind::Slot => Some(json!(slots.iter().any(|(l, name)| *name == p.prop && slot_edited(doc, *l)))),
                    _ => props.get(&p.prop).cloned(),
                };
                (p.prop.clone(), v.unwrap_or_else(|| p.default.clone()))
            })
            .collect(),
    )
}

/// The call for instance `id` at `depth`, passing the values that differ from the defaults and the
/// content of slots it fills (`None` when it isn't an instance of an existing component).
pub fn usage(doc: &Document, id: NodeId, lang: CodeLang, depth: usize) -> Option<String> {
    let c = main_of(doc, id).and_then(|m| describe(doc, m))?;
    let values = instance_values(doc, id, &c)?;
    let pad = if lang == CodeLang::React { "  ".repeat(depth) } else { indent(depth) };
    // A connected component: its real snippet. shortcut: inside a layer tree it brings no import
    // line; collect them at the top of the tree's code when people ask for it.
    if let Some(link) = crate::code_connect::link(doc, id, lang) {
        return Some(crate::code_connect::render(doc, &c, &values, link, lang, &pad));
    }
    let slots = slots_of(doc, id);
    // (parameter, expression, whether it's a plain string literal)
    let mut args: Vec<(&str, String, bool)> = Vec::new();
    for (p, (_, v)) in c.params.iter().zip(&values) {
        if let ParamKind::Slot = p.kind {
            if v.as_bool() == Some(true)
                && let Some((layer, _)) = slots.iter().find(|(_, name)| *name == p.prop)
            {
                args.push((&p.ident, slot_content(doc, *layer, lang, depth), false));
            }
            continue;
        }
        if *v == p.default || (p.kind == ParamKind::InstanceSwap && id_of(v) == id_of(&p.default)) {
            continue;
        }
        let expr = value_expr(doc, &c, p, v, lang);
        let plain = matches!(p.kind, ParamKind::Text | ParamKind::Variant(_)) && !expr.contains('\\');
        args.push((&p.ident, expr, plain));
    }
    let t = &c.type_name;
    Some(match lang {
        CodeLang::React | CodeLang::Leptos => {
            let leptos = lang == CodeLang::Leptos;
            // Plain strings read best as attribute values.
            let attrs: String = args
                .iter()
                .map(|(name, expr, plain)| {
                    let name = if leptos { rust_ident(name) } else { name.to_string() };
                    if *plain { format!(" {name}={expr}") } else { format!(" {name}={{{expr}}}") }
                })
                .collect();
            let t = if leptos { rust_type(t) } else { t.clone() };
            format!("{pad}<{t}{attrs} />\n")
        }
        CodeLang::SwiftUi => format!("{pad}{t}({})\n", args.iter().map(|(n, e, _)| format!("{n}: {e}")).collect::<Vec<_>>().join(", ")),
        _ => format!("{pad}{t}({})\n", args.iter().map(|(n, e, _)| format!("{n} = {e}")).collect::<Vec<_>>().join(", ")),
    })
}

/// The content an instance put in one of its slots, as an expression spanning lines.
fn slot_content(doc: &Document, slot: NodeId, lang: CodeLang, depth: usize) -> String {
    let Some(n) = doc.node(slot) else { return String::new() };
    let ctx = Ctx { doc, root: slot, comp: None, base: depth };
    let kids = ctx.kids(n);
    match lang {
        CodeLang::React => {
            let mut m = Markup::new(&ctx, CodeLang::React);
            for k in kids {
                m.node(k, Some(n), depth + 2, "");
            }
            let (p1, p2) = ("  ".repeat(depth), "  ".repeat(depth + 1));
            format!("(\n{p2}<>\n{}{p2}</>\n{p1})", m.markup)
        }
        CodeLang::Leptos => {
            let mut m = Markup::new(&ctx, CodeLang::Leptos);
            for k in kids {
                m.node(k, Some(n), depth + 1, "");
            }
            format!("|| view! {{\n{}{}}}", m.markup, indent(depth))
        }
        CodeLang::SwiftUi => {
            let mut out = String::new();
            for k in kids {
                swift(&ctx, k, depth + 2, &mut out, &[]);
            }
            format!("AnyView(Group {{\n{out}{}}})", indent(depth + 1))
        }
        _ => {
            let mut out = String::new();
            for k in kids {
                compose(&ctx, k, depth + 2, &mut out);
            }
            format!("{{\n{out}{}}}", indent(depth + 1))
        }
    }
}

// ------------------------------------------------------------------ definitions

/// `c`'s definition in `lang`.
pub fn definition(doc: &Document, c: &Component, lang: CodeLang) -> String {
    match lang {
        CodeLang::SwiftUi => swift_definition(doc, c),
        CodeLang::Compose => compose_definition(doc, c),
        CodeLang::Leptos => leptos(doc, c),
        _ => react(doc, c),
    }
}

/// `data-*` attribute names for variant parameters, unique among them (in order).
fn data_attrs(variants: &[&Param]) -> Vec<String> {
    let mut used = Vec::new();
    variants.iter().map(|p| format!("data-{}", class_name(&p.prop, &mut used))).collect()
}

/// `c`'s markup in React or Leptos (two levels in) and its stylesheet (variables as custom
/// properties, other variants as rules on their `data-*` attributes): (root class, markup, CSS).
fn web(doc: &Document, c: &Component, lang: CodeLang) -> (String, String, String) {
    let ctx = Ctx { doc, root: c.comp, comp: Some(c), base: 2 };
    let root_name = c.set.and_then(|s| doc.node(s)).or_else(|| doc.node(c.comp)).map(|n| n.name.clone()).unwrap_or_default();
    let root_class = class_name(&root_name, &mut Vec::new());
    let variants: Vec<&Param> = c.params.iter().filter(|p| matches!(p.kind, ParamKind::Variant(_))).collect();
    let attr_names = data_attrs(&variants);
    let pass = |root: NodeId| {
        let vctx = Ctx { doc, root, comp: Some(c), base: 2 };
        let mut m = Markup::new(&vctx, lang);
        m.root_class = Some(root_class.clone());
        m.node(root, None, 2, "");
        m.rules
    };
    let mut m = Markup::new(&ctx, lang);
    m.root_class = Some(root_class.clone());
    m.root_attrs = variants
        .iter()
        .zip(&attr_names)
        .map(|(p, attr)| match lang {
            CodeLang::Leptos => format!(" {attr}={{{}.as_str()}}", rust_ident(&p.ident)),
            _ => format!(" {attr}={{{}}}", p.ident),
        })
        .collect();
    m.node(c.comp, None, 2, "");
    // Other variants: what differs from this one, layer by layer.
    let mut variant_rules = String::new();
    let others: Vec<NodeId> = c.set.and_then(|s| doc.node(s)).map(|s| s.children.clone()).unwrap_or_default();
    for v in others.into_iter().filter(|v| *v != c.comp && is_component(doc, *v)).take(64) {
        let Some(vn) = doc.node(v) else { continue };
        let pairs = variant_props(&vn.name);
        let attrs: String = variants
            .iter()
            .zip(&attr_names)
            .filter_map(|(p, attr)| pairs.get(&p.prop).map(|val| format!("[{attr}=\"{}\"]", val.replace(['"', '\\'], ""))))
            .collect();
        for (path, _, decls) in pass(v) {
            let Some((_, class, base)) = m.rules.iter().find(|(p, _, _)| *p == path) else { continue };
            let base_lines: Vec<&str> = base.lines().collect();
            let diff: Vec<&str> = decls.lines().filter(|l| !base_lines.contains(l)).collect();
            if diff.is_empty() {
                continue;
            }
            let sel = if path.is_empty() { format!(".{root_class}{attrs}") } else { format!(".{root_class}{attrs} .{class}") };
            variant_rules.push_str(&format!("{sel} {{\n{}}}\n", diff.iter().map(|l| format!("  {l}\n")).collect::<String>()));
        }
    }
    let css = format!("{}{}{variant_rules}", m.tokens.root_rule(), m.styles);
    (root_class, m.markup, css)
}

/// A TSX function component with its props interface, and its stylesheet.
fn react(doc: &Document, c: &Component) -> String {
    let (root_class, markup, css) = web(doc, c, CodeLang::React);
    let t = &c.type_name;
    let mut out = format!("import \"./{root_class}.css\";\n\n");
    if c.params.is_empty() {
        out.push_str(&format!("export function {t}() {{\n"));
    } else {
        out.push_str(&format!("export interface {t}Props {{\n"));
        for p in &c.params {
            let ty = match &p.kind {
                ParamKind::Variant(opts) => opts.iter().map(|o| js_string(o)).collect::<Vec<_>>().join(" | "),
                ParamKind::Boolean => "boolean".into(),
                ParamKind::Text => "string".into(),
                ParamKind::InstanceSwap | ParamKind::Slot => "React.ReactNode".into(),
            };
            out.push_str(&format!("  {}?: {ty};\n", p.ident));
        }
        out.push_str("}\n\n");
        let args: Vec<String> = c
            .params
            .iter()
            .map(|p| match p.kind {
                ParamKind::Slot => p.ident.clone(),
                _ => format!("{} = {}", p.ident, value_expr(doc, c, p, &p.default, CodeLang::React)),
            })
            .collect();
        out.push_str(&format!("export function {t}({{\n{}}}: {t}Props) {{\n", args.iter().map(|a| format!("  {a},\n")).collect::<String>()));
    }
    out.push_str(&format!("  return (\n{markup}  );\n}}\n\n/* {root_class}.css */\n{css}"));
    out
}

/// A Leptos `#[component]` with its variant enums and props (defaults as `#[prop]` attributes),
/// and its stylesheet.
fn leptos(doc: &Document, c: &Component) -> String {
    let (root_class, markup, css) = web(doc, c, CodeLang::Leptos);
    let mut out = String::from("use leptos::prelude::*;\n\n");
    for p in &c.params {
        let ParamKind::Variant(opts) = &p.kind else { continue };
        let (e, cases) = (leptos_enum(c, p), rust_cases(opts));
        let default = opts.iter().position(|o| Some(o.as_str()) == p.default.as_str()).unwrap_or(0);
        out.push_str(&format!("#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]\npub enum {e} {{\n"));
        for (i, case) in cases.iter().enumerate() {
            out.push_str(&format!("{}    {case},\n", if i == default { "    #[default]\n" } else { "" }));
        }
        out.push_str(&format!("}}\n\nimpl {e} {{\n    /// The variant's name, as its `data-*` attribute.\n    pub fn as_str(self) -> &'static str {{\n        match self {{\n"));
        for (o, case) in opts.iter().zip(&cases) {
            out.push_str(&format!("            Self::{case} => {},\n", rust_string(o)));
        }
        out.push_str("        }\n    }\n}\n\n");
    }
    let t = rust_type(&c.type_name);
    out.push_str(&format!("#[component]\npub fn {t}("));
    if !c.params.is_empty() {
        out.push('\n');
        for p in &c.params {
            let default = value_expr(doc, c, p, &p.default, CodeLang::Leptos);
            let (attr, ty) = match &p.kind {
                ParamKind::Variant(_) => ("optional".to_string(), leptos_enum(c, p)),
                ParamKind::Boolean => (format!("default = {default}"), "bool".into()),
                ParamKind::Text => (format!("into, default = {default}.to_string()"), "String".into()),
                ParamKind::InstanceSwap => (format!("into, default = ({default}).into()"), "ViewFn".into()),
                ParamKind::Slot => ("optional, into".to_string(), "Option<ViewFn>".into()),
            };
            out.push_str(&format!("    #[prop({attr})] {}: {ty},\n", rust_ident(&p.ident)));
        }
    }
    out.push_str(&format!(") -> impl IntoView {{\n    view! {{\n{markup}    }}\n}}\n\n/* {root_class}.css */\n{css}"));
    out
}

/// A SwiftUI view struct: variant enums, stored properties with defaults, and the body.
fn swift_definition(doc: &Document, c: &Component) -> String {
    let ctx = Ctx { doc, root: c.comp, comp: Some(c), base: 2 };
    let mut out = format!("struct {}: View {{\n", c.type_name);
    for p in &c.params {
        if let ParamKind::Variant(opts) = &p.kind {
            out.push_str(&format!("    enum {}: String, CaseIterable {{\n", enum_name(c, p, true)));
            for (o, case) in opts.iter().zip(cases(opts, true)) {
                out.push_str(&format!("        case {case} = {}\n", literal(o, false)));
            }
            out.push_str("    }\n");
        }
    }
    for p in &c.params {
        let ty = match &p.kind {
            ParamKind::Variant(_) => enum_name(c, p, true),
            ParamKind::Boolean => "Bool".into(),
            ParamKind::Text => "String".into(),
            ParamKind::InstanceSwap => "AnyView".into(),
            ParamKind::Slot => "AnyView?".into(),
        };
        let default = if p.kind == ParamKind::Slot { "nil".into() } else { value_expr(doc, c, p, &p.default, CodeLang::SwiftUi) };
        out.push_str(&format!("    var {}: {ty} = {default}\n", p.ident));
    }
    if !c.params.is_empty() {
        out.push('\n');
    }
    out.push_str("    var body: some View {\n");
    swift(&ctx, c.comp, 2, &mut out, &[]);
    out.push_str("    }\n}\n");
    out
}

/// A `@Composable` function: variant enum classes, parameters with defaults, and the body.
fn compose_definition(doc: &Document, c: &Component) -> String {
    let ctx = Ctx { doc, root: c.comp, comp: Some(c), base: 1 };
    let mut out = String::new();
    for p in &c.params {
        if let ParamKind::Variant(opts) = &p.kind {
            out.push_str(&format!("enum class {} {{ {} }}\n\n", enum_name(c, p, false), cases(opts, false).join(", ")));
        }
    }
    out.push_str(&format!("@Composable\nfun {}(", c.type_name));
    if !c.params.is_empty() {
        out.push('\n');
        for p in &c.params {
            let ty = match &p.kind {
                ParamKind::Variant(_) => enum_name(c, p, false),
                ParamKind::Boolean => "Boolean".into(),
                ParamKind::Text => "String".into(),
                ParamKind::InstanceSwap => "@Composable () -> Unit".into(),
                ParamKind::Slot => "(@Composable () -> Unit)?".into(),
            };
            let default = if p.kind == ParamKind::Slot { "null".into() } else { value_expr(doc, c, p, &p.default, CodeLang::Compose) };
            out.push_str(&format!("    {}: {ty} = {default},\n", p.ident));
        }
    }
    out.push_str(") {\n");
    compose(&ctx, c.comp, 1, &mut out);
    out.push_str("}\n");
    out
}
