//! Creating nodes.

use figcraft_color::{ImageScaleMode, Paint};
use figcraft_doc::{Document, Node, NodeId, NodeKind, StrokeCap, TextAutoResize, TextData, VectorNetwork};
use figcraft_geom::Point;
use serde_json::{Value, json};

use super::{CommandSpec, ok};
use crate::params::{boolean, f64_of, get, id, num, num_or, opt_id, req_targets, string};
use crate::{Result, Session, bad, spec};

pub const PLACED_IMAGE_MAX_SIDE: f64 = 4096.0;

/// Frame presets shown by the Frame tool (category, name, width, height).
pub const FRAME_PRESETS: &[(&str, &str, f64, f64)] = &[
    ("Phone", "iPhone 16", 393.0, 852.0),
    ("Phone", "iPhone 16 Pro", 402.0, 874.0),
    ("Phone", "iPhone 16 Pro Max", 440.0, 956.0),
    ("Phone", "iPhone 16 Plus", 430.0, 932.0),
    ("Phone", "iPhone SE", 320.0, 568.0),
    ("Phone", "Android Compact", 412.0, 917.0),
    ("Phone", "Android Medium", 700.0, 840.0),
    ("Tablet", "iPad mini 8.3", 744.0, 1133.0),
    ("Tablet", "iPad Pro 11\"", 834.0, 1194.0),
    ("Tablet", "iPad Pro 12.9\"", 1024.0, 1366.0),
    ("Tablet", "Surface Pro 8", 1440.0, 960.0),
    ("Tablet", "Android Expanded", 1280.0, 800.0),
    ("Desktop", "MacBook Air", 1280.0, 832.0),
    ("Desktop", "MacBook Pro 14\"", 1512.0, 982.0),
    ("Desktop", "MacBook Pro 16\"", 1728.0, 1117.0),
    ("Desktop", "Desktop", 1440.0, 1024.0),
    ("Desktop", "Wireframe", 1440.0, 1024.0),
    ("Desktop", "TV", 1280.0, 720.0),
    ("Presentation", "Slide 16:9", 1920.0, 1080.0),
    ("Presentation", "Slide 4:3", 1024.0, 768.0),
    ("Watch", "Apple Watch Series 10 42mm", 187.0, 223.0),
    ("Watch", "Apple Watch 41mm", 176.0, 215.0),
    ("Watch", "Apple Watch 45mm", 198.0, 242.0),
    ("Paper", "A4", 595.0, 842.0),
    ("Paper", "A5", 420.0, 595.0),
    ("Paper", "A6", 297.0, 420.0),
    ("Paper", "Letter", 612.0, 792.0),
    ("Paper", "Tabloid", 792.0, 1224.0),
    ("Social media", "Instagram post", 1080.0, 1350.0),
    ("Social media", "Instagram story", 1080.0, 1920.0),
    ("Social media", "Facebook post", 1200.0, 630.0),
    ("Social media", "X post", 1200.0, 675.0),
    ("Social media", "LinkedIn post", 1200.0, 1200.0),
    ("Social media", "YouTube thumbnail", 1280.0, 720.0),
    ("Social media", "Dribbble shot", 1600.0, 1200.0),
];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "node.create",
            "Create layer",
            "{type: rectangle|ellipse|polygon|star|line|arrow|vector|text|frame|section|component|slice, x?, y? (relative to the parent frame; groups don't count), width?, height?, parent? (id; default: the page), index?, name?, text? | characters? (for text), path? | points? (vector: SVG path data | [[x,y],..]), closed? (vector), props? {node.set keys}, select? (default true)}",
            true,
            create
        ),
        spec!(
            "frame.create",
            "Create frame",
            "{preset? (name from FRAME_PRESETS), x?, y?, width?, height?, parent?, index?, name?, props?, select? (default true)} (a preset without x/y lands to the right of the page's frames)",
            true,
            frame_create
        ),
        spec!("shape.rectangle", "Rectangle", "{x, y, width, height, parent?, index?, name?, props?, select? (default true)}", true, |s, p| {
            create_kind(s, p, "rectangle")
        }),
        spec!("shape.ellipse", "Ellipse", "{x, y, width, height, parent?, index?, name?, props?, select? (default true)}", true, |s, p| create_kind(
            s, p, "ellipse"
        )),
        spec!(
            "shape.polygon",
            "Polygon",
            "{x, y, width, height, parent?, index?, name?, props? {pointCount}, select? (default true)}",
            true,
            |s, p| create_kind(s, p, "polygon")
        ),
        spec!(
            "shape.star",
            "Star",
            "{x, y, width, height, parent?, index?, name?, props? {pointCount, innerRadius}, select? (default true)}",
            true,
            |s, p| create_kind(s, p, "star")
        ),
        spec!("shape.line", "Line", "{x, y, width (length), parent?, index?, name?, props? {rotation}, select? (default true)}", true, |s, p| {
            create_kind(s, p, "line")
        }),
        spec!("shape.arrow", "Arrow", "{x, y, width (length), parent?, index?, name?, props?, select? (default true)}", true, |s, p| create_kind(
            s, p, "arrow"
        )),
        spec!(
            "text.create",
            "Text",
            "{text | characters, x, y, parent?, index?, name? (default: the first line), width? (fixed width wraps), props? (fontSize, fontFamily, …), select? (default true)}",
            true,
            |s, p| { create_kind(s, p, "text") }
        ),
        spec!(
            "vector.create",
            "Vector",
            "{path (SVG path data) | points [[x,y],..], closed?, x?, y?, width?, height? (scale the path; one keeps its proportion), parent?, index?, name?, props?, select? (default true)}",
            true,
            |s, p| create_kind(s, p, "vector")
        ),
        spec!(
            "image.place",
            "Place image",
            "{data (base64) | path, x?, y?, width?, height?, parent?, name? (default Image), target? (an existing layer: the image is added on top of its fills instead of making a layer), mode? (fill|fit|crop|tile)} → {id, image, width, height, imageProfile (its embedded colour profile; null: taken as sRGB)}",
            true,
            place_image
        ),
        spec!("frame.swapOrientation", "Swap orientation", "{ids?} (portrait ⇄ landscape: width and height trade places)", true, |s, p| {
            let ids = req_targets("frame.swapOrientation", s, p)?;
            let doc = s.doc()?;
            let sizes: Vec<(NodeId, f64, f64)> = ids.iter().filter_map(|i| doc.node(*i).map(|n| (*i, n.width, n.height))).collect();
            for (i, w, h) in sizes {
                super::arrange::resize_cmd(s, &json!({"ids": [i], "width": h, "height": w}))?;
            }
            ok()
        }),
        spec!("frame.presets", "List frame presets", "{}", false, |_, _| {
            Ok(Value::Array(FRAME_PRESETS.iter().map(|(c, n, w, h)| json!({"category": c, "name": n, "width": w, "height": h})).collect()))
        }),
    ]
}

