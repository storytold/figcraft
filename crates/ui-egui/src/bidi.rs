//! Names in right-to-left and mixed-direction scripts (Hebrew, Arabic…) in the panels: layer, page,
//! style, component and file names. egui has no bidi: it shapes each run of one font on its own,
//! reversing right-to-left runs but leaving the words and runs of a name in logical order. So
//! names go through [`figcraft_text::display`] (visual order, Arabic letters joined, mirrored
//! brackets) and every shown cluster is laid out as its own section, which egui's shaper leaves as
//! it is. Names are elided at their end (the left end of a right-to-left name) and edited by
//! [`single_line`], whose caret, clicks and selection follow the shown text (the arrow keys move
//! visually). Left-to-right names take egui's own path unchanged.

use std::collections::HashMap;
use std::sync::Arc;

use egui::text::{ByteIndex, CCursor, CCursorRange, LayoutJob, LayoutSection, TextFormat, TextWrapping};
use egui::{Color32, FontId, Galley, Id, Painter, Pos2, Rect, Response, Stroke, Ui, pos2, vec2};
use figcraft_text::display::{self, VisualLine};

/// Shown (visual, elided) lines by name, font and width: elision measures many candidates, so the
/// result is kept between frames (cleared when it grows large).
#[derive(Clone, Default)]
struct Elided(Arc<std::sync::Mutex<HashMap<ElidedKey, VisualLine>>>);

/// A name, its font's size and family (proportional or not) and the width it was cut to (bits).
type ElidedKey = (String, u32, bool, u32);

const ELIDED_MAX: usize = 4096;

/// `name` on one line in `font`, at most `max_w` wide (ending in an ellipsis when cut), ready to
/// paint: in visual order with Arabic letters joined when it has right-to-left text.
pub fn name_galley(painter: &Painter, name: &str, font: FontId, color: Color32, max_w: f32) -> Arc<Galley> {
    let max_w = max_w.max(1.0);
    if !display::has_rtl(name) {
        let mut job = LayoutJob::simple_singleline(name.to_string(), font, color);
        job.wrap = TextWrapping::truncate_at_width(max_w);
        return painter.layout_job(job);
    }
    let key = (name.to_string(), font.size.to_bits(), matches!(font.family, egui::FontFamily::Proportional), max_w.to_bits());
    let cache = painter.ctx().data_mut(|d| d.get_temp_mut_or_default::<Elided>(Id::new("bidi_elided")).clone());
    let hit = cache.0.lock().ok().and_then(|m| m.get(&key).cloned());
    let shown = hit.unwrap_or_else(|| {
        let fits = |v: &VisualLine| visual_galley(painter, v, &font, color).size().x <= max_w;
        let v = display::elide(name, fits);
        if let Ok(mut m) = cache.0.lock() {
            if m.len() >= ELIDED_MAX {
                m.clear();
            }
            m.insert(key, v.clone());
        }
        v
    });
    visual_galley(painter, &shown, &font, color)
}

/// A line in display order laid out with each cluster as its own section, so egui's shaper (which
/// would reverse a right-to-left run once more) gets one ordered, joined cluster at a time.
fn visual_galley(painter: &Painter, line: &VisualLine, font: &FontId, color: Color32) -> Arc<Galley> {
    let chars: Vec<char> = line.text.chars().collect();
    let format = TextFormat::simple(font.clone(), color);
    let mut job = LayoutJob::default();
    for c in &line.clusters {
        let start = job.text.len();
        job.text.extend(chars.get(c.shown.clone()).unwrap_or_default());
        // Pushed as is: `LayoutJob::append` would merge it with the section before.
        if job.text.len() > start {
            job.sections.push(LayoutSection { leading_space: 0.0, byte_range: ByteIndex(start)..ByteIndex(job.text.len()), format: format.clone() });
        }
    }
    painter.layout_job(job)
}

