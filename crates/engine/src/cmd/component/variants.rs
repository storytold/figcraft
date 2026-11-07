use super::{
    BTreeMap, BTreeSet, Document, EngineError, Node, NodeId, NodeKind, Result, Session, Value, bad, inside_instance, json, owning_instance,
    req_targets, roots, set_name_from, string, targets, variant_name, variant_options, variant_props, variant_set, z_sorted,
};

/// Wrap components in a new component set (named after the first) placed around them.
pub(super) fn wrap_in_set(cmd: &str, doc: &mut Document, ids: &[NodeId]) -> Result<NodeId> {
    let top = *ids.last().ok_or_else(|| bad(cmd, "nothing selected"))?;
    let parent = doc.get(top)?.parent.ok_or_else(|| bad(cmd, "page"))?;
    if inside_instance(doc, top) || owning_instance(doc, parent).is_some() {
        return Err(EngineError::Disabled(cmd.into(), INSIDE_INSTANCE.into()));
    }
    let bounds = doc.bounds_of(ids).unwrap_or_default();
    let pw = doc.world_transform(parent);
    let local = figcraft_geom::transform_rect_bbox(pw.inverse(), bounds.inflate(20.0, 20.0));
    let base = ids.first().and_then(|i| doc.node(*i)).and_then(|n| set_name_from(&n.name)).unwrap_or_else(|| doc.next_name("Component"));
    let set = doc.alloc_id();
    let mut n = Node::new(set, NodeKind::ComponentSet { description: String::new() }, base, local.width(), local.height());
    n.transform = figcraft_geom::Affine::translate((local.x0, local.y0));
    n.fills.clear();
    doc.insert(n, parent, doc.index_in_parent(top).map(|i| i + 1))?;
    for i in ids {
        doc.reparent(*i, set, None)?;
    }
    Ok(set)
}

const INSIDE_INSTANCE: &str = "components inside an instance can't become variants";

/// Why `ids` (distinct) can't become the variants of a new component set, or `None` when they
/// can: anything but main components, components that are variants already (combining them would
/// put a set in a set), library copies, components inside an instance, and a component inside
/// another one of `ids`.
fn combine_refusal(doc: &Document, ids: &[NodeId]) -> Option<&'static str> {
    if !ids.iter().all(|i| doc.node(*i).is_some_and(|n| matches!(n.kind, NodeKind::Component { .. }))) {
        Some("select components only")
    } else if ids.iter().any(|i| variant_set(doc, *i).is_some()) {
        Some("components that are variants already can't be combined again")
    } else if ids.iter().any(|i| doc.is_remote(*i)) {
        Some(crate::library::READ_ONLY)
    } else if ids.iter().any(|i| inside_instance(doc, *i)) {
        Some(INSIDE_INSTANCE)
    } else if roots(doc, ids).len() != ids.len() {
        Some("a component can't be combined with one inside it")
    } else {
        None
    }
}

/// Whether "Combine as variants" is on offer for `ids` (menus and the Design panel): two or more
/// main components that [`combine_refusal`] lets through.
pub fn can_combine_variants(doc: &Document, ids: &[NodeId]) -> bool {
    ids.len() >= 2 && combine_refusal(doc, ids).is_none() && ids.iter().collect::<BTreeSet<_>>().len() == ids.len()
}

/// Whether "Add variant" is on offer for `id`: a component set with variants, a variant, or a main
/// component not in a set yet (it becomes one); never a library copy or inside an instance.
pub fn can_add_variant(doc: &Document, id: NodeId) -> bool {
    let Some(n) = doc.node(id) else { return false };
    let local = !doc.is_remote(id) && !inside_instance(doc, id);
    match n.kind {
        NodeKind::ComponentSet { .. } => {
            local && n.children.iter().any(|c| doc.node(*c).is_some_and(|c| matches!(c.kind, NodeKind::Component { .. })))
        }
        NodeKind::Component { .. } => local,
        _ => false,
    }
}

/// Wrap `ids` (z-sorted main components) in a new component set, naming them "Property 1=Default",
/// "Property 1=VariantN" unless already in prop=value form. Returns the set.
fn make_variant_set(cmd: &str, doc: &mut Document, ids: &[NodeId]) -> Result<NodeId> {
    let set = wrap_in_set(cmd, doc, ids)?;
    for (k, i) in ids.iter().enumerate() {
        let node = doc.get_mut(*i)?;
        if !node.name.contains('=') {
            node.name = format!("Property 1={}", if k == 0 { "Default".to_string() } else { format!("Variant{}", k + 1) });
        }
    }
    Ok(set)
}

