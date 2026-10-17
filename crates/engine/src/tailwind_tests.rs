//! React + Tailwind code for pages: utilities instead of a stylesheet, theme utilities for bound
//! variables, fluid boxes, inlined instances, per-side borders, mobile-first responsive merges and
//! the Tailwind theme export.

use crate::Session;
use crate::components::tests::{ex, idv};
use crate::tailwind::classes;
use figcraft_doc::*;
use serde_json::json;

fn code(s: &mut Session, id: NodeId, extra: serde_json::Value) -> String {
    let mut p = json!({"id": id, "language": "tailwind"});
    if let (Some(p), Some(e)) = (p.as_object_mut(), extra.as_object()) {
        p.extend(e.clone());
    }
    ex(s, "inspect.code", p)["code"].as_str().unwrap_or_default().to_string()
}

/// A 1440 "Page" (vertical auto layout, bg bound to Brand/Accent) holding a 960 "Hero" text and a
/// horizontal "Row" of two "Card" frames that fill it. Returns (page, row).
fn page(s: &mut Session, name: &str, width: f64, row_dir: &str) -> (NodeId, NodeId) {
    let p = idv(&ex(s, "frame.create", json!({"x": 0, "y": 0, "width": width, "height": 900, "name": name})));
    ex(s, "layout.add", json!({"ids": [p], "direction": "vertical"}));
    ex(s, "node.set", json!({"id": p, "props": {"gap": 24, "padding": 32}}));
    let t = idv(&ex(s, "text.create", json!({"text": "Seu site em 7 dias", "x": 0, "y": 0, "parent": p, "props": {"name": "Hero"}})));
    ex(s, "node.set", json!({"id": t, "props": {"width": 960, "textAutoResize": "height"}}));
    let row = idv(&ex(s, "frame.create", json!({"x": 0, "y": 0, "width": 800, "height": 200, "parent": p, "name": "Row"})));
    ex(s, "layout.add", json!({"ids": [row], "direction": row_dir}));
    for _ in 0..2 {
        let c = idv(&ex(s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 100, "parent": row, "name": "Card"})));
        ex(s, "node.set", json!({"id": c, "props": {"sizingH": "fill"}}));
    }
    (p, row)
}

fn brand(s: &mut Session) {
    ex(s, "variable.createCollection", json!({"name": "Brand"}));
    ex(s, "variable.create", json!({"collection": "Brand", "name": "Brand/Accent", "type": "color", "value": "#3DDC97"}));
    ex(s, "variable.create", json!({"collection": "Brand", "name": "card", "type": "number", "value": 12}));
    ex(s, "variable.create", json!({"collection": "Brand", "name": "fade", "type": "number", "value": 0.5}));
}

#[test]
fn utilities_replace_the_stylesheet_and_variables_become_theme_utilities() {
    let mut s = Session::new();
    brand(&mut s);
    let (p, row) = page(&mut s, "Landing", 1440.0, "horizontal");
    ex(&mut s, "variable.bind", json!({"ids": [p], "field": "fill", "variable": "Brand/Accent"}));
    ex(&mut s, "node.set", json!({"id": row, "props": {"cornerRadius": 12}}));
    ex(&mut s, "variable.bind", json!({"ids": [row], "field": "cornerRadius", "variable": "card"}));
    let out = code(&mut s, p, json!({}));
    assert!(out.contains("export function Landing() {"), "{out}");
    assert!(!out.contains("import") && !out.contains("<style") && !out.contains(".css"), "no stylesheet: {out}");
    for want in ["flex flex-col", "gap-6", "p-8", "bg-accent", "rounded-card", "flex-1", "w-full"] {
        assert!(out.contains(want), "{want} in {out}");
    }
    // The page is as wide as itself: just full width; the 960 text is capped, not fixed.
    assert!(out.contains("max-w-[960px]") && !out.contains("w-[1440px]") && !out.contains("max-w-[1440px]"), "{out}");
}

