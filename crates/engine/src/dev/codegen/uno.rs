//! Uno Platform C# Markup: a layer and its subtree as a fluent C# expression tree of WinUI
//! elements (`Uno.Extensions.Markup`), inside a small `UserControl` class. Lengths are fixed and
//! children sit at their offsets in a `Canvas`, as in the Flutter target before its auto layout
//! code. Whatever has no counterpart is left as a `// not mapped: …` comment in the chain.

use std::collections::BTreeSet;

use figcraft_color::{BlendMode, Color, ImageScaleMode, PaintKind};
use figcraft_doc::{
    AutoLayout, CrossAlign, Document, EffectKind, LayoutMode, LetterSpacing, LineHeight, MainAlign, Node, NodeId, NodeKind, StrokeAlign, TextAlignH,
    TextAlignV, TextAutoResize, TextCase, TextData, TextDecoration, TextStyle,
};

use super::swift::one_line;
use super::{Ctx, MAX_CHILDREN, first_fill, num, stroke_color};

/// Longest string literal written (characters); the rest is cut.
const MAX_LITERAL: usize = 10_000;
/// Longest layer name written into a comment (characters).
const MAX_COMMENT: usize = 80;
/// Longest identifier or asset name written (characters).
const MAX_IDENT: usize = 40;
/// Most style runs of one text written as `Run`s.
const MAX_RUNS: usize = 200;
/// Where the app keeps the images and vectors the code refers to.
const ASSETS: &str = "ms-appx:///Assets";

/// C# indents by four spaces.
pub(crate) fn uno_indent(depth: usize) -> String {
    "    ".repeat(depth)
}

/// A C# string literal: `"` and `\` escaped, control and line separator characters as escapes,
/// never an interpolated string. Cut at [`MAX_LITERAL`] characters.
pub(crate) fn cs_literal(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars().take(MAX_LITERAL) {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // Other control characters and the line and paragraph separators end a C# line.
            c if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') => out.push_str(&format!("\\u{:04X}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// C# reserved words, which an identifier may not be (all lowercase, so only camelCase meets them).
const CS_KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "base",
    "bool",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "checked",
    "class",
    "const",
    "continue",
    "decimal",
    "default",
    "delegate",
    "do",
    "double",
    "else",
    "enum",
    "event",
    "explicit",
    "extern",
    "false",
    "finally",
    "fixed",
    "float",
    "for",
    "foreach",
    "goto",
    "if",
    "implicit",
    "in",
    "int",
    "interface",
    "internal",
    "is",
    "lock",
    "long",
    "namespace",
    "new",
    "null",
    "object",
    "operator",
    "out",
    "override",
    "params",
    "private",
    "protected",
    "public",
    "readonly",
    "ref",
    "return",
    "sbyte",
    "sealed",
    "short",
    "sizeof",
    "stackalloc",
    "static",
    "string",
    "struct",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "uint",
    "ulong",
    "unchecked",
    "unsafe",
    "ushort",
    "using",
    "virtual",
    "void",
    "volatile",
    "while",
];

/// The ASCII alphanumeric words of `name` after transliteration (so non-Latin names stay
/// readable), kept whole up to [`MAX_IDENT`] characters; the first word is kept even when it alone
/// is too long (the caller cuts it).
fn ascii_words(name: &str) -> Vec<String> {
    let mut kept: Vec<String> = Vec::new();
    let mut len = 0;
    for w in deunicode::deunicode(name).split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()) {
        if !kept.is_empty() && len + w.len() > MAX_IDENT {
            break;
        }
        len += w.len();
        kept.push(w.to_string());
    }
    kept
}

