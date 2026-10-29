//! Code for a layer, as shown by the Inspect panel and returned by `inspect.code`: CSS for the
//! layer itself, HTML or React for its subtree, and SwiftUI, Jetpack Compose, ArkUI (HarmonyOS) or
//! Flutter (stacks for auto layout, absolute offsets otherwise). Components become components
//! whose properties are parameters and instances become calls passing their values (see
//! [`crate::component_codegen`]); variables bound to a layer become CSS custom properties. The
//! output is a starting point, not a pixel-exact translation.

use figcraft_color::{Color, PaintKind};
use figcraft_doc::{
    ContentAlign, CrossAlign, Document, EffectKind, LayoutMode, LetterSpacing, LineHeight, MainAlign, Node, NodeId, NodeKind, Shadow, Sizing,
    TextAlignH, TextCase, TextDecoration, Variable,
};
use serde_json::json;

use crate::cmd::CommandSpec;
use crate::component_codegen::{Component, Param};
use crate::params::{get, id_of, num_or, string, targets};
use crate::{bad, spec};

mod arkui;
mod compose;
mod css;
mod flutter;
mod html;
mod swift;

pub(crate) use arkui::arkui;
pub(crate) use compose::{compose, kotlin_color};
pub(crate) use css::Tokens;
#[cfg(test)]
pub(crate) use css::css_decls;
use css::{css, tok};
pub use css::{css_var_name, css_var_of};
pub(crate) use flutter::{FLUTTER_IMPORT, Place, flutter, flutter_indent};
use html::html;
pub(crate) use html::{Markup, class_name};
pub(crate) use swift::swift;