/// The top-most frame-like (or section) node on the page containing page point `pt`,
/// excluding `exclude` and their descendants.
pub fn frame_at(doc: &Document, page: NodeId, pt: Point, exclude: &[NodeId]) -> Option<NodeId> {
    /// `locked`: inside an instance, where only slots (and what's in them) take layers.
    fn rec(doc: &Document, id: NodeId, pt: Point, exclude: &[NodeId], depth: usize, locked: bool) -> Option<NodeId> {
        if depth > 128 || exclude.contains(&id) {
            return None;
        }
        let n = doc.node(id)?;
        // Board objects (stickies, shapes with text) never take layers in.
        if !n.visible || n.locked || n.is_board_atom() {
            return None;
        }
        let is_container = n.kind.is_frame_like() || matches!(n.kind, NodeKind::Section { .. });
        let inside = doc.world_bounds(id).is_some_and(|b| b.contains(pt)) && {
            let inv = doc.world_transform(id).inverse();
            n.local_rect().contains(inv * pt)
        };
        if is_container && !inside {
            return None;
        }
        let locked = crate::components::locks_children(doc, id, locked);
        for c in n.children.iter().rev() {
            if let Some(f) = rec(doc, *c, pt, exclude, depth + 1, locked) {
                return Some(f);
            }
        }
        (is_container && inside && !locked).then_some(id)
    }
    let p = doc.node(page)?;
    p.children.iter().rev().find_map(|c| rec(doc, *c, pt, exclude, 0, false))
}

