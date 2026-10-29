use figcraft_doc::AutoLayout;

use super::swift::one_line;
use super::{
    CodeLang, CrossAlign, Ctx, LayoutMode, LetterSpacing, LineHeight, MainAlign, Node, NodeId, NodeKind, Param, Sizing, TextAlignH, TextDecoration,
    drop_shadow, first_fill, kotlin_color, literal, num, radius, stroke_color,
};

// ------------------------------------------------------------------ Flutter (Dart)

/// The import a generated Flutter unit starts with (a widget tree on its own needs it too).
pub(crate) const FLUTTER_IMPORT: &str = "import 'package:flutter/material.dart';\n\n";

/// Dart indents by two spaces.
pub(crate) fn flutter_indent(depth: usize) -> String {
    "  ".repeat(depth)
}

/// Where a child sits in its parent: in a Stack at its position in the frame, taking the room left
/// in a Row or Column, or wherever the parent puts it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Place {
    Flow,
    At(f64, f64),
    Fill,
}

/// A child and where its parent puts it.
type Placed = (NodeId, Place);

/// Dart's `FontWeight.w600` (a weight is hundreds).
fn flutter_weight(w: u16) -> String {
    format!("FontWeight.w{}", (w.clamp(100, 900) / 100) * 100)
}

/// A Row or Column's main-axis alignment (`None` for the default, the start).
fn flutter_main_align(a: MainAlign) -> Option<&'static str> {
    match a {
        MainAlign::Min => None,
        MainAlign::Center => Some("MainAxisAlignment.center"),
        MainAlign::Max => Some("MainAxisAlignment.end"),
        MainAlign::SpaceBetween => Some("MainAxisAlignment.spaceBetween"),
        MainAlign::SpaceAround => Some("MainAxisAlignment.spaceAround"),
        MainAlign::SpaceEvenly => Some("MainAxisAlignment.spaceEvenly"),
    }
}

/// A Row or Column's cross-axis alignment; a flow child that fills it stretches the line instead.
fn flutter_cross_align(a: CrossAlign, stretch: bool) -> &'static str {
    if stretch {
        return "CrossAxisAlignment.stretch";
    }
    match a {
        CrossAlign::Min => "CrossAxisAlignment.start",
        CrossAlign::Center => "CrossAxisAlignment.center",
        CrossAlign::Max => "CrossAxisAlignment.end",
        CrossAlign::Baseline => "CrossAxisAlignment.baseline",
    }
}

/// A frame's auto layout padding as `EdgeInsets` (`None` when it has none).
fn flutter_padding(n: &Node) -> Option<String> {
    let l = n.layout.as_ref()?;
    let [t, r, b, left] = l.padding;
    if ![t, r, b, left].iter().any(|v| *v > 0.0) {
        return None;
    }
    Some(if t == r && r == b && b == left {
        format!("EdgeInsets.all({})", num(t))
    } else if t == b && left == r {
        format!("EdgeInsets.symmetric(vertical: {}, horizontal: {})", num(t), num(left))
    } else {
        format!("EdgeInsets.fromLTRB({}, {}, {}, {})", num(left), num(t), num(r), num(b))
    })
}

/// A layer's fill, shape, corner radius, stroke and shadow as a `BoxDecoration`; `None` when it has
/// none of them. `pad` is the indentation of the constructor itself.
fn box_decoration(n: &Node, pad: &str) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(c) = first_fill(n) {
        parts.push(format!("color: {}", kotlin_color(c)));
    }
    if matches!(n.kind, NodeKind::Ellipse { .. }) {
        parts.push("shape: BoxShape.circle".into());
    } else if radius(n) > 0.0 {
        parts.push(format!("borderRadius: BorderRadius.circular({})", num(radius(n))));
    }
    if let Some(c) = stroke_color(n) {
        parts.push(format!("border: Border.all(color: {}, width: {})", kotlin_color(c), num(n.stroke.weight)));
    }
    if let Some(s) = drop_shadow(n) {
        parts.push(format!(
            "boxShadow: [BoxShadow(color: {}, blurRadius: {}, offset: Offset({}, {}))]",
            kotlin_color(s.color),
            num(s.radius / 2.0),
            num(s.offset.0),
            num(s.offset.1)
        ));
    }
    if parts.is_empty() {
        return None;
    }
    let inner = format!("{pad}  ");
    Some(format!("BoxDecoration(\n{}{pad})", parts.iter().map(|p| format!("{inner}{p},\n")).collect::<String>()))
}

