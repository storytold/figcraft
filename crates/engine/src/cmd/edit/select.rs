use super::{
    Affine, BTreeMap, BTreeSet, Node, NodeId, Result, Session, Value, bad, boolean, json, num, num_or, ok, req_num, req_string, req_targets, roots,
};

pub(super) fn layers_at(s: &mut Session, p: &Value) -> Result<Value> {
    let (x, y) = (req_num("select.layersAt", p, "x")?, req_num("select.layersAt", p, "y")?);
    let doc = s.doc()?;
    let zoom = s.ds()?.view.zoom.max(1e-6);
    let sel = s.selection();
    let result = crate::booleans::result_outline(doc, &sel);
    let layers: Vec<Value> = figcraft_doc::geometry::layers_at(doc, s.page()?, figcraft_geom::Point::new(x, y), 2.0 / zoom, Some(&result))
        .into_iter()
        .filter_map(|(id, depth)| doc.node(id).map(|n| json!({"id": id, "name": n.name, "type": n.kind.type_name(), "depth": depth})))
        .collect();
    Ok(json!({"layers": layers}))
}

pub(super) fn select_all(s: &mut Session, _p: &Value) -> Result<Value> {
    let doc = s.doc()?;
    let sel = s.selection();
    // Select all siblings of the current selection, else the page's top level.
    let parent = sel.first().and_then(|i| doc.node(*i)).and_then(|n| n.parent).unwrap_or(s.page()?);
    let ids: Vec<NodeId> = doc.get(parent)?.children.iter().copied().filter(|c| doc.node(*c).is_some_and(|n| n.visible && !n.locked)).collect();
    s.set_selection(ids.clone());
    Ok(json!({"selection": ids}))
}

/// What Select all would select, less the selection (hidden and locked layers neither).
pub(super) fn select_inverse(s: &mut Session, _p: &Value) -> Result<Value> {
    let doc = s.doc()?;
    let sel = s.selection();
    let parent = sel.first().and_then(|i| doc.node(*i)).and_then(|n| n.parent).unwrap_or(s.page()?);
    let ids: Vec<NodeId> =
        doc.get(parent)?.children.iter().copied().filter(|c| !sel.contains(c) && doc.node(*c).is_some_and(|n| n.visible && !n.locked)).collect();
    s.set_selection(ids.clone());
    Ok(json!({"selection": ids}))
}

/// `select.area`: what a marquee from (x, y) over the box selects; no box asks the app for one.
pub(super) fn select_area(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "select.area";
    let keys = ["x", "y", "width", "height"];
    if keys.iter().all(|k| p.get(*k).is_none()) {
        s.request_ui("selectArea", json!({}));
        return ok();
    }
    let [x, y, w, h] = keys.map(|k| num(p, k));
    let (Some(x), Some(y), Some(w), Some(h)) = (x, y, w, h) else { return Err(bad(cmd, "pass x, y, width and height (numbers)")) };
    if ![x, y, w, h].iter().all(|v| v.is_finite() && v.abs() < 1e9) {
        return Err(bad(cmd, "the box is out of range"));
    }
    let start = figcraft_geom::Point::new(x, y);
    let r = figcraft_geom::Rect::from_points(start, figcraft_geom::Point::new(x + w, y + h));
    let add = boolean(p, "add") == Some(true);
    let base = if add { s.selection() } else { Vec::new() };
    let within = s.marquee_within(start)?;
    let sel = s.marquee_selection(r, within, boolean(p, "deep") == Some(true), &base, add)?;
    s.set_selection(sel.clone());
    Ok(json!({"selection": sel}))
}

pub(super) fn select_children(s: &mut Session, _p: &Value) -> Result<Value> {
    let doc = s.doc()?;
    let mut ids = Vec::new();
    for i in s.selection() {
        if let Some(n) = doc.node(i) {
            ids.extend(n.children.iter().copied());
        }
    }
    if ids.is_empty() {
        return Ok(json!({"selection": s.selection()}));
    }
    s.set_selection(ids.clone());
    Ok(json!({"selection": ids}))
}

