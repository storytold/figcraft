use super::swift::one_line;
use super::{
    CodeLang, Color, CrossAlign, Ctx, LayoutMode, LetterSpacing, LineHeight, MainAlign, Node, NodeId, NodeKind, TextAlignH, TextCase, TextDecoration,
    drop_shadow, first_fill, indent, literal, num, radius, stroke_color,
};

// ------------------------------------------------------------------ ArkUI (ArkTS)

/// A colour as ArkUI writes it: `'#RRGGBB'`, or `'#AARRGGBB'` with transparency.
fn ark_color(c: Color) -> String {
    let [r, g, b, a] = c.to_rgba8();
    if a == 255 { format!("'#{r:02X}{g:02X}{b:02X}'") } else { format!("'#{a:02X}{r:02X}{g:02X}{b:02X}'") }
}

/// A layer's corner radii as `.borderRadius(...)` (uniform or per corner).
fn ark_radius(n: &Node, mods: &mut Vec<String>) {
    if let Some(r) = n.corners.uniform() {
        if r > 0.0 {
            mods.push(format!(".borderRadius({})", num(r)));
        }
    } else if n.corners.radii.iter().any(|r| *r > 0.0) {
        let [tl, tr, br, bl] = n.corners.radii;
        mods.push(format!(".borderRadius({{ topLeft: {}, topRight: {}, bottomRight: {}, bottomLeft: {} }})", num(tl), num(tr), num(br), num(bl)));
    }
}

/// ArkUI for a layer and its subtree at `depth`; `extra` attributes go last (a free child's
/// position). A layer a boolean parameter shows sits in an `if`.
pub(crate) fn arkui(ctx: &Ctx, id: NodeId, depth: usize, out: &mut String, extra: &[String]) {
    let Some(n) = ctx.doc.node(id) else { return };
    if id != ctx.root
        && let Some(p) = ctx.param(n, "visible")
    {
        let pad = indent(depth);
        out.push_str(&format!("{pad}if (this.{}) {{\n", p.ident));
        arkui_layer(ctx, n, depth + 1, out, extra);
        out.push_str(&format!("{pad}}}\n"));
        return;
    }
    arkui_layer(ctx, n, depth, out, extra);
}

fn text_layer(ctx: &Ctx, n: &Node, t: &figcraft_doc::TextData, pad: &str, out: &mut String, mods: &mut Vec<String>) {
    let text = ctx.param(n, "characters").map_or_else(|| literal(&t.characters, false), |p| format!("this.{}", p.ident));
    out.push_str(&format!("{pad}Text({text})\n"));
    let s = &t.style;
    mods.push(format!(".fontSize({})", num(s.font_size)));
    mods.push(format!(".fontWeight({})", s.weight().clamp(100, 900)));
    mods.push(format!(".fontFamily({})", literal(&s.font_family, false)));
    if s.italic() {
        mods.push(".fontStyle(FontStyle.Italic)".into());
    }
    if let Some(c) = first_fill(n) {
        mods.push(format!(".fontColor({})", ark_color(c)));
    }
    match s.line_height {
        LineHeight::Pixels(v) => mods.push(format!(".lineHeight({})", num(v))),
        // ArkUI takes a number of vp, not a percent string: a percent is of the font size.
        LineHeight::Percent(p) => mods.push(format!(".lineHeight({})", num(s.font_size * p / 100.0))),
        LineHeight::Auto => {}
    }
    match s.letter_spacing {
        LetterSpacing::Pixels(v) if v != 0.0 => mods.push(format!(".letterSpacing({})", num(v))),
        LetterSpacing::Percent(p) if p != 0.0 => mods.push(format!(".letterSpacing({})", num(s.font_size * p / 100.0))),
        _ => {}
    }
    match t.align_h {
        TextAlignH::Left => {}
        TextAlignH::Center => mods.push(".textAlign(TextAlign.Center)".into()),
        TextAlignH::Right => mods.push(".textAlign(TextAlign.End)".into()),
        TextAlignH::Justified => mods.push(".textAlign(TextAlign.JUSTIFY)".into()),
    }
    // Aligning needs a box to align in: the text layer's own width.
    if !matches!(t.align_h, TextAlignH::Left) {
        mods.push(format!(".width({})", num(n.width)));
    }
    match s.case {
        TextCase::Upper => mods.push(".textCase(TextCase.UpperCase)".into()),
        TextCase::Lower => mods.push(".textCase(TextCase.LowerCase)".into()),
        _ => {}
    }
    match s.decoration {
        TextDecoration::Underline => mods.push(".decoration({ type: TextDecorationType.Underline })".into()),
        TextDecoration::Strikethrough => mods.push(".decoration({ type: TextDecorationType.LineThrough })".into()),
        TextDecoration::None => {}
    }
}

