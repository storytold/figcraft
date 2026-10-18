//! Edit menu: undo/redo, clipboard, duplicate/delete, selection commands, find, properties.

use std::collections::{BTreeMap, BTreeSet};

use figcraft_doc::*;
use figcraft_geom::Affine;
use serde_json::{Value, json};

use super::{CommandSpec, ok};
use crate::params::*;
use crate::{EngineError, Result, Session, bad, spec};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!("edit.undo", "Undo", "{}", false, |s, _| Ok(json!({"undone": s.undo()?}))),
        spec!("edit.redo", "Redo", "{}", false, |s, _| Ok(json!({"redone": s.redo()?}))),
        spec!("edit.delete", "Delete", "{ids?}", true, delete),
        spec!(
            "edit.duplicate",
            "Duplicate",
            "{ids?, dx?, dy? (page offset; default: the offset of the last duplicate when ids are its copies, still selected; else top-level frames to the right with a 100 gap, everything else in place)}",
            true,
            duplicate
        ),
        spec!("edit.copy", "Copy", "{ids?}", false, copy),
        spec!("edit.cut", "Cut", "{ids?}", true, cut),
        spec!("edit.paste", "Paste", "{parent?, x?, y? (page coordinates of the pasted top-left)}", true, paste),
        spec!(
            "edit.pasteDesignLayers",
            "Paste design layers",
            "{html (the clipboard's HTML) | data (base64 of the clipboard's design data, at most 256 MB), parent?, x?, y?}  layers copied from another design app (a .fig canvas in the clipboard's HTML), read like a .fig file (best effort, with `warnings`), pasted as edit.paste pastes",
            true,
            paste_design_layers
        ),
        spec!("edit.pasteOver", "Paste over selection", "{}", true, paste_over),
        spec!("edit.pasteReplace", "Paste to replace", "{ids?}", true, paste_replace),
        spec!("edit.copyProperties", "Copy properties", "{id?}", false, copy_props),
        spec!("edit.pasteProperties", "Paste properties", "{ids?}", true, paste_props),
        spec!(
            "edit.findReplace",
            "Find and replace",
            "{find, replace? (text layers), layerNames? (bool), select? (default true)}",
            true,
            find_replace
        ),
        spec!("edit.setDefaultProperties", "Set default properties", "{id?}", false, |s, p| {
            let id = targets(s, p).first().copied().ok_or_else(|| bad("edit.setDefaultProperties", "select a layer"))?;
            let v = copy_props_of(s.doc()?, id);
            s.clipboard.props = Some(v);
            ok()
        }),
        spec!("select.set", "Select", "{ids: [..]}  (replace the selection)", false, |s, p| {
            let ids = match get(p, "ids") {
                Some(Value::Array(a)) => a.iter().filter_map(id_of).collect(),
                _ => id(p, "id").into_iter().collect(),
            };
            select_and_page(s, ids)?;
            Ok(json!({"selection": s.selection()}))
        }),
        spec!("select.add", "Add to selection", "{ids}", false, |s, p| {
            let mut sel = s.selection();
            for i in targets_param(p) {
                if !sel.contains(&i) {
                    sel.push(i);
                }
            }
            s.set_selection(sel);
            Ok(json!({"selection": s.selection()}))
        }),
        spec!("select.remove", "Remove from selection", "{ids}", false, |s, p| {
            let rm = targets_param(p);
            let sel: Vec<NodeId> = s.selection().into_iter().filter(|i| !rm.contains(i)).collect();
            s.set_selection(sel);
            Ok(json!({"selection": s.selection()}))
        }),
        spec!(
            "select.layersAt",
            "Layers at a point",
            "{x, y (page coordinates)} → {layers: [{id, name, type, depth}]} (every layer under the point, front-most first; what the context menu's Select layer lists)",
            false,
            layers_at
        ),
        spec!("select.all", "Select all", "{}", false, select_all),
        spec!("select.none", "Select none", "{}", false, |s, _| {
            s.set_selection(vec![]);
            if let Ok(ds) = s.ds_mut() {
                ds.text_edit = None;
                ds.vector_edit = None;
            }
            ok()
        }),
        spec!("select.inverse", "Select inverse", "{}", false, select_inverse),
        spec!(
            "select.area",
            "Select area",
            "{x, y, width, height (page units; a marquee from x, y), add? (toggle like a Shift marquee), deep? (nested layers, like Cmd/Ctrl)} → {selection}; with no box: draw one from the keyboard in the app (arrows move it, Cmd/Ctrl+arrows or Shift+arrows size it, Enter selects, Escape cancels)",
            false,
            select_area
        ),
        spec!("select.children", "Select children", "{}", false, select_children),
        spec!("select.parent", "Select parent", "{}", false, select_parent),
        spec!("select.next", "Select next sibling", "{}", false, |s, _| select_sibling(s, 1)),
        spec!("select.previous", "Select previous sibling", "{}", false, |s, _| select_sibling(s, -1)),
        spec!("select.matching", "Select matching layers", "{}", false, select_matching),
        spec!("select.sameFill", "Select same fill", "{}", false, |s, _| select_same(s, |a, b| a.fills == b.fills && !a.fills.is_empty())),
        spec!("select.sameStroke", "Select same stroke", "{}", false, |s, _| select_same(s, |a, b| a.strokes == b.strokes
            && a.stroke == b.stroke
            && !a.strokes.is_empty())),
        spec!("select.sameEffect", "Select same effect", "{}", false, |s, _| select_same(s, |a, b| a.effects == b.effects && !a.effects.is_empty())),
        spec!("select.sameText", "Select same text properties", "{}", false, |s, _| {
            select_same(s, |a, b| matches!((&a.kind, &b.kind), (NodeKind::Text(x), NodeKind::Text(y)) if x.style == y.style))
        }),
        spec!("select.sameFont", "Select same font", "{}", false, |s, _| {
            select_same(
                s,
                |a, b| matches!((&a.kind, &b.kind), (NodeKind::Text(x), NodeKind::Text(y)) if x.style.font_family == y.style.font_family && x.style.font_style == y.style.font_style),
            )
        }),
        spec!("select.sameInstance", "Select same instance", "{}", false, |s, _| {
            select_same(
                s,
                |a, b| matches!((&a.kind, &b.kind), (NodeKind::Instance { component: x, .. }, NodeKind::Instance { component: y, .. }) if x == y),
            )
        }),
        spec!("select.sameProperties", "Select same properties", "{}", false, |s, _| {
            select_same(s, |a, b| {
                a.kind.type_name() == b.kind.type_name()
                    && a.fills == b.fills
                    && a.strokes == b.strokes
                    && a.effects == b.effects
                    && a.corners == b.corners
            })
        }),
        spec!("select.byName", "Select layers by name", "{query, page? (default current; \"all\" for every page)}", false, |s, p| {
            let q = req_string("select.byName", p, "query")?;
            let page = if string(p, "page").as_deref() == Some("all") { None } else { Some(s.page()?) };
            let ids = s.doc()?.find_by_name(&q, page);
            select_and_page(s, ids.clone())?;
            Ok(json!({"selection": ids}))
        }),
        spec!(
            "transform.nudge",
            "Nudge",
            "{dx, dy, big? (use the big nudge amount)} (auto layout children move one place instead: along the flow, between wrapped rows, to the next grid cell, or into the neighbouring nested auto layout frame)",
            true,
            nudge
        ),
        spec!(
            "transform.nudgeSize",
            "Resize by nudge",
            "{dw, dh, big?} (grow or shrink the selection by the nudge amount, top-left fixed)",
            true,
            nudge_size
        ),
        spec!("object.rename", "Rename", "{ids?, name?} (without a name: rename the layer inline in the layers panel)", true, |s, p| {
            let ids = req_targets("object.rename", s, p)?;
            if p.get("name").is_none() {
                s.request_ui("layers.rename", json!({"ids": ids}));
                return ok();
            }
            let name = req_string("object.rename", p, "name")?;
            let doc = s.doc_mut()?;
            for i in ids {
                doc.get_mut(i)?.name = name.clone();
            }
            ok()
        }),
        spec!(
            "object.batchRename",
            "Batch rename",
            "{ids?, find? (match; empty: none), regex? (find is a regular expression), replace? (for each match), name? (pattern for the whole name), start? (where $n starts and $N stops; default 1)} codes in replace and name: $& current name (or the match), $` and $' text before and after the match, $1…$9 groups, $n up, $N down, $nnn padded; layers are numbered in stacking order, bottom first",
            true,
            batch_rename
        ),
        spec!("object.toggleVisible", "Show/Hide selection", "{ids?, visible?}", true, |s, p| toggle(s, p, "visible")),
        spec!("object.toggleLock", "Lock/Unlock selection", "{ids?, locked?}", true, |s, p| toggle(s, p, "locked")),
        spec!("object.hideOthers", "Hide other layers", "{}", true, hide_others),
        spec!(
            "object.opacityKeys",
            "Opacity from number keys",
            "{digits (\"1\"…\"9\": 10%…90%, \"0\": 100%; two digits: that percentage, \"45\" is 45%), ids?}",
            true,
            opacity_keys
        ),
        spec!("object.removeFill", "Remove fill", "{ids?}", true, |s, p| {
            for i in req_targets("object.removeFill", s, p)? {
                s.doc_mut()?.get_mut(i)?.fills.clear();
            }
            ok()
        }),
        spec!("object.removeStroke", "Remove stroke", "{ids?}", true, |s, p| {
            for i in req_targets("object.removeStroke", s, p)? {
                s.doc_mut()?.get_mut(i)?.strokes.clear();
            }
            ok()
        }),
        spec!("object.swapFillStroke", "Swap fill and stroke", "{ids?}", true, |s, p| {
            for i in req_targets("object.swapFillStroke", s, p)? {
                let n = s.doc_mut()?.get_mut(i)?;
                std::mem::swap(&mut n.fills, &mut n.strokes);
            }
            ok()
        }),
        spec!("layers.collapseAll", "Collapse layers", "{}", false, |s, _| {
            s.request_ui("layers.collapseAll", json!({}));
            ok()
        }),
        spec!("node.set", "Set properties", super::props::KEYS, true, set),
        spec!("node.get", "Get layer", "{id}", false, |s, p| {
            let i = req_id("node.get", p, "id")?;
            Ok(crate::inspect::node_json(s.doc()?, i, 1))
        }),
        spec!("node.reparent", "Move layer into", "{ids?, parent, index?}", true, reparent),
        spec!("node.reorder", "Reorder layer", "{id, index}", true, |s, p| {
            let i = req_id("node.reorder", p, "id")?;
            let idx = req_num("node.reorder", p, "index")?.max(0.0) as usize;
            s.doc_mut()?.reorder(i, idx)?;
            ok()
        }),
        spec!("fill.add", "Add fill", "{ids?, paint? (default #D9D9D9 or white for frames)}", true, |s, p| paint_list(s, p, true, true)),
        spec!("stroke.add", "Add stroke", "{ids?, paint? (default black)}", true, |s, p| paint_list(s, p, false, true)),
        spec!("fill.remove", "Remove fill", "{ids?, index}", true, |s, p| paint_list(s, p, true, false)),
        spec!("stroke.remove", "Remove stroke", "{ids?, index}", true, |s, p| paint_list(s, p, false, false)),
        spec!("fill.set", "Set fill", "{ids?, index?, paint | color, opacity?, visible?, blend?}", true, |s, p| paint_set(s, p, true)),
        spec!("stroke.set", "Set stroke", "{ids?, index?, paint | color, opacity?, visible?, blend?}", true, |s, p| paint_set(s, p, false)),
        spec!(
            "fill.pattern",
            "Pattern fill",
            "{source (layer id), ids?, index? (default the top fill), tile? (grid|horizontalHex|verticalHex), scale? (1 = the source's size), spacing? [x, y], align? (start|center|end), opacity?}  the source layer tiled; it updates as the source changes",
            true,
            pattern_fill
        ),
        spec!(
            "stroke.widthProfile",
            "Width profile",
            "{ids?, preset? (uniform|wedge|taper|eye|rounded|pinch) | points? [[t (0-1 along each subpath), scale (× stroke weight)], ...]} — none = uniform",
            true,
            width_profile
        ),
        spec!(
            "stroke.brush",
            "Brush",
            "{ids?, preset (taper|ink|ribbon|bristle: stretched along the path; dots|stars|confetti|leaves: scattered along it | null: plain), spacing? (× weight), sizeJitter? (0-1), rotationJitter? (0-1), scatter? (0-4 × weight), seed?}  layers without a stroke get a black one",
            true,
            stroke_brush
        ),
        spec!(
            "effect.add",
            "Add effect",
            "{ids?, type? (dropShadow|innerShadow|layerBlur|backgroundBlur|noise|texture|glass), effect? {..}}",
            true,
            effect_add
        ),
        spec!("effect.remove", "Remove effect", "{ids?, index}", true, |s, p| {
            let i = req_index("effect.remove", p, "index")?;
            for id in req_targets("effect.remove", s, p)? {
                // Its bindings go with it; later effects' move down one.
                super::lists::remove_item(s.doc_mut()?.get_mut(id)?, super::lists::BoundList::Effects, i);
            }
            ok()
        }),
        spec!("effect.set", "Set effect", "{ids?, index, effect {..}}", true, |s, p| {
            let i = req_index("effect.set", p, "index")?;
            let e: Effect = parse("effect.set", p, "effect")?.ok_or_else(|| bad("effect.set", "`effect` required"))?;
            for id in req_targets("effect.set", s, p)? {
                let n = s.doc_mut()?.get_mut(id)?;
                if let Some(slot) = n.effects.get_mut(i) {
                    *slot = e.clone();
                }
            }
            ok()
        }),
        spec!("grid.add", "Add layout grid", "{ids?, grid? {kind: grid|columns|rows, ..}}", true, |s, p| {
            let g: LayoutGrid = parse("grid.add", p, "grid")?.unwrap_or_default();
            for id in req_targets("grid.add", s, p)? {
                s.doc_mut()?.get_mut(id)?.grids.push(g.clone());
            }
            ok()
        }),
        spec!("grid.remove", "Remove layout grid", "{ids?, index}", true, |s, p| {
            let i = req_index("grid.remove", p, "index")?;
            for id in req_targets("grid.remove", s, p)? {
                super::lists::remove_item(s.doc_mut()?.get_mut(id)?, super::lists::BoundList::Grids, i);
            }
            ok()
        }),
        spec!(
            "export.add",
            "Add export setting",
            "{ids?, setting? {size: 2x|512w|512h|{by: scale|width|height, value}, suffix?, format?: png|jpg|svg|pdf|webp, bleed? (PDF: points, or with a unit: \"3mm\", \"0.125in\", \"9pt\"), cropMarks?, bleedMarks?, cmyk? (PDF print options)}} (without a setting: 1x, then one scale step up with an \"@2x\"-style suffix)",
            true,
            |s, p| {
                let cmd = "export.add";
                let setting = get(p, "setting").filter(|v| !v.is_null());
                // Check the setting once before touching any layer.
                let given = setting.map(|v| super::file::export_setting(cmd, ExportSetting::default(), v)).transpose()?;
                for id in req_targets(cmd, s, p)? {
                    let n = s.doc_mut()?.get_mut(id)?;
                    let e = given.clone().unwrap_or_else(|| ExportSetting::next_after(n.exports.last()));
                    n.exports.push(e);
                }
                ok()
            }
        ),
        spec!(
            "export.update",
            "Edit export setting",
            "{ids?, index, size? (2x|512w|512h|{by, value}), suffix?, format? (png|jpg|svg|pdf|webp), bleed? (PDF: points, or with a unit: \"3mm\", \"0.125in\", \"9pt\"), cropMarks?, bleedMarks?, cmyk? (PDF print options)}",
            true,
            |s, p| {
                let cmd = "export.update";
                let i = req_index(cmd, p, "index")?;
                super::file::export_setting(cmd, ExportSetting::default(), p)?;
                for id in req_targets(cmd, s, p)? {
                    let n = s.doc_mut()?.get_mut(id)?;
                    if let Some(e) = n.exports.get_mut(i) {
                        *e = super::file::export_setting(cmd, e.clone(), p)?;
                    }
                }
                ok()
            }
        ),
        spec!("export.remove", "Remove export setting", "{ids?, index}", true, |s, p| {
            let i = req_index("export.remove", p, "index")?;
            for id in req_targets("export.remove", s, p)? {
                let n = s.doc_mut()?.get_mut(id)?;
                if i < n.exports.len() {
                    n.exports.remove(i);
                }
            }
            ok()
        }),
        spec!("command.batch", "Run several commands", "{label?, commands: [{command, params}]} (one undo step)", false, |s, p| {
            let label = string(p, "label").unwrap_or_else(|| "Batch".into());
            let cmds: Vec<(String, Value)> = match get(p, "commands") {
                Some(Value::Array(a)) => {
                    let mut out = Vec::new();
                    for (i, c) in a.iter().enumerate() {
                        let cmd =
                            c.get("command").and_then(Value::as_str).ok_or_else(|| bad("command.batch", format!("step {i} has no `command`")))?;
                        out.push((cmd.to_string(), c.get("params").cloned().unwrap_or(json!({}))));
                    }
                    out
                }
                _ => return Err(bad("command.batch", "`commands` must be an array")),
            };
            Ok(Value::Array(s.batch(&label, &cmds)?))
        }),
        spec!("edit.repeat", "Repeat last command", "{}", true, |s, _| {
            let Some((id, p)) = s.last_command.clone().filter(|(id, _)| id != "edit.repeat") else {
                return Err(EngineError::Disabled("edit.repeat".into(), "nothing to repeat".into()));
            };
            let spec = crate::find_command(&id).ok_or_else(|| EngineError::UnknownCommand(id.clone()))?;
            (spec.run)(s, &p)
        }),
    ]
}