/// A C# identifier from a layer name: words joined as PascalCase (`pascal`) or camelCase, cut at
/// [`MAX_IDENT`] characters on a word boundary. A name left with nothing takes `kind`; a leading
/// digit gets a `_` prefix and a reserved word a `_` suffix (never `@`).
pub(crate) fn cs_ident(name: &str, pascal: bool, kind: &str) -> String {
    let mut out = String::new();
    for (i, w) in ascii_words(name).iter().enumerate() {
        let mut cs = w.chars();
        let Some(first) = cs.next() else { continue };
        // The first letter of each word is upper-case (the first word of camelCase lower-case);
        // the rest keeps its case, so `mainCard` stays recognisable.
        if i == 0 && !pascal {
            out.extend(first.to_lowercase());
        } else {
            out.extend(first.to_uppercase());
        }
        out.push_str(cs.as_str());
    }
    let mut out: String = out.chars().take(MAX_IDENT).collect();
    if out.is_empty() {
        let k: String = kind.chars().filter(char::is_ascii_alphanumeric).collect();
        let mut cs = k.chars();
        out = match cs.next() {
            Some(f) if pascal => f.to_uppercase().chain(cs).collect(),
            Some(f) => f.to_lowercase().chain(cs).collect(),
            None => "Layer".into(),
        };
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    if CS_KEYWORDS.contains(&out.as_str()) {
        out.push('_');
    }
    out
}

/// `base` made unique among `used` (C# names are case-sensitive): `Card`, `Card2`, `Card3`, ….
#[cfg_attr(not(test), allow(dead_code, reason = "names of component members (U3)"))]
pub(crate) fn cs_unique(base: &str, used: &mut BTreeSet<String>) -> String {
    let name = (1usize..)
        .map(|k| if k == 1 { base.to_string() } else { format!("{base}{k}") })
        .find(|n| !used.contains(n))
        .unwrap_or_else(|| base.to_string());
    used.insert(name.clone());
    name
}

/// The file name (without extension) of the image or vector asset of a layer: lowercase
/// `[a-z0-9_]`, starting with a letter, at most [`MAX_IDENT`] characters, unique among `used`
/// (`logo`, `logo_2`, `logo_3`, …). A name left with nothing takes `kind`.
pub(crate) fn asset_name(name: &str, kind: &str, used: &mut BTreeSet<String>) -> String {
    let mut base = ascii_words(name).iter().map(|w| w.to_ascii_lowercase()).collect::<Vec<_>>().join("_");
    base = base.chars().take(MAX_IDENT).collect();
    let base = base.trim_end_matches('_').to_string();
    let base = if base.is_empty() {
        let k: String = kind.to_ascii_lowercase().chars().filter(char::is_ascii_alphanumeric).collect();
        if k.is_empty() { "image".into() } else { k }
    } else {
        base
    };
    let base = if base.starts_with(|c: char| c.is_ascii_digit()) { format!("img_{base}") } else { base };
    let name =
        (1usize..).map(|k| if k == 1 { base.clone() } else { format!("{base}_{k}") }).find(|n| !used.contains(n)).unwrap_or_else(|| base.clone());
    used.insert(name.clone());
    name
}

/// `Color.FromArgb(0xAA, 0xRR, 0xGG, 0xBB)` (needs `using Windows.UI;`).
pub(crate) fn cs_color(c: Color) -> String {
    let [r, g, b, a] = c.to_rgba8();
    format!("Color.FromArgb(0x{a:02X}, 0x{r:02X}, 0x{g:02X}, 0x{b:02X})")
}

/// A layer name for a comment line: one line, cut.
fn comment_name(name: &str) -> String {
    let one = one_line(name);
    let cut: String = one.chars().take(MAX_COMMENT).collect();
    if cut.chars().count() < one.chars().count() { format!("{cut}…") } else { cut }
}

/// One element of the tree before it is written: comment lines above it, its constructor, the
/// chain of property calls (a call starting with `//` is a comment line) and its children.
struct Element {
    lead: Vec<String>,
    ctor: String,
    calls: Vec<String>,
    /// `Child` (one) or `Children` (several) and the child expressions, written two levels in.
    kids: Option<(&'static str, Vec<String>)>,
}

impl Element {
    fn new(ctor: &str) -> Self {
        Element { lead: Vec::new(), ctor: ctor.to_string(), calls: Vec::new(), kids: None }
    }

    fn call(&mut self, c: impl Into<String>) {
        self.calls.push(c.into());
    }

    fn note(&mut self, what: &str) {
        self.calls.push(format!("// not mapped: {what}"));
    }

    /// The element written at `depth`: a complete expression, no trailing newline.
    fn write(&self, depth: usize) -> String {
        let (pad, inner) = (uno_indent(depth), uno_indent(depth + 1));
        let mut out = String::new();
        for l in &self.lead {
            out.push_str(&format!("{pad}{l}\n"));
        }
        out.push_str(&format!("{pad}{}", self.ctor));
        for c in &self.calls {
            // A call of several lines is indented relative to its first.
            out.push_str(&format!("\n{inner}{}", c.replace('\n', &format!("\n{inner}"))));
        }
        if let Some((method, kids)) = &self.kids {
            out.push_str(&format!("\n{inner}.{method}(\n{})", kids.join(",\n")));
        }
        out
    }
}

/// Where a child sits in its parent: at an offset in a `Canvas`, or wherever the parent's flow
/// (an `AutoLayout`) puts it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Place {
    Flow,
    At(f64, f64),
}

/// What a tree walk keeps between layers.
struct Gen<'a> {
    ctx: &'a Ctx<'a>,
    /// Asset names given out, so no two layers share a file.
    assets: BTreeSet<String>,
    /// Frames written as a `Canvas` for lack of auto layout (one warning per view says how many).
    free_frames: usize,
}

