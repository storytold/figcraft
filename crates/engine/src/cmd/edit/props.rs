use super::{
    BTreeMap, Effect, EngineError, Map, Node, NodeId, NodeKind, Result, Session, Value, bad, blend, boolean, color, fraction_of, get, json, num, ok,
    opt_index, paint_of, parse, req_id, req_index, req_string, req_targets, roots, string, targets, z_sorted,
};

pub(super) fn batch_rename(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "object.batchRename";
    let ids = req_targets(cmd, s, p)?;
    let find = string(p, "find");
    let replace = string(p, "replace").unwrap_or_default();
    let pattern = string(p, "name");
    let rename =
        crate::rename::BatchRename::new(find.as_deref(), boolean(p, "regex").unwrap_or(false), &replace, pattern.as_deref(), num(p, "start"))
            .map_err(|e| bad(cmd, e))?;
    let doc = s.doc_mut()?;
    let names = rename.plan(doc, &ids);
    for (i, name) in &names {
        doc.get_mut(*i)?.name = name.clone();
    }
    Ok(json!({"names": names.iter().map(|(i, n)| json!({"id": i, "name": n})).collect::<Vec<_>>()}))
}

/// Char index of the first case-insensitive occurrence of `needle` in `hay` at or after `from`.
/// Works on char indices (what `replace_range` takes), so lowercasing never shifts offsets.
fn find_ci(hay: &[char], needle: &[char], from: usize) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    (from..hay.len())
        .find(|&i| hay.get(i..i + needle.len()).is_some_and(|w| w.iter().zip(needle).all(|(a, b)| a == b || a.to_lowercase().eq(b.to_lowercase()))))
}

pub(super) fn find_replace(s: &mut Session, p: &Value) -> Result<Value> {
    let find = req_string("edit.findReplace", p, "find")?;
    if find.is_empty() {
        return Err(bad("edit.findReplace", "`find` is empty"));
    }
    // Line breaks stored as the editor stores them (CRLF, CR, U+2028 and U+2029 become LF).
    let replace = string(p, "replace").map(|r| crate::cmd::props::normalize_line_endings(&r));
    let names = boolean(p, "layerNames").unwrap_or(false);
    let page = s.page()?;
    let doc = s.doc_mut()?;
    let mut hits = Vec::new();
    let needle: Vec<char> = find.chars().collect();
    let r_chars: Vec<char> = replace.as_deref().map(|r| r.chars().collect()).unwrap_or_default();
    for i in doc.subtree(page) {
        let Some(n) = doc.node(i) else { continue };
        let text: Option<Vec<char>> = match &n.kind {
            NodeKind::Text(t) => Some(t.characters.chars().collect()),
            _ => None,
        };
        let text_hit = text.as_deref().is_some_and(|c| find_ci(c, &needle, 0).is_some());
        let name_hit = names && find_ci(&n.name.chars().collect::<Vec<_>>(), &needle, 0).is_some();
        if !(text_hit || name_hit) || n.is_page() {
            continue;
        }
        hits.push(i);
        if let Some(r) = &replace {
            let n = doc.get_mut(i)?;
            if names && name_hit {
                let chars: Vec<char> = n.name.chars().collect();
                let mut out = String::new();
                let mut at = 0;
                while let Some(pos) = find_ci(&chars, &needle, at) {
                    out.extend(chars.get(at..pos).unwrap_or_default());
                    out.push_str(r);
                    at = pos + needle.len();
                }
                out.extend(chars.get(at..).unwrap_or_default());
                n.name = out;
            }
            if let (NodeKind::Text(t), Some(mut chars)) = (&mut n.kind, text) {
                // Replace occurrence by occurrence so styles shift correctly. The search resumes
                // after each inserted replacement, so a replacement that matches `find` (in any
                // case) neither loops forever nor stops the other occurrences.
                let mut cursor = 0;
                for _ in 0..10_000 {
                    let Some(start) = find_ci(&chars, &needle, cursor) else { break };
                    t.replace_range(start, start + needle.len(), r);
                    chars.splice(start..start + needle.len(), r_chars.iter().copied());
                    cursor = start + r_chars.len();
                }
            }
        }
    }
    if boolean(p, "select").unwrap_or(true) {
        s.set_selection(hits.clone());
    }
    Ok(json!({"matches": hits}))
}

