//! Code for a layer tree in the web languages the Inspect panel offers besides CSS, HTML,
//! React and Leptos: Tailwind utility classes, SCSS, Vue, Angular, Svelte and
//! styled-components. All of them read the same declarations the CSS generator writes (see
//! [`crate::codegen::css_props`]), so a layer looks the same in every one of them.

use figcraft_doc::*;

use crate::codegen::{MAX_DEPTH, Tokens, class_name, css_props, html_escape, in_flow, placement};

/// A layer of the tree with the declarations a stylesheet would give it.
struct Style {
    /// The class a stylesheet would use.
    class: String,
    /// The layer's name.
    name: String,
    /// The name a component (styled-components) uses.
    cname: String,
    /// The layer's declarations.
    decls: Vec<(String, String)>,
    /// The layer's children, in paint order.
    children: Vec<Style>,
    /// The text of a text layer.
    text: Option<String>,
}

/// The tree under `id`, with the declarations and the class names a stylesheet would use.
fn build(doc: &Document, id: NodeId, parent: Option<&Node>, root: bool, depth: usize, used: &mut Vec<String>, tokens: &mut Tokens) -> Option<Style> {
    let n = doc.node(id)?;
    let class = class_name(&n.name, used);
    let hugs = n.layout.is_some() || matches!(n.kind, NodeKind::Text(_));
    let hug = (hugs && n.child_layout.sizing_h == Sizing::Hug, hugs && n.child_layout.sizing_v == Sizing::Hug);
    let mut decls = placement(n, parent, root);
    decls.extend(css_props(doc, n, tokens, hug));
    if n.layout.is_none() && !n.children.is_empty() && !root && in_flow(n, parent) {
        decls.push(("position".into(), "relative".into()));
    }
    let text = match &n.kind {
        NodeKind::Text(t) => Some(t.characters.clone()),
        _ => None,
    };
    let mut children = Vec::new();
    if depth < MAX_DEPTH {
        for k in n.children.iter().copied().filter(|c| doc.node(*c).is_some_and(|k| k.visible)) {
            if let Some(child) = build(doc, k, Some(n), false, depth + 1, used, tokens) {
                children.push(child);
            }
        }
    }
    Some(Style { class, name: n.name.clone(), cname: String::new(), decls, children, text })
}

/// The tree under `id`, with the declarations and the class names a stylesheet would use.
fn tree(doc: &Document, id: NodeId) -> Option<(Style, Tokens)> {
    let mut used = Vec::new();
    let mut tokens = Tokens::default();
    let mut style = build(doc, id, None, true, 0, &mut used, &mut tokens)?;
    let mut names = Vec::new();
    assign_names(&mut style, &mut names);
    Some((style, tokens))
}

/// A unique component name for every layer of the tree.
fn assign_names(style: &mut Style, used: &mut Vec<String>) {
    let base = component_name(style);
    let mut name = base.clone();
    let mut k = 2;
    while used.contains(&name) {
        name = format!("{base}{k}");
        k += 1;
    }
    used.push(name.clone());
    style.cname = name;
    for child in &mut style.children {
        assign_names(child, used);
    }
}

// ------------------------------------------------------------------ Tailwind CSS

/// The layer tree as JSX with Tailwind utility classes instead of a stylesheet.
pub fn tailwind(doc: &Document, id: NodeId) -> String {
    let Some((style, tokens)) = tree(doc, id) else { return String::new() };
    let mut out = String::new();
    tailwind_markup(&style, 0, &mut out);
    let vars = tokens.root_rule();
    if vars.is_empty() { out } else { format!("{vars}\n{out}") }
}

/// A layer as JSX with its Tailwind classes; the children follow, indented.
fn tailwind_markup(style: &Style, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    let classes: Vec<String> = style.decls.iter().flat_map(|(k, v)| classes_for(k, v)).collect();
    let class_attr = if classes.is_empty() { String::new() } else { format!(" className=\"{}\"", classes.join(" ")) };
    if let Some(text) = &style.text {
        out.push_str(&format!("{pad}<p{class_attr}>{}</p>\n", html_escape(text)));
        return;
    }
    if style.children.is_empty() {
        out.push_str(&format!("{pad}<div{class_attr} />\n"));
        return;
    }
    out.push_str(&format!("{pad}<div{class_attr}>\n"));
    for child in &style.children {
        tailwind_markup(child, depth + 1, out);
    }
    out.push_str(&format!("{pad}</div>\n"));
}

