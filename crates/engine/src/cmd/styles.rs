//! Shared styles (paint, text, effect, grid) and variables (collections, modes, bindings).

use std::collections::BTreeMap;

use figcraft_color::{Color, Paint, PaintKind};
use figcraft_doc::{
    Collection, CollectionId, Document, Effect, EffectKind, Mode, ModeId, Node, NodeId, NodeKind, Style, StyleId, StyleKind, TextData, TextStyle,
    Variable, VariableId, VariableType, VariableValue, Variables, trim_num,
};
use serde_json::{Value, json};

use super::{CommandSpec, ok};
use crate::params::{color_of, f64_of, get, id, num, paints_of, req_string, req_targets, string, targets};
use crate::{EngineError, Result, Session, bad, spec};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "paint.documentColors",
            "Document colours",
            "{page?, limit? (default 21)} → solid colours used on the page, most used first",
            false,
            |s, p| {
                let page = id(p, "page").unwrap_or(s.page()?);
                let limit = num(p, "limit").map(|l| l.clamp(1.0, 500.0) as usize).unwrap_or(21);
                let colors = document_colors(s.doc()?, page);
                let out: Vec<Value> = colors.into_iter().take(limit).map(|(c, n)| json!({"color": c.to_hex(), "count": n})).collect();
                Ok(json!({"colors": out}))
            }
        ),
        spec!(
            "style.create",
            "Create style",
            "{type: paint|text|effect|grid, name, from? (node id; default first selected), description?, value? (paints/effects/grids/text style JSON)}",
            true,
            create
        ),
        spec!(
            "style.apply",
            "Apply style",
            "{ids?, style (id or name), target? (fill|stroke; paint styles), start?, end? (text styles: these chars; while editing text, the selected chars)}",
            true,
            apply
        ),
        spec!(
            "style.detach",
            "Detach style",
            "{ids?, which: fill|stroke|effect|grid|text, start?, end? (text: these chars; while editing text, the selected chars)}",
            true,
            detach
        ),
        spec!("style.update", "Edit style", "{style, name?, description?, value?, from? (node id: redefine from it)}", true, update),
        spec!(
            "style.bind",
            "Bind variable in style",
            "{style, field, variable? (id or name; none: unbind)}  bind a variable to a field of the style's value: a paint style's `fills/<i>/color` (or `fill`: its first paint) and `fills/<i>/stops/<j>/color`, an effect style's `effects/<i>/<color|radius|spread|offsetX|offsetY|startRadius>`, a grid style's `layoutGrids/<i>/<size|count|gutter|offset>`, a text style's `fontSize|lineHeight|letterSpacing` (numbers, px) and `fontFamily|fontStyle` (strings). Every layer using the style shows the field resolved in its own modes; the style keeps it resolved in the default modes",
            true,
            bind_style
        ),
        spec!("style.duplicate", "Duplicate style", "{style} → {id} (right below it, named \"<name> copy\")", true, duplicate),
        spec!(
            "style.move",
            "Move styles",
            "{styles [ids], before? (style id; default: the end), folder? (\"Brand/Blog\"; \"\" = no folder: the styles are renamed into it)}",
            true,
            move_styles
        ),
        spec!(
            "style.renameFolder",
            "Rename style folder",
            "{folder (\"Brand/Blog\"), name (\"Brand/News\")} (renames the styles in it)",
            true,
            rename_folder
        ),
        spec!("style.delete", "Delete style", "{style}", true, |s, p| {
            let id = req_string("style.delete", p, "style")?;
            let doc = s.doc_mut()?;
            let sid = doc.style(&id).map(|st| st.id.clone()).ok_or_else(|| bad("style.delete", "no such style"))?;
            if crate::library::is_remote_style(doc, &sid) {
                return Err(EngineError::Disabled("style.delete".into(), crate::library::READ_ONLY.into()));
            }
            doc.styles.retain(|st| st.id != sid);
            // Nodes (and text chars) keep their values but lose the link.
            for nid in doc.ids().collect::<Vec<_>>() {
                if let Some(n) = doc.node(nid)
                    && ([&n.styles.fill, &n.styles.stroke, &n.styles.effect, &n.styles.grid, &n.styles.text]
                        .iter()
                        .any(|r| r.as_deref() == Some(sid.as_str()))
                        || n.kind.text().is_some_and(|t| t.links_text_style(&sid)))
                {
                    let n = doc.get_mut(nid)?;
                    for r in [&mut n.styles.fill, &mut n.styles.stroke, &mut n.styles.effect, &mut n.styles.grid, &mut n.styles.text] {
                        if r.as_deref() == Some(sid.as_str()) {
                            *r = None;
                        }
                    }
                    if let NodeKind::Text(t) = &mut n.kind {
                        t.map_text_styles(|x| (x != sid).then(|| x.to_string()));
                    }
                }
            }
            ok()
        }),
        spec!("style.list", "List styles", "{type?}", false, |s, p| {
            let t = string(p, "type");
            Ok(Value::Array(
                s.doc()?
                    .styles
                    .iter()
                    .filter(|st| t.as_deref().is_none_or(|t| st.kind.type_name() == t))
                    .map(|st| serde_json::to_value(st).unwrap_or(Value::Null))
                    .collect(),
            ))
        }),
        spec!("variable.createCollection", "Create variable collection", "{name, modes? [names]}", true, |s, p| {
            let cmd = "variable.createCollection";
            let name = req_string(cmd, p, "name")?.trim().to_string();
            if name.is_empty() {
                return Err(bad(cmd, "name can't be empty"));
            }
            let modes: Vec<String> = match get(p, "modes") {
                Some(Value::Array(a)) => a.iter().filter_map(|m| m.as_str().map(|m| m.trim().to_string())).collect(),
                _ => vec!["Mode 1".into()],
            };
            if modes.is_empty() {
                return Err(bad(cmd, "a collection needs at least one mode"));
            }
            if modes.iter().any(String::is_empty) {
                return Err(bad(cmd, "mode names can't be empty"));
            }
            let doc = s.doc_mut()?;
            if doc.variables.collections.iter().any(|c| c.name == name) {
                return Err(bad(cmd, format!("a collection named `{name}` already exists")));
            }
            let id = doc.new_collection_id();
            let modes = modes.into_iter().enumerate().map(|(i, m)| Mode { id: format!("{id}/mode{}", i + 1), name: m }).collect();
            doc.variables.collections.push(Collection { id: id.clone(), name, modes, extends: None });
            Ok(json!({"id": id}))
        }),
        spec!("variable.addMode", "Add mode", "{collection, name, from? (mode id or name to duplicate; default: the default mode)}", true, |s, p| {
            let c = req_string("variable.addMode", p, "collection")?;
            let name = req_string("variable.addMode", p, "name")?;
            let missing = || bad("variable.addMode", "no such collection");
            let doc = s.doc_mut()?;
            let cid = doc.variables.collections.iter().find(|x| x.id == c || x.name == c).map(|x| x.id.clone()).ok_or_else(missing)?;
            super::variables::check_modes_editable(doc, "variable.addMode", &cid)?;
            // A fresh id (modes can be deleted, so the count alone may repeat one; in a live
            // session it comes from this client's id space).
            let mid = doc.new_mode_id(&cid).ok_or_else(missing)?;
            let coll = doc.variables.collections.iter_mut().find(|x| x.id == cid).ok_or_else(missing)?;
            let first = match string(p, "from") {
                Some(f) => Some(coll.find_mode(&f).map(|m| m.id.clone()).ok_or_else(|| bad("variable.addMode", format!("no mode `{f}`")))?),
                None => coll.modes.first().map(|m| m.id.clone()),
            };
            coll.modes.push(Mode { id: mid.clone(), name });
            // New mode starts with the default mode's values (a duplicate with its source's).
            for v in doc.variables.variables.iter_mut().filter(|v| v.collection == cid) {
                if let Some(val) = first.as_ref().and_then(|f| v.values.get(f)).cloned() {
                    v.values.insert(mid.clone(), val);
                }
            }
            Ok(json!({"id": mid}))
        }),
        spec!(
            "variable.create",
            "Create variable",
            "{collection, name, type: color|number|string|boolean, value (default mode) | values {modeName: value}, description?}",
            true,
            var_create
        ),
        spec!(
            "variable.set",
            "Set variable value",
            "{variable, mode? (name or id; default first), value | {alias: variable} (same type, no loops)}",
            true,
            var_set
        ),
        spec!(
            "variable.delete",
            "Delete variable",
            "{variable} (bound layers keep their values; variables aliasing it take its values)",
            true,
            |s, p| {
                let v = req_string("variable.delete", p, "variable")?;
                let doc = s.doc_mut()?;
                let var = doc.variables.get(&v).ok_or_else(|| bad("variable.delete", "no such variable"))?;
                if var.extends.is_some() {
                    return Err(bad("variable.delete", "an inherited variable stays while the collection it extends has it"));
                }
                // Bound layers keep their values; aliases to it take its values.
                let gone = std::collections::BTreeSet::from([var.id.clone()]);
                super::variables::forget_variables(doc, &gone)?;
                ok()
            }
        ),
        spec!(
            "variable.bind",
            "Bind variable",
            "{ids?, field (fill|stroke (first paint)|fills/<i>/color|strokes/<i>/color|fills/<i>/stops/<j>/color (gradient stops)|effects/<i>/<offsetX|offsetY|radius|startRadius|spread|color>|layoutGrids/<i>/<size|count|gutter|offset>|componentProperties/<name> (instances: boolean/text properties; variant properties take string variables naming a value)|width|height|opacity (percent)|cornerRadius|gap|paddingHorizontal|characters|visible|fontSize…; list indices bottom first), variable (null unbinds)}",
            true,
            bind
        ),
        spec!(
            "variable.setMode",
            "Set variable mode",
            "{ids? (frames/page), collection, mode (id or name; null: Auto, inherited from the parent)}",
            true,
            set_mode
        ),
        spec!("variable.list", "List variables", "{}", false, |s, _| Ok(serde_json::to_value(&s.doc()?.variables).unwrap_or(Value::Null))),
    ]
}