fn targets_param(p: &Value) -> Vec<NodeId> {
    match get(p, "ids") {
        Some(Value::Array(a)) => a.iter().filter_map(id_of).collect(),
        _ => id(p, "id").into_iter().collect(),
    }
}

/// Select `ids`, switching to their page if they're elsewhere.
pub fn select_and_page(s: &mut Session, ids: Vec<NodeId>) -> Result<()> {
    let doc = s.doc()?;
    for i in &ids {
        if !doc.contains(*i) {
            return Err(EngineError::NoNode(*i));
        }
    }
    if let Some(first) = ids.first()
        && let Some(pg) = doc.page_of(*first)
        && pg != s.page()?
    {
        // Library copies live outside the pages.
        if !doc.pages.contains(&pg) {
            return Err(EngineError::Disabled("select".into(), format!("{first} isn't on a page (it is a library component)")));
        }
        s.ds_mut()?.page = pg;
    }
    s.set_selection(ids);
    Ok(())
}

/// Top-level selection: drop ids whose ancestor is also selected.
pub fn roots(doc: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    let set: BTreeSet<NodeId> = ids.iter().copied().collect();
    ids.iter().copied().filter(|i| !doc.ancestors(*i).iter().any(|a| set.contains(a))).collect()
}

/// Ids sorted in document (z) order.
pub fn z_sorted(doc: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    let Some(page) = ids.first().and_then(|i| doc.page_of(*i)) else { return ids.to_vec() };
    let order: BTreeMap<NodeId, usize> = doc.subtree(page).into_iter().enumerate().map(|(i, id)| (id, i)).collect();
    let mut v = ids.to_vec();
    v.sort_by_key(|i| order.get(i).copied().unwrap_or(usize::MAX));
    v
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = req_targets("edit.delete", s, p)?;
    let doc = s.doc_mut()?;
    let ids = roots(doc, &ids);
    let mut parents = Vec::new();
    for i in &ids {
        if doc.node(*i).is_some_and(Node::is_page) {
            continue;
        }
        if let Some(pn) = doc.node(*i).and_then(|n| n.parent) {
            parents.push(pn);
        }
        doc.remove(*i)?;
    }
    // Select the parent frame like Figma? Figma clears the selection.
    s.set_selection(vec![]);
    Ok(json!({"deleted": ids}))
}

