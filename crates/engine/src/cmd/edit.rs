//! Edit menu: undo/redo, clipboard, duplicate/delete, selection commands, find, properties.

use std::collections::{BTreeMap, BTreeSet};

use figcraft_doc::{Document, Effect, ExportSetting, ImageData, LayoutGrid, Node, NodeId, NodeKind};
use figcraft_geom::Affine;
use serde_json::{Map, Value, json};

use super::{CommandSpec, ok};
use crate::params::{
    blend, boolean, color, fraction_of, get, id, id_of, num, num_or, opt_id, opt_index, paint_of, parse, req_id, req_index, req_num, req_string,
    req_targets, string, targets,
};
use crate::{BatchStep, EngineError, Result, Session, bad, spec};

mod clipboard;
mod props;
mod select;

pub use clipboard::{CLIP_CLOSE, CLIP_OPEN, copy_to_clipboard, design_clipboard_buffer, paste_into};
use clipboard::{copy, copy_props, copy_props_of, cut, paste, paste_design_layers, paste_over, paste_props, paste_replace};
use props::{
    batch_rename, effect_add, effect_copy, effect_paste, find_replace, paint_copy, paint_list, paint_paste, paint_set, pattern_fill, reparent, set,
    stroke_brush, width_profile,
};
pub use select::opacity_from_digits;
use select::{
    hide_others, layers_at, nudge, nudge_size, opacity_keys, select_all, select_area, select_children, select_inverse, select_matching,
    select_parent, select_same, select_sibling, toggle,
};

/// The layer types `node.find` takes, as `NodeKind::type_name` spells them.
const LAYER_TYPES: [&str; 16] = [
    "PAGE",
    "FRAME",
    "GROUP",
    "SECTION",
    "RECTANGLE",
    "ELLIPSE",
    "POLYGON",
    "STAR",
    "LINE",
    "VECTOR",
    "TEXT",
    "BOOLEAN_OPERATION",
    "COMPONENT",
    "COMPONENT_SET",
    "INSTANCE",
    "SLICE",
];

/// Lowercase for `node.find`'s substring filters. Greek final sigma becomes σ on both sides, so a
/// query lowercased on its own (`Σ` to `σ`) still matches a name ending in it (`ΟΣ` to `ος`).
fn fold(s: &str) -> String {
    s.to_lowercase().replace('ς', "σ")
}

