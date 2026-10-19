//! Lengths in `rem` as well as pixels. Layers keep pixel values (drawing, layout and export read
//! those); a field the user set in `rem` also keeps that amount (`Node::units`), so the panels show
//! it as typed ("2rem") and it follows the size of 1rem (Preferences, [`crate::Prefs::rem_px`]).
//!
//! Commands take a length as a number of pixels, `"24px"` or `"1.5rem"` (`node.set` and the keys
//! [`LENGTH_FIELDS`] lists); a number or `px` value drops the field's `rem`.

use std::cell::Cell;
use std::collections::BTreeSet;

use figcraft_doc::{Document, NodeId};
use serde_json::{Map, Value, json};

/// The size of 1rem in pixels unless Preferences say otherwise.
pub const DEFAULT_REM_PX: f64 = 16.0;
/// Bounds of 1rem in pixels.
pub const REM_PX_RANGE: (f64, f64) = (1.0, 1000.0);

/// The fields that take lengths in `rem`: sizes and their limits, gaps, padding, corner radii and
/// text sizes and spacing.
pub const LENGTH_FIELDS: &[&str] = &[
    "width",
    "height",
    "minWidth",
    "maxWidth",
    "minHeight",
    "maxHeight",
    "gap",
    "columnGap",
    "rowGap",
    "crossGap",
    "padding",
    "paddingHorizontal",
    "paddingVertical",
    "paddingTop",
    "paddingRight",
    "paddingBottom",
    "paddingLeft",
    "cornerRadius",
    "topLeftRadius",
    "topRightRadius",
    "bottomRightRadius",
    "bottomLeftRadius",
    "fontSize",
    "paragraphSpacing",
    "paragraphIndent",
];

/// Whether `field` takes lengths in `rem`.
pub fn is_length(field: &str) -> bool {
    LENGTH_FIELDS.contains(&field)
}

thread_local! {
    static REM_PX: Cell<f64> = const { Cell::new(DEFAULT_REM_PX) };
}

/// The size of 1rem the commands running on this thread use (the session sets it from its
/// preferences before each command).
pub fn rem_px() -> f64 {
    REM_PX.with(Cell::get)
}

pub(crate) fn set_rem_px(px: f64) {
    REM_PX.with(|c| c.set(clamp_rem(px)));
}

/// `px` kept within [`REM_PX_RANGE`] (16 for anything that isn't a number).
pub fn clamp_rem(px: f64) -> f64 {
    if px.is_finite() { px.clamp(REM_PX_RANGE.0, REM_PX_RANGE.1) } else { DEFAULT_REM_PX }
}

/// A length as typed: pixels, or an amount of rem.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Length {
    Px(f64),
    Rem(f64),
}

/// A length from `"1.5rem"`, `"24px"` or `"24"` (`None`: not a length).
pub fn parse_length(s: &str) -> Option<Length> {
    let s = s.trim();
    let (num, rem) = match s.strip_suffix("rem") {
        Some(n) => (n, true),
        None => (s.strip_suffix("px").unwrap_or(s), false),
    };
    let n: f64 = num.trim().parse().ok().filter(|x: &f64| x.is_finite() && x.abs() <= 1e7)?;
    Some(if rem { Length::Rem(n) } else { Length::Px(n) })
}

