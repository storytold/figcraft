//! Code for a layer tree in the native languages the Inspect panel offers besides SwiftUI and
//! Jetpack Compose: UIKit (iOS), Android XML views and Flutter (Dart). They read the same
//! document the other generators do, so a layer looks the same in every one of them.

use figcraft_color::{Color, PaintKind};
use figcraft_doc::*;

use crate::codegen::num;

/// The layer tree as UIKit views (iOS).
pub fn uikit(doc: &Document, id: NodeId) -> String {
    let Some(root) = doc.node(id) else { return String::new() };
    let name = crate::codegen::class_name(&root.name, &mut Vec::new());
    let mut out = format!("import UIKit\n\nfunc make{}() -> UIView {{\n", component_name(&name));
    let mut var = Var::default();
    let root_name = uikit_layer(doc, root, None, 1, &mut var, &mut out);
    out.push_str(&format!("  return {root_name}\n}}\n"));
    out
}

/// A component name from a class name: "card" → "Card".
fn component_name(class: &str) -> String {
    let mut out = String::new();
    for part in class.split(['-', '_', ' ']) {
        let mut c = part.chars();
        if let Some(f) = c.next() {
            out.push(f.to_ascii_uppercase());
            out.push_str(c.as_str());
        }
    }
    if out.is_empty() { "View".into() } else { out }
}

/// The names the generated views use, unique in the file.
#[derive(Default)]
struct Var {
    used: Vec<String>,
}

impl Var {
    /// A unique name for a view: "Card" → "card", "card-2" → "card2".
    fn name(&mut self, layer: &str) -> String {
        let mut base = String::new();
        for c in layer.chars() {
            if c.is_ascii_alphanumeric() {
                base.push(c.to_ascii_lowercase());
            } else if !base.is_empty() && !base.ends_with('_') {
                base.push('_');
            }
        }
        let base = base.trim_matches('_').to_string();
        let base = if base.is_empty() { "view".to_string() } else { base };
        let mut name = base.clone();
        let mut k = 2;
        while self.used.contains(&name) {
            name = format!("{base}{k}");
            k += 1;
        }
        self.used.push(name.clone());
        name
    }
}

/// A layer as UIKit views; the children are added as subviews. Returns the view's name.
fn uikit_layer(doc: &Document, n: &Node, parent: Option<&Node>, depth: usize, var: &mut Var, out: &mut String) -> String {
    let pad = "  ".repeat(depth);
    let name = var.name(&n.name);
    let [_, _, _, _, x, y] = n.transform.as_coeffs();
    let in_flow = parent.is_some_and(|p| p.layout.is_some()) && !n.child_layout.absolute;
    let (ox, oy) = if in_flow { (0.0, 0.0) } else { (x, y) };
    let kind = if matches!(n.kind, NodeKind::Text(_)) { "UILabel" } else { "UIView" };
    out.push_str(&format!(
        "{pad}let {name} = {kind}(frame: CGRect(x: {}, y: {}, width: {}, height: {}))\n",
        num(ox),
        num(oy),
        num(n.width),
        num(n.height)
    ));
    if let NodeKind::Text(t) = &n.kind {
        out.push_str(&format!("{pad}{name}.text = {}\n", swift_string(&t.characters)));
        out.push_str(&format!(
            "{pad}{name}.font = UIFont.systemFont(ofSize: {}, weight: {})\n",
            num(t.style.font_size),
            uikit_weight(t.style.weight())
        ));
        if let Some(c) = first_fill(n) {
            out.push_str(&format!("{pad}{name}.textColor = {}\n", uikit_color(c)));
        }
    } else if let Some(c) = first_fill(n) {
        out.push_str(&format!("{pad}{name}.backgroundColor = {}\n", uikit_color(c)));
    }
    let radius = crate::codegen::radius(n);
    if radius > 0.0 {
        out.push_str(&format!("{pad}{name}.layer.cornerRadius = {}\n", num(radius)));
    }
    if let Some(c) = stroke_color(n) {
        out.push_str(&format!("{pad}{name}.layer.borderWidth = {}\n", num(n.stroke.weight)));
        out.push_str(&format!("{pad}{name}.layer.borderColor = {}.cgColor\n", uikit_color(c)));
    }
    if n.opacity < 1.0 {
        out.push_str(&format!("{pad}{name}.alpha = {}\n", num(f64::from(n.opacity))));
    }
    if let Some(l) = &n.layout {
        let [t, r, b, lft] = l.padding;
        if [t, r, b, lft].iter().any(|v| *v > 0.0) {
            out.push_str(&format!(
                "{pad}{name}.layoutMargins = UIEdgeInsets(top: {}, left: {}, bottom: {}, right: {})\n",
                num(t),
                num(lft),
                num(b),
                num(r)
            ));
        }
    }
    let kids: Vec<NodeId> = n.children.iter().copied().filter(|c| doc.node(*c).is_some_and(|k| k.visible)).collect();
    for k in kids {
        let Some(kn) = doc.node(k) else { continue };
        let child = uikit_layer(doc, kn, Some(n), depth, var, out);
        out.push_str(&format!("{pad}{name}.addSubview({child})\n"));
    }
    name
}

