use super::{
    CodeLang, Color, Ctx, Document, GradientKind, LayoutMode, Node, NodeId, NodeKind, PADDING, Paint, PaintKind, drop_shadow, first_fill,
    gradient_axis, gradient_stops, indent, literal, named, native_gradient_kind, num, radius, stroke_color, visible_fills,
};

// ------------------------------------------------------------------ SwiftUI

fn swift_color(c: Color) -> String {
    format!("Color(red: {}, green: {}, blue: {}, opacity: {})", num(f64::from(c.r)), num(f64::from(c.g)), num(f64::from(c.b)), num(f64::from(c.a)))
}

fn swift_point(p: (f64, f64)) -> String {
    format!("UnitPoint(x: {}, y: {})", num(p.0), num(p.1))
}

/// A gradient fill on a layer of `w` x `h` as a SwiftUI gradient.
fn swift_gradient(g: &figcraft_color::Gradient, w: f64, h: f64, opacity: f32) -> String {
    let stops = gradient_stops(g, opacity)
        .into_iter()
        .map(|(at, c)| format!("Gradient.Stop(color: {}, location: {})", swift_color(c), num(at)))
        .collect::<Vec<_>>()
        .join(", ");
    let (dx, dy) = gradient_axis(g, w, h);
    match native_gradient_kind(g) {
        GradientKind::Linear => format!("LinearGradient(stops: [{stops}], startPoint: {}, endPoint: {})", swift_point(g.from), swift_point(g.to)),
        GradientKind::Angular => {
            format!("AngularGradient(stops: [{stops}], center: {}, angle: .degrees({}))", swift_point(g.from), num(dy.atan2(dx).to_degrees()))
        }
        _ => format!("RadialGradient(stops: [{stops}], center: {}, startRadius: 0, endRadius: {})", swift_point(g.from), num(dx.hypot(dy))),
    }
}

/// One paint (the one at document index `i`) as SwiftUI; `None` for an image or pattern, which no
/// native target writes yet. Only a solid at index 0 takes the `fill` variable's name.
fn swift_paint(doc: &Document, n: &Node, i: usize, p: &Paint) -> Option<String> {
    match &p.kind {
        PaintKind::Solid { .. } => {
            let c = swift_color(p.solid_color()?);
            Some(if i == 0 { named(doc, n, "iOS", &["fill"], c) } else { c })
        }
        PaintKind::Gradient(g) => Some(swift_gradient(g, n.width, n.height, p.opacity)),
        PaintKind::Image(_) | PaintKind::Pattern(_) | PaintKind::Shader(_) => None,
    }
}