/// The Tailwind classes for one CSS declaration.
fn classes_for(prop: &str, value: &str) -> Vec<String> {
    let v = value.trim();
    match prop {
        "width" => length_class("w", v),
        "height" => length_class("h", v),
        "display" => match v {
            "flex" => vec!["flex".into()],
            "inline-flex" => vec!["inline-flex".into()],
            "block" => vec!["block".into()],
            "inline-block" => vec!["inline-block".into()],
            "grid" => vec!["grid".into()],
            _ => Vec::new(),
        },
        "flex-direction" => match v {
            "column" => vec!["flex-col".into()],
            "row" => vec!["flex-row".into()],
            "row-reverse" => vec!["flex-row-reverse".into()],
            "column-reverse" => vec!["flex-col-reverse".into()],
            _ => Vec::new(),
        },
        "flex-wrap" => match v {
            "wrap" => vec!["flex-wrap".into()],
            "nowrap" => vec!["flex-nowrap".into()],
            _ => Vec::new(),
        },
        "flex" => match v {
            "1 1 0" | "1 1 0%" => vec!["grow".into()],
            _ => Vec::new(),
        },
        "align-self" => named(v, "self", &[("stretch", "stretch"), ("center", "center"), ("flex-start", "start"), ("flex-end", "end")]),
        "justify-content" => named(
            v,
            "justify",
            &[("flex-start", "start"), ("flex-end", "end"), ("space-between", "between"), ("space-around", "around"), ("space-evenly", "evenly")],
        ),
        "align-items" => named(v, "items", &[("flex-start", "start"), ("flex-end", "end"), ("baseline", "baseline"), ("stretch", "stretch")]),
        "align-content" => named(
            v,
            "content",
            &[("flex-start", "start"), ("flex-end", "end"), ("space-between", "between"), ("space-around", "around"), ("space-evenly", "evenly")],
        ),
        "gap" => {
            if v == "0px" {
                Vec::new()
            } else {
                length_class("gap", v)
            }
        }
        "padding" => box_class("p", v),
        "position" => match v {
            "absolute" => vec!["absolute".into()],
            "relative" => vec!["relative".into()],
            "fixed" => vec!["fixed".into()],
            "sticky" => vec!["sticky".into()],
            _ => Vec::new(),
        },
        "left" => length_class("left", v),
        "top" => length_class("top", v),
        "right" => length_class("right", v),
        "bottom" => length_class("bottom", v),
        "z-index" => vec![format!("z-[{v}]")],
        "border-radius" => radius_class(v),
        "opacity" => vec![format!("opacity-{}", percent(v))],
        "mix-blend-mode" => vec![format!("mix-blend-{}", v.replace(' ', "-"))],
        "transform" => transform_class(v),
        "transform-origin" => match v {
            "top left" | "0 0" => vec!["origin-top-left".into()],
            "top" | "top center" => vec!["origin-top".into()],
            "center" => vec!["origin-center".into()],
            "bottom" | "bottom center" => vec!["origin-bottom".into()],
            _ => Vec::new(),
        },
        "background" | "background-color" => vec![format!("bg-{}", arbitrary(v))],
        "color" => vec![format!("text-{}", arbitrary(v))],
        "box-shadow" => vec![format!("shadow-{}", arbitrary(v))],
        "filter" => vec![format!("blur-{}", arbitrary(v))],
        "backdrop-filter" => vec![format!("backdrop-blur-{}", arbitrary(v))],
        "font-family" => vec![format!("font-['{}']", v.trim_matches('"'))],
        "font-size" => vec![format!("text-{}", arbitrary(v))],
        "font-weight" => vec![format!("font-{}", weight(v))],
        "font-style" => match v {
            "italic" | "oblique" => vec!["italic".into()],
            _ => Vec::new(),
        },
        "line-height" => vec![format!("leading-{}", arbitrary(v))],
        "letter-spacing" => vec![format!("tracking-{}", arbitrary(v))],
        "text-align" => named(v, "text", &[("left", "left"), ("center", "center"), ("right", "right"), ("justify", "justify")]),
        "text-transform" => match v {
            "uppercase" => vec!["uppercase".into()],
            "lowercase" => vec!["lowercase".into()],
            "capitalize" => vec!["capitalize".into()],
            "small-caps" => vec!["small-caps".into()],
            _ => Vec::new(),
        },
        "text-decoration" => match v {
            "underline" => vec!["underline".into()],
            "line-through" => vec!["line-through".into()],
            _ => Vec::new(),
        },
        "text-decoration-style" => {
            named(v, "decoration", &[("dotted", "dotted"), ("dashed", "dashed"), ("wavy", "wavy"), ("double", "double"), ("solid", "solid")])
        }
        "text-decoration-thickness" => vec![format!("decoration-{}", arbitrary(v))],
        "text-decoration-color" => vec![format!("decoration-{}", arbitrary(v))],
        "text-underline-offset" => vec![format!("underline-offset-{}", arbitrary(v))],
        "overflow" => match v {
            "hidden" => vec!["overflow-hidden".into()],
            "auto" => vec!["overflow-auto".into()],
            "scroll" => vec!["overflow-scroll".into()],
            _ => Vec::new(),
        },
        "object-fit" => match v {
            "cover" => vec!["object-cover".into()],
            "contain" => vec!["object-contain".into()],
            "fill" => vec!["object-fill".into()],
            _ => Vec::new(),
        },
        "border" | "border-top" | "border-right" | "border-bottom" | "border-left" => border_classes(prop, v),
        _ => Vec::new(),
    }
}