/// Paint `name` (see [`name_galley`]) with its left end at `left_center`'s x, centred on its y;
/// returns the painted rectangle.
pub fn paint_name(painter: &Painter, left_center: Pos2, name: &str, font: FontId, color: Color32, max_w: f32) -> Rect {
    let galley = name_galley(painter, name, font, color, max_w);
    let rect = Rect::from_min_size(pos2(left_center.x, left_center.y - galley.size().y / 2.0), galley.size());
    painter.galley(rect.min, galley, color);
    rect
}

/// Paint a single-line name clipped to `max_w`, fading into `background` instead of eliding it.
pub fn paint_name_faded(painter: &Painter, left_center: Pos2, name: &str, font: FontId, color: Color32, max_w: f32, background: Color32) -> Rect {
    let rtl = display::has_rtl(name);
    let galley =
        if rtl { visual_galley(painter, &display::visual_line(name), &font, color) } else { painter.layout_no_wrap(name.to_string(), font, color) };
    let galley_size = galley.size();
    let max_w = max_w.max(1.0);
    let clip = Rect::from_min_size(pos2(left_center.x, left_center.y - galley_size.y / 2.0), vec2(max_w, galley_size.y));
    let painter = painter.with_clip_rect(clip);
    let galley_pos = pos2(if rtl { clip.max.x - galley_size.x } else { clip.min.x }, clip.min.y);
    painter.galley(galley_pos, galley, color);
    if galley_size.x > max_w {
        let fade_width = max_w.min(24.0);
        let (fade_left, fade_right) = if rtl { (clip.min.x, clip.min.x + fade_width) } else { (clip.max.x - fade_width, clip.max.x) };
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(pos2(fade_left, clip.min.y), background.gamma_multiply(0.0));
        mesh.colored_vertex(pos2(fade_right, clip.min.y), background);
        mesh.colored_vertex(pos2(fade_right, clip.max.y), background);
        mesh.colored_vertex(pos2(fade_left, clip.max.y), background.gamma_multiply(0.0));
        mesh.indices = vec![0, 1, 2, 0, 2, 3];
        painter.add(egui::Shape::mesh(mesh));
    }
    if rtl {
        Rect::from_min_max(pos2(galley_pos.x.max(clip.min.x), clip.min.y), pos2(clip.max.x, clip.max.y))
    } else {
        Rect::from_min_size(galley_pos, vec2(galley_size.x.min(max_w), galley_size.y))
    }
}

/// Each shown cluster's left and right x in a galley of `v.text` laid out from x = 0.
fn edges(v: &VisualLine, galley: &Galley) -> Vec<(f64, f64)> {
    let chars: Vec<(f64, f64)> = galley
        .rows
        .first()
        .map(|r| r.row.glyphs.iter().map(|g| (f64::from(g.pos.x), f64::from(g.pos.x + g.advance_width))).collect())
        .unwrap_or_default();
    v.cluster_edges(&chars)
}

/// The shown text of a field being edited, laid out once per frame.
struct Shown {
    line: VisualLine,
    galley: Arc<Galley>,
    edges: Vec<(f64, f64)>,
}

impl Shown {
    fn new(ui: &Ui, text: &str, font: &FontId, color: Color32) -> Shown {
        let line = display::visual_line(text);
        let galley = visual_galley(ui.painter(), &line, font, color);
        let edges = edges(&line, &galley);
        Shown { line, galley, edges }
    }

    fn caret_x(&self, i: usize) -> f32 {
        self.line.caret_x(i, &self.edges) as f32
    }

    fn hit(&self, x: f32) -> usize {
        self.line.hit(f64::from(x), &self.edges)
    }
}

fn scroll_id(id: Id) -> Id {
    id.with("bidi_scroll")
}

fn anchor_id(id: Id) -> Id {
    id.with("bidi_anchor")
}