/// Layers duplicated last (Duplicate or Alt-drag) and their copies, in the same order. While the
/// copies are the selection, Duplicate places the next copies as far from them as they are from
/// their sources, measured then: so moving the copies first sets the step.
#[derive(Clone, Debug)]
pub struct Duplicated {
    pub sources: Vec<NodeId>,
    pub copies: Vec<NodeId>,
}

impl Duplicated {
    pub fn is_selection(&self, sel: &[NodeId]) -> bool {
        let set = |v: &[NodeId]| v.iter().copied().collect::<BTreeSet<_>>();
        !self.copies.is_empty() && set(&self.copies) == set(sel)
    }

    /// The page offset from the sources to the copies, if both are still there and the copies move
    /// freely (an auto layout places its own).
    fn offset(&self, doc: &Document) -> Option<(f64, f64)> {
        let free = self
            .copies
            .iter()
            .all(|c| doc.node(*c).is_some_and(|n| n.child_layout.absolute || n.parent.and_then(|p| doc.node(p)).is_none_or(|p| p.layout.is_none())));
        if !free || self.sources.iter().chain(&self.copies).any(|i| !doc.contains(*i)) {
            return None;
        }
        let (a, b) = (doc.bounds_of(&self.sources)?, doc.bounds_of(&self.copies)?);
        let d = (b.x0 - a.x0, b.y0 - a.y0);
        (d.0.is_finite() && d.1.is_finite() && (d.0 != 0.0 || d.1 != 0.0)).then_some(d)
    }
}

