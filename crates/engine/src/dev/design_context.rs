//! Design context: what an AI agent needs to build a design, in one read (`inspect.designContext`,
//! the MCP tool `design_context`). The selection by default, as a tree in which every layer keeps
//! apart what the designer set (`explicit`), what layout or the file worked out (`derived`) and
//! what has no direct CSS equivalent (`unsupported`, with the reason). The variables, styles and
//! components the tree uses are joined once at the top level, and a connected instance gives its
//! code instead of its layers. Values are the file's own: nothing is guessed or converted, and
//! `missing` says what the design doesn't specify.

use std::collections::{BTreeMap, BTreeSet};

use figcraft_color::{GradientKind, PaintKind};
use figcraft_doc::{Document, EffectKind, Node, NodeId, NodeKind, Sizing, StrokeAlign, TextAutoResize};
use serde_json::{Map, Value, json};

use crate::cmd::CommandSpec;
use crate::cmd::component::describe_props;
use crate::code_connect;
use crate::codegen::CodeLang;
use crate::inspect::{camel, node_json};
use crate::params::{get, num, string, targets};
use crate::{Result, Session, bad, spec};

/// The format's version: fields may be added without changing it.
const VERSION: u64 = 1;
/// How deep the tree goes by default.
const DEFAULT_DEPTH: usize = 32;
/// The deepest tree asked for.
const MAX_DEPTH: usize = 128;
/// How many layers it holds by default.
const DEFAULT_NODES: usize = 2_000;
/// The most layers asked for.
const MAX_NODES: usize = 20_000;

/// Worked out rather than set, whatever the layer.
const DERIVED: [&str; 3] = ["absoluteBounds", "rotation", "gridCell"];

/// What the tree used, to join at the top level.
#[derive(Default)]
struct Used {
    variables: BTreeSet<String>,
    styles: BTreeSet<String>,
    components: BTreeSet<NodeId>,
    /// Layers placed so far, and those left out for the cap.
    nodes: usize,
    left_out: usize,
    /// Reached `depth` with children left.
    cut_by_depth: bool,
}

/// `property` of a layer has no direct CSS equivalent, because `why`.
fn limit(out: &mut Vec<Value>, property: &str, why: &str) {
    out.push(json!({"property": property, "why": why}));
}

/// What of `n` CSS can't say directly. The original values stay in `explicit`.
fn limits(n: &Node) -> Vec<Value> {
    let mut out = Vec::new();
    if matches!(n.blend, figcraft_color::BlendMode::PlusDarker) {
        limit(&mut out, "blend", "only some browsers have the plus-darker blend mode");
    }
    if n.corners.smoothing > 0.0 {
        limit(&mut out, "cornerSmoothing", "CSS border-radius has no corner smoothing");
    }
    if !n.strokes.is_empty() {
        match n.stroke.align {
            StrokeAlign::Inside => {}
            StrokeAlign::Center => limit(&mut out, "strokeAlign", "a CSS border can't straddle the box edge"),
            StrokeAlign::Outside => {
                limit(&mut out, "strokeAlign", "a CSS border sits inside the box's border area: an outline or box-shadow comes closest")
            }
        }
        if n.stroke.profile().is_some() {
            limit(&mut out, "widthProfile", "CSS strokes have one width");
        }
        if n.stroke.brush.is_some() {
            limit(&mut out, "brush", "CSS has no brush strokes");
        }
    }
    for p in n.fills.iter().chain(&n.strokes) {
        match &p.kind {
            PaintKind::Gradient(g) if g.kind == GradientKind::Diamond => limit(&mut out, "fills", "CSS has no diamond gradient"),
            PaintKind::Pattern(_) => limit(&mut out, "fills", "a pattern fill needs an image or an SVG pattern in CSS"),
            _ => {}
        }
    }
    for e in &n.effects {
        match e.kind {
            EffectKind::Noise(_) => limit(&mut out, "effects", "CSS has no noise effect"),
            EffectKind::Texture(_) => limit(&mut out, "effects", "CSS has no roughened edge effect"),
            EffectKind::Glass(_) => limit(&mut out, "effects", "CSS has no curved glass effect"),
            _ => {}
        }
    }
    if let NodeKind::Text(t) = &n.kind {
        if t.path.is_some() {
            limit(&mut out, "textPath", "text on a path needs SVG");
        }
        if t.leading_trim != figcraft_doc::LeadingTrim::None {
            limit(&mut out, "leadingTrim", "CSS text-box-trim isn't in every browser");
        }
        if t.hanging_punctuation {
            limit(&mut out, "hangingPunctuation", "CSS hanging-punctuation isn't in every browser");
        }
    }
    out
}