pub fn kind_for(t: &str) -> Option<(NodeKind, &'static str)> {
    Some(match t.to_ascii_lowercase().as_str() {
        "rectangle" | "rect" => (NodeKind::Rectangle, "Rectangle"),
        "ellipse" | "circle" | "oval" => (NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }, "Ellipse"),
        "polygon" | "triangle" => (NodeKind::Polygon { count: 3 }, "Polygon"),
        "star" => (NodeKind::Star { count: 5, inner_ratio: 0.38 }, "Star"),
        "line" => (NodeKind::Line, "Line"),
        "arrow" => (NodeKind::Line, "Arrow"),
        "vector" | "path" => (NodeKind::Vector { network: VectorNetwork::default() }, "Vector"),
        "text" => (NodeKind::Text(TextData::default()), "Text"),
        "frame" => (NodeKind::Frame, "Frame"),
        "section" => (NodeKind::Section { contents_hidden: false }, "Section"),
        "component" => (NodeKind::Component { description: String::new(), props: Default::default(), prop_order: Vec::new() }, "Component"),
        "group" => (NodeKind::Group, "Group"),
        "slice" => (NodeKind::Slice, "Slice"),
        _ => return None,
    })
}

fn create_kind(s: &mut Session, p: &Value, t: &str) -> Result<Value> {
    let mut p = p.clone();
    if let Some(o) = p.as_object_mut() {
        o.insert("type".into(), json!(t));
    }
    create(s, &p)
}

/// Parse SVG path data (`M L H V C S Q T A Z`, absolute and relative) into a bezier path.
pub fn parse_path_data(d: &str) -> Option<figcraft_geom::BezPath> {
    figcraft_geom::BezPath::from_svg(d).ok()
}

/// A vector's geometry made from SVG path data (see [`path_network`]).
pub(crate) struct PathShape {
    /// The network, with the path's own box moved to the origin and scaled to the size asked for.
    pub network: VectorNetwork,
    pub width: f64,
    pub height: f64,
    /// Where the path's own box starts in its coordinates (unscaled).
    pub offset: (f64, f64),
}

/// The network SVG path data `d` draws, for `node.create` and `node.set`: the path's own box moves
/// to the origin, and `width`/`height` scale it as [`vector_scale`] says. Data that doesn't parse
/// or draws nothing is refused.
pub(crate) fn path_network(cmd: &str, d: &str, width: Option<f64>, height: Option<f64>) -> Result<PathShape> {
    use figcraft_geom::Shape;
    let empty = || bad(cmd, "`path` draws nothing: a vector needs a visible line or area");
    let mut path = parse_path_data(d).ok_or_else(|| bad(cmd, "`path` is not valid SVG path data"))?;
    let bb = path.bounding_box();
    if !(bb.x0.is_finite() && bb.y0.is_finite() && bb.width().is_finite() && bb.height().is_finite()) {
        return Err(bad(cmd, "`path` has coordinates too large to draw"));
    }
    if bb.width() <= 1e-9 && bb.height() <= 1e-9 {
        return Err(empty());
    }
    path.apply_affine(figcraft_geom::Affine::translate((-bb.x0, -bb.y0)));
    let (sx, sy) = vector_scale(width, height, bb.width(), bb.height(), "a path").map_err(|e| bad(cmd, e))?;
    if sx != 1.0 || sy != 1.0 {
        path.apply_affine(figcraft_geom::Affine::scale_non_uniform(sx, sy));
    }
    let network = VectorNetwork::from_path(&path);
    if network.segments.is_empty() {
        return Err(empty());
    }
    Ok(PathShape { network, width: bb.width() * sx, height: bb.height() * sy, offset: (bb.x0, bb.y0) })
}