fn node_find(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "node.find";
    let filter = |key| -> Result<Option<String>> {
        get(p, key).map(|v| v.as_str().map(fold).ok_or_else(|| bad(cmd, format!("`{key}` must be a string")))).transpose()
    };
    let name = filter("name")?;
    let text = filter("text")?;
    let component = filter("component")?;
    let kind = filter("type")?;
    if let Some(kind) = &kind
        && !LAYER_TYPES.iter().any(|t| t.eq_ignore_ascii_case(kind))
    {
        return Err(bad(cmd, format!("unknown type; use one of {}", LAYER_TYPES.join(", "))));
    }
    let limit = match get(p, "limit") {
        None => 100,
        Some(v) => v.as_u64().filter(|n| *n <= 10000).ok_or_else(|| bad(cmd, "`limit` must be an integer from 0 to 10000"))? as usize,
    };
    let root = opt_id(cmd, p, "root")?.map_or_else(|| s.page(), Ok)?;
    let doc = s.doc()?;
    // Keep one child iterator per ancestor, not a materialised subtree or all pending siblings.
    let mut stack = vec![doc.get(root)?.children.iter()];
    let mut ids = Vec::new();
    let mut total = 0usize;
    while let Some(children) = stack.last_mut() {
        let Some(&id) = children.next() else {
            stack.pop();
            continue;
        };
        let n = doc.get(id)?;
        stack.push(n.children.iter());
        if name.as_ref().is_some_and(|q| !fold(&n.name).contains(q))
            || kind.as_ref().is_some_and(|q| !n.kind.type_name().eq_ignore_ascii_case(q))
            || text.as_ref().is_some_and(|q| !matches!(&n.kind, NodeKind::Text(t) if fold(&t.characters).contains(q)))
            || component.as_ref().is_some_and(
                |q| !matches!(&n.kind, NodeKind::Instance { component, .. } if doc.node(*component).is_some_and(|c| fold(&c.name).contains(q))),
            )
        {
            continue;
        }
        total += 1;
        if ids.len() < limit {
            ids.push(id);
        }
    }
    Ok(json!({"ids": ids, "total": total}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!("edit.undo", "Undo", "{}", false, |s, _| Ok(json!({"undone": s.undo()?}))),
        spec!("edit.redo", "Redo", "{}", false, |s, _| Ok(json!({"redone": s.redo()?}))),
        spec!("edit.delete", "Delete", "{ids?}", true, delete),
        spec!(
            "edit.duplicate",
            "Duplicate",
            "{ids?, dx?, dy? (page offset; default: the offset of the last duplicate when ids are its copies, still selected; else top-level frames to the right with a 100 gap, everything else in place), map? (true: also return `map`, each original layer's id, descendants included, → its copy's id)}  a main component's copy is an instance of it (a variant stays a new variant in its set)",
            true,
            duplicate
        ),
        spec!("edit.copy", "Copy", "{ids?}", false, copy),
        spec!("edit.cut", "Cut", "{ids?}", true, cut),
        spec!(
            "edit.paste",
            "Paste",
            "{parent?, x?, y? (page coordinates of the pasted top-left)}  a copied main component pastes as an instance of it within its file (cut ones, and pastes into other files, keep the component)",
            true,
            paste
        ),
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
        spec!("node.get", "Get layer", "{id, fields?: [node keys], outline?: bool (not with fields)}", false, |s, p| {
            let i = req_id("node.get", p, "id")?;
            let options = crate::inspect::InspectOptions::parse(p).map_err(|e| bad("node.get", e))?;
            Ok(crate::inspect::node_json_with_options(s.doc()?, i, 1, &options))
        }),
        spec!(
            "node.find",
            "Find layers",
            "{root? (strict descendants; default current page), name?, type? (canonical layer type), text?, component? (main component name), limit? (0-10000; default 100)}: every filter must match; name, text and component are case-insensitive substrings, type is exact (any case); returns {ids, total} and does not select",
            false,
            node_find
        ),
        spec!(
            "node.reparent",
            "Move layer into",
            "{ids?, parent, index? (final position among parent's children, bottom first; default top)}",
            true,
            reparent
        ),
        spec!("node.reorder", "Reorder layer", "{id, index}", true, |s, p| {
            let i = req_id("node.reorder", p, "id")?;
            let idx = req_num("node.reorder", p, "index")?.max(0.0) as usize;
            s.doc_mut()?.reorder(i, idx)?;
            ok()
        }),
        spec!("fill.add", "Add fill", "{ids?, paint? | color? (default #D9D9D9 or white for frames)}", true, |s, p| paint_list(s, p, true, true)),
        spec!("stroke.add", "Add stroke", "{ids?, paint? | color? (default black)}", true, |s, p| paint_list(s, p, false, true)),
        spec!("fill.remove", "Remove fill", "{ids?, index}", true, |s, p| paint_list(s, p, true, false)),
        spec!("stroke.remove", "Remove stroke", "{ids?, index}", true, |s, p| paint_list(s, p, false, false)),
        spec!("fill.set", "Set fill", "{ids?, index?, paint | color, opacity?, visible?, blend?}", true, |s, p| paint_set(s, p, true)),
        spec!("stroke.set", "Set stroke", "{ids?, index?, paint | color, opacity?, visible?, blend?}", true, |s, p| paint_set(s, p, false)),
        spec!("fill.copy", "Copy fill", "{id?, index? (default the top fill)}", false, |s, p| paint_copy(s, p, true)),
        spec!("fill.paste", "Paste fill", "{ids?, index? (replace the fill there; without: add one)}", true, |s, p| paint_paste(s, p, true)),
        spec!(
            "stroke.copy",
            "Copy stroke",
            "{id?, index? (default the top stroke)}  its paint and the stroke's weight, alignment, sides, caps, joins and dashes",
            false,
            |s, p| paint_copy(s, p, false)
        ),
        spec!(
            "stroke.paste",
            "Paste stroke",
            "{ids?, index? (replace the stroke there; without: add one)}  the copied stroke's settings come with its paint",
            true,
            |s, p| paint_paste(s, p, false)
        ),
        spec!(
            "fill.pattern",
            "Pattern fill",
            "{source (layer id), ids?, index? (default the top fill), tile? (grid|horizontalHex|verticalHex), scale? (1 = the source's size), spacing? [x, y], align? (start|center|end), opacity?, visible?, blend?}  the source layer tiled; it updates as the source changes",
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
            let e = super::props::effect_from("effect.set", p, "effect")?.ok_or_else(|| bad("effect.set", "`effect` required"))?;
            for id in req_targets("effect.set", s, p)? {
                let n = s.doc_mut()?.get_mut(id)?;
                if let Some(slot) = n.effects.get_mut(i) {
                    *slot = e.clone();
                }
            }
            ok()
        }),
        spec!("effect.copy", "Copy effect", "{id?, index? (default the top effect)}", false, effect_copy),
        spec!("effect.paste", "Paste effect", "{ids?, index? (replace the effect there; without: add one)}", true, effect_paste),
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
            "{ids?, setting? {size: 2x|512w|512h|{by: scale|width|height, value}, suffix?, format?: png|jpg|svg|pdf|webp|xml (Android Vector Drawable), bleed? (PDF: points, or with a unit: \"3mm\", \"0.125in\", \"9pt\"), cropMarks?, bleedMarks?, cmyk? (PDF print options), vdCompat? (XML: bestEffort|exact|ignore)}} (without a setting: 1x, then one scale step up with an \"@2x\"-style suffix)",
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
            "{ids?, index, size? (2x|512w|512h|{by, value}), suffix?, format? (png|jpg|svg|pdf|webp|xml: Android Vector Drawable), bleed? (PDF: points, or with a unit: \"3mm\", \"0.125in\", \"9pt\"), cropMarks?, bleedMarks?, cmyk? (PDF print options), vdCompat? (XML: bestEffort|exact|ignore), contentsOnly?, includeBoundingBox?, svgOutlineText?, svgIdAttribute?, svgSimplifyStroke?}",
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
        spec!("command.batch", "Run several commands", "{label?, onError?, commands: [{command, params, as?}]} (one undo step)", false, |s, p| {
            // `continue` rolls back only the failed steps and returns one outcome per step.
            let continue_on_error = match get(p, "onError") {
                None => false,
                Some(Value::String(mode)) if mode == "abort" => false,
                Some(Value::String(mode)) if mode == "continue" => true,
                Some(_) => return Err(bad("command.batch", "`onError` must be \"abort\" or \"continue\"")),
            };
            let label = string(p, "label").unwrap_or_else(|| "Batch".into());
            let cmds: Vec<BatchStep> = match get(p, "commands") {
                Some(Value::Array(a)) => {
                    let mut out = Vec::new();
                    for (i, c) in a.iter().enumerate() {
                        let cmd =
                            c.get("command").and_then(Value::as_str).ok_or_else(|| bad("command.batch", format!("step {i} has no `command`")))?;
                        crate::batch::refuse_unknown_step_keys(i, c)?;
                        let params = c.get("params").cloned().unwrap_or(json!({}));
                        out.push(BatchStep { name: crate::batch::step_name(i, c)?, ..BatchStep::new(cmd, params) });
                    }
                    out
                }
                _ => return Err(bad("command.batch", "`commands` must be an array")),
            };
            Ok(Value::Array(if continue_on_error { s.batch_continue(&label, &cmds)? } else { s.batch(&label, &cmds)? }))
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
        let nid = crate::components::copy_in_file(doc, i, parent, idx)?;
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
        // An instance keeps the name instances get by default.
        if rename && !matches!(doc.get(nid)?.kind, NodeKind::Instance { .. }) {
            let nn = doc.get_mut(nid)?;
            nn.name = bump_name(&nn.name);
        }
        out.push(nid);
    }
    // Board connectors copied with what they join join the copies.
    let map = crate::board::copy_map(doc, &ids, &out);
    crate::board::rejoin(doc, &crate::board::copied_items(doc, &map), &map, false);
    s.set_selection(out.clone());
    s.ds_mut()?.last_duplicate = Some(Duplicated { sources: ids, copies: out.clone() });
    if boolean(p, "map") == Some(true) {
        // JSON object keys are strings: each original layer's id → its copy's.
        let map: Map<String, Value> = map.into_iter().map(|(o, c)| (o.to_string(), json!(c))).collect();
        return Ok(json!({"ids": out, "map": map}));
    }
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