pub(super) fn combine(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "component.combineVariants";
    let mut seen = BTreeSet::new();
    let ids: Vec<NodeId> = req_targets(cmd, s, p)?.into_iter().filter(|i| seen.insert(*i)).collect();
    let doc = s.doc_mut()?;
    if let Some(why) = combine_refusal(doc, &ids) {
        return Err(EngineError::Disabled(cmd.into(), why.into()));
    }
    let ids = z_sorted(doc, &ids);
    let set = make_variant_set(cmd, doc, &ids)?;
    s.set_selection(vec![set]);
    Ok(json!({"id": set}))
}

/// Grow a component set so it wraps its variants with 20 px of padding (right and bottom).
pub(super) fn fit_set(doc: &mut Document, set: NodeId) {
    let Some(kids) = doc.node(set).map(|n| n.children.clone()) else { return };
    let (mut w, mut h) = (0.0f64, 0.0f64);
    for c in kids {
        if let Some(n) = doc.node(c) {
            let b = figcraft_geom::transform_rect_bbox(n.transform, n.local_rect());
            w = w.max(b.x1 + 20.0);
            h = h.max(b.y1 + 20.0);
        }
    }
    if let Some(n) = doc.node_mut(set) {
        n.width = n.width.max(w);
        n.height = n.height.max(h);
    }
}

pub(super) fn add_variant(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "component.addVariant";
    let t = targets(s, p).first().copied().ok_or_else(|| bad(cmd, "select a component set or component"))?;
    let doc = s.doc_mut()?;
    let (set, base) = match doc.get(t)?.kind {
        NodeKind::ComponentSet { .. } => {
            let last = doc.get(t)?.children.last().copied().ok_or_else(|| bad(cmd, "the component set is empty"))?;
            (t, last)
        }
        NodeKind::Component { .. } => match variant_set(doc, t) {
            Some(set) => (set, t),
            // A main component on its own becomes a set of itself, then gains the new variant.
            None => (make_variant_set(cmd, doc, &[t])?, t),
        },
        _ => return Err(bad(cmd, "select a component set or component")),
    };
    let options = variant_options(doc, set);
    let mut pairs = variant_props(&doc.get(base)?.name);
    let order: Vec<String> = options.iter().map(|(k, _)| k.clone()).collect();
    let existing: BTreeSet<BTreeMap<String, String>> =
        doc.get(set)?.children.iter().filter_map(|c| doc.node(*c)).map(|n| variant_props(&n.name)).collect();
    match (string(p, "property"), string(p, "value")) {
        (Some(prop), Some(value)) => {
            let (prop, value) = (prop.trim().to_string(), value.trim().to_string());
            if !options.iter().any(|(k, _)| *k == prop) {
                return Err(bad(cmd, format!("unknown variant property `{prop}`")));
            }
            if value.is_empty() || value.contains([',', '=']) {
                return Err(bad(cmd, "`value` must be non-empty without ',' or '='"));
            }
            pairs.insert(prop, value);
            if existing.contains(&pairs) {
                return Err(bad(cmd, "that variant already exists"));
            }
        }
        (Some(_), None) => return Err(bad(cmd, "`value` is required with `property`")),
        _ => {
            let prop = order.first().cloned().unwrap_or_else(|| "Property 1".into());
            let mut k = existing.len() + 1;
            loop {
                pairs.insert(prop.clone(), format!("Variant{k}"));
                if !existing.contains(&pairs) || k > 10_000 {
                    break;
                }
                k += 1;
            }
        }
    }
    let name = variant_name(&pairs, &order);
    let bottom = doc
        .get(set)?
        .children
        .iter()
        .filter_map(|c| doc.node(*c))
        .map(|n| figcraft_geom::transform_rect_bbox(n.transform, n.local_rect()).y1)
        .fold(0.0f64, f64::max);
    let dup = doc.duplicate_into(base, set, None)?;
    let n = doc.get_mut(dup)?;
    n.name = name;
    let c = n.transform.as_coeffs();
    n.transform = figcraft_geom::Affine::new([c[0], c[1], c[2], c[3], c[4], bottom + 20.0]);
    fit_set(doc, set);
    s.set_selection(vec![dup]);
    Ok(json!({"id": dup}))
}