pub(super) fn select_parent(s: &mut Session, _p: &Value) -> Result<Value> {
    let doc = s.doc()?;
    let mut ids: Vec<NodeId> = Vec::new();
    for i in s.selection() {
        if let Some(p) = doc.node(i).and_then(|n| n.parent)
            && !doc.node(p).is_some_and(Node::is_page)
            && !ids.contains(&p)
        {
            ids.push(p);
        }
    }
    if ids.is_empty() {
        return Ok(json!({"selection": s.selection()}));
    }
    s.set_selection(ids.clone());
    Ok(json!({"selection": ids}))
}

pub(super) fn select_sibling(s: &mut Session, dir: i64) -> Result<Value> {
    let doc = s.doc()?;
    let sel = s.selection();
    let Some(cur) = sel.last().copied() else {
        let first = doc.get(s.page()?)?.children.last().copied();
        s.set_selection(first.into_iter().collect());
        return Ok(json!({"selection": s.selection()}));
    };
    let parent = doc.get(cur)?.parent.ok_or_else(|| bad("select.next", "no parent"))?;
    let kids = &doc.get(parent)?.children;
    let idx = kids.iter().position(|c| *c == cur).unwrap_or(0) as i64;
    let n = kids.len() as i64;
    // Layers panel order is reversed (top of the list = last child): Tab goes down the list.
    let next = (idx - dir).rem_euclid(n.max(1)) as usize;
    let id = kids.get(next).copied().unwrap_or(cur);
    s.set_selection(vec![id]);
    Ok(json!({"selection": [id]}))
}

pub(super) fn select_matching(s: &mut Session, _p: &Value) -> Result<Value> {
    let doc = s.doc()?;
    let sel = s.selection();
    let Some(first) = sel.first().and_then(|i| doc.node(*i)) else { return Ok(json!({"selection": []})) };
    let (name, ty) = (first.name.clone(), first.kind.type_name());
    let page = s.page()?;
    let ids: Vec<NodeId> =
        doc.subtree(page).into_iter().filter(|i| doc.node(*i).is_some_and(|n| n.name == name && n.kind.type_name() == ty)).collect();
    s.set_selection(ids.clone());
    Ok(json!({"selection": ids}))
}

pub(super) fn select_same(s: &mut Session, same: impl Fn(&Node, &Node) -> bool) -> Result<Value> {
    let doc = s.doc()?;
    let sel = s.selection();
    let Some(first) = sel.first().and_then(|i| doc.node(*i)).cloned() else { return Ok(json!({"selection": []})) };
    let page = s.page()?;
    let ids: Vec<NodeId> = doc.subtree(page).into_iter().filter(|i| *i != page && doc.node(*i).is_some_and(|n| same(&first, n))).collect();
    s.set_selection(ids.clone());
    Ok(json!({"selection": ids}))
}

/// The opacity number keys give: one digit is tens of percent (0 is 100%), two digits typed in
/// quick succession the exact percentage.
pub fn opacity_from_digits(digits: &str) -> Option<f32> {
    let mut n = 0u32;
    let mut count = 0;
    for c in digits.chars() {
        // Input-derived: a number that outgrows `u32` stops the scan (None); it never wraps.
        n = n.checked_mul(10)?.checked_add(c.to_digit(10)?)?;
        count += 1;
    }
    match count {
        1 if n == 0 => Some(1.0),
        1 => Some(n as f32 / 10.0),
        2 => Some(n as f32 / 100.0),
        _ => None,
    }
}

pub(super) fn opacity_keys(s: &mut Session, p: &Value) -> Result<Value> {
    let digits = req_string("object.opacityKeys", p, "digits")?;
    let o = opacity_from_digits(&digits).ok_or_else(|| bad("object.opacityKeys", "`digits` must be one or two digits"))?;
    let ids = req_targets("object.opacityKeys", s, p)?;
    let doc = s.doc_mut()?;
    let before = crate::cmd::styles::bound_before(doc, &ids);
    for i in ids {
        doc.get_mut(i)?.opacity = o;
    }
    crate::cmd::styles::detach_edited(doc, &before);
    Ok(json!({"opacity": o}))
}