/// How much a vector's path is scaled to take the size asked for: with both sides, each scales
/// independently (as resizing a vector does); with one, the other keeps its proportion. A size on
/// an axis the geometry has none of is an error unless it is 0 (a line's height), and a size of 0
/// or less on an axis it has is refused rather than flattening it.
fn vector_scale(width: Option<f64>, height: Option<f64>, bw: f64, bh: f64, what: &str) -> std::result::Result<(f64, f64), String> {
    let fit = |asked: Option<f64>, have: f64, axis: &str| match asked {
        None => Ok(1.0),
        Some(a) if have <= 1e-9 => {
            if a == 0.0 {
                Ok(1.0)
            } else {
                Err(format!("`{axis}` can't size {what} with no {} extent", if axis == "width" { "horizontal" } else { "vertical" }))
            }
        }
        Some(a) if a > 0.0 && (a / have).is_finite() => Ok(a / have),
        Some(_) => Err(format!("`{axis}` must be a positive, finite size: 0 or less would flatten {what}")),
    };
    Ok(match (width, height) {
        (Some(_), Some(_)) => (fit(width, bw, "width")?, fit(height, bh, "height")?),
        (Some(_), None) => {
            let s = fit(width, bw, "width")?;
            (s, s)
        }
        (None, Some(_)) => {
            let s = fit(height, bh, "height")?;
            (s, s)
        }
        (None, None) => (1.0, 1.0),
    })
}

pub fn create(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "node.create";
    let t = string(p, "type").ok_or_else(|| bad(cmd, "`type` is required"))?;
    let (mut kind, base) = kind_for(&t).ok_or_else(|| bad(cmd, format!("unknown type `{t}`")))?;
    let page = s.page()?;
    let parent = crate::params::opt_id(cmd, p, "parent")?.unwrap_or(page);
    let doc = s.doc()?;
    let pn = doc.get(parent)?;
    if !pn.kind.is_container() {
        return Err(bad(cmd, format!("{parent} can't contain layers")));
    }
    if base == "Section" && !doc.takes_sections(parent) {
        return Err(bad(cmd, "sections go on the page or in other sections"));
    }
    // X and Y are in the frame's space: a group parent has none of its own.
    let space = if pn.kind.bounds_follow_children() { doc.frame_relative(parent) } else { figcraft_geom::Affine::IDENTITY };
    let x = num_or(p, "x", 0.0);
    let y = num_or(p, "y", 0.0);
    let (mut w, mut h) = match base {
        "Line" | "Arrow" => (num_or(p, "width", 100.0), 0.0),
        "Text" => (num_or(p, "width", 0.0), num_or(p, "height", 0.0)),
        "Frame" | "Component" => (num_or(p, "width", 100.0), num_or(p, "height", 100.0)),
        "Section" => (num_or(p, "width", 400.0), num_or(p, "height", 300.0)),
        _ => (num_or(p, "width", 100.0), num_or(p, "height", 100.0)),
    };
    // Vectors: path data or points define the shape and its box. `width`/`height`, when asked
    // for, scale the geometry: both sides independently, one side proportionally.
    let mut offset = (0.0, 0.0);
    let asked = (num(p, "width"), num(p, "height"));
    if let NodeKind::Vector { network } = &mut kind {
        if let Some(d) = string(p, "path") {
            let shape = path_network(cmd, &d, asked.0, asked.1)?;
            *network = shape.network;
            if boolean(p, "closed") == Some(false) {
                network.regions.clear();
            }
            (w, h, offset) = (shape.width, shape.height, shape.offset);
        } else if let Some(Value::Array(pts)) = get(p, "points") {
            let pts: Vec<Point> = pts.iter().filter_map(|v| Some(Point::new(f64_of(v.get(0)?)?, f64_of(v.get(1)?)?))).collect();
            if pts.len() < 2 {
                return Err(bad(cmd, "`points` needs at least two [x, y] pairs"));
            }
            let bb = figcraft_geom::points_bounds(pts.iter().copied()).unwrap_or_default();
            if bb.width() <= 1e-9 && bb.height() <= 1e-9 {
                return Err(bad(cmd, "`points` draws nothing: a vector needs a visible line or area"));
            }
            let (sx, sy) = vector_scale(asked.0, asked.1, bb.width(), bb.height(), "a point set").map_err(|e| bad(cmd, e))?;
            let local: Vec<Point> = pts.iter().map(|q| Point::new((q.x - bb.x0) * sx, (q.y - bb.y0) * sy)).collect();
            *network = VectorNetwork::from_points(&local, boolean(p, "closed").unwrap_or(false));
            w = bb.width() * sx;
            h = bb.height() * sy;
            offset = (bb.x0, bb.y0);
        }
    }
    if let NodeKind::Text(td) = &mut kind {
        td.characters = super::props::normalize_line_endings(&string(p, "text").or_else(|| string(p, "characters")).unwrap_or_default());
        if w > 0.0 {
            td.auto_resize = TextAutoResize::Height;
        }
    }
    let name = string(p, "name").unwrap_or_else(|| {
        if let NodeKind::Text(td) = &kind {
            let first: String = td.characters.lines().next().unwrap_or("").chars().take(64).collect();
            if !first.trim().is_empty() {
                return first;
            }
        }
        if matches!(base, "Frame" | "Section" | "Component" | "Group") {
            s.doc().map(|d| d.next_name(base)).unwrap_or_else(|_| base.to_string())
        } else {
            base.to_string()
        }
    });
    // In the app (it has a theme), text that names no fill is readable on its page: white on a
    // dark one. Headless sessions make the same thing everywhere.
    let asks_fill = get(p, "props").is_some_and(|o| ["fill", "fills", "fillStyle"].iter().any(|k| o.get(k).is_some()));
    let app_text = s.ui_dark.is_some() && base == "Text" && !asks_fill;
    let ds = s.ds_mut()?;
    let doc = ds.doc_mut();
    let nid = doc.alloc_id();
    let mut node = if base == "Section" { Node::new_section(nid, name, w, h) } else { Node::new(nid, kind, name, w, h) };
    if app_text && let Some(fills) = crate::tools::readable_text_fills(doc, parent) {
        node.fills = fills;
    }
    node.transform = space.inverse() * figcraft_geom::Affine::translate((x + offset.0, y + offset.1));
    if base == "Arrow" {
        node.stroke.cap_end = StrokeCap::LineArrow;
    }
    if let NodeKind::Text(_) = node.kind
        && let Some((tw, th)) = figcraft_text::auto_size(&node)
    {
        node.width = tw;
        node.height = th;
    }
    let index = crate::params::opt_index(cmd, p, "index")?;
    doc.insert(node, parent, index)?;
    if let Some(Value::Object(props)) = get(p, "props") {
        super::props::apply(cmd, doc, nid, props)?;
    }
    if boolean(p, "select").unwrap_or(true) {
        ds.selection = vec![nid];
    }
    Ok(json!({"id": nid}))
}