/// Uno C# Markup for layer `id` and its subtree: a `UserControl` class whose content is the tree.
pub(crate) fn uno(doc: &Document, id: NodeId) -> String {
    let Some(root) = doc.node(id) else { return String::new() };
    // The tree sits three levels in: namespace member, constructor body, `Content(` argument.
    let ctx = Ctx { base: 3, ..Ctx::new(doc, id) };
    let mut g = Gen { ctx: &ctx, assets: BTreeSet::new(), free_frames: 0 };
    let mut body = g.layer(root, 3, Place::Flow).unwrap_or_else(|| format!("{}new Border()", uno_indent(3)));
    if g.free_frames > 0 {
        // One warning per view, above everything else written for the root layer.
        let (count, verb) = if g.free_frames == 1 {
            ("1 frame has".to_string(), "its children are")
        } else {
            (format!("{} frames have", g.free_frames), "children are")
        };
        body = format!("{}// warning: {count} no auto layout: {verb} positioned absolutely and do not adapt to size.\n{body}", uno_indent(3));
    }
    let mut out = String::new();
    let usings: Vec<&str> = [
        ("using Microsoft.UI.Text;", "FontWeights."),
        ("using Windows.Foundation;", "new Rect("),
        ("using Windows.UI;", "Color.FromArgb("),
        ("using Windows.UI.Text;", "FontStyle."),
        ("using Uno.Toolkit.UI;", "new AutoLayout("),
    ]
    .into_iter()
    .filter(|(_, marker)| body.contains(marker))
    .map(|(u, _)| u)
    .collect();
    for u in &usings {
        out.push_str(u);
        out.push('\n');
    }
    if !usings.is_empty() {
        out.push('\n');
    }
    // A view named like a WinUI type (`Button`, `Border`) would hide it in the tree below.
    let mut name = cs_ident(&root.name, true, "Layer");
    if !name.ends_with("View") {
        name.push_str("View");
    }
    out.push_str(&format!("// {}\nnamespace MyApp;\n\n", comment_name(&root.name)));
    out.push_str(&format!("internal sealed partial class {name} : UserControl\n{{\n"));
    out.push_str(&format!("    public {name}()\n    {{\n        this.Content(\n{body});\n    }}\n}}\n"));
    out
}

