//! Code for a layer, as shown by the Inspect panel and returned by `inspect.code`: CSS for the
//! layer itself, HTML or React for its subtree, and SwiftUI or Jetpack Compose (stacks for auto
//! layout, absolute offsets otherwise). Components become components whose properties are
//! parameters and instances become calls passing their values (see [`crate::component_codegen`]);
//! variables bound to a layer become CSS custom properties. The output is a starting point, not a
//! pixel-exact translation.

use figcraft_color::{Color, PaintKind};
use figcraft_doc::*;
use serde_json::json;

use crate::cmd::CommandSpec;
use crate::component_codegen::{Component, Param};
use crate::params::*;
use crate::{bad, spec};

pub fn specs() -> Vec<CommandSpec> {
    vec![spec!(
        "inspect.code",
        "Code for layer",
        "{id?, language?: css|html|react|leptos|swiftui|compose (default css; html, react and leptos are the whole layer tree; components take their properties as parameters, instances pass their values; a component connected to code (codeConnect.set) gives its import and snippet instead), unit?: px|rem (CSS, HTML, React and Leptos; default px), rootFontSize? (rem: pixels per rem, default 16)} → {code, language, connected}",
        false,
        |s, p| {
            let cmd = "inspect.code";
            let id = targets(s, p).first().copied().ok_or_else(|| bad(cmd, "select a layer"))?;
            let lang = match string(p, "language") {
                Some(l) => CodeLang::from_id(&l).ok_or_else(|| bad(cmd, "language: css|html|react|leptos|swiftui|compose"))?,
                None => CodeLang::Css,
            };
            let rem = match string(p, "unit").map(|u| u.to_ascii_lowercase()).as_deref() {
                None | Some("px") => None,
                Some("rem") => Some(num_or(p, "rootFontSize", 16.0)).filter(|r| r.is_finite() && *r > 0.0 && *r <= 1000.0),
                Some(_) => return Err(bad(cmd, "unit: px|rem")),
            };
            if string(p, "unit").is_some_and(|u| u.eq_ignore_ascii_case("rem")) && rem.is_none() {
                return Err(bad(cmd, "rootFontSize: a positive number of pixels (up to 1000)"));
            }
            let doc = s.doc()?;
            if !doc.contains(id) {
                return Err(bad(cmd, "no such layer"));
            }
            let connected = crate::code_connect::link(doc, id, lang).is_some();
            let code = code_for(doc, id, lang);
            let code = match rem {
                Some(root) if lang.is_web() => px_to_rem(&code, root),
                _ => code,
            };
            Ok(json!({"code": code, "language": lang.id(), "connected": connected}))
        }
    )]
}

/// Languages the Inspect panel offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CodeLang {
    #[default]
    Css,
    /// The whole layer tree as nested HTML elements with a stylesheet.
    Html,
    /// A React function component (TSX) with its stylesheet.
    React,
    /// A Leptos component (`view!` markup) with its stylesheet.
    Leptos,
    SwiftUi,
    Compose,
}

impl CodeLang {
    pub const ALL: [CodeLang; 6] = [CodeLang::Css, CodeLang::Html, CodeLang::React, CodeLang::Leptos, CodeLang::SwiftUi, CodeLang::Compose];
    pub fn id(self) -> &'static str {
        match self {
            CodeLang::Css => "css",
            CodeLang::Html => "html",
            CodeLang::React => "react",
            CodeLang::Leptos => "leptos",
            CodeLang::SwiftUi => "swiftui",
            CodeLang::Compose => "compose",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            CodeLang::Css => "CSS",
            CodeLang::Html => "HTML + CSS",
            CodeLang::React => "React",
            CodeLang::Leptos => "Leptos",
            CodeLang::SwiftUi => "SwiftUI",
            CodeLang::Compose => "Compose",
        }
    }
    /// Languages whose lengths are CSS pixels (they can be shown in rem).
    pub fn is_web(self) -> bool {
        matches!(self, CodeLang::Css | CodeLang::Html | CodeLang::React | CodeLang::Leptos)
    }
    pub fn from_id(s: &str) -> Option<CodeLang> {
        if s.eq_ignore_ascii_case("tsx") || s.eq_ignore_ascii_case("jsx") {
            return Some(CodeLang::React);
        }
        CodeLang::ALL.into_iter().find(|l| l.id().eq_ignore_ascii_case(s) || l.label().eq_ignore_ascii_case(s))
    }
}

/// Layers deeper than this (or past this many children) are summarised.
const MAX_DEPTH: usize = 12;
const MAX_CHILDREN: usize = 200;

/// `v` without trailing zeros (2 decimals).
pub(crate) fn num(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let r = (v * 100.0).round() / 100.0;
    if (r - r.round()).abs() < 1e-9 { format!("{}", r.round() as i64) } else { format!("{r}") }
}

/// `code` with every pixel length (`12px`) in rem of `root` pixels (`0.75rem`); names that end
/// in "px", markup lines (layer text), string literals and comments are left alone.
pub fn px_to_rem(code: &str, root: f64) -> String {
    let mut out = String::with_capacity(code.len());
    let mut in_comment = false;
    for line in code.split_inclusive('\n') {
        if !in_comment && line.trim_start().starts_with('<') {
            out.push_str(line);
            continue;
        }
        // Lengths outside strings and comments convert; `plain` is where that stretch starts.
        let mut plain = 0;
        let mut chars = line.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if in_comment {
                if c == '*' && chars.peek().is_some_and(|(_, n)| *n == '/') {
                    chars.next();
                    out.push_str(line.get(plain..i + 2).unwrap_or(""));
                    plain = i + 2;
                    in_comment = false;
                }
            } else if c == '/' && chars.peek().is_some_and(|(_, n)| *n == '*') {
                chars.next();
                out.push_str(&lengths_to_rem(line.get(plain..i).unwrap_or(""), root));
                plain = i;
                in_comment = true;
            } else if c == '"' {
                out.push_str(&lengths_to_rem(line.get(plain..i).unwrap_or(""), root));
                let mut end = line.len();
                while let Some((j, s)) = chars.next() {
                    if s == '\\' {
                        chars.next();
                    } else if s == '"' {
                        end = j + 1;
                        break;
                    }
                }
                out.push_str(line.get(i..end).unwrap_or(""));
                plain = end;
            }
        }
        let rest = line.get(plain..).unwrap_or("");
        if in_comment {
            out.push_str(rest);
        } else {
            out.push_str(&lengths_to_rem(rest, root));
        }
    }
    out
}

