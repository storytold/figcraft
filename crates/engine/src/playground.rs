//! The component playground: a component tried with other property values without placing it or
//! touching the document. [`build`] makes a copy of the document holding one extra instance with
//! the values; what shows it (the Inspect panel, `component.playground`) renders that instance and
//! generates its code.

use figcraft_doc::{Document, ExportFormat, NodeId, NodeKind};
use serde_json::{Map, Value, json};

use crate::cmd::CommandSpec;
use crate::codegen::CodeLang;
use crate::params::*;
use crate::{EngineError, Result, Session, bad, spec};

/// The component a playground for `target` shows: an instance's component, a component or a
/// component set itself. None for other layers.
pub fn component_of(doc: &Document, target: NodeId) -> Option<NodeId> {
    match &doc.node(target)?.kind {
        NodeKind::Instance { component, .. } => Some(*component),
        NodeKind::Component { .. } | NodeKind::ComponentSet { .. } => Some(target),
        _ => None,
    }
}

/// A copy of `doc` with a new instance of `target`'s component (an instance starts from its own
/// property values) set to `values` (property name → value, as `instance.setProperties` takes
/// them), and that instance's id. `doc` itself is untouched.
pub fn build(doc: &Document, target: NodeId, values: &Map<String, Value>) -> Result<(Document, NodeId)> {
    let cmd = "component.playground";
    let comp = component_of(doc, target).ok_or_else(|| bad(cmd, "select an instance, a component or a component set"))?;
    let mut s = Session::with_document(doc.clone());
    // Off to the side of everything (the copy is never shown as a page).
    let made = s.execute("instance.create", &json!({"component": comp, "x": 1e6, "y": 1e6}))?;
    let inst = id_of(&made["id"]).ok_or_else(|| EngineError::Other("the playground instance wasn't made".into()))?;
    let mut all = match &doc.node(target).map(|n| &n.kind) {
        Some(NodeKind::Instance { props, .. }) => props.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        _ => Map::new(),
    };
    all.extend(values.clone());
    if !all.is_empty() {
        s.execute("instance.setProperties", &json!({"ids": [inst], "properties": all})).map_err(|e| bad(cmd, e.to_string()))?;
    }
    Ok((s.doc()?.clone(), inst))
}

/// The properties of the playground's instance `inst` in `copy` (as `component.properties` lists
/// an instance's): `{component, set, properties: [{name, type, options?, value}]}`.
pub fn properties(copy: &Document, inst: NodeId) -> Value {
    crate::cmd::component::describe_props(copy, inst)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![spec!(
        "component.playground",
        "Component playground",
        "{id? (an instance, a component or a component set; default the selection), properties? {name: value} (variant, boolean, text and instance-swap properties, as instance.setProperties takes them), language? css|html|react|swiftui|compose|arkui (default react), path? (write the preview there as a PNG), scale? (the PNG's, default 2)} → {component, properties: [{name, type, options?, value}], language, code, width, height, path?}  the component with those property values, tried on a copy: its code and its look. Nothing in the document changes",
        false,
        |s, p| {
            let cmd = "component.playground";
            let target = targets(s, p).first().copied().ok_or_else(|| bad(cmd, "select an instance, a component or a component set"))?;
            let values = match get(p, "properties") {
                None | Some(Value::Null) => Map::new(),
                Some(Value::Object(m)) => m.clone(),
                Some(_) => return Err(bad(cmd, "properties: {name: value}")),
            };
            let lang = match string(p, "language") {
                Some(l) => CodeLang::from_id(&l).ok_or_else(|| bad(cmd, "language: css|html|react|swiftui|compose|arkui"))?,
                None => CodeLang::React,
            };
            let (copy, inst) = build(s.doc()?, target, &values)?;
            let n = copy.get(inst)?;
            let mut out = properties(&copy, inst);
            out["language"] = json!(lang.id());
            out["code"] = json!(crate::codegen::code_for(&copy, inst, lang));
            (out["width"], out["height"]) = (json!(n.width), json!(n.height));
            if let Some(path) = string(p, "path") {
                let scale = num(p, "scale").filter(|k| k.is_finite()).map_or(2.0, |k| k.clamp(0.01, 16.0));
                let png = crate::cmd::file::export_bytes(&copy, &[inst], ExportFormat::Png, scale)?;
                crate::file_access::write(&path, &png)?;
                out["path"] = json!(path);
            }
            Ok(out)
        }
    )]
}
