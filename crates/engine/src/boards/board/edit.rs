use super::connectors::connector_of;
use super::derived::ends_now;
use super::items::{add, add_text, shape_padding, text_of};
use super::{
    Affine, AutoLayout, BTreeMap, BoardItem, BoardShape, Color, Connector, DocError, Document, End, INK, NodeId, NodeKind, Paint, Result, Routing,
    Session, Sizing, StrokeCap, TextAlignH, TextAutoResize, TextData, Value, VectorNetwork, bad, cap_of, color_param, json, line_shade, num,
    req_string, req_targets, require, routing_of, shape_of, string,
};

/// The label of connector `id`, if it has one.
pub fn label_of(doc: &Document, id: NodeId) -> Option<NodeId> {
    let parent = doc.node(id)?.parent?;
    doc.node(parent)?.children.iter().copied().find(|c| doc.node(*c).is_some_and(|n| n.board == Some(BoardItem::Label { connector: id })))
}

/// Give connector `id` the label `text` (empty removes it); the label's id.
pub(super) fn set_label(doc: &mut Document, id: NodeId, text: &str) -> Result<Option<NodeId>> {
    let existing = label_of(doc, id);
    if text.trim().is_empty() {
        if let Some(l) = existing {
            doc.remove(l)?;
        }
        return Ok(None);
    }
    if let Some(l) = existing {
        if let Some(t) = text_of(doc, l)
            && let Some(NodeKind::Text(td)) = doc.node_mut(t).map(|n| &mut n.kind)
        {
            let len = td.char_count();
            td.replace_range(0, len, text);
        }
        return Ok(Some(l));
    }
    let parent = doc.node(id).and_then(|n| n.parent).ok_or(DocError::NoNode(id))?;
    let l = add(doc, parent, NodeKind::Frame, "Label", (0.0, 0.0), (40.0, 24.0), |n| {
        n.fills = vec![Paint::solid(Color::WHITE)];
        n.corners.radii = [4.0; 4];
        n.clip = false;
        n.layout = Some(AutoLayout { padding: [3.0, 6.0, 3.0, 6.0], ..AutoLayout::default() });
        n.child_layout.sizing_h = Sizing::Hug;
        n.child_layout.sizing_v = Sizing::Hug;
        n.board = Some(BoardItem::Label { connector: id });
    })?;
    let t = add_text(doc, l, text, 12.0, INK, TextAlignH::Center, 40.0)?;
    if let Some(n) = doc.node_mut(t)
        && let NodeKind::Text(td) = &mut n.kind
    {
        // The label hugs its text: one line per paragraph.
        td.auto_resize = TextAutoResize::WidthAndHeight;
        n.child_layout.sizing_h = Sizing::Hug;
    }
    Ok(Some(l))
}

pub(super) fn label_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "board.label";
    require(s, cmd)?;
    let id = req_targets(cmd, s, p)?.first().copied().ok_or_else(|| bad(cmd, "select a connector"))?;
    let (id, _) = connector_of(cmd, s.doc()?, id)?;
    let text = req_string(cmd, p, "text")?;
    let label = set_label(s.doc_mut()?, id, &text)?;
    Ok(json!({"id": id, "label": label}))
}

/// The board objects among `ids`, each with its item (a label counts as its connector's text).
fn items(doc: &Document, ids: &[NodeId]) -> Vec<(NodeId, BoardItem)> {
    ids.iter().filter_map(|i| doc.node(*i).and_then(|n| n.board.clone()).map(|b| (*i, b))).collect()
}

