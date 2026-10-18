//! Icon fonts: layers turned into glyphs of a font the document carries with it.
//!
//! An icon is a layer whose kind becomes [`NodeKind::Icon`] the way a layer becomes a component: it
//! keeps its place, its size and everything it holds, and those stay editable. Its glyph is the
//! union of the outlines of what it holds — never its own box, which is its surface rather than ink
//! — fitted into the em square by the icon layer's own box and given a code point in the Private Use
//! Area. A layer that draws nothing (hidden, or with no fill) contributes nothing.
//!
//! A set of icons makes an icon font, which [`crate::iconfont`] compiles into a real TTF and
//! registers with the font database, so a text layer shows an icon by picking the family and typing
//! the code point.
//!
//! Icons are addressed by name, with `font` to say which icon font when more than one has an icon of
//! that name. The layers that draw one store its *id*: renaming an icon must not leave them pointing
//! at a name that has moved on, and deleting one must not leave them pointing at nothing.
//!
//! [`NodeKind::Icon`]: figcraft_doc::NodeKind::Icon

use base64::Engine as _;
use figcraft_doc::*;
use figcraft_geom::{Affine, BezPath, Rect};
use serde_json::{Value, json};

use super::CommandSpec;
use super::edit::{roots, z_sorted};
use crate::params::*;
use crate::{Result, Session, bad, spec};

/// The em square outlines are fitted to (the generated font's units per em).
const EM: f64 = figcraft_text::iconfont::UPEM as f64;
/// How much of the em square an icon's outline uses.
///
/// All of it. An icon is a square slot with the drawing in it: the container's longest side becomes
/// the em square, so a square icon comes out a square glyph with nothing around it. Leaving a margin
/// here shrank every icon and put a gap on all four sides of it.
const ICON_FILL: f64 = 1.0;
/// The size a text layer made by [`insert_text`] gets when no size is asked for.
const DEFAULT_TEXT_SIZE: f64 = 24.0;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "icon.create",
            "Create icon",
            "{ids?, font?, name?, codepoint?}  (every selected layer's outline becomes an icon of `font`, fitted to the em square; one icon per layer)",
            true,
            create
        ),
        spec!("icon.update", "Update icon", "{name, font?}  (extract the outline again from the layer the icon was made from)", true, update),
        spec!("icon.detach", "Detach icon", "{ids?}  (icon layers become plain shapes again and their icons leave their fonts)", true, detach),
        spec!("icon.delete", "Delete icon", "{name, font?}", true, delete),
        spec!("icon.rename", "Rename icon", "{name, font?, to}", true, rename),
        spec!("icon.move", "Move icon", "{name, font?, to}  (move the icon to this place in its font, 0-based)", true, move_icon),
        spec!(
            "icon.setCodepoint",
            "Set icon code point",
            "{name, font?, codepoint}  (a number, or a string like `E000` / `U+E000`)",
            true,
            set_codepoint
        ),
        spec!("icon.font.create", "New icon font", "{name?}  (an empty font family for icons)", true, font_create),
        spec!("icon.font.rename", "Rename icon font", "{font, name}", true, font_rename),
        spec!("icon.font.delete", "Delete icon font", "{font}", true, font_delete),
        spec!(
            "icon.generateFont",
            "Generate icon font",
            "{font?}  (compile the icons and register the family; without `font`: every icon font)",
            false,
            generate_font
        ),
        spec!(
            "icon.exportFont",
            "Export icon font",
            "{font, path?}  (the font as a TTF, plus every icon's code point; without `path`: base64 `data`)",
            false,
            export_font
        ),
        spec!(
            "icon.insertText",
            "Insert icon as text",
            "{name, font?, x?, y?, size?}  (a text layer using the icon font at the icon's code point)",
            true,
            insert_text
        ),
        spec!("icon.list", "List icons", "{font?} → the icon fonts, their icons and code points", false, list),
    ]
}