fn duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = req_targets("edit.duplicate", s, p)?;
    let rename = s.prefs.rename_duplicates;
    let explicit = num(p, "dx").is_some() || num(p, "dy").is_some();
    let ds = s.ds()?;
    let roots_now = z_sorted(&ds.doc, &roots(&ds.doc, &ids));
    let repeat = ds.last_duplicate.as_ref().filter(|d| !explicit && d.is_selection(&roots_now)).and_then(|d| d.offset(&ds.doc));
    let doc = s.doc_mut()?;
    let ids = roots_now;
    let mut out = Vec::new();
    for i in ids.iter().copied() {
        let n = doc.get(i)?;
        let parent = n.parent.ok_or_else(|| bad("edit.duplicate", "pages are duplicated with page.duplicate"))?;
        let top_frame = doc.node(parent).is_some_and(Node::is_page) && n.kind.is_frame_like();
        let idx = doc.index_in_parent(i).map(|x| x + 1);
        let w = n.width;
        let nid = doc.duplicate_into(i, parent, idx)?;
        let (dx, dy) = match (num(p, "dx"), num(p, "dy")) {
            (Some(dx), dy) => (dx, dy.unwrap_or(0.0)),
            (None, Some(dy)) => (0.0, dy),
            // Top-level frames duplicate to the right with a gap, everything else in place.
            _ if top_frame => (w + 100.0, 0.0),
            _ => (0.0, 0.0),
        };
        match repeat {
            Some((rx, ry)) => super::arrange::move_world(doc, nid, rx, ry)?,
            None => {
                let nn = doc.get_mut(nid)?;
                nn.transform = Affine::translate((dx, dy)) * nn.transform;
            }
        }
        if rename {
            let nn = doc.get_mut(nid)?;
            nn.name = bump_name(&nn.name);
        }
        out.push(nid);
    }
    s.set_selection(out.clone());
    s.ds_mut()?.last_duplicate = Some(Duplicated { sources: ids, copies: out.clone() });
    Ok(json!({"ids": out}))
}