/// A text layer and the chars a command covers in it (None = all of them).
pub(crate) type TextTarget = (NodeId, Option<(usize, usize)>);

/// Text layers a text style command acts on, each with the chars it covers (None = the whole
/// layer): chars `start..end` when given, else the selected chars of the text being edited (when
/// it is the target), else every targeted layer.
pub(crate) fn text_targets(cmd: &str, s: &Session, p: &Value) -> Result<Vec<TextTarget>> {
    let at = |k: &str| num(p, k).filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 1e12) as usize);
    let (start, end) = (at("start"), at("end"));
    if start.is_some() || end.is_some() {
        let range = (start.unwrap_or(0), end.unwrap_or(usize::MAX));
        return Ok(req_targets(cmd, s, p)?.into_iter().map(|i| (i, Some(range))).collect());
    }
    let given = targets(s, p);
    if let Some((id, a, b)) = crate::textedit::selected_range(s)
        && a < b
        && given == [id]
    {
        return Ok(vec![(id, Some((a, b)))]);
    }
    Ok(req_targets(cmd, s, p)?.into_iter().map(|i| (i, None)).collect())
}

/// A text style definition from JSON (`fontSize` or `font_size` keys; line height and letter
/// spacing also as for `node.set`: "auto", 24, "150%", "2%").
fn text_style_json(cmd: &str, v: &Value) -> Result<TextStyle> {
    let snake =
        |k: &str| k.chars().flat_map(|c| if c.is_ascii_uppercase() { vec!['_', c.to_ascii_lowercase()] } else { vec![c] }).collect::<String>();
    let friendly = |k: &str, x: &Value| -> Value {
        let parsed = match (k, x) {
            (_, Value::Object(_)) => None,
            ("line_height", x) => super::props::line_height(x).and_then(|l| serde_json::to_value(l).ok()),
            ("letter_spacing", x) => super::props::letter_spacing(x).and_then(|l| serde_json::to_value(l).ok()),
            _ => None,
        };
        parsed.unwrap_or_else(|| x.clone())
    };
    let v = match v {
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (snake(k), friendly(&snake(k), x))).collect()),
        other => other.clone(),
    };
    serde_json::from_value::<TextStyle>(v).map(|t| t.as_text_style()).map_err(|e| bad(cmd, e.to_string()))
}

/// The text style definition a layer's chars from `at` (None = its base style) show.
fn text_style_of(n: &Node, at: Option<usize>) -> Option<TextStyle> {
    let t = n.kind.text()?;
    Some(at.map_or(&t.style, |a| t.style_at(a)).as_text_style())
}

fn detach(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "style.detach";
    let which = req_string(cmd, p, "which")?;
    if which == "text" {
        for (i, range) in text_targets(cmd, s, p)? {
            let doc = s.doc_mut()?;
            match range {
                Some((a, b)) => {
                    if let NodeKind::Text(t) = &mut doc.get_mut(i)?.kind {
                        t.unlink_text_style(a, b);
                    }
                }
                None => apply_style_ref(doc, i, "textStyle", None)?,
            }
        }
        return ok();
    }
    for i in req_targets(cmd, s, p)? {
        let n = s.doc_mut()?.get_mut(i)?;
        match which.as_str() {
            "fill" => n.styles.fill = None,
            "stroke" => n.styles.stroke = None,
            "effect" => n.styles.effect = None,
            "grid" => n.styles.grid = None,
            _ => return Err(bad(cmd, "which: fill|stroke|effect|grid|text")),
        }
    }
    ok()
}

fn kind_from_node(t: &str, n: &Node) -> Option<StyleKind> {
    Some(match t {
        "paint" | "color" | "fill" => StyleKind::Paint { paints: n.fills.clone() },
        "stroke" => StyleKind::Paint { paints: n.strokes.clone() },
        "text" => StyleKind::Text { style: text_style_of(n, None)? },
        "effect" => StyleKind::Effect { effects: n.effects.clone() },
        "grid" => StyleKind::Grid { grids: n.grids.clone() },
        _ => return None,
    })
}

