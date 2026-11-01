//! Right-to-left and mixed-direction names and rename fields, driven through egui headless.
//! egui 0.37 shapes logical text with the Unicode bidi algorithm and keeps logical char offsets;
//! these tests assert that logical text and the actual painted order, not a preordered string.

use egui::text::{CCursor, CCursorRange};
use egui::{Event, Key, Modifiers, Pos2, RawInput, Rect, vec2};

use super::*;
use crate::theme::regular;

fn frame(ctx: &egui::Context, events: Vec<Event>, mut f: impl FnMut(&mut Ui)) {
    let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), events, ..Default::default() };
    ctx.run_ui(input, |ui| f(ui)).textures_delta.clear();
}

fn key(k: Key, shift: bool) -> Event {
    Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: if shift { Modifiers::SHIFT } else { Modifiers::NONE } }
}

fn caret(ctx: &egui::Context, id: Id) -> (usize, usize) {
    let r = egui::TextEdit::load_state(ctx, id).and_then(|s| s.cursor.char_range()).unwrap_or_default();
    (r.primary.index.0, r.secondary.index.0)
}

fn set_caret(ctx: &egui::Context, id: Id, i: usize) {
    let mut s = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    s.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(i))));
    s.store(ctx, id);
}

/// The caret's x in a native logical galley laid out from x = 0. These short, newline-free
/// samples use the same font and layout as the rename field; this is not a clipping oracle.
fn caret_x(ctx: &egui::Context, text: &str, cursor: CCursor) -> f32 {
    ctx.fonts_mut(|f| f.layout_no_wrap(text.to_string(), regular(11.0), Color32::WHITE).pos_from_cursor(cursor).min.x)
}

/// Read the primary character cursor of a test text field.
fn cursor(ctx: &egui::Context, id: Id) -> CCursor {
    egui::TextEdit::load_state(ctx, id).unwrap().cursor.char_range().unwrap().primary
}

/// A glyph's atlas bitmap, normalized out of its atlas position: which shape the shaper chose.
fn bitmap(image: &egui::ColorImage, glyph: &egui::epaint::text::Glyph) -> (usize, usize, Vec<u8>) {
    let [x0, y0] = glyph.uv_rect.min.map(usize::from);
    let [x1, y1] = glyph.uv_rect.max.map(usize::from);
    let mut pixels = Vec::with_capacity((x1 - x0) * (y1 - y0) * 4);
    for y in y0..y1 {
        for x in x0..x1 {
            pixels.extend(image[(x, y)].to_array());
        }
    }
    (x1 - x0, y1 - y0, pixels)
}

/// A context with the bundled (or explicitly tested) Hebrew and Arabic faces. The required
/// craft-fonts build must carry both; an optional no-font build skips instead of borrowing a
/// system face or treating tofu as evidence.
fn fonts() -> Option<egui::Context> {
    let has = |family: &str| figcraft_text::CRAFT_FONTS.iter().any(|f| f.family == family && f.style == "Regular");
    if !has("Noto Sans Hebrew") || !has("Noto Sans Arabic") {
        assert!(
            option_env!("CRAFT_FONTS_REQUIRED").is_none(),
            "the required craft-fonts input must include Noto Sans Hebrew and Noto Sans Arabic Regular"
        );
        eprintln!("optional craft-fonts input absent: right-to-left names were not qualified");
        return None;
    }
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx, false, 0);
    ctx.run_ui(RawInput::default(), |_| {}).textures_delta.clear();
    ctx.fonts_mut(|f| {
        for c in "שלוםكتاب".chars() {
            assert!(f.has_glyph(&regular(11.0), c), "the bundled RTL faces must draw {c:?}");
        }
    });
    Some(ctx)
}

