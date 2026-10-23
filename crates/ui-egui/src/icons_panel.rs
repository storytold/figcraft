//! The Icons tab: the icon fonts this file carries, the icons in them, and the button that compiles
//! a font so a text layer can use it.
//!
//! Each font is a fold header — its name (click to rename), a rebuild button and a delete button —
//! over its icons. The search field at the top filters them by name and code point.
//!
//! An icon's preview: **click** copies its character to the clipboard, **drag** it onto the canvas
//! to place a text layer there, or drag it within the list to reorder it. Its name and code point
//! are edited in place; the ✕ deletes it.
//!
//! The previews come from the icon's stored outline (`figcraft_render::render_path`), not from the
//! compiled font, so they are there before the font is ever generated and stay right after it.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use egui::{Color32, CornerRadius, Id, Rect, Sense, Stroke, Ui, pos2, vec2};
use figcraft_doc::{IconFont, IconGlyph};
use figcraft_geom::BezPath;
use serde_json::json;

use crate::FigApp;
use crate::i18n::{tr, trf};
use crate::icons::{self, Icon};
use crate::theme::{Tokens, regular, semibold};

const PAD: f32 = 16.0;
/// The side of an icon's preview, in points.
const THUMB: f32 = 26.0;
/// The height of an icon's row, and of a font's header.
const ROW: f32 = 34.0;
const HEADER: f32 = 32.0;
/// Previews built per frame (the rest follow on the frames after, so a big font never stalls one).
const THUMBS_PER_FRAME: usize = 6;
/// The widest a code point's field gets ("U+10FFFF").
const CP_W: f32 = 56.0;

/// What a drag carries: the icon the pointer picked up. Set while dragging and read where it lands
/// (the canvas places a text layer; the list reorders).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconDrag {
    /// The `IconFont::id` the icon belongs to.
    pub font: String,
    /// The icon's name in that font.
    pub icon: String,
}

/// What a click asks for, run once the list is drawn: the document is only read while drawing.
enum Act {
    Copy { font: String, icon: String },
    Move { font: String, icon: String, to: usize },
    Delete { font: String, icon: String },
    Rename { font: String, icon: String, to: String },
    Codepoint { font: String, icon: String, to: String },
    Generate { font: String },
    Export { font: String, name: String },
    RenameFont { font: String, to: String },
    DeleteFont { font: String },
}

/// A preview: which font and icon it is of, the outline and the colour it was drawn in.
type Key = (String, String, u64, [u8; 4]);

/// Preview textures, so an outline is rasterised once and not once a frame.
#[derive(Default, Clone)]
struct Thumbs(HashMap<Key, Option<egui::TextureHandle>>);

fn outline_hash(path: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    h.finish()
}

/// What the panel lists: the fonts, each with only the icons `query` keeps.
struct Row<'a> {
    font: &'a IconFont,
    glyphs: Vec<&'a IconGlyph>,
    /// Whether every glyph stays in (`query` is empty): a dropped icon goes back where it was by
    /// position, which only means anything in the full list.
    whole: bool,
}

/// The fonts to list, filtered by `query` (name or code point, lowercase). A font with no match is
/// left out while searching.
fn listed<'a>(fonts: &'a [IconFont], query: &str) -> Vec<Row<'a>> {
    fonts
        .iter()
        .filter_map(|f| {
            let glyphs: Vec<&IconGlyph> = f
                .glyphs
                .iter()
                .filter(|g| {
                    query.is_empty()
                        || g.name.to_lowercase().contains(query)
                        || format!("{:04x}", g.codepoint).contains(query)
                        || format!("u+{:04x}", g.codepoint).contains(query)
                })
                .collect();
            (!glyphs.is_empty() || query.is_empty()).then_some(Row { font: f, glyphs, whole: query.is_empty() })
        })
        .collect()
}