/// Moves `key` of `explicit` to `derived`.
fn derive(explicit: &mut Map<String, Value>, derived: &mut Map<String, Value>, key: &str) {
    if let Some(v) = explicit.remove(key) {
        derived.insert(key.into(), v);
    }
}

/// Splits a layer's properties: a size or a position that layout decides isn't the designer's.
fn split(doc: &Document, n: &Node, explicit: &mut Map<String, Value>) -> Map<String, Value> {
    let mut derived = Map::new();
    for key in DERIVED {
        derive(explicit, &mut derived, key);
    }
    let in_flow = n.parent.and_then(|p| doc.node(p)).is_some_and(|p| p.layout.is_some()) && !n.child_layout.absolute;
    if in_flow {
        derive(explicit, &mut derived, "x");
        derive(explicit, &mut derived, "y");
    }
    let (mut wide, mut tall) = (n.child_layout.sizing_h != Sizing::Fixed, n.child_layout.sizing_v != Sizing::Fixed);
    if let NodeKind::Text(t) = &n.kind {
        wide |= t.auto_resize == TextAutoResize::WidthAndHeight;
        tall |= matches!(t.auto_resize, TextAutoResize::WidthAndHeight | TextAutoResize::Height);
    }
    if wide {
        derive(explicit, &mut derived, "width");
    }
    if tall {
        derive(explicit, &mut derived, "height");
    }
    derived
}

/// What a layer's tokens are called: bindings by field, shared styles by kind, as they would be
/// looked up in the top-level `variables` and `styles`.
fn tokens(doc: &Document, n: &Node, used: &mut Used) -> Map<String, Value> {
    let mut out = Map::new();
    for (field, id) in &n.bound {
        used.variables.insert(id.clone());
        let name = doc.variables.variables.iter().find(|v| v.id == *id).map(|v| v.name.clone());
        out.insert(field.clone(), json!({"variable": id, "name": name}));
    }
    let refs = &n.styles;
    let mut styles: Vec<(String, &String)> =
        [("fill", &refs.fill), ("stroke", &refs.stroke), ("effect", &refs.effect), ("grid", &refs.grid), ("text", &refs.text)]
            .into_iter()
            .filter_map(|(k, s)| s.as_ref().map(|s| (format!("style.{k}"), s)))
            .collect();
    if let NodeKind::Text(t) = &n.kind {
        let spans = t.spans();
        styles.extend(spans.iter().filter_map(|(a, b, s)| s.style_id.as_ref().map(|i| (format!("style.text[{a}..{b}]"), i))));
    }
    for (key, id) in styles {
        used.styles.insert(id.clone());
        let name = doc.styles.iter().find(|s| s.id == *id).map(|s| s.name.clone());
        out.insert(key, json!({"style": id, "name": name}));
    }
    out
}

/// The code component a component, set or instance is connected to, per language, with the
/// instance's own values in its snippet.
fn code(doc: &Document, id: NodeId) -> Vec<Value> {
    let Some(owner) = code_connect::owner(doc, id).and_then(|o| doc.node(o)) else { return vec![] };
    owner
        .code_links
        .iter()
        .filter_map(|l| {
            let lang = CodeLang::from_id(&l.language)?;
            let snippet = code_connect::connected_code(doc, id, lang, l)?;
            Some(json!({"language": l.language, "importPath": l.import_path, "name": l.name, "code": snippet}))
        })
        .collect()
}

/// Layer `id` and, to `depth`, what's under it, while the cap holds.
fn layer(doc: &Document, id: NodeId, depth: usize, cap: usize, used: &mut Used) -> Option<Value> {
    let n = doc.node(id)?;
    used.nodes += 1;
    let Value::Object(mut explicit) = node_json(doc, id, 0) else { return None };
    explicit.remove("childCount");
    let mut out = Map::new();
    for key in ["id", "name", "type"] {
        if let Some(v) = explicit.remove(key) {
            out.insert(key.into(), v);
        }
    }
    let derived = split(doc, n, &mut explicit);
    let tokens = tokens(doc, n, used);
    let unsupported = limits(n);
    out.insert("explicit".into(), Value::Object(explicit));
    if !derived.is_empty() {
        out.insert("derived".into(), Value::Object(derived));
    }
    if !tokens.is_empty() {
        out.insert("tokens".into(), Value::Object(tokens));
    }
    if !unsupported.is_empty() {
        out.insert("unsupported".into(), Value::Array(unsupported));
    }
    if let Some(owner) = code_connect::owner(doc, id) {
        used.components.insert(owner);
    }
    let connected = code(doc, id);
    // An instance built from a code component needs its call, not its layers.
    let stop = matches!(n.kind, NodeKind::Instance { .. }) && !connected.is_empty();
    if !connected.is_empty() {
        out.insert("code".into(), Value::Array(connected));
    }
    if n.children.is_empty() {
        return Some(Value::Object(out));
    }
    if stop {
        out.insert("childCount".into(), json!(n.children.len()));
        return Some(Value::Object(out));
    }
    if depth == 0 {
        used.cut_by_depth = true;
        out.insert("childCount".into(), json!(n.children.len()));
        return Some(Value::Object(out));
    }
    let mut children = Vec::new();
    for c in &n.children {
        if used.nodes >= cap {
            used.left_out += 1;
            continue;
        }
        children.extend(layer(doc, *c, depth - 1, cap, used));
    }
    if children.len() < n.children.len() {
        out.insert("childCount".into(), json!(n.children.len()));
    }
    out.insert("children".into(), Value::Array(children));
    Some(Value::Object(out))
}