/// A frameless one-line text editor for a name (a [`egui::TextEdit`] with id `id`). Text with
/// right-to-left characters shows in visual order, and the caret, the selection, clicks, drags and
/// the left and right arrow keys follow what's on screen.
pub fn single_line(ui: &mut Ui, id: Id, text: &mut String, font: FontId, width: f32) -> Response {
    let rtl = display::has_rtl(text);
    let focused = ui.memory(|m| m.has_focus(id));
    let color = ui.visuals().text_color();
    if rtl && focused {
        visual_arrows(ui, id, text, &font, color);
    }
    // Centred in the height it's given (a rename row), like the name it replaces.
    let height = ui.available_height().max(0.0);
    let edit = egui::TextEdit::singleline(text)
        .id(id)
        .frame(egui::Frame::NONE)
        .font(font.clone())
        .desired_width(width)
        .vertical_align(egui::Align::Center)
        .min_size(vec2(0.0, height));
    if !rtl {
        ui.data_mut(|d| d.remove::<f32>(scroll_id(id)));
        return edit.show(ui).response.response;
    }
    // egui would draw the chars in logical order: hide its text, caret and selection and draw the
    // shown text over it.
    let out = ui
        .scope(|ui| {
            let v = ui.visuals_mut();
            v.selection.bg_fill = Color32::TRANSPARENT;
            v.selection.stroke = Stroke::new(0.0, Color32::TRANSPARENT);
            v.text_cursor.stroke = Stroke::NONE;
            edit.text_color(Color32::TRANSPARENT).show(ui)
        })
        .inner;
    let response = out.response.response;
    let shown = Shown::new(ui, text, &font, color);
    let clip = out.text_clip_rect;
    let mut scroll: f32 = ui.data(|d| d.get_temp(scroll_id(id))).unwrap_or(0.0);
    let origin = |scroll: f32| pos2(clip.min.x - scroll, out.galley_pos.y);
    // Clicks and drags place the caret by what's on screen.
    let mut range = out.state.cursor.char_range();
    if let Some(p) = response.interact_pointer_pos() {
        let at = shown.hit(p.x - origin(scroll).x);
        if response.double_clicked() {
            let (a, b) = word_at(text, at);
            range = Some(CCursorRange::two(CCursor::new(a), CCursor::new(b)));
        } else if response.clicked() || (response.is_pointer_button_down_on() && !response.dragged()) {
            // Pressed (not yet dragging): the caret and the drag's anchor go here.
            ui.data_mut(|d| d.insert_temp(anchor_id(id), at));
            range = Some(CCursorRange::one(CCursor::new(at)));
        } else if response.dragged() {
            let anchor = ui.data(|d| d.get_temp::<usize>(anchor_id(id))).unwrap_or(at);
            range = Some(CCursorRange { primary: CCursor::new(at), secondary: CCursor::new(anchor), h_pos: None });
        }
        if range != out.state.cursor.char_range() {
            let mut state = out.state.clone();
            state.cursor.set_char_range(range);
            state.store(ui.ctx(), id);
        }
    }
    // Keep the caret in view.
    let caret = range.map_or(0, |r| r.primary.index.0);
    let cx = shown.caret_x(caret);
    let w = clip.width().max(1.0);
    if cx - scroll > w - 1.0 {
        scroll = cx - w + 1.0;
    }
    if cx - scroll < 0.0 {
        scroll = cx;
    }
    scroll = scroll.clamp(0.0, (shown.galley.size().x - w + 1.0).max(0.0));
    ui.data_mut(|d| d.insert_temp(scroll_id(id), scroll));
    let o = origin(scroll);
    let painter = ui.painter().with_clip_rect(clip.expand2(vec2(1.0, 0.0)));
    let h = shown.galley.size().y;
    if focused && let Some(r) = range {
        let (a, b) = (r.primary.index.0, r.secondary.index.0);
        for (x0, x1) in shown.line.selection(a, b, &shown.edges) {
            let sel = Rect::from_min_max(pos2(o.x + x0 as f32, o.y), pos2(o.x + x1 as f32, o.y + h));
            painter.rect_filled(sel, 0.0, ui.visuals().selection.bg_fill);
        }
    }
    painter.galley(o, shown.galley.clone(), color);
    if focused {
        let x = (o.x + cx).round() + 0.5;
        painter.line_segment([pos2(x, o.y), pos2(x, o.y + h)], ui.visuals().text_cursor.stroke);
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
    }
    response
}