/// The modifiers filling a rectangle or ellipse: the bottom paint fills the `shape`, and each paint
/// above it is a copy of the shape laid over it.
fn swift_shape_fills(doc: &Document, n: &Node, shape: &str) -> Vec<String> {
    let paints = visible_fills(n).filter_map(|(i, p)| swift_paint(doc, n, i, p));
    paints.enumerate().map(|(at, c)| if at == 0 { format!(".fill({c})") } else { format!(".overlay({shape}.fill({c}))") }).collect()
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

/// A layer name safe for a single-line `//` comment: line breaks become spaces.
pub(super) fn one_line(name: &str) -> String {
    name.replace(['\n', '\r', '\u{85}', '\u{2028}', '\u{2029}'], " ")
}

fn swift_layer(ctx: &Ctx, n: &Node, depth: usize, out: &mut String, extra: &[String]) {
    let doc = ctx.doc;
    let pad = indent(depth);
    if ctx.too_deep(depth) {
        out.push_str(&format!("{pad}// {} …\n", one_line(&n.name)));
        return;
    }
    // Nested instances are calls; an instance swap parameter is a view.
    let call = match ctx.param(n, "mainComponent").filter(|_| n.id != ctx.root) {
        Some(p) => Some(format!("{pad}{}\n", p.ident)),
        None => ctx.call(n, CodeLang::SwiftUi, depth).map(|(code, _)| code),
    };
    if let Some(call) = call {
        out.push_str(&call);
        for m in extra {
            out.push_str(&format!("{pad}    {m}\n"));
        }
        return;
    }
    // A bound variable's iOS code syntax in place of the value.
    let bound = |fields: &[&str], value: String| named(doc, n, "iOS", fields, value);
    let fill = first_fill(n).map(|c| bound(&["fill"], swift_color(c)));
    let (w, h) = (bound(&["width"], num(n.width)), bound(&["height"], num(n.height)));
    let corner = bound(&["cornerRadius"], num(radius(n)));
    let mut mods: Vec<String> = Vec::new();
    match &n.kind {
        NodeKind::Text(t) => {
            let text = ctx.param(n, "characters").map_or_else(|| bound(&["characters"], literal(&t.characters, false)), |p| p.ident.clone());
            out.push_str(&format!("{pad}Text({text})\n"));
            mods.push(format!(
                ".font(.custom({}, size: {}).weight({}))",
                bound(&["fontFamily"], literal(&t.style.font_family, false)),
                bound(&["fontSize"], num(t.style.font_size)),
                swift_weight(t.style.weight())
            ));
            if let Some(c) = &fill {
                mods.push(format!(".foregroundColor({c})"));
            }
        }
        NodeKind::Ellipse { .. } => {
            out.push_str(&format!("{pad}Ellipse()\n"));
            mods.extend(swift_shape_fills(doc, n, "Ellipse()"));
            mods.push(format!(".frame(width: {w}, height: {h})"));
        }
        NodeKind::Rectangle => {
            out.push_str(&format!("{pad}RoundedRectangle(cornerRadius: {corner})\n"));
            mods.extend(swift_shape_fills(doc, n, &format!("RoundedRectangle(cornerRadius: {corner})")));
            mods.push(format!(".frame(width: {w}, height: {h})"));
        }
        k if k.is_frame_like() || matches!(k, NodeKind::Group) => {
            let children = ctx.kids(n);
            let open = match n.layout.as_ref().map(|l| (l.mode, l)) {
                Some((LayoutMode::Vertical, l)) => format!("VStack(alignment: .leading, spacing: {}) {{", bound(&["gap"], num(l.gap))),
                Some((_, l)) => format!("HStack(alignment: .center, spacing: {}) {{", bound(&["gap"], num(l.gap))),
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
                    let side = |at: usize, v: f64| bound(&PADDING[at], num(v));
                    mods.push(format!(
                        ".padding(EdgeInsets(top: {}, leading: {}, bottom: {}, trailing: {}))",
                        side(0, t),
                        side(3, lft),
                        side(2, b),
                        side(1, r)
                    ));
                }
            }
            mods.push(format!(".frame(width: {w}, height: {h}, alignment: .topLeading)"));
            // Each background goes behind the ones before it, so the top fill is written first.
            mods.extend(visible_fills(n).rev().filter_map(|(i, p)| swift_paint(doc, n, i, p)).map(|c| format!(".background({c})")));
            if radius(n) > 0.0 {
                mods.push(format!(".clipShape(RoundedRectangle(cornerRadius: {corner}))"));
            }
        }
        _ => {
            out.push_str(&format!("{pad}// {}: export as an image asset\n{pad}Image({})\n", n.kind.label(), literal(&n.name, false)));
            mods.push(".resizable()".into());
            mods.push(format!(".frame(width: {w}, height: {h})"));
        }
    }
    if let Some(c) = stroke_color(n) {
        mods.push(format!(
            ".overlay(RoundedRectangle(cornerRadius: {corner}).stroke({}, lineWidth: {}))",
            bound(&["stroke"], swift_color(c)),
            bound(&["strokeWeight"], num(n.stroke.weight))
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
        mods.push(format!(".opacity({})", bound(&["opacity"], num(f64::from(n.opacity)))));
    }
    mods.extend(extra.iter().cloned());
    for m in mods {
        out.push_str(&format!("{pad}    {m}\n"));
    }
}