/// A value as the panels show it: colours in hex.
fn value_json(v: &figcraft_doc::VariableValue) -> Value {
    match v {
        figcraft_doc::VariableValue::Color(c) => json!(c.to_css_hex()),
        figcraft_doc::VariableValue::Number(n) => json!(n),
        figcraft_doc::VariableValue::String(s) => json!(s),
        figcraft_doc::VariableValue::Boolean(b) => json!(b),
        other => serde_json::to_value(other).unwrap_or(Value::Null),
    }
}

/// The variables the tree uses: every mode's stored and resolved value. A value that is an alias
/// or a calculation stays in `value`, and aliased variables are listed too, in `aliases`.
fn variables_json(doc: &Document, ids: &BTreeSet<String>, missing: &mut Vec<Value>) -> Value {
    let store = &doc.variables;
    let mut out = Map::new();
    for id in ids {
        let Some(var) = store.variables.iter().find(|v| v.id == *id) else {
            missing.push(json!({"what": "variable", "id": id, "why": "a layer is bound to it, but it isn't in this file"}));
            continue;
        };
        let collection = store.collection_by_id(&var.collection);
        let modes: Vec<Value> = collection
            .map(|c| c.modes.as_slice())
            .unwrap_or_default()
            .iter()
            .map(|m| {
                let chosen = BTreeMap::from([(var.collection.clone(), m.id.clone())]);
                json!({
                    "mode": m.name,
                    "value": var.values.get(&m.id).map(value_json),
                    "resolved": store.resolve(id, &chosen).as_ref().map(value_json),
                })
            })
            .collect();
        let mut v = json!({"name": var.name, "collection": collection.map(|c| c.name.as_str()), "type": var.kind, "modes": modes});
        if !var.code_syntax.is_empty() {
            v["codeSyntax"] = json!(var.code_syntax);
        }
        if !var.description.is_empty() {
            v["description"] = json!(var.description);
        }
        out.insert(id.clone(), v);
    }
    Value::Object(out)
}

/// The shared styles the tree uses, with what each sets.
fn styles_json(doc: &Document, ids: &BTreeSet<String>, missing: &mut Vec<Value>) -> Value {
    let mut out = Map::new();
    for id in ids {
        match doc.styles.iter().find(|s| s.id == *id) {
            Some(st) => {
                out.insert(id.clone(), camel(serde_json::to_value(st).unwrap_or(Value::Null)));
            }
            None => missing.push(json!({"what": "style", "id": id, "why": "a layer links to it, but it isn't in this file"})),
        }
    }
    Value::Object(out)
}

/// The components and sets the tree uses: their properties, variants and code.
fn components_json(doc: &Document, ids: &BTreeSet<NodeId>) -> Value {
    let mut out = Map::new();
    for id in ids {
        let Some(n) = doc.node(*id) else { continue };
        let mut v = json!({"name": n.name, "type": n.kind.type_name(), "properties": describe_props(doc, *id)["properties"]});
        if let NodeKind::Component { description, .. } = &n.kind
            && !description.is_empty()
        {
            v["description"] = json!(description);
        }
        if !n.code_links.is_empty() {
            v["codeLinks"] = camel(serde_json::to_value(&n.code_links).unwrap_or(Value::Null));
        }
        out.insert(id.to_string(), v);
    }
    Value::Object(out)
}