pub(super) fn set_color(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "board.setColor";
    require(s, cmd)?;
    let ids = req_targets(cmd, s, p)?;
    let color = color_param(cmd, p)?.ok_or_else(|| bad(cmd, "`color` is required"))?;
    let doc = s.doc_mut()?;
    let mut changed = Vec::new();
    for (id, item) in items(doc, &ids) {
        match item {
            BoardItem::Sticky { .. } => doc.get_mut(id)?.fills = vec![Paint::solid(color)],
            BoardItem::Shape { .. } => {
                let Some(outline) = doc.get(id)?.children.first().copied() else { continue };
                let n = doc.get_mut(outline)?;
                n.fills = vec![Paint::solid(color)];
                n.strokes = vec![Paint::solid(line_shade(color))];
            }
            BoardItem::Connector(_) | BoardItem::Ink => doc.get_mut(id)?.strokes = vec![Paint::solid(line_shade(color))],
            BoardItem::Label { .. } | BoardItem::Stamp { .. } | BoardItem::Table => continue,
        }
        changed.push(id);
    }
    if changed.is_empty() {
        return Err(bad(cmd, "select stickies, shapes, connectors or ink"));
    }
    Ok(json!({"ids": changed}))
}

/// What `board.setStyle` changes (each `None` leaves it).
struct Restyle {
    shape: Option<BoardShape>,
    routing: Option<Routing>,
    size: Option<f64>,
    start: Option<StrokeCap>,
    end: Option<StrokeCap>,
}

impl Restyle {
    fn of(cmd: &str, p: &Value) -> Result<Restyle> {
        let size = num(p, "fontSize")
            .map(|v| if v.is_finite() && (1.0..=1000.0).contains(&v) { Ok(v) } else { Err(bad(cmd, "`fontSize` is from 1 to 1000")) })
            .transpose()?;
        Ok(Restyle {
            shape: string(p, "shape").map(|v| shape_of(cmd, &v)).transpose()?,
            routing: string(p, "style").map(|v| routing_of(cmd, &v)).transpose()?,
            size,
            start: cap_of(cmd, p, "startCap")?,
            end: cap_of(cmd, p, "endCap")?,
        })
    }

    /// Restyle board object `id` (`item`); whether anything about it changed.
    fn apply(&self, doc: &mut Document, id: NodeId, item: BoardItem) -> Result<bool> {
        let changed = match item {
            BoardItem::Shape { shape } => match self.shape.filter(|n| *n != shape) {
                Some(new) => {
                    reshape(doc, id, new)?;
                    true
                }
                None => false,
            },
            BoardItem::Connector(ref c) => self.restyle_connector(doc, id, c.clone())?,
            _ => false,
        };
        // Text sizes: shapes' and labels' text (stickies size theirs to fit).
        let sized = match (self.size, &item) {
            (Some(size), BoardItem::Shape { .. } | BoardItem::Label { .. }) => resize_text(doc, id, size),
            _ => false,
        };
        Ok(changed || sized)
    }

    fn restyle_connector(&self, doc: &mut Document, id: NodeId, mut c: Connector) -> Result<bool> {
        let n = doc.get_mut(id)?;
        if let Some(r) = self.routing {
            // A path shaped by hand is for the style it was shaped in.
            c.path = c.path.filter(|_| c.routing == r);
            c.routing = r;
            n.board = Some(BoardItem::Connector(c));
        }
        if let Some(cap) = self.start {
            n.stroke.cap_start = cap;
        }
        if let Some(cap) = self.end {
            n.stroke.cap_end = cap;
        }
        Ok(self.routing.is_some() || self.start.is_some() || self.end.is_some())
    }
}

/// Shape with text `id` takes outline `new` (its text room too).
fn reshape(doc: &mut Document, id: NodeId, new: BoardShape) -> Result<()> {
    let n = doc.get_mut(id)?;
    n.board = Some(BoardItem::Shape { shape: new });
    n.name = new.label().into();
    let (w, h) = (n.width, n.height);
    if let Some(l) = n.layout.as_mut() {
        l.padding = shape_padding(new, w, h);
    }
    if let Some(o) = doc.get(id)?.children.first().copied()
        && let Some(on) = doc.node_mut(o)
    {
        on.kind = NodeKind::Vector { network: VectorNetwork::from_path(&new.path(w, h)) };
        (on.width, on.height, on.transform) = (w, h, Affine::IDENTITY);
    }
    Ok(())
}

