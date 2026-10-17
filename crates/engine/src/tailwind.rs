//! React + Tailwind CSS v4 for a layer tree, ready to drop into a web project: every layer's look
//! as utility classes instead of a stylesheet, bound variables as theme utilities (`bg-accent`,
//! `rounded-card`) that `variable.export {format: "tailwind"}` declares in an `@theme` block, auto
//! layout frames and text that grow with the page (`w-full max-w-[…]`) instead of fixed pixel
//! boxes, and instances inlined with their overrides. Given a mobile frame too, the code is mobile
//! first: the mobile frame's classes, and the desktop frame's where they differ behind `lg:`.

use std::collections::BTreeMap;

use figcraft_doc::*;

use crate::codegen::{CodeLang, Ctx, Markup, css_var_name, css_var_of, num};
use crate::component_codegen::component_type_name;

/// The breakpoint desktop classes sit behind in responsive code.
// ponytail: one breakpoint (lg = 1024px); a tablet frame would need a second pass and `md:`.
const BREAKPOINT: &str = "lg";

/// What a number variable measures, from how layers use it (else its scopes): it picks the theme
/// namespace and whether the CSS value takes a unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumberRole {
    Radius,
    FontSize,
    Spacing,
    /// Another length (stroke, line height, letter spacing, effects): pixels, no namespace.
    Length,
    /// Opacity or font weight: no unit.
    Unitless,
}

fn role_of_field(field: &str) -> Option<NumberRole> {
    Some(match field {
        "cornerRadius" | "topLeftRadius" | "topRightRadius" | "bottomRightRadius" | "bottomLeftRadius" => NumberRole::Radius,
        "fontSize" => NumberRole::FontSize,
        "opacity" | "fontWeight" => NumberRole::Unitless,
        f if f == "gap" || f == "counterAxisSpacing" || f.starts_with("padding") || f == "width" || f == "height" => NumberRole::Spacing,
        f if f.starts_with("min") || f.starts_with("max") => NumberRole::Spacing,
        "strokeWeight" | "lineHeight" | "letterSpacing" | "paragraphSpacing" | "paragraphIndent" => NumberRole::Length,
        _ => return None,
    })
}

fn role_of_scope(scope: &str) -> Option<NumberRole> {
    Some(match scope {
        "CORNER_RADIUS" => NumberRole::Radius,
        "FONT_SIZE" => NumberRole::FontSize,
        "GAP" | "WIDTH_HEIGHT" => NumberRole::Spacing,
        "OPACITY" | "FONT_WEIGHT" => NumberRole::Unitless,
        "STROKE_FLOAT" | "EFFECT_FLOAT" | "LINE_HEIGHT" | "LETTER_SPACING" | "PARAGRAPH_SPACING" | "PARAGRAPH_INDENT" => NumberRole::Length,
        _ => return None,
    })
}

/// The role of a number variable: the first layer field bound to it, else its first scope with a
/// role; a number nothing tells about is a length (design tokens mostly are).
pub fn number_role(doc: &Document, v: &Variable) -> NumberRole {
    let bound = doc.nodes().find_map(|n| n.bound.iter().find(|(_, id)| **id == v.id).and_then(|(f, _)| role_of_field(f)));
    bound.or_else(|| v.scopes.as_ref().and_then(|s| s.iter().find_map(|s| role_of_scope(s)))).unwrap_or(NumberRole::Length)
}

/// A number variable's CSS value: pixels for lengths, bare for opacity and font weight.
pub fn number_css(doc: &Document, v: &Variable, x: f64) -> String {
    if number_role(doc, v) == NumberRole::Unitless { num(x) } else { format!("{}px", num(x)) }
}