/// Rewrite the length fields of `props` for node `id` into pixels, keeping (or dropping) each
/// field's `rem` amount on the node. Other keys pass through.
pub fn resolve_props(doc: &mut Document, id: NodeId, props: &Map<String, Value>) -> Map<String, Value> {
    let base = rem_px();
    let mut out = Map::new();
    let mut set: Vec<(String, Option<f64>)> = Vec::new();
    for (k, v) in props {
        if !is_length(k) {
            out.insert(k.clone(), v.clone());
            continue;
        }
        match v {
            Value::String(s) => match parse_length(s) {
                Some(Length::Rem(r)) => {
                    out.insert(k.clone(), json!(r * base));
                    set.push((k.clone(), Some(r)));
                }
                Some(Length::Px(p)) => {
                    out.insert(k.clone(), json!(p));
                    set.push((k.clone(), None));
                }
                // Not a length (e.g. "auto"): the field's own parser decides.
                None => {
                    out.insert(k.clone(), v.clone());
                }
            },
            Value::Number(_) => {
                out.insert(k.clone(), v.clone());
                set.push((k.clone(), None));
            }
            _ => {
                out.insert(k.clone(), v.clone());
                set.push((k.clone(), None));
            }
        }
    }
    let changes = set.iter().any(|(k, r)| doc.node(id).is_some_and(|n| n.units.get(k).copied() != *r));
    if changes && let Ok(n) = doc.get_mut(id) {
        for (k, r) in set {
            match r {
                Some(r) => n.units.insert(k, r),
                None => n.units.remove(&k),
            };
        }
    }
    out
}

/// The `rem` amount `field` of `n` shows: its own while its pixels still match it at `base` (an
/// edit in pixels elsewhere, e.g. a resize, makes it pixels again).
pub fn rem_of(units: &std::collections::BTreeMap<String, f64>, field: &str, px: f64, base: f64) -> Option<f64> {
    let r = *units.get(field)?;
    ((r * base - px).abs() < 0.01).then_some(r)
}

/// Re-apply every `rem` length of `doc` at 1rem = `base` px (Preferences changed it): the layers
/// whose pixels still match their rem at `old` get the new pixels. Returns the layers changed.
pub fn rebase(doc: &mut Document, old: f64, base: f64) -> BTreeSet<NodeId> {
    let mut changed = BTreeSet::new();
    let ids: Vec<NodeId> = doc.nodes().filter(|n| !n.units.is_empty()).map(|n| n.id).collect();
    let prev = rem_px();
    set_rem_px(base);
    for id in ids {
        let Some(n) = doc.node(id) else { continue };
        let mut props = Map::new();
        for (k, r) in &n.units {
            // Only fields still showing their rem (their pixels unchanged since).
            if current_px(n, k).is_some_and(|px| (r * old - px).abs() < 0.01) {
                props.insert(k.clone(), json!(format!("{r}rem")));
            }
        }
        if props.is_empty() {
            continue;
        }
        if crate::cmd::props::apply("units", doc, id, &props).is_ok() {
            changed.insert(id);
        }
    }
    set_rem_px(prev);
    changed
}

/// The pixels length field `k` of `n` has now (`None`: not a field it has, or mixed text).
pub fn current_px(n: &figcraft_doc::Node, k: &str) -> Option<f64> {
    use figcraft_doc::NodeKind;
    let layout = n.layout.as_ref();
    let text = match &n.kind {
        NodeKind::Text(t) => Some(t),
        _ => None,
    };
    let [pt, pr, pb, pl] = layout.map(|l| l.padding).unwrap_or_default();
    let r = n.corners.radii;
    Some(match k {
        "width" => n.width,
        "height" => n.height,
        "minWidth" => n.child_layout.min_w?,
        "maxWidth" => n.child_layout.max_w?,
        "minHeight" => n.child_layout.min_h?,
        "maxHeight" => n.child_layout.max_h?,
        "gap" | "columnGap" => layout?.gap,
        "rowGap" | "crossGap" => layout?.cross_gap,
        "padding" | "paddingTop" => pt,
        "paddingRight" => pr,
        "paddingBottom" => pb,
        "paddingLeft" | "paddingHorizontal" => pl,
        "paddingVertical" => pt,
        "cornerRadius" | "topLeftRadius" => r[0],
        "topRightRadius" => r[1],
        "bottomRightRadius" => r[2],
        "bottomLeftRadius" => r[3],
        "fontSize" => text?.style.font_size,
        "paragraphSpacing" => text?.paragraph_spacing,
        "paragraphIndent" => text?.paragraph_indent,
        _ => return None,
    })
}