/// A class for a length: `12px` → `w-[12px]`.
fn length_class(prefix: &str, value: &str) -> Vec<String> {
    if value.is_empty() {
        return Vec::new();
    }
    vec![format!("{prefix}-{}", arbitrary(value))]
}

/// A class for a spacing value with up to four sides: `4px 8px` → `px-[8px] py-[4px]`.
fn box_class(prefix: &str, value: &str) -> Vec<String> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    match parts.len() {
        0 => Vec::new(),
        1 => vec![format!("{prefix}-[{}]", parts[0])],
        2 => vec![format!("{prefix}-x-[{}]", parts[1]), format!("{prefix}-y-[{}]", parts[0])],
        3 => vec![format!("{prefix}-x-[{}]", parts[1]), format!("{prefix}-t-[{}]", parts[0]), format!("{prefix}-b-[{}]", parts[2])],
        _ => vec![format!("{prefix}-[{}]", parts.join("_"))],
    }
}

/// A class for a border radius: `12px` → `rounded-[12px]`, `50%` → `rounded-full`.
fn radius_class(value: &str) -> Vec<String> {
    if value == "50%" {
        return vec!["rounded-full".into()];
    }
    vec![format!("rounded-{}", arbitrary(value))]
}

/// The classes for a border: its width and its colour.
fn border_classes(prop: &str, value: &str) -> Vec<String> {
    let side = match prop {
        "border-top" => "t",
        "border-right" => "r",
        "border-bottom" => "b",
        "border-left" => "l",
        _ => "",
    };
    let mut out = Vec::new();
    let mut width = String::new();
    let mut color = String::new();
    for part in value.split_whitespace() {
        if part.ends_with("px") && part.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            width = part.to_string();
        } else if part.starts_with('#') || part.starts_with("rgb") || part.starts_with("var(") || part.starts_with("hsl") {
            color = part.to_string();
        }
    }
    if !width.is_empty() {
        out.push(match width.as_str() {
            "1px" => format!("border{side}"),
            "2px" => format!("border{side}-2"),
            "4px" => format!("border{side}-4"),
            _ => format!("border{side}-[{width}]"),
        });
    }
    if !color.is_empty() {
        out.push(format!("border{side}-{}", arbitrary(&color)));
    }
    out
}