/// Asserts names keep their logical text while glyphs paint in visual order, including
/// ellipsis placement at the overflow end and mixed-direction rows.
#[test]
fn names_keep_logical_text_and_paint_in_visual_order() {
    let Some(ctx) = fonts() else { return };
    frame(&ctx, vec![], |ui| {
        let p = ui.painter();
        let white = Color32::WHITE;
        let g = name_galley(p, "Rectangle with a rather long name", regular(11.0), white, 60.0);
        assert!(g.elided);
        assert!(g.size().x <= 61.0);
        // The job stays logical; the row keeps the logical start and ends in the overflow character.
        assert_eq!(g.text(), "Rectangle with a rather long name");
        let row = &g.rows[0].row;
        assert!(!row.rtl_paragraph);
        assert!(row.text().ends_with('…'), "{}", row.text());
        assert!(row.text().chars().count() < g.text().chars().count());
        let ellipsis = row.glyphs.last().unwrap();
        // Glyph origins are pixel-rounded while advances are not. Test visual order
        // using origins, rather than comparing an unrounded advance to a rounded origin.
        assert_eq!(row.visual_order().last(), Some(row.glyphs.len() - 1));
        assert!(row.glyphs[..row.glyphs.len() - 1].iter().all(|gl| gl.pos.x < ellipsis.pos.x));

        let g = name_galley(p, "Frame", regular(11.0), white, 200.0);
        assert!(!g.elided);
        assert_eq!(g.text(), "Frame");

        // Right to left: the logical text is unchanged; the glyphs are placed right to left.
        let g = name_galley(p, "שלום", regular(11.0), white, 500.0);
        assert_eq!(g.text(), "שלום");
        let row = &g.rows[0].row;
        assert!(row.rtl_paragraph);
        assert_eq!(row.text(), "שלום");
        assert_eq!(row.visual_order().collect::<Vec<_>>(), vec![3, 2, 1, 0]);
        let xs: Vec<f32> = row.glyphs.iter().map(|g| g.pos.x).collect();
        assert!(xs[0] > xs[1] && xs[1] > xs[2] && xs[2] > xs[3], "logical order paints right to left: {xs:?}");

        // Arabic keeps its logical letters and paints them right to left as joined shapes.
        let g = name_galley(p, "كتاب", regular(11.0), white, 500.0);
        assert_eq!(g.text(), "كتاب");
        let row = &g.rows[0].row;
        assert!(row.rtl_paragraph);
        assert_eq!(row.text(), "كتاب");
        assert_eq!(row.glyphs.len(), 4);
        assert_eq!(row.visual_order().collect::<Vec<_>>(), vec![3, 2, 1, 0]);
        let xs: Vec<f32> = row.glyphs.iter().map(|g| g.pos.x).collect();
        assert!(xs[0] > xs[1] && xs[1] > xs[2] && xs[2] > xs[3], "logical Arabic paints right to left: {xs:?}");

        // Mixed directions: every glyph stays in logical order in the row; visual_order is a
        // permutation placed strictly left to right on screen.
        let mixed = "abc אבג (1)";
        let g = name_galley(p, mixed, regular(11.0), white, 500.0);
        assert_eq!(g.text(), mixed);
        let row = &g.rows[0].row;
        assert_eq!(row.text(), mixed);
        let order: Vec<usize> = row.visual_order().collect();
        let xs: Vec<f32> = order.iter().map(|&i| row.glyphs[i].pos.x).collect();
        assert!(xs.is_sorted(), "{mixed}: {xs:?}");
        let mut sorted = order;
        sorted.sort_unstable();
        assert_eq!(sorted, (0..mixed.chars().count()).collect::<Vec<_>>());
    });
}

