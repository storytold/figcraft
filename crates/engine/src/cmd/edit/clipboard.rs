use super::{
    Affine, BTreeMap, BTreeSet, Document, EngineError, ImageData, Node, NodeId, NodeKind, Result, Session, Value, bad, delete, get, id, json, num,
    ok, req_targets, roots, string, targets, vector_clipboard, z_sorted,
};

/// The copied form of the selection.
pub fn copy_to_clipboard(s: &mut Session, cmd: &str, ids: &[NodeId]) -> Result<()> {
    let ds = s.ds()?;
    // Pages are not layers: they can't be pasted anywhere.
    let layers: Vec<NodeId> = ids.iter().copied().filter(|i| !ds.doc.node(*i).is_some_and(Node::is_page)).collect();
    if layers.is_empty() && !ids.is_empty() {
        return Err(EngineError::Disabled(cmd.into(), "pages can't be copied; use page.duplicate".into()));
    }
    let (nodes, images, bounds) = copied(&ds.doc, &layers);
    store_clipboard(s, cmd, nodes, images, bounds)
}

/// Make `nodes` (the copied layers, each root's transform in page space) the clipboard's layers.
pub(super) fn store_clipboard(
    s: &mut Session,
    cmd: &str,
    nodes: Vec<(NodeId, Vec<Node>)>,
    images: BTreeMap<String, ImageData>,
    bounds: Option<figcraft_geom::Rect>,
) -> Result<()> {
    let ds = s.ds()?;
    let (source, source_path, uid) = (ds.doc.clone(), ds.path.clone(), ds.uid);
    s.clipboard.source = Some(source);
    s.clipboard.source_path = source_path;
    s.clipboard.source_doc = uid;
    s.clipboard.copied_components = nodes
        .iter()
        .filter(|(_, sub)| {
            cmd == "edit.copy" && sub.first().is_some_and(|n| matches!(n.kind, NodeKind::Component { .. } | NodeKind::ComponentSet { .. }))
        })
        .map(|(root, _)| *root)
        .collect();
    s.clipboard.nodes = nodes;
    s.clipboard.images = images;
    s.clipboard.bounds = bounds;
    s.clipboard.layers_generation = s.clipboard.layers_generation.wrapping_add(1);
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
        collect_images(doc, &sub, &mut images);
        nodes.push((*i, sub));
    }
    let bounds = doc.bounds_of(&ids);
    (nodes, images, bounds)
}