impl Gen<'_> {
    /// A layer and its subtree at `depth`, placed in its parent as `at` says. `None` for a layer
    /// that draws nothing.
    fn layer(&mut self, n: &Node, depth: usize, at: Place) -> Option<String> {
        if matches!(n.kind, NodeKind::Slice) {
            return None;
        }
        if self.ctx.too_deep(depth) {
            return Some(format!("{pad}// {} …\n{pad}new Border()", comment_name(&n.name), pad = uno_indent(depth)));
        }
        let e = match &n.kind {
            NodeKind::Text(t) => self.text(n, t, depth, at),
            NodeKind::Ellipse { start, sweep, inner } => self.ellipse(n, (*start, *sweep, *inner), at),
            NodeKind::Rectangle | NodeKind::Group | NodeKind::Section { .. } => self.boxed(n, depth, at),
            k if k.is_frame_like() => self.boxed(n, depth, at),
            _ => self.asset(n, at),
        };
        Some(e.write(depth))
    }

    /// A frame-like layer, group or rectangle. Horizontal and vertical auto layout is an
    /// `AutoLayout`; anything else is a `Border` holding a `Canvas` with the children at their
    /// offsets.
    fn boxed(&mut self, n: &Node, depth: usize, at: Place) -> Element {
        match n.layout.as_ref() {
            Some(l) if matches!(l.mode, LayoutMode::Horizontal | LayoutMode::Vertical) && !matches!(n.kind, NodeKind::Rectangle) => {
                self.auto_layout(n, l, depth, at)
            }
            _ => self.canvas_box(n, depth, at),
        }
    }

    /// A frame with horizontal or vertical auto layout: an `AutoLayout` that takes the box
    /// (fill, border, corners) itself. A `Border` pushes its content in by its thickness, so
    /// the padding is written less the stroke weight, as FigCraft's layout ignores strokes (unless
    /// they are included). Where that would be negative the border sits behind the layout in a
    /// `Grid`, as for a frame without auto layout.
    fn auto_layout(&mut self, n: &Node, l: &AutoLayout, depth: usize, at: Place) -> Element {
        let [pt, pr, pb, pl] = l.padding;
        let weights = match (stroke_color(n), n.stroke.sides) {
            (None, _) => [0.0; 4],
            (Some(_), Some(s)) => s,
            (Some(_), None) => [n.stroke.weight; 4],
        };
        let weights = if l.strokes_included { [0.0; 4] } else { weights };
        let pad = [pt - weights[0], pr - weights[1], pb - weights[2], pl - weights[3]];
        let behind = pad.iter().any(|p| *p < 0.0);
        let mut line = Element::new("new AutoLayout()");
        let mut exprs: Vec<String> = Vec::new();
        for k in self.ctx.kids(n).iter().take(MAX_CHILDREN) {
            let Some(c) = self.ctx.doc.node(*k) else { continue };
            exprs.extend(self.layer(c, depth + if behind { 4 } else { 2 }, Place::Flow));
        }
        if !exprs.is_empty() {
            line.kids = Some(("Children", exprs));
        }
        let mut outer;
        if behind {
            // The layout inside keeps the whole padding; the border is drawn behind it.
            let mut border = Element::new("new Border()");
            self.paint_box(&mut border, n, false);
            layout_calls(&mut line, l, [pt, pr, pb, pl]);
            outer = Element::new("new Grid()");
            place(&mut outer, at);
            size(&mut outer, n);
            outer.kids = Some(("Children", vec![border.write(depth + 2), line.write(depth + 2)]));
            common(&mut outer, n);
        } else {
            outer = line;
            place(&mut outer, at);
            size(&mut outer, n);
            self.paint_box(&mut outer, n, false);
            layout_calls(&mut outer, l, pad);
            common(&mut outer, n);
        }
        if n.clip && n.corners.radii.iter().all(|r| *r <= 0.0) && !n.children.is_empty() {
            outer.call(format!(".Clip(new RectangleGeometry().Rect(new Rect(0, 0, {}, {})))", num(n.width), num(n.height)));
        }
        outer
    }

    /// A frame-like layer, group or rectangle without a layout of its own to map: a `Border`
    /// with its fill, stroke and corners holding a `Canvas` with the children at their offsets.
    /// With a stroke and children the border sits behind the canvas in a `Grid`, since a `Border`
    /// pushes its content in by its thickness.
    fn canvas_box(&mut self, n: &Node, depth: usize, at: Place) -> Element {
        let mut border = Element::new("new Border()");
        let sized = !matches!(n.kind, NodeKind::Group | NodeKind::Section { .. });
        self.paint_box(&mut border, n, sized);
        let mut exprs: Vec<String> = Vec::new();
        for k in self.ctx.kids(n).iter().take(MAX_CHILDREN) {
            let Some(c) = self.ctx.doc.node(*k) else { continue };
            let [_, _, _, _, x, y] = c.transform.as_coeffs();
            exprs.extend(self.layer(c, depth + 4, Place::At(x, y)));
        }
        let mut canvas = Element::new("new Canvas()");
        let has_kids = !exprs.is_empty();
        if has_kids {
            canvas.kids = Some(("Children", exprs));
            if n.layout.is_none() {
                self.free_frames += 1;
            }
        }
        let mut outer;
        if has_kids && stroke_color(n).is_some() {
            outer = Element::new("new Grid()");
            place(&mut outer, at);
            size(&mut outer, n);
            border.calls.retain(|c| !c.starts_with(".Width(") && !c.starts_with(".Height("));
            outer.kids = Some(("Children", vec![border.write(depth + 2), canvas.write(depth + 2)]));
            common(&mut outer, n);
        } else {
            outer = border;
            place(&mut outer, at);
            if has_kids {
                outer.kids = Some(("Child", vec![canvas.write(depth + 2)]));
            }
            common(&mut outer, n);
        }
        if n.layout.as_ref().is_some_and(|l| l.mode == LayoutMode::Grid) {
            outer.note("grid auto layout (children are positioned absolutely)");
        }
        if n.clip && has_kids && n.corners.radii.iter().all(|r| *r <= 0.0) {
            outer.call(format!(".Clip(new RectangleGeometry().Rect(new Rect(0, 0, {}, {})))", num(n.width), num(n.height)));
        }
        outer
    }

    /// The fill, stroke and corners of a `Border`.
    fn paint_box(&mut self, e: &mut Element, n: &Node, sized: bool) {
        if sized {
            size(e, n);
        }
        let visible: Vec<_> = n.fills.iter().filter(|p| p.visible).collect();
        match visible.first().map(|p| &p.kind) {
            Some(PaintKind::Image(img)) => {
                let name = asset_name(&n.name, "image", &mut self.assets);
                let stretch = match img.scale_mode {
                    ImageScaleMode::Fit => "Uniform",
                    ImageScaleMode::Fill | ImageScaleMode::Crop | ImageScaleMode::Tile => "UniformToFill",
                };
                e.call(format!(".Background(new ImageBrush().ImageSource(\"{ASSETS}/{name}.png\").Stretch(Stretch.{stretch}))"));
                match img.scale_mode {
                    ImageScaleMode::Crop => e.note("image crop (drawn as fill)"),
                    ImageScaleMode::Tile => e.note("tiled image (drawn as fill)"),
                    ImageScaleMode::Fill | ImageScaleMode::Fit => {}
                }
            }
            Some(PaintKind::Gradient(_)) => {
                if let Some(c) = first_fill(n) {
                    e.call(format!(".Background({})", cs_color(c)));
                }
                e.note("gradient fill (its first stop colour is used)");
            }
            Some(PaintKind::Pattern(_)) => e.note("pattern fill"),
            Some(PaintKind::Solid { .. }) => {
                if let Some(c) = first_fill(n) {
                    e.call(format!(".Background({})", cs_color(c)));
                }
            }
            None => {}
        }
        if visible.len() > 1 {
            e.note(&format!("{} more fills (only the first is drawn)", visible.len() - 1));
        }
        if let Some(c) = stroke_color(n) {
            e.call(format!(".BorderBrush({})", cs_color(c)));
            match n.stroke.sides {
                // Per-side weights are top, right, bottom, left; `Thickness` takes left first.
                Some([t, r, b, l]) => e.call(format!(".BorderThickness(new Thickness({}, {}, {}, {}))", num(l), num(t), num(r), num(b))),
                None => e.call(format!(".BorderThickness({})", num(n.stroke.weight))),
            }
            if n.stroke.align != StrokeAlign::Inside {
                e.note("stroke outside or centred (a border is drawn inside)");
            }
        }
        let [tl, tr, br, bl] = n.corners.radii;
        if n.corners.uniform().is_some() {
            if tl > 0.0 {
                e.call(format!(".CornerRadius({})", num(tl)));
            }
        } else {
            e.call(format!(".CornerRadius(new CornerRadius({}, {}, {}, {}))", num(tl), num(tr), num(br), num(bl)));
        }
        if n.corners.smoothing > 0.0 && n.corners.radii.iter().any(|r| *r > 0.0) {
            e.note("corner smoothing");
        }
    }

    /// An ellipse: `Ellipse` with `Fill` and a centred `Stroke`.
    fn ellipse(&mut self, n: &Node, (start, sweep, inner): (f64, f64, f64), at: Place) -> Element {
        let mut e = Element::new("new Ellipse()");
        place(&mut e, at);
        size(&mut e, n);
        if let Some(c) = first_fill(n) {
            e.call(format!(".Fill({})", cs_color(c)));
        }
        if let Some(c) = stroke_color(n) {
            e.call(format!(".Stroke({})", cs_color(c)));
            e.call(format!(".StrokeThickness({})", num(n.stroke.weight)));
            if n.stroke.align != StrokeAlign::Center {
                e.note("stroke inside or outside (an ellipse stroke is centred)");
            }
        }
        if start != 0.0 || (sweep - std::f64::consts::TAU).abs() > 1e-6 || inner > 0.0 {
            e.note("arc or ring (drawn as a full ellipse)");
        }
        common(&mut e, n);
        e
    }

    /// A vector, shape or other layer without a counterpart: the image the app is to hold.
    fn asset(&mut self, n: &Node, at: Place) -> Element {
        let name = asset_name(&n.name, n.kind.label(), &mut self.assets);
        let mut e = Element::new("new Image()");
        e.lead.push(format!("// {} \"{}\": export it as an SVG to Assets/{name}.svg", n.kind.label(), comment_name(&n.name)));
        place(&mut e, at);
        e.call(format!(".Source(\"{ASSETS}/{name}.svg\")"));
        size(&mut e, n);
        e.call(".Stretch(Stretch.Uniform)");
        common(&mut e, n);
        e
    }

    /// Text: a `TextBlock`, in a `Grid` of the box's height when it is aligned to the middle or
    /// bottom of a fixed box.
    fn text(&mut self, n: &Node, t: &TextData, depth: usize, at: Place) -> Element {
        let fixed = matches!(t.auto_resize, TextAutoResize::None | TextAutoResize::Truncate);
        let framed = fixed && t.align_v != TextAlignV::Top;
        let mut tb = text_block(n, t, !framed);
        if !framed {
            place(&mut tb, at);
            common(&mut tb, n);
            return tb;
        }
        tb.call(format!(".VerticalAlignment(VerticalAlignment.{})", if t.align_v == TextAlignV::Middle { "Center" } else { "Bottom" }));
        let mut g = Element::new("new Grid()");
        place(&mut g, at);
        size(&mut g, n);
        g.kids = Some(("Children", vec![tb.write(depth + 2)]));
        common(&mut g, n);
        g
    }
}