/// The outline of `ids` (and everything under them) in page space, the box it is fitted from, and
/// whether it fills even-odd.
///
/// What an icon is made of is what can be seen: a container contributes the union of what it
/// holds — the paths of the things under it — and never its own box, which is its surface rather
/// than ink. A layer that draws nothing contributes nothing.
pub(crate) fn icon_outline(doc: &Document, ids: &[NodeId]) -> Option<(BezPath, bool, Rect)> {
    let mut out = BezPath::new();
    let mut even_odd = false;
    let mut frame: Option<Rect> = None;
    for i in ids {
        let n = doc.node(*i)?;
        if let Some((mut p, rule)) = icon_path_of(doc, *i, 0) {
            p.apply_affine(doc.world_transform(*i));
            even_odd |= rule == figcraft_render::FillRule::EvenOdd;
            out.extend(p.elements().iter().copied());
        }
        let r = figcraft_geom::transform_rect_bbox(doc.world_transform(*i), n.local_rect());
        frame = Some(frame.map_or(r, |f| f.union(r)));
    }
    let frame = frame?;
    (!out.elements().is_empty()).then_some((out, even_odd, frame))
}

/// What `id` contributes to an icon, in `id`'s own space: what it draws itself, plus everything it
/// holds.
///
/// What it draws itself is its fill. A container's fill is its background, and that belongs to the
/// icon as much as the shapes on it do — a layer with no fill draws nothing on the canvas, so it
/// must not arrive in the font as a solid block, and nothing hidden arrives at any depth.
///
/// The kinds with no geometry of their own take it from what they are made of: a group from what it
/// holds, a boolean from its folded result, text from its glyphs.
fn icon_path_of(doc: &Document, id: NodeId, depth: usize) -> Option<(BezPath, figcraft_render::FillRule)> {
    use figcraft_render::FillRule;
    let n = doc.node(id)?;
    if !n.visible || depth > 64 {
        return None;
    }
    let mut out = BezPath::new();
    let mut even_odd = false;
    if n.fills.iter().any(|f| f.visible)
        && let Some((p, rule)) = own_outline(doc, n, depth)
    {
        out.extend(p.elements().iter().copied());
        even_odd |= rule == FillRule::EvenOdd;
    }
    if !n.hides_contents() {
        for c in n.children.iter().filter_map(|c| doc.node(*c)).filter(|c| c.visible) {
            if let Some((mut p, rule)) = icon_path_of(doc, c.id, depth + 1) {
                p.apply_affine(c.transform);
                even_odd |= rule == FillRule::EvenOdd;
                out.extend(p.elements().iter().copied());
            }
        }
    }
    (!out.elements().is_empty()).then_some((out, if even_odd { FillRule::EvenOdd } else { FillRule::Winding }))
}

/// The path `n` draws as a shape of its own: its glyphs for text, its folded result for a boolean,
/// its geometry for anything else (`None` for the kinds that have none — a group draws only what it
/// holds).
fn own_outline(doc: &Document, n: &Node, depth: usize) -> Option<(BezPath, figcraft_render::FillRule)> {
    match &n.kind {
        NodeKind::Text(_) | NodeKind::BooleanOp { .. } => figcraft_render::subtree_outline(doc, n, depth),
        _ => figcraft_doc::geometry::shape_path(n).map(|p| (p, figcraft_render::FillRule::Winding)),
    }
}

/// Fit an outline into the em square, y still pointing down, centred and using [`ICON_FILL`] of it.
///
/// The fitting is measured on `frame` — the icon layer's own box — and not on the outline: an icon
/// is a window onto its contents, so what the contents are is what moves inside it. Fitting the
/// outline instead would make every icon fill the em square whole, however small its shapes are
/// and wherever they sit, so swapping one shape for another would throw the whole icon across the
/// glyph. A shape with nothing under it has itself for a frame, which fits it exactly as before.
///
/// The contours come out simple and consistently wound, so the outline paints the same under either
/// fill rule — which the font's `glyf` table needs, since it always fills non-zero.
pub(crate) fn to_em_square(path: &BezPath, even_odd: bool, frame: Rect) -> Option<BezPath> {
    let mut clean = figcraft_pathops::normalized(path, even_odd).ok()?;
    let (w, h) = (frame.width(), frame.height());
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return None;
    }
    let s = EM * ICON_FILL / w.max(h);
    let tx = (EM - w * s) / 2.0 - frame.x0 * s;
    let ty = (EM - h * s) / 2.0 - frame.y0 * s;
    clean.apply_affine(Affine::new([s, 0.0, 0.0, s, tx, ty]));
    Some(clean)
}

