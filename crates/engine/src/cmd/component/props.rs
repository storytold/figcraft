use figcraft_doc::ordered_props;

use super::instance::preferred_arg;
use super::variants::{fit_set, wrap_in_set};
use super::{
    BTreeMap, Document, NodeId, NodeKind, PROP_BINDING, PropDef, Result, SLOT, Session, Value, VariableType, VariableValue, bad, boolean,
    default_variant, find_variant, get, id, id_of, inside_instance, instances_of, json, main_of, ok, owning_component, prop_default_value,
    req_string, req_targets, set_boolean_default, slot_edited, slots_of, string, takes_value, targets, variant_conflicts, variant_name,
    variant_options, variant_props, variant_set,
};
use crate::components::prop_list;

/// Whether instance `id` has a component property `name` that a `kind` variable can drive (a
/// boolean property takes a boolean variable, a text or variant property a string one).
pub(crate) fn check_prop_binding(doc: &Document, id: NodeId, name: &str, kind: VariableType) -> std::result::Result<(), String> {
    let Some(NodeKind::Instance { component, .. }) = doc.node(id).map(|n| &n.kind) else {
        return Err(format!("{id} is not an instance (component properties are bound on instances)"));
    };
    if variant_set(doc, *component).is_some_and(|set| variant_options(doc, set).iter().any(|(o, _)| o == name)) {
        return if kind == VariableType::String { Ok(()) } else { Err(format!("`{name}` is a variant property: bind a string variable")) };
    }
    match (prop_defs(doc, *component).get(name), kind) {
        (Some(PropDef::Boolean { .. }), VariableType::Boolean) | (Some(PropDef::Text { .. }), VariableType::String) => Ok(()),
        (Some(PropDef::Boolean { .. }), _) => Err(format!("`{name}` is a boolean property: bind a boolean variable")),
        (Some(PropDef::Text { .. }), _) => Err(format!("`{name}` is a text property: bind a string variable")),
        (Some(_), _) => Err(format!("`{name}`: only boolean and text properties can be bound to variables")),
        (None, _) => Err(format!("the instance has no property named `{name}`")),
    }
}

/// Write bound variable values into instance `id`'s boolean and text property values, and switch
/// it to the variant its bound variant properties name (only what changed; the instance then
/// follows like after `instance.setProperties`). A value that names no variant changes nothing.
pub(crate) fn apply_prop_bindings(doc: &mut Document, id: NodeId, sets: &[(String, VariableValue)]) {
    let Some(NodeKind::Instance { component, .. }) = doc.node(id).map(|n| &n.kind) else { return };
    let component = *component;
    if let Some(set) = variant_set(doc, component) {
        let options = variant_options(doc, set);
        let current = doc.node(component).map(|c| variant_props(&c.name)).unwrap_or_default();
        let mut wanted = current.clone();
        for (name, val) in sets {
            if let VariableValue::String(t) = val
                && options.iter().any(|(o, vals)| o == name && vals.contains(t))
            {
                wanted.insert(name.clone(), t.clone());
            }
        }
        if wanted != current
            && let Some(target) = find_variant(doc, set, &wanted)
            && let Ok(n) = doc.get_mut(id)
            && let NodeKind::Instance { component: c, .. } = &mut n.kind
        {
            *c = target;
        }
    }
    let Some(NodeKind::Instance { component, props }) = doc.node(id).map(|n| &n.kind) else { return };
    let Some(NodeKind::Component { props: defs, .. }) = doc.node(*component).map(|c| &c.kind) else { return };
    let mut next = props.clone();
    for (name, val) in sets {
        let v = match (defs.get(name), val) {
            (Some(PropDef::Boolean { .. }), VariableValue::Boolean(b)) => json!(b),
            (Some(PropDef::Text { .. }), VariableValue::String(t)) => json!(t),
            _ => continue,
        };
        next.insert(name.clone(), v);
    }
    if next != *props
        && let Ok(n) = doc.get_mut(id)
        && let NodeKind::Instance { props, .. } = &mut n.kind
    {
        *props = next;
    }
}