/// Plain and Shift-held left and right arrows move the caret across the shown text (consumed so
/// the editor doesn't move it logically). Word and line jumps stay logical.
fn visual_arrows(ui: &mut Ui, id: Id, text: &str, font: &FontId, color: Color32) {
    let moves: Vec<(bool, bool)> = ui.input_mut(|i| {
        let mut out = Vec::new();
        i.events.retain(|e| match e {
            egui::Event::Key { key: key @ (egui::Key::ArrowLeft | egui::Key::ArrowRight), pressed: true, modifiers, .. }
                if !modifiers.alt && !modifiers.command && !modifiers.ctrl =>
            {
                out.push((*key == egui::Key::ArrowRight, modifiers.shift));
                false
            }
            _ => true,
        });
        out
    });
    if moves.is_empty() {
        return;
    }
    let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) else { return };
    let shown = Shown::new(ui, text, font, color);
    let mut r = state.cursor.char_range().unwrap_or_default();
    for (right, shift) in moves {
        let (lo, hi) = (r.primary.index.0.min(r.secondary.index.0), r.primary.index.0.max(r.secondary.index.0));
        let next = if !shift && lo != hi {
            // A selection collapses to its end on the side of the arrow.
            let (xa, xb) = (shown.caret_x(lo), shown.caret_x(hi));
            if (xa < xb) == right { hi } else { lo }
        } else {
            shown.line.step(r.primary.index.0, right, &shown.edges)
        };
        r = if shift {
            CCursorRange { primary: CCursor::new(next), secondary: r.secondary, h_pos: None }
        } else {
            CCursorRange::one(CCursor::new(next))
        };
    }
    state.cursor.set_char_range(Some(r));
    state.store(ui.ctx(), id);
}

/// The word around char `i` (letters, digits and marks; else the single char).
fn word_at(text: &str, i: usize) -> (usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    let is_word =
        |k: usize| chars.get(k).is_some_and(|c| c.is_alphanumeric() || matches!(*c as u32, 0x0300..=0x036F | 0x0591..=0x05C7 | 0x064B..=0x065F));
    let i = i.min(chars.len());
    let at = if is_word(i) {
        i
    } else if i > 0 && is_word(i - 1) {
        i - 1
    } else {
        return (i, (i + 1).min(chars.len()));
    };
    let mut a = at;
    while a > 0 && is_word(a - 1) {
        a -= 1;
    }
    let mut b = at;
    while is_word(b) {
        b += 1;
    }
    (a, b)
}

/// Faces the UI adds for scripts its own fonts lack (desktop: from the system), as egui font data;
/// `taken` as for [`crate::theme::system_fallbacks`].
pub fn script_fallbacks(have: &dyn Fn(char) -> bool, taken: &mut Vec<Arc<figcraft_text::Face>>) -> Vec<(String, egui::FontData)> {
    // Hebrew, Arabic letters, Arabic presentation forms (what joined names are drawn with).
    const SAMPLES: [char; 3] = ['\u{05D0}', '\u{0628}', '\u{FE91}'];
    const FAMILIES: [&str; 11] = [
        "Segoe UI",
        "Arial",
        "Tahoma",
        "Noto Sans Hebrew",
        "Noto Sans Arabic",
        "Arial Hebrew",
        "Geeza Pro",
        "Lucida Grande",
        "DejaVu Sans",
        "FreeSans",
        "Noto Sans",
    ];
    crate::theme::system_fallbacks(SAMPLES, &FAMILIES, have, taken)
}

#[cfg(test)]
#[path = "rtl_names_tests.rs"]
mod tests;