fn create(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "style.create";
    let t = req_string(cmd, p, "type")?.to_ascii_lowercase();
    let name = style_name(cmd, &req_string(cmd, p, "name")?)?;
    let from = id(p, "from").or_else(|| s.selection().first().copied());
    // A text style made while editing text takes (and links) the selected chars.
    let range = match (t.as_str(), crate::textedit::selected_range(s)) {
        ("text", Some((tid, a, b))) if a < b && from.is_none_or(|f| f == tid) && get(p, "value").is_none() => Some((tid, a, b)),
        _ => None,
    };
    let from = range.map(|r| r.0).or(from);
    let doc = s.doc_mut()?;
    let kind = if let Some(v) = get(p, "value") {
        match t.as_str() {
            "paint" | "color" | "fill" => StyleKind::Paint { paints: paints_of(v).ok_or_else(|| bad(cmd, "value: paints"))? },
            "text" => StyleKind::Text { style: text_style_json(cmd, v)? },
            "effect" => StyleKind::Effect { effects: serde_json::from_value(v.clone()).map_err(|e| bad(cmd, e.to_string()))? },
            "grid" => StyleKind::Grid { grids: serde_json::from_value(v.clone()).map_err(|e| bad(cmd, e.to_string()))? },
            _ => return Err(bad(cmd, "type: paint|text|effect|grid")),
        }
    } else if let Some((tid, a, _)) = range {
        StyleKind::Text { style: doc.node(tid).and_then(|n| text_style_of(n, Some(a))).ok_or_else(|| bad(cmd, "not a text layer"))? }
    } else {
        let n = from.and_then(|f| doc.node(f)).ok_or_else(|| bad(cmd, "pass `value` or select a layer to take it from"))?;
        kind_from_node(&t, n).ok_or_else(|| bad(cmd, "that layer has no such property"))?
    };
    let sid = doc.new_style_id();
    let field = match t.as_str() {
        "stroke" => "strokeStyle",
        "text" => "textStyle",
        "effect" => "effectStyle",
        "grid" => "gridStyle",
        _ => "fillStyle",
    };
    // A style made from a layer keeps the variables bound to what it takes.
    let bound = match from.and_then(|f| doc.node(f)).filter(|_| get(p, "value").is_none() && range.is_none()) {
        Some(n) => crate::style_vars::from_layer(n, field, &kind),
        None => BTreeMap::new(),
    };
    doc.styles.push(Style { id: sid.clone(), name, description: string(p, "description").unwrap_or_default(), kind: kind.clone(), bound });
    // The source layer (or the selected chars) becomes linked to the new style.
    if let (Some((tid, a, b)), StyleKind::Text { style }) = (range, &kind) {
        if let NodeKind::Text(td) = &mut doc.get_mut(tid)?.kind {
            td.link_text_style(a, b, style, &sid);
        }
    } else if let Some(f) = from
        && get(p, "value").is_none()
    {
        apply_style_ref(doc, f, field, Some(&sid))?;
    }
    Ok(json!({"id": sid}))
}

/// Link node `id` to style `sid` for `field` (fillStyle, strokeStyle, …) and copy its value in.
pub fn apply_style_ref(doc: &mut Document, id: NodeId, field: &str, sid: Option<&str>) -> Result<()> {
    let style = match sid {
        Some(s) => {
            let st = doc.style(s).ok_or_else(|| EngineError::Other(format!("no such style `{s}`")))?;
            // Fields bound to variables come resolved in this layer's modes.
            Some(Style { kind: crate::style_vars::kind_for(doc, st, id).into_owned(), ..st.clone() })
        }
        None => None,
    };
    let n = doc.get_mut(id)?;
    // The style's values replace variables bound to what it sets.
    if let Some(st) = &style {
        n.bound.retain(|k, _| !style_sets(field, &st.kind, k));
    }
    match (field, &style) {
        ("fillStyle", Some(Style { kind: StyleKind::Paint { paints }, id, .. })) => {
            n.fills = paints.clone();
            n.styles.fill = Some(id.clone());
        }
        ("strokeStyle", Some(Style { kind: StyleKind::Paint { paints }, id, .. })) => {
            n.strokes = paints.clone();
            n.styles.stroke = Some(id.clone());
        }
        ("effectStyle", Some(Style { kind: StyleKind::Effect { effects }, id, .. })) => {
            n.effects = effects.clone();
            n.styles.effect = Some(id.clone());
        }
        ("gridStyle", Some(Style { kind: StyleKind::Grid { grids }, id, .. })) => {
            n.grids = grids.clone();
            n.styles.grid = Some(id.clone());
        }
        ("textStyle", Some(Style { kind: StyleKind::Text { style }, id, .. })) => {
            // Every char takes the style's typography (their fills and links stay).
            if let NodeKind::Text(t) = &mut n.kind {
                let len = t.char_count();
                t.link_text_style(0, len, style, id);
            }
            n.styles.text = Some(id.clone());
        }
        ("fillStyle", None) => n.styles.fill = None,
        ("strokeStyle", None) => n.styles.stroke = None,
        ("effectStyle", None) => n.styles.effect = None,
        ("gridStyle", None) => n.styles.grid = None,
        ("textStyle", None) => {
            if let NodeKind::Text(t) = &mut n.kind {
                let len = t.char_count();
                t.unlink_text_style(0, len);
            }
            n.styles.text = None;
        }
        _ => return Err(EngineError::Other(format!("style doesn't fit `{field}`"))),
    }
    Ok(())
}

pub(crate) fn apply(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "style.apply";
    let st = req_string(cmd, p, "style")?;
    let style = s.doc()?.style(&st).cloned().ok_or_else(|| bad(cmd, format!("no style `{st}`")))?;
    // Text styles go on chars: a range, the selected chars of the edited text, or whole layers.
    if let StyleKind::Text { style: def } = &style.kind {
        // A text style is no property of other layers: they're left alone.
        let doc = s.doc()?;
        let targets: Vec<TextTarget> =
            text_targets(cmd, s, p)?.into_iter().filter(|(i, _)| doc.node(*i).is_some_and(|n| n.kind.text().is_some())).collect();
        if targets.is_empty() {
            return Err(bad(cmd, "a text style goes on text layers"));
        }
        for (i, range) in targets {
            let doc = s.doc_mut()?;
            match range {
                Some((a, b)) => {
                    if let NodeKind::Text(t) = &mut doc.get_mut(i)?.kind {
                        t.link_text_style(a, b, def, &style.id);
                    }
                }
                None => apply_style_ref(doc, i, "textStyle", Some(&style.id))?,
            }
        }
        return ok();
    }
    let ids = req_targets(cmd, s, p)?;
    let doc = s.doc_mut()?;
    let field = match (&style.kind, string(p, "target").as_deref()) {
        (StyleKind::Paint { .. }, Some("stroke")) => "strokeStyle",
        (StyleKind::Paint { .. }, _) => "fillStyle",
        (StyleKind::Text { .. }, _) => "textStyle",
        (StyleKind::Effect { .. }, _) => "effectStyle",
        (StyleKind::Grid { .. }, _) => "gridStyle",
    };
    for i in ids {
        apply_style_ref(doc, i, field, Some(&style.id))?;
    }
    ok()
}

fn bind_style(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "style.bind";
    let st = req_string(cmd, p, "style")?;
    let field = req_string(cmd, p, "field")?;
    let doc = s.doc_mut()?;
    let idx = doc.styles.iter().position(|x| x.id == st || x.name == st).ok_or_else(|| bad(cmd, "no such style"))?;
    if doc.styles.get(idx).is_some_and(|x| crate::library::is_remote_style(doc, &x.id)) {
        return Err(EngineError::Disabled(cmd.into(), crate::library::READ_ONLY.into()));
    }
    let var = match string(p, "variable") {
        Some(v) => Some(doc.variables.get(&v).map(|x| (x.id.clone(), x.kind)).ok_or_else(|| bad(cmd, "no such variable"))?),
        None => None,
    };
    let Some(style) = doc.styles.get_mut(idx) else { return Err(bad(cmd, "no such style")) };
    let want = crate::style_vars::field_type(&style.kind, &field).ok_or_else(|| bad(cmd, format!("the style has no field `{field}` to bind")))?;
    let key = crate::style_vars::canonical(&field);
    match var {
        Some((_, kind)) if kind != want => return Err(bad(cmd, format!("`{field}` takes a {} variable, not a {} one", want.id(), kind.id()))),
        Some((id, _)) => {
            if style.bound.get(&key) != Some(&id)
                && let StyleKind::Paint { paints } = &mut style.kind
            {
                rebind_paint_alpha(paints, &mut [], &key, true);
            }
            style.bound.insert(key, id);
        }
        None => {
            if style.bound.remove(&key).is_some()
                && let StyleKind::Paint { paints } = &mut style.kind
            {
                rebind_paint_alpha(paints, &mut [], &key, false);
            }
        }
    }
    // The style takes its default-mode value, and each layer using it its own.
    let sid = style.id.clone();
    crate::style_vars::settle(doc);
    propagate_style(doc, &sid);
    ok()
}