/// Coerce a property value to the definition's type.
fn coerce(cmd: &str, doc: &Document, name: &str, def: &PropDef, v: &Value) -> Result<Value> {
    match def {
        PropDef::Boolean { .. } => match v {
            Value::Bool(b) => Ok(json!(b)),
            Value::String(t) if t == "true" || t == "false" => Ok(json!(t == "true")),
            _ => Err(bad(cmd, format!("`{name}` is a boolean property"))),
        },
        PropDef::Text { .. } => match v {
            Value::String(t) => Ok(json!(t)),
            Value::Number(n) => Ok(json!(n.to_string())),
            _ => Err(bad(cmd, format!("`{name}` is a text property"))),
        },
        PropDef::InstanceSwap { .. } => match id_of(v).and_then(|c| main_of(doc, c)) {
            Some(c) => Ok(json!(c.0)),
            None => Err(bad(cmd, format!("`{name}` takes a component id"))),
        },
        PropDef::Variant { options, .. } => {
            let t = v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
            if options.is_empty() || options.contains(&t) { Ok(json!(t)) } else { Err(bad(cmd, format!("`{name}` must be one of {options:?}"))) }
        }
        PropDef::Slot { .. } => Err(bad(cmd, format!("`{name}` is a slot: fill it with instance.slotInsert or by adding layers to it"))),
    }
}

pub(super) fn set_props(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "instance.setProperties";
    let ids = req_targets(cmd, s, p)?;
    let props = get(p, "properties").and_then(Value::as_object).cloned().ok_or_else(|| bad(cmd, "`properties` object required"))?;
    let doc = s.doc_mut()?;
    for i in ids {
        let Some(NodeKind::Instance { component, .. }) = doc.node(i).map(|n| n.kind.clone()) else {
            return Err(bad(cmd, format!("{i} is not an instance")));
        };
        // Variant properties: switch to the matching sibling variant.
        let set = variant_set(doc, component);
        let options = set.map(|s| variant_options(doc, s)).unwrap_or_default();
        let mut wanted = doc.node(component).map(|c| variant_props(&c.name)).unwrap_or_default();
        let mut target = component;
        let mut rest = Vec::new();
        for (k, v) in &props {
            if let Some((_, vals)) = options.iter().find(|(o, _)| o == k) {
                let val = v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
                if !vals.contains(&val) {
                    return Err(bad(cmd, format!("`{k}` must be one of {vals:?}")));
                }
                wanted.insert(k.clone(), val);
            } else {
                rest.push((k.clone(), v.clone()));
            }
        }
        if let Some(set) = set
            && wanted != doc.node(component).map(|c| variant_props(&c.name)).unwrap_or_default()
        {
            target = find_variant(doc, set, &wanted).unwrap_or(component);
        }
        let defs = match doc.node(target).map(|n| &n.kind) {
            Some(NodeKind::Component { props, .. }) => props.clone(),
            _ => BTreeMap::new(),
        };
        let mut values = Vec::new();
        for (k, v) in rest {
            let def = defs.get(&k).ok_or_else(|| {
                let known: Vec<&str> = defs.keys().map(String::as_str).chain(options.iter().map(|(o, _)| o.as_str())).collect();
                bad(cmd, format!("unknown property `{k}` (known: {})", known.join(", ")))
            })?;
            values.push((k.clone(), coerce(cmd, doc, &k, def, &v)?));
        }
        if let NodeKind::Instance { component: c, props: ip } = &mut doc.get_mut(i)?.kind {
            *c = target;
            for (k, v) in values {
                ip.insert(k, v);
            }
            for (k, d) in defs.iter().filter(|(_, d)| takes_value(d)) {
                ip.entry(k.clone()).or_insert_with(|| prop_default_value(d));
            }
        }
    }
    ok()
}

/// Where component properties live: a set (its variants share them) or a lone component.
pub(super) enum Owner {
    Set(NodeId),
    Comp(NodeId),
}

impl Owner {
    pub(super) fn resolve(doc: &Document, id: NodeId) -> Option<Owner> {
        let n = doc.node(id)?;
        match n.kind {
            NodeKind::ComponentSet { .. } => Some(Owner::Set(id)),
            _ => {
                let c = owning_component(doc, id)?;
                Some(match variant_set(doc, c) {
                    Some(s) => Owner::Set(s),
                    None => Owner::Comp(c),
                })
            }
        }
    }