/// The `Canvas` offsets of a child at `at`; a child in a flow carries nothing.
fn place(e: &mut Element, at: Place) {
    if let Place::At(x, y) = at {
        // The offsets come first in the chain: they read as the element's address.
        e.calls.insert(0, format!(".Canvas(left: {}, top: {})", num(x), num(y)));
    }
}

/// The calls an `AutoLayout` takes for its orientation, gap, padding (`pad` is top, right,
/// bottom, left as written) and alignment.
fn layout_calls(e: &mut Element, l: &AutoLayout, pad: [f64; 4]) {
    e.call(format!(".Orientation(Orientation.{})", if l.mode == LayoutMode::Vertical { "Vertical" } else { "Horizontal" }));
    // The space distributions make the gaps themselves.
    if l.gap != 0.0 && !l.main_align.is_auto_gap() {
        e.call(format!(".Spacing({})", num(l.gap)));
    }
    let [t, r, b, lf] = pad;
    if [t, r, b, lf].iter().any(|p| *p != 0.0) {
        if t == r && r == b && b == lf {
            e.call(format!(".Padding({})", num(t)));
        } else {
            e.call(format!(".Padding({}, {}, {}, {})", num(lf), num(t), num(r), num(b)));
        }
    }
    match l.main_align {
        MainAlign::Min => {}
        MainAlign::Center => e.call(".PrimaryAxisAlignment(AutoLayoutAlignment.Center)"),
        MainAlign::Max => e.call(".PrimaryAxisAlignment(AutoLayoutAlignment.End)"),
        MainAlign::SpaceBetween => e.call(".Justify(AutoLayoutJustify.SpaceBetween)"),
        MainAlign::SpaceAround | MainAlign::SpaceEvenly => {
            e.call(".Justify(AutoLayoutJustify.SpaceBetween)");
            e.note("space around/evenly exported as space between");
        }
    }
    match l.cross_align {
        CrossAlign::Min => {}
        CrossAlign::Center => e.call(".CounterAxisAlignment(AutoLayoutAlignment.Center)"),
        CrossAlign::Max => e.call(".CounterAxisAlignment(AutoLayoutAlignment.End)"),
        CrossAlign::Baseline => e.note("baseline alignment (children are aligned to the start)"),
    }
}

