use super::instances::PROP_BINDING;
use super::{Document, Node, NodeId, NodeKind, PropDef, Sizing, TextAutoResize, TextData, inside_instance, owning_component, variant_set};

/// The property groups whose value layer `n` takes from the same variables as its main layer
/// `src` (they differ only where the two resolve in different variable modes).
pub fn variable_groups(n: &Node, src: &Node) -> Vec<&'static str> {
    let mut out = Vec::new();
    for (k, v) in &n.bound {
        let Some(g) = binding_group(k) else { continue };
        if src.bound.get(k) == Some(v) && !out.contains(&g) {
            out.push(g);
        }
    }
    out
}

/// The override group a variable binding key feeds (`None` for component property and layout
/// grid bindings, which follow other rules).
pub(super) fn binding_group(key: &str) -> Option<&'static str> {
    use crate::cmd::lists::{BoundList, list_binding};
    if key.starts_with(PROP_BINDING) {
        return None;
    }
    match list_binding(key).map(|(l, _, _)| l) {
        Some(BoundList::Fills) => Some("fills"),
        Some(BoundList::Strokes) => Some("strokes"),
        Some(BoundList::Effects) => Some("effects"),
        Some(BoundList::Grids) => None,
        None => Some(crate::cmd::props::override_group(key)).filter(|g| *g != "other"),
    }
}

/// The variable bindings of `n` that feed group `g`.
pub(super) fn bindings_of<'a>(n: &'a Node, g: &'a str) -> impl Iterator<Item = (&'a String, &'a String)> + 'a {
    n.bound.iter().filter(move |(k, _)| binding_group(k) == Some(g))
}

/// Property groups that differ between two versions of a node.
pub fn diff_groups(a: &Node, b: &Node) -> Vec<&'static str> {
    let mut out = Vec::new();
    if a.fills != b.fills {
        out.push("fills");
    }
    if a.strokes != b.strokes || a.stroke != b.stroke {
        out.push("strokes");
    }
    if a.effects != b.effects {
        out.push("effects");
    }
    if a.opacity != b.opacity || a.blend != b.blend {
        out.push("opacity");
    }
    if a.visible != b.visible {
        out.push("visible");
    }
    if a.name != b.name {
        out.push("name");
    }
    if a.corners != b.corners {
        out.push("corners");
    }
    if (a.width - b.width).abs() > 1e-6 || (a.height - b.height).abs() > 1e-6 {
        out.push("size");
    }
    if resizing(a) != resizing(b) {
        out.push("resizing");
    }
    if a.transform != b.transform {
        out.push("position");
    }
    if let (NodeKind::Text(x), NodeKind::Text(y)) = (&a.kind, &b.kind) {
        if x.characters != y.characters {
            out.push("characters");
        }
        if text_style_differs(x, y) {
            out.push("textStyle");
        }
    }
    if let (Some(x), Some(y)) = (&a.layout, &b.layout) {
        if x.padding != y.padding {
            out.push("padding");
        }
        if x.gap != y.gap || x.cross_gap != y.cross_gap {
            out.push("gap");
        }
        if x.main_align != y.main_align || x.cross_align != y.cross_align || x.content_align != y.content_align {
            out.push("alignment");
        }
    }
    if a.grids != b.grids || a.styles.grid != b.styles.grid {
        out.push("grids");
    }
    if a.exports != b.exports {
        out.push("exports");
    }
    if a.interactions != b.interactions {
        out.push("interactions");
    }
    if a.var_modes != b.var_modes {
        out.push("modes");
    }
    if let (NodeKind::Instance { component: x, props: px }, NodeKind::Instance { component: y, props: py }) = (&a.kind, &b.kind) {
        if x != y {
            out.push("component");
        }
        if px != py {
            out.push("props");
        }
    }
    out
}

/// The `resizing` group of a layer: how it sizes on each axis, and its min and max sizes.
fn resizing(n: &Node) -> (Sizing, Sizing, [Option<f64>; 4]) {
    let c = &n.child_layout;
    (c.sizing_h, c.sizing_v, [c.min_w, c.max_w, c.min_h, c.max_h])
}