    pub(super) fn components(&self, doc: &Document) -> Vec<NodeId> {
        match self {
            Owner::Comp(c) => vec![*c],
            Owner::Set(s) => doc
                .node(*s)
                .map(|n| n.children.iter().copied().filter(|c| doc.node(*c).is_some_and(|x| matches!(x.kind, NodeKind::Component { .. }))).collect())
                .unwrap_or_default(),
        }
    }

    fn id(&self) -> NodeId {
        match self {
            Owner::Set(s) | Owner::Comp(s) => *s,
        }
    }
}

pub(super) fn prop_defs(doc: &Document, c: NodeId) -> BTreeMap<String, PropDef> {
    match doc.node(c).map(|n| &n.kind) {
        Some(NodeKind::Component { props, .. }) => props.clone(),
        _ => BTreeMap::new(),
    }
}

/// Change component `c`'s property definitions with `f`, keeping their order: the property
/// `rename` names (old, new) keeps its place, new ones go last.
fn set_prop_defs(doc: &mut Document, c: NodeId, rename: Option<(&str, &str)>, f: impl FnOnce(&mut BTreeMap<String, PropDef>)) -> Result<()> {
    if let NodeKind::Component { props, prop_order, .. } = &mut doc.get_mut(c)?.kind {
        let mut order: Vec<String> = ordered_props(props, prop_order)
            .into_iter()
            .map(|(k, _)| match rename {
                Some((old, new)) if k == old => new.to_string(),
                _ => k.clone(),
            })
            .collect();
        f(props);
        order.retain(|k| props.contains_key(k));
        let added: Vec<String> = props.keys().filter(|k| !order.contains(k)).cloned().collect();
        order.extend(added);
        *prop_order = order;
    }
    Ok(())
}

fn valid_prop_name(cmd: &str, name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.contains([',', '=']) || name.chars().count() > 200 {
        return Err(bad(cmd, "property names must be non-empty, without ',' or '='"));
    }
    Ok(name.to_string())
}

pub(super) fn add_prop(s: &mut Session, p: &Value) -> Result<Value> {
    add_prop_for("component.addProperty", s, p)
}

/// Add a component property for command `cmd`, which errors name (`component.makeSlot` adds its
/// slot property here).
pub(super) fn add_prop_for(cmd: &str, s: &mut Session, p: &Value) -> Result<Value> {
    let target = targets(s, p).first().copied().ok_or_else(|| bad(cmd, "select a component"))?;
    let name = valid_prop_name(cmd, &req_string(cmd, p, "name")?)?;
    let t = req_string(cmd, p, "type")?;
    let doc = s.doc_mut()?;
    if inside_instance(doc, target) {
        return Err(bad(cmd, "select a component, not a layer of an instance"));
    }
    let owner = Owner::resolve(doc, target).ok_or_else(|| bad(cmd, "select a component, a component set or a layer inside a component"))?;
    // A layer selected inside the component gets bound to the new property.
    let layer = id(p, "layer")
        .or_else(|| (target != owner.id() && !matches!(doc.node(target).map(|n| &n.kind), Some(NodeKind::Component { .. }))).then_some(target));
    let comps = owner.components(doc);
    let variants = match &owner {
        Owner::Set(s) => variant_options(doc, *s),
        Owner::Comp(_) => Vec::new(),
    };
    if variants.iter().any(|(k, _)| *k == name) || comps.iter().any(|c| prop_defs(doc, *c).contains_key(&name)) {
        return Err(bad(cmd, format!("a property named `{name}` already exists")));
    }
    let layer_node = layer.and_then(|l| doc.node(l)).cloned();
    let def = match t.to_ascii_lowercase().as_str() {
        // Without a default, a bound layer keeps showing as it does: its visibility is the default.
        "boolean" | "bool" => PropDef::Boolean { default: boolean(p, "default").or_else(|| layer_node.as_ref().map(|n| n.visible)).unwrap_or(true) },
        "text" => PropDef::Text {
            default: string(p, "default")
                .or_else(|| layer_node.as_ref().and_then(|n| n.kind.text()).map(|t| t.characters.clone()))
                .unwrap_or_else(|| name.clone()),
        },
        "instanceswap" | "instance_swap" | "instance" => {
            let d = id(p, "default")
                .or_else(|| match layer_node.as_ref().map(|n| &n.kind) {
                    Some(NodeKind::Instance { component, .. }) => Some(*component),
                    _ => None,
                })
                .and_then(|c| main_of(doc, c))
                .ok_or_else(|| bad(cmd, "default: a component id"))?;
            PropDef::InstanceSwap { default: d, preferred: preferred_arg(cmd, doc, p)? }
        }
        "variant" => {
            let dv = string(p, "default")
                .map(|d| d.trim().to_string())
                .filter(|d| !d.is_empty() && !d.contains([',', '=']))
                .unwrap_or_else(|| "Default".into());
            let set = match owner {
                Owner::Set(s) => s,
                Owner::Comp(c) => wrap_in_set(cmd, doc, &[c])?,
            };
            for c in Owner::Set(set).components(doc) {
                let cn = doc.get_mut(c)?;
                cn.name = if cn.name.contains('=') { format!("{}, {name}={dv}", cn.name) } else { format!("{name}={dv}") };
            }
            s.set_selection(vec![set]);
            return Ok(json!({"id": set}));
        }
        "slot" => {
            if !layer_node.as_ref().is_some_and(|n| matches!(n.kind, NodeKind::Frame)) {
                return Err(bad(cmd, "a slot is a frame inside the component: select one (or pass `layer`)"));
            }
            PropDef::Slot { preferred: preferred_arg(cmd, doc, p)? }
        }
        _ => return Err(bad(cmd, "type: boolean|text|instanceSwap|variant|slot")),
    };
    for c in &comps {
        set_prop_defs(doc, *c, None, |m| {
            m.insert(name.clone(), def.clone());
        })?;
        // Existing instances get the default.
        if !takes_value(&def) {
            continue;
        }
        for inst in instances_of(doc, *c) {
            if let NodeKind::Instance { props, .. } = &mut doc.get_mut(inst)?.kind {
                props.entry(name.clone()).or_insert_with(|| prop_default_value(&def));
            }
        }
    }
    if let Some(l) = layer {
        bind(cmd, doc, l, Some(&name), None)?;
    }
    Ok(json!({"id": owner.id(), "name": name}))
}