fn update(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "style.update";
    let st = req_string(cmd, p, "style")?;
    let doc = s.doc_mut()?;
    let idx = doc.styles.iter().position(|x| x.id == st || x.name == st).ok_or_else(|| bad(cmd, "no such style"))?;
    if doc.styles.get(idx).is_some_and(|x| crate::library::is_remote_style(doc, &x.id)) {
        return Err(EngineError::Disabled(cmd.into(), crate::library::READ_ONLY.into()));
    }
    let from = id(p, "from");
    let new_kind = if let Some(v) = get(p, "value") {
        let cur = doc.styles.get(idx).map(|x| x.kind.type_name()).unwrap_or("paint");
        Some(match cur {
            "paint" => StyleKind::Paint { paints: paints_of(v).ok_or_else(|| bad(cmd, "value: paints"))? },
            "text" => StyleKind::Text { style: text_style_json(cmd, v)? },
            "effect" => StyleKind::Effect { effects: serde_json::from_value(v.clone()).map_err(|e| bad(cmd, e.to_string()))? },
            _ => StyleKind::Grid { grids: serde_json::from_value(v.clone()).map_err(|e| bad(cmd, e.to_string()))? },
        })
    } else if let Some(f) = from {
        let t = doc.styles.get(idx).map(|x| x.kind.type_name()).unwrap_or("paint");
        doc.node(f).and_then(|n| kind_from_node(t, n))
    } else {
        None
    };
    let name = string(p, "name").map(|n| style_name(cmd, &n)).transpose()?;
    let Some(style) = doc.styles.get_mut(idx) else { return Err(bad(cmd, "no such style")) };
    if let Some(n) = name {
        style.name = n;
    }
    if let Some(d) = string(p, "description") {
        style.description = d;
    }
    if let Some(k) = new_kind {
        style.kind = k;
        crate::style_vars::prune(style);
    }
    let sid = style.id.clone();
    propagate_style(doc, &sid);
    ok()
}

/// Whether a `field` link (fillStyle, …) to a style of `kind` sets the property bound as `key`.
fn style_sets(field: &str, kind: &StyleKind, key: &str) -> bool {
    use super::lists::{BoundList, list_binding};
    let list = match (field, kind) {
        ("fillStyle", _) => BoundList::Fills,
        ("strokeStyle", _) => BoundList::Strokes,
        ("effectStyle", _) => BoundList::Effects,
        ("gridStyle", _) => BoundList::Grids,
        (_, StyleKind::Text { .. }) => {
            return matches!(
                key,
                "fontFamily" | "fontStyle" | "fontWeight" | "fontSize" | "lineHeight" | "letterSpacing" | "paragraphSpacing" | "paragraphIndent"
            );
        }
        _ => return false,
    };
    list_binding(key).is_some_and(|(l, _, _)| l == list)
}

/// A style name with each `/`-separated folder part trimmed; empty names are refused.
fn style_name(cmd: &str, name: &str) -> Result<String> {
    super::variables::clean_name(name).ok_or_else(|| bad(cmd, "name can't be empty"))
}

/// The index of local style `st` (id or name); library styles are read-only.
fn local_style_index(doc: &Document, cmd: &str, st: &str) -> Result<usize> {
    let idx = doc.styles.iter().position(|x| x.id == st || x.name == st).ok_or_else(|| bad(cmd, format!("no style `{st}`")))?;
    match doc.styles.get(idx) {
        Some(x) if crate::library::is_remote_style(doc, &x.id) => Err(EngineError::Disabled(cmd.into(), crate::library::READ_ONLY.into())),
        _ => Ok(idx),
    }
}

fn duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "style.duplicate";
    let doc = s.doc_mut()?;
    let idx = local_style_index(doc, cmd, &req_string(cmd, p, "style")?)?;
    let mut copy = doc.styles.get(idx).cloned().ok_or_else(|| bad(cmd, "no such style"))?;
    let base = format!("{} copy", copy.name);
    copy.name = (1..1000)
        .map(|k| if k == 1 { base.clone() } else { format!("{base} {k}") })
        .find(|n| !doc.styles.iter().any(|x| x.name == *n))
        .unwrap_or(base);
    copy.id = doc.new_style_id();
    let id = copy.id.clone();
    doc.styles.insert(idx + 1, copy);
    Ok(json!({"id": id}))
}

/// `name` moved into `folder` ("" = none): its last part under the folder.
fn in_folder(folder: &str, name: &str) -> String {
    let leaf = name.rsplit('/').next().unwrap_or(name);
    if folder.is_empty() { leaf.to_string() } else { format!("{folder}/{leaf}") }
}

fn move_styles(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "style.move";
    let doc = s.doc_mut()?;
    let keys: Vec<&str> = match get(p, "styles") {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
        _ => return Err(bad(cmd, "`styles` must be a list of style ids")),
    };
    let mut picked: Vec<usize> = keys.iter().map(|k| local_style_index(doc, cmd, k)).collect::<Result<_>>()?;
    if picked.is_empty() {
        return Err(bad(cmd, "no styles to move"));
    }
    picked.sort_unstable();
    picked.dedup();
    let folder = match string(p, "folder") {
        Some(f) if f.split('/').all(|x| x.trim().is_empty()) => Some(String::new()),
        Some(f) => Some(style_name(cmd, &f)?),
        None => None,
    };
    let before = match string(p, "before") {
        Some(b) => Some(doc.style(&b).map(|x| x.id.clone()).ok_or_else(|| bad(cmd, format!("no style `{b}`")))?),
        None => None,
    };
    // Take the moved styles out (keeping their order), then put them back before `before` (or last).
    let mut moved = Vec::with_capacity(picked.len());
    for i in picked.into_iter().rev() {
        moved.push(doc.styles.remove(i));
    }
    moved.reverse();
    if let Some(f) = &folder {
        for st in &mut moved {
            st.name = in_folder(f, &st.name);
        }
    }
    let at = before.and_then(|b| doc.styles.iter().position(|x| x.id == b)).unwrap_or(doc.styles.len());
    doc.styles.splice(at..at, moved);
    ok()
}

fn rename_folder(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "style.renameFolder";
    let from = style_name(cmd, &req_string(cmd, p, "folder")?)?;
    let to = style_name(cmd, &req_string(cmd, p, "name")?)?;
    let doc = s.doc_mut()?;
    let prefix = format!("{from}/");
    let inside: Vec<usize> = (0..doc.styles.len())
        .filter(|i| doc.styles.get(*i).is_some_and(|x| x.name.starts_with(&prefix) && !crate::library::is_remote_style(doc, &x.id)))
        .collect();
    if inside.is_empty() {
        return Err(bad(cmd, format!("no styles in folder `{from}`")));
    }
    for i in inside {
        if let Some(st) = doc.styles.get_mut(i)
            && let Some(rest) = st.name.strip_prefix(&prefix)
        {
            st.name = format!("{to}/{rest}");
        }
    }
    ok()
}

/// Re-apply style `sid` to every node (and text chars) linked to it.
pub fn propagate_style(doc: &mut Document, sid: &str) {
    let text_def = match doc.style(sid).map(|s| &s.kind) {
        Some(StyleKind::Text { style }) => Some(style.clone()),
        _ => None,
    };
    let ids: Vec<NodeId> = doc.ids().collect();
    for i in ids {
        let Some(n) = doc.node(i) else { continue };
        if let Some(def) = &text_def {
            if n.kind.text().is_some_and(|t| t.links_text_style(sid)) {
                let def = match doc.style(sid).map(|st| crate::style_vars::kind_for(doc, st, i).into_owned()) {
                    Some(StyleKind::Text { style }) => style,
                    _ => def.clone(),
                };
                if let Ok(n) = doc.get_mut(i)
                    && let NodeKind::Text(t) = &mut n.kind
                {
                    t.update_text_style(sid, &def);
                }
            }
            continue;
        }
        let refs = n.styles.clone();
        for (field, r) in [("fillStyle", &refs.fill), ("strokeStyle", &refs.stroke), ("effectStyle", &refs.effect), ("gridStyle", &refs.grid)] {
            if r.as_deref() == Some(sid) {
                let _ = apply_style_ref(doc, i, field, Some(sid));
            }
        }
    }
}