/// The outline a command should store for `ids`: normalised and fitted to the em square, or the
/// reason there isn't one.
fn outline_for(cmd: &str, doc: &Document, ids: &[NodeId]) -> Result<BezPath> {
    let (outline, even_odd, frame) = icon_outline(doc, ids).ok_or_else(|| bad(cmd, "the selection has nothing to draw"))?;
    to_em_square(&outline, even_odd, frame).ok_or_else(|| bad(cmd, "the selection is empty"))
}

/// A code point from `codepoint`: a number, or a string like `E000`, `U+E000` or `0xE000`.
fn codepoint_param(p: &Value) -> Option<u32> {
    let cp = match p.get("codepoint")? {
        Value::Number(n) => u32::try_from(n.as_u64()?).ok()?,
        Value::String(s) => {
            let t = s.trim();
            let t = t.strip_prefix("U+").or_else(|| t.strip_prefix("u+")).unwrap_or(t);
            let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
            u32::from_str_radix(t, 16).ok()?
        }
        _ => return None,
    };
    char::from_u32(cp).map(|_| cp)
}

/// The index of the icon font to act on: the named one, or — when a command may make one — the
/// only one there is, or a new empty one.
///
/// With several icon fonts and no `font`, the caller has to say which: guessing would put an icon
/// in a font the user didn't mean.
fn ensure_font(cmd: &str, doc: &mut Document, key: Option<&str>, create_if_missing: bool) -> Result<usize> {
    if let Some(k) = key {
        return doc.icon_font_index(k).ok_or_else(|| bad(cmd, format!("no icon font called {k:?}")));
    }
    match doc.icon_fonts.len() {
        0 if create_if_missing => {
            let name = doc.fresh_icon_font_name("Icons");
            let id = doc.fresh_icon_font_id();
            doc.icon_fonts.push(IconFont { id, name, glyphs: Vec::new() });
            Ok(doc.icon_fonts.len() - 1)
        }
        0 => Err(bad(cmd, "this file has no icon fonts")),
        1 => Ok(0),
        n => Err(bad(cmd, format!("{n} icon fonts exist: pass `font` to say which"))),
    }
}

/// The (font, icon) a command targets, by icon name (or id) and optional font.
fn resolve_icon(cmd: &str, doc: &Document, font: Option<&str>, name: &str) -> Result<(usize, usize)> {
    if let Some(k) = font {
        let fi = doc.icon_font_index(k).ok_or_else(|| bad(cmd, format!("no icon font called {k:?}")))?;
        let gi = doc.icon_fonts.get(fi).and_then(|f| f.glyphs.iter().position(|g| g.name == name || g.id == name));
        let fname = doc.icon_fonts.get(fi).map(|f| f.name.clone()).unwrap_or_default();
        return gi.map(|gi| (fi, gi)).ok_or_else(|| bad(cmd, format!("{fname:?} has no icon called {name:?}")));
    }
    let hits: Vec<(usize, usize)> = doc
        .icon_fonts
        .iter()
        .enumerate()
        .flat_map(|(fi, f)| f.glyphs.iter().enumerate().filter(|(_, g)| g.name == name || g.id == name).map(move |(gi, _)| (fi, gi)))
        .collect();
    match hits.as_slice() {
        [] => Err(bad(cmd, format!("no icon called {name:?}"))),
        [one] => Ok(*one),
        _ => Err(bad(cmd, format!("several icon fonts have an icon called {name:?}: pass `font`"))),
    }
}

/// The `(font, icon)` named by the params, or a helpful error.
fn target(cmd: &str, doc: &Document, p: &Value) -> Result<(usize, usize)> {
    let name = req_string(cmd, p, "name")?;
    resolve_icon(cmd, doc, string(p, "font").as_deref(), name.trim())
}