/// Bind (or with `property: None` unbind) a layer's field to a property of its component.
fn bind(cmd: &str, doc: &mut Document, layer: NodeId, property: Option<&str>, field: Option<&str>) -> Result<()> {
    let n = doc.get(layer)?.clone();
    if inside_instance(doc, layer) {
        return Err(bad(cmd, "layers inside an instance can't be bound (bind the nested instance itself, or edit its main)"));
    }
    let comp = owning_component(doc, layer).ok_or_else(|| bad(cmd, "the layer isn't inside a component"))?;
    let Some(property) = property.filter(|p| !p.is_empty()) else {
        let n = doc.get_mut(layer)?;
        match field {
            Some(f) => {
                n.prop_refs.remove(f);
            }
            None => n.prop_refs.clear(),
        }
        return Ok(());
    };
    let defs = prop_defs(doc, comp);
    let def = defs.get(property).ok_or_else(|| bad(cmd, format!("the component has no property `{property}`")))?;
    let expected = match def {
        PropDef::Boolean { .. } => "visible",
        PropDef::Text { .. } => "characters",
        PropDef::InstanceSwap { .. } => "mainComponent",
        PropDef::Variant { .. } => return Err(bad(cmd, "variant properties pick a variant; they can't be bound to a layer")),
        PropDef::Slot { .. } => SLOT,
    };
    let field = field.unwrap_or(expected);
    if field != expected {
        return Err(bad(cmd, format!("`{property}` can only control `{expected}`")));
    }
    match field {
        "characters" if n.kind.text().is_none() => return Err(bad(cmd, "text properties bind to text layers")),
        "mainComponent" if !matches!(n.kind, NodeKind::Instance { .. }) => return Err(bad(cmd, "instance swap properties bind to instances")),
        "visible" if layer == comp => return Err(bad(cmd, "the component itself can't be hidden by its own property")),
        SLOT if !matches!(n.kind, NodeKind::Frame) => return Err(bad(cmd, "slot properties bind to frames inside the component")),
        SLOT if doc
            .subtree(comp)
            .into_iter()
            .any(|d| d != layer && doc.node(d).is_some_and(|x| x.prop_refs.get(SLOT).map(String::as_str) == Some(property))) =>
        {
            return Err(bad(cmd, format!("another frame of the component is already the slot `{property}`")));
        }
        _ => {}
    }
    let n = doc.get_mut(layer)?;
    n.prop_refs.insert(field.to_string(), property.to_string());
    // On the main, a boolean property's value is the visibility of the layers it controls.
    if let PropDef::Boolean { default } = def {
        n.visible = *default;
    }
    Ok(())
}