pub fn show(app: &mut FigApp, ui: &mut Ui) {
    let w = ui.available_width();
    let Ok(ds) = app.session.ds() else { return };
    let fonts = ds.doc.icon_fonts().to_vec();
    let query = search_row(ui, w);
    if new_font_row(ui, w) {
        app.run("icon.font.create", json!({}));
        return;
    }
    if fonts.is_empty() {
        let keys = crate::toolbar::pretty_shortcut(crate::shortcuts::shortcut("icon.create").unwrap_or(""));
        hint(ui, &trf("No icon fonts yet. Select a layer and press {keys}.", &[("keys", &keys)]));
        return;
    }

    let t = Tokens::get(ui.ctx());
    let colour = t.text;
    let collapsed_id = Id::new("icon_fonts_collapsed");
    let collapsed: HashSet<String> = ui.data(|d| d.get_temp(collapsed_id)).unwrap_or_default();
    let mut thumbs: Thumbs = ui.data(|d| d.get_temp(Id::new("icon_thumbs"))).unwrap_or_default();
    let mut act: Option<Act> = None;
    let mut toggle: Option<String> = None;
    // Every preview on screen this frame, and the ones that still have to be built.
    let mut live: HashSet<Key> = HashSet::new();
    let mut wanted: Vec<(Key, String)> = Vec::new();
    let rows = listed(&fonts, &query);
    // The payload stays readable through the frame the pointer is let go on. Asking whether the
    // button is still down there answers no — it is already up — which is why the drop used to
    // never happen: the insertion line was not drawn, and nothing was listening for the release.
    let dragging = egui::DragAndDrop::payload::<IconDrag>(ui.ctx());
    let at = ui.ctx().pointer_interact_pos();
    let released = ui.ctx().input(|i| i.pointer.any_released());

    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        if rows.is_empty() {
            hint(ui, tr("No icons match the search."));
        }
        for row in &rows {
            let f = row.font;
            let open = !collapsed.contains(&f.id);
            if font_header(ui, w, f, open, &mut act) {
                toggle = Some(f.id.clone());
            }
            if !open {
                continue;
            }
            // Only a drag of this font's own icons can land in this font, and only when the whole
            // list is showing: while it is filtered, the places on screen are not the font's own.
            let drop = dragging.as_ref().filter(|d| d.font == f.id && row.whole);
            for (i, g) in row.glyphs.iter().enumerate() {
                let r = icon_row(ui, w, f, g, colour, &thumbs, &mut live, &mut wanted, &mut act, dragging.as_deref());
                // Where letting go over this row lands the icon: the half of the row the pointer is
                // in. Above its middle the icon takes this place, below it the next one — and past
                // the last row that is the end of the font, which is the place past the last icon.
                if released
                    && let (Some(d), Some(p)) = (drop, at)
                    && r.contains(p)
                {
                    let to = if p.y < r.center().y { i } else { i + 1 };
                    act.replace(Act::Move { font: f.id.clone(), icon: d.icon.clone(), to });
                }
            }
            if row.glyphs.is_empty() {
                hint(ui, tr("No icons in this font yet."));
            }
        }
    });

    // Keep only the previews still on screen, then build the ones that are missing. `live` is every
    // row drawn this frame — not just the missing ones — so a thumb built last frame survives into
    // this one instead of being dropped and rebuilt on the next, which is what made them flicker.
    refresh_thumbs(ui.ctx(), &mut thumbs, &live, &wanted);
    ui.data_mut(|d| d.insert_temp(Id::new("icon_thumbs"), thumbs));

    if let Some(id) = toggle {
        let mut c = collapsed;
        if !c.remove(&id) {
            c.insert(id);
        }
        ui.data_mut(|d| d.insert_temp(collapsed_id, c));
    }
    run(app, act);
}