/// Text style links follow edits (on `ids`): chars that no longer show their style's values (or
/// whose style is gone) unlink, and a layer's link is the style all its chars share.
pub fn sync_text_styles(doc: &mut Document, ids: &[NodeId]) {
    let linked = |n: &Node| match &n.kind {
        NodeKind::Text(t) => n.styles.text.is_some() || t.style.style_id.is_some() || t.runs.iter().any(|r| r.style.style_id.is_some()),
        _ => false,
    };
    let ids: Vec<NodeId> = ids.iter().copied().filter(|i| doc.node(*i).is_some_and(linked)).collect();
    if ids.is_empty() {
        return;
    }
    let defs: BTreeMap<String, TextStyle> = doc
        .styles
        .iter()
        .filter_map(|s| match &s.kind {
            StyleKind::Text { style } => Some((s.id.clone(), style.clone())),
            _ => None,
        })
        .collect();
    let varying: Vec<&Style> = doc.styles.iter().filter(|s| !s.bound.is_empty() && matches!(s.kind, StyleKind::Text { .. })).collect();
    let mut updates: Vec<(NodeId, Option<StyleId>, TextData)> = Vec::new();
    for id in ids {
        let Some(n) = doc.node(id) else { continue };
        let NodeKind::Text(t) = &n.kind else { continue };
        // Styles with bound fields read as this layer sees them.
        let mut defs = std::borrow::Cow::Borrowed(&defs);
        for st in &varying {
            if let StyleKind::Text { style } = crate::style_vars::kind_for(doc, st, id).into_owned() {
                defs.to_mut().insert(st.id.clone(), style);
            }
        }
        let mut t2 = t.clone();
        // A layer linked as a whole whose chars carry no link yet (imported): they link now.
        if let Some(sid) = &n.styles.text
            && t2.text_style_link() == Some(None)
            && let Some(def) = defs.get(sid)
            && t2.spans().iter().all(|(_, _, s)| s.shows_text_style(def))
        {
            let len = t2.char_count();
            t2.restyle(0, len, |s| s.style_id = Some(sid.clone()));
        }
        t2.check_text_styles(|sid| defs.get(sid));
        let link = t2.text_style_link().flatten().map(str::to_string);
        if t2 != *t || link != n.styles.text {
            updates.push((id, link, t2));
        }
    }
    for (id, link, t2) in updates {
        if let Ok(n) = doc.get_mut(id) {
            n.styles.text = link;
            n.kind = NodeKind::Text(t2);
        }
    }
}

/// Paint, effect and grid style links follow edits (on `ids`): a list edited away from its style's
/// values (by `node.set`, a paint row, a bound variable…) unlinks, as text chars do
/// ([`sync_text_styles`]).
pub fn sync_list_styles(doc: &mut Document, before: &Document, ids: &[NodeId]) {
    for &id in ids {
        let (Some(n), Some(b)) = (doc.node(id), before.node(id)) else { continue };
        if n.styles.is_empty() {
            continue;
        }
        // A style's bound fields count as this layer sees them (in its modes).
        let off = |r: &Option<StyleId>, same: &dyn Fn(&StyleKind) -> bool| {
            r.as_deref().and_then(|s| doc.style(s)).is_some_and(|st| !same(&crate::style_vars::kind_for(doc, st, id)))
        };
        let fill = n.fills != b.fills && off(&n.styles.fill, &|k| matches!(k, StyleKind::Paint { paints } if *paints == n.fills));
        let stroke = n.strokes != b.strokes && off(&n.styles.stroke, &|k| matches!(k, StyleKind::Paint { paints } if *paints == n.strokes));
        let effect = n.effects != b.effects && off(&n.styles.effect, &|k| matches!(k, StyleKind::Effect { effects } if *effects == n.effects));
        let grid = n.grids != b.grids && off(&n.styles.grid, &|k| matches!(k, StyleKind::Grid { grids } if *grids == n.grids));
        if !(fill || stroke || effect || grid) {
            continue;
        }
        let Ok(n) = doc.get_mut(id) else { continue };
        for (gone, r) in [(fill, &mut n.styles.fill), (stroke, &mut n.styles.stroke), (effect, &mut n.styles.effect), (grid, &mut n.styles.grid)] {
            if gone {
                *r = None;
            }
        }
    }
}

/// `variable.bind`: bind (or with no variable, unbind) `field` of the targets.
pub fn bind(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "variable.bind";
    let field = req_string(cmd, p, "field")?;
    let var = string(p, "variable");
    let ids = req_targets(cmd, s, p)?;
    let doc = s.doc_mut()?;
    let vid = match &var {
        Some(v) => Some(doc.variables.get(v).map(|x| x.id.clone()).ok_or_else(|| bad(cmd, "no such variable"))?),
        None => None,
    };
    bind_variable(cmd, doc, &ids, &field, vid.as_deref())?;
    ok()
}

/// Bind an already resolved variable id, or unbind, inside the caller's transaction. Props and
/// `variable.bind` share type checks and paint alpha handling; normalisation applies the value.
pub(crate) fn bind_variable(cmd: &str, doc: &mut Document, ids: &[NodeId], field: &str, vid: Option<&str>) -> Result<()> {
    // An instance's component property takes a variable of its type; other fields the types their
    // scopes belong to.
    if let Some(v) = vid.and_then(|v| doc.variables.get(v)) {
        for i in ids {
            match field.strip_prefix(crate::components::PROP_BINDING) {
                Some(name) => super::component::check_prop_binding(doc, *i, name, v.kind).map_err(|e| bad(cmd, e))?,
                None if !super::variables::field_takes(doc, *i, field, v.kind) => {
                    return Err(bad(cmd, format!("`{field}` doesn't take a {} variable", v.kind.id())));
                }
                None => {}
            }
        }
    }
    for i in ids {
        let n = doc.get_mut(*i)?;
        match vid {
            Some(v) => {
                if n.bound.get(field).map(String::as_str) != Some(v) {
                    rebind_paint_alpha(&mut n.fills, &mut n.strokes, field, true);
                }
                n.bound.insert(field.to_string(), v.to_string());
            }
            None => {
                if n.bound.remove(field).is_some() {
                    rebind_paint_alpha(&mut n.fills, &mut n.strokes, field, false);
                }
            }
        }
    }
    Ok(())
}

/// Remove every binding key of the slot (`fill` and `fills/0/color` name the same one); true when
/// one was there.
pub(crate) fn unbind_slot(n: &mut figcraft_doc::Node, fills: bool, index: usize) -> bool {
    use super::lists::{BoundList, list_binding};
    let list = if fills { BoundList::Fills } else { BoundList::Strokes };
    let before = n.bound.len();
    n.bound.retain(|key, _| list_binding(key).is_none_or(|(l, i, field)| !(l == list && i == index && field == "color")));
    n.bound.len() != before
}

/// A colour variable bound to a solid paint owns its whole colour, alpha included. Binding takes the
/// alpha a typed colour had (see `Paint::solid`) out of the opacity, so the variable's alpha applies
/// once; an opacity set on its own, before or while bound, multiplies on top and stays. Unbinding
/// moves the colour's alpha back into the opacity, where a typed colour keeps it.
pub(crate) fn rebind_paint_alpha(fills: &mut [Paint], strokes: &mut [Paint], field: &str, bind: bool) {
    use super::lists::{BoundList, list_binding};
    let Some((list, i, "color")) = list_binding(field) else { return };
    let paints = match list {
        BoundList::Fills => fills,
        BoundList::Strokes => strokes,
        _ => return,
    };
    if let Some(p) = paints.get_mut(i)
        && let PaintKind::Solid { color } = &mut p.kind
    {
        if bind {
            // What's left is the opacity set on its own (unknown under a colour alpha of 0: 1).
            p.opacity = if p.color_alpha > 0.0 { (p.opacity / p.color_alpha).clamp(0.0, 1.0) } else { 1.0 };
            p.color_alpha = 1.0;
        } else {
            p.opacity *= color.a;
            p.color_alpha *= color.a;
        }
        color.a = 1.0;
    }
}