fn create(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "icon.create";
    let ids = req_targets(cmd, s, p)?;
    let doc = s.doc_mut()?;
    let ids = z_sorted(doc, &roots(doc, &ids));
    let asked = string(p, "name");
    let fi = ensure_font(cmd, doc, string(p, "font").as_deref(), true)?;
    // One icon per selected layer: picking three shapes and asking for icons makes three, not one
    // union of them. (`ids` is bottom-to-top, and so are the icons made from it.)
    let mut made = Vec::with_capacity(ids.len());
    let mut taken: Option<u32> = None;
    for id in ids {
        let path = outline_for(cmd, doc, &[id])?;
        let base = asked.clone().or_else(|| doc.node(id).map(|n| n.name.clone())).unwrap_or_default();
        let font = doc.icon_fonts.get(fi).ok_or_else(|| bad(cmd, "no such icon font"))?;
        // An asked-for code point only makes sense for one icon; the rest follow on from it.
        let codepoint = match codepoint_param(p).filter(|_| taken.is_none()) {
            Some(cp) => {
                if let Some(clash) = font.glyph_at(cp) {
                    return Err(bad(cmd, format!("{} already uses that code point", clash.name)));
                }
                cp
            }
            None => font.next_codepoint(),
        };
        let name = font.fresh_glyph_name(&base);
        let gid = font.fresh_glyph_id();
        let font_id = font.id.clone();
        let Some(font) = doc.icon_fonts.get_mut(fi) else { return Err(bad(cmd, "no such icon font")) };
        font.glyphs.push(IconGlyph { id: gid.clone(), name: name.clone(), codepoint, path: path.to_svg(), node: Some(id) });
        taken = Some(codepoint);
        // The layer becomes the icon, exactly as a layer becomes a component: its kind changes and
        // it keeps everything else — place, size, corners, fills and contents. Nothing is taken out
        // of the tree, so the layers it holds stay selectable and editable, and detaching puts the
        // kind back without losing a thing.
        //
        // It stores the icon's *id*, not its name: renaming an icon must not leave every layer
        // that draws it pointing at a name that has moved on.
        let n = doc.get_mut(id)?;
        n.kind = NodeKind::Icon { font: font_id.clone(), glyph: gid.clone() };
        made.push(json!({"id": id, "name": name, "glyph": gid, "codepoint": codepoint}));
    }
    let font = doc.icon_fonts.get(fi).ok_or_else(|| bad(cmd, "no such icon font"))?;
    Ok(json!({
        "font": font.id,
        "fontName": font.name,
        "icons": made,
        // The first icon's id, name and code point, so a single-icon call reads like it always did.
        "id": made.first().map(|i| i["id"].clone()).unwrap_or_default(),
        "name": made.first().map(|i| i["name"].clone()).unwrap_or_default(),
        "glyph": made.first().map(|i| i["glyph"].clone()).unwrap_or_default(),
        "codepoint": made.first().map(|i| i["codepoint"].clone()).unwrap_or_default(),
    }))
}

/// Turn icon nodes back into plain layers and take their icons out of their fonts.
fn detach(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "icon.detach";
    let ids = req_targets(cmd, s, p)?;
    let doc = s.doc_mut()?;
    let ids = z_sorted(doc, &roots(doc, &ids));
    let mut done = Vec::with_capacity(ids.len());
    for id in ids {
        let Some(n) = doc.node(id) else { continue };
        let NodeKind::Icon { font, glyph } = n.kind.clone() else {
            return Err(bad(cmd, "only icon layers can be detached"));
        };
        let fi = doc.icon_font_index(&font).ok_or_else(|| bad(cmd, format!("no icon font called {font:?}")))?;
        // Take the icon out of its font, then put the layer back the way a plain one is: the box it
        // already has, drawn as a rectangle with the fill it had (its outline is no longer the
        // font's). Its size is untouched, so detaching undoes exactly what creating did.
        let removed = doc
            .icon_fonts
            .get_mut(fi)
            .and_then(|f| f.glyphs.iter().position(|g| g.id == glyph || g.name == glyph))
            .and_then(|gi| doc.icon_fonts.get_mut(fi).map(|f| f.glyphs.remove(gi)));
        let n = doc.get_mut(id)?;
        n.kind = NodeKind::Rectangle;
        done.push(json!({"id": id, "name": removed.as_ref().map(|g| g.name.clone()).unwrap_or(glyph), "node": id}));
    }
    Ok(json!({"detached": done}))
}