/// The Tailwind theme variable for `v`: `--color-*` for colours, `--radius-*`, `--text-*` and
/// `--spacing-*` for numbers by role, `--font-*` for font families; others keep their CSS name.
/// A name already in the namespace isn't prefixed twice ("Color/Accent" → `--color-accent`).
pub fn theme_name(doc: &Document, v: &Variable) -> String {
    let ns = match v.kind {
        VariableType::Color => Some("color"),
        VariableType::Number => match number_role(doc, v) {
            NumberRole::Radius => Some("radius"),
            NumberRole::FontSize => Some("text"),
            NumberRole::Spacing => Some("spacing"),
            NumberRole::Length | NumberRole::Unitless => None,
        },
        VariableType::String if v.scopes.as_ref().is_some_and(|s| s.iter().any(|s| s == "FONT_FAMILY")) => Some("font"),
        _ => None,
    };
    let base = css_var_of(v);
    let Some(ns) = ns else { return base };
    // The last path segment names it within the namespace ("Brand/Colors/Accent" → "accent").
    let slug = css_var_name(v.name.rsplit('/').next().unwrap_or(&v.name));
    let slug = slug.trim_start_matches('-');
    let slug = [format!("{ns}s-"), format!("{ns}-")].iter().find_map(|p| slug.strip_prefix(p.as_str())).unwrap_or(slug);
    // A variable named after its namespace ("radius", "colors") is the namespace's base value.
    let slug = if slug == ns || slug == format!("{ns}s") { "base" } else { slug };
    format!("--{ns}-{slug}")
}

// ------------------------------------------------------------------ declarations → classes

/// An arbitrary value: spaces become underscores (Tailwind's escape), underscores are escaped.
fn arb(v: &str) -> String {
    format!("[{}]", v.trim().replace('_', "\\_").replace(' ', "_"))
}

/// The custom property a `var(--name, fallback)` value reads.
fn var_name(v: &str) -> Option<&str> {
    let inner = v.trim().strip_prefix("var(")?;
    Some(inner.split([',', ')']).next()?.trim())
}

/// `util` with `value`: a theme utility when the value reads a variable of the namespace
/// (`bg-accent`), else the variable in parentheses (`bg-(--x)`, with a type hint for `text-`
/// sizes), else an arbitrary value.
fn util(util: &str, ns: &str, value: &str, hint: &str) -> String {
    match var_name(value) {
        Some(name) => match name.strip_prefix(&format!("--{ns}-")) {
            Some(key) if !ns.is_empty() => format!("{util}-{key}"),
            _ => format!("{util}-({hint}{name})"),
        },
        None => format!("{util}-{}", arb(value)),
    }
}

/// A length on the spacing scale (4px steps) when it fits one, else arbitrary.
fn spacing(prefix: &str, value: &str) -> String {
    if var_name(value).is_some() {
        return util(prefix, "spacing", value, "");
    }
    match value.strip_suffix("px").and_then(|n| n.parse::<f64>().ok()) {
        Some(0.0) => format!("{prefix}-0"),
        Some(px) if px > 0.0 && px <= 384.0 && (px / 4.0).fract() == 0.0 => format!("{prefix}-{}", num(px / 4.0)),
        Some(px) if px < 0.0 && (px / 4.0).fract() == 0.0 => format!("-{prefix}-{}", num(-px / 4.0)),
        _ => format!("{prefix}-{}", arb(value)),
    }
}

/// The values of a `padding` shorthand (top right bottom left), as utilities.
fn padding(value: &str) -> Vec<(String, String)> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut cur = String::new();
    for c in value.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ' ' if depth == 0 => {
                parts.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    parts.push(cur);
    let [t, r, b, l] = match parts.as_slice() {
        [a] => [a, a, a, a],
        [t, r, b, l] => [t, r, b, l],
        _ => return vec![("padding".into(), format!("p-{}", arb(value)))],
    };
    let zero = |v: &str| v == "0px" || v == "0";
    let mut out = Vec::new();
    if t == r && r == b && b == l {
        if !zero(t) {
            out.push(("padding".into(), spacing("p", t)));
        }
    } else if t == b && l == r {
        for (k, p, v) in [("padding-y", "py", t), ("padding-x", "px", l)] {
            if !zero(v) {
                out.push((k.into(), spacing(p, v)));
            }
        }
    } else {
        for (k, p, v) in [("padding-top", "pt", t), ("padding-right", "pr", r), ("padding-bottom", "pb", b), ("padding-left", "pl", l)] {
            if !zero(v) {
                out.push((k.into(), spacing(p, v)));
            }
        }
    }
    out
}