/// Asserts Arabic letters take joined contextual forms and truncation puts the ellipsis at
/// the visual end of a right-to-left row.
#[test]
fn arabic_joining_and_truncation_use_the_visual_end() {
    let Some(ctx) = fonts() else { return };
    frame(&ctx, vec![], |ui| {
        let font = regular(11.0);
        let isolated = ui.painter().layout_no_wrap("ب".into(), font.clone(), Color32::WHITE);
        let joined = ui.painter().layout_no_wrap("بب".into(), font, Color32::WHITE);
        let row = &joined.rows[0].row;
        assert_eq!(joined.text(), "بب");
        assert_eq!(row.text(), "بب");
        assert_eq!(row.glyphs.len(), 2);
        let image = ui.ctx().fonts_mut(|fonts| fonts.image());
        let isolated = bitmap(&image, &isolated.rows[0].row.glyphs[0]);
        let first = bitmap(&image, &row.glyphs[0]);
        let second = bitmap(&image, &row.glyphs[1]);
        assert_ne!(first, isolated, "the first ب takes its initial form, not the isolated one");
        assert_ne!(second, isolated, "the second ب takes its final form, not the isolated one");
        assert_ne!(first, second, "the joined letters take different contextual forms");

        let full = "مرحبا بكم في العالم";
        let g = name_galley(ui.painter(), full, regular(11.0), Color32::WHITE, 44.0);
        assert!(g.elided);
        assert_eq!(g.text(), full);
        let row = &g.rows[0].row;
        assert!(row.rtl_paragraph);
        let kept = row.text();
        assert!(kept.ends_with('…'), "{kept:?}");
        assert!(kept.chars().count() > 1, "the fixture must retain text before its ellipsis");
        assert!(full.starts_with(kept.trim_end_matches('…')), "{kept:?}");
        let ellipsis = row.glyphs.last().unwrap();
        assert_eq!(ellipsis.chr, '…');
        assert_eq!(row.visual_order().next(), Some(row.glyphs.len() - 1));
        assert!(
            row.glyphs[..row.glyphs.len() - 1].iter().all(|gl| ellipsis.pos.x < gl.pos.x),
            "the ellipsis is at the visual left of a right-to-left row: {kept:?}"
        );
        assert!(g.size().x <= 45.0);
    });
}

/// Asserts the shaper mirrors right-to-left parentheses in both glyph order and the atlas
/// shapes chosen for them.
#[test]
fn rtl_brackets_are_mirrored_by_the_shaper() {
    let Some(ctx) = fonts() else { return };
    frame(&ctx, vec![], |ui| {
        let text = "عدد (123) هنا";
        let g = name_galley(ui.painter(), text, regular(11.0), Color32::WHITE, 500.0);
        assert_eq!(g.text(), text);
        let row = &g.rows[0].row;
        assert_eq!(row.text(), text);
        assert_eq!(row.visual_order().collect::<Vec<_>>(), vec![12, 11, 10, 9, 8, 5, 6, 7, 4, 3, 2, 1, 0]);
        let xs: Vec<f32> = row.visual_order().map(|i| row.glyphs[i].pos.x).collect();
        assert!(xs.is_sorted(), "{xs:?}");
        assert_eq!(row.glyphs[4].chr, '(');
        assert_eq!(row.glyphs[8].chr, ')');

        // The atlas shape, chosen by which side of the middle row has ink: the logical "(" is
        // drawn as ")" on the right of the number, and the logical ")" as "(" on its left.
        let image = ui.ctx().fonts_mut(|fonts| fonts.image());
        let shape = |glyph: &egui::epaint::text::Glyph| {
            let [x0, y0] = glyph.uv_rect.min.map(usize::from);
            let [x1, y1] = glyph.uv_rect.max.map(usize::from);
            let y = usize::midpoint(y0, y1);
            let ink: Vec<f32> = (x0..x1).map(|x| f32::from(image[(x, y)].a())).collect();
            let weighted: f32 = ink.iter().enumerate().map(|(i, a)| i as f32 * a).sum();
            let alpha_sum = ink.iter().sum::<f32>();
            assert!(alpha_sum > 0.0, "the bracket's middle scanline must contain ink");
            let centroid = weighted / alpha_sum;
            if centroid < (ink.len() - 1) as f32 / 2.0 { '(' } else { ')' }
        };
        assert_eq!(shape(&row.glyphs[4]), ')');
        assert_eq!(shape(&row.glyphs[8]), '(');
    });
}

