//! The pretty-printed XML: elements two spaces deeper per level, one attribute per line four
//! spaces past its element (as Android Studio formats drawables). Numbers are compact (at most
//! three decimals, no `-0`) and always finite; text is escaped for attribute values.

use std::fmt::Write as _;

use figcraft_color::Color;

use crate::model::{Fill, GradKind, GroupItem, Item, ItemKind, NativeStroke, PathItem};

const ANDROID: &str = "http://schemas.android.com/apk/res/android";
const AAPT: &str = "http://schemas.android.com/aapt";

// Shared wire primitives; app-specific drawable projection and colour policy stay here.
pub(crate) use craft_vectordrawable::write::{esc, num, path_data};

/// `#RRGGBB`, or `#AARRGGBB` when not opaque.
pub(crate) fn color(c: Color) -> String {
    let [r, g, b, a] = c.sanitized().to_rgba8();
    if a == 255 { format!("#{r:02X}{g:02X}{b:02X}") } else { format!("#{a:02X}{r:02X}{g:02X}{b:02X}") }
}

struct Out {
    s: String,
}

impl Out {
    /// `<tag` and its attributes, one per line; `close` ends it with `/>`, else `>`.
    fn open(&mut self, depth: usize, tag: &str, attrs: &[(&str, String)], close: bool) {
        let pad = "  ".repeat(depth);
        let _ = write!(self.s, "{pad}<{tag}");
        for (i, (k, v)) in attrs.iter().enumerate() {
            if i == 0 && attrs.len() == 1 {
                let _ = write!(self.s, " {k}=\"{v}\"");
            } else {
                let _ = write!(self.s, "\n{pad}    {k}=\"{v}\"");
            }
        }
        self.s.push_str(if close { " />\n" } else { ">\n" });
    }

    fn close(&mut self, depth: usize, tag: &str) {
        let _ = writeln!(self.s, "{}</{tag}>", "  ".repeat(depth));
    }
}

fn uses_gradient(items: &[Item]) -> bool {
    items.iter().any(|it| match &it.kind {
        ItemKind::Path(p) => matches!(p.fill, Some(Fill::Gradient(_))),
        ItemKind::Group(g) => uses_gradient(&g.children),
    })
}

