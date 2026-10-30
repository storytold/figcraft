use super::{
    ContentAlign, CrossAlign, Document, EffectKind, LayoutMode, LetterSpacing, LineHeight, MainAlign, Node, NodeId, NodeKind, PADDING, PaintKind,
    TextAlignH, TextCase, TextDecoration, Variable, css_borders, num,
};

// ------------------------------------------------------------------ CSS

/// The CSS custom properties generated code uses: name → fallback value, in first-use order.
#[derive(Default)]
pub(crate) struct Tokens {
    vars: Vec<(String, String)>,
    /// Name variables as Tailwind theme variables (`--color-accent`), not by their CSS name.
    pub tailwind: bool,
    /// The file's theme names (computed on first use).
    theme: Option<crate::tailwind::Theme>,
}

impl Tokens {
    /// `var(--name, value)`, remembering the variable.
    fn var(&mut self, doc: &Document, variable: &Variable, value: String) -> String {
        let name =
            if self.tailwind { self.theme.get_or_insert_with(|| crate::tailwind::Theme::new(doc)).name(variable) } else { css_var_of(variable) };
        let out = format!("var({name}, {value})");
        if !self.vars.iter().any(|(n, _)| *n == name) {
            self.vars.push((name, value));
        }
        out
    }

    /// A `:root` rule declaring the variables (empty without any).
    pub fn root_rule(&self) -> String {
        if self.vars.is_empty() {
            return String::new();
        }
        let decls: String = self.vars.iter().map(|(n, v)| format!("  {n}: {v};\n")).collect();
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
pub(super) fn tok(doc: &Document, n: &Node, fields: &[&str], value: String, t: &mut Tokens) -> String {
    match fields.iter().find_map(|f| crate::devinspect::bound_variable(doc, n, f)) {
        Some(v) => t.var(doc, v, value),
        None => value,
    }
}

pub(super) fn css(doc: &Document, id: NodeId) -> String {
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
        layout_decls(doc, n, l, t, &mut put);
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
    for (key, value) in css_borders(doc, n, t) {
        put(&key, value);
    }
    effect_decls(n, &mut put);
    if let NodeKind::Text(tx) = &n.kind {
        text_decls(doc, n, tx, t, &mut put);
    }
    out
}

/// The flex declarations of auto layout `l` on `n`.
fn layout_decls(doc: &Document, n: &Node, l: &figcraft_doc::AutoLayout, t: &mut Tokens, put: &mut impl FnMut(&str, String)) {
    let px = |v: f64| format!("{}px", num(v));
    put("display", "flex".into());
    put("flex-direction", if l.mode == LayoutMode::Vertical { "column" } else { "row" }.into());
    if l.wrap {
        put("flex-wrap", "wrap".into());
    }
    put("gap", tok(doc, n, &["gap"], px(l.gap), t));
    let [pt, pr, pb, pl] = l.padding;
    let sides = [
        tok(doc, n, &PADDING[0], px(pt), t),
        tok(doc, n, &PADDING[1], px(pr), t),
        tok(doc, n, &PADDING[2], px(pb), t),
        tok(doc, n, &PADDING[3], px(pl), t),
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

/// Shadows and blurs of `n`.
fn effect_decls(n: &Node, put: &mut impl FnMut(&str, String)) {
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
}

/// The type declarations of text layer `n`.
fn text_decls(doc: &Document, n: &Node, tx: &figcraft_doc::TextData, t: &mut Tokens, put: &mut impl FnMut(&str, String)) {
    let px = |v: f64| format!("{}px", num(v));
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
    decoration_decls(s, put);
    if tx.leading_trim == figcraft_doc::LeadingTrim::CapHeight {
        put("text-box", "trim-both cap alphabetic".into());
    }
}

fn decoration_decls(s: &figcraft_doc::TextStyle, put: &mut impl FnMut(&str, String)) {
    let px = |v: f64| format!("{}px", num(v));
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