fn font_weight(v: &str) -> String {
    let named = match v.trim() {
        "100" => "thin",
        "200" => "extralight",
        "300" => "light",
        "400" => "normal",
        "500" => "medium",
        "600" => "semibold",
        "700" => "bold",
        "800" => "extrabold",
        "900" => "black",
        _ => return format!("font-{}", arb(v)),
    };
    format!("font-{named}")
}

/// A border shorthand (`1px solid c`) on `side` ("" or "t", "r", "b", "l") as width + colour.
fn border(side: &str, value: &str) -> Vec<(String, String)> {
    let mut it = value.splitn(3, ' ');
    let (width, _style, color) = (it.next().unwrap_or(""), it.next(), it.next().unwrap_or(""));
    let b = if side.is_empty() { "border".to_string() } else { format!("border-{side}") };
    let key = format!("border{}", if side.is_empty() { String::new() } else { format!("-{side}") });
    let w = match width {
        "1px" => b.clone(),
        "0px" => format!("{b}-0"),
        w => match var_name(w) {
            Some(name) => format!("{b}-({name})"),
            None => format!("{b}-{}", arb(w)),
        },
    };
    let mut out = vec![(format!("{key}-width"), w)];
    if !color.is_empty() {
        out.push(("border-color".into(), util("border", "color", color, "")));
    }
    out
}

/// One CSS declaration as utilities, keyed by the property they set (so responsive code can tell
/// which mobile class a desktop class replaces).
fn decl(key: &str, value: &str) -> Vec<(String, String)> {
    let one = |c: String| vec![(key.to_string(), c)];
    let kw = |c: &str| vec![(key.to_string(), c.to_string())];
    match (key, value) {
        ("display", "flex") => kw("flex"),
        ("flex-direction", "column") => kw("flex-col"),
        ("flex-direction", _) => kw("flex-row"),
        ("flex-wrap", "wrap") => kw("flex-wrap"),
        // Defaults flexbox already has.
        ("justify-content", "flex-start") | ("gap", "0px") => Vec::new(),
        ("justify-content", v) => one(format!("justify-{}", v.trim_start_matches("flex-").trim_start_matches("space-"))),
        ("align-items", v) => one(format!("items-{}", v.trim_start_matches("flex-"))),
        ("align-content", v) => one(format!("content-{}", v.trim_start_matches("flex-").trim_start_matches("space-"))),
        ("align-self", "stretch") => kw("self-stretch"),
        ("flex", "1 1 0") => kw("flex-1"),
        ("position", v) => one(v.to_string()),
        ("left" | "top" | "right" | "bottom", v) => one(spacing(key, v)),
        ("width", "100%") => kw("w-full"),
        ("width", v) => one(spacing("w", v)),
        ("height", v) => one(spacing("h", v)),
        ("max-width", v) => one(format!("max-w-{}", arb(v))),
        ("min-height", v) => one(format!("min-h-{}", arb(v))),
        ("margin-inline", "auto") => kw("mx-auto"),
        ("gap", v) => one(spacing("gap", v)),
        ("padding", v) => padding(v),
        ("border-radius", "50%") => kw("rounded-full"),
        ("border-radius", v) if v.strip_suffix("px").and_then(|n| n.parse::<f64>().ok()).is_some_and(|r| r >= 999.0) => kw("rounded-full"),
        ("border-radius", v) if v.contains(' ') && var_name(v).is_none() => one(format!("rounded-{}", arb(v))),
        ("border-radius", v) => one(util("rounded", "radius", v, "")),
        ("opacity", v) => match v.parse::<f64>() {
            Ok(o) if ((o * 100.0).round() - o * 100.0).abs() < 1e-9 => one(format!("opacity-{}", num(o * 100.0))),
            _ => one(util("opacity", "", v, "")),
        },
        ("background", v) if v.contains("gradient(") || v.starts_with("url(") => one(format!("bg-{}", arb(v))),
        ("background", v) => one(util("bg", "color", v, "color:")),
        ("color", v) => one(util("text", "color", v, "color:")),
        ("border", v) => border("", v),
        ("border-top", v) => border("t", v),
        ("border-right", v) => border("r", v),
        ("border-bottom", v) => border("b", v),
        ("border-left", v) => border("l", v),
        ("box-shadow", v) => one(format!("shadow-{}", arb(v))),
        ("filter", v) => one(match v.strip_prefix("blur(").and_then(|r| r.strip_suffix(')')) {
            Some(r) => format!("blur-{}", arb(r)),
            None => format!("[filter:{}]", arb(v).trim_matches(['[', ']'])),
        }),
        ("backdrop-filter", v) => one(match v.strip_prefix("blur(").and_then(|r| r.strip_suffix(')')) {
            Some(r) => format!("backdrop-blur-{}", arb(r)),
            None => format!("[backdrop-filter:{}]", arb(v).trim_matches(['[', ']'])),
        }),
        ("mix-blend-mode", v) => one(format!("mix-blend-{v}")),
        ("transform", v) => match v.strip_prefix("rotate(").and_then(|r| r.strip_suffix(')')) {
            Some(r) => one(format!("rotate-{}", arb(r))),
            None => one(format!("[transform:{}]", arb(v).trim_matches(['[', ']']))),
        },
        ("transform-origin", "top left" | "0 0") => kw("origin-top-left"),
        ("font-family", v) if var_name(v).is_some() => one(util("font", "font", v, "")),
        ("font-family", v) => one(format!("font-{}", arb(&format!("'{}'", v.trim_matches('"'))))),
        ("font-size", v) => one(util("text", "text", v, "length:")),
        ("font-weight", v) => one(font_weight(v)),
        ("font-style", "italic") => kw("italic"),
        ("line-height", v) => match v.strip_suffix('%').and_then(|p| p.parse::<f64>().ok()) {
            Some(p) => one(format!("leading-{}", arb(&num(p / 100.0)))),
            None => one(util("leading", "", v, "")),
        },
        ("letter-spacing", v) => one(util("tracking", "", v, "")),
        ("text-align", v) => one(format!("text-{v}")),
        ("text-transform", "uppercase") => kw("uppercase"),
        ("text-transform", "lowercase") => kw("lowercase"),
        ("text-transform", "capitalize") => kw("capitalize"),
        ("text-decoration", "underline") => kw("underline"),
        ("text-decoration", "line-through") => kw("line-through"),
        _ => one(format!("[{key}:{}]", arb(value).trim_matches(['[', ']']))),
    }
}