/// Flutter for a layer and its subtree at `depth`. A layer a boolean parameter shows sits behind an
/// `if (…)`: Dart's lists take conditions, so the element itself carries it.
pub(crate) fn flutter(ctx: &Ctx, id: NodeId, depth: usize, out: &mut String, place: Place) {
    let Some(n) = ctx.doc.node(id) else { return };
    if id != ctx.root
        && let Some(p) = ctx.param(n, "visible")
    {
        out.push_str(&format!("{}if ({})\n", flutter_indent(depth), p.ident));
        flutter_placed(ctx, n, depth + 1, out, place);
        return;
    }
    flutter_placed(ctx, n, depth, out, place);
}

/// A layer at `depth` and what wraps it: where its parent puts it, then its opacity. Every closing
/// parenthesis sits at its own level, so the caller can end the element with a comma in a list.
fn flutter_placed(ctx: &Ctx, n: &Node, depth: usize, out: &mut String, place: Place) {
    if ctx.too_deep(depth) {
        let pad = flutter_indent(depth);
        out.push_str(&format!("{pad}// {} …\n{pad}const SizedBox()", one_line(&n.name)));
        return;
    }
    let mut depth = depth;
    let mut close: Vec<String> = Vec::new();
    match place {
        Place::Flow => {}
        Place::At(x, y) => {
            let (pad, inner) = (flutter_indent(depth), flutter_indent(depth + 1));
            out.push_str(&format!("{pad}Positioned(\n{inner}left: {},\n{inner}top: {},\n{inner}child: ", num(x), num(y)));
            close.push(format!("\n{pad})"));
            depth += 1;
        }
        Place::Fill => {
            let (pad, inner) = (flutter_indent(depth), flutter_indent(depth + 1));
            out.push_str(&format!("{pad}Expanded(\n{inner}child: "));
            close.push(format!("\n{pad})"));
            depth += 1;
        }
    }
    if n.opacity < 1.0 {
        let (pad, inner) = (flutter_indent(depth), flutter_indent(depth + 1));
        out.push_str(&format!("{pad}Opacity(\n{inner}opacity: {},\n{inner}child: ", num(f64::from(n.opacity))));
        close.push(format!("\n{pad})"));
        depth += 1;
    }
    // A wrapper leaves its line open ("child: "), so the widget carries no indentation of its own.
    let mut body = String::new();
    flutter_widget(ctx, n, depth, &mut body);
    out.push_str(if close.is_empty() { body.as_str() } else { body.trim_start() });
    for c in close.iter().rev() {
        out.push_str(c);
    }
}