fn frame_layer(ctx: &Ctx, n: &Node, depth: usize, pad: &str, out: &mut String, mods: &mut Vec<String>) {
    let doc = ctx.doc;
    let open = match n.layout.as_ref().map(|l| (l.mode, l)) {
        Some((LayoutMode::Vertical, l)) => format!("Column({{ space: {} }})", num(l.gap)),
        Some((_, l)) => format!("Row({{ space: {} }})", num(l.gap)),
        None => "Stack({ alignContent: Alignment.TopStart })".to_string(),
    };
    out.push_str(&format!("{pad}{open} {{\n"));
    match ctx.param(n, "slot") {
        // A slot shows the instance's content, or else the main's (its default builder).
        Some(p) => out.push_str(&format!("{pad}    this.{}()\n", p.ident)),
        None => {
            for c in ctx.kids(n) {
                // Free children sit at their position inside a Stack, and so does a child
                // that ignores its auto layout parent.
                let pos: Vec<String> = match doc.node(c).filter(|cn| n.layout.is_none() || cn.child_layout.absolute) {
                    Some(cn) => {
                        let [_, _, _, _, x, y] = cn.transform.as_coeffs();
                        vec![format!(".position({{ x: {}, y: {} }})", num(x), num(y))]
                    }
                    None => Vec::new(),
                };
                arkui(ctx, c, depth + 1, out, &pos);
            }
        }
    }
    out.push_str(&format!("{pad}}}\n"));
    if let Some(l) = &n.layout {
        layout_mods(l, mods);
    }
    mods.push(format!(".width({})", num(n.width)));
    mods.push(format!(".height({})", num(n.height)));
    if let Some(c) = first_fill(n) {
        mods.push(format!(".backgroundColor({})", ark_color(c)));
    }
    if radius(n) > 0.0 {
        mods.push(".clip(true)".into());
    }
    ark_radius(n, mods);
}

fn layout_mods(l: &figcraft_doc::AutoLayout, mods: &mut Vec<String>) {
    let [t, r, b, lft] = l.padding;
    if [t, r, b, lft].iter().any(|v| *v > 0.0) {
        mods.push(format!(".padding({{ top: {}, right: {}, bottom: {}, left: {} }})", num(t), num(r), num(b), num(lft)));
    }
    let justify = match l.main_align {
        MainAlign::Min => None,
        MainAlign::Center => Some("FlexAlign.Center"),
        MainAlign::Max => Some("FlexAlign.End"),
        MainAlign::SpaceBetween => Some("FlexAlign.SpaceBetween"),
        MainAlign::SpaceAround => Some("FlexAlign.SpaceAround"),
        MainAlign::SpaceEvenly => Some("FlexAlign.SpaceEvenly"),
    };
    if let Some(j) = justify {
        mods.push(format!(".justifyContent({j})"));
    }
    let cross = match l.cross_align {
        CrossAlign::Min => None,
        CrossAlign::Center => Some("Center"),
        CrossAlign::Max => Some(if l.mode == LayoutMode::Vertical { "End" } else { "Bottom" }),
        CrossAlign::Baseline => None,
    };
    if let Some(c) = cross {
        let ty = if l.mode == LayoutMode::Vertical { "HorizontalAlign" } else { "VerticalAlign" };
        mods.push(format!(".alignItems({ty}.{c})"));
    }
}