/// A rule's declarations (`key: value;` lines) as (property, utility) pairs in order.
pub fn classes(decls: &str) -> Vec<(String, String)> {
    decls
        .lines()
        .filter_map(|l| {
            let (k, v) = l.trim().trim_end_matches(';').split_once(':')?;
            Some(decl(k.trim(), v.trim()))
        })
        .flatten()
        .collect()
}

/// The utility that undoes a mobile class at the breakpoint when desktop sets nothing for it.
fn reset(key: &str) -> Option<&'static str> {
    Some(match key {
        "display" => "block",
        "flex-direction" => "flex-row",
        "flex-wrap" => "flex-nowrap",
        "flex" => "flex-initial",
        "align-self" => "self-auto",
        "width" => "w-auto",
        "height" => "h-auto",
        "max-width" => "max-w-none",
        "min-height" => "min-h-0",
        "text-align" => "text-left",
        "justify-content" => "justify-start",
        "gap" => "gap-0",
        "padding" | "padding-x" | "padding-y" | "padding-top" | "padding-right" | "padding-bottom" | "padding-left" => "p-0",
        "position" => "static",
        "text-transform" => "normal-case",
        _ => return None,
    })
}

/// Mobile classes plus, behind the breakpoint, the desktop ones that differ (and resets for what
/// desktop leaves unset). A layer missing on mobile is hidden there.
fn merge(mobile: Option<&[(String, String)]>, desktop: &[(String, String)]) -> Vec<String> {
    let hidden = [("display".to_string(), "hidden".to_string())];
    // A mobile frame's widths are the phone's, not caps: below the breakpoint boxes fill the page.
    let fluid: Vec<(String, String)>;
    let mobile = match mobile {
        Some(m) => {
            fluid = m.iter().filter(|(k, _)| k != "max-width").cloned().collect();
            fluid.as_slice()
        }
        None => &hidden,
    };
    let mut out: Vec<String> = mobile.iter().map(|(_, c)| c.clone()).collect();
    let bp = |c: &str| format!("{BREAKPOINT}:{c}");
    for (_, c) in desktop {
        if !mobile.iter().any(|(_, m)| m == c) {
            out.push(bp(c));
        }
    }
    // Padding utilities overlap (p, px/py, pt…): desktop setting any of them replaces them all.
    let same = |a: &str, b: &str| a == b || (a.starts_with("padding") && b.starts_with("padding"));
    for (k, _) in mobile {
        if !desktop.iter().any(|(d, _)| same(d, k))
            && let Some(r) = reset(k)
        {
            out.push(bp(r));
        }
    }
    out.dedup();
    out
}