/// The search field; returns the lowercase query the list filters on.
fn search_row(ui: &mut Ui, w: f32) -> String {
    let t = Tokens::get(ui.ctx());
    let filter_id = Id::new("icon_filter");
    let mut filter: String = ui.data(|d| d.get_temp(filter_id)).unwrap_or_default();
    let (fr, _) = ui.allocate_exact_size(vec2(w, 44.0), Sense::hover());
    let field = Rect::from_min_max(pos2(fr.min.x + PAD, fr.min.y + 10.0), pos2(fr.max.x - PAD, fr.max.y - 6.0));
    ui.painter().rect_filled(field, CornerRadius::same(5), t.field);
    icons::paint(ui.painter(), Rect::from_center_size(pos2(field.min.x + 12.0, field.center().y), vec2(12.0, 12.0)), Icon::Search, t.icon_secondary);
    let te_rect = Rect::from_min_max(pos2(field.min.x + 24.0, field.center().y - 7.0), pos2(field.max.x - 6.0, field.center().y + 7.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(te_rect));
    let r = child.add(
        egui::TextEdit::singleline(&mut filter)
            .id(Id::new("icon_filter_field"))
            .frame(egui::Frame::NONE)
            .font(regular(11.0))
            .hint_text(tr("Search icons"))
            .desired_width(te_rect.width()),
    );
    crate::widgets::select_all_on_focus(ui, &r, &filter);
    if r.has_focus() {
        ui.painter().rect_stroke(field, CornerRadius::same(5), Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            filter.clear();
        }
    }
    ui.data_mut(|d| d.insert_temp(filter_id, filter.clone()));
    filter.trim().to_lowercase()
}

/// A font's header: fold it away, rename it, rebuild it, delete it. True when the header was
/// clicked to fold.
fn font_header(ui: &mut Ui, w: f32, f: &IconFont, open: bool, act: &mut Option<Act>) -> bool {
    let t = Tokens::get(ui.ctx());
    let (hr, resp) = ui.allocate_exact_size(vec2(w, HEADER), Sense::click());
    // The whole header lights up under the pointer — the font's name included — because the whole
    // header is what folds the font, and pressing it has to look like it does something.
    if resp.hovered() {
        let fill = if resp.is_pointer_button_down_on() { t.pressed } else { t.hover };
        ui.painter().rect_filled(hr.shrink2(vec2(8.0, 1.0)), CornerRadius::same(5), fill);
    }
    let chevron = Rect::from_center_size(pos2(hr.min.x + PAD + 5.0, hr.center().y), vec2(12.0, 12.0));
    icons::paint(ui.painter(), chevron, if open { Icon::Chevron } else { Icon::ChevronRight }, t.icon_secondary);
    let del = Rect::from_center_size(pos2(hr.max.x - PAD - 8.0, hr.center().y), vec2(20.0, 20.0));
    let gen_rect = Rect::from_center_size(pos2(del.min.x - 6.0 - 10.0, hr.center().y), vec2(20.0, 20.0));
    let exp_rect = Rect::from_center_size(pos2(gen_rect.min.x - 6.0 - 10.0, hr.center().y), vec2(20.0, 20.0));
    let name = Rect::from_min_max(pos2(chevron.max.x + 6.0, hr.min.y + 7.0), pos2(exp_rect.min.x - 8.0, hr.max.y - 7.0));
    if crate::widgets::icon_button_at(ui, Id::new(("icon_font_exp", &f.id)), exp_rect, Icon::Export, tr("Export the icon font")) {
        *act = Some(Act::Export { font: f.id.clone(), name: f.name.clone() });
    }
    if crate::widgets::icon_button_at(ui, Id::new(("icon_font_gen", &f.id)), gen_rect, Icon::Reset, tr("Generate the icon font")) {
        *act = Some(Act::Generate { font: f.id.clone() });
    }
    if crate::widgets::icon_button_at(ui, Id::new(("icon_font_del", &f.id)), del, Icon::Close, tr("Delete icon font")) {
        *act = Some(Act::DeleteFont { font: f.id.clone() });
    }
    if let Some(new) = crate::widgets::text_field_inline(ui, Id::new(("icon_font_name", &f.id)), &f.name, name, semibold(11.0))
        && new.trim() != f.name
    {
        *act = Some(Act::RenameFont { font: f.id.clone(), to: new });
    }
    // The name and the buttons have the click; the rest of the header folds the font away.
    let over_controls =
        ui.rect_contains_pointer(name) || ui.rect_contains_pointer(gen_rect) || ui.rect_contains_pointer(exp_rect) || ui.rect_contains_pointer(del);
    resp.clicked() && !over_controls
}

/// One icon: the whole row is the hand — press it anywhere and drag it out to the canvas or up and
/// down the list, or click it to copy its character. Its name, its code point and its ✕ are made
/// after the row and so sit above it, keeping their own clicks.
///
/// The row's rectangle comes back so the caller can say where a drop over it would land.
#[allow(clippy::too_many_arguments)]
fn icon_row(
    ui: &mut Ui,
    w: f32,
    f: &IconFont,
    g: &IconGlyph,
    colour: Color32,
    thumbs: &Thumbs,
    live: &mut HashSet<Key>,
    wanted: &mut Vec<(Key, String)>,
    act: &mut Option<Act>,
    dragging: Option<&IconDrag>,
) -> Rect {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(w, ROW), Sense::hover());
    // The whole row, made before anything on it: a press anywhere in it can start a drag, and the
    // fields and the ✕ drawn after this win their own corners of it.
    let grab = ui.interact(r, Id::new(("icon_grab", &f.id, &g.name)), Sense::click_and_drag());
    let being_dragged = dragging.is_some_and(|d| d.font == f.id && d.icon == g.name);
    if ui.rect_contains_pointer(r) && !being_dragged {
        ui.painter().rect_filled(r.shrink2(vec2(8.0, 1.0)), CornerRadius::same(5), t.hover);
    }
    let key: Key = (f.id.clone(), g.name.clone(), outline_hash(&g.path), colour.to_array());
    live.insert(key.clone());
    let preview = Rect::from_center_size(pos2(r.min.x + PAD + THUMB / 2.0, r.center().y), vec2(THUMB, THUMB));
    match thumbs.0.get(&key) {
        Some(Some(tex)) => {
            ui.painter().image(tex.id(), preview, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        Some(None) => {
            // The outline holds nothing to draw, and re-rendering it every frame won't change that.
            // A dashed box says so; the placeholder icon would look like a real, wrong glyph.
            ui.painter().rect_stroke(preview.shrink(4.0), CornerRadius::same(3), egui::Stroke::new(1.0, t.text_tertiary), egui::StrokeKind::Inside);
        }
        None => {
            // Until the preview is built (this frame, or the next): a dimmed stand-in.
            icons::paint(ui.painter(), preview.shrink(3.0), Icon::Icons, t.icon_secondary);
            wanted.push((key.clone(), g.path.clone()));
        }
    }

    let drag = IconDrag { font: f.id.clone(), icon: g.name.clone() };
    if grab.drag_started() {
        egui::DragAndDrop::set_payload(ui.ctx(), drag.clone());
    }
    if grab.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        // A small copy of the preview follows the pointer, so the drag has something to see.
        if let Some(pos) = ui.ctx().pointer_hover_pos() {
            let ghost = Rect::from_center_size(pos, vec2(THUMB, THUMB));
            ui.painter().rect_filled(ghost.expand(3.0), CornerRadius::same(5), t.hover);
            if let Some(Some(tex)) = thumbs.0.get(&key) {
                ui.painter().image(tex.id(), ghost, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), t.text);
            }
        }
    }
    if grab.clicked() {
        *act = Some(Act::Copy { font: f.id.clone(), icon: g.name.clone() });
    }

    let del = Rect::from_center_size(pos2(r.max.x - PAD - 8.0, r.center().y), vec2(20.0, 20.0));
    let cp = Rect::from_center_size(pos2(del.min.x - 8.0 - CP_W / 2.0, r.center().y), vec2(CP_W, 18.0));
    let name = Rect::from_min_max(pos2(preview.max.x + 8.0, r.min.y + 8.0), pos2(cp.min.x - 8.0, r.max.y - 8.0));
    if crate::widgets::icon_button_at(ui, Id::new(("icon_del", &f.id, &g.name)), del, Icon::Close, tr("Delete icon")) {
        *act = Some(Act::Delete { font: f.id.clone(), icon: g.name.clone() });
    }
    if let Some(new) = crate::widgets::text_field_inline(ui, Id::new(("icon_name", &f.id, &g.name)), &g.name, name, regular(11.0))
        && new.trim() != g.name
    {
        *act = Some(Act::Rename { font: f.id.clone(), icon: g.name.clone(), to: new });
    }
    let shown = format!("U+{:04X}", g.codepoint);
    if let Some(new) = crate::widgets::text_field_inline(ui, Id::new(("icon_cp", &f.id, &g.name)), &shown, cp, regular(10.0))
        && new.trim() != shown
    {
        *act = Some(Act::Codepoint { font: f.id.clone(), icon: g.name.clone(), to: new });
    }
    r
}