/// `code` with every pixel length in rem of `root` pixels; names that end in "px" are left alone.
fn lengths_to_rem(code: &str, root: f64) -> String {
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(i) = rest.find("px") {
        let (head, tail) = (rest.get(..i).unwrap_or(""), rest.get(i + 2..).unwrap_or(""));
        // The number right before "px".
        let start = head.char_indices().rev().take_while(|(_, c)| c.is_ascii_digit() || *c == '.' || *c == '-').last().map_or(head.len(), |(j, _)| j);
        let (before, number) = (head.get(..start).unwrap_or(""), head.get(start..).unwrap_or(""));
        let free = !before.chars().next_back().is_some_and(ident) && !tail.chars().next().is_some_and(|c| ident(c) || c == '-');
        match number.parse::<f64>() {
            Ok(v) if free && v.is_finite() => {
                out.push_str(before);
                out.push_str(&format!("{}rem", num(v / root)));
            }
            _ => {
                out.push_str(head);
                out.push_str("px");
            }
        }
        rest = tail;
    }
    out.push_str(rest);
    out
}

pub fn code_for(doc: &Document, id: NodeId, lang: CodeLang) -> String {
    match lang {
        CodeLang::Css => css(doc, id),
        CodeLang::Html => html(doc, id),
        CodeLang::React | CodeLang::Leptos | CodeLang::SwiftUi | CodeLang::Compose => crate::component_codegen::code(doc, id, lang),
    }
}

/// What a tree walk generates for: the document, the layer the code is for (instances under it
/// become calls), and the component whose properties are parameters.
pub(crate) struct Ctx<'a> {
    pub doc: &'a Document,
    pub root: NodeId,
    pub comp: Option<&'a Component>,
    /// Indentation the walk starts at (the depth cap counts from it).
    pub base: usize,
}

impl<'a> Ctx<'a> {
    pub fn new(doc: &'a Document, root: NodeId) -> Self {
        Ctx { doc, root, comp: None, base: 0 }
    }

    /// The component parameter controlling `field` ("visible", "characters", "mainComponent",
    /// "slot") of `n`.
    pub fn param(&self, n: &Node, field: &str) -> Option<&'a Param> {
        let name = n.prop_refs.get(field)?;
        self.comp?.params.iter().find(|p| &p.prop == name)
    }

    /// Visible children in paint order (bottom first), capped; in component code, hidden layers a
    /// boolean parameter shows are included.
    pub fn kids(&self, n: &Node) -> Vec<NodeId> {
        n.children
            .iter()
            .copied()
            .filter(|c| self.doc.node(*c).is_some_and(|k| k.visible || self.param(k, "visible").is_some()))
            .take(MAX_CHILDREN)
            .collect()
    }

    fn too_deep(&self, depth: usize) -> bool {
        depth > self.base + MAX_DEPTH
    }

    /// A call for an instance below the root (`None` for other layers, or a missing main).
    fn call(&self, n: &Node, lang: CodeLang, depth: usize) -> Option<String> {
        if n.id == self.root || !matches!(n.kind, NodeKind::Instance { .. }) {
            return None;
        }
        crate::component_codegen::usage(self.doc, n.id, lang, depth)
    }
}

fn first_fill(n: &Node) -> Option<Color> {
    n.fills.iter().filter(|p| p.visible).find_map(|p| match &p.kind {
        PaintKind::Solid { color } => Some(color.with_alpha(color.a * p.opacity)),
        PaintKind::Gradient(g) => g.sorted_stops().first().map(|s| s.color),
        PaintKind::Image(_) | PaintKind::Pattern(_) => None,
    })
}

fn stroke_color(n: &Node) -> Option<Color> {
    if !n.has_visible_stroke() {
        return None;
    }
    n.strokes.iter().filter(|p| p.visible).find_map(|p| p.solid_color().map(|c| c.with_alpha(c.a * p.opacity)))
}

fn drop_shadow(n: &Node) -> Option<&Shadow> {
    n.effects.iter().filter(|e| e.visible).find_map(|e| match &e.kind {
        EffectKind::DropShadow(s) => Some(s),
        _ => None,
    })
}

fn radius(n: &Node) -> f64 {
    n.corners.uniform().unwrap_or_else(|| n.corners.radii.iter().copied().fold(0.0, f64::max))
}