fn bump_name(n: &str) -> String {
    if let Some((base, num)) = n.rsplit_once(' ')
        && let Ok(k) = num.parse::<u64>()
        && let Some(next) = k.checked_add(1)
    {
        return format!("{base} {next}");
    }
    format!("{n} 2")
}

/// The copied form of the selection.
pub fn copy_to_clipboard(s: &mut Session, cmd: &str, ids: &[NodeId]) -> Result<()> {
    let ds = s.ds()?;
    // Pages are not layers: they can't be pasted anywhere.
    let layers: Vec<NodeId> = ids.iter().copied().filter(|i| !ds.doc.node(*i).is_some_and(Node::is_page)).collect();
    if layers.is_empty() && !ids.is_empty() {
        return Err(EngineError::Disabled(cmd.into(), "pages can't be copied; use page.duplicate".into()));
    }
    let (nodes, images, bounds) = copied(&ds.doc, &layers);
    let (source, source_path, uid) = (ds.doc.clone(), ds.path.clone(), ds.uid);
    s.clipboard.source = Some(source);
    s.clipboard.source_path = source_path;
    s.clipboard.source_doc = uid;
    s.clipboard.nodes = nodes;
    s.clipboard.images = images;
    s.clipboard.bounds = bounds;
    Ok(())
}

type Copied = (Vec<(NodeId, Vec<Node>)>, BTreeMap<String, ImageData>, Option<figcraft_geom::Rect>);

fn copied(doc: &Document, ids: &[NodeId]) -> Copied {
    let ids = z_sorted(doc, &roots(doc, ids));
    let mut nodes = Vec::new();
    let mut images = BTreeMap::new();
    for i in &ids {
        let world = doc.world_transform(*i);
        let mut sub: Vec<Node> = doc.subtree(*i).into_iter().filter_map(|x| doc.node(x).cloned()).collect();
        // Store the root's transform in page space so pastes land where it was.
        if let Some(r) = sub.first_mut() {
            r.transform = world;
        }
        for n in &sub {
            for p in n.fills.iter().chain(n.strokes.iter()) {
                if let figcraft_color::PaintKind::Image(img) = &p.kind
                    && let Some(d) = doc.images.get(&img.image)
                {
                    images.insert(img.image.clone(), d.clone());
                }
            }
        }
        nodes.push((*i, sub));
    }
    let bounds = doc.bounds_of(&ids);
    (nodes, images, bounds)
}

/// The markers around the copied layers in the clipboard's HTML, spelt as that format spells them.
pub const CLIP_OPEN: &str = "(figma)";
pub const CLIP_CLOSE: &str = "(/figma)";
/// Largest design data read from the clipboard: the `.fig` import's own canvas cap (256 MB). Its
/// base64 is checked first, so neither the text nor the decoded bytes can pass it.
const MAX_CLIP_DATA: usize = 256 * 1024 * 1024;
const MAX_CLIP_BASE64: usize = MAX_CLIP_DATA.div_ceil(3) * 4;

/// Base64 `text` (whitespace ignored, padding optional) when it fits [`MAX_CLIP_DATA`].
fn clip_base64(text: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    if text.bytes().filter(|b| !b.is_ascii_whitespace()).count() > MAX_CLIP_BASE64 {
        return None;
    }
    let b64: String = text.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    base64::engine::general_purpose::STANDARD.decode(&b64).or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(&b64)).ok()
}

/// Layers copied from another design app travel in the clipboard's HTML as a `.fig` canvas in
/// base64 between [`CLIP_OPEN`] and [`CLIP_CLOSE`]. This gives those bytes, or `None` when the HTML
/// holds no readable design data (it may just mention the marker).
pub fn design_clipboard_buffer(html: &str) -> Option<Vec<u8>> {
    let start = html.find(CLIP_OPEN)? + CLIP_OPEN.len();
    let len = html.get(start..)?.find(CLIP_CLOSE)?;
    let bytes = clip_base64(html.get(start..start + len)?)?;
    figcraft_format::fig::is_fig(&bytes).then_some(bytes)
}