/// `variable.setMode`: choose a collection's mode on frames (the page when none are given).
pub fn set_mode(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "variable.setMode";
    let c = req_string(cmd, p, "collection")?;
    let doc = s.doc()?;
    let coll = doc.variables.collection(&c).ok_or_else(|| bad(cmd, "no such collection"))?;
    // A null mode clears the explicit mode: the layer inherits it again.
    let mode = match p.get("mode") {
        Some(Value::Null) => None,
        _ => {
            let m = req_string(cmd, p, "mode")?;
            Some(coll.modes.iter().find(|x| x.id == m || x.name == m).map(|x| x.id.clone()).ok_or_else(|| bad(cmd, "no such mode"))?)
        }
    };
    let cid = coll.id.clone();
    let mut ids = targets(s, p);
    if ids.is_empty() {
        ids = vec![s.page()?];
    }
    let doc = s.doc_mut()?;
    for i in ids {
        let modes = &mut doc.get_mut(i)?.var_modes;
        match &mode {
            Some(m) => modes.insert(cid.clone(), m.clone()),
            None => modes.remove(&cid),
        };
    }
    ok()
}

fn var_value(kind: VariableType, v: &Value) -> Option<VariableValue> {
    if let Some(a) = v.get("alias").and_then(Value::as_str) {
        return Some(VariableValue::Alias(a.to_string()));
    }
    Some(match kind {
        VariableType::Color => VariableValue::Color(color_of(v)?),
        VariableType::Number => VariableValue::Number(f64_of(v)?),
        VariableType::String => VariableValue::String(v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())),
        VariableType::Boolean => VariableValue::Boolean(v.as_bool()?),
    })
}

fn var_create(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "variable.create";
    let c = req_string(cmd, p, "collection")?;
    let name = req_string(cmd, p, "name")?;
    let kind = match req_string(cmd, p, "type")?.to_ascii_lowercase().as_str() {
        "color" => VariableType::Color,
        "number" | "float" => VariableType::Number,
        "string" => VariableType::String,
        "boolean" | "bool" => VariableType::Boolean,
        _ => return Err(bad(cmd, "type: color|number|string|boolean")),
    };
    let doc = s.doc_mut()?;
    let coll = doc.variables.collection(&c).cloned().ok_or_else(|| bad(cmd, "no such collection"))?;
    let mut values = BTreeMap::new();
    if let Some(Value::Object(m)) = get(p, "values") {
        for (mode, v) in m {
            let mid = coll
                .modes
                .iter()
                .find(|x| x.id == *mode || x.name == *mode)
                .map(|x| x.id.clone())
                .ok_or_else(|| bad(cmd, format!("no mode `{mode}`")))?;
            values.insert(mid, var_value(kind, v).ok_or_else(|| bad(cmd, "invalid value"))?);
        }
    }
    if let Some(v) = get(p, "value") {
        let val = var_value(kind, v).ok_or_else(|| bad(cmd, "invalid value"))?;
        for m in &coll.modes {
            values.entry(m.id.clone()).or_insert_with(|| val.clone());
        }
    }
    let default = VariableValue::default_for(kind);
    for m in &coll.modes {
        values.entry(m.id.clone()).or_insert_with(|| default.clone());
    }
    // Aliases name an existing variable of the same type (stored by id; nothing refers to the new
    // variable yet, so they can't loop).
    for val in values.values_mut() {
        if let VariableValue::Alias(a) = val {
            let t = doc.variables.get(a).filter(|t| t.kind == kind).ok_or_else(|| bad(cmd, format!("no {} variable `{a}` to alias", kind.id())))?;
            *a = t.id.clone();
        }
    }
    let id = doc.new_variable_id();
    doc.variables.variables.push(Variable {
        id: id.clone(),
        name,
        collection: coll.id,
        kind,
        values,
        description: string(p, "description").unwrap_or_default(),
        scopes: None,
        hidden_from_publishing: false,
        extends: None,
        overrides: Default::default(),
        code_syntax: Default::default(),
    });
    Ok(json!({"id": id}))
}

pub fn var_set(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "variable.set";
    let v = req_string(cmd, p, "variable")?;
    let doc = s.doc_mut()?;
    let var = doc.variables.get(&v).cloned().ok_or_else(|| bad(cmd, "no such variable"))?;
    let coll = doc.variables.collection(&var.collection).cloned().ok_or_else(|| bad(cmd, "variable has no collection"))?;
    let mode = match string(p, "mode") {
        Some(m) => coll.modes.iter().find(|x| x.id == m || x.name == m).map(|x| x.id.clone()).ok_or_else(|| bad(cmd, "no such mode"))?,
        None => coll.default_mode().cloned().ok_or_else(|| bad(cmd, "collection has no modes"))?,
    };
    let val = match get(p, "value").and_then(|x| var_value(var.kind, x)).ok_or_else(|| bad(cmd, "invalid value"))? {
        VariableValue::Alias(a) => VariableValue::Alias(super::variables::alias_target(&doc.variables, &var, &a).map_err(|e| bad(cmd, e))?),
        v => v,
    };
    if let Some(slot) = doc.variables.variables.iter_mut().find(|x| x.id == var.id) {
        // An inherited variable's value of its own overrides the inherited one in that mode.
        if slot.extends.is_some() {
            slot.overrides.insert(mode.clone());
        }
        slot.values.insert(mode, val);
    }
    ok()
}

/// Write bound variable values into nodes (called during normalisation).
pub fn apply_bindings(doc: &mut Document) {
    apply_bindings_to(doc, None);
}

/// [`apply_bindings`] for the layers `only` (every bound layer when `None`).
pub fn apply_bindings_to(doc: &mut Document, only: Option<&std::collections::BTreeSet<NodeId>>) {
    apply_bindings_matching(doc, only, |_| true);
}

/// The layers `ids` before an explicit edit, for [`detach_edited`] (only those with bindings).
pub(crate) fn bound_before(doc: &Document, ids: &[NodeId]) -> Vec<std::sync::Arc<Node>> {
    ids.iter().filter_map(|i| doc.node_arc(*i)).filter(|n| !n.bound.is_empty()).cloned().collect()
}

/// An explicit edit sticks: each layer's own bound field (not a list item's) whose value the edit
/// changed drops its binding, which would otherwise write the variable's value back (#809).
/// Compared by value, so keys written unchanged keep their bindings and fields moved along with
/// another (proportions, min/max) lose theirs.
pub(crate) fn detach_edited(doc: &mut Document, before: &[std::sync::Arc<Node>]) {
    for old in before {
        let Some(n) = doc.node(old.id) else { continue };
        let edited: Vec<String> = n
            .bound
            .keys()
            // A binding the edit itself made (`boundVariables`) stays.
            .filter(|f| old.bound.get(*f) == n.bound.get(*f))
            .filter(|f| super::lists::list_binding(f).is_none() && !f.starts_with(crate::components::PROP_BINDING))
            .filter(|f| bound_field(old, f) != bound_field(n, f))
            .cloned()
            .collect();
        if edited.is_empty() {
            continue;
        }
        if let Ok(n) = doc.get_mut(old.id) {
            for f in edited {
                n.bound.remove(&f);
            }
        }
    }
}