/// Text content as a string literal (escaped for Swift; Kotlin also escapes `$`).
pub(crate) fn literal(s: &str, kotlin: bool) -> String {
    let mut out = String::from("\"");
    for c in s.chars().take(10_000) {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' | '\u{2028}' => out.push_str("\\n"),
            '\r' => {}
            '\t' => out.push_str("\\t"),
            '$' if kotlin => out.push_str("\\$"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub(crate) fn indent(depth: usize) -> String {
    "    ".repeat(depth)
}

// ------------------------------------------------------------------ CSS

/// The CSS custom properties generated code uses: name → fallback value, in first-use order.
#[derive(Default)]
pub(crate) struct Tokens(pub Vec<(String, String)>);

impl Tokens {
    /// `var(--name, value)`, remembering the variable.
    fn var(&mut self, variable: &Variable, value: String) -> String {
        let name = css_var_of(variable);
        let out = format!("var({name}, {value})");
        if !self.0.iter().any(|(n, _)| *n == name) {
            self.0.push((name, value));
        }
        out
    }

    /// A `:root` rule declaring the variables (empty without any).
    pub fn root_rule(&self) -> String {
        if self.0.is_empty() {
            return String::new();
        }
        let decls: String = self.0.iter().map(|(n, v)| format!("  {n}: {v};\n")).collect();
        format!(":root {{\n{decls}}}\n")
    }
}

/// A variable's CSS custom property name: "Colors/Brand 500" → `--colors-brand-500`.
pub fn css_var_name(variable: &str) -> String {
    let mut out = String::from("--");
    for c in variable.chars().flat_map(char::to_lowercase).take(80) {
        if c.is_alphanumeric() || c == '_' {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_end_matches('-');
    if trimmed.len() <= 2 { "--variable".into() } else { trimmed.to_string() }
}

/// A variable's CSS custom property: its Web code syntax (`var(--x)` or `--x`) when it has one,
/// else made from its name.
pub fn css_var_of(v: &Variable) -> String {
    let Some(web) = v.code_syntax.get("WEB").map(|w| w.trim()) else { return css_var_name(&v.name) };
    let inner = web.strip_prefix("var(").map_or(web, |r| r.split([',', ')']).next().unwrap_or("").trim());
    let valid = inner.len() > 2 && inner.starts_with("--") && inner.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_');
    if valid { inner.chars().take(80).collect() } else { css_var_name(inner) }
}

/// `value`, or a reference to the variable bound to the first of `fields` that has one.
fn tok(doc: &Document, n: &Node, fields: &[&str], value: String, t: &mut Tokens) -> String {
    match fields.iter().find_map(|f| crate::devinspect::bound_variable(doc, n, f)) {
        Some(v) => t.var(v, value),
        None => value,
    }
}

fn css(doc: &Document, id: NodeId) -> String {
    let Some(n) = doc.node(id) else { return String::new() };
    let mut t = Tokens::default();
    let decls = css_decls(doc, n, &mut t, (false, false));
    format!("/* {} */\n{decls}", n.name.replace("*/", "* /"))
}

/// The CSS declarations for `n`'s own look and size, one per line; `hug` leaves out the width
/// and/or height (they follow the content). Bound variables become `var(--name, value)`.
pub(crate) fn css_decls(doc: &Document, n: &Node, t: &mut Tokens, hug: (bool, bool)) -> String {
    let px = |v: f64| format!("{}px", num(v));
    let mut out = String::new();
    let mut put = |key: &str, value: String| out.push_str(&format!("{key}: {value};\n"));
    if !hug.0 {
        put("width", tok(doc, n, &["width"], px(n.width), t));
    }
    if !hug.1 {
        put("height", tok(doc, n, &["height"], px(n.height), t));
    }
    if let Some(l) = &n.layout {
        put("display", "flex".into());
        put("flex-direction", if l.mode == LayoutMode::Vertical { "column" } else { "row" }.into());
        if l.wrap {
            put("flex-wrap", "wrap".into());
        }
        put("gap", tok(doc, n, &["gap"], px(l.gap), t));
        let [pt, pr, pb, pl] = l.padding;
        let sides = [
            tok(doc, n, &["paddingTop", "paddingVertical", "padding"], px(pt), t),
            tok(doc, n, &["paddingRight", "paddingHorizontal", "padding"], px(pr), t),
            tok(doc, n, &["paddingBottom", "paddingVertical", "padding"], px(pb), t),
            tok(doc, n, &["paddingLeft", "paddingHorizontal", "padding"], px(pl), t),
        ];
        put("padding", sides.join(" "));
        let justify = match l.main_align {
            MainAlign::Min => "flex-start",
            MainAlign::Center => "center",
            MainAlign::Max => "flex-end",
            MainAlign::SpaceBetween => "space-between",
            MainAlign::SpaceAround => "space-around",
            MainAlign::SpaceEvenly => "space-evenly",
        };
        put("justify-content", justify.into());
        let align = match l.cross_align {
            CrossAlign::Min => "flex-start",
            CrossAlign::Center => "center",
            CrossAlign::Max => "flex-end",
            CrossAlign::Baseline => "baseline",
        };
        put("align-items", align.into());
        if l.wrap {
            // The lines: spread, or packed where the items align (baseline: at the start).
            let spread = l.content_align == ContentAlign::SpaceBetween;
            put(
                "align-content",
                if spread {
                    "space-between"
                } else if align == "baseline" {
                    "flex-start"
                } else {
                    align
                }
                .into(),
            );
        }
    }
    if matches!(n.kind, NodeKind::Ellipse { .. }) {
        put("border-radius", "50%".into());
    } else if let Some(r) = n.corners.uniform().filter(|r| *r > 0.0) {
        put("border-radius", tok(doc, n, &["cornerRadius"], px(r), t));
    } else if n.corners.radii.iter().any(|r| *r > 0.0) {
        let fields = ["topLeftRadius", "topRightRadius", "bottomRightRadius", "bottomLeftRadius"];
        let corners: Vec<String> = n.corners.radii.iter().zip(fields).map(|(r, f)| tok(doc, n, &[f, "cornerRadius"], px(*r), t)).collect();
        put("border-radius", corners.join(" "));
    }
    if n.opacity < 1.0 {
        put("opacity", tok(doc, n, &["opacity"], num(f64::from(n.opacity)), t));
    }
    if !matches!(n.blend, figcraft_color::BlendMode::Normal | figcraft_color::BlendMode::PassThrough) {
        put("mix-blend-mode", n.blend.css().into());
    }
    // Rotation turns counter-clockwise about the top-left corner (the position given); CSS
    // rotate turns clockwise.
    let rot = figcraft_geom::rotation_degrees(n.transform);
    if rot.abs() > 0.01 && rot.is_finite() {
        put("transform", format!("rotate({}deg)", num(-rot)));
        put("transform-origin", "top left".into());
    }
    for (i, p) in n.fills.iter().enumerate().filter(|(_, p)| p.visible) {
        match &p.kind {
            PaintKind::Solid { color } => {
                let key = if matches!(n.kind, NodeKind::Text(_)) { "color" } else { "background" };
                let c = color.with_alpha(color.a * p.opacity).to_css();
                // A variable binds the first paint's colour.
                put(key, if i == 0 { tok(doc, n, &["fill"], c, t) } else { c });
            }
            PaintKind::Gradient(g) => put("background", css_gradient(g, n.width, n.height)),
            PaintKind::Image(_) => put("background", "url(image.png) center / cover".into()),
            PaintKind::Pattern(_) => put("background", "url(pattern.png) repeat".into()),
        }
    }
    if let Some(c) = stroke_color(n) {
        let weight = tok(doc, n, &["strokeWeight"], px(n.stroke.weight), t);
        put("border", format!("{weight} solid {}", tok(doc, n, &["stroke"], c.to_css(), t)));
    }
    let shadows: Vec<String> = n
        .effects
        .iter()
        .filter(|e| e.visible)
        .filter_map(|e| match &e.kind {
            EffectKind::DropShadow(s) => {
                Some(format!("{}px {}px {}px {}px {}", num(s.offset.0), num(s.offset.1), num(s.radius), num(s.spread), s.color.to_css()))
            }
            EffectKind::InnerShadow(s) => {
                Some(format!("inset {}px {}px {}px {}px {}", num(s.offset.0), num(s.offset.1), num(s.radius), num(s.spread), s.color.to_css()))
            }
            _ => None,
        })
        .collect();
    if !shadows.is_empty() {
        put("box-shadow", shadows.join(", "));
    }
    for e in n.effects.iter().filter(|e| e.visible) {
        match e.kind {
            // CSS blurs are uniform: a progressive blur has no CSS equivalent and is left out.
            EffectKind::LayerBlur(ref b) if !b.is_progressive() => put("filter", format!("blur({}px)", num(b.radius / 2.0))),
            EffectKind::BackgroundBlur(ref b) if !b.is_progressive() => put("backdrop-filter", format!("blur({}px)", num(b.radius / 2.0))),
            // CSS has no refraction: the frost is the nearest.
            EffectKind::Glass(ref g) => put("backdrop-filter", format!("blur({}px)", num(g.sanitized().frost / 2.0))),
            _ => {}
        }
    }
    if let NodeKind::Text(tx) = &n.kind {
        let s = &tx.style;
        put("font-family", tok(doc, n, &["fontFamily"], format!("\"{}\"", s.font_family.replace('"', "'")), t));
        put("font-size", tok(doc, n, &["fontSize"], px(s.font_size), t));
        put("font-weight", s.weight().to_string());
        if s.italic() {
            put("font-style", "italic".into());
        }
        match s.line_height {
            LineHeight::Pixels(v) => put("line-height", tok(doc, n, &["lineHeight"], px(v), t)),
            LineHeight::Percent(p) => put("line-height", format!("{}%", num(p))),
            LineHeight::Auto => {}
        }
        match s.letter_spacing {
            LetterSpacing::Pixels(v) if v != 0.0 => put("letter-spacing", tok(doc, n, &["letterSpacing"], px(v), t)),
            LetterSpacing::Percent(p) if p != 0.0 => put("letter-spacing", format!("{}em", num(p / 100.0))),
            _ => {}
        }
        match tx.align_h {
            TextAlignH::Left => {}
            TextAlignH::Center => put("text-align", "center".into()),
            TextAlignH::Right => put("text-align", "right".into()),
            TextAlignH::Justified => put("text-align", "justify".into()),
        }
        match s.case {
            TextCase::Original => {}
            TextCase::Upper => put("text-transform", "uppercase".into()),
            TextCase::Lower => put("text-transform", "lowercase".into()),
            TextCase::Title => put("text-transform", "capitalize".into()),
            TextCase::SmallCaps => put("font-variant", "small-caps".into()),
        }
        match s.decoration {
            TextDecoration::None => {}
            TextDecoration::Underline => put("text-decoration", "underline".into()),
            TextDecoration::Strikethrough => put("text-decoration", "line-through".into()),
        }
        if s.decoration != TextDecoration::None {
            let l = s.look();
            let len = |m: LineHeight| match m {
                LineHeight::Auto => None,
                LineHeight::Pixels(v) => Some(px(v)),
                LineHeight::Percent(p) => Some(format!("{}%", num(p))),
            };
            match l.style {
                figcraft_doc::DecorationStyle::Solid => {}
                figcraft_doc::DecorationStyle::Dotted => put("text-decoration-style", "dotted".into()),
                figcraft_doc::DecorationStyle::Wavy => put("text-decoration-style", "wavy".into()),
            }
            if let Some(v) = len(l.thickness) {
                put("text-decoration-thickness", v);
            }
            if let Some(c) = l.color {
                put("text-decoration-color", c.to_css());
            }
            if s.decoration == TextDecoration::Underline {
                if let Some(v) = len(l.offset) {
                    put("text-underline-offset", v);
                }
                // Browsers skip ink by default; the canvas only when asked.
                if !l.skip_ink {
                    put("text-decoration-skip-ink", "none".into());
                }
            }
        }
        if tx.leading_trim == figcraft_doc::LeadingTrim::CapHeight {
            put("text-box", "trim-both cap alphabetic".into());
        }
    }
    out
}

/// A gradient fill on a `w`×`h` box as a CSS background: linear along its handles' direction
/// (CSS angles: 0deg up, 90deg right), radial with its handles' radii, angular as a conic
/// gradient from its start direction.
fn css_gradient(g: &figcraft_color::Gradient, w: f64, h: f64) -> String {
    let stops = g.sorted_stops().iter().map(|s| format!("{} {}%", s.color.to_css(), (s.position * 100.0).round())).collect::<Vec<_>>().join(", ");
    let (dx, dy) = ((g.to.0 - g.from.0) * w, (g.to.1 - g.from.1) * h);
    let angle = if dx == 0.0 && dy == 0.0 { 180.0 } else { dx.atan2(-dy).to_degrees().rem_euclid(360.0) };
    let pct = |v: f64| format!("{}%", num(v * 100.0));
    let at = format!("at {} {}", pct(g.from.0), pct(g.from.1));
    match g.kind {
        figcraft_color::GradientKind::Linear => format!("linear-gradient({}deg, {stops})", num(angle)),
        figcraft_color::GradientKind::Radial | figcraft_color::GradientKind::Diamond => {
            let (rx, ry) = (((g.width.0 - g.from.0) * w).hypot((g.width.1 - g.from.1) * h), dx.hypot(dy));
            let rel = |r: f64, side: f64| if side > 0.0 { pct(r / side) } else { "50%".into() };
            format!("radial-gradient({} {} {at}, {stops})", rel(rx, w), rel(ry, h))
        }
        figcraft_color::GradientKind::Angular => format!("conic-gradient(from {}deg {at}, {stops})", num(angle)),
    }
}

// ------------------------------------------------------------------ HTML and React markup

/// A CSS class name from a layer name ("Card / Title" → "card-title"), unique in `used`.
pub(crate) fn class_name(name: &str, used: &mut Vec<String>) -> String {
    let mut base = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            base.push(c);
        } else if !base.ends_with('-') && !base.is_empty() {
            base.push('-');
        }
    }
    let base = base.trim_end_matches('-').chars().take(40).collect::<String>();
    let base = if base.is_empty() || base.starts_with(|c: char| c.is_ascii_digit()) { format!("layer-{base}") } else { base };
    let name = (1..).map(|k| if k == 1 { base.clone() } else { format!("{base}-{k}") }).find(|n| !used.contains(n)).unwrap_or(base);
    used.push(name.clone());
    name
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('\n', "<br>")
}

/// Text as JSX children: HTML escapes, braces as expressions, line breaks as `<br />`.
fn jsx_text(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().take(10_000) {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '{' => out.push_str("{\"{\"}"),
            '}' => out.push_str("{\"}\"}"),
            '\n' => out.push_str("<br />"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// The whole tree under `id` as nested HTML with a stylesheet: auto layout frames become
/// flexboxes, other children are placed absolutely in their parent, text becomes paragraphs.
fn html(doc: &Document, id: NodeId) -> String {
    let ctx = Ctx::new(doc, id);
    let mut out = Markup::new(&ctx, CodeLang::Html);
    out.node(id, None, 0, "");
    format!("{}\n<style>\n{}{}</style>\n", out.markup, out.tokens.root_rule(), out.styles)
}

/// Markup being built for a layer tree: HTML (`class`), React JSX (`className`) or Leptos `view!`
/// markup (text as Rust string literals); the last two with component parameters and instances
/// as calls. With its stylesheet.
pub(crate) struct Markup<'c, 'a> {
    ctx: &'c Ctx<'a>,
    /// Html, React or Leptos.
    lang: CodeLang,
    used: Vec<String>,
    pub markup: String,
    pub styles: String,
    pub tokens: Tokens,
    /// The root's class (default: from its name) and extra JSX attributes.
    pub root_class: Option<String>,
    pub root_attrs: String,
    /// Each layer's path in the tree (names with sibling counts), class and declarations, so
    /// variants can be compared layer by layer.
    pub rules: Vec<(String, String, String)>,
}

impl<'c, 'a> Markup<'c, 'a> {
    pub fn new(ctx: &'c Ctx<'a>, lang: CodeLang) -> Self {
        Markup {
            ctx,
            lang,
            used: Vec::new(),
            markup: String::new(),
            styles: String::new(),
            tokens: Tokens::default(),
            root_class: None,
            root_attrs: String::new(),
            rules: Vec::new(),
        }
    }

    /// JSX indents by two spaces, HTML and Leptos by four.
    fn pad(&self, depth: usize) -> String {
        if self.lang == CodeLang::React { "  ".repeat(depth) } else { indent(depth) }
    }

    /// Component code (React or Leptos): parameters and instances as calls.
    fn code(&self) -> bool {
        self.lang != CodeLang::Html
    }

    /// A parameter's name in the generated code.
    fn ident(&self, p: &Param) -> String {
        if self.lang == CodeLang::Leptos { crate::component_codegen::rust_ident(&p.ident) } else { p.ident.clone() }
    }

    /// A layer, shown only while its boolean parameter is on (component code).
    pub fn node(&mut self, id: NodeId, parent: Option<&Node>, depth: usize, path: &str) {
        let Some(n) = self.ctx.doc.node(id) else { return };
        if self.code()
            && id != self.ctx.root
            && let Some(p) = self.ctx.param(n, "visible")
        {
            let (pad, p) = (self.pad(depth), self.ident(p));
            let (open, close) =
                if self.lang == CodeLang::Leptos { (format!("{{{p}.then(|| view! {{"), "})}") } else { (format!("{{{p} && ("), ")}") };
            self.markup.push_str(&format!("{pad}{open}\n"));
            self.element(n, parent, depth + 1, path);
            self.markup.push_str(&format!("{pad}{close}\n"));
            return;
        }
        self.element(n, parent, depth, path);
    }

    fn rule(&mut self, path: &str, class: &str, decls: String) {
        self.styles.push_str(&format!(".{class} {{\n"));
        for line in decls.lines() {
            self.styles.push_str(&format!("  {line}\n"));
        }
        self.styles.push_str("}\n");
        self.rules.push((path.to_string(), class.to_string(), decls));
    }

    fn element(&mut self, n: &Node, parent: Option<&Node>, depth: usize, path: &str) {
        let doc = self.ctx.doc;
        let root = n.id == self.ctx.root;
        let class = match self.root_class.take().filter(|_| root) {
            Some(c) => {
                self.used.push(c.clone());
                c
            }
            None => class_name(&n.name, &mut self.used),
        };
        let pad = self.pad(depth);
        let attr = if self.lang == CodeLang::React { "className" } else { "class" };
        // How it sits in its parent: in the flex flow, or placed at its position.
        let mut decls = String::new();
        let in_flow = parent.is_some_and(|p| p.layout.is_some()) && !n.child_layout.absolute;
        if root {
            decls.push_str("position: relative;\n");
        } else if in_flow {
            let vertical = parent.and_then(|p| p.layout.as_ref()).is_some_and(|l| l.mode == LayoutMode::Vertical);
            let (main, cross) =
                if vertical { (n.child_layout.sizing_v, n.child_layout.sizing_h) } else { (n.child_layout.sizing_h, n.child_layout.sizing_v) };
            if main == Sizing::Fill {
                decls.push_str("flex: 1 1 0;\n");
            }
            if cross == Sizing::Fill {
                decls.push_str("align-self: stretch;\n");
            }
        } else {
            let c = n.transform.as_coeffs();
            decls.push_str(&format!("position: absolute;\nleft: {}px;\ntop: {}px;\n", num(c[4]), num(c[5])));
            let rot = figcraft_geom::rotation_degrees(n.transform);
            if rot.abs() > 0.01 {
                decls.push_str(&format!("transform: rotate({}deg);\ntransform-origin: 0 0;\n", num(-rot)));
            }
        }
        // Nested instances are calls (component code), in a wrapper that places them when needed.
        if self.code() && !root && matches!(n.kind, NodeKind::Instance { .. }) {
            let at = depth + usize::from(!decls.is_empty());
            let run = if self.lang == CodeLang::Leptos { ".run()" } else { "" };
            let inner = match self.ctx.param(n, "mainComponent") {
                Some(p) => Some(format!("{}{{{}{run}}}\n", self.pad(at), self.ident(p))),
                None => self.ctx.call(n, self.lang, at),
            };
            if let Some(inner) = inner {
                if decls.is_empty() {
                    self.markup.push_str(&inner);
                } else {
                    self.markup.push_str(&format!("{pad}<div {attr}=\"{class}\">\n{inner}{pad}</div>\n"));
                    self.rule(path, &class, decls);
                }
                return;
            }
        }
        let hugs = n.layout.is_some() || matches!(n.kind, NodeKind::Text(_));
        let hug = (hugs && n.child_layout.sizing_h == Sizing::Hug, hugs && n.child_layout.sizing_v == Sizing::Hug);
        let mut own = css_decls(doc, n, &mut self.tokens, hug);
        if n.layout.is_none() && !n.children.is_empty() && !root && in_flow {
            own.push_str("position: relative;\n");
        }
        decls.push_str(&own);
        self.rule(path, &class, decls);
        let attrs = if root { self.root_attrs.clone() } else { String::new() };
        let open = |tag: &str| format!("{pad}<{tag} {attr}=\"{class}\"{attrs}>");
        match &n.kind {
            NodeKind::Text(t) => {
                let body = match (self.ctx.param(n, "characters").filter(|_| self.code()), self.lang) {
                    (Some(p), CodeLang::Leptos) => format!("{{{}.clone()}}", self.ident(p)),
                    (Some(p), _) => format!("{{{}}}", p.ident),
                    (None, CodeLang::React) => jsx_text(&t.characters),
                    // `view!` takes text as string literals and void elements self-closed.
                    (None, CodeLang::Leptos) => {
                        t.characters.chars().take(10_000).collect::<String>().lines().map(|l| format!("{l:?}")).collect::<Vec<_>>().join("<br/>")
                    }
                    (None, _) => html_escape(&t.characters),
                };
                self.markup.push_str(&format!("{}{body}</p>\n", open("p")));
            }
            _ => {
                let kids = self.ctx.kids(n);
                let slot = self.ctx.param(n, "slot").filter(|_| self.code());
                let leptos = self.lang == CodeLang::Leptos;
                let open = open("div");
                if let Some(p) = slot.filter(|_| kids.is_empty()) {
                    let content = if leptos { format!("{}.as_ref().map(|c| c.run())", self.ident(p)) } else { p.ident.clone() };
                    self.markup.push_str(&format!("{open}{{{content}}}</div>\n"));
                } else if kids.is_empty() || self.ctx.too_deep(depth) {
                    self.markup.push_str(&format!("{open}</div>\n"));
                } else {
                    self.markup.push_str(&format!("{open}\n"));
                    // A slot shows the instance's content, or else the main's.
                    let (p1, p2) = (self.pad(depth + 1), self.pad(depth + 2));
                    let inner = match slot {
                        Some(p) if leptos => {
                            self.markup.push_str(&format!("{p1}{{match &{} {{ Some(c) => c.run(), None => view! {{\n", self.ident(p)));
                            depth + 2
                        }
                        Some(p) => {
                            self.markup.push_str(&format!("{p1}{{{} ?? (\n{p2}<>\n", p.ident));
                            depth + 3
                        }
                        None => depth + 1,
                    };
                    let mut seen: Vec<&str> = Vec::new();
                    // Bottom layer first: later elements paint on top, like the layer order.
                    for k in kids {
                        let Some(kn) = doc.node(k) else { continue };
                        let count = seen.iter().filter(|s| **s == kn.name).count();
                        seen.push(&kn.name);
                        self.node(k, Some(n), inner, &format!("{path}/{}#{count}", kn.name));
                    }
                    match slot {
                        Some(_) if leptos => self.markup.push_str(&format!("{p1}}}.into_any() }}}}\n")),
                        Some(_) => self.markup.push_str(&format!("{p2}</>\n{p1})}}\n")),
                        None => {}
                    }
                    self.markup.push_str(&format!("{pad}</div>\n"));
                }
            }
        }
    }
}

// ------------------------------------------------------------------ SwiftUI

fn swift_color(c: Color) -> String {
    format!("Color(red: {}, green: {}, blue: {}, opacity: {})", num(f64::from(c.r)), num(f64::from(c.g)), num(f64::from(c.b)), num(f64::from(c.a)))
}

fn swift_weight(w: u16) -> &'static str {
    match w {
        0..=150 => ".thin",
        151..=250 => ".ultraLight",
        251..=350 => ".light",
        351..=450 => ".regular",
        451..=550 => ".medium",
        551..=650 => ".semibold",
        651..=750 => ".bold",
        751..=850 => ".heavy",
        _ => ".black",
    }
}

/// SwiftUI for a layer and its subtree at `depth`; `extra` modifiers go last (a free child's
/// offset). A layer a boolean parameter shows sits in an `if`.
pub(crate) fn swift(ctx: &Ctx, id: NodeId, depth: usize, out: &mut String, extra: &[String]) {
    let Some(n) = ctx.doc.node(id) else { return };
    if id != ctx.root
        && let Some(p) = ctx.param(n, "visible")
    {
        let pad = indent(depth);
        out.push_str(&format!("{pad}if {} {{\n", p.ident));
        swift_layer(ctx, n, depth + 1, out, extra);
        out.push_str(&format!("{pad}}}\n"));
        return;
    }
    swift_layer(ctx, n, depth, out, extra);
}

fn swift_layer(ctx: &Ctx, n: &Node, depth: usize, out: &mut String, extra: &[String]) {
    let doc = ctx.doc;
    let pad = indent(depth);
    if ctx.too_deep(depth) {
        out.push_str(&format!("{pad}// {} …\n", n.name));
        return;
    }
    // Nested instances are calls; an instance swap parameter is a view.
    let call = match ctx.param(n, "mainComponent").filter(|_| n.id != ctx.root) {
        Some(p) => Some(format!("{pad}{}\n", p.ident)),
        None => ctx.call(n, CodeLang::SwiftUi, depth),
    };
    if let Some(call) = call {
        out.push_str(&call);
        for m in extra {
            out.push_str(&format!("{pad}    {m}\n"));
        }
        return;
    }
    let mut mods: Vec<String> = Vec::new();
    match &n.kind {
        NodeKind::Text(t) => {
            let text = ctx.param(n, "characters").map_or_else(|| literal(&t.characters, false), |p| p.ident.clone());
            out.push_str(&format!("{pad}Text({text})\n"));
            mods.push(format!(
                ".font(.custom({}, size: {}).weight({}))",
                literal(&t.style.font_family, false),
                num(t.style.font_size),
                swift_weight(t.style.weight())
            ));
            if let Some(c) = first_fill(n) {
                mods.push(format!(".foregroundColor({})", swift_color(c)));
            }
        }
        NodeKind::Ellipse { .. } => {
            out.push_str(&format!("{pad}Ellipse()\n"));
            if let Some(c) = first_fill(n) {
                mods.push(format!(".fill({})", swift_color(c)));
            }
            mods.push(format!(".frame(width: {}, height: {})", num(n.width), num(n.height)));
        }
        NodeKind::Rectangle => {
            out.push_str(&format!("{pad}RoundedRectangle(cornerRadius: {})\n", num(radius(n))));
            if let Some(c) = first_fill(n) {
                mods.push(format!(".fill({})", swift_color(c)));
            }
            mods.push(format!(".frame(width: {}, height: {})", num(n.width), num(n.height)));
        }
        k if k.is_frame_like() || matches!(k, NodeKind::Group) => {
            let children = ctx.kids(n);
            let open = match n.layout.as_ref().map(|l| (l.mode, l)) {
                Some((LayoutMode::Vertical, l)) => format!("VStack(alignment: .leading, spacing: {}) {{", num(l.gap)),
                Some((_, l)) => format!("HStack(alignment: .center, spacing: {}) {{", num(l.gap)),
                None => "ZStack(alignment: .topLeading) {".to_string(),
            };
            out.push_str(&format!("{pad}{open}\n"));
            // A slot shows the instance's content, or else the main's.
            let slot = ctx.param(n, "slot");
            let inner = match slot {
                Some(p) => {
                    out.push_str(&format!("{pad}    if let {0} {{\n{pad}        {0}\n{pad}    }} else {{\n", p.ident));
                    depth + 2
                }
                None => depth + 1,
            };
            for c in &children {
                // Free children sit at their offset inside a ZStack.
                let offset: Vec<String> = match doc.node(*c).filter(|_| n.layout.is_none()) {
                    Some(cn) => {
                        let [_, _, _, _, x, y] = cn.transform.as_coeffs();
                        vec![format!(".offset(x: {}, y: {})", num(x), num(y))]
                    }
                    None => Vec::new(),
                };
                swift(ctx, *c, inner, out, &offset);
            }
            if slot.is_some() {
                out.push_str(&format!("{pad}    }}\n"));
            }
            out.push_str(&format!("{pad}}}\n"));
            if let Some(l) = &n.layout {
                let [t, r, b, lft] = l.padding;
                if [t, r, b, lft].iter().any(|v| *v > 0.0) {
                    mods.push(format!(".padding(EdgeInsets(top: {}, leading: {}, bottom: {}, trailing: {}))", num(t), num(lft), num(b), num(r)));
                }
            }
            mods.push(format!(".frame(width: {}, height: {}, alignment: .topLeading)", num(n.width), num(n.height)));
            if let Some(c) = first_fill(n) {
                mods.push(format!(".background({})", swift_color(c)));
            }
            if radius(n) > 0.0 {
                mods.push(format!(".clipShape(RoundedRectangle(cornerRadius: {}))", num(radius(n))));
            }
        }
        _ => {
            out.push_str(&format!("{pad}// {}: export as an image asset\n{pad}Image({})\n", n.kind.label(), literal(&n.name, false)));
            mods.push(".resizable()".into());
            mods.push(format!(".frame(width: {}, height: {})", num(n.width), num(n.height)));
        }
    }
    if let Some(c) = stroke_color(n) {
        mods.push(format!(
            ".overlay(RoundedRectangle(cornerRadius: {}).stroke({}, lineWidth: {}))",
            num(radius(n)),
            swift_color(c),
            num(n.stroke.weight)
        ));
    }
    if let Some(s) = drop_shadow(n) {
        mods.push(format!(
            ".shadow(color: {}, radius: {}, x: {}, y: {})",
            swift_color(s.color),
            num(s.radius / 2.0),
            num(s.offset.0),
            num(s.offset.1)
        ));
    }
    if n.opacity < 1.0 {
        mods.push(format!(".opacity({})", num(f64::from(n.opacity))));
    }
    mods.extend(extra.iter().cloned());
    for m in mods {
        out.push_str(&format!("{pad}    {m}\n"));
    }
}

// ------------------------------------------------------------------ Jetpack Compose

fn kotlin_color(c: Color) -> String {
    let [r, g, b, a] = c.to_rgba8();
    format!("Color(0x{a:02X}{r:02X}{g:02X}{b:02X})")
}

/// The Modifier chain for a layer (size, shadow, background, border, alpha, padding).
fn modifier(n: &Node, shape: &str, padding: bool) -> String {
    let mut m = format!("Modifier.size({}.dp, {}.dp)", num(n.width), num(n.height));
    if let Some(s) = drop_shadow(n) {
        m.push_str(&format!(".shadow({}.dp, {shape})", num(s.radius / 2.0)));
    }
    if let Some(c) = first_fill(n) {
        m.push_str(&format!(".background({}, {shape})", kotlin_color(c)));
    }
    if let Some(c) = stroke_color(n) {
        m.push_str(&format!(".border({}.dp, {}, {shape})", num(n.stroke.weight), kotlin_color(c)));
    }
    if n.opacity < 1.0 {
        m.push_str(&format!(".alpha({}f)", num(f64::from(n.opacity))));
    }
    if padding && let Some(l) = &n.layout {
        let [t, r, b, lft] = l.padding;
        if [t, r, b, lft].iter().any(|v| *v > 0.0) {
            m.push_str(&format!(".padding(start = {}.dp, top = {}.dp, end = {}.dp, bottom = {}.dp)", num(lft), num(t), num(r), num(b)));
        }
    }
    m
}

/// Jetpack Compose for a layer and its subtree at `depth`. A layer a boolean parameter shows sits
/// in an `if`.
pub(crate) fn compose(ctx: &Ctx, id: NodeId, depth: usize, out: &mut String) {
    let Some(n) = ctx.doc.node(id) else { return };
    if id != ctx.root
        && let Some(p) = ctx.param(n, "visible")
    {
        let pad = indent(depth);
        out.push_str(&format!("{pad}if ({}) {{\n", p.ident));
        compose_layer(ctx, n, depth + 1, out);
        out.push_str(&format!("{pad}}}\n"));
        return;
    }
    compose_layer(ctx, n, depth, out);
}

fn compose_layer(ctx: &Ctx, n: &Node, depth: usize, out: &mut String) {
    let doc = ctx.doc;
    let pad = indent(depth);
    if ctx.too_deep(depth) {
        out.push_str(&format!("{pad}// {} …\n", n.name));
        return;
    }
    // Nested instances are calls; an instance swap parameter is a composable.
    let call = match ctx.param(n, "mainComponent").filter(|_| n.id != ctx.root) {
        Some(p) => Some(format!("{pad}{}()\n", p.ident)),
        None => ctx.call(n, CodeLang::Compose, depth),
    };
    if let Some(call) = call {
        out.push_str(&call);
        return;
    }
    let shape = if radius(n) > 0.0 { format!("RoundedCornerShape({}.dp)", num(radius(n))) } else { "RectangleShape".to_string() };
    match &n.kind {
        NodeKind::Text(t) => {
            let color = first_fill(n).map(kotlin_color).unwrap_or_else(|| "Color.Unspecified".into());
            let text = ctx.param(n, "characters").map_or_else(|| literal(&t.characters, true), |p| p.ident.clone());
            out.push_str(&format!(
                "{pad}Text(\n{pad}    text = {text},\n{pad}    fontSize = {}.sp,\n{pad}    fontWeight = FontWeight.W{},\n{pad}    color = {color},\n{pad})\n",
                num(t.style.font_size),
                (t.style.weight().clamp(100, 900) / 100) * 100
            ));
        }
        NodeKind::Ellipse { .. } => out.push_str(&format!("{pad}Box(modifier = {})\n", modifier(n, "CircleShape", false))),
        NodeKind::Rectangle => out.push_str(&format!("{pad}Box(modifier = {})\n", modifier(n, &shape, false))),
        k if k.is_frame_like() || matches!(k, NodeKind::Group) => {
            let children = ctx.kids(n);
            let open = match n.layout.as_ref().map(|l| (l.mode, l)) {
                Some((LayoutMode::Vertical, l)) => format!(
                    "Column(\n{pad}    modifier = {},\n{pad}    verticalArrangement = Arrangement.spacedBy({}.dp),\n{pad}) {{",
                    modifier(n, &shape, true),
                    num(l.gap)
                ),
                Some((_, l)) => format!(
                    "Row(\n{pad}    modifier = {},\n{pad}    horizontalArrangement = Arrangement.spacedBy({}.dp),\n{pad}    verticalAlignment = Alignment.CenterVertically,\n{pad}) {{",
                    modifier(n, &shape, true),
                    num(l.gap)
                ),
                None => format!("Box(modifier = {}) {{", modifier(n, &shape, false)),
            };
            out.push_str(&format!("{pad}{open}\n"));
            // A slot shows the instance's content, or else the main's.
            let slot = ctx.param(n, "slot");
            let base = match slot {
                Some(p) => {
                    out.push_str(&format!("{pad}    if ({0} != null) {{\n{pad}        {0}()\n{pad}    }} else {{\n", p.ident));
                    depth + 1
                }
                None => depth,
            };
            for c in &children {
                if n.layout.is_none()
                    && let Some(cn) = doc.node(*c)
                {
                    let [_, _, _, _, x, y] = cn.transform.as_coeffs();
                    out.push_str(&format!("{}Box(modifier = Modifier.offset({}.dp, {}.dp)) {{\n", indent(base + 1), num(x), num(y)));
                    compose(ctx, *c, base + 2, out);
                    out.push_str(&format!("{}}}\n", indent(base + 1)));
                } else {
                    compose(ctx, *c, base + 1, out);
                }
            }
            if slot.is_some() {
                out.push_str(&format!("{pad}    }}\n"));
            }
            out.push_str(&format!("{pad}}}\n"));
        }
        _ => out.push_str(&format!(
            "{pad}// {}: export as an image asset\n{pad}Image(painter = painterResource(R.drawable.asset), contentDescription = {}, modifier = Modifier.size({}.dp, {}.dp))\n",
            n.kind.label(),
            literal(&n.name, true),
            num(n.width),
            num(n.height)
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;
    use crate::params::id_of;
    use serde_json::json;

    fn card(s: &mut Session) -> NodeId {
        let f = id_of(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 100, "name": "Card"})).unwrap()["id"]).unwrap();
        s.execute("node.set", &json!({"id": f, "props": {"fills": ["#FFFFFF"], "cornerRadius": 12}})).unwrap();
        s.execute("text.create", &json!({"text": "Say \"hi\" $5\nnow", "x": 0, "y": 0, "parent": f})).unwrap();
        s.execute("shape.ellipse", &json!({"x": 0, "y": 0, "width": 24, "height": 24, "parent": f})).unwrap();
        s.execute("layout.add", &json!({"ids": [f]})).unwrap();
        f
    }

    #[test]
    fn swiftui_and_compose_for_an_auto_layout_card() {
        let mut s = Session::new();
        let f = card(&mut s);
        let doc = s.doc().unwrap();
        let sw = code_for(doc, f, CodeLang::SwiftUi);
        assert!(sw.contains("Stack(alignment:"), "{sw}");
        assert!(sw.contains("Text(\"Say \\\"hi\\\" $5\\nnow\")"), "escaped: {sw}");
        assert!(sw.contains("Ellipse()"));
        assert!(sw.contains(".clipShape(RoundedRectangle(cornerRadius: 12))"));
        let kt = code_for(doc, f, CodeLang::Compose);
        assert!(kt.contains("Arrangement.spacedBy("), "{kt}");
        assert!(kt.contains("\\$5"), "Kotlin escapes $: {kt}");
        assert!(kt.contains("CircleShape"));
        assert!(kt.contains("RoundedCornerShape(12.dp)"));
        let css = code_for(doc, f, CodeLang::Css);
        assert!(css.contains("display: flex;") && css.contains("border-radius: 12px;"));
        // Through the command.
        let v = s.execute("inspect.code", &json!({"id": f, "language": "compose"})).unwrap();
        assert_eq!(v["code"].as_str().unwrap(), kt);
        assert!(s.execute("inspect.code", &json!({"id": f, "language": "cobol"})).is_err());
    }

    #[test]
    fn free_children_get_offsets_and_deep_trees_are_capped() {
        let mut s = Session::new();
        let mut parent = id_of(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap()["id"]).unwrap();
        let root = parent;
        s.execute("shape.rectangle", &json!({"x": 7, "y": 9, "width": 5, "height": 5, "parent": root})).unwrap();
        for _ in 0..20 {
            parent = id_of(&s.execute("frame.create", &json!({"x": 1, "y": 1, "width": 40, "height": 40, "parent": parent})).unwrap()["id"]).unwrap();
        }
        let doc = s.doc().unwrap();
        let sw = code_for(doc, root, CodeLang::SwiftUi);
        assert!(sw.contains(".offset(x: 7, y: 9)"), "{sw}");
        assert!(sw.contains("…"), "depth capped");
        let kt = code_for(doc, root, CodeLang::Compose);
        assert!(kt.contains("Modifier.offset(7.dp, 9.dp)"));
        assert!(code_for(doc, NodeId(999_999), CodeLang::Css).is_empty());
    }
}
