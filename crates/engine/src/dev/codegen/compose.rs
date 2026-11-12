use super::swift::one_line;
use super::{
    CodeLang, Color, Ctx, LayoutMode, Node, NodeId, NodeKind, PADDING, drop_shadow, first_fill, indent, literal, named, num, radius, stroke_color,
};
use figcraft_doc::Document;

// ------------------------------------------------------------------ Jetpack Compose

/// `Color(0xAARRGGBB)`, as Compose and Flutter both write a colour.
pub(crate) fn kotlin_color(c: Color) -> String {
    let [r, g, b, a] = c.to_rgba8();
    format!("Color(0x{a:02X}{r:02X}{g:02X}{b:02X})")
}

/// A bound variable's Android code syntax, else `value`.
fn bound(doc: &Document, n: &Node, fields: &[&str], value: String) -> String {
    named(doc, n, "ANDROID", fields, value)
}

/// [`bound`] for a length: `value` in `dp`.
fn bound_dp(doc: &Document, n: &Node, fields: &[&str], value: f64) -> String {
    bound(doc, n, fields, format!("{}.dp", num(value)))
}

/// The Modifier chain for a layer (size, shadow, background, border, alpha, padding).
fn modifier(doc: &Document, n: &Node, shape: &str, padding: bool) -> String {
    let mut m = format!("Modifier.size({}, {})", bound_dp(doc, n, &["width"], n.width), bound_dp(doc, n, &["height"], n.height));
    if let Some(s) = drop_shadow(n) {
        m.push_str(&format!(".shadow({}.dp, {shape})", num(s.radius / 2.0)));
    }
    if let Some(c) = first_fill(n) {
        m.push_str(&format!(".background({}, {shape})", bound(doc, n, &["fill"], kotlin_color(c))));
    }
    if let Some(c) = stroke_color(n) {
        m.push_str(&format!(
            ".border({}, {}, {shape})",
            bound_dp(doc, n, &["strokeWeight"], n.stroke.weight),
            bound(doc, n, &["stroke"], kotlin_color(c))
        ));
    }
    if n.opacity < 1.0 {
        m.push_str(&format!(".alpha({})", bound(doc, n, &["opacity"], format!("{}f", num(f64::from(n.opacity))))));
    }
    if padding && let Some(l) = &n.layout {
        let [t, r, b, lft] = l.padding;
        if [t, r, b, lft].iter().any(|v| *v > 0.0) {
            let side = |at: usize, v: f64| bound_dp(doc, n, &PADDING[at], v);
            m.push_str(&format!(".padding(start = {}, top = {}, end = {}, bottom = {})", side(3, lft), side(0, t), side(1, r), side(2, b)));
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
        out.push_str(&format!("{pad}// {} …\n", one_line(&n.name)));
        return;
    }
    // Nested instances are calls; an instance swap parameter is a composable.
    let call = match ctx.param(n, "mainComponent").filter(|_| n.id != ctx.root) {
        Some(p) => Some(format!("{pad}{}()\n", p.ident)),
        None => ctx.call(n, CodeLang::Compose, depth).map(|(code, _)| code),
    };
    if let Some(call) = call {
        out.push_str(&call);
        return;
    }
    let shape = if radius(n) > 0.0 {
        format!("RoundedCornerShape({})", bound_dp(doc, n, &["cornerRadius"], radius(n)))
    } else {
        "RectangleShape".to_string()
    };
    match &n.kind {
        NodeKind::Text(t) => {
            let color = first_fill(n).map_or_else(|| "Color.Unspecified".into(), |c| bound(doc, n, &["fill"], kotlin_color(c)));
            let text = ctx.param(n, "characters").map_or_else(|| bound(doc, n, &["characters"], literal(&t.characters, true)), |p| p.ident.clone());
            out.push_str(&format!(
                "{pad}Text(\n{pad}    text = {text},\n{pad}    fontSize = {},\n{pad}    fontWeight = FontWeight.W{},\n{pad}    color = {color},\n{pad})\n",
                bound(doc, n, &["fontSize"], format!("{}.sp", num(t.style.font_size))),
                (t.style.weight().clamp(100, 900) / 100) * 100
            ));
        }
        NodeKind::Ellipse { .. } => out.push_str(&format!("{pad}Box(modifier = {})\n", modifier(doc, n, "CircleShape", false))),
        NodeKind::Rectangle => out.push_str(&format!("{pad}Box(modifier = {})\n", modifier(doc, n, &shape, false))),
        k if k.is_frame_like() || matches!(k, NodeKind::Group) => {
            let children = ctx.kids(n);
            let open = match n.layout.as_ref().map(|l| (l.mode, l)) {
                Some((LayoutMode::Vertical, l)) => format!(
                    "Column(\n{pad}    modifier = {},\n{pad}    verticalArrangement = Arrangement.spacedBy({}),\n{pad}) {{",
                    modifier(doc, n, &shape, true),
                    bound_dp(doc, n, &["gap"], l.gap)
                ),
                Some((_, l)) => format!(
                    "Row(\n{pad}    modifier = {},\n{pad}    horizontalArrangement = Arrangement.spacedBy({}),\n{pad}    verticalAlignment = Alignment.CenterVertically,\n{pad}) {{",
                    modifier(doc, n, &shape, true),
                    bound_dp(doc, n, &["gap"], l.gap)
                ),
                None => format!("Box(modifier = {}) {{", modifier(doc, n, &shape, false)),
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