/// The stored value a layer-level binding writes, comparable (`None`: the layer has no such field).
fn bound_field(n: &Node, field: &str) -> Option<String> {
    let text = |f: &dyn Fn(&TextStyle) -> String| match &n.kind {
        NodeKind::Text(t) => Some(std::iter::once(&t.style).chain(t.runs.iter().map(|r| &r.style)).map(f).collect::<Vec<_>>().join(",")),
        _ => None,
    };
    let layout = n.layout.as_ref();
    let cl = &n.child_layout;
    let pad = |i: usize| layout.and_then(|l| l.padding.get(i).copied());
    Some(match field {
        "width" | "w" => format!("{}", n.width),
        "height" | "h" => format!("{}", n.height),
        "minWidth" => format!("{:?}", cl.min_w),
        "maxWidth" => format!("{:?}", cl.max_w),
        "minHeight" => format!("{:?}", cl.min_h),
        "maxHeight" => format!("{:?}", cl.max_h),
        "opacity" => format!("{}", n.opacity),
        "visible" => format!("{}", n.visible),
        "gap" | "columnGap" => format!("{:?}", layout.map(|l| l.gap)),
        "crossGap" | "rowGap" => format!("{:?}", layout.map(|l| l.cross_gap)),
        "padding" => format!("{:?}", layout.map(|l| l.padding)),
        "paddingTop" => format!("{:?}", pad(0)),
        "paddingRight" => format!("{:?}", pad(1)),
        "paddingBottom" => format!("{:?}", pad(2)),
        "paddingLeft" => format!("{:?}", pad(3)),
        "paddingHorizontal" => format!("{:?}", (pad(1), pad(3))),
        "paddingVertical" => format!("{:?}", (pad(0), pad(2))),
        "cornerRadius" => format!("{:?}", n.corners.radii),
        "topLeftRadius" => format!("{:?}", n.corners.radii.first()),
        "topRightRadius" => format!("{:?}", n.corners.radii.get(1)),
        "bottomRightRadius" => format!("{:?}", n.corners.radii.get(2)),
        "bottomLeftRadius" => format!("{:?}", n.corners.radii.get(3)),
        "strokeWeight" => format!("{:?}", (n.stroke.weight, n.stroke.sides)),
        "strokeTopWeight" | "strokeRightWeight" | "strokeBottomWeight" | "strokeLeftWeight" => format!("{:?}", n.stroke.sides),
        "arcStart" | "arcSweep" | "arcInner" => match &n.kind {
            NodeKind::Ellipse { start, sweep, inner } => format!(
                "{}",
                if field == "arcStart" {
                    start
                } else if field == "arcSweep" {
                    sweep
                } else {
                    inner
                }
            ),
            _ => return None,
        },
        "characters" => match &n.kind {
            NodeKind::Text(t) => t.characters.clone(),
            _ => return None,
        },
        "paragraphSpacing" | "paragraphIndent" => match &n.kind {
            NodeKind::Text(t) => format!("{}", if field == "paragraphSpacing" { t.paragraph_spacing } else { t.paragraph_indent }),
            _ => return None,
        },
        "fontFamily" => text(&|s| s.font_family.clone())?,
        "fontStyle" => text(&|s| s.font_style.clone())?,
        "fontWeight" => text(&|s| s.weight().to_string())?,
        "fontSize" => text(&|s| s.font_size.to_string())?,
        "lineHeight" => text(&|s| format!("{:?}", s.line_height))?,
        "letterSpacing" => text(&|s| format!("{:?}", s.letter_spacing))?,
        // shortcut: other layer-level fields keep their binding on edits; add a reader when one is bound in practice.
        _ => return None,
    })
}

/// Imported files (`.fig`, REST) keep a solid paint's alpha in its opacity, a bound paint's too,
/// while the bound colour brings its own alpha: take the variable's alpha back out of the opacity
/// (styles' paints and layers') before the bindings are first resolved, so it applies once.
pub(crate) fn unfold_imported_bound_alpha(doc: &mut Document) {
    use super::lists::{BoundList, list_binding};
    fn alphas(vars: &Variables, bound: &BTreeMap<String, VariableId>, modes: &BTreeMap<CollectionId, ModeId>) -> Vec<(BoundList, usize, f32)> {
        let found = bound.iter().filter_map(|(field, var)| match (list_binding(field)?, vars.resolve(var, modes)?) {
            ((list @ (BoundList::Fills | BoundList::Strokes), i, "color"), VariableValue::Color(c)) if c.a > 0.0 && c.a < 1.0 => Some((list, i, c.a)),
            _ => None,
        });
        found.collect()
    }
    fn unfold(p: Option<&mut Paint>, a: f32) {
        if let Some(p) = p
            && matches!(p.kind, PaintKind::Solid { .. })
        {
            p.opacity = (p.opacity / a).min(1.0);
        }
    }
    let pending = doc.take_dirty();
    for i in 0..doc.styles.len() {
        let found = doc.styles.get(i).map(|s| alphas(&doc.variables, &s.bound, &BTreeMap::new())).unwrap_or_default();
        if let Some(StyleKind::Paint { paints }) = doc.styles.get_mut(i).filter(|_| !found.is_empty()).map(|s| &mut s.kind) {
            for (_, j, a) in found {
                unfold(paints.get_mut(j), a);
            }
        }
    }
    let ids: Vec<NodeId> = doc.nodes().filter(|n| !n.bound.is_empty()).map(|n| n.id).collect();
    for id in ids {
        let found = doc.node(id).map(|n| alphas(&doc.variables, &n.bound, &doc.modes_for(id))).unwrap_or_default();
        if found.is_empty() {
            continue;
        }
        let Ok(n) = doc.get_mut(id) else { continue };
        for (list, j, a) in found {
            unfold(if list == BoundList::Fills { n.fills.get_mut(j) } else { n.strokes.get_mut(j) }, a);
        }
    }
    doc.take_dirty();
    doc.mark_dirty(pending);
}

/// Settle imported paint colours before the document is first shown. Other bindings can affect
/// layout or component structure, which must be resolved through the normal edit pipeline.
pub(crate) fn apply_paint_bindings_on_load(doc: &mut Document) {
    let pending = doc.take_dirty();
    apply_bindings_matching(doc, None, |field| {
        super::lists::list_binding(field).is_some_and(|(list, _, prop)| {
            matches!(list, super::lists::BoundList::Fills | super::lists::BoundList::Strokes)
                && (prop == "color" || super::lists::stop_binding(prop).is_some())
        })
    });
    doc.take_dirty();
    doc.mark_dirty(pending);
}