fn update(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "icon.update";
    let (fi, gi) = target(cmd, s.doc()?, p)?;
    let doc = s.doc_mut()?;
    let node = doc.icon_fonts.get(fi).and_then(|f| f.glyphs.get(gi)).and_then(|g| g.node);
    let node = node.ok_or_else(|| bad(cmd, "this icon wasn't made from a layer, so there's no outline to take again"))?;
    if doc.node(node).is_none() {
        return Err(bad(cmd, "the layer this icon was made from is gone"));
    }
    let path = outline_for(cmd, doc, &[node])?;
    let svg = path.to_svg();
    let (font_id, name) = match doc.icon_fonts.get_mut(fi) {
        Some(f) => {
            let name = f.glyphs.get(gi).map(|g| g.name.clone()).unwrap_or_default();
            if let Some(g) = f.glyphs.get_mut(gi) {
                g.path = svg;
            }
            (f.id.clone(), name)
        }
        None => return Err(bad(cmd, "no such icon font")),
    };
    Ok(json!({"font": font_id, "name": name}))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "icon.delete";
    let (fi, gi) = target(cmd, s.doc()?, p)?;
    let doc = s.doc_mut()?;
    let font_id = doc.icon_fonts.get(fi).map(|f| f.id.clone()).ok_or_else(|| bad(cmd, "no such icon font"))?;
    let gone = {
        let Some(font) = doc.icon_fonts.get_mut(fi) else { return Err(bad(cmd, "no such icon font")) };
        if gi >= font.glyphs.len() {
            return Err(bad(cmd, "no such icon"));
        }
        font.glyphs.remove(gi)
    };
    // The layers that drew it are plain layers again: they must not be left pointing at an icon
    // that has gone.
    let released = release(doc, &font_id, &gone);
    Ok(json!({"font": font_id, "name": gone.name, "codepoint": gone.codepoint, "released": released}))
}

/// Put every layer that draws `gone` of `font_id` back as a plain layer, and say how many: an icon
/// that leaves its font must not leave layers behind pointing at an icon that is gone.
fn release(doc: &mut Document, font_id: &str, gone: &IconGlyph) -> usize {
    let mut ids = if gone.id.is_empty() { Vec::new() } else { doc.icon_nodes_of(font_id, &gone.id) };
    for id in doc.icon_nodes_of(font_id, &gone.name) {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    for id in &ids {
        if let Ok(n) = doc.get_mut(*id) {
            n.kind = NodeKind::Rectangle;
        }
    }
    ids.len()
}

/// Every layer that draws an icon of `font_id` goes back to being a plain layer.
fn release_font(doc: &mut Document, font_id: &str) -> usize {
    let ids = doc.icon_nodes_in(font_id);
    for id in &ids {
        if let Ok(n) = doc.get_mut(*id) {
            n.kind = NodeKind::Rectangle;
        }
    }
    ids.len()
}

fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "icon.rename";
    let to = req_string(cmd, p, "to")?;
    let (fi, gi) = target(cmd, s.doc()?, p)?;
    let doc = s.doc_mut()?;
    let Some(font) = doc.icon_fonts.get(fi) else { return Err(bad(cmd, "no such icon font")) };
    if gi >= font.glyphs.len() {
        return Err(bad(cmd, "no such icon"));
    }
    let name = font.fresh_glyph_name(&to);
    let Some(font) = doc.icon_fonts.get_mut(fi) else { return Err(bad(cmd, "no such icon font")) };
    if let Some(g) = font.glyphs.get_mut(gi) {
        g.name = name.clone();
    }
    Ok(json!({"font": font.id, "name": name}))
}

/// Move an icon to another place in its font: the order the panel lists them in, and the order
/// their glyphs are compiled in.
fn move_icon(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "icon.move";
    let (fi, gi) = target(cmd, s.doc()?, p)?;
    let doc = s.doc_mut()?;
    let Some(font) = doc.icon_fonts.get(fi) else { return Err(bad(cmd, "no such icon font")) };
    if gi >= font.glyphs.len() {
        return Err(bad(cmd, "no such icon"));
    }
    let to = num(p, "to").ok_or_else(|| bad(cmd, "`to` must be a place in the font, 0-based"))?;
    // A place past the end means "last": dragging below the list is how the panel says that.
    let last = font.glyphs.len() - 1;
    let to = (to.max(0.0) as usize).min(last);
    if to == gi {
        let name = font.glyphs.get(gi).map(|g| g.name.clone()).unwrap_or_default();
        return Ok(json!({"font": font.id, "name": name, "to": to}));
    }
    let Some(font) = doc.icon_fonts.get_mut(fi) else { return Err(bad(cmd, "no such icon font")) };
    let g = font.glyphs.remove(gi);
    font.glyphs.insert(to, g);
    Ok(json!({"font": font.id, "name": font.glyphs.get(to).map(|g| g.name.clone()).unwrap_or_default(), "to": to}))
}