/// A UIColor for a colour: `UIColor(red: 1, green: 0, blue: 0, alpha: 1)`.
fn uikit_color(c: Color) -> String {
    format!("UIColor(red: {}, green: {}, blue: {}, alpha: {})", num(f64::from(c.r)), num(f64::from(c.g)), num(f64::from(c.b)), num(f64::from(c.a)))
}

/// The UIFont weight for a text weight: 600 → `.semibold`.
fn uikit_weight(w: u16) -> &'static str {
    match w {
        0..=150 => ".ultraLight",
        151..=250 => ".thin",
        251..=350 => ".light",
        351..=450 => ".regular",
        451..=550 => ".medium",
        551..=650 => ".semibold",
        651..=750 => ".bold",
        751..=850 => ".heavy",
        _ => ".black",
    }
}

/// A Swift string literal.
fn swift_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars().take(10_000) {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' | '\u{2028}' => out.push_str("\\n"),
            '\r' => {}
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ------------------------------------------------------------------ Android XML

/// The layer tree as an Android layout (XML views).
pub fn android_xml(doc: &Document, id: NodeId) -> String {
    let Some(root) = doc.node(id) else { return String::new() };
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    xml_layer(doc, root, 0, &mut out, true);
    out
}

/// A layer as an Android view.
fn xml_layer(doc: &Document, n: &Node, depth: usize, out: &mut String, root: bool) {
    let pad = "    ".repeat(depth);
    let text = match &n.kind {
        NodeKind::Text(t) => Some(t.characters.clone()),
        _ => None,
    };
    let is_text = text.is_some();
    let tag = if is_text {
        "TextView"
    } else if n.layout.is_some() {
        "LinearLayout"
    } else {
        "FrameLayout"
    };
    out.push_str(&format!("{pad}<{tag}\n"));
    if root {
        out.push_str(&format!("{pad}    xmlns:android=\"http://schemas.android.com/apk/res/android\"\n"));
    }
    out.push_str(&format!("{pad}    android:layout_width=\"{}\"\n", xml_size(n.width, n.child_layout.sizing_h, root)));
    out.push_str(&format!("{pad}    android:layout_height=\"{}\"\n", xml_size(n.height, n.child_layout.sizing_v, root)));
    if let Some(l) = &n.layout {
        out.push_str(&format!("{pad}    android:orientation=\"{}\"\n", if l.mode == LayoutMode::Vertical { "vertical" } else { "horizontal" }));
        if l.gap > 0.0 {
            out.push_str(&format!("{pad}    android:divider=\"{}\"\n", num(l.gap)));
        }
    }
    // A text layer's fill is its text colour, not a background.
    if !is_text && let Some(c) = first_fill(n) {
        out.push_str(&format!("{pad}    android:background=\"{}\"\n", android_color(c)));
    }
    if let Some(c) = stroke_color(n) {
        out.push_str(&format!("{pad}    android:strokeWidth=\"{}\"\n", num(n.stroke.weight)));
        out.push_str(&format!("{pad}    android:strokeColor=\"{}\"\n", android_color(c)));
    }
    if let Some(t) = &text {
        out.push_str(&format!("{pad}    android:text=\"{}\"\n", xml_escape(t)));
        if let NodeKind::Text(tx) = &n.kind {
            out.push_str(&format!("{pad}    android:textSize=\"{}sp\"\n", num(tx.style.font_size)));
            out.push_str(&format!("{pad}    android:textStyle=\"{}\"\n", if tx.style.italic() { "italic" } else { "normal" }));
            if let Some(c) = first_fill(n) {
                out.push_str(&format!("{pad}    android:textColor=\"{}\"\n", android_color(c)));
            }
        }
    }
    let kids: Vec<NodeId> = n.children.iter().copied().filter(|c| doc.node(*c).is_some_and(|k| k.visible)).collect();
    if kids.is_empty() {
        out.push_str(&format!("{pad}    />\n"));
        return;
    }
    out.push_str(&format!("{pad}    >\n"));
    for k in kids {
        let Some(kn) = doc.node(k) else { continue };
        xml_layer(doc, kn, depth + 1, out, false);
    }
    out.push_str(&format!("{pad}</{tag}>\n"));
}

/// The Android size of a length: a fixed one in dp, a hugging one `wrap_content`, a filling one
/// `match_parent`. The root of a layout keeps its own size.
fn xml_size(v: f64, sizing: Sizing, root: bool) -> String {
    if root {
        return format!("{}dp", num(v));
    }
    match sizing {
        Sizing::Hug => "wrap_content".into(),
        Sizing::Fill => "match_parent".into(),
        Sizing::Fixed => format!("{}dp", num(v)),
    }
}

/// An Android colour: `#FF6B6B`.
fn android_color(c: Color) -> String {
    let [r, g, b, _] = c.to_rgba8();
    format!("#{r:02X}{g:02X}{b:02X}")
}

/// An XML-escaped text: a line break becomes a character reference.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\n', "&#10;")
}