/// A layer's own look and its subtree as a Flutter widget at `depth`: a complete expression whose
/// closing parenthesis sits at `depth`.
fn flutter_widget(ctx: &Ctx, n: &Node, depth: usize, out: &mut String) {
    let (pad, inner) = (flutter_indent(depth), flutter_indent(depth + 1));
    // Nested instances are calls; an instance swap parameter is a widget.
    let call = match ctx.param(n, "mainComponent").filter(|_| n.id != ctx.root) {
        Some(p) => Some(p.ident.clone()),
        None => ctx.call(n, CodeLang::Flutter, depth).map(|(code, _)| code),
    };
    if let Some(call) = call {
        out.push_str(call.trim_end_matches('\n'));
        return;
    }
    match &n.kind {
        NodeKind::Text(t) => {
            let text = ctx.param(n, "characters").map_or_else(|| literal(&t.characters, true), |p| p.ident.clone());
            out.push_str(&format!("{pad}Text(\n{inner}{text},\n"));
            match t.align_h {
                TextAlignH::Left => {}
                TextAlignH::Center => out.push_str(&format!("{inner}textAlign: TextAlign.center,\n")),
                TextAlignH::Right => out.push_str(&format!("{inner}textAlign: TextAlign.right,\n")),
                TextAlignH::Justified => out.push_str(&format!("{inner}textAlign: TextAlign.justify,\n")),
            }
            let s = &t.style;
            let mut style: Vec<String> = Vec::new();
            if !s.font_family.is_empty() {
                style.push(format!("fontFamily: {}", literal(&s.font_family, true)));
            }
            style.push(format!("fontSize: {}", num(s.font_size)));
            style.push(format!("fontWeight: {}", flutter_weight(s.weight())));
            if let Some(c) = first_fill(n) {
                style.push(format!("color: {}", kotlin_color(c)));
            }
            if s.italic() {
                style.push("fontStyle: FontStyle.italic".into());
            }
            match s.line_height {
                // Dart's height is a multiple of the font size, not a length.
                LineHeight::Pixels(v) if v > 0.0 && s.font_size > 0.0 => style.push(format!("height: {}", num(v / s.font_size))),
                LineHeight::Percent(p) if p > 0.0 => style.push(format!("height: {}", num(p / 100.0))),
                _ => {}
            }
            match s.letter_spacing {
                LetterSpacing::Pixels(v) if v != 0.0 => style.push(format!("letterSpacing: {}", num(v))),
                LetterSpacing::Percent(p) if p != 0.0 && s.font_size > 0.0 => style.push(format!("letterSpacing: {}", num(p / 100.0 * s.font_size))),
                _ => {}
            }
            match s.decoration {
                TextDecoration::None => {}
                TextDecoration::Underline => style.push("decoration: TextDecoration.underline".into()),
                TextDecoration::Strikethrough => style.push("decoration: TextDecoration.lineThrough".into()),
            }
            let sp = flutter_indent(depth + 2);
            out.push_str(&format!("{inner}style: TextStyle(\n{}{inner}),\n{pad})", style.iter().map(|x| format!("{sp}{x},\n")).collect::<String>()));
        }
        NodeKind::Ellipse { .. } | NodeKind::Rectangle => {
            out.push_str(&format!("{pad}Container(\n{inner}width: {},\n{inner}height: {},\n", num(n.width), num(n.height)));
            if let Some(d) = box_decoration(n, &inner) {
                out.push_str(&format!("{inner}decoration: {d},\n"));
            }
            out.push_str(&format!("{pad})"));
        }
        k if k.is_frame_like() || matches!(k, NodeKind::Group) => {
            let children = ctx.kids(n);
            let slot = ctx.param(n, "slot");
            // The children block says whether it kept the padding for itself (an absolute child
            // puts the line in a Stack at the parent's own box).
            let mut kids = String::new();
            let padded_inside = !(children.is_empty() && slot.is_none()) && flutter_children(ctx, n, &children, slot, depth + 1, &mut kids);
            out.push_str(&format!("{pad}Container(\n{inner}width: {},\n{inner}height: {},\n", num(n.width), num(n.height)));
            if !padded_inside && let Some(p) = flutter_padding(n) {
                out.push_str(&format!("{inner}padding: {p},\n"));
            }
            if let Some(d) = box_decoration(n, &inner) {
                out.push_str(&format!("{inner}decoration: {d},\n"));
            }
            if children.is_empty() && slot.is_none() {
                out.push_str(&format!("{pad})"));
                return;
            }
            out.push_str(&format!("{inner}alignment: Alignment.topLeft,\n{inner}child: "));
            out.push_str(kids.trim_start());
            out.push_str(&format!(",\n{pad})"));
        }
        _ => out.push_str(&format!(
            "{pad}// {}: export as an image asset\n{pad}Image.asset({}, width: {}, height: {})",
            n.kind.label(),
            literal(&format!("{}.png", n.name), true),
            num(n.width),
            num(n.height)
        )),
    }
}