/// The whole file: a `width`×`height` dp drawable with a viewport of the same size.
pub(crate) fn document(width: f64, height: f64, items: &[Item]) -> String {
    let mut o = Out { s: String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n") };
    let mut attrs = vec![("xmlns:android", ANDROID.to_string())];
    if uses_gradient(items) {
        attrs.push(("xmlns:aapt", AAPT.to_string()));
    }
    attrs.extend([
        ("android:width", format!("{}dp", num(width))),
        ("android:height", format!("{}dp", num(height))),
        ("android:viewportWidth", num(width)),
        ("android:viewportHeight", num(height)),
    ]);
    o.open(0, "vector", &attrs, false);
    if items.is_empty() {
        // Android refuses a <vector> without a <path> ("no path defined"): one empty, unfilled path.
        o.open(1, "path", &[("android:pathData", "M0,0".to_string())], true);
    }
    for it in items {
        item(&mut o, 1, it);
    }
    o.close(0, "vector");
    o.s
}

fn item(o: &mut Out, depth: usize, it: &Item) {
    match &it.kind {
        ItemKind::Path(p) => path(o, depth, p),
        ItemKind::Group(g) => group(o, depth, g),
    }
}

fn group(o: &mut Out, depth: usize, g: &GroupItem) {
    let mut attrs = Vec::new();
    if let Some(n) = &g.name {
        attrs.push(("android:name", esc(n)));
    }
    if let Some(x) = &g.xf {
        let (sx, sy) = x.scale;
        if num(x.rotation) != "0" {
            attrs.push(("android:rotation", num(x.rotation)));
        }
        if num(sx) != "1" {
            attrs.push(("android:scaleX", num(sx)));
        }
        if num(sy) != "1" {
            attrs.push(("android:scaleY", num(sy)));
        }
        if num(x.translate.0) != "0" {
            attrs.push(("android:translateX", num(x.translate.0)));
        }
        if num(x.translate.1) != "0" {
            attrs.push(("android:translateY", num(x.translate.1)));
        }
    }
    if g.children.is_empty() && g.clip.is_none() {
        o.open(depth, "group", &attrs, true);
        return;
    }
    o.open(depth, "group", &attrs, false);
    if let Some(c) = &g.clip {
        o.open(depth + 1, "clip-path", &[("android:pathData", path_data(c))], true);
    }
    for c in &g.children {
        item(o, depth + 1, c);
    }
    o.close(depth, "group");
}

fn stroke_attrs(attrs: &mut Vec<(&str, String)>, s: &NativeStroke) {
    attrs.push(("android:strokeColor", color(s.color)));
    attrs.push(("android:strokeWidth", num(s.width)));
    if s.cap != "butt" {
        attrs.push(("android:strokeLineCap", s.cap.to_string()));
    }
    if s.join != "miter" {
        attrs.push(("android:strokeLineJoin", s.join.to_string()));
    } else if num(s.miter) != "4" {
        attrs.push(("android:strokeMiterLimit", num(s.miter)));
    }
}

fn path(o: &mut Out, depth: usize, p: &PathItem) {
    let mut attrs = Vec::new();
    if let Some(n) = &p.name {
        attrs.push(("android:name", esc(n)));
    }
    attrs.push(("android:pathData", path_data(&p.data)));
    if let Some(Fill::Solid(c)) = &p.fill {
        attrs.push(("android:fillColor", color(*c)));
    }
    if let Some(s) = &p.stroke {
        stroke_attrs(&mut attrs, s);
    }
    if p.even_odd && p.fill.is_some() {
        attrs.push(("android:fillType", "evenOdd".to_string()));
    }
    let Some(Fill::Gradient(g)) = &p.fill else {
        o.open(depth, "path", &attrs, true);
        return;
    };
    o.open(depth, "path", &attrs, false);
    o.open(depth + 1, "aapt:attr", &[("name", "android:fillColor".to_string())], false);
    let mut ga = Vec::new();
    match g.kind {
        GradKind::Linear { start, end } => {
            ga.push(("android:type", "linear".to_string()));
            ga.push(("android:startX", num(start.0)));
            ga.push(("android:startY", num(start.1)));
            ga.push(("android:endX", num(end.0)));
            ga.push(("android:endY", num(end.1)));
        }
        GradKind::Radial { center, radius } => {
            ga.push(("android:type", "radial".to_string()));
            ga.push(("android:centerX", num(center.0)));
            ga.push(("android:centerY", num(center.1)));
            ga.push(("android:gradientRadius", num(radius)));
        }
        GradKind::Sweep { center } => {
            ga.push(("android:type", "sweep".to_string()));
            ga.push(("android:centerX", num(center.0)));
            ga.push(("android:centerY", num(center.1)));
        }
    }
    o.open(depth + 2, "gradient", &ga, false);
    for (off, c) in &g.stops {
        o.open(depth + 3, "item", &[("android:offset", num(*off)), ("android:color", color(*c))], true);
    }
    o.close(depth + 2, "gradient");
    o.close(depth + 1, "aapt:attr");
    o.close(depth, "path");
}

#[cfg(test)]
mod shared_wire_tests {
    use super::*;
    use figcraft_geom::BezPath;

    #[test]
    fn shared_attributes_preserve_exact_xml() {
        assert_eq!(esc("<&>\"'\t\n\r\0\u{ffff}é𝄞&#9;"), "&lt;&amp;&gt;&quot;&apos;&#9;&#10;&#13;é𝄞&amp;#9;");
    }

    #[test]
    fn shared_numbers_and_paths_preserve_android_syntax() {
        assert_eq!(num(-0.0001), "0");
        assert_eq!(num(f64::NAN), "0");
        assert_eq!(num(f64::INFINITY), "0");
        assert_eq!(num(1.23456), "1.235");
        let mut p = BezPath::new();
        p.move_to((0.0, -0.0));
        p.line_to((1.23456, 2.0));
        p.quad_to((3.0, 4.0), (5.0, 6.0));
        p.curve_to((7.0, 8.0), (9.0, 10.0), (11.0, 12.0));
        p.close_path();
        assert_eq!(path_data(&p), "M0,0 L1.235,2 Q3,4 5,6 C7,8 9,10 11,12 Z");
    }
}