/// Re-apply the variable bindings whose field passes `include`, on the `only` nodes (every bound
/// node when `None`). Auto layout fields wait while their node has no auto layout.
fn apply_bindings_matching(doc: &mut Document, only: Option<&std::collections::BTreeSet<NodeId>>, include: impl Fn(&str) -> bool) {
    use super::lists::{BoundList, list_binding};
    if doc.variables.variables.is_empty() {
        return;
    }

    let bound: Vec<NodeId> = match only {
        Some(ids) => ids.iter().copied().filter(|i| doc.node(*i).is_some_and(|n| !n.bound.is_empty())).collect(),
        None => doc.nodes().filter(|n| !n.bound.is_empty()).map(|n| n.id).collect(),
    };
    for id in bound {
        let modes = doc.modes_for(id);
        let Some(n) = doc.node(id) else { continue };
        let binds = n.bound.clone();
        let has_layout = n.layout.is_some();
        let mut sets = serde_json::Map::new();
        let mut effect_sets: Vec<(usize, String, VariableValue)> = Vec::new();
        let mut paint_sets: Vec<(bool, usize, Option<usize>, Color)> = Vec::new();
        let mut grid_sets: Vec<(usize, String, f64)> = Vec::new();
        let mut prop_sets: Vec<(String, VariableValue)> = Vec::new();
        for (field, var) in binds {
            if !include(&field) {
                continue;
            }
            // A binding never creates an auto layout: without one it waits for the next.
            if !has_layout && super::props::LAYOUT_KEYS.contains(&field.as_str()) {
                continue;
            }
            let Some(val) = doc.variables.resolve(&var, &modes) else { continue };
            // One list item's value: a paint's colour, an effect's or a layout grid's field.
            if let Some((list, i, prop)) = list_binding(&field) {
                match (list, &val) {
                    (BoundList::Fills | BoundList::Strokes, VariableValue::Color(c)) if prop == "color" => {
                        paint_sets.push((list == BoundList::Fills, i, None, *c));
                    }
                    (BoundList::Fills | BoundList::Strokes, VariableValue::Color(c)) => {
                        if let Some(j) = super::lists::stop_binding(prop) {
                            paint_sets.push((list == BoundList::Fills, i, Some(j), *c));
                        }
                    }
                    (BoundList::Effects, _) => effect_sets.push((i, prop.to_string(), val)),
                    (BoundList::Grids, VariableValue::Number(x)) => grid_sets.push((i, prop.to_string(), *x)),
                    _ => {}
                }
                continue;
            }
            // `componentProperties/<name>`: an instance's boolean or text property value.
            if let Some(name) = field.strip_prefix(crate::components::PROP_BINDING) {
                prop_sets.push((name.to_string(), val));
                continue;
            }
            let jv = match &val {
                VariableValue::Color(c) => json!(c.to_css_hex()),
                // Opacity variables hold percentages.
                VariableValue::Number(x) if field == "opacity" => json!((x / 100.0).clamp(0.0, 1.0)),
                // Arc sweep and ratio variables hold percentages too (what the Arc fields show).
                VariableValue::Number(x) if field == "arcSweep" => json!(x * 3.6),
                VariableValue::Number(x) if field == "arcInner" => json!(x / 100.0),
                VariableValue::Number(x) if field == "characters" => json!(trim_num(*x)),
                VariableValue::Number(x) => json!(x),
                VariableValue::String(x) => json!(x),
                VariableValue::Boolean(b) => json!(b),
                VariableValue::Alias(_) => continue,
            };
            sets.insert(field, jv);
        }
        if !sets.is_empty() {
            // Most bound values are already in place: leave those layers untouched (shared with the
            // undo snapshots, and not re-synced or re-laid out as edited).
            let old = doc.node_arc(id).cloned();
            let was_dirty = doc.dirty().contains(&id);
            let _ = super::props::apply("variables", doc, id, &sets);
            if let Some(old) = old {
                // A value a variable gives a layer of an instance is no override.
                if doc.node(id).is_some_and(|n| n.overrides != old.overrides)
                    && let Ok(n) = doc.get_mut(id)
                {
                    n.overrides = old.overrides.clone();
                }
                doc.keep_if_unchanged(old, was_dirty);
            }
        }
        if !paint_sets.is_empty() {
            apply_paint_bindings(doc, id, &paint_sets);
        }
        if !effect_sets.is_empty() {
            apply_effect_bindings(doc, id, &effect_sets);
        }
        if !grid_sets.is_empty() {
            super::lists::apply_grid_bindings(doc, id, &grid_sets);
        }
        if !prop_sets.is_empty() {
            super::component::apply_prop_bindings(doc, id, &prop_sets);
        }
    }
}

/// Set paint `i`'s colour (or its gradient stop `stop`'s) to a bound colour; a first paint that
/// isn't there yet is added.
pub(crate) fn set_paint_color(list: &mut Vec<Paint>, i: usize, stop: Option<usize>, c: Color) {
    match (list.get_mut(i), stop) {
        (Some(p), Some(j)) => {
            if let PaintKind::Gradient(g) = &mut p.kind
                && let Some(s) = g.stops.get_mut(j)
            {
                s.color = c;
            }
        }
        (Some(p), None) => p.kind = PaintKind::Solid { color: c },
        (None, None) if i == 0 => list.push(Paint::solid(c)),
        (None, _) => {}
    }
}

/// Set `prop` of effect `i` to a bound value.
pub(crate) fn set_effect_field(effects: &mut [Effect], i: usize, prop: &str, val: &VariableValue) {
    let Some(e) = effects.get_mut(i) else { return };
    let num = match val {
        VariableValue::Number(x) if x.is_finite() => Some(*x),
        _ => None,
    };
    match (&mut e.kind, prop, num, val) {
        (EffectKind::DropShadow(s) | EffectKind::InnerShadow(s), "offsetX", Some(x), _) => s.offset.0 = x,
        (EffectKind::DropShadow(s) | EffectKind::InnerShadow(s), "offsetY", Some(x), _) => s.offset.1 = x,
        (EffectKind::DropShadow(s) | EffectKind::InnerShadow(s), "radius", Some(x), _) => s.radius = x.max(0.0),
        (EffectKind::DropShadow(s) | EffectKind::InnerShadow(s), "spread", Some(x), _) => s.spread = x,
        (EffectKind::DropShadow(s) | EffectKind::InnerShadow(s), "color", _, VariableValue::Color(c)) => s.color = *c,
        (EffectKind::LayerBlur(b) | EffectKind::BackgroundBlur(b), "radius", Some(x), _) => b.radius = x.max(0.0),
        (EffectKind::LayerBlur(b) | EffectKind::BackgroundBlur(b), "startRadius", Some(x), _) => b.start_radius = x.max(0.0),
        _ => {}
    }
}

/// Write bound colours into a node's fills (`true`) or strokes by index (or into a gradient
/// paint's stop), keeping the rest of the list and each paint's opacity, visibility and blend mode
/// (only what changed, so unchanged layers stay shared). A first-paint binding on an empty list
/// adds the paint.
fn apply_paint_bindings(doc: &mut Document, id: NodeId, sets: &[(bool, usize, Option<usize>, Color)]) {
    let Some(n) = doc.node(id) else { return };
    let (mut fills, mut strokes) = (n.fills.clone(), n.strokes.clone());
    for (is_fill, i, stop, c) in sets {
        set_paint_color(if *is_fill { &mut fills } else { &mut strokes }, *i, *stop, *c);
    }
    if doc.node(id).is_some_and(|n| n.fills != fills || n.strokes != strokes)
        && let Ok(n) = doc.get_mut(id)
    {
        n.fills = fills;
        n.strokes = strokes;
    }
}

/// Write bound values into a node's effects (only what changed, so unchanged layers stay shared).
fn apply_effect_bindings(doc: &mut Document, id: NodeId, sets: &[(usize, String, VariableValue)]) {
    let Some(mut effects) = doc.node(id).map(|n| n.effects.clone()) else { return };
    for (i, prop, val) in sets {
        set_effect_field(&mut effects, *i, prop, val);
    }
    if doc.node(id).is_some_and(|n| n.effects != effects)
        && let Ok(n) = doc.get_mut(id)
    {
        n.effects = effects;
    }
}

/// Solid fill and stroke colours used under `page` (alpha includes the paint's opacity), most used
/// first.
pub fn document_colors(doc: &Document, page: NodeId) -> Vec<(Color, usize)> {
    let mut counts: BTreeMap<[u8; 4], (Color, usize)> = BTreeMap::new();
    let mut stack = vec![page];
    let mut seen = 0usize;
    while let Some(id) = stack.pop() {
        seen += 1;
        if seen > 200_000 {
            break;
        }
        let Some(n) = doc.node(id) else { continue };
        stack.extend(n.children.iter().copied());
        for paint in n.fills.iter().chain(n.strokes.iter()).filter(|p| p.visible) {
            if let Some(c) = paint.solid_color() {
                counts.entry(c.to_rgba8()).or_insert((c, 0)).1 += 1;
            }
        }
    }
    let mut out: Vec<(Color, usize)> = counts.into_values().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.to_hex().cmp(&b.0.to_hex())));
    out
}