/// The markup walk for `id` in Tailwind mode: (markup, rules by path).
fn walk(doc: &Document, id: NodeId) -> (String, Vec<(String, String, String)>) {
    let ctx = Ctx::new(doc, id);
    let mut m = Markup::new(&ctx, CodeLang::React);
    m.inline = true;
    m.fluid = true;
    m.fluid_leaves = 0.5;
    m.tokens.tailwind = true;
    m.node(id, None, 2, "");
    (m.markup, m.rules)
}

/// React + Tailwind for `id` (with `mobile`, mobile first with desktop classes behind `lg:`).
pub fn code(doc: &Document, id: NodeId, mobile: Option<NodeId>) -> String {
    let Some(n) = doc.node(id) else { return String::new() };
    let (mut markup, rules) = walk(doc, id);
    let small: Option<BTreeMap<String, Vec<(String, String)>>> =
        mobile.filter(|m| doc.contains(*m) && *m != id).map(|m| walk(doc, m).1.into_iter().map(|(p, _, d)| (p, classes(&d))).collect());
    // ponytail: layers only on mobile are listed, not merged in; their place in the tree is ambiguous.
    let only_mobile: Vec<String> = small
        .iter()
        .flat_map(|m| m.keys())
        .filter(|p| !rules.iter().any(|(d, _, _)| d == *p))
        .map(|p| p.trim_start_matches('/').split('/').map(|seg| seg.rsplit_once('#').map_or(seg, |(n, _)| n)).collect::<Vec<_>>().join(" › "))
        .take(20)
        .collect();
    // Longest class names first, so "card" never rewrites inside "card-2"'s attribute.
    let mut rules = rules;
    rules.sort_by_key(|(_, c, _)| std::cmp::Reverse(c.len()));
    let paths: Vec<String> = rules.iter().map(|(p, _, _)| p.clone()).collect();
    for (path, class, decls) in &rules {
        let own = classes(decls);
        // Layer names may contain "/": the parent is the longest other path this one extends.
        let parent =
            paths.iter().filter(|p| p.len() < path.len() && path.starts_with(&format!("{p}/"))).max_by_key(|p| p.len()).map_or("", String::as_str);
        let list = match &small {
            // Inside a layer hidden on mobile: desktop's classes as they are.
            Some(m) if !path.is_empty() && !m.contains_key(parent) => own.into_iter().map(|(_, c)| c).collect(),
            Some(m) => merge(m.get(path).map(Vec::as_slice), &own),
            None => own.into_iter().map(|(_, c)| c).collect(),
        };
        markup = markup.replace(&format!("className=\"{class}\""), &format!("className=\"{}\"", list.join(" ")));
    }
    let name = component_type_name(&n.name);
    let mut mode = if small.is_some() { format!(", mobile first; desktop from {BREAKPOINT}") } else { String::new() };
    if !only_mobile.is_empty() {
        mode.push_str(&format!(". Only in the mobile frame, add by hand with {BREAKPOINT}:hidden: {}", only_mobile.join(", ")));
    }
    let header = comment_safe(&format!("{} as React + Tailwind CSS v4{mode}. Theme: variable.export {{format: \"tailwind\"}}.", n.name));
    format!("// {header}\nexport function {name}() {{\n  return (\n{markup}  );\n}}\n")
}

/// `text` for one `//` comment line: every JS line terminator (control characters, U+2028,
/// U+2029) becomes a space, so layer names in it can't end the comment and inject code.
fn comment_safe(text: &str) -> String {
    text.chars().take(2_000).map(|c| if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') { ' ' } else { c }).collect()
}