/// Keys only some layer types take, the test for those layers, and why the others skip them.
type TypedKeys<'a> = (&'a [&'a str], &'a dyn Fn(&Node) -> bool, &'a str);

pub(super) fn set(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = req_targets("node.set", s, p)?;
    let props = match get(p, "props") {
        Some(Value::Object(m)) => m.clone(),
        _ => {
            // Allow flat params: {"id": 3, "fill": "#f00"}.
            let mut m = p.as_object().cloned().unwrap_or_default();
            m.remove("id");
            m.remove("ids");
            m
        }
    };
    if props.is_empty() {
        return Err(bad("node.set", "no properties given"));
    }
    for (k, v) in &props {
        crate::cmd::props::variable_prop("node.set", k, v)?;
    }
    // A new rotation for the selection turns it about its moved rotation origin: each layer's
    // point under the origin stays put.
    let ds = s.ds()?;
    let pivot =
        ds.moved_origin().filter(|_| props.contains_key("rotation") && !props.contains_key("x") && !props.contains_key("y") && ds.selection == ids);
    let pinned: Vec<(NodeId, figcraft_geom::Point)> = match pivot {
        Some(o) => roots(&ds.doc, &ids)
            .into_iter()
            .map(|i| (i, ds.doc.world_transform(i)))
            .filter(|(_, w)| w.determinant().abs() > 1e-12)
            .map(|(i, w)| (i, w.inverse() * o))
            .collect(),
        None => Vec::new(),
    };
    let mut warnings = font_style_warnings(&s.ds()?.doc, &ids, &props);
    // Type-specific keys go to the layers that take them and are skipped, with a warning, on the
    // others: auto layout keys to frame-like layers, `path` to vectors. With none among the targets
    // that take them, the first refuses them (`props::layout_mut`, `props::apply`).
    let doc = &s.ds()?.doc;
    let layout_keys: Vec<&str> = props.keys().map(String::as_str).filter(|k| crate::cmd::props::LAYOUT_KEYS.contains(k)).collect();
    let path_keys: Vec<&str> = props.keys().map(String::as_str).filter(|k| *k == "path").collect();
    let frame_like = |n: &Node| n.kind.is_frame_like();
    let vector = |n: &Node| matches!(n.kind, NodeKind::Vector { .. });
    let typed: [TypedKeys; 2] = [
        (&layout_keys, &frame_like, "only frames, components and instances have auto layout"),
        (&path_keys, &vector, "only vector layers have a path"),
    ];
    let mut skipped: BTreeMap<NodeId, Map<String, Value>> = BTreeMap::new();
    for (keys, takes, why) in typed {
        let takes = |i: &NodeId| doc.node(*i).is_some_and(takes);
        if keys.is_empty() || !ids.iter().any(takes) {
            continue;
        }
        let others: Vec<NodeId> = ids.iter().copied().filter(|i| !takes(i)).collect();
        if others.is_empty() {
            continue;
        }
        for i in &others {
            skipped.entry(*i).or_insert_with(|| props.clone()).retain(|k, _| !keys.contains(&k.as_str()));
        }
        let names: Vec<String> =
            others.iter().take(10).filter_map(|i| doc.node(*i)).map(|n| format!("{} “{}” ({})", n.id, n.name, n.kind.type_name())).collect();
        let more = if others.len() > names.len() { format!(" and {} more", others.len() - names.len()) } else { String::new() };
        warnings.push(format!(
            "{} skipped on {}{more}: {why}",
            keys.iter().map(|k| format!("`{k}`")).collect::<Vec<_>>().join(", "),
            names.join(", ")
        ));
    }
    let doc = s.doc_mut()?;
    let before = crate::cmd::styles::bound_before(doc, &ids);
    for i in ids {
        crate::cmd::props::apply("node.set", doc, i, skipped.get(&i).unwrap_or(&props))?;
    }
    crate::cmd::styles::detach_edited(doc, &before);
    if let Some(o) = pivot {
        for (i, q) in pinned {
            let d = o - doc.world_transform(i) * q;
            if d.hypot().is_finite() {
                crate::cmd::arrange::move_world(doc, i, d.x, d.y)?;
            }
        }
    }
    if warnings.is_empty() { ok() } else { Ok(json!({"ok": true, "warnings": warnings})) }
}

/// A `fontStyle` draws in the family's face nearest to it, as a text layer resolves it. When that
/// face has another weight or slant than the name asks for ("Black" on a family whose heaviest is
/// Bold), say which face it is. Families that aren't installed, and names that match a style or a
/// named instance (spelled any way: "SemiBold", "Semi Bold"), say nothing.
fn font_style_warnings(doc: &figcraft_doc::Document, ids: &[NodeId], props: &serde_json::Map<String, Value>) -> Vec<String> {
    let Some(style) = props.get("fontStyle").and_then(Value::as_str) else { return Vec::new() };
    let mut families: Vec<String> = match props.get("fontFamily").and_then(Value::as_str) {
        Some(f) => vec![f.to_string()],
        None => ids
            .iter()
            .filter_map(|i| doc.node(*i))
            .filter_map(|n| n.kind.text())
            .flat_map(|t| t.spans().into_iter().map(|(_, _, st)| st.font_family.clone()).collect::<Vec<_>>())
            .collect(),
    };
    families.sort();
    families.dedup();
    let norm = |s: &str| s.to_lowercase().replace([' ', '-', '_'], "");
    let (want_w, want_i) = (i32::from(figcraft_doc::style_weight(style)), style.to_lowercase().contains("italic"));
    families
        .into_iter()
        .filter_map(|family| {
            let styles = figcraft_text::family_styles(&family);
            if styles.is_empty() || styles.iter().any(|x| norm(x) == norm(style)) {
                return None;
            }
            let face = figcraft_text::find_face(&family, style).filter(|f| f.family.eq_ignore_ascii_case(&family))?;
            let w = face.axis("wght").map_or(i32::from(face.weight), |a| a.clamp(f64::from(want_w)).round() as i32);
            if face.instance(style).is_some() || (w == want_w && face.italic == want_i) {
                return None;
            }
            Some(format!("{family} has no `{style}` style: it draws in {}, the nearest (it has: {})", face.style, styles.join(", ")))
        })
        .collect()
}

/// `stroke.widthProfile`: a preset or points (through `node.set`'s `widthProfile`, so instances
/// record the override).
pub(super) fn width_profile(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "stroke.widthProfile";
    let v = get(p, "preset").or_else(|| get(p, "points")).cloned().unwrap_or(Value::Null);
    let ids = req_targets(cmd, s, p)?;
    let props = serde_json::Map::from_iter([("widthProfile".to_string(), v)]);
    let doc = s.doc_mut()?;
    for i in ids {
        crate::cmd::props::apply(cmd, doc, i, &props)?;
    }
    ok()
}

pub(super) fn stroke_brush(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "stroke.brush";
    let v = if get(p, "preset").is_none_or(Value::is_null) { Value::Null } else { p.clone() };
    let brush = crate::cmd::props::brush_of(&v).ok_or_else(|| bad(cmd, crate::cmd::props::BRUSH_HELP))?;
    let ids = req_targets(cmd, s, p)?;
    let doc = s.doc_mut()?;
    for i in ids {
        let n = doc.get_mut(i)?;
        if brush.is_some() && n.strokes.is_empty() {
            n.strokes.push(figcraft_color::Paint::solid(figcraft_color::Color::BLACK));
        }
        n.stroke.brush = brush.clone();
    }
    ok()
}

pub(super) fn reparent(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = req_targets("node.reparent", s, p)?;
    let parent = req_id("node.reparent", p, "parent")?;
    let index = num(p, "index").map(|i| i.max(0.0) as usize);
    let doc = s.doc_mut()?;
    let ids = z_sorted(doc, &roots(doc, &ids));
    // `index` is where the first moved layer ends up among the parent's children (bottom first),
    // counted after the move, the way `node.reorder` counts it. The layers land together, keeping
    // their order, under the sibling that stays put and ends up just after them (none: on top).
    let above = index.and_then(|at| doc.get(parent).ok()?.children.iter().filter(|c| !ids.contains(c)).nth(at).copied());
    for i in &ids {
        // Counted with the layer still in place, as `reparent` expects.
        let at = above.and_then(|a| doc.node(parent)?.children.iter().position(|c| *c == a));
        doc.reparent(*i, parent, at)?;
    }
    ok()
}

pub(super) fn paint_list(s: &mut Session, p: &Value, fills: bool, add: bool) -> Result<Value> {
    let cmd = if fills { "fill" } else { "stroke" };
    let ids = req_targets(cmd, s, p)?;
    let index = if add || get(p, "index").is_none() { None } else { Some(req_index(cmd, p, "index")?) };
    let doc = s.doc_mut()?;
    for i in ids {
        let n = doc.get_mut(i)?;
        let frameish = n.kind.is_frame_like();
        let list = if fills { &mut n.fills } else { &mut n.strokes };
        if add {
            let paint = match get(p, "paint").or_else(|| get(p, "color")) {
                Some(v) => paint_of(v).ok_or_else(|| bad(cmd, "invalid paint"))?,
                None if fills && frameish => figcraft_color::Paint::solid(figcraft_color::Color::WHITE),
                None if fills => figcraft_color::Paint::solid(figcraft_color::Color::DEFAULT_SHAPE),
                None => figcraft_color::Paint::solid(figcraft_color::Color::BLACK),
            };
            list.push(paint);
        } else {
            let idx = index.unwrap_or(list.len().saturating_sub(1));
            // The paint's colour binding goes with it; later paints' move down one.
            crate::cmd::lists::remove_item(n, if fills { crate::cmd::lists::BoundList::Fills } else { crate::cmd::lists::BoundList::Strokes }, idx);
        }
    }
    ok()
}

pub(super) fn paint_set(s: &mut Session, p: &Value, fills: bool) -> Result<Value> {
    let cmd = if fills { "fill.set" } else { "stroke.set" };
    let ids = req_targets(cmd, s, p)?;
    let idx = opt_index(cmd, p, "index")?;
    let new_paint = match get(p, "paint") {
        Some(v) => Some(paint_of(v).ok_or_else(|| bad(cmd, "invalid paint"))?),
        None => None,
    };
    let col = color(p, "color");
    let doc = s.doc_mut()?;
    for i in ids {
        let n = doc.get_mut(i)?;
        let list = if fills { &mut n.fills } else { &mut n.strokes };
        let at = idx.unwrap_or(list.len().saturating_sub(1));
        if list.is_empty() {
            list.push(figcraft_color::Paint::solid(if fills { figcraft_color::Color::DEFAULT_SHAPE } else { figcraft_color::Color::BLACK }));
        }
        let Some(slot) = list.get_mut(at) else { return Err(bad(cmd, format!("no paint at index {at}"))) };
        let old = slot.clone();
        if let Some(np) = &new_paint {
            *slot = np.clone();
        }
        if let Some(c) = col {
            slot.kind = figcraft_color::PaintKind::Solid { color: c.with_alpha(1.0) };
            if c.a < 1.0 {
                slot.opacity = c.a;
                slot.color_alpha = c.a;
            }
        }
        if let Some(o) = get(p, "opacity").and_then(fraction_of) {
            slot.opacity = o.clamp(0.0, 1.0) as f32;
            slot.color_alpha = 1.0;
        }
        if let Some(v) = boolean(p, "visible") {
            slot.visible = v;
        }
        if let Some(b) = blend(p, "blend") {
            slot.blend = b;
        }
        crate::cmd::lists::rebind_edited_paint(n, fills, at, &old);
        if fills {
            n.styles.fill = None;
        } else {
            n.styles.stroke = None;
        }
    }
    ok()
}

/// Copy paint `index` (the top one by default) of the first target into the clipboard. A stroke
/// takes its node-wide settings (weight, alignment, sides, caps, joins, dashes) along.
pub(super) fn paint_copy(s: &mut Session, p: &Value, fills: bool) -> Result<Value> {
    let cmd = if fills { "fill.copy" } else { "stroke.copy" };
    let id = targets(s, p).first().copied().ok_or_else(|| bad(cmd, "select a layer"))?;
    let at = opt_index(cmd, p, "index")?;
    let (paint, stroke) = {
        let doc = s.doc()?;
        let n = doc.get(id)?;
        let list = if fills { &n.fills } else { &n.strokes };
        let at = at.unwrap_or(list.len().saturating_sub(1));
        let paint = list.get(at).cloned().ok_or_else(|| bad(cmd, format!("no {} at index {at}", if fills { "fill" } else { "stroke" })))?;
        (paint, (!fills).then(|| n.stroke.clone()))
    };
    // An image paint names its data by key: take the data along, so a paste into another file has
    // it too. A pattern names a layer in this file, so the file is recorded with the copy.
    let mut images = std::collections::BTreeMap::new();
    if let figcraft_color::PaintKind::Image(img) = &paint.kind
        && let Some(d) = s.doc()?.images.get(&img.image)
    {
        images.insert(img.image.clone(), d.clone());
    }
    s.clipboard.paint_images = images;
    s.clipboard.paint_source_doc = s.ds()?.uid;
    s.clipboard.paint = Some((fills, paint.clone()));
    s.clipboard.stroke = stroke;
    serde_json::to_value(&paint).map_err(|e| bad(cmd, e.to_string()))
}

/// Paste the copied paint onto the targets: replacing paint `index` there, or adding one. A
/// stroke's copied settings go onto each target too.
pub(super) fn paint_paste(s: &mut Session, p: &Value, fills: bool) -> Result<Value> {
    let cmd = if fills { "fill.paste" } else { "stroke.paste" };
    let Some((copied_fills, mut paint)) = s.clipboard.paint.clone() else {
        return Err(EngineError::Disabled(cmd.into(), "no copied fill or stroke".into()));
    };
    if copied_fills != fills {
        let what = if fills { "stroke" } else { "fill" };
        return Err(EngineError::Disabled(cmd.into(), format!("the copied paint is a {what}")));
    }
    let ids = req_targets(cmd, s, p)?;
    // A copied pattern pointing at a target or a layer inside it would make the layer pattern
    // itself; `fill.pattern` refuses the same, and the renderer would drop the pattern. Its node id
    // only names something in the file it was copied from, where it may be an unrelated layer.
    if let figcraft_color::PaintKind::Pattern(pt) = &paint.kind {
        if s.ds()?.uid != s.clipboard.paint_source_doc {
            return Err(bad(cmd, "a pattern can't be pasted into another file"));
        }
        let doc = s.doc()?;
        if ids.iter().any(|t| doc.is_ancestor(*t, NodeId(pt.source))) {
            return Err(bad(cmd, "a layer can't be filled with itself or a layer inside it"));
        }
    }
    // An image paint names its data by key; a copy from another file brings the data with it.
    if let figcraft_color::PaintKind::Image(img) = &mut paint.kind {
        let copied = img.image.clone();
        if let Some(d) = s.clipboard.paint_images.get(&copied).cloned() {
            let doc = s.doc_mut()?;
            // The destination may already use that key for different data (other layers keep
            // that image): the pasted paint takes the copied data's content hash instead. That key
            // can be taken too — images are usually keyed by their hash, so the copied key often
            // is that hash — and then a numbered variant of it. Image keys become archive member
            // names, which keep only letters and digits, so the number is appended plainly.
            let base = if doc.images.get(&copied).is_some_and(|existing| existing != &d) { figcraft_format::image_hash(&d.bytes) } else { copied };
            let mut key = base.clone();
            let mut n = 0u32;
            while doc.images.get(&key).is_some_and(|existing| existing != &d) {
                n = n.saturating_add(1);
                if n > 999 {
                    return Err(bad(cmd, "no free image key for the copied image"));
                }
                key = format!("{base}{n}");
            }
            doc.images.entry(key.clone()).or_insert(d);
            img.image = key;
        }
    }
    let mut q = json!({"ids": ids, "paint": serde_json::to_value(paint).map_err(|e| bad(cmd, e.to_string()))?});
    let out = match opt_index(cmd, p, "index")? {
        Some(at) => {
            q["index"] = json!(at);
            paint_set(s, &q, fills)?
        }
        None => paint_list(s, &q, fills, true)?,
    };
    if let Some(stroke) = s.clipboard.stroke.clone().filter(|_| !fills) {
        for id in ids {
            s.doc_mut()?.get_mut(id)?.stroke = stroke.clone();
        }
    }
    Ok(out)
}

pub(super) fn pattern_fill(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "fill.pattern";
    let src = req_id(CMD, p, "source")?;
    let doc = s.doc()?;
    if doc.node(src).is_none_or(|n| n.is_page()) {
        return Err(bad(CMD, "`source` must be a layer"));
    }
    if req_targets(CMD, s, p)?.iter().any(|t| doc.is_ancestor(*t, src)) {
        return Err(bad(CMD, "a layer can't be filled with itself or a layer inside it"));
    }
    let mut pt = figcraft_color::PatternPaint::new(src.0);
    pt.tile = parse(CMD, p, "tile")?.unwrap_or_default();
    pt.align = parse(CMD, p, "align")?.unwrap_or_default();
    pt.spacing = parse(CMD, p, "spacing")?.unwrap_or_default();
    let (lo, hi) = figcraft_color::PatternPaint::SCALE_RANGE;
    pt.scale = num(p, "scale").filter(|v| v.is_finite()).unwrap_or(1.0).clamp(lo, hi);
    let mut q = if p.is_object() { p.clone() } else { json!({}) };
    q["paint"] =
        serde_json::to_value(figcraft_color::Paint { kind: figcraft_color::PaintKind::Pattern(pt), ..figcraft_color::Paint::pattern(src.0) })
            .map_err(|e| bad(CMD, e.to_string()))?;
    paint_set(s, &q, true)
}

pub(super) fn effect_add(s: &mut Session, p: &Value) -> Result<Value> {
    let e: Effect = match crate::cmd::props::effect_from("effect.add", p, "effect")? {
        Some(e) => e,
        None => {
            let ty = string(p, "type").unwrap_or_else(|| "dropShadow".into());
            Effect::of_type(&ty).ok_or_else(|| bad("effect.add", format!("unknown effect type `{ty}`")))?
        }
    };
    for i in req_targets("effect.add", s, p)? {
        s.doc_mut()?.get_mut(i)?.effects.push(e.clone());
    }
    ok()
}

/// Copy effect `index` (the top one by default) of the first target into the clipboard.
pub(super) fn effect_copy(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "effect.copy";
    let id = targets(s, p).first().copied().ok_or_else(|| bad(CMD, "select a layer"))?;
    let at = opt_index(CMD, p, "index")?;
    let e = {
        let doc = s.doc()?;
        let list = &doc.get(id)?.effects;
        let at = at.unwrap_or(list.len().saturating_sub(1));
        list.get(at).cloned().ok_or_else(|| bad(CMD, format!("no effect at index {at}")))?
    };
    s.clipboard.effect = Some(e.clone());
    serde_json::to_value(&e).map_err(|err| bad(CMD, err.to_string()))
}

/// Paste the copied effect onto the targets: replacing effect `index` there, or adding one.
pub(super) fn effect_paste(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "effect.paste";
    let Some(e) = s.clipboard.effect.clone() else {
        return Err(EngineError::Disabled(CMD.into(), "no copied effect".into()));
    };
    let at = opt_index(CMD, p, "index")?;
    for id in req_targets(CMD, s, p)? {
        let n = s.doc_mut()?.get_mut(id)?;
        match at {
            Some(i) => {
                if let Some(slot) = n.effects.get_mut(i) {
                    *slot = e.clone();
                }
            }
            None => n.effects.push(e.clone()),
        }
    }
    ok()
}