#[test]
fn per_side_strokes_are_borders_on_those_sides() {
    let mut s = Session::new();
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 300, "height": 60, "name": "Nav"})));
    ex(&mut s, "node.set", json!({"id": f, "props": {"strokes": ["#262A31"], "strokeWeight": 1, "strokeSides": [0, 0, 1, 0]}}));
    let css = ex(&mut s, "inspect.code", json!({"id": f, "language": "css"}))["code"].as_str().unwrap_or_default().to_string();
    assert!(css.contains("border-bottom: 1px solid") && !css.contains("border:"), "{css}");
    let tw = code(&mut s, f, json!({}));
    assert!(tw.contains("border-b ") && tw.contains("border-[#262a31]") || tw.contains("border-[#262A31]"), "{tw}");
    // Uniform sides stay one border.
    ex(&mut s, "node.set", json!({"id": f, "props": {"strokeSides": [2, 2, 2, 2]}}));
    let css = ex(&mut s, "inspect.code", json!({"id": f, "language": "css"}))["code"].as_str().unwrap_or_default().to_string();
    assert!(css.contains("border: 2px solid"), "{css}");
}

#[test]
fn instances_are_inlined_with_their_overrides() {
    let mut s = Session::new();
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 200, "height": 100, "name": "Benefit"})));
    ex(&mut s, "layout.add", json!({"ids": [f], "direction": "vertical"}));
    ex(&mut s, "text.create", json!({"text": "Original", "x": 0, "y": 0, "parent": f, "props": {"name": "Title"}}));
    let comp = idv(&ex(&mut s, "component.create", json!({"ids": [f]})));
    let holder = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 400, "width": 600, "height": 200, "name": "Benefits"})));
    ex(&mut s, "layout.add", json!({"ids": [holder], "direction": "horizontal"}));
    let inst = idv(&ex(&mut s, "instance.create", json!({"component": comp, "x": 0, "y": 0, "parent": holder})));
    let doc = s.doc().unwrap();
    let title = doc.node(inst).unwrap().children[0];
    ex(&mut s, "text.set", json!({"id": title, "characters": "Agenda sozinha"}));
    let out = code(&mut s, holder, json!({}));
    assert!(out.contains("Agenda sozinha") && !out.contains("<Benefit"), "inlined with override: {out}");
}

#[test]
fn a_mobile_frame_makes_the_code_mobile_first() {
    let mut s = Session::new();
    let (desk, _) = page(&mut s, "Landing", 1440.0, "horizontal");
    let (mob, _) = page(&mut s, "Landing mobile", 390.0, "vertical");
    // A layer only on desktop is hidden on mobile.
    ex(&mut s, "text.create", json!({"text": "Só no desktop", "x": 0, "y": 0, "parent": desk, "props": {"name": "Aside"}}));
    let out = code(&mut s, desk, json!({"mobile": mob}));
    assert!(out.contains("mobile first"), "{out}");
    assert!(out.contains("flex flex-col") && out.contains("lg:flex-row"), "row stacks on mobile: {out}");
    assert!(out.contains("hidden") && out.contains("lg:block"), "desktop-only layer: {out}");
    assert!(s.execute("inspect.code", &json!({"id": desk, "language": "tailwind", "mobile": 999_999})).is_err(), "a missing mobile frame is refused");
}

#[test]
fn declarations_map_to_idiomatic_utilities() {
    let got: Vec<String> = classes(
        "display: flex;\njustify-content: space-between;\nalign-items: center;\npadding: 8px 16px 8px 16px;\nfont-weight: 600;\nfont-size: 18px;\nline-height: 120%;\nopacity: 0.5;\ncolor: var(--color-muted, #9aa3ae);\nbox-shadow: 0px 4px 12px 0px rgba(0, 0, 0, 0.25);\nwhite-space: nowrap;\n",
    )
    .into_iter()
    .map(|(_, c)| c)
    .collect();
    let want = [
        "flex",
        "justify-between",
        "items-center",
        "py-2",
        "px-4",
        "font-semibold",
        "text-[18px]",
        "leading-[1.2]",
        "opacity-50",
        "text-muted",
        "shadow-[0px_4px_12px_0px_rgba(0,_0,_0,_0.25)]",
        "[white-space:nowrap]",
    ];
    assert_eq!(got, want);
}

#[test]
fn variables_export_with_units_and_as_a_tailwind_theme() {
    let mut s = Session::new();
    brand(&mut s);
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 100})));
    ex(&mut s, "node.set", json!({"id": f, "props": {"cornerRadius": 12, "opacity": 0.5}}));
    ex(&mut s, "variable.bind", json!({"ids": [f], "field": "cornerRadius", "variable": "card"}));
    ex(&mut s, "variable.bind", json!({"ids": [f], "field": "opacity", "variable": "fade"}));
    let css = ex(&mut s, "variable.export", json!({"format": "css"}))["text"].as_str().unwrap_or_default().to_string();
    assert!(css.contains("--card: 12px;") && css.contains("--fade: 0.5;"), "{css}");
    let tw = ex(&mut s, "variable.export", json!({"format": "tailwind"}))["text"].as_str().unwrap_or_default().to_string();
    for want in ["@import \"tailwindcss\";", "@theme {", "--color-accent: #3ddc97;", "--radius-card: 12px;", "--fade: 0.5;"] {
        assert!(tw.to_lowercase().contains(&want.to_lowercase()), "{want} in {tw}");
    }
}