/// The "New icon font" button; true when it was clicked.
fn new_font_row(ui: &mut Ui, w: f32) -> bool {
    let (r, _) = ui.allocate_exact_size(vec2(w, 40.0), Sense::hover());
    let br = Rect::from_min_max(pos2(r.min.x + PAD, r.min.y + 4.0), pos2(r.max.x - PAD, r.max.y - 8.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(br));
    crate::widgets::button(&mut child, tr("New icon font"), false, true).clicked()
}

fn hint(ui: &mut Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width();
    let galley = ui.painter().layout(text.to_string(), regular(11.0), t.text_secondary, (w - PAD * 2.0).max(40.0));
    let (r, _) = ui.allocate_exact_size(vec2(w, galley.size().y + 12.0), Sense::hover());
    ui.painter().galley(pos2(r.min.x + PAD, r.min.y + 2.0), galley, t.text_secondary);
}

/// Decide which previews live on, which to build, and in what order, then build up to
/// [`THUMBS_PER_FRAME`] of the missing ones.
///
/// `live` is every preview drawn this frame and `missing` the ones with no texture yet. Dropping
/// the thumbs that are not in `live` and then building `missing` is the whole cache policy: a thumb
/// built on one frame is in `live` on the next, so it is kept. (Keeping only `missing` would drop
/// each thumb the frame after it was built and rebuild it the frame after that: the flicker the
/// `icons_panel` previews used to have.)
fn refresh_thumbs(ctx: &egui::Context, thumbs: &mut Thumbs, live: &HashSet<Key>, missing: &[(Key, String)]) {
    thumbs.0.retain(|k, _| live.contains(k));
    let mut built = 0;
    for (key, path) in missing {
        if built >= THUMBS_PER_FRAME {
            break;
        }
        if thumbs.0.contains_key(key) {
            continue;
        }
        built += 1;
        let tex = build_thumb(ctx, path, key.3);
        thumbs.0.insert(key.clone(), tex);
    }
}

/// The texture for one outline, drawn in `colour`. `None` when it holds nothing to draw.
fn build_thumb(ctx: &egui::Context, path: &str, colour: [u8; 4]) -> Option<egui::TextureHandle> {
    let outline = BezPath::from_svg(path).ok()?;
    let [r, g, b, a] = colour;
    let px = (THUMB * 4.0) as u32;
    let pix = figcraft_render::render_path(&outline, px, figcraft_color::Color::from_rgba8(r, g, b, a))?;
    let image = crate::render_worker::screen_image(&pix, figcraft_color::ColorProfile::Srgb);
    Some(ctx.load_texture(format!("icon-thumb-{px}-{r:02X}{g:02X}{b:02X}"), image, egui::TextureOptions::LINEAR))
}

fn run(app: &mut FigApp, act: Option<Act>) {
    let Some(act) = act else { return };
    match act {
        Act::Copy { font, icon } => {
            // The character the icon stands for, from the engine (it knows the code point; the
            // panel only knows the name).
            let ch = app.run("icon.list", json!({"font": font})).and_then(|out| {
                out["fonts"]
                    .as_array()?
                    .iter()
                    .flat_map(|f| f["glyphs"].as_array().map(Vec::as_slice).unwrap_or_default())
                    .find(|g| g["name"].as_str() == Some(icon.as_str()))
                    .and_then(|g| g["character"].as_str())
                    .map(str::to_string)
            });
            if let Some(ch) = ch {
                app.services.clipboard_set_text(&ch);
                app.toast(trf("Copied {icon}", &[("icon", &icon)]));
            }
        }
        Act::Move { font, icon, to } => {
            app.run("icon.move", json!({"font": font, "name": icon, "to": to}));
        }
        Act::Delete { font, icon } => {
            app.run("icon.delete", json!({"font": font, "name": icon}));
        }
        Act::Rename { font, icon, to } => {
            app.run("icon.rename", json!({"font": font, "name": icon, "to": to}));
        }
        Act::Codepoint { font, icon, to } => {
            app.run("icon.setCodepoint", json!({"font": font, "name": icon, "codepoint": to}));
        }
        Act::Generate { font } => {
            // Refreshing is part of generating. A font is compiled from the outlines its icons store,
            // so an icon whose layer has been edited since it was made would otherwise be compiled as
            // the shape it used to be: every icon that has a layer behind it takes its outline again
            // first, and only then is the font built.
            let names: Vec<String> = app
                .session
                .ds()
                .ok()
                .and_then(|ds| {
                    ds.doc.icon_font(&font).map(|f| f.glyphs.iter().filter(|g| g.node.is_some()).map(|g| g.name.clone()).collect::<Vec<String>>())
                })
                .unwrap_or_default();
            for name in names {
                app.run("icon.update", json!({"font": font.clone(), "name": name}));
            }
            app.run("icon.generateFont", json!({"font": font}));
        }
        Act::Export { font, name } => {
            // The font goes wherever the user says, as `<name>.ttf` — or `.woff`, which the engine
            // reads from the name: the same font with its tables deflated, for a web page.
            let file = format!("{name}.ttf");
            app.pick_save(tr("Export the icon font"), &file, &["ttf", "woff"], move |app, path| {
                let ps = path.to_string_lossy().to_string();
                if let Some(out) = app.run("icon.exportFont", json!({"font": font, "path": ps})) {
                    let n = out["icons"].as_array().map(Vec::len).unwrap_or(0);
                    app.toast(trf("Exported {n} icons as {name}", &[("n", &n.to_string()), ("name", &name)]));
                }
            });
        }
        Act::RenameFont { font, to } => {
            app.run("icon.font.rename", json!({"font": font, "name": to}));
        }
        Act::DeleteFont { font } => {
            app.run("icon.font.delete", json!({"font": font}));
        }
    }
}

/// The fonts a panel test can reach without a session: the ones [`show`] would list.
#[cfg(test)]
pub(crate) fn fonts_of(doc: &figcraft_doc::Document) -> Vec<IconFont> {
    doc.icon_fonts().to_vec()
}

#[cfg(test)]
#[path = "icons_panel_tests.rs"]
mod tests;
