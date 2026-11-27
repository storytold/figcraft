//! Compare a capture of a built page with the design (`inspect.compare`, the MCP tool
//! `compare_design`). A browser's text and antialiasing never match pixel for pixel, so the answer
//! isn't one score: it is the layers that differ, each with what kind of difference it looks like,
//! deepest first so a difference is reported once. It runs here, on the computer, on a capture the
//! developer's own tools made.

use figcraft_doc::{Document, NodeId, NodeKind};
use figcraft_geom::Rect;
use figcraft_render::Pixmap;
use serde_json::{Value, json};

use crate::cmd::CommandSpec;
use crate::cmd::file::read_capped;
use crate::params::{get, req_targets, string};
use crate::{Result, Session, bad, spec};

/// Largest capture read (bytes).
const MAX_CAPTURE: u64 = 256 << 20;
/// The largest side compared: bigger images are compared at this size.
const MAX_SIDE: u32 = 2_000;
/// How far (in compared pixels) a layer may have moved to count as moved.
const MAX_SHIFT: i32 = 16;
/// How different a pixel's channels must be to count, out of 255.
const DEFAULT_TOLERANCE: f64 = 24.0;
/// Text differs between renderers even when it's right: it needs this many times the tolerance.
const TEXT_FACTOR: f64 = 2.0;
/// The share of a layer's own pixels that must differ for it to be reported.
const MIN_SHARE: f64 = 0.02;
/// Most differences listed.
const MAX_LISTED: usize = 100;
/// Most layers looked at.
const MAX_LAYERS: usize = 5_000;
/// Most layers whose drawn shape is worked out; past it, a layer is matched by its box.
const MAX_DRAWN: usize = 300;
/// Longest `data` read (base64 characters): [`MAX_CAPTURE`] encoded, checked before decoding.
const MAX_DATA: usize = (MAX_CAPTURE as usize / 3 + 1) * 4;

/// Straight RGB over white, in a grid of `w` × `h`.
struct Image {
    w: usize,
    h: usize,
    rgb: Vec<[f32; 3]>,
}

impl Image {
    /// `p` box-filtered to `w` × `h`, which also smooths away most antialiasing.
    fn from(p: &Pixmap, w: usize, h: usize) -> Self {
        let (pw, ph) = (p.width() as usize, p.height() as usize);
        let data = p.data();
        let mut rgb = Vec::with_capacity(w * h);
        for y in 0..h {
            let (y0, y1) = (y * ph / h, ((y + 1) * ph / h).max(y * ph / h + 1).min(ph));
            for x in 0..w {
                let (x0, x1) = (x * pw / w, ((x + 1) * pw / w).max(x * pw / w + 1).min(pw));
                let mut sum = [0f32; 3];
                let mut count = 0f32;
                for sy in y0..y1 {
                    for sx in x0..x1 {
                        let Some(px) = data.get((sy * pw + sx) * 4..(sy * pw + sx) * 4 + 4) else { continue };
                        // Premultiplied: over white adds what the alpha lets through.
                        let rest = 255.0 - f32::from(px[3]);
                        for (s, c) in sum.iter_mut().zip(px) {
                            *s += f32::from(*c) + rest;
                        }
                        count += 1.0;
                    }
                }
                rgb.push(sum.map(|v| v / count.max(1.0)));
            }
        }
        Self { w, h, rgb }
    }

    fn at(&self, x: i32, y: i32) -> Option<&[f32; 3]> {
        (x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h).then(|| self.rgb.get(y as usize * self.w + x as usize)).flatten()
    }
}

/// How far apart two colours are: their most different channel.
fn delta(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max)
}

/// A rectangle of the grid: x0, y0, x1, y1.
type Area = (i32, i32, i32, i32);