/// The images the paints of `nodes` use, taken from `doc` into `images`.
pub(super) fn collect_images(doc: &Document, nodes: &[Node], images: &mut BTreeMap<String, ImageData>) {
    for n in nodes {
        for p in n.fills.iter().chain(n.strokes.iter()) {
            if let figcraft_color::PaintKind::Image(img) = &p.kind
                && let Some(d) = doc.images.get(&img.image)
            {
                images.insert(img.image.clone(), d.clone());
            }
        }
    }
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

/// Paste design layers from clipboard HTML or base64 `data`: decode and reject missing,
/// invalid or oversized data, load a temporary `.fig`, take its top-level page layers,
/// store them as a foreign-file clipboard source with a bumped layers generation, then
/// delegate to `paste` and append import warnings.
pub(super) fn paste_design_layers(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "edit.pasteDesignLayers";
    let bytes = match (string(p, "html"), string(p, "data")) {
        (Some(html), _) => design_clipboard_buffer(&html).ok_or_else(|| bad(CMD, "the HTML holds no copied design layers"))?,
        (None, Some(d)) => clip_base64(&d).ok_or_else(|| bad(CMD, "`data` is not base64 of at most 256 MB"))?,
        (None, None) => return Err(bad(CMD, "pass `html` or `data`")),
    };
    let other = crate::cmd::file::load_fig(&bytes, "Pasted layers").ok_or_else(|| bad(CMD, "the data is not copied design layers"))??;
    let doc = std::sync::Arc::new(other.doc);
    let tops: Vec<NodeId> = doc.pages.iter().filter_map(|pg| doc.node(*pg)).flat_map(|pg| pg.children.iter().copied()).collect();
    if tops.is_empty() {
        return Err(EngineError::Disabled(CMD.into(), "the copied layers came in empty".into()));
    }
    let (nodes, images, bounds) = copied(&doc, &tops);
    // A design-app paste replaces the copied layers, not the copied rows: a `fill.copy` before it
    // still pastes afterwards.
    let props = s.clipboard.props.take();
    let props_source_doc = s.clipboard.props_source_doc;
    let paint = s.clipboard.paint.take();
    let stroke = s.clipboard.stroke.take();
    let paint_images = std::mem::take(&mut s.clipboard.paint_images);
    let paint_source_doc = s.clipboard.paint_source_doc;
    let effect = s.clipboard.effect.take();
    // No open file has id 0. So paste treats this copy as a copy from a different file.
    let layers_generation = s.clipboard.layers_generation.wrapping_add(1);
    s.clipboard = crate::Clipboard {
        nodes,
        images,
        props,
        props_source_doc,
        paint,
        stroke,
        paint_images,
        paint_source_doc,
        effect,
        bounds,
        source_doc: 0,
        source: Some(doc),
        source_path: None,
        copied_components: BTreeSet::new(),
        layers_generation,
    };
    let mut out = paste(s, p)?;
    let pasted: Vec<NodeId> = out["ids"].as_array().map(|a| a.iter().filter_map(crate::params::id_of).collect()).unwrap_or_default();
    let missing = layers_missing_images(s.doc()?, &pasted);
    if !missing.is_empty() {
        out["missingImages"] = json!(missing);
    }
    if !other.issues.is_empty() {
        out["warnings"] = json!(other.issues);
    }
    Ok(out)
}

/// The most layers [`layers_missing_images`] names.
const MAX_MISSING_NAMED: usize = 20;

/// The names of the layers under `ids` with an image fill or stroke whose image the paste didn't
/// carry (#1646): it shows the importer's missing-image placeholder, or nothing at all.
fn layers_missing_images(doc: &figcraft_doc::Document, ids: &[NodeId]) -> Vec<String> {
    let missing = |p: &figcraft_color::Paint| match &p.kind {
        figcraft_color::PaintKind::Image(img) => doc.images.get(&img.image).is_none_or(figcraft_format::rest::is_missing_image),
        _ => false,
    };
    let mut names: Vec<String> = Vec::new();
    for n in ids.iter().flat_map(|i| doc.subtree(*i)).filter_map(|i| doc.node(i)) {
        if n.fills.iter().chain(&n.strokes).any(missing) && !names.contains(&n.name) {
            names.push(n.name.clone());
            if names.len() == MAX_MISSING_NAMED {
                break;
            }
        }
    }
    names
}

/// Whether the command names its layers (`ids` / `id`): it then acts on them even in vector edit mode.
fn names_layers(p: &Value) -> bool {
    get(p, "ids").is_some() || get(p, "id").is_some()
}

pub(super) fn copy(s: &mut Session, p: &Value) -> Result<Value> {
    // In vector edit mode with points or segments selected, Copy takes just those (#1875).
    if !names_layers(p)
        && let Some(sel) = vector_clipboard::edit_selection(s)
        && sel.any
    {
        // Only lone points selected: nothing to copy, and not the whole layer either.
        return vector_clipboard::copy_selection(s, "edit.copy", sel);
    }
    let ids = req_targets("edit.copy", s, p)?;
    copy_to_clipboard(s, "edit.copy", &ids)?;
    Ok(json!({"copied": s.clipboard.nodes.len()}))
}

pub(super) fn cut(s: &mut Session, p: &Value) -> Result<Value> {
    // In vector edit mode with points or segments selected, Cut takes just those and stays in edit mode (#1875).
    if !names_layers(p)
        && let Some(sel) = vector_clipboard::edit_selection(s)
        && sel.any
    {
        // Lone points with nothing to copy: `cut_selection` refuses, as Copy does.
        return vector_clipboard::cut_selection(s, sel);
    }
    let ids = req_targets("edit.cut", s, p)?;
    copy_to_clipboard(s, "edit.cut", &ids)?;
    let doc = s.doc()?;
    let layers: Vec<NodeId> = ids.into_iter().filter(|i| !doc.node(*i).is_some_and(Node::is_page)).collect();
    delete(s, &json!({"ids": layers}))
}

/// Paste the clipboard under `parent` with the copied top-left moved to `at` (page coords). A
/// copied main is pasted as an instance of it; in another file, as an instance of it from the
/// library the source file publishes, and as a copy of it when that library doesn't offer it.
pub fn paste_into(s: &mut Session, parent: NodeId, at: Option<(f64, f64)>) -> Result<Vec<NodeId>> {
    paste_layers(s, parent, at, false)
}

/// [`paste_into`]. With `as_variants`, copied variants pasted into `parent`, their own set, are
/// deep copies (new variants, as Duplicate makes) and the set grows to hold them. Only a plain
/// paste with that set, or a layer in it, selected asks for this; every other paste of a copied
/// main, into another set too, still makes an instance.
fn paste_layers(s: &mut Session, parent: NodeId, at: Option<(f64, f64)>, as_variants: bool) -> Result<Vec<NodeId>> {
    let clip = s.clipboard.clone();
    if clip.nodes.is_empty() {
        return Err(EngineError::Disabled("edit.paste".into(), "the clipboard is empty".into()));
    }
    // From another file: its instances follow the mains this file can have.
    let foreign = clip.source.as_ref().filter(|_| s.ds().is_ok_and(|d| d.uid != clip.source_doc));
    let copy_local = s.ds()?.uid == clip.source_doc;
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
    let into_set = as_variants && copy_local && copied_variants_set(doc, &clip.nodes) == Some(parent);
    let mut out = Vec::new();
    let mut all = BTreeMap::new();
    for (root, nodes) in &clip.nodes {
        let copied_main = nodes.first().filter(|n| clip.copied_components.contains(root) && matches!(n.kind, NodeKind::Component { .. }));
        let main = match (copied_main, foreign) {
            (Some(_), _) if copy_local => {
                (!into_set && doc.node(*root).is_some_and(|n| matches!(n.kind, NodeKind::Component { .. }))).then_some(*root)
            }
            (Some(_), Some(src)) => crate::library::published_main(doc, src, clip.source_path.as_deref(), *root)?,
            _ => None,
        };
        if let (Some(main), Some(n)) = (main, copied_main) {
            let instance = crate::components::create_instance(doc, main, parent, 0.0, 0.0)?;
            doc.get_mut(instance)?.transform = doc.world_transform(parent).inverse() * shift * n.transform;
            keep_a_cropped_images_proportions(doc, instance, parent);
            out.push(instance);
            continue;
        }
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
        let pasted = doc.paste_nodes(&nodes, &map, *root, into, None)?;
        keep_a_cropped_images_proportions(doc, pasted, into);
        out.push(pasted);
        all.extend(map);
    }
    if into_set {
        crate::cmd::component::fit_set(doc, parent);
    }
    // Board connectors pasted with what they join join the pasted copies.
    let originals: Vec<(NodeId, figcraft_doc::BoardItem)> =
        clip.nodes.iter().flat_map(|(_, nodes)| nodes).filter_map(|n| Some((*all.get(&n.id)?, n.board.clone()?))).collect();
    crate::board::rejoin(doc, &originals, &all, !copy_local);
    if let Some(src) = foreign {
        crate::library::adopt_pasted(doc, src, clip.source_path.as_deref(), &all)?;
    }
    crate::library::count_inserted(doc, &out);
    Ok(out)
}

/// Lock the aspect ratio of a layer pasted into an auto layout frame when it shows an image cropped
/// to it and fills one axis while fixed on the other, so layout can't squash the image.
fn keep_a_cropped_images_proportions(doc: &mut Document, pasted: NodeId, parent: NodeId) {
    use figcraft_doc::Sizing;
    if doc.node(parent).is_none_or(|p| p.layout.is_none()) {
        return;
    }
    let Ok(n) = doc.get_mut(pasted) else { return };
    let cropped = n
        .fills
        .iter()
        .any(|f| f.visible && matches!(&f.kind, figcraft_color::PaintKind::Image(i) if i.scale_mode == figcraft_color::ImageScaleMode::Crop));
    let sizing = (n.child_layout.sizing_h, n.child_layout.sizing_v);
    if cropped && matches!(sizing, (Sizing::Fill, Sizing::Fixed) | (Sizing::Fixed, Sizing::Fill)) {
        n.constrain_proportions = true;
    }
}

pub(super) fn paste(s: &mut Session, p: &Value) -> Result<Value> {
    // While editing a vector, the copied layers join its network (#1875); `parent` asks for layers.
    if id(p, "parent").is_none() && s.ds()?.vector_edit.is_some() {
        return vector_clipboard::paste_into_vector(s, p);
    }
    let page = s.page()?;
    // Figma pastes into the selected frame (if one is selected), at the same position when it fits,
    // else in the frame's centre; otherwise in place on the page (or the viewport centre).
    let sel = s.selection();
    let doc = s.doc()?;
    let copy_local = s.ds()?.uid == s.clipboard.source_doc;
    let variants_set = copied_variants_set(doc, &s.clipboard.nodes)
        .filter(|set| copy_local && id(p, "parent").is_none() && !sel.is_empty() && sel.iter().all(|i| doc.is_ancestor(*set, *i)));
    // Where the copied variants are now, not where they were copied: the set may have moved since.
    let below_the_variants = variants_set.and_then(|set| {
        let copied: Vec<NodeId> = s.clipboard.nodes.iter().map(|(root, _)| *root).collect();
        let left = doc.bounds_of(&copied)?.x0;
        let bottom = doc.node(set)?.children.iter().filter_map(|c| doc.world_bounds(*c)).map(|r| r.y1).reduce(f64::max)?;
        Some((left, bottom + 20.0))
    });
    let target = id(p, "parent").or(variants_set).or_else(|| {
        sel.first()
            .copied()
            .filter(|i| {
                sel.len() == 1
                    && doc.node(*i).is_some_and(|n| n.kind.is_frame_like() && !matches!(n.kind, NodeKind::Instance { .. }) && !n.is_board_atom())
            })
            .filter(|i| !copy_local || !s.clipboard.copied_components.iter().any(|root| doc.is_ancestor(*root, *i)))
    });
    // One layer selected that isn't a frame target: the copy joins its parent, just above it (#2355).
    let sibling_of = if target.is_none() && id(p, "parent").is_none() { sibling_anchor(s, &sel, copy_local) } else { None };
    let target = target.or_else(|| sibling_of.and_then(|i| doc.node(i).and_then(|n| n.parent)));
    let parent = target.unwrap_or(page);
    let at = match (num(p, "x"), num(p, "y")) {
        (Some(x), Some(y)) => Some((x, y)),
        _ if below_the_variants.is_some() => below_the_variants,
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
    let ids = paste_layers(s, parent, at, variants_set.is_some())?;
    if let Some(sel) = sibling_of {
        let above = s.doc()?.index_in_parent(sel).map(|ix| ix + 1);
        move_above(s, &ids, parent, above)?;
    }
    s.set_selection(ids.clone());
    Ok(json!({"ids": ids}))
}

/// The one selected layer a plain paste goes next to: not a page-level layer, not inside an instance
/// (its children are fixed), not a board atom, a component set (a variant's parent) or inside a copied
/// component. Else the page, as before.
fn sibling_anchor(s: &Session, sel: &[NodeId], copy_local: bool) -> Option<NodeId> {
    let [only] = sel else { return None };
    let doc = s.doc().ok()?;
    let parent = doc.node(*only)?.parent?;
    let pn = doc.node(parent)?;
    let ok = !pn.is_page()
        && pn.kind.is_frame_like()
        && !pn.is_board_atom()
        && !crate::components::inside_instance(doc, *only)
        && !matches!(pn.kind, NodeKind::Instance { .. } | NodeKind::ComponentSet { .. })
        && (!copy_local || !s.clipboard.copied_components.iter().any(|root| doc.is_ancestor(*root, parent)));
    ok.then_some(*only)
}

/// Move the pasted copies that landed in `parent` to `above` and after, in the copied order.
fn move_above(s: &mut Session, ids: &[NodeId], parent: NodeId, above: Option<usize>) -> Result<()> {
    let Some(above) = above else { return Ok(()) };
    let doc = s.doc_mut()?;
    for (k, i) in ids.iter().filter(|i| doc.node(**i).is_some_and(|n| n.parent == Some(parent))).copied().collect::<Vec<_>>().into_iter().enumerate()
    {
        doc.reorder(i, above + k)?;
    }
    Ok(())
}

/// The set the copied layers are variants of: every copied root is a main component in `doc` whose
/// parent is that one component set. `None` for anything else.
fn copied_variants_set(doc: &Document, copied: &[(NodeId, Vec<Node>)]) -> Option<NodeId> {
    let mut sets = copied.iter().map(|(root, _)| {
        let n = doc.node(*root).filter(|n| matches!(n.kind, NodeKind::Component { .. }))?;
        n.parent.filter(|set| doc.node(*set).is_some_and(|p| matches!(p.kind, NodeKind::ComponentSet { .. })))
    });
    let first = sets.next()??;
    sets.all(|set| set == Some(first)).then_some(first)
}

/// Paste over selection (Shift+Cmd/Ctrl+V): the copies go into the parent of the selected layer
/// [`last_seen`] picks, just above it, at the place it gives.
pub(super) fn paste_over(s: &mut Session, _p: &Value) -> Result<Value> {
    let sel = s.selection();
    let page = s.page()?;
    let doc = s.doc()?;
    let (parent, at, above) = match last_seen(doc, &sel) {
        Some((i, at)) => {
            let parent = doc.node(i).and_then(|n| n.parent).unwrap_or(page);
            (parent, at, doc.index_in_parent(i).map(|ix| ix + 1))
        }
        None => (page, None, None),
    };
    let ids = paste_into(s, parent, at)?;
    move_above(s, &ids, parent, above)?;
    s.set_selection(ids.clone());
    Ok(json!({"ids": ids}))
}

/// The selected layer Paste over selection pastes after, and where the copies' top-left goes.
///
/// One layer: that layer, at its position. Several in one auto layout frame's flow (none absolutely
/// positioned): the last of them in the flow, which the layout follows. Several elsewhere: the lowest (then rightmost), with the copies
/// below it, as far below as it is from the selected layers above it. A selection with nothing above
/// its lowest layer is a row: the copies go right of the rightmost, as far as it is from its
/// neighbour on the left. Layers that overlap every way get the copies right under the lowest.
fn last_seen(doc: &Document, sel: &[NodeId]) -> Option<(NodeId, Option<(f64, f64)>)> {
    let first = *sel.first()?;
    let top_left = |i: NodeId| doc.world_bounds(i).map(|b| (b.x0, b.y0));
    if sel.len() == 1 {
        return Some((first, top_left(first)));
    }
    let parent = doc.node(first).and_then(|n| n.parent);
    let one_flow = parent.and_then(|p| doc.node(p)).is_some_and(|p| p.layout.is_some())
        && sel.iter().all(|i| doc.node(*i).is_some_and(|n| n.parent == parent && !n.child_layout.absolute));
    if one_flow {
        let last = sel.iter().copied().max_by_key(|i| doc.index_in_parent(*i))?;
        return Some((last, top_left(last)));
    }
    let boxes: Vec<(NodeId, figcraft_geom::Rect)> = sel.iter().filter_map(|i| Some((*i, doc.world_bounds(*i)?))).collect();
    let (lowest, low) = *boxes.iter().max_by(|(_, a), (_, b)| a.y1.total_cmp(&b.y1).then(a.x1.total_cmp(&b.x1)))?;
    if let Some(above) = boxes.iter().filter(|(_, o)| o.y1 <= low.y0).map(|(_, o)| o.y1).max_by(f64::total_cmp) {
        return Some((lowest, Some((low.x0, low.y1 + (low.y0 - above)))));
    }
    let (rightmost, right) = *boxes.iter().max_by(|(_, a), (_, b)| a.x1.total_cmp(&b.x1).then(a.y1.total_cmp(&b.y1)))?;
    match boxes.iter().filter(|(_, o)| o.x1 <= right.x0).map(|(_, o)| o.x1).max_by(f64::total_cmp) {
        Some(left) => Some((rightmost, Some((right.x1 + (right.x0 - left), right.y0)))),
        None => Some((lowest, Some((low.x0, low.y1)))),
    }
}

pub(super) fn paste_replace(s: &mut Session, p: &Value) -> Result<Value> {
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

pub(super) fn copy_props_of(doc: &Document, id: NodeId) -> Value {
    let Some(n) = doc.node(id) else { return Value::Null };
    let mut v = json!({
        "fills": n.fills, "strokes": n.strokes, "strokeWeight": n.stroke.weight, "strokeAlign": n.stroke.align,
        "strokeSides": n.stroke.sides, "startCap": n.stroke.cap_start, "endCap": n.stroke.cap_end,
        "strokeJoin": n.stroke.join, "miterLimit": n.stroke.miter_limit, "dashes": n.stroke.dashes,
        "dashCap": n.stroke.dash_cap,
        "widthProfile": super::super::props::width_points_json(n.stroke.width_profile.as_ref()), "brush": n.stroke.brush,
        "effects": n.effects, "opacity": n.opacity, "blend": n.blend,
    });
    // A vector rounds each anchor on its own (`vector.edit.cornerRadius` keeps the radii in the
    // network's vertices, its layer corners stay empty): there is no style radius to carry, and
    // pasting the empty one would flatten every anchor's radius.
    if !matches!(n.kind, NodeKind::Vector { .. }) {
        v["cornerRadius"] = json!(n.corners.radii);
        v["cornerSmoothing"] = json!(n.corners.smoothing);
    }
    // A frame's layout grids go with its look, as its fills do.
    if n.kind.is_frame_like() {
        v["grids"] = json!(n.grids);
        v["gridStyle"] = json!(n.styles.grid);
    }
    // The links behind the values: the styles and the variables they come from, so the pasted
    // layer follows them as the copied one does.
    v["fillStyle"] = json!(n.styles.fill);
    v["strokeStyle"] = json!(n.styles.stroke);
    v["effectStyle"] = json!(n.styles.effect);
    v["boundVariables"] = Value::Object(n.bound.iter().filter(|(f, _)| copied_binding(f)).map(|(f, v)| (f.clone(), json!(v))).collect());
    if let NodeKind::Text(t) = &n.kind {
        v["textStyle"] = json!(n.styles.text);
        v["fontFamily"] = json!(t.style.font_family);
        v["fontStyle"] = json!(t.style.font_style);
        v["fontSize"] = json!(t.style.font_size);
        v["fontVariations"] = json!(t.style.variations);
        v["lineHeight"] = serde_json::to_value(t.style.line_height).unwrap_or(Value::Null);
        v["letterSpacing"] = serde_json::to_value(t.style.letter_spacing).unwrap_or(Value::Null);
    }
    v
}

pub(super) fn copy_props(s: &mut Session, p: &Value) -> Result<Value> {
    let id = targets(s, p).first().copied().ok_or_else(|| bad("edit.copyProperties", "select one layer"))?;
    let v = copy_props_of(s.doc()?, id);
    s.clipboard.props = Some(v.clone());
    s.clipboard.props_source_doc = s.ds()?.uid;
    Ok(v)
}

/// Copied properties `props` without their style and variable links, for a paste into another
/// file: the copied ids name the source file's styles and variables, and the same ids here can
/// name unrelated ones. The values paste unlinked, and the layer's own links on them go.
fn without_links(mut props: Value) -> Value {
    if let Some(m) = props.as_object_mut() {
        for (k, v) in m.iter_mut() {
            if k.ends_with("Style") && !k.starts_with("font") {
                *v = Value::Null;
            } else if k == "boundVariables" {
                *v = json!({});
            }
        }
    }
    props
}

/// Give layer `id` the properties [`copy_props_of`] copied (`props`), those it can take: the text
/// properties only to text, the corners only to a layer with corners.
pub(crate) fn apply_copied_props(cmd: &str, doc: &mut Document, id: NodeId, props: &Value) -> Result<()> {
    let mut m = props.as_object().cloned().unwrap_or_default();
    let n = doc.get(id)?;
    let (is_text, corners, frame) = (matches!(n.kind, NodeKind::Text(_)), n.kind.has_corner_radius(), n.kind.is_frame_like());
    let takes = |k: &str| {
        let text = k.starts_with("font") || k.starts_with("paragraph") || matches!(k, "lineHeight" | "letterSpacing" | "textStyle");
        let corner = k == "cornerSmoothing" || k.ends_with("Radius");
        let grid = k == "gridStyle" || k == "grids" || k.starts_with("layoutGrids/");
        (is_text || !text) && (corners || !corner) && (frame || !grid)
    };
    m.retain(|k, _| takes(k));
    // A style or variable the file doesn't have (deleted since the copy) isn't here to link to: the
    // values it gave are pasted without a link, so the layer's own link goes too.
    let here = |v: &Value| v.as_str().is_none_or(|sid| doc.style(sid).is_some());
    for (k, v) in m.iter_mut() {
        if k.ends_with("Style") && !k.starts_with("font") && !here(v) {
            *v = Value::Null;
        }
    }
    if let Some(Value::Object(copied)) = m.get_mut("boundVariables") {
        copied.retain(|_, v| v.as_str().is_some_and(|vid| doc.variables.get_by_id(vid).is_some()));
    }
    // The layer's own bindings on what is pasted give way to the copied layer's: kept, a binding
    // would hold the old value over the pasted one. A binding on what the copy doesn't carry (a
    // text layer's font size, pasted from a frame) stays.
    if let Some(Value::Object(copied)) = m.get("boundVariables") {
        let mut bound: serde_json::Map<String, Value> =
            n.bound.keys().filter(|f| copied_binding(f) && m.contains_key(pasted_key(f))).map(|f| (f.clone(), Value::Null)).collect();
        bound.extend(copied.iter().filter(|(f, _)| takes(f)).map(|(f, v)| (f.clone(), v.clone())));
        m.insert("boundVariables".into(), Value::Object(bound));
    }
    crate::cmd::props::apply(cmd, doc, id, &m)
}

/// The key of the properties [`copy_props_of`] copies that sets what a variable bound to `field`
/// sets: a corner's radius is `cornerRadius`, a side's stroke weight `strokeWeight`, the font
/// weight `fontStyle`, a layout grid's field `grids` and a paint's or effect's field its list.
fn pasted_key(field: &str) -> &str {
    match field {
        "topLeftRadius" | "topRightRadius" | "bottomRightRadius" | "bottomLeftRadius" => "cornerRadius",
        f if crate::cmd::props::STROKE_SIDES.contains(&f) => "strokeWeight",
        "fontWeight" => "fontStyle",
        f => match f.split_once('/') {
            Some(("layoutGrids", _)) => "grids",
            Some((list, _)) => list,
            None => f,
        },
    }
}

/// Whether a variable bound to `field` goes with the properties [`copy_props_of`] copies: not a
/// size, auto layout spacing, visibility or text content.
fn copied_binding(field: &str) -> bool {
    !matches!(
        field,
        "width"
            | "height"
            | "minWidth"
            | "maxWidth"
            | "minHeight"
            | "maxHeight"
            | "gap"
            | "columnGap"
            | "rowGap"
            | "crossGap"
            | "padding"
            | "paddingHorizontal"
            | "paddingVertical"
            | "paddingTop"
            | "paddingRight"
            | "paddingBottom"
            | "paddingLeft"
            | "visible"
            | "characters"
    )
}

pub(super) fn paste_props(s: &mut Session, p: &Value) -> Result<Value> {
    let mut props = s.clipboard.props.clone().ok_or_else(|| EngineError::Disabled("edit.pasteProperties".into(), "no copied properties".into()))?;
    if s.ds()?.uid != s.clipboard.props_source_doc {
        props = without_links(props);
    }
    let ids = req_targets("edit.pasteProperties", s, p)?;
    let doc = s.doc_mut()?;
    for i in ids {
        apply_copied_props("edit.pasteProperties", doc, i, &props)?;
    }
    ok()
}