fn paste_design_layers(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "edit.pasteDesignLayers";
    let bytes = match (string(p, "html"), string(p, "data")) {
        (Some(html), _) => design_clipboard_buffer(&html).ok_or_else(|| bad(CMD, "the HTML holds no copied design layers"))?,
        (None, Some(d)) => clip_base64(&d).ok_or_else(|| bad(CMD, "`data` is not base64 of at most 256 MB"))?,
        (None, None) => return Err(bad(CMD, "pass `html` or `data`")),
    };
    let other = super::file::load_fig(&bytes, "Pasted layers").ok_or_else(|| bad(CMD, "the data is not copied design layers"))??;
    let doc = std::sync::Arc::new(other.doc);
    let tops: Vec<NodeId> = doc.pages.iter().filter_map(|pg| doc.node(*pg)).flat_map(|pg| pg.children.iter().copied()).collect();
    if tops.is_empty() {
        return Err(EngineError::Disabled(CMD.into(), "the copied layers came in empty".into()));
    }
    let (nodes, images, bounds) = copied(&doc, &tops);
    let props = s.clipboard.props.take();
    // No open file has id 0. So paste treats this copy as a copy from a different file.
    s.clipboard = crate::Clipboard { nodes, images, props, bounds, source_doc: 0, source: Some(doc), source_path: None };
    let mut out = paste(s, p)?;
    if !other.issues.is_empty() {
        out["warnings"] = json!(other.issues);
    }
    Ok(out)
}

fn copy(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = req_targets("edit.copy", s, p)?;
    copy_to_clipboard(s, "edit.copy", &ids)?;
    Ok(json!({"copied": s.clipboard.nodes.len()}))
}

fn cut(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = req_targets("edit.cut", s, p)?;
    copy_to_clipboard(s, "edit.cut", &ids)?;
    let doc = s.doc()?;
    let layers: Vec<NodeId> = ids.into_iter().filter(|i| !doc.node(*i).is_some_and(Node::is_page)).collect();
    delete(s, &json!({"ids": layers}))
}

/// Paste the clipboard under `parent` with the copied top-left moved to `at` (page coords).
pub fn paste_into(s: &mut Session, parent: NodeId, at: Option<(f64, f64)>) -> Result<Vec<NodeId>> {
    let clip = s.clipboard.clone();
    if clip.nodes.is_empty() {
        return Err(EngineError::Disabled("edit.paste".into(), "the clipboard is empty".into()));
    }
    // From another file: its instances follow the mains this file can have.
    let foreign = clip.source.as_ref().filter(|_| s.ds().is_ok_and(|d| d.uid != clip.source_doc));
    let doc = s.doc_mut()?;
    for (k, v) in &clip.images {
        doc.images.entry(k.clone()).or_insert_with(|| v.clone());
    }
    // Sections go on the page (or in sections), never in the frame being pasted into.
    let page = doc.page_of(parent).unwrap_or(parent);
    let shift = match (at, clip.bounds) {
        (Some((x, y)), Some(b)) => Affine::translate((x - b.x0, y - b.y0)),
        _ => Affine::IDENTITY,
    };
    let mut out = Vec::new();
    let mut all = BTreeMap::new();
    for (root, nodes) in &clip.nodes {
        let mut map = BTreeMap::new();
        for n in nodes {
            map.insert(n.id, doc.alloc_id());
        }
        let mut nodes = nodes.clone();
        let section = nodes.first().is_some_and(|r| matches!(r.kind, NodeKind::Section { .. }));
        let into = if section && !doc.takes_sections(parent) { page } else { parent };
        if let Some(r) = nodes.first_mut() {
            r.transform = doc.world_transform(into).inverse() * shift * r.transform;
        }
        out.push(doc.paste_nodes(&nodes, &map, *root, into, None)?);
        all.extend(map);
    }
    if let Some(src) = foreign {
        crate::library::adopt_pasted(doc, src, clip.source_path.as_deref(), &all)?;
    }
    Ok(out)
}

fn paste(s: &mut Session, p: &Value) -> Result<Value> {
    let page = s.page()?;
    // Figma pastes into the selected frame (if one is selected), at the same position when it fits,
    // else in the frame's centre; otherwise in place on the page (or the viewport centre).
    let sel = s.selection();
    let doc = s.doc()?;
    let target = id(p, "parent").or_else(|| {
        sel.first()
            .copied()
            .filter(|i| sel.len() == 1 && doc.node(*i).is_some_and(|n| n.kind.is_frame_like() && !matches!(n.kind, NodeKind::Instance { .. })))
    });
    let parent = target.unwrap_or(page);
    let at = match (num(p, "x"), num(p, "y")) {
        (Some(x), Some(y)) => Some((x, y)),
        _ => {
            let b = s.clipboard.bounds;
            match (target, b) {
                (Some(t), Some(b)) => {
                    let tb = doc.world_bounds(t);
                    match tb {
                        Some(tb) if tb.contains(b.origin()) && tb.contains(figcraft_geom::Point::new(b.x1, b.y1)) => None,
                        Some(tb) => Some((tb.center().x - b.width() / 2.0, tb.center().y - b.height() / 2.0)),
                        None => None,
                    }
                }
                (None, Some(b)) => {
                    // Off-screen originals paste at the viewport centre.
                    let v = &s.ds()?.view;
                    let (ox, oy) = v.origin();
                    let vr = figcraft_geom::Rect::new(ox, oy, ox + v.vw / v.zoom.max(1e-9), oy + v.vh / v.zoom.max(1e-9));
                    if vr.intersect(b).is_zero_area() { Some((v.cx - b.width() / 2.0, v.cy - b.height() / 2.0)) } else { None }
                }
                _ => None,
            }
        }
    };
    let ids = paste_into(s, parent, at)?;
    s.set_selection(ids.clone());
    Ok(json!({"ids": ids}))
}

fn paste_over(s: &mut Session, _p: &Value) -> Result<Value> {
    let sel = s.selection();
    let page = s.page()?;
    let doc = s.doc()?;
    let (parent, at) = match sel.first() {
        Some(i) => {
            let parent = doc.node(*i).and_then(|n| n.parent).unwrap_or(page);
            (parent, doc.world_bounds(*i).map(|b| (b.x0, b.y0)))
        }
        None => (page, None),
    };
    let ids = paste_into(s, parent, at)?;
    s.set_selection(ids.clone());
    Ok(json!({"ids": ids}))
}