fn frame_create(s: &mut Session, p: &Value) -> Result<Value> {
    let mut q = p.clone();
    if let Some(name) = string(p, "preset") {
        let (_, pname, w, h) = FRAME_PRESETS
            .iter()
            .find(|(_, n, _, _)| n.eq_ignore_ascii_case(&name))
            .ok_or_else(|| bad("frame.create", format!("unknown preset `{name}`")))?;
        if let Some(o) = q.as_object_mut() {
            o.entry("width").or_insert(json!(w));
            o.entry("height").or_insert(json!(h));
            o.entry("name").or_insert(json!(pname));
        }
        // No position on the page: beside the right-most top-level layer, else in the view.
        if num(p, "x").is_none() && num(p, "y").is_none() && id(p, "parent").is_none() {
            let (x, y) = free_spot(s, *w, *h)?;
            if let Some(o) = q.as_object_mut() {
                o.insert("x".into(), json!(x));
                o.insert("y".into(), json!(y));
            }
        }
    }
    if let Some(o) = q.as_object_mut() {
        o.insert("type".into(), json!("frame"));
    }
    create(s, &q)
}

/// Where a new top-level frame of `w`×`h` goes: 100 to the right of the right-most top-level
/// layer (top-aligned with it), or centred in the view on an empty page.
fn free_spot(s: &Session, w: f64, h: f64) -> Result<(f64, f64)> {
    let ds = s.ds()?;
    let doc = &ds.doc;
    let right_most = doc
        .node(ds.page)
        .map(|pg| pg.children.iter().filter_map(|c| doc.world_bounds(*c)).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter(|b| b.x1.is_finite() && b.y0.is_finite())
        .max_by(|a, b| a.x1.total_cmp(&b.x1));
    Ok(match right_most {
        Some(b) => ((b.x1 + 100.0).round(), b.y0.round()),
        None => ((ds.view.cx - w / 2.0).round(), (ds.view.cy - h / 2.0).round()),
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn read_path(path: &str) -> Result<Vec<u8>> {
    crate::file_access::read(path)
}

#[cfg(target_arch = "wasm32")]
fn read_path(path: &str) -> Result<Vec<u8>> {
    Err(crate::EngineError::Other(format!("{path}: files can't be read in the browser")))
}

/// Layers that can show an image inside their own outline.
pub fn accepts_image_fill(kind: &NodeKind) -> bool {
    kind.is_shape() || kind.is_frame_like() || matches!(kind, NodeKind::Text(_) | NodeKind::BooleanOp { .. })
}

/// Add an image fill on top of the layer's fills: the others keep their place and variable
/// bindings; a linked fill style no longer describes the list, so the link goes.
fn add_image_fill(node: &mut Node, image: String, mode: ImageScaleMode) {
    node.fills.push(Paint::image(image, mode));
    node.styles.fill = None;
}

/// Place an image as a rectangle with an image fill (Figma's image layer).
pub fn place_image(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "image.place";
    let target = opt_id(cmd, p, "target")?;
    if let Some(id) = target {
        let node = s.doc()?.node(id).ok_or(crate::EngineError::NoNode(id))?;
        if !accepts_image_fill(&node.kind) {
            return Err(bad(cmd, "`target` must be a layer that can have a fill"));
        }
    }
    let bytes = if let Some(b64) = string(p, "data") {
        use base64::Engine as _;
        let b64 = b64.split_once(',').map(|(_, d)| d.to_string()).unwrap_or(b64);
        base64::engine::general_purpose::STANDARD.decode(b64.trim()).map_err(|e| bad(cmd, format!("`data`: {e}")))?
    } else if let Some(path) = string(p, "path") {
        read_path(&path)?
    } else {
        return Err(bad(cmd, "pass `data` (base64) or `path`"));
    };
    let mode = match string(p, "mode").unwrap_or_default().to_ascii_lowercase().as_str() {
        "fit" => ImageScaleMode::Fit,
        "crop" => ImageScaleMode::Crop,
        "tile" => ImageScaleMode::Tile,
        _ => ImageScaleMode::Fill,
    };
    if let Some(id) = target {
        let doc = s.doc_mut()?;
        let (hash, _, _) = figcraft_format::add_image(doc, bytes).map_err(|e| bad(cmd, e.to_string()))?;
        let profile = figcraft_render::images::profile_name(&hash, doc.images.get(&hash));
        let node = doc.get_mut(id)?;
        add_image_fill(node, hash.clone(), mode);
        return Ok(json!({"id": id, "image": hash, "width": node.width, "height": node.height, "imageProfile": profile}));
    }
    let page = s.page()?;
    let parent = crate::params::opt_id(cmd, p, "parent")?.unwrap_or(page);
    let ds = s.ds_mut()?;
    let doc = ds.doc_mut();
    let density = figcraft_format::pixel_density(&bytes);
    let (hash, iw, ih) = figcraft_format::add_image(doc, bytes).map_err(|e| bad(cmd, e.to_string()))?;
    let profile = figcraft_render::images::profile_name(&hash, doc.images.get(&hash));
    let (placed_w, placed_h) = figcraft_format::placed_size(iw, ih, density, PLACED_IMAGE_MAX_SIDE);
    let w = num(p, "width").unwrap_or(placed_w);
    let h = num(p, "height").unwrap_or(if num(p, "width").is_some() { w * f64::from(ih) / f64::from(iw).max(1.0) } else { placed_h });
    let nid = doc.alloc_id();
    let name = string(p, "name").unwrap_or_else(|| "Image".into());
    let mut node = Node::new(nid, NodeKind::Rectangle, name, w, h);
    node.transform = figcraft_geom::Affine::translate((num_or(p, "x", 0.0), num_or(p, "y", 0.0)));
    node.fills = vec![Paint::image(hash.clone(), mode)];
    doc.insert(node, parent, None)?;
    ds.selection = vec![nid];
    let _ = ok;
    Ok(json!({"id": nid, "image": hash, "width": w, "height": h, "imageProfile": profile}))
}