/// Board object `id`'s text at `size`; whether it has text.
fn resize_text(doc: &mut Document, id: NodeId, size: f64) -> bool {
    let Some(t) = text_of(doc, id) else { return false };
    if let Some(NodeKind::Text(td)) = doc.node_mut(t).map(|n| &mut n.kind) {
        set_font_size(td, size);
        return true;
    }
    false
}

pub(super) fn set_style(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "board.setStyle";
    require(s, cmd)?;
    let ids = req_targets(cmd, s, p)?;
    let restyle = Restyle::of(cmd, p)?;
    let doc = s.doc_mut()?;
    let mut changed = Vec::new();
    for (id, item) in items(doc, &ids) {
        if restyle.apply(doc, id, item)? {
            changed.push(id);
        }
    }
    if changed.is_empty() {
        return Err(bad(cmd, "nothing selected takes these (shapes with text, connectors, labels)"));
    }
    Ok(json!({"ids": changed}))
}

pub(super) fn set_font_size(td: &mut TextData, size: f64) {
    td.style.font_size = size;
    for r in &mut td.runs {
        r.style.font_size = size;
    }
}

/// Each layer of `sources` (copied one by one, in order, as `copies`) and its copy: the whole
/// subtrees pair up when they have the same shape (an instance made of a main component may not;
/// then only the roots do).
pub(crate) fn copy_map(doc: &Document, sources: &[NodeId], copies: &[NodeId]) -> BTreeMap<NodeId, NodeId> {
    let mut map = BTreeMap::new();
    for (s, c) in sources.iter().zip(copies) {
        let (a, b) = (doc.subtree(*s), doc.subtree(*c));
        if a.len() == b.len() {
            map.extend(a.into_iter().zip(b));
        } else {
            map.insert(*s, *c);
        }
    }
    map
}

/// The connectors and labels among copied layers, as `(copy, the original's board item)`, from the
/// originals in `doc` (`map`: original → copy).
pub(crate) fn copied_items(doc: &Document, map: &BTreeMap<NodeId, NodeId>) -> Vec<(NodeId, BoardItem)> {
    map.iter().filter_map(|(o, c)| doc.node(*o).and_then(|n| n.board.clone()).map(|b| (*c, b))).collect()
}

/// After a copy (`map`: each copied original → its copy; `originals`: the copies' items as the
/// originals had them), copied connectors join the copies of what they joined and labels follow
/// the copies of their connectors. From another file (`foreign`), what wasn't copied isn't here: a
/// connector's end there stays where it is drawn, as a point, and a label without its connector
/// goes.
pub(crate) fn rejoin(doc: &mut Document, originals: &[(NodeId, BoardItem)], map: &BTreeMap<NodeId, NodeId>, foreign: bool) {
    for (copy, item) in originals {
        let mut item = item.clone();
        let copied = |n: NodeId| map.contains_key(&n);
        match &mut item {
            BoardItem::Connector(c) if foreign => {
                let ends = ends_now(doc, *copy);
                for (end, at) in [(&mut c.from, ends.map(|e| e.0)), (&mut c.to, ends.map(|e| e.1))] {
                    if let (End::Node { node, .. }, Some(p)) = (*end, at)
                        && !copied(node)
                    {
                        *end = End::Point { x: p.x, y: p.y };
                    }
                }
            }
            BoardItem::Label { connector } if foreign && !copied(*connector) => {
                let _ = doc.remove(*copy);
                continue;
            }
            BoardItem::Connector(_) | BoardItem::Label { .. } => {}
            _ => continue,
        }
        item.remap(map);
        if let Some(n) = doc.node_mut(*copy) {
            n.board = Some(item);
        }
    }
}