pub fn specs() -> Vec<CommandSpec> {
    vec![spec!(
        "inspect.code",
        "Code for layer",
        "{id?, language?: css|html|react|leptos|vue|tailwind|swiftui|compose|arkui|flutter (default css; html, react, leptos and vue are the whole layer tree; components take their properties as parameters, instances pass their values; a component connected to code (codeConnect.set) gives its import and snippet instead; tailwind is React with Tailwind CSS v4 utilities for pages: fluid auto layout boxes, instances inlined with their overrides, bound variables as theme utilities), mobile? (tailwind: a mobile frame with the same layer names; the code is mobile first with the desktop layer's classes behind lg:), unit?: px|rem (CSS, HTML, React, Leptos and Vue; default px), rootFontSize? (rem: pixels per rem, default 16)} → {code, language, connected}",
        false,
        |s, p| {
            let cmd = "inspect.code";
            let id = targets(s, p).first().copied().ok_or_else(|| bad(cmd, "select a layer"))?;
            let lang = match string(p, "language") {
                Some(l) => {
                    CodeLang::from_id(&l).ok_or_else(|| bad(cmd, "language: css|html|react|leptos|vue|tailwind|swiftui|compose|arkui|flutter"))?
                }
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
            let mobile = match get(p, "mobile") {
                None | Some(serde_json::Value::Null) => None,
                Some(v) => Some(id_of(v).filter(|m| doc.contains(*m)).ok_or_else(|| bad(cmd, "mobile: the id of a mobile frame"))?),
            };
            let code = match (lang, mobile) {
                (CodeLang::Tailwind, Some(m)) => crate::tailwind::code(doc, id, Some(m)),
                _ => code_for(doc, id, lang),
            };
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
    /// A Vue single-file component (`<template>`, `<script setup>`, `<style scoped>`).
    Vue,
    /// A React function component styled with Tailwind CSS v4 utilities (no stylesheet).
    Tailwind,
    SwiftUi,
    Compose,
    /// HarmonyOS ArkUI: an ArkTS struct (`@Component`, `build`).
    ArkUi,
    /// A Flutter `StatelessWidget` (Dart) with its widget tree.
    Flutter,
}

impl CodeLang {
    pub const ALL: [CodeLang; 10] = [
        CodeLang::Css,
        CodeLang::Html,
        CodeLang::React,
        CodeLang::Leptos,
        CodeLang::Vue,
        CodeLang::Tailwind,
        CodeLang::SwiftUi,
        CodeLang::Compose,
        CodeLang::ArkUi,
        CodeLang::Flutter,
    ];
    pub fn id(self) -> &'static str {
        match self {
            CodeLang::Css => "css",
            CodeLang::Html => "html",
            CodeLang::React => "react",
            CodeLang::Leptos => "leptos",
            CodeLang::Vue => "vue",
            CodeLang::Tailwind => "tailwind",
            CodeLang::SwiftUi => "swiftui",
            CodeLang::Compose => "compose",
            CodeLang::ArkUi => "arkui",
            CodeLang::Flutter => "flutter",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            CodeLang::Css => "CSS",
            CodeLang::Html => "HTML + CSS",
            CodeLang::React => "React",
            CodeLang::Leptos => "Leptos",
            CodeLang::Vue => "Vue",
            CodeLang::Tailwind => "React + Tailwind",
            CodeLang::SwiftUi => "SwiftUI",
            CodeLang::Compose => "Compose",
            CodeLang::ArkUi => "ArkUI",
            CodeLang::Flutter => "Flutter",
        }
    }
    /// Languages whose lengths are CSS pixels that can be shown in rem (not Tailwind: its lengths
    /// are utility classes).
    pub fn is_web(self) -> bool {
        matches!(self, CodeLang::Css | CodeLang::Html | CodeLang::React | CodeLang::Leptos | CodeLang::Vue)
    }
    /// Languages whose strings interpolate `$` (Kotlin's `$name`, Dart's `$name`), where a literal
    /// `$` has to be escaped.
    pub fn escapes_dollar(self) -> bool {
        matches!(self, CodeLang::Compose | CodeLang::Flutter)
    }
    pub fn from_id(s: &str) -> Option<CodeLang> {
        if s.eq_ignore_ascii_case("tsx") || s.eq_ignore_ascii_case("jsx") {
            return Some(CodeLang::React);
        }
        if s.eq_ignore_ascii_case("sfc") || s.eq_ignore_ascii_case("vue3") {
            return Some(CodeLang::Vue);
        }
        if s.eq_ignore_ascii_case("arkts") {
            return Some(CodeLang::ArkUi);
        }
        if s.eq_ignore_ascii_case("dart") {
            return Some(CodeLang::Flutter);
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
    if (r - r.round()).abs() < 1e-9 {
        // `as i64` saturates past 2^63, so larger whole values are printed from the float.
        if r.abs() < 9e18 { format!("{}", r.round() as i64) } else { format!("{:.0}", r.round()) }
    } else {
        format!("{r}")
    }
}

/// `v` without trailing zeros (6 decimals), for lengths that must convert back to the same pixels.
fn num6(v: f64) -> String {
    let r = (v * 1_000_000.0).round() / 1_000_000.0;
    if r.is_finite() && r != 0.0 { format!("{r}") } else { "0".into() }
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
                out.push_str(&format!("{}rem", num6(v / root)));
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
        CodeLang::Tailwind => crate::tailwind::code(doc, id, None),
        CodeLang::React | CodeLang::Leptos | CodeLang::Vue | CodeLang::SwiftUi | CodeLang::Compose | CodeLang::ArkUi | CodeLang::Flutter => {
            crate::component_codegen::code(doc, id, lang)
        }
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

    pub(crate) fn too_deep(&self, depth: usize) -> bool {
        depth > self.base + MAX_DEPTH
    }

    /// A call for an instance below the root (`None` for other layers, or a missing main), with
    /// the CSS its slot content generated.
    fn call(&self, n: &Node, lang: CodeLang, depth: usize) -> Option<(String, String)> {
        if n.id == self.root || !matches!(n.kind, NodeKind::Instance { .. }) {
            return None;
        }
        crate::component_codegen::usage_with_styles(self.doc, n.id, lang, depth)
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
    n.strokes.iter().filter(|p| p.visible).find_map(|p| p.solid_color())
}

/// The CSS borders a solid stroke makes: one `border`, or with individual stroke sides a border
/// on each side that has a weight.
fn css_borders(doc: &Document, n: &Node, t: &mut Tokens) -> Vec<(String, String)> {
    let Some(c) = stroke_color(n) else { return Vec::new() };
    let Some(sides) = n.stroke.sides else {
        let weight = tok(doc, n, &["strokeWeight"], format!("{}px", num(n.stroke.weight)), t);
        return vec![("border".into(), format!("{weight} solid {}", tok(doc, n, &["stroke"], c.to_css(), t)))];
    };
    let color = tok(doc, n, &["stroke"], c.to_css(), t);
    let named = ["top", "right", "bottom", "left"].into_iter().zip(crate::cmd::props::STROKE_SIDES);
    named
        .zip(sides)
        .filter(|(_, weight)| *weight > 0.0)
        .map(|((side, field), weight)| {
            (format!("border-{side}"), format!("{} solid {color}", tok(doc, n, &[field], format!("{}px", num(weight)), t)))
        })
        .collect()
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

/// The index of `markup`'s first tag-closing `>`, skipping `>` inside quoted attribute values.
fn tag_end(markup: &str) -> Option<usize> {
    let mut quote = None;
    for (i, c) in markup.char_indices() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '>' => return Some(i),
                _ => {}
            },
        }
    }
    None
}

/// `markup`'s first element with a Vue `v-if` attribute (a call with no placing wrapper).
fn with_v_if(markup: &str, guard: Option<&str>) -> String {
    let Some(guard) = guard else { return markup.to_string() };
    let Some(end) = tag_end(markup) else { return markup.to_string() };
    let before = markup[..end].trim_end();
    let at = if before.ends_with('/') { before.len() - 1 } else { end };
    format!("{} v-if=\"{guard}\"{}", &markup[..at], &markup[at..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;
    use crate::params::id_of;
    use serde_json::json;

    #[test]
    fn num_keeps_huge_whole_numbers() {
        assert_eq!(num(1e20), "100000000000000000000");
        assert_eq!(num(-1e20), "-100000000000000000000");
        assert_eq!(num(1e18), "1000000000000000000");
        assert_eq!(num(-0.0), "0");
    }

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
    fn arkui_for_an_auto_layout_card() {
        let mut s = Session::new();
        let f = card(&mut s);
        let doc = s.doc().unwrap();
        let ark = code_for(doc, f, CodeLang::ArkUi);
        for want in [
            "Row({ space: ",
            "Text(\"Say \\\"hi\\\" $5\\nnow\")",
            ".fontSize(",
            ".fontWeight(",
            "Ellipse({ width: 24, height: 24 })",
            ".backgroundColor('#FFFFFF')",
            ".borderRadius(12)",
            ".width(",
            ".height(100)",
            ".clip(true)",
        ] {
            assert!(ark.contains(want), "ArkUI has {want:?}:\n{ark}");
        }
        // Through the command, and "arkts" reads as ArkUI too.
        let v = s.execute("inspect.code", &json!({"id": f, "language": "arkui"})).unwrap();
        assert_eq!(v["code"].as_str().unwrap(), ark);
        assert_eq!(v["language"], "arkui");
        assert_eq!(s.execute("inspect.code", &json!({"id": f, "language": "arkts"})).unwrap()["language"], "arkui");
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
        let ark = code_for(doc, root, CodeLang::ArkUi);
        assert!(ark.contains(".position({ x: 7, y: 9 })"), "{ark}");
        assert!(ark.contains("…"), "depth capped");
        assert!(code_for(doc, NodeId(999_999), CodeLang::Css).is_empty());
    }

    #[test]
    fn arkui_aligned_text_gets_the_layers_width() {
        let mut s = Session::new();
        let f = id_of(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 100})).unwrap()["id"]).unwrap();
        let t = id_of(&s.execute("text.create", &json!({"text": "Hi", "x": 0, "y": 0, "parent": f})).unwrap()["id"]).unwrap();
        s.execute("node.set", &json!({"id": t, "props": {"textAlign": "center", "width": 120.0}})).unwrap();
        let doc = s.doc().unwrap();
        let ark = code_for(doc, f, CodeLang::ArkUi);
        assert!(ark.contains(".textAlign(TextAlign.Center)"), "{ark}");
        let width = doc.node(t).map(|n| n.width).unwrap_or_default();
        assert!(ark.contains(&format!(".width({})", num(width))), "the alignment has a box: {ark}");
    }

    #[test]
    fn arkui_percent_line_height_and_letter_spacing_are_absolute_numbers() {
        // ArkUI's `letterSpacing` falls back to the default for a percent string, and `lineHeight`
        // takes no percent at all: both are numbers of vp, a percent being of the font size.
        let mut s = Session::new();
        let f = id_of(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 100})).unwrap()["id"]).unwrap();
        let t = id_of(&s.execute("text.create", &json!({"text": "Hi", "x": 0, "y": 0, "parent": f})).unwrap()["id"]).unwrap();
        s.execute("node.set", &json!({"id": t, "props": {"fontSize": 20.0, "lineHeight": "150%", "letterSpacing": "10%"}})).unwrap();
        let doc = s.doc().unwrap();
        let ark = code_for(doc, f, CodeLang::ArkUi);
        assert!(ark.contains(".lineHeight(30)"), "{ark}");
        assert!(ark.contains(".letterSpacing(2)"), "{ark}");
        assert!(!ark.contains('%'), "no percent strings: {ark}");
    }

    #[test]
    fn arkui_places_a_child_that_ignores_its_auto_layout_parent() {
        let mut s = Session::new();
        let f = id_of(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 100})).unwrap()["id"]).unwrap();
        let c = id_of(&s.execute("shape.rectangle", &json!({"x": 7, "y": 9, "width": 20, "height": 20, "parent": f})).unwrap()["id"]).unwrap();
        s.execute("node.set", &json!({"id": c, "props": {"absolute": true}})).unwrap();
        s.execute("layout.add", &json!({"ids": [f]})).unwrap();
        let doc = s.doc().unwrap();
        let [_, _, _, _, x, y] = doc.node(c).unwrap().transform.as_coeffs();
        let ark = code_for(doc, f, CodeLang::ArkUi);
        assert!(ark.contains("Row(") || ark.contains("Column("), "the parent is still auto layout: {ark}");
        assert!(ark.contains(&format!(".position({{ x: {}, y: {} }})", num(x), num(y))), "the absolute child keeps its place: {ark}");
    }

    #[test]
    fn flutter_builds_a_widget_tree() {
        let mut s = Session::new();
        let f = card(&mut s);
        let doc = s.doc().unwrap();
        let dart = code_for(doc, f, CodeLang::Flutter);
        // The card hugs its content, so its width is the layout's, not the 200 it was created with.
        let w = doc.node(f).map(|n| n.width).unwrap_or_default();
        assert!(dart.starts_with(&format!("Container(\n  width: {},\n  height: 100,\n", num(w))), "{dart}");
        assert!(dart.contains("borderRadius: BorderRadius.circular(12)"), "{dart}");
        assert!(dart.contains("child: Row(\n"), "{dart}");
        assert!(dart.contains("      Text(\n        \"Say \\\"hi\\\" \\$5\\nnow\",\n"), "escaped: {dart}");
        assert!(dart.contains("fontWeight: FontWeight.w400,"), "{dart}");
        assert!(dart.contains("shape: BoxShape.circle"), "{dart}");
        assert!(dart.ends_with(')'), "a bare expression, like the other native targets: {dart}");
        // Through the command: `dart` names the same language, and rem is for the web only.
        let v = s.execute("inspect.code", &json!({"id": f, "language": "flutter"})).unwrap();
        assert_eq!(v["code"].as_str().unwrap(), dart);
        assert_eq!(v["language"], "flutter");
        assert_eq!(s.execute("inspect.code", &json!({"id": f, "language": "dart"})).unwrap()["language"], "flutter");
        let rem = s.execute("inspect.code", &json!({"id": f, "language": "flutter", "unit": "rem"})).unwrap();
        assert_eq!(rem["code"].as_str().unwrap(), dart, "rem doesn't apply to Dart");
    }

    #[test]
    fn flutter_positions_free_children_and_bounds_the_tree() {
        let mut s = Session::new();
        let root = id_of(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap()["id"]).unwrap();
        s.execute("shape.rectangle", &json!({"x": 7, "y": 9, "width": 5, "height": 5, "parent": root})).unwrap();
        let mut parent = root;
        for _ in 0..20 {
            parent = id_of(&s.execute("frame.create", &json!({"x": 1, "y": 1, "width": 40, "height": 40, "parent": parent})).unwrap()["id"]).unwrap();
        }
        let dart = code_for(s.doc().unwrap(), root, CodeLang::Flutter);
        assert!(dart.contains("Stack(\n    alignment: Alignment.topLeft,\n"), "{dart}");
        assert!(dart.contains("Positioned(\n        left: 7,\n        top: 9,\n"), "{dart}");
        // A wrapper's closing parenthesis sits on its own line, not glued to the widget's.
        assert!(dart.contains("        )\n      ),\n"), "{dart}");
        assert!(dart.contains("…"), "the depth is capped: {dart}");
        assert!(dart.contains("const SizedBox()"), "a capped layer is still a widget in the list: {dart}");
    }

    #[test]
    fn flutter_keeps_a_capped_layers_name_on_its_comment_line() {
        let mut s = Session::new();
        let root = id_of(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap()["id"]).unwrap();
        let mut parent = root;
        for _ in 0..20 {
            let p = json!({"x": 1, "y": 1, "width": 40, "height": 40, "parent": parent, "name": "deep\nvoid main() {}"});
            parent = id_of(&s.execute("frame.create", &p).unwrap()["id"]).unwrap();
        }
        let dart = code_for(s.doc().unwrap(), root, CodeLang::Flutter);
        assert!(dart.contains("…"), "the depth is capped: {dart}");
        assert!(!dart.lines().any(|l| l.trim_start().starts_with("void main()")), "a name never leaves its comment: {dart}");
    }

    #[test]
    fn flutter_expands_and_stretches_fill_children() {
        let mut s = Session::new();
        let row = id_of(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 40})).unwrap()["id"]).unwrap();
        let a = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 20, "parent": row})).unwrap()["id"]).unwrap();
        s.execute("shape.rectangle", &json!({"x": 60, "y": 0, "width": 50, "height": 20, "parent": row})).unwrap();
        s.execute("layout.add", &json!({"ids": [row], "direction": "horizontal"})).unwrap();
        s.execute("node.set", &json!({"id": a, "props": {"sizingH": "fill", "sizingV": "fill"}})).unwrap();
        let dart = code_for(s.doc().unwrap(), row, CodeLang::Flutter);
        assert!(dart.contains("child: Row(\n"), "{dart}");
        assert!(dart.contains("Expanded(\n"), "a child filling the main axis takes the room left: {dart}");
        assert!(dart.contains("crossAxisAlignment: CrossAxisAlignment.stretch,"), "a child filling the cross axis stretches the line: {dart}");
    }

    #[test]
    fn flutter_lifts_a_child_that_ignores_its_auto_layout_parent_out_of_the_line() {
        let mut s = Session::new();
        let row = id_of(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 200, "height": 40})).unwrap()["id"]).unwrap();
        let flow = id_of(&s.execute("shape.rectangle", &json!({"x": 4, "y": 4, "width": 50, "height": 20, "parent": row})).unwrap()["id"]).unwrap();
        let over = id_of(&s.execute("shape.ellipse", &json!({"x": 150, "y": 6, "width": 30, "height": 30, "parent": row})).unwrap()["id"]).unwrap();
        s.execute("layout.add", &json!({"ids": [row], "direction": "horizontal"})).unwrap();
        s.execute("node.set", &json!({"id": row, "props": {"padding": [4, 4, 4, 4]}})).unwrap();
        // It asks for both axes, which the line would read as "stretch me and take the room left".
        s.execute("node.set", &json!({"id": over, "props": {"absolute": true, "sizingH": "fill", "sizingV": "fill"}})).unwrap();
        let doc = s.doc().unwrap();
        let dart = code_for(doc, row, CodeLang::Flutter);
        let at = |needle: &str| dart.find(needle).unwrap_or_else(|| panic!("no {needle:?}: {dart}"));
        // Over the line, in a Stack at the parent's own box, at its own offsets.
        let [_, _, _, _, x, y] = doc.node(over).unwrap().transform.as_coeffs();
        assert!(
            dart.contains(&format!("Positioned(\n        left: {},\n        top: {},\n", num(x), num(y))),
            "the absolute child keeps its place: {dart}"
        );
        let stack = at("child: Stack(\n");
        let fill = at("Positioned.fill(\n");
        let padding = at("padding: EdgeInsets.all(4),");
        let line = at("child: Row(\n");
        assert!(stack < fill && fill < padding && padding < line, "the line sits in the Stack, padded: {dart}");
        assert!(!dart[..padding].contains("padding: "), "the parent's padding would offset the absolute child: {dart}");
        // Only the flow child is in the line, so neither `Expanded` nor a stretch comes from it.
        let over_at = at("\n      Positioned(\n");
        let line_part = &dart[line..over_at];
        assert!(line_part.contains("width: 50,"), "the flow child is in the line: {dart}");
        assert!(!line_part.contains("width: 30,"), "the absolute child left the line: {dart}");
        assert!(!dart.contains("Expanded(\n"), "it takes no room in the line either: {dart}");
        assert!(dart.contains("crossAxisAlignment: CrossAxisAlignment.start,"), "{dart}");
        assert!(doc.node(flow).is_some());
    }

    #[test]
    fn arkui_wraps_a_positioned_instance_swap_so_the_position_lands_on_a_container() {
        let mut s = Session::new();
        let icon = id_of(&s.execute("frame.create", &json!({"x": 0, "y": 400, "width": 24, "height": 24, "name": "Icon"})).unwrap()["id"]).unwrap();
        s.execute("shape.star", &json!({"x": 0, "y": 0, "width": 24, "height": 24, "parent": icon})).unwrap();
        let heart = id_of(&s.execute("component.create", &json!({"ids": [icon]})).unwrap()["id"]).unwrap();
        let f = id_of(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 120, "height": 40, "name": "Chip"})).unwrap()["id"]).unwrap();
        let nested = id_of(&s.execute("instance.create", &json!({"component": heart, "x": 8, "y": 8, "parent": f})).unwrap()["id"]).unwrap();
        let chip = id_of(&s.execute("component.create", &json!({"ids": [f]})).unwrap()["id"]).unwrap();
        s.execute("component.addProperty", &json!({"id": chip, "name": "Icon", "type": "instanceSwap", "default": heart, "layer": nested})).unwrap();
        let doc = s.doc().unwrap();
        let ark = code_for(doc, chip, CodeLang::ArkUi);
        assert!(ark.contains("Stack() {\n"), "the builder is wrapped: {ark}");
        assert!(ark.contains("this.icon()"), "{ark}");
        // The position follows the wrapper's closing brace, not the void builder call.
        let lines: Vec<&str> = ark.lines().collect();
        let at = lines.iter().position(|l| l.contains(".position({ x: 8, y: 8 })")).unwrap_or_else(|| panic!("no position: {ark}"));
        assert_eq!(lines[at - 1].trim(), "}", "the position is on a container: {ark}");
    }

    #[test]
    fn too_deep_placeholder_keeps_a_multiline_layer_name_in_the_comment() {
        let mut s = Session::new();
        let name = "deep\nevil()\r\u{2028}more()";
        let mut parent = id_of(&s.execute("frame.create", &json!({"x": 0, "y": 0, "width": 50, "height": 50, "name": name})).unwrap()["id"]).unwrap();
        let root = parent;
        for _ in 0..16 {
            parent =
                id_of(&s.execute("frame.create", &json!({"x": 1, "y": 1, "width": 40, "height": 40, "parent": parent, "name": name})).unwrap()["id"])
                    .unwrap();
        }
        let doc = s.doc().unwrap();
        for lang in [CodeLang::SwiftUi, CodeLang::Compose, CodeLang::ArkUi] {
            let code = code_for(doc, root, lang);
            assert!(code.contains("// deep evil()  more() …\n"), "{code}");
            for line in code.lines() {
                let t = line.trim_start();
                assert!(!t.starts_with("evil()") && !t.starts_with("more()"), "layer name leaked as code:\n{code}");
            }
        }
    }
}