// ------------------------------------------------------------------ Flutter

/// The layer tree as a Flutter widget tree.
pub fn flutter(doc: &Document, id: NodeId) -> String {
    let Some(root) = doc.node(id) else { return String::new() };
    let mut out = String::new();
    flutter_layer(doc, root, 0, &mut out);
    out
}

/// A layer as a Flutter widget.
fn flutter_layer(doc: &Document, n: &Node, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    match &n.kind {
        NodeKind::Text(t) => {
            out.push_str(&format!("{pad}Text(\n{pad}  '{}',\n{pad}  style: TextStyle(\n", dart_string(&t.characters)));
            out.push_str(&format!("{pad}    fontSize: {},\n", num(t.style.font_size)));
            if let Some(c) = first_fill(n) {
                out.push_str(&format!("{pad}    color: {},\n", flutter_color(c)));
            }
            out.push_str(&format!("{pad}  ),\n{pad})\n"));
        }
        _ => {
            out.push_str(&format!("{pad}Container(\n"));
            out.push_str(&format!("{pad}  width: {},\n", num(n.width)));
            out.push_str(&format!("{pad}  height: {},\n", num(n.height)));
            let mut decoration = Vec::new();
            if let Some(c) = first_fill(n) {
                decoration.push(format!("{pad}    color: {},", flutter_color(c)));
            }
            let radius = crate::codegen::radius(n);
            if radius > 0.0 {
                decoration.push(format!("{pad}    borderRadius: BorderRadius.circular({}),", num(radius)));
            }
            if let Some(c) = stroke_color(n) {
                decoration.push(format!("{pad}    border: Border.all(color: {}, width: {}),", flutter_color(c), num(n.stroke.weight)));
            }
            if !decoration.is_empty() {
                out.push_str(&format!("{pad}  decoration: BoxDecoration(\n{}\n{pad}  ),\n", decoration.join("\n")));
            }
            if let Some(l) = &n.layout {
                let [t, r, b, lft] = l.padding;
                if [t, r, b, lft].iter().any(|v| *v > 0.0) {
                    out.push_str(&format!(
                        "{pad}  padding: const EdgeInsets.only(left: {}, top: {}, right: {}, bottom: {}),\n",
                        num(lft),
                        num(t),
                        num(r),
                        num(b)
                    ));
                }
            }
            if n.opacity < 1.0 {
                out.push_str(&format!("{pad}  opacity: {},\n", num(f64::from(n.opacity))));
            }
            let kids: Vec<NodeId> = n.children.iter().copied().filter(|c| doc.node(*c).is_some_and(|k| k.visible)).collect();
            if kids.is_empty() {
                out.push_str(&format!("{pad})\n"));
                return;
            }
            out.push_str(&format!("{pad}  child: {}\n", if n.layout.is_some() { "Column(" } else { "Stack(" }));
            if let Some(l) = &n.layout {
                let main = match l.main_align {
                    MainAlign::Min => "MainAxisAlignment.start",
                    MainAlign::Center => "MainAxisAlignment.center",
                    MainAlign::Max => "MainAxisAlignment.end",
                    MainAlign::SpaceBetween => "MainAxisAlignment.spaceBetween",
                    MainAlign::SpaceAround => "MainAxisAlignment.spaceAround",
                    MainAlign::SpaceEvenly => "MainAxisAlignment.spaceEvenly",
                };
                out.push_str(&format!("{pad}    mainAxisAlignment: {main},\n"));
                let cross = match l.cross_align {
                    CrossAlign::Min => "CrossAxisAlignment.start",
                    CrossAlign::Center => "CrossAxisAlignment.center",
                    CrossAlign::Max => "CrossAxisAlignment.end",
                    CrossAlign::Baseline => "CrossAxisAlignment.baseline",
                };
                out.push_str(&format!("{pad}    crossAxisAlignment: {cross},\n"));
                if l.gap > 0.0 {
                    out.push_str(&format!("{pad}    spacing: {},\n", num(l.gap)));
                }
            }
            out.push_str(&format!("{pad}    children: [\n"));
            for k in kids {
                let Some(kn) = doc.node(k) else { continue };
                let mut child = String::new();
                flutter_layer(doc, kn, 0, &mut child);
                // The comma goes on the widget's last line.
                let mut lines: Vec<String> = child.lines().map(str::to_string).collect();
                if let Some(last) = lines.last_mut() {
                    last.push(',');
                }
                for l in &lines {
                    out.push_str(&format!("{pad}      {l}\n"));
                }
            }
            out.push_str(&format!("{pad}    ],\n{pad}  ),\n{pad})\n"));
        }
    }
}

/// A Flutter colour: `Color(0xFFFF6B6B)`.
fn flutter_color(c: Color) -> String {
    let [r, g, b, a] = c.to_rgba8();
    format!("Color(0x{a:02X}{r:02X}{g:02X}{b:02X})")
}

/// A Dart string literal.
fn dart_string(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().take(10_000) {
        match c {
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            '$' => out.push_str(r"\$"),
            '\n' => out.push_str(r"\n"),
            '\r' => {}
            '\t' => out.push_str(r"\t"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

// ------------------------------------------------------------------ shared

/// The first solid fill of a layer.
fn first_fill(n: &Node) -> Option<Color> {
    n.fills.iter().filter(|p| p.visible).find_map(|p| match &p.kind {
        PaintKind::Solid { color } => Some(color.with_alpha(color.a * p.opacity)),
        PaintKind::Gradient(g) => g.sorted_stops().first().map(|s| s.color),
        PaintKind::Image(_) | PaintKind::Pattern(_) => None,
    })
}

/// The colour of a layer's stroke.
fn stroke_color(n: &Node) -> Option<Color> {
    if !n.has_visible_stroke() {
        return None;
    }
    n.strokes.iter().filter(|p| p.visible).find_map(|p| p.solid_color())
}
