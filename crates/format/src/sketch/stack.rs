//! Sketch 2025+ stacks (`groupLayout` `MSImmutableFlexGroupLayout`) → REST auto layout.
//!
//! The enums follow the order Sketch's plugin API documents them in (checked against files):
//! direction row, column; justify start, center, end, between, around, evenly; align items start,
//! center, end, stretch, none; stacking order first on top, last on top. Sizing (`horizontalSizing`,
//! `verticalSizing`) is fixed, fit (hug), fill, relative. Sketch lists a stack's first item last
//! (on top in the layer list), so the flow order is the reverse of its `layers`.

use serde_json::{Map as JsonMap, Value, json};

/// Whether `l` is a stack.
pub(crate) fn is_stack(l: &Value) -> bool {
    l.get("groupLayout").and_then(|g| g.get("_class")).and_then(Value::as_str) == Some("MSImmutableFlexGroupLayout")
}

/// Make `node` (the REST node of frame `l`, its children already built in Sketch's order) an auto
/// layout frame when `l` is a stack.
pub(crate) fn apply(l: &Value, node: &mut JsonMap<String, Value>) {
    if !is_stack(l) || !matches!(node.get("type").and_then(Value::as_str), Some("FRAME" | "COMPONENT" | "INSTANCE")) {
        return;
    }
    let Some(g) = l.get("groupLayout") else { return };
    let int = |k: &str| g.get(k).and_then(Value::as_i64).unwrap_or(0);
    let num = |v: &Value, k: &str| v.get(k).and_then(Value::as_f64).filter(|x| x.is_finite()).unwrap_or(0.0).clamp(0.0, 1e5);
    let row = int("flexDirection") == 0;
    node.insert("layoutMode".into(), json!(if row { "HORIZONTAL" } else { "VERTICAL" }));
    node.insert("itemSpacing".into(), json!(num(g, "allGuttersGap")));
    node.insert(
        "primaryAxisAlignItems".into(),
        json!(match int("justifyContent") {
            1 => "CENTER",
            2 => "MAX",
            3 => "SPACE_BETWEEN",
            4 => "SPACE_AROUND",
            5 => "SPACE_EVENLY",
            _ => "MIN",
        }),
    );
    let stretch = int("alignItems") == 3;
    node.insert(
        "counterAxisAlignItems".into(),
        json!(match int("alignItems") {
            1 => "CENTER",
            2 => "MAX",
            _ => "MIN",
        }),
    );
    if g.get("wrappingEnabled").and_then(Value::as_bool) == Some(true) {
        node.insert("layoutWrap".into(), json!("WRAP"));
        node.insert("counterAxisSpacing".into(), json!(num(g, "crossAxisGutterGap")));
    }
    for (k, rest) in
        [("leftPadding", "paddingLeft"), ("topPadding", "paddingTop"), ("rightPadding", "paddingRight"), ("bottomPadding", "paddingBottom")]
    {
        node.insert(rest.into(), json!(num(l, k)));
    }
    node.insert("itemReverseZIndex".into(), json!(int("stackingOrder") == 0));
    node.insert("strokesIncludedInLayout".into(), json!(g.get("bordersAffectLayout").and_then(Value::as_bool) == Some(true)));
    // The stack's own sizing: fit hugs its content.
    let (main, cross) = if row { ("horizontalSizing", "verticalSizing") } else { ("verticalSizing", "horizontalSizing") };
    let mode = |k: &str| if l.get(k).and_then(Value::as_i64) == Some(1) { "AUTO" } else { "FIXED" };
    node.insert("primaryAxisSizingMode".into(), json!(mode(main)));
    node.insert("counterAxisSizingMode".into(), json!(mode(cross)));
    // Items: flow order, sizing, absolute ones.
    // Source layers by id (the first of a repeated id).
    let mut sources: std::collections::BTreeMap<&str, &Value> = std::collections::BTreeMap::new();
    for src in super::tree::layers(l) {
        if let Some(id) = src.get("do_objectID").and_then(Value::as_str) {
            sources.entry(id).or_insert(src);
        }
    }
    if let Some(kids) = node.get_mut("children").and_then(Value::as_array_mut) {
        // Children were built one per source layer, skipped ones aside: match them by id.
        for kid in kids.iter_mut() {
            let Some(src) = kid.get("id").and_then(Value::as_str).and_then(|id| sources.get(id).copied()) else { continue };
            let Some(m) = kid.as_object_mut() else { continue };
            item(src, m, row, stretch);
        }
        kids.reverse();
    }
}

/// An item's place in its stack: absolute, or sized fixed, hugging or filling.
fn item(src: &Value, m: &mut JsonMap<String, Value>, row: bool, stretch: bool) {
    if src.get("flexItem").and_then(|f| f.get("ignoreLayout")).and_then(Value::as_bool) == Some(true) {
        m.insert("layoutPositioning".into(), json!("ABSOLUTE"));
        return;
    }
    let sizing = |k: &str| match src.get(k).and_then(Value::as_i64) {
        Some(1) => "HUG",
        Some(2) => "FILL",
        _ => "FIXED",
    };
    let (mut h, mut v) = (sizing("horizontalSizing"), sizing("verticalSizing"));
    // A stretching stack fills its items across, fixed and hugging ones too.
    if stretch {
        if row {
            v = "FILL";
        } else {
            h = "FILL";
        }
    }
    m.insert("layoutSizingHorizontal".into(), json!(h));
    m.insert("layoutSizingVertical".into(), json!(v));
    let (main, cross) = if row { (h, v) } else { (v, h) };
    if main == "FILL" {
        m.insert("layoutGrow".into(), json!(1));
    }
    if cross == "FILL" {
        m.insert("layoutAlign".into(), json!("STRETCH"));
    }
}