fn set_codepoint(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "icon.setCodepoint";
    let (fi, gi) = target(cmd, s.doc()?, p)?;
    let cp = codepoint_param(p).ok_or_else(|| bad(cmd, "`codepoint` must be a code point, e.g. 57344 or \"U+E000\""))?;
    let doc = s.doc_mut()?;
    let Some(font) = doc.icon_fonts.get(fi) else { return Err(bad(cmd, "no such icon font")) };
    if gi >= font.glyphs.len() {
        return Err(bad(cmd, "no such icon"));
    }
    if let Some(taken) = font.glyphs.iter().enumerate().find(|(i, g)| *i != gi && g.codepoint == cp) {
        return Err(bad(cmd, format!("{} already uses that code point", taken.1.name)));
    }
    let Some(font) = doc.icon_fonts.get_mut(fi) else { return Err(bad(cmd, "no such icon font")) };
    if let Some(g) = font.glyphs.get_mut(gi) {
        g.codepoint = cp;
    }
    Ok(json!({"font": font.id, "name": font.glyphs.get(gi).map(|g| g.name.clone()).unwrap_or_default(), "codepoint": cp}))
}

fn font_create(s: &mut Session, p: &Value) -> Result<Value> {
    let doc = s.doc_mut()?;
    let base = string(p, "name").unwrap_or_default();
    let name = doc.fresh_icon_font_name(&base);
    let id = doc.fresh_icon_font_id();
    doc.icon_fonts.push(IconFont { id: id.clone(), name: name.clone(), glyphs: Vec::new() });
    Ok(json!({"font": id, "name": name}))
}

fn font_rename(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "icon.font.rename";
    let key = req_string(cmd, p, "font")?;
    let to = req_string(cmd, p, "name")?;
    let doc = s.doc_mut()?;
    let fi = doc.icon_font_index(&key).ok_or_else(|| bad(cmd, format!("no icon font called {key:?}")))?;
    let name = doc.fresh_icon_font_name(&to);
    let Some(font) = doc.icon_fonts.get_mut(fi) else { return Err(bad(cmd, "no such icon font")) };
    font.name = name.clone();
    Ok(json!({"font": font.id, "name": name}))
}

fn font_delete(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "icon.font.delete";
    let key = req_string(cmd, p, "font")?;
    let doc = s.doc_mut()?;
    let fi = doc.icon_font_index(&key).ok_or_else(|| bad(cmd, format!("no icon font called {key:?}")))?;
    let gone = doc.icon_fonts.remove(fi);
    // Nothing of this font is left to draw, so no layer may be left drawing it.
    let released = release_font(doc, &gone.id);
    Ok(json!({"font": gone.id, "name": gone.name, "icons": gone.glyphs.len(), "released": released}))
}

fn generate_font(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "icon.generateFont";
    let key = string(p, "font");
    let doc = s.doc()?;
    if let Some(k) = key.as_deref()
        && doc.icon_font_index(k).is_none()
    {
        return Err(bad(cmd, format!("no icon font called {k:?}")));
    }
    let empty: Vec<String> = doc
        .icon_fonts()
        .iter()
        .filter(|f| key.as_deref().is_none_or(|k| f.id == k || f.name == k))
        .filter(|f| f.glyphs.is_empty())
        .map(|f| f.name.clone())
        .collect();
    if let Some(name) = empty.first() {
        return Err(bad(cmd, format!("{name:?} has no icons yet")));
    }
    let names: Vec<String> =
        doc.icon_fonts().iter().filter(|f| key.as_deref().is_none_or(|k| f.id == k || f.name == k)).map(|f| f.name.clone()).collect();
    crate::iconfont::forget(key.as_deref());
    let rebuilt = crate::iconfont::sync_icon_fonts(doc);
    Ok(json!({"fonts": names, "rebuilt": rebuilt}))
}