pub(super) fn bind_prop(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "component.bindProperty";
    let layer = id(p, "layer").or_else(|| targets(s, p).first().copied()).ok_or_else(|| bad(cmd, "`layer` required"))?;
    let property = string(p, "property");
    let field = string(p, "field");
    if let Some(f) = &field
        && !matches!(f.as_str(), "visible" | "characters" | "mainComponent" | SLOT)
    {
        return Err(bad(cmd, "field: visible|characters|mainComponent|slot"));
    }
    let doc = s.doc_mut()?;
    bind(cmd, doc, layer, property.as_deref(), field.as_deref())?;
    ok()
}

/// Rename `old` → `new` in the bindings of a component's layers and in its instances' values.
fn rename_prop_refs(doc: &mut Document, comp: NodeId, old: &str, new: Option<&str>) -> Result<()> {
    for d in doc.subtree(comp) {
        let refs = doc.node(d).map(|n| n.prop_refs.clone()).unwrap_or_default();
        if refs.values().any(|v| v == old)
            && let Some(n) = doc.node_mut(d)
        {
            match new {
                Some(nw) => n.prop_refs.values_mut().filter(|v| *v == old).for_each(|v| *v = nw.to_string()),
                None => n.prop_refs.retain(|_, v| v != old),
            }
        }
    }
    // Instances' values and the variables bound to them follow (a deleted property drops both).
    let old_key = format!("{PROP_BINDING}{old}");
    for inst in instances_of(doc, comp) {
        let n = doc.get_mut(inst)?;
        if let NodeKind::Instance { props, .. } = &mut n.kind
            && let Some(v) = props.remove(old)
            && let Some(nw) = new
        {
            props.insert(nw.to_string(), v);
        }
        if let Some(var) = n.bound.remove(&old_key)
            && let Some(nw) = new
        {
            n.bound.insert(format!("{PROP_BINDING}{nw}"), var);
        }
    }
    Ok(())
}