pub(super) fn nudge(s: &mut Session, p: &Value) -> Result<Value> {
    let big = boolean(p, "big").unwrap_or(false);
    let k = if big { s.prefs.nudge_big } else { s.prefs.nudge_small };
    let (ux, uy) = (num_or(p, "dx", 0.0), num_or(p, "dy", 0.0));
    let (dx, dy) = (ux * k, uy * k);
    let ids = req_targets("transform.nudge", s, p)?;
    // Edit a copy and keep it only if something moved, so an arrow that can't move anything adds
    // no undo step.
    let mut work = s.doc()?.clone();
    let doc = &mut work;
    let ids = roots(doc, &ids);
    let mut changed = false;
    // Children in an auto layout flow move one place instead (see `al_keys`).
    for i in &ids {
        if crate::al_keys::flowing_parent(doc, *i).is_none() && (dx, dy) != (0.0, 0.0) {
            let n = doc.get_mut(*i)?;
            n.transform = Affine::translate((dx, dy)) * n.transform;
            changed = true;
        }
    }
    for dir in crate::al_keys::Dir::from_units(ux, uy) {
        // Grouped per arrow: a move into a neighbouring frame changes the parent.
        let mut groups: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
        for i in &ids {
            if let Some(parent) = crate::al_keys::flowing_parent(doc, *i) {
                groups.entry(parent).or_default().push(*i);
            }
        }
        for (parent, block) in groups {
            changed |= crate::al_keys::arrow_move(doc, parent, &block, dir)?;
        }
    }
    if changed {
        s.ds_mut()?.doc = std::sync::Arc::new(work);
    }
    ok()
}

pub(super) fn nudge_size(s: &mut Session, p: &Value) -> Result<Value> {
    let big = boolean(p, "big").unwrap_or(false);
    let k = if big { s.prefs.nudge_big } else { s.prefs.nudge_small };
    let (dw, dh) = (num_or(p, "dw", 0.0) * k, num_or(p, "dh", 0.0) * k);
    let ids = req_targets("transform.nudgeSize", s, p)?;
    let ids = roots(s.doc()?, &ids);
    for i in ids {
        let Some((w, h)) = s.doc()?.node(i).map(|n| (n.width, n.height)) else { continue };
        let mut q = serde_json::Map::new();
        q.insert("ids".into(), json!([i]));
        // Only the axis that changes becomes fixed.
        if dw != 0.0 {
            q.insert("width".into(), json!((w + dw).max(0.0)));
        }
        if dh != 0.0 {
            q.insert("height".into(), json!((h + dh).max(0.0)));
        }
        crate::cmd::arrange::resize_cmd(s, &Value::Object(q))?;
    }
    ok()
}

pub(super) fn toggle(s: &mut Session, p: &Value, what: &str) -> Result<Value> {
    let ids = req_targets("object.toggle", s, p)?;
    let doc = s.doc_mut()?;
    let explicit = boolean(p, what);
    // Figma: if any is visible, hide all (and vice versa).
    let value = explicit.unwrap_or_else(|| match what {
        "visible" => !ids.iter().all(|i| doc.node(*i).is_some_and(|n| n.visible)),
        _ => !ids.iter().all(|i| doc.node(*i).is_some_and(|n| n.locked)),
    });
    for i in &ids {
        let n = doc.get_mut(*i)?;
        if what == "visible" {
            n.visible = value;
        } else {
            n.locked = value;
        }
    }
    if what == "locked" && value {
        s.set_selection(vec![]);
    }
    Ok(json!({what: value}))
}

pub(super) fn hide_others(s: &mut Session, _p: &Value) -> Result<Value> {
    let sel = s.selection();
    let page = s.page()?;
    let doc = s.doc_mut()?;
    let keep: BTreeSet<NodeId> = sel
        .iter()
        .flat_map(|i| {
            let mut v = doc.ancestors(*i);
            v.push(*i);
            v
        })
        .collect();
    let tops: Vec<NodeId> = doc.get(page)?.children.clone();
    for t in tops {
        if !keep.contains(&t) {
            doc.get_mut(t)?.visible = false;
        }
    }
    ok()
}