fn arkui_layer(ctx: &Ctx, n: &Node, depth: usize, out: &mut String, extra: &[String]) {
    let pad = indent(depth);
    if ctx.too_deep(depth) {
        out.push_str(&format!("{pad}// {} …\n", one_line(&n.name)));
        return;
    }
    // Nested instances are calls; an instance swap parameter is a builder.
    let swap = ctx.param(n, "mainComponent").filter(|_| n.id != ctx.root);
    let call = match swap {
        Some(p) => Some(format!("{pad}this.{}()\n", p.ident)),
        None => ctx.call(n, CodeLang::ArkUi, depth).map(|(code, _)| code),
    };
    if let Some(call) = call {
        if let Some(p) = swap.filter(|_| !extra.is_empty()) {
            // The builder returns nothing, so a position goes on a container around the call.
            out.push_str(&format!("{pad}Stack() {{\n{pad}    this.{}()\n{pad}}}\n", p.ident));
            for m in extra {
                out.push_str(&format!("{pad}{m}\n"));
            }
            return;
        }
        out.push_str(&call);
        for m in extra {
            out.push_str(&format!("{pad}    {m}\n"));
        }
        return;
    }
    let mut mods: Vec<String> = Vec::new();
    match &n.kind {
        NodeKind::Text(t) => {
            text_layer(ctx, n, t, &pad, out, &mut mods);
        }
        NodeKind::Ellipse { .. } => {
            out.push_str(&format!("{pad}Ellipse({{ width: {}, height: {} }})\n", num(n.width), num(n.height)));
            if let Some(c) = first_fill(n) {
                mods.push(format!(".fill({})", ark_color(c)));
            }
            if let Some(c) = stroke_color(n) {
                mods.push(format!(".stroke({})", ark_color(c)));
                mods.push(format!(".strokeWidth({})", num(n.stroke.weight)));
            }
        }
        NodeKind::Rectangle => {
            out.push_str(&format!("{pad}Column()\n"));
            if let Some(c) = first_fill(n) {
                mods.push(format!(".backgroundColor({})", ark_color(c)));
            }
            ark_radius(n, &mut mods);
            mods.push(format!(".width({})", num(n.width)));
            mods.push(format!(".height({})", num(n.height)));
        }
        k if k.is_frame_like() || matches!(k, NodeKind::Group) => {
            frame_layer(ctx, n, depth, &pad, out, &mut mods);
        }
        _ => {
            out.push_str(&format!("{pad}// {}: export as an image asset\n{pad}Image($r('app.media.asset'))\n", n.kind.label()));
            mods.push(format!(".width({})", num(n.width)));
            mods.push(format!(".height({})", num(n.height)));
        }
    }
    // Ellipses take their stroke as a shape attribute (above).
    if !matches!(n.kind, NodeKind::Ellipse { .. })
        && let Some(c) = stroke_color(n)
    {
        mods.push(format!(".border({{ width: {}, color: {} }})", num(n.stroke.weight), ark_color(c)));
    }
    if let Some(s) = drop_shadow(n) {
        mods.push(format!(
            ".shadow({{ radius: {}, color: {}, offsetX: {}, offsetY: {} }})",
            num(s.radius / 2.0),
            ark_color(s.color),
            num(s.offset.0),
            num(s.offset.1)
        ));
    }
    if n.opacity < 1.0 {
        mods.push(format!(".opacity({})", num(f64::from(n.opacity))));
    }
    let rot = figcraft_geom::rotation_degrees(n.transform);
    if rot.abs() > 0.01 && rot.is_finite() {
        mods.push(format!(".rotate({{ angle: {} }})", num(-rot)));
    }
    mods.extend(extra.iter().cloned());
    for m in mods {
        out.push_str(&format!("{pad}    {m}\n"));
    }
}