pub(super) fn edit_prop(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "component.editProperty";
    let target = targets(s, p).first().copied().ok_or_else(|| bad(cmd, "select a component"))?;
    let name = req_string(cmd, p, "name")?;
    let new_name = string(p, "newName").map(|n| valid_prop_name(cmd, &n)).transpose()?.filter(|n| *n != name);
    let doc = s.doc_mut()?;
    let owner = Owner::resolve(doc, target).ok_or_else(|| bad(cmd, "select a component or component set"))?;
    let comps = owner.components(doc);
    if let Some(nn) = &new_name {
        let taken = comps.iter().any(|c| prop_defs(doc, *c).contains_key(nn))
            || matches!(owner, Owner::Set(s) if variant_options(doc, s).iter().any(|(k, _)| k == nn));
        if taken {
            return Err(bad(cmd, format!("a property named `{nn}` already exists")));
        }
    }
    // Variant property of a set.
    if let Owner::Set(set) = owner
        && let Some((_, values)) = variant_options(doc, set).into_iter().find(|(k, _)| *k == name)
    {
        let order: Vec<String> = variant_options(doc, set).into_iter().map(|(k, _)| k).collect();
        let rename_value = match (string(p, "value"), string(p, "newValue")) {
            (Some(v), Some(nv)) => {
                let nv = nv.trim().to_string();
                if !values.contains(&v) {
                    return Err(bad(cmd, format!("`{name}` has no value `{v}`")));
                }
                if nv.is_empty() || nv.contains([',', '=']) {
                    return Err(bad(cmd, "`newValue` must be non-empty, without ',' or '='"));
                }
                Some((v, nv))
            }
            _ => None,
        };
        let mut order: Vec<String> = order.into_iter().map(|k| if k == name { new_name.clone().unwrap_or(k) } else { k }).collect();
        // Reordered: every variant's name lists the properties in the new order.
        if let Some(to) = get(p, "index").filter(|v| !v.is_null()) {
            let to = to.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad(cmd, "`index`: a property place (0 = first)"))?;
            let key = new_name.clone().unwrap_or(name.clone());
            order.retain(|k| *k != key);
            order.insert(to.min(order.len()), key);
        }
        for c in &comps {
            let mut pairs = variant_props(&doc.get(*c)?.name);
            if let Some((v, nv)) = &rename_value
                && pairs.get(&name) == Some(v)
            {
                pairs.insert(name.clone(), nv.clone());
            }
            if let Some(nn) = &new_name
                && let Some(v) = pairs.remove(&name)
            {
                pairs.insert(nn.clone(), v);
            }
            doc.get_mut(*c)?.name = variant_name(&pairs, &order);
        }
        // The default variant is the top-left one: the variant with that value (and the default's
        // other values, when there is one) trades places with it.
        if let Some(dv) = string(p, "default") {
            let key = new_name.clone().unwrap_or(name.clone());
            let current = default_variant(doc, set);
            let mut wanted = current.and_then(|d| doc.node(d)).map(|n| variant_props(&n.name)).unwrap_or_default();
            wanted.insert(key.clone(), dv.clone());
            let props_of = |c: &NodeId| doc.node(*c).map(|n| variant_props(&n.name)).unwrap_or_default();
            let pick =
                comps.iter().copied().find(|c| props_of(c) == wanted).or_else(|| comps.iter().copied().find(|c| props_of(c).get(&key) == Some(&dv)));
            let (Some(f), Some(d)) = (pick, current) else { return Err(bad(cmd, format!("no variant has {key}={dv}"))) };
            if f != d {
                let place = |doc: &Document, c: NodeId| doc.node(c).map(|n| n.transform.as_coeffs()).map(|[.., x, y]| (x, y)).unwrap_or_default();
                let (fp, dp) = (place(doc, f), place(doc, d));
                for (c, (x, y)) in [(f, dp), (d, fp)] {
                    let n = doc.get_mut(c)?;
                    let [a, b, cc, dd, ..] = n.transform.as_coeffs();
                    n.transform = figcraft_geom::Affine::new([a, b, cc, dd, x, y]);
                }
                fit_set(doc, set);
            }
        }
        return ok();
    }
    let def = comps.iter().find_map(|c| prop_defs(doc, *c).get(&name).cloned()).ok_or_else(|| bad(cmd, format!("no property named `{name}`")))?;
    let preferred = if get(p, "preferred").is_some() {
        if !matches!(def, PropDef::Slot { .. } | PropDef::InstanceSwap { .. }) {
            return Err(bad(cmd, "`preferred` is for instance swap and slot properties"));
        }
        Some(preferred_arg(cmd, doc, p)?)
    } else {
        None
    };
    let mut new_def = match get(p, "default") {
        Some(v) => match coerce(cmd, doc, &name, &def, v)? {
            Value::Bool(b) => PropDef::Boolean { default: b },
            Value::String(t) if matches!(def, PropDef::Text { .. }) => PropDef::Text { default: t },
            other => match (&def, id_of(&other)) {
                (PropDef::InstanceSwap { preferred, .. }, Some(c)) => PropDef::InstanceSwap { default: c, preferred: preferred.clone() },
                _ => def.clone(),
            },
        },
        None => def.clone(),
    };
    if let (Some(list), PropDef::Slot { preferred } | PropDef::InstanceSwap { preferred, .. }) = (preferred, &mut new_def) {
        *preferred = list;
    }
    for c in &comps {
        let had = prop_defs(doc, *c).contains_key(&name);
        if !had {
            continue;
        }
        let key = new_name.clone().unwrap_or(name.clone());
        let nd = new_def.clone();
        set_prop_defs(doc, *c, Some((&name, &key)), |m| {
            m.remove(&name);
            m.insert(key.clone(), nd);
        })?;
        if let Some(nn) = &new_name {
            rename_prop_refs(doc, *c, &name, Some(nn))?;
        }
    }
    // The main's bound layers show the new default.
    if let PropDef::Boolean { default } = new_def {
        set_boolean_default(doc, &comps, new_name.as_deref().unwrap_or(&name), default);
    }
    ok()
}