fn paste_replace(s: &mut Session, p: &Value) -> Result<Value> {
    let targets = req_targets("edit.pasteReplace", s, p)?;
    let mut out = Vec::new();
    for t in targets {
        let doc = s.doc()?;
        let Some(n) = doc.node(t) else { continue };
        let parent = n.parent.ok_or_else(|| bad("edit.pasteReplace", "can't replace a page"))?;
        let idx = doc.index_in_parent(t);
        let at = doc.world_bounds(t).map(|b| (b.x0, b.y0));
        let ids = paste_into(s, parent, at)?;
        let doc = s.doc_mut()?;
        for (k, i) in ids.iter().enumerate() {
            if let Some(ix) = idx {
                let _ = doc.reorder(*i, ix + k);
            }
        }
        doc.remove(t)?;
        out.extend(ids);
    }
    s.set_selection(out.clone());
    Ok(json!({"ids": out}))
}

fn copy_props_of(doc: &Document, id: NodeId) -> Value {
    let Some(n) = doc.node(id) else { return Value::Null };
    let mut v = json!({
        "fills": n.fills, "strokes": n.strokes, "strokeWeight": n.stroke.weight, "strokeAlign": n.stroke.align,
        "effects": n.effects, "opacity": n.opacity, "blend": n.blend, "cornerRadius": n.corners.radii,
        "cornerSmoothing": n.corners.smoothing,
    });
    if let NodeKind::Text(t) = &n.kind {
        v["fontFamily"] = json!(t.style.font_family);
        v["fontStyle"] = json!(t.style.font_style);
        v["fontSize"] = json!(t.style.font_size);
        v["fontVariations"] = json!(t.style.variations);
        v["lineHeight"] = serde_json::to_value(t.style.line_height).unwrap_or(Value::Null);
        v["letterSpacing"] = serde_json::to_value(t.style.letter_spacing).unwrap_or(Value::Null);
    }
    v
}

fn copy_props(s: &mut Session, p: &Value) -> Result<Value> {
    let id = targets(s, p).first().copied().ok_or_else(|| bad("edit.copyProperties", "select one layer"))?;
    let v = copy_props_of(s.doc()?, id);
    s.clipboard.props = Some(v.clone());
    Ok(v)
}

fn paste_props(s: &mut Session, p: &Value) -> Result<Value> {
    let props = s.clipboard.props.clone().ok_or_else(|| EngineError::Disabled("edit.pasteProperties".into(), "no copied properties".into()))?;
    let ids = req_targets("edit.pasteProperties", s, p)?;
    let doc = s.doc_mut()?;
    for i in ids {
        let mut m = props.as_object().cloned().unwrap_or_default();
        let is_text = matches!(doc.get(i)?.kind, NodeKind::Text(_));
        if !is_text {
            m.retain(|k, _| !k.starts_with("font") && k != "lineHeight" && k != "letterSpacing");
        }
        if !doc.get(i)?.kind.has_corner_radius() {
            m.remove("cornerRadius");
            m.remove("cornerSmoothing");
        }
        super::props::apply("edit.pasteProperties", doc, i, &m)?;
    }
    ok()
}