/// The mean colour of `area` in `img` as hex.
fn mean(img: &Image, (x0, y0, x1, y1): Area) -> Option<String> {
    let (mut sum, mut n) = ([0f32; 3], 0f32);
    for y in y0..y1 {
        for x in x0..x1 {
            if let Some(c) = img.at(x, y) {
                for (s, v) in sum.iter_mut().zip(c) {
                    *s += v;
                }
                n += 1.0;
            }
        }
    }
    (n > 0.0).then(|| format!("#{:02X}{:02X}{:02X}", (sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8))
}

/// How much `area` of the design differs from the capture moved by `dx`, `dy`, sampled.
fn cost(design: &Image, capture: &Image, (x0, y0, x1, y1): Area, dx: i32, dy: i32) -> f32 {
    let step = (((x1 - x0) * (y1 - y0)) / 4_000).clamp(1, 16);
    let (mut sum, mut n) = (0f32, 0f32);
    for y in (y0..y1).step_by(step as usize) {
        for x in (x0..x1).step_by(step as usize) {
            if let (Some(a), Some(b)) = (design.at(x, y), capture.at(x + dx, y + dy)) {
                sum += delta(a, b);
                n += 1.0;
            }
        }
    }
    if n > 0.0 { sum / n } else { f32::MAX }
}

/// Where `area` of the design best matches the capture, if a move explains it.
fn moved(design: &Image, capture: &Image, area: Area, tolerance: f32) -> Option<(i32, i32)> {
    let here = cost(design, capture, area, 0, 0);
    let mut best = (here, 0, 0);
    for dy in -MAX_SHIFT..=MAX_SHIFT {
        for dx in -MAX_SHIFT..=MAX_SHIFT {
            let c = cost(design, capture, area, dx, dy);
            if c < best.0 {
                best = (c, dx, dy);
            }
        }
    }
    (best.0 < here / 2.0 && best.0 < tolerance).then_some((best.1, best.2))
}

/// The visible layers under `id`, parents first, at most [`MAX_LAYERS`] (without recursion, so a
/// deep tree can't overflow the stack); and whether some were left out.
fn visible(doc: &Document, id: NodeId) -> (Vec<NodeId>, bool) {
    let (mut out, mut stack) = (Vec::new(), vec![id]);
    while let Some(c) = stack.pop() {
        let Some(n) = doc.node(c) else { continue };
        if !n.visible {
            continue;
        }
        if out.len() >= MAX_LAYERS {
            return (out, true);
        }
        out.push(c);
        stack.extend(n.children.iter().rev());
    }
    (out, false)
}

/// The cells of `area` (a grid of `per` cells to a CSS pixel from `origin`) that layer `id` draws
/// on, from its own drawing: what it leaves see-through (an ellipse's corners, the space around
/// glyphs) stays with the layers under it. `None` when it can't be drawn.
fn coverage(doc: &Document, id: NodeId, origin: (f64, f64), per: f64, (x0, y0, x1, y1): Area) -> Option<Vec<bool>> {
    let (w, h) = (usize::try_from(x1 - x0).ok()?, usize::try_from(y1 - y0).ok()?);
    if w == 0 || h == 0 {
        return Some(Vec::new());
    }
    let at = |cell: i32, o: f64| o + f64::from(cell) / per;
    let page = Rect::new(at(x0, origin.0), at(y0, origin.1), at(x1, origin.0), at(y1, origin.1));
    let drawn = figcraft_render::render_area(doc, &[id], page, per)?;
    let (pw, ph) = (drawn.width() as usize, drawn.height() as usize);
    let data = drawn.data();
    let span = |i: usize, n: usize, of: usize| (i * of / n, ((i + 1) * of / n).max(i * of / n + 1).min(of));
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        let (sy0, sy1) = span(y, h, ph);
        for x in 0..w {
            let (sx0, sx1) = span(x, w, pw);
            out.push((sy0..sy1).any(|sy| (sx0..sx1).any(|sx| data.get((sy * pw + sx) * 4 + 3).is_some_and(|a| *a > 0))));
        }
    }
    Some(out)
}

/// Number parameter `key`: `None` when left out; anything but a number is refused.
fn number(cmd: &str, p: &Value, key: &str) -> Result<Option<f64>> {
    match get(p, key) {
        None => Ok(None),
        Some(Value::Number(n)) => Ok(n.as_f64()),
        Some(_) => Err(bad(cmd, format!("`{key}` must be a number"))),
    }
}