/// The classes for a transform: `rotate(-45deg)` → `rotate-[-45deg]`.
fn transform_class(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in value.split_whitespace() {
        if let Some(deg) = part.strip_prefix("rotate(").and_then(|r| r.strip_suffix(')')) {
            out.push(format!("rotate-[{deg}deg]"));
        } else if let Some(scale) = part.strip_prefix("scale(").and_then(|r| r.strip_suffix(')')) {
            out.push(format!("scale-[{scale}]"));
        }
    }
    out
}

/// The Tailwind font weight for a CSS weight: `600` → `semibold`.
fn weight(value: &str) -> &'static str {
    match value.trim() {
        "100" => "thin",
        "200" => "extralight",
        "300" => "light",
        "400" | "normal" => "normal",
        "500" => "medium",
        "600" => "semibold",
        "700" | "bold" => "bold",
        "800" => "extrabold",
        "900" => "black",
        _ => "normal",
    }
}

/// A Tailwind opacity: `0.5` → `50`.
fn percent(value: &str) -> String {
    match value.trim().parse::<f64>() {
        Ok(v) if v.is_finite() => {
            let p = (v * 100.0).round();
            if (p - p.round()).abs() < 0.001 { format!("{}", p.round() as i64) } else { format!("{p}") }
        }
        _ => "100".into(),
    }
}

/// A named class for a value of a known set: `center` → `justify-center`.
fn named(value: &str, prefix: &str, names: &[(&str, &str)]) -> Vec<String> {
    match names.iter().find(|(k, _)| *k == value) {
        Some((_, n)) => vec![format!("{prefix}-{n}")],
        None => Vec::new(),
    }
}

/// A value as a Tailwind arbitrary value: `12px` → `[12px]`, `#FF0000` → `[#FF0000]`,
/// `var(--x, #FFF)` → `[var(--x)]`, `linear-gradient(…)` → `[linear-gradient(…)]`.
fn arbitrary(value: &str) -> String {
    let v = value.trim();
    if let Some(rest) = v.strip_prefix("var(") {
        let name = rest.split(',').next().unwrap_or("").trim();
        return format!("[var({name})]");
    }
    format!("[{}]", v.replace(' ', "_"))
}

// ------------------------------------------------------------------ SCSS

/// The layer tree as SCSS: the rules nested as the layers are.
pub fn scss(doc: &Document, id: NodeId) -> String {
    let Some((style, tokens)) = tree(doc, id) else { return String::new() };
    let mut out = tokens.root_rule();
    scss_rules(&style, 0, &mut out);
    out
}

/// A layer's rule, with its children's rules nested inside it.
fn scss_rules(style: &Style, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    out.push_str(&format!("{pad}.{} {{\n", style.class));
    for (key, value) in &style.decls {
        out.push_str(&format!("{pad}  {key}: {value};\n"));
    }
    for child in &style.children {
        scss_rules(child, depth + 1, out);
    }
    out.push_str(&format!("{pad}}}\n"));
}

// ------------------------------------------------------------------ Vue, Angular, Svelte

/// The layer tree as a Vue single-file component.
pub fn vue(doc: &Document, id: NodeId) -> String {
    let Some((style, tokens)) = tree(doc, id) else { return String::new() };
    let mut markup = String::new();
    html_markup(&style, 0, &mut markup);
    format!(
        "<template>\n{}</template>\n\n<style scoped>\n{}{}</style>\n",
        indent_block(&markup),
        tokens.root_rule(),
        indent_block(&stylesheet(&style))
    )
}

/// The layer tree as an Angular component.
pub fn angular(doc: &Document, id: NodeId) -> String {
    let Some((style, tokens)) = tree(doc, id) else { return String::new() };
    let mut markup = String::new();
    html_markup(&style, 0, &mut markup);
    let name = component_name(&style);
    format!(
        "import {{ Component }} from \"@angular/core\";\n\n@Component({{\n  selector: \"app-{}\",\n  template: `\n{}  `,\n  styles: [\n    `\n{}{}    `,\n  ],\n}})\nexport class {name}Component {{}}\n",
        kebab(&name),
        indent_block_by(&markup, 4),
        tokens.root_rule(),
        indent_block_by(&stylesheet(&style), 4),
    )
}