/// Compile a font to bytes and hand them out, with the map of icon name → code point that says
/// what to type to get each one.
fn export_font(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "icon.exportFont";
    let key = req_string(cmd, p, "font")?;
    let doc = s.doc()?;
    let fi = doc.icon_font_index(&key).ok_or_else(|| bad(cmd, format!("no icon font called {key:?}")))?;
    let font = doc.icon_fonts.get(fi).ok_or_else(|| bad(cmd, "no such icon font"))?;
    if font.glyphs.is_empty() {
        return Err(bad(cmd, format!("{:?} has no icons yet", font.name)));
    }
    let family = font.name.clone();
    // The same compile the font database gets, so what is exported is what the canvas shows.
    let mut glyphs: Vec<(u32, BezPath)> = Vec::with_capacity(font.glyphs.len());
    for g in &font.glyphs {
        match BezPath::from_svg(&g.path) {
            Ok(path) => glyphs.push((g.codepoint, path)),
            Err(e) => return Err(bad(cmd, format!("icon {:?} has an unreadable outline: {e}", g.name))),
        }
    }
    let ttf = figcraft_text::iconfont::build_ttf(&family, &glyphs).map_err(|e| bad(cmd, e.to_string()))?;
    // The format to write: what the caller says, or — since a file dialog picks the name — whatever
    // the path ends in. WOFF is the same font with its tables deflated, which is what a web page
    // wants; anything else is the TTF itself.
    let path = string(p, "path");
    let wanted = string(p, "format")
        .map(|f| f.to_ascii_lowercase())
        .or_else(|| path.as_deref().and_then(|p| std::path::Path::new(p).extension().map(|e| e.to_string_lossy().to_ascii_lowercase())));
    let (bytes, format) = match wanted.as_deref() {
        Some("woff") => {
            let woff = figcraft_text::woff::from_ttf(&ttf).ok_or_else(|| bad(cmd, "the font could not be packed as WOFF"))?;
            (woff, "woff")
        }
        _ => (ttf, "ttf"),
    };
    let map: Vec<Value> = font
        .glyphs
        .iter()
        .map(|g| json!({"name": g.name, "codepoint": g.codepoint, "character": char::from_u32(g.codepoint).map(String::from)}))
        .collect();
    match path {
        Some(path) => {
            super::file::write(&path, &bytes)?;
            Ok(json!({"path": path, "font": family, "format": format, "bytes": bytes.len(), "icons": map}))
        }
        None => Ok(json!({
            "font": family,
            "format": format,
            "bytes": bytes.len(),
            "data": base64::engine::general_purpose::STANDARD.encode(&bytes),
            "icons": map,
        })),
    }
}

fn insert_text(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "icon.insertText";
    let (fi, gi) = target(cmd, s.doc()?, p)?;
    let doc = s.doc()?;
    let Some(font) = doc.icon_fonts().get(fi) else { return Err(bad(cmd, "no such icon font")) };
    let Some(glyph) = font.glyphs.get(gi) else { return Err(bad(cmd, "no such icon")) };
    let Some(ch) = char::from_u32(glyph.codepoint) else { return Err(bad(cmd, "the icon's code point is not a character")) };
    let family = font.name.clone();
    let (name, codepoint) = (glyph.name.clone(), glyph.codepoint);
    let size = num(p, "size").filter(|v| v.is_finite() && *v > 0.0).unwrap_or(DEFAULT_TEXT_SIZE).min(5000.0);
    let page = s.page()?;
    let x = num(p, "x").unwrap_or(0.0);
    let y = num(p, "y").unwrap_or(0.0);
    let doc = s.doc_mut()?;
    let id = doc.create(page, NodeKind::Text(TextData::new(ch.to_string())), &name, x, y, size, size)?;
    if let NodeKind::Text(t) = &mut doc.get_mut(id)?.kind {
        t.restyle(0, 1, |st| {
            st.font_family = family.clone();
            st.font_style = "Regular".into();
            st.font_size = size;
        });
    }
    s.set_selection(vec![id]);
    Ok(json!({"id": id, "name": name, "font": family, "codepoint": codepoint, "size": size}))
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    let doc = s.doc()?;
    let key = string(p, "font");
    let fonts: Vec<Value> = doc
        .icon_fonts()
        .iter()
        .filter(|f| key.as_deref().is_none_or(|k| f.id == k || f.name == k))
        .map(|f| {
            let glyphs: Vec<Value> = f
                .glyphs
                .iter()
                .map(|g| {
                    json!({
                        "name": g.name,
                        "codepoint": g.codepoint,
                        "character": char::from_u32(g.codepoint).map(String::from),
                        "path": g.path,
                        "node": g.node,
                    })
                })
                .collect();
            json!({"id": f.id, "name": f.name, "glyphs": glyphs})
        })
        .collect();
    Ok(json!({"fonts": fonts}))
}