/// The Row, Column or Stack holding a frame-like layer's children, at `depth`. Returns whether it
/// took the parent's padding over (see [`flutter_widget`]).
fn flutter_children(ctx: &Ctx, n: &Node, children: &[NodeId], slot: Option<&Param>, depth: usize, out: &mut String) -> bool {
    let doc = ctx.doc;
    let (pad, inner) = (flutter_indent(depth), flutter_indent(depth + 1));
    // Where each child sits, and whether one of the children in the line asks it to stretch.
    let mut placed: Vec<Placed> = Vec::new();
    let mut stretch = false;
    for c in children {
        let Some(cn) = doc.node(*c) else { continue };
        let place = match n.layout.as_ref() {
            // A child that ignores its auto layout parent keeps its own offsets: it leaves the
            // line, so it neither stretches it nor takes the room left in it.
            Some(_) if cn.child_layout.absolute => {
                let [_, _, _, _, x, y] = cn.transform.as_coeffs();
                Place::At(x, y)
            }
            Some(l) => {
                let vertical = l.mode == LayoutMode::Vertical;
                let (main, cross) = if vertical {
                    (cn.child_layout.sizing_v, cn.child_layout.sizing_h)
                } else {
                    (cn.child_layout.sizing_h, cn.child_layout.sizing_v)
                };
                stretch |= cross == Sizing::Fill;
                if main == Sizing::Fill { Place::Fill } else { Place::Flow }
            }
            // Free children sit at their position in the frame.
            None => {
                let [_, _, _, _, x, y] = cn.transform.as_coeffs();
                Place::At(x, y)
            }
        };
        placed.push((*c, place));
    }
    let Some(l) = n.layout.as_ref() else {
        out.push_str(&format!("{pad}Stack(\n{inner}alignment: Alignment.topLeft,\n{inner}children: [\n"));
        flutter_items(ctx, &placed, slot, depth + 2, out);
        out.push_str(&format!("{inner}],\n{pad})"));
        return false;
    };
    let (over, flow): (Vec<Placed>, Vec<Placed>) = placed.into_iter().partition(|(_, p)| matches!(p, Place::At(..)));
    if over.is_empty() {
        out.push_str(&flutter_line(ctx, l, &flow, slot, stretch, depth));
        return false;
    }
    // A child at its own offsets goes over the line, in a Stack at the parent's own box, so its
    // offsets stay the parent's and the line keeps the parent's padding.
    let pad_edges = flutter_padding(n);
    let list = flutter_indent(depth + 2);
    let line_depth = depth + 3 + usize::from(pad_edges.is_some());
    out.push_str(&format!("{pad}Stack(\n{inner}alignment: Alignment.topLeft,\n{inner}children: [\n"));
    if !flow.is_empty() || slot.is_some() {
        // Filling the Stack gives the line the parent's size, which `Expanded` children need.
        out.push_str(&format!("{list}Positioned.fill(\n{list}  child: "));
        match pad_edges {
            Some(p) => {
                out.push_str(&format!("Padding(\n{list}    padding: {p},\n{list}    child: "));
                out.push_str(flutter_line(ctx, l, &flow, slot, stretch, line_depth).trim_start());
                out.push_str(&format!(",\n{list}  )"));
            }
            None => out.push_str(flutter_line(ctx, l, &flow, slot, stretch, line_depth).trim_start()),
        }
        out.push_str(&format!(",\n{list}),\n"));
    }
    flutter_items(ctx, &over, None, depth + 2, out);
    out.push_str(&format!("{inner}],\n{pad})"));
    true
}

/// The Row or Column holding a frame's flow children, at `depth`: a complete widget expression.
fn flutter_line(ctx: &Ctx, l: &AutoLayout, flow: &[Placed], slot: Option<&Param>, stretch: bool, depth: usize) -> String {
    let (pad, inner) = (flutter_indent(depth), flutter_indent(depth + 1));
    let mut out = format!("{pad}{}(\n", if l.mode == LayoutMode::Vertical { "Column" } else { "Row" });
    if let Some(m) = flutter_main_align(l.main_align) {
        out.push_str(&format!("{inner}mainAxisAlignment: {m},\n"));
    }
    out.push_str(&format!("{inner}crossAxisAlignment: {},\n", flutter_cross_align(l.cross_align, stretch)));
    if !stretch && matches!(l.cross_align, CrossAlign::Baseline) {
        out.push_str(&format!("{inner}textBaseline: TextBaseline.alphabetic,\n"));
    }
    // The gap between the children; the space distributions already spread them.
    if l.gap > 0.0 && flutter_main_align(l.main_align).is_none_or(|m| !m.contains("Space")) {
        out.push_str(&format!("{inner}spacing: {},\n", num(l.gap)));
    }
    out.push_str(&format!("{inner}children: [\n"));
    flutter_items(ctx, flow, slot, depth + 2, &mut out);
    out.push_str(&format!("{inner}],\n{pad})"));
    out
}

/// A children list (`[` already open) at `depth`; a slot shows the instance's content, or else the
/// main's.
fn flutter_items(ctx: &Ctx, kids: &[Placed], slot: Option<&Param>, depth: usize, out: &mut String) {
    let list = flutter_indent(depth);
    if let Some(p) = slot {
        let name = &p.ident;
        out.push_str(&format!("{list}...?{name},\n"));
        out.push_str(&format!("{list}if ({name} == null) ...[\n"));
    }
    let kid = depth + usize::from(slot.is_some());
    for (id, place) in kids {
        flutter(ctx, *id, kid, out, *place);
        out.push_str(",\n");
    }
    if slot.is_some() {
        out.push_str(&format!("{list}],\n"));
    }
}