pub(super) fn delete_prop(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "component.deleteProperty";
    let target = targets(s, p).first().copied().ok_or_else(|| bad(cmd, "select a component"))?;
    let name = req_string(cmd, p, "name")?;
    let doc = s.doc_mut()?;
    let owner = Owner::resolve(doc, target).ok_or_else(|| bad(cmd, "select a component or component set"))?;
    let comps = owner.components(doc);
    if let Owner::Set(set) = owner
        && variant_options(doc, set).iter().any(|(k, _)| *k == name)
    {
        let order: Vec<String> = variant_options(doc, set).into_iter().map(|(k, _)| k).collect();
        for c in &comps {
            let mut pairs = variant_props(&doc.get(*c)?.name);
            pairs.remove(&name);
            let n = doc.get_mut(*c)?;
            n.name = if pairs.is_empty() { "Default".into() } else { variant_name(&pairs, &order) };
        }
        return ok();
    }
    let mut found = false;
    for c in &comps {
        if prop_defs(doc, *c).contains_key(&name) {
            found = true;
            set_prop_defs(doc, *c, None, |m| {
                m.remove(&name);
            })?;
            rename_prop_refs(doc, *c, &name, None)?;
        }
    }
    if !found {
        return Err(bad(cmd, format!("no property named `{name}`")));
    }
    ok()
}

fn def_json(name: &str, d: &PropDef) -> Value {
    match d {
        PropDef::Boolean { default } => json!({"name": name, "type": "boolean", "default": default}),
        PropDef::Text { default } => json!({"name": name, "type": "text", "default": default}),
        PropDef::InstanceSwap { default, preferred } => json!({"name": name, "type": "instanceSwap", "default": default, "preferred": preferred}),
        PropDef::Variant { options, default } => json!({"name": name, "type": "variant", "default": default, "options": options}),
        PropDef::Slot { preferred } => json!({"name": name, "type": "slot", "preferred": preferred}),
    }
}

/// Properties of a component, set or instance as JSON (`component.properties`).
pub(crate) fn describe_props(doc: &Document, id: NodeId) -> Value {
    let Some(n) = doc.node(id) else { return Value::Null };
    let (owner_comp, values) = match &n.kind {
        NodeKind::Instance { component, props } => (Some(*component), Some(props.clone())),
        _ => (main_of(doc, id).or_else(|| owning_component(doc, id)), None),
    };
    let Some(comp) = owner_comp else { return json!({"properties": []}) };
    let mut list: Vec<Value> = Vec::new();
    if let Some(set) = variant_set(doc, comp) {
        let current = doc.node(comp).map(|c| variant_props(&c.name)).unwrap_or_default();
        let defaults = default_variant(doc, set).and_then(|d| doc.node(d)).map(|n| variant_props(&n.name)).unwrap_or_default();
        for (k, vals) in variant_options(doc, set) {
            let mut v = json!({"name": k, "type": "variant", "options": vals, "default": defaults.get(&k).or(vals.first())});
            if values.is_some() {
                v["value"] = json!(current.get(&k));
            }
            list.push(v);
        }
    }
    // Slots: the frames (on a main) or this instance's copy, with whether it holds its own content.
    let slots = slots_of(doc, if values.is_some() { id } else { variant_set(doc, comp).unwrap_or(comp) });
    for (k, d) in prop_list(doc, comp) {
        let mut v = def_json(&k, &d);
        if !takes_value(&d) {
            let layers: Vec<NodeId> = slots.iter().filter(|(_, n)| *n == k).map(|(l, _)| *l).collect();
            if values.is_some() {
                v["value"] = layers.first().map_or(Value::Null, |l| {
                    let count = doc.node(*l).map_or(0, |n| n.children.len());
                    json!({"layer": l, "edited": slot_edited(doc, *l), "layers": count})
                });
            } else {
                v["layers"] = json!(layers);
            }
        } else if let Some(vals) = &values {
            v["value"] = vals.get(&k).cloned().unwrap_or_else(|| prop_default_value(&d));
        }
        list.push(v);
    }
    let set = variant_set(doc, comp);
    let conflicts = set.map(|s| variant_conflicts(doc, s)).unwrap_or_default();
    json!({"component": comp, "set": set, "properties": list, "conflicts": conflicts})
}