/// The layer tree as a Svelte component.
pub fn svelte(doc: &Document, id: NodeId) -> String {
    let Some((style, tokens)) = tree(doc, id) else { return String::new() };
    let mut markup = String::new();
    html_markup(&style, 0, &mut markup);
    format!(
        "<script lang=\"ts\">\n  // {}\n</script>\n\n{markup}\n<style>\n{}{}</style>\n",
        style.name.replace(['\n', '\r'], " "),
        tokens.root_rule(),
        indent_block(&stylesheet(&style)),
    )
}

/// The layer tree as nested HTML elements with a stylesheet.
fn html_markup(style: &Style, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    if let Some(text) = &style.text {
        out.push_str(&format!("{pad}<p class=\"{}\">{}</p>\n", style.class, html_escape(text)));
        return;
    }
    if style.children.is_empty() {
        out.push_str(&format!("{pad}<div class=\"{}\"></div>\n", style.class));
        return;
    }
    out.push_str(&format!("{pad}<div class=\"{}\">\n", style.class));
    for child in &style.children {
        html_markup(child, depth + 1, out);
    }
    out.push_str(&format!("{pad}</div>\n"));
}

/// The stylesheet of a tree: one rule per layer.
fn stylesheet(style: &Style) -> String {
    let mut out = String::new();
    stylesheet_into(style, &mut out);
    out
}

fn stylesheet_into(style: &Style, out: &mut String) {
    out.push_str(&format!(".{} {{\n", style.class));
    for (key, value) in &style.decls {
        out.push_str(&format!("  {key}: {value};\n"));
    }
    out.push_str("}\n");
    for child in &style.children {
        stylesheet_into(child, out);
    }
}

/// A block of text indented by two spaces.
fn indent_block(text: &str) -> String {
    indent_block_by(text, 2)
}

/// A block of text indented by `n` spaces.
fn indent_block_by(text: &str, n: usize) -> String {
    let pad = " ".repeat(n);
    text.lines().map(|l| if l.is_empty() { String::new() } else { format!("{pad}{l}\n") }).collect()
}

// ------------------------------------------------------------------ styled-components

/// The layer tree as React components styled with styled-components.
pub fn styled_components(doc: &Document, id: NodeId) -> String {
    let Some((style, _tokens)) = tree(doc, id) else { return String::new() };
    let mut out = String::from("import styled from \"styled-components\";\n\n");
    styled_definitions(&style, true, &mut out);
    out.push_str("// Usage\n");
    styled_markup(&style, 0, &mut out);
    out
}

/// The styled components of a tree, the children before the parent that refers to them.
fn styled_definitions(style: &Style, root: bool, out: &mut String) {
    for child in &style.children {
        styled_definitions(child, false, out);
    }
    let tag = if style.text.is_some() { "p" } else { "div" };
    out.push_str(&format!("{}const {} = styled.{tag}`\n", if root { "export " } else { "" }, style.cname));
    for (key, value) in &style.decls {
        out.push_str(&format!("  {key}: {value};\n"));
    }
    out.push_str("`;\n\n");
}

/// The layer tree as JSX with the styled components as the tags.
fn styled_markup(style: &Style, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    let name = &style.cname;
    if let Some(text) = &style.text {
        out.push_str(&format!("{pad}<{name}>{}</{name}>\n", html_escape(text)));
        return;
    }
    if style.children.is_empty() {
        out.push_str(&format!("{pad}<{name} />\n"));
        return;
    }
    out.push_str(&format!("{pad}<{name}>\n"));
    for child in &style.children {
        styled_markup(child, depth + 1, out);
    }
    out.push_str(&format!("{pad}</{name}>\n"));
}

/// A component name from a class name: "card-title" → "CardTitle".
fn component_name(style: &Style) -> String {
    let mut out = String::new();
    for part in style.class.split(['-', '_', ' ']) {
        let mut c = part.chars();
        if let Some(f) = c.next() {
            out.push(f.to_ascii_uppercase());
            out.push_str(c.as_str());
        }
    }
    if out.is_empty() { "Component".into() } else { out }
}

/// "CardTitle" → "card-title".
fn kebab(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_uppercase() {
            if !out.is_empty() {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}