/// The font families under `ids`' tree, with their styles.
fn fonts(doc: &Document, roots: &[NodeId], cap: usize) -> Value {
    let mut used: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut stack: Vec<NodeId> = roots.to_vec();
    let mut seen = 0usize;
    while let Some(id) = stack.pop() {
        seen += 1;
        if seen > cap {
            break;
        }
        let Some(n) = doc.node(id) else { continue };
        if let NodeKind::Text(t) = &n.kind {
            for (_, _, s) in t.spans() {
                used.entry(s.font_family.clone()).or_default().insert(s.font_style.clone());
            }
        }
        stack.extend(n.children.iter().copied());
    }
    let gone = crate::fonts::missing(doc);
    Value::Array(
        used.into_iter()
            .map(|(family, styles)| {
                let installed = !gone.iter().any(|m| m.family.eq_ignore_ascii_case(&family));
                json!({"family": family, "styles": styles, "installed": installed})
            })
            .collect(),
    )
}

/// What the request names: (where it came from, the layers).
fn roots(s: &Session, p: &Value) -> Result<(&'static str, Vec<NodeId>)> {
    let cmd = "inspect.designContext";
    let doc = s.doc()?;
    if get(p, "ids").is_some() || get(p, "id").is_some() {
        let ids: Vec<NodeId> = targets(s, p);
        if let Some(gone) = ids.iter().find(|i| !doc.contains(**i)) {
            return Err(bad(cmd, format!("no layer {gone}")));
        }
        return Ok(("ids", ids));
    }
    let page = |pg: NodeId| doc.node(pg).map(|n| n.children.clone()).unwrap_or_default();
    match string(p, "scope").as_deref() {
        None | Some("selection") => match s.selection() {
            ids if ids.is_empty() => Ok(("page", page(s.page()?))),
            ids => Ok(("selection", ids)),
        },
        Some("page") => Ok(("page", page(s.page()?))),
        Some("document") => Ok(("document", doc.pages.iter().flat_map(|pg| page(*pg)).collect())),
        Some(_) => Err(bad(cmd, "scope: selection|page|document")),
    }
}

/// A whole-number parameter within `1..=max`.
fn limit_param(p: &Value, key: &str, default: usize, max: usize) -> Result<usize> {
    match get(p, key) {
        None | Some(Value::Null) => Ok(default),
        Some(_) => match num(p, key).filter(|v| v.is_finite() && *v >= 0.0) {
            Some(v) => Ok((v as usize).min(max)),
            None => Err(bad("inspect.designContext", format!("{key}: a whole number"))),
        },
    }
}

fn design_context(s: &mut Session, p: &Value) -> Result<Value> {
    let (source, ids) = roots(s, p)?;
    let depth = limit_param(p, "depth", DEFAULT_DEPTH, MAX_DEPTH)?;
    let cap = limit_param(p, "maxNodes", DEFAULT_NODES, MAX_NODES)?.max(1);
    let doc = s.doc()?;
    let mut used = Used::default();
    let mut nodes = Vec::new();
    for id in &ids {
        if used.nodes >= cap {
            used.left_out += 1;
            continue;
        }
        nodes.extend(layer(doc, *id, depth, cap, &mut used));
    }
    let mut missing = vec![
        json!({"what": "responsive behaviour", "why": "the file stores no breakpoints: sizes are as designed here, and constraints, sizing and annotations are all it says about other widths"}),
    ];
    let variables = variables_json(doc, &used.variables, &mut missing);
    let styles = styles_json(doc, &used.styles, &mut missing);
    let components = components_json(doc, &used.components);
    let fonts = fonts(doc, &ids, cap.saturating_mul(4));
    let page = doc.node(s.page()?).map(|n| n.name.clone());
    let mut out = json!({
        "version": VERSION,
        "document": {"name": doc.name, "page": {"id": s.page()?, "name": page}, "colorProfile": doc.color_profile.id()},
        "target": {"source": source, "ids": ids, "layers": used.nodes},
        "nodes": nodes,
        "variables": variables,
        "styles": styles,
        "components": components,
        "fonts": fonts,
        "assets": crate::devinspect::assets(doc, &ids),
        "missing": missing,
    });
    if used.left_out > 0 || used.cut_by_depth {
        out["truncated"] = json!({"layersLeftOut": used.left_out, "depth": used.cut_by_depth, "maxNodes": cap, "hint": "call again with the ids of the parts you need"});
    }
    Ok(out)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![spec!(
        "inspect.designContext",
        "Design context",
        "{ids? | id? (those layers), scope?: selection|page|document (default the selection, or the page with nothing selected), depth? (default 32), maxNodes? (default 2000)}  everything needed to build them, in one read  → {version, document, target {source, ids, layers}, nodes [{id, name, type, explicit, derived?, tokens?, unsupported? [{property, why}], code?, children?, childCount?}], variables, styles, components, fonts, assets, missing [{what, why}], truncated?}",
        false,
        design_context
    )]
}