#[test]
fn desktop_padding_on_other_sides_is_not_reset_away() {
    let mut s = Session::new();
    let (desk, _) = page(&mut s, "Landing", 1440.0, "horizontal");
    let (mob, _) = page(&mut s, "Landing mobile", 390.0, "vertical");
    ex(&mut s, "node.set", json!({"id": desk, "props": {"paddingTop": 24, "paddingBottom": 24, "paddingLeft": 64, "paddingRight": 64}}));
    ex(&mut s, "node.set", json!({"id": mob, "props": {"padding": 24}}));
    let out = code(&mut s, desk, json!({"mobile": mob}));
    assert!(out.contains("p-6 ") && out.contains("lg:py-6 lg:px-16"), "{out}");
    assert!(!out.contains("lg:p-0"), "a reset would undo lg:px-16: {out}");
}

#[test]
fn a_layer_name_cant_break_out_of_the_header_comment() {
    let mut s = Session::new();
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 100, "name": "A\rB\u{2028}alert(1)\u{2029}C\nD"})));
    let out = code(&mut s, f, json!({}));
    let header = out.lines().next().unwrap_or_default();
    assert!(header.starts_with("// ") && header.contains("alert(1)"), "{out}");
    assert!(!out.contains(['\r', '\u{2028}', '\u{2029}']), "{out:?}");
    let name = out.lines().nth(1).and_then(|l| l.strip_prefix("export function ")).and_then(|l| l.strip_suffix("() {")).unwrap_or_default();
    assert!(!name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'), "an identifier: {name:?}");
}

#[test]
fn slashed_layer_names_and_phone_widths_merge_right() {
    let mut s = Session::new();
    let mut frames = Vec::new();
    for (name, width, dir, sizing) in [("Landing", 1440.0, "horizontal", "hug"), ("Landing mobile", 390.0, "vertical", "fill")] {
        let p = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": width, "height": 600, "name": name})));
        ex(&mut s, "layout.add", json!({"ids": [p], "direction": "vertical"}));
        let ctas = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 342, "height": 120, "parent": p, "name": "CTAs"})));
        ex(&mut s, "layout.add", json!({"ids": [ctas], "direction": dir}));
        let b = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 200, "height": 48, "parent": ctas, "name": "Button/Primary"})));
        ex(&mut s, "layout.add", json!({"ids": [b], "direction": "horizontal"}));
        ex(&mut s, "node.set", json!({"id": b, "props": {"sizingH": sizing}}));
        ex(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": width * 0.8, "height": 1, "parent": p, "props": {"name": "Divider"}}));
        frames.push(p);
    }
    // A layer only the mobile frame has.
    ex(&mut s, "text.create", json!({"text": "7 dias", "x": 0, "y": 0, "parent": frames[1], "props": {"name": "Tag"}}));
    let out = code(&mut s, frames[0], json!({"mobile": frames[1]}));
    let button =
        out.lines().find(|l| l.contains("lg:") && l.contains("rounded") || l.contains("self-stretch") && l.contains("flex-row")).unwrap_or_default();
    assert!(button.contains("self-stretch") && button.contains("lg:self-auto"), "the slashed name keeps its mobile classes: {out}");
    assert!(!out.contains("max-w-[342px] ") || out.contains("lg:max-w-[342px]"), "no phone-width cap below lg: {out}");
    assert!(out.contains("Only in the mobile frame") && out.contains("Tag"), "{out}");
    let divider = out.lines().filter(|l| l.contains("<div") && l.contains("h-[1px]") || l.contains("h-px")).count();
    assert!(out.contains("w-full") && divider <= 2, "{out}");
}