/// The layer's fixed width and height.
fn size(e: &mut Element, n: &Node) {
    e.call(format!(".Width({})", num(n.width)));
    e.call(format!(".Height({})", num(n.height)));
}

/// What every layer shares: opacity, rotation, and the notes for what has no counterpart.
fn common(e: &mut Element, n: &Node) {
    if n.opacity < 1.0 {
        e.call(format!(".Opacity({})", num(f64::from(n.opacity))));
    }
    // Rotation turns counter-clockwise about the top-left corner (the offsets given); a WinUI
    // `RotateTransform` turns clockwise about the element's own origin.
    let rot = figcraft_geom::rotation_degrees(n.transform);
    if rot.abs() > 0.01 && rot.is_finite() {
        e.call(format!(".RenderTransform(new RotateTransform().Angle({}))", num(-rot)));
    }
    if !matches!(n.blend, BlendMode::Normal | BlendMode::PassThrough) {
        e.note(&format!("blend mode {}", n.blend.css()));
    }
    for fx in n.effects.iter().filter(|f| f.visible) {
        e.note(match fx.kind {
            EffectKind::DropShadow(_) => "drop shadow (needs Uno.Toolkit ShadowContainer)",
            EffectKind::InnerShadow(_) => "inner shadow (needs Uno.Toolkit ShadowContainer)",
            EffectKind::LayerBlur(_) => "layer blur",
            EffectKind::BackgroundBlur(_) => "background blur",
            EffectKind::Noise(_) => "noise",
            EffectKind::Texture(_) => "texture",
            EffectKind::Glass(_) => "glass",
        });
    }
    if n.is_mask {
        e.note("mask");
    }
}