/// Whether two texts differ in anything but their characters (the `textStyle` group).
fn text_style_differs(x: &TextData, y: &TextData) -> bool {
    x.style != y.style
        || x.runs != y.runs
        || x.auto_resize != y.auto_resize
        || x.align_h != y.align_h
        || x.align_v != y.align_v
        || x.paragraph_spacing != y.paragraph_spacing
        || x.paragraph_indent != y.paragraph_indent
        || x.list != y.list
        || x.paragraphs != y.paragraphs
        || x.list_spacing != y.list_spacing
        || x.hanging_punctuation != y.hanging_punctuation
        || x.hanging_list != y.hanging_list
        || x.max_lines != y.max_lines
        || x.features != y.features
}

/// Whether a geometry change of `id` comes from layout rather than the user: an ancestor was
/// resized in the same edit (constraints), the parent is an auto layout frame, or auto-sized text.
pub(super) fn geometry_derived(doc: &Document, before: &Document, id: NodeId, group: &str) -> bool {
    for a in doc.ancestors(id) {
        let (Some(x), Some(y)) = (doc.node(a), before.node(a)) else { continue };
        if (x.width - y.width).abs() > 1e-6 || (x.height - y.height).abs() > 1e-6 {
            return true;
        }
    }
    let Some(n) = doc.node(id) else { return true };
    if group == "position" && n.parent.and_then(|p| doc.node(p)).is_some_and(|p| p.layout.is_some()) && !n.child_layout.absolute {
        return true;
    }
    if group == "size"
        && let NodeKind::Text(t) = &n.kind
    {
        let wchanged = before.node(id).is_some_and(|b| (b.width - n.width).abs() > 1e-6);
        return match t.auto_resize {
            TextAutoResize::WidthAndHeight => true,
            TextAutoResize::Height => !wchanged,
            _ => false,
        };
    }
    false
}

/// The components sharing the properties of component `c`: the variants of its set, or `c` alone.
pub fn property_owners(doc: &Document, c: NodeId) -> Vec<NodeId> {
    match variant_set(doc, c).and_then(|s| doc.node(s)) {
        Some(set) => set.children.iter().copied().filter(|v| doc.node(*v).is_some_and(|n| matches!(n.kind, NodeKind::Component { .. }))).collect(),
        None => vec![c],
    }
}

/// Make `value` the default of boolean property `name` on components `comps`, showing or hiding
/// their layers bound to it: on a main component, a boolean property's value is the visibility of
/// its layers. Returns the components and layers it changed.
pub fn set_boolean_default(doc: &mut Document, comps: &[NodeId], name: &str, value: bool) -> Vec<NodeId> {
    let mut out = Vec::new();
    for &c in comps {
        let Some(NodeKind::Component { props, .. }) = doc.node(c).map(|n| &n.kind) else { continue };
        let Some(PropDef::Boolean { default }) = props.get(name) else { continue };
        if *default != value
            && let Some(NodeKind::Component { props, .. }) = doc.node_mut(c).map(|n| &mut n.kind)
        {
            props.insert(name.to_string(), PropDef::Boolean { default: value });
            out.push(c);
        }
        // The main's own layers (not those of instances nested in it, which follow their mains).
        let layers: Vec<NodeId> = doc
            .subtree(c)
            .into_iter()
            .filter(|d| *d != c && !inside_instance(doc, *d))
            .filter(|d| doc.node(*d).is_some_and(|n| n.visible != value && n.prop_refs.get("visible").map(String::as_str) == Some(name)))
            .collect();
        for d in layers {
            if let Some(n) = doc.node_mut(d) {
                n.visible = value;
                out.push(d);
            }
        }
    }
    out
}

/// A main component's layer bound to a boolean property was shown or hidden: that is the
/// property's new default, which its other bound layers (in every variant) take too. Returns the
/// components and layers that changed as a result.
pub(super) fn follow_boolean_defaults(doc: &mut Document, before: &Document, changed: &[NodeId]) -> Vec<NodeId> {
    let mut out = Vec::new();
    for &id in changed {
        let (Some(n), Some(b)) = (doc.node(id), before.node(id)) else { continue };
        if n.visible == b.visible || inside_instance(doc, id) {
            continue;
        }
        let Some(name) = n.prop_refs.get("visible").cloned() else { continue };
        let visible = n.visible;
        let Some(c) = owning_component(doc, id).filter(|c| *c != id) else { continue };
        out.extend(set_boolean_default(doc, &property_owners(doc, c), &name, visible));
    }
    out
}