fn compare(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "inspect.compare";
    // Everything but the capture first: it's the costly part.
    let id = match req_targets(cmd, s, p)?.as_slice() {
        [id] => *id,
        [] => return Err(bad(cmd, "select the frame to compare with")),
        _ => return Err(bad(cmd, "compare one frame at a time")),
    };
    // A page is built from a frame, a component or an instance of one. Any other layer would still
    // give a result, and it would look like a real answer: refuse it instead.
    let target = s.doc()?.get(id)?;
    if !target.kind.is_frame_like() || matches!(target.kind, NodeKind::ComponentSet { .. }) {
        let kind = target.kind.type_name().to_lowercase();
        return Err(bad(cmd, format!("{:?} is a {kind}: compare a frame, component or instance", target.name)));
    }
    let (density, tolerance) = (number(cmd, p, "density")?, number(cmd, p, "tolerance")?);
    let bytes = match (get(p, "data"), string(p, "path")) {
        (Some(_), Some(_)) => return Err(bad(cmd, "pass `path` or `data`, not both")),
        (Some(Value::String(d)), None) => {
            use base64::Engine as _;
            let d = d.trim();
            if d.len() > MAX_DATA {
                return Err(bad(cmd, format!("data: a capture up to {} MB", MAX_CAPTURE >> 20)));
            }
            base64::engine::general_purpose::STANDARD.decode(d).map_err(|_| bad(cmd, "data: base64"))?
        }
        (Some(_), None) => return Err(bad(cmd, "data: a base64 string")),
        (None, Some(path)) => read_capped(&path, MAX_CAPTURE)?,
        (None, None) => return Err(bad(cmd, "pass `path` or `data`: a capture of the page")),
    };
    let capture = figcraft_render::images::decode(&bytes).ok_or_else(|| bad(cmd, "the capture isn't an image FigCraft can read"))?;
    let doc = s.doc()?;
    let root = doc.get(id)?;
    let bounds = doc.world_bounds(id).ok_or_else(|| bad(cmd, "the layer has no size"))?;
    let (fw, fh) = (bounds.width(), bounds.height());
    if !(fw > 0.0 && fh > 0.0) {
        return Err(bad(cmd, "the layer has no size"));
    }
    // A capture at 2x device pixels is the same design: the frame sets the scale.
    let density = density.filter(|d| d.is_finite() && *d > 0.0).unwrap_or_else(|| f64::from(capture.width()) / fw).clamp(0.1, 8.0);
    let tolerance = tolerance.filter(|t| t.is_finite()).unwrap_or(DEFAULT_TOLERANCE).clamp(1.0, 255.0) as f32;
    let (cw, ch) = (f64::from(capture.width()) / density, f64::from(capture.height()) / density);
    let same_size = (cw - fw).abs() < 1.0 && (ch - fh).abs() < 1.0;
    // Compared on the area both have, at one size.
    let (w, h) = (fw.min(cw), fh.min(ch));
    // Bounded by the larger of the two as well: both are gridded whole, so a capture far smaller
    // than the frame must not make the frame's grid (and its drawing) huge. No floor: a gigantic
    // frame just gets a coarser grid.
    let per = (f64::from(MAX_SIDE) / fw.max(fh).max(cw).max(ch)).min(density);
    // Every grid comes from `per` itself, never from a rounded overlap: one cell of a tiny capture
    // must not stand for thousands of the frame's.
    let cells = |css: f64| ((css * per).ceil() as usize).max(1);
    // Only the frame's own rectangle, as the capture holds it: not its export area, which takes in
    // strokes, effects and layers past an edge that doesn't clip. Drawn at twice the compared size
    // at most (enough to smooth its edges as the capture's are), not at the capture's density: a
    // tall page at 2x would be hundreds of megabytes.
    let drawn = figcraft_render::render_area(doc, &[id], bounds, density.min(per * 2.0)).ok_or_else(|| bad(cmd, "the layer can't be rendered"))?;
    let (gw, gh) = (cells(w), cells(h));
    let design = Image::from(&drawn, cells(fw), cells(fh));
    let shot = Image::from(&capture, cells(cw), cells(ch));
    let mut differing = vec![false; gw * gh];
    let mut changed = 0usize;
    for y in 0..gh {
        for x in 0..gw {
            if let (Some(a), Some(b)) = (design.at(x as i32, y as i32), shot.at(x as i32, y as i32))
                && delta(a, b) > tolerance
            {
                changed += 1;
                if let Some(d) = differing.get_mut(y * gw + x) {
                    *d = true;
                }
            }
        }
    }
    let (layers, layers_cut) = visible(doc, id);
    let mut claimed = vec![false; gw * gh];
    let (mut drawn_shapes, mut by_box) = (0usize, 0usize);
    // (weight, layer, its area, share differing, is text)
    let mut found: Vec<(f64, NodeId, Area, f64, bool)> = Vec::new();
    // Deepest first: a child's difference isn't its parent's too.
    for layer in layers.iter().rev() {
        let (Some(n), Some(b)) = (doc.node(*layer), doc.world_bounds(*layer)) else { continue };
        let corner = |v: f64, origin: f64, limit: usize| (((v - origin) * per).round().max(0.0) as usize).min(limit) as i32;
        let area = (corner(b.x0, bounds.x0, gw), corner(b.y0, bounds.y0, gh), corner(b.x1, bounds.x0, gw), corner(b.y1, bounds.y0, gh));
        let (x0, y0, x1, y1) = area;
        // A layer with an unclaimed difference in its box claims only the cells it draws on, so a
        // see-through part leaves them to the layers under it. Drawing each one costs, so it is
        // done only for those, and only up to a limit (said in the result).
        let open = (y0..y1).any(|y| {
            (x0..x1).any(|x| {
                let at = y as usize * gw + x as usize;
                claimed.get(at).is_some_and(|c| !c) && differing.get(at).is_some_and(|d| *d)
            })
        });
        let shape = if !open {
            None
        } else if drawn_shapes < MAX_DRAWN {
            drawn_shapes += 1;
            coverage(doc, *layer, (bounds.x0, bounds.y0), per, area)
        } else {
            by_box += 1;
            None
        };
        let (mut own, mut bad_pixels) = (0usize, 0usize);
        let text = matches!(n.kind, NodeKind::Text(_));
        let width = (x1 - x0).max(0) as usize;
        for y in y0..y1 {
            for x in x0..x1 {
                if shape.as_ref().is_some_and(|c| !c.get((y - y0) as usize * width + (x - x0) as usize).copied().unwrap_or(false)) {
                    continue;
                }
                let at = y as usize * gw + x as usize;
                let (Some(free), Some(differs)) = (claimed.get_mut(at), differing.get(at)) else { continue };
                if *free {
                    continue;
                }
                own += 1;
                let far =
                    !text || design.at(x, y).zip(shot.at(x, y)).is_some_and(|(a, b)| f64::from(delta(a, b)) > f64::from(tolerance) * TEXT_FACTOR);
                bad_pixels += usize::from(*differs && far);
                *free = true;
            }
        }
        let share = if own > 0 { bad_pixels as f64 / own as f64 } else { 0.0 };
        if own == 0 || share < MIN_SHARE {
            continue;
        }
        found.push(((own as f64) * share, *layer, area, share, text));
    }
    found.sort_by(|a, b| b.0.total_cmp(&a.0));
    let count = found.len();
    found.truncate(MAX_LISTED);
    // Only the listed ones are searched for a move: it tries every offset, so it is the costly part.
    let differences: Vec<Value> = found
        .into_iter()
        .filter_map(|(_, layer, area, share, text)| {
            let n = doc.node(layer)?;
            let shift = moved(&design, &shot, area, tolerance);
            let (expected, actual) = (mean(&design, area), mean(&shot, area));
            let mut d = json!({
                "id": layer, "name": n.name, "type": n.kind.type_name(),
                "kind": if shift.is_some() { "position" } else if expected != actual && share > 0.8 { "color" } else { "content" },
                "percent": (share * 1000.0).round() / 10.0,
                "expected": expected, "actual": actual,
            });
            if let Some((dx, dy)) = shift {
                d["offset"] = json!({"x": (f64::from(dx) / per).round(), "y": (f64::from(dy) / per).round()});
            }
            if text {
                d["note"] = json!("text: check the font, size and line height before anything else");
            }
            Some(d)
        })
        .collect();
    let mut out = json!({
        "target": {"id": id, "name": root.name, "width": fw, "height": fh},
        "viewport": {"width": fw.ceil(), "height": fh.ceil()},
        "capture": {"width": capture.width(), "height": capture.height(), "density": density, "sameSize": same_size, "cssWidth": cw, "cssHeight": ch},
        "changedPercent": ((changed as f64 / (gw * gh) as f64) * 1000.0).round() / 10.0,
        "differences": differences,
        "total": count,
        "note": "changedPercent alone proves nothing: fonts and antialiasing differ between renderers. Work through `differences`, and take a position or colour difference more seriously than text content",
    });
    if layers_cut {
        // Layers past the cap weren't looked at: their differences fall to the layers above them.
        out["truncated"]["maxLayers"] = json!(MAX_LAYERS);
    }
    if by_box > 0 {
        // Past the limit, a layer was matched by its whole box: a see-through part of it may
        // hide a difference that belongs to a layer under it.
        out["truncated"]["matchedByBox"] = json!(by_box);
    }
    Ok(out)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![spec!(
        "inspect.compare",
        "Compare with a capture",
        "{id? (a frame, component or instance; default the first selected layer, refused when it is anything else), path | data (base64; not both): a capture of the built page, density? (device pixels per CSS pixel; default from the widths), tolerance? (out of 255, default 24)}  the layers that differ, deepest first  → {target, viewport {width, height}, capture {width, height, density, sameSize, cssWidth, cssHeight}, changedPercent, differences: [{id, name, type, kind: position|color|content, percent, expected, actual, offset?, note?}] (at most 100, largest first), total, note, truncated? {maxLayers?, matchedByBox?}}",
        false,
        compare
    )]
}