/// `FontWeights.SemiBold` for a weight in hundreds (the nearest).
fn cs_weight(w: u16) -> &'static str {
    match w {
        0..=150 => "Thin",
        151..=250 => "ExtraLight",
        251..=350 => "Light",
        351..=450 => "Normal",
        451..=550 => "Medium",
        551..=650 => "SemiBold",
        651..=750 => "Bold",
        751..=850 => "ExtraBold",
        851..=950 => "Black",
        _ => "ExtraBlack",
    }
}

/// The colour of the first visible solid or gradient paint (a gradient's first stop).
fn paints_color(paints: &[figcraft_color::Paint]) -> Option<Color> {
    paints.iter().filter(|p| p.visible).find_map(|p| match &p.kind {
        PaintKind::Solid { color } => Some(color.with_alpha(color.a * p.opacity)),
        PaintKind::Gradient(g) => g.sorted_stops().first().map(|s| s.color),
        PaintKind::Image(_) | PaintKind::Pattern(_) => None,
    })
}

/// `characters` as a string expression in `case`: the case is applied when the code runs, so the
/// string keeps the original text (and the app can translate it).
fn text_expr(characters: &str, case: TextCase) -> String {
    let lit = cs_literal(characters);
    match case {
        TextCase::Original | TextCase::SmallCaps => lit,
        TextCase::Upper => format!("{lit}.ToUpperInvariant()"),
        TextCase::Lower => format!("{lit}.ToLowerInvariant()"),
        TextCase::Title => format!("System.Globalization.CultureInfo.CurrentCulture.TextInfo.ToTitleCase({lit})"),
    }
}

/// The calls a `TextBlock` or `Run` takes for the parts of `s` that differ from `base` (all of
/// them without a base). `fill` is the colour the text is drawn in.
fn style_calls(e: &mut Element, s: &TextStyle, fill: Option<Color>, base: Option<(&TextStyle, Option<Color>)>) {
    if !s.font_family.is_empty() && base.is_none_or(|(b, _)| b.font_family != s.font_family) {
        e.call(format!(".FontFamily({})", cs_literal(&s.font_family)));
    }
    if base.is_none_or(|(b, _)| b.font_size != s.font_size) {
        e.call(format!(".FontSize({})", num(s.font_size)));
    }
    if (base.is_none() && s.weight() != 400) || base.is_some_and(|(b, _)| b.weight() != s.weight()) {
        e.call(format!(".FontWeight(FontWeights.{})", cs_weight(s.weight())));
    }
    if (base.is_none() && s.italic()) || base.is_some_and(|(b, _)| b.italic() != s.italic()) {
        e.call(format!(".FontStyle(FontStyle.{})", if s.italic() { "Italic" } else { "Normal" }));
    }
    if let Some(c) = fill
        && base.is_none_or(|(_, f)| f != fill)
    {
        e.call(format!(".Foreground({})", cs_color(c)));
    }
    if base.is_none() {
        match s.line_height {
            LineHeight::Pixels(v) if v > 0.0 && v.is_finite() => e.call(format!(".LineHeight({})", num(v))),
            LineHeight::Percent(p) if p > 0.0 && p.is_finite() && s.font_size > 0.0 => {
                e.call(format!(".LineHeight({})", num(p / 100.0 * s.font_size)))
            }
            _ => {}
        }
    }
    // `CharacterSpacing` is in thousandths of an em.
    let spacing = |x: &TextStyle| match x.letter_spacing {
        LetterSpacing::Pixels(v) if v.is_finite() && x.font_size > 0.0 => (v / x.font_size * 1000.0).round(),
        LetterSpacing::Percent(p) if p.is_finite() => (p * 10.0).round(),
        _ => 0.0,
    };
    if base.is_none_or(|(b, _)| spacing(b) != spacing(s)) && (base.is_some() || spacing(s) != 0.0) {
        e.call(format!(".CharacterSpacing({})", num(spacing(s))));
    }
    if base.is_none_or(|(b, _)| b.decoration != s.decoration) {
        match s.decoration {
            TextDecoration::None if base.is_some() => e.call(".TextDecorations(Windows.UI.Text.TextDecorations.None)"),
            TextDecoration::None => {}
            TextDecoration::Underline => e.call(".TextDecorations(Windows.UI.Text.TextDecorations.Underline)"),
            TextDecoration::Strikethrough => e.call(".TextDecorations(Windows.UI.Text.TextDecorations.Strikethrough)"),
        }
    }
}