#[test]
fn a_variable_named_after_its_namespace_is_its_base() {
    let mut s = Session::new();
    ex(&mut s, "variable.createCollection", json!({"name": "Brand"}));
    ex(&mut s, "variable.create", json!({"collection": "Brand", "name": "radius", "type": "number", "value": 12}));
    let f = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 100, "height": 100})));
    ex(&mut s, "node.set", json!({"id": f, "props": {"cornerRadius": 12}}));
    ex(&mut s, "variable.bind", json!({"ids": [f], "field": "cornerRadius", "variable": "radius"}));
    assert!(code(&mut s, f, json!({})).contains("rounded-base"));
    let tw = ex(&mut s, "variable.export", json!({"format": "tailwind"}))["text"].as_str().unwrap_or_default().to_string();
    assert!(tw.contains("--radius-base: 12px;") && !tw.contains("--radius-radius"), "{tw}");
}

#[test]
fn mobile_only_layer_names_cant_break_out_of_the_header_comment() {
    let mut s = Session::new();
    let (desk, _) = page(&mut s, "Landing", 1440.0, "horizontal");
    let (mob, _) = page(&mut s, "Landing mobile", 390.0, "vertical");
    ex(&mut s, "text.create", json!({"text": "x", "x": 0, "y": 0, "parent": mob, "props": {"name": "Tag\nalert(1)\u{2028}y"}}));
    let out = code(&mut s, desk, json!({"mobile": mob}));
    let header = out.lines().next().unwrap_or_default();
    assert!(header.starts_with("// ") && header.contains("alert(1)"), "listed inside the comment: {out}");
    assert!(out.lines().nth(1).is_some_and(|l| l.starts_with("export function ")), "{out}");
    assert!(!out.contains(['\r', '\u{2028}', '\u{2029}']), "{out:?}");
}

#[test]
fn page_code_guesses_the_h1_landmarks_and_buttons_nothing_marked() {
    let mut s = Session::new();
    let p = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 1440, "height": 900, "name": "Landing"})));
    ex(&mut s, "layout.add", json!({"ids": [p], "direction": "vertical"}));
    let nav = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 1440, "height": 64, "parent": p, "name": "Nav"})));
    ex(&mut s, "text.create", json!({"text": "LookADev", "x": 0, "y": 0, "parent": nav, "props": {"name": "Logo", "fontSize": 22}}));
    ex(&mut s, "text.create", json!({"text": "Seu site em 7 dias", "x": 0, "y": 0, "parent": p, "props": {"name": "Hero copy", "fontSize": 60}}));
    ex(&mut s, "text.create", json!({"text": "Investimento", "x": 0, "y": 0, "parent": p, "props": {"name": "Pricing title", "fontSize": 32}}));
    let cta = idv(&ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 200, "height": 48, "parent": p, "name": "CTA Primary"})));
    ex(&mut s, "text.create", json!({"text": "Agendar", "x": 0, "y": 0, "parent": cta, "props": {"name": "Label", "fontSize": 16}}));
    ex(&mut s, "frame.create", json!({"x": 0, "y": 0, "width": 1440, "height": 80, "parent": p, "name": "Footer"}));
    let out = code(&mut s, p, json!({}));
    assert!(out.contains("<nav className="), "{out}");
    assert!(out.contains("<h1 className=") && out.contains(">Seu site em 7 dias</h1>"), "{out}");
    assert!(out.contains("<h2 className=") && out.contains(">Investimento</h2>"), "{out}");
    assert!(out.contains("<button className=") && out.contains("type=\"button\"") && out.contains(">Agendar</span>"), "{out}");
    assert!(out.contains("<footer className="), "{out}");
    assert_eq!(out.matches("<h1").count(), 1);
    // A heading level 1 annotation elsewhere wins: no second h1 is guessed.
    let logo = s.doc().unwrap().node(nav).unwrap().children[0];
    ex(&mut s, "annotation.add", json!({"id": logo, "label": "h1", "category": "accessibility"}));
    let out = code(&mut s, p, json!({}));
    assert_eq!(out.matches("<h1").count(), 1, "{out}");
    assert!(out.contains(">LookADev</h1>"), "{out}");
    // Plain React code guesses nothing.
    let react = ex(&mut s, "inspect.code", json!({"id": p, "language": "react"}))["code"].as_str().unwrap_or_default().to_string();
    assert!(!react.contains("<nav") && !react.contains("<footer"), "{react}");
}

#[test]
fn page_code_leaves_resets_to_tailwind_preflight() {
    let mut s = Session::new();
    let (p, _) = page(&mut s, "Landing", 1440.0, "horizontal");
    let out = code(&mut s, p, json!({}));
    for leak in ["box-sizing", "[margin:0]", "text-decoration", "cursor"] {
        assert!(!out.contains(leak), "{leak} in {out}");
    }
}
