//! Component code: a component (or component set) becomes a React, Leptos, Vue, SwiftUI, Jetpack
//! Compose, ArkUI or Flutter component whose properties are its parameters (a variant property → a
//! union or enum, boolean → a flag showing the layers bound to it, text → a string, instance swap →
//! a view parameter, slot → optional content), and an instance becomes a call passing the values it
//! sets. Variants of a set share one definition (the set's default variant); in React and Leptos
//! their differences become CSS rules on `data-*` attributes. Returned by `inspect.componentCode`
//! and used by `inspect.code`. A component connected to code (see [`crate::code_connect`]) shows
//! its real snippet instead.

use figcraft_doc::{Document, NodeId, NodeKind, PropDef};
use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::codegen::{CodeLang, Ctx, FLUTTER_IMPORT, Markup, Place, arkui, class_name, compose, flutter, flutter_indent, indent, literal, swift};
use crate::components::{prop_default_value, slot_edited, slots_of, variant_options, variant_props, variant_set};
use crate::params::{id_of, string, targets};
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
        "{id? (component, set or instance; default the selection), language?: react|leptos|vue|swiftui|compose|arkui|flutter (default react)} → {component, name, set, properties: [{name, param, type, default, value? (instances)}], definition, usage? (instances; the connected snippet when the component is connected to code), connected: {importPath, name, template, import, code} | null}",
        false,
        |s, p| {
            let cmd = "inspect.componentCode";
            let id = targets(s, p).first().copied().ok_or_else(|| bad(cmd, "select a component or instance"))?;
            let lang = match string(p, "language") {
                // One list of code languages, shared with codeConnect (see `code_connect::language`).
                Some(l) => crate::code_connect::language(&l).ok_or_else(|| bad(cmd, "language: react|leptos|vue|swiftui|compose|arkui|flutter"))?,
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

/// Code for any layer in React, Leptos, Vue, SwiftUI, Compose, ArkUI or Flutter: an instance's call
/// and its component's definition, a component's (or set's) definition, or the layer tree (React
/// and Leptos wrap it in a function component), with the imports of the connected components it
/// calls at the top.
pub fn code(doc: &Document, id: NodeId, lang: CodeLang) -> String {
    let Some(n) = doc.node(id) else { return String::new() };
    if let Some(code) = crate::code_connect::link(doc, id, lang).and_then(|l| crate::code_connect::connected_code(doc, id, lang, l)) {
        return code;
    }
    let comp = main_of(doc, id).and_then(|m| describe(doc, m));
    let code = match (&n.kind, comp) {
        (NodeKind::Instance { .. }, Some(c)) => {
            let call = usage(doc, id, lang, 0).unwrap_or_default();
            let def = definition(doc, &c, lang);
            match def.strip_prefix(FLUTTER_IMPORT) {
                // Dart's imports come before the code that uses them.
                Some(rest) if lang == CodeLang::Flutter => format!("{FLUTTER_IMPORT}{call}\n{rest}"),
                _ => format!("{call}\n{def}"),
            }
        }
        (NodeKind::Component { .. } | NodeKind::ComponentSet { .. }, Some(c)) => definition(doc, &c, lang),
        _ => match lang {
            CodeLang::React | CodeLang::Leptos | CodeLang::Vue => {
                let c = Component { comp: id, set: None, type_name: component_type_name(&n.name), params: Vec::new() };
                match lang {
                    CodeLang::Leptos => leptos(doc, &c),
                    CodeLang::Vue => vue(doc, &c),
                    _ => react(doc, &c),
                }
            }
            CodeLang::SwiftUi => {
                let mut out = String::new();
                swift(&Ctx::new(doc, id), id, 0, &mut out, &[]);
                out
            }
            CodeLang::ArkUi => {
                let mut out = String::new();
                arkui(&Ctx::new(doc, id), id, 0, &mut out, &[]);
                out
            }
            CodeLang::Flutter => {
                let mut out = String::new();
                flutter(&Ctx::new(doc, id), id, 0, &mut out, Place::Flow);
                out
            }
            _ => {
                let mut out = String::new();
                compose(&Ctx::new(doc, id), id, 0, &mut out);
                out
            }
        },
    };
    let imports = crate::code_connect::tree_imports(doc, id, lang);
    if lang == CodeLang::Vue && !imports.is_empty() {
        return vue_with_imports(&imports, code);
    }
    crate::code_connect::with_imports(&imports, code)
}

/// A Vue single-file component imports in its `<script setup>`, which is added when it has none.
fn vue_with_imports(imports: &[String], code: String) -> String {
    const SETUP: &str = "<script setup lang=\"ts\">\n\n";
    let lines = imports.join("\n");
    // `find` returns char boundaries, so the splits below are safe.
    if let Some(at) = code.find(SETUP).map(|at| at + SETUP.len()) {
        let (head, tail) = code.split_at(at);
        return format!("{head}{lines}\n\n{tail}");
    }
    match code.find("<template>") {
        Some(at) => {
            let (head, tail) = code.split_at(at);
            format!("{head}{SETUP}{lines}\n\n</script>\n\n{tail}")
        }
        None => crate::code_connect::with_imports(imports, code),
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
    "arguments",
    "as",
    "await",
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
    "eval",
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
    "implements",
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
    "protected",
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

/// Types the generated SwiftUI, Compose, ArkUI and Flutter code names: a variant enum may not shadow them.
const TYPES: &[&str] = &[
    "Alignment",
    "AnyView",
    "Arrangement",
    "Bool",
    "Boolean",
    "Border",
    "BorderRadius",
    "BorderSide",
    "Box",
    "BoxDecoration",
    "BoxShadow",
    "BoxShape",
    "BuildContext",
    "CaseIterable",
    "Circle",
    "CircleShape",
    "Color",
    "Column",
    "Composable",
    "CrossAxisAlignment",
    "Edge",
    "EdgeInsets",
    "Ellipse",
    "EmptyView",
    "Expanded",
    "FlexAlign",
    "Font",
    "FontStyle",
    "FontWeight",
    "Group",
    "HStack",
    "HorizontalAlign",
    "Image",
    "MainAxisAlignment",
    "Modifier",
    "Offset",
    "Opacity",
    "Positioned",
    "Rect",
    "RectangleShape",
    "RoundedCornerShape",
    "RoundedRectangle",
    "Row",
    "Shape",
    "SizedBox",
    "Stack",
    "StatelessWidget",
    "String",
    "Text",
    "TextAlign",
    "TextBaseline",
    "TextCase",
    "TextDecoration",
    "TextDecorationType",
    "TextStyle",
    "Unit",
    "VStack",
    "VerticalAlign",
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

/// Enum member names for variant values in Dart: lowerCamel, unique, never a keyword (a `default`
/// value becomes `defaultValue`).
fn dart_cases(values: &[String]) -> Vec<String> {
    let mut used = Vec::new();
    values.iter().map(|v| ident(v, &mut used)).collect()
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

/// A JavaScript string literal safe inside a Vue single-file component: `<` is escaped, so a
/// default containing `</script>` can't close the `<script setup>` block early.
pub(crate) fn vue_string(s: &str) -> String {
    js_string(s).replace('<', "\\u003C")
}

/// A plain Vue attribute value: its JS string quotes stay as the attribute delimiters, and HTML
/// entities inside are escaped so the value reads back unchanged.
fn html_attr_value(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// A Vue binding expression inside a double-quoted attribute.
fn html_attr_expr(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
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
    // React and Vue both take JavaScript: strings, and variant values as plain strings.
    let js = matches!(lang, CodeLang::React | CodeLang::Vue);
    match (&p.kind, lang) {
        (ParamKind::Boolean, _) => value.as_bool().unwrap_or(false).to_string(),
        (ParamKind::Text, CodeLang::Vue) => vue_string(value.as_str().unwrap_or("")),
        (ParamKind::Text, CodeLang::React) => js_string(value.as_str().unwrap_or("")),
        (ParamKind::Text, CodeLang::Leptos) => rust_string(value.as_str().unwrap_or("")),
        (ParamKind::Variant(opts), CodeLang::Leptos) => {
            let i = opts.iter().position(|o| Some(o.as_str()) == value.as_str()).unwrap_or(0);
            format!("{}::{}", leptos_enum(c, p), rust_cases(opts).get(i).cloned().unwrap_or_else(|| "Unknown".into()))
        }
        (ParamKind::Text, l) => literal(value.as_str().unwrap_or(""), l.escapes_dollar()),
        (ParamKind::Variant(opts), l) => {
            let v = value.as_str().unwrap_or("");
            if l == CodeLang::Vue {
                return vue_string(v);
            }
            if l == CodeLang::React {
                return js_string(v);
            }
            let i = opts.iter().position(|o| o == v).unwrap_or(0);
            match l {
                CodeLang::SwiftUi => format!(".{}", cases(opts, true).get(i).cloned().unwrap_or_else(|| "unknown".into())),
                CodeLang::Flutter => format!("{}.{}", enum_name(c, p, false), dart_cases(opts).get(i).cloned().unwrap_or_else(|| "unknown".into())),
                _ => format!("{}.{}", enum_name(c, p, false), cases(opts, false).get(i).cloned().unwrap_or_else(|| "Unknown".into())),
            }
        }
        (ParamKind::InstanceSwap, l) => {
            let t = id_of(value).and_then(|i| component_type(doc, i));
            match (t, l) {
                // A Vue binding takes the component itself, not an element.
                (Some(t), CodeLang::Vue) => t,
                (Some(t), _) if js => format!("<{t} />"),
                (Some(t), CodeLang::Leptos) => format!("|| view! {{ <{} /> }}", rust_type(&t)),
                (None, CodeLang::Leptos) => "|| ()".into(),
                (Some(t), CodeLang::SwiftUi) => format!("AnyView({t}())"),
                (Some(t), CodeLang::Flutter) => format!("{t}()"),
                // ArkUI takes a builder: `@BuilderParam` is a `() => void`.
                (Some(t), CodeLang::ArkUi) => format!("() => {{ {t}() }}"),
                (None, CodeLang::ArkUi) => "() => {}".into(),
                (Some(t), _) => format!("{{ {t}() }}"),
                (None, _) if js => "null".into(),
                (None, CodeLang::SwiftUi) => "AnyView(EmptyView())".into(),
                (None, CodeLang::Flutter) => "const SizedBox.shrink()".into(),
                (None, _) => "{}".into(),
            }
        }
        (ParamKind::Slot, _) => "null".into(),
    }
}

// ------------------------------------------------------------------ instances

/// The `<slot name="…">` of a slot parameter: unique among the component's slot parameters, unlike
/// `class_name` called with a fresh list per parameter (two idents can lowercase alike).
pub(crate) fn slot_name(c: &Component, p: &Param) -> String {
    let mut used = Vec::new();
    for q in c.params.iter().filter(|q| matches!(q.kind, ParamKind::Slot)) {
        let name = class_name(&q.ident, "slot", &mut used);
        if q.ident == p.ident {
            return name;
        }
    }
    class_name(&p.ident, "slot", &mut Vec::new())
}

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
    usage_with_styles(doc, id, lang, depth).map(|(code, _)| code)
}

/// `usage` and the CSS generated for the slot content it carries (the caller's stylesheet needs
/// those rules; `usage` alone drops them at a snippet boundary).
pub(crate) fn usage_with_styles(doc: &Document, id: NodeId, lang: CodeLang, depth: usize) -> Option<(String, String)> {
    let c = main_of(doc, id).and_then(|m| describe(doc, m))?;
    let values = instance_values(doc, id, &c)?;
    let pad = if matches!(lang, CodeLang::React | CodeLang::Vue | CodeLang::Flutter) { "  ".repeat(depth) } else { indent(depth) };
    // A connected component: its real snippet (its import is at the top of the code: `code`).
    if let Some(link) = crate::code_connect::link(doc, id, lang) {
        return Some((crate::code_connect::render(doc, &c, &values, link, lang, &pad), String::new()));
    }
    let slots = slots_of(doc, id);
    // (parameter, expression, whether it's a plain string literal)
    let mut args: Vec<(&str, String, bool)> = Vec::new();
    let mut slot_css = String::new();
    for (p, (_, v)) in c.params.iter().zip(&values) {
        if let ParamKind::Slot = p.kind {
            if v.as_bool() == Some(true)
                && let Some((layer, _)) = slots.iter().find(|(_, name)| *name == p.prop)
            {
                let prefix = format!("slot-{}-", slot_name(&c, p));
                let (content, css) = slot_content(doc, *layer, lang, depth, &prefix);
                slot_css.push_str(&css);
                args.push((&p.ident, content, false));
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
    let code = match lang {
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
        CodeLang::Vue => {
            // A plain string is an attribute value and anything else a `:name` binding, named as
            // the definition declares the prop; a slot is a `<template #name>` child instead,
            // named as its `<slot>` element is.
            let is_slot = |name: &str| c.params.iter().any(|p| p.ident == name && matches!(p.kind, ParamKind::Slot));
            let attrs: String = args
                .iter()
                .filter(|(name, _, _)| !is_slot(name))
                .map(
                    |(name, expr, plain)| {
                        if *plain { format!(" {name}={}", html_attr_value(expr)) } else { format!(" :{name}=\"{}\"", html_attr_expr(expr)) }
                    },
                )
                .collect();
            let slots: String = args
                .iter()
                .filter(|(name, _, _)| is_slot(name))
                .map(|(name, expr, _)| {
                    let slot = c.params.iter().find(|q| q.ident == *name);
                    let name = slot.map(|p| slot_name(&c, p)).unwrap_or_else(|| class_name(name, "slot", &mut Vec::new()));
                    format!("{pad}  <template #{name}>\n{expr}{pad}  </template>\n")
                })
                .collect();
            if slots.is_empty() { format!("{pad}<{t}{attrs} />\n") } else { format!("{pad}<{t}{attrs}>\n{slots}{pad}</{t}>\n") }
        }
        CodeLang::SwiftUi | CodeLang::Flutter => {
            format!("{pad}{t}({})\n", args.iter().map(|(n, e, _)| format!("{n}: {e}")).collect::<Vec<_>>().join(", "))
        }
        CodeLang::ArkUi => {
            let list = args.iter().map(|(n, e, _)| format!("{n}: {e}")).collect::<Vec<_>>().join(", ");
            // ArkTS calls take an object literal, or nothing at all.
            if list.is_empty() { format!("{pad}{t}()\n") } else { format!("{pad}{t}({{ {list} }})\n") }
        }
        _ => format!("{pad}{t}({})\n", args.iter().map(|(n, e, _)| format!("{n} = {e}")).collect::<Vec<_>>().join(", ")),
    };
    Some((code, slot_css))
}

/// The content an instance put in one of its slots, as an expression spanning lines, and the CSS
/// its layers generated (so the caller's stylesheet keeps it). `prefix` keeps two slots' class
/// names apart when their layers share a name.
fn slot_content(doc: &Document, slot: NodeId, lang: CodeLang, depth: usize, prefix: &str) -> (String, String) {
    let Some(n) = doc.node(slot) else { return (String::new(), String::new()) };
    let ctx = Ctx { doc, root: slot, comp: None, base: depth };
    let kids = ctx.kids(n);
    match lang {
        CodeLang::React => {
            let mut m = Markup::new(&ctx, CodeLang::React);
            m.class_prefix = Some(prefix.to_string());
            for k in kids {
                m.node(k, Some(n), depth + 2, "");
            }
            let (p1, p2) = ("  ".repeat(depth), "  ".repeat(depth + 1));
            (format!("(\n{p2}<>\n{}{p2}</>\n{p1})", m.markup), m.styles)
        }
        CodeLang::Leptos => {
            let mut m = Markup::new(&ctx, CodeLang::Leptos);
            m.class_prefix = Some(prefix.to_string());
            for k in kids {
                m.node(k, Some(n), depth + 1, "");
            }
            (format!("|| view! {{\n{}{}}}", m.markup, indent(depth)), m.styles)
        }
        CodeLang::Vue => {
            // Just the markup: `usage` wraps it in the `<template #name>` of that slot.
            let mut m = Markup::new(&ctx, CodeLang::Vue);
            m.class_prefix = Some(prefix.to_string());
            for k in kids {
                m.node(k, Some(n), depth + 2, "");
            }
            (m.markup, m.styles)
        }
        CodeLang::SwiftUi => {
            let mut out = String::new();
            for k in kids {
                swift(&ctx, k, depth + 2, &mut out, &[]);
            }
            (format!("AnyView(Group {{\n{out}{}}})", indent(depth + 1)), String::new())
        }
        CodeLang::ArkUi => {
            let mut out = String::new();
            for k in kids {
                arkui(&ctx, k, depth + 2, &mut out, &[]);
            }
            (format!("() => {{\n{out}{}}}", indent(depth + 1)), String::new())
        }
        CodeLang::Flutter => {
            // A slot holds a list of widgets, so it takes however many layers the slot has.
            let mut out = String::new();
            for k in kids {
                flutter(&ctx, k, depth + 1, &mut out, Place::Flow);
                out.push_str(",\n");
            }
            (format!("[\n{out}{}]", flutter_indent(depth)), String::new())
        }
        _ => {
            let mut out = String::new();
            for k in kids {
                compose(&ctx, k, depth + 2, &mut out);
            }
            (format!("{{\n{out}{}}}", indent(depth + 1)), String::new())
        }
    }
}

// ------------------------------------------------------------------ definitions

/// `c`'s definition in `lang`.
pub fn definition(doc: &Document, c: &Component, lang: CodeLang) -> String {
    match lang {
        CodeLang::SwiftUi => swift_definition(doc, c),
        CodeLang::Compose => compose_definition(doc, c),
        CodeLang::Flutter => flutter_definition(doc, c),
        CodeLang::Leptos => leptos(doc, c),
        CodeLang::Vue => vue(doc, c),
        CodeLang::ArkUi => arkui_definition(doc, c),
        _ => react(doc, c),
    }
}

/// `data-*` attribute names for variant parameters, unique among them (in order).
fn data_attrs(variants: &[&Param]) -> Vec<String> {
    let mut used = Vec::new();
    variants.iter().map(|p| format!("data-{}", class_name(&p.prop, "prop", &mut used))).collect()
}

/// `c`'s markup in React or Leptos (two levels in) and its stylesheet (variables as custom
/// properties, other variants as rules on their `data-*` attributes): (root class, markup, CSS).
fn web(doc: &Document, c: &Component, lang: CodeLang) -> (String, String, String) {
    let ctx = Ctx { doc, root: c.comp, comp: Some(c), base: 2 };
    let root_name = c.set.and_then(|s| doc.node(s)).or_else(|| doc.node(c.comp)).map(|n| n.name.clone()).unwrap_or_default();
    let root_class = class_name(&root_name, "component", &mut Vec::new());
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
            CodeLang::Vue => format!(" :{attr}=\"{}\"", p.ident),
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

/// A Vue single-file component: `<script setup>` with typed props and their defaults, the
/// template, and a scoped stylesheet. Slots become Vue's own `<slot>` elements, so they are not
/// props. Variants switch on the `data-*` attributes of the root element, as on the web.
fn vue(doc: &Document, c: &Component) -> String {
    // The stylesheet sits inline in the SFC, so the root class name isn't needed here.
    let (_, markup, css) = web(doc, c, CodeLang::Vue);
    // A slot layer is a `<slot>` element in the template, so it takes no prop.
    let props: Vec<&Param> = c.params.iter().filter(|p| !matches!(p.kind, ParamKind::Slot)).collect();
    let mut out = String::new();
    if !props.is_empty() {
        // `__props` can't be a generated prop name (idents are camelCase words), so a property
        // named `Props` isn't shadowed by the binding.
        out.push_str("<script setup lang=\"ts\">\n\nconst __props = withDefaults(\n  defineProps<{\n");
        for p in &props {
            let ty = match &p.kind {
                ParamKind::Variant(opts) => opts.iter().map(|o| vue_string(o)).collect::<Vec<_>>().join(" | "),
                ParamKind::Boolean => "boolean".into(),
                ParamKind::Text => "string".into(),
                ParamKind::InstanceSwap | ParamKind::Slot => "unknown".into(),
            };
            out.push_str(&format!("    {}?: {ty};\n", p.ident));
        }
        out.push_str("  }>(),\n  {\n");
        for p in &props {
            // An instance swap defaults to its component reference; a slot isn't in `props`.
            out.push_str(&format!("    {}: {},\n", p.ident, value_expr(doc, c, p, &p.default, CodeLang::Vue)));
        }
        out.push_str("  },\n);\n\n</script>\n\n");
    }
    out.push_str(&format!("<template>\n{markup}</template>\n\n<style scoped>\n{css}</style>\n"));
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

/// A Flutter `StatelessWidget`: variant enums, a const constructor with defaults, and the widget
/// tree in `build`.
fn flutter_definition(doc: &Document, c: &Component) -> String {
    let ctx = Ctx { doc, root: c.comp, comp: Some(c), base: 2 };
    let mut out = String::from(FLUTTER_IMPORT);
    for p in &c.params {
        if let ParamKind::Variant(opts) = &p.kind {
            out.push_str(&format!("enum {} {{ {} }}\n\n", enum_name(c, p, false), dart_cases(opts).join(", ")));
        }
    }
    let t = &c.type_name;
    out.push_str(&format!("class {t} extends StatelessWidget {{\n"));
    if c.params.is_empty() {
        out.push_str(&format!("  const {t}({{super.key}});\n"));
    } else {
        out.push_str(&format!("  const {t}({{\n    super.key,\n"));
        for p in &c.params {
            match flutter_default(doc, c, p) {
                Some(d) => out.push_str(&format!("    this.{} = {d},\n", p.ident)),
                None => out.push_str(&format!("    this.{},\n", p.ident)),
            }
        }
        out.push_str("  });\n\n");
        for p in &c.params {
            out.push_str(&format!("  final {} {};\n", flutter_type(c, p), p.ident));
        }
    }
    out.push_str("\n  @override\n  Widget build(BuildContext context) {\n    return ");
    let mut body = String::new();
    flutter(&ctx, c.comp, 2, &mut body, Place::Flow);
    out.push_str(body.trim_start());
    out.push_str(";\n  }\n}\n");
    out
}

/// A parameter's Dart type.
fn flutter_type(c: &Component, p: &Param) -> String {
    match &p.kind {
        ParamKind::Variant(_) => enum_name(c, p, false),
        ParamKind::Boolean => "bool".into(),
        ParamKind::Text => "String".into(),
        ParamKind::InstanceSwap => "Widget".into(),
        // A slot holds however many layers it was filled with.
        ParamKind::Slot => "List<Widget>?".into(),
    }
}

/// A parameter's default as a Dart constant (`None` for a slot, which has none): Dart wants its
/// defaults to be constants, so an instance swap is a `const` widget.
fn flutter_default(doc: &Document, c: &Component, p: &Param) -> Option<String> {
    match &p.kind {
        ParamKind::Slot => None,
        ParamKind::InstanceSwap => Some(match id_of(&p.default).and_then(|i| component_type(doc, i)) {
            Some(t) => format!("const {t}()"),
            None => "const SizedBox.shrink()".into(),
        }),
        _ => Some(value_expr(doc, c, p, &p.default, CodeLang::Flutter)),
    }
}

/// An ArkUI (`@Component`) struct: variant enums, `@Prop` parameters, a default builder for each
/// instance swap and slot, and the `build` body.
fn arkui_definition(doc: &Document, c: &Component) -> String {
    let ctx = Ctx { doc, root: c.comp, comp: Some(c), base: 1 };
    let mut out = String::new();
    for p in &c.params {
        if let ParamKind::Variant(opts) = &p.kind {
            out.push_str(&format!("export enum {} {{ {} }}\n\n", enum_name(c, p, false), cases(opts, false).join(", ")));
        }
    }
    // A default builder per instance swap or slot, named apart from every parameter.
    let mut used: Vec<String> = c.params.iter().map(|p| p.ident.clone()).collect();
    let mut builders: Vec<(String, String)> = Vec::new();
    for p in &c.params {
        if !matches!(p.kind, ParamKind::InstanceSwap | ParamKind::Slot) {
            continue;
        }
        let base = format!("default{}Builder", capitalized(&p.ident));
        let name = (1..).map(|k| if k == 1 { base.clone() } else { format!("{base}{k}") }).find(|n| !used.contains(n)).unwrap_or(base);
        used.push(name.clone());
        builders.push((p.ident.clone(), name));
    }
    out.push_str("@Component\nexport struct ");
    out.push_str(&c.type_name);
    out.push_str(" {\n");
    for p in &c.params {
        match &p.kind {
            ParamKind::Variant(_) => {
                out.push_str(&format!("    @Prop {}: {} = {}\n", p.ident, enum_name(c, p, false), value_expr(doc, c, p, &p.default, CodeLang::ArkUi)))
            }
            ParamKind::Boolean => out.push_str(&format!("    @Prop {}: boolean = {}\n", p.ident, p.default.as_bool().unwrap_or(false))),
            ParamKind::Text => out.push_str(&format!("    @Prop {}: string = {}\n", p.ident, value_expr(doc, c, p, &p.default, CodeLang::ArkUi))),
            ParamKind::InstanceSwap | ParamKind::Slot => {
                let name = builders.iter().find(|(ident, _)| *ident == p.ident).map_or("", |(_, name)| name.as_str());
                out.push_str(&format!("    @BuilderParam {}: () => void = this.{name}\n", p.ident));
            }
        }
    }
    for (ident, name) in &builders {
        let Some(p) = c.params.iter().find(|p| p.ident == *ident) else { continue };
        out.push_str(&format!("\n    @Builder\n    {name}() {{\n"));
        match &p.kind {
            ParamKind::InstanceSwap => {
                if let Some(t) = id_of(&p.default).and_then(|i| component_type(doc, i)) {
                    out.push_str(&format!("        {t}()\n"));
                }
            }
            ParamKind::Slot => {
                let layer = slots_of(doc, c.comp).into_iter().find(|(_, slot)| slot == &p.prop).map(|(layer, _)| layer);
                let kids = layer.and_then(|l| doc.node(l)).map(|n| ctx.kids(n)).unwrap_or_default();
                for k in kids {
                    arkui(&ctx, k, 2, &mut out, &[]);
                }
            }
            _ => {}
        }
        out.push_str("    }\n");
    }
    out.push_str("\n    build() {\n");
    arkui(&ctx, c.comp, 2, &mut out, &[]);
    out.push_str("    }\n}\n");
    out
}