/// A `TextBlock` for a text layer. `with_height` sets the box's height when the text has a fixed
/// box (a text in a `Grid` of that height leaves it to the grid).
fn text_block(n: &Node, t: &TextData, with_height: bool) -> Element {
    let mut e = Element::new("new TextBlock()");
    let s = &t.style;
    let fill = first_fill(n);
    // Text in one style is a `Text`; style runs make `Inlines` of `Run`s.
    let runs: Vec<_> = t.runs.iter().filter(|r| r.start < r.end).collect();
    let chars: Vec<char> = t.characters.chars().take(MAX_LITERAL * 10).collect();
    if runs.is_empty() || runs.len() > MAX_RUNS {
        e.call(format!(".Text({})", text_expr(&t.characters, s.case)));
        if !runs.is_empty() {
            e.note("style runs (too many)");
        }
    } else {
        let mut pieces: Vec<String> = Vec::new();
        let mut at = 0;
        let mut run = |from: usize, to: usize, style: &TextStyle, fills: Option<Color>, plain: bool| {
            let part: String = chars.get(from..to.min(chars.len())).unwrap_or_default().iter().collect();
            if part.is_empty() {
                return;
            }
            let mut r = Element::new("new Run()");
            r.call(format!(".Text({})", text_expr(&part, style.case)));
            if !plain {
                style_calls(&mut r, style, fills, Some((s, fill)));
            }
            pieces.push(r.write(1));
        };
        for r in &runs {
            let (start, end) = (r.start.max(at), r.end.min(chars.len()));
            if start >= end {
                continue;
            }
            run(at, start, s, fill, true);
            let run_fill = r.style.fills.as_deref().and_then(paints_color).or(fill);
            run(start, end, &r.style, run_fill, false);
            at = end;
        }
        run(at, chars.len(), s, fill, true);
        e.call(format!(".Inlines(\n{})", pieces.join(",\n")));
    }
    style_calls(&mut e, s, fill, None);
    match t.align_h {
        TextAlignH::Left => {}
        TextAlignH::Center => e.call(".TextAlignment(TextAlignment.Center)"),
        TextAlignH::Right => e.call(".TextAlignment(TextAlignment.Right)"),
        TextAlignH::Justified => e.call(".TextAlignment(TextAlignment.Justify)"),
    }
    match t.auto_resize {
        TextAutoResize::WidthAndHeight => e.call(".TextWrapping(TextWrapping.NoWrap)"),
        TextAutoResize::Height => {
            e.call(format!(".Width({})", num(n.width)));
            e.call(".TextWrapping(TextWrapping.Wrap)");
        }
        TextAutoResize::None | TextAutoResize::Truncate => {
            e.call(format!(".Width({})", num(n.width)));
            if with_height {
                e.call(format!(".Height({})", num(n.height)));
            }
            e.call(".TextWrapping(TextWrapping.Wrap)");
        }
    }
    if t.auto_resize == TextAutoResize::Truncate || (t.max_lines > 0 && t.auto_resize != TextAutoResize::WidthAndHeight) {
        e.call(".TextTrimming(TextTrimming.CharacterEllipsis)");
        if t.max_lines > 0 {
            e.call(format!(".MaxLines({})", t.max_lines));
        }
    }
    if s.case == TextCase::SmallCaps {
        e.note("small caps");
    }
    e
}
