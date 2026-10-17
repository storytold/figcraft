//! Text drawn in its own font by FigCraft's text engine, as textures for the panels: font family
//! names in the font picker, text styles in the style browser. Rendered lazily (a few per frame:
//! callers pass a budget) and cached for the session. Textures are white; paint them tinted.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use egui::Id;
use figcraft_color::{Color, Paint};
use figcraft_doc::{Document, Node, NodeKind, TextData, TextStyle};

/// Most previews kept before the cache starts over.
const MAX: usize = 2000;

#[derive(Clone, Default)]
struct Cache(Arc<Mutex<HashMap<u64, Option<egui::TextureHandle>>>>);

/// What a preview looks like: the typography that shows plus the text and the screen's scale, and
/// the fonts there are (a font found or downloaded later draws it anew).
fn key(style: &TextStyle, text: &str, ppp: f32) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (&style.font_family, &style.font_style, style.font_size.to_bits(), text, ppp.to_bits()).hash(&mut h);
    figcraft_text::layout_generation().hash(&mut h);
    (style.line_height.label(), style.letter_spacing.label(), style.decoration as u8, style.case as u8).hash(&mut h);
    for (k, v) in &style.variations {
        (k, v.to_bits()).hash(&mut h);
    }
    h.finish()
}

/// `text` in `style` (one line, at most 400 × 60 points), or None when it can't be drawn: no such
/// font, or `budget` previews were already rendered this frame (asks for another frame then).
pub fn texture(ctx: &egui::Context, style: &TextStyle, text: &str, budget: &mut usize) -> Option<egui::TextureHandle> {
    let ppp = ctx.pixels_per_point();
    let k = key(style, text, ppp);
    let cache = ctx.data_mut(|d| d.get_temp_mut_or_default::<Cache>(Id::new("font_previews")).clone());
    if let Some(t) = cache.0.lock().ok()?.get(&k) {
        return t.clone();
    }
    if *budget == 0 {
        ctx.request_repaint();
        return None;
    }
    *budget -= 1;
    let tex = render(ctx, style, text, f64::from(ppp));
    let mut map = cache.0.lock().ok()?;
    if map.len() > MAX {
        map.clear();
    }
    map.insert(k, tex.clone());
    tex
}

/// A family's name in its own regular style (None when the family doesn't draw itself: symbol
/// fonts, or it resolves to another family).
pub fn family(ctx: &egui::Context, family: &str, size: f64, budget: &mut usize) -> Option<egui::TextureHandle> {
    let styles = figcraft_text::family_styles(family);
    let style = styles.iter().find(|s| s.eq_ignore_ascii_case("Regular")).or(styles.first())?.clone();
    let face = figcraft_text::find_face(family, &style)?;
    if !face.family.eq_ignore_ascii_case(family) || !family.chars().all(|c| c == ' ' || face.has_char(c)) {
        return None;
    }
    let st = TextStyle { font_family: family.to_string(), font_style: style, font_size: size, ..TextStyle::default() };
    texture(ctx, &st, family, budget)
}

fn render(ctx: &egui::Context, style: &TextStyle, text: &str, ppp: f64) -> Option<egui::TextureHandle> {
    let mut doc = Document::new("preview");
    let page = doc.first_page()?;
    let id = doc.alloc_id();
    let mut td = TextData::new(text);
    td.style = TextStyle { fills: None, link: None, style_id: None, ..style.clone() };
    let mut n = Node::new(id, NodeKind::Text(td), "preview", 1.0, 1.0);
    n.fills = vec![Paint::solid(Color::WHITE)];
    let (w, h) = figcraft_text::auto_size(&n)?;
    n.width = w.min(400.0);
    n.height = h.min(60.0);
    doc.insert(n, page, None).ok()?;
    let pix = figcraft_render::render_node(&doc, id, ppp.clamp(1.0, 3.0))?;
    let (pw, ph) = (pix.width() as usize, pix.height() as usize);
    let rgba = figcraft_render::to_rgba8(&pix);
    if rgba.len() != pw * ph * 4 || pw == 0 || ph == 0 {
        return None;
    }
    let img = egui::ColorImage::from_rgba_unmultiplied([pw, ph], &rgba);
    Some(ctx.load_texture("font-preview", img, egui::TextureOptions::LINEAR))
}

/// Paint a preview texture with its left edge at `left_center`'s x, centred on its y, clipped to
/// `max_w` × `max_h` points, tinted `color`; returns the painted width.
pub fn paint(painter: &egui::Painter, tex: &egui::TextureHandle, left_center: egui::Pos2, max_w: f32, max_h: f32, color: egui::Color32) -> f32 {
    let ppp = painter.ctx().pixels_per_point().max(0.5);
    let size = tex.size_vec2() / ppp;
    let shown = egui::vec2(size.x.min(max_w.max(0.0)), size.y.min(max_h.max(0.0)));
    // Clip from the middle vertically (tall line heights), from the start horizontally.
    let v0 = ((size.y - shown.y) / 2.0 / size.y.max(1.0)).max(0.0);
    let uv = egui::Rect::from_min_max(egui::pos2(0.0, v0), egui::pos2(shown.x / size.x.max(1.0), v0 + shown.y / size.y.max(1.0)));
    let r = egui::Rect::from_min_size(egui::pos2(left_center.x, left_center.y - shown.y / 2.0), shown);
    painter.image(tex.id(), r, uv, color);
    shown.x
}