/// Asserts Right/Left move the caret visually while logical offsets jump, Shift extends
/// visually, and typing inserts at the logical caret; Latin text keeps egui's own caret.
#[test]
fn rename_caret_moves_visually_with_logical_offsets() {
    let Some(ctx) = fonts() else { return };
    let id = Id::unique("rename_test");
    let mut text = "abc אבג".to_string();
    let run = |events: Vec<Event>, text: &mut String| {
        frame(&ctx, events, |ui| {
            ui.memory_mut(|m| m.request_focus(id));
            single_line(ui, id, text, regular(11.0), 200.0);
        });
    };
    run(vec![], &mut text);
    run(vec![], &mut text);
    set_caret(&ctx, id, 3);
    let x3 = caret_x(&ctx, &text, cursor(&ctx, id));
    // Right from the space enters the Hebrew word at its visual left. The cursor's logical
    // offsets jump (4, then 6, then 5, then 4) while its painted x strictly advances.
    run(vec![key(Key::ArrowRight, false)], &mut text);
    assert_eq!(caret(&ctx, id).0, 4);
    let x4 = caret_x(&ctx, &text, cursor(&ctx, id));
    assert!(x4 > x3, "right moves on screen: {x3} -> {x4}");
    run(vec![key(Key::ArrowRight, false)], &mut text);
    assert_eq!(caret(&ctx, id).0, 6);
    let x6 = caret_x(&ctx, &text, cursor(&ctx, id));
    assert!(x6 > x4, "right moves on screen: {x4} -> {x6}");
    // Shift extends from where the caret was; the selection is a logical range (5..6) whose
    // two ends are painted on the screen in that order.
    run(vec![key(Key::ArrowRight, true)], &mut text);
    assert_eq!(caret(&ctx, id), (5, 6));
    let selection = egui::TextEdit::load_state(&ctx, id).unwrap().cursor.char_range().unwrap();
    assert!(caret_x(&ctx, &text, selection.primary) > caret_x(&ctx, &text, selection.secondary));
    // Without Shift a selection collapses to its side on screen.
    run(vec![key(Key::ArrowLeft, false)], &mut text);
    assert_eq!(caret(&ctx, id), (6, 6));
    run(vec![key(Key::ArrowLeft, false), key(Key::ArrowLeft, false)], &mut text);
    assert_eq!(caret(&ctx, id).0, 3);
    // Typing still goes in at the logical caret.
    run(vec![Event::Text("ד".into())], &mut text);
    assert_eq!(text, "abcד אבג");
    // Plain left-to-right text keeps egui's own caret.
    let mut latin = "Frame".to_string();
    let lid = Id::unique("latin");
    let mut run = |events: Vec<Event>| {
        frame(&ctx, events, |ui| {
            ui.memory_mut(|m| m.request_focus(lid));
            single_line(ui, lid, &mut latin, regular(11.0), 200.0);
        });
    };
    run(vec![]);
    run(vec![]);
    set_caret(&ctx, lid, 1);
    run(vec![key(Key::ArrowRight, false)]);
    assert_eq!(caret(&ctx, lid).0, 2);
}

/// Asserts clicks map right-to-left visual positions to logical carets, so a click at the
/// visual left end leaves the caret at the logical end and typing appends there.
#[test]
fn clicks_place_the_logical_caret_by_what_is_shown() {
    let Some(ctx) = fonts() else { return };
    let id = Id::unique("click_test");
    let mut text = "שלום".to_string();
    let mut rect = Rect::NOTHING;
    let run = |events: Vec<Event>, text: &mut String, rect: &mut Rect| {
        frame(&ctx, events, |ui| {
            ui.memory_mut(|m| m.request_focus(id));
            *rect = single_line(ui, id, text, regular(11.0), 200.0).rect;
        });
    };
    run(vec![], &mut text, &mut rect);
    run(vec![], &mut text, &mut rect);
    // The text's right end is its logical start.
    let galley = ctx.fonts_mut(|f| f.layout_no_wrap("שלום".into(), regular(11.0), Color32::WHITE));
    let row = &galley.rows[0].row;
    assert!(row.rtl_paragraph);
    assert_eq!(row.visual_order().collect::<Vec<_>>(), vec![3, 2, 1, 0]);
    let right = Pos2::new(rect.min.x + galley.size().x - 1.0, rect.center().y);
    let press = |p: Pos2, pressed: bool| Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    run(vec![Event::PointerMoved(right), press(right, true)], &mut text, &mut rect);
    run(vec![press(right, false)], &mut text, &mut rect);
    assert_eq!(caret(&ctx, id).0, 0);
    let left = Pos2::new(rect.min.x + 1.0, rect.center().y);
    run(vec![Event::PointerMoved(left), press(left, true)], &mut text, &mut rect);
    run(vec![press(left, false)], &mut text, &mut rect);
    assert_eq!(caret(&ctx, id).0, 4);
    // A click at the visual left end leaves the logical caret at the end of the word, so typing
    // appends to the logical text rather than to the shown (reversed) string.
    run(vec![Event::Text("א".into())], &mut text, &mut rect);
    assert_eq!(text, "שלוםא");
}