fn layers_at(s: &mut Session, p: &Value) -> Result<Value> {
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

fn select_all(s: &mut Session, _p: &Value) -> Result<Value> {
    let doc = s.doc()?;
    let sel = s.selection();
    // Select all siblings of the current selection, else the page's top level.
    let parent = sel.first().and_then(|i| doc.node(*i)).and_then(|n| n.parent).unwrap_or(s.page()?);
    let ids: Vec<NodeId> = doc.get(parent)?.children.iter().copied().filter(|c| doc.node(*c).is_some_and(|n| n.visible && !n.locked)).collect();
    s.set_selection(ids.clone());
    Ok(json!({"selection": ids}))
}

/// What Select all would select, less the selection (hidden and locked layers neither).
fn select_inverse(s: &mut Session, _p: &Value) -> Result<Value> {
    let doc = s.doc()?;
    let sel = s.selection();
    let parent = sel.first().and_then(|i| doc.node(*i)).and_then(|n| n.parent).unwrap_or(s.page()?);
    let ids: Vec<NodeId> =
        doc.get(parent)?.children.iter().copied().filter(|c| !sel.contains(c) && doc.node(*c).is_some_and(|n| n.visible && !n.locked)).collect();
    s.set_selection(ids.clone());
    Ok(json!({"selection": ids}))
}

/// `select.area`: what a marquee from (x, y) over the box selects; no box asks the app for one.
fn select_area(s: &mut Session, p: &Value) -> Result<Value> {
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

fn select_children(s: &mut Session, _p: &Value) -> Result<Value> {
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

fn select_parent(s: &mut Session, _p: &Value) -> Result<Value> {
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

fn select_sibling(s: &mut Session, dir: i64) -> Result<Value> {
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

fn select_matching(s: &mut Session, _p: &Value) -> Result<Value> {
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

fn select_same(s: &mut Session, same: impl Fn(&Node, &Node) -> bool) -> Result<Value> {
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

fn opacity_keys(s: &mut Session, p: &Value) -> Result<Value> {
    let digits = req_string("object.opacityKeys", p, "digits")?;
    let o = opacity_from_digits(&digits).ok_or_else(|| bad("object.opacityKeys", "`digits` must be one or two digits"))?;
    for i in req_targets("object.opacityKeys", s, p)? {
        s.doc_mut()?.get_mut(i)?.opacity = o;
    }
    Ok(json!({"opacity": o}))
}

fn nudge(s: &mut Session, p: &Value) -> Result<Value> {
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

fn nudge_size(s: &mut Session, p: &Value) -> Result<Value> {
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
        super::arrange::resize_cmd(s, &Value::Object(q))?;
    }
    ok()
}

fn toggle(s: &mut Session, p: &Value, what: &str) -> Result<Value> {
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

fn hide_others(s: &mut Session, _p: &Value) -> Result<Value> {
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

fn batch_rename(s: &mut Session, p: &Value) -> Result<Value> {
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

fn find_replace(s: &mut Session, p: &Value) -> Result<Value> {
    let find = req_string("edit.findReplace", p, "find")?;
    if find.is_empty() {
        return Err(bad("edit.findReplace", "`find` is empty"));
    }
    // Line breaks stored as the editor stores them (CRLF, CR, U+2028 and U+2029 become LF).
    let replace = string(p, "replace").map(|r| super::props::normalize_line_endings(&r));
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

fn set(s: &mut Session, p: &Value) -> Result<Value> {
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
    // Auto layout keys go to the layers that can have auto layout; with none among the targets,
    // the first refuses them (`props::layout_mut`).
    let layout_keys: Vec<&str> = props.keys().map(String::as_str).filter(|k| super::props::LAYOUT_KEYS.contains(k)).collect();
    let mut without_layout = props.clone();
    let mut skipped = BTreeSet::new();
    if !layout_keys.is_empty() {
        let doc = &s.ds()?.doc;
        let frame_like = |i: &NodeId| doc.node(*i).is_some_and(|n| n.kind.is_frame_like());
        if ids.iter().any(frame_like) {
            skipped = ids.iter().copied().filter(|i| !frame_like(i)).collect();
            without_layout.retain(|k, _| !layout_keys.contains(&k.as_str()));
        }
        if !skipped.is_empty() {
            let names: Vec<String> =
                skipped.iter().take(10).filter_map(|i| doc.node(*i)).map(|n| format!("{} “{}” ({})", n.id, n.name, n.kind.type_name())).collect();
            let more = if skipped.len() > names.len() { format!(" and {} more", skipped.len() - names.len()) } else { String::new() };
            warnings.push(format!(
                "{} skipped on {}{more}: only frames, components and instances have auto layout",
                layout_keys.iter().map(|k| format!("`{k}`")).collect::<Vec<_>>().join(", "),
                names.join(", ")
            ));
        }
    }
    let doc = s.doc_mut()?;
    for i in ids {
        let props = if skipped.contains(&i) { &without_layout } else { &props };
        super::props::apply("node.set", doc, i, props)?;
    }
    if let Some(o) = pivot {
        for (i, q) in pinned {
            let d = o - doc.world_transform(i) * q;
            if d.hypot().is_finite() {
                super::arrange::move_world(doc, i, d.x, d.y)?;
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
fn width_profile(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "stroke.widthProfile";
    let v = get(p, "preset").or_else(|| get(p, "points")).cloned().unwrap_or(Value::Null);
    let ids = req_targets(cmd, s, p)?;
    let props = serde_json::Map::from_iter([("widthProfile".to_string(), v)]);
    let doc = s.doc_mut()?;
    for i in ids {
        super::props::apply(cmd, doc, i, &props)?;
    }
    ok()
}

fn stroke_brush(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "stroke.brush";
    let v = if get(p, "preset").is_none_or(Value::is_null) { Value::Null } else { p.clone() };
    let brush = super::props::brush_of(&v).ok_or_else(|| bad(cmd, super::props::BRUSH_HELP))?;
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

fn reparent(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = req_targets("node.reparent", s, p)?;
    let parent = req_id("node.reparent", p, "parent")?;
    let index = num(p, "index").map(|i| i.max(0.0) as usize);
    let doc = s.doc_mut()?;
    let ids = z_sorted(doc, &roots(doc, &ids));
    // The layers land together under the first sibling at or above `index` that stays put (none: on
    // top), wherever they came from.
    let above = index.and_then(|at| doc.get(parent).ok()?.children.iter().skip(at).find(|c| !ids.contains(c)).copied());
    for i in &ids {
        // Counted with the layer still in place, as `reparent` expects.
        let at = above.and_then(|a| doc.node(parent)?.children.iter().position(|c| *c == a));
        doc.reparent(*i, parent, at)?;
    }
    ok()
}

fn paint_list(s: &mut Session, p: &Value, fills: bool, add: bool) -> Result<Value> {
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
            super::lists::remove_item(n, if fills { super::lists::BoundList::Fills } else { super::lists::BoundList::Strokes }, idx);
        }
    }
    ok()
}

fn paint_set(s: &mut Session, p: &Value, fills: bool) -> Result<Value> {
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
            }
        }
        if let Some(o) = num(p, "opacity") {
            slot.opacity = (if o > 1.0 { o / 100.0 } else { o }).clamp(0.0, 1.0) as f32;
        }
        if let Some(v) = boolean(p, "visible") {
            slot.visible = v;
        }
        if let Some(b) = blend(p, "blend") {
            slot.blend = b;
        }
        super::lists::remap_stop_bindings(n, fills, at, &old);
        if fills {
            n.styles.fill = None;
        } else {
            n.styles.stroke = None;
        }
    }
    ok()
}

fn pattern_fill(s: &mut Session, p: &Value) -> Result<Value> {
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

fn effect_add(s: &mut Session, p: &Value) -> Result<Value> {
    let e: Effect = match get(p, "effect") {
        Some(v) => serde_json::from_value(v.clone()).map_err(|e| bad("effect.add", e.to_string()))?,
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
